use super::super::*;

fn parse_graph_uuid(id: &str, field_name: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::bad_request(format!("Invalid {} UUID", field_name)))
}

pub(crate) async fn get_graph_neighborhood(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<EdgeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_graph_uuid(&id, "entity") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_neighborhood(uid, 100).await {
        Ok(edges) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_graph_neighborhood", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    edges,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph neighborhood failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load graph neighborhood",
                ))),
            )
        }
    }
}

pub(crate) async fn get_graph_path(
    State(state): State<AppState>,
    Path((from, to)): Path<(String, String)>,
) -> (StatusCode, Json<ApiResponse<Vec<EdgeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let from_id = match parse_graph_uuid(&from, "from") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let to_id = match parse_graph_uuid(&to, "to") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_path_edges(from_id, to_id).await {
        Ok(edges) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_graph_path", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    edges,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph path failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load graph path",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_graph_uuid;
    use uuid::Uuid;

    #[test]
    fn test_parse_graph_uuid_accepts_valid_uuid() {
        let id = Uuid::new_v4().to_string();

        let parsed = parse_graph_uuid(&id, "entity").expect("valid uuid should parse");

        assert_eq!(parsed.to_string(), id);
    }

    #[test]
    fn test_parse_graph_uuid_uses_field_name_in_error() {
        let err = parse_graph_uuid("bad-id", "from").expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 400);
        assert_eq!(err.message, "Invalid from UUID");
    }
}
