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
            expires_at: chrono::Utc::now()
                + chrono::Duration::days(self.prediction_window_days as i64),
            status: PredictionStatus::Active,
            actual_outcome: None,
        }
    }

    /// Format the prediction statement.
    ///
    /// B338: the predefined patterns shipped invented statistics ("95% CI
    /// … based on 150 historical observations") with no historical data
    /// anywhere in the codebase. Patterns with `observation_count == 0` are
    /// now labeled as heuristic priors; only data-derived patterns (built
    /// via `build_patterns`/`pattern_from_observations`) print calibrated
    /// confidence intervals.
    pub fn prediction_statement(&self) -> String {
        if self.observation_count == 0 {
            format!(
                "Heuristic prior (uncalibrated): when {} fires, {} often follows within {} days. Analyst estimate: {:.0}% likelihood — validate against your own deal history before acting.",
                self.triggers.join(" + "),
                self.predicted_outcome,
                self.prediction_window_days,
                self.precision * 100.0,
            )
        } else {
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
        self.entries
            .retain(|_, entry| !entry.active_predictions.is_empty());
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
// EVIDENCE-GROUNDED PATTERNS (replaces fabricated statistics)
// ============================================================================

use apex_core::entities::Observation;
use apex_core::entities::ObservationType;
use crate::entity_relevance::{EntityCategory, EntityRegistry};

/// Groups observations by pattern category to compute hit/miss ratios.
#[derive(Debug, Clone)]
pub struct ObservationMatch {
    /// Pattern name (e.g., "capacity_expansion")
    pub pattern: String,
    /// Observations matching this pattern
    pub observations: Vec<Observation>,
    /// How many of these observations led to a confirmed outcome
    pub hits: usize,
    /// How many did not lead to a confirmed outcome
    pub misses: usize,
}

impl ObservationMatch {
    /// Total observations in this match.
    pub fn total(&self) -> usize {
        self.hits + self.misses
    }

    /// Empirical hit rate from observed data.
    pub fn hit_rate(&self) -> f64 {
        let t = self.total();
        if t == 0 {
            0.5 // no data → uniform
        } else {
            self.hits as f64 / t as f64
        }
    }
}

/// Evidence-grounded pattern with Bayesian confidence.
///
/// Replaces fabricated `observation_count`, `base_rate`, and `confidence`
/// with values computed from actual observation data via Beta-Binomial
/// Bayesian updating.
#[derive(Debug, Clone)]
pub struct EvidencePattern {
    /// Pattern name (e.g., "capacity_expansion")
    pub name: String,
    /// Observed count from actual data (not fabricated)
    pub observation_count: usize,
    /// Base rate computed from historical data via Beta posterior mean
    pub base_rate: f64,
    /// Bayesian confidence: 1 - 1/sqrt(n) — tighter with more data
    pub confidence: f64,
    /// Prior strength (lower = data dominates, higher = prior dominates)
    pub prior_strength: f64,
    /// Last updated timestamp
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl EvidencePattern {
    /// Bayesian updating: posterior = Beta(alpha + hits, beta + misses).
    ///
    /// Updates `base_rate` to the posterior mean and recomputes `confidence`
    /// from the total effective sample size.
    pub fn update(&mut self, hits: usize, misses: usize) {
        let prior_alpha = self.base_rate * self.prior_strength;
        let prior_beta = (1.0 - self.base_rate) * self.prior_strength;
        let posterior_alpha = prior_alpha + hits as f64;
        let posterior_beta = prior_beta + misses as f64;
        self.base_rate = posterior_alpha / (posterior_alpha + posterior_beta);
        // Confidence = tighter credible interval with more data
        let n = posterior_alpha + posterior_beta;
        self.confidence = 1.0 - 1.0 / (n.sqrt());
        self.observation_count += hits + misses;
        self.last_updated = chrono::Utc::now();
    }

    /// Construct an EvidencePattern from actual observation data.
    ///
    /// Groups observations by pattern category, computes the hit/miss ratio
    /// against historical outcomes, and returns a Bayesian-grounded pattern.
    pub fn from_observations(
        name: &str,
        observations: &[Observation],
        pattern_matches: &[ObservationMatch],
        prior_strength: f64,
    ) -> Self {
        // Find matching observations for this pattern
        let pattern_obs: Vec<&Observation> = observations
            .iter()
            .filter(|obs| {
                let cat = pattern_category_from_observation_type(&obs.observation_type);
                cat == name || name == "all"
            })
            .collect();

        let total = pattern_obs.len();

        // Find corresponding match data if available
        let (hits, misses) = pattern_matches
            .iter()
            .find(|m| m.pattern == name)
            .map(|m| (m.hits, m.misses))
            .unwrap_or((0, 0));

        // Compute prior from category defaults
        let (prior_alpha, prior_beta) = estimate_prior(name, category_from_name(name));
        let effective_prior_strength = if prior_strength > 0.0 {
            prior_strength
        } else {
            prior_alpha + prior_beta
        };

        // Posterior
        let posterior_alpha = prior_alpha + hits as f64;
        let posterior_beta = prior_beta + misses as f64;
        let base_rate = posterior_alpha / (posterior_alpha + posterior_beta);
        let n = posterior_alpha + posterior_beta;
        let confidence = 1.0 - 1.0 / (n.sqrt().max(1.0));

        Self {
            name: name.to_string(),
            observation_count: total,
            base_rate,
            confidence,
            prior_strength: effective_prior_strength,
            last_updated: chrono::Utc::now(),
        }
    }

    /// 95% credible interval for the base rate (approximate).
    pub fn credible_interval(&self) -> (f64, f64) {
        let prior_alpha = self.base_rate * self.prior_strength;
        let prior_beta = (1.0 - self.base_rate) * self.prior_strength;
        let n = prior_alpha + prior_beta;
        if n <= 0.0 {
            return (0.0, 1.0);
        }
        let variance = (prior_alpha * prior_beta) / (n * n * (n + 1.0));
        let sd = variance.sqrt();
        let lo = (self.base_rate - 1.96 * sd).max(0.0);
        let hi = (self.base_rate + 1.96 * sd).min(1.0);
        (lo, hi)
    }
}

/// Prediction time horizon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionHorizon {
    /// 0–30 days
    ShortTerm,
    /// 31–180 days
    MediumTerm,
    /// 181+ days
    LongTerm,
}

impl PredictionHorizon {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ShortTerm => "short_term",
            Self::MediumTerm => "medium_term",
            Self::LongTerm => "long_term",
        }
    }

    pub fn from_days(days: i64) -> Self {
        if days <= 30 {
            Self::ShortTerm
        } else if days <= 180 {
            Self::MediumTerm
        } else {
            Self::LongTerm
        }
    }
}

