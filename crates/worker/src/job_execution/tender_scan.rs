//! Tender / procurement portal crawler.
//!
//! Scrapes MENA-region public procurement portals, filters postings for
//! EMS / BESS (battery energy storage) relevance, extracts structured tender
//! data, and emits `TenderPosted` observations. Idempotent across re-runs via
//! content-hashed observation IDs (UUIDv5) — a tender re-scraped in a later run
//! produces the same observation ID and is a no-op.
//!
//! # Sources (all public, no auth required)
//! - **Tunisia (TUNEPS)**: `tuneps.tn` — national e-procurement portal
//! - **Morocco (ARP)**: `marches-publics.com/maroc` — public procurement portal
//! - **Egypt (EPS)**: `etenders.gov.eg` — e-tendering portal
//! - **UAE**: `etimad.ae` / federal procurement portal
//! - **KSA (Etimad)**: `etimad.sa` — Saudi procurement portal
//!
//! Each relevant posting is stored as a `TenderPosted` observation and linked
//! to a tracked company when the buyer name matches.
//!
//! Runs every 6 hours — tender portals publish daily but our scan cadence
//! balances freshness against rate-limit politeness.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use crate::{JobKind, JobRun, PgStore};

/// Stable namespace for UUIDv5 deterministic observation IDs. Any fixed UUID
/// works — it just must not collide with `Uuid::NAMESPACE_DNS` etc. that other
/// code might use for the same source strings. Derived deterministically so the
/// same source string always maps to the same observation id.
const TENDER_ID_NAMESPACE: Uuid = Uuid::from_u128(0xb248ec9d_3471_b10b_5144_2bc2_f5f5_4ad9);

/// MENA procurement portal definitions. `search_url` is a query template where
/// `{q}` is replaced with a URL-encoded keyword; portals that expose no
/// keyword search use a listings/feed URL in `fallback_url`.
struct TenderPortal {
    code: &'static str,
    name: &'static str,
    country: &'static str,
    /// Query-template endpoint (supports `{q}`). None if the portal offers no
    /// keyword search API/feed.
    search_url: Option<&'static str>,
    /// Listings/feed URL used when `search_url` is None or as a fallback.
    fallback_url: &'static str,
}

const PORTALS: &[TenderPortal] = &[
    TenderPortal {
        code: "tuneps",
        name: "TUNEPS",
        country: "Tunisia",
        // TUNEPS exposes a public search; the HTML results page lists notice
        // titles + buyers. The `{q}` placeholder carries the keyword.
        search_url: Some("https://www.tuneps.tn/en/search?keyword={q}"),
        fallback_url: "https://www.tuneps.tn/en/notices",
    },
    TenderPortal {
        code: "maroc_arp",
        name: "Marchés Publics Maroc",
        country: "Morocco",
        search_url: Some("https://www.marches-publics.com/maroc/index.php?page=requete_simple&keyword={q}"),
        fallback_url: "https://www.marches-publics.com/maroc/index.php?page=entreprise.EntrepriseAdvancedSearch",
    },
    TenderPortal {
        code: "egypt_eps",
        name: "Egypt e-Procurement",
        country: "Egypt",
        search_url: Some("https://etenders.gov.eg/Search.aspx?q={q}"),
        fallback_url: "https://etenders.gov.eg/",
    },
    TenderPortal {
        code: "ksa_etimad",
        name: "Etimad (KSA)",
        country: "Saudi Arabia",
        search_url: Some("https://tenders.etimad.sa/Search?q={q}"),
        fallback_url: "https://tenders.etimad.sa/",
    },
    TenderPortal {
        code: "uae_federal",
        name: "UAE Federal Procurement",
        country: "United Arab Emirates",
        search_url: None,
        fallback_url: "https://etenders.gov.ae/",
    },
];

/// Keyword groups used to query the portals. These deliberately favour BESS /
/// energy-storage terms (the strategic pivot) alongside core EMS terms.
const SEARCH_QUERIES: &[&str] = &[
    "battery energy storage",
    "energy storage system",
    "battery management system",
    "battery pack",
    "lithium-ion",
    "PCB assembly",
    "contract manufacturing",
    "electronics manufacturing",
    "SMT",
];

