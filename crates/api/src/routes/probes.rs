//! Capability probes that measure real behaviour rather than configuration.
//!
//! Every probe here answers "can this deployment actually do the thing?":
//!
//! * **LLM** — contacts the configured endpoint (`/health`, falling back to a
//!   1-token completion for OpenAI-compatible APIs without a health route).
//! * **Browser** — runs a cached self-test that launches the renderer, loads a
//!   `data:` fixture, executes its inline JavaScript and reads the resulting
//!   DOM back over CDP.
//! * **Search index** — compares the database observation high-water mark with
//!   the index's last committed checkpoint; an empty or lagging index is never
//!   `ok`.
//! * **Embeddings** — generates a canary vector, stores it, retrieves it by
//!   nearest neighbour, and deletes it; column presence proves nothing.
//! * **Alert engine / outbox / scheduled jobs / source coverage** — read the
//!   worker-published operational state and apply configurable policy
//!   thresholds.
//!
//! Network- and renderer-backed probes are cached for a short TTL so frequently
//! polled health endpoints stay cheap without reporting stale health.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use apex_core::config::ConfigErrors;
use apex_crawl::sources_registry::SourceCoverageSummary;
use apex_store::postgres::{
    AlertEngineStateRow, NotificationDeliveryHealth, OutboxBacklog, WorkerJobStateRecord,
};
use apex_store::tantivy_index::IndexCheckpoint;

use super::capabilities::CapabilityStatus;

/// Entity type under which embedding canary rows are stored and removed.
pub const CANARY_ENTITY_TYPE: &str = "readiness_canary";

/// Minimum cosine similarity for a canary to count as a successful
/// nearest-neighbour round trip.
pub const CANARY_MIN_SIMILARITY: f64 = 0.99;

// ─────────────────────────────────────────────────────────────────────────────
// Policy (published in readiness responses; configurable thresholds)
// ─────────────────────────────────────────────────────────────────────────────

/// Configurable thresholds and probe budgets backing the readiness contract.
///
/// Defaults are production-sane; every field can be overridden with an
/// `APEX_*` environment variable so deployments can tune policy without a
/// rebuild. The resolved policy is published in `/api/health/ready` responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessPolicy {
    /// TTL for the cached LLM endpoint probe.
    pub llm_probe_ttl_secs: u64,
    /// Per-request timeout for the LLM endpoint probe.
    pub llm_probe_timeout_secs: u64,
    /// TTL for the cached browser render self-test.
    pub browser_probe_ttl_secs: u64,
    /// Wall-clock cap for one browser render self-test.
    pub browser_probe_timeout_secs: u64,
    /// TTL for the cached embedding round-trip canary.
    pub embedding_canary_ttl_secs: u64,
    /// Wall-clock cap for one embedding round trip.
    pub embedding_canary_timeout_secs: u64,
    /// Maximum allowed lag between the database high-water mark and the
    /// search index's last committed checkpoint.
    pub search_index_max_lag_secs: i64,
    /// Maximum age of the newest observation before crawl freshness degrades.
    pub crawl_freshness_max_age_secs: i64,
    /// Multidimensional capability-family coverage matrix. Replaces the old
    /// single `min_operational_sources` gate: a required family with zero
    /// operational sources fails readiness even when the deployment-wide
    /// total is high.
    #[serde(default)]
    pub coverage: apex_crawl::coverage::CoveragePolicy,
    /// Maximum age of the worker-refreshed alert-engine state row.
    pub alert_engine_max_age_secs: i64,
    /// Maximum unpublished outbox events before the publisher degrades.
    pub outbox_max_pending: i64,
    /// Maximum age of the oldest unpublished outbox event.
    pub outbox_max_oldest_pending_secs: i64,
    /// Maximum age of the notification retry processor's last run.
    pub notification_delivery_max_processor_age_secs: i64,
    /// Maximum due-but-unclaimed channel deliveries before readiness degrades.
    pub notification_delivery_max_overdue: i64,
    /// Maximum age of the oldest overdue channel delivery.
    pub notification_delivery_max_overdue_age_secs: i64,
    /// Maximum dead-lettered channel deliveries awaiting operator replay.
    pub notification_delivery_max_dead_lettered: i64,
    /// Maximum new dead letters inside the success window (change rate).
    pub notification_delivery_max_dead_letter_rate: i64,
    /// Maximum channel deliveries stuck in `delivering` with an expired lease.
    pub notification_delivery_max_stuck_leases: i64,
    /// Recent window used for the delivery success ratio and dead-letter rate.
    pub notification_delivery_success_window_secs: i64,
    /// Minimum recent attempts before the success ratio is enforced.
    pub notification_delivery_min_recent_attempts: i64,
    /// Minimum successful-attempt percentage inside the window.
    pub notification_delivery_min_success_percent: i64,
    /// Scheduled jobs whose freshness gates readiness.
    pub critical_jobs: Vec<String>,
    /// Maximum age of a critical job's last run.
    pub critical_job_max_age_secs: i64,
}

pub const DEFAULT_CRITICAL_JOBS: &[&str] = &["crawl_cycle", "triage_processing"];

impl Default for ReadinessPolicy {
    fn default() -> Self {
        Self {
            llm_probe_ttl_secs: 60,
            llm_probe_timeout_secs: 5,
            browser_probe_ttl_secs: 300,
            browser_probe_timeout_secs: 30,
            embedding_canary_ttl_secs: 300,
            embedding_canary_timeout_secs: 10,
            search_index_max_lag_secs: 3_600,
            crawl_freshness_max_age_secs: crate::system_status::DATA_FRESH_WITHIN_SECS,
            coverage: apex_crawl::coverage::CoveragePolicy::default(),
            alert_engine_max_age_secs: 900,
            outbox_max_pending: 100,
            outbox_max_oldest_pending_secs: 300,
            notification_delivery_max_processor_age_secs: 900,
            notification_delivery_max_overdue: 250,
            notification_delivery_max_overdue_age_secs: 600,
            notification_delivery_max_dead_lettered: 25,
            notification_delivery_max_dead_letter_rate: 10,
            notification_delivery_max_stuck_leases: 0,
            notification_delivery_success_window_secs: 3_600,
            notification_delivery_min_recent_attempts: 5,
            notification_delivery_min_success_percent: 90,
            critical_jobs: DEFAULT_CRITICAL_JOBS
                .iter()
                .map(|job| (*job).to_string())
                .collect(),
            critical_job_max_age_secs: 7_200,
        }
    }
}

