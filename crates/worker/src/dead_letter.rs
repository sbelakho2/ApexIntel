//! Dead-Letter Queue (DLQ) for failed notification payloads.
//!
//! When webhook/email notifications fail after all retries, the failed payload
//! is persisted to the database for later replay or manual inspection, rather
//! than being silently dropped.
//!
//! The DLQ stores:
//! - The original notification payload
//! - Failure metadata (error message, channel, timestamp)
//! - Retry count and last attempt timestamp
//!
//! A background reaper process periodically retries queued items with
//! exponential backoff.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use apex_worker::notifications::{Notification, PendingAlert};

/// Maximum number of retry attempts before an item is permanently failed.
const MAX_DLQ_RETRIES: i32 = 5;

/// Initial delay between DLQ retry attempts (seconds).
const BASE_RETRY_DELAY_SECS: i64 = 60;

/// Maximum backoff multiplier (2^n, capped at this many seconds).
const MAX_RETRY_DELAY_SECS: i64 = 3600;

/// A dead-letter queue entry persisted to the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeadLetterEntry {
    pub id: Uuid,
    pub notification_id: String,
    pub channel: String,
    pub destination: Option<String>,
    pub alert_payload: serde_json::Value,
    pub formatted_body: String,
    pub error_message: String,
    pub retry_count: i32,
    pub first_failure_at: DateTime<Utc>,
    pub last_failure_at: DateTime<Utc>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub permanently_failed: bool,
    pub created_at: DateTime<Utc>,
}

/// Manages the dead-letter queue: enqueue, retry, and purge.
pub struct DeadLetterQueue {
    pool: PgPool,
}

