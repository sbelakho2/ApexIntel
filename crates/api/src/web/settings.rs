//! Settings handler — GET /settings, POST /settings
//!
//! The page is split by audience (P0 audit #18):
//! - **Personal Preferences** (every signed-in user): theme, locale, table
//!   layout, default region, session length, and notification/digest
//!   preferences. Persisted through the single `user_preferences` contract in
//!   `apex_store::postgres::preferences` (`theme` + `locale` columns,
//!   `preferences` JSONB `settings_page` object).
//! - **System Configuration** (admin-only, read-only): the real environment
//!   values owned by the API and worker processes. Nothing here is editable
//!   because each service reads these variables at startup; an editable
//!   control would promise a reload that never happens.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Extension,
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use axum_extra::extract::Form;
use serde::Deserialize;

use super::PageContext;
use crate::middleware::session::{
    appearance_cookie_headers, WebSession, MAX_SESSION_HOURS, MIN_SESSION_HOURS,
};
use crate::system_status::{format_age, DATA_FRESH_WITHIN_SECS, WORKER_HEARTBEAT_STALE_AFTER_SECS};
use apex_store::postgres::{PgStore, UserPreferencesRecord, UserSettingsPrefs, WarningListFilters};

// ─── Personal preference vocabulary ─────────────────────────────────────────

pub const SUPPORTED_THEMES: [&str; 3] = ["light", "dark", "system"];
pub const SUPPORTED_LOCALES: [&str; 13] = [
    "en", "fr", "ar", "he", "zh", "ja", "ko", "de", "es", "pt", "it", "ru", "tr",
];
pub const TABLE_LAYOUTS: [&str; 2] = ["comfortable", "compact"];
pub const REGIONS: [&str; 3] = ["Global", "EU", "MENA"];
pub const MIN_SEVERITIES: [&str; 2] = ["high", "medium"];
pub const DIGEST_FREQUENCIES: [&str; 2] = ["Daily", "Weekly"];

fn normalize_choice(raw: Option<&str>, allowed: &[&str], fallback: &str) -> String {
    match raw.map(str::trim) {
        Some(value) if allowed.contains(&value) => value.to_string(),
        _ => fallback.to_string(),
    }
}

fn normalize_session_hours(raw: Option<i64>, fallback: i64) -> i64 {
    match raw {
        Some(hours) => hours.clamp(MIN_SESSION_HOURS, MAX_SESSION_HOURS),
        None => fallback.clamp(MIN_SESSION_HOURS, MAX_SESSION_HOURS),
    }
}

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct HealthCheckInfo {
    pub name: String,
    pub status: String,
    /// Semantic colour class derived from the measured status (Askama cannot
    /// call `str::starts_with` in a template expression).
    pub status_class: String,
}

