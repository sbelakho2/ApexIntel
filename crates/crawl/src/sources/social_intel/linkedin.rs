//! LinkedIn Intelligence Module
//!
//! Provides structured data from LinkedIn public pages and company profiles:
//! - Company overview (industry, size, headquarters)
//! - Employee headcount tracking (growth/decline signals)
//! - Recent hires and departures (hiring velocity)
//! - Job posting analysis (skill requirements, geography expansion)
//!
//! # Authentication
//! Set `LINKEDIN_USERNAME` and `LINKEDIN_PASSWORD` environment variables
//! for authenticated scraping. Falls back to public page scraping when
//! credentials are absent (limited data).

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A parsed LinkedIn company profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedinCompany {
    /// Company LinkedIn slug / vanity URL path.
    pub slug: String,
    /// Full company name.
    pub name: String,
    /// Industry classification.
    pub industry: Option<String>,
    /// Company size label (e.g. "201-500 employees").
    pub size: Option<String>,
    /// Numeric min employee count if parseable.
    pub employee_count_min: Option<u32>,
    /// Headquarters location.
    pub headquarters: Option<String>,
    /// Founding year (if available).
    pub founded: Option<u32>,
    /// Specialties (comma-separated tags).
    pub specialties: Vec<String>,
    /// LinkedIn company page URL.
    pub profile_url: String,
    /// Description / about text.
    pub description: Option<String>,
    /// Number of followers (if available from public page).
    pub follower_count: Option<u64>,
    /// Timestamp when this data was fetched.
    pub fetched_at: DateTime<Utc>,
}

/// A LinkedIn personnel movement event (hire or departure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedinPersonnelEvent {
    /// Event type.
    pub event_type: PersonnelEventType,
    /// Person's name.
    pub person_name: String,
    /// Person's LinkedIn profile URL.
    pub profile_url: String,
    /// Job title at time of event.
    pub title: String,
    /// Company slug (before or after move).
    pub company_slug: String,
    /// Date of the event (hire date or departure date).
    pub event_date: Option<NaiveDate>,
    /// When this was detected.
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersonnelEventType {
    Hire,
    Promotion,
    Departure,
    Transfer,
}

impl PersonnelEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hire => "hire",
            Self::Promotion => "promotion",
            Self::Departure => "departure",
            Self::Transfer => "transfer",
        }
    }
}

/// LinkedIn intelligence configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedinMonitorConfig {
    /// LinkedIn username for authenticated scraping.
    pub username: Option<String>,
    /// LinkedIn password.
    pub password: Option<String>,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Rate limit interval between requests in milliseconds.
    pub rate_limit_ms: u64,
    /// Company slugs to monitor.
    pub company_slugs: Vec<String>,
    /// Keywords to detect in job postings.
    pub job_keywords: Vec<String>,
}

impl Default for LinkedinMonitorConfig {
    fn default() -> Self {
        Self {
            username: None,
            password: None,
            timeout_secs: 30,
            rate_limit_ms: 5000,
            company_slugs: Vec::new(),
            job_keywords: vec![
                "new hire".to_string(),
                "joins".to_string(),
                "promoted".to_string(),
                "leaves".to_string(),
                "departed".to_string(),
            ],
        }
    }
}

/// LinkedIn intelligence monitor.
#[derive(Debug, Clone)]
pub struct LinkedinMonitor {
    client: Client,
    config: LinkedinMonitorConfig,
}

