//! Robust retry logic with exponential backoff, jitter, and circuit breaker.
//!
//! Addresses the 21% crawl success rate by automatically retrying transient
//! failures (5xx, timeouts, connection errors) while avoiding hammering
//! permanently-failing endpoints.
//!
//! # Architecture
//!
//! 1. **RetryConfig** — configurable parameters for retry behavior
//! 2. **retry_with_backoff** — async function wrapping reqwest HTTP requests
//! 3. **RetryPolicy** — classifies errors as transient (retryable) or permanent
//! 4. **CircuitBreaker** — stops requests to failing domains after N consecutive failures

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use rand::Rng;
use reqwest::{Client, StatusCode};
use tracing::{debug, info, warn};

// ─── Configuration ──────────────────────────────────────────────────────────

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 4).
    pub max_retries: usize,
    /// Base delay between retries in milliseconds (default: 500).
    pub base_delay_ms: u64,
    /// Maximum delay between retries in milliseconds (default: 60_000).
    pub max_delay_ms: u64,
    /// Jitter factor: 1.0 = full jitter, 0.0 = no jitter.
    pub jitter_factor: f64,
    /// Number of consecutive failures before circuit opens (default: 5).
    pub circuit_failure_threshold: usize,
    /// Cooldown period for open circuits in seconds (default: 30).
    pub circuit_cooldown_secs: u64,
    /// HTTP status codes that should be retried (default: 429, 5xx).
    pub retryable_status_codes: Vec<StatusCode>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay_ms: 500,
            max_delay_ms: 60_000,
            jitter_factor: 1.0,
            circuit_failure_threshold: 5,
            circuit_cooldown_secs: 30,
            retryable_status_codes: vec![
                StatusCode::TOO_MANY_REQUESTS,
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::BAD_GATEWAY,
                StatusCode::SERVICE_UNAVAILABLE,
                StatusCode::GATEWAY_TIMEOUT,
            ],
        }
    }
}

impl RetryConfig {
    /// Create a faster retry config for high-priority crawls.
    pub fn aggressive() -> Self {
        Self {
            max_retries: 6,
            base_delay_ms: 200,
            max_delay_ms: 30_000,
            ..Default::default()
        }
    }

    /// Create a conservative retry config for low-priority crawls.
    pub fn conservative() -> Self {
        Self {
            max_retries: 2,
            base_delay_ms: 1000,
            max_delay_ms: 120_000,
            ..Default::default()
        }
    }
}

// ─── Error Types ────────────────────────────────────────────────────────────

/// Categories for request errors to determine retryability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Retryable: the request may succeed on a subsequent attempt.
    Transient,
    /// Permanent: the request will not succeed regardless of retries.
    Permanent,
}

/// Result of a retry attempt.
#[derive(Debug)]
pub struct RetryResult {
    /// The response data (body text).
    pub data: String,
    /// The final HTTP status code.
    pub status_code: u16,
    /// Number of attempts made (1 + retries).
    pub attempts: usize,
    /// Total duration of all attempts.
    pub total_duration: Duration,
}

/// Error from retry exhaustion or permanent failure.
#[derive(Debug, thiserror::Error)]
pub enum RetryError {
    #[error("all {attempts} retry attempts exhausted, last error: {last_error}")]
    Exhausted {
        attempts: usize,
        last_error: String,
    },
    #[error("permanent error: {0}")]
    Permanent(String),
    #[error("circuit breaker open for domain {domain}")]
    CircuitOpen { domain: String },
    #[error("request error: {0}")]
    Request(String),
}

// ─── Error Classification ───────────────────────────────────────────────────

/// Classify a reqwest error as transient or permanent.
fn classify_reqwest_error(error: &reqwest::Error) -> ErrorCategory {
    if error.is_timeout() {
        return ErrorCategory::Transient;
    }
    if error.is_connect() {
        return ErrorCategory::Transient;
    }
    if error.is_request() {
        // Request was sent but no response — could be transient network issue
        return ErrorCategory::Transient;
    }
    // DNS errors, TLS errors, etc. — permanent
    ErrorCategory::Permanent
}

/// Classify an HTTP status code for retryability.
fn classify_http_status(status: StatusCode, retryable: &[StatusCode]) -> ErrorCategory {
    if retryable.contains(&status) {
        ErrorCategory::Transient
    } else if status.is_server_error() {
        ErrorCategory::Transient
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCategory::Transient
    } else {
        ErrorCategory::Permanent
    }
}

// ─── Backoff Computation ────────────────────────────────────────────────────

/// Compute the delay for a given retry attempt using exponential backoff
/// with full jitter.
///
/// Full jitter: `random(0, min(max_delay, base * 2^attempt))`
pub fn compute_backoff(config: &RetryConfig, attempt: usize) -> Duration {
    let exponential = (config.base_delay_ms as f64 * 2_f64.powi(attempt as i32)) as u64;
    let capped = exponential.min(config.max_delay_ms);

    if config.jitter_factor <= 0.0 {
        return Duration::from_millis(capped);
    }

    let mut rng = rand::thread_rng();
    let jittered = if config.jitter_factor >= 1.0 {
        // Full jitter: random in [0, capped]
        rng.gen_range(0..=capped)
    } else {
        // Partial jitter: capped/2 + random(0, capped/2)
        let half = capped / 2;
        half + rng.gen_range(0..=half)
    };

    Duration::from_millis(jittered)
}

