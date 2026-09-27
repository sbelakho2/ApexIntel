//! `/api/health/capabilities` — capability health derived from real probes.
//!
//! Every value is measured rather than assumed: the LLM entry contacts the
//! configured endpoint, the browser entry runs a real render self-test, the
//! search-index entry compares database and index high-water marks, the
//! embeddings entry runs a generation → storage → nearest-neighbour canary,
//! NATS/database/heartbeat/freshness are live probes, and the alert-engine,
//! outbox, source-coverage and scheduled-job entries read worker-published
//! operational state against configurable policy thresholds.

use std::time::Duration as StdDuration;

use apex_core::profile::DeploymentProfile;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use apex_store::postgres::{PgStore, SchemaLineage, ServiceHeartbeatRow};
use apex_store::tantivy_index::SearchIndex;

use super::probes::{self, BrowserProbeState, EmbeddingGenerator, LlmProbeTarget, ReadinessPolicy};
#[cfg(test)]
use crate::responses::aggregate_health;
use crate::responses::{ComponentHealth, HealthStatus};
use crate::system_status::{format_age, StatusStrip, WORKER_HEARTBEAT_STALE_AFTER_SECS};

pub const CAPABILITIES_PATH: &str = "/api/health/capabilities";

const NATS_PROBE_TIMEOUT: StdDuration = StdDuration::from_secs(2);

/// Typed measured state of one capability.
///
/// The generic conversion (JSON payloads, `/api/health` components, UI badges)
/// must never collapse [`CapabilityState::Disabled`] or
/// [`CapabilityState::NotConfigured`] into `Healthy`: only the
/// profile-specific readiness composition decides whether a `Disabled`
/// capability is acceptable for a given deployment profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    /// Measured healthy.
    Healthy,
    /// Measured but outside policy (stale, overloaded, below threshold).
    Degraded,
    /// Measured and missing/broken (connection refused, schema mismatch).
    Unavailable,
    /// Deliberately switched off (profile does not require it, feature flag).
    Disabled,
    /// Required but the deployment has no configuration for it.
    NotConfigured,
    /// No measurement exists for this capability in this response.
    NotMeasured,
}

impl CapabilityState {
    /// Map the persisted probe status string to the typed state. Unknown
    /// strings are `NotMeasured` — never assumed healthy.
    pub fn from_status(status: &str) -> Self {
        match status {
            "ok" | "healthy" => Self::Healthy,
            "degraded" => Self::Degraded,
            "unavailable" => Self::Unavailable,
            "disabled" => Self::Disabled,
            "not_configured" => Self::NotConfigured,
            _ => Self::NotMeasured,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::Disabled => "disabled",
            Self::NotConfigured => "not_configured",
            Self::NotMeasured => "not_measured",
        }
    }

    /// True only for a measured healthy capability.
    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }
}

/// One capability probe result.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CapabilityStatus {
    /// `ok` | `degraded` | `unavailable` | `disabled` | `not_configured` |
    /// `not_measured`.
    pub status: String,
    /// Typed form of `status`, published so the UI/API can render
    /// Disabled/NotConfigured distinctly from Healthy.
    pub state: CapabilityState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
    /// Search-index lag (database high-water minus indexed high-water).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lag_seconds: Option<i64>,
    /// Full multidimensional coverage report (source-coverage capability
    /// only): the published policy matrix plus every family's measured
    /// dimensions and the aggregate evidence-quality summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<apex_crawl::coverage::SourceCoverageReport>,
}

impl CapabilityStatus {
    pub fn new(status: &str, detail: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            state: CapabilityState::from_status(status),
            detail: detail.into(),
            last_seen_at: None,
            age_seconds: None,
            lag_seconds: None,
            coverage: None,
        }
    }

    /// A capability the deployment has not configured.
    pub fn not_configured(detail: impl Into<String>) -> Self {
        Self::new("not_configured", detail)
    }

    /// A capability with no measurement available in this response.
    pub fn not_measured(detail: impl Into<String>) -> Self {
        Self::new("not_measured", detail)
    }

    /// Typed state derived from the probe status.
    pub fn state(&self) -> CapabilityState {
        CapabilityState::from_status(&self.status)
    }

    /// Attach the published source-coverage matrix report.
    pub fn with_coverage(mut self, coverage: apex_crawl::coverage::SourceCoverageReport) -> Self {
        self.coverage = Some(coverage);
        self
    }

    pub fn is_ok(&self) -> bool {
        self.state().is_healthy()
    }
}

/// Inputs the capability plan needs beyond the database search index: the
/// resolved policy, the configured LLM endpoint, the browser renderer, and the
/// embedding generator.
pub struct ProbeContext<'a> {
    pub pool: &'a sqlx::PgPool,
    pub search_index: &'a SearchIndex,
    pub nats_url: Option<&'a str>,
    pub policy: &'a ReadinessPolicy,
    pub llm: Option<&'a LlmProbeTarget>,
    pub browser: &'a BrowserProbeState,
    pub embedding_generator: Option<&'a dyn EmbeddingGenerator>,
    /// Schema lineage already measured for this request. When set, the
    /// `schema` probe reuses it instead of querying again, so readiness never
    /// evaluates the lineage contract twice from different snapshots.
    pub schema_lineage: Option<&'a SchemaLineage>,
}

/// Full capability report.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Capabilities {
    pub llm: CapabilityStatus,
    pub embeddings: CapabilityStatus,
    pub nats: CapabilityStatus,
    pub browser_renderer: CapabilityStatus,
    pub database: CapabilityStatus,
    /// Schema lineage: applied migration head == embedded head and the
    /// applied history checksum-verifies.
    pub schema: CapabilityStatus,
    pub search_index: CapabilityStatus,
    pub worker_heartbeat: CapabilityStatus,
    pub crawl_freshness: CapabilityStatus,
    /// Minimum operational source coverage (full profile).
    pub source_coverage: CapabilityStatus,
    /// Alert-rule engine state published by the worker (full profile).
    pub alert_engine: CapabilityStatus,
    /// Outbox publisher backlog (full profile).
    pub outbox: CapabilityStatus,
    /// Durable channel delivery / retry processor health (full profile).
    pub notification_delivery: CapabilityStatus,
    /// Critical scheduled-job freshness (full profile).
    pub scheduled_jobs: CapabilityStatus,
}

