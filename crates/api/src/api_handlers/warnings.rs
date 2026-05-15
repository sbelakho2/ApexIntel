#![allow(clippy::disallowed_methods)]

use crate::*;
use apex_store::postgres::{AcknowledgeWarningResult, WarningReviewOutcome};

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
    let date_to = match parse_date_end(&params.date_to) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
    };
    if let Err(msg) = validate_date_range(&date_from, &date_to) {
        let api_err = ApiError::bad_request(msg);
        return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
    }

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(value) => value,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_region_codes(&regions) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let severities = match parse_csv_lower_strict(&params.severities, 32, "severities") {
        Ok(value) => value,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
    if let Err(api_err) = validate_severity_codes(&severities) {
        return (
            StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(error_response(api_err)),
        );
    }
    let warning_types = match parse_csv_lower_strict(&params.warning_types, 32, "warning_types") {
        Ok(value) => value,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };
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
        Err(response) => return response,
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
                Err(response) => return response,
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

async fn fetch_warning_page(
    state: &AppState,
    filters: &WarningListFilters,
    order_by: WarningOrderBy,
    desc: bool,
    per_page: u32,
    page: u32,
    request_id: &str,
) -> Result<
    Vec<WarningRow>,
    (
        StatusCode,
        Json<ApiResponse<PagedResponse<WarningResponse>>>,
    ),
> {
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
            Err((
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            ))
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
    Json<ApiResponse<routes::warnings::AcknowledgeResponse>>,
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
                        "has_note": body.note.as_ref().map(|note| !note.trim().is_empty()).unwrap_or(false)
                    }),
                )
                .await;
            let now = Utc::now();
            let resp = routes::warnings::AcknowledgeResponse {
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
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
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

    match state.store.delete_warning(id_parsed).await {
        Ok(true) => {
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
    body: axum::body::Bytes,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

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
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    match state.store.delete_all_warnings().await {
        Ok(count) => {
            tracing::warn!(request_id = %request_id, "deleted ALL warnings: {} rows", count);
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
