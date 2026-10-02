//! Industry Forum Presence Detection Module
//!
//! Detects and monitors tracked entities' presence on industry forums and
//! community platforms (Reddit, Hacker News, Stack Exchange, Discourse,
//! Discord, and custom feeds), returning normalised [`ForumMention`] items.
//!
//! Reddit/Irc/custom feeds are read through their RSS endpoints; the other
//! platforms are queried through their public JSON APIs. Every fetch goes
//! through the shared guarded HTTP client and every parse failure is reported
//! as a [`ParseOutcome::ParseFailed`] — never silently downgraded to an empty
//! result.

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// A forum/community source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumSource {
    pub source_id: String,
    pub name: String,
    pub url: String,
    pub forum_type: ForumType,
    pub region: String,
    /// Per-source keywords; matched in addition to the monitor's global
    /// tracked keywords.
    pub topics: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForumType {
    Reddit,
    Discourse,
    HackerNews,
    StackExchange,
    Discord,
    Irc,
    CustomForum,
}

impl ForumType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reddit => "reddit",
            Self::Discourse => "discourse",
            Self::HackerNews => "hacker_news",
            Self::StackExchange => "stack_exchange",
            Self::Discord => "discord",
            Self::Irc => "irc",
            Self::CustomForum => "custom_forum",
        }
    }
}

/// A forum post/mention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumMention {
    pub mention_id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: ForumType,
    pub author: Option<String>,
    pub title: String,
    pub content_preview: String,
    pub url: String,
    pub score: Option<i64>,
    pub comment_count: Option<u64>,
    pub published_at: Option<DateTime<Utc>>,
    pub matched_entities: Vec<String>,
    pub matched_keywords: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

impl ForumMention {
    pub fn is_trending(&self) -> bool {
        self.score.map(|s| s >= 100).unwrap_or(false)
    }
}

/// Forum monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumMonitorConfig {
    pub tracked_entities: Vec<String>,
    pub tracked_keywords: Vec<String>,
    /// Extra RSS feed URLs folded into the scan.
    pub custom_feeds: Vec<String>,
    pub max_results: u32,
}

impl Default for ForumMonitorConfig {
    fn default() -> Self {
        Self {
            tracked_entities: Vec::new(),
            tracked_keywords: vec!["defense contractor".to_string(), "arms".to_string()],
            custom_feeds: Vec::new(),
            max_results: 50,
        }
    }
}

/// Forum presence monitor.
#[derive(Debug, Clone)]
pub struct ForumMonitor {
    client: Client,
    config: ForumMonitorConfig,
    sources: Vec<ForumSource>,
}

