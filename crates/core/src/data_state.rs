//! Explicit data-load state for repository-backed views.
//!
//! A failed repository call must never be rendered as an empty result set.
//! `DataState` forces every load path to distinguish "the query succeeded and
//! there is genuinely nothing to show" (`Empty`) from "the query failed"
//! (`Degraded`), so the UI can render a distinct degraded marker with an
//! incident id instead of a misleading "no results" state.

use chrono::{DateTime, Utc};

/// Outcome of a repository-backed load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataState<T> {
    /// The query succeeded and returned data.
    Loaded(T),
    /// The query succeeded and returned no data.
    Empty,
    /// The query failed; `error_id` identifies the incident for log correlation.
    Degraded { error_id: String },
}

impl<T> DataState<T> {
    /// Build a state from a repository result, classifying successful but
    /// empty payloads via `is_empty`. Failures are logged with a fresh
    /// incident id that is carried by the `Degraded` state.
    pub fn from_result<E: std::fmt::Display>(
        result: Result<T, E>,
        context: &str,
        is_empty: impl FnOnce(&T) -> bool,
    ) -> Self {
        match result {
            Ok(value) => {
                if is_empty(&value) {
                    Self::Empty
                } else {
                    Self::Loaded(value)
                }
            }
            Err(error) => {
                let error_id = new_incident_id();
                tracing::error!(incident_id = %error_id, "{context}: {error}");
                Self::Degraded { error_id }
            }
        }
    }

    /// Wrap an already-loaded value.
    pub fn loaded(value: T) -> Self {
        Self::Loaded(value)
    }

    /// Build a degraded state for a caller that already logged the failure.
    pub fn degraded(error_id: impl Into<String>) -> Self {
        Self::Degraded {
            error_id: error_id.into(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded { .. })
    }

    /// Incident id for a degraded state, if any.
    pub fn error_id(&self) -> Option<&str> {
        match self {
            Self::Degraded { error_id } => Some(error_id),
            _ => None,
        }
    }

    /// Borrow the state without consuming it.
    pub fn as_ref(&self) -> DataState<&T> {
        match self {
            Self::Loaded(value) => DataState::Loaded(value),
            Self::Empty => DataState::Empty,
            Self::Degraded { error_id } => DataState::Degraded {
                error_id: error_id.clone(),
            },
        }
    }

    /// Map the loaded value while preserving `Empty`/`Degraded`.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> DataState<U> {
        match self {
            Self::Loaded(value) => DataState::Loaded(map(value)),
            Self::Empty => DataState::Empty,
            Self::Degraded { error_id } => DataState::Degraded { error_id },
        }
    }

    /// Consume the state, using `fallback` for `Empty`/`Degraded`.
    ///
    /// The degraded marker must still be rendered separately; this only
    /// supplies the display payload.
    pub fn into_loaded_or(self, fallback: T) -> T {
        match self {
            Self::Loaded(value) => value,
            Self::Empty | Self::Degraded { .. } => fallback,
        }
    }

    /// Consume the state, using `T::default()` for `Empty`/`Degraded`.
    pub fn into_loaded_or_default(self) -> T
    where
        T: Default,
    {
        self.into_loaded_or(T::default())
    }
}

impl<T> DataState<Vec<T>> {
    /// Consume a list load. `Empty` and `Degraded` both produce an empty vec;
    /// callers MUST render [`DegradedNotice`] when the state was degraded so
    /// the empty vec is never presented as "zero results".
    pub fn into_items(self) -> Vec<T> {
        match self {
            Self::Loaded(items) => items,
            Self::Empty | Self::Degraded { .. } => Vec::new(),
        }
    }
}

/// Human-facing degraded marker: "Data unavailable — query failed at HH:MM UTC
/// · incident <id>".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradedNotice {
    pub error_id: String,
    pub failed_at: DateTime<Utc>,
}

impl DegradedNotice {
    pub fn new(error_id: impl Into<String>, failed_at: DateTime<Utc>) -> Self {
        Self {
            error_id: error_id.into(),
            failed_at,
        }
    }