/// Structured schema-lineage evidence published by `/api/health/ready`: both
/// migration heads plus the checksum verification result, so an operator can
/// see exactly which part of the lineage contract passed or failed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SchemaLineageReport {
    /// `ok` | `mismatch` | `unavailable`.
    pub status: String,
    /// Newest migration embedded in the running binary (expected head).
    pub expected_head: Option<i64>,
    /// Newest migration the database has applied.
    pub applied_head: Option<i64>,
    pub applied_count: i64,
    /// True when the applied history checksum-verifies against the embedded
    /// migrations (independent of head equality).
    pub checksums_verified: bool,
    pub detail: String,
}

impl SchemaLineageReport {
    /// Build the report from the measured lineage, or from the query error.
    pub fn from_result(result: Result<SchemaLineage, String>) -> Self {
        match result {
            Ok(lineage) => {
                let status = if lineage.is_current() {
                    "ok"
                } else {
                    "mismatch"
                };
                let detail = match &lineage.problem {
                    Some(problem) => problem.clone(),
                    None => format!(
                        "applied migration head {} matches embedded head {} ({} migrations checksum-verified)",
                        lineage.applied_head.unwrap_or_default(),
                        lineage.expected_head.unwrap_or_default(),
                        lineage.applied_count
                    ),
                };
                Self {
                    status: status.to_string(),
                    expected_head: lineage.expected_head,
                    applied_head: lineage.applied_head,
                    applied_count: lineage.applied_count,
                    checksums_verified: lineage.checksums_verified,
                    detail,
                }
            }
            Err(error) => Self {
                status: "unavailable".to_string(),
                expected_head: None,
                applied_head: None,
                applied_count: 0,
                checksums_verified: false,
                detail: error,
            },
        }
    }
}

/// The product surfaces the five root health endpoints expose, and from which
/// full product readiness is composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurface {
    Process,
    Data,
    Intelligence,
    Delivery,
}

impl ProductSurface {
    pub const ALL: [Self; 4] = [
        Self::Process,
        Self::Data,
        Self::Intelligence,
        Self::Delivery,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Data => "data",
            Self::Intelligence => "intelligence",
            Self::Delivery => "delivery",
        }
    }

    /// Capabilities that make up this surface.
    pub fn capabilities(self) -> &'static [&'static str] {
        match self {
            Self::Process => &["database", "schema", "worker_heartbeat", "scheduled_jobs"],
            Self::Data => &[
                "crawl_freshness",
                "source_coverage",
                "search_index",
                "browser_renderer",
            ],
            Self::Intelligence => &["llm", "embeddings", "alert_engine"],
            Self::Delivery => &["nats", "outbox", "notification_delivery"],
        }
    }
}

/// Measured health of one product surface.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceHealth {
    /// `process` | `data` | `intelligence` | `delivery`.
    pub surface: String,
    /// `ok` | `degraded` | `unhealthy`.
    pub status: String,
    pub checks: Vec<ComponentHealth>,
}

impl SurfaceHealth {
    pub fn from_checks(surface: ProductSurface, checks: Vec<ComponentHealth>) -> Self {
        let status = if checks
            .iter()
            .any(|check| check.status == HealthStatus::Unhealthy)
        {
            "unhealthy"
        } else if checks
            .iter()
            .any(|check| check.status == HealthStatus::Degraded)
        {
            "degraded"
        } else {
            "ok"
        };
        Self {
            surface: surface.as_str().to_string(),
            status: status.to_string(),
            checks,
        }
    }

