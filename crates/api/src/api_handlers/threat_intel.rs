//! Threat Intelligence API handlers.
//!
//! - `GET /api/threat-intel` — returns threat intelligence assessments

use crate::*;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ThreatIntelItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: String,
    pub category: String,
    pub source: String,
    pub confidence: f64,
    pub affected_entity_count: u32,
    pub detected_at: String,
    pub mitre_tactic: Option<String>,
}

pub(crate) async fn get_threat_intel(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<ThreatIntelItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let rows = match sqlx::query(
        r#"
        SELECT
            id::text,
            COALESCE(entity_name, 'Threat Event') AS title,
            COALESCE(details->>'description', 'Threat detected') AS description,
            CASE
                WHEN details->>'severity' = 'critical' THEN 'critical'
                WHEN details->>'severity' = 'high' THEN 'high'
                WHEN details->>'severity' = 'medium' THEN 'medium'
                ELSE 'low'
            END AS severity,
            COALESCE(details->>'category', 'cyber') AS category,
            COALESCE(details->>'source', 'System') AS source,
            COALESCE((details->>'confidence')::float, 0.7) AS confidence,
            COALESCE((details->>'affected_entity_count')::int, 1) AS affected_entity_count,
            created_at::text AS detected_at,
            details->>'mitre_tactic' AS mitre_tactic
        FROM activity_feed
        WHERE action_type = 'threat_detected'
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .fetch_all(&state.store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_threat_intel query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to fetch threat intelligence"))),
            );
        }
    };

    let items: Vec<ThreatIntelItem> = rows
        .iter()
        .map(|row| ThreatIntelItem {
            id: row.get::<String, _>("id"),
            title: row.get::<String, _>("title"),
            description: row.get::<String, _>("description"),
            severity: row.get::<String, _>("severity"),
            category: row.get::<String, _>("category"),
            source: row.get::<String, _>("source"),
            confidence: row.get::<f64, _>("confidence"),
            affected_entity_count: row.get::<i32, _>("affected_entity_count") as u32,
            detected_at: row.get::<String, _>("detected_at"),
            mitre_tactic: row.try_get::<String, _>("mitre_tactic").ok(),
        })
        .collect();

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_threat_intel", duration_ms);

    (
        StatusCode::OK,
        Json(success_with_meta(
            items,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}