// ─── Circuit Breaker ────────────────────────────────────────────────────────

/// Circuit breaker state for a domain.
#[derive(Debug, Clone)]
struct DomainState {
    consecutive_failures: usize,
    open_since: Option<Instant>,
}

/// Thread-safe circuit breaker that tracks per-domain failure counts
/// and temporarily blocks requests to failing domains.
#[derive(Debug)]
pub struct CircuitBreaker {
    states: Mutex<HashMap<String, DomainState>>,
    failure_threshold: usize,
    cooldown: Duration,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: usize, cooldown_secs: u64) -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            failure_threshold,
            cooldown: Duration::from_secs(cooldown_secs),
        }
    }

    /// Check if requests to a domain are allowed.
    /// Returns true if the circuit is closed (requests allowed).
    pub fn allow_request(&self, domain: &str) -> bool {
        let states = self.states.lock().unwrap_or_else(|e| {
            warn!("circuit breaker mutex poisoned: {e}");
            panic!("circuit breaker mutex poisoned: {e}")
        });
        if let Some(state) = states.get(domain) {
            if let Some(open_since) = state.open_since {
                if open_since.elapsed() < self.cooldown {
                    debug!(
                        domain = %domain,
                        "circuit breaker: request blocked for domain"
                    );
                    return false;
                }
                // Cooldown expired — allow a trial request
            }
        }
        true
    }

    /// Record a successful request to a domain (resets the failure count).
    pub fn record_success(&self, domain: &str) {
        let mut states = self.states.lock().unwrap_or_else(|e| {
            warn!("circuit breaker mutex poisoned: {e}");
            panic!("circuit breaker mutex poisoned: {e}")
        });
        states.remove(domain);
        debug!(domain = %domain, "circuit breaker: success, circuit closed");
    }

    /// Record a failed request to a domain.
    pub fn record_failure(&self, domain: &str) {
        let mut states = self.states.lock().unwrap_or_else(|e| {
            warn!("circuit breaker mutex poisoned: {e}");
            panic!("circuit breaker mutex poisoned: {e}")
        });
        let entry = states.entry(domain.to_string()).or_insert(DomainState {
            consecutive_failures: 0,
            open_since: None,
        });
        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= self.failure_threshold {
            entry.open_since = Some(Instant::now());
            info!(
                domain = %domain,
                failures = entry.consecutive_failures,
                "circuit breaker: circuit opened for domain"
            );
        }
    }

    /// Get the current state for all tracked domains.
    pub fn domain_states(&self) -> HashMap<String, (usize, Option<Duration>)> {
        let states = self.states.lock().unwrap_or_else(|e| {
            warn!("circuit breaker mutex poisoned: {e}");
            panic!("circuit breaker mutex poisoned: {e}")
        });
        states
            .iter()
            .map(|(domain, state)| {
                let time_remaining = state.open_since.map(|since| {
                    let open_duration = since.elapsed();
                    if open_duration >= self.cooldown {
                        Duration::from_secs(0)
                    } else {
                        self.cooldown - open_duration
                    }
                });
                (domain.clone(), (state.consecutive_failures, time_remaining))
            })
            .collect()
    }
}

// ─── Main Retry Function ────────────────────────────────────────────────────

