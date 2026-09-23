//! Core triage types — shared across the triage engine, store, API, and worker.
//!
//! These types are in `apex-core` to avoid circular dependencies (the store crate
//! needs them but cannot depend on `apex-triage`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Triage Dimensions ──────────────────────────────────────────────────────

/// Five-dimensional triage score produced by LLM reasoning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageDimensions {
    /// How time-sensitive is this? (0.0 = not urgent, 1.0 = immediate action)
    pub urgency: f64,
    /// Potential business impact (0.0 = negligible, 1.0 = severe)
    pub impact: f64,
    /// Can the user take concrete action? (0.0 = informational only, 1.0 = actionable)
    pub actionability: f64,
    /// Is this new information or a repeat? (0.0 = stale/duplicate, 1.0 = novel)
    pub novelty: f64,
    /// LLM's confidence in this assessment (0.0 = guessing, 1.0 = certain)
    pub confidence: f64,
}

impl TriageDimensions {
    /// Clamp all dimension values to [0.0, 1.0].
    pub fn clamp(&mut self) {
        self.urgency = self.urgency.clamp(0.0, 1.0);
        self.impact = self.impact.clamp(0.0, 1.0);
        self.actionability = self.actionability.clamp(0.0, 1.0);
        self.novelty = self.novelty.clamp(0.0, 1.0);
        self.confidence = self.confidence.clamp(0.0, 1.0);
    }
}

// ─── Triage Item Type ──────────────────────────────────────────────────────

/// The item types that can be triaged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriageItemType {
    Insight,
    Warning,
    Alert,
}

impl TriageItemType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Insight => "insight",
            Self::Warning => "warning",
            Self::Alert => "alert",
        }
    }

    /// Parse a [`TriageItemType`] from its string representation.
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "insight" => Self::Insight,
            "warning" => Self::Warning,
            "alert" => Self::Alert,
            _ => Self::Insight,
        }
    }
}

// ─── Triage Status ─────────────────────────────────────────────────────────

/// Status of a triage queue item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriageStatus {
    Pending,
    Triaged,
    Acknowledged,
    Resolved,
    Dismissed,
}

impl TriageStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Triaged => "triaged",
            Self::Acknowledged => "acknowledged",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }

    /// Parse a [`TriageStatus`] from its string representation.
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "pending" => Self::Pending,
            "triaged" => Self::Triaged,
            "acknowledged" => Self::Acknowledged,
            "resolved" => Self::Resolved,
            "dismissed" => Self::Dismissed,
            _ => Self::Pending,
        }
    }
}

// ─── Triage Queue Item ─────────────────────────────────────────────────────

/// A single item in the triage queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageQueueItem {
    pub id: Uuid,
    pub item_type: TriageItemType,
    /// The ID of the source item (insight_id, warning_id, or alert_id)
    pub source_id: String,
    pub title: String,
    pub description: String,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<String>,
    /// Static severity from the source system
    pub static_severity: Option<String>,
    /// LLM-generated triage dimensions
    pub dimensions: Option<TriageDimensions>,
    /// Composite priority score (0.0 = lowest, 1.0 = highest)
    pub composite_score: f64,
    /// Whether a human has overridden the score
    pub is_overridden: bool,
    /// Human-overridden score (if any)
    pub override_score: Option<f64>,
    /// Triage status
    pub status: TriageStatus,
    pub created_at: DateTime<Utc>,
    pub triaged_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    /// Computed severity band label (e.g. "critical", "high", "medium", "low", "info")
    #[serde(default)]
    pub score_band: String,
    /// Computed CSS colour class for the severity band
    #[serde(default)]
    pub score_band_color: String,
}

// ─── Triage Weights ────────────────────────────────────────────────────────

/// Configuration for composite score calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageWeights {
    pub urgency: f64,
    pub impact: f64,
    pub actionability: f64,
    pub novelty: f64,
    pub confidence: f64,
}

impl Default for TriageWeights {
    fn default() -> Self {
        Self {
            urgency: 0.30,
            impact: 0.35,
            actionability: 0.15,
            novelty: 0.10,
            confidence: 0.10,
        }
    }
}

