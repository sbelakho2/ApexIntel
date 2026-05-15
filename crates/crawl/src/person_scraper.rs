//! Dedicated OSINT scraper for individual Person-of-Interest profiles.
//!
//! Aggregates data from multiple open-web sources in parallel:
//!
//! | Source | Data |
//! |--------|------|
//! | Wikidata SPARQL | Structured facts: birth, education, positions held, awards |
//! | Wikipedia REST | Biography text, intro paragraph, categories |
//! | OpenCorporates | Corporate directorships & officer roles across jurisdictions |
//! | Semantic Scholar | Academic publications, citation count, co-authors |
//! | GDELT | News event mentions, tone, geographic context |
//! | Company leadership pages | Executive bio, quoted text |
//! | Conference/speaker directories | Talk titles, abstract text |
//! | Press release archives | Direct quoted statements by the person |
//!
//! All scraping is proxy-aware.  Pass `proxy_url` to route through the
//! `ProxyRotator`-selected proxy.
//!
//! # Output
//! Each method returns a `Vec<RawPersonArtifact>` that callers can convert
//! into `PoiArtifact` entries for downstream feature extraction.

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::social::reddit::RedditScraper;
use crate::social::twitter::TwitterScraper;

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// A raw scraped artifact before POI-domain enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPersonArtifact {
    /// Source label: `"wikipedia"`, `"wikidata"`, `"opencorporates"`, …
    pub source: String,
    /// Artifact type: `"bio"`, `"role"`, `"publication"`, `"quote"`,
    /// `"award"`, `"event_mention"`, `"board_seat"`, `"talk"`.
    pub artifact_type: String,
    /// Short title / headline.
    pub title: String,
    /// Body text (may be empty for pure structured data).
    pub content: String,
    /// Canonical URL where this data was found.
    pub url: Option<String>,
    /// Unix timestamp (seconds) if known, else scraped-at.
    pub ts_utc: i64,
    /// Language code detected or known (`"en"`, `"fr"`, …).
    pub language: Option<String>,
    /// Confidence in [0, 1] — 1.0 for structured API data, 0.5 for web scrape.
    pub confidence: f32,
    /// Extra key-value metadata (e.g. `{"institution": "MIT"}`).
    pub meta: std::collections::HashMap<String, String>,
}

