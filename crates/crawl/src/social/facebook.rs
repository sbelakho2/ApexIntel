//! Facebook (Meta) public page and group scraper.
//!
//! Scrapes mbasic.facebook.com (the minimal HTML version) using standard
//! HTTP — no headless browser required for public pages.  This avoids
//! Meta's aggressive JS-based bot detection on the main site.
//!
//! # Rate limiting
//! One request per 8 seconds to avoid IP blocks.

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::debug;

/// Minimum delay between requests to mbasic.facebook.com.
const REQUEST_DELAY: Duration = Duration::from_secs(8);

/// Facebook intelligence scraper (public pages/groups only).
pub struct FacebookScraper {
    client: Client,
}

impl FacebookScraper {
    /// Build a new scraper with the given user agent.
    pub fn new(user_agent: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .context("building Facebook HTTP client")?;
        Ok(Self { client })
    }

    /// Build from environment defaults.
    pub fn from_env() -> Result<Self> {
        Self::new("ApexIntel/1.0 (+https://apexintel.io) OSINT Collector")
    }

    // ── Public API ───────────────────────────────────────────────

    /// Scrape a public page's recent posts via mbasic.facebook.com.
    pub async fn scrape_page(&self, page_id: &str, max_posts: usize) -> Result<Vec<SocialPost>> {
        let url = format!("https://mbasic.facebook.com/{}", page_id);
        debug!(page_id, "Fetching Facebook page");
        let html = self.fetch_html(&url).await?;
        Ok(self.parse_page_posts(&html, page_id, max_posts))
    }

    /// Scrape a public group's recent posts.
    pub async fn scrape_group(&self, group_id: &str, max_posts: usize) -> Result<Vec<SocialPost>> {
        let url = format!("https://mbasic.facebook.com/groups/{}", group_id);
        debug!(group_id, "Fetching Facebook group");
        let html = self.fetch_html(&url).await?;
        Ok(self.parse_group_posts(&html, group_id, max_posts))
    }

    /// Scrape a public event page for discussion signals.
    pub async fn scrape_event(&self, event_id: &str) -> Result<Vec<SocialPost>> {
        let url = format!("https://mbasic.facebook.com/events/{}", event_id);
        debug!(event_id, "Fetching Facebook event");
        let html = self.fetch_html(&url).await?;
        Ok(self.parse_event_posts(&html, event_id))
    }

    // ── Private helpers ──────────────────────────────────────────

    async fn fetch_html(&self, url: &str) -> Result<String> {
        tokio::time::sleep(REQUEST_DELAY).await;

        let resp = self
            .client
            .get(url)
            .header("Accept-Language", "en-US,en;q=0.9,fr;q=0.8,ar;q=0.7")
            .send()
            .await
            .context("Facebook GET request failed")?;

        if !resp.status().is_success() {
            anyhow::bail!("Facebook returned HTTP {}", resp.status());
        }

        resp.text().await.context("reading Facebook response body")
    }

    fn parse_page_posts(&self, html: &str, page_id: &str, max_posts: usize) -> Vec<SocialPost> {
        let mut posts = Vec::new();

        // mbasic.facebook.com wraps stories in <article> or divs with
        // data-ft attributes.  We use a simple text-based scanner
        // because pulling in the `scraper` crate for HTML DOM parsing
        // adds weight; raw text works well on mbasic's minimal markup.

        for block in html.split("<article") {
            if posts.len() >= max_posts {
                break;
            }

            // Extract text content between the first ">" and "</article"
            let text = extract_text_block(block);
            if text.len() < 15 {
                continue;
            }

            // Try to find a permalink
            let permalink = extract_href(block, "/story.php")
                .or_else(|| extract_href(block, "/permalink"))
                .map(|h| {
                    if h.starts_with("http") {
                        h
                    } else {
                        format!("https://www.facebook.com{}", h)
                    }
                })
                .unwrap_or_default();

            // Relative timestamp
            let ts_text = extract_between(block, "<abbr>", "</abbr>").unwrap_or_default();
            let published = parse_relative_time(&ts_text).unwrap_or_else(Utc::now);

            let post_id = if permalink.is_empty() {
                format!("fb:{}:{}", page_id, sha256_12(&text))
            } else {
                format!("fb:{}", sha256_12(&permalink))
            };

            let mut post = SocialPost::minimal("facebook", &post_id, page_id, &text, published);
            post.post_url = permalink;
            posts.push(post);
        }

        posts
    }

