//! Supply Chain Risk API handlers.
//!
//! - `GET /api/supply-risk` — returns supply chain risk assessments

use crate::*;
use serde::Serialize;
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
    Extension(auth): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<Vec<SupplyRiskItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // Visibility-scoped read (see get_threat_intel).
    let records = match state
        .store
        .list_activity_feed(
            auth.user_id.as_str(),
            auth.role.can_admin(),
            None,
            None,
            None,
            500,
        )
        .await
    {
        Ok(records) => records,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_supply_risks query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch supply chain risks",
                ))),
            );
        }
    };

    let category_of = |record: &apex_store::postgres::ActivityFeedRecord| {
        record
            .details
            .get("category")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("supplier")
            .to_string()
    };
    let items: Vec<SupplyRiskItem> = records
        .iter()
        .filter(|record| {
            record.action_type == "threat_detected"
                || matches!(
                    category_of(record).as_str(),
                    "supplier" | "logistics" | "geopolitical" | "regulatory"
                )
        })
        .take(100)
        .map(|record| SupplyRiskItem {
            id: record.id.to_string(),
            name: record
                .entity_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            risk_level: record
                .details
                .get("risk_level")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("medium")
                .to_string(),
            category: category_of(record),
            impact_score: record
                .details
                .get("impact_score")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(50),
            last_detected: record
                .details
                .get("detected_at")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| record.created_at.to_rfc3339()),
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
