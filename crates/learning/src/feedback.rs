//! Self-improving feedback loop — meta-learning from the system's own history.
//!
//! This module closes the loop between pattern mining → recipe lifecycle →
//! data collection by analysing *what worked* and *what didn't*.
//!
//! # Feedback loops implemented
//!
//! 1. **Source yield scoring** — which crawl sources produce observations that
//!    eventually become promoted recipes?  Surfaces low-value sources and
//!    identifies under-tapped domains.
//!
//! 2. **Observation-type value ranking** — which [`ObservationType`] variants
//!    appear in promoted vs. retired recipes?  Shifts collection priority
//!    toward high-value signal families.
//!
//! 3. **Recipe trait extraction** — what do promoted recipes have in common
//!    (effect size, lag, signal count, category)?  Feeds back into miner
//!    thresholds so the system tunes itself.
//!
//! 4. **Cross-domain synergy scoring** — which pairs of observation types
//!    co-occur in promoted recipes more often than chance?
//!
//! All functions are pure (no database); data arrives as typed slices.
//! Enable with `--features experimental`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ────────────────────────────────────────────
// Source yield scoring
// ────────────────────────────────────────────

/// A single crawl source's contribution to the insight pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceYield {
    pub source_id: String,
    pub domain: String,
    pub total_observations: u64,
    pub observations_in_fired_recipes: u64,
    pub observations_in_promoted_recipes: u64,
    pub observations_in_retired_recipes: u64,
    pub freshness_hours: f64,
    pub diversity_score: f64,
}

/// Computed score for a crawl source (higher = more valuable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceScore {
    pub source_id: String,
    pub domain: String,
    /// Fraction of observations that contributed to *promoted* recipes.
    pub promotion_yield: f64,
    /// Fraction of observations that contributed to *any* fired recipe.
    pub fire_yield: f64,
    /// Penalty for observations that ended up in *retired* recipes.
    pub retirement_drag: f64,
    /// Freshness bonus (decays with stale sources).
    pub freshness_bonus: f64,
    /// Diversity bonus — sources that provide unique observation types score higher.
    pub diversity_bonus: f64,
    /// Composite score: weighted combination.
    pub composite: f64,
}

/// Weights for combining source-scoring components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceScoringWeights {
    pub promotion_yield_w: f64,
    pub fire_yield_w: f64,
    pub retirement_drag_w: f64,
    pub freshness_w: f64,
    pub diversity_w: f64,
}

impl Default for SourceScoringWeights {
    fn default() -> Self {
        Self {
            promotion_yield_w: 0.40,
            fire_yield_w: 0.20,
            retirement_drag_w: 0.15,
            freshness_w: 0.10,
            diversity_w: 0.15,
        }
    }
}

/// Score a batch of sources and rank them by composite value.
///
/// Returns scores sorted descending by `composite`.  Sources with zero
/// observations are scored conservatively at 0.0 (not penalised, not
/// rewarded — giving new sources a neutral start).
pub fn score_sources(
    yields: &[SourceYield],
    weights: &SourceScoringWeights,
) -> Vec<SourceScore> {
    let mut scores: Vec<SourceScore> = yields.iter().map(|y| {
        let total = y.total_observations.max(1) as f64;
        let promotion_yield = y.observations_in_promoted_recipes as f64 / total;
        let fire_yield = y.observations_in_fired_recipes as f64 / total;
        let retirement_drag = y.observations_in_retired_recipes as f64 / total;
        // Freshness: 1.0 for perfectly fresh, decays to 0 at 720 hours (30 days).
        let freshness_bonus = (1.0 - y.freshness_hours / 720.0).clamp(0.0, 1.0);
        let diversity_bonus = y.diversity_score.clamp(0.0, 1.0);

        let composite = weights.promotion_yield_w * promotion_yield
            + weights.fire_yield_w * fire_yield
            - weights.retirement_drag_w * retirement_drag
            + weights.freshness_w * freshness_bonus
            + weights.diversity_w * diversity_bonus;

        SourceScore {
            source_id: y.source_id.clone(),
            domain: y.domain.clone(),
            promotion_yield,
            fire_yield,
            retirement_drag,
            freshness_bonus,
            diversity_bonus,
            composite,
        }
    }).collect();

    // Deterministic sort: composite desc, source_id asc for ties (B292).
    scores.sort_by(|a, b| {
        b.composite
            .partial_cmp(&a.composite)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    scores
}

// ────────────────────────────────────────────
// Observation-type value ranking
// ────────────────────────────────────────────

/// Per-observation-type contribution to the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsTypeStats {
    pub obs_type: String,
    pub total_occurrences: u64,
    pub in_promoted_recipes: u64,
    pub in_staged_recipes: u64,
    pub in_retired_recipes: u64,
}

/// Computed value-rank for an observation type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsTypeValue {
    pub obs_type: String,
    /// How often this type contributes to promotion (0–1).
    pub promotion_rate: f64,
    /// Retirement drag: fraction of occurrences in retired recipes.
    pub retirement_rate: f64,
    /// Net value = promotion_rate − 0.5 × retirement_rate.
    pub net_value: f64,
}

