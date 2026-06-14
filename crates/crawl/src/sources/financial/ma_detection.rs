//! M&A Activity Detection Module
//!
//! Detects merger & acquisition activity from multiple sources:
//! - SEC EDGAR filings (8-K, SC 13D, SC 14D)
//! - Press release RSS feeds
//! - Crunchbase news
//! - Bloomberg terminal data (if available)
//! - Corporate registry changes
//!
//! # Intelligence Use Cases
//! - Target identification (who is being acquired)
//! - Acquirer detection (who is buying)
//! - Deal size estimation
//! - Strategic intent inference

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

/// A detected M&A event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaEvent {
    pub deal_id: String,
    pub deal_type: MaDealType,
    pub target_name: String,
    pub target_ticker: Option<String>,
    pub target_cik: Option<String>,
    pub acquirer_name: Option<String>,
    pub acquirer_ticker: Option<String>,
    pub announcement_date: Option<NaiveDate>,
    pub estimated_value: Option<f64>,
    pub currency: Option<String>,
    pub status: MaStatus,
    pub source: String,
    pub source_url: Option<String>,
    pub headline: String,
    pub description: Option<String>,
    pub detected_at: DateTime<Utc>,
    pub confidence: MaConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaDealType {
    Acquisition,
    Merger,
    JointVenture,
    MinorityInvestment,
    Divestiture,
    SpinOff,
    LBO,
    IPO,
    Unknown,
}

impl MaDealType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acquisition => "acquisition",
            Self::Merger => "merger",
            Self::JointVenture => "joint_venture",
            Self::MinorityInvestment => "minority_investment",
            Self::Divestiture => "divestiture",
            Self::SpinOff => "spin_off",
            Self::LBO => "lbo",
            Self::IPO => "ipo",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_headline(headline: &str) -> Self {
        let lower = headline.to_lowercase();
        if lower.contains("acquires") || lower.contains("acquisition") || lower.contains("acquire") {
            Self::Acquisition
        } else if lower.contains("merger") || lower.contains("merges") || lower.contains("merged") {
            Self::Merger
        } else if lower.contains("joint venture") || lower.contains("jv") {
            Self::JointVenture
        } else if lower.contains("invests") || lower.contains("minority") || lower.contains("stake") {
            Self::MinorityInvestment
        } else if lower.contains("divest") || lower.contains("spins off") || lower.contains("spinoff") {
            Self::Divestiture
        } else if lower.contains("spin off") {
            Self::SpinOff
        } else if lower.contains("lbo") || lower.contains("leveraged buyout") {
            Self::LBO
        } else if lower.contains("ipo") || lower.contains("going public") || lower.contains("listing") {
            Self::IPO
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaStatus {
    Announced,
    Pending,
    Completed,
    Withdrawn,
    Rumored,
    Unknown,
}

impl MaStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announced => "announced",
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Withdrawn => "withdrawn",
            Self::Rumored => "rumored",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaConfidence {
    Low,
    Medium,
    High,
}

impl MaConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// M&A monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaMonitorConfig {
    pub tickers: Vec<String>,
    pub keywords: Vec<String>,
    pub sources: Vec<MaSource>,
    pub min_deal_value: Option<f64>,
    pub max_age_days: u32,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaSource {
    SecEdgar,
    Crunchbase,
    PrNews,
    Bloomberg,
}

impl MaSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecEdgar => "sec_edgar",
            Self::Crunchbase => "crunchbase",
            Self::PrNews => "pr_news",
            Self::Bloomberg => "bloomberg",
        }
    }
}

impl Default for MaMonitorConfig {
    fn default() -> Self {
        Self {
            tickers: Vec::new(),
            keywords: vec![
                "acquires".to_string(), "acquisition".to_string(),
                "merger".to_string(), "merges".to_string(),
                "invests".to_string(), "stake".to_string(),
                "buyout".to_string(), "takeover".to_string(),
            ],
            sources: vec![MaSource::SecEdgar, MaSource::Crunchbase, MaSource::PrNews],
            min_deal_value: None,
            max_age_days: 365,
            timeout_secs: 30,
        }
    }
}

impl MaMonitorConfig {
    pub fn add_ticker(mut self, ticker: impl Into<String>) -> Self {
        self.tickers.push(ticker.into()); self
    }

    pub fn add_keyword(mut self, kw: impl Into<String>) -> Self {
        self.keywords.push(kw.into()); self
    }
}

/// M&A activity monitor.
#[derive(Debug, Clone)]
pub struct MaMonitor {
    client: Client,
    config: MaMonitorConfig,
    /// Detected events cache.
    events: Vec<MaEvent>,
}

