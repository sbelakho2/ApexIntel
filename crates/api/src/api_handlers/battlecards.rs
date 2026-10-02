//! Battlecards API handlers — full CRUD + regenerate + export.
//!
//! Uses `PgStore` battlecard methods directly (not activity_feed).

use crate::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

use apex_insights::entity_relevance::EntityProfile;
use apex_store::postgres::{BattlecardWriteOutcome, CompanyRow, CreateBattlecardOutcome};

/// GET /api/battlecards — list all battlecards with optional filters.
#[derive(Debug, Deserialize)]
pub struct ListBattlecardsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BattlecardItem {
    pub id: String,
    pub account_name: String,
    pub status: String,
    pub competitor_id: String,
    /// Absent when the competitor company row cannot be resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub competitor_name: Option<String>,
    /// Derived from battlecard content (weakness/kill-shot density).
    /// Absent when the card has not been generated yet (B314) — the previous
    /// implementation returned hardcoded "medium"/0.5 for every card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threat_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win_probability: Option<f64>,
    pub competitor_count: u32,
    pub key_intel: String,
    pub last_updated: Option<String>,
}

/// Derive an honest threat level from generated battlecard content.
fn derive_threat_level(
    weaknesses: Option<&serde_json::Value>,
    kill_shots: Option<&serde_json::Value>,
) -> Option<String> {
    let weaknesses_count = weaknesses
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let kill_shot_count = kill_shots
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if weaknesses_count == 0 && kill_shot_count == 0 {
        return None;
    }
    let score = weaknesses_count + kill_shot_count * 2;
    Some(
        if score >= 6 {
            "high"
        } else if score >= 2 {
            "medium"
        } else {
            "low"
        }
        .to_string(),
    )
}

/// Extract win rate from the generated `win_loss` section when present.
fn derive_win_probability(win_loss: Option<&serde_json::Value>) -> Option<f64> {
    win_loss
        .and_then(|v| {
            v.get("win_rate")
                .or_else(|| v.get("win_probability"))
                .and_then(|rate| rate.as_f64())
        })
        .map(|rate| rate.clamp(0.0, 1.0))
}

pub(crate) async fn list_battlecards(
    State(state): State<AppState>,
    Query(params): Query<ListBattlecardsQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<BattlecardItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 100);
    let status = match params
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => match apex_api::routes::battlecards::normalize_status(raw) {
            Some(status) => Some(status),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::validation(
                        "status",
                        "status must be one of draft, published, archived",
                    ))),
                )
            }
        },
    };

    match state
        .store
        .list_battlecards(status, None, page, per_page)
        .await
    {
        Ok(rows) => {
            let competitor_ids: Vec<Uuid> = rows.iter().map(|r| r.competitor_id).collect();
            // Names are display-only enrichment: a lookup failure leaves
            // `competitor_name` absent rather than failing the list.
            let competitor_names: std::collections::HashMap<Uuid, String> = if competitor_ids
                .is_empty()
            {
                std::collections::HashMap::new()
            } else {
                match state.store.get_company_names_by_ids(&competitor_ids).await {
                    Ok(names) => names
                        .into_iter()
                        .map(|(id, name, _region, _company_type)| (id, name))
                        .collect(),
                    Err(err) => {
                        tracing::warn!(request_id = %request_id, "battlecard competitor name lookup failed: {err:#}");
                        std::collections::HashMap::new()
                    }
                }
            };
            let items: Vec<BattlecardItem> = rows
                .into_iter()
                .map(|row| BattlecardItem {
                    id: row.id.to_string(),
                    account_name: row.title,
                    status: row.status,
                    competitor_id: row.competitor_id.to_string(),
                    competitor_name: competitor_names.get(&row.competitor_id).cloned(),
                    threat_level: derive_threat_level(
                        row.weaknesses.as_ref(),
                        row.kill_shots.as_ref(),
                    ),
                    win_probability: derive_win_probability(row.win_loss.as_ref()),
                    // A battlecard targets exactly one competitor.
                    competitor_count: 1,
                    key_intel: row
                        .positioning
                        .as_ref()
                        .and_then(|v| v.get("summary"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("No intel available")
                        .to_string(),
                    last_updated: Some(row.updated_at.to_rfc3339()),
                })
                .collect();

            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_battlecards", duration_ms);
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
        Err(err) => {
            tracing::error!(request_id = %request_id, "list_battlecards failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch battlecards",
                ))),
            )
        }
    }
}

