//! Social intelligence (SOCMINT) aggregator.
//!
//! Coordinates cross-platform social intelligence collection by unifying
//! signals from Twitter/X, LinkedIn, industry forums, and executive
//! movement tracking into a single scan interface.
//!
//! The [`SocialIntelMonitor`] owns optional sub-monitors for each platform
//! and produces a [`SocialIntelReport`] that consolidates all collected
//! mentions, movements, and engagement signals for downstream insight
//! generation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};

use super::social_media::executive_tracking::{ExecutiveMovement, TrackedExecutive};
use super::social_media::forums::ForumMention;
use super::social_media::linkedin::LinkedInCompany;
use super::social_media::twitter::Tweet;

#[derive(Debug, Error)]
pub enum SocialIntelError {
    #[error("Social platform authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("Platform rate limited: {0}")]
    RateLimited(String),
    #[error("No sub-monitors configured")]
    NoMonitors,
}

/// A unified social intelligence mention extracted from any platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialMention {
    /// Stable identifier (platform:platform_id).
    pub mention_id: String,
    /// Source platform: twitter | linkedin | forum | executive_tracking
    pub platform: String,
    /// Author or entity name.
    pub author: Option<String>,
    /// Headline or title text.
    pub title: String,
    /// Content preview / body excerpt.
    pub content_preview: String,
    /// Canonical source URL.
    pub url: String,
    /// Engagement score (platform-normalised).
    pub engagement_score: f64,
    /// Whether the author is a high-influence account.
    pub high_influence: bool,
    /// Matched entity names (companies / people).
    pub matched_entities: Vec<String>,
    /// Matched keywords.
    pub matched_keywords: Vec<String>,
    /// Timestamp of the original post / event.
    pub published_at: Option<DateTime<Utc>>,
    /// When this mention was collected.
    pub fetched_at: DateTime<Utc>,
}

impl SocialMention {
    /// Convert a [`Tweet`] into a normalised [`SocialMention`].
    pub fn from_tweet(tweet: &Tweet) -> Self {
        Self {
            mention_id: format!("twitter:{}", tweet.tweet_id),
            platform: "twitter".to_string(),
            author: Some(tweet.author_username.clone()),
            title: tweet.text.chars().take(120).collect(),
            content_preview: tweet.text.clone(),
            url: format!("https://twitter.com/i/web/status/{}", tweet.tweet_id),
            engagement_score: tweet.engagement_score() as f64,
            high_influence: tweet.is_high_influence(),
            matched_entities: tweet.mentions.clone(),
            matched_keywords: tweet.matched_keywords.clone(),
            published_at: Some(tweet.created_at),
            fetched_at: tweet.fetched_at,
        }
    }

    /// Convert a [`ForumMention`] into a normalised [`SocialMention`].
    pub fn from_forum(mention: &ForumMention) -> Self {
        Self {
            mention_id: format!("forum:{}", mention.mention_id),
            platform: mention.source_type.as_str().to_string(),
            author: mention.author.clone(),
            title: mention.title.clone(),
            content_preview: mention.content_preview.clone(),
            url: mention.url.clone(),
            engagement_score: mention.score.unwrap_or(0) as f64,
            high_influence: mention.is_trending(),
            matched_entities: mention.matched_entities.clone(),
            matched_keywords: mention.matched_keywords.clone(),
            published_at: mention.published_at,
            fetched_at: mention.fetched_at,
        }
    }

