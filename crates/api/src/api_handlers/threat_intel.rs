//! Threat Intelligence API handlers.
//!
//! - `GET /api/threat-intel` — returns threat intelligence assessments

use crate::*;
use serde::Serialize;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub(crate) struct ThreatIntelItem {
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
    Extension(auth): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<Vec<ThreatIntelItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // Visibility-scoped read: private rows and activity from invisible
    // workspaces must not surface here (the raw table has no access control).
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
            tracing::error!(request_id = %request_id, "get_threat_intel query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch threat intelligence",
                ))),
            );
        }
    };

    let items: Vec<ThreatIntelItem> = records
        .iter()
        .filter(|record| record.action_type == "threat_detected")
        .take(100)
        .map(|record| {
            let severity = match record
                .details
                .get("severity")
                .and_then(serde_json::Value::as_str)
            {
                Some("critical") => "critical",
                Some("high") => "high",
                Some("medium") => "medium",
                _ => "low",
            };
            ThreatIntelItem {
                id: record.id.to_string(),
                title: record
                    .entity_name
                    .clone()
                    .unwrap_or_else(|| "Threat Event".to_string()),
                description: record
                    .details
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Threat detected")
                    .to_string(),
                severity: severity.to_string(),
                category: record
                    .details
                    .get("category")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("cyber")
                    .to_string(),
                source: record
                    .details
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("System")
                    .to_string(),
                confidence: record
                    .details
                    .get("confidence")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.7),
                affected_entity_count: record
                    .details
                    .get("affected_entity_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(1) as u32,
                detected_at: record.created_at.to_rfc3339(),
                mitre_tactic: record
                    .details
                    .get("mitre_tactic")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            }
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
