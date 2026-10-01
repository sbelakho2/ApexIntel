//! Durable per-channel notification delivery state (migration 069).
//!
//! The retry processor claims due rows with `FOR UPDATE SKIP LOCKED`, stamps a
//! short lease, and **commits before attempting the channel send**. The channel
//! send therefore runs outside any database transaction; a crash mid-send leaves
//! the lease to expire and the row is retried instead of being lost.
//!
//! Delivery guarantee is at-least-once: every attempt for a delivery row
//! carries the same **stable** transport idempotency key
//! (event/channel/destination/payload-hash), so a send that was accepted but
//! whose settlement was lost is deduplicated by the receiver when the row is
//! reclaimed. The attempt number travels separately (header), never inside the
//! key.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

/// One per-channel delivery row, as claimed by the worker.
///
/// The primary identity is `delivery_key` (= event + channel + destination);
/// there is deliberately no surrogate id.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationDeliveryRow {
    pub delivery_key: String,
    pub notification_event_id: Option<Uuid>,
    pub channel: String,
    pub destination: String,
    pub payload: Value,
    pub payload_hash: Option<String>,
    pub status: String,
    pub attempts: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A channel destination (webhook URL, email recipient list, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryChannel {
    pub channel: String,
    pub destination: String,
}

/// A domain notification event to persist together with its alert outbox row
/// and per-channel delivery rows, in one transaction.
#[derive(Debug, Clone)]
pub struct NewNotificationEvent {
    /// Stable identity across scheduler runs (e.g. `sla_breach:<warning-id>`).
    pub dedupe_key: String,
    pub source_type: String,
    pub source_id: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub body: String,
    /// Domain payload (the serialized `PendingAlert`). Stored as-is on
    /// `notification_events` and wrapped as `{ "alert": ... }` for the
    /// per-channel transport payload.
    pub payload: Value,
    /// Outbox row identity for the real-time alert stream.
    pub outbox_aggregate_type: String,
    pub outbox_aggregate_id: Uuid,
    pub outbox_event_type: String,
    /// Serialized `AlertEvent` published by the canonical outbox drain.
    pub outbox_payload: Value,
}

/// Result of enqueuing a domain notification event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEnqueueOutcome {
    pub notification_event_id: Uuid,
    pub outbox_id: Option<Uuid>,
    pub deliveries_enqueued: usize,
    /// The dedupe key already existed; nothing new was written.
    pub already_enqueued: bool,
}

/// Delivery backlog snapshot for metrics and readiness checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotificationBacklog {
    /// Rows waiting to be attempted (pending or failed, not dead-lettered).
    pub pending: i64,
    /// Waiting rows whose `next_retry_at` is already due.
    pub overdue: i64,
    /// Terminal failures awaiting operator replay.
    pub dead_lettered: i64,
}

/// Full delivery health snapshot for the readiness capability probe.
///
/// Backlog alone cannot tell a healthy pipeline from a stalled one: a stuck
/// `delivering` lease, a dead-letter spike, or a processor that stopped running
/// leave the backlog counts looking harmless. This snapshot adds the
/// retry-processor window statistics the `notification_delivery` capability
/// probe evaluates (`NotificationDeliveryHealth` is policy-checked in the API).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct NotificationDeliveryHealth {
    /// Rows waiting to be attempted (pending or failed, not dead-lettered).
    pub pending: i64,
    /// Waiting rows whose `next_retry_at` is already due.
    pub overdue: i64,
    /// Age in seconds of the oldest overdue row (`None` when nothing is due).
    pub oldest_overdue_age_secs: Option<i64>,
    /// Terminal failures awaiting operator replay.
    pub dead_lettered: i64,
    /// Rows dead-lettered inside the recent window (change rate).
    pub dead_lettered_recent: i64,
    /// Claimed rows whose lease already expired: a crash (or a settlement
    /// write failure) left them `delivering` past their lease.
    pub stuck_delivering: i64,
    /// Attempts recorded inside the recent window.
    pub attempts_recent: i64,
    /// Recent-window attempts that settled as delivered.
    pub delivered_recent: i64,
    /// Recent-window attempts that settled as dead-lettered (terminal
    /// failures). Transient `failed` attempts are excluded: retries with
    /// backoff are normal at-least-once behaviour, not delivery failure.
    pub failed_recent: i64,
}

