#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::*;
use apex_api::destructive_actions::{
    authorize_delete_all_warnings, delete_all_warnings_audit_payload,
};
use apex_store::postgres::{AcknowledgeWarningResult, WarningReviewOutcome};
use axum::http::HeaderMap;

#[derive(Debug, Deserialize)]
struct BulkDeleteRequest {
    ids: Vec<String>,
}

fn parse_bulk_delete_ids(body: &[u8]) -> Result<(Vec<Uuid>, usize), ApiError> {
    let body_value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| ApiError::bad_request("invalid JSON body"))?;

    let req: BulkDeleteRequest = serde_json::from_value(body_value)
        .map_err(|_| ApiError::validation("body", "expected { ids: string[] }"))?;

    if req.ids.is_empty() {
        return Err(ApiError::validation("ids", "at least one ID required"));
    }

    if req.ids.len() > 1000 {
        return Err(ApiError::validation("ids", "maximum 1000 IDs per request"));
    }

    let mut parsed_ids = Vec::with_capacity(req.ids.len());
    for id_str in &req.ids {
        match Uuid::parse_str(id_str) {
            Ok(uuid) => parsed_ids.push(uuid),
            Err(_) => {
                return Err(ApiError::validation(
                    "ids",
                    format!("invalid UUID: {}", id_str),
                ));
            }
        }
    }

    Ok((parsed_ids, req.ids.len()))
}

fn review_outcome_as_str(outcome: &WarningReviewOutcome) -> &'static str {
    match outcome {
        WarningReviewOutcome::TruePositive => "true_positive",
        WarningReviewOutcome::FalsePositive => "false_positive",
        WarningReviewOutcome::Inconclusive => "inconclusive",
    }
}

