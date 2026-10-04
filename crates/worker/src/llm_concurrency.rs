//! Process-wide LLM concurrency gate (#92).
//!
//! A single local model endpoint serves every worker job. Without a cap, the
//! scheduler could run several LLM-heavy jobs at once (insight generation,
//! recipe fire, POI extraction, embeddings, triage scoring, self-improvement
//! evaluation) and overload it until every call times out.
//!
//! Every worker call site acquires one of `APEX_LLM_MAX_CONCURRENCY` (default
//! 2) process-wide permits immediately before sending a model request, so at
//! most N requests are in flight across all jobs. The scheduler additionally
//! probes this gate before claiming an LLM-heavy run (see `runtime.rs`), so a
//! saturated gate leaves waiting runs queued for the next tick instead of
//! starting work that would immediately block on the gate.
//!
//! The permit is held around a single model call, never for a whole run: a run
//! that held a permit for its lifetime while its own calls acquired more
//! permits from the same gate could deadlock (two runs, two permits, no
//! permit left for their calls).

use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Default concurrent model calls for the whole worker process.
pub const DEFAULT_LLM_MAX_CONCURRENCY: usize = 2;

/// Upper bound so a misconfigured environment cannot remove the limit.
pub const MAX_LLM_MAX_CONCURRENCY: usize = 64;

struct LlmGate {
    semaphore: Arc<Semaphore>,
    capacity: usize,
}

static LLM_GATE: OnceLock<LlmGate> = OnceLock::new();

/// Parse `APEX_LLM_MAX_CONCURRENCY` into a usable permit count.
///
/// Unset, empty, non-numeric, zero and values above
/// [`MAX_LLM_MAX_CONCURRENCY`] resolve to the default/clamp; this is the pure
/// part of the gate so it can be unit-tested without touching the process-wide
/// singleton.
pub fn resolve_llm_max_concurrency(raw: Option<&str>) -> usize {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_LLM_MAX_CONCURRENCY)
        .min(MAX_LLM_MAX_CONCURRENCY)
}

fn gate() -> &'static LlmGate {
    LLM_GATE.get_or_init(|| {
        let capacity =
            resolve_llm_max_concurrency(std::env::var("APEX_LLM_MAX_CONCURRENCY").ok().as_deref());
        tracing::info!(
            capacity,
            "llm concurrency gate initialized (APEX_LLM_MAX_CONCURRENCY)"
        );
        LlmGate {
            semaphore: Arc::new(Semaphore::new(capacity)),
            capacity,
        }
    })
}

/// Configured permit count for the process-wide gate.
pub fn llm_gate_capacity() -> usize {
    gate().capacity
}

/// Currently unclaimed permit count (diagnostics and scheduler admission).
pub fn available_llm_slots() -> usize {
    gate().semaphore.available_permits()
}

/// Acquire a model-call slot, waiting until one is free.
///
/// The gate is never closed, so this only returns `None` if the runtime was
/// unable to allocate; callers treat that as "no slot" and skip the call
/// rather than panicking.
pub async fn acquire_llm_slot() -> Option<OwnedSemaphorePermit> {
    match Arc::clone(&gate().semaphore).acquire_owned().await {
        Ok(permit) => Some(permit),
        Err(error) => {
            // The gate is never closed; this only happens during runtime
            // teardown. Surface it instead of silently dropping the slot.
            tracing::error!(%error, "llm_concurrency: failed to acquire model-call slot");
            None
        }
    }
}

/// Try to acquire a model-call slot immediately.
pub fn try_acquire_llm_slot() -> Option<OwnedSemaphorePermit> {
    Arc::clone(&gate().semaphore).try_acquire_owned().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        available_llm_slots, resolve_llm_max_concurrency, DEFAULT_LLM_MAX_CONCURRENCY,
        MAX_LLM_MAX_CONCURRENCY,
    };

    #[test]
    fn unset_empty_and_invalid_values_default_to_two() {
        assert_eq!(resolve_llm_max_concurrency(None), 2);
        assert_eq!(resolve_llm_max_concurrency(Some("")), 2);
        assert_eq!(resolve_llm_max_concurrency(Some("   ")), 2);
        assert_eq!(resolve_llm_max_concurrency(Some("abc")), 2);
        assert_eq!(resolve_llm_max_concurrency(Some("0")), 2);
        assert_eq!(resolve_llm_max_concurrency(Some("-3")), 2);
    }

    #[test]
    fn explicit_values_are_parsed_and_upper_clamped() {
        assert_eq!(resolve_llm_max_concurrency(Some("1")), 1);
        assert_eq!(resolve_llm_max_concurrency(Some("2")), 2);
        assert_eq!(resolve_llm_max_concurrency(Some(" 12 ")), 12);
        assert_eq!(
            resolve_llm_max_concurrency(Some("9999")),
            MAX_LLM_MAX_CONCURRENCY
        );
        assert_eq!(DEFAULT_LLM_MAX_CONCURRENCY, 2);
    }

    #[test]
    fn gate_reports_a_positive_capacity_and_available_slots() {
        assert!(super::llm_gate_capacity() >= 1);
        assert!(available_llm_slots() <= super::llm_gate_capacity());
    }
}
