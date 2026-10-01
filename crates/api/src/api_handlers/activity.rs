//! Activity feed API handlers.
//!
//! Serves the real-time activity feed from the `activity_feed` table (created by
//! migration 0042). This is the backend for the ActivityFeed section of the
//! server-rendered UI.
//!
//! - `GET /api/activity` — paginated activity feed with optional type filter
//! - `POST /api/activity` — insert a system event (worker-driven)

use crate::*;
use apex_shared::ActivityEvent as SharedActivityEvent;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub action_type: Option<String>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActivityEvent {
    pub id: String,
    pub event_type: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub timestamp: String,
    pub entity_name: Option<String>,
    pub entity_id: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
}

/// Convert the shared type into the API response shape.
impl From<SharedActivityEvent> for ActivityEvent {
    fn from(e: SharedActivityEvent) -> Self {
        Self {
            id: e.id,
            event_type: e.event_type,
            title: e.title,
            description: e.description,
            severity: e.severity,
            timestamp: e.timestamp,
            entity_name: e.entity_name,
            entity_id: e.entity_id,
            source: e.source,
            source_url: e.source_url,
        }
    }
}

/// `GET /api/activity` — returns paginated activity feed.
pub(crate) async fn get_activity_feed(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Query(params): Query<ActivityQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<ActivityEvent>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(50).min(200) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    let wanted_action = params.action_type.as_deref();

    // Read through the visibility-scoped store query: a raw `SELECT` here
    // returned other actors' private rows and rows attached to workspaces the
    // caller cannot see (the feed defaults new rows to `private`). Type
    // filtering stays in-process because the scoped query has no action_type
    // predicate.
    let fetch = offset.saturating_add(limit);
    let records = match state
        .store
        .list_activity_feed(
            auth_ctx.user_id.as_str(),
            auth_ctx.role.can_admin(),
            None,
            None,
            None,
            fetch,
        )
        .await
    {
        Ok(records) => records,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_activity_feed query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch activity feed",
                ))),
            );
        }
    };

    let events: Vec<ActivityEvent> = records
        .into_iter()
        .filter(|record| wanted_action.is_none_or(|action| record.action_type == action))
        .skip(offset.max(0) as usize)
        .take(limit.max(0) as usize)
        .map(|record| {
            let details = if record.details.is_object() {
                record.details
            } else {
                serde_json::json!({})
            };
            let entity_name = record.entity_name;
            let severity = match record.action_type.as_str() {
                "insight_generated" | "threat_detected" | "job_failed" => "high",
                "poi_discovered" | "company_detected" | "crawl_completed" => "medium",
                _ => "low",
            };
            ActivityEvent {
                id: record.id.to_string(),
                event_type: record.action_type,
                title: entity_name
                    .clone()
                    .unwrap_or_else(|| "System Event".to_string()),
                description: details
                    .get("description")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                severity: severity.to_string(),
                timestamp: record.created_at.to_rfc3339(),
                entity_name,
                entity_id: record.entity_id,
                source: details
                    .get("source")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                source_url: details
                    .get("source_url")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }
        })
        .collect();

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_activity_feed", duration_ms);

    (
        StatusCode::OK,
        Json(success_with_meta(
            events,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

/// `POST /api/activity` — insert a new activity event (called by worker/background jobs).
#[derive(Debug, Deserialize)]
pub struct CreateActivityRequest {
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub entity_name: Option<String>,
    pub details: Option<serde_json::Value>,
}

pub(crate) async fn create_activity_event(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Json(payload): Json<CreateActivityRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let details = payload.details.unwrap_or(serde_json::json!({}));

    // Attribution is the authenticated principal: a request could otherwise
    // forge `system`-attributed activity visible to the whole organization.
    // Only admins may record organization-visible activity through the API;
    // background jobs write the table directly.
    if !auth.role.can_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(error_response(ApiError::forbidden(
                "only admins may record activity",
            ))),
        );
    }

    let result = sqlx::query(
        r#"
        INSERT INTO activity_feed (
            actor_id, actor_name, action_type, entity_type,
            entity_id, entity_name, details, visibility
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'organization')
        RETURNING id::text
        "#,
    )
    .bind(auth.user_id.as_str())
    .bind(auth.user_id.as_str())
    .bind(&payload.action_type)
    .bind(&payload.entity_type)
    .bind(&payload.entity_id)
    .bind(&payload.entity_name)
    .bind(&details)
    .fetch_one(&state.store.pool)
    .await;

    match result {
        Ok(row) => {
            let id: String = row.get("id");
            (
                StatusCode::CREATED,
                Json(success_with_meta(
                    serde_json::json!({ "id": id }),
                    ResponseMeta::now().with_request_id(request_id),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "create_activity_event failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create activity event",
                ))),
            )
        }
    }
}
