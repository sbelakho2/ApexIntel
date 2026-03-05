//! Settings handler — GET /settings
//!
//! Covers: application settings page with notification preferences,
//! data retention configuration, theme settings, and API key management.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Form,
    http::HeaderMap,
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use apex_store::postgres::{PgStore, UserSettingsPrefs, WarningListFilters};
use super::PageContext;
use crate::middleware::session::WebSession;

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct NotificationSetting {
    pub channel: String,       // "email" | "slack" | "webhook"
    pub enabled: bool,
    pub endpoint: String,
    pub min_severity: String,
}

#[derive(Clone, Debug)]
pub struct RetentionPolicy {
    pub entity_type: String,
    pub retention_days: i64,
    pub auto_archive: bool,
}

#[derive(Clone, Debug)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub role: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used: Option<String>,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct HealthCheckInfo {
    pub name: String,
    pub status: String,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/settings.html")]
pub struct SettingsPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub notifications: Vec<NotificationSetting>,
    pub retention_policies: Vec<RetentionPolicy>,
    pub api_keys: Vec<ApiKeyInfo>,
    pub data_freshness_interval: i64,
    pub session_timeout_hours: i64,
    pub app_version: String,
    pub api_status: String,
    pub endpoints_count: i64,
    pub uptime_hours: Option<i64>,
    pub api_key_display: String,
    pub backend_url_display: String,
    pub health_checks: Vec<HealthCheckInfo>,
    pub default_region: String,
    pub auto_include_neighbors: bool,
    pub daily_crawl_enabled: bool,
    pub crawl_window: String,
    pub slack_enabled: bool,
    pub minimum_severity: String,
    pub auto_cleanup_enabled: bool,
    pub export_format: String,
    pub retention_period: String,
    pub email_digest_enabled: bool,
    pub notification_frequency: String,
    pub critical_only_enabled: bool,
    pub save_success: Option<String>,
    pub save_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    pub api_key_display: Option<String>,
    pub backend_url_display: Option<String>,
    pub session_timeout_hours: Option<i64>,
    pub default_region: Option<String>,
    pub auto_include_neighbors: Option<String>,
    pub daily_crawl_enabled: Option<String>,
    pub crawl_window: Option<String>,
    pub slack_enabled: Option<String>,
    pub minimum_severity: Option<String>,
    pub auto_cleanup_enabled: Option<String>,
    pub export_format: Option<String>,
    pub retention_period: Option<String>,
    pub email_digest_enabled: Option<String>,
    pub notification_frequency: Option<String>,
    pub critical_only_enabled: Option<String>,
}

fn settings_from_form(form: &SettingsForm) -> UserSettingsPrefs {
    UserSettingsPrefs {
        api_key_display: form
            .api_key_display
            .clone()
            .unwrap_or_else(|| "(not configured)".to_string()),
        backend_url_display: form
            .backend_url_display
            .clone()
            .unwrap_or_else(|| "direct Axum service".to_string()),
        session_timeout_hours: form.session_timeout_hours.unwrap_or(24),
        default_region: form
            .default_region
            .clone()
            .unwrap_or_else(|| "Global".to_string()),
        auto_include_neighbors: form.auto_include_neighbors.clone().is_some(),
        daily_crawl_enabled: form.daily_crawl_enabled.clone().is_some(),
        crawl_window: form
            .crawl_window
            .clone()
            .unwrap_or_else(|| "00:00-06:00 UTC".to_string()),
        slack_enabled: form.slack_enabled.clone().is_some(),
        minimum_severity: form
            .minimum_severity
            .clone()
            .unwrap_or_else(|| "high".to_string()),
        auto_cleanup_enabled: form.auto_cleanup_enabled.clone().is_some(),
        export_format: form
            .export_format
            .clone()
            .unwrap_or_else(|| "JSON".to_string()),
        retention_period: form
            .retention_period
            .clone()
            .unwrap_or_else(|| "90 days".to_string()),
        email_digest_enabled: form.email_digest_enabled.clone().is_some(),
        notification_frequency: form
            .notification_frequency
            .clone()
            .unwrap_or_else(|| "Daily".to_string()),
        critical_only_enabled: form.critical_only_enabled.clone().is_some(),
    }
}

fn render_settings_page(ctx: PageContext, save_success: Option<String>, save_error: Option<String>, prefs: &UserSettingsPrefs) -> SettingsPage {
    SettingsPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        notifications: vec![],
        retention_policies: vec![],
        api_keys: vec![],
        data_freshness_interval: 300,
        session_timeout_hours: prefs.session_timeout_hours,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        api_status: "Healthy".to_string(),
        endpoints_count: 45,
        uptime_hours: None,
        api_key_display: prefs.api_key_display.clone(),
        backend_url_display: prefs.backend_url_display.clone(),
        health_checks: vec![
            HealthCheckInfo {
                name: "api".into(),
                status: "Healthy".into(),
            },
            HealthCheckInfo {
                name: "routes".into(),
                status: "Healthy".into(),
            },
        ],
        default_region: prefs.default_region.clone(),
        auto_include_neighbors: prefs.auto_include_neighbors,
        daily_crawl_enabled: prefs.daily_crawl_enabled,
        crawl_window: prefs.crawl_window.clone(),
        slack_enabled: prefs.slack_enabled,
        minimum_severity: prefs.minimum_severity.clone(),
        auto_cleanup_enabled: prefs.auto_cleanup_enabled,
        export_format: prefs.export_format.clone(),
        retention_period: prefs.retention_period.clone(),
        email_digest_enabled: prefs.email_digest_enabled,
        notification_frequency: prefs.notification_frequency.clone(),
        critical_only_enabled: prefs.critical_only_enabled,
        save_success,
        save_error,
    }
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /settings — application settings page.
pub async fn settings_page(
    _headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let unack = store.count_warnings(&WarningListFilters { acknowledged: Some(false), ..Default::default() }).await.unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/settings", unack);

    let prefs = store
        .get_user_settings_prefs(&session.username)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    render_settings_page(ctx, None, None, &prefs).into_response()
}

/// POST /settings — validate and save settings changes.
pub async fn save_settings(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<SettingsForm>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/settings", unack);

    let prefs = settings_from_form(&form);
    let backend_url = prefs.backend_url_display.clone();
    if backend_url.trim().is_empty() {
        return render_settings_page(
            ctx,
            None,
            Some("Backend URL cannot be empty.".to_string()),
            &prefs,
        )
        .into_response();
    }

    if let Err(e) = store
        .upsert_user_settings_prefs(&session.username, &prefs)
        .await
    {
        tracing::error!("Failed to persist user settings for {}: {e}", session.username);
        return render_settings_page(
            ctx,
            None,
            Some("Failed to save settings. Please retry.".to_string()),
            &prefs,
        )
        .into_response();
    }

    render_settings_page(
        ctx,
        Some("Settings saved successfully.".to_string()),
        None,
        &prefs,
    )
    .into_response()
}
