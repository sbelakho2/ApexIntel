//! API rate limiting middleware using a token-bucket algorithm.
//!
//! Provides per-IP rate limiting with configurable limits per endpoint
//! category. Critical endpoints (admin, auth) get stricter limits,
//! while read-heavy endpoints (search, list) get more generous ones.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

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

    pub fn label(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Standard => "standard",
            Self::Generous => "generous",
            Self::Search => "search",
            Self::Internal => "internal",
        }
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

/// Environment flag that *requests* disabling the limiter. It is never
/// sufficient on its own: [`ALLOW_TEST_RATE_LIMIT_DISABLED_ENV`] must also be
/// `1`, and production startup refuses the variable otherwise.
pub const RATE_LIMIT_DISABLED_ENV: &str = "RATE_LIMIT_DISABLED";

/// Explicit test/CI override, same pattern as `APEX_ALLOW_TEST_SESSION_SECRET`.
/// Only a literal `1` counts.
pub const ALLOW_TEST_RATE_LIMIT_DISABLED_ENV: &str = "APEX_ALLOW_TEST_RATE_LIMIT_DISABLED";

fn env_truthy(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// True when the environment variable *asks* for the limiter to be disabled.
pub fn rate_limit_disabled_requested(value: Option<&str>) -> bool {
    value.is_some_and(env_truthy)
}

/// True only for the explicit test override (`APEX_ALLOW_TEST_RATE_LIMIT_DISABLED=1`).
pub fn test_rate_limit_disable_allowed(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether limit enforcement may actually be disabled. Both the request and
/// the explicit test override are required: a production deployment cannot
/// silently turn off rate limiting by setting a single variable.
pub fn should_disable_rate_limit(
    rate_limit_disabled: Option<&str>,
    allow_test_override: Option<&str>,
) -> bool {
    rate_limit_disabled_requested(rate_limit_disabled)
        && test_rate_limit_disable_allowed(allow_test_override)
}

/// Startup validation mirroring `validate_session_secret`: refuse to boot when
/// `RATE_LIMIT_DISABLED` is set without `APEX_ALLOW_TEST_RATE_LIMIT_DISABLED=1`.
/// Returns a plain string so callers decide how to surface it (the binary maps
/// it to an `anyhow` startup error).
pub fn validate_rate_limit_disable(
    rate_limit_disabled: Option<&str>,
    allow_test_override: Option<&str>,
) -> Result<(), String> {
    if rate_limit_disabled_requested(rate_limit_disabled)
        && !test_rate_limit_disable_allowed(allow_test_override)
    {
        return Err(format!(
            "{RATE_LIMIT_DISABLED_ENV} is set but {ALLOW_TEST_RATE_LIMIT_DISABLED_ENV}=1 is not; \
             refusing to start with rate limiting disabled"
        ));
    }
    Ok(())
}

/// Per-IP, per-tier rate limiting state.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
    /// Test/dev escape hatch. `RATE_LIMIT_DISABLED=true` only takes effect
    /// together with `APEX_ALLOW_TEST_RATE_LIMIT_DISABLED=1`; it is used by the
    /// UI e2e suite, which sweeps every route rapidly. Startup validation in
    /// `main.rs` refuses the request without the override.
    disabled: bool,
}

struct RateLimiterInner {
    buckets: HashMap<(String, String, u32, u64), TokenBucket>,
    allowlist: Vec<String>,
    blocklist: Vec<String>,
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
        let disabled = should_disable_rate_limit(
            std::env::var(RATE_LIMIT_DISABLED_ENV).ok().as_deref(),
            std::env::var(ALLOW_TEST_RATE_LIMIT_DISABLED_ENV)
                .ok()
                .as_deref(),
        );
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                allowlist: Vec::new(),
                blocklist: Vec::new(),
                last_cleanup: Instant::now(),
            })),
            disabled,
        }
    }

    pub fn allow_identifier(&self, identifier: &str) {
        let mut inner = self.inner.lock();
        if !inner.allowlist.iter().any(|allowed| allowed == identifier) {
            inner.allowlist.push(identifier.to_string());
        }
    }

    pub fn block_identifier(&self, identifier: &str) {
        let mut inner = self.inner.lock();
        if !inner.blocklist.iter().any(|blocked| blocked == identifier) {
            inner.blocklist.push(identifier.to_string());
        }
    }

    pub fn check(&self, identifier: &str, tier: RateTier) -> RateLimitResult {
        self.check_with_limit(identifier, tier.label(), tier.max_requests(), tier.window())
    }

    pub fn check_with_limit(
        &self,
        identifier: &str,
        bucket_name: &str,
        max_requests: u32,
        window: Duration,
    ) -> RateLimitResult {
        if self.disabled {
            return RateLimitResult {
                allowed: true,
                remaining: max_requests,
                limit: max_requests,
                retry_after_secs: 0,
            };
        }
        let mut inner = self.inner.lock();

        if inner.allowlist.iter().any(|allowed| allowed == identifier) {
            return RateLimitResult {
                allowed: true,
                remaining: max_requests,
                limit: max_requests,
                retry_after_secs: 0,
            };
        }

        if inner.blocklist.iter().any(|blocked| blocked == identifier) {
            return RateLimitResult {
                allowed: false,
                remaining: 0,
                limit: max_requests,
                retry_after_secs: window.as_secs(),
            };
        }

        if inner.last_cleanup.elapsed() > Duration::from_secs(300) {
            inner
                .buckets
                .retain(|_, bucket| bucket.last_refill.elapsed() < Duration::from_secs(600));
            inner.last_cleanup = Instant::now();
        }

        let key = (
            identifier.to_string(),
            bucket_name.to_string(),
            max_requests,
            window.as_secs(),
        );
        let bucket = inner
            .buckets
            .entry(key)
            .or_insert_with(|| TokenBucket::new(max_requests, window));

        let allowed = bucket.try_consume();
        let remaining = bucket.tokens_remaining();
        let retry_after = bucket.retry_after();

        RateLimitResult {
            allowed,
            remaining,
            limit: max_requests,
            // Never report 0 for a rejected request: `as_secs()` truncates
            // sub-second waits to 0, which told clients to retry immediately.
            retry_after_secs: if allowed {
                0
            } else {
                retry_after.as_secs_f64().ceil().max(1.0) as u64
            },
        }
    }

    pub fn allow_ip(&self, ip: std::net::IpAddr) {
        self.allow_identifier(&ip.to_string());
    }

    pub fn block_ip(&self, ip: std::net::IpAddr) {
        self.block_identifier(&ip.to_string());
    }

    pub fn check_ip(&self, ip: std::net::IpAddr, tier: RateTier) -> RateLimitResult {
        self.check(&ip.to_string(), tier)
    }

    /// Generate rate-limit HTTP headers for the response.
    pub fn headers(result: &RateLimitResult) -> Vec<(String, String)> {
        let mut headers = vec![
            ("X-RateLimit-Limit".into(), result.limit.to_string()),
            ("X-RateLimit-Remaining".into(), result.remaining.to_string()),
        ];
        if !result.allowed {
            headers.push(("Retry-After".into(), result.retry_after_secs.to_string()));
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
        assert_eq!(
            classify_endpoint("/api/admin/replay", "POST"),
            RateTier::Strict
        );
        assert_eq!(
            classify_endpoint("/api/auth/login", "POST"),
            RateTier::Strict
        );
        assert_eq!(classify_endpoint("/api/search", "GET"), RateTier::Search);
        assert_eq!(classify_endpoint("/api/health", "GET"), RateTier::Internal);
        assert_eq!(
            classify_endpoint("/api/warnings", "GET"),
            RateTier::Generous
        );
        assert_eq!(
            classify_endpoint("/api/companies", "POST"),
            RateTier::Standard
        );
    }

    #[test]
    fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        for _ in 0..10 {
            let result = limiter.check_ip(ip, RateTier::Strict);
            assert!(result.allowed);
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = RateLimiter::new();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..10 {
            limiter.check_ip(ip, RateTier::Strict);
        }
        let result = limiter.check_ip(ip, RateTier::Strict);
        assert!(!result.allowed);
        assert!(result.retry_after_secs > 0);
    }

    #[test]
    fn test_allowlist_bypasses_limits() {
        let limiter = RateLimiter::new();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        limiter.allow_ip(ip);
        for _ in 0..50 {
            let result = limiter.check_ip(ip, RateTier::Strict);
            assert!(result.allowed);
        }
    }

    #[test]
    fn test_blocklist_always_rejects() {
        let limiter = RateLimiter::new();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        limiter.block_ip(ip);
        let result = limiter.check_ip(ip, RateTier::Generous);
        assert!(!result.allowed);
    }

    #[test]
    fn test_different_ips_independent() {
        let limiter = RateLimiter::new();
        let ip1 = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..10 {
            limiter.check_ip(ip1, RateTier::Strict);
        }
        let result = limiter.check_ip(ip2, RateTier::Strict);
        assert!(result.allowed);
    }

    #[test]
    fn test_custom_limit_is_enforced_per_identifier_and_bucket() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            let result = limiter.check_with_limit(
                "api-key-1",
                "GET:/api/observations",
                3,
                Duration::from_secs(60),
            );
            assert!(result.allowed);
        }

        let result = limiter.check_with_limit(
            "api-key-1",
            "GET:/api/observations",
            3,
            Duration::from_secs(60),
        );
        assert!(!result.allowed);
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

    /// `RATE_LIMIT_DISABLED` alone must never disable enforcement; the
    /// explicit `APEX_ALLOW_TEST_RATE_LIMIT_DISABLED=1` override is required.
    #[test]
    fn test_rate_limit_disable_requires_explicit_test_override() {
        assert!(!should_disable_rate_limit(None, None));
        assert!(!should_disable_rate_limit(Some("true"), None));
        assert!(!should_disable_rate_limit(Some("1"), None));
        assert!(!should_disable_rate_limit(Some("true"), Some("0")));
        assert!(!should_disable_rate_limit(Some("true"), Some("yes")));
        assert!(!should_disable_rate_limit(Some("false"), Some("1")));
        assert!(should_disable_rate_limit(Some("true"), Some("1")));
        assert!(should_disable_rate_limit(Some("TRUE"), Some("1")));
        assert!(should_disable_rate_limit(Some("1"), Some("1")));
    }

    /// Startup validation refuses the production misconfiguration.
    #[test]
    fn test_validate_rate_limit_disable_refuses_request_without_override() {
        assert!(validate_rate_limit_disable(None, None).is_ok());
        assert!(validate_rate_limit_disable(Some("false"), None).is_ok());
        assert!(validate_rate_limit_disable(Some("true"), Some("1")).is_ok());

        let error = validate_rate_limit_disable(Some("true"), None)
            .expect_err("RATE_LIMIT_DISABLED without the test override must be refused");
        assert!(error.contains(ALLOW_TEST_RATE_LIMIT_DISABLED_ENV));
    }
}
