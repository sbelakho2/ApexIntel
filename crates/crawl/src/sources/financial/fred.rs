//! FRED (Federal Reserve Economic Data) Intelligence Module
//!
//! Queries the St. Louis Federal Reserve's FRED API for macroeconomic indicators
//! relevant to supply-chain risk assessment: manufacturing indices, trade data,
//! currency exchange rates, commodity prices, and industrial production.
//!
//! # Capabilities
//! - Macroeconomic indicator retrieval
//! - Manufacturing sector health monitoring
//! - Currency and commodity trend analysis
//! - Trade balance and tariff impact assessment

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// A FRED series observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FredObservation {
    /// FRED series ID (e.g. INDPRO, CPIAUCSL, DCOILWTICO).
    pub series_id: String,
    /// Human-readable title.
    pub title: String,
    /// Observation date.
    pub date: NaiveDate,
    /// Numeric value.
    pub value: f64,
    /// Units (e.g. "Index 2017=100", "Percent", "Millions of Dollars").
    pub units: String,
    /// Frequency (e.g. "Monthly", "Quarterly").
    pub frequency: String,
    /// Seasonal adjustment (e.g. "Seasonally Adjusted", "Not Seasonally Adjusted").
    pub seasonal_adjustment: String,
}

/// FRED intelligence signal derived from economic data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FredSignal {
    /// FRED series ID.
    pub series_id: String,
    /// Indicator name.
    pub indicator_name: String,
    /// Latest value.
    pub latest_value: f64,
    /// Change direction and magnitude.
    pub change_pct: Option<f64>,
    /// Period over which the change was computed.
    pub change_period: String,
    /// Intelligence significance.
    pub significance: String,
    /// When fetched.
    pub fetched_at: DateTime<Utc>,
}

/// FRED API configuration.
#[derive(Debug, Clone)]
pub struct FredConfig {
    /// FRED API base URL.
    pub base_url: String,
    /// FRED API key.
    pub api_key: Option<String>,
    /// Series IDs to monitor.
    pub series_ids: Vec<String>,
    /// Request timeout.
    pub timeout_secs: u64,
}

impl Default for FredConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.stlouisfed.org/fred/series/observations".to_string(),
            api_key: std::env::var("FRED_API_KEY").ok(),
            series_ids: vec![
                // Industrial Production Index
                "INDPRO".to_string(),
                // Manufacturing Production Index
                "IPDMAN".to_string(),
                // Capacity Utilization: Manufacturing
                "CAPUTLG2211A2S".to_string(),
                // Crude Oil Prices: West Texas Intermediate
                "DCOILWTICO".to_string(),
                // Consumer Price Index for All Urban Consumers
                "CPIAUCSL".to_string(),
                // Trade Balance: Goods and Services
                "BOPGSTB".to_string(),
                // Producer Price Index by Commodity
                "PPIACO".to_string(),
                // Global Supply Chain Pressure Index
                "GSCPI".to_string(),
                // Nonfarm Business Sector: Unit Labor Costs
                "ULCNFB".to_string(),
                // Semiconductor and electronic component manufacturing
                "IPG33641A3S".to_string(),
            ],
            timeout_secs: 30,
        }
    }
}

/// FRED economic data monitor.
#[derive(Debug, Clone)]
pub struct FredMonitor {
    client: Client,
    config: FredConfig,
}