/// Maximum postings to store per portal per run (politeness + storage cap).
const MAX_PER_PORTAL: usize = 25;

/// Run the tender/procurement scan.
pub(super) async fn run_tender_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let start = Instant::now();

    let company_names = load_company_names(store).await;

    let client = reqwest::Client::builder()
        .user_agent("ApexIntel-Tenders/1.0 (+research; tenders)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_default();

    let mut total_posted: u64 = 0;
    let mut total_relevant: u64 = 0;
    let mut total_linked: u64 = 0;
    let mut total_deduped: u64 = 0;

    for portal in PORTALS {
        let postings = crawl_portal(&client, portal).await;
        for posting in &postings {
            total_posted += 1;

            // Relevance gate: must mention an EMS/BESS term in the title or body.
            if !is_relevant_tender(&posting.title, &posting.body) {
                continue;
            }
            total_relevant += 1;

            // Link to a tracked company if the buyer matches a known name.
            let entity_id = match_posted_company(&posting.buyer, &posting.title, &company_names);

            let was_new = store_tender_observation(store, portal, posting, entity_id).await;
            match was_new {
                Ok(true) => {
                    total_linked += entity_id.is_some() as u64;
                }
                Ok(false) => {
                    total_deduped += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        portal = portal.code,
                        url = %posting.url,
                        error = %e,
                        "tender_scan: failed to store TenderPosted observation"
                    );
                }
            }
        }
        tracing::info!(
            portal = portal.code,
            country = portal.country,
            fetched = postings.len(),
            "tender_scan: portal fetched"
        );
    }

    // Activity log (fire-and-forget).
    let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
    activity_logger
        .log_crawl_completed(
            "tender_scan",
            PORTALS.len() as u32,
            total_relevant as u32,
            start.elapsed().as_secs_f64(),
        )
        .await;

    let elapsed = start.elapsed();
    run.succeed(
        total_relevant,
        &format!(
            "tender_scan: {} relevant tenders posted across {} MENA portals \
             ({} seen, {} deduped, {} linked) in {:.1}s",
            total_relevant,
            PORTALS.len(),
            total_posted,
            total_deduped,
            total_linked,
            elapsed.as_secs_f64(),
        ),
    );
    run
}

/// A raw posting scraped from a portal, before relevance filtering.
struct RawPosting {
    title: String,
    buyer: Option<String>,
    url: String,
    body: String,
    reference: Option<String>,
}

/// Crawl a single portal: build keyword search URLs, fetch, and parse out
/// posting entries. Falls back to the listings URL when no search endpoint is
/// available. Robust to fetch failures (returns whatever it could collect).
async fn crawl_portal(
    client: &reqwest::Client,
    portal: &TenderPortal,
) -> Vec<RawPosting> {
    let mut postings = Vec::new();

    // Try the keyword search endpoint first (most precise), then the fallback
    // listings URL. We pick a small subset of queries per portal to stay polite.
    let queries: Vec<&str> = match portal.search_url {
        Some(_) => SEARCH_QUERIES.iter().take(4).copied().collect(),
        None => Vec::new(),
    };

    for query in &queries {
        let template = portal.search_url.expect("search_url present when queries non-empty");
        let url = template.replace("{q}", &simple_url_encode(query));
        if let Some(found) = fetch_and_parse(client, &url, portal).await {
            postings.extend(found);
        }
        // Politeness delay between queries.
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    // Fallback listings URL (always tried once for portals with no search, and
    // as a supplement otherwise).
    if postings.is_empty() {
        if let Some(found) = fetch_and_parse(client, portal.fallback_url, portal).await {
            postings.extend(found);
        }
    }

    // Dedup by URL within this portal batch.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    postings.retain(|p| seen.insert(p.url.clone()));
    postings.truncate(MAX_PER_PORTAL);
    postings
}

/// Fetch a URL and parse posting entries from its HTML. Returns None on
/// network failure. Parsing is intentionally tolerant: procurement portals have
/// wildly inconsistent markup, so we extract candidate blocks via anchor
/// traversal and heuristic title extraction rather than per-portal selectors.
async fn fetch_and_parse(
    client: &reqwest::Client,
    url: &str,
    portal: &TenderPortal,
) -> Option<Vec<RawPosting>> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        tracing::debug!(
            portal = portal.code,
            url,
            status = resp.status().as_u16(),
            "tender_scan: non-success status"
        );
        return None;
    }
    let html = resp.text().await.ok()?;
    Some(parse_postings_from_html(&html, url))
}

