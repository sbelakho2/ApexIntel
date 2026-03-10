//! Adaptive source scoring — data-driven crawl prioritisation.
//!
//! Closes the gap between "what we crawl" and "what actually produces
//! valuable insights".  Each crawl source gets a dynamic score that
//! factors in historical yield, freshness, novelty, and diversity.
//!
//! The score feeds back into the crawl scheduler to allocate bandwidth
//! proportionally: high-value sources are crawled more often; low-value
//! sources are throttled to free budget.
//!
//! # Scoring model
//!
//! ```text
//!   score = w_yield  × yield_ratio
//!         + w_fresh  × freshness
//!         + w_novel  × novelty
//!         + w_div    × diversity
//!         − w_error  × error_rate
//! ```
//!
//! All components are normalised to [0, 1].
//! Weights are configurable; defaults are tuned for a conservative start-up
//! phase where the system does not yet have lifecycle data (all sources score
//! neutrally until they accumulate history).
//!
//! Pure functions — no database, no side effects.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ────────────────────────────────────────────
// Types
// ────────────────────────────────────────────

/// Raw telemetry for a single crawl source, collected over a rolling window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTelemetry {
    pub source_id: String,
    pub domain: String,
    /// Total observations ingested from this source in the window.
    pub observations_ingested: u64,
    /// Observations that triggered at least one recipe fire.
    pub observations_in_fires: u64,
    /// Observations that appeared in promoted recipes.
    pub observations_in_promotions: u64,
    /// Median time-to-ingest in seconds (freshness proxy).
    pub median_ingest_latency_secs: f64,
    /// Fraction of crawl attempts that failed (4xx/5xx/timeout).
    pub error_rate: f64,
    /// Set of distinct [`ObservationType`] variants this source produces.
    pub observation_types_produced: Vec<String>,
    /// Hours since the last successful crawl.
    pub hours_since_last_crawl: f64,
    /// Current crawl interval in hours.
    pub crawl_interval_hours: f64,
}

/// Scored and ranked crawl source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSource {
    pub source_id: String,
    pub domain: String,
    /// Yield ratio: observations_in_fires / observations_ingested.
    pub yield_ratio: f64,
    /// Freshness: higher = more responsive source.
    pub freshness: f64,
    /// Novelty: higher = produces observation types that other sources don't.
    pub novelty: f64,
    /// Diversity: how many different observation types this source contributes.
    pub diversity: f64,
    /// Error rate (penalty).
    pub error_rate: f64,
    /// Composite score.
    pub score: f64,
    /// Suggested crawl interval in hours.
    pub suggested_interval_hours: f64,
}

/// Scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringConfig {
    pub weight_yield: f64,
    pub weight_freshness: f64,
    pub weight_novelty: f64,
    pub weight_diversity: f64,
    pub weight_error: f64,
    /// Maximum crawl interval in hours (for low-scoring sources).
    pub max_interval_hours: f64,
    /// Minimum crawl interval in hours (for high-scoring sources).
    pub min_interval_hours: f64,
    /// Freshness decay half-life in hours.
    pub freshness_halflife_hours: f64,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            weight_yield: 0.35,
            weight_freshness: 0.15,
            weight_novelty: 0.20,
            weight_diversity: 0.15,
            weight_error: 0.15,
            max_interval_hours: 168.0, // weekly
            min_interval_hours: 1.0,   // hourly
            freshness_halflife_hours: 48.0,
        }
    }
}

impl ScoringConfig {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let total = self.weight_yield
            + self.weight_freshness
            + self.weight_novelty
            + self.weight_diversity
            + self.weight_error;
        if (total - 1.0).abs() > 0.05 {
            errors.push(format!("Weights sum to {:.3}, expected ~1.0", total));
        }
        if self.max_interval_hours <= self.min_interval_hours {
            errors.push("max_interval_hours must exceed min_interval_hours".into());
        }
        if self.freshness_halflife_hours <= 0.0 {
            errors.push("freshness_halflife_hours must be > 0".into());
        }
        errors
    }
}

