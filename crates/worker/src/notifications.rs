//! Real-time alert notification dispatch system.
//!
//! Sends structured alert notifications over multiple channels when insight
//! cards breach severity / priority thresholds.
//!
//! # Supported channels
//! - **Webhook** — generic HTTP POST (JSON payload); used for Slack, Teams, etc.
//! - **Email** — SMTP via external relay (prepared but not sent here; caller
//!   injects an email sender)
//! - **Log** — structured tracing emit; always active for audit purposes
//!
//! # Design
//! `NotificationDispatcher::dispatch` is the main entry point.  It accepts a
//! slice of `InsightCard`-derived `PendingAlert` items, filters them through
//! configured thresholds, and fires one `Notification` per channel per alert.
//!
//! LLM-enhanced alert bodies are available when the `llm` feature is active.

use anyhow::{Context, Result};
use apex_core::sla::SeveritySlaConfig;
use chrono::{DateTime, Utc};
use lettre::message::{header::ContentType, Mailbox, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

// ─────────────────────────────────────────────────────────────────────────────
// Alert data types
// ─────────────────────────────────────────────────────────────────────────────

/// Severity of an outgoing alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl AlertSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Parse from string representation (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }
}

/// A pending alert derived from an `InsightCard` or a pipeline event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAlert {
    /// Originating insight / event id.
    pub source_id: String,
    pub entity_id: String,
    pub entity_name: String,
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

