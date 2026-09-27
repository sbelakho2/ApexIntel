//! Transactional alert outbox: claim/lease publisher storage (migration 061,
//! hardened by 069).
//!
//! # Delivery guarantee — at-least-once, not exactly-once
//!
//! A row is claimed with a lease in TX1 (`FOR UPDATE SKIP LOCKED` + lease
//! stamp), the JetStream publish is attempted **outside any transaction**, and
//! TX2 stamps either `published_at` (success) or the retry/dead-letter outcome.
//! A crash after a successful publish but before TX2 leaves the row unpublished
//! and the lease expires; the next drain republishes it. JetStream deduplicates
//! that redelivery through the `Nats-Msg-Id` header (the stable outbox row id)
//! inside the stream's duplicate window, and consumers are expected to be
//! idempotent. Exactly-once delivery is explicitly not claimed.

use super::*;

/// Maximum publish attempts before an outbox event stops being retried.
///
/// Without this cap a permanently unpublishable payload (schema mismatch,
/// rejected subject, oversize) would stay at the head of the oldest-first
/// drain forever and starve every newer alert once a full batch accrues.
/// Exhausted rows move to the explicit `dead_lettered` state (never falsely
/// delivered) and require an operator replay after the cause is fixed.
pub const MAX_OUTBOX_ATTEMPTS: i32 = 10;

/// Default lease for a claimed outbox row (seconds). Long enough to cover a
/// publish + ACK round trip, short enough that a crashed claim is retried
/// promptly.
pub const DEFAULT_OUTBOX_LEASE_SECS: f64 = 120.0;

/// One row of `event_outbox` (migration 061).
///
/// The outbox is written in the same transaction as its source aggregate (for
/// warnings: [`PgStore::insert_warning_with_outbox`]) and published at least
/// once per successful JetStream ACK by the single alert publisher.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventOutboxRow {
    pub id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub event_type: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub dead_letter_reason: Option<String>,
}

/// Result of draining one outbox batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxDrainOutcome {
    /// Events whose publish ACK was awaited and stamped `published_at`.
    pub published: usize,
    /// Events whose publish failed; `attempts` was already incremented at claim
    /// and `last_error` records the failure for the next drain.
    pub failed: usize,
    /// Events that exhausted [`MAX_OUTBOX_ATTEMPTS`] and are now dead-lettered.
    pub dead_lettered: usize,
}

/// Measured publisher backlog for readiness probes.
///
/// A healthy outbox publisher keeps `pending` small and `oldest_pending_at`
/// recent; `exhausted` counts poison rows that will never be retried and must
/// be surfaced instead of silently starving newer alerts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct OutboxBacklog {
    /// Unpublished events still eligible for retry (`attempts < MAX`).
    pub pending: i64,
    /// Unpublished events that exhausted [`MAX_OUTBOX_ATTEMPTS`].
    pub exhausted: i64,
    /// Creation time of the oldest unpublished event.
    pub oldest_pending_at: Option<DateTime<Utc>>,
    /// Newest successful publish, if anything was ever published.
    pub last_published_at: Option<DateTime<Utc>>,
}

/// A leased claim: the owner identity plus the rows it holds.
///
/// The database lock is released when the claim is created (TX1 commits); the
/// lease keeps other publishers from claiming the same rows.
#[derive(Debug, Clone)]
pub struct OutboxClaim {
    pub owner: String,
    pub rows: Vec<EventOutboxRow>,
}

impl OutboxClaim {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn into_parts(self) -> (String, Vec<EventOutboxRow>) {
        (self.owner, self.rows)
    }
}

/// Unpublished/backlog snapshot for metrics and readiness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxPublisherBacklog {
    /// Rows committed but not yet published (pending or retryable).
    pub unpublished: i64,
    /// Rows eligible for claiming for longer than the stuck threshold.
    pub overdue: i64,
    /// Rows that exhausted their attempts and need operator replay.
    pub dead_lettered: i64,
}

/// Dead-lettered outbox row (admin listing).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeadLetterOutboxRow {
    pub id: Uuid,
    pub event_type: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Keep `last_error` bounded so a pathological error cannot bloat the row.
fn truncate_outbox_error(error: &str) -> String {
    const MAX: usize = 2000;
    if error.chars().count() <= MAX {
        return error.to_string();
    }
    let truncated: String = error.chars().take(MAX).collect();
    format!("{truncated}…")
}

