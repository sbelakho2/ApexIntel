//! ICP (Ideal Customer Profile) targeting API handlers.
//!
//! - `GET /api/icp/targets` — top target accounts ranked by ICP fit score
//! - `POST /api/icp/companies/:id/score` — score (or re-score) a single account
//!
//! Backed by [`apex_insights::icp_scorer`] + the `companies.icp_fit_score` /
//! `icp_breakdown` columns added by migration `20260701`.

use crate::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

use apex_insights::icp_scorer::{IcpDefinition, IcpInput, IcpScorer};
use apex_store::postgres::{CompanyListFilters, CompanyRow};

#[derive(Debug, Serialize)]
pub struct IcpTargetItem {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub region: Option<String>,
    pub country_code: Option<String>,
    pub industry_tags: Vec<String>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub icp_fit_score: f64,
    pub intent_signal_score: f64,
    pub components: Vec<apex_insights::icp_scorer::ScoreComponent>,
}

/// Build the ICP scoring input from a stored company row.
fn icp_input_from_company(c: &CompanyRow) -> IcpInput {
    IcpInput {
        employee_estimate: c.employee_estimate,
        revenue_estimate_usd: c.revenue_estimate_usd,
        industry_tags: c.industry_tags.clone().unwrap_or_default(),
        tech_stack: Vec::new(),
        region: c.region.clone(),
        country_code: c.country_code.clone(),
        intent_signal_score: 0.0,
        strategic_relevance: c.strategic_relevance.unwrap_or(0.0).clamp(0.0, 1.0),
        funding_stage: None,
        headcount_growth_pct: None,
    }
}

/// GET /api/icp/targets — top target accounts by ICP fit.
pub(crate) async fn list_icp_targets(
    State(state): State<AppState>,
    Query(params): Query<ListIcpTargetsQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<IcpTargetItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = params.limit.unwrap_or(25).clamp(1, 200);

    // Prefer pre-scored rows (from the nightly ICP job); fall back to live scoring.
    match state.store.list_icp_top_targets(limit).await {
        Ok(rows) if !rows.is_empty() => {
            let items: Vec<IcpTargetItem> = rows
                .into_iter()
                .map(|r| {
                    let components: Vec<apex_insights::icp_scorer::ScoreComponent> = r
                        .icp_breakdown
                        .as_ref()
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    IcpTargetItem {
                        id: r.id.to_string(),
                        name: r.name,
                        domain: r.domain,
                        region: r.region,
                        country_code: r.country_code,
                        industry_tags: r.industry_tags.unwrap_or_default(),
                        employee_estimate: r.employee_estimate,
                        revenue_estimate_usd: r.revenue_estimate_usd,
                        icp_fit_score: r.icp_fit_score,
                        intent_signal_score: r.intent_signal_score,
                        components,
                    }
                })
                .collect();
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_icp_targets", duration_ms);
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
        Ok(_) => {
            // No pre-scored accounts yet — score live from the company list.
            let companies = match state
                .store
                .list_companies(
                    &CompanyListFilters {
                        is_competitor: Some(false),
                        ..Default::default()
                    },
                    None,
                    false,
                    limit,
                    0,
                )
                .await
            {
                Ok(c) => c,
                Err(err) => {
                    tracing::error!(request_id = %request_id, "list_icp_targets: companies load failed: {err:#}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(error_response(ApiError::internal(
                            "Failed to load companies",
                        ))),
                    );
                }
            };
            let def = IcpDefinition::default();
            let mut items: Vec<IcpTargetItem> = companies
                .iter()
                .map(|c| {
                    let input = icp_input_from_company(c);
                    let score = IcpScorer::score(&input, &def);
                    to_item(c, &score)
                })
                .collect();
            items.sort_by(|a, b| {
                b.icp_fit_score
                    .partial_cmp(&a.icp_fit_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            items.truncate(limit as usize);
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_icp_targets_live", duration_ms);
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
            tracing::error!(request_id = %request_id, "list_icp_targets failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to fetch ICP targets",
                ))),
            )
        }
    }
}

/// POST /api/icp/companies/:id/score — score a single account and persist.
pub(crate) async fn score_company_icp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<IcpTargetItem>>) {
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid company ID"))),
            )
        }
    };

    let company = match state.store.get_company(uid).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Company", &id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "score_company_icp: load failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load company"))),
            );
        }
    };

    let input = icp_input_from_company(&company);
    let def = IcpDefinition::default();
    let score = IcpScorer::score(&input, &def);

    let breakdown = match serde_json::to_value(&score.components) {
        Ok(v) => v,
        Err(_) => serde_json::json!([]),
    };
    if let Err(e) = state
        .store
        .update_company_icp_score(
            uid,
            score.icp_fit_score,
            score.intent_signal_score,
            &breakdown,
            None,
            None,
            None,
        )
        .await
    {
        tracing::warn!(request_id = %request_id, error = %e, "score_company_icp: persist failed");
    }

    let item = to_item(&company, &score);
    (
        StatusCode::OK,
        Json(success_with_meta(
            item,
            ResponseMeta::now().with_request_id(request_id),
        )),
    )
}

fn to_item(c: &CompanyRow, score: &apex_insights::icp_scorer::IcpScore) -> IcpTargetItem {
    IcpTargetItem {
        id: c.id.to_string(),
        name: c.name.clone(),
        domain: c.domain.clone(),
        region: c.region.clone(),
        country_code: c.country_code.clone(),
        industry_tags: c.industry_tags.clone().unwrap_or_default(),
        employee_estimate: c.employee_estimate,
        revenue_estimate_usd: c.revenue_estimate_usd,
        icp_fit_score: score.icp_fit_score,
        intent_signal_score: score.intent_signal_score,
        components: score.components.clone(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListIcpTargetsQuery {
    pub limit: Option<i64>,
}
