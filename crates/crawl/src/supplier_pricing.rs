//! Supplier pricing crawl through the r.jina.ai reader.
//!
//! The only supported pricing sources are **Alibaba**, **1688**, **LCSC**
//! (marketplaces) plus **Baidu** (discovery fallback). The retired
//! Google/Nexar/Mouser/DigiKey/Octopart integrations were removed: their
//! credentials were parsed but had no runtime consumer.
//!
//! Every fetch is routed through `https://r.jina.ai/{target-url}` so the
//! target sites see the reader, not this process. The pipeline is deliberately
//! "smart" about three things the operators asked for:
//!
//! 1. **When / how / what to retry** — a `RetryDecision` classifier only
//!    retries idempotent GET fetches on transient failures (429 honouring
//!    `Retry-After`, 425, 5xx, connect/timeout/body errors) with jittered
//!    exponential backoff; 400/401/402/403 are permanent and fail fast.
//!    A per-source circuit breaker stops hammering a source that keeps
//!    failing, and per-source pacing plus a global concurrency cap keeps the
//!    reader below its rate limits.
//! 2. **Teaser avoidance** — a price is quotable only when a quantity tier
//!    covers the requested quantity. "From US $X" prices and MOQ-less
//!    singles are flagged `teaser` and can never be the reported price.
//! 3. **Lowest price** — search results are crawled to listing pages, offers
//!    parsed per listing, and the lowest unit price that actually covers the
//!    target quantity wins (tie-broken by larger quantity ceiling). When a
//!    marketplace search yields no listings, discovery falls back to Baidu
//!    `site:` queries for that marketplace's listing pages.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::http::external_client;

// ─────────────────────────────────────────────────────────────────────────────
// Sources
// ─────────────────────────────────────────────────────────────────────────────

/// The marketplace/search sources the pricing pipeline may crawl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PricingSource {
    Alibaba,
    OneSixEightEight,
    Lcsc,
    Baidu,
}

impl PricingSource {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "alibaba" => Some(Self::Alibaba),
            "1688" => Some(Self::OneSixEightEight),
            "lcsc" => Some(Self::Lcsc),
            "baidu" => Some(Self::Baidu),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alibaba => "alibaba",
            Self::OneSixEightEight => "1688",
            Self::Lcsc => "lcsc",
            Self::Baidu => "baidu",
        }
    }

    /// The marketplace whose listing pages this source's search discovers.
    /// Baidu is only a discovery engine, never a marketplace itself.
    pub fn search_url(&self, query: &str) -> String {
        let q = urlencoding(query);
        match self {
            Self::Alibaba => format!("https://www.alibaba.com/trade/search?SearchText={q}"),
            Self::OneSixEightEight => {
                format!("https://s.1688.com/selloffer/offer_search.htm?keywords={q}")
            }
            Self::Lcsc => format!("https://www.lcsc.com/search?q={q}"),
            Self::Baidu => format!("https://www.baidu.com/s?wd={q}"),
        }
    }

    /// Baidu discovery query for another marketplace's listing pages.
    pub fn baidu_discovery_url(&self, query: &str) -> String {
        let domain = match self {
            Self::Alibaba => "alibaba.com",
            Self::OneSixEightEight => "detail.1688.com",
            Self::Lcsc => "lcsc.com",
            Self::Baidu => "baidu.com",
        };
        Self::Baidu.search_url(&format!("site:{domain} {query}"))
    }

    /// Whether a URL is a listing page on this source's marketplace.
    pub fn is_listing_url(&self, url: &str) -> bool {
        let url = url.trim().trim_end_matches('/');
        match self {
            Self::Alibaba => {
                (url.contains("alibaba.com/product-detail/")
                    || url.contains("alibaba.com/showroom/"))
                    && !url.contains("search?")
            }
            Self::OneSixEightEight => {
                url.contains("detail.1688.com/offer/") && !url.contains("search")
            }
            Self::Lcsc => {
                url.contains("lcsc.com/product-detail/") || url.contains("lcsc.com/global/")
            }
            Self::Baidu => false,
        }
    }
}

fn urlencoding(input: &str) -> String {
    input
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SupplierPricingConfig {
    /// Reader base URL, e.g. `https://r.jina.ai`.
    pub reader_base_url: String,
    /// Optional reader API key (`Authorization: Bearer`).
    pub api_key: Option<String>,
    /// Maximum listing pages crawled per source per query.
    pub max_listings_per_source: usize,
    /// Minimum interval between two reader requests for one source.
    pub min_request_interval: Duration,
    /// Total retry attempts after the first request (default 3 → 4 requests).
    pub max_retries: usize,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    /// Consecutive failures before a source circuit opens.
    pub circuit_failure_threshold: usize,
    pub circuit_cooldown: Duration,
    /// Reader request timeout.
    pub timeout: Duration,
    /// Global concurrency across all sources.
    pub max_concurrent_requests: usize,
    /// Reader markdown cache TTL (same URL).
    pub cache_ttl: Duration,
    /// Default target quantity used when the caller does not specify one.
    pub default_target_quantity: u64,
    /// CNY→USD rate used ONLY when comparing offers across currencies.
    /// `None` forbids cross-currency comparison (same-currency only).
    pub fx_cny_usd: Option<f64>,
    /// Optional on-disk reader cache (JSON): makes repeat runs and restarts
    /// resume without re-hitting the reader for pages fetched within the TTL.
    pub cache_path: Option<std::path::PathBuf>,
}

impl Default for SupplierPricingConfig {
    fn default() -> Self {
        Self {
            reader_base_url: "https://r.jina.ai".to_string(),
            api_key: None,
            max_listings_per_source: 8,
            min_request_interval: Duration::from_millis(1200),
            max_retries: 3,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            circuit_failure_threshold: 5,
            circuit_cooldown: Duration::from_secs(300),
            timeout: Duration::from_secs(60),
            max_concurrent_requests: 4,
            cache_ttl: Duration::from_secs(86400),
            default_target_quantity: 100,
            fx_cny_usd: Some(0.1398),
            cache_path: None,
        }
    }
}

impl SupplierPricingConfig {
    /// Build from the process environment:
    /// `JINA_API_KEY` (optional), `JINA_BASE_URL` (default `https://r.jina.ai`).
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(base) = std::env::var(apex_core::env::JINA_BASE_URL) {
            let base = base.trim().trim_end_matches('/');
            if !base.is_empty() {
                config.reader_base_url = base.to_string();
            }
        }
        if let Ok(key) = std::env::var(apex_core::env::JINA_API_KEY) {
            let key = key.trim();
            if !key.is_empty() {
                config.api_key = Some(key.to_string());
            }
        }
        if let Ok(path) = std::env::var(apex_core::env::APEX_SUPPLIER_PRICING_CACHE) {
            let path = path.trim();
            if !path.is_empty() {
                config.cache_path = Some(std::path::PathBuf::from(path));
            }
        }
        if let Ok(rate) = std::env::var(apex_core::env::APEX_FX_CNY_USD) {
            if let Ok(rate) = rate.trim().parse::<f64>() {
                if rate.is_finite() && rate > 0.0 {
                    config.fx_cny_usd = Some(rate);
                }
            }
        }
        config
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors and retry classification
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SupplierError {
    #[error("supplier pricing request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("reader refused {target}: HTTP {status} {reason}")]
    ReaderRefused {
        target: String,
        status: u16,
        reason: String,
    },
    #[error("reader quota exhausted for {target}")]
    ReaderQuota { target: String },
    #[error("source {source_name} circuit is open (retry in {retry_in:?})")]
    CircuitOpen {
        source_name: &'static str,
        retry_in: Duration,
    },
    #[error("no listing pages discovered for {query} on {source_name}")]
    NoListings {
        query: String,
        source_name: &'static str,
    },
    #[error("reader returned no usable content for {target}")]
    EmptyContent { target: String },
    #[error("supplier pricing pipeline is shutting down")]
    ShuttingDown,
    #[error(
        "reader temporarily blocked {target} by the target site (blocked_until={blocked_until:?})"
    )]
    SourceBlocked {
        target: String,
        blocked_until: Option<chrono::DateTime<chrono::Utc>>,
    },
}

/// What to do after a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAction {
    /// Retry after the given delay.
    Retry(Duration),
    /// Do not retry; fail now.
    Fail,
}

/// Classify an HTTP failure into a retry decision.
///
/// - 429: retry honouring `Retry-After` (clamped); falls back to the caller's
///   backoff when the header is absent.
/// - 425 / 5xx: retry with backoff.
/// - 402 (reader quota), 400, 401, 403: permanent — a retry would burn quota
///   or repeat an authorization problem.
pub fn classify_retry(
    status: u16,
    headers: &HeaderMap,
    base_backoff: Duration,
    max_backoff: Duration,
) -> RetryAction {
    match status {
        429 => {
            let after = headers
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(base_backoff);
            // Honour the reader's rate window; the caller's backoff cap is
            // for transient failures, not for an explicit Retry-After.
            RetryAction::Retry(
                after
                    .min(Duration::from_secs(120))
                    .max(Duration::from_millis(500)),
            )
        }
        425 | 500 | 502 | 503 | 504 => RetryAction::Retry(base_backoff.min(max_backoff)),
        _ => RetryAction::Fail,
    }
}

