//! Durable login throttle backed by `login_attempt_throttle` (migration 069).
//!
//! Every API replica shares one row per attempt key, and `record_failure`
//! reads the row with `SELECT ... FOR UPDATE` inside a transaction, so
//! concurrent failures from different replicas cannot lose an increment.
//!
//! Rows carry a lazy TTL (`expires_at`): expired rows are deleted by the next
//! write (and by the read that touches them), keeping the table bounded
//! without a background job. Admin-locked rows have `expires_at IS NULL` and
//! are only released by [`PgStore::login_throttle_record_success`] or
//! [`PgStore::login_throttle_clear_lock`].

use super::*;
use crate::login_throttle::{LoginThrottleState, LoginThrottleStatus};

#[derive(Debug, Clone, sqlx::FromRow)]
struct LoginThrottleRow {
    failure_count_10m: i32,
    failure_count_1h: i32,
    window_10m_started_at: Option<DateTime<Utc>>,
    window_1h_started_at: Option<DateTime<Utc>>,
    backoff_until: Option<DateTime<Utc>>,
    temp_lock_until: Option<DateTime<Utc>>,
    admin_locked: bool,
    expires_at: Option<DateTime<Utc>>,
}

impl From<LoginThrottleRow> for LoginThrottleState {
    fn from(row: LoginThrottleRow) -> Self {
        Self {
            failures_10m: i64::from(row.failure_count_10m),
            failures_1h: i64::from(row.failure_count_1h),
            window_10m_started_at: row.window_10m_started_at,
            window_1h_started_at: row.window_1h_started_at,
            backoff_until: row.backoff_until,
            temp_lock_until: row.temp_lock_until,
            admin_locked: row.admin_locked,
            // For admin-locked rows `expires_at` holds the bounded admin-lock
            // deadline (the lazy sweep deletes the row when it passes).
            admin_lock_expires_at: if row.admin_locked {
                row.expires_at
            } else {
                None
            },
        }
    }
}

const LOGIN_THROTTLE_SELECT: &str = "SELECT failure_count_10m, failure_count_1h, \
     window_10m_started_at, window_1h_started_at, backoff_until, temp_lock_until, \
     admin_locked, expires_at FROM login_attempt_throttle";

impl PgStore {
    /// Evaluate the throttle for `attempt_key` at `now` without recording an
    /// attempt. Expired rows are dropped (lazy TTL) and report as untouched.
    pub async fn login_throttle_evaluate(
        &self,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus> {
        let sql = format!("{LOGIN_THROTTLE_SELECT} WHERE attempt_key = $1");
        let row: Option<LoginThrottleRow> = sqlx::query_as(&sql)
            .bind(attempt_key)
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(LoginThrottleStatus::default());
        };

        if row.expires_at.is_some_and(|expires_at| expires_at < now) {
            sqlx::query(
                "DELETE FROM login_attempt_throttle WHERE attempt_key = $1 AND expires_at < $2",
            )
            .bind(attempt_key)
            .bind(now)
            .execute(&self.pool)
            .await?;
            return Ok(LoginThrottleStatus::default());
        }

        let mut state = LoginThrottleState::from(row);
        state.prune(now);
        Ok(state.status(now))
    }

