//! SEC EDGAR Filings Monitoring Module
//!
//! Monitors SEC EDGAR for regulatory filings relevant to tracked entities:
//! - 8-K (material events)
//! - 10-K/10-Q (annual/quarterly reports)
//! - DEF 14A (proxy statements)
//! - Form 4 (insider transactions)
//! - SF-1/S-3 (stock offerings)
//!
//! Uses the EDGAR full-text search API and CIK lookup.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// A filing discovered via EDGAR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgarFiling {
    pub accession_number: String,
    pub cik: String,
    pub company_name: String,
    pub form_type: String,
    pub filing_date: NaiveDate,
    pub acceptance_datetime: DateTime<Utc>,
    pub document_url: String,
    pub description: Option<String>,
    pub is_recent: bool,
    pub fetched_at: DateTime<Utc>,
}

impl EdgarFiling {
    /// Whether this filing is a material event (8-K).
    pub fn is_material_event(&self) -> bool {
        self.form_type == "8-K"
    }

    /// Whether this is an insider trading report (Form 4).
    pub fn is_insider_transaction(&self) -> bool {
        self.form_type == "4"
    }

    /// Whether this is a proxy filing.
    pub fn is_proxy(&self) -> bool {
        self.form_type.starts_with("DEF") || self.form_type.starts_with("PR")
    }
}

/// EDGAR filing type filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgarFilingType {
    All,
    Annual10K,
    Quarterly10Q,
    Current8K,
    ProxyDef14A,
    Form4Insider,
    S1IPO,
}

impl EdgarFilingType {
    pub fn filter_string(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Annual10K => "10-K",
            Self::Quarterly10Q => "10-Q",
            Self::Current8K => "8-K",
            Self::ProxyDef14A => "DEF 14A",
            Self::Form4Insider => "4",
            Self::S1IPO => "S-1",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Annual10K => "annual_10k",
            Self::Quarterly10Q => "quarterly_10q",
            Self::Current8K => "current_8k",
            Self::ProxyDef14A => "proxy_def14a",
            Self::Form4Insider => "form4_insider",
            Self::S1IPO => "s1_ipo",
        }
    }
}

/// EDGAR monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgarMonitorConfig {
    /// Company tickers or CIK numbers to monitor.
    pub tickers: Vec<String>,
    /// Filing types to include.
    pub filing_types: Vec<EdgarFilingType>,
    /// Start date for filing search.
    pub start_date: Option<NaiveDate>,
    /// End date for filing search.
    pub end_date: Option<NaiveDate>,
    /// Maximum results per company.
    pub max_results: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for EdgarMonitorConfig {
    fn default() -> Self {
        Self {
            tickers: Vec::new(),
            filing_types: vec![EdgarFilingType::All],
            start_date: None,
            end_date: None,
            max_results: 40,
            timeout_secs: 30,
        }
    }
}

impl EdgarMonitorConfig {
    pub fn add_ticker(mut self, ticker: impl Into<String>) -> Self {
        self.tickers.push(ticker.into()); self
    }
}

/// SEC EDGAR filings monitor.
#[derive(Debug, Clone)]
pub struct EdgarMonitor {
    client: Client,
    config: EdgarMonitorConfig,
}

impl EdgarMonitor {
    pub fn new(config: EdgarMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (compliance@apexintel.io) EDGAR Monitor")
            .build()
            .context("building EDGAR HTTP client")?;
        Ok(Self { client, config })
    }

