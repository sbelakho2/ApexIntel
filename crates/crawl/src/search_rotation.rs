//! Search engine rotation for OSINT crawling.
//!
//! Provides a 27-engine rotation pool that spreads queries across providers
//! to avoid rate limits, reduce fingerprinting, and improve recall.  Each
//! [`SearchEngine`] declares its query template, result selector hints, and
//! backoff policy.
//!
//! # Usage pattern
//! ```
//! use apex_crawl::search_rotation::SearchPool;
//! let pool = SearchPool::default();
//! let engine = pool.next().unwrap();               // round-robin selection
//! let url = engine.build_url("CBRN export controls Israel");
//! println!("Fetching: {url}");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::governor_limiter::CrawlGovernor;
use crate::rate_limit::RateLimitManager;

// ─────────────────────────────────────────────────────────────────────────────
// Engine definition
// ─────────────────────────────────────────────────────────────────────────────

/// The type of search provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    /// Classic web search (Google, Bing, DDG…)
    WebSearch,
    /// News-specific aggregator (Google News, Newsnow…)
    NewsAggregator,
    /// Developer / code-hosting search
    CodeSearch,
    /// Academic / paper search
    AcademicSearch,
    /// Government / legal document search
    GovernmentRegistry,
    /// Patent search
    PatentSearch,
    /// Social listening
    SocialSearch,
    /// Dark-web / forum OSINT
    ForumSearch,
}

/// A single search engine with its query-construction rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEngine {
    /// Machine-readable identifier, e.g. `"google"`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Base URL prefix; `{query}` is replaced with the URL-encoded search term.
    pub url_template: String,
    /// Kind of search.
    pub kind: EngineKind,
    /// Maximum requests per minute before the engine starts blocking.
    pub rate_limit_rpm: u32,
    /// Whether a proxy is advisable to avoid geo-blocks.
    pub use_proxy: bool,
    /// CSS selector for result link elements (for HTML scraping fallback).
    pub result_link_selector: Option<String>,
    /// Whether this engine supports JSON API responses.
    pub json_api: bool,
    /// Whether the engine is currently enabled in the pool.
    pub enabled: bool,
}

impl SearchEngine {
    fn new(id: &str, name: &str, url_template: &str, kind: EngineKind, rpm: u32) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            url_template: url_template.to_string(),
            kind,
            rate_limit_rpm: rpm,
            use_proxy: false,
            result_link_selector: None,
            json_api: false,
            enabled: true,
        }
    }

    fn proxy(mut self) -> Self {
        self.use_proxy = true;
        self
    }

    fn selector(mut self, sel: &str) -> Self {
        self.result_link_selector = Some(sel.to_string());
        self
    }

    fn json_api(mut self) -> Self {
        self.json_api = true;
        self
    }

    /// Build the search URL for a given query string.
    ///
    /// The placeholder `{query}` is replaced with the URL-encoded query.
    pub fn build_url(&self, query: &str) -> String {
        let encoded = urlencoding_simple(query);
        self.url_template.replace("{query}", &encoded)
    }
}

/// Minimal URL encoder (replaces spaces with `+` and encodes special chars).
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            ' ' => out.push('+'),
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                for b in c.to_string().as_bytes() {
                    out.push('%');
                    out.push_str(&format!("{:02X}", b));
                }
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Engine catalogue
// ─────────────────────────────────────────────────────────────────────────────