fn health_status_class(status: &str) -> String {
    if status.starts_with("Healthy") || status.starts_with("Fresh") {
        "apex-text-positive".to_string()
    } else if status.starts_with("Degraded") || status.starts_with("Stale") {
        "apex-text-warning".to_string()
    } else {
        "apex-text-danger".to_string()
    }
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
        let mut checks = vec![(
            "api".to_string(),
            if database_ok {
                "Healthy".to_string()
            } else {
                "Unhealthy".to_string()
            },
        )];

        match store.latest_service_heartbeat("worker").await {
            Ok(Some(row)) => {
                let age = chrono::Utc::now().signed_duration_since(row.last_seen_at);
                let stale = age.num_seconds() > WORKER_HEARTBEAT_STALE_AFTER_SECS;
                checks.push((
                    "worker".to_string(),
                    if stale {
                        format!("Degraded · heartbeat {} ago", format_age(age))
                    } else {
                        format!("Healthy · heartbeat {} ago", format_age(age))
                    },
                ));
            }
            Ok(None) => checks.push((
                "worker".to_string(),
                "Degraded · no heartbeat recorded".to_string(),
            )),
            Err(error) => checks.push(("worker".to_string(), format!("Unavailable · {error}"))),
        }

        match store.newest_observation_ts().await {
            Ok(Some(ts)) => {
                let age = chrono::Utc::now().signed_duration_since(ts);
                let stale = age.num_seconds() > DATA_FRESH_WITHIN_SECS;
                checks.push((
                    "data_freshness".to_string(),
                    if stale {
                        format!("Stale · newest observation {} ago", format_age(age))
                    } else {
                        format!("Fresh · newest observation {} ago", format_age(age))
                    },
                ));
            }
            Ok(None) => checks.push((
                "data_freshness".to_string(),
                "Unknown · no observations recorded".to_string(),
            )),
            Err(error) => checks.push((
                "data_freshness".to_string(),
                format!("Unavailable · {error}"),
            )),
        }

        let check_is_healthy = |name: &str, prefix: &str| {
            checks
                .iter()
                .find(|(check_name, _)| check_name == name)
                .is_some_and(|(_, status)| status.starts_with(prefix))
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
            health_checks: checks
                .into_iter()
                .map(|(name, status)| HealthCheckInfo {
                    status_class: health_status_class(&status),
                    name,
                    status,
                })
                .collect(),
        }
    }
}

// ─── System configuration (read-only environment readout) ───────────────────

/// One read-only configuration value with the environment key and where the
/// rendered value comes from ("environment", "default", "unset").
#[derive(Clone, Debug)]
pub struct SystemConfigItem {
    pub label: String,
    pub env_key: String,
    pub value: String,
    pub source: String,
    /// True for credential-bearing keys: only never/configured is rendered.
    pub sensitive: bool,
}

