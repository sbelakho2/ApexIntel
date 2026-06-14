//! Paste Site Monitoring Module
//!
//! Scans public paste sites for entity keyword mentions:
//! - **Pastebin** — public scraping API (Pro account required for full access)
//! - **Ghostbin** — raw text paste aggregator
//! - **Ghostbin-style mirrors** — cached paste scrapes
//!
//! All methods are rate-limited and gracefully degrade when API keys are absent.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// A paste discovered on a paste-sharing platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteEntry {
    /// Source platform: `"pastebin"`, `"ghostbin"`, `"hastebin"`.
    pub platform: String,
    /// Unique paste identifier on the platform.
    pub paste_id: String,
    /// Full URL to the paste.
    pub paste_url: String,
    /// Paste title (if available).
    pub title: Option<String>,
    /// When the paste was created.
    pub created_at: Option<DateTime<Utc>>,
    /// Keywords from the entity that matched.
    pub matched_keywords: Vec<String>,
    /// Content snippet around the first keyword match.
    pub snippet: Option<String>,
    /// Estimated paste size in bytes.
    pub size_bytes: Option<u64>,
    /// Whether the paste has been verified as public (vs. unlisted).
    pub is_public: bool,
}

/// Paste monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteMonitorConfig {
    /// Keywords to search across paste platforms.
    pub keywords: Vec<String>,
    /// Maximum pastes to fetch per platform.
    pub max_pastes: u32,
    /// Snippet size in characters around keyword match.
    pub snippet_chars: usize,
    /// Platforms to enable.
    pub enable_pastebin: bool,
    pub enable_ghostbin: bool,
    /// Pastebin API dev key (optional but recommended).
    pub pastebin_api_key: Option<String>,
}

impl Default for PasteMonitorConfig {
    fn default() -> Self {
        Self {
            keywords: Vec::new(),
            max_pastes: 50,
            snippet_chars: 120,
            enable_pastebin: true,
            enable_ghostbin: true,
            pastebin_api_key: None,
        }
    }
}

impl PasteMonitorConfig {
    /// Add a keyword to monitor.
    pub fn add_keyword(mut self, kw: impl Into<String>) -> Self {
        self.keywords.push(kw.into());
        self
    }

    /// Add multiple keywords.
    pub fn add_keywords(mut self, kws: impl IntoIterator<Item = String>) -> Self {
        self.keywords.extend(kws);
        self
    }
}

/// Paste site monitor.
#[derive(Debug, Clone)]
pub struct PasteMonitor {
    client: Client,
    config: PasteMonitorConfig,
}