    /// Surface endpoints answer 503 whenever any required check is not healthy.
    pub fn http_status(&self) -> StatusCode {
        if self.status == "ok" {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Profile-aware readiness response: the measured checks, the surfaces they
/// compose, and the required-capability set + thresholds published as policy.
/// `schema_lineage` publishes the exact migration-lineage proof (embedded head,
/// applied head, checksum verification) that full readiness requires.
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessReport {
    pub status: HealthStatus,
    pub version: String,
    pub uptime_secs: u64,
    pub profile: String,
    pub required_capabilities: Vec<String>,
    pub thresholds: ReadinessPolicy,
    pub schema_lineage: SchemaLineageReport,
    pub surfaces: Vec<SurfaceHealth>,
    pub checks: Vec<ComponentHealth>,
}

impl Capabilities {
    /// Worst-of status across all capabilities (`ok` only when every
    /// capability is measured healthy). Disabled and NotConfigured
    /// capabilities degrade the generic aggregate: the profile-specific
    /// readiness composition is the only place that may accept them.
    pub fn overall_status(&self) -> &'static str {
        let all = [
            &self.llm,
            &self.embeddings,
            &self.nats,
            &self.browser_renderer,
            &self.database,
            &self.schema,
            &self.search_index,
            &self.worker_heartbeat,
            &self.crawl_freshness,
            &self.source_coverage,
            &self.alert_engine,
            &self.outbox,
            &self.notification_delivery,
            &self.scheduled_jobs,
        ];
        if all
            .iter()
            .any(|c| c.state() == CapabilityState::Unavailable)
        {
            "unavailable"
        } else if all.iter().any(|c| !c.state().is_healthy()) {
            "degraded"
        } else {
            "ok"
        }
    }

    /// UI status strip derived from the same measured values.
    pub fn status_strip(&self) -> StatusStrip {
        StatusStrip::from_parts(
            self.database.is_ok(),
            self.worker_heartbeat
                .age_seconds
                .map(chrono::Duration::seconds),
            self.crawl_freshness
                .age_seconds
                .map(chrono::Duration::seconds),
        )
    }

    /// Component checks embedded in `/api/health`.
    pub fn health_checks(&self) -> Vec<ComponentHealth> {
        [
            ("database", &self.database),
            ("schema", &self.schema),
            ("llm", &self.llm),
            ("embeddings", &self.embeddings),
            ("nats", &self.nats),
            ("browser_renderer", &self.browser_renderer),
            ("search_index", &self.search_index),
            ("worker_heartbeat", &self.worker_heartbeat),
            ("crawl_freshness", &self.crawl_freshness),
            ("source_coverage", &self.source_coverage),
            ("alert_engine", &self.alert_engine),
            ("outbox", &self.outbox),
            ("notification_delivery", &self.notification_delivery),
            ("scheduled_jobs", &self.scheduled_jobs),
        ]
        .into_iter()
        .map(|(name, capability)| ComponentHealth {
            name: name.to_string(),
            status: match capability.state() {
                CapabilityState::Healthy => HealthStatus::Healthy,
                // Only a dead database makes the API process itself
                // unhealthy; optional dependencies (NATS, browser, worker
                // heartbeat, freshness) degrade it instead. Disabled,
                // NotConfigured and NotMeasured are never Healthy.
                CapabilityState::Unavailable if name == "database" => HealthStatus::Unhealthy,
                _ => HealthStatus::Degraded,
            },
            message: Some(capability.detail.clone()),
        })
        .collect()
    }

    /// Look up one capability by its stable probe name.
    pub fn capability(&self, name: &str) -> Option<&CapabilityStatus> {
        match name {
            "llm" => Some(&self.llm),
            "embeddings" => Some(&self.embeddings),
            "nats" => Some(&self.nats),
            "browser_renderer" => Some(&self.browser_renderer),
            "database" => Some(&self.database),
            "schema" => Some(&self.schema),
            "search_index" => Some(&self.search_index),
            "worker_heartbeat" => Some(&self.worker_heartbeat),
            "crawl_freshness" => Some(&self.crawl_freshness),
            "source_coverage" => Some(&self.source_coverage),
            "alert_engine" => Some(&self.alert_engine),
            "outbox" => Some(&self.outbox),
            "notification_delivery" => Some(&self.notification_delivery),
            "scheduled_jobs" => Some(&self.scheduled_jobs),
            _ => None,
        }
    }

    /// Readiness report for `profile`: only the capabilities the profile
    /// requires are included, and any one of them not reporting `ok` is
    /// `Unhealthy` (never a soft `Degraded`), so callers answer 503.
    pub fn readiness_checks(&self, profile: DeploymentProfile) -> Vec<ComponentHealth> {
        self.readiness_checks_for(profile.required_capabilities())
    }

    /// Readiness report for an explicit capability-name set. An unknown name
    /// fails closed as `Unhealthy` rather than being silently dropped, so a
    /// profile can never claim a requirement that is not actually measured.
    /// Only the profile-specific composition here may accept a `Disabled`
    /// capability, and only by *excluding* it (`names`); a required capability
    /// reporting any non-healthy state — including Disabled and
    /// NotConfigured — fails closed.
    pub fn readiness_checks_for(&self, names: &[&str]) -> Vec<ComponentHealth> {
        names
            .iter()
            .map(|name| match self.capability(name) {
                Some(capability) => ComponentHealth {
                    name: (*name).to_string(),
                    status: if capability.is_ok() {
                        HealthStatus::Healthy
                    } else {
                        HealthStatus::Unhealthy
                    },
                    message: Some(capability.detail.clone()),
                },
                None => ComponentHealth {
                    name: (*name).to_string(),
                    status: HealthStatus::Unhealthy,
                    message: Some("capability has no registered probe".to_string()),
                },
            })
            .collect()
    }

    /// Checks for one product surface, restricted to the capabilities the
    /// deployment profile actually requires. A surface with nothing required
    /// (e.g. delivery under `core`) is trivially healthy.
    pub fn surface_checks(
        &self,
        surface: ProductSurface,
        profile: DeploymentProfile,
    ) -> Vec<ComponentHealth> {
        let names: Vec<&str> = surface
            .capabilities()
            .iter()
            .copied()
            .filter(|name| profile.requires_capability(name))
            .collect();
        self.readiness_checks_for(&names)
    }

    /// Every surface measured for `profile`, in a stable order.
    pub fn surface_reports(&self, profile: DeploymentProfile) -> Vec<SurfaceHealth> {
        ProductSurface::ALL
            .into_iter()
            .map(|surface| {
                SurfaceHealth::from_checks(surface, self.surface_checks(surface, profile))
            })
            .collect()
    }
}

/// Map a readiness status to the probe's HTTP status: 503 exactly when a
/// capability the profile requires is missing.
pub fn readiness_http_status(status: &HealthStatus) -> StatusCode {
    match status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Run every capability probe. Probe results are measured per call; the
/// network/renderer/canary probes cache their own results behind short TTLs so
/// this stays cheap when polled.
pub async fn probe_capabilities(ctx: &ProbeContext<'_>) -> Capabilities {
    probe_capabilities_plan(ctx, None).await
}

/// Probe only the capabilities `profile` requires. Optional capabilities are
/// reported `disabled` without touching the network or filesystem, keeping the
/// frequently polled readiness probe cheap and free of unneeded side effects.
pub async fn probe_capabilities_for_profile(
    ctx: &ProbeContext<'_>,
    profile: DeploymentProfile,
) -> Capabilities {
    probe_capabilities_plan(ctx, Some(profile)).await
}

async fn probe_capabilities_plan(
    ctx: &ProbeContext<'_>,
    profile: Option<DeploymentProfile>,
) -> Capabilities {
    let store = PgStore::from_pool(ctx.pool.clone());
    let required = |name: &str| match profile {
        Some(profile) => profile.requires_capability(name),
        None => true,
    };

    let database = probe_database(ctx.pool).await;
    let schema = if required("schema") {
        probe_schema_lineage(&store, ctx.schema_lineage).await
    } else {
        not_required()
    };
    let embeddings = if required("embeddings") {
        probes::probe_embeddings(ctx.embedding_generator, &store, ctx.policy).await
    } else {
        not_required()
    };
    let search_index = if required("search_index") {
        probe_search_index(ctx.search_index, &store, ctx.policy).await
    } else {
        not_required()
    };
    let worker_heartbeat = probe_worker_heartbeat(&store).await;
    let crawl_freshness = if required("crawl_freshness") {
        probe_crawl_freshness(&store, ctx.policy).await
    } else {
        not_required()
    };
    let nats = if required("nats") {
        probe_nats(ctx.nats_url).await
    } else {
        not_required()
    };
    let browser_renderer = if required("browser_renderer") {
        probes::probe_browser_renderer(ctx.browser, ctx.policy).await
    } else {
        not_required()
    };
    let llm = if required("llm") {
        probes::probe_llm(ctx.llm, ctx.policy).await
    } else {
        not_required()
    };
    let source_coverage = if required("source_coverage") {
        probe_source_coverage(&store, ctx.policy).await
    } else {
        not_required()
    };
    let alert_engine = if required("alert_engine") {
        probe_alert_engine(&store, ctx.policy).await
    } else {
        not_required()
    };
    let outbox = if required("outbox") {
        probe_outbox(&store, ctx.policy).await
    } else {
        not_required()
    };
    let notification_delivery = if required("notification_delivery") {
        probe_notification_delivery(&store, ctx.policy).await
    } else {
        not_required()
    };
    let scheduled_jobs = if required("scheduled_jobs") {
        probe_scheduled_jobs(&store, ctx.policy).await
    } else {
        not_required()
    };

    Capabilities {
        llm,
        embeddings,
        nats,
        browser_renderer,
        database,
        schema,
        search_index,
        worker_heartbeat,
        crawl_freshness,
        source_coverage,
        alert_engine,
        outbox,
        notification_delivery,
        scheduled_jobs,
    }
}

fn not_required() -> CapabilityStatus {
    CapabilityStatus::new("disabled", "not required by the deployment profile")
}

async fn probe_database(pool: &sqlx::PgPool) -> CapabilityStatus {
    let started = std::time::Instant::now();
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => CapabilityStatus::new(
            "ok",
            format!("SELECT 1 succeeded in {}ms", started.elapsed().as_millis()),
        ),
        Err(error) => CapabilityStatus::new("unavailable", format!("query failed: {error}")),
    }
}

/// Schema-lineage health: the applied migration head must equal the head
/// embedded in this binary AND the applied history must checksum-verify. A
/// stale or divergent schema is `unavailable`, so full readiness can never
/// claim a running service whose database lineage it cannot prove. A lineage
/// already measured for the current request is reused, so the readiness
/// response and the capability always describe the same snapshot.
async fn probe_schema_lineage(
    store: &PgStore,
    measured: Option<&SchemaLineage>,
) -> CapabilityStatus {
    let lineage = match measured {
        Some(lineage) => Ok(lineage.clone()),
        None => store.schema_lineage().await,
    };
    match lineage {
        Ok(lineage) => {
            let expected = lineage
                .expected_head
                .map(|version| version.to_string())
                .unwrap_or_else(|| "none".to_string());
            let applied = lineage
                .applied_head
                .map(|version| version.to_string())
                .unwrap_or_else(|| "none".to_string());
            if lineage.is_current() {
                CapabilityStatus::new(
                    "ok",
                    format!(
                        "applied schema head {applied} matches embedded {expected}; {} migrations checksum-verified",
                        lineage.applied_count
                    ),
                )
            } else {
                CapabilityStatus::new(
                    "unavailable",
                    format!(
                        "schema lineage mismatch (embedded head {expected}, applied head {applied}): {}",
                        lineage
                            .problem
                            .unwrap_or_else(|| "unknown mismatch".to_string())
                    ),
                )
            }
        }
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!("schema lineage query failed: {error}"),
        ),
    }
}

/// Search-index health: empty is never `ok`, and the indexed high-water mark
/// must keep up with the database's newest observation.
async fn probe_search_index(
    index: &SearchIndex,
    store: &PgStore,
    policy: &ReadinessPolicy,
) -> CapabilityStatus {
    let docs = index.num_docs();
    let checkpoint = index.checkpoint();
    let db_high_water = match store.newest_observation_ts().await {
        Ok(high_water) => high_water,
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("observation high-water query failed: {error}"),
            )
        }
    };
    probes::evaluate_search_index(
        docs,
        checkpoint.as_ref(),
        db_high_water,
        policy.search_index_max_lag_secs,
        Utc::now(),
    )
}