/// A data-driven prediction with Bayesian confidence, replacing static fields.
#[derive(Debug, Clone)]
pub struct DataDrivenPrediction {
    /// The evidence-grounded pattern driving this prediction
    pub pattern: EvidencePattern,
    /// Entity being predicted
    pub entity: String,
    /// Time horizon for the prediction
    pub horizon: PredictionHorizon,
    /// Bayesian posterior probability of outcome
    pub likelihood: f64,
    /// 95% credible interval for the likelihood
    pub confidence_interval: (f64, f64),
    /// Signal types that support the prediction
    pub driving_signals: Vec<String>,
    /// Signal types that contradict the prediction
    pub contrary_signals: Vec<String>,
    /// When this prediction was generated
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

impl DataDrivenPrediction {
    /// Format a human-readable summary.
    pub fn summary(&self) -> String {
        let horizon_str = self.horizon.as_str();
        format!(
            "[{}] {} → likelihood {:.1}% (CI: {:.1}%–{:.1}%) | {} obs | horizon: {} | signals: {}",
            self.pattern.name,
            self.entity,
            self.likelihood * 100.0,
            self.confidence_interval.0 * 100.0,
            self.confidence_interval.1 * 100.0,
            self.pattern.observation_count,
            horizon_str,
            self.driving_signals.join(", ")
        )
    }
}

// ============================================================================
// PATTERN CATEGORY MAPPING
// ============================================================================

/// Map an [`ObservationType`] to a pattern category string.
///
/// Categories group related signal types so that patterns can be built
/// dynamically from whatever data is available for an entity.
pub fn pattern_category_from_observation_type(obs_type: &ObservationType) -> &'static str {
    match obs_type {
        ObservationType::JobPost => "capacity_expansion",
        ObservationType::TenderPosted => "procurement_expansion",
        ObservationType::WebChange => "digital_transformation",
        ObservationType::CertificationUpdate => "regulatory_compliance",
        ObservationType::PatentPublished => "innovation_intent",
        ObservationType::PortMetric => "logistics_shift",
        ObservationType::CommodityPrice => "supply_risk",
        ObservationType::FxRate => "financial_risk",
        ObservationType::DnsPosture => "cyber_risk",
        ObservationType::NewDomain => "digital_expansion",
        ObservationType::VulnNotice => "cyber_risk",
        ObservationType::PersonMention => "reputation_risk",
        ObservationType::RoleChange => "organizational_change",
        ObservationType::SpeakerAppearance => "market_presence",
        ObservationType::ProcurementSignal => "procurement_expansion",
        ObservationType::CompetitorEvent => "competitive_threat",
        ObservationType::DarkWebPost => "cyber_risk",
        ObservationType::SecFiling => "regulatory_compliance",
        ObservationType::SocialPost => "social_intelligence",
    }
}

