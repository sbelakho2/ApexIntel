use super::super::*;
use apex_api::routes::collaboration::{
    sanitized_email, sanitized_notes, validate_annotation_request, validate_saved_search_request,
    validate_user_request, validate_watchlist_request, AnalystUser, Annotation, AnnotationQuery,
    AuditLogEntry, AuditLogQuery, ExportHistoryEntry, SavedSearch, UpsertAnalystUserRequest,
    UpsertAnnotationRequest, UpsertSavedSearchRequest, UpsertWatchlistRequest, Watchlist,
};

fn analyst_user_from_record(record: apex_store::postgres::AnalystUserRecord) -> AnalystUser {
    AnalystUser {
        id: record.id,
        display_name: record.display_name,
        email: record.email,
        role: record.role,
        notification_channels: record.notification_channels,
        is_active: record.is_active,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn saved_search_from_record(record: apex_store::postgres::SavedSearchRecord) -> SavedSearch {
    SavedSearch {
        id: record.id.to_string(),
        user_id: record.user_id,
        name: record.name,
        query_text: record.query_text,
        filters: record.filters,
        default_sort: record.default_sort,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn watchlist_from_record(record: apex_store::postgres::WatchlistRecord) -> Watchlist {
    Watchlist {
        id: record.id.to_string(),
        user_id: record.user_id,
        name: record.name,
        entities: record.entities,
        notes: record.notes,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn annotation_from_record(record: apex_store::postgres::AnnotationRecord) -> Annotation {
    Annotation {
        id: record.id.to_string(),
        user_id: record.user_id,
        entity_type: record.entity_type,
        entity_id: record.entity_id,
        body: record.body,
        tags: record.tags,
        visibility: record.visibility,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn export_history_from_record(
    record: apex_store::postgres::ExportHistoryRecord,
) -> ExportHistoryEntry {
    ExportHistoryEntry {
        id: record.id.to_string(),
        user_id: record.user_id,
        export_type: record.export_type,
        format: record.format,
        filters: record.filters,
        row_count: record.row_count,
        download_name: record.download_name,
        requested_at: record.requested_at,
    }
}

fn audit_log_from_record(record: apex_store::postgres::AuditLogRecord) -> AuditLogEntry {
    AuditLogEntry {
        id: record.id.to_string(),
        event_type: record.event_type,
        actor: record.actor,
        detail: record.detail,
        created_at: record.created_at,
    }
}

pub(crate) async fn list_users(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<AnalystUser>>>) {
    match state.store.list_analyst_users().await {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records.into_iter().map(analyst_user_from_record).collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_users failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list analyst users",
                ))),
            )
        }
    }
}

pub(crate) async fn list_audit_log(
    State(state): State<AppState>,
    Query(params): Query<AuditLogQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<AuditLogEntry>>>) {
    let limit = params.limit.unwrap_or(100).clamp(1, 200) as i64;

    match state
        .store
        .list_audit_log(params.actor.as_deref(), params.event_type.as_deref(), limit)
        .await
    {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records.into_iter().map(audit_log_from_record).collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_audit_log failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list audit log",
                ))),
            )
        }
    }
}

pub(crate) async fn create_user(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpsertAnalystUserRequest>,
) -> (StatusCode, Json<ApiResponse<AnalystUser>>) {
    if let Err(message) = validate_user_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation(
                "display_name",
                message,
            ))),
        );
    }

    let user_id = format!(
        "usr-{}",
        body.display_name
            .to_ascii_lowercase()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
    );
    let email = sanitized_email(body.email.clone());
    let result = state
        .store
        .upsert_analyst_user(
            &user_id,
            body.display_name.trim(),
            email.as_deref(),
            body.role.trim(),
            &body.notification_channels,
            body.is_active.unwrap_or(true),
        )
        .await;

    match result {
        Ok(record) => {
            let detail = serde_json::json!({"user_id": user_id, "role": record.role});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "analyst_user_upserted", &detail)
                .await;
            (
                StatusCode::CREATED,
                Json(success(analyst_user_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("create_user failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to upsert analyst user",
                ))),
            )
        }
    }
}

