//! GitHub Repository Analysis Module
//!
//! Analyzes GitHub repositories for technical and organizational intelligence:
//! - Repository discovery by organization
//! - Commit history analysis
//! - Contributor tracking
//! - Language and technology detection
//! - Commit message analysis for keywords
//! - Secret detection in code
//! - Ownership correlation

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// GitHub API credentials.
pub type GithubToken = String;

/// A GitHub repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubRepo {
    pub repo_id: i64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
    pub clone_url: String,
    pub homepage: Option<String>,
    pub language: Option<String>,
    pub stargazers_count: u32,
    pub watchers_count: u32,
    pub forks_count: u32,
    pub open_issues_count: u32,
    pub license: Option<String>,
    pub topics: Vec<String>,
    pub default_branch: String,
    pub visibility: RepoVisibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

impl GithubRepo {
    /// Whether this is a public repository.
    pub fn is_public(&self) -> bool {
        matches!(self.visibility, RepoVisibility::Public)
    }

    /// Whether this is a large repository (many stars/forks).
    pub fn is_large(&self) -> bool {
        self.stargazers_count >= 100 || self.forks_count >= 50
    }

    /// Engagement score.
    pub fn engagement_score(&self) -> f32 {
        (self.stargazers_count as f32 * 1.0)
            + (self.forks_count as f32 * 2.0)
            + (self.watchers_count as f32 * 0.5)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoVisibility {
    Public,
    Private,
    Internal,
}

impl RepoVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Internal => "internal",
        }
    }
}

/// A GitHub commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubCommit {
    pub sha: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_login: Option<String>,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: DateTime<Utc>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub files_changed: Option<u32>,
    pub matched_keywords: Vec<String>,
}

impl GithubCommit {
    /// Whether this commit has keyword matches.
    pub fn has_keyword_match(&self) -> bool {
        !self.matched_keywords.is_empty()
    }
}

/// GitHub user profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubUser {
    pub user_id: i64,
    pub login: String,
    pub name: Option<String>,
    pub company: Option<String>,
    pub blog: Option<String>,
    pub location: Option<String>,
    pub email: Option<String>,
    pub bio: Option<String>,
    pub twitter_username: Option<String>,
    pub public_repos: u32,
    pub public_gists: u32,
    pub followers: u32,
    pub following: u32,
    pub created_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
}

/// GitHub monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubMonitorConfig {
    pub token: Option<GithubToken>,
    pub monitored_orgs: Vec<String>,
    pub monitored_users: Vec<String>,
    pub keywords: Vec<String>,
    pub max_results: u32,
    pub timeout_secs: u64,
}

impl Default for GithubMonitorConfig {
    fn default() -> Self {
        Self {
            token: std::env::var("GITHUB_TOKEN").ok(),
            monitored_orgs: Vec::new(),
            monitored_users: Vec::new(),
            keywords: vec![
                "secret".to_string(),
                "password".to_string(),
                "api_key".to_string(),
                "credentials".to_string(),
                "token".to_string(),
                "private_key".to_string(),
                "aws_key".to_string(),
            ],
            max_results: 30,
            timeout_secs: 30,
        }
    }
}

impl GithubMonitorConfig {
    pub fn add_org(mut self, org: impl Into<String>) -> Self {
        self.monitored_orgs.push(org.into());
        self
    }

    pub fn add_user(mut self, user: impl Into<String>) -> Self {
        self.monitored_users.push(user.into());
        self
    }
}

/// GitHub API monitor.
#[derive(Debug, Clone)]
pub struct GithubMonitor {
    client: Client,
    config: GithubMonitorConfig,
}