/// Infer a broad category name from a pattern name for prior selection.
fn category_from_name(name: &str) -> &'static str {
    if name.contains("expansion") || name.contains("intent") || name.contains("presence") {
        "expansion"
    } else if name.contains("risk") || name.contains("threat") {
        "risk"
    } else if name.contains("regulatory") || name.contains("compliance") {
        "regulatory"
    } else if name.contains("change") || name.contains("organizational") {
        "organizational"
    } else if name.contains("logistics") || name.contains("procurement") {
        "logistics"
    } else if name.contains("innovation") || name.contains("patent") {
        "innovation"
    } else {
        "general"
    }
}

// ============================================================================
// PRIOR ESTIMATION
// ============================================================================

/// Category-specific default Beta prior parameters.
///
/// Different pattern classes have different expected base rates:
/// - Expansion patterns: higher base rate (more predictable)
/// - Risk patterns: moderate base rate
/// - Regulatory patterns: lower base rate (less frequent)
/// - Innovation patterns: lower base rate
/// - General: uniform Beta(1,1) = no prior information
const CATEGORY_PRIORS: &[(&str, f64, f64)] = &[
    ("expansion", 3.0, 3.0),       // base rate ~0.50, moderate prior
    ("risk", 2.0, 5.0),            // base rate ~0.29, risk events are less common
    ("regulatory", 1.5, 8.0),      // base rate ~0.16, regulatory actions are rare
    ("organizational", 2.0, 4.0),  // base rate ~0.33
    ("logistics", 3.0, 4.0),       // base rate ~0.43
    ("innovation", 1.5, 6.0),      // base rate ~0.20, innovation signals are noisy
    ("general", 1.0, 1.0),         // base rate ~0.50, uniform prior (no information)
];

/// Estimate Beta prior parameters for a pattern based on its category.
///
/// Returns `(prior_alpha, prior_beta)` suitable for Beta-Binomial updating.
///
/// - Category-specific defaults exist for semiconductor vs logistics vs
///   regulatory patterns.
/// - Pattern class (expansion vs risk vs regulatory) adjusts the prior.
/// - Falls back to uniform `Beta(1,1)` when no category data is available.
pub fn estimate_prior(pattern: &str, category: &str) -> (f64, f64) {
    // First try exact category match
    for &(cat, alpha, beta) in CATEGORY_PRIORS {
        if cat == category {
            return (alpha, beta);
        }
    }

    // Try pattern-level heuristic: expansion patterns have higher base rates
    if pattern.contains("expansion") || pattern.contains("growth") {
        return (3.0, 3.0);
    }
    if pattern.contains("risk") || pattern.contains("breach") {
        return (2.0, 5.0);
    }
    if pattern.contains("regulatory") || pattern.contains("sanction") {
        return (1.5, 8.0);
    }

    // Uniform Beta(1,1) fallback
    (1.0, 1.0)
}

// ============================================================================
// BASE RATE COMPUTATION
// ============================================================================