/// Execute an HTTP request with retry and exponential backoff.
///
/// # Arguments
/// * `client` - reqwest Client instance
/// * `url` - The URL to fetch
/// * `config` - Retry configuration
/// * `circuit_breaker` - Optional circuit breaker for domain-level throttling
///
/// # Returns
/// `Ok(RetryResult)` on success, `Err(RetryError)` if all retries exhausted or permanent error.
pub async fn retry_with_backoff(
    client: &Client,
    url: &str,
    config: &RetryConfig,
    circuit_breaker: Option<&CircuitBreaker>,
) -> Result<RetryResult, RetryError> {
    let domain = extract_domain(url);
    let start = Instant::now();

    // Check circuit breaker
    if let Some(cb) = circuit_breaker {
        if !cb.allow_request(&domain) {
            return Err(RetryError::CircuitOpen { domain });
        }
    }

    let mut last_error: Option<String> = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            let delay = compute_backoff(config, attempt.saturating_sub(1));
            debug!(
                url = %url,
                attempt,
                delay_ms = delay.as_millis(),
                "retry: backing off before attempt"
            );
            tokio::time::sleep(delay).await;
        }

        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();

                if status.is_success() {
                    let body = response.text().await.map_err(|e| {
                        RetryError::Request(format!("failed to read response body: {e}"))
                    })?;

                    if let Some(cb) = circuit_breaker {
                        cb.record_success(&domain);
                    }

                    return Ok(RetryResult {
                        data: body,
                        status_code: status.as_u16(),
                        attempts: attempt + 1,
                        total_duration: start.elapsed(),
                    });
                }

                // Non-success status — classify and possibly retry
                let category = classify_http_status(status, &config.retryable_status_codes);
                let body_text = response.text().await.unwrap_or_default();
                last_error = Some(format!("HTTP {}: {}", status.as_u16(), truncate(&body_text, 200)));

                match category {
                    ErrorCategory::Transient => {
                        warn!(
                            url = %url,
                            status = status.as_u16(),
                            attempt = attempt + 1,
                            "retry: transient HTTP error, will retry"
                        );
                        continue;
                    }
                    ErrorCategory::Permanent => {
                        if let Some(cb) = circuit_breaker {
                            cb.record_failure(&domain);
                        }
                        return Err(RetryError::Permanent(last_error.take().unwrap_or_default()));
                    }
                }
            }
            Err(e) => {
                let category = classify_reqwest_error(&e);
                last_error = Some(format!("reqwest error: {e}"));

                match category {
                    ErrorCategory::Transient => {
                        warn!(
                            url = %url,
                            attempt = attempt + 1,
                            error = %e,
                            "retry: transient reqwest error, will retry"
                        );
                        continue;
                    }
                    ErrorCategory::Permanent => {
                        if let Some(cb) = circuit_breaker {
                            cb.record_failure(&domain);
                        }
                        return Err(RetryError::Permanent(format!("{e}")));
                    }
                }
            }
        }
    }

    // All retries exhausted
    if let Some(cb) = circuit_breaker {
        cb.record_failure(&domain);
    }

    Err(RetryError::Exhausted {
        attempts: config.max_retries + 1,
        last_error: last_error.unwrap_or_else(|| "unknown error".to_string()),
    })
}

/// Simple fetch with default retry config (no circuit breaker).
pub async fn fetch_with_retry(client: &Client, url: &str) -> Result<RetryResult, RetryError> {
    retry_with_backoff(client, url, &RetryConfig::default(), None).await
}

// ─── Domain Extraction ──────────────────────────────────────────────────────

fn extract_domain(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("unknown")
        .split(':')
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_increases_exponentially() {
        let config = RetryConfig {
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            jitter_factor: 0.0, // No jitter for deterministic test
            ..Default::default()
        };

        let d0 = compute_backoff(&config, 0);
        let d1 = compute_backoff(&config, 1);
        let d2 = compute_backoff(&config, 2);

        assert_eq!(d0, Duration::from_millis(100));
        assert_eq!(d1, Duration::from_millis(200));
        assert_eq!(d2, Duration::from_millis(400));
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let config = RetryConfig {
            base_delay_ms: 100,
            max_delay_ms: 500,
            jitter_factor: 0.0,
            ..Default::default()
        };

        let d10 = compute_backoff(&config, 10); // Would be 100 * 2^10 = 102_400
        assert_eq!(d10, Duration::from_millis(500));
    }

    #[test]
    fn test_backoff_with_jitter_stays_in_range() {
        let config = RetryConfig {
            base_delay_ms: 100,
            max_delay_ms: 500,
            jitter_factor: 1.0,
            ..Default::default()
        };

        for _ in 0..100 {
            let d = compute_backoff(&config, 2); // Base: 400, capped at 500
            assert!(d.as_millis() <= 500);
        }
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://example.com/path"), "example.com");
        assert_eq!(extract_domain("http://sub.example.co.uk:8080/path"), "sub.example.co.uk");
        assert_eq!(extract_domain("https://foxconn.com"), "foxconn.com");
    }

    #[test]
    fn test_classify_http_status() {
        let retryable = RetryConfig::default().retryable_status_codes;

        assert_eq!(
            classify_http_status(StatusCode::SERVICE_UNAVAILABLE, &retryable),
            ErrorCategory::Transient
        );
        assert_eq!(
            classify_http_status(StatusCode::TOO_MANY_REQUESTS, &retryable),
            ErrorCategory::Transient
        );
        assert_eq!(
            classify_http_status(StatusCode::NOT_FOUND, &retryable),
            ErrorCategory::Permanent
        );
        assert_eq!(
            classify_http_status(StatusCode::UNAUTHORIZED, &retryable),
            ErrorCategory::Permanent
        );
    }

    #[test]
    fn test_circuit_breaker_opens_and_closes() {
        let cb = CircuitBreaker::new(3, 5);
        let domain = "broken.example.com";

        // Allow initial requests
        assert!(cb.allow_request(domain));

        // Record failures
        cb.record_failure(domain);
        cb.record_failure(domain);
        assert!(cb.allow_request(domain)); // Not at threshold yet
        cb.record_failure(domain);

        // Circuit should be open now
        assert!(!cb.allow_request(domain));

        // Record success should close it
        cb.record_success(domain);
        assert!(cb.allow_request(domain));
    }
}