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
//! If the configured Redis backend fails at call time the facade degrades to
//! PostgreSQL (another durable backend) rather than failing open.

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
    pub async fn evaluate(&self, attempt_key: &str, now: DateTime<Utc>) -> AuthThrottleStatus {
        if let Some(redis) = &self.redis {
            match redis_backend::evaluate(redis.clone(), attempt_key, now).await {
                Ok(status) => return status.into(),
                Err(error) => tracing::warn!(
                    %error,
                    "Redis login throttle read failed; falling back to PostgreSQL"
                ),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_evaluate(attempt_key, now).await {
                Ok(status) => return status.into(),
                Err(error) => tracing::warn!(
                    %error,
                    "PostgreSQL login throttle read failed; falling back to in-memory"
                ),
            }
        }
        self.memory.evaluate(attempt_key, now)
    }

    /// Record a failed attempt and return the resulting decision.
    pub async fn record_failure(
        &self,
        attempt_key: &str,
        now: DateTime<Utc>,
    ) -> AuthThrottleStatus {
        if let Some(redis) = &self.redis {
            match redis_backend::record_failure(redis.clone(), attempt_key, now).await {
                Ok(status) => return status.into(),
                Err(error) => tracing::warn!(
                    %error,
                    "Redis login throttle write failed; falling back to PostgreSQL"
                ),
            }
        }
        if let Some(store) = &self.store {
            match store.login_throttle_record_failure(attempt_key, now).await {
                Ok(status) => return status.into(),
                Err(error) => tracing::warn!(
                    %error,
                    "PostgreSQL login throttle write failed; falling back to in-memory"
                ),
            }
        }
        self.memory.record_failure(attempt_key, now)
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

/// Redis backend: the whole read-modify-write runs as one Lua script, so
/// increments are atomic across replicas and the key TTL is refreshed in the
/// same operation.
mod redis_backend {
    use chrono::{DateTime, Utc};

    use apex_store::login_throttle::{
        LoginThrottleStatus, LOGIN_ADMIN_LOCK_THRESHOLD_1H, LOGIN_BACKOFF_STEPS_SECS,
        LOGIN_ROW_TTL_SECS, LOGIN_TEMP_LOCK_SECS, LOGIN_TEMP_LOCK_THRESHOLD_10M,
        LOGIN_WINDOW_10M_SECS, LOGIN_WINDOW_1H_SECS,
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
local ttl_secs = {ttl_secs}
local steps = {{{step_1}, {step_2}, {step_3}, {step_4}}}

local fields = redis.call('HMGET', key, 'f10', 'f1h', 'w10', 'w1h', 'backoff', 'lock', 'admin')
local f10 = tonumber(fields[1]) or 0
local f1h = tonumber(fields[2]) or 0
local w10 = tonumber(fields[3]) or 0
local w1h = tonumber(fields[4]) or 0
local backoff = tonumber(fields[5]) or 0
local lock = tonumber(fields[6]) or 0
local admin = fields[7] == '1'

if w10 > 0 and now - w10 >= window_10m_ms then f10 = 0; w10 = 0 end
if w1h > 0 and now - w1h >= window_1h_ms then f1h = 0; w1h = 0 end
if backoff > 0 and now >= backoff then backoff = 0 end
if lock > 0 and now >= lock then lock = 0 end

if mode == 'record' then
    if f10 == 0 then w10 = now end
    if f1h == 0 then w1h = now end
    f10 = f10 + 1
    f1h = f1h + 1
    if f1h >= admin_threshold then
        admin = true
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
        'backoff', backoff, 'lock', lock, 'admin', admin and '1' or '0')
    if admin then
        redis.call('PERSIST', key)
    else
        redis.call('EXPIRE', key, ttl_secs)
    end
end

local retry = 0
if not admin then
    if lock > now then
        retry = math.ceil((lock - now) / 1000)
    elseif backoff > now then
        retry = math.ceil((backoff - now) / 1000)
    end
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
                    ttl_secs = LOGIN_ROW_TTL_SECS,
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
        // EVAL rather than the gated `script` cargo feature: the script is
        // small, and one round trip keeps the read-modify-write atomic.
        let values: Vec<i64> = redis::cmd("EVAL")
            .arg(script_source())
            .arg(1)
            .arg(key)
            .arg(mode)
            .arg(now.timestamp_millis())
            .query_async(&mut connection)
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

        assert!(throttle.clear_lock("bob|fp").await);
        assert!(
            throttle
                .evaluate("bob|fp", now + Duration::minutes(21))
                .await
                .allowed
        );
    }
}
