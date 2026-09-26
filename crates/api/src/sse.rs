//! Server-Sent Events (SSE) manager for real-time alert streaming.
//!
//! Provides:
//! - [`SseManager`] — manages per-user SSE connections and dispatches alerts
//! - [`SseEvent`] — typed SSE event with id, event type, and JSON data
//! - NATS JetStream consumer integration for receiving alerts from the worker
//!
//! # Architecture
//! The API server holds a single [`SseManager`] in `AppState`. A background task
//! consumes alerts from NATS JetStream and dispatches them to connected SSE clients
//! via fan-out per user. Each authenticated user gets their own event stream,
//! keyed by the stable principal UUID derived from their user name.
//!
//! # Delivery guarantees
//! The consumer never acks a message before it has been processed: the payload
//! is parsed first, then routed and dispatched, and only a fully handled
//! message is acked. Transient failures are nacked with a delay so JetStream
//! redelivers them; permanently bad payloads are copied to the `dead_letter`
//! stream before they are acked.

use crate::alert_router::{AlertEvent, AlertRouter, PrincipalDirectory};
use anyhow::{Context, Result};
use apex_core::alert_config::AlertAudience;
use axum::response::sse::{Event, KeepAlive, Sse};
#[cfg(test)]
use chrono::Utc;
use futures_util::stream::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// An SSE event ready to be sent to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseEvent {
    /// Event ID (for `Last-Event-ID` reconnection).
    pub id: String,
    /// Event type (e.g., "alert", "warning", "insight").
    pub event: String,
    /// JSON-encoded alert payload.
    pub data: String,
    /// Optional reconnection delay in milliseconds.
    pub retry: Option<u32>,
}

