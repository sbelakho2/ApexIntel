//! POI network-expansion engine — discovers new Persons of Interest by
//! systematically mining the professional network around existing seed POIs.
//!
//! # Expansion strategies
//!
//! | Strategy | Source | Signal |
//! |----------|--------|--------|
//! | Org leadership | Company website `/leadership`, `/team`, `/board` | Same org, different titles |
//! | GDELT co-mention | GDELT 2.0 EventSearch API | Co-mentioned in news events |
//! | OpenCorporates board | OpenCorporates officer/director search | Co-directors of same company |
//! | Conference speakers | Event aggregator / CFP pages | Same sector conference |
//! | LinkedIn "also viewed" | LinkedIn public pages (proxied) | Peer network signals |
//! | Citation network | Semantic Scholar co-author graph | Academic/technical POIs |
//! | Dark-web intel | TorClient (pwndb, exposed.vc, dread) | Contact detail enrichment |
//!
//! # Data flow
//! ```text
//! existing_persons ──► PoiExpansionEngine::expand_from_seeds()
//!                              │
//!              ┌──────────────┴────────────────┐
//!              ▼                               ▼
//!      org leadership              GDELT co-mention
//!      scraper                     + OpenCorporates
//!              │                               │
//!              └──────────────┬────────────────┘
//!                             ▼
//!                    Vec<DiscoveredPoi> (deduplicated)
//!                             │
//!                    caller: store.insert_person()
//! ```

use anyhow::{Context, Result};
use apex_core::person_names::looks_like_person_name as unicode_person_name;
use chrono::Utc;
use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tracing::{debug, info};

use crate::headers::random_headers;
use crate::proxy::ProxyRotator;

// ─────────────────────────────────────────────────────────────────────────────
// Regex helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Matches typical Western full-name patterns (2–4 capitalised words).
static RE_FULL_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z][a-z]{1,20}(?:\s+[A-Z][a-z]{1,20}){1,3})\b").unwrap());

/// Capitalised job title with trailing preposition — "Chief Executive Officer at".
static RE_TITLE_AT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(CEO|CTO|CFO|COO|CMO|CISO|CRO|President|Director|VP|Vice President|Head of|Manager|Secretary|Minister|General|Admiral|Ambassador|Chairman|Commissioner|Governor)\b[^<]{0,120}?\bat\b",
    )
    .unwrap()
});

/// HTML tag stripper.
static RE_HTML_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

/// Anchor href extractor.
static RE_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href\s*=\s*["']([^"'#]+)["']"#).unwrap());

/// Email regex.
static RE_EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap());

/// LinkedIn profile URL.
static RE_LINKEDIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?linkedin\.com/in/([a-zA-Z0-9\-_%]+)").unwrap()
});

/// Downstream worker logic drops GDELT candidates below 0.55 before LLM validation.
const GDELT_BASE_CONFIDENCE: f32 = 0.55;

/// Common GDELT headline phrases that usually yield market/news fragments, not people.
static GDELT_NON_PERSON_TITLE_PHRASES: &[&str] = &[
    "price target",
    "financial results",
    "shares purchased",
    "shares gap down",
    "trading up",
    "should you buy",
    "breaking news",
    "newsflow drives markets",
    "million deal",
    "raises investments",
    "stock price",
    "erratic futures",
];

/// Individual words that are especially noisy in GDELT title-derived false positives.
static GDELT_NON_PERSON_WORDS: &[&str] = &[
    "beam",
    "capital",
    "deal",
    "financial",
    "futures",
    "high",
    "industrial",
    "infineon",
    "investments",
    "iron",
    "israel",
    "israeli",
    "japan",
    "klasse",
    "markets",
    "million",
    "ministry",
    "news",
    "newsflow",
    "partners",
    "performance",
    "price",
    "production",
    "purchased",
    "raises",
    "results",
    "shares",
    "stock",
    "system",
    "systems",
    "target",
    "trading",
    "urges",
];

/// Multi-word headline fragments that often match the full-name regex but are not people.
static GDELT_NON_PERSON_NAME_PHRASES: &[&str] = &[
    "human history",
    "is on fire",
    "chips will show up",
    "not expecting any press",
    "investment might be the",
    "last before",
    "on the rise",
    "under pressure",
    "in focus",
    "at risk",
];

/// Function words that should not appear inside a capitalized GDELT person candidate.
static GDELT_NON_PERSON_CONNECTOR_WORDS: &[&str] = &[
    "is", "are", "was", "were", "be", "been", "being", "on", "in", "at", "for", "with", "from",
    "into", "onto", "under", "over", "up", "down", "off", "out", "to", "of", "by", "as", "will",
    "would", "can", "could", "may", "might", "should", "shall", "must", "not", "no", "any", "the",
    "a", "an", "before", "after",
];

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal seed POI info required by the expansion engine.
#[derive(Debug, Clone)]
pub struct SeedPoi {
    pub id: String,
    pub name: String,
    pub organization: String,
    /// Optional primary website of the organisation (used for leadership scrape).
    pub org_website: Option<String>,
    pub region: Option<String>,
    pub role_family: String,
}

/// Candidate POI discovered from the network of an existing seed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPoi {
    /// Full name as extracted from source.
    pub name: String,
    /// Inferred job title.
    pub inferred_role: Option<String>,
    /// Inferred organisation name.
    pub inferred_org: Option<String>,
    /// URL where this person was discovered.
    pub source_url: String,
    /// How they were found: `"org_leadership"`, `"gdelt_co_mention"`,
    /// `"opencorporates_board"`, `"conference_speaker"`, `"citation_coauthor"`,
    /// `"linkedin_related"`.
    pub discovery_method: String,
    /// Email address if found alongside the name.
    pub contact_email: Option<String>,
    /// LinkedIn profile URL if found.
    pub contact_linkedin: Option<String>,
    /// Confidence the extracted name is a real person [0, 1].
    pub confidence: f32,
    /// ID of the seed POI that led to this discovery.
    pub seed_person_id: String,
    /// UTC timestamp of discovery.
    pub ts_discovered: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Engine