/// Return the full list of supported search engines.
pub fn all_engines() -> Vec<SearchEngine> {
    vec![
        // ── General Web ──────────────────────────────────────────
        SearchEngine::new(
            "google",
            "Google Web",
            "https://www.google.com/search?q={query}&num=10",
            EngineKind::WebSearch,
            10,
        ).selector("div.g a[href]").proxy(),

        SearchEngine::new(
            "bing",
            "Microsoft Bing",
            "https://www.bing.com/search?q={query}",
            EngineKind::WebSearch,
            20,
        ).selector("li.b_algo h2 a"),

        SearchEngine::new(
            "duckduckgo",
            "DuckDuckGo",
            "https://duckduckgo.com/html/?q={query}",
            EngineKind::WebSearch,
            30,
        ).selector("a.result__a"),

        SearchEngine::new(
            "brave_search",
            "Brave Search",
            "https://search.brave.com/search?q={query}",
            EngineKind::WebSearch,
            30,
        ).selector("a[data-pos]"),

        SearchEngine::new(
            "yandex",
            "Yandex",
            "https://yandex.com/search/?text={query}",
            EngineKind::WebSearch,
            20,
        ).selector("li.serp-item a.link").proxy(),

        SearchEngine::new(
            "baidu",
            "Baidu",
            "https://www.baidu.com/s?wd={query}",
            EngineKind::WebSearch,
            15,
        ).selector("div.result h3 a").proxy(),

        SearchEngine::new(
            "ecosia",
            "Ecosia",
            "https://www.ecosia.org/search?q={query}",
            EngineKind::WebSearch,
            30,
        ),

        SearchEngine::new(
            "startpage",
            "Startpage (privacy proxy to Google)",
            "https://www.startpage.com/search?q={query}",
            EngineKind::WebSearch,
            20,
        ).selector("a.result-link"),

        SearchEngine::new(
            "metager",
            "MetaGer (German multi-engine)",
            "https://metager.de/meta/meta.ger3?eingabe={query}&lang=en",
            EngineKind::WebSearch,
            15,
        ),

        SearchEngine::new(
            "mojeek",
            "Mojeek (independent crawler)",
            "https://www.mojeek.com/search?q={query}",
            EngineKind::WebSearch,
            30,
        ).selector("a.ob"),

        // ── News Aggregators ──────────────────────────────────────
        SearchEngine::new(
            "google_news",
            "Google News",
            "https://news.google.com/search?q={query}",
            EngineKind::NewsAggregator,
            10,
        ).proxy(),

        SearchEngine::new(
            "bing_news",
            "Bing News",
            "https://www.bing.com/news/search?q={query}",
            EngineKind::NewsAggregator,
            20,
        ).selector("a.news-card-body-cards-container"),

        SearchEngine::new(
            "duckduckgo_news",
            "DuckDuckGo News",
            "https://duckduckgo.com/news.js?q={query}&kl=wt-wt",
            EngineKind::NewsAggregator,
            30,
        ).json_api(),

        SearchEngine::new(
            "newsnow",
            "NewsNow",
            "https://www.newsnow.co.uk/h/World+News?search={query}",
            EngineKind::NewsAggregator,
            10,
        ),

        SearchEngine::new(
            "gdelt_project",
            "GDELT Event DB",
            "https://api.gdeltproject.org/api/v2/doc/doc?query={query}&mode=ArtList&maxrecords=75&format=json",
            EngineKind::NewsAggregator,
            60,
        ).json_api(),

        SearchEngine::new(
            "event_registry",
            "EventRegistry News API",
            "https://eventregistry.org/api/v1/article/getArticles?keyword={query}&articlesSortBy=rel&articlesCount=20",
            EngineKind::NewsAggregator,
            30,
        ).json_api(),

        SearchEngine::new(
            "mediastack",
            "MediaStack News API",
            "https://api.mediastack.com/v1/news?keywords={query}&limit=25",
            EngineKind::NewsAggregator,
            60,
        ).json_api(),

        // ── Academic ──────────────────────────────────────────────
        SearchEngine::new(
            "semantic_scholar",
            "Semantic Scholar",
            "https://api.semanticscholar.org/graph/v1/paper/search?query={query}&limit=10&fields=title,abstract,year,authors,externalIds",
            EngineKind::AcademicSearch,
            100,
        ).json_api(),

        SearchEngine::new(
            "crossref",
            "CrossRef DOI Search",
            "https://api.crossref.org/works?query={query}&rows=25&select=DOI,title,abstract,published,author",
            EngineKind::AcademicSearch,
            50,
        ).json_api(),

        SearchEngine::new(
            "pubmed",
            "PubMed (medical/life sciences)",
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&retmode=json&term={query}&retmax=25",
            EngineKind::AcademicSearch,
            30,
        ).json_api(),

        SearchEngine::new(
            "arxiv_search",
            "arXiv Full-Text Search",
            "http://export.arxiv.org/api/query?search_query=all:{query}&max_results=25",
            EngineKind::AcademicSearch,
            30,
        ),

        // ── Patents ───────────────────────────────────────────────
        SearchEngine::new(
            "epo_ops",
            "EPO Open Patent Services",
            "https://ops.epo.org/3.2/rest-services/published-data/search?q={query}&Range=1-25",
            EngineKind::PatentSearch,
            60,
        ).json_api(),

        SearchEngine::new(
            "google_patents_search",
            "Google Patents",
            "https://patents.google.com/xhr/query?url=q%3D{query}%26num%3D25",
            EngineKind::PatentSearch,
            10,
        ).proxy(),

        SearchEngine::new(
            "wipo_patentscope",
            "WIPO PatentScope",
            "https://patentscope.wipo.int/search/en/search.jsf?query={query}",
            EngineKind::PatentSearch,
            10,
        ),

        // ── Social ────────────────────────────────────────────────
        SearchEngine::new(
            "twitter_api",
            "Twitter/X Search API",
            "https://api.twitter.com/2/tweets/search/recent?query={query}&max_results=100&tweet.fields=created_at,author_id,public_metrics",
            EngineKind::SocialSearch,
            450, // 450 req/15min window = 30 rpm
        ).json_api(),

        SearchEngine::new(
            "reddit_search",
            "Reddit Search",
            "https://www.reddit.com/search.json?q={query}&sort=new&limit=25",
            EngineKind::SocialSearch,
            30,
        ).json_api(),

        // ── Government / Legal ────────────────────────────────────
        SearchEngine::new(
            "europe_pmc",
            "Europe PMC (life sciences)",
            "https://www.ebi.ac.uk/europepmc/webservices/rest/search?query={query}&pageSize=25&format=json",
            EngineKind::AcademicSearch,
            60,
        ).json_api(),

        SearchEngine::new(
            "courtlistener",
            "CourtListener US Case Law",
            "https://www.courtlistener.com/api/rest/v3/search/?type=o&q={query}&format=json",
            EngineKind::GovernmentRegistry,
            100,
        ).json_api(),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Rotation pool
// ─────────────────────────────────────────────────────────────────────────────

/// Thread-safe round-robin search engine pool.
///
/// `next()` returns the next enabled engine in sequence, wrapping around.
/// Only engines with `enabled = true` participate in rotation.
#[derive(Clone)]
pub struct SearchPool {
    engines: Arc<Vec<SearchEngine>>,
    cursor: Arc<AtomicUsize>,
    web_cursor: Arc<AtomicUsize>,
    rate_limits: Arc<Mutex<RateLimitManager>>,
    rpm_windows: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    governor: Arc<CrawlGovernor>,
}

impl SearchPool {
    /// Create a pool from an explicit engine list.
    pub fn new(engines: Vec<SearchEngine>) -> Self {
        Self::with_runtime_controls(
            engines,
            Arc::new(Mutex::new(RateLimitManager::new())),
            Arc::new(CrawlGovernor::with_limits(30, 120)),
        )
    }

    pub fn with_runtime_controls(
        engines: Vec<SearchEngine>,
        rate_limits: Arc<Mutex<RateLimitManager>>,
        governor: Arc<CrawlGovernor>,
    ) -> Self {
        Self {
            engines: Arc::new(engines),
            cursor: Arc::new(AtomicUsize::new(0)),
            web_cursor: Arc::new(AtomicUsize::new(0)),
            rate_limits,
            rpm_windows: Arc::new(Mutex::new(HashMap::new())),
            governor,
        }
    }

    /// Return the next enabled engine with health-aware rotation. Returns `None` if the
    /// pool is empty.
    pub fn next(&self) -> Option<&SearchEngine> {
        self.next_healthy_engine(None, &self.cursor)
    }

    /// Return the next web-search-only engine.
    pub fn next_web(&self) -> Option<&SearchEngine> {
        self.next_healthy_engine(Some(&EngineKind::WebSearch), &self.web_cursor)
    }

    /// Return engines matching a specific kind.
    pub fn engines_of_kind(&self, kind: &EngineKind) -> Vec<&SearchEngine> {
        self.engines
            .iter()
            .filter(|e| e.enabled && &e.kind == kind)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.engines.len()
    }

    pub fn enabled_len(&self) -> usize {
        self.engines.iter().filter(|e| e.enabled).count()
    }

    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    pub fn record_success(&self, engine_id: &str) {
        self.rate_limits
            .lock()
            .expect("search pool rate-limit lock poisoned")
            .record_success(engine_id);
    }

    pub fn record_failure(&self, engine_id: &str, is_soft: bool) {
        self.rate_limits
            .lock()
            .expect("search pool rate-limit lock poisoned")
            .record_failure(engine_id, is_soft);
    }

    fn next_healthy_engine(
        &self,
        kind: Option<&EngineKind>,
        cursor: &AtomicUsize,
    ) -> Option<&SearchEngine> {
        let eligible_ids: Vec<String> = self
            .engines
            .iter()
            .filter(|engine| {
                engine.enabled && kind.map(|target| &engine.kind == target).unwrap_or(true)
            })
            .map(|engine| engine.id.clone())
            .collect();
        if eligible_ids.is_empty() {
            return None;
        }

        let ranked_ids = self
            .rate_limits
            .lock()
            .expect("search pool rate-limit lock poisoned")
            .get_engines_by_health(&eligible_ids);
        if ranked_ids.is_empty() {
            return None;
        }

        let offset = cursor.fetch_add(1, Ordering::Relaxed) % ranked_ids.len();
        for engine_id in ranked_ids
            .iter()
            .cycle()
            .skip(offset)
            .take(ranked_ids.len())
        {
            let engine = self
                .engines
                .iter()
                .find(|candidate| candidate.id == *engine_id)?;
            if !self.engine_is_within_rpm(engine) {
                continue;
            }
            if !self.governor.try_acquire(&engine.id) {
                continue;
            }
            self.record_engine_selection(engine);
            return Some(engine);
        }

        None
    }

    fn engine_is_within_rpm(&self, engine: &SearchEngine) -> bool {
        let mut windows = self
            .rpm_windows
            .lock()
            .expect("search pool rpm-window lock poisoned");
        let window = windows.entry(engine.id.clone()).or_default();
        let now = Instant::now();
        while window
            .front()
            .map(|instant| now.duration_since(*instant).as_secs() >= 60)
            .unwrap_or(false)
        {
            window.pop_front();
        }
        window.len() < engine.rate_limit_rpm as usize
    }

    fn record_engine_selection(&self, engine: &SearchEngine) {
        let mut windows = self
            .rpm_windows
            .lock()
            .expect("search pool rpm-window lock poisoned");
        windows
            .entry(engine.id.clone())
            .or_default()
            .push_back(Instant::now());
    }
}

impl Default for SearchPool {
    fn default() -> Self {
        Self::new(all_engines())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_count_at_least_27() {
        assert!(all_engines().len() >= 27);
    }

    #[test]
    fn ids_are_unique() {
        let engines = all_engines();
        let mut seen = std::collections::HashSet::new();
        for e in &engines {
            assert!(seen.insert(&e.id), "Duplicate engine id: {}", e.id);
        }
    }

    #[test]
    fn build_url_encodes_spaces() {
        let engine = SearchEngine::new(
            "test",
            "Test",
            "https://example.com?q={query}",
            EngineKind::WebSearch,
            10,
        );
        let url = engine.build_url("CBRN export controls");
        assert!(url.contains("CBRN+export+controls") || url.contains("CBRN%20export%20controls"));
    }

    #[test]
    fn pool_round_robins() {
        let pool = SearchPool::with_runtime_controls(
            vec![
                SearchEngine::new(
                    "a",
                    "A",
                    "https://a.example/?q={query}",
                    EngineKind::WebSearch,
                    100,
                ),
                SearchEngine::new(
                    "b",
                    "B",
                    "https://b.example/?q={query}",
                    EngineKind::WebSearch,
                    100,
                ),
                SearchEngine::new(
                    "c",
                    "C",
                    "https://c.example/?q={query}",
                    EngineKind::WebSearch,
                    100,
                ),
            ],
            Arc::new(Mutex::new(RateLimitManager::new())),
            Arc::new(CrawlGovernor::with_limits(100, 100)),
        );
        assert!(pool.enabled_len() > 0);
        let e1 = pool.next().unwrap().id.clone();
        // After going through the full cycle, we should see diversity
        let mut seen = std::collections::HashSet::from([e1.clone()]);
        for _ in 1..pool.enabled_len() {
            seen.insert(pool.next().unwrap().id.clone());
        }
        assert!(seen.len() > 1);
        let _ = e1;
    }

    #[test]
    fn web_search_engines_present() {
        let pool = SearchPool::default();
        let web = pool.engines_of_kind(&EngineKind::WebSearch);
        assert!(!web.is_empty());
    }

    #[test]
    fn news_aggregator_engines_present() {
        let pool = SearchPool::default();
        let news = pool.engines_of_kind(&EngineKind::NewsAggregator);
        assert!(!news.is_empty());
    }

    #[test]
    fn academic_engines_present() {
        let pool = SearchPool::default();
        let academic = pool.engines_of_kind(&EngineKind::AcademicSearch);
        assert!(!academic.is_empty());
    }

    #[test]
    fn unhealthy_engine_is_skipped() {
        let engines = vec![
            SearchEngine::new(
                "blocked",
                "Blocked",
                "https://blocked.example/?q={query}",
                EngineKind::WebSearch,
                5,
            ),
            SearchEngine::new(
                "healthy",
                "Healthy",
                "https://healthy.example/?q={query}",
                EngineKind::WebSearch,
                5,
            ),
        ];
        let rate_limits = Arc::new(Mutex::new(RateLimitManager::new()));
        {
            let mut state = rate_limits.lock().unwrap();
            for _ in 0..4 {
                state.record_failure("blocked", false);
            }
        }

        let pool = SearchPool::with_runtime_controls(
            engines,
            rate_limits,
            Arc::new(CrawlGovernor::with_limits(100, 100)),
        );

        assert_eq!(pool.next_web().unwrap().id, "healthy");
    }

    #[test]
    fn engine_rpm_limit_is_enforced() {
        let pool = SearchPool::with_runtime_controls(
            vec![SearchEngine::new(
                "limited",
                "Limited",
                "https://limited.example/?q={query}",
                EngineKind::WebSearch,
                1,
            )],
            Arc::new(Mutex::new(RateLimitManager::new())),
            Arc::new(CrawlGovernor::with_limits(100, 100)),
        );

        assert_eq!(pool.next_web().unwrap().id, "limited");
        assert!(pool.next_web().is_none());
    }

    #[test]
    fn person_query_builder_returns_diverse_queries() {
        let queries = PersonOsintQueryBuilder::build_queries("Jane Doe", Some("Acme Corp"));
        assert!(
            queries.len() >= 8,
            "Expected at least 8 query templates, got {}",
            queries.len()
        );
        // All must contain the person's name
        for q in &queries {
            assert!(
                q.contains("Jane Doe") || q.contains("jane doe"),
                "Query missing name: {q}"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Person OSINT Query Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Generates a diverse set of OSINT search queries for a person name + org.
///
/// The queries cover: professional profiles, media appearances, publications,
/// government/legal records, conference speaking, patent filings, and corporate roles.
/// Using all of these across the engine pool maximises recall.
pub struct PersonOsintQueryBuilder;

impl PersonOsintQueryBuilder {
    /// Return a battery of search query strings for the given person.
    ///
    /// # Arguments
    /// * `name`    – Full name of the person (e.g. `"Jane Doe"`).
    /// * `company` – Optional current employer to disambiguate the person.
    pub fn build_queries(name: &str, company: Option<&str>) -> Vec<String> {
        let org_clause = company.map(|c| format!(" \"{c}\"")).unwrap_or_default();

        let mut queries: Vec<String> = Vec::with_capacity(24);

        // ── Professional profile ─────────────────────────────────
        queries.push(format!("site:linkedin.com/in \"{name}\""));
        queries.push(format!("\"{name}\"{org_clause} linkedin"));

        // ── News & media ─────────────────────────────────────────
        queries.push(format!(
            "\"{name}\"{org_clause} interview OR statement OR remarks"
        ));
        queries.push(format!(
            "\"{name}\"{org_clause} keynote OR speech OR panel OR conference"
        ));
        queries.push(format!(
            "\"{name}\"{org_clause} site:reuters.com OR site:bloomberg.com OR site:ft.com"
        ));
        queries.push(format!("\"{name}\"{org_clause} press release"));

        // ── Academic & research ──────────────────────────────────
        queries.push(format!(
            "\"{name}\"{org_clause} research paper OR publication OR study"
        ));
        queries.push(format!("site:scholar.google.com \"{name}\""));
        queries.push(format!("site:semanticscholar.org \"{name}\""));

        // ── Patent & IP ──────────────────────────────────────────
        queries.push(format!("site:patents.google.com inventor:\"{name}\""));
        queries.push(format!("site:lens.org \"{name}\" inventor"));

        // ── Corporate / legal records ────────────────────────────
        queries.push(format!(
            "\"{name}\"{org_clause} board of directors OR advisory board"
        ));
        queries.push(format!("site:opencorporates.com \"{name}\""));
        queries.push(format!("site:sec.gov \"{name}\""));
        queries.push(format!("site:companieshouse.gov.uk \"{name}\""));

        // ── Government & regulatory ──────────────────────────────
        queries.push(format!(
            "\"{name}\" site:.gov OR site:.gov.il OR site:.gov.de"
        ));
        queries.push(format!(
            "\"{name}\"{org_clause} testimony OR hearing OR deposition OR subpoena"
        ));
        queries.push(format!(
            "\"{name}\"{org_clause} sanction OR watchlist OR enforcement"
        ));

        // ── Document leaks / transparency ────────────────────────
        queries.push(format!("\"{name}\"{org_clause} filetype:pdf"));
        queries.push(format!(
            "\"{name}\"{org_clause} site:wikileaks.org OR site:icij.org OR site:occrp.org"
        ));

        // ── Social / forum ───────────────────────────────────────
        queries.push(format!("site:twitter.com \"{name}\""));
        queries.push(format!("site:reddit.com \"{name}\"{org_clause}"));

        // ── Event speaker ────────────────────────────────────────
        queries.push(format!(
            "\"{name}\"{org_clause} speaker bio OR about the speaker"
        ));
        queries.push(format!(
            "\"{name}\"{org_clause} site:techcrunch.com OR site:wired.com OR site:theregister.com"
        ));

        queries
    }

    /// Build search URLs by pairing each query with a pool engine and returning
    /// `(query_string, url)` pairs.  Engines are selected by round-robin so the
    /// load is spread.
    pub fn build_urls(
        name: &str,
        company: Option<&str>,
        pool: &SearchPool,
    ) -> Vec<(String, String)> {
        let queries = Self::build_queries(name, company);
        queries
            .into_iter()
            .filter_map(|q| {
                let engine = pool.next_web().or_else(|| pool.next())?;
                Some((q.clone(), engine.build_url(&q)))
            })
            .collect()
    }
}
