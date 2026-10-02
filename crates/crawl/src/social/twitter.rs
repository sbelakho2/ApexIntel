//! Twitter/X v2 API scraper.
//!
//! Wraps the Twitter API v2 recent-search and user-timeline endpoints.
//! Uses bearer token authentication (free/basic tier).  Falls back to
//! scraping nitter.net mirror instances when the API is unavailable.
//!
//! # Env vars expected
//! - `TWITTER_BEARER_TOKEN` — Twitter API v2 bearer token (optional; uses
//!   nitter fallback if absent)

use super::SocialPost;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::acquisition::{AcquisitionOutcome, AdapterPrerequisite, SourceAdapter};

const TWITTER_API_BASE: &str = "https://api.twitter.com/2";
/// Public nitter instances used as fallback when bearer token is unavailable.
const NITTER_INSTANCES: &[&str] = &[
    "https://nitter.privacydev.net",
    "https://nitter.poast.org",
    "https://nitter.nl",
    "https://nitter.cz",
    "https://nitter.1d4.us",
];

/// Search queries for OSINT-relevant Twitter content (used with v2 search or nitter).
pub const OSINT_TWITTER_QUERIES: &[&str] = &[
    "supply chain disruption",
    "semiconductor shortage",
    "defense contract awarded",
    "sanctions entity list",
    "factory expansion manufacturing",
    "export controls chip",
    "trade tariff",
    "military procurement",
    "cybersecurity breach",
    "acquisition merger electronics",
];

// ─────────────────────────────────────────────────────────────────────────────
// API response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct TweetSearchResponse {
    data: Option<Vec<TweetData>>,
    meta: Option<SearchMeta>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct TweetData {
    id: String,
    text: String,
    created_at: Option<String>,
    public_metrics: Option<PublicMetrics>,
    author_id: Option<String>,
    lang: Option<String>,
    entities: Option<TweetEntities>,
}

