use crate::env;
use crate::errors::{ApexError, Result};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// NATS endpoint used when `NATS_URL` is unset/blank (B289).
pub const DEFAULT_NATS_URL: &str = "nats://127.0.0.1:4222";

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
/// | `ENABLE_HEADLESS_BROWSER` | `false`                       | Enable only when scraping JS pages; requires Chromium under the non-root `apexintel` service account (no `--no-sandbox`) |
/// | `ENABLE_WASM_PREVIEW`     | `false`                       | **Retired** — the Rust/WASM preview UI was removed and CI forbids the crate; setting this to `true` fails `AppConfig::validate()` |
/// | `CRAWL_INTERVAL_SECS`     | `3600` (1 h)                 | How often to re-crawl domains        |
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
    /// **Retired.** Gate for the Rust/WASM preview UI; the preview crate was
    /// removed from the workspace and CI forbids it. Default: `false`. Setting
    /// this to `true` is rejected by [`AppConfig::validate`] so an operator who
    /// still sets `ENABLE_WASM_PREVIEW=true` fails loudly instead of the flag
    /// being silently ignored.
    pub enable_wasm_preview: bool,

    // ── Scheduling ──────────────────────────────────────────────
    /// Crawl polling interval in seconds.  Default: `3600` (1 hour).
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
            nats_url: nats_url_from_env(),
            minio_url: env_or(env::MINIO_URL, "http://127.0.0.1:9000"),
            minio_bucket: env_or(env::MINIO_BUCKET, "apexintel"),

            llm_base_url: opt_env(env::LLM_BASE_URL),
            llm_api_key: opt_secret_env(env::LLM_API_KEY),
            llm_model: env_or(env::LLM_MODEL, "Qwen3-30B-A3B-Q4_K_M"),

            smtp_url: opt_secret_env(env::SMTP_URL),

            enable_proxy_rotation: parse_bool_env(env::ENABLE_PROXY_ROTATION, false)?,
            enable_headless_browser: parse_bool_env(env::ENABLE_HEADLESS_BROWSER, false)?,
            enable_wasm_preview: parse_bool_env(env::ENABLE_WASM_PREVIEW, false)?,

            crawl_interval_secs: parse_u64_env(env::CRAWL_INTERVAL_SECS, 3600)?,
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

    /// `NATS_URL` only when explicitly set and non-empty (after trimming).
    ///
    /// [`AppConfig::nats_url`] always carries the localhost default; the
    /// worker's alert publishers must distinguish "not configured" (they
    /// disable publishing and leave events queued) from "dial the local
    /// broker", so they resolve their endpoint through this helper instead.
    /// The raw value is returned so call sites keep their exact trim and log
    /// behavior.
    pub fn nats_url_configured_from_env() -> Option<String> {
        std::env::var(env::NATS_URL)
            .ok()
            .filter(|url| !url.trim().is_empty())
    }

    pub fn redis_url_value(&self) -> &str {
        self.redis_url.expose_secret()
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
    /// | `enable_wasm_preview`     | `false`       | Retired Rust/WASM preview UI; `true` is rejected |
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
        if self.enable_wasm_preview {
            errors.push(
                "AppConfig.enable_wasm_preview = true is invalid: the Rust/WASM preview UI is \
                 retired (the crate was removed and CI forbids it); unset ENABLE_WASM_PREVIEW or \
                 set it to false"
                    .to_string(),
            );
        }

        errors
    }
}

/// One malformed configuration value.
///
/// Resolution code collects these instead of silently substituting a default:
/// an operator who typo'd `APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT=banana`
/// must see a configuration error, not a readiness verdict computed from the
/// fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigError {
    /// Environment variable (or field path) that failed to parse.
    pub variable: String,
    /// Raw value as supplied by the environment.
    pub value: String,
    /// Short description of the accepted values.
    pub expected: String,
}

impl ConfigError {
    pub fn new(
        variable: impl Into<String>,
        value: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        Self {
            variable: variable.into(),
            value: value.into(),
            expected: expected.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}={:?} is invalid; expected {}",
            self.variable, self.value, self.expected
        )
    }
}

impl std::error::Error for ConfigError {}

/// Collected configuration errors from one resolution pass.
///
/// Every threshold resolver keeps its defaults for absent variables, but a
/// *present and malformed* value is recorded here and the resolver returns
/// `Err(ConfigErrors)` so startup (or the readiness probe) fails loudly instead
/// of applying a silent default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConfigErrors {
    pub errors: Vec<ConfigError>,
}

impl ConfigErrors {
    pub fn new() -> Self {
        Self::default()
    }

