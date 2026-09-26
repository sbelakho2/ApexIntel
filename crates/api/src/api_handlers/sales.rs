//! Sales-activation API handlers: contact methods, engagement events, buying centers.
//!
//! - `GET /api/persons/:id/contacts` — verified contact methods for a person
//! - `POST /api/persons/:id/engagement` — record an outreach contact/outcome
//! - `GET /api/persons/:id/engagement` — outreach history + response rate
//! - `GET /api/companies/:id/buying-center` — decision-unit graph for an account
//! - `POST /api/companies/:id/buying-center/members` — add/upsert a committee member
//!
//! These close the activation loop: which person (contacts), how we reached them
//! and what happened (engagement), and the deal's decision structure (buying center).

use crate::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

// ─── Contact methods ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ContactMethodItem {
    pub id: String,
    pub contact_type: String,
    pub value: String,
    pub confidence: f64,
    pub verification_status: String,
    pub source: String,
    pub is_primary: bool,
    pub verified_at: Option<String>,
}

pub(crate) async fn list_person_contacts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<ContactMethodItem>>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid person ID"))),
            )
        }
    };
    match state.store.list_contact_methods(uid).await {
        Ok(rows) => {
            let items: Vec<ContactMethodItem> = rows
                .into_iter()
                .map(|r| ContactMethodItem {
                    id: r.id.to_string(),
                    contact_type: r.contact_type,
                    value: r.value,
                    confidence: r.confidence,
                    verification_status: r.verification_status,
                    source: r.source,
                    is_primary: r.is_primary,
                    verified_at: r.verified_at.map(|t| t.to_rfc3339()),
                })
                .collect();
            (
                StatusCode::OK,
                Json(success_with_meta(
                    items,
                    ResponseMeta::now().with_request_id(request_id),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "list_person_contacts failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch contacts",
                ))),
            )
        }
    }
}

// ─── Engagement events ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RecordEngagementRequest {
    pub channel: String,
    pub direction: Option<String>,
    pub outcome: String,
    pub outcome_weight: Option<f64>,
    pub subject: Option<String>,
    pub opportunity_id: Option<String>,
    pub cadence_step: Option<i32>,
    pub owner_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EngagementEventItem {
    pub id: String,
    pub channel: String,
    pub direction: String,
    pub outcome: String,
    pub outcome_weight: f64,
    pub subject: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Serialize)]
pub struct EngagementSummary {
    pub person_id: String,
    pub total_contacts: i64,
    pub responses: i64,
    pub response_rate: f64,
    pub events: Vec<EngagementEventItem>,
}

pub(crate) async fn record_engagement(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RecordEngagementRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let person_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid person ID"))),
            )
        }
    };

    // Map the outcome enum to a real weight if not supplied, mirroring
    // poi::engagement_tracker::EngagementOutcome weights.
    let weight = payload
        .outcome_weight
        .unwrap_or_else(|| match payload.outcome.as_str() {
            "positive" | "meeting_booked" => 1.0,
            "reply" => 0.7,
            "neutral" => 0.0,
            "no_response" => -0.3,
            "negative" | "bounce" => -1.0,
            _ => 0.0,
        });

    let opportunity_id = match &payload.opportunity_id {
        Some(o) => Some(match Uuid::parse_str(o) {
            Ok(u) => u,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::bad_request(
                        "Invalid opportunity_id",
                    ))),
                )
            }
        }),
        None => None,
    };

    let new_evt = apex_store::postgres::NewEngagementEvent {
        person_id,
        opportunity_id,
        channel: payload.channel,
        direction: payload.direction.unwrap_or_else(|| "outbound".to_string()),
        outcome: payload.outcome,
        outcome_weight: weight,
        subject: payload.subject,
        message_ref: None,
        cadence_step: payload.cadence_step,
        occurred_at: chrono::Utc::now(),
        owner_id: payload.owner_id,
        metadata: serde_json::json!({}),
    };

    match state.store.record_engagement_event(&new_evt).await {
        Ok(eid) => (
            StatusCode::CREATED,
            Json(success_with_meta(
                serde_json::json!({"id": eid.to_string(), "recorded": true}),
                ResponseMeta::now().with_request_id(request_id),
            )),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "record_engagement failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to record engagement",
                ))),
            )
        }
    }
}

