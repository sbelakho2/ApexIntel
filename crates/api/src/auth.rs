//! Authentication — API key and bearer token validation.
//!
//! Pure validation logic. The actual middleware integration is done at binary level.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use apex_core::identity::{UserId, Username};

use parking_lot::Mutex;
use subtle::ConstantTimeEq;

const AUTH_BACKOFF_STEPS_SECS: [i64; 4] = [1, 2, 4, 8];
const TEMP_LOCK_THRESHOLD_10M: usize = 10;
const ADMIN_LOCK_THRESHOLD_1H: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthThrottleStatus {
    pub allowed: bool,
    pub retry_after_secs: u64,
    pub failure_count_10m: usize,
    pub failure_count_1h: usize,
    pub admin_unlock_required: bool,
}

#[derive(Debug, Clone, Default)]
struct AuthAttemptState {
    failures_10m: Vec<DateTime<Utc>>,
    failures_1h: Vec<DateTime<Utc>>,
    backoff_until: Option<DateTime<Utc>>,
    temp_lock_until: Option<DateTime<Utc>>,
    admin_locked: bool,
}

#[derive(Clone, Default)]
pub struct AuthAttemptTracker {
    inner: Arc<Mutex<HashMap<String, AuthAttemptState>>>,
}

impl AuthAttemptTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn evaluate(&self, attempt_key: &str, now: DateTime<Utc>) -> AuthThrottleStatus {
        let mut inner = self.inner.lock();
        let state = inner.entry(attempt_key.to_string()).or_default();
        prune_attempt_state(state, now);
        throttle_status_from_state(state, now)
    }

    pub fn record_failure(&self, attempt_key: &str, now: DateTime<Utc>) -> AuthThrottleStatus {
        let mut inner = self.inner.lock();
        let state = inner.entry(attempt_key.to_string()).or_default();
        prune_attempt_state(state, now);
        state.failures_10m.push(now);
        state.failures_1h.push(now);

        if state.failures_1h.len() >= ADMIN_LOCK_THRESHOLD_1H {
            state.admin_locked = true;
            state.backoff_until = None;
            state.temp_lock_until = None;
            return throttle_status_from_state(state, now);
        }

        if state.failures_10m.len() >= TEMP_LOCK_THRESHOLD_10M {
            state.temp_lock_until = Some(now + chrono::Duration::minutes(10));
            state.backoff_until = None;
            return throttle_status_from_state(state, now);
        }

        let idx = state.failures_10m.len().saturating_sub(1);
        let delay_secs = AUTH_BACKOFF_STEPS_SECS
            .get(idx)
            .copied()
            .unwrap_or(*AUTH_BACKOFF_STEPS_SECS.last().unwrap_or(&8));
        state.backoff_until = Some(now + chrono::Duration::seconds(delay_secs));
        throttle_status_from_state(state, now)
    }

    pub fn record_success(&self, attempt_key: &str) {
        let mut inner = self.inner.lock();
        inner.remove(attempt_key);
    }

    pub fn clear_lock(&self, attempt_key: &str) -> bool {
        let mut inner = self.inner.lock();
        if let Some(state) = inner.get_mut(attempt_key) {
            state.failures_10m.clear();
            state.failures_1h.clear();
            state.backoff_until = None;
            state.temp_lock_until = None;
            state.admin_locked = false;
            return true;
        }
        false
    }
}

fn prune_attempt_state(state: &mut AuthAttemptState, now: DateTime<Utc>) {
    let cutoff_10m = now - chrono::Duration::minutes(10);
    let cutoff_1h = now - chrono::Duration::hours(1);
    state.failures_10m.retain(|ts| *ts >= cutoff_10m);
    state.failures_1h.retain(|ts| *ts >= cutoff_1h);

    if state.backoff_until.is_some_and(|until| now >= until) {
        state.backoff_until = None;
    }
    if state.temp_lock_until.is_some_and(|until| now >= until) {
        state.temp_lock_until = None;
    }
}

