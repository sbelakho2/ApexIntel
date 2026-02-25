//! Search route — request/response types and logic for full-text search.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for search endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchQuery {
    pub q: String,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub entity_types: Option<String>,
    pub regions: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Search result envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchHit>,
    pub total_hits: u64,
    pub facets: SearchFacets,
}

/// A single search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: String,
    pub entity_type: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub region: Option<String>,
    pub url: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Search facets for filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFacets {
    pub by_type: Vec<FacetCount>,
    pub by_region: Vec<FacetCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetCount {
    pub value: String,
    pub count: u64,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Validate search query (non-empty, reasonable length).
pub fn validate_search_query(q: &str) -> Result<String, String> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        return Err("Search query cannot be empty".to_string());
    }
    if trimmed.chars().count() > 500 {
        return Err("Search query too long (max 500 chars)".to_string());
    }
    Ok(trimmed.to_string())
}

/// Extract search tokens (simple whitespace tokenizer + lowercase).
pub fn tokenize_query(q: &str) -> Vec<String> {
    q.split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() >= 2) // skip single-char tokens
        .collect()
}

/// HTML-escape text to prevent XSS when rendering in innerHTML contexts.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Simple search highlighting: wrap matched terms in `<mark>` tags.
///
/// Uses char-based indexing to avoid panics on multi-byte UTF-8 text.
/// HTML-escapes the snippet before inserting `<mark>` tags to prevent XSS.
pub fn highlight_snippet(text: &str, tokens: &[String], max_len: usize) -> String {
    // Work in chars to avoid slicing mid-codepoint
    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();

    // Build a lowercase version from the *original* chars to preserve index alignment.
    // (to_lowercase() can change char count for Turkish İ etc — we lowercase per-char instead.)
    let lower_chars: Vec<Vec<char>> = chars.iter().map(|c| c.to_lowercase().collect()).collect();

    // Find first match position (in char units) by scanning char-by-char
    let best_char_pos = tokens
        .iter()
        .filter_map(|t| {
            let needle: Vec<char> = t.chars().collect();
            if needle.is_empty() {
                return None;
            }
            // Linear scan: compare lowercased chars at each position
            'outer: for start in 0..total_chars {
                let mut ni = 0;
                let mut ci = start;
                while ni < needle.len() && ci < total_chars {
                    let lc = &lower_chars[ci];
                    // Match each char of the lowered original against needle
                    for &ch in lc {
                        if ni < needle.len() && ch == needle[ni] {
                            ni += 1;
                        } else if ni < needle.len() {
                            continue 'outer;
                        }
                    }
                    ci += 1;
                }
                if ni == needle.len() {
                    return Some(start);
                }
            }
            None
        })
        .min()
        .unwrap_or(0);

    let start = if best_char_pos > max_len / 4 {
        best_char_pos - max_len / 4
    } else {
        0
    };
    let end = (start + max_len).min(total_chars);
    // HTML-escape the raw text before inserting <mark> tags to prevent XSS.
    let mut snippet: String = html_escape(&chars[start..end].iter().collect::<String>());

    if start > 0 {
        snippet = format!("...{}", snippet);
    }
    if end < total_chars {
        snippet = format!("{}...", snippet);
    }

    // Collect all non-overlapping match ranges across all tokens, then insert
    // <mark> tags in a single back-to-front pass so earlier insertions never
    // shift later indices and tokens cannot match inside already-inserted tags.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for token in tokens {
        if token.is_empty() {
            continue;
        }
        let escaped = regex::escape(token);
        if let Ok(re) = Regex::new(&format!("(?i){}", escaped)) {
            for m in re.find_iter(&snippet) {
                let candidate = (m.start(), m.end());
                // Skip if it overlaps any already-collected range
                let overlaps = ranges.iter().any(|&(s, e)| candidate.0 < e && candidate.1 > s);
                if !overlaps {
                    ranges.push(candidate);
                }
            }
        }
    }

    // Sort by start position descending so back-to-front insertion preserves indices
    ranges.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, e) in ranges {
        snippet.insert_str(e, "</mark>");
        snippet.insert_str(s, "<mark>");
    }

    snippet
}