/// Columns returned by claim queries, qualified with the `event_outbox` alias:
/// the `FROM` also contains the `claimed` CTE, so unqualified `id` (and the
/// rest) would be ambiguous.
const OUTBOX_RETURNING: &str = "e.id, e.aggregate_type, e.aggregate_id, e.event_type, \
     e.payload, e.created_at, e.published_at, e.attempts, e.last_error, e.lease_owner, \
     e.lease_until, e.dead_lettered_at, e.dead_letter_reason";

impl PgStore {
    /// TX1: claim up to `limit` publishable outbox rows with a lease.
    ///
    /// `SELECT ... FOR UPDATE SKIP LOCKED` picks rows not already leased (or
    /// whose lease expired), then the same statement stamps the lease and
    /// increments `attempts` (the attempt is persisted BEFORE the publish). The
    /// transaction commits before the caller attempts any publish, so no
    /// database lock is held during the network call.
    pub async fn claim_unpublished_outbox(
        &self,
        owner: &str,
        lease_secs: f64,
        limit: i64,
    ) -> Result<OutboxClaim> {
        let limit = limit.clamp(1, 500);
        let mut tx = self.pool.begin().await?;
        let sql = format!(
            "WITH claimed AS ( \
                 SELECT id FROM event_outbox \
                  WHERE published_at IS NULL \
                    AND dead_lettered_at IS NULL \
                    AND attempts < $4 \
                    AND (lease_until IS NULL OR lease_until <= now()) \
                  ORDER BY created_at ASC, id ASC \
                  LIMIT $3 \
                  FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE event_outbox e \
                SET lease_owner = $1, \
                    lease_until = now() + make_interval(secs => $2::double precision), \
                    attempts = e.attempts + 1 \
               FROM claimed \
              WHERE e.id = claimed.id \
             RETURNING {OUTBOX_RETURNING}"
        );
        let rows = sqlx::query_as::<_, EventOutboxRow>(&sql)
            .bind(owner)
            .bind(lease_secs)
            .bind(limit)
            .bind(MAX_OUTBOX_ATTEMPTS)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(OutboxClaim {
            owner: owner.to_string(),
            rows,
        })
    }

    /// TX1: claim one specific publishable outbox row with a lease.
    ///
    /// Returns an empty claim when the row is already leased by another
    /// publisher, published, or dead-lettered, so the fast path can skip its
    /// attempt and leave delivery to the drain.
    pub async fn claim_outbox_event(
        &self,
        owner: &str,
        lease_secs: f64,
        id: Uuid,
    ) -> Result<OutboxClaim> {
        let mut tx = self.pool.begin().await?;
        let sql = format!(
            "WITH claimed AS ( \
                 SELECT id FROM event_outbox \
                  WHERE id = $3 \
                    AND published_at IS NULL \
                    AND dead_lettered_at IS NULL \
                    AND attempts < $4 \
                    AND (lease_until IS NULL OR lease_until <= now()) \
                  FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE event_outbox e \
                SET lease_owner = $1, \
                    lease_until = now() + make_interval(secs => $2::double precision), \
                    attempts = e.attempts + 1 \
               FROM claimed \
              WHERE e.id = claimed.id \
             RETURNING {OUTBOX_RETURNING}"
        );
        let rows = sqlx::query_as::<_, EventOutboxRow>(&sql)
            .bind(owner)
            .bind(lease_secs)
            .bind(id)
            .bind(MAX_OUTBOX_ATTEMPTS)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(OutboxClaim {
            owner: owner.to_string(),
            rows,
        })
    }

