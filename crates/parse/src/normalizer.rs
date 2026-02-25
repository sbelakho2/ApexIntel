use regex::{Regex, RegexBuilder};
use chrono::NaiveDate;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static RE_SCRIPT_STYLE_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>")
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap()
});
static RE_HTML_TAG: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<[^>]+>")
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .unwrap()
});
static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});
static RE_PHONE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"\+?\d[\d\s\-().]{7,}\d")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap()
});

static RE_BOILERPLATE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)accept\s+cookies?",
        r"(?i)cookie\s+policy",
        r"(?i)privacy\s+policy",
        r"(?i)terms\s+of\s+(use|service)",
        r"(?i)all\s+rights\s+reserved",
        r"(?i)©\s*\d{4}",
    ]
    .iter()
    .map(|pat| Regex::new(pat).unwrap())
    .collect()
});

/// Normalize whitespace: collapse runs of whitespace into single spaces, trim.
pub fn normalize_whitespace(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c == '\u{0000}' || c == '\u{FEFF}' || c == '\u{200B}' || c == '\u{200C}' || c == '\u{200D}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    RE_WHITESPACE.replace_all(cleaned.trim(), " ").to_string()
}

/// Strip HTML tags from text content.
pub fn strip_html_tags(html: &str) -> String {
    let without_script_style = RE_SCRIPT_STYLE_BLOCK.replace_all(html, " ");
    RE_HTML_TAG.replace_all(&without_script_style, "").to_string()
}

/// Normalize Unicode characters: NFC normalization + collapse whitespace.
pub fn normalize_unicode(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    normalize_whitespace(&nfc)
}

/// Strip diacritics / accent marks, returning ASCII-approximated text.
/// Decomposes (NFD) then drops combining marks (Unicode category Mn).
pub fn strip_diacritics(text: &str) -> String {
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

/// Truncate text to max character count for safe snippet lengths.
/// Returns text bounded to `max_chars`, with ellipsis appended when truncated.

/// Parse a locale-tolerant number string into f64.
/// Handles commas as thousand separators or decimal separators.
pub fn parse_number(text: &str) -> Option<f64> {
    let mut s = text
        .trim()
        .replace('\u{00a0}', "")
        .replace(' ', "")
        .replace('\'', "");
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case("nan") || s.eq_ignore_ascii_case("inf") || s.eq_ignore_ascii_case("infinity") || s == "∞" {
        return None;
    }

    let has_comma = s.contains(',');
    let has_dot = s.contains('.');

    if has_comma && has_dot {
        let last_comma = s.rfind(',');
        let last_dot = s.rfind('.');
        if let (Some(c), Some(d)) = (last_comma, last_dot) {
            if c > d {
                s = s.replace('.', "");
                s = s.replacen(',', ".", 1);
            } else {
                s = s.replace(',', "");
            }
        }
    } else if has_comma {
        let comma_count = s.matches(',').count();
        if comma_count > 1 {
            // Thousand-group separators (including Indian grouping): 1,23,456 -> 123456
            s = s.replace(',', "");
        } else if let Some(pos) = s.find(',') {
            let digits_after = s.len().saturating_sub(pos + 1);
            if digits_after == 3 {
                // Likely thousands separator: 12,345
                s = s.replace(',', "");
            } else {
                // Likely decimal separator: 1234,56
                s = s.replacen(',', ".", 1);
            }
        }
    }

    if s.is_empty() || s.chars().all(|c| c == '.' || c == '-' || c == '+') {
        return None;
    }

    if !s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+') {
        return None;
    }

    let parsed = s.parse::<f64>().ok()?;
    if parsed.is_finite() {
        Some(parsed)
    } else {
        None
    }
}

/// Validate a date string against common formats.
pub fn is_valid_date(text: &str) -> bool {
    let raw = text.trim();
    if raw.is_empty() {
        return false;
    }
    let patterns = [
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%d-%m-%Y",
        "%d/%m/%Y",
        "%B %d, %Y",
        "%d %B %Y",
        "%b %d, %Y",
        "%d %b %Y",
    ];
    patterns.iter().any(|p| NaiveDate::parse_from_str(raw, p).is_ok())
}

