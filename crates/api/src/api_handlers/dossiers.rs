use crate::*;
use apex_api::routes::dossiers::validate_dossier_id;

fn parse_dossier_uuid(id: &str) -> Result<Uuid, ApiError> {
    validate_dossier_id(id).map_err(ApiError::bad_request)
}

pub(crate) async fn get_company_dossier(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CompanyDossier>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_company_dossier(uid).await {
        Ok(Some(dossier)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_company_dossier", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    dossier,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Company", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "company dossier failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load company dossier",
                ))),
            )
        }
    }
}

pub(crate) async fn get_person_dossier(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonDossier>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_person_dossier(uid).await {
        Ok(Some(dossier)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_dossier", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    dossier,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Person", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "person dossier failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load person dossier",
                ))),
            )
        }
    }
}

pub(crate) async fn get_person_engagement(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonEngagement>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_person_engagement(uid).await {
        Ok(Some(engagement)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_engagement", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    engagement,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("Person", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "person engagement failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load engagement",
                ))),
            )
        }
    }
}

pub(crate) async fn get_person_role_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<RoleHistoryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_role_history(uid, 100).await {
        Ok(history) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_role_history", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    history,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "role history failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load role history",
                ))),
            )
        }
    }
}

pub(crate) async fn get_person_changes_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<PersonChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_person_changes(uid, 100).await {
        Ok(changes) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_changes", duration_ms);
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
            tracing::error!(request_id = %request_id, "person changes failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load person changes",
                ))),
            )
        }
    }
}

pub(crate) async fn get_person_dossier_entries(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state
        .store
        .get_dossier_entries("person", uid, None, 200)
        .await
    {
        Ok(entries) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_person_dossier_entries", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    entries,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "person dossier entries failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load dossier entries",
                ))),
            )
        }
    }
}

pub(crate) async fn get_company_changes_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<CompanyChangeRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_company_changes(uid, 100).await {
        Ok(changes) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_company_changes", duration_ms);
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
            tracing::error!(request_id = %request_id, "company changes failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load company changes",
                ))),
            )
        }
    }
}

pub(crate) async fn get_company_dossier_entries(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state
        .store
        .get_dossier_entries("company", uid, None, 200)
        .await
    {
        Ok(entries) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_company_dossier_entries", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    entries,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "company dossier entries failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load dossier entries",
                ))),
            )
        }
    }
}

pub(crate) async fn verify_dossier_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.verify_dossier_entry(uid).await {
        Ok(true) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("verify_dossier_entry", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    serde_json::json!({"verified": true}),
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found("DossierEntry", &id))),
        ),
        Err(err) => {
            tracing::error!(request_id = %request_id, "verify dossier entry failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to verify dossier entry",
                ))),
            )
        }
    }
}

pub(crate) async fn get_dossier_entry_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DossierEntryRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_dossier_uuid(&id) {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };

    match state.store.get_dossier_entry_history(uid).await {
        Ok(entries) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_dossier_entry_history", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    entries,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "dossier entry history failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load dossier entry history",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_dossier_uuid;
    use uuid::Uuid;

    #[test]
    fn test_parse_dossier_uuid_accepts_valid_uuid() {
        let id = Uuid::new_v4().to_string();

        let parsed = parse_dossier_uuid(&id).expect("valid uuid should parse");

        assert_eq!(parsed.to_string(), id);
    }

    #[test]
    fn test_parse_dossier_uuid_rejects_invalid_uuid() {
        let err = parse_dossier_uuid("bad-id").expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 400);
        assert!(err.message.contains("dossier"));
    }
}