/// POST /api/battlecards — create a new battlecard.
#[derive(Debug, Deserialize)]
pub struct CreateBattlecardRequest {
    pub our_company_id: String,
    pub competitor_id: String,
    pub title: String,
}

pub(crate) async fn create_battlecard(
    State(state): State<AppState>,
    Json(payload): Json<CreateBattlecardRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let our_id = match Uuid::parse_str(&payload.our_company_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid our_company_id",
                ))),
            )
        }
    };
    let comp_id = match Uuid::parse_str(&payload.competitor_id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid competitor_id",
                ))),
            )
        }
    };

    if our_id == comp_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::validation(
                "competitor_id",
                "competitor_id must differ from our_company_id",
            ))),
        );
    }
    let title = match apex_api::routes::battlecards::validate_title(&payload.title) {
        Ok(title) => title,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::validation("title", message))),
            )
        }
    };

    match state.store.create_battlecard(our_id, comp_id, &title).await {
        Ok(CreateBattlecardOutcome::Created(id)) => (
            StatusCode::CREATED,
            Json(success_with_meta(
                serde_json::json!({"id": id.to_string()}),
                ResponseMeta::now().with_request_id(request_id),
            )),
        ),
        Ok(CreateBattlecardOutcome::Duplicate(existing)) => {
            let mut details = std::collections::BTreeMap::new();
            details.insert("existing_id".to_string(), existing.to_string());
            (
                StatusCode::CONFLICT,
                Json(error_response(
                    ApiError::new(
                        ErrorCode::Conflict,
                        "A battlecard for this company pair already exists",
                    )
                    .with_details(details),
                )),
            )
        }
        Ok(CreateBattlecardOutcome::UnknownCompany) => (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::bad_request(
                "our_company_id or competitor_id does not reference an existing company",
            ))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "create_battlecard failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create battlecard",
                ))),
            )
        }
    }
}

/// GET /api/battlecards/:id — get a single battlecard.
pub(crate) async fn get_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };

    match state.store.get_battlecard(uid).await {
        Ok(Some(row)) => {
            let body = serde_json::json!({
                "id": row.id.to_string(),
                "title": row.title,
                "status": row.status,
                "competitor_id": row.competitor_id.to_string(),
                "positioning": row.positioning,
                "pricing": row.pricing,
                "feature_matrix": row.feature_matrix,
                "strengths": row.strengths,
                "weaknesses": row.weaknesses,
                "objection_handlers": row.objection_handlers,
                "kill_shots": row.kill_shots,
                "recent_news": row.recent_news,
                "win_loss": row.win_loss,
                "created_at": row.created_at.to_rfc3339(),
                "updated_at": row.updated_at.to_rfc3339(),
            });
            (
                StatusCode::OK,
                Json(success_with_meta(
                    body,
                    ResponseMeta::now().with_request_id(request_id),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Battlecard", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "get_battlecard failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch battlecard",
                ))),
            )
        }
    }
}

/// DELETE /api/battlecards/:id — delete a battlecard.
pub(crate) async fn delete_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };

    match state.store.delete_battlecard(uid).await {
        Ok(true) => (
            StatusCode::OK,
            Json(success_with_meta(
                serde_json::json!({"deleted": true}),
                ResponseMeta::now().with_request_id(request_id),
            )),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Battlecard", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete_battlecard failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to delete battlecard",
                ))),
            )
        }
    }
}