pub(crate) async fn update_user(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(body): Json<UpsertAnalystUserRequest>,
) -> (StatusCode, Json<ApiResponse<AnalystUser>>) {
    if let Err(message) = validate_user_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation(
                "display_name",
                message,
            ))),
        );
    }

    match state
        .store
        .upsert_analyst_user(
            &id,
            body.display_name.trim(),
            sanitized_email(body.email.clone()).as_deref(),
            body.role.trim(),
            &body.notification_channels,
            body.is_active.unwrap_or(true),
        )
        .await
    {
        Ok(record) => {
            let detail = serde_json::json!({"user_id": id, "role": record.role});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "analyst_user_updated", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(analyst_user_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("update_user failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update analyst user",
                ))),
            )
        }
    }
}

pub(crate) async fn list_saved_searches(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<Vec<SavedSearch>>>) {
    match state.store.list_saved_searches(&auth_ctx.user_id).await {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records.into_iter().map(saved_search_from_record).collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_saved_searches failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list saved searches",
                ))),
            )
        }
    }
}

pub(crate) async fn create_saved_search(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpsertSavedSearchRequest>,
) -> (StatusCode, Json<ApiResponse<SavedSearch>>) {
    if let Err(message) = validate_saved_search_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("query_text", message))),
        );
    }

    match state
        .store
        .upsert_saved_search(
            None,
            &auth_ctx.user_id,
            body.name.trim(),
            body.query_text.trim(),
            &body.filters,
            body.default_sort.as_deref(),
        )
        .await
    {
        Ok(record) => {
            let detail = serde_json::json!({"saved_search_id": record.id, "name": record.name});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "saved_search_created", &detail)
                .await;
            (
                StatusCode::CREATED,
                Json(success(saved_search_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("create_saved_search failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create saved search",
                ))),
            )
        }
    }
}

pub(crate) async fn update_saved_search(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(body): Json<UpsertSavedSearchRequest>,
) -> (StatusCode, Json<ApiResponse<SavedSearch>>) {
    if let Err(message) = validate_saved_search_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("query_text", message))),
        );
    }
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid saved search id",
                ))),
            )
        }
    };

    match state
        .store
        .upsert_saved_search(
            Some(id),
            &auth_ctx.user_id,
            body.name.trim(),
            body.query_text.trim(),
            &body.filters,
            body.default_sort.as_deref(),
        )
        .await
    {
        Ok(record) => {
            let detail = serde_json::json!({"saved_search_id": record.id, "name": record.name});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "saved_search_updated", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(saved_search_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("update_saved_search failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update saved search",
                ))),
            )
        }
    }
}

pub(crate) async fn delete_saved_search(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid saved search id",
                ))),
            )
        }
    };
    match state.store.delete_saved_search(&auth_ctx.user_id, id).await {
        Ok(true) => {
            let detail = serde_json::json!({"saved_search_id": id});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "saved_search_deleted", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(serde_json::json!({"deleted": true}))),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "saved search",
                &id.to_string(),
            ))),
        ),
        Err(err) => {
            tracing::error!("delete_saved_search failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to delete saved search",
                ))),
            )
        }
    }
}

pub(crate) async fn list_watchlists(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<Vec<Watchlist>>>) {
    match state.store.list_watchlists(&auth_ctx.user_id).await {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records.into_iter().map(watchlist_from_record).collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_watchlists failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list watchlists",
                ))),
            )
        }
    }
}

pub(crate) async fn create_watchlist(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpsertWatchlistRequest>,
) -> (StatusCode, Json<ApiResponse<Watchlist>>) {
    if let Err(message) = validate_watchlist_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("entities", message))),
        );
    }
    match state
        .store
        .upsert_watchlist(
            None,
            &auth_ctx.user_id,
            body.name.trim(),
            &body.entities,
            sanitized_notes(body.notes.clone()).as_deref(),
        )
        .await
    {
        Ok(record) => {
            let detail = serde_json::json!({"watchlist_id": record.id, "name": record.name});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "watchlist_created", &detail)
                .await;
            (
                StatusCode::CREATED,
                Json(success(watchlist_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("create_watchlist failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create watchlist",
                ))),
            )
        }
    }
}

pub(crate) async fn update_watchlist(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(body): Json<UpsertWatchlistRequest>,
) -> (StatusCode, Json<ApiResponse<Watchlist>>) {
    if let Err(message) = validate_watchlist_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("entities", message))),
        );
    }
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid watchlist id",
                ))),
            )
        }
    };
    match state
        .store
        .upsert_watchlist(
            Some(id),
            &auth_ctx.user_id,
            body.name.trim(),
            &body.entities,
            sanitized_notes(body.notes.clone()).as_deref(),
        )
        .await
    {
        Ok(record) => {
            let detail = serde_json::json!({"watchlist_id": record.id, "name": record.name});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "watchlist_updated", &detail)
                .await;
            (StatusCode::OK, Json(success(watchlist_from_record(record))))
        }
        Err(err) => {
            tracing::error!("update_watchlist failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update watchlist",
                ))),
            )
        }
    }
}

