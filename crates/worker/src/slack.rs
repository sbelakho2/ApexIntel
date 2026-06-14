//! Slack webhook integration for ApexIntel alerting.
//!
//! Builds richly formatted Slack Block Kit messages from alert/warning/insight
//! data and delivers them via Slack Incoming Webhooks with retry logic, timeout
//! handling, and rate limiting.
//!
//! # Architecture
//!
//! - [`SlackMessage`] — builds Block Kit payloads from domain data
//! - [`SlackWebhook`] — sends messages over HTTP with retry + rate limiting
//! - [`SlackConfig`] — parses webhook URLs from env vars and YAML config
//!
//! # Integration
//!
//! ```ignore
//! let config = SlackConfig::from_env();
//! let webhook = SlackWebhook::new(&config)?;
//! let msg = SlackMessage::alert("critical", "Threat detected", "...")
//!     .with_entity("Acme Corp")
//!     .with_source_url("https://apexintel.io/warnings/123");
//! webhook.send(&msg).await?;
//! ```

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Severity
// ─────────────────────────────────────────────────────────────────────────────

/// Severity level for Slack messages, with corresponding emoji and color.
/// Severity level ordering: `Info < Low < Medium < High < Critical`.
///
/// The derived `PartialOrd` / `Ord` uses declaration order (first = least), so
/// variants are declared from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlackMessageSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl SlackMessageSeverity {
    /// Return the emoji glyph for this severity.
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Critical => "🔴",
            Self::High => "🟠",
            Self::Medium => "🟡",
            Self::Low => "⚪",
            Self::Info => "ℹ️",
        }
    }

    /// Return the hex color for Slack attachment sidebar.
    pub fn color_hex(self) -> &'static str {
        match self {
            Self::Critical => "FF0000",
            Self::High => "FF8C00",
            Self::Medium => "FFD700",
            Self::Low => "36A64F",
            Self::Info => "808080",
        }
    }

    /// Return the hex color with `#` prefix for Slack attachment sidebar.
    pub fn color(self) -> &'static str {
        match self {
            Self::Critical => "#FF0000",
            Self::High => "#FF8C00",
            Self::Medium => "#FFD700",
            Self::Low => "#36A64F",
            Self::Info => "#808080",
        }
    }

    /// Parse from a string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }

    /// Convert to a static string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert type routing
// ─────────────────────────────────────────────────────────────────────────────

/// High-level alert type for per-channel routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertType {
    /// Security-critical alerts (breaches, vulnerabilities, etc.)
    Security,
    /// Competitive intelligence insights
    Insight,
    /// Recipe match / opportunity alerts
    RecipeMatch,
    /// Person-of-interest updates
    PoiUpdate,
    /// Warning verification results
    Warning,
    /// General / uncategorized
    General,
}

impl AlertType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Insight => "insight",
            Self::RecipeMatch => "recipe_match",
            Self::PoiUpdate => "poi_update",
            Self::Warning => "warning",
            Self::General => "general",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "security" => Self::Security,
            "insight" => Self::Insight,
            "recipe_match" | "recipe" => Self::RecipeMatch,
            "poi_update" | "poi" => Self::PoiUpdate,
            "warning" => Self::Warning,
            _ => Self::General,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slack Message
// ─────────────────────────────────────────────────────────────────────────────

/// A richly formatted Slack message built with Block Kit.
///
/// Use the builder-style methods to construct a message, then call
/// [`to_blocks`](Self::to_blocks) to get the serializable payload.
#[derive(Debug, Clone)]
pub struct SlackMessage {
    /// Severity level (drives emoji and color).
    pub severity: SlackMessageSeverity,
    /// Alert type for channel routing.
    pub alert_type: AlertType,
    /// Message title (shown in header block).
    pub title: String,
    /// Message description / body text.
    pub description: String,
    /// Optional entity name (company, person, etc.).
    pub entity: Option<String>,
    /// Optional source URL (link back to ApexIntel).
    pub source_url: Option<String>,
    /// Optional region/timestamp fields.
    pub region: Option<String>,
    pub timestamp: Option<String>,
    /// Optional extra fields for display (key-value pairs).
    pub fields: Vec<(String, String)>,
    /// Whether to include action buttons.
    pub include_actions: bool,
}

