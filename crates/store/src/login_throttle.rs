//! Shared login-throttle policy and state machine.
//!
//! One state machine backs every throttle backend: the in-memory tracker used
//! by tests/no-database deployments, the durable PostgreSQL table (migration
//! 069), and the Redis fallback. Keeping the policy here means a lockout
//! threshold can never drift between the backends.
//!
//! Policy (unchanged from the original in-memory tracker):
//!   * progressive backoff after each failure in the 10-minute window:
//!     1s, 2s, 4s, then 8s;
//!   * temporary lock for 10 minutes after 10 failures in 10 minutes;
//!   * admin unlock required after 20 failures in 1 hour.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Progressive backoff steps applied after the Nth failure in the 10-minute
/// window (clamped to the last step).
pub const LOGIN_BACKOFF_STEPS_SECS: [i64; 4] = [1, 2, 4, 8];
/// Failures in the 10-minute window that trigger the temporary lock.
pub const LOGIN_TEMP_LOCK_THRESHOLD_10M: i64 = 10;
/// Failures in the 1-hour window that require an administrator to unlock.
pub const LOGIN_ADMIN_LOCK_THRESHOLD_1H: i64 = 20;
/// Length of the short window, in seconds.
pub const LOGIN_WINDOW_10M_SECS: i64 = 600;
/// Length of the long window, in seconds.
pub const LOGIN_WINDOW_1H_SECS: i64 = 3600;
/// Length of the temporary lock, in seconds.
pub const LOGIN_TEMP_LOCK_SECS: i64 = 600;
/// Bounded lifetime of the strongest lock (20 failures in 1 hour). An admin
/// lock is not permanent: after this window the key unlocks and its durable
/// row/key expires, so unauthenticated failures can never create rows that
/// outlive the lock. Administrators can still release it earlier through
/// `clear_lock`.
pub const LOGIN_ADMIN_LOCK_SECS: i64 = 24 * 3600;
/// Lazy TTL for durable rows that carry no admin lock: one full long window
/// plus slack. The next read/write prunes expired rows.
pub const LOGIN_ROW_TTL_SECS: i64 = LOGIN_WINDOW_1H_SECS + 300;

/// Outcome of evaluating or recording one login attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginThrottleStatus {
    pub allowed: bool,
    pub retry_after_secs: u64,
    pub failure_count_10m: i64,
    pub failure_count_1h: i64,
    pub admin_unlock_required: bool,
}

impl Default for LoginThrottleStatus {
    fn default() -> Self {
        Self {
            allowed: true,
            retry_after_secs: 0,
            failure_count_10m: 0,
            failure_count_1h: 0,
            admin_unlock_required: false,
        }
    }
}

/// Whole seconds a caller must wait until `until`, rounded **up**.
///
/// Timestamps round-trip through PostgreSQL at microsecond precision while
/// `Utc::now()` carries nanoseconds on Linux, so a lock with 0.999999s
/// remaining must report 1 second — truncation would report 0 and tell the
/// caller to retry immediately against a live lock.
fn seconds_until(until: DateTime<Utc>, now: DateTime<Utc>) -> u64 {
    let millis = (until - now).num_milliseconds();
    if millis <= 0 {
        0
    } else {
        u64::try_from((millis + 999) / 1000).unwrap_or(u64::MAX)
    }
}

/// The throttle state for one attempt key: progressive backoff, a temporary
/// lock, and an admin lock, all derived from measured windows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginThrottleState {
    pub failures_10m: i64,
    pub failures_1h: i64,
    pub window_10m_started_at: Option<DateTime<Utc>>,
    pub window_1h_started_at: Option<DateTime<Utc>>,
    pub backoff_until: Option<DateTime<Utc>>,
    pub temp_lock_until: Option<DateTime<Utc>>,
    pub admin_locked: bool,
    /// When the admin lock releases on its own; `None` only for state that
    /// predates the bounded lock (it is treated as the full window).
    #[serde(default)]
    pub admin_lock_expires_at: Option<DateTime<Utc>>,
}

