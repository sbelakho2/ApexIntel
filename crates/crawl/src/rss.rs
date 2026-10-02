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
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::time::Duration;
use tracing::{debug, warn};
use url::Url;

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
    /// Full item body (`content:encoded`/`content`) when the feed carries one,
    /// falling back to the description. HTML is already stripped.
    pub content: String,
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
        // Resolve item links against the feed URL the caller asked for.
        let base = Url::parse(url).ok();
        parse_feed_with_base(&response.body, base.as_ref())
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
        all_items.sort_by_key(|b| std::cmp::Reverse(b.published));
        all_items
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Feed parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse an RSS 2.0 or Atom XML string into `FeedItem`s without a base URL.
pub fn parse_feed(xml: &str) -> Result<Vec<FeedItem>> {
    parse_feed_with_base(xml, None)
}

/// Parse an RSS 2.0 or Atom XML string, resolving relative item links against
/// the feed URL when one is known.
///
/// Only absolute `http(s)` links survive: relative links are resolved with
/// `Url::join` when `feed_url` is supplied (and dropped when it is not), and
/// non-http(s) schemes (`javascript:`, `data:`, `file:`, …) are discarded
/// rather than stored raw.
pub fn parse_feed_with_base(xml: &str, feed_url: Option<&Url>) -> Result<Vec<FeedItem>> {
    let mut reader = Reader::from_str(xml);
    // Do not trim individual text events: text split around comments/entities
    // must keep its bytes so appending (never overwriting) reconstructs the
    // original field value.
    reader.config_mut().trim_text(false);
    let mut items = Vec::new();
    let mut in_item = false;
    let mut saw_element = false;
    let mut buffers = ItemBuffers::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                saw_element = true;
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" || name == "entry" {
                    in_item = true;
                    buffers = ItemBuffers::default();
                } else if in_item && name == "category" {
                    buffers.category.clear();
                    if let Some(term) = attribute(e, b"term") {
                        if !term.trim().is_empty() {
                            buffers.categories.push(term.trim().to_string());
                        }
                    }
                } else if in_item && name == "link" {
                    buffers.begin_link(e);
                }
                buffers.tag = name;
            }
            Ok(Event::Empty(ref e)) => {
                saw_element = true;
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_item && name == "link" {
                    buffers.begin_link(e);
                } else if in_item && name == "category" {
                    // Atom categories are empty elements carrying `term`.
                    if let Some(term) = attribute(e, b"term") {
                        if !term.trim().is_empty() {
                            buffers.categories.push(term.trim().to_string());
                        }
                    }
                }
                // The empty element owns no text: never attribute following
                // text to its (already closed) name.
                buffers.tag.clear();
            }
            Ok(Event::Text(ref e)) if in_item => {
                if let Ok(text) = e.unescape() {
                    buffers.append(&text);
                }
            }
            Ok(Event::CData(ref e)) if in_item => {
                if let Ok(text) = e.decode() {
                    buffers.append(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_item && name == "link" {
                    buffers.finish_text_link();
                } else if in_item && name == "category" {
                    let value = buffers.category.trim().to_string();
                    if !value.is_empty() {
                        buffers.categories.push(value);
                    }
                    buffers.category.clear();
                } else if in_item && (name == "item" || name == "entry") {
                    items.push(buffers.take_item(feed_url));
                    in_item = false;
                }
                // Reset the current element so trailing text is never
                // attributed to the element that just closed.
                buffers.tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                if !saw_element && items.is_empty() {
                    return Err(anyhow::anyhow!("feed XML parse error: {e}"));
                }
                warn!(error = %e, "XML parse error in feed; keeping items parsed so far");
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    if !saw_element {
        return Err(anyhow::anyhow!(
            "feed body contains no XML elements (not an RSS/Atom document)"
        ));
    }

    Ok(items)
}

/// Mutable buffers for the item currently being parsed.
///
/// Text is *appended* per current element so entity/CDATA-split chunks survive;
/// buffers are reset only when a new `<item>`/`<entry>` starts.
#[derive(Default)]
struct ItemBuffers {
    tag: String,
    title: String,
    link: String,
    link_chosen: bool,
    link_is_alternate: bool,
    /// True while the current `<link>` carries `rel="self"`/`rel="enclosure"`.
    link_skippable: bool,
    description: String,
    content: String,
    pub_date: String,
    author: String,
    guid: String,
    categories: Vec<String>,
    category: String,
}

impl ItemBuffers {
    fn append(&mut self, text: &str) {
        match self.tag.as_str() {
            "title" => self.title.push_str(text),
            "link" => {
                if !self.link_chosen && !self.link_skippable {
                    self.link.push_str(text);
                }
            }
            "description" | "summary" => self.description.push_str(text),
            "content" | "content:encoded" => self.content.push_str(text),
            "pubDate" | "published" | "updated" | "dc:date" => self.pub_date.push_str(text),
            "author" | "dc:creator" | "name" => self.author.push_str(text),
            "guid" | "id" => self.guid.push_str(text),
            "category" => self.category.push_str(text),
            _ => {}
        }
    }

    /// Handle `<link …>` attributes (Atom): prefer `rel="alternate"`, else the
    /// first link that is neither `self` nor `enclosure`.
    fn begin_link(&mut self, element: &BytesStart<'_>) {
        self.link_skippable = false;
        let rel = attribute(element, b"rel").map(|value| value.trim().to_ascii_lowercase());
        if let Some(href) = attribute(element, b"href") {
            let href = href.trim();
            if href.is_empty() {
                return;
            }
            match rel.as_deref() {
                Some("self") | Some("enclosure") => {
                    self.link_skippable = true;
                }
                Some("alternate") => {
                    self.link = href.to_string();
                    self.link_chosen = true;
                    self.link_is_alternate = true;
                }
                _ => {
                    if !self.link_chosen {
                        self.link = href.to_string();
                        self.link_chosen = true;
                    }
                }
            }
        } else if matches!(rel.as_deref(), Some("self") | Some("enclosure")) {
            self.link_skippable = true;
        }
    }

    /// Commit an RSS-style `<link>https://…</link>` body when it is usable.
    fn finish_text_link(&mut self) {
        if self.link_chosen || self.link_skippable {
            return;
        }
        let value = self.link.trim();
        if !value.is_empty() {
            self.link = value.to_string();
            self.link_chosen = true;
        }
    }

    fn take_item(&mut self, feed_url: Option<&Url>) -> FeedItem {
        let title = self.title.trim().to_string();
        let description = strip_html(self.description.trim());
        let content = strip_html(self.content.trim());
        let description = if description.is_empty() {
            content.clone()
        } else {
            description
        };
        let content = if content.is_empty() {
            description.clone()
        } else {
            content
        };
        let link = resolve_http_link(self.link.trim(), feed_url);
        let guid = {
            let raw = self.guid.trim();
            if raw.is_empty() {
                link.clone()
            } else {
                raw.to_string()
            }
        };
        let author = {
            let raw = self.author.trim();
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_string())
            }
        };
        let categories = self
            .categories
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();
        FeedItem {
            title,
            link,
            description,
            content,
            published: parse_date(self.pub_date.trim()),
            author,
            categories,
            guid,
        }
    }
}

