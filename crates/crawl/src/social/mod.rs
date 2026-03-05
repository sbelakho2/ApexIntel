//! Social media intelligence scrapers.
//!
//! Sub-modules provide thin, rate-limited wrappers around public and
//! (where available) authenticated social APIs.  All scrapers return
//! [`SocialPost`] structs so downstream code is API-agnostic.
//!
//! # Architecture
//! - [`twitter`] — Twitter/X v2 API (bearer token) + public timeline fallback
//! - [`linkedin`] — LinkedIn public company page scraper + employee intel
//! - [`telegram`] — Open Telegram channel archiver via `t.me/s/`
//! - [`reddit`] — Reddit search + subreddit monitoring via public JSON API
//! - [`facebook`] — Facebook/Meta public page, group, and event scraper
//! - [`youtube`] — YouTube channel RSS + Data API v3 video monitoring
//! - [`discord`] — Discord community widget + invite preview monitor

pub mod twitter;
pub mod linkedin;
pub mod telegram;
pub mod reddit;
pub mod facebook;
pub mod youtube;
pub mod discord;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Shared output types
// ─────────────────────────────────────────────────────────────────────────────

/// A normalised social media post from any platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialPost {
    /// Originating platform: `"twitter"`, `"linkedin"`, `"telegram"`, `"reddit"`.
    pub platform: String,
    /// Native post identifier.
    pub post_id: String,
    /// Platform username / handle.
    pub author_handle: String,
    /// Display name if available.
    pub author_display_name: Option<String>,
    /// Number of followers/subscribers of the author.
    pub author_follower_count: Option<u64>,
    /// Verified/blue-tick status.
    pub author_verified: bool,
    /// Post body text (stripped of URLs for NLP).
    pub text: String,
    /// Original text including URLs.
    pub raw_text: String,
    /// Publication timestamp.
    pub published_at: DateTime<Utc>,
    /// Like count (0 if unavailable).
    pub like_count: u64,
    /// Repost / retweet / share count.
    pub share_count: u64,
    /// Reply count.
    pub reply_count: u64,
    /// URLs extracted from the post.
    pub urls: Vec<String>,
    /// Hashtags mentioned.
    pub hashtags: Vec<String>,
    /// Mentions (without `@`).
    pub mentions: Vec<String>,
    /// Direct link to the post.
    pub post_url: String,
    /// Language code detected (`"en"`, `"he"`, `"zh"`, …).
    pub language: Option<String>,
    /// Intelligence relevance score in [0, 1] (populated downstream).
    pub relevance_score: Option<f32>,
}

impl SocialPost {
    /// Construct a minimal post with required fields only.
    pub fn minimal(
        platform: &str,
        post_id: &str,
        author_handle: &str,
        text: &str,
        published_at: DateTime<Utc>,
    ) -> Self {
        Self {
            platform: platform.to_string(),
            post_id: post_id.to_string(),
            author_handle: author_handle.to_string(),
            author_display_name: None,
            author_follower_count: None,
            author_verified: false,
            text: strip_urls(text),
            raw_text: text.to_string(),
            published_at,
            like_count: 0,
            share_count: 0,
            reply_count: 0,
            urls: extract_urls(text),
            hashtags: extract_hashtags(text),
            mentions: extract_mentions(text),
            post_url: String::new(),
            language: None,
            relevance_score: None,
        }
    }

    /// Engagement score: weighted sum of likes, shares, replies.
    pub fn engagement_score(&self) -> f64 {
        self.like_count as f64 + self.share_count as f64 * 3.0 + self.reply_count as f64 * 2.0
    }

    /// Platform credibility factor for veracity analysis.
    ///
    /// T1 (0.85-0.95): verified official accounts, company pages, gov feeds
    /// T2 (0.55-0.70): established platforms with moderation (Reddit, HN, Mastodon)
    /// T3 (0.25-0.45): anonymous or unmoderated channels (Telegram, random forums)
    pub fn platform_credibility(&self) -> f64 {
        let base: f64 = match self.platform.as_str() {
            "linkedin" => 0.85,
            "twitter"  => 0.60,
            "reddit"   => 0.55,
            "mastodon" => 0.60,
            "bluesky"  => 0.55,
            "youtube"  => 0.65,
            "facebook" => 0.50,
            "telegram" => 0.35,
            "discord"  => 0.35,
            "forum"    => 0.50,
            _ => 0.40,
        };
        let verified_bonus: f64 = if self.author_verified { 0.15 } else { 0.0 };
        let engagement_bonus: f64 = if self.engagement_score() > 1000.0 { 0.10 }
                                     else if self.engagement_score() > 100.0 { 0.05 }
                                     else { 0.0 };
        (base + verified_bonus + engagement_bonus).min(0.98)
    }

    /// Whether this post needs corroboration before being treated as intelligence.
    pub fn needs_corroboration(&self) -> bool {
        self.platform_credibility() < 0.60 || !self.author_verified
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Text helpers
// ─────────────────────────────────────────────────────────────────────────────

fn strip_urls(text: &str) -> String {
    // Simple URL stripper — removes http(s):// tokens
    text.split_whitespace()
        .filter(|w| !w.starts_with("http://") && !w.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|u| u.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '=').to_string())
        .collect()
}

fn extract_hashtags(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with('#') && w.len() > 1)
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn extract_mentions(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with('@') && w.len() > 1)
        .map(|w| w[1..].trim_matches(|c: char| !c.is_alphanumeric() && c != '_').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn social_post_minimal_builds() {
        let p = SocialPost::minimal("twitter", "123", "elonmusk", "Hello world #AI @openai https://x.com", Utc::now());
        assert_eq!(p.platform, "twitter");
        assert!(!p.raw_text.is_empty());
        assert!(p.hashtags.contains(&"#AI".to_string()) || p.hashtags.contains(&"AI".to_string()));
        assert!(!p.urls.is_empty());
    }

    #[test]
    fn engagement_score_computed() {
        let mut p = SocialPost::minimal("linkedin", "id1", "handle", "test", Utc::now());
        p.like_count = 10;
        p.share_count = 5;
        p.reply_count = 3;
        // 10 + 15 + 6 = 31
        assert!((p.engagement_score() - 31.0).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_urls_works() {
        let urls = extract_urls("Check https://example.com and http://test.org for details");
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn strip_urls_removes_links() {
        let stripped = strip_urls("Buy now https://buy.com great deal");
        assert!(!stripped.contains("https://"));
        assert!(stripped.contains("Buy"));
    }
}
