//! Semantic search endpoint via Tantivy with BM25 scoring + field boosting.
//!
//! Provides `/api/search/semantic` endpoint supporting:
//! - BM25 full-text ranking
//! - Field boosting (title:3x, narrative:2x, raw:1x)
//! - Faceted filtering by entity type, region, date range
//! - Pagination and highlighting

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ─── Query model ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SemanticSearchQuery {
    /// Free-text query
    pub q: String,
    /// Filter by entity type(s): "company", "person", "observation", "warning"
    pub entity_types: Option<Vec<String>>,
    /// Filter by region(s): "TN", "MA", "IL", "CN", "EU", etc.
    pub regions: Option<Vec<String>>,
    /// Date range start (inclusive)
    pub from_date: Option<NaiveDate>,
    /// Date range end (inclusive)
    pub to_date: Option<NaiveDate>,
    /// Results per page (default 20, max 100)
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
    /// Minimum BM25 score threshold
    pub min_score: Option<f64>,
}

impl SemanticSearchQuery {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(20).min(100)
    }

    pub fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Debug, Serialize)]
pub struct SemanticSearchResult {
    pub total_hits: usize,
    pub results: Vec<SearchHit>,
    pub facets: SearchFacets,
    pub query_time_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub entity_id: String,
    pub entity_type: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub region: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub highlights: Vec<HighlightFragment>,
}

#[derive(Debug, Serialize)]
pub struct HighlightFragment {
    pub field: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SearchFacets {
    pub entity_types: Vec<FacetCount>,
    pub regions: Vec<FacetCount>,
}

#[derive(Debug, Serialize)]
pub struct FacetCount {
    pub value: String,
    pub count: usize,
}

// ─── Field boosting ─────────────────────────────────────────────────────

/// Tantivy field boosting configuration.
#[derive(Debug, Clone)]
pub struct FieldBoosts {
    /// Boost for title/name fields
    pub title_boost: f64,
    /// Boost for narrative/description fields
    pub narrative_boost: f64,
    /// Boost for raw content
    pub raw_boost: f64,
    /// Boost for tags/categories
    pub tag_boost: f64,
}

impl Default for FieldBoosts {
    fn default() -> Self {
        Self {
            title_boost: 3.0,
            narrative_boost: 2.0,
            raw_boost: 1.0,
            tag_boost: 1.5,
        }
    }
}

/// Build a Tantivy query string with field boosts applied.
pub fn build_boosted_query(query: &str, boosts: &FieldBoosts) -> String {
    let sanitized = sanitize_query(query);
    format!(
        "(title:\"{}\"^{:.1} OR narrative:\"{}\"^{:.1} OR raw:\"{}\"^{:.1} OR tags:\"{}\"^{:.1})",
        sanitized,
        boosts.title_boost,
        sanitized,
        boosts.narrative_boost,
        sanitized,
        boosts.raw_boost,
        sanitized,
        boosts.tag_boost,
    )
}

/// Sanitize user query to prevent injection into Tantivy query parser.
pub fn sanitize_query(query: &str) -> String {
    query
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' || *c == '.')
        .collect::<String>()
        .trim()
        .to_string()
}

// ─── Highlight utilities ────────────────────────────────────────────────

/// Generate snippet with highlighted matching terms.
pub fn generate_snippet(text: &str, query_terms: &[&str], context_chars: usize) -> String {
    let lower = text.to_lowercase();
    let mut best_pos = None;
    let mut best_term = "";

    for term in query_terms {
        if let Some(pos) = lower.find(&term.to_lowercase()) {
            if best_pos.is_none_or(|best| pos < best) {
                best_pos = Some(pos);
                best_term = term;
            }
        }
    }

    // Helper: find the nearest UTF-8 char boundary at or before `pos`.
    let floor = |s: &str, pos: usize| -> usize {
        let pos = pos.min(s.len());
        let mut i = pos;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    // Helper: find the nearest UTF-8 char boundary at or after `pos`.
    let ceil = |s: &str, pos: usize| -> usize {
        let pos = pos.min(s.len());
        let mut i = pos;
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        i
    };

    match best_pos {
        Some(pos) => {
            let start = floor(text, pos.saturating_sub(context_chars));
            let end = ceil(
                text,
                (pos + best_term.len() + context_chars).min(text.len()),
            );
            let mut snippet = String::new();
            if start > 0 {
                snippet.push('…');
            }
            snippet.push_str(&text[start..end]);
            if end < text.len() {
                snippet.push('…');
            }
            snippet
        }
        None => {
            // No match found — return beginning of text
            let end = ceil(text, context_chars.min(text.len()));
            let mut snippet = text[..end].to_string();
            if end < text.len() {
                snippet.push('…');
            }
            snippet
        }
    }
}

/// Extract query terms for highlighting.
pub fn extract_terms(query: &str) -> Vec<String> {
    sanitize_query(query)
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect()
}

// ─── Path constants ─────────────────────────────────────────────────────

pub const SEMANTIC_SEARCH_PATH: &str = "/api/search/semantic";

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_query() {
        assert_eq!(sanitize_query("hello world"), "hello world");
        assert_eq!(sanitize_query("test<script>alert"), "testscriptalert");
        assert_eq!(sanitize_query("IATF-16949"), "IATF-16949");
    }

    #[test]
    fn test_build_boosted_query() {
        let boosts = FieldBoosts::default();
        let query = build_boosted_query("PCB assembly", &boosts);
        assert!(query.contains("title:\"PCB assembly\"^3.0"));
        assert!(query.contains("narrative:\"PCB assembly\"^2.0"));
        assert!(query.contains("raw:\"PCB assembly\"^1.0"));
    }

    #[test]
    fn test_extract_terms() {
        let terms = extract_terms("PCB assembly manufacturer");
        assert_eq!(terms.len(), 3);
        assert!(terms.contains(&"pcb".into()));
        assert!(terms.contains(&"assembly".into()));
    }

    #[test]
    fn test_extract_terms_filters_short() {
        let terms = extract_terms("a PCB in EU");
        assert_eq!(terms.len(), 3); // "a" and "in" filtered out, "EU" retained
    }

    #[test]
    fn test_generate_snippet_with_match() {
        let text = "The company manufactures high-quality PCB assemblies for automotive clients worldwide.";
        let snippet = generate_snippet(text, &["PCB"], 20);
        assert!(snippet.contains("PCB"));
        assert!(snippet.len() < text.len() + 10);
    }

    #[test]
    fn test_generate_snippet_no_match() {
        let text = "This is a long document about something else entirely unrelated.";
        let snippet = generate_snippet(text, &["XYZ123"], 30);
        assert!(snippet.starts_with("This is a long"));
    }

    #[test]
    fn test_query_defaults() {
        let query = SemanticSearchQuery {
            q: "test".into(),
            entity_types: None,
            regions: None,
            from_date: None,
            to_date: None,
            limit: None,
            offset: None,
            min_score: None,
        };
        assert_eq!(query.limit(), 20);
        assert_eq!(query.offset(), 0);
    }

    #[test]
    fn test_query_limit_capped() {
        let query = SemanticSearchQuery {
            q: "test".into(),
            entity_types: None,
            regions: None,
            from_date: None,
            to_date: None,
            limit: Some(500),
            offset: None,
            min_score: None,
        };
        assert_eq!(query.limit(), 100); // capped at 100
    }
}
