//! Generic RSS/Atom feed parser for news, blogs, and press releases.
//!
//! Supports RSS 2.0, Atom, and Dublin Core extensions.
//! Used across the crawl pipeline for:
//! - Competitor press release monitoring
//! - Industry news aggregation
//! - Trade publication tracking
//! - Academic publication feeds

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::time::Duration;
use tracing::{debug, warn};

use crate::client::{CrawlClient, CrawlClientConfig, CrawlRequest};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// A single item from an RSS or Atom feed.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    pub description: String,
    pub published: Option<DateTime<Utc>>,
    pub author: Option<String>,
    pub categories: Vec<String>,
    pub guid: String,
}

/// RSS/Atom feed fetcher with built-in HTTP client.
pub struct RssFetcher {
    client: CrawlClient,
}

impl RssFetcher {
    /// Create a new fetcher with a given timeout.
    pub fn new(timeout_secs: u64) -> Result<Self> {
        let client = CrawlClient::new(CrawlClientConfig {
            timeout: Duration::from_secs(timeout_secs),
            user_agent: "ApexIntel/1.0 (+https://apexintel.io) RSS Reader".to_string(),
            ..CrawlClientConfig::default()
        })
        .map_err(anyhow::Error::from)
        .context("building RSS client")?;
        Ok(Self { client })
    }

    pub fn with_client(client: CrawlClient) -> Self {
        Self { client }
    }

    /// Fetch and parse a feed from a URL.
    pub async fn fetch(&self, url: &str) -> Result<Vec<FeedItem>> {
        debug!(url, "Fetching RSS feed");
        let response = self
            .client
            .fetch_text(&CrawlRequest::new(url))
            .await
            .map_err(anyhow::Error::from)
            .context("RSS HTTP GET")?;
        parse_feed(&response.body)
    }

    /// Fetch multiple feeds and merge results, sorted by date descending.
    pub async fn fetch_many(&self, urls: &[&str]) -> Vec<FeedItem> {
        let mut all_items = Vec::new();
        for url in urls {
            match self.fetch(url).await {
                Ok(items) => {
                    debug!(url, count = items.len(), "Parsed feed items");
                    all_items.extend(items);
                }
                Err(e) => {
                    warn!(url, error = %e, "Failed to fetch feed");
                }
            }
        }
        all_items.sort_by(|a, b| b.published.cmp(&a.published));
        all_items
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Feed parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse an RSS 2.0 or Atom XML string into `FeedItem`s.
pub fn parse_feed(xml: &str) -> Result<Vec<FeedItem>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut items = Vec::new();
    let mut in_item = false;
    let mut tag = String::new();
    let mut title = String::new();
    let mut link = String::new();
    let mut desc = String::new();
    let mut pub_date = String::new();
    let mut author = String::new();
    let mut guid = String::new();
    let mut categories: Vec<String> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" || name == "entry" {
                    in_item = true;
                }
                // Atom feeds use href attribute on <link>
                if name == "link" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            let href = String::from_utf8_lossy(&attr.value).to_string();
                            if in_item && link.is_empty() {
                                link = href;
                            }
                        }
                    }
                }
                tag = name;
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_item && name == "link" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            let href = String::from_utf8_lossy(&attr.value).to_string();
                            if link.is_empty() {
                                link = href;
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(ref e)) if in_item => {
                let text = e.unescape().unwrap_or_default().to_string();
                match tag.as_str() {
                    "title" => title = text,
                    "link" if link.is_empty() => link = text,
                    "description" | "summary" | "content" | "content:encoded" => desc = text,
                    "pubDate" | "published" | "updated" | "dc:date" => pub_date = text,
                    "author" | "dc:creator" | "name" => {
                        if author.is_empty() {
                            author = text;
                        }
                    }
                    "guid" | "id" => guid = text,
                    "category" => categories.push(text),
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if (name == "item" || name == "entry") && in_item {
                    if guid.is_empty() {
                        guid = link.clone();
                    }
                    let published = parse_date(&pub_date);
                    items.push(FeedItem {
                        title: title.clone(),
                        link: link.clone(),
                        description: desc.clone(),
                        published,
                        author: if author.is_empty() {
                            None
                        } else {
                            Some(author.clone())
                        },
                        categories: categories.clone(),
                        guid: guid.clone(),
                    });
                    title.clear();
                    link.clear();
                    desc.clear();
                    pub_date.clear();
                    author.clear();
                    guid.clear();
                    categories.clear();
                    in_item = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                warn!(error = %e, "XML parse error in feed");
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(items)
}

/// Best-effort date parsing supporting RFC 2822, RFC 3339, and common variants.
fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    let normalized = s.trim();
    // RFC 2822 (RSS)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(normalized) {
        return Some(dt.with_timezone(&Utc));
    }
    // Relaxed RFC 2822 for feeds with an incorrect weekday token.
    let relaxed_rfc2822 = normalized
        .split_once(", ")
        .map(|(_, rest)| rest)
        .unwrap_or(normalized);
    if let Ok(dt) = chrono::DateTime::parse_from_str(relaxed_rfc2822, "%d %b %Y %H:%M:%S %z") {
        return Some(dt.with_timezone(&Utc));
    }
    // RFC 3339 (Atom)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(normalized) {
        return Some(dt.with_timezone(&Utc));
    }
    // ISO 8601 without timezone
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(normalized, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc());
    }
    // Common date format
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(normalized, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc());
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Test Feed</title>
    <item>
      <title>First Article</title>
      <link>https://example.com/1</link>
      <description>First description</description>
      <pubDate>Mon, 01 Mar 2026 10:00:00 +0000</pubDate>
      <guid>article-001</guid>
      <category>Electronics</category>
      <category>PCB</category>
    </item>
    <item>
      <title>Second Article</title>
      <link>https://example.com/2</link>
      <description>Second description</description>
      <pubDate>Tue, 02 Mar 2026 12:00:00 +0000</pubDate>
      <author>John Doe</author>
    </item>
  </channel>
</rss>"#;

    const SAMPLE_ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Feed</title>
  <entry>
    <title>Atom Entry</title>
    <link href="https://example.com/atom/1"/>
    <id>urn:uuid:1234</id>
    <published>2026-03-01T08:00:00Z</published>
    <summary>Atom summary</summary>
    <author><name>Jane Smith</name></author>
  </entry>
</feed>"#;

    #[test]
    fn parse_rss_feed() {
        let items = parse_feed(SAMPLE_RSS)
            .unwrap_or_else(|error| panic!("sample RSS should parse: {error}"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First Article");
        assert_eq!(items[0].guid, "article-001");
        assert_eq!(items[0].categories.len(), 2);
        assert!(items[0].published.is_some());
        assert_eq!(items[1].author, Some("John Doe".into()));
    }

    #[test]
    fn parse_atom_feed() {
        let items = parse_feed(SAMPLE_ATOM)
            .unwrap_or_else(|error| panic!("sample Atom should parse: {error}"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Atom Entry");
        assert_eq!(items[0].link, "https://example.com/atom/1");
        assert_eq!(items[0].guid, "urn:uuid:1234");
        assert_eq!(items[0].author, Some("Jane Smith".into()));
    }

    #[test]
    fn parse_date_rfc2822() {
        let dt = parse_date("Mon, 01 Mar 2026 10:00:00 +0000");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_date_rfc3339() {
        let dt = parse_date("2026-03-01T08:00:00Z");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_date_empty() {
        assert!(parse_date("").is_none());
    }

    #[test]
    fn fetcher_builds() {
        assert!(RssFetcher::new(30).is_ok());
    }
}
