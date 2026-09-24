//! Single source of truth for crawl concurrency limits.
//!
//! Browser rendering is deliberately serial: one persistent Chromium process
//! per worker, at most two isolated browser contexts/pages in flight, and a
//! global browser concurrency of two. Ordinary HTTP crawling is allowed to run
//! 8–16 requests in parallel (default 12); per-domain politeness stays in
//! [`crate::rate_limit`], which throttles each host independently of this
//! fleet-wide gate.

use std::sync::Arc;

use tokio::sync::Semaphore;

/// Number of persistent Chromium processes a worker keeps alive. One process
/// is shared by all browser fetches; extra pages are isolated contexts inside
/// it rather than extra processes.
pub const BROWSER_PROCESSES: usize = 1;

/// Maximum browser contexts/pages that may be active in the single process at
/// the same time.
pub const BROWSER_CONTEXTS: usize = 2;

/// Fleet-wide ceiling for concurrent browser fetches per worker. Matches
/// [`BROWSER_CONTEXTS`]: a page cannot outlive the context that owns it.
pub const GLOBAL_BROWSER_CONCURRENCY: usize = 2;

/// Lower bound for ordinary (non-browser) HTTP concurrency.
pub const MIN_HTTP_CONCURRENCY: usize = 8;

/// Upper bound for ordinary (non-browser) HTTP concurrency.
pub const MAX_HTTP_CONCURRENCY: usize = 16;

/// Default ordinary HTTP concurrency, inside the 8–16 window.
pub const DEFAULT_HTTP_CONCURRENCY: usize = 12;

/// Clamp a configured HTTP concurrency into the supported 8–16 window.
pub fn clamp_http_concurrency(configured: usize) -> usize {
    configured.clamp(MIN_HTTP_CONCURRENCY, MAX_HTTP_CONCURRENCY)
}

/// Bounded gate for browser fetches. Every fetch acquires one permit for the
/// lifetime of its page/context; the semaphore never admits more than
/// [`GLOBAL_BROWSER_CONCURRENCY`] pages.
#[derive(Clone)]
pub struct BrowserConcurrencyGate {
    semaphore: Arc<Semaphore>,
    limit: usize,
}

impl BrowserConcurrencyGate {
    pub fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub async fn acquire(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::AcquireError> {
        self.semaphore.clone().acquire_owned().await
    }

    #[cfg(test)]
    fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }
}

impl Default for BrowserConcurrencyGate {
    fn default() -> Self {
        Self::new(GLOBAL_BROWSER_CONCURRENCY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_limits_are_the_documented_constants() {
        assert_eq!(BROWSER_PROCESSES, 1, "one persistent Chromium process");
        assert_eq!(BROWSER_CONTEXTS, 2, "two contexts/pages per process");
        assert_eq!(
            GLOBAL_BROWSER_CONCURRENCY, 2,
            "two global concurrent browser fetches"
        );
        assert_eq!(
            GLOBAL_BROWSER_CONCURRENCY, BROWSER_CONTEXTS,
            "global browser ceiling cannot exceed the context budget"
        );
    }

    #[test]
    fn http_concurrency_default_lives_inside_the_supported_window() {
        assert!((MIN_HTTP_CONCURRENCY..=MAX_HTTP_CONCURRENCY).contains(&DEFAULT_HTTP_CONCURRENCY));
        assert!(
            (DEFAULT_HTTP_CONCURRENCY..=MAX_HTTP_CONCURRENCY).contains(&DEFAULT_HTTP_CONCURRENCY),
            "default must not exceed the upper bound"
        );
    }

    #[test]
    fn http_concurrency_is_clamped_to_eight_sixteen() {
        assert_eq!(clamp_http_concurrency(0), MIN_HTTP_CONCURRENCY);
        assert_eq!(clamp_http_concurrency(1), MIN_HTTP_CONCURRENCY);
        assert_eq!(clamp_http_concurrency(7), MIN_HTTP_CONCURRENCY);
        assert_eq!(clamp_http_concurrency(8), 8);
        assert_eq!(clamp_http_concurrency(12), 12);
        assert_eq!(clamp_http_concurrency(16), 16);
        assert_eq!(clamp_http_concurrency(17), MAX_HTTP_CONCURRENCY);
        assert_eq!(clamp_http_concurrency(usize::MAX), MAX_HTTP_CONCURRENCY);
    }

    #[test]
    fn browser_gate_defaults_to_two_permits() {
        let gate = BrowserConcurrencyGate::default();
        assert_eq!(gate.limit(), GLOBAL_BROWSER_CONCURRENCY);
    }

    #[test]
    fn browser_gate_admits_exactly_two_concurrent_fetches() {
        let gate = BrowserConcurrencyGate::new(GLOBAL_BROWSER_CONCURRENCY);
        let first = gate.try_acquire().expect("first permit");
        let second = gate.try_acquire().expect("second permit");
        assert!(
            gate.try_acquire().is_none(),
            "third concurrent browser fetch must be rejected"
        );
        drop(first);
        let third = gate.try_acquire().expect("permit after release");
        drop((second, third));
    }

    #[test]
    fn browser_gate_zero_is_raised_to_one() {
        let gate = BrowserConcurrencyGate::new(0);
        assert_eq!(gate.limit(), 1);
    }
}
