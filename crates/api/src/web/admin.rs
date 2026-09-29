//! Admin handler — GET /admin
//!
//! Covers: admin dashboard with system health, crawl status, POI coverage,
//! recipe performance, queue depths, and data pipeline metrics.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Form,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Extension,
};

use super::PageContext;
use crate::middleware::session::WebSession;
use crate::system_status::{format_age, DATA_FRESH_WITHIN_SECS, WORKER_HEARTBEAT_STALE_AFTER_SECS};
use apex_core::data_state::{DataState, DegradedNotice};
use apex_crawl::sources::{
    all_sources, crawl_source_budget_from_env, scheduler_backlog, source_coverage_summary,
    DeploymentCapabilities, SchedulerBacklog, SourceCoverageSummary,
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

/// One capability-health badge rendered on the admin page. `state` carries the
/// distinct typed label (Disabled and Not configured are rendered differently
/// from Healthy) and `status_class` the semantic colour class.
#[derive(Clone, Debug)]
pub struct CapabilityBadge {
    pub name: String,
    pub state: String,
    pub status_class: String,
    pub detail: String,
}

/// Map a measured capability state to its badge label and colour class.
/// Disabled, Not configured and Not measured are each distinct: none of them
/// may render as Healthy.
pub fn capability_badge(
    state: crate::routes::capabilities::CapabilityState,
) -> (&'static str, &'static str) {
    use crate::routes::capabilities::CapabilityState;
    match state {
        CapabilityState::Healthy => ("Healthy", "apex-text-positive"),
        CapabilityState::Degraded => ("Degraded", "apex-text-warning"),
        CapabilityState::Unavailable => ("Unavailable", "apex-text-danger"),
        CapabilityState::Disabled => ("Disabled", "text-rams-muted"),
        CapabilityState::NotConfigured => ("Not configured", "apex-text-warning"),
        CapabilityState::NotMeasured => ("Not measured", "apex-text-info"),
    }
}

/// Capability badges from the latest published measurement. An empty result
/// (no measurement yet) renders nothing rather than a fabricated Healthy row.
pub fn capability_badges() -> Vec<CapabilityBadge> {
    let Some(capabilities) = crate::system_status::current_capabilities() else {
        return Vec::new();
    };
    const NAMES: [&str; 14] = [
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
    NAMES
        .iter()
        .filter_map(|name| {
            capabilities.capability(name).map(|capability| {
                let (label, status_class) = capability_badge(capability.state());
                CapabilityBadge {
                    name: name.replace('_', " "),
                    state: label.to_string(),
                    status_class: status_class.to_string(),
                    detail: capability.detail.clone(),
                }
            })
        })
        .collect()
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

/// One dead-lettered per-channel notification delivery (admin replay).
#[derive(Clone, Debug)]
pub struct DeliveryDeadLetterItem {
    pub delivery_key: String,
    pub channel: String,
    pub destination: String,
    pub attempts: i32,
    pub error: String,
    pub dead_lettered_at: String,
}

/// One dead-lettered outbox alert event (admin replay).
#[derive(Clone, Debug)]
pub struct OutboxDeadLetterItem {
    pub id: String,
    pub event_type: String,
    pub attempts: i32,
    pub error: String,
    pub dead_lettered_at: String,
}

/// Form for replaying a dead-lettered channel delivery.
#[derive(Debug, serde::Deserialize)]
pub struct DeliveryReplayForm {
    pub delivery_key: String,
}

/// Form for replaying a dead-lettered outbox event.
#[derive(Debug, serde::Deserialize)]
pub struct OutboxReplayForm {
    pub outbox_id: String,
}

const DEAD_LETTER_LIST_LIMIT: i64 = 25;

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
    /// Measured capability states (Disabled/Not configured/Not measured render
    /// distinctly from Healthy).
    pub capability_badges: Vec<CapabilityBadge>,
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
    /// Weighted-fair scheduler backlog for one pass at the configured
    /// `CRAWL_MAX_SOURCES` budget (P0 scheduler quality).
    pub source_backlog: SchedulerBacklog,
    /// Dead-lettered per-channel notification deliveries awaiting replay.
    pub delivery_dead_letters: Vec<DeliveryDeadLetterItem>,
    /// Dead-lettered outbox alert events awaiting replay.
    pub outbox_dead_letters: Vec<OutboxDeadLetterItem>,
    /// Process-wide source-adapter parser health (P0 #26): fetch/parse/success
    /// counters and the parser success rate shown on the admin dashboard.
    pub parser_metrics: apex_crawl::parse_outcome::ParserMetricsSnapshot,

    pub degraded_notice: Option<String>,
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
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web admin page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/admin", unack_state.into_loaded_or(0));

    // Crawl status
    let crawl_state = DataState::from_result(
        store.get_admin_crawl_status().await,
        "get_admin_crawl_status failed (web admin page)",
        |_| false,
    );
    DegradedNotice::capture(&crawl_state, &mut degraded_notice);
    let crawl_statuses: Vec<CrawlStatus> = match &crawl_state {
        DataState::Loaded(cs) => vec![CrawlStatus {
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
        }],
        DataState::Empty | DataState::Degraded { .. } => vec![],
    };

    // Recipe performance
    let recipe_perf_state = DataState::from_result(
        store.get_admin_recipe_performance().await,
        "get_admin_recipe_performance failed (web admin page)",
        |_| false,
    );
    DegradedNotice::capture(&recipe_perf_state, &mut degraded_notice);
    let recipe_performance: Vec<RecipePerformance> = match &recipe_perf_state {
        DataState::Loaded(rp) => rp
            .recipes
            .iter()
            .map(|r| RecipePerformance {
                recipe_id: 0,
                name: r.recipe_code.clone(),
                total_runs: r.fired_count,
                // Expected successes from measured precision only; an
                // unreviewed recipe contributes no fabricated count.
                success_count: r.precision_score.map_or(0, |precision| {
                    ((precision.clamp(0.0, 1.0)) * r.fired_count as f64).round() as i64
                }),
                failure_count: 0,
                avg_duration_ms: 0,
                last_run: r
                    .last_fired
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".into()),
            })
            .collect(),
        DataState::Empty | DataState::Degraded { .. } => vec![],
    };

    // POI coverage
    let poi_cov_state = DataState::from_result(
        store.get_admin_poi_coverage().await,
        "get_admin_poi_coverage failed (web admin page)",
        |_| false,
    );
    DegradedNotice::capture(&poi_cov_state, &mut degraded_notice);
    let poi_coverage: Vec<PoiCoverage> = match &poi_cov_state {
        DataState::Loaded(pc) => {
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
        }
        DataState::Empty | DataState::Degraded { .. } => vec![],
    };

    // Totals
    let total_observations = match &crawl_state {
        DataState::Loaded(cs) => cs.total_fingerprints,
        _ => 0,
    };
    let total_entities = match &poi_cov_state {
        DataState::Loaded(pc) => pc.total_persons,
        _ => 0,
    };

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

    // Audit item 3: an `app_users` row whose role is not one of the four known
    // roles can never authenticate (the login path fails closed instead of
    // defaulting to analyst). Surface it here so an administrator notices and
    // fixes the row instead of debugging a "wrong password" report.
    let role_metric = match store.count_app_users_with_unknown_roles().await {
        Ok(0) => SystemMetric {
            name: "Identity Roles".into(),
            value: "All roles valid".into(),
            status: "ok".into(),
        },
        Ok(count) => SystemMetric {
            name: "Identity Roles".into(),
            value: format!("{count} account(s) with unknown roles — logins rejected"),
            status: "warning".into(),
        },
        Err(error) => {
            tracing::error!("Failed to count app_users with unknown roles: {error}");
            SystemMetric {
                name: "Identity Roles".into(),
                value: "Unavailable".into(),
                status: "error".into(),
            }
        }
    };

    let uptime = fmt_process_uptime();

    // B316: real ingestion panel — observation volume/freshness per source
    // type replaces the hardcoded SEC EDGAR / DNS DB / News Crawler list.
    let observation_sources_state = DataState::from_result(
        store.get_observation_source_stats().await,
        "get_observation_source_stats failed (web admin page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&observation_sources_state, &mut degraded_notice);
    let observation_sources: Vec<SourceItem> = observation_sources_state
        .into_items()
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

    let governance_state = DataState::from_result(
        store.get_admin_llm_governance_overview(10).await,
        "get_admin_llm_governance_overview failed (web admin page)",
        |_| false,
    );
    DegradedNotice::capture(&governance_state, &mut degraded_notice);
    let governance = match &governance_state {
        DataState::Loaded(overview) => Some(overview),
        DataState::Empty | DataState::Degraded { .. } => None,
    };
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

    let (source_coverage, source_backlog) = {
        let registry = all_sources();
        let runtime_states_state = DataState::from_result(
            store.load_source_runtime_states().await,
            "load_source_runtime_states failed (web admin page)",
            Vec::is_empty,
        );
        DegradedNotice::capture(&runtime_states_state, &mut degraded_notice);
        let runtime_states = runtime_states_state.into_items();
        let now = chrono::Utc::now();
        (
            source_coverage_summary(
                &registry,
                &runtime_states,
                &DeploymentCapabilities::from_env(),
                now,
            ),
            scheduler_backlog(
                &registry,
                &runtime_states,
                crawl_source_budget_from_env(),
                now,
            ),
        )
    };

    // Durable notification delivery: dead-lettered rows are operator-replayable.
    let delivery_dead_letters_state = DataState::from_result(
        store
            .list_dead_lettered_notifications(DEAD_LETTER_LIST_LIMIT)
            .await,
        "list_dead_lettered_notifications failed (web admin dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&delivery_dead_letters_state, &mut degraded_notice);
    let delivery_dead_letters: Vec<DeliveryDeadLetterItem> = delivery_dead_letters_state
        .into_items()
        .into_iter()
        .map(|row| DeliveryDeadLetterItem {
            delivery_key: row.delivery_key,
            channel: row.channel,
            destination: row.destination,
            attempts: row.attempts,
            error: row.last_error.unwrap_or_default(),
            dead_lettered_at: row
                .dead_lettered_at
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".to_string()),
        })
        .collect();
    let outbox_dead_letters_state = DataState::from_result(
        store
            .list_dead_lettered_outbox(DEAD_LETTER_LIST_LIMIT)
            .await,
        "list_dead_lettered_outbox failed (web admin dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&outbox_dead_letters_state, &mut degraded_notice);
    let outbox_dead_letters: Vec<OutboxDeadLetterItem> = outbox_dead_letters_state
        .into_items()
        .into_iter()
        .map(|row| OutboxDeadLetterItem {
            id: row.id.to_string(),
            event_type: row.event_type,
            attempts: row.attempts,
            error: row.last_error.unwrap_or_default(),
            dead_lettered_at: row
                .dead_lettered_at
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".to_string()),
        })
        .collect();

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
            role_metric,
        ],
        capability_badges: capability_badges(),
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
        source_backlog,
        delivery_dead_letters,
        outbox_dead_letters,
        parser_metrics: apex_crawl::parse_outcome::PARSER_METRICS.snapshot(),

        degraded_notice,
    };

    super::render_template(&tpl)
}

/// POST /admin/notifications/delivery/replay — requeue a dead-lettered channel
/// delivery with a fresh attempt budget.
pub async fn admin_replay_delivery(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<DeliveryReplayForm>,
) -> Response {
    if !session.can_admin() {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }
    match store
        .replay_dead_lettered_notification(&form.delivery_key)
        .await
    {
        Ok(true) => {
            tracing::info!(
                delivery_key = %form.delivery_key,
                username = %session.username,
                "admin replayed a dead-lettered notification delivery"
            );
            Redirect::to("/admin").into_response()
        }
        Ok(false) => {
            tracing::warn!(
                delivery_key = %form.delivery_key,
                "admin replay found no dead-lettered delivery with that key"
            );
            (
                StatusCode::NOT_FOUND,
                "No dead-lettered delivery with that key",
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(
                delivery_key = %form.delivery_key,
                error = %error,
                "admin replay of a notification delivery failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to replay notification delivery",
            )
                .into_response()
        }
    }
}

/// POST /admin/notifications/outbox/replay — requeue a dead-lettered outbox
/// alert event so the canonical drain retries it.
pub async fn admin_replay_outbox(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<OutboxReplayForm>,
) -> Response {
    if !session.can_admin() {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }
    let outbox_id = match uuid::Uuid::parse_str(form.outbox_id.trim()) {
        Ok(outbox_id) => outbox_id,
        Err(_) => {
            tracing::warn!(outbox_id = %form.outbox_id, "admin replay received an invalid outbox id");
            return (StatusCode::BAD_REQUEST, "Invalid outbox ID").into_response();
        }
    };
    match store.replay_dead_lettered_outbox(outbox_id).await {
        Ok(true) => {
            tracing::info!(
                %outbox_id,
                username = %session.username,
                "admin replayed a dead-lettered outbox event"
            );
            Redirect::to("/admin").into_response()
        }
        Ok(false) => {
            tracing::warn!(
                %outbox_id,
                "admin replay found no dead-lettered outbox event with that id"
            );
            (
                StatusCode::NOT_FOUND,
                "No dead-lettered outbox event with that ID",
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(
                %outbox_id,
                error = %error,
                "admin replay of an outbox event failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to replay outbox event",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::routes::capabilities::CapabilityState;

    #[test]
    fn capability_badges_render_every_state_distinctly() {
        let healthy = capability_badge(CapabilityState::Healthy);
        let disabled = capability_badge(CapabilityState::Disabled);
        let not_configured = capability_badge(CapabilityState::NotConfigured);
        let not_measured = capability_badge(CapabilityState::NotMeasured);

        assert_eq!(healthy.0, "Healthy");
        assert_eq!(disabled.0, "Disabled");
        assert_eq!(not_configured.0, "Not configured");
        assert_eq!(not_measured.0, "Not measured");

        // Disabled must never share the label or colour of Healthy, and the
        // three non-healthy states must not collapse into one another.
        assert_ne!(disabled.0, healthy.0);
        assert_ne!(disabled.1, healthy.1);
        assert_ne!(disabled.0, not_configured.0);
        assert_ne!(not_configured.0, not_measured.0);
    }

    #[test]
    fn capability_badges_are_absent_until_measured() {
        // No published snapshot in this test process: an unmeasured platform
        // renders no badge rather than a fabricated Healthy row.
        assert!(capability_badges().is_empty());
    }
}