/// Dead-lettered delivery row (admin listing).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeadLetterNotificationRow {
    pub delivery_key: String,
    pub channel: String,
    pub destination: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// SHA-256 of the notification payload; embedded in the per-attempt transport
/// idempotency key so identical payloads are recognisable across retries.
pub fn notification_payload_hash(payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    hex::encode(Sha256::digest(bytes))
}

/// Maximum length of a stored delivery error.
fn truncate_delivery_error(error: &str) -> String {
    const MAX: usize = 2000;
    if error.chars().count() <= MAX {
        return error.to_string();
    }
    let truncated: String = error.chars().take(MAX).collect();
    format!("{truncated}…")
}

impl PgStore {
    /// Persist a domain notification event, its alert outbox row and one
    /// `pending` delivery row per channel — all in ONE transaction.
    ///
    /// `dedupe_key` is unique: a repeated scheduler run returns
    /// `already_enqueued = true` and writes nothing.
    pub async fn enqueue_notification_event(
        &self,
        event: &NewNotificationEvent,
        channels: &[DeliveryChannel],
    ) -> Result<NotificationEnqueueOutcome> {
        let mut tx = self.pool.begin().await?;

        // Single conflict-safe insert instead of SELECT-then-INSERT: two
        // concurrent scheduler runs for the same dedupe key must not both pass
        // an existence check and then collide on the unique constraint (which
        // surfaced as a storage error rather than the documented
        // `already_enqueued` outcome).
        let inserted: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO notification_events \
                 (dedupe_key, source_type, source_id, severity, category, title, body, payload) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (dedupe_key) DO NOTHING \
             RETURNING id",
        )
        .bind(&event.dedupe_key)
        .bind(&event.source_type)
        .bind(&event.source_id)
        .bind(&event.severity)
        .bind(&event.category)
        .bind(&event.title)
        .bind(&event.body)
        .bind(&event.payload)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((notification_event_id,)) = inserted else {
            // The row already existed (this call, or a concurrent run, lost
            // the race): resolve the canonical id and write nothing else.
            let (notification_event_id,) = sqlx::query_as::<_, (Uuid,)>(
                "SELECT id FROM notification_events WHERE dedupe_key = $1",
            )
            .bind(&event.dedupe_key)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(NotificationEnqueueOutcome {
                notification_event_id,
                outbox_id: None,
                deliveries_enqueued: 0,
                already_enqueued: true,
            });
        };

        let (outbox_id,) = sqlx::query_as::<_, (Uuid,)>(
            "INSERT INTO event_outbox (aggregate_type, aggregate_id, event_type, payload) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(&event.outbox_aggregate_type)
        .bind(event.outbox_aggregate_id)
        .bind(&event.outbox_event_type)
        .bind(&event.outbox_payload)
        .fetch_one(&mut *tx)
        .await?;

        // The transport payload the retry processor deserializes (alert plus
        // optional subject/body rendered at claim time). Persisting the raw
        // alert here would make every claimed row fail to parse and
        // dead-letter, so the wrapper shape is part of the persisted contract.
        let delivery_payload = serde_json::json!({ "alert": &event.payload });
        let payload_hash = notification_payload_hash(&delivery_payload);
        let mut deliveries_enqueued = 0usize;
        for channel in channels {
            // Per-channel idempotency identity: event + channel + destination.
            let delivery_key = format!(
                "{notification_event_id}:{}:{}",
                channel.channel, channel.destination
            );
            let inserted = sqlx::query(
                "INSERT INTO notification_delivery_state \
                     (delivery_key, notification_event_id, channel, destination, payload, \
                      payload_hash, status, next_retry_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', now()) \
                 ON CONFLICT (delivery_key) DO NOTHING",
            )
            .bind(&delivery_key)
            .bind(notification_event_id)
            .bind(&channel.channel)
            .bind(&channel.destination)
            .bind(&delivery_payload)
            .bind(&payload_hash)
            .execute(&mut *tx)
            .await?;
            deliveries_enqueued += inserted.rows_affected() as usize;
        }

        tx.commit().await?;
        Ok(NotificationEnqueueOutcome {
            notification_event_id,
            outbox_id: Some(outbox_id),
            deliveries_enqueued,
            already_enqueued: false,
        })
    }