impl LinkedinMonitor {
    /// Create from environment variables (`LINKEDIN_USERNAME`, `LINKEDIN_PASSWORD`).
    pub fn from_env() -> Result<Self> {
        let config = LinkedinMonitorConfig {
            username: std::env::var("LINKEDIN_USERNAME").ok(),
            password: std::env::var("LINKEDIN_PASSWORD").ok(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Create with explicit configuration.
    pub fn new(config: LinkedinMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .context("building LinkedIn HTTP client")?;

        Ok(Self { client, config })
    }

    /// Fetch a company profile by slug (e.g. `"elbit-systems"`).
    ///
    /// Public page scraping is used when no credentials are configured.
    /// Authenticated scraping provides richer data.
    pub async fn fetch_company(&self, slug: &str) -> Result<LinkedinCompany> {
        info!(slug = %slug, "Fetching LinkedIn company profile");
        let url = format!("https://www.linkedin.com/company/{}/", slug);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("LinkedIn company fetch")?;

        if !resp.status().is_success() {
            anyhow::bail!("LinkedIn returned {} for /company/{}", resp.status(), slug);
        }

        let body = resp.text().await.context("read LinkedIn response body")?;
        self.parse_company_profile(slug, &body)
    }

    /// Fetch multiple company profiles.
    pub async fn fetch_companies(&self, slugs: &[&str]) -> Vec<Result<LinkedinCompany>> {
        let mut results = Vec::with_capacity(slugs.len());
        for slug in slugs {
            results.push(self.fetch_company(slug).await);
            // Respect rate limit
            tokio::time::sleep(Duration::from_millis(self.config.rate_limit_ms)).await;
        }
        results
    }

    /// Parse a company profile HTML page into a `LinkedinCompany` struct.
    fn parse_company_profile(&self, slug: &str, html: &str) -> Result<LinkedinCompany> {
        use regex::Regex;

        let name_re = Regex::new(r#"og:title"[^>]*content="([^"]+)""#)
            .context("name regex")?;
        let industry_re = Regex::new(r#"industry"[^>]*>([^<]+)<"#)
            .context("industry regex")?;
        let size_re = Regex::new(r#"(\d+(?:,\d{3})*(?:\s*-\s*\d+(?:,\d{3})*)?)\s*employees?"#)
            .context("size regex")?;
        let hq_re = Regex::new(r#"headquarters"[^>]*>([^<]+)<"#)
            .context("hq regex")?;

        let name = name_re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| html_escape::decode_html_entities(&m.as_str().to_string()))
            .unwrap_or_else(|| slug.to_string());

        let industry = industry_re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let size = size_re
            .captures(html)
            .and_then(|c| c.get(0))
            .map(|m| m.as_str().trim().to_string());

        let employee_count_min = size.as_ref().and_then(|s| {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            digits.parse::<u32>().ok()
        });

        let headquarters = hq_re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        Ok(LinkedinCompany {
            slug: slug.to_string(),
            name,
            industry,
            size,
            employee_count_min,
            headquarters,
            founded: None,
            specialties: Vec::new(),
            profile_url: format!("https://www.linkedin.com/company/{}/", slug),
            description: None,
            follower_count: None,
            fetched_at: Utc::now(),
        })
    }

    /// Extract personnel movement events from a company's activity feed.
    ///
    /// Looks for "joined", "hired", "promoted", "departed" patterns.
    pub async fn detect_personnel_events(&self, slug: &str) -> Result<Vec<LinkedinPersonnelEvent>> {
        let url = format!("https://www.linkedin.com/company/{}/posts/", slug);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("LinkedIn posts fetch")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), slug = %slug, "LinkedIn posts page not accessible");
            return Ok(Vec::new());
        }

        let body = resp.text().await.context("read posts page")?;
        let events = self.parse_personnel_events(slug, &body);

        debug!(slug = %slug, count = events.len(), "Personnel events detected");
        Ok(events)
    }

    fn parse_personnel_events(&self, company_slug: &str, html: &str) -> Vec<LinkedinPersonnelEvent> {
        use regex::Regex;

        let mut events = Vec::new();
        let now = Utc::now();

        for pattern in &["joined the team", "new hire", "is joining", "has joined"] {
            let re = Regex::new(&format!(r"(?i){}", pattern)).ok()?;
            for cap in re.captures_iter(html) {
                if let Some(text) = cap.get(0) {
                    events.push(LinkedinPersonnelEvent {
                        event_type: PersonnelEventType::Hire,
                        person_name: "Detected via text".to_string(),
                        profile_url: String::new(),
                        title: text.as_str().to_string(),
                        company_slug: company_slug.to_string(),
                        event_date: None,
                        detected_at: now,
                    });
                }
            }
        }

        for pattern in &["has left", "departed", "no longer with"] {
            let re = Regex::new(&format!(r"(?i){}", pattern)).ok()?;
            for cap in re.captures_iter(html) {
                if let Some(text) = cap.get(0) {
                    events.push(LinkedinPersonnelEvent {
                        event_type: PersonnelEventType::Departure,
                        person_name: "Detected via text".to_string(),
                        profile_url: String::new(),
                        title: text.as_str().to_string(),
                        company_slug: company_slug.to_string(),
                        event_date: None,
                        detected_at: now,
                    });
                }
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linkedin_monitor_config_defaults() {
        let cfg = LinkedinMonitorConfig::default();
        assert!(cfg.company_slugs.is_empty());
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn linkedin_monitor_constructs() {
        let result = LinkedinMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn personnel_event_type_ordering() {
        assert_ne!(PersonnelEventType::Hire, PersonnelEventType::Departure);
        assert_eq!(PersonnelEventType::Promotion.as_str(), "promotion");
    }

    #[test]
    fn linkedin_company_debug() {
        let company = LinkedinCompany {
            slug: "test-slug".to_string(),
            name: "Test Company".to_string(),
            industry: Some("Defense".to_string()),
            size: Some("10,001+ employees".to_string()),
            employee_count_min: Some(10001),
            headquarters: Some("Tel Aviv, Israel".to_string()),
            founded: Some(1987),
            specialties: vec!["Aerospace".to_string(), "Electronics".to_string()],
            profile_url: "https://linkedin.com/company/test-slug".to_string(),
            description: None,
            follower_count: Some(50000),
            fetched_at: Utc::now(),
        };
        assert_eq!(company.slug, "test-slug");
        assert_eq!(company.employee_count_min, Some(10001));
    }
}
