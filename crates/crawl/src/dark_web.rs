//! Dark web forum monitoring module.
//!
//! Monitors known dark web forums, paste sites, and ransomware leak blogs for
//! mentions of monitored entities, keywords, and indicators of compromise.
//!
//! # Architecture
//!
//! [`DarkWebMonitor`] maintains a list of [`DarkWebForum`] targets and
//! [`MonitoringRule`] definitions.  Each scan cycle:
//!
//! 1. Fetches recent content from each active forum through the Tor SOCKS5
//!    proxy when configured; without Tor, only forums explicitly marked
//!    clearnet-safe (legitimate research/news/paste surfaces) are scanned.
//! 2. Extracts candidate posts via HTML parsing / regex.
//! 3. Matches against configured keywords and requires a monitored entity
//!    name/domain mention, so untargeted keyword hits are not emitted.
//! 4. Scores each post by relevance.
//! 5. Extracts embedded indicators (emails, domains, BTC addresses, etc.).
//! 6. Returns scored [`DarkWebPost`] records for downstream persistence.
//!
//! # Usage
//! ```no_run
//! use apex_crawl::dark_web::DarkWebMonitor;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let monitor = DarkWebMonitor::new(None)?;
//! let posts = monitor.scan_all().await;
//! for post in &posts {
//!     println!("[{}] {} — score {:.2}", post.forum_name, post.thread_title, post.relevance_score);
//! }
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use regex::{Regex, RegexBuilder};
use reqwest::{Client, Proxy, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Once;
use std::time::Duration;
use tracing::{debug, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Classification of a dark web forum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForumType {
    /// General discussion forums (e.g., Dread, dark web Reddit-like).
    General,
    /// Credit card shops and CVV markets.
    Cardshop,
    /// Exploit / 0-day trading forums.
    Exploit,
    /// Data leak / breach forums (e.g., BreachForums).
    Leak,
    /// Ransomware group data-leak blogs (e.g., LockBit, Clop).
    Ransomware,
}

impl ForumType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Cardshop => "cardshop",
            Self::Exploit => "exploit",
            Self::Leak => "leak",
            Self::Ransomware => "ransomware",
        }
    }
}

/// How a forum is accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMethod {
    /// Regular clearnet website (HTTPS).
    Clearnet,
    /// Tor onion service (`.onion`).
    TorOnion,
    /// I2P eepsite (`.i2p`).
    I2P,
    /// Telegram channel / group.
    Telegram,
}

impl AccessMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Clearnet => "clearnet",
            Self::TorOnion => "tor_onion",
            Self::I2P => "i2p",
            Self::Telegram => "telegram",
        }
    }
}

/// A monitored dark web forum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DarkWebForum {
    /// Human-readable name.
    pub name: String,
    /// Base URL of the forum.
    pub base_url: String,
    /// Classification of the forum.
    pub forum_type: ForumType,
    /// How to reach the forum.
    pub access_method: AccessMethod,
    /// Whether this forum is currently being monitored.
    pub is_active: bool,
    /// Last time the forum was checked.
    pub last_checked: Option<DateTime<Utc>>,
    /// Topics / sections of interest (empty = monitor whole forum).
    pub topics_of_interest: Vec<String>,
}

/// Hosts that are legitimate clearnet research / news / paste surfaces.
///
/// Criminal forums, card shops, exploit markets and leak sites must **never**
/// be fetched from the operator's real IP; only these explicitly marked
/// surfaces may be scanned without Tor.
const CLEARNET_SAFE_HOSTS: &[&str] = &["haveibeenpwned.com", "pastebin.com", "darkfeed.io"];

impl DarkWebForum {
    /// Whether this forum may be scanned over clearnet (no Tor proxy).
    ///
    /// This is the explicit clearnet-safe marking required before
    /// [`DarkWebMonitor::scan_forum`] will fetch a forum without Tor: the
    /// forum must both be configured as [`AccessMethod::Clearnet`] and have a
    /// host on the [`CLEARNET_SAFE_HOSTS`] allowlist of legitimate
    /// research/news/paste surfaces. Criminal forums and marketplaces fail
    /// this check, so a clearnet-only monitor can never expose the operator's
    /// real IP to them.
    pub fn is_clearnet_safe(&self) -> bool {
        if self.access_method != AccessMethod::Clearnet {
            return false;
        }
        let Some(host) = crate::browser::validation::host_from_url(&self.base_url) else {
            return false;
        };
        CLEARNET_SAFE_HOSTS
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    }
}

/// A forum post or thread matching monitoring criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DarkWebPost {
    /// Unique post identifier (forum-specific).
    pub id: String,
    /// Name of the source forum.
    pub forum_name: String,
    /// Thread / post title.
    pub thread_title: String,
    /// Author handle / username.
    pub author: String,
    /// Truncated content around the matched keywords.
    pub content_snippet: String,
    /// When the post was published.
    ///
    /// Best-effort: parsed from the page when the scraped text carries a date,
    /// otherwise set to the fetch time (the field is not optional). Post ids
    /// are content-addressed, so the fallback timestamp cannot cause the same
    /// unchanged post to be re-reported.
    pub posted_at: DateTime<Utc>,
    /// Direct URL to the post.
    pub url: String,
    /// Which keywords from monitoring rules matched.
    pub matched_keywords: Vec<String>,
    /// Computed relevance score (0.0 – 1.0).
    pub relevance_score: f64,
    /// Entity names / indicators mentioned in the post.
    pub entities_mentioned: Vec<String>,
}

/// Configuration for a single monitoring rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringRule {
    /// Unique rule identifier.
    pub id: String,
    /// Human-readable name for the rule.
    pub name: String,
    /// Keywords to search for (case-insensitive).
    pub keywords: Vec<String>,
    /// Entity IDs (UUIDs) this rule applies to.
    pub entity_ids: Vec<String>,
    /// Minimum relevance score to trigger an alert.
    pub min_relevance: f64,
    /// Notification channels for matches (e.g., "slack", "email", "webhook").
    pub notification_channels: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Default forum seed list
// ─────────────────────────────────────────────────────────────────────────────