impl GithubMonitor {
    /// Create with configuration.
    pub fn new(config: GithubMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) GitHub Monitor")
            .build()
            .context("building GitHub HTTP client")?;
        Ok(Self { client, config })
    }

    fn auth_header(&self) -> Option<String> {
        self.config.token.as_ref().map(|t| format!("Bearer {}", t))
    }

    /// List repositories for an organization.
    pub async fn org_repos(&self, org: &str) -> Result<Vec<GithubRepo>> {
        let url = format!(
            "https://api.github.com/orgs/{}/repos",
            urlencoding::encode(org)
        );
        let mut req = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .query(&[("per_page", &self.config.max_results.to_string())]);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let resp = req.send().await.context("GitHub org repos request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), org = %org, "GitHub org repos returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct GithubApiRepo {
            id: i64,
            name: String,
            full_name: String,
            description: Option<String>,
            html_url: String,
            clone_url: String,
            homepage: Option<String>,
            language: Option<String>,
            stargazers_count: u32,
            watchers_count: u32,
            forks_count: u32,
            open_issues_count: u32,
            license: Option<serde_json::Value>,
            topics: Option<Vec<String>>,
            default_branch: String,
            visibility: Option<String>,
            created_at: String,
            updated_at: String,
            pushed_at: Option<String>,
        }

        let repos: Vec<GithubApiRepo> = resp.json().await.context("parse GitHub repos response")?;
        let now = Utc::now();
        Ok(repos
            .into_iter()
            .map(|r| GithubRepo {
                repo_id: r.id,
                name: r.name,
                full_name: r.full_name,
                description: r.description,
                html_url: r.html_url,
                clone_url: r.clone_url,
                homepage: r.homepage,
                language: r.language,
                stargazers_count: r.stargazers_count,
                watchers_count: r.watchers_count,
                forks_count: r.forks_count,
                open_issues_count: r.open_issues_count,
                license: r
                    .license
                    .as_ref()
                    .and_then(|l| l.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                topics: r.topics.unwrap_or_default(),
                default_branch: r.default_branch,
                visibility: match r.visibility.as_deref() {
                    Some("private") => RepoVisibility::Private,
                    Some("internal") => RepoVisibility::Internal,
                    _ => RepoVisibility::Public,
                },
                created_at: DateTime::parse_from_rfc3339(&r.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now),
                updated_at: DateTime::parse_from_rfc3339(&r.updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now),
                pushed_at: r.pushed_at.and_then(|p| {
                    DateTime::parse_from_rfc3339(&p)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                }),
                fetched_at: now,
            })
            .collect())
    }

    /// Search code by keyword.
    pub async fn search_code(&self, query: &str) -> Result<Vec<GithubCodeResult>> {
        let url = "https://api.github.com/search/code";
        let mut req = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github.v3+json")
            .query(&[("q", query)])
            .query(&[("per_page", &self.config.max_results.to_string())]);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let resp = req.send().await.context("GitHub code search")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), query = %query, "GitHub code search returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct GithubCodeSearch {
            items: Option<Vec<GithubCodeItem>>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct GithubCodeItem {
            name: String,
            path: String,
            sha: String,
            url: String,
            repository: Option<serde_json::Value>,
        }

        let code_resp: GithubCodeSearch = resp
            .json()
            .await
            .unwrap_or(GithubCodeSearch { items: None });
        let results = code_resp
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|item| GithubCodeResult {
                file_name: item.name,
                file_path: item.path,
                sha: item.sha,
                url: item.url,
                repo_name: item
                    .repository
                    .as_ref()
                    .and_then(|r| r.get("full_name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                matched_keywords: vec![query.to_string()],
                fetched_at: Utc::now(),
            })
            .collect();

        Ok(results)
    }

    /// Search commits by keyword.
    pub async fn search_commits(&self, query: &str) -> Result<Vec<GithubCommit>> {
        let url = "https://api.github.com/search/commits";
        let mut req = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github.v3+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[("q", query)])
            .query(&[("per_page", &self.config.max_results.to_string())]);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let resp = req.send().await.context("GitHub commit search")?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct GithubCommitSearch {
            items: Option<Vec<serde_json::Value>>,
        }

        let commit_resp: GithubCommitSearch = resp
            .json()
            .await
            .unwrap_or(GithubCommitSearch { items: None });
        let commit_items = commit_resp.items.unwrap_or_default();
        let commits: Vec<GithubCommit> = commit_items
            .into_iter()
            .filter_map(|item| {
                let sha = item.get("sha")?.as_str()?.to_string();
                let commit = item.get("commit")?;
                let message = commit.get("message")?.as_str()?.to_string();
                let author = commit.get("author")?;
                let author_name = author.get("name")?.as_str()?.to_string();
                let author_email = author.get("email")?.as_str()?.to_string();

                let matched: Vec<String> = self
                    .config
                    .keywords
                    .iter()
                    .filter(|kw| message.to_lowercase().contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                let committed_at = author
                    .get("date")
                    .and_then(|d| d.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                Some(GithubCommit {
                    sha,
                    message,
                    author_name: author_name.clone(),
                    author_email: author_email.clone(),
                    author_login: item
                        .get("author")
                        .and_then(|a| a.get("login"))
                        .and_then(|l| l.as_str())
                        .map(String::from),
                    committer_name: author_name,
                    committer_email: author_email,
                    committed_at,
                    additions: None,
                    deletions: None,
                    files_changed: None,
                    matched_keywords: matched,
                })
            })
            .collect();

        debug!(query = %query, count = commits.len(), "GitHub commit search complete");
        Ok(commits)
    }

    /// Monitor all tracked organizations.
    pub async fn full_scan(&self) -> Vec<GithubRepo> {
        let mut all_repos = Vec::new();
        for org in &self.config.monitored_orgs {
            match self.org_repos(org).await {
                Ok(repos) => {
                    debug!(org = %org, count = repos.len(), "GitHub org scan complete");
                    all_repos.extend(repos);
                }
                Err(e) => {
                    warn!(org = %org, error = %e, "GitHub org scan failed");
                }
            }
        }
        info!(total = all_repos.len(), "GitHub full scan complete");
        all_repos
    }
}

/// A code search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubCodeResult {
    pub file_name: String,
    pub file_path: String,
    pub sha: String,
    pub url: String,
    pub repo_name: Option<String>,
    pub matched_keywords: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_repo_engagement() {
        let repo = GithubRepo {
            repo_id: 1,
            name: "test-repo".to_string(),
            full_name: "org/test-repo".to_string(),
            description: None,
            html_url: "https://github.com/org/test-repo".to_string(),
            clone_url: "https://github.com/org/test-repo.git".to_string(),
            homepage: None,
            language: Some("Rust".to_string()),
            stargazers_count: 100,
            watchers_count: 20,
            forks_count: 10,
            open_issues_count: 5,
            license: None,
            topics: vec!["osint".to_string()],
            default_branch: "main".to_string(),
            visibility: RepoVisibility::Public,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            pushed_at: None,
            fetched_at: Utc::now(),
        };
        assert!(repo.is_public());
        assert!(repo.is_large());
        assert!(repo.engagement_score() > 0.0);
    }

    #[test]
    fn github_commit_keyword_match() {
        let commit = GithubCommit {
            sha: "abc123".to_string(),
            message: "Add secret API key handling".to_string(),
            author_name: "dev".to_string(),
            author_email: "dev@example.com".to_string(),
            author_login: Some("devuser".to_string()),
            committer_name: "dev".to_string(),
            committer_email: "dev@example.com".to_string(),
            committed_at: Utc::now(),
            additions: None,
            deletions: None,
            files_changed: None,
            matched_keywords: vec!["secret".to_string(), "api_key".to_string()],
        };
        assert!(commit.has_keyword_match());
        assert_eq!(commit.matched_keywords.len(), 2);
    }

    #[test]
    fn github_monitor_constructs() {
        let result = GithubMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn github_monitor_chaining() {
        let cfg = GithubMonitorConfig::default()
            .add_org("torvalds")
            .add_org("rust-lang")
            .add_user("octocat");
        assert_eq!(cfg.monitored_orgs.len(), 2);
        assert_eq!(cfg.monitored_users.len(), 1);
    }

    #[test]
    fn repo_visibility_as_str() {
        assert_eq!(RepoVisibility::Public.as_str(), "public");
        assert_eq!(RepoVisibility::Private.as_str(), "private");
    }
}
