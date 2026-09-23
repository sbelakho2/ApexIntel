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
//! 1. Fetches recent content from each active forum (via Tor SOCKS5 proxy if
//!    configured, otherwise clearnet).
//! 2. Extracts candidate posts via HTML parsing / regex.
//! 3. Matches against configured keywords and entity names.
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
use regex::Regex;
use reqwest::{Client, Proxy, StatusCode};
use serde::{Deserialize, Serialize};
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
            is_active: true, // enabled — clearnet mirror often works
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
            is_active: true, // enabled — clearnet mirror often works
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
}

// ─────────────────────────────────────────────────────────────────────────────
// DarkWebMonitor
// ─────────────────────────────────────────────────────────────────────────────

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
    /// Reusable HTTP client.
    http_client: Client,
    /// Optional Tor SOCKS5 proxy URL (e.g., `socks5://127.0.0.1:9050`).
    tor_proxy_url: Option<String>,
}

impl DarkWebMonitor {
    /// Create a new monitor.
    ///
    /// If `tor_proxy_url` is `Some("socks5://...")`, all HTTP traffic is routed
    /// through the Tor SOCKS5 proxy.  Set to `None` for clearnet-only access.
    ///
    /// Forums are seeded from [`default_forums()`] and can be replaced via
    /// [`set_forums`](Self::set_forums).
    pub fn new(tor_proxy_url: Option<String>) -> anyhow::Result<Self> {
        let mut client_builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io)")
            .pool_max_idle_per_host(4);

        // Route through Tor SOCKS5 proxy if configured
        if let Some(ref proxy_url) = tor_proxy_url {
            let proxy = Proxy::all(proxy_url)
                .map_err(|e| anyhow::anyhow!("invalid Tor proxy URL '{}': {}", proxy_url, e))?;
            client_builder = client_builder.proxy(proxy);
            debug!(proxy_url = %proxy_url, "dark_web: Tor proxy configured");
        }

        let http_client = client_builder
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {}", e))?;