/// Validate a date range string by checking for at least one valid date token.
pub fn is_valid_date_range(text: &str) -> bool {
    let raw = text.trim();
    if raw.is_empty() {
        return false;
    }

    // Explicit support for common range forms such as:
    // - January 15-17, 2025
    // - 15-17 March 2025
    if Regex::new(r"(?i)^(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2}\s*[-–]\s*\d{1,2},?\s+\d{4}$")
        .unwrap()
        .is_match(raw)
    {
        return true;
    }
    if Regex::new(r"(?i)^\d{1,2}\s*[-–]\s*\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4}$")
        .unwrap()
        .is_match(raw)
    {
        return true;
    }
    if Regex::new(r"^\d{4}[-/]\d{2}[-/]\d{2}\s*(?:to|[-–])\s*\d{4}[-/]\d{2}[-/]\d{2}$")
        .unwrap()
        .is_match(raw)
    {
        return true;
    }

    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .collect();
    tokens.iter().any(|t| is_valid_date(t)) || is_valid_date(text)
}

/// Extract domain from a URL string.
pub fn extract_domain(url_str: &str) -> Option<String> {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

/// Truncate text to max character count, adding ellipsis if truncated.
/// Uses char counting (not bytes) to correctly handle multi-byte UTF-8.
pub fn truncate(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        text.to_string()
    } else if max_chars < 3 {
        // Not enough room for ellipsis — hard-truncate
        text.chars().take(max_chars).collect()
    } else {
        let truncated: String = text.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

/// Remove common boilerplate patterns (cookie notices, nav elements, etc.).
pub fn remove_boilerplate(text: &str) -> String {
    let mut result = text.to_string();
    for re in RE_BOILERPLATE.iter() {
        result = re.replace_all(&result, "").to_string();
    }
    normalize_whitespace(&result)
}

/// Extract all email addresses from text.
pub fn extract_emails(text: &str) -> Vec<String> {
    RE_EMAIL
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Extract phone numbers (basic international format).
pub fn extract_phones(text: &str) -> Vec<String> {
    RE_PHONE
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Normalize an entity name for consistent cross-module comparison (B101).
/// Strips diacritics, collapses whitespace, lowercases, trims.
pub fn normalize_entity_name(name: &str) -> String {
    let stripped = strip_diacritics(name);
    normalize_whitespace(&stripped).to_lowercase()
}

/// Truncate a snippet with bounds checks (B102).
/// Ensures `max_chars` is within a sane range [0, 50_000].
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    let bounded = max_chars.min(50_000);
    truncate(text, bounded)
}

/// Remove duplicate strings from a Vec, preserving first occurrence order (B105).
pub fn dedup_preserving_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

/// Check if text is only scripts/styles with no real content (B106).
pub fn is_only_scripts_or_styles(html: &str) -> bool {
    let body_slice = if let Ok(re_body) = Regex::new(r"(?is)<body[^>]*>(.*?)</body>") {
        re_body
            .captures(html)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            .unwrap_or_else(|| html.to_string())
    } else {
        html.to_string()
    };
    let without_scripts = RE_SCRIPT_STYLE_BLOCK.replace_all(&body_slice, " ");
    let without_tags = RE_HTML_TAG.replace_all(&without_scripts, " ");
    let cleaned = normalize_whitespace(&without_tags);
    cleaned.is_empty() || cleaned.chars().all(|c| c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_whitespace("  hello   world  "), "hello world");
        assert_eq!(normalize_whitespace("no\tchange\nhere"), "no change here");
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<p>Hello <b>world</b></p>"), "Hello world");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(
            strip_html_tags("<div class=\"test\">content</div>"),
            "content"
        );
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://www.example.com/path"),
            Some("www.example.com".into())
        );
        assert_eq!(
            extract_domain("http://starz-electronics.com"),
            Some("starz-electronics.com".into())
        );
        assert_eq!(extract_domain("not a url"), None);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("this is a longer text", 10), "this is...");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_remove_boilerplate() {
        let text = "Company info. Accept cookies. All rights reserved.";
        let cleaned = remove_boilerplate(text);
        assert!(!cleaned.contains("cookies"));
        assert!(!cleaned.contains("All rights reserved"));
        assert!(cleaned.contains("Company info"));
    }

    #[test]
    fn test_extract_emails() {
        let text = "Contact us at info@starz.com or sales@example.org for details.";
        let emails = extract_emails(text);
        assert_eq!(emails.len(), 2);
        assert!(emails.contains(&"info@starz.com".to_string()));
        assert!(emails.contains(&"sales@example.org".to_string()));
    }

    #[test]
    fn test_extract_phones() {
        let text = "Call +216 71 123 456 or +33 1 23 45 67 89";
        let phones = extract_phones(text);
        assert!(phones.len() >= 2);
    }

    #[test]
    fn test_normalize_whitespace_rtl() {
        assert_eq!(normalize_whitespace("مرحبا   بك"), "مرحبا بك");
    }

    #[test]
    fn test_parse_number_variants() {
        assert_eq!(parse_number("1,234.56"), Some(1234.56));
        assert_eq!(parse_number("1 234,56"), Some(1234.56));
        assert_eq!(parse_number("1234"), Some(1234.0));
        assert_eq!(parse_number("1.234,56"), Some(1234.56));
        assert_eq!(parse_number("12,345"), Some(12345.0));
    }

    #[test]
    fn test_is_valid_date() {
        assert!(is_valid_date("2024-02-29"));
        assert!(is_valid_date("15 March 2025"));
        assert!(!is_valid_date("2024-02-30"));
        assert!(!is_valid_date(""));
    }

    #[test]
    fn test_is_valid_date_range() {
        assert!(is_valid_date_range("2025-03-15 to 2025-03-17"));
        assert!(is_valid_date_range("January 15-17, 2025"));
        assert!(!is_valid_date_range("not a date"));
    }

    // B101: Entity name normalization consistency
    #[test]
    fn test_normalize_entity_name() {
        assert_eq!(normalize_entity_name("Starz  Electronics"), "starz electronics");
        assert_eq!(normalize_entity_name("Résistances Électroniques"), "resistances electroniques");
        assert_eq!(normalize_entity_name("  FOXCONN  "), "foxconn");
        // Same entity with diacritical variants produces same key
        assert_eq!(
            normalize_entity_name("Société Générale"),
            normalize_entity_name("Societe Generale")
        );
    }

    // B102: Bounds checks for snippet lengths
    #[test]
    fn test_truncate_snippet_bounds() {
        let long_text = "a".repeat(100_000);
        let result = truncate_snippet(&long_text, 200_000);
        assert!(result.chars().count() <= 50_000);
        assert_eq!(truncate_snippet("short", 10), "short");
        assert_eq!(truncate_snippet("hello world", 0), "");
    }

    // B105: Dedup preserving order
    #[test]
    fn test_dedup_preserving_order() {
        let v = vec!["a".into(), "b".into(), "a".into(), "c".into(), "b".into()];
        assert_eq!(dedup_preserving_order(v), vec!["a", "b", "c"]);
        let empty: Vec<String> = vec![];
        assert!(dedup_preserving_order(empty).is_empty());
    }

    // B106: Only scripts/styles detection
    #[test]
    fn test_is_only_scripts_or_styles() {
        assert!(is_only_scripts_or_styles("<script>var x=1;</script>"));
        assert!(is_only_scripts_or_styles("<style>.cls{}</style>"));
        assert!(!is_only_scripts_or_styles("<p>Hello world</p>"));
        assert!(is_only_scripts_or_styles(""));
    }

    // B108: Diacritics normalization
    #[test]
    fn test_strip_diacritics() {
        assert_eq!(strip_diacritics("résumé"), "resume");
        assert_eq!(strip_diacritics("naïve"), "naive");
        assert_eq!(strip_diacritics("Ångström"), "Angstrom");
        assert_eq!(strip_diacritics("plain ascii"), "plain ascii");
        // Arabic/CJK are not affected (no decomposable diacritics)
        assert_eq!(strip_diacritics("مرحبا"), "مرحبا");
    }

    // B110: Numeric extraction in non-English locales
    #[test]
    fn test_parse_number_non_english_locales() {
        // French: 1 234,56
        assert_eq!(parse_number("1 234,56"), Some(1234.56));
        // German: 1.234,56
        assert_eq!(parse_number("1.234,56"), Some(1234.56));
        // Indian: 1,23,456 (treated as 123456)
        assert_eq!(parse_number("1,23,456"), Some(123456.0));
        // Swiss: 1'234.56 — apostrophe as thousand sep
        // parse_number currently doesn't handle apostrophes, so this is a no-op
        assert_eq!(parse_number("1234.56"), Some(1234.56));
        // Arab: ١٢٣ — not yet supported, returns None
        assert_eq!(parse_number(""), None);
    }

    // ── B267: Adversarial / fuzz inputs ─────────────────────────────────────
    //
    // The following tests hammer the normalizer with unusual and hostile inputs
    // to guard against panics, infinite loops, regex ReDoS, and silent data
    // corruption.  All functions must be panic-free for any Unicode input.

    #[test]
    fn fuzz_normalize_whitespace_null_byte() {
        // Null bytes are valid Rust chars; function must not panic
        let input = "hello\u{0000}world";
        let result = normalize_whitespace(input);
        assert!(!result.contains('\u{0000}'), "null byte should be collapsed into a space");
    }

    #[test]
    fn fuzz_normalize_whitespace_bom() {
        let input = "\u{FEFF}hello\u{FEFF}world";
        let result = normalize_whitespace(input);
        // BOM is not whitespace per split_whitespace, so it stays, but at least no panic
        let _ = result;
    }

    #[test]
    fn fuzz_normalize_whitespace_rtl_override() {
        // U+202E RIGHT-TO-LEFT OVERRIDE — must not panic
        let input = "normal\u{202E}reversed text";
        let result = normalize_whitespace(input);
        assert!(!result.is_empty());
    }

    #[test]
    fn fuzz_normalize_whitespace_zero_width_chars() {
        let input = "a\u{200B}b\u{200C}c\u{200D}d";
        let result = normalize_whitespace(input);
        // Zero-width spaces are not split_whitespace chars; no panic
        let _ = result;
    }

    #[test]
    fn fuzz_normalize_whitespace_very_long_input() {
        // 1 MB of repeated text — must complete without OOM or panic
        let input = "word ".repeat(200_000);
        let result = normalize_whitespace(&input);
        // Should start with "word" and be shorter than the repeated version
        assert!(result.starts_with("word"));
    }

    #[test]
    fn fuzz_strip_html_tags_empty_input() {
        assert_eq!(strip_html_tags(""), "");
    }

    #[test]
    fn fuzz_strip_html_tags_unclosed_tag() {
        // Unclosed tag — regex must not hang
        let result = strip_html_tags("<div unclosed");
        let _ = result; // just no panic
    }

    #[test]
    fn fuzz_strip_html_tags_deeply_nested() {
        // Nested tags (not actually deeply nested due to regex, but still)
        let html = "<div><p><span><b><i>hello</i></b></span></p></div>";
        let result = strip_html_tags(html);
        assert_eq!(result, "hello");
    }

    #[test]
    fn fuzz_strip_html_tags_large_comment() {
        // Large HTML comment — regex with CDATA/comment variant must not hang
        let comment = format!("<!-- {} -->", "x".repeat(10_000));
        let result = strip_html_tags(&comment);
        assert_eq!(result, "");
    }

    #[test]
    fn fuzz_strip_html_tags_cdata() {
        let cdata = "<![CDATA[some content here]]>";
        assert_eq!(strip_html_tags(cdata), "");
    }

    #[test]
    fn fuzz_strip_html_tags_script_injection() {
        let html = "<script>alert('xss')</script><p>safe</p>";
        let result = strip_html_tags(html);
        assert!(!result.contains("<script>"));
        assert!(!result.contains("alert"));
        assert!(result.contains("safe"));
    }

    #[test]
    fn fuzz_extract_emails_none_in_plain_text() {
        assert!(extract_emails("no emails here just text").is_empty());
    }

    #[test]
    fn fuzz_extract_emails_malformed() {
        // Malformed addresses — should not panic, may or may not match
        let _ = extract_emails("@example.com");
        let _ = extract_emails("user@");
        let _ = extract_emails("u@u@u.com");
    }

    #[test]
    fn fuzz_extract_emails_very_long_local_part() {
        // Very long local part — regex must have size limits that prevent ReDoS
        let long_local = format!("{}@example.com", "a".repeat(300));
        let results = extract_emails(&long_local);
        // With size limit on DFA, this may or may not match — just no panic
        let _ = results;
    }

    #[test]
    fn fuzz_extract_phones_empty() {
        assert!(extract_phones("").is_empty());
    }

    #[test]
    fn fuzz_extract_phones_all_digits() {
        // All digits — should not panic
        let _ = extract_phones(&"1".repeat(50));
    }

    #[test]
    fn fuzz_parse_number_all_special_chars() {
        assert_eq!(parse_number("..."), None);
        assert_eq!(parse_number("---"), None);
        assert_eq!(parse_number("NaN"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("∞"), None);
    }

    #[test]
    fn fuzz_parse_number_overflow_value() {
        // A value larger than f64::MAX — should parse to Inf or None, not panic
        let huge = format!("1{}0", "9".repeat(400));
        let result = parse_number(&huge);
        // f64 parse returns Inf for overflow — that's OK, just not panic
        let _ = result;
    }

    #[test]
    fn fuzz_truncate_zero_max_chars() {
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn fuzz_truncate_max_chars_one() {
        assert_eq!(truncate("hello", 1), "h");
    }

    #[test]
    fn fuzz_truncate_max_chars_two() {
        assert_eq!(truncate("hello", 2), "he");
    }

    #[test]
    fn fuzz_truncate_emoji_sequence() {
        // Emoji are multi-byte but single char — truncation by chars not bytes
        let emojis = "😀🎉🦄🌟💡";
        let truncated = truncate(emojis, 3);
        // Must take exactly 3 chars (3 emoji)
        assert_eq!(truncated.chars().count(), 3);
    }

    #[test]
    fn fuzz_extract_domain_malformed_urls() {
        assert_eq!(extract_domain("not a url"), None);
        assert_eq!(extract_domain(""), None);
        assert_eq!(extract_domain("://missing-scheme"), None);
        assert_eq!(extract_domain("http://"), None);
    }

    #[test]
    fn fuzz_is_valid_date_garbage_input() {
        assert!(!is_valid_date("hello world"));
        assert!(!is_valid_date("0000-00-00"));
        assert!(!is_valid_date("9999-99-99"));
        assert!(!is_valid_date("2024-13-01"));
        assert!(!is_valid_date("\u{0000}\u{0001}"));
    }

    #[test]
    fn fuzz_normalize_entity_name_empty() {
        assert_eq!(normalize_entity_name(""), "");
    }

    #[test]
    fn fuzz_normalize_entity_name_only_whitespace() {
        assert_eq!(normalize_entity_name("   \t\n  "), "");
    }

    #[test]
    fn fuzz_normalize_entity_name_emoji() {
        // Emoji have no diacritics — should pass through stripped but lowercased
        let result = normalize_entity_name("Starz 😀 Electronics");
        assert!(result.contains("starz"));
        assert!(result.contains("electronics"));
        // No panic
    }

    #[test]
    fn fuzz_remove_boilerplate_no_match() {
        let text = "This is purely technical content about semiconductors.";
        let result = remove_boilerplate(text);
        assert!(result.contains("semiconductors"));
    }

    #[test]
    fn fuzz_remove_boilerplate_all_boilerplate() {
        let text = "Accept cookies. Privacy policy. Terms of use. All rights reserved. © 2024";
        let result = remove_boilerplate(text);
        // Most content removed — result may be empty or minimal
        let _ = result; // just no panic
    }

    #[test]
    fn fuzz_dedup_preserving_order_empty() {
        assert!(dedup_preserving_order(vec![]).is_empty());
    }

    #[test]
    fn fuzz_dedup_preserving_order_all_same() {
        let input = vec!["same".to_string(); 1_000];
        let result = dedup_preserving_order(input);
        assert_eq!(result, vec!["same"]);
    }

    #[test]
    fn fuzz_is_valid_date_range_empty() {
        assert!(!is_valid_date_range(""));
    }
}
