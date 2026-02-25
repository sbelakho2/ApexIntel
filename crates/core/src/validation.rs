use crate::errors::{ApexError, Result};
use std::collections::HashMap;
use url::Url;

const MAX_ID_LEN: usize = 200;
const MAX_URL_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    Empty,
    TooLong,
    Unsafe,
    InvalidFormat,
}

pub fn validate_nonempty_id(id: &str, field: &str) -> Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(ApexError::validation(format!("{field} must not be empty")));
    }
    if trimmed.chars().count() > MAX_ID_LEN {
        return Err(ApexError::validation(format!("{field} must be <= {MAX_ID_LEN} chars")));
    }
    if !is_safe_id(trimmed) {
        return Err(ApexError::validation(format!("{field} contains unsafe characters")));
    }
    Ok(())
}

pub fn is_safe_id(id: &str) -> bool {
    let bad = ["..", "/", "\\", "\u{0000}"];
    !bad.iter().any(|pat| id.contains(pat))
}

pub fn normalize_email(email: &str) -> Option<String> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}

pub fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

pub fn clamp_ratio(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

pub fn clamp_range(value: f64, min: f64, max: f64) -> f64 {
    if !value.is_finite() {
        return min;
    }
    value.clamp(min, max)
}

pub fn normalize_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_URL_LEN {
        return None;
    }
    let mut url = Url::parse(trimmed).ok()?;

    // Normalize scheme + host casing
    let scheme = url.scheme().to_lowercase();
    let host = url.host_str().map(|h| h.to_lowercase());
    if !scheme.is_empty() {
        let _ = url.set_scheme(&scheme);
    }
    if let Some(h) = host {
        let _ = url.set_host(Some(&h));
    }

    // Remove fragment
    url.set_fragment(None);

    // Strip default ports
    if (scheme == "http" && url.port() == Some(80)) || (scheme == "https" && url.port() == Some(443)) {
        let _ = url.set_port(None);
    }

    // Normalize empty path to "/"
    if url.path().is_empty() {
        url.set_path("/");
    }

    // Stable query ordering
    if let Some(query) = url.query() {
        let mut pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        pairs.sort();
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in pairs {
            serializer.append_pair(&k, &v);
        }
        let new_query = serializer.finish();
        if new_query != query {
            url.set_query(Some(&new_query));
        }
    }

    Some(url.to_string())
}

pub fn redact_secrets(message: &str, secrets: &[&str]) -> String {
    let mut redacted = message.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[REDACTED]");
        }
    }
    redacted
}

pub fn validate_map_size<K, V>(map: &HashMap<K, V>, max_entries: usize, name: &str) -> Result<()> {
    if map.len() > max_entries {
        return Err(ApexError::validation(format!("{name} exceeds max size {max_entries}")));
    }
    Ok(())
}

pub fn validate_country_code(code: &str) -> Result<()> {
    let trimmed = code.trim();
    if trimmed.len() != 2 || !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApexError::validation("invalid country code"));
    }
    Ok(())
}

pub fn validate_region_code(code: &str) -> Result<()> {
    let trimmed = code.trim();
    if trimmed.len() < 2 || trimmed.len() > 4 {
        return Err(ApexError::validation("invalid region code"));
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(ApexError::validation("invalid region code"));
    }
    Ok(())
}

/// Validate a probability-like numeric value is finite and within `[0, 1]` (B303).
pub fn validate_probability(value: f64, field: &str) -> Result<f64> {
    if !value.is_finite() {
        return Err(ApexError::validation(format!("{field} must be finite")));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(ApexError::validation(format!("{field} must be in [0, 1]")));
    }
    Ok(value)
}

// ────────────────────────────────────────────
// Arithmetic helpers (B276, B277, B278)
// ────────────────────────────────────────────

/// Perform safe division, returning `0.0` when the divisor is effectively zero (B278).
///
/// The zero threshold is `f64::EPSILON * 2.0` — small enough that legitimate
/// sub-epsilon denominators are treated as zero, preventing astronomically large
/// but meaningless quotients.  Non-finite inputs (NaN, ±∞) also return `0.0`.
///
/// # Examples
/// ```
/// use apex_core::validation::safe_div;
/// assert_eq!(safe_div(10.0, 2.0), 5.0);
/// assert_eq!(safe_div(1.0, 0.0), 0.0);
/// assert_eq!(safe_div(f64::NAN, 1.0), 0.0);
/// ```
pub fn safe_div(numerator: f64, denominator: f64) -> f64 {
    if !numerator.is_finite() || !denominator.is_finite() {
        return 0.0;
    }
    if denominator.abs() < f64::EPSILON * 2.0 {
        return 0.0;
    }
    numerator / denominator
}

