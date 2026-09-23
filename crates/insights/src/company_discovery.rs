//! # Company Discovery Engine
//!
//! Zero-shot company extraction from arbitrary text sources using pattern
//! matching — no pre-defined company list required.
//!
//! # Strategy
//!
//! 1. **Suffix-based** — "[Capitalized] Corp/Inc/Ltd/GmbH/SARL/LLC/PLC/etc."
//! 2. **Comma-suffix** — "Company Name," followed by a known suffix
//! 3. **Multi-word** — 3+ consecutive capitalized words that look like a name
//! 4. **Ticker** — `$TICKER`, `NASDAQ:TICKER`, `NYSE:TICKER`, etc.
//! 5. **URL-derived** — Domain-derived names from web crawl sources
//!
//! Confidence is assigned based on pattern specificity:
//! - Ticker pattern → 0.9
//! - "Name Corp/Inc/Ltd" → 0.8
//! - 3+ capitalized words → 0.5
//! - Single capitalized word before known context → 0.3

use regex::Regex;
use std::cmp::Reverse;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Where a company mention was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiscoverySource {
    NewsArticle,
    JobPosting,
    PatentFiling,
    RegulatoryFiling,
    SocialMedia,
    TradeShow,
    FinancialReport,
    WebCrawl,
    AcademicPaper,
    Other(String),
}

