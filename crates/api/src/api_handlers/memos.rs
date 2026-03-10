use super::super::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListMemosQuery {
    page: Option<u64>,
    per_page: Option<u64>,
}

fn normalize_memo_pagination(page: Option<u64>, per_page: Option<u64>) -> (i64, i64) {
    let per_page = per_page.unwrap_or(20).min(100) as i64;
    let page = page.unwrap_or(1).max(1) as i64;
    (page, per_page)
}

pub(crate) async fn get_weekly_memo(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<WeeklyMemo>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    match state.store.get_weekly_memo_full().await {
        Ok(Some(memo)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_weekly_memo", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    memo,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("WeeklyMemo", "latest"))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "weekly memo failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to generate weekly memo",
                ))),
            )
        }
    }
}

pub(crate) async fn list_memos(
    State(state): State<AppState>,
    Query(params): Query<ListMemosQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<WeeklyMemo>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page) = normalize_memo_pagination(params.page, params.per_page);
    let offset = (page - 1) * per_page;

    match state.store.list_weekly_memos(per_page, offset).await {
        Ok((memos, total)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("list_memos", duration_ms);
            let payload = PagedResponse {
                items: memos,
                total: total as u64,
                page: page as u32,
                per_page: per_page as u32,
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
            tracing::error!(request_id = %request_id, "list memos failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to list memos"))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_memo_pagination;

    #[test]
    fn test_normalize_memo_pagination_defaults() {
        assert_eq!(normalize_memo_pagination(None, None), (1, 20));
    }

    #[test]
    fn test_normalize_memo_pagination_clamps_values() {
        assert_eq!(normalize_memo_pagination(Some(0), Some(500)), (1, 100));
    }
}
