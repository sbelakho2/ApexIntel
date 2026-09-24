//! Triage web handlers — HTML pages for the AI Triage Engine.
//!
//! These handlers render server-side pages using Askama + HTMX, following the
//! same pattern as [`crate::web::insights`] and [`crate::web::warnings`].

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    Extension, Form,
};
use serde::Deserialize;
use uuid::Uuid;

use apex_core::triage::{TriageQueueItem, TriageStatus, TriageThresholds};
use apex_store::postgres::{PgStore, WarningListFilters};
use apex_triage::TriageQueue;

use crate::middleware::session::WebSession;
use crate::web::{is_htmx_request, render_template, PageContext};

// ─── Query parameters ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TriageListQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct OverrideForm {
    pub score: f64,
    pub reason: Option<String>,
}

// ─── Template data ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/triage_queue.html")]
pub(crate) struct TriageQueuePage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub items: Vec<TriageQueueItem>,
    pub stats: QueueStats,
    pub current_status: String,
    pub page: u32,
    pub total_pages: u32,
    pub thresholds: TriageThresholds,
}

#[derive(Template)]
#[template(path = "pages/triage_queue.html")]
pub(crate) struct TriageQueuePartial {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub items: Vec<TriageQueueItem>,
    pub stats: QueueStats,
    pub current_status: String,
    pub page: u32,
    pub total_pages: u32,
    pub thresholds: TriageThresholds,
}

#[derive(Template)]
#[template(path = "pages/triage_detail.html")]
pub(crate) struct TriageDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub item: TriageQueueItem,
    pub thresholds: TriageThresholds,
    // Pre-computed display values (Askama 0.12 compatibility)
    pub score_pct: i64,
    pub urgency_pct: i64,
    pub impact_pct: i64,
    pub actionability_pct: i64,
    pub novelty_pct: i64,
    pub confidence_pct: i64,
}

pub(crate) struct QueueStats {
    pub total: u64,
    pub pending: u64,
    pub triaged: u64,
    pub acknowledged: u64,
    pub resolved: u64,
    pub dismissed: u64,
    pub critical_count: u64,
    pub high_count: u64,
    pub medium_count: u64,
    pub low_count: u64,
    pub override_rate: f64,
    pub resolution_rate: f64,
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn make_queue(pool: Arc<PgStore>) -> TriageQueue {
    TriageQueue::new(pool.pool.clone())
}

fn parse_status(s: Option<&str>) -> Option<TriageStatus> {
    s.map(TriageStatus::from_str)
}

fn page_from_ctx(ctx: &PageContext) -> (String, String, i64, String, bool) {
    (
        ctx.current_path.clone(),
        ctx.username.clone(),
        ctx.warning_count,
        ctx.theme.clone(),
        ctx.can_admin,
    )
}

async fn fetch_stats(queue: &TriageQueue) -> QueueStats {
    match queue.stats().await {
        Ok(s) => QueueStats {
            total: s.total,
            pending: s.pending,
            triaged: s.triaged,
            acknowledged: s.acknowledged,
            resolved: s.resolved,
            dismissed: s.dismissed,
            critical_count: s.critical_count,
            high_count: s.high_count,
            medium_count: s.medium_count,
            low_count: s.low_count,
            override_rate: s.override_rate,
            resolution_rate: s.resolution_rate,
        },
        Err(_) => QueueStats {
            total: 0,
            pending: 0,
            triaged: 0,
            acknowledged: 0,
            resolved: 0,
            dismissed: 0,
            critical_count: 0,
            high_count: 0,
            medium_count: 0,
            low_count: 0,
            override_rate: 0.0,
            resolution_rate: 0.0,
        },
    }
}

// ─── Handlers ─────────────────────────────────────────────────────────────

/// GET /triage — main triage queue page.
pub async fn list_triage(
    Extension(store): Extension<Arc<PgStore>>,
    Extension(session): Extension<WebSession>,
    headers: HeaderMap,
    Query(params): Query<TriageListQuery>,
) -> impl IntoResponse {
    let queue = make_queue(store.clone());
    let thresholds = TriageThresholds::default();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = 50usize;
    let offset = ((page - 1) * per_page as u32) as u64;

    let status_filter = parse_status(params.status.as_deref());
    let items = queue
        .list(status_filter.clone(), per_page, offset)
        .await
        .unwrap_or_default();
    let total = queue.count(status_filter.clone()).await.unwrap_or(0);
    let total_pages = if per_page > 0 {
        (total as f64 / per_page as f64).ceil() as u32
    } else {
        0
    };
    let stats = fetch_stats(&queue).await;

    let current_status = params.status.unwrap_or_else(|| "all".to_string());

    // Nav badge shows unacknowledged warnings, not the triage queue size.
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let pctx = PageContext::from_session(&session, "/triage", unack);
    let (current_path, username, warning_count, theme, can_admin) = page_from_ctx(&pctx);

    if is_htmx_request(&headers) {
        let partial = TriageQueuePartial {
            current_path,
            username,
            warning_count,
            theme,
            can_admin,
            items,
            stats,
            current_status,
            page,
            total_pages,
            thresholds,
        };
        render_template(&partial)
    } else {
        let page_data = TriageQueuePage {
            current_path,
            username,
            warning_count,
            theme,
            can_admin,
            items,
            stats,
            current_status,
            page,
            total_pages,
            thresholds,
        };
        render_template(&page_data)
    }
}

/// GET /triage/:id — triage item detail page.
pub async fn get_triage_item(
    Extension(store): Extension<Arc<PgStore>>,
    Extension(session): Extension<WebSession>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);
    let thresholds = TriageThresholds::default();

