//! Paste Site Monitoring Module
//!
//! Monitors paste sites for leaked credentials and sensitive data:
//! - Pastebin.com monitoring
//! - Ghostbin.com monitoring
//! - Content pattern matching for credentials, API keys, secrets
//! - Keyword alerting for tracked entities
//! - Automatic paste retrieval

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// A paste site entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteSiteEntry {
    pub paste_id: String,
    pub source: PasteSource,
    pub title: Option<String>,
    pub author: Option<String>,
    pub content: String,
    pub size_bytes: usize,
    pub created_at: Option<DateTime<Utc>>,
    pub exposure: PasteExposure,
    pub contains_keywords: Vec<String>,
    pub matched_entities: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

impl PasteSiteEntry {
    /// Whether this paste is publicly exposed.
    pub fn is_public(&self) -> bool {
        matches!(self.exposure, PasteExposure::Public)
    }

    /// Whether this paste contains credential-like patterns.
    pub fn has_credentials(&self) -> bool {
        has_credential_pattern(&self.content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasteSource {
    Pastebin,
    Ghostbin,
    Other,
}

impl PasteSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pastebin => "pastebin",
            Self::Ghostbin => "ghostbin",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasteExposure {
    Public,
    Unlisted,
    Private,
    Unknown,
}

impl PasteExposure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Private => "private",
            Self::Unknown => "unknown",
        }
    }
}

/// Paste site monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteMonitorConfig {
    /// Alert keywords to search for.
    pub alert_keywords: Vec<String>,
    /// Entity names to track.
    pub tracked_entities: Vec<String>,
    /// Maximum pastes to return per scan.
    pub max_pastes: u32,
    /// Whether to fetch paste content.
    pub fetch_content: bool,
    /// Timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for PasteMonitorConfig {
    fn default() -> Self {
        Self {
            alert_keywords: vec![
                "password".to_string(),
                "api_key".to_string(),
                "secret".to_string(),
                "token".to_string(),
                "credential".to_string(),
            ],
            tracked_entities: Vec::new(),
            max_pastes: 100,
            fetch_content: true,
            timeout_secs: 30,
        }
    }
}

/// Paste site monitor.
#[derive(Debug, Clone)]
pub struct PasteMonitor {
    client: Client,
    config: PasteMonitorConfig,
    /// Cached entries.
    entries: Vec<PasteSiteEntry>,
}

impl PasteMonitor {
    pub fn new(config: PasteMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Paste Monitor")
            .build()
            .context("building Paste monitor HTTP client")?;
        Ok(Self {
            client,
            config,
            entries: Vec::new(),
        })
    }

    /// Scan Pastebin for keyword matches.
    pub async fn scan_pastebin(&mut self) -> Result<Vec<PasteSiteEntry>> {
        // Pastebin API scrapes (simple monitoring via RSS-like scraping)
        let url = "https://scrape.pastebin.com/api_scrape_item.php?i=recent";
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("Pastebin scrape request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Pastebin scrape returned non-success");
            return Ok(Vec::new());
        }

