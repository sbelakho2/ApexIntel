//! Search execution runtime: bounded, non-blocking Tantivy access, query
//! enhancement, entity-aware ranking and snippet highlighting.
//!
//! ## Why this exists
//!
//! Tantivy search is synchronous CPU work (a `Count` collector visits every
//! matching document, `TopDocs` scores them). Calling it directly inside axum
//! handlers blocked tokio worker threads: one `GET /search` ran up to six
//! index passes (main search + total probe + four facet probes) on the async
//! runtime, so a burst of searches starved every other request and the whole
//! site slowed down (incident 2026-10-06). Every index access now goes
//! through [`SearchRuntime`], which
//!
//! - runs index work on `spawn_blocking` (never blocks the async runtime),
//! - bounds concurrency with a semaphore (bursts queue instead of thrashing),
//! - caches count-only probes briefly (facets, totals) so typing and paging
//!   do not re-run identical counts,
//! - ranks entity types above raw documents, and
//! - produces HTML-safe highlighted snippets.

use apex_store::tantivy_index::{SearchIndex, SearchResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Maximum number of index passes executing at once. Searches above this
/// queue on the semaphore; the async runtime stays free for all other work.
pub const MAX_CONCURRENT_SEARCHES: usize = 6;

/// Count probes (facets, totals) are cached for this long. Short enough that
/// fresh documents appear quickly, long enough that keystroke-by-keystroke
/// search does not re-run identical counts.
pub const COUNT_CACHE_TTL: Duration = Duration::from_secs(20);
const COUNT_CACHE_MAX_ENTRIES: usize = 1024;

/// Entity-aware ranking boosts applied to mixed search results: a company or
/// person hit should outrank a raw observation document matching the same
/// text. The match set is unchanged — boosts never add documents.
pub fn entity_boosts() -> Vec<(String, f64)> {
    vec![
        ("company".to_string(), 4.0),
        ("person".to_string(), 3.5),
        ("insight".to_string(), 2.5),
        ("warning".to_string(), 2.0),
        ("report".to_string(), 1.5),
    ]
}

/// The facet entity types the search UI reports counts for.
pub const FACET_ENTITY_TYPES: [&str; 4] = ["company", "person", "warning", "insight"];

static RUNTIME: OnceLock<SearchRuntime> = OnceLock::new();

/// Process-wide search runtime.
pub fn search_runtime() -> &'static SearchRuntime {
    RUNTIME.get_or_init(SearchRuntime::new)
}

/// Bounded, cached search executor (see module docs).
pub struct SearchRuntime {
    semaphore: Arc<Semaphore>,
    counts: Mutex<HashMap<(String, String), (Instant, u64)>>,
}

impl SearchRuntime {
    pub fn new() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SEARCHES)),
            counts: Mutex::new(HashMap::new()),
        }
    }

    /// Run one synchronous index call under the concurrency bound, off the
    /// async runtime.
    async fn run_blocking<T, F>(&self, work: F) -> Result<T, anyhow::Error>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, anyhow::Error> + Send + 'static,
    {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("search runtime is shut down"))?;
        let joined = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|error| anyhow::anyhow!("search task failed: {error}"))?;
        joined
    }

    /// Ranked full-text search with entity boosts (see [`entity_boosts`]).
    pub async fn search_ranked(
        &self,
        index: Arc<SearchIndex>,
        query: String,
        boosts: Vec<(String, f64)>,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchResult>, u64), anyhow::Error> {
        self.run_blocking(move || index.search_ranked(&query, &boosts, limit, offset))
            .await
    }

    /// Unboosted full-text search (kept for callers that do not want entity
    /// ranking, e.g. diagnostics).
    pub async fn search_with_total(
        &self,
        index: Arc<SearchIndex>,
        query: String,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchResult>, u64), anyhow::Error> {
        self.run_blocking(move || index.search_with_total(&query, limit, offset))
            .await
    }

    /// Ranked search constrained to one entity type (facet filter).
    pub async fn search_entity_type_ranked(
        &self,
        index: Arc<SearchIndex>,
        query: String,
        entity_type: String,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchResult>, u64), anyhow::Error> {
        self.run_blocking(move || {
            index.search_entity_type_with_total(&query, &entity_type, limit, offset)
        })
        .await
    }

    /// Cached count for one (query, entity type) pair. Used for facet counts
    /// so repeated keystrokes/pages reuse the same count.
    pub async fn cached_count_for_type(
        &self,
        index: Arc<SearchIndex>,
        query: String,
        entity_type: String,
    ) -> Result<u64, anyhow::Error> {
        let key = (query.clone(), entity_type.clone());
        if let Some(value) = self.cached_count(&key) {
            return Ok(value);
        }
        let count = self
            .run_blocking(move || index.count_entity_type(&query, &entity_type))
            .await?;
        self.store_count(key, count);
        Ok(count)
    }

    fn cached_count(&self, key: &(String, String)) -> Option<u64> {
        let counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts
            .get(key)
            .filter(|(stamp, _)| stamp.elapsed() < COUNT_CACHE_TTL)
            .map(|(_, value)| *value)
    }

    fn store_count(&self, key: (String, String), value: u64) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if counts.len() >= COUNT_CACHE_MAX_ENTRIES {
            let now = Instant::now();
            counts.retain(|_, (stamp, _)| now.duration_since(*stamp) < COUNT_CACHE_TTL);
            if counts.len() >= COUNT_CACHE_MAX_ENTRIES {
                counts.clear();
            }
        }
        counts.insert(key, (Instant::now(), value));
    }
}