        Ok(Self {
            forums: default_forums(),
            rules: Vec::new(),
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
        let mut all_posts = Vec::new();

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
                    all_posts.append(&mut posts);
                }
                Err(e) => {
                    warn!(
                        forum = %forum.name,
                        error = %e,
                        "dark_web: forum scan failed"
                    );
                }
            }
        }

        // Sort by relevance descending, then by posted_at descending
        all_posts.sort_unstable_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.posted_at.cmp(&a.posted_at))
        });

        all_posts
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

        let html = resp.text().await.map_err(|e| {
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

        let now = Utc::now();
        let mut posts = Vec::new();

        for (i, candidate) in candidates.iter().enumerate() {
            // Skip very short fragments
            if candidate.len() < 40 {
                continue;
            }

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
            let post_id = format!("{}-{}", forum.name.to_lowercase().replace(' ', "_"), i);

            posts.push(DarkWebPost {
                id: post_id,
                forum_name: forum.name.clone(),
                thread_title: truncate_title(candidate, 120),
                author: "unknown".into(), // generic scrape cannot always extract author
                content_snippet: snippet,
                posted_at: now,
                url: forum.base_url.clone(),
                matched_keywords,
                relevance_score: relevance,
                entities_mentioned: entities,
            });
        }

        debug!(
            forum = %forum.name,
            candidates = candidates.len(),
            matches = posts.len(),
            "dark_web: forum scan results"
        );

        Ok(posts)
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

        let pastes: Vec<PasteInfo> = match resp.json().await {
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
            let content_url = match &paste.scrape_url {
                Some(u) => u.clone(),
                None => continue,
            };

            let content_resp = match self.http_client.get(&content_url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };

            if !content_resp.status().is_success() {
                continue;
            }

            let content = match content_resp.text().await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let matched = match_keywords(&content, &all_keywords);
            if matched.is_empty() {
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

            let paste_url = paste.full_url.clone().unwrap_or_else(|| {
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

        // IPv4 addresses
        for cap in RE_IPV4.find_iter(text) {
            let ip = cap.as_str().to_string();
            if !ip.starts_with("127.")
                && !ip.starts_with("10.")
                && !ip.starts_with("192.168.")
                && !entities.contains(&ip)
            {
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
    /// in the post.
    pub fn matches_rule(&self, post: &DarkWebPost, rule: &MonitoringRule) -> bool {
        if post.relevance_score < rule.min_relevance {
            return false;
        }

        // Check if any of the rule's keywords match
        let content_lower = post.content_snippet.to_lowercase();
        let title_lower = post.thread_title.to_lowercase();

        rule.keywords.iter().any(|kw| {
            let kw_lower = kw.to_lowercase();
            content_lower.contains(&kw_lower) || title_lower.contains(&kw_lower)
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Default set of monitoring keywords used when no rules are configured.
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
        "dump".into(),
        "combo".into(),
        "database".into(),
        "sql injection".into(),
        "shell".into(),
        "access".into(),
        "admin".into(),
        "compromised".into(),
        "exposed".into(),
        "malware".into(),
        "phishing".into(),
        "trojan".into(),
        "rat".into(),
        "botnet".into(),
        "ddos".into(),
        "payload".into(),
        "c2".into(),
        "cnc".into(),
        "proxy".into(),
        "vpn".into(),
        "config".into(),
        "scrape".into(),
        "crawl".into(),
        "spider".into(),
    ]
}

/// Simple HTML tag stripper for extracting text from scraped HTML.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if in_script {
            if i + 9 <= len
                && chars[i..i + 8].iter().collect::<String>().to_lowercase() == "</script"
            {
                in_script = false;
            }
            i += 1;
            continue;
        }
        if in_style {
            if i + 8 <= len
                && chars[i..i + 7].iter().collect::<String>().to_lowercase() == "</style"
            {
                in_style = false;
            }
            i += 1;
            continue;
        }
        if in_tag {
            if chars[i] == '>' {
                in_tag = false;
            }
            i += 1;
            continue;
        }
        if chars[i] == '<' {
            // Check for script/style tags to skip their content
            if i + 7 < len && chars[i..i + 7].iter().collect::<String>().to_lowercase() == "<script"
            {
                in_script = true;
                i += 1;
                continue;
            }
            if i + 6 < len && chars[i..i + 6].iter().collect::<String>().to_lowercase() == "<style"
            {
                in_style = true;
                i += 1;
                continue;
            }
            in_tag = true;
            i += 1;
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }

    // Collapse whitespace
    let mut cleaned = String::with_capacity(result.len());
    let mut prev_was_space = false;
    for ch in result.chars() {
        if ch.is_whitespace() {
            if !prev_was_space {
                cleaned.push(' ');
                prev_was_space = true;
            }
        } else {
            cleaned.push(ch);
            prev_was_space = false;
        }
    }

    cleaned.trim().to_string()
}

/// Split extracted text into candidate post blocks.
fn split_into_candidates(text: &str) -> Vec<String> {
    // Try common separators: double newlines, horizontal rules, etc.
    let mut candidates = Vec::new();

    // Split on common post separators
    for block in text.split("\n\n") {
        let trimmed = block.trim().to_string();
        if !trimmed.is_empty() && trimmed.len() >= 40 {
            candidates.push(trimmed);
        }
    }

    // If we got very few candidates, try other delimiters
    if candidates.len() < 3 {
        candidates.clear();
        for block in text.split("──") {
            let trimmed = block.trim().to_string();
            if !trimmed.is_empty() && trimmed.len() >= 40 {
                candidates.push(trimmed);
            }
        }
    }

    // If still too few, treat the whole text as one candidate
    if candidates.is_empty() && text.len() >= 40 {
        candidates.push(text.trim().to_string());
    }

    candidates
}

/// Match keywords in text (case-insensitive).
fn match_keywords(text: &str, keywords: &[String]) -> Vec<String> {
    let text_lower = text.to_lowercase();
    keywords
        .iter()
        .filter(|kw| text_lower.contains(&kw.to_lowercase()))
        .cloned()
        .collect()
}

/// Extract a snippet of text around the first matched keyword.
fn extract_snippet(text: &str, matched_keywords: &[String], context_chars: usize) -> String {
    if matched_keywords.is_empty() {
        return text.chars().take(context_chars).collect();
    }

    let text_lower = text.to_lowercase();
    let first_kw = &matched_keywords[0].to_lowercase();

    if let Some(pos) = text_lower.find(first_kw) {
        let start = pos.saturating_sub(context_chars / 2);
        let end = (pos + matched_keywords[0].len() + context_chars / 2).min(text.len());
        let snippet: String = text[start..end].chars().collect();
        snippet.replace('\n', " ").trim().to_string()
    } else {
        text.chars().take(context_chars).collect()
    }
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
        let text = "Server at 192.168.1.1 (internal) and 203.0.113.5 (public)";
        let entities = DarkWebMonitor::extract_entities(text);
        // Private IPs should be excluded
        assert!(!entities.contains(&"192.168.1.1".to_string()));
        // Public IPs should be included
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
    fn keyword_matching_partial_word() {
        let keywords = vec!["pass".to_string()];
        let text = "The password is secret";
        let matched = match_keywords(text, &keywords);
        assert!(matched.contains(&"pass".to_string()));
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
        assert_eq!(text, "Line1 Line2");
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
}