/// Jittered backoff for attempt `n` (0-based).
pub fn backoff_for_attempt(attempt: usize, base: Duration, max: Duration) -> Duration {
    let exp = 2u32.saturating_pow(attempt.min(10) as u32) as u64;
    let raw = base.as_millis().saturating_mul(exp as u128) as u64;
    let capped = raw.min(max.as_millis() as u64);
    let jitter = rand::thread_rng().gen_range(0..=capped / 2 + 1);
    Duration::from_millis(capped - jitter / 2 + jitter / 2)
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit + pacing + cache (per pipeline instance)
// ─────────────────────────────────────────────────────────────────────────────

struct SourceState {
    consecutive_failures: usize,
    opened_at: Option<Instant>,
    last_request_at: Option<Instant>,
    /// When the source started serving shells (rows without content): back
    /// off entirely for the cooldown instead of burning reader quota.
    shell_opened_at: Option<Instant>,
}

impl SourceState {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            opened_at: None,
            last_request_at: None,
            shell_opened_at: None,
        }
    }
}

struct ReaderCacheEntry {
    stored_at: Instant,
    content: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// A single successful reader fetch.
#[derive(Debug, Clone)]
pub struct ReaderContent {
    pub target_url: String,
    pub content: String,
    pub status: u16,
    pub attempts: usize,
    pub served_from_cache: bool,
    /// Reader format actually returned ("markdown" or "text").
    pub format: &'static str,
}

pub struct SupplierPricingClient {
    config: SupplierPricingConfig,
    http: reqwest::Client,
    semaphore: Semaphore,
    states: Mutex<HashMap<PricingSource, SourceState>>,
    cache: Mutex<HashMap<String, ReaderCacheEntry>>,
    request_counter: AtomicU64,
}

impl SupplierPricingClient {
    pub fn new(config: SupplierPricingConfig) -> Result<Self, SupplierError> {
        let http = external_client(config.timeout, None)?;
        let mut cache = HashMap::new();
        if let Some(path) = &config.cache_path {
            if let Ok(raw) = std::fs::read_to_string(path) {
                let now = unix_now();
                // Append-only JSONL: the last entry per URL wins.
                for line in raw.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok((url, entry)) = serde_json::from_str::<(String, DiskCacheEntry)>(line)
                    {
                        let age = now.saturating_sub(entry.stored_unix);
                        if age <= config.cache_ttl.as_secs() && is_cacheable_content(&entry.content)
                        {
                            cache.insert(
                                url,
                                ReaderCacheEntry {
                                    stored_at: Instant::now(),
                                    content: entry.content,
                                },
                            );
                        }
                    }
                }
            }
        }
        Ok(Self {
            semaphore: Semaphore::new(config.max_concurrent_requests.max(1)),
            states: Mutex::new(HashMap::new()),
            cache: Mutex::new(cache),
            config,
            http,
            request_counter: AtomicU64::new(0),
        })
    }

    pub fn config(&self) -> &SupplierPricingConfig {
        &self.config
    }

    /// Fetch a target URL through the reader with pacing, caching, circuit
    /// breaking and the full retry policy.
    pub async fn fetch(
        &self,
        target_url: &str,
        source: PricingSource,
    ) -> Result<ReaderContent, SupplierError> {
        self.fetch_inner(target_url, source, true).await
    }

    /// Like [`Self::fetch`], but bypasses the cache read: used for a second
    /// attempt when the first response rendered as an empty shell (the
    /// reader's server-side render varies per attempt).
    pub async fn fetch_fresh(
        &self,
        target_url: &str,
        source: PricingSource,
    ) -> Result<ReaderContent, SupplierError> {
        self.fetch_inner(target_url, source, false).await
    }

    async fn fetch_inner(
        &self,
        target_url: &str,
        source: PricingSource,
        use_cache: bool,
    ) -> Result<ReaderContent, SupplierError> {
        // Cache first — no reader traffic for a URL seen recently.
        if use_cache {
            if let Some(entry) = self.cache_hit(target_url) {
                return Ok(ReaderContent {
                    target_url: target_url.to_string(),
                    content: entry,
                    status: 200,
                    attempts: 1,
                    served_from_cache: true,
                    format: "markdown",
                });
            }
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| SupplierError::ShuttingDown)?;

        self.pace(source).await;
        self.check_circuit(source)?;

        let reader_url = format!(
            "{}/{}",
            self.config.reader_base_url.trim_end_matches('/'),
            target_url
        );
        let mut attempts = 0usize;
        // First attempt uses markdown; if the reader returns an empty body,
        // one follow-up with `text` is allowed (format fallback, not a
        // network retry).
        let mut format: &'static str = "markdown";
        let mut fallback_tried = false;

        loop {
            attempts += 1;
            let headers = self.reader_headers(format);
            let response = self.http.get(&reader_url).headers(headers).send().await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    // Network/timeout/connection errors are transient.
                    if self
                        .should_retry(source, attempts, None, error.to_string())
                        .await
                    {
                        continue;
                    }
                    return Err(SupplierError::Http(error));
                }
            };

            let status = response.status().as_u16();
            if status == 200 {
                let body = response.text().await.unwrap_or_default();
                if !body.trim().is_empty() {
                    self.record_success(source);
                    self.cache_put(target_url, body.clone());
                    self.request_counter.fetch_add(1, Ordering::Relaxed);
                    return Ok(ReaderContent {
                        target_url: target_url.to_string(),
                        content: body,
                        status,
                        attempts,
                        served_from_cache: false,
                        format,
                    });
                }
                if !fallback_tried {
                    // Empty markdown: the target may only expose plain text.
                    format = "text";
                    fallback_tried = true;
                    continue;
                }
                return Err(SupplierError::EmptyContent {
                    target: target_url.to_string(),
                });
            }

            // A dated temporary site block (Alibaba-style "blocked until …")
            // is not a permanent refusal: skip the source for this run rather
            // than burning retry attempts against a known-bad window.
            if status == 403 {
                let body = response.text().await.unwrap_or_default();
                if let Some(until) = parse_block_until(&body) {
                    self.record_failure(source);
                    return Err(SupplierError::SourceBlocked {
                        target: target_url.to_string(),
                        blocked_until: Some(until),
                    });
                }
                self.record_failure(source);
                return Err(SupplierError::ReaderRefused {
                    target: target_url.to_string(),
                    status,
                    reason: "permanent reader refusal".to_string(),
                });
            }