    /// Record one failed attempt at `now` and return the resulting decision.
    ///
    /// The read-modify-write runs in a transaction with a row lock, so the
    /// counter is exact even when two replicas record failures concurrently.
    pub async fn login_throttle_record_failure(
        &self,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus> {
        let mut tx = self.pool.begin().await?;

        // Serialize on the attempt key (see `login_throttle_reserve`): the
        // insert-then-select path has the same lost-update window for a key
        // that does not exist yet.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(attempt_key)
            .execute(&mut *tx)
            .await?;

        // Lazy GC: drop rows whose windows and locks are long over. Admin
        // locks have NULL expires_at and are never swept.
        sqlx::query(
            "DELETE FROM login_attempt_throttle WHERE expires_at IS NOT NULL AND expires_at < $1",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO login_attempt_throttle (attempt_key) VALUES ($1) ON CONFLICT (attempt_key) DO NOTHING")
            .bind(attempt_key)
            .execute(&mut *tx)
            .await?;

        let sql = format!("{LOGIN_THROTTLE_SELECT} WHERE attempt_key = $1 FOR UPDATE");
        let row: LoginThrottleRow = sqlx::query_as(&sql)
            .bind(attempt_key)
            .fetch_one(&mut *tx)
            .await?;

        let mut state = LoginThrottleState::from(row);
        let status = state.record_failure(now);
        let expires_at = state.expires_at(now);

        sqlx::query(
            "UPDATE login_attempt_throttle SET \
                 failure_count_10m = $2, \
                 failure_count_1h = $3, \
                 window_10m_started_at = $4, \
                 window_1h_started_at = $5, \
                 backoff_until = $6, \
                 temp_lock_until = $7, \
                 admin_locked = $8, \
                 expires_at = $9, \
                 updated_at = now() \
             WHERE attempt_key = $1",
        )
        .bind(attempt_key)
        .bind(i32::try_from(state.failures_10m).unwrap_or(i32::MAX))
        .bind(i32::try_from(state.failures_1h).unwrap_or(i32::MAX))
        .bind(state.window_10m_started_at)
        .bind(state.window_1h_started_at)
        .bind(state.backoff_until)
        .bind(state.temp_lock_until)
        .bind(state.admin_locked)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(status)
    }

    /// Atomically check-and-reserve one login attempt.
    ///
    /// A blocked key is returned without mutating state. Otherwise the attempt
    /// is recorded (so concurrent requests cannot all pass a check made before
    /// any failure was recorded) and the caller receives `allowed: true` for
    /// this attempt; `record_success` clears the reservation on a valid login.
    pub async fn login_throttle_reserve(
        &self,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus> {
        let mut tx = self.pool.begin().await?;

        // Serialize on the attempt key for the transaction. `SELECT ... FOR
        // UPDATE` locks no row for a key that does not exist yet, so two
        // concurrent first attempts would both read the default state and the
        // loser would overwrite the winner's counter. The transaction-scoped
        // advisory lock closes that window (released automatically on
        // commit/rollback).
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(attempt_key)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "DELETE FROM login_attempt_throttle WHERE expires_at IS NOT NULL AND expires_at < $1",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let sql = format!("{LOGIN_THROTTLE_SELECT} WHERE attempt_key = $1 FOR UPDATE");
        let existing: Option<LoginThrottleRow> = sqlx::query_as(&sql)
            .bind(attempt_key)
            .fetch_optional(&mut *tx)
            .await?;

        let is_new = existing.is_none();
        let mut state = match existing {
            Some(row) => LoginThrottleState::from(row),
            None => LoginThrottleState::default(),
        };
        state.prune(now);
        let pre = state.status(now);
        if !pre.allowed {
            tx.commit().await?;
            return Ok(pre);
        }

        if is_new {
            sqlx::query(
                "INSERT INTO login_attempt_throttle (attempt_key) VALUES ($1) \
                 ON CONFLICT (attempt_key) DO NOTHING",
            )
            .bind(attempt_key)
            .execute(&mut *tx)
            .await?;
        }

        let post = state.record_failure(now);
        let expires_at = state.expires_at(now);
        sqlx::query(
            "UPDATE login_attempt_throttle SET \
                 failure_count_10m = $2, \
                 failure_count_1h = $3, \
                 window_10m_started_at = $4, \
                 window_1h_started_at = $5, \
                 backoff_until = $6, \
                 temp_lock_until = $7, \
                 admin_locked = $8, \
                 expires_at = $9, \
                 updated_at = now() \
             WHERE attempt_key = $1",
        )
        .bind(attempt_key)
        .bind(i32::try_from(state.failures_10m).unwrap_or(i32::MAX))
        .bind(i32::try_from(state.failures_1h).unwrap_or(i32::MAX))
        .bind(state.window_10m_started_at)
        .bind(state.window_1h_started_at)
        .bind(state.backoff_until)
        .bind(state.temp_lock_until)
        .bind(state.admin_locked)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(LoginThrottleStatus {
            allowed: true,
            retry_after_secs: 0,
            failure_count_10m: post.failure_count_10m,
            failure_count_1h: post.failure_count_1h,
            admin_unlock_required: false,
        })
    }

    /// Clear all throttle state for `attempt_key` (successful login).
    pub async fn login_throttle_record_success(&self, attempt_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM login_attempt_throttle WHERE attempt_key = $1")
            .bind(attempt_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Administrator unlock: clear the throttle and report whether any state
    /// existed.
    pub async fn login_throttle_clear_lock(&self, attempt_key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM login_attempt_throttle WHERE attempt_key = $1")
            .bind(attempt_key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete durable rows whose TTL has passed. Exposed for maintenance
    /// tasks; the login path also prunes lazily.
    pub async fn login_throttle_prune_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM login_attempt_throttle WHERE expires_at IS NOT NULL AND expires_at < $1",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
