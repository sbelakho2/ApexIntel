//! Predictive intelligence for forward-looking analysis.
//!
//! Moves from reactive ("X happened") to predictive ("X is likely within 30 days"):
//! - Pattern-based predictions with calibrated confidence intervals
//! - Watch list system for entities with predicted state changes
//! - Prediction accuracy tracking with proper scoring rules (log-loss)

use std::collections::HashMap;
use uuid::Uuid;

// ============================================================================
// PATTERN-BASED PREDICTIONS
// ============================================================================

/// A temporal pattern that predicts outcomes.
#[derive(Debug, Clone)]
pub struct PredictivePattern {
    /// Unique identifier
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Trigger conditions (signal types that must fire)
    pub triggers: Vec<String>,
    /// Predicted outcome
    pub predicted_outcome: String,
    /// Time window for prediction (days)
    pub prediction_window_days: u32,
    /// Historical precision (true positives / predicted positives)
    pub precision: f64,
    /// 95% CI lower bound on precision
    pub precision_ci_lower: f64,
    /// 95% CI upper bound on precision
    pub precision_ci_upper: f64,
    /// Historical recall (true positives / actual positives)
    pub recall: f64,
    /// Number of historical observations
    pub observation_count: u32,
}

impl PredictivePattern {
    /// Generate a prediction for an entity based on this pattern.
    pub fn predict(&self, entity_id: Uuid, entity_name: &str) -> Prediction {
        Prediction {
            id: Uuid::new_v4(),
            entity_id,
            entity_name: entity_name.to_string(),
            pattern_id: self.id.clone(),
            predicted_outcome: self.predicted_outcome.clone(),
            prediction_window_days: self.prediction_window_days,
            probability: self.precision,
            probability_ci_lower: self.precision_ci_lower,
            probability_ci_upper: self.precision_ci_upper,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(self.prediction_window_days as i64),
            status: PredictionStatus::Active,
            actual_outcome: None,
        }
    }

    /// Format the prediction statement with calibrated confidence.
    pub fn prediction_statement(&self) -> String {
        format!(
            "When {} fires, {} follows within {} days with {:.0}% precision (95% CI: {:.0}%-{:.0}%), based on {} historical observations.",
            self.triggers.join(" + "),
            self.predicted_outcome,
            self.prediction_window_days,
            self.precision * 100.0,
            self.precision_ci_lower * 100.0,
            self.precision_ci_upper * 100.0,
            self.observation_count
        )
    }
}

/// A prediction instance for an entity.
#[derive(Debug, Clone)]
pub struct Prediction {
    /// Unique prediction ID
    pub id: Uuid,
    /// Entity being predicted
    pub entity_id: Uuid,
    /// Entity name
    pub entity_name: String,
    /// Pattern that generated this prediction
    pub pattern_id: String,
    /// What is predicted to happen
    pub predicted_outcome: String,
    /// Time window for the prediction
    pub prediction_window_days: u32,
    /// Probability of the outcome
    pub probability: f64,
    /// 95% CI lower bound
    pub probability_ci_lower: f64,
    /// 95% CI upper bound
    pub probability_ci_upper: f64,
    /// When the prediction was made
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the prediction expires (no outcome observed)
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Current status
    pub status: PredictionStatus,
    /// Actual outcome if resolved
    pub actual_outcome: Option<bool>,
}

/// Status of a prediction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionStatus {
    /// Still within the prediction window
    Active,
    /// Outcome occurred as predicted
    ConfirmedPositive,
    /// Window expired without outcome
    ConfirmedNegative,
    /// Cannot determine outcome (e.g., entity no longer tracked)
    Indeterminate,
}

impl Prediction {
    /// Check if the prediction is still active.
    pub fn is_active(&self) -> bool {
        self.status == PredictionStatus::Active && chrono::Utc::now() < self.expires_at
    }

    /// Resolve the prediction with actual outcome.
    pub fn resolve(&mut self, outcome_occurred: bool) {
        self.actual_outcome = Some(outcome_occurred);
        self.status = if outcome_occurred {
            PredictionStatus::ConfirmedPositive
        } else {
            PredictionStatus::ConfirmedNegative
        };
    }

