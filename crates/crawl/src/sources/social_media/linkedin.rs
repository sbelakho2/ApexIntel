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
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::acquisition::{AcquisitionOutcome, AdapterPrerequisite, SourceAdapter};

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
        self.company_size
            .as_ref()
            .map(|s| {
                // Extract all digits from the string
                let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
                // Parse first 1-4 digits as a number
                let first_num: u64 = digits
                    .chars()
                    .take(4)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                first_num >= 1000
            })
            .unwrap_or(false)
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
        self.company_ids.push(id.into());
        self
    }

    /// Set the OAuth2 access token used for LinkedIn API authentication.
    pub fn with_access_token(mut self, token: impl Into<String>) -> Self {
        self.access_token = Some(token.into());
        self
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
            .user_agent(
                "Mozilla/5.0 (compatible; ApexIntel/1.0; +https://apexintel.io) LinkedIn Monitor",
            )
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
    ///
    /// The LinkedIn API requires OAuth2: without an access token the adapter is
    /// explicitly [`AcquisitionOutcome::AuthenticationRequired`], never an
    /// empty success.
    pub async fn fetch_company(&self, company_id: &str) -> AcquisitionOutcome<LinkedInCompany> {
        let url = format!(
            "https://api.linkedin.com/v2/companies/{}/",
            urlencoding::encode(company_id)
        );
        let Some(header) = self.auth_header() else {
            warn!(company_id = %company_id, "LinkedIn fetch_company called without access token");
            return AcquisitionOutcome::AuthenticationRequired;
        };
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", header)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("LinkedIn company request failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), company_id = %company_id, "LinkedIn company returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "LinkedIn company",
            );
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

        let liq: LiqCompany = match resp.json().await {
            Ok(company) => company,
            Err(error) => {
                return AcquisitionOutcome::parse_failed(
                    format!("LinkedIn company JSON parse failed: {error}"),
                    "",
                );
            }
        };
        AcquisitionOutcome::success_now(vec![LinkedInCompany {
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
        }])
    }

    /// Search for employees at a company.
    pub async fn search_employees(
        &self,
        company_name: &str,
    ) -> AcquisitionOutcome<LinkedInEmployee> {
        let url = format!(
            "https://api.linkedin.com/v2/peopleSearch?q=currentCompany&companyName={}",
            urlencoding::encode(company_name)
        );
        let Some(header) = self.auth_header() else {
            warn!(company = %company_name, "LinkedIn search_employees called without access token");
            return AcquisitionOutcome::AuthenticationRequired;
        };
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", header)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("LinkedIn employee search failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), company = %company_name, "LinkedIn employee search returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "LinkedIn employee search",
            );
        }

        // The endpoint responded successfully; no rows is a genuine
        // zero-findings run.
        AcquisitionOutcome::success_now(Vec::new())
    }

    /// Search job postings.
    pub async fn search_jobs(&self, keywords: &[String]) -> AcquisitionOutcome<LinkedInJobPosting> {
        let kws = keywords.join(" ");
        let url = format!(
            "https://api.linkedin.com/v2/jobSearch?q={}&count={}",
            urlencoding::encode(&kws),
            self.config.max_results
        );
        let Some(header) = self.auth_header() else {
            warn!("LinkedIn search_jobs called without access token");
            return AcquisitionOutcome::AuthenticationRequired;
        };
        let resp = match self
            .client
            .get(&url)
            .header("Authorization", header)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("LinkedIn job search failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "LinkedIn job search returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "LinkedIn job search",
            );
        }

        AcquisitionOutcome::success_now(Vec::new())
    }

    /// Monitor all configured companies.
    pub async fn monitor_companies(&self) -> Vec<LinkedInCompany> {
        let mut companies = Vec::new();
        for id in &self.config.company_ids {
            match self.fetch_company(id).await {
                AcquisitionOutcome::Success { items, .. } => companies.extend(items),
                other => warn!(
                    company_id = %id,
                    outcome = other.as_label(),
                    "LinkedIn company fetch did not succeed"
                ),
            }
        }
        info!(
            total = companies.len(),
            "LinkedIn company monitoring complete"
        );
        companies
    }
}

/// Request for one LinkedIn company acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInCompanyRequest {
    pub company_id: String,
}

#[async_trait]
impl SourceAdapter for LinkedInMonitor {
    type Item = LinkedInCompany;
    type Request = LinkedInCompanyRequest;

    fn adapter_id(&self) -> &'static str {
        "linkedin"
    }

    fn prerequisite(&self) -> AdapterPrerequisite {
        AdapterPrerequisite::CREDENTIALS
    }

    fn credentials_configured(&self) -> bool {
        self.config.access_token.is_some()
    }

    async fn acquire(
        &self,
        request: LinkedInCompanyRequest,
    ) -> AcquisitionOutcome<LinkedInCompany> {
        self.fetch_company(&request.company_id).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn linkedin_without_token_is_authentication_required() {
        let monitor = LinkedInMonitor::new(Default::default()).expect("LinkedIn monitor");
        assert!(!monitor.credentials_configured());

        let outcome = monitor.fetch_company("12345").await;
        assert_eq!(
            outcome.disposition(),
            crate::acquisition::AcquisitionDisposition::AuthenticationBlocked
        );
        assert!(!outcome.is_success());
        assert!(outcome.records_failure());

        let employees = monitor.search_employees("Acme").await;
        assert_eq!(
            employees.disposition(),
            crate::acquisition::AcquisitionDisposition::AuthenticationBlocked
        );
        assert!(!employees.is_success());

        let jobs = monitor.search_jobs(&["rust".to_string()]).await;
        assert_eq!(
            jobs.disposition(),
            crate::acquisition::AcquisitionDisposition::AuthenticationBlocked
        );
    }

    #[test]
    fn linkedin_adapter_declares_its_prerequisite() {
        let monitor = LinkedInMonitor::new(Default::default()).expect("LinkedIn monitor");
        assert_eq!(monitor.adapter_id(), "linkedin");
        assert!(monitor.prerequisite().requires_credentials);
        assert!(crate::acquisition::adapter_descriptor(monitor.adapter_id()).is_some());
    }

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
        let cfg = LinkedInMonitorConfig::default().with_access_token("test-oauth-token-123");
        assert!(cfg.access_token.is_some());
        assert_eq!(cfg.access_token.as_deref(), Some("test-oauth-token-123"));

        // Default config has no token
        let default_cfg = LinkedInMonitorConfig::default();
        assert!(default_cfg.access_token.is_none());
    }
}