    match queue.get_by_id(id).await {
        Ok(Some(item)) => {
            let pctx = PageContext::from_session(&session, &format!("/triage/{}", id), 0);
            let (current_path, username, warning_count, theme, can_admin) = page_from_ctx(&pctx);
            let dims = item.dimensions.as_ref();
            render_template(&TriageDetailPage {
                current_path,
                username,
                warning_count,
                theme,
                can_admin,
                score_pct: (item.composite_score * 100.0) as i64,
                urgency_pct: (dims.map(|d| d.urgency).unwrap_or(0.0) * 100.0) as i64,
                impact_pct: (dims.map(|d| d.impact).unwrap_or(0.0) * 100.0) as i64,
                actionability_pct: (dims.map(|d| d.actionability).unwrap_or(0.0) * 100.0) as i64,
                novelty_pct: (dims.map(|d| d.novelty).unwrap_or(0.0) * 100.0) as i64,
                confidence_pct: (dims.map(|d| d.confidence).unwrap_or(0.0) * 100.0) as i64,
                item,
                thresholds,
            })
        }
        Ok(None) => {
            let pctx = PageContext::from_session(&session, "/triage", 0);
            crate::web::errors::not_found_with_context(
                &pctx.username,
                "/triage",
                pctx.warning_count,
            )
        }
        Err(e) => {
            tracing::error!("Failed to fetch triage item {id}: {e}");
            let pctx = PageContext::from_session(&session, "/triage", 0);
            crate::web::errors::internal_error_with_context(
                &pctx.username,
                pctx.warning_count,
                &e.to_string(),
                "",
            )
        }
    }
}

/// POST /triage/:id/acknowledge — acknowledge via HTMX then redirect.
pub async fn acknowledge_triage_html(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.acknowledge(id).await {
        Ok(_) => Redirect::to(&format!("/triage/{}", id)).into_response(),
        Err(e) => {
            tracing::error!("Failed to acknowledge triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to acknowledge").into_response()
        }
    }
}

/// POST /triage/:id/resolve — resolve via HTMX then redirect.
pub async fn resolve_triage_html(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.resolve(id).await {
        Ok(_) => Redirect::to(&format!("/triage/{}", id)).into_response(),
        Err(e) => {
            tracing::error!("Failed to resolve triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to resolve").into_response()
        }
    }
}

/// POST /triage/:id/dismiss — dismiss via HTMX then redirect.
pub async fn dismiss_triage_html(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.dismiss(id).await {
        Ok(_) => Redirect::to(&format!("/triage/{}", id)).into_response(),
        Err(e) => {
            tracing::error!("Failed to dismiss triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to dismiss").into_response()
        }
    }
}

/// POST /triage/:id/override — override score via HTMX then redirect.
pub async fn override_triage_html(
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
    Form(form): Form<OverrideForm>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    if !(0.0..=1.0).contains(&form.score) {
        return (StatusCode::BAD_REQUEST, "Score must be between 0.0 and 1.0").into_response();
    }

    match queue
        .override_score(id, form.score, form.reason.as_deref().unwrap_or(""))
        .await
    {
        Ok(_) => Redirect::to(&format!("/triage/{}", id)).into_response(),
        Err(e) => {
            tracing::error!("Failed to override triage item {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to override score",
            )
                .into_response()
        }
    }
}
