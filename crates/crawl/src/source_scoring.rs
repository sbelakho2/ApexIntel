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

use apex_core::measurement::Measurement;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ────────────────────────────────────────────
// Types
// ────────────────────────────────────────────

/// Raw telemetry for a single crawl source, collected over a rolling window.
///
/// Every analytical field is a [`Measurement`]: a source with no metric row in
/// the window has *unmeasured* yield/freshness/novelty/error, never zero
/// values. The configured crawl interval is deployment configuration (always
/// known) and stays a plain number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTelemetry {
    pub source_id: String,
    pub domain: String,
    /// Total observations ingested from this source in the window.
    pub observations_ingested: Measurement<u64>,
    /// Observations that triggered at least one recipe fire.
    pub observations_in_fires: Measurement<u64>,
    /// Observations that appeared in promoted recipes.
    pub observations_in_promotions: Measurement<u64>,
    /// Median time-to-ingest in seconds (freshness proxy). Not used by the
    /// composite score; carried for reporting.
    pub median_ingest_latency_secs: Measurement<f64>,
    /// Fraction of crawl attempts that failed (4xx/5xx/timeout). Unmeasured is
    /// never treated as "zero errors".
    pub error_rate: Measurement<f64>,
    /// Set of distinct observation types this source produces.
    pub observation_types_produced: Measurement<Vec<String>>,
    /// Hours since the last successful crawl.
    pub hours_since_last_crawl: Measurement<f64>,
    /// Current crawl interval in hours (configuration, always known).
    pub crawl_interval_hours: f64,
}

