//! Authentication — API key and bearer token validation.
//!
//! Pure validation logic. The actual middleware integration is done at binary level.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use subtle::ConstantTimeEq;

// ────────────────────────────────────────────
// API Key management
// ────────────────────────────────────────────

/// An API key with associated permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key_id: String,
    pub owner_user_id: String,
    pub key_hash: String,
    pub name: String,
    pub role: ApiRole,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub rate_limit_per_min: u32,
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiRole {
    Admin,
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

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Self::Admin),
            "analyst" => Some(Self::Analyst),
            "viewer" => Some(Self::Viewer),
            "service" => Some(Self::Service),
            _ => None,
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

// ────────────────────────────────────────────
// Token validation
// ────────────────────────────────────────────

/// Result of validating a bearer token.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthResult {
    Valid {
        key_id: String,
        owner_user_id: String,
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
        if stored_bytes.len() == token_bytes.len() {
            if bool::from(stored_bytes.ct_eq(token_bytes)) {
                matched_key = Some(key);
            }
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
            owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-viewer".to_string(),
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
            owner_user_id: "usr-disabled".to_string(),
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
            owner_user_id: "usr-expired".to_string(),
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
        assert_eq!(ApiRole::from_str("admin"), Some(ApiRole::Admin));
        assert_eq!(ApiRole::from_str("analyst"), Some(ApiRole::Analyst));
        assert_eq!(ApiRole::from_str("viewer"), Some(ApiRole::Viewer));
        assert_eq!(ApiRole::from_str("service"), Some(ApiRole::Service));
        assert_eq!(ApiRole::from_str("unknown"), None);
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
                owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-admin".to_string(),
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
            owner_user_id: "usr-analyst".to_string(),
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
            owner_user_id: "usr-viewer".to_string(),
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
            owner_user_id: "usr-k".to_string(),
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
            owner_user_id: "usr-test".to_string(),
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
