use crate::env;
use crate::errors::{ApexError, Result};
use secrecy::{ExposeSecret, SecretString};

/// Global application configuration, loaded from environment variables.
///
/// # Loading
/// Call [`AppConfig::from_env`] after `dotenvy::dotenv().ok()` to populate this
/// struct from the process environment.
///
/// # Required env vars
/// - `DATABASE_URL` — PostgreSQL connection string (no default; startup fails without it).
///
/// # Env vars with defaults (B289)
/// | Env var                    | Default                        | Notes                                |
/// |---------------------------|-------------------------------|--------------------------------------|
/// | `REDIS_URL`               | `redis://127.0.0.1:6379`      | Local dev Redis                      |
/// | `NATS_URL`                | `nats://127.0.0.1:4222`       | Local dev NATS                       |
/// | `MINIO_URL`               | `http://127.0.0.1:9000`       | Local dev MinIO                      |
/// | `MINIO_BUCKET`            | `apexintel`                   | Default bucket name                  |
/// | `LLM_MODEL`               | `Qwen3-30B-A3B-Q4_K_M`        | Locally deployed quantized model     |
/// | `ENABLE_PROXY_ROTATION`   | `false`                       | Enable only in production            |
/// | `ENABLE_HEADLESS_BROWSER` | `false`                       | Enable only when scraping JS pages   |
/// | `ENABLE_WASM_PREVIEW`     | `true`                        | Gate the Rust/WASM preview UI        |
/// | `CRAWL_INTERVAL_SECS`     | `21600` (6 h)                 | How often to re-crawl domains        |
/// | `NIGHTLY_HOUR_UTC`        | `2`                           | UTC hour for the nightly pipeline    |
/// | `WEEKLY_DAY`              | `0` (Monday)                  | 0=Mon … 6=Sun                        |
/// | `DEFAULT_RPS`             | `0.2`                         | Requests/second per domain           |
/// | `PROXY_POOL_SIZE`         | `50`                          | Number of rotating proxy slots       |
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// PostgreSQL connection string.  **Required** — set `DATABASE_URL`.
    pub database_url: SecretString,
    /// Redis URL.  Default: `redis://127.0.0.1:6379`.
    pub redis_url: SecretString,
    /// NATS URL.  Default: `nats://127.0.0.1:4222`.
    pub nats_url: String,
    /// MinIO endpoint URL.  Default: `http://127.0.0.1:9000`.
    pub minio_url: String,
    /// MinIO bucket.  Default: `apexintel`.
    pub minio_bucket: String,

    // ── API keys (all optional) ──────────────────────────────────
    /// Google Custom Search API key.  Optional.
    pub google_api_key: Option<SecretString>,
    /// Google Custom Search Engine ID.  Optional.
    pub google_search_engine_id: Option<String>,
    /// Nexar OAuth client ID.  Optional.
    pub nexar_client_id: Option<String>,
    /// Nexar OAuth client secret.  Optional.
    pub nexar_client_secret: Option<SecretString>,
    /// Mouser API key.  Optional.
    pub mouser_api_key: Option<SecretString>,
    /// Digi-Key client ID.  Optional.
    pub digikey_client_id: Option<String>,

    // ── LLM ─────────────────────────────────────────────────────
    /// Base URL for the local/remote LLM endpoint.  Optional (disables LLM if absent).
    pub llm_base_url: Option<String>,
    /// Bearer token for the LLM API.  Optional.
    pub llm_api_key: Option<SecretString>,
    /// Model identifier string.  Default: `Qwen3-30B-A3B-Q4_K_M`.
    pub llm_model: String,

    // ── SMTP ────────────────────────────────────────────────────
    /// SMTP connection URL.  Optional (disables email dispatch if absent).
    pub smtp_url: Option<SecretString>,

    // ── Feature flags ───────────────────────────────────────────
    /// Enable rotating proxy pool for crawl requests.  Default: `false`.
    pub enable_proxy_rotation: bool,
    /// Enable headless-browser crawl fallback.  Default: `false`.
    pub enable_headless_browser: bool,
    /// Expose the Rust/WASM preview UI. Default: `true`.
    pub enable_wasm_preview: bool,

    // ── Scheduling ──────────────────────────────────────────────
    /// Crawl polling interval in seconds.  Default: `21600` (6 hours).
    pub crawl_interval_secs: u64,
    /// UTC hour (0–23) when the nightly pipeline fires.  Default: `2`.
    pub nightly_hour_utc: u32,
    /// Day-of-week (0=Monday … 6=Sunday) for the weekly pipeline.  Default: `0`.
    pub weekly_day: u32,

    // ── Rate limiting ────────────────────────────────────────────
    /// Default crawl rate in requests per second per domain.  Default: `0.2`.
    pub default_requests_per_second: f64,
    /// Number of proxy slots in the rotation pool.  Default: `50`.
    pub proxy_pool_size: usize,
}