    /// Claim due `pending`/`failed` delivery rows — plus `delivering` rows
    /// whose lease already expired (a crashed attempt or lost settlement) —
    /// with a fresh lease.
    ///
    /// TX1 only: the row locks are released when the claim transaction commits,
    /// so the caller can attempt the channel send without holding any database
    /// lock. `attempts` is incremented at claim time (persist BEFORE attempt).
    pub async fn claim_due_notification_deliveries(
        &self,
        owner: &str,
        lease_secs: f64,
        limit: i64,
    ) -> Result<Vec<NotificationDeliveryRow>> {
        let limit = limit.clamp(1, 500);
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query_as::<_, NotificationDeliveryRow>(
            "WITH claimed AS ( \
                 SELECT delivery_key FROM notification_delivery_state \
                  WHERE status IN ('pending', 'failed', 'delivering') \
                    AND dead_lettered_at IS NULL \
                    AND (next_retry_at IS NULL OR next_retry_at <= now()) \
                    AND (lease_until IS NULL OR lease_until <= now()) \
                  ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC \
                  LIMIT $3 \
                  FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE notification_delivery_state d \
                SET status = 'delivering', \
                    lease_owner = $1, \
                    lease_until = now() + make_interval(secs => $2::double precision), \
                    attempts = d.attempts + 1, \
                    last_attempt_at = now(), \
                    updated_at = now() \
               FROM claimed \
              WHERE d.delivery_key = claimed.delivery_key \
             RETURNING d.delivery_key, d.notification_event_id, d.channel, d.destination, \
                       d.payload, d.payload_hash, d.status, d.attempts, d.next_retry_at, \
                       d.lease_owner, d.lease_until, d.last_error, d.dead_lettered_at, \
                       d.created_at, d.updated_at",
        )
        .bind(owner)
        .bind(lease_secs)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }

