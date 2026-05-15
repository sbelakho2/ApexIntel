use std::path::PathBuf;

use anyhow::Result;
use apex_core::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProviderChoice {
    LlamaCpp,
    OpenAi,
    AzureOpenAi,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub cors_origin: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportConfig {
    pub default_window: u32,
    pub max_window: u32,
    pub chunk_size: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchConfig {
    pub index_path: PathBuf,
    pub allow_degraded_startup: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiKeysConfig {
    pub file_path: Option<PathBuf>,
    pub reload_interval_secs: u64,
    pub env_slots: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriorityWeights {
    pub decision_power: f64,
    pub domain_relevance: f64,
    pub network_centrality: f64,
    pub engagement_potential: f64,
    pub intelligence_value: f64,
}

impl PriorityWeights {
    pub fn total(&self) -> f64 {
        self.decision_power
            + self.domain_relevance
            + self.network_centrality
            + self.engagement_potential
            + self.intelligence_value
    }
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            decision_power: 0.25,
            domain_relevance: 0.20,
            network_centrality: 0.20,
            engagement_potential: 0.15,
            intelligence_value: 0.20,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpBudgetConfig {
    pub dependency_timeout_secs: u64,
    pub llm_health_timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRuntimeConfig {
    pub provider_override: Option<LlmProviderChoice>,
    pub validate_model_name: bool,
    pub allowed_models: Vec<String>,
    pub primary_max_tokens: u32,
    pub primary_timeout_secs: u32,
    pub lightweight_max_tokens: u32,
    pub lightweight_timeout_secs: u32,
}

#[derive(Debug, Clone)]
pub struct ApiRuntimeConfig {
    pub app: AppConfig,
    pub server: ServerConfig,
    pub export: ExportConfig,
    pub search: SearchConfig,
    pub api_keys: ApiKeysConfig,
    pub priority_weights: PriorityWeights,
    pub http_budgets: HttpBudgetConfig,
    pub llm: LlmRuntimeConfig,
}

impl ApiRuntimeConfig {
    pub fn from_env() -> Result<Self> {
        let app = AppConfig::from_env()?;

        Ok(Self {
            app,
            server: ServerConfig {
                host: env_or("HOST", "0.0.0.0"),
                port: parse_u16_env("PORT", 8080)?,
                cors_origin: env_or("CORS_ORIGIN", "http://localhost:3000"),
            },
            export: ExportConfig {
                default_window: parse_u32_env("API_EXPORT_DEFAULT_WINDOW", 1_000)?,
                max_window: parse_u32_env("API_EXPORT_MAX_WINDOW", 10_000)?,
                chunk_size: parse_u32_env("API_EXPORT_CHUNK_SIZE", 250)?,
            },
            search: SearchConfig {
                index_path: PathBuf::from(env_or("SEARCH_INDEX_PATH", "data/search")),
                allow_degraded_startup: parse_bool_env("API_ALLOW_DEGRADED_SEARCH_STARTUP", true)?,
            },
            api_keys: ApiKeysConfig {
                file_path: std::env::var("API_KEYS_FILE")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from),
                reload_interval_secs: parse_u64_env("API_KEYS_RELOAD_INTERVAL_SECS", 15)?,
                env_slots: parse_usize_env("API_KEYS_ENV_SLOTS", 50)?,
            },
            priority_weights: PriorityWeights {
                decision_power: parse_f64_env("API_PRIORITY_WEIGHT_DECISION_POWER", 0.25)?,
                domain_relevance: parse_f64_env("API_PRIORITY_WEIGHT_DOMAIN_RELEVANCE", 0.20)?,
                network_centrality: parse_f64_env("API_PRIORITY_WEIGHT_NETWORK_CENTRALITY", 0.20)?,
                engagement_potential: parse_f64_env(
                    "API_PRIORITY_WEIGHT_ENGAGEMENT_POTENTIAL",
                    0.15,
                )?,
                intelligence_value: parse_f64_env("API_PRIORITY_WEIGHT_INTELLIGENCE_VALUE", 0.20)?,
            },
            http_budgets: HttpBudgetConfig {
                dependency_timeout_secs: parse_u64_env("API_DEPENDENCY_TIMEOUT_SECS", 5)?,
                llm_health_timeout_secs: parse_u64_env("API_LLM_HEALTH_TIMEOUT_SECS", 10)?,
            },
            llm: LlmRuntimeConfig {
                provider_override: parse_llm_provider_env("API_LLM_PROVIDER")?,
                validate_model_name: parse_bool_env("API_VALIDATE_LLM_MODEL_NAME", true)?,
                allowed_models: parse_csv_env("API_LLM_ALLOWED_MODELS"),
                primary_max_tokens: parse_u32_env("API_LLM_PRIMARY_MAX_TOKENS", 4096)?,
                primary_timeout_secs: parse_u32_env("API_LLM_PRIMARY_TIMEOUT_SECS", 300)?,
                lightweight_max_tokens: parse_u32_env("API_LLM_LIGHTWEIGHT_MAX_TOKENS", 1024)?,
                lightweight_timeout_secs: parse_u32_env("API_LLM_LIGHTWEIGHT_TIMEOUT_SECS", 120)?,
            },
        })
    }

    pub fn llm_model_name(&self) -> &str {
        self.app.llm_model.trim()
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = self.app.validate();

        if self.server.host.trim().is_empty() {
            errors.push("ApiRuntimeConfig.server.host must not be empty".to_string());
        }
        if self.server.port == 0 {
            errors.push("ApiRuntimeConfig.server.port must be > 0".to_string());
        }
        if self.export.default_window == 0 {
            errors.push("ApiRuntimeConfig.export.default_window must be >= 1".to_string());
        }
        if self.export.max_window == 0 {
            errors.push("ApiRuntimeConfig.export.max_window must be >= 1".to_string());
        }
        if self.export.default_window > self.export.max_window {
            errors.push("ApiRuntimeConfig.export.default_window must be <= max_window".to_string());
        }
        if self.export.chunk_size == 0 {
            errors.push("ApiRuntimeConfig.export.chunk_size must be >= 1".to_string());
        }
        if self.api_keys.env_slots == 0 {
            errors.push("ApiRuntimeConfig.api_keys.env_slots must be >= 1".to_string());
        }
        if self.priority_weights.total() <= f64::EPSILON {
            errors.push("ApiRuntimeConfig.priority_weights total must be > 0".to_string());
        }
        if self.http_budgets.dependency_timeout_secs == 0 {
            errors.push(
                "ApiRuntimeConfig.http_budgets.dependency_timeout_secs must be >= 1".to_string(),
            );
        }
        if self.http_budgets.llm_health_timeout_secs == 0 {
            errors.push(
                "ApiRuntimeConfig.http_budgets.llm_health_timeout_secs must be >= 1".to_string(),
            );
        }
        if self.llm.primary_max_tokens == 0 || self.llm.lightweight_max_tokens == 0 {
            errors.push("ApiRuntimeConfig.llm max token values must be >= 1".to_string());
        }
        if self.llm.primary_timeout_secs == 0 || self.llm.lightweight_timeout_secs == 0 {
            errors.push("ApiRuntimeConfig.llm timeout values must be >= 1".to_string());
        }
        if self.llm.validate_model_name
            && self
                .app
                .llm_base_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            && !self.llm.allowed_models.is_empty()
            && !self
                .llm
                .allowed_models
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(self.llm_model_name()))
        {
            errors.push(format!(
                "ApiRuntimeConfig.llm.model_name '{}' is not present in API_LLM_ALLOWED_MODELS",
                self.llm_model_name()
            ));
        }

        errors
    }
}

fn parse_csv_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
    match std::env::var(key) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => anyhow::bail!("{} must be a boolean, got '{}'", key, other),
        },
        Err(_) => Ok(default),
    }
}

fn parse_u16_env(key: &str, default: u16) -> Result<u16> {
    match std::env::var(key) {
        Ok(value) => value.trim().parse::<u16>().map_err(Into::into),
        Err(_) => Ok(default),
    }
}

fn parse_u32_env(key: &str, default: u32) -> Result<u32> {
    match std::env::var(key) {
        Ok(value) => value.trim().parse::<u32>().map_err(Into::into),
        Err(_) => Ok(default),
    }
}

fn parse_u64_env(key: &str, default: u64) -> Result<u64> {
    match std::env::var(key) {
        Ok(value) => value.trim().parse::<u64>().map_err(Into::into),
        Err(_) => Ok(default),
    }
}

fn parse_usize_env(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Ok(value) => value.trim().parse::<usize>().map_err(Into::into),
        Err(_) => Ok(default),
    }
}