/// PUT /api/battlecards/:id/section — update a battlecard section.
#[derive(Debug, Deserialize)]
pub struct UpdateSectionRequest {
    pub section: String,
    pub data: serde_json::Value,
}

pub(crate) async fn update_battlecard_section(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateSectionRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };

    match state
        .store
        .update_battlecard_section(uid, &payload.section, &payload.data, auth.user_id.as_str())
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(success_with_meta(
                serde_json::json!({"updated": true}),
                ResponseMeta::now().with_request_id(request_id),
            )),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Battlecard", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "update_battlecard_section failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update battlecard section",
                ))),
            )
        }
    }
}

/// PATCH /api/battlecards/:id — update title, status and/or sections.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBattlecardRequest {
    pub title: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub sections: std::collections::BTreeMap<String, serde_json::Value>,
    /// Optimistic concurrency: when set, the update only applies if the
    /// battlecard's `updated_at` still equals this value (409 otherwise).
    pub expected_updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) async fn patch_battlecard(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(payload): Json<PatchBattlecardRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };
    let title = match payload.title.as_deref() {
        None => None,
        Some(raw) => match apex_api::routes::battlecards::validate_title(raw) {
            Ok(title) => Some(title),
            Err(message) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::validation("title", message))),
                )
            }
        },
    };
    let status = match payload.status.as_deref() {
        None => None,
        Some(raw) => match apex_api::routes::battlecards::normalize_status(raw) {
            Some(status) => Some(status),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::validation(
                        "status",
                        "status must be one of draft, published, archived",
                    ))),
                )
            }
        },
    };
    let mut sections: Vec<(&str, serde_json::Value)> = Vec::with_capacity(payload.sections.len());
    for (name, value) in &payload.sections {
        match apex_api::routes::battlecards::SECTIONS
            .iter()
            .find(|(section, _)| section == name)
        {
            Some((section, _)) => sections.push((section, value.clone())),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(ApiError::validation(
                        "sections",
                        format!("unknown battlecard section '{name}'"),
                    ))),
                )
            }
        }
    }
    if title.is_none() && status.is_none() && sections.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::bad_request(
                "Provide at least one of title, status or sections",
            ))),
        );
    }

    match state
        .store
        .update_battlecard_details(
            uid,
            title.as_deref(),
            status,
            &sections,
            auth.user_id.as_str(),
            payload.expected_updated_at,
        )
        .await
    {
        Ok(BattlecardWriteOutcome::Updated) => match state.store.get_battlecard(uid).await {
            Ok(Some(row)) => (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::to_value(apex_api::routes::battlecards::BattlecardResponse::from(
                        row,
                    ))
                    .unwrap_or(serde_json::Value::Null),
                    ResponseMeta::now().with_request_id(request_id),
                )),
            ),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Battlecard", &id))),
            ),
            Err(err) => {
                tracing::error!(request_id = %request_id, "patch_battlecard reload failed: {err:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Battlecard updated but could not be reloaded",
                    ))),
                )
            }
        },
        Ok(BattlecardWriteOutcome::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Battlecard", &id))),
        ),
        Ok(BattlecardWriteOutcome::Conflict) => (
            StatusCode::CONFLICT,
            Json(error_response(ApiError::new(
                ErrorCode::Conflict,
                "Battlecard changed since expected_updated_at; reload and retry",
            ))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "patch_battlecard failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update battlecard",
                ))),
            )
        }
    }
}

/// Build an [`EntityProfile`] from a stored [`CompanyRow`] so the battlecard
/// engine can reason about real firmographics/keywords.
fn entity_profile_from_company(c: &CompanyRow) -> EntityProfile {
    let industry: Vec<String> = c.industry_tags.clone().unwrap_or_default();
    let category = infer_category(&industry);
    EntityProfile {
        entity_name: c.name.clone(),
        industry_keywords: industry.clone(),
        product_keywords: industry.clone(),
        geographic_keywords: c.region.clone().map(|r| vec![r]).unwrap_or_default(),
        competitor_keywords: Vec::new(),
        topic_keywords: industry,
        category,
        country_code: c.country_code.clone(),
        ticker: None,
        is_dynamically_discovered: false,
        last_verified: c.updated_at,
        verification_count: 0,
    }
}

