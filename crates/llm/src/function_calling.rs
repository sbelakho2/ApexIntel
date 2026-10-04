//! Experimental: OpenAI-compatible function / tool calling (B290).
//!
//! Enabled only when compiled with the `experimental` Cargo feature.  The API
//! follows the OpenAI "tools" schema (June 2023 API update).
//!
//! # Provider compatibility
//!
//! | Provider     | Support level      | Notes                                     |
//! |-------------|-------------------|------------------------------------------|
//! | `OpenAi`    | Full               | Parallel tool calls, streaming            |
//! | `AzureOpenAi` | Full             | Same as OpenAI (Azure passes through)    |
//! | `LlamaCpp`  | Limited            | Use JSON-mode + manual parsing as alt     |
//!
//! # Stability notice
//!
//! This module is **experimental** and its types may change without a semver
//! major bump while the feature flag is active.  Stabilisation requires:
//! - Confirmed production usage across all three providers.
//! - A streaming variant of [`FunctionCall`] (SSE token-by-token response).
//! - Parallel / multi-turn tool-call support.
//!
//! # Example
//!
//! ```rust,ignore
//! use apex_llm::function_calling::{FunctionSpec, ParamSchema};
//! use std::collections::HashMap;
//!
//! let mut params = HashMap::new();
//! params.insert("entity_id".to_string(), ParamSchema {
//!     kind: "string".to_string(),
//!     description: Some("UUID of the entity to look up".to_string()),
//!     enum_values: None,
//! });
//!
//! let spec = FunctionSpec {
//!     name: "get_entity".to_string(),
//!     description: "Retrieve entity details by ID".to_string(),
//!     parameters: params,
//!     required: vec!["entity_id".to_string()],
//! };
//!
//! assert!(spec.validate().is_empty());
//! ```

use crate::inference::ChatMessage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

// ─────────────────────────────────────────────────────────────────────────────
// Schema types
// ─────────────────────────────────────────────────────────────────────────────

/// Allowed JSON-Schema primitive types for function parameters.
///
/// Mirrors the `"type"` field in the OpenAI tools schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
}

impl ParamType {
    /// Returns the JSON-Schema string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

impl std::fmt::Display for ParamType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// JSON-Schema–based description of a single function parameter.
///
/// Serialises to the `properties` entry of the OpenAI tool-spec `parameters`
/// object (which is itself a JSON Schema object with `type: "object"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSchema {
    /// JSON Schema primitive type for this parameter.
    #[serde(rename = "type")]
    pub kind: ParamType,

    /// Human-readable description shown to the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// If set, constrains the model to one of these values (enum).
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// FunctionSpec
// ─────────────────────────────────────────────────────────────────────────────

/// A callable function that the LLM may invoke (OpenAI "tool" schema).
///
/// Serialises directly into the `"function"` field of the OpenAI
/// `tools: [{"type": "function", "function": {...}}]` array.
///
/// Call [`FunctionSpec::validate`] before sending to the API to catch
/// schema errors (missing required-field declarations, etc.) early.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    /// Machine-readable function name — must match `[a-zA-Z0-9_-]{1,64}`.
    pub name: String,

    /// Plain-English description for the model to understand when to call this function.
    pub description: String,

    /// Parameter schema keyed by parameter name.
    pub parameters: HashMap<String, ParamSchema>,

    /// Names of parameters that must be supplied by the model.
    pub required: Vec<String>,
}

impl FunctionSpec {
    /// Validate the spec for internal consistency.
    ///
    /// Returns a list of error strings; an empty list means the spec is valid.
    ///
    /// # Checks performed
    /// - Every name in `required` appears as a key in `parameters`.
    /// - `name` is non-empty and contains only `[a-zA-Z0-9_-]` characters.
    /// - At least one parameter is declared when `required` is non-empty.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.name.is_empty() {
            errors.push("FunctionSpec.name must not be empty".to_string());
        } else if !self
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            errors.push(format!(
                "FunctionSpec.name `{}` contains invalid characters (allow: [a-zA-Z0-9_-])",
                self.name
            ));
        }

        for req in &self.required {
            if !self.parameters.contains_key(req.as_str()) {
                errors.push(format!(
                    "required param `{}` is not declared in FunctionSpec.parameters",
                    req
                ));
            }
        }

        if !self.required.is_empty() && self.parameters.is_empty() {
            errors.push(
                "FunctionSpec.parameters is empty but required fields are declared".to_string(),
            );
        }

        errors
    }

    /// Serialise this spec into the provider-ready JSON body fragment.
    ///
    /// Returns the `{"type": "function", "function": {...}}` object that
    /// belongs in the `tools` array of an OpenAI chat-completions request.
    pub fn to_tool_json(&self) -> serde_json::Value {
        let properties = self
            .parameters
            .iter()
            .map(|(k, v)| {
                let val = serde_json::to_value(v).unwrap_or_else(|e| {
                    warn!(param=%k, error=%e, "Failed to serialize function parameter schema, using null fallback");
                    serde_json::Value::Null
                });
                (k.clone(), val)
            })
            .collect::<serde_json::Map<_, _>>();

        let mut parameters = serde_json::Map::new();
        parameters.insert("type".to_string(), "object".into());
        parameters.insert(
            "properties".to_string(),
            serde_json::Value::Object(properties),
        );
        parameters.insert(
            "required".to_string(),
            serde_json::to_value(&self.required).unwrap_or_else(|error| {
                panic!("function required fields should serialize: {error}")
            }),
        );

        let mut function = serde_json::Map::new();
        function.insert("name".to_string(), self.name.clone().into());
        function.insert("description".to_string(), self.description.clone().into());
        function.insert(
            "parameters".to_string(),
            serde_json::Value::Object(parameters),
        );

        let mut tool = serde_json::Map::new();
        tool.insert("type".to_string(), "function".into());
        tool.insert("function".to_string(), serde_json::Value::Object(function));
        serde_json::Value::Object(tool)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FunctionCall (model response)
// ─────────────────────────────────────────────────────────────────────────────

/// A function-call decision returned by the LLM in its completion.
///
/// Extracted from the `tool_calls[*].function` field of the OpenAI response.
/// Callers should always call [`FunctionCall::parse_arguments`] rather than
/// accessing `arguments` directly, to get proper deserialisation errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// Name of the function the model decided to invoke.
    pub name: String,

    /// Arguments chosen by the model, as a JSON object.
    pub arguments: serde_json::Value,
}