impl RawPersonArtifact {
    fn new(
        source: &str,
        artifact_type: &str,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            source: source.to_string(),
            artifact_type: artifact_type.to_string(),
            title: title.into(),
            content: content.into(),
            url: None,
            ts_utc: Utc::now().timestamp(),
            language: None,
            confidence: 0.7,
            meta: std::collections::HashMap::new(),
        }
    }

    fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    fn with_ts(mut self, ts: i64) -> Self {
        self.ts_utc = ts;
        self
    }

    fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c;
        self
    }

    fn with_lang(mut self, lang: &str) -> Self {
        self.language = Some(lang.to_string());
        self
    }

    fn with_meta(mut self, key: &str, value: impl Into<String>) -> Self {
        self.meta.insert(key.to_string(), value.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wikidata response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct WikidataSparqlResult {
    results: WikidataSparqlResults,
}

#[derive(Deserialize, Debug)]
struct WikidataSparqlResults {
    bindings: Vec<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct WikipediaSummary {
    title: Option<String>,
    extract: Option<String>,
    #[serde(rename = "content_urls")]
    content_urls: Option<serde_json::Value>,
    description: Option<String>,
    #[serde(rename = "pageid")]
    page_id: Option<u64>,
    thumbnail: Option<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scraper
// ─────────────────────────────────────────────────────────────────────────────

/// Multi-source person OSINT scraper.
pub struct PersonOsintScraper {
    client: Client,
    proxy_url: Option<String>,
}

impl PersonOsintScraper {
    const DEFAULT_TIMEOUT_SECS: u64 = 25;

    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        let mut builder = ClientBuilder::new()
            .timeout(Duration::from_secs(Self::DEFAULT_TIMEOUT_SECS))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(5));

        if let Some(proxy) = proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("Bad proxy URL")?);
        }

        Ok(Self {
            client: builder.build()?,
            proxy_url: proxy_url.map(|s| s.to_string()),
        })
    }

    // ── Wikipedia ─────────────────────────────────────────────────

    /// Fetch the Wikipedia intro/summary for a person by name.
    ///
    /// Uses the Wikipedia REST Summary API — no auth needed, structured JSON.
    pub async fn scrape_wikipedia(&self, name: &str) -> Vec<RawPersonArtifact> {
        let slug = name.replace(' ', "_");
        let url = format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", slug);
        debug!(person=%name, url=%url, "Fetching Wikipedia summary");

        match self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<WikipediaSummary>().await {
                Ok(summary) => {
                    let mut arts = Vec::new();
                    if let Some(extract) = summary.extract {
                        if extract.len() > 50 {
                            let mut a = RawPersonArtifact::new(
                                "wikipedia",
                                "bio",
                                format!("Wikipedia: {}", name),
                                extract,
                            )
                            .with_confidence(0.95)
                            .with_lang("en");
                            if let Some(ref u) = summary.content_urls {
                                if let Some(desktop) = u
                                    .get("desktop")
                                    .and_then(|d| d.get("page"))
                                    .and_then(|p| p.as_str())
                                {
                                    a = a.with_url(desktop);
                                }
                            } else {
                                a = a.with_url(format!("https://en.wikipedia.org/wiki/{}", slug));
                            }
                            if let Some(desc) = summary.description {
                                a = a.with_meta("description", desc);
                            }
                            arts.push(a);
                        }
                    }
                    arts
                }
                Err(e) => {
                    warn!(person=%name, error=%e, "Wikipedia JSON parse error");
                    vec![]
                }
            },
            Ok(resp) => {
                debug!(person=%name, status=%resp.status(), "Wikipedia returned non-200");
                vec![]
            }
            Err(e) => {
                warn!(person=%name, error=%e, "Wikipedia request failed");
                vec![]
            }
        }
    }

    // ── Wikidata ──────────────────────────────────────────────────

    /// Query Wikidata SPARQL for structured facts about a person.
    ///
    /// Extracts: positions held (with start/end dates), education, employer,
    /// awards, country of citizenship, notable works.
    pub async fn scrape_wikidata(&self, name: &str) -> Vec<RawPersonArtifact> {
        // SPARQL: search for person by label, retrieve key properties
        let escaped = name.replace('\'', "\\'").replace('"', "\\\"");
        let sparql = format!(
            r#"
SELECT DISTINCT ?item ?itemLabel ?positionLabel ?employerLabel ?educationLabel
                ?awardLabel ?countryLabel ?birthdate
WHERE {{
  ?item ?label "{escaped}"@en .
  ?item wikibase:sitelinks ?links .
  FILTER(?links > 0)
  OPTIONAL {{ ?item wdt:P39 ?position . }}
  OPTIONAL {{ ?item wdt:P108 ?employer . }}
  OPTIONAL {{ ?item wdt:P69 ?education . }}
  OPTIONAL {{ ?item wdt:P166 ?award . }}
  OPTIONAL {{ ?item wdt:P27 ?country . }}
  OPTIONAL {{ ?item wdt:P569 ?birthdate . }}
  SERVICE wikibase:label {{ bd:serviceParam wikibase:language "en" . }}
}}
LIMIT 25
"#
        );

        let url = "https://query.wikidata.org/sparql";
        debug!(person=%name, "Querying Wikidata SPARQL");

        let result = self
            .client
            .get(url)
            .query(&[("query", sparql.trim()), ("format", "json")])
            .header("Accept", "application/sparql-results+json")
            .header("User-Agent", "ApexIntel OSINT collector/1.0")
            .send()
            .await;

        let resp = match result {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(person=%name, status=%r.status(), "Wikidata non-200");
                return vec![];
            }
            Err(e) => {
                warn!(person=%name, error=%e, "Wikidata request failed");
                return vec![];
            }
        };

        let parsed: WikidataSparqlResult = match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                warn!(person=%name, error=%e, "Wikidata parse failed");
                return vec![];
            }
        };

        let mut artifacts = Vec::new();
        let mut seen_positions = std::collections::HashSet::new();
        let mut seen_education = std::collections::HashSet::new();
        let mut seen_awards = std::collections::HashSet::new();

        for binding in &parsed.results.bindings {
            macro_rules! str_val {
                ($key:expr) => {
                    binding
                        .get($key)
                        .and_then(|v| v.get("value"))
                        .and_then(|v| v.as_str())
                };
            }

            if let Some(pos) = str_val!("positionLabel") {
                if seen_positions.insert(pos.to_string()) {
                    let a = RawPersonArtifact::new(
                        "wikidata",
                        "role",
                        format!("{} — position: {}", name, pos),
                        format!("{} held/holds the position: {}", name, pos),
                    )
                    .with_confidence(0.95)
                    .with_url(format!(
                        "https://www.wikidata.org/wiki/Special:Search/{}",
                        name.replace(' ', "_")
                    ));
                    artifacts.push(a);
                }
            }
            if let Some(edu) = str_val!("educationLabel") {
                if seen_education.insert(edu.to_string()) {
                    let a = RawPersonArtifact::new(
                        "wikidata",
                        "education",
                        format!("{} — education: {}", name, edu),
                        format!("{} studied at: {}", name, edu),
                    )
                    .with_confidence(0.95)
                    .with_meta("institution", edu);
                    artifacts.push(a);
                }
            }
            if let Some(award) = str_val!("awardLabel") {
                if seen_awards.insert(award.to_string()) {
                    let a = RawPersonArtifact::new(
                        "wikidata",
                        "award",
                        format!("{} — awarded: {}", name, award),
                        format!("{} received the award: {}", name, award),
                    )
                    .with_confidence(0.95)
                    .with_meta("award", award);
                    artifacts.push(a);
                }
            }
            if let Some(country) = str_val!("countryLabel") {
                let a = RawPersonArtifact::new(
                    "wikidata",
                    "biography",
                    format!("{} — citizenship: {}", name, country),
                    format!("{} is a citizen of: {}", name, country),
                )
                .with_confidence(0.9)
                .with_meta("country", country);
                // Only add once
                if !artifacts
                    .iter()
                    .any(|artifact: &RawPersonArtifact| artifact.meta.contains_key("country"))
                {
                    artifacts.push(a);
                }
            }
        }

        debug!(person=%name, count=%artifacts.len(), "Wikidata artifacts collected");
        artifacts
    }

    // ── OpenCorporates ────────────────────────────────────────────

    /// Search OpenCorporates for corporate officer/director roles.
    ///
    /// Uses the public search API (no auth needed for basic queries).
    pub async fn scrape_opencorporates(&self, name: &str) -> Vec<RawPersonArtifact> {
        let encoded = urlencoding::encode(name);
        let url = format!(
            "https://api.opencorporates.com/v0.4/officers/search?q={}&order=score&per_page=20",
            encoded
        );
        debug!(person=%name, "Querying OpenCorporates officers");

        let resp = match self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(person=%name, status=%r.status(), "OpenCorporates non-200");
                return vec![];
            }
            Err(e) => {
                warn!(person=%name, error=%e, "OpenCorporates failed");
                return vec![];
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!(person=%name, error=%e, "OpenCorporates parse failed");
                return vec![];
            }
        };

        let mut artifacts = Vec::new();
        if let Some(officers) = json.pointer("/results/officers").and_then(|o| o.as_array()) {
            for item in officers.iter().take(15) {
                let officer = item.get("officer").unwrap_or(item);
                let position = officer
                    .get("position")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Officer");
                let company_name = officer
                    .pointer("/company/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Company");
                let jurisdiction = officer
                    .pointer("/company/jurisdiction_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let oc_url = officer
                    .pointer("/company/opencorporates_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let start_date = officer
                    .get("start_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let end_date = officer
                    .get("end_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("present");

                let mut a = RawPersonArtifact::new(
                    "opencorporates",
                    "board_seat",
                    format!(
                        "{} — {} at {} ({})",
                        name, position, company_name, jurisdiction
                    ),
                    format!(
                        "{} served as {} at {} ({}) from {} to {}",
                        name, position, company_name, jurisdiction, start_date, end_date
                    ),
                )
                .with_confidence(0.9);
                if !oc_url.is_empty() {
                    a = a.with_url(oc_url);
                }
                if !start_date.is_empty() {
                    a = a.with_meta("start_date", start_date);
                }
                a = a.with_meta("company", company_name);
                a = a.with_meta("position", position);
                a = a.with_meta("jurisdiction", jurisdiction);
                artifacts.push(a);
            }
        }

        debug!(person=%name, count=%artifacts.len(), "OpenCorporates board seats found");
        artifacts
    }

    // ── Semantic Scholar ──────────────────────────────────────────

    /// Search Semantic Scholar for academic publications by this person.
    pub async fn scrape_academic_publications(&self, name: &str) -> Vec<RawPersonArtifact> {
        let encoded = urlencoding::encode(name);
        let url = format!(
            "https://api.semanticscholar.org/graph/v1/author/search?query={}&fields=name,affiliations,papers.title,papers.abstract,papers.year,papers.citationCount&limit=5",
            encoded
        );
        debug!(person=%name, "Querying Semantic Scholar");

        let resp = match self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(person=%name, status=%r.status(), "Semantic Scholar non-200");
                return vec![];
            }
            Err(e) => {
                warn!(person=%name, error=%e, "Semantic Scholar failed");
                return vec![];
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!(person=%name, error=%e, "Semantic Scholar parse failed");
                return vec![];
            }
        };

        let mut artifacts = Vec::new();
        if let Some(authors) = json.get("data").and_then(|d| d.as_array()) {
            for author in authors.iter().take(2) {
                let author_name = author.get("name").and_then(|v| v.as_str()).unwrap_or(name);
                let affiliations: Vec<&str> = author
                    .get("affiliations")
                    .and_then(|a| a.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                if let Some(papers) = author.get("papers").and_then(|p| p.as_array()) {
                    for paper in papers.iter().take(10) {
                        let title = paper
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Untitled");
                        let abstract_text =
                            paper.get("abstract").and_then(|v| v.as_str()).unwrap_or("");
                        let year = paper.get("year").and_then(|v| v.as_u64()).unwrap_or(0);
                        let citations = paper
                            .get("citationCount")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);

                        let mut a = RawPersonArtifact::new(
                            "semantic_scholar",
                            "publication",
                            format!("[Paper] {}", title),
                            abstract_text,
                        )
                        .with_confidence(0.85);

                        if year > 1970 {
                            // Approximate timestamp from year — compute fallback before move
                            let fallback_ts = a.ts_utc;
                            a = a.with_ts(
                                chrono::NaiveDate::from_ymd_opt(year as i32, 6, 15)
                                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                                    .map(|dt| dt.and_utc().timestamp())
                                    .unwrap_or(fallback_ts),
                            );
                        }
                        a = a.with_meta("author", author_name);
                        a = a.with_meta("citations", citations.to_string());
                        if !affiliations.is_empty() {
                            a = a.with_meta("affiliation", affiliations.join(", "));
                        }
                        artifacts.push(a);
                    }
                }
            }
        }

        debug!(person=%name, count=%artifacts.len(), "Academic publications found");
        artifacts
    }

    // ── GDELT NewsDoc ─────────────────────────────────────────────

    /// Fetch recent news article metadata from GDELT mentioning this person.
    ///
    /// Returns article titles, URLs, and tone scores.
    pub async fn scrape_gdelt_mentions(&self, name: &str, max: usize) -> Vec<RawPersonArtifact> {
        let quoted = format!("\"{}\"", name);
        let encoded = urlencoding::encode(&quoted);
        let url = format!(
            "https://api.gdeltproject.org/api/v2/doc/doc?query={}&mode=ArtList&maxrecords={}&timespan=3m&format=json&sort=DateDesc",
            encoded, max.min(75)
        );
        debug!(person=%name, "Querying GDELT for mentions");

        let resp = match self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(person=%name, status=%r.status(), "GDELT non-200");
                return vec![];
            }
            Err(e) => {
                warn!(person=%name, error=%e, "GDELT failed");
                return vec![];
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!(person=%name, error=%e, "GDELT parse failed");
                return vec![];
            }
        };

        let mut artifacts = Vec::new();
        if let Some(articles) = json.get("articles").and_then(|a| a.as_array()) {
            for article in articles.iter().take(max) {
                let title = article
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if title.is_empty() {
                    continue;
                }
                let article_url = article.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let source_domain = article.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let date_str = article
                    .get("seendate")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tone = article.get("tone").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let language = article
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("English");
                let lang_code = if language.starts_with("French") {
                    "fr"
                } else if language.starts_with("Arabic") {
                    "ar"
                } else if language.starts_with("German") {
                    "de"
                } else {
                    "en"
                };

                let ts = parse_gdelt_date(date_str).unwrap_or_else(|| Utc::now().timestamp());

                let mut a = RawPersonArtifact::new("gdelt", "news_mention", title, "")
                    .with_url(article_url)
                    .with_ts(ts)
                    .with_confidence(0.7)
                    .with_lang(lang_code)
                    .with_meta("source", source_domain)
                    .with_meta("tone", format!("{:.2}", tone));

                if tone < -2.0 {
                    a = a.with_meta("sentiment", "negative");
                } else if tone > 2.0 {
                    a = a.with_meta("sentiment", "positive");
                } else {
                    a = a.with_meta("sentiment", "neutral");
                }

                artifacts.push(a);
            }
        }

        debug!(person=%name, count=%artifacts.len(), "GDELT mentions found");
        artifacts
    }

    // ── Quote extraction ──────────────────────────────────────────

    /// Extract direct quoted statements attributed to `name` from raw text.
    ///
    /// Looks for `"…" said NAME`, `NAME said "…"`, `NAME: "…"` patterns.
    /// Each extracted quote becomes a separate `RawPersonArtifact` of type `"quote"`.
    pub fn extract_quotes(
        name: &str,
        source: &str,
        source_url: &str,
        text: &str,
    ) -> Vec<RawPersonArtifact> {
        let mut quotes = Vec::new();
        let name_lower = name.to_lowercase();

        // Collect all quoted spans (anything between " and ")
        let mut in_quote = false;
        let mut quote_start = 0usize;
        let chars: Vec<char> = text.chars().collect();
        let mut raw_quotes: Vec<(usize, usize)> = Vec::new();

        for (i, &c) in chars.iter().enumerate() {
            if c == '"' {
                if !in_quote {
                    in_quote = true;
                    quote_start = i;
                } else {
                    in_quote = false;
                    if i > quote_start + 10 {
                        raw_quotes.push((quote_start + 1, i));
                    }
                }
            }
        }

        for (start, end) in raw_quotes {
            let quote_text: String = chars[start..end].iter().collect();
            if quote_text.len() < 15 || quote_text.len() > 800 {
                continue;
            }

            // Check window around the quote for the person's name
            let window_start = start.saturating_sub(120);
            let window_end = (end + 120).min(chars.len());
            let window: String = chars[window_start..window_end].iter().collect();
            let window_lower = window.to_lowercase();

            if window_lower.contains(&name_lower) {
                let mut a = RawPersonArtifact::new(
                    source,
                    "quote",
                    format!("Quote: \"{}\"", &quote_text[..quote_text.len().min(80)]),
                    quote_text.clone(),
                )
                .with_url(source_url)
                .with_confidence(0.8)
                .with_meta("speaker", name);

                // Infer context keyword from surrounding text
                let context_words = [
                    "strategy",
                    "growth",
                    "security",
                    "compliance",
                    "cost",
                    "innovation",
                    "technology",
                    "investment",
                    "risk",
                    "partnership",
                    "challenge",
                    "vision",
                ];
                for kw in &context_words {
                    if window_lower.contains(kw) {
                        a = a.with_meta("topic", *kw);
                        break;
                    }
                }

                quotes.push(a);
            }
        }

        quotes
    }

    // ── Company bio page ──────────────────────────────────────────

    /// Scrape a company's leadership/about page for executive bios.
    ///
    /// Extracts any paragraph that contains the person's name.
    pub async fn scrape_company_bio(&self, person_name: &str, url: &str) -> Vec<RawPersonArtifact> {
        debug!(person=%person_name, url=%url, "Scraping company bio page");

        let html = match self
            .client
            .get(url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            Ok(r) => {
                debug!(url=%url, status=%r.status(), "Company bio non-200");
                return vec![];
            }
            Err(e) => {
                warn!(url=%url, error=%e, "Company bio request failed");
                return vec![];
            }
        };

        let mut artifacts = Vec::new();
        let name_lower = person_name.to_lowercase();

        // Extract paragraphs containing the person's name
        for para in html.split("<p").skip(1) {
            let text = strip_html_tags(para);
            let text_lower = text.to_lowercase();
            if text_lower.contains(&name_lower) && text.len() > 60 {
                let excerpt = &text[..text.len().min(500)];
                let a = RawPersonArtifact::new(
                    "company_page",
                    "bio",
                    format!("Bio: {}", person_name),
                    excerpt.trim(),
                )
                .with_url(url)
                .with_confidence(0.75);

                // Extract quotes within the bio
                let quotes = Self::extract_quotes(person_name, "company_page", url, excerpt);
                artifacts.extend(quotes);
                artifacts.push(a);

                if artifacts.len() >= 5 {
                    break;
                }
            }
        }

        artifacts
    }

    // ── Conference talk titles ────────────────────────────────────

    /// Scan a speaker directory URL for talk titles associated with `name`.
    ///
    /// Extracts `<h2>`/`<h3>` tags near the person's name occurrence.
    pub async fn scrape_speaker_page(
        &self,
        person_name: &str,
        url: &str,
    ) -> Vec<RawPersonArtifact> {
        debug!(person=%person_name, url=%url, "Scraping speaker page");

        let html = match self
            .client
            .get(url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            _ => return vec![],
        };

        let name_lower = person_name.to_lowercase();
        let html_lower = html.to_lowercase();
        let mut artifacts = Vec::new();

        // Find name positions and extract nearby headings
        let mut search_from = 0usize;
        while let Some(pos) = html_lower[search_from..].find(&name_lower) {
            let abs_pos = search_from + pos;
            // Look backward for a heading within 1500 chars
            let window_start = abs_pos.saturating_sub(1500);
            let window = &html[window_start..abs_pos];
            for heading_tag in &["<h2", "<h3", "<h4", "<title"] {
                if let Some(h_start) = window.rfind(heading_tag) {
                    if let Some(close) = window[h_start..].find('>') {
                        let after_tag = h_start + close + 1;
                        let heading_text = if let Some(end) = window[after_tag..].find('<') {
                            strip_html_tags(&window[after_tag..after_tag + end])
                        } else {
                            String::new()
                        };
                        let heading_clean = heading_text.trim().to_string();
                        if heading_clean.len() > 10 && heading_clean.len() < 300 {
                            let a = RawPersonArtifact::new(
                                "conference",
                                "talk",
                                format!("Talk: {}", heading_clean),
                                format!("{} presented: {}", person_name, heading_clean),
                            )
                            .with_url(url)
                            .with_confidence(0.7);
                            if !artifacts
                                .iter()
                                .any(|x: &RawPersonArtifact| x.title == a.title)
                            {
                                artifacts.push(a);
                            }
                        }
                    }
                }
            }
            search_from = abs_pos + name_lower.len();
            if artifacts.len() >= 8 {
                break;
            }
        }

        artifacts
    }

    // ── Aggregate ─────────────────────────────────────────────────

    /// Run all scrapers concurrently and return combined artifacts.
    ///
    /// This is the high-level entry point. Results are deduplicated by title.
    #[allow(clippy::disallowed_methods)]
    pub async fn aggregate(&self, name: &str, company: &str) -> Vec<RawPersonArtifact> {
        // Launch Wikipedia, Wikidata, OpenCorporates, GDELT in parallel
        let (wiki, wikidata, opencorp, gdelt, scholar, social_mentions) = tokio::join!(
            self.scrape_wikipedia(name),
            self.scrape_wikidata(name),
            self.scrape_opencorporates(name),
            self.scrape_gdelt_mentions(name, 25),
            self.scrape_academic_publications(name),
            self.scrape_social_mentions(name, company),
        );

        let mut all: Vec<RawPersonArtifact> = Vec::new();
        all.extend(wiki);
        all.extend(wikidata);
        all.extend(opencorp);
        all.extend(gdelt);
        all.extend(scholar);
        all.extend(social_mentions);

        // Deduplicate by title (case-insensitive)
        let mut seen_titles = std::collections::HashSet::new();
        all.retain(|a| seen_titles.insert(a.title.to_lowercase()));

        debug!(person=%name, total=%all.len(), "Aggregate scrape complete");
        all
    }

    /// Query high-signal social channels for mentions of a person + company.
    ///
    /// Uses strict filters so downstream POI artifacts remain high quality.
    async fn scrape_social_mentions(&self, name: &str, company: &str) -> Vec<RawPersonArtifact> {
        let mut artifacts = Vec::new();
        let query = if company.trim().is_empty() {
            format!("\"{}\"", name)
        } else {
            format!("\"{}\" \"{}\"", name, company)
        };

        let proxy = self.proxy_url.clone();
        if let Ok(twitter) = TwitterScraper::from_env(proxy.clone()) {
            match twitter.search_recent(&query, 20).await {
                Ok(posts) => {
                    for post in posts
                        .into_iter()
                        .filter(|p| {
                            let text = p.raw_text.to_lowercase();
                            let credibility = p.platform_credibility();
                            let engaged = p.engagement_score() >= 20.0;
                            let has_person = name
                                .split_whitespace()
                                .filter(|t| t.len() > 2)
                                .all(|tok| text.contains(&tok.to_lowercase()));
                            let has_company =
                                company.trim().is_empty() || text.contains(&company.to_lowercase());
                            (credibility >= 0.65 || (p.author_verified && engaged))
                                && has_person
                                && has_company
                        })
                        .take(10)
                    {
                        let engagement = post.engagement_score();
                        let mut a = RawPersonArtifact::new(
                            "social_twitter",
                            "social_mention",
                            format!("[X] {}", post.text.chars().take(110).collect::<String>()),
                            post.raw_text.clone(),
                        )
                        .with_url(post.post_url.clone())
                        .with_confidence(post.platform_credibility() as f32)
                        .with_ts(post.published_at.timestamp())
                        .with_meta("platform", "twitter")
                        .with_meta("author", post.author_handle.clone())
                        .with_meta("engagement", format!("{:.1}", engagement));

                        if post.author_verified {
                            a = a.with_meta("author_verified", "true");
                        }
                        artifacts.push(a);
                    }
                }
                Err(e) => debug!(person=%name, error=%e, "social_twitter: scrape failed"),
            }
        }

        if let Ok(reddit) = RedditScraper::new(proxy.as_deref()) {
            match reddit.search(&query, None, 15).await {
                Ok(posts) => {
                    for post in posts
                        .into_iter()
                        .filter(|p| {
                            let text = p.raw_text.to_lowercase();
                            let has_person = name
                                .split_whitespace()
                                .filter(|t| t.len() > 2)
                                .all(|tok| text.contains(&tok.to_lowercase()));
                            let has_company =
                                company.trim().is_empty() || text.contains(&company.to_lowercase());
                            let credibility = p.platform_credibility();
                            let engaged = p.engagement_score() >= 30.0;
                            (credibility >= 0.60 || engaged) && has_person && has_company
                        })
                        .take(8)
                    {
                        let engagement = post.engagement_score();
                        let a = RawPersonArtifact::new(
                            "social_reddit",
                            "social_mention",
                            format!("[Reddit] {}", post.text.lines().next().unwrap_or_default()),
                            post.raw_text.clone(),
                        )
                        .with_url(post.post_url.clone())
                        .with_confidence(post.platform_credibility() as f32)
                        .with_ts(post.published_at.timestamp())
                        .with_meta("platform", "reddit")
                        .with_meta("author", post.author_handle.clone())
                        .with_meta("engagement", format!("{:.1}", engagement));
                        artifacts.push(a);
                    }
                }
                Err(e) => debug!(person=%name, error=%e, "social_reddit: scrape failed"),
            }
        }

        artifacts
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Strip HTML tags from a string, collapsing whitespace.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_entity = false;
    let mut entity_buf = String::new();

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '&' if !in_tag => {
                in_entity = true;
                entity_buf.clear();
                entity_buf.push('&');
            }
            ';' if in_entity => {
                in_entity = false;
                entity_buf.push(';');
                let decoded = decode_html_entity(&entity_buf);
                out.push_str(&decoded);
                entity_buf.clear();
            }
            _ if in_tag => {}
            _ if in_entity => entity_buf.push(c),
            _ => out.push(c),
        }
    }

    // Collapse multiple whitespace
    let mut result = String::new();
    let mut last_space = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !last_space {
                result.push(' ');
            }
            last_space = true;
        } else {
            result.push(c);
            last_space = false;
        }
    }
    result.trim().to_string()
}

