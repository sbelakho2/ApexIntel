//! Webhook notification dispatcher for Slack, Microsoft Teams, and generic HTTP.
//!
//! Extends the existing `notifications.rs` with external webhook delivery.
//! This module handles the outbound HTTP calls; the existing notification
//! pipeline handles filtering, throttling, and dedup before calling us.
//!
//! # Integration
//! ```ignore
//! let dispatcher = WebhookDispatcher::new(config)?;
//! dispatcher.dispatch(&alert).await?;
//! ```

use anyhow::Result;
use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use tracing::{info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Webhook endpoints and credentials.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub slack_url: Option<String>,
    pub teams_url: Option<String>,
    pub generic_webhook_urls: Vec<String>,
    pub email_recipients: Vec<String>,
    pub sms_recipients: Vec<String>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_pass: String,
    pub from_address: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            slack_url: None,
            teams_url: None,
            generic_webhook_urls: Vec::new(),
            email_recipients: Vec::new(),
            sms_recipients: Vec::new(),
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_user: String::new(),
            smtp_pass: String::new(),
            from_address: "alerts@apexintel.io".into(),
        }
    }
}

impl WebhookConfig {
    /// Build from environment variables.
    pub fn from_env() -> Self {
        Self {
            slack_url: std::env::var("SLACK_WEBHOOK_URL").ok(),
            teams_url: std::env::var("TEAMS_WEBHOOK_URL").ok(),
            generic_webhook_urls: std::env::var("GENERIC_WEBHOOK_URLS")
                .map(|s| s.split(',').map(|u| u.trim().to_string()).collect())
                .unwrap_or_default(),
            email_recipients: std::env::var("ALERT_EMAIL_RECIPIENTS")
                .map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
                .unwrap_or_default(),
            sms_recipients: Vec::new(),
            smtp_host: std::env::var("SMTP_HOST").unwrap_or_default(),
            smtp_port: std::env::var("SMTP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(587),
            smtp_user: std::env::var("SMTP_USER").unwrap_or_default(),
            smtp_pass: std::env::var("SMTP_PASS").unwrap_or_default(),
            from_address: std::env::var("ALERT_FROM_ADDRESS")
                .unwrap_or_else(|_| "alerts@apexintel.io".into()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert payload
// ─────────────────────────────────────────────────────────────────────────────

/// Structured alert payload sent to all webhook channels.
#[derive(Debug, Serialize, Clone)]
pub struct AlertPayload {
    pub severity: String,
    pub title: String,
    pub description: String,
    pub entity: String,
    pub region: String,
    pub recipe_id: String,
    pub confidence: f64,
    pub actions: Vec<String>,
    pub dashboard_url: String,
    pub timestamp: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher
// ─────────────────────────────────────────────────────────────────────────────

pub struct WebhookDispatcher {
    client: Client,
    config: WebhookConfig,
}

impl WebhookDispatcher {
    pub fn new(config: WebhookConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build webhook HTTP client: {e}"))?;
        Ok(Self { client, config })
    }

    /// Dispatch an alert to all configured channels.
    pub async fn dispatch(&self, alert: &AlertPayload) -> Result<()> {
        let mut errors = Vec::new();

        // Slack
        if let Some(ref url) = self.config.slack_url {
            if let Err(e) = self.send_slack(url, alert).await {
                errors.push(format!("Slack: {}", e));
            } else {
                info!(severity = %alert.severity, "Slack alert sent");
            }
        }

        // Microsoft Teams
        if let Some(ref url) = self.config.teams_url {
            if let Err(e) = self.send_teams(url, alert).await {
                errors.push(format!("Teams: {}", e));
            } else {
                info!(severity = %alert.severity, "Teams alert sent");
            }
        }

        // Generic webhooks
        for url in &self.config.generic_webhook_urls {
            if let Err(e) = self.send_generic(url, alert).await {
                errors.push(format!("Webhook {}: {}", url, e));
            }
        }

        if !errors.is_empty() {
            warn!("Webhook dispatch partial failure: {:?}", errors);
        }
        Ok(())
    }

    // ── Slack ────────────────────────────────────────────────

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn send_slack(&self, url: &str, alert: &AlertPayload) -> Result<()> {
        let color = severity_color(&alert.severity);
        let payload = serde_json::json!({
            "attachments": [{
                "color": color,
                "title": format!("[{}] {}", alert.severity.to_uppercase(), alert.title),
                "text": alert.description,
                "fields": [
                    { "title": "Entity", "value": &alert.entity, "short": true },
                    { "title": "Region", "value": &alert.region, "short": true },
                    { "title": "Confidence", "value": format!("{:.0}%", alert.confidence * 100.0), "short": true },
                    { "title": "Recipe", "value": &alert.recipe_id, "short": true },
                ],
                "actions": [{
                    "type": "button",
                    "text": "View in Dashboard",
                    "url": &alert.dashboard_url,
                }],
                "ts": chrono::Utc::now().timestamp()
            }]
        });
        self.client
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    // ── Microsoft Teams ─────────────────────────────────────

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    async fn send_teams(&self, url: &str, alert: &AlertPayload) -> Result<()> {
        let theme_color = severity_color_hex(&alert.severity);
        let payload = serde_json::json!({
            "@type": "MessageCard",
            "@context": "http://schema.org/extensions",
            "themeColor": theme_color,
            "summary": &alert.title,
            "sections": [{
                "activityTitle": format!("[{}] {}", alert.severity.to_uppercase(), alert.title),
                "activitySubtitle": &alert.region,
                "text": &alert.description,
                "facts": [
                    { "name": "Entity", "value": &alert.entity },
                    { "name": "Confidence", "value": format!("{:.0}%", alert.confidence * 100.0) },
                    { "name": "Recipe", "value": &alert.recipe_id },
                ],
            }],
            "potentialAction": [{
                "@type": "OpenUri",
                "name": "View Dashboard",
                "targets": [{ "os": "default", "uri": &alert.dashboard_url }],
            }]
        });
        self.client
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    // ── Generic webhook ─────────────────────────────────────

    async fn send_generic(&self, url: &str, alert: &AlertPayload) -> Result<()> {
        self.client
            .post(url)
            .json(alert)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn severity_color(severity: &str) -> &'static str {
    match severity {
        "critical" => "#FF0000",
        "high" => "#FF8C00",
        "medium" => "#FFD700",
        "low" => "#36A64F",
        _ => "#808080",
    }
}

fn severity_color_hex(severity: &str) -> &'static str {
    match severity {
        "critical" => "FF0000",
        "high" => "FF8C00",
        "medium" => "FFD700",
        "low" => "36A64F",
        _ => "808080",
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

    #[test]
    fn webhook_config_default() {
        let cfg = WebhookConfig::default();
        assert!(cfg.slack_url.is_none());
        assert!(cfg.teams_url.is_none());
        assert_eq!(cfg.smtp_port, 587);
    }

    #[test]
    fn alert_payload_serializes() {
        let alert = AlertPayload {
            severity: "high".into(),
            title: "Test Alert".into(),
            description: "Something happened".into(),
            entity: "CompanyX".into(),
            region: "TN".into(),
            recipe_id: "R-001".into(),
            confidence: 0.85,
            actions: vec!["investigate".into()],
            dashboard_url: "https://apexintel.io/warnings/123".into(),
            timestamp: "2026-03-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("\"severity\":\"high\""));
        assert!(json.contains("CompanyX"));
    }

    #[test]
    fn severity_colors_valid() {
        assert_eq!(severity_color("critical"), "#FF0000");
        assert_eq!(severity_color("high"), "#FF8C00");
        assert_eq!(severity_color("low"), "#36A64F");
        assert_eq!(severity_color("unknown"), "#808080");
    }

    #[test]
    fn dispatcher_builds() {
        let cfg = WebhookConfig::default();
        let _ = WebhookDispatcher::new(cfg).expect("should build in tests");
    }
}