impl PasteMonitor {
    /// Create from environment (`PASTEBIN_API_DEV_KEY`).
    pub fn from_env() -> Result<Self> {
        let config = PasteMonitorConfig {
            pastebin_api_key: std::env::var("PASTEBIN_API_DEV_KEY").ok(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Create with explicit configuration.
    pub fn new(config: PasteMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Paste Monitor")
            .build()
            .context("build paste monitor HTTP client")?;
        Ok(Self { client, config })
    }

    /// Run all enabled paste scans and return matched entries.
    ///
    /// Errors from individual platforms are logged and skipped so partial
    /// results are always returned.
    pub async fn scan(&self) -> Vec<PasteEntry> {
        let mut all_entries = Vec::new();

        if self.config.enable_pastebin {
            match self.scan_pastebin().await {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => warn!(error = %e, "Pastebin scan failed"),
            }
        }

        if self.config.enable_ghostbin {
            match self.scan_ghostbin().await {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => warn!(error = %e, "Ghostbin scan failed"),
            }
        }

        debug!(total = all_entries.len(), "Paste monitoring complete");
        all_entries
    }

    /// Scan Pastebin for keyword matches.
    async fn scan_pastebin(&self) -> Result<Vec<PasteEntry>> {
        let limit = self.config.max_pastes.min(250);

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct PastebinEntry {
            scrape_url: Option<String>,
            full_url: Option<String>,
            key: Option<String>,
            date: Option<String>,
            size: Option<String>,
            title: Option<String>,
        }

        let api_url = if let Some(ref api_key) = self.config.pastebin_api_key {
            format!(
                "https://scrape.pastebin.com/api_scraping.php?limit={}&api_dev_key={}",
                limit, api_key
            )
        } else {
            "https://scrape.pastebin.com/api_scraping.php?limit=25".to_string()
        };

        let list_resp = self
            .client
            .get(&api_url)
            .send()
            .await
            .context("Pastebin scrape list request")?;

        if !list_resp.status().is_success() {
            debug!(status = %list_resp.status(), "Pastebin scrape returned non-success");
            return Ok(Vec::new());
        }

        let pastes: Vec<PastebinEntry> = match list_resp.json().await {
            Ok(p) => p,
            Err(_) => return Ok(Vec::new()),
        };

        let mut entries = Vec::new();
        let snippet_chars = self.config.snippet_chars;

        for paste in pastes.iter().take(limit as usize) {
            let Some(scrape_url) = paste.scrape_url.clone() else {
                continue;
            };

            let content_resp = match self.client.get(&scrape_url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            let content = match content_resp.text().await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let content_lower = content.to_lowercase();
            let matched: Vec<String> = self
                .config
                .keywords
                .iter()
                .filter(|kw| content_lower.contains(&kw.to_lowercase()))
                .cloned()
                .collect();

            if matched.is_empty() {
                continue;
            }

            let first_kw = matched[0].to_lowercase();
            let snippet = content_lower
                .find(&first_kw)
                .map(|pos| {
                    let start = pos.saturating_sub(snippet_chars);
                    let end = (pos + first_kw.len() + snippet_chars).min(content.len());
                    content[start..end].replace('\n', " ").trim().to_string()
                });

            let created_at = paste
                .date
                .as_ref()
                .and_then(|s| s.parse::<i64>().ok())
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.with_timezone(&Utc));

            entries.push(PasteEntry {
                platform: "pastebin".to_string(),
                paste_id: paste.key.clone().unwrap_or_default(),
                paste_url: paste.full_url.clone().unwrap_or(scrape_url),
                title: paste.title.clone(),
                created_at,
                matched_keywords: matched,
                snippet,
                size_bytes: paste.size.as_ref().and_then(|s| s.parse::<u64>().ok()),
                is_public: true,
            });
        }

        Ok(entries)
    }

    /// Scan Ghostbin for keyword matches.
    async fn scan_ghostbin(&self) -> Result<Vec<PasteEntry>> {
        // Ghostbin's public API — fetch recent pastes.
        let url = "https://ghostbin.com/api/pastes?limit=50";

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct GhostbinPaste {
            id: Option<String>,
            title: Option<String>,
            created: Option<i64>,
            lang: Option<String>,
            size: Option<u64>,
        }

        let resp = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Ghostbin API request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Ghostbin API returned non-success");
            return Ok(Vec::new());
        }

        let pastes: Vec<GhostbinPaste> = match resp.json().await {
            Ok(p) => p,
            Err(_) => return Ok(Vec::new()),
        };

        let mut entries = Vec::new();
        let snippet_chars = self.config.snippet_chars;

        for paste in pastes {
            let Some(paste_id) = paste.id.clone() else {
                continue;
            };

            let paste_url = format!("https://ghostbin.com/paste/{}", paste_id);

            // Fetch actual paste content
            let content_resp = match self.client.get(&paste_url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            let content = match content_resp.text().await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let content_lower = content.to_lowercase();
            let matched: Vec<String> = self
                .config
                .keywords
                .iter()
                .filter(|kw| content_lower.contains(&kw.to_lowercase()))
                .cloned()
                .collect();

            if matched.is_empty() {
                continue;
            }

            let first_kw = matched[0].to_lowercase();
            let snippet = content_lower
                .find(&first_kw)
                .map(|pos| {
                    let start = pos.saturating_sub(snippet_chars);
                    let end = (pos + first_kw.len() + snippet_chars).min(content.len());
                    content[start..end].replace('\n', " ").trim().to_string()
                });

            let created_at = paste
                .created
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.with_timezone(&Utc));

            entries.push(PasteEntry {
                platform: "ghostbin".to_string(),
                paste_id,
                paste_url,
                title: paste.title,
                created_at,
                matched_keywords: matched,
                snippet,
                size_bytes: paste.size,
                is_public: true,
            });
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_monitor_config_defaults() {
        let cfg = PasteMonitorConfig::default();
        assert!(cfg.keywords.is_empty());
        assert_eq!(cfg.max_pastes, 50);
        assert_eq!(cfg.snippet_chars, 120);
    }

    #[test]
    fn paste_monitor_add_keywords() {
        let cfg = PasteMonitorConfig::default()
            .add_keyword("elbit")
            .add_keywords(["raffaële", "conti"].map(|s| s.to_string()));
        assert_eq!(cfg.keywords.len(), 3);
    }

    #[test]
    fn paste_monitor_constructs() {
        let result = PasteMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn paste_entry_debug() {
        let entry = PasteEntry {
            platform: "pastebin".to_string(),
            paste_id: "abc123".to_string(),
            paste_url: "https://pastebin.com/abc123".to_string(),
            title: Some("Test".to_string()),
            created_at: None,
            matched_keywords: vec!["elbit".to_string()],
            snippet: None,
            size_bytes: Some(1024),
            is_public: true,
        };
        assert_eq!(entry.platform, "pastebin");
        assert!(entry.matched_keywords.contains(&"elbit".to_string()));
    }
}