impl ForumMonitor {
    pub fn new(config: ForumMonitorConfig) -> Self {
        let client = crate::http::external_client_or_panic(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(30),
            user_agent: Some("ApexIntel/1.0 (+https://apexintel.io) Forum Monitor".to_string()),
            ..crate::http::ExternalClientOptions::default()
        });
        Self {
            client,
            config,
            sources: Self::default_sources(),
        }
    }

    fn default_sources() -> Vec<ForumSource> {
        vec![
            ForumSource {
                source_id: "reddit-defense".to_string(),
                name: "r/Defense".to_string(),
                url: "https://www.reddit.com/r/Defense/.rss".to_string(),
                forum_type: ForumType::Reddit,
                region: "global".to_string(),
                topics: vec!["defense".to_string()],
                enabled: true,
            },
            ForumSource {
                source_id: "reddit-tech".to_string(),
                name: "r/tech".to_string(),
                url: "https://www.reddit.com/r/tech/.rss".to_string(),
                forum_type: ForumType::Reddit,
                region: "global".to_string(),
                topics: vec!["technology".to_string()],
                enabled: true,
            },
        ]
    }

    pub async fn scan_forums(&self) -> ParseOutcome<ForumMention> {
        let mut all_mentions = Vec::new();
        let mut any_parsed = false;
        let mut first_fetch_failure: Option<(String, Option<u16>)> = None;
        let mut first_parse_failure: Option<(String, String)> = None;

        for source in self.sources.iter().filter(|s| s.enabled) {
            let outcome = match source.forum_type {
                ForumType::Reddit | ForumType::Irc => self.scan_rss_source(source).await,
                ForumType::Discourse => self.scan_discourse(source).await,
                ForumType::HackerNews => self.scan_hackernews(source).await,
                ForumType::StackExchange => self.scan_stackexchange(source).await,
                ForumType::Discord => self.scan_discord(source).await,
                ForumType::CustomForum => self.scan_custom_forum(source).await,
            };
            match outcome {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    all_mentions.extend(items);
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    warn!(source = %source.source_id, error = %error, "Forum feed failed");
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    warn!(source = %source.source_id, error = %error, "Forum feed parser failed");
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        // Extra custom feeds configured globally.
        for feed in &self.config.custom_feeds {
            let source = ForumSource {
                source_id: format!("custom-{feed}"),
                name: feed.clone(),
                url: feed.clone(),
                forum_type: ForumType::CustomForum,
                region: "global".to_string(),
                topics: Vec::new(),
                enabled: true,
            };
            match self.scan_custom_forum(&source).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    all_mentions.extend(items);
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        all_mentions.sort_by_key(|m| std::cmp::Reverse(m.published_at));
        all_mentions.truncate(self.config.max_results as usize);
        info!(total = all_mentions.len(), "Forum monitoring complete");

        if let Some((error, redacted_sample)) = first_parse_failure {
            return ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            };
        }
        if !any_parsed {
            if let Some((error, http_status)) = first_fetch_failure {
                return ParseOutcome::FetchFailed { error, http_status };
            }
        }
        ParseOutcome::ParsedSuccessfully {
            items: all_mentions,
        }
    }

    /// Scan an RSS-backed source (Reddit, Irc, or a custom feed).
    async fn scan_rss_source(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        match self.fetch_rss(&source.url).await {
            ParseOutcome::ParsedSuccessfully { items } => {
                ParseOutcome::parsed(self.items_to_mentions(&items, source))
            }
            ParseOutcome::FetchFailed { error, http_status } => {
                ParseOutcome::FetchFailed { error, http_status }
            }
            ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            } => ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            },
        }
    }

    async fn fetch_rss(&self, url: &str) -> ParseOutcome<ForumRssItem> {
        let resp = match self.client.get(url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("forum feed request failed: {error}"),
                    None,
                )
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return ParseOutcome::fetch_failed(
                format!("forum feed returned HTTP {status}"),
                Some(status),
            );
        }
        let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(body) => body,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read forum feed: {error}"),
                    None,
                )
            }
        };
        let outcome = self.parse_rss(&body);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    fn parse_rss(&self, xml: &str) -> ParseOutcome<ForumRssItem> {
        use quick_xml::events::Event;
        use quick_xml::Reader;
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut items = Vec::new();
        let mut in_item = false;
        let mut current_tag = String::new();
        let mut title = String::new();
        let mut description = String::new();
        let mut link = String::new();
        let mut guid = String::new();
        let mut author = Option::<String>::None;
        let mut published: Option<DateTime<Utc>> = None;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag == "item" || tag == "entry" {
                        in_item = true;
                    } else if in_item {
                        current_tag = tag;
                    }
                }
                Ok(Event::End(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag == "item" || tag == "entry" {
                        items.push(ForumRssItem {
                            title: title.clone(),
                            description: description.clone(),
                            link: link.clone(),
                            guid: if guid.is_empty() {
                                link.clone()
                            } else {
                                guid.clone()
                            },
                            author: author.clone(),
                            published,
                        });
                        title.clear();
                        description.clear();
                        link.clear();
                        guid.clear();
                        author = None;
                        published = None;
                        in_item = false;
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_item {
                        let text = e.unescape().unwrap_or_default().to_string();
                        match current_tag.as_str() {
                            "title" => title = text,
                            "description" | "summary" | "content" => description = text,
                            "link" => link = text,
                            "guid" | "id" => guid = text,
                            "author" | "dc:creator" => author = Some(text),
                            "pubDate" | "published" | "updated" => {
                                published = parse_feed_timestamp(&text);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => {
                    // Truncated/malformed XML is a parser incident, not an
                    // empty feed: return a parse failure with a redacted sample.
                    return ParseOutcome::parse_failed(
                        format!("failed to parse forum RSS XML: {error}"),
                        xml,
                    );
                }
                _ => {}
            }
        }
        ParseOutcome::parsed(items)
    }

    fn items_to_mentions(&self, items: &[ForumRssItem], source: &ForumSource) -> Vec<ForumMention> {
        items
            .iter()
            .filter_map(|item| {
                let combined = format!("{} {}", item.title, item.description);
                let (entities, keywords) = self.match_terms(source, &combined);
                if entities.is_empty() && keywords.is_empty() {
                    return None;
                }
                Some(ForumMention {
                    mention_id: format!("{}-{}", source.source_id, item.guid),
                    source_id: source.source_id.clone(),
                    source_name: source.name.clone(),
                    source_type: source.forum_type,
                    author: item.author.clone(),
                    title: item.title.clone(),
                    content_preview: strip_html_tags(&item.description)
                        .chars()
                        .take(200)
                        .collect(),
                    url: item.link.clone(),
                    score: None,
                    comment_count: None,
                    published_at: item.published,
                    matched_entities: entities,
                    matched_keywords: keywords,
                    fetched_at: Utc::now(),
                })
            })
            .collect()
    }

    /// Scan a Discourse forum through its public `search.json` endpoint.
    async fn scan_discourse(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        let keywords = self.keyword_list(source);
        if keywords.is_empty() {
            return ParseOutcome::parsed(Vec::new());
        }
        let search_url = format!("{}/search.json", source.url.trim_end_matches('/'));
        let query = keywords.join(" ");
        let params = [("q", query.as_str())];

        let resp = match self.client.get(&search_url).query(&params).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("Discourse search request failed: {error}"),
                    None,
                )
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), "Discourse search returned non-success");
            return ParseOutcome::fetch_failed(
                format!("Discourse search returned HTTP {status}"),
                Some(status),
            );
        }
        let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(body) => body,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read Discourse response: {error}"),
                    None,
                )
            }
        };
        self.parse_discourse(&body, source)
    }

    fn parse_discourse(&self, body: &str, source: &ForumSource) -> ParseOutcome<ForumMention> {
        #[derive(Deserialize)]
        struct DiscourseSearch {
            #[serde(default)]
            posts: Vec<DiscoursePost>,
        }
        #[derive(Deserialize)]
        struct DiscoursePost {
            id: Option<i64>,
            title: Option<String>,
            cooked: Option<String>,
            username: Option<String>,
            created_at: Option<String>,
            like_count: Option<u64>,
            reply_count: Option<u64>,
            topic_id: Option<i64>,
            topic_slug: Option<String>,
        }

        let search: DiscourseSearch = match serde_json::from_str(body) {
            Ok(search) => search,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse Discourse JSON: {error}"),
                    body,
                )
            }
        };
        let base = source.url.trim_end_matches('/');
        let mentions = search
            .posts
            .into_iter()
            .filter_map(|post| {
                let title = post.title?;
                let cooked = strip_html_tags(&post.cooked?);
                let (entities, keywords) = self.match_terms(source, &format!("{title} {cooked}"));
                if entities.is_empty() && keywords.is_empty() {
                    return None;
                }
                let published_at = post.created_at.as_ref().and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });
                let activity_id = post.id.map(|id| id.to_string()).unwrap_or_default();
                let activity_url = post
                    .topic_id
                    .map(|id| {
                        format!(
                            "{base}/t/{}/{}",
                            post.topic_slug.clone().unwrap_or_default(),
                            id
                        )
                    })
                    .unwrap_or_else(|| source.url.clone());
                Some(ForumMention {
                    mention_id: format!("{}-{}", source.source_id, activity_id),
                    source_id: source.source_id.clone(),
                    source_name: source.name.clone(),
                    source_type: source.forum_type,
                    author: Some(
                        post.username
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    title,
                    content_preview: cooked.chars().take(200).collect(),
                    url: activity_url,
                    score: post.like_count.map(|n| n as i64),
                    comment_count: post.reply_count,
                    published_at,
                    matched_entities: entities,
                    matched_keywords: keywords,
                    fetched_at: Utc::now(),
                })
            })
            .collect();
        ParseOutcome::parsed(mentions)
    }

    /// Scan Hacker News through the public Algolia search API.
    async fn scan_hackernews(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        let keywords = self.keyword_list(source);
        if keywords.is_empty() {
            return ParseOutcome::parsed(Vec::new());
        }
        let query = keywords.join(" ");
        let url = format!(
            "https://hn.algolia.com/api/v1/search?query={}&tags=story",
            urlencoding::encode(&query)
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("Hacker News API request failed: {error}"),
                    None,
                )
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), "HN API returned non-success");
            return ParseOutcome::fetch_failed(
                format!("Hacker News API returned HTTP {status}"),
                Some(status),
            );
        }
        let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(body) => body,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read Hacker News response: {error}"),
                    None,
                )
            }
        };
        self.parse_hackernews(&body, source)
    }

    fn parse_hackernews(&self, body: &str, source: &ForumSource) -> ParseOutcome<ForumMention> {
        #[derive(Deserialize)]
        struct HnResponse {
            #[serde(default)]
            hits: Vec<HnHit>,
        }
        #[derive(Deserialize)]
        struct HnHit {
            #[serde(rename = "objectID", default)]
            object_id: String,
            title: Option<String>,
            author: Option<String>,
            created_at: Option<String>,
            points: Option<i64>,
            num_comments: Option<i64>,
            url: Option<String>,
        }

        let response: HnResponse = match serde_json::from_str(body) {
            Ok(response) => response,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse Hacker News JSON: {error}"),
                    body,
                )
            }
        };
        let mentions = response
            .hits
            .into_iter()
            .filter_map(|hit| {
                let title = hit.title?;
                let (entities, keywords) = self.match_terms(source, &title);
                if entities.is_empty() && keywords.is_empty() {
                    return None;
                }
                let published_at = hit.created_at.as_ref().and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });
                let activity_url = hit.url.unwrap_or_else(|| {
                    format!("https://news.ycombinator.com/item?id={}", hit.object_id)
                });
                Some(ForumMention {
                    mention_id: format!("{}-{}", source.source_id, hit.object_id),
                    source_id: source.source_id.clone(),
                    source_name: source.name.clone(),
                    source_type: source.forum_type,
                    author: hit.author,
                    title,
                    content_preview: String::new(),
                    url: activity_url,
                    score: hit.points,
                    comment_count: hit.num_comments.map(|n| n.max(0) as u64),
                    published_at,
                    matched_entities: entities,
                    matched_keywords: keywords,
                    fetched_at: Utc::now(),
                })
            })
            .collect();
        ParseOutcome::parsed(mentions)
    }

    /// Scan Stack Exchange through its public API.
    async fn scan_stackexchange(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        let tags = self.keyword_list(source).join(";");
        if tags.is_empty() {
            return ParseOutcome::parsed(Vec::new());
        }
        let url = format!(
            "https://api.stackexchange.com/2.3/questions?order=desc&sort=creation&tagged={}&site=stackoverflow",
            urlencoding::encode(&tags)
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("Stack Exchange API request failed: {error}"),
                    None,
                )
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), "Stack Exchange API returned non-success");
            return ParseOutcome::fetch_failed(
                format!("Stack Exchange API returned HTTP {status}"),
                Some(status),
            );
        }
        let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(body) => body,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read Stack Exchange response: {error}"),
                    None,
                )
            }
        };
        self.parse_stackexchange(&body, source)
    }

    fn parse_stackexchange(&self, body: &str, source: &ForumSource) -> ParseOutcome<ForumMention> {
        #[derive(Deserialize)]
        struct SoResponse {
            #[serde(default)]
            items: Vec<SoQuestion>,
        }
        #[derive(Deserialize)]
        struct SoQuestion {
            question_id: Option<i64>,
            title: Option<String>,
            body_markdown: Option<String>,
            owner: Option<SoOwner>,
            creation_date: Option<i64>,
            score: Option<i64>,
            answer_count: Option<i64>,
            link: Option<String>,
        }
        #[derive(Deserialize)]
        struct SoOwner {
            display_name: Option<String>,
            link: Option<String>,
        }

        let response: SoResponse = match serde_json::from_str(body) {
            Ok(response) => response,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse Stack Exchange JSON: {error}"),
                    body,
                )
            }
        };
        let mentions = response
            .items
            .into_iter()
            .filter_map(|question| {
                let title = question.title?;
                let body_text = question.body_markdown.clone().unwrap_or_default();
                let (entities, keywords) =
                    self.match_terms(source, &format!("{title} {body_text}"));
                if entities.is_empty() && keywords.is_empty() {
                    return None;
                }
                let published_at = question
                    .creation_date
                    .and_then(|ts| DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc));
                let activity_id = question
                    .question_id
                    .map(|id| id.to_string())
                    .unwrap_or_default();
                Some(ForumMention {
                    mention_id: format!("{}-{}", source.source_id, activity_id),
                    source_id: source.source_id.clone(),
                    source_name: source.name.clone(),
                    source_type: source.forum_type,
                    author: question
                        .owner
                        .as_ref()
                        .and_then(|owner| owner.display_name.clone())
                        .or_else(|| Some("unknown".to_string())),
                    title,
                    content_preview: strip_html_tags(&body_text).chars().take(200).collect(),
                    url: question.link.unwrap_or_else(|| source.url.clone()),
                    score: question.score,
                    comment_count: question.answer_count.map(|n| n.max(0) as u64),
                    published_at,
                    matched_entities: entities,
                    matched_keywords: keywords,
                    fetched_at: Utc::now(),
                })
            })
            .collect();
        ParseOutcome::parsed(mentions)
    }

    /// Scan Discord's public discoverable-guild search endpoint.
    async fn scan_discord(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        let keywords = self.keyword_list(source);
        if keywords.is_empty() {
            return ParseOutcome::parsed(Vec::new());
        }

        let mut mentions = Vec::new();
        let mut any_parsed = false;
        let mut first_fetch_failure: Option<(String, Option<u16>)> = None;
        let mut first_parse_failure: Option<(String, String)> = None;

        for keyword in keywords.iter().take(10) {
            let url = format!(
                "https://discord.com/api/v9/discoverable-guilds?query={}&limit=10",
                urlencoding::encode(keyword)
            );
            let resp = match self
                .client
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    first_fetch_failure.get_or_insert((
                        format!("Discord discoverable-guilds request failed: {error}"),
                        None,
                    ));
                    continue;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                first_fetch_failure
                    .get_or_insert((format!("Discord API returned HTTP {status}"), Some(status)));
                continue;
            }
            let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES)
                .await
            {
                Ok(body) => body,
                Err(error) => {
                    first_fetch_failure
                        .get_or_insert((format!("failed to read Discord response: {error}"), None));
                    continue;
                }
            };
            match parse_discord_guilds(&body) {
                Ok(guilds) => {
                    any_parsed = true;
                    for guild in guilds {
                        if let Some(mention) = self.guild_to_mention(guild, source) {
                            mentions.push(mention);
                        }
                    }
                }
                Err(error) => {
                    first_parse_failure
                        .get_or_insert((format!("failed to parse Discord JSON: {error}"), body));
                }
            }
        }

        if let Some((error, redacted_sample)) = first_parse_failure {
            return ParseOutcome::ParseFailed {
                error,
                redacted_sample: crate::parse_outcome::redact_sample(&redacted_sample),
            };
        }
        if !any_parsed {
            if let Some((error, http_status)) = first_fetch_failure {
                return ParseOutcome::FetchFailed { error, http_status };
            }
        }
        ParseOutcome::parsed(mentions)
    }

    fn guild_to_mention(&self, guild: DiscordGuild, source: &ForumSource) -> Option<ForumMention> {
        let name = guild.name?;
        let description = guild.description.unwrap_or_default();
        let (entities, keywords) = self.match_terms(source, &format!("{name} {description}"));
        if entities.is_empty() && keywords.is_empty() {
            return None;
        }
        let guild_id = guild.id.unwrap_or_default();
        Some(ForumMention {
            mention_id: format!("{}-{}", source.source_id, guild_id),
            source_id: source.source_id.clone(),
            source_name: source.name.clone(),
            source_type: source.forum_type,
            author: Some(name.clone()),
            title: format!("Discord Server: {name}"),
            content_preview: description.chars().take(200).collect(),
            url: format!("https://discord.com/servers/{guild_id}"),
            score: guild.approximate_member_count,
            comment_count: guild.approximate_presence_count.map(|n| n.max(0) as u64),
            published_at: Some(Utc::now()),
            matched_entities: entities,
            matched_keywords: keywords,
            fetched_at: Utc::now(),
        })
    }

    /// Scan a custom forum: RSS first, HTML keyword scan as fallback.
    async fn scan_custom_forum(&self, source: &ForumSource) -> ParseOutcome<ForumMention> {
        let rss_failure = match self.fetch_rss(&source.url).await {
            ParseOutcome::ParsedSuccessfully { items } => {
                let mentions = self.items_to_mentions(&items, source);
                if !mentions.is_empty() {
                    return ParseOutcome::parsed(mentions);
                }
                None
            }
            ParseOutcome::FetchFailed { error, http_status } => {
                Some(ParseOutcome::FetchFailed { error, http_status })
            }
            ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            } => Some(ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            }),
        };

        // Fallback: scrape the base URL HTML for keyword matches.
        let resp = match self.client.get(&source.url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return rss_failure.unwrap_or_else(|| {
                    ParseOutcome::fetch_failed(
                        format!("custom forum HTML fetch failed: {error}"),
                        None,
                    )
                });
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return rss_failure.unwrap_or_else(|| {
                ParseOutcome::fetch_failed(
                    format!("custom forum HTML returned HTTP {status}"),
                    Some(status),
                )
            });
        }
        let html = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(html) => html,
            Err(error) => {
                return rss_failure.unwrap_or_else(|| {
                    ParseOutcome::fetch_failed(
                        format!("failed to read custom forum HTML: {error}"),
                        None,
                    )
                });
            }
        };

        let plain = strip_html_tags(&html);
        let (entities, keywords) = self.match_terms(source, &plain);
        if entities.is_empty() && keywords.is_empty() {
            return ParseOutcome::parsed(Vec::new());
        }

        // Extract a relevant snippet around the first keyword match.
        let lower = plain.to_lowercase();
        let first_match = keywords
            .iter()
            .filter_map(|kw| lower.find(&kw.to_lowercase()).map(|pos| (pos, kw.len())))
            .min_by_key(|(pos, _)| *pos);
        let snippet = match first_match {
            Some((pos, len)) => {
                let start = pos.saturating_sub(200);
                let end = (pos + len + 300).min(plain.len());
                plain.get(start..end).unwrap_or(&plain).to_string()
            }
            None => plain.chars().take(500).collect(),
        };

        ParseOutcome::parsed(vec![ForumMention {
            mention_id: format!("{}-html", source.source_id),
            source_id: source.source_id.clone(),
            source_name: source.name.clone(),
            source_type: source.forum_type,
            author: Some("scraper".to_string()),
            title: format!("{} — keyword match", source.name),
            content_preview: snippet.chars().take(200).collect(),
            url: source.url.clone(),
            score: None,
            comment_count: None,
            published_at: None,
            matched_entities: entities,
            matched_keywords: keywords,
            fetched_at: Utc::now(),
        }])
    }

    pub fn mentions_for_entity<'a>(
        &self,
        mentions: &'a [ForumMention],
        entity: &str,
    ) -> Vec<&'a ForumMention> {
        mentions
            .iter()
            .filter(|m| {
                m.matched_entities
                    .iter()
                    .any(|e| e.to_lowercase() == entity.to_lowercase())
            })
            .collect()
    }

    pub fn trending_mentions<'a>(&self, mentions: &'a [ForumMention]) -> Vec<&'a ForumMention> {
        mentions.iter().filter(|m| m.is_trending()).collect()
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Global tracked keywords plus the source's own topics, deduplicated.
    fn keyword_list(&self, source: &ForumSource) -> Vec<String> {
        let mut keywords: Vec<String> = Vec::new();
        for keyword in self
            .config
            .tracked_keywords
            .iter()
            .chain(source.topics.iter())
        {
            if !keyword.trim().is_empty()
                && !keywords
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(keyword))
            {
                keywords.push(keyword.clone());
            }
        }
        keywords
    }

    /// Match tracked entities and keywords in `text`, returning both sets.
    fn match_terms(&self, source: &ForumSource, text: &str) -> (Vec<String>, Vec<String>) {
        let lower = text.to_lowercase();
        let mut entities: Vec<String> = Vec::new();
        for entity in &self.config.tracked_entities {
            if !entity.trim().is_empty()
                && lower.contains(&entity.to_lowercase())
                && !entities
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(entity))
            {
                entities.push(entity.clone());
            }
        }
        let keywords = self
            .keyword_list(source)
            .into_iter()
            .filter(|keyword| lower.contains(&keyword.to_lowercase()))
            .collect();
        (entities, keywords)
    }
}