impl DiscoverySource {
    /// Human-readable label.
    pub fn as_str(&self) -> &str {
        match self {
            Self::NewsArticle => "news_article",
            Self::JobPosting => "job_posting",
            Self::PatentFiling => "patent_filing",
            Self::RegulatoryFiling => "regulatory_filing",
            Self::SocialMedia => "social_media",
            Self::TradeShow => "trade_show",
            Self::FinancialReport => "financial_report",
            Self::WebCrawl => "web_crawl",
            Self::AcademicPaper => "academic_paper",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// A discovered company candidate before verification.
#[derive(Debug, Clone)]
pub struct CompanyCandidate {
    /// The company name as mentioned in source.
    pub raw_name: String,
    /// Normalized name (lowercase, stripped of legal suffixes).
    pub normalized_name: String,
    /// Where this was found.
    pub source: DiscoverySource,
    /// How confident we are this is a real company (0.0 – 1.0).
    pub extraction_confidence: f64,
    /// Context surrounding the mention (for verification).
    pub context_snippet: String,
    /// Additional metadata extracted from context.
    pub metadata: HashMap<String, String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Compiled patterns (lazy-static for performance)
// ─────────────────────────────────────────────────────────────────────────────

static COMPANY_SUFFIXES: &[&str] = &[
    "Corporation",
    "Incorporated",
    "Limited",
    "Corp.",
    "Corp",
    "Inc.",
    "Inc",
    "Ltd.",
    "Ltd",
    "LLC.",
    "LLC",
    "PLC.",
    "PLC",
    "GmbH",
    "SARL",
    "S.A.R.L.",
    "S.A.",
    "N.V.",
    "Co.",
    "Co",
    "AG",
    "KG",
    "Company",
];

fn suffix_pattern() -> Regex {
    // Sort suffixes by length descending so longer matches (e.g., "Inc.") win over shorter ("Inc")
    let mut sorted: Vec<&str> = COMPANY_SUFFIXES.to_vec();
    sorted.sort_by_key(|b| Reverse(b.len()));
    let suffixes = sorted
        .iter()
        .map(|s| regex::escape(s))
        .collect::<Vec<_>>()
        .join("|");
    // Use non-greedy inner group so the suffix isn't consumed as part of the name.
    // E.g., "NVIDIA Corporation" → name="NVIDIA", suffix="Corporation"
    // Use (?:[\s,;!?)]|$) instead of \b to handle suffixes ending in punctuation (e.g., "Inc.")
    // NOTE: (?i) is applied ONLY to the suffix alternation (not globally), so [A-Z] in the
    // name part remains case-sensitive — preventing false matches like "announced new GPUs. Apple"
    // NOTE: No `.` in the name character class to prevent cross-sentence matching (e.g., "GPUs. Apple Inc.")
    Regex::new(&format!(
        r"\b((?:[A-Z][a-zA-Z&\-]+(?:\s+[A-Z][a-zA-Z&\-]+)*?))\s+((?i:{}))(?:[\s,;!?)]|$)",
        suffixes
    ))
    .unwrap_or_else(|e| panic!("valid suffix regex: {e}"))
}

fn ticker_pattern() -> Regex {
    Regex::new(r"(?i)(?:NASDAQ|NYSE|AMS|LSE|TSE|HKEX|ASX|TSX|BSE|NSE|EURONEXT|SHG|SHE|OTCQX|OTCQB|OTCPK):([A-Z0-9]{1,6}(?:-[A-Z0-9]{1,6})?)|\$([A-Z0-9]{1,6}(?:-[A-Z0-9]{1,6})?)")
        .unwrap_or_else(|e| panic!("valid ticker regex: {e}"))
}

fn url_pattern() -> Regex {
    Regex::new(r"(?i)(?:https?://)?(?:www\.)?([a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.(?:com|org|net|io|ai|co\.uk|de|fr|jp|cn|sg|tw)(?:/[^\s]*)?)")
        .unwrap_or_else(|e| panic!("valid url regex: {e}"))
}

fn capitalized_words_pattern() -> Regex {
    Regex::new(r"\b([A-Z][a-zA-Z]+(?: [A-Z][a-zA-Z]+){2,})\b")
        .unwrap_or_else(|e| panic!("valid capitalized words regex: {e}"))
}

fn comma_suffix_pattern() -> Regex {
    let mut sorted: Vec<&str> = COMPANY_SUFFIXES.to_vec();
    sorted.sort_by_key(|b| Reverse(b.len()));
    let suffixes = sorted
        .iter()
        .map(|s| regex::escape(s))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"\b([A-Z][a-zA-Z&\-]+(?: [A-Z][a-zA-Z&\-]+)*),\s*((?i:{}))(?:[\s,;!?)]|$)",
        suffixes
    ))
    .unwrap_or_else(|e| panic!("valid comma-suffix regex: {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Core extraction functions
// ─────────────────────────────────────────────────────────────────────────────

/// Extract company mentions from arbitrary text without relying on a known list.
///
/// Uses multiple pattern-matching strategies and assigns confidence scores
/// based on pattern specificity.
pub fn extract_company_mentions(text: &str, source: DiscoverySource) -> Vec<CompanyCandidate> {
    let mut candidates: Vec<CompanyCandidate> = Vec::new();
    let mut seen_normalized: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Strategy 1: Ticker patterns (highest confidence)
    extract_ticker_pattern(text)
        .into_iter()
        .for_each(|(exchange, ticker)| {
            let raw_name = format!("{exchange}:{ticker}");
            let normalized = normalize_company_name(&raw_name);
            if seen_normalized.insert(normalized.clone()) {
                let snippet = extract_surrounding_context(text, &raw_name, 80);
                let mut metadata = HashMap::new();
                metadata.insert("ticker".to_string(), ticker.clone());
                metadata.insert("exchange".to_string(), exchange);
                candidates.push(CompanyCandidate {
                    raw_name: raw_name.clone(),
                    normalized_name: normalized,
                    source: source.clone(),
                    extraction_confidence: 0.9,
                    context_snippet: snippet,
                    metadata,
                });
            }
        });

    // Strategy 2: "Name Corp/Inc/Ltd" pattern (high confidence)
    let suffix_re = suffix_pattern();
    for cap in suffix_re.captures_iter(text) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let suffix = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        if name.len() > 2 {
            let raw_name = format!("{} {}", name, suffix);
            let normalized = normalize_company_name(&raw_name);
            if seen_normalized.insert(normalized.clone()) {
                let snippet = extract_surrounding_context(text, &raw_name, 80);
                let mut metadata = extract_metadata(text, &raw_name);
                let name_metadata = extract_ticker_from_context(text, name);
                metadata.extend(name_metadata);
                candidates.push(CompanyCandidate {
                    raw_name: raw_name.clone(),
                    normalized_name: normalized,
                    source: source.clone(),
                    extraction_confidence: 0.8,
                    context_snippet: snippet,
                    metadata,
                });
            }
        }
    }

    // Strategy 3: "Name," followed by suffix (high confidence)
    let comma_re = comma_suffix_pattern();
    for cap in comma_re.captures_iter(text) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        if name.len() > 2 {
            let normalized = normalize_company_name(name);
            if seen_normalized.insert(normalized.clone()) {
                let snippet = extract_surrounding_context(text, name, 80);
                let metadata = extract_metadata(text, name);
                candidates.push(CompanyCandidate {
                    raw_name: name.to_string(),
                    normalized_name: normalized,
                    source: source.clone(),
                    extraction_confidence: 0.8,
                    context_snippet: snippet,
                    metadata,
                });
            }
        }
    }

    // Strategy 4: 3+ consecutive capitalized words (medium confidence)
    let cap_words_re = capitalized_words_pattern();
    for cap in cap_words_re.captures_iter(text) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        if name.len() > 5 && !has_stopwords(name) {
            let normalized = normalize_company_name(name);
            if seen_normalized.insert(normalized.clone()) {
                let snippet = extract_surrounding_context(text, name, 80);
                let metadata = extract_metadata(text, name);
                candidates.push(CompanyCandidate {
                    raw_name: name.to_string(),
                    normalized_name: normalized,
                    source: source.clone(),
                    extraction_confidence: 0.5,
                    context_snippet: snippet,
                    metadata,
                });
            }
        }
    }

    // Strategy 5: URL-derived names (for web crawl sources)
    if matches!(source, DiscoverySource::WebCrawl) {
        let url_re = url_pattern();
        for cap in url_re.captures_iter(text) {
            if let Some(domain) = cap.get(1) {
                let domain_str = domain.as_str();
                // Extract company-like name from domain
                let name_parts: Vec<&str> = domain_str.split('.').collect();
                if let Some(base) = name_parts.first() {
                    let company_name = domain_name_to_company_name(base);
                    let normalized = normalize_company_name(&company_name);
                    if seen_normalized.insert(normalized.clone()) && company_name.len() > 3 {
                        let snippet = extract_surrounding_context(text, domain_str, 80);
                        let mut metadata = HashMap::new();
                        metadata.insert("website".to_string(), format!("https://{}", domain_str));
                        candidates.push(CompanyCandidate {
                            raw_name: company_name,
                            normalized_name: normalized,
                            source: source.clone(),
                            extraction_confidence: 0.3,
                            context_snippet: snippet,
                            metadata,
                        });
                    }
                }
            }
        }
    }

    candidates
}

/// Extract ticker patterns from text, returning `(exchange, ticker)` pairs.
pub fn extract_ticker_pattern(text: &str) -> Vec<(String, String)> {
    let ticker_re = ticker_pattern();
    let mut results = Vec::new();

    for cap in ticker_re.captures_iter(text) {
        // Pattern 1: "EXCHANGE:TICKER"
        if let Some(ticker) = cap.get(1) {
            let exchange = if cap.get(0).map_or("", |m| m.as_str()).starts_with("NASDAQ:") {
                "NASDAQ"
            } else if cap.get(0).map_or("", |m| m.as_str()).starts_with("NYSE:") {
                "NYSE"
            } else {
                // Determine from the full match prefix
                let full = cap.get(0).map(|m| m.as_str()).unwrap_or("");
                let colon_pos = full.find(':').unwrap_or(0);
                &full[..colon_pos]
            };
            results.push((exchange.to_string(), ticker.as_str().to_string()));
        }
        // Pattern 2: "$TICKER"
        if let Some(ticker) = cap.get(2) {
            results.push(("OTC".to_string(), ticker.as_str().to_string()));
        }
    }

    results
}

/// Normalize a company name: lowercase, strip legal suffixes, remove punctuation,
/// collapse whitespace.
pub fn normalize_company_name(name: &str) -> String {
    let name = name.trim();
    let mut result = name.to_lowercase();

    // Strip common legal suffixes
    let suffixes = [
        " inc.",
        " inc",
        " incorporated",
        " corp.",
        " corp",
        " corporation",
        " ltd.",
        " ltd",
        " limited",
        " llc",
        " plc",
        " plc.",
        " gmbh",
        " sarl",
        " s.a.r.l.",
        " s.a.",
        " n.v.",
        " ag",
        " co.",
        " co",
        " kg",
        " pty ltd",
        " pty. ltd.",
    ];
    for suffix in &suffixes {
        if result.ends_with(suffix) {
            let trimmed_len = result.len().saturating_sub(suffix.len());
            result = result[..trimmed_len].trim_end().to_string();
            break;
        }
    }

    // Remove punctuation (keep letters, digits, spaces, hyphens, ampersands)
    result = result
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '&')
        .collect();

    // Collapse whitespace
    let words: Vec<&str> = result.split_whitespace().collect();
    words.join(" ")
}

/// Extract metadata from surrounding context of a candidate mention.
pub fn extract_metadata(text: &str, _candidate_name: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();

    // Phone numbers
    let phone_re = Regex::new(r"\+?\d{1,3}[-.\s]?\(?\d{1,4}\)?[-.\s]?\d{1,4}[-.\s]?\d{1,9}")
        .unwrap_or_else(|e| panic!("valid phone regex: {e}"));
    if let Some(phone) = phone_re.find(text) {
        metadata.insert("phone".to_string(), phone.as_str().to_string());
    }

    // Website URLs
    let url_re =
        Regex::new(r"https?://[^\s,;)]+").unwrap_or_else(|e| panic!("valid url regex: {e}"));
    if let Some(url) = url_re.find(text) {
        metadata.insert("website".to_string(), url.as_str().to_string());
    }

    // Email domains
    let email_re = Regex::new(r"[a-zA-Z0-9._%+-]+@([a-zA-Z0-9.-]+\.[a-zA-Z]{2,})")
        .unwrap_or_else(|e| panic!("valid email regex: {e}"));
    for cap in email_re.captures_iter(text) {
        if let Some(domain) = cap.get(1) {
            metadata.insert("email_domain".to_string(), domain.as_str().to_string());
            break; // Take the first one
        }
    }

    // Revenue figures
    let revenue_re = Regex::new(r"(?i)(?:revenue|turnover|sales|earnings|income)\s*(?:of|:)?\s*\$?([\d,.]+)\s*(?:billion|million|trillion|B|M|T|bn|mn)?")
        .unwrap_or_else(|e| panic!("valid revenue regex: {e}"));
    if let Some(rev) = revenue_re.find(text) {
        metadata.insert("revenue_mentioned".to_string(), rev.as_str().to_string());
    }

    // Employee counts
    let emp_re = Regex::new(r"(?i)(\d[\d,]*)\s*(?:employees|staff|workers|headcount|people)")
        .unwrap_or_else(|e| panic!("valid employee regex: {e}"));
    if let Some(emp) = emp_re.find(text) {
        metadata.insert("employees_mentioned".to_string(), emp.as_str().to_string());
    }

    // Locations (simple pattern: capitalized place name after "in", "at", "based in")
    let loc_re = Regex::new(r"(?i)(?:based in|headquartered in|located in|operates in)\s+([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*)")
        .unwrap_or_else(|e| panic!("valid location regex: {e}"));
    if let Some(loc) = loc_re.find(text) {
        metadata.insert(
            "headquarters_mentioned".to_string(),
            loc.as_str().to_string(),
        );
    }

    metadata
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract surrounding context around a mention.
fn extract_surrounding_context(text: &str, mention: &str, radius: usize) -> String {
    if let Some(pos) = text.find(mention) {
        let start = pos.saturating_sub(radius);
        let end = (pos + mention.len() + radius).min(text.len());
        let ctx = &text[start..end];
        // Add ellipsis if truncated
        let prefix = if start > 0 { "…" } else { "" };
        let suffix = if end < text.len() { "…" } else { "" };
        format!("{}{}{}", prefix, ctx, suffix)
    } else {
        text.chars().take(160).collect()
    }
}

/// Attempt to extract ticker from nearby context.
fn extract_ticker_from_context(text: &str, name: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    if let Some(pos) = text.find(name) {
        let start = pos.saturating_sub(100);
        let end = (pos + name.len() + 100).min(text.len());
        let context = &text[start..end];

        // Look for ticker patterns near the name
        let ticker_re = ticker_pattern();
        if let Some(cap) = ticker_re.captures(context) {
            if let Some(ticker) = cap.get(1).or_else(|| cap.get(2)) {
                metadata.insert("ticker".to_string(), ticker.as_str().to_string());
            }
        }
    }
    metadata
}

/// Convert a domain name (e.g., "apple", "microsoft") to a company-like name.
fn domain_name_to_company_name(domain: &str) -> String {
    // Split on hyphens and special chars
    let parts: Vec<&str> = domain.split(|c: char| !c.is_alphanumeric()).collect();
    let words: Vec<String> = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut s = p.to_string();
            if s.len() > 1 {
                let first = s.remove(0).to_uppercase().to_string();
                format!("{}{}", first, s)
            } else {
                s.to_uppercase()
            }
        })
        .collect();
    words.join(" ")
}