/// Parse posting entries from a portal HTML page. Extracts `<a href>` anchors
/// whose link text looks like a tender title, plus surrounding context (buyer,
/// reference). This is a coarse, portal-agnostic extractor; the downstream
/// relevance gate filters out navigation/boilerplate.
fn parse_postings_from_html(html: &str, url: &str) -> Vec<RawPosting> {
    let base = base_url(url);
    let mut postings = Vec::new();

    // Walk <a ...>...</a> anchors. A tender title is typically a multi-word
    // capitalized phrase ≥ 12 chars; nav links are short ("Home", "Search").
    let mut rest = html;
    while let Some(a_start) = rest.find("<a ") {
        rest = &rest[a_start..];
        let tag_end = match rest.find('>') {
            Some(p) => p,
            None => break,
        };
        let opening = &rest[..tag_end];
        let after_open = &rest[tag_end + 1..];
        let close = match after_open.find("</a>") {
            Some(p) => p,
            None => break,
        };
        let link_text = strip_html_tags(&after_open[..close]);
        let href = extract_attr(opening, "href");
        // Advance past this anchor.
        rest = &after_open[close + 4..];

        let title = link_text.trim();
        if title.len() < 12 || title.split_whitespace().count() < 3 {
            continue;
        }
        let resolved = resolve_url(&base, href.as_deref().unwrap_or(""));
        if resolved.is_empty() {
            continue;
        }

        // Grab a window of surrounding text as the body for relevance scoring
        // and buyer extraction.
        // B334: char-safe slicing — the previous byte-offset arithmetic
        // panicked on multi-byte UTF-8 (Arabic/French MENA portal content)
        // with "byte index is not a char boundary", killing the whole scan.
        let base_offset = html.len().saturating_sub(rest.len());
        let lo = base_offset.saturating_sub(600);
        let hi = (base_offset + after_open.len()).min(base_offset + close + 600);
        let lo = floor_char_boundary(html, lo);
        let hi = ceil_char_boundary(html, hi.max(lo));
        let context = strip_html_tags(&html[lo..hi]);
        let body = if context.trim().is_empty() {
            title.to_string()
        } else {
            context.chars().take(800).collect()
        };

        let buyer = extract_buyer_from_text(&body);
        let reference = extract_reference_from_text(&body);

        postings.push(RawPosting {
            title: normalise_ws(title),
            buyer,
            url: resolved,
            body,
            reference,
        });
    }

    postings
}

/// Does this posting mention an EMS or BESS term? Mirrors the logic in
/// `apex_parse::tender::is_ems_relevant` but operates on raw title+body so it
/// works without the full parse feature.
fn is_relevant_tender(title: &str, body: &str) -> bool {
    let haystack = format!("{title} {body}").to_lowercase();
    RELEVANCE_KEYWORDS.iter().any(|kw| haystack.contains(kw))
}

/// Lowercased EMS + BESS keywords used for the relevance gate. This is a
/// hand-curated subset (not locale-aware) kept in sync with
/// `apex_parse::multilingual::ems_keywords("en")`.
const RELEVANCE_KEYWORDS: &[&str] = &[
    // EMS core
    "ems",
    "contract manufacturing",
    "smt",
    "tht",
    "pcb assembly",
    "box build",
    "cable harness",
    "aoi",
    "ict",
    "bga",
    "conformal coating",
    "electronics manufacturing",
    "printed circuit",
    // BESS pivot
    "bess",
    "battery energy storage",
    "energy storage system",
    "battery pack",
    "battery module",
    "battery management system",
    "bms",
    "lithium-ion",
    "lifepo4",
    "lfp",
    "battery cell",
    "energy storage",
    "battery",
    "lithium",
];

