//! Supply Chain Risk API handlers.
//!
//! - `GET /api/supply-risk` — returns supply chain risk assessments

use crate::*;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct SupplyRiskItem {
    pub id: String,
    pub name: String,
    pub risk_level: String,
    pub category: String,
    pub impact_score: i64,
    pub last_detected: String,
}

pub(crate) async fn get_supply_risks(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<SupplyRiskItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let rows = match sqlx::query(
        r#"
        SELECT
            id::text,
            COALESCE(entity_name, 'Unknown') AS name,
            COALESCE(details->>'risk_level', 'medium') AS risk_level,
            COALESCE(details->>'category', 'supplier') AS category,
            COALESCE((details->>'impact_score')::int, 50) AS impact_score,
            COALESCE(details->>'detected_at', created_at::text) AS last_detected
        FROM activity_feed
        WHERE action_type = 'threat_detected'
           OR details->>'category' IN ('supplier', 'logistics', 'geopolitical', 'regulatory')
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .fetch_all(&state.store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_supply_risks query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to fetch supply chain risks"))),
            );
        }
    };

    let items: Vec<SupplyRiskItem> = rows
        .iter()
        .map(|row| SupplyRiskItem {
            id: row.get::<String, _>("id"),
            name: row.get::<String, _>("name"),
            risk_level: row.get::<String, _>("risk_level"),
            category: row.get::<String, _>("category"),
            impact_score: row.get::<i32, _>("impact_score") as i64,
            last_detected: row.get::<String, _>("last_detected"),
        })
        .collect();

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_supply_risks", duration_ms);

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