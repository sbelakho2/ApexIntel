//! NATS JetStream publisher for alert events.
//!
//! Provides a [`NatsPublisher`] that connects to NATS, ensures the `alerts`
//! JetStream stream exists, and publishes [`AlertEvent`]s to subjects
//! `alerts.events.{event_type}`.
//!
//! # Stream configuration
//! - Name: `alerts`
//! - Subjects: `alerts.>`
//! - Max age: 7 days
//! - Storage: file
//! - Retention: interest-based (auto-cleanup when consumers acknowledge)

use std::sync::Arc;

use anyhow::{Context, Result};
use apex_core::alert_config::AlertAudience;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

/// Environment variable that makes NATS a required capability
/// (`REQUIRE_NATS=true`).
pub const REQUIRE_NATS_ENV: &str = "REQUIRE_NATS";

/// Environment variable naming the deployment environment; `production`
/// implies NATS is required.
pub const APEX_ENV_ENV: &str = "APEX_ENV";

/// Whether NATS is a required capability for this process.
///
/// Explicit `REQUIRE_NATS` wins; otherwise `APEX_ENV=production` makes NATS
/// required. In a required deployment an unavailable JetStream is a hard
/// failure — [`NatsPublisher::publish_alert`] returns `Err` instead of
/// silently dropping the alert.
pub fn nats_required_from_env() -> bool {
    let flag = std::env::var(REQUIRE_NATS_ENV).ok();
    let apex_env = std::env::var(APEX_ENV_ENV).ok();
    nats_required(flag.as_deref(), apex_env.as_deref())
}

/// Pure resolution of the NATS capability requirement (testable without env).
fn nats_required(flag: Option<&str>, apex_env: Option<&str>) -> bool {
    if let Some(flag) = flag {
        if !flag.trim().is_empty() {
            return apex_core::env::parse_truthy_flag(flag);
        }
    }
    matches!(
        apex_env
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "production" | "prod"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Alert event types
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of alert event being published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEventType {
    NewInsight,
    NewWarning,
    RecipeMatch,
    CompetitorChange,
    SupplyChainRisk,
    SystemAlert,
}

impl AlertEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewInsight => "new_insight",
            Self::NewWarning => "new_warning",
            Self::RecipeMatch => "recipe_match",
            Self::CompetitorChange => "competitor_change",
            Self::SupplyChainRisk => "supply_chain_risk",
            Self::SystemAlert => "system_alert",
        }
    }
}

impl std::fmt::Display for AlertEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An alert event to be published through NATS JetStream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: Uuid,
    pub event_type: AlertEventType,
    pub severity: apex_core::alert_config::AlertSeverity,
    pub title: String,
    pub description: String,
    pub entity_id: Option<Uuid>,
    pub entity_name: Option<String>,
    /// Who this alert is addressed to. `Users(vec![])` addresses nobody and
    /// only a deliberate `Broadcast` reaches every connected user.
    pub audience: AlertAudience,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl AlertEvent {
    /// Build the NATS subject for this event.
    pub fn subject(&self) -> String {
        format!("alerts.events.{}", self.event_type)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NATS Publisher
// ─────────────────────────────────────────────────────────────────────────────

/// A JetStream publish whose broker ACK has **not** been awaited yet.
///
/// Reporting success without [`PendingPublishAck::wait_for_ack`] would claim a
/// delivery that JetStream may never have accepted.
pub trait PendingPublishAck: Send {
    fn wait_for_ack(self: Box<Self>) -> BoxFuture<'static, Result<()>>;
}

/// The publish transport seam, so the ACK-ordering contract is testable
/// without a live NATS server.
#[async_trait]
pub trait JetStreamTransport: Send + Sync {
    /// Start a publish. The returned ACK handle must be awaited before the
    /// caller reports success.
    async fn publish(
        &self,
        subject: String,
        payload: Vec<u8>,
    ) -> Result<Box<dyn PendingPublishAck>>;

    /// Start a publish with headers (used for dead-letter copies). The returned
    /// ACK handle must be awaited before the caller reports success.
    async fn publish_with_headers(
        &self,
        subject: String,
        headers: async_nats::HeaderMap,
        payload: Vec<u8>,
    ) -> Result<Box<dyn PendingPublishAck>>;
}

/// Real transport over an `async_nats` JetStream context.
pub struct NatsJetStreamTransport {
    jetstream: async_nats::jetstream::Context,
}

impl NatsJetStreamTransport {
    pub fn new(jetstream: async_nats::jetstream::Context) -> Self {
        Self { jetstream }
    }
}

struct NatsPendingAck(async_nats::jetstream::context::PublishAckFuture);

impl PendingPublishAck for NatsPendingAck {
    fn wait_for_ack(self: Box<Self>) -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            self.0
                .await
                .map(|_ack| ())
                .map_err(|e| anyhow::anyhow!("NATS JetStream publish ACK failed: {e}"))
        })
    }
}

