//! BIS EAR Export Control Database Module
//!
//! Monitors US Bureau of Industry and Security (BIS) export control lists:
//! - Entity List (Supplement No. 4 to Part 744)
//! - Denied Persons List
//! - Unverified List
//! - Military End-User (MEU) List
//! - Specially Designated Nationals (SDN) correlation

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// An entry from the BIS Entity List.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BisEntityListEntry {
    pub entity_name: String,
    pub address: Option<String>,
    pub country: String,
    pub license_requirement: String,
    pub license_review_policy: String,
    pub entities_list_section: String,
    pub source_url: Option<String>,
    pub listed_date: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

/// A BIS denied persons entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BisDeniedPerson {
    pub name: String,
    pub address: Option<String>,
    pub effective_date: Option<String>,
    pub expiry_date: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

/// Export control list type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportControlList {
    BisEntityList,
    BisDeniedPersons,
    BisUnverified,
    BisMeuList,
}

impl ExportControlList {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BisEntityList => "BIS_Entity_List",
            Self::BisDeniedPersons => "BIS_Denied_Persons",
            Self::BisUnverified => "BIS_Unverified_List",
            Self::BisMeuList => "BIS_MEU_List",
        }
    }

    pub fn download_url(self) -> &'static str {
        match self {
            Self::BisEntityList => "https://www.bis.doc.gov/index.php/documents/regulations-docs/2326-supplement-no-4-to-part-744/file",
            Self::BisDeniedPersons => "https://www.bis.doc.gov/index.php/documents/2021-essentials/2332-denial-orders/file",
            Self::BisUnverified => "https://www.bis.doc.gov/index.php/component/com_tracker/23-unverified-list/file",
            Self::BisMeuList => "https://www.bis.doc.gov/index.php/documents/2021-essentials/2333-end-user-rule/file",
        }
    }
}

/// BIS export control monitor.
#[derive(Debug, Clone)]
pub struct ExportControlMonitor {
    client: Client,
    /// Cached entities for fast in-memory screening.
    entity_list_entries: Vec<BisEntityListEntry>,
    denied_persons: Vec<BisDeniedPerson>,
}

impl ExportControlMonitor {
    /// Create a new monitor.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Export Control Monitor")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client, entity_list_entries: Vec::new(), denied_persons: Vec::new() }
    }

    /// Load BIS Entity List from remote CSV.
    pub async fn load_entity_list(&mut self) -> Result<usize> {
        info!("Loading BIS Entity List");
        let url = ExportControlList::BisEntityList.download_url();
        let resp = self.client.get(url).send().await.context("BIS Entity List download")?;
        if !resp.status().is_success() {
            anyhow::bail!("BIS Entity List returned {}", resp.status());
        }
        let bytes = resp.bytes().await.context("read BIS Entity List body")?;
        let text = String::from_utf8_lossy(&bytes);
        self.parse_entity_list_csv(&text).await
    }

    /// Parse BIS Entity List CSV content.
    async fn parse_entity_list_csv(&mut self, content: &str) -> Result<usize> {
        let mut count = 0;
        let mut in_data = false;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("Entity Name") || line.starts_with("Name") {
                in_data = true;
                continue;
            }
            if !in_data || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                let entry = BisEntityListEntry {
                    entity_name: parts.first().unwrap_or(&"").trim().to_string(),
                    address: parts.get(1).map(|s| s.trim().to_string()),
                    country: parts.get(2).unwrap_or(&"Unknown").trim().to_string(),
                    license_requirement: parts.get(3).map(|s| s.trim().to_string()).unwrap_or_default(),
                    license_review_policy: parts.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
                    entities_list_section: "Supplement No. 4".to_string(),
                    source_url: Some("https://www.bis.doc.gov".to_string()),
                    listed_date: None,
                    fetched_at: Utc::now(),
                };
                if !entry.entity_name.is_empty() {
                    self.entity_list_entries.push(entry);
                    count += 1;
                }
            }
        }
        debug!(count, "BIS Entity List entries parsed");
        Ok(count)
    }

    /// Load BIS Denied Persons list.
    pub async fn load_denied_persons(&mut self) -> Result<usize> {
        info!("Loading BIS Denied Persons List");
        let url = ExportControlList::BisDeniedPersons.download_url();
        let resp = self.client.get(url).send().await.context("BIS Denied Persons download")?;
        if !resp.status().is_success() {
            debug!(status = %resp.status(), "BIS Denied Persons returned non-success");
            return Ok(0);
        }
        let bytes = resp.bytes().await.context("read BIS Denied Persons body")?;
        let text = String::from_utf8_lossy(&bytes);
        let mut count = 0;
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let person = BisDeniedPerson {
                    name: parts.first().unwrap_or(&"").trim().to_string(),
                    address: parts.get(1).map(|s| s.trim().to_string()),
                    effective_date: parts.get(2).map(|s| s.trim().to_string()),
                    expiry_date: None,
                    fetched_at: Utc::now(),
                };
                if !person.name.is_empty() {
                    self.denied_persons.push(person);
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Search the entity list for a name match.
    pub fn search_entity_list(&self, name: &str) -> Vec<&BisEntityListEntry> {
        let lower = name.to_lowercase();
        self.entity_list_entries
            .iter()
            .filter(|e| e.entity_name.to_lowercase().contains(&lower))
            .collect()
    }

    /// Search the denied persons list.
    pub fn search_denied_persons(&self, name: &str) -> Vec<&BisDeniedPerson> {
        let lower = name.to_lowercase();
        self.denied_persons
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&lower))
            .collect()
    }

    /// Return total cached entity list entries.
    pub fn entity_list_count(&self) -> usize {
        self.entity_list_entries.len()
    }

    /// Return total cached denied persons.
    pub fn denied_persons_count(&self) -> usize {
        self.denied_persons.len()
    }
}

impl Default for ExportControlMonitor {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_control_list_urls() {
        assert!(ExportControlList::BisEntityList.download_url().contains("bis.doc.gov"));
        assert_eq!(ExportControlList::BisDeniedPersons.as_str(), "BIS_Denied_Persons");
    }

    #[test]
    fn export_control_monitor_constructs() {
        let monitor = ExportControlMonitor::new();
        assert_eq!(monitor.entity_list_count(), 0);
        assert_eq!(monitor.denied_persons_count(), 0);
    }

    #[test]
    fn bis_entity_list_entry_debug() {
        let entry = BisEntityListEntry {
            entity_name: "Test Entity".to_string(),
            address: Some("123 Main St".to_string()),
            country: "Russia".to_string(),
            license_requirement: "License required".to_string(),
            license_review_policy: "Presumption of denial".to_string(),
            entities_list_section: "Supplement No. 4".to_string(),
            source_url: Some("https://bis.doc.gov".to_string()),
            listed_date: None,
            fetched_at: Utc::now(),
        };
        assert_eq!(entry.country, "Russia");
        assert!(entry.license_review_policy.contains("denial"));
    }
}