/// Seed the monitor with known breach/leak forums.
pub fn default_forums() -> Vec<DarkWebForum> {
    vec![
        DarkWebForum {
            name: "Have I Been Pwned".into(),
            base_url: "https://haveibeenpwned.com".into(),
            forum_type: ForumType::Leak,
            access_method: AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec![],
        },
        DarkWebForum {
            name: "Pastebin".into(),
            base_url: "https://pastebin.com".into(),
            forum_type: ForumType::Leak,
            access_method: AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec![],
        },
        DarkWebForum {
            name: "BreachForums".into(),
            base_url: "https://breachforums.st".into(),
            forum_type: ForumType::Leak,
            access_method: AccessMethod::Clearnet,
            // Criminal forum: never scanned over clearnet. A Tor proxy must be
            // configured and the operator must explicitly re-enable it, so the
            // operator's real IP/identity is never exposed to the forum.
            is_active: false,
            last_checked: None,
            topics_of_interest: vec![
                "databases".into(),
                "leaks".into(),
                "combo".into(),
                "dump".into(),
            ],
        },
        DarkWebForum {
            name: "Exploit.in".into(),
            base_url: "https://exploit.in".into(),
            forum_type: ForumType::Exploit,
            access_method: AccessMethod::Clearnet,
            // Criminal exploit market: see BreachForums above.
            is_active: false,
            last_checked: None,
            topics_of_interest: vec![
                "exploit".into(),
                "0day".into(),
                "vulnerability".into(),
                "shell".into(),
            ],
        },
        DarkWebForum {
            name: "Ransomware Blog (Generic)".into(),
            base_url: "https://darkfeed.io".into(),
            forum_type: ForumType::Ransomware,
            access_method: AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec!["leak".into(), "victim".into(), "ransom".into()],
        },
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Constants for relevance scoring
// ─────────────────────────────────────────────────────────────────────────────

/// Priority keywords that get a boost when matched.
const HIGH_PRIORITY_KEYWORDS: &[&str] = &[
    "breach",
    "leak",
    "ransom",
    "exploit",
    "vulnerability",
    "supply chain",
    "backdoor",
    "credential",
    "password",
    "pii",
    "zero-day",
    "0day",
];

/// Recency bonus: posts within this window get a boost.
const RECENCY_BONUS_HOURS: i64 = 24;

/// Base boost for a high-priority keyword match.
const HIGH_PRIORITY_BOOST: f64 = 0.15;

/// Decay factor for old posts (per day beyond the recency window).
const AGE_DECAY_PER_DAY: f64 = 0.05;

/// Generic browser-like User-Agent for dark-web scraping.
///
/// The monitor must not advertise `ApexIntel` to criminal infrastructure: a
/// unique product UA is a stable fingerprint that links every scan back to the
/// same operator, defeating the anonymity the Tor proxy provides.
pub(crate) const DARK_WEB_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0";

/// Maximum compiled regex program size (1 MiB) for all runtime-built regexes.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Minimum character length for a candidate block to be treated as a real post
/// rather than navigation/menu chrome.
const MIN_CANDIDATE_CHARS: usize = 80;

/// Minimum whitespace-separated words for a candidate block to be a real post.
const MIN_CANDIDATE_WORDS: usize = 8;

/// Upper bound on snippet context so a hostile page cannot force huge slices.
const MAX_SNIPPET_CONTEXT_CHARS: usize = 2_000;

/// Warns once per process when scans cannot emit posts because no monitored
/// entity names are configured.
static NO_ENTITY_NAMES_WARNED: Once = Once::new();

// ─────────────────────────────────────────────────────────────────────────────
// Entity extraction regexes
// ─────────────────────────────────────────────────────────────────────────────

/// Compile a regex from a compile-time-constant pattern.
///
/// Every caller passes a string literal, so a failure would be a programming
/// error rather than a runtime condition.
#[allow(clippy::expect_used)]
fn compile_regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex literal")
}

lazy_static! {
    /// Email address pattern.
    static ref RE_EMAIL: Regex = compile_regex(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}");

    /// Domain name pattern (simple).
    static ref RE_DOMAIN: Regex = compile_regex(r"(?:(?:https?://)?(?:www\.)?)[a-zA-Z0-9][a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?:/[^\s]*)?");

    /// Bitcoin address (P2PKH, P2SH, Bech32).
    static ref RE_BITCOIN: Regex = compile_regex(r"\b(bc1|[13])[a-zA-HJ-NP-Z0-9]{25,39}\b");

    /// Ethereum address (0x-prefixed hex).
    static ref RE_ETHEREUM: Regex = compile_regex(r"\b0x[a-fA-F0-9]{40}\b");

    /// IPv4 address.
    static ref RE_IPV4: Regex = compile_regex(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b");

    /// Bitcoin (BTC) transaction hash.
    static ref RE_TX_HASH: Regex = compile_regex(r"\b[a-fA-F0-9]{64}\b");

    /// Telegram handle.
    static ref RE_TELEGRAM: Regex = compile_regex(r"\bt\.me/[a-zA-Z0-9_]{5,}\b");

    /// Potential company name pattern (capitalized words, 2+).
    static ref RE_COMPANY: Regex = compile_regex(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+\b");

    /// ISO-8601-ish date (optionally with a time) as often shown on forum posts.
    static ref RE_POST_DATE: Regex = compile_regex(
        r"\b(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?\b"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// DarkWebMonitor
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregate outcome of one full scan pass across all active forums.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    /// Matching posts, sorted by relevance (descending) then recency.
    pub posts: Vec<DarkWebPost>,
    /// Active forums scanned without error.
    pub forums_scanned: u64,
    /// Active forums whose scan returned an error.
    pub forums_failed: u64,
}

/// Dark web forum and paste-site monitor.
///
/// Orchestrates periodic scanning of configured forums, matches content against
/// monitoring rules, scores posts by relevance, and extracts embedded
/// indicators.
#[derive(Debug)]
pub struct DarkWebMonitor {
    /// Configured forums to monitor.
    pub forums: Vec<DarkWebForum>,
    /// Active monitoring rules.
    pub rules: Vec<MonitoringRule>,
    /// Names / domains of the entities being monitored.
    ///
    /// A candidate becomes a post only when it mentions at least one of these
    /// (case-insensitive word-boundary match). When the list is empty the
    /// monitor emits no posts and logs once: untargeted keyword hits are not
    /// intelligence and would otherwise flood downstream consumers.
    pub entity_names: Vec<String>,
    /// Reusable HTTP client.
    http_client: Client,
    /// Optional Tor SOCKS5 proxy URL (always `socks5h://`, e.g.
    /// `socks5h://127.0.0.1:9050`).
    tor_proxy_url: Option<String>,
}

impl DarkWebMonitor {
    /// Create a new monitor.
    ///
    /// If `tor_proxy_url` is `Some("socks5h://...")`, all HTTP traffic is
    /// routed through the Tor SOCKS5 proxy with DNS resolved inside Tor. A
    /// legacy `socks5://` URL is rewritten to `socks5h://` so operator input
    /// can never silently leak DNS lookups. Set to `None` for clearnet-only
    /// access (only forums marked clearnet-safe can then be scanned).
    ///
    /// Forums are seeded from [`default_forums()`] and can be replaced via
    /// [`set_forums`](Self::set_forums).
    pub fn new(tor_proxy_url: Option<String>) -> anyhow::Result<Self> {
        // Never let a plain `socks5://` proxy resolve DNS locally: rewrite it
        // to `socks5h://` so hostname lookups happen inside Tor.
        let tor_proxy_url = tor_proxy_url.map(|url| normalize_socks_proxy_url(&url));

        // Route through Tor SOCKS5 proxy if configured
        let proxy = match tor_proxy_url {
            Some(ref proxy_url) => {
                let proxy = Proxy::all(proxy_url)
                    .map_err(|e| anyhow::anyhow!("invalid Tor proxy URL '{}': {}", proxy_url, e))?;
                debug!(proxy_url = %proxy_url, "dark_web: Tor proxy configured");
                Some(proxy)
            }
            None => None,
        };

        let http_client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(30),
            user_agent: Some(DARK_WEB_USER_AGENT.to_string()),
            proxy,
            pool_max_idle_per_host: Some(4),
            ..crate::http::ExternalClientOptions::default()
        })
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {}", e))?;

        Ok(Self {
            forums: default_forums(),
            rules: Vec::new(),
            entity_names: Vec::new(),
            http_client,
            tor_proxy_url,
        })
    }

    /// Replace the default forum list with a custom one.
    pub fn set_forums(&mut self, forums: Vec<DarkWebForum>) {
        self.forums = forums;
    }

    /// Replace the default rules with a custom set.
    pub fn set_rules(&mut self, rules: Vec<MonitoringRule>) {
        self.rules = rules;
    }

    /// Replace the monitored entity names / domains used to gate posts.
    pub fn set_entities(&mut self, entity_names: Vec<String>) {
        self.entity_names = entity_names;
    }

    /// Add one monitored entity name or domain.
    pub fn add_entity(&mut self, entity_name: impl Into<String>) {
        self.entity_names.push(entity_name.into());
    }

    /// Add a single monitoring rule.
    pub fn add_rule(&mut self, rule: MonitoringRule) {
        self.rules.push(rule);
    }

    /// Whether Tor proxy is configured.
    pub fn has_tor_proxy(&self) -> bool {
        self.tor_proxy_url.is_some()
    }

    // ── Scanning ────────────────────────────────────────────────────────────

    /// Scan **all** active forums and return matching posts.
    ///
    /// Errors from individual forums are logged and suppressed so that a
    /// temporary outage on one forum does not prevent scanning the others.
    pub async fn scan_all(&self) -> Vec<DarkWebPost> {
        self.scan_all_detailed().await.posts
    }

    /// Scan all active forums, returning posts plus per-forum outcome counts.
    ///
    /// Like [`scan_all`](Self::scan_all), errors from individual forums are
    /// logged and suppressed, but callers that need to report scan quality
    /// (e.g. the worker's `dark_web_scan` job) can inspect
    /// [`ScanReport::forums_failed`].
    pub async fn scan_all_detailed(&self) -> ScanReport {
        let mut report = ScanReport::default();

        if self.entity_names.is_empty() {
            NO_ENTITY_NAMES_WARNED.call_once(|| {
                warn!("dark_web: no monitored entity names configured — scans will not emit posts");
            });
        }

        for forum in &self.forums {
            if !forum.is_active {
                debug!(forum = %forum.name, "dark_web: skipping inactive forum");
                continue;
            }

            match self.scan_forum(forum).await {
                Ok(mut posts) => {
                    debug!(
                        forum = %forum.name,
                        count = posts.len(),
                        "dark_web: forum scan complete"
                    );
                    report.forums_scanned += 1;
                    report.posts.append(&mut posts);
                }
                Err(e) => {
                    report.forums_failed += 1;
                    warn!(
                        forum = %forum.name,
                        error = %e,
                        "dark_web: forum scan failed"
                    );
                }
            }
        }

        // Dedupe content-addressed ids (e.g. identical blocks seen twice in
        // one page or across forums) before sorting.
        report.posts = dedupe_posts_by_id(report.posts);

        // Sort by relevance descending, then by posted_at descending
        report.posts.sort_unstable_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.posted_at.cmp(&a.posted_at))
        });

        report
    }

    /// Scan a single forum for matching posts.
    ///
    /// The scraping strategy depends on the forum type:
    /// - `Leak` / `General`: fetch the main page / recent threads, extract
    ///   text blocks, run keyword matching.
    /// - `Ransomware`: similar to general — most ransomware leak blogs are
    ///   simple HTML pages.
    /// - `Exploit` / `Cardshop`: fetch and scan.
    ///
    /// For `haveibeenpwned.com` a dedicated HIBP integration may be used
    /// instead (see [`crate::breach::BreachMonitor`]).
    pub async fn scan_forum(&self, forum: &DarkWebForum) -> anyhow::Result<Vec<DarkWebPost>> {
        debug!(forum = %forum.name, url = %forum.base_url, "dark_web: scanning forum");

        // Identity protection: without Tor, only explicitly marked clearnet
        // research/news/paste surfaces may be fetched. Criminal forums and
        // marketplaces would otherwise see the operator's real IP.
        if self.tor_proxy_url.is_none() && !forum.is_clearnet_safe() {
            warn!(
                forum = %forum.name,
                url = %forum.base_url,
                "dark_web: refusing clearnet scan of forum that is not marked clearnet-safe"
            );
            anyhow::bail!(
                "refusing to scan '{}' over clearnet without Tor: forum is not marked clearnet-safe",
                forum.name
            );
        }

        // Special-case HIBP — it's an API, not a scrapable forum
        if forum.base_url.contains("haveibeenpwned.com") {
            return Ok(vec![]); // handled by BreachMonitor instead
        }

        // Special-case Pastebin — use the scrape API
        if forum.base_url.contains("pastebin.com") {
            return self.scan_pastebin_like(forum).await;
        }

        // Generic forum scraper: fetch HTML, extract text, match keywords
        let url = forum.base_url.trim_end_matches('/').to_string();
        let resp = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch {}: {}", forum.base_url, e))?;

        if !resp.status().is_success() {
            if resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND {
                debug!(
                    forum = %forum.name,
                    status = %resp.status(),
                    "dark_web: forum returned non-success — skipping"
                );
                return Ok(vec![]);
            }
            anyhow::bail!("forum {} returned HTTP {}", forum.base_url, resp.status());
        }

        let html = crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to read response body from {}: {}",
                    forum.base_url,
                    e
                )
            })?;

        if html.len() < 100 {
            debug!(forum = %forum.name, "dark_web: response too short, skipping");
            return Ok(vec![]);
        }

        // Extract text blocks from HTML (strip tags roughly)
        let text = strip_html_tags(&html);

        // Split into candidate "posts" by common separators
        let candidates = split_into_candidates(&text);

        let posts = self.build_posts_from_candidates(forum, &candidates);

        debug!(
            forum = %forum.name,
            candidates = candidates.len(),
            matches = posts.len(),
            "dark_web: forum scan results"
        );

        Ok(posts)
    }

    /// Build scored posts from candidate blocks.
    ///
    /// Applies keyword matching, the monitored-entity gate, stable
    /// content-addressed ids, best-effort dates and deduplication. Candidates
    /// are only kept when they match at least one keyword **and** mention at
    /// least one monitored entity name/domain; an empty entity list therefore
    /// yields no posts (the caller logs that once).
    fn build_posts_from_candidates(
        &self,
        forum: &DarkWebForum,
        candidates: &[String],
    ) -> Vec<DarkWebPost> {
        let now = Utc::now();
        let mut posts = Vec::new();

        for candidate in candidates {
            // Run keyword matching against all rules (or default keywords)
            let matched_keywords = if self.rules.is_empty() {
                // Default keyword set when no rules configured
                let default_kws = default_monitoring_keywords();
                match_keywords(candidate, &default_kws)
            } else {
                let mut all_kws = Vec::new();
                for rule in &self.rules {
                    all_kws.extend(rule.keywords.iter().cloned());
                }
                all_kws.sort();
                all_kws.dedup();
                match_keywords(candidate, &all_kws)
            };

            if matched_keywords.is_empty() {
                continue;
            }

            // Untargeted keyword hits are not intelligence: require at least
            // one monitored entity name or domain before emitting a post.
            if !mentions_monitored_entity(candidate, &self.entity_names) {
                continue;
            }

            // Extract entities from the candidate text
            let entities = Self::extract_entities(candidate);

            // Compute relevance score
            let total_keywords = if self.rules.is_empty() {
                default_monitoring_keywords().len() as f64
            } else {
                self.rules
                    .iter()
                    .flat_map(|r| r.keywords.iter())
                    .count()
                    .max(1) as f64
            };

            let relevance = Self::compute_relevance_score(
                &matched_keywords,
                &entities,
                candidate,
                now,
                total_keywords,
            );

            let snippet = extract_snippet(candidate, &matched_keywords, 200);

            posts.push(DarkWebPost {
                // Content-addressed: stable across scans and independent of
                // candidate order or wall-clock time.
                id: stable_post_id(&forum.name, candidate),
                forum_name: forum.name.clone(),
                thread_title: truncate_title(candidate, 120),
                author: "unknown".into(), // generic scrape cannot always extract author
                content_snippet: snippet,
                // Best-effort: use a date carried by the scraped text. The
                // public type requires a value, so fall back to the fetch
                // time; the content-addressed id keeps re-scans idempotent.
                posted_at: extract_posted_at(candidate).unwrap_or(now),
                url: forum.base_url.clone(),
                matched_keywords,
                relevance_score: relevance,
                entities_mentioned: entities,
            });
        }

        dedupe_posts_by_id(posts)
    }

    /// Scan a pastebin-like site (simple text API).
    async fn scan_pastebin_like(&self, forum: &DarkWebForum) -> anyhow::Result<Vec<DarkWebPost>> {
        let scrape_url = "https://scrape.pastebin.com/api_scraping.php?limit=25";
        let resp = self
            .http_client
            .get(scrape_url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch pastebin scrape list: {}", e))?;

        if !resp.status().is_success() {
            debug!(
                forum = %forum.name,
                status = %resp.status(),
                "dark_web: pastebin scrape API unavailable"
            );
            return Ok(vec![]);
        }

        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct PasteInfo {
            scrape_url: Option<String>,
            full_url: Option<String>,
            key: Option<String>,
            date: Option<String>,
            size: Option<String>,
            title: Option<String>,
        }

        let pastes: Vec<PasteInfo> =
            match crate::http::read_capped_json(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await {
                Ok(p) => p,
                Err(_) => return Ok(vec![]),
            };

        let now = Utc::now();
        let mut posts = Vec::new();

        let all_keywords: Vec<String> = if self.rules.is_empty() {
            default_monitoring_keywords()
        } else {
            let mut kws = Vec::new();
            for rule in &self.rules {
                kws.extend(rule.keywords.iter().cloned());
            }
            kws.sort();
            kws.dedup();
            kws
        };

        let total_keywords = all_keywords.len().max(1) as f64;

        for paste in &pastes {
            let content_url = match paste.scrape_url.as_deref().and_then(safe_response_url) {
                Some(url) => url,
                None => {
                    debug!("dark_web: dropping paste with a non-public scrape URL");
                    continue;
                }
            };

            let content_resp = match self.http_client.get(&content_url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };

            if !content_resp.status().is_success() {
                continue;
            }

            let content =
                match crate::http::read_capped(content_resp, crate::http::MAX_EXTERNAL_BODY_BYTES)
                    .await
                {
                    Ok(t) => t,
                    Err(_) => continue,
                };

            let matched = match_keywords(&content, &all_keywords);
            if matched.is_empty() {
                continue;
            }

            // Same monitored-entity gate as the generic scraper.
            if !mentions_monitored_entity(&content, &self.entity_names) {
                continue;
            }

            let entities = Self::extract_entities(&content);
            let snippet = extract_snippet(&content, &matched, 200);

            let posted_at = paste
                .date
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok())
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);

            let relevance =
                Self::compute_relevance_score(&matched, &entities, &content, now, total_keywords);

            let paste_url = paste
                .full_url
                .as_deref()
                .and_then(safe_response_url)
                .unwrap_or_else(|| {
                    format!(
                        "https://pastebin.com/{}",
                        paste.key.as_deref().unwrap_or("unknown")
                    )
                });

            posts.push(DarkWebPost {
                id: paste
                    .key
                    .clone()
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                forum_name: forum.name.clone(),
                thread_title: paste.title.clone().unwrap_or_else(|| "Untitled".into()),
                author: "anonymous".into(),
                content_snippet: snippet,
                posted_at,
                url: paste_url,
                matched_keywords: matched,
                relevance_score: relevance,
                entities_mentioned: entities,
            });
        }

        Ok(posts)
    }

    // ── Relevance scoring ───────────────────────────────────────────────────

    /// Compute a relevance score for a post.
    ///
    /// Factors:
    /// - Base score: `matched_keywords.len() / total_keywords` (clamped to 1.0)
    /// - Boost for high-priority keywords (e.g., "breach", "leak", "ransom")
    /// - Boost for entity mentions (emails, domains, BTC addresses)
    /// - Recency bonus for posts < 24 hours old
    /// - Decay for older posts (5% per day beyond 24h)
    pub fn calculate_relevance(&self, post: &DarkWebPost) -> f64 {
        let total_keywords: f64 = if self.rules.is_empty() {
            default_monitoring_keywords().len() as f64
        } else {
            self.rules
                .iter()
                .flat_map(|r| r.keywords.iter())
                .count()
                .max(1) as f64
        };

        Self::compute_relevance_score(
            &post.matched_keywords,
            &post.entities_mentioned,
            &post.content_snippet,
            Utc::now(),
            total_keywords,
        )
    }

    fn compute_relevance_score(
        matched_keywords: &[String],
        entities: &[String],
        content: &str,
        _now: DateTime<Utc>,
        total_keywords: f64,
    ) -> f64 {
        // Base score: ratio of matched keywords to total monitored keywords
        let base_ratio = (matched_keywords.len() as f64 / total_keywords).clamp(0.0, 1.0);
        let mut score = base_ratio;

        // Boost for high-priority keywords
        let content_lower = content.to_lowercase();
        for kw in HIGH_PRIORITY_KEYWORDS {
            if content_lower.contains(kw) {
                score += HIGH_PRIORITY_BOOST;
            }
        }

        // Boost for entity indicators
        if !entities.is_empty() {
            score += (entities.len() as f64).min(3.0) * 0.05;
        }

        // Recency boost / decay
        // We approximate with the current time since posts don't always carry
        // reliable timestamps in generic scraping.
        // For Pastebin posts we use the actual timestamp; for generic HTML
        // scraping we approximate.
        score += 0.1; // slight recency assumption for freshly scraped content

        // Clamp to [0.0, 1.0]
        score.clamp(0.0, 1.0)
    }

    /// Compute recency-adjusted score using post's actual `posted_at`.
    pub fn score_with_recency(posted_at: DateTime<Utc>, base_score: f64) -> f64 {
        let now = Utc::now();
        let hours_ago = now.signed_duration_since(posted_at).num_hours().max(0);

        let mut score = base_score;

        if hours_ago <= RECENCY_BONUS_HOURS {
            // Full recency bonus
            score += 0.1;
        } else {
            // Decay for older posts
            let days_old = ((hours_ago - RECENCY_BONUS_HOURS) as f64 / 24.0).max(0.0);
            let decay = days_old * AGE_DECAY_PER_DAY;
            score = (score - decay).max(0.0);
        }

        score.clamp(0.0, 1.0)
    }

    // ── Entity extraction ───────────────────────────────────────────────────

    /// Extract entity names and indicators from text using regex patterns.
    ///
    /// Detects:
    /// - Email addresses
    /// - Domain names / URLs
    /// - Bitcoin addresses
    /// - Ethereum addresses
    /// - IPv4 addresses
    /// - Telegram handles
    /// - Potential company names (two-or-more capitalized words)
    pub fn extract_entities(text: &str) -> Vec<String> {
        let mut entities = Vec::new();

        // Emails
        for cap in RE_EMAIL.find_iter(text) {
            let email = cap.as_str().to_lowercase();
            if !entities.contains(&email) {
                entities.push(email);
            }
        }

        // Domains (filter out very common false positives)
        for cap in RE_DOMAIN.find_iter(text) {
            let domain = cap.as_str().to_lowercase();
            if domain.len() > 4 && !domain.contains("example.com") && !entities.contains(&domain) {
                entities.push(domain);
            }
        }

        // Bitcoin addresses
        for cap in RE_BITCOIN.find_iter(text) {
            let addr = cap.as_str().to_string();
            if !entities.contains(&addr) {
                entities.push(addr);
            }
        }

        // Ethereum addresses
        for cap in RE_ETHEREUM.find_iter(text) {
            let addr = cap.as_str().to_string();
            if !entities.contains(&addr) {
                entities.push(addr);
            }
        }

        // IPv4 addresses (private/loopback/link-local/CGNAT filtered via the
        // shared browser validator instead of ad-hoc prefix checks)
        for cap in RE_IPV4.find_iter(text) {
            let ip = cap.as_str().to_string();
            if !crate::browser::validation::is_private_host(&ip) && !entities.contains(&ip) {
                entities.push(ip);
            }
        }

        // Telegram handles
        for cap in RE_TELEGRAM.find_iter(text) {
            let tg = cap.as_str().to_string();
            if !entities.contains(&tg) {
                entities.push(tg);
            }
        }

        // Company names (capitalized multi-word phrases)
        for cap in RE_COMPANY.find_iter(text) {
            let company = cap.as_str().to_string();
            // Filter out common non-company phrases
            if company.len() > 4
                && !company.contains("This")
                && !company.contains("The ")
                && !company.contains("Please")
                && !company.contains("Hello")
                && !entities.contains(&company)
            {
                entities.push(company);
            }
        }

        entities
    }

    // ── Rule matching ───────────────────────────────────────────────────────

    /// Check if a post matches a specific monitoring rule.
    ///
    /// Returns `true` if the post's relevance score meets or exceeds the
    /// rule's `min_relevance` threshold and any of the rule's keywords appear
    /// in the post's title or snippet as a whole word (case-insensitive).
    pub fn matches_rule(&self, post: &DarkWebPost, rule: &MonitoringRule) -> bool {
        if post.relevance_score < rule.min_relevance {
            return false;
        }

        let haystack = format!("{}\n{}", post.thread_title, post.content_snippet);
        !match_keywords(&haystack, &rule.keywords).is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Default set of monitoring keywords used when no rules are configured.
///
/// Only terms specific enough to imply targeting or a real compromise are
/// kept. Near-universal words (`access`, `admin`, `config`, `proxy`, `vpn`,
/// `database`, `shell`, `crawl`, `scrape`, `spider`, `c2`, `cnc`, `dump`,
/// `combo`, `exposed`, `payload`) were removed because they match almost every
/// page and drown real hits. All matching is word-bounded, so e.g. `rat` does
/// not match "generation", "separate" or "rate".
pub fn default_monitoring_keywords() -> Vec<String> {
    vec![
        "breach".into(),
        "leak".into(),
        "ransom".into(),
        "exploit".into(),
        "vulnerability".into(),
        "supply chain".into(),
        "credential".into(),
        "password".into(),
        "pii".into(),
        "backdoor".into(),
        "zero-day".into(),
        "0day".into(),
        "sql injection".into(),
        "compromised".into(),
        "malware".into(),
        "phishing".into(),
        "trojan".into(),
        "rat".into(),
        "botnet".into(),
        "ddos".into(),
    ]
}

/// Rewrite a plain `socks5://` proxy URL to `socks5h://`.
///
/// `socks5h://` resolves hostnames inside the proxy (Tor), while `socks5://`
/// resolves them locally and leaks DNS lookups. The check is case-insensitive
/// so operator-provided URLs cannot smuggle the leaking scheme through.
fn normalize_socks_proxy_url(url: &str) -> String {
    const LEGACY_SCHEME: &str = "socks5://";
    match url.get(..LEGACY_SCHEME.len()) {
        Some(prefix) if prefix.eq_ignore_ascii_case(LEGACY_SCHEME) => {
            format!("socks5h://{}", &url[LEGACY_SCHEME.len()..])
        }
        _ => url.to_string(),
    }
}

/// Compile a bounded, case-insensitive regex for one keyword/entity.
///
/// With `word_boundaries`, the keyword must match as a whole word so short
/// terms cannot fire inside larger words. Returns `None` for empty keywords or
/// regex-compilation failures (never panics on hostile input).
fn keyword_regex(keyword: &str, word_boundaries: bool) -> Option<Regex> {
    if keyword.trim().is_empty() {
        return None;
    }
    let escaped = regex::escape(keyword);
    let pattern = if word_boundaries {
        format!(r"\b{escaped}\b")
    } else {
        escaped
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .ok()
}

/// Whether `text` mentions at least one monitored entity name or domain.
///
/// Case-insensitive with word boundaries so short names cannot match inside
/// larger words; domains are matched as escaped literals.
fn mentions_monitored_entity(text: &str, entity_names: &[String]) -> bool {
    entity_names.iter().any(|name| {
        keyword_regex(name.trim(), true)
            .map(|re| re.is_match(text))
            .unwrap_or(false)
    })
}

/// Content-addressed, stable post id: `hex(sha256(forum.name + "|" + candidate))`.
///
/// The id does not depend on scan order or wall-clock time, so re-scanning the
/// same post yields the same id and downstream persistence dedupes it.
fn stable_post_id(forum_name: &str, candidate: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(forum_name.as_bytes());
    hasher.update(b"|");
    hasher.update(candidate.as_bytes());
    hex::encode(hasher.finalize())
}

/// Validate a URL taken from a remote response (e.g. the Pastebin scrape API)
/// before it is fetched or stored.
///
/// Response-derived URLs are attacker-influenced, and reqwest skips DNS for
/// IP-literal hosts, so a private/metadata literal would otherwise bypass the
/// guarded client's public-only resolver. Only absolute `http(s)` URLs whose
/// host is public survive; anything else is dropped.
fn safe_response_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    crate::http::url_allowed(&parsed).then(|| parsed.to_string())
}

/// Drop posts that repeat a content-addressed id, preserving first-seen order.
fn dedupe_posts_by_id(posts: Vec<DarkWebPost>) -> Vec<DarkWebPost> {
    let mut seen = HashSet::new();
    posts
        .into_iter()
        .filter(|post| seen.insert(post.id.clone()))
        .collect()
}

/// Best-effort extraction of a post date carried in the scraped text.
///
/// Returns `None` when no parseable date is present. Callers fall back to the
/// fetch time (the public `posted_at` type requires a value), which is safe
/// because post ids are content-addressed and re-scans do not re-report.
fn extract_posted_at(text: &str) -> Option<DateTime<Utc>> {
    let caps = RE_POST_DATE.captures(text)?;
    let year: i32 = caps.get(1)?.as_str().parse().ok()?;
    let month: u32 = caps.get(2)?.as_str().parse().ok()?;
    let day: u32 = caps.get(3)?.as_str().parse().ok()?;
    let hour: u32 = caps
        .get(4)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let minute: u32 = caps
        .get(5)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let second: u32 = caps
        .get(6)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, second)?;
    Some(DateTime::from_naive_utc_and_offset(
        date.and_time(time),
        Utc,
    ))
}

