use super::*;

/// One row of `event_outbox` (migration 061).
///
/// The outbox is written in the same transaction as its source aggregate (for
/// warnings: [`PgStore::insert_warning_with_outbox`]) and published exactly
/// once per successful JetStream ACK by the worker's single alert publisher.
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

/// An event to append to the outbox in the same transaction as its aggregate.
#[derive(Debug, Clone)]
pub struct NewOutboxEvent {
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub event_type: String,
    pub payload: Value,
}

impl NewOutboxEvent {
    pub fn new(
        aggregate_type: impl Into<String>,
        aggregate_id: Uuid,
        event_type: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            aggregate_type: aggregate_type.into(),
            aggregate_id,
            event_type: event_type.into(),
            payload,
        }
    }
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

    pub fn len(&self) -> usize {
        self.events.len()
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

    /// Roll the whole batch back (nothing was marked published).
    pub async fn rollback(self) -> Result<()> {
        self.tx.rollback().await?;
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
    /// Append an event to the outbox. Callers that need atomicity with their
    /// aggregate must use the transactional variants (e.g.
    /// [`PgStore::insert_warning_with_outbox`]).
    pub async fn enqueue_outbox_event(&self, event: &NewOutboxEvent) -> Result<Uuid> {
        let (id,) = sqlx::query_as::<_, (Uuid,)>(
            "INSERT INTO event_outbox (aggregate_type, aggregate_id, event_type, payload) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(&event.aggregate_type)
        .bind(event.aggregate_id)
        .bind(&event.event_type)
        .bind(&event.payload)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Lock up to `limit` unpublished outbox events for publishing.
    ///
    /// Uses `SELECT ... FOR UPDATE SKIP LOCKED`: rows locked by another
    /// publisher are skipped instead of blocking, and the lock is held until the
    /// returned [`OutboxBatch`] is committed or rolled back. The caller must
    /// publish each event, await the broker ACK, call `mark_published`, and only
    /// then commit.
    pub async fn lock_unpublished_outbox(&self, limit: i64) -> Result<OutboxBatch> {
        let limit = limit.clamp(1, 500);
        let mut tx = self.pool.begin().await?;
        let events = sqlx::query_as::<_, EventOutboxRow>(
            "SELECT id, aggregate_type, aggregate_id, event_type, payload, created_at, \
                    published_at, attempts, last_error \
               FROM event_outbox \
              WHERE published_at IS NULL \
              ORDER BY created_at ASC, id ASC \
              LIMIT $1 \
              FOR UPDATE SKIP LOCKED",
        )
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        Ok(OutboxBatch { tx, events })
    }

    /// Stamp `published_at` on one outbox event after the broker ACK was
    /// awaited. Returns `true` when this call marked it (a second call is a
    /// no-op, so duplicate fast-path/drain publishes cannot double-stamp).
    pub async fn mark_outbox_published(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE event_outbox SET published_at = now(), last_error = NULL \
             WHERE id = $1 AND published_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Count unpublished outbox events (dashboard/health checks).
    pub async fn count_unpublished_outbox(&self) -> Result<i64> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*)::bigint FROM event_outbox WHERE published_at IS NULL")
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
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