            let retry = classify_retry(
                status,
                response.headers(),
                self.config.base_backoff,
                self.config.max_backoff,
            );
            match retry {
                RetryAction::Retry(delay) => {
                    if self.should_retry_with_delay(source, attempts, delay).await {
                        continue;
                    }
                    return Err(SupplierError::ReaderRefused {
                        target: target_url.to_string(),
                        status,
                        reason: "retry budget exhausted".to_string(),
                    });
                }
                RetryAction::Fail => {
                    self.record_failure(source);
                    if status == 402 {
                        return Err(SupplierError::ReaderQuota {
                            target: target_url.to_string(),
                        });
                    }
                    return Err(SupplierError::ReaderRefused {
                        target: target_url.to_string(),
                        status,
                        reason: "permanent reader refusal".to_string(),
                    });
                }
            }
        }
    }

    /// Total reader requests issued (test/metrics seam).
    pub fn request_count(&self) -> u64 {
        self.request_counter.load(Ordering::Relaxed)
    }

    // ── internals ──────────────────────────────────────────────────────────

    fn reader_headers(&self, format: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Return-Format",
            HeaderValue::from_static(match format {
                "markdown" => "markdown",
                _ => "text",
            }),
        );
        if let Some(key) = &self.config.api_key {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {key}")) {
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
        }
        headers
    }

    async fn pace(&self, source: PricingSource) {
        let wait = {
            let mut states = self.lock_states();
            let state = states.entry(source).or_insert_with(SourceState::new);
            match state.last_request_at {
                Some(last) => {
                    let elapsed = last.elapsed();
                    if elapsed < self.config.min_request_interval {
                        self.config.min_request_interval - elapsed
                    } else {
                        Duration::ZERO
                    }
                }
                None => Duration::ZERO,
            }
        };
        if !wait.is_zero() {
            sleep(wait).await;
        }
        let mut states = self.lock_states();
        states
            .entry(source)
            .or_insert_with(SourceState::new)
            .last_request_at = Some(Instant::now());
    }

    fn check_circuit(&self, source: PricingSource) -> Result<(), SupplierError> {
        let states = self.lock_states();
        if let Some(state) = states.get(&source) {
            for opened_at in [state.opened_at, state.shell_opened_at]
                .into_iter()
                .flatten()
            {
                let retry_in = self
                    .config
                    .circuit_cooldown
                    .saturating_sub(opened_at.elapsed());
                if !retry_in.is_zero() {
                    return Err(SupplierError::CircuitOpen {
                        source_name: source.as_str(),
                        retry_in,
                    });
                }
            }
        }
        Ok(())
    }

    /// A source served a shell (no usable rows): back off for the cooldown
    /// rather than retrying into a known-bad render window.
    fn record_shell(&self, source: PricingSource) {
        let mut states = self.lock_states();
        states
            .entry(source)
            .or_insert_with(SourceState::new)
            .shell_opened_at = Some(Instant::now());
    }

    async fn should_retry(
        &self,
        source: PricingSource,
        attempts: usize,
        _status: Option<u16>,
        _reason: String,
    ) -> bool {
        self.should_retry_with_delay(source, attempts, Duration::ZERO)
            .await
    }

    async fn should_retry_with_delay(
        &self,
        source: PricingSource,
        attempts: usize,
        explicit: Duration,
    ) -> bool {
        if attempts > self.config.max_retries {
            self.record_failure(source);
            return false;
        }
        self.record_failure(source);
        let delay = if explicit.is_zero() {
            backoff_for_attempt(attempts, self.config.base_backoff, self.config.max_backoff)
        } else {
            explicit
        };
        sleep(delay).await;
        self.pace(source).await;
        self.check_circuit(source).is_ok()
    }

    fn record_failure(&self, source: PricingSource) {
        let mut states = self.lock_states();
        let state = states.entry(source).or_insert_with(SourceState::new);
        state.consecutive_failures += 1;
        if state.consecutive_failures >= self.config.circuit_failure_threshold {
            state.opened_at = Some(Instant::now());
        }
    }

    fn record_success(&self, source: PricingSource) {
        let mut states = self.lock_states();
        let state = states.entry(source).or_insert_with(SourceState::new);
        state.consecutive_failures = 0;
        state.opened_at = None;
    }

    fn lock_states(&self) -> std::sync::MutexGuard<'_, HashMap<PricingSource, SourceState>> {
        self.states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, ReaderCacheEntry>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cache_hit(&self, url: &str) -> Option<String> {
        let mut cache = self.lock_cache();
        let entry = cache.get(url)?;
        if entry.stored_at.elapsed() > self.config.cache_ttl {
            cache.remove(url);
            return None;
        }
        Some(entry.content.clone())
    }

    fn cache_put(&self, url: &str, content: String) {
        if !is_cacheable_content(&content) {
            return;
        }
        {
            let mut cache = self.lock_cache();
            if cache.len() >= 512 {
                let oldest = cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.stored_at)
                    .map(|(key, _)| key.clone());
                if let Some(key) = oldest {
                    cache.remove(&key);
                }
            }
            cache.insert(
                url.to_string(),
                ReaderCacheEntry {
                    stored_at: Instant::now(),
                    content: content.clone(),
                },
            );
        }
        self.persist_cache(url, content);
    }

    /// Best-effort append of one entry to the configured disk cache (JSONL:
    /// O(1) writes; the last entry per URL wins on load).
    fn persist_cache(&self, url: &str, content: String) {
        let Some(path) = &self.config.cache_path else {
            return;
        };
        if !is_cacheable_content(&content) {
            return;
        }
        let entry = DiskCacheEntry {
            stored_unix: unix_now(),
            content,
        };
        if let Ok(line) = serde_json::to_string(&(url, entry)) {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DiskCacheEntry {
    stored_unix: u64,
    content: String,
}

/// A reader response with no price signal and no listing links is a render
/// shell (or an error page); caching it would poison the TTL window.
fn is_cacheable_content(content: &str) -> bool {
    content.len() > 20_000
        || content.contains('$')
        || content.contains('¥')
        || content.contains("product-detail")
        || content.contains("results found")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Listing discovery
// ─────────────────────────────────────────────────────────────────────────────

/// Extract candidate listing URLs from reader markdown.
pub fn extract_listing_links(markdown: &str, source: PricingSource) -> Vec<String> {
    let mut links = Vec::new();
    // Markdown links: [label](url)
    let mut rest = markdown;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let end = after.find(')').map(|i| start + 2 + i).unwrap_or(rest.len());
        let url = &rest[start + 2..end];
        let url = url.trim().trim_matches('<').trim_matches('>');
        if url.starts_with("http") && source.is_listing_url(url) {
            links.push(url.to_string());
        }
        rest = &rest[end.min(rest.len())..];
    }
    // Bare URLs.
    for token in markdown.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        let token = token.trim();
        if token.starts_with("http") && source.is_listing_url(token) {
            links.push(token.to_string());
        }
    }
    // Percent-decoded `url=` params (search-engine redirect wrappers).
    for token in markdown.split(['&', ' ', '"']) {
        if let Some(rest) = token.strip_prefix("url=") {
            if let Some(decoded) = percent_decode(rest) {
                if source.is_listing_url(&decoded) {
                    links.push(decoded);
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    links.retain(|link| seen.insert(link.trim_end_matches('/').to_string()));
    links
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Offer parsing: teaser avoidance and quantity tiers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceOffer {
    pub unit_price: f64,
    pub currency: String,
    pub moq: u64,
    pub max_qty: Option<u64>,
    /// True when the price is a "from"/promo single that does not cover a
    /// quantity tier (never quotable).
    pub teaser: bool,
    /// High bound of a negotiated range ("US $0.01 - US $0.05"). Ranges are
    /// never quotable: quoting the low end of a range is the same deception
    /// as quoting a teaser.
    pub price_max: Option<f64>,
    pub raw: String,
}

/// Parse every price mention in reader markdown into offers.
pub fn parse_price_offers(markdown: &str) -> Vec<PriceOffer> {
    let mut offers = Vec::new();
    let lines: Vec<&str> = markdown.lines().map(str::trim).collect();

    // Quantity-tier lines: `1-99 : US $0.42`, `100 - 999 pcs: $0.08`,
    // `>=1000: $0.05`, `1000+: $0.05`.
    let tier_re = regex_tier();
    for line in &lines {
        if let Some(captures) = tier_re.captures(line) {
            let low = captures.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let high = captures.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            let price = captures.get(3).map(|m| m.as_str().trim()).unwrap_or("");
            if let Some(unit) = parse_price_token(price) {
                let moq = low.parse::<u64>().unwrap_or(1).max(1);
                let max_qty = high.parse::<u64>().ok();
                offers.push(PriceOffer {
                    unit_price: unit.0,
                    currency: unit.1,
                    moq,
                    max_qty,
                    teaser: false,
                    price_max: None,
                    raw: line.to_string(),
                });
            }
        }
    }

    // Markdown table rows: `| 1 - 99 | US $0.42 |` or `| 1 - 99 | 100 - 999 |`.
    let table_tier_re = regex_table_tier();
    for line in &lines {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let open_tier_re = regex_open_cell();
        for pair in cells.windows(2) {
            let qty = pair[0];
            let price_cell = pair[1];
            if let Some(captures) = open_tier_re.captures(qty) {
                // `1000+` cell: open-ended tier.
                if let Some(unit) = parse_price_token(price_cell) {
                    let low = captures
                        .name("low")
                        .map(|m| m.as_str().replace(',', ""))
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(1)
                        .max(1);
                    offers.push(PriceOffer {
                        unit_price: unit.0,
                        currency: unit.1,
                        moq: low,
                        max_qty: None,
                        teaser: false,
                        price_max: None,
                        raw: line.to_string(),
                    });
                }
                continue;
            }
            if let Some(captures) = table_tier_re.captures(qty) {
                let low = captures.name("low").map(|m| m.as_str()).unwrap_or("1");
                let high = captures.name("high").map(|m| m.as_str());
                if let Some(unit) = parse_price_token(price_cell) {
                    let moq = low.replace(',', "").parse::<u64>().unwrap_or(1).max(1);
                    let max_qty = high.and_then(|h| h.replace(',', "").parse::<u64>().ok());
                    offers.push(PriceOffer {
                        unit_price: unit.0,
                        currency: unit.1,
                        moq,
                        max_qty,
                        teaser: false,
                        price_max: None,
                        raw: line.to_string(),
                    });
                }
            }
        }
    }

    // "N+ : $x" / "N+ pcs: $x" step-break lines (open-ended tiers).
    let step_break_re = regex_step_break();
    for line in &lines {
        if let Some(captures) = step_break_re.captures(line) {
            let low = captures.get(1).map(|m| m.as_str().trim()).unwrap_or("1");
            let price = captures.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            if let Some(unit) = parse_price_token(price) {
                offers.push(PriceOffer {
                    unit_price: unit.0,
                    currency: unit.1,
                    moq: low.replace(',', "").parse::<u64>().unwrap_or(1).max(1),
                    max_qty: None,
                    teaser: false,
                    price_max: None,
                    raw: line.to_string(),
                });
            }
        }
    }

    // MOQ mention followed by a price within the next two lines.
    let moq_re = regex_moq();
    let price_re = regex_price();
    for (index, line) in lines.iter().enumerate() {
        let Some(moq_match) = moq_re.captures(line) else {
            continue;
        };
        let moq: u64 = moq_match
            .get(1)
            .and_then(|m| m.as_str().replace(',', "").parse().ok())
            .unwrap_or(1)
            .max(1);
        for other in lines.iter().skip(index + 1).take(3) {
            if let Some(price_match) = price_re.captures(other) {
                let price = price_match.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                if let Some(unit) = parse_price_token(price) {
                    offers.push(PriceOffer {
                        unit_price: unit.0,
                        currency: unit.1,
                        moq,
                        max_qty: None,
                        teaser: false,
                        price_max: None,
                        raw: other.to_string(),
                    });
                }
                break;
            }
        }
    }

    // Concatenated break strings, e.g. LCSC `100+$0.0052 1,000+$0.0047 More
    // 10,000+$0.0043`. Scanned over the whole text (they sit inside table
    // cells), with a guard window that skips totals ("Ext. Price: $0.52",
    // subtotals, full-reel lines).
    let break_re = regex_break();
    for captures in break_re.captures_iter(markdown) {
        let Some(m) = captures.get(0) else { continue };
        let start = m.start();
        let line_start = markdown[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = markdown[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(markdown.len());
        let whole_line = &markdown[line_start..line_end];
        let before = &markdown[..start];
        let window: String = before.chars().rev().take(60).collect();
        let lower = window.to_lowercase();
        if [
            "ext. price",
            "ext price",
            "subtotal",
            "total price",
            "full reel",
            "extended",
        ]
        .iter()
        .any(|guard| lower.contains(guard))
        {
            continue;
        }
        let low = captures
            .name("low")
            .map(|v| v.as_str().replace(',', ""))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1)
            .max(1);
        let price = captures
            .name("price")
            .map(|v| v.as_str().trim())
            .unwrap_or("");
        if let Some(unit) = parse_price_token(price) {
            offers.push(PriceOffer {
                unit_price: unit.0,
                currency: unit.1,
                moq: low,
                max_qty: None,
                teaser: false,
                price_max: None,
                raw: whole_line.trim().to_string(),
            });
        }
    }

    // Negotiated ranges ("US $0.01 - US $0.05"): recorded with both bounds;
    // never quotable (the low end is bait).
    let range_re = regex_price_range();
    for captures in range_re.captures_iter(markdown) {
        let low = captures
            .name("low")
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        let high = captures
            .name("high")
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        let (Some(min), Some(max)) = (parse_price_token(low), parse_price_token(high)) else {
            continue;
        };
        if min.1 != max.1 || min.0 >= max.0 {
            continue;
        }
        let Some(m) = captures.get(0) else {
            continue;
        };
        let start = m.start();
        let line_start = markdown[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = markdown[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(markdown.len());
        offers.push(PriceOffer {
            unit_price: min.0,
            currency: min.1,
            moq: 1,
            max_qty: None,
            teaser: false,
            price_max: Some(max.0),
            raw: markdown[line_start..line_end].trim().to_string(),
        });
    }

    // "From US $X" teasers and standalone prices without a tier. Lines whose
    // price mentions already yielded quantity breaks ("100+$0.0052") must not
    // re-emit the same price as a bogus MOQ-1 tier.
    for line in &lines {
        if regex_break().is_match(line)
            || tier_re.is_match(line)
            || regex_price_range().is_match(line)
        {
            continue;
        }
        if let Some(captures) = regex_price().captures(line) {
            let price = captures.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if let Some(unit) = parse_price_token(price) {
                let teaser = line.to_lowercase().contains("from ")
                    || line.to_lowercase().contains("from$")
                    || line.contains("起订");
                offers.push(PriceOffer {
                    unit_price: unit.0,
                    currency: unit.1,
                    moq: 1,
                    max_qty: None,
                    teaser,
                    price_max: None,
                    raw: line.to_string(),
                });
            }
        }
    }

    // Promo rows are time-limited: their prices must not be quoted as stable
    // evidence, so every offer on a promo line is demoted to a teaser.
    for offer in &mut offers {
        if offer.raw.to_lowercase().contains("promo")
            || offer.raw.to_lowercase().contains("promotion")
        {
            offer.teaser = true;
        }
    }

    // Deduplicate identical offers.
    let mut seen = std::collections::HashSet::new();
    offers.retain(|offer| {
        seen.insert(format!(
            "{}|{}|{}|{:?}|{}|{:?}",
            offer.unit_price,
            offer.currency,
            offer.moq,
            offer.max_qty,
            offer.teaser,
            offer.price_max
        ))
    });
    offers
}

/// The quotable price for a requested quantity: the lowest non-teaser tier
/// whose `moq <= qty` and (when bounded) `qty <= max_qty`. Teaser-only
/// listings return `None` — an inferred price is never supplier evidence.
pub fn quotable_price(
    offers: &[PriceOffer],
    qty: u64,
    preferred_currency: Option<&str>,
) -> Option<PriceOffer> {
    let mut best: Option<&PriceOffer> = None;
    for offer in offers {
        if offer.teaser || offer.price_max.is_some() {
            continue;
        }
        if offer.moq > qty {
            continue;
        }
        if let Some(max) = offer.max_qty {
            if qty > max {
                continue;
            }
        }
        let covers = best.is_none()
            || offer.unit_price < best.as_ref().map(|b| b.unit_price).unwrap_or(f64::MAX)
            || (offer.unit_price == best.as_ref().map(|b| b.unit_price).unwrap_or(f64::MAX)
                && best
                    .as_ref()
                    .is_some_and(|b| currency_preferred(offer, b, preferred_currency)));
        if covers {
            best = Some(offer);
        }
    }
    best.cloned()
}

fn currency_preferred(a: &PriceOffer, b: &PriceOffer, preferred: Option<&str>) -> bool {
    match preferred {
        Some(currency) => a.currency == currency && b.currency != currency,
        None => false,
    }
}

/// One listing's offer set plus its teaser flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListingQuote {
    pub listing_url: String,
    pub source: String,
    pub offers: Vec<PriceOffer>,
    pub quotable: Option<PriceOffer>,
    pub reconfirmation_required: bool,
}

/// The chosen lowest price across crawled listings, with the statistics that
/// make the choice defensible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowestQuote {
    pub listing_url: String,
    pub source: String,
    pub unit_price: f64,
    pub currency: String,
    pub moq: u64,
    pub max_qty: Option<u64>,
    pub candidates: Vec<ListingQuote>,
    pub teaser_only_listings: usize,
    /// Median quotable unit price (USD-normalized) across all crawled
    /// listings; the anchor used for outlier rejection.
    pub median_unit_usd: Option<f64>,
    /// Number of quotable offers the choice was made from.
    pub sample_size: usize,
    /// True when any compared offer was converted from CNY.
    pub fx_applied: bool,
    /// True when the chosen price was far below the median (kept, flagged).
    pub outlier_risk: bool,
    /// The raw fetched text line(s) the chosen price was parsed from.
    pub evidence_raw: String,
    /// Chosen listing is a platform "Other Suppliers" row whose price the
    /// platform itself marks as needing reconfirmation.
    pub reconfirmation_required: bool,
    /// Evidence is too thin to auto-publish: a single supplier-market row
    /// with nothing else to corroborate it. Reported, never persisted as a
    /// verified quote.
    pub low_confidence: bool,
}

/// Convert an offer to USD for cross-currency comparison.
pub fn offer_to_usd(offer: &PriceOffer, fx: Option<f64>) -> Option<f64> {
    match offer.currency.as_str() {
        "CNY" => fx.map(|rate| offer.unit_price * rate),
        _ => Some(offer.unit_price),
    }
}

/// Choose the lowest quotable price across listings for `qty`, statistically
/// guarded:
/// - offers whose tier does not cover `qty`, or that are teasers, never count;
/// - when `fx` is set, currencies are normalized to USD for the comparison;
///   when it is `None`, cross-currency comparison is forbidden (fail closed);
/// - with >= 3 samples, offers below 20% of the median are rejected as
///   outliers (a teaser masquerading as a tier would land there) and the
///   lowest *in-bracket* offer wins; when every offer is an outlier the
///   lowest is kept but flagged `outlier_risk`.
pub fn lowest_quote(
    listings: &[ListingQuote],
    qty: u64,
    preferred_currency: Option<&str>,
    fx: Option<f64>,
) -> Option<LowestQuote> {
    let currency_ok = |offer: &PriceOffer| -> bool {
        match (fx, preferred_currency) {
            (Some(_), _) => offer.currency == "USD" || offer.currency == "CNY",
            (None, Some(currency)) => offer.currency == currency,
            (None, None) => true,
        }
    };

    // (listing_index, offer_index, offer, usd)
    let mut quotable: Vec<(usize, usize, PriceOffer, f64)> = Vec::new();
    for (index, listing) in listings.iter().enumerate() {
        let Some(offer) = quotable_price(&listing.offers, qty, preferred_currency) else {
            continue;
        };
        if !currency_ok(&offer) {
            continue;
        }
        let usd = match offer_to_usd(&offer, fx) {
            Some(usd) => usd,
            None => continue,
        };
        quotable.push((index, 0, offer, usd));
    }

    if quotable.is_empty() {
        return None;
    }
    let sample_size = quotable.len();
    let mut usd_values: Vec<f64> = quotable.iter().map(|(_, _, _, usd)| *usd).collect();
    usd_values.sort_by(|a, b| a.total_cmp(b));
    let median = usd_values[usd_values.len() / 2];
    let outlier_floor = median * 0.2;

    let mut best: Option<(usize, PriceOffer, f64, bool)> = None;
    for (index, _, offer, usd) in &quotable {
        let is_outlier = sample_size >= 3 && *usd < outlier_floor;
        let better = match &best {
            None => true,
            Some((_, current, current_usd, current_outlier)) => {
                if is_outlier != *current_outlier {
                    // A non-outlier always beats an outlier, whatever the
                    // price — an outlier is a teaser masquerading as a tier.
                    !is_outlier
                } else {
                    *usd < *current_usd
                        || (*usd == *current_usd
                            && offer.max_qty.unwrap_or(0) > current.max_qty.unwrap_or(0))
                }
            }
        };
        if better {
            best = Some((*index, offer.clone(), *usd, is_outlier));
        }
    }
    let (index, offer, _usd, outlier_risk) = best?;
    let evidence_raw = offer.raw.clone();
    let reconfirmation_required = listings[index].reconfirmation_required;
    let low_confidence = reconfirmation_required && sample_size == 1;
    let fx_applied = quotable
        .iter()
        .any(|(_, _, offer, _)| offer.currency == "CNY");
    Some(LowestQuote {
        listing_url: listings[index].listing_url.clone(),
        source: listings[index].source.clone(),
        unit_price: offer.unit_price,
        currency: offer.currency.clone(),
        moq: offer.moq,
        max_qty: offer.max_qty,
        candidates: listings.to_vec(),
        teaser_only_listings: listings
            .iter()
            .filter(|listing| {
                !listing.offers.is_empty()
                    && quotable_price(&listing.offers, qty, preferred_currency).is_none()
            })
            .count(),
        median_unit_usd: Some(median),
        sample_size,
        fx_applied,
        outlier_risk,
        evidence_raw,
        reconfirmation_required,
        low_confidence,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Regexes (lazily compiled once)
// ─────────────────────────────────────────────────────────────────────────────

fn regex_tier() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)
            (?P<low>[0-9,]+)\s*[-–—~]\s*(?P<high>[0-9,]+)\s*
            (?:pieces?|pcs|units?|件|个)?
            \s*[:：]\s*(?P<price>\$[0-9.,]+|US\s*\$[0-9.,]+|CNY[0-9.,]+|¥[0-9.,]+|€[0-9.,]+)",
        )
        .unwrap_or_else(|error| panic!("invalid tier regex: {error}"))
    })
}

fn regex_zero_stock() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // A real zero: `0` not preceded by another digit or comma
        // ("974,400 in stock" must NOT match "0 in stock").
        regex::Regex::new(r"(?i)(^|[^0-9,])0+\s*in\s*stock")
            .unwrap_or_else(|error| panic!("invalid zero-stock regex: {error}"))
    })
}

fn regex_block_until() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)blocked\s+until\s+(?P<when>[A-Z][a-z]{2}\s+[A-Z][a-z]{2}\s+\d{2}\s+\d{4}\s+\d{2}:\d{2}:\d{2}\s+GMT[+-]\d{4})",
        )
        .unwrap_or_else(|error| panic!("invalid block-until regex: {error}"))
    })
}