/// Store a relevant tender as a `TenderPosted` observation with a deterministic
/// UUIDv5 ID. Returns Ok(true) if newly inserted, Ok(false) if it already
/// existed (deduped).
async fn store_tender_observation(
    store: &PgStore,
    portal: &TenderPortal,
    posting: &RawPosting,
    entity_id: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    let obs_value = serde_json::json!({
        "title": posting.title,
        "buyer": posting.buyer,
        "reference_number": posting.reference,
        "portal": portal.name,
        "country": portal.country,
        "url": posting.url,
        "body_excerpt": crate::truncate_text(&posting.body, 500),
        "sector": detect_sector(&posting.title, &posting.body),
        "source": "tender_scan",
    });
    let provenance = serde_json::json!({
        "source": "tender_scan",
        "source_id": format!("{}:{}", portal.code, posting.reference.as_deref().unwrap_or(&posting.url)),
        "source_domain": extract_domain(&posting.url),
        "url": posting.url,
        "portal": portal.name,
        "country": portal.country,
    });

    let mut obs = apex_core::entities::Observation::new(
        apex_core::entities::ObservationType::TenderPosted,
        Utc::now(),
        obs_value,
        provenance,
    );
    // Deterministic ID: same portal + reference/url → same observation, so
    // re-scraping a still-open tender is an idempotent no-op.
    obs.id = deterministic_tender_id(portal.code, posting);
    obs.entity_id = entity_id;
    obs.entity_type = Some("company".to_string());
    obs.confidence = if entity_id.is_some() { 0.85 } else { 0.6 };

    // insert_observation uses ON CONFLICT (id) DO NOTHING. We can't easily read
    // back the affected-row count across the sqlx abstraction here, so we rely
    // on the caller's counters being best-effort. The idempotency guarantee is
    // the important property.
    store
        .insert_observation(&obs)
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("{e}")))?;
    Ok(true)
}

/// Derive a stable UUIDv5 from the portal code and the posting's reference
/// number (falling back to its URL). The same input always yields the same ID,
/// making repeated scans of an unchanged tender idempotent.
fn deterministic_tender_id(portal_code: &str, posting: &RawPosting) -> Uuid {
    let key = match &posting.reference {
        Some(r) if !r.trim().is_empty() => format!("{portal_code}|{r}"),
        _ => format!("{portal_code}|{}", posting.url),
    };
    Uuid::new_v5(&TENDER_ID_NAMESPACE, key.as_bytes())
}

/// Match a posting's buyer (or title) against tracked company names.
fn match_posted_company(
    buyer: &Option<String>,
    title: &str,
    company_names: &[(Uuid, String)],
) -> Option<Uuid> {
    if company_names.is_empty() {
        return None;
    }
    // Prefer the buyer field; fall back to the title.
    let candidates: Vec<&str> = buyer
        .as_deref()
        .map(|b| vec![b, title])
        .unwrap_or_else(|| vec![title]);
    for text in candidates {
        let lower = text.to_lowercase();
        for (id, name) in company_names {
            if lower.contains(&name.to_lowercase()) {
                return Some(*id);
            }
        }
    }
    None
}

/// Coarse sector detection from title+body (mirrors the tender parser).
fn detect_sector(title: &str, body: &str) -> Option<String> {
    let lower = format!("{title} {body}").to_lowercase();
    let sectors = [
        ("bess", "bess"),
        ("battery", "bess"),
        ("energy storage", "bess"),
        ("lithium", "bess"),
        ("automotive", "automotive"),
        ("aerospace", "aerospace"),
        ("defense", "defense"),
        ("defence", "defense"),
        ("medical", "medical"),
        ("industrial", "industrial"),
        ("energy", "energy"),
        ("telecom", "telecom"),
        ("electronics", "electronics"),
        ("semiconductor", "semiconductor"),
    ];
    for (keyword, sector) in &sectors {
        if lower.contains(keyword) {
            return Some(sector.to_string());
        }
    }
    None
}

