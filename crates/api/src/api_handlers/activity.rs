//! Activity feed API handlers.
//!
//! Serves the real-time activity feed from the `activity_feed` table (created by
//! migration 0042). This is the backend for the ActivityFeed page in the WASM frontend.
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
    Query(params): Query<ActivityQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<ActivityEvent>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(50).min(200) as i64;
    let offset = params.offset.unwrap_or(0) as i64;

    let rows = match sqlx::query(
        r#"
        SELECT
            id::text,
            action_type,
            COALESCE(entity_name, 'System Event') AS title,
            details->>'description' AS description,
            CASE
                WHEN action_type IN ('insight_generated', 'threat_detected') THEN 'high'
                WHEN action_type IN ('poi_discovered', 'company_detected', 'crawl_completed') THEN 'medium'
                WHEN action_type IN ('job_failed') THEN 'high'
                ELSE 'low'
            END AS severity,
            created_at::text AS timestamp,
            entity_name,
            entity_id,
            details->>'source' AS source,
            details->>'source_url' AS source_url
        FROM activity_feed
        WHERE ($3::text IS NULL OR action_type = $3)
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .bind(&params.action_type)
    .fetch_all(&state.store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_activity_feed query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to fetch activity feed"))),
            );
        }
    };

    let events: Vec<ActivityEvent> = rows
        .iter()
        .map(|row| ActivityEvent {
            id: row.get::<String, _>("id"),
            event_type: row.get::<String, _>("action_type"),
            title: row.get::<String, _>("title"),
            description: row.try_get::<String, _>("description").ok(),
            severity: row.get::<String, _>("severity"),
            timestamp: row.get::<String, _>("timestamp"),
            entity_name: row.try_get::<String, _>("entity_name").ok(),
            entity_id: row.try_get::<String, _>("entity_id").ok(),
            source: row.try_get::<String, _>("source").ok(),
            source_url: row.try_get::<String, _>("source_url").ok(),
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
    Json(payload): Json<CreateActivityRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let details = payload.details.unwrap_or(serde_json::json!({}));

    let result = sqlx::query(
        r#"
        INSERT INTO activity_feed (
            actor_id, actor_name, action_type, entity_type,
            entity_id, entity_name, details, visibility
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'organization')
        RETURNING id::text
        "#,
    )
    .bind("system")
    .bind("ApexIntel System")
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