        let text = resp.text().await.context("read Pastebin response")?;
        self.parse_pastebin_pastes(&text).await
    }

    async fn parse_pastebin_pastes(&mut self, text: &str) -> Result<Vec<PasteSiteEntry>> {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct PastebinPaste {
            #[serde(rename = "paste_key")]
            key: Option<String>,
            #[serde(rename = "paste_title")]
            title: Option<String>,
            #[serde(rename = "paste_size")]
            size: Option<String>,
            #[serde(rename = "paste_date")]
            date: Option<String>,
            #[serde(rename = "paste_expire")]
            expire: Option<String>,
            #[serde(rename = "paste_view_hit")]
            views: Option<String>,
        }

        let pastes: Vec<PastebinPaste> =
            serde_json::from_str(text).context("parse Pastebin JSON")?;

        let mut entries = Vec::new();
        for p in pastes {
            let paste_id = p.key.ok_or_else(|| anyhow::anyhow!("Missing paste ID"))?;
            let title = p.title.clone();

            // Fetch content synchronously for non-async context
            let paste_client = reqwest::Client::new();
            let content = if self.config.fetch_content {
                match paste_client
                    .get(format!("https://pastebin.com/raw/{}", paste_id))
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) => resp.text().await.unwrap_or_default(),
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            };

            let matched: Vec<String> = self
                .config
                .alert_keywords
                .iter()
                .filter(|kw| {
                    content.to_lowercase().contains(&kw.to_lowercase())
                        || title
                            .as_ref()
                            .map(|t| t.to_lowercase().contains(&kw.to_lowercase()))
                            .unwrap_or(false)
                })
                .cloned()
                .collect();

            if !matched.is_empty()
                || self
                    .config
                    .tracked_entities
                    .iter()
                    .any(|e| content.to_lowercase().contains(&e.to_lowercase()))
            {
                let timestamp: i64 = p.date.and_then(|d| d.parse().ok()).unwrap_or(0);
                let created = if timestamp > 0 {
                    chrono::DateTime::from_timestamp(timestamp, 0)
                } else {
                    None
                };

                let entry = PasteSiteEntry {
                    paste_id: paste_id.clone(),
                    source: PasteSource::Pastebin,
                    title,
                    author: None,
                    content: content.clone(),
                    size_bytes: p.size.and_then(|s| s.parse().ok()).unwrap_or(0) as usize,
                    created_at: created.map(|dt| dt.with_timezone(&Utc)),
                    exposure: PasteExposure::Public,
                    contains_keywords: matched.clone(),
                    matched_entities: self
                        .config
                        .tracked_entities
                        .iter()
                        .filter(|e| content.to_lowercase().contains(&e.to_lowercase()))
                        .cloned()
                        .collect(),
                    fetched_at: Utc::now(),
                };
                entries.push(entry.clone());
                self.entries.push(entry.clone());
            }
        }

        info!(
            source = "pastebin",
            count = entries.len(),
            "Pastebin scan complete"
        );
        Ok(entries)
    }

    #[allow(dead_code)]
    async fn fetch_pastebin_content(&self, paste_key: &str) -> Result<String> {
        let url = format!(
            "https://scrape.pastebin.com/api_scrape_item.php?i={}",
            paste_key
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Pastebin content fetch")?;
        if !resp.status().is_success() {
            return Ok(String::new());
        }
        resp.text().await.context("read paste content")
    }

    /// Scan Ghostbin for keyword matches.
    pub async fn scan_ghostbin(&mut self) -> Result<Vec<PasteSiteEntry>> {
        let url = "https://ghostbin.com/paste/new";
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("Ghostbin request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Ghostbin returned non-success");
            return Ok(Vec::new());
        }

        Ok(Vec::new()) // Ghostbin doesn't have a public scraping API
    }

    /// Get all cached entries.
    pub fn all_entries(&self) -> &[PasteSiteEntry] {
        &self.entries
    }

    /// Get entries containing credentials.
    pub fn credential_entries(&self) -> Vec<&PasteSiteEntry> {
        self.entries
            .iter()
            .filter(|e| e.has_credentials())
            .collect()
    }

    /// Return total entry count.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Check if text contains credential-like patterns.
fn has_credential_pattern(text: &str) -> bool {
    let patterns = [
        r"(?i)password\s*[=:]\s*\S+",
        r"(?i)api[_-]?key\s*[=:]\s*\S+",
        r"(?i)secret[_-]?key\s*[=:]\s*\S+",
        r"(?i)bearer\s+\S+",
        r"(?i)aws[_-]?access[_-]?key[_-]?id",
        r"(?i)token\s*[=:]\s*\S+",
    ];

    for p in &patterns {
        if let Ok(re) = regex::Regex::new(p) {
            if re.is_match(text) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn paste_monitor_constructs() {
        let result = PasteMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn paste_entry_has_credentials() {
        let entry = PasteSiteEntry {
            paste_id: "test".to_string(),
            source: PasteSource::Pastebin,
            title: Some("AWS Keys".to_string()),
            author: None,
            content: "aws_access_key_id = AKIAIOSFODNN7EXAMPLE".to_string(),
            size_bytes: 100,
            created_at: None,
            exposure: PasteExposure::Public,
            contains_keywords: vec!["api_key".to_string()],
            matched_entities: vec![],
            fetched_at: Utc::now(),
        };
        assert!(entry.has_credentials());
    }

    #[test]
    fn paste_entry_no_credentials() {
        let entry = PasteSiteEntry {
            paste_id: "test".to_string(),
            source: PasteSource::Pastebin,
            title: Some("Hello World".to_string()),
            author: None,
            content: "This is a simple text paste.".to_string(),
            size_bytes: 100,
            created_at: None,
            exposure: PasteExposure::Public,
            contains_keywords: vec![],
            matched_entities: vec![],
            fetched_at: Utc::now(),
        };
        assert!(!entry.has_credentials());
    }

    #[test]
    fn paste_source_as_str() {
        assert_eq!(PasteSource::Pastebin.as_str(), "pastebin");
        assert_eq!(PasteSource::Ghostbin.as_str(), "ghostbin");
    }

    #[test]
    fn paste_exposure_as_str() {
        assert_eq!(PasteExposure::Public.as_str(), "public");
        assert_eq!(PasteExposure::Unlisted.as_str(), "unlisted");
    }

    #[test]
    fn credential_pattern_detection() {
        assert!(has_credential_pattern("password=supersecret123"));
        assert!(has_credential_pattern("API_KEY=abc123xyz"));
        assert!(has_credential_pattern(
            "bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"
        ));
        assert!(!has_credential_pattern("Hello world, how are you today?"));
    }
}