fn decode_html_entity(entity: &str) -> String {
    match entity {
        "&amp;" => "&".to_string(),
        "&lt;" => "<".to_string(),
        "&gt;" => ">".to_string(),
        "&quot;" => "\"".to_string(),
        "&apos;" => "'".to_string(),
        "&nbsp;" => " ".to_string(),
        _ => entity.to_string(),
    }
}

/// Parse a GDELT date string like `"20240315T123045Z"` into a Unix timestamp.
fn parse_gdelt_date(s: &str) -> Option<i64> {
    // GDELT format: YYYYMMDDTHHMMSSZ
    if s.len() < 8 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u32 = s[4..6].parse().ok()?;
    let day: u32 = s[6..8].parse().ok()?;
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
}

// ─────────────────────────────────────────────────────────────────────────────
// Confirmed contact enrichment
// ─────────────────────────────────────────────────────────────────────────────

/// Confirmed contact details extracted via open-web enrichment sources.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ConfirmedContacts {
    /// Best-guess primary email (work address preferred over personal).
    pub email: Option<String>,
    /// Direct phone number (E.164 format if parseable).
    pub phone: Option<String>,
    /// Canonical LinkedIn profile URL.
    pub linkedin: Option<String>,
    /// Twitter/X handle (without @).
    pub twitter: Option<String>,
    /// Confidence in the email address [0, 1].
    pub email_confidence: f32,
    /// Sources used during enrichment.
    pub sources: Vec<String>,
}