/// Round a floating-point score to `decimal_places` decimal places (B277).
///
/// Uses the multiply-round-divide idiom which is numerically stable for values
/// in the displayable range `[−1.0e9, 1.0e9]`.  Non-finite values (`NaN`, `±∞`)
/// are returned unchanged so callers can detect them explicitly.
///
/// # Examples
/// ```
/// use apex_core::validation::round_to_dp;
/// assert_eq!(round_to_dp(0.123456, 3), 0.123);
/// assert_eq!(round_to_dp(0.9999, 2), 1.0);
/// assert!(round_to_dp(f64::NAN, 2).is_nan());
/// ```
pub fn round_to_dp(value: f64, decimal_places: u32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let factor = 10_f64.powi(decimal_places as i32);
    (value * factor).round() / factor
}

/// Convert a Unix timestamp in seconds to whole days with overflow protection (B276).
///
/// Returns `None` when the arithmetic would overflow `i64`, which can occur for
/// very large or very negative timestamps.  Callers should treat `None` as an
/// invalid/unusable timestamp rather than silently wrapping or panicking.
///
/// Integer division truncates toward zero (i.e., day 0 spans `[0, 86399]`).
///
/// # Examples
/// ```
/// use apex_core::validation::timestamp_to_days_checked;
/// assert_eq!(timestamp_to_days_checked(86_400), Some(1));
/// assert_eq!(timestamp_to_days_checked(0), Some(0));
/// assert_eq!(timestamp_to_days_checked(-86_400), Some(-1));
/// assert_eq!(timestamp_to_days_checked(1_700_000_000), Some(1_700_000_000 / 86_400));
/// ```
pub fn timestamp_to_days_checked(unix_secs: i64) -> Option<i64> {
    const SECS_PER_DAY: i64 = 86_400;
    // checked_div guards against i64::MIN / -1 overflow (undefined behaviour
    // in C, but Rust panics in debug mode and wraps in release — neither is
    // desirable for a time-conversion utility).
    unix_secs.checked_div(SECS_PER_DAY)
}

// ────────────────────────────────────────────
// String trimming / Unicode helpers (B279, B307)
// ────────────────────────────────────────────

/// Trim leading/trailing whitespace from a user-provided string (B279).
///
/// Unlike `str::trim()` (ASCII-only in practice), this removes all Unicode
/// `White_Space` code points including U+00A0 (non-breaking space), U+FEFF
/// (BOM / zero-width no-break space), and U+200B (zero-width space).
/// Returns an owned `String` suitable for further validation.
pub fn trim_user_string(s: &str) -> String {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}' || c == '\u{200B}')
        .to_string()
}

/// Collapse internal runs of Unicode whitespace into a single ASCII space,
/// then trim the result (B307).
///
/// Handles all Unicode `White_Space` characters plus zero-width joiners/
/// non-joiners (U+200B–U+200D) and the BOM (U+FEFF).  Equivalent to
/// `normalize_whitespace` in the `parse` crate but operating on full Unicode
/// rather than just ASCII whitespace.
pub fn normalize_unicode_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        let is_ws = ch.is_whitespace()
            || ch == '\u{FEFF}'
            || ch == '\u{200B}'
            || ch == '\u{200C}'
            || ch == '\u{200D}';
        if is_ws {
            if !in_space {
                result.push(' ');
                in_space = true;
            }
        } else {
            result.push(ch);
            in_space = false;
        }
    }
    result.trim_matches(' ').to_string()
}

// ────────────────────────────────────────────
// UUID validation (B282)
// ────────────────────────────────────────────

// ────────────────────────────────────────────
// Safe string concatenation (B280)
// ────────────────────────────────────────────