impl SlackMessage {
    /// Create a new Slack message with the given severity, title, and description.
    pub fn new(
        severity: SlackMessageSeverity,
        alert_type: AlertType,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            alert_type,
            title: title.into(),
            description: description.into(),
            entity: None,
            source_url: None,
            region: None,
            timestamp: None,
            fields: Vec::new(),
            include_actions: true,
        }
    }

    /// Convenience constructor for alerts.
    pub fn alert(
        severity: impl Into<SlackMessageSeverity>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let severity: SlackMessageSeverity = severity.into();
        Self::new(
            severity,
            AlertType::General,
            title,
            description,
        )
    }

    /// Set the entity name associated with this message.
    pub fn with_entity(mut self, entity: impl Into<String>) -> Self {
        self.entity = Some(entity.into());
        self
    }

    /// Set the source URL for the "View in ApexIntel" action button.
    pub fn with_source_url(mut self, url: impl Into<String>) -> Self {
        self.source_url = Some(url.into());
        self
    }

    /// Set the region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set the timestamp.
    pub fn with_timestamp(mut self, ts: impl Into<String>) -> Self {
        self.timestamp = Some(ts.into());
        self
    }

    /// Add an extra field to display.
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((key.into(), value.into()));
        self
    }

    /// Add multiple extra fields.
    pub fn with_fields<I, K, V>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in fields {
            self.fields.push((k.into(), v.into()));
        }
        self
    }

    /// Enable or disable action buttons (default: enabled).
    pub fn with_actions(mut self, include: bool) -> Self {
        self.include_actions = include;
        self
    }

    /// Build the Slack Block Kit JSON payload.
    ///
    /// Returns a [`serde_json::Value`] suitable for serialization and POSTing
    /// to a Slack Incoming Webhook.
    pub fn to_blocks(&self) -> serde_json::Value {
        let emoji = self.severity.emoji();
        let severity_upper = self.severity.as_str().to_uppercase();

        // ── Header block ──────────────────────────────────────────────
        let header_text = format!("{} {} — {}", emoji, severity_upper, self.title);
        let mut blocks: Vec<serde_json::Value> = vec![
            serde_json::json!({
                "type": "header",
                "text": {
                    "type": "plain_text",
                    "text": header_text,
                    "emoji": true
                }
            }),
        ];

        // ── Entity / context section ──────────────────────────────────
        let mut context_fields: Vec<serde_json::Value> = Vec::new();
        if let Some(ref entity) = self.entity {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": format!("*Entity:*\n{}", entity)
            }));
        }
        if let Some(ref region) = self.region {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": format!("*Region:*\n{}", region)
            }));
        }
        if let Some(ref ts) = self.timestamp {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": format!("*Timestamp:*\n{}", ts)
            }));
        }

        if !context_fields.is_empty() {
            blocks.push(serde_json::json!({
                "type": "section",
                "fields": context_fields
            }));
        }

        // ── Description section ────────────────────────────────────────
        // Slack Block Kit has a 3000 character limit on mrkdwn text blocks,
        // so truncate if necessary.
        let desc = if self.description.len() > 2900 {
            format!("{}…", &self.description[..2900])
        } else {
            self.description.clone()
        };
        blocks.push(serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": desc
            }
        }));

        // ── Extra fields section ───────────────────────────────────────
        if !self.fields.is_empty() {
            let field_blocks: Vec<serde_json::Value> = self
                .fields
                .iter()
                .map(|(k, v)| {
                    serde_json::json!({
                        "type": "mrkdwn",
                        "text": format!("*{}:*\n{}", k, v)
                    })
                })
                .collect();

            // Slack allows up to 10 fields per section; chunk if needed.
            for chunk in field_blocks.chunks(10) {
                blocks.push(serde_json::json!({
                    "type": "section",
                    "fields": chunk
                }));
            }
        }

        // ── Action buttons ─────────────────────────────────────────────
        if self.include_actions {
            let mut elements: Vec<serde_json::Value> = Vec::new();

            if let Some(ref url) = self.source_url {
                elements.push(serde_json::json!({
                    "type": "button",
                    "text": {
                        "type": "plain_text",
                        "text": "🔍 View in ApexIntel",
                        "emoji": true
                    },
                    "url": url,
                    "action_id": "view_apexintel"
                }));
            }

            elements.push(serde_json::json!({
                "type": "button",
                "text": {
                    "type": "plain_text",
                    "text": "✅ Acknowledge",
                    "emoji": true
                },
                "style": "primary",
                "action_id": "acknowledge_alert",
                "value": "acknowledged"
            }));

            elements.push(serde_json::json!({
                "type": "button",
                "text": {
                    "type": "plain_text",
                    "text": "❌ Dismiss",
                    "emoji": true
                },
                "style": "danger",
                "action_id": "dismiss_alert",
                "value": "dismissed"
            }));

            blocks.push(serde_json::json!({
                "type": "actions",
                "elements": elements
            }));
        }

        // ── Context / footer ──────────────────────────────────────────
        blocks.push(serde_json::json!({
            "type": "context",
            "elements": [
                {
                    "type": "mrkdwn",
                    "text": format!("ApexIntel • {} • `{}`", self.alert_type.as_str(), chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"))
                }
            ]
        }));

        // ── Build the full payload with attachment color ───────────────
        serde_json::json!({
            "text": format!("{} *[{}]* {} — {}", emoji, severity_upper, self.title, self.description.chars().take(120).collect::<String>()),
            "attachments": [
                {
                    "color": self.severity.color(),
                    "blocks": blocks
                }
            ]
        })
    }

    /// Serialize the message to a JSON string suitable for the Slack Webhook API.
    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string(&self.to_blocks()).context("failed to serialize Slack message to JSON")
    }
}

