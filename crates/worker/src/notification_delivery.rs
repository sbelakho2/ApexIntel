//! Durable per-channel notification delivery (retry processor).
//!
//! ```text
//! domain alert/event persisted (notification_events + event_outbox, one TX)
//!     -> per-channel outbox rows persisted `pending` BEFORE any attempt
//!     -> retry processor job (every minute) claims due rows with a lease
//!     -> channel transport attempts the send OUTSIDE any transaction
//!     -> delivered | failed(next_retry_at backoff+jitter) | dead_lettered
//! ```
//!
//! Guarantees:
//! * **Persist before attempt** — a crash mid-send leaves a `delivering` row
//!   whose lease expires, so the next cycle retries it; nothing is lost.
//! * **Claim/lease** — `FOR UPDATE SKIP LOCKED` plus `lease_owner`/`lease_until`
//!   mean one delivery row is attempted by one worker at a time, and no
//!   database lock is held while the channel request is in flight.
//! * **Bounded retries** — retryable failures back off exponentially with
//!   jitter up to a per-channel attempt budget; permanent failures (bad
//!   destination, 4xx, 5xx SMTP reply) dead-letter immediately.
//! * **At-least-once** — a delivery accepted by the channel but not recorded
//!   (crash) is retried; every attempt for a delivery row carries the same
//!   stable idempotency key built from
//!   (notification_event_id, channel, destination, payload_hash) — the attempt
//!   number is NOT part of the key — so the channel can deduplicate a
//!   redelivery after the lease expires and the row is reclaimed. Exactly-once
//!   is not claimed.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use lettre::message::{header::ContentType, Mailbox, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};
use uuid::Uuid;

use apex_store::postgres::{
    notification_payload_hash, DeliveryChannel, NewNotificationEvent, NotificationDeliveryRow,
    PgStore,
};

use crate::notifications::{
    redact_url, EmailConfig, NotificationConfig, PendingAlert, WebhookConfig, WebhookFormat,
};

/// Maximum number of deliveries attempted concurrently in one cycle.
///
/// A batch of 40 rows at 8-way concurrency is 5 waves; with the 20s SMTP
/// timeout that is a 100s worst case, inside the 120s claim lease, so a slow
/// destination cannot outlive its lease and trigger a duplicate send from
/// another worker (audit #76). The unit test
/// `delivery_batch_worst_case_fits_inside_the_claim_lease` pins this invariant.
const DELIVERY_CONCURRENCY: usize = 8;

/// Per-request SMTP timeout; together with the concurrency cap this bounds a
/// delivery cycle (audit #74).
const SMTP_TIMEOUT_SECS: u64 = 20;

/// Maximum attempts for webhook/generic channels.
pub const MAX_DELIVERY_ATTEMPTS: i32 = 8;

/// Email relays tend to reject faster; keep the budget smaller.
pub const EMAIL_MAX_ATTEMPTS: i32 = 5;

/// Initial retry backoff.
pub const BASE_RETRY_DELAY_SECS: i64 = 30;

/// Backoff ceiling.
pub const MAX_RETRY_DELAY_SECS: i64 = 3600;

/// Claim lease: long enough for one channel attempt plus settlement.
pub const DELIVERY_LEASE_SECS: f64 = 120.0;

/// Default rows claimed per cycle.
///
/// Bounded by the lease invariant: `ceil(batch / DELIVERY_CONCURRENCY) *
/// SMTP_TIMEOUT_SECS <= DELIVERY_LEASE_SECS` (audit #76). The previous value
/// of 50 produced a 140s worst case against a 120s lease, so late rows could
/// be reclaimed and re-sent while still in flight.
pub const DEFAULT_DELIVERY_BATCH: i64 = 40;

/// How a failed attempt must be treated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryFailure {
    /// Transient: retry with backoff until the channel budget is exhausted.
    Retryable(String),
    /// Permanent: dead-letter immediately (retrying cannot succeed).
    Permanent(String),
}

impl DeliveryFailure {
    pub fn message(&self) -> &str {
        match self {
            Self::Retryable(message) | Self::Permanent(message) => message,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

impl std::fmt::Display for DeliveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Retry classification for an HTTP response status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryDisposition {
    Retryable,
    Permanent,
}

/// Webhook/HTTP classification: timeouts, request-too-early, rate limits and
/// server errors retry; other 4xx are permanent (bad destination/payload).
pub fn classify_http_status(status: u16) -> DeliveryDisposition {
    match status {
        408 | 425 | 429 => DeliveryDisposition::Retryable,
        500..=599 => DeliveryDisposition::Retryable,
        _ => DeliveryDisposition::Permanent,
    }
}

/// SMTP classification: 4xx replies are transient, 5xx permanent.
pub fn classify_smtp_permanent(is_permanent: bool) -> DeliveryDisposition {
    if is_permanent {
        DeliveryDisposition::Permanent
    } else {
        DeliveryDisposition::Retryable
    }
}

/// Per-channel attempt budgets.
pub fn max_attempts_for_channel(channel: &str) -> i32 {
    if channel.eq_ignore_ascii_case("email") {
        EMAIL_MAX_ATTEMPTS
    } else {
        MAX_DELIVERY_ATTEMPTS
    }
}

/// Exponential backoff with jitter.
///
/// `attempts` is the number of attempts already made for this row (>= 1).
/// `jitter_fraction` in [0, 1) scales the delay between 50% and 100% of the
/// capped exponential value, so a fleet of workers does not retry in lockstep.
pub fn retry_delay_secs(attempts: i32, jitter_fraction: f64) -> i64 {
    let exponent = (attempts.clamp(1, 16) - 1) as u32;
    let exponential = BASE_RETRY_DELAY_SECS.saturating_mul(1i64 << exponent.min(31));
    let capped = exponential.min(MAX_RETRY_DELAY_SECS);
    let jitter = jitter_fraction.clamp(0.0, 0.999_999);
    ((capped as f64) * (0.5 + 0.5 * jitter)).round() as i64
}

/// Clock-derived jitter in [0, 1); avoids a `rand` dependency.
fn clock_jitter_fraction() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.subsec_nanos() % 1_000_000) as f64 / 1_000_000.0)
        .unwrap_or(0.5)
}

/// Stable per-delivery idempotency key:
/// sha256(notification_event_id | channel | destination | payload_hash).
///
/// The attempt number is deliberately **not** part of the identity. A send
/// that the channel accepted but whose settlement was lost (crash, lease
/// expiry, row reclaim) is retried with the same key, so the receiver can
/// deduplicate the redelivery. The attempt travels separately as the
/// `X-Apex-Attempt` header.
pub fn transport_idempotency_key(
    notification_event_id: Option<Uuid>,
    channel: &str,
    destination: &str,
    payload_hash: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        notification_event_id
            .map(|id| id.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"|");
    hasher.update(channel.as_bytes());
    hasher.update(b"|");
    hasher.update(destination.as_bytes());
    hasher.update(b"|");
    hasher.update(payload_hash.unwrap_or_default().as_bytes());
    hex::encode(hasher.finalize())
}

/// The domain payload persisted for one channel delivery.
///
/// The enqueue path stores `{ "alert": <PendingAlert> }`; `subject`/`body`
/// are optional pre-rendered transport content. When absent, the router
/// renders the channel body from the alert at delivery time, so a stored row
/// is always deliverable rather than dead-lettering on a missing field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotificationDeliveryPayload {
    pub alert: PendingAlert,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body: String,
}

