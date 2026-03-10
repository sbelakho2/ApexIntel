use super::super::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListCompetitorsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct AllCompetitorChangesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    #[serde(default)]
    #[allow(dead_code)]
    all: bool,
}

fn normalize_all_competitor_changes_pagination(
    page: Option<u32>,
    per_page: Option<u32>,
) -> (u32, u32) {
    (
        page.unwrap_or(1).max(1),
        per_page.unwrap_or(50).clamp(1, 100),
    )
}

fn parse_competitor_uuid(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::bad_request("Invalid UUID"))
}

pub(crate) async fn list_competitors(
    State(state): State<AppState>,
    Query(params): Query<ListCompetitorsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CompanyRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let total = state.store.count_competitors().await.unwrap_or(0).max(0) as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = state
        .store
        .list_competitors(per_page as i64, offset)
        .await
        .unwrap_or_default();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_competitors", duration_ms);
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

pub(crate) async fn get_competitor_changes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<CompanyChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_competitor_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_competitor_changes(uid, 100).await {
        Ok(changes) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_competitor_changes", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    changes,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "competitor changes failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load competitor changes",
                ))),
            )
        }
    }
}

pub(crate) async fn list_all_competitor_changes(
    State(state): State<AppState>,
    Query(query): Query<AllCompetitorChangesQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<CompetitorChange>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page) = normalize_all_competitor_changes_pagination(query.page, query.per_page);

    match state
        .store
        .get_all_competitor_changes_paged(page as i64, per_page as i64)
        .await
    {
        Ok((items, total)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_all_competitor_changes", duration_ms);
            let payload = PagedResponse {
                items,
                total: total as u64,
                page,
                per_page,
            };
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
        Err(err) => {
            tracing::error!(request_id = %request_id, "list all competitor changes failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list competitor changes",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_all_competitor_changes_pagination, parse_competitor_uuid};

    #[test]
    fn test_normalize_all_competitor_changes_pagination_defaults() {
        assert_eq!(
            normalize_all_competitor_changes_pagination(None, None),
            (1, 50)
        );
    }

    #[test]
    fn test_normalize_all_competitor_changes_pagination_clamps_values() {
        assert_eq!(
            normalize_all_competitor_changes_pagination(Some(0), Some(500)),
            (1, 100)
        );
    }

    #[test]
    fn test_parse_competitor_uuid_rejects_invalid_uuid() {
        let err = parse_competitor_uuid("bad-id").expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 400);
        assert_eq!(err.message, "Invalid UUID");
    }
}