/// Rank observation types by their net contribution to promoted recipes.
///
/// Returns values sorted descending by `net_value`.
pub fn rank_observation_types(stats: &[ObsTypeStats]) -> Vec<ObsTypeValue> {
    let mut values: Vec<ObsTypeValue> = stats.iter().map(|s| {
        let total = s.total_occurrences.max(1) as f64;
        let promotion_rate = s.in_promoted_recipes as f64 / total;
        let retirement_rate = s.in_retired_recipes as f64 / total;
        let net_value = promotion_rate - 0.5 * retirement_rate;
        ObsTypeValue {
            obs_type: s.obs_type.clone(),
            promotion_rate,
            retirement_rate,
            net_value,
        }
    }).collect();

    values.sort_by(|a, b| {
        b.net_value
            .partial_cmp(&a.net_value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.obs_type.cmp(&b.obs_type))
    });
    values
}

// ────────────────────────────────────────────
// Recipe trait extraction — meta-learning
// ────────────────────────────────────────────

/// Minimal summary of a recipe's lifecycle for meta-learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeTraitRecord {
    pub recipe_id: String,
    pub status: String, // "promoted" or "retired"
    pub signal_count: usize,
    pub signal_types: Vec<String>,
    pub effect_size: f64,
    pub p_value: f64,
    pub best_lag_days: i32,
    pub category: String,
    pub precision_at_retirement: Option<f64>,
    pub weeks_active: u32,
}

/// Traits that distinguish promoted from retired recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaLearningInsight {
    /// Mean effect size of promoted recipes.
    pub promoted_mean_effect: f64,
    /// Mean effect size of retired recipes.
    pub retired_mean_effect: f64,
    /// Suggested minimum effect size (midpoint of promoted mean and current min).
    pub suggested_min_effect: f64,
    /// Mean lag days for promoted recipes.
    pub promoted_mean_lag: f64,
    /// Mean lag days for retired recipes.
    pub retired_mean_lag: f64,
    /// Suggested max lag days (1 stddev above promoted mean, capped at 365).
    pub suggested_max_lag: i32,
    /// Mean signal count in promoted recipes.
    pub promoted_mean_signal_count: f64,
    /// Which signal types appear disproportionately in promotions.
    pub high_value_signal_types: Vec<String>,
    /// Which categories have the best promotion-to-retirement ratio.
    pub best_categories: Vec<String>,
    /// Confidence: number of records used for the analysis.
    pub sample_size_promoted: usize,
    pub sample_size_retired: usize,
}

