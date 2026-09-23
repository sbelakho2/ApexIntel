//! Adversarial regression tests for the core domain layer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::entities::{Observation, ObservationType};
use apex_core::triage::{composite_score, TriageDimensions, TriageWeights};
use apex_core::validation::{redact_secrets, round_to_dp};

#[test]
fn round_to_dp_stays_finite_for_absurd_precision() {
    // `10^400` overflows to +inf and previously produced NaN for finite input.
    assert_eq!(round_to_dp(0.0, 400), 0.0);
    assert!(round_to_dp(5.0, 400).is_finite());
    assert_eq!(round_to_dp(1.2345, 2), 1.23);
    assert!(round_to_dp(f64::NAN, 2).is_nan());
}

#[test]
fn redact_secrets_is_order_independent() {
    assert_eq!(
        redact_secrets("key=abcdef", &["abc", "abcdef"]),
        "key=[REDACTED]"
    );
    assert_eq!(
        redact_secrets("key=abcdef", &["abcdef", "abc"]),
        "key=[REDACTED]"
    );
    assert_eq!(redact_secrets("plain", &[""]), "plain");
}

#[test]
fn composite_score_normalises_weights_and_guards_nan() {
    let max_dims = TriageDimensions {
        urgency: 1.0,
        impact: 1.0,
        actionability: 1.0,
        novelty: 1.0,
        confidence: 1.0,
    };
    // Weights that sum to 3.0 previously produced a score of 3.0, bypassing all
    // severity bands.
    let heavy = TriageWeights {
        urgency: 0.6,
        impact: 0.7,
        actionability: 0.6,
        novelty: 0.6,
        confidence: 0.5,
    };
    let score = composite_score(&max_dims, &heavy);
    assert!((0.0..=1.0).contains(&score), "score={score}");

    // All-zero weights and NaN dimensions are both safe.
    let zero = TriageWeights {
        urgency: 0.0,
        impact: 0.0,
        actionability: 0.0,
        novelty: 0.0,
        confidence: 0.0,
    };
    assert_eq!(composite_score(&max_dims, &zero), 0.0);
    let nan_dims = TriageDimensions {
        urgency: f64::NAN,
        impact: 1.0,
        actionability: 1.0,
        novelty: 1.0,
        confidence: 1.0,
    };
    assert_eq!(composite_score(&nan_dims, &TriageWeights::default()), 0.0);
}

#[test]
fn stabilize_id_is_stable_across_array_order_and_volatile_fields() {
    // Objects nested inside arrays must be canonicalised (key order, volatile
    // fields) or dedup silently fails for list payloads.
    let mut a = Observation::new(
        ObservationType::WebChange,
        chrono::Utc::now(),
        serde_json::json!({"officers": [{"name": "x", "role": "y", "checked_at": "t1"}]}),
        serde_json::json!({"source": "s", "url": "u"}),
    );
    let mut b = Observation::new(
        ObservationType::WebChange,
        chrono::Utc::now(),
        serde_json::json!({"officers": [{"role": "y", "checked_at": "t2", "name": "x"}]}),
        serde_json::json!({"source": "s", "url": "u"}),
    );
    a.stabilize_id("web");
    b.stabilize_id("web");
    assert_eq!(a.id, b.id, "array/volatile canonicalisation failed");

    // A `|` inside a provenance field must not be confusable with a field
    // boundary.
    let mut c = Observation::new(
        ObservationType::WebChange,
        chrono::Utc::now(),
        serde_json::json!({"k": 1}),
        serde_json::json!({"source": "a|b", "source_id": "c"}),
    );
    let mut d = Observation::new(
        ObservationType::WebChange,
        chrono::Utc::now(),
        serde_json::json!({"k": 1}),
        serde_json::json!({"source": "a", "source_id": "b|c"}),
    );
    c.stabilize_id("web");
    d.stabilize_id("web");
    assert_ne!(c.id, d.id, "provenance delimiter ambiguity collided IDs");
}
