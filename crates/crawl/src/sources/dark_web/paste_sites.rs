//! Paste Site Monitoring Module
//!
//! Monitors paste sites for leaked credentials and sensitive data:
//! - Pastebin.com monitoring
//! - Ghostbin.com monitoring
//! - Content pattern matching for credentials, API keys, secrets
//! - Keyword alerting for tracked entities
//! - Automatic paste retrieval

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

use crate::acquisition::{AcquisitionOutcome, AdapterPrerequisite, SourceAdapter};

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
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(config.timeout_secs),
            user_agent: Some(crate::dark_web::DARK_WEB_USER_AGENT.to_string()),
            ..crate::http::ExternalClientOptions::default()
        })
        .context("building Paste monitor HTTP client")?;
        Ok(Self {
            client,
            config,
            entries: Vec::new(),
        })
    }

    /// Scan Pastebin for keyword matches.
    pub async fn scan_pastebin(&mut self) -> AcquisitionOutcome<PasteSiteEntry> {
        // Pastebin API scrapes (simple monitoring via RSS-like scraping)
        let url = "https://scrape.pastebin.com/api_scrape_item.php?i=recent";
        let resp = match self.client.get(url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("Pastebin scrape request failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Pastebin scrape returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "Pastebin scrape",
            );
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("read Pastebin response failed: {error}"),
                    None,
                );
            }
        };
        self.parse_pastebin_pastes(&text).await
    }

    async fn parse_pastebin_pastes(&mut self, text: &str) -> AcquisitionOutcome<PasteSiteEntry> {
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

        let pastes: Vec<PastebinPaste> = match serde_json::from_str(text) {
            Ok(pastes) => pastes,
            Err(error) => {
                return AcquisitionOutcome::parse_failed(
                    format!("parse Pastebin JSON failed: {error}"),
                    text,
                );
            }
        };

        let mut entries = Vec::new();
        for p in pastes {
            let Some(paste_id) = p.key else {
                // An entry without an ID cannot be identified: skip it rather
                // than failing the whole page (the parser contract still held).
                continue;
            };
            let title = p.title.clone();

            // Fetch content synchronously for non-async context
            let paste_client =
                crate::http::external_client_or_panic(crate::http::ExternalClientOptions {
                    timeout: Duration::from_secs(10),
                    ..crate::http::ExternalClientOptions::default()
                });
            let content = if self.config.fetch_content {
                match paste_client
                    .get(format!("https://pastebin.com/raw/{}", paste_id))
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES)
                            .await
                            .unwrap_or_default()
                    }
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
        AcquisitionOutcome::success_now(entries)
    }

    /// Scan Ghostbin for keyword matches.
    ///
    /// Ghostbin has no public scraping API, so the adapter is explicitly
    /// [`AcquisitionOutcome::NotApplicable`] rather than a fake empty success.
    pub async fn scan_ghostbin(&mut self) -> AcquisitionOutcome<PasteSiteEntry> {
        AcquisitionOutcome::NotApplicable
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

/// Request for one paste-sites scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PasteSiteScanRequest;

#[async_trait]
impl SourceAdapter for PasteMonitor {
    type Item = PasteSiteEntry;
    type Request = PasteSiteScanRequest;

    fn adapter_id(&self) -> &'static str {
        "dark_web_paste_sites"
    }

    fn prerequisite(&self) -> AdapterPrerequisite {
        AdapterPrerequisite::NONE
    }

    async fn acquire(&self, _request: PasteSiteScanRequest) -> AcquisitionOutcome<PasteSiteEntry> {
        // `scan_pastebin` needs `&mut self` for its entry cache; clone the
        // monitor so the adapter contract stays `&self`.
        let mut monitor = self.clone();
        monitor.scan_pastebin().await
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
    fn paste_sites_adapter_declares_its_prerequisite() {
        let monitor = PasteMonitor::new(Default::default()).expect("paste sites monitor");
        assert_eq!(monitor.adapter_id(), "dark_web_paste_sites");
        assert!(!monitor.prerequisite().requires_credentials);
        assert!(crate::acquisition::adapter_descriptor(monitor.adapter_id()).is_some());
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