// ────────────────────────────────────────────
// Scoring engine
// ────────────────────────────────────────────

/// Score and rank crawl sources.
///
/// For each source, computes a composite score in [0, 1] and suggests a
/// new crawl interval inversely proportional to the score.
///
/// **Novelty** is computed globally: a source that is the *only* provider
/// of an observation type gets a higher novelty bonus.
pub fn score_and_rank(telemetry: &[SourceTelemetry], config: &ScoringConfig) -> Vec<ScoredSource> {
    if telemetry.is_empty() {
        return Vec::new();
    }

    // Build a global map: obs_type → how many sources produce it.
    let mut type_source_count: HashMap<&str, usize> = HashMap::new();
    for t in telemetry {
        for ot in &t.observation_types_produced {
            *type_source_count.entry(ot.as_str()).or_default() += 1;
        }
    }
    let total_sources = telemetry.len();

    let mut scored: Vec<ScoredSource> = telemetry
        .iter()
        .map(|t| {
            let ingested = t.observations_ingested.max(1) as f64;

            // Yield: fraction of observations that triggered recipe fires.
            let yield_ratio = t.observations_in_fires as f64 / ingested;

            // Freshness: exponential decay from hours since last crawl.
            let freshness = (-t.hours_since_last_crawl.max(0.0) * (2.0f64.ln())
                / config.freshness_halflife_hours)
                .exp()
                .clamp(0.0, 1.0);

            // Novelty: average "uniqueness" of the observation types this source produces.
            // If a type is produced by only 1 source → novelty=1; by all sources → novelty→0.
            let novelty = if t.observation_types_produced.is_empty() {
                0.0
            } else {
                let sum: f64 = t
                    .observation_types_produced
                    .iter()
                    .map(|ot| {
                        let n = *type_source_count.get(ot.as_str()).unwrap_or(&1) as f64;
                        1.0 - (n - 1.0) / total_sources as f64
                    })
                    .sum();
                (sum / t.observation_types_produced.len() as f64).clamp(0.0, 1.0)
            };

            // Diversity: normalised count of distinct observation types.
            // Cap denominator at 16 (total ObservationType variants).
            let diversity = (t.observation_types_produced.len() as f64 / 16.0).clamp(0.0, 1.0);

            let error_rate = t.error_rate.clamp(0.0, 1.0);

            let score = (config.weight_yield * yield_ratio
                + config.weight_freshness * freshness
                + config.weight_novelty * novelty
                + config.weight_diversity * diversity
                - config.weight_error * error_rate)
                .clamp(0.0, 1.0);

            // Suggested interval: inversely proportional to score.
            // score=1 → min_interval; score=0 → max_interval.
            let interval_range = config.max_interval_hours - config.min_interval_hours;
            let suggested_interval_hours = config.max_interval_hours - score * interval_range;

            ScoredSource {
                source_id: t.source_id.clone(),
                domain: t.domain.clone(),
                yield_ratio,
                freshness,
                novelty,
                diversity,
                error_rate,
                score,
                suggested_interval_hours,
            }
        })
        .collect();

    // Sort descending by score, deterministic tiebreak by source_id (B292).
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    scored
}

// ────────────────────────────────────────────
// Gap analysis: discover under-covered domains
// ────────────────────────────────────────────

/// An observation-type coverage gap — a type that is in demand (appears in
/// promoted recipes) but under-served by crawl sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageGap {
    pub obs_type: String,
    /// How many promoted recipes use this type.
    pub demand: usize,
    /// How many crawl sources currently produce it.
    pub supply: usize,
    /// demand / supply ratio (higher = bigger gap).
    pub gap_ratio: f64,
}