    /// Mark a claimed delivery delivered. Returns `false` when the lease was
    /// lost (another owner settled it first).
    ///
    /// The settlement and its attempt-log row commit together, so the
    /// readiness success-ratio window sees exactly the settlements that
    /// happened.
    pub async fn mark_notification_delivered(
        &self,
        delivery_key: &str,
        owner: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE notification_delivery_state \
                SET status = 'delivered', \
                    delivered_at = now(), \
                    lease_owner = NULL, \
                    lease_until = NULL, \
                    last_error = NULL, \
                    next_retry_at = NULL, \
                    updated_at = now() \
              WHERE delivery_key = $1 AND lease_owner = $2 AND status = 'delivering'",
        )
        .bind(delivery_key)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        let settled = result.rows_affected() > 0;
        if settled {
            record_delivery_attempt(&mut *tx, delivery_key, "delivered", None).await?;
        }
        tx.commit().await?;
        Ok(settled)
    }

    /// Record a retryable failure: schedule the next attempt at `next_retry_at`.
    pub async fn mark_notification_retry(
        &self,
        delivery_key: &str,
        owner: &str,
        next_retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<bool> {
        let error = truncate_delivery_error(error);
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE notification_delivery_state \
                SET status = 'failed', \
                    lease_owner = NULL, \
                    lease_until = NULL, \
                    next_retry_at = $3, \
                    last_error = $4, \
                    updated_at = now() \
              WHERE delivery_key = $1 AND lease_owner = $2 AND status = 'delivering'",
        )
        .bind(delivery_key)
        .bind(owner)
        .bind(next_retry_at)
        .bind(&error)
        .execute(&mut *tx)
        .await?;
        let settled = result.rows_affected() > 0;
        if settled {
            record_delivery_attempt(&mut *tx, delivery_key, "failed", Some(&error)).await?;
        }
        tx.commit().await?;
        Ok(settled)
    }

    /// Move a claimed delivery to the terminal dead-letter state.
    pub async fn mark_notification_dead_lettered(
        &self,
        delivery_key: &str,
        owner: &str,
        reason: &str,
    ) -> Result<bool> {
        let reason = truncate_delivery_error(reason);
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE notification_delivery_state \
                SET status = 'dead_lettered', \
                    lease_owner = NULL, \
                    lease_until = NULL, \
                    next_retry_at = NULL, \
                    dead_lettered_at = now(), \
                    dead_letter_reason = $3, \
                    last_error = $3, \
                    updated_at = now() \
              WHERE delivery_key = $1 AND lease_owner = $2 AND status = 'delivering'",
        )
        .bind(delivery_key)
        .bind(owner)
        .bind(&reason)
        .execute(&mut *tx)
        .await?;
        let settled = result.rows_affected() > 0;
        if settled {
            record_delivery_attempt(&mut *tx, delivery_key, "dead_lettered", Some(&reason)).await?;
        }
        tx.commit().await?;
        Ok(settled)
    }

    /// Operator replay: reset a dead-lettered delivery so the retry processor
    /// attempts it again with a fresh attempt budget.
    pub async fn replay_dead_lettered_notification(&self, delivery_key: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE notification_delivery_state \
                SET status = 'pending', \
                    attempts = 0, \
                    next_retry_at = now(), \
                    lease_owner = NULL, \
                    lease_until = NULL, \
                    dead_lettered_at = NULL, \
                    dead_letter_reason = NULL, \
                    last_error = NULL, \
                    updated_at = now() \
              WHERE delivery_key = $1 AND status = 'dead_lettered'",
        )
        .bind(delivery_key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delivery backlog snapshot used by metrics and the readiness probe.
    pub async fn notification_delivery_backlog(
        &self,
        now: DateTime<Utc>,
    ) -> Result<NotificationBacklog> {
        notification_delivery_backlog_on(&self.pool, now).await
    }

    /// Full delivery health snapshot used by the `notification_delivery`
    /// readiness capability probe.
    pub async fn notification_delivery_health(
        &self,
        now: DateTime<Utc>,
        window_secs: i64,
    ) -> Result<NotificationDeliveryHealth> {
        notification_delivery_health_on(&self.pool, now, window_secs).await
    }

    /// Dead-lettered channel deliveries, newest first (admin UI).
    pub async fn list_dead_lettered_notifications(
        &self,
        limit: i64,
    ) -> Result<Vec<DeadLetterNotificationRow>> {
        let limit = limit.clamp(1, 200);
        Ok(sqlx::query_as::<_, DeadLetterNotificationRow>(
            "SELECT delivery_key, channel, destination, attempts, last_error, \
                    dead_lettered_at, created_at \
               FROM notification_delivery_state \
              WHERE status = 'dead_lettered' \
              ORDER BY dead_lettered_at DESC NULLS LAST, created_at DESC \
              LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}

/// Backlog snapshot that can run on a pool or a single connection (readiness
/// probe), without holding a transaction.
pub async fn notification_delivery_backlog_on<'e, E>(
    executor: E,
    now: DateTime<Utc>,
) -> Result<NotificationBacklog>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let (pending, overdue, dead_lettered) = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT \
             COUNT(*) FILTER (WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL)::bigint, \
             COUNT(*) FILTER (WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL \
                                AND (next_retry_at IS NULL OR next_retry_at <= $1))::bigint, \
             COUNT(*) FILTER (WHERE status = 'dead_lettered')::bigint \
           FROM notification_delivery_state",
    )
    .bind(now)
    .fetch_one(executor)
    .await?;
    Ok(NotificationBacklog {
        pending,
        overdue,
        dead_lettered,
    })
}

/// Append one settlement to the attempt log. Called in the same transaction as
/// the state update so the readiness success-ratio window cannot drift from
/// the settlement it describes.
async fn record_delivery_attempt<'e, E>(
    executor: E,
    delivery_key: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO notification_delivery_attempts (delivery_key, attempted_at, status, error) \
         VALUES ($1, now(), $2, $3)",
    )
    .bind(delivery_key)
    .bind(status)
    .bind(error)
    .execute(executor)
    .await?;
    Ok(())
}

