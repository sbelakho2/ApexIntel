//! HTTP inference client for the local llama-server (OpenAI-compatible API).
//!
//! Provides a thin, production-ready wrapper around the `/v1/chat/completions`
//! endpoint with:
//! - Configurable retry with exponential back-off
//! - Structured JSON output via `response_format: { type: "json_object" }`
//! - Think/no-think mode control via `/no_think` suffix (Qwen3 specific)
//! - Token budget enforcement
//! - Latency tracing
//! - Full error typing (no raw strings leaking into callers)
//!
//! # Quick start
//!
//! ```rust,ignore
//! use apex_llm::inference::{LlmClient, ChatMessage, Role};
//!
//! let client = LlmClient::from_env()?;
//! let resp = client.complete(vec![
//!     ChatMessage { role: Role::System, content: "You are an OSINT analyst.".into() },
//!     ChatMessage { role: Role::User,   content: "Summarise risk for Foxconn Q2 2025.".into() },
//! ]).await?;
//! println!("{}", resp.text);
//! ```

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Role in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::System => write!(f, "system"),
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
        }
    }
}

/// A single turn in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Inference configuration knobs.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    /// Model identifier sent in the request body.
    pub model: String,
    /// Sampling temperature.  Range `[0.0, 2.0]`.  Default `0.2`.
    pub temperature: f32,
    /// Maximum tokens to generate.  Default `2048`.
    pub max_tokens: u32,
    /// If true, wrap the request in `response_format: json_object` mode.
    pub json_mode: bool,
    /// Optional provider-supported seed for deterministic sampling.
    pub seed: Option<u64>,
    /// If true, append `/no_think` to the system prompt to suppress CoT tokens
    /// (Qwen3-specific).  Reduces latency significantly on short tasks.
    pub suppress_thinking: bool,
    /// If true, repeat structured calls and assert key fields remain stable.
    pub enforce_structural_determinism: bool,
    /// Maximum retries on transient errors.  Default `3`.
    pub max_retries: u32,
    /// Base delay for exponential back-off.  Default `1 s`.
    pub retry_base_delay: Duration,
    /// Timeout for the full request.  Default `120 s`.
    pub timeout: Duration,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            model: "Qwen3-30B-A3B-Q4_K_M".into(),
            temperature: 0.2,
            max_tokens: 2048,
            json_mode: false,
            seed: None,
            suppress_thinking: true,
            enforce_structural_determinism: false,
            max_retries: 3,
            retry_base_delay: Duration::from_secs(1),
            timeout: Duration::from_secs(120),
        }
    }
}

impl InferenceConfig {
    /// Create a config tuned for fast, factual extraction tasks.
    pub fn fast_extraction() -> Self {
        Self {
            temperature: 0.0,
            max_tokens: 512,
            suppress_thinking: true,
            ..Default::default()
        }
    }

    /// Create a config tuned for structured JSON output.
    pub fn json_structured() -> Self {
        Self {
            temperature: 0.0,
            max_tokens: 1024,
            json_mode: true,
            seed: Some(0),
            suppress_thinking: true,
            enforce_structural_determinism: true,
            ..Default::default()
        }
    }

    /// Create a config tuned for creative narrative generation.
    pub fn narrative() -> Self {
        Self {
            temperature: 0.4,
            max_tokens: 2048,
            suppress_thinking: false,
            ..Default::default()
        }
    }

    /// Create a config for deep reasoning tasks (thinking ON, larger budget).
    pub fn deep_reasoning() -> Self {
        Self {
            temperature: 0.6,
            max_tokens: 4096,
            suppress_thinking: false,
            ..Default::default()
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if !(0.0..=2.0).contains(&(self.temperature as f64)) {
            errs.push(format!("temperature {} out of [0, 2]", self.temperature));
        }
        if self.max_tokens == 0 {
            errs.push("max_tokens must be > 0".into());
        }
        if self.max_retries > 10 {
            errs.push("max_retries > 10 is unreasonable".into());
        }
        errs
    }
}

/// The raw result of a successful completion.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// The model's text output (thinking tokens stripped if present).
    pub text: String,
    /// Prompt tokens consumed.
    pub prompt_tokens: u32,
    /// Completion tokens consumed.
    pub completion_tokens: u32,
    /// Total tokens consumed.
    pub total_tokens: u32,
    /// Wall-clock latency of the full HTTP round-trip.
    pub latency: Duration,
    /// Stable hash of the fully prepared prompt payload.
    pub prompt_hash: String,
    /// Stable hash of the returned text payload.
    pub response_hash: String,
}

