//! YouTube public channel and video monitor.
//!
//! Uses the YouTube Data API v3 (free tier: 10,000 units/day) when a key
//! is configured.  Falls back to RSS feed scraping for channel monitoring
//! which requires no API key.
//!
//! # Env vars
//! - `YOUTUBE_API_KEY` — YouTube Data API v3 key (optional; RSS works without)

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, warn};

const YOUTUBE_RSS_BASE: &str = "https://www.youtube.com/feeds/videos.xml";
const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3";

/// YouTube intelligence scraper.
pub struct YouTubeScraper {
    client: Client,
    api_key: Option<String>,
}

impl YouTubeScraper {
    pub fn new(api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) OSINT Collector")
            .build()
            .context("building YouTube HTTP client")?;
        Ok(Self { client, api_key })
    }

    /// Build from environment variable `YOUTUBE_API_KEY`.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("YOUTUBE_API_KEY").ok().filter(|k| !k.is_empty());
        Self::new(key)
    }

    // ── Channel RSS (no API key) ────────────────────────────────

    /// Fetch latest videos from a channel via RSS (no API key required).
    pub async fn fetch_channel_rss(
        &self,
        channel_id: &str,
        max: usize,
    ) -> Result<Vec<SocialPost>> {
        let url = format!("{}?channel_id={}", YOUTUBE_RSS_BASE, channel_id);
        debug!(channel_id, "Fetching YouTube RSS");

        let xml = self
            .client
            .get(&url)
            .send()
            .await
            .context("YouTube RSS GET")?
            .text()
            .await
            .context("reading YouTube RSS body")?;

        Ok(parse_youtube_rss(&xml, max))
    }

    // ── API search (requires key) ───────────────────────────────

    /// Search videos by keyword via YouTube Data API v3.
    pub async fn search_videos(
        &self,
        query: &str,
        max: usize,
    ) -> Result<Vec<SocialPost>> {
        let key = self
            .api_key
            .as_deref()
            .context("YouTube API key required for search")?;

        let url = format!(
            "{}/search?part=snippet&type=video&q={}&maxResults={}&key={}",
            YOUTUBE_API_BASE,
            urlencoding::encode(query),
            max.min(50),
            key
        );

        debug!(query, "YouTube API search");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("YouTube API search request")?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "YouTube API non-200");
            return Ok(vec![]);
        }

        let data: YtSearchResponse = resp.json().await.context("parsing YouTube API response")?;

        let posts = data
            .items
            .into_iter()
            .filter_map(|item| {
                let video_id = item.id.video_id?;
                let s = item.snippet;
                let published = s
                    .published_at
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now());

                let mut post = SocialPost::minimal(
                    "youtube",
                    &format!("yt:{}", video_id),
                    &s.channel_title,
                    &format!("{} — {}", s.title, s.description),
                    published,
                );
                post.post_url = format!("https://youtube.com/watch?v={}", video_id);
                post.urls = vec![post.post_url.clone()];
                post.author_display_name = Some(s.channel_title);
                Some(post)
            })
            .collect();

        Ok(posts)
    }

    /// Get video statistics (view count, like count) for enrichment.
    pub async fn get_video_stats(&self, video_id: &str) -> Result<VideoStats> {
        let key = self
            .api_key
            .as_deref()
            .context("YouTube API key required for stats")?;

        let url = format!(
            "{}/videos?part=statistics&id={}&key={}",
            YOUTUBE_API_BASE, video_id, key
        );

        let resp: YtVideoResponse = self
            .client
            .get(&url)
            .send()
            .await?
            .json()
            .await
            .context("parsing video stats")?;

        let stats = resp
            .items
            .first()
            .map(|v| VideoStats {
                view_count: v.statistics.view_count.parse().unwrap_or(0),
                like_count: v.statistics.like_count.parse().unwrap_or(0),
                comment_count: v.statistics.comment_count.parse().unwrap_or(0),
            })
            .unwrap_or_default();

        Ok(stats)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RSS parser (no API key needed)
// ─────────────────────────────────────────────────────────────────────────────

fn parse_youtube_rss(xml: &str, max: usize) -> Vec<SocialPost> {
    let mut posts = Vec::new();

    // Simple tag-based parser — YouTube RSS is extremely uniform Atom XML:
    //   <entry>
    //     <yt:videoId>...</yt:videoId>
    //     <title>...</title>
    //     <published>...</published>
    //     <author><name>...</name></author>
    //   </entry>

    for entry_block in xml.split("<entry>").skip(1) {
        if posts.len() >= max {
            break;
        }

        let video_id = extract_tag(entry_block, "yt:videoId").unwrap_or_default();
        let title = extract_tag(entry_block, "title").unwrap_or_default();
        let published_str = extract_tag(entry_block, "published").unwrap_or_default();
        let author = extract_tag(entry_block, "name").unwrap_or_default();

        if video_id.is_empty() || title.is_empty() {
            continue;
        }

        let published = published_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now());

        let mut post = SocialPost::minimal(
            "youtube",
            &format!("yt:{}", video_id),
            &author,
            &title,
            published,
        );
        post.post_url = format!("https://youtube.com/watch?v={}", video_id);
        post.urls = vec![post.post_url.clone()];
        post.author_display_name = Some(author);

        posts.push(post);
    }

    posts
}