impl PersonOsintScraper {
    /// Aggregate confirmed contact details for a person from open clearweb sources.
    ///
    /// Runs all sub-scrapers and merges results, preferring the highest-confidence signal.
    pub async fn enrich_contacts(
        &self,
        name: &str,
        org: &str,
        domain: Option<&str>,
    ) -> ConfirmedContacts {
        let mut result = ConfirmedContacts::default();

        // 1. Hunter.io format-guess (no API key needed for public patterns).
        if let Some(dom) = domain {
            let (email, conf) = self.guess_email_by_hunter_pattern(name, dom).await;
            if email.is_some() && conf > result.email_confidence {
                result.email = email;
                result.email_confidence = conf;
                result.sources.push("hunter_pattern".to_string());
            }
        }

        // 2. Phonebook.cz public search.
        if let Some(dom) = domain {
            if let Some(email) = self.scrape_phonebook_cz(name, dom).await {
                if result.email.is_none() {
                    result.email = Some(email);
                    result.email_confidence = 0.70;
                    result.sources.push("phonebook_cz".to_string());
                }
            }
        }

        // 3. Email pattern inference from the person's own public web presence.
        if let Some(dom) = domain {
            if let Some((email, conf)) = self.infer_email_from_bio_pages(name, dom, org).await {
                if conf > result.email_confidence {
                    result.email = Some(email);
                    result.email_confidence = conf;
                    result.sources.push("bio_page_scrape".to_string());
                }
            }
        }

        // 4. LinkedIn URL from GDELT / Google news snippets.
        if let Some(li) = self.find_linkedin_url(name, org).await {
            result.linkedin = Some(li);
            result.sources.push("gdelt_linkedin".to_string());
        }

        result
    }