impl CompletionResponse {
    /// Parse the text as JSON, stripping any remaining think tags first.
    pub fn parse_json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        let clean = strip_think_tags(&self.text);
        let extracted = crate::validators::extract_json(&clean)
            .ok_or_else(|| anyhow!("No JSON found in LLM response"))?;
        serde_json::from_str(&extracted).with_context(|| {
            format!(
                "Failed to deserialize JSON from LLM response: {}",
                crate::truncate_utf8(&extracted, 200)
            )
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wire types — OpenAI API request/response
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Deserialize)]
struct ChatResponse {
    usage: Option<UsageInfo>,
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct UsageInfo {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// HTTP client for the local llama-server.
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    pub default_config: InferenceConfig,
}

impl LlmClient {
    /// Construct a client from explicit parameters.
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        config: InferenceConfig,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_else(|error| panic!("failed to build reqwest client: {error}"));
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            default_config: config,
        }
    }

    /// Load client from environment variables:
    /// - `LLM_BASE_URL` (required, e.g. `http://localhost:8080`)
    /// - `LLM_API_KEY`  (optional)
    /// - `LLM_MODEL`    (optional, default `Qwen3-30B-A3B-Q4_K_M`)
    pub fn from_env() -> Result<Self> {
        let base_url =
            std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
        let api_key = std::env::var("LLM_API_KEY").ok();
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
        let config = InferenceConfig {
            model,
            ..Default::default()
        };
        Ok(Self::new(base_url, api_key, config))
    }

    /// Send a chat completion request with the given messages and config.
    pub async fn complete_with_config(
        &self,
        messages: Vec<ChatMessage>,
        config: &InferenceConfig,
    ) -> Result<CompletionResponse> {
        let errors = config.validate();
        if !errors.is_empty() {
            bail!("Invalid InferenceConfig: {}", errors.join("; "));
        }

        // Optionally append /no_think to suppress thinking tokens (Qwen3).
        let messages = if config.suppress_thinking {
            inject_no_think(messages)
        } else {
            messages
        };

        let prompt_hash = hash_messages(&messages);
        let response = self.complete_once(&messages, config, &prompt_hash).await?;

        if config.enforce_structural_determinism && config.json_mode && config.temperature == 0.0 {
            let repeated = self.complete_once(&messages, config, &prompt_hash).await?;
            assert_structural_determinism(&response.text, &repeated.text)?;
        }

        Ok(response)
    }