impl SseEvent {
    /// Create a new SSE event.
    pub fn new(event: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            event: event.into(),
            data: data.into(),
            retry: None,
        }
    }

    /// Set a custom event ID.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the reconnection delay.
    pub fn with_retry(mut self, ms: u32) -> Self {
        self.retry = Some(ms);
        self
    }

    /// Convert to an axum SSE `Event`.
    pub fn into_axum_event(self) -> Event {
        let mut event = Event::default()
            .id(self.id)
            .event(self.event)
            .data(self.data);
        if let Some(retry) = self.retry {
            event = event.retry(Duration::from_millis(retry as u64));
        }
        event
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SseManager
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum events buffered per client. A slow client that fills its buffer has
/// newer events dropped rather than growing memory without bound.
const SSE_CHANNEL_CAPACITY: usize = 1024;

/// Delay before JetStream redelivers a nacked alert message.
const ALERT_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Manages per-user SSE connections and dispatches alerts to connected clients.
///
/// Each authenticated user may have multiple browser tabs open, each of which
/// registers a separate sender channel. When an alert arrives, it is fanned out
/// to all senders registered for the target users.
pub struct SseManager {
    /// Active SSE connections keyed by principal UUID.
    connections: Arc<RwLock<HashMap<Uuid, Vec<mpsc::Sender<SseEvent>>>>>,
    /// Authenticated principal ID → user name, so the alert router can consult
    /// `user_preferences` (keyed by user name) for connected users.
    principals: Arc<PrincipalDirectory>,
}

impl SseManager {
    /// Create a new empty SSE manager.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            principals: Arc::new(PrincipalDirectory::new()),
        }
    }

    /// The principal directory shared with the alert router.
    pub fn principal_directory(&self) -> Arc<PrincipalDirectory> {
        self.principals.clone()
    }

    /// Register a new SSE connection for an authenticated principal.
    ///
    /// `user_id` must be the stable principal UUID derived from `username`
    /// (`apex_core::alert_config::user_principal_id`); the user name is
    /// recorded so per-user preferences can be resolved for this principal.
    ///
    /// Returns a sender and receiver pair. The sender is stored internally for
    /// dispatch and should be passed to [`unregister`](SseManager::unregister)
    /// when the connection closes.
    pub async fn register(
        &self,
        user_id: Uuid,
        username: &str,
    ) -> (mpsc::Sender<SseEvent>, mpsc::Receiver<SseEvent>) {
        let (tx, rx) = mpsc::channel(SSE_CHANNEL_CAPACITY);

        self.principals.record(user_id, username);

        let mut conns = self.connections.write().await;
        conns.entry(user_id).or_default().push(tx.clone());

        info!(
            user_id = %user_id,
            username = %username,
            total_connections = conns.get(&user_id).map_or(0, Vec::len),
            "SSE connection registered"
        );

        (tx, rx)
    }

    /// Remove a disconnected SSE connection for a user.
    pub async fn unregister(&self, user_id: Uuid, tx: &mpsc::Sender<SseEvent>) {
        let mut conns = self.connections.write().await;

        let mut last_connection = false;
        if let Some(senders) = conns.get_mut(&user_id) {
            senders.retain(|sender| !sender.same_channel(tx));
            if senders.is_empty() {
                conns.remove(&user_id);
                last_connection = true;
            }
        }

        if last_connection {
            self.principals.forget(user_id);
        }

        info!(
            user_id = %user_id,
            remaining = conns.get(&user_id).map_or(0, Vec::len),
            "SSE connection unregistered"
        );
    }

    /// Dispatch an [`AlertEvent`] to the connected SSE clients addressed by its
    /// audience.
    ///
    /// [`AlertAudience::Broadcast`] reaches every connected user;
    /// [`AlertAudience::Users`] reaches only the listed principals, and an
    /// empty list reaches nobody.
    ///
    /// Returns the number of clients the event was sent to. Events are dropped
    /// for clients whose buffer is full (slow consumers).
    pub async fn dispatch_alert(&self, alert: &AlertEvent) -> usize {
        let event = SseEvent::new(
            alert.event_type.as_str(),
            serde_json::to_string(alert).unwrap_or_else(|_| "{}".to_string()),
        );

        let conns = self.connections.read().await;
        let mut sent_count = 0usize;

        let deliver = |senders: &[mpsc::Sender<SseEvent>], event: &SseEvent, sent: &mut usize| {
            for sender in senders {
                match sender.try_send(event.clone()) {
                    Ok(()) => *sent += 1,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        debug!("SSE client buffer full — dropping event");
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        // Channel closed — will be cleaned up on next unregister
                    }
                }
            }
        };

        match &alert.audience {
            AlertAudience::Broadcast => {
                for senders in conns.values() {
                    deliver(senders, &event, &mut sent_count);
                }
            }
            AlertAudience::Users(user_ids) => {
                for user_id in user_ids {
                    if let Some(senders) = conns.get(user_id) {
                        deliver(senders, &event, &mut sent_count);
                    }
                }
            }
        }

        if sent_count > 0 {
            trace!(
                alert_id = %alert.id,
                event_type = %alert.event_type,
                sent_to = sent_count,
                "Alert dispatched to SSE clients"
            );
        }

        sent_count
    }

    /// Start a background task that consumes alerts from NATS JetStream and
    /// dispatches them to connected SSE clients via the alert router.
    ///
    /// Spawns a tokio task that runs indefinitely.
    pub async fn start_nats_consumer(
        self: Arc<Self>,
        nats_url: &str,
        alert_router: Arc<AlertRouter>,
    ) {
        let client = match async_nats::connect(nats_url).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    nats_url = %nats_url,
                    error = %e,
                    "NATS consumer unavailable — SSE will not receive real-time alerts"
                );
                return;
            }
        };

        let jetstream = async_nats::jetstream::new(client.clone());

        // Ensure stream exists (idempotent — worker may have already created it)
        if let Err(e) = Self::ensure_stream(&jetstream).await {
            warn!(error = %e, "Failed to ensure JetStream stream 'alerts'");
        }

        // Dead-letter stream: bad payloads are copied here before they are
        // acked, so nothing is silently dropped.
        if let Err(e) = Self::ensure_dead_letter_stream(&jetstream).await {
            warn!(error = %e, "Failed to ensure JetStream stream 'dead_letter'");
        }

        // Create a push consumer
        let consumer: async_nats::jetstream::consumer::PushConsumer = match jetstream
            .create_consumer_on_stream(
                async_nats::jetstream::consumer::push::Config {
                    durable_name: Some("sse_bridge".to_string()),
                    deliver_subject: format!("sse_bridge.deliver.{}", Uuid::new_v4()),
                    deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
                    ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                    ack_wait: Duration::from_secs(30),
                    max_deliver: 3,
                    ..Default::default()
                },
                "alerts",
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "Failed to create NATS JetStream consumer");
                return;
            }
        };

        info!("NATS JetStream consumer 'sse_bridge' started");

        let manager = self.clone();
        let processor = NatsAlertProcessor {
            manager,
            router: alert_router,
        };
        let dead_letter = NatsDeadLetter {
            jetstream: jetstream.clone(),
        };
        tokio::spawn(async move {
            // Reconnect loop: a closed or errored subscription must not
            // permanently disable real-time alerts for the process lifetime.
            loop {
                let mut messages = match consumer.messages().await {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        error!(error = %e, "Failed to open NATS consumer message stream");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };
                info!("NATS consumer message stream opened");

                loop {
                    match tokio::time::timeout(Duration::from_secs(5), messages.next()).await {
                        Ok(Some(Ok(msg))) => {
                            let message = NatsAlertMessage(&msg);
                            consume_alert_message(&message, &processor, &dead_letter).await;
                        }
                        Ok(Some(Err(e))) => {
                            error!(error = %e, "NATS consumer message error");
                        }
                        Ok(None) => {
                            info!("NATS consumer stream ended, reconnecting...");
                            break;
                        }
                        Err(_) => {
                            // Timeout — normal, just loop and wait for more messages
                        }
                    }
                }

                warn!("NATS consumer stream closed — reconnecting");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    async fn ensure_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
        use async_nats::jetstream::stream::Config;

        match jetstream.get_stream("alerts").await {
            Ok(_) => {
                info!("JetStream stream 'alerts' already exists");
                Ok(())
            }
            Err(_) => {
                let cfg = Config {
                    name: "alerts".to_string(),
                    subjects: vec!["alerts.>".to_string()],
                    max_age: Duration::from_secs(7 * 86400),
                    storage: async_nats::jetstream::stream::StorageType::File,
                    retention: async_nats::jetstream::stream::RetentionPolicy::Interest,
                    ..Config::default()
                };
                jetstream
                    .create_stream(cfg)
                    .await
                    .context("failed to create JetStream stream 'alerts'")?;
                info!("JetStream stream 'alerts' created");
                Ok(())
            }
        }
    }

    async fn ensure_dead_letter_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
        use async_nats::jetstream::stream::Config;

        match jetstream.get_stream("dead_letter").await {
            Ok(_) => Ok(()),
            Err(_) => {
                let cfg = Config {
                    name: "dead_letter".to_string(),
                    subjects: vec!["dead_letter.>".to_string()],
                    max_age: Duration::from_secs(30 * 86400),
                    storage: async_nats::jetstream::stream::StorageType::File,
                    ..Config::default()
                };
                jetstream
                    .create_stream(cfg)
                    .await
                    .context("failed to create JetStream stream 'dead_letter'")?;
                info!("JetStream stream 'dead_letter' created");
                Ok(())
            }
        }
    }

    /// Build the SSE response stream for a user's receiver.
    pub fn build_sse_stream(
        rx: mpsc::Receiver<SseEvent>,
    ) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
            .map(|event| Ok::<_, std::convert::Infallible>(event.into_axum_event()));

        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("keepalive"),
        )
    }

    /// Build an SSE stream that unregisters its subscriber slot when the
    /// client disconnects (B296). The guard is moved into the stream's map
    /// closure, so it drops exactly when axum drops the response body —
    /// closing the tab or terminating the fetch releases the `SseManager`
    /// entry instead of leaking it until process restart.
    pub fn build_sse_stream_with_cleanup(
        rx: mpsc::Receiver<SseEvent>,
        manager: Arc<Self>,
        user_id: uuid::Uuid,
        tx: mpsc::Sender<SseEvent>,
    ) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
        struct UnregisterGuard {
            manager: Arc<SseManager>,
            user_id: uuid::Uuid,
            tx: mpsc::Sender<SseEvent>,
        }
        impl Drop for UnregisterGuard {
            fn drop(&mut self) {
                let manager = self.manager.clone();
                let user_id = self.user_id;
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    manager.unregister(user_id, &tx).await;
                });
            }
        }

        let guard = UnregisterGuard {
            manager,
            user_id,
            tx,
        };
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(move |event| {
            let _guard = &guard;
            Ok::<_, std::convert::Infallible>(event.into_axum_event())
        });

        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("keepalive"),
        )
    }
}

