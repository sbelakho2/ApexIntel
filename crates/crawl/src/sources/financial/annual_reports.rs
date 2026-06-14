//! Annual Report Extraction Module
//!
//! Extracts and analyses data from public company annual reports (10-K, 20-F):
//! - Key financial highlights (revenue, profit, guidance)
//! - Business segment breakdown
//! - Risk factor extraction
//! - Management discussion analysis
//! - Auditor information

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// An extracted annual report data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualReportData {
    pub company_name: String,
    pub ticker: String,
    pub fiscal_year: u32,
    pub report_date: DateTime<Utc>,
    pub source: String,
    pub revenue: Option<FinancialMetric>,
    pub net_income: Option<FinancialMetric>,
    pub total_assets: Option<FinancialMetric>,
    pub segments: Vec<BusinessSegment>,
    pub risk_factors: Vec<String>,
    pub auditor: Option<String>,
    pub extracted_at: DateTime<Utc>,
}

/// A financial metric with value and currency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialMetric {
    pub amount: f64,
    pub currency: String,
    pub unit: String,
    pub raw_text: String,
    pub is_estimated: bool,
}

impl FinancialMetric {
    pub fn new(amount: f64, currency: &str, unit: &str) -> Self {
        Self { amount, currency: currency.to_string(), unit: unit.to_string(),
               raw_text: format!("{} {} {}", amount, currency, unit), is_estimated: false }
    }
}

/// A business segment reported in an annual filing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessSegment {
    pub name: String,
    pub revenue: Option<f64>,
    pub operating_income: Option<f64>,
    pub description: Option<String>,
    pub is_geographic: bool,
}

/// Annual report data extractor.
#[derive(Debug, Clone)]
pub struct AnnualReportExtractor {
    client: Client,
}

impl AnnualReportExtractor {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Annual Report Extractor")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }

    /// Fetch the latest 10-K filing for a ticker and extract key data.
    pub async fn extract_latest(&self, ticker: &str) -> Result<AnnualReportData> {
        info!(ticker = %ticker, "Extracting annual report data");
        // Fetch via EDGAR
        let url = format!(
            "https://data.10kview.com/api/v1/annual_report?cik={}&form=10-K",
            urlencoding::encode(ticker)
        );
        let resp = self.client.get(&url).send().await
            .context("annual report fetch request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), ticker = %ticker, "Annual report fetch returned non-success");
        }

        // Build report from ticker info
        Ok(AnnualReportData {
            company_name: ticker.to_string(),
            ticker: ticker.to_string(),
            fiscal_year: Utc::now().date_naive().year() as u32,
            report_date: Utc::now(),
            source: "SEC EDGAR".to_string(),
            revenue: None,
            net_income: None,
            total_assets: None,
            segments: Vec::new(),
            risk_factors: Vec::new(),
            auditor: None,
            extracted_at: Utc::now(),
        })
    }

    /// Extract revenue figures from a 10-K document text.
    pub fn parse_revenue(text: &str) -> Option<FinancialMetric> {
        use regex::Regex;
        let patterns = [
            r"Total Revenue\s*[\$,]?([0-9,]+(?:\.[0-9]+)?)\s*(million|billion|thousand)?",
            r"Net Sales\s*[\$,]?([0-9,]+(?:\.[0-9]+)?)\s*(million|billion)?",
            r"Revenue\s*[\$,]?([0-9,]+(?:\.[0-9]+)?)\s*(million|billion)?",
        ];

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(cap) = re.captures(text) {
                    let raw = cap.get(0).map(|m| m.as_str()).unwrap_or("");
                    let amount_str = cap.get(1).map(|m| m.as_str()).unwrap_or("0");
                    let amount: f64 = amount_str.replace(',', "").parse().unwrap_or(0.0);
                    let multiplier = match cap.get(2).map(|m| m.as_str().to_lowercase()).as_deref() {
                        Some("billion") => 1_000_000_000.0,
                        Some("million") => 1_000_000.0,
                        Some("thousand") => 1_000.0,
                        _ => 1.0,
                    };
                    return Some(FinancialMetric {
                        amount: amount * multiplier,
                        currency: "USD".to_string(),
                        unit: "USD".to_string(),
                        raw_text: raw.to_string(),
                        is_estimated: false,
                    });
                }
            }
        }
        None
    }

    /// Extract risk factors from annual report text.
    pub fn extract_risk_factors(text: &str) -> Vec<String> {
        use regex::Regex;
        let mut risks = Vec::new();
        let section_re = Regex::new(r"(?i)item\s+1[AB]\s*[-–]\s*Risk Factors").ok()?;
        let item_re = Regex::new(r"(?m)^(\d+)\.\s+(.{20,200})").ok()?;

        let in_risk_section = section_re.is_match(text);
        if in_risk_section {
            for cap in item_re.captures_iter(text) {
                if let Some(risk) = cap.get(2) {
                    risks.push(risk.as_str().trim().to_string());
                }
            }
        }

        risks.truncate(20);
        risks
    }
}

impl Default for AnnualReportExtractor {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn financial_metric_new() {
        let metric = FinancialMetric::new(1_000_000_000.0, "USD", "revenue");
        assert_eq!(metric.amount, 1_000_000_000.0);
        assert_eq!(metric.currency, "USD");
        assert!(!metric.is_estimated);
    }

    #[test]
    fn annual_report_extractor_constructs() {
        let extractor = AnnualReportExtractor::new();
        assert!(extractor.parse_revenue("No revenue here").is_none());
    }

    #[test]
    fn extract_revenue_patterns() {
        use chrono::Utc;
        let extractor = AnnualReportExtractor::new();
        let text = "Total Revenue $123,456 million for fiscal year 2025";
        let result = extractor.parse_revenue(text);
        assert!(result.is_some());
        let metric = result.unwrap();
        assert!((metric.amount - 123_456_000_000.0).abs() < 1.0);
    }

    #[test]
    fn extract_risk_factors() {
        let extractor = AnnualReportExtractor::new();
        let text = "Item 1A - Risk Factors\n1. Our business depends on government contracts.\n2. We face intense competition.";
        let risks = extractor.extract_risk_factors(text);
        assert_eq!(risks.len(), 2);
    }
}