pub(crate) async fn list_person_engagement(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<EngagementSummary>>) {
    let request_id = Uuid::new_v4().to_string();
    let start = Instant::now();
    let person_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid person ID"))),
            )
        }
    };

    let events = match state.store.list_engagement_events(person_id, 100).await {
        Ok(e) => e,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list_person_engagement failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch engagement history",
                ))),
            );
        }
    };

    let total = events.len() as i64;
    let responses = events
        .iter()
        .filter(|e| matches!(e.outcome.as_str(), "positive" | "reply" | "meeting_booked"))
        .count() as i64;
    let response_rate = if total > 0 {
        responses as f64 / total as f64
    } else {
        0.0
    };

    let items: Vec<EngagementEventItem> = events
        .into_iter()
        .map(|e| EngagementEventItem {
            id: e.id.to_string(),
            channel: e.channel,
            direction: e.direction,
            outcome: e.outcome,
            outcome_weight: e.outcome_weight,
            subject: e.subject,
            occurred_at: e.occurred_at.to_rfc3339(),
        })
        .collect();

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_person_engagement", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            EngagementSummary {
                person_id: id,
                total_contacts: total,
                responses,
                response_rate,
                events: items,
            },
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

// ─── Buying center ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BuyingCenterView {
    pub id: String,
    pub name: String,
    pub status: String,
    pub deal_value: Option<f64>,
    pub members: Vec<BuyingCenterMemberView>,
}

#[derive(Debug, Serialize)]
pub struct BuyingCenterMemberView {
    pub id: String,
    pub person_id: String,
    pub role: String,
    pub influence_score: f64,
    pub budget_authority: bool,
    pub need_signal: f64,
}

pub(crate) async fn list_company_buying_center(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<BuyingCenterView>>>) {
    let request_id = Uuid::new_v4().to_string();
    let company_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid company ID"))),
            )
        }
    };

    let centers = match state.store.list_buying_centers(company_id, 20).await {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list_company_buying_center failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch buying centers",
                ))),
            );
        }
    };

    let mut views: Vec<BuyingCenterView> = Vec::with_capacity(centers.len());
    for c in centers {
        let members = state
            .store
            .list_buying_center_members(c.id)
            .await
            .unwrap_or_default();
        let member_views: Vec<BuyingCenterMemberView> = members
            .into_iter()
            .map(|m| BuyingCenterMemberView {
                id: m.id.to_string(),
                person_id: m.person_id.to_string(),
                role: m.role,
                influence_score: m.influence_score,
                budget_authority: m.budget_authority,
                need_signal: m.need_signal,
            })
            .collect();
        views.push(BuyingCenterView {
            id: c.id.to_string(),
            name: c.name,
            status: c.status,
            deal_value: c.deal_value,
            members: member_views,
        });
    }

    (
        StatusCode::OK,
        Json(success_with_meta(
            views,
            ResponseMeta::now().with_request_id(request_id),
        )),
    )
}

#[derive(Debug, Deserialize)]
pub struct AddBuyingMemberRequest {
    pub person_id: String,
    pub role: String,
    pub influence_score: Option<f64>,
    pub budget_authority: Option<bool>,
    pub need_signal: Option<f64>,
    pub timeline_horizon: Option<String>,
    pub notes: Option<String>,
    pub opportunity_id: Option<String>,
}

pub(crate) async fn add_buying_member(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<AddBuyingMemberRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let company_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid company ID"))),
            )
        }
    };
    let person_id = match Uuid::parse_str(&payload.person_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid person_id"))),
            )
        }
    };
    let opportunity_id = match &payload.opportunity_id {
        Some(o) => Some(match Uuid::parse_str(o) {
            Ok(u) => u,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::bad_request(
                        "Invalid opportunity_id",
                    ))),
                )
            }
        }),
        None => None,
    };

    let bc_id = match state
        .store
        .upsert_buying_center(
            company_id,
            opportunity_id,
            &format!("{} Buying Center", id),
            None,
        )
        .await
    {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(request_id = %request_id, "add_buying_member: upsert center failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to get/create buying center",
                ))),
            );
        }
    };

    let member = apex_store::postgres::NewBuyingMember {
        buying_center_id: bc_id,
        person_id,
        role: payload.role,
        influence_score: payload.influence_score.unwrap_or(0.5),
        budget_authority: payload.budget_authority.unwrap_or(false),
        need_signal: payload.need_signal.unwrap_or(0.0),
        timeline_horizon: payload.timeline_horizon,
        notes: payload.notes,
        metadata: serde_json::json!({}),
    };

    match state.store.upsert_buying_center_member(&member).await {
        Ok(mid) => (
            StatusCode::CREATED,
            Json(success_with_meta(
                serde_json::json!({"id": mid.to_string(), "buying_center_id": bc_id.to_string()}),
                ResponseMeta::now().with_request_id(request_id),
            )),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "add_buying_member failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to add buying member",
                ))),
            )
        }
    }
}