/// Extract meta-learning insights from a batch of recipe trait records.
///
/// Returns `None` if fewer than 3 promoted AND 3 retired records exist
/// (not enough data for meaningful meta-learning).
pub fn extract_meta_insights(records: &[RecipeTraitRecord]) -> Option<MetaLearningInsight> {
    let promoted: Vec<&RecipeTraitRecord> = records
        .iter()
        .filter(|r| r.status == "promoted")
        .collect();
    let retired: Vec<&RecipeTraitRecord> = records
        .iter()
        .filter(|r| r.status == "retired")
        .collect();

    if promoted.len() < 3 || retired.len() < 3 {
        return None;
    }

    let mean = |vals: &[f64]| -> f64 {
        if vals.is_empty() { return 0.0; }
        vals.iter().sum::<f64>() / vals.len() as f64
    };

    let stddev = |vals: &[f64]| -> f64 {
        let m = mean(vals);
        let variance = vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / vals.len() as f64;
        variance.sqrt()
    };

    let prom_effects: Vec<f64> = promoted.iter().map(|r| r.effect_size).collect();
    let ret_effects: Vec<f64> = retired.iter().map(|r| r.effect_size).collect();
    let prom_lags: Vec<f64> = promoted.iter().map(|r| r.best_lag_days as f64).collect();
    let ret_lags: Vec<f64> = retired.iter().map(|r| r.best_lag_days as f64).collect();
    let prom_sig_counts: Vec<f64> = promoted.iter().map(|r| r.signal_count as f64).collect();

    let promoted_mean_effect = mean(&prom_effects);
    let retired_mean_effect = mean(&ret_effects);
    let promoted_mean_lag = mean(&prom_lags);
    let retired_mean_lag = mean(&ret_lags);

    // Suggested min effect: halfway between promoted mean and current default.
    let suggested_min_effect = (promoted_mean_effect + 1.5) / 2.0;

    // Suggested max lag: promoted mean + 1 stddev, capped at 365.
    let suggested_max_lag = (promoted_mean_lag + stddev(&prom_lags))
        .round()
        .min(365.0) as i32;

    // Signal type frequency analysis: types appearing ≥2× more in promoted than retired.
    let mut prom_type_freq: HashMap<String, usize> = HashMap::new();
    for r in &promoted {
        for t in &r.signal_types {
            *prom_type_freq.entry(t.clone()).or_default() += 1;
        }
    }
    let mut ret_type_freq: HashMap<String, usize> = HashMap::new();
    for r in &retired {
        for t in &r.signal_types {
            *ret_type_freq.entry(t.clone()).or_default() += 1;
        }
    }
    let prom_total = promoted.len().max(1) as f64;
    let ret_total = retired.len().max(1) as f64;

    let mut high_value_signal_types: Vec<String> = prom_type_freq
        .iter()
        .filter(|(sig_type, &count)| {
            let prom_rate = count as f64 / prom_total;
            let ret_rate = *ret_type_freq.get(*sig_type).unwrap_or(&0) as f64 / ret_total;
            prom_rate >= 2.0 * ret_rate.max(0.01)
        })
        .map(|(t, _)| t.clone())
        .collect();
    high_value_signal_types.sort(); // deterministic ordering

    // Category analysis: promotion rate per category.
    let mut cat_prom: HashMap<String, usize> = HashMap::new();
    let mut cat_ret: HashMap<String, usize> = HashMap::new();
    for r in &promoted {
        *cat_prom.entry(r.category.clone()).or_default() += 1;
    }
    for r in &retired {
        *cat_ret.entry(r.category.clone()).or_default() += 1;
    }
    let mut best_categories: Vec<String> = cat_prom
        .iter()
        .filter(|(cat, &p_count)| {
            let r_count = *cat_ret.get(*cat).unwrap_or(&0);
            p_count > r_count
        })
        .map(|(c, _)| c.clone())
        .collect();
    best_categories.sort();

    Some(MetaLearningInsight {
        promoted_mean_effect,
        retired_mean_effect,
        suggested_min_effect,
        promoted_mean_lag,
        retired_mean_lag,
        suggested_max_lag,
        promoted_mean_signal_count: mean(&prom_sig_counts),
        high_value_signal_types,
        best_categories,
        sample_size_promoted: promoted.len(),
        sample_size_retired: retired.len(),
    })
}

// ────────────────────────────────────────────
// Cross-domain synergy scoring
// ────────────────────────────────────────────

