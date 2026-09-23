//! Job-level retry with exponential backoff and dead-letter queuing.
//!
//! When a job fails, this module determines whether to retry
//! (with configurable backoff) or permanently fail (routing to DLQ).
//!
//! # Integration
//!
//! Works with the worker trigger queue from migration 20260621 and
//! the existing dead_letter module for notification payloads.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

// ─── Configuration ──────────────────────────────────────────────────────────

/// Per-job-type retry policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (including the initial attempt).
    pub max_attempts: usize,
    /// Base delay between retries in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay between retries in milliseconds.
    pub max_delay_ms: u64,
    /// Whether to use jitter on backoff delays.
    pub use_jitter: bool,
    /// Whether to route to dead-letter queue after exhausting retries.
    pub route_to_dlq: bool,
    /// The category label for this job type (used for observability).
    pub job_category: String,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay_ms: 500,
            max_delay_ms: 60_000,
            use_jitter: true,
            route_to_dlq: true,
            job_category: "default".to_string(),
        }
    }
}

impl RetryPolicy {
    /// Retry policy for critical/high-priority jobs.
    pub fn critical() -> Self {
        Self {
            max_attempts: 6,
            base_delay_ms: 200,
            max_delay_ms: 30_000,
            use_jitter: true,
            route_to_dlq: true,
            job_category: "critical".to_string(),
        }
    }

    /// Retry policy for best-effort/low-priority jobs.
    pub fn best_effort() -> Self {
        Self {
            max_attempts: 2,
            base_delay_ms: 1_000,
            max_delay_ms: 120_000,
            use_jitter: true,
            route_to_dlq: false,
            job_category: "best_effort".to_string(),
        }
    }

    /// Retry policy for crawl jobs (aggressive retry).
    pub fn crawl() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 300,
            max_delay_ms: 45_000,
            use_jitter: true,
            route_to_dlq: true,
            job_category: "crawl".to_string(),
        }
    }

    /// Retry policy for enrichment jobs (conservative).
    pub fn enrichment() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1_000,
            max_delay_ms: 90_000,
            use_jitter: true,
            route_to_dlq: true,
            job_category: "enrichment".to_string(),
        }
    }
}

/// Compute the backoff delay for a given attempt.
pub fn compute_backoff(policy: &RetryPolicy, attempt: usize) -> Duration {
    let exponential = (policy.base_delay_ms as f64 * 2_f64.powi(attempt as i32)) as u64;
    let capped = exponential.min(policy.max_delay_ms);

    if !policy.use_jitter {
        return Duration::from_millis(capped);
    }

    // Simple pseudo-jitter using hash of timestamp mod capped
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let jittered = (now_ns.wrapping_mul(6364136223846793005).wrapping_add(1)) % (capped + 1);
    Duration::from_millis(jittered)
}

// ─── Job Retry State ────────────────────────────────────────────────────────

/// Tracks retry state for a job instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryState {
    /// Current attempt number (1-based).
    pub attempt: usize,
    /// Maximum allowed attempts.
    pub max_attempts: usize,
    /// The error from the last failed attempt.
    pub last_error: Option<String>,
    /// Whether the job has been routed to the dead-letter queue.
    pub in_dead_letter: bool,
    /// Timestamp of the next scheduled retry.
    pub next_retry_at_ms: Option<i64>,
}

impl RetryState {
    pub fn new(policy: &RetryPolicy) -> Self {
        Self {
            attempt: 1,
            max_attempts: policy.max_attempts,
            last_error: None,
            in_dead_letter: false,
            next_retry_at_ms: None,
        }
    }

    /// Returns true if the job should be retried.
    pub fn should_retry(&self) -> bool {
        self.attempt < self.max_attempts && !self.in_dead_letter
    }

    /// Record a failure and compute the next retry delay.
    pub fn record_failure(&mut self, error: &str, policy: &RetryPolicy) -> Option<Duration> {
        self.last_error = Some(error.to_string());
        self.attempt += 1;

        if self.should_retry() {
            let delay = compute_backoff(policy, self.attempt.saturating_sub(1));
            let now_ms = chrono::Utc::now().timestamp_millis();
            self.next_retry_at_ms = Some(now_ms + delay.as_millis() as i64);
            Some(delay)
        } else {
            warn!(
                attempt = self.attempt,
                max_attempts = self.max_attempts,
                error = %error,
                category = %policy.job_category,
                "job_retry_exhausted"
            );
            None
        }
    }

