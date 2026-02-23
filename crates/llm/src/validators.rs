//! JSON schema validation for LLM outputs.
//!
//! Pure functions that validate LLM-generated content before it enters the pipeline:
//! - JSON structure validation (required fields, types)
//! - Response cleaning (extract JSON from markdown code blocks)
//! - Content quality checks

use serde_json::Value;
use std::collections::HashSet;

// ────────────────────────────────────────────
// JSON extraction & cleaning
// ────────────────────────────────────────────

/// Extract JSON from a response that may contain markdown code fences.
/// Handles ```json ... ``` blocks and bare JSON.
pub fn extract_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();

    // Try to extract from ```json ... ``` or ``` ... ``` block
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        // Skip optional language tag (json, JSON, etc.)
        let content_start = if after_fence.starts_with("json") || after_fence.starts_with("JSON") {
            after_fence.find('\n').map(|i| i + 1).unwrap_or(0)
        } else if after_fence.starts_with('\n') {
            1
        } else {
            0
        };

        let content = &after_fence[content_start..];
        if let Some(end) = content.find("```") {
            let json_str = content[..end].trim();
            return Some(json_str.to_string());
        }
    }

    // Try bare JSON: starts with { or [
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(trimmed.to_string());
    }

    None
}

/// Parse JSON from a raw LLM response, handling code fences.
pub fn parse_json_response(raw: &str) -> Result<Value, String> {
    let json_str = extract_json(raw).ok_or_else(|| "No JSON found in response".to_string())?;
    serde_json::from_str(&json_str).map_err(|e| format!("Invalid JSON: {}", e))
}

// ────────────────────────────────────────────
// Schema validation
// ────────────────────────────────────────────

/// Check that a JSON value has all required fields.
pub fn check_required_fields(value: &Value, required: &[&str]) -> Vec<String> {
    let mut missing = Vec::new();
    if let Value::Object(map) = value {
        for field in required {
            if !map.contains_key(*field) {
                missing.push(field.to_string());
            }
        }
    } else {
        missing.push("(root is not an object)".to_string());
    }
    missing
}

/// Check that a field is a string and non-empty.
pub fn check_string_field(value: &Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Check that a field is an array.
pub fn check_array_field(value: &Value, field: &str) -> bool {
    value.get(field).and_then(|v| v.as_array()).is_some()
}

/// Check that a field is a number.
pub fn check_number_field(value: &Value, field: &str) -> bool {
    value.get(field).and_then(|v| v.as_f64()).is_some()
}

/// Validate a recipe JSON against the expected schema.
pub fn validate_recipe_json(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let required = ["id", "signals", "narrative_template", "action_playbook"];
    let missing = check_required_fields(value, &required);
    for field in &missing {
        errors.push(format!("Missing required field: {}", field));
    }

    if !check_string_field(value, "narrative_template") && !missing.contains(&"narrative_template".to_string()) {
        errors.push("narrative_template must be a non-empty string".to_string());
    }

    if !check_array_field(value, "signals") && !missing.contains(&"signals".to_string()) {
        errors.push("signals must be an array".to_string());
    }

    errors
}

/// Validate an insight JSON.
pub fn validate_insight_json(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let required = ["recipe_id", "entity_id", "narrative", "severity", "confidence"];
    let missing = check_required_fields(value, &required);
    for field in &missing {
        errors.push(format!("Missing required field: {}", field));
    }

    if let Some(conf) = value.get("confidence").and_then(|v| v.as_f64()) {
        if !(0.0..=1.0).contains(&conf) {
            errors.push(format!("confidence must be 0-1, got {}", conf));
        }
    }

    if let Some(sev) = value.get("severity").and_then(|v| v.as_str()) {
        let valid = ["critical", "high", "medium", "low", "info"];
        if !valid.contains(&sev.to_lowercase().as_str()) {
            errors.push(format!("Invalid severity: {}", sev));
        }
    }

    errors
}

// ────────────────────────────────────────────
// Content quality checks
// ────────────────────────────────────────────

/// Check minimum content length.
pub fn check_min_length(text: &str, min_chars: usize) -> bool {
    text.trim().len() >= min_chars
}

/// Check that content is not just repeated characters or gibberish.
pub fn check_content_quality(text: &str) -> Vec<String> {
    let mut issues = Vec::new();

    let trimmed = text.trim();
    if trimmed.is_empty() {
        issues.push("Content is empty".to_string());
        return issues;
    }

    // Check for excessive repetition
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() > 10 {
        let unique: HashSet<&char> = chars.iter().collect();
        let ratio = unique.len() as f64 / chars.len() as f64;
        if ratio < 0.05 {
            issues.push("Content has excessive character repetition".to_string());
        }
    }

    // Check for very short content
    if trimmed.len() < 10 {
        issues.push("Content is suspiciously short".to_string());
    }

    // Check for common LLM refusal patterns
    let lower = trimmed.to_lowercase();
    let refusal_patterns = [
        "i cannot",
        "i'm unable to",
        "as an ai",
        "i don't have access",
        "i apologize, but",
    ];
    for pattern in &refusal_patterns {
        if lower.starts_with(pattern) {
            issues.push(format!("Content appears to be a refusal: starts with '{}'", pattern));
        }
    }

    issues
}

/// Validate that a JSON response has a unique ID (not a copy of another recipe).
pub fn check_unique_id(value: &Value, existing_ids: &[&str]) -> bool {
    if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
        !existing_ids.contains(&id)
    } else {
        true // no id field -> can't check
    }
}