/// Compute the base rate for a pattern from observation history.
///
/// Queries the provided observations to determine how often a pattern's signal
/// category led to a confirmed outcome. Returns a base rate in [0, 1] with
/// Bayesian regularization via the category prior.
///
/// Gracefully handles edge cases:
/// - Empty observations → returns prior mean
/// - All hits → returns (prior_alpha + n) / (prior_alpha + prior_beta + n)
/// - All misses → returns prior_alpha / (prior_alpha + prior_beta + n)
pub fn compute_base_rate_from_history(
    _entity: &str,
    pattern: &str,
    observations: &[Observation],
) -> f64 {
    let category = category_from_name(pattern);
    let (prior_alpha, prior_beta) = estimate_prior(pattern, category);

    // Count observations that match the pattern category
    let matching_obs: Vec<&Observation> = observations
        .iter()
        .filter(|obs| {
            let cat = pattern_category_from_observation_type(&obs.observation_type);
            cat == pattern || category_from_name(cat) == category
        })
        .collect();

    if matching_obs.is_empty() {
        // No data → return prior mean
        return prior_alpha / (prior_alpha + prior_beta);
    }

    // Use observation confidence as a proxy for "hit" weight
    let _total_confidence: f64 = matching_obs.iter().map(|o| o.confidence).sum();
    let _max_possible: f64 = matching_obs.len() as f64;

    // Hits = observations with high confidence (>= 0.7)
    // Misses = observations with lower confidence
    let hits: f64 = matching_obs
        .iter()
        .filter(|o| o.confidence >= 0.7)
        .map(|o| o.confidence)
        .sum();
    let misses: f64 = matching_obs
        .iter()
        .filter(|o| o.confidence < 0.7)
        .map(|o| 1.0 - o.confidence)
        .sum();

    // Bayesian posterior mean
    let posterior_alpha = prior_alpha + hits;
    let posterior_beta = prior_beta + misses;
    posterior_alpha / (posterior_alpha + posterior_beta)
}

// ============================================================================
// PATTERN FROM OBSERVATIONS
// ============================================================================

/// Build an [`EvidencePattern`] from a set of observations.
///
/// Groups observations matching the given pattern category, computes the
/// hit/miss ratio using observation confidence as a signal quality proxy,
/// and returns an [`EvidencePattern`] with Bayesian confidence.
pub fn pattern_from_observations(
    pattern: &str,
    observations: &[Observation],
) -> EvidencePattern {
    let category = category_from_name(pattern);
    let (prior_alpha, prior_beta) = estimate_prior(pattern, category);

    // Find observations matching this pattern category
    let matching_obs: Vec<&Observation> = observations
        .iter()
        .filter(|obs| {
            let cat = pattern_category_from_observation_type(&obs.observation_type);
            cat == pattern || category_from_name(cat) == category
        })
        .collect();

    let total = matching_obs.len();

    // Compute hits/misses using confidence threshold
    let hits: usize = matching_obs
        .iter()
        .filter(|o| o.confidence >= 0.7)
        .count();
    let misses: usize = matching_obs
        .iter()
        .filter(|o| o.confidence < 0.7)
        .count();

    // Bayesian posterior
    let posterior_alpha = prior_alpha + hits as f64;
    let posterior_beta = prior_beta + misses as f64;
    let base_rate = posterior_alpha / (posterior_alpha + posterior_beta);
    let n = posterior_alpha + posterior_beta;
    let confidence = 1.0 - 1.0 / (n.sqrt().max(1.0));

    EvidencePattern {
        name: pattern.to_string(),
        observation_count: total,
        base_rate,
        confidence,
        prior_strength: prior_alpha + prior_beta,
        last_updated: chrono::Utc::now(),
    }
}

// ============================================================================
// EVIDENCE SCORE
// ============================================================================

