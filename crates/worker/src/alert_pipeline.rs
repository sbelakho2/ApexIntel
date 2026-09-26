//! Transactional outbox publisher — the ONE alert publication path.
//!
//! ```text
//! warning INSERT + outbox event (one transaction)
//!     -> semantic triage
//!     -> publisher CLAIMS the outbox row (TX1: FOR UPDATE SKIP LOCKED + lease)
//!     -> AlertEvaluator rules
//!     -> publish with Nats-Msg-Id = outbox row id (OUTSIDE any transaction)
//!     -> await JetStream ACK
//!     -> TX2: published_at  (or attempts/last_error, or dead_lettered)
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
//!   unpublished (crash, ACK failure, NATS outage, expired lease).
//!
//! Both claim rows with `FOR UPDATE SKIP LOCKED` **and commit before
//! publishing**, so no database lock is held across the broker round trip and a
//! second publisher skips leased rows instead of double-publishing.
//!
//! No other component publishes warning alerts to NATS: direct
//! `NatsPublisher::publish_alert` calls are confined to the alert transport
//! module and enforced by `scripts/ci/check_alert_publish.sh`.
//!
//! # Delivery guarantees — at-least-once, not exactly-once
//! - A crash between the warning commit and the publish leaves the outbox row
//!   unpublished; the next drain retries it (never silently lost).
//! - The stable outbox row id is sent as the JetStream `Nats-Msg-Id` header, so
//!   a republish caused by a crash after the broker accepted (but before TX2
//!   recorded) the event is deduplicated inside the stream's duplicate window.
//! - Consumers must still be idempotent: exactly-once delivery is explicitly
//!   not claimed.
//! - Events that exhaust
//!   [`apex_store::postgres::MAX_OUTBOX_ATTEMPTS`] move to the explicit
//!   dead-letter state, raise an operator alert, and stop being claimed until an
//!   operator replays them (admin UI / [`PgStore::replay_dead_lettered_outbox`]).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use tracing::{error, info, warn};
use uuid::Uuid;

use apex_store::postgres::{
    EventOutboxRow, OutboxBacklog, OutboxClaim, OutboxDrainOutcome, PgStore,
    DEFAULT_OUTBOX_LEASE_SECS,
};
use apex_worker::nats_stream::{AlertEvent, NatsPublisher};

use crate::alert_evaluator::{AlertEvaluator, DomainEvent};
use crate::alert_transport::AlertTransport;

/// Default path to the alert rules file.
const DEFAULT_ALERT_RULES_PATH: &str = "config/runtime/alert-rules.yaml";

/// How often the drain task looks for unpublished events.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum events claimed per drain batch.
pub const DEFAULT_BATCH_SIZE: i64 = 25;

/// Upper bound for one publish + ACK round trip. Bounds how long one event's
/// lease must cover when the broker accepts messages but never acknowledges
/// them.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(15);

/// Resolve the alert rules path from the environment (`ALERT_RULES_PATH`).
fn alert_rules_path_from_env() -> String {
    std::env::var("ALERT_RULES_PATH").unwrap_or_else(|_| DEFAULT_ALERT_RULES_PATH.to_string())
}