/// Identify observation types that are in high demand (promoted recipes)
/// but served by few crawl sources.
///
/// `recipe_signal_types`: for each promoted recipe, its signal types.
/// `source_types`: for each source, its produced types.
pub fn coverage_gaps(
    recipe_signal_types: &[Vec<String>],
    source_types: &[Vec<String>],
) -> Vec<CoverageGap> {
    // Demand: count how many promoted recipes include each obs type.
    let mut demand: HashMap<String, usize> = HashMap::new();
    for recipe in recipe_signal_types {
        for t in recipe {
            *demand.entry(t.clone()).or_default() += 1;
        }
    }

    // Supply: count how many sources produce each obs type.
    let mut supply: HashMap<String, usize> = HashMap::new();
    for source in source_types {
        // Deduplicate within source.
        let unique: HashSet<&String> = source.iter().collect();
        for t in unique {
            *supply.entry(t.clone()).or_default() += 1;
        }
    }

    let mut gaps: Vec<CoverageGap> = demand
        .iter()
        .map(|(obs_type, &d)| {
            let s = *supply.get(obs_type).unwrap_or(&0);
            let gap_ratio = d as f64 / (s as f64).max(0.01);
            CoverageGap {
                obs_type: obs_type.clone(),
                demand: d,
                supply: s,
                gap_ratio,
            }
        })
        .collect();

    gaps.sort_by(|a, b| {
        b.gap_ratio
            .partial_cmp(&a.gap_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.obs_type.cmp(&b.obs_type))
    });
    gaps
}

// ────────────────────────────────────────────
// Schedule adjustment
// ────────────────────────────────────────────

/// A concrete schedule change to apply to a crawl source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleAdjustment {
    pub source_id: String,
    pub domain: String,
    pub current_interval_hours: f64,
    pub new_interval_hours: f64,
    pub reason: String,
}

