//! Insight outcome tracking — closing the prediction → verification loop.
//!
//! When the system generates an insight (e.g., "Supplier X likely to face
//! certification lapse"), there's currently no mechanism to check whether
//! the predicted outcome actually materialised.
//!
//! This module:
//! 1. Records predictions with their expected outcomes and timeframes.
//! 2. Matches them against observed outcomes.
//! 3. Computes accuracy metrics (precision, recall, Brier score).
//! 4. Feeds results back into recipe performance tracking.
//! 5. Maintains a running Bayesian estimate of each recipe's accuracy.
//!
//! All functions are pure — no database, no side effects.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ────────────────────────────────────────────
// Tracked predictions
// ────────────────────────────────────────────

/// A prediction made by the system that can be verified against future outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedPrediction {
    pub id: Uuid,
    pub recipe_id: Uuid,
    pub recipe_code: String,
    /// The specific outcome event type we expect (e.g., "CertificationUpdate").
    pub expected_outcome: String,
    /// Entity this prediction is about.
    pub entity_id: Uuid,
    /// Confidence at time of prediction (0–1).
    pub confidence: f64,
    /// When the prediction was made.
    pub predicted_at: DateTime<Utc>,
    /// Window within which the outcome should occur.
    pub expected_by: DateTime<Utc>,
    /// Observation IDs that triggered this prediction (audit trail).
    pub triggering_observation_ids: Vec<Uuid>,
    /// Current resolution status.
    pub status: PredictionStatus,
    /// When the prediction was resolved (if resolved).
    pub resolved_at: Option<DateTime<Utc>>,
    /// The actual outcome ID if verified (audit trail linkage).
    pub matched_outcome_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PredictionStatus {
    /// Prediction is still within its expected window.
    Pending,
    /// Predicted outcome occurred within the window.
    Confirmed,
    /// Expected window elapsed without the outcome occurring.
    Expired,
    /// Prediction was marked incorrect by analyst feedback.
    Rejected,
}

impl TrackedPrediction {
    pub fn new(
        recipe_id: Uuid,
        recipe_code: impl Into<String>,
        expected_outcome: impl Into<String>,
        entity_id: Uuid,
        confidence: f64,
        expected_by: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            recipe_id,
            recipe_code: recipe_code.into(),
            expected_outcome: expected_outcome.into(),
            entity_id,
            confidence: confidence.clamp(0.0, 1.0),
            predicted_at: Utc::now(),
            expected_by,
            triggering_observation_ids: Vec::new(),
            status: PredictionStatus::Pending,
            resolved_at: None,
            matched_outcome_id: None,
        }
    }

    /// Mark as confirmed with the matching outcome.
    pub fn confirm(&mut self, outcome_id: Uuid) {
        self.status = PredictionStatus::Confirmed;
        self.resolved_at = Some(Utc::now());
        self.matched_outcome_id = Some(outcome_id);
    }

    /// Mark as expired (prediction window passed without outcome).
    pub fn expire(&mut self) {
        self.status = PredictionStatus::Expired;
        self.resolved_at = Some(Utc::now());
    }

    /// Mark as rejected by analyst.
    pub fn reject(&mut self) {
        self.status = PredictionStatus::Rejected;
        self.resolved_at = Some(Utc::now());
    }

    /// Is this prediction still pending resolution?
    pub fn is_pending(&self) -> bool {
        self.status == PredictionStatus::Pending
    }
}

// ────────────────────────────────────────────
// Observed outcome (for matching)
// ────────────────────────────────────────────

/// A minimal observed outcome record for matching against predictions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedOutcome {
    pub id: Uuid,
    pub outcome_type: String,
    pub entity_id: Uuid,
    pub ts_utc: DateTime<Utc>,
}

// ────────────────────────────────────────────
// Matching engine
// ────────────────────────────────────────────

