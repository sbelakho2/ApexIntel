//! `/api/health/capabilities` — capability health derived from real probes.
//!
//! Every value is measured rather than assumed: the LLM entry reflects the
//! compiled feature set, NATS/embeddings/database entries are live probes, the
//! worker heartbeat reads `service_heartbeats.last_seen_at` (migration 049),
//! and crawl freshness reads the newest `observations.ts_utc`.

use std::time::Duration as StdDuration;

use apex_core::profile::DeploymentProfile;
use chrono::Utc;
use serde::Serialize;

use apex_store::postgres::{PgStore, ServiceHeartbeatRow};
use apex_store::tantivy_index::SearchIndex;

#[cfg(test)]
use crate::responses::aggregate_health;
use crate::responses::{ComponentHealth, HealthStatus};
use crate::system_status::{
    format_age, StatusStrip, DATA_FRESH_WITHIN_SECS, WORKER_HEARTBEAT_STALE_AFTER_SECS,
};

pub const CAPABILITIES_PATH: &str = "/api/health/capabilities";

const NATS_PROBE_TIMEOUT: StdDuration = StdDuration::from_secs(2);

/// One capability probe result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapabilityStatus {
    /// `ok` | `degraded` | `unavailable` | `disabled`.
    pub status: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
}

impl CapabilityStatus {
    pub fn new(status: &str, detail: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            detail: detail.into(),
            last_seen_at: None,
            age_seconds: None,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }
}

/// Full capability report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Capabilities {
    pub llm: CapabilityStatus,
    pub embeddings: CapabilityStatus,
    pub nats: CapabilityStatus,
    pub browser_renderer: CapabilityStatus,
    pub database: CapabilityStatus,
    pub search_index: CapabilityStatus,
    pub worker_heartbeat: CapabilityStatus,
    pub crawl_freshness: CapabilityStatus,
}

