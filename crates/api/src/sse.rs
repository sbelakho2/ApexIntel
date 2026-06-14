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
//! via fan-out per user. Each authenticated user gets their own event stream.

use crate::alert_router::AlertRouter;
use anyhow::{Context, Result};
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{DateTime, Utc};
use futures_util::stream::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, trace, warn};
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

/// Manages per-user SSE connections and dispatches alerts to connected clients.
///
/// Each authenticated user may have multiple browser tabs open, each of which
/// registers a separate sender channel. When an alert arrives, it is fanned out
/// to all senders registered for the target users.
pub struct SseManager {
    /// Active SSE connections keyed by user_id.
    connections: Arc<RwLock<HashMap<Uuid, Vec<mpsc::UnboundedSender<SseEvent>>>>>,
}

impl SseManager {
    /// Create a new empty SSE manager.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new SSE connection for a user.
    ///
    /// Returns a sender and receiver pair. The sender is stored internally for
    /// dispatch and should be passed to [`unregister`](SseManager::unregister)
    /// when the connection closes.
    pub async fn register(
        &self,
        user_id: Uuid,
    ) -> (mpsc::UnboundedSender<SseEvent>, mpsc::UnboundedReceiver<SseEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut conns = self.connections.write().await;
        conns.entry(user_id).or_default().push(tx.clone());

        info!(
            user_id = %user_id,
            total_connections = conns.get(&user_id).map_or(0, Vec::len),
            "SSE connection registered"
        );

        (tx, rx)
    }

    /// Remove a disconnected SSE connection for a user.
    pub async fn unregister(&self, user_id: Uuid, tx: &mpsc::UnboundedSender<SseEvent>) {
        let mut conns = self.connections.write().await;

        if let Some(senders) = conns.get_mut(&user_id) {
            senders.retain(|sender| !sender.same_channel(tx));
            if senders.is_empty() {
                conns.remove(&user_id);
            }
        }

        info!(
            user_id = %user_id,
            remaining = conns.get(&user_id).map_or(0, Vec::len),
            "SSE connection unregistered"
        );
    }

    /// Dispatch an [`AlertEvent`] to all connected SSE clients for the
    /// targeted users. If `user_ids` is empty, the alert is broadcast to all
    /// connected users.
    ///
    /// Returns the number of clients the event was sent to.
    pub async fn dispatch_alert(
        &self,
        alert: &crate::alert_router::AlertEvent,
    ) -> usize {
        let event = SseEvent::new(
            alert.event_type.as_str(),
            serde_json::to_string(alert).unwrap_or_else(|_| "{}".to_string()),
        );

        let conns = self.connections.read().await;
        let mut sent_count = 0usize;

        if alert.user_ids.is_empty() {
            // Broadcast to all connected users
            for senders in conns.values() {
                for sender in senders {
                    if sender.send(event.clone()).is_err() {
                        // Channel closed — will be cleaned up on next unregister
                    } else {
                        sent_count += 1;
                    }
                }
            }
        } else {
            // Send only to specified users
            for user_id in &alert.user_ids {
                if let Some(senders) = conns.get(user_id) {
                    for sender in senders {
                        if sender.send(event.clone()).is_err() {
                            // Channel closed — will be cleaned up on next unregister
                        } else {
                            sent_count += 1;
                        }
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
        tokio::spawn(async move {
            let mut messages = match consumer.messages().await {
                Ok(msgs) => msgs,
                Err(e) => {
                    error!(error = %e, "Failed to open NATS consumer message stream");
                    return;
                }
            };
            info!("NATS consumer message stream opened");

            loop {
                match tokio::time::timeout(Duration::from_secs(5), messages.next()).await {
                    Ok(Some(Ok(msg))) => {
                        let payload = msg.payload.clone();
                        if let Err(e) = msg.ack().await {
                            warn!(error = %e, "Failed to ack NATS message");
                        }

                        // Deserialize the alert event
                        match serde_json::from_slice::<crate::alert_router::AlertEvent>(&payload) {
                            Ok(alert) => {
                                // Route and dispatch
                                let targets = alert_router.route_alert(&alert).await;
                                let mut routed_alert = alert.clone();
                                if !targets.is_empty() {
                                    routed_alert.user_ids = targets;
                                }
                                manager.dispatch_alert(&routed_alert).await;
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to deserialize alert from NATS");
                            }
                        }
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

            warn!("NATS consumer task exiting — SSE will stop receiving alerts");
        });
    }

    async fn ensure_stream(
        jetstream: &async_nats::jetstream::Context,
    ) -> Result<()> {
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

    /// Build the SSE response stream for a user's receiver.
    pub fn build_sse_stream(
        rx: mpsc::UnboundedReceiver<SseEvent>,
    ) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
        let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
            .map(|event| Ok::<_, std::convert::Infallible>(event.into_axum_event()));

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
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert_router::AlertEventType;

    fn make_test_event() -> SseEvent {
        SseEvent::new("test", r#"{"msg":"hello"}"#)
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

        let (tx, rx) = manager.register(user_id).await;
        assert_eq!(
            manager.connections.read().await.get(&user_id).map_or(0, Vec::len),
            1
        );

        // Unregister using the returned sender
        manager.unregister(user_id, &tx).await;
        assert!(
            manager.connections.read().await.get(&user_id).is_none(),
            "connection should be removed after unregister"
        );

        // Drop the receiver to avoid lingering
        drop(rx);
    }

    #[tokio::test]
    async fn dispatch_to_specific_user() {
        let manager = SseManager::new();
        let user_id = Uuid::new_v4();
        let other_user = Uuid::new_v4();

        let (_tx, mut rx) = manager.register(user_id).await;
        let (_tx_other, _rx_other) = manager.register(other_user).await;

        let alert = crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Test".to_string(),
            description: "Test description".to_string(),
            entity_id: None,
            entity_name: None,
            user_ids: vec![user_id],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 1, "Should deliver to one user");

        // Verify the user received the event
        if let Some(event) = rx.try_recv().ok() {
            assert_eq!(event.event, "new_warning");
        }
    }

    #[tokio::test]
    async fn broadcast_to_all_users() {
        let manager = SseManager::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        let (_tx_a, mut rx_a) = manager.register(user_a).await;
        let (_tx_b, mut rx_b) = manager.register(user_b).await;

        let alert = crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::SystemAlert,
            severity: apex_core::alert_config::AlertSeverity::Info,
            title: "Broadcast".to_string(),
            description: "Broadcast test".to_string(),
            entity_id: None,
            entity_name: None,
            user_ids: vec![], // empty = broadcast
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 2, "Should broadcast to both users");

        assert!(rx_a.try_recv().is_ok());
        assert!(rx_b.try_recv().is_ok());
    }
}
