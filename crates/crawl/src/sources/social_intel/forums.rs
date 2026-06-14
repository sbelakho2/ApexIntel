//! Industry Forum Presence Detection Module
//!
//! Detects and monitors tracked entities' presence on industry forums and
//! community platforms (e.g. Reddit subreddits, specialized defense forums,
//! Stack Overflow communities, Hacker News).
//!
//! # Detections
//! - Account registration on monitored forums
//! - Post/comment activity by tracked employees
//! - Topic discussion related to tracked entities
//! - Sentiment signals in industry discussions

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};

/// A forum activity record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumActivity {
    /// Forum platform name.
    pub platform: String,
    /// Forum/section identifier.
    pub forum_id: String,
    /// Post/comment identifier.
    pub activity_id: String,
    /// Activity type.
    pub activity_type: ForumActivityType,
    /// Author username.
    pub author: String,
    /// Author profile URL.
    pub profile_url: String,
    /// Activity title/text.
    pub title: String,
    /// Content body (stripped).
    pub body: String,
    /// Keywords matched.
    pub matched_keywords: Vec<String>,
    /// Publication timestamp.
    pub published_at: Option<DateTime<Utc>>,
    /// Like/upvote count.
    pub upvote_count: u64,
    /// Reply count.
    pub reply_count: u64,
    /// Direct URL.
    pub activity_url: String,
    /// Sentiment label.
    pub sentiment: SentimentLabel,
    /// Intelligence relevance score [0, 1].
    pub relevance_score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForumActivityType {
    Post,
    Comment,
    Thread,
    Profile,
    PrivateMessage,
}

impl ForumActivityType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Comment => "comment",
            Self::Thread => "thread",
            Self::Profile => "profile",
            Self::PrivateMessage => "private_message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentimentLabel {
    Positive,
    Neutral,
    Negative,
    Mixed,
}

impl SentimentLabel {
    pub fn from_score(score: f32) -> Self {
        if score > 0.3 {
            Self::Positive
        } else if score < -0.3 {
            Self::Negative
        } else {
            Self::Neutral
        }
    }
}

/// A forum being monitored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredForum {
    /// Unique forum identifier.
    pub forum_id: String,
    /// Human-readable name.
    pub name: String,
    /// Base URL.
    pub base_url: String,
    /// RSS feed URL (if available).
    pub rss_url: Option<String>,
    /// Platform type.
    pub platform_type: ForumPlatform,
    /// Keywords relevant to this forum.
    pub target_keywords: Vec<String>,
    /// Username patterns associated with tracked entities.
    pub tracked_usernames: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForumPlatform {
    Reddit,
    Discourse,
    CustomForum,
    StackOverflow,
    HackerNews,
    Discord,
}

impl ForumPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reddit => "reddit",
            Self::Discourse => "discourse",
            Self::CustomForum => "custom_forum",
            Self::StackOverflow => "stackoverflow",
            Self::HackerNews => "hackernews",
            Self::Discord => "discord",
        }
    }
}

/// Forum intelligence configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumMonitorConfig {
    /// Forums to monitor.
    pub forums: Vec<MonitoredForum>,
    /// Global keywords across all forums.
    pub global_keywords: Vec<String>,
    /// Maximum activities to return per forum.
    pub max_activities: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for ForumMonitorConfig {
    fn default() -> Self {
        let mut forums = Vec::new();

        // Default monitored forums
        forums.push(MonitoredForum {
            forum_id: "reddit-defense".to_string(),
            name: "r/defense".to_string(),
            base_url: "https://www.reddit.com/r/defense/".to_string(),
            rss_url: Some("https://www.reddit.com/r/defense/.rss".to_string()),
            platform_type: ForumPlatform::Reddit,
            target_keywords: vec!["defense".to_string(), "military".to_string()],
            tracked_usernames: Vec::new(),
        });

        forums.push(MonitoredForum {
            forum_id: "reddit-worldnews".to_string(),
            name: "r/worldnews".to_string(),
            base_url: "https://www.reddit.com/r/worldnews/".to_string(),
            rss_url: Some("https://www.reddit.com/r/worldnews/.rss".to_string()),
            platform_type: ForumPlatform::Reddit,
            target_keywords: vec!["sanctions".to_string(), "export control".to_string()],
            tracked_usernames: Vec::new(),
        });

        Self {
            forums,
            global_keywords: Vec::new(),
            max_activities: 50,
            timeout_secs: 30,
        }
    }
}