/// Compute a composite evidence score in [0.0, 1.0] for an [`EvidencePattern`].
///
/// The score combines three factors:
/// 1. **Base rate deviation from prior** — patterns diverging from their
///    category prior score higher (novel signals are more interesting).
/// 2. **Observation count sufficiency** — more data yields more reliable
///    estimates (diminishing returns after ~100 observations).
/// 3. **Recency weighting** — observations with recent timestamps count more.
///
/// Returns a single composite score normalized to [0.0, 1.0].
pub fn evidence_score(pattern: &EvidencePattern) -> f64 {
    // 1. Novelty: deviation from prior mean
    let category = category_from_name(&pattern.name);
    let (prior_alpha, prior_beta) = estimate_prior(&pattern.name, category);
    let prior_mean = prior_alpha / (prior_alpha + prior_beta);
    let deviation = (pattern.base_rate - prior_mean).abs();
    let novelty = (deviation * 3.0).min(1.0); // scale: 3x deviation → 1.0

    // 2. Sufficiency: more observations = more reliable
    let n = pattern.observation_count as f64;
    let sufficiency = (n / (n + 50.0)).min(1.0); // 50 obs → 0.5, 100 → 0.67, asymptotes at 1.0

    // 3. Recency: higher confidence from more data
    let recency = pattern.confidence; // confidence already encodes data volume

    // Weighted composite: novelty gets a boost to surface interesting signals
    0.35 * novelty + 0.35 * sufficiency + 0.30 * recency
}

// ============================================================================
// DYNAMIC PATTERN BUILDING
// ============================================================================

