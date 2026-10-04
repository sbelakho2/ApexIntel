//! Model-agnostic LLM client — trait, provider implementations, routing.
//!
//! # Provider compatibility
//!
//! | Provider       | API            | Auth            | Format notes |
//! |---------------|----------------|-----------------|---------------------------------------------------|
//! | `LlamaCpp`    | OpenAI-compat  | None (local)    | `max_tokens` caps output; `top_k`/`top_p` via ext |
//! | `OpenAi`      | OpenAI native  | `OPENAI_API_KEY`| Chat completions `/v1/chat/completions`           |
//! | `AzureOpenAi` | Azure OpenAI   | `AZURE_API_KEY` | Base URL includes deployment name                 |
//!
//! All three providers use the **OpenAI-compatible chat completions format**
//! (`messages: [{role, content}]`).  llama-server (llama.cpp) exposes exactly
//! this interface on port 8080 by default.
//!
//! # Local model setup
//!
//! Deployed on Hetzner EX44 (i5-13500, 64 GB RAM, no GPU) using GGUF Q4_K_M
//! quantisation via `llama-server`.  Typical inference: 2–4 tokens/sec.
//! Primary and lightweight tiers share the same server instance; `max_tokens`
//! differentiates them.  No Ollama wrapper is used.
//!
//! # Testability
//!
//! All pure logic (routing, config validation, response parsing) is testable
//! without external services.  Async HTTP is hidden behind the `LlmClient`
//! trait — provide a mock impl in tests.
pub mod advanced_prompting;
pub mod anti_hallucination;
pub mod cache;
pub mod evaluation;
pub mod inference;
pub mod insight_gen;
pub mod poi_profiler;
pub mod prompt_registry;
pub mod recipe_hypothesis;
pub mod self_improvement;
pub mod tiering;
pub mod validators;

// Re-exports from advanced_prompting
pub mod embeddings;

pub use advanced_prompting::{
    AdvancedPromptingEngine, AnalysisPerspective, CalibratedConfidence, CalibrationConfig,
    CalibrationFactor, ChainOfThoughtConfig, ConfidenceLevel, MultiPerspectiveConfig,
    MultiPerspectiveResult, PerspectiveResult, ReasoningPath, ReasoningStep, SelfConsistencyConfig,
    SelfConsistencyResult,
};

pub mod agents;
pub mod quality_control;
pub mod rag;

// ─────────────────────────────────────────────────────────────────────────────
// Experimental modules (B290)
//
// Compiled only when the `experimental` Cargo feature is enabled:
//   cargo build -p apex-llm --features experimental
//
// Rationale: these modules expose APIs that are not yet stable across all
// supported providers (llama.cpp / OpenAI / Azure) and may break without a
// semver major bump until stabilised.
// ─────────────────────────────────────────────────────────────────────────────

/// OpenAI-compatible function / tool calling.
///
/// Enable with `--features experimental`.  See [`function_calling`] module docs
/// for provider compatibility and stability caveats.
#[cfg(feature = "experimental")]
pub mod function_calling;

use anyhow::Result;
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

pub const EXPERIMENTAL_FEATURES_ENABLED: bool = cfg!(feature = "experimental");

pub fn truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

// ────────────────────────────────────────────
// Provider & Config
// ────────────────────────────────────────────

/// LLM provider backend.
///
/// # Compatibility
///
/// All three variants use the OpenAI-compatible chat completions format.
/// Certain parameters differ between providers:
///
/// | Field            | LlamaCpp         | OpenAi         | AzureOpenAi     |
/// |-----------------|------------------|----------------|------------------|
/// | `base_url`       | `http://host:8080` | `https://api.openai.com/v1` | deployment-specific URL |
/// | `api_key`        | not required     | required       | required (`AZURE_API_KEY`) |
/// | `model_name`     | name from GGUF   | e.g. `gpt-4o`  | deployment name  |
/// | streaming        | supported        | supported      | supported        |
/// | function calling | limited          | full support   | full support     |
///
/// Use [`ModelConfig::llamacpp_default`] for the typical local deployment.
/// Newtype wrapper around `SecretString` that provides safe Serialize/Deserialize.
///
/// - **Serialize**: always outputs the literal string `"***REDACTED***"`, preventing
///   accidental leakage of the API key through serialization (e.g. logging configs).
/// - **Deserialize**: reads a plain string from the input and wraps it in `SecretString`,
///   which zeroes memory on drop.
#[derive(Debug, Clone)]
pub struct ApiKeySecret(SecretString);

impl ApiKeySecret {
    /// Access the underlying secret string.
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<String> for ApiKeySecret {
    fn from(s: String) -> Self {
        Self(SecretString::from(s))
    }
}

impl From<&str> for ApiKeySecret {
    fn from(s: &str) -> Self {
        Self(SecretString::from(s.to_string()))
    }
}

impl Serialize for ApiKeySecret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("***REDACTED***")
    }
}

impl<'de> Deserialize<'de> for ApiKeySecret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(SecretString::from(s)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LlmProvider {
    LlamaCpp,
    OpenAi,
    AzureOpenAi,
}

impl LlmProvider {
    pub fn as_str(&self) -> &str {
        match self {
            Self::LlamaCpp => "llamacpp",
            Self::OpenAi => "openai",
            Self::AzureOpenAi => "azure_openai",
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::LlamaCpp)
    }

    /// Parse an explicit provider name (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "llamacpp" | "llama.cpp" | "llama_cpp" | "local" => Some(Self::LlamaCpp),
            "openai" => Some(Self::OpenAi),
            "azure_openai" | "azureopenai" | "azure" => Some(Self::AzureOpenAi),
            _ => None,
        }
    }

    /// Provider chosen by the explicit `LLM_PROVIDER` environment variable.
    ///
    /// Returns `None` when the variable is absent or unrecognized, so callers
    /// can fall back to their own default instead of silently guessing.
    pub fn from_env() -> Option<Self> {
        std::env::var("LLM_PROVIDER")
            .ok()
            .and_then(|value| Self::parse(&value))
    }
}

/// Environment switch that must be explicitly set to `1` before any cloud LLM
/// provider may be called.
pub const ALLOW_CLOUD_LLM_ENV: &str = "APEX_ALLOW_CLOUD_LLM";

/// True when cloud LLM calls have been explicitly enabled.
pub fn cloud_llm_allowed() -> bool {
    cloud_llm_allowed_value(std::env::var(ALLOW_CLOUD_LLM_ENV).ok().as_deref())
}

fn cloud_llm_allowed_value(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1"))
}

fn provider_allowed(provider: &LlmProvider, allow_cloud: bool) -> Result<()> {
    if provider.is_local() || allow_cloud {
        return Ok(());
    }
    anyhow::bail!(
        "cloud LLM provider '{}' is disabled; set {}=1 to explicitly allow cloud calls",
        provider.as_str(),
        ALLOW_CLOUD_LLM_ENV
    )
}

/// Refuse cloud providers unless `APEX_ALLOW_CLOUD_LLM=1` is set.
///
/// Enforced by [`OpenAiCompatibleClient`] before every request, so an
/// inferred or misconfigured cloud endpoint cannot leak data silently.
pub fn ensure_provider_allowed(provider: &LlmProvider) -> Result<()> {
    provider_allowed(provider, cloud_llm_allowed())
}

