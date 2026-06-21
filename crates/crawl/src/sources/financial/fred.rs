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
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) FRED Monitor")
            .build()
            .context("building FRED monitor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Fetch observations for all configured FRED series.
    pub async fn scan(&self) -> Vec<FredSignal> {
        let mut all_signals = Vec::new();

        for series_id in &self.config.series_ids {
            match self.fetch_series(series_id).await {
                Ok(observations) => {
                    if let Some(signal) = self.build_signal(series_id, &observations) {
                        all_signals.push(signal);
                    }
                }
                Err(e) => {
                    warn!(series_id = %series_id, error = %e, "FRED series fetch failed");
                }
            }
        }

        info!(total = all_signals.len(), "FRED economic monitoring scan complete");
        all_signals
    }

    /// Fetch recent observations for a FRED series.
    async fn fetch_series(&self, series_id: &str) -> Result<Vec<FredObservation>> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .context("FRED API key not configured")?;

        let url = format!(
            "{}?series_id={}&api_key={}&file_type=json&sort_order=desc&limit=24",
            self.config.base_url, series_id, api_key
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("FRED API request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), series_id = %series_id, "FRED API returned non-success");
            return Ok(Vec::new());
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

        let fred_resp: FredResponse = resp.json().await.unwrap_or_default();
        let raw_obs = fred_resp.observations.unwrap_or_default();

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

        Ok(observations)
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
}