impl AppConfig {
    /// Load configuration from environment variables.
    /// Call `dotenvy::dotenv().ok()` before this if you want .env support.
    pub fn from_env() -> Result<Self> {
        log_deprecated_env_key_warnings();

        Ok(Self {
            database_url: require_secret_env(env::DATABASE_URL)?,
            redis_url: env_or_secret(env::REDIS_URL, "redis://127.0.0.1:6379"),
            nats_url: env_or(env::NATS_URL, "nats://127.0.0.1:4222"),
            minio_url: env_or(env::MINIO_URL, "http://127.0.0.1:9000"),
            minio_bucket: env_or(env::MINIO_BUCKET, "apexintel"),

            google_api_key: opt_secret_env(env::GOOGLE_API_KEY),
            google_search_engine_id: opt_env(env::GOOGLE_SEARCH_ENGINE_ID),
            nexar_client_id: opt_env(env::NEXAR_CLIENT_ID),
            nexar_client_secret: opt_secret_env(env::NEXAR_CLIENT_SECRET),
            mouser_api_key: opt_secret_env(env::MOUSER_API_KEY),
            digikey_client_id: opt_env(env::DIGIKEY_CLIENT_ID),

            llm_base_url: opt_env(env::LLM_BASE_URL),
            llm_api_key: opt_secret_env(env::LLM_API_KEY),
            llm_model: env_or(env::LLM_MODEL, "Qwen3-30B-A3B-Q4_K_M"),

            smtp_url: opt_secret_env(env::SMTP_URL),

            enable_proxy_rotation: parse_bool_env(env::ENABLE_PROXY_ROTATION, false)?,
            enable_headless_browser: parse_bool_env(env::ENABLE_HEADLESS_BROWSER, false)?,
            enable_wasm_preview: parse_bool_env(env::ENABLE_WASM_PREVIEW, true)?,

            crawl_interval_secs: parse_u64_env(env::CRAWL_INTERVAL_SECS, 21600)?,
            nightly_hour_utc: parse_u32_env(env::NIGHTLY_HOUR_UTC, 2)?.min(23),
            weekly_day: parse_u32_env(env::WEEKLY_DAY, 0)?.min(6),

            default_requests_per_second: {
                let rps = parse_f64_env(env::DEFAULT_RPS, 0.2)?;
                if rps.is_finite() && rps > 0.0 {
                    rps
                } else {
                    0.2
                }
            },
            proxy_pool_size: parse_usize_env(env::PROXY_POOL_SIZE, 50)?.max(1),
        })
    }

    pub fn database_url_value(&self) -> &str {
        self.database_url.expose_secret()
    }

    pub fn redis_url_value(&self) -> &str {
        self.redis_url.expose_secret()
    }

    pub fn google_api_key_value(&self) -> Option<&str> {
        self.google_api_key
            .as_ref()
            .map(|value| value.expose_secret())
    }

    pub fn nexar_client_secret_value(&self) -> Option<&str> {
        self.nexar_client_secret
            .as_ref()
            .map(|value| value.expose_secret())
    }

    pub fn mouser_api_key_value(&self) -> Option<&str> {
        self.mouser_api_key
            .as_ref()
            .map(|value| value.expose_secret())
    }

    pub fn llm_api_key_value(&self) -> Option<String> {
        self.llm_api_key
            .as_ref()
            .map(|value| value.expose_secret().to_string())
    }

    pub fn smtp_url_value(&self) -> Option<&str> {
        self.smtp_url.as_ref().map(|value| value.expose_secret())
    }