/// Per-model configuration including provider, endpoint, and generation parameters.
///
/// # Compatibility notes
///
/// - **llama.cpp**: `api_key` is ignored; set `base_url` to your `llama-server` address.
///   Recommended `temperature`: 0.1–0.3 for structured extraction; 0.4–0.7 for narration.
///   `max_tokens` caps total generated tokens (input + output for some model variants);
///   keep below `n_ctx` set at server startup (default 4096 for Q4_K_M).
/// - **OpenAI**: Set `api_key` from `OPENAI_API_KEY` env var.  `gpt-4o` supports 128k context.
///   `max_tokens` governs output only; cost is metered per input+output token.
/// - **Azure OpenAI**: `base_url` must include the deployment name, e.g.
///   `https://{resource}.openai.azure.com/openai/deployments/{deployment}`.
///   Rotate `api_key` from `AZURE_API_KEY`; never embed in config files.
///
/// Validate before use with [`ModelConfig::validate`]; log safely with
/// [`ModelConfig::redacted_api_key`] (B199).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub model_name: String,
    pub provider: LlmProvider,
    pub base_url: String,
    /// API key stored as `ApiKeySecret` (backed by `SecretString`) which zeroes
    /// memory on drop.  Serializes as `"***REDACTED***"` to prevent leakage.
    /// Use `redacted_api_key()` for safe logging.
    pub api_key: Option<ApiKeySecret>,
    pub max_tokens: u32,
    pub temperature: f64,
    pub timeout_seconds: u32,
}

impl ModelConfig {
    /// Default for llama.cpp server (llama-server) running locally on CPU.
    /// Hetzner EX44: i5-13500, 64 GB RAM, GGUF Q4_K_M (~17 GB).
    pub fn llamacpp_default() -> Self {
        Self {
            model_name: "Qwen3-30B-A3B-Q4_K_M".to_string(),
            provider: LlmProvider::LlamaCpp,
            base_url: "http://localhost:8080".to_string(),
            api_key: None,
            max_tokens: 4096,
            temperature: 0.2,
            timeout_seconds: 300,
        }
    }

    /// Lightweight config — same llama-server instance, shorter output for
    /// fast classification / extraction / simple JSON tasks.
    pub fn llamacpp_lightweight() -> Self {
        Self {
            model_name: "Qwen3-30B-A3B-Q4_K_M".to_string(),
            provider: LlmProvider::LlamaCpp,
            base_url: "http://localhost:8080".to_string(),
            api_key: None,
            max_tokens: 1024,
            temperature: 0.1,
            timeout_seconds: 120,
        }
    }

    pub fn openai_default() -> Self {
        Self {
            model_name: "gpt-4o".to_string(),
            provider: LlmProvider::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: None,
            max_tokens: 4096,
            temperature: 0.2,
            timeout_seconds: 60,
        }
    }

    /// Build a config from an explicit provider plus optional overrides.
    ///
    /// The provider is always taken from `provider` — never inferred from the
    /// URL — so a cloud endpoint cannot be selected implicitly.
    pub fn from_parts(
        provider: LlmProvider,
        base_url: Option<String>,
        model_name: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        let defaults = match provider {
            LlmProvider::LlamaCpp => Self::llamacpp_default(),
            LlmProvider::OpenAi | LlmProvider::AzureOpenAi => Self::openai_default(),
        };
        let non_empty =
            |value: Option<String>| value.filter(|candidate| !candidate.trim().is_empty());
        Self {
            model_name: non_empty(model_name).unwrap_or(defaults.model_name),
            provider,
            base_url: non_empty(base_url).unwrap_or(defaults.base_url),
            api_key: non_empty(api_key).map(ApiKeySecret::from),
            max_tokens: defaults.max_tokens,
            temperature: defaults.temperature,
            timeout_seconds: defaults.timeout_seconds,
        }
    }

    /// Build the provider and endpoint from environment variables.
    ///
    /// The provider comes from the explicit `LLM_PROVIDER` variable
    /// (`llamacpp` | `openai` | `azure_openai`), defaulting to the local
    /// llama.cpp server. `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`
    /// override the provider defaults. Cloud providers remain subject to
    /// [`ensure_provider_allowed`] at call time.
    pub fn from_env() -> Self {
        let provider = LlmProvider::from_env().unwrap_or(LlmProvider::LlamaCpp);
        Self::from_parts(
            provider,
            std::env::var("LLM_BASE_URL").ok(),
            std::env::var("LLM_MODEL").ok(),
            std::env::var("LLM_API_KEY").ok(),
        )
    }

    /// Build chat completions endpoint URL.
    /// Trailing slashes in `base_url` are stripped for consistency (B193).
    pub fn chat_endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.provider {
            LlmProvider::LlamaCpp => format!("{}/v1/chat/completions", base),
            LlmProvider::OpenAi => format!("{}/chat/completions", base),
            LlmProvider::AzureOpenAi => format!("{}/chat/completions", base),
        }
    }

    /// Validate config values. Returns a list of issues (B192, B196).
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.model_name.trim().is_empty() {
            issues.push("model_name must not be empty".to_string()); // B196
        }
        if self.max_tokens == 0 || self.max_tokens > 128_000 {
            issues.push(format!(
                "max_tokens must be 1..=128000, got {}",
                self.max_tokens
            )); // B192
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            issues.push(format!(
                "temperature must be 0.0..=2.0, got {}",
                self.temperature
            )); // B192
        }
        issues
    }

    /// Redact the API key for safe logging (B199).
    pub fn redacted_api_key(&self) -> String {
        match &self.api_key {
            None => "(none)".to_string(),
            Some(k) => {
                let k = k.expose_secret();
                // Count and slice by chars: a non-ASCII key must not panic the
                // redaction helper that exists specifically for safe logging.
                if k.chars().count() <= 8 {
                    "***".to_string()
                } else {
                    let prefix: String = k.chars().take(4).collect();
                    let suffix: String = k
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<Vec<char>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("{}...{}", prefix, suffix)
                }
            }
        }
    }
}

// ────────────────────────────────────────────
// Task Routing
// ────────────────────────────────────────────

/// Maximum LLM response size in bytes before truncation (B206).
pub const MAX_RESPONSE_SIZE: usize = 512_000; // 512 KB

/// Read an HTTP response body with a hard byte cap.
///
/// The configured LLM endpoint is normally trusted infrastructure, but a
/// compromised or spoofed endpoint can return an arbitrarily large body;
/// `Response::json`/`Response::text` would buffer it without bound. Streaming
/// the body and rejecting anything over `limit` keeps memory use bounded even
/// for chunked responses with no `Content-Length`.
pub(crate) async fn read_body_limited(
    mut resp: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len > limit as u64 {
            anyhow::bail!("LLM response body of {len} bytes exceeds the {limit}-byte limit");
        }
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("LLM response body exceeds the {limit}-byte limit");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Routing policy that determines which provider handles each task class.
///
/// `local_only_tasks` are tasks that must **never** call cloud providers —
/// enforced by returning [`ProviderChoice::Unavailable`] when the local model
/// is down.  This protects sensitive data.
///
/// `api_fallback_tasks` prefer local but can call the API when the local
/// model is overloaded or offline.
///
/// All task matching in [`route_task`] is case-insensitive (B191).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingConfig {
    /// Tasks that always use local model (data never leaves premises).
    pub local_only_tasks: Vec<String>,
    /// Tasks that can fall back to API if local is overloaded.
    pub api_fallback_tasks: Vec<String>,
    /// Maximum concurrent API requests.
    pub max_api_concurrent: u32,
    /// Monthly API spend cap in USD.
    pub monthly_api_budget_usd: f64,
    /// Allowed provider types. If non-empty, only listed providers may be used (B208).
    #[serde(default)]
    pub allowed_providers: Vec<LlmProvider>,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            local_only_tasks: vec![
                "poi_synthesis".to_string(),
                "entity_extraction".to_string(),
                "competitive_analysis".to_string(),
            ],
            api_fallback_tasks: vec![
                "recipe_hypothesis".to_string(),
                "memo_generation".to_string(),
                "narrative_rendering".to_string(),
            ],
            max_api_concurrent: 5,
            monthly_api_budget_usd: 500.0,
            allowed_providers: vec![], // empty = all allowed
        }
    }
}