#[async_trait]
impl JetStreamTransport for NatsJetStreamTransport {
    async fn publish(
        &self,
        subject: String,
        payload: Vec<u8>,
    ) -> Result<Box<dyn PendingPublishAck>> {
        let ack = self
            .jetstream
            .publish(subject, payload.into())
            .await
            .context("failed to publish to NATS JetStream")?;
        Ok(Box::new(NatsPendingAck(ack)))
    }

    async fn publish_with_headers(
        &self,
        subject: String,
        headers: async_nats::HeaderMap,
        payload: Vec<u8>,
    ) -> Result<Box<dyn PendingPublishAck>> {
        let ack = self
            .jetstream
            .publish_with_headers(subject, headers, payload.into())
            .await
            .context("failed to publish to NATS JetStream")?;
        Ok(Box::new(NatsPendingAck(ack)))
    }
}

/// Publishes alert events to NATS JetStream.
///
/// By default an unavailable NATS degrades gracefully: `publish_alert` logs a
/// warning and returns `Ok(())` without delivering. When NATS is a **required**
/// capability ([`nats_required_from_env`] — `REQUIRE_NATS=true` or
/// `APEX_ENV=production`), the same condition returns `Err` so the caller
/// reports a degraded outcome instead of a false success.
#[derive(Clone)]
pub struct NatsPublisher {
    client: Option<async_nats::Client>,
    transport: Option<Arc<dyn JetStreamTransport>>,
    nats_url: String,
    required: bool,
}

impl NatsPublisher {
    /// Create a new disabled publisher (no NATS connection, not required).
    pub fn disabled() -> Self {
        Self::disabled_with_requirement(false)
    }

    /// A disabled publisher that reports unavailable NATS as an error.
    pub fn disabled_with_requirement(required: bool) -> Self {
        Self {
            client: None,
            transport: None,
            nats_url: String::new(),
            required,
        }
    }

    /// Connect to NATS and ensure the JetStream stream exists, resolving the
    /// capability requirement from the environment.
    pub async fn connect(nats_url: &str) -> Self {
        Self::connect_with_requirement(nats_url, nats_required_from_env()).await
    }

    /// Connect to NATS with an explicit capability requirement.
    ///
    /// If the connection fails, the publisher operates in degraded mode: a
    /// no-op when NATS is optional, or an error on every publish when NATS is
    /// required.
    pub async fn connect_with_requirement(nats_url: &str, required: bool) -> Self {
        match Self::try_connect(nats_url).await {
            Ok((client, jetstream)) => {
                info!(nats_url = %nats_url, "NATS JetStream publisher connected");
                Self {
                    client: Some(client),
                    transport: Some(Arc::new(NatsJetStreamTransport::new(jetstream))),
                    nats_url: nats_url.to_string(),
                    required,
                }
            }
            Err(e) => {
                warn!(
                    nats_url = %nats_url,
                    required,
                    error = %e,
                    "NATS unavailable — alert publishing degraded"
                );
                Self {
                    client: None,
                    transport: None,
                    nats_url: nats_url.to_string(),
                    required,
                }
            }
        }
    }

    /// Build a publisher around an injected transport (tests).
    #[cfg(test)]
    pub fn with_transport(transport: Arc<dyn JetStreamTransport>, required: bool) -> Self {
        Self {
            client: None,
            transport: Some(transport),
            nats_url: "test://transport".to_string(),
            required,
        }
    }

    async fn try_connect(
        nats_url: &str,
    ) -> Result<(async_nats::Client, async_nats::jetstream::Context)> {
        let client = async_nats::connect(nats_url)
            .await
            .context("failed to connect to NATS")?;
        let jetstream = async_nats::jetstream::new(client.clone());

        // Ensure the stream exists (idempotent)
        Self::ensure_stream(&jetstream).await?;

        Ok((client, jetstream))
    }

    /// Ensure the `alerts` JetStream stream exists, creating it if necessary.
    async fn ensure_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
        use async_nats::jetstream::stream::Config;