/// Map industry tags to the closest [`EntityCategory`].
fn infer_category(industry: &[String]) -> apex_insights::entity_relevance::EntityCategory {
    use apex_insights::entity_relevance::EntityCategory;
    let has = |kw: &str| industry.iter().any(|t| t.to_lowercase().contains(kw));
    if has("semiconductor") {
        EntityCategory::Semiconductor
    } else if has("ems") || has("manufacturing service") {
        EntityCategory::Ems
    } else if has("oem") {
        EntityCategory::Oem
    } else if has("automotive") || has("vehicle") {
        EntityCategory::Automotive
    } else if has("logistic") || has("distribut") || has("freight") {
        EntityCategory::Logistics
    } else if has("software") || has("technology") || has("saas") || has("platform") {
        EntityCategory::Technology
    } else {
        EntityCategory::Other("company".to_string())
    }
}

/// POST /api/battlecards/:id/regenerate — regenerate a battlecard from real data.
///
/// Loads closed deals + competitor pricing + recent insights, runs the
/// [`BattlecardEngine`] (data-driven win/loss + pricing + feature matrix), and
/// optionally layers LLM-grounded narrative on positioning/objections/strengths
/// when the `llm` feature is enabled and a model is reachable. Every regenerated
/// section is persisted back to the battlecard's JSONB columns atomically.
/// HTMX callers receive `HX-Refresh: true` so the page re-renders the new
/// content.
pub(crate) async fn regenerate_battlecard(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let htmx = headers.contains_key("hx-request");
    let (status, body) = regenerate_battlecard_inner(&state, &auth, &id).await;
    let mut response = (status, body).into_response();
    if htmx && status.is_success() {
        response
            .headers_mut()
            .insert("HX-Refresh", axum::http::HeaderValue::from_static("true"));
    }
    response
}