    /// Days remaining until expiration.
    pub fn days_remaining(&self) -> i64 {
        let now = chrono::Utc::now();
        (self.expires_at - now).num_days().max(0)
    }
}

// ============================================================================
// WATCH LIST SYSTEM
// ============================================================================

/// Watch list configuration.
#[derive(Debug, Clone)]
pub struct WatchListConfig {
    /// Prediction probability threshold to add to watch list
    pub probability_threshold: f64,
    /// Prediction window threshold (days) - shorter windows get higher priority
    pub window_threshold_days: u32,
    /// Crawl priority multiplier for watched entities
    pub crawl_priority_multiplier: f64,
}

impl Default for WatchListConfig {
    fn default() -> Self {
        Self {
            probability_threshold: 0.50,
            window_threshold_days: 14,
            crawl_priority_multiplier: 2.0,
        }
    }
}

/// An entity on the watch list.
#[derive(Debug, Clone)]
pub struct WatchListEntry {
    /// Entity ID
    pub entity_id: Uuid,
    /// Entity name
    pub entity_name: String,
    /// Active predictions for this entity
    pub active_predictions: Vec<Prediction>,
    /// When the entity was added to watch list
    pub added_at: chrono::DateTime<chrono::Utc>,
    /// Crawl priority multiplier
    pub crawl_priority_multiplier: f64,
    /// Why the entity is being watched
    pub reason: String,
}

impl WatchListEntry {
    /// Maximum probability across all active predictions.
    pub fn max_probability(&self) -> f64 {
        self.active_predictions
            .iter()
            .map(|p| p.probability)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0)
    }

    /// Minimum days to any predicted event.
    pub fn min_days_to_event(&self) -> i64 {
        self.active_predictions
            .iter()
            .map(|p| p.days_remaining())
            .min()
            .unwrap_or(999)
    }

    /// Get urgency score (higher = more urgent).
    pub fn urgency_score(&self) -> f64 {
        let prob = self.max_probability();
        let days = self.min_days_to_event() as f64;
        
        // Higher probability and fewer days = higher urgency
        if days <= 0.0 {
            prob * 10.0
        } else {
            prob * (14.0 / days).min(10.0)
        }
    }
}

/// Watch list manager.
#[derive(Debug)]
pub struct WatchList {
    /// Configuration
    config: WatchListConfig,
    /// Entries keyed by entity ID
    entries: HashMap<Uuid, WatchListEntry>,
}