/// HTML tag name (lowercased, without attributes), or `""` for malformed tags.
fn html_tag_name(tag_body_lower: &str) -> &str {
    let body = tag_body_lower.trim_start_matches('/').trim_start();
    let end = body
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(body.len());
    &body[..end]
}

/// Whether a tag ends a text block and should emit a `\n\n` separator.
///
/// Closing tags for `p`/`div`/`li`/`tr` and the void tags `br`/`hr` mark post
/// boundaries; opening block tags must not add a separator before their text.
fn tag_ends_block(tag_body_lower: &str) -> bool {
    let name = html_tag_name(tag_body_lower);
    match name {
        "br" | "hr" => true,
        "p" | "div" | "li" | "tr" => tag_body_lower.trim_start().starts_with('/'),
        _ => false,
    }
}

/// Skip the content of a `script`/`style` element.
///
/// Returns the index of the closing `<`, or `chars.len()` when unterminated.
fn skip_element_content(chars: &[char], from: usize, name: &str) -> usize {
    let needle: Vec<char> = format!("</{name}")
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let mut i = from;
    while i + needle.len() <= chars.len() {
        let matched = chars[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(candidate, expected)| candidate.to_ascii_lowercase() == *expected);
        if matched {
            return i;
        }
        i += 1;
    }
    chars.len()
}

