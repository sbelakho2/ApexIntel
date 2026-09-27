//! Typed measurement semantics.
//!
//! A number that is not known is not zero. `Measurement` keeps "we measured a
//! value" apart from "we did not measure anything", "the measurement failed"
//! and "there is not enough evidence yet", so a caller can never silently
//! collapse a missing or failed observation into `0.0`, `0.5` or a default
//! that then reads as a real result downstream.

use serde::{Deserialize, Serialize};

/// Why a measurement could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureReason {
    /// Stable machine-readable code (e.g. `dns_query_failed`).
    pub code: String,
    /// Human-readable detail safe to log and surface.
    pub message: String,
}

impl FailureReason {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Render the reason as `code: message`.
    pub fn display(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

/// Outcome of one attempt to measure a quantity.
///
/// * [`Measurement::Measured`] — a real value, including a genuine zero.
/// * [`Measurement::NotMeasured`] — nothing was available to measure.
/// * [`Measurement::Unavailable`] — the attempt failed; the value is unknown.
/// * [`Measurement::InsufficientEvidence`] — data was present but not enough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
pub enum Measurement<T> {
    /// A real, trustworthy value.
    Measured(T),
    /// Nothing to measure (e.g. no captures, no samples).
    NotMeasured,
    /// The measurement attempt failed.
    Unavailable(FailureReason),
    /// Some data existed, but not enough to support a measurement.
    InsufficientEvidence,
}

impl<T> Measurement<T> {
    /// Wrap a real value. A genuine zero must use this variant, not a default.
    pub const fn measured(value: T) -> Self {
        Self::Measured(value)
    }

    /// Nothing was available to measure.
    pub const fn not_measured() -> Self {
        Self::NotMeasured
    }

    /// The measurement attempt failed.
    pub fn unavailable(reason: FailureReason) -> Self {
        Self::Unavailable(reason)
    }

    /// Data existed but was not sufficient.
    pub const fn insufficient_evidence() -> Self {
        Self::InsufficientEvidence
    }

    pub const fn is_measured(&self) -> bool {
        matches!(self, Self::Measured(_))
    }

    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }

    pub const fn is_insufficient_evidence(&self) -> bool {
        matches!(self, Self::InsufficientEvidence)
    }

    /// Borrow the measured value, if any.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Measured(value) => Some(value),
            _ => None,
        }
    }

    /// Copy the measured value, if any.
    pub fn value_copied(&self) -> Option<T>
    where
        T: Copy,
    {
        self.value().copied()
    }

    /// Failure detail for an unavailable measurement.
    pub fn failure_reason(&self) -> Option<&FailureReason> {
        match self {
            Self::Unavailable(reason) => Some(reason),
            _ => None,
        }
    }

    /// Stable machine-readable label for metrics and logs.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Measured(_) => "measured",
            Self::NotMeasured => "not_measured",
            Self::Unavailable(_) => "unavailable",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }

    /// Map a measured value while preserving every unmeasured state.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Measurement<U> {
        match self {
            Self::Measured(value) => Measurement::Measured(map(value)),
            Self::NotMeasured => Measurement::NotMeasured,
            Self::Unavailable(reason) => Measurement::Unavailable(reason),
            Self::InsufficientEvidence => Measurement::InsufficientEvidence,
        }
    }
}

impl Measurement<f64> {
    /// Render for reports: the value at fixed precision, or the reason it is
    /// absent. Never renders an unmeasured value as `0`.
    pub fn display_fixed(&self, decimals: usize) -> String {
        match self {
            Self::Measured(value) => format!("{value:.decimals$}"),
            Self::NotMeasured => "not measured".to_string(),
            Self::Unavailable(reason) => format!("unavailable ({})", reason.code),
            Self::InsufficientEvidence => "insufficient evidence".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_zero_is_distinct_from_unmeasured_states() {
        let zero = Measurement::measured(0.0_f64);
        assert!(zero.is_measured());
        assert_eq!(zero.value_copied(), Some(0.0));

        for state in [
            Measurement::<f64>::not_measured(),
            Measurement::insufficient_evidence(),
            Measurement::unavailable(FailureReason::new("query_failed", "db down")),
        ] {
            assert!(!state.is_measured());
            assert_eq!(state.value_copied(), None);
            assert_ne!(state, zero);
        }
    }

    #[test]
    fn label_is_stable_for_metrics() {
        assert_eq!(Measurement::measured(1_i64).label(), "measured");
        assert_eq!(Measurement::<i64>::not_measured().label(), "not_measured");
        assert_eq!(
            Measurement::<i64>::unavailable(FailureReason::new("x", "y")).label(),
            "unavailable"
        );
        assert_eq!(
            Measurement::<i64>::insufficient_evidence().label(),
            "insufficient_evidence"
        );
    }

    #[test]
    fn display_fixed_never_prints_zero_for_unmeasured() {
        assert_eq!(Measurement::measured(0.625_f64).display_fixed(2), "0.62");
        assert_eq!(
            Measurement::<f64>::not_measured().display_fixed(2),
            "not measured"
        );
        assert_eq!(
            Measurement::<f64>::unavailable(FailureReason::new("drift_db_down", "timeout"))
                .display_fixed(2),
            "unavailable (drift_db_down)"
        );
        assert_eq!(
            Measurement::<f64>::insufficient_evidence().display_fixed(2),
            "insufficient evidence"
        );
    }

    #[test]
    fn serde_roundtrip_preserves_state() {
        let values = [
            Measurement::measured(0.75_f64),
            Measurement::not_measured(),
            Measurement::unavailable(FailureReason::new("model_call_failed", "503")),
            Measurement::insufficient_evidence(),
        ];
        for value in values {
            let json = serde_json::to_string(&value).expect("serialize");
            let back: Measurement<f64> = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, value);
        }
    }

    #[test]
    fn map_preserves_unmeasured_states() {
        let measured = Measurement::measured(2_i32).map(|v| v * 3);
        assert_eq!(measured.value_copied(), Some(6));

        let unavailable =
            Measurement::<i32>::unavailable(FailureReason::new("x", "y")).map(|v| v * 3);
        assert!(unavailable.is_unavailable());
    }
}