impl RoutingConfig {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.max_api_concurrent == 0 {
            issues.push("max_api_concurrent must be > 0".to_string());
        }
        if !self.monthly_api_budget_usd.is_finite() || self.monthly_api_budget_usd < 0.0 {
            issues.push(format!(
                "monthly_api_budget_usd must be >= 0.0, got {}",
                self.monthly_api_budget_usd
            ));
        }
        issues
    }
}

/// Determine which provider to route a task to.
/// Task matching is case-insensitive (B191).
pub fn route_task(task: &str, routing: &RoutingConfig, local_available: bool) -> ProviderChoice {
    let task_lower = task.to_lowercase();

    // Local-only tasks never go to cloud
    if routing
        .local_only_tasks
        .iter()
        .any(|t| t.to_lowercase() == task_lower)
    {
        tracing::info!(task = %task, choice = "Local", "route_task: local-only task"); // B195
        return if local_available {
            ProviderChoice::Local
        } else {
            ProviderChoice::Unavailable
        };
    }

    // Fallback-capable tasks prefer local, fall back to API
    if routing
        .api_fallback_tasks
        .iter()
        .any(|t| t.to_lowercase() == task_lower)
    {
        let choice = if local_available {
            ProviderChoice::Local
        } else {
            ProviderChoice::ApiFallback
        };
        tracing::info!(task = %task, ?choice, "route_task: fallback-capable task"); // B195
        return choice;
    }

    // Default: prefer local if available
    let choice = if local_available {
        ProviderChoice::Local
    } else {
        ProviderChoice::ApiFallback
    };
    tracing::info!(task = %task, ?choice, "route_task: default routing"); // B195
    choice
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderChoice {
    Local,
    ApiFallback,
    Unavailable,
}

// ────────────────────────────────────────────
// LLM Config (full system)
// ────────────────────────────────────────────

/// Full LLM configuration for the pipeline: primary, optional fallback, and lightweight model.
///
/// `primary` is used for complex multi-step reasoning (recipe hypothesis, dossier generation).
/// `fallback` is an optional cloud model invoked when the local server is unavailable.
/// `lightweight` targets fast single-step tasks (entity extraction, classification).
/// `routing` controls which tasks may call cloud providers.
///
/// Default configuration targets the local llama-server + OpenAI fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    pub primary: ModelConfig,
    pub fallback: Option<ModelConfig>,
    pub lightweight: Option<ModelConfig>,
    pub routing: RoutingConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            primary: ModelConfig::llamacpp_default(),
            fallback: Some(ModelConfig::openai_default()),
            lightweight: Some(ModelConfig::llamacpp_lightweight()),
            routing: RoutingConfig::default(),
        }
    }
}

// ────────────────────────────────────────────
// Chat message types
// ────────────────────────────────────────────

/// A single chat message with `role` and `content`.
///
/// Roles must be one of `"system"`, `"user"`, or `"assistant"`.
/// Use the named constructors [`ChatMessage::system`], [`ChatMessage::user`],
/// [`ChatMessage::assistant`] rather than constructing directly to avoid typos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// Build messages list for a system + user prompt pair.
pub fn build_messages(system: &str, user: &str) -> Vec<ChatMessage> {
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// Build the request body for an OpenAI-compatible chat completions call.
pub fn build_request_body(
    model: &str,
    messages: &[ChatMessage],
    temperature: f64,
    max_tokens: u32,
    json_mode: bool,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), model.into());
    body.insert(
        "messages".to_string(),
        serde_json::to_value(messages).unwrap_or_else(|error| {
            // Serializing a Vec<ChatMessage> of owned strings is a
            // programmer-constant contract; if it ever breaks, send an empty
            // message list so the remote API rejects the request explicitly
            // (an error response) instead of panicking the worker process.
            tracing::error!(%error, "llm: chat message serialization failed; sending empty message list");
            serde_json::Value::Array(Vec::new())
        }),
    );
    body.insert("temperature".to_string(), temperature.into());
    body.insert("max_tokens".to_string(), max_tokens.into());

    if json_mode {
        let mut response_format = serde_json::Map::new();
        response_format.insert("type".to_string(), "json_object".into());
        body.insert(
            "response_format".to_string(),
            serde_json::Value::Object(response_format),
        );
    }

    serde_json::Value::Object(body)
}

