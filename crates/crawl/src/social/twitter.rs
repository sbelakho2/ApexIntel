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
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, warn};

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
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) OSINT Collector");

        if let Some(ref proxy) = proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("Invalid proxy URL")?);
        }

        Ok(Self {
            client: builder.build()?,
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

        let resp: TweetSearchResponse = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("Twitter user timeline request failed")?
            .json()
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

        let data: TweetSearchResponse = resp.json().await.context("Twitter API parse error")?;
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
        let html = self.client.get(url).send().await?.text().await?;

        // Parse Nitter HTML: look for tweet cards
        let mut posts = Vec::new();
        let now = Utc::now();

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

                        let post = SocialPost::minimal("twitter", &post_id, &author, &text, now);
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
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

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

                post.language = t.lang;
                post.post_url = format!("https://twitter.com/i/web/status/{}", &t.id);

                post
            })
            .collect()
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
}