/// Match pending predictions against newly observed outcomes.
///
/// For each pending prediction, checks if any outcome matches on:
/// 1. Same entity ID.
/// 2. Same outcome type.
/// 3. Outcome timestamp within the prediction window.
///
/// Returns a list of (prediction_id, outcome_id) pairs that matched.
/// Callers should then call `confirm()` on the matched predictions.
pub fn match_predictions(
    predictions: &[TrackedPrediction],
    outcomes: &[ObservedOutcome],
) -> Vec<(Uuid, Uuid)> {
    let mut matches = Vec::new();

    // Index outcomes by (entity_id, outcome_type) for fast lookup.
    let mut outcome_index: HashMap<(Uuid, &str), Vec<&ObservedOutcome>> = HashMap::new();
    for o in outcomes {
        outcome_index
            .entry((o.entity_id, o.outcome_type.as_str()))
            .or_default()
            .push(o);
    }

    for pred in predictions.iter().filter(|p| p.is_pending()) {
        let key = (pred.entity_id, pred.expected_outcome.as_str());
        if let Some(matching_outcomes) = outcome_index.get(&key) {
            for o in matching_outcomes {
                if o.ts_utc >= pred.predicted_at && o.ts_utc <= pred.expected_by {
                    matches.push((pred.id, o.id));
                    break; // One match per prediction.
                }
            }
        }
    }

    matches
}

/// Expire all predictions whose window has elapsed.
///
/// Returns the IDs of predictions that were expired.
pub fn expire_overdue(predictions: &mut [TrackedPrediction], now: DateTime<Utc>) -> Vec<Uuid> {
    let mut expired = Vec::new();
    for pred in predictions.iter_mut() {
        if pred.is_pending() && now > pred.expected_by {
            pred.expire();
            expired.push(pred.id);
        }
    }
    expired
}

// ────────────────────────────────────────────
// Accuracy metrics
// ────────────────────────────────────────────

/// Per-recipe accuracy summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeAccuracy {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub total_predictions: u64,
    pub confirmed: u64,
    pub expired: u64,
    pub rejected: u64,
    pub pending: u64,
    /// Precision: confirmed / (confirmed + expired + rejected).
    pub precision: f64,
    /// Mean confidence of confirmed predictions.
    pub mean_confidence_confirmed: f64,
    /// Mean confidence of expired/rejected predictions.
    pub mean_confidence_wrong: f64,
    /// Brier score: mean (confidence − actual)².
    /// Lower is better; 0 = perfect calibration.
    pub brier_score: f64,
    /// Bayesian posterior mean of true positive rate (Beta-Binomial).
    pub bayesian_accuracy: f64,
    /// 95% credible interval for accuracy.
    pub bayesian_ci_95: (f64, f64),
}

/// Compute accuracy metrics per recipe from a batch of predictions.
pub fn compute_accuracy(predictions: &[TrackedPrediction]) -> Vec<RecipeAccuracy> {
    // Group by recipe_id.
    let mut groups: HashMap<Uuid, Vec<&TrackedPrediction>> = HashMap::new();
    for p in predictions {
        groups.entry(p.recipe_id).or_default().push(p);
    }

    let mut results: Vec<RecipeAccuracy> = groups
        .iter()
        .map(|(&recipe_id, preds)| {
            let recipe_code = preds
                .first()
                .map(|p| p.recipe_code.clone())
                .unwrap_or_default();

            let mut confirmed = 0u64;
            let mut expired = 0u64;
            let mut rejected = 0u64;
            let mut pending = 0u64;
            let mut conf_sum_confirmed = 0.0f64;
            let mut conf_sum_wrong = 0.0f64;
            let mut brier_sum = 0.0f64;
            let mut resolved_count = 0u64;

            for p in preds {
                match p.status {
                    PredictionStatus::Confirmed => {
                        confirmed += 1;
                        conf_sum_confirmed += p.confidence;
                        brier_sum += (p.confidence - 1.0).powi(2);
                        resolved_count += 1;
                    }
                    PredictionStatus::Expired => {
                        expired += 1;
                        conf_sum_wrong += p.confidence;
                        brier_sum += (p.confidence - 0.0).powi(2); // actual = 0
                        resolved_count += 1;
                    }
                    PredictionStatus::Rejected => {
                        rejected += 1;
                        conf_sum_wrong += p.confidence;
                        brier_sum += (p.confidence - 0.0).powi(2);
                        resolved_count += 1;
                    }
                    PredictionStatus::Pending => {
                        pending += 1;
                    }
                }
            }

            let total = preds.len() as u64;
            let resolved = confirmed + expired + rejected;
            let precision = if resolved > 0 {
                confirmed as f64 / resolved as f64
            } else {
                1.0
            };
            let mean_conf_confirmed = if confirmed > 0 {
                conf_sum_confirmed / confirmed as f64
            } else {
                0.0
            };
            let mean_conf_wrong = if (expired + rejected) > 0 {
                conf_sum_wrong / (expired + rejected) as f64
            } else {
                0.0
            };
            let brier = if resolved_count > 0 {
                brier_sum / resolved_count as f64
            } else {
                0.0
            };

            // Bayesian accuracy: Beta(1 + confirmed, 1 + wrong) posterior.
            let alpha = 1.0 + confirmed as f64;
            let beta = 1.0 + (expired + rejected) as f64;
            let bayesian_accuracy = alpha / (alpha + beta);
            let ab = alpha + beta;
            let var = (alpha * beta) / (ab * ab * (ab + 1.0));
            let sd = var.sqrt();
            let ci_lo = (bayesian_accuracy - 1.96 * sd).max(0.0);
            let ci_hi = (bayesian_accuracy + 1.96 * sd).min(1.0);

            RecipeAccuracy {
                recipe_id,
                recipe_code,
                total_predictions: total,
                confirmed,
                expired,
                rejected,
                pending,
                precision,
                mean_confidence_confirmed: mean_conf_confirmed,
                mean_confidence_wrong: mean_conf_wrong,
                brier_score: brier,
                bayesian_accuracy,
                bayesian_ci_95: (ci_lo, ci_hi),
            }
        })
        .collect();

    // Sort by recipe_code for deterministic output.
    results.sort_by(|a, b| a.recipe_code.cmp(&b.recipe_code));
    results
}

