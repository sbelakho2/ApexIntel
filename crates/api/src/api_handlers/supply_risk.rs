//! Supply Chain Risk API handlers.
//!
//! - `GET /api/supply-risk` — returns supply chain risk assessments

use crate::*;
use serde::Serialize;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub(crate) struct SupplyRiskItem {
    pub id: String,
    pub name: String,
    /// Risk level recorded on the activity details; `None` = not recorded
    /// (never a defaulted "medium").
    pub risk_level: Option<String>,
    pub category: String,
    /// Impact score recorded on the activity details; `None` = not recorded
    /// (never a defaulted 50).
    pub impact_score: Option<i64>,
    pub last_detected: String,
}

fn supply_risk_item(record: &apex_store::postgres::ActivityFeedRecord) -> SupplyRiskItem {
    let category = record
        .details
        .get("category")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("supplier")
        .to_string();
    SupplyRiskItem {
        id: record.id.to_string(),
        name: record
            .entity_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
        // A missing detail is unknown, not a fabricated "medium".
        risk_level: record
            .details
            .get("risk_level")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        category,
        // A missing detail is unknown, not a fabricated 50.
        impact_score: record
            .details
            .get("impact_score")
            .and_then(serde_json::Value::as_i64),
        last_detected: record
            .details
            .get("detected_at")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| record.created_at.to_rfc3339()),
    }
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
        .map(supply_risk_item)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(details: serde_json::Value) -> apex_store::postgres::ActivityFeedRecord {
        apex_store::postgres::ActivityFeedRecord {
            id: uuid::Uuid::new_v4(),
            actor_id: "analyst".into(),
            actor_name: "Analyst".into(),
            action_type: "threat_detected".into(),
            entity_type: None,
            entity_id: None,
            entity_name: Some("Acme".into()),
            details,
            workspace_id: None,
            team_id: None,
            visibility: "workspace".into(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Missing risk details are unknown, never a defaulted "medium"/50.
    #[test]
    fn unrecorded_risk_details_stay_unknown() {
        let item = supply_risk_item(&record(serde_json::json!({})));
        assert_eq!(item.risk_level, None);
        assert_eq!(item.impact_score, None);
    }

    #[test]
    fn recorded_risk_details_pass_through() {
        let item = supply_risk_item(&record(serde_json::json!({
            "risk_level": "high",
            "impact_score": 82,
        })));
        assert_eq!(item.risk_level.as_deref(), Some("high"));
        assert_eq!(item.impact_score, Some(82));
    }
}
