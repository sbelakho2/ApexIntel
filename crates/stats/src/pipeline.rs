//! Unified statistical analysis pipeline.
//!
//! Wires together all stats modules (changepoint, anomaly, correlation,
//! mutual_info, bayesian, graph_risk, fdr, fisher, hazard) into a single
//! coherent analysis pass that produces a [`StatsPipelineResult`] ready to be
//! merged into the recipe engine's [`FeatureMap`].
//!
//! # Design goals
//! - **Single entry point**: callers provide a [`StatsPipelineInput`] and get
//!   back enriched features — no need to call individual modules.
//! - **Graceful degradation**: if any stage has insufficient data it is simply
//!   skipped and a diagnostic is appended to `warnings`.
//! - **Deterministic**: given the same input, the same output is always produced.
//! - **Zero I/O**: no database or network calls inside this module.
//!
//! # Feature key convention
//!
//! All keys follow the pattern `stats.<module>.<metric>`, e.g.:
//! - `stats.changepoint.detected_count`
//! - `stats.changepoint.last_index`
//! - `stats.anomaly.mad_count`
//! - `stats.anomaly.ewma_count`
//! - `stats.bayesian.posterior`
//! - `stats.correlation.max_lagged_r`
//! - `stats.mutual_info.mi`
//! - `stats.graph_risk.propagated`
//! - `stats.hazard.hazard_rate`
//! - `stats.fdr.significant_count`

use crate::{
    anomaly, bayesian, changepoint, correlation, fdr, graph_risk, hazard, mutual_info,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

/// Input to the statistical pipeline stage for a single entity's time series.
#[derive(Debug, Clone)]
pub struct StatsPipelineInput {
    /// Entity identifier (company ID, POI ID, etc.)
    pub entity_id: String,
    /// Primary time series to analyze (observations ordered oldest → newest).
    pub primary_series: Vec<f64>,
    /// Optional secondary series for cross-correlation and MI (same length as primary).
    pub secondary_series: Option<Vec<f64>>,
    /// Evidence items for Bayesian fusion: (p_if_true, p_if_false) per signal.
    pub bayesian_likelihoods: Vec<(f64, f64)>,
    /// Prior probability for Bayesian fusion. Default: 0.05.
    pub bayesian_prior: f64,
    /// P-values for FDR correction (from Fisher exact tests or similar).
    pub p_values: Vec<f64>,
    /// Significance level for FDR. Default: 0.05.
    pub fdr_alpha: f64,
    /// Node risk scores for graph propagation `{node_id: risk_score}`.
    pub graph_node_scores: HashMap<String, f64>,
    /// Graph edges for risk propagation `{from_node: [to_node, ...]}`.
    pub graph_edges: HashMap<String, Vec<String>>,
    /// Survival times for hazard analysis.
    pub survival_times: Vec<f64>,
    /// Event indicators for hazard analysis (1=event, 0=censored).
    pub survival_events: Vec<u8>,
    /// Changepoint detection penalty. Default: 3.0.
    pub changepoint_penalty: f64,
    /// Anomaly MAD threshold. Default: 3.5.
    pub anomaly_mad_threshold: f64,
    /// EWMA alpha for anomaly detection. Default: 0.2.
    pub ewma_alpha: f64,
    /// Maximum lag for cross-correlation. Default: 5.
    pub max_correlation_lag: usize,
}

impl Default for StatsPipelineInput {
    fn default() -> Self {
        Self::new("", vec![])
    }
}

impl StatsPipelineInput {
    pub fn new(entity_id: impl Into<String>, primary_series: Vec<f64>) -> Self {
        Self {
            entity_id: entity_id.into(),
            primary_series,
            secondary_series: None,
            bayesian_likelihoods: Vec::new(),
            bayesian_prior: 0.05,
            p_values: Vec::new(),
            fdr_alpha: 0.05,
            graph_node_scores: HashMap::new(),
            graph_edges: HashMap::new(),
            survival_times: Vec::new(),
            survival_events: Vec::new(),
            changepoint_penalty: 3.0,
            anomaly_mad_threshold: 3.5,
            ewma_alpha: 0.2,
            max_correlation_lag: 5,
        }
    }
}

/// Outcome of the statistical pipeline for a single entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsPipelineResult {
    pub entity_id: String,
    /// Computed features keyed by `stats.<module>.<metric>`.
    pub features: HashMap<String, f64>,
    /// Stage-level warnings (e.g. insufficient data).
    pub warnings: Vec<StageWarning>,
    /// Overall statistical alert level: None | Low | Medium | High
    pub alert_level: AlertLevel,
}