// ─────────────────────────────────────────────────────────────────────────────

/// Multi-strategy POI discovery engine.
pub struct PoiExpansionEngine {
    client: reqwest::Client,
    proxy_rotator: Option<Arc<Mutex<ProxyRotator>>>,
}

impl PoiExpansionEngine {
    /// Build the engine.  `proxy_rotator` is optional; if `None` all requests
    /// use the default network interface.
    pub fn new(proxy_rotator: Option<Arc<Mutex<ProxyRotator>>>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Mozilla/5.0 AppleWebKit/537.36")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .context("PoiExpansionEngine: build client")?;
        Ok(Self {
            client,
            proxy_rotator,
        })
    }

    /// Run all expansion strategies against a slice of seed POIs and return
    /// deduplicated `DiscoveredPoi` candidates, capped at `limit`.
    ///
    /// Duplicate detection is name-based (normalised lowercase).
    pub async fn expand_from_seeds(
        &self,
        seeds: &[SeedPoi],
        known_names: &HashSet<String>,
        limit: usize,
    ) -> Vec<DiscoveredPoi> {
        let mut all: Vec<DiscoveredPoi> = vec![];
        let mut seen_names: HashSet<String> =
            known_names.iter().map(|n| normalise_name(n)).collect();

        for seed in seeds {
            if all.len() >= limit {
                break;
            }
            info!(
                seed_name = %seed.name,
                seed_id = %seed.id,
                "poi_expansion: processing seed"
            );

            // Strategy 1 — Org leadership pages
            let from_org = self.expand_org_leadership(seed).await;
            for d in from_org {
                let key = normalise_name(&d.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(d);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            // Strategy 2 — GDELT co-mention
            let from_gdelt = self.expand_gdelt_co_mentions(seed).await;
            for d in from_gdelt {
                let key = normalise_name(&d.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(d);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            // Strategy 3 — OpenCorporates board
            let from_oc = self.expand_opencorporates_board(seed).await;
            for d in from_oc {
                let key = normalise_name(&d.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(d);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            // Strategy 4 — Conference / speaker directories
            let from_conf = self.expand_conference_speakers(seed).await;
            for d in from_conf {
                let key = d.name.to_lowercase();
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(d);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            // Strategy 5 — Semantic Scholar / academic co-authors
            let from_scholar = self.expand_semantic_scholar(seed).await;
            for d in from_scholar {
                let key = d.name.to_lowercase();
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(d);
                    if all.len() >= limit {
                        break;
                    }
                }
            }
        }

        info!(discovered = all.len(), "poi_expansion: complete");
        all
    }

    // ─── Strategy 1: Org leadership ──────────────────────────────────────

    async fn expand_org_leadership(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        let website = match seed.org_website.as_deref() {
            Some(w) if !w.is_empty() => w.to_string(),
            _ => return vec![],
        };
        let base = website.trim_end_matches('/');
        let mut results = vec![];
        let mut candidate_urls = default_leadership_urls(base);

        if let Ok(homepage_html) = self.get_text(base).await {
            let homepage_candidates = extract_candidate_leadership_urls(base, &homepage_html);
            if !homepage_candidates.is_empty() {
                let mut seen = HashSet::new();
                candidate_urls = homepage_candidates
                    .into_iter()
                    .chain(candidate_urls.into_iter())
                    .filter(|url| seen.insert(url.to_ascii_lowercase()))
                    .collect();
            }
        }

        for url in &candidate_urls {
            match self.get_text(&url).await {
                Ok(html) => {
                    let names = extract_names_from_leadership_html(&html, &seed.organization);
                    for (name, role, email, linkedin) in names {
                        results.push(DiscoveredPoi {
                            name,
                            inferred_role: role,
                            inferred_org: Some(seed.organization.clone()),
                            source_url: url.clone(),
                            discovery_method: "org_leadership".to_string(),
                            contact_email: email,
                            contact_linkedin: linkedin,
                            confidence: 0.75,
                            seed_person_id: seed.id.clone(),
                            ts_discovered: Utc::now().timestamp(),
                        });
                    }
                    if !results.is_empty() {
                        break; // Found a working page, stop trying other paths.
                    }
                }
                Err(e) => debug!("leadership_scrape {url}: {e:#}"),
            }
        }
        results
    }

    // ─── Strategy 2: GDELT co-mentions ───────────────────────────────────

    async fn expand_gdelt_co_mentions(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        // GDELT 2.0 EventSearch — returns JSON with actor names.
        let query = format!("\"{}\" \"{}\"", seed.name, seed.organization);
        let url = format!(
            "https://api.gdeltproject.org/api/v2/doc/doc?query={}&mode=artlist&maxrecords=25&format=json",
            urlencoding::encode(&query)
        );
        let json_str = match self.get_text(&url).await {
            Ok(s) => s,
            Err(e) => {
                debug!("gdelt_co_mention {}: {e:#}", seed.name);
                return vec![];
            }
        };
        parse_gdelt_response(&json_str, seed)
    }

    // ─── Strategy 3: OpenCorporates board/officers ────────────────────────

    async fn expand_opencorporates_board(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        // Search for the company and get its officer list.
        let url = format!(
            "https://api.opencorporates.com/v0.4/companies/search?q={}&inactive=false&format=json",
            urlencoding::encode(&seed.organization)
        );
        let json_str = match self.get_text(&url).await {
            Ok(s) => s,
            Err(e) => {
                debug!("opencorporates_search {}: {e:#}", seed.organization);
                return vec![];
            }
        };
        parse_opencorporates_officers(&json_str, seed)
    }

    // ─── Strategy 4: Conference speakers ─────────────────────────────────

    async fn expand_conference_speakers(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        // Query Lanyrd / Sessionize / Eventbrite speaker search (public JSON endpoints).
        let queries = build_conference_queries(&seed.name, &seed.organization, &seed.role_family);
        let mut results = vec![];

        for url in queries {
            match self.get_text(&url).await {
                Ok(html) => {
                    let names = extract_speakers_from_html(&html);
                    for (name, role) in names {
                        results.push(DiscoveredPoi {
                            name,
                            inferred_role: role,
                            inferred_org: None,
                            source_url: url.clone(),
                            discovery_method: "conference_speaker".to_string(),
                            contact_email: None,
                            contact_linkedin: None,
                            confidence: 0.55,
                            seed_person_id: seed.id.clone(),
                            ts_discovered: Utc::now().timestamp(),
                        });
                    }
                }
                Err(e) => debug!("conference_speakers {}: {e:#}", seed.name),
            }
        }
        results
    }

    // ─── Strategy 5: Semantic Scholar co-authors ──────────────────────────

    async fn expand_semantic_scholar(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        let url = format!(
            "https://api.semanticscholar.org/graph/v1/author/search?query={}&fields=name,affiliations,paperCount&limit=5",
            urlencoding::encode(&seed.name)
        );
        let json_str = match self.get_text(&url).await {
            Ok(s) => s,
            Err(e) => {
                debug!("semantic_scholar {}: {e:#}", seed.name);
                return vec![];
            }
        };
        parse_semantic_scholar_coauthors(&json_str, seed)
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    async fn get_text(&self, url: &str) -> Result<String> {
        let proxy = self.proxy_rotator.as_ref().and_then(|rotator| {
            let mut guard = rotator.lock().ok()?;
            guard.get_next()
        });

        let client = if let Some(proxy_url) = proxy.as_ref() {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 AppleWebKit/537.36")
                .redirect(reqwest::redirect::Policy::limited(5))
                .proxy(reqwest::Proxy::all(proxy_url).context("invalid proxy url")?)
                .build()
                .context("PoiExpansionEngine: build proxied client")?
        } else {
            self.client.clone()
        };

        let mut req = client.get(url);
        req = req.headers(random_headers(None));
        let resp = req.send().await.context("GET failed")?;
        let status = resp.status();

        if let (Some(rotator), Some(proxy_url)) = (self.proxy_rotator.as_ref(), proxy.as_ref()) {
            if let Ok(mut guard) = rotator.lock() {
                if status.is_success() {
                    guard.report_success(proxy_url);
                } else {
                    guard.report_failure(proxy_url);
                }
            }
        }

        if !status.is_success() {
            return Err(anyhow::anyhow!("HTTP {}", status));
        }
        let text = resp.text().await.context("read body")?;
        Ok(text)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Regex for names inside heading or strong tags (structured leadership cards).
static RE_HEADING_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?:h[1-6]|strong|b|span[^>]*class=[^>]*(?:name|title|person|member|leader|exec)[^>]*)(?:\s[^>]*)?>([^<]{4,60})</(?:h[1-6]|strong|b|span)>")
        .unwrap()
});

/// Extract `(name, role, email, linkedin)` tuples from a company leadership HTML page.
/// Prefers names found inside heading/bold/span.name tags for higher precision.
fn extract_names_from_leadership_html(
    html: &str,
    org: &str,
) -> Vec<(String, Option<String>, Option<String>, Option<String>)> {
    let mut results = vec![];
    let mut seen = HashSet::new();

    // Phase 1: Extract names from structured HTML elements (higher confidence).
    for cap in RE_HEADING_NAME.captures_iter(html) {
        let raw = RE_HTML_TAGS.replace_all(&cap[1], " ");
        let raw = raw.trim();
        if let Some(name_cap) = RE_FULL_NAME.captures(raw) {
            let name = name_cap[1].to_string();
            if name.len() < 5 || seen.contains(&name) {
                continue;
            }
            if !is_plausible_org_leadership_candidate(&name, org) {
                continue;
            }
            // Find role, email, linkedin in surrounding HTML.
            let start = cap.get(0).map(|m| m.start()).unwrap_or(0);
            let vicinity = &html[start.saturating_sub(200)..std::cmp::min(start + 500, html.len())];
            let role = RE_TITLE_AT.captures(vicinity).map(|c| c[1].to_string());
            let role = sanitize_leadership_role(infer_target_role(vicinity).or(role));
            if role.is_none() {
                continue;
            }
            let email = RE_EMAIL.find(vicinity).map(|m| m.as_str().to_lowercase());
            let linkedin = RE_LINKEDIN
                .captures(vicinity)
                .map(|c| format!("https://linkedin.com/in/{}", &c[1]));

            seen.insert(name.clone());
            results.push((name, role, email, linkedin));
        }
    }

    // Phase 2: Fallback — scan stripped text for name patterns near title keywords.
    // Only used if Phase 1 found nothing, to avoid duplicating noisy matches.
    if results.is_empty() {
        let stripped = RE_HTML_TAGS.replace_all(html, " ");
        let lines: Vec<&str> = stripped.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if let Some(cap) = RE_FULL_NAME.captures(trimmed) {
                let name = cap[1].to_string();
                if name.len() < 5 || seen.contains(&name) {
                    continue;
                }
                if !is_plausible_org_leadership_candidate(&name, org) {
                    continue;
                }
                // Require a title keyword within 3 lines — avoids matching random
                // capitalized phrases in navigation or product sections.
                let context: String = lines[i..std::cmp::min(i + 4, lines.len())].join(" ");
                let role = RE_TITLE_AT.captures(&context).map(|c| c[1].to_string());
                if role.is_none() {
                    continue; // Skip names not followed by a recognizable title
                }
                let role = sanitize_leadership_role(infer_target_role(&context).or(role));
                if role.is_none() {
                    continue;
                }

                let vicinity: String =
                    lines[i.saturating_sub(2)..std::cmp::min(i + 6, lines.len())].join(" ");
                let email = RE_EMAIL.find(&vicinity).map(|m| m.as_str().to_lowercase());
                let linkedin = RE_LINKEDIN
                    .captures(&vicinity)
                    .map(|c| format!("https://linkedin.com/in/{}", &c[1]));

                seen.insert(name.clone());
                results.push((name, role, email, linkedin));
            }
        }
    }
    results
}

/// Parse GDELT 2.0 artlist JSON response and extract co-mentioned person names.
fn parse_gdelt_response(json: &str, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
    // GDELT artlist format: { "articles": [ { "title": "...", "url": "...", "seendate": "...", ... } ] }
    let seed_name_lower = seed.name.to_lowercase();
    let mut results = vec![];
    let mut seen = HashSet::new();

    // Quick regex scan for article titles — extract any name NOT the seed.
    let title_re = Regex::new(r#""title"\s*:\s*"([^"]{10,200})""#).unwrap();
    let url_re = Regex::new(r#""url"\s*:\s*"([^"]+)""#).unwrap();

    let titles: Vec<&str> = title_re
        .captures_iter(json)
        .filter_map(|c| Some(c.get(1)?.as_str()))
        .collect();
    let urls: Vec<&str> = url_re
        .captures_iter(json)
        .filter_map(|c| Some(c.get(1)?.as_str()))
        .collect();

    for (i, title) in titles.iter().enumerate() {
        if !is_plausible_gdelt_title(title, &seed_name_lower) {
            continue;
        }
        for cap in RE_FULL_NAME.captures_iter(title) {
            let name = cap[1].to_string();
            if name.to_lowercase() == seed_name_lower || seen.contains(&name) || name.len() < 5 {
                continue;
            }
            if !looks_like_person_name(&name) {
                continue;
            }
            if !is_plausible_gdelt_person_candidate(title, &name, &seed_name_lower) {
                continue;
            }
            seen.insert(name.clone());
            let source_url = urls.get(i).map(|u| u.to_string()).unwrap_or_default();
            results.push(DiscoveredPoi {
                name,
                inferred_role: None,
                inferred_org: None,
                source_url,
                discovery_method: "gdelt_co_mention".to_string(),
                contact_email: None,
                contact_linkedin: None,
                confidence: GDELT_BASE_CONFIDENCE,
                seed_person_id: seed.id.clone(),
                ts_discovered: Utc::now().timestamp(),
            });
        }
    }
    results
}

fn is_plausible_gdelt_title(title: &str, seed_name_lower: &str) -> bool {
    let title_lower = title.to_lowercase();
    title_lower.contains(seed_name_lower)
        && !GDELT_NON_PERSON_TITLE_PHRASES
            .iter()
            .any(|phrase| title_lower.contains(phrase))
}

fn is_plausible_gdelt_person_candidate(title: &str, name: &str, seed_name_lower: &str) -> bool {
    let title_lower = title.to_lowercase();
    let name_lower = name.to_lowercase();
    if !title_lower.contains(seed_name_lower) {
        return false;
    }
    if name_lower.contains(seed_name_lower) {
        return false;
    }
    if GDELT_NON_PERSON_NAME_PHRASES
        .iter()
        .any(|phrase| name_lower == *phrase)
    {
        return false;
    }
    if name_lower
        .split_whitespace()
        .any(|word| GDELT_NON_PERSON_CONNECTOR_WORDS.contains(&word))
    {
        return false;
    }
    !name_lower
        .split_whitespace()
        .any(|word| GDELT_NON_PERSON_WORDS.contains(&word))
}

fn default_leadership_urls(base: &str) -> Vec<String> {
    [
        "/leadership",
        "/team",
        "/management",
        "/about/team",
        "/about/leadership",
        "/about/management",
        "/about/executives",
        "/company/leadership",
        "/who-we-are/leadership",
        "/leadership-team",
        "/executives",
        "/about-us/team",
        "/about-us/leadership",
        "/company/team",
        "/board",
    ]
    .iter()
    .map(|path| format!("{base}{path}"))
    .collect()
}

fn leadership_path_matches(path: &str) -> bool {
    let normalized = path.trim_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let exact_segments = [
        "leadership",
        "team",
        "management",
        "executive",
        "executives",
        "board",
        "about",
        "company",
        "who-we-are",
        "about-us",
    ];

    let joined = segments.join("/");
    joined.contains("leadership")
        || segments
            .iter()
            .any(|segment| exact_segments.contains(segment))
}

fn extract_candidate_leadership_urls(base: &str, html: &str) -> Vec<String> {
    let Ok(base_url) = Url::parse(base) else {
        return default_leadership_urls(base);
    };
    const MAX_DISCOVERED_LEADERSHIP_URLS: usize = 12;

    let mut urls = Vec::new();
    let mut seen = HashSet::new();
    for default_url in default_leadership_urls(base) {
        if seen.insert(default_url.to_ascii_lowercase()) {
            urls.push(default_url);
        }
    }

    for cap in RE_HREF.captures_iter(html) {
        let href = cap[1].trim();
        let href_lower = href.to_ascii_lowercase();
        if href_lower.starts_with("mailto:") || href_lower.starts_with("tel:") {
            continue;
        }

        let Ok(joined) = base_url.join(href) else {
            continue;
        };
        if joined.scheme() != "http" && joined.scheme() != "https" {
            continue;
        }
        if joined.host_str() != base_url.host_str() {
            continue;
        }
        if joined.query().is_some() {
            continue;
        }
        if !leadership_path_matches(joined.path()) {
            continue;
        }

        let normalized = joined.to_string();
        if seen.insert(normalized.to_ascii_lowercase()) {
            urls.push(normalized);
            if urls.len() >= MAX_DISCOVERED_LEADERSHIP_URLS {
                break;
            }
        }
    }

    urls
}

/// Parse OpenCorporates search JSON, drill into officer lists.
fn parse_opencorporates_officers(json: &str, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
    // Extract officer names from the nested JSON without a full parser.
    // "name":"Jane Smith","position":"director"
    let officer_re =
        Regex::new(r#""name"\s*:\s*"([^"]{3,80})"\s*,\s*"position"\s*:\s*"([^"]{3,50})""#).unwrap();

    let seed_name_lower = seed.name.to_lowercase();
    let mut results = vec![];
    let mut seen = HashSet::new();

    for cap in officer_re.captures_iter(json) {
        let name = cap[1].trim().to_string();
        let position = cap[2].trim().to_string();
        if !is_target_decision_role(&position) {
            continue;
        }
        if name.to_lowercase() == seed_name_lower || seen.contains(&name) {
            continue;
        }
        if !looks_like_person_name(&name) {
            continue;
        }
        seen.insert(name.clone());
        results.push(DiscoveredPoi {
            name,
            inferred_role: Some(position),
            inferred_org: Some(seed.organization.clone()),
            source_url: format!(
                "https://opencorporates.com/companies/search?q={}",
                urlencoding::encode(&seed.organization)
            ),
            discovery_method: "opencorporates_board".to_string(),
            contact_email: None,
            contact_linkedin: None,
            confidence: 0.65,
            seed_person_id: seed.id.clone(),
            ts_discovered: Utc::now().timestamp(),
        });
    }
    results
}

/// Build conference speaker query URLs for a given seed.
fn build_conference_queries(name: &str, _org: &str, _role_family: &str) -> Vec<String> {
    // Sessionize public speaker search (CFP aggregator).
    let sessions_url = format!(
        "https://sessionize.com/api/v2/search/speakers?query={}",
        urlencoding::encode(name)
    );
    // TED talks speaker page.
    let ted_url = format!(
        "https://www.ted.com/speakers?q={}",
        urlencoding::encode(name)
    );
    // Conf.tube (open-source conference talks).
    let conftube_url = format!(
        "https://conf.tube/api/v1/search/videos?search={}&filter=local",
        urlencoding::encode(name)
    );
    vec![sessions_url, ted_url, conftube_url]
}

/// Extract `(name, role)` speaker pairs from conference HTML/JSON pages.
///
/// Only returns names that pass `looks_like_person_name()` validation to filter
/// out website navigation, topic labels, error messages, and other non-person text.
fn extract_speakers_from_html(html: &str) -> Vec<(String, Option<String>)> {
    let stripped = RE_HTML_TAGS.replace_all(html, " ");
    let mut seen = HashSet::new();
    let mut results = vec![];
    for cap in RE_FULL_NAME.captures_iter(&stripped) {
        let name = cap[1].to_string();
        if name.len() < 5 || seen.contains(&name) {
            continue;
        }
        // Critical: validate this actually looks like a person name.
        // Without this, we scrape nav elements, topic tags, error messages, etc.
        if !looks_like_person_name(&name) {
            debug!(rejected_name = %name, "extract_speakers: rejected non-person name");
            continue;
        }
        seen.insert(name.clone());
        results.push((name, None));
    }
    results
}

/// Parse Semantic Scholar author search response and return co-author candidates.
fn parse_semantic_scholar_coauthors(json: &str, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
    // Response: { "data": [ { "authorId": "...", "name": "...", "affiliations": ["..."] } ] }
    let author_re = Regex::new(r#""name"\s*:\s*"([^"]{3,80})""#).unwrap();
    let affil_re = Regex::new(r#""affiliations"\s*:\s*\[([^\]]*)\]"#).unwrap();

    let seed_name_lower = seed.name.to_lowercase();
    let mut results = vec![];
    let mut seen = HashSet::new();

    let affiliations: Vec<String> = affil_re
        .captures_iter(json)
        .flat_map(|c| {
            c[1].split(',')
                .filter_map(|s| {
                    let s = s.trim().trim_matches('"');
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    for (i, cap) in author_re.captures_iter(json).enumerate() {
        let name = cap[1].trim().to_string();
        if name.to_lowercase() == seed_name_lower || seen.contains(&name) {
            continue;
        }
        if !looks_like_person_name(&name) {
            continue;
        }
        seen.insert(name.clone());
        let affil = affiliations.get(i).cloned();
        results.push(DiscoveredPoi {
            name,
            inferred_role: Some("Researcher".to_string()),
            inferred_org: affil,
            source_url: format!(
                "https://api.semanticscholar.org/graph/v1/author/search?query={}",
                urlencoding::encode(&seed.name)
            ),
            discovery_method: "citation_coauthor".to_string(),
            contact_email: None,
            contact_linkedin: None,
            confidence: 0.50,
            seed_person_id: seed.id.clone(),
            ts_discovered: Utc::now().timestamp(),
        });
    }
    results
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Normalise a full name for deduplication (lowercase, collapse whitespace).
fn normalise_name(name: &str) -> String {
    name.split_whitespace()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_org_tokens(org: &str) -> HashSet<String> {
    org.split(|c: char| !c.is_alphabetic())
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn is_plausible_org_leadership_candidate(name: &str, org: &str) -> bool {
    if !looks_like_person_name(name) {
        return false;
    }

    let candidate_words: Vec<String> = name
        .split_whitespace()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    if candidate_words
        .iter()
        .any(|word| ORG_ENTITY_WORDS.contains(&word.as_str()))
    {
        return false;
    }

    let org_tokens = normalize_org_tokens(org);
    if org_tokens.is_empty() {
        return true;
    }

    let overlapping = candidate_words
        .iter()
        .filter(|word| org_tokens.contains(word.as_str()))
        .count();

    overlapping < 2 && overlapping < candidate_words.len()
}

fn sanitize_leadership_role(role: Option<String>) -> Option<String> {
    let role = role?.trim().to_string();
    if role.is_empty() {
        return None;
    }

    let lower = role.to_ascii_lowercase();
    let junk_roles = [
        "postgres",
        "postgresql",
        "mysql",
        "mariadb",
        "mongodb",
        "redis",
        "nginx",
        "apache",
    ];
    if junk_roles.contains(&lower.as_str()) {
        return None;
    }

    is_target_decision_role(&role).then_some(role)
}

fn infer_target_role(context: &str) -> Option<String> {
    let lc = context.to_ascii_lowercase();
    let patterns: [(&str, &str); 25] = [
        ("deputy director general", "Deputy Director General"),
        ("director general", "Director General"),
        ("deputy director", "Deputy Director"),
        ("department director", "Department Director"),
        ("department head", "Department Head"),
        ("head of procurement", "Head of Procurement"),
        ("procurement director", "Procurement Director"),
        ("procurement manager", "Procurement Manager"),
        ("purchasing manager", "Purchasing Manager"),
        ("sourcing manager", "Sourcing Manager"),
        ("category manager", "Category Manager"),
        ("commodity manager", "Commodity Manager"),
        ("head of operations", "Head of Operations"),
        ("operations director", "Operations Director"),
        ("operations manager", "Operations Manager"),
        ("program director", "Program Director"),
        ("program manager", "Program Manager"),
        ("policy director", "Policy Director"),
        ("policy manager", "Policy Manager"),
        ("licensing director", "Licensing Director"),
        ("regulatory affairs", "Regulatory Affairs"),
        ("compliance manager", "Compliance Manager"),
        ("quality manager", "Quality Manager"),
        ("engineering manager", "Engineering Manager"),
        ("supply chain manager", "Supply Chain Manager"),
    ];
    for (needle, role) in patterns {
        if lc.contains(needle) {
            return Some(role.to_string());
        }
    }
    None
}

fn is_target_decision_role(role: &str) -> bool {
    let r = role.to_ascii_lowercase();
    let junk = [
        "investor relations",
        "media",
        "press",
        "communications",
        "marketing",
        "sales",
        "business development",
        "customer service",
        "support",
        "assistant",
        "coordinator",
        "specialist",
        "analyst",
        "recruiter",
        "talent acquisition",
        "human resources",
        "office manager",
        "administrator",
        "receptionist",
    ];
    if junk.iter().any(|k| r.contains(k)) {
        return false;
    }

    let executive = [
        "ceo",
        "chief executive",
        "chief financial",
        "chief operating",
        "chief technology",
        "chief information",
        "chief procurement",
        "chief commercial",
        "chief strategy",
        "chief security",
        "chief legal",
        "chairman",
        "chairwoman",
        "chair",
        "board",
        "president",
        "founder",
        "co-founder",
        "owner",
        "managing partner",
        "managing director",
        "executive vice president",
        "senior vice president",
        "vice president",
        "vp",
        "general manager",
        "board member",
        "supervisory board",
    ];
    if executive.iter().any(|k| r.contains(k)) {
        return true;
    }

    let government = [
        "director general",
        "deputy director general",
        "deputy director",
        "department director",
        "department head",
        "head of procurement",
        "procurement director",
        "procurement manager",
        "policy director",
        "policy manager",
        "program director",
        "program manager",
        "licensing director",
        "regulatory affairs director",
        "regulatory affairs manager",
        "compliance director",
        "compliance manager",
        "acquisition director",
        "acquisition manager",
        "minister",
        "secretary of state",
        "governor",
        "ambassador",
        "prime minister",
        "senator",
        "mayor",
        "admiral",
        "general",
    ];
    if government.iter().any(|k| r.contains(k)) {
        return true;
    }

    let functional_scope = [
        "procurement",
        "purchasing",
        "sourcing",
        "buyer",
        "category",
        "commodity",
        "operations",
        "supply chain",
        "compliance",
        "regulatory",
        "licensing",
        "quality",
        "engineering",
        "contracts",
        "tender",
        "acquisition",
    ];
    let seniority = [
        "head of",
        "director",
        "manager",
        "vice president",
        "vp",
        "chief",
        "lead",
        "officer",
        "general manager",
    ];

    functional_scope.iter().any(|k| r.contains(k))
        && seniority.iter().any(|k| r.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdelt_candidates_meet_worker_minimum_confidence() {
        let seed = SeedPoi {
            id: "seed-1".to_string(),
            name: "Jane Smith".to_string(),
            organization: "Acme Corp".to_string(),
            org_website: None,
            region: None,
            role_family: "procurement".to_string(),
        };

        let json = r#"{
            "articles": [
                {
                    "title": "Jane Smith and John Carter discuss sourcing plans",
                    "url": "https://example.com/story"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].name, "John Carter");
        assert_eq!(discoveries[0].discovery_method, "gdelt_co_mention");
        assert!(discoveries[0].confidence >= GDELT_BASE_CONFIDENCE);
    }

    #[test]
    fn gdelt_rejects_titles_without_seed_name() {
        let seed = SeedPoi {
            id: "seed-1".to_string(),
            name: "Jane Smith".to_string(),
            organization: "Acme Corp".to_string(),
            org_website: None,
            region: None,
            role_family: "procurement".to_string(),
        };

        let json = r#"{
            "articles": [
                {
                    "title": "John Carter discusses sourcing plans",
                    "url": "https://example.com/story"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert!(discoveries.is_empty());
    }

    #[test]
    fn gdelt_rejects_headline_fragments_and_organization_phrases() {
        let seed = SeedPoi {
            id: "seed-1".to_string(),
            name: "Jane Smith".to_string(),
            organization: "Acme Corp".to_string(),
            org_website: None,
            region: None,
            role_family: "procurement".to_string(),
        };

        let json = r#"{
            "articles": [
                {
                    "title": "Jane Smith reviews Financial Results and market updates",
                    "url": "https://example.com/results"
                },
                {
                    "title": "Jane Smith meets Glen Capital Partners on sourcing strategy",
                    "url": "https://example.com/partners"
                },
                {
                    "title": "Jane Smith and John Carter discuss sourcing plans",
                    "url": "https://example.com/people"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].name, "John Carter");
    }

    #[test]
    fn gdelt_rejects_capitalized_headline_fragments_like_is_on_fire() {
        let seed = SeedPoi {
            id: "seed-1".to_string(),
            name: "Steve Sanghi".to_string(),
            organization: "Microchip Technology".to_string(),
            org_website: None,
            region: None,
            role_family: "procurement".to_string(),
        };

        let json = r#"{
            "articles": [
                {
                    "title": "Steve Sanghi Is On Fire after semiconductor rally",
                    "url": "https://example.com/fire"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert!(discoveries.is_empty());
    }

    #[test]
    fn target_role_filter_rejects_generic_contact_roles() {
        assert!(!is_target_decision_role("Investor Relations Contact"));
        assert!(!is_target_decision_role("Human Resources Specialist"));
        assert!(!is_target_decision_role("Marketing Coordinator"));
    }

    #[test]
    fn person_name_validation_rejects_temporal_phrase_names() {
        assert!(!looks_like_person_name("Through December"));
        assert!(!is_plausible_org_leadership_candidate(
            "Through December",
            "NXP Semiconductors"
        ));
    }

    #[test]
    fn leadership_role_sanitizer_rejects_infrastructure_terms() {
        assert_eq!(sanitize_leadership_role(Some("postgres".to_string())), None);
        assert_eq!(
            sanitize_leadership_role(Some("Vice President".to_string())),
            Some("Vice President".to_string())
        );
    }

    #[test]
    fn target_role_filter_keeps_real_decision_makers() {
        assert!(is_target_decision_role("Procurement Manager"));
        assert!(is_target_decision_role("VP Supply Chain"));
        assert!(is_target_decision_role("Chief Operating Officer"));
        assert!(is_target_decision_role("Director of Engineering"));
    }

    #[test]
    fn gdelt_rejects_recent_sentence_fragments_seen_in_production() {
        let seed = SeedPoi {
            id: "seed-1".to_string(),
            name: "Steve Sanghi".to_string(),
            organization: "Microchip Technology".to_string(),
            org_website: None,
            region: None,
            role_family: "procurement".to_string(),
        };

        let json = r#"{
            "articles": [
                {
                    "title": "Steve Sanghi reflects on Human History and industrial policy",
                    "url": "https://example.com/history"
                },
                {
                    "title": "Steve Sanghi says Chips Will Show Up after supply crunch",
                    "url": "https://example.com/chips"
                },
                {
                    "title": "Steve Sanghi is Not Expecting Any Press during launch week",
                    "url": "https://example.com/press"
                },
                {
                    "title": "Steve Sanghi warns Investment Might Be The next bottleneck",
                    "url": "https://example.com/investment"
                },
                {
                    "title": "Steve Sanghi says Last Before marks the final pre-brief",
                    "url": "https://example.com/before"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert!(discoveries.is_empty());
    }

    #[test]
    fn leadership_extraction_rejects_org_labels_but_keeps_people() {
        let html = r#"
            <section>
                <h2>Glen Capital Partners</h2>
                <p>Investment platform overview</p>
                <h3>Jane Smith</h3>
                <p>Procurement Director</p>
            </section>
        "#;

        let people = extract_names_from_leadership_html(html, "Glen Capital Partners");
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].0, "Jane Smith");
    }

    #[test]
    fn homepage_link_discovery_finds_same_host_leadership_pages() {
        let html = r#"
            <html>
                <body>
                    <a href="/about/leadership">Leadership</a>
                    <a href="https://example.com/company/team">Team</a>
                    <a href="https://example.com/products/steam-controller">Steam</a>
                    <a href="https://external.example.org/leadership">External</a>
                    <a href="/products">Products</a>
                </body>
            </html>
        "#;

        let urls = extract_candidate_leadership_urls("https://example.com", html);

        assert!(urls
            .iter()
            .any(|url| url == "https://example.com/about/leadership"));
        assert!(urls
            .iter()
            .any(|url| url == "https://example.com/company/team"));
        assert!(!urls.iter().any(|url| url.contains("steam-controller")));
        assert!(!urls.iter().any(|url| url.contains("external.example.org")));
    }
}

/// Reject obvious non-person strings (all-caps acronyms, single word, numbers,
/// common website/navigation phrases, topic labels, error messages, etc.).
fn looks_like_person_name(s: &str) -> bool {
    if !unicode_person_name(s) {
        return false;
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() > 5 {
        return false;
    }
    // Reject names where every word is very short (likely acronyms or labels).
    let total_chars: usize = words.iter().map(|w| w.len()).sum();
    if total_chars < 6 {
        return false;
    }
    // Blocklist of common non-person phrases that match the name regex.
    let lower = s.to_lowercase();
    if NON_PERSON_PHRASES.iter().any(|phrase| lower == *phrase) {
        return false;
    }
    if looks_like_temporal_phrase(&words) {
        return false;
    }
    // Reject if ANY word is a common non-person keyword.
    if words
        .iter()
        .any(|w| NON_PERSON_WORDS.contains(&w.to_lowercase().as_str()))
    {
        return false;
    }
    true
}

fn looks_like_temporal_phrase(words: &[&str]) -> bool {
    if words.len() < 2 {
        return false;
    }

    let first = words[0].to_ascii_lowercase();
    if !TEMPORAL_PREFIX_WORDS.contains(&first.as_str()) {
        return false;
    }

    words[1..]
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .any(|word| MONTH_WORDS.contains(&word.as_str()))
}

/// Common capitalized phrases that are NOT person names.
static NON_PERSON_PHRASES: &[&str] = &[
    // Website navigation / UI elements
    "session entity",
    "menu main",
    "public speaking",
    "health care",
    "too many requests",
    "unhandled promise rejection",
    "social change",
    "personal growth",
    "climate change",
    "mental health",
    "urban planning",
    "global issues",
    "medical research",
    "product design",
    "cognitive science",
    "open source",
    "social media",
    "alternative energy",
    "interface design",
    "mission blue",
    "data commons",
    "big bang",
    "dark matter",
    "string theory",
    "solar system",
    "human origins",
    "human rights",
    "body language",
    "gender equality",
    "nuclear energy",
    "world cultures",
    "ancient world",
    "foreign policy",
    "extreme sports",
    "disaster relief",
    "global development",
    "medical imaging",
    "augmented reality",
    "behavioral psychology",
    "biological warfare",
    "computer programming",
    "criminal justice",
    "developmental science",
    "risk taking",
    "artificial intelligence",
    "autism spectrum disorder",
    "computer science",
    "crisis management",
    "data protection",
    "digital media",
    "live music",
    "behavioral economics",
    "spoken word",
    "decision making",
    "public spaces",
    "extraterrestrial life",
    "macarthur grant",
    "goal setting",
    "women in business",
    "charter for compassion",
    "new york",
    "performance art",
    "industrial design",
    "online video",
    "big problems",
    "audacious projects",
    "third world",
    "middle east",
    "united states",
    "session replay",
    "new relic warning",
    "through december",
    // Generic labels
    "read more",
    "learn more",
    "sign up",
    "log in",
    "sign in",
    "watch now",
    "view all",
    "show more",
    "load more",
    "click here",
    "find out",
    "get started",
    "contact us",
    "about us",
    "terms service",
    "privacy policy",
    "cookie policy",
    "copyright notice",
];

/// Individual words that strongly indicate a non-person phrase.
static NON_PERSON_WORDS: &[&str] = &[
    // Website UI / navigation
    "menu",
    "session",
    "login",
    "logout",
    "signup",
    "subscribe",
    "unsubscribe",
    "download",
    "upload",
    "install",
    "configure",
    "settings",
    "dashboard",
    "analytics",
    "requests",
    "rejection",
    "error",
    "warning",
    "undefined",
    "null",
    "nan",
    "true",
    "false",
    "cookie",
    "cookies",
    "copyright",
    "newsletter",
    "podcast",
    "webinar",
    "blog",
    "search",
    "filter",
    "translate",
    "initiatives",
    "membership",
    "courses",
    "speakers",
    "participate",
    "nominate",
    "recommend",
    "discover",
    "browse",
    "fellows",
    "updates",
    "topics",
    "explore",
    "inspiration",
    "clearance",
    "mixpanel",
    "relic",
    "replay",
    "bcg",
    // Font / CSS / tech
    "emoji",
    "sans",
    "serif",
    "mono",
    "arial",
    "helvetica",
    "verdana",
    "noto",
    "roboto",
    "inter",
    "lato",
    "montserrat",
    // Supply chain / industry terms
    "supply",
    "chain",
    "logistics",
    "manufacturing",
    "semiconductor",
    "semiconductors",
    "automotive",
    "aerospace",
    "defense",
    "electronics",
    "embedded",
    "modular",
    "busway",
    "switchgear",
    "capacitors",
    "interconnect",
    "mechanicals",
    "circuits",
    "cooling",
    "modules",
    "fulfillment",
    "aftermarket",
    "procurement",
    "warehouse",
    // Product / business terms
    "products",
    "solutions",
    "services",
    "technology",
    "technologies",
    "devices",
    "systems",
    "components",
    "materials",
    "equipment",
    "software",
    "hardware",
    "platform",
    "infrastructure",
    // Website section labels
    "overview",
    "careers",
    "investor",
    "investors",
    "financials",
    "governance",
    "locations",
    "leadership",
    "announcements",
    "announces",
    "insights",
    "resources",
    "capabilities",
    "sustainability",
    "compliance",
    "diversity",
    "inclusion",
    // Place names
    "silicon",
    "valley",
    "global",
    "americas",
    "pacific",
    "atlantic",
    // Generic words that never appear in person names
    "critical",
    "power",
    "advanced",
    "flexible",
    "liquid",
    "consumer",
    "industrial",
    "commercial",
    "technical",
    "digital",
    "strategic",
    "operational",
    "corporate",
    "executive",
    "professional",
    "integrated",
    "innovative",
    "creative",
    "dynamic",
    "crown",
    "added",
    "center",
    "group",
    "network",
    "marksmen",
    "tag",
    "manager",
];

static TEMPORAL_PREFIX_WORDS: &[&str] = &[
    "through",
    "during",
    "until",
    "before",
    "after",
    "since",
    "from",
];

static MONTH_WORDS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

static ORG_ENTITY_WORDS: &[&str] = &[
    "capital",
    "partners",
    "partner",
    "group",
    "holdings",
    "holding",
    "ventures",
    "venture",
    "management",
    "advisors",
    "advisor",
    "associates",
    "company",
    "corporation",
    "corp",
    "inc",
    "llc",
    "ltd",
    "limited",
    "plc",
];

mod urlencoding {
    pub fn encode(s: &str) -> String {
        percent_encode(s)
    }

    fn percent_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 3);
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}