/// Compute schedule adjustments from scored sources.
///
/// Only returns changes where the suggested interval differs from the
/// current interval by more than 10%.
pub fn compute_adjustments(
    scored: &[ScoredSource],
    telemetry: &[SourceTelemetry],
) -> Vec<ScheduleAdjustment> {
    let current_intervals: HashMap<&str, f64> = telemetry
        .iter()
        .map(|t| (t.source_id.as_str(), t.crawl_interval_hours))
        .collect();

    scored
        .iter()
        .filter_map(|s| {
            let current = *current_intervals.get(s.source_id.as_str()).unwrap_or(&24.0);
            let pct_change = ((s.suggested_interval_hours - current) / current).abs();
            if pct_change > 0.10 {
                let direction = if s.suggested_interval_hours < current {
                    "increase frequency"
                } else {
                    "decrease frequency"
                };
                Some(ScheduleAdjustment {
                    source_id: s.source_id.clone(),
                    domain: s.domain.clone(),
                    current_interval_hours: current,
                    new_interval_hours: s.suggested_interval_hours,
                    reason: format!(
                        "{}: score={:.3}, yield={:.3}, novelty={:.3}, errors={:.3}",
                        direction, s.score, s.yield_ratio, s.novelty, s.error_rate
                    ),
                })
            } else {
                None
            }
        })
        .collect()
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_telemetry() -> Vec<SourceTelemetry> {
        vec![
            SourceTelemetry {
                source_id: "reuters".into(),
                domain: "reuters.com".into(),
                observations_ingested: 1000,
                observations_in_fires: 200,
                observations_in_promotions: 50,
                median_ingest_latency_secs: 30.0,
                error_rate: 0.01,
                observation_types_produced: vec![
                    "JobPost".into(),
                    "CommodityPrice".into(),
                    "CompetitorEvent".into(),
                ],
                hours_since_last_crawl: 2.0,
                crawl_interval_hours: 4.0,
            },
            SourceTelemetry {
                source_id: "obscure-blog".into(),
                domain: "random-blog.net".into(),
                observations_ingested: 50,
                observations_in_fires: 0,
                observations_in_promotions: 0,
                median_ingest_latency_secs: 3600.0,
                error_rate: 0.40,
                observation_types_produced: vec!["WebChange".into()],
                hours_since_last_crawl: 200.0,
                crawl_interval_hours: 24.0,
            },
            SourceTelemetry {
                source_id: "tunisian-tenders".into(),
                domain: "tenders.gov.tn".into(),
                observations_ingested: 300,
                observations_in_fires: 100,
                observations_in_promotions: 30,
                median_ingest_latency_secs: 120.0,
                error_rate: 0.05,
                observation_types_produced: vec!["TenderPosted".into(), "ProcurementSignal".into()],
                hours_since_last_crawl: 6.0,
                crawl_interval_hours: 12.0,
            },
        ]
    }

    #[test]
    fn test_score_and_rank_ordering() {
        let tel = sample_telemetry();
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        assert_eq!(scored.len(), 3);
        // tunisian-tenders has highest yield (100/300=0.333) → should be first.
        assert_eq!(scored[0].source_id, "tunisian-tenders");
        // reuters has second-highest yield → second.
        assert_eq!(scored[1].source_id, "reuters");
        // obscure blog has zero yield + high errors → should be last.
        assert_eq!(scored[2].source_id, "obscure-blog");
    }

    #[test]
    fn test_score_freshness_decay() {
        let tel = sample_telemetry();
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        // reuters (2h since crawl) should be fresher than obscure-blog (200h).
        assert!(scored[0].freshness > scored[2].freshness);
    }

    #[test]
    fn test_score_novelty() {
        // If a source is the only one providing a type, novelty should be high.
        let tel = vec![SourceTelemetry {
            source_id: "unique".into(),
            domain: "unique.com".into(),
            observations_ingested: 100,
            observations_in_fires: 10,
            observations_in_promotions: 5,
            median_ingest_latency_secs: 60.0,
            error_rate: 0.0,
            observation_types_produced: vec!["PatentPublished".into()], // unique type
            hours_since_last_crawl: 1.0,
            crawl_interval_hours: 4.0,
        }];
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        assert_eq!(scored.len(), 1);
        assert!(
            (scored[0].novelty - 1.0).abs() < 0.01,
            "Sole producer should get novelty ≈ 1.0"
        );
    }

    #[test]
    fn test_suggested_interval() {
        let tel = sample_telemetry();
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        // Higher-scored sources should get shorter intervals.
        assert!(scored[0].suggested_interval_hours < scored[2].suggested_interval_hours);
    }

    #[test]
    fn test_coverage_gaps() {
        let recipe_types = vec![
            vec!["JobPost".into(), "CommodityPrice".into()],
            vec!["JobPost".into(), "PatentPublished".into()],
            vec!["PatentPublished".into()],
        ];
        let source_types = vec![
            vec!["JobPost".into(), "CommodityPrice".into()],
            vec!["WebChange".into()],
        ];
        let gaps = coverage_gaps(&recipe_types, &source_types);
        // PatentPublished: demand=2, supply=0 → biggest gap.
        assert!(!gaps.is_empty());
        assert_eq!(gaps[0].obs_type, "PatentPublished");
        assert_eq!(gaps[0].demand, 2);
        assert_eq!(gaps[0].supply, 0);
    }

    #[test]
    fn test_compute_adjustments() {
        let tel = sample_telemetry();
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        let adjustments = compute_adjustments(&scored, &tel);
        // Should recommend changes for at least the obscure blog (big gap between
        // current 24h and suggested ~168h).
        assert!(
            !adjustments.is_empty(),
            "Expected at least one schedule adjustment"
        );
    }

    #[test]
    fn test_empty_telemetry() {
        let scored = score_and_rank(&[], &ScoringConfig::default());
        assert!(scored.is_empty());
    }

    #[test]
    fn test_config_validate() {
        let cfg = ScoringConfig::default();
        assert!(cfg.validate().is_empty());

        let bad = ScoringConfig {
            weight_yield: 0.9,
            weight_freshness: 0.9,
            ..ScoringConfig::default()
        };
        assert!(!bad.validate().is_empty());
    }
}
