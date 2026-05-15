//! Stats-pipeline bridge for the recipe engine.
//!
//! Converts raw time-series observations into recipe-ready [`FeatureMap`] keys
//! by routing them through the unified stats pipeline.  Callers build a
//! [`StatsEnrichmentInput`] from whatever entity data is available, call
//! [`enrich_features`], and merge the returned [`FeatureMap`] slice into their
//! existing feature map before invoking [`RecipeEngine::evaluate_all`].
//!
//! # Key guarantees
//! - Zero I/O: all computation is pure / deterministic.
//! - Additive: returned keys all start with `stats.` so they cannot collide
//!   with existing observation-derived keys.
//! - Graceful: insufficient data produces empty maps (no panics).

pub use apex_stats::calibration::{
    alert_score_rank_correlation, calibration_curve, fit_best_alert_calibration_model,
    AlertCalibrationModel, CalibrationSample, ReliabilityBin,
};
pub use apex_stats::pipeline::alert_score_from_features;
use apex_stats::pipeline::{
    run_pipeline, run_pipeline_with_calibration, StatsPipelineInput, StatsPipelineResult,
};
use std::collections::HashMap;
use tracing::debug;

use crate::engine::FeatureMap;

// ────────────────────────────────────────────
// Public input type
// ────────────────────────────────────────────

/// Subset of entity data needed to run the stats enrichment pass.
///
/// Callers should populate as many fields as possible — missing fields simply
/// skip the corresponding pipeline stage (graceful degradation is built-in to
/// [`StatsPipelineInput::default`]).
#[derive(Debug, Clone, Default)]
pub struct StatsEnrichmentInput {
    /// Entity identifier (company UUID as string, or POI UUID).
    pub entity_id: String,

    /// Primary numeric time series (e.g., weekly observation counts).
    /// At least 4 points are needed for changepoint / anomaly detection.
    pub primary_series: Vec<f64>,

    /// Secondary time series for cross-correlation / mutual-information checks.
    pub secondary_series: Option<Vec<f64>>,

    /// Prior probability that this entity is a risk entity (used for Bayesian update).
    /// If omitted defaults to `0.5` (uninformative).
    pub prior_prob: Option<f64>,

    /// New evidence signals: (likelihood_ratio, weight).
    /// A likelihood_ratio > 1.0 means the observation is more likely under H1 (risk).
    pub evidence_signals: Vec<(f64, f64)>,

    /// Flat graph edges from this entity (neighbour IDs for risk propagation).
    pub graph_edges: HashMap<String, Vec<String>>,

    /// Initial risk scores for nodes in the above graph (keyed by entity ID).
    pub graph_node_scores: HashMap<String, f64>,

    /// P-values to correct for multiple comparisons via Benjamini-Hochberg.
    pub p_values: Vec<f64>,

    /// Desired FDR threshold (α), defaults to `0.05`.
    pub fdr_alpha: Option<f64>,

    /// Survival / time-to-event observations (e.g., days between re-observations).
    pub survival_times: Vec<f64>,

    /// Event indicators: 1 = event occurred, 0 = censored.
    pub survival_events: Vec<u8>,

    /// Maximum lag to consider in cross-correlation (defaults to 5).
    pub max_lag: Option<usize>,
}

// ────────────────────────────────────────────
// Enrichment entry point
// ────────────────────────────────────────────

/// Run the full stats pipeline for `input` and return a [`FeatureMap`] that
/// can be merged directly into the recipe engine's existing feature set.
///
/// # Example
/// ```
/// use apex_recipes::stats_enrichment::{enrich_features, StatsEnrichmentInput};
///
/// let mut features = std::collections::HashMap::new();
/// features.insert("JobPost.count".to_string(), 12.0);
///
/// let stats_input = StatsEnrichmentInput {
///     entity_id: "company-abc".to_string(),
///     primary_series: (0..20).map(|i| i as f64).collect(),
///     ..Default::default()
/// };
/// let extra = enrich_features(&stats_input);
/// features.extend(extra);
/// ```
pub fn analyze(input: &StatsEnrichmentInput) -> StatsPipelineResult {
    analyze_with_calibration(input, None)
}