/// Concatenate `parts` with `separator` between each element, with a
/// pre-allocation total-length guard (B280).
///
/// `max_chars` is measured in **Unicode scalar values** (code points), not
/// bytes, so the bound is encoding-independent.  The length check is performed
/// entirely with lightweight `chars().count()` arithmetic *before* any heap
/// allocation is committed; oversized inputs are rejected without touching the
/// allocator.
///
/// Empty `parts` slices return an empty string immediately.  Individual parts
/// are joined as-is; callers that need trimming should pre-process with
/// [`trim_user_string`].
///
/// # Errors
///
/// Returns `Err(ApexError::Validation)` when the combined character count
/// (parts + separators) would exceed `max_chars`.  Arithmetic uses
/// `saturating_add` / `saturating_mul` to avoid wrapping on pathological input.
///
/// # Examples
///
/// ```
/// use apex_core::validation::safe_concat;
///
/// // Normal join
/// let s = safe_concat(&["hello", "world"], ", ", 100).unwrap();
/// assert_eq!(s, "hello, world");
///
/// // Empty slice returns empty string
/// let empty = safe_concat(&[], " ", 100).unwrap();
/// assert_eq!(empty, "");
///
/// // Exceeds limit → error (no allocation)
/// let long = "x".repeat(60);
/// let err = safe_concat(&[long.as_str(), long.as_str()], "", 100);
/// assert!(err.is_err());
/// ```
pub fn safe_concat(parts: &[&str], separator: &str, max_chars: usize) -> Result<String> {
    if parts.is_empty() {
        return Ok(String::new());
    }

    // Count total Unicode code points before allocating.
    let sep_char_count: usize = separator.chars().count();
    // (n − 1) separators for n parts; saturating arithmetic prevents overflow on enormous n.
    let sep_total: usize = sep_char_count.saturating_mul(parts.len().saturating_sub(1));
    let parts_total: usize = parts
        .iter()
        .map(|p| p.chars().count())
        .try_fold(0usize, |acc, n| acc.checked_add(n))
        .unwrap_or(usize::MAX); // checked_add fails only if the total overflows usize
    let total: usize = parts_total.saturating_add(sep_total);

    if total > max_chars {
        return Err(ApexError::validation(format!(
            "concatenated string would be {total} chars, exceeding the {max_chars} char limit"
        )));
    }

    // Now safe to allocate — capacity == byte length, which may exceed
    // char count for multi-byte code points, but is always an upper bound.
    let byte_cap: usize = parts.iter().map(|p| p.len()).sum::<usize>()
        + separator.len().saturating_mul(parts.len().saturating_sub(1));
    let mut result = String::with_capacity(byte_cap);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push_str(separator);
        }
        result.push_str(part);
    }
    Ok(result)
}

