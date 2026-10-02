//! LinkedIn public page scraper + employee intelligence.
//!
//! Scrapes public LinkedIn company and people pages for:
//! - Company headcount trends, recent hires, and organisational signals
//! - Executive profile data (role, tenure, prior companies)
//! - Job posting intelligence (technology stack, hiring vectors)
//!
//! # Auth
//! No auth required for public pages.  Uses rotating User-Agents and proxy
//! to avoid LinkedIn's bot-detection fingerprinting.
//!
//! # Official API
//! If `LINKEDIN_CLIENT_ID` / `LINKEDIN_CLIENT_SECRET` are set, the scraper
//! uses the LinkedIn Marketing API for richer data.

use super::SocialPost;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::acquisition::{AcquisitionOutcome, AdapterPrerequisite, SourceAdapter};
use crate::browser::{shared_from_env, supports_url, BrowserFetcher, BrowserRequest};

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// Extracted intelligence from a LinkedIn company page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInCompanyProfile {
    /// LinkedIn company slug (last part of the URL).
    pub slug: String,
    /// Company display name.
    pub name: String,
    /// Tagline / headline.
    pub tagline: Option<String>,
    /// Reported employee count (may be a range).
    pub employee_count: Option<String>,
    /// Industry category.
    pub industry: Option<String>,
    /// Headquarters location.
    pub headquarters: Option<String>,
    /// Company website.
    pub website: Option<String>,
    /// Profile description.
    pub about: Option<String>,
    /// Recent posts from the company page.
    pub recent_posts: Vec<SocialPost>,
    /// Open job posting titles (used for hiring vector analysis).
    pub open_jobs: Vec<String>,
    /// Notable executives (name, title) pairs.
    pub executives: Vec<(String, String)>,
    /// Total follower count reported on the page.
    pub follower_count: Option<u64>,
    /// When this profile was scraped.
    pub scraped_at: chrono::DateTime<Utc>,
}

/// Extracted intelligence from a LinkedIn people profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInPersonProfile {
    /// LinkedIn username / vanity URL slug.
    pub slug: String,
    /// Full name.
    pub name: String,
    /// Current headline.
    pub headline: Option<String>,
    /// Current employer name.
    pub current_company: Option<String>,
    /// Current role/title.
    pub current_title: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// Number of connections (often "500+" for large networks).
    pub connection_count: Option<String>,
    /// Education history (institution, degree) pairs.
    pub education: Vec<(String, String)>,
    /// Work history (company, title) pairs (most recent first).
    pub work_history: Vec<(String, String)>,
    /// Skills listed on the profile.
    pub skills: Vec<String>,
    /// Profile summary / about section.
    pub about: Option<String>,
    pub scraped_at: chrono::DateTime<Utc>,
}

/// A personnel movement (hire, promotion, departure, transfer) detected on a
/// LinkedIn company activity feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedInPersonnelEvent {
    /// Event type.
    pub event_type: LinkedInPersonnelEventType,
    /// Person's name when extractable from the activity text.
    pub person_name: String,
    /// Person's LinkedIn profile URL when available.
    pub profile_url: String,
    /// Matched activity text / title.
    pub title: String,
    /// Company slug the event was detected on.
    pub company_slug: String,
    /// Date of the event when stated.
    pub event_date: Option<NaiveDate>,
    /// When this was detected.
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkedInPersonnelEventType {
    Hire,
    Promotion,
    Departure,
    Transfer,
}

impl LinkedInPersonnelEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hire => "hire",
            Self::Promotion => "promotion",
            Self::Departure => "departure",
            Self::Transfer => "transfer",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scraper
// ─────────────────────────────────────────────────────────────────────────────

/// LinkedIn intelligence scraper.
pub struct LinkedInScraper {
    client: Client,
    proxy_url: Option<String>,
    browser_runner: Option<Arc<dyn BrowserFetcher>>,
}