    /// TX2a: stamp `published_at` after the broker ACK was awaited.
    ///
    /// The `lease_owner` guard makes a stale settlement a no-op; returns
    /// `false` when the lease was lost.
    pub async fn mark_outbox_published(&self, id: Uuid, owner: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE event_outbox \
                SET published_at = now(), last_error = NULL, \
                    lease_owner = NULL, lease_until = NULL \
              WHERE id = $1 AND lease_owner = $2 AND published_at IS NULL",
        )
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// TX2b: record a failed publish attempt.
    ///
    /// `attempts` was already incremented at claim time. Rows that reached
    /// [`MAX_OUTBOX_ATTEMPTS`] move to the explicit dead-letter state; returns
    /// `true` when this attempt dead-lettered the row.
    pub async fn record_outbox_failure(&self, id: Uuid, owner: &str, error: &str) -> Result<bool> {
        let error = truncate_outbox_error(error);
        let (dead_lettered,): (bool,) = sqlx::query_as(
            "UPDATE event_outbox \
                SET last_error = $3, \
                    lease_owner = NULL, \
                    lease_until = NULL, \
                    dead_lettered_at = CASE WHEN attempts >= $4 THEN COALESCE(dead_lettered_at, now()) ELSE dead_lettered_at END, \
                    dead_letter_reason = CASE WHEN attempts >= $4 THEN COALESCE(dead_letter_reason, $3) ELSE dead_letter_reason END \
              WHERE id = $1 AND lease_owner = $2 AND published_at IS NULL \
             RETURNING dead_lettered_at IS NOT NULL",
        )
        .bind(id)
        .bind(owner)
        .bind(error)
        .bind(MAX_OUTBOX_ATTEMPTS)
        .fetch_one(&self.pool)
        .await?;
        Ok(dead_lettered)
    }

    /// Release leases held by `owner` for rows that were not settled (e.g. the
    /// publish task was aborted). Rows settle normally through
    /// [`Self::mark_outbox_published`] / [`Self::record_outbox_failure`].
    pub async fn release_outbox_claims(&self, owner: &str, ids: &[Uuid]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            "UPDATE event_outbox \
                SET lease_owner = NULL, lease_until = NULL \
              WHERE lease_owner = $1 AND published_at IS NULL AND id = ANY($2)",
        )
        .bind(owner)
        .bind(ids)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Unpublished/backlog snapshot used by metrics and readiness.
    ///
    /// `overdue` counts rows that have been claimable since before
    /// `stuck_before` (i.e. the drain is not keeping up).
    pub async fn outbox_readiness_backlog(
        &self,
        _stuck_before: DateTime<Utc>,
    ) -> Result<OutboxBacklog> {
        // Readiness cares about retryable vs exhausted work and publish
        // freshness; `stuck_before` is retained for signature compatibility
        // with the scheduler's clock.
        let (pending, exhausted, oldest_pending_at, last_published_at) =
            sqlx::query_as::<_, (i64, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
                "SELECT \
                   COUNT(*) FILTER (WHERE published_at IS NULL \
                                      AND dead_lettered_at IS NULL \
                                      AND attempts < $1)::BIGINT AS pending, \
                   COUNT(*) FILTER (WHERE published_at IS NULL \
                                      AND (dead_lettered_at IS NOT NULL \
                                           OR attempts >= $1))::BIGINT AS exhausted, \
                   MIN(created_at) FILTER (WHERE published_at IS NULL) AS oldest_pending_at, \
                   MAX(published_at) AS last_published_at \
                 FROM event_outbox",
            )
            .bind(MAX_OUTBOX_ATTEMPTS)
            .fetch_one(&self.pool)
            .await?;
        Ok(OutboxBacklog {
            pending,
            exhausted,
            oldest_pending_at,
            last_published_at,
        })
    }