    /// Convert an [`ExecutiveMovement`] into a normalised [`SocialMention`].
    pub fn from_movement(movement: &ExecutiveMovement) -> Self {
        Self {
            mention_id: format!("exec:{}", movement.movement_id),
            platform: "executive_tracking".to_string(),
            author: Some(movement.executive_name.clone()),
            title: movement.headline.clone(),
            content_preview: movement
                .description
                .clone()
                .unwrap_or_else(|| movement.headline.clone()),
            url: movement
                .source_url
                .clone()
                .unwrap_or_else(|| format!("exec://{}", movement.movement_id)),
            engagement_score: match movement.confidence {
                super::social_media::executive_tracking::MovementConfidence::High => 1.0,
                super::social_media::executive_tracking::MovementConfidence::Medium => 0.6,
                super::social_media::executive_tracking::MovementConfidence::Low => 0.3,
            },
            high_influence: movement.is_hire() || movement.is_departure(),
            matched_entities: vec![
                movement.executive_name.clone(),
                movement
                    .to_company
                    .clone()
                    .unwrap_or_else(|| movement.from_company.clone().unwrap_or_default()),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect(),
            matched_keywords: vec![movement.source.clone()],
            published_at: movement.announcement_date.and_then(|d| {
                d.and_hms_opt(0, 0, 0)
                    .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            }),
            fetched_at: movement.detected_at,
        }
    }
}

/// Consolidated social intelligence report from a cross-platform scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialIntelReport {
    /// All mentions collected during the scan, normalised across platforms.
    pub mentions: Vec<SocialMention>,
    /// LinkedIn company snapshots collected.
    pub companies: Vec<LinkedInCompany>,
    /// Executive movements detected.
    pub movements: Vec<ExecutiveMovement>,
    /// Scan start time.
    pub scanned_at: DateTime<Utc>,
    /// Total mentions per platform.
    pub platform_counts: HashMap<String, usize>,
    /// Aggregate high-influence mention count.
    pub high_influence_count: usize,
}

impl SocialIntelReport {
    /// Total number of mentions across all platforms.
    pub fn total_mentions(&self) -> usize {
        self.mentions.len()
    }

    /// Filter mentions to only high-influence ones.
    pub fn high_influence_mentions(&self) -> Vec<&SocialMention> {
        self.mentions.iter().filter(|m| m.high_influence).collect()
    }

    /// Get mentions mentioning a specific entity (case-insensitive).
    pub fn mentions_for_entity(&self, entity: &str) -> Vec<&SocialMention> {
        let lower = entity.to_lowercase();
        self.mentions
            .iter()
            .filter(|m| m.matched_entities.iter().any(|e| e.to_lowercase() == lower))
            .collect()
    }
}

/// Configuration for the social intelligence monitor.
#[derive(Debug, Clone, Default)]
pub struct SocialIntelConfig {
    /// Twitter monitor configuration (None = skip Twitter).
    pub twitter: Option<super::social_media::twitter::TwitterMonitorConfig>,
    /// LinkedIn monitor configuration (None = skip LinkedIn).
    pub linkedin: Option<super::social_media::linkedin::LinkedInMonitorConfig>,
    /// Tracked executives for movement detection.
    pub tracked_executives: Vec<TrackedExecutive>,
    /// Pre-collected forum mentions to fold into the report.
    pub forum_mentions: Vec<ForumMention>,
}

/// Cross-platform social intelligence aggregator.
///
/// Owns optional sub-monitors and orchestrates a unified scan that produces
/// a consolidated [`SocialIntelReport`].  Each platform is scanned
/// independently; failures on one platform do not abort the others.
pub struct SocialIntelMonitor {
    twitter: Option<super::social_media::twitter::TwitterMonitor>,
    linkedin: Option<super::social_media::linkedin::LinkedInMonitor>,
    config: SocialIntelConfig,
}

impl SocialIntelMonitor {
    /// Create a new monitor from the given configuration.
    ///
    /// Sub-monitors are constructed eagerly; if a platform's monitor fails
    /// to initialise (e.g. HTTP client error) it is logged and skipped.
    pub fn new(config: SocialIntelConfig) -> Self {
        let twitter = config.twitter.as_ref().and_then(|tc| {
            match super::social_media::twitter::TwitterMonitor::new(tc.clone()) {
                Ok(m) => Some(m),
                Err(e) => {
                    warn!(error = %e, "Failed to initialise Twitter monitor — skipping");
                    None
                }
            }
        });

        let linkedin = config.linkedin.as_ref().and_then(|lc| {
            match super::social_media::linkedin::LinkedInMonitor::new(lc.clone()) {
                Ok(m) => Some(m),
                Err(e) => {
                    warn!(error = %e, "Failed to initialise LinkedIn monitor — skipping");
                    None
                }
            }
        });

        Self {
            twitter,
            linkedin,
            config,
        }
    }

