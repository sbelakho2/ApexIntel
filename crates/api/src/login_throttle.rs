//! Durable, multi-replica login throttle.
//!
//! Audit item 1: the login path evaluated the in-memory
//! [`crate::auth::AuthAttemptTracker`], whose counters live in one process —
//! a restart cleared them and each replica kept its own. This facade keeps the
//! same policy but moves the state to shared storage:
//!
//!   * **Redis** when `REDIS_URL` is configured — atomic Lua script so the
//!     read-modify-write is one server-side operation;
//!   * **PostgreSQL** otherwise (migration 069,
//!     [`apex_store::postgres::PgStore`] methods) — transaction with a row
//!     lock so concurrent failures cannot lose increments;
//!   * **in-memory** only when no store is available (unit tests / no-database
//!     deployments).
//!
//! Every configured durable backend is consulted and written to: a lockout
//! recorded in one backend can never be hidden by another backend answering
//! first (e.g. PostgreSQL during a Redis outage, then Redis after recovery).
//! When a configured backend errors at call time the facade merges whatever
//! the other durable backends report rather than failing open.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use redis::AsyncCommands;

use apex_store::postgres::PgStore;

use crate::auth::{AuthAttemptTracker, AuthThrottleStatus};

/// Which backend a [`LoginThrottle`] was configured with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LoginThrottleBackend {
    Redis,
    Postgres,
    Memory,
}

impl LoginThrottleBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::Memory => "memory",
        }
    }

    /// Durable backends survive a process restart and are shared by replicas.
    pub fn is_durable(self) -> bool {
        matches!(self, Self::Redis | Self::Postgres)
    }
}

/// Login throttle with a durable backend when one is configured.
#[derive(Clone)]
pub struct LoginThrottle {
    store: Option<Arc<PgStore>>,
    redis: Option<redis::aio::ConnectionManager>,
    memory: AuthAttemptTracker,
    backend: LoginThrottleBackend,
}

impl LoginThrottle {
    /// Build the strongest configured backend. Redis wins when a connection
    /// manager is present; otherwise PostgreSQL. With neither, the in-memory
    /// fallback is used (tests/dev only).
    pub fn new(store: Option<Arc<PgStore>>, redis: Option<redis::aio::ConnectionManager>) -> Self {
        let backend = match (&redis, &store) {
            (Some(_), _) => LoginThrottleBackend::Redis,
            (None, Some(_)) => LoginThrottleBackend::Postgres,
            (None, None) => LoginThrottleBackend::Memory,
        };
        Self {
            store,
            redis,
            memory: AuthAttemptTracker::new(),
            backend,
        }
    }

    /// In-memory fallback (unit tests and no-database deployments).
    pub fn in_memory() -> Self {
        Self::new(None, None)
    }

    pub fn backend(&self) -> LoginThrottleBackend {
        self.backend
    }

    pub fn is_durable(&self) -> bool {
        self.backend.is_durable()
    }

