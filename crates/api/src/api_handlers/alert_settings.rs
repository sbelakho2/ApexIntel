//! API handlers for alert threshold configuration.
//!
//! Endpoints:
//! - `GET    /api/settings/alerts`              — list all entity configs + global defaults
//! - `GET    /api/settings/alerts/entity/:id`   — get entity config
//! - `PUT    /api/settings/alerts/entity/:id`   — upsert entity config
//! - `DELETE /api/settings/alerts/entity/:id`   — delete entity config
//! - `PUT    /api/settings/alerts/global`       — update global defaults

use crate::*;

use apex_core::alert_config::{AlertChannel, EntityAlertConfig, GlobalAlertDefaults};

/// Upper bound for `cooldown_minutes` (7 days): a longer cooldown effectively
/// disables alerting for the entity, which must not be configurable silently.
const MAX_COOLDOWN_MINUTES: u32 = 10_080;
/// Upper bound for `max_daily_alerts` / `max_daily`. `0` keeps its documented
/// "unlimited" meaning; anything above this is a typo, not a policy.
const MAX_DAILY_ALERTS: u32 = 1_000;

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

/// Log the store failure server-side and return a generic client error:
/// SQLx errors can carry constraint names, table names and connection details.
fn alert_settings_store_error(context: &'static str, error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "alert settings store error: {context}");
    ApiError::internal(context)
}

// ─── Authorization / validation helpers ──────────────────────────────────────

/// #146: alert settings are org-wide. The admin sub-router already applies
/// [`apex_api::middleware::session::require_admin`]; this is the in-handler
/// defense so the handler itself can never be mounted without the check.
fn ensure_alert_settings_admin(role: &apex_api::auth::ApiRole) -> Result<(), ApiError> {
    if !role.can_admin() {
        return Err(ApiError::forbidden(
            "Admin role required to change alert settings",
        ));
    }
    Ok(())
}

/// In-app delivery is the floor of alerting: a configuration that removes it
/// would silently turn alerts into e-mails/webhooks only (or nothing at all).
fn validate_channel_selection(field: &str, channels: &[AlertChannel]) -> Result<(), ApiError> {
    if !channels.contains(&AlertChannel::InApp) {
        return Err(ApiError::validation(
            field,
            "in-app delivery must remain enabled",
        ));
    }
    Ok(())
}

fn validate_cooldown(field: &str, minutes: u32) -> Result<(), ApiError> {
    if minutes > MAX_COOLDOWN_MINUTES {
        return Err(ApiError::validation(
            field,
            format!("cooldown must be between 0 and {MAX_COOLDOWN_MINUTES} minutes"),
        ));
    }
    Ok(())
}

fn validate_daily_cap(field: &str, max_daily: u32) -> Result<(), ApiError> {
    if max_daily > MAX_DAILY_ALERTS {
        return Err(ApiError::validation(
            field,
            format!("daily cap must be between 0 (unlimited) and {MAX_DAILY_ALERTS}"),
        ));
    }
    Ok(())
}

fn validate_entity_alert_config(config: &EntityAlertConfig) -> Result<(), ApiError> {
    validate_channel_selection("config.enabled_channels", &config.enabled_channels)?;
    validate_cooldown("config.cooldown_minutes", config.cooldown_minutes)?;
    validate_daily_cap("config.max_daily_alerts", config.max_daily_alerts)?;

    for (index, rule) in config.override_rules.iter().enumerate() {
        if rule.alert_type.trim().is_empty() {
            return Err(ApiError::validation(
                &format!("config.override_rules[{index}].alert_type"),
                "alert_type must not be empty",
            ));
        }
        validate_cooldown(
            &format!("config.override_rules[{index}].cooldown_minutes"),
            rule.cooldown_minutes,
        )?;
        validate_daily_cap(
            &format!("config.override_rules[{index}].max_daily"),
            rule.max_daily,
        )?;
    }
    Ok(())
}

fn validate_global_alert_defaults(config: &GlobalAlertDefaults) -> Result<(), ApiError> {
    validate_channel_selection("config.enabled_channels", &config.enabled_channels)?;
    validate_cooldown("config.cooldown_minutes", config.cooldown_minutes)?;
    validate_daily_cap("config.max_daily_alerts", config.max_daily_alerts)
}