impl From<&str> for SlackMessageSeverity {
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rate limiter
// ─────────────────────────────────────────────────────────────────────────────

/// Per-URL rate limiter to enforce max 1 message per 2 seconds per webhook URL.
#[derive(Debug)]
struct PerUrlRateLimiter {
    last_send: Mutex<HashMap<String, tokio::time::Instant>>,
}

impl PerUrlRateLimiter {
    fn new() -> Self {
        Self {
            last_send: Mutex::new(HashMap::new()),
        }
    }

    /// Wait if necessary to respect the rate limit for the given URL.
    async fn wait_if_needed(&self, url: &str) {
        let mut last_send = self.last_send.lock().await;
        if let Some(last) = last_send.get(url) {
            let elapsed = last.elapsed();
            let min_interval = Duration::from_secs(2);
            if elapsed < min_interval {
                let wait = min_interval - elapsed;
                drop(last_send);
                tokio::time::sleep(wait).await;
                let mut last_send = self.last_send.lock().await;
                last_send.insert(url.to_string(), tokio::time::Instant::now());
                return;
            }
        }
        last_send.insert(url.to_string(), tokio::time::Instant::now());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slack Webhook
// ─────────────────────────────────────────────────────────────────────────────

/// A Slack Incoming Webhook client with retry logic, timeout handling, and
/// per-URL rate limiting.
pub struct SlackWebhook {
    /// Webhook URLs mapped by channel name (e.g., "security", "intel", "general").
    urls: HashMap<String, Vec<String>>,
    /// Default webhook URLs (used when no channel-specific URL matches).
    default_urls: Vec<String>,
    /// Shared HTTP client with timeout.
    client: Client,
    /// Per-URL rate limiter.
    rate_limiter: Arc<PerUrlRateLimiter>,
    /// Timeout for each HTTP request.
    timeout_secs: u64,
}

impl SlackWebhook {
    /// Create a new SlackWebhook from configuration.
    pub fn new(config: &SlackConfig) -> Result<Self> {
        let timeout = config.timeout_secs;
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .build()
            .context("failed to build Slack webhook HTTP client")?;

        Ok(Self {
            urls: config.webhooks.clone(),
            default_urls: config.default_urls.clone(),
            client,
            rate_limiter: Arc::new(PerUrlRateLimiter::new()),
            timeout_secs: timeout,
        })
    }

    /// Send a Slack message to all appropriate webhooks based on alert type.
    ///
    /// The message is routed to channel-specific URLs when available, otherwise
    /// falls back to default URLs.
    pub async fn send(&self, message: &SlackMessage) -> Result<()> {
        let channel_name = message.alert_type.as_str();
        let payload = message.to_json_string()?;

        // Collect all target URLs: channel-specific first, then defaults.
        let mut targets = self
            .urls
            .get(channel_name)
            .cloned()
            .unwrap_or_default();
        targets.extend(self.default_urls.clone());

        if targets.is_empty() {
            warn!(
                alert_type = %channel_name,
                "No Slack webhook URLs configured; message not sent"
            );
            return Ok(());
        }

        let mut errors = Vec::new();
        for url in &targets {
            if let Err(e) = self.send_to_url(url, &payload).await {
                errors.push(format!("{}: {}", url, e));
                error!(
                    url = %url,
                    error = %e,
                    "Failed to send Slack message"
                );
            } else {
                info!(
                    url = %url,
                    alert_type = %channel_name,
                    severity = %message.severity.as_str(),
                    "Slack message delivered"
                );
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Slack delivery partial failure: {}",
                errors.join("; ")
            ))
        }
    }

    /// Send a raw JSON payload to a single webhook URL with retry logic.
    async fn send_to_url(&self, url: &str, payload: &str) -> Result<()> {
        // Enforce rate limit: max 1 message per 2 seconds per URL.
        self.rate_limiter.wait_if_needed(url).await;

        let mut last_error = None;
        for attempt in 1..=3 {
            match self.send_attempt(url, payload).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(
                        url = %url,
                        attempt,
                        error = %e,
                        "Slack webhook attempt failed"
                    );
                    last_error = Some(e);
                    if attempt < 3 {
                        // Exponential backoff: 1s, 2s
                        let backoff = Duration::from_secs(attempt as u64);
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("Slack webhook delivery failed after 3 retries")
        }))
    }

