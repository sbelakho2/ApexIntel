//! Government Tender Portal Module

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A tender/contract opportunity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tender {
    pub tender_id: String,
    pub title: String,
    pub description: String,
    pub buyer_name: String,
    pub buyer_country: String,
    pub estimated_value: Option<f64>,
    pub currency: Option<String>,
    pub publication_date: Option<NaiveDate>,
    pub submission_deadline: Option<NaiveDate>,
    pub contract_type: ContractType,
    pub source: TenderSource,
    pub source_url: String,
    pub keywords_matched: Vec<String>,
    pub is_defense_related: bool,
    pub is_relevant: bool,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractType {
    Supply, Services, Works, Framework, Concession, Unknown,
}

impl ContractType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supply => "supply", Self::Services => "services",
            Self::Works => "works", Self::Framework => "framework",
            Self::Concession => "concession", Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TenderSource {
    SamGov, TedEu, FindATenderUk, WorldBank, UnProcurement,
}

impl TenderSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SamGov => "sam_gov", Self::TedEu => "ted_eu",
            Self::FindATenderUk => "find_a_tender_uk", Self::WorldBank => "world_bank",
            Self::UnProcurement => "un_procurement",
        }
    }
    pub fn display_name(self) -> &'static str {
        match self {
            Self::SamGov => "US SAM.gov", Self::TedEu => "EU TED",
            Self::FindATenderUk => "UK Find a Tender", Self::WorldBank => "World Bank",
            Self::UnProcurement => "UN Procurement",
        }
    }
}

/// Tender search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenderMonitorConfig {
    pub keywords: Vec<String>,
    pub countries: Vec<String>,
    pub min_value: Option<f64>,
    pub contract_types: Vec<ContractType>,
    pub include_defense: bool,
    pub max_results: u32,
    pub timeout_secs: u64,
}

impl Default for TenderMonitorConfig {
    fn default() -> Self {
        Self {
            keywords: vec!["defense".to_string(), "aerospace".to_string(), "radar".to_string(),
                          "surveillance".to_string(), "electronics".to_string()],
            countries: vec!["US".to_string(), "EU".to_string(), "UK".to_string()],
            min_value: None,
            contract_types: vec![ContractType::Supply, ContractType::Services],
            include_defense: true,
            max_results: 100,
            timeout_secs: 30,
        }
    }
}

impl TenderMonitorConfig {
    pub fn add_keyword(mut self, kw: impl Into<String>) -> Self {
        self.keywords.push(kw.into()); self
    }
    pub fn add_country(mut self, c: impl Into<String>) -> Self {
        self.countries.push(c.into()); self
    }
}

/// Tender portal monitor.
#[derive(Debug, Clone)]
pub struct TenderMonitor {
    client: Client,
    config: TenderMonitorConfig,
}

impl TenderMonitor {
    pub fn new(config: TenderMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Tender Monitor")
            .build()
            .context("building Tender monitor HTTP client")?;
        Ok(Self { client, config })
    }

    pub async fn scan(&self) -> Vec<Tender> {
        let mut all_tenders = Vec::new();

        match self.scan_sam_gov().await {
            Ok(tenders) => { debug!(source = "sam_gov", count = tenders.len(), "SAM.gov scan complete"); all_tenders.extend(tenders); }
            Err(e) => warn!(error = %e, "SAM.gov scan failed"),
        }

        match self.scan_ted_eu().await {
            Ok(tenders) => { debug!(source = "ted_eu", count = tenders.len(), "TED EU scan complete"); all_tenders.extend(tenders); }
            Err(e) => warn!(error = %e, "TED EU scan failed"),
        }

        all_tenders.sort_by_key(|t| std::cmp::Reverse(t.publication_date));
        info!(total = all_tenders.len(), "Tender monitoring scan complete");
        all_tenders
    }

    async fn scan_sam_gov(&self) -> Result<Vec<Tender>> {
        let keywords = self.config.keywords.join(" OR ");
        let url = "https://sam.gov/api/prod/sgs/v1/search/";
        let params = [("q", &keywords), ("page", &"0".to_string()), ("size", &self.config.max_results.to_string())];

        let resp = self.client.get(url).query(&params).send().await.context("SAM.gov API request")?;
        if !resp.status().is_success() { debug!(status = %resp.status(), "SAM.gov returned non-success"); return Ok(Vec::new()); }

        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let items = json.get("response_data").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        let tenders = items.iter().filter_map(|v| self.parse_sam_gov_tender(v)).collect();
        Ok(tenders)
    }