impl Default for SseManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NATS consumer processing
// ─────────────────────────────────────────────────────────────────────────────

/// What the consumer decided to do with one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageDisposition {
    /// Fully handled (or permanently bad and dead-lettered): the message was acked.
    Ack,
    /// Transient failure: the message was nacked with a delay and will be retried.
    NakRetry(Duration),
}

/// Result of routing and dispatching one alert.
enum ProcessingOutcome {
    Dispatched,
    /// A transient failure (e.g. the subscription lookup failed): retry later.
    Transient(String),
}

/// Minimal view of a JetStream message, so the consume flow can be unit-tested
/// with fakes that record the ack/nak ordering.
trait AlertMessage {
    fn payload(&self) -> &[u8];
    async fn ack(&self) -> std::result::Result<(), String>;
    async fn nak_with_delay(&self, delay: Duration) -> std::result::Result<(), String>;
}

/// Processing step: route the alert and dispatch it to the flushers.
trait AlertProcessor {
    async fn process(&self, alert: &AlertEvent) -> ProcessingOutcome;
}

/// Destination for permanently bad payloads.
trait DeadLetterSink {
    async fn dead_letter(&self, payload: &[u8], reason: &str) -> std::result::Result<(), String>;
}

/// Consume one alert message.
///
/// Ordering is the contract: the payload is parsed, then routed and
/// dispatched, and only then acked. A parse failure is dead-lettered before the
/// ack; a dead-letter failure nacks instead, so the payload is retried rather
/// than lost. Transient processing failures nack with a delay.
async fn consume_alert_message<M, P, D>(
    message: &M,
    processor: &P,
    dead_letter: &D,
) -> MessageDisposition
where
    M: AlertMessage,
    P: AlertProcessor,
    D: DeadLetterSink,
{
    // Parse first — nothing is acked yet.
    let alert = match serde_json::from_slice::<AlertEvent>(message.payload()) {
        Ok(alert) => alert,
        Err(e) => {
            let reason = format!("invalid alert payload: {e}");
            warn!(error = %e, "Dead-lettering unparseable alert payload");
            let disposition = match dead_letter.dead_letter(message.payload(), &reason).await {
                Ok(()) => MessageDisposition::Ack,
                Err(dead_letter_error) => {
                    error!(
                        error = %dead_letter_error,
                        "Failed to dead-letter alert payload; nacking to retry"
                    );
                    MessageDisposition::NakRetry(ALERT_RETRY_DELAY)
                }
            };
            return apply_disposition(message, disposition).await;
        }
    };

    // Route and dispatch before acking.
    let disposition = match processor.process(&alert).await {
        ProcessingOutcome::Dispatched => MessageDisposition::Ack,
        ProcessingOutcome::Transient(e) => {
            warn!(
                alert_id = %alert.id,
                error = %e,
                "Alert processing failed; nacking for retry"
            );
            MessageDisposition::NakRetry(ALERT_RETRY_DELAY)
        }
    };

    apply_disposition(message, disposition).await
}