impl PendingAlert {
    pub fn new(
        source_id: impl Into<String>,
        entity_id: impl Into<String>,
        entity_name: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        severity: AlertSeverity,
        priority_score: f64,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            entity_id: entity_id.into(),
            entity_name: entity_name.into(),
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

impl Notification {
    fn new(channel: &str, alert: PendingAlert, formatted_body: String) -> Self {
        Self {
            id: format!("{}:{}", channel, alert.source_id),
            channel: channel.to_string(),
            destination: None,
            alert,
            subject: None,
            formatted_body,
            dispatched_at: None,
            dispatch_success: None,
            error_message: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Channel configuration
// ─────────────────────────────────────────────────────────────────────────────

/// A webhook endpoint destination (Slack, Teams, generic HTTP).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub name: String,
    pub url: String,
    /// Optional `Authorization: Bearer <token>` header.
    pub bearer_token: Option<String>,
    /// Minimum severity to send over this webhook.
    pub min_severity: AlertSeverity,
    /// Minimum priority_score [0, 1] to send over this webhook.
    pub min_priority: f64,
}

impl WebhookConfig {
    pub fn slack(url: impl Into<String>) -> Self {
        Self {
            name: "slack".into(),
            url: url.into(),
            bearer_token: None,
            min_severity: AlertSeverity::Medium,
            min_priority: 0.5,
        }
    }

    pub fn critical_only(url: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            bearer_token: None,
            min_severity: AlertSeverity::Critical,
            min_priority: 0.8,
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

impl Default for AlertSeverity {
    fn default() -> Self {
        Self::Info
    }
}

impl NotificationConfig {
    /// Create from environment variables.
    ///
    /// Reads:
    /// - `SLACK_WEBHOOK_URL`
    /// - `CRITICAL_WEBHOOK_URL` (optional extra endpoint)
    /// - `ALERT_EMAIL_TO` (comma-separated list)
    /// - `ALERT_EMAIL_FROM`
    pub fn from_env() -> Self {
        let mut cfg = NotificationConfig::default();
        cfg.log_min_severity = AlertSeverity::Low;

        if let Ok(url) = std::env::var("SLACK_WEBHOOK_URL") {
            cfg.webhooks.push(WebhookConfig::slack(url));
        }
        if let Ok(url) = std::env::var("CRITICAL_WEBHOOK_URL") {
            cfg.webhooks
                .push(WebhookConfig::critical_only(url, "critical-hook"));
        }
        if let Ok(to_raw) = std::env::var("ALERT_EMAIL_TO") {
            let to_addresses: Vec<String> =
                to_raw.split(',').map(|s| s.trim().to_string()).collect();
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

        cfg
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher
// ─────────────────────────────────────────────────────────────────────────────

/// Dispatches alerts to all configured channels.
pub struct NotificationDispatcher {
    config: NotificationConfig,
    http: reqwest::Client,
}

impl NotificationDispatcher {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    pub fn from_env() -> Self {
        Self::new(NotificationConfig::from_env())
    }

    /// Dispatch a batch of pending alerts; returns all `Notification` records
    /// (dispatched and skipped) for audit logging.
    pub async fn dispatch_batch(&self, alerts: Vec<PendingAlert>) -> Vec<Notification> {
        let mut records = Vec::new();

        for alert in alerts {
            // Always log
            if alert.severity >= self.config.log_min_severity {
                info!(
                    entity = %alert.entity_name,
                    title = %alert.title,
                    severity = %alert.severity.as_str(),
                    priority = %alert.priority_score,
                    "🔔 Alert dispatched"
                );
            }

            // Fire webhooks
            for webhook in &self.config.webhooks {
                if alert.severity < webhook.min_severity
                    || alert.priority_score < webhook.min_priority
                {
                    debug!(webhook = %webhook.name, "Alert below webhook threshold — skipping");
                    continue;
                }

                let body = self.format_slack_message(&alert);
                let mut notif = Notification::new(&webhook.name, alert.clone(), body.clone());
                notif.destination = Some(webhook.url.clone());

                match self.send_webhook_with_retry(webhook, &body).await {
                    Ok(_) => {
                        notif.dispatched_at = Some(Utc::now());
                        notif.dispatch_success = Some(true);
                        debug!(webhook = %webhook.name, "Webhook delivered");
                    }
                    Err(e) => {
                        error!(webhook = %webhook.name, error = %e, "Webhook delivery failed");
                        notif.dispatch_success = Some(false);
                        notif.error_message = Some(e.to_string());
                    }
                }
                records.push(notif);
            }

            // Deliver alert emails through the same retrying dispatcher path.
            if let Some(ref email_cfg) = self.config.email {
                if alert.severity >= email_cfg.min_severity
                    && alert.priority_score >= email_cfg.min_priority
                {
                    let subject = format!(
                        "{} [{}] {}",
                        email_cfg.subject_prefix,
                        alert.severity.as_str().to_uppercase(),
                        alert.title
                    );
                    let formatted = self.format_email_body(&alert);
                    let mut notif = Notification::new("email", alert.clone(), formatted.clone());
                    notif.destination = Some(email_cfg.to_addresses.join(","));
                    notif.subject = Some(subject.clone());

                    match self
                        .send_email_with_retry(email_cfg, &subject, &formatted)
                        .await
                    {
                        Ok(_) => {
                            notif.dispatched_at = Some(Utc::now());
                            notif.dispatch_success = Some(true);
                        }
                        Err(e) => {
                            error!(error = %e, "Email delivery failed");
                            notif.dispatch_success = Some(false);
                            notif.error_message = Some(e.to_string());
                        }
                    }
                    records.push(notif);
                }
            }
        }

        records
    }

    async fn send_webhook(&self, cfg: &WebhookConfig, body: &str) -> Result<()> {
        let mut req = self
            .http
            .post(&cfg.url)
            .header("Content-Type", "application/json")
            .body(body.to_string());

        if let Some(ref token) = cfg.bearer_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let resp = req.send().await.context("Webhook HTTP request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Webhook returned {status}: {text}");
        }

        Ok(())
    }

    async fn send_webhook_with_retry(&self, cfg: &WebhookConfig, body: &str) -> Result<()> {
        let mut last_error = None;
        for attempt in 1..=3 {
            match self.send_webhook(cfg, body).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < 3 {
                        tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("webhook delivery failed")))
    }

    async fn send_email(&self, cfg: &EmailConfig, subject: &str, text_body: &str) -> Result<()> {
        let smtp_host =
            std::env::var("ALERT_SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let smtp_port = std::env::var("ALERT_SMTP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(25);
        let smtp_user = std::env::var("ALERT_SMTP_USER").unwrap_or_default();
        let smtp_pass = std::env::var("ALERT_SMTP_PASS").unwrap_or_default();
        let smtp_starttls = std::env::var("ALERT_SMTP_STARTTLS")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);

        let mut builder = Message::builder()
            .from(cfg.from_address.parse::<Mailbox>()?)
            .subject(subject);
        for to in &cfg.to_addresses {
            builder = builder.to(to.parse::<Mailbox>()?);
        }

        let email = builder.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(text_body.to_string()),
        )?;

        let mailer = if smtp_starttls {
            let mut transport =
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)?.port(smtp_port);
            if !smtp_user.trim().is_empty() {
                transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
            }
            transport.build()
        } else {
            let mut transport =
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp_host).port(smtp_port);
            if !smtp_user.trim().is_empty() {
                transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
            }
            transport.build()
        };

        mailer.send(email).await?;
        Ok(())
    }

    async fn send_email_with_retry(
        &self,
        cfg: &EmailConfig,
        subject: &str,
        text_body: &str,
    ) -> Result<()> {
        let mut last_error = None;
        for attempt in 1..=3 {
            match self.send_email(cfg, subject, text_body).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < 3 {
                        tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("email delivery failed")))
    }

    // ── Formatters ────────────────────────────────────────────────────────────

    fn format_slack_message(&self, alert: &PendingAlert) -> String {
        let emoji = match alert.severity {
            AlertSeverity::Critical => "🚨",
            AlertSeverity::High => "🔴",
            AlertSeverity::Medium => "🟡",
            AlertSeverity::Low => "🔵",
            AlertSeverity::Info => "ℹ️",
        };

        let body_text = alert.llm_narrative.as_deref().unwrap_or(&alert.body);
        let region = alert.region.as_deref().unwrap_or("Global");

        serde_json::json!({
            "text": format!("{emoji} *[{}] {}*\n", alert.severity.as_str().to_uppercase(), alert.title),
            "blocks": [
                {
                    "type": "header",
                    "text": {
                        "type": "plain_text",
                        "text": format!("{emoji} {} — {}", alert.severity.as_str().to_uppercase(), alert.title)
                    }
                },
                {
                    "type": "section",
                    "fields": [
                        { "type": "mrkdwn", "text": format!("*Entity:*\n{}", alert.entity_name) },
                        { "type": "mrkdwn", "text": format!("*Region:*\n{}", region) },
                        { "type": "mrkdwn", "text": format!("*Category:*\n{}", alert.category) },
                        { "type": "mrkdwn", "text": format!("*Priority Score:*\n{:.2}", alert.priority_score) },
                    ]
                },
                {
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": body_text }
                },
                {
                    "type": "context",
                    "elements": [
                        { "type": "mrkdwn", "text": format!("Source ID: `{}` | {}", alert.source_id, alert.created_at.format("%Y-%m-%d %H:%M UTC")) }
                    ]
                }
            ]
        })
        .to_string()
    }

    fn format_email_body(&self, alert: &PendingAlert) -> String {
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
            alert.entity_name,
            alert.category,
            alert.region.as_deref().unwrap_or("Global"),
            alert.priority_score,
            body_text,
            alert.source_id,
            alert.created_at.format("%Y-%m-%d %H:%M UTC"),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SLA enforcement
// ─────────────────────────────────────────────────────────────────────────────

fn shared_sla_config_from_env() -> SeveritySlaConfig {
    fn parse_env(key: &str, default: i64) -> i64 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    let defaults = SeveritySlaConfig::default();
    SeveritySlaConfig {
        critical_seconds: parse_env("SLA_CRITICAL_SECONDS", defaults.critical_seconds),
        high_seconds: parse_env("SLA_HIGH_SECONDS", defaults.high_seconds),
        medium_seconds: parse_env("SLA_MEDIUM_SECONDS", defaults.medium_seconds),
        low_seconds: parse_env("SLA_LOW_SECONDS", defaults.low_seconds),
    }
}

/// A warning record fetched from the database for SLA evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaWarningRecord {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub warning_type: String,
    pub entity_id: Option<String>,
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
/// 3. Passing the returned `Vec<PendingAlert>` to `NotificationDispatcher::dispatch_batch()`.
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

    pub fn from_env() -> Self {
        Self::new(shared_sla_config_from_env())
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
            record.entity_id.clone().unwrap_or_else(|| "system".into()),
            "SLA Enforcement",
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
            record.entity_id.clone().unwrap_or_else(|| "system".into()),
            "SLA Enforcement",
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
    use super::*;

    fn make_alert(severity: AlertSeverity, priority: f64) -> PendingAlert {
        PendingAlert::new(
            "src-001",
            "entity-1",
            "Test Corp",
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
        let cfg = NotificationConfig::default();
        let dispatcher = NotificationDispatcher::new(cfg);
        let alert = make_alert(AlertSeverity::Critical, 0.95);
        let msg = dispatcher.format_slack_message(&alert);
        let _parsed: serde_json::Value =
            serde_json::from_str(&msg).expect("slack message should be valid JSON");
    }

    #[test]
    fn email_body_contains_entity_name() {
        let cfg = NotificationConfig::default();
        let dispatcher = NotificationDispatcher::new(cfg);
        let alert = make_alert(AlertSeverity::High, 0.8);
        let body = dispatcher.format_email_body(&alert);
        assert!(body.contains("Test Corp"));
        assert!(body.contains("HIGH"));
    }

    #[tokio::test]
    async fn dispatch_batch_empty_returns_empty() {
        let dispatcher = NotificationDispatcher::new(NotificationConfig::default());
        let records = dispatcher.dispatch_batch(vec![]).await;
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn dispatch_batch_below_threshold_skips_webhook() {
        let mut cfg = NotificationConfig::default();
        cfg.webhooks.push(WebhookConfig {
            name: "test".into(),
            url: "https://httpbin.org/post".into(),
            bearer_token: None,
            min_severity: AlertSeverity::Critical,
            min_priority: 0.9,
        });

        let dispatcher = NotificationDispatcher::new(cfg);
        // Send low severity alert — should be filtered
        let alert = make_alert(AlertSeverity::Low, 0.1);
        let records = dispatcher.dispatch_batch(vec![alert]).await;
        // No webhook dispatch attempted (but no email config either)
        assert!(records.is_empty());
    }

    // ── SlaEnforcer tests ─────────────────────────────────────────────────────

    fn make_warning(severity: &str, age_seconds: i64, acknowledged: bool) -> SlaWarningRecord {
        SlaWarningRecord {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Test Warning".into(),
            severity: severity.to_string(),
            warning_type: "test_type".into(),
            entity_id: None,
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
}
