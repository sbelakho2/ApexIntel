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

/// The throttle state for one attempt key.
///
/// Windows are represented by a counter plus the time the current window
/// started: that keeps the durable representation to a single row of plain
/// columns while never counting a failure twice or forgetting one inside the
/// window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginThrottleState {
    pub failures_10m: i64,
    pub failures_1h: i64,
    pub window_10m_started_at: Option<DateTime<Utc>>,
    pub window_1h_started_at: Option<DateTime<Utc>>,
    pub backoff_until: Option<DateTime<Utc>>,
    pub temp_lock_until: Option<DateTime<Utc>>,
    pub admin_locked: bool,
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
    }

    /// Derive the decision for `now` from the current state.
    pub fn status(&self, now: DateTime<Utc>) -> LoginThrottleStatus {
        let retry_after_secs = if self.admin_locked {
            0
        } else if let Some(until) = self.temp_lock_until {
            (until - now).num_seconds().max(0) as u64
        } else if let Some(until) = self.backoff_until {
            (until - now).num_seconds().max(0) as u64
        } else {
            0
        };

        LoginThrottleStatus {
            allowed: !self.admin_locked
                && self.temp_lock_until.is_none_or(|until| now >= until)
                && self.backoff_until.is_none_or(|until| now >= until),
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

    /// Lazy-TTL deadline for a durable row: `None` for admin locks, which stay
    /// until explicitly cleared.
    pub fn expires_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.admin_locked {
            None
        } else {
            Some(now + Duration::seconds(LOGIN_ROW_TTL_SECS))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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