impl ReadinessPolicy {
    /// Resolve the policy from the environment.
    ///
    /// Unset variables keep their default. A present-but-malformed threshold
    /// is a configuration error instead of a silent fallback: readiness must
    /// not report a verdict computed from a value the operator never chose.
    /// All errors are collected so one startup pass reports every problem.
    pub fn from_env() -> std::result::Result<Self, ConfigErrors> {
        let defaults = Self::default();
        let mut errors = ConfigErrors::new();
        let coverage =
            apex_crawl::coverage::CoveragePolicy::from_env().unwrap_or_else(|coverage_errors| {
                errors.extend(coverage_errors);
                apex_crawl::coverage::CoveragePolicy::default()
            });
        let policy = Self {
            llm_probe_ttl_secs: env_u64(
                "APEX_LLM_PROBE_TTL_SECS",
                defaults.llm_probe_ttl_secs,
                &mut errors,
            ),
            llm_probe_timeout_secs: env_u64(
                "APEX_LLM_PROBE_TIMEOUT_SECS",
                defaults.llm_probe_timeout_secs,
                &mut errors,
            ),
            browser_probe_ttl_secs: env_u64(
                "APEX_BROWSER_PROBE_TTL_SECS",
                defaults.browser_probe_ttl_secs,
                &mut errors,
            ),
            browser_probe_timeout_secs: env_u64(
                "APEX_BROWSER_PROBE_TIMEOUT_SECS",
                defaults.browser_probe_timeout_secs,
                &mut errors,
            ),
            embedding_canary_ttl_secs: env_u64(
                "APEX_EMBEDDING_CANARY_TTL_SECS",
                defaults.embedding_canary_ttl_secs,
                &mut errors,
            ),
            embedding_canary_timeout_secs: env_u64(
                "APEX_EMBEDDING_CANARY_TIMEOUT_SECS",
                defaults.embedding_canary_timeout_secs,
                &mut errors,
            ),
            search_index_max_lag_secs: env_i64(
                "APEX_SEARCH_INDEX_MAX_LAG_SECS",
                defaults.search_index_max_lag_secs,
                &mut errors,
            ),
            crawl_freshness_max_age_secs: env_i64(
                "APEX_CRAWL_FRESHNESS_MAX_AGE_SECS",
                defaults.crawl_freshness_max_age_secs,
                &mut errors,
            ),
            coverage,
            alert_engine_max_age_secs: env_i64(
                "APEX_ALERT_ENGINE_MAX_AGE_SECS",
                defaults.alert_engine_max_age_secs,
                &mut errors,
            ),
            outbox_max_pending: env_i64(
                "APEX_OUTBOX_MAX_PENDING",
                defaults.outbox_max_pending,
                &mut errors,
            ),
            outbox_max_oldest_pending_secs: env_i64(
                "APEX_OUTBOX_MAX_OLDEST_PENDING_SECS",
                defaults.outbox_max_oldest_pending_secs,
                &mut errors,
            ),
            notification_delivery_max_processor_age_secs: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_PROCESSOR_AGE_SECS",
                defaults.notification_delivery_max_processor_age_secs,
                &mut errors,
            ),
            notification_delivery_max_overdue: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_OVERDUE",
                defaults.notification_delivery_max_overdue,
                &mut errors,
            ),
            notification_delivery_max_overdue_age_secs: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_OVERDUE_AGE_SECS",
                defaults.notification_delivery_max_overdue_age_secs,
                &mut errors,
            ),
            notification_delivery_max_dead_lettered: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_DEAD_LETTERED",
                defaults.notification_delivery_max_dead_lettered,
                &mut errors,
            ),
            notification_delivery_max_dead_letter_rate: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_DEAD_LETTER_RATE",
                defaults.notification_delivery_max_dead_letter_rate,
                &mut errors,
            ),
            notification_delivery_max_stuck_leases: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MAX_STUCK_LEASES",
                defaults.notification_delivery_max_stuck_leases,
                &mut errors,
            ),
            notification_delivery_success_window_secs: env_i64(
                "APEX_NOTIFICATION_DELIVERY_SUCCESS_WINDOW_SECS",
                defaults.notification_delivery_success_window_secs,
                &mut errors,
            ),
            notification_delivery_min_recent_attempts: env_i64(
                "APEX_NOTIFICATION_DELIVERY_MIN_RECENT_ATTEMPTS",
                defaults.notification_delivery_min_recent_attempts,
                &mut errors,
            ),
            notification_delivery_min_success_percent: env_percentage(
                "APEX_NOTIFICATION_DELIVERY_MIN_SUCCESS_PERCENT",
                defaults.notification_delivery_min_success_percent,
                &mut errors,
            ),
            critical_jobs: env_csv("APEX_CRITICAL_JOBS", &defaults.critical_jobs, &mut errors),
            critical_job_max_age_secs: env_i64(
                "APEX_CRITICAL_JOB_MAX_AGE_SECS",
                defaults.critical_job_max_age_secs,
                &mut errors,
            ),
        };
        errors.into_result()?;
        Ok(policy)
    }

    pub fn llm_probe_ttl(&self) -> Duration {
        Duration::from_secs(self.llm_probe_ttl_secs)
    }

    pub fn llm_probe_timeout(&self) -> Duration {
        Duration::from_secs(self.llm_probe_timeout_secs.max(1))
    }

    pub fn browser_probe_ttl(&self) -> Duration {
        Duration::from_secs(self.browser_probe_ttl_secs)
    }

    pub fn browser_probe_timeout(&self) -> Duration {
        Duration::from_secs(self.browser_probe_timeout_secs.max(1))
    }

    pub fn embedding_canary_ttl(&self) -> Duration {
        Duration::from_secs(self.embedding_canary_ttl_secs)
    }

    pub fn embedding_canary_timeout(&self) -> Duration {
        Duration::from_secs(self.embedding_canary_timeout_secs.max(1))
    }
}

fn env_u64(name: &str, default: u64, errors: &mut ConfigErrors) -> u64 {
    match std::env::var(name) {
        // A present-but-blank value is treated as unset: placeholder lines
        // like `APEX_OUTBOX_MAX_PENDING=` must not abort startup.
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(value) => value,
            Err(_) => {
                errors.push(name, raw.trim(), "a non-negative integer");
                default
            }
        },
    }
}

fn env_i64(name: &str, default: i64, errors: &mut ConfigErrors) -> i64 {
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                errors.push(name, raw.trim(), "an integer");
                default
            }
        },
    }
}

fn env_percentage(name: &str, default: i64, errors: &mut ConfigErrors) -> i64 {
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(value) if (0..=100).contains(&value) => value,
            Ok(_) => {
                errors.push(name, raw.trim(), "an integer percentage in 0..=100");
                default
            }
            Err(_) => {
                errors.push(name, raw.trim(), "an integer percentage in 0..=100");
                default
            }
        },
    }
}

fn env_csv(name: &str, default: &[String], errors: &mut ConfigErrors) -> Vec<String> {
    match std::env::var(name) {
        Ok(raw) => {
            let parsed: Vec<String> = raw
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            if !parsed.is_empty() {
                parsed
            } else {
                // Blank means unset; a non-blank value with no names (e.g.
                // ",") is a real configuration error.
                if !raw.trim().is_empty() {
                    errors.push(name, raw.trim(), "at least one job name");
                }
                default.to_vec()
            }
        }
        Err(_) => default.to_vec(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Short-TTL probe cache
// ─────────────────────────────────────────────────────────────────────────────

struct CachedProbe {
    status: CapabilityStatus,
    recorded_at: Instant,
}

fn probe_cache() -> &'static Mutex<HashMap<&'static str, CachedProbe>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, CachedProbe>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_probe(name: &'static str, ttl: Duration) -> Option<CapabilityStatus> {
    let cache = match probe_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache
        .get(name)
        .filter(|entry| entry.recorded_at.elapsed() < ttl)
        .map(|entry| entry.status.clone())
}

fn store_probe(name: &'static str, status: &CapabilityStatus) {
    let mut cache = match probe_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache.insert(
        name,
        CachedProbe {
            status: status.clone(),
            recorded_at: Instant::now(),
        },
    );
}

/// Drop every cached probe result. Used by tests and available to operators
/// who need an immediate re-measurement after a fix.
pub fn reset_probe_cache() {
    let mut cache = match probe_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache.clear();
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM endpoint probe
// ─────────────────────────────────────────────────────────────────────────────

/// Endpoint details for the LLM probe, resolved from the runtime model config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LlmProbeTarget {
    pub base_url: String,
    /// Full chat-completions URL (`{base}/v1/chat/completions`).
    pub chat_endpoint: String,
    pub model: String,
    pub timeout_secs: u64,
}

/// Contact the configured LLM endpoint. `/health` is preferred (cheap and
/// side-effect free); an OpenAI-style API without a health route is probed
/// with a single-token completion so "reachable but wrong model" still fails.
pub async fn probe_llm_endpoint(target: &LlmProbeTarget) -> CapabilityStatus {
    let timeout = Duration::from_secs(target.timeout_secs.max(1));
    // The configured endpoint is trusted infrastructure, but its *responses*
    // are not: refuse redirects so a 3xx can never steer this server into
    // fetching arbitrary internal URLs (SSRF), and 307/308 cannot replay the
    // completion POST to a redirect target.
    // Operator-configured LLM probe endpoint, not crawled content.
    #[allow(clippy::disallowed_methods)]
    let client = match reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("could not build LLM probe client: {error}"),
            )
        }
    };

    let health_url = format!("{}/health", target.base_url.trim_end_matches('/'));
    let started = Instant::now();
    match client.get(&health_url).send().await {
        Ok(response) if response.status().is_success() => CapabilityStatus::new(
            "ok",
            format!(
                "{} /health 200 in {}ms (model {})",
                target.base_url,
                started.elapsed().as_millis(),
                target.model
            ),
        ),
        Ok(response)
            if response.status() == reqwest::StatusCode::NOT_FOUND
                || response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED =>
        {
            probe_llm_completion(&client, target).await
        }
        Ok(response) => CapabilityStatus::new(
            "degraded",
            format!(
                "{} /health returned HTTP {}",
                target.base_url,
                response.status()
            ),
        ),
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!("{} unreachable: {error}", target.base_url),
        ),
    }
}

async fn probe_llm_completion(
    client: &reqwest::Client,
    target: &LlmProbeTarget,
) -> CapabilityStatus {
    let body = serde_json::json!({
        "model": target.model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
        "temperature": 0.0,
    });
    let started = Instant::now();
    match client.post(&target.chat_endpoint).json(&body).send().await {
        Ok(response) if response.status().is_success() => CapabilityStatus::new(
            "ok",
            format!(
                "model {} answered a 1-token completion in {}ms (no /health route)",
                target.model,
                started.elapsed().as_millis()
            ),
        ),
        Ok(response) => CapabilityStatus::new(
            "degraded",
            format!(
                "model {} completion probe returned HTTP {}",
                target.model,
                response.status()
            ),
        ),
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!(
                "completion probe to {} failed: {error}",
                target.chat_endpoint
            ),
        ),
    }
}

/// Cached LLM capability probe. `target == None` means the build has the LLM
/// feature but no model configured — a deployment that cannot prove inference.
pub async fn probe_llm(
    target: Option<&LlmProbeTarget>,
    policy: &ReadinessPolicy,
) -> CapabilityStatus {
    if !cfg!(feature = "llm") {
        return CapabilityStatus::new("disabled", "binary built without the llm feature");
    }
    if let Some(cached) = cached_probe("llm", policy.llm_probe_ttl()) {
        return cached;
    }
    let status = match target {
        Some(target) => probe_llm_endpoint(target).await,
        None => CapabilityStatus::not_configured(
            "llm feature compiled but no LLM model configured (set LLM_MODEL/LLM_BASE_URL)",
        ),
    };
    store_probe("llm", &status);
    status
}

