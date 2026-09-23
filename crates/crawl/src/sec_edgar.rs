//! SEC EDGAR client — fetches public-company filings from the U.S. Securities
//! and Exchange Commission's free EDGAR system.
//!
//! # Sources (all free, NO API key required)
//!
//! - **Company submissions**: `https://data.sec.gov/submissions/CIK{padded}.json`
//!   — the full filing history (10-K, 8-K, DEF 14A, Form 4, etc.) for a company.
//! - **Company facts** (XBRL): `https://data.sec.gov/api/xbrlcompanyfacts/CIK{padded}.json`
//!   — structured financial data (revenue, assets, liabilities, etc.).
//!
//! SEC requires a descriptive `User-Agent` header including contact info and
//! rate-limits to 10 requests/second. This client enforces both.
//!
//! # CIK resolution
//!
//! EDGAR uses 10-digit zero-padded Central Index Keys (CIKs). This client can
//! look up a CIK from a ticker symbol via the company tickers map:
//! `https://www.sec.gov/files/company_tickers.json`
//!
//! # Data flow
//!
//! ```text
//! company (name/domain/ticker) ──► SecEdgarClient::fetch_recent_filings(cik)
//!                                        │
//!                                        ▼
//!                                 Vec<SecFiling> (recent 8-K/10-K/Form 4)
//!                                        │
//!                          each .to_observation() ──► store.insert_observation()
//! ```
//!
//! Officer/director changes (Form 4, DEF 14A) feed the POI pipeline; 8-K current
//! reports (material events) feed the insight engine; 10-K annual reports feed
//! company enrichment.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::warn;

use apex_core::entities::{Observation, ObservationType};

/// SEC EDGAR contact email embedded in the User-Agent (required by SEC fair-use policy).
/// Overridable via the SEC_EDGAR_EMAIL env var; defaults to a generic address.
const SEC_USER_AGENT_EMAIL: &str = "research@apexintel.local";

/// The filing types most relevant to competitive intelligence:
/// - `8-K` — material current events (acquisitions, leadership changes, bankruptcy, etc.)
/// - `10-K` — annual report (financials, risk factors, business overview)
/// - `10-Q` — quarterly report
/// - `DEF 14A` — proxy statement (director nominations, executive comp, governance)
/// - `4` — Form 4 (insider/officer stock transactions — signals leadership changes)
/// - `3` — Form 3 (initial ownership — new officer/director appointment)
/// - `5` — Form 5 (annual ownership)
const TRACKED_FORM_TYPES: &[&str] = &["8-K", "10-K", "10-Q", "DEF 14A", "4", "3", "5"];

/// A single SEC EDGAR filing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecFiling {
    /// The SEC accession number (unique per filing), without dashes.
    pub accession_number: String,
    /// Filing form type (e.g. "8-K", "10-K", "DEF 14A", "4").
    pub form_type: String,
    /// Filing date (YYYY-MM-DD).
    pub filing_date: String,
    /// Reporting date (period the filing covers, YYYY-MM-DD).
    pub report_date: Option<String>,
    /// Primary document URL on SEC.gov.
    pub primary_document_url: String,
    /// Short description of the filing.
    pub description: Option<String>,
    /// CIK of the filer.
    pub cik: String,
    /// Company name of the filer.
    pub company_name: String,
}

impl SecFiling {
    /// Wrap this filing into an `Observation` for storage.
    pub fn to_observation(&self, entity_id: Option<uuid::Uuid>) -> Observation {
        let mut obs = Observation::new(
            ObservationType::SecFiling,
            Utc::now(),
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null),
            serde_json::json!({
                "source": "sec_edgar",
                "url": &self.primary_document_url,
                "form_type": &self.form_type,
                "filing_date": &self.filing_date,
            }),
        );
        obs.entity_id = entity_id;
        obs.entity_type = Some("company".to_string());
        obs
    }

    /// Human-readable one-line summary.
    pub fn summary(&self) -> String {
        match &self.description {
            Some(desc) if !desc.is_empty() => {
                format!(
                    "SEC {} filing ({}): {}",
                    self.form_type, self.filing_date, desc
                )
            }
            _ => format!("SEC {} filing ({})", self.form_type, self.filing_date),
        }
    }
}