fn throttle_status_from_state(state: &AuthAttemptState, now: DateTime<Utc>) -> AuthThrottleStatus {
    let retry_after_secs = if state.admin_locked {
        0
    } else if let Some(until) = state.temp_lock_until {
        (until - now).num_seconds().max(0) as u64
    } else if let Some(until) = state.backoff_until {
        (until - now).num_seconds().max(0) as u64
    } else {
        0
    };

    AuthThrottleStatus {
        allowed: !state.admin_locked
            && state
                .temp_lock_until
                .map(|until| now >= until)
                .unwrap_or(true)
            && state
                .backoff_until
                .map(|until| now >= until)
                .unwrap_or(true),
        retry_after_secs,
        failure_count_10m: state.failures_10m.len(),
        failure_count_1h: state.failures_1h.len(),
        admin_unlock_required: state.admin_locked,
    }
}

pub fn client_fingerprint(ip_hint: Option<&str>, user_agent: Option<&str>) -> String {
    let source = format!(
        "{}|{}",
        ip_hint.unwrap_or("unknown-ip"),
        user_agent.unwrap_or("unknown-ua")
    );
    hash_api_key(&source)
}

// ────────────────────────────────────────────
// API Key management
// ────────────────────────────────────────────

/// An API key with associated permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key_id: String,
    /// Canonical `app_users.id` this key acts as.
    pub owner_user_id: UserId,
    pub key_hash: String,
    pub name: String,
    pub role: ApiRole,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub rate_limit_per_min: u32,
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ApiRole {
    Admin,
    #[default]
    Analyst,
    Viewer,
    Service,
}

impl ApiRole {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Admin => "admin",
            Self::Analyst => "analyst",
            Self::Viewer => "viewer",
            Self::Service => "service",
        }
    }

    /// Can this role access admin endpoints?
    pub fn can_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    /// Can this role write (acknowledge warnings, promote recipes)?
    pub fn can_write(&self) -> bool {
        matches!(self, Self::Admin | Self::Analyst)
    }

    /// Can this role read data?
    pub fn can_read(&self) -> bool {
        true // all roles can read
    }
}

impl FromStr for ApiRole {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "analyst" => Ok(Self::Analyst),
            "viewer" => Ok(Self::Viewer),
            "service" => Ok(Self::Service),
            _ => Err(()),
        }
    }
}

// ────────────────────────────────────────────
// Principal
// ────────────────────────────────────────────

/// Unified authenticated principal — whoever a request acts as, independent of
/// the transport that authenticated it (API key or browser session).
///
/// Browser sessions are signed with this data in the cookie payload
/// (`uid`/`sub`/`role`/`iat`/`exp`/`sv`); API keys map onto it via
/// [`AuthResult`]. `session_version` starts at
/// [`crate::middleware::session::SESSION_VERSION`] and exists so sessions can
/// be invalidated by bumping the version in the future.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// Canonical `app_users.id` used for ownership.
    pub user_id: UserId,
    /// Login/display name; never an ownership key.
    pub username: Username,
    pub role: ApiRole,
    pub session_version: u32,
}

impl Principal {
    pub fn new(
        user_id: impl Into<UserId>,
        username: impl Into<Username>,
        role: ApiRole,
        session_version: u32,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            username: username.into(),
            role,
            session_version,
        }
    }

    /// Can this principal access admin-only surfaces?
    pub fn can_admin(&self) -> bool {
        self.role.can_admin()
    }
}

// ────────────────────────────────────────────
// Token validation
// ────────────────────────────────────────────

/// Result of validating a bearer token.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthResult {
    Valid {
        key_id: String,
        owner_user_id: UserId,
        role: ApiRole,
    },
    Expired {
        key_id: String,
    },
    Disabled {
        key_id: String,
    },
    InvalidKey,
    MissingHeader,
}

impl AuthResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    pub fn role(&self) -> Option<&ApiRole> {
        match self {
            Self::Valid { role, .. } => Some(role),
            _ => None,
        }
    }
}

/// Hash an API key (simple SHA-256 for key lookup).
pub fn hash_api_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

