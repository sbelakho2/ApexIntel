//! Transactional outbox publisher — the ONE alert publication path.
//!
//! ```text
//! warning INSERT + outbox event (one transaction)
//!     -> semantic triage
//!     -> publisher claims the outbox row (FOR UPDATE SKIP LOCKED)
//!     -> AlertEvaluator rules
//!     -> publish -> await JetStream ACK -> published_at -> commit
//!     -> NATS `alerts.events.*`
//!     -> API SSE consumer -> browser toast + bell
//! ```
//!
//! This module replaced the old warning-table-polling producer (which published
//! domain events for every new warning row independently of triage and the
//! AlertEvaluator). Alert events now have exactly one source — the outbox row
//! committed with its warning — and one publication implementation:
//!
//! * The ingress calls [`deliver_outbox_event`] right after triage for a
//!   low-latency delivery attempt.
//! * The background drain calls [`drain_once`] to redeliver everything left
//!   unpublished (crash, ACK failure, NATS outage, exhausted fast-path claim).
//!
//! Both claim rows with `FOR UPDATE SKIP LOCKED`, so a row is only ever
//! published by the holder of its lock; the other side skips it. No other
//! component publishes warning alerts to NATS.
//!
//! # Delivery guarantees
//! - A crash between the warning commit and the publish leaves the outbox row
//!   unpublished; the next drain retries it (at-least-once, never lost).
//! - `published_at` is stamped only after the JetStream ACK resolves inside the
//!   same transaction that holds the row lock; an ACK failure records
//!   `attempts`/`last_error` and leaves the row for retry.
//! - Events that exhaust [`MAX_OUTBOX_ATTEMPTS`] stop being selected, so a
//!   permanently unpublishable payload cannot starve newer alerts.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use tracing::{info, warn};
use uuid::Uuid;

use apex_store::postgres::{
    EventOutboxRow, OutboxBatch, OutboxDrainOutcome, PgStore, MAX_OUTBOX_ATTEMPTS,
};
use apex_worker::nats_stream::{AlertEvent, NatsPublisher};

use crate::alert_evaluator::{AlertEvaluator, DomainEvent};

/// Default path to the alert rules file.
const DEFAULT_ALERT_RULES_PATH: &str = "config/runtime/alert-rules.yaml";

/// How often the drain task looks for unpublished events.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum events locked per drain batch.
pub const DEFAULT_BATCH_SIZE: i64 = 25;

/// Upper bound for one publish + ACK round trip. Bounds how long a batch holds
/// its row locks when the broker accepts messages but never acknowledges them.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(15);

/// Resolve the alert rules path from the environment (`ALERT_RULES_PATH`).
fn alert_rules_path_from_env() -> String {
    std::env::var("ALERT_RULES_PATH").unwrap_or_else(|_| DEFAULT_ALERT_RULES_PATH.to_string())
}

/// Load the alert rules, if present. Missing/invalid rules must not stop alert
/// delivery: the base warning alert still publishes.
pub fn load_rules_evaluator() -> Option<Arc<AlertEvaluator>> {
    let path = alert_rules_path_from_env();
    match AlertEvaluator::load_from_path(&path) {
        Ok(evaluator) => {
            info!(rules_path = %path, "outbox publisher: alert rules loaded");
            Some(Arc::new(evaluator))
        }
        Err(e) => {
            warn!(
                rules_path = %path,
                error = %e,
                "outbox publisher: alert rules unavailable; publishing base alert events only"
            );
            None
        }
    }
}

/// Storage half of the drain: lock a batch or one specific event, then settle.
#[async_trait]
pub trait OutboxBatchStore: Send + Sync {
    /// Lock up to `limit` unpublished events (production:
    /// `SELECT ... FOR UPDATE SKIP LOCKED`) and return the held batch.
    async fn lock_batch(&self, limit: i64) -> anyhow::Result<Box<dyn LockedOutboxBatch>>;

