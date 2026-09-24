//! `/api/health/capabilities` — capability health derived from real probes.
//!
//! Every value is measured rather than assumed: the LLM entry reflects the
//! compiled feature set, NATS/embeddings/database entries are live probes, the
//! worker heartbeat reads `service_heartbeats.last_seen_at` (migration 049),
//! and crawl freshness reads the newest `observations.ts_utc`.

use std::time::Duration as StdDuration;

use chrono::Utc;
use serde::Serialize;

use apex_store::postgres::{PgStore, ServiceHeartbeatRow};
use apex_store::tantivy_index::SearchIndex;

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
}

/// Run every capability probe. Results are measured per call; nothing is
/// cached so `/api/health/capabilities` never reports stale health.
pub async fn probe_capabilities(
    pool: &sqlx::PgPool,
    search_index: &SearchIndex,
    nats_url: Option<&str>,
) -> Capabilities {
    let store = PgStore::from_pool(pool.clone());

    let database = probe_database(pool).await;
    let embeddings = probe_embeddings(pool).await;

    let docs = search_index.num_docs();
    let search_index_status = CapabilityStatus::new(
        "ok",
        format!("{docs} documents indexed in the in-process search index"),
    );

    let worker_heartbeat = probe_worker_heartbeat(&store).await;
    let crawl_freshness = probe_crawl_freshness(&store).await;
    let nats = probe_nats(nats_url).await;
    let browser_renderer = probe_browser_renderer();
    let llm = probe_llm();

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
                 WHERE table_name = 'observations' AND column_name = 'embedding'
             )"#,
    )
    .fetch_one(pool)
    .await;

    match probe {
        Ok(true) => CapabilityStatus::new(
            "ok",
            "pgvector extension and observations.embedding present",
        ),
        Ok(false) => CapabilityStatus::new(
            "degraded",
            "pgvector extension or observations.embedding column missing",
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
}