impl MaMonitor {
    pub fn new(config: MaMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) M&A Monitor")
            .build()
            .context("building M&A monitor HTTP client")?;
        Ok(Self { client, config, events: Vec::new() })
    }

    /// Scan Crunchbase news for M&A mentions.
    pub async fn scan_crunchbase(&self, keywords: &[String]) -> Result<Vec<MaEvent>> {
        let url = "https://news.crunchbase.com/feed/";
        let resp = self.client.get(url).send().await.context("Crunchbase news request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "Crunchbase returned non-success");
            return Ok(Vec::new());
        }

        let body = resp.text().await.context("read Crunchbase feed")?;
        let items = crate::rss::parse_feed(&body).unwrap_or_default();

        let events: Vec<MaEvent> = items.iter().filter_map(|item| {
            let combined = format!("{} {}", item.title, item.description);
            let lower = combined.to_lowercase();

            let matched: Vec<String> = keywords.iter()
                .filter(|kw| lower.contains(&kw.to_lowercase()))
                .cloned()
                .collect();

            if matched.is_empty() {
                return None;
            }

            let deal_type = MaDealType::from_headline(&item.title);
            let deal_id = format!("cb-{}-{}", item.guid.chars().take(12).collect::<String>(), Utc::now().timestamp());

            Some(MaEvent {
                deal_id,
                deal_type,
                target_name: item.title.split("acquires").nth(1)
                    .or_else(|| item.title.split("merges with").nth(1))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| item.title.clone()),
                target_ticker: None,
                target_cik: None,
                acquirer_name: item.title.split("acquires").nth(0)
                    .or_else(|| item.title.split("merges with").nth(0))
                    .map(|s| s.trim().to_string()),
                acquirer_ticker: None,
                announcement_date: item.published.map(|dt| dt.date_naive()),
                estimated_value: None,
                currency: None,
                status: MaStatus::Announced,
                source: "crunchbase".to_string(),
                source_url: Some(item.link.clone()),
                headline: item.title.clone(),
                description: Some(item.description.clone()),
                detected_at: Utc::now(),
                confidence: MaConfidence::Medium,
            })
        }).collect();

        debug!(count = events.len(), "Crunchbase M&A scan complete");
        Ok(events)
    }

    /// Parse M&A keywords from news text.
    pub fn detect_ma_in_text(&self, text: &str) -> Vec<MaEvent> {
        let mut events = Vec::new();
        let lower = text.to_lowercase();

        for kw in &self.config.keywords {
            if lower.contains(&kw.to_lowercase()) {
                let deal_type = MaDealType::from_headline(&lower);
                events.push(MaEvent {
                    deal_id: format!("txt-{}-{}", kw, Utc::now().timestamp()),
                    deal_type,
                    target_name: "Extracted from text".to_string(),
                    target_ticker: None,
                    target_cik: None,
                    acquirer_name: None,
                    acquirer_ticker: None,
                    announcement_date: None,
                    estimated_value: None,
                    currency: None,
                    status: MaStatus::Rumored,
                    source: "text_analysis".to_string(),
                    source_url: None,
                    headline: format!("M&A keyword detected: {}", kw),
                    description: Some(text.chars().take(200).collect()),
                    detected_at: Utc::now(),
                    confidence: MaConfidence::Low,
                });
            }
        }

        events
    }

    /// Get all detected events.
    pub fn all_events(&self) -> &[MaEvent] {
        &self.events
    }

    /// Return events by deal type.
    pub fn events_by_type(&self, deal_type: MaDealType) -> Vec<&MaEvent> {
        self.events.iter().filter(|e| e.deal_type == deal_type).collect()
    }

    /// Return events by status.
    pub fn events_by_status(&self, status: MaStatus) -> Vec<&MaEvent> {
        self.events.iter().filter(|e| e.status == status).collect()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn ma_deal_type_from_headline() {
        assert_eq!(MaDealType::from_headline("Company A acquires Company B"), MaDealType::Acquisition);
        assert_eq!(MaDealType::from_headline("ABC merges with XYZ"), MaDealType::Merger);
        assert_eq!(MaDealType::from_headline("Firm invests in startup"), MaDealType::MinorityInvestment);
    }

    #[test]
    fn ma_deal_type_as_str() {
        assert_eq!(MaDealType::Acquisition.as_str(), "acquisition");
        assert_eq!(MaDealType::LBO.as_str(), "lbo");
    }

    #[test]
    fn ma_status_as_str() {
        assert_eq!(MaStatus::Announced.as_str(), "announced");
        assert_eq!(MaStatus::Completed.as_str(), "completed");
    }

    #[test]
    fn ma_monitor_constructs() {
        let result = MaMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn ma_monitor_config_chaining() {
        let cfg = MaMonitorConfig::default()
            .add_ticker("AAPL")
            .add_keyword("acquires");
        assert_eq!(cfg.tickers.len(), 1);
        assert_eq!(cfg.keywords.len(), 9); // default 8 + added 1
    }

    #[test]
    fn ma_event_debug() {
        let event = MaEvent {
            deal_id: "test-deal-001".to_string(),
            deal_type: MaDealType::Acquisition,
            target_name: "Target Corp".to_string(),
            target_ticker: Some("TGT".to_string()),
            target_cik: None,
            acquirer_name: Some("Acquirer Inc".to_string()),
            acquirer_ticker: Some("ACQ".to_string()),
            announcement_date: Some(Utc::now().date_naive()),
            estimated_value: Some(5_000_000_000.0),
            currency: Some("USD".to_string()),
            status: MaStatus::Announced,
            source: "sec_edgar".to_string(),
            source_url: Some("https://sec.gov".to_string()),
            headline: "Acquirer Inc acquires Target Corp for $5B".to_string(),
            description: None,
            detected_at: Utc::now(),
            confidence: MaConfidence::High,
        };
        assert_eq!(event.deal_type.as_str(), "acquisition");
        assert_eq!(event.estimated_value.unwrap() / 1e9, 5.0);
        assert_eq!(event.confidence.as_str(), "high");
    }

    #[test]
    fn detect_ma_in_text() {
        let monitor = MaMonitor::new(MaMonitorConfig::default()).unwrap();
        let events = monitor.detect_ma_in_text("Elbit Systems acquires a small defense contractor");
        assert!(!events.is_empty());
        assert_eq!(events[0].deal_type, MaDealType::Acquisition);
    }
}