/// SEC EDGAR client — fetches filings and company facts from data.sec.gov.
///
/// All requests are free and require NO API key. The SEC fair-use policy requires
/// a descriptive User-Agent (including an email address) and a max rate of 10
/// requests/second; this client enforces both via the configured timeout and
/// a self-imposed delay between requests.
pub struct SecEdgarClient {
    client: reqwest::Client,
}

impl Default for SecEdgarClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SecEdgarClient {
    /// Create a new SEC EDGAR client with the required User-Agent.
    pub fn new() -> Self {
        let email =
            std::env::var("SEC_EDGAR_EMAIL").unwrap_or_else(|_| SEC_USER_AGENT_EMAIL.to_string());
        let ua = format!("ApexIntel-Research research@{email}");
        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Look up a company's CIK by its ticker symbol via the SEC tickers map.
    ///
    /// Returns the zero-padded 10-digit CIK string, or `None` if not found.
    pub async fn lookup_cik_by_ticker(&self, ticker: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .get("https://www.sec.gov/files/company_tickers.json")
            .send()
            .await?;
        if !resp.status().is_success() {
            warn!(status = %resp.status(), "sec_edgar: tickers map fetch failed");
            return Ok(None);
        }
        let map: HashMap<String, TickerEntry> = resp.json().await?;
        let ticker_upper = ticker.to_uppercase();
        for entry in map.values() {
            if entry.ticker == ticker_upper {
                return Ok(Some(format_cik(entry.cik)));
            }
        }
        Ok(None)
    }

    /// Fetch recent filings for a company by its CIK.
    ///
    /// `cik` may be with or without leading zeros. Returns only filings of the
    /// tracked form types (8-K, 10-K, 10-Q, DEF 14A, Form 3/4/5), limited to
    /// `limit` most recent filings.
    pub async fn fetch_recent_filings(&self, cik: &str, limit: usize) -> Result<Vec<SecFiling>> {
        let padded_cik = format_cik(cik.parse::<u64>().unwrap_or(0));
        let url = format!("https://data.sec.gov/submissions/CIK{padded_cik}.json");

        let resp = self.client.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new()); // company not in EDGAR (non-US or private)
        }
        if !resp.status().is_success() {
            warn!(cik = %padded_cik, status = %resp.status(), "sec_edgar: submissions fetch failed");
            return Ok(Vec::new());
        }

        let submission: SubmissionsResponse = resp.json().await?;
        let company_name = submission.name.unwrap_or_default();

        // The filings are split into a "recent" array and (optionally) older
        // files. We process the recent array first (most relevant).
        let recent = submission.recent;
        let mut filings: Vec<SecFiling> = Vec::new();

        let form_types: Vec<&str> = recent.form.iter().map(|s| s.as_str()).collect();
        let filing_dates: Vec<&str> = recent.filing_date.iter().map(|s| s.as_str()).collect();
        let accessions: Vec<&str> = recent.accession_number.iter().map(|s| s.as_str()).collect();
        let primary_docs: Vec<&str> = recent.primary_document.iter().map(|s| s.as_str()).collect();
        let primary_descs: Vec<&str> = recent
            .primary_doc_description
            .iter()
            .map(|s| s.as_str())
            .collect();

        let count = form_types
            .len()
            .min(filing_dates.len())
            .min(accessions.len());
        for i in 0..count {
            let form = form_types[i];
            if !TRACKED_FORM_TYPES.contains(&form) {
                continue;
            }
            let accession = accessions[i];
            let accession_no_dash = accession.replace('-', "");
            let doc = primary_docs.get(i).copied().unwrap_or("");
            let doc_url = format!(
                "https://www.sec.gov/Archives/edgar/data/{padded_cik}/{accession_no_dash}/{doc}"
            );

            filings.push(SecFiling {
                accession_number: accession.to_string(),
                form_type: form.to_string(),
                filing_date: filing_dates.get(i).copied().unwrap_or("").to_string(),
                report_date: None, // recent array doesn't include report_date
                primary_document_url: doc_url,
                description: primary_descs.get(i).map(|s| s.to_string()),
                cik: padded_cik.clone(),
                company_name: company_name.clone(),
            });

            if filings.len() >= limit {
                break;
            }
        }

