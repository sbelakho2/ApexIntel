//! Psychological Profiles API handlers.
//!
//! - `GET /api/psych-profiles` — returns psychological profile summaries

use crate::*;
use serde::Serialize;
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct PsychProfileItem {
    pub person_id: String,
    pub person_name: String,
    pub current_role: String,
    pub company_name: String,
    pub decision_style: String,
    pub influence_role: String,
    pub pain_points: Vec<String>,
    pub change_appetite: f64,
    pub communication_style: String,
    pub recommended_approach: String,
    pub traits: Vec<String>,
}

pub(crate) async fn get_psych_profiles(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<PsychProfileItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let rows = match sqlx::query(
        r#"
        SELECT
            pp.person_id::text AS person_id,
            COALESCE(p.name, pp.person_id::text) AS person_name,
            COALESCE(p.role, 'Unknown') AS current_role,
            COALESCE(p.organization, 'Unknown') AS company_name,
            COALESCE(pp.decision_style, 'unknown') AS decision_style,
            COALESCE(ip.influence_role, 'unknown') AS influence_role,
            COALESCE(pp.pain_points::text, '[]') AS pain_points_json,
            COALESCE(pp.change_appetite, 0.5) AS change_appetite,
            COALESCE(ep.communication_style, 'Professional and direct') AS communication_style,
            COALESCE(ep.recommended_approach, 'Present data-driven value proposition') AS recommended_approach,
            COALESCE(pp.traits::text, '[]') AS traits_json
        FROM psychological_profiles pp
        LEFT JOIN persons p ON p.id::text = pp.person_id::text
        LEFT JOIN influence_profiles ip ON ip.person_id = pp.person_id
        LEFT JOIN engagement_profiles ep ON ep.person_id = pp.person_id
        ORDER BY pp.updated_at DESC NULLS LAST
        LIMIT 50
        "#,
    )
    .fetch_all(&state.store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_psych_profiles query failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to fetch psychological profiles"))),
            );
        }
    };

    let items: Vec<PsychProfileItem> = rows
        .iter()
        .map(|row| {
            let pain_points_json: String = row.get("pain_points_json");
            let traits_json: String = row.get("traits_json");

            let pain_points: Vec<String> =
                serde_json::from_str(&pain_points_json).unwrap_or_default();
            let traits: Vec<String> =
                serde_json::from_str(&traits_json).unwrap_or_default();

            PsychProfileItem {
                person_id: row.get("person_id"),
                person_name: row.get("person_name"),
                current_role: row.get("current_role"),
                company_name: row.get("company_name"),
                decision_style: row.get("decision_style"),
                influence_role: row.get("influence_role"),
                pain_points,
                change_appetite: row.get("change_appetite"),
                communication_style: row.get("communication_style"),
                recommended_approach: row.get("recommended_approach"),
                traits,
            }
        })
        .collect();

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_psych_profiles", duration_ms);

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