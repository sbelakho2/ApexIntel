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
/// 30 min → 1 h → 2 h → 4 h → 8 h, then held at the last rung.
///
/// The ladder is capped by [`MAX_FAILURE_BACKOFF`] before the source's normal
/// interval is applied as a floor (see [`failure_backoff`]), so the cap can
/// never pull a source *below* its normal cadence: a source with a 24 h
/// interval keeps 24 h between attempts while failing.
pub const FAILURE_BACKOFF_LADDER: [Duration; 5] = [
    Duration::minutes(30),
    Duration::hours(1),
    Duration::hours(2),
    Duration::hours(4),
    Duration::hours(8),
];

/// Hard ceiling on the failure backoff ladder. Failures never push a source
/// out further than this unless its own configured interval is larger.
pub const MAX_FAILURE_BACKOFF: Duration = Duration::hours(8);

/// Smoothing factor for the persisted rolling success-rate and latency EWMAs.
/// Each attempt keeps 85% of the previous estimate and incorporates 15% of the
/// new observation.
pub const EWMA_ALPHA: f64 = 0.15;

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
/// failures.
///
/// The ladder is 30 min → 1 h → 2 h → 4 h → 8 h and is capped at
/// [`MAX_FAILURE_BACKOFF`]. The result is the *greater* of that capped ladder
/// step and the source's configured interval: failures never retry a source
/// faster than its normal cadence, and a long-interval source keeps its
/// interval (a 24 h source stays at 24 h while failing). There is no
/// `min(backoff, interval)` fallback — that inverted the ladder and let a
/// broken fast source keep retrying at full rate.
pub fn failure_backoff(consecutive_failures: i32, min_interval: Duration) -> Duration {
    let last_index = FAILURE_BACKOFF_LADDER.len() as i32 - 1;
    let index = (consecutive_failures.max(1) - 1).min(last_index) as usize;
    let ladder = FAILURE_BACKOFF_LADDER[index].min(MAX_FAILURE_BACKOFF);
    ladder.max(min_interval.max(Duration::zero()))
}

/// Update the persisted rolling success rate after one attempt.
///
/// Success: `old * 0.85 + 1.0 * 0.15`, or `1.0` when there is no history.
/// Failure: `old * 0.85`, or `0.0` when there is no history. The returned
/// value is always in `[0.0, 1.0]` and is persisted by both record methods.
pub fn ewma_success_rate(previous: Option<f64>, success: bool) -> f64 {
    let Some(previous) = previous else {
        return if success { 1.0 } else { 0.0 };
    };
    let retained = previous.clamp(0.0, 1.0) * (1.0 - EWMA_ALPHA);
    if success {
        retained + EWMA_ALPHA
    } else {
        retained
    }
}

