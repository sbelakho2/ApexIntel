//! Canonical POI priority model (audit P0).
//!
//! Priority is the weighted composite of the stored five-dimension priority
//! vector (`persons.priority_vector`); influence is a **separate measurement**
//! (`persons.influence_score`). The two must never be conflated, and an absent
//! vector means priority is *unmeasured* — never a zero score.
//!
//! This module is the single Rust implementation of the composite. The SQL
//! mirror lives in `migrations/091_persons_priority_score.sql` and the write
//! paths in `crates/store/src/postgres/persons.rs`; all three must use the
//! default weights below.

use serde::{Deserialize, Serialize};

use crate::validation::clamp_ratio;

/// The five stored priority dimensions, in weight order.
pub const PRIORITY_DIMENSIONS: [&str; 5] = [
    "decision_power",
    "domain_relevance",
    "network_centrality",
    "engagement_potential",
    "intelligence_value",
];

/// Default dimension weights. Must stay in sync with migration 091 and the
/// store write paths.
pub const DEFAULT_PRIORITY_WEIGHTS: [f64; 5] = [0.25, 0.20, 0.20, 0.15, 0.20];

/// The five-dimension priority vector as persisted on the person row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StoredPriorityVector {
    pub decision_power: f64,
    pub domain_relevance: f64,
    pub network_centrality: f64,
    pub engagement_potential: f64,
    pub intelligence_value: f64,
}

impl StoredPriorityVector {
    /// Parse the stored jsonb vector. Returns `None` unless **every**
    /// dimension is present and a finite number: a partial vector is
    /// unmeasured, not a zero-padded one.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        let dimension = |key: &str| -> Option<f64> {
            object
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
        };
        Some(Self {
            decision_power: dimension(PRIORITY_DIMENSIONS[0])?,
            domain_relevance: dimension(PRIORITY_DIMENSIONS[1])?,
            network_centrality: dimension(PRIORITY_DIMENSIONS[2])?,
            engagement_potential: dimension(PRIORITY_DIMENSIONS[3])?,
            intelligence_value: dimension(PRIORITY_DIMENSIONS[4])?,
        })
    }

    pub fn as_values(&self) -> [f64; 5] {
        [
            self.decision_power,
            self.domain_relevance,
            self.network_centrality,
            self.engagement_potential,
            self.intelligence_value,
        ]
    }

    /// Weighted composite with explicit weights, normalized to sum to one.
    /// A degenerate total weight falls back to the default weights.
    pub fn composite_with_weights(&self, weights: &[f64; 5]) -> f64 {
        let total_weight: f64 = weights.iter().sum();
        let normalized = if !total_weight.is_finite() || total_weight <= f64::EPSILON {
            DEFAULT_PRIORITY_WEIGHTS
        } else {
            [
                weights[0] / total_weight,
                weights[1] / total_weight,
                weights[2] / total_weight,
                weights[3] / total_weight,
                weights[4] / total_weight,
            ]
        };
        let values = self.as_values();
        let score: f64 = normalized
            .iter()
            .zip(values.iter())
            .map(|(weight, value)| weight * value)
            .sum();
        clamp_ratio(score)
    }

    /// Weighted composite with the default weights.
    pub fn composite(&self) -> f64 {
        self.composite_with_weights(&DEFAULT_PRIORITY_WEIGHTS)
    }
}

/// Canonical priority score from a stored jsonb vector; `None` when the
/// vector is absent or incomplete.
pub fn priority_score_from_json(value: &serde_json::Value) -> Option<f64> {
    StoredPriorityVector::from_json(value).map(|vector| vector.composite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_partial_vectors_are_unmeasured() {
        assert_eq!(priority_score_from_json(&serde_json::Value::Null), None);
        assert_eq!(
            priority_score_from_json(&serde_json::json!({"decision_power": 0.9})),
            None,
            "a partial vector must not be zero-padded"
        );
        assert_eq!(
            priority_score_from_json(&serde_json::json!({
                "cost": 1.0, "quality": 1.0, "speed": 1.0,
                "resilience": 1.0, "compliance": 1.0, "security": 1.0
            })),
            None,
            "the legacy six-dimension vector is not the canonical priority vector"
        );
    }

    #[test]
    fn composite_matches_the_declared_weights() {
        let vector = StoredPriorityVector {
            decision_power: 0.8,
            domain_relevance: 0.5,
            network_centrality: 0.25,
            engagement_potential: 1.0,
            intelligence_value: 0.0,
        };
        let expected = 0.25 * 0.8 + 0.20 * 0.5 + 0.20 * 0.25 + 0.15 * 1.0;
        assert!((vector.composite() - expected).abs() < 1e-12);
        assert_eq!(
            priority_score_from_json(&serde_json::json!({
                "decision_power": 1.0,
                "domain_relevance": 1.0,
                "network_centrality": 1.0,
                "engagement_potential": 1.0,
                "intelligence_value": 1.0,
            })),
            Some(1.0)
        );
    }

    #[test]
    fn degenerate_weights_fall_back_to_defaults() {
        let vector = StoredPriorityVector {
            decision_power: 1.0,
            domain_relevance: 0.0,
            network_centrality: 0.0,
            engagement_potential: 0.0,
            intelligence_value: 0.0,
        };
        assert!((vector.composite_with_weights(&[0.0; 5]) - 0.25).abs() < 1e-12);
    }
}