    /// Lock one specific unpublished event; an empty batch means another
    /// publisher owns it, it is already published, or it is exhausted.
    async fn lock_event(&self, id: Uuid) -> anyhow::Result<Box<dyn LockedOutboxBatch>>;
}

/// A locked batch. Marks are applied by [`LockedOutboxBatch::commit`]; dropping
/// the batch without committing simulates the crash case (nothing marked).
#[async_trait]
pub trait LockedOutboxBatch: Send {
    fn locked_events(&self) -> Vec<EventOutboxRow>;
    fn is_empty(&self) -> bool;
    async fn mark_published(&mut self, id: Uuid) -> anyhow::Result<()>;
    async fn record_failure(&mut self, id: Uuid, error: &str) -> anyhow::Result<()>;
    async fn commit(self: Box<Self>) -> anyhow::Result<()>;
}

/// Publication half of the drain. Implementations must await the JetStream ACK
/// before returning `Ok`.
#[async_trait]
pub trait OutboxEventPublisher: Send + Sync {
    async fn publish_event(&self, event: &AlertEvent) -> anyhow::Result<()>;
}

#[async_trait]
impl OutboxBatchStore for PgStore {
    async fn lock_batch(&self, limit: i64) -> anyhow::Result<Box<dyn LockedOutboxBatch>> {
        Ok(Box::new(self.lock_unpublished_outbox(limit).await?))
    }

    async fn lock_event(&self, id: Uuid) -> anyhow::Result<Box<dyn LockedOutboxBatch>> {
        Ok(Box::new(self.lock_outbox_event(id).await?))
    }
}

#[async_trait]
impl LockedOutboxBatch for OutboxBatch {
    fn locked_events(&self) -> Vec<EventOutboxRow> {
        OutboxBatch::events(self).to_vec()
    }

    fn is_empty(&self) -> bool {
        OutboxBatch::is_empty(self)
    }

    async fn mark_published(&mut self, id: Uuid) -> anyhow::Result<()> {
        OutboxBatch::mark_published(self, id).await
    }

    async fn record_failure(&mut self, id: Uuid, error: &str) -> anyhow::Result<()> {
        OutboxBatch::record_failure(self, id, error).await
    }

    async fn commit(self: Box<Self>) -> anyhow::Result<()> {
        OutboxBatch::commit(*self).await
    }
}

/// Production publisher: evaluates alert rules, then publishes the alert event
/// and awaits the broker ACK.
pub struct NatsAlertEventPublisher {
    publisher: NatsPublisher,
    evaluator: Option<Arc<AlertEvaluator>>,
}

impl NatsAlertEventPublisher {
    pub fn new(publisher: NatsPublisher, evaluator: Option<Arc<AlertEvaluator>>) -> Self {
        Self {
            publisher,
            evaluator,
        }
    }

    /// Domain-event view of a stored warning alert, for rule evaluation.
    fn domain_event_for(event: &AlertEvent) -> DomainEvent {
        let warning_type = event
            .metadata
            .get("warning_type")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        DomainEvent {
            id: event.id,
            source: format!("warning_{warning_type}"),
            severity: event.severity.as_str().to_string(),
            title: event.title.clone(),
            description: event.description.clone(),
            entity_id: event.entity_id,
            entity_name: event.entity_name.clone(),
            metadata: event.metadata.clone(),
            created_at: event.created_at,
        }
    }
}

#[async_trait]
impl OutboxEventPublisher for NatsAlertEventPublisher {
    async fn publish_event(&self, event: &AlertEvent) -> anyhow::Result<()> {
        // An unavailable transport must fail the row, never let it be stamped
        // published: the drain retries it instead of dropping the alert.
        if !self.publisher.is_connected() {
            anyhow::bail!(
                "NATS transport unavailable; leaving outbox event {} unpublished for retry",
                event.id
            );
        }

        if let Some(evaluator) = &self.evaluator {
            let domain = Self::domain_event_for(event);
            for rule_alert in evaluator.evaluate(&domain).await {
                if let Err(error) = self.publisher.publish_alert(&rule_alert).await {
                    // The evaluator already recorded the rule's cooldown before
                    // the publish; release it so the outbox retry redelivers the
                    // rule alert instead of silently suppressing it.
                    evaluator.forget_firings(&domain).await;
                    return Err(error).context("failed to publish rule-derived alert");
                }
            }
        }

        self.publisher.publish_alert(event).await
    }
}

