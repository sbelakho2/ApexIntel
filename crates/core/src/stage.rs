//! Structured stage outcomes.
//!
//! A learning or generation stage that fails must not be indistinguishable
//! from a stage that legitimately produced nothing. `StageResult` makes the
//! difference explicit: `Empty` is "ran, found nothing", `Failed` carries the
//! structured reason, and `Partial` keeps the value that *was* produced
//! together with the failure that limited it.

use serde::{Deserialize, Serialize};

/// Terminal status of one pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    /// The stage completed and produced its value.
    Success,
    /// The stage completed but there was nothing to produce.
    Empty,
    /// The stage failed; its value is unknown.
    Failed,
    /// The stage produced a value, but part of it failed.
    Partial,
}

impl StageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Empty => "empty",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }
}

/// Classification of a stage failure, used for metrics and alert routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The model/LLM call itself failed.
    ModelCallFailed,
    /// The model responded, but the response could not be parsed or validated.
    InvalidResponse,
    /// The parsed response was missing a required field.
    MissingField,
    /// A backing store or dependency failed.
    Storage,
    /// Anything else.
    Internal,
}

impl FailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModelCallFailed => "model_call_failed",
            Self::InvalidResponse => "invalid_response",
            Self::MissingField => "missing_field",
            Self::Storage => "storage",
            Self::Internal => "internal",
        }
    }
}

/// Serializable failure detail attached to a [`StageResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFailure {
    /// Stage identifier (e.g. `prompt_improvements`).
    pub stage: String,
    /// Failure classification.
    pub kind: FailureKind,
    /// Detail, including the underlying error text where available.
    pub message: String,
}

impl StructuredFailure {
    pub fn new(stage: impl Into<String>, kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            kind,
            message: message.into(),
        }
    }

    /// Render as `stage (kind): message`.
    pub fn display(&self) -> String {
        format!("{} ({}): {}", self.stage, self.kind.as_str(), self.message)
    }
}

/// Outcome of one pipeline stage.
///
/// Invariants enforced by the constructors:
/// * `Success` ⇒ `value` is `Some`, `failure` is `None`.
/// * `Empty` ⇒ both `value` and `failure` are `None`.
/// * `Failed` ⇒ `value` is `None`, `failure` is `Some`.
/// * `Partial` ⇒ both `value` and `failure` are `Some`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageResult<T> {
    pub status: StageStatus,
    pub value: Option<T>,
    pub failure: Option<StructuredFailure>,
}

impl<T> StageResult<T> {
    pub const fn success(value: T) -> Self {
        Self {
            status: StageStatus::Success,
            value: Some(value),
            failure: None,
        }
    }

    pub const fn empty() -> Self {
        Self {
            status: StageStatus::Empty,
            value: None,
            failure: None,
        }
    }

    pub fn failed(failure: StructuredFailure) -> Self {
        Self {
            status: StageStatus::Failed,
            value: None,
            failure: Some(failure),
        }
    }

    pub fn partial(value: T, failure: StructuredFailure) -> Self {
        Self {
            status: StageStatus::Partial,
            value: Some(value),
            failure: Some(failure),
        }
    }

    pub const fn is_success(&self) -> bool {
        matches!(self.status, StageStatus::Success)
    }

    pub const fn is_empty(&self) -> bool {
        matches!(self.status, StageStatus::Empty)
    }

    pub const fn is_failed(&self) -> bool {
        matches!(self.status, StageStatus::Failed)
    }

    pub const fn is_partial(&self) -> bool {
        matches!(self.status, StageStatus::Partial)
    }

    /// Whether the stage produced a usable value (success or partial).
    pub const fn has_value(&self) -> bool {
        self.value.is_some()
    }

    /// Whether any part of the stage failed.
    pub const fn has_failure(&self) -> bool {
        self.failure.is_some()
    }

    pub fn value_ref(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn failure_ref(&self) -> Option<&StructuredFailure> {
        self.failure.as_ref()
    }
}

impl<T> Default for StageResult<T> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_stage_never_looks_empty() {
        let failed: StageResult<Vec<i32>> = StageResult::failed(StructuredFailure::new(
            "prompt_improvements",
            FailureKind::InvalidResponse,
            "expected value at line 1 column 1",
        ));

        assert!(failed.is_failed());
        assert!(!failed.is_empty());
        assert!(!failed.is_success());
        assert!(failed.value_ref().is_none());
        let failure = failed.failure_ref().expect("failure retained");
        assert_eq!(failure.kind, FailureKind::InvalidResponse);
        assert!(failure.message.contains("expected value"));
    }

    #[test]
    fn empty_stage_is_not_a_failure() {
        let empty: StageResult<Vec<i32>> = StageResult::empty();
        assert!(empty.is_empty());
        assert!(!empty.has_failure());
        assert!(empty.value_ref().is_none());
    }

    #[test]
    fn partial_stage_keeps_value_and_failure() {
        let partial = StageResult::partial(
            vec![1, 2, 3],
            StructuredFailure::new("critique", FailureKind::ModelCallFailed, "timeout"),
        );
        assert!(partial.is_partial());
        assert!(partial.has_value());
        assert!(partial.has_failure());
        assert_eq!(partial.value_ref().map(Vec::len), Some(3));
    }

    #[test]
    fn constructors_preserve_invariants() {
        let success = StageResult::success(7);
        assert!(success.is_success());
        assert!(!success.has_failure());

        let defaulted: StageResult<i32> = StageResult::default();
        assert!(defaulted.is_empty());
    }

    #[test]
    fn serde_roundtrip_preserves_status_and_failure() {
        let value = StageResult::partial(
            "value".to_string(),
            StructuredFailure::new("stage", FailureKind::Storage, "db down"),
        );
        let json = serde_json::to_string(&value).expect("serialize");
        let back: StageResult<String> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, value);
        assert_eq!(back.status.as_str(), "partial");
    }
}
