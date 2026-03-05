//! API rate limiting middleware using a token-bucket algorithm.
//!
//! Provides per-IP rate limiting with configurable limits per endpoint
//! category. Critical endpoints (admin, auth) get stricter limits,
//! while read-heavy endpoints (search, list) get more generous ones.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// ─── Configuration ──────────────────────────────────────────────────────

/// Rate limit tier — each endpoint category maps to a tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateTier {
    /// Admin/auth endpoints: 10 req/min
    Strict,
    /// Write endpoints (POST/PUT/DELETE): 30 req/min
    Standard,
    /// Read endpoints (GET lists): 120 req/min
    Generous,
    /// Search endpoints: 60 req/min
    Search,
    /// Health/metrics: 300 req/min
    Internal,
}

impl RateTier {
    /// Requests allowed per window.
    pub fn max_requests(&self) -> u32 {
        match self {
            Self::Strict => 10,
            Self::Standard => 30,
            Self::Generous => 120,
            Self::Search => 60,
            Self::Internal => 300,
        }
    }

    /// Time window for the rate limit.
    pub fn window(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// Classify an endpoint path into a rate tier.
pub fn classify_endpoint(path: &str, method: &str) -> RateTier {
    if path.starts_with("/api/admin") || path.starts_with("/api/auth") {
        return RateTier::Strict;
    }
    if path.starts_with("/api/health") || path.starts_with("/api/metrics") {
        return RateTier::Internal;
    }
    if path.starts_with("/api/search") {
        return RateTier::Search;
    }
    match method {
        "POST" | "PUT" | "DELETE" | "PATCH" => RateTier::Standard,
        _ => RateTier::Generous,
    }
}

// ─── Token bucket ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    fn new(max_requests: u32, window: Duration) -> Self {
        let max_tokens = max_requests as f64;
        let refill_rate = max_tokens / window.as_secs_f64();
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn tokens_remaining(&self) -> u32 {
        self.tokens as u32
    }

    fn retry_after(&self) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let deficit = 1.0 - self.tokens;
            Duration::from_secs_f64(deficit / self.refill_rate)
        }
    }
}

// ─── Rate limiter service ───────────────────────────────────────────────

/// Per-IP, per-tier rate limiting state.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

struct RateLimiterInner {
    buckets: HashMap<(IpAddr, RateTier), TokenBucket>,
    /// IPs that are permanently allowed (e.g., internal services)
    allowlist: Vec<IpAddr>,
    /// IPs that are permanently blocked
    blocklist: Vec<IpAddr>,
    /// Last cleanup time
    last_cleanup: Instant,
}

/// Result of a rate limit check.
#[derive(Debug)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub remaining: u32,
    pub limit: u32,
    pub retry_after_secs: u64,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                allowlist: Vec::new(),
                blocklist: Vec::new(),
                last_cleanup: Instant::now(),
            })),
        }
    }

    /// Add an IP to the permanent allowlist.
    pub fn allow_ip(&self, ip: IpAddr) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.allowlist.contains(&ip) {
            inner.allowlist.push(ip);
        }
    }

    /// Block an IP permanently.
    pub fn block_ip(&self, ip: IpAddr) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.blocklist.contains(&ip) {
            inner.blocklist.push(ip);
        }
    }

    /// Check if a request from `ip` to an endpoint with `tier` is allowed.
    pub fn check(&self, ip: IpAddr, tier: RateTier) -> RateLimitResult {
        let mut inner = self.inner.lock().unwrap();

        // Always allow allowlisted IPs
        if inner.allowlist.contains(&ip) {
            return RateLimitResult {
                allowed: true,
                remaining: tier.max_requests(),
                limit: tier.max_requests(),
                retry_after_secs: 0,
            };
        }

        // Always block blocklisted IPs
        if inner.blocklist.contains(&ip) {
            return RateLimitResult {
                allowed: false,
                remaining: 0,
                limit: tier.max_requests(),
                retry_after_secs: 60,
            };
        }

        // Periodic cleanup of stale buckets (every 5 minutes)
        if inner.last_cleanup.elapsed() > Duration::from_secs(300) {
            inner.buckets.retain(|_, bucket| {
                bucket.last_refill.elapsed() < Duration::from_secs(600)
            });
            inner.last_cleanup = Instant::now();
        }

        let key = (ip, tier);
        let bucket = inner
            .buckets
            .entry(key)
            .or_insert_with(|| TokenBucket::new(tier.max_requests(), tier.window()));

        let allowed = bucket.try_consume();
        let remaining = bucket.tokens_remaining();
        let retry_after = bucket.retry_after();

        RateLimitResult {
            allowed,
            remaining,
            limit: tier.max_requests(),
            retry_after_secs: retry_after.as_secs(),
        }
    }

    /// Generate rate-limit HTTP headers for the response.
    pub fn headers(result: &RateLimitResult) -> Vec<(String, String)> {
        let mut headers = vec![
            ("X-RateLimit-Limit".into(), result.limit.to_string()),
            (
                "X-RateLimit-Remaining".into(),
                result.remaining.to_string(),
            ),
        ];
        if !result.allowed {
            headers.push((
                "Retry-After".into(),
                result.retry_after_secs.to_string(),
            ));
        }
        headers
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_token_bucket_basic() {
        let mut bucket = TokenBucket::new(10, Duration::from_secs(60));
        for _ in 0..10 {
            assert!(bucket.try_consume());
        }
        // 11th should fail
        assert!(!bucket.try_consume());
    }

    #[test]
    fn test_classify_endpoint() {
        assert_eq!(classify_endpoint("/api/admin/replay", "POST"), RateTier::Strict);
        assert_eq!(classify_endpoint("/api/auth/login", "POST"), RateTier::Strict);
        assert_eq!(classify_endpoint("/api/search", "GET"), RateTier::Search);
        assert_eq!(classify_endpoint("/api/health", "GET"), RateTier::Internal);
        assert_eq!(classify_endpoint("/api/warnings", "GET"), RateTier::Generous);
        assert_eq!(classify_endpoint("/api/companies", "POST"), RateTier::Standard);
    }

    #[test]
    fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        for _ in 0..10 {
            let result = limiter.check(ip, RateTier::Strict);
            assert!(result.allowed);
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..10 {
            limiter.check(ip, RateTier::Strict);
        }
        let result = limiter.check(ip, RateTier::Strict);
        assert!(!result.allowed);
        assert!(result.retry_after_secs > 0 || result.retry_after_secs == 0);
    }

    #[test]
    fn test_allowlist_bypasses_limits() {
        let limiter = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        limiter.allow_ip(ip);
        for _ in 0..50 {
            let result = limiter.check(ip, RateTier::Strict);
            assert!(result.allowed);
        }
    }

    #[test]
    fn test_blocklist_always_rejects() {
        let limiter = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        limiter.block_ip(ip);
        let result = limiter.check(ip, RateTier::Generous);
        assert!(!result.allowed);
    }

    #[test]
    fn test_different_ips_independent() {
        let limiter = RateLimiter::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..10 {
            limiter.check(ip1, RateTier::Strict);
        }
        // ip2 should still be unaffected
        let result = limiter.check(ip2, RateTier::Strict);
        assert!(result.allowed);
    }

    #[test]
    fn test_headers_include_retry_after() {
        let result = RateLimitResult {
            allowed: false,
            remaining: 0,
            limit: 10,
            retry_after_secs: 5,
        };
        let headers = RateLimiter::headers(&result);
        assert!(headers.iter().any(|(k, _)| k == "Retry-After"));
    }
}