    /// Whether at least one platform monitor is active.
    pub fn has_active_monitors(&self) -> bool {
        self.twitter.is_some() || self.linkedin.is_some()
    }

    /// Run a full cross-platform scan and produce a consolidated report.
    ///
    /// Each platform is scanned independently.  Errors on individual
    /// platforms are logged and do not abort the overall scan.
    pub async fn full_scan(&self) -> SocialIntelReport {
        let scanned_at = Utc::now();
        let mut mentions = Vec::new();
        let mut companies = Vec::new();
        let mut movements = Vec::new();
        let mut platform_counts: HashMap<String, usize> = HashMap::new();

        // ── Twitter scan ──────────────────────────────────────────────
        if let Some(ref twitter) = self.twitter {
            let tweets = twitter.full_scan().await;
            let count = tweets.len();
            for tweet in &tweets {
                mentions.push(SocialMention::from_tweet(tweet));
            }
            if count > 0 {
                platform_counts.insert("twitter".to_string(), count);
            }
            info!(
                platform = "twitter",
                mentions = count,
                "SocialIntel scan complete"
            );
        }

        // ── LinkedIn scan ─────────────────────────────────────────────
        if let Some(ref linkedin) = self.linkedin {
            let li_companies = linkedin.monitor_companies().await;
            let count = li_companies.len();
            companies.extend(li_companies);
            if count > 0 {
                platform_counts.insert("linkedin".to_string(), count);
            }
            info!(
                platform = "linkedin",
                companies = count,
                "SocialIntel scan complete"
            );
        }

        // ── Forum mentions (pre-collected) ────────────────────────────
        let forum_count = self.config.forum_mentions.len();
        for forum_mention in &self.config.forum_mentions {
            mentions.push(SocialMention::from_forum(forum_mention));
        }
        if forum_count > 0 {
            platform_counts.insert("forum".to_string(), forum_count);
        }

        // ── Executive tracking (pre-loaded movements) ────────────────
        // In a production deployment, movements would be fetched from a
        // dedicated source.  Here we fold in any movements already attached
        // to tracked executives via the config.
        for exec in &self.config.tracked_executives {
            // Generate a movement signal if the executive has a previous
            // title/company differing from current — indicating a detected change.
            if let (Some(prev_title), Some(curr_title)) =
                (exec.previous_title.as_ref(), exec.current_title.as_ref())
            {
                if prev_title != curr_title {
                    let movement = ExecutiveMovement {
                        movement_id: format!("auto-{}", exec.executive_id),
                        executive_id: exec.executive_id.clone(),
                        executive_name: exec.full_name.clone(),
                        movement_type:
                            super::social_media::executive_tracking::MovementType::Promotion,
                        from_company: exec.previous_company.clone(),
                        from_title: exec.previous_title.clone(),
                        to_company: Some(exec.company.clone()),
                        to_title: exec.current_title.clone(),
                        announcement_date: None,
                        source: "profile_diff".to_string(),
                        source_url: exec.linkedin_url.clone(),
                        headline: format!(
                            "{} moved from {} to {} at {}",
                            exec.full_name, prev_title, curr_title, exec.company
                        ),
                        description: None,
                        confidence:
                            super::social_media::executive_tracking::MovementConfidence::Medium,
                        detected_at: exec.last_updated,
                    };
                    mentions.push(SocialMention::from_movement(&movement));
                    movements.push(movement);
                }
            }
        }
        let exec_count = movements.len();
        if exec_count > 0 {
            platform_counts.insert("executive_tracking".to_string(), exec_count);
        }

        let high_influence_count = mentions.iter().filter(|m| m.high_influence).count();

        info!(
            total_mentions = mentions.len(),
            high_influence = high_influence_count,
            platforms = platform_counts.len(),
            "SocialIntel full scan complete"
        );

        SocialIntelReport {
            mentions,
            companies,
            movements,
            scanned_at,
            platform_counts,
            high_influence_count,
        }
    }
}