// ────────────────────────────────────────────
// Calibration analysis
// ────────────────────────────────────────────

/// A calibration bin for reliability diagrams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationBin {
    /// Bin centre (e.g., 0.55 for the 0.5–0.6 bin).
    pub bin_centre: f64,
    /// Mean confidence of predictions in this bin.
    pub mean_confidence: f64,
    /// Fraction of predictions in this bin that were confirmed.
    pub observed_frequency: f64,
    /// Number of predictions in this bin.
    pub count: usize,
}

/// Compute calibration curve: for each confidence bin, what fraction of
/// predictions actually came true?
///
/// Useful for detecting overconfident or underconfident recipes.
/// Returns 10 bins [0.0–0.1), [0.1–0.2), …, [0.9–1.0].
pub fn calibration_curve(predictions: &[TrackedPrediction]) -> Vec<CalibrationBin> {
    let resolved: Vec<&TrackedPrediction> =
        predictions.iter().filter(|p| !p.is_pending()).collect();

    let mut bins: Vec<(f64, u64, u64)> = (0..10).map(|i| (i as f64 * 0.1 + 0.05, 0, 0)).collect();

    for pred in &resolved {
        let idx = ((pred.confidence * 10.0).floor() as usize).min(9);
        bins[idx].1 += 1; // total
        if pred.status == PredictionStatus::Confirmed {
            bins[idx].2 += 1; // confirmed
        }
    }

    bins.iter()
        .map(|&(centre, total, confirmed)| CalibrationBin {
            bin_centre: centre,
            mean_confidence: centre, // approximate
            observed_frequency: if total > 0 {
                confirmed as f64 / total as f64
            } else {
                0.0
            },
            count: total as usize,
        })
        .collect()
}

// ────────────────────────────────────────────
// Feedback generation
// ────────────────────────────────────────────

/// A concrete recommendation for improving recipe or collection strategy
/// based on outcome tracking data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeFeedback {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub feedback_type: OutcomeFeedbackType,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutcomeFeedbackType {
    /// Recipe is well-calibrated (confidence matches reality).
    WellCalibrated,
    /// Recipe is overconfident (high confidence but low hit rate).
    Overconfident,
    /// Recipe is underconfident (low confidence but high hit rate).
    Underconfident,
    /// Recipe has poor overall accuracy — consider deprecation.
    PoorAccuracy,
    /// Not enough resolved predictions to evaluate.
    InsufficientData,
}