fn regex_lcsc_code() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?x)\bC([0-9]{5,})\b")
            .unwrap_or_else(|error| panic!("invalid lcsc-code regex: {error}"))
    })
}

fn regex_stock() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?i)(?P<count>[0-9][0-9,]*)\s*in stock")
            .unwrap_or_else(|error| panic!("invalid stock regex: {error}"))
    })
}

fn regex_open_cell() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?x)^(?:≥|>\s*)?(?P<low>[0-9][0-9,]*)\s*\+$")
            .unwrap_or_else(|error| panic!("invalid open-tier regex: {error}"))
    })
}

fn regex_table_tier() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?x)^(?P<low>[0-9][0-9,]*)\s*(?:-|–|—|~|to)\s*(?P<high>[0-9][0-9,]*)$")
            .unwrap_or_else(|error| panic!("invalid table-tier regex: {error}"))
    })
}

fn regex_break() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)(?P<low>[0-9][0-9,]*)\s*\+\s*(?P<price>\$[0-9][0-9.,]*|US\s*\$[0-9][0-9.,]*|CNY[0-9][0-9.,]*|CN¥[0-9][0-9.,]*|¥[0-9][0-9.,]*|€[0-9][0-9.,]*)",
        )
        .unwrap_or_else(|error| panic!("invalid break regex: {error}"))
    })
}