/// Claim owner identity: instance id (or hostname) plus process id, so two
/// workers on one host never share a lease identity.
pub fn claim_owner() -> String {
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
    format!("{instance}:{}", std::process::id())
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

/// Storage half of the publisher: claim (TX1), settle (TX2), and the operator
/// dead-letter alert.
#[async_trait]
pub trait OutboxClaimStore: Send + Sync {
    /// TX1: claim up to `limit` publishable rows with a lease and commit.
    async fn claim_batch(&self, owner: &str, limit: i64) -> anyhow::Result<OutboxClaim>;

    /// TX1: claim one specific publishable row with a lease and commit.
    async fn claim_event(&self, owner: &str, id: Uuid) -> anyhow::Result<OutboxClaim>;

    /// TX2a: stamp `published_at`; `false` when the lease was lost.
    async fn mark_published(&self, owner: &str, id: Uuid) -> anyhow::Result<bool>;

    /// TX2b: record the failed attempt; `true` when the row is now dead-lettered.
    async fn record_failure(&self, owner: &str, id: Uuid, error: &str) -> anyhow::Result<bool>;

    /// Clear leases for rows that could not be settled (publish task aborted).
    async fn release_claims(&self, owner: &str, ids: &[Uuid]) -> anyhow::Result<u64>;

    /// Operator alert raised when a row moves to the dead-letter state.
    async fn alert_dead_lettered(&self, row: &EventOutboxRow, reason: &str) -> anyhow::Result<()>;
}

/// Publication half of the publisher. Implementations must await the JetStream
/// ACK before returning `Ok`.
#[async_trait]
pub trait OutboxEventPublisher: Send + Sync {
    /// Publish `event` and await the broker ACK.
    ///
    /// `msg_id` is the stable transport identity (`Nats-Msg-Id`).
    async fn publish_event(&self, event: &AlertEvent, msg_id: &str) -> anyhow::Result<()>;
}

#[async_trait]
impl OutboxClaimStore for PgStore {
    async fn claim_batch(&self, owner: &str, limit: i64) -> anyhow::Result<OutboxClaim> {
        Ok(self
            .claim_unpublished_outbox(owner, DEFAULT_OUTBOX_LEASE_SECS, limit)
            .await?)
    }

    async fn claim_event(&self, owner: &str, id: Uuid) -> anyhow::Result<OutboxClaim> {
        Ok(self
            .claim_outbox_event(owner, DEFAULT_OUTBOX_LEASE_SECS, id)
            .await?)
    }

    async fn mark_published(&self, owner: &str, id: Uuid) -> anyhow::Result<bool> {
        Ok(self.mark_outbox_published(id, owner).await?)
    }

    async fn record_failure(&self, owner: &str, id: Uuid, error: &str) -> anyhow::Result<bool> {
        Ok(self.record_outbox_failure(id, owner, error).await?)
    }

    async fn release_claims(&self, owner: &str, ids: &[Uuid]) -> anyhow::Result<u64> {
        Ok(self.release_outbox_claims(owner, ids).await?)
    }

    async fn alert_dead_lettered(&self, row: &EventOutboxRow, reason: &str) -> anyhow::Result<()> {
        Ok(self.insert_dead_letter_operator_alert(row, reason).await?)
    }
}

/// Production publisher: evaluates alert rules, then publishes the alert event
/// through the alert transport and awaits the broker ACK.
pub struct NatsAlertEventPublisher {
    transport: AlertTransport,
    evaluator: Option<Arc<AlertEvaluator>>,
}

impl NatsAlertEventPublisher {
    pub fn new(publisher: NatsPublisher, evaluator: Option<Arc<AlertEvaluator>>) -> Self {
        Self {
            transport: AlertTransport::new(publisher),
            evaluator,
        }
    }

    /// Build around an already-wrapped transport (tests / shared connection).
    pub fn with_transport(
        transport: AlertTransport,
        evaluator: Option<Arc<AlertEvaluator>>,
    ) -> Self {
        Self {
            transport,
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
    async fn publish_event(&self, event: &AlertEvent, msg_id: &str) -> anyhow::Result<()> {
        // An unavailable transport must fail the row, never let it be stamped
        // published: the drain retries it instead of dropping the alert.
        if !self.transport.is_connected() {
            anyhow::bail!(
                "NATS transport unavailable; leaving outbox event {} unpublished for retry",
                event.id
            );
        }

        if let Some(evaluator) = &self.evaluator {
            let domain = Self::domain_event_for(event);
            for rule_alert in evaluator.evaluate(&domain).await {
                // Rule-derived alerts get their own stable id derived from the
                // outbox row, so a retry of the same row deduplicates too.
                let rule_msg_id = format!("{msg_id}:rule:{}", rule_alert.id);
                if let Err(error) = self
                    .transport
                    .publish_event(&rule_alert, &rule_msg_id)
                    .await
                {
                    // The evaluator already recorded the rule's cooldown before
                    // the publish; release it so the outbox retry redelivers the
                    // rule alert instead of silently suppressing it.
                    evaluator.forget_firings(&domain).await;
                    return Err(error).context("failed to publish rule-derived alert");
                }
            }
        }

        self.transport.publish_event(event, msg_id).await
    }
}

/// Publish every claimed event, settling each with a short TX2.
///
/// The publish (and its timeout) runs OUTSIDE any transaction: rows were
/// claimed and the claim transaction committed before this function starts. A
/// publish failure records the attempt and dead-letters exhausted rows; it
/// never aborts the rest of the batch.
async fn publish_claimed_events(
    store: &dyn OutboxClaimStore,
    publisher: &dyn OutboxEventPublisher,
    claim: OutboxClaim,
) -> anyhow::Result<(OutboxDrainOutcome, Option<String>)> {
    let (owner, rows) = claim.into_parts();
    let mut outcome = OutboxDrainOutcome::default();
    let mut last_error: Option<String> = None;
    let mut unsettled: Vec<Uuid> = Vec::new();

    for row in rows {
        let result = match serde_json::from_value::<AlertEvent>(row.payload.clone()) {
            Ok(event) => {
                let msg_id = row.id.to_string();
                let attempted =
                    tokio::time::timeout(PUBLISH_TIMEOUT, publisher.publish_event(&event, &msg_id))
                        .await;
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
            Ok(()) => match store.mark_published(&owner, row.id).await {
                Ok(_) => outcome.published += 1,
                Err(error) => {
                    // The broker accepted but we could not record it; the lease
                    // expires and the row is republished (at-least-once).
                    warn!(
                        outbox_id = %row.id,
                        error = %error,
                        "outbox publisher: published but failed to record published_at; \
                         the lease expires and the event is redelivered"
                    );
                    unsettled.push(row.id);
                }
            },
            Err(error) => {
                let reason = error.to_string();
                match store.record_failure(&owner, row.id, &reason).await {
                    Ok(true) => {
                        outcome.dead_lettered += 1;
                        crate::observability::WORKER_METRICS.record_outbox_dead_lettered();
                        error!(
                            outbox_id = %row.id,
                            attempts = row.attempts,
                            error = %reason,
                            "outbox publisher: event dead-lettered after exhausting attempts; \
                             operator replay required"
                        );
                        if let Err(alert_error) = store.alert_dead_lettered(&row, &reason).await {
                            warn!(
                                outbox_id = %row.id,
                                error = %alert_error,
                                "outbox publisher: failed to record dead-letter operator alert"
                            );
                        }
                    }
                    Ok(false) => {
                        outcome.failed += 1;
                        warn!(
                            outbox_id = %row.id,
                            attempts = row.attempts,
                            error = %reason,
                            "outbox publisher: publish failed; event remains unpublished for retry"
                        );
                    }
                    Err(record_error) => {
                        warn!(
                            outbox_id = %row.id,
                            error = %record_error,
                            "outbox publisher: failed to record publish failure; \
                             the lease expires and the event is retried"
                        );
                        unsettled.push(row.id);
                    }
                }
                last_error = Some(reason);
            }
        }
    }

    if !unsettled.is_empty() {
        let _ = store.release_claims(&owner, &unsettled).await;
    }

    Ok((outcome, last_error))
}

/// Drain one outbox batch: claim (TX1) then publish outside any transaction.
pub async fn drain_once(
    store: &dyn OutboxClaimStore,
    publisher: &dyn OutboxEventPublisher,
    owner: &str,
    limit: i64,
) -> anyhow::Result<OutboxDrainOutcome> {
    let claim = store.claim_batch(owner, limit).await?;
    let (outcome, _) = publish_claimed_events(store, publisher, claim).await?;
    Ok(outcome)
}

/// Deliver one specific outbox event (the ingress fast path).
///
/// * `Ok(true)` — this call held the lease, published the event and awaited its
///   ACK, then stamped `published_at`.
/// * `Ok(false)` — another publisher holds the lease, it is already published,
///   or it is dead-lettered; delivery is left to the drain.
/// * `Err` — this call held the lease, the publish failed, and the failure was
///   recorded for retry.
pub async fn deliver_outbox_event(
    store: &dyn OutboxClaimStore,
    publisher: &dyn OutboxEventPublisher,
    owner: &str,
    id: Uuid,
) -> anyhow::Result<bool> {
    let claim = store.claim_event(owner, id).await?;
    if claim.is_empty() {
        return Ok(false);
    }
    let (outcome, last_error) = publish_claimed_events(store, publisher, claim).await?;
    if outcome.published > 0 {
        return Ok(true);
    }
    Err(anyhow::anyhow!(
        "{}",
        last_error.unwrap_or_else(|| format!("outbox event {id} was not published"))
    ))
}

/// Unpublished/dead-letter backlog for metrics and readiness checks.
pub async fn outbox_backlog(
    store: &PgStore,
    stuck_before: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<OutboxBacklog> {
    store.outbox_backlog(stuck_before).await
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
        let owner = claim_owner();
        let mut interval = tokio::time::interval(DEFAULT_POLL_INTERVAL);

        info!(owner = %owner, "outbox alert publisher started (single canonical alert path)");
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
            match drain_once(store.as_ref(), &drain_publisher, &owner, DEFAULT_BATCH_SIZE).await {
                Ok(outcome) if outcome.published > 0 || outcome.failed > 0 => {
                    info!(
                        published = outcome.published,
                        failed = outcome.failed,
                        dead_lettered = outcome.dead_lettered,
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
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use apex_store::postgres::MAX_OUTBOX_ATTEMPTS;
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
            lease_owner: None,
            lease_until: None,
            dead_lettered_at: None,
            dead_letter_reason: None,
        }
    }

    /// Memory claim/lease store mirroring the SQL semantics:
    /// `FOR UPDATE SKIP LOCKED` + lease stamp + attempts increment in TX1,
    /// short settlements in TX2.
    #[derive(Default)]
    struct MemoryOutbox {
        rows: Arc<Mutex<Vec<EventOutboxRow>>>,
        leases: Arc<Mutex<HashMap<Uuid, String>>>,
        published: Arc<Mutex<HashSet<Uuid>>>,
        dead_letters: Arc<Mutex<Vec<(Uuid, String)>>>,
        operator_alerts: Arc<Mutex<Vec<Uuid>>>,
        /// Number of simulated claim transactions currently open; the publisher
        /// asserts this is zero while publishing.
        open_claim_txs: Arc<AtomicUsize>,
    }

    impl MemoryOutbox {
        fn with_events(events: Vec<EventOutboxRow>) -> Self {
            Self {
                rows: Arc::new(Mutex::new(events)),
                leases: Arc::new(Mutex::new(HashMap::new())),
                published: Arc::new(Mutex::new(HashSet::new())),
                dead_letters: Arc::new(Mutex::new(Vec::new())),
                operator_alerts: Arc::new(Mutex::new(Vec::new())),
                open_claim_txs: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn unpublished(&self) -> Vec<Uuid> {
            let published = self.published.lock().unwrap();
            self.rows
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

        fn dead_letter_ids(&self) -> Vec<Uuid> {
            self.dead_letters
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect()
        }

        fn operator_alert_ids(&self) -> Vec<Uuid> {
            self.operator_alerts.lock().unwrap().clone()
        }

        fn attempts(&self, id: Uuid) -> i32 {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.id == id)
                .map(|row| row.attempts)
                .unwrap_or(0)
        }

        fn leased_ids(&self) -> Vec<Uuid> {
            self.leases.lock().unwrap().keys().copied().collect()
        }

        fn claim_rows(&self, owner: &str, only: Option<Uuid>, limit: Option<usize>) -> OutboxClaim {
            // TX1 opens, locks and stamps the lease, then commits before any
            // publish can observe the claim.
            self.open_claim_txs.fetch_add(1, Ordering::SeqCst);
            let published = self.published.lock().unwrap().clone();
            let mut rows = self.rows.lock().unwrap();
            let leases = self.leases.lock().unwrap().clone();
            let mut claimed = Vec::new();
            for row in rows.iter_mut() {
                if (Some(row.id) == only || only.is_none())
                    && !published.contains(&row.id)
                    && row.dead_lettered_at.is_none()
                    && row.attempts < MAX_OUTBOX_ATTEMPTS
                    && !leases.contains_key(&row.id)
                {
                    row.attempts += 1;
                    row.lease_owner = Some(owner.to_string());
                    claimed.push(row.clone());
                    if let Some(limit) = limit {
                        if claimed.len() >= limit {
                            break;
                        }
                    }
                }
            }
            drop(rows);
            drop(leases);
            {
                let mut leases = self.leases.lock().unwrap();
                for row in &claimed {
                    leases.insert(row.id, owner.to_string());
                }
            }
            self.open_claim_txs.fetch_sub(1, Ordering::SeqCst);
            OutboxClaim {
                owner: owner.to_string(),
                rows: claimed,
            }
        }
    }

    #[async_trait]
    impl OutboxClaimStore for MemoryOutbox {
        async fn claim_batch(&self, owner: &str, limit: i64) -> anyhow::Result<OutboxClaim> {
            Ok(self.claim_rows(owner, None, Some(limit.max(1) as usize)))
        }

        async fn claim_event(&self, owner: &str, id: Uuid) -> anyhow::Result<OutboxClaim> {
            Ok(self.claim_rows(owner, Some(id), None))
        }

        async fn mark_published(&self, owner: &str, id: Uuid) -> anyhow::Result<bool> {
            let mut leases = self.leases.lock().unwrap();
            if leases.get(&id).map(String::as_str) != Some(owner) {
                return Ok(false);
            }
            leases.remove(&id);
            drop(leases);
            self.published.lock().unwrap().insert(id);
            Ok(true)
        }

        async fn record_failure(&self, owner: &str, id: Uuid, error: &str) -> anyhow::Result<bool> {
            let mut leases = self.leases.lock().unwrap();
            if leases.get(&id).map(String::as_str) != Some(owner) {
                return Ok(false);
            }
            leases.remove(&id);
            drop(leases);
            let mut rows = self.rows.lock().unwrap();
            let row = rows.iter_mut().find(|row| row.id == id);
            let dead = match row {
                Some(row) => {
                    row.last_error = Some(error.to_string());
                    if row.attempts >= MAX_OUTBOX_ATTEMPTS {
                        row.dead_lettered_at = Some(chrono::Utc::now());
                        row.dead_letter_reason = Some(error.to_string());
                        true
                    } else {
                        false
                    }
                }
                None => false,
            };
            drop(rows);
            if dead {
                self.dead_letters
                    .lock()
                    .unwrap()
                    .push((id, error.to_string()));
            }
            Ok(dead)
        }

        async fn release_claims(&self, owner: &str, ids: &[Uuid]) -> anyhow::Result<u64> {
            let mut leases = self.leases.lock().unwrap();
            let before = leases.len();
            leases.retain(|id, lease_owner| !(lease_owner == owner && ids.contains(id)));
            Ok((before - leases.len()) as u64)
        }

        async fn alert_dead_lettered(
            &self,
            row: &EventOutboxRow,
            _reason: &str,
        ) -> anyhow::Result<()> {
            self.operator_alerts.lock().unwrap().push(row.id);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingPublisher {
        delivered: Mutex<Vec<(Uuid, String)>>,
        fail_ids: HashSet<Uuid>,
        /// Set when a publish observes an open claim transaction.
        observed_open_claim_tx: Mutex<bool>,
        open_claim_txs: Option<Arc<AtomicUsize>>,
    }

    impl RecordingPublisher {
        fn watching(open_claim_txs: Arc<AtomicUsize>) -> Self {
            Self {
                open_claim_txs: Some(open_claim_txs),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl OutboxEventPublisher for RecordingPublisher {
        async fn publish_event(&self, event: &AlertEvent, msg_id: &str) -> anyhow::Result<()> {
            if let Some(counter) = &self.open_claim_txs {
                if counter.load(Ordering::SeqCst) != 0 {
                    *self.observed_open_claim_tx.lock().unwrap() = true;
                }
            }
            if self.fail_ids.contains(&event.id) {
                anyhow::bail!("simulated JetStream ACK failure for {}", event.id);
            }
            self.delivered
                .lock()
                .unwrap()
                .push((event.id, msg_id.to_string()));
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
        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();

        assert_eq!(outcome.published, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            publisher.delivered.lock().unwrap().as_slice(),
            &[(event.id, event.id.to_string())],
            "the committed event must be delivered with its outbox id as msg id"
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

        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 1);
        assert_eq!(outcome.dead_lettered, 0);
        assert!(
            store.unpublished().contains(&event.id),
            "an unacknowledged event must remain unpublished"
        );
        assert_eq!(store.attempts(event.id), 1, "the attempt is persisted");

        // The next drain (with a healthy broker) publishes it.
        let healthy = RecordingPublisher::default();
        let outcome = drain_once(&store, &healthy, "drain-2", 10).await.unwrap();
        assert_eq!(outcome.published, 1);
        assert!(store.published_ids().contains(&event.id));
    }

    #[tokio::test]
    async fn invalid_payload_is_recorded_as_failure_not_published() {
        let id = Uuid::new_v4();
        let store =
            MemoryOutbox::with_events(vec![outbox_row(id, serde_json::json!({"nope": true}))]);
        let publisher = RecordingPublisher::default();

        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
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

        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
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

        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 0);
        assert!(publisher.delivered.lock().unwrap().is_empty());
        assert!(store.unpublished().contains(&event.id));
    }

    #[tokio::test]
    async fn exhaustion_dead_letters_the_row_and_raises_an_operator_alert() {
        let event = sample_event(Uuid::new_v4());
        let mut row = outbox_row(event.id, serde_json::to_value(&event).unwrap());
        // The next attempt (claim increments before publishing) exhausts the
        // budget; the failure must dead-letter the row, not silently retry
        // forever.
        row.attempts = MAX_OUTBOX_ATTEMPTS - 1;
        let store = MemoryOutbox::with_events(vec![row]);
        let publisher = RecordingPublisher {
            fail_ids: HashSet::from([event.id]),
            ..RecordingPublisher::default()
        };

        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
        assert_eq!(outcome.dead_lettered, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(store.dead_letter_ids(), vec![event.id]);
        assert_eq!(
            store.operator_alert_ids(),
            vec![event.id],
            "dead-lettering must raise the operator alert"
        );

        // A dead-lettered row is terminal: no further claims.
        let outcome = drain_once(&store, &publisher, "drain-2", 10).await.unwrap();
        assert_eq!(outcome.published, 0);
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.dead_lettered, 0);
    }

    #[tokio::test]
    async fn claim_lease_prevents_double_publish_and_holds_no_lock_during_publish() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let publisher = RecordingPublisher::watching(Arc::clone(&store.open_claim_txs));

        // First drain claims the row, publishes outside the claim transaction,
        // and settles it.
        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
        assert_eq!(outcome.published, 1);
        assert!(
            !*publisher.observed_open_claim_tx.lock().unwrap(),
            "publishing must not run inside the claim transaction"
        );
        assert!(
            store.leased_ids().is_empty(),
            "a settled row must release its lease"
        );

        // While one publisher holds the lease, another claims nothing.
        let event2 = sample_event(Uuid::new_v4());
        store.rows.lock().unwrap().push(outbox_row(
            event2.id,
            serde_json::to_value(&event2).unwrap(),
        ));
        let held = store.claim_batch("drain-2", 10).await.unwrap();
        assert_eq!(held.rows.len(), 1);
        let skipped = store.claim_batch("drain-3", 10).await.unwrap();
        assert!(
            skipped.is_empty(),
            "a leased row must not be claimed by another publisher"
        );

        // Settlement releases the lease and makes the row published exactly
        // once for the holder.
        assert_eq!(
            publisher.delivered.lock().unwrap().len(),
            1,
            "the second, skipped publisher must not have published anything"
        );
        store.mark_published(&held.owner, event2.id).await.unwrap();
        assert_eq!(store.published_ids().len(), 2);
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
        let drain = store.claim_batch("drain-1", 10).await.unwrap();
        assert!(!drain.is_empty());

        let publisher = RecordingPublisher::default();
        let delivered = deliver_outbox_event(&store, &publisher, "fast-path", event.id)
            .await
            .unwrap();
        assert!(
            !delivered,
            "a row held by another publisher must be skipped, not published twice"
        );
        assert!(publisher.delivered.lock().unwrap().is_empty());

        // Once the drain settles, the event is published by it.
        store.mark_published(&drain.owner, event.id).await.unwrap();
        assert!(store.published_ids().contains(&event.id));
    }

    #[tokio::test]
    async fn fast_path_delivers_an_unclaimed_event_once() {
        let event = sample_event(Uuid::new_v4());
        let store = MemoryOutbox::with_events(vec![outbox_row(
            event.id,
            serde_json::to_value(&event).unwrap(),
        )]);
        let publisher = RecordingPublisher::default();

        let delivered = deliver_outbox_event(&store, &publisher, "fast-path", event.id)
            .await
            .unwrap();
        assert!(delivered);
        assert!(store.published_ids().contains(&event.id));

        // A second attempt finds nothing to claim.
        let delivered = deliver_outbox_event(&store, &publisher, "fast-path", event.id)
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

        let result = deliver_outbox_event(&store, &failing, "fast-path", event.id).await;
        assert!(result.is_err(), "a failed ACK must surface as an error");
        assert!(
            store.unpublished().contains(&event.id),
            "the event must stay queued after a failed fast-path publish"
        );
        assert_eq!(store.attempts(event.id), 1);
    }

    /// Transport mock: fails the first publish (the rule alert) and counts the
    /// successful ones, so the rule-cooldown release is observable.
    #[derive(Default)]
    struct FailFirstTransport {
        calls: Arc<AtomicUsize>,
        succeeded: Arc<AtomicUsize>,
        msg_ids: Arc<Mutex<Vec<Option<String>>>>,
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
            msg_id: Option<String>,
            _payload: Vec<u8>,
        ) -> anyhow::Result<Box<dyn PendingPublishAck>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.msg_ids.lock().unwrap().push(msg_id);
            Ok(Box::new(RecordingAck {
                succeeded: Arc::clone(&self.succeeded),
                will_fail: call == 0,
            }))
        }
    }

    struct FlakyTransportCounters {
        calls: Arc<AtomicUsize>,
        succeeded: Arc<AtomicUsize>,
        msg_ids: Arc<Mutex<Vec<Option<String>>>>,
    }

    fn flaky_rule_publisher(
        evaluator: Arc<AlertEvaluator>,
    ) -> (NatsAlertEventPublisher, FlakyTransportCounters) {
        let transport = FailFirstTransport::default();
        let counters = FlakyTransportCounters {
            calls: Arc::clone(&transport.calls),
            succeeded: Arc::clone(&transport.succeeded),
            msg_ids: Arc::clone(&transport.msg_ids),
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
        let outcome = drain_once(&store, &publisher, "drain-1", 10).await.unwrap();
        assert_eq!(outcome.failed, 1);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

        // Retry: the rule alert must be attempted again despite its 1h cooldown.
        let outcome = drain_once(&store, &publisher, "drain-2", 10).await.unwrap();
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

        // Rule-derived and base publishes carry stable, distinct msg ids derived
        // from the outbox row id.
        let ids = transport.msg_ids.lock().unwrap().clone();
        assert_eq!(ids.len(), 3);
        let outbox_id = event.id.to_string();
        assert!(ids[0].as_deref().unwrap().contains(outbox_id.as_str()));
        assert!(ids[0].as_deref().unwrap().contains(":rule:"));
        assert_eq!(ids[2].as_deref(), Some(event.id.to_string().as_str()));
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

        publisher
            .publish_event(&event, &event.id.to_string())
            .await
            .unwrap();
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

    #[test]
    fn claim_owner_includes_process_identity() {
        let owner = claim_owner();
        assert!(owner.ends_with(&format!(":{}", std::process::id())));
    }
}
