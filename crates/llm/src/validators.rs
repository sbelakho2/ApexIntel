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

    // Try all fenced code blocks in order and return the first JSON-looking payload.
    let mut search_from = 0usize;
    while let Some(start_rel) = trimmed[search_from..].find("```") {
        let start = search_from + start_rel;
        let after_fence = &trimmed[start + 3..];
        let after_fence_trimmed = after_fence.trim_start();
        let content_start =
            if after_fence_trimmed.starts_with("json") || after_fence_trimmed.starts_with("JSON") {
                after_fence
                    .find('\n')
                    .map(|i| i + 1)
                    .unwrap_or(after_fence.len())
            } else if after_fence.starts_with('\n') {
                1
            } else {
                0
            };

        let content = &after_fence[content_start..];
        let end = content
            .find("\n```")
            .map(|i| i + 1)
            .or_else(|| content.find("```"));
        if let Some(end) = end {
            let block = content[..end].trim();
            if block.starts_with('{') || block.starts_with('[') {
                return Some(block.to_string());
            }
            search_from = start + 3 + content_start + end + 3;
            continue;
        }
        break;
    }

    // Try bare JSON: starts with { or [
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(trimmed.to_string());
    }

    None
}

/// Parse JSON from a raw LLM response, handling code fences.
/// Returns error if no JSON found or if the extracted string is not valid JSON (B201).
pub fn parse_json_response(raw: &str) -> Result<Value, String> {
    let json_str = extract_json(raw).ok_or_else(|| "No JSON found in response".to_string())?;
    // B201: explicitly validate that extracted content parses as JSON
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
    value
        .get(field)
        .and_then(|v| v.as_f64())
        .map(|n| n.is_finite())
        .unwrap_or(false)
}

/// Validate a recipe JSON against the expected schema.
pub fn validate_recipe_json(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let required = ["id", "signals", "narrative_template", "action_playbook"];
    let missing = check_required_fields(value, &required);
    for field in &missing {
        errors.push(format!("Missing required field: {}", field));
    }

    if !check_string_field(value, "narrative_template")
        && !missing.contains(&"narrative_template".to_string())
    {
        errors.push("narrative_template must be a non-empty string".to_string());
    }

    // B204: action_playbook must be a non-empty string
    if !missing.contains(&"action_playbook".to_string()) {
        if !check_string_field(value, "action_playbook") {
            errors.push("action_playbook must be a non-empty string".to_string());
        }
    }

    if !check_array_field(value, "signals") && !missing.contains(&"signals".to_string()) {
        errors.push("signals must be an array".to_string());
    }

    // B205: validate signals items are objects or strings with non-empty values
    if let Some(signals) = value.get("signals").and_then(|v| v.as_array()) {
        if signals.is_empty() {
            errors.push("signals array must not be empty".to_string());
        }
        for (i, sig) in signals.iter().enumerate() {
            if !(sig.is_object() || sig.is_string()) {
                errors.push(format!("signals[{}] must be an object or string", i));
            }
        }
    }

    errors
}