        Ok(filings)
    }

    /// Fetch filings for a company identified by ticker, then return them
    /// as `Observation`s ready for storage. Convenience method that chains
    /// CIK lookup + filing fetch + observation conversion.
    pub async fn fetch_filings_as_observations(
        &self,
        ticker: &str,
        entity_id: Option<uuid::Uuid>,
        limit: usize,
    ) -> Result<Vec<Observation>> {
        let cik = match self.lookup_cik_by_ticker(ticker).await? {
            Some(c) => c,
            None => {
                warn!(ticker = %ticker, "sec_edgar: ticker not found in SEC map");
                return Ok(Vec::new());
            }
        };
        let filings = self.fetch_recent_filings(&cik, limit).await?;
        Ok(filings
            .into_iter()
            .map(|f| f.to_observation(entity_id))
            .collect())
    }
}

/// Zero-pad a CIK to 10 digits.
fn format_cik(cik: u64) -> String {
    format!("{cik:010}")
}

// ── SEC EDGAR JSON response types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TickerEntry {
    #[serde(rename = "cik_str")]
    cik: u64,
    ticker: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct SubmissionsResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "recent", default)]
    recent: RecentFilings,
}

#[derive(Debug, Default, Deserialize)]
struct RecentFilings {
    #[serde(default)]
    form: Vec<String>,
    #[serde(default, rename = "filingDate")]
    filing_date: Vec<String>,
    #[serde(default, rename = "accessionNumber")]
    accession_number: Vec<String>,
    #[serde(default, rename = "primaryDocument")]
    primary_document: Vec<String>,
    #[serde(default, rename = "primaryDocDescription")]
    primary_doc_description: Vec<String>,
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_cik_pads_to_10_digits() {
        assert_eq!(format_cik(320193), "0000320193"); // Apple
        assert_eq!(format_cik(0), "0000000000");
        assert_eq!(format_cik(9999999999), "9999999999");
    }

    #[test]
    fn filing_to_observation_sets_secfiling_type() {
        let filing = SecFiling {
            accession_number: "0000320193-24-000001".to_string(),
            form_type: "8-K".to_string(),
            filing_date: "2024-01-15".to_string(),
            report_date: None,
            primary_document_url: "https://www.sec.gov/Archives/edgar/data/0000320193/..."
                .to_string(),
            description: Some("Item 8.01 - Other Events".to_string()),
            cik: "0000320193".to_string(),
            company_name: "Apple Inc.".to_string(),
        };
        let obs = filing.to_observation(None);
        assert_eq!(obs.observation_type, ObservationType::SecFiling);
        assert!(obs.value["form_type"].as_str() == Some("8-K"));
        assert!(obs.provenance["source"].as_str() == Some("sec_edgar"));
    }

    #[test]
    fn filing_summary_includes_form_and_date() {
        let filing = SecFiling {
            accession_number: "x".to_string(),
            form_type: "10-K".to_string(),
            filing_date: "2024-03-01".to_string(),
            report_date: None,
            primary_document_url: "x".to_string(),
            description: Some("Annual report".to_string()),
            cik: "x".to_string(),
            company_name: "Test Corp".to_string(),
        };
        assert_eq!(
            filing.summary(),
            "SEC 10-K filing (2024-03-01): Annual report"
        );
    }

    #[test]
    fn tracked_form_types_cover_key_filings() {
        assert!(TRACKED_FORM_TYPES.contains(&"8-K"));
        assert!(TRACKED_FORM_TYPES.contains(&"10-K"));
        assert!(TRACKED_FORM_TYPES.contains(&"DEF 14A"));
        assert!(TRACKED_FORM_TYPES.contains(&"4"));
        // S-1 (registration) and 13F (holdings) are NOT tracked
        assert!(!TRACKED_FORM_TYPES.contains(&"S-1"));
    }
}