fn regex_step_break() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)^(?P<low>[0-9][0-9,]*)\s*\+\s*(?:pieces?|pcs|units?|件|个)?\s*[:：]?\s*(?P<price>\$[0-9][0-9.,]*|US\s*\$[0-9][0-9.,]*|CNY[0-9][0-9.,]*|CN¥[0-9][0-9.,]*|¥[0-9][0-9.,]*|€[0-9][0-9.,]*)",
        )
        .unwrap_or_else(|error| panic!("invalid step-break regex: {error}"))
    })
}

fn regex_moq() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:MOQ|minimum order|min\.? order|起订量|最小起订)[^\d]{0,12}([0-9][0-9,]*)",
        )
        .unwrap_or_else(|error| panic!("invalid moq regex: {error}"))
    })
}

fn regex_price_range() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)(?P<low>\$[0-9][0-9.,]*|US\s*\$[0-9][0-9.,]*|CNY[0-9][0-9.,]*|CN¥[0-9][0-9.,]*|¥[0-9][0-9.,]*|€[0-9][0-9.,]*)\s*[-–—~]\s*(?P<high>\$[0-9][0-9.,]*|US\s*\$[0-9][0-9.,]*|CNY[0-9][0-9.,]*|CN¥[0-9][0-9.,]*|¥[0-9][0-9.,]*|€[0-9][0-9.,]*)",
        )
        .unwrap_or_else(|error| panic!("invalid price-range regex: {error}"))
    })
}

fn regex_price() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)
            (
              US\s*\$[0-9][0-9.,]*
            | \$[0-9][0-9.,]*
            | CNY\s*[0-9][0-9.,]*
            | ¥[0-9][0-9.,]*
            | €[0-9][0-9.,]*
            | RMB\s*[0-9][0-9.,]*
            | [0-9][0-9.,]*\s*元
            )",
        )
        .unwrap_or_else(|error| panic!("invalid price regex: {error}"))
    })
}

fn parse_price_token_stripped(digits: &str, currency: &str) -> Option<(f64, String)> {
    let digits = digits.trim().trim_start_matches('=').trim();
    let normalized = digits.replace(',', "");
    let value: f64 = normalized.parse().ok()?;
    (value.is_finite() && value > 0.0 && value <= 1_000_000.0)
        .then(|| (value, currency.to_string()))
}

/// Normalize a price token to (value, currency code).
fn parse_price_token(token: &str) -> Option<(f64, String)> {
    let token = token.trim();
    // "0.25元" suffix form (no prefix) is Chinese yuan.
    if let Some(stripped) = token.strip_suffix('元') {
        return parse_price_token_stripped(stripped.trim(), "CNY");
    }
    let _ = token; // suffix handled via strip_suffix above
    let (currency, digits): (&str, &str) = match token {
        t if t.starts_with("US $") => ("USD", &t[4..]),
        t if t.starts_with("US$") => ("USD", &t[3..]),
        t if t.starts_with("CNY") => ("CNY", &t[3..]),
        t if t.starts_with("CN¥") => ("CNY", &t[4..]),
        // '¥' is 2 UTF-8 bytes, '€' is 3.
        t if t.starts_with('¥') => ("CNY", &t[2..]),
        t if t.starts_with("RMB") => ("CNY", &t[3..]),
        t if t.starts_with('€') => ("EUR", &t[3..]),
        t if t.starts_with('$') => ("USD", &t[1..]),
        _ => return None,
    };
    // "0.31元" / "0.31 元" suffix form.
    if let Some(stripped) = digits.strip_suffix('元') {
        return parse_price_token_stripped(stripped, "CNY");
    }
    let digits = digits.trim().trim_start_matches('=').trim();
    let normalized = digits.replace(',', "");
    let value: f64 = normalized.parse().ok()?;
    (value.is_finite() && value > 0.0 && value <= 1_000_000.0)
        .then(|| (value, currency.to_string()))
}

/// A reader response that is a login/verification wall, not product content.
pub fn looks_like_login_wall(markdown: &str) -> bool {
    let text = markdown.to_lowercase();
    let short = markdown.chars().count() < 400;
    let wall_marker = [
        "sign in",
        "log in",
        "login",
        "captcha",
        "verify you are human",
        "please enable javascript",
    ]
    .iter()
    .any(|marker| text.contains(marker));
    short && wall_marker
}

/// Parse the reader's temporary-block payload ("blocked until <date>") that
/// sites like Alibaba return for anonymous readers. Returns the expiry when
/// the body is a dated abuse-alleviation block.
pub fn parse_block_until(body: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let re = regex_block_until();
    let captures = re.captures(body)?;
    let token = captures.name("when")?.as_str();
    chrono::DateTime::parse_from_str(token, "%a %b %d %Y %H:%M:%S GMT%z")
        .ok()
        .map(|when| when.with_timezone(&chrono::Utc))
}

