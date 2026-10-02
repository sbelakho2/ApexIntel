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
use apex_core::text::truncate_utf8;
use reqwest::Client;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::notification_delivery::{classify_http_status, DeliveryDisposition};
use crate::notifications::redact_url;

// ─────────────────────────────────────────────────────────────────────────────
// mrkdwn safety helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Escape the characters Slack's mrkdwn parser interprets as control syntax.
///
/// Crawled text is attacker-influenced: `<` enables special mentions
/// (`<!channel>` pages everyone) and disguised links (`<https://evil|text>`),
/// while bare `&` can corrupt surrounding entities. Slack renders the escaped
/// entities back to the literal characters, so this is lossless for readers.
pub(crate) fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Clip `s` to at most `max_chars` **characters**, appending a single-character
/// ellipsis when truncation happens.
///
/// Slack block limits are character-based (header 150, section text 3000,
/// field text 2000) and a 400 `invalid_blocks` response is classified as a
/// permanent delivery failure, so oversized crawled text must be clipped
/// instead of dead-lettering the alert. Byte slicing is never used, so
/// multibyte input cannot panic or split a character.
pub(crate) fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

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
        Self::new(severity, AlertType::General, title, description)
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
        let escaped_description = slack_escape(&self.description);
        let escaped_title = slack_escape(&self.title);

        // ── Header block ──────────────────────────────────────────────
        // Header text is `plain_text` (never parsed as mrkdwn) and capped at
        // 150 characters; a long crawled title must be clipped, not rejected.
        let header_text = clip(
            &format!("{} {} — {}", emoji, severity_upper, self.title),
            150,
        );
        let mut blocks: Vec<serde_json::Value> = vec![serde_json::json!({
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": header_text,
                "emoji": true
            }
        })];

        // ── Entity / context section ──────────────────────────────────
        let mut context_fields: Vec<serde_json::Value> = Vec::new();
        if let Some(ref entity) = self.entity {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": clip(&format!("*Entity:*\n{}", slack_escape(entity)), 2000)
            }));
        }
        if let Some(ref region) = self.region {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": clip(&format!("*Region:*\n{}", slack_escape(region)), 2000)
            }));
        }
        if let Some(ref ts) = self.timestamp {
            context_fields.push(serde_json::json!({
                "type": "mrkdwn",
                "text": clip(&format!("*Timestamp:*\n{}", ts), 2000)
            }));
        }

        if !context_fields.is_empty() {
            blocks.push(serde_json::json!({
                "type": "section",
                "fields": context_fields
            }));
        }

        // ── Description section ────────────────────────────────────────
        // Slack Block Kit has a 3000 character limit on mrkdwn text blocks, so
        // truncate at a 2900-character budget (leaving room for the ellipsis).
        // The cut must land on a UTF-8 boundary: byte slicing a multibyte
        // description previously panicked (audit #67).
        let desc = if escaped_description.chars().count() > 2900 {
            format!("{}…", truncate_utf8(&escaped_description, 2900))
        } else {
            escaped_description.clone()
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
                        "text": clip(
                            &format!("*{}:*\n{}", slack_escape(k), slack_escape(v)),
                            2000
                        )
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
        // Only the link button survives: Acknowledge/Dismiss had no Slack
        // interaction handler and did nothing when clicked (audit #80). The
        // block is omitted entirely when there is no URL button, because an
        // actions block with zero elements is rejected by Slack.
        if self.include_actions {
            if let Some(ref url) = self.source_url {
                blocks.push(serde_json::json!({
                    "type": "actions",
                    "elements": [{
                        "type": "button",
                        "text": {
                            "type": "plain_text",
                            "text": "🔍 View in ApexIntel",
                            "emoji": true
                        },
                        "url": url,
                        "action_id": "view_apexintel"
                    }]
                }));
            }
        }

        // ── Context / footer ──────────────────────────────────────────
        blocks.push(serde_json::json!({
            "type": "context",
            "elements": [
                {
                    "type": "mrkdwn",
                    "text": clip(
                        &format!(
                            "ApexIntel • {} • `{}`",
                            self.alert_type.as_str(),
                            chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
                        ),
                        3000
                    )
                }
            ]
        }));

        // ── Build the full payload with attachment color ───────────────
        let fallback = format!(
            "{} *[{}]* {} — {}",
            emoji,
            severity_upper,
            escaped_title,
            escaped_description.chars().take(120).collect::<String>()
        );
        serde_json::json!({
            "text": clip(&fallback, 4000),
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
        serde_json::to_string(&self.to_blocks())
            .context("failed to serialize Slack message to JSON")
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

    /// Reserve the next send slot for `url` and wait until that slot is due.
    ///
    /// The slot (`max(last + 2s, now)`) is computed and stored **inside the
    /// lock**, so two concurrent senders for the same URL can never be handed
    /// overlapping slots; each waits on its own reserved instant (audit #77).
    async fn wait_if_needed(&self, url: &str) {
        let min_interval = Duration::from_secs(2);
        let slot = {
            let mut last_send = self.last_send.lock().await;
            let now = tokio::time::Instant::now();
            let slot = match last_send.get(url) {
                Some(last) => (*last + min_interval).max(now),
                None => now,
            };
            last_send.insert(url.to_string(), slot);
            slot
        };
        tokio::time::sleep_until(slot).await;
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
        // Operator-configured Slack webhook endpoint, not crawled content.
        #[allow(clippy::disallowed_methods)]
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

        let targets = self.targets_for_channel(channel_name);

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
                // Redacted: the webhook path is the credential.
                errors.push(format!("{}: {}", redact_url(url), e));
                error!(
                    url = %redact_url(url),
                    error = %e,
                    "Failed to send Slack message"
                );
            } else {
                info!(
                    url = %redact_url(url),
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

    /// Resolve the target URLs for an alert type.
    ///
    /// Channel-specific targets win outright: the default URLs are consulted
    /// only when the channel has none configured, and duplicates are removed,
    /// so a URL listed in both sets is never posted to twice (audit #77).
    fn targets_for_channel(&self, channel_name: &str) -> Vec<String> {
        let specific = self.urls.get(channel_name).cloned().unwrap_or_default();
        let targets = if specific.is_empty() {
            self.default_urls.clone()
        } else {
            specific
        };
        let mut seen = HashSet::new();
        targets
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .collect()
    }

    /// Send a raw JSON payload to a single webhook URL with retry logic.
    ///
    /// Permanent failures (e.g. a revoked webhook returning 404) are **not**
    /// retried; retryable failures honor `Retry-After` when Slack sends one
    /// (429 rate limiting) and otherwise use exponential backoff.
    async fn send_to_url(&self, url: &str, payload: &str) -> Result<()> {
        // Enforce rate limit: max 1 message per 2 seconds per URL.
        self.rate_limiter.wait_if_needed(url).await;

        let mut last_error = None;
        for attempt in 1..=3 {
            match self.send_attempt(url, payload).await {
                Ok(()) => return Ok(()),
                Err(failure) => {
                    let retryable = matches!(failure, SendFailure::Retryable { .. });
                    let retry_after = failure.retry_after();
                    let error = failure.into_error();
                    warn!(
                        url = %redact_url(url),
                        attempt,
                        error = %error,
                        retryable,
                        "Slack webhook attempt failed"
                    );
                    if !retryable {
                        // A 4xx other than 408/425/429 means retrying cannot
                        // succeed; return immediately instead of burning
                        // three attempts (and three rate-limit slots).
                        return Err(error);
                    }
                    last_error = Some(error);
                    if attempt < 3 {
                        // Honor Retry-After on 429; otherwise exponential
                        // backoff: 1s, 2s.
                        let backoff =
                            retry_after.unwrap_or_else(|| Duration::from_secs(1 << (attempt - 1)));
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| anyhow::anyhow!("Slack webhook delivery failed after 3 retries")))
    }

    /// Single HTTP POST attempt to a Slack webhook URL.
    async fn send_attempt(&self, url: &str, payload: &str) -> std::result::Result<(), SendFailure> {
        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .map_err(|error| SendFailure::Retryable {
                // reqwest's Display embeds the webhook URL, so strip it:
                // the URL is the credential and must never reach logs.
                error: anyhow::anyhow!(
                    "Slack webhook HTTP request failed: {}",
                    error.without_url()
                ),
                retry_after: None,
            })?;

        let status = resp.status();
        let retry_after = parse_retry_after(resp.headers());
        let body = match resp.text().await {
            Ok(body) => body,
            Err(error) => {
                // Transport read failure: report the status without a body
                // rather than silently substituting empty text.
                tracing::debug!(
                    error = %error.without_url(),
                    "Slack webhook response body unreadable"
                );
                String::new()
            }
        };

        if !status.is_success() {
            let message = format!(
                "Slack webhook returned HTTP {}: {}",
                status.as_u16(),
                body.chars().take(200).collect::<String>()
            );
            return match classify_http_status(status.as_u16()) {
                DeliveryDisposition::Retryable => Err(SendFailure::Retryable {
                    error: anyhow::anyhow!(message),
                    retry_after,
                }),
                DeliveryDisposition::Permanent => {
                    Err(SendFailure::Permanent(anyhow::anyhow!(message)))
                }
            };
        }

        // Slack returns `ok` in the body for successful deliveries.
        if body.contains("\"ok\":false") || body == "false" {
            return Err(SendFailure::Retryable {
                error: anyhow::anyhow!("Slack webhook returned error: {body}"),
                retry_after,
            });
        }

        Ok(())
    }
}

/// Outcome of one Slack webhook POST attempt.
enum SendFailure {
    /// Transient (network error, 408/425/429, 5xx): retry with backoff.
    Retryable {
        error: anyhow::Error,
        retry_after: Option<Duration>,
    },
    /// Permanent (any other 4xx, e.g. a revoked webhook): do not retry.
    Permanent(anyhow::Error),
}

impl SendFailure {
    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Retryable { retry_after, .. } => *retry_after,
            Self::Permanent(_) => None,
        }
    }

    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Retryable { error, .. } | Self::Permanent(error) => error,
        }
    }
}

/// Parse a `Retry-After` header in delta-seconds form.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Slack webhook configuration.
///
/// The environment path reads the single consolidated variable set shared
/// with the durable notification router ([`crate::notifications::WebhookConfig`]);
/// per-channel routing is still available from the YAML file.
///
/// Loaded from:
/// - Environment variable `SLACK_WEBHOOK_URL` (the consolidated Slack webhook)
/// - YAML config file at `config/runtime/slack_webhooks.yaml`
#[derive(Clone, Default, Serialize)]
pub struct SlackConfig {
    /// Channel-specific webhook URLs: map of channel name -> list of URLs.
    pub webhooks: HashMap<String, Vec<String>>,
    /// Default webhook URLs (used when no channel-specific match is found).
    pub default_urls: Vec<String>,
    /// HTTP request timeout in seconds (default: 10).
    pub timeout_secs: u64,
}

/// `Debug` must never print webhook URLs: the last path segment is the
/// credential. URLs are shown redacted instead (audit #73).
impl std::fmt::Debug for SlackConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted: HashMap<&str, Vec<String>> = self
            .webhooks
            .iter()
            .map(|(channel, urls)| {
                (
                    channel.as_str(),
                    urls.iter().map(|url| redact_url(url)).collect(),
                )
            })
            .collect();
        f.debug_struct("SlackConfig")
            .field("webhooks", &redacted)
            .field(
                "default_urls",
                &self
                    .default_urls
                    .iter()
                    .map(|url| redact_url(url))
                    .collect::<Vec<_>>(),
            )
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl SlackConfig {
    /// Build configuration from environment variables.
    ///
    /// Reads the durable pipeline's variable set:
    /// - `SLACK_WEBHOOK_URL` — the Slack incoming webhook
    /// - `SLACK_WEBHOOK_TIMEOUT_SECS` — timeout (default: 10)
    ///
    /// The legacy `SLACK_WEBHOOK_URLS`, `SLACK_WEBHOOK_CHANNEL_<NAME>` and
    /// `SLACK_WEBHOOK_<CHANNEL>_URL` variables were a second, conflicting
    /// pipeline and are no longer read (audit #81).
    pub fn from_env() -> Self {
        let timeout_secs = std::env::var("SLACK_WEBHOOK_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let default_urls = std::env::var("SLACK_WEBHOOK_URL")
            .map(|s| parse_comma_separated(&s))
            .unwrap_or_default();

        Self {
            webhooks: HashMap::new(),
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
        assert!(
            blocks.get("attachments").is_some(),
            "should have attachments"
        );

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
        let has_actions = inner_blocks
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("actions"));
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
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
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

        let has_actions = inner_blocks
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("actions"));
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
        assert!(
            text.len() <= 2904,
            "description should be truncated to 2900 chars + ellipsis"
        );
        assert!(
            text.ends_with('…'),
            "truncated description should end with ellipsis"
        );
    }

    #[test]
    fn slack_message_multibyte_description_does_not_panic() {
        // 3000 four-byte chars: a byte slice at 2900 panicked before audit #67.
        let msg = SlackMessage::new(
            SlackMessageSeverity::Low,
            AlertType::General,
            "Unicode description",
            "😀".repeat(3000),
        );
        let blocks = msg.to_blocks();
        let inner = blocks["attachments"][0]["blocks"].as_array().unwrap();
        let desc = inner
            .iter()
            .find(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("section")
                    && b.get("text")
                        .and_then(|t| t.get("type"))
                        .and_then(|t| t.as_str())
                        == Some("mrkdwn")
            })
            .expect("description section");
        let text = desc["text"]["text"].as_str().unwrap();
        assert!(text.chars().count() <= 2901);
        assert!(text.ends_with('…'));
        assert!(
            std::str::from_utf8(text.as_bytes()).is_ok(),
            "truncation must land on a UTF-8 boundary"
        );
    }

    #[test]
    fn slack_escape_neutralizes_mrkdwn_control_syntax() {
        assert_eq!(
            slack_escape("a & b <https://evil|Click> <!channel>"),
            "a &amp; b &lt;https://evil|Click&gt; &lt;!channel&gt;"
        );
    }

    #[test]
    fn clip_is_char_safe_and_appends_ellipsis() {
        assert_eq!(clip("abc", 5), "abc");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("😀😀😀", 2), "😀…");
        assert_eq!(clip("anything", 0), "");
        assert_eq!(clip("", 10), "");
    }

    #[test]
    fn slack_mrkdwn_fields_are_escaped() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::High,
            AlertType::Warning,
            "<!channel> breach",
            "<https://evil.example|Click here> & more",
        )
        .with_entity("<https://evil.example|Acme>")
        .with_region("<!here>")
        .with_field("Detail", "<!channel> go");

        let blocks = msg.to_blocks();
        let inner = blocks["attachments"][0]["blocks"].as_array().unwrap();

        let mut mrkdwn = String::new();
        for block in inner {
            if block.get("type").and_then(|t| t.as_str()) == Some("header") {
                // Header is plain_text: no mrkdwn parsing, intentionally raw.
                continue;
            }
            if let Some(text) = block
                .get("text")
                .and_then(|t| t.get("text"))
                .and_then(|t| t.as_str())
            {
                mrkdwn.push_str(text);
            }
            if let Some(fields) = block.get("fields").and_then(|f| f.as_array()) {
                for field in fields {
                    if let Some(text) = field.get("text").and_then(|t| t.as_str()) {
                        mrkdwn.push_str(text);
                    }
                }
            }
        }

        assert!(!mrkdwn.contains("<!channel>"), "mention must not survive");
        assert!(
            !mrkdwn.contains("<https://evil.example"),
            "disguised link must not survive"
        );
        assert!(mrkdwn.contains("&lt;!channel&gt;"));
        assert!(mrkdwn.contains("&lt;https://evil.example|Acme&gt;"));
        assert!(mrkdwn.contains("&amp; more"));

        let fallback = blocks["text"].as_str().unwrap();
        assert!(fallback.contains("&lt;!channel&gt;"));
        assert!(!fallback.contains("<!channel>"));
    }

    #[test]
    fn slack_actions_only_contain_the_view_link_button() {
        let msg = SlackMessage::new(
            SlackMessageSeverity::Critical,
            AlertType::Security,
            "Breach",
            "Details",
        )
        .with_source_url("https://apexintel.io/warnings/123");
        let blocks = msg.to_blocks();
        let inner = blocks["attachments"][0]["blocks"].as_array().unwrap();
        let actions: Vec<&serde_json::Value> = inner
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("actions"))
            .collect();
        assert_eq!(actions.len(), 1, "exactly one actions block");
        let elements = actions[0]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 1, "only the URL button survives");
        assert_eq!(elements[0]["action_id"].as_str(), Some("view_apexintel"));

        let rendered = serde_json::to_string(&blocks).unwrap();
        assert!(!rendered.contains("acknowledge_alert"));
        assert!(!rendered.contains("dismiss_alert"));
    }

    #[test]
    fn slack_actions_block_is_omitted_without_a_url_button() {
        // include_actions defaults to true, but an actions block with zero
        // elements is rejected by Slack (400 invalid_blocks).
        let msg = SlackMessage::new(
            SlackMessageSeverity::Info,
            AlertType::General,
            "No link",
            "Nothing to click",
        );
        let blocks = msg.to_blocks();
        let inner = blocks["attachments"][0]["blocks"].as_array().unwrap();
        assert!(!inner
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("actions")));
    }

    #[test]
    fn slack_webhook_targets_prefer_channel_specific_and_dedup() {
        let mut webhooks = HashMap::new();
        webhooks.insert(
            "security".to_string(),
            vec![
                "https://hooks.slack.com/services/sec".to_string(),
                "https://hooks.slack.com/services/default".to_string(),
            ],
        );
        let config = SlackConfig {
            webhooks,
            default_urls: vec![
                "https://hooks.slack.com/services/default".to_string(),
                "https://hooks.slack.com/services/other".to_string(),
            ],
            timeout_secs: 10,
        };
        let webhook = SlackWebhook::new(&config).unwrap();

        // Channel-specific targets win outright: defaults are not added.
        assert_eq!(
            webhook.targets_for_channel("security"),
            vec![
                "https://hooks.slack.com/services/sec",
                "https://hooks.slack.com/services/default"
            ]
        );
        // Unknown channel falls back to the defaults, deduplicated.
        assert_eq!(
            webhook.targets_for_channel("general"),
            vec![
                "https://hooks.slack.com/services/default",
                "https://hooks.slack.com/services/other"
            ]
        );
    }

    #[test]
    fn retry_after_header_is_parsed_in_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "7".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(7)));

        let mut http_date = reqwest::header::HeaderMap::new();
        http_date.insert(
            "retry-after",
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(parse_retry_after(&http_date), None);
    }

    #[test]
    fn slack_config_debug_redacts_every_webhook_url() {
        let mut config = SlackConfig {
            webhooks: HashMap::new(),
            default_urls: vec![
                "https://hooks.slack.com/services/T00/B00/defaultsecret7788".to_string()
            ],
            timeout_secs: 10,
        };
        config.webhooks.insert(
            "security".to_string(),
            vec!["https://hooks.slack.com/services/T00/B00/channelsecret1234".to_string()],
        );

        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("channelsecret1234"),
            "Debug leaked the channel webhook path: {rendered}"
        );
        assert!(
            !rendered.contains("defaultsecret7788"),
            "Debug leaked the default webhook path: {rendered}"
        );
        assert!(
            rendered.contains("hooks.slack.com"),
            "the host is still useful for operators: {rendered}"
        );
        assert!(rendered.contains("security"), "{rendered}");
        assert!(rendered.contains("timeout_secs"), "{rendered}");
    }

    /// A slot reservation must happen while the map lock is held: two
    /// concurrent senders for the same URL must be handed slots two seconds
    /// apart, never the same instant (audit #77). If the slot were computed
    /// outside the lock both senders would reserve `now` and return together.
    #[tokio::test]
    async fn rate_limiter_hands_out_non_overlapping_slots() {
        let limiter = Arc::new(PerUrlRateLimiter::new());
        let url = "https://hooks.slack.com/services/T00/B00/ratelimited";

        let started = tokio::time::Instant::now();
        let first = {
            let limiter = Arc::clone(&limiter);
            tokio::spawn(async move {
                limiter.wait_if_needed(url).await;
                tokio::time::Instant::now()
            })
        };
        let second = {
            let limiter = Arc::clone(&limiter);
            tokio::spawn(async move {
                limiter.wait_if_needed(url).await;
                tokio::time::Instant::now()
            })
        };
        let first_done = first.await.unwrap();
        let second_done = second.await.unwrap();

        let gap = if first_done >= second_done {
            first_done - second_done
        } else {
            second_done - first_done
        };
        assert!(
            gap >= Duration::from_millis(1_800),
            "concurrent senders must get slots 2s apart, gap was {gap:?}"
        );

        let stored = limiter.last_send.lock().await;
        let last = stored.get(url).copied().expect("slot recorded");
        assert!(
            last - started >= Duration::from_millis(1_800),
            "the reserved slot must be in the future, not a stale instant"
        );
    }

    // ── Config tests ──────────────────────────────────────────────────

    #[test]
    fn slack_config_from_env_empty() {
        let _guard = SLACK_ENV_TEST_LOCK.lock().unwrap();
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
    fn slack_config_from_env_with_url() {
        let _guard = SLACK_ENV_TEST_LOCK.lock().unwrap();
        // Temporarily set env vars
        unsafe {
            std::env::set_var(
                "SLACK_WEBHOOK_URL",
                "https://hooks.slack.com/services/T00/B00/xxx",
            );
        }

        let config = SlackConfig::from_env();
        assert_eq!(config.default_urls.len(), 1);
        assert!(config.default_urls[0].contains("hooks.slack.com"));

        // Clean up
        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_URL");
        }
    }

    #[test]
    fn slack_config_from_env_with_timeout() {
        let _guard = SLACK_ENV_TEST_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("SLACK_WEBHOOK_TIMEOUT_SECS", "30");
        }

        let config = SlackConfig::from_env();
        assert_eq!(config.timeout_secs, 30);

        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_TIMEOUT_SECS");
        }
    }

    /// A mutex that serializes all tests that touch `SLACK_WEBHOOK_*` env vars,
    /// preventing races between parallel test threads. Tests that set or clear
    /// these vars must lock this mutex for their entire body.
    static SLACK_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn slack_config_from_env_ignores_the_deleted_variable_set() {
        let _guard = SLACK_ENV_TEST_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var(
                "SLACK_WEBHOOK_URL",
                "https://hooks.slack.com/services/T00/B00/consolidated",
            );
            std::env::set_var(
                "SLACK_WEBHOOK_SECURITY_URL",
                "https://hooks.slack.com/services/T00/B02/zzz",
            );
            std::env::set_var(
                "SLACK_WEBHOOK_CHANNEL_INSIGHT",
                "https://hooks.slack.com/services/T00/B03/aaa",
            );
            std::env::set_var(
                "SLACK_WEBHOOK_URLS",
                "https://hooks.slack.com/services/T00/B04/legacy",
            );
        }

        let config = SlackConfig::from_env();

        // Exactly one variable set survives: the legacy per-channel / plural
        // variables must not create channels or defaults (audit #81).
        assert!(config.webhooks.is_empty());
        assert_eq!(
            config.default_urls,
            vec!["https://hooks.slack.com/services/T00/B00/consolidated"]
        );

        unsafe {
            std::env::remove_var("SLACK_WEBHOOK_URL");
            std::env::remove_var("SLACK_WEBHOOK_SECURITY_URL");
            std::env::remove_var("SLACK_WEBHOOK_CHANNEL_INSIGHT");
            std::env::remove_var("SLACK_WEBHOOK_URLS");
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
        assert!(
            text.contains("security"),
            "context should mention alert type"
        );
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
