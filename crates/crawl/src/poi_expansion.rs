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
fn compile_regex(pattern: &'static str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|error| panic!("invalid regex {pattern:?}: {error}"))
}

static RE_FULL_NAME: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"\b([A-Z][a-z]{1,20}(?:\s+[A-Z][a-z]{1,20}){1,3})\b"));

/// Structured speaker-name fields from JSON APIs or HTML data blobs.
static RE_STRUCTURED_SPEAKER_NAME: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(
        r#"(?i)"(?:fullName|speakerName|speaker_name|presenterName|presenter_name|displayName|display_name)"\s*:\s*"([^"\\]{4,80})""#,
    )
});

/// Explicit speaker labels in plain text or lightly stripped HTML.
static RE_LABELED_SPEAKER_NAME: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(
        r"(?i)(?:speaker|presenter|panelist|keynote(?:\s+speaker)?|host)\s*[:=\-]\s*([A-Z][a-z]{1,20}(?:\s+[A-Z][a-z]{1,20}){1,3})",
    )
});

/// Capitalised job title with trailing preposition — "Chief Executive Officer at".
static RE_TITLE_AT: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(
        r"(?i)(CEO|CTO|CFO|COO|CMO|CISO|CRO|President|Director|VP|Vice President|Head of|Manager|Secretary|Minister|General|Admiral|Ambassador|Chairman|Commissioner|Governor)\b[^<]{0,120}?\bat\b",
    )
});

/// HTML tag stripper.
static RE_HTML_TAGS: LazyLock<Regex> = LazyLock::new(|| compile_regex(r"<[^>]+>"));

/// Anchor href extractor with inner HTML for link-text classification.
static RE_HREF: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"'#]+)["'][^>]*>(.*?)</a>"#));

/// Email regex.
static RE_EMAIL: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}"));

/// LinkedIn profile URL.
static RE_LINKEDIN: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"https?://(?:www\.)?linkedin\.com/in/([a-zA-Z0-9\-_%]+)"));

/// GDELT title extractor.
static RE_GDELT_TITLE: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r#""title"\s*:\s*"([^"]{10,200})""#));

/// GDELT URL extractor.
static RE_GDELT_URL: LazyLock<Regex> = LazyLock::new(|| compile_regex(r#""url"\s*:\s*"([^"]+)""#));

/// OpenCorporates officer extractor.
static RE_OFFICER: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(r#""name"\s*:\s*"([^"]{3,80})"\s*,\s*"position"\s*:\s*"([^"]{3,50})""#)
});

/// JSON-LD script extractor.
static RE_JSON_LD_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(r#"(?is)<script[^>]*type=[\"']application/ld\+json[\"'][^>]*>(.*?)</script>"#)
});

const TARGET_COMPANY_DISCOVERIES_PER_SITE: usize = 5;
type LeadershipPersonCandidate = (String, Option<String>, Option<String>, Option<String>);

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
    "chip",
    "deal",
    "factory",
    "financial",
    "facility",
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
    "plant",
    "trading",
    "urges",
];

/// Geographic or geopolitical words that often surface in GDELT headlines as
/// title-cased spans but are not people.
static GDELT_NON_PERSON_GEO_WORDS: &[&str] = &[
    "africa", "america", "asia", "china", "europe", "india", "japan", "korea", "taiwan", "world",
];

