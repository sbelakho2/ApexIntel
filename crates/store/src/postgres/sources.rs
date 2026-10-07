//! Per-source crawl scheduling state (migration 047).
//!
//! The weighted-fair crawl scheduler in `apex-crawl` reads
//! [`SourceRuntimeStateRow`]s through the `SourceRuntimeStateProvider` trait;
//! the nightly crawl cycle writes attempt outcomes back through
//! [`PgStore::record_source_success`],
//! [`PgStore::record_source_attempt_failure`] and — for sources the
//! deployment cannot execute (e.g. a `Browser` source with the headless
//! renderer disabled) — [`PgStore::mark_source_unavailable`].

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
    /// Newest published timestamp seen in the last successful feed parse
    /// (migration 110). Lets detection tell a quiet feed apart from a stalled
    /// ingestion path.
    pub last_item_at: Option<DateTime<Utc>>,
    /// Items the last successful parse produced (migration 110).
    pub last_item_count: Option<i32>,
    pub updated_at: DateTime<Utc>,
}

/// Row shape consumed by the due-source selector.
pub type DueSourceRow = SourceRuntimeStateRow;

const SOURCE_RUNTIME_COLUMNS: &str = "source_slug, last_attempt_at, last_success_at, \
     next_due_at, consecutive_failures, rolling_success_rate, rolling_latency_ms, \
     last_http_status, circuit_open_until, etag, last_modified, last_error, \
     last_item_at, last_item_count, updated_at";

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
    ///
    /// One `INSERT ... ON CONFLICT DO UPDATE` merges the new observation into
    /// the stored row, so two workers recording concurrent attempts cannot
    /// lose an increment or an EWMA step the way the previous read-then-upsert
    /// pair could. The backoff ladder is expressed in SQL because the merged
    /// `consecutive_failures` value is only known inside the statement; it must
    /// stay in lockstep with [`FAILURE_BACKOFF_LADDER`] ([`failure_backoff`]).
    pub async fn record_source_attempt_failure(
        &self,
        source_slug: &str,
        last_error: &str,
        last_http_status: Option<i32>,
        min_interval: Duration,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let min_interval_secs = min_interval.num_seconds().max(0) as f64;
        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, next_due_at, consecutive_failures,
                   rolling_success_rate, last_http_status, circuit_open_until,
                   last_error, updated_at
               ) VALUES (
                   $1, $2,
                   $2 + GREATEST(interval '30 minutes', make_interval(secs => $5)),
                   1, 0.0, $3,
                   $2 + GREATEST(interval '30 minutes', make_interval(secs => $5)),
                   $4, $2
               )
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   consecutive_failures = source_runtime_state.consecutive_failures + 1,
                   rolling_success_rate = LEAST(
                       GREATEST(COALESCE(source_runtime_state.rolling_success_rate, 0.0), 0.0),
                       1.0
                   ) * (1.0 - $6::DOUBLE PRECISION),
                   next_due_at = EXCLUDED.last_attempt_at + GREATEST(
                       CASE
                           WHEN source_runtime_state.consecutive_failures + 1 <= 1
                               THEN interval '30 minutes'
                           WHEN source_runtime_state.consecutive_failures + 1 = 2
                               THEN interval '1 hour'
                           WHEN source_runtime_state.consecutive_failures + 1 = 3
                               THEN interval '2 hours'
                           WHEN source_runtime_state.consecutive_failures + 1 = 4
                               THEN interval '4 hours'
                           WHEN source_runtime_state.consecutive_failures + 1 >= 6
                               THEN interval '7 days'
                           ELSE interval '8 hours'
                       END,
                       make_interval(secs => $5)),
                   last_http_status = EXCLUDED.last_http_status,
                   circuit_open_until = EXCLUDED.last_attempt_at + GREATEST(
                       CASE
                           WHEN source_runtime_state.consecutive_failures + 1 <= 1
                               THEN interval '30 minutes'
                           WHEN source_runtime_state.consecutive_failures + 1 = 2
                               THEN interval '1 hour'
                           WHEN source_runtime_state.consecutive_failures + 1 = 3
                               THEN interval '2 hours'
                           WHEN source_runtime_state.consecutive_failures + 1 = 4
                               THEN interval '4 hours'
                           WHEN source_runtime_state.consecutive_failures + 1 >= 6
                               THEN interval '7 days'
                           ELSE interval '8 hours'
                       END,
                       make_interval(secs => $5)),
                   last_error = EXCLUDED.last_error,
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(now)
        .bind(last_http_status)
        .bind(last_error)
        .bind(min_interval_secs)
        .bind(EWMA_ALPHA)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Record a **parser** failure for a source: the fetch succeeded but the
    /// body no longer deserializes (schema change), so the source must be
    /// marked degraded and backed off without ever touching `last_success_at`.
    ///
    /// This shares the failure backoff/EWMA path with
    /// [`Self::record_source_attempt_failure`]; the distinct entry point and
    /// `parser_failure:` error prefix let operators separate transport
    /// failures from parser incidents. `last_success_at` is preserved by the
    /// underlying upsert (it is not part of the conflict update), so a source
    /// with earlier successful parses keeps its validation timestamp.
    pub async fn record_source_parse_failure(
        &self,
        source_slug: &str,
        parser_error: &str,
        redacted_sample: Option<&str>,
        last_http_status: Option<i32>,
        min_interval: Duration,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let mut message = format!("parser_failure: {parser_error}");
        if let Some(sample) = redacted_sample {
            if !sample.is_empty() {
                message.push_str(&format!(" | sample: {sample}"));
            }
        }
        self.record_source_attempt_failure(
            source_slug,
            &message,
            last_http_status,
            min_interval,
            now,
        )
        .await
    }

    /// Record a successful attempt: resets the failure counter, clears the
    /// circuit breaker, advances both rolling EWMAs (success rate and latency),
    /// stamps `last_success_at` and schedules the next attempt at
    /// `now + min_interval`.
    ///
    /// Like [`Self::record_source_attempt_failure`], this is one
    /// `INSERT ... ON CONFLICT DO UPDATE`: the stored EWMA values are read and
    /// blended inside the statement, so concurrent workers cannot overwrite
    /// each other's counter/EWMA updates with stale values.
    #[allow(clippy::too_many_arguments)] // Flat audit payload; the storage statement is the single call boundary.
    pub async fn record_source_success(
        &self,
        source_slug: &str,
        min_interval: Duration,
        latency_ms: Option<f64>,
        last_http_status: Option<i32>,
        newest_item_at: Option<DateTime<Utc>>,
        item_count: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let next_due_at = next_due_after_success(now, min_interval);
        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, last_attempt_at, last_success_at, next_due_at,
                   consecutive_failures, rolling_success_rate, rolling_latency_ms,
                   last_http_status, circuit_open_until, last_error,
                   last_item_at, last_item_count, updated_at
               ) VALUES ($1, $2, $2, $3, 0, 1.0, $4, $5, NULL, NULL, $7, $8, $2)
               ON CONFLICT (source_slug) DO UPDATE SET
                   last_attempt_at = EXCLUDED.last_attempt_at,
                   last_success_at = EXCLUDED.last_success_at,
                   next_due_at = EXCLUDED.next_due_at,
                   consecutive_failures = 0,
                   rolling_success_rate = LEAST(
                       GREATEST(COALESCE(source_runtime_state.rolling_success_rate, 1.0), 0.0),
                       1.0
                   ) * (1.0 - $6::DOUBLE PRECISION) + $6::DOUBLE PRECISION,
                   rolling_latency_ms = CASE
                       WHEN $4::DOUBLE PRECISION IS NULL
                           THEN source_runtime_state.rolling_latency_ms
                       WHEN source_runtime_state.rolling_latency_ms IS NULL
                           THEN $4::DOUBLE PRECISION
                       ELSE source_runtime_state.rolling_latency_ms
                                * (1.0 - $6::DOUBLE PRECISION)
                            + $4::DOUBLE PRECISION * $6::DOUBLE PRECISION
                   END,
                   last_http_status = EXCLUDED.last_http_status,
                   circuit_open_until = NULL,
                   last_error = NULL,
                   last_item_at = COALESCE(EXCLUDED.last_item_at,
                                           source_runtime_state.last_item_at),
                   last_item_count = COALESCE(EXCLUDED.last_item_count,
                                              source_runtime_state.last_item_count),
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(now)
        .bind(next_due_at)
        .bind(latency_ms)
        .bind(last_http_status)
        .bind(EWMA_ALPHA)
        .bind(newest_item_at)
        .bind(item_count)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Mark a source unavailable because this deployment cannot satisfy its
    /// required fetch capability (for example a `Browser`-strategy source
    /// while the headless renderer is disabled).
    ///
    /// This is **not** a crawl attempt: `last_attempt_at` and
    /// `consecutive_failures` are left untouched, but the circuit is opened
    /// until `now + retry_after` and `last_error` records the capability gap
    /// so the scheduler backs off and the admin surfaces stay truthful.
    pub async fn mark_source_unavailable(
        &self,
        source_slug: &str,
        last_error: &str,
        retry_after: Duration,
        now: DateTime<Utc>,
    ) -> Result<SourceRuntimeStateRow> {
        let next_due_at = now + retry_after;
        let row = sqlx::query_as::<_, SourceRuntimeStateRow>(&format!(
            r#"INSERT INTO source_runtime_state (
                   source_slug, next_due_at, consecutive_failures,
                   circuit_open_until, last_error, updated_at
               ) VALUES ($1, $2, 0, $2, $3, $4)
               ON CONFLICT (source_slug) DO UPDATE SET
                   next_due_at = EXCLUDED.next_due_at,
                   circuit_open_until = EXCLUDED.circuit_open_until,
                   last_error = EXCLUDED.last_error,
                   updated_at = EXCLUDED.updated_at
               RETURNING {SOURCE_RUNTIME_COLUMNS}"#
        ))
        .bind(source_slug)
        .bind(next_due_at)
        .bind(last_error)
        .bind(now)
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