/// Extract text content of an XML tag like `<tag>content</tag>`.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    let content = xml[start..start + end].trim().to_string();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// API response types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct YtSearchResponse {
    items: Vec<YtSearchItem>,
}

#[derive(Deserialize, Debug)]
struct YtSearchItem {
    id: YtId,
    snippet: YtSnippet,
}

#[derive(Deserialize, Debug)]
struct YtId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct YtSnippet {
    title: String,
    description: String,
    channel_title: String,
    published_at: String,
}

#[derive(Deserialize, Debug)]
struct YtVideoResponse {
    items: Vec<YtVideoItem>,
}

#[derive(Deserialize, Debug)]
struct YtVideoItem {
    statistics: YtStatistics,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct YtStatistics {
    view_count: String,
    like_count: String,
    comment_count: String,
}

/// Video engagement statistics.
#[derive(Debug, Default, Clone)]
pub struct VideoStats {
    pub view_count: u64,
    pub like_count: u64,
    pub comment_count: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
<entry>
<yt:videoId>dQw4w9WgXcQ</yt:videoId>
<title>Sample Video Title</title>
<published>2026-01-15T12:00:00+00:00</published>
<author><name>TestChannel</name></author>
</entry>
<entry>
<yt:videoId>abc123xyz</yt:videoId>
<title>Another Video</title>
<published>2026-01-14T08:00:00+00:00</published>
<author><name>TestChannel</name></author>
</entry>
</feed>"#;

    #[test]
    fn parse_rss_extracts_entries() {
        let posts = parse_youtube_rss(SAMPLE_RSS, 10);
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].post_id, "yt:dQw4w9WgXcQ");
        assert_eq!(posts[0].author_handle, "TestChannel");
        assert!(posts[0].text.contains("Sample Video Title"));
        assert!(posts[0].post_url.contains("dQw4w9WgXcQ"));
    }

    #[test]
    fn parse_rss_respects_max() {
        let posts = parse_youtube_rss(SAMPLE_RSS, 1);
        assert_eq!(posts.len(), 1);
    }

    #[test]
    fn extract_tag_works() {
        assert_eq!(
            extract_tag("<yt:videoId>abc</yt:videoId>", "yt:videoId"),
            Some("abc".into())
        );
        assert_eq!(extract_tag("<foo>bar</foo>", "baz"), None);
    }

    #[test]
    fn scraper_builds_without_key() {
        let s = YouTubeScraper::new(None);
        assert!(s.is_ok());
    }
}