/// Update the persisted rolling latency (ms) after one successful attempt.
///
/// `old * 0.85 + current * 0.15`; the first observation seeds the estimate and
/// a missing observation leaves the previous value untouched.
pub fn ewma_latency_ms(previous: Option<f64>, current_ms: Option<f64>) -> Option<f64> {
    match (previous, current_ms) {
        (previous, None) => previous,
        (None, Some(current)) => Some(current),
        (Some(previous), Some(current)) => {
            Some(previous * (1.0 - EWMA_ALPHA) + current * EWMA_ALPHA)
        }
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

    /// Record a failed attempt: increments `consecutive_failures`, decays the
    /// rolling success-rate EWMA, stores the error/http status, and pushes
    /// `next_due_at`/`circuit_open_until` out by the failure backoff (see
    /// [`failure_backoff`]).
    pub async fn record_source_attempt_failure(
        &self,
        source_slug: &str,
        last_error: &str,
        last_http_status: Option<i32>,
        min_interval: Duration,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let mut tx = self.pool.begin().await?;
        let previous: Option<(i32, Option<f64>)> = sqlx::query_as(
            "SELECT consecutive_failures, rolling_success_rate FROM source_runtime_state \
             WHERE source_slug = $1 FOR UPDATE",
        )
        .bind(source_slug)
        .fetch_optional(&mut *tx)
        .await?;
        let (previous_failures, previous_rate) = previous.unwrap_or((0, None));
        let consecutive_failures = previous_failures.saturating_add(1);
        let rolling_success_rate = ewma_success_rate(previous_rate, false);
        let next_due_at = now + failure_backoff(consecutive_failures, min_interval);

        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, next_due_at, consecutive_failures,
                   rolling_success_rate, last_http_status, circuit_open_until,
                   last_error, updated_at
               ) VALUES ($1, $2, $3, $4, $5, $6, $3, $7, $2)
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   next_due_at = EXCLUDED.next_due_at,
                   consecutive_failures = EXCLUDED.consecutive_failures,
                   rolling_success_rate = EXCLUDED.rolling_success_rate,
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
        .bind(rolling_success_rate)
        .bind(last_http_status)
        .bind(last_error)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }

    /// Record a successful attempt: resets the failure counter, clears the
    /// circuit breaker, advances both rolling EWMAs (success rate and latency),
    /// stamps `last_success_at` and schedules the next attempt at
    /// `now + min_interval`.
    pub async fn record_source_success(
        &self,
        source_slug: &str,
        min_interval: Duration,
        latency_ms: Option<f64>,
        last_http_status: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let mut tx = self.pool.begin().await?;
        let previous: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT rolling_success_rate, rolling_latency_ms FROM source_runtime_state \
             WHERE source_slug = $1 FOR UPDATE",
        )
        .bind(source_slug)
        .fetch_optional(&mut *tx)
        .await?;
        let (previous_rate, previous_latency) = previous.unwrap_or((None, None));
        let rolling_success_rate = ewma_success_rate(previous_rate, true);
        let rolling_latency_ms = ewma_latency_ms(previous_latency, latency_ms);
        let next_due_at = next_due_after_success(now, min_interval);

        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, last_success_at, next_due_at,
                   consecutive_failures, rolling_success_rate, rolling_latency_ms,
                   last_http_status, circuit_open_until, last_error, updated_at
               ) VALUES ($1, $2, $2, $3, 0, $4, $5, $6, NULL, NULL, $2)
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   last_success_at = EXCLUDED.last_success_at,
                   next_due_at = EXCLUDED.next_due_at,
                   consecutive_failures = 0,
                   rolling_success_rate = EXCLUDED.rolling_success_rate,
                   rolling_latency_ms = EXCLUDED.rolling_latency_ms,
                   last_http_status = EXCLUDED.last_http_status,
                   circuit_open_until = NULL,
                   last_error = NULL,
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(now)
        .bind(next_due_at)
        .bind(rolling_success_rate)
        .bind(rolling_latency_ms)
        .bind(last_http_status)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn failure_backoff_ladder_never_retries_faster_than_normal_interval() {
        let fast = Duration::minutes(15);
        assert_eq!(failure_backoff(1, fast), Duration::minutes(30));
        assert_eq!(failure_backoff(2, fast), Duration::hours(1));
        assert_eq!(failure_backoff(3, fast), Duration::hours(2));
        assert_eq!(failure_backoff(4, fast), Duration::hours(4));
        assert_eq!(failure_backoff(5, fast), Duration::hours(8));
        assert_eq!(failure_backoff(9, fast), Duration::hours(8));

        let hourly = Duration::minutes(60);
        assert_eq!(failure_backoff(1, hourly), Duration::minutes(60));
        assert_eq!(failure_backoff(2, hourly), Duration::hours(1));
        assert_eq!(failure_backoff(3, hourly), Duration::hours(2));
        assert_eq!(failure_backoff(9, hourly), Duration::hours(8));

        let zero = Duration::zero();
        assert_eq!(failure_backoff(1, zero), Duration::minutes(30));
        assert_eq!(failure_backoff(9, zero), Duration::hours(8));
    }

    #[test]
    fn failure_backoff_keeps_long_interval_sources_at_their_interval() {
        let daily = Duration::hours(24);
        for consecutive_failures in 0..12 {
            assert_eq!(
                failure_backoff(consecutive_failures, daily),
                Duration::hours(24),
                "a 24h-interval source must keep 24h after {consecutive_failures} failures"
            );
        }
        assert!(
            failure_backoff(9, daily) > MAX_FAILURE_BACKOFF,
            "the ladder cap must not pull a long-interval source below its own cadence"
        );
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn ewma_success_rate_seeds_and_decays() {
        assert_close(ewma_success_rate(None, true), 1.0);
        assert_close(ewma_success_rate(None, false), 0.0);
        assert_close(ewma_success_rate(Some(0.5), true), 0.575);
        assert_close(ewma_success_rate(Some(0.5), false), 0.425);
        assert_close(ewma_success_rate(Some(1.0), false), 0.85);
        assert_close(ewma_success_rate(Some(1.0), true), 1.0);
    }

    #[test]
    fn ewma_latency_keeps_prior_on_missing_observation() {
        assert_eq!(ewma_latency_ms(None, None), None);
        assert_eq!(ewma_latency_ms(None, Some(100.0)), Some(100.0));
        assert_eq!(ewma_latency_ms(Some(100.0), None), Some(100.0));
        let blended = ewma_latency_ms(Some(100.0), Some(200.0))
            .unwrap_or_else(|| panic!("EWMA must keep a value"));
        assert_close(blended, 115.0);
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
