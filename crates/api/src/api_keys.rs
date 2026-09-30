use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use apex_core::identity::UserId;
use chrono::Utc;
use serde::Deserialize;

use crate::auth::{self, ApiKey, ApiRole};
use crate::config::ApiKeysConfig;

#[derive(Debug, Clone)]
pub enum ApiKeySource {
    Env { slots: usize },
    File { path: PathBuf, slots: usize },
}

#[derive(Debug)]
pub struct ApiKeyManager {
    source: ApiKeySource,
    keys: RwLock<Arc<HashMap<String, ApiKey>>>,
    modified_at: RwLock<Option<SystemTime>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiKeyFileRecord {
    raw_key: String,
    name: String,
    role: String,
    /// Canonical `app_users.id` this key acts as. Defaults to a stable
    /// per-slot `file-user-N` principal when the file does not carry one.
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    rate_limit_per_min: Option<u32>,
    #[serde(default)]
    allowed_origins: Vec<String>,
}

impl ApiKeyManager {
    pub fn new(config: &ApiKeysConfig) -> Result<Self> {
        let source = if let Some(path) = config.file_path.clone() {
            ApiKeySource::File {
                path,
                slots: config.env_slots,
            }
        } else {
            ApiKeySource::Env {
                slots: config.env_slots,
            }
        };

        let (keys, modified_at) = load_from_source(&source)?;

        Ok(Self {
            source,
            keys: RwLock::new(Arc::new(keys)),
            modified_at: RwLock::new(modified_at),
        })
    }

    pub fn snapshot(&self) -> Arc<HashMap<String, ApiKey>> {
        self.keys
            .read()
            .unwrap_or_else(|poisoned| {
                tracing::error!("api key snapshot lock poisoned, recovering");
                poisoned.into_inner()
            })
            .clone()
    }

    pub fn key_count(&self) -> usize {
        self.snapshot().len()
    }

    pub fn reload(&self) -> Result<bool> {
        if let ApiKeySource::File { path, .. } = &self.source {
            let next_modified = file_modified_at(path)?;
            let current_modified = *self.modified_at.read().unwrap_or_else(|poisoned| {
                tracing::error!("api key modified lock poisoned, recovering");
                poisoned.into_inner()
            });
            if next_modified == current_modified {
                return Ok(false);
            }
        }

        let (keys, modified_at) = load_from_source(&self.source)?;
        *self.keys.write().unwrap_or_else(|poisoned| {
            tracing::error!("api key update lock poisoned, recovering");
            poisoned.into_inner()
        }) = Arc::new(keys);
        *self.modified_at.write().unwrap_or_else(|poisoned| {
            tracing::error!("api key modified update lock poisoned, recovering");
            poisoned.into_inner()
        }) = modified_at;
        Ok(true)
    }
}

pub fn spawn_api_key_reloader(manager: Arc<ApiKeyManager>, interval: Duration) {
    spawn_api_key_reloader_with_provisioning(manager, interval, None);
}

/// Reload keys on an interval and, when a store is supplied, provision the
/// canonical `app_users` rows for any principals introduced by the new
/// snapshot. Without this, a hot-reloaded key owner would fail the
/// `app_users(id)` foreign keys on user-owned tables.
pub fn spawn_api_key_reloader_with_provisioning(
    manager: Arc<ApiKeyManager>,
    interval: Duration,
    store: Option<Arc<apex_store::postgres::PgStore>>,
) {
    if interval.is_zero() {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match manager.reload() {
                Ok(true) => {
                    tracing::info!(api_key_count = manager.key_count(), "API keys reloaded");
                    if let Some(store) = &store {
                        ensure_api_key_principals(store, &manager.snapshot()).await;
                    }
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "API key reload failed; keeping previous snapshot")
                }
            }
        }
    });
}

/// Provision every API-key owner in the canonical `app_users` identity table
/// (migration 059). API-key principals never log in, but user-owned tables
/// carry an `app_users(id)` foreign key. Insert-only: an existing verified
/// identity is never overwritten with the key's configured role.
pub async fn ensure_api_key_principals(
    store: &apex_store::postgres::PgStore,
    keys: &HashMap<String, ApiKey>,
) {
    for key in keys.values() {
        if let Err(err) = store
            .ensure_app_user_exists(
                key.owner_user_id.as_str(),
                key.owner_user_id.as_str(),
                key.role.as_str(),
            )
            .await
        {
            tracing::warn!(
                key_id = %key.key_id,
                owner_user_id = %key.owner_user_id,
                "failed to provision app_users identity for API-key owner: {err:#}"
            );
        }
    }
}

