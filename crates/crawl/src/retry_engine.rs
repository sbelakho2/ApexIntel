//! Smart Retry Logic for ApexIntel OSINT Platform
//!
//! Implements:
//! - Exponential backoff with jitter
//! - Per-engine failure tracking
//! - Automatic fallback to cached content
//! - Circuit breaker pattern per domain

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::errors::CrawlError;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

const DEFAULT_BASE_DELAY_MS: u64 = 500;
const DEFAULT_MAX_DELAY_MS: u64 = 60_000; // 60 seconds
const DEFAULT_MAX_RETRIES: u32 = 5;
const JITTER_FACTOR: f64 = 0.3;
const CIRCUIT_BREAKER_FAILURE_THRESHOLD: u32 = 5;
const CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS: u64 = 300_000; // 5 minutes

// ─────────────────────────────────────────────────────────────────────────────
// Circuit Breaker State
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing, reject requests
    HalfOpen, // Testing recovery
}

impl CircuitState {
    fn as_str(&self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit Breaker
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DomainCircuitBreaker {
    pub domain: String,
    pub state: CircuitState,
    pub failure_count: u32,
    pub last_failure: Option<Instant>,
    pub last_state_change: Instant,
    pub success_count_in_half_open: u32,
}

impl DomainCircuitBreaker {
    fn new(domain: String) -> Self {
        Self {
            domain,
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure: None,
            last_state_change: Instant::now(),
            success_count_in_half_open: 0,
        }
    }

    pub fn is_available(&self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(last_failure) = self.last_failure {
                    last_failure.elapsed()
                        >= Duration::from_millis(CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS)
                } else {
                    true
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count_in_half_open += 1;
                if self.success_count_in_half_open >= 2 {
                    self.transition_to(CircuitState::Closed);
                    self.failure_count = 0;
                }
            }
            CircuitState::Open => {}
        }
    }

    pub fn record_failure(&mut self) {
        self.last_failure = Some(Instant::now());
        self.failure_count += 1;

        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= CIRCUIT_BREAKER_FAILURE_THRESHOLD {
                    self.transition_to(CircuitState::Open);
                }
            }
            CircuitState::HalfOpen => {
                self.transition_to(CircuitState::Open);
            }
            CircuitState::Open => {}
        }
    }

    /// Promote an `Open` breaker to `HalfOpen` once the recovery window has
    /// elapsed, so a recovered domain can be probed and closed again.
    fn maybe_half_open(&mut self) {
        if self.state == CircuitState::Open
            && self
                .last_failure
                .map(|lf| {
                    lf.elapsed() >= Duration::from_millis(CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS)
                })
                .unwrap_or(true)
        {
            self.transition_to(CircuitState::HalfOpen);
        }
    }

    fn transition_to(&mut self, new_state: CircuitState) {
        if self.state != new_state {
            info!(
                domain = %self.domain,
                from = %self.state.as_str(),
                to = %new_state.as_str(),
                "circuit_breaker: state transition"
            );
            self.state = new_state;
            self.last_state_change = Instant::now();
            self.success_count_in_half_open = 0;
        }
    }

    pub fn time_until_retry(&self) -> Option<Duration> {
        if self.state == CircuitState::Open {
            self.last_failure.map(|lf| {
                let elapsed = lf.elapsed();
                let timeout = Duration::from_millis(CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS);
                if elapsed >= timeout {
                    Duration::ZERO
                } else {
                    timeout - elapsed
                }
            })
        } else {
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry Strategy Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetryConfig {
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub max_retries: u32,
    pub jitter_factor: f64,
    pub exponential_base: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(DEFAULT_BASE_DELAY_MS),
            max_delay: Duration::from_millis(DEFAULT_MAX_DELAY_MS),
            max_retries: DEFAULT_MAX_RETRIES,
            jitter_factor: JITTER_FACTOR,
            exponential_base: 2.0,
        }
    }
}

