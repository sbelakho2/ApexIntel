//! API handlers for battlecard CRUD and operations.
//!
//! Pattern follows `competitors.rs` — each handler is a public async fn
//! that extracts state, validates input, delegates to PgStore, and returns
//! a JSON envelope.

use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use apex_api::responses::{
    success_with_meta, ApiError, ApiResponse, PagedResponse, ResponseMeta,
};
use apex_api::routes::battlecards::{
    BattlecardResponse, CreateBattlecardBody, ExportQuery, ListBattlecardsQuery, UpdateSectionBody,
};
use apex_store::postgres::BattlecardRow;

use crate::*;

/// Validate a UUID string and return a 400 error on failure.
fn parse_battlecard_uuid(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::bad_request("Invalid battlecard UUID"))
}

/// GET /api/battlecards — list battlecards with optional filters.
pub(crate) async fn list_battlecards(
    State(state): State<AppState>,
    Query(query): Query<ListBattlecardsQuery>,
) -> Result<(StatusCode, Json<ApiResponse<PagedResponse<BattlecardResponse>>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);

    let total = state
        .store
        .count_battlecards(query.status.as_deref(), query.competitor_id)
        .await
        .map_err(|e| {
            tracing::error!(request_id = %request_id, "count_battlecards failed: {e:#}");
            ApiError::internal("Failed to count battlecards")
        })?;

    let rows = state
        .store
        .list_battlecards(query.status.as_deref(), query.competitor_id, page, per_page)
        .await
        .map_err(|e| {
            tracing::error!(request_id = %request_id, "list_battlecards failed: {e:#}");
            ApiError::internal("Failed to list battlecards")
        })?;

    let items: Vec<BattlecardResponse> = rows.into_iter().map(BattlecardResponse::from).collect();

    let payload = PagedResponse {
        items,
        total: total.max(0) as u64,
        page,
        per_page,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_battlecards", duration_ms);

    Ok((
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

/// GET /api/battlecards/:id — get a single battlecard.
pub(crate) async fn get_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<ApiResponse<BattlecardResponse>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let uid = parse_battlecard_uuid(&id)?;

    let row = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard failed: {e:#}");
        ApiError::internal("Failed to get battlecard")
    })?;

    match row {
        Some(row) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_battlecard", duration_ms);
            Ok((
                StatusCode::OK,
                Json(success_with_meta(
                    BattlecardResponse::from(row),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            ))
        }
        None => Err(ApiError::not_found("battlecard", &id)),
    }
}

/// POST /api/battlecards — create a new battlecard.
pub(crate) async fn create_battlecard(
    State(state): State<AppState>,
    Json(body): Json<CreateBattlecardBody>,
) -> Result<(StatusCode, Json<ApiResponse<BattlecardResponse>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let title = body.title.unwrap_or_default();
    let id = state
        .store
        .create_battlecard(body.our_company_id, body.competitor_id, &title)
        .await
        .map_err(|e| {
            tracing::error!(request_id = %request_id, "create_battlecard failed: {e:#}");
            if e.to_string().contains("unique constraint")
                || e.to_string().contains("duplicate key")
            {
                ApiError::bad_request("A battlecard already exists for this company pair")
            } else {
                ApiError::internal("Failed to create battlecard")
            }
        })?;

    // Fetch the newly created row to return a full response
    let row = state
        .store
        .get_battlecard(id)
        .await
        .map_err(|e| {
            tracing::error!(request_id = %request_id, "get_battlecard after create failed: {e:#}");
            ApiError::internal("Failed to retrieve created battlecard")
        })?
        .ok_or_else(|| ApiError::internal("Created battlecard not found"))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("create_battlecard", duration_ms);

    Ok((
        StatusCode::CREATED,
        Json(success_with_meta(
            BattlecardResponse::from(row),
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

/// PATCH /api/battlecards/:id — update a single JSONB section.
pub(crate) async fn update_battlecard_section(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSectionBody>,
) -> Result<(StatusCode, Json<ApiResponse<BattlecardResponse>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let uid = parse_battlecard_uuid(&id)?;

    state
        .store
        .update_battlecard_section(uid, &body.section, &body.data)
        .await
        .map_err(|e| {
            tracing::error!(request_id = %request_id, "update_battlecard_section failed: {e:#}");
            if e.to_string().contains("invalid section name") {
                ApiError::bad_request(&e.to_string())
            } else {
                ApiError::internal("Failed to update battlecard section")
            }
        })?;

    let row = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard after update failed: {e:#}");
        ApiError::internal("Failed to retrieve updated battlecard")
    })?
    .ok_or_else(|| ApiError::not_found("battlecard", &uid.to_string()))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("update_battlecard_section", duration_ms);

    Ok((
        StatusCode::OK,
        Json(success_with_meta(
            BattlecardResponse::from(row),
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

/// POST /api/battlecards/:id/regenerate — regenerate all sections (touch timestamps).
pub(crate) async fn regenerate_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<ApiResponse<BattlecardResponse>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let uid = parse_battlecard_uuid(&id)?;

    // Check battlecard exists
    let _existing = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard failed: {e:#}");
        ApiError::internal("Failed to get battlecard")
    })?
    .ok_or_else(|| ApiError::not_found("battlecard", &uid.to_string()))?;

    // Touch the regeneration timestamp
    state.store.update_battlecard_timestamp(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "update_battlecard_timestamp failed: {e:#}");
        ApiError::internal("Failed to regenerate battlecard")
    })?;

    let row = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard after regenerate failed: {e:#}");
        ApiError::internal("Failed to retrieve regenerated battlecard")
    })?
    .ok_or_else(|| ApiError::not_found("battlecard", &uid.to_string()))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("regenerate_battlecard", duration_ms);

    Ok((
        StatusCode::OK,
        Json(success_with_meta(
            BattlecardResponse::from(row),
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

/// GET /api/battlecards/:id/export — export battlecard in requested format.
pub(crate) async fn export_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Result<(StatusCode, Json<ApiResponse<String>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let uid = parse_battlecard_uuid(&id)?;

    let row = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard failed: {e:#}");
        ApiError::internal("Failed to get battlecard")
    })?
    .ok_or_else(|| ApiError::not_found("battlecard", &uid.to_string()))?;

    // Convert row to BattlecardData for export
    let battlecard_data = row_to_battlecard_data(&row);

    let exported = match query.format.as_str() {
        "markdown" => {
            apex_insights::battlecards::BattlecardDistributor::to_markdown(&battlecard_data)
        }
        "slack" => {
            let blocks =
                apex_insights::battlecards::BattlecardDistributor::slack_blocks(&battlecard_data);
            serde_json::to_string_pretty(&blocks)
                .map_err(|e| ApiError::internal(&format!("Failed to serialize slack blocks: {}", e)))?
        }
        other => {
            return Err(ApiError::bad_request(&format!(
                "Unsupported export format '{}'. Use 'markdown' or 'slack'.",
                other
            )));
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("export_battlecard", duration_ms);

    Ok((
        StatusCode::OK,
        Json(success_with_meta(
            exported,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

/// DELETE /api/battlecards/:id — delete a battlecard.
pub(crate) async fn delete_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<ApiResponse<()>>), ApiError> {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let uid = parse_battlecard_uuid(&id)?;

    // Check exists
    let existing = state.store.get_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "get_battlecard failed: {e:#}");
        ApiError::internal("Failed to get battlecard")
    })?;

    if existing.is_none() {
        return Err(ApiError::not_found("battlecard", &uid.to_string()));
    }

    state.store.delete_battlecard(uid).await.map_err(|e| {
        tracing::error!(request_id = %request_id, "delete_battlecard failed: {e:#}");
        ApiError::internal("Failed to delete battlecard")
    })?;

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("delete_battlecard", duration_ms);

    Ok((
        StatusCode::OK,
        Json(success_with_meta(
            (),
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    ))
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Convert a `BattlecardRow` to `BattlecardData` for distribution/export.
fn row_to_battlecard_data(row: &BattlecardRow) -> apex_insights::battlecards::BattlecardData {
    use apex_insights::battlecards::{
        BattlecardData, FeatureMatrixSection, NewsItem, ObjectionHandlerPair, PositioningSection,
        PricingSection, StrengthItem, WeaknessItem, WinLossSection,
    };
    use apex_insights::battlecards::kill_shot::KillShot;

    // Deserialize each JSONB section or use defaults
    let positioning: PositioningSection = row
        .positioning
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let pricing: PricingSection = row
        .pricing
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let feature_matrix: FeatureMatrixSection = row
        .feature_matrix
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let strengths: Vec<StrengthItem> = row
        .strengths
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let weaknesses: Vec<WeaknessItem> = row
        .weaknesses
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let objection_handlers: Vec<ObjectionHandlerPair> = row
        .objection_handlers
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let kill_shots: Vec<KillShot> = row
        .kill_shots
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let recent_news: Vec<NewsItem> = row
        .recent_news
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let win_loss: WinLossSection = row
        .win_loss
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    BattlecardData {
        positioning,
        pricing,
        feature_matrix,
        strengths,
        weaknesses,
        objection_handlers,
        kill_shots,
        recent_news,
        win_loss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_battlecard_uuid_rejects_invalid() {
        let err = parse_battlecard_uuid("bad-id").expect_err("invalid uuid should fail");
        assert_eq!(err.http_status(), 400);
        assert!(err.message.contains("Invalid battlecard UUID"));
    }

    #[test]
    fn test_parse_battlecard_uuid_accepts_valid() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        assert!(parse_battlecard_uuid(uuid).is_ok());
    }
}