impl LoginThrottleState {
    /// Drop failures that fell out of their window and clear expired locks.
    pub fn prune(&mut self, now: DateTime<Utc>) {
        if self
            .window_10m_started_at
            .is_some_and(|start| (now - start).num_seconds() >= LOGIN_WINDOW_10M_SECS)
        {
            self.failures_10m = 0;
            self.window_10m_started_at = None;
        }
        if self
            .window_1h_started_at
            .is_some_and(|start| (now - start).num_seconds() >= LOGIN_WINDOW_1H_SECS)
        {
            self.failures_1h = 0;
            self.window_1h_started_at = None;
        }
        if self.backoff_until.is_some_and(|until| now >= until) {
            self.backoff_until = None;
        }
        if self.temp_lock_until.is_some_and(|until| now >= until) {
            self.temp_lock_until = None;
        }
        if self.admin_locked && self.admin_lock_expires_at.is_some_and(|until| now >= until) {
            self.admin_locked = false;
            self.admin_lock_expires_at = None;
        }
    }

    /// Derive the decision for `now` from the current state.
    pub fn status(&self, now: DateTime<Utc>) -> LoginThrottleStatus {
        let retry_after_secs = if self.admin_locked {
            self.admin_lock_expires_at
                .map(|until| seconds_until(until, now))
                .unwrap_or(0)
        } else if let Some(until) = self.temp_lock_until {
            seconds_until(until, now)
        } else if let Some(until) = self.backoff_until {
            seconds_until(until, now)
        } else {
            0
        };

        let allowed = !self.admin_locked
            && self.temp_lock_until.is_none_or(|until| now >= until)
            && self.backoff_until.is_none_or(|until| now >= until);
        LoginThrottleStatus {
            allowed,
            retry_after_secs,
            failure_count_10m: self.failures_10m,
            failure_count_1h: self.failures_1h,
            admin_unlock_required: self.admin_locked,
        }
    }

    /// Record one failed attempt at `now`, returning the new decision.
    pub fn record_failure(&mut self, now: DateTime<Utc>) -> LoginThrottleStatus {
        self.prune(now);

        if self.failures_10m == 0 {
            self.window_10m_started_at = Some(now);
        }
        if self.failures_1h == 0 {
            self.window_1h_started_at = Some(now);
        }
        self.failures_10m += 1;
        self.failures_1h += 1;

        if self.failures_1h >= LOGIN_ADMIN_LOCK_THRESHOLD_1H {
            self.admin_locked = true;
            // Bounded: each fresh failure extends the lock, but it always
            // releases on its own so a row can never become permanent.
            self.admin_lock_expires_at = Some(now + Duration::seconds(LOGIN_ADMIN_LOCK_SECS));
            self.backoff_until = None;
            self.temp_lock_until = None;
        } else if self.failures_10m >= LOGIN_TEMP_LOCK_THRESHOLD_10M {
            self.temp_lock_until = Some(now + Duration::seconds(LOGIN_TEMP_LOCK_SECS));
            self.backoff_until = None;
        } else {
            let index = (self.failures_10m - 1).clamp(0, i64::from(u32::MAX)) as usize;
            let delay = LOGIN_BACKOFF_STEPS_SECS[index.min(LOGIN_BACKOFF_STEPS_SECS.len() - 1)];
            self.backoff_until = Some(now + Duration::seconds(delay));
        }

        self.status(now)
    }

