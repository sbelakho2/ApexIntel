use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DefaultKeyedStateStore};
use std::num::NonZeroU32;
use std::sync::Arc;

pub type DomainLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

/// Polite crawl governor enforcing per-domain and global rate limits.
pub struct CrawlGovernor {
    domain_limiter: Arc<DomainLimiter>,
    global_limiter: Arc<RateLimiter<governor::state::NotKeyed, governor::state::InMemoryState, DefaultClock>>,
    domain_rps: f64,
    global_rps: u32,
}

impl CrawlGovernor {
    /// Create with default limits: 1 req/domain/2.5s, 3 global concurrent.
    pub fn new() -> Self {
        Self::with_limits(1, 3)
    }

    /// Create with custom per-domain and global RPS limits.
    pub fn with_limits(domain_rps: u32, global_rps: u32) -> Self {
        let domain_rps = domain_rps.max(1);
        let global_rps = global_rps.max(1);
        let domain_quota = Quota::per_second(NonZeroU32::new(domain_rps).unwrap())
            .allow_burst(NonZeroU32::new(1).unwrap());
        let global_quota = Quota::per_second(NonZeroU32::new(global_rps).unwrap());

        Self {
            domain_limiter: Arc::new(RateLimiter::keyed(domain_quota)),
            global_limiter: Arc::new(RateLimiter::direct(global_quota)),
            domain_rps: domain_rps as f64,
            global_rps,
        }
    }

    /// Wait until a slot is available for the given domain.
    /// Acquires domain-local token first (more restrictive) to avoid holding
    /// a global token while waiting on the per-domain limiter.
    pub async fn wait_for_slot(&self, domain: &str) {
        self.domain_limiter.until_key_ready(&domain.to_string()).await;
        self.global_limiter.until_ready().await;
    }

    /// Try to acquire a slot without waiting. Returns true if acquired.
    /// Checks domain first: if domain fails, no global token is wasted.
    pub fn try_acquire(&self, domain: &str) -> bool {
        if self.domain_limiter.check_key(&domain.to_string()).is_err() {
            return false;
        }
        self.global_limiter.check().is_ok()
    }

    pub fn domain_rps(&self) -> f64 {
        self.domain_rps
    }

    pub fn global_rps(&self) -> u32 {
        self.global_rps
    }
}

impl Default for CrawlGovernor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_governor_creation() {
        let gov = CrawlGovernor::new();
        assert!((gov.domain_rps() - 1.0).abs() < f64::EPSILON);
        assert_eq!(gov.global_rps(), 3);
    }

    #[test]
    fn test_governor_custom_limits() {
        let gov = CrawlGovernor::with_limits(5, 10);
        assert!((gov.domain_rps() - 5.0).abs() < f64::EPSILON);
        assert_eq!(gov.global_rps(), 10);
    }

    #[test]
    fn test_governor_zero_limits_clamped() {
        let gov = CrawlGovernor::with_limits(0, 0);
        assert!((gov.domain_rps() - 1.0).abs() < f64::EPSILON);
        assert_eq!(gov.global_rps(), 1);
    }

    #[test]
    fn test_try_acquire_first_succeeds() {
        let gov = CrawlGovernor::with_limits(1, 10);
        // First acquire should succeed
        assert!(gov.try_acquire("example.com"));
    }

    #[test]
    fn test_try_acquire_rate_limited() {
        let gov = CrawlGovernor::with_limits(1, 10);
        // First should succeed
        assert!(gov.try_acquire("example.com"));
        // Second immediately should fail (1 per second limit)
        assert!(!gov.try_acquire("example.com"));
    }

    #[test]
    fn test_different_domains_independent() {
        let gov = CrawlGovernor::with_limits(1, 10);
        assert!(gov.try_acquire("domain1.com"));
        assert!(gov.try_acquire("domain2.com"));
    }

    #[tokio::test]
    async fn test_wait_for_slot() {
        let gov = CrawlGovernor::with_limits(10, 100);
        // Should complete quickly with high limits
        gov.wait_for_slot("example.com").await;
    }
}
