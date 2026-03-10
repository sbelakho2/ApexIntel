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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": self.parameters.iter()
                        .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null)))
                        .collect::<HashMap<_, _>>(),
                    "required": self.required,
                }
            }
        })
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
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
            .unwrap();
        assert!(required.iter().any(|v| v == "flag_id"));
    }

    // ─ FunctionCall ───────────────────────────────────────────────────────

    #[test]
    fn test_function_call_parse_arguments_success() {
        let call = FunctionCall {
            name: "get_entity".to_string(),
            arguments: serde_json::json!({"entity_id": "abc-123"}),
        };
        let parsed: HashMap<String, String> = call.parse_arguments().unwrap();
        assert_eq!(parsed["entity_id"], "abc-123");
    }

    #[test]
    fn test_function_call_validate_against_all_required_present() {
        let spec = make_spec("f", &[("a", ParamType::String)], &["a"]);
        let call = FunctionCall {
            name: "f".to_string(),
            arguments: serde_json::json!({"a": "hello"}),
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
            arguments: serde_json::json!({"a": "hello"}), // "b" missing
        };
        let missing = call.validate_against(&spec);
        assert_eq!(missing, vec!["b".to_string()]);
    }

    #[test]
    fn test_function_call_validate_against_non_object_arguments() {
        let spec = make_spec("f", &[("a", ParamType::String)], &["a"]);
        let call = FunctionCall {
            name: "f".to_string(),
            arguments: serde_json::json!("not-an-object"),
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
}