/// A warning from a specific pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageWarning {
    pub stage: String,
    pub message: String,
}

/// Statistical alert level derived from the pipeline outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    None,
    Low,
    Medium,
    High,
}

impl std::fmt::Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertLevel::None => write!(f, "none"),
            AlertLevel::Low => write!(f, "low"),
            AlertLevel::Medium => write!(f, "medium"),
            AlertLevel::High => write!(f, "high"),
        }
    }
}

impl StatsPipelineResult {
    fn new(entity_id: String) -> Self {
        Self {
            entity_id,
            features: HashMap::new(),
            warnings: Vec::new(),
            alert_level: AlertLevel::None,
        }
    }

    fn insert(&mut self, key: impl Into<String>, value: f64) {
        self.features.insert(key.into(), value);
    }

    fn warn(&mut self, stage: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(StageWarning {
            stage: stage.into(),
            message: message.into(),
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline runner
// ─────────────────────────────────────────────────────────────────────────────

/// Run the full statistical analysis pipeline.
pub fn run_pipeline(input: &StatsPipelineInput) -> StatsPipelineResult {
    let mut result = StatsPipelineResult::new(input.entity_id.clone());

    // 1. Changepoint detection
    run_changepoint_stage(input, &mut result);

    // 2. Anomaly detection
    run_anomaly_stage(input, &mut result);

    // 3. Cross-correlation (if secondary series available)
    run_correlation_stage(input, &mut result);

    // 4. Mutual information (if secondary series available)
    run_mutual_info_stage(input, &mut result);

    // 5. Bayesian evidence fusion
    run_bayesian_stage(input, &mut result);

    // 6. Graph risk propagation
    run_graph_risk_stage(input, &mut result);

    // 7. FDR correction
    run_fdr_stage(input, &mut result);

    // 8. Hazard / survival analysis
    run_hazard_stage(input, &mut result);

    // Derive overall alert level from combined features
    result.alert_level = derive_alert_level(&result.features);

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Stage implementations
// ─────────────────────────────────────────────────────────────────────────────

fn run_changepoint_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.primary_series.len() < 4 {
        result.warn("changepoint", "Insufficient data (< 4 points)");
        result.insert("stats.changepoint.detected_count", 0.0);
        result.insert("stats.changepoint.last_index", -1.0);
        return;
    }

    let config = changepoint::PeltConfig {
        penalty: input.changepoint_penalty,
        min_segment: 2,
    };

    let cps = changepoint::detect_changepoints(&input.primary_series, &config);
    let count = cps.len() as f64;
    let last_idx = cps.last().map(|&i| i as f64).unwrap_or(-1.0);

    result.insert("stats.changepoint.detected_count", count);
    result.insert("stats.changepoint.last_index", last_idx);

    // Flag if a changepoint was detected in the last 3 observations
    let n = input.primary_series.len();
    let recent_cp = cps.iter().any(|&i| i >= n.saturating_sub(3));
    result.insert("stats.changepoint.recent_shift", if recent_cp { 1.0 } else { 0.0 });

    if count > 0.0 {
        warn!(
            entity = %input.entity_id,
            changepoints = count,
            last_at = last_idx,
            "Changepoints detected"
        );
    }
}

fn run_anomaly_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.primary_series.len() < 3 {
        result.warn("anomaly", "Insufficient data (< 3 points)");
        result.insert("stats.anomaly.mad_count", 0.0);
        result.insert("stats.anomaly.ewma_count", 0.0);
        result.insert("stats.anomaly.max_mad_zscore", 0.0);
        return;
    }

    // MAD anomaly detection
    let mad_anomalies = anomaly::mad_zscore(&input.primary_series, input.anomaly_mad_threshold);
    let mad_count = mad_anomalies.len() as f64;
    let max_zscore = mad_anomalies
        .iter()
        .map(|(_, z)| z.abs())
        .fold(0.0_f64, f64::max);

    result.insert("stats.anomaly.mad_count", mad_count);
    result.insert("stats.anomaly.max_mad_zscore", max_zscore);

    // EWMA anomaly detection (requires >= 10 points)
    if input.primary_series.len() >= 10 {
        let ewma_anomalies = anomaly::ewma_control(&input.primary_series, input.ewma_alpha, 3.0);
        result.insert("stats.anomaly.ewma_count", ewma_anomalies.len() as f64);

        // Check if most recent observation is anomalous
        let n = input.primary_series.len();
        let last_is_anomaly = ewma_anomalies.iter().any(|(i, _)| *i == n - 1) as u8 as f64;
        result.insert("stats.anomaly.latest_is_anomaly", last_is_anomaly);
    } else {
        result.warn("anomaly", "EWMA skipped: < 10 datapoints");
        result.insert("stats.anomaly.ewma_count", 0.0);
        result.insert("stats.anomaly.latest_is_anomaly", 0.0);
    }
}

fn run_correlation_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    let secondary = match &input.secondary_series {
        Some(s) => s,
        None => {
            result.insert("stats.correlation.max_lagged_r", 0.0);
            result.insert("stats.correlation.best_lag", 0.0);
            return;
        }
    };

    if input.primary_series.len() < 5 || secondary.len() < 5 {
        result.warn("correlation", "Insufficient data for cross-correlation (< 5 points)");
        result.insert("stats.correlation.max_lagged_r", 0.0);
        result.insert("stats.correlation.best_lag", 0.0);
        return;
    }

    let max_lag = input.max_correlation_lag.min(input.primary_series.len() / 3);
    let corr_results = correlation::lagged_xcorr(
        &input.primary_series,
        secondary,
        max_lag as i32,
    );

    if corr_results.is_empty() {
        result.warn("correlation", "Cross-correlation returned empty results");
        result.insert("stats.correlation.max_lagged_r", 0.0);
        result.insert("stats.correlation.best_lag", 0.0);
        return;
    }

    let (best_lag, best_r) = corr_results
        .iter()
        .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap_or(std::cmp::Ordering::Equal))
        .copied()
        .unwrap_or((0, 0.0));

    result.insert("stats.correlation.max_lagged_r", best_r.abs());
    result.insert("stats.correlation.best_lag", best_lag as f64);
    result.insert(
        "stats.correlation.lead_or_lag",
        if best_lag > 0 {
            1.0
        } else if best_lag < 0 {
            -1.0
        } else {
            0.0
        },
    );

}

fn run_mutual_info_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    let secondary = match &input.secondary_series {
        Some(s) => s,
        None => {
            result.insert("stats.mutual_info.mi", 0.0);
            return;
        }
    };