    async fn complete_once(
        &self,
        messages: &[ChatMessage],
        config: &InferenceConfig,
        prompt_hash: &str,
    ) -> Result<CompletionResponse> {
        let response_format = if config.json_mode {
            Some(ResponseFormat {
                kind: "json_object",
            })
        } else {
            None
        };

        let body = ChatRequest {
            model: &config.model,
            messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            seed: config.seed,
            response_format,
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut last_err = anyhow!("No retries attempted");

        for attempt in 0..=config.max_retries {
            if attempt > 0 {
                let delay = config.retry_base_delay * 2u32.pow(attempt - 1);
                let delay = delay.min(Duration::from_secs(30));
                warn!(attempt, delay=?delay, "LLM request failed, retrying...");
                sleep(delay).await;
            }

            let t0 = Instant::now();
            let mut req = self.http.post(&url).json(&body);
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let result = req.send().await;
            match result {
                Err(e) => {
                    last_err = anyhow!("HTTP send error on attempt {}: {}", attempt, e);
                    continue;
                }
                Ok(resp) => {
                    let status = resp.status();
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
                    {
                        last_err = anyhow!("HTTP {} on attempt {}", status, attempt);
                        continue;
                    }
                    if !status.is_success() {
                        let body_text = resp.text().await.unwrap_or_default();
                        bail!(
                            "HTTP {} (non-retryable): {}",
                            status,
                            crate::truncate_utf8(&body_text, 500)
                        );
                    }

                    let latency = t0.elapsed();
                    let chat_resp: ChatResponse = resp
                        .json()
                        .await
                        .with_context(|| "Failed to deserialize OpenAI response")?;

                    let raw_text = chat_resp
                        .choices
                        .into_iter()
                        .next()
                        .and_then(|c| c.message.content)
                        .unwrap_or_default();

                    let text = strip_think_tags(&raw_text);
                    let response_hash = hash_text(&text);

                    let usage = chat_resp.usage.unwrap_or(UsageInfo {
                        prompt_tokens: Some(0),
                        completion_tokens: Some(0),
                        total_tokens: Some(0),
                    });

                    debug!(
                        latency_ms = latency.as_millis(),
                        total_tokens = usage.total_tokens.unwrap_or(0),
                        %prompt_hash,
                        %response_hash,
                        seed = ?config.seed,
                        "LLM inference complete"
                    );
                    info!(%prompt_hash, %response_hash, seed = ?config.seed, "LLM prompt/response hashed");

                    return Ok(CompletionResponse {
                        text,
                        prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                        completion_tokens: usage.completion_tokens.unwrap_or(0),
                        total_tokens: usage.total_tokens.unwrap_or(0),
                        latency,
                        prompt_hash: prompt_hash.to_string(),
                        response_hash,
                    });
                }
            }
        }

        Err(last_err)
    }

    /// Complete with the client's default configuration.
    pub async fn complete(&self, messages: Vec<ChatMessage>) -> Result<CompletionResponse> {
        self.complete_with_config(messages, &self.default_config.clone())
            .await
    }

    /// Convenience: single-turn user prompt with default config.
    pub async fn prompt(&self, user_message: impl Into<String>) -> Result<String> {
        let msgs = vec![ChatMessage::user(user_message)];
        let resp = self.complete(msgs).await?;
        Ok(resp.text)
    }

    /// Convenience: system + user prompt, returns parsed JSON of type T.
    pub async fn extract_json<T: serde::de::DeserializeOwned>(
        &self,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<T> {
        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.complete_with_config(messages, &config).await?;
        resp.parse_json::<T>()
    }

    /// Health check — returns true if the LLM server is reachable and responding.
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        self.http
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Strip `<think>…</think>` blocks from Qwen3 output.
/// Multiple blocks are stripped; content after the last `</think>` is returned.
pub fn strip_think_tags(text: &str) -> String {
    // Fast path: no think block present.
    if !text.contains("<think>") {
        return text.trim().to_string();
    }

    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";

    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut cursor = 0usize;

    while cursor < text.len() {
        let open_idx = text[cursor..].find(OPEN).map(|idx| cursor + idx);
        let close_idx = text[cursor..].find(CLOSE).map(|idx| cursor + idx);

        let Some((next_idx, is_open)) = (match (open_idx, close_idx) {
            (Some(open), Some(close)) if open <= close => Some((open, true)),
            (Some(_open), Some(close)) => Some((close, false)),
            (Some(open), None) => Some((open, true)),
            (None, Some(close)) => Some((close, false)),
            (None, None) => None,
        }) else {
            if depth == 0 {
                out.push_str(&text[cursor..]);
            }
            break;
        };

        if is_open {
            if depth == 0 {
                out.push_str(&text[cursor..next_idx]);
            }
            depth += 1;
            cursor = next_idx + OPEN.len();
            continue;
        }

        if depth > 0 {
            depth -= 1;
        } else {
            out.push_str(&text[cursor..(next_idx + CLOSE.len())]);
        }
        cursor = next_idx + CLOSE.len();
    }

    out.trim().to_string()
}

/// Inject `/no_think` into the last system message (or add a new one) so the
/// Qwen3 model skips its chain-of-thought reasoning block.
fn inject_no_think(mut messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    // Find an existing system message and append the directive.
    for msg in &mut messages {
        if msg.role == Role::System {
            if !msg.content.ends_with("/no_think") {
                msg.content.push_str("\n/no_think");
            }
            return messages;
        }
    }
    // No system message — prepend one.
    messages.insert(
        0,
        ChatMessage::system("You are a precise intelligence analysis assistant.\n/no_think"),
    );
    messages
}

fn hash_text(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_messages(messages: &[ChatMessage]) -> String {
    let payload = messages
        .iter()
        .map(|message| format!("{}:{}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n");
    hash_text(&payload)
}

fn structural_signature(text: &str) -> Result<Option<(String, String)>> {
    let clean = strip_think_tags(text);
    let extracted = crate::validators::extract_json(&clean)
        .ok_or_else(|| anyhow!("No JSON found in deterministic response"))?;
    let value: serde_json::Value = serde_json::from_str(&extracted)
        .with_context(|| "Failed to parse deterministic JSON response")?;
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    match (object.get("severity"), object.get("category")) {
        (Some(severity), Some(category)) => Ok(Some((
            severity.as_str().unwrap_or_default().to_string(),
            category.as_str().unwrap_or_default().to_string(),
        ))),
        _ => Ok(None),
    }
}

fn assert_structural_determinism(first: &str, second: &str) -> Result<()> {
    let first_signature = structural_signature(first)?;
    let second_signature = structural_signature(second)?;
    if let (Some(first_signature), Some(second_signature)) = (first_signature, second_signature) {
        if first_signature != second_signature {
            bail!(
                "json_structured structural determinism violated: first={:?}, second={:?}",
                first_signature,
                second_signature
            );
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn strip_think_tags_removes_block() {
        let raw = "<think>internal reasoning</think>\nActual answer here.";
        assert_eq!(strip_think_tags(raw), "Actual answer here.");
    }

    #[test]
    fn strip_think_tags_nested() {
        let raw = "<think>outer <think>inner</think> outer</think>real content";
        assert_eq!(strip_think_tags(raw), "real content");
    }

    #[test]
    fn strip_think_tags_no_think() {
        let raw = "Just a regular response.";
        assert_eq!(strip_think_tags(raw), "Just a regular response.");
    }

    #[test]
    fn strip_think_tags_preserves_utf8() {
        let raw = "<think>分析中</think>Résumé café — 東京";
        assert_eq!(strip_think_tags(raw), "Résumé café — 東京");
    }

    #[test]
    fn inject_no_think_appends_to_system() {
        let messages = vec![
            ChatMessage::system("You are an analyst."),
            ChatMessage::user("Summarize X."),
        ];
        let result = inject_no_think(messages);
        assert!(result[0].content.ends_with("/no_think"));
    }

    #[test]
    fn inject_no_think_creates_system_if_absent() {
        let messages = vec![ChatMessage::user("Hello")];
        let result = inject_no_think(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, Role::System);
        assert!(result[0].content.contains("/no_think"));
    }

    #[test]
    fn inference_config_validate_ok() {
        let cfg = InferenceConfig::default();
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn json_structured_enables_seed_and_determinism() {
        let cfg = InferenceConfig::json_structured();
        assert_eq!(cfg.seed, Some(0));
        assert!(cfg.enforce_structural_determinism);
    }

    #[test]
    fn inference_config_validate_bad_temperature() {
        let cfg = InferenceConfig {
            temperature: 5.0,
            ..Default::default()
        };
        assert!(!cfg.validate().is_empty());
    }

    #[test]
    fn inference_config_validate_zero_tokens() {
        let cfg = InferenceConfig {
            max_tokens: 0,
            ..Default::default()
        };
        assert!(!cfg.validate().is_empty());
    }

    #[test]
    fn from_env_uses_defaults_when_env_absent() {
        // Serialize env var mutations to prevent data races in parallel tests.
        let guard = ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| panic!("env test lock should not be poisoned: {error}"));
        // Clear env vars to test defaults.
        std::env::remove_var("LLM_BASE_URL");
        std::env::remove_var("LLM_API_KEY");
        // Drop the guard explicitly before re-acquiring the lock.
        // `let`-shadowing evaluates the RHS before dropping the old binding,
        // which would deadlock because std::sync::Mutex is not reentrant.
        drop(guard);
        let _guard = ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| panic!("env test lock should not be poisoned: {error}"));
        let client = LlmClient::from_env()
            .unwrap_or_else(|error| panic!("client should load default env config: {error}"));
        assert_eq!(client.base_url, "http://localhost:8080");
        assert!(client.api_key.is_none());
    }

    #[test]
    fn completion_response_parse_json_works() {
        let resp = CompletionResponse {
            text: r#"{"score": 0.9, "label": "risk"}"#.into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            latency: Duration::from_millis(100),
            prompt_hash: "prompt".into(),
            response_hash: "response".into(),
        };
        let v: serde_json::Value = resp
            .parse_json()
            .unwrap_or_else(|error| panic!("completion JSON should parse: {error}"));
        assert_eq!(v["score"], 0.9);
    }

    #[test]
    fn completion_response_parse_json_strips_fences() {
        let resp = CompletionResponse {
            text: "```json\n{\"k\": 1}\n```".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            latency: Duration::from_secs(0),
            prompt_hash: "prompt".into(),
            response_hash: "response".into(),
        };
        let v: serde_json::Value = resp
            .parse_json()
            .unwrap_or_else(|error| panic!("fenced completion JSON should parse: {error}"));
        assert_eq!(v["k"], 1);
    }

    #[test]
    fn structural_determinism_accepts_matching_signatures() {
        let first = r#"{"severity":"warning","category":"supply_chain"}"#;
        let second = r#"{"severity":"warning","category":"supply_chain"}"#;
        assert!(assert_structural_determinism(first, second).is_ok());
    }

    #[test]
    fn structural_determinism_rejects_mismatch() {
        let first = r#"{"severity":"critical","category":"financial"}"#;
        let second = r#"{"severity":"warning","category":"financial"}"#;
        assert!(assert_structural_determinism(first, second).is_err());
    }
}
