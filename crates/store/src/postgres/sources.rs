//! Per-source crawl scheduling state (migration 047).
//!
//! The weighted-fair crawl scheduler in `apex-crawl` reads
//! [`SourceRuntimeStateRow`]s through the `SourceRuntimeStateProvider` trait;
//! the nightly crawl cycle writes attempt outcomes back through
//! [`PgStore::record_source_success`] and
//! [`PgStore::record_source_attempt_failure`].

use super::*;
use chrono::Duration;

/// Failure backoff ladder applied by the scheduler when a source fails:
/// 30 min → 1 h → 2 h → 4 h → 8 h (then capped at 8 h), further capped by the
/// source's configured minimum interval.
pub const FAILURE_BACKOFF_LADDER: [Duration; 5] = [
    Duration::minutes(30),
    Duration::hours(1),
    Duration::hours(2),
    Duration::hours(4),
    Duration::hours(8),
];

/// Persisted scheduling state for one source slug.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SourceRuntimeStateRow {
    pub source_slug: String,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub next_due_at: DateTime<Utc>,
    pub consecutive_failures: i32,
    pub rolling_success_rate: Option<f64>,
    pub rolling_latency_ms: Option<f64>,
    pub last_http_status: Option<i32>,
    pub circuit_open_until: Option<DateTime<Utc>>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Row shape consumed by the due-source selector.
pub type DueSourceRow = SourceRuntimeStateRow;

const SOURCE_RUNTIME_COLUMNS: &str = "source_slug, last_attempt_at, last_success_at, \
     next_due_at, consecutive_failures, rolling_success_rate, rolling_latency_ms, \
     last_http_status, circuit_open_until, etag, last_modified, last_error, updated_at";

/// Backoff before the next attempt after `consecutive_failures` consecutive
/// failures. The ladder is 30 min → 1 h → 2 h → 4 h → 8 h; the result never
/// exceeds the source's configured interval, so a source with a short
/// interval is retried at its normal cadence even while failing.
pub fn failure_backoff(consecutive_failures: i32, min_interval: Duration) -> Duration {
    let last_index = FAILURE_BACKOFF_LADDER.len() as i32 - 1;
    let index = (consecutive_failures.max(1) - 1).min(last_index) as usize;
    let backoff = FAILURE_BACKOFF_LADDER[index];
    if min_interval > Duration::zero() && backoff > min_interval {
        min_interval
    } else {
        backoff
    }
}

/// Due time after a successful attempt: now + the source's minimum interval.
pub fn next_due_after_success(now: DateTime<Utc>, min_interval: Duration) -> DateTime<Utc> {
    now + min_interval
}

impl PgStore {
    /// Load every persisted source runtime state row, ordered by slug.
    pub async fn load_source_runtime_states(&self) -> Result<Vec<SourceRuntimeStateRow>> {
        let rows = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            "SELECT {SOURCE_RUNTIME_COLUMNS} FROM source_runtime_state ORDER BY source_slug"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Record a failed attempt: increments `consecutive_failures`, stores the
    /// error/http status, and pushes `next_due_at`/`circuit_open_until` out by
    /// the failure backoff (see [`failure_backoff`]).
    pub async fn record_source_attempt_failure(
        &self,
        source_slug: &str,
        last_error: &str,
        last_http_status: Option<i32>,
        min_interval: Duration,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let mut tx = self.pool.begin().await?;
        let previous_failures: Option<i32> = sqlx::query_scalar(
            "SELECT consecutive_failures FROM source_runtime_state \
             WHERE source_slug = $1 FOR UPDATE",
        )
        .bind(source_slug)
        .fetch_optional(&mut *tx)
        .await?;
        let consecutive_failures = previous_failures.unwrap_or(0).saturating_add(1);
        let next_due_at = now + failure_backoff(consecutive_failures, min_interval);

        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, next_due_at, consecutive_failures,
                   last_http_status, circuit_open_until, last_error, updated_at
               ) VALUES ($1, $2, $3, $4, $5, $3, $6, $2)
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   next_due_at = EXCLUDED.next_due_at,
                   consecutive_failures = EXCLUDED.consecutive_failures,
                   last_http_status = EXCLUDED.last_http_status,
                   circuit_open_until = EXCLUDED.circuit_open_until,
                   last_error = EXCLUDED.last_error,
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(now)
        .bind(next_due_at)
        .bind(consecutive_failures)
        .bind(last_http_status)
        .bind(last_error)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }

    /// Record a successful attempt: resets the failure counter, clears the
    /// circuit breaker, stamps `last_success_at` and schedules the next
    /// attempt at `now + min_interval`.
    pub async fn record_source_success(
        &self,
        source_slug: &str,
        min_interval: Duration,
        rolling_latency_ms: Option<f64>,
        last_http_status: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let next_due_at = next_due_after_success(now, min_interval);
        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, last_success_at, next_due_at,
                   consecutive_failures, last_http_status, circuit_open_until,
                   rolling_latency_ms, last_error, updated_at
               ) VALUES ($1, $2, $2, $3, 0, $4, NULL, $5, NULL, $2)
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   last_success_at = EXCLUDED.last_success_at,
                   next_due_at = EXCLUDED.next_due_at,
                   consecutive_failures = 0,
                   last_http_status = EXCLUDED.last_http_status,
                   circuit_open_until = NULL,
                   rolling_latency_ms = EXCLUDED.rolling_latency_ms,
                   last_error = NULL,
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(now)
        .bind(next_due_at)
        .bind(last_http_status)
        .bind(rolling_latency_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn failure_backoff_ladder_is_capped_by_source_interval() {
        let wide = Duration::hours(24);
        assert_eq!(failure_backoff(0, wide), Duration::minutes(30));
        assert_eq!(failure_backoff(1, wide), Duration::minutes(30));
        assert_eq!(failure_backoff(2, wide), Duration::hours(1));
        assert_eq!(failure_backoff(3, wide), Duration::hours(2));
        assert_eq!(failure_backoff(4, wide), Duration::hours(4));
        assert_eq!(failure_backoff(5, wide), Duration::hours(8));
        assert_eq!(failure_backoff(9, wide), Duration::hours(8));

        let tight = Duration::minutes(60);
        assert_eq!(failure_backoff(1, tight), Duration::minutes(30));
        assert_eq!(failure_backoff(2, tight), Duration::minutes(60));
        assert_eq!(failure_backoff(5, tight), Duration::minutes(60));
    }

    #[test]
    fn success_schedules_next_attempt_at_min_interval() {
        let now = Utc::now();
        assert_eq!(
            next_due_after_success(now, Duration::minutes(15)),
            now + Duration::minutes(15)
        );
        assert_eq!(next_due_after_success(now, Duration::zero()), now);
    }
}
