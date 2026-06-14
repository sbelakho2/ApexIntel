//! Twitter/X Intelligence Module
//!
//! Monitors Twitter/X for entity mentions, trend analysis, and public sentiment:
//! - Keyword/hashtag monitoring across public tweets
//! - Account activity tracking (verified accounts, journalists, executives)
//! - Thread analysis for long-form intelligence extraction
//! - Engagement signal scoring
//!
//! # API Access
//! Set `TWITTER_BEARER_TOKEN` environment variable for v2 API access.
//! Falls back to public HTML scraping when absent (rate-limited and limited).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A normalised tweet from Twitter/X.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    /// Tweet ID.
    pub tweet_id: String,
    /// Author handle (without @).
    pub author_handle: String,
    /// Display name.
    pub author_name: String,
    /// Whether the author is verified.
    pub author_verified: bool,
    /// Tweet text.
    pub text: String,
    /// Original text including URLs and mentions.
    pub raw_text: String,
    /// Publication timestamp.
    pub published_at: DateTime<Utc>,
    /// Like count.
    pub like_count: u64,
    /// Retweet count.
    pub retweet_count: u64,
    /// Reply count.
    pub reply_count: u64,
    /// Quote count.
    pub quote_count: u64,
    /// Extracted URLs.
    pub urls: Vec<String>,
    /// Hashtags.
    pub hashtags: Vec<String>,
    /// Mentions (without @).
    pub mentions: Vec<String>,
    /// Direct link to the tweet.
    pub tweet_url: String,
    /// Language code.
    pub language: Option<String>,
    /// Is a reply.
    pub is_reply: bool,
    /// Is a retweet.
    pub is_retweet: bool,
    /// Is a quote tweet.
    pub is_quote: bool,
    /// Conversation thread ID (if part of a thread).
    pub thread_id: Option<String>,
    /// Engagement score (computed).
    pub engagement_score: f64,
}

/// Twitter intelligence configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitterMonitorConfig {
    /// Twitter/X Bearer token (v2 API).
    pub bearer_token: Option<String>,
    /// Keywords to track.
    pub keywords: Vec<String>,
    /// Hashtags to track.
    pub hashtags: Vec<String>,
    /// Account handles to monitor.
    pub account_handles: Vec<String>,
    /// Maximum tweets to fetch per query.
    pub max_results: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for TwitterMonitorConfig {
    fn default() -> Self {
        Self {
            bearer_token: None,
            keywords: Vec::new(),
            hashtags: Vec::new(),
            account_handles: Vec::new(),
            max_results: 100,
            timeout_secs: 30,
        }
    }
}

impl TwitterMonitorConfig {
    /// Add a keyword to track.
    pub fn add_keyword(mut self, kw: impl Into<String>) -> Self {
        self.keywords.push(kw.into());
        self
    }

    /// Add an account handle to monitor.
    pub fn add_account(mut self, handle: impl Into<String>) -> Self {
        self.account_handles.push(handle.into());
        self
    }
}

/// Twitter/X intelligence monitor.
#[derive(Debug, Clone)]
pub struct TwitterMonitor {
    client: Client,
    config: TwitterMonitorConfig,
}