async fn probe_nats(nats_url: Option<&str>) -> CapabilityStatus {
    let Some(url) = nats_url.filter(|url| !url.trim().is_empty()) else {
        return CapabilityStatus::not_configured("NATS_URL not configured");
    };

    match tokio::time::timeout(NATS_PROBE_TIMEOUT, async_nats::connect(url)).await {
        Ok(Ok(_client)) => CapabilityStatus::new("ok", format!("connected to {url}")),
        Ok(Err(error)) => CapabilityStatus::new("unavailable", format!("connect failed: {error}")),
        Err(_) => CapabilityStatus::new("unavailable", "connect timed out after 2s"),
    }
}

async fn probe_worker_heartbeat(store: &PgStore) -> CapabilityStatus {
    match store.latest_service_heartbeat("worker").await {
        Ok(Some(row)) => heartbeat_capability(row),
        Ok(None) => CapabilityStatus::new("unavailable", "no worker heartbeat recorded"),
        Err(error) => {
            CapabilityStatus::new("unavailable", format!("heartbeat query failed: {error}"))
        }
    }
}

fn heartbeat_capability(row: ServiceHeartbeatRow) -> CapabilityStatus {
    let age = Utc::now().signed_duration_since(row.last_seen_at);
    let age_seconds = age.num_seconds().max(0);
    let mut status = CapabilityStatus::new(
        if age_seconds > WORKER_HEARTBEAT_STALE_AFTER_SECS {
            "degraded"
        } else {
            "ok"
        },
        format!(
            "worker {} (v{}) heartbeat {} ago",
            row.instance_id,
            row.version,
            format_age(age)
        ),
    );
    status.last_seen_at = Some(row.last_seen_at.to_rfc3339());
    status.age_seconds = Some(age_seconds);
    status
}

