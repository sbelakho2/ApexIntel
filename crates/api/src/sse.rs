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

use crate::alert_router::{AlertAudience, AlertRouter, AlertRoutingDecision};
use anyhow::Result;
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{DateTime, Utc};
use futures_util::stream::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// The subset of an alert sent to SSE clients. Deliberately excludes
/// `audience` (co-recipient principal UUIDs) and `metadata` (internal worker
/// fields); `link` is built server-side from the alert's own metadata.
#[derive(Debug, Clone, Serialize)]
struct ClientAlert<'a> {
    id: Uuid,
    event_type: &'a str,
    severity: apex_core::alert_config::AlertSeverity,
    title: &'a str,
    description: &'a str,
    entity_ids: &'a [Uuid],
    entity_name: Option<&'a str>,
    link: Option<String>,
    created_at: DateTime<Utc>,
}

impl<'a> From<&'a crate::alert_router::AlertEvent> for ClientAlert<'a> {
    fn from(alert: &'a crate::alert_router::AlertEvent) -> Self {
        Self {
            id: alert.id,
            event_type: alert.event_type.as_str(),
            severity: alert.severity,
            title: &alert.title,
            description: &alert.description,
            entity_ids: &alert.entity_ids,
            entity_name: alert.entity_name.as_deref(),
            link: alert_client_link(alert),
            created_at: alert.created_at,
        }
    }
}

