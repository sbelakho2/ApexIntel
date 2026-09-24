//! Search handler — GET /search
//!
//! Covers: full-text search across all entities (companies, persons,
//! warnings, insights) with faceted results and highlighting.

use std::sync::Arc;

use askama::Template;
use axum::{
    http::HeaderMap,
    response::{Html, IntoResponse},
    Extension,
};
use serde::Deserialize;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::autocomplete::AutocompleteIndex;
use apex_store::postgres::{PgStore, WarningListFilters};
use apex_store::tantivy_index::SearchIndex;

// ─── Query params ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchPageQuery {
    pub q: Option<String>,
    pub entity_type: Option<String>, // "all" | "company" | "person" | "warning" | "insight"
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SearchResultItem {
    pub entity_type: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub snippet: String,
    pub score: f64,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct SearchFacet {
    pub label: String,
    /// URL slug for the facet link (`all`, `company`, `person`, …).
    pub slug: String,
    pub count: i64,
    pub active: bool,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/search.html")]
pub struct SearchPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub query: String,
    pub results: Vec<SearchResultItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub facets: Vec<SearchFacet>,
    pub active_type: String,
    pub took_ms: i64,
}

/// HTMX partial for live-search swaps: facets + results + pager (B302).
/// Previously the HTMX branch returned an HTML comment placeholder, so every
/// keystroke replaced the results area with nothing.
#[derive(Template)]
#[template(path = "pages/search/_results.html")]
pub struct SearchResultsPartial {
    pub query: String,
    pub results: Vec<SearchResultItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub facets: Vec<SearchFacet>,
    pub active_type: String,
    pub took_ms: i64,
}

// ─── HTMX autocomplete partial template ────────────────────────────────────

#[derive(Template)]
#[template(path = "search_suggestions.html")]
pub struct SearchSuggestionsPartial {
    pub suggestions: Vec<SuggestItemPartial>,
}

#[derive(Clone, Debug)]
pub struct SuggestItemPartial {
    pub text: String,
    pub entity_type: String,
    pub id: String,
    pub subtext: Option<String>,
    pub score: f64,
    pub url: String,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /search — entity search page.
pub async fn search_page(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Extension(search_index): Extension<Arc<SearchIndex>>,
    axum::extract::Query(params): axum::extract::Query<SearchPageQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/search", unack);
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);
    let query_str = params.q.unwrap_or_default();
    // B303: accept both singular (`company`) and plural (`companies`) facet
    // slugs — the templates used to emit plurals while the index stores
    // singular entity types, so every facet filter matched zero documents.
    let active_type = normalize_entity_type(params.entity_type.as_deref().unwrap_or("all"));

    let start = std::time::Instant::now();

    let (results, total, facets) = if query_str.trim().is_empty() {
        (vec![], 0i64, build_empty_facets(&active_type))
    } else {
        // B303: sanitize before handing to the Tantivy query parser — raw
        // `field:value`, `+`, and `^` operators made /search behave
        // differently from /api/search and could silently error out.
        let sanitized = crate::routes::semantic_search::sanitize_query(&query_str);
        if sanitized.is_empty() {
            (vec![], 0i64, build_empty_facets(&active_type))
        } else {
            let offset = ((page - 1) * per_page) as usize;
            let limit = per_page as usize;

            let (items, total_hits) = if active_type == "all" {
                search_index
                    .search_with_total(&sanitized, limit, offset)
                    .unwrap_or_else(|e| {
                        tracing::error!("Search failed: {e}");
                        (vec![], 0)
                    })
            } else {
                search_index
                    .search_entity_type_with_total(&sanitized, &active_type, limit, offset)
                    .unwrap_or_else(|e| {
                        tracing::error!("Search entity_type failed: {e}");
                        (vec![], 0)
                    })
            };

            let mapped: Vec<SearchResultItem> = items
                .iter()
                .map(|sr| {
                    let url = if sr.url.is_empty() {
                        format!("/{}/{}", sr.entity_type, sr.entity_id)
                    } else {
                        sr.url.clone()
                    };
                    SearchResultItem {
                        entity_type: sr.entity_type.clone(),
                        id: sr.entity_id.clone(),
                        title: sr.title.clone(),
                        subtitle: sr.region.clone(),
                        snippet: sr.snippet.clone(),
                        score: sr.score as f64,
                        url,
                    }
                })
                .collect();

            // B302: real facet counts. One count-only probe per entity type —
            // the previous `build_empty_facets` rendered `(0)` next to every
            // facet even when results existed.
            let all_total = search_index
                .search_with_total(&sanitized, 1, 0)
                .map(|(_, total)| total as i64)
                .unwrap_or(total_hits as i64);
            let facet_counts: Vec<(&str, i64)> = ["company", "person", "warning", "insight"]
                .into_iter()
                .map(|t| {
                    let count = search_index
                        .search_entity_type_with_total(&sanitized, t, 1, 0)
                        .map(|(_, total)| total as i64)
                        .unwrap_or(0);
                    (t, count)
                })
                .collect();

            let facets = build_facets(&active_type, all_total, &facet_counts);

            (mapped, total_hits as i64, facets)
        }
    };

    let took_ms = start.elapsed().as_millis() as i64;
    let total_pages = if per_page > 0 {
        (total + per_page - 1) / per_page
    } else {
        0
    };

    if is_htmx_request(&headers) {
        let partial = SearchResultsPartial {
            query: query_str,
            results,
            total,
            page,
            per_page,
            total_pages,
            facets,
            active_type,
            took_ms,
        };
        super::render_template(&partial)
    } else {
        let tpl = SearchPage {
            current_path: ctx.current_path,
            can_admin: ctx.can_admin,
            status_strip: crate::system_status::StatusStrip::current(),
            username: ctx.username,
            warning_count: ctx.warning_count,
            theme: ctx.theme,
            query: query_str,
            results,
            total,
            page,
            per_page,
            total_pages,
            facets,
            active_type,
            took_ms,
        };
        super::render_template(&tpl)
    }
}

/// Map plural/singular/unknown facet slugs onto the singular entity types
/// stored in the search index (`company|person|warning|insight`).
fn normalize_entity_type(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "company" | "companies" => "company".into(),
        "person" | "persons" | "people" => "person".into(),
        "warning" | "warnings" => "warning".into(),
        "insight" | "insights" => "insight".into(),
        _ => "all".into(),
    }
}