fn parse_f64_env(key: &str, default: f64) -> Result<f64> {
    match std::env::var(key) {
        Ok(value) => value.trim().parse::<f64>().map_err(Into::into),
        Err(_) => Ok(default),
    }
}

fn parse_llm_provider_env(key: &str) -> Result<Option<LlmProviderChoice>> {
    let Some(value) = std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };

    let provider = match value.trim().to_ascii_lowercase().as_str() {
        "llamacpp" | "llama_cpp" | "llama-cpp" => LlmProviderChoice::LlamaCpp,
        "openai" => LlmProviderChoice::OpenAi,
        "azure_openai" | "azure-openai" | "azure" => LlmProviderChoice::AzureOpenAi,
        other => anyhow::bail!(
            "{} must be one of llamacpp|openai|azure_openai, got '{}'",
            key,
            other
        ),
    };

    Ok(Some(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_config_test_env() {
        std::env::remove_var("HOST");
        std::env::remove_var("PORT");
        std::env::remove_var("LLM_BASE_URL");
        std::env::remove_var("API_DEPENDENCY_TIMEOUT_SECS");
        std::env::remove_var("API_LLM_ALLOWED_MODELS");
        std::env::remove_var("LLM_MODEL");
    }

    #[test]
    fn config_uses_default_host_when_env_missing() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        let config = ApiRuntimeConfig::from_env().expect("config");
        assert_eq!(config.server.host, "0.0.0.0");
    }

    #[test]
    fn config_rejects_invalid_port_values() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var("PORT", "not-a-port");
        let err = parse_u16_env("PORT", 8080).expect_err("invalid port should fail");
        assert!(err.to_string().contains("invalid digit"));
        std::env::remove_var("PORT");
    }

    #[test]
    fn config_snapshot_matches_expected_defaults() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        let config = ApiRuntimeConfig::from_env().expect("config");
        assert_eq!(config.export.default_window, 1_000);
        assert_eq!(config.export.max_window, 10_000);
        assert_eq!(config.http_budgets.dependency_timeout_secs, 5);
        assert_eq!(config.llm.primary_timeout_secs, 300);
    }

    #[test]
    fn http_clients_use_env_timeout_override() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        std::env::set_var("API_DEPENDENCY_TIMEOUT_SECS", "17");
        let config = ApiRuntimeConfig::from_env().expect("config");
        assert_eq!(config.http_budgets.dependency_timeout_secs, 17);
        std::env::remove_var("API_DEPENDENCY_TIMEOUT_SECS");
    }

    #[test]
    fn default_timeout_values_are_applied_when_env_missing() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        std::env::remove_var("API_DEPENDENCY_TIMEOUT_SECS");
        let config = ApiRuntimeConfig::from_env().expect("config");
        assert_eq!(config.http_budgets.dependency_timeout_secs, 5);
        assert_eq!(config.http_budgets.llm_health_timeout_secs, 10);
    }

    #[test]
    fn llm_model_name_uses_env_or_config_override() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        std::env::set_var("LLM_MODEL", "custom-model");
        let config = ApiRuntimeConfig::from_env().expect("config");
        assert_eq!(config.llm_model_name(), "custom-model");
        std::env::remove_var("LLM_MODEL");
    }

    #[test]
    fn startup_rejects_unknown_model_name_when_validation_enabled() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        std::env::set_var("LLM_BASE_URL", "http://localhost:8080");
        std::env::set_var("LLM_MODEL", "unknown-model");
        std::env::set_var("API_LLM_ALLOWED_MODELS", "approved-model,backup-model");

        let config = ApiRuntimeConfig::from_env().expect("config");
        let errors = config.validate();

        assert!(errors
            .iter()
            .any(|err| err.contains("API_LLM_ALLOWED_MODELS")));
    }

    #[test]
    fn default_model_name_is_reported_from_config_not_literal_handler_code() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );

        let config = ApiRuntimeConfig::from_env().expect("config");

        assert_eq!(config.llm_model_name(), config.app.llm_model);
    }

    #[test]
    fn priority_vector_rejects_invalid_weight_configuration() {
        let _guard = ENV_LOCK.lock().expect("config env lock");
        reset_config_test_env();
        std::env::set_var(
            "DATABASE_URL",
            "postgres://postgres:postgres@localhost/apex",
        );
        let mut config = ApiRuntimeConfig::from_env().expect("config");
        config.priority_weights.decision_power = 0.0;
        config.priority_weights.domain_relevance = 0.0;
        config.priority_weights.network_centrality = 0.0;
        config.priority_weights.engagement_potential = 0.0;
        config.priority_weights.intelligence_value = 0.0;

        let errors = config.validate();
        assert!(errors
            .iter()
            .any(|err| err.contains("priority_weights total must be > 0")));
    }
}