impl RetryConfig {
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.base_delay.is_zero() {
            errors.push("base_delay must be non-zero".into());
        }
        if self.max_delay < self.base_delay {
            errors.push("max_delay must be >= base_delay".into());
        }
        if self.jitter_factor < 0.0 || self.jitter_factor > 1.0 {
            errors.push("jitter_factor must be in [0, 1]".into());
        }
        errors
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry Decision
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RetryDecision {
    pub should_retry: bool,
    pub delay: Duration,
    pub attempt: u32,
    pub reason: Option<String>,
    pub use_cached_content: bool,
}

impl RetryDecision {
    fn retry_now(attempt: u32, delay: Duration) -> Self {
        Self {
            should_retry: true,
            delay,
            attempt,
            reason: None,
            use_cached_content: false,
        }
    }

    fn retry_with_cache(attempt: u32, delay: Duration) -> Self {
        Self {
            should_retry: true,
            delay,
            attempt,
            reason: Some("using cached content".into()),
            use_cached_content: true,
        }
    }

    fn stop(attempt: u32, reason: String) -> Self {
        Self {
            should_retry: false,
            delay: Duration::ZERO,
            attempt,
            reason: Some(reason),
            use_cached_content: false,
        }
    }

    fn circuit_open(domain: &str, retry_after: Duration) -> Self {
        Self {
            should_retry: false,
            delay: Duration::ZERO,
            attempt: 0,
            reason: Some(format!(
                "circuit breaker open for {}, retry after {:.1}s",
                domain,
                retry_after.as_secs_f64()
            )),
            use_cached_content: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cache Manager for Fallback
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedContent {
    pub url: String,
    pub content: String,
    pub content_type: Option<String>,
    pub cached_at: Duration,
    pub max_age: Duration,
    pub status: u16,
}

impl CachedContent {
    pub fn is_fresh(&self) -> bool {
        self.cached_at < self.max_age
    }

    pub fn age(&self) -> Duration {
        self.cached_at
    }
}

#[derive(Debug, Clone, Default)]
pub struct ContentCache {
    entries: HashMap<String, CachedContent>,
    max_entries: usize,
    default_max_age: Duration,
}

impl ContentCache {
    pub fn new(max_entries: usize, default_max_age: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            default_max_age,
        }
    }

    pub fn get(&self, url: &str) -> Option<&CachedContent> {
        self.entries.get(url)
    }

    pub fn put(&mut self, url: &str, content: CachedContent) {
        // Simple eviction: remove oldest if at capacity
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(url) {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.cached_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(url.to_string(), content);
    }

    pub fn invalidate(&mut self, url: &str) {
        self.entries.remove(url);
    }

    pub fn invalidate_domain(&mut self, domain: &str) {
        self.entries.retain(|url, _| !url.contains(domain));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry Engine Error
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum RetryEngineError {
    #[error("circuit breaker open for domain {domain}, retry after {retry_after_secs}s")]
    CircuitBreakerOpen {
        domain: String,
        retry_after_secs: u64,
    },

    #[error("using cached content for {url}: {message}")]
    CircuitBreakerFallback {
        url: String,
        message: String,
        cached_content: String,
        cached_status: u16,
    },

    #[error("max retries ({max_retries}) exceeded for {url}: {last_error}")]
    MaxRetriesExceeded {
        url: String,
        max_retries: u32,
        last_error: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry Engine
// ─────────────────────────────────────────────────────────────────────────────

pub struct RetryEngine {
    config: RetryConfig,
    circuit_breakers: Arc<RwLock<HashMap<String, DomainCircuitBreaker>>>,
    cache: Arc<RwLock<ContentCache>>,
}

impl RetryEngine {
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            cache: Arc::new(RwLock::new(ContentCache::new(
                10_000,
                Duration::from_secs(3600),
            ))),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(RetryConfig::default())
    }

    /// Calculate retry delay with exponential backoff and jitter
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let exponential_delay = self.config.base_delay.as_millis() as f64
            * self.config.exponential_base.powf(attempt as f64);

        // Apply jitter using deterministic seed based on attempt number
        let jitter_seed = ((attempt as f64 * 17.31).sin() * 1000.0) % 1.0;
        let jitter = exponential_delay * self.config.jitter_factor * jitter_seed.abs();

        let total_delay_ms =
            (exponential_delay + jitter).min(self.config.max_delay.as_millis() as f64);
        Duration::from_millis(total_delay_ms as u64)
    }

    /// Determine if a request should be retried based on the error and attempt
    pub fn should_retry(&self, error: &CrawlError, attempt: u32, domain: &str) -> RetryDecision {
        // Check circuit breaker - use try_read to avoid blocking in tests
        if let Ok(breakers) = self.circuit_breakers.try_read() {
            if let Some(cb) = breakers.get(domain) {
                if !cb.is_available() {
                    let retry_after = cb.time_until_retry().unwrap_or(Duration::from_secs(300));
                    return RetryDecision::circuit_open(domain, retry_after);
                }
            }
        }

        // Check if error is retryable
        if !error.is_retryable() {
            return RetryDecision::stop(
                attempt,
                format!("non-retryable error: {:?}", error.category()),
            );
        }

        // Check max retries
        if attempt >= self.config.max_retries {
            // If we have cached content, use it. `try_read` (not `blocking_read`)
            // because this runs inside an async context.
            if let Ok(cache) = self.cache.try_read() {
                if let Some(cached) = cache.get(domain) {
                    if cached.is_fresh() {
                        return RetryDecision::retry_with_cache(attempt, Duration::ZERO);
                    }
                }
            }
            return RetryDecision::stop(
                attempt,
                format!("max retries ({}) exceeded", self.config.max_retries),
            );
        }

        // Calculate delay
        let delay = error
            .retry_after()
            .unwrap_or_else(|| self.calculate_delay(attempt));

        RetryDecision::retry_now(attempt + 1, delay)
    }

    /// Record a successful request
    pub async fn record_success(&self, domain: &str) {
        let mut breakers = self.circuit_breakers.write().await;
        let cb = breakers
            .entry(domain.to_string())
            .or_insert_with(|| DomainCircuitBreaker::new(domain.to_string()));
        cb.record_success();
    }

    /// Record a failed request
    pub async fn record_failure(&self, domain: &str) {
        let mut breakers = self.circuit_breakers.write().await;
        let cb = breakers
            .entry(domain.to_string())
            .or_insert_with(|| DomainCircuitBreaker::new(domain.to_string()));
        cb.record_failure();
    }

    /// Get cached content for a domain/URL
    pub async fn get_cached(&self, url: &str) -> Option<CachedContent> {
        self.cache.read().await.get(url).cloned()
    }

    /// Cache content from a successful fetch
    pub async fn cache_content(
        &self,
        url: String,
        content: String,
        content_type: Option<String>,
        status: u16,
        max_age: Option<Duration>,
    ) {
        let default_max_age = {
            let cache = self.cache.read().await;
            cache.default_max_age
        };

        let cached = CachedContent {
            url: url.clone(),
            content,
            content_type,
            cached_at: Duration::ZERO,
            max_age: max_age.unwrap_or(default_max_age),
            status,
        };

        self.cache.write().await.put(&url, cached);
    }

    /// Invalidate cache for a domain
    pub async fn invalidate_domain(&self, domain: &str) {
        self.cache.write().await.invalidate_domain(domain);
    }

    /// Get circuit breaker state for a domain
    pub async fn circuit_state(&self, domain: &str) -> Option<CircuitState> {
        self.circuit_breakers
            .read()
            .await
            .get(domain)
            .map(|cb| cb.state)
    }

    /// Check if a domain is available (circuit not open)
    pub async fn is_domain_available(&self, domain: &str) -> bool {
        let mut breakers = self.circuit_breakers.write().await;
        if let Some(cb) = breakers.get_mut(domain) {
            cb.maybe_half_open();
            cb.is_available()
        } else {
            true
        }
    }

    /// Reset circuit breaker for a domain (for manual intervention)
    pub async fn reset_circuit(&self, domain: &str) {
        let mut breakers = self.circuit_breakers.write().await;
        if let Some(cb) = breakers.get_mut(domain) {
            cb.transition_to(CircuitState::Closed);
            cb.failure_count = 0;
        }
    }

    /// Get health statistics for all circuits
    pub async fn circuit_stats(&self) -> HashMap<String, CircuitStats> {
        let breakers = self.circuit_breakers.read().await;
        breakers
            .iter()
            .map(|(domain, cb)| {
                (
                    domain.clone(),
                    CircuitStats {
                        domain: domain.clone(),
                        state: cb.state,
                        failure_count: cb.failure_count,
                        time_until_retry: cb.time_until_retry(),
                    },
                )
            })
            .collect()
    }

    /// Perform retry with automatic circuit breaker management
    /// Returns the result directly or a wrapped retry error
    pub async fn execute_with_retry<F, Fut>(
        &self,
        domain: &str,
        operation: F,
    ) -> Result<String, RetryEngineError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<String, CrawlError>>,
    {
        let mut attempt = 0u32;

        loop {
            // Check circuit breaker
            if !self.is_domain_available(domain).await {
                let retry_after = self
                    .circuit_breakers
                    .read()
                    .await
                    .get(domain)
                    .and_then(|cb| cb.time_until_retry())
                    .unwrap_or(Duration::from_secs(300));

                warn!(
                    domain = %domain,
                    retry_after_s = %retry_after.as_secs(),
                    "retry_engine: circuit breaker open, failing fast"
                );

                // Fail fast instead of blocking the caller for the whole
                // recovery window (up to 5 minutes). The caller/scheduler can
                // retry after the breaker half-opens.
                if let Some(cached) = self.get_cached(domain).await {
                    if cached.is_fresh() {
                        return Err(RetryEngineError::CircuitBreakerFallback {
                            url: cached.url.clone(),
                            message: format!(
                                "circuit open, using cached content from {:.1}s ago",
                                cached.age().as_secs_f64()
                            ),
                            cached_content: cached.content,
                            cached_status: cached.status,
                        });
                    }
                }
                return Err(RetryEngineError::CircuitBreakerOpen {
                    domain: domain.to_string(),
                    retry_after_secs: retry_after.as_secs(),
                });
            }

            match operation().await {
                Ok(result) => {
                    self.record_success(domain).await;
                    return Ok(result);
                }
                Err(error) => {
                    self.record_failure(domain).await;
                    let decision = self.should_retry(&error, attempt, domain);

                    if !decision.should_retry {
                        if decision.use_cached_content {
                            if let Some(cached) = self.get_cached(domain).await {
                                if cached.is_fresh() {
                                    return Err(RetryEngineError::CircuitBreakerFallback {
                                        url: cached.url.clone(),
                                        message: "retries exhausted, using cached content".into(),
                                        cached_content: cached.content,
                                        cached_status: cached.status,
                                    });
                                }
                            }
                        }
                        return Err(RetryEngineError::MaxRetriesExceeded {
                            url: domain.to_string(),
                            max_retries: self.config.max_retries,
                            last_error: error.to_string(),
                        });
                    }

                    debug!(
                        domain = %domain,
                        attempt = decision.attempt,
                        delay_ms = decision.delay.as_millis(),
                        "retry_engine: scheduling retry"
                    );

                    if !decision.delay.is_zero() {
                        sleep(decision.delay).await;
                    }
                    attempt = decision.attempt;
                }
            }
        }
    }
}

impl Default for RetryEngine {
    fn default() -> Self {
        Self::with_default_config()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit Stats (for monitoring)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CircuitStats {
    pub domain: String,
    pub state: CircuitState,
    pub failure_count: u32,
    pub time_until_retry: Option<Duration>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = DomainCircuitBreaker::new("example.com".into());
        assert_eq!(cb.state, CircuitState::Closed);
        assert!(cb.is_available());
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let mut cb = DomainCircuitBreaker::new("example.com".into());
        for _ in 0..CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            cb.record_failure();
        }
        assert_eq!(cb.state, CircuitState::Open);
        assert!(!cb.is_available());
    }

    #[test]
    fn test_circuit_breaker_half_open_after_timeout() {
        let mut cb = DomainCircuitBreaker::new("example.com".into());
        for _ in 0..CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            cb.record_failure();
        }
        assert_eq!(cb.state, CircuitState::Open);

        // Simulate time passage
        cb.last_failure = Some(Instant::now() - Duration::from_secs(400));
        assert!(cb.is_available());
    }

    #[test]
    fn test_circuit_breaker_recovery() {
        let mut cb = DomainCircuitBreaker::new("example.com".into());
        cb.transition_to(CircuitState::HalfOpen);

        cb.record_success();
        cb.record_success();

        assert_eq!(cb.state, CircuitState::Closed);
        assert_eq!(cb.failure_count, 0);
    }

    #[test]
    fn test_retry_config_validation() {
        let valid = RetryConfig::default();
        assert!(valid.validate().is_empty());

        let invalid = RetryConfig {
            base_delay: Duration::ZERO,
            ..RetryConfig::default()
        };
        assert!(!invalid.validate().is_empty());
    }

    #[test]
    fn test_retry_decision_exponential_delay() {
        let engine = RetryEngine::with_default_config();
        let d0 = engine.calculate_delay(0);
        let d1 = engine.calculate_delay(1);
        let d2 = engine.calculate_delay(2);

        // Each delay should be larger than the previous
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn test_cache_operations() {
        let mut cache = ContentCache::new(10, Duration::from_secs(60));

        cache.put(
            "https://example.com/page1",
            CachedContent {
                url: "https://example.com/page1".into(),
                content: "test content".into(),
                content_type: Some("text/html".into()),
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(60),
                status: 200,
            },
        );

        let cached = cache.get("https://example.com/page1");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().content, "test content");
    }

    #[test]
    fn test_cache_invalidation() {
        let mut cache = ContentCache::new(10, Duration::from_secs(60));

        cache.put(
            "https://example.com/page1",
            CachedContent {
                url: "https://example.com/page1".into(),
                content: "test".into(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(60),
                status: 200,
            },
        );

        cache.invalidate("https://example.com/page1");
        assert!(cache.get("https://example.com/page1").is_none());
    }

    #[test]
    fn test_domain_invalidation() {
        let mut cache = ContentCache::new(10, Duration::from_secs(60));

        cache.put(
            "https://example.com/page1",
            CachedContent {
                url: "https://example.com/page1".into(),
                content: "test".into(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(60),
                status: 200,
            },
        );

        cache.put(
            "https://other.com/page1",
            CachedContent {
                url: "https://other.com/page1".into(),
                content: "test".into(),
                content_type: None,
                cached_at: Duration::ZERO,
                max_age: Duration::from_secs(60),
                status: 200,
            },
        );

        cache.invalidate_domain("example.com");
        assert!(cache.get("https://example.com/page1").is_none());
        assert!(cache.get("https://other.com/page1").is_some());
    }

    #[tokio::test]
    async fn test_circuit_state_tracking() {
        let engine = RetryEngine::with_default_config();

        // Record failures to trigger circuit breaker
        for _ in 0..6 {
            engine.record_failure("test-domain.com").await;
        }

        let state = engine.circuit_state("test-domain.com").await;
        assert_eq!(state, Some(CircuitState::Open));

        // Reset and check
        engine.reset_circuit("test-domain.com").await;
        let state = engine.circuit_state("test-domain.com").await;
        assert_eq!(state, Some(CircuitState::Closed));
    }

    #[tokio::test]
    async fn test_circuit_stats() {
        let engine = RetryEngine::with_default_config();

        engine.record_failure("domain1.com").await;
        engine.record_failure("domain1.com").await;

        let stats = engine.circuit_stats().await;
        assert!(stats.contains_key("domain1.com"));
    }

    #[test]
    fn test_cached_content_freshness() {
        let mut content = CachedContent {
            url: "https://example.com".into(),
            content: "test".into(),
            content_type: None,
            cached_at: Duration::ZERO,
            max_age: Duration::from_secs(60),
            status: 200,
        };

        assert!(content.is_fresh());

        // Age the content
        content.cached_at = Duration::from_secs(120);
        assert!(!content.is_fresh());
    }

    #[tokio::test]
    async fn test_execute_with_retry_success() {
        let engine = RetryEngine::with_default_config();

        let result = engine
            .execute_with_retry("example.com", || async { Ok("success".to_string()) })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_execute_with_retry_eventual_failure() {
        let engine = RetryEngine::with_default_config();

        // Create an operation that always fails with a non-retryable error
        let result = engine
            .execute_with_retry("example.com", || async {
                Err(CrawlError::RobotsDenied {
                    url: "https://example.com".to_string(),
                    user_agent: "test".to_string(),
                })
            })
            .await;

        // Should fail immediately since RobotsDenied is not retryable
        assert!(result.is_err());
    }

    #[test]
    fn test_circuit_state_serialization() {
        let states = vec![
            (CircuitState::Closed, "closed"),
            (CircuitState::Open, "open"),
            (CircuitState::HalfOpen, "half_open"),
        ];

        for (state, expected) in states {
            assert_eq!(state.as_str(), expected);
        }
    }

    #[tokio::test]
    async fn execute_with_retry_fails_fast_when_circuit_open() {
        // Regression (B364): an open breaker used to sleep for the full
        // recovery window (up to 5 minutes) inside the caller. It must now
        // return an error essentially immediately.
        let engine = RetryEngine::with_default_config();
        for _ in 0..CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            engine.record_failure("troubled.example.com").await;
        }
        assert!(!engine.is_domain_available("troubled.example.com").await);

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            engine.execute_with_retry("troubled.example.com", || async {
                // Must not even be called: the breaker is open.
                Err(CrawlError::Transport {
                    url: "https://troubled.example.com".to_string(),
                    message: "still failing".to_string(),
                    category: crate::errors::CrawlFailureCategory::Network,
                })
            }),
        )
        .await
        .expect("execute_with_retry must not block on an open circuit breaker");

        assert!(matches!(
            result,
            Err(RetryEngineError::CircuitBreakerOpen { .. })
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn execute_with_retry_stops_on_non_retryable_error_without_looping() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let engine = RetryEngine::with_default_config();
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        let result = engine
            .execute_with_retry("example.com", move || {
                let calls = calls_clone.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(CrawlError::InvalidUrl {
                        url: "not a url".to_string(),
                        message: "malformed".to_string(),
                    })
                }
            })
            .await;

        assert!(result.is_err());
        // A non-retryable error must be attempted exactly once.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn calculate_delay_is_bounded_and_monotonic_in_expectation() {
        let engine = RetryEngine::with_default_config();
        // The jittered delay must never exceed max_delay.
        for attempt in 0..12u32 {
            let delay = engine.calculate_delay(attempt);
            assert!(
                delay <= engine.config.max_delay,
                "attempt {attempt} delay {delay:?} exceeds max"
            );
        }
        // Larger attempts must not produce systematically tiny delays compared
        // to attempt 0; the exponential base dominates the +/-30% jitter.
        let max_attempt = engine.calculate_delay(10);
        assert!(max_attempt >= engine.config.base_delay);
    }

    #[tokio::test]
    async fn circuit_breaker_stats_track_state_and_count() {
        let engine = RetryEngine::with_default_config();
        engine.record_failure("stats.example.com").await;
        let stats = engine.circuit_stats().await;
        let entry = stats.get("stats.example.com").expect("stats entry present");
        assert_eq!(entry.failure_count, 1);
        assert_eq!(entry.state, CircuitState::Closed);

        // Recording a success resets the failure count.
        engine.record_success("stats.example.com").await;
        let stats = engine.circuit_stats().await;
        let entry = stats.get("stats.example.com").unwrap();
        assert_eq!(entry.failure_count, 0);
    }
}
