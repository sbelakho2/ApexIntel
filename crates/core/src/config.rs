use crate::errors::{ApexError, Result};
use serde::{Deserialize, Serialize};

/// Global application configuration, loaded from env vars and/or config files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub nats_url: String,
    pub minio_url: String,
    pub minio_bucket: String,

    // API keys
    pub google_api_key: Option<String>,
    pub google_search_engine_id: Option<String>,
    pub nexar_client_id: Option<String>,
    pub nexar_client_secret: Option<String>,
    pub mouser_api_key: Option<String>,
    pub digikey_client_id: Option<String>,

    // LLM
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: String,

    // SMTP
    pub smtp_url: Option<String>,

    // Feature flags
    pub enable_proxy_rotation: bool,
    pub enable_headless_browser: bool,

    // Scheduling
    pub crawl_interval_secs: u64,
    pub nightly_hour_utc: u32,
    pub weekly_day: u32, // 0=Mon .. 6=Sun

    // Rate limiting defaults
    pub default_requests_per_second: f64,
    pub proxy_pool_size: usize,
}

impl AppConfig {
    /// Load configuration from environment variables.
    /// Call `dotenvy::dotenv().ok()` before this if you want .env support.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: require_env("DATABASE_URL")?,
            redis_url: env_or("REDIS_URL", "redis://127.0.0.1:6379"),
            nats_url: env_or("NATS_URL", "nats://127.0.0.1:4222"),
            minio_url: env_or("MINIO_URL", "http://127.0.0.1:9000"),
            minio_bucket: env_or("MINIO_BUCKET", "apexintel"),

            google_api_key: opt_env("GOOGLE_API_KEY"),
            google_search_engine_id: opt_env("GOOGLE_SEARCH_ENGINE_ID"),
            nexar_client_id: opt_env("NEXAR_CLIENT_ID"),
            nexar_client_secret: opt_env("NEXAR_CLIENT_SECRET"),
            mouser_api_key: opt_env("MOUSER_API_KEY"),
            digikey_client_id: opt_env("DIGIKEY_CLIENT_ID"),

            llm_base_url: opt_env("LLM_BASE_URL"),
            llm_api_key: opt_env("LLM_API_KEY"),
            llm_model: env_or("LLM_MODEL", "Qwen3-Next-80B-A3B-Instruct-Q4_K_M"),

            smtp_url: opt_env("SMTP_URL"),

            enable_proxy_rotation: env_or("ENABLE_PROXY_ROTATION", "false")
                .parse()
                .unwrap_or(false),
            enable_headless_browser: env_or("ENABLE_HEADLESS_BROWSER", "false")
                .parse()
                .unwrap_or(false),

            crawl_interval_secs: env_or("CRAWL_INTERVAL_SECS", "21600")
                .parse()
                .unwrap_or(21600),
            nightly_hour_utc: env_or("NIGHTLY_HOUR_UTC", "2")
                .parse()
                .unwrap_or(2),
            weekly_day: env_or("WEEKLY_DAY", "0").parse().unwrap_or(0),

            default_requests_per_second: env_or("DEFAULT_RPS", "0.2")
                .parse()
                .unwrap_or(0.2),
            proxy_pool_size: env_or("PROXY_POOL_SIZE", "50")
                .parse()
                .unwrap_or(50),
        })
    }
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| ApexError::Config(format!("missing required env var: {key}")))
}

fn opt_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_env_missing() {
        // Ensure a key that doesn't exist returns config error
        std::env::remove_var("__APEX_TEST_MISSING__");
        let res = require_env("__APEX_TEST_MISSING__");
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("__APEX_TEST_MISSING__"));
    }

    #[test]
    fn test_require_env_present() {
        std::env::set_var("__APEX_TEST_PRESENT__", "hello");
        let res = require_env("__APEX_TEST_PRESENT__");
        assert_eq!(res.unwrap(), "hello");
        std::env::remove_var("__APEX_TEST_PRESENT__");
    }

    #[test]
    fn test_opt_env() {
        std::env::remove_var("__APEX_OPT_MISS__");
        assert!(opt_env("__APEX_OPT_MISS__").is_none());

        std::env::set_var("__APEX_OPT_HIT__", "val");
        assert_eq!(opt_env("__APEX_OPT_HIT__").unwrap(), "val");
        std::env::remove_var("__APEX_OPT_HIT__");
    }

    #[test]
    fn test_env_or_default() {
        std::env::remove_var("__APEX_DEFAULT__");
        assert_eq!(env_or("__APEX_DEFAULT__", "fallback"), "fallback");

        std::env::set_var("__APEX_DEFAULT__", "override");
        assert_eq!(env_or("__APEX_DEFAULT__", "fallback"), "override");
        std::env::remove_var("__APEX_DEFAULT__");
    }

    #[test]
    fn test_from_env_missing_database_url() {
        // Test that require_env fails for a missing variable.
        // We cannot safely unset DATABASE_URL in parallel tests,
        // so we verify the mechanism with a variable that's never set.
        let res = require_env("__APEX_DEFINITELY_NOT_SET_XYZ__");
        assert!(res.is_err());
    }

    #[test]
    fn test_from_env_with_database_url() {
        std::env::set_var("DATABASE_URL", "postgres://test:test@localhost/test");
        let cfg = AppConfig::from_env().unwrap();
        assert_eq!(cfg.database_url, "postgres://test:test@localhost/test");
        assert_eq!(cfg.llm_model, "Qwen3-Next-80B-A3B-Instruct-Q4_K_M");
        assert_eq!(cfg.crawl_interval_secs, 21600);
        assert!(!cfg.enable_proxy_rotation);
        std::env::remove_var("DATABASE_URL");
    }
}