impl DeadLetterQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensure the dead_letter_queue table exists (idempotent).
    pub async fn ensure_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dead_letter_queue (
                id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                notification_id TEXT NOT NULL,
                channel         TEXT NOT NULL,
                destination     TEXT,
                alert_payload   JSONB NOT NULL,
                formatted_body  TEXT NOT NULL,
                error_message   TEXT NOT NULL,
                retry_count     INTEGER NOT NULL DEFAULT 0,
                first_failure_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                last_failure_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
                next_retry_at    TIMESTAMPTZ,
                permanently_failed BOOLEAN NOT NULL DEFAULT false,
                created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                UNIQUE (notification_id, channel)
            );

            CREATE INDEX IF NOT EXISTS idx_dlq_next_retry
                ON dead_letter_queue (next_retry_at)
                WHERE permanently_failed = false AND next_retry_at IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_dlq_channel
                ON dead_letter_queue (channel, permanently_failed);
            "#,
        )
        .execute(&self.pool)
        .await?;

        info!("Dead-letter queue schema ensured");
        Ok(())
    }

    /// Enqueue a failed notification for later replay.
    pub async fn enqueue(
        &self,
        notification: &Notification,
        error_message: &str,
    ) -> Result<DeadLetterEntry> {
        let payload = serde_json::to_value(&notification.alert)?;
        let next_retry_at = Utc::now() + chrono::Duration::seconds(BASE_RETRY_DELAY_SECS);

        let row = sqlx::query_as::<_, DeadLetterEntry>(
            r#"
            INSERT INTO dead_letter_queue (
                notification_id, channel, destination, alert_payload,
                formatted_body, error_message, retry_count,
                first_failure_at, last_failure_at, next_retry_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, 0, now(), now(), $7)
            ON CONFLICT (notification_id, channel) DO UPDATE
            SET
                retry_count = dead_letter_queue.retry_count + 1,
                last_failure_at = now(),
                next_retry_at = $7,
                error_message = $6
            RETURNING *
            "#,
        )
        .bind(&notification.id)
        .bind(&notification.channel)
        .bind(&notification.destination)
        .bind(&payload)
        .bind(&notification.formatted_body)
        .bind(error_message)
        .bind(next_retry_at)
        .fetch_one(&self.pool)
        .await?;

        warn!(
            dlq_id = %row.id,
            channel = %row.channel,
            notification_id = %row.notification_id,
            retry_count = row.retry_count,
            "Notification moved to dead-letter queue",
        );

        Ok(row)
    }

    /// Fetch entries that are due for retry.
    pub async fn fetch_due_entries(&self, limit: usize) -> Result<Vec<DeadLetterEntry>> {
        let entries = sqlx::query_as::<_, DeadLetterEntry>(
            r#"
            SELECT *
            FROM dead_letter_queue
            WHERE permanently_failed = false
              AND next_retry_at <= now()
              AND retry_count < $1
            ORDER BY next_retry_at ASC
            LIMIT $2
            "#,
        )
        .bind(MAX_DLQ_RETRIES)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        debug!(
            count = entries.len(),
            "Fetched due dead-letter entries for retry"
        );
        Ok(entries)
    }

    /// Mark an entry as successfully replayed (remove from queue).
    pub async fn mark_succeeded(&self, entry_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM dead_letter_queue WHERE id = $1")
            .bind(entry_id)
            .execute(&self.pool)
            .await?;

        info!(dlq_id = %entry_id, "Dead-letter entry replayed successfully; removed from queue");
        Ok(())
    }

    /// Mark an entry as failed again with exponential backoff.
    pub async fn mark_failed_again(
        &self,
        entry_id: Uuid,
        error_message: &str,
    ) -> Result<()> {
        let entry = sqlx::query_as::<_, DeadLetterEntry>(
            "SELECT * FROM dead_letter_queue WHERE id = $1",
        )
        .bind(entry_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(entry) = entry else {
            return Ok(());
        };

        let new_retry_count = entry.retry_count + 1;

        if new_retry_count >= MAX_DLQ_RETRIES {
            // Permanently fail this entry
            sqlx::query(
                r#"
                UPDATE dead_letter_queue
                SET permanently_failed = true,
                    retry_count = $1,
                    last_failure_at = now(),
                    error_message = $2,
                    next_retry_at = NULL
                WHERE id = $3
                "#,
            )
            .bind(new_retry_count)
            .bind(error_message)
            .bind(entry_id)
            .execute(&self.pool)
            .await?;

            warn!(
                dlq_id = %entry_id,
                retry_count = new_retry_count,
                "Dead-letter entry permanently failed after max retries"
            );
        } else {
            // Exponential backoff: BASE * 2^retry_count, capped at MAX
            let delay_secs = (BASE_RETRY_DELAY_SECS * 2i64.pow(new_retry_count as u32))
                .min(MAX_RETRY_DELAY_SECS);
            let next_retry_at = Utc::now() + chrono::Duration::seconds(delay_secs);

            sqlx::query(
                r#"
                UPDATE dead_letter_queue
                SET retry_count = $1,
                    last_failure_at = now(),
                    error_message = $2,
                    next_retry_at = $3
                WHERE id = $4
                "#,
            )
            .bind(new_retry_count)
            .bind(error_message)
            .bind(next_retry_at)
            .bind(entry_id)
            .execute(&self.pool)
            .await?;

            debug!(
                dlq_id = %entry_id,
                retry_count = new_retry_count,
                next_retry_secs = delay_secs,
                "Dead-letter entry scheduled for retry"
            );
        }

        Ok(())
    }

    /// Get count of pending dead-letter entries.
    pub async fn pending_count(&self) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dead_letter_queue WHERE permanently_failed = false",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Purge all permanently failed entries older than the given duration.
    pub async fn purge_old_permanent_failures(&self, older_than_days: i64) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(older_than_days);
        let result = sqlx::query(
            r#"
            DELETE FROM dead_letter_queue
            WHERE permanently_failed = true AND last_failure_at < $1
            "#,
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected();
        if deleted > 0 {
            info!(
                deleted = deleted,
                cutoff = %cutoff,
                "Purged old permanently-failed dead-letter entries"
            );
        }

        Ok(deleted)
    }

    /// Convert a dead-letter entry back into a Notification for replay.
    pub fn entry_to_notification(entry: &DeadLetterEntry) -> Option<Notification> {
        let alert: PendingAlert = serde_json::from_value(entry.alert_payload.clone()).ok()?;

        Some(Notification {
            id: entry.notification_id.clone(),
            channel: entry.channel.clone(),
            destination: entry.destination.clone(),
            alert,
            subject: None,
            formatted_body: entry.formatted_body.clone(),
            dispatched_at: None,
            dispatch_success: None,
            error_message: Some(entry.error_message.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_retries_constant() {
        assert!(MAX_DLQ_RETRIES > 0, "DLQ should allow at least one retry");
        assert!(MAX_DLQ_RETRIES <= 10, "DLQ retries should be bounded");
    }

    #[test]
    fn test_backoff_is_bounded() {
        let delay = BASE_RETRY_DELAY_SECS * 2i64.pow(MAX_DLQ_RETRIES as u32);
        assert!(
            delay <= MAX_RETRY_DELAY_SECS || delay < 86400,
            "DLQ retry delay should be capped or reasonable"
        );
    }
}