fn build_facets(active_type: &str, all_total: i64, counts: &[(&str, i64)]) -> Vec<SearchFacet> {
    let mut facets = vec![SearchFacet {
        label: "All".into(),
        slug: "all".into(),
        count: all_total,
        active: active_type == "all",
    }];
    let labels = [
        ("company", "Companies"),
        ("person", "Persons"),
        ("warning", "Warnings"),
        ("insight", "Insights"),
    ];
    for (slug, label) in labels {
        let count = counts
            .iter()
            .find(|(s, _)| *s == slug)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        facets.push(SearchFacet {
            label: label.into(),
            slug: slug.into(),
            count,
            active: active_type == slug,
        });
    }
    facets
}

fn build_empty_facets(active_type: &str) -> Vec<SearchFacet> {
    build_facets(active_type, 0, &[])
}

/// GET /search/suggestions — HTMX partial returning autocomplete dropdown.
pub async fn suggestions_html(
    Extension(autocomplete_index): Extension<Arc<std::sync::RwLock<AutocompleteIndex>>>,
    axum::extract::Query(params): axum::extract::Query<SearchPageQuery>,
) -> impl IntoResponse {
    let query = params.q.unwrap_or_default().trim().to_lowercase();
    if query.len() < 2 {
        return Html("".to_string()).into_response();
    }

    // B305: recover from lock poisoning instead of panicking the worker
    // thread — the autocomplete index is a pure read-mostly cache and a
    // poisoned lock previously turned every subsequent suggestion request
    // into a 500.
    let results = autocomplete_index
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .suggest(&query, 10);

    let items: Vec<SuggestItemPartial> = results
        .into_iter()
        .map(|s| {
            let url = match s.entity_type.as_str() {
                "company" => format!("/companies/{}", s.id),
                "person" => format!("/persons/{}", s.id),
                "insight" => format!("/insights/{}", s.id),
                "warning" => format!("/warnings/{}", s.id),
                _ => format!("/search?q={}", urlencoding(&s.text)),
            };
            SuggestItemPartial {
                text: s.text,
                entity_type: s.entity_type,
                id: s.id.to_string(),
                subtext: s.subtext,
                score: s.score,
                url,
            }
        })
        .collect();

    if items.is_empty() {
        return Html("".to_string()).into_response();
    }

    let tpl = SearchSuggestionsPartial { suggestions: items };
    super::render_template(&tpl)
}

/// Percent-encode a query string value (B304). The previous version only
/// replaced spaces — `&`, `#`, `%`, and `+` in entity names silently corrupted
/// the fallback search URL.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
