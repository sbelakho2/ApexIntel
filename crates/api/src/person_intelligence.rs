//! Canonical POI intelligence numbers (audit P2-18).
//!
//! Priority, influence, engagement and completeness are computed in exactly one
//! place — here — and consumed by the JSON API (`routes::persons`,
//! `mappings::person_row_to_item`, the person-detail endpoint) and by the
//! Askama views (`web::persons`). The previous code computed these in three
//! places with different formulas, including one that labelled influence as
//! priority and one that turned unknown values into zero vectors.
//!
//! Every number is either measured or explicitly absent:
//!
//! * `priority_score` comes from the **stored** priority vector; a missing
//!   vector yields `None` (no synthesized zero vector, no band).
//! * `influence_score` is the measured 0..=1 influence scaled to the legacy
//!   0..=100 API scale; `None` stays "not measured".
//! * `data_completeness` is the share of key profile fields that are populated
//!   (measured from the actual inputs), `None` when no checklist is supplied.

use serde::{Deserialize, Serialize};

use crate::config::PriorityWeights;
use crate::routes::persons::PriorityVector;

/// Priority band on the legacy A/B/C scale, measured from the priority score.
pub fn priority_band(score: f64) -> &'static str {
    if score >= 0.8 {
        "A"
    } else if score >= 0.5 {
        "B"
    } else {
        "C"
    }
}

/// Verbal priority tier, measured from the priority score.
pub fn priority_tier(score: f64) -> &'static str {
    if score >= 0.8 {
        "critical"
    } else if score >= 0.6 {
        "high"
    } else if score >= 0.4 {
        "medium"
    } else {
        "low"
    }
}

/// Influence tier on the 0..=100 scale; `None` (unmeasured) is explicit.
pub fn influence_tier(score: Option<i64>) -> &'static str {
    match score {
        Some(score) if score >= 70 => "high",
        Some(score) if score >= 40 => "medium",
        Some(_) => "low",
        None => "not measured",
    }
}

/// The measured intelligence view of one person, shared by every surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonIntelligenceView {
    /// Weighted composite of the stored priority vector; `None` when the vector
    /// is not stored or unparseable.
    pub priority_score: Option<f64>,
    /// A/B/C band; `None` when priority is unmeasured.
    pub priority_band: Option<String>,
    /// critical/high/medium/low tier; `None` when priority is unmeasured.
    pub priority_tier_label: Option<String>,
    /// Measured influence on the legacy 0..=100 scale.
    pub influence_score: Option<i64>,
    /// high/medium/low, or "not measured".
    pub influence_tier_label: String,
    /// Stored engagement classification, or "not measured" when absent.
    pub engagement_status: String,
    /// Only present when a real readiness computation supplies it.
    pub engagement_readiness: Option<f64>,
    /// Share (0.0..=1.0) of key profile fields that are populated; `None` when
    /// no checklist was supplied.
    pub data_completeness: Option<f64>,
}

impl PersonIntelligenceView {
    /// Build the view from canonical inputs.
    ///
    /// * `priority_vector` — the stored jsonb vector (as read from the row).
    /// * `influence` — measured influence 0..=1.
    /// * `engagement_status` — stored classification, if any.
    /// * `engagement_readiness` — only when a real computation exists.
    /// * `profile_fields` — booleans for the key fields used for completeness.
    pub fn from_measurements(
        priority_vector: Option<&serde_json::Value>,
        influence: Option<f64>,
        engagement_status: Option<&str>,
        engagement_readiness: Option<f64>,
        profile_fields: &[bool],
    ) -> Self {
        let priority_score = priority_vector
            .and_then(|value| serde_json::from_value::<PriorityVector>(value.clone()).ok())
            .map(|vector| vector.composite_with_weights(&PriorityWeights::default()));

        let influence_score = influence.map(|value| (value.clamp(0.0, 1.0) * 100.0).round() as i64);

        let engagement_status = match engagement_status.map(str::trim) {
            Some(status) if !status.is_empty() && status != "untracked" => status.to_string(),
            _ => "not measured".to_string(),
        };

        let data_completeness = if profile_fields.is_empty() {
            None
        } else {
            let filled = profile_fields.iter().filter(|filled| **filled).count();
            Some(filled as f64 / profile_fields.len() as f64)
        };

        Self {
            priority_score,
            priority_band: priority_score.map(|score| priority_band(score).to_string()),
            priority_tier_label: priority_score.map(|score| priority_tier(score).to_string()),
            influence_score,
            influence_tier_label: influence_tier(influence_score).to_string(),
            engagement_status,
            engagement_readiness,
            data_completeness,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn vector_json() -> serde_json::Value {
        serde_json::json!({
            "decision_power": 0.9,
            "domain_relevance": 0.8,
            "network_centrality": 0.7,
            "engagement_potential": 0.6,
            "intelligence_value": 0.8,
        })
    }

    /// A missing vector is unknown priority, not a zero-priority band.
    #[test]
    fn missing_priority_vector_is_not_a_zero_band() {
        let view = PersonIntelligenceView::from_measurements(None, None, None, None, &[]);
        assert_eq!(view.priority_score, None);
        assert_eq!(view.priority_band, None);
        assert_eq!(view.influence_score, None);
        assert_eq!(view.influence_tier_label, "not measured");
        assert_eq!(view.engagement_status, "not measured");
        assert_eq!(view.data_completeness, None);
    }

    /// Priority and influence are separate measurements: one never labels the
    /// other.
    #[test]
    fn priority_and_influence_are_independent() {
        let vector = vector_json();
        let view =
            PersonIntelligenceView::from_measurements(Some(&vector), Some(0.42), None, None, &[]);
        let priority = view.priority_score.expect("vector parses");
        assert!((0.0..=1.0).contains(&priority));
        assert_eq!(view.influence_score, Some(42));
        assert_eq!(view.influence_tier_label, "medium");

        let unscored = PersonIntelligenceView::from_measurements(None, Some(0.95), None, None, &[]);
        assert_eq!(unscored.priority_score, None);
        assert_eq!(unscored.influence_score, Some(95));
        assert_eq!(unscored.influence_tier_label, "high");
    }

    #[test]
    fn data_completeness_counts_supplied_fields() {
        let view = PersonIntelligenceView::from_measurements(
            None,
            None,
            None,
            None,
            &[true, true, false, false],
        );
        assert_eq!(view.data_completeness, Some(0.5));
    }

    #[test]
    fn untracked_engagement_is_not_measured() {
        let view =
            PersonIntelligenceView::from_measurements(None, None, Some("untracked"), None, &[]);
        assert_eq!(view.engagement_status, "not measured");
    }
}