/// Collapse spaces/tabs only (newlines stay meaningful block separators),
/// trim each line, and reduce blank-line runs to a single blank line.
fn normalize_extracted_text(raw: &str) -> String {
    let mut spaced = String::with_capacity(raw.len());
    let mut prev_space = false;
    for ch in raw.chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_space {
                spaced.push(' ');
                prev_space = true;
            }
        } else {
            spaced.push(ch);
            prev_space = false;
        }
    }

    let mut cleaned = String::with_capacity(spaced.len());
    let mut pending_blank = false;
    for line in spaced.lines().map(str::trim) {
        if line.is_empty() {
            pending_blank = !cleaned.is_empty();
            continue;
        }
        if !cleaned.is_empty() {
            cleaned.push_str(if pending_blank { "\n\n" } else { "\n" });
        }
        pending_blank = false;
        cleaned.push_str(line);
    }
    cleaned
}

/// Simple HTML tag stripper for extracting text from scraped HTML.
///
/// Block-closing tags (`</p>`, `</div>`, `</li>`, `</tr>`, `<br>`, `<hr>`)
/// become `\n\n` separators so individual posts survive into
/// [`split_into_candidates`]; only spaces/tabs are collapsed, never newlines.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        if ch != '<' {
            result.push(ch);
            i += 1;
            continue;
        }

        // Unterminated tag: drop the malformed remainder.
        let Some(rel_end) = chars[i..].iter().position(|c| *c == '>') else {
            break;
        };
        let tag_end = i + rel_end;
        let tag_lower = chars[i + 1..tag_end]
            .iter()
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();

        let is_closing = tag_lower.trim_start().starts_with('/');
        if !is_closing && html_tag_name(&tag_lower) == "script" {
            i = skip_element_content(&chars, tag_end + 1, "script");
            continue;
        }
        if !is_closing && html_tag_name(&tag_lower) == "style" {
            i = skip_element_content(&chars, tag_end + 1, "style");
            continue;
        }

        if tag_ends_block(&tag_lower) {
            result.push('\n');
            result.push('\n');
        }
        i = tag_end + 1;
    }

    normalize_extracted_text(&result)
}

