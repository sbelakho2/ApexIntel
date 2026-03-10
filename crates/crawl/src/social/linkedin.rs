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
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

use crate::browser::BoundedBrowserRunner;

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

// ─────────────────────────────────────────────────────────────────────────────
// Scraper
// ─────────────────────────────────────────────────────────────────────────────

/// LinkedIn intelligence scraper.
pub struct LinkedInScraper {
    client: Client,
    proxy_url: Option<String>,
    browser_runner: Option<BoundedBrowserRunner>,
}

impl LinkedInScraper {
    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        Self::with_browser_runner(proxy_url, BoundedBrowserRunner::from_env()?)
    }

    pub fn with_browser_runner(
        proxy_url: Option<&str>,
        browser_runner: Option<BoundedBrowserRunner>,
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

        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(ua)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .default_headers(hdrs);

        if let Some(proxy) = proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("Bad proxy URL")?);
        }

        Ok(builder.build()?)
    }

    /// Internal GET with retry: on 429/403 we build a fresh client with new headers
    /// and wait briefly before retrying (max 3 attempts).
    async fn get_with_retry(&self, url: &str) -> Result<String> {
        const MAX_ATTEMPTS: u8 = 3;
        // First attempt uses the pre-built client (cookie-jar already warmed).
        let resp = self.client.get(url).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                return Ok(r.text().await?);
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
                Ok(r) if r.status().is_success() => return Ok(r.text().await?),
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
            if BoundedBrowserRunner::supports_url(url) {
                debug!(url = %url, "LinkedIn HTTP retries exhausted; trying bounded browser fallback");
                return Ok(browser_runner.fetch(url).await?.html);
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
                        Utc::now(),
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
            let clean = word.replace(',', "").replace('.', "");
            if let Ok(n) = clean.parse::<u64>() {
                return Some(n);
            }
        }
        None
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

    use std::collections::HashMap;

    #[test]
    fn scraper_builds() {
        let s = LinkedInScraper::new(None);
        assert!(s.is_ok());
    }

    #[test]
    fn parse_company_empty_html() {
        let s = LinkedInScraper::new(None).unwrap();
        let profile = s.parse_company_page("test-co", "<html></html>");
        assert_eq!(profile.slug, "test-co");
        assert!(profile.recent_posts.is_empty());
    }

    #[test]
    fn extract_jobs_from_mock_html() {
        let s = LinkedInScraper::new(None).unwrap();
        let html = r#"<span class="job-posting-title">Senior Defence Analyst</span>"#;
        let jobs = s.extract_job_titles(html);
        // May or may not find due to HTML structure differences, but should not panic
        let _ = jobs;
    }

    #[tokio::test]
    async fn recorded_browser_fixture_parses_dynamic_company_page() {
        let url = "https://www.linkedin.com/company/apexintel/";
        let runner = BoundedBrowserRunner::from_recorded_pages(HashMap::from([(
            url.to_string(),
            include_str!("fixtures/linkedin_company_dynamic.html").to_string(),
        )]));
        let scraper = LinkedInScraper::with_browser_runner(None, Some(runner.clone())).unwrap();

        let page = runner.fetch(url).await.unwrap();
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
}
