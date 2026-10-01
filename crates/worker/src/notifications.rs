//! Alert data types, channel configuration and formatters for the durable
//! notification pipeline.
//!
//! Delivery is owned exclusively by
//! [`crate::notification_delivery`]: alerts are persisted, one row per channel
//! selected by [`crate::notification_delivery::ConfiguredChannelRouter::channels_for`]
//! (severity/priority thresholds applied), and the retry processor performs
//! the webhook/SMTP sends. This module no longer performs any network sends.
//!
//! LLM-enhanced alert bodies are available when the `llm` feature is active.

use apex_core::config::ConfigErrors;
use apex_core::sla::SeveritySlaConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::slack::{clip, slack_escape};

/// Redact a credential-bearing URL to `scheme://host/…last4`.
///
/// Webhook URLs embed their secret in the path, so they must never be written
/// to logs or persisted error text. Keeping the scheme, host and last four
/// characters lets an operator identify the endpoint without leaking it.
pub(crate) fn redact_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let (scheme, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, trimmed),
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|host| !host.is_empty())
        .unwrap_or("…");
    let last4: String = {
        let chars: Vec<char> = trimmed.chars().collect();
        chars[chars.len().saturating_sub(4)..].iter().collect()
    };
    match scheme {
        Some(scheme) => format!("{scheme}://{host}/…{last4}"),
        None => format!("{host}/…{last4}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert data types
// ─────────────────────────────────────────────────────────────────────────────

/// Severity of an outgoing alert.
///
/// Re-exported from `apex_core::alert_config` to ensure a single canonical
/// definition across the entire platform.
pub use apex_core::alert_config::AlertSeverity;

/// Who/what an outgoing alert is about.
///
/// Re-exported from `apex_core::alert_config` so workers, the API and the UI
/// share the exact `Entity` / `Entities` / `Users` / `SystemBroadcast` model.
pub use apex_core::alert_config::AlertScope;

/// A pending alert derived from an `InsightCard` or a pipeline event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "PendingAlertWire")]
pub struct PendingAlert {
    /// Originating insight / event id.
    pub source_id: String,
    /// Entity set, explicit users, or an explicit system broadcast. Replaces
    /// the legacy `"system"` fake entity string.
    pub scope: AlertScope,
    pub title: String,
    pub body: String,
    pub severity: AlertSeverity,
    /// Priority score in [0, 1].
    pub priority_score: f64,
    pub category: String,
    pub region: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Optional LLM-generated narrative override.
    pub llm_narrative: Option<String>,
}

/// Wire form that still accepts the legacy `entity_id` / `entity_name` pair,
/// so notification dead-letter rows written before the `AlertScope` migration
/// remain replayable instead of being silently dropped.
#[derive(Deserialize)]
struct PendingAlertWire {
    source_id: String,
    #[serde(default)]
    scope: Option<AlertScope>,
    #[serde(default)]
    entity_id: Option<String>,
    #[serde(default)]
    entity_name: Option<String>,
    title: String,
    body: String,
    severity: AlertSeverity,
    priority_score: f64,
    #[serde(default)]
    category: String,
    #[serde(default)]
    region: Option<String>,
    created_at: DateTime<Utc>,
    #[serde(default)]
    llm_narrative: Option<String>,
}

impl From<PendingAlertWire> for PendingAlert {
    fn from(wire: PendingAlertWire) -> Self {
        let scope = wire
            .scope
            .unwrap_or_else(|| match wire.entity_id.as_deref() {
                // The legacy SLA fallback used a fake `"system"` entity id for
                // warnings with no entities. Those alerts resolved to nobody
                // (`Users([])`), so replaying them must not silently upgrade
                // them into a platform-wide broadcast.
                Some("system") | None | Some("") => AlertScope::Users {
                    user_ids: Vec::new(),
                },
                Some(entity_id) => AlertScope::Entity {
                    entity_id: entity_id.to_string(),
                    entity_name: wire.entity_name.clone(),
                },
            });

        Self {
            source_id: wire.source_id,
            scope,
            title: wire.title,
            body: wire.body,
            severity: wire.severity,
            priority_score: wire.priority_score,
            category: wire.category,
            region: wire.region,
            created_at: wire.created_at,
            llm_narrative: wire.llm_narrative,
        }
    }
}

impl PendingAlert {
    pub fn new(
        source_id: impl Into<String>,
        scope: AlertScope,
        title: impl Into<String>,
        body: impl Into<String>,
        severity: AlertSeverity,
        priority_score: f64,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            scope,
            title: title.into(),
            body: body.into(),
            severity,
            priority_score,
            category: String::new(),
            region: None,
            created_at: Utc::now(),
            llm_narrative: None,
        }
    }

    /// Human-readable label for notification bodies.
    pub fn display_name(&self) -> String {
        self.scope.display()
    }

    /// Convert into the canonical real-time alert event.
    ///
    /// The event id is deterministic (UUIDv5 of the source id) so repeated
    /// scheduler runs describe the same logical alert. The broker dedupe
    /// identity is the outbox row id sent as `Nats-Msg-Id`, not this id.
    pub fn to_alert_event(&self) -> crate::nats_stream::AlertEvent {
        use crate::nats_stream::{AlertEvent, AlertEventType};

        let event_type = match self.category.as_str() {
            "insight" | "competitive_intel" | "market_intelligence" => AlertEventType::NewInsight,
            "warning" | "verification" | "sla_breach" | "sla_reminder" => {
                AlertEventType::NewWarning
            }
            "recipe_match" | "opportunity" | "demand_procurement" => AlertEventType::RecipeMatch,
            "competitor_change" | "competitor" => AlertEventType::CompetitorChange,
            "supply_chain" | "supply_chain_risk" => AlertEventType::SupplyChainRisk,
            _ => AlertEventType::SystemAlert,
        };

        AlertEvent {
            id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, self.source_id.as_bytes()),
            event_type,
            severity: self.severity,
            title: self.title.clone(),
            description: self
                .llm_narrative
                .clone()
                .unwrap_or_else(|| self.body.clone()),
            // Full entity set: every referenced entity is parsed so the API
            // router can resolve all of their real subscribers. A non-UUID
            // entity yields None and no subscribers for that id rather than
            // reaching everyone.
            entity_ids: self
                .scope
                .entity_ids()
                .into_iter()
                .filter_map(|entity_id| uuid::Uuid::parse_str(entity_id).ok())
                .collect(),
            entity_name: match &self.scope {
                AlertScope::Entity { entity_name, .. } => entity_name.clone(),
                _ => None,
            },
            // Audience is derived from the scope: explicit users stay explicit,
            // a deliberate system broadcast is the only Broadcast, and entity
            // scopes resolve real subscribers in the API alert router.
            audience: self.scope.audience(),
            metadata: serde_json::json!({
                "source_id": self.source_id,
                "category": self.category,
                "priority_score": self.priority_score,
                "region": self.region,
            }),
            created_at: self.created_at,
        }
    }
}

