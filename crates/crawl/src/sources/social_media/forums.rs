//! Industry Forum Presence Detection Module

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// A forum/community source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumSource {
    pub source_id: String,
    pub name: String,
    pub url: String,
    pub forum_type: ForumType,
    pub region: String,
    pub topics: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForumType {
    Reddit,
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
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Forum Monitor")
            .build()
            .unwrap_or_else(|_| Client::new());
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
            match self.fetch_rss(&source.url).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    all_mentions.extend(self.items_to_mentions(&items, source));
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
        all_mentions.sort_by_key(|m| std::cmp::Reverse(m.published_at));
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
        let body = match resp.text().await {
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

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag == "item" {
                        in_item = true;
                    } else if in_item {
                        current_tag = tag;
                    }
                }
                Ok(Event::End(e)) => {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag == "item" {
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
                        });
                        title.clear();
                        description.clear();
                        link.clear();
                        guid.clear();
                        author = None;
                        in_item = false;
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_item {
                        let text = e.unescape().unwrap_or_default().to_string();
                        match current_tag.as_str() {
                            "title" => title = text,
                            "description" => description = text,
                            "link" => link = text,
                            "guid" => guid = text,
                            "author" | "dc:creator" => author = Some(text),
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
                let combined = format!("{} {}", item.title, item.description).to_lowercase();
                let entities: Vec<String> = self
                    .config
                    .tracked_entities
                    .iter()
                    .filter(|e| combined.contains(&e.to_lowercase()))
                    .cloned()
                    .collect();
                let keywords: Vec<String> = self
                    .config
                    .tracked_keywords
                    .iter()
                    .filter(|kw| combined.contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();
                if entities.is_empty() && keywords.is_empty() {
                    return None;
                }
                Some(ForumMention {
                    mention_id: format!(
                        "{}-{}-{}",
                        source.source_id,
                        item.guid,
                        Utc::now().timestamp()
                    ),
                    source_id: source.source_id.clone(),
                    source_name: source.name.clone(),
                    source_type: source.forum_type,
                    author: item.author.clone(),
                    title: item.title.clone(),
                    content_preview: item.description.chars().take(200).collect(),
                    url: item.link.clone(),
                    score: None,
                    comment_count: None,
                    published_at: Some(Utc::now()),
                    matched_entities: entities,
                    matched_keywords: keywords,
                    fetched_at: Utc::now(),
                })
            })
            .collect()
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
}

struct ForumRssItem {
    title: String,
    description: String,
    link: String,
    guid: String,
    author: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
