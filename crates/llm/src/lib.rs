//! Model-agnostic LLM client — trait, provider implementations, routing.
//!
//! Supports multiple providers: llama.cpp (local Qwen3-Next-80B-A3B on CPU),
//! OpenAI, Azure.  Deployed on a Hetzner EX44 (i5-13500, 64 GB RAM, no GPU)
//! using GGUF Q4_K_M quantisation via llama-server.  Primary and lightweight
//! tiers both hit the same llama-server instance — no Ollama wrapper overhead.
//! All logic is testable without external services; async HTTP is sealed behind
//! the trait boundary.

pub mod validators;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Provider & Config
// ────────────────────────────────────────────

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model_name: String,
    pub provider: LlmProvider,
    pub base_url: String,
    pub api_key: Option<String>,
    pub max_tokens: u32,
    pub temperature: f64,
    pub timeout_seconds: u32,
}

impl ModelConfig {
    /// Default for llama.cpp server (llama-server) running locally on CPU.
    /// Hetzner EX44: i5-13500, 64 GB RAM, GGUF Q4_K_M (~45 GB).
    pub fn llamacpp_default() -> Self {
        Self {
            model_name: "Qwen3-Next-80B-A3B-Instruct-Q4_K_M".to_string(),
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
            model_name: "Qwen3-Next-80B-A3B-Instruct-Q4_K_M".to_string(),
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

    /// Build chat completions endpoint URL.
    pub fn chat_endpoint(&self) -> String {
        match self.provider {
            LlmProvider::LlamaCpp => format!("{}/v1/chat/completions", self.base_url),
            LlmProvider::OpenAi => format!("{}/chat/completions", self.base_url),
            LlmProvider::AzureOpenAi => format!("{}/chat/completions", self.base_url),
        }
    }
}

// ────────────────────────────────────────────
// Task Routing
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Tasks that always use local model (data never leaves premises).
    pub local_only_tasks: Vec<String>,
    /// Tasks that can fall back to API if local is overloaded.
    pub api_fallback_tasks: Vec<String>,
    /// Maximum concurrent API requests.
    pub max_api_concurrent: u32,
    /// Monthly API spend cap in USD.
    pub monthly_api_budget_usd: f64,
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
        }
    }
}

/// Determine which provider to route a task to.
pub fn route_task(task: &str, routing: &RoutingConfig, local_available: bool) -> ProviderChoice {
    // Local-only tasks never go to cloud
    if routing.local_only_tasks.iter().any(|t| t == task) {
        return if local_available {
            ProviderChoice::Local
        } else {
            ProviderChoice::Unavailable
        };
    }

    // Fallback-capable tasks prefer local, fall back to API
    if routing.api_fallback_tasks.iter().any(|t| t == task) {
        return if local_available {
            ProviderChoice::Local
        } else {
            ProviderChoice::ApiFallback
        };
    }

    // Default: prefer local if available
    if local_available {
        ProviderChoice::Local
    } else {
        ProviderChoice::ApiFallback
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
    });

    if json_mode {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }

    body
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
        prompt_tokens: usage.get("prompt_tokens")?.as_u64()? as u32,
        completion_tokens: usage.get("completion_tokens")?.as_u64()? as u32,
        total_tokens: usage.get("total_tokens")?.as_u64()? as u32,
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

    pub fn reset(&mut self) {
        self.month_spend_usd = 0.0;
        self.request_count = 0;
    }
}

// ────────────────────────────────────────────
// LLM Client trait
// ────────────────────────────────────────────

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String>;
    async fn generate_text(&self, system: &str, user: &str) -> Result<String>;
}

/// OpenAI-compatible client (works with OpenAI, llama.cpp).
pub struct OpenAiCompatibleClient {
    config: ModelConfig,
    http: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub fn new(config: ModelConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds as u64))
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    async fn call(&self, system: &str, user: &str, json_mode: bool) -> Result<String> {
        let messages = build_messages(system, user);
        let body = build_request_body(
            &self.config.model_name,
            &messages,
            self.config.temperature,
            self.config.max_tokens,
            json_mode,
        );

        let endpoint = self.config.chat_endpoint();
        let mut req = self.http.post(&endpoint).json(&body);

        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await?;
        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let error_msg = resp_body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("LLM API error ({}): {}", status, error_msg);
        }

        extract_response_content(&resp_body)
            .ok_or_else(|| anyhow::anyhow!("No content in LLM response"))
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
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        assert_eq!(cfg.model_name, "Qwen3-Next-80B-A3B-Instruct-Q4_K_M");
        assert_eq!(cfg.provider, LlmProvider::LlamaCpp);
        assert_eq!(cfg.max_tokens, 4096);
        assert!((cfg.temperature - 0.2).abs() < 0.01);
        assert_eq!(cfg.timeout_seconds, 300);
    }

    #[test]
    fn test_model_config_llamacpp_lightweight() {
        let cfg = ModelConfig::llamacpp_lightweight();
        assert_eq!(cfg.model_name, "Qwen3-Next-80B-A3B-Instruct-Q4_K_M");
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
    fn test_chat_endpoint_llamacpp() {
        let cfg = ModelConfig::llamacpp_default();
        assert_eq!(cfg.chat_endpoint(), "http://localhost:8080/v1/chat/completions");
    }

    #[test]
    fn test_chat_endpoint_openai() {
        let cfg = ModelConfig::openai_default();
        assert_eq!(cfg.chat_endpoint(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_chat_endpoint_llamacpp_lightweight() {
        let cfg = ModelConfig::llamacpp_lightweight();
        assert_eq!(cfg.chat_endpoint(), "http://localhost:8080/v1/chat/completions");
    }

    #[test]
    fn test_routing_config_default() {
        let cfg = RoutingConfig::default();
        assert_eq!(cfg.local_only_tasks.len(), 3);
        assert!(cfg.local_only_tasks.contains(&"poi_synthesis".to_string()));
        assert!(cfg.local_only_tasks.contains(&"entity_extraction".to_string()));
        assert!(cfg.local_only_tasks.contains(&"competitive_analysis".to_string()));
        assert_eq!(cfg.api_fallback_tasks.len(), 3);
        assert_eq!(cfg.max_api_concurrent, 5);
        assert!((cfg.monthly_api_budget_usd - 500.0).abs() < 0.01);
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
    fn test_route_task_unknown_task() {
        let routing = RoutingConfig::default();
        assert_eq!(route_task("custom_task", &routing, true), ProviderChoice::Local);
        assert_eq!(route_task("custom_task", &routing, false), ProviderChoice::ApiFallback);
    }

    #[test]
    fn test_llm_config_default() {
        let cfg = LlmConfig::default();
        assert_eq!(cfg.primary.provider, LlmProvider::LlamaCpp);
        assert!(cfg.fallback.is_some());
        assert_eq!(cfg.fallback.as_ref().unwrap().provider, LlmProvider::OpenAi);
        assert!(cfg.lightweight.is_some());
        assert_eq!(cfg.lightweight.as_ref().unwrap().provider, LlmProvider::LlamaCpp);
        assert_eq!(cfg.lightweight.as_ref().unwrap().max_tokens, 1024);
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
        assert_eq!(extract_response_content(&resp), Some("Hello, world!".to_string()));
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
        let usage = extract_usage(&resp).unwrap();
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
    fn test_config_serialization() {
        let cfg = LlmConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: LlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.primary.model_name, cfg.primary.model_name);
        assert_eq!(parsed.primary.provider, cfg.primary.provider);
    }
}