impl Default for SearchRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Query enhancement ──────────────────────────────────────────────────────

/// Build the enhanced Tantivy query for a raw user query.
///
/// Clauses: exact phrase (highest boost), per-term title/body/tags matching,
/// fuzzy variants for terms long enough that a typo is plausible (≥ 4 chars),
/// and a prefix clause on the final term so partial words match while typing.
/// All user input is sanitized first; an empty result means "nothing
/// searchable".
pub fn build_enhanced_search_query(raw_query: &str) -> String {
    let sanitized = crate::routes::semantic_search::sanitize_query(raw_query);
    let mut terms = crate::routes::semantic_search::extract_terms(&sanitized);
    terms.dedup();
    terms.truncate(8);
    if terms.is_empty() {
        return sanitized;
    }

    let exact_phrase = format!("\"{}\"^4", sanitized);
    let title_terms = terms
        .iter()
        .map(|term| format!("title:{term}^3 tags:{term}^2 body:{term}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fuzzy_terms = terms
        .iter()
        .filter(|term| term.len() >= 4)
        .map(|term| format!("title:{term}~1^1.5 body:{term}~1 tags:{term}"))
        .collect::<Vec<_>>()
        .join(" ");

    // Prefix clause for the trailing term supports typeahead ("samsun" finds
    // Samsung) without the cost of leading-wildcard queries.
    let prefix_clause = terms
        .last()
        .filter(|term| term.len() >= 2)
        .map(|term| format!("title:{term}*^2.5 body:{term}*"))
        .unwrap_or_default();

    let mut clauses = vec![format!("({exact_phrase})"), format!("({title_terms})")];
    if !fuzzy_terms.is_empty() {
        clauses.push(format!("({fuzzy_terms})"));
    }
    if !prefix_clause.is_empty() {
        clauses.push(format!("({prefix_clause})"));
    }
    clauses.join(" OR ")
}

// ─── Result post-processing ─────────────────────────────────────────────────

/// Tokenize a raw query for highlighting: alphanumeric runs, lower-cased,
/// minimum 2 chars, at most 8 tokens.
pub fn query_tokens(raw_query: &str) -> Vec<String> {
    raw_query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .take(8)
        .collect()
}

/// Collapse near-duplicate hits (same entity type + id) keeping the best
/// score, blend a recency factor into the score, and re-sort.
///
/// Recency is a modest multiplier (≤ +12% for same-day documents, decaying
/// over ~180 days) so it breaks ties toward fresh intelligence without
/// overriding relevance.
pub fn dedupe_and_rank(results: Vec<SearchResult>, now_ts: i64) -> Vec<SearchResult> {
    const RECENCY_HALF_LIFE_SECS: f64 = 180.0 * 24.0 * 3600.0;
    const RECENCY_WEIGHT: f64 = 0.12;

    let mut best: HashMap<(String, String), SearchResult> = HashMap::new();
    for result in results {
        let key = (result.entity_type.clone(), result.entity_id.clone());
        match best.get(&key) {
            Some(existing) if existing.score >= result.score => {}
            _ => {
                best.insert(key, result);
            }
        }
    }
    let mut ranked: Vec<SearchResult> = best.into_values().collect();
    for result in &mut ranked {
        let age_secs = (now_ts - result.timestamp).max(0) as f64;
        let recency = (-age_secs / RECENCY_HALF_LIFE_SECS).exp().clamp(0.0, 1.0);
        result.score = (f64::from(result.score) * (1.0 + RECENCY_WEIGHT * recency)) as f32;
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked
}

/// Highlight query tokens in a snippet, returning HTML with `<mark>` spans.
///
/// The text is HTML-escaped **before** marks are inserted, so the output is
/// safe to render with `|safe`.
pub fn highlight_snippet_html(text: &str, tokens: &[String]) -> String {
    let escaped = escape_html(text);
    if tokens.is_empty() {
        return escaped;
    }
    let lower = escaped.to_lowercase();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for token in tokens {
        let token_lower = token.to_lowercase();
        if token_lower.is_empty() {
            continue;
        }
        let mut start = 0usize;
        while let Some(position) = lower[start..].find(&token_lower) {
            let absolute = start + position;
            let end = absolute + token_lower.len();
            let boundary_before = absolute == 0
                || !lower[..absolute]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            let boundary_after = end >= lower.len()
                || !lower[end..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
            if boundary_before && boundary_after {
                spans.push((absolute, end));
            }
            start = end;
        }
    }
    if spans.is_empty() {
        return escaped;
    }
    spans.sort_unstable();
    // Merge overlapping spans so nested tokens cannot produce broken markup.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => {
                *last_end = (*last_end).max(end);
            }
            _ => merged.push((start, end)),
        }
    }
    let mut out = String::with_capacity(escaped.len() + merged.len() * 17);
    let mut cursor = 0usize;
    for (start, end) in merged {
        out.push_str(&escaped[cursor..start]);
        out.push_str("<mark>");
        out.push_str(&escaped[start..end]);
        out.push_str("</mark>");
        cursor = end;
    }
    out.push_str(&escaped[cursor..]);
    out
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn hit(entity_type: &str, entity_id: &str, score: f32, timestamp: i64) -> SearchResult {
        SearchResult {
            id: format!("{entity_type}-{entity_id}"),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            title: "t".into(),
            snippet: "s".into(),
            url: String::new(),
            region: String::new(),
            score,
            timestamp,
        }
    }

    #[test]
    fn enhanced_query_includes_phrase_fuzzy_and_prefix_clauses() {
        let query = build_enhanced_search_query("pcb assembly");
        assert!(query.contains("\"pcb assembly\"^4"));
        assert!(query.contains("title:pcb^3"));
        assert!(query.contains("assembly~1"));
        assert!(
            query.contains("title:assembly*^2.5"),
            "prefix clause missing: {query}"
        );
    }

    #[test]
    fn enhanced_query_is_empty_for_unsearchable_input() {
        assert_eq!(build_enhanced_search_query("   ").trim(), "");
        assert_eq!(build_enhanced_search_query("???").trim(), "");
    }

    #[test]
    fn dedupe_keeps_best_score_per_entity() {
        // Ancient timestamps keep the recency multiplier ~1.0 so the score
        // assertion is about dedupe, not recency.
        let now = 1_800_000_000i64;
        let ancient = now - 10 * 180 * 24 * 3600;
        let results = vec![
            hit("company", "1", 2.0, ancient),
            hit("company", "1", 3.0, ancient),
            hit("observation", "2", 1.0, ancient),
        ];
        let ranked = dedupe_and_rank(results, now);
        assert_eq!(ranked.len(), 2);
        let company = ranked.iter().find(|r| r.entity_type == "company").unwrap();
        assert!(
            (company.score - 3.0).abs() < 0.01,
            "score {}",
            company.score
        );
    }

    #[test]
    fn recency_breaks_ties_towards_fresh_documents() {
        let now = 1_800_000_000i64;
        let old = now - 300 * 24 * 3600;
        let results = vec![
            hit("observation", "old", 1.0, old),
            hit("observation", "new", 1.0, now),
        ];
        let ranked = dedupe_and_rank(results, now);
        assert_eq!(ranked[0].entity_id, "new");
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn highlight_escapes_html_and_marks_tokens() {
        let html = highlight_snippet_html("<b>samsung</b> & battery", &["samsung".into()]);
        assert!(html.contains("&lt;b&gt;<mark>samsung</mark>&lt;/b&gt; &amp; battery"));
        assert!(!html.contains("<b>"));
    }

    #[test]
    fn highlight_does_not_mark_inside_words() {
        let html = highlight_snippet_html("samsungex supply", &["samsung".into()]);
        assert!(
            !html.contains("<mark>"),
            "mid-word match must not be marked: {html}"
        );
    }
    #[tokio::test]
    async fn runtime_executes_index_search_off_the_async_runtime() {
        let index = std::sync::Arc::new(SearchIndex::in_memory().unwrap());
        let mut writer = index.writer(15_000_000).unwrap();
        index
            .index_document(
                &writer,
                &uuid::Uuid::new_v4().to_string(),
                "company",
                &uuid::Uuid::new_v4().to_string(),
                "Samsung SDI battery operations",
                "Battery cell supply chain update",
                "https://example.test",
                "KR",
                &[],
                1_700_000_000,
            )
            .unwrap();
        writer.commit().unwrap();
        index.reload().unwrap();

        let runtime = SearchRuntime::new();
        let (results, total) = runtime
            .search_ranked(
                index.clone(),
                build_enhanced_search_query("samsung battery"),
                entity_boosts(),
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_type, "company");

        let count = runtime
            .cached_count_for_type(index.clone(), "samsung battery".into(), "company".into())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn count_cache_returns_fresh_values_and_expires_stale_ones() {
        let runtime = SearchRuntime::new();
        let key = ("q".to_string(), "company".to_string());
        runtime.store_count(key.clone(), 7);
        assert_eq!(runtime.cached_count(&key), Some(7));

        // Poison the entry with a stale timestamp; it must read as a miss.
        {
            let mut counts = runtime.counts.lock().unwrap();
            counts.insert(
                key.clone(),
                (Instant::now() - COUNT_CACHE_TTL - Duration::from_secs(1), 7),
            );
        }
        assert_eq!(runtime.cached_count(&key), None);
    }
}