impl Capabilities {
    /// Worst-of status across all capabilities (`ok` when everything is fine).
    pub fn overall_status(&self) -> &'static str {
        let all = [
            &self.llm,
            &self.embeddings,
            &self.nats,
            &self.browser_renderer,
            &self.database,
            &self.search_index,
            &self.worker_heartbeat,
            &self.crawl_freshness,
        ];
        if all.iter().any(|c| c.status == "unavailable") {
            "unavailable"
        } else if all.iter().any(|c| c.status == "degraded") {
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
            ("llm", &self.llm),
            ("embeddings", &self.embeddings),
            ("nats", &self.nats),
            ("browser_renderer", &self.browser_renderer),
            ("search_index", &self.search_index),
            ("worker_heartbeat", &self.worker_heartbeat),
            ("crawl_freshness", &self.crawl_freshness),
        ]
        .into_iter()
        .map(|(name, capability)| ComponentHealth {
            name: name.to_string(),
            status: match capability.status.as_str() {
                "ok" | "disabled" => HealthStatus::Healthy,
                // Only a dead database makes the API process itself
                // unhealthy; optional dependencies (NATS, browser, worker
                // heartbeat, freshness) degrade it instead.
                "unavailable" if name == "database" => HealthStatus::Unhealthy,
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
            "search_index" => Some(&self.search_index),
            "worker_heartbeat" => Some(&self.worker_heartbeat),
            "crawl_freshness" => Some(&self.crawl_freshness),
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
}

/// Map a readiness status to the probe's HTTP status: 503 exactly when a
/// capability the profile requires is missing.
pub fn readiness_http_status(status: &HealthStatus) -> axum::http::StatusCode {
    match status {
        HealthStatus::Healthy | HealthStatus::Degraded => axum::http::StatusCode::OK,
        HealthStatus::Unhealthy => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Run every capability probe. Results are measured per call; nothing is
/// cached so `/api/health/capabilities` never reports stale health.
pub async fn probe_capabilities(
    pool: &sqlx::PgPool,
    search_index: &SearchIndex,
    nats_url: Option<&str>,
) -> Capabilities {
    probe_capabilities_plan(pool, search_index, nats_url, None).await
}

/// Probe only the capabilities `profile` requires. Optional capabilities are
/// reported `disabled` without touching the network or filesystem, keeping the
/// frequently polled readiness probe cheap and free of unneeded side effects.
pub async fn probe_capabilities_for_profile(
    pool: &sqlx::PgPool,
    search_index: &SearchIndex,
    nats_url: Option<&str>,
    profile: DeploymentProfile,
) -> Capabilities {
    probe_capabilities_plan(pool, search_index, nats_url, Some(profile)).await
}

async fn probe_capabilities_plan(
    pool: &sqlx::PgPool,
    search_index: &SearchIndex,
    nats_url: Option<&str>,
    profile: Option<DeploymentProfile>,
) -> Capabilities {
    let store = PgStore::from_pool(pool.clone());
    let required = |name: &str| match profile {
        Some(profile) => profile.requires_capability(name),
        None => true,
    };

    let database = probe_database(pool).await;
    let embeddings = probe_embeddings(pool).await;

    let docs = search_index.num_docs();
    let search_index_status = CapabilityStatus::new(
        "ok",
        format!("{docs} documents indexed in the in-process search index"),
    );

    let worker_heartbeat = probe_worker_heartbeat(&store).await;
    let crawl_freshness = if required("crawl_freshness") {
        probe_crawl_freshness(&store).await
    } else {
        not_required()
    };
    let nats = if required("nats") {
        probe_nats(nats_url).await
    } else {
        not_required()
    };
    let browser_renderer = if required("browser_renderer") {
        probe_browser_renderer()
    } else {
        not_required()
    };
    let llm = if required("llm") {
        probe_llm()
    } else {
        not_required()
    };

    Capabilities {
        llm,
        embeddings,
        nats,
        browser_renderer,
        database,
        search_index: search_index_status,
        worker_heartbeat,
        crawl_freshness,
    }
}

fn not_required() -> CapabilityStatus {
    CapabilityStatus::new("disabled", "not required by the deployment profile")
}

fn probe_llm() -> CapabilityStatus {
    if cfg!(feature = "llm") {
        CapabilityStatus::new("ok", "llm feature compiled in")
    } else {
        CapabilityStatus::new("disabled", "binary built without the llm feature")
    }
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

async fn probe_embeddings(pool: &sqlx::PgPool) -> CapabilityStatus {
    let probe = sqlx::query_scalar::<_, bool>(
        r#"SELECT
             EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector')
             AND EXISTS (
                 SELECT 1 FROM information_schema.columns
                 WHERE table_name = 'embeddings' AND column_name = 'embedding'
             )"#,
    )
    .fetch_one(pool)
    .await;

    match probe {
        Ok(true) => {
            CapabilityStatus::new("ok", "pgvector extension and embeddings.embedding present")
        }
        Ok(false) => CapabilityStatus::new(
            "degraded",
            "pgvector extension or embeddings.embedding column missing",
        ),
        Err(error) => CapabilityStatus::new("degraded", format!("probe failed: {error}")),
    }
}

async fn probe_nats(nats_url: Option<&str>) -> CapabilityStatus {
    let Some(url) = nats_url.filter(|url| !url.trim().is_empty()) else {
        return CapabilityStatus::new("disabled", "NATS_URL not configured");
    };

    match tokio::time::timeout(NATS_PROBE_TIMEOUT, async_nats::connect(url)).await {
        Ok(Ok(_client)) => CapabilityStatus::new("ok", format!("connected to {url}")),
        Ok(Err(error)) => CapabilityStatus::new("unavailable", format!("connect failed: {error}")),
        Err(_) => CapabilityStatus::new("unavailable", "connect timed out after 2s"),
    }
}

fn probe_browser_renderer() -> CapabilityStatus {
    let enabled = std::env::var("ENABLE_HEADLESS_BROWSER")
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false);
    if !enabled {
        return CapabilityStatus::new(
            "disabled",
            "headless browser disabled (ENABLE_HEADLESS_BROWSER is not truthy)",
        );
    }

    let binary = std::env::var("HEADLESS_BROWSER_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "google-chrome".to_string());

    if binary_on_path(&binary) {
        CapabilityStatus::new("ok", format!("headless browser binary found: {binary}"))
    } else {
        CapabilityStatus::new(
            "degraded",
            format!("ENABLE_HEADLESS_BROWSER is set but binary '{binary}' was not found on PATH"),
        )
    }
}

fn binary_on_path(binary: &str) -> bool {
    let candidate = std::path::Path::new(binary);
    if candidate.components().count() > 1 {
        return candidate.is_file();
    }
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|dir| {
                let full = dir.join(binary);
                full.is_file()
            })
        })
        .unwrap_or(false)
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

async fn probe_crawl_freshness(store: &PgStore) -> CapabilityStatus {
    match store.newest_observation_ts().await {
        Ok(Some(newest)) => {
            let age = Utc::now().signed_duration_since(newest);
            let age_seconds = age.num_seconds().max(0);
            let mut status = CapabilityStatus::new(
                if age_seconds > DATA_FRESH_WITHIN_SECS {
                    "degraded"
                } else {
                    "ok"
                },
                format!("newest observation {} ago", format_age(age)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_capabilities() -> Capabilities {
        Capabilities {
            llm: CapabilityStatus::new("ok", "llm feature compiled in"),
            embeddings: CapabilityStatus::new("ok", "pgvector present"),
            nats: CapabilityStatus::new("disabled", "NATS_URL not configured"),
            browser_renderer: CapabilityStatus::new("disabled", "disabled in env"),
            database: CapabilityStatus::new("ok", "SELECT 1 succeeded in 1ms"),
            search_index: CapabilityStatus::new("ok", "12 documents indexed"),
            worker_heartbeat: {
                let mut status = CapabilityStatus::new("ok", "worker heartbeat 5s ago");
                status.age_seconds = Some(5);
                status
            },
            crawl_freshness: {
                let mut status = CapabilityStatus::new("ok", "newest observation 4m ago");
                status.age_seconds = Some(240);
                status
            },
        }
    }

    #[test]
    fn capability_payload_has_required_keys_and_types() {
        let json = serde_json::to_value(sample_capabilities()).expect("serializes");

        let object = json.as_object().expect("capabilities object");
        for key in [
            "llm",
            "embeddings",
            "nats",
            "browser_renderer",
            "database",
            "search_index",
            "worker_heartbeat",
            "crawl_freshness",
        ] {
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
    }

    #[test]
    fn overall_status_is_degraded_when_any_probe_is_degraded() {
        let mut caps = sample_capabilities();
        assert_eq!(caps.overall_status(), "ok");

        caps.nats = CapabilityStatus::new("degraded", "connect timed out");
        assert_eq!(caps.overall_status(), "degraded");

        caps.nats = CapabilityStatus::new("unavailable", "connect refused");
        assert_eq!(caps.overall_status(), "unavailable");
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
                "llm",
                "embeddings",
                "nats",
                "browser_renderer",
                "search_index",
                "worker_heartbeat",
                "crawl_freshness",
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
    fn core_profile_readiness_permits_disabled_nats_browser_and_llm() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("disabled", "NATS_URL not configured");
        caps.browser_renderer = CapabilityStatus::new("disabled", "browser disabled");
        caps.llm = CapabilityStatus::new("disabled", "binary built without the llm feature");

        let status = readiness_status(&caps, DeploymentProfile::Core);
        assert_eq!(status, HealthStatus::Healthy);
        assert_eq!(readiness_http_status(&status), axum::http::StatusCode::OK);
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
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn full_profile_readiness_fails_503_when_optional_capability_missing() {
        let caps = sample_capabilities();
        assert!(!caps.nats.is_ok() && !caps.browser_renderer.is_ok());

        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn full_profile_readiness_is_200_when_every_capability_is_ok() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("ok", "connected to nats://127.0.0.1:4222");
        caps.browser_renderer = CapabilityStatus::new("ok", "headless browser binary found");

        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Healthy);
        assert_eq!(readiness_http_status(&status), axum::http::StatusCode::OK);
    }

    #[test]
    fn full_profile_readiness_fails_503_without_llm_capability() {
        let mut caps = sample_capabilities();
        caps.nats = CapabilityStatus::new("ok", "connected");
        caps.browser_renderer = CapabilityStatus::new("ok", "chrome found");
        caps.llm = CapabilityStatus::new("disabled", "binary built without the llm feature");

        let status = readiness_status(&caps, DeploymentProfile::Full);
        assert_eq!(status, HealthStatus::Unhealthy);
        assert_eq!(
            readiness_http_status(&status),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
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
            vec!["database", "worker_heartbeat", "embeddings", "search_index"]
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
                "worker_heartbeat",
                "llm",
                "embeddings",
                "nats",
                "search_index",
                "browser_renderer",
            ]
        );
    }
}