// ─────────────────────────────────────────────────────────────────────────────
// Browser render self-test
// ─────────────────────────────────────────────────────────────────────────────

/// One render self-test over the real renderer.
#[async_trait]
pub trait BrowserSelfTest: Send + Sync {
    async fn self_test(&self) -> anyhow::Result<apex_crawl::browser::BrowserSelfTestReport>;
}

#[async_trait]
impl BrowserSelfTest for apex_crawl::browser::PersistentChromiumBrowser {
    async fn self_test(&self) -> anyhow::Result<apex_crawl::browser::BrowserSelfTestReport> {
        apex_crawl::browser::PersistentChromiumBrowser::self_test(self).await
    }
}

/// How the renderer is wired in this process.
#[derive(Clone)]
pub enum BrowserProbeState {
    /// `ENABLE_HEADLESS_BROWSER` is not truthy.
    Disabled,
    /// Enabled but the renderer could not be constructed (bad config/binary).
    Unavailable(String),
    /// A live renderer that can be self-tested.
    Ready(std::sync::Arc<dyn BrowserSelfTest>),
}

impl BrowserProbeState {
    pub fn from_env() -> Self {
        let enabled = std::env::var("ENABLE_HEADLESS_BROWSER")
            .map(|value| apex_core::env::parse_truthy_flag(&value))
            .unwrap_or(false);
        if !enabled {
            return Self::Disabled;
        }
        match apex_crawl::browser::from_env() {
            Ok(Some(browser)) => Self::Ready(std::sync::Arc::new(browser)),
            Ok(None) => Self::Unavailable(
                "ENABLE_HEADLESS_BROWSER is set but the renderer factory returned none".to_string(),
            ),
            Err(error) => Self::Unavailable(format!("renderer configuration rejected: {error}")),
        }
    }

    pub fn as_probe(&self) -> Option<&dyn BrowserSelfTest> {
        match self {
            Self::Ready(probe) => Some(probe.as_ref()),
            Self::Disabled | Self::Unavailable(_) => None,
        }
    }
}

/// Cached browser capability probe: a real render of an inline JS fixture whose
/// DOM marker can only appear if JavaScript executed.
pub async fn probe_browser_renderer(
    state: &BrowserProbeState,
    policy: &ReadinessPolicy,
) -> CapabilityStatus {
    match state {
        BrowserProbeState::Disabled => {
            return CapabilityStatus::new(
                "disabled",
                "headless browser disabled (ENABLE_HEADLESS_BROWSER is not truthy)",
            )
        }
        BrowserProbeState::Unavailable(reason) => {
            return CapabilityStatus::new(
                "degraded",
                format!("ENABLE_HEADLESS_BROWSER is set but the renderer is unavailable: {reason}"),
            )
        }
        BrowserProbeState::Ready(_) => {}
    }

    if let Some(cached) = cached_probe("browser_renderer", policy.browser_probe_ttl()) {
        return cached;
    }

    let Some(probe) = state.as_probe() else {
        let status = CapabilityStatus::new("degraded", "no browser renderer available to test");
        return status;
    };

    let status = match tokio::time::timeout(policy.browser_probe_timeout(), probe.self_test()).await
    {
        Ok(Ok(report)) => CapabilityStatus::new(
            "ok",
            format!(
                "rendered data: fixture; JS marker '{}' verified in DOM in {}ms",
                report.marker, report.elapsed_ms
            ),
        ),
        Ok(Err(error)) => CapabilityStatus::new(
            "degraded",
            format!("browser render self-test failed: {error}"),
        ),
        Err(_) => CapabilityStatus::new(
            "degraded",
            format!(
                "browser render self-test timed out after {}s",
                policy.browser_probe_timeout_secs.max(1)
            ),
        ),
    };
    store_probe("browser_renderer", &status);
    status
}

// ─────────────────────────────────────────────────────────────────────────────
// Search index lag
// ─────────────────────────────────────────────────────────────────────────────

/// Pure evaluation of index health from measured values.
///
/// * empty index → `unavailable` (never `ok`),
/// * no recorded commit → `unavailable`,
/// * commit high-water mark older than `max_lag_secs` behind the database →
///   `degraded`,
/// * otherwise `ok`, exposing the lag and last successful commit.
pub fn evaluate_search_index(
    docs: u64,
    checkpoint: Option<&IndexCheckpoint>,
    db_high_water: Option<DateTime<Utc>>,
    max_lag_secs: i64,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    if docs == 0 {
        return CapabilityStatus::new(
            "unavailable",
            "search index is empty: 0 documents indexed (no successful commit recorded)",
        );
    }

    let Some(checkpoint) = checkpoint else {
        return CapabilityStatus::new(
            "unavailable",
            format!(
                "search index holds {docs} documents but has never recorded a successful commit"
            ),
        );
    };

    let mut status = match db_high_water {
        None => CapabilityStatus::new(
            "ok",
            format!(
                "{docs} documents indexed; no observations to index yet (last commit {})",
                checkpoint.last_commit_at.to_rfc3339()
            ),
        ),
        Some(high_water) => match checkpoint.high_water_ts {
            None => CapabilityStatus::new(
                "degraded",
                format!(
                    "{docs} documents indexed but the index has never indexed an observation \
                     (database high-water {})",
                    high_water.to_rfc3339()
                ),
            ),
            Some(indexed_high_water) => {
                let lag_secs = (high_water - indexed_high_water).num_seconds().max(0);
                if lag_secs > max_lag_secs {
                    CapabilityStatus::new(
                        "degraded",
                        format!(
                            "index lag {lag_secs}s exceeds policy {max_lag_secs}s \
                             (database high-water {}, index high-water {})",
                            high_water.to_rfc3339(),
                            indexed_high_water.to_rfc3339()
                        ),
                    )
                } else {
                    CapabilityStatus::new(
                        "ok",
                        format!(
                            "{docs} documents indexed; lag {lag_secs}s (policy {max_lag_secs}s), \
                             last commit {}",
                            checkpoint.last_commit_at.to_rfc3339()
                        ),
                    )
                }
            }
        },
    };

    status.last_seen_at = Some(checkpoint.last_commit_at.to_rfc3339());
    status.age_seconds = Some((now - checkpoint.last_commit_at).num_seconds().max(0));
    if let (Some(high_water), Some(indexed_high_water)) = (db_high_water, checkpoint.high_water_ts)
    {
        status.lag_seconds = Some((high_water - indexed_high_water).num_seconds().max(0));
    }
    status
}

// ─────────────────────────────────────────────────────────────────────────────
// Embedding canary round trip
// ─────────────────────────────────────────────────────────────────────────────

/// Generation half of the embedding canary.
#[async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    fn model_name(&self) -> &str;
    async fn generate(&self, text: &str) -> anyhow::Result<Vec<f64>>;
}

/// Storage + nearest-neighbour half of the embedding canary.
#[async_trait]
pub trait EmbeddingCanaryStore: Send + Sync {
    /// Remove canary rows left behind by a previous crashed, timed-out, or
    /// failed probe. Called before every round trip so leaks are bounded by
    /// the probe TTL instead of accumulating forever.
    async fn sweep_canaries(&self) -> anyhow::Result<u64>;
    async fn store_canary(
        &self,
        canary_id: &str,
        embedding: &[f64],
        model: &str,
    ) -> anyhow::Result<()>;
    async fn nearest(
        &self,
        embedding: &[f64],
        limit: usize,
    ) -> anyhow::Result<Vec<CanaryNeighbour>>;
    async fn delete_canary(&self, canary_id: &str) -> anyhow::Result<()>;
}

/// A nearest-neighbour search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct CanaryNeighbour {
    pub entity_type: String,
    pub entity_id: String,
    pub similarity: f64,
}

/// Evidence from a successful generation → storage → nearest-neighbour cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRoundTrip {
    pub dimension: usize,
    pub rank: usize,
    pub similarity: f64,
    pub elapsed_ms: u64,
}

