//! NATS JetStream publisher for alert events.
//!
//! Provides a [`NatsPublisher`] that connects to NATS, ensures the `alerts`
//! JetStream stream exists via the shared
//! [`apex_shared::ensure_alerts_stream`] helper, and publishes
//! [`AlertEvent`]s to subjects `alerts.events.{event_type}`.
//!
//! # Stream configuration
//! The canonical stream definition lives in `apex-shared` so the API SSE
//! bridge and the worker cannot race each other into different configs
//! (audit #78): name `alerts`, subjects `alerts.>`, file storage,
//! [`async_nats::jetstream::stream::RetentionPolicy::Limits`] with a 2-hour
//! max age, and a 2-hour duplicate window.
//!
//! # Reconnect behaviour
//! If NATS is unavailable when the publisher is constructed, a background
//! task retries connecting with exponential backoff (1s → 60s cap) and
//! swaps a fresh transport into the live publisher once the broker is back
//! (audit #79), so a NATS outage during startup no longer disables alert
//! publishing until the next worker restart. The publisher API is unchanged.
//!
//! # Delivery guarantee
//! Publishing is **at-least-once**: the outbox publisher retries a row until
//! the JetStream ACK resolves, so a crash after the broker accepted a message
//! but before the ACK was recorded can republish it. Passing the stable outbox
//! row id as `Nats-Msg-Id` lets JetStream suppress that duplicate inside the
//! duplicate window; consumers must still be idempotent. This is not
//! exactly-once delivery.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use apex_core::alert_config::AlertAudience;
use apex_shared::ensure_alerts_stream;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
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
#[serde(from = "AlertEventWire")]
pub struct AlertEvent {
    pub id: Uuid,
    pub event_type: AlertEventType,
    pub severity: apex_core::alert_config::AlertSeverity,
    pub title: String,
    pub description: String,
    /// Complete entity set the alert references. The API resolves subscribers
    /// for every entry (union), so a multi-entity warning never notifies only
    /// the first entity's subscribers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_ids: Vec<Uuid>,
    pub entity_name: Option<String>,
    /// Who this alert is addressed to. `Users(vec![])` addresses nobody and
    /// only a deliberate `Broadcast` reaches every connected user.
    pub audience: AlertAudience,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Wire form that still accepts the pre-`entity_ids` singular field.
///
/// `event_outbox` rows written by older binaries carry `entity_id`; without
/// this the rename would deserialize successfully into `entity_ids = []` and
/// silently publish those alerts to nobody. The legacy id is merged into the
/// full set instead.
#[derive(Deserialize)]
struct AlertEventWire {
    id: Uuid,
    event_type: AlertEventType,
    severity: apex_core::alert_config::AlertSeverity,
    title: String,
    description: String,
    #[serde(default)]
    entity_ids: Vec<Uuid>,
    #[serde(default)]
    entity_id: Option<Uuid>,
    entity_name: Option<String>,
    audience: AlertAudience,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<AlertEventWire> for AlertEvent {
    fn from(wire: AlertEventWire) -> Self {
        let mut entity_ids = wire.entity_ids;
        if entity_ids.is_empty() {
            if let Some(entity_id) = wire.entity_id {
                entity_ids.push(entity_id);
            }
        }
        Self {
            id: wire.id,
            event_type: wire.event_type,
            severity: wire.severity,
            title: wire.title,
            description: wire.description,
            entity_ids,
            entity_name: wire.entity_name,
            audience: wire.audience,
            metadata: wire.metadata,
            created_at: wire.created_at,
        }
    }
}

impl AlertEvent {
    /// Build the NATS subject for this event.
    pub fn subject(&self) -> String {
        format!("alerts.events.{}", self.event_type)
    }