impl FunctionCall {
    /// Attempt to parse `arguments` into a concrete type `T`.
    ///
    /// # Errors
    /// Returns [`serde_json::Error`] if the JSON shape does not match `T`.
    pub fn parse_arguments<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.arguments.clone())
    }

    /// Validate that the arguments satisfy the declared [`FunctionSpec`].
    ///
    /// Checks that every field in `spec.required` is present in `arguments`.
    /// Returns a list of missing field names (empty list = valid).
    pub fn validate_against(&self, spec: &FunctionSpec) -> Vec<String> {
        let args = match self.arguments.as_object() {
            Some(obj) => obj,
            None => {
                return vec!["FunctionCall.arguments is not a JSON object".to_string()];
            }
        };
        spec.required
            .iter()
            .filter(|req| !args.contains_key(req.as_str()))
            .cloned()
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool execution framework
// ─────────────────────────────────────────────────────────────────────────────
// Previously the function-calling layer could DECLARE tools and PARSE the model's
// `tool_calls`, but it could never EXECUTE them — the loop was open. This adds
// a real tool registry + dispatcher + agent loop so the LLM can autonomously
// enrich, score, and draft by invoking registered tools, with results fed back
// until the model produces a final answer (or the iteration budget is exhausted).

use async_trait::async_trait;

/// A single tool the LLM agent may invoke.
///
/// Implementations own whatever side-effecting client they need (a store handle,
/// an HTTP client, a contact enricher, …) and execute synchronously-ish under
/// `execute`. The returned JSON is fed back to the model as the tool result.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The function specification the model sees (name + JSON-schema params).
    fn spec(&self) -> FunctionSpec;

    /// Execute the tool with the model-supplied arguments. Returns a JSON value
    /// that becomes the `tool` role message content in the next LLM turn.
    ///
    /// Errors are surfaced to the model as the tool result so it can recover
    /// (the agent loop wraps this), not propagated as a hard failure.
    async fn execute(&self, arguments: &serde_json::Value) -> Result<serde_json::Value, ToolError>;
}

/// A typed tool-execution error — serialized into the conversation so the model
/// can reason about why a tool failed and retry or abandon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ToolError {
    UnknownTool { name: String },
    InvalidArguments { name: String, errors: Vec<String> },
    ExecutionFailed { name: String, message: String },
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::UnknownTool { name } => write!(f, "unknown tool: {name}"),
            ToolError::InvalidArguments { name, errors } => {
                write!(f, "invalid arguments for {name}: {}", errors.join("; "))
            }
            ToolError::ExecutionFailed { name, message } => {
                write!(f, "tool {name} failed: {message}")
            }
        }
    }
}

impl std::error::Error for ToolError {}

/// A registry of named tools the agent may invoke.
///
/// Tools are stored in a `BTreeMap` so [`ToolRegistry::tool_specs_json`] emits a
/// stable, name-sorted catalog. Prompt hashing depends on byte-identical
/// prompts, and a `HashMap` iteration order would vary with its random seed.
pub struct ToolRegistry {
    tools: std::collections::BTreeMap<String, std::sync::Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::collections::BTreeMap::new(),
        }
    }

    /// Register a tool under its spec name.
    pub fn register(&mut self, tool: std::sync::Arc<dyn Tool>) {
        let name = tool.spec().name;
        self.tools.insert(name, tool);
    }

    /// The OpenAI `tools` array for all registered tools.
    pub fn tool_specs_json(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|t| t.spec().to_tool_json())
            .collect()
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&std::sync::Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Execute a [`FunctionCall`] by dispatching to the registered tool.
    ///
    /// Validates arguments against the spec first. On any failure returns a
    /// [`ToolError`] the caller can serialize into the conversation.
    pub async fn execute_call(&self, call: &FunctionCall) -> Result<serde_json::Value, ToolError> {
        let tool = match self.get(&call.name) {
            Some(t) => t.clone(),
            None => {
                return Err(ToolError::UnknownTool {
                    name: call.name.clone(),
                });
            }
        };
        let spec = tool.spec();
        let missing = call.validate_against(&spec);
        if !missing.is_empty() {
            return Err(ToolError::InvalidArguments {
                name: call.name.clone(),
                errors: missing,
            });
        }
        tool.execute(&call.arguments)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                name: call.name.clone(),
                message: e.to_string(),
            })
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The outcome of a full agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    /// The model's final text answer (its last non-tool-call completion).
    pub final_answer: String,
    /// How many tool-call iterations ran.
    pub iterations: u32,
    /// Trace of every tool invocation: (name, arguments, result-or-error).
    pub tool_trace: Vec<ToolTraceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTraceEntry {
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: serde_json::Value,
    pub ok: bool,
}