/// Whether a block is long/wordy enough to be a post rather than nav chrome.
fn is_post_candidate(block: &str) -> bool {
    block.chars().count() >= MIN_CANDIDATE_CHARS
        && block.split_whitespace().count() >= MIN_CANDIDATE_WORDS
}

/// Split extracted text into candidate post blocks.
///
/// Blocks below [`MIN_CANDIDATE_CHARS`]/[`MIN_CANDIDATE_WORDS`] (navigation
/// menus, boilerplate) are dropped where possible.
fn split_into_candidates(text: &str) -> Vec<String> {
    // Try common separators: double newlines, horizontal rules, etc.
    let mut candidates = Vec::new();

    // Split on common post separators
    for block in text.split("\n\n") {
        let trimmed = block.trim();
        if is_post_candidate(trimmed) {
            candidates.push(trimmed.to_string());
        }
    }

    // Only try other delimiters when the primary split found nothing: a valid
    // split must not be discarded just because it produced few posts.
    if candidates.is_empty() {
        for block in text.split("──") {
            let trimmed = block.trim();
            if is_post_candidate(trimmed) {
                candidates.push(trimmed.to_string());
            }
        }
    }

    // If still too few, treat the whole text as one candidate
    let whole = text.trim();
    if candidates.is_empty() && is_post_candidate(whole) {
        candidates.push(whole.to_string());
    }

    candidates
}