    /// Primary entity for display and per-entity config lookups, if any.
    pub fn primary_entity_id(&self) -> Option<Uuid> {
        self.entity_ids.first().copied()
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
    ///
    /// `msg_id` is sent as the JetStream `Nats-Msg-Id` header: the broker
    /// deduplicates republishes of the same id inside the stream's duplicate
    /// window (at-least-once transport, not exactly-once).
    async fn publish(
        &self,
        subject: String,
        msg_id: Option<String>,
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
        msg_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<Box<dyn PendingPublishAck>> {
        let mut headers = async_nats::HeaderMap::new();
        if let Some(msg_id) = msg_id {
            headers.insert("Nats-Msg-Id".to_string(), msg_id);
        }
        let ack = self
            .jetstream
            .publish_with_headers(subject, headers, payload.into())
            .await
            .context("failed to publish to NATS JetStream")?;
        Ok(Box::new(NatsPendingAck(ack)))
    }
}

/// State shared by every clone of a [`NatsPublisher`].
///
/// The transport sits behind an async lock so the background reconnect task
/// can swap in a fresh JetStream context at runtime; `connected` mirrors that
/// slot lock-free for the synchronous [`NatsPublisher::is_connected`] probe
/// used by the outbox drain loop.
struct PublisherState {
    transport: RwLock<Option<Arc<dyn JetStreamTransport>>>,
    connected: AtomicBool,
    nats_url: String,
    required: bool,
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
    state: Arc<PublisherState>,
}

impl NatsPublisher {
    /// Create a new disabled publisher (no NATS connection, not required).
    pub fn disabled() -> Self {
        Self::disabled_with_requirement(false)
    }

    /// A disabled publisher that reports unavailable NATS as an error.
    pub fn disabled_with_requirement(required: bool) -> Self {
        Self::from_state(PublisherState {
            transport: RwLock::new(None),
            connected: AtomicBool::new(false),
            nats_url: String::new(),
            required,
        })
    }

    fn from_state(state: PublisherState) -> Self {
        Self {
            state: Arc::new(state),
        }
    }

    /// A fully connected publisher around an established JetStream context.
    fn connected(
        jetstream: async_nats::jetstream::Context,
        nats_url: &str,
        required: bool,
    ) -> Self {
        Self::from_state(PublisherState {
            transport: RwLock::new(Some(Arc::new(NatsJetStreamTransport::new(jetstream)))),
            connected: AtomicBool::new(true),
            nats_url: nats_url.to_string(),
            required,
        })
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
    /// required. Either way a background task keeps retrying with exponential
    /// backoff (1s → 60s cap) and swaps the transport in once NATS is
    /// reachable, so a broker that is down at startup no longer disables
    /// alert publishing until the worker restarts (audit #79).
    pub async fn connect_with_requirement(nats_url: &str, required: bool) -> Self {
        match Self::try_connect(nats_url).await {
            Ok(jetstream) => {
                info!(nats_url = %nats_url, "NATS JetStream publisher connected");
                Self::connected(jetstream, nats_url, required)
            }
            Err(e) => {
                warn!(
                    nats_url = %nats_url,
                    required,
                    error = %e,
                    "NATS unavailable — alert publishing degraded; reconnecting in the background"
                );
                let publisher = Self::from_state(PublisherState {
                    transport: RwLock::new(None),
                    connected: AtomicBool::new(false),
                    nats_url: nats_url.to_string(),
                    required,
                });
                Self::spawn_reconnect_loop(Arc::downgrade(&publisher.state));
                publisher
            }
        }
    }

    /// Build a publisher around an injected transport.
    ///
    /// Used by tests (and available to any caller that wants to inject a
    /// transport) so the ACK-ordering contract can be verified without a live
    /// NATS server.
    pub fn with_transport(transport: Arc<dyn JetStreamTransport>, required: bool) -> Self {
        Self::from_state(PublisherState {
            transport: RwLock::new(Some(transport)),
            connected: AtomicBool::new(true),
            nats_url: "test://transport".to_string(),
            required,
        })
    }

    /// Retry `try_connect` forever with exponential backoff until it succeeds
    /// or every publisher clone has been dropped.
    ///
    /// Holds only a [`Weak`] reference so an unused publisher cannot leak a
    /// task; the strong `Arc` is upgraded for each attempt.
    fn spawn_reconnect_loop(state: Weak<PublisherState>) {
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                let Some(state) = state.upgrade() else {
                    return; // publisher dropped
                };
                if state.nats_url.is_empty() || state.connected.load(Ordering::SeqCst) {
                    return; // disabled publisher, or another path connected first
                }
                tokio::time::sleep(backoff).await;
                if state.connected.load(Ordering::SeqCst) {
                    return;
                }
                match Self::try_connect(&state.nats_url).await {
                    Ok(jetstream) => {
                        let transport: Arc<dyn JetStreamTransport> =
                            Arc::new(NatsJetStreamTransport::new(jetstream));
                        *state.transport.write().await = Some(transport);
                        state.connected.store(true, Ordering::SeqCst);
                        info!(
                            nats_url = %state.nats_url,
                            "NATS JetStream publisher reconnected after outage"
                        );
                        return;
                    }
                    Err(e) => {
                        warn!(
                            nats_url = %state.nats_url,
                            error = %e,
                            retry_in_secs = backoff.as_secs(),
                            "NATS reconnect attempt failed; will retry"
                        );
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                    }
                }
            }
        });
    }

