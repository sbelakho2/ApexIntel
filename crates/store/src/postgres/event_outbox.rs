use super::*;

/// Maximum publish attempts before an outbox event stops being retried.
///
/// Without this cap a permanently unpublishable payload (schema mismatch,
/// rejected subject, oversize) would stay at the head of the oldest-first
/// drain forever and starve every newer alert once a full batch accrues.
/// Exhausted rows remain `published_at IS NULL` (never falsely delivered) and
/// keep their `last_error` for operator inspection.
pub const MAX_OUTBOX_ATTEMPTS: i32 = 10;

/// One row of `event_outbox` (migration 061).
///
/// The outbox is written in the same transaction as its source aggregate (for
/// warnings: [`PgStore::insert_warning_with_outbox`]) and published exactly
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
}

/// Result of draining one outbox batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxDrainOutcome {
    /// Events whose publish ACK was awaited and stamped `published_at`.
    pub published: usize,
    /// Events whose publish failed; `attempts` was incremented and `last_error`
    /// recorded, and they remain unpublished for the next drain.
    pub failed: usize,
}

/// A batch of unpublished outbox events locked with `FOR UPDATE SKIP LOCKED`.
///
/// The transaction is held open until [`OutboxBatch::commit`] so two concurrent
/// publishers can never publish the same event: a second publisher skips the
/// locked rows. If the process dies mid-publish the transaction rolls back and
/// the events stay unpublished, so the next drain retries them instead of the
/// alert being lost (at-least-once delivery).
pub struct OutboxBatch {
    tx: Transaction<'static, Postgres>,
    events: Vec<EventOutboxRow>,
}

impl OutboxBatch {
    /// The locked events, oldest first.
    pub fn events(&self) -> &[EventOutboxRow] {
        &self.events
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Stamp `published_at` after the broker ACK was awaited.
    pub async fn mark_published(&mut self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE event_outbox SET published_at = now(), last_error = NULL \
             WHERE id = $1 AND published_at IS NULL",
        )
        .bind(id)
        .execute(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// Record a failed publish attempt without marking the event published.
    pub async fn record_failure(&mut self, id: Uuid, error: &str) -> Result<()> {
        let error = truncate_outbox_error(error);
        sqlx::query(
            "UPDATE event_outbox SET attempts = attempts + 1, last_error = $2 WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .execute(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// Commit the batch: locks are released and every stamped `published_at`
    /// becomes durable.
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
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

impl PgStore {
    /// Lock up to `limit` unpublished, still-retryable outbox events.
    ///
    /// Uses `SELECT ... FOR UPDATE SKIP LOCKED`: rows locked by another
    /// publisher are skipped instead of blocking, and the lock is held until the
    /// returned [`OutboxBatch`] is committed or rolled back. Events that already
    /// exhausted [`MAX_OUTBOX_ATTEMPTS`] are excluded so poison rows cannot
    /// starve newer alerts. The caller must publish each event, await the broker
    /// ACK, call `mark_published`, and only then commit.
    pub async fn lock_unpublished_outbox(&self, limit: i64) -> Result<OutboxBatch> {
        let limit = limit.clamp(1, 500);
        let mut tx = self.pool.begin().await?;
        let events = sqlx::query_as::<_, EventOutboxRow>(
            "SELECT id, aggregate_type, aggregate_id, event_type, payload, created_at, \
                    published_at, attempts, last_error \
               FROM event_outbox \
              WHERE published_at IS NULL AND attempts < $2 \
              ORDER BY created_at ASC, id ASC \
              LIMIT $1 \
              FOR UPDATE SKIP LOCKED",
        )
        .bind(limit)
        .bind(MAX_OUTBOX_ATTEMPTS)
        .fetch_all(&mut *tx)
        .await?;
        Ok(OutboxBatch { tx, events })
    }

    /// Lock one specific unpublished, still-retryable outbox event.
    ///
    /// Returns an empty batch when another publisher already holds the row
    /// (`SKIP LOCKED`) or when it is published/exhausted, so the caller can skip
    /// its immediate delivery attempt and let the drain own the event.
    pub async fn lock_outbox_event(&self, id: Uuid) -> Result<OutboxBatch> {
        let mut tx = self.pool.begin().await?;
        let events = sqlx::query_as::<_, EventOutboxRow>(
            "SELECT id, aggregate_type, aggregate_id, event_type, payload, created_at, \
                    published_at, attempts, last_error \
               FROM event_outbox \
              WHERE id = $1 AND published_at IS NULL AND attempts < $2 \
              FOR UPDATE SKIP LOCKED",
        )
        .bind(id)
        .bind(MAX_OUTBOX_ATTEMPTS)
        .fetch_all(&mut *tx)
        .await?;
        Ok(OutboxBatch { tx, events })
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