    /// Dead-lettered outbox rows, newest first (admin UI).
    pub async fn list_dead_lettered_outbox(&self, limit: i64) -> Result<Vec<DeadLetterOutboxRow>> {
        let limit = limit.clamp(1, 200);
        Ok(sqlx::query_as::<_, DeadLetterOutboxRow>(
            "SELECT id, event_type, attempts, last_error, dead_lettered_at, created_at \
               FROM event_outbox \
              WHERE dead_lettered_at IS NOT NULL \
              ORDER BY dead_lettered_at DESC, created_at DESC \
              LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Operator replay: clear the dead-letter state and reset the attempt
    /// budget so the canonical drain retries the event.
    pub async fn replay_dead_lettered_outbox(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE event_outbox \
                SET dead_lettered_at = NULL, \
                    dead_letter_reason = NULL, \
                    attempts = 0, \
                    last_error = NULL, \
                    lease_owner = NULL, \
                    lease_until = NULL \
              WHERE id = $1 AND dead_lettered_at IS NOT NULL AND published_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Operator alert for a dead-lettered outbox event: an `activity_feed` row
    /// (error level log happens at the call site as well).
    pub async fn insert_dead_letter_operator_alert(
        &self,
        row: &EventOutboxRow,
        reason: &str,
    ) -> Result<()> {
        let details = serde_json::json!({
            "outbox_id": row.id.to_string(),
            "event_type": row.event_type,
            "aggregate_type": row.aggregate_type,
            "aggregate_id": row.aggregate_id.to_string(),
            "attempts": row.attempts,
            "reason": truncate_outbox_error(reason),
        });
        sqlx::query(
            "INSERT INTO activity_feed \
                 (actor_id, actor_name, action_type, entity_type, entity_id, entity_name, \
                  details, workspace_id, team_id, visibility, created_at) \
             VALUES ('system', 'Alert Outbox', 'alert_dead_lettered', 'event_outbox', $1, $2, \
                     $3, NULL, NULL, 'team', NOW())",
        )
        .bind(row.id.to_string())
        .bind(&row.event_type)
        .bind(details)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether the hardened lease/dead-letter columns exist (startup check).
    pub async fn event_outbox_lease_columns_present(&self) -> Result<bool> {
        let (present,): (bool,) = sqlx::query_as(
            "SELECT COUNT(*) = 4 FROM information_schema.columns \
              WHERE table_schema = 'public' AND table_name = 'event_outbox' \
                AND column_name IN ('lease_owner', 'lease_until', 'dead_lettered_at', 'dead_letter_reason')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(present)
    }

    /// Whether the `event_outbox` table exists (startup capability check).
    ///
    /// Warning persistence depends on it (migration 061); a worker running
    /// against a database without it would fail every warning insert.
    pub async fn event_outbox_present(&self) -> Result<bool> {
        let (present,): (bool,) =
            sqlx::query_as("SELECT to_regclass('public.event_outbox') IS NOT NULL")
                .fetch_one(&self.pool)
                .await?;
        Ok(present)
    }

    /// Measure the publisher backlog with bounded, index-backed queries.
    ///
    /// The pending aggregates filter on `published_at IS NULL` so the partial
    /// `idx_event_outbox_unpublished` index applies, and the last publish is a
    /// `MAX` over the partial `idx_event_outbox_published_at` index. Health
    /// probes poll this frequently, and the table is append-only, so neither
    /// query may scan published history.
    pub async fn outbox_backlog(&self) -> Result<OutboxBacklog> {
        let (pending, exhausted, oldest_pending_at) =
            sqlx::query_as::<_, (i64, i64, Option<DateTime<Utc>>)>(
                r#"SELECT
                       COUNT(*) FILTER (WHERE attempts < $1),
                       COUNT(*) FILTER (WHERE attempts >= $1),
                       MIN(created_at)
                     FROM event_outbox
                    WHERE published_at IS NULL"#,
            )
            .bind(MAX_OUTBOX_ATTEMPTS)
            .fetch_one(&self.pool)
            .await?;

        let last_published_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT MAX(published_at) FROM event_outbox WHERE published_at IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(OutboxBacklog {
            pending,
            exhausted,
            oldest_pending_at,
            last_published_at,
        })
    }
}

/// Backlog snapshot that can run on a pool or a single connection (readiness
/// probe), without holding a transaction.
pub async fn outbox_backlog_on<'e, E>(
    executor: E,
    stuck_before: DateTime<Utc>,
) -> Result<OutboxPublisherBacklog>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let (unpublished, overdue, dead_lettered) = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT \
             COUNT(*) FILTER (WHERE published_at IS NULL AND dead_lettered_at IS NULL)::bigint, \
             COUNT(*) FILTER (WHERE published_at IS NULL AND dead_lettered_at IS NULL \
                                AND created_at <= $1)::bigint, \
             COUNT(*) FILTER (WHERE dead_lettered_at IS NOT NULL)::bigint \
           FROM event_outbox",
    )
    .bind(stuck_before)
    .fetch_one(executor)
    .await?;
    Ok(OutboxPublisherBacklog {
        unpublished,
        overdue,
        dead_lettered,
    })
}

#[cfg(test)]
mod tests {
    use super::truncate_outbox_error;

    #[test]
    fn outbox_error_is_bounded() {
        let long = "x".repeat(5000);
        let truncated = truncate_outbox_error(&long);
        assert_eq!(truncated.chars().count(), 2001);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn short_outbox_error_is_preserved() {
        assert_eq!(truncate_outbox_error("boom"), "boom");
    }
}