        let cfg = Config {
            name: "alerts".to_string(),
            subjects: vec!["alerts.>".to_string()],
            max_age: chrono::Duration::days(7)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(604800)),
            storage: async_nats::jetstream::stream::StorageType::File,
            retention: async_nats::jetstream::stream::RetentionPolicy::Interest,
            ..Config::default()
        };

        match jetstream.get_stream("alerts").await {
            Ok(_) => {
                info!("NATS JetStream stream 'alerts' already exists");
                Ok(())
            }
            Err(_) => {
                jetstream
                    .create_stream(cfg)
                    .await
                    .context("failed to create JetStream stream 'alerts'")?;
                info!("NATS JetStream stream 'alerts' created");
                Ok(())
            }
        }
    }

    /// Publish an alert event to NATS JetStream and await the broker ACK.
    ///
    /// Success is reported only after `wait_for_ack` resolves: the JetStream
    /// `publish` call returns a *future* that must itself be awaited, so
    /// reporting success right after the first `.await` would silently claim a
    /// delivery the server never accepted.
    ///
    /// If NATS is unavailable: an error when NATS is required, otherwise a
    /// logged warning and `Ok(())` (graceful degradation).
    pub async fn publish_alert(&self, alert: &AlertEvent) -> Result<()> {
        let Some(ref transport) = self.transport else {
            if self.required {
                anyhow::bail!(
                    "NATS JetStream is required ({REQUIRE_NATS_ENV}/APEX_ENV=production) \
                     but the publisher is not connected; alert {} not delivered",
                    alert.id
                );
            }
            warn!(
                alert_id = %alert.id,
                "NATS publisher not connected — skipping alert publish"
            );
            return Ok(());
        };

        let subject = alert.subject();
        let payload =
            serde_json::to_vec(alert).context("failed to serialize AlertEvent to JSON")?;

        let ack = transport
            .publish(subject.clone(), payload)
            .await
            .context(format!(
                "failed to publish alert to NATS subject '{subject}'"
            ))?;
        ack.wait_for_ack().await.context(format!(
            "NATS JetStream did not acknowledge alert on subject '{subject}'"
        ))?;

        info!(
            alert_id = %alert.id,
            event_type = %alert.event_type,
            subject = %subject,
            "Alert event published to NATS JetStream (ACK awaited)"
        );

        Ok(())
    }

    /// Publish a raw payload with headers and await the broker ACK (dead-letter
    /// copies). Not awaited ACKs are the same silent-loss bug as alert
    /// publishes, so the contract is identical.
    pub async fn publish_with_headers_acked(
        &self,
        subject: &str,
        headers: async_nats::HeaderMap,
        payload: Vec<u8>,
    ) -> Result<()> {
        let Some(ref transport) = self.transport else {
            if self.required {
                anyhow::bail!(
                    "NATS JetStream is required ({REQUIRE_NATS_ENV}/APEX_ENV=production) \
                     but the publisher is not connected; payload for '{subject}' not delivered"
                );
            }
            warn!(subject, "NATS publisher not connected — skipping publish");
            return Ok(());
        };
        let ack = transport
            .publish_with_headers(subject.to_string(), headers, payload)
            .await
            .context(format!("failed to publish to NATS subject '{subject}'"))?;
        ack.wait_for_ack().await.context(format!(
            "NATS JetStream did not acknowledge publish on subject '{subject}'"
        ))?;
        Ok(())
    }

    /// Whether NATS is a required capability for this process.
    pub fn required(&self) -> bool {
        self.required
    }

    /// Returns `true` if NATS is connected and operational.
    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    /// Borrow the underlying NATS client, when connected.
    pub fn client(&self) -> Option<&async_nats::Client> {
        self.client.as_ref()
    }

    /// Returns the NATS URL this publisher was configured with.
    pub fn nats_url(&self) -> &str {
        &self.nats_url
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Records the publish/ACK ordering so a missing `wait_for_ack` await is a
    /// test failure instead of a silent success.
    #[derive(Default)]
    struct AckRecording {
        calls: AtomicUsize,
        publishes: AtomicUsize,
        acks: AtomicUsize,
        fail_ack: AtomicBool,
        order: Mutex<Vec<&'static str>>,
    }

    impl AckRecording {
        fn push(&self, entry: &'static str) {
            if let Ok(mut order) = self.order.lock() {
                order.push(entry);
            }
        }

        fn order(&self) -> Vec<&'static str> {
            self.order.lock().map(|o| o.clone()).unwrap_or_default()
        }
    }

    struct MockAck {
        recording: Arc<AckRecording>,
    }

    impl PendingPublishAck for MockAck {
        fn wait_for_ack(self: Box<Self>) -> BoxFuture<'static, Result<()>> {
            Box::pin(async move {
                self.recording.acks.fetch_add(1, Ordering::SeqCst);
                self.recording.push("ack");
                if self.recording.fail_ack.load(Ordering::SeqCst) {
                    anyhow::bail!("simulated JetStream ACK failure");
                }
                Ok(())
            })
        }
    }

    struct MockTransport {
        recording: Arc<AckRecording>,
    }

    #[async_trait]
    impl JetStreamTransport for MockTransport {
        async fn publish(
            &self,
            _subject: String,
            _payload: Vec<u8>,
        ) -> Result<Box<dyn PendingPublishAck>> {
            self.recording.calls.fetch_add(1, Ordering::SeqCst);
            self.recording.publishes.fetch_add(1, Ordering::SeqCst);
            self.recording.push("publish");
            Ok(Box::new(MockAck {
                recording: self.recording.clone(),
            }))
        }

        async fn publish_with_headers(
            &self,
            _subject: String,
            _headers: async_nats::HeaderMap,
            _payload: Vec<u8>,
        ) -> Result<Box<dyn PendingPublishAck>> {
            self.publish(_subject, _payload).await
        }
    }

    fn test_alert() -> AlertEvent {
        AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test warning".to_string(),
            description: "A test warning event".to_string(),
            entity_id: None,
            entity_name: None,
            audience: AlertAudience::Users(vec![]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn publish_alert_awaits_the_jetstream_ack() {
        // If `publish_alert` stopped at `transport.publish(..).await` and never
        // awaited the ACK future, `acks` would stay 0 and this test would fail.
        let recording = Arc::new(AckRecording::default());
        let publisher = NatsPublisher::with_transport(
            Arc::new(MockTransport {
                recording: recording.clone(),
            }),
            false,
        );

        publisher.publish_alert(&test_alert()).await.unwrap();

        assert_eq!(recording.publishes.load(Ordering::SeqCst), 1);
        assert_eq!(
            recording.acks.load(Ordering::SeqCst),
            1,
            "publish_alert must await the JetStream ACK before reporting success"
        );
        assert_eq!(recording.order(), vec!["publish", "ack"]);
    }

    #[tokio::test]
    async fn publish_alert_fails_when_ack_fails() {
        let recording = Arc::new(AckRecording::default());
        recording.fail_ack.store(true, Ordering::SeqCst);
        let publisher = NatsPublisher::with_transport(
            Arc::new(MockTransport {
                recording: recording.clone(),
            }),
            false,
        );

        let result = publisher.publish_alert(&test_alert()).await;
        assert!(
            result.is_err(),
            "an unacknowledged publish is not a success"
        );
        assert_eq!(recording.acks.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn required_nats_publisher_errors_when_unavailable() {
        let publisher = NatsPublisher::disabled_with_requirement(true);
        assert!(publisher.required());
        assert!(!publisher.is_connected());
        let result = publisher.publish_alert(&test_alert()).await;
        assert!(
            result.is_err(),
            "missing NATS must be an error when NATS is required"
        );
        // The dead-letter path has the same contract.
        let dead_letter = publisher
            .publish_with_headers_acked("dead_letter.alerts", async_nats::HeaderMap::new(), vec![])
            .await;
        assert!(dead_letter.is_err());
    }

    #[tokio::test]
    async fn optional_nats_publisher_degrades_quietly_when_unavailable() {
        let publisher = NatsPublisher::disabled();
        assert!(!publisher.required());
        publisher.publish_alert(&test_alert()).await.unwrap();
    }

    #[test]
    fn nats_requirement_resolution_prefers_explicit_flag() {
        assert!(nats_required(Some("true"), Some("development")));
        assert!(nats_required(Some("1"), None));
        assert!(!nats_required(Some("false"), Some("production")));
        assert!(nats_required(None, Some("production")));
        assert!(nats_required(None, Some("production ")));
        assert!(nats_required(None, Some("PROD")));
        assert!(!nats_required(None, Some("staging")));
        assert!(!nats_required(None, None));
        assert!(!nats_required(Some("  "), Some("staging")));
    }

    #[test]
    fn alert_event_subject_format() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test warning".to_string(),
            description: "A test warning event".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_name: Some("Test Corp".to_string()),
            audience: AlertAudience::Users(vec![]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        assert_eq!(event.subject(), "alerts.events.new_warning");
    }

    #[test]
    fn alert_event_type_as_str() {
        assert_eq!(AlertEventType::NewInsight.as_str(), "new_insight");
        assert_eq!(AlertEventType::NewWarning.as_str(), "new_warning");
        assert_eq!(AlertEventType::RecipeMatch.as_str(), "recipe_match");
        assert_eq!(
            AlertEventType::CompetitorChange.as_str(),
            "competitor_change"
        );
        assert_eq!(
            AlertEventType::SupplyChainRisk.as_str(),
            "supply_chain_risk"
        );
        assert_eq!(AlertEventType::SystemAlert.as_str(), "system_alert");
    }

    #[test]
    fn disabled_publisher_is_not_connected() {
        let publisher = NatsPublisher::disabled();
        assert!(!publisher.is_connected());
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::RecipeMatch,
            severity: apex_core::alert_config::AlertSeverity::Critical,
            title: "Recipe matched".to_string(),
            description: "A new recipe match found".to_string(),
            entity_id: None,
            entity_name: None,
            audience: AlertAudience::Users(vec![Uuid::new_v4()]),
            metadata: serde_json::json!({"score": 0.95}),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AlertEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.audience, event.audience);
    }
}
