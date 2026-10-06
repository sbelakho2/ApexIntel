//! Search handler — GET /search
//!
//! Covers: full-text search across all entities (companies, persons,
//! warnings, insights) with faceted results and highlighting.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Form, Path},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{is_htmx_request, safe_href, PageContext};
use crate::middleware::session::WebSession;
use crate::search_runtime::{
    build_enhanced_search_query, dedupe_and_rank, entity_boosts, highlight_snippet_html,
    query_tokens, search_runtime, FACET_ENTITY_TYPES,
};
use apex_core::data_state::{DataState, DegradedNotice};
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

// ─── Saved searches (server-rendered personal search section) ──────────────

/// One of the caller's saved searches, rendered on the search page.
#[derive(Clone, Debug)]
pub struct SavedSearchItem {
    pub id: Uuid,
    pub name: String,
    pub query: String,
    pub entity_type: String,
}

impl SavedSearchItem {
    /// Rebuild the search URL for "load this saved search" links.
    pub fn load_url(&self) -> String {
        let mut url = format!("/search?q={}", urlencode(&self.query));
        if !self.entity_type.is_empty() && self.entity_type != "all" {
            url.push_str(&format!("&entity_type={}", urlencode(&self.entity_type)));
        }
        url
    }
}

fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// Body of `POST /search/saved-searches`.
#[derive(Debug, Deserialize)]
pub struct SaveSearchForm {
    pub name: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub entity_type: Option<String>,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/search.html")]
pub struct SearchPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
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
    pub saved_searches: Vec<SavedSearchItem>,

    pub degraded_notice: Option<String>,
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
    pub degraded_notice: Option<String>,
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

/// #127: clamp the page before computing the offset — an unbounded page
/// overflowed the `(page - 1) * per_page` multiplication (debug panic) and
/// asked the index for an absurd offset. 500 pages is the documented cap.
pub const MAX_SEARCH_PAGE: i64 = 500;

fn clamp_search_page(page: Option<i64>) -> i64 {
    page.unwrap_or(1).clamp(1, MAX_SEARCH_PAGE)
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
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web search page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/search", unack_state.into_loaded_or(0));
    let page = clamp_search_page(params.page);
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
        // Sanitization + enhancement happen inside the shared builder, so
        // /search and /api/search rank identically (B303: they used to
        // diverge because the page skipped the enhanced query).
        let enhanced = build_enhanced_search_query(&query_str);
        if enhanced.trim().is_empty() {
            (vec![], 0i64, build_empty_facets(&active_type))
        } else {
            let offset = ((page - 1) * per_page) as usize;
            let limit = per_page as usize;

            let runtime = search_runtime();
            let search_state = DataState::from_result(
                if active_type == "all" {
                    runtime
                        .search_ranked(
                            search_index.clone(),
                            enhanced.clone(),
                            entity_boosts(),
                            limit,
                            offset,
                        )
                        .await
                } else {
                    runtime
                        .search_entity_type_ranked(
                            search_index.clone(),
                            enhanced.clone(),
                            active_type.clone(),
                            limit,
                            offset,
                        )
                        .await
                },
                "search index query failed (web search page)",
                |(items, _)| items.is_empty(),
            );
            DegradedNotice::capture(&search_state, &mut degraded_notice);
            let (items, total_hits) = search_state.into_loaded_or((vec![], 0));

            // Rank: collapse duplicate entities, nudge fresh documents up.
            let items = dedupe_and_rank(items, chrono::Utc::now().timestamp());
            let tokens = query_tokens(&query_str);

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
                        // HTML-escaped + <mark> highlighted; rendered |safe.
                        snippet: highlight_snippet_html(&sr.snippet, &tokens),
                        score: sr.score as f64,
                        url: safe_href(&url),
                    }
                })
                .collect();

            // Facet counts: cached, count-only probes. They are skipped
            // entirely when the query matches nothing (all zero anyway).
            let facet_counts: Vec<(&str, i64)> = if total_hits == 0 {
                FACET_ENTITY_TYPES.iter().map(|t| (*t, 0)).collect()
            } else {
                let mut counts = Vec::with_capacity(FACET_ENTITY_TYPES.len());
                for facet_type in FACET_ENTITY_TYPES {
                    let count_state = DataState::from_result(
                        runtime
                            .cached_count_for_type(
                                search_index.clone(),
                                enhanced.clone(),
                                facet_type.to_string(),
                            )
                            .await,
                        "search index facet probe failed (web search page)",
                        |_| false,
                    );
                    DegradedNotice::capture(&count_state, &mut degraded_notice);
                    let count = count_state.map(|c| c as i64).into_loaded_or(0);
                    counts.push((facet_type, count));
                }
                counts
            };

            // The overall total comes from the main search itself; the old
            // separate "all total" probe was a duplicate index pass.
            let all_total = total_hits as i64;
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
            degraded_notice: degraded_notice.clone(),
        };
        super::render_template(&partial)
    } else {
        let saved_searches_state = DataState::from_result(
            store
                .list_saved_searches_scoped(&session.user_id, session.role.as_str())
                .await,
            "list_saved_searches_scoped failed (web search page)",
            Vec::is_empty,
        );
        DegradedNotice::capture(&saved_searches_state, &mut degraded_notice);
        let saved_searches = saved_searches_state
            .into_items()
            .into_iter()
            .map(|record| {
                let entity_type = record
                    .filters
                    .get("entity_type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("all")
                    .to_string();
                SavedSearchItem {
                    id: record.id,
                    name: record.name,
                    query: record.query_text,
                    entity_type,
                }
            })
            .collect();
        let tpl = SearchPage {
            current_path: ctx.current_path,
            can_admin: ctx.can_admin,
            can_write: ctx.can_write,
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
            saved_searches,

            degraded_notice,
        };
        super::render_template(&tpl)
    }
}