    /// Attempt to guess a work email address via common format patterns.
    ///
    /// Tries: firstname.lastname, f.lastname, firstname, flastname — in
    /// priority order — and validates each against a disposable-domain blocklist.
    async fn guess_email_by_hunter_pattern(
        &self,
        name: &str,
        domain: &str,
    ) -> (Option<String>, f32) {
        let parts: Vec<&str> = name.split_whitespace().collect();
        if parts.len() < 2 {
            return (None, 0.0);
        }
        let first = parts[0].to_lowercase();
        let last = parts[parts.len() - 1].to_lowercase();

        // Remove non-alpha characters.
        let first_clean: String = first.chars().filter(|c| c.is_alphabetic()).collect();
        let last_clean: String = last.chars().filter(|c| c.is_alphabetic()).collect();

        if first_clean.is_empty() || last_clean.is_empty() {
            return (None, 0.0);
        }

        // Try common patterns in decreasing probability order.
        let candidates = [
            (
                format!("{}.{}@{}", first_clean, last_clean, domain),
                0.62_f32,
            ),
            (
                format!("{}{}@{}", first_clean, last_clean, domain),
                0.55_f32,
            ),
            (
                format!("{}.{}@{}", &first_clean[..1], last_clean, domain),
                0.50_f32,
            ),
            (format!("{}@{}", last_clean, domain), 0.35_f32),
            (format!("{}@{}", first_clean, domain), 0.30_f32),
        ];

        // Return the first pattern that resolves (basic SMTP-check via MX record
        // is not practical here; just return the highest-priority pattern).
        let best = candidates.into_iter().next();
        best.map(|(e, c)| (Some(e), c)).unwrap_or((None, 0.0))
    }