impl FredMonitor {
    /// Create with explicit configuration.
    pub fn new(config: FredConfig) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(config.timeout_secs),
            user_agent: Some("ApexIntel/1.0 (+https://apexintel.io) FRED Monitor".to_string()),
            ..crate::http::ExternalClientOptions::default()
        })
        .context("building FRED monitor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Fetch observations for all configured FRED series.
    ///
    /// Aggregates per-series outcomes conservatively: a series whose response
    /// no longer deserializes is reported as a parse failure (marking the
    /// source degraded) instead of being dropped from an empty-looking scan.
    pub async fn scan(&self) -> ParseOutcome<FredSignal> {
        let mut all_signals = Vec::new();
        let mut any_parsed = false;
        let mut any_empty_parse = false;
        let mut first_fetch_failure: Option<(String, Option<u16>)> = None;
        let mut first_parse_failure: Option<(String, String)> = None;

        for series_id in &self.config.series_ids {
            match self.fetch_series(series_id).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    if items.is_empty() {
                        any_empty_parse = true;
                    }
                    if let Some(signal) = self.build_signal(series_id, &items) {
                        all_signals.push(signal);
                    }
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    warn!(series_id = %series_id, error = %error, "FRED series fetch failed");
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    warn!(series_id = %series_id, error = %error, "FRED series parser failed");
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        info!(
            total = all_signals.len(),
            any_empty_parse, "FRED economic monitoring scan complete"
        );

        if let Some((error, redacted_sample)) = first_parse_failure {
            return ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            };
        }
        if !any_parsed {
            if let Some((error, http_status)) = first_fetch_failure {
                return ParseOutcome::FetchFailed { error, http_status };
            }
        }
        ParseOutcome::ParsedSuccessfully { items: all_signals }
    }

    /// Fetch recent observations for a FRED series.
    ///
    /// Fetch failure (network/HTTP/key), parse failure (schema change) and a
    /// successfully deserialized — possibly empty — observation list are
    /// distinct outcomes.
    async fn fetch_series(&self, series_id: &str) -> ParseOutcome<FredObservation> {
        let Some(api_key) = self.config.api_key.as_deref() else {
            let outcome =
                ParseOutcome::fetch_failed("FRED API key not configured".to_string(), None);
            PARSER_METRICS.record(&outcome);
            return outcome;
        };

        let url = format!(
            "{}?series_id={}&api_key={}&file_type=json&sort_order=desc&limit=24",
            self.config.base_url, series_id, api_key
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("FRED API request failed: {}", error.without_url()),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), series_id = %series_id, "FRED API returned non-success");
            let outcome = ParseOutcome::fetch_failed(
                format!("FRED API returned HTTP {status}"),
                Some(status),
            );
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        #[derive(Deserialize, Default)]
        struct FredResponse {
            observations: Option<Vec<FredRawObservation>>,
        }
        #[derive(Deserialize)]
        struct FredRawObservation {
            date: String,
            value: String,
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read FRED response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let fred_resp: FredResponse = match serde_json::from_str(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse FRED JSON for {series_id}: {error}"),
                    &text,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };
        let raw_obs = fred_resp.observations.unwrap_or_default();

        // A missing/null `observations` array is a valid "no data" response;
        // individual malformed rows are skipped as best-effort telemetry (the
        // envelope deserialized successfully, which is the parser contract).
        let observations: Vec<FredObservation> = raw_obs
            .iter()
            .filter_map(|o| {
                let date = NaiveDate::parse_from_str(&o.date, "%Y-%m-%d").ok()?;
                let value = o.value.parse::<f64>().ok()?;
                Some(FredObservation {
                    series_id: series_id.to_string(),
                    title: series_id.to_string(),
                    date,
                    value,
                    units: String::new(),
                    frequency: String::new(),
                    seasonal_adjustment: String::new(),
                })
            })
            .collect();

        let outcome = ParseOutcome::parsed(observations);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Build an intelligence signal from FRED observations.
    fn build_signal(
        &self,
        series_id: &str,
        observations: &[FredObservation],
    ) -> Option<FredSignal> {
        if observations.len() < 2 {
            return None;
        }

        let latest = &observations[0];
        let prev = &observations[1];
        let change_pct = if prev.value != 0.0 {
            Some(((latest.value - prev.value) / prev.value.abs()) * 100.0)
        } else {
            None
        };

        let significance = match series_id {
            "INDPRO" | "IPDMAN" => {
                if let Some(pct) = change_pct {
                    if pct > 2.0 {
                        "Strong industrial expansion — signals increased demand for manufacturing inputs"
                    } else if pct < -2.0 {
                        "Contraction warning — may indicate softening demand or supply disruptions"
                    } else {
                        "Industrial activity stable — baseline monitoring"
                    }
                } else {
                    "Industrial production data available"
                }
            }
            "DCOILWTICO" => {
                if let Some(pct) = change_pct {
                    if pct > 5.0 {
                        "Oil price spike — rising logistics and materials costs"
                    } else if pct < -5.0 {
                        "Oil price decline — potential logistics cost relief"
                    } else {
                        "Oil prices stable"
                    }
                } else {
                    "Crude oil price data available"
                }
            }
            "GSCPI" => {
                if let Some(pct) = change_pct {
                    if pct > 10.0 {
                        "Global supply chain pressure increasing — risk of delays and cost escalation"
                    } else if pct < -10.0 {
                        "Supply chain pressures easing — normalization underway"
                    } else {
                        "Supply chain pressure stable"
                    }
                } else {
                    "Supply chain pressure index available"
                }
            }
            "BOPGSTB" => {
                "Trade balance indicator — relevant for tariff and trade policy monitoring"
            }
            "CPIAUCSL" => {
                if let Some(pct) = change_pct {
                    if pct > 0.5 {
                        "Inflation pressure rising — may impact input costs"
                    } else {
                        "Consumer prices stable or declining"
                    }
                } else {
                    "CPI data available"
                }
            }
            _ => "Economic indicator updated — component of macro risk assessment",
        };

        Some(FredSignal {
            series_id: series_id.to_string(),
            indicator_name: series_id.to_string(),
            latest_value: latest.value,
            change_pct,
            change_period: format!("{} to {}", prev.date, latest.date),
            significance: significance.to_string(),
            fetched_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fred_default_config_has_series() {
        let cfg = FredConfig::default();
        assert!(!cfg.series_ids.is_empty());
        assert!(cfg.series_ids.contains(&"INDPRO".to_string()));
        assert!(cfg.series_ids.contains(&"GSCPI".to_string()));
    }

    #[test]
    fn fred_monitor_constructs() {
        let result = FredMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn fred_build_signal_requires_two_observations() {
        let monitor = FredMonitor::new(Default::default()).unwrap();
        let obs = vec![FredObservation {
            series_id: "TEST".into(),
            title: "Test".into(),
            date: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            value: 100.0,
            units: String::new(),
            frequency: String::new(),
            seasonal_adjustment: String::new(),
        }];
        assert!(monitor.build_signal("TEST", &obs).is_none());
    }

    #[test]
    fn fred_build_signal_computes_change() {
        let monitor = FredMonitor::new(Default::default()).unwrap();
        let obs = vec![
            FredObservation {
                series_id: "TEST".into(),
                title: "Test".into(),
                date: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
                value: 110.0,
                units: String::new(),
                frequency: String::new(),
                seasonal_adjustment: String::new(),
            },
            FredObservation {
                series_id: "TEST".into(),
                title: "Test".into(),
                date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
                value: 100.0,
                units: String::new(),
                frequency: String::new(),
                seasonal_adjustment: String::new(),
            },
        ];
        let signal = monitor.build_signal("TEST", &obs).unwrap();
        assert_eq!(signal.latest_value, 110.0);
        assert!((signal.change_pct.unwrap() - 10.0).abs() < 0.01);
    }

    fn monitor_for(base_url: String) -> FredMonitor {
        FredMonitor::new(FredConfig {
            base_url,
            api_key: Some("test-key".to_string()),
            series_ids: vec!["INDPRO".to_string()],
            timeout_secs: 5,
        })
        .expect("FRED monitor")
    }

    #[tokio::test]
    async fn fred_schema_change_is_parse_failure_not_empty_success() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(200).set_body_string("{\"observations\": \"not-an-array\"}"),
            )
            .mount(&server)
            .await;

        let outcome = monitor_for(server.uri()).scan().await;
        assert!(
            outcome.is_parse_failure(),
            "expected ParseFailed: {outcome:?}"
        );
        assert!(!outcome.is_parsed_success());
        assert!(outcome.redacted_sample().is_some());
        assert!(outcome.failure_error().is_some());
    }

    #[tokio::test]
    async fn fred_empty_observations_is_parsed_success() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"observations\": []}"))
            .mount(&server)
            .await;

        let outcome = monitor_for(server.uri()).scan().await;
        assert!(outcome.is_parsed_success());
        assert_eq!(outcome.item_count(), 0);
    }

    #[tokio::test]
    async fn fred_http_error_is_fetch_failure() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let outcome = monitor_for(server.uri()).scan().await;
        assert!(outcome.is_fetch_failure());
        assert!(!outcome.is_parse_failure());
    }
}