impl ForumMonitorConfig {
    /// Add a forum to monitor.
    pub fn add_forum(mut self, forum: MonitoredForum) -> Self {
        self.forums.push(forum);
        self
    }

    /// Add global keywords.
    pub fn add_keywords(mut self, kws: impl IntoIterator<Item = String>) -> Self {
        self.global_keywords.extend(kws);
        self
    }
}

/// Forum intelligence monitor.
#[derive(Debug, Clone)]
pub struct ForumMonitor {
    client: Client,
    config: ForumMonitorConfig,
}

impl ForumMonitor {
    /// Create with explicit configuration.
    pub fn new(config: ForumMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Forum Monitor")
            .build()
            .context("building Forum monitor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Scan all configured forums and return matching activities.
    pub async fn scan(&self) -> Vec<ForumActivity> {
        let mut all_activities = Vec::new();

        for forum in &self.config.forums {
            match self.scan_forum(forum).await {
                Ok(activities) => {
                    debug!(forum = %forum.name, count = activities.len(), "Forum scan complete");
                    all_activities.extend(activities);
                }
                Err(e) => {
                    warn!(forum = %forum.name, error = %e, "Forum scan failed — skipping");
                }
            }
        }

        all_activities.sort_by(|a, b| {
            b.published_at.cmp(&a.published_at)
        });

        info!(total = all_activities.len(), "Forum monitoring scan complete");
        all_activities
    }

    /// Scan a single forum.
    async fn scan_forum(&self, forum: &MonitoredForum) -> Result<Vec<ForumActivity>> {
        match forum.platform_type {
            ForumPlatform::Reddit => self.scan_reddit(forum).await,
            ForumPlatform::Discourse => self.scan_discourse(forum).await,
            ForumPlatform::StackOverflow => self.scan_stackoverflow(forum).await,
            ForumPlatform::HackerNews => self.scan_hackernews(forum).await,
            _ => {
                info!(forum = %forum.name, "Platform type not yet implemented");
                Ok(Vec::new())
            }
        }
    }

    /// Scan a Reddit subreddit.
    async fn scan_reddit(&self, forum: &MonitoredForum) -> Result<Vec<ForumActivity>> {
        let rss_url = forum
            .rss_url
            .as_ref()
            .context("Reddit forum requires RSS URL")?;

        let resp = self
            .client
            .get(rss_url)
            .send()
            .await
            .context("Reddit RSS fetch")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Reddit RSS returned non-success");
            return Ok(Vec::new());
        }

        let body = resp.text().await.context("read Reddit RSS")?;
        let items = crate::rss::parse_feed(&body).unwrap_or_default();

        let keywords: Vec<String> = forum
            .target_keywords
            .iter()
            .chain(self.config.global_keywords.iter())
            .cloned()
            .collect();

        let activities = items
            .into_iter()
            .filter(|item| {
                let combined = format!("{} {}", item.title, item.description).to_lowercase();
                keywords.iter().any(|kw| combined.contains(&kw.to_lowercase()))
            })
            .map(|item| {
                let body = crate::social::strip_urls(&item.description);
                let matched = keywords
                    .iter()
                    .filter(|kw| body.to_lowercase().contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                ForumActivity {
                    platform: "reddit".to_string(),
                    forum_id: forum.forum_id.clone(),
                    activity_id: item.guid.clone(),
                    activity_type: ForumActivityType::Post,
                    author: item.author.clone().unwrap_or_else(|| "unknown".to_string()),
                    profile_url: String::new(),
                    title: item.title.clone(),
                    body,
                    matched_keywords: matched,
                    published_at: item.published,
                    upvote_count: 0,
                    reply_count: 0,
                    activity_url: item.link.clone(),
                    sentiment: SentimentLabel::Neutral,
                    relevance_score: 0.5,
                }
            })
            .collect();

        Ok(activities)
    }

    /// Scan a Discourse forum.
    async fn scan_discourse(&self, forum: &MonitoredForum) -> Result<Vec<ForumActivity>> {
        let search_url = format!("{}/search.json", forum.base_url.trim_end_matches('/'));

        let keywords = forum
            .target_keywords
            .iter()
            .chain(self.config.global_keywords.iter())
            .join(" ");
        let params = [("q", keywords.as_str())];

        let resp = self
            .client
            .get(&search_url)
            .query(&params)
            .send()
            .await
            .context("Discourse search request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Discourse search returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct DiscourseSearch {
            posts: Option<Vec<DiscoursePost>>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
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

        let search: DiscourseSearch = resp.json().await.unwrap_or_default();
        let posts = search.posts.unwrap_or_default();

        let activities = posts
            .into_iter()
            .filter_map(|p| {
                let title = p.title?;
                let body = p.cooked?;
                let combined = format!("{} {}", title, body).to_lowercase();
                let matched: Vec<String> = forum
                    .target_keywords
                    .iter()
                    .chain(self.config.global_keywords.iter())
                    .filter(|kw| combined.contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                if matched.is_empty() {
                    return None;
                }

                let published_at = p.created_at.as_ref().and_then(|s| {
                    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&Utc))
                });

                Some(ForumActivity {
                    platform: "discourse".to_string(),
                    forum_id: forum.forum_id.clone(),
                    activity_id: p.id.map(|i| i.to_string()).unwrap_or_default(),
                    activity_type: ForumActivityType::Post,
                    author: p.username.unwrap_or_else(|| "unknown".to_string()),
                    profile_url: format!("{}/u/{}", forum.base_url, p.username.unwrap_or_default()),
                    title,
                    body,
                    matched_keywords: matched,
                    published_at,
                    upvote_count: p.like_count.unwrap_or(0),
                    reply_count: p.reply_count.unwrap_or(0),
                    activity_url: p.topic_id.map(|id| {
                        format!("{}/t/{}/{}", forum.base_url, p.topic_slug.unwrap_or_default(), id)
                    }).unwrap_or_else(|| forum.base_url.clone()),
                    sentiment: SentimentLabel::Neutral,
                    relevance_score: 0.5,
                })
            })
            .collect();

        Ok(activities)
    }

    /// Scan Stack Overflow for tagged questions.
    async fn scan_stackoverflow(&self, forum: &MonitoredForum) -> Result<Vec<ForumActivity>> {
        let tags = forum.target_keywords.join(";");
        let url = format!(
            "https://api.stackexchange.com/2.3/questions?order=desc&sort=creation&tagged={}&site=stackoverflow",
            urlencoding::encode(&tags)
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Stack Overflow API request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Stack Overflow API returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct SoResponse {
            items: Option<Vec<SoQuestion>>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct SoQuestion {
            question_id: Option<i64>,
            title: Option<String>,
            body_markdown: Option<String>,
            owner: Option<SoOwner>,
            creation_date: Option<i64>,
            score: Option<i64>,
            answer_count: Option<i64>,
            tags: Option<Vec<String>>,
            link: Option<String>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct SoOwner {
            display_name: Option<String>,
            link: Option<String>,
        }

        let so_resp: SoResponse = resp.json().await.unwrap_or_default();
        let questions = so_resp.items.unwrap_or_default();

        let activities = questions
            .into_iter()
            .filter_map(|q| {
                let title = q.title?;
                let body = q.body_markdown?;
                let combined = format!("{} {}", title, body).to_lowercase();
                let matched: Vec<String> = forum
                    .target_keywords
                    .iter()
                    .chain(self.config.global_keywords.iter())
                    .filter(|kw| combined.contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                if matched.is_empty() {
                    return None;
                }

                let published_at = q.creation_date
                    .and_then(|ts| DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc));

                Some(ForumActivity {
                    platform: "stackoverflow".to_string(),
                    forum_id: forum.forum_id.clone(),
                    activity_id: q.question_id.map(|i| i.to_string()).unwrap_or_default(),
                    activity_type: ForumActivityType::Question,
                    author: q.owner.as_ref().and_then(|o| o.display_name.clone()).unwrap_or_else(|| "unknown".to_string()),
                    profile_url: q.owner.and_then(|o| o.link).unwrap_or_default(),
                    title,
                    body,
                    matched_keywords: matched,
                    published_at,
                    upvote_count: q.score.unwrap_or(0) as u64,
                    reply_count: q.answer_count.unwrap_or(0) as u64,
                    activity_url: q.link.unwrap_or_else(|| forum.base_url.clone()),
                    sentiment: SentimentLabel::Neutral,
                    relevance_score: 0.5,
                })
            })
            .collect();

        Ok(activities)
    }

    /// Scan Hacker News for keyword mentions via their API.
    async fn scan_hackernews(&self, forum: &MonitoredForum) -> Result<Vec<ForumActivity>> {
        // HN Algolia API for keyword search
        let keywords = forum.target_keywords.join(" ");
        let url = format!(
            "https://hn.algolia.com/api/v1/search?query={}&tags=story",
            urlencoding::encode(&keywords)
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Hacker News API request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "HN API returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct HnResponse {
            hits: Option<Vec<HnHit>>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct HnHit {
            object_id: Option<String>,
            title: Option<String>,
            author: Option<String>,
            created_at: Option<String>,
            points: Option<i64>,
            num_comments: Option<i64>,
            url: Option<String>,
            _tags: Option<Vec<String>>,
        }

        let hn_resp: HnResponse = resp.json().await.unwrap_or_default();
        let hits = hn_resp.hits.unwrap_or_default();

        let activities = hits
            .into_iter()
            .filter_map(|hit| {
                let title = hit.title?;
                let combined = title.to_lowercase();
                let matched: Vec<String> = forum
                    .target_keywords
                    .iter()
                    .chain(self.config.global_keywords.iter())
                    .filter(|kw| combined.contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                if matched.is_empty() {
                    return None;
                }

                let published_at = hit.created_at.as_ref().and_then(|s| {
                    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&Utc))
                });

                Some(ForumActivity {
                    platform: "hackernews".to_string(),
                    forum_id: forum.forum_id.clone(),
                    activity_id: hit.object_id.unwrap_or_default(),
                    activity_type: ForumActivityType::Post,
                    author: hit.author.unwrap_or_else(|| "unknown".to_string()),
                    profile_url: format!("https://news.ycombinator.com/user?id={}", hit.author.unwrap_or_default()),
                    title,
                    body: String::new(),
                    matched_keywords: matched,
                    published_at,
                    upvote_count: hit.points.unwrap_or(0) as u64,
                    reply_count: hit.num_comments.unwrap_or(0) as u64,
                    activity_url: hit.url.unwrap_or_else(|| "https://news.ycombinator.com".to_string()),
                    sentiment: SentimentLabel::Neutral,
                    relevance_score: 0.5,
                })
            })
            .collect();

        Ok(activities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forum_monitor_config_defaults() {
        let cfg = ForumMonitorConfig::default();
        assert!(!cfg.forums.is_empty());
        assert_eq!(cfg.max_activities, 50);
    }

    #[test]
    fn sentiment_from_score() {
        assert_eq!(SentimentLabel::from_score(0.8), SentimentLabel::Positive);
        assert_eq!(SentimentLabel::from_score(-0.5), SentimentLabel::Negative);
        assert_eq!(SentimentLabel::from_score(0.1), SentimentLabel::Neutral);
    }

    #[test]
    fn forum_monitor_constructs() {
        let result = ForumMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn forum_activity_type_as_str() {
        assert_eq!(ForumActivityType::Post.as_str(), "post");
        assert_eq!(ForumActivityType::Comment.as_str(), "comment");
    }
}