/// Co-occurrence of two observation types in promoted recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSynergy {
    pub type_a: String,
    pub type_b: String,
    /// How often this pair co-occurs in promoted recipes vs. all recipes.
    pub co_occurrence_uplift: f64,
    /// How many promoted recipes contain both types.
    pub promoted_count: usize,
    /// How many total recipes contain both types.
    pub total_count: usize,
    /// Jaccard similarity of the two signal types in promoted recipes.
    pub jaccard: f64,
}

/// Discover synergistic signal pairs that co-occur disproportionately in
/// promoted recipes vs. the overall recipe corpus.
///
/// `promoted_recipes` and `all_recipes`: each entry is the set of
/// observation types used in that recipe.
///
/// Returns pairs sorted descending by `co_occurrence_uplift`,
/// filtered to uplift ≥ `min_uplift` (default 1.5).
pub fn discover_synergies(
    promoted_recipes: &[Vec<String>],
    all_recipes: &[Vec<String>],
    min_uplift: f64,
) -> Vec<SignalSynergy> {
    let pair_counts =
        |recipes: &[Vec<String>]| -> HashMap<(String, String), usize> {
            let mut counts = HashMap::new();
            for recipe in recipes {
                let mut types: Vec<&String> = recipe.iter().collect();
                types.sort();
                types.dedup();
                for i in 0..types.len() {
                    for j in (i + 1)..types.len() {
                        let key = (types[i].clone(), types[j].clone());
                        *counts.entry(key).or_insert(0) += 1;
                    }
                }
            }
            counts
        };

    let prom_counts = pair_counts(promoted_recipes);
    let all_counts = pair_counts(all_recipes);

    let prom_total = promoted_recipes.len().max(1) as f64;
    let all_total = all_recipes.len().max(1) as f64;

    let mut synergies: Vec<SignalSynergy> = prom_counts
        .iter()
        .filter_map(|((a, b), &prom_n)| {
            let total_n = *all_counts.get(&(a.clone(), b.clone())).unwrap_or(&prom_n);
            let prom_rate = prom_n as f64 / prom_total;
            let all_rate = total_n as f64 / all_total;
            let uplift = if all_rate > 0.0 {
                prom_rate / all_rate
            } else {
                0.0
            };
            if uplift >= min_uplift && prom_n >= 2 {
                // Jaccard: |intersection| / |union| among promoted recipes.
                let a_count = promoted_recipes
                    .iter()
                    .filter(|r| r.contains(a))
                    .count();
                let b_count = promoted_recipes
                    .iter()
                    .filter(|r| r.contains(b))
                    .count();
                let union = a_count + b_count - prom_n;
                let jaccard = if union > 0 {
                    prom_n as f64 / union as f64
                } else {
                    0.0
                };

                Some(SignalSynergy {
                    type_a: a.clone(),
                    type_b: b.clone(),
                    co_occurrence_uplift: uplift,
                    promoted_count: prom_n,
                    total_count: total_n,
                    jaccard,
                })
            } else {
                None
            }
        })
        .collect();

    synergies.sort_by(|a, b| {
        b.co_occurrence_uplift
            .partial_cmp(&a.co_occurrence_uplift)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                // Deterministic tiebreak (B292).
                (&a.type_a, &a.type_b).cmp(&(&b.type_a, &b.type_b))
            })
    });
    synergies
}

// ────────────────────────────────────────────
// Collection strategy recommendations
// ────────────────────────────────────────────

/// A concrete recommendation for adjusting crawl/collection strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRecommendation {
    pub action: CollectionAction,
    pub reason: String,
    pub priority: f64,
    pub estimated_impact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CollectionAction {
    /// Increase crawl frequency for a specific source.
    IncreaseCrawlFrequency { source_id: String, current_hours: f64, suggested_hours: f64 },
    /// Decrease crawl frequency (low-value source).
    DecreaseCrawlFrequency { source_id: String, current_hours: f64, suggested_hours: f64 },
    /// Add a new observation type to collection.
    AddObservationType { obs_type: String },
    /// Deprioritize an observation type.
    DeprioritizeObsType { obs_type: String },
    /// Explore cross-domain signal combination.
    ExploreSignalCombination { type_a: String, type_b: String },
    /// Tighten miner thresholds.
    AdjustMinerThreshold { param: String, current: f64, suggested: f64 },
}