/// A fully-formed notification ready to dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub channel: String,
    pub destination: Option<String>,
    pub alert: PendingAlert,
    pub subject: Option<String>,
    pub formatted_body: String,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub dispatch_success: Option<bool>,
    pub error_message: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Channel configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Wire format used to render an alert for a webhook endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookFormat {
    /// Slack Block Kit payload (Slack incoming webhook).
    Slack,
    /// Microsoft Teams `MessageCard` payload.
    Teams,
    /// Generic JSON object: `{title, severity, description, entity, link}`.
    #[default]
    Json,
}

/// A webhook endpoint destination (Slack, Teams, generic HTTP).
#[derive(Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub name: String,
    pub url: String,
    /// Optional `Authorization: Bearer <token>` header.
    pub bearer_token: Option<String>,
    /// Wire format for this endpoint. Slack hooks use [`WebhookFormat::Slack`];
    /// non-Slack endpoints default to [`WebhookFormat::Json`].
    #[serde(default)]
    pub format: WebhookFormat,
    /// Minimum severity to send over this webhook.
    pub min_severity: AlertSeverity,
    /// Minimum priority_score [0, 1] to send over this webhook.
    pub min_priority: f64,
}

/// `Debug` must never print the webhook URL or bearer token: both are
/// credentials. The URL is shown redacted and the token as `<redacted>`
/// (audit #73).
impl std::fmt::Debug for WebhookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookConfig")
            .field("name", &self.name)
            .field("url", &redact_url(&self.url))
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("format", &self.format)
            .field("min_severity", &self.min_severity)
            .field("min_priority", &self.min_priority)
            .finish()
    }
}

impl WebhookConfig {
    pub fn slack(url: impl Into<String>) -> Self {
        Self {
            name: "slack".into(),
            url: url.into(),
            bearer_token: None,
            format: WebhookFormat::Slack,
            min_severity: AlertSeverity::Medium,
            min_priority: 0.5,
        }
    }

    pub fn critical_only(url: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            bearer_token: None,
            // A critical paging hook is usually a non-Slack receiver; its
            // historical Slack-shaped body was unintended (audit #72).
            format: WebhookFormat::Json,
            min_severity: AlertSeverity::Critical,
            min_priority: 0.8,
        }
    }

    /// Read a `<HOOK>_FORMAT` env override (`slack`, `teams`, `json`).
    fn format_from_env(key: &str) -> Option<WebhookFormat> {
        let value = std::env::var(key).ok()?;
        match value.trim().to_ascii_lowercase().as_str() {
            "slack" => Some(WebhookFormat::Slack),
            "teams" => Some(WebhookFormat::Teams),
            "json" => Some(WebhookFormat::Json),
            _ => None,
        }
    }
}

/// Parameters for a prepared email alert (SMTP sending handled downstream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub to_addresses: Vec<String>,
    pub from_address: String,
    pub subject_prefix: String,
    pub min_severity: AlertSeverity,
    pub min_priority: f64,
}

/// Overall notification configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationConfig {
    pub webhooks: Vec<WebhookConfig>,
    pub email: Option<EmailConfig>,
    /// Belt-and-suspenders: always log alerts at this severity or above.
    pub log_min_severity: AlertSeverity,
}

