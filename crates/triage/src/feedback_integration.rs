//! Feedback Integration — connects the triage engine with the existing
//! [`InsightFeedbackTracker`] to close the feedback loop.
//!
//! When users acknowledge, resolve, or dismiss triage items, this module
//! translates those actions into feedback signals that improve future
//! recipe performance, fatigue detection, and threshold recommendations.

use chrono::Utc;

use apex_core::triage::{TriageDimensions, TriageItemType, TriageStatus};
use apex_insights::insight_feedback::{InsightFeedback, InsightFeedbackRecord, InsightFeedbackTracker};

/// Translates triage actions into feedback signals for the insight feedback loop.
pub struct FeedbackIntegration {
    tracker: InsightFeedbackTracker,
}

impl FeedbackIntegration {
    /// Create a new [`FeedbackIntegration`] wrapping an [`InsightFeedbackTracker`].
    pub fn new(tracker: InsightFeedbackTracker) -> Self {
        Self { tracker }
    }

    /// Get a reference to the underlying tracker (for direct access).
    pub fn tracker(&self) -> &InsightFeedbackTracker {
        &self.tracker
    }

    /// Get a mutable reference to the underlying tracker.
    pub fn tracker_mut(&mut self) -> &mut InsightFeedbackTracker {
        &mut self.tracker
    }

    /// Record feedback when a triage item is acknowledged (user has seen it).
    ///
    /// This translates to a "viewed" feedback signal for the source insight/warning.
    pub fn record_acknowledged(
        &mut self,
        item_type: &TriageItemType,
        source_id: &str,
        _dimensions: Option<&TriageDimensions>,
        entity_name: Option<&str>,
    ) {
        if *item_type != TriageItemType::Insight {
            return;
        }

        let record = InsightFeedbackRecord {
            insight_id: source_id.to_string(),
            feedback: InsightFeedback::Viewed,
            entity_id: entity_name.unwrap_or("unknown").to_string(),
            recipe_code: "triage".to_string(),
            timestamp: Utc::now().timestamp(),
            user_id: None,
            notes: Some("Triage acknowledged".to_string()),
        };
        self.tracker.record_feedback(record);
    }

    /// Record feedback when a triage item is dismissed (false positive).
    ///
    /// This is the most valuable signal — it tells the system a generated
    /// insight was irrelevant. This feeds into recipe performance tracking
    /// and can trigger recipe deprecation or adjustment.
    pub fn record_dismissed(
        &mut self,
        item_type: &TriageItemType,
        source_id: &str,
        _dimensions: Option<&TriageDimensions>,
        entity_name: Option<&str>,
        user_id: Option<&str>,
    ) {
        if *item_type != TriageItemType::Insight {
            return;
        }

        let feedback = InsightFeedback::FalsePositive;
        let record = InsightFeedbackRecord {
            insight_id: source_id.to_string(),
            feedback,
            entity_id: entity_name.unwrap_or("unknown").to_string(),
            recipe_code: "triage".to_string(),
            timestamp: Utc::now().timestamp(),
            user_id: user_id.map(|s| s.to_string()),
            notes: Some("Triage dismissed — false positive".to_string()),
        };
        self.tracker.record_feedback(record);
    }

    /// Record feedback when a triage item is resolved (action taken).
    ///
    /// This is a positive signal — the insight was actionable and the user
    /// took action on it.
    pub fn record_resolved(
        &mut self,
        item_type: &TriageItemType,
        source_id: &str,
        _dimensions: Option<&TriageDimensions>,
        entity_name: Option<&str>,
        user_id: Option<&str>,
    ) {
        if *item_type != TriageItemType::Insight {
            return;
        }

        let record = InsightFeedbackRecord {
            insight_id: source_id.to_string(),
            feedback: InsightFeedback::Actioned,
            entity_id: entity_name.unwrap_or("unknown").to_string(),
            recipe_code: "triage".to_string(),
            timestamp: Utc::now().timestamp(),
            user_id: user_id.map(|s| s.to_string()),
            notes: Some("Triage resolved — action taken".to_string()),
        };
        self.tracker.record_feedback(record);
    }

    /// Batch-process a status change from the triage queue into feedback.
    ///
    /// This is the primary integration hook called when a triage item's
    /// status changes.
    pub fn on_status_change(
        &mut self,
        item_type: &TriageItemType,
        source_id: &str,
        new_status: &TriageStatus,
        dimensions: Option<&TriageDimensions>,
        entity_name: Option<&str>,
        user_id: Option<&str>,
    ) {
        match new_status {
            TriageStatus::Acknowledged => {
                self.record_acknowledged(item_type, source_id, dimensions, entity_name);
            }
            TriageStatus::Dismissed => {
                self.record_dismissed(item_type, source_id, dimensions, entity_name, user_id);
            }
            TriageStatus::Resolved => {
                self.record_resolved(item_type, source_id, dimensions, entity_name, user_id);
            }
            _ => {
                // Pending and Triaged statuses don't generate feedback
            }
        }
    }