/// Full delivery health snapshot that can run on a pool or a single connection
/// (readiness probe), without holding a transaction.
///
/// `window_secs` bounds the recent attempt/dead-letter change-rate figures.
pub async fn notification_delivery_health_on<'e, E>(
    executor: E,
    now: DateTime<Utc>,
    window_secs: i64,
) -> Result<NotificationDeliveryHealth>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let window_secs = window_secs.clamp(1, 86_400);
    let health = sqlx::query_as::<_, NotificationDeliveryHealth>(
        "SELECT \
             (SELECT COUNT(*) FROM notification_delivery_state \
               WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL)::bigint \
                 AS pending, \
             (SELECT COUNT(*) FROM notification_delivery_state \
               WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL \
                 AND (next_retry_at IS NULL OR next_retry_at <= $1))::bigint \
                 AS overdue, \
             (SELECT (EXTRACT(EPOCH FROM ($1 - MIN(next_retry_at)))::bigint) \
                FROM notification_delivery_state \
               WHERE status IN ('pending', 'failed') AND dead_lettered_at IS NULL \
                 AND next_retry_at IS NOT NULL AND next_retry_at <= $1) \
                 AS oldest_overdue_age_secs, \
             (SELECT COUNT(*) FROM notification_delivery_state \
               WHERE status = 'dead_lettered')::bigint \
                 AS dead_lettered, \
             (SELECT COUNT(*) FROM notification_delivery_state \
               WHERE status = 'dead_lettered' AND dead_lettered_at >= $1 - make_interval(secs => $2::double precision))::bigint \
                 AS dead_lettered_recent, \
             (SELECT COUNT(*) FROM notification_delivery_state \
               WHERE status = 'delivering' AND lease_until IS NOT NULL AND lease_until < $1)::bigint \
                 AS stuck_delivering, \
             (SELECT COUNT(*) FROM notification_delivery_attempts \
               WHERE attempted_at >= $1 - make_interval(secs => $2::double precision))::bigint \
                 AS attempts_recent, \
             (SELECT COUNT(*) FROM notification_delivery_attempts \
               WHERE attempted_at >= $1 - make_interval(secs => $2::double precision) \
                 AND status = 'delivered')::bigint \
                 AS delivered_recent, \
             (SELECT COUNT(*) FROM notification_delivery_attempts \
               WHERE attempted_at >= $1 - make_interval(secs => $2::double precision) \
                 AND status = 'dead_lettered')::bigint \
                 AS failed_recent",
    )
    .bind(now)
    .bind(window_secs as f64)
    .fetch_one(executor)
    .await?;
    Ok(health)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn payload_hash_is_stable_and_payload_sensitive() {
        let payload = serde_json::json!({"alert_id": "a", "body": "hello"});
        let first = notification_payload_hash(&payload);
        let second = notification_payload_hash(&payload);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);

        let changed = serde_json::json!({"alert_id": "a", "body": "hello!"});
        assert_ne!(first, notification_payload_hash(&changed));
    }

    #[test]
    fn delivery_error_is_bounded() {
        let long = "x".repeat(5000);
        let truncated = truncate_delivery_error(&long);
        assert_eq!(truncated.chars().count(), 2001);
        assert!(truncated.ends_with('…'));
    }
}
