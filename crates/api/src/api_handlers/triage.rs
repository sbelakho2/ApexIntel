//! Triage Queue API handlers — REST endpoints for the AI Triage Engine.
//!
//! These handlers provide programmatic access to the triage queue, including
//! listing, scoring overrides, status changes, and aggregate statistics.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use uuid::Uuid;

use apex_core::triage::{TriageItemType, TriageStatus};
use apex_store::postgres::PgStore;
use apex_triage::TriageQueue;

use crate::ApiAuthContext;
use crate::AppState;
use apex_api::responses::{error_response, success, ApiError, PagedResponse};

// ─── Query parameters ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ListTriageQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OverrideScorePayload {
    pub score: f64,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BatchScorePayload {
    pub scores: Vec<BatchScoreItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BatchScoreItem {
    pub id: Uuid,
    pub urgency: f64,
    pub impact: f64,
    pub actionability: f64,
    pub novelty: f64,
    pub confidence: f64,
}

// ─── Response types ───────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub(crate) struct TriageItemResponse {
    pub id: Uuid,
    pub item_type: String,
    pub source_id: String,
    pub title: String,
    pub description: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub static_severity: Option<String>,
    pub dimensions: Option<DimensionScores>,
    pub composite_score: Option<f64>,
    pub is_overridden: bool,
    pub override_score: Option<f64>,
    pub score_band: String,
    pub score_band_color: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub triaged_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DimensionScores {
    pub urgency: f64,
    pub impact: f64,
    pub actionability: f64,
    pub novelty: f64,
    pub confidence: f64,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct TriageStatsResponse {
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
    pub avg_composite: f64,
    pub override_rate: f64,
    pub resolution_rate: f64,
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn item_to_response(item: apex_core::triage::TriageQueueItem) -> TriageItemResponse {
    let dimensions = item.dimensions.as_ref().map(|d| DimensionScores {
        urgency: d.urgency,
        impact: d.impact,
        actionability: d.actionability,
        novelty: d.novelty,
        confidence: d.confidence,
    });

    TriageItemResponse {
        id: item.id,
        item_type: item.item_type.as_str().to_string(),
        source_id: item.source_id,
        title: item.title,
        description: Some(item.description),
        entity_id: item.entity_id.map(|id| id.to_string()),
        entity_name: item.entity_name,
        static_severity: item.static_severity,
        dimensions,
        composite_score: Some(item.composite_score),
        is_overridden: item.is_overridden,
        override_score: item.override_score,
        score_band: item.score_band,
        score_band_color: item.score_band_color,
        status: item.status.as_str().to_string(),
        created_at: item.created_at,
        triaged_at: item.triaged_at,
    }
}

fn stats_to_response(stats: apex_core::triage::TriageStats) -> TriageStatsResponse {
    TriageStatsResponse {
        total: stats.total,
        pending: stats.pending,
        triaged: stats.triaged,
        acknowledged: stats.acknowledged,
        resolved: stats.resolved,
        dismissed: stats.dismissed,
        critical_count: stats.critical_count,
        high_count: stats.high_count,
        medium_count: stats.medium_count,
        low_count: stats.low_count,
        avg_composite: stats.avg_composite,
        override_rate: stats.override_rate,
        resolution_rate: stats.resolution_rate,
    }
}

fn make_queue(pool: Arc<PgStore>) -> TriageQueue {
    TriageQueue::new(pool.pool.clone())
}

#[allow(dead_code)]
fn parse_item_type(s: Option<&str>) -> Option<TriageItemType> {
    s.map(TriageItemType::from_str)
}

fn parse_status(s: Option<&str>) -> Option<TriageStatus> {
    s.map(TriageStatus::from_str)
}

// ─── Handlers ─────────────────────────────────────────────────────────────

/// GET /api/triage — list triage queue items.
pub(crate) async fn list_triage(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Query(params): Query<ListTriageQuery>,
) -> impl IntoResponse {
    let _start = Instant::now();
    let queue = make_queue(state.store.clone());
    let threshold = apex_core::triage::TriageThresholds::default();

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 200);
    let offset = ((page - 1) * per_page) as u64;

    let status_filter = parse_status(params.status.as_deref());

    let items = queue
        .list(status_filter.clone(), per_page as usize, offset)
        .await;
    let total = queue.count(status_filter).await.unwrap_or(0);

    match items {
        Ok(items) => {
            let responses: Vec<TriageItemResponse> = items
                .into_iter()
                .map(|item| {
                    let mut resp = item_to_response(item);
                    let score = resp.composite_score.unwrap_or(0.0);
                    resp.score_band =
                        apex_core::triage::score_to_band(score, &threshold).to_string();
                    resp.score_band_color =
                        apex_core::triage::score_band_color(score, &threshold).to_string();
                    resp
                })
                .collect();

            let paged = PagedResponse {
                items: responses,
                page,
                per_page,
                total: total as u64,
            };

            (StatusCode::OK, Json(success(paged)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// GET /api/triage/stats — aggregate triage statistics.
pub(crate) async fn get_triage_stats(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.stats().await {
        Ok(stats) => (StatusCode::OK, Json(success(stats_to_response(stats)))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// GET /api/triage/:id — get a single triage item.
pub(crate) async fn get_triage_item(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.get_by_id(id).await {
        Ok(Some(item)) => (StatusCode::OK, Json(success(item_to_response(item)))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "triage",
                &id.to_string(),
            ))),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// POST /api/triage/:id/override — manually override a triage score.
pub(crate) async fn override_triage_score(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<Uuid>,
    Json(payload): Json<OverrideScorePayload>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    if !(0.0..=1.0).contains(&payload.score) {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::validation(
                "score",
                "score must be between 0.0 and 1.0",
            ))),
        );
    }

    let overridden_by = payload.reason.as_deref().unwrap_or("api-override");

    match queue.override_score(id, payload.score, overridden_by).await {
        Ok(item) => (StatusCode::OK, Json(success(item_to_response(item)))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// POST /api/triage/:id/acknowledge — mark a triage item as acknowledged.
pub(crate) async fn acknowledge_triage_item(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.acknowledge(id).await {
        Ok(item) => (StatusCode::OK, Json(success(item_to_response(item)))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// POST /api/triage/:id/resolve — mark a triage item as resolved.
pub(crate) async fn resolve_triage_item(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.resolve(id).await {
        Ok(item) => (StatusCode::OK, Json(success(item_to_response(item)))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// POST /api/triage/:id/dismiss — mark a triage item as dismissed.
pub(crate) async fn dismiss_triage_item(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.dismiss(id).await {
        Ok(item) => (StatusCode::OK, Json(success(item_to_response(item)))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}

/// GET /api/triage/bands — triage band distribution counts.
pub(crate) async fn get_triage_bands(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
) -> impl IntoResponse {
    let queue = make_queue(state.store.clone());

    match queue.band_counts().await {
        Ok(bands) => (StatusCode::OK, Json(success(bands))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(ApiError::internal(e.to_string()))),
        ),
    }
}