/// Server-built destination for a notification click. Only ids the server
/// itself persisted are used, so a client cannot choose a redirect target.
///
/// N1: the alert's own id is NOT a warning or insight id, so an alert without
/// a persisted `warning_id`/`insight_id` gets no link instead of a fabricated
/// URL that 404s.
fn alert_client_link(alert: &crate::alert_router::AlertEvent) -> Option<String> {
    let id_field = |key: &str| {
        alert
            .metadata
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    if let Some(warning_id) = id_field("warning_id") {
        return Some(format!("/warnings/{warning_id}"));
    }
    if let Some(insight_id) = id_field("insight_id") {
        return Some(format!("/insights/{insight_id}"));
    }
    None
}

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

/// Maximum concurrent SSE streams per principal. Excess connections evict the
/// oldest, so one principal cannot pin an unbounded number of streams.
const SSE_MAX_CONNECTIONS_PER_USER: usize = 8;

/// Maximum lifetime of one SSE stream. The stream is closed after this and the
/// client's native reconnect re-authenticates, so a disabled session stops
/// receiving alerts within the cap.
const SSE_CONNECTION_MAX_SECS: u64 = 900;

/// Maximum number of recently dispatched events retained for `Last-Event-ID`
/// reconnects. Older events are evicted; a client whose cursor has been evicted
/// receives a `resync` event and is expected to refetch canonical state.
const SSE_REPLAY_CAPACITY: usize = 256;

/// Event type emitted when a reconnecting client's `Last-Event-ID` is outside
/// the replay window. The client must resynchronise from the canonical REST
/// state (for example `GET /warnings/unread-count`) instead of assuming its
/// incrementally updated view is complete.
pub const SSE_RESYNC_EVENT: &str = "resync";

/// Build the canonical-state-resync marker event.
pub fn resync_event(reason: &str) -> SseEvent {
    SseEvent::new(
        SSE_RESYNC_EVENT,
        serde_json::json!({ "reason": reason }).to_string(),
    )
}

/// Manages per-user SSE connections and dispatches alerts to connected clients.
///
/// Each authenticated user may have multiple browser tabs open, each of which
/// registers a separate sender channel. When an alert arrives, it is fanned out
/// to all senders registered for the target users.
pub struct SseManager {
    /// Active SSE connections keyed by user_id.
    connections: Arc<RwLock<HashMap<Uuid, Vec<mpsc::Sender<SseEvent>>>>>,
    /// Bounded log of recently dispatched events, used to replay anything a
    /// reconnecting client missed (`Last-Event-ID`). Retained process-wide
    /// because the event stream is a shared broadcast, but every entry keeps
    /// its explicit audience so a replay can never leak another user's alert.
    recent: Arc<Mutex<VecDeque<ReplayEntry>>>,
    /// Alert event ids already dispatched by this process. A JetStream
    /// redelivery (crash before ack, NAK) must not duplicate a client event.
    processed_alerts: Arc<Mutex<VecDeque<Uuid>>>,
}

/// A retained event plus the audience it was addressed to. The audience is the
/// explicit [`AlertAudience`] — never an "empty means broadcast" convention.
struct ReplayEntry {
    event: SseEvent,
    audience: AlertAudience,
}

impl ReplayEntry {
    fn is_visible_to(&self, user_id: Uuid) -> bool {
        match &self.audience {
            AlertAudience::Broadcast => true,
            AlertAudience::Users(targets) => targets.contains(&user_id),
        }
    }
}

/// Maximum number of alert ids retained for redelivery deduplication.
const SSE_PROCESSED_CAPACITY: usize = 1024;

impl SseManager {
    /// Create a new empty SSE manager.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            recent: Arc::new(Mutex::new(VecDeque::with_capacity(SSE_REPLAY_CAPACITY))),
            processed_alerts: Arc::new(Mutex::new(VecDeque::with_capacity(SSE_PROCESSED_CAPACITY))),
        }
    }

    /// Claim an alert id for processing. Returns `false` when this alert was
    /// already dispatched, so a JetStream redelivery is acked without emitting
    /// a duplicate client event.
    pub async fn claim_alert(&self, alert_id: Uuid) -> bool {
        let mut processed = self.processed_alerts.lock().await;
        if processed.contains(&alert_id) {
            return false;
        }
        if processed.len() >= SSE_PROCESSED_CAPACITY {
            processed.pop_front();
        }
        processed.push_back(alert_id);
        true
    }

    /// Register a new SSE connection for a user.
    ///
    /// Returns a sender and receiver pair. The sender is stored internally for
    /// dispatch and should be passed to [`unregister`](SseManager::unregister)
    /// when the connection closes.
    pub async fn register(
        &self,
        user_id: Uuid,
    ) -> (mpsc::Sender<SseEvent>, mpsc::Receiver<SseEvent>) {
        self.register_with_last_event_id(user_id, None).await
    }

    /// Register a reconnecting SSE connection.
    ///
    /// When `last_event_id` is provided and still inside the replay window, all
    /// events after it that are visible to `user_id` (broadcasts plus alerts
    /// addressed to them) are queued to the new connection. When the cursor is
    /// unknown or already evicted, a [`SSE_RESYNC_EVENT`] marker is queued so
    /// the client can refetch canonical state instead of silently missing
    /// events.
    pub async fn register_with_last_event_id(
        &self,
        user_id: Uuid,
        last_event_id: Option<&str>,
    ) -> (mpsc::Sender<SseEvent>, mpsc::Receiver<SseEvent>) {
        let (tx, rx) = mpsc::channel(SSE_CHANNEL_CAPACITY);
        let last_event_id = last_event_id
            .filter(|id| !id.is_empty())
            .map(str::to_string);

        // Hold both locks (connections, then recent — the same order
        // `dispatch_alert` takes) while snapshotting and publishing the sender.
        // This closes the race where an alert could be delivered live *and*
        // replayed, or slip between the snapshot and registration and be lost.
        let (replay, resync) = {
            let mut conns = self.connections.write().await;
            let recent = self.recent.lock().await;

            let (replay, resync) = match last_event_id.as_deref() {
                None => (Vec::new(), false),
                Some(last_id) => match recent.iter().position(|entry| entry.event.id == last_id) {
                    Some(position) => (
                        recent
                            .iter()
                            .skip(position + 1)
                            .filter(|entry| entry.is_visible_to(user_id))
                            .map(|entry| entry.event.clone())
                            .collect(),
                        false,
                    ),
                    None => (Vec::new(), true),
                },
            };

            let senders = conns.entry(user_id).or_default();
            if senders.len() >= SSE_MAX_CONNECTIONS_PER_USER {
                // Evict the oldest stream: a principal gets a bounded number
                // of concurrent connections.
                senders.remove(0);
            }
            senders.push(tx.clone());
            info!(
                user_id = %user_id,
                total_connections = conns.get(&user_id).map_or(0, Vec::len),
                "SSE connection registered"
            );

            (replay, resync)
        };

        if resync {
            let _ = tx.try_send(resync_event("replay_window_exhausted"));
            info!(
                user_id = %user_id,
                "SSE reconnect cursor outside replay window — resync requested"
            );
        } else if !replay.is_empty() {
            let replayed = replay.len();
            for event in replay {
                // The bounded channel can only overflow if a client reconnects
                // while far behind; the resync marker above covers that case on
                // the next reconnect.
                let _ = tx.try_send(event);
            }
            info!(user_id = %user_id, replayed, "SSE reconnect replayed missed events");
        }

        (tx, rx)
    }

    /// Remove a disconnected SSE connection for a user.
    pub async fn unregister(&self, user_id: Uuid, tx: &mpsc::Sender<SseEvent>) {
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

    /// Dispatch an [`AlertEvent`] to the SSE clients its explicit
    /// [`AlertAudience`] addresses: a `Broadcast` reaches every connected
    /// user, `Users(list)` reaches exactly those users, and an empty `Users`
    /// list reaches nobody. The audience is never inferred from emptiness.
    ///
    /// Returns the number of clients the event was sent to. Events are dropped
    /// for clients whose buffer is full (slow consumers).
    pub async fn dispatch_alert(&self, alert: &crate::alert_router::AlertEvent) -> usize {
        // Never serialize the internal event: it carries the full audience
        // (every other recipient's principal UUID) and the worker's raw
        // metadata. Clients receive only the fields they render, with the
        // link built server-side.
        let client_alert = ClientAlert::from(alert);
        let event = SseEvent::new(
            alert.event_type.as_str(),
            serde_json::to_string(&client_alert).unwrap_or_else(|_| "{}".to_string()),
        );

        let conns = self.connections.read().await;
        let mut sent_count = 0usize;

        // Retain the event so a reconnecting client can replay anything it
        // missed while the stream was down. Record it even when nobody was
        // connected: a targeted alert that arrived while the user was offline
        // must still be replayed on their next connection. Recording happens
        // under the connections read lock (same lock order as
        // `register_with_last_event_id`) so a concurrent registration either
        // snapshots this event or receives it live, never both.
        {
            let mut recent = self.recent.lock().await;
            if recent.len() >= SSE_REPLAY_CAPACITY {
                recent.pop_front();
            }
            recent.push_back(ReplayEntry {
                event: event.clone(),
                audience: alert.audience.clone(),
            });
        }

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
                // Broadcast to all connected users
                for senders in conns.values() {
                    deliver(senders, &event, &mut sent_count);
                }
            }
            AlertAudience::Users(user_ids) => {
                // Send only to specified users; an empty list sends to nobody.
                for user_id in user_ids {
                    if let Some(senders) = conns.get(user_id) {
                        deliver(senders, &event, &mut sent_count);
                    }
                }
            }
        }

        trace!(
            alert_id = %alert.id,
            event_type = %alert.event_type,
            sent_to = sent_count,
            "Alert dispatched to SSE clients"
        );

        sent_count
    }

    /// Number of events currently retained for `Last-Event-ID` replay.
    pub async fn replay_len(&self) -> usize {
        self.recent.lock().await.len()
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

        // Ensure the stream exists with the ONE canonical config defined in
        // `apex-shared`. The helper is idempotent and also reconciles an
        // existing stream whose mutable config differs (for example one the
        // API created before the worker ran, which would otherwise keep the
        // broker's 2-minute duplicate-window default; audit #78).
        if let Err(e) = apex_shared::ensure_alerts_stream(&jetstream).await {
            warn!(error = %e, "Failed to ensure JetStream stream 'alerts'");
        }

        // Create a push consumer. The subject filter excludes the
        // `alerts.dead_letter.*` subjects so a dead-lettered payload is never
        // consumed again by this bridge.
        //
        // Delivery order is receive → deserialize → route → dispatch → ACK:
        // the message is acked only after it was processed, so a crash or
        // routing failure retries instead of silently losing the alert.
        let consumer_config = async_nats::jetstream::consumer::push::Config {
            durable_name: Some("sse_bridge".to_string()),
            deliver_subject: format!("sse_bridge.deliver.{}", Uuid::new_v4()),
            // Restart guarantee: this bridge is REALTIME. `DeliverPolicy::New`
            // means a redeploy may drop alerts that were unpublished to this
            // process; the in-memory replay ring is the only client-replay
            // mechanism. The durable name exists so JetStream retries
            // *routing* failures within a process lifetime (NAK/redelivery),
            // not to replay history across redeploys. Deliberate choice.
            deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
            ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
            ack_wait: Duration::from_secs(30),
            // A transient policy-DB outage must survive far longer than a few
            // redeliveries; 20 deliveries at a 30s ack-wait is ~10 minutes of
            // retry budget before JetStream stops.
            max_deliver: 20,
            filter_subject: "alerts.events.>".to_string(),
            ..Default::default()
        };

        // A durable consumer's configuration is immutable. Recreate it only
        // when the desired shape actually differs so a restart does not
        // discard messages the previous process had delivered-but-unacked.
        let needs_recreate = match jetstream
            .get_consumer_from_stream::<async_nats::jetstream::consumer::push::Config, _, _>(
                "sse_bridge",
                "alerts",
            )
            .await
        {
            Ok(mut existing) => match existing.info().await {
                Ok(info) => {
                    let current = &info.config;
                    current.filter_subject != consumer_config.filter_subject
                        || current.ack_policy != consumer_config.ack_policy
                        || current.ack_wait != consumer_config.ack_wait
                        || current.max_deliver != consumer_config.max_deliver
                        || current.deliver_policy != consumer_config.deliver_policy
                }
                Err(_) => true,
            },
            Err(_) => true,
        };
        if needs_recreate {
            if let Err(e) = jetstream
                .delete_consumer_from_stream("alerts", "sse_bridge")
                .await
            {
                debug!(error = %e, "No existing sse_bridge consumer to delete (expected on first start)");
            }
        }

        let consumer: async_nats::jetstream::consumer::PushConsumer = match jetstream
            .get_consumer_from_stream::<async_nats::jetstream::consumer::push::Config, _, _>(
                "sse_bridge",
                "alerts",
            )
            .await
        {
            Ok(existing) => existing,
            Err(_) => match jetstream
                .create_consumer_on_stream(consumer_config, "alerts")
                .await
            {
                Ok(created) => created,
                Err(e) => {
                    error!(error = %e, "Failed to create NATS JetStream consumer");
                    return;
                }
            },
        };

        info!("NATS JetStream consumer 'sse_bridge' started");

        let manager = self.clone();
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

                // Periodic stranded-message visibility: JetStream stops
                // redelivering after `max_deliver`; a persistent policy outage
                // would leave alerts pending with no application trace. Poll
                // the consumer info on idle ticks and report the backlog.
                let mut idle_ticks: u32 = 0;
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), messages.next()).await {
                        Ok(Some(Ok(msg))) => {
                            let payload = msg.payload.clone();

                            // 1. Deserialize + validate. A malformed event is
                            //    permanent: dead-letter it and ACK so it cannot
                            //    poison the consumer forever.
                            let alert = match serde_json::from_slice::<
                                crate::alert_router::AlertEvent,
                            >(&payload)
                            {
                                Ok(alert) => alert,
                                Err(e) => {
                                    warn!(
                                        error = %e,
                                        "Malformed alert payload — dead-lettering"
                                    );
                                    // Dead-letter only after the broker ACKs
                                    // the DLQ publish: a client-side accept
                                    // without the broker ACK must not consume
                                    // the original, or the malformed payload
                                    // is lost instead of durably recorded.
                                    let dead_lettered = match jetstream
                                        .publish("alerts.dead_letter.malformed", payload.clone())
                                        .await
                                    {
                                        Ok(ack_future) => match ack_future.await {
                                            Ok(_ack) => true,
                                            Err(dlq_error) => {
                                                warn!(
                                                    error = %dlq_error,
                                                    "Dead-letter publish was not ACKed by JetStream"
                                                );
                                                false
                                            }
                                        },
                                        Err(dlq_error) => {
                                            warn!(
                                                error = %dlq_error,
                                                "Failed to start dead-letter publish"
                                            );
                                            false
                                        }
                                    };
                                    if dead_lettered {
                                        if let Err(e) = msg.ack().await {
                                            warn!(error = %e, "Failed to ack dead-lettered NATS message");
                                        }
                                    } else if let Err(e) = msg
                                        .ack_with(async_nats::jetstream::AckKind::Nak(None))
                                        .await
                                    {
                                        warn!(
                                            error = %e,
                                            "Failed to NAK after failed dead-letter publish"
                                        );
                                    }
                                    continue;
                                }
                            };

                            // 2. Resolve routing. A storage failure is
                            //    retryable: NAK so JetStream redelivers, never
                            //    ack and never leak through unresolved policy.
                            match alert_router.route_alert(&alert).await {
                                AlertRoutingDecision::RetryableFailure(error) => {
                                    warn!(
                                        alert_id = %alert.id,
                                        %error,
                                        "Alert routing unresolved — NAK, will retry"
                                    );
                                    if let Err(e) = msg
                                        .ack_with(async_nats::jetstream::AckKind::Nak(None))
                                        .await
                                    {
                                        warn!(error = %e, "Failed to NAK NATS message");
                                    }
                                    continue;
                                }
                                AlertRoutingDecision::NoRecipients => {
                                    // Resolved: nobody is authorized for this
                                    // event. Ack and drop — this is an
                                    // authorization outcome, not a failure.
                                    debug!(
                                        alert_id = %alert.id,
                                        "Alert routed to no recipients — dropping"
                                    );
                                    if let Err(e) = msg.ack().await {
                                        warn!(error = %e, "Failed to ack suppressed NATS message");
                                    }
                                    continue;
                                }
                                AlertRoutingDecision::Broadcast => {
                                    if manager.claim_alert(alert.id).await {
                                        let routed = alert.with_audience(AlertAudience::Broadcast);
                                        manager.dispatch_alert(&routed).await;
                                    } else {
                                        debug!(
                                            alert_id = %alert.id,
                                            "Duplicate delivery of alert — skipping dispatch"
                                        );
                                    }
                                    if let Err(e) = msg.ack().await {
                                        warn!(error = %e, "Failed to ack NATS message");
                                    }
                                }
                                AlertRoutingDecision::Targets(user_ids) => {
                                    if manager.claim_alert(alert.id).await {
                                        let routed =
                                            alert.with_audience(AlertAudience::Users(user_ids));
                                        manager.dispatch_alert(&routed).await;
                                    } else {
                                        debug!(
                                            alert_id = %alert.id,
                                            "Duplicate delivery of alert — skipping dispatch"
                                        );
                                    }
                                    if let Err(e) = msg.ack().await {
                                        warn!(error = %e, "Failed to ack NATS message");
                                    }
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
                            // Timeout — normal; use idle ticks for delivery
                            // health reporting.
                            idle_ticks = idle_ticks.wrapping_add(1);
                            if idle_ticks.is_multiple_of(12) {
                                if let Ok(mut handle) = jetstream
                                    .get_consumer_from_stream::<
                                        async_nats::jetstream::consumer::push::Config,
                                        _,
                                        _,
                                    >("sse_bridge", "alerts")
                                    .await
                                {
                                    if let Ok(info) = handle.info().await {
                                        if info.num_pending > 0
                                            || info.num_redelivered > 0
                                            || info.num_ack_pending > 0
                                        {
                                            warn!(
                                                num_pending = info.num_pending,
                                                num_redelivered = info.num_redelivered,
                                                num_ack_pending = info.num_ack_pending,
                                                "SSE bridge delivery backlog: alerts pending or being redelivered"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                warn!("NATS consumer stream closed — reconnecting");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
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
        shutdown: tokio_util::sync::CancellationToken,
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
        // End the stream on process shutdown (SIGTERM/ctrl-c) so graceful
        // drain is not blocked by open streams, and cap the stream's lifetime
        // so long-lived connections periodically re-authenticate — a disabled
        // user stops receiving alerts within the cap.
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
            .take_until(async move {
                tokio::select! {
                    _ = shutdown.cancelled() => {},
                    _ = tokio::time::sleep(Duration::from_secs(SSE_CONNECTION_MAX_SECS)) => {},
                }
            })
            .map(move |event| {
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
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert_router::AlertEventType;

    fn make_test_event() -> SseEvent {
        SseEvent::new("test", r#"{"msg":"hello"}"#)
    }

    fn alert_for(user_id: Uuid, title: &str) -> crate::alert_router::AlertEvent {
        crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: title.to_string(),
            description: "description".to_string(),
            entity_ids: vec![],
            entity_name: None,
            audience: AlertAudience::Users(vec![user_id]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    // ── N1: links only for persisted warning/insight ids ────────────────

    #[test]
    fn alert_link_uses_persisted_warning_or_insight_id() {
        let mut alert = alert_for(Uuid::new_v4(), "Warning");
        alert.metadata = serde_json::json!({ "warning_id": "w-123" });
        assert_eq!(
            alert_client_link(&alert).as_deref(),
            Some("/warnings/w-123")
        );

        let mut alert = alert_for(Uuid::new_v4(), "Insight");
        alert.event_type = AlertEventType::NewInsight;
        alert.metadata = serde_json::json!({ "insight_id": "i-456" });
        assert_eq!(
            alert_client_link(&alert).as_deref(),
            Some("/insights/i-456")
        );
    }

    #[test]
    fn alert_link_is_none_without_a_persisted_id() {
        // The alert id is not a warning/insight id: no fabricated link.
        let alert = alert_for(Uuid::new_v4(), "No metadata");
        assert_eq!(alert_client_link(&alert), None);

        let mut insight_alert = alert_for(Uuid::new_v4(), "Insight-shaped");
        insight_alert.event_type = AlertEventType::NewInsight;
        assert_eq!(alert_client_link(&insight_alert), None);

        let mut blank = alert_for(Uuid::new_v4(), "Blank id");
        blank.metadata = serde_json::json!({ "warning_id": "   ", "insight_id": null });
        assert_eq!(alert_client_link(&blank), None);
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
            entity_ids: vec![],
            entity_name: None,
            audience: AlertAudience::Users(vec![user_id]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 1, "Should deliver to one user");

        // Verify the user received the event
        if let Ok(event) = rx.try_recv() {
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
            entity_ids: vec![],
            entity_name: None,
            audience: AlertAudience::Broadcast,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };

        let sent = manager.dispatch_alert(&alert).await;
        assert_eq!(sent, 2, "Should broadcast to both users");

        assert!(rx_a.try_recv().is_ok());
        assert!(rx_b.try_recv().is_ok());
    }

    #[tokio::test]
    async fn dispatch_drops_for_full_buffer_without_blocking_or_panicking() {
        let manager = SseManager::new();
        let user_id = Uuid::new_v4();
        let (_tx, mut rx) = manager.register(user_id).await;

        let alert = crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Slow consumer".to_string(),
            description: "Buffer pressure".to_string(),
            entity_ids: vec![],
            entity_name: None,
            audience: AlertAudience::Users(vec![user_id]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };

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
        let alert = crate::alert_router::AlertEvent {
            id: Uuid::new_v4(),
            event_type: AlertEventType::SystemAlert,
            severity: apex_core::alert_config::AlertSeverity::Info,
            title: "Nobody".to_string(),
            description: "no subscribers".to_string(),
            entity_ids: vec![],
            entity_name: None,
            audience: AlertAudience::Users(vec![Uuid::new_v4()]),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        assert_eq!(manager.dispatch_alert(&alert).await, 0);
    }

    #[tokio::test]
    async fn reconnect_replays_events_after_last_event_id() {
        let manager = SseManager::new();
        let user = Uuid::new_v4();

        // First connection receives two alerts, then disconnects.
        let (tx, mut rx) = manager.register(user).await;
        manager.dispatch_alert(&alert_for(user, "first")).await;
        manager.dispatch_alert(&alert_for(user, "second")).await;
        let first = rx.try_recv().expect("first event");
        let second = rx.try_recv().expect("second event");
        manager.unregister(user, &tx).await;
        drop(rx);

        // Reconnect with the first event's cursor: only the second replays.
        let (_tx2, mut rx2) = manager
            .register_with_last_event_id(user, Some(first.id.as_str()))
            .await;
        let replayed = rx2.try_recv().expect("replayed event");
        assert_eq!(replayed.id, second.id);
        assert_eq!(replayed.event, "new_warning");
        assert!(
            rx2.try_recv().is_err(),
            "already-seen events must not replay"
        );
    }

    #[tokio::test]
    async fn reconnect_with_evicted_cursor_requests_canonical_resync() {
        let manager = SseManager::new();
        let user = Uuid::new_v4();
        let (tx, mut rx) = manager.register(user).await;
        manager.dispatch_alert(&alert_for(user, "delivered")).await;
        let _ = rx.try_recv().expect("delivered event");
        manager.unregister(user, &tx).await;

        let (_tx2, mut rx2) = manager
            .register_with_last_event_id(user, Some("00000000-0000-0000-0000-000000000000"))
            .await;
        let resync = rx2.try_recv().expect("resync event");
        assert_eq!(resync.event, SSE_RESYNC_EVENT);
        let payload: serde_json::Value = serde_json::from_str(&resync.data).expect("resync json");
        assert_eq!(payload["reason"], "replay_window_exhausted");
    }

    #[tokio::test]
    async fn fresh_connection_does_not_replay_or_request_resync() {
        let manager = SseManager::new();
        let other = Uuid::new_v4();
        let (tx, mut rx) = manager.register(other).await;
        manager
            .dispatch_alert(&alert_for(other, "broadcast-ish"))
            .await;
        let _ = rx.try_recv().expect("event");
        manager.unregister(other, &tx).await;

        let (_tx, mut fresh) = manager.register(Uuid::new_v4()).await;
        assert!(
            fresh.try_recv().is_err(),
            "a first connection must not receive replay or resync traffic"
        );
    }

    /// An alert addressed to A must never replay to B, even when B reconnects
    /// with a cursor that predates the alert.
    #[tokio::test]
    async fn replay_filters_targeted_alerts_to_their_recipients() {
        let manager = SseManager::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        let (_tx_a, mut rx_a) = manager.register(user_a).await;
        let (tx_b, mut rx_b) = manager.register(user_b).await;

        // Shared broadcast, then an alert targeted at A only.
        manager
            .dispatch_alert(&crate::alert_router::AlertEvent {
                audience: AlertAudience::Broadcast,
                ..alert_for(user_a, "broadcast")
            })
            .await;
        let broadcast_id = rx_a.try_recv().expect("broadcast for A").id;
        let _ = rx_b.try_recv().expect("broadcast for B");

        manager.dispatch_alert(&alert_for(user_a, "A only")).await;
        let _ = rx_a.try_recv().expect("A receives its alert");
        assert!(rx_b.try_recv().is_err(), "B must not observe A's alert");

        // B reconnects with the broadcast cursor: nothing else is visible.
        manager.unregister(user_b, &tx_b).await;
        drop(rx_b);
        let (_tx_b2, mut rx_b2) = manager
            .register_with_last_event_id(user_b, Some(&broadcast_id))
            .await;
        assert!(
            rx_b2.try_recv().is_err(),
            "replay must not leak another user's targeted alert"
        );

        // A reconnects with the same cursor and does get the missed alert.
        manager.unregister(user_a, &_tx_a).await;
        drop(rx_a);
        let (_tx_a2, mut rx_a2) = manager
            .register_with_last_event_id(user_a, Some(&broadcast_id))
            .await;
        let replayed = rx_a2.try_recv().expect("A's missed alert replays");
        assert_eq!(replayed.event, "new_warning");
    }

    /// A targeted alert dispatched while its recipient was offline must replay
    /// on their next connection.
    #[tokio::test]
    async fn offline_targeted_alert_is_retained_for_replay() {
        let manager = SseManager::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        // A broadcast establishes a shared cursor while only B is connected,
        // then B disconnects and A's alert is dispatched with A still offline.
        let (tx_b, mut rx_b) = manager.register(user_b).await;
        manager
            .dispatch_alert(&crate::alert_router::AlertEvent {
                audience: AlertAudience::Broadcast,
                ..alert_for(user_a, "broadcast")
            })
            .await;
        let broadcast_id = rx_b.try_recv().expect("broadcast for B").id;
        manager.unregister(user_b, &tx_b).await;
        drop(rx_b);

        manager
            .dispatch_alert(&alert_for(user_a, "offline alert"))
            .await;

        // B reconnects with the cursor and sees nothing new.
        let (_tx_b2, mut rx_b2) = manager
            .register_with_last_event_id(user_b, Some(&broadcast_id))
            .await;
        assert!(
            rx_b2.try_recv().is_err(),
            "offline alert addressed to A must not replay to B"
        );

        // A connects for the first time with that cursor and receives it.
        let (_tx_a, mut rx_a) = manager
            .register_with_last_event_id(user_a, Some(&broadcast_id))
            .await;
        let replayed = rx_a.try_recv().expect("A replays its offline alert");
        assert_eq!(replayed.event, "new_warning");
    }

    /// The replay window is bounded; a cursor evicted from it asks for a
    /// canonical-state resync instead of silently skipping events.
    #[tokio::test]
    async fn replay_window_is_bounded_and_evicted_cursor_resyncs() {
        let manager = SseManager::new();
        let user = Uuid::new_v4();
        let (tx, mut rx) = manager.register(user).await;

        let total = SSE_REPLAY_CAPACITY + 10;
        let mut ids = Vec::with_capacity(total);
        for i in 0..total {
            manager
                .dispatch_alert(&alert_for(user, &format!("event {i}")))
                .await;
            ids.push(rx.try_recv().expect("event").id);
        }

        assert_eq!(manager.replay_len().await, SSE_REPLAY_CAPACITY);

        manager.unregister(user, &tx).await;
        drop(rx);

        // A cursor before the retained window resyncs.
        let (_tx2, mut rx2) = manager
            .register_with_last_event_id(user, Some(&ids[0]))
            .await;
        let first = rx2.try_recv().expect("resync marker");
        assert_eq!(first.event, SSE_RESYNC_EVENT);

        // A cursor inside the window replays exactly the tail after it.
        let (_tx3, mut rx3) = manager
            .register_with_last_event_id(user, Some(&ids[total - 2]))
            .await;
        let tail: Vec<SseEvent> = std::iter::from_fn(|| rx3.try_recv().ok()).collect();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].id, ids[total - 1]);
    }
}