/// Directional or regional modifiers commonly used in place names.
static GDELT_GEO_MODIFIER_WORDS: &[&str] = &[
    "central",
    "east",
    "eastern",
    "global",
    "international",
    "north",
    "northern",
    "south",
    "southern",
    "west",
    "western",
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

struct FetchedPage {
    final_url: Url,
    body: String,
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

    pub async fn expand_from_company_seeds(
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
                seed_org = %seed.organization,
                seed_id = %seed.id,
                "poi_expansion: processing company seed"
            );

            let from_org = self.expand_org_leadership(seed).await;
            for discovery in from_org {
                let key = normalise_name(&discovery.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(discovery);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            if all.len() >= limit {
                break;
            }

            let from_oc = self.expand_opencorporates_board(seed).await;
            for discovery in from_oc {
                let key = normalise_name(&discovery.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(discovery);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            if all.len() >= limit {
                break;
            }

            // Strategy: procurement team pages
            let from_proc = self.expand_procurement_team(seed).await;
            for discovery in from_proc {
                let key = normalise_name(&discovery.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(discovery);
                    if all.len() >= limit {
                        break;
                    }
                }
            }

            if all.len() >= limit {
                break;
            }

            // Strategy: engineering team pages
            let from_eng = self.expand_engineering_team(seed).await;
            for discovery in from_eng {
                let key = normalise_name(&discovery.name);
                if !seen_names.contains(&key) {
                    seen_names.insert(key);
                    all.push(discovery);
                    if all.len() >= limit {
                        break;
                    }
                }
            }
        }

        info!(
            discovered = all.len(),
            "poi_expansion: company-seed complete"
        );
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
        let mut seen_result_names = HashSet::new();
        let homepage = match self.fetch_page(base).await {
            Ok(page) => page,
            Err(error) => {
                debug!(
                    org = %seed.organization,
                    website = %base,
                    error = %error,
                    "leadership_scrape: skipping leadership path expansion after homepage fetch failed"
                );
                return vec![];
            }
        };

        let homepage_base = homepage.final_url.as_str().trim_end_matches('/');

        push_page_discoveries(
            &mut results,
            &mut seen_result_names,
            extract_names_from_leadership_html(&homepage.body, &seed.organization),
            seed,
            homepage.final_url.as_str(),
        );
        if results.len() >= TARGET_COMPANY_DISCOVERIES_PER_SITE {
            return results;
        }

        let mut candidate_urls = default_leadership_urls(homepage_base);
        let homepage_candidates = extract_candidate_leadership_urls(homepage_base, &homepage.body);
        if !homepage_candidates.is_empty() {
            let mut seen = HashSet::new();
            candidate_urls = homepage_candidates
                .into_iter()
                .chain(candidate_urls.into_iter())
                .filter(|url| seen.insert(url.to_ascii_lowercase()))
                .collect();
        }
        candidate_urls.truncate(12);

        for url in &candidate_urls {
            match self.fetch_page(url).await {
                Ok(page) => {
                    if !is_valid_leadership_page_url(&homepage.final_url, &page.final_url) {
                        debug!(
                            org = %seed.organization,
                            requested_url = %url,
                            final_url = %page.final_url,
                            "leadership_scrape: skipping redirected non-leadership page"
                        );
                        continue;
                    }

                    push_page_discoveries(
                        &mut results,
                        &mut seen_result_names,
                        extract_names_from_leadership_html(&page.body, &seed.organization),
                        seed,
                        page.final_url.as_str(),
                    );
                    if results.len() >= TARGET_COMPANY_DISCOVERIES_PER_SITE {
                        break;
                    }
                }
                Err(e) => debug!("leadership_scrape {url}: {e:#}"),
            }
        }

        if results.len() < TARGET_COMPANY_DISCOVERIES_PER_SITE {
            for url in default_org_profile_urls(homepage_base) {
                match self.fetch_page(&url).await {
                    Ok(page) => {
                        if !is_valid_org_profile_page_url(&homepage.final_url, &page.final_url) {
                            continue;
                        }

                        push_page_discoveries(
                            &mut results,
                            &mut seen_result_names,
                            extract_names_from_leadership_html(&page.body, &seed.organization),
                            seed,
                            page.final_url.as_str(),
                        );
                        if results.len() >= TARGET_COMPANY_DISCOVERIES_PER_SITE {
                            break;
                        }
                    }
                    Err(e) => debug!("org_profile_scrape {url}: {e:#}"),
                }
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

    async fn expand_procurement_team(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        let website = match seed.org_website.as_deref() {
            Some(w) if !w.is_empty() => w.trim_end_matches('/').to_string(),
            _ => return vec![],
        };

        let procurement_paths = [
            "/procurement",
            "/purchasing",
            "/supply-chain",
            "/sourcing",
            "/suppliers",
            "/vendor-registration",
            "/supplier-diversity",
            "/procurement-team",
            "/achats",
            "/approvisionnement",
        ];

        let mut results = vec![];
        let mut seen = HashSet::new();

        for path in &procurement_paths {
            let url = format!("{}{}", website, path);
            match self.fetch_page(&url).await {
                Ok(page) => {
                    let names = filter_team_page_candidates(
                        extract_names_from_leadership_html(&page.body, &seed.organization),
                        TeamPageKind::Procurement,
                    );
                    for (name, role, email, linkedin) in names {
                        if seen.contains(&normalise_name(&name)) {
                            continue;
                        }
                        seen.insert(normalise_name(&name));
                        results.push(DiscoveredPoi {
                            name,
                            inferred_role: role,
                            inferred_org: Some(seed.organization.clone()),
                            source_url: page.final_url.to_string(),
                            discovery_method: "procurement_team_page".to_string(),
                            contact_email: email,
                            contact_linkedin: linkedin,
                            confidence: 0.65,
                            seed_person_id: seed.id.clone(),
                            ts_discovered: chrono::Utc::now().timestamp(),
                        });
                    }
                }
                Err(e) => debug!("procurement_team_scrape {url}: {e:#}"),
            }
        }

        results
    }

    async fn expand_engineering_team(&self, seed: &SeedPoi) -> Vec<DiscoveredPoi> {
        let website = match seed.org_website.as_deref() {
            Some(w) if !w.is_empty() => w.trim_end_matches('/').to_string(),
            _ => return vec![],
        };

        let engineering_paths = [
            "/engineering",
            "/technology",
            "/r-and-d",
            "/innovation",
            "/research",
            "/technical-team",
            "/engineering-team",
            "/recherche-developpement",
        ];

        let mut results = vec![];
        let mut seen = HashSet::new();

        for path in &engineering_paths {
            let url = format!("{}{}", website, path);
            match self.fetch_page(&url).await {
                Ok(page) => {
                    let names = filter_team_page_candidates(
                        extract_names_from_leadership_html(&page.body, &seed.organization),
                        TeamPageKind::Engineering,
                    );
                    for (name, role, email, linkedin) in names {
                        if seen.contains(&normalise_name(&name)) {
                            continue;
                        }
                        seen.insert(normalise_name(&name));
                        results.push(DiscoveredPoi {
                            name,
                            inferred_role: role,
                            inferred_org: Some(seed.organization.clone()),
                            source_url: page.final_url.to_string(),
                            discovery_method: "engineering_team_page".to_string(),
                            contact_email: email,
                            contact_linkedin: linkedin,
                            confidence: 0.60,
                            seed_person_id: seed.id.clone(),
                            ts_discovered: chrono::Utc::now().timestamp(),
                        });
                    }
                }
                Err(e) => debug!("engineering_team_scrape {url}: {e:#}"),
            }
        }

        results
    }

    // ─── Strategy 5: Conference speakers ─────────────────────────────────

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
        debug!(
            seed_name = %seed.name,
            "semantic_scholar disabled: author search returns seed-name variants, not real coauthors"
        );
        vec![]
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    async fn get_text(&self, url: &str) -> Result<String> {
        Ok(self.fetch_page(url).await?.body)
    }

    async fn fetch_page(&self, url: &str) -> Result<FetchedPage> {
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
        let final_url = resp.url().clone();
        let text = resp.text().await.context("read body")?;
        Ok(FetchedPage {
            final_url,
            body: text,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Regex for names inside heading or strong tags (structured leadership cards).
static RE_HEADING_NAME: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(
        r"<(?:h[1-6]|strong|b|(?:span|div|p|li)[^>]*class=[^>]*(?:name|title|person|member|leader|exec|team|profile)[^>]*)(?:\s[^>]*)?>([^<]{4,80})</(?:h[1-6]|strong|b|span|div|p|li)>",
    )
});

fn jsonld_type_is_person(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(type_name) => type_name.eq_ignore_ascii_case("Person"),
        serde_json::Value::Array(values) => values.iter().any(jsonld_type_is_person),
        _ => false,
    }
}

fn jsonld_linkedin_url(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(url)) if url.contains("linkedin.com/in/") => {
            Some(url.to_string())
        }
        Some(serde_json::Value::Array(values)) => values.iter().find_map(|entry| match entry {
            serde_json::Value::String(url) if url.contains("linkedin.com/in/") => {
                Some(url.to_string())
            }
            _ => None,
        }),
        _ => None,
    }
}

fn collect_jsonld_people(value: &serde_json::Value, people: &mut Vec<LeadershipPersonCandidate>) {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("@type")
                .or_else(|| map.get("type"))
                .is_some_and(jsonld_type_is_person)
            {
                if let Some(name) = map.get("name").and_then(|entry| entry.as_str()) {
                    let role = map
                        .get("jobTitle")
                        .or_else(|| map.get("jobtitle"))
                        .and_then(|entry| entry.as_str())
                        .map(|entry| entry.to_string());
                    let email = map
                        .get("email")
                        .and_then(|entry| entry.as_str())
                        .map(|entry| entry.trim_start_matches("mailto:").to_ascii_lowercase());
                    let linkedin = jsonld_linkedin_url(map.get("sameAs"));
                    people.push((name.to_string(), role, email, linkedin));
                }
            }

            for child in map.values() {
                collect_jsonld_people(child, people);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_jsonld_people(child, people);
            }
        }
        _ => {}
    }
}

fn extract_names_from_jsonld(html: &str, org: &str) -> Vec<LeadershipPersonCandidate> {
    let mut results = vec![];
    let mut seen = HashSet::new();

    for cap in RE_JSON_LD_SCRIPT.captures_iter(html) {
        let raw_json = cap[1].trim();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_json) else {
            continue;
        };

        let mut people = Vec::new();
        collect_jsonld_people(&value, &mut people);
        for (name, role, email, linkedin) in people {
            if name.len() < 5 || seen.contains(&name) {
                continue;
            }
            if !is_plausible_org_leadership_candidate(&name, org) {
                continue;
            }
            let role = sanitize_leadership_role(role);

            seen.insert(name.clone());
            results.push((name, role, email, linkedin));
        }
    }

    results
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut boundary = index.min(s.len());
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn ceil_char_boundary(s: &str, index: usize) -> usize {
    let mut boundary = index.min(s.len());
    while boundary < s.len() && !s.is_char_boundary(boundary) {
        boundary += 1;
    }
    boundary
}

fn html_window(html: &str, start: usize, before: usize, after: usize) -> &str {
    let window_start = floor_char_boundary(html, start.saturating_sub(before));
    let mut window_end = ceil_char_boundary(html, start.saturating_add(after));
    if window_end < window_start {
        window_end = window_start;
    }
    &html[window_start..window_end]
}

/// Extract `(name, role, email, linkedin)` tuples from a company leadership HTML page.
/// Prefers names found inside heading/bold/span.name tags for higher precision.
fn extract_names_from_leadership_html(html: &str, org: &str) -> Vec<LeadershipPersonCandidate> {
    let mut results = vec![];
    let mut seen = HashSet::new();

    for (name, role, email, linkedin) in extract_names_from_jsonld(html, org) {
        seen.insert(name.clone());
        results.push((name, role, email, linkedin));
    }

    // Phase 1: Extract names from structured HTML elements (higher confidence).
    for cap in RE_HEADING_NAME.captures_iter(html) {
        let raw = RE_HTML_TAGS.replace_all(&cap[1], " ");
        let raw = raw.trim();
        let name = if let Some(name_cap) = RE_FULL_NAME.captures(raw) {
            name_cap[1].to_string()
        } else if looks_like_person_name(raw) {
            raw.to_string()
        } else {
            continue;
        };
        if name.len() < 5 || seen.contains(&name) {
            continue;
        }
        if !is_plausible_org_leadership_candidate(&name, org) {
            continue;
        }
        // Find role, email, linkedin in surrounding HTML.
        let start = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let vicinity = html_window(html, start, 200, 500);
        let role = RE_TITLE_AT.captures(vicinity).map(|c| c[1].to_string());
        let role = sanitize_leadership_role(infer_target_role(vicinity).or(role));
        let email = RE_EMAIL.find(vicinity).map(|m| m.as_str().to_lowercase());
        let linkedin = RE_LINKEDIN
            .captures(vicinity)
            .map(|c| format!("https://linkedin.com/in/{}", &c[1]));

        seen.insert(name.clone());
        results.push((name, role, email, linkedin));
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

    let titles: Vec<&str> = RE_GDELT_TITLE
        .captures_iter(json)
        .filter_map(|c| Some(c.get(1)?.as_str()))
        .collect();
    let urls: Vec<&str> = RE_GDELT_URL
        .captures_iter(json)
        .filter_map(|c| Some(c.get(1)?.as_str()))
        .collect();

    // First pass: count co-mention frequency per candidate name
    let mut mention_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    for title in titles.iter() {
        if !is_plausible_gdelt_title(title, &seed_name_lower) {
            continue;
        }
        for cap in RE_FULL_NAME.captures_iter(title) {
            let name = cap[1].to_string();
            if name.to_lowercase() == seed_name_lower || name.len() < 5 {
                continue;
            }
            *mention_counts.entry(name).or_insert(0) += 1;
        }
    }

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
            // Graduate confidence by co-mention frequency: 1->0.55, 2->0.65, 3+->0.75
            let mentions = mention_counts.get(&name).copied().unwrap_or(1);
            let confidence = match mentions {
                1 => GDELT_BASE_CONFIDENCE,
                2 => 0.65,
                _ => 0.75f32.min(GDELT_BASE_CONFIDENCE + 0.05 * mentions as f32),
            };
            results.push(DiscoveredPoi {
                name,
                inferred_role: None,
                inferred_org: None,
                source_url,
                discovery_method: "gdelt_co_mention".to_string(),
                contact_email: None,
                contact_linkedin: None,
                confidence,
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
    let candidate_words: Vec<&str> = name_lower.split_whitespace().collect();
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
    if candidate_words
        .iter()
        .any(|word| GDELT_NON_PERSON_WORDS.contains(word))
    {
        return false;
    }
    if candidate_words
        .iter()
        .any(|word| GDELT_NON_PERSON_GEO_WORDS.contains(word))
    {
        return false;
    }
    if candidate_words.len() >= 2
        && GDELT_GEO_MODIFIER_WORDS.contains(&candidate_words[0])
        && candidate_words[1..]
            .iter()
            .any(|word| GDELT_NON_PERSON_GEO_WORDS.contains(word))
    {
        return false;
    }

    true
}

fn default_leadership_urls(base: &str) -> Vec<String> {
    [
        // Leadership / executive pages
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
        // Procurement / sourcing pages (Fix 2)
        "/procurement",
        "/sourcing",
        "/supply-chain",
        "/purchasing",
        "/about/procurement",
        "/about/supply-chain",
        // Quality / compliance pages (Fix 3)
        "/quality",
        "/quality-assurance",
        "/compliance",
        "/certifications",
        "/about/quality",
        // Engineering / R&D pages (Fix 4)
        "/engineering",
        "/technology",
        "/r-and-d",
        "/research-development",
        "/innovation",
        "/about/engineering",
        // Operations / supply-chain pages (Fix 5)
        "/operations",
        "/manufacturing",
        "/logistics",
        "/about/operations",
    ]
    .iter()
    .map(|path| format!("{base}{path}"))
    .collect()
}

fn default_org_profile_urls(base: &str) -> Vec<String> {
    ["/about", "/about-us", "/who-we-are"]
        .iter()
        .map(|path| format!("{base}{path}"))
        .collect()
}

fn path_segment_tokens(path: &str) -> Vec<String> {
    path.trim_matches('/')
        .split('/')
        .flat_map(|segment| {
            segment
                .split(|character: char| !character.is_ascii_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(|token| token.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn generic_listing_path_matches(path: &str) -> bool {
    let tokens = path_segment_tokens(path);
    let token_set: HashSet<&str> = tokens.iter().map(|token| token.as_str()).collect();

    (token_set.contains("our") && token_set.contains("companies"))
        || token_set.contains("portfolio")
        || token_set.contains("products")
        || token_set.contains("services")
        || token_set.contains("solutions")
        || token_set.contains("brands")
        || token_set.contains("locations")
}

fn normalize_anchor_text(html: &str) -> String {
    RE_HTML_TAGS
        .replace_all(
            &html
                .replace("&nbsp;", " ")
                .replace("&amp;", " & ")
                .replace("&#39;", " '")
                .replace("&quot;", " \""),
            " ",
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn leadership_link_text_matches(text: &str) -> bool {
    let normalized = normalize_anchor_text(text).to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 80 {
        return false;
    }

    let blocked_exact = [
        "our companies",
        "portfolio",
        "products",
        "services",
        "solutions",
        "brands",
        "locations",
        "contact",
        "news",
        "careers",
        "investors",
        "investor relations",
        "privacy",
        "privacy policy",
        "terms",
        "cookie policy",
    ];
    if blocked_exact.iter().any(|blocked| normalized == *blocked) {
        return false;
    }

    let tokens: HashSet<&str> = normalized.split_whitespace().collect();
    tokens.contains("team")
        || tokens.contains("leadership")
        || tokens.contains("board")
        || tokens.contains("people")
        || tokens.contains("directors")
        || tokens.contains("management")
        || tokens.contains("procurement")
        || tokens.contains("sourcing")
        || tokens.contains("purchasing")
        || tokens.contains("quality")
        || tokens.contains("engineering")
        || tokens.contains("technology")
        || tokens.contains("operations")
        || tokens.contains("manufacturing")
        || tokens.contains("logistics")
        || tokens.contains("compliance")
        || normalized.contains("meet the team")
        || normalized.contains("leadership team")
        || normalized.contains("management team")
        || normalized.contains("board of directors")
        || normalized.contains("supply chain")
        || normalized.contains("r&d")
}

fn leadership_path_matches(path: &str) -> bool {
    let normalized = path.trim_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if generic_listing_path_matches(path) {
        return false;
    }

    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let leadership_segments = [
        "leadership",
        "team",
        "management",
        "executive",
        "executives",
        "board",
        "people",
        "directors",
        "procurement",
        "sourcing",
        "purchasing",
        "supply-chain",
        "quality",
        "compliance",
        "engineering",
        "technology",
        "r-and-d",
        "innovation",
        "operations",
        "manufacturing",
        "logistics",
    ];
    let tokens = path_segment_tokens(path);

    let joined = segments.join("/");
    joined.contains("leadership")
        || tokens
            .iter()
            .any(|token| leadership_segments.contains(&token.as_str()))
}

fn org_profile_path_matches(path: &str) -> bool {
    let normalized = path.trim_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let profile_segments = ["about", "about-us", "who-we-are"];

    segments
        .iter()
        .any(|segment| profile_segments.contains(segment))
}

fn is_valid_leadership_page_url(homepage_url: &Url, page_url: &Url) -> bool {
    matches!(page_url.scheme(), "http" | "https")
        && page_url.host_str() == homepage_url.host_str()
        && (leadership_path_matches(page_url.path()) || org_profile_path_matches(page_url.path()))
}

fn is_valid_org_profile_page_url(homepage_url: &Url, page_url: &Url) -> bool {
    matches!(page_url.scheme(), "http" | "https")
        && page_url.host_str() == homepage_url.host_str()
        && (leadership_path_matches(page_url.path()) || org_profile_path_matches(page_url.path()))
}

fn push_page_discoveries(
    results: &mut Vec<DiscoveredPoi>,
    seen_names: &mut HashSet<String>,
    names: Vec<LeadershipPersonCandidate>,
    seed: &SeedPoi,
    source_url: &str,
) {
    for (name, role, email, linkedin) in names {
        let normalized = normalise_name(&name);
        if normalized.is_empty() || !seen_names.insert(normalized) {
            continue;
        }

        results.push(DiscoveredPoi {
            name,
            inferred_role: role,
            inferred_org: Some(seed.organization.clone()),
            source_url: source_url.to_string(),
            discovery_method: "org_leadership".to_string(),
            contact_email: email,
            contact_linkedin: linkedin,
            confidence: 0.75,
            seed_person_id: seed.id.clone(),
            ts_discovered: Utc::now().timestamp(),
        });
    }
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
        let anchor_text = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
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
        if generic_listing_path_matches(joined.path()) {
            continue;
        }
        if !leadership_path_matches(joined.path())
            && !leadership_link_text_matches(anchor_text)
            && !org_profile_path_matches(joined.path())
        {
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
    let officer_re = &*RE_OFFICER;

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
fn build_conference_queries(name: &str, org: &str, role_family: &str) -> Vec<String> {
    let mut queries = vec![format!(
        "https://sessionize.com/speakers?query={}",
        urlencoding::encode(name)
    )];

    // Add role-family-specific conference search queries for non-exec roles
    let domain_keywords: &[&str] = match role_family {
        "Supply Chain" => &[
            "procurement conference",
            "supply chain summit",
            "ISM conference",
        ],
        "Engineering" => &[
            "engineering conference",
            "technical summit",
            "innovation conference",
        ],
        "Quality" => &[
            "quality conference",
            "ASQ conference",
            "continuous improvement summit",
        ],
        "Operations" => &[
            "manufacturing conference",
            "operations summit",
            "lean summit",
        ],
        "Security" => &[
            "cybersecurity conference",
            "infosec conference",
            "RSA speakers",
        ],
        "Finance" => &["CFO summit", "finance conference", "treasury conference"],
        _ => &[],
    };

    for keyword in domain_keywords {
        if !org.is_empty() {
            queries.push(format!(
                "https://sessionize.com/speakers?query={}+{}",
                urlencoding::encode(org),
                urlencoding::encode(keyword)
            ));
        }
    }

    queries
}

/// Extract `(name, role)` speaker pairs from conference HTML/JSON pages.
///
/// Only returns names that pass `looks_like_person_name()` validation to filter
/// out website navigation, topic labels, error messages, and other non-person text.
fn extract_speakers_from_html(html: &str) -> Vec<(String, Option<String>)> {
    let stripped = RE_HTML_TAGS.replace_all(html, " ");
    let mut seen = HashSet::new();
    let mut results = vec![];

    for cap in RE_STRUCTURED_SPEAKER_NAME.captures_iter(html) {
        let name = cap[1].trim().to_string();
        if name.len() < 5 || seen.contains(&name) {
            continue;
        }
        if !looks_like_person_name(&name) {
            debug!(rejected_name = %name, "extract_speakers: rejected non-person name");
            continue;
        }
        seen.insert(name.clone());
        results.push((name, None));
    }

    for cap in RE_LABELED_SPEAKER_NAME.captures_iter(&stripped) {
        let name = cap[1].trim().to_string();
        if name.len() < 5 || seen.contains(&name) {
            continue;
        }
        if !looks_like_person_name(&name) {
            debug!(rejected_name = %name, "extract_speakers: rejected non-person name");
            continue;
        }
        seen.insert(name.clone());
        results.push((name, None));
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

    if looks_like_marketing_section_heading(name) {
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

    let role_like_words = [
        "chair",
        "chief",
        "commercial",
        "compliance",
        "director",
        "engineering",
        "executive",
        "finance",
        "head",
        "information",
        "lead",
        "manager",
        "officer",
        "operations",
        "president",
        "procurement",
        "program",
        "quality",
        "sales",
        "strategy",
        "supply",
        "technology",
        "vice",
        "vp",
    ];
    let role_word_hits = candidate_words
        .iter()
        .filter(|word| role_like_words.contains(&word.as_str()))
        .count();
    if role_word_hits >= 2 {
        return false;
    }

    let company_style_tail_words = [
        "electric",
        "elettronica",
        "electronics",
        "technology",
        "technologies",
        "systems",
        "solutions",
        "automation",
        "manufacturing",
        "semiconductor",
        "semiconductors",
        "oyj",
        "oü",
        "spa",
        "srl",
    ];
    if candidate_words
        .last()
        .map(|word| company_style_tail_words.contains(&word.as_str()))
        .unwrap_or(false)
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

fn looks_like_marketing_section_heading(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if ORG_LEADERSHIP_MARKETING_PHRASES.contains(&lower.as_str())
        || ORG_LEADERSHIP_SECTION_HEADING_PHRASES.contains(&lower.as_str())
    {
        return true;
    }

    let words: Vec<&str> = lower.split_whitespace().collect();
    let marketing_word_hits = words
        .iter()
        .filter(|word| ORG_LEADERSHIP_MARKETING_WORDS.contains(word))
        .count();
    let section_heading_hits = words
        .iter()
        .filter(|word| ORG_LEADERSHIP_SECTION_HEADING_WORDS.contains(word))
        .count();
    if marketing_word_hits >= 2 || section_heading_hits >= 2 {
        return true;
    }

    lower.starts_with("about ")
        || lower.starts_with("who we ")
        || lower.starts_with("why ")
        || lower.starts_with("watch us ")
        || lower.starts_with("connect with ")
        || lower.starts_with("building ")
        || lower.starts_with("celebrating ")
        || lower.starts_with("general ")
        || lower.starts_with("important ")
        || lower.starts_with("registration ")
        || lower.starts_with("customize ")
        || lower.starts_with("new product ")
        || lower.starts_with("office ")
        || lower.starts_with("job ")
        || lower.starts_with("kontaktieren ")
        || lower.ends_with(" association")
        || lower.ends_with(" assembly")
        || lower.ends_with(" assemblies")
        || lower.ends_with(" building")
        || lower.ends_with(" chamber")
        || lower.ends_with(" chambers")
        || lower.ends_with(" code")
        || lower.ends_with(" certificates")
        || lower.ends_with(" council")
        || lower.ends_with(" defence")
        || lower.ends_with(" development")
        || lower.ends_with(" employers")
        || lower.ends_with(" engineering")
        || lower.ends_with(" inquiry")
        || lower.ends_with(" layout")
        || lower.ends_with(" office")
        || lower.ends_with(" report")
        || lower.ends_with(" releases")
        || (lower.starts_with("the ") && marketing_word_hits >= 1)
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

#[derive(Clone, Copy)]
enum TeamPageKind {
    Procurement,
    Engineering,
}

fn role_matches_team_page(role: &str, page_kind: TeamPageKind) -> bool {
    let lower = role.to_ascii_lowercase();
    match page_kind {
        TeamPageKind::Procurement => [
            "procurement",
            "purchasing",
            "sourcing",
            "buyer",
            "category",
            "commodity",
            "supply chain",
            "vendor",
            "supplier diversity",
            "materials",
        ]
        .iter()
        .any(|marker| lower.contains(marker)),
        TeamPageKind::Engineering => [
            "engineering",
            "technology",
            "technical",
            "r&d",
            "research",
            "design",
            "innovation",
            "product development",
        ]
        .iter()
        .any(|marker| lower.contains(marker)),
    }
}

fn filter_team_page_candidates(
    candidates: Vec<LeadershipPersonCandidate>,
    page_kind: TeamPageKind,
) -> Vec<LeadershipPersonCandidate> {
    candidates
        .into_iter()
        .filter(|(_, role, _, _)| {
            role.as_deref()
                .is_some_and(|role| role_matches_team_page(role, page_kind))
        })
        .collect()
}

fn infer_target_role(context: &str) -> Option<String> {
    let lc = context.to_ascii_lowercase();
    let patterns = [
        ("deputy director general", "Deputy Director General"),
        ("director general", "Director General"),
        ("deputy director", "Deputy Director"),
        ("department director", "Department Director"),
        ("department head", "Department Head"),
        ("head of procurement", "Head of Procurement"),
        ("head of supply chain", "Head of Supply Chain"),
        ("procurement director", "Procurement Director"),
        ("procurement manager", "Procurement Manager"),
        ("purchasing director", "Purchasing Director"),
        ("purchasing manager", "Purchasing Manager"),
        ("sourcing manager", "Sourcing Manager"),
        ("senior buyer", "Senior Buyer"),
        ("strategic buyer", "Strategic Buyer"),
        ("category manager", "Category Manager"),
        ("commodity manager", "Commodity Manager"),
        ("vendor management manager", "Vendor Management Manager"),
        ("vendor management director", "Vendor Management Director"),
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
        ("head of engineering", "Head of Engineering"),
        ("engineering director", "Engineering Director"),
        ("engineering manager", "Engineering Manager"),
        ("head of technology", "Head of Technology"),
        ("technical director", "Technical Director"),
        ("research director", "Research Director"),
        ("innovation director", "Innovation Director"),
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

    functional_scope.iter().any(|k| r.contains(k)) && seniority.iter().any(|k| r.contains(k))
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
    fn person_name_validation_rejects_single_word_and_topic_names() {
        assert!(!looks_like_person_name("Series"));
        assert!(!looks_like_person_name("Talks"));
        assert!(!looks_like_person_name("Books"));
        assert!(!looks_like_person_name("Machine Learning"));
        assert!(!looks_like_person_name("Public Health"));
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
    fn conference_speaker_extraction_keeps_structured_speaker_names() {
        let html = r#"{
            "results": [
                {"fullName": "John Smith", "tagLine": "VP Engineering"},
                {"speakerName": "Marie Dupont"}
            ]
        }"#;

        let speakers = extract_speakers_from_html(html);
        assert_eq!(speakers.len(), 2);
        assert!(speakers.iter().any(|(name, _)| name == "John Smith"));
        assert!(speakers.iter().any(|(name, _)| name == "Marie Dupont"));
    }

    #[test]
    fn conference_speaker_extraction_keeps_labeled_speaker_names() {
        let html = "Speaker: John Smith";

        let speakers = extract_speakers_from_html(html);
        assert_eq!(speakers.len(), 1);
        assert!(speakers.iter().any(|(name, _)| name == "John Smith"));
    }

    #[test]
    fn conference_speaker_extraction_rejects_production_topic_fragments() {
        let html = r#"
            <div>Public Speaking</div>
            <div>Chief Information Officer</div>
            <div>Driverless Cars</div>
            <div>Machine Learning</div>
            <div>Series Go</div>
            <div>Talks Talks</div>
            <div>Books Short</div>
            <div>About Our</div>
        "#;

        let speakers = extract_speakers_from_html(html);
        assert!(speakers.is_empty());
    }

    #[test]
    fn conference_queries_use_supported_sessionize_endpoint_only() {
        let urls = build_conference_queries("John Smith", "Flex", "engineering");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://sessionize.com/speakers?query=John+Smith");
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
    fn gdelt_rejects_geopolitical_and_facility_fragments_seen_in_production() {
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
                    "title": "Steve Sanghi says South Korea expands chip incentives",
                    "url": "https://example.com/korea"
                },
                {
                    "title": "Steve Sanghi visits Chip Plant after supplier review",
                    "url": "https://example.com/plant"
                },
                {
                    "title": "Steve Sanghi and John Carter discuss sourcing plans",
                    "url": "https://example.com/person"
                }
            ]
        }"#;

        let discoveries = parse_gdelt_response(json, &seed);
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].name, "John Carter");
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
    fn leadership_extraction_rejects_company_style_name_candidates() {
        let html = r#"
            <section>
                <h3>Schneider Electric</h3>
                <p>Vice President</p>
                <h3>Jane Smith</h3>
                <p>Procurement Director</p>
            </section>
        "#;

        let people = extract_names_from_leadership_html(html, "Acme Manufacturing");
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].0, "Jane Smith");
    }

    #[test]
    fn leadership_extraction_rejects_role_title_phrases_as_people() {
        assert!(!is_plausible_org_leadership_candidate(
            "Vehicle Program Director",
            "Acme Manufacturing"
        ));
    }

    #[test]
    fn leadership_extraction_rejects_footer_and_company_listing_phrases() {
        assert!(!looks_like_person_name("Our Companies"));
        assert!(!looks_like_person_name(
            "Privacy Statement          General Terms"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Our Companies",
            "Acme Manufacturing"
        ));
    }

    #[test]
    fn leadership_extraction_handles_unicode_vicinity_windows() {
        let html = format!(
            "<section>{}<h3>Jane Smith</h3><p>Procurement Director</p></section>",
            "é".repeat(250)
        );

        let people = extract_names_from_leadership_html(&html, "Acme Manufacturing");

        assert_eq!(people.len(), 1);
        assert_eq!(people[0].0, "Jane Smith");
    }

    #[test]
    fn leadership_extraction_rejects_meeting_title_phrases() {
        assert!(!looks_like_person_name("Annual General Meeting"));
        assert!(!looks_like_person_name("The Annual General Meeting"));
        assert!(!looks_like_person_name("Shareholder Meeting"));
    }

    #[test]
    fn leadership_extraction_rejects_section_heading_phrases_seen_in_production() {
        assert!(!looks_like_person_name("Get In Touch"));
        assert!(!looks_like_person_name("Supplier Day"));
        assert!(!looks_like_person_name("Markets We Serve"));
        assert!(!looks_like_person_name("Our Expertise"));
        assert!(!looks_like_person_name("About Benchmark"));
        assert!(!looks_like_person_name("Financial Reporting"));
        assert!(!looks_like_person_name("Stock Information"));
        assert!(!looks_like_person_name("Our Team"));
        assert!(!looks_like_person_name("Who We Are"));
        assert!(!looks_like_person_name("Why Creation"));
        assert!(!looks_like_person_name("Advisory Board"));
        assert!(!looks_like_person_name("Key Figures"));
        assert!(!looks_like_person_name("Rapid Prototyping"));
        assert!(!looks_like_person_name("Press Releases"));
        assert!(!looks_like_person_name("Markets Served"));
        assert!(!looks_like_person_name("Quick Links"));
        assert!(!looks_like_person_name("Other Links"));
        assert!(!looks_like_person_name("Upcoming Events"));
        assert!(!looks_like_person_name("Latest News"));
        assert!(!looks_like_person_name("Quality Certifications"));
        assert!(!looks_like_person_name("Workmanship Standards"));
        assert!(!looks_like_person_name("RoHS Compliant"));
        assert!(!looks_like_person_name("Safety Achievements"));
        assert!(!looks_like_person_name("Engineering Support"));
        assert!(!looks_like_person_name("Printed Circuit Board Assembly"));
        assert!(!looks_like_person_name("Product Maintenance"));
        assert!(!looks_like_person_name("System Integration"));
        assert!(!looks_like_person_name("Production Outsourcing"));
        assert!(!looks_like_person_name("Cable Assembly"));
        assert!(!looks_like_person_name("PCB Assembly"));
        assert!(!looks_like_person_name("Module Building"));
        assert!(!looks_like_person_name("Box Build"));
        assert!(!looks_like_person_name("Box Build Assembly"));
        assert!(!looks_like_person_name("Cable And Wire Harness Assemblies"));
        assert!(!looks_like_person_name("Magnetic Assemblies"));
        assert!(!looks_like_person_name("After Sales"));
        assert!(!looks_like_person_name("Estonian Defence"));
        assert!(!looks_like_person_name("Accreditations And Certificates"));
        assert!(!looks_like_person_name("Firmware Development"));
        assert!(!looks_like_person_name("PCB Layout"));
        assert!(!looks_like_person_name("Mechanical Engineering"));
        assert!(!looks_like_person_name("Device Manufacturers Need To"));
        assert!(!looks_like_person_name("What Drives Us"));
        assert!(!looks_like_person_name("Environmental Stewardship"));
        assert!(!looks_like_person_name("Pollution Prevention"));
        assert!(!looks_like_person_name("Precision Plastics"));
        assert!(!looks_like_person_name("Winning Team"));
        assert!(!looks_like_person_name("Terrestrial Networks"));
        assert!(!looks_like_person_name("What Exactly Are These"));
        assert!(!looks_like_person_name("The Sweet Spots Where"));
        assert!(!looks_like_person_name("Mobile Assets"));
        assert!(!looks_like_person_name("Remote Sensor Networks"));
        assert!(!looks_like_person_name("The Reality Check"));
        assert!(!looks_like_person_name("Latency That"));
        assert!(!looks_like_person_name("Data Rates From"));
        assert!(!looks_like_person_name("Costs That"));
        assert!(!looks_like_person_name("Coverage Gaps"));
        assert!(!looks_like_person_name("The Bottom Line"));
        assert!(!looks_like_person_name("Quality Over Price"));
        assert!(!looks_like_person_name("Fideltronik Headquarters"));
        assert!(!looks_like_person_name("Annual Report"));
        assert!(!looks_like_person_name("Stock Exchange Releases"));
        assert!(!looks_like_person_name("International Trade Council"));
        assert!(!looks_like_person_name("Entrepreneurs Association"));
        assert!(!looks_like_person_name("British Chamber"));
        assert!(!looks_like_person_name("Oulu Office"));
        assert!(!looks_like_person_name("North America"));
        assert!(!looks_like_person_name("Johor Bahru"));
        assert!(!looks_like_person_name("Financial Director"));
        assert!(!looks_like_person_name("Scanfil Oyj"));
        assert!(!looks_like_person_name("General Enquiry"));
        assert!(!looks_like_person_name("Important Documents"));
        assert!(!looks_like_person_name("Registration No"));
        assert!(!looks_like_person_name("Customize Consent Preferences"));
        assert!(!looks_like_person_name("New Product Introduction"));
        assert!(!looks_like_person_name("Evertiq Expo"));
        assert!(!looks_like_person_name("Combined Naval Event"));
        assert!(!looks_like_person_name("Life Sciences"));
        assert!(!looks_like_person_name("Office Working Hours"));
        assert!(!looks_like_person_name("Job Vacancies"));
        assert!(!looks_like_person_name("Kontaktieren Sie"));
        assert!(!looks_like_person_name("Clean Energy"));
        assert!(!looks_like_person_name("End Innovation"));
        assert!(!looks_like_person_name("Key Features"));
        assert!(!looks_like_person_name("Supplier Toolbox"));
        assert!(!looks_like_person_name("EMC Lab"));
        assert!(!looks_like_person_name("Sourced. Supplied. Smart."));
        assert!(!looks_like_person_name("Test Design"));
        assert!(!is_plausible_org_leadership_candidate(
            "Connect With Us",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Manufacturer For Industry",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Building Optics That Matter",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Celebrating World Quality Week",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Watch Us Deploy Automated",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Watch Us Build",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Successfully Ramp",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Introducing Smart Factories",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "The Story Behind Autonomous",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "The Challenges",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "For The Medical Industry",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Product Event Briefing",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Office Working Schedule",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Important Event Bulletin",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Advisory Board",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Key Figures",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Rapid Prototyping",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Quick Links",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Upcoming Events",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Quality Certifications",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Engineering Support",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "System Integration",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Winning Team",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Remote Sensor Networks",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "British Chamber",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Annual Report",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Sensor Development",
            "Acme Manufacturing"
        ));
        assert!(!is_plausible_org_leadership_candidate(
            "Analog Engineering",
            "Acme Manufacturing"
        ));
        assert!(!looks_like_person_name("Our Vision"));
        assert!(!looks_like_person_name("Our Mission"));
        assert!(!looks_like_person_name("Our Purpose"));
        assert!(!looks_like_person_name("Our Values"));
        assert!(!looks_like_person_name("Our Purpose Journey"));
        assert!(!looks_like_person_name("Your Vision"));
        assert!(!looks_like_person_name("Your Smart Home"));
        assert!(!looks_like_person_name("Customer Committed"));
        assert!(!looks_like_person_name("Market Conditions"));
        assert!(!looks_like_person_name("Gen Communications"));
        assert!(!looks_like_person_name("Benchmark Celebrates"));
        assert!(!looks_like_person_name("For Cutting"));
        assert!(!looks_like_person_name("China Tariffs"));
    }

    #[test]
    fn team_page_filter_requires_page_specific_roles() {
        let candidates = vec![
            (
                "Jane Smith".to_string(),
                Some("Senior Buyer".to_string()),
                None,
                None,
            ),
            ("Key Features".to_string(), None, None, None),
            (
                "John Carter".to_string(),
                Some("Engineering Director".to_string()),
                None,
                None,
            ),
            (
                "Supplier Toolbox".to_string(),
                Some("Vice President".to_string()),
                None,
                None,
            ),
        ];

        let procurement =
            filter_team_page_candidates(candidates.clone(), TeamPageKind::Procurement);
        assert_eq!(procurement.len(), 1);
        assert_eq!(procurement[0].0, "Jane Smith");

        let engineering = filter_team_page_candidates(candidates, TeamPageKind::Engineering);
        assert_eq!(engineering.len(), 1);
        assert_eq!(engineering[0].0, "John Carter");
    }

    #[test]
    fn infer_target_role_detects_team_page_buyers_and_engineers() {
        assert_eq!(
            infer_target_role("Meet Jane Smith, Senior Buyer for indirect materials"),
            Some("Senior Buyer".to_string())
        );
        assert_eq!(
            infer_target_role("Leadership card: John Carter, Head of Supply Chain"),
            Some("Head of Supply Chain".to_string())
        );
        assert_eq!(
            infer_target_role("Engineering leadership: Marie Dubois, Engineering Director"),
            Some("Engineering Director".to_string())
        );
    }

    #[test]
    fn leadership_extraction_keeps_jsonld_people() {
        let html = r#"
            <html>
                <head>
                    <script type="application/ld+json">
                        {
                            "@context": "https://schema.org",
                            "@type": "Organization",
                            "member": [
                                {
                                    "@type": "Person",
                                    "name": "Jane Smith",
                                    "jobTitle": "Procurement Director",
                                    "sameAs": ["https://www.linkedin.com/in/jane-smith"]
                                }
                            ]
                        }
                    </script>
                </head>
            </html>
        "#;

        let people = extract_names_from_leadership_html(html, "Acme Manufacturing");
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].0, "Jane Smith");
        assert_eq!(people[0].1.as_deref(), Some("Procurement Director"));
        assert_eq!(
            people[0].3.as_deref(),
            Some("https://www.linkedin.com/in/jane-smith")
        );
    }

    #[test]
    fn leadership_extraction_keeps_jsonld_people_without_job_title() {
        let html = r#"
            <html>
                <head>
                    <script type="application/ld+json">
                        {
                            "@context": "https://schema.org",
                            "@type": "Organization",
                            "member": [
                                {
                                    "@type": "Person",
                                    "name": "Francois Dupont",
                                    "sameAs": ["https://www.linkedin.com/in/francois-dupont"]
                                }
                            ]
                        }
                    </script>
                </head>
            </html>
        "#;

        let people = extract_names_from_leadership_html(html, "Acme Manufacturing");
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].0, "Francois Dupont");
        assert_eq!(people[0].1, None);
        assert_eq!(
            people[0].3.as_deref(),
            Some("https://www.linkedin.com/in/francois-dupont")
        );
    }

    #[test]
    fn leadership_extraction_keeps_structured_team_card_names_without_role() {
        let html = r#"
            <section class="team-grid">
                <div class="team-member-name">Francois Dupont</div>
                <div class="team-member-name">Jane Smith</div>
            </section>
        "#;

        let people = extract_names_from_leadership_html(html, "Acme Manufacturing");
        assert_eq!(people.len(), 2);
        assert!(people
            .iter()
            .any(|person| person.0 == "Francois Dupont" && person.1.is_none()));
        assert!(people
            .iter()
            .any(|person| person.0 == "Jane Smith" && person.1.is_none()));
    }

    #[test]
    fn homepage_link_discovery_finds_same_host_leadership_pages() {
        let html = r#"
            <html>
                <body>
                    <a href="/about/leadership">Leadership</a>
                    <a href="https://example.com/company/team">Team</a>
                    <a href="/our-team"><span>OUR TEAM</span></a>
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
        assert!(urls.iter().any(|url| url == "https://example.com/our-team"));
        assert!(!urls.iter().any(|url| url.contains("steam-controller")));
        assert!(!urls.iter().any(|url| url.contains("external.example.org")));
    }

    #[test]
    fn homepage_link_discovery_uses_team_link_text_for_profile_pages() {
        let html = r#"
            <html>
                <body>
                    <a href="/about-us"><span>OUR TEAM</span></a>
                    <a href="/legal">Privacy</a>
                </body>
            </html>
        "#;

        let urls = extract_candidate_leadership_urls("https://example.com", html);

        assert!(urls.iter().any(|url| url == "https://example.com/about-us"));
        assert!(!urls.iter().any(|url| url == "https://example.com/legal"));
    }

    #[test]
    fn homepage_link_discovery_rejects_generic_company_listing_pages() {
        let html = r#"
            <html>
                <body>
                    <a href="/our-companies">Our Companies</a>
                    <a href="/company/portfolio">Portfolio</a>
                    <a href="/about/team">Team</a>
                </body>
            </html>
        "#;

        let urls = extract_candidate_leadership_urls("https://example.com", html);

        assert!(!urls
            .iter()
            .any(|url| url == "https://example.com/our-companies"));
        assert!(!urls
            .iter()
            .any(|url| url == "https://example.com/company/portfolio"));
        assert!(urls
            .iter()
            .any(|url| url == "https://example.com/about/team"));
    }

    #[test]
    fn leadership_page_validation_rejects_redirects_to_generic_company_pages() {
        let homepage = Url::parse("https://example.com")
            .unwrap_or_else(|error| panic!("test fixture URL should parse: {error}"));
        let redirected_listing = Url::parse("https://example.com/our-companies")
            .unwrap_or_else(|error| panic!("test fixture URL should parse: {error}"));
        let valid_team_page = Url::parse("https://example.com/about/team")
            .unwrap_or_else(|error| panic!("test fixture URL should parse: {error}"));
        let valid_our_team_page = Url::parse("https://example.com/our-team")
            .unwrap_or_else(|error| panic!("test fixture URL should parse: {error}"));
        let valid_about_page = Url::parse("https://example.com/about-us")
            .unwrap_or_else(|error| panic!("test fixture URL should parse: {error}"));

        assert!(!is_valid_leadership_page_url(
            &homepage,
            &redirected_listing
        ));
        assert!(is_valid_leadership_page_url(&homepage, &valid_team_page));
        assert!(is_valid_leadership_page_url(
            &homepage,
            &valid_our_team_page
        ));
        assert!(is_valid_leadership_page_url(&homepage, &valid_about_page));
    }
}

/// Reject obvious non-person strings (all-caps acronyms, single word, numbers,
/// common website/navigation phrases, topic labels, error messages, etc.).
fn looks_like_person_name(s: &str) -> bool {
    if !unicode_person_name(s) {
        return false;
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() < 2 {
        return false;
    }
    if words.len() > 5 {
        return false;
    }
    // Reject names where every word is very short (likely acronyms or labels).
    let total_chars: usize = words.iter().map(|w| w.len()).sum();
    if total_chars < 6 {
        return false;
    }
    // Blocklist of common non-person phrases that match the name regex.
    let normalized_words = words
        .iter()
        .map(|word| {
            word.trim_matches(|character: char| !character.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if NON_PERSON_PHRASES
        .iter()
        .any(|phrase| normalized_words == *phrase)
    {
        return false;
    }
    if looks_like_temporal_phrase(&words) {
        return false;
    }
    // Reject if ANY word is a common non-person keyword.
    if normalized_words
        .split_whitespace()
        .any(|word| NON_PERSON_WORDS.contains(&word))
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
    "about our",
    "attend now",
    "books short",
    "chief information officer",
    "driverless cars",
    "factory work",
    "financial firms",
    "freedom of information",
    "gender spectrum",
    "great initiative",
    "heart health",
    "higher education",
    "human body",
    "human experience",
    "intelligence augmentation",
    "machine learning",
    "mental illness",
    "public health",
    "session entity",
    "menu main",
    "public speaking",
    "series go",
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
    "annual report",
    "accreditations and certificates",
    "after sales",
    "british chamber",
    "box build",
    "box build assembly",
    "cable and wire harness assemblies",
    "cable assembly",
    "device manufacturers need to",
    "engineering support",
    "enforce tac",
    "entrepreneurs association",
    "environmental stewardship",
    "estonian defence",
    "financial director",
    "fideltronik headquarters",
    "firmware development",
    "high voltage",
    "international trade council",
    "johor bahru",
    "landkreises starnberg",
    "latency that",
    "magnetic assemblies",
    "data rates from",
    "costs that",
    "coverage gaps",
    "makes business sense",
    "maximizing quality",
    "mechanical engineering",
    "mobile assets",
    "module building",
    "north america",
    "oulu office",
    "pcb assembly",
    "pcb layout",
    "printed circuit board assembly",
    "precision plastics",
    "product maintenance",
    "production outsourcing",
    "pollution prevention",
    "quality over price",
    "remote sensor networks",
    "scanfil oyj",
    "stock exchange releases",
    "system integration",
    "terrestrial networks",
    "the bottom line",
    "the reality check",
    "the sweet spots where",
    "what drives us",
    "what exactly are these",
    "winning team",
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
    "annual general meeting",
    "the annual general meeting",
    "shareholder meeting",
    "new relic warning",
    "through december",
    "get in touch",
    "supplier day",
    "markets we serve",
    "our expertise",
    "financial reporting",
    "stock information",
    "our team",
    "who we are",
    "why creation",
    "advisory board",
    "key figures",
    "rapid prototyping",
    "press releases",
    "markets served",
    "quick links",
    "other links",
    "upcoming events",
    "latest news",
    "quality certifications",
    "workmanship standards",
    "rohs compliant",
    "safety achievements",
    "general enquiry",
    "important documents",
    "registration no",
    "registration number",
    "customize consent preferences",
    "new product introduction",
    "evertiq expo",
    "combined naval event",
    "life sciences",
    "office working hours",
    "job vacancies",
    "job vacancy",
    "key features",
    "kontaktieren sie",
    "clean energy",
    "end innovation",
    "emc lab",
    "sourced supplied smart",
    "supplier toolbox",
    "test design",
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
    "our companies",
    "privacy statement",
    "general terms",
    "privacy statement general terms",
    "terms service",
    "privacy policy",
    "cookie policy",
    "copyright notice",
];

/// Individual words that strongly indicate a non-person phrase.
static NON_PERSON_WORDS: &[&str] = &[
    // Website UI / navigation
    "about",
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
    "privacy",
    "statement",
    "terms",
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
    "mission",
    "market",
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
    "companies",
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
    "our",
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
    "purpose",
    "capabilities",
    "sustainability",
    "compliance",
    "diversity",
    "inclusion",
    "values",
    "vision",
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
    "committed",
    "communications",
    "executive",
    "professional",
    "integrated",
    "innovative",
    "creative",
    "dynamic",
    "crown",
    "conditions",
    "customer",
    "cutting",
    "added",
    "celebrates",
    "center",
    "group",
    "journey",
    "network",
    "marksmen",
    "tag",
    "tariffs",
    "manager",
    "your",
];

static TEMPORAL_PREFIX_WORDS: &[&str] = &[
    "through", "during", "until", "before", "after", "since", "from",
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

static ORG_LEADERSHIP_MARKETING_PHRASES: &[&str] = &[
    "connect with us",
    "manufacturer for industry",
    "building optics that matter",
    "celebrating world quality week",
    "watch us deploy automated",
    "watch us build",
];

static ORG_LEADERSHIP_SECTION_HEADING_PHRASES: &[&str] = &[
    "who we are",
    "why creation",
    "general enquiry",
    "important documents",
    "registration no",
    "registration number",
    "customize consent preferences",
    "new product introduction",
    "evertiq expo",
    "combined naval event",
    "life sciences",
    "office working hours",
    "job vacancies",
    "job vacancy",
    "kontaktieren sie",
    "clean energy",
    "end innovation",
    "quick links",
    "other links",
    "upcoming events",
    "latest news",
    "quality certifications",
    "workmanship standards",
    "rohs compliant",
    "safety achievements",
    "annual report",
    "at any stage",
    "automated labelling",
    "automated visual inspection system",
    "contact jaltek",
    "contacta con nosotros",
    "creating quality",
    "defence expo sweden",
    "engineering support",
    "from classroom",
    "full product assembly",
    "generation depaneling machine",
    "industry certified",
    "injection molding",
    "jaltek supports women",
    "juarez safety memo",
    "make uk defence",
    "markets we specialize in",
    "metal fabrication",
    "oulu office",
    "pide presupuesto",
    "printed circuit board assembly",
    "product maintenance",
    "production outsourcing",
    "quality has no boundaries",
    "rare combination",
    "read report",
    "sales inquiry",
    "scanfil code",
    "scanfil supplier code",
    "stock exchange releases",
    "stockholm office",
    "system integration",
    "tariff update",
    "vantaa office",
];

static ORG_LEADERSHIP_MARKETING_WORDS: &[&str] = &[
    "autonomous",
    "automated",
    "build",
    "building",
    "celebrating",
    "challenges",
    "communications",
    "connect",
    "deploy",
    "factories",
    "industry",
    "introducing",
    "manufacturer",
    "medical",
    "optics",
    "quality",
    "ramp",
    "smart",
    "story",
    "successfully",
    "week",
];

static ORG_LEADERSHIP_SECTION_HEADING_WORDS: &[&str] = &[
    "combined",
    "annual",
    "assembly",
    "association",
    "board",
    "chamber",
    "chambers",
    "certifications",
    "code",
    "compliant",
    "consent",
    "council",
    "creation",
    "customize",
    "document",
    "documents",
    "employers",
    "enquiry",
    "event",
    "events",
    "exchange",
    "expo",
    "fabrication",
    "figures",
    "hours",
    "important",
    "inspection",
    "introduction",
    "integration",
    "job",
    "jobs",
    "key",
    "labelling",
    "latest",
    "links",
    "maintenance",
    "molding",
    "news",
    "office",
    "outsourcing",
    "preferences",
    "product",
    "products",
    "report",
    "releases",
    "prototyping",
    "rapid",
    "registration",
    "rohs",
    "safety",
    "science",
    "sciences",
    "supplier",
    "suppliers",
    "standards",
    "support",
    "system",
    "team",
    "upcoming",
    "vacancies",
    "vacancy",
    "visual",
    "working",
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
    "companies",
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
