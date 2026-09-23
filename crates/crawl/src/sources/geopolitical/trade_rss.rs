//! Trade Publication RSS Aggregation Module
//!
//! Aggregates RSS feeds from defense, trade, and geopolitical publications.

use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A trade publication feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeFeedSource {
    pub feed_id: String,
    pub name: String,
    pub feed_url: String,
    pub region: TradeRegion,
    pub category: TradeCategory,
    pub topics: Vec<String>,
    pub enabled: bool,
    pub last_fetched: Option<DateTime<Utc>>,
    pub last_item_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeRegion {
    Global,
    NorthAmerica,
    Europe,
    MiddleEast,
    AsiaPacific,
    Russia,
    Africa,
}

impl TradeRegion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::NorthAmerica => "north_america",
            Self::Europe => "europe",
            Self::MiddleEast => "middle_east",
            Self::AsiaPacific => "asia_pacific",
            Self::Russia => "russia",
            Self::Africa => "africa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeCategory {
    Defense,
    Trade,
    Geopolitics,
    SupplyChain,
    Finance,
    Technology,
    Energy,
}

impl TradeCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Defense => "defense",
            Self::Trade => "trade",
            Self::Geopolitics => "geopolitics",
            Self::SupplyChain => "supply_chain",
            Self::Finance => "finance",
            Self::Technology => "technology",
            Self::Energy => "energy",
        }
    }
}

/// Trade RSS aggregation monitor.
#[derive(Debug, Clone)]
pub struct TradeRssMonitor {
    client: Client,
    sources: Vec<TradeFeedSource>,
}

impl TradeRssMonitor {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Trade Monitor")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            sources: Self::default_sources(),
        }
    }

    fn default_sources() -> Vec<TradeFeedSource> {
        vec![
            TradeFeedSource {
                feed_id: "breaking_defense".into(),
                name: "Breaking Defense".into(),
                feed_url: "https://breakingdefense.com/feed/".into(),
                region: TradeRegion::NorthAmerica,
                category: TradeCategory::Defense,
                topics: vec!["defense".into(), "military".into(), "procurement".into()],
                enabled: true,
                last_fetched: None,
                last_item_count: 0,
            },
            TradeFeedSource {
                feed_id: "defense_news".into(),
                name: "Defense News".into(),
                feed_url: "https://www.defensenews.com/arc/outboundfeeds/rss/".into(),
                region: TradeRegion::Global,
                category: TradeCategory::Defense,
                topics: vec!["defense".into(), "industry".into()],
                enabled: true,
                last_fetched: None,
                last_item_count: 0,
            },
            TradeFeedSource {
                feed_id: "supply_chain_dive".into(),
                name: "Supply Chain Dive".into(),
                feed_url: "https://www.supplychaindive.com/feeds/news/".into(),
                region: TradeRegion::Global,
                category: TradeCategory::SupplyChain,
                topics: vec![
                    "supply chain".into(),
                    "logistics".into(),
                    "semiconductors".into(),
                ],
                enabled: true,
                last_fetched: None,
                last_item_count: 0,
            },
        ]
    }

    /// Fetch all enabled feeds.
    pub async fn fetch_all(&self) -> Vec<TradeFeedItem> {
        let mut all_items = Vec::new();
        let enabled: Vec<_> = self.sources.iter().filter(|s| s.enabled).collect();
        for source in enabled {
            match self.fetch_feed(&source.feed_url).await {
                Ok(items) => {
                    debug!(source = %source.feed_id, count = items.len(), "Trade feed fetched");
                    for item in items {
                        let keywords: Vec<String> = source
                            .topics
                            .iter()
                            .filter(|t| {
                                item.title.to_lowercase().contains(&t.to_lowercase())
                                    || item.description.to_lowercase().contains(&t.to_lowercase())
                            })
                            .cloned()
                            .collect();
                        all_items.push(TradeFeedItem {
                            title: item.title,
                            description: item.description,
                            link: item.link,
                            guid: item.guid,
                            published: item.published,
                            author: item.author,
                            source_id: source.feed_id.clone(),
                            source_name: source.name.clone(),
                            region: source.region,
                            category: source.category,
                            topics_matched: keywords,
                            relevance_score: 0.5,
                            fetched_at: Utc::now(),
                        });
                    }
                }
                Err(e) => warn!(source = %source.feed_id, error = %e, "Trade feed fetch failed"),
            }
        }
        all_items.sort_by_key(|i| std::cmp::Reverse(i.published));
        info!(total = all_items.len(), "Trade RSS aggregation complete");
        all_items
    }

    async fn fetch_feed(&self, url: &str) -> Result<Vec<RssItem>> {
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let body = resp.text().await?;
        self.parse_rss(&body)
    }

    fn parse_rss(&self, xml: &str) -> Result<Vec<RssItem>> {
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
        let mut pub_date = String::new();
        let mut author = Option::<String>::None;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag_name == "item" {
                        in_item = true;
                    } else if in_item {
                        current_tag = tag_name;
                    }
                }
                Ok(Event::End(e)) => {
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if tag_name == "item" {
                        items.push(RssItem {
                            title: title.clone(),
                            description: description.clone(),
                            link: link.clone(),
                            guid: if guid.is_empty() {
                                link.clone()
                            } else {
                                guid.clone()
                            },
                            published: chrono::Utc::now(),
                            author: author.clone(),
                        });
                        title.clear();
                        description.clear();
                        link.clear();
                        pub_date.clear();
                        author = None;
                        guid.clear();
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
                            "pubDate" => pub_date = text,
                            "author" | "dc:creator" => author = Some(text),
                            _ => {}
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
        Ok(items)
    }

    pub fn feeds_by_region(&self, region: TradeRegion) -> Vec<&TradeFeedSource> {
        self.sources.iter().filter(|s| s.region == region).collect()
    }

    pub fn feeds_by_category(&self, category: TradeCategory) -> Vec<&TradeFeedSource> {
        self.sources
            .iter()
            .filter(|s| s.category == category)
            .collect()
    }

    pub fn feed_count(&self) -> usize {
        self.sources.len()
    }
}