    fn parse_group_posts(&self, html: &str, group_id: &str, max: usize) -> Vec<SocialPost> {
        let mut posts = Vec::new();

        for block in html.split("story_body_container") {
            if posts.len() >= max {
                break;
            }

            let text = extract_text_block(block);
            if text.len() < 15 {
                continue;
            }

            // Extract author (first <strong><a>…</a></strong> block)
            let author = extract_between(block, "<strong>", "</strong>")
                .and_then(|s| extract_between(&s, ">", "<"))
                .unwrap_or_else(|| format!("group:{}", group_id));

            let post = SocialPost::minimal(
                "facebook",
                &format!("fb:g:{}:{}", group_id, sha256_12(&text)),
                &author,
                &text,
                Utc::now(),
            );
            posts.push(post);
        }

        posts
    }

    fn parse_event_posts(&self, html: &str, event_id: &str) -> Vec<SocialPost> {
        let mut posts = Vec::new();

        // Event description block
        for block in html.split("event_description") {
            let text = extract_text_block(block);
            if text.len() < 15 {
                continue;
            }

            posts.push(SocialPost::minimal(
                "facebook",
                &format!("fb:ev:{}:{}", event_id, sha256_12(&text)),
                &format!("event:{}", event_id),
                &text,
                Utc::now(),
            ));
        }

        posts
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Text helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Produce a 12-char hex hash of input.
fn sha256_12(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))[..12].to_string()
}

/// Extract text content from an HTML block, stripping tags.
fn extract_text_block(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    // Clean up whitespace and decode entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract href value from a link containing the given prefix.
fn extract_href(html: &str, prefix: &str) -> Option<String> {
    let href_pos = html.find(&format!("href=\"{}", prefix))?;
    let after = &html[href_pos + 6..]; // skip `href="`
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// Extract text between two delimiters.
fn extract_between(html: &str, start: &str, end: &str) -> Option<String> {
    let s = html.find(start)? + start.len();
    let e = html[s..].find(end)?;
    Some(html[s..s + e].to_string())
}

/// Parse Facebook's relative time strings (multilingual).
fn parse_relative_time(text: &str) -> Option<DateTime<Utc>> {
    let text = text.to_lowercase();
    let now = Utc::now();

    if text.contains("just now") || text.contains("à l'instant") || text.contains("الآن") {
        return Some(now);
    }
    if text.contains("yesterday") || text.contains("hier") || text.contains("أمس") {
        return Some(now - chrono::Duration::days(1));
    }

    let n = extract_number(&text)?;

    if text.contains("min") || text.contains("دقيقة") {
        Some(now - chrono::Duration::minutes(n))
    } else if text.contains("hr") || text.contains("hour") || text.contains("heure") || text.contains("ساعة") {
        Some(now - chrono::Duration::hours(n))
    } else if text.contains("day") || text.contains("jour") || text.contains("يوم") {
        Some(now - chrono::Duration::days(n))
    } else if text.contains("week") || text.contains("semaine") || text.contains("أسبوع") {
        Some(now - chrono::Duration::weeks(n))
    } else {
        None
    }
}

fn extract_number(text: &str) -> Option<i64> {
    text.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_parsing() {
        assert!(parse_relative_time("just now").is_some());
        assert!(parse_relative_time("2 hrs ago").is_some());
        assert!(parse_relative_time("il y a 3 heures").is_some());
        assert!(parse_relative_time("yesterday").is_some());
        assert!(parse_relative_time("5 days ago").is_some());
        assert!(parse_relative_time("2 semaines").is_some());
        assert!(parse_relative_time("à l'instant").is_some());
    }

    #[test]
    fn sha256_short_hash() {
        let hash = sha256_12("test input");
        assert_eq!(hash.len(), 12);
        // deterministic
        assert_eq!(hash, sha256_12("test input"));
    }

    #[test]
    fn extract_text_block_strips_tags() {
        let html = "<p>Hello <b>world</b> &amp; foo</p>";
        let text = extract_text_block(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(text.contains("& foo"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn extract_href_finds_link() {
        let html = r#"<a href="/story.php?id=123&ref=m">text</a>"#;
        let href = extract_href(html, "/story.php");
        assert_eq!(href.unwrap(), "/story.php?id=123&ref=m");
    }

    #[test]
    fn scraper_builds() {
        let s = FacebookScraper::from_env();
        assert!(s.is_ok());
    }
}