    async fn try_connect(nats_url: &str) -> Result<async_nats::jetstream::Context> {
        let client = async_nats::connect(nats_url)
            .await
            .context("failed to connect to NATS")?;
        let jetstream = async_nats::jetstream::new(client);

        // Ensure the stream exists with the ONE canonical config shared by
        // this publisher and the API SSE bridge (audit #78).
        ensure_alerts_stream(&jetstream).await?;

        Ok(jetstream)
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
    ///
    /// Reachable from production code only through the alert transport module
    /// (`crates/worker/src/alert_transport.rs`); CI enforces this with
    /// `scripts/ci/check_alert_publish.sh`.
    pub async fn publish_alert(&self, alert: &AlertEvent) -> Result<()> {
        self.publish_alert_with_msg_id(alert, None).await
    }

    /// Publish an alert event with a stable transport message id.
    ///
    /// The id is sent as the JetStream `Nats-Msg-Id` header so a republish of
    /// the same at-least-once event is deduplicated inside the stream's
    /// duplicate window.
    pub async fn publish_alert_with_msg_id(
        &self,
        alert: &AlertEvent,
        msg_id: Option<&str>,
    ) -> Result<()> {
        let transport = self.state.transport.read().await.clone();
        let Some(transport) = transport else {
            if self.state.required {
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
            .publish(subject.clone(), msg_id.map(str::to_string), payload)
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

    /// Whether NATS is a required capability for this process.
    pub fn required(&self) -> bool {
        self.state.required
    }

    /// Returns `true` if NATS is connected and operational.
    pub fn is_connected(&self) -> bool {
        self.state.connected.load(Ordering::SeqCst)
    }

    /// Returns the NATS URL this publisher was configured with.
    pub fn nats_url(&self) -> &str {
        &self.state.nats_url
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
        msg_ids: Mutex<Vec<Option<String>>>,
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

        fn msg_ids(&self) -> Vec<Option<String>> {
            self.msg_ids.lock().map(|m| m.clone()).unwrap_or_default()
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
            msg_id: Option<String>,
            _payload: Vec<u8>,
        ) -> Result<Box<dyn PendingPublishAck>> {
            self.recording.calls.fetch_add(1, Ordering::SeqCst);
            self.recording.publishes.fetch_add(1, Ordering::SeqCst);
            self.recording.push("publish");
            if let Ok(mut ids) = self.recording.msg_ids.lock() {
                ids.push(msg_id);
            }
            Ok(Box::new(MockAck {
                recording: self.recording.clone(),
            }))
        }
    }

    fn test_alert() -> AlertEvent {
        AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test warning".to_string(),
            description: "A test warning event".to_string(),
            entity_ids: Vec::new(),
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
    async fn publish_alert_with_msg_id_sends_the_stable_transport_id() {
        // The outbox publisher passes the outbox row id; JetStream's
        // `Nats-Msg-Id` dedupe depends on this header being set.
        let recording = Arc::new(AckRecording::default());
        let publisher = NatsPublisher::with_transport(
            Arc::new(MockTransport {
                recording: recording.clone(),
            }),
            false,
        );

        publisher
            .publish_alert_with_msg_id(&test_alert(), Some("outbox-row-123"))
            .await
            .unwrap();
        publisher.publish_alert(&test_alert()).await.unwrap();

        assert_eq!(
            recording.msg_ids(),
            vec![Some("outbox-row-123".to_string()), None],
            "the stable outbox id must be passed as Nats-Msg-Id; publish_alert has no id"
        );
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
            entity_ids: vec![Uuid::new_v4()],
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

    /// Audit #79: a broker that is down at startup must not disable publishing
    /// until the next worker restart. `connect_with_requirement` returns a
    /// degraded publisher immediately and spawns the background reconnect loop
    /// (1s -> 60s backoff); while disconnected a required publisher errors so
    /// the outbox drain keeps the event instead of dropping it.
    #[tokio::test]
    async fn failed_startup_connect_degrades_and_reconnects_in_background() {
        let publisher = tokio::time::timeout(
            Duration::from_secs(10),
            NatsPublisher::connect_with_requirement("nats://127.0.0.1:1", true),
        )
        .await
        .expect("a refused startup connect must fail fast, not hang");

        assert!(
            !publisher.is_connected(),
            "no broker: the publisher starts degraded instead of panicking"
        );
        assert!(publisher.required());
        assert_eq!(publisher.nats_url(), "nats://127.0.0.1:1");
        let result = publisher.publish_alert(&test_alert()).await;
        assert!(
            result.is_err(),
            "a required publisher must error while the broker is unreachable"
        );
    }

    #[test]
    fn alert_event_serde_roundtrip() {
        let event = AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::RecipeMatch,
            severity: apex_core::alert_config::AlertSeverity::Critical,
            title: "Recipe matched".to_string(),
            description: "A new recipe match found".to_string(),
            entity_ids: Vec::new(),
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

    #[test]
    fn legacy_singular_entity_id_still_addresses_its_entity() {
        let entity_id = Uuid::new_v4();
        // Frozen pre-rename shape: what older binaries wrote to event_outbox.
        let legacy = serde_json::json!({
            "id": Uuid::new_v4(),
            "event_type": "new_warning",
            "severity": "high",
            "title": "Legacy",
            "description": "queued before the entity_ids rename",
            "entity_id": entity_id,
            "entity_name": "Acme",
            "audience": {"kind": "users", "user_ids": []},
            "metadata": {"warning_id": Uuid::new_v4()},
            "created_at": "2026-01-01T00:00:00Z"
        });

        let event: AlertEvent = serde_json::from_value(legacy).expect("legacy payload parses");
        assert_eq!(
            event.entity_ids,
            vec![entity_id],
            "the legacy id must not be dropped into an empty set"
        );
        assert_eq!(event.primary_entity_id(), Some(entity_id));
    }
}