impl NotificationConfig {
    /// Create from environment variables.
    ///
    /// This is the single environment contract for Slack/webhook alert
    /// delivery (audit #81). Reads:
    /// - `SLACK_WEBHOOK_URL` — Slack endpoint (format defaults to Slack,
    ///   override with `SLACK_WEBHOOK_FORMAT=slack|teams|json`)
    /// - `CRITICAL_WEBHOOK_URL` — critical-only paging endpoint (format
    ///   defaults to JSON, override with `CRITICAL_WEBHOOK_FORMAT`)
    /// - `ALERT_EMAIL_TO` (comma-separated list, empty entries ignored)
    /// - `ALERT_EMAIL_FROM`
    pub fn from_env() -> Self {
        let mut cfg = NotificationConfig {
            log_min_severity: AlertSeverity::Low,
            ..Default::default()
        };

        if let Ok(url) = std::env::var("SLACK_WEBHOOK_URL") {
            let mut webhook = WebhookConfig::slack(url);
            if let Some(format) = WebhookConfig::format_from_env("SLACK_WEBHOOK_FORMAT") {
                webhook.format = format;
            }
            cfg.webhooks.push(webhook);
        }
        if let Ok(url) = std::env::var("CRITICAL_WEBHOOK_URL") {
            let mut webhook = WebhookConfig::critical_only(url, "critical-hook");
            if let Some(format) = WebhookConfig::format_from_env("CRITICAL_WEBHOOK_FORMAT") {
                webhook.format = format;
            }
            cfg.webhooks.push(webhook);
        }
        if let Ok(to_raw) = std::env::var("ALERT_EMAIL_TO") {
            // A trailing comma (or a whitespace-only entry) must not create a
            // delivery row that can never succeed (audit #74).
            let to_addresses: Vec<String> = to_raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !to_addresses.is_empty() {
                let from = std::env::var("ALERT_EMAIL_FROM")
                    .unwrap_or_else(|_| "alerts@apexintel.io".to_string());
                cfg.email = Some(EmailConfig {
                    to_addresses,
                    from_address: from,
                    subject_prefix: "[ApexIntel Alert]".to_string(),
                    min_severity: AlertSeverity::High,
                    min_priority: 0.65,
                });
            }
        }

        cfg
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity-aware alert filtering
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if the alert **should** be dispatched according to the
/// entity's alert configuration.
///
/// When no per-entity config exists the alert is allowed through (the caller
/// should fall back to channel-level thresholds in
/// [`crate::notification_delivery::ConfiguredChannelRouter::channels_for`]).
pub fn should_send_alert(
    alert: &PendingAlert,
    entity_config: Option<&apex_core::alert_config::EntityAlertConfig>,
) -> bool {
    match entity_config {
        Some(cfg) => !cfg.is_alert_suppressed(&alert.category, alert.severity),
        None => true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatters
// ─────────────────────────────────────────────────────────────────────────────

/// Slack Block Kit JSON for an alert.
///
/// This is the Slack rendering used by the durable channel router; the former
/// `NotificationDispatcher` fan-out that duplicated webhook and SMTP sending
/// was dead code and has been removed (audit #81) — the router in
/// `notification_delivery.rs` is the single delivery pipeline.
pub(crate) fn format_slack_message(alert: &PendingAlert) -> String {
    let emoji = match alert.severity {
        AlertSeverity::Critical => "🚨",
        AlertSeverity::High => "🔴",
        AlertSeverity::Medium => "🟡",
        AlertSeverity::Low => "🔵",
        AlertSeverity::Info => "ℹ️",
    };

    let body_text = alert.llm_narrative.as_deref().unwrap_or(&alert.body);
    let region = alert.region.as_deref().unwrap_or("Global");
    let entity = alert.display_name();
    let severity_upper = alert.severity.as_str().to_uppercase();

    // Every crawled/human-authored string is escaped before it reaches a
    // mrkdwn field, and every block is clipped to its Slack limit (header 150,
    // field 2000, section 3000, top-level text 4000) so oversized alert text
    // is delivered instead of rejected as `invalid_blocks` (audit #56/#68).
    serde_json::json!({
        "text": clip(
            &format!(
                "{emoji} *[{}] {}*\n",
                severity_upper,
                slack_escape(&alert.title)
            ),
            4000
        ),
        "blocks": [
            {
                "type": "header",
                "text": {
                    "type": "plain_text",
                    "text": clip(
                        &format!("{emoji} {} — {}", severity_upper, alert.title),
                        150
                    )
                }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": clip(&format!("*Entity:*\n{}", slack_escape(&entity)), 2000) },
                    { "type": "mrkdwn", "text": clip(&format!("*Region:*\n{}", slack_escape(region)), 2000) },
                    { "type": "mrkdwn", "text": clip(&format!("*Category:*\n{}", slack_escape(&alert.category)), 2000) },
                    { "type": "mrkdwn", "text": clip(&format!("*Priority Score:*\n{:.2}", alert.priority_score), 2000) },
                ]
            },
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": clip(&slack_escape(body_text), 3000) }
            },
            {
                "type": "context",
                "elements": [
                    { "type": "mrkdwn", "text": clip(&format!("Source ID: `{}` | {}", alert.source_id, alert.created_at.format("%Y-%m-%d %H:%M UTC")), 3000) }
                ]
            }
        ]
    })
    .to_string()
}

/// Plain-text email body for an alert.
pub(crate) fn format_email_body(alert: &PendingAlert) -> String {
    let body_text = alert.llm_narrative.as_deref().unwrap_or(&alert.body);
    format!(
        "ApexIntel Intelligence Alert\n\
        ==============================\n\
        Severity:       {}\n\
        Entity:         {}\n\
        Category:       {}\n\
        Region:         {}\n\
        Priority Score: {:.2}\n\
        \n\
        {}\n\
        \n\
        -- \n\
        Source ID: {}\n\
        Generated: {}\n",
        alert.severity.as_str().to_uppercase(),
        alert.display_name(),
        alert.category,
        alert.region.as_deref().unwrap_or("Global"),
        alert.priority_score,
        body_text,
        alert.source_id,
        alert.created_at.format("%Y-%m-%d %H:%M UTC"),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// SLA enforcement
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the shared SLA windows from the environment.
///
/// An absent variable keeps its default; a present-but-malformed value is a
/// configuration error instead of a silent fallback, because these windows
/// gate breach detection.
fn shared_sla_config_from_env() -> std::result::Result<SeveritySlaConfig, ConfigErrors> {
    fn parse_env(key: &str, default: i64, errors: &mut ConfigErrors) -> i64 {
        match std::env::var(key) {
            Err(_) => default,
            Ok(raw) => match raw.trim().parse::<i64>() {
                Ok(value) if value > 0 => value,
                _ => {
                    errors.push(key, raw, "a positive integer");
                    default
                }
            },
        }
    }

    let defaults = SeveritySlaConfig::default();
    let mut errors = ConfigErrors::new();
    let config = SeveritySlaConfig {
        critical_seconds: parse_env(
            "SLA_CRITICAL_SECONDS",
            defaults.critical_seconds,
            &mut errors,
        ),
        high_seconds: parse_env("SLA_HIGH_SECONDS", defaults.high_seconds, &mut errors),
        medium_seconds: parse_env("SLA_MEDIUM_SECONDS", defaults.medium_seconds, &mut errors),
        low_seconds: parse_env("SLA_LOW_SECONDS", defaults.low_seconds, &mut errors),
    };
    errors.into_result()?;
    Ok(config)
}

/// A warning record fetched from the database for SLA evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaWarningRecord {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub warning_type: String,
    /// Full entity set referenced by the warning. The previous `entity_id`
    /// field took `entity_ids[1]`, silently dropping every other entity.
    #[serde(default)]
    pub entity_ids: Vec<String>,
    /// True only for warnings that producers explicitly marked as system
    /// broadcasts (`NewWarning::system_broadcast`). Persisted in migration 069.
    #[serde(default)]
    pub is_system_broadcast: bool,
    pub created_at: chrono::DateTime<Utc>,
    pub acknowledged: bool,
}

impl SlaWarningRecord {
    /// Compute how long this warning has been unacknowledged (seconds).
    pub fn age_seconds(&self) -> i64 {
        (Utc::now() - self.created_at).num_seconds().max(0)
    }

    /// Check whether this warning has breached its SLA given the configured windows.
    pub fn is_sla_breached(&self, windows: &SeveritySlaConfig) -> bool {
        if self.acknowledged {
            return false;
        }
        self.age_seconds() > windows.deadline_seconds(&self.severity)
    }

    /// Seconds remaining until SLA breach (negative if already breached).
    pub fn sla_seconds_remaining(&self, windows: &SeveritySlaConfig) -> i64 {
        windows.deadline_seconds(&self.severity) - self.age_seconds()
    }

    /// Alert scope for this warning.
    ///
    /// Only warnings explicitly persisted as system broadcasts become
    /// [`AlertScope::SystemBroadcast`]. An unscoped warning (no entities, no
    /// broadcast flag) is [`AlertScope::Users`] with an empty list — explicitly
    /// nobody — because "no entities" is not the same as "everyone": inferring
    /// a broadcast here would page the whole platform and bypass per-user
    /// preferences.
    pub fn scope(&self) -> AlertScope {
        if self.is_system_broadcast {
            return AlertScope::SystemBroadcast;
        }
        match self.entity_ids.as_slice() {
            [] => AlertScope::Users {
                user_ids: Vec::new(),
            },
            [entity_id] => AlertScope::entity(entity_id.clone(), None::<String>),
            entity_ids => AlertScope::entities(entity_ids.iter().cloned()),
        }
    }
}

/// Evaluates unacknowledged warnings against SLA deadlines and produces
/// escalation alerts for dispatch.
///
/// This is a lightweight struct — it holds configuration but no persistent
/// state.  Call `check_sla_violations()` on a periodic tick (every 60s is
/// recommended).
///
/// # Integration
/// The caller is responsible for:
/// 1. Fetching `SlaWarningRecord`s from the database.
/// 2. Calling `check_sla_violations()` with those records.
/// 3. Enqueuing the returned `Vec<PendingAlert>` into the durable pipeline
///    with [`crate::notification_delivery::ConfiguredChannelRouter::channels_for`].
///
/// ```
/// use apex_core::sla::SeveritySlaConfig;
/// use apex_worker::notifications::{SlaEnforcer, SlaWarningRecord};
/// let enforcer = SlaEnforcer::new(SeveritySlaConfig::default());
/// // let violations = enforcer.check_sla_violations(&records);
/// ```
pub struct SlaEnforcer {
    windows: SeveritySlaConfig,
}

impl SlaEnforcer {
    pub fn new(windows: SeveritySlaConfig) -> Self {
        Self { windows }
    }

    pub fn from_env() -> std::result::Result<Self, ConfigErrors> {
        Ok(Self::new(shared_sla_config_from_env()?))
    }

    /// The resolved SLA windows this enforcer applies.
    ///
    /// Callers must use these (not `SeveritySlaConfig::default()`) for any
    /// companion metadata so enforcement and reporting share one config.
    pub fn windows(&self) -> &SeveritySlaConfig {
        &self.windows
    }

    pub fn build_breach_alert(&self, record: &SlaWarningRecord) -> Option<PendingAlert> {
        if !record.is_sla_breached(&self.windows) {
            return None;
        }

        let overdue_seconds =
            record.age_seconds() - self.windows.deadline_seconds(&record.severity);
        let overdue_minutes = overdue_seconds / 60;
        let title = format!(
            "SLA BREACH — {} unacknowledged warning: {}",
            record.severity.to_uppercase(),
            record.title
        );
        let body = format!(
            "Warning ID {} has not been acknowledged and has breached its {} SLA by {}m. Original warning: '{}'. Severity: {}.",
            record.id, record.severity, overdue_minutes, record.title, record.severity,
        );

        let severity = AlertSeverity::from_str(&record.severity);
        let escalated_severity = severity.max(AlertSeverity::High);

        let mut alert = PendingAlert::new(
            format!("sla-breach:{}", record.id),
            record.scope(),
            title,
            body,
            escalated_severity,
            0.95,
        );
        alert.category = "sla_breach".to_string();
        Some(alert)
    }

    pub fn build_approaching_alert(
        &self,
        record: &SlaWarningRecord,
        warn_ahead_seconds: i64,
    ) -> Option<PendingAlert> {
        if record.acknowledged {
            return None;
        }

        let remaining = record.sla_seconds_remaining(&self.windows);
        if remaining <= 0 || remaining > warn_ahead_seconds {
            return None;
        }

        let title = format!(
            "SLA reminder — {} warning nearing deadline: {}",
            record.severity.to_uppercase(),
            record.title
        );
        let body = format!(
            "Warning ID {} is still unacknowledged and will breach its {} SLA in {}m. Original warning: '{}'.",
            record.id,
            record.severity,
            (remaining / 60).max(1),
            record.title,
        );

        let mut alert = PendingAlert::new(
            format!("sla-reminder:{}", record.id),
            record.scope(),
            title,
            body,
            AlertSeverity::High,
            0.8,
        );
        alert.category = "sla_reminder".to_string();
        Some(alert)
    }

    /// Evaluate a slice of warning records and return `PendingAlert`s for
    /// every record that has breached its SLA.
    pub fn check_sla_violations(&self, records: &[SlaWarningRecord]) -> Vec<PendingAlert> {
        let mut escalations = Vec::new();

        for record in records {
            if let Some(alert) = self.build_breach_alert(record) {
                info!(
                    warning_id = %record.id,
                    severity = %record.severity,
                    "SLA breach escalation triggered"
                );
                escalations.push(alert);
            }
        }

        escalations
    }

    /// Return only records that are approaching their SLA deadline (within `warn_ahead_seconds`).
    ///
    /// Useful for sending a "heads-up" notification before the actual breach.
    pub fn approaching_sla<'a>(
        &self,
        records: &'a [SlaWarningRecord],
        warn_ahead_seconds: i64,
    ) -> Vec<&'a SlaWarningRecord> {
        records
            .iter()
            .filter(|r| {
                if r.acknowledged {
                    return false;
                }
                let remaining = r.sla_seconds_remaining(&self.windows);
                remaining > 0 && remaining <= warn_ahead_seconds
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default
    )]

    use super::*;

    fn make_alert(severity: AlertSeverity, priority: f64) -> PendingAlert {
        PendingAlert::new(
            "src-001",
            AlertScope::entity("entity-1", Some("Test Corp")),
            "Test Alert",
            "Body text.",
            severity,
            priority,
        )
    }

    #[test]
    fn severity_ordering_works() {
        assert!(AlertSeverity::Critical > AlertSeverity::High);
        assert!(AlertSeverity::High > AlertSeverity::Medium);
        assert!(AlertSeverity::Medium > AlertSeverity::Low);
        assert!(AlertSeverity::Low > AlertSeverity::Info);
    }

    #[test]
    fn from_str_parses_case_insensitive() {
        assert_eq!(AlertSeverity::from_str("CRITICAL"), AlertSeverity::Critical);
        assert_eq!(AlertSeverity::from_str("medium"), AlertSeverity::Medium);
        assert_eq!(AlertSeverity::from_str("unknown"), AlertSeverity::Info);
    }

    #[test]
    fn slack_message_format_is_valid_json() {
        let alert = make_alert(AlertSeverity::Critical, 0.95);
        let msg = format_slack_message(&alert);
        let _parsed: serde_json::Value =
            serde_json::from_str(&msg).expect("slack message should be valid JSON");
    }

    #[test]
    fn email_body_contains_entity_name() {
        let alert = make_alert(AlertSeverity::High, 0.8);
        let body = format_email_body(&alert);
        assert!(body.contains("Test Corp"));
        assert!(body.contains("HIGH"));
    }

    #[test]
    fn slack_body_escapes_crawled_text_and_clips_blocks() {
        let mut alert = make_alert(AlertSeverity::Critical, 0.95);
        alert.title = "<!channel> breach".to_string();
        alert.body = "<https://evil.example|Click here> & more".to_string();
        alert.category = "<!here> cyber".to_string();

        let msg = format_slack_message(&alert);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        // Top-level fallback text is mrkdwn: escaped.
        let fallback = parsed["text"].as_str().unwrap();
        assert!(fallback.contains("&lt;!channel&gt;"));
        assert!(!fallback.contains("<!channel>"));

        // Body section is mrkdwn: escaped.
        let body_text = parsed["blocks"][2]["text"]["text"].as_str().unwrap();
        assert_eq!(
            body_text,
            "&lt;https://evil.example|Click here&gt; &amp; more"
        );

        // Field values are mrkdwn: escaped.
        let fields = parsed["blocks"][1]["fields"].as_array().unwrap();
        let category = fields[2]["text"].as_str().unwrap();
        assert!(category.contains("&lt;!here&gt;"));
        assert!(!category.contains("<!here>"));

        // Header is plain_text and short; clipped to 150 chars.
        let header = parsed["blocks"][0]["text"]["text"].as_str().unwrap();
        assert!(header.chars().count() <= 150);

        // Long crawled values never exceed their block limits.
        alert.title = "T".repeat(10_000);
        alert.body = "B".repeat(10_000);
        let msg = format_slack_message(&alert);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert!(parsed["text"].as_str().unwrap().chars().count() <= 4000);
        assert!(
            parsed["blocks"][0]["text"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 150
        );
        assert!(
            parsed["blocks"][2]["text"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 3000
        );
    }

    #[test]
    fn slack_body_escapes_entity_region_text_and_fallback() {
        let mut alert = make_alert(AlertSeverity::Critical, 0.95);
        alert.scope = AlertScope::entity(
            "entity-1",
            Some("<!channel> Acme & Sons <https://evil.example|Co>"),
        );
        alert.region = Some("<https://evil.example|Region> & <!here>".to_string());
        alert.title = "<!channel> *pwn* & <https://evil.example|t>".to_string();

        let msg = format_slack_message(&alert);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        // The top-level fallback text is mrkdwn: the hostile title is escaped.
        let fallback = parsed["text"].as_str().unwrap();
        assert!(fallback.contains("&lt;!channel&gt;"), "{fallback}");
        assert!(
            fallback.contains("&lt;https://evil.example|t&gt;"),
            "{fallback}"
        );
        assert!(fallback.contains("&amp;"), "{fallback}");
        assert!(!fallback.contains("<!channel>"), "{fallback}");

        // Entity and region fields are mrkdwn and must be escaped too.
        let fields = parsed["blocks"][1]["fields"].as_array().unwrap();
        let entity = fields[0]["text"].as_str().unwrap();
        assert_eq!(
            entity,
            "*Entity:*\n&lt;!channel&gt; Acme &amp; Sons &lt;https://evil.example|Co&gt;"
        );
        let region = fields[1]["text"].as_str().unwrap();
        assert_eq!(
            region,
            "*Region:*\n&lt;https://evil.example|Region&gt; &amp; &lt;!here&gt;"
        );

        // The body section and every field stay inside their Slack limits even
        // when the crawled text is huge (audit #68).
        alert.scope = AlertScope::entity("entity-1", Some(&"E".repeat(10_000)));
        alert.region = Some("R".repeat(10_000));
        let msg = format_slack_message(&alert);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert!(parsed["text"].as_str().unwrap().chars().count() <= 4000);
        assert!(
            parsed["blocks"][0]["text"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 150
        );
        for field in parsed["blocks"][1]["fields"].as_array().unwrap() {
            assert!(
                field["text"].as_str().unwrap().chars().count() <= 2000,
                "every field is clipped to Slack's 2000-char field limit"
            );
        }
        assert!(
            parsed["blocks"][2]["text"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 3000
        );
    }

    #[test]
    fn webhook_format_env_override_parses_known_values() {
        let key = "APEX_TEST_WEBHOOK_FORMAT";
        std::env::set_var(key, "teams");
        assert_eq!(
            WebhookConfig::format_from_env(key),
            Some(WebhookFormat::Teams)
        );
        std::env::set_var(key, "JSON");
        assert_eq!(
            WebhookConfig::format_from_env(key),
            Some(WebhookFormat::Json)
        );
        std::env::set_var(key, " Slack ");
        assert_eq!(
            WebhookConfig::format_from_env(key),
            Some(WebhookFormat::Slack)
        );
        std::env::set_var(key, "pager");
        assert_eq!(WebhookConfig::format_from_env(key), None);
        std::env::remove_var(key);
        assert_eq!(WebhookConfig::format_from_env(key), None);
    }

    #[test]
    fn webhook_format_defaults_and_env_parsing() {
        assert_eq!(WebhookFormat::default(), WebhookFormat::Json);
        assert_eq!(WebhookConfig::slack("u").format, WebhookFormat::Slack);
        assert_eq!(
            WebhookConfig::critical_only("u", "critical-hook").format,
            WebhookFormat::Json
        );
    }

    #[test]
    fn redact_url_hides_the_credential_path() {
        let redacted = redact_url("https://hooks.slack.com/services/T00/B00/supersecret1234");
        assert_eq!(redacted, "https://hooks.slack.com/…1234");
        assert!(!redacted.contains("supersecret"));
        assert_eq!(redact_url(""), "");
    }

    #[test]
    fn webhook_config_debug_never_prints_secrets() {
        let mut webhook =
            WebhookConfig::slack("https://hooks.slack.com/services/T00/B00/supersecret1234");
        webhook.bearer_token = Some("bearer-secret-token".to_string());
        let rendered = format!("{webhook:?}");
        assert!(!rendered.contains("supersecret1234"));
        assert!(!rendered.contains("bearer-secret-token"));
        assert!(rendered.contains("<redacted>"));
    }

    // ── SlaEnforcer tests ─────────────────────────────────────────────────────

    fn make_warning(severity: &str, age_seconds: i64, acknowledged: bool) -> SlaWarningRecord {
        SlaWarningRecord {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Test Warning".into(),
            severity: severity.to_string(),
            warning_type: "test_type".into(),
            entity_ids: Vec::new(),
            is_system_broadcast: false,
            created_at: Utc::now() - chrono::Duration::seconds(age_seconds),
            acknowledged,
        }
    }

    #[test]
    fn sla_windows_default_values() {
        let w = SeveritySlaConfig::default();
        assert_eq!(w.deadline_seconds("critical"), 900);
        assert_eq!(w.deadline_seconds("high"), 3600);
        assert_eq!(w.deadline_seconds("medium"), 14400);
        assert_eq!(w.deadline_seconds("low"), 86400);
    }

    #[test]
    fn sla_windows_case_insensitive() {
        let w = SeveritySlaConfig::default();
        assert_eq!(w.deadline_seconds("CRITICAL"), 900);
        assert_eq!(w.deadline_seconds("P0"), 900);
        assert_eq!(w.deadline_seconds("P1"), 3600);
    }

    #[test]
    fn sla_breach_detected_after_deadline() {
        let windows = SeveritySlaConfig {
            critical_seconds: 60,
            ..Default::default()
        };
        // Warning is 120s old with a 60s window — should be breached
        let record = make_warning("critical", 120, false);
        assert!(record.is_sla_breached(&windows));
    }

    #[test]
    fn sla_not_breached_within_window() {
        let windows = SeveritySlaConfig {
            critical_seconds: 3600,
            ..Default::default()
        };
        let record = make_warning("critical", 10, false);
        assert!(!record.is_sla_breached(&windows));
    }

    #[test]
    fn acknowledged_warning_never_breached() {
        let windows = SeveritySlaConfig {
            critical_seconds: 0,
            ..Default::default()
        };
        let record = make_warning("critical", 9999, /* acknowledged = */ true);
        assert!(!record.is_sla_breached(&windows));
    }

    #[test]
    fn sla_enforcer_emits_alert_for_breach() {
        let windows = SeveritySlaConfig {
            critical_seconds: 10,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        let records = vec![make_warning("critical", 60, false)];
        let alerts = enforcer.check_sla_violations(&records);
        assert!(
            !alerts.is_empty(),
            "Should produce escalation for SLA breach"
        );
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert!(alerts[0].title.contains("SLA BREACH"));
        assert_eq!(alerts[0].category, "sla_breach");
    }

    #[test]
    fn sla_enforcer_escalates_low_to_high() {
        let windows = SeveritySlaConfig {
            low_seconds: 10,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        // Low severity breached — should be escalated to High
        let records = vec![make_warning("low", 60, false)];
        let alerts = enforcer.check_sla_violations(&records);
        assert!(!alerts.is_empty());
        assert_eq!(
            alerts[0].severity,
            AlertSeverity::High,
            "Low severity should escalate to High on SLA breach"
        );
    }

    #[test]
    fn sla_enforcer_skips_acknowledged() {
        let windows = SeveritySlaConfig {
            critical_seconds: 0,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        let records = vec![make_warning("critical", 99999, true)];
        let alerts = enforcer.check_sla_violations(&records);
        assert!(
            alerts.is_empty(),
            "Acknowledged warnings should not trigger SLA breach"
        );
    }

    #[test]
    fn sla_enforcer_approaching_sla() {
        let windows = SeveritySlaConfig {
            high_seconds: 3600,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        // Warning that's 3300s old — 300s remaining — within 600s warn-ahead
        let record = make_warning("high", 3300, false);
        let record_cloned = record.clone();
        let record_arr = [record_cloned];
        let approaching = enforcer.approaching_sla(&record_arr, 600);
        assert!(
            !approaching.is_empty(),
            "Should detect warning approaching SLA"
        );

        // Warning that's only 10s old — far from deadline
        let fresh = make_warning("high", 10, false);
        let fresh_arr = [fresh];
        let not_approaching = enforcer.approaching_sla(&fresh_arr, 600);
        assert!(
            not_approaching.is_empty(),
            "Fresh warning should not be flagged"
        );
    }

    #[test]
    fn sla_alert_preserves_the_full_entity_set() {
        let windows = SeveritySlaConfig {
            critical_seconds: 10,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        let mut record = make_warning("critical", 60, false);
        record.entity_ids = vec![
            "11111111-1111-4111-8111-111111111111".into(),
            "22222222-2222-4222-8222-222222222222".into(),
        ];

        let alert = enforcer.build_breach_alert(&record).expect("breach alert");
        assert_eq!(
            alert.scope.entity_ids(),
            vec![
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222"
            ],
            "every entity on the warning must survive into the alert, not just entity_ids[1]"
        );
    }

    #[test]
    fn sla_alert_without_entities_reaches_nobody_not_everyone() {
        let windows = SeveritySlaConfig {
            critical_seconds: 10,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        let record = make_warning("critical", 60, false);

        let alert = enforcer.build_breach_alert(&record).expect("breach alert");
        // An unscoped warning (no entities, not flagged as a broadcast) must
        // address nobody. Inferring SystemBroadcast here would page every
        // connected user and bypass per-user preferences.
        assert_eq!(
            alert.scope,
            AlertScope::Users {
                user_ids: Vec::new()
            }
        );
        assert_eq!(
            alert.scope.audience(),
            apex_core::alert_config::AlertAudience::Users(Vec::new())
        );
        assert!(
            !serde_json::to_string(&alert)
                .expect("serialize")
                .contains("\"system\""),
            "system alerts must not fabricate a `system` entity id"
        );
    }

    #[test]
    fn sla_alert_for_a_flagged_system_broadcast_stays_a_broadcast() {
        let windows = SeveritySlaConfig {
            critical_seconds: 10,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        let mut record = make_warning("critical", 60, false);
        record.is_system_broadcast = true;

        let alert = enforcer.build_breach_alert(&record).expect("breach alert");
        assert_eq!(alert.scope, AlertScope::SystemBroadcast);
        assert_eq!(
            alert.scope.audience(),
            apex_core::alert_config::AlertAudience::Broadcast
        );
    }

    #[test]
    fn legacy_pending_alert_payloads_stay_replayable() {
        let legacy_system = serde_json::json!({
            "source_id": "sla-breach:warning-1",
            "entity_id": "system",
            "entity_name": "SLA Enforcement",
            "title": "SLA BREACH",
            "body": "overdue",
            "severity": "high",
            "priority_score": 0.95,
            "category": "sla_breach",
            "region": null,
            "created_at": "2026-01-01T00:00:00Z",
            "llm_narrative": null
        });
        let alert: PendingAlert =
            serde_json::from_value(legacy_system).expect("legacy system alert deserializes");
        // Legacy entity-less SLA alerts resolved to nobody; replay must keep
        // that audience instead of upgrading them into a broadcast.
        assert_eq!(
            alert.scope,
            AlertScope::Users {
                user_ids: Vec::new()
            }
        );

        let legacy_entity = serde_json::json!({
            "source_id": "warning-2",
            "entity_id": "entity-9",
            "entity_name": "Acme Corp",
            "title": "Warning",
            "body": "body",
            "severity": "medium",
            "priority_score": 0.5,
            "category": "warning",
            "region": "EU",
            "created_at": "2026-01-01T00:00:00Z",
            "llm_narrative": null
        });
        let alert: PendingAlert =
            serde_json::from_value(legacy_entity).expect("legacy entity alert deserializes");
        assert_eq!(alert.display_name(), "Acme Corp");
    }

    #[test]
    fn sla_enforcer_exposes_the_same_windows_it_enforces() {
        let windows = SeveritySlaConfig {
            high_seconds: 123,
            ..Default::default()
        };
        let enforcer = SlaEnforcer::new(windows);
        assert_eq!(enforcer.windows().deadline_seconds("high"), 123);
    }

    #[test]
    fn malformed_sla_window_is_a_configuration_error() {
        use std::sync::{LazyLock, Mutex};
        static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let name = "SLA_CRITICAL_SECONDS";
        std::env::set_var(name, "tomorrow");
        let result = shared_sla_config_from_env();
        std::env::remove_var(name);

        let errors = result.expect_err("'tomorrow' is not a number of seconds");
        assert_eq!(errors.errors[0].variable, name);
    }
}