    if input.primary_series.len() < 10 || secondary.len() < 10 {
        result.warn("mutual_info", "Insufficient data for MI (< 10 points)");
        result.insert("stats.mutual_info.mi", 0.0);
        return;
    }

    let mi = mutual_info::estimate(&input.primary_series, secondary, 10);
    result.insert("stats.mutual_info.mi", mi);
    // Use the pre-built normalized_mi directly
    let nmi = mutual_info::normalized_mi(&input.primary_series, secondary, 10);
    result.insert("stats.mutual_info.normalized_mi", nmi.clamp(0.0, 1.0));
}

fn run_bayesian_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.bayesian_likelihoods.is_empty() {
        result.insert("stats.bayesian.posterior", input.bayesian_prior);
        return;
    }

    let posterior = bayesian::fuse_signals(input.bayesian_prior, &input.bayesian_likelihoods);
    let lift = posterior / input.bayesian_prior.max(1e-9);

    result.insert("stats.bayesian.posterior", posterior);
    result.insert("stats.bayesian.lift", lift.clamp(0.0, 1000.0));
    result.insert(
        "stats.bayesian.is_significant",
        if posterior >= 0.5 { 1.0 } else { 0.0 },
    );
}

fn run_graph_risk_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.graph_node_scores.is_empty() {
        result.insert("stats.graph_risk.propagated", 0.0);
        result.insert("stats.graph_risk.max_node_risk", 0.0);
        return;
    }

    // Build adjacency list from graph_edges (equal weight 1.0 per edge)
    let adjacency: std::collections::HashMap<String, Vec<(String, f64)>> = input
        .graph_edges
        .iter()
        .map(|(from, tos)| (from.clone(), tos.iter().map(|t| (t.clone(), 1.0_f64)).collect()))
        .collect();
    let propagated = graph_risk::propagate(&adjacency, &input.graph_node_scores, 3, 0.5);
    let max_risk = propagated
        .values()
        .copied()
        .fold(0.0_f64, f64::max);
    let entity_risk = propagated
        .get(&input.entity_id)
        .copied()
        .unwrap_or(0.0);

    result.insert("stats.graph_risk.propagated", entity_risk);
    result.insert("stats.graph_risk.max_node_risk", max_risk);
    result.insert("stats.graph_risk.network_size", propagated.len() as f64);
}