/// Send the ack/nak that matches the decision, after processing finished.
async fn apply_disposition<M: AlertMessage>(
    message: &M,
    disposition: MessageDisposition,
) -> MessageDisposition {
    let result = match disposition {
        MessageDisposition::Ack => message.ack().await,
        MessageDisposition::NakRetry(delay) => message.nak_with_delay(delay).await,
    };

    if let Err(e) = result {
        warn!(error = %e, "Failed to settle NATS message; JetStream will redeliver");
    }

    disposition
}

/// Real JetStream message wrapper.
struct NatsAlertMessage<'a>(&'a async_nats::jetstream::Message);

impl AlertMessage for NatsAlertMessage<'_> {
    fn payload(&self) -> &[u8] {
        &self.0.payload
    }

    async fn ack(&self) -> std::result::Result<(), String> {
        self.0.ack().await.map_err(|e| e.to_string())
    }

    async fn nak_with_delay(&self, delay: Duration) -> std::result::Result<(), String> {
        self.0
            .ack_with(async_nats::jetstream::AckKind::Nak(Some(delay)))
            .await
            .map_err(|e| e.to_string())
    }
}

/// Real routing/dispatch processor: resolves the audience and fans the alert
/// out to connected SSE clients.
struct NatsAlertProcessor {
    manager: Arc<SseManager>,
    router: Arc<AlertRouter>,
}

