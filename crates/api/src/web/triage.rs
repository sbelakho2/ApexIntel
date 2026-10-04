//! Triage web handlers — HTML pages for the AI Triage Engine.
//!
//! These handlers render server-side pages using Askama + HTMX, following the
//! same pattern as [`crate::web::insights`] and [`crate::web::warnings`].

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Form,
};
use serde::Deserialize;
use uuid::Uuid;

use apex_core::data_state::{DataState, DegradedNotice};
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
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub items: Vec<TriageQueueItem>,
    pub stats: QueueStats,
    pub current_status: String,
    pub page: u32,
    pub total_pages: u32,
    pub thresholds: TriageThresholds,
    /// Rendered when the triage queue query failed.
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/triage_queue.html")]
pub(crate) struct TriageQueuePartial {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub items: Vec<TriageQueueItem>,
    pub stats: QueueStats,
    pub current_status: String,
    pub page: u32,
    pub total_pages: u32,
    pub thresholds: TriageThresholds,
    /// Rendered when the triage queue query failed.
    pub degraded_notice: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/triage_detail.html")]
pub(crate) struct TriageDetailPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub item: TriageQueueItem,
    pub thresholds: TriageThresholds,
    // Pre-computed display values (Askama 0.12 compatibility). Dimension
    // scores are `None` when the row stores no dimensions, so the template can
    // render unknown instead of five fabricated zeros.
    pub score_pct: i64,
    pub urgency_pct: Option<i64>,
    pub impact_pct: Option<i64>,
    pub actionability_pct: Option<i64>,
    pub novelty_pct: Option<i64>,
    pub confidence_pct: Option<i64>,
}

