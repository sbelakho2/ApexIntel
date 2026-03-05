//! Reddit intelligence scraper.
//!
//! Uses Reddit's unauthenticated JSON API (`*.reddit.com/.json`).  The API
//! returns up to 100 posts per page and supports `after` pagination tokens.
//!
//! # Use cases
//! - Monitor defence, geopolitics, and tech subreddits for emerging narratives
//! - Track entity mentions across communities
//! - Detect early signals on supply chain disruptions, conflict escalations, etc.

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::debug;

const REDDIT_BASE: &str = "https://www.reddit.com";

// ─────────────────────────────────────────────────────────────────────────────
// Target subreddits for OSINT
// ─────────────────────────────────────────────────────────────────────────────

/// Default OSINT-relevant subreddits to monitor.
pub const OSINT_SUBREDDITS: &[&str] = &[
    // Geopolitics & conflict
    "worldnews", "geopolitics", "CredibleDefense",
    "UkraineWarVideoReport", "europe", "MiddleEast",
    "IsraelPalestine", "china", "geopol",
    "IndoPacificRegion", "AfricaNews",
    // Defense & military
    "defense", "MilitaryProcurement", "drones",
    "ArmyTech", "WarCollege",
    // Security & cyber
    "netsec", "cybersecurity", "Intelligence", "osint",
    "SecurityAnalysis", "ReverseEngineering",
    // Supply chain & manufacturing
    "supplychain", "manufacturing", "EMS",
    "pcbdesign", "Metalworking", "3Dprinting",
    // Electronics & semiconductors
    "electronics", "semiconductor", "FPGA",
    "embedded", "RFelectronics",
    // Economy & trade
    "investing", "economics", "sanction",
    "ExportControls", "TradePolicy", "SanctionsCompliance",
    // Technology & innovation
    "technology", "artificial", "Automate",
    // Energy & environment
    "energy", "RenewableEnergy", "nuclear",
];

// ─────────────────────────────────────────────────────────────────────────────
// Reddit JSON API shapes
// ─────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct RedditListing {
    data: ListingData,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ListingData {
    children: Vec<PostWrapper>,
    after: Option<String>,
    before: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct PostWrapper {
    data: RedditPost,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct RedditPost {
    id: String,
    title: String,
    selftext: Option<String>,
    author: String,
    subreddit: String,
    score: Option<i64>,
    upvote_ratio: Option<f64>,
    num_comments: Option<u64>,
    url: Option<String>,
    permalink: String,
    created_utc: f64,
    over_18: Option<bool>,
    domain: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scraper
// ─────────────────────────────────────────────────────────────────────────────

/// Reddit OSINT scraper.
pub struct RedditScraper {
    client: Client,
}

impl RedditScraper {
    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntelOSINT/1.0 (+https://apexintel.io; research bot)");

        if let Some(proxy) = proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("Invalid proxy")?);
        }

        Ok(Self { client: builder.build()? })
    }

    /// Fetch the hottest posts from a subreddit.
    pub async fn subreddit_hot(&self, subreddit: &str, limit: u32) -> Result<Vec<SocialPost>> {
        let url = format!("{}/r/{}/.json?limit={}", REDDIT_BASE, subreddit, limit.min(100));
        self.fetch_listing(&url).await
    }

    /// Fetch the newest posts from a subreddit.
    pub async fn subreddit_new(&self, subreddit: &str, limit: u32) -> Result<Vec<SocialPost>> {
        let url = format!("{}/r/{}/new.json?limit={}", REDDIT_BASE, subreddit, limit.min(100));
        self.fetch_listing(&url).await
    }

    /// Full-text search across Reddit (experimental; only returns recent results).
    pub async fn search(&self, query: &str, subreddit: Option<&str>, limit: u32) -> Result<Vec<SocialPost>> {
        let url = if let Some(sr) = subreddit {
            format!("{}/r/{}/search.json?q={}&limit={}&restrict_sr=1&sort=new",
                REDDIT_BASE, sr, urlencoding::encode(query), limit.min(100))
        } else {
            format!("{}/search.json?q={}&limit={}&sort=new",
                REDDIT_BASE, urlencoding::encode(query), limit.min(100))
        };
        self.fetch_listing(&url).await
    }

    /// Fetch top posts from all OSINT subreddits in one pass.
    pub async fn sample_osint_subreddits(&self, posts_per_subreddit: u32) -> Vec<SocialPost> {
        let mut all = Vec::new();
        for sr in OSINT_SUBREDDITS {
            match self.subreddit_new(sr, posts_per_subreddit).await {
                Ok(posts) => all.extend(posts),
                Err(e) => tracing::warn!(subreddit=%sr, error=%e, "Reddit fetch failed"),
            }
        }
        all
    }

    // ── Private ───────────────────────────────────────────────────

    async fn fetch_listing(&self, url: &str) -> Result<Vec<SocialPost>> {
        debug!(url=%url, "Reddit API request");

        let resp: RedditListing = self
            .client
            .get(url)
            .send()
            .await
            .context("Reddit HTTP request failed")?
            .json()
            .await
            .context("Reddit JSON parse failed")?;

        Ok(resp.data.children.into_iter().map(|w| self.map_post(w.data)).collect())
    }

    fn map_post(&self, p: RedditPost) -> SocialPost {
        let published_at = Utc.timestamp_opt(p.created_utc as i64, 0)
            .single()
            .unwrap_or_else(Utc::now);

        // Combine title + body for text field
        let full_text = match &p.selftext {
            Some(body) if !body.is_empty() && body != "[removed]" && body != "[deleted]" => {
                format!("{}\n\n{}", p.title, body)
            }
            _ => p.title.clone(),
        };

        let post_url = format!("https://www.reddit.com{}", p.permalink.trim_end_matches('/'));

        let mut post = SocialPost::minimal(
            "reddit",
            &p.id,
            &p.author,
            &full_text,
            published_at,
        );

        post.like_count = p.score.unwrap_or(0).max(0) as u64;
        post.reply_count = p.num_comments.unwrap_or(0);
        post.post_url = post_url;

        post
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scraper_builds() {
        assert!(RedditScraper::new(None).is_ok());
    }

    #[test]
    fn osint_subreddits_not_empty() {
        assert!(!OSINT_SUBREDDITS.is_empty());
    }

    #[test]
    fn map_post_handles_no_body() {
        let s = RedditScraper::new(None).unwrap();
        let p = RedditPost {
            id: "abc123".to_string(),
            title: "Test title".to_string(),
            selftext: None,
            author: "testuser".to_string(),
            subreddit: "worldnews".to_string(),
            score: Some(100),
            upvote_ratio: Some(0.95),
            num_comments: Some(42),
            url: None,
            permalink: "/r/worldnews/comments/abc123/test/".to_string(),
            created_utc: 1_700_000_000.0,
            over_18: Some(false),
            domain: None,
        };
        let post = s.map_post(p);
        assert_eq!(post.platform, "reddit");
        assert!(post.raw_text.contains("Test title"));
        assert_eq!(post.like_count, 100);
        assert_eq!(post.reply_count, 42);
    }
}