    fn parse_sam_gov_tender(&self, value: &serde_json::Value) -> Option<Tender> {
        let tender_id = value["noticeId"].as_str().or(value["uiId"].as_str())?.to_string();
        let title = value["title"].as_str()?.to_string();
        let description = value["description"].as_str().unwrap_or("").to_string();
        let keywords_matched: Vec<String> = self.config.keywords.iter()
            .filter(|kw| title.to_lowercase().contains(&kw.to_lowercase())
                     || description.to_lowercase().contains(&kw.to_lowercase()))
            .cloned()
            .collect();
        let is_relevant = !keywords_matched.is_empty();
        let source_url = format!("https://sam.gov/opp/{}/view", tender_id);

        Some(Tender {
            tender_id,
            title,
            description,
            buyer_name: value["agencyName"].as_str().unwrap_or("Unknown").to_string(),
            buyer_country: "US".to_string(),
            estimated_value: None,
            currency: Some("USD".to_string()),
            publication_date: None,
            submission_deadline: None,
            contract_type: ContractType::Supply,
            source: TenderSource::SamGov,
            source_url,
            keywords_matched,
            is_defense_related: false,
            is_relevant,
            fetched_at: Utc::now(),
        })
    }

    async fn scan_ted_eu(&self) -> Result<Vec<Tender>> {
        let url = "https://ted.europa.eu/api/v1/search/notice";
        let params = [("q", &self.config.keywords.join(" ")), ("limit", &self.config.max_results.to_string())];

        let resp = self.client.get(url).query(&params).send().await.context("TED EU API request")?;
        if !resp.status().is_success() { debug!(status = %resp.status(), "TED EU returned non-success"); return Ok(Vec::new()); }

        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let notices = json.get("result").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        let tenders = notices.iter().filter_map(|n| {
            let tender_id = n["id"].as_str()?.to_string();
            let title = n["title"].as_str().unwrap_or("").to_string();
            let keywords_matched: Vec<String> = self.config.keywords.iter()
                .filter(|kw| title.to_lowercase().contains(&kw.to_lowercase())).cloned().collect();
            let is_relevant = !keywords_matched.is_empty();
            let source_url = format!("https://ted.europa.eu/en/notice/{}", tender_id);

            Some(Tender {
                tender_id,
                title,
                description: n["name"].as_str().unwrap_or("").to_string(),
                buyer_name: "EU Buyer".to_string(),
                buyer_country: n["country"].as_str().unwrap_or("EU").to_string(),
                estimated_value: None,
                currency: Some("EUR".to_string()),
                publication_date: None,
                submission_deadline: None,
                contract_type: ContractType::Services,
                source: TenderSource::TedEu,
                source_url,
                keywords_matched,
                is_defense_related: false,
                is_relevant,
                fetched_at: Utc::now(),
            })
        }).collect();
        Ok(tenders)
    }

    pub fn relevant_tenders<'a>(&self, tenders: &'a [Tender]) -> Vec<&'a Tender> {
        tenders.iter().filter(|t| t.is_relevant).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tender_monitor_config_defaults() {
        let cfg = TenderMonitorConfig::default();
        assert!(!cfg.keywords.is_empty());
        assert!(cfg.include_defense);
    }

    #[test]
    fn tender_monitor_chaining() {
        let cfg = TenderMonitorConfig::default().add_keyword("cyber").add_country("DE");
        assert!(cfg.keywords.contains(&"cyber".to_string()));
        assert!(cfg.countries.contains(&"DE".to_string()));
    }

    #[test]
    fn tender_monitor_constructs() {
        let result = TenderMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn tender_source_display() {
        assert_eq!(TenderSource::SamGov.display_name(), "US SAM.gov");
        assert_eq!(TenderSource::TedEu.as_str(), "ted_eu");
    }

    #[test]
    fn contract_type_as_str() {
        assert_eq!(ContractType::Supply.as_str(), "supply");
        assert_eq!(ContractType::Works.as_str(), "works");
    }
}
