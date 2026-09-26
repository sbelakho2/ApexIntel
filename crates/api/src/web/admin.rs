//! Admin handler — GET /admin
//!
//! Covers: admin dashboard with system health, crawl status, POI coverage,
//! recipe performance, queue depths, and data pipeline metrics.

use std::sync::Arc;

use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};

use super::PageContext;
use crate::middleware::session::WebSession;
use crate::system_status::{format_age, DATA_FRESH_WITHIN_SECS, WORKER_HEARTBEAT_STALE_AFTER_SECS};
use apex_core::data_state::DataState;
use apex_crawl::sources::{
    all_sources, source_coverage_summary, DeploymentCapabilities, SourceCoverageSummary,
};
use apex_store::postgres::{PgStore, WarningListFilters};
use apex_store::tantivy_index::SearchIndex;

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CrawlStatus {
    pub source: String,
    pub status: String, // "running" | "idle" | "error"
    pub last_run: String,
    pub items_crawled: i64,
    pub error_count: i64,
    pub next_run: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PoiCoverage {
    pub category: String,
    pub total: i64,
    pub covered: i64,
    pub coverage_pct: f64,
}

#[derive(Clone, Debug)]
pub struct RecipePerformance {
    pub recipe_id: i64,
    pub name: String,
    pub total_runs: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub avg_duration_ms: i64,
    pub last_run: String,
}

#[derive(Clone, Debug)]
pub struct SystemMetric {
    pub name: String,
    pub value: String,
    pub status: String, // "ok" | "warning" | "error"
}

#[derive(Clone, Debug)]
pub struct QueueInfo {
    pub name: String,
    pub depth: i64,
    pub processing: i64,
    pub failed: i64,
}

#[derive(Clone, Debug)]
pub struct PromptVersionItem {
    pub prompt_id: String,
    pub version: String,
    pub workflow: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct WorkflowRunItem {
    pub workflow: String,
    pub prompt_label: String,
    pub model_name: String,
    pub gate_status: String,
    pub validation_issue_count: usize,
    pub duration_ms: i64,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct ImprovementRunItem {
    pub run_kind: String,
    pub run_key: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct TrainingDatasetItem {
    pub dataset_name: String,
    pub dataset_version: String,
    pub source_run_kind: String,
    pub example_count: i64,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AdminStats {
    /// Live sqlx pool size for this API process.
    pub pool_size: u32,
}

#[derive(Clone, Debug)]
pub struct SourceItem {
    pub name: String,
    pub status: String,
    pub status_class: String,
    pub records: i64,
    pub last_ingested: String,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/admin.html")]
pub struct AdminPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub crawl_statuses: Vec<CrawlStatus>,
    pub poi_coverage: Vec<PoiCoverage>,
    pub recipe_performance: Vec<RecipePerformance>,
    pub system_metrics: Vec<SystemMetric>,
    pub queues: Vec<QueueInfo>,
    pub prompt_versions: Vec<PromptVersionItem>,
    pub workflow_runs: Vec<WorkflowRunItem>,
    pub improvement_runs: Vec<ImprovementRunItem>,
    pub training_datasets: Vec<TrainingDatasetItem>,
    pub db_size: String,
    pub uptime: String,
    pub total_observations: i64,
    pub total_entities: i64,
    pub stats: AdminStats,
    /// Real per-source ingestion stats (B316).
    pub observation_sources: Vec<SourceItem>,
    /// Declared vs operational crawl-source coverage (P0 #25). Only
    /// operational sources count toward the product's source count.
    pub source_coverage: SourceCoverageSummary,
}

fn fmt_ts(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%Y-%m-%d %H:%M").to_string()
}

/// Process start time — powers the real uptime tile (B316).
static PROCESS_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn fmt_process_uptime() -> String {
    let start = PROCESS_START.get_or_init(std::time::Instant::now);
    let secs = start.elapsed().as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn validation_issue_count(value: &serde_json::Value) -> usize {
    value.as_array().map(|items| items.len()).unwrap_or(0)
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /admin — admin system dashboard.
///
/// The `/admin` route is also guarded by `require_web_admin` at the router
/// level; this check is defense in depth for direct handler invocation.
pub async fn admin_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Extension(search_index): Extension<Arc<SearchIndex>>,
) -> Response {
    if !session.can_admin() {
        tracing::warn!(
            username = %session.username,
            role = %session.role.as_str(),
            "admin page access denied: admin role required"
        );
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/admin", unack);

    // Crawl status
    let crawl = store.get_admin_crawl_status().await.ok();
    let crawl_statuses: Vec<CrawlStatus> = if let Some(ref cs) = crawl {
        vec![CrawlStatus {
            source: "Web Crawler".into(),
            status: if cs.latest_crawl_ts.is_some() {
                "idle".into()
            } else {
                "unknown".into()
            },
            last_run: cs
                .latest_crawl_ts
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".into()),
            items_crawled: cs.total_fingerprints,
            error_count: 0,
            next_run: None,
        }]
    } else {
        vec![]
    };

    // Recipe performance
    let recipe_perf = store.get_admin_recipe_performance().await.ok();
    let recipe_performance: Vec<RecipePerformance> = recipe_perf
        .as_ref()
        .map(|rp| {
            rp.recipes
                .iter()
                .map(|r| RecipePerformance {
                    recipe_id: 0,
                    name: r.recipe_code.clone(),
                    total_runs: r.fired_count,
                    success_count: ((r.precision_score.clamp(0.0, 1.0)) * r.fired_count as f64)
                        .round() as i64,
                    failure_count: 0,
                    avg_duration_ms: 0,
                    last_run: r
                        .last_fired
                        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "—".into()),
                })
                .collect()
        })
        .unwrap_or_default();

    // POI coverage
    let poi_cov = store.get_admin_poi_coverage().await.ok();
    let poi_coverage: Vec<PoiCoverage> = if let Some(ref pc) = poi_cov {
        let covered_pct = if pc.total_persons > 0 {
            (pc.with_artifacts as f64 / pc.total_persons as f64) * 100.0
        } else {
            0.0
        };
        vec![PoiCoverage {
            category: "Persons with artifacts".into(),
            total: pc.total_persons,
            covered: pc.with_artifacts,
            coverage_pct: covered_pct,
        }]
    } else {
        vec![]
    };

    // Totals
    let total_observations = crawl.as_ref().map(|c| c.total_fingerprints).unwrap_or(0);
    let total_entities = poi_cov.as_ref().map(|p| p.total_persons).unwrap_or(0);

    // B316: real database/process statistics. Each system metric is measured
    // from a probe instead of being hard-coded to "Connected"/"Ready".
    let db_size_state = DataState::from_result(
        store.get_database_size().await,
        "failed to fetch database size",
        |_| false,
    );
    let db_size = match &db_size_state {
        DataState::Loaded(size) => size.clone(),
        DataState::Empty | DataState::Degraded { .. } => "—".to_string(),
    };
    let database_metric = SystemMetric {
        name: "Database".into(),
        value: if db_size_state.is_loaded() {
            "Connected".into()
        } else {
            "Unreachable".into()
        },
        status: if db_size_state.is_loaded() {
            "ok".into()
        } else {
            "error".into()
        },
    };

    let docs = search_index.num_docs();
    let search_index_metric = SystemMetric {
        name: "Search Index".into(),
        value: format!("{docs} documents"),
        status: if docs > 0 {
            "ok".into()
        } else {
            "warning".into()
        },
    };

    let heartbeat_state = DataState::from_result(
        store.latest_service_heartbeat("worker").await,
        "failed to fetch worker heartbeat",
        |row| row.is_none(),
    );
    let worker_metric = match &heartbeat_state {
        DataState::Loaded(Some(row)) => {
            let age = chrono::Utc::now().signed_duration_since(row.last_seen_at);
            let stale = age.num_seconds() > WORKER_HEARTBEAT_STALE_AFTER_SECS;
            SystemMetric {
                name: "Worker Heartbeat".into(),
                value: format!("{} ago", format_age(age)),
                status: if stale { "warning".into() } else { "ok".into() },
            }
        }
        DataState::Loaded(None) | DataState::Empty => SystemMetric {
            name: "Worker Heartbeat".into(),
            value: "No heartbeat recorded".into(),
            status: "warning".into(),
        },
        DataState::Degraded { .. } => SystemMetric {
            name: "Worker Heartbeat".into(),
            value: "Unavailable".into(),
            status: "error".into(),
        },
    };

    let freshness_state = DataState::from_result(
        store.newest_observation_ts().await,
        "failed to fetch newest observation timestamp",
        |ts| ts.is_none(),
    );
    let freshness_metric = match &freshness_state {
        DataState::Loaded(Some(ts)) => {
            let age = chrono::Utc::now().signed_duration_since(*ts);
            let stale = age.num_seconds() > DATA_FRESH_WITHIN_SECS;
            SystemMetric {
                name: "Data Freshness".into(),
                value: format!("newest {} ago", format_age(age)),
                status: if stale { "warning".into() } else { "ok".into() },
            }
        }
        DataState::Loaded(None) | DataState::Empty => SystemMetric {
            name: "Data Freshness".into(),
            value: "No observations".into(),
            status: "warning".into(),
        },
        DataState::Degraded { .. } => SystemMetric {
            name: "Data Freshness".into(),
            value: "Unavailable".into(),
            status: "error".into(),
        },
    };

    let uptime = fmt_process_uptime();

    // B316: real ingestion panel — observation volume/freshness per source
    // type replaces the hardcoded SEC EDGAR / DNS DB / News Crawler list.
    let observation_sources: Vec<SourceItem> = store
        .get_observation_source_stats()
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to fetch observation source stats: {e}");
            vec![]
        })
        .into_iter()
        .map(|(kind, records, latest)| {
            let stale = latest
                .as_ref()
                .is_some_and(|ts| chrono::Utc::now() - *ts > chrono::Duration::hours(48));
            let (status, status_class) = if stale {
                ("stale".to_string(), "apex-text-warning".to_string())
            } else {
                ("active".to_string(), "apex-text-positive".to_string())
            };
            SourceItem {
                name: kind,
                status,
                status_class,
                records,
                last_ingested: latest
                    .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".to_string()),
            }
        })
        .collect();

    let governance = store.get_admin_llm_governance_overview(10).await.ok();
    let prompt_versions = governance
        .as_ref()
        .map(|overview| {
            overview
                .prompt_versions
                .iter()
                .map(|record| PromptVersionItem {
                    prompt_id: record.prompt_id.clone(),
                    version: record.version.clone(),
                    workflow: record.workflow.clone(),
                    created_at: fmt_ts(record.created_at),
                })
                .collect()
        })
        .unwrap_or_default();
    let workflow_runs = governance
        .as_ref()
        .map(|overview| {
            overview
                .workflow_runs
                .iter()
                .map(|record| WorkflowRunItem {
                    workflow: record.workflow.clone(),
                    prompt_label: format!("{} {}", record.prompt_id, record.prompt_version),
                    model_name: record.model_name.clone(),
                    gate_status: if record.quality_gate_passed {
                        "passed".to_string()
                    } else {
                        "failed".to_string()
                    },
                    validation_issue_count: validation_issue_count(&record.validation_issues),
                    duration_ms: record.duration_ms,
                    created_at: fmt_ts(record.created_at),
                })
                .collect()
        })
        .unwrap_or_default();
    let improvement_runs = governance
        .as_ref()
        .map(|overview| {
            overview
                .improvement_runs
                .iter()
                .map(|record| ImprovementRunItem {
                    run_kind: record.run_kind.clone(),
                    run_key: record.run_key.clone(),
                    created_at: fmt_ts(record.created_at),
                })
                .collect()
        })
        .unwrap_or_default();
    let training_datasets = governance
        .as_ref()
        .map(|overview| {
            overview
                .training_datasets
                .iter()
                .map(|record| TrainingDatasetItem {
                    dataset_name: record.dataset_name.clone(),
                    dataset_version: record.dataset_version.clone(),
                    source_run_kind: record.source_run_kind.clone(),
                    example_count: record.example_count,
                    created_at: fmt_ts(record.created_at),
                })
                .collect()
        })
        .unwrap_or_default();

    let source_coverage = {
        let registry = all_sources();
        let runtime_states = store
            .load_source_runtime_states()
            .await
            .unwrap_or_else(|error| {
                tracing::error!("Failed to fetch source runtime state: {error}");
                vec![]
            });
        source_coverage_summary(
            &registry,
            &runtime_states,
            &DeploymentCapabilities::from_env(),
            chrono::Utc::now(),
        )
    };

    let tpl = AdminPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        crawl_statuses,
        poi_coverage,
        recipe_performance,
        system_metrics: vec![
            database_metric,
            search_index_metric,
            worker_metric,
            freshness_metric,
        ],
        queues: vec![],
        prompt_versions,
        workflow_runs,
        improvement_runs,
        training_datasets,
        db_size,
        uptime,
        total_observations,
        total_entities,
        stats: AdminStats {
            pool_size: store.pool.size(),
        },
        observation_sources,
        source_coverage,
    };

    super::render_template(&tpl)
}