/// Whether the reader returned a document (datasheet PDF/catalog) instead of
/// the product page — those carry no purchasable price tiers.
pub fn is_document_content(markdown: &str) -> bool {
    let text = markdown.to_lowercase();
    (text.contains("number of pages") && text.contains("markdown content"))
        || text.contains("reference sheet")
        || text.contains("product specifications in this catalog")
}

/// Whether the listing text says nothing is in stock (not worth pricing).
pub fn indicates_out_of_stock(markdown: &str) -> bool {
    let text = markdown.to_lowercase();
    [
        "0 in stock",
        "out of stock",
        "no stock",
        "stock: 0",
        "in stock: 0",
        "0 pieces in stock",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// One listing row reconstructed from a search-results page.
#[derive(Debug, Clone)]
pub struct SearchListing {
    pub listing_url: Option<String>,
    pub offers: Vec<PriceOffer>,
    pub in_stock: bool,
    /// Marketplace "Other Suppliers" row: priced by a third party on the
    /// platform and flagged by the platform itself as needing
    /// reconfirmation ("price and lead time need to be re-confirmed").
    pub reconfirmation_required: bool,
}

/// LCSC catalog code (`C12345`) mentioned on a row; used to synthesize a
/// listing URL for rows that carry prices but no href.
pub fn lcsc_code(line: &str) -> Option<String> {
    let re = regex_lcsc_code();
    re.captures(line)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Tokens of a BOM query used for listing relevance: alphanumeric runs of
/// two or more characters, lowercased.
pub fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else {
            if current.len() >= 2 {
                tokens.push(current.clone());
            }
            current.clear();
        }
    }
    if current.len() >= 2 {
        tokens.push(current);
    }
    tokens
}

/// Every token of the original query must appear in the row's text; a
/// relaxed-search row for another product class ("USB" cable vs "USB-C 16P"
/// receptacle) must never win.
pub fn row_matches_query(block: &SearchListing, query: &str) -> bool {
    let mut haystack = String::new();
    if let Some(url) = &block.listing_url {
        haystack.push_str(url);
        haystack.push(' ');
    }
    for offer in &block.offers {
        haystack.push_str(&offer.raw);
        haystack.push(' ');
    }
    let haystack = haystack.to_lowercase();
    query_tokens(query)
        .iter()
        .all(|token| haystack.contains(token))
}

/// Evaluation/dev-kit rows are a different product class from the bare part
/// a BOM orders; they must not be quoted for it.
pub fn is_kit_or_board_row(line: &str) -> bool {
    let lower = line.to_lowercase();
    [
        "evaluation board",
        "evaluation kit",
        "eval board",
        "dev board",
        "development board",
        "dev kit",
        "discovery kit",
        "arduino",
        "raspberry pi",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Rows in the "Other Suppliers" section state an estimated lead time
/// instead of an in-stock count.
pub fn is_supplier_market_row(line: &str) -> bool {
    line.to_lowercase().contains("estimated lead time")
}

/// Stock quantity mentioned on a line, if any.
pub fn stock_count(line: &str) -> Option<u64> {
    let lower = line.to_lowercase();
    if lower.contains("out of stock") {
        return Some(0);
    }
    let zero = regex_zero_stock();
    if zero.is_match(&lower) {
        return Some(0);
    }
    let re = regex_stock();
    re.captures(line)
        .and_then(|c| c.name("count"))
        .and_then(|m| m.as_str().replace(',', "").parse::<u64>().ok())
}

/// Reconstruct listing rows from a search-results page: each row is the line
/// carrying a listing URL plus its following lines, until the next row.
pub fn parse_search_page(markdown: &str, source: PricingSource) -> Vec<SearchListing> {
    let mut out: Vec<SearchListing> = Vec::new();
    let mut current: Option<SearchListing> = None;
    for line in markdown.lines() {
        let first = extract_listing_links(line, source).into_iter().next();
        let line_offers = parse_price_offers(line);
        let stock = stock_count(line);
        let supplier_market = is_supplier_market_row(line);
        let synthesized =
            if first.is_none() && !line_offers.is_empty() && source == PricingSource::Lcsc {
                lcsc_code(line)
                    .map(|code| format!("https://www.lcsc.com/product-detail/{code}.html"))
            } else {
                None
            };
        if let Some(url) = first.or(synthesized) {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(SearchListing {
                listing_url: Some(url),
                offers: line_offers,
                in_stock: stock.map(|n| n > 0).unwrap_or(true),
                reconfirmation_required: supplier_market,
            });
        } else if let Some(block) = current.as_mut() {
            if !line_offers.is_empty() {
                block.offers.extend(line_offers);
            }
            if let Some(n) = stock {
                block.in_stock = n > 0;
            }
            block.reconfirmation_required |= supplier_market;
        }
    }
    if let Some(block) = current.take() {
        out.push(block);
    }
    out
}

/// Whether a search page reports zero hits.
pub fn zero_results(markdown: &str) -> bool {
    let lower = markdown.to_lowercase();
    [
        "0 results found",
        "no results found",
        "no data found",
        "couldn't find",
        "0 results",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Relaxed query candidates for marketplaces whose exact-match search returns
/// zero rows: the stem before the first dash/slash, and (for long monolithic
/// part numbers) the first 12 characters.
pub fn relaxed_queries(part: &str) -> Vec<String> {
    let trimmed = part.trim();
    let mut out = Vec::new();
    let stem: String = trimmed
        .split(['-', '/', ' ', '(', '#', '+'].as_ref())
        .next()
        .unwrap_or(trimmed)
        .to_string();
    if stem.chars().count() >= 4 && stem != trimmed {
        out.push(stem);
    }
    if trimmed.chars().count() > 12 && !trimmed.contains('-') {
        let prefix: String = trimmed.chars().take(12).collect();
        if prefix != trimmed {
            out.push(prefix);
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministic price for a part number: search each marketplace, crawl its
/// listings, and report the lowest quotable price.
pub struct SupplierPricePipeline {
    pub client: Arc<SupplierPricingClient>,
}

impl SupplierPricePipeline {
    pub fn new(client: Arc<SupplierPricingClient>) -> Self {
        Self { client }
    }

    pub async fn price_for_part(
        &self,
        part_number: &str,
        target_quantity: u64,
        preferred_currency: Option<&str>,
    ) -> Result<LowestQuote, SupplierError> {
        let mut listings = Vec::new();
        for source in [
            PricingSource::Alibaba,
            PricingSource::OneSixEightEight,
            PricingSource::Lcsc,
        ] {
            // Exact query first; when it yields no usable rows (zero results,
            // all rows out of stock, or rows without tiers), relax to the
            // part's stem/prefix — marketplaces index stems, not full
            // ordering strings.
            let mut selected: Option<(String, ReaderContent)> = None;
            for query in
                std::iter::once(part_number.to_string()).chain(relaxed_queries(part_number))
            {
                let url = source.search_url(&query);
                match self.client.fetch(&url, source).await {
                    Ok(fetched) => {
                        let usable = parse_search_page(&fetched.content, source)
                            .iter()
                            .any(|block| block.in_stock && !block.offers.is_empty());
                        if usable {
                            selected = Some((url, fetched));
                            break;
                        }
                        selected = Some((url, fetched)); // keep for fallback links
                        continue;
                    }
                    Err(SupplierError::CircuitOpen { .. }) => break,
                    Err(_) => continue,
                }
            }
            let Some((search_url, mut content)) = selected else {
                continue;
            };
            // The reader's server-side render is per-attempt: a shell page
            // (no rows, no links, no zero-results message) gets one fresh
            // attempt before the part is declared unpriced.
            // Transient reader throttling renders a shell; push through with
            // bounded fresh attempts before giving up on this source.
            let mut attempts = 0;
            loop {
                let shell = parse_search_page(&content.content, source)
                    .iter()
                    .all(|block| !block.in_stock || block.offers.is_empty())
                    && extract_listing_links(&content.content, source).is_empty()
                    && !zero_results(&content.content);
                if !shell {
                    break;
                }
                if attempts >= 4 {
                    self.client.record_shell(source);
                    break;
                }
                attempts += 1;
                sleep(Duration::from_secs(25)).await;
                match self.client.fetch_fresh(&search_url, source).await {
                    Ok(fresh) => content = fresh,
                    Err(_) => break,
                }
            }
            let still_shell = parse_search_page(&content.content, source)
                .iter()
                .all(|block| !block.in_stock || block.offers.is_empty())
                && extract_listing_links(&content.content, source).is_empty()
                && !zero_results(&content.content);
            if still_shell {
                continue;
            }

            // Primary evidence: the server-rendered search page carries
            // per-listing quantity tiers. Listing pages on these marketplaces
            // frequently render prices client-side (or the reader lands on a
            // datasheet PDF), so the search rows are the reliable way in.
            let mut blocks = parse_search_page(&content.content, source);
            blocks.retain(|block| {
                // Wrong-class rows (a relaxed "USB" hit for an industrial
                // cable, or an evaluation kit for a bare part) must never be
                // quoted for this part number.
                let text_ok = block
                    .offers
                    .iter()
                    .all(|offer| !is_kit_or_board_row(&offer.raw));
                text_ok && row_matches_query(block, part_number)
            });
            let usable_count = blocks
                .iter()
                .filter(|block| block.in_stock && !block.offers.is_empty())
                .count();
            // Sparse first page (relaxed queries): crawl page 2 for depth.
            if usable_count < 3 && !search_url.contains("page=") {
                let page2 = format!("{search_url}&page=2");
                if let Ok(more) = self.client.fetch(&page2, source).await {
                    if !looks_like_login_wall(&more.content) && !is_document_content(&more.content)
                    {
                        blocks.extend(parse_search_page(&more.content, source));
                    }
                }
            }
            let usable: Vec<&SearchListing> = blocks
                .iter()
                .filter(|block| block.in_stock && !block.offers.is_empty())
                .collect();
            // Genuine platform rows first; "Other Suppliers" rows (priced by
            // third parties, flagged for reconfirmation) only when the
            // platform has no genuine quotable row.
            let genuine: Vec<&SearchListing> = usable
                .iter()
                .filter(|block| !block.reconfirmation_required)
                .cloned()
                .collect();
            let chosen_rows: Vec<&SearchListing> =
                if genuine.is_empty() { usable } else { genuine };
            let mut from_search = 0usize;
            for block in chosen_rows
                .into_iter()
                .take(self.client.config().max_listings_per_source)
            {
                let Some(url) = block.listing_url.clone() else {
                    continue;
                };
                let quotable = quotable_price(&block.offers, target_quantity, preferred_currency);
                listings.push(ListingQuote {
                    listing_url: url,
                    source: source.as_str().to_string(),
                    offers: block.offers.clone(),
                    quotable,
                    reconfirmation_required: block.reconfirmation_required,
                });
                from_search += 1;
            }

            // Fallback: when the search page carried no usable rows, crawl
            // listing pages directly (guarded against walls, zero stock and
            // PDF/document responses).
            if from_search == 0 {
                let mut links = extract_listing_links(&content.content, source);
                if links.is_empty() {
                    links = self
                        .baidu_discovery(part_number, source)
                        .await
                        .unwrap_or_default();
                }
                for url in links
                    .into_iter()
                    .take(self.client.config().max_listings_per_source)
                {
                    match self.client.fetch(&url, source).await {
                        Ok(content) => {
                            if looks_like_login_wall(&content.content)
                                || indicates_out_of_stock(&content.content)
                                || is_document_content(&content.content)
                            {
                                continue;
                            }
                            let offers = parse_price_offers(&content.content);
                            if offers.is_empty() {
                                continue;
                            }
                            let quotable =
                                quotable_price(&offers, target_quantity, preferred_currency);
                            listings.push(ListingQuote {
                                listing_url: url,
                                source: source.as_str().to_string(),
                                offers,
                                quotable,
                                reconfirmation_required: false,
                            });
                        }
                        Err(SupplierError::CircuitOpen { .. }) => break,
                        Err(_) => continue,
                    }
                }
            }
        }
        lowest_quote(
            &listings,
            target_quantity,
            preferred_currency,
            self.client.config().fx_cny_usd,
        )
        .ok_or(SupplierError::NoListings {
            query: part_number.to_string(),
            source_name: "marketplaces",
        })
    }

    async fn search_links(
        &self,
        query: &str,
        source: PricingSource,
    ) -> Result<Vec<String>, SupplierError> {
        let url = source.search_url(query);
        let content = self.client.fetch(&url, source).await?;
        Ok(extract_listing_links(&content.content, source))
    }

    async fn baidu_discovery(
        &self,
        query: &str,
        target: PricingSource,
    ) -> Result<Vec<String>, SupplierError> {
        let url = target.baidu_discovery_url(query);
        let content = self.client.fetch(&url, PricingSource::Baidu).await?;
        Ok(extract_listing_links(&content.content, target))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic observation id (stable per part/source/listing/price)
// ─────────────────────────────────────────────────────────────────────────────

pub fn quote_observation_id(
    part_number: &str,
    listing_url: &str,
    unit_price: f64,
    currency: &str,
) -> uuid::Uuid {
    let composite = format!("{part_number}|{listing_url}|{unit_price:.6}|{currency}");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, composite.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offers_json(values: &[(&str, f64, u64, Option<u64>, bool)]) -> Vec<PriceOffer> {
        values
            .iter()
            .map(|(currency, price, moq, max, teaser)| PriceOffer {
                unit_price: *price,
                currency: currency.to_string(),
                moq: *moq,
                max_qty: *max,
                teaser: *teaser,
                price_max: None,
                raw: String::new(),
            })
            .collect()
    }

    #[test]
    fn tier_lines_parse_with_moq_and_max() {
        let markdown = "US $0.42\n1-99 pieces: US $0.42\n100-999: US $0.08\n>=1000: US $0.05";
        let offers = parse_price_offers(markdown);
        let tiers: Vec<_> = offers.iter().filter(|o| !o.teaser).collect();
        assert!(tiers
            .iter()
            .any(|o| o.moq == 1 && o.max_qty == Some(99) && (o.unit_price - 0.42).abs() < 1e-9));
        assert!(tiers.iter().any(|o| o.moq == 100 && o.max_qty == Some(999)));
    }

    #[test]
    fn from_prices_are_teasers() {
        let offers = parse_price_offers("From US $0.05\n$0.42");
        assert!(offers
            .iter()
            .any(|o| o.teaser && (o.unit_price - 0.05).abs() < 1e-9));
        assert!(offers
            .iter()
            .any(|o| !o.teaser && (o.unit_price - 0.42).abs() < 1e-9));
    }

    #[test]
    fn quotable_price_requires_a_covering_non_teaser_tier() {
        let teaser_only = offers_json(&[("USD", 0.05, 1, None, true)]);
        assert!(quotable_price(&teaser_only, 100, None).is_none());

        let tiers = offers_json(&[
            ("USD", 0.42, 1, Some(99), false),
            ("USD", 0.08, 100, Some(999), false),
            ("USD", 0.05, 1000, Some(4999), false),
        ]);
        let quote = quotable_price(&tiers, 100, None).expect("tier covers 100");
        assert!((quote.unit_price - 0.08).abs() < 1e-9);

        // Requesting above the largest bounded tier fails closed.
        assert!(quotable_price(&tiers, 5000, None).is_none());
    }

    #[test]
    fn lowest_quote_picks_minimum_covering_price() {
        let listings = vec![
            ListingQuote {
                listing_url: "a".into(),
                source: "alibaba".into(),
                offers: offers_json(&[("USD", 0.50, 1, Some(1000), false)]),
                quotable: None,
                reconfirmation_required: false,
            },
            ListingQuote {
                listing_url: "b".into(),
                source: "1688".into(),
                offers: offers_json(&[
                    ("USD", 0.42, 1, Some(99), false),
                    ("USD", 0.07, 100, Some(999), false),
                ]),
                quotable: None,
                reconfirmation_required: false,
            },
            ListingQuote {
                listing_url: "c".into(),
                source: "lcsc".into(),
                offers: offers_json(&[("USD", 0.05, 1, None, true)]),
                quotable: None,
                reconfirmation_required: false,
            },
        ];
        let quote =
            lowest_quote(&listings, 100, Some("USD"), Some(0.1398)).expect("quotable exists");
        assert_eq!(quote.listing_url, "b");
        assert!((quote.unit_price - 0.07).abs() < 1e-9);
        assert_eq!(quote.teaser_only_listings, 1);
    }

    #[test]
    fn retry_classification_is_exact() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(
            classify_retry(
                429,
                &headers,
                Duration::from_secs(1),
                Duration::from_secs(30)
            ),
            RetryAction::Retry(Duration::from_secs(7))
        );
        assert!(matches!(
            classify_retry(
                500,
                &HeaderMap::new(),
                Duration::from_secs(1),
                Duration::from_secs(30)
            ),
            RetryAction::Retry(_)
        ));
        for permanent in [400u16, 401, 402, 403, 404] {
            assert_eq!(
                classify_retry(
                    permanent,
                    &HeaderMap::new(),
                    Duration::from_secs(1),
                    Duration::from_secs(30)
                ),
                RetryAction::Fail,
                "status {permanent} must not retry"
            );
        }
    }

    #[test]
    fn backoff_is_bounded_and_grows() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(30);
        let first = backoff_for_attempt(0, base, max);
        let second = backoff_for_attempt(1, base, max);
        let late = backoff_for_attempt(10, base, max);
        assert!(first <= base + base / 2 + Duration::from_millis(1));
        assert!(second >= first);
        assert!(late <= max + Duration::from_millis(2));
    }

    #[test]
    fn listing_extraction_filters_by_source() {
        let markdown = r#"
[great](https://www.alibaba.com/product-detail/abc.html)
[nope](https://example.com/other)
[1688](https://detail.1688.com/offer/123.html)
https://www.lcsc.com/product-detail/C12345.html
[junk](https://www.alibaba.com/trade/search?SearchText=x)
"#;
        let alibaba = extract_listing_links(markdown, PricingSource::Alibaba);
        assert_eq!(alibaba.len(), 1);
        assert!(alibaba[0].contains("product-detail"));
        let lcsc = extract_listing_links(markdown, PricingSource::Lcsc);
        assert_eq!(lcsc.len(), 1);
    }

    #[test]
    fn search_urls_are_encoded() {
        let url = PricingSource::Alibaba.search_url("GRM155R71C104K 10µF");
        assert!(url.contains("GRM155R71C104K%2010%C2%B5F"));
        assert!(url.starts_with("https://www.alibaba.com/trade/search?"));
    }

    #[tokio::test]
    async fn fetch_routes_through_the_reader_with_retry_and_honours_retry_after() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let config = SupplierPricingConfig {
            reader_base_url: server.uri(),
            max_retries: 2,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(20),
            min_request_interval: Duration::ZERO,
            circuit_failure_threshold: 10,
            ..Default::default()
        };
        let client = SupplierPricingClient::new(config).expect("client");

        Mock::given(method("GET"))
            .and(path_regex(".*alibaba\\.com/product-detail/abc"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(".*alibaba\\.com/product-detail/abc"))
            .respond_with(ResponseTemplate::new(200).set_body_string("# offer\n1-99: US $0.42"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let content = client
            .fetch(
                "https://www.alibaba.com/product-detail/abc.html",
                PricingSource::Alibaba,
            )
            .await
            .expect("fetch succeeds after 429");
        assert_eq!(content.attempts, 2);
        assert!(content.content.contains("US $0.42"));
        assert!(!content.served_from_cache);
    }

    #[tokio::test]
    async fn fetch_caches_and_skips_second_reader_call() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let config = SupplierPricingConfig {
            reader_base_url: server.uri(),
            min_request_interval: Duration::ZERO,
            ..Default::default()
        };
        let client = SupplierPricingClient::new(config).expect("client");

        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("100+$0.07 product-detail cached body"),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let first = client
            .fetch(
                "https://www.lcsc.com/product-detail/C1.html",
                PricingSource::Lcsc,
            )
            .await
            .expect("first");
        assert!(!first.served_from_cache);
        let second = client
            .fetch(
                "https://www.lcsc.com/product-detail/C1.html",
                PricingSource::Lcsc,
            )
            .await
            .expect("second");
        assert!(second.served_from_cache);
        assert_eq!(second.content, "100+$0.07 product-detail cached body");
    }

    #[tokio::test]
    async fn permanent_refusal_never_retries() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let config = SupplierPricingConfig {
            reader_base_url: server.uri(),
            max_retries: 3,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(10),
            min_request_interval: Duration::ZERO,
            circuit_failure_threshold: 10,
            ..Default::default()
        };
        let client = SupplierPricingClient::new(config).expect("client");

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;

        let error = client
            .fetch(
                "https://www.alibaba.com/product-detail/blocked.html",
                PricingSource::Alibaba,
            )
            .await
            .expect_err("403 must fail immediately");
        assert!(matches!(
            error,
            SupplierError::ReaderRefused { status: 403, .. }
        ));
    }

    #[test]
    fn table_rows_and_step_breaks_parse_as_tiers() {
        let markdown = "| Quantity | Price |\n| --- | --- |\n| 1 - 99 | US $0.42 |\n| 100 - 999 | US $0.08 |\n| 1000+ | US $0.05 |";
        let offers = parse_price_offers(markdown);
        assert!(offers
            .iter()
            .any(|o| o.moq == 1 && o.max_qty == Some(99) && (o.unit_price - 0.42).abs() < 1e-9));
        assert!(offers
            .iter()
            .any(|o| o.moq == 100 && o.max_qty == Some(999)));
        assert!(offers
            .iter()
            .any(|o| o.moq == 1000 && o.max_qty.is_none() && !o.teaser));
    }

    #[test]
    fn cny_forms_parse_as_cny() {
        let offers = parse_price_offers("1-99: ¥0.31\n100+: CN¥0.28\n0.25元");
        assert!(offers
            .iter()
            .any(|o| o.currency == "CNY" && (o.unit_price - 0.31).abs() < 1e-9));
        assert!(offers
            .iter()
            .any(|o| o.currency == "CNY" && (o.unit_price - 0.28).abs() < 1e-9 && o.moq == 100));
        assert!(offers
            .iter()
            .any(|o| o.currency == "CNY" && (o.unit_price - 0.25).abs() < 1e-9));
    }

    #[test]
    fn fx_normalization_ranks_across_currencies() {
        // 0.28 CNY ≈ 0.0391 USD beats a 0.08 USD offer.
        let listings = vec![
            ListingQuote {
                listing_url: "usd".into(),
                source: "alibaba".into(),
                offers: offers_json(&[("USD", 0.08, 100, Some(999), false)]),
                quotable: None,
                reconfirmation_required: false,
            },
            ListingQuote {
                listing_url: "cny".into(),
                source: "1688".into(),
                offers: offers_json(&[("CNY", 0.28, 100, Some(999), false)]),
                quotable: None,
                reconfirmation_required: false,
            },
        ];
        let quote = lowest_quote(&listings, 100, Some("USD"), Some(0.1398)).expect("quote");
        assert_eq!(quote.listing_url, "cny");
        assert!(quote.fx_applied);
        assert_eq!(quote.currency, "CNY");
        assert!((quote.median_unit_usd.unwrap() - 0.08).abs() < 1e-6);
    }

    #[test]
    fn without_fx_cross_currency_fails_closed() {
        let listings = vec![ListingQuote {
            listing_url: "cny".into(),
            source: "1688".into(),
            offers: offers_json(&[("CNY", 0.28, 100, Some(999), false)]),
            quotable: None,
            reconfirmation_required: false,
        }];
        // Preferred currency USD, no FX: the CNY-only offer is not comparable.
        assert!(lowest_quote(&listings, 100, Some("USD"), None).is_none());
    }

    #[test]
    fn outlier_prices_are_rejected_with_enough_samples() {
        let make = |url: &str, price: f64| ListingQuote {
            listing_url: url.into(),
            source: "lcsc".into(),
            offers: offers_json(&[("USD", price, 100, Some(999), false)]),
            quotable: None,
            reconfirmation_required: false,
        };
        let listings = vec![
            make("a", 0.005), // outlier: 25x below the median
            make("b", 0.12),
            make("c", 0.13),
            make("d", 0.14),
        ];
        let quote = lowest_quote(&listings, 100, Some("USD"), Some(0.1398)).expect("quote");
        assert_eq!(quote.listing_url, "b");
        assert!(!quote.outlier_risk);
        assert_eq!(quote.sample_size, 4);
    }

    #[test]
    fn negotiated_ranges_are_never_quotable() {
        let offers = parse_price_offers("US $0.01 - US $0.05");
        assert!(offers
            .iter()
            .any(|o| o.price_max.is_some() && (o.unit_price - 0.01).abs() < 1e-9));
        assert!(offers
            .iter()
            .any(|o| o.price_max.is_some() && (o.price_max.unwrap() - 0.05).abs() < 1e-9));
        // A range on its own cannot be quoted.
        assert!(quotable_price(&offers, 100, None).is_none());
    }

    #[test]
    fn promo_lines_are_demoted_to_teasers() {
        let offers = parse_price_offers("Promo price: 100+$0.0052");
        assert!(offers
            .iter()
            .any(|o| o.teaser && (o.unit_price - 0.0052).abs() < 1e-9));
        assert!(quotable_price(&offers, 100, None).is_none());
    }

    #[test]
    fn temporary_block_expiry_is_parsed() {
        let when = chrono::Utc::now() + chrono::Duration::hours(1);
        let when_text = when.format("%a %b %d %Y %H:%M:%S GMT+0000").to_string();
        let body = format!(
            "{{\"data\":null,\"code\":403,\"message\":\"Anonymous access blocked until {when_text}\"}}"
        );
        let until = parse_block_until(&body).expect("blocked-until parsed");
        assert!(until > chrono::Utc::now());
    }

    #[test]
    fn login_walls_and_zero_stock_are_detected() {
        assert!(looks_like_login_wall(
            "# Sign in\nPlease sign in to continue"
        ));
        assert!(!looks_like_login_wall(
            "# STM32F103C8T6\nUS $0.42 1-99 pieces\nsome longer content"
        ));
        assert!(indicates_out_of_stock("1-99: US $0.42\nIn Stock: 0"));
        assert!(!indicates_out_of_stock("1-99: US $0.42\nIn Stock: 1234"));
    }

    #[tokio::test]
    async fn circuit_breaker_opens_after_consecutive_failures() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let config = SupplierPricingConfig {
            reader_base_url: server.uri(),
            max_retries: 0,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(5),
            min_request_interval: Duration::ZERO,
            circuit_failure_threshold: 2,
            circuit_cooldown: Duration::from_secs(60),
            ..Default::default()
        };
        let client = SupplierPricingClient::new(config).expect("client");

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;

        let _ = client
            .fetch(
                "https://www.lcsc.com/product-detail/C2.html",
                PricingSource::Lcsc,
            )
            .await;
        let _ = client
            .fetch(
                "https://www.lcsc.com/product-detail/C3.html",
                PricingSource::Lcsc,
            )
            .await;
        let third = client
            .fetch(
                "https://www.lcsc.com/product-detail/C4.html",
                PricingSource::Lcsc,
            )
            .await;
        assert!(matches!(
            third,
            Err(SupplierError::CircuitOpen {
                source_name: "lcsc",
                ..
            })
        ));
    }
}