/// Extract bearer token from an Authorization header value.
pub fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    if let Some(token) = trimmed.strip_prefix("Bearer ") {
        let token = token.trim();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    } else {
        None
    }
}

/// Validate a token against a key registry.
/// Uses constant-time comparison to prevent timing side-channel attacks.
pub fn validate_token(
    token: &str,
    registry: &HashMap<String, ApiKey>,
    now: DateTime<Utc>,
) -> AuthResult {
    let token_hash = hash_api_key(token);
    let token_bytes = token_hash.as_bytes();

    // Constant-time scan: always iterate all keys, no early exit
    let mut matched_key: Option<&ApiKey> = None;
    for key in registry.values() {
        let stored_bytes = key.key_hash.as_bytes();
        if stored_bytes.len() == token_bytes.len() && bool::from(stored_bytes.ct_eq(token_bytes)) {
            matched_key = Some(key);
        }
    }

    let key = match matched_key {
        Some(k) => k,
        None => return AuthResult::InvalidKey,
    };

    if !key.enabled {
        return AuthResult::Disabled {
            key_id: key.key_id.clone(),
        };
    }

    if let Some(expires) = key.expires_at {
        if now > expires {
            return AuthResult::Expired {
                key_id: key.key_id.clone(),
            };
        }
    }

    AuthResult::Valid {
        key_id: key.key_id.clone(),
        owner_user_id: key.owner_user_id.clone(),
        role: key.role.clone(),
    }
}

/// Check if an origin is allowed for a given API key.
pub fn check_origin(key: &ApiKey, origin: &str) -> bool {
    if key.allowed_origins.is_empty() {
        return true; // no restriction
    }
    key.allowed_origins
        .iter()
        .any(|o| if o == "*" { true } else { o == origin })
}

// ────────────────────────────────────────────
// Permission check helpers
// ────────────────────────────────────────────