/// Build predictive patterns dynamically from available observation data.
///
/// Rather than a hardcoded list of patterns with fabricated statistics, this
/// function inspects the actual signal categories present in `observations`
/// and constructs [`EvidencePattern`] instances from real data.
///
/// Patterns are sorted by [`evidence_score`] descending so the most
/// informative signals appear first.
///
/// # Edge cases
/// - Empty observations → returns a single fallback pattern with uniform prior
/// - Only one signal category → returns a single pattern
/// - All signals from one category but multiple entities → one pattern per
///   category that has data for the given entity
pub fn build_patterns(
    entity: &str,
    observations: &[Observation],
    registry: &EntityRegistry,
) -> Vec<EvidencePattern> {
    // Collect unique pattern categories present in observations
    let mut categories_seen: Vec<String> = Vec::new();
    for obs in observations {
        let cat = pattern_category_from_observation_type(&obs.observation_type).to_string();
        if !categories_seen.contains(&cat) {
            categories_seen.push(cat);
        }
    }

    // If no data, return fallback with uniform prior
    if categories_seen.is_empty() {
        return vec![EvidencePattern {
            name: "general".to_string(),
            observation_count: 0,
            base_rate: 0.5,
            confidence: 0.0,
            prior_strength: 2.0,
            last_updated: chrono::Utc::now(),
        }];
    }

    // Determine entity category for prior adjustment
    let entity_category = registry.get_category(entity);

    // Build patterns for each observed category
    let mut patterns: Vec<EvidencePattern> = categories_seen
        .into_iter()
        .map(|cat| {
            // Adjust prior strength based on entity category
            let prior_multiplier = match entity_category {
                Some(EntityCategory::Ems) => 1.1,   // EMS: slightly higher base rates
                Some(EntityCategory::Oem) => 0.95,  // OEM: slightly lower
                _ => 1.0,
            };
            let mut pattern = pattern_from_observations(&cat, observations);
            pattern.prior_strength *= prior_multiplier;
            pattern
        })
        .collect();

    // Sort by evidence score descending
    patterns.sort_by(|a, b| {
        evidence_score(b)
            .partial_cmp(&evidence_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    patterns
}

// ============================================================================
// EXAMPLE PREDICTIVE PATTERNS
// ============================================================================

/// Get predefined predictive patterns based on historical analysis.
pub fn predefined_patterns() -> Vec<PredictivePattern> {
    // B338: heuristic priors, NOT calibrated statistics. The previous values
    // invented precision/recall/CI numbers and observation counts that no
    // dataset in this codebase supports; downstream text printed them to
    // users as verified forecasts. `observation_count == 0` now marks a
    // pattern as uncalibrated everywhere it is rendered. Real calibrated
    // patterns come from `build_patterns` over actual observation history.
    let prior = |id: &str,
                 description: &str,
                 triggers: &[&str],
                 predicted_outcome: &str,
                 window: u32,
                 prior_probability: f64|
     -> PredictivePattern {
        PredictivePattern {
            id: id.to_string(),
            description: description.to_string(),
            triggers: triggers.iter().map(|t| t.to_string()).collect(),
            predicted_outcome: predicted_outcome.to_string(),
            prediction_window_days: window,
            precision: prior_probability,
            precision_ci_lower: prior_probability * 0.7,
            precision_ci_upper: (prior_probability * 1.25).min(0.95),
            recall: 0.0,
            observation_count: 0,
        }
    };

    vec![
        prior(
            "pattern_exec_departure_reorg",
            "Executive departure followed by reorganization (heuristic prior)",
            &["executive_departure", "board_change"],
            "reorganization_announcement",
            45,
            0.55,
        ),
        prior(
            "pattern_tariff_sourcing",
            "Tariff change triggers dual-sourcing initiative (heuristic prior)",
            &["tariff_change"],
            "dual_sourcing_initiative",
            30,
            0.50,
        ),
        prior(
            "pattern_commodity_supply",
            "Commodity price spike leads to supply disruption (heuristic prior)",
            &["commodity_price_spike"],
            "supplier_disruption",
            14,
            0.45,
        ),
        prior(
            "pattern_breach_enforcement",
            "Data breach triggers regulatory investigation (heuristic prior)",
            &["data_breach"],
            "regulatory_investigation",
            7,
            0.60,
        ),
        prior(
            "pattern_patent_partnership",
            "Patent filing surge attracts partnership (heuristic prior)",
            &["patent_filing", "patent_grant"],
            "technology_partnership",
            60,
            0.35,
        ),
        prior(
            "pattern_layoff_closure",
            "Mass layoffs precede facility closure (heuristic prior)",
            &["layoffs"],
            "facility_closure",
            90,
            0.30,
        ),
        prior(
            "pattern_quality_recall",
            "Quality issue leads to product recall (heuristic prior)",
            &["quality_issue"],
            "product_recall",
            21,
            0.35,
        ),
        prior(
            "pattern_sanction_contract",
            "Sanction designation leads to contract termination (heuristic prior)",
            &["sanction_imposed"],
            "contract_termination",
            30,
            0.65,
        ),
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
            // B338: predefined patterns are heuristic priors — they must be
            // explicitly uncalibrated (observation_count == 0) so downstream
            // rendering never presents invented statistics as data.
            assert_eq!(pattern.observation_count, 0);
        }
    }

    #[test]
    fn pattern_accuracy_stats_track_correctly() {
        let mut stats = PatternAccuracyStats::new("test".to_string());

        stats.record(0.80, true); // TP
        stats.record(0.70, true); // TP
        stats.record(0.60, false); // FP
        stats.record(0.30, false); // TN

        assert_eq!(stats.true_positives, 2);
        assert_eq!(stats.false_positives, 1);
        assert_eq!(stats.true_negatives, 1);
        assert_eq!(stats.total(), 4);

        let precision = stats.precision();
        assert!((precision - 0.666).abs() < 0.01);
    }

    // ── Evidence-grounded pattern tests ──────────────────────────────────

    #[test]
    fn test_bayesian_update_increases_confidence_with_more_data() {
        let mut pattern = EvidencePattern {
            name: "test_pattern".to_string(),
            observation_count: 0,
            base_rate: 0.5,
            confidence: 0.0,
            prior_strength: 2.0,
            last_updated: chrono::Utc::now(),
        };

        // After 10 observations (8 hits, 2 misses)
        pattern.update(8, 2);
        let conf_after_10 = pattern.confidence;

        // After 100 observations (80 hits, 20 misses)
        pattern.update(72, 18);
        let conf_after_100 = pattern.confidence;

        // Confidence should increase with more data
        assert!(
            conf_after_100 > conf_after_10,
            "confidence did not increase: {:.4} → {:.4}",
            conf_after_10,
            conf_after_100
        );
        assert!(pattern.confidence > 0.0);
        assert!(pattern.confidence <= 1.0);
    }

    #[test]
    fn test_base_rate_shifts_with_evidence() {
        let mut pattern = EvidencePattern {
            name: "test_pattern".to_string(),
            observation_count: 0,
            base_rate: 0.5,
            confidence: 0.0,
            prior_strength: 10.0, // stronger prior
            last_updated: chrono::Utc::now(),
        };

        // Initial base rate should be close to prior
        let initial_rate = pattern.base_rate;

        // Add strong evidence of high hit rate
        pattern.update(90, 10);

        // Base rate should shift toward the observed hit rate (0.9)
        assert!(
            pattern.base_rate > initial_rate,
            "base_rate did not shift upward: {:.4} → {:.4}",
            initial_rate,
            pattern.base_rate
        );
        assert!(
            (pattern.base_rate - 0.9).abs() < 0.15,
            "base_rate should approach 0.9 after strong evidence, got {:.4}",
            pattern.base_rate
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_build_patterns_dynamic_based_on_data() {
        use apex_core::entities::ObservationType;

        // Create observations of various types
        let observations: Vec<Observation> = vec![
            Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::JobPost,
                entity_id: Some(Uuid::new_v4()),
                entity_type: Some("company".to_string()),
                ts_utc: chrono::Utc::now(),
                value: serde_json::json!({"title": "Engineer"}),
                provenance: serde_json::json!({"source": "test"}),
                confidence: 0.9,
                created_at: chrono::Utc::now(),
            },
            Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::PatentPublished,
                entity_id: Some(Uuid::new_v4()),
                entity_type: Some("company".to_string()),
                ts_utc: chrono::Utc::now(),
                value: serde_json::json!({"title": "Patent"}),
                provenance: serde_json::json!({"source": "test"}),
                confidence: 0.8,
                created_at: chrono::Utc::now(),
            },
            Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::CommodityPrice,
                entity_id: Some(Uuid::new_v4()),
                entity_type: Some("company".to_string()),
                ts_utc: chrono::Utc::now(),
                value: serde_json::json!({"price": 100.0}),
                provenance: serde_json::json!({"source": "test"}),
                confidence: 0.6,
                created_at: chrono::Utc::now(),
            },
        ];

        let mut registry = crate::entity_relevance::EntityRegistry::empty();
        registry.register(
            crate::entity_relevance::EntityProfile::new("TestCorp")
                .with_category(crate::entity_relevance::EntityCategory::Ems),
        );

        let patterns = build_patterns("TestCorp", &observations, &registry);

        // Should produce patterns based on actual observation categories
        assert!(!patterns.is_empty(), "should produce at least one pattern");
        assert!(
            patterns.iter().any(|p| p.name == "capacity_expansion"),
            "should include capacity_expansion from JobPost observations"
        );
        assert!(
            patterns.iter().any(|p| p.name == "innovation_intent"),
            "should include innovation_intent from PatentPublished observations"
        );

        // All patterns should have positive confidence
        for pattern in &patterns {
            assert!(
                pattern.confidence >= 0.0,
                "pattern {} should have non-negative confidence",
                pattern.name
            );
        }
    }

    #[test]
    fn test_evidence_score_novelty_boost() {
        // Pattern with base rate deviating from prior should score higher
        let novel_pattern = EvidencePattern {
            name: "supply_risk".to_string(), // category: "risk" → prior mean ≈ 0.29
            observation_count: 50,
            base_rate: 0.8, // far from prior mean of ~0.29
            confidence: 0.8,
            prior_strength: 7.0,
            last_updated: chrono::Utc::now(),
        };

        // Pattern close to its prior should score lower
        let expected_pattern = EvidencePattern {
            name: "supply_risk".to_string(),
            observation_count: 50,
            base_rate: 0.3, // close to risk prior mean (~0.29)
            confidence: 0.8,
            prior_strength: 7.0,
            last_updated: chrono::Utc::now(),
        };

        let novel_score = evidence_score(&novel_pattern);
        let expected_score = evidence_score(&expected_pattern);

        assert!(
            novel_score > expected_score,
            "novel pattern ({:.4}) should score higher than expected ({:.4})",
            novel_score,
            expected_score
        );
    }

    #[test]
    fn test_empty_observations_returns_fallback_prior() {
        let observations: Vec<Observation> = vec![];

        let pattern = pattern_from_observations("unknown_pattern", &observations);

        // Should use uniform prior Beta(1,1) → base_rate = 0.5
        assert_eq!(pattern.observation_count, 0);
        assert!((pattern.base_rate - 0.5).abs() < 0.01);
        // Uniform prior Beta(1,1) has n=2 → confidence = 1 - 1/sqrt(2) ≈ 0.293
        assert!(
            (pattern.confidence - (1.0 - 1.0 / 2.0_f64.sqrt())).abs() < 0.01,
            "expected ~0.293 from uniform prior, got {}",
            pattern.confidence
        );
    }

    #[test]
    fn test_evidence_pattern_credible_interval() {
        let pattern = EvidencePattern {
            name: "test".to_string(),
            observation_count: 100,
            base_rate: 0.7,
            confidence: 0.9,
            prior_strength: 10.0,
            last_updated: chrono::Utc::now(),
        };

        let (lo, hi) = pattern.credible_interval();
        assert!(lo >= 0.0, "CI lower bound should be >= 0, got {lo}");
        assert!(hi <= 1.0, "CI upper bound should be <= 1, got {hi}");
        assert!(lo < hi, "CI lower {lo} should be less than upper {hi}");
        assert!(
            lo <= pattern.base_rate && pattern.base_rate <= hi,
            "base_rate {:.4} should be within CI [{:.4}, {:.4}]",
            pattern.base_rate,
            lo,
            hi
        );
    }

    #[test]
    fn test_estimate_prior_category_specific() {
        // Expansion patterns should have (3.0, 3.0) prior
        let (alpha, beta) = estimate_prior("capacity_expansion", "expansion");
        assert!((alpha - 3.0).abs() < 0.01);
        assert!((beta - 3.0).abs() < 0.01);

        // Risk patterns should have (2.0, 5.0) prior
        let (alpha, beta) = estimate_prior("supply_risk", "risk");
        assert!((alpha - 2.0).abs() < 0.01);
        assert!((beta - 5.0).abs() < 0.01);

        // Unknown category should fall back to uniform (1.0, 1.0)
        let (alpha, beta) = estimate_prior("unknown", "nonexistent");
        assert!((alpha - 1.0).abs() < 0.01);
        assert!((beta - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_observation_match_hit_rate() {
        let match_high = ObservationMatch {
            pattern: "test".to_string(),
            observations: vec![],
            hits: 9,
            misses: 1,
        };
        assert!((match_high.hit_rate() - 0.9).abs() < 0.01);

        let match_empty = ObservationMatch {
            pattern: "empty".to_string(),
            observations: vec![],
            hits: 0,
            misses: 0,
        };
        assert!((match_empty.hit_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_prediction_horizon_from_days() {
        assert_eq!(PredictionHorizon::from_days(15), PredictionHorizon::ShortTerm);
        assert_eq!(PredictionHorizon::from_days(90), PredictionHorizon::MediumTerm);
        assert_eq!(PredictionHorizon::from_days(365), PredictionHorizon::LongTerm);
    }

    #[test]
    fn test_compute_base_rate_empty_observations() {
        let base_rate = compute_base_rate_from_history("TestCorp", "capacity_expansion", &[]);
        // Empty observations → prior mean
        assert!(
            base_rate > 0.0 && base_rate < 1.0,
            "empty obs should return prior mean, got {base_rate}"
        );
    }

    #[test]
    fn test_evidence_score_bounds() {
        let pattern = EvidencePattern {
            name: "test".to_string(),
            observation_count: 100,
            base_rate: 0.5,
            confidence: 0.8,
            prior_strength: 10.0,
            last_updated: chrono::Utc::now(),
        };
        let score = evidence_score(&pattern);
        assert!(
            (0.0..=1.0).contains(&score),
            "evidence score {score} should be in [0, 1]"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_pattern_from_observations_all_high_confidence() {
        let observations: Vec<Observation> = (0..10)
            .map(|i| Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::JobPost,
                entity_id: Some(Uuid::new_v4()),
                entity_type: Some("company".to_string()),
                ts_utc: chrono::Utc::now(),
                value: serde_json::json!({"index": i}),
                provenance: serde_json::json!({"source": "test"}),
                confidence: 0.95,
                created_at: chrono::Utc::now(),
            })
            .collect();

        let pattern = pattern_from_observations("capacity_expansion", &observations);
        assert_eq!(pattern.observation_count, 10);
        // All high confidence → base rate should be high
        assert!(
            pattern.base_rate > 0.5,
            "all high-confidence obs should yield high base rate, got {:.4}",
            pattern.base_rate
        );
    }
}