/// Hard cap on the serialized bytes of one tool result fed back to the model.
const MAX_TOOL_RESULT_BYTES: usize = 16_384;

/// Hard cap on the accumulated conversation bytes kept across iterations.
/// Tool results are attacker-influenced (the model may echo untrusted values),
/// so the loop must not grow the request without bound.
const MAX_CONVERSATION_BYTES: usize = 262_144;

/// Hard cap on tool calls executed from a single model response. A crafted or
/// injected response can contain a large `tool_calls` array; only the first
/// [`MAX_TOOL_CALLS_PER_ITERATION`] calls are executed.
const MAX_TOOL_CALLS_PER_ITERATION: usize = 16;

/// Append the registered tool catalog to the system prompt.
///
/// [`crate::inference::LlmClient`]'s chat-completions request carries no
/// `tools` array, so the catalog has to travel in the system prompt; without it
/// the model has no way to know which tools exist and any tool call it emits is
/// guesswork. The catalog is the same JSON shape as the OpenAI `tools` entries
/// (name, description, parameters, required), and the calling convention is
/// stated explicitly.
fn system_prompt_with_tools(system_prompt: &str, tools: &[serde_json::Value]) -> String {
    if tools.is_empty() {
        return system_prompt.to_string();
    }
    let catalog = serde_json::to_string(tools).unwrap_or_else(|error| {
        warn!(
            %error,
            "failed to serialize tool catalog into the system prompt; advertising no tools"
        );
        "[]".to_string()
    });
    format!(
        "{system_prompt}\n\n# Available tools\n\
         To call a tool, reply with a JSON object of the form \
         {{\"name\": \"<tool>\", \"arguments\": {{...}}}} \
         or {{\"tool_calls\": [{{\"name\": \"<tool>\", \"arguments\": {{...}}}}]}}. \
         Tool catalog (JSON): {catalog}"
    )
}

/// Serialize a tool result for the conversation, truncating to
/// [`MAX_TOOL_RESULT_BYTES`] on a UTF-8 boundary.
fn bounded_tool_result_text(value: &serde_json::Value) -> String {
    let text = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"tool result is not serializable\"}".to_string());
    crate::truncate_utf8(&text, MAX_TOOL_RESULT_BYTES).to_string()
}

fn conversation_bytes(messages: &[ChatMessage]) -> usize {
    messages.iter().map(|message| message.content.len()).sum()
}

/// Run a multi-turn agent loop with the given LLM client, tool registry, and
/// task prompt.
///
/// The client is accepted as a trait object, so the production shape
/// (`Arc<dyn apex_llm::LlmClient>` / `Box<dyn apex_llm::LlmClient>`) can be
/// passed directly via `as_ref()`.
///
/// Each iteration: send the conversation (system + user + accumulated tool
/// results) with the tool specs; if the model emits `tool_calls`, execute each
/// and append the results as `tool` messages; if the model emits a plain
/// completion (no tool calls), that's the final answer.
///
/// Stops when the model produces a final answer, `max_iterations` is reached
/// (defaults to 6), or the conversation byte cap is hit. This closes the
/// previously-open function-calling loop.
pub async fn run_agent_loop(
    client: &dyn crate::LlmClient,
    registry: &ToolRegistry,
    system_prompt: &str,
    task: &str,
    max_iterations: u32,
) -> Result<AgentResult, anyhow::Error> {
    use crate::inference::{ChatMessage, InferenceConfig};

    let config = InferenceConfig {
        max_tokens: 1024,
        temperature: 0.2,
        json_mode: false, // free-form so the model can emit tool_calls OR prose
        suppress_thinking: true,
        ..Default::default()
    };

    let tools = registry.tool_specs_json();
    let mut messages: Vec<ChatMessage> = vec![
        ChatMessage::system(system_prompt_with_tools(system_prompt, &tools)),
        ChatMessage::user(task.to_string()),
    ];
    let mut trace: Vec<ToolTraceEntry> = Vec::new();
    let mut iterations = 0u32;
    let mut conversation_len = conversation_bytes(&messages);

    loop {
        if iterations >= max_iterations {
            return Ok(AgentResult {
                final_answer: "(reached max tool-call iterations without a final answer)".into(),
                iterations,
                tool_trace: trace,
            });
        }
        if conversation_len > MAX_CONVERSATION_BYTES {
            return Ok(AgentResult {
                final_answer:
                    "(conversation byte cap reached; return this partial answer for review)".into(),
                iterations,
                tool_trace: trace,
            });
        }
        iterations += 1;

        let text = client.complete_messages(messages.clone(), &config).await?;

        // Parse any tool_calls embedded in the response. The model may emit them
        // as a JSON object/array under a conventional key when not using native
        // tool-calling (llama.cpp path).
        let calls = parse_tool_calls(&text);
        if calls.is_empty() {
            // No tool calls → final answer.
            return Ok(AgentResult {
                final_answer: strip_tool_call_block(&text),
                iterations,
                tool_trace: trace,
            });
        }

        // Append the assistant turn (the raw text including the tool-call block).
        conversation_len += text.len();
        messages.push(ChatMessage::assistant(text));

        // Execute each requested tool and feed the result back. Per-iteration
        // and per-result caps keep an injected `tool_calls` payload from
        // fanning out into unbounded work or conversation growth.
        for call in calls.iter().take(MAX_TOOL_CALLS_PER_ITERATION) {
            let outcome = registry.execute_call(call).await;
            let (result_json, ok) = match outcome {
                Ok(v) => (v, true),
                Err(e) => (
                    serde_json::to_value(&e).unwrap_or(serde_json::json!(null)),
                    false,
                ),
            };
            trace.push(ToolTraceEntry {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: result_json.clone(),
                ok,
            });
            let tool_msg = serde_json::json!({
                "tool": call.name,
                "result": result_json,
            });
            let tool_text = bounded_tool_result_text(&tool_msg);
            let tool_message = format!(
                "[tool_result] {tool_text}\n\nContinue. You may call another tool or give your final answer."
            );
            conversation_len += tool_message.len();
            messages.push(ChatMessage::user(tool_message));
        }
    }
}