/// Check if the authenticated role is allowed for an endpoint.
pub fn check_permission(auth: &AuthResult, required: PermissionLevel) -> bool {
    match auth {
        AuthResult::Valid { role, .. } => match required {
            PermissionLevel::Read => role.can_read(),
            PermissionLevel::Write => role.can_write(),
            PermissionLevel::Admin => role.can_admin(),
        },
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PermissionLevel {
    Read,
    Write,
    Admin,
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> HashMap<String, ApiKey> {
        let mut reg = HashMap::new();

        let admin_key = ApiKey {
            key_id: "k1".to_string(),
            owner_user_id: "usr-admin".into(),
            key_hash: hash_api_key("admin-secret-key"),
            name: "Admin Key".to_string(),
            role: ApiRole::Admin,
            created_at: Utc::now(),
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 100,
            allowed_origins: vec![],
        };
        reg.insert("k1".to_string(), admin_key);

        let viewer_key = ApiKey {
            key_id: "k2".to_string(),
            owner_user_id: "usr-viewer".into(),
            key_hash: hash_api_key("viewer-key"),
            name: "Viewer Key".to_string(),
            role: ApiRole::Viewer,
            created_at: Utc::now(),
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            enabled: true,
            rate_limit_per_min: 30,
            allowed_origins: vec!["https://app.starz.com".to_string()],
        };
        reg.insert("k2".to_string(), viewer_key);

        let disabled_key = ApiKey {
            key_id: "k3".to_string(),
            owner_user_id: "usr-disabled".into(),
            key_hash: hash_api_key("disabled-key"),
            name: "Disabled Key".to_string(),
            role: ApiRole::Analyst,
            created_at: Utc::now(),
            expires_at: None,
            enabled: false,
            rate_limit_per_min: 50,
            allowed_origins: vec![],
        };
        reg.insert("k3".to_string(), disabled_key);

        let expired_key = ApiKey {
            key_id: "k4".to_string(),
            owner_user_id: "usr-expired".into(),
            key_hash: hash_api_key("expired-key"),
            name: "Expired Key".to_string(),
            role: ApiRole::Analyst,
            created_at: Utc::now() - chrono::Duration::days(60),
            expires_at: Some(Utc::now() - chrono::Duration::days(1)),
            enabled: true,
            rate_limit_per_min: 50,
            allowed_origins: vec![],
        };
        reg.insert("k4".to_string(), expired_key);

        reg
    }

    // ── ApiRole ──

    #[test]
    fn test_role_as_str() {
        assert_eq!(ApiRole::Admin.as_str(), "admin");
        assert_eq!(ApiRole::Analyst.as_str(), "analyst");
        assert_eq!(ApiRole::Viewer.as_str(), "viewer");
        assert_eq!(ApiRole::Service.as_str(), "service");
    }

    #[test]
    fn test_role_from_str() {
        assert_eq!("admin".parse::<ApiRole>().ok(), Some(ApiRole::Admin));
        assert_eq!("analyst".parse::<ApiRole>().ok(), Some(ApiRole::Analyst));
        assert_eq!("viewer".parse::<ApiRole>().ok(), Some(ApiRole::Viewer));
        assert_eq!("service".parse::<ApiRole>().ok(), Some(ApiRole::Service));
        assert_eq!("unknown".parse::<ApiRole>().ok(), None);
    }

    #[test]
    fn test_role_permissions() {
        assert!(ApiRole::Admin.can_admin());
        assert!(ApiRole::Admin.can_write());
        assert!(ApiRole::Admin.can_read());

        assert!(!ApiRole::Analyst.can_admin());
        assert!(ApiRole::Analyst.can_write());
        assert!(ApiRole::Analyst.can_read());

        assert!(!ApiRole::Viewer.can_admin());
        assert!(!ApiRole::Viewer.can_write());
        assert!(ApiRole::Viewer.can_read());

        assert!(!ApiRole::Service.can_admin());
        assert!(!ApiRole::Service.can_write());
        assert!(ApiRole::Service.can_read());
    }

    #[test]
    fn auth_attempt_tracker_applies_progressive_backoff() {
        let tracker = AuthAttemptTracker::new();
        let now = Utc::now();
        let first = tracker.record_failure("key-hash", now);
        assert!(!first.allowed);
        assert_eq!(first.retry_after_secs, 1);

        let second = tracker.record_failure("key-hash", now + chrono::Duration::seconds(1));
        assert!(!second.allowed);
        assert_eq!(second.retry_after_secs, 2);

        let later = tracker.evaluate("key-hash", now + chrono::Duration::seconds(4));
        assert!(later.allowed);
    }

    #[test]
    fn auth_attempt_tracker_temp_locks_after_ten_failures() {
        let tracker = AuthAttemptTracker::new();
        let now = Utc::now();
        let mut last = AuthThrottleStatus {
            allowed: true,
            retry_after_secs: 0,
            failure_count_10m: 0,
            failure_count_1h: 0,
            admin_unlock_required: false,
        };
        for idx in 0..10 {
            last = tracker.record_failure("key-hash", now + chrono::Duration::seconds(idx));
        }
        assert!(!last.allowed);
        assert!(last.retry_after_secs >= 599);
        assert_eq!(last.failure_count_10m, 10);
    }

    #[test]
    fn auth_attempt_tracker_requires_admin_unlock_after_twenty_failures() {
        let tracker = AuthAttemptTracker::new();
        let now = Utc::now();
        let mut last = AuthThrottleStatus {
            allowed: true,
            retry_after_secs: 0,
            failure_count_10m: 0,
            failure_count_1h: 0,
            admin_unlock_required: false,
        };
        for idx in 0..20 {
            last = tracker.record_failure("key-hash", now + chrono::Duration::minutes(idx));
        }
        assert!(!last.allowed);
        assert!(last.admin_unlock_required);
        assert!(tracker.clear_lock("key-hash"));
        assert!(
            tracker
                .evaluate("key-hash", now + chrono::Duration::hours(2))
                .allowed
        );
    }

    #[test]
    fn auth_progressive_backoff() {
        let tracker = AuthAttemptTracker::new();
        let now = Utc::now();
        let first = tracker.record_failure("registry-key", now);
        for index in 1..10 {
            tracker.record_failure(
                "registry-key",
                now + chrono::Duration::seconds(index as i64 * 10),
            );
        }
        let eleventh = tracker.record_failure("registry-key", now + chrono::Duration::seconds(110));

        assert_eq!(first.retry_after_secs, 1);
        assert!(eleventh.retry_after_secs > first.retry_after_secs * 8);
    }

    #[test]
    fn auth_lockout_after_threshold() {
        let tracker = AuthAttemptTracker::new();
        let now = Utc::now();
        let mut status = tracker.evaluate("registry-lock", now);
        for index in 0..20 {
            status = tracker.record_failure(
                "registry-lock",
                now + chrono::Duration::minutes(index as i64),
            );
        }

        assert!(!status.allowed);
        assert!(status.admin_unlock_required);
        assert_ne!(status.retry_after_secs, 401);
    }

    #[test]
    fn client_fingerprint_changes_with_inputs() {
        let a = client_fingerprint(Some("1.2.3.4"), Some("ua-a"));
        let b = client_fingerprint(Some("1.2.3.4"), Some("ua-b"));
        assert_ne!(a, b);
    }

    // ── Token extraction ──

    #[test]
    fn test_extract_bearer_token_valid() {
        assert_eq!(
            extract_bearer_token("Bearer my-secret-token"),
            Some("my-secret-token")
        );
    }

    #[test]
    fn test_extract_bearer_token_trimmed() {
        assert_eq!(
            extract_bearer_token("  Bearer   my-token  "),
            Some("my-token")
        );
    }

    #[test]
    fn test_extract_bearer_token_no_prefix() {
        assert_eq!(extract_bearer_token("my-token"), None);
    }

    #[test]
    fn test_extract_bearer_token_empty() {
        assert_eq!(extract_bearer_token("Bearer "), None);
        assert_eq!(extract_bearer_token(""), None);
    }

    #[test]
    fn test_extract_bearer_token_basic_auth() {
        assert_eq!(extract_bearer_token("Basic dXNlcjpwYXNz"), None);
    }

    // ── Key hashing ──

    #[test]
    fn test_hash_deterministic() {
        let h1 = hash_api_key("test-key");
        let h2 = hash_api_key("test-key");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_different_keys() {
        let h1 = hash_api_key("key-a");
        let h2 = hash_api_key("key-b");
        assert_ne!(h1, h2);
    }

    // ── Token validation ──

    #[test]
    fn test_validate_admin_token() {
        let reg = make_registry();
        let result = validate_token("admin-secret-key", &reg, Utc::now());
        assert_eq!(
            result,
            AuthResult::Valid {
                key_id: "k1".to_string(),
                owner_user_id: "usr-admin".into(),
                role: ApiRole::Admin,
            }
        );
    }

    #[test]
    fn test_validate_viewer_token() {
        let reg = make_registry();
        let result = validate_token("viewer-key", &reg, Utc::now());
        assert!(result.is_valid());
        assert_eq!(result.role(), Some(&ApiRole::Viewer));
    }

    #[test]
    fn test_validate_unknown_token() {
        let reg = make_registry();
        let result = validate_token("unknown-key", &reg, Utc::now());
        assert_eq!(result, AuthResult::InvalidKey);
    }

    #[test]
    fn test_validate_disabled_token() {
        let reg = make_registry();
        let result = validate_token("disabled-key", &reg, Utc::now());
        assert_eq!(
            result,
            AuthResult::Disabled {
                key_id: "k3".to_string()
            }
        );
    }

    #[test]
    fn test_validate_expired_token() {
        let reg = make_registry();
        let result = validate_token("expired-key", &reg, Utc::now());
        assert_eq!(
            result,
            AuthResult::Expired {
                key_id: "k4".to_string()
            }
        );
    }

    // ── Origin checks ──

    #[test]
    fn test_check_origin_no_restriction() {
        let key = ApiKey {
            key_id: "k1".to_string(),
            owner_user_id: "usr-admin".into(),
            key_hash: String::new(),
            name: String::new(),
            role: ApiRole::Admin,
            created_at: Utc::now(),
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 100,
            allowed_origins: vec![], // no restriction
        };
        assert!(check_origin(&key, "https://anything.com"));
    }

    #[test]
    fn test_check_origin_wildcard() {
        let key = ApiKey {
            key_id: "k1".to_string(),
            owner_user_id: "usr-admin".into(),
            key_hash: String::new(),
            name: String::new(),
            role: ApiRole::Admin,
            created_at: Utc::now(),
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 100,
            allowed_origins: vec!["*".to_string()],
        };
        assert!(check_origin(&key, "https://anything.com"));
    }

    #[test]
    fn test_check_origin_matched() {
        let key = ApiKey {
            key_id: "k1".to_string(),
            owner_user_id: "usr-admin".into(),
            key_hash: String::new(),
            name: String::new(),
            role: ApiRole::Admin,
            created_at: Utc::now(),
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 100,
            allowed_origins: vec!["https://app.starz.com".to_string()],
        };
        assert!(check_origin(&key, "https://app.starz.com"));
        assert!(!check_origin(&key, "https://evil.com"));
    }

    // ── Permission checks ──

    #[test]
    fn test_check_permission_admin() {
        let auth = AuthResult::Valid {
            key_id: "k1".to_string(),
            owner_user_id: "usr-admin".into(),
            role: ApiRole::Admin,
        };
        assert!(check_permission(&auth, PermissionLevel::Admin));
        assert!(check_permission(&auth, PermissionLevel::Write));
        assert!(check_permission(&auth, PermissionLevel::Read));
    }

    #[test]
    fn test_check_permission_analyst() {
        let auth = AuthResult::Valid {
            key_id: "k2".to_string(),
            owner_user_id: "usr-analyst".into(),
            role: ApiRole::Analyst,
        };
        assert!(!check_permission(&auth, PermissionLevel::Admin));
        assert!(check_permission(&auth, PermissionLevel::Write));
        assert!(check_permission(&auth, PermissionLevel::Read));
    }

    #[test]
    fn test_check_permission_viewer() {
        let auth = AuthResult::Valid {
            key_id: "k3".to_string(),
            owner_user_id: "usr-viewer".into(),
            role: ApiRole::Viewer,
        };
        assert!(!check_permission(&auth, PermissionLevel::Admin));
        assert!(!check_permission(&auth, PermissionLevel::Write));
        assert!(check_permission(&auth, PermissionLevel::Read));
    }

    #[test]
    fn test_check_permission_invalid() {
        let auth = AuthResult::InvalidKey;
        assert!(!check_permission(&auth, PermissionLevel::Read));
    }

    #[test]
    fn test_check_permission_expired() {
        let auth = AuthResult::Expired {
            key_id: "k4".to_string(),
        };
        assert!(!check_permission(&auth, PermissionLevel::Read));
    }

    // ── AuthResult ──

    #[test]
    fn test_auth_result_is_valid() {
        assert!(AuthResult::Valid {
            key_id: "k".to_string(),
            owner_user_id: "usr-k".into(),
            role: ApiRole::Admin
        }
        .is_valid());
        assert!(!AuthResult::InvalidKey.is_valid());
        assert!(!AuthResult::MissingHeader.is_valid());
    }

    // ── Serialization ──

    #[test]
    fn test_api_key_serialization() {
        let key = ApiKey {
            key_id: "test".to_string(),
            owner_user_id: "usr-test".into(),
            key_hash: "abc".to_string(),
            name: "Test Key".to_string(),
            role: ApiRole::Analyst,
            created_at: Utc::now(),
            expires_at: None,
            enabled: true,
            rate_limit_per_min: 50,
            allowed_origins: vec!["https://app.example.com".to_string()],
        };
        let json = serde_json::to_string(&key).unwrap();
        let back: ApiKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key_id, "test");
        assert_eq!(back.role, ApiRole::Analyst);
    }
}