/// Extract the content string from an OpenAI-compatible chat response.
pub fn extract_response_content(body: &serde_json::Value) -> Option<String> {
    body.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

/// Extract usage stats from a response.
pub fn extract_usage(body: &serde_json::Value) -> Option<UsageStats> {
    let usage = body.get("usage")?;
    Some(UsageStats {
        prompt_tokens: u32::try_from(usage.get("prompt_tokens")?.as_u64()?).unwrap_or(u32::MAX),
        completion_tokens: u32::try_from(usage.get("completion_tokens")?.as_u64()?)
            .unwrap_or(u32::MAX),
        total_tokens: u32::try_from(usage.get("total_tokens")?.as_u64()?).unwrap_or(u32::MAX),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl UsageStats {
    /// Estimate cost in USD for OpenAI GPT-4o pricing.
    pub fn estimated_cost_usd(&self) -> f64 {
        // GPT-4o: $2.50/1M input, $10/1M output
        let input_cost = self.prompt_tokens as f64 * 2.5 / 1_000_000.0;
        let output_cost = self.completion_tokens as f64 * 10.0 / 1_000_000.0;
        input_cost + output_cost
    }
}

// ────────────────────────────────────────────
// Spend tracker (in-memory)
// ────────────────────────────────────────────

pub struct SpendTracker {
    month_spend_usd: f64,
    budget_usd: f64,
    request_count: u32,
}

impl SpendTracker {
    pub fn new(budget_usd: f64) -> Self {
        Self {
            month_spend_usd: 0.0,
            budget_usd,
            request_count: 0,
        }
    }

    pub fn record(&mut self, cost_usd: f64) {
        self.month_spend_usd += cost_usd;
        self.request_count += 1;

        let utilization = self.budget_utilization();
        tracing::info!(
            month_spend_usd = self.month_spend_usd,
            remaining_budget_usd = self.remaining_budget(),
            utilization_pct = utilization * 100.0,
            request_count = self.request_count,
            "llm_api_budget_usage"
        );
        if utilization >= 1.0 {
            tracing::warn!(
                month_spend_usd = self.month_spend_usd,
                budget_usd = self.budget_usd,
                "llm_api_budget_exceeded"
            );
        } else if utilization >= 0.8 {
            tracing::warn!(
                month_spend_usd = self.month_spend_usd,
                budget_usd = self.budget_usd,
                "llm_api_budget_nearing_limit"
            );
        }
    }

    pub fn within_budget(&self) -> bool {
        self.month_spend_usd < self.budget_usd
    }

    pub fn remaining_budget(&self) -> f64 {
        (self.budget_usd - self.month_spend_usd).max(0.0)
    }

    pub fn month_spend(&self) -> f64 {
        self.month_spend_usd
    }

    pub fn request_count(&self) -> u32 {
        self.request_count
    }

    pub fn budget_utilization(&self) -> f64 {
        if self.budget_usd <= 0.0 {
            1.0
        } else {
            (self.month_spend_usd / self.budget_usd).max(0.0)
        }
    }

    pub fn reset(&mut self) {
        self.month_spend_usd = 0.0;
        self.request_count = 0;
    }
}

// ────────────────────────────────────────────
// LLM Client trait
// ────────────────────────────────────────────

/// Flatten a multi-turn [`crate::inference::ChatMessage`] conversation into a
/// single `(system, user)` prompt pair for single-turn backends.
///
/// System messages are joined in order; every other turn is rendered as a
/// role-labelled transcript block so turn boundaries survive the flattening.
/// Deterministic: the same messages always produce the same prompt bytes.
fn flatten_messages(messages: &[crate::inference::ChatMessage]) -> (String, String) {
    use crate::inference::Role;

    let mut system = String::new();
    let mut user = String::new();
    for message in messages {
        match message.role {
            Role::System => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&message.content);
            }
            Role::Assistant => {
                if !user.is_empty() {
                    user.push_str("\n\n");
                }
                user.push_str("[assistant]\n");
                user.push_str(&message.content);
            }
            Role::User => {
                if !user.is_empty() {
                    user.push_str("\n\n");
                }
                user.push_str("[user]\n");
                user.push_str(&message.content);
            }
        }
    }
    (system, user)
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String>;
    async fn generate_text(&self, system: &str, user: &str) -> Result<String>;

    /// Multi-turn chat completion: send the full conversation (system + user +
    /// assistant + tool-feedback turns) and return the model's text.
    ///
    /// [`crate::function_calling::run_agent_loop`] requires this method so a
    /// trait object (`&dyn LlmClient`, `Box<dyn LlmClient>`,
    /// `Arc<dyn LlmClient>`) — the shape the worker holds — can run the
    /// tool-calling loop. Implementors with a native chat-completions endpoint
    /// must override it and send `messages` verbatim.
    ///
    /// The default implementation is a conservative single-turn fallback for
    /// backends that only expose [`LlmClient::generate_text`]: it flattens the
    /// conversation into one role-labelled transcript and forwards it. It
    /// cannot honour `config` (the single-prompt methods take no config) and
    /// loses native role boundaries, so multi-turn-capable clients must
    /// override it.
    async fn complete_messages(
        &self,
        messages: Vec<crate::inference::ChatMessage>,
        _config: &crate::inference::InferenceConfig,
    ) -> Result<String> {
        let (system, user) = flatten_messages(&messages);
        self.generate_text(&system, &user).await
    }
}

/// OpenAI-compatible client (works with OpenAI, llama.cpp).
pub struct OpenAiCompatibleClient {
    config: ModelConfig,
    /// `None` when HTTP client construction failed: the client is degraded and
    /// every call returns an explicit error instead of panicking the process.
    http: Option<reqwest::Client>,
}

impl OpenAiCompatibleClient {
    pub fn new(config: ModelConfig) -> Self {
        let timeout = std::time::Duration::from_secs(config.timeout_seconds as u64);
        let connect_timeout = std::time::Duration::from_secs(10);
        // Operator-configured LLM endpoint (trusted infrastructure), not crawled content.
        #[allow(clippy::disallowed_methods)]
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .build()
            .map_err(|error| {
                tracing::error!(
                    %error,
                    "llm: failed to build HTTP client; client is degraded and will fail calls"
                );
                error
            })
            .ok();
        Self { config, http }
    }

    /// True when the underlying HTTP client could not be constructed. Callers
    /// may surface this so an operator sees the LLM path is degraded.
    pub fn is_degraded(&self) -> bool {
        self.http.is_none()
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    async fn call(&self, system: &str, user: &str, json_mode: bool) -> Result<String> {
        // Inject /no_think for Qwen3 to suppress chain-of-thought tokens
        let system_with_nothink = if system.ends_with("/no_think") {
            system.to_string()
        } else {
            format!("{}\n/no_think", system)
        };
        let messages = build_messages(&system_with_nothink, user);
        let body = build_request_body(
            &self.config.model_name,
            &messages,
            self.config.temperature,
            self.config.max_tokens,
            json_mode,
        );
        self.post_chat_body(&body).await
    }

    /// POST a prepared chat-completions body with the shared retry policy.
    ///
    /// Cloud providers are refused unless the operator opted in explicitly; a
    /// degraded instance (no HTTP client) fails explicitly instead of
    /// reporting a successful call.
    async fn post_chat_body(&self, body: &serde_json::Value) -> Result<String> {
        crate::ensure_provider_allowed(&self.config.provider)?;
        let Some(http) = self.http.as_ref() else {
            anyhow::bail!(
                "LLM HTTP client is unavailable (construction failed at startup); \
                 refusing to report a successful call"
            );
        };

        let endpoint = self.config.chat_endpoint();

        const MAX_RETRIES: u32 = 3;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let backoff_ms = 500 * 2u64.pow(attempt - 1); // 500ms, 1s, 2s
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }

            let mut req = http.post(&endpoint).json(body);
            if let Some(ref key) = self.config.api_key {
                req = req.header("Authorization", format!("Bearer {}", key.expose_secret()));
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_connect() || e.is_timeout() => {
                    last_err = Some(e.into());
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();

            // Retry on 429 (rate limit), 500, 502, 503, 504 (transient server errors) — B200
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
                || status == reqwest::StatusCode::BAD_GATEWAY
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                || status == reqwest::StatusCode::GATEWAY_TIMEOUT
            {
                tracing::warn!(status = %status, attempt, "LLM API transient error, retrying");
                last_err = Some(anyhow::anyhow!("LLM API returned {}", status));
                continue;
            }

            let resp_body_bytes = read_body_limited(resp, MAX_RESPONSE_SIZE).await?;
            let resp_body: serde_json::Value = serde_json::from_slice(&resp_body_bytes)
                .map_err(|error| anyhow::anyhow!("Invalid JSON in LLM response: {error}"))?;

            if !status.is_success() {
                let error_msg = resp_body
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                anyhow::bail!("LLM API error ({}): {}", status, error_msg);
            }

            // Strip Qwen3 <think>...</think> tags from the response
            let raw = extract_response_content(&resp_body)
                .ok_or_else(|| anyhow::anyhow!("No content in LLM response"))?;
            return Ok(crate::inference::strip_think_tags(&raw));
        }

        Err(last_err
            .unwrap_or_else(|| anyhow::anyhow!("LLM call failed after {} retries", MAX_RETRIES)))
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
        self.call(system, user, true).await
    }

    async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
        self.call(system, user, false).await
    }

    /// Multi-turn chat completion: every message is sent verbatim.
    ///
    /// The endpoint and model name are fixed at construction (operator
    /// environment); `config` contributes sampling parameters, JSON mode, seed,
    /// `/no_think` suppression, and the optional structural-determinism
    /// repeat.
    async fn complete_messages(
        &self,
        messages: Vec<crate::inference::ChatMessage>,
        config: &crate::inference::InferenceConfig,
    ) -> Result<String> {
        let errors = config.validate();
        if !errors.is_empty() {
            anyhow::bail!("Invalid InferenceConfig: {}", errors.join("; "));
        }

        let messages = if config.suppress_thinking {
            crate::inference::inject_no_think(messages)
        } else {
            messages
        };

        let wire_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|message| ChatMessage {
                role: message.role.to_string(),
                content: message.content.clone(),
            })
            .collect();

        let mut body = build_request_body(
            &self.config.model_name,
            &wire_messages,
            f64::from(config.temperature),
            config.max_tokens,
            config.json_mode,
        );
        if let Some(seed) = config.seed {
            if let Some(object) = body.as_object_mut() {
                object.insert("seed".to_string(), seed.into());
            }
        }

        let first = self.post_chat_body(&body).await?;
        if config.enforce_structural_determinism && config.json_mode && config.temperature == 0.0 {
            let repeated = self.post_chat_body(&body).await?;
            crate::inference::assert_structural_determinism(&first, &repeated)?;
        }
        Ok(first)
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_llm_provider_as_str() {
        assert_eq!(LlmProvider::LlamaCpp.as_str(), "llamacpp");
        assert_eq!(LlmProvider::OpenAi.as_str(), "openai");
        assert_eq!(LlmProvider::AzureOpenAi.as_str(), "azure_openai");
    }

    #[test]
    fn test_llm_provider_is_local() {
        assert!(LlmProvider::LlamaCpp.is_local());
        assert!(!LlmProvider::OpenAi.is_local());
        assert!(!LlmProvider::AzureOpenAi.is_local());
    }

    #[test]
    fn test_model_config_llamacpp() {
        let cfg = ModelConfig::llamacpp_default();
        assert_eq!(cfg.model_name, "Qwen3-30B-A3B-Q4_K_M");
        assert_eq!(cfg.provider, LlmProvider::LlamaCpp);
        assert_eq!(cfg.max_tokens, 4096);
        assert!((cfg.temperature - 0.2).abs() < 0.01);
        assert_eq!(cfg.timeout_seconds, 300);
    }

    #[test]
    fn test_model_config_llamacpp_lightweight() {
        let cfg = ModelConfig::llamacpp_lightweight();
        assert_eq!(cfg.model_name, "Qwen3-30B-A3B-Q4_K_M");
        assert_eq!(cfg.provider, LlmProvider::LlamaCpp);
        assert_eq!(cfg.max_tokens, 1024);
        assert!((cfg.temperature - 0.1).abs() < 0.01);
        assert_eq!(cfg.timeout_seconds, 120);
    }

    #[test]
    fn test_model_config_openai() {
        let cfg = ModelConfig::openai_default();
        assert_eq!(cfg.model_name, "gpt-4o");
        assert_eq!(cfg.provider, LlmProvider::OpenAi);
    }

    #[test]
    fn test_llm_provider_parse_is_case_insensitive() {
        assert_eq!(LlmProvider::parse("LLAMACPP"), Some(LlmProvider::LlamaCpp));
        assert_eq!(LlmProvider::parse("llama.cpp"), Some(LlmProvider::LlamaCpp));
        assert_eq!(LlmProvider::parse("OpenAI"), Some(LlmProvider::OpenAi));
        assert_eq!(
            LlmProvider::parse("azure_openai"),
            Some(LlmProvider::AzureOpenAi)
        );
        assert_eq!(LlmProvider::parse("azure"), Some(LlmProvider::AzureOpenAi));
        assert_eq!(LlmProvider::parse("anthropic"), None);
    }

    #[test]
    fn test_cloud_provider_gate() {
        // Local providers never need the opt-in.
        assert!(provider_allowed(&LlmProvider::LlamaCpp, false).is_ok());
        // Cloud providers are refused without the explicit opt-in ...
        assert!(provider_allowed(&LlmProvider::OpenAi, false).is_err());
        assert!(provider_allowed(&LlmProvider::AzureOpenAi, false).is_err());
        // ... and allowed with it.
        assert!(provider_allowed(&LlmProvider::OpenAi, true).is_ok());
        assert!(provider_allowed(&LlmProvider::AzureOpenAi, true).is_ok());

        assert!(cloud_llm_allowed_value(Some("1")));
        assert!(cloud_llm_allowed_value(Some(" 1 ")));
        assert!(!cloud_llm_allowed_value(Some("true")));
        assert!(!cloud_llm_allowed_value(Some("0")));
        assert!(!cloud_llm_allowed_value(None));
    }

    #[test]
    fn test_model_config_from_parts_uses_explicit_provider() {
        let cloud = ModelConfig::from_parts(
            LlmProvider::OpenAi,
            None,
            Some("gpt-4o-mini".to_string()),
            Some("sk-test".to_string()),
        );
        assert_eq!(cloud.provider, LlmProvider::OpenAi);
        assert_eq!(cloud.model_name, "gpt-4o-mini");
        assert!(cloud.api_key.is_some());
        // Blank overrides fall back to provider defaults.
        let local = ModelConfig::from_parts(
            LlmProvider::LlamaCpp,
            Some("  ".to_string()),
            Some(String::new()),
            None,
        );
        assert_eq!(local.provider, LlmProvider::LlamaCpp);
        assert_eq!(local.base_url, "http://localhost:8080");
        assert_eq!(local.model_name, "Qwen3-30B-A3B-Q4_K_M");
        assert!(local.api_key.is_none());
    }

    #[test]
    fn test_chat_endpoint_llamacpp() {
        let cfg = ModelConfig::llamacpp_default();
        assert_eq!(
            cfg.chat_endpoint(),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_endpoint_openai() {
        let cfg = ModelConfig::openai_default();
        assert_eq!(
            cfg.chat_endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_endpoint_llamacpp_lightweight() {
        let cfg = ModelConfig::llamacpp_lightweight();
        assert_eq!(
            cfg.chat_endpoint(),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn test_routing_config_default() {
        let cfg = RoutingConfig::default();
        assert_eq!(cfg.local_only_tasks.len(), 3);
        assert!(cfg.local_only_tasks.contains(&"poi_synthesis".to_string()));
        assert!(cfg
            .local_only_tasks
            .contains(&"entity_extraction".to_string()));
        assert!(cfg
            .local_only_tasks
            .contains(&"competitive_analysis".to_string()));
        assert_eq!(cfg.api_fallback_tasks.len(), 3);
        assert_eq!(cfg.max_api_concurrent, 5);
        assert!((cfg.monthly_api_budget_usd - 500.0).abs() < 0.01);
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn test_routing_config_validate_rejects_zero_max_api_concurrent() {
        let cfg = RoutingConfig {
            max_api_concurrent: 0,
            ..Default::default()
        };
        let issues = cfg.validate();
        assert!(issues.iter().any(|m| m.contains("max_api_concurrent")));
    }

    #[test]
    fn test_routing_config_validate_rejects_negative_budget() {
        let cfg = RoutingConfig {
            monthly_api_budget_usd: -1.0,
            ..Default::default()
        };
        let issues = cfg.validate();
        assert!(issues.iter().any(|m| m.contains("monthly_api_budget_usd")));
    }

    #[test]
    fn test_route_task_local_only_available() {
        let routing = RoutingConfig::default();
        let choice = route_task("poi_synthesis", &routing, true);
        assert_eq!(choice, ProviderChoice::Local);
    }

    #[test]
    fn test_route_task_local_only_unavailable() {
        let routing = RoutingConfig::default();
        let choice = route_task("entity_extraction", &routing, false);
        assert_eq!(choice, ProviderChoice::Unavailable);
    }

    #[test]
    fn test_route_task_fallback_local_available() {
        let routing = RoutingConfig::default();
        let choice = route_task("memo_generation", &routing, true);
        assert_eq!(choice, ProviderChoice::Local);
    }

    #[test]
    fn test_route_task_fallback_local_unavailable() {
        let routing = RoutingConfig::default();
        let choice = route_task("narrative_rendering", &routing, false);
        assert_eq!(choice, ProviderChoice::ApiFallback);
    }

    #[test]
    fn test_route_task_unknown_task_defaults_with_local_available() {
        let routing = RoutingConfig::default();
        let choice = route_task("totally_unknown_task", &routing, true);
        assert_eq!(choice, ProviderChoice::Local);
    }

    #[test]
    fn test_route_task_unknown_task_defaults_with_local_unavailable() {
        let routing = RoutingConfig::default();
        let choice = route_task("totally_unknown_task", &routing, false);
        assert_eq!(choice, ProviderChoice::ApiFallback);
    }

    #[test]
    fn test_route_task_unknown_task() {
        let routing = RoutingConfig::default();
        assert_eq!(
            route_task("custom_task", &routing, true),
            ProviderChoice::Local
        );
        assert_eq!(
            route_task("custom_task", &routing, false),
            ProviderChoice::ApiFallback
        );
    }

    #[test]
    fn test_llm_config_default() {
        let cfg = LlmConfig::default();
        assert_eq!(cfg.primary.provider, LlmProvider::LlamaCpp);
        assert!(cfg.fallback.is_some());
        assert!(
            matches!(cfg.fallback.as_ref(), Some(config) if config.provider == LlmProvider::OpenAi)
        );
        assert!(cfg.lightweight.is_some());
        assert!(
            matches!(cfg.lightweight.as_ref(), Some(config) if config.provider == LlmProvider::LlamaCpp)
        );
        assert!(matches!(cfg.lightweight.as_ref(), Some(config) if config.max_tokens == 1024));
    }

    #[test]
    fn test_chat_message_constructors() {
        let sys = ChatMessage::system("You are a helpful assistant.");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content, "You are a helpful assistant.");

        let usr = ChatMessage::user("Hello");
        assert_eq!(usr.role, "user");

        let asst = ChatMessage::assistant("Hi there!");
        assert_eq!(asst.role, "assistant");
    }

    #[test]
    fn test_build_messages() {
        let msgs = build_messages("sys prompt", "user prompt");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys prompt");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "user prompt");
    }

    #[test]
    fn test_build_request_body_text() {
        let msgs = build_messages("sys", "usr");
        let body = build_request_body("gpt-4o", &msgs, 0.3, 4096, false);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn test_build_request_body_json_mode() {
        let msgs = build_messages("sys", "usr");
        let body = build_request_body("gpt-4o", &msgs, 0.2, 2048, true);
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_extract_response_content_success() {
        let resp = serde_json::json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "Hello, world!"
                    }
                }
            ]
        });
        assert_eq!(
            extract_response_content(&resp),
            Some("Hello, world!".to_string())
        );
    }

    #[test]
    fn test_extract_response_content_missing() {
        let resp = serde_json::json!({"error": "bad request"});
        assert_eq!(extract_response_content(&resp), None);
    }

    #[test]
    fn test_extract_response_content_empty_choices() {
        let resp = serde_json::json!({"choices": []});
        assert_eq!(extract_response_content(&resp), None);
    }

    #[test]
    fn test_extract_usage() {
        let resp = serde_json::json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        });
        let usage = extract_usage(&resp).unwrap_or_else(|| panic!("usage should be present"));
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn test_extract_usage_missing() {
        let resp = serde_json::json!({"choices": []});
        assert!(extract_usage(&resp).is_none());
    }

    #[test]
    fn test_usage_estimated_cost() {
        let usage = UsageStats {
            prompt_tokens: 1000,
            completion_tokens: 500,
            total_tokens: 1500,
        };
        let cost = usage.estimated_cost_usd();
        // 1000 * 2.5/1M + 500 * 10/1M = 0.0025 + 0.005 = 0.0075
        assert!((cost - 0.0075).abs() < 0.0001);
    }

    #[test]
    fn test_spend_tracker() {
        let mut tracker = SpendTracker::new(100.0);
        assert!(tracker.within_budget());
        assert!((tracker.remaining_budget() - 100.0).abs() < 0.01);
        assert_eq!(tracker.request_count(), 0);

        tracker.record(30.0);
        assert!(tracker.within_budget());
        assert!((tracker.remaining_budget() - 70.0).abs() < 0.01);
        assert_eq!(tracker.request_count(), 1);

        tracker.record(80.0);
        assert!(!tracker.within_budget());
        assert!((tracker.remaining_budget() - 0.0).abs() < 0.01);
        assert_eq!(tracker.request_count(), 2);
    }

    #[test]
    fn test_spend_tracker_reset() {
        let mut tracker = SpendTracker::new(100.0);
        tracker.record(50.0);
        tracker.reset();
        assert!(tracker.within_budget());
        assert!((tracker.month_spend() - 0.0).abs() < 0.01);
        assert_eq!(tracker.request_count(), 0);
    }

    #[test]
    fn test_spend_tracker_budget_utilization() {
        let mut tracker = SpendTracker::new(200.0);
        assert!((tracker.budget_utilization() - 0.0).abs() < 1e-9);
        tracker.record(50.0);
        assert!((tracker.budget_utilization() - 0.25).abs() < 1e-9);
        tracker.record(150.0);
        assert!((tracker.budget_utilization() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_config_serialization() {
        let cfg = LlmConfig::default();
        let json = serde_json::to_string(&cfg)
            .unwrap_or_else(|error| panic!("LLM config should serialize: {error}"));
        let parsed: LlmConfig = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("LLM config should deserialize: {error}"));
        assert_eq!(parsed.primary.model_name, cfg.primary.model_name);
        assert_eq!(parsed.primary.provider, cfg.primary.provider);
    }

    #[test]
    fn test_model_config_rejects_unknown_fields() {
        let mut value = serde_json::to_value(ModelConfig::openai_default())
            .unwrap_or_else(|error| panic!("model config should serialize: {error}"));
        value["unused_field"] = serde_json::Value::Bool(true);
        let err = serde_json::from_value::<ModelConfig>(value)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"));
        assert!(err.contains("unused_field"));
    }

    #[test]
    fn test_routing_config_rejects_unknown_fields() {
        let mut value = serde_json::to_value(RoutingConfig::default())
            .unwrap_or_else(|error| panic!("routing config should serialize: {error}"));
        value["unexpected"] = serde_json::Value::String("x".to_string());
        let err = serde_json::from_value::<RoutingConfig>(value)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"));
        assert!(err.contains("unexpected"));
    }

    #[test]
    fn test_llm_config_rejects_unknown_fields() {
        let mut value = serde_json::to_value(LlmConfig::default())
            .unwrap_or_else(|error| panic!("LLM config should serialize: {error}"));
        value["mystery"] = serde_json::Value::Bool(true);
        let err = serde_json::from_value::<LlmConfig>(value)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"));
        assert!(err.contains("mystery"));
    }

    // ── B191: case-insensitive route_task ────────────────────────
    #[test]
    fn test_route_task_case_insensitive() {
        let routing = RoutingConfig::default();
        assert_eq!(
            route_task("POI_SYNTHESIS", &routing, true),
            ProviderChoice::Local
        );
        assert_eq!(
            route_task("Entity_Extraction", &routing, false),
            ProviderChoice::Unavailable
        );
        assert_eq!(
            route_task("MEMO_GENERATION", &routing, true),
            ProviderChoice::Local
        );
        assert_eq!(
            route_task("Narrative_Rendering", &routing, false),
            ProviderChoice::ApiFallback
        );
    }

    // ── B192: validate max_tokens and temperature ────────────────
    #[test]
    fn test_model_config_validate_valid() {
        let cfg = ModelConfig::llamacpp_default();
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn test_model_config_validate_zero_tokens() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.max_tokens = 0;
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("max_tokens")));
    }

    #[test]
    fn test_model_config_validate_temp_out_of_range() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.temperature = 3.0;
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("temperature")));
    }

    #[test]
    fn test_model_config_validate_negative_temp() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.temperature = -0.1;
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("temperature")));
    }

    // ── B193: trailing slash in chat_endpoint ────────────────────
    #[test]
    fn test_chat_endpoint_trailing_slash() {
        let mut cfg = ModelConfig::openai_default();
        cfg.base_url = "https://api.openai.com/v1/".to_string();
        assert_eq!(
            cfg.chat_endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_endpoint_multiple_trailing_slashes() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.base_url = "http://localhost:8080///".to_string();
        assert_eq!(
            cfg.chat_endpoint(),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    // ── B194: timeout per provider ───────────────────────────────
    #[test]
    fn test_timeout_per_provider() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.timeout_seconds = 600;
        let client = OpenAiCompatibleClient::new(cfg);
        assert_eq!(client.config().timeout_seconds, 600);

        let cfg2 = ModelConfig::openai_default();
        let client2 = OpenAiCompatibleClient::new(cfg2);
        assert_eq!(client2.config().timeout_seconds, 60);
    }

    // ── B196: empty model_name ───────────────────────────────────
    #[test]
    fn test_model_config_validate_empty_name() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.model_name = "".to_string();
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("model_name")));
    }

    #[test]
    fn test_model_config_validate_whitespace_name() {
        let mut cfg = ModelConfig::llamacpp_default();
        cfg.model_name = "   ".to_string();
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("model_name")));
    }

    // ── B197: build_request_body json mode ───────────────────────
    #[test]
    fn test_build_request_body_json_mode_has_format() {
        let msgs = build_messages("s", "u");
        let body = build_request_body("model", &msgs, 0.2, 1024, true);
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_build_request_body_no_json_mode_no_format() {
        let msgs = build_messages("s", "u");
        let body = build_request_body("model", &msgs, 0.2, 1024, false);
        assert!(body.get("response_format").is_none());
    }

    // ── B198: malformed responses ────────────────────────────────
    #[test]
    fn test_extract_response_content_no_message() {
        let resp = serde_json::json!({"choices": [{"index": 0}]});
        assert_eq!(extract_response_content(&resp), None);
    }

    #[test]
    fn test_extract_response_content_content_is_number() {
        let resp = serde_json::json!({"choices": [{"message": {"content": 42}}]});
        assert_eq!(extract_response_content(&resp), None);
    }

    #[test]
    fn test_extract_response_content_null_content() {
        let resp = serde_json::json!({"choices": [{"message": {"content": null}}]});
        assert_eq!(extract_response_content(&resp), None);
    }

    // ── B199: redacted API key ───────────────────────────────────
    #[test]
    fn test_redacted_api_key_none() {
        let cfg = ModelConfig::llamacpp_default();
        assert_eq!(cfg.redacted_api_key(), "(none)");
    }

    #[test]
    fn test_redacted_api_key_short() {
        let mut cfg = ModelConfig::openai_default();
        cfg.api_key = Some(ApiKeySecret::from("abc"));
        assert_eq!(cfg.redacted_api_key(), "***");
    }

    #[test]
    fn test_redacted_api_key_long() {
        let mut cfg = ModelConfig::openai_default();
        cfg.api_key = Some(ApiKeySecret::from("sk-1234567890abcdef"));
        let redacted = cfg.redacted_api_key();
        assert!(redacted.starts_with("sk-1"));
        assert!(redacted.ends_with("cdef"));
        assert!(redacted.contains("..."));
    }

    // Audit: redaction is used for safe logging and must not panic on a key
    // containing non-ASCII characters. The old byte slicing panicked at a
    // non-char-boundary index.
    #[test]
    fn test_redacted_api_key_multibyte_no_panic() {
        let mut cfg = ModelConfig::openai_default();
        cfg.api_key = Some(ApiKeySecret::from("aéééééééééé"));
        let redacted = cfg.redacted_api_key();
        assert!(redacted.starts_with("aééé"));
        assert!(redacted.contains("..."));
        assert_ne!(redacted, "aéééééééééé");

        // Short multi-byte keys are fully redacted rather than sliced.
        cfg.api_key = Some(ApiKeySecret::from("aéééé"));
        assert_eq!(cfg.redacted_api_key(), "***");
    }

    // ── B206: MAX_RESPONSE_SIZE constant ─────────────────────────
    #[test]
    fn test_max_response_size_constant() {
        assert_eq!(MAX_RESPONSE_SIZE, 512_000);
    }

    // ── Multi-turn trait method ──────────────────────────────────

    #[test]
    fn flatten_messages_preserves_turn_order_and_roles() {
        let messages = vec![
            crate::inference::ChatMessage::system("sys one"),
            crate::inference::ChatMessage::system("sys two"),
            crate::inference::ChatMessage::user("task"),
            crate::inference::ChatMessage::assistant("{\"name\":\"echo\"}"),
            crate::inference::ChatMessage::user("[tool_result] {}"),
        ];
        let (system, user) = flatten_messages(&messages);
        assert_eq!(system, "sys one\n\nsys two");
        let task = user.find("[user]\ntask").expect("task turn present");
        let assistant = user
            .find("[assistant]\n{\"name\":\"echo\"}")
            .expect("assistant turn present");
        let tool = user
            .find("[user]\n[tool_result] {}")
            .expect("tool-feedback turn present");
        assert!(
            task < assistant && assistant < tool,
            "flattening must preserve turn order: {user}"
        );
    }

    struct DefaultOnlyClient;

    #[async_trait]
    impl LlmClient for DefaultOnlyClient {
        async fn generate_json(&self, _system: &str, _user: &str) -> Result<String> {
            Ok("{}".to_string())
        }

        async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
            Ok(format!("SYS={system}|USER={user}"))
        }
    }

    #[tokio::test]
    async fn complete_messages_default_forwards_flattened_prompt_to_generate_text() {
        let client: &dyn LlmClient = &DefaultOnlyClient;
        let messages = vec![
            crate::inference::ChatMessage::system("sys"),
            crate::inference::ChatMessage::user("task"),
            crate::inference::ChatMessage::assistant("thinking"),
            crate::inference::ChatMessage::user("[tool_result] {}"),
        ];
        let text = client
            .complete_messages(messages, &crate::inference::InferenceConfig::default())
            .await
            .expect("default complete_messages must forward to generate_text");
        assert_eq!(
            text,
            "SYS=sys|USER=[user]\ntask\n\n[assistant]\nthinking\n\n[user]\n[tool_result] {}"
        );
    }

    async fn spawn_raw_http_response(
        response: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test listener should bind: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test listener address: {error}"));
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // Drain the request headers first. Closing a socket while the
                // peer's request is still unread can trigger a TCP RST that
                // discards the response, making the test flaky.
                let mut received = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            received.extend_from_slice(&buf[..n]);
                            if received.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (addr, server)
    }

    /// Serve exactly one HTTP request and return the captured request body.
    async fn spawn_capturing_http_response(
        response: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test listener should bind: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test listener address: {error}"));
        let server = tokio::spawn(async move {
            let mut captured = String::new();
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    if let Some(header_end) =
                        buf.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        let body_start = header_end + 4;
                        while buf.len() < body_start + content_length {
                            match socket.read(&mut chunk).await {
                                Ok(0) => break,
                                Ok(read) => buf.extend_from_slice(&chunk[..read]),
                                Err(_) => break,
                            }
                        }
                        let body_end = (body_start + content_length).min(buf.len());
                        captured = String::from_utf8_lossy(&buf[body_start..body_end]).to_string();
                        break;
                    }
                    match socket.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(read) => buf.extend_from_slice(&chunk[..read]),
                        Err(_) => break,
                    }
                }
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
            captured
        });
        (addr, server)
    }

    // Audit: a valid, small response must still parse after the bounded-body
    // refactor.
    #[tokio::test]
    async fn small_response_body_is_parsed() {
        let body = r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let (addr, server) = spawn_raw_http_response(response).await;

        // Local provider: these tests exercise body handling, not cloud access.
        let mut config = ModelConfig::llamacpp_default();
        config.base_url = format!("http://{addr}");
        config.timeout_seconds = 5;
        let client = OpenAiCompatibleClient::new(config);
        let generated = client
            .generate_text("system", "user")
            .await
            .unwrap_or_else(|error| panic!("valid response should parse: {error}"));
        assert_eq!(generated, "{\"ok\":true}");
        server.abort();
    }

    // The production implementor (`OpenAiCompatibleClient`, the type the worker
    // builds behind `Arc<dyn LlmClient>`) must send every conversation turn to
    // the endpoint, not a flattened single prompt.
    #[tokio::test]
    async fn complete_messages_sends_all_turns_for_production_client() {
        let response_body =
            r#"{"choices":[{"message":{"content":"final answer"},"finish_reason":"stop"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let (addr, server) = spawn_capturing_http_response(response).await;

        // Local provider: this test exercises the multi-turn body, not cloud access.
        let mut config = ModelConfig::llamacpp_default();
        config.base_url = format!("http://{addr}");
        config.timeout_seconds = 5;
        let model_name = config.model_name.clone();
        let client = OpenAiCompatibleClient::new(config);

        let messages = vec![
            crate::inference::ChatMessage::system("You are an analyst."),
            crate::inference::ChatMessage::user("Find the company."),
            crate::inference::ChatMessage::assistant(r#"{"name":"lookup","arguments":{}}"#),
            crate::inference::ChatMessage::user("[tool_result] {}"),
        ];
        let inference_config = crate::inference::InferenceConfig {
            temperature: 0.2,
            max_tokens: 512,
            suppress_thinking: true,
            ..Default::default()
        };

        let text = client
            .complete_messages(messages, &inference_config)
            .await
            .unwrap_or_else(|error| panic!("multi-turn completion should succeed: {error}"));
        assert_eq!(text, "final answer");

        let captured = server
            .await
            .unwrap_or_else(|error| panic!("server task should join: {error}"));
        let body: serde_json::Value = serde_json::from_str(&captured)
            .unwrap_or_else(|error| panic!("captured body should be JSON: {error}\n{captured}"));
        assert_eq!(body["model"], model_name);
        assert_eq!(body["max_tokens"], 512);
        assert!(
            body["temperature"]
                .as_f64()
                .is_some_and(|value| (value - 0.2).abs() < 1e-6),
            "temperature must come from the per-call config: {}",
            body["temperature"]
        );
        let sent = body["messages"]
            .as_array()
            .unwrap_or_else(|| panic!("messages must be an array: {}", body["messages"]));
        let roles: Vec<&str> = sent
            .iter()
            .map(|message| message["role"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
        assert_eq!(
            sent[2]["content"], r#"{"name":"lookup","arguments":{}}"#,
            "assistant turn must be sent verbatim"
        );
        assert_eq!(sent[3]["content"], "[tool_result] {}");
        assert!(
            sent[0]["content"]
                .as_str()
                .is_some_and(|system| system.ends_with("/no_think")),
            "suppress_thinking must append /no_think to the system turn: {}",
            sent[0]["content"]
        );
    }

    // Audit: MAX_RESPONSE_SIZE must actually cap the bytes buffered from the
    // endpoint. A hostile/compromised server returning a chunked body larger
    // than the limit must be rejected, not buffered without bound.
    #[tokio::test]
    async fn oversized_response_body_is_rejected() {
        let chunk = "x".repeat(100_000);
        let chunks = (MAX_RESPONSE_SIZE / chunk.len()) + 2;
        let mut body = String::from("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n");
        for _ in 0..chunks {
            body.push_str(&format!("{:x}\r\n{}\r\n", chunk.len(), chunk));
        }
        body.push_str("0\r\n\r\n");
        let (addr, server) = spawn_raw_http_response(body).await;

        // Local provider: these tests exercise body handling, not cloud access.
        let mut config = ModelConfig::llamacpp_default();
        config.base_url = format!("http://{addr}");
        config.timeout_seconds = 5;
        let client = OpenAiCompatibleClient::new(config);
        let error = client
            .generate_text("system", "user")
            .await
            .expect_err("oversized response body must be rejected");
        assert!(
            error.to_string().contains("limit"),
            "unexpected error: {error}"
        );
        server.abort();
    }

    // ── B208: allowed_providers config ───────────────────────────
    #[test]
    fn test_routing_config_allowed_providers_default_empty() {
        let cfg = RoutingConfig::default();
        assert!(cfg.allowed_providers.is_empty());
    }

    #[test]
    fn test_routing_config_allowed_providers_restricted() {
        let cfg = RoutingConfig {
            allowed_providers: vec![LlmProvider::LlamaCpp],
            ..Default::default()
        };
        assert_eq!(cfg.allowed_providers.len(), 1);
        assert!(cfg.allowed_providers.contains(&LlmProvider::LlamaCpp));
        assert!(!cfg.allowed_providers.contains(&LlmProvider::OpenAi));
    }
}