/// Run the full embedding round trip and clean the canary row up afterwards.
///
/// A stale sweep runs first: if a previous attempt timed out between insert
/// and delete (or the delete failed), its row is removed here instead of
/// accumulating in the `embeddings` table.
pub async fn embedding_round_trip(
    generator: &dyn EmbeddingGenerator,
    store: &dyn EmbeddingCanaryStore,
) -> anyhow::Result<EmbeddingRoundTrip> {
    let started = Instant::now();
    store.sweep_canaries().await?;
    let canary_id = format!("readiness-canary-{}", uuid::Uuid::new_v4());
    let canary_text = format!("apex readiness embedding canary {canary_id}");

    let vector = generator.generate(&canary_text).await?;
    if vector.is_empty() {
        anyhow::bail!("embedding generator returned an empty vector");
    }

    store
        .store_canary(&canary_id, &vector, generator.model_name())
        .await?;
    let search = store.nearest(&vector, 5).await;
    let cleanup = store.delete_canary(&canary_id).await;
    let neighbours = search?;
    cleanup?;

    let rank = neighbours
        .iter()
        .position(|hit| hit.entity_type == CANARY_ENTITY_TYPE && hit.entity_id == canary_id)
        .map(|index| index + 1)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "stored canary was not returned by nearest-neighbour search ({} hits)",
                neighbours.len()
            )
        })?;
    let hit = &neighbours[rank - 1];
    if hit.similarity < CANARY_MIN_SIMILARITY {
        anyhow::bail!(
            "canary nearest-neighbour similarity {:.4} below required {:.2}",
            hit.similarity,
            CANARY_MIN_SIMILARITY
        );
    }

    Ok(EmbeddingRoundTrip {
        dimension: vector.len(),
        rank,
        similarity: hit.similarity,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Cached embedding capability probe. Without a generator the deployment
/// cannot prove generation, so the probe degrades instead of trusting the
/// `embeddings` table schema.
pub async fn probe_embeddings(
    generator: Option<&dyn EmbeddingGenerator>,
    store: &dyn EmbeddingCanaryStore,
    policy: &ReadinessPolicy,
) -> CapabilityStatus {
    let Some(generator) = generator else {
        return CapabilityStatus::new(
            "degraded",
            "no embedding generator configured; cannot prove generation/storage/round-trip",
        );
    };

    if let Some(cached) = cached_probe("embeddings", policy.embedding_canary_ttl()) {
        return cached;
    }

    let status = match tokio::time::timeout(
        policy.embedding_canary_timeout(),
        embedding_round_trip(generator, store),
    )
    .await
    {
        Ok(Ok(receipt)) => CapabilityStatus::new(
            "ok",
            format!(
                "canary {}d round-trip ok (rank {}, similarity {:.4}, {}ms, model {})",
                receipt.dimension,
                receipt.rank,
                receipt.similarity,
                receipt.elapsed_ms,
                generator.model_name()
            ),
        ),
        Ok(Err(error)) => CapabilityStatus::new(
            "degraded",
            format!("embedding generation/storage/nearest-neighbour round-trip failed: {error}"),
        ),
        Err(_) => CapabilityStatus::new(
            "degraded",
            format!(
                "embedding canary timed out after {}s",
                policy.embedding_canary_timeout_secs.max(1)
            ),
        ),
    };
    store_probe("embeddings", &status);
    status
}

/// The production embedding generator: the same `EmbeddingClient` the worker
/// and API use for real indexing.
#[cfg(feature = "llm")]
#[async_trait]
impl EmbeddingGenerator for apex_llm::embeddings::EmbeddingClient {
    fn model_name(&self) -> &str {
        apex_llm::embeddings::EmbeddingClient::model_name(self)
    }

    async fn generate(&self, text: &str) -> anyhow::Result<Vec<f64>> {
        Ok(self.embed(text).await?)
    }
}

#[async_trait]
impl EmbeddingCanaryStore for apex_store::postgres::PgStore {
    async fn sweep_canaries(&self) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM embeddings WHERE entity_type = $1")
            .bind(CANARY_ENTITY_TYPE)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn store_canary(
        &self,
        canary_id: &str,
        embedding: &[f64],
        model: &str,
    ) -> anyhow::Result<()> {
        self.upsert_embedding(
            CANARY_ENTITY_TYPE,
            canary_id,
            0,
            embedding,
            "readiness canary",
            model,
        )
        .await
    }

    async fn nearest(
        &self,
        embedding: &[f64],
        limit: usize,
    ) -> anyhow::Result<Vec<CanaryNeighbour>> {
        let hits = self.vector_search(embedding, limit).await?;
        Ok(hits
            .into_iter()
            .map(|hit| CanaryNeighbour {
                entity_type: hit.entity_type,
                entity_id: hit.entity_id,
                similarity: hit.similarity,
            })
            .collect())
    }

    async fn delete_canary(&self, canary_id: &str) -> anyhow::Result<()> {
        self.delete_entity_embeddings(CANARY_ENTITY_TYPE, canary_id)
            .await?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert engine state
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate the worker-published alert-engine state against policy.
pub fn evaluate_alert_engine(
    state: Option<&AlertEngineStateRow>,
    policy: &ReadinessPolicy,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    let Some(state) = state else {
        return CapabilityStatus::new(
            "unavailable",
            "no alert-engine state recorded; the worker has not reported a rules load",
        );
    };

    let age_secs = (now - state.updated_at).num_seconds().max(0);
    let hash = state.config_hash.as_deref().unwrap_or("");

    let mut status = if !state.last_reload_success {
        CapabilityStatus::new(
            "degraded",
            format!(
                "alert-rule reload failed: {}",
                state
                    .last_reload_error
                    .as_deref()
                    .unwrap_or("unknown error")
            ),
        )
    } else if state.rule_count <= 0 {
        CapabilityStatus::new("degraded", "alert engine loaded 0 rules")
    } else if hash.is_empty() {
        CapabilityStatus::new("degraded", "alert engine config hash missing")
    } else if age_secs > policy.alert_engine_max_age_secs {
        CapabilityStatus::new(
            "degraded",
            format!(
                "alert-engine state is {age_secs}s old (policy {}s); the worker is not refreshing it",
                policy.alert_engine_max_age_secs
            ),
        )
    } else {
        CapabilityStatus::new(
            "ok",
            format!(
                "{} rules loaded from {} (config {}…), refreshed {age_secs}s ago",
                state.rule_count,
                state.rules_path,
                &hash[..hash.len().min(12)]
            ),
        )
    };

    status.last_seen_at = Some(state.updated_at.to_rfc3339());
    status.age_seconds = Some(age_secs);
    status
}

// ─────────────────────────────────────────────────────────────────────────────
// Outbox publisher backlog
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate outbox backlog/publisher health against policy.
pub fn evaluate_outbox(
    backlog: Option<&OutboxBacklog>,
    policy: &ReadinessPolicy,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    let Some(backlog) = backlog else {
        return CapabilityStatus::new("unavailable", "outbox backlog query failed");
    };

    let oldest_age_secs = backlog
        .oldest_pending_at
        .map(|oldest| (now - oldest).num_seconds().max(0));
    let last_publish = backlog
        .last_published_at
        .map(|published| {
            format!(
                "last publish {} ago",
                crate::system_status::format_age(now - published)
            )
        })
        .unwrap_or_else(|| "nothing published yet".to_string());

    if backlog.exhausted > 0 {
        return CapabilityStatus::new(
            "degraded",
            format!(
                "{} outbox events exhausted publish attempts; {} pending, {last_publish}",
                backlog.exhausted, backlog.pending
            ),
        );
    }
    if backlog.pending > policy.outbox_max_pending {
        return CapabilityStatus::new(
            "degraded",
            format!(
                "{} pending outbox events exceed policy {}; {last_publish}",
                backlog.pending, policy.outbox_max_pending
            ),
        );
    }
    if let Some(age) = oldest_age_secs {
        if age > policy.outbox_max_oldest_pending_secs {
            return CapabilityStatus::new(
                "degraded",
                format!(
                    "oldest pending outbox event is {age}s old (policy {}s); {last_publish}",
                    policy.outbox_max_oldest_pending_secs
                ),
            );
        }
    }

    CapabilityStatus::new(
        "ok",
        format!("{} pending, 0 exhausted; {last_publish}", backlog.pending),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Durable notification delivery
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate durable channel delivery health against policy.
///
/// Five independent failure modes gate readiness, because each one can leave
/// the raw backlog looking harmless:
///
/// * the retry processor stopped running (stale `notification_delivery` job);
/// * due rows keep aging (`oldest_overdue_age_secs`) even if the count is low;
/// * dead letters accumulate (terminal count and recent change rate);
/// * a crash left rows `delivering` past their lease (stuck leases);
/// * attempts fail more often than the success-ratio policy allows.
pub fn evaluate_notification_delivery(
    health: Option<&NotificationDeliveryHealth>,
    processor: Option<&WorkerJobStateRecord>,
    policy: &ReadinessPolicy,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    let Some(health) = health else {
        return CapabilityStatus::new("unavailable", "notification delivery health query failed");
    };

    let mut problems: Vec<String> = Vec::new();

    let processor_age_secs = match processor {
        None => {
            problems.push(
                "retry processor has never recorded a run; channel deliveries are not attempted"
                    .to_string(),
            );
            None
        }
        Some(state) => {
            if state.circuit_open {
                problems.push("retry processor circuit is open".to_string());
            }
            if state.consecutive_failures >= 3 {
                problems.push(format!(
                    "retry processor has {} consecutive failures",
                    state.consecutive_failures
                ));
            }
            match state.last_run {
                None => {
                    problems.push("retry processor has never run".to_string());
                    None
                }
                Some(last_run) => {
                    let age_secs = (now - last_run).num_seconds().max(0);
                    if age_secs > policy.notification_delivery_max_processor_age_secs {
                        problems.push(format!(
                            "retry processor last ran {age_secs}s ago (policy {}s)",
                            policy.notification_delivery_max_processor_age_secs
                        ));
                    }
                    Some(age_secs)
                }
            }
        }
    };

    if health.overdue > policy.notification_delivery_max_overdue {
        problems.push(format!(
            "{} overdue deliveries exceed policy {}",
            health.overdue, policy.notification_delivery_max_overdue
        ));
    }
    if let Some(oldest_age) = health.oldest_overdue_age_secs {
        if oldest_age > policy.notification_delivery_max_overdue_age_secs {
            problems.push(format!(
                "oldest overdue delivery is {oldest_age}s old (policy {}s)",
                policy.notification_delivery_max_overdue_age_secs
            ));
        }
    }
    if health.dead_lettered > policy.notification_delivery_max_dead_lettered {
        problems.push(format!(
            "{} dead-lettered deliveries await operator replay (policy {})",
            health.dead_lettered, policy.notification_delivery_max_dead_lettered
        ));
    }
    if health.dead_lettered_recent > policy.notification_delivery_max_dead_letter_rate {
        problems.push(format!(
            "{} deliveries dead-lettered in the last {}s (policy rate {})",
            health.dead_lettered_recent,
            policy.notification_delivery_success_window_secs,
            policy.notification_delivery_max_dead_letter_rate
        ));
    }
    if health.stuck_delivering > policy.notification_delivery_max_stuck_leases {
        problems.push(format!(
            "{} deliveries are stuck 'delivering' past their lease (policy {})",
            health.stuck_delivering, policy.notification_delivery_max_stuck_leases
        ));
    }
    // Success ratio over terminal outcomes in the window: a delivery that
    // succeeded after transient retries counts as a success, and an empty
    // window (nothing concluded yet) is not a failure. `min_recent_attempts`
    // keeps a tiny sample from gating readiness.
    let terminal_recent = health.delivered_recent + health.failed_recent;
    if terminal_recent >= policy.notification_delivery_min_recent_attempts {
        let success_percent = health.delivered_recent * 100 / terminal_recent;
        if success_percent < policy.notification_delivery_min_success_percent {
            problems.push(format!(
                "delivery success ratio {success_percent}% ({}/{} deliveries concluded in the last {}s) below policy {}%",
                health.delivered_recent,
                terminal_recent,
                policy.notification_delivery_success_window_secs,
                policy.notification_delivery_min_success_percent
            ));
        }
    }

    let mut status = if problems.is_empty() {
        CapabilityStatus::new(
            "ok",
            format!(
                "{} pending, {} overdue, {} dead-lettered, {} stuck; {}/{} deliveries concluded in the window succeeded",
                health.pending,
                health.overdue,
                health.dead_lettered,
                health.stuck_delivering,
                health.delivered_recent,
                terminal_recent
            ),
        )
    } else {
        CapabilityStatus::new(
            "degraded",
            format!("delivery health: {}", problems.join("; ")),
        )
    };
    status.age_seconds = processor_age_secs;
    status.last_seen_at = processor
        .and_then(|state| state.last_run)
        .map(|last_run| last_run.to_rfc3339());
    status
}

// ─────────────────────────────────────────────────────────────────────────────
// Critical scheduled jobs
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate freshness of the configured critical scheduled jobs.
pub fn evaluate_scheduled_jobs(
    states: &[WorkerJobStateRecord],
    policy: &ReadinessPolicy,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    if states.is_empty() {
        return CapabilityStatus::new(
            "unavailable",
            "no scheduled job state recorded; the worker has never persisted a run",
        );
    }

    let mut problems: Vec<String> = Vec::new();
    let mut fresh: Vec<String> = Vec::new();
    for job in &policy.critical_jobs {
        let Some(state) = states.iter().find(|state| &state.job_kind == job) else {
            problems.push(format!("{job}: no state recorded"));
            continue;
        };
        let Some(last_run) = state.last_run else {
            problems.push(format!("{job}: never ran"));
            continue;
        };
        let age_secs = (now - last_run).num_seconds().max(0);
        if state.circuit_open {
            problems.push(format!("{job}: circuit open"));
        } else if state.consecutive_failures >= 3 {
            problems.push(format!(
                "{job}: {} consecutive failures",
                state.consecutive_failures
            ));
        } else if age_secs > policy.critical_job_max_age_secs {
            problems.push(format!(
                "{job}: last run {age_secs}s ago exceeds {}s",
                policy.critical_job_max_age_secs
            ));
        } else {
            fresh.push(format!(
                "{job} {}",
                crate::system_status::format_age(now - last_run)
            ));
        }
    }

    if !problems.is_empty() {
        return CapabilityStatus::new(
            "degraded",
            format!("critical job freshness: {}", problems.join("; ")),
        );
    }

    CapabilityStatus::new(
        "ok",
        format!("{} critical jobs fresh ({})", fresh.len(), fresh.join(", ")),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Operational source coverage
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate the multidimensional source-coverage matrix.
///
/// The single deployment-wide minimum is gone: every required capability
/// family must meet its own operational-source count, freshness, fetch and
/// parser ratios and independent-domain minimum, and priority companies must
/// be covered by recent observations. The full matrix — policy rows plus
/// measured dimensions — is published on the capability status so the UI and
/// operators can see which dimension failed.
pub fn evaluate_source_coverage(
    summary: &SourceCoverageSummary,
    policy: &ReadinessPolicy,
    priority_companies: apex_crawl::coverage::PriorityCompanyCoverage,
    now: DateTime<Utc>,
) -> CapabilityStatus {
    let report =
        apex_crawl::coverage::evaluate_coverage(summary, &policy.coverage, priority_companies, now);
    CapabilityStatus::new(&report.status, report.detail.clone()).with_coverage(report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::routes::capabilities::CapabilityState;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use chrono::TimeZone;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap()
    }

    /// Serialises tests that share the process-global probe cache.
    async fn cache_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        LOCK.lock().await
    }

    /// Serialises tests that mutate the process environment for `from_env`.
    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn invalid_readiness_threshold_fails_loudly_and_names_the_variable() {
        let _guard = env_test_lock();
        std::env::set_var("APEX_OUTBOX_MAX_PENDING", "not-a-number");
        let error = ReadinessPolicy::from_env()
            .expect_err("a malformed readiness threshold must fail loudly")
            .to_string();
        std::env::remove_var("APEX_OUTBOX_MAX_PENDING");

        assert!(
            error.contains("APEX_OUTBOX_MAX_PENDING"),
            "the error must name the variable: {error}"
        );
        assert!(
            error.contains("not-a-number"),
            "the error must name the bad value: {error}"
        );
    }

    #[test]
    fn blank_readiness_threshold_falls_back_to_default() {
        let _guard = env_test_lock();
        std::env::set_var("APEX_OUTBOX_MAX_PENDING", "   ");
        let policy = ReadinessPolicy::from_env().expect("a blank threshold is treated as unset");
        std::env::remove_var("APEX_OUTBOX_MAX_PENDING");

        assert_eq!(
            policy.outbox_max_pending,
            ReadinessPolicy::default().outbox_max_pending
        );
    }

    #[test]
    fn unset_readiness_thresholds_keep_their_defaults() {
        let _guard = env_test_lock();
        std::env::remove_var("APEX_OUTBOX_MAX_PENDING");
        let policy = ReadinessPolicy::from_env().expect("unset thresholds use defaults");
        assert_eq!(
            policy.outbox_max_pending,
            ReadinessPolicy::default().outbox_max_pending
        );
    }

    #[test]
    fn invalid_critical_jobs_csv_fails_loudly() {
        let _guard = env_test_lock();
        std::env::set_var("APEX_CRITICAL_JOBS", ",");
        let error = ReadinessPolicy::from_env()
            .expect_err("an empty job list must fail loudly")
            .to_string();
        std::env::remove_var("APEX_CRITICAL_JOBS");

        assert!(error.contains("APEX_CRITICAL_JOBS"), "{error}");
    }

    fn sample_job(
        kind: &str,
        last_run: Option<DateTime<Utc>>,
        consecutive_failures: i32,
        circuit_open: bool,
    ) -> WorkerJobStateRecord {
        WorkerJobStateRecord {
            job_kind: kind.to_string(),
            last_run,
            last_status: Some("succeeded".to_string()),
            last_error: None,
            last_duration_ms: Some(42),
            consecutive_failures,
            max_consecutive_failures: 5,
            circuit_open,
            updated_at: now(),
        }
    }

    fn sample_alert_state(
        success: bool,
        rule_count: i32,
        hash: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> AlertEngineStateRow {
        AlertEngineStateRow {
            id: "default".to_string(),
            rules_path: "config/runtime/alert-rules.yaml".to_string(),
            rule_count,
            config_hash: hash.map(str::to_string),
            last_reload_success: success,
            last_reload_error: if success {
                None
            } else {
                Some("yaml parse error".to_string())
            },
            last_reload_at: Some(updated_at),
            updated_at,
        }
    }

    // ── LLM ─────────────────────────────────────────────────────────────────

    fn closed_port_base_url() -> String {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral listener for probe");
        let addr = listener.local_addr().expect("listener addr");
        drop(listener);
        format!("http://{addr}")
    }

    async fn spawn_router(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe server");
        let addr = listener.local_addr().expect("probe server addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    fn llm_target(base_url: String) -> LlmProbeTarget {
        LlmProbeTarget {
            chat_endpoint: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            base_url,
            model: "probe-model".to_string(),
            timeout_secs: 2,
        }
    }

    #[tokio::test]
    async fn dead_llm_endpoint_is_not_ok() {
        let target = llm_target(closed_port_base_url());
        let status = probe_llm_endpoint(&target).await;
        assert_ne!(status.status, "ok", "dead endpoint must not report ok");
        assert_eq!(status.status, "unavailable");
    }

    #[tokio::test]
    async fn healthy_llm_health_route_is_ok() {
        let base_url = spawn_router(Router::new().route("/health", get(|| async { "ok" }))).await;
        let status = probe_llm_endpoint(&llm_target(base_url)).await;
        assert_eq!(status.status, "ok", "detail: {}", status.detail);
    }

    #[tokio::test]
    async fn openai_style_endpoint_without_health_uses_tiny_completion() {
        let router = Router::new()
            .route(
                "/health",
                get(|| async { (axum::http::StatusCode::NOT_FOUND, "") }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async { Json(json!({"choices": [{"message": {"content": "pong"}}]})) }),
            );
        let base_url = spawn_router(router).await;
        let status = probe_llm_endpoint(&llm_target(base_url)).await;
        assert_eq!(status.status, "ok", "detail: {}", status.detail);
        assert!(status.detail.contains("1-token completion"));
    }

    #[tokio::test]
    async fn llm_endpoint_returning_error_is_degraded() {
        let base_url = spawn_router(Router::new().route(
            "/health",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "") }),
        ))
        .await;
        let status = probe_llm_endpoint(&llm_target(base_url)).await;
        assert_eq!(status.status, "degraded");
    }

    #[tokio::test]
    async fn llm_probe_does_not_follow_redirects() {
        let internal_hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&internal_hits);
        let internal_url = spawn_router(Router::new().route(
            "/internal-metadata",
            get(move || {
                let counter = Arc::clone(&counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "secret"
                }
            }),
        ))
        .await;

        let redirect_to = format!("{internal_url}/internal-metadata");
        let base_url = spawn_router(Router::new().route(
            "/health",
            get(move || {
                let location = redirect_to.clone();
                async move {
                    (
                        axum::http::StatusCode::FOUND,
                        [(axum::http::header::LOCATION, location)],
                    )
                }
            }),
        ))
        .await;

        let status = probe_llm_endpoint(&llm_target(base_url)).await;
        assert_ne!(status.status, "ok", "a redirect must not count as healthy");
        assert_eq!(
            internal_hits.load(Ordering::SeqCst),
            0,
            "the probe must never follow a redirect to another host"
        );
    }

    #[cfg(feature = "llm")]
    #[tokio::test]
    async fn llm_probe_is_cached_within_ttl() {
        let _guard = cache_test_lock().await;
        reset_probe_cache();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let base_url = spawn_router(Router::new().route(
            "/health",
            get(move || {
                let counter = Arc::clone(&counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "ok"
                }
            }),
        ))
        .await;
        let policy = ReadinessPolicy {
            llm_probe_ttl_secs: 3_600,
            ..ReadinessPolicy::default()
        };
        let target = llm_target(base_url);

        let first = probe_llm(Some(&target), &policy).await;
        let second = probe_llm(Some(&target), &policy).await;
        assert_eq!(first.status, "ok");
        assert_eq!(second.status, "ok");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "second call must be cached");

        reset_probe_cache();
        let _ = probe_llm(Some(&target), &policy).await;
        assert_eq!(hits.load(Ordering::SeqCst), 2, "reset forces re-measure");
        reset_probe_cache();
    }

    #[tokio::test]
    async fn llm_probe_without_target_is_not_ok() {
        let _guard = cache_test_lock().await;
        reset_probe_cache();
        let status = probe_llm(None, &ReadinessPolicy::default()).await;
        assert_ne!(status.status, "ok");
        if cfg!(feature = "llm") {
            // The llm feature is compiled in but no model is configured:
            // that is a configuration gap, not a measured degradation.
            assert_eq!(status.status, "not_configured");
            assert_eq!(status.state(), CapabilityState::NotConfigured);
        } else {
            assert_eq!(status.status, "disabled");
            assert_eq!(status.state(), CapabilityState::Disabled);
        }
        reset_probe_cache();
    }

    // ── Browser ─────────────────────────────────────────────────────────────

    struct FakeBrowser {
        result: Result<apex_crawl::browser::BrowserSelfTestReport, String>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl BrowserSelfTest for FakeBrowser {
        async fn self_test(&self) -> anyhow::Result<apex_crawl::browser::BrowserSelfTestReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.result {
                Ok(report) => Ok(report.clone()),
                Err(error) => Err(anyhow::anyhow!(error.clone())),
            }
        }
    }

    fn report() -> apex_crawl::browser::BrowserSelfTestReport {
        apex_crawl::browser::BrowserSelfTestReport {
            marker: "rendered-42".to_string(),
            js_executed: true,
            dom_verified: true,
            elapsed_ms: 12,
            final_url: "data:text/html,fixture".to_string(),
        }
    }

    #[tokio::test]
    async fn browser_probe_fails_when_render_self_test_fails() {
        let _guard = cache_test_lock().await;
        reset_probe_cache();
        let state = BrowserProbeState::Ready(Arc::new(FakeBrowser {
            result: Err("chromium exploded".to_string()),
            calls: Arc::new(AtomicUsize::new(0)),
        }));
        let status = probe_browser_renderer(&state, &ReadinessPolicy::default()).await;
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("chromium exploded"));
        reset_probe_cache();
    }

    #[tokio::test]
    async fn browser_probe_fails_when_renderer_cannot_be_constructed() {
        let state = BrowserProbeState::Unavailable("bad config".to_string());
        let status = probe_browser_renderer(&state, &ReadinessPolicy::default()).await;
        assert_eq!(status.status, "degraded");
    }

    #[tokio::test]
    async fn browser_probe_is_disabled_when_env_flag_is_off() {
        let status =
            probe_browser_renderer(&BrowserProbeState::Disabled, &ReadinessPolicy::default()).await;
        assert_eq!(status.status, "disabled");
    }

    #[tokio::test]
    async fn browser_probe_caches_a_successful_render() {
        let _guard = cache_test_lock().await;
        reset_probe_cache();
        let calls = Arc::new(AtomicUsize::new(0));
        let state = BrowserProbeState::Ready(Arc::new(FakeBrowser {
            result: Ok(report()),
            calls: Arc::clone(&calls),
        }));
        let policy = ReadinessPolicy {
            browser_probe_ttl_secs: 3_600,
            ..ReadinessPolicy::default()
        };

        let first = probe_browser_renderer(&state, &policy).await;
        let second = probe_browser_renderer(&state, &policy).await;
        assert_eq!(first.status, "ok");
        assert_eq!(second.status, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        reset_probe_cache();
    }

    // ── Search index ────────────────────────────────────────────────────────

    #[test]
    fn empty_search_index_is_never_ok() {
        let status = evaluate_search_index(0, None, Some(now()), 3_600, now());
        assert_eq!(status.status, "unavailable");
        assert!(status.detail.contains("empty"));
    }

    #[test]
    fn uncommitted_search_index_is_not_ok() {
        let status = evaluate_search_index(12, None, Some(now()), 3_600, now());
        assert_eq!(status.status, "unavailable");
    }

    #[test]
    fn lagging_search_index_degrades_beyond_policy() {
        let checkpoint = IndexCheckpoint {
            last_commit_at: now() - chrono::Duration::minutes(1),
            high_water_ts: Some(now() - chrono::Duration::hours(3)),
            high_water_id: None,
            indexed_documents: 12,
        };
        let status = evaluate_search_index(12, Some(&checkpoint), Some(now()), 3_600, now());
        assert_eq!(status.status, "degraded");
        assert_eq!(status.lag_seconds, Some(3 * 3_600));
        assert!(status.detail.contains("exceeds policy"));
    }

    #[test]
    fn fresh_search_index_reports_lag_and_last_commit() {
        let commit_at = now() - chrono::Duration::minutes(5);
        let checkpoint = IndexCheckpoint {
            last_commit_at: commit_at,
            high_water_ts: Some(now() - chrono::Duration::minutes(10)),
            high_water_id: None,
            indexed_documents: 42,
        };
        let status = evaluate_search_index(42, Some(&checkpoint), Some(now()), 3_600, now());
        assert_eq!(status.status, "ok");
        assert_eq!(status.lag_seconds, Some(600));
        let expected_last_seen = commit_at.to_rfc3339();
        assert_eq!(
            status.last_seen_at.as_deref(),
            Some(expected_last_seen.as_str())
        );
        assert_eq!(status.age_seconds, Some(300));
    }

    #[test]
    fn search_index_ok_when_database_has_no_observations() {
        let checkpoint = IndexCheckpoint {
            last_commit_at: now(),
            high_water_ts: None,
            high_water_id: None,
            indexed_documents: 3,
        };
        let status = evaluate_search_index(3, Some(&checkpoint), None, 3_600, now());
        assert_eq!(status.status, "ok");
    }

    // ── Embeddings ──────────────────────────────────────────────────────────

    struct FakeGenerator {
        vector: Result<Vec<f64>, String>,
    }

    #[async_trait]
    impl EmbeddingGenerator for FakeGenerator {
        fn model_name(&self) -> &str {
            "fake-embedding-model"
        }

        async fn generate(&self, _text: &str) -> anyhow::Result<Vec<f64>> {
            match &self.vector {
                Ok(vector) => Ok(vector.clone()),
                Err(error) => Err(anyhow::anyhow!(error.clone())),
            }
        }
    }

    #[derive(Default)]
    struct InMemoryCanaryStore {
        rows: Mutex<HashMap<String, Vec<f64>>>,
        /// When true, nearest() returns hits that never include the canary.
        drop_hits: bool,
    }

    fn cosine(a: &[f64], b: &[f64]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }

    #[async_trait]
    impl EmbeddingCanaryStore for InMemoryCanaryStore {
        async fn sweep_canaries(&self) -> anyhow::Result<u64> {
            let mut rows = self.rows.lock().expect("canary store lock");
            let removed = rows.len() as u64;
            rows.clear();
            Ok(removed)
        }

        async fn store_canary(
            &self,
            canary_id: &str,
            embedding: &[f64],
            _model: &str,
        ) -> anyhow::Result<()> {
            self.rows
                .lock()
                .expect("canary store lock")
                .insert(canary_id.to_string(), embedding.to_vec());
            Ok(())
        }

        async fn nearest(
            &self,
            embedding: &[f64],
            limit: usize,
        ) -> anyhow::Result<Vec<CanaryNeighbour>> {
            let rows = self.rows.lock().expect("canary store lock");
            if self.drop_hits {
                return Ok(Vec::new());
            }
            let mut hits: Vec<CanaryNeighbour> = rows
                .iter()
                .map(|(id, vector)| CanaryNeighbour {
                    entity_type: CANARY_ENTITY_TYPE.to_string(),
                    entity_id: id.clone(),
                    similarity: cosine(embedding, vector),
                })
                .collect();
            hits.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
            hits.truncate(limit);
            Ok(hits)
        }

        async fn delete_canary(&self, canary_id: &str) -> anyhow::Result<()> {
            self.rows
                .lock()
                .expect("canary store lock")
                .remove(canary_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn embedding_canary_fails_when_generator_is_dead() {
        let generator = FakeGenerator {
            vector: Err("connection refused".to_string()),
        };
        let store = InMemoryCanaryStore::default();
        let error = embedding_round_trip(&generator, &store)
            .await
            .expect_err("dead generator must fail the canary");
        assert!(error.to_string().contains("connection refused"));
    }

    #[tokio::test]
    async fn embedding_canary_fails_without_nearest_neighbour_hit() {
        let generator = FakeGenerator {
            vector: Ok(vec![1.0, 0.0, 0.0]),
        };
        let store = InMemoryCanaryStore {
            drop_hits: true,
            ..InMemoryCanaryStore::default()
        };
        let error = embedding_round_trip(&generator, &store)
            .await
            .expect_err("missing nearest-neighbour hit must fail the canary");
        assert!(error.to_string().contains("nearest-neighbour"));
    }

    #[tokio::test]
    async fn embedding_canary_round_trips_generated_vector() {
        let generator = FakeGenerator {
            vector: Ok(vec![0.25, 0.5, 0.75]),
        };
        let store = InMemoryCanaryStore::default();
        let receipt = embedding_round_trip(&generator, &store)
            .await
            .expect("round trip succeeds");
        assert_eq!(receipt.dimension, 3);
        assert_eq!(receipt.rank, 1);
        assert!(receipt.similarity > 0.99);
        assert!(
            store.rows.lock().expect("canary lock").is_empty(),
            "canary row must be cleaned up"
        );
    }

    #[tokio::test]
    async fn embedding_canary_sweeps_rows_left_by_a_previous_probe() {
        let generator = FakeGenerator {
            vector: Ok(vec![1.0, 0.0, 0.0]),
        };
        let store = InMemoryCanaryStore::default();
        store
            .rows
            .lock()
            .expect("canary lock")
            .insert("readiness-canary-stale".to_string(), vec![1.0, 0.0, 0.0]);

        let receipt = embedding_round_trip(&generator, &store)
            .await
            .expect("round trip succeeds");
        assert_eq!(
            receipt.rank, 1,
            "stale row must not shadow the fresh canary"
        );
        assert!(
            store.rows.lock().expect("canary lock").is_empty(),
            "stale canary rows must be swept"
        );
    }

    #[tokio::test]
    async fn embedding_probe_without_generator_is_not_ok() {
        let store = InMemoryCanaryStore::default();
        let status = probe_embeddings(None, &store, &ReadinessPolicy::default()).await;
        assert_eq!(status.status, "degraded");
    }

    // ── Alert engine ────────────────────────────────────────────────────────

    #[test]
    fn alert_engine_without_state_is_unavailable() {
        let status = evaluate_alert_engine(None, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "unavailable");
    }

    #[test]
    fn alert_engine_failed_reload_is_degraded() {
        let state = sample_alert_state(false, 0, None, now());
        let status = evaluate_alert_engine(Some(&state), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("yaml parse error"));
    }

    #[test]
    fn alert_engine_zero_rules_is_degraded() {
        let state = sample_alert_state(true, 0, Some("abc"), now());
        let status = evaluate_alert_engine(Some(&state), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("0 rules"));
    }

    #[test]
    fn alert_engine_stale_state_is_degraded() {
        let state = sample_alert_state(
            true,
            25,
            Some("deadbeef"),
            now() - chrono::Duration::hours(1),
        );
        let status = evaluate_alert_engine(Some(&state), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("not refreshing"));
    }

    #[test]
    fn alert_engine_fresh_state_is_ok() {
        let state = sample_alert_state(
            true,
            25,
            Some("deadbeefcafebabe"),
            now() - chrono::Duration::seconds(30),
        );
        let status = evaluate_alert_engine(Some(&state), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "ok");
        assert!(status.detail.contains("25 rules"));
        assert_eq!(status.age_seconds, Some(30));
    }

    // ── Outbox ──────────────────────────────────────────────────────────────

    #[test]
    fn outbox_missing_backlog_is_unavailable() {
        let status = evaluate_outbox(None, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "unavailable");
    }

    #[test]
    fn outbox_exhausted_events_are_degraded() {
        let backlog = OutboxBacklog {
            pending: 1,
            exhausted: 2,
            oldest_pending_at: Some(now() - chrono::Duration::minutes(1)),
            last_published_at: Some(now() - chrono::Duration::minutes(2)),
        };
        let status = evaluate_outbox(Some(&backlog), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("exhausted"));
    }

    #[test]
    fn outbox_backlog_beyond_policy_is_degraded() {
        let backlog = OutboxBacklog {
            pending: 500,
            exhausted: 0,
            oldest_pending_at: Some(now()),
            last_published_at: None,
        };
        let status = evaluate_outbox(Some(&backlog), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("exceed policy"));
    }

    #[test]
    fn outbox_oldest_pending_beyond_policy_is_degraded() {
        let backlog = OutboxBacklog {
            pending: 3,
            exhausted: 0,
            oldest_pending_at: Some(now() - chrono::Duration::hours(1)),
            last_published_at: None,
        };
        let status = evaluate_outbox(Some(&backlog), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("oldest pending"));
    }

    #[test]
    fn outbox_healthy_backlog_is_ok() {
        let backlog = OutboxBacklog {
            pending: 3,
            exhausted: 0,
            oldest_pending_at: Some(now() - chrono::Duration::seconds(5)),
            last_published_at: Some(now() - chrono::Duration::seconds(10)),
        };
        let status = evaluate_outbox(Some(&backlog), &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "ok");
    }

    // ── Durable notification delivery ───────────────────────────────────────

    fn delivery_health() -> NotificationDeliveryHealth {
        NotificationDeliveryHealth {
            pending: 0,
            overdue: 0,
            oldest_overdue_age_secs: None,
            dead_lettered: 0,
            dead_lettered_recent: 0,
            stuck_delivering: 0,
            attempts_recent: 10,
            delivered_recent: 10,
            failed_recent: 0,
        }
    }

    #[test]
    fn notification_delivery_missing_probe_data_is_unavailable() {
        let status = evaluate_notification_delivery(
            None,
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "unavailable");
    }

    #[test]
    fn notification_delivery_healthy_pipeline_is_ok() {
        let status = evaluate_notification_delivery(
            Some(&delivery_health()),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "ok");
        assert!(status
            .detail
            .contains("10/10 deliveries concluded in the window succeeded"));
        assert_eq!(status.age_seconds, Some(0));
    }

    #[test]
    fn notification_delivery_backlog_beyond_policy_is_degraded() {
        let health = NotificationDeliveryHealth {
            overdue: 500,
            oldest_overdue_age_secs: Some(120),
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&health),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("overdue deliveries exceed policy"));
    }

    #[test]
    fn notification_delivery_oldest_overdue_age_beyond_policy_is_degraded() {
        let health = NotificationDeliveryHealth {
            overdue: 1,
            oldest_overdue_age_secs: Some(3_600),
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&health),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("oldest overdue delivery"));
    }

    #[test]
    fn notification_delivery_dead_letters_beyond_policy_are_degraded() {
        let by_count = NotificationDeliveryHealth {
            dead_lettered: 100,
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&by_count),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("dead-lettered deliveries"));

        let by_rate = NotificationDeliveryHealth {
            dead_lettered: 1,
            dead_lettered_recent: 50,
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&by_rate),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("dead-lettered in the last"));
    }

    #[test]
    fn notification_delivery_stuck_leases_are_degraded() {
        let health = NotificationDeliveryHealth {
            stuck_delivering: 2,
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&health),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status
            .detail
            .contains("stuck 'delivering' past their lease"));
    }

    #[test]
    fn notification_delivery_stale_retry_processor_is_degraded() {
        let status = evaluate_notification_delivery(
            Some(&delivery_health()),
            Some(&sample_job(
                "notification_delivery",
                Some(now() - chrono::Duration::hours(2)),
                0,
                false,
            )),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("retry processor last ran"));

        let never = evaluate_notification_delivery(
            Some(&delivery_health()),
            None,
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(never.status, "degraded");
        assert!(never.detail.contains("never recorded a run"));
    }

    #[test]
    fn notification_delivery_low_success_ratio_is_degraded() {
        let failing = NotificationDeliveryHealth {
            attempts_recent: 10,
            delivered_recent: 5,
            failed_recent: 5,
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&failing),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("success ratio 50%"));

        // A small sample must not fail readiness on its own.
        let small_sample = NotificationDeliveryHealth {
            attempts_recent: 2,
            delivered_recent: 0,
            failed_recent: 2,
            ..delivery_health()
        };
        let status = evaluate_notification_delivery(
            Some(&small_sample),
            Some(&sample_job("notification_delivery", Some(now()), 0, false)),
            &ReadinessPolicy::default(),
            now(),
        );
        assert_eq!(status.status, "ok");
    }

    // ── Scheduled jobs ──────────────────────────────────────────────────────

    #[test]
    fn scheduled_jobs_without_state_is_unavailable() {
        let status = evaluate_scheduled_jobs(&[], &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "unavailable");
    }

    #[test]
    fn stale_critical_job_is_degraded() {
        let states = vec![
            sample_job(
                "crawl_cycle",
                Some(now() - chrono::Duration::hours(5)),
                0,
                false,
            ),
            sample_job(
                "triage_processing",
                Some(now() - chrono::Duration::minutes(2)),
                0,
                false,
            ),
        ];
        let status = evaluate_scheduled_jobs(&states, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("crawl_cycle"));
    }

    #[test]
    fn missing_critical_job_state_is_degraded() {
        let states = vec![sample_job(
            "triage_processing",
            Some(now() - chrono::Duration::minutes(2)),
            0,
            false,
        )];
        let status = evaluate_scheduled_jobs(&states, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("crawl_cycle: no state recorded"));
    }

    #[test]
    fn open_circuit_critical_job_is_degraded() {
        let states = vec![
            sample_job(
                "crawl_cycle",
                Some(now() - chrono::Duration::minutes(1)),
                0,
                true,
            ),
            sample_job(
                "triage_processing",
                Some(now() - chrono::Duration::minutes(1)),
                0,
                false,
            ),
        ];
        let status = evaluate_scheduled_jobs(&states, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("circuit open"));
    }

    #[test]
    fn fresh_critical_jobs_are_ok() {
        let states = vec![
            sample_job(
                "crawl_cycle",
                Some(now() - chrono::Duration::minutes(3)),
                0,
                false,
            ),
            sample_job(
                "triage_processing",
                Some(now() - chrono::Duration::seconds(45)),
                0,
                false,
            ),
        ];
        let status = evaluate_scheduled_jobs(&states, &ReadinessPolicy::default(), now());
        assert_eq!(status.status, "ok");
        assert!(status.detail.contains("2 critical jobs fresh"));
    }

    // ── Source coverage ─────────────────────────────────────────────────────

    fn family_snapshot(
        family: apex_crawl::coverage::CoverageFamily,
        operational: usize,
    ) -> apex_crawl::coverage::FamilyCoverage {
        apex_crawl::coverage::FamilyCoverage {
            family,
            declared: 5,
            registered: 5,
            operational,
            validated: operational,
            independent_domains: 5,
            attempted: 5,
            parser_success_pct: Some(100),
            fetch_success_pct: Some(100),
            latest_success_at: Some(now()),
            ..apex_crawl::coverage::FamilyCoverage::default()
        }
    }

    fn healthy_matrix() -> SourceCoverageSummary {
        SourceCoverageSummary {
            declared: 680,
            registered: 500,
            operational: 60,
            validated: 120,
            never_crawled: 300,
            temporarily_degraded: 10,
            families: apex_crawl::coverage::CoverageFamily::ALL
                .into_iter()
                .map(|family| family_snapshot(family, 5))
                .collect(),
            ..SourceCoverageSummary::default()
        }
    }

    #[test]
    fn source_coverage_required_family_with_zero_sources_is_degraded() {
        // 60 operational sources overall, but procurement has none: the
        // deployment-wide total must not mask the family gap.
        let mut summary = healthy_matrix();
        for entry in &mut summary.families {
            if entry.family == apex_crawl::coverage::CoverageFamily::Procurement {
                entry.operational = 0;
            }
        }
        let status = evaluate_source_coverage(
            &summary,
            &ReadinessPolicy::default(),
            apex_crawl::coverage::PriorityCompanyCoverage::default(),
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("procurement"));
        let report = status
            .coverage
            .as_ref()
            .expect("coverage matrix is published in the detail");
        let procurement = report
            .families
            .iter()
            .find(|row| row.family == apex_crawl::coverage::CoverageFamily::Procurement)
            .expect("procurement row published");
        assert_eq!(procurement.status, "degraded");
    }

    #[test]
    fn source_coverage_meeting_the_matrix_is_ok() {
        let status = evaluate_source_coverage(
            &healthy_matrix(),
            &ReadinessPolicy::default(),
            apex_crawl::coverage::PriorityCompanyCoverage {
                total: 20,
                covered: 15,
            },
            now(),
        );
        assert_eq!(status.status, "ok", "detail: {}", status.detail);
        let report = status.coverage.as_ref().expect("coverage report");
        assert_eq!(
            report.families.len(),
            apex_crawl::coverage::CoverageFamily::ALL.len()
        );
        assert_eq!(report.satisfied_required_families, report.required_families);
        assert_eq!(report.priority_company_pct, Some(75));
    }

    #[test]
    fn source_coverage_uncovered_priority_companies_degrade() {
        let status = evaluate_source_coverage(
            &healthy_matrix(),
            &ReadinessPolicy::default(),
            apex_crawl::coverage::PriorityCompanyCoverage {
                total: 20,
                covered: 1,
            },
            now(),
        );
        assert_eq!(status.status, "degraded");
        assert!(status.detail.contains("priority-company coverage"));
    }

    #[test]
    fn policy_defaults_are_serializable() {
        let policy = ReadinessPolicy::default();
        let json = serde_json::to_value(&policy).expect("policy serializes");
        assert_eq!(json["search_index_max_lag_secs"], 3_600);
        assert!(json["critical_jobs"]
            .as_array()
            .expect("critical_jobs array")
            .contains(&json!("crawl_cycle")));
        let families = json["coverage"]["families"]
            .as_array()
            .expect("coverage matrix is published as policy");
        assert_eq!(families.len(), 11);
        assert!(families.iter().any(|row| {
            row["family"] == json!("procurement") && row["min_operational_sources"] == json!(3)
        }));
    }

    #[test]
    fn malformed_readiness_threshold_is_a_configuration_error() {
        use std::sync::{LazyLock, Mutex};
        static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let name = "APEX_LLM_PROBE_TTL_SECS";
        std::env::set_var(name, "soon");
        let result = ReadinessPolicy::from_env();
        std::env::remove_var(name);

        let errors = result.expect_err("'soon' is not a TTL in seconds");
        assert!(
            errors.errors.iter().any(|error| error.variable == name),
            "the configuration error must name the malformed variable: {errors}"
        );
    }
}