/// First attribute with `key`, with XML entities resolved.
fn attribute(element: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref() == key)
        .and_then(|attribute| attribute.unescape_value().ok())
        .map(|value| value.to_string())
}

/// Resolve a raw item link and keep it only when it is absolute `http(s)`.
fn resolve_http_link(raw: &str, base: Option<&Url>) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let resolved = match base {
        Some(base) => base.join(raw).ok(),
        None => Url::parse(raw).ok(),
    };
    match resolved {
        Some(url) if matches!(url.scheme(), "http" | "https") => url.to_string(),
        _ => {
            debug!(link = raw, "Dropping non-http(s) feed item link");
            String::new()
        }
    }
}

/// Cheap check for markup before invoking the full body extractor.
fn looks_like_html(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || pair[1] == b'/' || pair[1] == b'!')
    })
}

/// Strip HTML from a feed description/body using the parse crate's body
/// extractor, so literal `<p>` markup never reaches the UI. Plain text passes
/// through untouched.
fn strip_html(fragment: &str) -> String {
    let trimmed = fragment.trim();
    if trimmed.is_empty() || !looks_like_html(trimmed) {
        return trimmed.to_string();
    }
    match apex_parse::html::extract_page(trimmed, None) {
        Ok(page) => page.body_text.trim().to_string(),
        Err(error) => {
            warn!(error = %error, "Feed HTML extraction failed; falling back to tag stripping");
            apex_parse::normalizer::normalize_whitespace(&apex_parse::normalizer::strip_html_tags(
                trimmed,
            ))
        }
    }
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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

    #[test]
    fn cdata_title_and_description_are_captured() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title><![CDATA[Breaking CDATA Title]]></title>
    <link>https://example.com/cdata</link>
    <description><![CDATA[<p>CDATA body</p>]]></description>
  </item>
</channel></rss>"#;
        let items = parse_feed(xml).expect("CDATA feed parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Breaking CDATA Title");
        assert_eq!(items[0].description, "CDATA body");
    }

    #[test]
    fn entity_split_text_is_appended_not_overwritten() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Tom &amp; Jerry</title>
    <link>https://example.com/tom</link>
    <description>Tom &amp; Jerry split &amp; survive</description>
  </item>