/// POST /search/saved-searches — save the current query for the caller.
pub async fn save_search(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<SaveSearchForm>,
) -> impl IntoResponse {
    let name = form.name.trim();
    let query = form.q.trim();
    if name.is_empty() || query.is_empty() {
        return Redirect::to(&format!("/search?q={}", urlencode(query))).into_response();
    }
    let entity_type = normalize_entity_type(form.entity_type.as_deref().unwrap_or("all"));
    let filters = serde_json::json!({ "entity_type": entity_type });
    if let Err(error) = store
        .upsert_saved_search_scoped(
            &session.user_id,
            session.role.as_str(),
            None,
            name,
            query,
            &filters,
            None,
        )
        .await
    {
        tracing::error!(%error, "failed to save search (web search page)");
        // Authoritative persistence: do not redirect as if the search was
        // saved when the write failed.
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save search").into_response();
    }
    Redirect::to(&format!(
        "/search?q={}&entity_type={}",
        urlencode(query),
        urlencode(&entity_type)
    ))
    .into_response()
}

/// POST /search/saved-searches/:id/delete — delete one of the caller's saved
/// searches. Deleting another user's row is a no-op (ownership is part of the
/// scoped statement).
pub async fn delete_saved_search(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid saved search ID").into_response();
        }
    };
    if let Err(error) = store
        .delete_saved_search_scoped(&session.user_id, session.role.as_str(), uuid)
        .await
    {
        tracing::error!(%error, "failed to delete saved search (web search page)");
        // Authoritative persistence: do not redirect as if the search was
        // deleted when the write failed.
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to delete saved search",
        )
            .into_response();
    }
    Redirect::to("/search").into_response()
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

#[cfg(test)]
mod tests {
    use super::clamp_search_page;

    #[test]
    fn search_page_is_clamped_to_the_documented_cap() {
        assert_eq!(clamp_search_page(None), 1);
        assert_eq!(clamp_search_page(Some(0)), 1);
        assert_eq!(clamp_search_page(Some(-4)), 1);
        assert_eq!(clamp_search_page(Some(3)), 3);
        assert_eq!(clamp_search_page(Some(500)), 500);
        assert_eq!(clamp_search_page(Some(501)), 500);
        assert_eq!(
            clamp_search_page(Some(i64::MAX)),
            500,
            "an unbounded page overflowed the offset multiplication"
        );
    }
}