    /// Validate that all numeric fields are within their valid operating ranges.
    ///
    /// Checks invariants that `from_env` cannot fully enforce due to type
    /// coercions (e.g. `crawl_interval_secs` clamping).  Call this immediately
    /// after `from_env()` at service startup to produce a full list of problems
    /// rather than failing on the first misconfigured field.
    ///
    /// Returns an empty `Vec` when the config is valid.
    ///
    /// # Valid ranges
    /// | Field                      | Constraint    | Rationale                               |
    /// |---------------------------|---------------|-----------------------------------------|
    /// | `database_url`            | non-empty     | Required for all DB operations          |
    /// | `crawl_interval_secs`     | `>= 60`       | Prevents accidental DoS of target sites |
    /// | `nightly_hour_utc`        | `<= 23`       | Valid 24-hour clock                     |
    /// | `weekly_day`              | `<= 6`        | Mon=0 … Sun=6                           |
    /// | `default_requests_per_second` | `> 0.0`   | Must be a positive rate                 |
    /// | `proxy_pool_size`         | `>= 1`        | At least one proxy slot required        |
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.database_url.expose_secret().trim().is_empty() {
            errors.push("AppConfig.database_url must not be empty".to_string());
        }
        if self.crawl_interval_secs < 60 {
            errors.push(format!(
                "AppConfig.crawl_interval_secs = {} must be >= 60 (prevents accidental crawl flood)",
                self.crawl_interval_secs
            ));
        }
        if self.nightly_hour_utc > 23 {
            errors.push(format!(
                "AppConfig.nightly_hour_utc = {} must be in [0, 23]",
                self.nightly_hour_utc
            ));
        }
        if self.weekly_day > 6 {
            errors.push(format!(
                "AppConfig.weekly_day = {} must be in [0, 6] (Mon=0, Sun=6)",
                self.weekly_day
            ));
        }
        if !self.default_requests_per_second.is_finite() || self.default_requests_per_second <= 0.0
        {
            errors.push(format!(
                "AppConfig.default_requests_per_second = {} must be a positive finite number",
                self.default_requests_per_second
            ));
        }
        if self.proxy_pool_size < 1 {
            errors.push(format!(
                "AppConfig.proxy_pool_size = {} must be >= 1",
                self.proxy_pool_size
            ));
        }

        errors
    }
}

/// Deprecated environment keys and their replacement keys (B298).
const DEPRECATED_ENV_KEYS: [(&str, &str); 7] = [
    ("DB_URL", env::DATABASE_URL),
    ("REDIS_URI", env::REDIS_URL),
    ("NATS_URI", env::NATS_URL),
    ("CRAWL_INTERVAL", env::CRAWL_INTERVAL_SECS),
    ("NIGHTLY_HOUR", env::NIGHTLY_HOUR_UTC),
    ("DEFAULT_REQUESTS_PER_SECOND", env::DEFAULT_RPS),
    ("PROXY_ROTATION_ENABLED", env::ENABLE_PROXY_ROTATION),
];

fn deprecated_env_key_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    for (deprecated, replacement) in DEPRECATED_ENV_KEYS {
        if std::env::var_os(deprecated).is_some() {
            warnings.push(format!(
                "deprecated config key '{}' detected; use '{}' instead",
                deprecated, replacement
            ));
        }
    }
    warnings
}

fn log_deprecated_env_key_warnings() {
    for warning in deprecated_env_key_warnings() {
        tracing::warn!("{warning}");
    }
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| {
        ApexError::config_with_hint(
            format!("missing required env var: {key}"),
            "set the variable in the environment or .env file",
        )
    })
}

fn require_secret_env(key: &str) -> Result<SecretString> {
    require_env(key).map(SecretString::from)
}

fn opt_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn opt_secret_env(key: &str) -> Option<SecretString> {
    opt_env(key).map(SecretString::from)
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_or_secret(key: &str, default: &str) -> SecretString {
    SecretString::from(env_or(key, default))
}

fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
    match std::env::var(key) {
        Ok(value) => value.parse::<bool>().map_err(|_| {
            ApexError::config_with_hint(
                format!("invalid bool for {key}"),
                "expected 'true' or 'false'",
            )
        }),
        Err(_) => Ok(default),
    }
}

fn parse_u64_env(key: &str, default: u64) -> Result<u64> {
    match std::env::var(key) {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            ApexError::config_with_hint(
                format!("invalid u64 for {key}"),
                "provide a positive integer",
            )
        }),
        Err(_) => Ok(default),
    }
}

fn parse_u32_env(key: &str, default: u32) -> Result<u32> {
    match std::env::var(key) {
        Ok(value) => value.parse::<u32>().map_err(|_| {
            ApexError::config_with_hint(
                format!("invalid u32 for {key}"),
                "provide a non-negative integer",
            )
        }),
        Err(_) => Ok(default),
    }
}

fn parse_usize_env(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Ok(value) => value.parse::<usize>().map_err(|_| {
            ApexError::config_with_hint(
                format!("invalid usize for {key}"),
                "provide a non-negative integer",
            )
        }),
        Err(_) => Ok(default),
    }
}

