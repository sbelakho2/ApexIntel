use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::Result;
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
    if interval.is_zero() {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match manager.reload() {
                Ok(true) => {
                    tracing::info!(api_key_count = manager.key_count(), "API keys reloaded")
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "API key reload failed; keeping previous snapshot")
                }
            }
        }
    });
}

fn load_from_source(
    source: &ApiKeySource,
) -> Result<(HashMap<String, ApiKey>, Option<SystemTime>)> {
    match source {
        ApiKeySource::Env { slots } => Ok((load_api_keys_from_env(*slots), None)),
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

pub fn load_api_keys_from_env(slots: usize) -> HashMap<String, ApiKey> {
    let mut registry = HashMap::new();
    for i in 1..=slots {
        let env_key = format!("API_KEY_{}", i);
        if let Ok(val) = std::env::var(&env_key) {
            let parts: Vec<&str> = val.splitn(3, ',').collect();
            if parts.len() < 3 {
                tracing::warn!(
                    env_key,
                    "Invalid API key format; expected raw_key,name,role"
                );
                continue;
            }
            let raw_key = parts[0].trim();
            let name = parts[1].trim();
            let role = parts[2]
                .trim()
                .parse::<ApiRole>()
                .unwrap_or(ApiRole::Viewer);
            let key_id = format!("key-{}", i);
            registry.insert(
                key_id.clone(),
                ApiKey {
                    key_id,
                    owner_user_id: format!("user-{}", i),
                    key_hash: auth::hash_api_key(raw_key),
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
    }
    registry
}

pub fn load_api_keys_from_file(path: &Path, slots: usize) -> Result<HashMap<String, ApiKey>> {
    let raw = fs::read_to_string(path)?;
    let records: Vec<ApiKeyFileRecord> = serde_json::from_str(&raw)?;
    let mut registry = HashMap::new();

    for (index, record) in records.into_iter().take(slots).enumerate() {
        let key_id = format!("file-key-{}", index + 1);
        registry.insert(
            key_id.clone(),
            ApiKey {
                key_id,
                owner_user_id: format!("file-user-{}", index + 1),
                key_hash: auth::hash_api_key(record.raw_key.trim()),
                name: record.name.trim().to_string(),
                role: record
                    .role
                    .trim()
                    .parse::<ApiRole>()
                    .unwrap_or(ApiRole::Viewer),
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

    #[test]
    fn api_keys_reload_after_file_change() {
        let path = temp_file(
            "api-keys-reload",
            r#"[{"raw_key":"alpha","name":"Alpha","role":"admin"}]"#,
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
            r#"[{"raw_key":"beta","name":"Beta","role":"viewer"},{"raw_key":"gamma","name":"Gamma","role":"admin"}]"#,
        )
        .expect("update api key file");

        assert!(manager.reload().expect("reload should succeed"));
        assert_eq!(manager.key_count(), 2);
    }

    #[test]
    fn api_keys_keep_old_snapshot_when_reload_fails() {
        let path = temp_file(
            "api-keys-fail",
            r#"[{"raw_key":"alpha","name":"Alpha","role":"admin"}]"#,
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
    fn revoked_key_is_rejected_after_reload() {
        let path = temp_file(
            "api-keys-revoke",
            r#"[{"raw_key":"alpha","name":"Alpha","role":"admin"}]"#,
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
            .any(|key| auth::hash_api_key("alpha") == key.key_hash));

        std::thread::sleep(Duration::from_millis(10));
        fs::write(&path, r#"[]"#).expect("revoke all api keys");
        assert!(manager.reload().expect("reload should succeed"));
        let next = manager.snapshot();
        assert!(!next
            .values()
            .any(|key| auth::hash_api_key("alpha") == key.key_hash));
    }
}