</channel></rss>"#;
        let items = parse_feed(xml).expect("entity feed parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Tom & Jerry");
        assert_eq!(items[0].description, "Tom & Jerry split & survive");
    }

    #[test]
    fn text_split_across_events_is_appended_not_overwritten() {
        // A comment and a CDATA section split the element text into separate
        // events; every fragment must be appended, not just the last one.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Part1<!-- split --> Part2</title>
    <link>https://example.com/split</link>
    <description>pre<![CDATA[<b>mid</b>]]>post</description>
  </item>
</channel></rss>"#;
        let items = parse_feed(xml).expect("split feed parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Part1 Part2");
        let description = &items[0].description;
        for fragment in ["pre", "mid", "post"] {
            assert!(
                description.contains(fragment),
                "fragment {fragment:?} was dropped from {description:?}"
            );
        }
    }

    #[test]
    fn self_and_enclosure_links_are_skipped() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>Multi Link</title>
    <link rel="self" href="https://example.com/self"/>
    <link rel="enclosure" href="https://example.com/audio.mp3"/>
    <link rel="alternate" href="https://example.com/entry"/>
    <id>urn:uuid:multi</id>
  </entry>
</feed>"#;
        let items = parse_feed(xml).expect("atom feed parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].link, "https://example.com/entry");
    }

    #[test]
    fn first_non_self_link_wins_without_alternate() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>Plain Link</title>
    <link rel="self" href="https://example.com/self"/>
    <link rel="enclosure" href="https://example.com/audio.mp3"/>
    <link href="https://example.com/plain"/>
  </entry>
</feed>"#;
        let items = parse_feed(xml).expect("atom feed parses");
        assert_eq!(items[0].link, "https://example.com/plain");
    }

    #[test]
    fn atom_category_term_is_captured() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>Categorized</title>
    <link rel="alternate" href="https://example.com/cat"/>
    <category term="Semiconductors"/>
    <category term="Supply Chain"/>
  </entry>
</feed>"#;
        let items = parse_feed(xml).expect("atom feed parses");
        assert_eq!(
            items[0].categories,
            vec!["Semiconductors".to_string(), "Supply Chain".to_string()]
        );
    }

    #[test]
    fn relative_item_link_is_resolved_against_feed_url() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Relative</title>
    <link>/news/1?utm=1&amp;b=2</link>
  </item>
</channel></rss>"#;
        let base = Url::parse("https://example.com/feed.xml").expect("valid base");
        let items = parse_feed_with_base(xml, Some(&base)).expect("feed parses");
        assert_eq!(items[0].link, "https://example.com/news/1?utm=1&b=2");
    }

    #[test]
    fn hostile_javascript_link_is_dropped() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Hostile</title>
    <link>javascript:alert(1)</link>
    <guid>hostile-1</guid>
  </item>
</channel></rss>"#;
        let base = Url::parse("https://example.com/feed.xml").expect("valid base");
        let items = parse_feed_with_base(xml, Some(&base)).expect("feed parses");
        assert_eq!(items[0].link, "");
        // The raw hostile link is never stored elsewhere either.
        assert!(!items[0].description.contains("javascript"));
    }

    #[test]
    fn description_html_is_stripped() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Markup</title>
    <link>https://example.com/markup</link>
    <description>&lt;p&gt;Hello &lt;b&gt;world&lt;/b&gt;&lt;/p&gt;</description>
  </item>
</channel></rss>"#;
        let items = parse_feed(xml).expect("feed parses");
        assert!(
            !items[0].description.contains('<'),
            "{:?}",
            items[0].description
        );
        assert!(
            !items[0].description.contains('>'),
            "{:?}",
            items[0].description
        );
        assert!(
            items[0].description.contains("Hello"),
            "{:?}",
            items[0].description
        );
        assert!(
            items[0].description.contains("world"),
            "{:?}",
            items[0].description
        );
    }

    #[test]
    fn non_http_absolute_link_is_dropped() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item>
    <title>Ftp</title>
    <link>ftp://example.com/file</link>
  </item>
</channel></rss>"#;
        let items = parse_feed(xml).expect("feed parses");
        assert_eq!(items[0].link, "");
    }

    #[test]
    fn trailing_text_after_element_close_is_not_attributed() {
        // Whitespace/noise after `</title>` must not be appended to the title
        // through a stale current-element pointer.
        let xml = "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><item>\
                   <title>Clean</title>   stray   <link>https://example.com/clean</link>\
                   </item></channel></rss>";
        let items = parse_feed(xml).expect("feed parses");
        assert_eq!(items[0].title, "Clean");
    }
}