    /// Check whether an attempt is currently allowed. Must run before the
    /// password is verified.
    ///
    /// All configured durable backends are queried and the most restrictive
    /// answer wins, so state recorded in one backend is never hidden by
    /// another backend that answers first with "allowed".
    pub async fn evaluate(&self, attempt_key: &str, now: DateTime<Utc>) -> AuthThrottleStatus {
        let mut statuses: Vec<AuthThrottleStatus> = Vec::new();
        if let Some(redis) = &self.redis {
            match redis_backend::evaluate(redis.clone(), attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "Redis login throttle read failed"),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_evaluate(attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "PostgreSQL login throttle read failed"),
            }
        }
        merge_statuses(statuses).unwrap_or_else(|| self.memory.evaluate(attempt_key, now))
    }

    /// Atomically reserve one login attempt across every configured backend.
    ///
    /// A locked key is rejected without incrementing; otherwise the attempt is
    /// recorded and `allowed: true` is returned for this attempt. This closes
    /// the evaluate-then-record race where N concurrent requests all passed the
    /// check before any failure was recorded.
    pub async fn reserve(&self, attempt_key: &str, now: DateTime<Utc>) -> AuthThrottleStatus {
        let mut statuses: Vec<AuthThrottleStatus> = Vec::new();
        if let Some(redis) = &self.redis {
            match redis_backend::reserve(redis.clone(), attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "Redis login throttle reserve failed"),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_reserve(attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "PostgreSQL login throttle reserve failed"),
            }
        }
        merge_statuses(statuses).unwrap_or_else(|| self.memory.reserve(attempt_key, now))
    }

    /// Record a failed attempt and return the resulting decision.
    ///
    /// The failure is written to every configured durable backend so a later
    /// fail-over cannot lose the lockout; the most restrictive resulting
    /// status is reported.
    pub async fn record_failure(
        &self,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> AuthThrottleStatus {
        let mut statuses: Vec<AuthThrottleStatus> = Vec::new();
        if let Some(redis) = &self.redis {
            match redis_backend::record_failure(redis.clone(), attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "Redis login throttle write failed"),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_record_failure(attempt_key, now).await {
                Ok(status) => statuses.push(status.into()),
                Err(error) => tracing::warn!(%error, "PostgreSQL login throttle write failed"),
            }
        }
        merge_statuses(statuses).unwrap_or_else(|| self.memory.record_failure(attempt_key, now))
    }

    /// Clear all state after a successful login.
    pub async fn record_success(&self, attempt_key: &str) {
        if let Some(redis) = &self.redis {
            let mut connection = redis.clone();
            let key = redis_backend::storage_key(attempt_key);
            let deleted: Result<i64, redis::RedisError> = connection.del(&key).await;
            if let Err(error) = deleted {
                tracing::warn!(%error, "Redis login throttle clear failed");
            }
        }
        if let Some(store) = &self.store {
            if let Err(error) = store.login_throttle_record_success(attempt_key).await {
                tracing::warn!(%error, "PostgreSQL login throttle clear failed");
            }
        }
        self.memory.record_success(attempt_key);
    }

    /// Administrator unlock. Returns whether any backend held state.
    pub async fn clear_lock(&self, attempt_key: &str) -> bool {
        let mut cleared = self.memory.clear_lock(attempt_key);
        if let Some(redis) = &self.redis {
            let mut connection = redis.clone();
            let key = redis_backend::storage_key(attempt_key);
            let deleted: Result<i64, redis::RedisError> = connection.del(&key).await;
            match deleted {
                Ok(count) => cleared |= count > 0,
                Err(error) => tracing::warn!(%error, "Redis login throttle unlock failed"),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_clear_lock(attempt_key).await {
                Ok(existed) => cleared |= existed,
                Err(error) => tracing::warn!(%error, "PostgreSQL login throttle unlock failed"),
            }
        }
        cleared
    }
}

/// Merge the answers of every configured backend, keeping the most
/// restrictive one: an admin lock wins, then any blocked status (longest
/// retry first), then the highest failure counts. `None` when no durable
/// backend answered.
fn merge_statuses(mut statuses: Vec<AuthThrottleStatus>) -> Option<AuthThrottleStatus> {
    let first = statuses.pop()?;
    Some(statuses.into_iter().fold(first, most_restrictive))
}

fn most_restrictive(a: AuthThrottleStatus, b: AuthThrottleStatus) -> AuthThrottleStatus {
    if a.admin_unlock_required != b.admin_unlock_required {
        return if a.admin_unlock_required { a } else { b };
    }
    if a.allowed != b.allowed {
        return if a.allowed { b } else { a };
    }
    if a.retry_after_secs != b.retry_after_secs {
        return if a.retry_after_secs > b.retry_after_secs {
            a
        } else {
            b
        };
    }
    AuthThrottleStatus {
        allowed: a.allowed,
        retry_after_secs: a.retry_after_secs,
        failure_count_10m: a.failure_count_10m.max(b.failure_count_10m),
        failure_count_1h: a.failure_count_1h.max(b.failure_count_1h),
        admin_unlock_required: a.admin_unlock_required,
    }
}

/// Redis backend: the whole read-modify-write runs as one Lua script, so
/// increments are atomic across replicas and the key TTL is refreshed in the
/// same operation.
mod redis_backend {
    use chrono::{DateTime, Utc};

    use apex_store::login_throttle::{
        LoginThrottleStatus, LOGIN_ADMIN_LOCK_SECS, LOGIN_ADMIN_LOCK_THRESHOLD_1H,
        LOGIN_BACKOFF_STEPS_SECS, LOGIN_ROW_TTL_SECS, LOGIN_TEMP_LOCK_SECS,
        LOGIN_TEMP_LOCK_THRESHOLD_10M, LOGIN_WINDOW_10M_SECS, LOGIN_WINDOW_1H_SECS,
    };

    pub const KEY_PREFIX: &str = "login_throttle:";

    pub fn storage_key(attempt_key: &str) -> String {
        format!("{KEY_PREFIX}{attempt_key}")
    }

    /// Keep the Lua constants in lockstep with the Rust policy constants.
    /// (Redis scripts cannot read Rust constants, so the script is built once
    /// with the values interpolated.)
    fn script_source() -> &'static str {
        static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        SCRIPT
            .get_or_init(|| {
                format!(
                    r#"
local key = KEYS[1]
local mode = ARGV[1]
local now = tonumber(ARGV[2])
local window_10m_ms = {window_10m_ms}
local window_1h_ms = {window_1h_ms}
local temp_threshold = {temp_threshold}
local admin_threshold = {admin_threshold}
local temp_lock_ms = {temp_lock_ms}
local admin_lock_ms = {admin_lock_ms}
local ttl_secs = {ttl_secs}
local admin_ttl_secs = {admin_ttl_secs}
local steps = {{{step_1}, {step_2}, {step_3}, {step_4}}}

local fields = redis.call('HMGET', key, 'f10', 'f1h', 'w10', 'w1h', 'backoff', 'lock', 'admin', 'a_until')
local f10 = tonumber(fields[1]) or 0
local f1h = tonumber(fields[2]) or 0
local w10 = tonumber(fields[3]) or 0
local w1h = tonumber(fields[4]) or 0
local backoff = tonumber(fields[5]) or 0
local lock = tonumber(fields[6]) or 0
local admin = fields[7] == '1'
local a_until = tonumber(fields[8]) or 0

if w10 > 0 and now - w10 >= window_10m_ms then f10 = 0; w10 = 0 end
if w1h > 0 and now - w1h >= window_1h_ms then f1h = 0; w1h = 0 end
if backoff > 0 and now >= backoff then backoff = 0 end
if lock > 0 and now >= lock then lock = 0 end
if admin and a_until > 0 and now >= a_until then admin = false; a_until = 0 end

-- reserve: blocked keys are answered without mutating state
if mode == 'reserve' and (admin or lock > now or backoff > now) then
    local retry = 0
    if admin then
        if a_until > now then retry = math.ceil((a_until - now) / 1000) end
    elseif lock > now then
        retry = math.ceil((lock - now) / 1000)
    elseif backoff > now then
        retry = math.ceil((backoff - now) / 1000)
    end
    return {{0, retry, f10, f1h, admin and 1 or 0}}
end

if mode == 'record' or mode == 'reserve' then
    if f10 == 0 then w10 = now end
    if f1h == 0 then w1h = now end
    f10 = f10 + 1
    f1h = f1h + 1
    if f1h >= admin_threshold then
        admin = true
        a_until = now + admin_lock_ms
        backoff = 0
        lock = 0
    elseif f10 >= temp_threshold then
        lock = now + temp_lock_ms
        backoff = 0
    else
        local idx = f10
        if idx > 4 then idx = 4 end
        backoff = now + steps[idx]
    end
    redis.call('HSET', key,
        'f10', f10, 'f1h', f1h, 'w10', w10, 'w1h', w1h,
        'backoff', backoff, 'lock', lock, 'admin', admin and '1' or '0',
        'a_until', a_until)
    if admin then
        redis.call('EXPIRE', key, admin_ttl_secs)
    else
        redis.call('EXPIRE', key, ttl_secs)
    end
end

-- reserve: the reserved attempt itself is allowed; the next one observes the
-- recorded state.
if mode == 'reserve' then
    return {{1, 0, f10, f1h, 0}}
end

local retry = 0
if admin then
    if a_until > now then
        retry = math.ceil((a_until - now) / 1000)
    end
elseif lock > now then
    retry = math.ceil((lock - now) / 1000)
elseif backoff > now then
    retry = math.ceil((backoff - now) / 1000)
end
local allowed = 1
if admin or lock > now or backoff > now then allowed = 0 end
return {{allowed, retry, f10, f1h, admin and 1 or 0}}
"#,
                    window_10m_ms = LOGIN_WINDOW_10M_SECS * 1000,
                    window_1h_ms = LOGIN_WINDOW_1H_SECS * 1000,
                    temp_threshold = LOGIN_TEMP_LOCK_THRESHOLD_10M,
                    admin_threshold = LOGIN_ADMIN_LOCK_THRESHOLD_1H,
                    temp_lock_ms = LOGIN_TEMP_LOCK_SECS * 1000,
                    admin_lock_ms = LOGIN_ADMIN_LOCK_SECS * 1000,
                    ttl_secs = LOGIN_ROW_TTL_SECS,
                    admin_ttl_secs = LOGIN_ADMIN_LOCK_SECS,
                    step_1 = LOGIN_BACKOFF_STEPS_SECS[0] * 1000,
                    step_2 = LOGIN_BACKOFF_STEPS_SECS[1] * 1000,
                    step_3 = LOGIN_BACKOFF_STEPS_SECS[2] * 1000,
                    step_4 = LOGIN_BACKOFF_STEPS_SECS[3] * 1000,
                )
            })
            .as_str()
    }

    async fn run(
        mut connection: redis::aio::ConnectionManager,
        attempt_key: &str,
        mode: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus, redis::RedisError> {
        let key = storage_key(attempt_key);
        // EVALSHA via `redis::Script` (uploads once, then runs by digest with
        // automatic `EVAL` fallback on NOSCRIPT) instead of shipping the whole
        // script on every login attempt.
        let values: Vec<i64> = redis::Script::new(script_source())
            .key(key)
            .arg(mode)
            .arg(now.timestamp_millis())
            .invoke_async(&mut connection)
            .await?;

        let value = |index: usize| values.get(index).copied().unwrap_or(0);
        Ok(LoginThrottleStatus {
            allowed: value(0) != 0,
            retry_after_secs: value(1).max(0) as u64,
            failure_count_10m: value(2).max(0),
            failure_count_1h: value(3).max(0),
            admin_unlock_required: value(4) != 0,
        })
    }

    pub async fn evaluate(
        connection: redis::aio::ConnectionManager,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus, redis::RedisError> {
        run(connection, attempt_key, "evaluate", now).await
    }

    pub async fn record_failure(
        connection: redis::aio::ConnectionManager,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus, redis::RedisError> {
        run(connection, attempt_key, "record", now).await
    }

    pub async fn reserve(
        connection: redis::aio::ConnectionManager,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> Result<LoginThrottleStatus, redis::RedisError> {
        run(connection, attempt_key, "reserve", now).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn in_memory_fallback_blocks_and_clears_on_success() {
        let throttle = LoginThrottle::in_memory();
        assert_eq!(throttle.backend(), LoginThrottleBackend::Memory);
        assert!(!throttle.is_durable());

        let now = Utc::now();
        for offset in 0..10 {
            throttle
                .record_failure("alice|fp", now + Duration::seconds(offset))
                .await;
        }
        let locked = throttle
            .evaluate("alice|fp", now + Duration::seconds(10))
            .await;
        assert!(!locked.allowed);
        assert!(locked.retry_after_secs >= 590);

        throttle.record_success("alice|fp").await;
        let cleared = throttle
            .evaluate("alice|fp", now + Duration::seconds(11))
            .await;
        assert!(cleared.allowed);
        assert_eq!(cleared.failure_count_10m, 0);
    }

    #[tokio::test]
    async fn in_memory_fallback_requires_admin_unlock_after_twenty() {
        let throttle = LoginThrottle::in_memory();
        let now = Utc::now();
        for offset in 0..20 {
            throttle
                .record_failure("bob|fp", now + Duration::minutes(offset))
                .await;
        }
        let locked = throttle
            .evaluate("bob|fp", now + Duration::minutes(20))
            .await;
        assert!(locked.admin_unlock_required);
        assert!(!locked.allowed);
        assert!(
            locked.retry_after_secs > 0,
            "the strongest lock is bounded and reports its remaining time"
        );

        assert!(throttle.clear_lock("bob|fp").await);
        assert!(
            throttle
                .evaluate("bob|fp", now + Duration::minutes(21))
                .await
                .allowed
        );
    }

    /// `reserve` is a single check-and-record operation: the failure is
    /// recorded before the result is returned, so the next attempt already
    /// observes it. The old evaluate-then-record pair let both of these pass.
    #[tokio::test]
    async fn in_memory_reserve_records_before_returning() {
        let throttle = LoginThrottle::in_memory();
        let now = Utc::now();

        let first = throttle.reserve("carol|fp", now).await;
        assert!(first.allowed);
        assert_eq!(first.failure_count_10m, 1);

        let second = throttle.reserve("carol|fp", now).await;
        assert!(
            !second.allowed,
            "the first reservation's failure must already be visible"
        );
        assert_eq!(
            second.failure_count_10m, 1,
            "a blocked reserve must not increment the counters"
        );
    }

    /// Concurrent reservations against one key cannot all pass: exactly one
    /// wins and the rest observe the recorded backoff.
    #[tokio::test]
    async fn concurrent_reservations_are_serialized() {
        let throttle = LoginThrottle::in_memory();
        let now = Utc::now();
        let attempts = (0..20).map(|_| {
            let throttle = throttle.clone();
            async move { throttle.reserve("dave|fp", now).await }
        });
        let results = futures_util::future::join_all(attempts).await;
        let allowed = results.iter().filter(|status| status.allowed).count();
        assert_eq!(
            allowed, 1,
            "exactly one of the concurrent attempts may be reserved"
        );
    }

    #[test]
    fn merge_statuses_prefers_the_most_restrictive_answer() {
        let allowed = AuthThrottleStatus {
            allowed: true,
            retry_after_secs: 0,
            failure_count_10m: 0,
            failure_count_1h: 0,
            admin_unlock_required: false,
        };
        let backoff = AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 2,
            failure_count_10m: 3,
            failure_count_1h: 3,
            admin_unlock_required: false,
        };
        let temp_lock = AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 600,
            failure_count_10m: 10,
            failure_count_1h: 10,
            admin_unlock_required: false,
        };
        let admin_lock = AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 86_400,
            failure_count_10m: 10,
            failure_count_1h: 20,
            admin_unlock_required: true,
        };

        assert!(merge_statuses(Vec::new()).is_none());

        let merged = merge_statuses(vec![allowed, backoff.clone()]).expect("merged");
        assert!(!merged.allowed, "a blocked backend must win over allowed");
        assert_eq!(merged.retry_after_secs, 2);

        let merged = merge_statuses(vec![backoff, temp_lock.clone()]).expect("merged");
        assert_eq!(merged.retry_after_secs, 600);
        assert_eq!(merged.failure_count_10m, 10);

        let merged = merge_statuses(vec![temp_lock, admin_lock]).expect("merged");
        assert!(
            merged.admin_unlock_required,
            "an admin lock is the strongest"
        );
    }
}
