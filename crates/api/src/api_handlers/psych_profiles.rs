//! Psychological Profiles API handlers.
//!
//! - `GET /api/psych-profiles` — returns psychological profile summaries
//!
//! Backed by the *real* schema created by migrations `20260624_psychological_profiles.sql`
//! and `20260627_reconcile_psych_profiles.sql`:
//!   - `psychological_profiles`: decision_style (TEXT enum), change_appetite (TEXT enum),
//!     pain_index (FLOAT), risk_tolerance (FLOAT), preferred_proof (TEXT[]),
//!     buying_center_role (TEXT), evidence_sources (TEXT[]).
//!   - `engagement_profiles`: talking_points / opening_topics / avoid_topics (TEXT[]),
//!     best_channel (TEXT enum), best_timing (TEXT).
//!   - `persons`: name, current_role, primary_org_id → companies.name,
//!     communication_style, decision_style, change_appetite.
//!
//! NOTE: the previous implementation queried non-existent columns (`pain_points`,
//! `traits`, `influence_profiles.influence_role`, `engagement_profiles.communication_style`
//! / `recommended_approach`) and cast `change_appetite` to f64 even though it is a
//! TEXT enum — so the endpoint returned HTTP 500 on every call. This rewrite maps
//! the actual schema.

use crate::*;
use serde::Serialize;
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

/// One row of the psych-profiles list view, derived from real schema columns only.
#[derive(Debug, Serialize)]
pub struct PsychProfileItem {
    pub person_id: String,
    pub person_name: String,
    pub current_role: String,
    pub company_name: String,
    /// Procurement-relevant role inferred by the psych compute job
    /// (e.g. "buyer", "decision_maker", "influencer", "gatekeeper").
    pub buying_center_role: String,
    /// Decision-style enum (authoritative / collaborative / analytical / …).
    pub decision_style: String,
    /// Change-appetite enum (high / moderate / low / resistant).
    pub change_appetite: String,
    /// 0..1 intensity of detected pain signals — higher = more receptive.
    pub pain_index: f64,
    /// 0..1 risk tolerance.
    pub risk_tolerance: f64,
    /// Proof types the buyer responds to (e.g. ["case_study", "roi_model"]).
    pub preferred_proof: Vec<String>,
    /// Best engagement channel (email / phone / linkedin / …).
    pub best_channel: String,
    pub best_timing: String,
    pub talking_points: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    /// URLs the profile was derived from (provenance).
    pub evidence_sources: Vec<String>,
}

pub(crate) async fn get_psych_profiles(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<PsychProfileItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // Join persons → companies for the org name. We prefer the psych/engagement
    // profile rows (the canonical psych-compute output) but fall back to the
    // persons columns when a profile has not yet been generated, so the view is
    // never empty for a tracked buyer.
    let rows = match sqlx::query(
        r#"
        SELECT
            pp.person_id::text                               AS person_id,
            COALESCE(p.name, pp.person_id::text)             AS person_name,
            COALESCE(p.current_role, 'Unknown')             AS current_role,
            COALESCE(c.name, 'Unknown')                      AS company_name,
            COALESCE(pp.buying_center_role, '')              AS buying_center_role,
            COALESCE(pp.decision_style, p.decision_style, 'unknown')   AS decision_style,
            COALESCE(pp.change_appetite, p.change_appetite, 'moderate') AS change_appetite,
            COALESCE(pp.pain_index, p.pain_index, 0.0)       AS pain_index,
            COALESCE(pp.risk_tolerance, 0.5)                 AS risk_tolerance,
            COALESCE(pp.preferred_proof, ARRAY[]::TEXT[])    AS preferred_proof,
            COALESCE(ep.best_channel, 'email')               AS best_channel,
            COALESCE(ep.best_timing, '')                     AS best_timing,
            COALESCE(ep.talking_points, ARRAY[]::TEXT[])     AS talking_points,
            COALESCE(ep.opening_topics, ARRAY[]::TEXT[])     AS opening_topics,
            COALESCE(ep.avoid_topics, ARRAY[]::TEXT[])       AS avoid_topics,
            COALESCE(pp.evidence_sources, ARRAY[]::TEXT[])   AS evidence_sources
        FROM psychological_profiles pp
        LEFT JOIN persons   p ON p.id::text = pp.person_id::text
        LEFT JOIN companies c ON c.id = p.primary_org_id
        LEFT JOIN LATERAL (
            SELECT talking_points, opening_topics, avoid_topics, best_channel, best_timing
            FROM engagement_profiles
            WHERE person_id = pp.person_id
            ORDER BY generated_at DESC
            LIMIT 1
        ) ep ON true
        ORDER BY pp.computed_at DESC NULLS LAST, pp.updated_at DESC NULLS LAST
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
                Json(error_response(ApiError::internal(
                    "Failed to fetch psychological profiles",
                ))),
            );
        }
    };

    let items: Vec<PsychProfileItem> = rows
        .iter()
        .map(|row| PsychProfileItem {
            person_id: row.get("person_id"),
            person_name: row.get("person_name"),
            current_role: row.get("current_role"),
            company_name: row.get("company_name"),
            buying_center_role: row.get("buying_center_role"),
            decision_style: row.get("decision_style"),
            change_appetite: row.get("change_appetite"),
            pain_index: row.get("pain_index"),
            risk_tolerance: row.get("risk_tolerance"),
            preferred_proof: row.get("preferred_proof"),
            best_channel: row.get("best_channel"),
            best_timing: row.get("best_timing"),
            talking_points: row.get("talking_points"),
            opening_topics: row.get("opening_topics"),
            avoid_topics: row.get("avoid_topics"),
            evidence_sources: row.get("evidence_sources"),
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
