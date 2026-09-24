//! Settings handler — GET /settings
//!
//! Covers: application settings page with notification preferences,
//! data retention configuration, theme settings, and API key management.

use std::sync::Arc;

use askama::Template;
use axum::{http::HeaderMap, response::IntoResponse, Extension};
use axum_extra::extract::Form;
use serde::Deserialize;

use super::PageContext;
use crate::middleware::session::WebSession;
use crate::system_status::{format_age, DATA_FRESH_WITHIN_SECS, WORKER_HEARTBEAT_STALE_AFTER_SECS};
use apex_store::postgres::{PgStore, UserSettingsPrefs, WarningListFilters};

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct NotificationSetting {
    pub channel: String, // "email" | "slack" | "webhook"
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

/// Measured health for the settings page — replaces the previously
/// hard-coded "Healthy" status and health-check list.
#[derive(Clone, Debug)]
pub struct SystemHealthView {
    pub api_status: String,
    pub health_checks: Vec<HealthCheckInfo>,
}

impl SystemHealthView {
    pub async fn probe(store: &PgStore) -> Self {
        let database_ok = store.get_database_size().await.is_ok();
        let mut checks = vec![HealthCheckInfo {
            name: "api".into(),
            status: if database_ok {
                "Healthy".into()
            } else {
                "Unhealthy".into()
            },
        }];

        match store.latest_service_heartbeat("worker").await {
            Ok(Some(row)) => {
                let age = chrono::Utc::now().signed_duration_since(row.last_seen_at);
                let stale = age.num_seconds() > WORKER_HEARTBEAT_STALE_AFTER_SECS;
                checks.push(HealthCheckInfo {
                    name: "worker".into(),
                    status: if stale {
                        format!("Degraded · heartbeat {} ago", format_age(age))
                    } else {
                        format!("Healthy · heartbeat {} ago", format_age(age))
                    },
                });
            }
            Ok(None) => checks.push(HealthCheckInfo {
                name: "worker".into(),
                status: "Degraded · no heartbeat recorded".into(),
            }),
            Err(error) => checks.push(HealthCheckInfo {
                name: "worker".into(),
                status: format!("Unavailable · {error}"),
            }),
        }

        match store.newest_observation_ts().await {
            Ok(Some(ts)) => {
                let age = chrono::Utc::now().signed_duration_since(ts);
                let stale = age.num_seconds() > DATA_FRESH_WITHIN_SECS;
                checks.push(HealthCheckInfo {
                    name: "data_freshness".into(),
                    status: if stale {
                        format!("Stale · newest observation {} ago", format_age(age))
                    } else {
                        format!("Fresh · newest observation {} ago", format_age(age))
                    },
                });
            }
            Ok(None) => checks.push(HealthCheckInfo {
                name: "data_freshness".into(),
                status: "Unknown · no observations recorded".into(),
            }),
            Err(error) => checks.push(HealthCheckInfo {
                name: "data_freshness".into(),
                status: format!("Unavailable · {error}"),
            }),
        }

        let check_is_healthy = |name: &str, prefix: &str| {
            checks
                .iter()
                .find(|check| check.name == name)
                .is_some_and(|check| check.status.starts_with(prefix))
        };
        let worker_ok = check_is_healthy("worker", "Healthy");
        let data_ok = check_is_healthy("data_freshness", "Fresh");
        let api_status = if !database_ok {
            "Unhealthy"
        } else if worker_ok && data_ok {
            "Healthy"
        } else {
            "Degraded"
        }
        .to_string();

        Self {
            api_status,
            health_checks: checks,
        }
    }
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/settings.html")]
pub struct SettingsPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

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
    pub email_digest_recipients: String,
    pub email_digest_time_cet: String,
    pub email_digest_weekday: String,
    pub digest_category_demand_signal: bool,
    pub digest_category_supply_risk: bool,
    pub digest_category_competitive_intel: bool,
    pub digest_category_security_posture: bool,
    pub digest_category_macro_shift: bool,
    pub digest_category_poi_movement: bool,
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
    pub email_digest_recipients: Option<String>,
    #[serde(default)]
    pub email_digest_categories: Vec<String>,
    pub email_digest_time_cet: Option<String>,
    pub email_digest_weekday: Option<String>,
}

fn normalize_digest_categories(raw: Option<Vec<String>>) -> Vec<String> {
    const ALLOWED: &[&str] = &[
        "demand_signal",
        "supply_risk",
        "competitive_intel",
        "security_posture",
        "macro_shift",
        "poi_movement",
    ];

    let mut out = Vec::new();
    for v in raw.unwrap_or_default() {
        let normalized = v.trim().to_ascii_lowercase();
        if ALLOWED.contains(&normalized.as_str()) && !out.iter().any(|e: &String| e == &normalized)
        {
            out.push(normalized);
        }
    }
    out
}

fn split_recipients(raw: &str) -> Vec<String> {
    raw.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn is_valid_email_address(email: &str) -> bool {
    let trimmed = email.trim();
    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() || local.is_empty() || domain.is_empty() {
        return false;
    }
    domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn is_valid_hhmm(raw: &str) -> bool {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() != 2 {
        return false;
    }
    let Ok(hour) = parts[0].parse::<u32>() else {
        return false;
    };
    let Ok(minute) = parts[1].parse::<u32>() else {
        return false;
    };
    hour <= 23 && minute <= 59
}

fn is_valid_weekday(raw: &str) -> bool {
    matches!(raw, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")
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
        email_digest_recipients: form.email_digest_recipients.clone().unwrap_or_default(),
        email_digest_categories: normalize_digest_categories(Some(
            form.email_digest_categories.clone(),
        )),
        email_digest_time_cet: form
            .email_digest_time_cet
            .clone()
            .unwrap_or_else(|| "08:00".to_string()),
        email_digest_weekday: form
            .email_digest_weekday
            .clone()
            .unwrap_or_else(|| "Mon".to_string()),
        email_digest_last_sent_at: None,
    }
}

fn render_settings_page(
    ctx: PageContext,
    save_success: Option<String>,
    save_error: Option<String>,
    prefs: &UserSettingsPrefs,
    health: &SystemHealthView,
) -> SettingsPage {
    SettingsPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        notifications: vec![],
        retention_policies: vec![],
        api_keys: vec![],
        data_freshness_interval: 300,
        session_timeout_hours: prefs.session_timeout_hours,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        api_status: health.api_status.clone(),
        endpoints_count: crate::routes::all_endpoints().len() as i64,
        uptime_hours: None,
        api_key_display: prefs.api_key_display.clone(),
        backend_url_display: prefs.backend_url_display.clone(),
        health_checks: health.health_checks.clone(),
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
        email_digest_recipients: prefs.email_digest_recipients.clone(),
        email_digest_time_cet: prefs.email_digest_time_cet.clone(),
        email_digest_weekday: prefs.email_digest_weekday.clone(),
        digest_category_demand_signal: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "demand_signal"),
        digest_category_supply_risk: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "supply_risk"),
        digest_category_competitive_intel: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "competitive_intel"),
        digest_category_security_posture: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "security_posture"),
        digest_category_macro_shift: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "macro_shift"),
        digest_category_poi_movement: prefs
            .email_digest_categories
            .iter()
            .any(|c| c == "poi_movement"),
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
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/settings", unack);

    let prefs = store
        .get_user_settings_prefs_scoped(&session.username, session.role.as_str())
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let health = SystemHealthView::probe(&store).await;

    super::render_template(&render_settings_page(ctx, None, None, &prefs, &health))
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
    let health = SystemHealthView::probe(&store).await;

    let mut prefs = settings_from_form(&form);
    let backend_url = prefs.backend_url_display.clone();
    if backend_url.trim().is_empty() {
        return super::render_template(&render_settings_page(
            ctx,
            None,
            Some("Backend URL cannot be empty.".to_string()),
            &prefs,
            &health,
        ));
    }

    if prefs.email_digest_enabled {
        let recipients = split_recipients(&prefs.email_digest_recipients);
        if recipients.is_empty() {
            return super::render_template(&render_settings_page(
                ctx,
                None,
                Some("Add at least one digest recipient when email digest is enabled.".to_string()),
                &prefs,
                &health,
            ));
        }
        let invalid: Vec<String> = recipients
            .iter()
            .filter(|r| !is_valid_email_address(r))
            .cloned()
            .collect();
        if !invalid.is_empty() {
            return super::render_template(&render_settings_page(
                ctx,
                None,
                Some(format!(
                    "Invalid recipient email(s): {}",
                    invalid.join(", ")
                )),
                &prefs,
                &health,
            ));
        }
        if !is_valid_hhmm(&prefs.email_digest_time_cet) {
            return super::render_template(&render_settings_page(
                ctx,
                None,
                Some("Digest send time must use HH:MM format (CET).".to_string()),
                &prefs,
                &health,
            ));
        }
        if prefs.notification_frequency.eq_ignore_ascii_case("weekly")
            && !is_valid_weekday(&prefs.email_digest_weekday)
        {
            return super::render_template(&render_settings_page(
                ctx,
                None,
                Some("For weekly digest, select a valid weekday.".to_string()),
                &prefs,
                &health,
            ));
        }
    }

    if let Ok(Some(existing)) = store
        .get_user_settings_prefs_scoped(&session.username, session.role.as_str())
        .await
    {
        prefs.email_digest_last_sent_at = existing.email_digest_last_sent_at;
    }

    if let Err(e) = store
        .upsert_user_settings_prefs_scoped(&session.username, session.role.as_str(), &prefs)
        .await
    {
        tracing::error!(
            "Failed to persist user settings for {}: {e}",
            session.username
        );
        return super::render_template(&render_settings_page(
            ctx,
            None,
            Some("Failed to save settings. Please retry.".to_string()),
            &prefs,
            &health,
        ));
    }

    super::render_template(&render_settings_page(
        ctx,
        Some("Settings saved successfully.".to_string()),
        None,
        &prefs,
        &health,
    ))
}
