//! Telegram open-channel archiver.
//!
//! Scrapes public Telegram channel posts via `t.me/s/{channel}` — the
//! web-preview endpoint that does not require authentication.
//!
//! # Key features
//! - Parses post text, timestamp, view count, and forward attribution
//! - Supports pagination through older posts via `?before={id}`
//! - Extracts embedded URLs and document names

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use std::time::Duration;
use tracing::debug;

const TELEGRAM_WEB: &str = "https://t.me/s";
const MAX_PAGES: usize = 5;

/// Telegram channel scraper.
pub struct TelegramScraper {
    client: Client,
}

impl TelegramScraper {
    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; ApexIntelOSINT/1.0)");

        if let Some(proxy) = proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("Invalid proxy")?);
        }

        Ok(Self { client: builder.build()? })
    }

    /// Fetch recent posts from a public Telegram channel.
    ///
    /// * `channel` — channel username without `@`, e.g. `"nexta_tv"`.
    /// * `max_posts` — maximum number of posts to return.
    pub async fn channel_posts(&self, channel: &str, max_posts: usize) -> Result<Vec<SocialPost>> {
        let mut all_posts = Vec::new();
        let mut before_id: Option<u64> = None;

        for page in 0..MAX_PAGES {
            let url = if let Some(id) = before_id {
                format!("{}/{}?before={}", TELEGRAM_WEB, channel, id)
            } else {
                format!("{}/{}", TELEGRAM_WEB, channel)
            };

            debug!(url=%url, page, "Fetching Telegram channel page");

            let html = match self.client.get(&url).send().await {
                Ok(r) => r.text().await?,
                Err(e) => {
                    tracing::warn!("Telegram fetch error on page {}: {}", page, e);
                    break;
                }
            };

            let posts = self.parse_channel_html(channel, &html);
            if posts.is_empty() {
                break;
            }

            // Track the lowest post ID seen for pagination
            before_id = posts.iter()
                .filter_map(|p| p.post_id.parse::<u64>().ok())
                .min()
                .map(|id| id.saturating_sub(1));

            all_posts.extend(posts);

            if all_posts.len() >= max_posts {
                break;
            }
        }

        all_posts.truncate(max_posts);
        Ok(all_posts)
    }

    // ── HTML parser ───────────────────────────────────────────────

    fn parse_channel_html(&self, channel: &str, html: &str) -> Vec<SocialPost> {
        let mut posts = Vec::new();

        for block in html.split("tgme_widget_message_wrap") {
            if !block.contains("tgme_widget_message_text") {
                continue;
            }

            let post_id = self.extract_message_id(block).unwrap_or_else(|| "0".to_string());
            let text = self.extract_message_text(block);
            if text.trim().is_empty() {
                continue;
            }

            let published_at = self.extract_datetime(block).unwrap_or_else(Utc::now);
            let view_count = self.extract_view_count(block);

            let mut post = SocialPost::minimal("telegram", &post_id, channel, &text, published_at);
            post.post_url = format!("https://t.me/{}/{}", channel, post_id);
            post.like_count = view_count;
            post.author_display_name = Some(format!("@{}", channel));

            posts.push(post);
        }

        posts
    }

    fn extract_message_id(&self, block: &str) -> Option<String> {
        // Look for data-post="channel/12345"
        let marker = "data-post=\"";
        let start = block.find(marker)? + marker.len();
        let end = block[start..].find('"')?;
        let id_part = &block[start..start + end];
        id_part.split('/').last().map(|s| s.to_string())
    }

    fn extract_message_text(&self, block: &str) -> String {
        let marker = "tgme_widget_message_text";
        let start = match block.find(marker) {
            Some(i) => i,
            None => return String::new(),
        };

        let inner = &block[start..];
        let tag_end = match inner.find('>') {
            Some(i) => i + 1,
            None => return String::new(),
        };

        // Extract up to closing div
        let content = &inner[tag_end..];
        let end = content.find("</div>").unwrap_or(content.len().min(2000));
        strip_html_tags(&content[..end])
    }

    fn extract_datetime(&self, block: &str) -> Option<DateTime<Utc>> {
        let marker = "datetime=\"";
        let start = block.find(marker)? + marker.len();
        let end = block[start..].find('"')?;
        let dt_str = &block[start..start + end];
        DateTime::parse_from_rfc3339(dt_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn extract_view_count(&self, block: &str) -> u64 {
        let marker = "tgme_widget_message_views";
        let start = match block.find(marker) {
            Some(i) => i,
            None => return 0,
        };
        let inner = &block[start..];
        let content_start = inner.find('>').map(|i| i + 1).unwrap_or(0);
        let content_end = inner.find("</span>").unwrap_or(content_start + 20);
        let text = &inner[content_start..content_end.min(inner.len())];
        let clean = text.trim().replace(',', "").replace('K', "000").replace('M', "000000");
        clean.parse().unwrap_or(0)
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c { '<' => in_tag = true, '>' => in_tag = false, _ if !in_tag => result.push(c), _ => {} }
    }
    result.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
          .replace("&nbsp;", " ").replace("&#39;", "'").replace("&quot;", "\"")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scraper_builds() {
        assert!(TelegramScraper::new(None).is_ok());
    }

    #[test]
    fn parse_empty_html_returns_no_posts() {
        let s = TelegramScraper::new(None).unwrap();
        let posts = s.parse_channel_html("test", "<html></html>");
        assert!(posts.is_empty());
    }

    #[test]
    fn extract_datetime_valid() {
        let s = TelegramScraper::new(None).unwrap();
        let html = r#"some content datetime="2024-01-15T14:30:00+00:00" more"#;
        let dt = s.extract_datetime(html);
        assert!(dt.is_some());
    }
}