#[derive(Clone, Debug)]
pub struct SystemConfigSection {
    pub title: String,
    pub description: String,
    pub items: Vec<SystemConfigItem>,
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn value_item(label: &str, key: &str, default: Option<&str>) -> SystemConfigItem {
    match env_value(key) {
        Some(value) => SystemConfigItem {
            label: label.to_string(),
            env_key: key.to_string(),
            value,
            source: "environment".to_string(),
            sensitive: false,
        },
        None => match default {
            Some(default_value) => SystemConfigItem {
                label: label.to_string(),
                env_key: key.to_string(),
                value: default_value.to_string(),
                source: "default".to_string(),
                sensitive: false,
            },
            None => SystemConfigItem {
                label: label.to_string(),
                env_key: key.to_string(),
                value: "not configured".to_string(),
                source: "unset".to_string(),
                sensitive: false,
            },
        },
    }
}

fn secret_item(label: &str, key: &str) -> SystemConfigItem {
    let configured = env_value(key).is_some();
    SystemConfigItem {
        label: label.to_string(),
        env_key: key.to_string(),
        value: if configured {
            "configured".to_string()
        } else {
            "not configured".to_string()
        },
        source: if configured {
            "environment".to_string()
        } else {
            "unset".to_string()
        },
        sensitive: true,
    }
}

fn informational_item(label: &str, env_key: &str, value: &str, source: &str) -> SystemConfigItem {
    SystemConfigItem {
        label: label.to_string(),
        env_key: env_key.to_string(),
        value: value.to_string(),
        source: source.to_string(),
        sensitive: false,
    }
}

fn backend_bind_item() -> SystemConfigItem {
    let host = env_value("HOST").unwrap_or_else(|| "0.0.0.0".to_string());
    let port = env_value("PORT").unwrap_or_else(|| "8080".to_string());
    let from_env = env_value("HOST").is_some() || env_value("PORT").is_some();
    informational_item(
        "API bind address",
        "HOST / PORT",
        &format!("{host}:{port}"),
        if from_env { "environment" } else { "default" },
    )
}

/// The read-only system configuration sections rendered on `/settings` for
/// admins. Every value is read from the live process environment, so what the
/// page shows is exactly what the services started with.
pub fn system_config_sections() -> Vec<SystemConfigSection> {
    let section =
        |title: &str, description: &str, items: Vec<SystemConfigItem>| SystemConfigSection {
            title: title.to_string(),
            description: description.to_string(),
            items,
        };

    vec![
        section(
            "Service",
            "How the API process is bound and what it advertises.",
            vec![
                backend_bind_item(),
                value_item("CORS origin", "CORS_ORIGIN", Some("http://localhost:3000")),
                value_item(
                    "Email link base URL",
                    "EMAIL_DIGEST_BASE_URL",
                    None,
                ),
            ],
        ),
        section(
            "Crawl scheduling",
            "Read by the worker at startup; change the environment and restart the worker to apply.",
            vec![
                value_item(
                    "Crawl interval (seconds)",
                    "CRAWL_INTERVAL_SECS",
                    Some("21600"),
                ),
                value_item("Nightly pipeline hour (UTC)", "NIGHTLY_HOUR_UTC", Some("2")),
                value_item("Weekly pipeline day (0=Mon)", "WEEKLY_DAY", Some("0")),
                value_item(
                    "Requests/second per domain",
                    "DEFAULT_RPS",
                    Some("0.2"),
                ),
                value_item("Proxy pool size", "PROXY_POOL_SIZE", Some("50")),
                value_item("Proxy rotation", "ENABLE_PROXY_ROTATION", Some("false")),
                value_item(
                    "Headless browser fallback",
                    "ENABLE_HEADLESS_BROWSER",
                    Some("false"),
                ),
            ],
        ),
        section(
            "Source budgets & job limits",
            "Crawl and job throttles enforced by the worker and crawler.",
            vec![
                value_item("Max sources per crawl", "CRAWL_MAX_SOURCES", None),
                value_item("Min crawl success ratio", "CRAWL_MIN_SUCCESS_RATIO", None),
                value_item(
                    "Manual trigger concurrency",
                    "MANUAL_TRIGGER_CONCURRENCY",
                    None,
                ),
                value_item(
                    "Manual trigger timeout (seconds)",
                    "MANUAL_TRIGGER_TIMEOUT_SECS",
                    None,
                ),
                value_item("Job timeout (seconds)", "JOB_TIMEOUT_SECS", None),
                value_item(
                    "Worker max concurrent jobs",
                    "WORKER_MAX_CONCURRENT_JOBS",
                    None,
                ),
            ],
        ),
        section(
            "LLM",
            "Model endpoint and token/timeout budgets used by insight and enrichment workflows.",
            vec![
                value_item("LLM base URL", "LLM_BASE_URL", None),
                value_item(
                    "LLM model",
                    "LLM_MODEL",
                    Some("Qwen3-30B-A3B-Q4_K_M"),
                ),
                secret_item("LLM API key", "LLM_API_KEY"),
                value_item("Provider override", "API_LLM_PROVIDER", None),
                value_item("Allowed models", "API_LLM_ALLOWED_MODELS", None),
                value_item(
                    "Primary max tokens",
                    "API_LLM_PRIMARY_MAX_TOKENS",
                    Some("4096"),
                ),
                value_item(
                    "Primary timeout (seconds)",
                    "API_LLM_PRIMARY_TIMEOUT_SECS",
                    Some("300"),
                ),
            ],
        ),
        section(
            "SMTP & email delivery",
            "Mail transports for digests and alert notifications. Credentials are never rendered.",
            vec![
                secret_item("SMTP URL", "SMTP_URL"),
                value_item("Digest SMTP host", "EMAIL_DIGEST_SMTP_HOST", None),
                value_item("Digest SMTP port", "EMAIL_DIGEST_SMTP_PORT", None),
                value_item(
                    "Digest SMTP STARTTLS",
                    "EMAIL_DIGEST_SMTP_STARTTLS",
                    None,
                ),
                value_item("Alert SMTP host", "ALERT_SMTP_HOST", None),
                value_item("Alert sender address", "ALERT_FROM_ADDRESS", None),
                value_item("Alert recipients", "ALERT_EMAIL_RECIPIENTS", None),
            ],
        ),
        section(
            "Retention",
            "There is no runtime retention control. Nothing prunes observations automatically, so no editable setting exists here.",
            vec![informational_item(
                "Observation retention",
                "—",
                "no scheduled pruning configured",
                "not runtime-configurable",
            )],
        ),
        section(
            "Alert policy",
            "Delivery policy for real-time warnings and SLA reminders.",
            vec![
                value_item("Realtime alerts", "REALTIME_ALERTS_ENABLED", None),
                secret_item("Critical webhook", "CRITICAL_WEBHOOK_URL"),
                secret_item("Slack webhook", "SLACK_WEBHOOK_URL"),
                secret_item("Generic webhooks", "GENERIC_WEBHOOK_URLS"),
                value_item(
                    "SLA reminder lead (seconds)",
                    "SLA_REMINDER_AHEAD_SECONDS",
                    None,
                ),
                value_item("Alert rules path", "ALERT_RULES_PATH", None),
            ],
        ),
        section(
            "Integrations",
            "Third-party data providers. Absent keys disable the matching collector.",
            vec![
                secret_item("Google Custom Search", "GOOGLE_API_KEY"),
                secret_item("GitHub", "GITHUB_TOKEN"),
                secret_item("Have I Been Pwned", "HIBP_API_KEY"),
                secret_item("Clearbit", "CLEARBIT_API_KEY"),
                secret_item("Apollo", "APOLLO_API_KEY"),
                secret_item("FRED", "FRED_API_KEY"),
                secret_item("Mouser", "MOUSER_API_KEY"),
                value_item("Nexar client ID", "NEXAR_CLIENT_ID", None),
                value_item("Digi-Key client ID", "DIGIKEY_CLIENT_ID", None),
                value_item("StarzCRM bridge", "STARZCRM_ENABLED", Some("false")),
            ],
        ),
    ]
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/settings.html")]
pub struct SettingsPage {
    // ── base layout fields ──
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    /// Stored theme (`light` | `dark` | `system`) rendered server-side so the
    /// saved preference applies on first paint.
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    // ── personal preferences ──
    pub stored_locale: String,
    pub table_layout: String,
    pub session_timeout_hours: i64,
    pub default_region: String,
    pub minimum_severity: String,
    pub critical_only_enabled: bool,
    pub email_digest_enabled: bool,
    pub notification_frequency: String,
    pub email_digest_recipients: String,
    pub email_digest_time_cet: String,
    pub email_digest_weekday: String,
    pub digest_category_demand_signal: bool,
    pub digest_category_supply_risk: bool,
    pub digest_category_competitive_intel: bool,
    pub digest_category_security_posture: bool,
    pub digest_category_macro_shift: bool,
    pub digest_category_poi_movement: bool,

    // ── measured health ──
    pub api_status: String,
    pub health_checks: Vec<HealthCheckInfo>,
    pub app_version: String,

    // ── system configuration (rendered only for admins) ──
    pub system_config_sections: Vec<SystemConfigSection>,

    // ── feedback ──
    pub save_success: Option<String>,
    pub save_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    pub theme: Option<String>,
    pub locale: Option<String>,
    pub table_layout: Option<String>,
    pub session_timeout_hours: Option<i64>,
    pub default_region: Option<String>,
    pub minimum_severity: Option<String>,
    pub critical_only_enabled: Option<String>,
    pub email_digest_enabled: Option<String>,
    pub notification_frequency: Option<String>,
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

/// Build the persisted personal preferences from the submitted form. Only
/// personal fields exist here by design (P0 #18).
fn settings_from_form(form: &SettingsForm, previous: &UserSettingsPrefs) -> UserSettingsPrefs {
    UserSettingsPrefs {
        session_timeout_hours: normalize_session_hours(
            form.session_timeout_hours,
            previous.session_timeout_hours,
        ),
        default_region: normalize_choice(
            form.default_region.as_deref(),
            &REGIONS,
            &previous.default_region,
        ),
        minimum_severity: normalize_choice(
            form.minimum_severity.as_deref(),
            &MIN_SEVERITIES,
            &previous.minimum_severity,
        ),
        critical_only_enabled: form.critical_only_enabled.clone().is_some(),
        email_digest_enabled: form.email_digest_enabled.clone().is_some(),
        notification_frequency: normalize_choice(
            form.notification_frequency.as_deref(),
            &DIGEST_FREQUENCIES,
            &previous.notification_frequency,
        ),
        email_digest_recipients: form
            .email_digest_recipients
            .clone()
            .unwrap_or_else(|| previous.email_digest_recipients.clone()),
        email_digest_categories: normalize_digest_categories(Some(
            form.email_digest_categories.clone(),
        )),
        email_digest_time_cet: form
            .email_digest_time_cet
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| previous.email_digest_time_cet.clone()),
        email_digest_weekday: normalize_choice(
            form.email_digest_weekday.as_deref(),
            &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            &previous.email_digest_weekday,
        ),
        email_digest_last_sent_at: previous.email_digest_last_sent_at,
        table_layout: normalize_choice(
            form.table_layout.as_deref(),
            &TABLE_LAYOUTS,
            &previous.table_layout,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_settings_page(
    ctx: PageContext,
    record: Option<&UserPreferencesRecord>,
    prefs: &UserSettingsPrefs,
    health: &SystemHealthView,
    save_success: Option<String>,
    save_error: Option<String>,
) -> SettingsPage {
    let theme = record
        .map(|record| record.theme.as_str())
        .filter(|theme| SUPPORTED_THEMES.contains(theme))
        .unwrap_or("system")
        .to_string();
    let locale = record
        .map(|record| record.locale.as_str())
        .filter(|locale| SUPPORTED_LOCALES.contains(locale))
        .unwrap_or("en")
        .to_string();

    SettingsPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme,
        status_strip: ctx.status_strip,
        stored_locale: locale,
        table_layout: prefs.table_layout.clone(),
        session_timeout_hours: normalize_session_hours(
            Some(prefs.session_timeout_hours),
            prefs.session_timeout_hours,
        ),
        default_region: prefs.default_region.clone(),
        minimum_severity: prefs.minimum_severity.clone(),
        critical_only_enabled: prefs.critical_only_enabled,
        email_digest_enabled: prefs.email_digest_enabled,
        notification_frequency: prefs.notification_frequency.clone(),
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
        api_status: health.api_status.clone(),
        health_checks: health.health_checks.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        system_config_sections: system_config_sections(),
        save_success,
        save_error,
    }
}

fn append_appearance_cookies(response: &mut Response, theme: &str, table_layout: &str) {
    for cookie in appearance_cookie_headers(theme, table_layout) {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
}

// ─── Handler ────────────────────────────────────────────────────────────────

async fn unacknowledged_warnings(store: &PgStore) -> i64 {
    store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0)
}

/// GET /settings — personal preferences (all users) + read-only system
/// configuration (admins only).
pub async fn settings_page(
    _headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let unack = unacknowledged_warnings(&store).await;
    let ctx = PageContext::from_session(&session, "/settings", unack);

    let record = store
        .get_user_preferences_record(&session.user_id)
        .await
        .ok()
        .flatten();
    let prefs = store
        .get_user_settings_prefs_scoped(&session.user_id, session.role.as_str())
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let health = SystemHealthView::probe(&store).await;

    let page = render_settings_page(ctx, record.as_ref(), &prefs, &health, None, None);
    let theme = page.theme.clone();
    let table_layout = page.table_layout.clone();
    let mut response = super::render_template(&page);
    append_appearance_cookies(&mut response, &theme, &table_layout);
    response
}

/// POST /settings — validate and save personal preferences only.
pub async fn save_settings(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<SettingsForm>,
) -> impl IntoResponse {
    let unack = unacknowledged_warnings(&store).await;
    let ctx = PageContext::from_session(&session, "/settings", unack);
    let health = SystemHealthView::probe(&store).await;

    let existing_record = store
        .get_user_preferences_record(&session.user_id)
        .await
        .ok()
        .flatten();
    let previous = store
        .get_user_settings_prefs(&session.user_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let theme = normalize_choice(
        form.theme.as_deref(),
        &SUPPORTED_THEMES,
        existing_record
            .as_ref()
            .map(|record| record.theme.as_str())
            .unwrap_or("system"),
    );
    let locale = normalize_choice(
        form.locale.as_deref(),
        &SUPPORTED_LOCALES,
        existing_record
            .as_ref()
            .map(|record| record.locale.as_str())
            .unwrap_or("en"),
    );
    let mut prefs = settings_from_form(&form, &previous);

    if prefs.email_digest_enabled {
        let recipients = split_recipients(&prefs.email_digest_recipients);
        if recipients.is_empty() {
            return super::render_template(&render_settings_page(
                ctx,
                existing_record.as_ref(),
                &prefs,
                &health,
                None,
                Some("Add at least one digest recipient when email digest is enabled.".to_string()),
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
                existing_record.as_ref(),
                &prefs,
                &health,
                None,
                Some(format!(
                    "Invalid recipient email(s): {}",
                    invalid.join(", ")
                )),
            ));
        }
        if !is_valid_hhmm(&prefs.email_digest_time_cet) {
            return super::render_template(&render_settings_page(
                ctx,
                existing_record.as_ref(),
                &prefs,
                &health,
                None,
                Some("Digest send time must use HH:MM format (CET).".to_string()),
            ));
        }
        if prefs.notification_frequency.eq_ignore_ascii_case("weekly")
            && !is_valid_weekday(&prefs.email_digest_weekday)
        {
            return super::render_template(&render_settings_page(
                ctx,
                existing_record.as_ref(),
                &prefs,
                &health,
                None,
                Some("For weekly digest, select a valid weekday.".to_string()),
            ));
        }
    }

    if let Ok(Some(existing)) = store
        .get_user_settings_prefs_scoped(&session.user_id, session.role.as_str())
        .await
    {
        prefs.email_digest_last_sent_at = existing.email_digest_last_sent_at;
    }

    if let Err(e) = store
        .upsert_user_settings_prefs_scoped(
            &session.user_id,
            session.role.as_str(),
            &theme,
            &locale,
            &prefs,
        )
        .await
    {
        tracing::error!(
            "Failed to persist user settings for {}: {e}",
            session.username
        );
        return super::render_template(&render_settings_page(
            ctx,
            existing_record.as_ref(),
            &prefs,
            &health,
            None,
            Some("Failed to save settings. Please retry.".to_string()),
        ));
    }

    // The response itself carries the new appearance so the browser applies
    // the saved theme/table layout immediately (and on future first visits).
    let saved_record = UserPreferencesRecord {
        theme: theme.clone(),
        locale,
        preferences: existing_record
            .map(|record| record.preferences)
            .unwrap_or_else(|| serde_json::json!({})),
        updated_at: chrono::Utc::now(),
    };
    let page = render_settings_page(
        ctx,
        Some(&saved_record),
        &prefs,
        &health,
        Some("Preferences saved.".to_string()),
        None,
    );
    let table_layout = page.table_layout.clone();
    let mut response = super::render_template(&page);
    append_appearance_cookies(&mut response, &theme, &table_layout);
    response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn theme_choice_falls_back_to_existing_value_for_garbage_input() {
        assert_eq!(
            normalize_choice(Some("dark"), &SUPPORTED_THEMES, "system"),
            "dark"
        );
        assert_eq!(
            normalize_choice(Some("rainbow"), &SUPPORTED_THEMES, "system"),
            "system"
        );
        assert_eq!(normalize_choice(None, &SUPPORTED_THEMES, "dark"), "dark");
    }

    #[test]
    fn session_hours_are_clamped_server_side() {
        assert_eq!(normalize_session_hours(Some(0), 24), MIN_SESSION_HOURS);
        assert_eq!(normalize_session_hours(Some(10_000), 24), MAX_SESSION_HOURS);
        assert_eq!(normalize_session_hours(Some(8), 24), 8);
        assert_eq!(normalize_session_hours(None, 72), 72);
    }

    #[test]
    fn removed_system_controls_are_not_parsed_into_personal_prefs() {
        let form = SettingsForm {
            theme: Some("dark".into()),
            locale: Some("fr".into()),
            table_layout: Some("compact".into()),
            session_timeout_hours: Some(8),
            default_region: Some("EU".into()),
            minimum_severity: Some("medium".into()),
            critical_only_enabled: Some("on".into()),
            email_digest_enabled: None,
            notification_frequency: Some("Weekly".into()),
            email_digest_recipients: None,
            email_digest_categories: vec!["warnings".into(), "demand_signal".into()],
            email_digest_time_cet: Some("07:30".into()),
            email_digest_weekday: Some("Fri".into()),
        };

        let prefs = settings_from_form(&form, &UserSettingsPrefs::default());

        assert_eq!(prefs.session_timeout_hours, 8);
        assert_eq!(prefs.table_layout, "compact");
        assert_eq!(prefs.default_region, "EU");
        assert_eq!(prefs.minimum_severity, "medium");
        assert!(prefs.critical_only_enabled);
        assert_eq!(prefs.email_digest_categories, vec!["demand_signal"]);
        assert_eq!(prefs.notification_frequency, "Weekly");
        assert_eq!(prefs.email_digest_time_cet, "07:30");
        assert_eq!(prefs.email_digest_weekday, "Fri");
    }

    #[test]
    fn invalid_locale_and_layout_fall_back_to_previous_values() {
        let previous = UserSettingsPrefs {
            table_layout: "compact".into(),
            ..UserSettingsPrefs::default()
        };
        let form = SettingsForm {
            theme: None,
            locale: None,
            table_layout: Some("tiny".into()),
            session_timeout_hours: None,
            default_region: None,
            minimum_severity: None,
            critical_only_enabled: None,
            email_digest_enabled: None,
            notification_frequency: None,
            email_digest_recipients: None,
            email_digest_categories: vec![],
            email_digest_time_cet: None,
            email_digest_weekday: None,
        };

        let prefs = settings_from_form(&form, &previous);

        assert_eq!(prefs.table_layout, "compact");
        assert_eq!(prefs.default_region, "Global");
    }

    #[test]
    fn system_config_sections_never_expose_secret_values() {
        std::env::set_var("TEST_SETTINGS_SECRET_TOKEN", "super-secret-value");
        let sections = system_config_sections();
        std::env::remove_var("TEST_SETTINGS_SECRET_TOKEN");

        for section in &sections {
            for item in &section.items {
                assert!(
                    !item.value.contains("super-secret-value"),
                    "secret value leaked for {}",
                    item.env_key
                );
            }
        }
    }

    #[test]
    fn table_layout_helper_reads_settings_page_json() {
        use crate::middleware::session::table_layout_from_preferences;

        let preferences = serde_json::json!({"settings_page": {"table_layout": "compact"}});
        assert_eq!(
            table_layout_from_preferences(&preferences).as_deref(),
            Some("compact")
        );
        assert_eq!(table_layout_from_preferences(&serde_json::json!({})), None);
    }
}