/// A claimed delivery ready for a channel attempt.
#[derive(Debug, Clone)]
pub struct NotificationDelivery {
    pub delivery_key: String,
    pub notification_event_id: Option<Uuid>,
    pub channel: String,
    pub destination: String,
    pub attempts: i32,
    pub idempotency_key: String,
    pub payload: NotificationDeliveryPayload,
}

impl NotificationDelivery {
    /// Parse a claimed row into a delivery. A payload that cannot be
    /// deserialized is a permanent failure.
    pub fn from_row(row: &NotificationDeliveryRow) -> Result<Self, DeliveryFailure> {
        let payload: NotificationDeliveryPayload = serde_json::from_value(row.payload.clone())
            .map_err(|error| {
                DeliveryFailure::Permanent(format!("delivery payload is not valid JSON: {error}"))
            })?;
        let payload_hash = row
            .payload_hash
            .clone()
            .unwrap_or_else(|| notification_payload_hash(&row.payload));
        let idempotency_key = transport_idempotency_key(
            row.notification_event_id,
            &row.channel,
            &row.destination,
            Some(&payload_hash),
        );
        Ok(Self {
            delivery_key: row.delivery_key.clone(),
            notification_event_id: row.notification_event_id,
            channel: row.channel.clone(),
            destination: row.destination.clone(),
            attempts: row.attempts,
            idempotency_key,
            payload,
        })
    }
}

/// Outcome counters for one retry-processor cycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryCycleOutcome {
    pub claimed: usize,
    pub delivered: usize,
    pub retried: usize,
    pub dead_lettered: usize,
}

/// Storage half of the retry processor.
#[async_trait]
pub trait DeliveryClaimStore: Send + Sync {
    async fn claim_due_deliveries(
        &self,
        owner: &str,
        limit: i64,
    ) -> Result<Vec<NotificationDeliveryRow>>;
    async fn mark_delivered(&self, owner: &str, delivery_key: &str) -> Result<bool>;
    async fn mark_retry(
        &self,
        owner: &str,
        delivery_key: &str,
        next_retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<bool>;
    async fn mark_dead_lettered(
        &self,
        owner: &str,
        delivery_key: &str,
        error: &str,
    ) -> Result<bool>;
}

#[async_trait]
impl DeliveryClaimStore for PgStore {
    async fn claim_due_deliveries(
        &self,
        owner: &str,
        limit: i64,
    ) -> Result<Vec<NotificationDeliveryRow>> {
        Ok(self
            .claim_due_notification_deliveries(owner, DELIVERY_LEASE_SECS, limit)
            .await?)
    }

    async fn mark_delivered(&self, owner: &str, delivery_key: &str) -> Result<bool> {
        Ok(self
            .mark_notification_delivered(delivery_key, owner)
            .await?)
    }

    async fn mark_retry(
        &self,
        owner: &str,
        delivery_key: &str,
        next_retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<bool> {
        Ok(self
            .mark_notification_retry(delivery_key, owner, next_retry_at, error)
            .await?)
    }

    async fn mark_dead_lettered(
        &self,
        owner: &str,
        delivery_key: &str,
        error: &str,
    ) -> Result<bool> {
        Ok(self
            .mark_notification_dead_lettered(delivery_key, owner, error)
            .await?)
    }
}

/// The channel side of the retry processor.
#[async_trait]
pub trait ChannelTransport: Send + Sync {
    /// Attempt one channel delivery. `Ok(())` means the channel accepted it.
    async fn deliver(&self, delivery: &NotificationDelivery) -> Result<(), DeliveryFailure>;
}

/// Build the shared HTTP client for channel deliveries.
///
/// `Err` on failure (TLS backend init, ...): the caller propagates instead of
/// falling back to a silently different client or panicking.
fn build_notification_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build notification HTTP client")
}

/// Endpoint link for an external payload, when a deployment base URL is set.
///
/// `PendingAlert` carries no URL, so the link is derived from the documented
/// `EMAIL_DIGEST_BASE_URL`; when unset the JSON field is `null` and the Teams
/// action is omitted.
fn alert_link(alert: &PendingAlert) -> Option<String> {
    let base = std::env::var("EMAIL_DIGEST_BASE_URL").ok()?;
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    Some(format!("{base}/warnings/{}", alert.source_id))
}

/// Teams theme colour per severity.
fn severity_theme_color(severity: crate::notifications::AlertSeverity) -> &'static str {
    use crate::notifications::AlertSeverity;
    match severity {
        AlertSeverity::Critical => "FF0000",
        AlertSeverity::High => "FF8C00",
        AlertSeverity::Medium => "FFD700",
        AlertSeverity::Low => "36A64F",
        AlertSeverity::Info => "808080",
    }
}

/// Render the Microsoft Teams `MessageCard` payload for an alert.
fn teams_message_card(alert: &PendingAlert) -> String {
    let mut card = serde_json::json!({
        "@type": "MessageCard",
        "@context": "http://schema.org/extensions",
        "themeColor": severity_theme_color(alert.severity),
        "summary": alert.title,
        "title": format!("[{}] {}", alert.severity.as_str().to_uppercase(), alert.title),
        "text": alert.llm_narrative.as_deref().unwrap_or(&alert.body),
        "sections": [{
            "facts": [
                { "name": "Entity", "value": alert.display_name() },
                { "name": "Category", "value": alert.category },
                { "name": "Region", "value": alert.region.as_deref().unwrap_or("Global") },
                { "name": "Severity", "value": alert.severity.as_str() },
                { "name": "Priority", "value": format!("{:.2}", alert.priority_score) },
            ]
        }]
    });
    if let Some(link) = alert_link(alert) {
        card["potentialAction"] = serde_json::json!([{
            "@type": "OpenUri",
            "name": "View in ApexIntel",
            "targets": [{ "os": "default", "uri": link }]
        }]);
    }
    card.to_string()
}

/// Render the generic JSON payload: `{title, severity, description, entity, link}`.
fn generic_json_body(alert: &PendingAlert) -> String {
    serde_json::json!({
        "title": alert.title,
        "severity": alert.severity.as_str(),
        "description": alert.llm_narrative.as_deref().unwrap_or(&alert.body),
        "entity": alert.display_name(),
        "link": alert_link(alert),
    })
    .to_string()
}

/// Render an alert body for one webhook format (audit #72).
fn render_webhook_body(format: WebhookFormat, alert: &PendingAlert) -> String {
    match format {
        WebhookFormat::Slack => crate::notifications::format_slack_message(alert),
        WebhookFormat::Teams => teams_message_card(alert),
        WebhookFormat::Json => generic_json_body(alert),
    }
}

/// SMTP transport settings, validated and built once per process.
///
/// Parsed from the same `ALERT_SMTP_*` variables the legacy dispatcher read,
/// so existing deployments keep working (audit #74). A non-loopback host
/// requires TLS; plaintext is allowed only with an explicit
/// `ALERT_SMTP_ALLOW_PLAINTEXT=1` **and** no credentials, because basic-auth
/// credentials over an unencrypted connection leak.
#[derive(Clone)]
struct SmtpSettings {
    host: String,
    port: u16,
    user: String,
    pass: String,
    starttls: bool,
}

/// `Debug` must never print the SMTP password.
impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field(
                "pass",
                &if self.pass.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("starttls", &self.starttls)
            .finish()
    }
}

/// Built once per process; the `Result` is cached so a misconfiguration is not
/// re-parsed (or worse, silently changed) per delivery.
static SMTP: OnceLock<Result<SmtpSettings, String>> = OnceLock::new();

