//! Triage configuration — weights, thresholds, LLM routing, budget tracking.

use apex_core::triage::{TriageThresholds, TriageWeights};
use apex_llm::{ModelConfig, SpendTracker};
use serde::{Deserialize, Serialize};

/// Configuration for the triage engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageConfig {
    /// Task name for LLM routing (used in RoutingConfig)
    pub llm_task_name: String,
    /// Model config override (None = use lightweight from LlmConfig)
    pub model_config: Option<ModelConfig>,
    /// Weights for composite score
    pub weights: TriageWeights,
    /// Thresholds for severity bands
    pub thresholds: TriageThresholds,
    /// Batch size for batched triage processing
    pub batch_size: usize,
    /// Minimum composite score to trigger alert routing
    pub alert_threshold: f64,
    /// Whether to enable embedding-based dedup pre-filtering
    pub enable_semantic_dedup: bool,
    /// Similarity threshold for considering items as duplicates
    pub dedup_similarity_threshold: f64,
    /// Whether triage is local-only (data never leaves premises)
    pub local_only: bool,
}

impl Default for TriageConfig {
    fn default() -> Self {
        Self {
            llm_task_name: "triage_scoring".to_string(),
            model_config: None,
            weights: TriageWeights::default(),
            thresholds: TriageThresholds::default(),
            batch_size: 10,
            alert_threshold: 0.7,
            enable_semantic_dedup: true,
            dedup_similarity_threshold: 0.85,
            local_only: true,
        }
    }
}

impl TriageConfig {
    /// Load configuration from environment variables.
    ///
    /// Variables:
    /// - `TRIAGE_BATCH_SIZE` (default: 10)
    /// - `TRIAGE_ALERT_THRESHOLD` (default: 0.7)
    /// - `TRIAGE_ENABLE_DEDUP` (default: true)
    /// - `TRIAGE_DEDUP_THRESHOLD` (default: 0.85)
    /// - `TRIAGE_LOCAL_ONLY` (default: true)
    /// - `TRIAGE_LLM_TASK_NAME` (default: "triage_scoring")
    pub fn from_env() -> Self {
        Self {
            batch_size: std::env::var("TRIAGE_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            alert_threshold: std::env::var("TRIAGE_ALERT_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.7),
            enable_semantic_dedup: std::env::var("TRIAGE_ENABLE_DEDUP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            dedup_similarity_threshold: std::env::var("TRIAGE_DEDUP_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.85),
            local_only: std::env::var("TRIAGE_LOCAL_ONLY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            llm_task_name: std::env::var("TRIAGE_LLM_TASK_NAME")
                .unwrap_or_else(|_| "triage_scoring".to_string()),
            ..Default::default()
        }
    }

    /// Load configuration from a YAML config file.
    pub fn from_yaml(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        let config: TriageConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}

/// Tracks LLM API spend for triage operations.
///
/// Each triage LLM call costs ~200-400 tokens at local inference (free).
/// If cloud fallback is enabled, calls use GPT-4o at ~$0.01-0.02 per 10-item batch.
pub struct TriageSpendTracker {
    inner: SpendTracker,
    /// Track number of triage calls made
    call_count: u64,
}

impl TriageSpendTracker {
    /// Create a new spend tracker with the given monthly budget in USD.
    pub fn new(budget_usd: f64) -> Self {
        Self {
            inner: SpendTracker::new(budget_usd),
            call_count: 0,
        }
    }

    /// Record a triage LLM call with estimated cost.
    pub fn record_call(&mut self, estimated_cost_usd: f64) {
        self.inner.record(estimated_cost_usd);
        self.call_count += 1;
    }

    /// Check if the triage engine is within budget.
    pub fn within_budget(&self) -> bool {
        self.inner.within_budget()
    }

    /// Get the number of triage calls made.
    pub fn call_count(&self) -> u64 {
        self.call_count
    }

    /// Get remaining budget in USD.
    pub fn remaining_budget(&self) -> f64 {
        self.inner.remaining_budget()
    }

    /// Get total spend in USD.
    pub fn total_spend(&self) -> f64 {
        self.inner.month_spend()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triage_config_default() {
        let config = TriageConfig::default();
        assert_eq!(config.llm_task_name, "triage_scoring");
        assert_eq!(config.batch_size, 10);
        assert!((config.alert_threshold - 0.7).abs() < 1e-9);
        assert!(config.enable_semantic_dedup);
        assert!(config.local_only);
        assert!((config.weights.urgency - 0.30).abs() < 1e-9);
        assert!((config.thresholds.critical - 0.80).abs() < 1e-9);
    }

    #[test]
    fn test_triage_spend_tracker() {
        let mut tracker = TriageSpendTracker::new(100.0);
        assert!(tracker.within_budget());
        assert_eq!(tracker.call_count(), 0);

        tracker.record_call(0.02);
        assert!(tracker.within_budget());
        assert_eq!(tracker.call_count(), 1);

        tracker.record_call(99.99);
        assert!(!tracker.within_budget());
        assert_eq!(tracker.call_count(), 2);
    }

    #[test]
    fn test_triage_config_from_env() {
        // Set env vars, then verify from_env picks them up
        std::env::set_var("TRIAGE_BATCH_SIZE", "20");
        std::env::set_var("TRIAGE_ALERT_THRESHOLD", "0.8");
        std::env::set_var("TRIAGE_ENABLE_DEDUP", "false");

        let config = TriageConfig::from_env();
        assert_eq!(config.batch_size, 20);
        assert!((config.alert_threshold - 0.8).abs() < 1e-9);
        assert!(!config.enable_semantic_dedup);

        // Clean up
        std::env::remove_var("TRIAGE_BATCH_SIZE");
        std::env::remove_var("TRIAGE_ALERT_THRESHOLD");
        std::env::remove_var("TRIAGE_ENABLE_DEDUP");
    }
}
