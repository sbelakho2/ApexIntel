use super::super::*;

fn decode_user_preferences(
    user_id: &str,
    record: Option<apex_store::postgres::UserPreferencesRecord>,
) -> (UserPreferences, serde_json::Value) {
    let mut prefs = UserPreferences {
        user_id: user_id.to_string(),
        ..UserPreferences::default()
    };
    let mut raw = serde_json::json!({});

    if let Some(record) = record {
        prefs.theme = record.theme;
        prefs.locale = record.locale;
        prefs.updated_at = record.updated_at;
        raw = if record.preferences.is_object() {
            record.preferences
        } else {
            serde_json::json!({})
        };

        if let Some(value) = raw.get("default_region") {
            prefs.default_region = value.as_str().map(|s| s.to_string());
        }
        if let Some(value) = raw.get("dashboard_layout").and_then(|v| v.as_str()) {
            prefs.dashboard_layout = value.to_string();
        }
        if let Some(value) = raw.get("notifications") {
            if let Ok(parsed) = serde_json::from_value::<NotificationPrefs>(value.clone()) {
                prefs.notifications = parsed;
            }
        }
        if let Some(value) = raw.get("table_columns") {
            if let Ok(parsed) =
                serde_json::from_value::<HashMap<String, Vec<String>>>(value.clone())
            {
                prefs.table_columns = parsed;
            }
        }
        if let Some(value) = raw.get("custom") {
            if let Ok(parsed) =
                serde_json::from_value::<HashMap<String, serde_json::Value>>(value.clone())
            {
                prefs.custom = parsed;
            }
        }
    }

    (prefs, raw)
}

pub(crate) async fn get_preferences(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> (StatusCode, Json<ApiResponse<PreferencesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let user_id = auth_ctx.user_id;
    let role = auth_ctx.role.as_str();

    let record = match state
        .store
        .get_user_preferences_record_scoped(&user_id, role)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get preferences failed: {err:#}");
            let api_err = ApiError::internal("Failed to fetch preferences");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let (preferences, _) = decode_user_preferences(&user_id, record);
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(PreferencesResponse { preferences }, meta)),
    )
}

pub(crate) async fn update_preferences(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
    Json(body): Json<UpdatePreferencesRequest>,
) -> (StatusCode, Json<ApiResponse<PreferencesResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let user_id = auth_ctx.user_id;
    let role = auth_ctx.role.as_str();

    let existing = match state
        .store
        .get_user_preferences_record_scoped(&user_id, role)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(request_id = %request_id, "fetch existing preferences failed: {err:#}");
            let api_err = ApiError::internal("Failed to load existing preferences");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let (mut preferences, mut raw) = decode_user_preferences(&user_id, existing);
    preferences.apply_update(body);

    if !raw.is_object() {
        raw = serde_json::json!({});
    }
    if let Some(obj) = raw.as_object_mut() {
        obj.insert(
            "default_region".to_string(),
            serde_json::to_value(&preferences.default_region).unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "dashboard_layout".to_string(),
            serde_json::to_value(&preferences.dashboard_layout).unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "notifications".to_string(),
            serde_json::to_value(&preferences.notifications).unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "table_columns".to_string(),
            serde_json::to_value(&preferences.table_columns).unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "custom".to_string(),
            serde_json::to_value(&preferences.custom).unwrap_or(serde_json::Value::Null),
        );
    }

    if let Err(err) = state
        .store
        .upsert_user_preferences_record_scoped(
            &user_id,
            role,
            &preferences.theme,
            &preferences.locale,
            &raw,
        )
        .await
    {
        tracing::error!(request_id = %request_id, "upsert preferences failed: {err:#}");
        let api_err = ApiError::internal("Failed to save preferences");
        return (
            StatusCode::from_u16(api_err.http_status())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(error_response(api_err)),
        );
    }

    let _ = state
        .store
        .record_audit_event(
            &user_id,
            "preferences_updated",
            &serde_json::json!({
                "theme": preferences.theme,
                "locale": preferences.locale,
                "default_region": preferences.default_region,
                "dashboard_layout": preferences.dashboard_layout
            }),
        )
        .await;

    preferences.updated_at = Utc::now();
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(PreferencesResponse { preferences }, meta)),
    )
}

#[cfg(test)]
mod tests {
    use super::decode_user_preferences;
    use apex_store::postgres::UserPreferencesRecord;
    use chrono::Utc;

    #[test]
    fn test_decode_user_preferences_defaults_when_record_is_missing() {
        let (prefs, raw) = decode_user_preferences("user-123", None);

        assert_eq!(prefs.user_id, "user-123");
        assert_eq!(prefs.theme, "system");
        assert_eq!(prefs.locale, "en");
        assert_eq!(raw, serde_json::json!({}));
    }

    #[test]
    fn test_decode_user_preferences_restores_nested_preference_fields() {
        let updated_at = Utc::now();
        let record = UserPreferencesRecord {
            theme: "dark".to_string(),
            locale: "fr".to_string(),
            preferences: serde_json::json!({
                "default_region": "EMEA",
                "dashboard_layout": "dense",
                "notifications": {
                    "email_enabled": false,
                    "slack_enabled": true,
                    "browser_push": false,
                    "min_severity": "high",
                    "quiet_start_hour": 22,
                    "quiet_end_hour": 6
                },
                "table_columns": {
                    "warnings": ["severity", "title"]
                },
                "custom": {
                    "beta": true
                }
            }),
            updated_at,
        };

        let (prefs, raw) = decode_user_preferences("user-456", Some(record));

        assert_eq!(prefs.user_id, "user-456");
        assert_eq!(prefs.theme, "dark");
        assert_eq!(prefs.locale, "fr");
        assert_eq!(prefs.default_region.as_deref(), Some("EMEA"));
        assert_eq!(prefs.dashboard_layout, "dense");
        assert!(!prefs.notifications.email_enabled);
        assert!(prefs.notifications.slack_enabled);
        assert_eq!(
            prefs.table_columns.get("warnings").cloned(),
            Some(vec!["severity".to_string(), "title".to_string()])
        );
        assert_eq!(prefs.custom.get("beta"), Some(&serde_json::json!(true)));
        assert_eq!(prefs.updated_at, updated_at);
        assert!(raw.is_object());
    }
}