async fn probe_crawl_freshness(store: &PgStore, policy: &ReadinessPolicy) -> CapabilityStatus {
    match store.newest_observation_ts().await {
        Ok(Some(newest)) => {
            let age = Utc::now().signed_duration_since(newest);
            let age_seconds = age.num_seconds().max(0);
            let mut status = CapabilityStatus::new(
                if age_seconds > policy.crawl_freshness_max_age_secs {
                    "degraded"
                } else {
                    "ok"
                },
                format!(
                    "newest observation {} ago (policy max {}s)",
                    format_age(age),
                    policy.crawl_freshness_max_age_secs
                ),
            );
            status.last_seen_at = Some(newest.to_rfc3339());
            status.age_seconds = Some(age_seconds);
            status
        }
        Ok(None) => CapabilityStatus::new("unavailable", "no observations recorded"),
        Err(error) => {
            CapabilityStatus::new("unavailable", format!("freshness query failed: {error}"))
        }
    }
}

async fn probe_source_coverage(store: &PgStore, policy: &ReadinessPolicy) -> CapabilityStatus {
    let states = match store.load_source_runtime_states().await {
        Ok(states) => states,
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("source runtime state query failed: {error}"),
            )
        }
    };
    let registry = apex_crawl::sources_registry::all_sources();
    let deployment_caps = apex_crawl::sources_registry::DeploymentCapabilities::from_env();
    let now = Utc::now();
    let summary = apex_crawl::sources_registry::source_coverage_summary(
        &registry,
        &states,
        &deployment_caps,
        now,
    );
    let window_start =
        now - chrono::Duration::seconds(policy.coverage.priority_company_window_secs.max(0));
    let priority_companies = match store.priority_company_coverage(window_start).await {
        Ok((total, covered)) => apex_crawl::coverage::PriorityCompanyCoverage {
            total: total.max(0) as usize,
            covered: covered.max(0) as usize,
        },
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("priority-company coverage query failed: {error}"),
            )
        }
    };
    probes::evaluate_source_coverage(&summary, policy, priority_companies, now)
}

async fn probe_alert_engine(store: &PgStore, policy: &ReadinessPolicy) -> CapabilityStatus {
    match store.alert_engine_state().await {
        Ok(state) => probes::evaluate_alert_engine(state.as_ref(), policy, Utc::now()),
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!("alert-engine state query failed: {error}"),
        ),
    }
}

async fn probe_outbox(store: &PgStore, policy: &ReadinessPolicy) -> CapabilityStatus {
    match store.outbox_backlog().await {
        Ok(backlog) => probes::evaluate_outbox(Some(&backlog), policy, Utc::now()),
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!("outbox backlog query failed: {error}"),
        ),
    }
}

/// Durable channel delivery health: backlog and oldest overdue age,
/// dead-letter count and recent change rate, stuck `delivering` leases, the
/// recent attempt success ratio, and the retry processor's own freshness.
async fn probe_notification_delivery(
    store: &PgStore,
    policy: &ReadinessPolicy,
) -> CapabilityStatus {
    let now = Utc::now();
    let health = match store
        .notification_delivery_health(now, policy.notification_delivery_success_window_secs)
        .await
    {
        Ok(health) => health,
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("notification delivery health query failed: {error}"),
            )
        }
    };
    let processor = match store.list_worker_job_states().await {
        Ok(states) => states
            .into_iter()
            .find(|state| state.job_kind == "notification_delivery"),
        Err(error) => {
            return CapabilityStatus::new(
                "unavailable",
                format!("worker job state query failed: {error}"),
            )
        }
    };
    probes::evaluate_notification_delivery(Some(&health), processor.as_ref(), policy, now)
}