/// Publish every event in a locked batch, then commit.
///
/// Returns the per-stage outcome plus the last publish error (for caller
/// reporting). A publish failure records the attempt and leaves the event
/// unpublished; it never aborts the rest of the batch.
async fn publish_locked_batch(
    mut batch: Box<dyn LockedOutboxBatch>,
    publisher: &dyn OutboxEventPublisher,
) -> anyhow::Result<(OutboxDrainOutcome, Option<String>)> {
    let events = batch.locked_events();
    let mut outcome = OutboxDrainOutcome::default();
    let mut last_error: Option<String> = None;

    for row in events {
        let result = match serde_json::from_value::<AlertEvent>(row.payload.clone()) {
            Ok(event) => {
                let attempted =
                    tokio::time::timeout(PUBLISH_TIMEOUT, publisher.publish_event(&event)).await;
                match attempted {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "publish exceeded {PUBLISH_TIMEOUT:?} and was abandoned"
                    )),
                }
            }
            Err(e) => Err(anyhow::anyhow!(
                "outbox event {} payload is not a valid AlertEvent: {e}",
                row.id
            )),
        };

        match result {
            Ok(()) => {
                batch.mark_published(row.id).await?;
                outcome.published += 1;
            }
            Err(error) => {
                let next_attempt = row.attempts + 1;
                if next_attempt >= MAX_OUTBOX_ATTEMPTS {
                    warn!(
                        outbox_id = %row.id,
                        attempts = next_attempt,
                        error = %error,
                        "outbox publisher: event exhausted its attempts and will not be retried"
                    );
                } else {
                    warn!(
                        outbox_id = %row.id,
                        attempts = next_attempt,
                        error = %error,
                        "outbox publisher: publish failed; event remains unpublished for retry"
                    );
                }
                batch.record_failure(row.id, &error.to_string()).await?;
                outcome.failed += 1;
                last_error = Some(error.to_string());
            }
        }
    }

    batch.commit().await?;
    Ok((outcome, last_error))
}

/// Drain one outbox batch.
pub async fn drain_once(
    store: &dyn OutboxBatchStore,
    publisher: &dyn OutboxEventPublisher,
    limit: i64,
) -> anyhow::Result<OutboxDrainOutcome> {
    let batch = store.lock_batch(limit).await?;
    let (outcome, _) = publish_locked_batch(batch, publisher).await?;
    Ok(outcome)
}

/// Deliver one specific outbox event (the ingress fast path).
///
/// * `Ok(true)` — this call held the row lock, published the event and awaited
///   its ACK, then stamped `published_at`.
/// * `Ok(false)` — another publisher owns the row, it is already published, or
///   the row is exhausted; delivery is left to the drain.
/// * `Err` — this call held the row, the publish failed, and the failure was
///   recorded for retry.
pub async fn deliver_outbox_event(
    store: &dyn OutboxBatchStore,
    publisher: &dyn OutboxEventPublisher,
    id: Uuid,
) -> anyhow::Result<bool> {
    let batch = store.lock_event(id).await?;
    if batch.is_empty() {
        return Ok(false);
    }
    let (outcome, last_error) = publish_locked_batch(batch, publisher).await?;
    if outcome.published > 0 {
        return Ok(true);
    }
    Err(anyhow::anyhow!(
        "{}",
        last_error.unwrap_or_else(|| format!("outbox event {id} was not published"))
    ))
}