async fn regenerate_battlecard_inner(
    state: &AppState,
    auth: &ApiAuthContext,
    id: &str,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };

    // 1. Load the battlecard + both companies.
    let bc = match state.store.get_battlecard(uid).await {
        Ok(Some(bc)) => bc,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Battlecard", id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "regenerate_battlecard: load failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load battlecard",
                ))),
            );
        }
    };

    let our_company = match state.store.get_company(bc.our_company_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found(
                    "Company",
                    &bc.our_company_id.to_string(),
                ))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "regenerate_battlecard: company load failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load a battlecard input",
                ))),
            );
        }
    };
    let competitor = match state.store.get_company(bc.competitor_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found(
                    "Company",
                    &bc.competitor_id.to_string(),
                ))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "regenerate_battlecard: company load failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load a battlecard input",
                ))),
            );
        }
    };

    // 2. Load REAL closed deals + competitor pricing for the battlecard context.
    let deal_rows = match state
        .store
        .list_closed_deals(bc.our_company_id, Some(bc.competitor_id), 200)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "battlecard generation: input load failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load a battlecard input",
                ))),
            );
        }
    };
    let closed_deals: Vec<apex_insights::battlecards::ClosedDeal> = deal_rows
        .iter()
        .map(|d| apex_insights::battlecards::ClosedDeal {
            deal_name: d.deal_name.clone(),
            value: d.deal_value,
            won: d.won,
            loss_reason: d.loss_reason.clone(),
            competitor_name: competitor.name.clone(),
            closed_at: d.closed_at,
        })
        .collect();

    let pricing_rows = match state.store.list_competitor_pricing(bc.competitor_id).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "battlecard generation: input load failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load a battlecard input",
                ))),
            );
        }
    };
    let pricing: Vec<apex_insights::battlecards::PricingObservation> = pricing_rows
        .iter()
        .map(|p| apex_insights::battlecards::PricingObservation {
            product_category: p.product_category.clone(),
            pricing_model: p.pricing_model.clone(),
            price_range_low: p.price_range_low,
            price_range_high: p.price_range_high,
            currency: p.currency.clone(),
            average_contract_value: p.average_contract_value,
            discounting_behavior: p.discounting_behavior.clone(),
            competitive_position: p.competitive_position.clone(),
            confidence: p.confidence,
            evidence_url: p.evidence_url.clone(),
        })
        .collect();

    let ctx = apex_insights::battlecards::BattlecardContext {
        closed_deals: closed_deals.clone(),
        pricing: pricing.clone(),
    };

    // 3. Load recent insights mentioning the competitor (evidence for LLM).
    let filters = apex_store::postgres::InsightListFilters {
        search: Some(competitor.name.clone()),
        ..Default::default()
    };
    let insight_rows = match state.store.list_insights(&filters, 20, 0).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "battlecard generation: input load failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load a battlecard input",
                ))),
            );
        }
    };
    let insights: Vec<apex_insights::Insight> = insight_rows
        .iter()
        .map(|r| {
            apex_insights::Insight::new(&r.title, &r.summary)
                .with_confidence(r.confidence.unwrap_or(0.5))
        })
        .collect();

    // 4. Run the data-driven battlecard engine.
    let comp_profile = entity_profile_from_company(&competitor);
    let our_profile = entity_profile_from_company(&our_company);
    let engine = apex_insights::battlecards::BattlecardEngine::new();
    // Mutated below only when the `llm` feature is enabled.
    #[cfg_attr(not(feature = "llm"), allow(unused_mut))]
    let mut data = engine
        .generate_full_battlecard(
            &comp_profile,
            &our_profile,
            &insights,
            &ctx,
            bc.our_company_id,
            bc.competitor_id,
        )
        .await;

    // 5. Optional LLM-grounded narrative enrichment.
    #[cfg_attr(not(feature = "llm"), allow(unused_mut))]
    let mut llm_used = false;
    #[cfg(feature = "llm")]
    {
        match apex_llm::inference::LlmClient::from_env() {
            Err(error) => {
                tracing::debug!(%error, "regenerate_battlecard: LLM not configured; data-driven sections only");
            }
            Ok(client) => match apex_insights::battlecards::llm_sections::synthesize_llm_sections(
                &client,
                &our_company.name,
                &competitor.name,
                &insights,
                3,
            )
            .await
            {
                Ok(Some(llm)) => {
                    llm_used = true;
                    data.positioning = apex_insights::battlecards::llm_sections::enrich_positioning(
                        data.positioning,
                        &llm,
                    );
                    data.strengths = apex_insights::battlecards::llm_sections::merge_strengths(
                        data.strengths,
                        &llm.strengths,
                    );
                    data.weaknesses = apex_insights::battlecards::llm_sections::merge_weaknesses(
                        data.weaknesses,
                        &llm.weaknesses,
                    );
                    if !llm.objection_handlers.is_empty() {
                        data.objection_handlers = llm.objection_handlers;
                    }
                }
                Ok(None) => {
                    tracing::debug!(request_id = %request_id, "regenerate_battlecard: LLM produced no grounded sections");
                }
                Err(error) => {
                    tracing::warn!(request_id = %request_id, "regenerate_battlecard: LLM enrichment failed, keeping data-driven sections: {error:#}");
                }
            },
        }
    }

    // 6. Persist every regenerated section atomically (all sections plus
    //    `regenerated_at`, or nothing).
    fn section_json<T: Serialize>(
        name: &'static str,
        value: &T,
    ) -> Result<(&'static str, serde_json::Value), String> {
        serde_json::to_value(value)
            .map(|json| (name, json))
            .map_err(|e| format!("{name}: {e}"))
    }
    let sections: Result<Vec<(&str, serde_json::Value)>, String> = [
        section_json("positioning", &data.positioning),
        section_json("pricing", &data.pricing),
        section_json("feature_matrix", &data.feature_matrix),
        section_json("strengths", &data.strengths),
        section_json("weaknesses", &data.weaknesses),
        section_json("objection_handlers", &data.objection_handlers),
        section_json("kill_shots", &data.kill_shots),
        section_json("recent_news", &data.recent_news),
        section_json("win_loss", &data.win_loss),
    ]
    .into_iter()
    .collect();
    let sections = match sections {
        Ok(sections) => sections,
        Err(error) => {
            tracing::error!(request_id = %request_id, %error, "regenerate_battlecard: section serialization failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to serialize the regenerated battlecard",
                ))),
            );
        }
    };
    match state
        .store
        .apply_battlecard_regeneration(uid, &sections, auth.user_id.as_str())
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Battlecard", id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, battlecard_id = %uid, "regenerate_battlecard: persist failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to save the regenerated battlecard",
                ))),
            );
        }
    }

    // 7. Activity-feed echo. The card is already saved, so a failure here is
    //    logged rather than turned into a request failure.
    let details = serde_json::json!({
        "logged_at": chrono::Utc::now().to_rfc3339(),
        "battlecard_id": uid.to_string(),
        "competitor": &competitor.name,
        "deals_analyzed": closed_deals.len(),
        "pricing_points": pricing.len(),
        "llm_enriched": llm_used,
        "summary": format!("Battlecard regenerated for competitor {} from {} deals + {} pricing points", competitor.name, closed_deals.len(), pricing.len()),
    });
    if let Err(error) = sqlx::query(
        r#"INSERT INTO activity_feed
             (actor_id, actor_name, action_type, entity_type, entity_id, entity_name,
              details, workspace_id, team_id, visibility, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())"#,
    )
    .bind(auth.user_id.as_str())
    .bind("Battlecard Generator")
    .bind("battlecard_generated")
    .bind(Some("company"))
    .bind(bc.competitor_id.to_string())
    .bind(&bc.title)
    .bind(&details)
    .bind(None::<uuid::Uuid>)
    .bind(None::<&str>)
    .bind("team")
    .execute(&state.store.pool)
    .await
    {
        tracing::warn!(
            request_id = %request_id,
            battlecard_id = %uid,
            %error,
            "regenerate_battlecard: activity feed insert failed"
        );
    }

    (
        StatusCode::OK,
        Json(success_with_meta(
            serde_json::json!({
                "regenerated": true,
                "id": id,
                "deals_analyzed": closed_deals.len(),
                "pricing_points": pricing.len(),
                "llm_enriched": llm_used,
            }),
            ResponseMeta::now().with_request_id(request_id),
        )),
    )
}

/// GET /api/battlecards/:id/export — export a battlecard.
pub(crate) async fn export_battlecard(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid battlecard ID",
                ))),
            )
        }
    };

    match state.store.get_battlecard(uid).await {
        Ok(Some(row)) => {
            let names = match apex_api::routes::battlecards::company_names(
                &state.store,
                &[row.competitor_id, row.our_company_id],
            )
            .await
            {
                Ok(names) => names,
                Err(err) => {
                    tracing::error!(request_id = %request_id, "export_battlecard company lookup failed: {err:#}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(error_response(ApiError::internal(
                            "Failed to export battlecard",
                        ))),
                    );
                }
            };
            let content = apex_api::routes::battlecards::render_markdown(
                &row,
                names.get(&row.competitor_id).map(String::as_str),
                names.get(&row.our_company_id).map(String::as_str),
            );
            let body = serde_json::json!({
                "format": "markdown",
                "filename": format!("{}.md", apex_api::routes::battlecards::export_filename(&row.title)),
                "content": content,
            });
            (
                StatusCode::OK,
                Json(success_with_meta(
                    body,
                    ResponseMeta::now().with_request_id(request_id),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Battlecard", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "export_battlecard failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to export battlecard",
                ))),
            )
        }
    }
}