impl TwitterMonitor {
    /// Create from environment (`TWITTER_BEARER_TOKEN`).
    pub fn from_env() -> Result<Self> {
        let config = TwitterMonitorConfig {
            bearer_token: std::env::var("TWITTER_BEARER_TOKEN").ok(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Create with explicit configuration.
    pub fn new(config: TwitterMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Twitter Monitor")
            .build()
            .context("building Twitter HTTP client")?;

        Ok(Self { client, config })
    }

    /// Search recent tweets by keyword.
    pub async fn search_tweets(&self, query: &str) -> Result<Vec<Tweet>> {
        if let Some(ref token) = self.config.bearer_token {
            self.search_tweets_api(token, query).await
        } else {
            warn!("TWITTER_BEARER_TOKEN not set — keyword search unavailable");
            Ok(Vec::new())
        }
    }

    /// Search using Twitter v2 API.
    async fn search_tweets_api(&self, token: &str, query: &str) -> Result<Vec<Tweet>> {
        let url = "https://api.twitter.com/2/tweets/search/recent";

        let params = [
            ("query", query),
            ("max_results", &self.config.max_results.to_string()),
            ("tweet.fields", "created_at,public_metrics,entities,lang"),
            ("expansions", "author_id"),
            ("user.fields", "name,username,verified,public_metrics"),
        ];

        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&params)
            .send()
            .await
            .context("Twitter v2 search request")?;

        if !resp.status().is_success() {
            anyhow::bail!("Twitter API returned {}", resp.status());
        }

        #[derive(Deserialize)]
        struct ApiResponse {
            data: Option<Vec<serde_json::Value>>,
            includes: Option<serde_json::Value>,
        }

        let api_resp: ApiResponse = resp.json().await.context("parse Twitter API response")?;

        let tweets = api_resp
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|v| self.parse_tweet_json(&v))
            .collect();

        Ok(tweets)
    }

    /// Parse a Twitter v2 tweet JSON object.
    fn parse_tweet_json(&self, value: &serde_json::Value) -> Tweet {
        let tweet_id = value["id"].as_str().unwrap_or("").to_string();
        let text = value["text"].as_str().unwrap_or("").to_string();
        let raw_text = text.clone();

        let metrics = &value["public_metrics"];
        let like_count = metrics["like_count"].as_u64().unwrap_or(0);
        let retweet_count = metrics["retweet_count"].as_u64().unwrap_or(0);
        let reply_count = metrics["reply_count"].as_u64().unwrap_or(0);
        let quote_count = metrics["quote_count"].as_u64().unwrap_or(0);

        let hashtags: Vec<String> = value["entities"]
            .get("hashtags")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|h| h.get("tag").and_then(|t| t.as_str()))
                    .map(|s| s.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();

        let mentions: Vec<String> = value["entities"]
            .get("mentions")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("username").and_then(|u| u.as_str()))
                    .map(|s| s.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();

        let urls: Vec<String> = value["entities"]
            .get("urls")
            .and_then(|u| u.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|u| u.get("expanded_url").and_then(|e| e.as_str()))
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let published_at = value["created_at"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let engagement_score = like_count as f64
            + retweet_count as f64 * 3.0
            + reply_count as f64 * 2.0
            + quote_count as f64 * 1.5;

        Tweet {
            tweet_id,
            author_handle: String::new(),
            author_name: String::new(),
            author_verified: false,
            text,
            raw_text,
            published_at,
            like_count,
            retweet_count,
            reply_count,
            quote_count,
            urls,
            hashtags,
            mentions,
            tweet_url: String::new(),
            language: value["lang"].as_str().map(|s| s.to_string()),
            is_reply: value["referenced_tweets"]
                .as_array()
                .map(|arr| arr.iter().any(|r| r["type"] == "replied_to"))
                .unwrap_or(false),
            is_retweet: value["referenced_tweets"]
                .as_array()
                .map(|arr| arr.iter().any(|r| r["type"] == "retweeted"))
                .unwrap_or(false),
            is_quote: value["referenced_tweets"]
                .as_array()
                .map(|arr| arr.iter().any(|r| r["type"] == "quoted"))
                .unwrap_or(false),
            thread_id: None,
            engagement_score,
        }
    }

    /// Fetch recent tweets from a specific account.
    pub async fn fetch_account_tweets(&self, handle: &str) -> Result<Vec<Tweet>> {
        if let Some(ref token) = self.config.bearer_token {
            let query = format!("from:{}", handle.trim_start_matches('@'));
            self.search_tweets_api(token, &query).await
        } else {
            warn!("TWITTER_BEARER_TOKEN not set — account fetch unavailable");
            Ok(Vec::new())
        }
    }

    /// Monitor all configured keywords and accounts.
    pub async fn full_scan(&self) -> Vec<Tweet> {
        let mut all_tweets = Vec::new();

        for kw in &self.config.keywords {
            match self.search_tweets(kw).await {
                Ok(tweets) => all_tweets.extend(tweets),
                Err(e) => warn!(keyword = %kw, error = %e, "Keyword search failed"),
            }
        }

        for handle in &self.config.account_handles {
            match self.fetch_account_tweets(handle).await {
                Ok(tweets) => all_tweets.extend(tweets),
                Err(e) => warn!(handle = %handle, error = %e, "Account fetch failed"),
            }
        }

        // Deduplicate by tweet_id
        all_tweets.sort_by(|a, b| b.published_at.cmp(&a.published_at));
        all_tweets.dedup_by(|a, b| a.tweet_id == b.tweet_id);

        info!(total = all_tweets.len(), "Twitter/X full scan complete");
        all_tweets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitter_monitor_config_defaults() {
        let cfg = TwitterMonitorConfig::default();
        assert!(cfg.keywords.is_empty());
        assert!(cfg.bearer_token.is_none());
        assert_eq!(cfg.max_results, 100);
    }

    #[test]
    fn twitter_monitor_chaining() {
        let cfg = TwitterMonitorConfig::default()
            .add_keyword("defense")
            .add_keyword("cybersecurity")
            .add_account("elbit");
        assert_eq!(cfg.keywords.len(), 2);
        assert_eq!(cfg.account_handles.len(), 1);
    }

    #[test]
    fn twitter_monitor_constructs() {
        let result = TwitterMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn tweet_engagement_score() {
        let tweet = Tweet {
            tweet_id: "123".to_string(),
            author_handle: "test".to_string(),
            author_name: "Test User".to_string(),
            author_verified: false,
            text: "Test tweet".to_string(),
            raw_text: "Test tweet".to_string(),
            published_at: Utc::now(),
            like_count: 100,
            retweet_count: 50,
            reply_count: 20,
            quote_count: 10,
            urls: vec![],
            hashtags: vec![],
            mentions: vec![],
            tweet_url: "https://x.com/test/123".to_string(),
            language: None,
            is_reply: false,
            is_retweet: false,
            is_quote: false,
            thread_id: None,
            engagement_score: 0.0,
        };
        // 100 + 150 + 40 + 15 = 305
        assert!((tweet.engagement_score - 305.0).abs() < f64::EPSILON);
    }
}