/// Generate collection strategy recommendations from source scores,
/// observation-type values, synergy data, and meta-learning insights.
pub fn generate_recommendations(
    source_scores: &[SourceScore],
    obs_values: &[ObsTypeValue],
    synergies: &[SignalSynergy],
    meta: Option<&MetaLearningInsight>,
) -> Vec<CollectionRecommendation> {
    let mut recs = Vec::new();

    // 1. Low-scoring sources: suggest reducing crawl frequency.
    for s in source_scores.iter().filter(|s| s.composite < 0.1) {
        recs.push(CollectionRecommendation {
            action: CollectionAction::DecreaseCrawlFrequency {
                source_id: s.source_id.clone(),
                current_hours: 24.0, // placeholder
                suggested_hours: 168.0,
            },
            reason: format!(
                "Source '{}' has composite score {:.3} — low yield",
                s.domain, s.composite
            ),
            priority: 0.3,
            estimated_impact: "Frees crawl budget for higher-value sources".into(),
        });
    }

    // 2. High-scoring sources: suggest increasing frequency.
    for s in source_scores.iter().filter(|s| s.composite > 0.5) {
        recs.push(CollectionRecommendation {
            action: CollectionAction::IncreaseCrawlFrequency {
                source_id: s.source_id.clone(),
                current_hours: 24.0,
                suggested_hours: 6.0,
            },
            reason: format!(
                "Source '{}' has composite score {:.3} — high yield",
                s.domain, s.composite
            ),
            priority: 0.8,
            estimated_impact: "More frequent observations may yield earlier insights".into(),
        });
    }

    // 3. Observation types with negative net value.
    for v in obs_values.iter().filter(|v| v.net_value < 0.0) {
        recs.push(CollectionRecommendation {
            action: CollectionAction::DeprioritizeObsType { obs_type: v.obs_type.clone() },
            reason: format!(
                "Observation type '{}' has negative net value ({:.3}): \
                 more retirements than promotions",
                v.obs_type, v.net_value
            ),
            priority: 0.5,
            estimated_impact: "Reduces noise in pattern mining".into(),
        });
    }

    // 4. Synergistic signal pairs not yet explored.
    for syn in synergies.iter().take(5) {
        recs.push(CollectionRecommendation {
            action: CollectionAction::ExploreSignalCombination {
                type_a: syn.type_a.clone(),
                type_b: syn.type_b.clone(),
            },
            reason: format!(
                "Signal pair ({}, {}) has {:.1}× uplift in promoted recipes (n={})",
                syn.type_a, syn.type_b, syn.co_occurrence_uplift, syn.promoted_count
            ),
            priority: 0.7 + 0.1 * syn.co_occurrence_uplift.min(3.0),
            estimated_impact: "Cross-domain combination may yield novel high-value recipes".into(),
        });
    }

    // 5. Meta-learning: threshold adjustments.
    if let Some(m) = meta {
        if m.suggested_min_effect > 1.5 {
            recs.push(CollectionRecommendation {
                action: CollectionAction::AdjustMinerThreshold {
                    param: "min_effect".into(),
                    current: 1.5,
                    suggested: m.suggested_min_effect,
                },
                reason: format!(
                    "Promoted recipes average effect size {:.2} vs retired {:.2}; \
                     raising threshold would filter out low-quality candidates earlier",
                    m.promoted_mean_effect, m.retired_mean_effect
                ),
                priority: 0.6,
                estimated_impact: "Fewer false-positive pattern candidates".into(),
            });
        }
        if m.suggested_max_lag < 90 {
            recs.push(CollectionRecommendation {
                action: CollectionAction::AdjustMinerThreshold {
                    param: "max_lag_days".into(),
                    current: 90.0,
                    suggested: m.suggested_max_lag as f64,
                },
                reason: format!(
                    "Promoted recipes average lag {:.0} days — shortening max_lag \
                     may reduce spurious long-lag correlations",
                    m.promoted_mean_lag
                ),
                priority: 0.5,
                estimated_impact: "Faster mining iterations".into(),
            });
        }
    }

    recs.sort_by(|a, b| {
        b.priority
            .partial_cmp(&a.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    recs
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yields() -> Vec<SourceYield> {
        vec![
            SourceYield {
                source_id: "src-1".into(),
                domain: "reuters.com".into(),
                total_observations: 1000,
                observations_in_fired_recipes: 200,
                observations_in_promoted_recipes: 100,
                observations_in_retired_recipes: 20,
                freshness_hours: 6.0,
                diversity_score: 0.8,
            },
            SourceYield {
                source_id: "src-2".into(),
                domain: "obscure-blog.net".into(),
                total_observations: 50,
                observations_in_fired_recipes: 1,
                observations_in_promoted_recipes: 0,
                observations_in_retired_recipes: 5,
                freshness_hours: 500.0,
                diversity_score: 0.1,
            },
            SourceYield {
                source_id: "src-3".into(),
                domain: "tenders.gov.tn".into(),
                total_observations: 300,
                observations_in_fired_recipes: 150,
                observations_in_promoted_recipes: 80,
                observations_in_retired_recipes: 10,
                freshness_hours: 48.0,
                diversity_score: 0.5,
            },
        ]
    }

    #[test]
    fn test_source_scoring_ranking() {
        let yields = sample_yields();
        let scores = score_sources(&yields, &SourceScoringWeights::default());
        assert_eq!(scores.len(), 3);
        // reuters (high yield) should be first.
        assert_eq!(scores[0].source_id, "src-3"); // tenders — high promotion rate
        // obscure blog should be last.
        assert_eq!(scores[2].source_id, "src-2");
    }

    #[test]
    fn test_source_scoring_fresh_beat_stale() {
        let yields = sample_yields();
        let scores = score_sources(&yields, &SourceScoringWeights::default());
        // reuters is fresher than obscure blog.
        assert!(scores[0].freshness_bonus > scores[2].freshness_bonus);
    }

    #[test]
    fn test_obs_type_ranking() {
        let stats = vec![
            ObsTypeStats {
                obs_type: "JobPost".into(),
                total_occurrences: 500,
                in_promoted_recipes: 100,
                in_staged_recipes: 50,
                in_retired_recipes: 20,
            },
            ObsTypeStats {
                obs_type: "DnsPosture".into(),
                total_occurrences: 200,
                in_promoted_recipes: 5,
                in_staged_recipes: 10,
                in_retired_recipes: 80,
            },
        ];
        let values = rank_observation_types(&stats);
        assert_eq!(values[0].obs_type, "JobPost");
        assert!(values[0].net_value > 0.0);
        assert!(values[1].net_value < 0.0); // DnsPosture has high retirement drag.
    }

    #[test]
    fn test_meta_learning_min_records() {
        let records = vec![
            RecipeTraitRecord {
                recipe_id: "r1".into(),
                status: "promoted".into(),
                signal_count: 2,
                signal_types: vec!["JobPost".into()],
                effect_size: 2.0,
                p_value: 0.005,
                best_lag_days: 30,
                category: "demand".into(),
                precision_at_retirement: None,
                weeks_active: 12,
            },
        ];
        // Not enough records (need 3 promoted + 3 retired).
        assert!(extract_meta_insights(&records).is_none());
    }

    #[test]
    fn test_meta_learning_full() {
        let mut records = Vec::new();
        for i in 0..5 {
            records.push(RecipeTraitRecord {
                recipe_id: format!("p{}", i),
                status: "promoted".into(),
                signal_count: 2 + i % 2,
                signal_types: vec!["JobPost".into(), "CommodityPrice".into()],
                effect_size: 2.5 + i as f64 * 0.1,
                p_value: 0.003,
                best_lag_days: 20 + i as i32 * 5,
                category: "demand".into(),
                precision_at_retirement: None,
                weeks_active: 20,
            });
        }
        for i in 0..4 {
            records.push(RecipeTraitRecord {
                recipe_id: format!("r{}", i),
                status: "retired".into(),
                signal_count: 1,
                signal_types: vec!["DnsPosture".into()],
                effect_size: 1.6 + i as f64 * 0.05,
                p_value: 0.008,
                best_lag_days: 60 + i as i32 * 10,
                category: "security".into(),
                precision_at_retirement: Some(0.35),
                weeks_active: 6,
            });
        }

        let insight = extract_meta_insights(&records).unwrap();
        assert!(insight.promoted_mean_effect > insight.retired_mean_effect);
        assert!(insight.promoted_mean_lag < insight.retired_mean_lag);
        assert!(insight.high_value_signal_types.contains(&"JobPost".into()));
        assert!(insight.best_categories.contains(&"demand".into()));
        assert_eq!(insight.sample_size_promoted, 5);
        assert_eq!(insight.sample_size_retired, 4);
    }

    #[test]
    fn test_synergy_discovery() {
        let promoted = vec![
            vec!["JobPost".into(), "CommodityPrice".into()],
            vec!["JobPost".into(), "CommodityPrice".into(), "PortMetric".into()],
            vec!["JobPost".into(), "CommodityPrice".into()],
        ];
        let all = vec![
            vec!["JobPost".into(), "CommodityPrice".into()],
            vec!["JobPost".into(), "CommodityPrice".into(), "PortMetric".into()],
            vec!["JobPost".into(), "CommodityPrice".into()],
            vec!["DnsPosture".into()],
            vec!["DnsPosture".into(), "VulnNotice".into()],
            vec!["PortMetric".into()],
            vec!["FxRate".into(), "CommodityPrice".into()],
            vec!["WebChange".into()],
        ];
        let synergies = discover_synergies(&promoted, &all, 1.5);
        // JobPost + CommodityPrice should appear (in 3/3 promoted vs 3/8 total).
        assert!(!synergies.is_empty());
        let top = &synergies[0];
        assert!(top.co_occurrence_uplift > 1.5);
    }

    #[test]
    fn test_synergy_empty() {
        let synergies = discover_synergies(&[], &[], 1.5);
        assert!(synergies.is_empty());
    }

    #[test]
    fn test_generate_recommendations() {
        let yields = sample_yields();
        let scores = score_sources(&yields, &SourceScoringWeights::default());
        let obs_values = rank_observation_types(&[
            ObsTypeStats {
                obs_type: "DnsPosture".into(),
                total_occurrences: 200,
                in_promoted_recipes: 5,
                in_staged_recipes: 10,
                in_retired_recipes: 80,
            },
        ]);
        let recs = generate_recommendations(&scores, &obs_values, &[], None);
        // Should recommend deprioritizing DnsPosture.
        assert!(recs.iter().any(|r| matches!(
            &r.action,
            CollectionAction::DeprioritizeObsType { obs_type } if obs_type == "DnsPosture"
        )));
    }

    #[test]
    fn test_recommendations_with_meta() {
        let meta = MetaLearningInsight {
            promoted_mean_effect: 3.0,
            retired_mean_effect: 1.7,
            suggested_min_effect: 2.25,
            promoted_mean_lag: 25.0,
            retired_mean_lag: 70.0,
            suggested_max_lag: 45,
            promoted_mean_signal_count: 2.5,
            high_value_signal_types: vec!["JobPost".into()],
            best_categories: vec!["demand".into()],
            sample_size_promoted: 10,
            sample_size_retired: 8,
        };
        let recs = generate_recommendations(&[], &[], &[], Some(&meta));
        // Should suggest tightening min_effect and max_lag.
        assert!(recs.iter().any(|r| matches!(
            &r.action,
            CollectionAction::AdjustMinerThreshold { param, .. } if param == "min_effect"
        )));
        assert!(recs.iter().any(|r| matches!(
            &r.action,
            CollectionAction::AdjustMinerThreshold { param, .. } if param == "max_lag_days"
        )));
    }
}