    /// Single HTTP POST attempt to a Slack webhook URL.
    async fn send_attempt(&self, url: &str, payload: &str) -> Result<()> {
        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .context("Slack webhook HTTP request failed")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            anyhow::bail!(
                "Slack webhook returned HTTP {}: {}",
                status.as_u16(),
                body.chars().take(200).collect::<String>()
            );
        }

        // Slack returns `ok` in the body for successful deliveries.
        if body.contains("\"ok\":false") || body == "false" {
            anyhow::bail!("Slack webhook returned error: {body}");
        }

        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Slack webhook configuration.
///
/// Supports multiple webhook URLs for different channels, loaded from:
/// - Environment variable `SLACK_WEBHOOK_URLS` (comma-separated, default channel)
/// - Environment variable `SLACK_WEBHOOK_CHANNEL_<NAME>` (channel-specific URLs)
/// - YAML config file at `config/runtime/slack_webhooks.yaml`
#[derive(Debug, Clone, Default, Serialize)]
pub struct SlackConfig {
    /// Channel-specific webhook URLs: map of channel name -> list of URLs.
    pub webhooks: HashMap<String, Vec<String>>,
    /// Default webhook URLs (used when no channel-specific match is found).
    pub default_urls: Vec<String>,
    /// HTTP request timeout in seconds (default: 10).
    pub timeout_secs: u64,
}

impl SlackConfig {
    /// Build configuration from environment variables.
    ///
    /// Reads:
    /// - `SLACK_WEBHOOK_URLS` — comma-separated default webhook URLs
    /// - `SLACK_WEBHOOK_TIMEOUT_SECS` — timeout (default: 10)
    /// - `SLACK_WEBHOOK_CHANNEL_<NAME>` — channel-specific URLs (comma-separated)
    ///
    /// Channel names are lowercased; valid names: `security`, `insight`,
    /// `recipe_match`, `poi_update`, `warning`, `general`.
    pub fn from_env() -> Self {
        let timeout_secs = std::env::var("SLACK_WEBHOOK_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let default_urls = std::env::var("SLACK_WEBHOOK_URLS")
            .map(|s| parse_comma_separated(&s))
            .unwrap_or_default();

        let mut webhooks: HashMap<String, Vec<String>> = HashMap::new();

        // Scan for SLACK_WEBHOOK_CHANNEL_* variables.
        for (key, value) in std::env::vars() {
            if let Some(channel) = key
                .strip_prefix("SLACK_WEBHOOK_CHANNEL_")
                .map(|s| s.to_ascii_lowercase())
            {
                let urls = parse_comma_separated(&value);
                if !urls.is_empty() {
                    webhooks.insert(channel, urls);
                }
            }
        }

        // Also support SLACK_WEBHOOK_SECURITY_URL, SLACK_WEBHOOK_INSIGHT_URL, etc.
        for channel in &[
            "security",
            "insight",
            "recipe_match",
            "poi_update",
            "warning",
            "general",
        ] {
            let env_key = format!("SLACK_WEBHOOK_{}_URL", channel.to_uppercase());
            if let Ok(url) = std::env::var(&env_key) {
                webhooks
                    .entry(channel.to_string())
                    .or_default()
                    .push(url);
            }
        }

        Self {
            webhooks,
            default_urls,
            timeout_secs,
        }
    }

    /// Build configuration from a YAML file.
    ///
    /// Expected YAML structure:
    /// ```yaml
    /// timeout_secs: 10
    /// default_urls:
    ///   - "https://hooks.slack.com/services/..."
    /// channels:
    ///   security:
    ///     - "https://hooks.slack.com/services/..."
    ///   insight:
    ///     - "https://hooks.slack.com/services/..."
    /// ```
    pub fn from_yaml(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read Slack config file: {path}"))?;
        let parsed: SlackConfigFile = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse Slack config YAML: {path}"))?;

        let mut webhooks: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(channels) = parsed.channels {
            for (channel, urls) in channels {
                let channel_lower = channel.to_ascii_lowercase();
                webhooks.insert(channel_lower, urls);
            }
        }

        Ok(Self {
            webhooks,
            default_urls: parsed.default_urls.unwrap_or_default(),
            timeout_secs: parsed.timeout_secs.unwrap_or(10),
        })
    }

    /// Merge another config into this one (environment overrides YAML).
    pub fn merge(&mut self, other: SlackConfig) {
        self.default_urls.extend(other.default_urls);
        for (channel, urls) in other.webhooks {
            self.webhooks.entry(channel).or_default().extend(urls);
        }
        self.timeout_secs = other.timeout_secs.max(self.timeout_secs);
    }
}

/// Helper struct for YAML deserialization.
#[derive(Debug, Deserialize)]
struct SlackConfigFile {
    timeout_secs: Option<u64>,
    default_urls: Option<Vec<String>>,
    channels: Option<HashMap<String, Vec<String>>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_comma_separated(s: &str) -> Vec<String> {
    s.split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

// Use serde::Deserialize for the YAML config struct.
use serde::Deserialize;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Severity tests ────────────────────────────────────────────────

    #[test]
    fn severity_emoji_mapping() {
        assert_eq!(SlackMessageSeverity::Critical.emoji(), "🔴");
        assert_eq!(SlackMessageSeverity::High.emoji(), "🟠");
        assert_eq!(SlackMessageSeverity::Medium.emoji(), "🟡");
        assert_eq!(SlackMessageSeverity::Low.emoji(), "⚪");
        assert_eq!(SlackMessageSeverity::Info.emoji(), "ℹ️");
    }

    #[test]
    fn severity_color_mapping() {
        assert_eq!(SlackMessageSeverity::Critical.color(), "#FF0000");
        assert_eq!(SlackMessageSeverity::High.color(), "#FF8C00");
        assert_eq!(SlackMessageSeverity::Medium.color(), "#FFD700");
        assert_eq!(SlackMessageSeverity::Low.color(), "#36A64F");
    }

    #[test]
    fn severity_from_str() {
        assert_eq!(
            SlackMessageSeverity::from_str("CRITICAL"),
            SlackMessageSeverity::Critical
        );
        assert_eq!(
            SlackMessageSeverity::from_str("high"),
            SlackMessageSeverity::High
        );
        assert_eq!(
            SlackMessageSeverity::from_str("unknown_thing"),
            SlackMessageSeverity::Info
        );
    }

    #[test]
    fn severity_ordering() {
        assert!(SlackMessageSeverity::Critical > SlackMessageSeverity::High);
        assert!(SlackMessageSeverity::High > SlackMessageSeverity::Medium);
        assert!(SlackMessageSeverity::Medium > SlackMessageSeverity::Low);
        assert!(SlackMessageSeverity::Low > SlackMessageSeverity::Info);
    }

    // ── Alert type tests ──────────────────────────────────────────────

    #[test]
    fn alert_type_round_trip() {
        for t in &[
            AlertType::Security,
            AlertType::Insight,
            AlertType::RecipeMatch,
            AlertType::PoiUpdate,
            AlertType::Warning,
            AlertType::General,
        ] {
            assert_eq!(AlertType::from_str(t.as_str()), *t);
        }
    }

    #[test]
    fn alert_type_from_str_fallback() {
        assert_eq!(AlertType::from_str("unknown"), AlertType::General);
        assert_eq!(AlertType::from_str("recipe"), AlertType::RecipeMatch);
    }

    // ── Block Kit builder tests ───────────────────────────────────────

    #[test]
    fn slack_message_builds_valid_blocks() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Critical,
            AlertType::Security,
            "Critical breach detected",
            "A data breach has been detected affecting Acme Corp. Employee credentials were found on the dark web.",
        )
        .with_entity("Acme Corp")
        .with_region("US")
        .with_source_url("https://apexintel.io/warnings/123")
        .with_timestamp("2026-06-14 10:00 UTC");