/// Build facets from search results.
pub fn build_facets(results: &[SearchHit]) -> SearchFacets {
    let mut by_type: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut by_region: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for hit in results {
        *by_type.entry(hit.entity_type.clone()).or_insert(0) += 1;
        if let Some(ref region) = hit.region {
            *by_region.entry(region.clone()).or_insert(0) += 1;
        }
    }

    let mut type_facets: Vec<FacetCount> = by_type
        .into_iter()
        .map(|(value, count)| FacetCount { value, count })
        .collect();
    type_facets.sort_by(|a, b| b.count.cmp(&a.count));

    let mut region_facets: Vec<FacetCount> = by_region
        .into_iter()
        .map(|(value, count)| FacetCount { value, count })
        .collect();
    region_facets.sort_by(|a, b| b.count.cmp(&a.count));

    SearchFacets {
        by_type: type_facets,
        by_region: region_facets,
    }
}

/// Score sorting: sort by relevance score descending.
pub fn sort_by_score(results: &mut [SearchHit]) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(entity_type: &str, region: Option<&str>, score: f64) -> SearchHit {
        SearchHit {
            id: uuid::Uuid::new_v4().to_string(),
            entity_type: entity_type.to_string(),
            title: "Test hit".to_string(),
            snippet: "Some content...".to_string(),
            score,
            region: region.map(|s| s.to_string()),
            url: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_validate_search_query_ok() {
        assert!(validate_search_query("pcb assembly").is_ok());
    }

    #[test]
    fn test_validate_search_query_empty() {
        assert!(validate_search_query("").is_err());
        assert!(validate_search_query("   ").is_err());
    }

    #[test]
    fn test_validate_search_query_too_long() {
        assert!(validate_search_query(&"x".repeat(501)).is_err());
    }

    #[test]
    fn test_tokenize_query() {
        let tokens = tokenize_query("PCB Assembly manufacturing");
        assert_eq!(tokens, vec!["pcb", "assembly", "manufacturing"]);
    }

    #[test]
    fn test_tokenize_query_filters_short() {
        let tokens = tokenize_query("a PCB b");
        assert_eq!(tokens, vec!["pcb"]); // "a" and "b" filtered out
    }

    #[test]
    fn test_highlight_snippet() {
        let text = "This is a long text about PCB assembly in Tunisia region with many details";
        let tokens = vec!["pcb".to_string()];
        let snippet = highlight_snippet(text, &tokens, 40);
        // Snippet is ~40 chars of text + possible ellipsis + <mark>/<mark> tags (~13 per match)
        assert!(snippet.contains("<mark>"));
        assert!(snippet.len() <= 80);
    }

    #[test]
    fn test_highlight_snippet_no_match() {
        let text = "Some generic text";
        let tokens = vec!["xyz".to_string()];
        let snippet = highlight_snippet(text, &tokens, 40);
        assert!(!snippet.is_empty());
    }

    #[test]
    fn test_build_facets() {
        let results = vec![
            make_hit("company", Some("TN"), 0.9),
            make_hit("company", Some("MA"), 0.8),
            make_hit("person", Some("TN"), 0.7),
            make_hit("product", None, 0.6),
        ];
        let facets = build_facets(&results);
        assert_eq!(facets.by_type.len(), 3);
        assert_eq!(facets.by_type[0].value, "company"); // 2 hits
        assert_eq!(facets.by_type[0].count, 2);
        assert_eq!(facets.by_region.len(), 2);
    }

    #[test]
    fn test_build_facets_empty() {
        let facets = build_facets(&[]);
        assert!(facets.by_type.is_empty());
        assert!(facets.by_region.is_empty());
    }

    #[test]
    fn test_sort_by_score() {
        let mut results = vec![
            make_hit("company", Some("TN"), 0.5),
            make_hit("person", Some("MA"), 0.9),
            make_hit("product", None, 0.7),
        ];
        sort_by_score(&mut results);
        assert!(results[0].score > results[1].score);
        assert!(results[1].score > results[2].score);
    }

    #[test]
    fn test_search_response_serialization() {
        let resp = SearchResponse {
            query: "pcb assembly".to_string(),
            results: vec![make_hit("company", Some("TN"), 0.9)],
            total_hits: 1,
            facets: build_facets(&[make_hit("company", Some("TN"), 0.9)]),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("pcb assembly"));
        assert!(json.contains("\"total_hits\":1"));
    }
}
