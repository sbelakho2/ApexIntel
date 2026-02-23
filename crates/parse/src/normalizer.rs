use regex::Regex;

/// Normalize whitespace: collapse runs of whitespace into single spaces, trim.
pub fn normalize_whitespace(text: &str) -> String {
    let re = Regex::new(r"\s+").unwrap();
    re.replace_all(text.trim(), " ").to_string()
}

/// Strip HTML tags from text content.
pub fn strip_html_tags(html: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(html, "").to_string()
}

/// Normalize Unicode characters: convert common diacritical forms.
pub fn normalize_unicode(text: &str) -> String {
    // Basic normalization: trim, collapse whitespace
    // More advanced NFC/NFD could be added if needed
    normalize_whitespace(text)
}

/// Extract domain from a URL string.
pub fn extract_domain(url_str: &str) -> Option<String> {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

/// Truncate text to max length, adding ellipsis if truncated.
pub fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let boundary = text
            .char_indices()
            .take_while(|&(i, _)| i < max_len.saturating_sub(3))
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &text[..boundary])
    }
}

/// Remove common boilerplate patterns (cookie notices, nav elements, etc.).
pub fn remove_boilerplate(text: &str) -> String {
    let patterns = [
        r"(?i)accept\s+cookies?",
        r"(?i)cookie\s+policy",
        r"(?i)privacy\s+policy",
        r"(?i)terms\s+of\s+(use|service)",
        r"(?i)all\s+rights\s+reserved",
        r"(?i)©\s*\d{4}",
    ];

    let mut result = text.to_string();
    for pat in &patterns {
        let re = Regex::new(pat).unwrap();
        result = re.replace_all(&result, "").to_string();
    }
    normalize_whitespace(&result)
}

/// Extract all email addresses from text.
pub fn extract_emails(text: &str) -> Vec<String> {
    let re = Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
    re.find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Extract phone numbers (basic international format).
pub fn extract_phones(text: &str) -> Vec<String> {
    let re = Regex::new(r"\+?\d[\d\s\-().]{7,}\d").unwrap();
    re.find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
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
}
