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
use apex_core::data_state::{DataState, DegradedNotice};

// ─────────────────────────────────────────────────────────────────────────────
// Template
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/settings_alerts.html")]
pub struct AlertSettingsPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub save_success: Option<String>,
    pub save_error: Option<String>,
    /// Rendered when alert-config queries failed, instead of an empty list.
    pub degraded_notice: Option<String>,

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

    let mut degraded_notice: Option<String> = None;

    // Load global defaults from DB (fall back to Rust defaults). A failed
    // query is reported as degraded; only "no row configured" uses defaults.
    let global_defaults_state = DataState::from_result(
        store.get_global_alert_defaults().await,
        "failed to load global alert defaults",
        |value| value.is_none(),
    );
    DegradedNotice::capture(&global_defaults_state, &mut degraded_notice);
    let global_defaults = global_defaults_state
        .into_loaded_or_default()
        .unwrap_or_default();

    // Load entity overrides
    let entity_configs_state = DataState::from_result(
        store.list_entity_alert_configs().await,
        "failed to list entity alert configs",
        |configs| configs.is_empty(),
    );
    DegradedNotice::capture(&entity_configs_state, &mut degraded_notice);
    let entity_configs = entity_configs_state.into_items();

    let entity_count = entity_configs.len();

    render_template(&AlertSettingsPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        save_success: None,
        save_error: None,
        degraded_notice,

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