impl QueueStats {
    fn band_pct(count: u64, total: u64) -> i64 {
        if total == 0 {
            0
        } else {
            ((count as f64 / total as f64) * 100.0).round() as i64
        }
    }
    pub(crate) fn critical_pct(&self) -> i64 {
        Self::band_pct(self.critical_count, self.total)
    }
    pub(crate) fn high_pct(&self) -> i64 {
        Self::band_pct(self.high_count, self.total)
    }
    pub(crate) fn medium_pct(&self) -> i64 {
        Self::band_pct(self.medium_count, self.total)
    }
    pub(crate) fn low_pct(&self) -> i64 {
        Self::band_pct(self.low_count, self.total)
    }
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

/// A stored 0–1 triage dimension as a percentage, or `None` when the row has
/// no stored dimension. Never defaulted to 0, which would show an unmeasured
/// dimension as a measured zero.
fn dimension_pct(value: Option<f64>) -> Option<i64> {
    value.map(|v| (v * 100.0) as i64)
}

fn page_from_ctx(ctx: &PageContext) -> (String, String, i64, String, bool, bool) {
    (
        ctx.current_path.clone(),
        ctx.username.clone(),
        ctx.warning_count,
        ctx.theme.clone(),
        ctx.can_admin,
        ctx.can_write,
    )
}

async fn fetch_stats(queue: &TriageQueue) -> Result<QueueStats, String> {
    match queue.stats().await {
        Ok(s) => Ok(QueueStats {
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
        }),
        Err(error) => {
            let error_id = apex_core::data_state::new_incident_id();
            tracing::error!(incident_id = %error_id, "failed to fetch triage stats: {error}");
            Err(error_id)
        }
    }
}

fn empty_stats() -> QueueStats {
    QueueStats {
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
    let mut degraded_notice: Option<String> = None;

    let items_state = DataState::from_result(
        queue.list(status_filter.clone(), per_page, offset).await,
        "failed to list triage queue",
        |items| items.is_empty(),
    );
    DegradedNotice::capture(&items_state, &mut degraded_notice);
    let items = items_state.into_items();

    let total_state = DataState::from_result(
        queue.count(status_filter.clone()).await,
        "failed to count triage queue",
        |_| false,
    );
    DegradedNotice::capture(&total_state, &mut degraded_notice);
    let total = total_state.into_loaded_or(0);

    let total_pages = if per_page > 0 {
        (total as f64 / per_page as f64).ceil() as u32
    } else {
        0
    };

    let stats = match fetch_stats(&queue).await {
        Ok(stats) => stats,
        Err(error_id) => {
            DegradedNotice::capture(&DataState::<()>::degraded(error_id), &mut degraded_notice);
            empty_stats()
        }
    };

    let current_status = params.status.unwrap_or_else(|| "all".to_string());

    // Nav badge shows unacknowledged warnings, not the triage queue size.
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web triage page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let pctx = PageContext::from_session(&session, "/triage", unack_state.into_loaded_or(0));
    let (current_path, username, warning_count, theme, can_admin, can_write) = page_from_ctx(&pctx);

    if is_htmx_request(&headers) {
        let partial = TriageQueuePartial {
            current_path,
            username,
            warning_count,
            theme,
            can_admin,
            can_write,
            status_strip: pctx.status_strip.clone(),
            degraded_notice: degraded_notice.clone(),
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
            can_write,
            status_strip: pctx.status_strip.clone(),
            degraded_notice,
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
            let (current_path, username, warning_count, theme, can_admin, can_write) =
                page_from_ctx(&pctx);
            let dims = item.dimensions.as_ref();
            render_template(&TriageDetailPage {
                current_path,
                username,
                warning_count,
                theme,
                can_admin,
                can_write,
                status_strip: pctx.status_strip.clone(),
                score_pct: (item.composite_score * 100.0) as i64,
                urgency_pct: dimension_pct(dims.map(|d| d.urgency)),
                impact_pct: dimension_pct(dims.map(|d| d.impact)),
                actionability_pct: dimension_pct(dims.map(|d| d.actionability)),
                novelty_pct: dimension_pct(dims.map(|d| d.novelty)),
                confidence_pct: dimension_pct(dims.map(|d| d.confidence)),
                item,
                thresholds,
            })
        }
        Ok(None) => {
            let pctx = PageContext::from_session(&session, "/triage", 0);
            crate::web::errors::not_found_for(&pctx, "/triage")
        }
        Err(e) => {
            tracing::error!("Failed to fetch triage item {id}: {e}");
            let pctx = PageContext::from_session(&session, "/triage", 0);
            // No correlation id at this call site: `internal_error_for`
            // generates a unique incident id so one is always shown (#152).
            crate::web::errors::internal_error_for(&pctx, "")
        }
    }
}

/// POST /triage/:id/acknowledge — acknowledge via HTMX then redirect.
pub async fn acknowledge_triage_html(
    headers: HeaderMap,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.acknowledge(id).await {
        Ok(_) => crate::web::redirect_or_hx_redirect(&headers, &format!("/triage/{id}")),
        Err(e) => {
            tracing::error!("Failed to acknowledge triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to acknowledge").into_response()
        }
    }
}

/// POST /triage/:id/resolve — resolve via HTMX then redirect.
pub async fn resolve_triage_html(
    headers: HeaderMap,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.resolve(id).await {
        Ok(_) => crate::web::redirect_or_hx_redirect(&headers, &format!("/triage/{id}")),
        Err(e) => {
            tracing::error!("Failed to resolve triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to resolve").into_response()
        }
    }
}

/// POST /triage/:id/dismiss — dismiss via HTMX then redirect.
pub async fn dismiss_triage_html(
    headers: HeaderMap,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(store);

    match queue.dismiss(id).await {
        Ok(_) => crate::web::redirect_or_hx_redirect(&headers, &format!("/triage/{id}")),
        Err(e) => {
            tracing::error!("Failed to dismiss triage item {id}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to dismiss").into_response()
        }
    }
}

/// POST /triage/:id/override — override score via HTMX then redirect.
pub async fn override_triage_html(
    headers: HeaderMap,
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
        Ok(_) => crate::web::redirect_or_hx_redirect(&headers, &format!("/triage/{id}")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_status::StatusStrip;

    fn empty_stats() -> QueueStats {
        super::empty_stats()
    }

    #[test]
    fn degraded_queue_renders_marker_instead_of_no_results() {
        let page = TriageQueuePage {
            current_path: "/triage".into(),
            can_admin: false,
            can_write: false,
            username: "analyst".into(),
            warning_count: 0,
            theme: String::new(),
            status_strip: StatusStrip::unknown(),
            items: vec![],
            stats: empty_stats(),
            current_status: "all".into(),
            page: 1,
            total_pages: 0,
            thresholds: TriageThresholds::default(),
            degraded_notice: Some(
                "Data unavailable — query failed at 14:03 UTC · incident inc-queue777".into(),
            ),
        };
        let html = page.render().expect("triage queue renders");

        assert!(html.contains("incident inc-queue777"));
        assert!(html.contains("data-degraded=\"true\""));
        assert!(!html.contains("No triage items found."));
    }

    #[test]
    fn empty_queue_still_renders_no_results_when_healthy() {
        let page = TriageQueuePage {
            current_path: "/triage".into(),
            can_admin: false,
            can_write: false,
            username: "analyst".into(),
            warning_count: 0,
            theme: String::new(),
            status_strip: StatusStrip::unknown(),
            items: vec![],
            stats: empty_stats(),
            current_status: "all".into(),
            page: 1,
            total_pages: 0,
            thresholds: TriageThresholds::default(),
            degraded_notice: None,
        };
        let html = page.render().expect("triage queue renders");

        assert!(html.contains("No triage items found."));
        assert!(!html.contains("data-degraded=\"true\""));
    }

    /// A row without stored dimensions must not render five zeros as measured
    /// dimension scores; absence stays absent.
    #[test]
    fn missing_dimensions_are_unknown_not_zero() {
        assert_eq!(dimension_pct(None), None);
        assert_eq!(dimension_pct(Some(0.0)), Some(0));
        assert_eq!(dimension_pct(Some(0.42)), Some(42));
    }
}