pub(crate) async fn list_warnings(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Query(params): Query<ListWarningsQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<WarningResponse>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    let date_from = match parse_date_start(&params.date_from) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    let date_to = match parse_query_date(&params.date_to, "date_to") {
        Ok(value) => value,
        Err(api_err) => {
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };

    let regions = parse_csv_upper_strict(params.regions.as_deref());
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let severities = parse_csv_lower_strict(params.severities.as_deref());
    if let Err(api_err) = validate_severity_codes(&severities) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let warning_types = parse_csv_lower_strict(params.warning_types.as_deref());
    if let Err(api_err) = validate_warning_type_codes(&warning_types) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }

    if params.include_deleted == Some(true) && !auth_ctx.role.can_admin() {
        let api_err = ApiError::forbidden("Admin role required to include deleted warnings");
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::FORBIDDEN),
            Json(error_response(api_err)),
        );
    }

    let filters = WarningListFilters {
        regions,
        severities,
        warning_types,
        exclude_hygiene_signals: false,
        acknowledged: params.acknowledged,
        date_from,
        date_to,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(value) => value,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        include_deleted: params.include_deleted.unwrap_or(false) && auth_ctx.role.can_admin(),
    };

    let order_by = params
        .sort_by
        .clone()
        .map(map_warning_sort)
        .unwrap_or(WarningOrderBy::CreatedAt);
    let desc = params.sort_dir.clone().unwrap_or_default() == SortDirection::Desc;

    let mut total = match tracing::info_span!("db.count_warnings", request_id = %request_id)
        .in_scope(|| state.store.count_warnings(&filters))
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to count warnings");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let mut resolved_page = clamp_page(page, per_page, total);
    let mut rows = match fetch_warning_page(
        &state,
        &filters,
        order_by,
        desc,
        per_page,
        resolved_page,
        &request_id,
    )
    .await
    {
        Ok(rows) => rows,
        Err(response) => return *response,
    };

    if rows.is_empty() && total > 0 && resolved_page > 1 {
        total = match tracing::info_span!("db.count_warnings_refresh", request_id = %request_id)
            .in_scope(|| state.store.count_warnings(&filters))
            .await
        {
            Ok(value) => value.max(0) as u64,
            Err(err) => {
                tracing::error!(request_id = %request_id, "refresh warning count failed: {err:#}");
                let api_err = ApiError::internal("Failed to count warnings");
                return (
                    StatusCode::from_u16(api_err.http_status())
                        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    Json(error_response(api_err)),
                );
            }
        };
        resolved_page = clamp_page(page, per_page, total);

        loop {
            rows = match fetch_warning_page(
                &state,
                &filters,
                order_by,
                desc,
                per_page,
                resolved_page,
                &request_id,
            )
            .await
            {
                Ok(rows) => rows,
                Err(response) => return *response,
            };
            if !rows.is_empty() || resolved_page == 1 {
                break;
            }
            resolved_page -= 1;
        }
    }

    let items = rows.into_iter().map(warning_row_to_response).collect();
    let payload = PagedResponse {
        items,
        total,
        page: resolved_page,
        per_page,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_warnings", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

type WarningPageError = (
    StatusCode,
    Json<ApiResponse<PagedResponse<WarningResponse>>>,
);

async fn fetch_warning_page(
    state: &AppState,
    filters: &WarningListFilters,
    order_by: WarningOrderBy,
    desc: bool,
    per_page: u32,
    page: u32,
    request_id: &str,
) -> Result<Vec<WarningRow>, Box<WarningPageError>> {
    let offset = ((page - 1) as i64).saturating_mul(per_page as i64);
    match tracing::info_span!("db.list_warnings", request_id = %request_id, page = page)
        .in_scope(|| {
            state
                .store
                .list_warnings(filters, Some(order_by), desc, per_page as i64, offset)
        })
        .await
    {
        Ok(value) => Ok(value),
        Err(err) => {
            tracing::error!(request_id = %request_id, page = page, "list warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to list warnings");
            Err(Box::new((
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )))
        }
    }
}

pub(crate) async fn acknowledge_warning(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> (
    StatusCode,
    Json<ApiResponse<apex_api::routes::warnings::AcknowledgeResponse>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let id_parsed = match validate_warning_id(&id) {
        Ok(uuid) => uuid,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let body_value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::bad_request("invalid JSON body");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    if let Err(msg) = validate_json_depth(&body_value, MAX_JSON_DEPTH) {
        let err = ApiError::validation("body", msg);
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }
    let body: AcknowledgeRequest = match serde_json::from_value(body_value) {
        Ok(v) => v,
        Err(_) => {
            let err = ApiError::validation("body", "invalid request schema");
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(error_response(err)),
            );
        }
    };

    if let Err(msg) = validate_acknowledge(&body) {
        let err = ApiError::validation("body", msg);
        return (
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            Json(error_response(err)),
        );
    }

    let review_outcome = body.review_outcome.as_ref().map(review_outcome_as_str);

    match tracing::info_span!("db.acknowledge_warning", request_id = %request_id)
        .in_scope(|| {
            state.store.acknowledge_warning(
                id_parsed,
                body.user_id.trim(),
                body.note.as_deref(),
                review_outcome,
            )
        })
        .await
    {
        Ok(AcknowledgeWarningResult::Acknowledged | AcknowledgeWarningResult::ReviewedExisting) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "warning_acknowledged",
                    &serde_json::json!({
                        "warning_id": id_parsed,
                        "acknowledged_by": body.user_id,
                        "review_outcome": review_outcome,
            // false-success-classification: best-effort — optional boolean default; absence is not a failure
                        "has_note": body.note.as_ref().map(|note| !note.trim().is_empty()).unwrap_or(false)
                    }),
                )
                .await;
            let now = Utc::now();
            let resp = apex_api::routes::warnings::AcknowledgeResponse {
                warning_id: id_parsed.to_string(),
                acknowledged: true,
                acknowledged_by: body.user_id.clone(),
                acknowledged_at: now,
                review_outcome: review_outcome.map(str::to_string),
                reviewed_by: review_outcome.map(|_| body.user_id.clone()),
                reviewed_at: review_outcome.map(|_| now),
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("acknowledge_warning", duration_ms);
            (StatusCode::OK, Json(success_with_meta(resp, meta)))
        }
        Ok(AcknowledgeWarningResult::AlreadyAcknowledged) => {
            let err = ApiError::new(
                apex_api::responses::ErrorCode::Conflict,
                "Warning is already acknowledged",
            );
            (StatusCode::CONFLICT, Json(error_response(err)))
        }
        Ok(AcknowledgeWarningResult::NotFound) => {
            let err = ApiError::not_found("warning", &id);
            (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(err)),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "acknowledge warning failed: {err:#}");
            let api_err = ApiError::internal("Failed to acknowledge warning");
            (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

pub(crate) async fn delete_warning(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    // B292: destructive warning operations are documented admin-only — the
    // route catalog (`routes/mod.rs`) says so but nothing enforced it.
    if !auth_ctx.role.can_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(error_response(ApiError::forbidden("Admin role required"))),
        );
    }
    let id_parsed = match validate_warning_id(&id) {
        Ok(uuid) => uuid,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    match state.store.delete_warning(id_parsed).await {
        Ok(true) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "warning_deleted",
                    &serde_json::json!({
                        "warning_id": id_parsed,
                        "deleted_by": auth_ctx.user_id,
                    }),
                )
                .await;
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_warning", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({
                        "deleted": true,
                        "warning_id": id
                    }),
                    meta,
                )),
            )
        }
        Ok(false) => {
            let err = ApiError::not_found("warning", &id);
            (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(err)),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete warning failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete warning");
            (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

pub(crate) async fn delete_warnings_bulk(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    // B292: admin-only per the route catalog.
    if !auth_ctx.role.can_admin() {
        return (
            StatusCode::FORBIDDEN,
            Json(error_response(ApiError::forbidden("Admin role required"))),
        );
    }

    let (parsed_ids, requested_count) = match parse_bulk_delete_ids(&body) {
        Ok(result) => result,
        Err(err) => {
            let status =
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY);
            return (status, Json(error_response(err)));
        }
    };

    match state.store.delete_warnings(&parsed_ids).await {
        Ok(count) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "warnings_bulk_deleted",
                    &serde_json::json!({
                        "deleted_count": count,
                        "requested_count": requested_count,
                        "deleted_by": auth_ctx.user_id,
                    }),
                )
                .await;
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_warnings_bulk", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({
                        "deleted_count": count,
                        "requested_count": requested_count
                    }),
                    meta,
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "bulk delete warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete warnings");
            (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

pub(crate) async fn delete_all_warnings(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let authorization = match authorize_delete_all_warnings(&headers, &auth_ctx) {
        Ok(auth) => auth,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::FORBIDDEN),
                Json(error_response(err)),
            );
        }
    };

    match state.store.delete_all_warnings().await {
        Ok(count) => {
            // false-success-classification: best-effort — audit-trail write after the primary mutation succeeded
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "all_warnings_deleted",
                    &delete_all_warnings_audit_payload(&auth_ctx, &authorization, count),
                )
                .await;
            tracing::warn!(request_id = %request_id, user_id = %auth_ctx.user_id, reason = %authorization.reason, "deleted ALL warnings: {} rows", count);
            let duration_ms = start.elapsed().as_millis() as u64;
            let meta = ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms);
            log_latency("delete_all_warnings", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({
                        "deleted_count": count,
                        "message": "All warnings deleted"
                    }),
                    meta,
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "delete all warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to delete all warnings");
            (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            )
        }
    }
}