        let blocks = msg.to_blocks();
        assert!(blocks.is_object(), "blocks should be a JSON object");

        // Verify structure
        assert!(blocks.get("text").is_some(), "should have fallback text");
        assert!(blocks.get("attachments").is_some(), "should have attachments");

        let attachments = blocks["attachments"].as_array().unwrap();
        assert_eq!(attachments.len(), 1, "should have one attachment");

        let attachment = &attachments[0];
        assert_eq!(
            attachment["color"].as_str(),
            Some("#FF0000"),
            "critical severity should have red color"
        );

        let inner_blocks = attachment["blocks"].as_array().unwrap();
        assert!(!inner_blocks.is_empty(), "should have blocks");

        // First block should be a header
        assert_eq!(
            inner_blocks[0]["type"].as_str(),
            Some("header"),
            "first block should be header"
        );

        // Last block should be context
        let last = inner_blocks.last().unwrap();
        assert_eq!(
            last["type"].as_str(),
            Some("context"),
            "last block should be context"
        );

        // Should have actions
        let has_actions = inner_blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("actions")
        });
        assert!(has_actions, "should have actions block");
    }

    #[test]
    fn slack_message_critical_has_red_color() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Critical,
            AlertType::General,
            "Test",
            "Description",
        );
        let blocks = msg.to_blocks();
        let color = blocks["attachments"][0]["color"].as_str().unwrap();
        assert_eq!(color, "#FF0000");
    }

    #[test]
    fn slack_message_high_has_orange_color() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::High,
            AlertType::General,
            "Test",
            "Description",
        );
        let blocks = msg.to_blocks();
        let color = blocks["attachments"][0]["color"].as_str().unwrap();
        assert_eq!(color, "#FF8C00");
    }

    #[test]
    fn slack_message_medium_has_yellow_color() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Medium,
            AlertType::General,
            "Test",
            "Description",
        );
        let blocks = msg.to_blocks();
        let color = blocks["attachments"][0]["color"].as_str().unwrap();
        assert_eq!(color, "#FFD700");
    }

    #[test]
    fn slack_message_low_has_green_color() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Low,
            AlertType::General,
            "Test",
            "Description",
        );
        let blocks = msg.to_blocks();
        let color = blocks["attachments"][0]["color"].as_str().unwrap();
        assert_eq!(color, "#36A64F");
    }

    #[test]
    fn slack_message_serializes_to_valid_json() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::High,
            AlertType::Insight,
            "New competitive insight",
            "Competitor X is expanding into the Moroccan market with a new facility.",
        )
        .with_entity("Competitor X")
        .with_region("MA");

        let json = msg.to_json_string().expect("should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("should be valid JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn slack_message_with_fields() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Medium,
            AlertType::RecipeMatch,
            "Recipe match found",
            "New tender opportunity matches Starz capabilities.",
        )
        .with_entity("Starz")
        .with_field("Confidence", "87%")
        .with_field("Impact", "High")
        .with_field("Category", "Demand Procurement");

        let blocks = msg.to_blocks();
        let attachments = blocks["attachments"].as_array().unwrap();
        let inner_blocks = attachments[0]["blocks"].as_array().unwrap();

        // Find a section with fields
        let field_sections: Vec<&serde_json::Value> = inner_blocks
            .iter()
            .filter(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("section")
                    && b.get("fields").is_some()
            })
            .collect();
        assert!(!field_sections.is_empty(), "should have field sections");
    }

    #[test]
    fn slack_message_without_actions() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Info,
            AlertType::General,
            "Info message",
            "Just an informational update.",
        )
        .with_actions(false);

        let blocks = msg.to_blocks();
        let attachments = blocks["attachments"].as_array().unwrap();
        let inner_blocks = attachments[0]["blocks"].as_array().unwrap();

        let has_actions = inner_blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("actions")
        });
        assert!(!has_actions, "should not have actions block");
    }

    #[test]
    fn slack_message_long_description_truncated() {
        let long_desc = "A".repeat(3000);
        let msg = SlackMessage::new(
            SlackMessageSeverity::Low,
            AlertType::General,
            "Long description test",
            &long_desc,
        );
        let blocks = msg.to_blocks();
        let attachments = blocks["attachments"].as_array().unwrap();
        let inner_blocks = attachments[0]["blocks"].as_array().unwrap();

        // Find the description section
        let desc_section = inner_blocks
            .iter()
            .find(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("section")
                    && b.get("text")
                        .and_then(|t| t.get("type"))
                        .and_then(|t| t.as_str())
                        == Some("mrkdwn")
            })
            .expect("should have description section");

        let text = desc_section["text"]["text"].as_str().unwrap();
        assert!(text.len() <= 2904, "description should be truncated to 2900 chars + ellipsis");
        assert!(text.ends_with('…'), "truncated description should end with ellipsis");
    }

    // ── Config tests ──────────────────────────────────────────────────

    #[test]
    fn slack_config_from_env_empty() {
        // No env vars set — should produce empty config with defaults.
        // Save and clear all SLACK_WEBHOOK_* vars to prevent races with
        // parallel tests that manipulate these env vars.
        unsafe {
            let slack_keys: Vec<String> = std::env::vars()
                .filter(|(k, _)| k.starts_with("SLACK_WEBHOOK"))
                .map(|(k, _)| k)
                .collect();
            let saved: Vec<(String, Option<String>)> = slack_keys
                .iter()
                .map(|k| (k.clone(), std::env::var(k).ok()))
                .collect();
            for k in &slack_keys {
                std::env::remove_var(k);
            }

            // Re-check that no SLACK_WEBHOOK_* vars leaked from another
            // parallel test during our save/remove window.
            for (k, _) in std::env::vars() {
                if k.starts_with("SLACK_WEBHOOK") && !slack_keys.contains(&k) {
                    std::env::remove_var(&k);
                }
            }

            let config = SlackConfig::from_env();
            assert!(config.default_urls.is_empty());
            assert!(config.webhooks.is_empty());
            assert_eq!(config.timeout_secs, 10);

            // Restore saved vars.
            for (k, v) in saved {
                if let Some(val) = v {
                    std::env::set_var(k, val);
                }
            }
        }
    }

    #[test]
    fn slack_config_from_env_with_urls() {
        // Temporarily set env vars
        unsafe {
            std::env::set_var(
                "SLACK_WEBHOOK_URLS",
                "https://hooks.slack.com/services/T00/B00/xxx,https://hooks.slack.com/services/T00/B01/yyy",
            );
        }

        let config = SlackConfig::from_env();
        assert_eq!(config.default_urls.len(), 2);
        assert!(config.default_urls[0].contains("hooks.slack.com"));

        // Clean up
        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_URLS");
        }
    }

    #[test]
    fn slack_config_from_env_with_timeout() {
        unsafe {
            std::env::set_var("SLACK_WEBHOOK_TIMEOUT_SECS", "30");
        }

        let config = SlackConfig::from_env();
        assert_eq!(config.timeout_secs, 30);

        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_TIMEOUT_SECS");
        }
    }

    #[test]
    fn slack_config_from_env_with_channel_urls() {
        unsafe {
            std::env::set_var(
                "SLACK_WEBHOOK_SECURITY_URL",
                "https://hooks.slack.com/services/T00/B02/zzz",
            );
            std::env::set_var(
                "SLACK_WEBHOOK_CHANNEL_INSIGHT",
                "https://hooks.slack.com/services/T00/B03/aaa,https://hooks.slack.com/services/T00/B04/bbb",
            );
        }

        let config = SlackConfig::from_env();

        let security = config.webhooks.get("security");
        assert!(security.is_some(), "should have security channel");
        assert_eq!(security.unwrap().len(), 1);

        let insight = config.webhooks.get("insight");
        assert!(insight.is_some(), "should have insight channel");
        assert_eq!(insight.unwrap().len(), 2);

        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_SECURITY_URL");
            std::env::remove_var("SLACK_WEBHOOK_CHANNEL_INSIGHT");
        }
    }

    #[test]
    fn slack_config_from_yaml_nonexistent() {
        let result = SlackConfig::from_yaml("/tmp/nonexistent_slack_config.yaml");
        assert!(result.is_err(), "should fail for nonexistent file");
    }

    #[test]
    fn slack_message_alert_type_routing() {
        let security_msg = SlackMessage::new(
            SlackMessageSeverity::Critical,
            AlertType::Security,
            "Security alert",
            "Breach detected",
        );
        let blocks = security_msg.to_blocks();
        let attachments = blocks["attachments"].as_array().unwrap();
        let inner_blocks = attachments[0]["blocks"].as_array().unwrap();
        let context = inner_blocks.last().unwrap();
        let text = context["elements"][0]["text"].as_str().unwrap();
        assert!(text.contains("security"), "context should mention alert type");
    }

    #[test]
    fn slack_message_general_alert_type() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Info,
            AlertType::General,
            "General notice",
            "System maintenance scheduled.",
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("General"));
    }

    // ── Integration-like tests (no HTTP) ──────────────────────────────

    #[test]
    fn slack_webhook_constructs() {
        let config = SlackConfig::default();
        let webhook = SlackWebhook::new(&config);
        assert!(webhook.is_ok(), "should construct with default config");
    }

    #[test]
    fn slack_webhook_send_with_no_urls() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = SlackConfig::default();
            let webhook = SlackWebhook::new(&config).unwrap();
            let msg = SlackMessage::new(
                SlackMessageSeverity::Info,
                AlertType::General,
                "Test",
                "No URLs test",
            );
            // Should succeed (no-op) because no URLs are configured
            let result = webhook.send(&msg).await;
            assert!(result.is_ok(), "send with no URLs should succeed as no-op");
        });
    }

    #[test]
    fn parse_comma_separated_works() {
        let result = parse_comma_separated("a, b, c");
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_comma_separated_empty() {
        let result = parse_comma_separated("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_comma_separated_single() {
        let result = parse_comma_separated("https://hooks.slack.com/services/T00/B00/xxx");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn slack_config_merge() {
        let mut base = SlackConfig {
            default_urls: vec!["https://hooks.slack.com/services/default".into()],
            webhooks: HashMap::new(),
            timeout_secs: 10,
        };

        let mut other_webhooks = HashMap::new();
        other_webhooks.insert(
            "security".into(),
            vec!["https://hooks.slack.com/services/security".into()],
        );

        let other = SlackConfig {
            default_urls: vec!["https://hooks.slack.com/services/extra".into()],
            webhooks: other_webhooks,
            timeout_secs: 15,
        };

        base.merge(other);

        assert_eq!(base.default_urls.len(), 2);
        assert!(base.webhooks.contains_key("security"));
        assert_eq!(base.timeout_secs, 15);
    }

    // ── Test each alert type mapping ──────────────────────────────────

    #[test]
    fn alert_type_security_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Critical,
            AlertType::Security,
            "Security incident",
            description_for_type("security"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("security"));
        assert!(json.contains("🔴"));
    }

    #[test]
    fn alert_type_insight_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Medium,
            AlertType::Insight,
            "Market insight",
            description_for_type("insight"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("insight"));
        assert!(json.contains("🟡"));
    }

    #[test]
    fn alert_type_recipe_match_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::High,
            AlertType::RecipeMatch,
            "Recipe match",
            description_for_type("recipe"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("recipe_match"));
        assert!(json.contains("🟠"));
    }

    #[test]
    fn alert_type_poi_update_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Low,
            AlertType::PoiUpdate,
            "POI update",
            description_for_type("poi"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("poi_update"));
    }

    #[test]
    fn alert_type_warning_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::High,
            AlertType::Warning,
            "Warning verification",
            description_for_type("warning"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("warning"));
    }

    #[test]
    fn alert_type_general_mapping() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Info,
            AlertType::General,
            "General info",
            description_for_type("general"),
        );
        let json = msg.to_json_string().expect("should serialize");
        assert!(json.contains("General"));
    }

    fn description_for_type(t: &str) -> String {
        format!("This is a test description for the {t} alert type with sufficient content to verify the Block Kit message structure is correct.")
    }
}