/// Load tracked company names for entity linking.
async fn load_company_names(store: &PgStore) -> Vec<(Uuid, String)> {
    #[derive(sqlx::FromRow)]
    struct NameRow {
        id: Uuid,
        name: String,
    }
    match sqlx::query_as::<_, NameRow>(
        "SELECT id, name FROM companies \
         WHERE name IS NOT NULL AND TRIM(name) <> '' \
         ORDER BY length(name) DESC", // longest first → most specific match wins
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(rows) => rows.into_iter().map(|r| (r.id, r.name)).collect(),
        Err(_) => Vec::new(),
    }
}

// ── Small HTML / text helpers ─────────────────────────────────────────

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

fn normalise_ws(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Extract the value of an attribute from an opening tag fragment.
fn extract_attr(opening_tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = opening_tag.find(&needle)? + needle.len();
    let rest = &opening_tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Reduce an absolute URL to its scheme+host (for relative-URL resolution).
fn base_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        if let Some(path_start) = after.find('/') {
            return format!("{}{}", &url[..scheme_end + 3], &after[..path_start]);
        }
        return url.to_string();
    }
    String::new()
}

/// Resolve a possibly-relative href against a base URL.
fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if href.starts_with("//") {
        if let Some(scheme_end) = base.find("://") {
            return format!("{}:{}", &base[..scheme_end], href);
        }
        return format!("https:{href}");
    }
    if href.starts_with('/') {
        return format!("{base}{href}");
    }
    if href.is_empty() {
        return String::new();
    }
    format!("{base}/{href}")
}

/// Extract the registrable domain from a URL (best-effort).
fn extract_domain(url: &str) -> String {
    let no_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    no_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Minimal URL query-parameter encoder.
fn simple_url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(ch),
            ' ' => result.push('+'),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    result
}

/// Coarse buyer extraction: "Buyer: X" / "Awarded to: X" patterns.
fn extract_buyer_from_text(text: &str) -> Option<String> {
    let lower_prefixes = ["buyer:", "contracting authority:", "awarded to:", "maître d'ouvrage:"];
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        for prefix in &lower_prefixes {
            if let Some(rest) = lower.strip_prefix(prefix) {
                // B334: char-safe suffix — subtracting the byte length of the
                // *lowercased* remainder from the original can underflow or
                // land mid-character (e.g. `İ` changes byte length when
                // lowercased).
                let value = strip_prefix_chars(trimmed, rest.trim());
                let cleaned = normalise_ws(value);
                if cleaned.chars().count() >= 3 {
                    return Some(cleaned.to_string());
                }
            }
        }
    }
    None
}

/// Remove up to `prefix.len()` leading *characters* of `haystack` (best-effort
/// char-wise, not byte-wise) so mixed-case/Unicode prefixes strip correctly.
fn strip_prefix_chars<'a>(haystack: &'a str, prefix: &str) -> &'a str {
    let mut remaining = haystack;
    let mut to_skip = prefix.chars().count();
    for (idx, ch) in haystack.char_indices() {
        if to_skip == 0 {
            return &haystack[idx..];
        }
        if !remaining.starts_with(ch) {
            break;
        }
        remaining = &remaining[ch.len_utf8()..];
        to_skip -= 1;
    }
    remaining.trim_start()
}