/// Scored and ranked crawl source.
///
/// `score` and its components are [`Measurement`]s: a source without enough
/// measured history is **ineligible** and carries no score — the ranker never
/// renormalizes a couple of favourable dimensions into an apparently strong
/// score, and it reports completeness separately from the score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSource {
    pub source_id: String,
    pub domain: String,
    /// Yield ratio: observations_in_fires / observations_ingested.
    pub yield_ratio: Measurement<f64>,
    /// Freshness: higher = more responsive source.
    pub freshness: Measurement<f64>,
    /// Novelty: higher = produces observation types that other sources don't.
    pub novelty: Measurement<f64>,
    /// Diversity: how many different observation types this source contributes.
    pub diversity: Measurement<f64>,
    /// Error rate (penalty). `NotMeasured` never means zero.
    pub error_rate: Measurement<f64>,
    /// Composite over the measured components (weights renormalized).
    pub score: Measurement<f64>,
    /// Fraction (0.0..=1.0) of the five score components that were measured.
    pub measurement_completeness: f64,
    /// True when the source has enough measured history for a score.
    pub eligible: bool,
    /// Why the source is ineligible, when it is.
    pub ineligibility_reason: Option<String>,
    /// Suggested crawl interval; `None` for ineligible sources.
    pub suggested_interval_hours: Option<f64>,
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
    /// Minimum measured observations before a source is eligible for a score.
    /// Below this, the ranker reports "insufficient evidence" instead of
    /// scoring noise.
    pub min_observations_for_score: u64,
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
            min_observations_for_score: 1,
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
        if let Some(types) = t.observation_types_produced.value() {
            for ot in types {
                *type_source_count.entry(ot.as_str()).or_default() += 1;
            }
        }
    }
    let total_sources = telemetry.len();

    let mut scored: Vec<ScoredSource> = telemetry
        .iter()
        .map(|t| {
            let ingested = t.observations_ingested.value_copied();
            let fires = t.observations_in_fires.value_copied();

            let (yield_ratio, ingested_measured) = match (ingested, fires) {
                (Some(ingested), Some(fires)) => {
                    if ingested >= config.min_observations_for_score {
                        (
                            Measurement::measured(fires as f64 / ingested.max(1) as f64),
                            Some(ingested),
                        )
                    } else {
                        (Measurement::insufficient_evidence(), Some(ingested))
                    }
                }
                _ => (Measurement::NotMeasured, None),
            };

            // Freshness: exponential decay from hours since last crawl; an
            // unknown age is not "fresh".
            let freshness = match t.hours_since_last_crawl.value_copied() {
                Some(hours) => Measurement::measured(
                    (-hours.max(0.0) * (2.0f64.ln()) / config.freshness_halflife_hours)
                        .exp()
                        .clamp(0.0, 1.0),
                ),
                None => Measurement::NotMeasured,
            };

            // Novelty and diversity need the measured produced-type set.
            let (novelty, diversity) = match t.observation_types_produced.value() {
                Some(types) => {
                    let novelty = if types.is_empty() {
                        Measurement::measured(0.0)
                    } else {
                        let sum: f64 = types
                            .iter()
                            .map(|ot| {
                                let n = *type_source_count.get(ot.as_str()).unwrap_or(&1) as f64;
                                1.0 - (n - 1.0) / total_sources as f64
                            })
                            .sum();
                        Measurement::measured((sum / types.len() as f64).clamp(0.0, 1.0))
                    };
                    let diversity =
                        Measurement::measured((types.len() as f64 / 16.0).clamp(0.0, 1.0));
                    (novelty, diversity)
                }
                None => (Measurement::NotMeasured, Measurement::NotMeasured),
            };

            let error_rate = match t.error_rate.value_copied() {
                Some(rate) => Measurement::measured(rate.clamp(0.0, 1.0)),
                None => Measurement::NotMeasured,
            };

            // Composite over measured components only, with weights
            // renormalized — completeness is reported separately and the score
            // stays `NotMeasured` when nothing was measured.
            let components: [(f64, &Measurement<f64>); 4] = [
                (config.weight_yield, &yield_ratio),
                (config.weight_freshness, &freshness),
                (config.weight_novelty, &novelty),
                (config.weight_diversity, &diversity),
            ];
            let mut weighted_sum = 0.0;
            let mut weight_total = 0.0;
            let mut measured_components = 0u32;
            for (weight, component) in &components {
                if let Some(value) = component.value() {
                    weighted_sum += weight * value;
                    weight_total += weight;
                    measured_components += 1;
                }
            }
            if let Some(rate) = error_rate.value() {
                weighted_sum -= config.weight_error * rate;
                weight_total += config.weight_error;
                measured_components += 1;
            }
            let measurement_completeness = f64::from(measured_components) / 5.0;
            let eligible = ingested_measured
                .is_some_and(|ingested| ingested >= config.min_observations_for_score)
                && measured_components > 0;
            let ineligibility_reason = if eligible {
                None
            } else if ingested_measured.is_some() {
                Some(format!(
                    "fewer than {} observations measured in the window",
                    config.min_observations_for_score
                ))
            } else {
                Some("no measured telemetry in the window".to_string())
            };
            let score = if eligible && weight_total > 0.0 {
                Measurement::measured((weighted_sum / weight_total).clamp(0.0, 1.0))
            } else {
                Measurement::NotMeasured
            };

            let suggested_interval_hours = score.value().map(|score| {
                let interval_range = config.max_interval_hours - config.min_interval_hours;
                config.max_interval_hours - score * interval_range
            });

            ScoredSource {
                source_id: t.source_id.clone(),
                domain: t.domain.clone(),
                yield_ratio,
                freshness,
                novelty,
                diversity,
                error_rate,
                score,
                measurement_completeness,
                eligible,
                ineligibility_reason,
                suggested_interval_hours,
            }
        })
        .collect();

    // Sort: eligible sources by score (descending), then ineligible ones
    // deterministically by id.
    scored.sort_by(|a, b| {
        let a_score = a.score.value_copied();
        let b_score = b.score.value_copied();
        match (a_score, b_score) {
            (Some(a_score), Some(b_score)) => b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.source_id.cmp(&b.source_id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.source_id.cmp(&b.source_id),
        }
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
            // Ineligible sources carry no interval suggestion.
            let suggested = s.suggested_interval_hours?;
            let score = s.score.value_copied()?;
            let current = *current_intervals.get(s.source_id.as_str()).unwrap_or(&24.0);
            let pct_change = ((suggested - current) / current).abs();
            if pct_change > 0.10 {
                let direction = if suggested < current {
                    "increase frequency"
                } else {
                    "decrease frequency"
                };
                Some(ScheduleAdjustment {
                    source_id: s.source_id.clone(),
                    domain: s.domain.clone(),
                    current_interval_hours: current,
                    new_interval_hours: suggested,
                    reason: format!(
                        "score {:.3} ({:.0}% of components measured) suggests to {direction} \
                         from {:.1}h to {:.1}h",
                        score,
                        s.measurement_completeness * 100.0,
                        current,
                        suggested,
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

    fn m<T>(value: T) -> Measurement<T> {
        Measurement::measured(value)
    }

    fn sample_telemetry() -> Vec<SourceTelemetry> {
        vec![
            SourceTelemetry {
                source_id: "reuters".into(),
                domain: "reuters.com".into(),
                observations_ingested: m(1000),
                observations_in_fires: m(200),
                observations_in_promotions: m(50),
                median_ingest_latency_secs: m(30.0),
                error_rate: m(0.01),
                observation_types_produced: m(vec![
                    "JobPost".into(),
                    "CommodityPrice".into(),
                    "CompetitorEvent".into(),
                ]),
                hours_since_last_crawl: m(2.0),
                crawl_interval_hours: 4.0,
            },
            SourceTelemetry {
                source_id: "obscure-blog".into(),
                domain: "random-blog.net".into(),
                observations_ingested: m(50),
                observations_in_fires: m(0),
                observations_in_promotions: m(0),
                median_ingest_latency_secs: m(3600.0),
                error_rate: m(0.40),
                observation_types_produced: m(vec!["WebChange".into()]),
                hours_since_last_crawl: m(200.0),
                crawl_interval_hours: 24.0,
            },
            SourceTelemetry {
                source_id: "tunisian-tenders".into(),
                domain: "tenders.gov.tn".into(),
                observations_ingested: m(300),
                observations_in_fires: m(100),
                observations_in_promotions: m(30),
                median_ingest_latency_secs: m(120.0),
                error_rate: m(0.05),
                observation_types_produced: m(vec![
                    "TenderPosted".into(),
                    "ProcurementSignal".into(),
                ]),
                hours_since_last_crawl: m(6.0),
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
        assert!(
            scored[0].freshness.value_copied().unwrap()
                > scored[2].freshness.value_copied().unwrap()
        );
    }

    #[test]
    fn test_score_novelty() {
        // If a source is the only one providing a type, novelty should be high.
        let tel = vec![SourceTelemetry {
            source_id: "unique".into(),
            domain: "unique.com".into(),
            observations_ingested: m(100),
            observations_in_fires: m(10),
            observations_in_promotions: m(5),
            median_ingest_latency_secs: m(60.0),
            error_rate: m(0.0),
            observation_types_produced: m(vec!["PatentPublished".into()]), // unique type
            hours_since_last_crawl: m(1.0),
            crawl_interval_hours: 4.0,
        }];
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        assert_eq!(scored.len(), 1);
        let novelty = scored[0].novelty.value_copied().unwrap();
        assert!(
            (novelty - 1.0).abs() < 0.01,
            "Sole producer should get novelty ≈ 1.0"
        );
    }

    #[test]
    fn test_suggested_interval() {
        let tel = sample_telemetry();
        let scored = score_and_rank(&tel, &ScoringConfig::default());
        // Higher-scored sources should get shorter intervals.
        assert!(
            scored[0].suggested_interval_hours.unwrap()
                < scored[2].suggested_interval_hours.unwrap()
        );
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