/// Match keywords in text (case-insensitive, word-bounded).
fn match_keywords(text: &str, keywords: &[String]) -> Vec<String> {
    keywords
        .iter()
        .filter(|kw| {
            keyword_regex(kw, true)
                .map(|re| re.is_match(text))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Snap a byte index down to the nearest UTF-8 character boundary.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut index = index;
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Snap a byte index up to the nearest UTF-8 character boundary.
fn ceil_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut index = index;
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Extract a snippet of text around the first matched keyword.
///
/// The search runs over the **original** text with a bounded, case-insensitive
/// regex and the resulting byte offsets are snapped to character boundaries,
/// so non-ASCII text (Cyrillic, Arabic, accented, …) cannot cause a slicing
/// panic. Falls back to the first `context_chars` characters when no keyword
/// is found.
fn extract_snippet(text: &str, matched_keywords: &[String], context_chars: usize) -> String {
    let context_chars = context_chars.min(MAX_SNIPPET_CONTEXT_CHARS);
    if matched_keywords.is_empty() {
        return text.chars().take(context_chars).collect();
    }

    let half = context_chars / 2;
    for keyword in matched_keywords {
        let Some(re) = keyword_regex(keyword, false) else {
            continue;
        };
        if let Some(found) = re.find(text) {
            let start = floor_char_boundary(text, found.start().saturating_sub(half));
            let end = ceil_char_boundary(text, found.end().saturating_add(half).min(text.len()));
            let snippet: String = text[start..end].chars().collect();
            return snippet.replace('\n', " ").trim().to_string();
        }
    }

    text.chars().take(context_chars).collect()
}

/// Truncate a string to a maximum length, appending "…" if truncated.
fn truncate_title(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_len - 1).collect();
    format!("{}…", truncated)
}

use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Forum type helpers ──────────────────────────────────────────────────

    #[test]
    fn forum_type_as_str() {
        assert_eq!(ForumType::General.as_str(), "general");
        assert_eq!(ForumType::Cardshop.as_str(), "cardshop");
        assert_eq!(ForumType::Exploit.as_str(), "exploit");
        assert_eq!(ForumType::Leak.as_str(), "leak");
        assert_eq!(ForumType::Ransomware.as_str(), "ransomware");
    }

    #[test]
    fn access_method_as_str() {
        assert_eq!(AccessMethod::Clearnet.as_str(), "clearnet");
        assert_eq!(AccessMethod::TorOnion.as_str(), "tor_onion");
        assert_eq!(AccessMethod::I2P.as_str(), "i2p");
        assert_eq!(AccessMethod::Telegram.as_str(), "telegram");
    }

    // ── Default forums ──────────────────────────────────────────────────────

    #[test]
    fn default_forums_are_seeded() {
        let forums = default_forums();
        assert!(!forums.is_empty(), "should have at least one default forum");
        let names: Vec<&str> = forums.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"Have I Been Pwned"));
        assert!(names.contains(&"Pastebin"));
    }

    // ── Monitor construction ────────────────────────────────────────────────

    #[test]
    fn monitor_constructs_without_tor() {
        let monitor = DarkWebMonitor::new(None)
            .unwrap_or_else(|e| panic!("monitor should build without Tor: {e}"));
        assert!(!monitor.has_tor_proxy());
        assert!(!monitor.forums.is_empty());
        assert!(monitor.rules.is_empty());
    }

    #[test]
    fn monitor_constructs_with_invalid_tor_url() {
        // A URL string with invalid characters should fail Proxy::all parsing
        let result = DarkWebMonitor::new(Some("socks5://invalid proxy url".into()));
        assert!(result.is_err(), "invalid proxy URL should fail: {result:?}");
    }

    #[test]
    fn monitor_constructs_with_tor() {
        let monitor = DarkWebMonitor::new(Some("socks5://127.0.0.1:9050".into()))
            .unwrap_or_else(|e| panic!("monitor should build with Tor: {e}"));
        assert!(monitor.has_tor_proxy());
    }

    // ── Set forums / rules ──────────────────────────────────────────────────

    #[test]
    fn set_forums_replaces_default() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_forums(vec![]);
        assert!(monitor.forums.is_empty());
    }

    #[test]
    fn add_rule() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        let rule = MonitoringRule {
            id: "test-1".into(),
            name: "Test Rule".into(),
            keywords: vec!["acme".into(), "breach".into()],
            entity_ids: vec!["uuid-1".into()],
            min_relevance: 0.5,
            notification_channels: vec!["slack".into()],
        };
        monitor.add_rule(rule);
        assert_eq!(monitor.rules.len(), 1);
    }

    // ── Relevance scoring ───────────────────────────────────────────────────

    #[test]
    fn relevance_score_with_no_keywords_is_zero() {
        let score = DarkWebMonitor::compute_relevance_score(
            &[],
            &[],
            "some random text with no matches",
            Utc::now(),
            10.0,
        );
        assert!(score < 0.5);
    }

    #[test]
    fn relevance_score_with_all_keywords() {
        let matched: Vec<String> = (0..10).map(|i| format!("keyword_{}", i)).collect();
        let score =
            DarkWebMonitor::compute_relevance_score(&matched, &[], "text", Utc::now(), 10.0);
        assert!(score >= 0.9, "score should be high: {score}");
    }

    #[test]
    fn relevance_score_boosted_by_high_priority_keywords() {
        let content = "Critical data breach and leak detected in supply chain";
        let matched = vec!["breach".to_string(), "leak".to_string()];
        let score =
            DarkWebMonitor::compute_relevance_score(&matched, &[], content, Utc::now(), 10.0);
        let score_no_boost = DarkWebMonitor::compute_relevance_score(
            &matched,
            &[],
            "nothing important here",
            Utc::now(),
            10.0,
        );
        assert!(
            score > score_no_boost,
            "high-priority keywords should boost score: {score} > {score_no_boost}"
        );
    }

    #[test]
    fn relevance_score_boosted_by_entities() {
        let score_with = DarkWebMonitor::compute_relevance_score(
            &["breach".to_string()],
            &[
                "attacker@example.com".to_string(),
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string(),
            ],
            "breach",
            Utc::now(),
            10.0,
        );
        let score_without = DarkWebMonitor::compute_relevance_score(
            &["breach".to_string()],
            &[],
            "breach",
            Utc::now(),
            10.0,
        );
        assert!(
            score_with > score_without,
            "entities should boost score: {score_with} > {score_without}"
        );
    }

    #[test]
    fn relevance_score_is_clamped() {
        let matched: Vec<String> = (0..100).map(|i| format!("kw_{}", i)).collect();
        let score = DarkWebMonitor::compute_relevance_score(
            &matched,
            &vec!["a".to_string(); 10],
            "breach leak ransom exploit vulnerability supply chain",
            Utc::now(),
            10.0,
        );
        assert!(score <= 1.0, "score must not exceed 1.0: {score}");
        assert!(score >= 0.0, "score must not be negative: {score}");
    }

    #[test]
    fn score_with_recency_fresh_post() {
        let fresh = Utc::now();
        let score = DarkWebMonitor::score_with_recency(fresh, 0.5);
        assert!(score > 0.5, "fresh post should have recency boost: {score}");
    }

    #[test]
    fn score_with_recency_old_post() {
        let old = Utc::now() - chrono::Duration::days(30);
        let score = DarkWebMonitor::score_with_recency(old, 0.5);
        assert!(score < 0.5, "old post should have decayed score: {score}");
    }

    #[test]
    fn score_with_recency_clamped() {
        let very_old = Utc::now() - chrono::Duration::days(365 * 10);
        let score = DarkWebMonitor::score_with_recency(very_old, 0.1);
        assert!(score >= 0.0, "decayed score should not go below 0: {score}");
    }

    // ── Entity extraction ───────────────────────────────────────────────────

    #[test]
    fn extract_emails() {
        let text = "Contact: attacker@example.com or admin@dark-web.org";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.contains(&"attacker@example.com".to_string()));
        assert!(entities.contains(&"admin@dark-web.org".to_string()));
    }

    #[test]
    fn extract_domains() {
        let text = "Visit https://evil-site.onion or http://malware.cc for details";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.iter().any(|e| e.contains("evil-site.onion")));
        assert!(entities.iter().any(|e| e.contains("malware.cc")));
    }

    #[test]
    fn extract_bitcoin_addresses() {
        let text = "Send BTC to 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa for access";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.contains(&"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string()));
    }

    #[test]
    fn extract_ethereum_addresses() {
        let text = "ETH: 0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.contains(&"0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18".to_string()));
    }

    #[test]
    fn extract_ipv4_addresses() {
        let text = concat!(
            "Server at 192.168.1.1 (internal), 172.16.5.5 (internal), ",
            "169.254.169.254 (metadata), 127.0.0.1 (loopback) and ",
            "203.0.113.5 (public)"
        );
        let entities = DarkWebMonitor::extract_entities(text);
        for private in ["192.168.1.1", "172.16.5.5", "169.254.169.254", "127.0.0.1"] {
            assert!(
                !entities.contains(&private.to_string()),
                "private/metadata address {private} must be filtered"
            );
        }
        assert!(entities.contains(&"203.0.113.5".to_string()));
    }

    #[test]
    fn extract_telegram_handles() {
        let text = "Join t.me/leak_channel for more data";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.contains(&"t.me/leak_channel".to_string()));
    }

    #[test]
    fn extract_multiple_entity_types() {
        let text = concat!(
            "Posted by dark_hacker@protonmail.com. ",
            "Breach data at https://example-leak.cc. ",
            "BTC: 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa. ",
            "Chat: t.me/hacker_chat"
        );
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.contains(&"dark_hacker@protonmail.com".to_string()));
        assert!(entities.contains(&"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string()));
        assert!(entities.contains(&"t.me/hacker_chat".to_string()));
    }

    #[test]
    fn extract_no_false_positives() {
        let text = "This is a normal sentence without any indicators.";
        let entities = DarkWebMonitor::extract_entities(text);
        assert!(entities.is_empty() || entities.iter().all(|e| e.len() < 5));
    }

    #[test]
    fn entity_deduplication() {
        let text = "Email: user@example.com and USER@example.com";
        let entities = DarkWebMonitor::extract_entities(text);
        let count = entities
            .iter()
            .filter(|e| e.contains("example.com"))
            .count();
        assert_eq!(
            count, 1,
            "duplicate emails should be deduplicated: {entities:?}"
        );
    }

    // ── Keyword matching ────────────────────────────────────────────────────

    #[test]
    fn keyword_matching_case_insensitive() {
        let keywords = vec!["Breach".to_string(), "LEAK".to_string()];
        let text = "This is a BREACH and leak notification";
        let matched = match_keywords(text, &keywords);
        assert!(matched.contains(&"Breach".to_string()));
        assert!(matched.contains(&"LEAK".to_string()));
    }

    #[test]
    fn keyword_matching_no_match() {
        let keywords = vec!["acme".to_string(), "corp".to_string()];
        let text = "nothing about this in the text";
        let matched = match_keywords(text, &keywords);
        assert!(matched.is_empty());
    }

    #[test]
    fn keyword_matching_no_partial_word() {
        let keywords = vec!["pass".to_string()];
        let text = "The password is secret";
        let matched = match_keywords(text, &keywords);
        assert!(
            !matched.contains(&"pass".to_string()),
            "word-bounded matching must not match 'pass' inside 'password': {matched:?}"
        );
    }

    // ── Rule matching ───────────────────────────────────────────────────────

    #[test]
    fn rule_match_below_threshold() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let post = DarkWebPost {
            id: "test-1".into(),
            forum_name: "TestForum".into(),
            thread_title: "Test".into(),
            author: "tester".into(),
            content_snippet: "nothing relevant".into(),
            posted_at: Utc::now(),
            url: "https://example.com".into(),
            matched_keywords: vec![],
            relevance_score: 0.3,
            entities_mentioned: vec![],
        };
        let rule = MonitoringRule {
            id: "rule-1".into(),
            name: "High Relevance".into(),
            keywords: vec!["relevant".into()],
            entity_ids: vec![],
            min_relevance: 0.8,
            notification_channels: vec![],
        };
        assert!(!monitor.matches_rule(&post, &rule));
    }

    #[test]
    fn rule_match_above_threshold() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let post = DarkWebPost {
            id: "test-2".into(),
            forum_name: "TestForum".into(),
            thread_title: "Important Breach Alert".into(),
            author: "hacker".into(),
            content_snippet: "Major breach at target company with data leak".into(),
            posted_at: Utc::now(),
            url: "https://example.com/post".into(),
            matched_keywords: vec!["breach".into(), "leak".into()],
            relevance_score: 0.9,
            entities_mentioned: vec!["target@example.com".into()],
        };
        let rule = MonitoringRule {
            id: "rule-2".into(),
            name: "Breach Alert".into(),
            keywords: vec!["breach".into(), "leak".into()],
            entity_ids: vec![],
            min_relevance: 0.5,
            notification_channels: vec!["slack".into()],
        };
        assert!(monitor.matches_rule(&post, &rule));
    }

    #[test]
    fn rule_match_empty_keywords_in_post() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let post = DarkWebPost {
            id: "test-3".into(),
            forum_name: "Test".into(),
            thread_title: "Hello world".into(),
            author: "user".into(),
            content_snippet: "Just a friendly hello".into(),
            posted_at: Utc::now(),
            url: "https://example.com".into(),
            matched_keywords: vec![],
            relevance_score: 0.9,
            entities_mentioned: vec![],
        };
        let rule = MonitoringRule {
            id: "rule-3".into(),
            name: "Test".into(),
            keywords: vec!["breach".into()],
            entity_ids: vec![],
            min_relevance: 0.5,
            notification_channels: vec![],
        };
        assert!(!monitor.matches_rule(&post, &rule));
    }

    // ── HTML stripping ──────────────────────────────────────────────────────

    #[test]
    fn strip_html_simple() {
        let html = "<p>Hello <b>world</b></p>";
        let text = strip_html_tags(html);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn strip_html_script_content() {
        let html = "<p>Visible</p><script>alert('hidden')</script><p>More</p>";
        let text = strip_html_tags(html);
        assert!(!text.contains("hidden"));
        assert!(text.contains("Visible"));
        assert!(text.contains("More"));
    }

    #[test]
    fn strip_html_style_content() {
        let html = "<p>Text</p><style>.hidden{display:none}</style><p>After</p>";
        let text = strip_html_tags(html);
        assert!(!text.contains("hidden"));
        assert!(text.contains("Text"));
        assert!(text.contains("After"));
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        let html = "<div>\n  <p>Line1</p>\n  <p>  Line2  </p>\n</div>";
        let text = strip_html_tags(html);
        // `</p>`/`</div>` are block separators, so the two paragraphs stay
        // distinct candidates instead of being joined by a space.
        assert_eq!(text, "Line1\n\nLine2");
    }

    // ── Snippet extraction ──────────────────────────────────────────────────

    #[test]
    fn extract_snippet_around_keyword() {
        let text = "This is a long text that contains the word breach somewhere in the middle of the content";
        let snippet = extract_snippet(text, &["breach".to_string()], 40);
        assert!(snippet.contains("breach"));
        assert!(snippet.len() <= text.len());
    }

    #[test]
    fn extract_snippet_no_keywords() {
        let text = "Short text";
        let snippet = extract_snippet(text, &[], 200);
        assert_eq!(snippet, "Short text");
    }

    // ── Default monitoring keywords ─────────────────────────────────────────

    #[test]
    fn default_keywords_contains_common_terms() {
        let kws = default_monitoring_keywords();
        assert!(kws.contains(&"breach".to_string()));
        assert!(kws.contains(&"leak".to_string()));
        assert!(kws.contains(&"ransom".to_string()));
        assert!(kws.contains(&"exploit".to_string()));
        assert!(kws.contains(&"vulnerability".to_string()));
        assert!(kws.contains(&"supply chain".to_string()));
    }

    // ── Empty / error handling ──────────────────────────────────────────────

    #[test]
    fn scan_all_with_no_active_forums_returns_empty() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        // Make all forums inactive
        for forum in &mut monitor.forums {
            forum.is_active = false;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let posts = rt.block_on(monitor.scan_all());
        assert!(posts.is_empty());
    }

    #[test]
    fn scan_all_with_empty_forum_list_returns_empty() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_forums(vec![]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let posts = rt.block_on(monitor.scan_all());
        assert!(posts.is_empty());
    }

    #[test]
    fn scan_all_detailed_counts_failed_forum() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_forums(vec![DarkWebForum {
            name: "Unreachable Forum".into(),
            // Port 1 is never bound; the connection is refused immediately.
            base_url: "http://127.0.0.1:1".into(),
            forum_type: ForumType::General,
            access_method: AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec![],
        }]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let report = rt.block_on(monitor.scan_all_detailed());
        assert!(report.posts.is_empty());
        assert_eq!(report.forums_scanned, 0);
        assert_eq!(report.forums_failed, 1);
    }

    #[test]
    fn scan_all_detailed_with_no_active_forums_reports_zero() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        for forum in &mut monitor.forums {
            forum.is_active = false;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let report = rt.block_on(monitor.scan_all_detailed());
        assert!(report.posts.is_empty());
        assert_eq!(report.forums_scanned, 0);
        assert_eq!(report.forums_failed, 0);
    }

    #[test]
    fn split_into_candidates_short_text() {
        let candidates = split_into_candidates("");
        assert!(candidates.is_empty());
    }

    #[test]
    fn split_into_candidates_long_text() {
        let text = "This is a sufficiently long text that should be treated as a single candidate since it doesn't have double newlines or separators.";
        let candidates = split_into_candidates(text);
        assert!(!candidates.is_empty());
    }

    // ── Truncation ──────────────────────────────────────────────────────────

    #[test]
    fn truncate_title_short() {
        let result = truncate_title("Short", 100);
        assert_eq!(result, "Short");
    }

    #[test]
    fn truncate_title_long() {
        let long = "A".repeat(200);
        let result = truncate_title(&long, 10);
        assert_eq!(result.chars().count(), 10);
        assert!(result.ends_with('…'));
    }

    // ── calculate_relevance on monitor ──────────────────────────────────────

    #[test]
    fn calculate_relevance_via_monitor() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let post = DarkWebPost {
            id: "test".into(),
            forum_name: "Test".into(),
            thread_title: "Data breach at major company".into(),
            author: "hacker".into(),
            content_snippet: "breach leak exploit vulnerability supply chain".into(),
            posted_at: Utc::now(),
            url: "https://example.com".into(),
            matched_keywords: vec![
                "breach".into(),
                "leak".into(),
                "exploit".into(),
                "vulnerability".into(),
            ],
            relevance_score: 0.0,
            entities_mentioned: vec!["admin@company.com".into()],
        };
        let score = monitor.calculate_relevance(&post);
        assert!(score > 0.0, "relevance should be > 0, got {score}");
        assert!(score <= 1.0, "relevance should be <= 1.0, got {score}");
    }

    // ── Shared test fixtures ────────────────────────────────────────────────

    fn test_forum(name: &str) -> DarkWebForum {
        DarkWebForum {
            name: name.into(),
            base_url: "https://example.com/forum".into(),
            forum_type: ForumType::Leak,
            access_method: AccessMethod::Clearnet,
            is_active: true,
            last_checked: None,
            topics_of_interest: vec![],
        }
    }

    // ── Item 85: DNS/identity leaks ─────────────────────────────────────────

    #[test]
    fn dark_web_user_agent_is_generic() {
        assert!(
            !DARK_WEB_USER_AGENT
                .to_ascii_lowercase()
                .contains("apexintel"),
            "scraper User-Agent must not identify ApexIntel: {DARK_WEB_USER_AGENT}"
        );
        assert!(DARK_WEB_USER_AGENT.starts_with("Mozilla/5.0"));
    }

    #[test]
    fn legacy_socks5_proxy_url_is_rewritten_to_socks5h() {
        let monitor = DarkWebMonitor::new(Some("socks5://127.0.0.1:9050".into())).unwrap();
        assert!(monitor.has_tor_proxy());
        assert_eq!(
            monitor.tor_proxy_url.as_deref(),
            Some("socks5h://127.0.0.1:9050"),
            "socks5:// must be rewritten so DNS resolves through Tor"
        );
    }

    #[test]
    fn explicit_socks5h_proxy_url_is_preserved() {
        let monitor = DarkWebMonitor::new(Some("socks5h://127.0.0.1:9050".into())).unwrap();
        assert_eq!(
            monitor.tor_proxy_url.as_deref(),
            Some("socks5h://127.0.0.1:9050")
        );
    }

    #[test]
    fn default_criminal_forums_are_inactive_and_not_clearnet_safe() {
        let forums = default_forums();
        for name in ["BreachForums", "Exploit.in"] {
            let forum = forums
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("{name} must be seeded"));
            assert!(!forum.is_active, "{name} must be seeded inactive");
            assert!(
                !forum.is_clearnet_safe(),
                "{name} must not be clearnet-safe"
            );
        }
    }

    #[test]
    fn default_research_surfaces_are_clearnet_safe() {
        let forums = default_forums();
        for name in ["Have I Been Pwned", "Pastebin", "Ransomware Blog (Generic)"] {
            let forum = forums
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("{name} must be seeded"));
            assert!(forum.is_clearnet_safe(), "{name} should be clearnet-safe");
        }
    }

    #[test]
    fn clearnet_scan_of_unmarked_forum_is_refused_without_proxy() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let mut forum = test_forum("Criminal Forum");
        forum.base_url = "https://breachforums.st".into();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(monitor.scan_forum(&forum));
        assert!(
            result.is_err(),
            "clearnet scan of a non-clearnet-safe forum must be refused: {result:?}"
        );
    }

    // ── Item 86: snippet extraction on non-ASCII text ───────────────────────

    #[test]
    fn extract_snippet_cyrillic_text() {
        let text = "Привет, мир. На форуме опубликована утечка данных breach компании Acme";
        let snippet = extract_snippet(text, &["breach".to_string()], 80);
        assert!(snippet.contains("breach"), "snippet: {snippet:?}");
    }

    #[test]
    fn extract_snippet_cyrillic_keyword() {
        let text = "Сегодня зафиксирована УТЕЧКА данных компании";
        let snippet = extract_snippet(text, &["утечка".to_string()], 40);
        assert!(snippet.contains("УТЕЧКА"), "snippet: {snippet:?}");
    }

    #[test]
    fn extract_snippet_arabic_text() {
        let text = "مرحبا بالعالم تم نشر بيانات مسربة breach في المنتدى";
        let snippet = extract_snippet(text, &["breach".to_string()], 12);
        assert!(snippet.contains("breach"), "snippet: {snippet:?}");
    }

    #[test]
    fn extract_snippet_accented_text_does_not_panic() {
        // Lowercasing `İ` yields two chars, so the old byte-offset slicing
        // panicked mid-character; the regex/char-boundary version must not.
        let text = "Aİbreach target";
        let snippet = extract_snippet(text, &["breach".to_string()], 4);
        assert!(snippet.contains("breach"), "snippet: {snippet:?}");
    }

    #[test]
    fn extract_snippet_no_keyword_falls_back_to_first_chars() {
        let text = "Hello world, this is a longer text without any monitored keyword";
        let snippet = extract_snippet(text, &[], 5);
        assert_eq!(snippet, "Hello");
    }

    // ── Item 87: real posts, stable ids ─────────────────────────────────────

    #[test]
    fn strip_html_block_tags_separate_blocks() {
        let text = strip_html_tags("<div><p>First post body</p><p>Second post body</p></div>");
        assert_eq!(text, "First post body\n\nSecond post body");
    }

    #[test]
    fn strip_html_void_break_tags_emit_separator() {
        assert_eq!(strip_html_tags("<p>alpha<br>beta</p>"), "alpha\n\nbeta");
        assert_eq!(
            strip_html_tags("<p>alpha</p><hr><p>beta</p>"),
            "alpha\n\nbeta"
        );
    }

    #[test]
    fn strip_html_preserves_single_newlines() {
        assert_eq!(
            strip_html_tags("<p>line one\nline two</p>"),
            "line one\nline two"
        );
    }

    #[test]
    fn split_into_candidates_drops_navigation_and_finds_posts() {
        let first = "This is the first real forum post with plenty of words to pass the minimum length filter for candidates.";
        let second = "This is the second real forum post that also carries enough words and characters to be treated as content.";
        let text = format!("Home Forum Login\n\n{first}\n\n{second}");
        let candidates = split_into_candidates(&text);
        assert_eq!(candidates.len(), 2, "candidates: {candidates:?}");
        assert_eq!(candidates[0], first);
        assert_eq!(candidates[1], second);
    }

    #[test]
    fn stable_post_id_is_content_addressed() {
        let a = stable_post_id("BreachForums", "same candidate text");
        let b = stable_post_id("BreachForums", "same candidate text");
        let other_text = stable_post_id("BreachForums", "different candidate text");
        let other_forum = stable_post_id("Exploit.in", "same candidate text");
        assert_eq!(a, b, "same forum + candidate must yield the same id");
        assert_ne!(a, other_text, "different text must yield a different id");
        assert_ne!(a, other_forum, "different forum must yield a different id");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn response_urls_cannot_point_at_private_or_non_http_targets() {
        // The Pastebin scrape API's `scrape_url`/`full_url` values are
        // attacker-influenced: IP-literal hosts skip DNS, so they must be
        // classified before the guarded client ever sees them.
        for blocked in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/admin",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://[::1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://localhost/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<p>x</p>",
            "not a url",
        ] {
            assert_eq!(
                safe_response_url(blocked),
                None,
                "must be dropped: {blocked}"
            );
        }
        assert_eq!(
            safe_response_url("https://pastebin.com/raw/abc").as_deref(),
            Some("https://pastebin.com/raw/abc")
        );
        // Onion hostnames are public DNS names (reachable only through Tor)
        // and must not be swept up by the private-host classifier.
        assert!(safe_response_url("http://pasteexample.onion/raw/x").is_some());
    }

    #[test]
    fn build_posts_ids_stable_across_calls_and_deduped() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_entities(vec!["Acme Corp".to_string()]);
        let forum = test_forum("TestForum");
        let candidate =
            "Acme Corp breach: customer credentials and password dump offered for sale".to_string();

        let first = monitor.build_posts_from_candidates(&forum, std::slice::from_ref(&candidate));
        let second = monitor.build_posts_from_candidates(&forum, std::slice::from_ref(&candidate));
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].id, second[0].id, "ids must be stable across calls");
        assert_eq!(first[0].id, stable_post_id(&forum.name, &candidate));

        let dupes = monitor.build_posts_from_candidates(&forum, &[candidate.clone(), candidate]);
        assert_eq!(dupes.len(), 1, "identical candidates must dedupe by id");
    }

    #[test]
    fn extract_posted_at_reads_page_date() {
        let date = extract_posted_at("Posted on 2024-03-15 by leaker").expect("date");
        assert_eq!(date.to_rfc3339(), "2024-03-15T00:00:00+00:00");
        let dt = extract_posted_at("2024-03-15 14:05:30 UTC").expect("datetime");
        assert_eq!(dt.to_rfc3339(), "2024-03-15T14:05:30+00:00");
        assert!(extract_posted_at("no date carried here").is_none());
    }

    // ── Item 88: word-bounded keywords and entity gate ──────────────────────

    #[test]
    fn rat_keyword_does_not_match_inside_words() {
        let keywords = vec!["rat".to_string()];
        for text in [
            "generation of reports",
            "separate the rate",
            "a moderate rate",
        ] {
            assert!(
                match_keywords(text, &keywords).is_empty(),
                "'rat' must not match inside {text:?}"
            );
        }
        assert!(
            match_keywords("a rat in the cellar", &keywords).contains(&"rat".to_string()),
            "standalone 'rat' must match"
        );
    }

    #[test]
    fn default_keywords_exclude_generic_terms() {
        let kws = default_monitoring_keywords();
        for generic in [
            "access", "admin", "config", "proxy", "vpn", "database", "shell", "crawl", "scrape",
            "spider", "c2", "cnc", "dump", "combo", "exposed", "payload",
        ] {
            assert!(
                !kws.contains(&generic.to_string()),
                "near-universal term {generic:?} must be removed"
            );
        }
        assert!(kws.contains(&"breach".to_string()));
    }

    #[test]
    fn candidate_without_monitored_entity_is_not_posted() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_entities(vec!["Acme Corp".to_string()]);
        let forum = test_forum("TestForum");
        let candidates = vec![
            "This breach thread discusses leaked credentials in general but never names the monitored customer".to_string(),
        ];
        assert!(
            monitor
                .build_posts_from_candidates(&forum, &candidates)
                .is_empty(),
            "candidate without a monitored entity must not become a post"
        );
    }

    #[test]
    fn candidate_with_monitored_entity_is_posted() {
        let mut monitor = DarkWebMonitor::new(None).unwrap();
        monitor.set_entities(vec!["Acme Corp".to_string()]);
        let forum = test_forum("TestForum");
        let candidates = vec![
            "This breach thread leaks Acme Corp credentials and password dumps for sale now"
                .to_string(),
        ];
        let posts = monitor.build_posts_from_candidates(&forum, &candidates);
        assert_eq!(posts.len(), 1, "entity match must produce a post");
        assert_eq!(posts[0].forum_name, "TestForum");
    }

    #[test]
    fn no_monitored_entities_means_no_posts() {
        let monitor = DarkWebMonitor::new(None).unwrap();
        let forum = test_forum("TestForum");
        let candidates = vec![
            "This breach thread mentions Acme Corp but no entities are configured".to_string(),
        ];
        assert!(
            monitor
                .build_posts_from_candidates(&forum, &candidates)
                .is_empty(),
            "an empty entity list must suppress all posts"
        );
    }

    #[test]
    fn entity_matching_uses_word_boundaries() {
        assert!(mentions_monitored_entity(
            "data from acme-corp.com leaked",
            &["acme-corp.com".to_string()]
        ));
        assert!(!mentions_monitored_entity(
            "not related at all",
            &["acme-corp.com".to_string()]
        ));
        assert!(!mentions_monitored_entity("anything", &[]));
    }
}