/// Validate that `id` is a syntactically correct UUID (any version) (B282).
///
/// Accepts both the canonical hyphenated form
/// (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`) and the compact non-hyphenated
/// form (`xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`).  The `field` name is included in
/// the returned error message for actionable diagnostics.
///
/// # Examples
/// ```
/// use apex_core::validation::validate_uuid;
/// assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000", "id").is_ok());
/// assert!(validate_uuid("not-a-uuid", "id").is_err());
/// assert!(validate_uuid("", "id").is_err());
/// ```
pub fn validate_uuid(id: &str, field: &str) -> Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(ApexError::validation(format!("{field} must not be empty")));
    }
    uuid::Uuid::parse_str(trimmed)
        .map(|_| ())
        .map_err(|_| ApexError::validation(format!("{field} is not a valid UUID: got '{trimmed}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_nonempty_id() {
        assert!(validate_nonempty_id("abc", "id").is_ok());
        assert!(validate_nonempty_id("  ", "id").is_err());
    }

    #[test]
    fn test_normalize_email() {
        assert_eq!(normalize_email(" Test@Example.COM "), Some("test@example.com".to_string()));
        assert_eq!(normalize_email("   "), None);
    }

    #[test]
    fn test_normalize_url() {
        let url = normalize_url("https://Example.com:443/path/?b=2&a=1#frag").unwrap();
        assert!(url.starts_with("https://example.com/path/"));
        assert!(!url.contains('#'));
        assert!(url.contains("a=1"));
    }

    #[test]
    fn test_validate_uuid_accepts_hyphenated_and_compact() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000", "id").is_ok());
        assert!(validate_uuid("550e8400e29b41d4a716446655440000", "id").is_ok());
    }

    #[test]
    fn test_validate_uuid_rejects_empty_and_garbage() {
        assert!(validate_uuid(" ", "id").is_err());
        assert!(validate_uuid("not-a-uuid", "id").is_err());
    }

    #[test]
    fn validate_probability_accepts_bounds_and_midpoint() {
        assert_eq!(validate_probability(0.0, "p").unwrap(), 0.0);
        assert_eq!(validate_probability(0.5, "p").unwrap(), 0.5);
        assert_eq!(validate_probability(1.0, "p").unwrap(), 1.0);
    }

    #[test]
    fn validate_probability_rejects_nan_inf_and_out_of_range() {
        assert!(validate_probability(f64::NAN, "p").is_err());
        assert!(validate_probability(f64::INFINITY, "p").is_err());
        assert!(validate_probability(-0.01, "p").is_err());
        assert!(validate_probability(1.01, "p").is_err());
    }

    #[test]
    fn test_clamp_ratio() {
        assert_eq!(clamp_ratio(1.2), 1.0);
        assert_eq!(clamp_ratio(-0.2), 0.0);
        assert_eq!(clamp_ratio(f64::NAN), 0.0);
    }

    // ── B278: safe_div ──────────────────────────────────────

    #[test]
    fn safe_div_normal_case() {
        assert_eq!(safe_div(10.0, 2.0), 5.0);
    }

    #[test]
    fn safe_div_zero_denominator_returns_zero() {
        assert_eq!(safe_div(99.0, 0.0), 0.0);
    }

    #[test]
    fn safe_div_negative_denominator_works() {
        assert_eq!(safe_div(6.0, -2.0), -3.0);
    }

    #[test]
    fn safe_div_nan_numerator_returns_zero() {
        assert_eq!(safe_div(f64::NAN, 2.0), 0.0);
    }

    #[test]
    fn safe_div_nan_denominator_returns_zero() {
        assert_eq!(safe_div(2.0, f64::NAN), 0.0);
    }

    #[test]
    fn safe_div_inf_numerator_returns_zero() {
        assert_eq!(safe_div(f64::INFINITY, 2.0), 0.0);
    }

    #[test]
    fn safe_div_sub_epsilon_denominator_returns_zero() {
        assert_eq!(safe_div(1.0, f64::EPSILON * 0.5), 0.0);
    }

    #[test]
    fn safe_div_one_over_one() {
        assert_eq!(safe_div(1.0, 1.0), 1.0);
    }

    // ── B277: round_to_dp ───────────────────────────────────

    #[test]
    fn round_to_dp_three_places() {
        assert_eq!(round_to_dp(0.123456, 3), 0.123);
    }

    #[test]
    fn round_to_dp_rounds_up() {
        assert_eq!(round_to_dp(0.9999, 2), 1.0);
    }

    #[test]
    fn round_to_dp_zero_places() {
        assert_eq!(round_to_dp(2.7, 0), 3.0);
    }

    #[test]
    fn round_to_dp_negative_value() {
        assert_eq!(round_to_dp(-0.555, 2), -0.56);
    }

    #[test]
    fn round_to_dp_nan_passthrough() {
        assert!(round_to_dp(f64::NAN, 3).is_nan());
    }

    #[test]
    fn round_to_dp_inf_passthrough() {
        assert_eq!(round_to_dp(f64::INFINITY, 3), f64::INFINITY);
    }

    #[test]
    fn round_to_dp_zero_is_zero() {
        assert_eq!(round_to_dp(0.0, 4), 0.0);
    }

    // ── B276: timestamp_to_days_checked ─────────────────────

    #[test]
    fn timestamp_to_days_one_day() {
        assert_eq!(timestamp_to_days_checked(86_400), Some(1));
    }

    #[test]
    fn timestamp_to_days_epoch() {
        assert_eq!(timestamp_to_days_checked(0), Some(0));
    }

    #[test]
    fn timestamp_to_days_partial_day_truncates() {
        assert_eq!(timestamp_to_days_checked(86_399), Some(0));
    }

    #[test]
    fn timestamp_to_days_negative_timestamp() {
        // -1 second = day 0 (truncation toward zero)
        assert_eq!(timestamp_to_days_checked(-1), Some(0));
    }

    #[test]
    fn timestamp_to_days_negative_full_day() {
        assert_eq!(timestamp_to_days_checked(-86_400), Some(-1));
    }

    #[test]
    fn timestamp_to_days_i64_min_does_not_panic() {
        // i64::MIN / 86_400 is fine — no overflow since 86_400 is positive
        let result = timestamp_to_days_checked(i64::MIN);
        // Should return Some and be a very large negative number of days
        assert!(result.is_some());
        assert!(result.unwrap() < 0);
    }

    #[test]
    fn timestamp_to_days_large_timestamp() {
        let days = timestamp_to_days_checked(1_700_000_000); // ~2023
        assert_eq!(days, Some(1_700_000_000 / 86_400));
    }

    // ── B279: trim_user_string ──────────────────────────────

    #[test]
    fn trim_user_string_ascii_spaces() {
        assert_eq!(trim_user_string("  hello  "), "hello");
    }

    #[test]
    fn trim_user_string_nbsp() {
        // U+00A0 non-breaking space
        let s = "\u{00A0}hello\u{00A0}";
        assert_eq!(trim_user_string(s), "hello");
    }

    #[test]
    fn trim_user_string_bom() {
        let s = "\u{FEFF}hello";
        assert_eq!(trim_user_string(s), "hello");
    }

    #[test]
    fn trim_user_string_zwsp() {
        let s = "\u{200B}hello\u{200B}";
        assert_eq!(trim_user_string(s), "hello");
    }

    #[test]
    fn trim_user_string_empty_remains_empty() {
        assert_eq!(trim_user_string(""), "");
    }

    #[test]
    fn trim_user_string_only_whitespace_becomes_empty() {
        assert_eq!(trim_user_string("   \t\n  "), "");
    }

    #[test]
    fn trim_user_string_internal_spaces_untouched() {
        assert_eq!(trim_user_string("  hello world  "), "hello world");
    }

    // ── B307: normalize_unicode_whitespace ──────────────────

    #[test]
    fn normalize_unicode_ws_collapses_nbsp() {
        let s = "hello\u{00A0}\u{00A0}world";
        assert_eq!(normalize_unicode_whitespace(s), "hello world");
    }

    #[test]
    fn normalize_unicode_ws_collapses_mixed() {
        let s = "foo  \t\u{00A0} bar";
        assert_eq!(normalize_unicode_whitespace(s), "foo bar");
    }

    #[test]
    fn normalize_unicode_ws_trims_leading_trailing() {
        assert_eq!(normalize_unicode_whitespace("  hello  "), "hello");
    }

    #[test]
    fn normalize_unicode_ws_empty_input() {
        assert_eq!(normalize_unicode_whitespace(""), "");
    }

    #[test]
    fn normalize_unicode_ws_all_whitespace() {
        assert_eq!(normalize_unicode_whitespace("   \t\n  "), "");
    }

    // ── B282: validate_uuid ─────────────────────────────────

    #[test]
    fn validate_uuid_valid_v4() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000", "id").is_ok());
    }

    #[test]
    fn validate_uuid_valid_no_hyphens() {
        assert!(validate_uuid("550e8400e29b41d4a716446655440000", "id").is_ok());
    }

    #[test]
    fn validate_uuid_empty_is_err() {
        assert!(validate_uuid("", "id").is_err());
    }

    #[test]
    fn validate_uuid_whitespace_only_is_err() {
        assert!(validate_uuid("   ", "id").is_err());
    }

    #[test]
    fn validate_uuid_random_string_is_err() {
        assert!(validate_uuid("not-a-uuid", "id").is_err());
    }

    #[test]
    fn validate_uuid_too_short_is_err() {
        assert!(validate_uuid("550e8400-e29b-41d4", "id").is_err());
    }

    #[test]
    fn validate_uuid_error_contains_field_name() {
        let err = validate_uuid("bad", "entity_id").unwrap_err();
        assert!(err.to_string().contains("entity_id"));
    }

    #[test]
    fn validate_uuid_trimmed_before_check() {
        // Leading/trailing whitespace should not cause rejection
        assert!(validate_uuid("  550e8400-e29b-41d4-a716-446655440000  ", "id").is_ok());
    }

    // ── B280: safe_concat ───────────────────────────────────

    #[test]
    fn safe_concat_normal_join() {
        let s = safe_concat(&["hello", "world"], ", ", 100).unwrap();
        assert_eq!(s, "hello, world");
    }

    #[test]
    fn safe_concat_single_element_no_separator() {
        let s = safe_concat(&["only"], ", ", 100).unwrap();
        assert_eq!(s, "only");
    }

    #[test]
    fn safe_concat_empty_slice_returns_empty() {
        let s = safe_concat(&[], " ", 100).unwrap();
        assert_eq!(s, "");
    }

    #[test]
    fn safe_concat_empty_separator() {
        let s = safe_concat(&["ab", "cd", "ef"], "", 10).unwrap();
        assert_eq!(s, "abcdef");
    }

    #[test]
    fn safe_concat_exactly_at_limit() {
        // "ab" + "cd" = 4 chars; limit = 4 — should succeed
        let s = safe_concat(&["ab", "cd"], "", 4).unwrap();
        assert_eq!(s, "abcd");
    }

    #[test]
    fn safe_concat_one_over_limit_returns_err() {
        // "ab" + "cd" = 4 chars; limit = 3 — should fail
        let err = safe_concat(&["ab", "cd"], "", 3);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("4") && msg.contains("3"));
    }

    #[test]
    fn safe_concat_separator_counted_in_total() {
        // "a" + ", " + "b" = 4 chars; limit = 3 — separator pushes over
        let err = safe_concat(&["a", "b"], ", ", 3);
        assert!(err.is_err());
    }

    #[test]
    fn safe_concat_unicode_multibyte_counted_as_chars_not_bytes() {
        // Each "😀" is 4 bytes but 1 char; total chars = 3.
        let s = safe_concat(&["😀", "😀", "😀"], "", 3).unwrap();
        assert_eq!(s, "😀😀😀");
        // Byte length would be 12 — should still pass char limit of 3.
    }

    #[test]
    fn safe_concat_unicode_multibyte_over_char_limit_fails() {
        // 4 chars ("😀😀😀😀"), limit 3 — should fail
        let err = safe_concat(&["😀😀", "😀😀"], "", 3);
        assert!(err.is_err());
    }

    #[test]
    fn safe_concat_very_long_parts_rejected_before_allocation() {
        let long = "x".repeat(1_000);
        let err = safe_concat(&[long.as_str(), long.as_str()], "", 100);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("2000") && msg.contains("100"));
    }

    #[test]
    fn safe_concat_all_empty_parts_passes() {
        let s = safe_concat(&["", "", ""], "-", 10).unwrap();
        // Empty parts joined with "-" → "--"  (2 separators, 0 content)
        assert_eq!(s, "--");
    }

    // ── B287: empty input tests ──

    #[test]
    fn normalize_email_empty_string_returns_none() {
        // Totally empty input → None (no valid email possible)
        assert_eq!(normalize_email(""), None);
    }

    #[test]
    fn validate_nonempty_id_empty_string_is_err() {
        // Empty string is not a valid non-empty ID
        assert!(validate_nonempty_id("", "id").is_err());
    }

    #[test]
    fn trim_user_string_empty_string_returns_empty() {
        // Already covered by trim_user_string_empty_remains_empty but verify here too
        assert_eq!(trim_user_string(""), "");
    }

    // ── B288: boundary condition tests ──

    #[test]
    fn clamp_ratio_at_exact_lower_bound() {
        // 0.0 is the lower boundary — must be preserved exactly, not clamped
        assert_eq!(clamp_ratio(0.0), 0.0);
    }

    #[test]
    fn clamp_ratio_at_exact_upper_bound() {
        // 1.0 is the upper boundary — must be preserved exactly, not raised or lowered
        assert_eq!(clamp_ratio(1.0), 1.0);
    }

    #[test]
    fn clamp_ratio_just_inside_bounds() {
        // Values in (0, 1) must pass through unchanged
        let v = 0.5 - f64::EPSILON;
        assert_eq!(clamp_ratio(v), v);
        let v2 = 0.5 + f64::EPSILON;
        assert_eq!(clamp_ratio(v2), v2);
    }

    #[test]
    fn clamp_ratio_negative_inf_clamped_to_zero() {
        // Non-finite values (including ±Inf) are treated as missing data → 0.0
        assert_eq!(clamp_ratio(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn clamp_ratio_positive_inf_returns_zero_not_one() {
        // IMPORTANT: the implementation returns 0.0 for ANY non-finite value,
        // including +∞.  Callers expecting +∞ → 1.0 should use clamp_range instead.
        assert_eq!(clamp_ratio(f64::INFINITY), 0.0);
    }
}