    /// Mark the job as routed to the dead-letter queue.
    pub fn route_to_dead_letter(&mut self) {
        self.in_dead_letter = true;
        info!("job_routed_to_dead_letter");
    }

    /// Reset retry state for a fresh attempt.
    pub fn reset(&mut self, policy: &RetryPolicy) {
        self.attempt = 1;
        self.max_attempts = policy.max_attempts;
        self.last_error = None;
        self.in_dead_letter = false;
        self.next_retry_at_ms = None;
    }
}

// ─── Retry Outcome ──────────────────────────────────────────────────────────

/// The result of retry decision logic.
#[derive(Debug, Clone)]
pub enum RetryOutcome {
    /// Job should be retried after the specified delay.
    Retry { delay: Duration, attempt: usize },
    /// Job should be permanently failed and routed to DLQ.
    PermanentFailure {
        attempt: usize,
        error: String,
        route_to_dlq: bool,
    },
    /// Job succeeded, no retry needed.
    Success,
}

/// Determine the retry outcome for a failed job.
pub fn evaluate_retry(state: &mut RetryState, error: &str, policy: &RetryPolicy) -> RetryOutcome {
    if let Some(delay) = state.record_failure(error, policy) {
        RetryOutcome::Retry {
            delay,
            attempt: state.attempt,
        }
    } else {
        if policy.route_to_dlq {
            state.route_to_dead_letter();
        }
        RetryOutcome::PermanentFailure {
            attempt: state.attempt,
            error: error.to_string(),
            route_to_dlq: policy.route_to_dlq,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_backoff_increases() {
        let policy = RetryPolicy {
            use_jitter: false,
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            ..Default::default()
        };

        assert_eq!(compute_backoff(&policy, 0), Duration::from_millis(100));
        assert_eq!(compute_backoff(&policy, 1), Duration::from_millis(200));
        assert_eq!(compute_backoff(&policy, 2), Duration::from_millis(400));
    }

    #[test]
    fn test_backoff_capped() {
        let policy = RetryPolicy {
            use_jitter: false,
            base_delay_ms: 100,
            max_delay_ms: 500,
            ..Default::default()
        };

        let d = compute_backoff(&policy, 10);
        assert_eq!(d, Duration::from_millis(500));
    }

    #[test]
    fn test_retry_state_should_retry() {
        let policy = RetryPolicy::default();
        let mut state = RetryState::new(&policy);
        assert!(state.should_retry()); // attempt 1 < max 4

        state.record_failure("test error", &policy);
        assert!(state.should_retry()); // attempt 2

        state.record_failure("test error", &policy);
        assert!(state.should_retry()); // attempt 3

        state.record_failure("test error", &policy);
        assert!(!state.should_retry()); // attempt 4 == max 4
    }

    #[test]
    fn test_retry_exhaustion() {
        // max_attempts = 3 means: initial try (attempt 1) + up to 2 retries
        // (attempts 2 and 3). After 2 failures (attempt 3), no more retries.
        let policy = RetryPolicy {
            max_attempts: 3,
            ..Default::default()
        };
        let mut state = RetryState::new(&policy);

        // First failure → attempt becomes 2, still < 3 → retry.
        let outcome = evaluate_retry(&mut state, "first error", &policy);
        assert!(matches!(outcome, RetryOutcome::Retry { .. }));

        // Second failure → attempt becomes 3, 3 < 3 = false → permanent failure.
        let outcome = evaluate_retry(&mut state, "second error", &policy);
        assert!(matches!(outcome, RetryOutcome::PermanentFailure { .. }));
        assert!(state.in_dead_letter);
    }

    #[test]
    fn test_best_effort_no_dlq() {
        let policy = RetryPolicy::best_effort();
        let mut state = RetryState::new(&policy);

        evaluate_retry(&mut state, "error", &policy);
        let outcome = evaluate_retry(&mut state, "error", &policy);
        assert!(
            matches!(outcome, RetryOutcome::PermanentFailure { route_to_dlq, .. } if !route_to_dlq)
        );
    }

    #[test]
    fn test_reset_state() {
        let policy = RetryPolicy::default();
        let mut state = RetryState::new(&policy);
        state.record_failure("error", &policy);
        state.record_failure("error", &policy);

        state.reset(&RetryPolicy::critical());
        assert_eq!(state.attempt, 1);
        assert_eq!(state.max_attempts, 6);
        assert!(state.last_error.is_none());
        assert!(!state.in_dead_letter);
    }
}