impl AlertProcessor for NatsAlertProcessor {
    async fn process(&self, alert: &AlertEvent) -> ProcessingOutcome {
        match self.router.route_alert(alert).await {
            Ok(audience) => {
                let mut routed_alert = alert.clone();
                routed_alert.audience = audience;
                self.manager.dispatch_alert(&routed_alert).await;
                ProcessingOutcome::Dispatched
            }
            Err(e) => ProcessingOutcome::Transient(e.to_string()),
        }
    }
}

/// Real dead-letter sink: copies the raw payload to the `dead_letter` stream.
struct NatsDeadLetter {
    jetstream: async_nats::jetstream::Context,
}

impl DeadLetterSink for NatsDeadLetter {
    async fn dead_letter(&self, payload: &[u8], reason: &str) -> std::result::Result<(), String> {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("X-Apex-Dead-Letter-Reason", reason.to_string());
        // `JetStream::publish_with_headers` returns a future that must itself be
        // awaited: only its completion is the broker ACK. Without the second
        // await the dead-letter copy would be reported as written even when the
        // stream rejected it, and the caller would ack a message it never
        // preserved.
        let ack = self
            .jetstream
            .publish_with_headers("dead_letter.alerts", headers, payload.to_vec().into())
            .await
            .map_err(|e| e.to_string())?;
        ack.await
            .map(|_| ())
            .map_err(|e| format!("JetStream ACK failed for dead-letter copy: {e}"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert_router::AlertEventType;

    fn make_test_event() -> SseEvent {
        SseEvent::new("test", r#"{"msg":"hello"}"#)
    }

    fn test_alert(
        event_type: AlertEventType,
        audience: AlertAudience,
    ) -> crate::alert_router::AlertEvent {
        crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test".to_string(),
            description: "Test description".to_string(),
            entity_id: None,
            entity_name: None,
            audience,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn sse_event_creation() {
        let event = make_test_event();
        assert_eq!(event.event, "test");
        assert_eq!(event.data, r#"{"msg":"hello"}"#);
        assert!(!event.id.is_empty());
    }

    #[test]
    fn sse_event_with_retry() {
        let event = make_test_event().with_retry(5000);
        assert_eq!(event.retry, Some(5000));
    }

    #[tokio::test]
    async fn register_and_unregister() {
        let manager = SseManager::new();
        let user_id = Uuid::new_v4();

        let (tx, rx) = manager.register(user_id, "tester").await;
        assert_eq!(
            manager
                .connections
                .read()
                .await
                .get(&user_id)
                .map_or(0, Vec::len),
            1
        );

        // Unregister using the returned sender
        manager.unregister(user_id, &tx).await;
        assert!(
            manager.connections.read().await.get(&user_id).is_none(),
            "connection should be removed after unregister"
        );
        assert!(
            manager.principals.username_for(user_id).is_none(),
            "principal should be forgotten with the last connection"
        );

        // Drop the receiver to avoid lingering
        drop(rx);
    }

    #[tokio::test]
    async fn dispatch_to_specific_user_never_reaches_others() {
        let manager = SseManager::new();
        let user_id = Uuid::new_v4();
        let other_user = Uuid::new_v4();

        let (_tx, mut rx) = manager.register(user_id, "alice").await;
        let (_tx_other, mut rx_other) = manager.register(other_user, "bob").await;

        let alert = test_alert(
            AlertEventType::NewWarning,
            AlertAudience::Users(vec![user_id]),
        );

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 1, "Should deliver to one user");

        // Verify the target user received the event...
        let event = rx.try_recv().expect("target user should receive the alert");
        assert_eq!(event.event, "new_warning");
        // ...and the other user did not.
        assert!(
            rx_other.try_recv().is_err(),
            "non-target user must never receive the alert"
        );
    }

    #[tokio::test]
    async fn empty_users_audience_delivers_to_nobody() {
        let manager = SseManager::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        let (_tx_a, mut rx_a) = manager.register(user_a, "alice").await;
        let (_tx_b, mut rx_b) = manager.register(user_b, "bob").await;

        let alert = test_alert(AlertEventType::SystemAlert, AlertAudience::Users(vec![]));

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 0, "empty audience addresses nobody");
        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
    }

    #[tokio::test]
    async fn broadcast_delivers_to_all_users() {
        let manager = SseManager::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        let (_tx_a, mut rx_a) = manager.register(user_a, "alice").await;
        let (_tx_b, mut rx_b) = manager.register(user_b, "bob").await;

        let alert = test_alert(AlertEventType::SystemAlert, AlertAudience::Broadcast);

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 2, "Should broadcast to both users");

        assert!(rx_a.try_recv().is_ok());
        assert!(rx_b.try_recv().is_ok());
    }

    #[tokio::test]
    async fn dispatch_drops_for_full_buffer_without_blocking_or_panicking() {
        let manager = SseManager::new();
        let user_id = Uuid::new_v4();
        let (_tx, mut rx) = manager.register(user_id, "alice").await;

        let alert = test_alert(
            AlertEventType::NewWarning,
            AlertAudience::Users(vec![user_id]),
        );

        // Never consume: the bounded buffer must absorb exactly its capacity
        // and then drop the rest instead of growing without bound.
        let mut delivered = 0usize;
        for _ in 0..(SSE_CHANNEL_CAPACITY + 25) {
            delivered += manager.dispatch_alert(&alert).await;
        }
        assert_eq!(delivered, SSE_CHANNEL_CAPACITY);

        let mut drained = 0usize;
        while rx.try_recv().is_ok() {
            drained += 1;
        }
        assert_eq!(drained, SSE_CHANNEL_CAPACITY);
    }

    #[tokio::test]
    async fn dispatch_to_unknown_user_sends_nothing() {
        let manager = SseManager::new();
        let alert = test_alert(
            AlertEventType::SystemAlert,
            AlertAudience::Users(vec![Uuid::new_v4()]),
        );
        assert_eq!(manager.dispatch_alert(&alert).await, 0);
    }

    // ── Consumer ack-ordering fake ────────────────────────────────────────────

    #[derive(Default)]
    struct RecordingLog {
        entries: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingLog {
        fn push(&self, entry: impl Into<String>) {
            if let Ok(mut entries) = self.entries.lock() {
                entries.push(entry.into());
            }
        }

        fn snapshot(&self) -> Vec<String> {
            self.entries
                .lock()
                .map(|entries| entries.clone())
                .unwrap_or_default()
        }
    }

    struct FakeMessage {
        payload: Vec<u8>,
        log: Arc<RecordingLog>,
    }

    impl AlertMessage for FakeMessage {
        fn payload(&self) -> &[u8] {
            &self.payload
        }

        async fn ack(&self) -> std::result::Result<(), String> {
            self.log.push("ack");
            Ok(())
        }

        async fn nak_with_delay(&self, _delay: Duration) -> std::result::Result<(), String> {
            self.log.push("nak");
            Ok(())
        }
    }

    struct FakeProcessor {
        outcome: ProcessingOutcome,
        log: Arc<RecordingLog>,
    }

    impl AlertProcessor for FakeProcessor {
        async fn process(&self, alert: &AlertEvent) -> ProcessingOutcome {
            self.log.push(format!("process:{}", alert.id));
            match &self.outcome {
                ProcessingOutcome::Dispatched => ProcessingOutcome::Dispatched,
                ProcessingOutcome::Transient(e) => ProcessingOutcome::Transient(e.clone()),
            }
        }
    }

    struct FakeDeadLetter {
        fails: bool,
        log: Arc<RecordingLog>,
    }

    impl DeadLetterSink for FakeDeadLetter {
        async fn dead_letter(
            &self,
            _payload: &[u8],
            _reason: &str,
        ) -> std::result::Result<(), String> {
            self.log.push("dead_letter");
            if self.fails {
                Err("dead-letter stream unavailable".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn fake_payload() -> Vec<u8> {
        let alert = test_alert(AlertEventType::NewWarning, AlertAudience::Users(vec![]));
        serde_json::to_vec(&alert).unwrap()
    }

    #[tokio::test]
    async fn ack_happens_after_processing_succeeds() {
        let log = Arc::new(RecordingLog::default());
        let message = FakeMessage {
            payload: fake_payload(),
            log: log.clone(),
        };
        let processor = FakeProcessor {
            outcome: ProcessingOutcome::Dispatched,
            log: log.clone(),
        };
        let dead_letter = FakeDeadLetter {
            fails: false,
            log: log.clone(),
        };

        let disposition = consume_alert_message(&message, &processor, &dead_letter).await;

        assert_eq!(disposition, MessageDisposition::Ack);
        let entries = log.snapshot();
        assert!(
            entries[0].starts_with("process:"),
            "processing must run before ack, got {entries:?}"
        );
        assert_eq!(entries[1], "ack", "ack must be last, got {entries:?}");
    }

    #[tokio::test]
    async fn transient_failure_nacks_without_acking() {
        let log = Arc::new(RecordingLog::default());
        let message = FakeMessage {
            payload: fake_payload(),
            log: log.clone(),
        };
        let processor = FakeProcessor {
            outcome: ProcessingOutcome::Transient("subscription lookup failed".to_string()),
            log: log.clone(),
        };
        let dead_letter = FakeDeadLetter {
            fails: false,
            log: log.clone(),
        };

        let disposition = consume_alert_message(&message, &processor, &dead_letter).await;

        assert_eq!(
            disposition,
            MessageDisposition::NakRetry(ALERT_RETRY_DELAY),
            "transient failures must be retried"
        );
        let entries = log.snapshot();
        assert!(entries[0].starts_with("process:"), "got {entries:?}");
        assert_eq!(entries[1], "nak", "must not ack, got {entries:?}");
        assert!(!entries.iter().any(|e| e == "ack"));
    }

    #[tokio::test]
    async fn bad_payload_is_dead_lettered_then_acked() {
        let log = Arc::new(RecordingLog::default());
        let message = FakeMessage {
            payload: b"not json at all".to_vec(),
            log: log.clone(),
        };
        let processor = FakeProcessor {
            outcome: ProcessingOutcome::Dispatched,
            log: log.clone(),
        };
        let dead_letter = FakeDeadLetter {
            fails: false,
            log: log.clone(),
        };

        let disposition = consume_alert_message(&message, &processor, &dead_letter).await;

        assert_eq!(disposition, MessageDisposition::Ack);
        let entries = log.snapshot();
        assert_eq!(entries, vec!["dead_letter".to_string(), "ack".to_string()]);
    }

    #[tokio::test]
    async fn failed_dead_letter_nacks_to_avoid_message_loss() {
        let log = Arc::new(RecordingLog::default());
        let message = FakeMessage {
            payload: b"{broken".to_vec(),
            log: log.clone(),
        };
        let processor = FakeProcessor {
            outcome: ProcessingOutcome::Dispatched,
            log: log.clone(),
        };
        let dead_letter = FakeDeadLetter {
            fails: true,
            log: log.clone(),
        };

        let disposition = consume_alert_message(&message, &processor, &dead_letter).await;

        assert_eq!(disposition, MessageDisposition::NakRetry(ALERT_RETRY_DELAY));
        let entries = log.snapshot();
        assert_eq!(entries, vec!["dead_letter".to_string(), "nak".to_string()]);
    }
}