    /// A single-error set, for resolvers that parse one variable at a time.
    pub fn single(
        variable: impl Into<String>,
        value: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        let mut errors = Self::new();
        errors.push(variable, value, expected);
        errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn push(
        &mut self,
        variable: impl Into<String>,
        value: impl Into<String>,
        expected: impl Into<String>,
    ) {
        self.errors
            .push(ConfigError::new(variable, value, expected));
    }

    /// Collect errors from `other` into `self`.
    pub fn extend(&mut self, other: ConfigErrors) {
        self.errors.extend(other.errors);
    }

    pub fn into_result(self) -> std::result::Result<(), Self> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl std::fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered = self
            .errors
            .iter()
            .map(ConfigError::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        write!(f, "configuration errors: {rendered}")
    }
}

impl std::error::Error for ConfigErrors {}

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

/// Resolve `NATS_URL` with the documented localhost default (B289).
///
/// Single source for the default so `AppConfig::from_env` and runtime probes
/// can never drift apart.
pub fn nats_url_from_env() -> String {
    env_or(env::NATS_URL, DEFAULT_NATS_URL)
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
    fn config_errors_render_and_fail_loudly() {
        let errors = ConfigErrors::single(
            "APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT",
            "banana",
            "an integer percentage in 0..=100",
        );
        assert!(!errors.is_empty());
        let rendered = errors.to_string();
        assert!(rendered.contains("APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT"));
        assert!(rendered.contains("banana"));

        let error = errors.errors[0].clone();
        assert_eq!(
            error.variable,
            "APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT"
        );
        assert_eq!(error.value, "banana");
        assert!(error.to_string().contains("expected"));
    }

    #[test]
    fn empty_config_errors_convert_to_ok() {
        let mut errors = ConfigErrors::new();
        errors.push("A", "b", "c");
        let mut other = ConfigErrors::new();
        other.push("D", "e", "f");
        errors.extend(other);
        assert_eq!(errors.errors.len(), 2);
        assert!(errors.into_result().is_err());
        assert!(ConfigErrors::new().into_result().is_ok());
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
        assert_eq!(cfg.crawl_interval_secs, 3600);
        assert!(!cfg.enable_proxy_rotation);
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_nats_url_flows_through_and_defaults_to_localhost() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://test:test@localhost/nats-test");

        std::env::remove_var(env::NATS_URL);
        let cfg = AppConfig::from_env().unwrap();
        assert_eq!(cfg.nats_url, DEFAULT_NATS_URL);
        assert_eq!(nats_url_from_env(), DEFAULT_NATS_URL);
        assert_eq!(AppConfig::nats_url_configured_from_env(), None);

        std::env::set_var(env::NATS_URL, "nats://nats.internal:4333");
        let cfg = AppConfig::from_env().unwrap();
        assert_eq!(cfg.nats_url, "nats://nats.internal:4333");
        assert_eq!(nats_url_from_env(), "nats://nats.internal:4333");
        assert_eq!(
            AppConfig::nats_url_configured_from_env().as_deref(),
            Some("nats://nats.internal:4333")
        );

        // A set-but-blank value keeps the historical semantics: the defaulted
        // config field preserves the raw value, and the worker publishers
        // treat it as unconfigured.
        std::env::set_var(env::NATS_URL, "   ");
        let cfg = AppConfig::from_env().unwrap();
        assert_eq!(cfg.nats_url, "   ");
        assert_eq!(AppConfig::nats_url_configured_from_env(), None);

        std::env::remove_var(env::NATS_URL);
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
    fn test_from_env_wasm_preview_defaults_false() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        std::env::remove_var(env::ENABLE_WASM_PREVIEW);
        let cfg = AppConfig::from_env().unwrap();
        assert!(
            !cfg.enable_wasm_preview,
            "the retired WASM preview flag must default to false"
        );
        assert!(
            cfg.validate().is_empty(),
            "the default configuration must validate cleanly"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_app_config_wasm_preview_true_is_rejected() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("DATABASE_URL", "postgres://x@localhost/test");
        std::env::set_var(env::ENABLE_WASM_PREVIEW, "true");
        let cfg = AppConfig::from_env().unwrap();
        assert!(
            cfg.enable_wasm_preview,
            "an explicit true is parsed honestly, then rejected by validate()"
        );
        let errs = cfg.validate();
        assert!(
            errs.iter().any(|e| e.contains("enable_wasm_preview")),
            "an explicitly enabled retired flag must fail validation: {errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("retired")),
            "the validation error must explain the flag is retired: {errs:?}"
        );
        std::env::remove_var(env::ENABLE_WASM_PREVIEW);
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