/// Record the change in the audit trail. Best-effort: the mutation already
/// committed, so a failed audit insert is logged (an operator can see the
/// difference) rather than reported as a failed change.
async fn audit_alert_settings_change(
    state: &AppState,
    actor: &str,
    event_type: &str,
    detail: serde_json::Value,
) {
    if let Err(error) = state
        .store
        .record_audit_event(actor, event_type, &detail)
        .await
    {
        tracing::error!(error = %error, event_type, "failed to write alert settings audit event");
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// GET /api/settings/alerts
///
/// Returns all entity alert configs together with the global defaults.
pub(crate) async fn list_alert_settings(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AlertSettingsListResponse>>, ApiError> {
    let entities = state
        .store
        .list_entity_alert_configs()
        .await
        .map_err(|e| alert_settings_store_error("Failed to list entity alert configs", e))?;

    let global_defaults = state
        .store
        .get_global_alert_defaults()
        .await
        .map_err(|e| alert_settings_store_error("Failed to get global defaults", e))?;

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
        .map_err(|e| alert_settings_store_error("Failed to get alert config", e))?;

    match config {
        Some(cfg) => Ok(Json(success(AlertConfigResponse { config: cfg }))),
        None => Err(ApiError::not_found("entity alert config", &entity_id)),
    }
}

/// PUT /api/settings/alerts/entity/:entity_id
pub(crate) async fn upsert_entity_alert_config(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
    Json(body): Json<UpsertEntityConfigRequest>,
) -> Result<Json<ApiResponse<AlertConfigResponse>>, ApiError> {
    ensure_alert_settings_admin(&auth_ctx.role)?;

    // Ensure the entity_id in the path matches the config
    if body.config.entity_id != entity_id {
        return Err(ApiError::validation(
            "entity_id",
            "Path entity_id must match config.entity_id",
        ));
    }
    validate_entity_alert_config(&body.config)?;

    state
        .store
        .upsert_entity_alert_config(&entity_id, &body.config)
        .await
        .map_err(|e| alert_settings_store_error("Failed to save alert config", e))?;

    audit_alert_settings_change(
        &state,
        &auth_ctx.user_id,
        "alert_settings_entity_updated",
        serde_json::json!({
            "entity_id": entity_id,
            "min_severity": body.config.min_severity.as_str(),
            "enabled_channels": body.config.enabled_channels.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "cooldown_minutes": body.config.cooldown_minutes,
            "max_daily_alerts": body.config.max_daily_alerts,
            "enabled": body.config.enabled,
        }),
    )
    .await;

    Ok(Json(success(AlertConfigResponse {
        config: body.config,
    })))
}

/// DELETE /api/settings/alerts/entity/:entity_id
pub(crate) async fn delete_entity_alert_config(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Path(entity_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_alert_settings_admin(&auth_ctx.role)?;

    let deleted = state
        .store
        .delete_entity_alert_config(&entity_id)
        .await
        .map_err(|e| alert_settings_store_error("Failed to delete alert config", e))?;

    if !deleted {
        return Err(ApiError::not_found("entity alert config", &entity_id));
    }

    audit_alert_settings_change(
        &state,
        &auth_ctx.user_id,
        "alert_settings_entity_deleted",
        serde_json::json!({ "entity_id": entity_id }),
    )
    .await;

    Ok(Json(success(())))
}

/// PUT /api/settings/alerts/global
pub(crate) async fn upsert_global_alert_defaults(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpsertGlobalDefaultsRequest>,
) -> Result<Json<ApiResponse<GlobalDefaultsResponse>>, ApiError> {
    ensure_alert_settings_admin(&auth_ctx.role)?;
    validate_global_alert_defaults(&body.config)?;

    state
        .store
        .upsert_global_alert_defaults(&body.config)
        .await
        .map_err(|e| alert_settings_store_error("Failed to save global defaults", e))?;

    audit_alert_settings_change(
        &state,
        &auth_ctx.user_id,
        "alert_settings_global_updated",
        serde_json::json!({
            "min_severity": body.config.min_severity.as_str(),
            "enabled_channels": body.config.enabled_channels.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "cooldown_minutes": body.config.cooldown_minutes,
            "max_daily_alerts": body.config.max_daily_alerts,
        }),
    )
    .await;

    Ok(Json(success(GlobalDefaultsResponse {
        config: body.config,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_api::auth::ApiRole;
    use apex_core::alert_config::{AlertOverride, AlertSeverity};

    // ── #146: role gate ──────────────────────────────────────────────────────

    #[test]
    fn analysts_and_readers_cannot_change_alert_settings() {
        for role in [ApiRole::Analyst, ApiRole::Viewer, ApiRole::Service] {
            let err = ensure_alert_settings_admin(&role).expect_err("role must be denied");
            assert_eq!(err.http_status(), 403, "{role:?} must be forbidden");
        }
    }

    #[test]
    fn admins_can_change_alert_settings() {
        assert!(ensure_alert_settings_admin(&ApiRole::Admin).is_ok());
    }

    // ── #146: range + channel validation ────────────────────────────────────

    #[test]
    fn entity_config_rejects_cooldown_over_max() {
        let config = EntityAlertConfig {
            entity_id: "c-1".into(),
            cooldown_minutes: MAX_COOLDOWN_MINUTES + 1,
            ..Default::default()
        };
        let err = validate_entity_alert_config(&config).expect_err("cooldown too large");
        assert_eq!(
            err.details
                .as_ref()
                .and_then(|d| d.get("field"))
                .map(String::as_str),
            Some("config.cooldown_minutes")
        );
    }

    #[test]
    fn entity_config_rejects_daily_cap_over_max() {
        let config = EntityAlertConfig {
            entity_id: "c-1".into(),
            max_daily_alerts: MAX_DAILY_ALERTS + 1,
            ..Default::default()
        };
        let err = validate_entity_alert_config(&config).expect_err("daily cap too large");
        assert_eq!(
            err.details
                .as_ref()
                .and_then(|d| d.get("field"))
                .map(String::as_str),
            Some("config.max_daily_alerts")
        );
    }

    #[test]
    fn entity_config_accepts_documented_zero_and_boundaries() {
        let config = EntityAlertConfig {
            entity_id: "c-1".into(),
            cooldown_minutes: MAX_COOLDOWN_MINUTES,
            max_daily_alerts: 0,
            ..Default::default()
        };
        assert!(validate_entity_alert_config(&config).is_ok());
    }

    #[test]
    fn entity_config_refuses_config_that_removes_in_app_delivery() {
        let config = EntityAlertConfig {
            entity_id: "c-1".into(),
            enabled_channels: vec![AlertChannel::Email, AlertChannel::Slack],
            ..Default::default()
        };
        let err = validate_entity_alert_config(&config).expect_err("in-app must stay enabled");
        assert_eq!(
            err.details
                .as_ref()
                .and_then(|d| d.get("field"))
                .map(String::as_str),
            Some("config.enabled_channels")
        );
    }

    #[test]
    fn entity_config_refuses_empty_and_unknown_override_rules() {
        let empty_type = EntityAlertConfig {
            entity_id: "c-1".into(),
            override_rules: vec![AlertOverride {
                alert_type: "  ".into(),
                min_severity: AlertSeverity::Low,
                enabled: true,
                cooldown_minutes: 5,
                max_daily: 5,
            }],
            ..Default::default()
        };
        assert!(validate_entity_alert_config(&empty_type).is_err());

        let bad_override = EntityAlertConfig {
            entity_id: "c-1".into(),
            override_rules: vec![AlertOverride {
                alert_type: "warning".into(),
                min_severity: AlertSeverity::Low,
                enabled: true,
                cooldown_minutes: MAX_COOLDOWN_MINUTES + 1,
                max_daily: 5,
            }],
            ..Default::default()
        };
        let err = validate_entity_alert_config(&bad_override).expect_err("override cooldown");
        assert_eq!(
            err.details
                .as_ref()
                .and_then(|d| d.get("field"))
                .map(String::as_str),
            Some("config.override_rules[0].cooldown_minutes")
        );
    }

    #[test]
    fn global_defaults_refuse_in_app_removal() {
        let config = GlobalAlertDefaults {
            enabled_channels: vec![AlertChannel::Webhook],
            ..Default::default()
        };
        let err = validate_global_alert_defaults(&config).expect_err("in-app must stay enabled");
        assert_eq!(
            err.details
                .as_ref()
                .and_then(|d| d.get("field"))
                .map(String::as_str),
            Some("config.enabled_channels")
        );
    }

    #[test]
    fn global_defaults_refuse_out_of_range_values() {
        let config = GlobalAlertDefaults {
            cooldown_minutes: MAX_COOLDOWN_MINUTES + 1,
            ..Default::default()
        };
        assert!(validate_global_alert_defaults(&config).is_err());

        let config = GlobalAlertDefaults {
            max_daily_alerts: MAX_DAILY_ALERTS + 1,
            ..Default::default()
        };
        assert!(validate_global_alert_defaults(&config).is_err());
    }

    // ── existing store-error contract ───────────────────────────────────────

    #[test]
    fn store_errors_do_not_leak_internal_details() {
        let err = alert_settings_store_error(
            "Failed to list entity alert configs",
            "error returned from database: relation \"alert_secret_table\" does not exist",
        );

        assert_eq!(err.message, "Failed to list entity alert configs");
        assert!(!err.message.contains("alert_secret_table"));
        assert_eq!(err.http_status(), 500);
    }
}