pub(crate) async fn delete_watchlist(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid watchlist id",
                ))),
            )
        }
    };
    match state.store.delete_watchlist(&auth_ctx.user_id, id).await {
        Ok(true) => {
            let detail = serde_json::json!({"watchlist_id": id});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "watchlist_deleted", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(serde_json::json!({"deleted": true}))),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "watchlist",
                &id.to_string(),
            ))),
        ),
        Err(err) => {
            tracing::error!("delete_watchlist failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to delete watchlist",
                ))),
            )
        }
    }
}

pub(crate) async fn list_annotations(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Query(query): Query<AnnotationQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<Annotation>>>) {
    match state
        .store
        .list_annotations(
            &auth_ctx.user_id,
            query.entity_type.as_deref(),
            query.entity_id.as_deref(),
        )
        .await
    {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records.into_iter().map(annotation_from_record).collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_annotations failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list annotations",
                ))),
            )
        }
    }
}

pub(crate) async fn create_annotation(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpsertAnnotationRequest>,
) -> (StatusCode, Json<ApiResponse<Annotation>>) {
    if let Err(message) = validate_annotation_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("body", message))),
        );
    }
    let visibility = body
        .visibility
        .clone()
        .unwrap_or_else(|| "private".to_string());
    match state
        .store
        .upsert_annotation(
            None,
            &auth_ctx.user_id,
            body.entity_type.trim(),
            body.entity_id.trim(),
            body.body.trim(),
            &body.tags,
            &visibility,
        )
        .await
    {
        Ok(record) => {
            let detail =
                serde_json::json!({"annotation_id": record.id, "entity_id": record.entity_id});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "annotation_created", &detail)
                .await;
            (
                StatusCode::CREATED,
                Json(success(annotation_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("create_annotation failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to create annotation",
                ))),
            )
        }
    }
}

pub(crate) async fn update_annotation(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
    Json(body): Json<UpsertAnnotationRequest>,
) -> (StatusCode, Json<ApiResponse<Annotation>>) {
    if let Err(message) = validate_annotation_request(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(error_response(ApiError::validation("body", message))),
        );
    }
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid annotation id",
                ))),
            )
        }
    };
    let visibility = body
        .visibility
        .clone()
        .unwrap_or_else(|| "private".to_string());
    match state
        .store
        .upsert_annotation(
            Some(id),
            &auth_ctx.user_id,
            body.entity_type.trim(),
            body.entity_id.trim(),
            body.body.trim(),
            &body.tags,
            &visibility,
        )
        .await
    {
        Ok(record) => {
            let detail =
                serde_json::json!({"annotation_id": record.id, "entity_id": record.entity_id});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "annotation_updated", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(annotation_from_record(record))),
            )
        }
        Err(err) => {
            tracing::error!("update_annotation failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to update annotation",
                ))),
            )
        }
    }
}

pub(crate) async fn delete_annotation(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(ApiError::bad_request(
                    "Invalid annotation id",
                ))),
            )
        }
    };
    match state.store.delete_annotation(&auth_ctx.user_id, id).await {
        Ok(true) => {
            let detail = serde_json::json!({"annotation_id": id});
            let _ = state
                .store
                .record_audit_event(&auth_ctx.user_id, "annotation_deleted", &detail)
                .await;
            (
                StatusCode::OK,
                Json(success(serde_json::json!({"deleted": true}))),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(error_response(ApiError::not_found(
                "annotation",
                &id.to_string(),
            ))),
        ),
        Err(err) => {
            tracing::error!("delete_annotation failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to delete annotation",
                ))),
            )
        }
    }
}

pub(crate) async fn list_export_history(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<Vec<ExportHistoryEntry>>>) {
    match state.store.list_export_history(&auth_ctx.user_id).await {
        Ok(records) => (
            StatusCode::OK,
            Json(success(
                records
                    .into_iter()
                    .map(export_history_from_record)
                    .collect(),
            )),
        ),
        Err(err) => {
            tracing::error!("list_export_history failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list export history",
                ))),
            )
        }
    }
}