async fn probe_scheduled_jobs(store: &PgStore, policy: &ReadinessPolicy) -> CapabilityStatus {
    match store.list_worker_job_states().await {
        Ok(states) => probes::evaluate_scheduled_jobs(&states, policy, Utc::now()),
        Err(error) => CapabilityStatus::new(
            "unavailable",
            format!("scheduled job state query failed: {error}"),
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ok_capability(detail: &str) -> CapabilityStatus {
        CapabilityStatus::new("ok", detail)
    }

    fn sample_capabilities() -> Capabilities {
        Capabilities {
            llm: ok_capability("model probe-model answered"),
            embeddings: ok_capability("canary round-trip ok"),
            nats: ok_capability("connected to nats://127.0.0.1:4222"),
            browser_renderer: ok_capability("rendered data: fixture; JS marker verified"),
            database: ok_capability("SELECT 1 succeeded in 1ms"),
            schema: ok_capability(
                "applied schema head 78 matches embedded 78; 78 migrations checksum-verified",
            ),
            search_index: {
                let mut status = ok_capability("42 documents indexed; lag 600s");
                status.lag_seconds = Some(600);
                status
            },
            worker_heartbeat: {
                let mut status = ok_capability("worker heartbeat 5s ago");
                status.age_seconds = Some(5);
                status
            },
            crawl_freshness: {
                let mut status = ok_capability("newest observation 4m ago");
                status.age_seconds = Some(240);
                status
            },
            source_coverage: ok_capability("60 operational sources"),
            alert_engine: ok_capability("25 rules loaded"),
            outbox: ok_capability("0 pending"),
            notification_delivery: ok_capability("0 pending, 0 overdue, 0 dead-lettered"),
            scheduled_jobs: ok_capability("2 critical jobs fresh"),
        }
    }

    const ALL_CAPABILITY_NAMES: [&str; 14] = [
        "database",
        "schema",
        "worker_heartbeat",
        "llm",
        "embeddings",
        "nats",
        "search_index",
        "browser_renderer",
        "crawl_freshness",
        "source_coverage",
        "alert_engine",
        "outbox",
        "notification_delivery",
        "scheduled_jobs",
    ];

    #[test]
    fn capability_state_mapping_is_total_and_never_defaults_to_healthy() {
        assert_eq!(CapabilityState::from_status("ok"), CapabilityState::Healthy);
        assert_eq!(
            CapabilityState::from_status("degraded"),
            CapabilityState::Degraded
        );
        assert_eq!(
            CapabilityState::from_status("unavailable"),
            CapabilityState::Unavailable
        );
        assert_eq!(
            CapabilityState::from_status("disabled"),
            CapabilityState::Disabled
        );
        assert_eq!(
            CapabilityState::from_status("not_configured"),
            CapabilityState::NotConfigured
        );
        assert_eq!(
            CapabilityState::from_status("not_measured"),
            CapabilityState::NotMeasured
        );
        // An unknown status is never healthy.
        assert_eq!(
            CapabilityState::from_status("something-new"),
            CapabilityState::NotMeasured
        );
        assert!(!CapabilityState::NotMeasured.is_healthy());
        assert!(!CapabilityState::Disabled.is_healthy());
        assert!(!CapabilityState::NotConfigured.is_healthy());
        assert!(CapabilityState::Healthy.is_healthy());
    }

    #[test]
    fn capability_status_publishes_the_typed_state() {
        let json = serde_json::to_value(CapabilityStatus::not_configured("no URL")).expect("json");
        assert_eq!(json["state"], "not_configured");
        assert_eq!(json["status"], "not_configured");

        let json =
            serde_json::to_value(CapabilityStatus::new("disabled", "not required")).expect("json");
        assert_eq!(json["state"], "disabled");
    }

    #[test]
    fn generic_checks_and_overall_status_never_treat_disabled_as_healthy() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::not_configured("NATS_URL not configured");
        caps.browser_renderer = CapabilityStatus::new("disabled", "headless browser disabled");
        caps.llm = CapabilityStatus::not_measured("probe did not run");

        assert_ne!(
            caps.overall_status(),
            "ok",
            "disabled/not_configured/not_measured must degrade the generic aggregate"
        );

        let checks = caps.health_checks();
        for name in ["nats", "browser_renderer", "llm"] {
            let check = checks
                .iter()
                .find(|check| check.name == name)
                .expect("check is published");
            assert_ne!(
                check.status,
                HealthStatus::Healthy,
                "{name} must not be reported Healthy when it is not measured healthy"
            );
        }
    }

    #[test]
    fn profile_readiness_composition_is_where_disabled_becomes_acceptable() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("disabled", "not required by core");
        caps.llm = CapabilityStatus::not_configured("no model");

        // Core does not require either capability, so readiness stays green.
        assert_eq!(
            readiness_status(&caps, DeploymentProfile::Core),
            HealthStatus::Healthy
        );

        // Full requires both, and neither is measured healthy: fail closed.
        let full = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(full, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&full),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn capability_payload_has_required_keys_and_types() {
        let json = serde_json::to_value(sample_capabilities()).expect("serializes");

        let object = json.as_object().expect("capabilities object");
        for key in ALL_CAPABILITY_NAMES {
            let entry = object
                .get(key)
                .unwrap_or_else(|| panic!("missing capability key {key}"));
            let entry_object = entry
                .as_object()
                .unwrap_or_else(|| panic!("capability {key} should be an object"));
            assert!(
                entry_object.get("status").is_some_and(|v| v.is_string()),
                "{key}.status must be a string"
            );
            assert!(
                entry_object.get("detail").is_some_and(|v| v.is_string()),
                "{key}.detail must be a string"
            );
        }
        assert_eq!(
            json["search_index"]["lag_seconds"], 600,
            "index lag must be exposed"
        );
    }

    #[test]
    fn overall_status_is_degraded_when_any_probe_is_degraded() {
        let mut caps = sample_capabilities();
        assert_eq!(caps.overall_status(), "ok");

        caps.nats = CapabilityStatus::new("degraded", "connect timed out");
        assert_eq!(caps.overall_status(), "degraded");

        caps.nats = CapabilityStatus::new("unavailable", "connect refused");
        assert_eq!(caps.overall_status(), "unavailable");

        let mut caps = sample_capabilities();
        caps.alert_engine = CapabilityStatus::new("degraded", "0 rules");
        assert_eq!(caps.overall_status(), "degraded");
    }

    #[test]
    fn status_strip_reports_current_data_and_stale_worker() {
        let caps = sample_capabilities();
        let strip = caps.status_strip();
        assert!(strip.system_ok);
        assert_eq!(
            strip.data_freshness,
            "Data current · newest observation 4m ago"
        );

        let mut stale = caps;
        stale.worker_heartbeat.age_seconds = Some(600);
        let strip = stale.status_strip();
        assert!(!strip.system_ok);
        assert!(strip.system_status.contains("worker heartbeat 10m ago"));
    }

    #[test]
    fn health_checks_expose_every_capability() {
        let checks = sample_capabilities().health_checks();
        let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "database",
                "schema",
                "llm",
                "embeddings",
                "nats",
                "browser_renderer",
                "search_index",
                "worker_heartbeat",
                "crawl_freshness",
                "source_coverage",
                "alert_engine",
                "outbox",
                "notification_delivery",
                "scheduled_jobs",
            ]
        );
        assert!(checks
            .iter()
            .all(|check| check.status == HealthStatus::Healthy));
    }

    #[test]
    fn optional_dependency_outage_degrades_but_database_outage_is_unhealthy() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("unavailable", "connection refused");
        let checks = caps.health_checks();
        let nats = checks
            .iter()
            .find(|check| check.name == "nats")
            .expect("nats check");
        assert_eq!(nats.status, HealthStatus::Degraded);

        let mut caps = sample_capabilities();
        caps.database = CapabilityStatus::new("unavailable", "connection refused");
        let checks = caps.health_checks();
        let database = checks
            .iter()
            .find(|check| check.name == "database")
            .expect("database check");
        assert_eq!(database.status, HealthStatus::Unhealthy);
    }

    fn readiness_status(caps: &Capabilities, profile: DeploymentProfile) -> HealthStatus {
        aggregate_health(&caps.readiness_checks(profile))
    }

    #[test]
    fn unknown_required_capability_fails_closed() {
        let caps = sample_capabilities();
        let checks = caps.readiness_checks_for(&["database", "not_a_capability"]);

        let unknown = checks
            .iter()
            .find(|check| check.name == "not_a_capability")
            .expect("unknown capability is reported, not dropped");
        assert_eq!(unknown.status, HealthStatus::Unhealthy);
        assert_eq!(aggregate_health(&checks), HealthStatus::Unhealthy);
    }

    #[test]
    fn core_profile_readiness_permits_disabled_optional_capabilities() {
        let mut caps = sample_capabilities();
        for name in [
            "nats",
            "browser_renderer",
            "llm",
            "crawl_freshness",
            "source_coverage",
            "alert_engine",
            "outbox",
            "notification_delivery",
            "scheduled_jobs",
        ] {
            caps = with_capability(
                caps,
                name,
                CapabilityStatus::new("disabled", "not required"),
            );
        }

        let status = readiness_status(&caps, DeploymentProfile::Core);
        assert_eq!(status, HealthStatus::Healthy);
        assert_eq!(readiness_http_status(&status), StatusCode::OK);
    }

    #[test]
    fn core_profile_readiness_fails_503_when_required_capability_missing() {
        let mut caps = sample_capabilities();
        caps.worker_heartbeat =
            CapabilityStatus::new("unavailable", "no worker heartbeat recorded");

        let status = readiness_status(&caps, DeploymentProfile::Core);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn full_profile_readiness_fails_503_when_optional_capability_missing() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("disabled", "NATS_URL not configured");
        caps.browser_renderer = CapabilityStatus::new("disabled", "browser disabled");

        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn full_profile_readiness_is_200_when_every_capability_is_ok() {
        let caps = sample_capabilities();
        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Healthy);
        assert_eq!(readiness_http_status(&status), StatusCode::OK);
    }

    #[test]
    fn full_readiness_fails_503_for_every_required_capability() {
        for name in ALL_CAPABILITY_NAMES {
            let mut caps = sample_capabilities();
            caps = with_capability(caps, name, CapabilityStatus::new("degraded", "broken"));
            let status = readiness_status(&caps, DeploymentProfile::Full);
            assert_eq!(
                status,
                HealthStatus::Unhealthy,
                "{name} must gate full readiness"
            );
            assert_eq!(
                readiness_http_status(&status),
                StatusCode::SERVICE_UNAVAILABLE,
                "{name} must yield 503"
            );
        }
    }

    #[test]
    fn full_readiness_includes_crawl_freshness_and_alert_engine_state() {
        let caps = sample_capabilities();
        let checks = caps.readiness_checks(DeploymentProfile::Full);
        let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();
        assert!(
            names.contains(&"crawl_freshness"),
            "full must require crawl freshness"
        );
        assert!(
            names.contains(&"alert_engine"),
            "full must require alert-engine state"
        );
        assert!(names.contains(&"source_coverage"));
        assert!(names.contains(&"outbox"));
        assert!(names.contains(&"scheduled_jobs"));

        let mut caps = sample_capabilities();
        caps.crawl_freshness = CapabilityStatus::new("degraded", "stale data");
        assert_eq!(
            readiness_status(&caps, DeploymentProfile::Full),
            HealthStatus::Unhealthy
        );

        let mut caps = sample_capabilities();
        caps.alert_engine = CapabilityStatus::new("degraded", "reload failed");
        assert_eq!(
            readiness_status(&caps, DeploymentProfile::Full),
            HealthStatus::Unhealthy
        );
    }

    fn healthy_delivery_health() -> apex_store::postgres::NotificationDeliveryHealth {
        apex_store::postgres::NotificationDeliveryHealth {
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

    fn delivery_processor(
        last_run: chrono::DateTime<Utc>,
    ) -> apex_store::postgres::WorkerJobStateRecord {
        apex_store::postgres::WorkerJobStateRecord {
            job_kind: "notification_delivery".to_string(),
            last_run: Some(last_run),
            last_status: Some("succeeded".to_string()),
            last_error: None,
            last_duration_ms: Some(10),
            consecutive_failures: 0,
            max_consecutive_failures: 5,
            circuit_open: false,
            updated_at: last_run,
        }
    }

    #[test]
    fn full_readiness_fails_503_when_notification_delivery_degrades() {
        let policy = ReadinessPolicy::default();
        let now = Utc::now();

        // Backlog/dead-letters above policy: the capability degrades and full
        // readiness answers 503.
        let mut caps = sample_capabilities();
        caps.notification_delivery = probes::evaluate_notification_delivery(
            Some(&apex_store::postgres::NotificationDeliveryHealth {
                dead_lettered: 100,
                ..healthy_delivery_health()
            }),
            Some(&delivery_processor(now)),
            &policy,
            now,
        );
        assert!(!caps.notification_delivery.is_ok());
        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            StatusCode::SERVICE_UNAVAILABLE
        );

        // A stale retry processor: deliveries are not being attempted, so full
        // readiness answers 503 even with an empty backlog.
        let mut caps = sample_capabilities();
        caps.notification_delivery = probes::evaluate_notification_delivery(
            Some(&healthy_delivery_health()),
            Some(&delivery_processor(now - chrono::Duration::seconds(3_600))),
            &policy,
            now,
        );
        assert!(!caps.notification_delivery.is_ok());
        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            StatusCode::SERVICE_UNAVAILABLE
        );

        // A healthy pipeline keeps full readiness green.
        let mut caps = sample_capabilities();
        caps.notification_delivery = probes::evaluate_notification_delivery(
            Some(&healthy_delivery_health()),
            Some(&delivery_processor(now)),
            &policy,
            now,
        );
        assert!(caps.notification_delivery.is_ok());
        assert_eq!(
            readiness_status(&caps, DeploymentProfile::Full),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn readiness_reports_a_schema_head_mismatch_as_unhealthy() {
        let mut caps = sample_capabilities();
        caps.schema = CapabilityStatus::new(
            "unavailable",
            "schema lineage mismatch (embedded head 79, applied head 78): database schema is stale",
        );

        for profile in [DeploymentProfile::Core, DeploymentProfile::Full] {
            let status = readiness_status(&caps, profile);
            assert_eq!(
                status,
                HealthStatus::Unhealthy,
                "{profile} must gate on schema lineage"
            );
            assert_eq!(
                readiness_http_status(&status),
                StatusCode::SERVICE_UNAVAILABLE,
                "{profile} must answer 503 on a schema head mismatch"
            );
        }

        let checks = caps.health_checks();
        let schema = checks
            .iter()
            .find(|check| check.name == "schema")
            .expect("schema check is published");
        assert_eq!(schema.status, HealthStatus::Degraded);
        assert!(schema
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("embedded head 79"));
    }

    #[test]
    fn schema_lineage_report_exposes_both_heads_and_checksum_result() {
        let current = SchemaLineage {
            expected_head: Some(78),
            applied_head: Some(78),
            applied_count: 78,
            checksums_verified: true,
            problem: None,
        };
        let report = SchemaLineageReport::from_result(Ok(current));
        assert_eq!(report.status, "ok");
        assert_eq!(report.expected_head, Some(78));
        assert_eq!(report.applied_head, Some(78));
        assert!(report.checksums_verified);
        assert!(report.detail.contains("matches embedded head 78"));

        let stale = SchemaLineage {
            expected_head: Some(79),
            applied_head: Some(78),
            applied_count: 78,
            checksums_verified: true,
            problem: Some(
                "database schema is stale: latest applied migration is 78, embedded latest is 79"
                    .to_string(),
            ),
        };
        let report = SchemaLineageReport::from_result(Ok(stale));
        assert_eq!(report.status, "mismatch");
        assert_eq!(report.expected_head, Some(79));
        assert_eq!(report.applied_head, Some(78));
        assert!(
            report.checksums_verified,
            "the applied rows still verify; the report must say so"
        );
        assert!(report.detail.contains("stale"));

        let unavailable = SchemaLineageReport::from_result(Err("connection refused".to_string()));
        assert_eq!(unavailable.status, "unavailable");
        assert_eq!(unavailable.expected_head, None);
        assert!(!unavailable.checksums_verified);
        assert!(unavailable.detail.contains("connection refused"));
    }

    #[test]
    fn readiness_report_lists_exactly_the_required_capabilities() {
        let caps = sample_capabilities();

        let core_checks = caps.readiness_checks(DeploymentProfile::Core);
        let core: Vec<&str> = core_checks
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert_eq!(
            core,
            vec![
                "database",
                "schema",
                "worker_heartbeat",
                "embeddings",
                "search_index"
            ]
        );

        let full_checks = caps.readiness_checks(DeploymentProfile::Full);
        let full: Vec<&str> = full_checks
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert_eq!(
            full,
            vec![
                "database",
                "schema",
                "worker_heartbeat",
                "llm",
                "embeddings",
                "nats",
                "search_index",
                "browser_renderer",
                "crawl_freshness",
                "source_coverage",
                "alert_engine",
                "outbox",
                "notification_delivery",
                "scheduled_jobs",
            ]
        );
    }

    #[test]
    fn full_surfaces_compose_the_whole_required_capability_set() {
        let caps = sample_capabilities();
        let reports = caps.surface_reports(DeploymentProfile::Full);
        assert_eq!(reports.len(), 4);
        assert!(reports.iter().all(|report| report.status == "ok"));

        let composed: Vec<String> = reports
            .iter()
            .flat_map(|report| report.checks.iter().map(|check| check.name.clone()))
            .collect();
        for name in ALL_CAPABILITY_NAMES {
            assert!(
                composed.contains(&name.to_string()),
                "surface composition is missing {name}"
            );
        }
        assert_eq!(composed.len(), ALL_CAPABILITY_NAMES.len());
    }

    #[test]
    fn surface_status_is_503_when_a_required_check_is_degraded() {
        let mut caps = sample_capabilities();
        caps.outbox = CapabilityStatus::new("degraded", "backlog exceeds policy");
        let reports = caps.surface_reports(DeploymentProfile::Full);

        let delivery = reports
            .iter()
            .find(|report| report.surface == "delivery")
            .expect("delivery surface");
        assert_eq!(delivery.status, "unhealthy");
        assert_eq!(delivery.http_status(), StatusCode::SERVICE_UNAVAILABLE);

        let process = reports
            .iter()
            .find(|report| report.surface == "process")
            .expect("process surface");
        assert_eq!(process.status, "ok");
        assert_eq!(process.http_status(), StatusCode::OK);
    }

    #[test]
    fn core_surfaces_only_include_required_capabilities() {
        let mut caps = sample_capabilities();
        for name in [
            "nats",
            "browser_renderer",
            "llm",
            "crawl_freshness",
            "source_coverage",
            "alert_engine",
            "outbox",
            "notification_delivery",
            "scheduled_jobs",
        ] {
            caps = with_capability(
                caps,
                name,
                CapabilityStatus::new("disabled", "not required"),
            );
        }

        let reports = caps.surface_reports(DeploymentProfile::Core);

        let process = reports
            .iter()
            .find(|report| report.surface == "process")
            .expect("process surface");
        assert_eq!(process.http_status(), StatusCode::OK);
        assert!(process.checks.is_empty() || process.status == "ok");

        let delivery = reports
            .iter()
            .find(|report| report.surface == "delivery")
            .expect("delivery surface");
        assert!(delivery.checks.is_empty());

        let data = reports
            .iter()
            .find(|report| report.surface == "data")
            .expect("data surface");
        assert!(data.checks.iter().all(|check| check.name == "search_index"));
    }

    fn with_capability(
        mut caps: Capabilities,
        name: &str,
        status: CapabilityStatus,
    ) -> Capabilities {
        match name {
            "llm" => caps.llm = status,
            "embeddings" => caps.embeddings = status,
            "nats" => caps.nats = status,
            "browser_renderer" => caps.browser_renderer = status,
            "database" => caps.database = status,
            "schema" => caps.schema = status,
            "search_index" => caps.search_index = status,
            "worker_heartbeat" => caps.worker_heartbeat = status,
            "crawl_freshness" => caps.crawl_freshness = status,
            "source_coverage" => caps.source_coverage = status,
            "alert_engine" => caps.alert_engine = status,
            "outbox" => caps.outbox = status,
            "notification_delivery" => caps.notification_delivery = status,
            "scheduled_jobs" => caps.scheduled_jobs = status,
            other => panic!("unknown capability {other}"),
        }
        caps
    }
}