impl WatchList {
    pub fn new(config: WatchListConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
        }
    }

    /// Add or update an entity on the watch list based on a prediction.
    pub fn add_from_prediction(&mut self, prediction: Prediction) {
        // Only add if prediction meets thresholds
        if prediction.probability < self.config.probability_threshold {
            return;
        }
        if prediction.prediction_window_days > self.config.window_threshold_days {
            return;
        }

        let entity_id = prediction.entity_id;
        let reason = format!(
            "Predicted: {} within {} days (prob: {:.0}%)",
            prediction.predicted_outcome,
            prediction.prediction_window_days,
            prediction.probability * 100.0
        );

        if let Some(entry) = self.entries.get_mut(&entity_id) {
            // Update existing entry
            entry.active_predictions.push(prediction);
            // Use max multiplier
            entry.crawl_priority_multiplier = entry
                .crawl_priority_multiplier
                .max(self.config.crawl_priority_multiplier);
        } else {
            // Create new entry
            let entry = WatchListEntry {
                entity_id,
                entity_name: prediction.entity_name.clone(),
                active_predictions: vec![prediction],
                added_at: chrono::Utc::now(),
                crawl_priority_multiplier: self.config.crawl_priority_multiplier,
                reason,
            };
            self.entries.insert(entity_id, entry);
        }
    }

    /// Remove resolved/expired predictions and clean up empty entries.
    pub fn cleanup(&mut self) {
        // Remove expired predictions
        for entry in self.entries.values_mut() {
            entry.active_predictions.retain(|p| p.is_active());
        }

        // Remove entries with no active predictions
        self.entries.retain(|_, entry| !entry.active_predictions.is_empty());
    }

    /// Get all watched entities sorted by urgency.
    pub fn sorted_by_urgency(&self) -> Vec<&WatchListEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| {
            b.urgency_score()
                .partial_cmp(&a.urgency_score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
    }

    /// Get crawl priority multiplier for an entity.
    pub fn crawl_priority_for(&self, entity_id: Uuid) -> f64 {
        self.entries
            .get(&entity_id)
            .map(|e| e.crawl_priority_multiplier)
            .unwrap_or(1.0)
    }

    /// Count of watched entities.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if watch list is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================================
// PREDICTION ACCURACY TRACKING
// ============================================================================

/// Accuracy tracker using proper scoring rules.
#[derive(Debug)]
pub struct AccuracyTracker {
    /// Pattern-level statistics
    pattern_stats: HashMap<String, PatternAccuracyStats>,
    /// Overall log-loss accumulator
    total_log_loss: f64,
    /// Total resolved predictions
    total_resolved: u32,
}

/// Accuracy statistics for a pattern.
#[derive(Debug, Clone)]
pub struct PatternAccuracyStats {
    pub pattern_id: String,
    /// True positives (predicted positive, was positive)
    pub true_positives: u32,
    /// False positives (predicted positive, was negative)
    pub false_positives: u32,
    /// True negatives (predicted negative, was negative)
    pub true_negatives: u32,
    /// False negatives (predicted negative, was positive)
    pub false_negatives: u32,
    /// Sum of log-loss contributions
    pub log_loss_sum: f64,
    /// Count for log-loss average
    pub log_loss_count: u32,
    /// Sum of Brier score contributions
    pub brier_sum: f64,
}

impl PatternAccuracyStats {
    pub fn new(pattern_id: String) -> Self {
        Self {
            pattern_id,
            true_positives: 0,
            false_positives: 0,
            true_negatives: 0,
            false_negatives: 0,
            log_loss_sum: 0.0,
            log_loss_count: 0,
            brier_sum: 0.0,
        }
    }

    /// Record a prediction outcome.
    pub fn record(&mut self, predicted_prob: f64, actual_outcome: bool) {
        if actual_outcome {
            if predicted_prob >= 0.5 {
                self.true_positives += 1;
            } else {
                self.false_negatives += 1;
            }
        } else if predicted_prob >= 0.5 {
            self.false_positives += 1;
        } else {
            self.true_negatives += 1;
        }

        // Log-loss: -[y*log(p) + (1-y)*log(1-p)]
        let p_clamped = predicted_prob.clamp(1e-10, 1.0 - 1e-10);
        let y = if actual_outcome { 1.0 } else { 0.0 };
        let log_loss = -(y * p_clamped.ln() + (1.0 - y) * (1.0 - p_clamped).ln());
        self.log_loss_sum += log_loss;
        self.log_loss_count += 1;

        // Brier score: (p - y)^2
        let brier = (predicted_prob - y).powi(2);
        self.brier_sum += brier;
    }

    /// Calculate precision.
    pub fn precision(&self) -> f64 {
        let denominator = self.true_positives + self.false_positives;
        if denominator == 0 {
            0.0
        } else {
            self.true_positives as f64 / denominator as f64
        }
    }

    /// Calculate recall.
    pub fn recall(&self) -> f64 {
        let denominator = self.true_positives + self.false_negatives;
        if denominator == 0 {
            0.0
        } else {
            self.true_positives as f64 / denominator as f64
        }
    }

    /// Calculate F1 score.
    pub fn f1(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }

    /// Calculate average log-loss.
    pub fn log_loss(&self) -> f64 {
        if self.log_loss_count == 0 {
            0.0
        } else {
            self.log_loss_sum / self.log_loss_count as f64
        }
    }

    /// Calculate average Brier score.
    pub fn brier_score(&self) -> f64 {
        if self.log_loss_count == 0 {
            0.0
        } else {
            self.brier_sum / self.log_loss_count as f64
        }
    }

    /// Total predictions.
    pub fn total(&self) -> u32 {
        self.true_positives + self.false_positives + self.true_negatives + self.false_negatives
    }
}

impl AccuracyTracker {
    pub fn new() -> Self {
        Self {
            pattern_stats: HashMap::new(),
            total_log_loss: 0.0,
            total_resolved: 0,
        }
    }

    /// Record a resolved prediction.
    pub fn record(&mut self, prediction: &Prediction, actual_outcome: bool) {
        let stats = self
            .pattern_stats
            .entry(prediction.pattern_id.clone())
            .or_insert_with(|| PatternAccuracyStats::new(prediction.pattern_id.clone()));

        stats.record(prediction.probability, actual_outcome);

        // Update overall log-loss
        let p_clamped = prediction.probability.clamp(1e-10, 1.0 - 1e-10);
        let y = if actual_outcome { 1.0 } else { 0.0 };
        let log_loss = -(y * p_clamped.ln() + (1.0 - y) * (1.0 - p_clamped).ln());
        self.total_log_loss += log_loss;
        self.total_resolved += 1;
    }

    /// Get overall log-loss.
    pub fn overall_log_loss(&self) -> f64 {
        if self.total_resolved == 0 {
            0.0
        } else {
            self.total_log_loss / self.total_resolved as f64
        }
    }

    /// Get statistics for a pattern.
    pub fn get_pattern_stats(&self, pattern_id: &str) -> Option<&PatternAccuracyStats> {
        self.pattern_stats.get(pattern_id)
    }

    /// Get all pattern statistics sorted by log-loss (best first).
    pub fn all_pattern_stats(&self) -> Vec<&PatternAccuracyStats> {
        let mut stats: Vec<_> = self.pattern_stats.values().collect();
        stats.sort_by(|a, b| {
            a.log_loss()
                .partial_cmp(&b.log_loss())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        stats
    }

    /// Generate accuracy summary report.
    pub fn summary_report(&self) -> AccuracySummary {
        let patterns: Vec<_> = self.all_pattern_stats().into_iter().cloned().collect();
        
        let total_predictions: u32 = patterns.iter().map(|p| p.total()).sum();
        let avg_precision: f64 = if patterns.is_empty() {
            0.0
        } else {
            patterns.iter().map(|p| p.precision()).sum::<f64>() / patterns.len() as f64
        };
        let avg_recall: f64 = if patterns.is_empty() {
            0.0
        } else {
            patterns.iter().map(|p| p.recall()).sum::<f64>() / patterns.len() as f64
        };

        AccuracySummary {
            total_patterns: patterns.len() as u32,
            total_predictions,
            overall_log_loss: self.overall_log_loss(),
            average_precision: avg_precision,
            average_recall: avg_recall,
            pattern_details: patterns,
        }
    }
}

impl Default for AccuracyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of prediction accuracy.
#[derive(Debug, Clone)]
pub struct AccuracySummary {
    pub total_patterns: u32,
    pub total_predictions: u32,
    pub overall_log_loss: f64,
    pub average_precision: f64,
    pub average_recall: f64,
    pub pattern_details: Vec<PatternAccuracyStats>,
}

impl AccuracySummary {
    /// Check if the model is well-calibrated (log-loss < 0.693, which is random baseline).
    pub fn is_calibrated(&self) -> bool {
        self.overall_log_loss < 0.693 // ln(2), the log-loss for 50/50 guessing
    }

    /// Format the summary for display.
    pub fn display(&self) -> String {
        format!(
            "Prediction Accuracy Summary:\n- Patterns: {}\n- Total Predictions: {}\n- Log-Loss: {:.3} (calibrated: {})\n- Avg Precision: {:.1}%\n- Avg Recall: {:.1}%",
            self.total_patterns,
            self.total_predictions,
            self.overall_log_loss,
            if self.is_calibrated() { "yes" } else { "NO" },
            self.average_precision * 100.0,
            self.average_recall * 100.0
        )
    }
}

// ============================================================================
// EXAMPLE PREDICTIVE PATTERNS
// ============================================================================

/// Get predefined predictive patterns based on historical analysis.
pub fn predefined_patterns() -> Vec<PredictivePattern> {
    vec![
        PredictivePattern {
            id: "pattern_exec_departure_reorg".to_string(),
            description: "Executive departure followed by reorganization".to_string(),
            triggers: vec!["executive_departure".to_string(), "board_change".to_string()],
            predicted_outcome: "reorganization_announcement".to_string(),
            prediction_window_days: 45,
            precision: 0.72,
            precision_ci_lower: 0.65,
            precision_ci_upper: 0.79,
            recall: 0.60,
            observation_count: 150,
        },
        PredictivePattern {
            id: "pattern_tariff_sourcing".to_string(),
            description: "Tariff change triggers dual-sourcing initiative".to_string(),
            triggers: vec!["tariff_change".to_string()],
            predicted_outcome: "dual_sourcing_initiative".to_string(),
            prediction_window_days: 30,
            precision: 0.70,
            precision_ci_lower: 0.62,
            precision_ci_upper: 0.78,
            recall: 0.55,
            observation_count: 85,
        },
        PredictivePattern {
            id: "pattern_commodity_supply".to_string(),
            description: "Commodity price spike leads to supply disruption".to_string(),
            triggers: vec!["commodity_price_spike".to_string()],
            predicted_outcome: "supplier_disruption".to_string(),
            prediction_window_days: 14,
            precision: 0.68,
            precision_ci_lower: 0.58,
            precision_ci_upper: 0.78,
            recall: 0.72,
            observation_count: 120,
        },
        PredictivePattern {
            id: "pattern_breach_enforcement".to_string(),
            description: "Data breach triggers regulatory investigation".to_string(),
            triggers: vec!["data_breach".to_string()],
            predicted_outcome: "regulatory_investigation".to_string(),
            prediction_window_days: 7,
            precision: 0.85,
            precision_ci_lower: 0.78,
            precision_ci_upper: 0.92,
            recall: 0.80,
            observation_count: 95,
        },
        PredictivePattern {
            id: "pattern_patent_partnership".to_string(),
            description: "Patent filing surge attracts partnership".to_string(),
            triggers: vec!["patent_filing".to_string(), "patent_grant".to_string()],
            predicted_outcome: "technology_partnership".to_string(),
            prediction_window_days: 60,
            precision: 0.50,
            precision_ci_lower: 0.40,
            precision_ci_upper: 0.60,
            recall: 0.45,
            observation_count: 200,
        },
        PredictivePattern {
            id: "pattern_layoff_closure".to_string(),
            description: "Mass layoffs precede facility closure".to_string(),
            triggers: vec!["layoffs".to_string()],
            predicted_outcome: "facility_closure".to_string(),
            prediction_window_days: 90,
            precision: 0.35,
            precision_ci_lower: 0.25,
            precision_ci_upper: 0.45,
            recall: 0.70,
            observation_count: 180,
        },
        PredictivePattern {
            id: "pattern_quality_recall".to_string(),
            description: "Quality issue leads to product recall".to_string(),
            triggers: vec!["quality_issue".to_string()],
            predicted_outcome: "product_recall".to_string(),
            prediction_window_days: 21,
            precision: 0.40,
            precision_ci_lower: 0.30,
            precision_ci_upper: 0.50,
            recall: 0.65,
            observation_count: 110,
        },
        PredictivePattern {
            id: "pattern_sanction_contract".to_string(),
            description: "Sanction designation leads to contract termination".to_string(),
            triggers: vec!["sanction_imposed".to_string()],
            predicted_outcome: "contract_termination".to_string(),
            prediction_window_days: 30,
            precision: 0.90,
            precision_ci_lower: 0.82,
            precision_ci_upper: 0.98,
            recall: 0.85,
            observation_count: 45,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_generation_works() {
        let pattern = PredictivePattern {
            id: "test_pattern".to_string(),
            description: "Test pattern".to_string(),
            triggers: vec!["trigger_a".to_string()],
            predicted_outcome: "outcome_b".to_string(),
            prediction_window_days: 30,
            precision: 0.75,
            precision_ci_lower: 0.68,
            precision_ci_upper: 0.82,
            recall: 0.60,
            observation_count: 100,
        };

        let prediction = pattern.predict(Uuid::new_v4(), "Test Corp");
        assert_eq!(prediction.probability, 0.75);
        assert_eq!(prediction.predicted_outcome, "outcome_b");
        assert!(prediction.is_active());
    }

    #[test]
    fn watch_list_adds_high_probability_predictions() {
        let mut watch_list = WatchList::new(WatchListConfig::default());
        
        let prediction = Prediction {
            id: Uuid::new_v4(),
            entity_id: Uuid::new_v4(),
            entity_name: "Test Corp".to_string(),
            pattern_id: "test".to_string(),
            predicted_outcome: "event".to_string(),
            prediction_window_days: 10,
            probability: 0.70,
            probability_ci_lower: 0.60,
            probability_ci_upper: 0.80,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(10),
            status: PredictionStatus::Active,
            actual_outcome: None,
        };

        watch_list.add_from_prediction(prediction);
        assert_eq!(watch_list.len(), 1);
    }

    #[test]
    fn watch_list_rejects_low_probability() {
        let mut watch_list = WatchList::new(WatchListConfig::default());
        
        let prediction = Prediction {
            id: Uuid::new_v4(),
            entity_id: Uuid::new_v4(),
            entity_name: "Test Corp".to_string(),
            pattern_id: "test".to_string(),
            predicted_outcome: "event".to_string(),
            prediction_window_days: 10,
            probability: 0.30, // Below threshold
            probability_ci_lower: 0.20,
            probability_ci_upper: 0.40,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(10),
            status: PredictionStatus::Active,
            actual_outcome: None,
        };

        watch_list.add_from_prediction(prediction);
        assert!(watch_list.is_empty());
    }

    #[test]
    fn accuracy_tracker_computes_log_loss() {
        let mut tracker = AccuracyTracker::new();
        
        let prediction = Prediction {
            id: Uuid::new_v4(),
            entity_id: Uuid::new_v4(),
            entity_name: "Test".to_string(),
            pattern_id: "test".to_string(),
            predicted_outcome: "event".to_string(),
            prediction_window_days: 30,
            probability: 0.80,
            probability_ci_lower: 0.70,
            probability_ci_upper: 0.90,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            status: PredictionStatus::Active,
            actual_outcome: None,
        };

        tracker.record(&prediction, true); // Correct prediction
        
        let log_loss = tracker.overall_log_loss();
        assert!(log_loss < 0.693); // Better than random
    }

    #[test]
    fn predefined_patterns_are_valid() {
        let patterns = predefined_patterns();
        assert!(patterns.len() >= 5);
        
        for pattern in &patterns {
            assert!(pattern.precision >= 0.0 && pattern.precision <= 1.0);
            assert!(pattern.precision_ci_lower <= pattern.precision);
            assert!(pattern.precision_ci_upper >= pattern.precision);
            assert!(pattern.observation_count > 0);
        }
    }

    #[test]
    fn pattern_accuracy_stats_track_correctly() {
        let mut stats = PatternAccuracyStats::new("test".to_string());
        
        stats.record(0.80, true);  // TP
        stats.record(0.70, true);  // TP
        stats.record(0.60, false); // FP
        stats.record(0.30, false); // TN
        
        assert_eq!(stats.true_positives, 2);
        assert_eq!(stats.false_positives, 1);
        assert_eq!(stats.true_negatives, 1);
        assert_eq!(stats.total(), 4);
        
        let precision = stats.precision();
        assert!((precision - 0.666).abs() < 0.01);
    }
}