    /// Lazy-TTL deadline for a durable row. Every state has one: admin locks
    /// expire at their bounded deadline, so no row can outlive its lock and
    /// unauthenticated traffic cannot mint permanent rows.
    pub fn expires_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.admin_locked {
            Some(
                self.admin_lock_expires_at
                    .unwrap_or(now + Duration::seconds(LOGIN_ADMIN_LOCK_SECS)),
            )
        } else {
            Some(now + Duration::seconds(LOGIN_ROW_TTL_SECS))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A live lock must never report 0 seconds. PostgreSQL stores timestamps
    /// at microsecond precision while `Utc::now()` carries nanoseconds on
    /// Linux, so a 0.999999s remainder truncates to 0 without ceiling
    /// arithmetic — telling the caller to retry immediately.
    #[test]
    fn retry_seconds_round_up_for_sub_second_remainders() {
        let now = Utc::now();
        let mut state = LoginThrottleState::default();
        state.backoff_until = Some(now + Duration::nanoseconds(999_999_000));
        assert_eq!(state.status(now).retry_after_secs, 1);

        state.backoff_until = Some(now + Duration::milliseconds(1));
        assert_eq!(state.status(now).retry_after_secs, 1);

        state.backoff_until = Some(now + Duration::seconds(2));
        assert_eq!(state.status(now).retry_after_secs, 2);

        state.backoff_until = Some(now - Duration::milliseconds(1));
        assert_eq!(state.status(now).retry_after_secs, 0);
    }

    #[test]
    fn progressive_backoff_doubles_up_to_the_cap() {
        let now = Utc::now();
        let mut state = LoginThrottleState::default();

        assert_eq!(state.record_failure(now).retry_after_secs, 1);
        assert_eq!(
            state
                .record_failure(now + Duration::seconds(1))
                .retry_after_secs,
            2
        );
        assert_eq!(
            state
                .record_failure(now + Duration::seconds(3))
                .retry_after_secs,
            4
        );
        for step in 4..8 {
            state.record_failure(now + Duration::seconds(step * 10));
        }
        assert_eq!(
            state
                .record_failure(now + Duration::seconds(80))
                .retry_after_secs,
            8
        );
    }

    #[test]
    fn temp_lock_after_ten_failures_and_admin_lock_after_twenty() {
        let now = Utc::now();
        let mut state = LoginThrottleState::default();
        for offset in 0..10 {
            state.record_failure(now + Duration::seconds(offset));
        }
        let locked = state.status(now + Duration::seconds(9));
        assert!(!locked.allowed);
        assert_eq!(locked.retry_after_secs, LOGIN_TEMP_LOCK_SECS as u64);

        let mut admin = LoginThrottleState::default();
        for offset in 0..20 {
            admin.record_failure(now + Duration::minutes(offset));
        }
        assert!(
            admin
                .status(now + Duration::minutes(19))
                .admin_unlock_required
        );
    }

    #[test]
    fn admin_lock_releases_after_the_bounded_window() {
        let now = Utc::now();
        let mut state = LoginThrottleState::default();
        for offset in 0..20 {
            state.record_failure(now + Duration::minutes(offset));
        }
        let locked = state.status(now + Duration::minutes(20));
        assert!(locked.admin_unlock_required);
        assert!(!locked.allowed);
        assert!(
            locked.retry_after_secs > 0,
            "the bounded lock reports its remaining time"
        );
        assert!(
            state.expires_at(now + Duration::minutes(20)).is_some(),
            "admin locks must carry a lazy-TTL deadline"
        );

        let after = now + Duration::seconds(LOGIN_ADMIN_LOCK_SECS) + Duration::minutes(21);
        state.prune(after);
        let released = state.status(after);
        assert!(
            released.allowed,
            "the admin lock must release without manual intervention"
        );
        assert!(!released.admin_unlock_required);
    }

    #[test]
    fn expired_windows_reset_the_counters() {
        let now = Utc::now();
        let mut state = LoginThrottleState::default();
        state.record_failure(now);
        let later = now + Duration::minutes(11);
        assert!(state.status(later).allowed);
        let reset = state.record_failure(later);
        assert_eq!(reset.failure_count_10m, 1);
        assert_eq!(reset.failure_count_1h, 2);
    }
}