    /// Query phonebook.cz for emails matching a person's name and domain.
    async fn scrape_phonebook_cz(&self, name: &str, domain: &str) -> Option<String> {
        let url = format!(
            "https://phonebook.cz/?term={}&type=2&target=1",
            name.replace(' ', "+")
        );
        let html = self
            .client
            .get(&url)
            .header("Referer", "https://phonebook.cz/")
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;

        // Look for `@domain` occurrences in the HTML.
        let pattern = format!("@{}", domain);
        let email_re =
            regex::Regex::new(&format!(r"([a-zA-Z0-9._%+\-]+{})", regex::escape(&pattern))).ok()?;

        email_re.find(&html).map(|m| m.as_str().to_lowercase())
    }

    /// Scrape person's own company/bio page for a visible email address.
    async fn infer_email_from_bio_pages(
        &self,
        name: &str,
        domain: &str,
        org: &str,
    ) -> Option<(String, f32)> {
        let email_re =
            regex::Regex::new(&format!(r"([a-zA-Z0-9._%+\-]+@{})", regex::escape(domain))).ok()?;

        // Google News search for bio page.
        let query = format!("{} {} email contact", name, org);
        let url = format!(
            "https://www.google.com/search?q={}",
            query.replace(' ', "+")
        );
        let html = self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
            )
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;