fn smtp_settings() -> Result<&'static SmtpSettings, DeliveryFailure> {
    match SMTP.get_or_init(SmtpSettings::from_env) {
        Ok(settings) => Ok(settings),
        Err(error) => Err(DeliveryFailure::Retryable(format!(
            "SMTP configuration error: {error}"
        ))),
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

impl SmtpSettings {
    fn from_env() -> Result<Self, String> {
        let host = std::env::var("ALERT_SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("ALERT_SMTP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(25);
        let user = std::env::var("ALERT_SMTP_USER").unwrap_or_default();
        let pass = std::env::var("ALERT_SMTP_PASS").unwrap_or_default();
        let starttls = env_flag("ALERT_SMTP_STARTTLS");
        let allow_plaintext = env_flag("ALERT_SMTP_ALLOW_PLAINTEXT");
        let has_credentials = !user.trim().is_empty();

        if allow_plaintext && !starttls && has_credentials {
            return Err(
                "ALERT_SMTP_ALLOW_PLAINTEXT=1 cannot be combined with SMTP credentials; \
                 remove ALERT_SMTP_USER or enable ALERT_SMTP_STARTTLS"
                    .to_string(),
            );
        }
        if !is_loopback_host(&host) && !starttls && !allow_plaintext {
            return Err(format!(
                "refusing plaintext SMTP to non-loopback host '{host}': set \
                 ALERT_SMTP_STARTTLS=1, or ALERT_SMTP_ALLOW_PLAINTEXT=1 with no credentials"
            ));
        }

        Ok(Self {
            host,
            port,
            user,
            pass,
            starttls,
        })
    }

    fn mailer(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
        let builder = if self.starttls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)
                .map_err(|error| format!("SMTP relay error: {error}"))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.host)
        };
        let mut builder = builder
            .port(self.port)
            .timeout(Some(Duration::from_secs(SMTP_TIMEOUT_SECS)));
        if !self.user.trim().is_empty() {
            builder = builder.credentials(Credentials::new(self.user.clone(), self.pass.clone()));
        }
        Ok(builder.build())
    }
}

/// Production transport: dispatches to webhook and email channels from the
/// environment-derived [`NotificationConfig`].
#[derive(Debug)]
pub struct ConfiguredChannelRouter {
    config: NotificationConfig,
    http: reqwest::Client,
}

impl ConfiguredChannelRouter {
    /// Build the production router.
    ///
    /// A client that cannot be built (TLS backend init failure, ...) is an
    /// `Err` the caller must handle; this constructor never panics.
    pub fn new(config: NotificationConfig) -> Result<Self> {
        Self::with_http_client(config, build_notification_http_client())
    }