    /// Search filings for a ticker.
    pub async fn search_filings(&self, ticker: &str) -> Result<Vec<EdgarFiling>> {
        let filter = self.config.filing_types.iter()
            .map(|t| t.filter_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("+OR+");

        let date_range = match (self.config.start_date, self.config.end_date) {
            (Some(start), Some(end)) => format!("&startdt={}&enddt={}", start, end),
            _ => String::new(),
        };

        let url = format!(
            "https://efts.sec.gov/LATEST/search-index?q=%22{}%22&dateRange=custom{}&category=form-type{}",
            urlencoding::encode(ticker),
            date_range,
            if filter.is_empty() { String::new() } else { format!("&forms={}", filter) }
        );

        let resp = self.client.get(&url).send().await.context("EDGAR search request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), ticker = %ticker, "EDGAR returned non-success");
            return Ok(Vec::new());
        }

        let text = resp.text().await.context("read EDGAR response")?;
        self.parse_filings(&text, ticker)
    }

    fn parse_filings(&self, json: &str, ticker: &str) -> Result<Vec<EdgarFiling>> {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct EdgarSearchResponse {
            #[serde(rename = "hits")]
            hits: Option<EdgarHits>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct EdgarHits {
            total: Option<i64>,
            #[serde(rename = "hits")]
            items: Option<Vec<EdgarHit>>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct EdgarHit {
            #[serde(rename = "_id")]
            id: Option<String>,
            #[serde(rename = "_source")]
            source: Option<EdgarHitSource>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct EdgarHitSource {
            #[serde(rename = "accessionNumber")]
            accession: Option<String>,
            cik: Option<String>,
            #[serde(rename = "displayName")]
            company_name: Option<String>,
            #[serde(rename = "form")]
            form_type: Option<String>,
            #[serde(rename = "filedAt")]
            filed_at: Option<String>,
            #[serde(rename = "description")]
            description: Option<String>,
        }

        let search_resp: EdgarSearchResponse = serde_json::from_str(json)
            .context("parse EDGAR JSON response")?;

        let hits = search_resp.hits;
        let items = match hits {
            Some(h) => h.items.unwrap_or_default(),
            None => return Ok(Vec::new()),
        };

        let now = Utc::now();
        let filings: Vec<EdgarFiling> = items.into_iter().filter_map(|hit| {
            let source = hit.source?;
            let accession = source.accession.clone().unwrap_or_default();
            let cik = source.cik.clone().unwrap_or_default();
            let company_name = source.company_name.clone().unwrap_or_else(|| ticker.to_string());
            let form_type = source.form_type.clone().unwrap_or_default();
            let filed_at = source.filed_at.as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);

            Some(EdgarFiling {
                accession_number: accession.clone(),
                cik: cik.clone(),
                company_name,
                form_type: form_type.clone(),
                filing_date: filed_at.date_naive(),
                acceptance_datetime: filed_at,
                document_url: format!(
                    "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={}&type={}&dateb=&owner=include&count={}",
                    cik,
                    form_type.clone(),
                    self.config.max_results
                ),
                description: source.description,
                is_recent: (now - filed_at).num_days() <= 7,
                fetched_at: now,
            })
        }).collect();

        debug!(ticker = %ticker, count = filings.len(), "EDGAR filings parsed");
        Ok(filings)
    }

    /// Monitor all configured tickers.
    pub async fn full_scan(&self) -> Vec<EdgarFiling> {
        let mut all_filings = Vec::new();
        for ticker in &self.config.tickers {
            match self.search_filings(ticker).await {
                Ok(filings) => all_filings.extend(filings),
                Err(e) => warn!(ticker = %ticker, error = %e, "EDGAR ticker scan failed"),
            }
        }
        all_filings.sort_by_key(|f| std::cmp::Reverse(f.filing_date));
        info!(total = all_filings.len(), "EDGAR full scan complete");
        all_filings
    }

    /// Get material events (8-K) from filings.
    pub fn material_events(filings: &[EdgarFiling]) -> Vec<&EdgarFiling> {
        filings.iter().filter(|f| f.is_material_event()).collect()
    }

    /// Get insider transactions (Form 4) from filings.
    pub fn insider_transactions(filings: &[EdgarFiling]) -> Vec<&EdgarFiling> {
        filings.iter().filter(|f| f.is_insider_transaction()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edgar_filing_type_filter() {
        assert_eq!(EdgarFilingType::Annual10K.filter_string(), "10-K");
        assert_eq!(EdgarFilingType::Form4Insider.as_str(), "form4_insider");
    }

    #[test]
    fn edgar_filing_is_material_event() {
        let filing = EdgarFiling {
            accession_number: "0001234567-22-001234".to_string(),
            cik: "0001234567".to_string(),
            company_name: "Test Corp".to_string(),
            form_type: "8-K".to_string(),
            filing_date: Utc::now().date_naive(),
            acceptance_datetime: Utc::now(),
            document_url: "https://sec.gov".to_string(),
            description: None,
            is_recent: true,
            fetched_at: Utc::now(),
        };
        assert!(filing.is_material_event());
        assert!(!filing.is_insider_transaction());
    }

    #[test]
    fn edgar_monitor_constructs() {
        let result = EdgarMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn edgar_monitor_config_chaining() {
        let cfg = EdgarMonitorConfig::default()
            .add_ticker("AAPL")
            .add_ticker("GOOGL");
        assert_eq!(cfg.tickers.len(), 2);
    }
}
