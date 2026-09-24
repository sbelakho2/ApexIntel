#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListStagingQuery {
    page: Option<u32>,
    per_page: Option<u32>,
}

fn build_recipe_status_payload(status_key: &str, recipe_code: &str) -> serde_json::Value {
    serde_json::json!({
        status_key: true,
        "recipe_code": recipe_code,
    })
}

pub(crate) async fn list_staging_recipes(
    State(state): State<AppState>,
    Query(params): Query<ListStagingQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RecipeStatRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let total_state = DataState::from_result(
        state.store.count_staging_recipes().await,
        "failed to count staging recipes",
        |_| false,
    );
    if total_state.is_degraded() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(degraded_api_error(
                "Failed to count staging recipes",
                &total_state,
            ))),
        );
    }
    let total = total_state.into_loaded_or(0).max(0) as u64;

    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items_state = DataState::from_result(
        state
            .store
            .list_staging_recipes(per_page as i64, offset)
            .await,
        "failed to list staging recipes",
        |rows| rows.is_empty(),
    );
    if items_state.is_degraded() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_response(degraded_api_error(
                "Failed to list staging recipes",
                &items_state,
            ))),
        );
    }
    let items = items_state.into_items();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_staging_recipes", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn promote_recipe(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.promote_recipe(&id).await {
        Ok(true) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "recipe_promoted",
                    &serde_json::json!({"recipe_code": id}),
                )
                .await;
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("promote_recipe", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    build_recipe_status_payload("promoted", &id),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Recipe", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "promote_recipe failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to promote recipe",
                ))),
            )
        }
    }
}

pub(crate) async fn deprecate_recipe(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.deprecate_recipe(&id).await {
        Ok(true) => {
            let _ = state
                .store
                .record_audit_event(
                    &auth_ctx.user_id,
                    "recipe_deprecated",
                    &serde_json::json!({"recipe_code": id}),
                )
                .await;
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("deprecate_recipe", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    build_recipe_status_payload("deprecated", &id),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Recipe", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "deprecate_recipe failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to deprecate recipe",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_recipe_status_payload;

    #[test]
    fn test_build_recipe_status_payload_marks_promoted_recipe() {
        let payload = build_recipe_status_payload("promoted", "recipe_123");

        assert_eq!(payload["promoted"], serde_json::json!(true));
        assert_eq!(payload["recipe_code"], serde_json::json!("recipe_123"));
    }

    #[test]
    fn test_build_recipe_status_payload_marks_deprecated_recipe() {
        let payload = build_recipe_status_payload("deprecated", "recipe_456");

        assert_eq!(payload["deprecated"], serde_json::json!(true));
        assert_eq!(payload["recipe_code"], serde_json::json!("recipe_456"));
    }
}