fn run_fdr_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.p_values.is_empty() {
        result.insert("stats.fdr.significant_count", 0.0);
        return;
    }

    let adjusted = fdr::bh_correct(&input.p_values);
    let significant_count = adjusted
        .iter()
        .filter(|&&p| p < input.fdr_alpha)
        .count() as f64;
    let proportion = significant_count / (input.p_values.len() as f64);

    result.insert("stats.fdr.significant_count", significant_count);
    result.insert("stats.fdr.significant_proportion", proportion);

    // Min p-value gives strongest individual signal
    let min_p = input.p_values.iter().cloned().fold(f64::MAX, f64::min);
    result.insert("stats.fdr.min_p_value", min_p);
}

fn run_hazard_stage(input: &StatsPipelineInput, result: &mut StatsPipelineResult) {
    if input.survival_times.is_empty() || input.survival_events.is_empty() {
        result.insert("stats.hazard.hazard_rate", 0.0);
        return;
    }

    if input.survival_times.len() != input.survival_events.len() {
        result.warn("hazard", "survival_times and survival_events length mismatch");
        result.insert("stats.hazard.hazard_rate", 0.0);
        return;
    }

    // Build (time, event_occurred) pairs for kaplan_meier
    let paired: Vec<(f64, bool)> = input
        .survival_times
        .iter()
        .zip(input.survival_events.iter())
        .map(|(&t, &e)| (t, e == 1))
        .collect();
    let km_curve = hazard::kaplan_meier(&paired);
    let cum_h = hazard::cumulative_hazard(&km_curve);

    let mean_hazard = if cum_h.is_empty() {
        0.0
    } else {
        cum_h.last().map(|(_, h)| *h).unwrap_or(0.0)
    };
    let mean_hazard = if mean_hazard.is_finite() { mean_hazard } else { 0.0 };
    let max_hazard = cum_h
        .iter()
        .map(|(_, h)| *h)
        .filter(|h| h.is_finite())
        .fold(0.0_f64, f64::max);

    result.insert("stats.hazard.hazard_rate", mean_hazard);
    result.insert("stats.hazard.max_hazard", max_hazard);
    result.insert("stats.hazard.event_count", input.survival_events.iter().filter(|&&e| e == 1).count() as f64);
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert level derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Combine stats pipeline features into a single alert level.
fn derive_alert_level(features: &HashMap<String, f64>) -> AlertLevel {
    let mut score = 0.0_f64;

    // Changepoints are strong signals
    if features.get("stats.changepoint.recent_shift").copied().unwrap_or(0.0) > 0.5 {
        score += 2.0;
    }
    // Any detected changepoint is meaningful; 2+ indicates persistent instability
    if features.get("stats.changepoint.detected_count").copied().unwrap_or(0.0) >= 1.0 {
        score += 1.0;
    }
    if features.get("stats.changepoint.detected_count").copied().unwrap_or(0.0) >= 2.0 {
        score += 0.5;
    }

    // Recent MAD anomaly
    if features.get("stats.anomaly.latest_is_anomaly").copied().unwrap_or(0.0) > 0.5 {
        score += 2.0;
    }
    if features.get("stats.anomaly.mad_count").copied().unwrap_or(0.0) >= 2.0 {
        score += 1.0;
    }

    // Strong Bayesian posterior
    let posterior = features.get("stats.bayesian.posterior").copied().unwrap_or(0.0);
    if posterior >= 0.8 { score += 2.0; }
    else if posterior >= 0.5 { score += 1.0; }

    // High graph risk
    if features.get("stats.graph_risk.propagated").copied().unwrap_or(0.0) >= 0.7 {
        score += 1.5;
    }

    // FDR-significant signals
    if features.get("stats.fdr.significant_count").copied().unwrap_or(0.0) >= 2.0 {
        score += 1.0;
    }

    // High MI / correlation (leading indicators)
    if features.get("stats.mutual_info.normalized_mi").copied().unwrap_or(0.0) >= 0.5 {
        score += 0.5;
    }

    match score as u32 {
        0 => AlertLevel::None,
        1..=2 => AlertLevel::Low,
        3..=4 => AlertLevel::Medium,
        _ => AlertLevel::High,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(n: usize) -> StatsPipelineInput {
        let primary: Vec<f64> = (0..n).map(|i| i as f64 + (i % 3) as f64 * 0.5).collect();
        StatsPipelineInput::new("entity-001", primary)
    }

    #[test]
    fn pipeline_runs_with_minimal_data() {
        let input = make_input(3);
        let result = run_pipeline(&input);
        assert_eq!(result.entity_id, "entity-001");
        assert!(result.features.contains_key("stats.changepoint.detected_count"));
        assert!(result.features.contains_key("stats.anomaly.mad_count"));
        assert!(result.features.contains_key("stats.bayesian.posterior"));
    }

    #[test]
    fn pipeline_with_full_data_produces_all_features() {
        let mut input = make_input(30);
        input.secondary_series = Some((0..30_usize).map(|i| (i as f64 * 1.1).sin()).collect());
        input.bayesian_likelihoods = vec![(0.8, 0.2), (0.7, 0.3)];
        input.p_values = vec![0.001, 0.03, 0.04, 0.1, 0.5];
        input.survival_times = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        input.survival_events = vec![1, 1, 0, 1, 0];

        let result = run_pipeline(&input);

        // Should have features from all stages
        assert!(result.features.contains_key("stats.changepoint.detected_count"));
        assert!(result.features.contains_key("stats.anomaly.ewma_count"));
        assert!(result.features.contains_key("stats.correlation.max_lagged_r"));
        assert!(result.features.contains_key("stats.mutual_info.mi"));
        assert!(result.features.contains_key("stats.bayesian.posterior"));
        assert!(result.features.contains_key("stats.fdr.significant_count"));
        assert!(result.features.contains_key("stats.hazard.hazard_rate"));
    }

    #[test]
    fn alert_level_none_for_flat_noise() {
        let primary: Vec<f64> = (0..20).map(|i| 1.0 + (i % 2) as f64 * 0.001).collect();
        let input = StatsPipelineInput::new("e1", primary);
        let result = run_pipeline(&input);
        // Flat series with micro-noise should not be High alert
        assert!(
            result.alert_level != AlertLevel::High,
            "Flat series should not be High alert"
        );
    }

    #[test]
    fn alert_level_high_for_sudden_shift() {
        // Step function — major changepoint, anomalies
        let mut primary: Vec<f64> = vec![1.0; 15];
        primary.extend(vec![100.0; 15]);
        let mut input = StatsPipelineInput::new("e2", primary);
        input.bayesian_likelihoods = vec![(0.95, 0.05), (0.9, 0.1)];
        let result = run_pipeline(&input);
        assert!(
            result.alert_level == AlertLevel::High || result.alert_level == AlertLevel::Medium,
            "Sudden step shift should be Medium or High: {:?}", result.alert_level
        );
    }

    #[test]
    fn pipeline_tolerates_empty_optional_fields() {
        let input = StatsPipelineInput::new("e3", vec![1.0, 2.0, 3.0]);
        let result = run_pipeline(&input);
        // Should not panic, should have zero-valued features for skipped stages
        assert_eq!(result.features.get("stats.correlation.max_lagged_r"), Some(&0.0));
        assert_eq!(result.features.get("stats.mutual_info.mi"), Some(&0.0));
        assert_eq!(result.features.get("stats.hazard.hazard_rate"), Some(&0.0));
    }

    #[test]
    fn derive_alert_none_for_empty_features() {
        let level = derive_alert_level(&HashMap::new());
        assert_eq!(level, AlertLevel::None);
    }

    #[test]
    fn derive_alert_high_for_many_signals() {
        let mut features = HashMap::new();
        features.insert("stats.changepoint.recent_shift".into(), 1.0);
        features.insert("stats.changepoint.detected_count".into(), 3.0);
        features.insert("stats.anomaly.latest_is_anomaly".into(), 1.0);
        features.insert("stats.anomaly.mad_count".into(), 3.0);
        features.insert("stats.bayesian.posterior".into(), 0.9);
        let level = derive_alert_level(&features);
        assert_eq!(level, AlertLevel::High);
    }
}