impl Default for TradeRssMonitor {
    fn default() -> Self {
        Self::new()
    }
}

struct RssItem {
    title: String,
    description: String,
    link: String,
    guid: String,
    published: DateTime<Utc>,
    author: Option<String>,
}

/// A feed item enriched with trade source metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeFeedItem {
    pub title: String,
    pub description: String,
    pub link: String,
    pub guid: String,
    pub published: DateTime<Utc>,
    pub author: Option<String>,
    pub source_id: String,
    pub source_name: String,
    pub region: TradeRegion,
    pub category: TradeCategory,
    pub topics_matched: Vec<String>,
    pub relevance_score: f32,
    pub fetched_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_feed_source_defaults() {
        let monitor = TradeRssMonitor::new();
        assert!(monitor.feed_count() > 0);
    }

    #[test]
    fn feeds_by_region() {
        let monitor = TradeRssMonitor::new();
        let na_feeds = monitor.feeds_by_region(TradeRegion::NorthAmerica);
        assert!(!na_feeds.is_empty());
    }

    #[test]
    fn feeds_by_category() {
        let monitor = TradeRssMonitor::new();
        let defense_feeds = monitor.feeds_by_category(TradeCategory::Defense);
        assert!(!defense_feeds.is_empty());
    }

    #[test]
    fn trade_region_as_str() {
        assert_eq!(TradeRegion::Global.as_str(), "global");
        assert_eq!(TradeRegion::MiddleEast.as_str(), "middle_east");
    }

    #[test]
    fn trade_category_as_str() {
        assert_eq!(TradeCategory::Defense.as_str(), "defense");
        assert_eq!(TradeCategory::SupplyChain.as_str(), "supply_chain");
    }
}
