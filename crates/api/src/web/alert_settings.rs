//! Web handler for the Alert Settings HTML page (`/settings/alerts`).
//!
//! Renders the `settings_alerts.html` Askama template with global defaults
//! and a list of per-entity alert overrides loaded from the store.

use askama::Template;
use axum::{extract::Extension, response::Response};
use std::sync::Arc;

use apex_core::alert_config::{AlertChannel, EntityAlertConfig};
use apex_store::postgres::PgStore;

use super::{render_template, PageContext};
use crate::middleware::session::WebSession;

// ─────────────────────────────────────────────────────────────────────────────
// Template
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/settings_alerts.html")]
pub struct AlertSettingsPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub save_success: Option<String>,
    pub save_error: Option<String>,

    // Global defaults
    pub global_min_severity: String,
    pub global_cooldown: u32,
    pub global_max_daily: u32,
    pub global_channels_in_app: bool,
    pub global_channels_email: bool,
    pub global_channels_slack: bool,
    pub global_channels_webhook: bool,

    // Entity overrides
    pub entity_configs: Vec<EntityAlertConfig>,
    pub entity_count: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler
// ─────────────────────────────────────────────────────────────────────────────

/// GET /settings/alerts
pub async fn alert_settings_page(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> Response {
    let unack = store
        .count_warnings(&apex_store::postgres::WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/settings/alerts", unack);

    // Load global defaults from DB (fall back to Rust defaults)
    let global_defaults = store
        .get_global_alert_defaults()
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    // Load entity overrides
    let entity_configs = store.list_entity_alert_configs().await.unwrap_or_default();

    let entity_count = entity_configs.len();

    render_template(&AlertSettingsPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        save_success: None,
        save_error: None,

        global_min_severity: global_defaults.min_severity.as_str().to_string(),
        global_cooldown: global_defaults.cooldown_minutes,
        global_max_daily: global_defaults.max_daily_alerts,
        global_channels_in_app: global_defaults
            .enabled_channels
            .contains(&AlertChannel::InApp),
        global_channels_email: global_defaults
            .enabled_channels
            .contains(&AlertChannel::Email),
        global_channels_slack: global_defaults
            .enabled_channels
            .contains(&AlertChannel::Slack),
        global_channels_webhook: global_defaults
            .enabled_channels
            .contains(&AlertChannel::Webhook),

        entity_configs,
        entity_count,
    })
}