// ─── Triage Decision ──────────────────────────────────────────────────────

/// A historical record of a triage decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageDecision {
    pub id: Uuid,
    pub queue_item_id: Uuid,
    pub original_dimensions: TriageDimensions,
    pub original_composite: f64,
    pub override_dimensions: Option<TriageDimensions>,
    pub override_composite: Option<f64>,
    pub overridden_by: Option<String>, // user_id
    pub overridden_at: Option<DateTime<Utc>>,
    pub decision_type: TriageDecisionType,
    pub created_at: DateTime<Utc>,
}

/// Whether a triage decision was automatic or a human override.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriageDecisionType {
    AutoTriage,
    UserOverride,
}

// ─── Triage Thresholds ─────────────────────────────────────────────────────

/// Score thresholds for severity bands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageThresholds {
    pub critical: f64, // >= 0.80
    pub high: f64,     // >= 0.60
    pub medium: f64,   // >= 0.40
    pub low: f64,      // >= 0.20
}

impl Default for TriageThresholds {
    fn default() -> Self {
        Self {
            critical: 0.80,
            high: 0.60,
            medium: 0.40,
            low: 0.20,
        }
    }
}

// ─── Triage Stats ─────────────────────────────────────────────────────────

/// Statistics about the triage queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageStats {
    pub total: u64,
    pub pending: u64,
    pub triaged: u64,
    pub acknowledged: u64,
    pub resolved: u64,
    pub dismissed: u64,
    pub critical_count: u64,
    pub high_count: u64,
    pub medium_count: u64,
    pub low_count: u64,
    pub avg_urgency: f64,
    pub avg_impact: f64,
    pub avg_actionability: f64,
    pub avg_novelty: f64,
    pub avg_confidence: f64,
    pub avg_composite: f64,
    /// Count of items whose score has been manually overridden
    #[serde(default)]
    pub overridden_count: u64,
    /// Fraction of items whose score has been manually overridden
    pub override_rate: f64,
    /// Fraction of items that are resolved or dismissed
    #[serde(default)]
    pub resolution_rate: f64,
}

// ─── Composite Score Calculation ──────────────────────────────────────────