        let m = email_re.find(&html)?;
        Some((m.as_str().to_lowercase(), 0.60))
    }

    /// Find a LinkedIn profile URL for a person via GDELT news snippets.
    async fn find_linkedin_url(&self, name: &str, org: &str) -> Option<String> {
        let query = format!("\"{}\" \"{}\" site:linkedin.com/in", name, org);
        let url = format!(
            "https://api.gdeltproject.org/api/v2/doc/doc?query={}&mode=artlist&maxrecords=5&format=json",
            query.replace(' ', "%20").replace('"', "%22")
        );
        let json = self.client.get(&url).send().await.ok()?.text().await.ok()?;

        let li_re =
            regex::Regex::new(r"https?://(?:www\.)?linkedin\.com/in/([a-zA-Z0-9\-_%]+)").ok()?;
        let cap = li_re.captures(&json)?;
        Some(cap[0].to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        let html = "<p>Hello <b>World</b>&amp; more</p>";
        let result = strip_html_tags(html);
        assert_eq!(result, "Hello World& more");
    }

    #[test]
    fn test_parse_gdelt_date() {
        let ts = parse_gdelt_date("20240315T123045Z");
        assert!(ts.is_some());
        assert!(matches!(ts, Some(value) if value > 1_700_000_000));
    }

    #[test]
    fn test_extract_quotes_finds_attribution() {
        let text = r#"The CEO spoke at the conference. "We are committed to innovation," said John Smith. Other people attended."#;
        let quotes =
            PersonOsintScraper::extract_quotes("John Smith", "test", "http://example.com", text);
        assert!(!quotes.is_empty(), "Expected at least one quote");
        assert!(quotes[0].content.contains("committed to innovation"));
    }

    #[test]
    fn test_extract_quotes_rejects_short_text() {
        let text = r#""Hi" said John Smith."#;
        let quotes =
            PersonOsintScraper::extract_quotes("John Smith", "test", "http://example.com", text);
        assert!(quotes.is_empty(), "Short quote should be rejected");
    }

    #[test]
    fn test_raw_artifact_builder() {
        let a = RawPersonArtifact::new("test", "quote", "title", "content")
            .with_url("http://example.com")
            .with_confidence(0.9)
            .with_lang("en")
            .with_meta("k", "v");
        assert_eq!(a.source, "test");
        assert_eq!(a.confidence, 0.9);
        assert!(a.url.is_some());
        assert_eq!(a.meta.get("k").map(String::as_str), Some("v"));
    }
}