impl Default for SocialIntelMonitor {
    fn default() -> Self {
        Self::new(SocialIntelConfig::default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn monitor_with_no_config_has_no_active_monitors() {
        let monitor = SocialIntelMonitor::default();
        assert!(!monitor.has_active_monitors());
    }

    #[test]
    fn monitor_with_twitter_config_has_active_monitor() {
        let config = SocialIntelConfig {
            twitter: Some(super::super::social_media::twitter::TwitterMonitorConfig::default()),
            ..Default::default()
        };
        let monitor = SocialIntelMonitor::new(config);
        assert!(monitor.has_active_monitors());
    }

    #[tokio::test]
    async fn full_scan_with_no_monitors_returns_empty_report() {
        let monitor = SocialIntelMonitor::default();
        let report = monitor.full_scan().await;
        assert_eq!(report.total_mentions(), 0);
        assert!(report.high_influence_mentions().is_empty());
    }

    #[test]
    fn social_mention_from_tweet_normalises_fields() {
        let tweet = super::super::social_media::twitter::Tweet {
            tweet_id: "123".to_string(),
            author_username: "analyst".to_string(),
            author_id: "456".to_string(),
            text: "Breaking news about supply chain".to_string(),
            created_at: Utc::now(),
            like_count: Some(500),
            retweet_count: Some(200),
            reply_count: Some(50),
            quote_count: Some(10),
            language: Some("en".to_string()),
            is_reply: false,
            is_retweet: false,
            is_quote: false,
            hashtags: vec![],
            mentions: vec!["Foxconn".to_string()],
            urls: vec![],
            matched_keywords: vec!["supply chain".to_string()],
            fetched_at: Utc::now(),
        };
        let mention = SocialMention::from_tweet(&tweet);
        assert_eq!(mention.platform, "twitter");
        assert_eq!(mention.author.as_deref(), Some("analyst"));
        assert!(mention.url.contains("123"));
        assert!(mention.engagement_score > 0.0);
        assert!(mention.matched_entities.contains(&"Foxconn".to_string()));
    }

    #[test]
    fn report_filters_by_entity() {
        let now = Utc::now();
        let mentions = vec![
            SocialMention {
                mention_id: "1".to_string(),
                platform: "twitter".to_string(),
                author: None,
                title: "t1".to_string(),
                content_preview: "c1".to_string(),
                url: "u1".to_string(),
                engagement_score: 10.0,
                high_influence: false,
                matched_entities: vec!["Foxconn".to_string()],
                matched_keywords: vec![],
                published_at: Some(now),
                fetched_at: now,
            },
            SocialMention {
                mention_id: "2".to_string(),
                platform: "twitter".to_string(),
                author: None,
                title: "t2".to_string(),
                content_preview: "c2".to_string(),
                url: "u2".to_string(),
                engagement_score: 10.0,
                high_influence: false,
                matched_entities: vec!["TSMC".to_string()],
                matched_keywords: vec![],
                published_at: Some(now),
                fetched_at: now,
            },
        ];
        let report = SocialIntelReport {
            mentions,
            companies: vec![],
            movements: vec![],
            scanned_at: now,
            platform_counts: HashMap::new(),
            high_influence_count: 0,
        };
        assert_eq!(report.mentions_for_entity("foxconn").len(), 1);
        assert_eq!(report.mentions_for_entity("tsmc").len(), 1);
        assert_eq!(report.mentions_for_entity("samsung").len(), 0);
    }
}