fn load_from_source(
    source: &ApiKeySource,
) -> Result<(HashMap<String, ApiKey>, Option<SystemTime>)> {
    match source {
        ApiKeySource::Env { slots } => Ok((load_api_keys_from_env(*slots)?, None)),
        ApiKeySource::File { path, slots } => {
            let keys = load_api_keys_from_file(path, *slots)?;
            let modified = file_modified_at(path)?;
            Ok((keys, modified))
        }
    }
}

fn file_modified_at(path: &Path) -> Result<Option<SystemTime>> {
    Ok(fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok()))
}

/// Parse the comma-separated `API_KEY_<n>` slots.
///
/// A malformed slot or an unknown role is a configuration error that names the
/// offending variable and value: roles are security-relevant, so an invalid
/// role must never be silently rewritten to a different privilege level.
pub fn load_api_keys_from_env(slots: usize) -> Result<HashMap<String, ApiKey>> {
    let mut registry = HashMap::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();
    for i in 1..=slots {
        let env_key = format!("API_KEY_{}", i);
        let Ok(val) = std::env::var(&env_key) else {
            continue;
        };
        if val.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = val.splitn(4, ',').collect();
        if parts.len() < 3 {
            // Never echo `val`: it contains the raw key material and this
            // error is written to startup logs.
            anyhow::bail!(
                "{env_key}: invalid API key format (expected raw_key,name,role[,user_id]); found {} field(s)",
                parts.len()
            );
        }
        let raw_key = parts[0].trim();
        if raw_key.is_empty() {
            anyhow::bail!("{env_key}: raw_key must not be empty");
        }
        if raw_key.len() < 32 {
            anyhow::bail!(
                "{env_key}: raw_key must be at least 32 characters (generate with `openssl rand -hex 32`)"
            );
        }
        let name = parts[1].trim();
        let role = parts[2].trim().parse::<ApiRole>().map_err(|error| {
            anyhow::anyhow!("{env_key}: invalid role '{}': {error}", parts[2].trim())
        })?;
        let owner_user_id = parts
            .get(3)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(UserId::from)
            .unwrap_or_else(|| UserId::new(format!("user-{}", i)));
        let key_id = format!("key-{}", i);
        let key_hash = auth::hash_api_key(raw_key);
        if !seen_hashes.insert(key_hash.clone()) {
            // With a duplicate hash, `validate_token` would pick a role based
            // on HashMap iteration order.
            anyhow::bail!(
                "{env_key}: duplicate API key material (already present in an earlier slot)"
            );
        }
        // Keyed by key hash so `validate_token` is O(1) rather than a scan
        // over every key on every request.
        registry.insert(
            key_hash.clone(),
            ApiKey {
                key_id,
                owner_user_id,
                key_hash,
                name: name.to_string(),
                role,
                created_at: Utc::now(),
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 120,
                allowed_origins: Vec::new(),
            },
        );
    }
    Ok(registry)
}