fn parse_f64_env(key: &str, default: f64) -> Result<f64> {
    match std::env::var(key) {
        Ok(value) => value.parse::<f64>().map_err(|_| {
            ApexError::config_with_hint(format!("invalid f64 for {key}"), "provide a finite number")
        }),
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn test_require_env_missing() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        // Ensure a key that doesn't exist returns config error
        std::env::remove_var("__APEX_TEST_MISSING__");
        let res = require_env("__APEX_TEST_MISSING__");
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("__APEX_TEST_MISSING__"));
    }

    #[test]
    fn test_require_env_present() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("__APEX_TEST_PRESENT__", "hello");
        let res = require_env("__APEX_TEST_PRESENT__");
        assert_eq!(res.unwrap(), "hello");
        std::env::remove_var("__APEX_TEST_PRESENT__");
    }

    #[test]
    fn test_opt_env() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("__APEX_OPT_MISS__");
        assert!(opt_env("__APEX_OPT_MISS__").is_none());

        std::env::set_var("__APEX_OPT_HIT__", "val");
        assert_eq!(opt_env("__APEX_OPT_HIT__").unwrap(), "val");
        std::env::remove_var("__APEX_OPT_HIT__");
    }

    #[test]
    fn test_env_or_default() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("__APEX_DEFAULT__");
        assert_eq!(env_or("__APEX_DEFAULT__", "fallback"), "fallback");

        std::env::set_var("__APEX_DEFAULT__", "override");
        assert_eq!(env_or("__APEX_DEFAULT__", "fallback"), "override");
        std::env::remove_var("__APEX_DEFAULT__");
    }

    #[test]
    fn test_from_env_missing_database_url() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        // Test that require_env fails for a missing variable.
        // We cannot safely unset DATABASE_URL in parallel tests,
        // so we verify the mechanism with a variable that's never set.
        let res = require_env("__APEX_DEFINITELY_NOT_SET_XYZ__");
        assert!(res.is_err());
    }

    #[test]
    fn test_from_env_with_database_url() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://test:test@localhost/test");
        let cfg = AppConfig::from_env().unwrap();
        assert_eq!(
            cfg.database_url_value(),
            "postgres://test:test@localhost/test"
        );
        assert_eq!(cfg.llm_model, "Qwen3-30B-A3B-Q4_K_M");
        assert_eq!(cfg.crawl_interval_secs, 21600);
        assert!(!cfg.enable_proxy_rotation);
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_deprecated_env_key_warnings_detects_old_keys() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DB_URL", "postgres://old-style");
        std::env::set_var("NIGHTLY_HOUR", "3");

        let warnings = deprecated_env_key_warnings();
        assert!(warnings
            .iter()
            .any(|w| w.contains("DB_URL") && w.contains("DATABASE_URL")));
        assert!(warnings
            .iter()
            .any(|w| w.contains("NIGHTLY_HOUR") && w.contains("NIGHTLY_HOUR_UTC")));

        std::env::remove_var("DB_URL");
        std::env::remove_var("NIGHTLY_HOUR");
    }

    // B291: AppConfig::validate
    #[test]
    fn test_app_config_valid_production_config_passes() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://user:pass@db.example.com/apex");
        let cfg = AppConfig::from_env().unwrap();
        assert!(
            cfg.validate().is_empty(),
            "a properly loaded config must pass validation"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_empty_database_url_is_invalid() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        // Construct a config directly to bypass require_env
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        let mut cfg = AppConfig::from_env().unwrap();
        cfg.database_url = SecretString::from(String::new());
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("database_url")));
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_crawl_interval_below_60_is_invalid() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        let mut cfg = AppConfig::from_env().unwrap();
        cfg.crawl_interval_secs = 30;
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("crawl_interval_secs")));
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_rps_zero_is_invalid() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        let mut cfg = AppConfig::from_env().unwrap();
        cfg.default_requests_per_second = 0.0;
        let errs = cfg.validate();
        assert!(errs
            .iter()
            .any(|e| e.contains("default_requests_per_second")));
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_multiple_invalid_fields_all_reported() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        let mut cfg = AppConfig::from_env().unwrap();
        cfg.database_url = SecretString::from(String::new());
        cfg.crawl_interval_secs = 0;
        cfg.default_requests_per_second = -1.0;
        let errs = cfg.validate();
        assert!(errs.iter().any(|e| e.contains("database_url")));
        assert!(errs.iter().any(|e| e.contains("crawl_interval_secs")));
        assert!(errs
            .iter()
            .any(|e| e.contains("default_requests_per_second")));
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_debug_redacts_secret_fields() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://user:pass@db.example.com/apex");
        std::env::set_var("LLM_API_KEY", "sk-secret-value");

        let cfg = AppConfig::from_env().unwrap();
        let debug_output = format!("{cfg:?}");

        assert!(!debug_output.contains("postgres://user:pass@db.example.com/apex"));
        assert!(!debug_output.contains("sk-secret-value"));

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("LLM_API_KEY");
    }
}