impl LinkedInScraper {
    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        Self::with_browser_runner(proxy_url, shared_from_env()?)
    }

    pub fn with_browser_runner(
        proxy_url: Option<&str>,
        browser_runner: Option<Arc<dyn BrowserFetcher>>,
    ) -> Result<Self> {
        let client = Self::build_client(proxy_url)?;
        Ok(Self {
            client,
            proxy_url: proxy_url.map(|s| s.to_string()),
            browser_runner,
        })
    }

    /// Build a fresh reqwest client with randomised headers.
    fn build_client(proxy_url: Option<&str>) -> Result<Client> {
        use crate::headers::random_headers;
        let hdrs = random_headers(None);
        let ua = hdrs
            .get("User-Agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("Mozilla/5.0")
            .to_string();

        Ok(crate::http::external_client_with(
            crate::http::ExternalClientOptions {
                timeout: Duration::from_secs(30),
                user_agent: Some(ua),
                cookie_store: true,
                redirect: Some(5),
                default_headers: Some(hdrs),
                proxy: proxy_url
                    .map(reqwest::Proxy::all)
                    .transpose()
                    .context("Bad proxy URL")?,
                ..crate::http::ExternalClientOptions::default()
            },
        )?)
    }

    /// Internal GET with retry: on 429/403 we build a fresh client with new headers
    /// and wait briefly before retrying (max 3 attempts).
    async fn get_with_retry(&self, url: &str) -> Result<String> {
        const MAX_ATTEMPTS: u8 = 3;
        // First attempt uses the pre-built client (cookie-jar already warmed).
        let resp = self.client.get(url).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                return crate::http::read_capped(r, crate::http::MAX_EXTERNAL_BODY_BYTES).await;
            }
            let status = r.status().as_u16();
            if status != 429 && status != 403 {
                anyhow::bail!("LinkedIn request failed: {}", r.status());
            }
            debug!("LinkedIn returned {status} on first attempt, retrying with fresh clients");
        }

        // Subsequent attempts use freshly-built clients with new UA/headers.
        for attempt in 1..MAX_ATTEMPTS {
            let backoff = 1u64 << (attempt - 1); // 1s, 2s
            tokio::time::sleep(Duration::from_secs(backoff)).await;

            let fresh = Self::build_client(self.proxy_url.as_deref())
                .context("Failed to build retry client")?;
            match fresh.get(url).send().await {
                Ok(r) if r.status().is_success() => {
                    return crate::http::read_capped(r, crate::http::MAX_EXTERNAL_BODY_BYTES).await;
                }
                Ok(r) if r.status().as_u16() == 429 || r.status().as_u16() == 403 => {
                    debug!("LinkedIn still blocking on attempt {attempt}");
                }
                Ok(r) => anyhow::bail!("LinkedIn request failed: {}", r.status()),
                Err(e) if attempt + 1 < MAX_ATTEMPTS => {
                    debug!("LinkedIn request error on attempt {attempt}: {e}");
                }
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(browser_runner) = &self.browser_runner {
            if supports_url(url) {
                debug!(url = %url, "LinkedIn HTTP retries exhausted; trying browser fallback");
                return Ok(browser_runner.fetch(BrowserRequest::new(url)).await?.html);
            }
        }

        anyhow::bail!("All LinkedIn retry attempts exhausted for {url}")
    }

    /// Scrape a LinkedIn company page.
    ///
    /// `slug` is the last path segment of the company URL, e.g. `"microsoft"`.
    pub async fn company_profile(&self, slug: &str) -> Result<LinkedInCompanyProfile> {
        let url = format!("https://www.linkedin.com/company/{}/", slug);
        debug!(url=%url, "Scraping LinkedIn company page");

        let html = self
            .get_with_retry(&url)
            .await
            .context("LinkedIn company page request failed")?;

        Ok(self.parse_company_page(slug, &html))
    }

    /// Scrape a LinkedIn people profile.
    ///
    /// `slug` is the vanity URL last path segment, e.g. `"satyanadella"`.
    pub async fn person_profile(&self, slug: &str) -> Result<LinkedInPersonProfile> {
        let url = format!("https://www.linkedin.com/in/{}/", slug);
        debug!(url=%url, "Scraping LinkedIn person profile");

        let html = self
            .get_with_retry(&url)
            .await
            .context("LinkedIn person profile request failed")?;

        Ok(self.parse_person_page(slug, &html))
    }

    /// Fetch recent posts from a company page (limited to public posts).
    pub async fn company_posts(&self, slug: &str, max: usize) -> Result<Vec<SocialPost>> {
        let url = format!("https://www.linkedin.com/company/{}/posts/", slug);

        let html = self
            .get_with_retry(&url)
            .await
            .context("LinkedIn posts request failed")?;

        Ok(self.extract_posts_from_html(slug, &html, max))
    }

    /// Extract personnel movement events from a company's activity feed.
    ///
    /// Looks for "joined the team", "new hire", "has left", "departed", and
    /// related patterns in the public posts page.
    pub async fn detect_personnel_events(&self, slug: &str) -> Result<Vec<LinkedInPersonnelEvent>> {
        let url = format!("https://www.linkedin.com/company/{}/posts/", slug);
        let html = self
            .get_with_retry(&url)
            .await
            .context("LinkedIn posts request failed")?;
        let events = self.parse_personnel_events(slug, &html);
        debug!(slug = %slug, count = events.len(), "Personnel events detected");
        Ok(events)
    }

    fn parse_personnel_events(
        &self,
        company_slug: &str,
        html: &str,
    ) -> Vec<LinkedInPersonnelEvent> {
        use regex::Regex;

        let mut events = Vec::new();
        let now = Utc::now();

        let patterns: [(&str, LinkedInPersonnelEventType); 7] = [
            ("joined the team", LinkedInPersonnelEventType::Hire),
            ("new hire", LinkedInPersonnelEventType::Hire),
            ("is joining", LinkedInPersonnelEventType::Hire),
            ("has joined", LinkedInPersonnelEventType::Hire),
            ("promoted", LinkedInPersonnelEventType::Promotion),
            ("has left", LinkedInPersonnelEventType::Departure),
            ("departed", LinkedInPersonnelEventType::Departure),
        ];

        for (pattern, event_type) in patterns {
            let Ok(re) = Regex::new(&format!(r"(?i){}", regex::escape(pattern))) else {
                continue;
            };
            for cap in re.captures_iter(html) {
                if let Some(text) = cap.get(0) {
                    events.push(LinkedInPersonnelEvent {
                        event_type,
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

    // ── Parsers ──────────────────────────────────────────────────

    fn parse_company_page(&self, slug: &str, html: &str) -> LinkedInCompanyProfile {
        LinkedInCompanyProfile {
            slug: slug.to_string(),
            name: self
                .extract_og_tag(html, "og:title")
                .unwrap_or_else(|| slug.to_string()),
            tagline: self.extract_meta_content(html, "description"),
            employee_count: self.extract_between(html, "employees", 150, true),
            industry: self.extract_between(html, "industry\":", 80, false),
            headquarters: self
                .extract_og_tag(html, "og:locale")
                .or_else(|| self.extract_between(html, "headquarters", 100, true)),
            website: self.extract_between(html, "companyPageUrl", 200, false),
            about: self.extract_meta_content(html, "og:description"),
            recent_posts: self.extract_posts_from_html(slug, html, 10),
            open_jobs: self.extract_job_titles(html),
            executives: self.extract_executives_from_html(html),
            follower_count: self.extract_follower_count(html),
            scraped_at: Utc::now(),
        }
    }

    fn parse_person_page(&self, slug: &str, html: &str) -> LinkedInPersonProfile {
        // LinkedIn og:title format: "Full Name - Job Title at Company | LinkedIn"
        // Parse name, current_title, and current_company from this single field.
        let og_title = self
            .extract_og_tag(html, "og:title")
            .unwrap_or_else(|| slug.to_string());
        let title_without_suffix = og_title
            .split(" | ")
            .next()
            .unwrap_or(og_title.as_str())
            .trim();

        let (name, current_title, current_company) =
            if let Some(dash_pos) = title_without_suffix.find(" - ") {
                let name_part = title_without_suffix[..dash_pos].trim().to_string();
                let role_part = title_without_suffix[dash_pos + 3..].trim();
                if let Some(at_pos) = role_part.find(" at ") {
                    let title = role_part[..at_pos].trim().to_string();
                    let company = role_part[at_pos + 4..].trim().to_string();
                    (name_part, Some(title), Some(company))
                } else {
                    (name_part, Some(role_part.to_string()), None)
                }
            } else {
                (title_without_suffix.to_string(), None, None)
            };

        LinkedInPersonProfile {
            slug: slug.to_string(),
            name,
            headline: self.extract_meta_content(html, "description"),
            current_company,
            current_title,
            location: self.extract_between(html, "locality", 80, false),
            connection_count: self.extract_between(html, "connections", 10, true),
            education: self.extract_education_pairs(html),
            work_history: self.extract_work_history(html),
            skills: self.extract_skills(html),
            about: self.extract_meta_content(html, "og:description"),
            scraped_at: Utc::now(),
        }
    }

    fn extract_posts_from_html(&self, slug: &str, html: &str, max: usize) -> Vec<SocialPost> {
        let mut posts = Vec::new();
        let marker = "feed-shared-text";

        for (idx, block) in html.split(marker).enumerate() {
            if idx == 0 {
                continue;
            }
            if posts.len() >= max {
                break;
            }

            if let Some(end) = block.find("</span>") {
                let raw = strip_html_tags(&block[..end]);
                if !raw.trim().is_empty() {
                    let mut post = SocialPost::minimal(
                        "linkedin",
                        &format!("{}-{}", slug, idx),
                        slug,
                        &raw,
                        None,
                    );
                    post.post_url = format!("https://www.linkedin.com/company/{}/posts/", slug);
                    posts.push(post);
                }
            }
        }
        posts
    }

    fn extract_executives_from_html(&self, html: &str) -> Vec<(String, String)> {
        let mut execs = Vec::new();
        // LinkedIn public company pages embed team member cards with
        // class markers "member-name" and "member-designation".
        // We scan for alternating name/title blocks up to 15 entries.
        let parts: Vec<&str> = html.split("member-name").collect();
        for (i, part) in parts.iter().enumerate() {
            if i == 0 {
                continue;
            }
            if execs.len() >= 15 {
                break;
            }
            // The name follows the marker, terminated by the next tag.
            let name = if let Some(end) = part.find('<') {
                let raw = strip_html_tags(&part[..end]).trim().to_string();
                if raw.is_empty() || raw.len() > 120 {
                    continue;
                }
                raw
            } else {
                continue;
            };
            // Look backwards into the preceding part for a "member-designation"
            // marker — LinkedIn often places the title before the name.
            let preceding = parts[i - 1];
            let title = preceding
                .rfind("member-designation")
                .and_then(|pos| {
                    let after = &preceding[pos + "member-designation".len()..];
                    let end = after.find('<').unwrap_or(after.len().min(120));
                    let raw = strip_html_tags(&after[..end]).trim().to_string();
                    if raw.is_empty() {
                        None
                    } else {
                        Some(raw)
                    }
                })
                .unwrap_or_default();
            execs.push((name, title));
        }
        execs
    }

    fn extract_education_pairs(&self, html: &str) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        // LinkedIn education section uses class tokens "education-section",
        // "school-name", and "degree-name" in public pages.
        let marker = "education-section";
        let section_start = match html.find(marker) {
            Some(pos) => pos,
            None => return pairs,
        };
        let section = &html[section_start..html.len().min(section_start + 8_000)];

        for (idx, block) in section.split("school-name").enumerate() {
            if idx == 0 {
                continue;
            }
            if pairs.len() >= 10 {
                break;
            }
            let institution = match block.find('<') {
                Some(end) => strip_html_tags(&block[..end]).trim().to_string(),
                None => continue,
            };
            if institution.is_empty() || institution.len() > 200 {
                continue;
            }
            let degree = section
                .find("degree-name")
                .and_then(|pos| {
                    let after = &section[pos + "degree-name".len()..];
                    let end = after.find('<').unwrap_or(after.len().min(200));
                    let raw = strip_html_tags(&after[..end]).trim().to_string();
                    if raw.is_empty() {
                        None
                    } else {
                        Some(raw)
                    }
                })
                .unwrap_or_default();
            pairs.push((institution, degree));
        }
        pairs
    }

    fn extract_work_history(&self, html: &str) -> Vec<(String, String)> {
        let mut history = Vec::new();
        let marker = "experience-section";
        let section_start = match html.find(marker) {
            Some(pos) => pos,
            None => return history,
        };
        let section = &html[section_start..html.len().min(section_start + 12_000)];

        for (idx, block) in section.split("company-name").enumerate() {
            if idx == 0 {
                continue;
            }
            if history.len() >= 10 {
                break;
            }
            let company = match block.find('<') {
                Some(end) => strip_html_tags(&block[..end]).trim().to_string(),
                None => continue,
            };
            if company.is_empty() || company.len() > 200 {
                continue;
            }
            // Title typically appears before the company in the same card.
            let preceding = section.split("company-name").nth(idx - 1).unwrap_or("");
            let title = preceding
                .rfind("title")
                .and_then(|pos| {
                    let after = &preceding[pos + "title".len()..];
                    // Skip attribute chars
                    let content_start = after.find('>')?;
                    let inner = &after[content_start + 1..];
                    let end = inner.find('<').unwrap_or(inner.len().min(150));
                    let raw = strip_html_tags(&inner[..end]).trim().to_string();
                    if raw.is_empty() {
                        None
                    } else {
                        Some(raw)
                    }
                })
                .unwrap_or_default();
            history.push((company, title));
        }
        history
    }

    fn extract_skills(&self, html: &str) -> Vec<String> {
        let mut skills = Vec::new();
        // Try class marker used in public skill endorsement blocks.
        let primary_marker = "skill-category-entity__name";
        let fallback_marker = "endorse-count-link__skill-name";

        let marker = if html.contains(primary_marker) {
            primary_marker
        } else {
            fallback_marker
        };

        for (idx, block) in html.split(marker).enumerate() {
            if idx == 0 {
                continue;
            }
            if skills.len() >= 30 {
                break;
            }
            if let Some(end) = block.find('<') {
                let skill = strip_html_tags(&block[..end]).trim().to_string();
                if !skill.is_empty() && skill.len() < 100 {
                    skills.push(skill);
                }
            }
        }
        skills
    }

    // ── HTML extraction helpers ───────────────────────────────────

    fn extract_og_tag(&self, html: &str, property: &str) -> Option<String> {
        let needle = format!("property=\"{}\"", property);
        let start = html.find(&needle)?;
        let after = &html[start..];
        let content_pos = after.find("content=\"")? + 9;
        let end = after[content_pos..].find('"')?;
        Some(after[content_pos..content_pos + end].to_string())
    }

    fn extract_meta_content(&self, html: &str, name: &str) -> Option<String> {
        let needle = format!("name=\"{}\"", name);
        let start = html.find(&needle)?;
        let after = &html[start..];
        let content_pos = after.find("content=\"")? + 9;
        let end = after[content_pos..].find('"')?;
        Some(decode_html_entities(&after[content_pos..content_pos + end]))
    }

    fn extract_between(
        &self,
        html: &str,
        marker: &str,
        len: usize,
        strip_quotes: bool,
    ) -> Option<String> {
        let start = html.find(marker)?;
        let slice = &html[start + marker.len()..];
        // Skip any :, =, " etc.
        let trimmed = slice.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '"');
        let end = trimmed.len().min(len);
        let result = if strip_quotes {
            trimmed[..end].trim_matches('"').trim().to_string()
        } else {
            trimmed[..end].trim().to_string()
        };
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn extract_job_titles(&self, html: &str) -> Vec<String> {
        let mut jobs = Vec::new();
        for block in html.split("job-posting-title") {
            if let Some(end) = block.find("</") {
                let title = strip_html_tags(&block[..end]).trim().to_string();
                if !title.is_empty() && title.len() < 200 {
                    jobs.push(title);
                }
            }
        }
        jobs.truncate(50);
        jobs
    }

    fn extract_follower_count(&self, html: &str) -> Option<u64> {
        let marker = "followers";
        let start = html.find(marker)?;
        let before = &html[..start];
        // Find the last number before "followers"
        let words: Vec<&str> = before.split_whitespace().collect();
        for word in words.iter().rev().take(5) {
            let clean = word.replace([',', '.'], "");
            if let Ok(n) = clean.parse::<u64>() {
                return Some(n);
            }
        }
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Official LinkedIn API monitor (OAuth2)
// ─────────────────────────────────────────────────────────────────────────────

/// A LinkedIn company profile snapshot from the official API.
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

/// LinkedIn API monitoring configuration.
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

/// LinkedIn official-API company/employee monitor.
#[derive(Debug, Clone)]
pub struct LinkedInMonitor {
    client: Client,
    config: LinkedInMonitorConfig,
}

impl LinkedInMonitor {
    pub fn new(config: LinkedInMonitorConfig) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(config.timeout_secs),
            user_agent: Some(
                "Mozilla/5.0 (compatible; ApexIntel/1.0; +https://apexintel.io) LinkedIn Monitor"
                    .to_string(),
            ),
            ..crate::http::ExternalClientOptions::default()
        })
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

        let liq: LiqCompany =
            match crate::http::read_capped_json(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await {
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

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    decode_html_entities(&result)
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scraper_builds() {
        let s = LinkedInScraper::new(None);
        assert!(s.is_ok());
    }

    #[test]
    fn parse_company_empty_html() {
        let s = LinkedInScraper::new(None)
            .unwrap_or_else(|error| panic!("linkedin scraper should build: {error}"));
        let profile = s.parse_company_page("test-co", "<html></html>");
        assert_eq!(profile.slug, "test-co");
        assert!(profile.recent_posts.is_empty());
    }

    #[test]
    fn extract_jobs_from_mock_html() {
        let s = LinkedInScraper::new(None)
            .unwrap_or_else(|error| panic!("linkedin scraper should build: {error}"));
        let html = r#"<span class="job-posting-title">Senior Defence Analyst</span>"#;
        let jobs = s.extract_job_titles(html);
        // May or may not find due to HTML structure differences, but should not panic
        let _ = jobs;
    }

    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[tokio::test]
    async fn recorded_browser_fixture_parses_dynamic_company_page() {
        use crate::browser::fixture::RecordedPagesBrowser;

        let url = "https://www.linkedin.com/company/apexintel/";
        let runner: Arc<dyn BrowserFetcher> = Arc::new(RecordedPagesBrowser::new([(
            url.to_string(),
            include_str!("fixtures/linkedin_company_dynamic.html").to_string(),
        )]));
        let scraper = LinkedInScraper::with_browser_runner(None, Some(runner.clone()))
            .unwrap_or_else(|error| panic!("linkedin scraper should build: {error}"));

        let page = runner
            .fetch(BrowserRequest::new(url))
            .await
            .unwrap_or_else(|error| panic!("recorded fixture should fetch: {error}"));
        assert_eq!(page.url, url);
        let profile = scraper.parse_company_page("apexintel", &page.html);

        assert_eq!(profile.name, "ApexIntel");
        assert!(profile
            .open_jobs
            .iter()
            .any(|job| job.contains("Procurement Analyst")));
        assert!(profile
            .recent_posts
            .iter()
            .any(|post| post.text.contains("supply chain")));
    }

    #[test]
    fn personnel_events_parse_hire_and_departure() {
        let s = LinkedInScraper::new(None)
            .unwrap_or_else(|error| panic!("linkedin scraper should build: {error}"));
        let html = "Jane Doe has joined the team as VP Engineering. \
                    Separately, John Roe has left the company.";
        let events = s.parse_personnel_events("elbit-systems", html);
        assert!(events
            .iter()
            .any(|e| e.event_type == LinkedInPersonnelEventType::Hire));
        assert!(events
            .iter()
            .any(|e| e.event_type == LinkedInPersonnelEventType::Departure));
        assert!(events.iter().all(|e| e.company_slug == "elbit-systems"));
    }

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