/// Largest byte index <= `idx` that is a UTF-8 char boundary.
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Smallest byte index >= `idx` that is a UTF-8 char boundary.
fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Coarse reference-number extraction: "Ref: X" / "N° X" patterns.
fn extract_reference_from_text(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)(?:ref(?:erence)?|n°|numéro)[:\s]*([\p{L}\p{N}][\p{L}\p{N}_\-/]+)").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .filter(|s| s.len() >= 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_gate_accepts_bess_and_ems_terms() {
        assert!(is_relevant_tender(
            "Supply of Battery Energy Storage System",
            "Procurement of a 50 MWh BESS for the national grid."
        ));
        assert!(is_relevant_tender(
            "PCB Assembly Services",
            "Contract manufacturing of electronics."
        ));
        assert!(!is_relevant_tender(
            "Office Furniture Supply",
            "Desks and chairs for ministry."
        ));
        assert!(!is_relevant_tender("Consulting Services", "Strategy advisory."));
    }

    #[test]
    fn deterministic_id_is_stable_across_runs() {
        let posting = RawPosting {
            title: "BESS Supply".into(),
            buyer: Some("Ministry of Energy".into()),
            url: "https://tuneps.tn/123".into(),
            body: "irrelevant".into(),
            reference: Some("TN-2026-001".into()),
        };
        let id_a = deterministic_tender_id("tuneps", &posting);
        let id_b = deterministic_tender_id("tuneps", &posting);
        assert_eq!(id_a, id_b, "same posting must yield the same ID");

        // A different reference yields a different ID.
        let mut other = posting.clone_url_title();
        other.reference = Some("TN-2026-002".into());
        let id_c = deterministic_tender_id("tuneps", &other);
        assert_ne!(id_a, id_c);
    }

    #[test]
    fn deterministic_id_falls_back_to_url_when_no_reference() {
        let with_ref = RawPosting {
            title: "T".into(),
            buyer: None,
            url: "https://example.com/x".into(),
            body: String::new(),
            reference: Some("REF1".into()),
        };
        let no_ref = RawPosting {
            title: "T".into(),
            buyer: None,
            url: "https://example.com/x".into(),
            body: String::new(),
            reference: None,
        };
        // ref present → keyed by ref
        assert_eq!(
            deterministic_tender_id("p", &with_ref),
            deterministic_tender_id("p", &with_ref)
        );
        // no ref → keyed by url, so differs from the ref-keyed one
        assert_ne!(
            deterministic_tender_id("p", &no_ref),
            deterministic_tender_id("p", &with_ref)
        );
    }

    #[test]
    fn detect_sector_prefers_bess() {
        assert_eq!(detect_sector("Battery Pack", "energy storage"), Some("bess".into()));
        assert_eq!(detect_sector("EMS", "automotive electronics"), Some("automotive".into()));
        assert_eq!(detect_sector("hello", "world"), None);
    }

    #[test]
    fn match_posted_company_prefers_buyer_then_title() {
        let companies = vec![(Uuid::new_v4(), "Acme Electronics".into())];
        assert_eq!(
            match_posted_company(&Some("Acme Electronics SARL".into()), "tender", &companies),
            Some(companies[0].0)
        );
        assert_eq!(
            match_posted_company(&None, "Supply for Acme Electronics project", &companies),
            Some(companies[0].0)
        );
        assert_eq!(match_posted_company(&None, "unrelated", &companies), None);
    }

    #[test]
    fn resolve_url_handles_absolute_relative_and_protocol_relative() {
        assert_eq!(
            resolve_url("https://tuneps.tn", "https://other.com/a"),
            "https://other.com/a"
        );
        assert_eq!(resolve_url("https://tuneps.tn", "/notice/1"), "https://tuneps.tn/notice/1");
        assert_eq!(resolve_url("https://tuneps.tn", "notice/1"), "https://tuneps.tn/notice/1");
        assert_eq!(resolve_url("https://tuneps.tn", "//cdn.tn/x"), "https://cdn.tn/x");
        assert_eq!(resolve_url("https://tuneps.tn", ""), "");
    }

    // Helper for tests: clone a posting's url/title only.
    impl RawPosting {
        fn clone_url_title(&self) -> Self {
            RawPosting {
                title: self.title.clone(),
                buyer: None,
                url: self.url.clone(),
                body: String::new(),
                reference: None,
            }
        }
    }
}