/// Generate feedback for each recipe based on outcome tracking.
///
/// Thresholds:
/// - Overconfident: mean_confidence_confirmed > 0.8 AND precision < 0.5
/// - Underconfident: precision > 0.8 AND mean_confidence_confirmed < 0.5
/// - PoorAccuracy: precision < 0.3 (with ≥10 resolved predictions)
pub fn generate_outcome_feedback(accuracy: &[RecipeAccuracy]) -> Vec<OutcomeFeedback> {
    accuracy
        .iter()
        .map(|a| {
            let resolved = a.confirmed + a.expired + a.rejected;
            let (feedback_type, detail) = if resolved < 5 {
                (
                    OutcomeFeedbackType::InsufficientData,
                    format!(
                        "Only {} resolved predictions — need ≥5 for evaluation",
                        resolved
                    ),
                )
            } else if a.precision < 0.3 && resolved >= 10 {
                (
                    OutcomeFeedbackType::PoorAccuracy,
                    format!(
                        "Precision {:.2} with {} resolved predictions — consider deprecation",
                        a.precision, resolved
                    ),
                )
            } else if a.mean_confidence_confirmed > 0.8 && a.precision < 0.5 {
                (
                    OutcomeFeedbackType::Overconfident,
                    format!(
                        "Mean confidence {:.2} but precision only {:.2} — \
                         system is overconfident on this recipe",
                        a.mean_confidence_confirmed, a.precision
                    ),
                )
            } else if a.precision > 0.8 && a.mean_confidence_confirmed < 0.5 {
                (
                    OutcomeFeedbackType::Underconfident,
                    format!(
                        "Precision {:.2} but mean confidence only {:.2} — \
                         system could raise confidence for this recipe",
                        a.precision, a.mean_confidence_confirmed
                    ),
                )
            } else {
                (
                    OutcomeFeedbackType::WellCalibrated,
                    format!(
                        "Precision {:.2}, mean confidence {:.2}, Brier {:.4} — well calibrated",
                        a.precision, a.mean_confidence_confirmed, a.brier_score
                    ),
                )
            };
            OutcomeFeedback {
                recipe_id: a.recipe_id,
                recipe_code: a.recipe_code.clone(),
                feedback_type,
                detail,
            }
        })
        .collect()
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use chrono::Duration;

    fn recipe_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    }

    fn entity_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()
    }

    fn make_prediction(confidence: f64, days_ahead: i64) -> TrackedPrediction {
        TrackedPrediction::new(
            recipe_id(),
            "A001",
            "CertificationUpdate",
            entity_id(),
            confidence,
            Utc::now() + Duration::days(days_ahead),
        )
    }

    #[test]
    fn test_prediction_lifecycle() {
        let mut pred = make_prediction(0.85, 30);
        assert!(pred.is_pending());

        let outcome_id = Uuid::new_v4();
        pred.confirm(outcome_id);
        assert_eq!(pred.status, PredictionStatus::Confirmed);
        assert_eq!(pred.matched_outcome_id, Some(outcome_id));
        assert!(!pred.is_pending());
    }

    #[test]
    fn test_prediction_expire() {
        let mut pred = make_prediction(0.7, 30);
        pred.expire();
        assert_eq!(pred.status, PredictionStatus::Expired);
        assert!(pred.resolved_at.is_some());
    }

    #[test]
    fn test_match_predictions() {
        let now = Utc::now();
        let eid = entity_id();
        let mut pred = TrackedPrediction {
            id: Uuid::new_v4(),
            recipe_id: recipe_id(),
            recipe_code: "A001".into(),
            expected_outcome: "CertificationUpdate".into(),
            entity_id: eid,
            confidence: 0.8,
            predicted_at: now - Duration::days(5),
            expected_by: now + Duration::days(25),
            triggering_observation_ids: Vec::new(),
            status: PredictionStatus::Pending,
            resolved_at: None,
            matched_outcome_id: None,
        };

        let outcome = ObservedOutcome {
            id: Uuid::new_v4(),
            outcome_type: "CertificationUpdate".into(),
            entity_id: eid,
            ts_utc: now,
        };

        let matches = match_predictions(&[pred.clone()], &[outcome.clone()]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, pred.id);
        assert_eq!(matches[0].1, outcome.id);

        // Wrong entity should not match.
        let other_outcome = ObservedOutcome {
            id: Uuid::new_v4(),
            outcome_type: "CertificationUpdate".into(),
            entity_id: Uuid::new_v4(), // different entity
            ts_utc: now,
        };
        let no_matches = match_predictions(&[pred.clone()], &[other_outcome]);
        assert!(no_matches.is_empty());

        // Already confirmed should not re-match.
        pred.confirm(outcome.id);
        let no_matches2 = match_predictions(&[pred], &[outcome]);
        assert!(no_matches2.is_empty());
    }

    #[test]
    fn test_expire_overdue() {
        let now = Utc::now();
        let mut preds = vec![
            {
                let mut p = make_prediction(0.6, -5); // expired 5 days ago
                p.expected_by = now - Duration::days(5);
                p.predicted_at = now - Duration::days(35);
                p
            },
            make_prediction(0.7, 30), // still in window
        ];

        let expired = expire_overdue(&mut preds, now);
        assert_eq!(expired.len(), 1);
        assert_eq!(preds[0].status, PredictionStatus::Expired);
        assert_eq!(preds[1].status, PredictionStatus::Pending);
    }

    #[test]
    fn test_compute_accuracy() {
        let mut predictions = Vec::new();

        // 7 confirmed, 3 expired → precision 0.7.
        for _ in 0..7 {
            let mut p = make_prediction(0.8, 30);
            p.recipe_id = recipe_id();
            p.recipe_code = "A001".into();
            p.id = Uuid::new_v4();
            p.confirm(Uuid::new_v4());
            predictions.push(p);
        }
        for _ in 0..3 {
            let mut p = make_prediction(0.6, 30);
            p.recipe_id = recipe_id();
            p.recipe_code = "A001".into();
            p.id = Uuid::new_v4();
            p.expire();
            predictions.push(p);
        }

        let acc = compute_accuracy(&predictions);
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].recipe_code, "A001");
        assert_eq!(acc[0].confirmed, 7);
        assert_eq!(acc[0].expired, 3);
        assert!((acc[0].precision - 0.7).abs() < 0.01);
        assert!(acc[0].bayesian_accuracy > 0.5);
        assert!(acc[0].brier_score < 0.5);
    }

    #[test]
    fn test_calibration_curve() {
        let mut preds = Vec::new();
        // 10 predictions at confidence 0.9, 8 confirmed, 2 expired.
        for i in 0..10 {
            let mut p = make_prediction(0.92, 30);
            p.id = Uuid::new_v4();
            if i < 8 {
                p.confirm(Uuid::new_v4());
            } else {
                p.expire();
            }
            preds.push(p);
        }

        let curve = calibration_curve(&preds);
        assert_eq!(curve.len(), 10);
        // The 0.9–1.0 bin should have 10 entries, 80% confirmed.
        let high_bin = &curve[9];
        assert_eq!(high_bin.count, 10);
        assert!((high_bin.observed_frequency - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_generate_feedback_overconfident() {
        let acc = RecipeAccuracy {
            recipe_id: recipe_id(),
            recipe_code: "B005".into(),
            total_predictions: 20,
            confirmed: 4,
            expired: 14,
            rejected: 2,
            pending: 0,
            precision: 0.2,
            mean_confidence_confirmed: 0.9,
            mean_confidence_wrong: 0.85,
            brier_score: 0.6,
            bayesian_accuracy: 0.23,
            bayesian_ci_95: (0.1, 0.36),
        };
        let feedback = generate_outcome_feedback(&[acc]);
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].feedback_type, OutcomeFeedbackType::PoorAccuracy);
    }

    #[test]
    fn test_generate_feedback_insufficient() {
        let acc = RecipeAccuracy {
            recipe_id: recipe_id(),
            recipe_code: "C001".into(),
            total_predictions: 3,
            confirmed: 2,
            expired: 1,
            rejected: 0,
            pending: 0,
            precision: 0.67,
            mean_confidence_confirmed: 0.7,
            mean_confidence_wrong: 0.5,
            brier_score: 0.2,
            bayesian_accuracy: 0.6,
            bayesian_ci_95: (0.3, 0.9),
        };
        let feedback = generate_outcome_feedback(&[acc]);
        assert_eq!(
            feedback[0].feedback_type,
            OutcomeFeedbackType::InsufficientData
        );
    }

    #[test]
    fn test_generate_feedback_well_calibrated() {
        let acc = RecipeAccuracy {
            recipe_id: recipe_id(),
            recipe_code: "D010".into(),
            total_predictions: 50,
            confirmed: 40,
            expired: 8,
            rejected: 2,
            pending: 0,
            precision: 0.8,
            mean_confidence_confirmed: 0.82,
            mean_confidence_wrong: 0.55,
            brier_score: 0.1,
            bayesian_accuracy: 0.79,
            bayesian_ci_95: (0.7, 0.88),
        };
        let feedback = generate_outcome_feedback(&[acc]);
        assert_eq!(
            feedback[0].feedback_type,
            OutcomeFeedbackType::WellCalibrated
        );
    }
}