/// Common words that are unlikely to be company names.
fn has_stopwords(name: &str) -> bool {
    let stopwords = [
        "the", "this", "that", "these", "those", "what", "which", "where", "when", "why", "how",
        "who", "whom", "with", "without", "about", "between", "through", "during", "before",
        "after", "above", "below", "from", "they", "them", "their", "there", "here", "where",
        "which", "would", "could", "should", "have", "has", "had", "been", "being", "some", "any",
        "each", "every", "both", "few", "more", "most", "other", "into", "over", "such", "only",
        "own", "same", "than", "very", "just", "also", "can", "will", "may",
    ];
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // If all words are stopwords, likely not a company name
    if words.is_empty() {
        return true;
    }
    words.iter().all(|w| stopwords.contains(w))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Normalization ─────────────────────────────────────────────────────

    #[test]
    fn test_normalize_strips_suffix() {
        assert_eq!(normalize_company_name("NVIDIA Corporation"), "nvidia");
        assert_eq!(normalize_company_name("Apple Inc."), "apple");
        assert_eq!(normalize_company_name("Microsoft Corp"), "microsoft");
        assert_eq!(normalize_company_name("Tesla, Inc."), "tesla");
        assert_eq!(normalize_company_name("Siemens AG"), "siemens");
    }

    #[test]
    fn test_normalize_lowercases_and_collapses() {
        assert_eq!(
            normalize_company_name("  Advanced Micro Devices  "),
            "advanced micro devices"
        );
    }

    #[test]
    fn test_normalize_handles_gmbh() {
        assert_eq!(normalize_company_name("Rheinmetall AG"), "rheinmetall");
        assert_eq!(normalize_company_name("Bosch GmbH"), "bosch");
    }

    // ── Ticker extraction ─────────────────────────────────────────────────

    #[test]
    fn test_extract_ticker_nasdaq() {
        let result = extract_ticker_pattern("NASDAQ:AAPL reported earnings");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "NASDAQ");
        assert_eq!(result[0].1, "AAPL");
    }

    #[test]
    fn test_extract_ticker_nyse() {
        let result = extract_ticker_pattern("NYSE:TSM is a semiconductor giant");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "NYSE");
        assert_eq!(result[0].1, "TSM");
    }

    #[test]
    fn test_extract_ticker_dollar() {
        let result = extract_ticker_pattern("$MSFT jumped 5% today");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "MSFT");
    }

    #[test]
    fn test_extract_ticker_multiple() {
        let result = extract_ticker_pattern("Portfolio: $AAPL, $GOOGL, $MSFT");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_extract_ticker_no_match() {
        let result = extract_ticker_pattern("No tickers here");
        assert!(result.is_empty());
    }

    // ── Extract company mentions ──────────────────────────────────────────

    #[test]
    fn test_extract_suffix_pattern() {
        let text = "NVIDIA Corporation announced new GPUs. Apple Inc. is hiring.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        let names: Vec<&str> = candidates.iter().map(|c| c.raw_name.as_str()).collect();
        assert!(
            names.contains(&"NVIDIA Corporation"),
            "Should find NVIDIA Corporation"
        );
        assert!(names.contains(&"Apple Inc."), "Should find Apple Inc.");
    }

    #[test]
    fn test_extract_ticker_candidates() {
        let text = "NASDAQ:AAPL and NYSE:TSM are both great investments.";
        let candidates = extract_company_mentions(text, DiscoverySource::FinancialReport);
        let names: Vec<&str> = candidates.iter().map(|c| c.raw_name.as_str()).collect();
        assert!(names.contains(&"NASDAQ:AAPL"));
        assert!(names.contains(&"NYSE:TSM"));
        // Ticker candidates should have confidence 0.9
        for c in &candidates {
            assert!((c.extraction_confidence - 0.9).abs() < 0.01);
        }
    }

    #[test]
    fn test_extract_capitalized_words() {
        let text = "Advanced Micro Devices is a great semiconductor company.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        let names: Vec<&str> = candidates.iter().map(|c| c.raw_name.as_str()).collect();
        // "Advanced Micro Devices" is 3 capitalized words
        assert!(
            names.contains(&"Advanced Micro Devices")
                || names.iter().any(|n| n.contains("Advanced Micro Devices"))
        );
    }

    #[test]
    fn test_extract_with_metadata() {
        let text = "NVIDIA Corporation (NASDAQ:NVDA) has 26,000 employees worldwide.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        // Some candidate should have employee metadata
        let has_emp = candidates
            .iter()
            .any(|c| c.metadata.contains_key("employees_mentioned"));
        assert!(has_emp, "Should extract employee count metadata");
    }

    #[test]
    fn test_extract_empty_text() {
        let candidates = extract_company_mentions("", DiscoverySource::NewsArticle);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_extract_no_companies() {
        let text = "The weather today is sunny and warm.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        // Possibly capitalized word patterns might not fire if only single words
        // But "The weather today is sunny and warm" is mostly lowercase except "The"
        // This should return no candidates
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_extract_comma_suffix() {
        let text = "Apple, Inc. reported record earnings.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        // Should match via either suffix pattern or comma-suffix pattern
        assert!(!candidates.is_empty());
    }

    #[test]
    fn test_extract_gmbh() {
        let text = "Robert Bosch GmbH announced a new acquisition.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        let names: Vec<&str> = candidates.iter().map(|c| c.raw_name.as_str()).collect();
        assert!(names.contains(&"Robert Bosch GmbH"), "Should find Bosch");
    }

    #[test]
    fn test_extract_tse_ticker() {
        let text = "TSE:7203 is Toyota's ticker on the Tokyo exchange.";
        let candidates = extract_company_mentions(text, DiscoverySource::NewsArticle);
        let has_ticker = candidates.iter().any(|c| c.raw_name.contains("TSE:7203"));
        assert!(has_ticker, "Should extract TSE ticker");
    }

    #[test]
    fn test_domain_name_conversion() {
        let name = domain_name_to_company_name("apple");
        assert_eq!(name, "Apple");
    }

    #[test]
    fn test_domain_name_with_hyphens() {
        let name = domain_name_to_company_name("my-company");
        assert_eq!(name, "My Company");
    }

    #[test]
    fn test_extract_context_snippet() {
        let text = "This is a very long text about NVIDIA Corporation which is mentioned here.";
        let snippet = extract_surrounding_context(text, "NVIDIA Corporation", 20);
        assert!(snippet.contains("NVIDIA Corporation"));
        assert!(snippet.len() <= text.len() + 2); // +2 for ellipsis chars
    }

    #[test]
    fn test_has_stopwords_detects_common_words() {
        assert!(has_stopwords("the this that"));
        assert!(!has_stopwords("NVIDIA Corporation"));
        assert!(has_stopwords("have has had")); // All stopwords
    }

    #[test]
    fn test_discovery_source_as_str() {
        assert_eq!(DiscoverySource::NewsArticle.as_str(), "news_article");
        assert_eq!(DiscoverySource::WebCrawl.as_str(), "web_crawl");
        assert_eq!(DiscoverySource::Other("custom".into()).as_str(), "custom");
    }

    #[test]
    fn test_normalize_handles_plc() {
        assert_eq!(normalize_company_name("BAE Systems PLC"), "bae systems");
        assert_eq!(
            normalize_company_name("Rolls-Royce Holdings plc"),
            "rolls-royce holdings"
        );
    }
}