/// Validate an insight JSON.
pub fn validate_insight_json(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let required = [
        "recipe_id",
        "entity_id",
        "narrative",
        "severity",
        "confidence",
    ];
    let missing = check_required_fields(value, &required);
    for field in &missing {
        errors.push(format!("Missing required field: {}", field));
    }

    if value.get("confidence").is_some() && !check_number_field(value, "confidence") {
        errors.push("confidence must be a finite number".to_string());
    } else if let Some(conf) = value.get("confidence").and_then(|v| v.as_f64()) {
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

/// Validate the envelope schema for an LLM output record before DB storage (B379).
///
/// Expected shape:
/// `{ "schema_version": "v1", "kind": "...", "payload": { ... }, "created_at": "..." }`
pub fn validate_stored_llm_output(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let required = ["schema_version", "kind", "payload", "created_at"];
    for missing in check_required_fields(value, &required) {
        errors.push(format!("Missing required field: {}", missing));
    }

    if let Some(schema_version) = value.get("schema_version").and_then(|v| v.as_str()) {
        if !schema_version.starts_with('v') {
            errors.push("schema_version must start with 'v' (e.g., v1)".to_string());
        }
    }

    if !check_string_field(value, "kind") && value.get("kind").is_some() {
        errors.push("kind must be a non-empty string".to_string());
    }

    if value.get("payload").is_some()
        && !value.get("payload").map(|p| p.is_object()).unwrap_or(false)
    {
        errors.push("payload must be an object".to_string());
    }

    if !check_string_field(value, "created_at") && value.get("created_at").is_some() {
        errors.push("created_at must be a non-empty string".to_string());
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
        let unique: HashSet<char> = chars.iter().copied().collect();
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
            issues.push(format!(
                "Content appears to be a refusal: starts with '{}'",
                pattern
            ));
        }
    }

    issues
}

/// Validate that a JSON response has a unique ID (not a copy of another recipe).
/// Returns `false` if the `id` field is missing or not a string (B209).
pub fn check_unique_id(value: &Value, existing_ids: &[&str]) -> bool {
    match value.get("id").and_then(|v| v.as_str()) {
        Some(id) if id.trim().is_empty() => false, // B209: empty id is invalid
        Some(id) => !existing_ids.contains(&id),
        None => false, // B209: missing id is explicitly invalid
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
        let text =
            "Starz Electronics has been awarded ISO 9001 certification for their Tunis facility.";
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
        // B209: missing id is now explicitly invalid
        let val: Value = serde_json::json!({"name": "test"});
        assert!(!check_unique_id(&val, &["R001"]));
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
        let raw = r#"{"id": "R001", "signals": [{"type":"x"}], "narrative_template": "Some long enough narrative text here.", "action_playbook": "Take these actions"}"#;
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

    // ════════════════════════════════════════════
    // B201: validate response format is JSON
    // ════════════════════════════════════════════

    #[test]
    fn test_parse_json_response_plain_text_rejected() {
        let raw = "This response is just plain text, not JSON.";
        let result = parse_json_response(raw);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No JSON"));
    }

    #[test]
    fn test_parse_json_response_html_rejected() {
        let raw = "<html><body>Not JSON</body></html>";
        let result = parse_json_response(raw);
        assert!(result.is_err());
    }

    // ════════════════════════════════════════════
    // B202: nested code fences
    // ════════════════════════════════════════════

    #[test]
    fn test_extract_json_nested_backticks_in_string() {
        // JSON value contains backticks — should still extract correctly
        let raw = "```json\n{\"code\": \"use `var`\"}\n```";
        let result = extract_json(raw).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["code"], "use `var`");
    }

    #[test]
    fn test_extract_json_double_fence() {
        // Outer fence wraps inner fence — should get the outer content
        let raw = "```json\n{\"example\": \"```inner```\"}\n```";
        let result = extract_json(raw);
        assert!(result.is_some());
    }

    // ════════════════════════════════════════════
    // B203: multilingual content quality
    // ════════════════════════════════════════════

    #[test]
    fn test_check_content_quality_chinese() {
        let text = "星茨电子已获得ISO 9001认证，其突尼斯工厂的质量管理体系得到了国际认可。";
        let issues = check_content_quality(text);
        assert!(issues.is_empty(), "Chinese text should pass: {:?}", issues);
    }

    #[test]
    fn test_check_content_quality_arabic() {
        let text = "حصلت شركة ستارز للإلكترونيات على شهادة آيزو ٩٠٠١ لمنشأتها في تونس.";
        let issues = check_content_quality(text);
        assert!(issues.is_empty(), "Arabic text should pass: {:?}", issues);
    }

    #[test]
    fn test_check_content_quality_mixed_scripts() {
        let text = "Starz Electronics (星茨电子) achieved ISO 9001 certification for Tunis.";
        let issues = check_content_quality(text);
        assert!(
            issues.is_empty(),
            "Mixed-script text should pass: {:?}",
            issues
        );
    }

    // ════════════════════════════════════════════
    // B204: action_playbook type check
    // ════════════════════════════════════════════

    #[test]
    fn test_validate_recipe_json_action_playbook_not_string() {
        let val: Value = serde_json::json!({
            "id": "R001",
            "signals": [{"type": "hiring"}],
            "narrative_template": "Company is hiring",
            "action_playbook": 42
        });
        let errors = validate_recipe_json(&val);
        assert!(
            errors.iter().any(|e| e.contains("action_playbook")),
            "Should flag non-string action_playbook: {:?}",
            errors
        );
    }

    #[test]
    fn test_validate_recipe_json_action_playbook_empty() {
        let val: Value = serde_json::json!({
            "id": "R001",
            "signals": [{"type": "hiring"}],
            "narrative_template": "Company is hiring",
            "action_playbook": ""
        });
        let errors = validate_recipe_json(&val);
        assert!(errors.iter().any(|e| e.contains("action_playbook")));
    }

    // ════════════════════════════════════════════
    // B205: signals item validation
    // ════════════════════════════════════════════

    #[test]
    fn test_validate_recipe_json_signals_non_object_items() {
        let val: Value = serde_json::json!({
            "id": "R001",
            "signals": [42, "string"],
            "narrative_template": "Narrative text here",
            "action_playbook": "Take action"
        });
        let errors = validate_recipe_json(&val);
        assert!(
            errors.iter().any(|e| e.contains("signals[0]")),
            "Should flag non-object signal items: {:?}",
            errors
        );
    }

    #[test]
    fn test_validate_recipe_json_signals_empty_array() {
        let val: Value = serde_json::json!({
            "id": "R001",
            "signals": [],
            "narrative_template": "Narrative text here",
            "action_playbook": "Take action"
        });
        let errors = validate_recipe_json(&val);
        assert!(errors
            .iter()
            .any(|e| e.contains("signals array must not be empty")));
    }

    // ════════════════════════════════════════════
    // B207: refusal detection false positives
    // ════════════════════════════════════════════

    #[test]
    fn test_check_content_quality_no_false_positive_i_can() {
        let text = "I can confirm that the certification has been granted for ISO 27001.";
        let issues = check_content_quality(text);
        assert!(
            !issues.iter().any(|i| i.contains("refusal")),
            "Should not flag 'I can confirm': {:?}",
            issues
        );
    }

    #[test]
    fn test_check_content_quality_no_false_positive_as_an_analyst() {
        let text = "As an analyst reviewing the supply chain data, there are three key findings.";
        let issues = check_content_quality(text);
        assert!(
            !issues.iter().any(|i| i.contains("refusal")),
            "Should not flag 'As an analyst': {:?}",
            issues
        );
    }

    #[test]
    fn test_check_content_quality_actual_refusal_still_detected() {
        let text = "I cannot generate the requested analysis because the data is insufficient.";
        let issues = check_content_quality(text);
        assert!(issues.iter().any(|i| i.contains("refusal")));
    }

    // ════════════════════════════════════════════
    // B209: check_unique_id missing / empty id
    // ════════════════════════════════════════════

    #[test]
    fn test_check_unique_id_missing_id_explicit() {
        let val: Value = serde_json::json!({"name": "test"});
        assert!(
            !check_unique_id(&val, &[]),
            "Missing id should return false"
        );
    }

    #[test]
    fn test_check_unique_id_empty_string_id() {
        let val: Value = serde_json::json!({"id": ""});
        assert!(!check_unique_id(&val, &[]), "Empty id should return false");
    }

    #[test]
    fn test_check_unique_id_whitespace_id() {
        let val: Value = serde_json::json!({"id": "   "});
        assert!(
            !check_unique_id(&val, &[]),
            "Whitespace id should return false"
        );
    }

    #[test]
    fn test_check_unique_id_numeric_id_ignored() {
        let val: Value = serde_json::json!({"id": 123});
        assert!(
            !check_unique_id(&val, &[]),
            "Non-string id should return false"
        );
    }

    // ════════════════════════════════════════════
    // B210: JSON extraction with leading text
    // ════════════════════════════════════════════

    #[test]
    fn test_extract_json_leading_text_before_fence() {
        let raw = "Sure, here is the JSON:\n```json\n{\"key\": \"value\"}\n```";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_leading_text_bare_object() {
        // Leading text before bare JSON — extract_json only finds { at start
        let raw = "Here is the output: {\"key\": 42}";
        // This should return None because the trimmed string doesn't start with {
        assert!(extract_json(raw).is_none());
    }

    #[test]
    fn test_extract_json_leading_whitespace_bare_object() {
        let raw = "  \n  {\"key\": 42}";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, r#"{"key": 42}"#);
    }

    #[test]
    fn test_extract_json_leading_text_with_trailing_text() {
        let raw = "Analysis complete.\n```json\n[1,2,3]\n```\nEnd of response.";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, "[1,2,3]");
    }

    #[test]
    fn test_extract_json_multiple_code_blocks_prefers_first_json_block() {
        let raw = "```text\nnot json\n```\n\n```json\n{\"id\":\"R001\"}\n```\n\n```json\n{\"id\":\"R002\"}\n```";
        let result = extract_json(raw).unwrap();
        assert_eq!(result, "{\"id\":\"R001\"}");
    }

    #[test]
    fn test_validate_insight_json_confidence_must_be_number() {
        let val: Value = serde_json::json!({
            "recipe_id": "R001",
            "entity_id": "E001",
            "narrative": "text",
            "severity": "high",
            "confidence": "0.8"
        });
        let errors = validate_insight_json(&val);
        assert!(errors.iter().any(|e| e.contains("finite number")));
    }

    #[test]
    fn test_validate_stored_llm_output_valid() {
        let val: Value = serde_json::json!({
            "schema_version": "v1",
            "kind": "recipe_response",
            "payload": {"id": "R001"},
            "created_at": "2025-01-01T00:00:00Z"
        });
        let errors = validate_stored_llm_output(&val);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn test_validate_stored_llm_output_invalid_schema() {
        let val: Value = serde_json::json!({
            "schema_version": "1",
            "kind": "",
            "payload": [1,2,3],
            "created_at": ""
        });
        let errors = validate_stored_llm_output(&val);
        assert!(errors.iter().any(|e| e.contains("schema_version")));
        assert!(errors.iter().any(|e| e.contains("kind")));
        assert!(errors.iter().any(|e| e.contains("payload")));
        assert!(errors.iter().any(|e| e.contains("created_at")));
    }
}
