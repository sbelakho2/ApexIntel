//! API handlers for alert threshold configuration.
//!
//! Endpoints:
//! - `GET    /api/settings/alerts`              — list all entity configs + global defaults
//! - `GET    /api/settings/alerts/entity/:id`   — get entity config
//! - `PUT    /api/settings/alerts/entity/:id`   — upsert entity config
//! - `DELETE /api/settings/alerts/entity/:id`   — delete entity config
//! - `PUT    /api/settings/alerts/global`       — update global defaults

use crate::*;

use apex_core::alert_config::{EntityAlertConfig, GlobalAlertDefaults};

// ─── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct AlertSettingsListResponse {
    pub entities: Vec<EntityAlertConfig>,
    pub global_defaults: Option<GlobalAlertDefaults>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AlertConfigResponse {
    pub config: EntityAlertConfig,
}

#[derive(Debug, Serialize)]
pub(crate) struct GlobalDefaultsResponse {
    pub config: GlobalAlertDefaults,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpsertEntityConfigRequest {
    pub config: EntityAlertConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpsertGlobalDefaultsRequest {
    pub config: GlobalAlertDefaults,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// GET /api/settings/alerts
///
/// Returns all entity alert configs together with the global defaults.
pub(crate) async fn list_alert_settings(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AlertSettingsListResponse>>, ApiError> {
    let entities =
        state.store.list_entity_alert_configs().await.map_err(|e| {
            ApiError::internal(format!("Failed to list entity alert configs: {}", e))
        })?;

    let global_defaults = state
        .store
        .get_global_alert_defaults()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get global defaults: {}", e)))?;

    Ok(Json(success(AlertSettingsListResponse {
        entities,
        global_defaults,
    })))
}

/// GET /api/settings/alerts/entity/:entity_id
pub(crate) async fn get_entity_alert_config(
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
) -> Result<Json<ApiResponse<AlertConfigResponse>>, ApiError> {
    let config = state
        .store
        .get_entity_alert_config(&entity_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get alert config: {}", e)))?;

    match config {
        Some(cfg) => Ok(Json(success(AlertConfigResponse { config: cfg }))),
        None => Err(ApiError::not_found("entity alert config", &entity_id)),
    }
}

/// PUT /api/settings/alerts/entity/:entity_id
pub(crate) async fn upsert_entity_alert_config(
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
    Json(body): Json<UpsertEntityConfigRequest>,
) -> Result<Json<ApiResponse<AlertConfigResponse>>, ApiError> {
    // Ensure the entity_id in the path matches the config
    if body.config.entity_id != entity_id {
        return Err(ApiError::validation(
            "entity_id",
            "Path entity_id must match config.entity_id",
        ));
    }

    state
        .store
        .upsert_entity_alert_config(&entity_id, &body.config)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to save alert config: {}", e)))?;

    Ok(Json(success(AlertConfigResponse {
        config: body.config,
    })))
}

/// DELETE /api/settings/alerts/entity/:entity_id
pub(crate) async fn delete_entity_alert_config(
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let deleted = state
        .store
        .delete_entity_alert_config(&entity_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete alert config: {}", e)))?;

    if !deleted {
        return Err(ApiError::not_found("entity alert config", &entity_id));
    }

    Ok(Json(success(())))
}

/// PUT /api/settings/alerts/global
pub(crate) async fn upsert_global_alert_defaults(
    State(state): State<AppState>,
    Json(body): Json<UpsertGlobalDefaultsRequest>,
) -> Result<Json<ApiResponse<GlobalDefaultsResponse>>, ApiError> {
    state
        .store
        .upsert_global_alert_defaults(&body.config)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to save global defaults: {}", e)))?;

    Ok(Json(success(GlobalDefaultsResponse {
        config: body.config,
    })))
}
