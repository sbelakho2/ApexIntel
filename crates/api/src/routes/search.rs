//! Search route — request/response types and logic for full-text search.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for search endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    if trimmed.len() > 500 {
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

/// Simple search highlighting: wrap matched terms in `<mark>` tags.
pub fn highlight_snippet(text: &str, tokens: &[String], max_len: usize) -> String {
    let lower = text.to_lowercase();
    // Find the first matching token position
    let best_pos = tokens
        .iter()
        .filter_map(|t| lower.find(t.as_str()))
        .min()
        .unwrap_or(0);

    // Extract a window around the match
    let start = if best_pos > max_len / 4 {
        best_pos - max_len / 4
    } else {
        0
    };
    let end = (start + max_len).min(text.len());
    let mut snippet = text[start..end].to_string();

    // Add ellipsis
    if start > 0 {
        snippet = format!("...{}", snippet);
    }
    if end < text.len() {
        snippet = format!("{}...", snippet);
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
        assert!(snippet.len() <= 50); // ~40 + ellipsis
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