pub fn load_api_keys_from_file(path: &Path, slots: usize) -> Result<HashMap<String, ApiKey>> {
    let raw = fs::read_to_string(path)?;
    let records: Vec<ApiKeyFileRecord> = serde_json::from_str(&raw)
        .map_err(|error| anyhow::anyhow!("{}: invalid API key file: {error}", path.display()))?;
    let mut registry = HashMap::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();

    for (index, record) in records.into_iter().take(slots).enumerate() {
        let key_id = format!("file-key-{}", index + 1);
        let owner_user_id = record
            .user_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(UserId::from)
            .unwrap_or_else(|| UserId::new(format!("file-user-{}", index + 1)));
        let role = record.role.trim().parse::<ApiRole>().map_err(|error| {
            anyhow::anyhow!(
                "{}: entry #{} (name '{}'): invalid role '{}': {error}",
                path.display(),
                index + 1,
                record.name.trim(),
                record.role.trim()
            )
        })?;
        if record.raw_key.trim().is_empty() {
            anyhow::bail!(
                "{}: entry #{} (name '{}'): raw_key must not be empty",
                path.display(),
                index + 1,
                record.name.trim()
            );
        }
        if record.raw_key.trim().len() < 32 {
            anyhow::bail!(
                "{}: entry #{} (name '{}'): raw_key must be at least 32 characters \
                 (generate with `openssl rand -hex 32`)",
                path.display(),
                index + 1,
                record.name.trim()
            );
        }
        let key_hash = auth::hash_api_key(record.raw_key.trim());
        if !seen_hashes.insert(key_hash.clone()) {
            anyhow::bail!(
                "{}: entry #{} (name '{}'): duplicate API key material",
                path.display(),
                index + 1,
                record.name.trim()
            );
        }
        // Keyed by key hash (O(1) `validate_token`).
        registry.insert(
            key_hash.clone(),
            ApiKey {
                key_id,
                owner_user_id,
                key_hash,
                name: record.name.trim().to_string(),
                role,
                created_at: Utc::now(),
                expires_at: None,
                enabled: true,
                rate_limit_per_min: record.rate_limit_per_min.unwrap_or(120),
                allowed_origins: record.allowed_origins,
            },
        );
    }

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("apex-api-{}-{}.json", name, uuid::Uuid::new_v4()));
        fs::write(&path, content).expect("write temp api key file");
        path
    }

    /// Serialises tests that mutate `API_KEY_*` in the process environment.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn short_api_keys_are_rejected() {
        let path = temp_file(
            "api-keys-short",
            r#"[{"raw_key":"short","name":"Short","role":"admin"}]"#,
        );
        assert!(
            load_api_keys_from_file(&path, 50).is_err(),
            "keys below the 32-character floor must be rejected"
        );
    }

    #[test]
    fn duplicate_api_key_material_is_rejected() {
        // Duplicate hashes make `validate_token` pick a role by HashMap
        // iteration order, so they are a configuration error.
        let key = "duplicate-key-0123456789abcdef012345";
        let path = temp_file(
            "api-keys-duplicate",
            &format!(
                r#"[{{"raw_key":"{key}","name":"A","role":"admin"}},{{"raw_key":"{key}","name":"B","role":"viewer"}}]"#
            ),
        );
        assert!(load_api_keys_from_file(&path, 50).is_err());
    }

    #[test]
    fn api_key_file_records_carry_a_canonical_user_id() {
        let path = temp_file(
            "api-keys-user-id",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"analyst","user_id":"usr-42"},
                {"raw_key":"beta-key-0123456789abcdef012345678","name":"Beta","role":"viewer"}]"#,
        );
        let keys = load_api_keys_from_file(&path, 50).expect("load keys");

        let alpha = keys
            .values()
            .find(|key| key.name == "Alpha")
            .expect("alpha key");
        assert_eq!(alpha.owner_user_id.as_str(), "usr-42");

        // A record without `user_id` keeps a stable per-slot principal so the
        // app_users foreign key still has a canonical target.
        let beta = keys
            .values()
            .find(|key| key.name == "Beta")
            .expect("beta key");
        assert_eq!(beta.owner_user_id.as_str(), "file-user-2");
    }

    #[test]
    fn api_keys_reload_after_file_change() {
        let path = temp_file(
            "api-keys-reload",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"admin"}]"#,
        );
        let config = ApiKeysConfig {
            file_path: Some(path.clone()),
            reload_interval_secs: 1,
            env_slots: 50,
        };
        let manager = ApiKeyManager::new(&config).expect("manager");
        assert_eq!(manager.key_count(), 1);

        std::thread::sleep(Duration::from_millis(10));
        fs::write(
            &path,
            r#"[{"raw_key":"beta-key-0123456789abcdef012345678","name":"Beta","role":"viewer"},{"raw_key":"gamma-key-0123456789abcdef0123456","name":"Gamma","role":"admin"}]"#,
        )
        .expect("update api key file");

        assert!(manager.reload().expect("reload should succeed"));
        assert_eq!(manager.key_count(), 2);
    }

    #[test]
    fn api_keys_keep_old_snapshot_when_reload_fails() {
        let path = temp_file(
            "api-keys-fail",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"admin"}]"#,
        );
        let config = ApiKeysConfig {
            file_path: Some(path.clone()),
            reload_interval_secs: 1,
            env_slots: 50,
        };
        let manager = ApiKeyManager::new(&config).expect("manager");
        let original = manager.snapshot();

        std::thread::sleep(Duration::from_millis(10));
        fs::write(&path, "not-json").expect("break api key file");
        assert!(manager.reload().is_err());
        assert_eq!(manager.snapshot().len(), original.len());
    }

    #[test]
    fn invalid_env_role_fails_loudly_and_names_the_variable() {
        let _guard = env_lock();
        std::env::set_var(
            "API_KEY_50",
            "secret-key-0123456789abcdef01234567,Ops Key,superuser,user-50",
        );
        let error = load_api_keys_from_env(50).expect_err("an invalid env role must fail loudly");
        std::env::remove_var("API_KEY_50");

        let message = error.to_string();
        assert!(
            message.contains("API_KEY_50"),
            "the error must name the variable: {message}"
        );
        assert!(
            message.contains("superuser"),
            "the error must name the bad role: {message}"
        );
        assert!(
            message.contains("admin, analyst, viewer, service"),
            "the error must list the valid roles: {message}"
        );
    }

    #[test]
    fn invalid_env_format_fails_loudly_without_echoing_key_material() {
        let _guard = env_lock();
        std::env::set_var("API_KEY_49", "only-a-key");
        let error = load_api_keys_from_env(50).expect_err("a malformed slot must fail loudly");
        std::env::remove_var("API_KEY_49");

        let message = error.to_string();
        assert!(message.contains("API_KEY_49"));
        assert!(
            !message.contains("only-a-key"),
            "the error must not echo the raw key material: {message}"
        );
    }

    #[test]
    fn invalid_file_role_fails_with_file_and_entry_context() {
        let path = temp_file(
            "api-keys-bad-role",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"root"}]"#,
        );
        let error = load_api_keys_from_file(&path, 50).expect_err("an invalid role must fail");
        let message = error.to_string();

        assert!(message.contains("entry #1"), "{message}");
        assert!(message.contains("Alpha"), "{message}");
        assert!(message.contains("root"), "{message}");
    }

    #[test]
    fn api_keys_reload_keeps_previous_snapshot_when_new_role_is_invalid() {
        let path = temp_file(
            "api-keys-bad-role-reload",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"admin"}]"#,
        );
        let config = ApiKeysConfig {
            file_path: Some(path.clone()),
            reload_interval_secs: 1,
            env_slots: 50,
        };
        let manager = ApiKeyManager::new(&config).expect("manager");
        let original = manager.snapshot();
        assert!(original.values().any(|key| key.role == ApiRole::Admin));

        std::thread::sleep(Duration::from_millis(10));
        fs::write(
            &path,
            r#"[{"raw_key":"beta-key-0123456789abcdef012345678","name":"Beta","role":"root"}]"#,
        )
        .expect("write malformed role");

        assert!(
            manager.reload().is_err(),
            "a malformed role must be a reload error"
        );
        let kept = manager.snapshot();
        assert_eq!(kept.len(), original.len());
        assert!(
            kept.values().any(|key| key.role == ApiRole::Admin),
            "the previous valid snapshot must remain active"
        );
    }

    #[test]
    fn revoked_key_is_rejected_after_reload() {
        let path = temp_file(
            "api-keys-revoke",
            r#"[{"raw_key":"alpha-key-0123456789abcdef01234567","name":"Alpha","role":"admin"}]"#,
        );
        let config = ApiKeysConfig {
            file_path: Some(path.clone()),
            reload_interval_secs: 1,
            env_slots: 50,
        };
        let manager = ApiKeyManager::new(&config).expect("manager");
        let snapshot = manager.snapshot();
        assert!(snapshot
            .values()
            .any(|key| auth::hash_api_key("alpha-key-0123456789abcdef01234567") == key.key_hash));

        std::thread::sleep(Duration::from_millis(10));
        fs::write(&path, r#"[]"#).expect("revoke all api keys");
        assert!(manager.reload().expect("reload should succeed"));
        let next = manager.snapshot();
        assert!(!next
            .values()
            .any(|key| auth::hash_api_key("alpha-key-0123456789abcdef01234567") == key.key_hash));
    }
}