/// Spawn the canonical outbox drain task.
///
/// Returns immediately; all work happens on a background tokio task. When NATS
/// is not configured the task logs and exits (events stay queued for a later
/// restart with NATS configured); when NATS is temporarily unreachable the task
/// reconnects on its poll interval.
pub fn spawn(store: Arc<PgStore>, evaluator: Option<Arc<AlertEvaluator>>) {
    let enabled = std::env::var("REALTIME_ALERTS_ENABLED")
        .ok()
        .map(|v| apex_core::env::parse_truthy_flag(&v))
        .unwrap_or(true);
    if !enabled {
        info!("outbox alert publisher disabled (REALTIME_ALERTS_ENABLED=false)");
        return;
    }

    tokio::spawn(async move {
        let nats_url = match std::env::var("NATS_URL") {
            Ok(url) if !url.trim().is_empty() => url.trim().to_string(),
            _ => {
                info!(
                    "outbox alert publisher: NATS_URL unset; alert events stay queued until \
                     a worker with NATS configured drains them"
                );
                return;
            }
        };

        let mut publisher = NatsPublisher::connect(&nats_url).await;
        let mut interval = tokio::time::interval(DEFAULT_POLL_INTERVAL);

        info!("outbox alert publisher started (single canonical alert path)");
        loop {
            interval.tick().await;

            if !publisher.is_connected() {
                publisher = NatsPublisher::connect(&nats_url).await;
                if !publisher.is_connected() {
                    continue;
                }
            }

            let drain_publisher =
                NatsAlertEventPublisher::new(publisher.clone(), evaluator.clone());
            match drain_once(store.as_ref(), &drain_publisher, DEFAULT_BATCH_SIZE).await {
                Ok(outcome) if outcome.published > 0 || outcome.failed > 0 => {
                    info!(
                        published = outcome.published,
                        failed = outcome.failed,
                        "outbox publisher: drained alert events"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, "outbox publisher: drain failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use apex_worker::nats_stream::{JetStreamTransport, PendingPublishAck};

    fn sample_event(id: Uuid) -> AlertEvent {
        AlertEvent {
            id,
            event_type: apex_worker::nats_stream::AlertEventType::NewWarning,
            severity: apex_core::alert_config::AlertSeverity::High,
            title: "Persisted warning".to_string(),
            description: "Committed before the crash".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_name: Some("Acme".to_string()),
            audience: apex_core::alert_config::AlertAudience::Users(vec![]),
            metadata: serde_json::json!({"warning_type": "volume_anomaly"}),
            created_at: chrono::Utc::now(),
        }
    }

    fn outbox_row(id: Uuid, payload: serde_json::Value) -> EventOutboxRow {
        EventOutboxRow {
            id,
            aggregate_type: "warning".to_string(),
            aggregate_id: Uuid::new_v4(),
            event_type: "new_warning".to_string(),
            payload,
            created_at: chrono::Utc::now(),
            published_at: None,
            attempts: 0,
            last_error: None,
        }
    }

    #[derive(Default)]
    struct MemoryOutbox {
        events: Arc<Mutex<Vec<EventOutboxRow>>>,
        published: Arc<Mutex<HashSet<Uuid>>>,
        claims: Arc<Mutex<HashSet<Uuid>>>,
        failures: Arc<Mutex<Vec<(Uuid, String)>>>,
    }

    impl MemoryOutbox {
        fn with_events(events: Vec<EventOutboxRow>) -> Self {
            Self {
                events: Arc::new(Mutex::new(events)),
                published: Arc::new(Mutex::new(HashSet::new())),
                claims: Arc::new(Mutex::new(HashSet::new())),
                failures: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn unpublished(&self) -> Vec<Uuid> {
            let published = self.published.lock().unwrap();
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|row| !published.contains(&row.id))
                .map(|row| row.id)
                .collect()
        }

        fn published_ids(&self) -> HashSet<Uuid> {
            self.published.lock().unwrap().clone()
        }

        fn failure_count(&self) -> usize {
            self.failures.lock().unwrap().len()
        }

        fn lock_rows(&self, limit: Option<usize>, only: Option<Uuid>) -> MemoryBatch {
            let published = self.published.lock().unwrap().clone();
            let mut claims = self.claims.lock().unwrap();
            let mut locked = Vec::new();
            for row in self.events.lock().unwrap().iter() {
                if (Some(row.id) == only || only.is_none())
                    && !published.contains(&row.id)
                    && row.attempts < MAX_OUTBOX_ATTEMPTS
                    && !claims.contains(&row.id)
                {
                    claims.insert(row.id);
                    locked.push(row.clone());
                    if let Some(limit) = limit {
                        if locked.len() >= limit {
                            break;
                        }
                    }
                }
            }
            drop(claims);
            MemoryBatch {
                published: Arc::clone(&self.published),
                claims: Arc::clone(&self.claims),
                failures: Arc::clone(&self.failures),
                locked,
                marked: Vec::new(),
                failed: Vec::new(),
            }
        }
    }

    struct MemoryBatch {
        published: Arc<Mutex<HashSet<Uuid>>>,
        claims: Arc<Mutex<HashSet<Uuid>>>,
        failures: Arc<Mutex<Vec<(Uuid, String)>>>,
        locked: Vec<EventOutboxRow>,
        marked: Vec<Uuid>,
        failed: Vec<(Uuid, String)>,
    }

    #[async_trait]
    impl LockedOutboxBatch for MemoryBatch {
        fn locked_events(&self) -> Vec<EventOutboxRow> {
            self.locked.clone()
        }

        fn is_empty(&self) -> bool {
            self.locked.is_empty()
        }

        async fn mark_published(&mut self, id: Uuid) -> anyhow::Result<()> {
            self.marked.push(id);
            Ok(())
        }

        async fn record_failure(&mut self, id: Uuid, error: &str) -> anyhow::Result<()> {
            self.failed.push((id, error.to_string()));
            Ok(())
        }

        async fn commit(self: Box<Self>) -> anyhow::Result<()> {
            {
                let mut published = self.published.lock().unwrap();
                for id in &self.marked {
                    published.insert(*id);
                }
            }
            {
                let mut failures = self.failures.lock().unwrap();
                for (id, error) in &self.failed {
                    failures.push((*id, error.clone()));
                }
            }
            let mut claims = self.claims.lock().unwrap();
            for row in &self.locked {
                claims.remove(&row.id);
            }
            Ok(())
        }
    }

    #[async_trait]
    impl OutboxBatchStore for MemoryOutbox {
        async fn lock_batch(&self, limit: i64) -> anyhow::Result<Box<dyn LockedOutboxBatch>> {
            Ok(Box::new(self.lock_rows(Some(limit.max(1) as usize), None)))
        }

        async fn lock_event(&self, id: Uuid) -> anyhow::Result<Box<dyn LockedOutboxBatch>> {
            Ok(Box::new(self.lock_rows(None, Some(id))))
        }
    }

    #[derive(Default)]
    struct RecordingPublisher {
        delivered: Mutex<Vec<Uuid>>,
        fail_ids: HashSet<Uuid>,
    }

    #[async_trait]
    impl OutboxEventPublisher for RecordingPublisher {
        async fn publish_event(&self, event: &AlertEvent) -> anyhow::Result<()> {
            if self.fail_ids.contains(&event.id) {
                anyhow::bail!("simulated JetStream ACK failure for {}", event.id);
            }
            self.delivered.lock().unwrap().push(event.id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn outbox_drains_after_crash_between_commit_and_publish() {
        // Simulate the crash: the warning + outbox row committed, then the
        // process died before publishing anything.
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        assert_eq!(store.unpublished(), vec![event.id]);

        let publisher = RecordingPublisher::default();
        let outcome = drain_once(&store, &publisher, 10).await.unwrap();

        assert_eq!(outcome.published, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            publisher.delivered.lock().unwrap().as_slice(),
            &[event.id],
            "the committed event must be delivered by the drain"
        );
        assert!(
            store.published_ids().contains(&event.id),
            "drained event must be stamped published"
        );
        assert!(store.unpublished().is_empty());
    }

    #[tokio::test]
    async fn failed_ack_leaves_event_unpublished_for_the_next_drain() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let publisher = RecordingPublisher {
            fail_ids: HashSet::from([event.id]),
            ..RecordingPublisher::default()
        };

        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 1);
        assert!(
            store.unpublished().contains(&event.id),
            "an unacknowledged event must remain unpublished"
        );
        assert_eq!(store.failure_count(), 1);

        // The next drain (with a healthy broker) publishes it.
        let healthy = RecordingPublisher::default();
        let outcome = drain_once(&store, &healthy, 10).await.unwrap();
        assert_eq!(outcome.published, 1);
        assert!(store.published_ids().contains(&event.id));
    }

    #[tokio::test]
    async fn invalid_payload_is_recorded_as_failure_not_published() {
        let id = Uuid::new_v4();
        let store =
            MemoryOutbox::with_events(vec![outbox_row(id, serde_json::json!({"nope": true}))]);
        let publisher = RecordingPublisher::default();

        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 1);
        assert!(publisher.delivered.lock().unwrap().is_empty());
        assert!(store.unpublished().contains(&id));
    }

    #[tokio::test]
    async fn unavailable_transport_never_marks_events_published() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let publisher = NatsAlertEventPublisher::new(NatsPublisher::disabled(), None);

        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 1);
        assert!(
            store.unpublished().contains(&event.id),
            "a disabled transport must not consume the event"
        );
    }

    #[tokio::test]
    async fn exhausted_event_is_not_reclaimed() {
        let event = sample_event(Uuid::new_v4());
        let mut row = outbox_row(event.id, serde_json::to_value(&event).unwrap());
        row.attempts = MAX_OUTBOX_ATTEMPTS;
        let store = MemoryOutbox::with_events(vec![row]);
        let publisher = RecordingPublisher::default();

        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 0);
        assert!(publisher.delivered.lock().unwrap().is_empty());
        assert!(store.unpublished().contains(&event.id));
    }

    #[tokio::test]
    async fn second_publisher_skips_a_claimed_event() {
        // Mirrors `FOR UPDATE SKIP LOCKED`: while one publisher holds the row,
        // the fast path must not publish it again.
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let drain = store.lock_batch(10).await.unwrap();
        assert!(!drain.is_empty());

        let publisher = RecordingPublisher::default();
        let delivered = deliver_outbox_event(&store, &publisher, event.id)
            .await
            .unwrap();
        assert!(
            !delivered,
            "a row held by another publisher must be skipped, not published twice"
        );
        assert!(publisher.delivered.lock().unwrap().is_empty());

        // Once the drain commits, the event is published by it.
        let (outcome, _) = publish_locked_batch(drain, &publisher).await.unwrap();
        assert_eq!(outcome.published, 1);
    }

    #[tokio::test]
    async fn fast_path_delivers_an_unclaimed_event_once() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let publisher = RecordingPublisher::default();

        let delivered = deliver_outbox_event(&store, &publisher, event.id)
            .await
            .unwrap();
        assert!(delivered);
        assert!(store.published_ids().contains(&event.id));

        // A second attempt finds nothing to claim.
        let delivered = deliver_outbox_event(&store, &publisher, event.id)
            .await
            .unwrap();
        assert!(!delivered);
        assert_eq!(publisher.delivered.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fast_path_failure_is_recorded_for_retry() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let failing = RecordingPublisher {
            fail_ids: HashSet::from([event.id]),
            ..RecordingPublisher::default()
        };

        let result = deliver_outbox_event(&store, &failing, event.id).await;
        assert!(result.is_err(), "a failed ACK must surface as an error");
        assert!(
            store.unpublished().contains(&event.id),
            "the event must stay queued after a failed fast-path publish"
        );
        assert_eq!(store.failure_count(), 1);
    }

    /// Transport mock: fails the first publish (the rule alert) and counts the
    /// successful ones, so the rule-cooldown release is observable.
    #[derive(Default)]
    struct FailFirstTransport {
        calls: Arc<AtomicUsize>,
        succeeded: Arc<AtomicUsize>,
    }

    struct RecordingAck {
        succeeded: Arc<AtomicUsize>,
        will_fail: bool,
    }

    impl PendingPublishAck for RecordingAck {
        fn wait_for_ack(
            self: Box<Self>,
        ) -> futures::future::BoxFuture<'static, anyhow::Result<()>> {
            Box::pin(async move {
                if self.will_fail {
                    anyhow::bail!("simulated rule-alert ACK failure");
                }
                self.succeeded.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[async_trait]
    impl JetStreamTransport for FailFirstTransport {
        async fn publish(
            &self,
            _subject: String,
            _payload: Vec<u8>,
        ) -> anyhow::Result<Box<dyn PendingPublishAck>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(RecordingAck {
                succeeded: Arc::clone(&self.succeeded),
                will_fail: call == 0,
            }))
        }
    }

    struct FlakyTransportCounters {
        calls: Arc<AtomicUsize>,
        succeeded: Arc<AtomicUsize>,
    }

    fn flaky_rule_publisher(
        evaluator: Arc<AlertEvaluator>,
    ) -> (NatsAlertEventPublisher, FlakyTransportCounters) {
        let transport = FailFirstTransport::default();
        let counters = FlakyTransportCounters {
            calls: Arc::clone(&transport.calls),
            succeeded: Arc::clone(&transport.succeeded),
        };
        let nats = NatsPublisher::with_transport(Arc::new(transport), false);
        (
            NatsAlertEventPublisher::new(nats, Some(evaluator)),
            counters,
        )
    }

    #[tokio::test]
    async fn rule_alert_failure_releases_cooldown_and_redelivers_on_retry() {
        let evaluator = Arc::new(
            AlertEvaluator::load_from_yaml(
                r#"
version: 1
rules:
  - name: any-warning
    source: warning
    condition: "true"
    severity: info
    for: 1h
    notify: []
"#,
            )
            .unwrap(),
        );
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let (publisher, transport) = flaky_rule_publisher(evaluator);

        // First drain: the rule alert fails, the cooldown is released, and the
        // row stays unpublished (the base alert is never reached).
        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(outcome.failed, 1);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

        // Retry: the rule alert must be attempted again despite its 1h cooldown.
        let outcome = drain_once(&store, &publisher, 10).await.unwrap();
        assert_eq!(
            outcome.published, 1,
            "the retry must publish the rule alert and the base alert"
        );
        assert_eq!(
            transport.succeeded.load(Ordering::SeqCst),
            2,
            "the retry must ACK the rule alert and the base alert"
        );
        assert!(store.published_ids().contains(&event.id));
    }

    #[tokio::test]
    async fn rule_evaluation_publishes_rule_alerts_and_the_base_alert() {
        let evaluator = Arc::new(
            AlertEvaluator::load_from_yaml(
                r#"
version: 1
rules:
  - name: any-warning
    source: warning
    condition: "true"
    severity: info
    for: 0s
    notify: []
"#,
            )
            .unwrap(),
        );
        let event = sample_event(Uuid::new_v4());
        let (publisher, transport) = flaky_rule_publisher(evaluator);
        // Make the transport healthy: the first publish must succeed here.
        transport.calls.store(1, Ordering::SeqCst);

        publisher.publish_event(&event).await.unwrap();
        assert_eq!(transport.succeeded.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn domain_event_maps_warning_type_for_rule_sources() {
        let event = sample_event(Uuid::new_v4());
        let domain = NatsAlertEventPublisher::domain_event_for(&event);
        assert_eq!(domain.source, "warning_volume_anomaly");
        assert_eq!(domain.severity, "high");
        assert_eq!(domain.id, event.id);
        assert_eq!(domain.entity_id, event.entity_id);
    }

    #[test]
    fn domain_event_tolerates_missing_warning_type() {
        let mut event = sample_event(Uuid::new_v4());
        event.metadata = serde_json::json!({});
        let domain = NatsAlertEventPublisher::domain_event_for(&event);
        assert_eq!(domain.source, "warning_unknown");
    }
}