    /// Build a notice from a degraded state; `None` for healthy states.
    pub fn from_state<T>(state: &DataState<T>) -> Option<Self> {
        state
            .error_id()
            .map(|error_id| Self::new(error_id.to_string(), Utc::now()))
    }

    /// Record the first degraded state seen into `slot`, preserving earlier
    /// notices so a page reports its earliest failure.
    pub fn capture<T>(state: &DataState<T>, slot: &mut Option<String>) {
        if slot.is_none() {
            *slot = Self::from_state(state).map(|notice| notice.message());
        }
    }

    /// Render the degraded marker text.
    pub fn message(&self) -> String {
        format!(
            "Data unavailable — query failed at {} UTC · incident {}",
            self.failed_at.format("%H:%M"),
            self.error_id
        )
    }
}

/// Short incident identifier attached to failed loads and logged with the error.
pub fn new_incident_id() -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("inc-{}", &uuid[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_non_empty_load_is_loaded() {
        let state: DataState<Vec<i32>> =
            DataState::from_result(Ok::<Vec<i32>, String>(vec![1, 2, 3]), "test", |v| {
                v.is_empty()
            });

        assert!(state.is_loaded());
        assert!(!state.is_empty());
        assert!(!state.is_degraded());
        assert_eq!(state.into_items(), vec![1, 2, 3]);
    }

    #[test]
    fn successful_empty_load_is_empty_not_degraded() {
        let state: DataState<Vec<i32>> =
            DataState::from_result(Ok::<Vec<i32>, String>(Vec::new()), "test", |v| v.is_empty());

        assert!(state.is_empty());
        assert!(!state.is_loaded());
        assert!(!state.is_degraded());
        assert!(state.error_id().is_none());
    }

    #[test]
    fn failed_load_is_degraded_with_incident_id() {
        let state: DataState<Vec<i32>> =
            DataState::from_result(Err("connection refused"), "test", |v| v.is_empty());

        assert!(state.is_degraded());
        assert!(!state.is_empty());
        let error_id = state.error_id().expect("degraded state carries an id");
        assert!(error_id.starts_with("inc-"), "unexpected id: {error_id}");
        assert_eq!(error_id.len(), 12);
    }

    #[test]
    fn degraded_state_maps_preserving_error_id() {
        let state: DataState<Vec<i32>> = DataState::degraded("inc-abc12345");
        let mapped = state.map(|items| items.len());

        assert_eq!(
            mapped,
            DataState::Degraded {
                error_id: "inc-abc12345".to_string()
            }
        );
        assert_eq!(mapped.error_id(), Some("inc-abc12345"));
    }

    #[test]
    fn into_loaded_or_default_never_masks_failure_as_data() {
        let degraded: DataState<Vec<i32>> = DataState::degraded("inc-abc12345");
        let empty: DataState<Vec<i32>> = DataState::Empty;

        assert_eq!(degraded.clone().into_items(), Vec::<i32>::new());
        assert_eq!(degraded.into_loaded_or_default(), Vec::<i32>::new());
        assert_eq!(empty.into_loaded_or_default(), Vec::<i32>::new());
    }

    #[test]
    fn degraded_notice_message_matches_ui_contract() {
        let failed_at = DateTime::parse_from_rfc3339("2026-09-24T14:03:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let notice = DegradedNotice::new("inc-1a2b3c4d", failed_at);

        assert_eq!(
            notice.message(),
            "Data unavailable — query failed at 14:03 UTC · incident inc-1a2b3c4d"
        );
    }

    #[test]
    fn capture_records_first_failure_only() {
        let first: DataState<Vec<i32>> = DataState::degraded("inc-first111");
        let second: DataState<Vec<i32>> = DataState::degraded("inc-second22");
        let mut slot = None;

        DegradedNotice::capture(&first, &mut slot);
        DegradedNotice::capture(&second, &mut slot);

        let notice = slot.expect("first failure captured");
        assert!(notice.contains("inc-first111"));
        assert!(!notice.contains("inc-second22"));

        let healthy: DataState<Vec<i32>> = DataState::Loaded(vec![]);
        let mut slot = None;
        DegradedNotice::capture(&healthy, &mut slot);
        assert!(slot.is_none());
    }
}