/// Full validation pipeline for a recipe response.
pub fn validate_recipe_response(raw: &str, existing_ids: &[&str]) -> Result<Value, Vec<String>> {
    let mut errors = Vec::new();

    // Step 1: Extract and parse JSON
    let value = match parse_json_response(raw) {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            return Err(errors);
        }
    };

    // Step 2: Schema validation
    errors.extend(validate_recipe_json(&value));

    // Step 3: Quality check on narrative
    if let Some(narrative) = value.get("narrative_template").and_then(|v| v.as_str()) {
        let quality = check_content_quality(narrative);
        for q in &quality {
            errors.push(format!("Quality: {}", q));
        }
    }

    // Step 4: Unique ID
    if !check_unique_id(&value, existing_ids) {
        errors.push("Recipe ID already exists".to_string());
    }

    if errors.is_empty() {
        Ok(value)
    } else {
        Err(errors)
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- JSON extraction --

    #[test]
    fn test_extract_json_bare() {
        let raw = r#"{"key": "value"}"#;
        let result = extract_json(raw).unwrap();
        assert_eq!(result, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_code_fence() {
        let raw = "Here is the result:\n```json\n{\"key\": \"value\"}\n```\nDone.";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_code_fence_no_lang() {
        let raw = "```\n{\"key\": 42}\n```";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, r#"{"key": 42}"#);
    }

    #[test]
    fn test_extract_json_array() {
        let raw = "[1, 2, 3]";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, "[1, 2, 3]");
    }

    #[test]
    fn test_extract_json_none() {
        let raw = "This is just text with no JSON.";
        assert!(extract_json(raw).is_none());
    }

    #[test]
    fn test_parse_json_response_valid() {
        let raw = r#"```json
{"id": "R001", "name": "test"}
```"#;
        let value = parse_json_response(raw).unwrap();
        assert_eq!(value["id"], "R001");
    }

    #[test]
    fn test_parse_json_response_invalid() {
        let raw = r#"```json
{invalid json}
```"#;
        assert!(parse_json_response(raw).is_err());
    }

    // -- Schema validation --

    #[test]
    fn test_check_required_fields_all_present() {
        let val: Value = serde_json::json!({"a": 1, "b": 2, "c": 3});
        let missing = check_required_fields(&val, &["a", "b"]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_check_required_fields_missing() {
        let val: Value = serde_json::json!({"a": 1});
        let missing = check_required_fields(&val, &["a", "b", "c"]);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&"b".to_string()));
        assert!(missing.contains(&"c".to_string()));
    }

    #[test]
    fn test_check_required_fields_not_object() {
        let val: Value = serde_json::json!([1, 2, 3]);
        let missing = check_required_fields(&val, &["a"]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("not an object"));
    }

    #[test]
    fn test_check_string_field() {
        let val: Value = serde_json::json!({"name": "test", "empty": "", "num": 42});
        assert!(check_string_field(&val, "name"));
        assert!(!check_string_field(&val, "empty"));
        assert!(!check_string_field(&val, "num"));
        assert!(!check_string_field(&val, "missing"));
    }

    #[test]
    fn test_check_array_field() {
        let val: Value = serde_json::json!({"items": [1, 2], "name": "test"});
        assert!(check_array_field(&val, "items"));
        assert!(!check_array_field(&val, "name"));
    }

    #[test]
    fn test_check_number_field() {
        let val: Value = serde_json::json!({"score": 0.95, "name": "test"});
        assert!(check_number_field(&val, "score"));
        assert!(!check_number_field(&val, "name"));
    }

    #[test]
    fn test_validate_recipe_json_valid() {
        let val: Value = serde_json::json!({
            "id": "R001",
            "signals": [{"type": "hiring"}],
            "narrative_template": "Company {company} is hiring",
            "action_playbook": "Contact the hiring manager"
        });
        let errors = validate_recipe_json(&val);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_recipe_json_missing_fields() {
        let val: Value = serde_json::json!({"id": "R001"});
        let errors = validate_recipe_json(&val);
        assert!(errors.len() >= 2); // missing signals, narrative_template, action_playbook
    }

    #[test]
    fn test_validate_insight_json_valid() {
        let val: Value = serde_json::json!({
            "recipe_id": "R001",
            "entity_id": "E001",
            "narrative": "Some insight text",
            "severity": "high",
            "confidence": 0.85
        });
        let errors = validate_insight_json(&val);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_insight_json_bad_confidence() {
        let val: Value = serde_json::json!({
            "recipe_id": "R001",
            "entity_id": "E001",
            "narrative": "text",
            "severity": "high",
            "confidence": 1.5
        });
        let errors = validate_insight_json(&val);
        assert!(errors.iter().any(|e| e.contains("confidence")));
    }

    #[test]
    fn test_validate_insight_json_bad_severity() {
        let val: Value = serde_json::json!({
            "recipe_id": "R001",
            "entity_id": "E001",
            "narrative": "text",
            "severity": "extreme",
            "confidence": 0.5
        });
        let errors = validate_insight_json(&val);
        assert!(errors.iter().any(|e| e.contains("severity")));
    }

    // -- Content quality --

    #[test]
    fn test_check_min_length() {
        assert!(check_min_length("This is long enough text", 10));
        assert!(!check_min_length("Short", 10));
        assert!(!check_min_length("   ", 1));
    }

    #[test]
    fn test_check_content_quality_good() {
        let text = "Starz Electronics has been awarded ISO 9001 certification for their Tunis facility.";
        let issues = check_content_quality(text);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_check_content_quality_empty() {
        let issues = check_content_quality("");
        assert!(issues.iter().any(|i| i.contains("empty")));
    }

    #[test]
    fn test_check_content_quality_too_short() {
        let issues = check_content_quality("Hi");
        assert!(issues.iter().any(|i| i.contains("short")));
    }

    #[test]
    fn test_check_content_quality_repetition() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let issues = check_content_quality(text);
        assert!(issues.iter().any(|i| i.contains("repetition")));
    }

    #[test]
    fn test_check_content_quality_refusal() {
        let issues = check_content_quality("I cannot generate that content because...");
        assert!(issues.iter().any(|i| i.contains("refusal")));
    }

    // -- Unique ID --

    #[test]
    fn test_check_unique_id_new() {
        let val: Value = serde_json::json!({"id": "R999"});
        assert!(check_unique_id(&val, &["R001", "R002"]));
    }

    #[test]
    fn test_check_unique_id_duplicate() {
        let val: Value = serde_json::json!({"id": "R001"});
        assert!(!check_unique_id(&val, &["R001", "R002"]));
    }

    #[test]
    fn test_check_unique_id_no_id_field() {
        let val: Value = serde_json::json!({"name": "test"});
        assert!(check_unique_id(&val, &["R001"]));
    }

    // -- Full pipeline --

    #[test]
    fn test_validate_recipe_response_valid() {
        let raw = r#"```json
{
    "id": "R100",
    "signals": [{"type": "hiring_surge"}],
    "narrative_template": "Company {company} shows a significant hiring surge indicating expansion plans.",
    "action_playbook": "Schedule outreach with procurement team to discuss capacity needs."
}
```"#;
        let result = validate_recipe_response(raw, &["R001", "R002"]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_recipe_response_duplicate_id() {
        let raw = r#"{"id": "R001", "signals": [1], "narrative_template": "Some long enough narrative text here.", "action_playbook": "Take these actions"}"#;
        let result = validate_recipe_response(raw, &["R001"]);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("already exists")));
    }

    #[test]
    fn test_validate_recipe_response_no_json() {
        let raw = "This is not JSON at all.";
        let result = validate_recipe_response(raw, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_recipe_response_missing_fields() {
        let raw = r#"{"id": "R100"}"#;
        let result = validate_recipe_response(raw, &[]);
        assert!(result.is_err());
    }
}
