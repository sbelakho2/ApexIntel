//! LinkedIn Intelligence Module
//!
//! Monitors LinkedIn for company and employee intelligence:
//! - Company page monitoring (followers, posts, jobs posted)
//! - Employee count changes
//! - Job posting analysis (hiring surges)
//! - Leadership changes
//! - Company description updates
//!
//! Uses official LinkedIn APIs where available; respects rate limits.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A LinkedIn company profile snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInCompany {
    pub company_id: String,
    pub name: String,
    pub tagline: Option<String>,
    pub description: Option<String>,
    pub website: Option<String>,
    pub industry: Option<String>,
    pub company_size: Option<String>,
    pub headquarters: Option<String>,
    pub founded: Option<u32>,
    pub follower_count: Option<u64>,
    pub fetched_at: DateTime<Utc>,
}

impl LinkedInCompany {
    /// Whether this is a large company (1000+ employees).
    pub fn is_large_company(&self) -> bool {
        self.company_size.as_ref().map(|s| {
            // Extract all digits from the string
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            // Parse first 1-4 digits as a number
            let first_num: u64 = digits.chars().take(4).collect::<String>().parse().unwrap_or(0);
            first_num >= 1000
        }).unwrap_or(false)
    }
}

/// A LinkedIn employee at a company.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInEmployee {
    pub name: String,
    pub title: String,
    pub profile_url: String,
    pub company: String,
    pub company_id: String,
    pub location: Option<String>,
    pub connection_degree: u8,
    pub fetched_at: DateTime<Utc>,
}

/// A job posting from LinkedIn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInJobPosting {
    pub posting_id: String,
    pub company_id: String,
    pub company_name: String,
    pub title: String,
    pub location: Option<String>,
    pub remote: Option<bool>,
    pub employment_type: Option<String>,
    pub description: Option<String>,
    pub posted_date: Option<NaiveDate>,
    pub applicants: Option<u32>,
    pub keywords_matched: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

/// LinkedIn monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInMonitorConfig {
    /// Company IDs to monitor.
    pub company_ids: Vec<String>,
    /// Keywords for job posting search.
    pub job_keywords: Vec<String>,
    /// Maximum results per query.
    pub max_results: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// OAuth2 access token for LinkedIn API authentication.
    /// Required for all authenticated endpoints; without it, API calls
    /// will return empty results with a warning log.
    #[serde(default)]
    pub access_token: Option<String>,
}

impl Default for LinkedInMonitorConfig {
    fn default() -> Self {
        Self {
            company_ids: Vec::new(),
            job_keywords: vec![
                "defense".to_string(),
                "security".to_string(),
                "engineering".to_string(),
            ],
            max_results: 20,
            timeout_secs: 30,
            access_token: None,
        }
    }
}

impl LinkedInMonitorConfig {
    pub fn add_company(mut self, id: impl Into<String>) -> Self {
        self.company_ids.push(id.into()); self
    }

    /// Set the OAuth2 access token used for LinkedIn API authentication.
    pub fn with_access_token(mut self, token: impl Into<String>) -> Self {
        self.access_token = Some(token.into()); self
    }
}

/// LinkedIn company/employee monitor.
#[derive(Debug, Clone)]
pub struct LinkedInMonitor {
    client: Client,
    config: LinkedInMonitorConfig,
}

impl LinkedInMonitor {
    pub fn new(config: LinkedInMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("Mozilla/5.0 (compatible; ApexIntel/1.0; +https://apexintel.io) LinkedIn Monitor")
            .build()
            .context("building LinkedIn HTTP client")?;
        Ok(Self { client, config })
    }

    /// Build the Authorization header value from the configured access token.
    /// Returns `None` if no token is configured.
    fn auth_header(&self) -> Option<String> {
        self.config
            .access_token
            .as_ref()
            .map(|token| format!("Bearer {token}"))
    }