/// POST /api/warnings/:id/analysis — enqueue a durable, evidence-bound
/// analysis run and return its id immediately (audit P1-5).
///
/// The request never runs the model: it gathers the bounded evidence set,
/// computes its digest, deduplicates identical in-flight/succeeded runs
/// (migration 079) and returns `202 Accepted` with the run id. Poll
/// `GET /api/warnings/:id/analysis/:run_id` for status and result.
#[cfg(feature = "llm")]
pub(crate) async fn enqueue_warning_analysis(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match Uuid::parse_str(&id) {
        Ok(uid) => uid,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request("Invalid UUID"))),
            )
        }
    };

    let warning = match state.store.get_warning(uid).await {
        Ok(Some(warning)) => warning,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Warning", &id))),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "enqueue_warning_analysis: get_warning failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to load warning"))),
            );
        }
    };

    let Some(runtime) = state.llm.as_ref() else {
        return llm_service_unavailable("LLM not configured");
    };
    let model = apex_api::warning_analysis::WarningAnalysisModel {
        primary: runtime.primary.clone(),
        profile: state.intelligence_profile.clone(),
    };
    let requested_by = auth_ctx.user_id.to_string();

    match apex_api::warning_analysis::enqueue_analysis(
        &state.store,
        &model,
        &warning,
        Some(&requested_by),
    )
    .await
    {
        Ok((run, deduplicated, context)) => {
            if run.status == "queued" {
                apex_api::warning_analysis::spawn_analysis_executor(state.store.clone(), context);
            }
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("enqueue_warning_analysis", duration_ms);
            (
                StatusCode::ACCEPTED,
                Json(success_with_meta(
                    serde_json::json!({
                        "analysis_run_id": run.id,
                        "warning_id": run.warning_id,
                        "status": run.status,
                        "deduplicated": deduplicated,
                        "model": run.model,
                        "prompt_version": run.prompt_version,
                        "evidence_digest": run.evidence_digest,
                        "observations_available": run.observations_available,
                        "observations_sent": run.observations_sent,
                        "insights_available": run.insights_available,
                        "insights_sent": run.insights_sent,
                    }),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "enqueue_warning_analysis failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to enqueue warning analysis",
                ))),
            )
        }
    }
}