/// Parse embedded tool-call JSON from a free-form LLM completion.
///
/// Supports two conventions the local model reliably produces:
///   1. A `tool_calls` array: `{"tool_calls":[{"name":..,"arguments":{..}}]}`
///   2. A single `tool_call` object: `{"name":..,"arguments":{..}}`
fn parse_tool_calls(text: &str) -> Vec<FunctionCall> {
    // Strip thinking tags if present.
    let cleaned = crate::inference::strip_think_tags(text);

    // Try to find the first balanced JSON object in the text.
    let json_str = match extract_first_json(&cleaned) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let value: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // Convention 1: array under tool_calls.
    if let Some(arr) = value.get("tool_calls").and_then(|v| v.as_array()) {
        return arr.iter().filter_map(parse_one_call).collect();
    }
    // Convention 2: a single call at the top level.
    if let Some(call) = parse_one_call(&value) {
        return vec![call];
    }
    Vec::new()
}

fn parse_one_call(value: &serde_json::Value) -> Option<FunctionCall> {
    let name = value.get("name").and_then(|v| v.as_str())?.to_string();
    let arguments = value
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    Some(FunctionCall { name, arguments })
}

/// Extract the first balanced `{ ... }` JSON object from a string.
fn extract_first_json(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        let c = b as char;
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Remove a leading/embedded tool-call JSON block so the final answer is clean
/// prose.
///
/// Only JSON that actually looks like a tool call (a `tool_calls` key or a
/// string `name`) is removed. A final answer that is itself structured JSON —
/// e.g. the recipe hypothesis the agent is asked to produce — must survive
/// intact: [`run_agent_loop`] reaches this function only when
/// [`parse_tool_calls`] found no call, so stripping arbitrary JSON would
/// silently discard valid structured answers.
fn strip_tool_call_block(text: &str) -> String {
    let cleaned = crate::inference::strip_think_tags(text);
    if let Some(json) = extract_first_json(&cleaned) {
        let looks_like_tool_call = serde_json::from_str::<serde_json::Value>(&json)
            .map(|value| {
                value.get("tool_calls").is_some()
                    || value.get("name").and_then(|name| name.as_str()).is_some()
            })
            .unwrap_or(false);
        if looks_like_tool_call {
            // If the whole answer is just the JSON tool-call, return empty (no prose answer).
            let trimmed = cleaned.replace(&json, "");
            return trimmed.trim().to_string();
        }
    }
    cleaned.trim().to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn string_args(pairs: &[(&str, &str)]) -> serde_json::Value {
        serde_json::to_value(HashMap::<_, _>::from_iter(pairs.iter().copied()))
            .unwrap_or_else(|error| panic!("string arguments should serialize: {error}"))
    }

    fn make_spec(name: &str, params: &[(&str, ParamType)], required: &[&str]) -> FunctionSpec {
        let parameters = params
            .iter()
            .map(|(k, t)| {
                (
                    k.to_string(),
                    ParamSchema {
                        kind: t.clone(),
                        description: Some(format!("{k} parameter")),
                        enum_values: None,
                    },
                )
            })
            .collect();
        FunctionSpec {
            name: name.to_string(),
            description: format!("Test function: {name}"),
            parameters,
            required: required.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ─ FunctionSpec::validate ─────────────────────────────────────────────

    #[test]
    fn test_function_spec_valid_spec_returns_no_errors() {
        let spec = make_spec(
            "lookup_entity",
            &[("entity_id", ParamType::String)],
            &["entity_id"],
        );
        assert!(
            spec.validate().is_empty(),
            "valid spec must pass validation"
        );
    }

    #[test]
    fn test_function_spec_empty_name_is_invalid() {
        let spec = make_spec("", &[("x", ParamType::Integer)], &[]);
        let errs = spec.validate();
        assert!(!errs.is_empty());
        assert!(errs[0].contains("name must not be empty"));
    }

    #[test]
    fn test_function_spec_invalid_name_characters_rejected() {
        let spec = make_spec("bad name!", &[("x", ParamType::Integer)], &[]);
        let errs = spec.validate();
        assert!(errs.iter().any(|e| e.contains("invalid characters")));
    }

    #[test]
    fn test_function_spec_missing_required_param_reported() {
        let spec = make_spec("fn", &[("a", ParamType::String)], &["a", "b"]);
        let errs = spec.validate();
        assert!(errs.iter().any(|e| e.contains("`b`")));
    }

    #[test]
    fn test_function_spec_all_required_present_passes_validation() {
        let spec = make_spec(
            "multi",
            &[("x", ParamType::Number), ("y", ParamType::Boolean)],
            &["x", "y"],
        );
        assert!(spec.validate().is_empty());
    }

    // ─ FunctionSpec::to_tool_json ─────────────────────────────────────────

    #[test]
    fn test_to_tool_json_produces_openai_tool_shape() {
        let spec = make_spec("get_flag", &[("flag_id", ParamType::String)], &["flag_id"]);
        let json = spec.to_tool_json();
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "get_flag");
        let required = json["function"]["parameters"]["required"]
            .as_array()
            .unwrap_or_else(|| panic!("required field should be an array"));
        assert!(required.iter().any(|v| v == "flag_id"));
    }

    // ─ FunctionCall ───────────────────────────────────────────────────────

    #[test]
    fn test_function_call_parse_arguments_success() {
        let call = FunctionCall {
            name: "get_entity".to_string(),
            arguments: string_args(&[("entity_id", "abc-123")]),
        };
        let parsed: HashMap<String, String> = call
            .parse_arguments()
            .unwrap_or_else(|error| panic!("function arguments should parse: {error}"));
        assert_eq!(parsed["entity_id"], "abc-123");
    }

    #[test]
    fn test_function_call_validate_against_all_required_present() {
        let spec = make_spec("f", &[("a", ParamType::String)], &["a"]);
        let call = FunctionCall {
            name: "f".to_string(),
            arguments: string_args(&[("a", "hello")]),
        };
        assert!(call.validate_against(&spec).is_empty());
    }

    #[test]
    fn test_function_call_validate_against_detects_missing_required() {
        let spec = make_spec(
            "f",
            &[("a", ParamType::String), ("b", ParamType::Number)],
            &["a", "b"],
        );
        let call = FunctionCall {
            name: "f".to_string(),
            arguments: string_args(&[("a", "hello")]), // "b" missing
        };
        let missing = call.validate_against(&spec);
        assert_eq!(missing, vec!["b".to_string()]);
    }

    #[test]
    fn test_function_call_validate_against_non_object_arguments() {
        let spec = make_spec("f", &[("a", ParamType::String)], &["a"]);
        let call = FunctionCall {
            name: "f".to_string(),
            arguments: serde_json::Value::String("not-an-object".to_string()),
        };
        let errs = call.validate_against(&spec);
        assert!(errs[0].contains("not a JSON object"));
    }

    // ─ ParamType ──────────────────────────────────────────────────────────

    #[test]
    fn test_param_type_display_matches_json_schema() {
        assert_eq!(ParamType::String.as_str(), "string");
        assert_eq!(ParamType::Integer.as_str(), "integer");
        assert_eq!(ParamType::Boolean.as_str(), "boolean");
    }

    // ─ Tool execution framework ───────────────────────────────────────────

    #[test]
    fn extract_first_json_finds_balanced_object() {
        let s = "prefix {\"a\":1, \"b\":{\"c\":2}} suffix";
        assert_eq!(
            extract_first_json(s).as_deref(),
            Some(r#"{"a":1, "b":{"c":2}}"#)
        );
    }

    #[test]
    fn extract_first_json_handles_strings_with_braces() {
        let s = r#"call {"x": "a}b", "y": 2} done"#;
        assert!(extract_first_json(s).is_some());
    }

    #[test]
    fn extract_first_json_returns_none_without_brace() {
        assert!(extract_first_json("no json here").is_none());
    }

    // Audit: tool results fed back into the conversation must be bounded so a
    // huge tool payload cannot grow the request without limit.
    #[test]
    fn bounded_tool_result_text_truncates_oversized_payload() {
        let huge = serde_json::json!({"blob": "x".repeat(100_000)});
        let text = bounded_tool_result_text(&huge);
        assert!(
            text.len() <= MAX_TOOL_RESULT_BYTES,
            "tool result text must respect the byte cap: {}",
            text.len()
        );

        let small = serde_json::json!({"ok": true});
        assert_eq!(
            bounded_tool_result_text(&small),
            r#"{"ok":true}"#.to_string()
        );
    }

    #[test]
    fn conversation_bytes_sums_message_content() {
        use crate::inference::ChatMessage;
        let messages = vec![ChatMessage::system("abc"), ChatMessage::user("de")];
        assert_eq!(conversation_bytes(&messages), 5);
        assert!(conversation_bytes(&[]) == 0);
    }

    #[test]
    fn parse_tool_calls_single_object() {
        let calls = parse_tool_calls(
            r#"I'll look that up. {"name":"get_entity","arguments":{"entity_id":"123"}}"#,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_entity");
        assert_eq!(calls[0].arguments["entity_id"], "123");
    }

    #[test]
    fn parse_tool_calls_array_form() {
        let calls = parse_tool_calls(
            r#"{"tool_calls":[{"name":"a","arguments":{}},{"name":"b","arguments":{"k":1}}]}"#,
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, "b");
    }

    #[test]
    fn parse_tool_calls_none_for_plain_prose() {
        assert!(parse_tool_calls("The answer is 42.").is_empty());
    }

    #[test]
    fn strip_tool_call_block_removes_json() {
        let out = strip_tool_call_block(r#"Sure. {"name":"x","arguments":{}} Here is my answer."#);
        assert!(!out.contains(r#""name":"x""#));
        assert!(out.contains("Here is my answer."));
    }

    #[test]
    fn strip_tool_call_block_preserves_structured_non_tool_json() {
        let recipe = r#"{"id":"r1","join":"Entity","outcome":"x","signals":["a"]}"#;
        assert_eq!(
            strip_tool_call_block(recipe),
            recipe,
            "a structured final answer must not be stripped as a tool-call block"
        );
    }

    struct EchoTool;

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn spec(&self) -> FunctionSpec {
            FunctionSpec {
                name: "echo".to_string(),
                description: "Echo the argument back.".to_string(),
                parameters: [(
                    "msg".to_string(),
                    ParamSchema {
                        kind: ParamType::String,
                        description: Some("message".to_string()),
                        enum_values: None,
                    },
                )]
                .into_iter()
                .collect(),
                required: vec!["msg".to_string()],
            }
        }
        async fn execute(&self, args: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Ok(
                serde_json::json!({"echoed": args.get("msg").cloned().unwrap_or(serde_json::Value::Null)}),
            )
        }
    }

    #[tokio::test]
    async fn registry_executes_registered_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(std::sync::Arc::new(EchoTool));
        let call = FunctionCall {
            name: "echo".into(),
            arguments: serde_json::json!({"msg": "hello"}),
        };
        let res = reg.execute_call(&call).await.unwrap();
        assert_eq!(res["echoed"], "hello");
    }

    #[tokio::test]
    async fn registry_unknown_tool_errors() {
        let reg = ToolRegistry::new();
        let call = FunctionCall {
            name: "nope".into(),
            arguments: serde_json::json!({}),
        };
        let err = reg.execute_call(&call).await.unwrap_err();
        assert!(matches!(err, ToolError::UnknownTool { .. }));
    }

    #[tokio::test]
    async fn registry_missing_required_arg_errors() {
        let mut reg = ToolRegistry::new();
        reg.register(std::sync::Arc::new(EchoTool));
        let call = FunctionCall {
            name: "echo".into(),
            arguments: serde_json::json!({}),
        };
        let err = reg.execute_call(&call).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments { .. }));
    }

    // ── #93: run_agent_loop correctness tests ─────────────────────────────
    //
    // These drive the loop against a minimal local OpenAI-compatible endpoint
    // so the stop conditions, caps, and trace bookkeeping are exercised for
    // real (no HTTP client trait abstraction is introduced).

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn chat_completion(content: &str) -> String {
        serde_json::json!({
            "choices": [{
                "message": { "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string()
    }

    async fn read_http_request_body(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(header_end) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
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
                    let read = socket.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                }
                return buf[body_start..(body_start + content_length).min(buf.len())].to_vec();
            }
            let read = socket.read(&mut chunk).await.unwrap();
            if read == 0 {
                return buf;
            }
            buf.extend_from_slice(&chunk[..read]);
        }
    }

    /// Minimal OpenAI-compatible server: answers one request per scripted
    /// response (in order) and returns the captured request bodies.
    async fn mock_llm_server(
        responses: Vec<String>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut captured = Vec::with_capacity(responses.len());
            for response in responses {
                let accepted =
                    tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                        .await;
                let Ok(Ok((mut socket, _))) = accepted else {
                    break;
                };
                let body = read_http_request_body(&mut socket).await;
                captured.push(String::from_utf8_lossy(&body).to_string());
                let payload = response.as_bytes();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(payload).await.unwrap();
                socket.flush().await.unwrap();
                let _ = socket.shutdown().await;
            }
            captured
        });
        (format!("http://{addr}"), handle)
    }

    fn test_client(base_url: &str) -> crate::inference::LlmClient {
        crate::inference::LlmClient::new(
            base_url,
            None,
            crate::inference::InferenceConfig {
                max_retries: 0,
                timeout: std::time::Duration::from_secs(10),
                ..Default::default()
            },
        )
    }

    struct FailingTool;

    #[async_trait::async_trait]
    impl Tool for FailingTool {
        fn spec(&self) -> FunctionSpec {
            FunctionSpec {
                name: "fail".to_string(),
                description: "Always fails.".to_string(),
                parameters: HashMap::new(),
                required: Vec::new(),
            }
        }

        async fn execute(&self, _args: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Err(ToolError::ExecutionFailed {
                name: "fail".to_string(),
                message: "simulated backend outage".to_string(),
            })
        }
    }

    struct SilentTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for SilentTool {
        fn spec(&self) -> FunctionSpec {
            FunctionSpec {
                name: self.0.to_string(),
                description: format!("{} test tool", self.0),
                parameters: HashMap::new(),
                required: Vec::new(),
            }
        }

        async fn execute(&self, _args: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({"ok": true}))
        }
    }

    #[test]
    fn tool_specs_json_is_deterministically_sorted() {
        let mut first = ToolRegistry::new();
        first.register(std::sync::Arc::new(SilentTool("zeta")));
        first.register(std::sync::Arc::new(SilentTool("alpha")));
        let mut second = ToolRegistry::new();
        second.register(std::sync::Arc::new(SilentTool("alpha")));
        second.register(std::sync::Arc::new(SilentTool("zeta")));

        let names = |specs: &[serde_json::Value]| {
            specs
                .iter()
                .map(|spec| {
                    spec["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&first.tool_specs_json()), vec!["alpha", "zeta"]);
        assert_eq!(
            names(&first.tool_specs_json()),
            names(&second.tool_specs_json()),
            "tool ordering must not depend on registration order"
        );
    }

    #[tokio::test]
    async fn agent_loop_advertises_registered_tools_in_system_prompt() {
        let (url, server) = mock_llm_server(vec![chat_completion("done")]).await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "You are a test agent.", "say hi", 3)
            .await
            .expect("agent loop should complete");
        assert_eq!(result.final_answer, "done");
        assert_eq!(result.iterations, 1);

        let captured = server.await.unwrap();
        assert_eq!(captured.len(), 1);
        let request = &captured[0];
        assert!(
            request.contains("Available tools"),
            "system prompt must announce the tool catalog: {request}"
        );
        assert!(
            request.contains("echo"),
            "registered tool must be named in the system prompt: {request}"
        );
        assert!(
            request.contains("msg"),
            "tool parameter schema must be advertised: {request}"
        );
    }

    /// A client that implements only the single-prompt trait methods. Its
    /// `complete_messages` must therefore be the trait's default flattening
    /// implementation — proving the default keeps trait-object callers working.
    struct SinglePromptOnlyLlm {
        seen: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl crate::LlmClient for SinglePromptOnlyLlm {
        async fn generate_json(&self, _system: &str, _user: &str) -> anyhow::Result<String> {
            Ok("{}".to_string())
        }

        async fn generate_text(&self, system: &str, user: &str) -> anyhow::Result<String> {
            let mut seen = self.seen.lock().unwrap_or_else(|error| error.into_inner());
            seen.push((system.to_string(), user.to_string()));
            Ok("default-final".to_string())
        }
    }

    #[tokio::test]
    async fn agent_loop_default_complete_messages_flattens_for_single_prompt_clients() {
        let client = SinglePromptOnlyLlm {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "system prompt", "do the task", 3)
            .await
            .expect("the default complete_messages must run the loop");
        assert_eq!(result.final_answer, "default-final");
        assert_eq!(result.iterations, 1);

        let seen = client
            .seen
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].0.contains("system prompt") && seen[0].0.contains("Available tools"),
            "system half must carry the prompt and tool catalog: {}",
            seen[0].0
        );
        assert!(
            seen[0].1.contains("[user]\ndo the task"),
            "user half must carry the role-labelled task turn: {}",
            seen[0].1
        );
    }

    // The exact production shape: worker code holds `Arc<dyn LlmClient>`, not
    // the concrete `inference::LlmClient`. This pins both boxing shapes and
    // proves a multi-turn (tool-call) conversation survives the trait object.
    #[tokio::test]
    async fn agent_loop_runs_through_boxed_and_arced_trait_objects() {
        let tool_call = r#"{"tool_calls":[{"name":"echo","arguments":{"msg":"hi"}}]}"#;
        let (url, server) = mock_llm_server(vec![
            chat_completion("boxed answer"),
            chat_completion(tool_call),
            chat_completion("arced answer"),
        ])
        .await;
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let boxed: Box<dyn crate::LlmClient> = Box::new(test_client(&url));
        let result = run_agent_loop(boxed.as_ref(), &registry, "system", "task", 3)
            .await
            .expect("boxed trait object must run the loop");
        assert_eq!(result.final_answer, "boxed answer");

        let arced: std::sync::Arc<dyn crate::LlmClient> = std::sync::Arc::new(test_client(&url));
        let result = run_agent_loop(arced.as_ref(), &registry, "system", "task", 3)
            .await
            .expect("arced trait object must run the multi-turn loop");
        assert_eq!(result.iterations, 2);
        assert_eq!(result.tool_trace.len(), 1);
        assert_eq!(result.final_answer, "arced answer");

        let captured = server.await.unwrap();
        assert_eq!(
            captured.len(),
            3,
            "every trait-object call must reach the model endpoint"
        );
        assert!(
            captured[2].contains(r#""role":"assistant""#) && captured[2].contains("[tool_result]"),
            "the post-tool request must carry the assistant and tool-result turns: {}",
            captured[2]
        );
    }

    #[tokio::test]
    async fn agent_loop_preserves_structured_json_final_answer() {
        let recipe = r#"{"id":"r1","join":"Entity","outcome":"supplier_distress","signals":["late_filing"]}"#;
        let (url, server) = mock_llm_server(vec![chat_completion(recipe)]).await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "system", "task", 3)
            .await
            .expect("agent loop should complete");
        assert_eq!(
            result.final_answer, recipe,
            "the model's structured final answer must reach the caller verbatim"
        );
        assert_eq!(result.iterations, 1);

        let captured = server.await.unwrap();
        assert_eq!(captured.len(), 1);
    }

    #[tokio::test]
    async fn agent_loop_stops_at_max_iterations() {
        let tool_call = r#"{"tool_calls":[{"name":"echo","arguments":{"msg":"hi"}}]}"#;
        let (url, server) =
            mock_llm_server(vec![chat_completion(tool_call), chat_completion(tool_call)]).await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "system", "task", 2)
            .await
            .expect("hitting the iteration cap must not be an error");
        assert_eq!(result.iterations, 2);
        assert_eq!(result.tool_trace.len(), 2);
        assert!(
            result.final_answer.contains("max tool-call iterations"),
            "sentinel answer must explain the stop: {}",
            result.final_answer
        );

        let captured = server.await.unwrap();
        assert_eq!(captured.len(), 2, "no model call may happen after the cap");
    }

    #[tokio::test]
    async fn agent_loop_stops_at_conversation_byte_cap() {
        let tool_call = serde_json::json!({
            "tool_calls": [{"name": "echo", "arguments": {"msg": "hi"}}],
            "padding": "x".repeat(MAX_CONVERSATION_BYTES),
        })
        .to_string();
        assert!(tool_call.len() > MAX_CONVERSATION_BYTES);
        let (url, server) = mock_llm_server(vec![chat_completion(&tool_call)]).await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "system", "task", 5)
            .await
            .expect("hitting the byte cap must not be an error");
        assert_eq!(
            result.iterations, 1,
            "the second model call must not happen"
        );
        assert_eq!(result.tool_trace.len(), 1);
        assert!(
            result.final_answer.contains("byte cap"),
            "sentinel answer must explain the stop: {}",
            result.final_answer
        );

        let captured = server.await.unwrap();
        assert_eq!(captured.len(), 1, "no model call may happen after the cap");
    }

    #[tokio::test]
    async fn agent_loop_caps_tool_calls_per_iteration() {
        let calls: Vec<serde_json::Value> = (0..20)
            .map(|index| {
                serde_json::json!({
                    "name": "echo",
                    "arguments": { "msg": format!("m{index}") },
                })
            })
            .collect();
        assert!(calls.len() > MAX_TOOL_CALLS_PER_ITERATION);
        let tool_call = serde_json::json!({ "tool_calls": calls }).to_string();
        let (url, _server) =
            mock_llm_server(vec![chat_completion(&tool_call), chat_completion("final")]).await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool));

        let result = run_agent_loop(&client, &registry, "system", "task", 3)
            .await
            .expect("agent loop should complete");
        assert_eq!(result.iterations, 2);
        assert_eq!(
            result.tool_trace.len(),
            MAX_TOOL_CALLS_PER_ITERATION,
            "only the first {MAX_TOOL_CALLS_PER_ITERATION} calls from one response may execute"
        );
        assert_eq!(result.final_answer, "final");
    }

    #[test]
    fn bounded_tool_result_text_truncates_on_utf8_boundary() {
        let value = serde_json::json!({"blob": "日".repeat(20_000)});
        let original = serde_json::to_string(&value).unwrap();
        assert!(original.len() > MAX_TOOL_RESULT_BYTES);

        let text = bounded_tool_result_text(&value);
        assert!(text.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(
            original.starts_with(&text),
            "truncation must be a clean prefix of the serialized payload"
        );
        assert!(
            text.len() > MAX_TOOL_RESULT_BYTES - 4,
            "truncation must back off at most one multi-byte character: {}",
            text.len()
        );
        assert!(
            !text.contains('\u{FFFD}'),
            "the byte cut must land on a UTF-8 character boundary"
        );
    }

    #[tokio::test]
    async fn agent_loop_records_tool_error_with_ok_false() {
        let tool_call = r#"{"name":"fail","arguments":{}}"#;
        let (url, server) = mock_llm_server(vec![
            chat_completion(tool_call),
            chat_completion("recovered"),
        ])
        .await;
        let client = test_client(&url);
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(FailingTool));

        let result = run_agent_loop(&client, &registry, "system", "task", 3)
            .await
            .expect("tool errors are fed back to the model, not fatal");
        assert_eq!(result.final_answer, "recovered");
        assert_eq!(result.tool_trace.len(), 1);
        let entry = &result.tool_trace[0];
        assert_eq!(entry.name, "fail");
        assert!(
            !entry.ok,
            "a failed tool execution must be traced with ok=false"
        );
        assert_eq!(entry.result["kind"], "ExecutionFailed");
        assert!(
            entry.result["message"]
                .as_str()
                .is_some_and(|message| message.contains("simulated backend outage")),
            "trace must carry the tool failure message: {:?}",
            entry.result
        );

        let captured = server.await.unwrap();
        assert_eq!(captured.len(), 2);
        assert!(
            captured[1].contains("ExecutionFailed"),
            "the tool error must be fed back into the conversation: {}",
            captured[1]
        );
    }
}