/// Calculate a composite priority score from dimensions and weights.
///
/// The result is normalised by the weight sum (so custom weights need not add
/// up to 1.0) and clamped to `[0.0, 1.0]`; non-finite inputs yield `0.0`.
pub fn composite_score(dimensions: &TriageDimensions, weights: &TriageWeights) -> f64 {
    let total_weight = weights.urgency
        + weights.impact
        + weights.actionability
        + weights.novelty
        + weights.confidence;
    if !total_weight.is_finite() || total_weight <= 0.0 {
        return 0.0;
    }
    let raw = dimensions.urgency * weights.urgency
        + dimensions.impact * weights.impact
        + dimensions.actionability * weights.actionability
        + dimensions.novelty * weights.novelty
        + dimensions.confidence * weights.confidence;
    let normalized = raw / total_weight;
    if normalized.is_finite() {
        normalized.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Map a composite score to a severity band label.
pub fn score_to_band(score: f64, thresholds: &TriageThresholds) -> &'static str {
    if score >= thresholds.critical {
        "critical"
    } else if score >= thresholds.high {
        "high"
    } else if score >= thresholds.medium {
        "medium"
    } else if score >= thresholds.low {
        "low"
    } else {
        "info"
    }
}

/// Map a composite score to a CSS colour class for severity bands.
pub fn score_band_color(score: f64, thresholds: &TriageThresholds) -> &'static str {
    if score >= thresholds.critical {
        "text-rams-red"
    } else if score >= thresholds.high {
        "text-rams-orange"
    } else if score >= thresholds.medium {
        "text-rams-yellow"
    } else if score >= thresholds.low {
        "text-rams-steel"
    } else {
        "text-rams-muted"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triage_dimensions_clamp() {
        let mut dims = TriageDimensions {
            urgency: -0.1,
            impact: 1.5,
            actionability: 0.5,
            novelty: 0.0,
            confidence: 1.0,
        };
        dims.clamp();
        assert!((dims.urgency - 0.0).abs() < 1e-9);
        assert!((dims.impact - 1.0).abs() < 1e-9);
        assert!((dims.actionability - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_composite_score_zero_weights() {
        let weights = TriageWeights {
            urgency: 0.0,
            impact: 0.0,
            actionability: 0.0,
            novelty: 0.0,
            confidence: 0.0,
        };
        let dims = TriageDimensions {
            urgency: 1.0,
            impact: 1.0,
            actionability: 1.0,
            novelty: 1.0,
            confidence: 1.0,
        };
        assert!((composite_score(&dims, &weights) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_composite_score_equal_weights() {
        let weights = TriageWeights {
            urgency: 0.2,
            impact: 0.2,
            actionability: 0.2,
            novelty: 0.2,
            confidence: 0.2,
        };
        let dims = TriageDimensions {
            urgency: 1.0,
            impact: 0.5,
            actionability: 0.0,
            novelty: 0.5,
            confidence: 1.0,
        };
        let score = composite_score(&dims, &weights);
        // (1.0 + 0.5 + 0.0 + 0.5 + 1.0) * 0.2 = 3.0 * 0.2 = 0.6
        assert!((score - 0.6).abs() < 1e-9);
    }

    #[test]
    fn test_composite_score_default_weights() {
        let weights = TriageWeights::default();
        let dims = TriageDimensions {
            urgency: 0.8,
            impact: 0.9,
            actionability: 0.6,
            novelty: 0.4,
            confidence: 0.7,
        };
        let score = composite_score(&dims, &weights);
        // 0.8*0.3 + 0.9*0.35 + 0.6*0.15 + 0.4*0.1 + 0.7*0.1
        // = 0.24 + 0.315 + 0.09 + 0.04 + 0.07 = 0.755
        assert!((score - 0.755).abs() < 1e-9);
    }

    #[test]
    fn test_score_to_band() {
        let thresholds = TriageThresholds::default();
        assert_eq!(score_to_band(0.90, &thresholds), "critical");
        assert_eq!(score_to_band(0.70, &thresholds), "high");
        assert_eq!(score_to_band(0.50, &thresholds), "medium");
        assert_eq!(score_to_band(0.30, &thresholds), "low");
        assert_eq!(score_to_band(0.10, &thresholds), "info");
    }

    #[test]
    fn test_triage_item_type_as_str() {
        assert_eq!(TriageItemType::Insight.as_str(), "insight");
        assert_eq!(TriageItemType::Warning.as_str(), "warning");
        assert_eq!(TriageItemType::Alert.as_str(), "alert");
    }

    #[test]
    fn test_triage_serde_roundtrip() {
        let dims = TriageDimensions {
            urgency: 0.8,
            impact: 0.9,
            actionability: 0.6,
            novelty: 0.4,
            confidence: 0.7,
        };
        let json = serde_json::to_string(&dims).unwrap();
        let parsed: TriageDimensions = serde_json::from_str(&json).unwrap();
        assert!((parsed.urgency - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_triage_weights_default() {
        let w = TriageWeights::default();
        assert!((w.urgency - 0.30).abs() < 1e-9);
        assert!((w.impact - 0.35).abs() < 1e-9);
        assert!((w.actionability - 0.15).abs() < 1e-9);
        assert!((w.novelty - 0.10).abs() < 1e-9);
        assert!((w.confidence - 0.10).abs() < 1e-9);
    }

    #[test]
    fn test_score_band_color() {
        let thresholds = TriageThresholds::default();
        assert_eq!(score_band_color(0.90, &thresholds), "text-rams-red");
        assert_eq!(score_band_color(0.70, &thresholds), "text-rams-orange");
        assert_eq!(score_band_color(0.50, &thresholds), "text-rams-yellow");
        assert_eq!(score_band_color(0.30, &thresholds), "text-rams-steel");
        assert_eq!(score_band_color(0.10, &thresholds), "text-rams-muted");
    }
}