/// GET /api/warnings/:id/analysis/:run_id — run status and, on success, the
/// persisted claims (with their real evidence ids) plus the rendered output.
#[cfg(feature = "llm")]
pub(crate) async fn get_warning_analysis_run(
    State(state): State<AppState>,
    Extension(_auth_ctx): Extension<ApiAuthContext>,
    Path((id, run_id)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let uid = match Uuid::parse_str(&id) {
        Ok(uid) => uid,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid warning UUID",
                ))),
            )
        }
    };
    let run_uuid = match Uuid::parse_str(&run_id) {
        Ok(run_uuid) => run_uuid,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid analysis run UUID",
                ))),
            )
        }
    };

    // Housekeeping before reporting: a run abandoned by a crashed process must
    // resolve to an explicit failure instead of looking in-progress forever.
    if let Err(error) = state
        .store
        .expire_stale_warning_analysis_runs(apex_api::warning_analysis::STALE_RUN_SECONDS)
        .await
    {
        tracing::warn!(%error, "expire_stale_warning_analysis_runs failed; run status unchanged");
    }

    let run = match state.store.get_warning_analysis_run(run_uuid).await {
        Ok(Some(run)) if run.warning_id == uid => run,
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_response(ApiError::not_found("Analysis run", &run_id))),
            )
        }
        Err(err) => {
            tracing::error!("get_warning_analysis_run failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load analysis run",
                ))),
            );
        }
    };

    let claims = if run.status == "succeeded" {
        match state.store.list_warning_analysis_claims(run_uuid).await {
            Ok(claims) => claims,
            Err(err) => {
                tracing::error!("list_warning_analysis_claims failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to load analysis claims",
                    ))),
                );
            }
        }
    } else {
        Vec::new()
    };

    (
        StatusCode::OK,
        Json(success_with_meta(
            serde_json::json!({
                "analysis_run_id": run.id,
                "warning_id": run.warning_id,
                "status": run.status,
                "model": run.model,
                "prompt_version": run.prompt_version,
                "evidence_digest": run.evidence_digest,
                "observations_available": run.observations_available,
                "observations_sent": run.observations_sent,
                "insights_available": run.insights_available,
                "insights_sent": run.insights_sent,
                "error": run.error,
                "output": run.output,
                "claims": claims,
                "created_at": run.created_at,
                "started_at": run.started_at,
                "finished_at": run.finished_at,
            }),
            ResponseMeta::now().with_request_id(Uuid::new_v4().to_string()),
        )),
    )
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn enqueue_warning_analysis(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn get_warning_analysis_run(
    State(_state): State<AppState>,
    Path((_id, _run_id)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    llm_service_unavailable("LLM feature disabled")
}

#[cfg(test)]
mod tests {
    use super::parse_bulk_delete_ids;
    use uuid::Uuid;

    #[test]
    fn test_parse_bulk_delete_ids_accepts_valid_uuid_list() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let body = format!(r#"{{"ids":["{}","{}"]}}"#, first, second);

        let (ids, requested_count) =
            parse_bulk_delete_ids(body.as_bytes()).expect("valid body should parse");

        assert_eq!(requested_count, 2);
        assert_eq!(ids, vec![first, second]);
    }

    #[test]
    fn test_parse_bulk_delete_ids_rejects_invalid_uuid() {
        let err = parse_bulk_delete_ids(br#"{"ids":["not-a-uuid"]}"#)
            .expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("invalid UUID"));
    }

    #[test]
    fn test_parse_bulk_delete_ids_rejects_more_than_maximum_ids() {
        let ids = (0..1001)
            .map(|_| format!(r#""{}""#, Uuid::new_v4()))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(r#"{{"ids":[{}]}}"#, ids);

        let err = parse_bulk_delete_ids(body.as_bytes()).expect_err("too many ids should fail");

        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("maximum 1000 IDs"));
    }
}