/// Twitter v2 `entities` object: hashtags, mentions, and expanded URLs.
#[allow(dead_code)]
#[derive(Deserialize, Debug, Default)]
struct TweetEntities {
    #[serde(default)]
    hashtags: Vec<TweetHashtag>,
    #[serde(default)]
    mentions: Vec<TweetMention>,
    #[serde(default)]
    urls: Vec<TweetUrl>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct TweetHashtag {
    tag: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct TweetMention {
    username: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct TweetUrl {
    expanded_url: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct PublicMetrics {
    retweet_count: Option<u64>,
    reply_count: Option<u64>,
    like_count: Option<u64>,
    quote_count: Option<u64>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct SearchMeta {
    next_token: Option<String>,
    result_count: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// Twitter intelligence scraper.
pub struct TwitterScraper {
    client: Client,
    bearer_token: Option<String>,
    #[allow(dead_code)]
    proxy_url: Option<String>,
}

impl TwitterScraper {
    /// Create a new scraper.
    ///
    /// * `bearer_token` — Twitter API v2 bearer token; `None` enables nitter
    ///   fallback.
    /// * `proxy_url` — optional HTTP proxy in `http://user:pass@host:port`
    ///   format.
    pub fn new(bearer_token: Option<String>, proxy_url: Option<String>) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(30),
            user_agent: Some("ApexIntel/1.0 (+https://apexintel.io) OSINT Collector".to_string()),
            proxy: proxy_url
                .as_deref()
                .map(reqwest::Proxy::all)
                .transpose()
                .context("Invalid proxy URL")?,
            ..crate::http::ExternalClientOptions::default()
        })?;

        Ok(Self {
            client,
            bearer_token,
            proxy_url,
        })
    }

    /// Create from environment variable `TWITTER_BEARER_TOKEN`.
    pub fn from_env(proxy_url: Option<String>) -> Result<Self> {
        let token = std::env::var("TWITTER_BEARER_TOKEN").ok();
        Self::new(token, proxy_url)
    }

    // ── Public API v2 ─────────────────────────────────────────────

    /// Search recent tweets (last 7 days, max 100 per call).
    ///
    /// `query` supports Twitter search operators, e.g.:
    /// `"defense procurement Israel" lang:en -is:retweet`
    pub async fn search_recent(&self, query: &str, max_results: u32) -> Result<Vec<SocialPost>> {
        match &self.bearer_token {
            Some(token) => self.api_search(query, max_results, token).await,
            None => {
                warn!("No Twitter bearer token; falling back to nitter search");
                self.nitter_search(query, max_results).await
            }
        }
    }

    /// Fetch recent tweets from a public user timeline.
    pub async fn user_timeline(&self, user_id: &str, max_results: u32) -> Result<Vec<SocialPost>> {
        let token = match &self.bearer_token {
            Some(t) => t,
            None => {
                warn!("No bearer token for user_timeline; returning empty");
                return Ok(vec![]);
            }
        };

        let url = format!(
            "{}/users/{}/tweets?max_results={}&tweet.fields=created_at,public_metrics,lang",
            TWITTER_API_BASE,
            user_id,
            max_results.min(100)
        );

        let resp: TweetSearchResponse = crate::http::read_capped_json(
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
                .context("Twitter user timeline request failed")?,
            crate::http::MAX_EXTERNAL_BODY_BYTES,
        )
        .await
        .context("Failed to parse Twitter user timeline response")?;

        Ok(self.map_tweets(resp.data.unwrap_or_default()))
    }

    // ── Private helpers ──────────────────────────────────────────

    async fn api_search(
        &self,
        query: &str,
        max_results: u32,
        token: &str,
    ) -> Result<Vec<SocialPost>> {
        let capped = max_results.min(100);
        let encoded = urlencoding::encode(query);
        let url = format!(
            "{}/tweets/search/recent?query={}&max_results={}&tweet.fields=created_at,public_metrics,author_id,lang",
            TWITTER_API_BASE, encoded, capped
        );

        debug!(endpoint=%url, "Twitter API search");

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("Twitter API request failed")?;

        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            warn!("Twitter rate limit hit; returning empty");
            return Ok(vec![]);
        }

        let data: TweetSearchResponse =
            crate::http::read_capped_json(resp, crate::http::MAX_EXTERNAL_BODY_BYTES)
                .await
                .context("Twitter API parse error")?;
        Ok(self.map_tweets(data.data.unwrap_or_default()))
    }

    async fn nitter_search(&self, query: &str, max_results: u32) -> Result<Vec<SocialPost>> {
        // Try each nitter instance in order
        for instance in NITTER_INSTANCES {
            let url = format!(
                "{}/search?q={}&f=tweets",
                instance,
                urlencoding::encode(query)
            );
            if let Ok(posts) = self.scrape_nitter_page(&url, max_results).await {
                return Ok(posts);
            }
        }
        warn!("All nitter instances failed for query: {}", query);
        Ok(vec![])
    }

    async fn scrape_nitter_page(&self, url: &str, max_results: u32) -> Result<Vec<SocialPost>> {
        let html = crate::http::read_capped(
            self.client.get(url).send().await?,
            crate::http::MAX_EXTERNAL_BODY_BYTES,
        )
        .await?;

        // Parse Nitter HTML: look for tweet cards
        let mut posts = Vec::new();
        for (idx, block) in html.split("timeline-item").enumerate() {
            if idx == 0 {
                continue;
            } // Skip header
            if posts.len() >= max_results as usize {
                break;
            }

            // Extract tweet-content text block
            if let Some(content_start) = block.find("tweet-content") {
                let inner = &block[content_start..];
                if let (Some(start), Some(end)) = (inner.find('>'), inner.find("</div>")) {
                    let text = strip_html_tags(&inner[start + 1..end]);
                    if !text.trim().is_empty() {
                        // Extract tweet link
                        let post_url = extract_attr(block, "href")
                            .map(|h| format!("https://twitter.com{}", h))
                            .unwrap_or_default();

                        let post_id = post_url
                            .split('/')
                            .next_back()
                            .unwrap_or("unknown")
                            .to_string();

                        // Extract author handle
                        let author = extract_attr(block, "class=\"username\"")
                            .unwrap_or_else(|| "unknown".to_string());

                        let post = SocialPost::minimal("twitter", &post_id, &author, &text, None);
                        posts.push(post);
                    }
                }
            }
        }

        Ok(posts)
    }

    fn map_tweets(&self, tweets: Vec<TweetData>) -> Vec<SocialPost> {
        tweets
            .into_iter()
            .map(|t| {
                let published_at = t
                    .created_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                let mut post = SocialPost::minimal(
                    "twitter",
                    &t.id,
                    t.author_id.as_deref().unwrap_or("unknown"),
                    &t.text,
                    published_at,
                );

                if let Some(metrics) = t.public_metrics {
                    post.like_count = metrics.like_count.unwrap_or(0);
                    post.share_count =
                        metrics.retweet_count.unwrap_or(0) + metrics.quote_count.unwrap_or(0);
                    post.reply_count = metrics.reply_count.unwrap_or(0);
                }

                if let Some(entities) = t.entities {
                    post.hashtags = entities
                        .hashtags
                        .into_iter()
                        .filter_map(|hashtag| hashtag.tag)
                        .map(|tag| format!("#{tag}"))
                        .collect();
                    post.mentions = entities
                        .mentions
                        .into_iter()
                        .filter_map(|mention| mention.username)
                        .collect();
                    post.urls = entities
                        .urls
                        .into_iter()
                        .filter_map(|url| url.expanded_url)
                        .collect();
                }

                post.language = t.lang;
                post.post_url = format!("https://twitter.com/i/web/status/{}", t.id);

                post
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Twitter/X API monitor (credentialed adapter)
// ─────────────────────────────────────────────────────────────────────────────

/// A Twitter/X tweet from the credentialed API monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    pub tweet_id: String,
    pub author_username: String,
    pub author_id: String,
    pub text: String,
    /// Publication time as stated by the API; `None` when absent/unparseable.
    pub created_at: Option<DateTime<Utc>>,
    pub like_count: Option<i64>,
    pub retweet_count: Option<i64>,
    pub reply_count: Option<i64>,
    pub quote_count: Option<i64>,
    pub language: Option<String>,
    pub is_reply: bool,
    pub is_retweet: bool,
    pub is_quote: bool,
    pub hashtags: Vec<String>,
    pub mentions: Vec<String>,
    pub urls: Vec<String>,
    pub matched_keywords: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

impl Tweet {
    /// Total engagement score.
    pub fn engagement_score(&self) -> i64 {
        let likes = self.like_count.unwrap_or(0);
        let rts = self.retweet_count.unwrap_or(0);
        let replies = self.reply_count.unwrap_or(0);
        let quotes = self.quote_count.unwrap_or(0);
        likes + (rts * 2) + (replies * 3) + (quotes * 2)
    }

    /// Whether this tweet exhibits high-influence engagement patterns.
    ///
    /// Since the Twitter API v2 tweet payload does not include the author's
    /// follower count (that requires a separate user lookup), we use the
    /// observed engagement metrics as a reliable proxy. A tweet whose
    /// weighted engagement score exceeds [`HIGH_INFLUENCE_THRESHOLD`] indicates
    /// the author account has substantial reach and amplification power.
    ///
    /// The threshold is calibrated against the weighted engagement formula:
    /// e.g. 200 likes + 100 retweets (×2) + 50 replies (×3) + 25 quotes (×2)
    /// = 200 + 200 + 150 + 50 = 600.
    pub fn is_high_influence(&self) -> bool {
        self.engagement_score() >= HIGH_INFLUENCE_THRESHOLD
    }
}

/// Minimum weighted engagement score for a tweet to be classified as
/// high-influence.  See [`Tweet::is_high_influence`].
pub const HIGH_INFLUENCE_THRESHOLD: i64 = 500;

/// Twitter account metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitterAccount {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub bio: Option<String>,
    pub follower_count: Option<i64>,
    pub following_count: Option<i64>,
    pub tweet_count: Option<i64>,
    pub location: Option<String>,
    pub website: Option<String>,
    pub verified: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

impl TwitterAccount {
    /// Whether this is a large account.
    pub fn is_large(&self) -> bool {
        self.follower_count.unwrap_or(0) > 100_000
    }
}

/// Twitter credential/API monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitterMonitorConfig {
    /// Accounts to monitor.
    pub tracked_accounts: Vec<String>,
    /// Keywords to track.
    pub tracked_keywords: Vec<String>,
    /// Hashtags to track.
    pub tracked_hashtags: Vec<String>,
    /// API bearer token (Twitter API v2).
    pub bearer_token: Option<String>,
    /// Maximum tweets per fetch.
    pub max_tweets: u32,
    /// Timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for TwitterMonitorConfig {
    fn default() -> Self {
        Self {
            tracked_accounts: Vec::new(),
            tracked_keywords: vec![
                "defense".to_string(),
                "cybersecurity".to_string(),
                "intelligence".to_string(),
            ],
            tracked_hashtags: Vec::new(),
            bearer_token: None,
            max_tweets: 100,
            timeout_secs: 30,
        }
    }
}

impl TwitterMonitorConfig {
    pub fn add_account(mut self, username: impl Into<String>) -> Self {
        self.tracked_accounts.push(username.into());
        self
    }

    pub fn add_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.tracked_keywords.push(keyword.into());
        self
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }
}

/// Twitter/X monitor using the credentialed v2 API adapter.
#[derive(Debug, Clone)]
pub struct TwitterMonitor {
    client: Client,
    config: TwitterMonitorConfig,
}

impl TwitterMonitor {
    pub fn new(config: TwitterMonitorConfig) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(config.timeout_secs),
            user_agent: Some("ApexIntel/1.0 (+https://apexintel.io) Twitter Monitor".to_string()),
            ..crate::http::ExternalClientOptions::default()
        })
        .context("building Twitter HTTP client")?;
        Ok(Self { client, config })
    }

    /// Fetch recent tweets from an account.
    ///
    /// Without a bearer token the adapter is explicitly
    /// [`AcquisitionOutcome::AuthenticationRequired`], never an empty success.
    pub async fn fetch_user_tweets(&self, username: &str) -> AcquisitionOutcome<Tweet> {
        let Some(bearer) = self.config.bearer_token.as_deref() else {
            warn!(username = %username, "Twitter API requires a bearer token");
            return AcquisitionOutcome::AuthenticationRequired;
        };

        let url = format!(
            "https://api.twitter.com/2/users/by/username/{}/tweets",
            urlencoding::encode(username)
        );
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("Twitter user tweets request failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), username = %username, "Twitter returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "Twitter user tweets",
            );
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct TwitterApiResponse {
            data: Option<Vec<serde_json::Value>>,
        }

        let twitter_resp: TwitterApiResponse =
            match crate::http::read_capped_json(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await {
                Ok(twitter_resp) => twitter_resp,
                Err(error) => {
                    return AcquisitionOutcome::parse_failed(
                        format!("parse Twitter user tweets response failed: {error}"),
                        "",
                    );
                }
            };
        let tweets: Vec<Tweet> = twitter_resp
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                let tweet_id = t["id"].as_str()?.to_string();
                let text = t["text"].as_str()?.to_string();
                let created_at = t["created_at"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                let hashtags: Vec<String> = t["entities"]["hashtags"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|h| h["tag"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let matched: Vec<String> = self
                    .config
                    .tracked_keywords
                    .iter()
                    .filter(|kw| text.to_lowercase().contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                Some(Tweet {
                    tweet_id,
                    author_username: username.to_string(),
                    author_id: t["author_id"].as_str().unwrap_or("").to_string(),
                    text,
                    created_at,
                    like_count: t["public_metrics"]["like_count"].as_i64(),
                    retweet_count: t["public_metrics"]["retweet_count"].as_i64(),
                    reply_count: t["public_metrics"]["reply_count"].as_i64(),
                    quote_count: t["public_metrics"]["quote_count"].as_i64(),
                    language: t["lang"].as_str().map(String::from),
                    is_reply: t["reply_to"].is_array(),
                    is_retweet: t["referenced_tweets"]
                        .as_array()
                        .map(|arr| arr.iter().any(|r| r["type"].as_str() == Some("retweeted")))
                        .unwrap_or(false),
                    is_quote: t["referenced_tweets"]
                        .as_array()
                        .map(|arr| arr.iter().any(|r| r["type"].as_str() == Some("quoted")))
                        .unwrap_or(false),
                    hashtags,
                    mentions: t["entities"]["mentions"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["username"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    urls: t["entities"]["urls"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|u| u["expanded_url"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    matched_keywords: matched,
                    fetched_at: Utc::now(),
                })
            })
            .collect();

        debug!(username = %username, count = tweets.len(), "Twitter tweets fetched");
        AcquisitionOutcome::success_now(tweets)
    }

    /// Search tweets by keyword.
    ///
    /// Without a bearer token the adapter is explicitly
    /// [`AcquisitionOutcome::AuthenticationRequired`], never an empty success.
    pub async fn search_tweets(&self, query: &str) -> AcquisitionOutcome<Tweet> {
        let Some(bearer) = self.config.bearer_token.as_deref() else {
            warn!(query = %query, "Twitter API requires a bearer token");
            return AcquisitionOutcome::AuthenticationRequired;
        };

        let url = format!(
            "https://api.twitter.com/2/tweets/search/recent?query={}&max_results={}",
            urlencoding::encode(query),
            self.config.max_tweets
        );
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("Twitter search request failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "Twitter search",
            );
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct TwitterSearchResponse {
            data: Option<Vec<serde_json::Value>>,
        }

        let search_resp: TwitterSearchResponse =
            match crate::http::read_capped_json(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await {
                Ok(search_resp) => search_resp,
                Err(error) => {
                    return AcquisitionOutcome::parse_failed(
                        format!("parse Twitter search response failed: {error}"),
                        "",
                    );
                }
            };
        let tweets: Vec<Tweet> = search_resp
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|t| Tweet {
                tweet_id: t["id"].as_str().unwrap_or("").to_string(),
                author_username: t["author_id"].as_str().unwrap_or("").to_string(),
                author_id: t["author_id"].as_str().unwrap_or("").to_string(),
                text: t["text"].as_str().unwrap_or("").to_string(),
                created_at: t["created_at"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                like_count: t["public_metrics"]["like_count"].as_i64(),
                retweet_count: t["public_metrics"]["retweet_count"].as_i64(),
                reply_count: t["public_metrics"]["reply_count"].as_i64(),
                quote_count: t["public_metrics"]["quote_count"].as_i64(),
                language: t["lang"].as_str().map(String::from),
                is_reply: false,
                is_retweet: false,
                is_quote: false,
                hashtags: Vec::new(),
                mentions: Vec::new(),
                urls: Vec::new(),
                matched_keywords: vec![query.to_string()],
                fetched_at: Utc::now(),
            })
            .collect();

        AcquisitionOutcome::success_now(tweets)
    }

    /// Monitor all tracked accounts.
    pub async fn full_scan(&self) -> Vec<Tweet> {
        let mut all_tweets = Vec::new();
        for username in &self.config.tracked_accounts {
            match self.fetch_user_tweets(username).await {
                AcquisitionOutcome::Success { items, .. } => all_tweets.extend(items),
                other => warn!(
                    username = %username,
                    outcome = other.as_label(),
                    "Twitter account scan did not succeed"
                ),
            }
        }
        // Deduplicate by tweet_id, newest first — the same tweet can appear
        // under multiple tracked accounts/queries.
        all_tweets.sort_by_key(|tweet| std::cmp::Reverse(tweet.created_at));
        all_tweets.dedup_by(|a, b| a.tweet_id == b.tweet_id);
        info!(total = all_tweets.len(), "Twitter full scan complete");
        all_tweets
    }

    /// Get high-engagement tweets.
    pub fn top_engagement(tweets: &[Tweet]) -> Vec<Tweet> {
        let mut sorted = tweets.to_vec();
        sorted.sort_by_key(|t| std::cmp::Reverse(t.engagement_score()));
        sorted.into_iter().take(10).collect::<Vec<Tweet>>()
    }
}

/// Request for one Twitter/X keyword search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitterKeywordRequest {
    pub query: String,
}

#[async_trait]
impl SourceAdapter for TwitterMonitor {
    type Item = Tweet;
    type Request = TwitterKeywordRequest;

    fn adapter_id(&self) -> &'static str {
        "twitter"
    }

    fn prerequisite(&self) -> AdapterPrerequisite {
        AdapterPrerequisite::CREDENTIALS
    }

    fn credentials_configured(&self) -> bool {
        self.config.bearer_token.is_some()
    }

    async fn acquire(&self, request: TwitterKeywordRequest) -> AcquisitionOutcome<Tweet> {
        self.search_tweets(&request.query).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTML utilities
// ─────────────────────────────────────────────────────────────────────────────

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn extract_attr(html: &str, attr: &str) -> Option<String> {
    let start_idx = html.find(attr)?;
    let after = &html[start_idx + attr.len()..];
    let val_start = after.find('"')? + 1;
    let val_end = after[val_start..].find('"')?;
    Some(after[val_start..val_start + val_end].to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_tags_works() {
        let html = "<b>Hello</b> <i>world</i> &amp; friends";
        let result = strip_html_tags(html);
        assert_eq!(result, "Hello world & friends");
    }

    #[test]
    fn map_tweets_handles_empty() {
        let scraper = TwitterScraper::new(None, None)
            .unwrap_or_else(|error| panic!("twitter scraper should build: {error}"));
        let posts = scraper.map_tweets(vec![]);
        assert!(posts.is_empty());
    }

    #[test]
    fn scraper_builds_without_token() {
        let s = TwitterScraper::new(None, None);
        assert!(s.is_ok());
    }

    #[test]
    fn map_tweets_extracts_entities_and_metrics() {
        let scraper = TwitterScraper::new(None, None)
            .unwrap_or_else(|error| panic!("twitter scraper should build: {error}"));
        let tweets = vec![TweetData {
            id: "123".to_string(),
            text: "Sanctions update #trade @analyst https://t.co/x".to_string(),
            created_at: Some("2024-01-15T10:00:00Z".to_string()),
            public_metrics: Some(PublicMetrics {
                retweet_count: Some(5),
                reply_count: Some(2),
                like_count: Some(10),
                quote_count: Some(1),
            }),
            author_id: Some("42".to_string()),
            lang: Some("en".to_string()),
            entities: Some(TweetEntities {
                hashtags: vec![TweetHashtag {
                    tag: Some("trade".to_string()),
                }],
                mentions: vec![TweetMention {
                    username: Some("analyst".to_string()),
                }],
                urls: vec![TweetUrl {
                    expanded_url: Some("https://example.com/sanctions".to_string()),
                }],
            }),
        }];
        let posts = scraper.map_tweets(tweets);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].hashtags, vec!["#trade".to_string()]);
        assert_eq!(posts[0].mentions, vec!["analyst".to_string()]);
        assert_eq!(
            posts[0].urls,
            vec!["https://example.com/sanctions".to_string()]
        );
        assert_eq!(posts[0].like_count, 10);
        assert_eq!(posts[0].share_count, 6);
        assert_eq!(posts[0].reply_count, 2);
    }

    #[tokio::test]
    async fn twitter_without_token_is_authentication_required_not_empty_success() {
        let monitor = TwitterMonitor::new(Default::default()).expect("Twitter monitor");
        assert!(!monitor.credentials_configured());

        let account = monitor.fetch_user_tweets("acme").await;
        assert_eq!(
            account.disposition(),
            crate::acquisition::AcquisitionDisposition::AuthenticationBlocked
        );
        assert!(!account.is_success());

        let search = monitor.search_tweets("acme").await;
        assert_eq!(
            search.disposition(),
            crate::acquisition::AcquisitionDisposition::AuthenticationBlocked
        );
        assert!(!search.is_success());
        assert!(search.records_failure());
    }

    #[test]
    fn twitter_adapter_declares_its_prerequisite() {
        let monitor = TwitterMonitor::new(Default::default()).expect("Twitter monitor");
        assert_eq!(monitor.adapter_id(), "twitter");
        assert!(monitor.prerequisite().requires_credentials);
        assert!(crate::acquisition::adapter_descriptor(monitor.adapter_id()).is_some());
    }

    #[test]
    fn tweet_engagement_score() {
        let tweet = Tweet {
            tweet_id: "123".to_string(),
            author_username: "test".to_string(),
            author_id: "456".to_string(),
            text: "Test tweet".to_string(),
            created_at: Some(Utc::now()),
            like_count: Some(100),
            retweet_count: Some(50),
            reply_count: Some(10),
            quote_count: Some(5),
            language: Some("en".to_string()),
            is_reply: false,
            is_retweet: false,
            is_quote: false,
            hashtags: vec![],
            mentions: vec![],
            urls: vec![],
            matched_keywords: vec![],
            fetched_at: Utc::now(),
        };
        // 100 + (50*2) + (10*3) + (5*2) = 100 + 100 + 30 + 10 = 240
        assert_eq!(tweet.engagement_score(), 240);
    }

    #[test]
    fn twitter_monitor_constructs_and_chains() {
        let result = TwitterMonitor::new(Default::default());
        assert!(result.is_ok());

        let cfg = TwitterMonitorConfig::default()
            .add_account("elonmusk")
            .add_keyword("defense")
            .with_bearer_token("test-token");
        assert_eq!(cfg.tracked_accounts.len(), 1);
        assert!(cfg.bearer_token.is_some());
    }

    #[test]
    fn top_engagement_orders_by_weighted_score() {
        let low = Tweet {
            tweet_id: "1".to_string(),
            author_username: "a".to_string(),
            author_id: "1".to_string(),
            text: "Low engagement".to_string(),
            created_at: Some(Utc::now()),
            like_count: Some(10),
            retweet_count: Some(5),
            reply_count: Some(1),
            quote_count: Some(0),
            language: None,
            is_reply: false,
            is_retweet: false,
            is_quote: false,
            hashtags: vec![],
            mentions: vec![],
            urls: vec![],
            matched_keywords: vec![],
            fetched_at: Utc::now(),
        };
        let high = Tweet {
            tweet_id: "2".to_string(),
            like_count: Some(1000),
            retweet_count: Some(500),
            reply_count: Some(100),
            quote_count: Some(50),
            text: "High engagement".to_string(),
            ..low.clone()
        };
        let top = TwitterMonitor::top_engagement(&[low, high]);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].tweet_id, "2"); // Highest engagement first
    }

    #[test]
    fn is_high_influence_uses_engagement_proxy() {
        let low = Tweet {
            tweet_id: "1".to_string(),
            author_username: "a".to_string(),
            author_id: "1".to_string(),
            text: "Low engagement".to_string(),
            created_at: Some(Utc::now()),
            like_count: Some(10),
            retweet_count: Some(5),
            reply_count: Some(1),
            quote_count: Some(0),
            language: None,
            is_reply: false,
            is_retweet: false,
            is_quote: false,
            hashtags: vec![],
            mentions: vec![],
            urls: vec![],
            matched_keywords: vec![],
            fetched_at: Utc::now(),
        };
        // 10 + (5*2) + (1*3) + (0*2) = 23
        assert!(!low.is_high_influence());

        let high = Tweet {
            like_count: Some(200),
            retweet_count: Some(100),
            reply_count: Some(50),
            quote_count: Some(25),
            ..low
        };
        // 200 + (100*2) + (50*3) + (25*2) = 600 >= 500
        assert!(high.is_high_influence());
    }
}
