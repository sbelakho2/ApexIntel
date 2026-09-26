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
//!   (crash) is retried; every attempt carries a stable idempotency key built
//!   from (notification_event_id, channel, destination, attempt, payload_hash)
//!   so the channel can deduplicate. Exactly-once is not claimed.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

use crate::notifications::{EmailConfig, NotificationConfig, PendingAlert, WebhookConfig};

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
pub const DEFAULT_DELIVERY_BATCH: i64 = 50;

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

/// Stable per-attempt idempotency key:
/// sha256(notification_event_id | channel | destination | attempt | payload_hash).
pub fn transport_idempotency_key(
    notification_event_id: Option<Uuid>,
    channel: &str,
    destination: &str,
    attempt: i32,
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
    hasher.update(attempt.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(payload_hash.unwrap_or_default().as_bytes());
    hex::encode(hasher.finalize())
}

/// The domain payload persisted for one channel delivery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotificationDeliveryPayload {
    pub alert: PendingAlert,
    pub subject: Option<String>,
    /// Formatted channel body (webhook JSON / email text).
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
            row.attempts.max(0),
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

/// Production transport: dispatches to webhook and email channels from the
/// environment-derived [`NotificationConfig`].
pub struct ConfiguredChannelRouter {
    config: NotificationConfig,
    http: reqwest::Client,
}

impl ConfiguredChannelRouter {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|error| panic!("failed to build reqwest client: {error}")),
        }
    }

    pub fn from_env() -> Self {
        Self::new(NotificationConfig::from_env())
    }

    /// The channel destinations configured for this deployment.
    pub fn channels(&self) -> Vec<DeliveryChannel> {
        let mut channels = Vec::new();
        for webhook in &self.config.webhooks {
            channels.push(DeliveryChannel {
                channel: webhook.name.clone(),
                destination: webhook.url.clone(),
            });
        }
        if let Some(email) = &self.config.email {
            channels.push(DeliveryChannel {
                channel: "email".to_string(),
                destination: email.to_addresses.join(","),
            });
        }
        channels
    }

    async fn deliver_webhook(
        &self,
        webhook: &WebhookConfig,
        delivery: &NotificationDelivery,
    ) -> Result<(), DeliveryFailure> {
        let mut request = self
            .http
            .post(&delivery.destination)
            .header("Content-Type", "application/json")
            // Stable per-attempt idempotency identity (item 5/36).
            .header("Idempotency-Key", &delivery.idempotency_key)
            .header("X-Apex-Notification-Event", delivery.delivery_key.as_str())
            .body(delivery.payload.body.clone());
        if let Some(ref token) = webhook.bearer_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|error| {
            DeliveryFailure::Retryable(format!("webhook transport error: {error}"))
        })?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
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
        let subject = delivery
            .payload
            .subject
            .clone()
            .unwrap_or_else(|| "[ApexIntel Alert]".to_string());
        let builder = Message::builder()
            .from(config.from_address.parse::<Mailbox>().map_err(|error| {
                DeliveryFailure::Permanent(format!("invalid from address: {error}"))
            })?)
            .subject(subject);
        let mut builder = builder;
        for to in &config.to_addresses {
            builder = builder.to(to.parse::<Mailbox>().map_err(|error| {
                DeliveryFailure::Permanent(format!("invalid to address: {error}"))
            })?);
        }
        let message = builder
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(delivery.payload.body.clone()),
            )
            .map_err(|error| DeliveryFailure::Permanent(format!("invalid email: {error}")))?;

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

        let mailer = if smtp_starttls {
            let mut transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)
                .map_err(|error| DeliveryFailure::Retryable(format!("SMTP relay error: {error}")))?
                .port(smtp_port);
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

        match mailer.send(message).await {
            Ok(_) => Ok(()),
            Err(error) => {
                let message = format!("SMTP delivery failed: {error}");
                match classify_smtp_permanent(error.is_permanent()) {
                    DeliveryDisposition::Retryable => Err(DeliveryFailure::Retryable(message)),
                    DeliveryDisposition::Permanent => Err(DeliveryFailure::Permanent(message)),
                }
            }
        }
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
            None => Err(DeliveryFailure::Permanent(format!(
                "channel '{}' is no longer configured for destination {}",
                delivery.channel, delivery.destination
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
pub fn notification_event_for(alert: &PendingAlert) -> NewNotificationEvent {
    let event = alert.to_alert_event();
    let payload = serde_json::to_value(alert).unwrap_or_else(|_| serde_json::json!({}));
    let outbox_payload = serde_json::to_value(&event).unwrap_or_else(|_| serde_json::json!({}));
    NewNotificationEvent {
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
    }
}

#[async_trait]
impl NotificationEnqueuer for PgStore {
    async fn enqueue_alert(
        &self,
        alert: &PendingAlert,
        channels: &[DeliveryChannel],
    ) -> Result<EnqueueOutcome> {
        let event = notification_event_for(alert);
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
pub async fn process_due_notifications(
    store: &dyn DeliveryClaimStore,
    transport: &dyn ChannelTransport,
    owner: &str,
    limit: i64,
    now: DateTime<Utc>,
) -> Result<DeliveryCycleOutcome> {
    let claimed = store.claim_due_deliveries(owner, limit).await?;
    let mut outcome = DeliveryCycleOutcome {
        claimed: claimed.len(),
        ..DeliveryCycleOutcome::default()
    };

    for row in claimed {
        let delivery = match NotificationDelivery::from_row(&row) {
            Ok(delivery) => delivery,
            Err(failure) => {
                settle_dead_letter(store, owner, &row.delivery_key, &failure, &mut outcome).await;
                continue;
            }
        };

        let result = transport.deliver(&delivery).await;
        match result {
            Ok(()) => match store.mark_delivered(owner, &delivery.delivery_key).await {
                Ok(true) => outcome.delivered += 1,
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
                    settle_dead_letter(
                        store,
                        owner,
                        &delivery.delivery_key,
                        &failure,
                        &mut outcome,
                    )
                    .await;
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
                            outcome.retried += 1;
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
    }

    Ok(outcome)
}

async fn settle_dead_letter(
    store: &dyn DeliveryClaimStore,
    owner: &str,
    delivery_key: &str,
    failure: &DeliveryFailure,
    outcome: &mut DeliveryCycleOutcome,
) {
    match store
        .mark_dead_lettered(owner, delivery_key, failure.message())
        .await
    {
        Ok(true) => {
            outcome.dead_lettered += 1;
            warn!(
                delivery_key = %delivery_key,
                error = %failure,
                "notification delivery: dead-lettered; operator replay required"
            );
        }
        Ok(false) => warn!(
            delivery_key = %delivery_key,
            "notification delivery: lease lost before dead-letter settlement"
        ),
        Err(error) => warn!(
            delivery_key = %delivery_key,
            error = %error,
            "notification delivery: failed to record dead-letter state"
        ),
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
            "entity-1",
            "Acme",
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
    fn idempotency_key_is_stable_and_identity_sensitive() {
        let event_id = Uuid::new_v4();
        let key = transport_idempotency_key(Some(event_id), "email", "a@b.c", 1, Some("hash"));
        assert_eq!(
            key,
            transport_idempotency_key(Some(event_id), "email", "a@b.c", 1, Some("hash"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "email", "a@b.c", 2, Some("hash"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "slack", "a@b.c", 1, Some("hash"))
        );
        assert_ne!(
            key,
            transport_idempotency_key(Some(event_id), "email", "a@b.c", 1, Some("other"))
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
        let first = notification_event_for(&alert);
        let second = notification_event_for(&alert);
        assert_eq!(first.dedupe_key, "sla-reminder:w9");
        assert_eq!(first.dedupe_key, second.dedupe_key);
        assert_eq!(first.outbox_aggregate_type, "sla_alert");
        assert_eq!(first.outbox_event_type, "new_warning");
    }
}
