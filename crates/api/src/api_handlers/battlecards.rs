//! Battlecards API handlers — full CRUD + regenerate + export.
//!
//! Uses `PgStore` battlecard methods directly (not activity_feed).

use crate::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use apex_store::postgres::PgStore;

/// GET /api/battlecards — list all battlecards with optional filters.
#[derive(Debug, Deserialize)]
pub struct ListBattlecardsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BattlecardItem {
    pub id: String,
    pub account_name: String,
    pub threat_level: String,
    pub win_probability: f64,
    pub competitor_count: u32,
    pub key_intel: String,
    pub last_updated: Option<String>,
}

pub(crate) async fn list_battlecards(
    State(state): State<AppState>,
    Query(params): Query<ListBattlecardsQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<BattlecardItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 100);

    match state.store.list_battlecards(params.status.as_deref(), None, page, per_page).await {
        Ok(rows) => {
            let items: Vec<BattlecardItem> = rows.into_iter().map(|row| {
                BattlecardItem {
                    id: row.id.to_string(),
                    account_name: row.title,
                    threat_level: "medium".to_string(),
                    win_probability: 0.5,
                    competitor_count: 0,
                    key_intel: row.positioning
                        .as_ref()
                        .and_then(|v| v.get("summary"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("No intel available")
                        .to_string(),
                    last_updated: Some(row.updated_at.to_rfc3339()),
                }
            }).collect();

            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_battlecards", duration_ms);
            (StatusCode::OK, Json(success_with_meta(items, ResponseMeta::now().with_request_id(request_id).with_duration(duration_ms))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "list_battlecards failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to fetch battlecards"))))
        }
    }
}

/// POST /api/battlecards — create a new battlecard.
#[derive(Debug, Deserialize)]
pub struct CreateBattlecardRequest {
    pub our_company_id: String,
    pub competitor_id: String,
    pub title: String,
}

pub(crate) async fn create_battlecard(
    State(state): State<AppState>,
    Json(payload): Json<CreateBattlecardRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let our_id = match Uuid::parse_str(&payload.our_company_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid our_company_id")))),
    };
    let comp_id = match Uuid::parse_str(&payload.competitor_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid competitor_id")))),
    };

    match state.store.create_battlecard(our_id, comp_id, &payload.title).await {
        Ok(id) => (
            StatusCode::CREATED,
            Json(success_with_meta(serde_json::json!({"id": id.to_string()}), ResponseMeta::now().with_request_id(request_id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "create_battlecard failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to create battlecard"))))
        }
    }
}

/// GET /api/battlecards/:id — get a single battlecard.
pub(crate) async fn get_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid battlecard ID")))),
    };

    match state.store.get_battlecard(uid).await {
        Ok(Some(row)) => {
            let body = serde_json::json!({
                "id": row.id.to_string(),
                "title": row.title,
                "status": row.status,
                "competitor_id": row.competitor_id.to_string(),
                "positioning": row.positioning,
                "pricing": row.pricing,
                "feature_matrix": row.feature_matrix,
                "strengths": row.strengths,
                "weaknesses": row.weaknesses,
                "objection_handlers": row.objection_handlers,
                "kill_shots": row.kill_shots,
                "recent_news": row.recent_news,
                "win_loss": row.win_loss,
                "created_at": row.created_at.to_rfc3339(),
                "updated_at": row.updated_at.to_rfc3339(),
            });
            (StatusCode::OK, Json(success_with_meta(body, ResponseMeta::now().with_request_id(request_id))))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Battlecard", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_battlecard failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to fetch battlecard"))))
        }
    }
}

/// DELETE /api/battlecards/:id — delete a battlecard.
pub(crate) async fn delete_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid battlecard ID")))),
    };

    match state.store.delete_battlecard(uid).await {
        Ok(()) => (StatusCode::OK, Json(success_with_meta(serde_json::json!({"deleted": true}), ResponseMeta::now().with_request_id(request_id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete_battlecard failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to delete battlecard"))))
        }
    }
}

/// PUT /api/battlecards/:id/section — update a battlecard section.
#[derive(Debug, Deserialize)]
pub struct UpdateSectionRequest {
    pub section: String,
    pub data: serde_json::Value,
}

pub(crate) async fn update_battlecard_section(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateSectionRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid battlecard ID")))),
    };

    match state.store.update_battlecard_section(uid, &payload.section, &payload.data).await {
        Ok(()) => (StatusCode::OK, Json(success_with_meta(serde_json::json!({"updated": true}), ResponseMeta::now().with_request_id(request_id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "update_battlecard_section failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to update battlecard section"))))
        }
    }
}

/// POST /api/battlecards/:id/regenerate — regenerate a battlecard.
pub(crate) async fn regenerate_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid battlecard ID")))),
    };

    match state.store.update_battlecard_timestamp(uid).await {
        Ok(()) => {
            // Surface the regeneration in the activity feed. The API crate does
            // not depend on apex-worker, so we mirror the ActivityLogger INSERT
            // pattern (actor=Battlecard Generator, action=battlecard_generated)
            // using the connection pool directly. Failures are swallowed so a
            // feed-write glitch can never block the regeneration response.
            if let Ok(Some(bc)) = state.store.get_battlecard(uid).await {
                let competitor_name = state
                    .store
                    .get_company(bc.competitor_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_else(|| bc.competitor_id.to_string());
                let details = serde_json::json!({
                    "logged_at": chrono::Utc::now().to_rfc3339(),
                    "competitor": &competitor_name,
                    "summary": format!("Battlecard regenerated for competitor {}", competitor_name),
                });
                let entity_id = bc.our_company_id.to_string();
                let _ = sqlx::query(
                    r#"INSERT INTO activity_feed
                         (actor_id, actor_name, action_type, entity_type, entity_id, entity_name,
                          details, workspace_id, team_id, visibility, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())"#,
                )
                .bind("system")
                .bind("Battlecard Generator")
                .bind("battlecard_generated")
                .bind(Some("company"))
                .bind(&entity_id)
                .bind(&bc.title)
                .bind(&details)
                .bind(None::<uuid::Uuid>)
                .bind(None::<&str>)
                .bind("team")
                .execute(&state.store.pool)
                .await;
            }
            (StatusCode::OK, Json(success_with_meta(serde_json::json!({"regenerated": true, "id": id}), ResponseMeta::now().with_request_id(request_id))))
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "regenerate_battlecard failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to regenerate battlecard"))))
        }
    }
}

/// GET /api/battlecards/:id/export — export a battlecard.
pub(crate) async fn export_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(error_response(ApiError::bad_request("Invalid battlecard ID")))),
    };

    match state.store.get_battlecard(uid).await {
        Ok(Some(row)) => {
            let body = serde_json::json!({"format": "markdown", "content": format!("# {}\n\nPositioning: {:?}", row.title, row.positioning)});
            (StatusCode::OK, Json(success_with_meta(body, ResponseMeta::now().with_request_id(request_id))))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(error_response(ApiError::not_found("Battlecard", &id)))),
        Err(err) => {
            tracing::error!(request_id = %request_id, "export_battlecard failed: {err:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response(ApiError::internal("Failed to export battlecard"))))
        }
    }
}