    /// Testable seam: the client build result is injected so the failure path
    /// is provably an `Err` rather than a panic.
    fn with_http_client(config: NotificationConfig, http: Result<reqwest::Client>) -> Result<Self> {
        Ok(Self {
            config,
            http: http.context("failed to build notification HTTP client")?,
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(NotificationConfig::from_env())
    }

    /// The channel destinations that may receive **this** alert.
    ///
    /// Per-channel `min_severity` / `min_priority` thresholds are applied here
    /// (audit #71): the previous `channels()` enqueued every configured
    /// endpoint for every alert, which paged the critical-only hook with High
    /// reminders and bypassed the email threshold.
    ///
    /// The stored `destination` is the channel **name** (or the recipient
    /// address for email), never the secret URL: the delivery worker resolves
    /// the current webhook by name and posts to its freshly looked-up URL
    /// (audit #70/#73). Email rows are one per recipient, so a single bad
    /// address cannot fail the others (audit #74).
    pub fn channels_for(&self, alert: &PendingAlert) -> Vec<DeliveryChannel> {
        let mut out: Vec<_> = self
            .config
            .webhooks
            .iter()
            .filter(|w| alert.severity >= w.min_severity && alert.priority_score >= w.min_priority)
            .map(|w| DeliveryChannel {
                channel: w.name.clone(),
                destination: w.name.clone(),
            })
            .collect();
        if let Some(email) =
            self.config.email.as_ref().filter(|e| {
                alert.severity >= e.min_severity && alert.priority_score >= e.min_priority
            })
        {
            out.extend(
                email
                    .to_addresses
                    .iter()
                    .map(|to| to.trim())
                    .filter(|to| !to.is_empty())
                    .map(|to| DeliveryChannel {
                        channel: "email".to_string(),
                        destination: to.to_string(),
                    }),
            );
        }
        out
    }

    /// The transport body for a delivery: the pre-rendered body when present,
    /// otherwise rendered from the alert for the channel. This keeps every
    /// persisted row deliverable instead of sending an empty payload.
    fn delivery_body(&self, delivery: &NotificationDelivery) -> String {
        if !delivery.payload.body.trim().is_empty() {
            return delivery.payload.body.clone();
        }
        if delivery.channel.eq_ignore_ascii_case("email") {
            crate::notifications::format_email_body(&delivery.payload.alert)
        } else {
            crate::notifications::format_slack_message(&delivery.payload.alert)
        }
    }

    /// The transport body for a specific webhook, honoring its configured
    /// wire format (Slack Block Kit, Teams MessageCard, generic JSON).
    fn webhook_body(&self, webhook: &WebhookConfig, delivery: &NotificationDelivery) -> String {
        if !delivery.payload.body.trim().is_empty() {
            return delivery.payload.body.clone();
        }
        render_webhook_body(webhook.format, &delivery.payload.alert)
    }

    async fn deliver_webhook(
        &self,
        webhook: &WebhookConfig,
        delivery: &NotificationDelivery,
    ) -> Result<(), DeliveryFailure> {
        let mut request = self
            .http
            // Post to the freshly resolved configuration URL, not the URL
            // captured at enqueue time: a rotated/revoked webhook must take
            // effect on the next attempt (audit #70). New rows store the
            // channel name in `destination`, so no secret lives in the DB.
            .post(&webhook.url)
            .header("Content-Type", "application/json")
            // Stable delivery identity: identical across every attempt for the
            // same event/channel/destination/payload, so a redelivery after a
            // lost settlement is deduplicated by the receiver.
            .header("Idempotency-Key", &delivery.idempotency_key)
            .header("X-Apex-Delivery-Key", &delivery.idempotency_key)
            // The attempt number is observability metadata, never identity.
            .header("X-Apex-Attempt", delivery.attempts.max(1).to_string())
            .header("X-Apex-Notification-Event", delivery.delivery_key.as_str())
            .body(self.webhook_body(webhook, delivery));
        if let Some(ref token) = webhook.bearer_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|error| {
            // reqwest's Display embeds the request URL, which is the webhook
            // credential; `without_url()` keeps it out of `last_error`.
            DeliveryFailure::Retryable(format!("webhook transport error: {}", error.without_url()))
        })?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status().as_u16();
        let text = match response.text().await {
            Ok(text) => text,
            Err(error) => {
                // Transport read failure: report the status, not a silent
                // empty body.
                tracing::debug!(
                    error = %error.without_url(),
                    "webhook error response body unreadable"
                );
                String::new()
            }
        };
        let message = format!("webhook returned {status}: {text}");
        match classify_http_status(status) {
            DeliveryDisposition::Retryable => Err(DeliveryFailure::Retryable(message)),
            DeliveryDisposition::Permanent => Err(DeliveryFailure::Permanent(message)),
        }
    }

    async fn deliver_email(
        &self,
        config: &EmailConfig,
        delivery: &NotificationDelivery,
    ) -> Result<(), DeliveryFailure> {
        // One row carries one recipient (new rows) or a legacy comma-joined
        // list. Empty/whitespace entries are dropped, and each recipient gets
        // its own message so no recipient sees the others' addresses and one
        // bad mailbox cannot fail the whole group (audit #74).
        let recipients: Vec<String> = delivery
            .destination
            .split(',')
            .map(|to| to.trim().to_string())
            .filter(|to| !to.is_empty())
            .collect();
        if recipients.is_empty() {
            return Err(DeliveryFailure::Permanent(
                "email delivery has no recipient".to_string(),
            ));
        }

        let subject = delivery.payload.subject.clone().unwrap_or_else(|| {
            format!(
                "{} [{}] {}",
                config.subject_prefix,
                delivery.payload.alert.severity.as_str().to_uppercase(),
                delivery.payload.alert.title
            )
        });
        let from = config.from_address.parse::<Mailbox>().map_err(|error| {
            DeliveryFailure::Permanent(format!("invalid from address: {error}"))
        })?;
        let body = self.delivery_body(delivery);

        // Settings are validated and cached once per process; a misconfigured
        // relay is retryable so a fix + restart can still deliver the backlog.
        let settings = smtp_settings()?;
        let mailer = settings.mailer().map_err(|error| {
            DeliveryFailure::Retryable(format!("SMTP configuration error: {error}"))
        })?;

        for to in &recipients {
            let message = Message::builder()
                .from(from.clone())
                .subject(subject.clone())
                .to(to.parse::<Mailbox>().map_err(|error| {
                    DeliveryFailure::Permanent(format!("invalid to address: {error}"))
                })?)
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(body.clone()),
                )
                .map_err(|error| DeliveryFailure::Permanent(format!("invalid email: {error}")))?;

            if let Err(error) = mailer.send(message).await {
                let message = format!("SMTP delivery failed: {error}");
                return match classify_smtp_permanent(error.is_permanent()) {
                    DeliveryDisposition::Retryable => Err(DeliveryFailure::Retryable(message)),
                    DeliveryDisposition::Permanent => Err(DeliveryFailure::Permanent(message)),
                };
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelTransport for ConfiguredChannelRouter {
    async fn deliver(&self, delivery: &NotificationDelivery) -> Result<(), DeliveryFailure> {
        if delivery.channel.eq_ignore_ascii_case("email") {
            let Some(config) = self.config.email.as_ref() else {
                return Err(DeliveryFailure::Permanent(
                    "email channel is no longer configured".to_string(),
                ));
            };
            return self.deliver_email(config, delivery).await;
        }

        // Webhook channel names come from the configured webhooks; match by
        // name first, then by destination URL (config may have been renamed).
        let webhook = self
            .config
            .webhooks
            .iter()
            .find(|webhook| webhook.name == delivery.channel)
            .or_else(|| {
                self.config
                    .webhooks
                    .iter()
                    .find(|webhook| webhook.url == delivery.destination)
            });
        match webhook {
            Some(webhook) => self.deliver_webhook(webhook, delivery).await,
            // Legacy rows stored the secret URL as the destination; redact it
            // so it never lands in `last_error` (audit #73).
            None => Err(DeliveryFailure::Permanent(format!(
                "channel '{}' is no longer configured for destination {}",
                delivery.channel,
                redact_url(&delivery.destination)
            ))),
        }
    }
}

/// Enqueue helper: persist the domain alert + outbox event + per-channel rows.
#[async_trait]
pub trait NotificationEnqueuer: Send + Sync {
    async fn enqueue_alert(
        &self,
        alert: &PendingAlert,
        channels: &[DeliveryChannel],
    ) -> Result<EnqueueOutcome>;
}

/// Result of enqueuing one alert.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnqueueOutcome {
    pub already_enqueued: bool,
    pub deliveries_enqueued: usize,
}

/// Build the durable notification event for a pending alert.
///
/// Serialization failures are errors: a fabricated empty payload (`{}`) must
/// never be enqueued, because it would dedupe under the real identity while
/// carrying no content, and the channel delivery would send an empty body.
pub fn notification_event_for(alert: &PendingAlert) -> Result<NewNotificationEvent> {
    let event = alert.to_alert_event();
    let (payload, outbox_payload) = serialize_notification_payloads(alert, &event)?;
    Ok(NewNotificationEvent {
        dedupe_key: alert.source_id.clone(),
        source_type: alert.category.clone(),
        source_id: alert.source_id.clone(),
        severity: alert.severity.as_str().to_string(),
        category: alert.category.clone(),
        title: alert.title.clone(),
        body: alert.body.clone(),
        payload,
        outbox_aggregate_type: if alert.category.starts_with("sla_") {
            "sla_alert".to_string()
        } else {
            "alert".to_string()
        },
        outbox_aggregate_id: event.id,
        outbox_event_type: event.event_type.as_str().to_string(),
        outbox_payload,
    })
}

/// Serialize the domain alert and its outbox event, surfacing any serialization
/// failure instead of substituting an empty object.
fn serialize_notification_payloads<A, E>(
    alert: &A,
    event: &E,
) -> Result<(serde_json::Value, serde_json::Value)>
where
    A: serde::Serialize + ?Sized,
    E: serde::Serialize + ?Sized,
{
    let payload = serde_json::to_value(alert)
        .context("failed to serialize the notification alert payload")?;
    let outbox_payload = serde_json::to_value(event)
        .context("failed to serialize the notification outbox payload")?;
    Ok((payload, outbox_payload))
}

#[async_trait]
impl NotificationEnqueuer for PgStore {
    async fn enqueue_alert(
        &self,
        alert: &PendingAlert,
        channels: &[DeliveryChannel],
    ) -> Result<EnqueueOutcome> {
        let event = notification_event_for(alert)?;
        let outcome = self.enqueue_notification_event(&event, channels).await?;
        Ok(EnqueueOutcome {
            already_enqueued: outcome.already_enqueued,
            deliveries_enqueued: outcome.deliveries_enqueued,
        })
    }
}

/// Summary of enqueuing a batch of SLA alerts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlaEnqueueSummary {
    pub alerts_enqueued: usize,
    pub alerts_already_enqueued: usize,
    pub channel_deliveries: usize,
}

/// Enqueue SLA breach/reminder alerts into the same durable pipeline as other
/// alerts: domain event + alert outbox row + per-channel delivery rows.
pub async fn enqueue_sla_alerts(
    store: &dyn NotificationEnqueuer,
    alerts: Vec<PendingAlert>,
    channels: &[DeliveryChannel],
) -> Result<SlaEnqueueSummary> {
    let mut summary = SlaEnqueueSummary::default();
    for alert in alerts {
        let outcome = store
            .enqueue_alert(&alert, channels)
            .await
            .with_context(|| {
                format!("failed to enqueue notification event '{}'", alert.source_id)
            })?;
        if outcome.already_enqueued {
            summary.alerts_already_enqueued += 1;
        } else {
            summary.alerts_enqueued += 1;
            summary.channel_deliveries += outcome.deliveries_enqueued;
        }
    }
    Ok(summary)
}

/// Run one retry-processor cycle: claim due rows, attempt outside any
/// transaction, settle.
///
/// Rows are processed up to [`DELIVERY_CONCURRENCY`] at a time. Sequential
/// processing let a slow destination outlive the 120s claim lease, after which
/// another worker re-sent the same delivery (audit #76). Per-row error
/// handling and state updates are unchanged; only the counters are shared.
pub async fn process_due_notifications(
    store: &dyn DeliveryClaimStore,
    transport: &dyn ChannelTransport,
    owner: &str,
    limit: i64,
    now: DateTime<Utc>,
) -> Result<DeliveryCycleOutcome> {
    let claimed = store.claim_due_deliveries(owner, limit).await?;
    let outcome = std::sync::Mutex::new(DeliveryCycleOutcome {
        claimed: claimed.len(),
        ..DeliveryCycleOutcome::default()
    });
    let outcome = &outcome;

    futures::stream::iter(claimed)
        .for_each_concurrent(DELIVERY_CONCURRENCY, |row| async move {
            let delivery = match NotificationDelivery::from_row(&row) {
                Ok(delivery) => delivery,
                Err(failure) => {
                    if settle_dead_letter(store, owner, &row.delivery_key, &failure).await {
                        record(outcome, |counts| counts.dead_lettered += 1);
                    }
                    return;
                }
            };

            let result = transport.deliver(&delivery).await;
            match result {
                Ok(()) => match store.mark_delivered(owner, &delivery.delivery_key).await {
                    Ok(true) => record(outcome, |counts| counts.delivered += 1),
                    Ok(false) => warn!(
                        delivery_key = %delivery.delivery_key,
                        "notification delivery: lease lost before settlement; another worker owns it"
                    ),
                    Err(error) => warn!(
                        delivery_key = %delivery.delivery_key,
                        error = %error,
                        "notification delivery: failed to record delivery; lease will expire and retry"
                    ),
                },
                Err(failure) => {
                    let max_attempts = max_attempts_for_channel(&delivery.channel);
                    if !failure.is_retryable() || delivery.attempts >= max_attempts {
                        if settle_dead_letter(store, owner, &delivery.delivery_key, &failure).await
                        {
                            record(outcome, |counts| counts.dead_lettered += 1);
                        }
                    } else {
                        let delay = retry_delay_secs(delivery.attempts, clock_jitter_fraction());
                        let next_retry_at = now + chrono::Duration::seconds(delay);
                        match store
                            .mark_retry(
                                owner,
                                &delivery.delivery_key,
                                next_retry_at,
                                failure.message(),
                            )
                            .await
                        {
                            Ok(true) => {
                                record(outcome, |counts| counts.retried += 1);
                                debug!(
                                    delivery_key = %delivery.delivery_key,
                                    attempt = delivery.attempts,
                                    retry_in_secs = delay,
                                    error = %failure,
                                    "notification delivery: retry scheduled"
                                );
                            }
                            Ok(false) => warn!(
                                delivery_key = %delivery.delivery_key,
                                "notification delivery: lease lost before retry settlement"
                            ),
                            Err(error) => warn!(
                                delivery_key = %delivery.delivery_key,
                                error = %error,
                                "notification delivery: failed to schedule retry"
                            ),
                        }
                    }
                }
            }
        })
        .await;

    let outcome = *outcome
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(outcome)
}

/// Apply a counter update, recovering from a poisoned lock.
fn record(
    outcome: &std::sync::Mutex<DeliveryCycleOutcome>,
    update: impl FnOnce(&mut DeliveryCycleOutcome),
) {
    let mut counts = outcome
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update(&mut counts);
}

/// Settle a permanent failure as a dead letter.
///
/// Returns `true` when the dead-letter state was recorded (the caller then
/// increments the cycle counter); a lost lease or a write error returns
/// `false` after logging, exactly as before the concurrent refactor.
async fn settle_dead_letter(
    store: &dyn DeliveryClaimStore,
    owner: &str,
    delivery_key: &str,
    failure: &DeliveryFailure,
) -> bool {
    match store
        .mark_dead_lettered(owner, delivery_key, failure.message())
        .await
    {
        Ok(true) => {
            warn!(
                delivery_key = %delivery_key,
                error = %failure,
                "notification delivery: dead-lettered; operator replay required"
            );
            true
        }
        Ok(false) => {
            warn!(
                delivery_key = %delivery_key,
                "notification delivery: lease lost before dead-letter settlement"
            );
            false
        }
        Err(error) => {
            warn!(
                delivery_key = %delivery_key,
                error = %error,
                "notification delivery: failed to record dead-letter state"
            );
            false
        }
    }
}

/// Claim owner identity for the retry processor.
pub fn delivery_claim_owner() -> String {
    let instance = std::env::var("APEX_INSTANCE_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "worker".to_string());
    format!("notification-delivery:{instance}:{}", std::process::id())
}

/// Log a one-line cycle summary at info level when anything happened.
pub fn log_cycle(outcome: &DeliveryCycleOutcome) {
    if outcome.delivered > 0 || outcome.retried > 0 || outcome.dead_lettered > 0 {
        info!(
            claimed = outcome.claimed,
            delivered = outcome.delivered,
            retried = outcome.retried,
            dead_lettered = outcome.dead_lettered,
            "notification delivery: cycle complete"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::Mutex;

    use apex_store::postgres::NotificationDeliveryRow;
    use chrono::TimeZone;

    fn alert(source: &str, category: &str) -> PendingAlert {
        let mut alert = PendingAlert::new(
            source,
            apex_core::alert_config::AlertScope::entity("entity-1", Some("Acme")),
            "SLA BREACH — critical warning",
            "warning breached SLA",
            crate::notifications::AlertSeverity::Critical,
            0.95,
        );
        alert.category = category.to_string();
        alert
    }

    fn delivery_row(key: &str, attempts: i32, payload: &PendingAlert) -> NotificationDeliveryRow {
        let payload_value = serde_json::to_value(NotificationDeliveryPayload {
            alert: payload.clone(),
            subject: Some("subject".to_string()),
            body: "{}".to_string(),
        })
        .unwrap();
        NotificationDeliveryRow {
            delivery_key: key.to_string(),
            notification_event_id: Some(Uuid::new_v4()),
            channel: "webhook".to_string(),
            destination: "https://example.test/hook".to_string(),
            payload: payload_value.clone(),
            payload_hash: Some(notification_payload_hash(&payload_value)),
            status: "delivering".to_string(),
            attempts,
            next_retry_at: None,
            lease_owner: None,
            lease_until: None,
            last_error: None,
            dead_lettered_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[derive(Default)]
    struct FakeStore {
        queued: Mutex<Vec<NotificationDeliveryRow>>,
        delivered: Mutex<Vec<String>>,
        retries: Mutex<Vec<(String, DateTime<Utc>, String)>>,
        dead_letters: Mutex<Vec<(String, String)>>,
        claimed: Mutex<usize>,
    }

    impl FakeStore {
        fn with_rows(rows: Vec<NotificationDeliveryRow>) -> Self {
            Self {
                queued: Mutex::new(rows),
                ..Self::default()
            }
        }

        fn retry_count(&self) -> usize {
            self.retries.lock().unwrap().len()
        }

        fn next_retry_in_secs(&self, now: DateTime<Utc>) -> Option<i64> {
            self.retries
                .lock()
                .unwrap()
                .first()
                .map(|(_, next, _)| (*next - now).num_seconds())
        }
    }

    #[async_trait]
    impl DeliveryClaimStore for FakeStore {
        async fn claim_due_deliveries(
            &self,
            _owner: &str,
            limit: i64,
        ) -> Result<Vec<NotificationDeliveryRow>> {
            let mut queued = self.queued.lock().unwrap();
            let take = (limit as usize).min(queued.len());
            let claimed: Vec<_> = queued.drain(..take).collect();
            *self.claimed.lock().unwrap() += claimed.len();
            Ok(claimed)
        }

        async fn mark_delivered(&self, _owner: &str, delivery_key: &str) -> Result<bool> {
            self.delivered
                .lock()
                .unwrap()
                .push(delivery_key.to_string());
            Ok(true)
        }

        async fn mark_retry(
            &self,
            _owner: &str,
            delivery_key: &str,
            next_retry_at: DateTime<Utc>,
            error: &str,
        ) -> Result<bool> {
            self.retries.lock().unwrap().push((
                delivery_key.to_string(),
                next_retry_at,
                error.to_string(),
            ));
            Ok(true)
        }

        async fn mark_dead_lettered(
            &self,
            _owner: &str,
            delivery_key: &str,
            error: &str,
        ) -> Result<bool> {
            self.dead_letters
                .lock()
                .unwrap()
                .push((delivery_key.to_string(), error.to_string()));
            Ok(true)
        }
    }

    /// A transport whose behavior per channel is scripted (FIFO).
    #[derive(Default)]
    struct FakeTransport {
        results: Mutex<Vec<Result<(), DeliveryFailure>>>,
        delivered: Mutex<Vec<NotificationDelivery>>,
    }

    impl FakeTransport {
        fn always(failure: DeliveryFailure) -> Self {
            Self {
                results: Mutex::new(vec![Err(failure)]),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl ChannelTransport for FakeTransport {
        async fn deliver(&self, delivery: &NotificationDelivery) -> Result<(), DeliveryFailure> {
            self.delivered.lock().unwrap().push(delivery.clone());
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                return Ok(());
            }
            results.remove(0)
        }
    }

    #[test]
    fn backoff_is_exponential_capped_and_jittered() {
        // attempt 1: 30s, attempt 2: 60s, attempt 3: 120s (no jitter: half)
        assert_eq!(retry_delay_secs(1, 0.0), 15);
        assert_eq!(retry_delay_secs(2, 0.0), 30);
        assert_eq!(retry_delay_secs(3, 0.0), 60);
        // full jitter doubles the delay at the same attempt
        assert_eq!(retry_delay_secs(3, 0.999_999), 120);
        // capped at MAX_RETRY_DELAY_SECS
        assert!(retry_delay_secs(30, 0.999_999) <= MAX_RETRY_DELAY_SECS);
        // monotonic across attempts
        assert!(retry_delay_secs(4, 0.0) >= retry_delay_secs(3, 0.0));
    }

    #[test]
    fn http_classification_retries_transient_and_dead_letters_client_errors() {
        assert_eq!(classify_http_status(408), DeliveryDisposition::Retryable);
        assert_eq!(classify_http_status(429), DeliveryDisposition::Retryable);
        assert_eq!(classify_http_status(500), DeliveryDisposition::Retryable);
        assert_eq!(classify_http_status(503), DeliveryDisposition::Retryable);
        assert_eq!(classify_http_status(400), DeliveryDisposition::Permanent);
        assert_eq!(classify_http_status(404), DeliveryDisposition::Permanent);
        assert_eq!(classify_http_status(422), DeliveryDisposition::Permanent);
    }

    #[test]
    fn channel_attempt_budgets_differ_by_channel() {
        assert_eq!(max_attempts_for_channel("email"), EMAIL_MAX_ATTEMPTS);
        assert_eq!(max_attempts_for_channel("slack"), MAX_DELIVERY_ATTEMPTS);
        assert_eq!(max_attempts_for_channel("EMAIL"), EMAIL_MAX_ATTEMPTS);
    }

    #[test]
    fn idempotency_key_is_stable_across_attempts_and_identity_sensitive() {
        let event_id = Uuid::new_v4();
        let key = transport_idempotency_key(Some(event_id), "email", "a@b.c", Some("hash"));
        // Stable across attempts: the same delivery reclaimed after a lost
        // settlement must arrive with an identical key, so the receiver can
        // deduplicate it.
        assert_eq!(
            key,
            transport_idempotency_key(Some(event_id), "email", "a@b.c", Some("hash"))
        );
        assert_eq!(key.len(), 64);
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "slack", "a@b.c", Some("hash"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "email", "x@y.z", Some("hash"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "email", "a@b.c", Some("other"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(None, "email", "a@b.c", Some("hash"))
        );
    }

    #[test]
    fn delivery_body_renders_from_the_alert_when_absent() {
        let router =
            ConfiguredChannelRouter::new(NotificationConfig::default()).expect("client builds");
        let alert = alert("sla-breach:w1", "sla_breach");
        let delivery = NotificationDelivery {
            delivery_key: "key-1".to_string(),
            notification_event_id: Some(Uuid::new_v4()),
            channel: "webhook".to_string(),
            destination: "https://example.test/hook".to_string(),
            attempts: 2,
            idempotency_key: "stable".to_string(),
            payload: NotificationDeliveryPayload {
                alert: alert.clone(),
                subject: None,
                body: String::new(),
            },
        };

        // The enqueue path stores only the alert; the router must render a
        // real body rather than sending an empty payload.
        let rendered = router.delivery_body(&delivery);
        assert!(rendered.contains(&alert.title));
        assert!(serde_json::from_str::<serde_json::Value>(&rendered).is_ok());

        let email = NotificationDelivery {
            channel: "email".to_string(),
            ..delivery.clone()
        };
        assert!(router.delivery_body(&email).contains(&alert.body));

        let pre_rendered = NotificationDelivery {
            payload: NotificationDeliveryPayload {
                alert,
                subject: Some("subject".to_string()),
                body: "{\"custom\":true}".to_string(),
            },
            ..delivery
        };
        assert_eq!(router.delivery_body(&pre_rendered), "{\"custom\":true}");
    }

    #[tokio::test]
    async fn email_delivery_without_a_recipient_is_permanently_rejected() {
        // Audit #74: empty/whitespace recipients are dropped before any SMTP
        // work; a row with no usable recipient can never succeed, so it must
        // dead-letter instead of retrying forever.
        let router =
            ConfiguredChannelRouter::new(NotificationConfig::default()).expect("client builds");
        let email = EmailConfig {
            to_addresses: Vec::new(),
            from_address: "alerts@apexintel.io".to_string(),
            subject_prefix: "[ApexIntel Alert]".to_string(),
            min_severity: crate::notifications::AlertSeverity::Low,
            min_priority: 0.0,
        };
        let delivery = NotificationDelivery {
            delivery_key: "key-email".to_string(),
            notification_event_id: Some(Uuid::new_v4()),
            channel: "email".to_string(),
            destination: " ,  ".to_string(),
            attempts: 1,
            idempotency_key: "stable".to_string(),
            payload: NotificationDeliveryPayload {
                alert: alert("sla-breach:w1", "sla_breach"),
                subject: None,
                body: String::new(),
            },
        };

        match router.deliver_email(&email, &delivery).await {
            Err(DeliveryFailure::Permanent(message)) => assert!(
                message.contains("no recipient"),
                "the failure must name the missing recipient, got: {message}"
            ),
            other => panic!("empty recipients must be a permanent failure, got {other:?}"),
        }
    }

    #[test]
    fn constructor_failure_is_an_err_not_a_panic() {
        let config = NotificationConfig::default();
        let error = ConfiguredChannelRouter::with_http_client(
            config.clone(),
            Err(anyhow::anyhow!("tls backend init failed")),
        )
        .expect_err("a client build failure must surface as Err");
        assert!(
            error.to_string().contains("notification HTTP client"),
            "the error must name the client build failure, got: {error}"
        );

        let router = ConfiguredChannelRouter::new(config).expect("client builds");
        assert!(router.channels_for(&alert("src", "warning")).is_empty());
    }

    fn alert_with(severity: crate::notifications::AlertSeverity, priority: f64) -> PendingAlert {
        let mut alert = alert("src-1", "warning");
        alert.severity = severity;
        alert.priority_score = priority;
        alert
    }

    #[test]
    fn channels_for_applies_per_channel_thresholds() {
        use crate::notifications::{AlertSeverity, EmailConfig, WebhookFormat};

        let config = NotificationConfig {
            webhooks: vec![
                WebhookConfig::slack("https://hooks.slack.com/services/T00/B00/secret"),
                WebhookConfig::critical_only("https://pager.example.test/hook", "critical-hook"),
            ],
            email: Some(EmailConfig {
                to_addresses: vec![
                    "oncall@example.test".to_string(),
                    "  ".to_string(),
                    "lead@example.test".to_string(),
                ],
                from_address: "alerts@apexintel.io".to_string(),
                subject_prefix: "[ApexIntel Alert]".to_string(),
                min_severity: AlertSeverity::High,
                min_priority: 0.65,
            }),
            log_min_severity: AlertSeverity::Low,
        };
        let router = ConfiguredChannelRouter::new(config).expect("client builds");

        // Below every threshold: nobody is paged.
        assert!(router
            .channels_for(&alert_with(AlertSeverity::Low, 0.1))
            .is_empty());

        // High/0.7: the Slack hook and email pass; the critical-only paging
        // hook must NOT receive a High reminder (audit #71).
        let channels = router.channels_for(&alert_with(AlertSeverity::High, 0.7));
        assert_eq!(
            channels
                .iter()
                .map(|c| c.channel.as_str())
                .collect::<Vec<_>>(),
            vec!["slack", "email", "email"]
        );
        assert!(!channels.iter().any(|c| c.channel == "critical-hook"));

        // Critical/0.95: the paging hook is included.
        let channels = router.channels_for(&alert_with(AlertSeverity::Critical, 0.95));
        assert!(channels.iter().any(|c| c.channel == "critical-hook"));

        // Destinations are channel names / recipient addresses, never URLs.
        for channel in &channels {
            assert!(
                !channel.destination.contains("https://"),
                "a secret URL must never be stored as a destination: {}",
                channel.destination
            );
        }
        // ...and empty recipients never become delivery rows.
        assert_eq!(channels.iter().filter(|c| c.channel == "email").count(), 2);

        // The webhook's format survives configuration construction.
        assert_eq!(router.config.webhooks[0].format, WebhookFormat::Slack);
        assert_eq!(router.config.webhooks[1].format, WebhookFormat::Json);
    }

    #[test]
    fn webhook_formats_render_the_expected_bodies() {
        use crate::notifications::WebhookFormat;

        let alert = alert("sla-breach:w1", "sla_breach");

        let slack = render_webhook_body(WebhookFormat::Slack, &alert);
        let slack: serde_json::Value = serde_json::from_str(&slack).unwrap();
        assert!(slack.get("text").is_some(), "Slack body has fallback text");

        let json = render_webhook_body(WebhookFormat::Json, &alert);
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["title"], serde_json::json!(alert.title));
        assert_eq!(json["severity"], serde_json::json!("critical"));
        assert_eq!(json["entity"], serde_json::json!(alert.display_name()));
        assert!(json.get("text").is_none());

        let teams = render_webhook_body(WebhookFormat::Teams, &alert);
        let teams: serde_json::Value = serde_json::from_str(&teams).unwrap();
        assert_eq!(teams["@type"], serde_json::json!("MessageCard"));
        assert!(teams["sections"].is_array());
    }

    /// The claim lease must outlive the batch's worst case, otherwise another
    /// worker can reclaim a row while the original owner is still sending it
    /// and the channel receives a duplicate (audit #76).
    ///
    /// Every claimed row carries the same `lease_until = now + lease`, so the
    /// last wave of `ceil(batch / concurrency)` sequential waves each taking
    /// the full SMTP timeout must finish inside the lease.
    #[test]
    fn delivery_batch_worst_case_fits_inside_the_claim_lease() {
        let waves = (DEFAULT_DELIVERY_BATCH as usize).div_ceil(DELIVERY_CONCURRENCY);
        let worst_case_secs = waves as u64 * SMTP_TIMEOUT_SECS;
        assert!(
            worst_case_secs as f64 <= DELIVERY_LEASE_SECS,
            "worst-case batch duration {worst_case_secs}s exceeds the {DELIVERY_LEASE_SECS}s \
             claim lease: a slow destination outlives its lease and the delivery is re-sent \
             by another worker (waves={waves}, concurrency={DELIVERY_CONCURRENCY}, \
             batch={DEFAULT_DELIVERY_BATCH}, smtp_timeout={SMTP_TIMEOUT_SECS}s)"
        );
    }

    #[tokio::test]
    async fn claimed_rows_are_processed_concurrently_within_the_cap() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct SlowTransport {
            active: AtomicUsize,
            max_active: AtomicUsize,
        }

        #[async_trait]
        impl ChannelTransport for SlowTransport {
            async fn deliver(
                &self,
                _delivery: &NotificationDelivery,
            ) -> Result<(), DeliveryFailure> {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let rows: Vec<_> = (0..16)
            .map(|index| {
                delivery_row(
                    &format!("key-{index}"),
                    1,
                    &alert(&format!("sla-breach:{index}"), "sla_breach"),
                )
            })
            .collect();
        let store = FakeStore::with_rows(rows);
        let transport = SlowTransport {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
        };

        let outcome = process_due_notifications(&store, &transport, "owner-1", 50, Utc::now())
            .await
            .unwrap();
        assert_eq!(outcome.claimed, 16);
        assert_eq!(outcome.delivered, 16);
        let max_active = transport.max_active.load(Ordering::SeqCst);
        assert!(
            max_active > 1,
            "rows must be attempted concurrently, saw max {max_active}"
        );
        assert!(
            max_active <= DELIVERY_CONCURRENCY,
            "concurrency must stay within the cap, saw {max_active}"
        );
    }

    #[test]
    fn smtp_settings_require_tls_for_non_loopback_hosts() {
        let lock = std::sync::Mutex::new(());
        let _guard = lock.lock().unwrap();
        let keys = [
            "ALERT_SMTP_HOST",
            "ALERT_SMTP_USER",
            "ALERT_SMTP_PASS",
            "ALERT_SMTP_STARTTLS",
            "ALERT_SMTP_ALLOW_PLAINTEXT",
        ];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        // Loopback plaintext is the existing local-relay default.
        assert!(SmtpSettings::from_env().is_ok());

        std::env::set_var("ALERT_SMTP_HOST", "smtp.example.test");
        let error = SmtpSettings::from_env().expect_err("non-loopback plaintext must fail");
        assert!(
            error.contains("STARTTLS"),
            "clear error expected, got: {error}"
        );

        // Explicit plaintext is allowed only without credentials.
        std::env::set_var("ALERT_SMTP_ALLOW_PLAINTEXT", "1");
        assert!(SmtpSettings::from_env().is_ok());
        std::env::set_var("ALERT_SMTP_USER", "relay-user");
        std::env::set_var("ALERT_SMTP_PASS", "relay-pass");
        let error = SmtpSettings::from_env().expect_err("plaintext + credentials must fail");
        assert!(
            error.contains("credentials"),
            "clear error expected, got: {error}"
        );

        // STARTTLS permits credentials on a remote relay.
        std::env::set_var("ALERT_SMTP_STARTTLS", "1");
        let settings = SmtpSettings::from_env().expect("TLS with credentials is valid");
        assert!(settings.starttls);
        assert!(!format!("{settings:?}").contains("relay-pass"));

        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    /// A serializer that always fails, standing in for a payload type that
    /// cannot be represented as JSON.
    struct FailingSerialize;

    impl serde::Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom(
                "intentional serialization failure",
            ))
        }
    }

    #[test]
    fn serialization_failure_is_an_error_not_an_empty_payload() {
        let alert = alert("sla-breach:w1", "sla_breach");
        let event = alert.to_alert_event();

        let failed = serialize_notification_payloads(&FailingSerialize, &event)
            .expect_err("a serialization failure must surface as an error");
        assert!(
            failed.to_string().contains("failed to serialize"),
            "the error must identify the payload serialization, got: {failed}"
        );

        // The healthy path never fabricates an empty payload.
        let built = notification_event_for(&alert).expect("healthy alert serializes");
        assert_ne!(built.payload, serde_json::json!({}));
        assert_ne!(built.outbox_payload, serde_json::json!({}));
        assert_eq!(
            built.payload["source_id"],
            serde_json::json!("sla-breach:w1")
        );
    }

    #[tokio::test]
    async fn retryable_failure_schedules_a_backoff_retry() {
        let now = Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap();
        let row = delivery_row("key-1", 1, &alert("sla-breach:w1", "sla_breach"));
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport::always(DeliveryFailure::Retryable("503".to_string()));

        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();

        assert_eq!(outcome.claimed, 1);
        assert_eq!(outcome.retried, 1);
        assert_eq!(outcome.dead_lettered, 0);
        assert_eq!(store.retry_count(), 1);
        let delay = store.next_retry_in_secs(now).unwrap();
        assert!(
            (15..=30).contains(&delay),
            "retry must be scheduled with backoff+jitter, got {delay}s"
        );
    }

    #[tokio::test]
    async fn retry_then_success_is_recorded_once() {
        let now = Utc::now();
        let row = delivery_row("key-1", 1, &alert("sla-breach:w1", "sla_breach"));
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport {
            results: Mutex::new(vec![
                Err(DeliveryFailure::Retryable("timeout".to_string())),
                Ok(()),
            ]),
            delivered: Mutex::new(Vec::new()),
        };

        // Cycle 1 fails; the row would be requeued by the real store.
        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();
        assert_eq!(outcome.retried, 1);

        let row = delivery_row("key-1", 2, &alert("sla-breach:w1", "sla_breach"));
        store.queued.lock().unwrap().push(row);
        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();
        assert_eq!(outcome.delivered, 1);
        assert_eq!(store.delivered.lock().unwrap().as_slice(), &["key-1"]);
        assert_eq!(transport.delivered.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn permanent_failure_dead_letters_immediately() {
        let now = Utc::now();
        let row = delivery_row("key-1", 1, &alert("sla-breach:w1", "sla_breach"));
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport::always(DeliveryFailure::Permanent("410 gone".to_string()));

        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();
        assert_eq!(outcome.dead_lettered, 1);
        assert_eq!(outcome.retried, 0);
        assert_eq!(store.dead_letters.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn max_attempts_dead_letters_even_for_retryable_failures() {
        let now = Utc::now();
        let row = delivery_row(
            "key-1",
            MAX_DELIVERY_ATTEMPTS,
            &alert("sla-breach:w1", "sla_breach"),
        );
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport::always(DeliveryFailure::Retryable("503".to_string()));

        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();
        assert_eq!(outcome.dead_lettered, 1);
        assert_eq!(outcome.retried, 0);
        assert_eq!(store.retry_count(), 0);
    }

    #[tokio::test]
    async fn invalid_payload_is_dead_lettered_not_retried() {
        let now = Utc::now();
        let mut row = delivery_row("key-1", 1, &alert("sla-breach:w1", "sla_breach"));
        row.payload = serde_json::json!({"unexpected": true});
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport::default();

        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, now)
            .await
            .unwrap();
        assert_eq!(outcome.dead_lettered, 1);
        assert!(transport.delivered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sla_alert_reaches_a_fake_channel_through_the_durable_pipeline() {
        // The full SLA path: enqueue into the domain event + outbox +
        // per-channel rows, then the retry processor delivers it.
        let enqueued: Mutex<Vec<(String, Vec<DeliveryChannel>)>> = Mutex::new(Vec::new());
        struct FakeEnqueuer<'a> {
            rows: &'a Mutex<Vec<(String, Vec<DeliveryChannel>)>>,
        }

        #[async_trait]
        impl NotificationEnqueuer for FakeEnqueuer<'_> {
            async fn enqueue_alert(
                &self,
                alert: &PendingAlert,
                channels: &[DeliveryChannel],
            ) -> Result<EnqueueOutcome> {
                self.rows
                    .lock()
                    .unwrap()
                    .push((alert.source_id.clone(), channels.to_vec()));
                Ok(EnqueueOutcome {
                    already_enqueued: false,
                    deliveries_enqueued: channels.len(),
                })
            }
        }

        let channels = vec![DeliveryChannel {
            channel: "webhook".to_string(),
            destination: "https://example.test/hook".to_string(),
        }];
        let alert = alert("sla-breach:w1", "sla_breach");
        // The AlertEvent that lands in the outbox carries the SLA identity.
        let event = alert.to_alert_event();
        assert_eq!(event.subject(), "alerts.events.new_warning");
        assert_eq!(
            event.metadata.get("category").and_then(|v| v.as_str()),
            Some("sla_breach")
        );

        let summary = enqueue_sla_alerts(
            &FakeEnqueuer { rows: &enqueued },
            vec![alert.clone()],
            &channels,
        )
        .await
        .unwrap();
        assert_eq!(summary.alerts_enqueued, 1);
        assert_eq!(summary.channel_deliveries, 1);
        assert_eq!(enqueued.lock().unwrap()[0].0, "sla-breach:w1");

        // The retry processor then claims the channel row and delivers it.
        let row = delivery_row("sla-breach:w1:webhook", 1, &alert);
        let store = FakeStore::with_rows(vec![row]);
        let transport = FakeTransport::default();
        let outcome = process_due_notifications(&store, &transport, "owner-1", 10, Utc::now())
            .await
            .unwrap();

        assert_eq!(outcome.delivered, 1);
        let delivered = transport.delivered.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].channel, "webhook");
        assert_eq!(delivered[0].payload.alert.source_id, "sla-breach:w1");
        assert_eq!(delivered[0].payload.alert.category, "sla_breach");
        assert!(!delivered[0].idempotency_key.is_empty());
    }

    #[test]
    fn notification_event_identity_is_stable_and_sla_tagged() {
        let alert = alert("sla-reminder:w9", "sla_reminder");
        let first = notification_event_for(&alert).expect("serializes");
        let second = notification_event_for(&alert).expect("serializes");
        assert_eq!(first.dedupe_key, "sla-reminder:w9");
        assert_eq!(first.dedupe_key, second.dedupe_key);
        assert_eq!(first.outbox_aggregate_type, "sla_alert");
        assert_eq!(first.outbox_event_type, "new_warning");
    }
}