/// One item parsed out of a forum RSS/Atom feed.
struct ForumRssItem {
    title: String,
    description: String,
    link: String,
    guid: String,
    author: Option<String>,
    published: Option<DateTime<Utc>>,
}

/// A Discord discoverable guild from the public search API.
#[derive(Debug, Clone, Deserialize)]
struct DiscordGuild {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    approximate_member_count: Option<i64>,
    approximate_presence_count: Option<i64>,
}

fn parse_discord_guilds(body: &str) -> Result<Vec<DiscordGuild>, serde_json::Error> {
    serde_json::from_str(body)
}

/// Parse an RSS/Atom timestamp (RFC 2822 pubDate or RFC 3339 published).
fn parse_feed_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    DateTime::parse_from_rfc2822(trimmed)
        .or_else(|_| DateTime::parse_from_rfc3339(trimmed))
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Strip HTML tags and decode the common entities.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                last_was_space = true;
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space && !result.is_empty() {
                        result.push(' ');
                    }
                    last_was_space = true;
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn monitor_with_keywords(keywords: &[&str]) -> ForumMonitor {
        ForumMonitor::new(ForumMonitorConfig {
            tracked_keywords: keywords.iter().map(|k| k.to_string()).collect(),
            ..Default::default()
        })
    }

    fn source(forum_type: ForumType, topics: &[&str]) -> ForumSource {
        ForumSource {
            source_id: format!("test-{}", forum_type.as_str()),
            name: "Test Forum".to_string(),
            url: "https://forum.example.com".to_string(),
            forum_type,
            region: "global".to_string(),
            topics: topics.iter().map(|t| t.to_string()).collect(),
            enabled: true,
        }
    }

    #[test]
    fn forum_monitor_source_count() {
        let m = ForumMonitor::new(Default::default());
        assert!(m.source_count() > 0);
    }

    #[test]
    fn forum_monitor_chaining() {
        let cfg = ForumMonitorConfig {
            tracked_entities: vec!["Lockheed Martin".to_string()],
            tracked_keywords: vec!["F-35".to_string()],
            custom_feeds: vec![],
            max_results: 50,
        };
        assert!(cfg
            .tracked_entities
            .contains(&"Lockheed Martin".to_string()));
    }

    #[test]
    fn forum_type_as_str() {
        assert_eq!(ForumType::Reddit.as_str(), "reddit");
        assert_eq!(ForumType::Discourse.as_str(), "discourse");
        assert_eq!(ForumType::HackerNews.as_str(), "hacker_news");
        assert_eq!(ForumType::StackExchange.as_str(), "stack_exchange");
        assert_eq!(ForumType::Discord.as_str(), "discord");
        assert_eq!(ForumType::Irc.as_str(), "irc");
        assert_eq!(ForumType::CustomForum.as_str(), "custom_forum");
    }

    #[test]
    fn forum_mention_is_trending() {
        let m = ForumMention {
            mention_id: "test".to_string(),
            source_id: "r".to_string(),
            source_name: "r/Defense".to_string(),
            source_type: ForumType::Reddit,
            author: Some("user".to_string()),
            title: "Test".to_string(),
            content_preview: "...".to_string(),
            url: "https://".to_string(),
            score: Some(500),
            comment_count: None,
            published_at: Some(Utc::now()),
            matched_entities: vec![],
            matched_keywords: vec![],
            fetched_at: Utc::now(),
        };
        assert!(m.is_trending());
    }

    #[test]
    fn parse_hackernews_extracts_mentions() {
        let monitor = monitor_with_keywords(&[]);
        let source = source(ForumType::HackerNews, &["sanctions"]);
        let body = r#"{"hits":[{"objectID":"1","title":"New sanctions on exporters","author":"alice","created_at":"2024-01-15T10:00:00Z","points":150,"num_comments":42,"url":"https://example.com/a"},{"objectID":"2","title":"Unrelated story","points":5}]}"#;
        let outcome = monitor.parse_hackernews(body, &source);
        let items = outcome.into_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mention_id, "test-hacker_news-1");
        assert_eq!(items[0].score, Some(150));
        assert_eq!(items[0].comment_count, Some(42));
        assert_eq!(items[0].matched_keywords, vec!["sanctions".to_string()]);
    }

    #[test]
    fn parse_hackernews_malformed_is_parse_failure() {
        let monitor = monitor_with_keywords(&[]);
        let source = source(ForumType::HackerNews, &["sanctions"]);
        let outcome = monitor.parse_hackernews("not json", &source);
        assert!(outcome.is_parse_failure());
        assert!(!outcome.is_parsed_success());
    }

    #[test]
    fn parse_stackexchange_extracts_mentions() {
        let monitor = monitor_with_keywords(&[]);
        let source = source(ForumType::StackExchange, &["sanctions"]);
        let body = r#"{"items":[{"question_id":7,"title":"Sanctions screening libraries","body_markdown":"<p>sanctions text</p>","owner":{"display_name":"bob","link":"https://so/u/bob"},"creation_date":1705312800,"score":9,"answer_count":3,"link":"https://stackoverflow.com/q/7","tags":["sanctions"]}]}"#;
        let outcome = monitor.parse_stackexchange(body, &source);
        let items = outcome.into_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mention_id, "test-stack_exchange-7");
        assert_eq!(items[0].author.as_deref(), Some("bob"));
        assert_eq!(items[0].score, Some(9));
        assert_eq!(items[0].comment_count, Some(3));
    }

    #[test]
    fn parse_discourse_extracts_mentions() {
        let monitor = monitor_with_keywords(&[]);
        let source = source(ForumType::Discourse, &["sanctions"]);
        let body = r#"{"posts":[{"id":11,"title":"Sanctions compliance tooling","cooked":"<p>sanctions text</p>","username":"carol","created_at":"2024-01-15T10:00:00Z","like_count":4,"reply_count":2,"topic_id":99,"topic_slug":"sanctions-tooling"}]}"#;
        let outcome = monitor.parse_discourse(body, &source);
        let items = outcome.into_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mention_id, "test-discourse-11");
        assert_eq!(items[0].content_preview, "sanctions text");
        assert!(items[0].url.ends_with("/t/sanctions-tooling/99"));
    }

    #[test]
    fn discord_guilds_parse_and_match() {
        let monitor = monitor_with_keywords(&[]);
        let source = source(ForumType::Discord, &["defense"]);
        let body = r#"[{"id":"42","name":"Defense Tech","description":"defense community","approximate_member_count":120,"approximate_presence_count":30}]"#;
        let guilds = parse_discord_guilds(body).expect("guild json");
        assert_eq!(guilds.len(), 1);
        let mention = monitor
            .guild_to_mention(guilds.into_iter().next().unwrap(), &source)
            .expect("guild must match");
        assert_eq!(mention.mention_id, "test-discord-42");
        assert_eq!(mention.score, Some(120));
        assert!(mention.is_trending());
    }

    #[test]
    fn parse_feed_timestamp_handles_rfc2822_and_3339() {
        assert!(parse_feed_timestamp("Mon, 15 Jan 2024 10:00:00 GMT").is_some());
        assert!(parse_feed_timestamp("2024-01-15T10:00:00Z").is_some());
        assert!(parse_feed_timestamp("not a date").is_none());
    }

    #[test]
    fn strip_html_tags_collapses_markup() {
        assert_eq!(
            strip_html_tags("<p>Hello &amp; <b>world</b></p>"),
            "Hello & world"
        );
    }
}