    /// Get recipe performance from the tracker.
    pub fn recipe_performances(&self) -> Vec<(String, f64, f64, f64)> {
        self.tracker
            .get_all_performances()
            .iter()
            .map(|p| {
                (
                    p.recipe_code.clone(),
                    p.precision(),
                    p.recall(),
                    p.f1_score(),
                )
            })
            .collect()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_integration_ignores_non_insight() {
        let tracker = InsightFeedbackTracker::new();
        let mut integration = FeedbackIntegration::new(tracker);

        // Should not panic and should not record feedback for warnings
        integration.record_dismissed(
            &TriageItemType::Warning,
            "warn-1",
            None,
            Some("Acme Corp"),
            None,
        );

        // Tracker should still be pristine (no records)
        let performances = integration.recipe_performances();
        assert!(performances.is_empty());
    }

    #[test]
    fn test_feedback_integration_acknowledge() {
        let tracker = InsightFeedbackTracker::new();
        let mut integration = FeedbackIntegration::new(tracker);

        integration.record_acknowledged(
            &TriageItemType::Insight,
            "insight-1",
            None,
            Some("Acme Corp"),
        );

        // Should not crash; acknowledge is "viewed" feedback
        let performances = integration.recipe_performances();
        // Viewed feedback creates a performance record with total_firings=1
        // but does not increment TP/FP/FN (neutral prior: precision=0.5, recall=0.5)
        assert_eq!(performances.len(), 1);
        let (_recipe, precision, recall, f1) = &performances[0];
        assert!((*precision - 0.5).abs() < 0.001);
        assert!((*recall - 0.5).abs() < 0.001);
        assert!((*f1 - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_feedback_integration_dismissed_updates_tracker() {
        let tracker = InsightFeedbackTracker::new();
        let mut integration = FeedbackIntegration::new(tracker);

        // First record a firing to set up recipe performance tracking
        integration.tracker_mut().record_firing("Acme Corp", "recipe-1", "insight-1");

        integration.record_dismissed(
            &TriageItemType::Insight,
            "insight-1",
            None,
            Some("Acme Corp"),
            Some("user-1"),
        );

        // Dismissed (false positive) should affect recipe performance
        let performances = integration.recipe_performances();
        assert_eq!(performances.len(), 1);

        let (_recipe, precision, recall, f1) = &performances[0];
        // With 1 false positive and 0 true positives, precision should be 0
        assert!((*precision - 0.0).abs() < 0.001);
        // recall: tp=0, fn=0 → neutral prior of 0.5 (not 1.0)
        assert!((*recall - 0.5).abs() < 0.001);
        // f1 = 2 * p * r / (p + r) = 2 * 0.0 * 0.5 / (0.0 + 0.5) = 0.0
        assert!((*f1 - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_on_status_change_routes_correctly() {
        let tracker = InsightFeedbackTracker::new();
        let mut integration = FeedbackIntegration::new(tracker);

        // Pending should not record anything
        integration.on_status_change(
            &TriageItemType::Insight,
            "insight-1",
            &TriageStatus::Pending,
            None,
            Some("Acme Corp"),
            None,
        );

        // Resolved should record "actioned"
        integration.on_status_change(
            &TriageItemType::Insight,
            "insight-2",
            &TriageStatus::Resolved,
            None,
            Some("Acme Corp"),
            Some("user-1"),
        );

        // Dismissed should record "false_positive"
        integration.on_status_change(
            &TriageItemType::Insight,
            "insight-3",
            &TriageStatus::Dismissed,
            None,
            Some("Acme Corp"),
            None,
        );

        // No crash means routing worked
        let performances = integration.recipe_performances();
        // Both Actioned and Dismissed create performance records (both use recipe_code "triage")
        // Actioned → true_positives=1, Dismissed → false_positives=1
        // So we have 1 performance entry for recipe "triage" with tp=1, fp=1
        assert_eq!(performances.len(), 1);
        let (_recipe, precision, recall, f1) = &performances[0];
        // precision = tp/(tp+fp) = 1/(1+1) = 0.5
        assert!((*precision - 0.5).abs() < 0.001);
        // recall = tp/(tp+fn) = 1/(1+0) = 1.0
        assert!((*recall - 1.0).abs() < 0.001);
        // f1 = 2 * 0.5 * 1.0 / (0.5 + 1.0) = 1.0 / 1.5 = 0.666...
        assert!((*f1 - 2.0 / 3.0).abs() < 0.001);
    }
}