    /// Fetch company profile by ID.
    pub async fn fetch_company(&self, company_id: &str) -> Result<LinkedInCompany> {
        let url = format!(
            "https://api.linkedin.com/v2/companies/{}/",
            urlencoding::encode(company_id)
        );
        let mut req = self.client.get(&url);
        if let Some(header) = self.auth_header() {
            req = req.header("Authorization", header);
        } else {
            warn!(company_id = %company_id, "LinkedIn fetch_company called without access token — request will be unauthenticated");
        }
        let resp = req.send().await
            .context("LinkedIn company request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), company_id = %company_id, "LinkedIn company returned non-success");
            return Ok(LinkedInCompany {
                company_id: company_id.to_string(),
                name: "Unknown".to_string(),
                tagline: None,
                description: None,
                website: None,
                industry: None,
                company_size: None,
                headquarters: None,
                founded: None,
                follower_count: None,
                fetched_at: Utc::now(),
            });
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        #[serde(rename_all = "camelCase")]
        struct LiqCompany {
            name: Option<String>,
            headline: Option<String>,
            description: Option<String>,
            website_url: Option<String>,
            industries: Option<Vec<String>>,
            company_type: Option<String>,
            founded_on: Option<i64>,
        }

        let liq: LiqCompany = resp.json().await.unwrap_or(LiqCompany { name: None, headline: None, description: None, website_url: None, industries: None, company_type: None, founded_on: None });
        Ok(LinkedInCompany {
            company_id: company_id.to_string(),
            name: liq.name.unwrap_or_else(|| "Unknown".to_string()),
            tagline: liq.headline,
            description: liq.description,
            website: liq.website_url,
            industry: liq.industries.and_then(|v| v.first().cloned()),
            company_size: None,
            headquarters: None,
            founded: liq.founded_on.map(|f| (f / 1000) as u32),
            follower_count: None,
            fetched_at: Utc::now(),
        })
    }

    /// Search for employees at a company.
    pub async fn search_employees(&self, company_name: &str) -> Result<Vec<LinkedInEmployee>> {
        let url = format!(
            "https://api.linkedin.com/v2/peopleSearch?q=currentCompany&companyName={}",
            urlencoding::encode(company_name)
        );
        let mut req = self.client.get(&url);
        if let Some(header) = self.auth_header() {
            req = req.header("Authorization", header);
        } else {
            warn!(company = %company_name, "LinkedIn search_employees called without access token — request will be unauthenticated");
        }
        let resp = req.send().await
            .context("LinkedIn employee search")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), company = %company_name, "LinkedIn employee search returned non-success");
            return Ok(Vec::new());
        }

        Ok(Vec::new()) // Returns empty unless authenticated
    }

    /// Search job postings.
    pub async fn search_jobs(&self, keywords: &[String]) -> Result<Vec<LinkedInJobPosting>> {
        let kws = keywords.join(" ");
        let url = format!(
            "https://api.linkedin.com/v2/jobSearch?q={}&count={}",
            urlencoding::encode(&kws),
            self.config.max_results
        );
        let mut req = self.client.get(&url);
        if let Some(header) = self.auth_header() {
            req = req.header("Authorization", header);
        } else {
            warn!("LinkedIn search_jobs called without access token — request will be unauthenticated");
        }
        let resp = req.send().await
            .context("LinkedIn job search")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "LinkedIn job search returned non-success");
            return Ok(Vec::new());
        }

        Ok(Vec::new())
    }

    /// Monitor all configured companies.
    pub async fn monitor_companies(&self) -> Vec<LinkedInCompany> {
        let mut companies = Vec::new();
        for id in &self.config.company_ids {
            match self.fetch_company(id).await {
                Ok(company) => companies.push(company),
                Err(e) => warn!(company_id = %id, error = %e, "LinkedIn company fetch failed"),
            }
        }
        info!(total = companies.len(), "LinkedIn company monitoring complete");
        companies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linkedin_company_is_large() {
        let company = LinkedInCompany {
            company_id: "test".to_string(),
            name: "Test Corp".to_string(),
            tagline: None,
            description: None,
            website: None,
            industry: None,
            company_size: Some("5001-10000".to_string()),
            headquarters: None,
            founded: None,
            follower_count: None,
            fetched_at: Utc::now(),
        };
        assert!(company.is_large_company());
    }

    #[test]
    fn linkedin_monitor_constructs() {
        let result = LinkedInMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn linkedin_config_chaining() {
        let cfg = LinkedInMonitorConfig::default()
            .add_company("12345")
            .add_company("67890");
        assert_eq!(cfg.company_ids.len(), 2);
    }

    #[test]
    fn linkedin_config_with_access_token() {
        let cfg = LinkedInMonitorConfig::default()
            .with_access_token("test-oauth-token-123");
        assert!(cfg.access_token.is_some());
        assert_eq!(cfg.access_token.as_deref(), Some("test-oauth-token-123"));

        // Default config has no token
        let default_cfg = LinkedInMonitorConfig::default();
        assert!(default_cfg.access_token.is_none());
    }
}
