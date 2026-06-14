//! Twitter/X Intelligence Module
//!
//! Monitors Twitter/X for entity mentions and sentiment:
//! - Account activity monitoring
//! - Hashtag tracking
//! - Mention analysis
//! - Tweet collection and classification
//! - Sentiment scoring (basic)
//!
//! Uses Twitter API v2 where available.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A Twitter/X tweet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    pub tweet_id: String,
    pub author_username: String,
    pub author_id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
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

/// Twitter monitoring configuration.
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
        self.tracked_accounts.push(username.into()); self
    }

    pub fn add_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.tracked_keywords.push(keyword.into()); self
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into()); self
    }
}

/// Twitter/X monitor.
#[derive(Debug, Clone)]
pub struct TwitterMonitor {
    client: Client,
    config: TwitterMonitorConfig,
}

impl TwitterMonitor {
    pub fn new(config: TwitterMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Twitter Monitor")
            .build()
            .context("building Twitter HTTP client")?;
        Ok(Self { client, config })
    }

    /// Fetch recent tweets from an account.
    pub async fn fetch_user_tweets(&self, username: &str) -> Result<Vec<Tweet>> {
        let bearer = self.config.bearer_token.as_ref()
            .context("Twitter API requires a bearer token")?;

        let url = format!(
            "https://api.twitter.com/2/users/by/username/{}/tweets",
            urlencoding::encode(username)
        );
        let resp = self.client.get(&url)
            .header("Authorization", format!("Bearer {}", bearer))
            .send().await
            .context("Twitter user tweets request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), username = %username, "Twitter returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct TwitterApiResponse { data: Option<Vec<serde_json::Value>> }

        let twitter_resp: TwitterApiResponse = resp.json().await.unwrap_or(TwitterApiResponse { data: None });
        let tweets: Vec<Tweet> = twitter_resp.data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                let tweet_id = t["id"].as_str()?.to_string();
                let text = t["text"].as_str()?.to_string();
                let created_at = t["created_at"].as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                let hashtags: Vec<String> = t["entities"]["hashtags"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|h| h["tag"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let matched: Vec<String> = self.config.tracked_keywords.iter()
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
                            arr.iter().filter_map(|m| m["username"].as_str().map(String::from)).collect()
                        })
                        .unwrap_or_default(),
                    urls: t["entities"]["urls"]
                        .as_array()
                        .map(|arr| {
                            arr.iter().filter_map(|u| u["expanded_url"].as_str().map(String::from)).collect()
                        })
                        .unwrap_or_default(),
                    matched_keywords: matched,
                    fetched_at: Utc::now(),
                })
            })
            .collect();

        debug!(username = %username, count = tweets.len(), "Twitter tweets fetched");
        Ok(tweets)
    }

    /// Search tweets by keyword.
    pub async fn search_tweets(&self, query: &str) -> Result<Vec<Tweet>> {
        let bearer = self.config.bearer_token.as_ref()
            .context("Twitter API requires a bearer token")?;

        let url = format!(
            "https://api.twitter.com/2/tweets/search/recent?query={}&max_results={}",
            urlencoding::encode(query),
            self.config.max_tweets
        );
        let resp = self.client.get(&url)
            .header("Authorization", format!("Bearer {}", bearer))
            .send().await
            .context("Twitter search request")?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct TwitterSearchResponse { data: Option<Vec<serde_json::Value>> }

        let search_resp: TwitterSearchResponse = resp.json().await.unwrap_or(TwitterSearchResponse { data: None });
        let tweets: Vec<Tweet> = search_resp.data
            .unwrap_or_default()
            .into_iter()
            .map(|t| Tweet {
                tweet_id: t["id"].as_str().unwrap_or("").to_string(),
                author_username: t["author_id"].as_str().unwrap_or("").to_string(),
                author_id: t["author_id"].as_str().unwrap_or("").to_string(),
                text: t["text"].as_str().unwrap_or("").to_string(),
                created_at: t["created_at"].as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now),
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

        Ok(tweets)
    }

    /// Monitor all tracked accounts.
    pub async fn full_scan(&self) -> Vec<Tweet> {
        let mut all_tweets = Vec::new();
        for username in &self.config.tracked_accounts {
            match self.fetch_user_tweets(username).await {
                Ok(tweets) => all_tweets.extend(tweets),
                Err(e) => warn!(username = %username, error = %e, "Twitter account scan failed"),
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tweet_engagement_score() {
        let tweet = Tweet {
            tweet_id: "123".to_string(),
            author_username: "test".to_string(),
            author_id: "456".to_string(),
            text: "Test tweet".to_string(),
            created_at: Utc::now(),
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
    fn twitter_monitor_constructs() {
        let result = TwitterMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn twitter_monitor_chaining() {
        let cfg = TwitterMonitorConfig::default()
            .add_account("elonmusk")
            .add_keyword("defense")
            .with_bearer_token("test-token");
        assert_eq!(cfg.tracked_accounts.len(), 1);
        assert!(cfg.bearer_token.is_some());
    }

    #[test]
    fn top_engagement() {
        use chrono::Utc;
        let tweets = vec![
            Tweet { tweet_id: "1".to_string(), author_username: "a".to_string(), author_id: "1".to_string(),
                text: "Low engagement".to_string(), created_at: Utc::now(),
                like_count: Some(10), retweet_count: Some(5), reply_count: Some(1), quote_count: Some(0),
                language: None, is_reply: false, is_retweet: false, is_quote: false,
                hashtags: vec![], mentions: vec![], urls: vec![], matched_keywords: vec![], fetched_at: Utc::now() },
            Tweet { tweet_id: "2".to_string(), author_username: "b".to_string(), author_id: "2".to_string(),
                text: "High engagement".to_string(), created_at: Utc::now(),
                like_count: Some(1000), retweet_count: Some(500), reply_count: Some(100), quote_count: Some(50),
                language: None, is_reply: false, is_retweet: false, is_quote: false,
                hashtags: vec![], mentions: vec![], urls: vec![], matched_keywords: vec![], fetched_at: Utc::now() },
        ];
        let top = TwitterMonitor::top_engagement(&tweets);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].tweet_id, "2"); // Highest engagement first
    }

    #[test]
    fn is_high_influence_uses_engagement_proxy() {
        // Low-engagement tweet should NOT be classified as high influence
        let low = Tweet {
            tweet_id: "1".to_string(), author_username: "a".to_string(), author_id: "1".to_string(),
            text: "Low engagement".to_string(), created_at: Utc::now(),
            like_count: Some(10), retweet_count: Some(5), reply_count: Some(1), quote_count: Some(0),
            language: None, is_reply: false, is_retweet: false, is_quote: false,
            hashtags: vec![], mentions: vec![], urls: vec![], matched_keywords: vec![], fetched_at: Utc::now(),
        };
        // 10 + (5*2) + (1*3) + (0*2) = 23
        assert!(!low.is_high_influence());

        // High-engagement tweet SHOULD be classified as high influence
        let high = Tweet {
            tweet_id: "2".to_string(), author_username: "b".to_string(), author_id: "2".to_string(),
            text: "High engagement".to_string(), created_at: Utc::now(),
            like_count: Some(200), retweet_count: Some(100), reply_count: Some(50), quote_count: Some(25),
            language: None, is_reply: false, is_retweet: false, is_quote: false,
            hashtags: vec![], mentions: vec![], urls: vec![], matched_keywords: vec![], fetched_at: Utc::now(),
        };
        // 200 + (100*2) + (50*3) + (25*2) = 200 + 200 + 150 + 50 = 600 >= 500
        assert!(high.is_high_influence());
    }
}