pub fn analyze_with_calibration(
    input: &StatsEnrichmentInput,
    calibration_model: Option<&AlertCalibrationModel>,
) -> StatsPipelineResult {
    if input.entity_id.is_empty() {
        return StatsPipelineResult {
            entity_id: String::new(),
            features: FeatureMap::new(),
            labels: HashMap::new(),
            warnings: Vec::new(),
            alert_score: 0.0,
            alert_probability: 0.0,
            calibration_method: "legacy".to_string(),
            alert_level: apex_stats::pipeline::AlertLevel::None,
        };
    }

    // Build the unified pipeline input.
    let mut pipeline_input = StatsPipelineInput {
        entity_id: input.entity_id.clone(),
        primary_series: input.primary_series.clone(),
        ..Default::default()
    };

    if let Some(ref sec) = input.secondary_series {
        pipeline_input.secondary_series = Some(sec.clone());
    }
    if let Some(prior) = input.prior_prob {
        pipeline_input.bayesian_prior = prior;
    }
    pipeline_input.bayesian_likelihoods = input.evidence_signals.clone();
    pipeline_input.graph_edges = input.graph_edges.clone();
    pipeline_input.graph_node_scores = input.graph_node_scores.clone();
    pipeline_input.p_values = input.p_values.clone();
    if let Some(alpha) = input.fdr_alpha {
        pipeline_input.fdr_alpha = alpha;
    }
    pipeline_input.survival_times = input.survival_times.clone();
    pipeline_input.survival_events = input.survival_events.clone();
    if let Some(lag) = input.max_lag {
        pipeline_input.max_correlation_lag = lag;
    }

    match calibration_model {
        Some(model) => run_pipeline_with_calibration(&pipeline_input, Some(model)),
        None => run_pipeline(&pipeline_input),
    }
}

pub fn enrich_features(input: &StatsEnrichmentInput) -> FeatureMap {
    let result = analyze(input);

    if !result.warnings.is_empty() {
        debug!(
            entity_id = %input.entity_id,
            warning_count = result.warnings.len(),
            "Stats enrichment warnings"
        );
    }

    result.features
}

/// Helper: merge stats-enriched features into an existing feature map,
/// returning the numbers of keys added and overwritten (for diagnostics).
pub fn merge_into(base: &mut FeatureMap, stats: FeatureMap) -> (usize, usize) {
    let mut added = 0usize;
    let mut overwritten = 0usize;
    for (k, v) in stats {
        if base.insert(k, v).is_some() {
            overwritten += 1;
        } else {
            added += 1;
        }
    }
    (added, overwritten)
}

// ────────────────────────────────────────────
// Convenience builder for common patterns
// ────────────────────────────────────────────

/// Build a [`StatsEnrichmentInput`] from a weekly observation count vector.
///
/// This is the most common calling pattern: provide only the count series
/// and let all other stages gracefully no-op.
pub fn from_observation_counts(
    entity_id: impl Into<String>,
    counts: Vec<f64>,
) -> StatsEnrichmentInput {
    StatsEnrichmentInput {
        entity_id: entity_id.into(),
        primary_series: counts,
        ..Default::default()
    }
}

// ────────────────────────────────────────────
// Unit tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;

    #[test]
    fn empty_entity_returns_empty_map() {
        let input = StatsEnrichmentInput::default();
        let result = enrich_features(&input);
        assert!(result.is_empty());
    }

    #[test]
    fn single_point_series_does_not_panic() {
        let input = StatsEnrichmentInput {
            entity_id: "test".to_string(),
            primary_series: vec![42.0],
            ..Default::default()
        };
        let result = enrich_features(&input);
        // Pipeline degrades gracefully; may produce some keys or none
        let _ = result;
    }

    #[test]
    fn sufficient_series_produces_stats_keys() {
        let input = StatsEnrichmentInput {
            entity_id: "test-entity".to_string(),
            primary_series: vec![
                1.0, 2.0, 3.0, 1.5, 2.5, 3.5, 1.0, 5.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0, 5.0,
                3.0, 2.0, 1.0, 4.0,
            ],
            ..Default::default()
        };
        let result = enrich_features(&input);
        // At least anomaly or changepoint keys should appear
        let has_stats_keys = result.keys().any(|k| k.starts_with("stats."));
        assert!(
            has_stats_keys,
            "expected stats.* keys, got: {:?}",
            result.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn merge_into_counts_correctly() {
        let mut base: FeatureMap = [("JobPost.count".to_string(), 5.0)].into_iter().collect();
        let stats: FeatureMap = [
            ("stats.anomaly.ewma_count".to_string(), 2.0),
            ("JobPost.count".to_string(), 7.0), // would overwrite
        ]
        .into_iter()
        .collect();
        let (added, overwritten) = merge_into(&mut base, stats);
        assert_eq!(added, 1);
        assert_eq!(overwritten, 1);
    }
}
