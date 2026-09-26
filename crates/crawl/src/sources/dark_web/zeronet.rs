//! ZeroNet Intelligence Module
//!
//! Monitors ZeroNet distributed web for intelligence signals.
//! ZeroNet is a peer-to-peer web platform using Bitcoin cryptography and BitTorrent
//! network for decentralized hosting — a significant dark web alternative to Tor.
//!
//! # Capabilities
//! - Zite (ZeroNet site) discovery
//! - Keyword-based content search across known ZeroNet trackers
//! - Public zite indexing
//! - OSINT signal extraction from decentralized zites

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// A ZeroNet zite (distributed site) discovery signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZeroNetSignal {
    /// Zite address (e.g. 1HeLLo4uzjaLetFx6NH3PMwFP3qbRbTf3D).
    pub zite_address: String,
    /// Zite title or name.
    pub title: String,
    /// Content description.
    pub description: String,
    /// Source of the discovery (tracker, index, etc.).
    pub discovery_source: String,
    /// Keywords matched in content.
    pub matched_keywords: Vec<String>,
    /// When the zite was first observed.
    pub observed_at: Option<DateTime<Utc>>,
    /// URL for the ZeroNet tracker entry.
    pub tracker_url: Option<String>,
    /// Relevance score [0, 1].
    pub relevance_score: f32,
    /// Whether the content matches target entities.
    pub is_related: bool,
}

/// ZeroNet monitor configuration.
#[derive(Debug, Clone)]
pub struct ZeroNetConfig {
    /// Trackers to query for zite discovery.
    pub trackers: Vec<String>,
    /// Keywords to monitor.
    pub keywords: Vec<String>,
    /// Maximum signals per scan.
    pub max_signals: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for ZeroNetConfig {
    fn default() -> Self {
        Self {
            trackers: vec![
                "https://zn.amorgan.xyz".to_string(),
                "https://zeronet.bit.surf".to_string(),
            ],
            keywords: vec![
                "electronics".to_string(),
                "manufacturing".to_string(),
                "supply chain".to_string(),
                "semiconductor".to_string(),
            ],
            max_signals: 50,
            timeout_secs: 30,
        }
    }
}

/// ZeroNet distributed web monitor.
#[derive(Debug, Clone)]
pub struct ZeroNetMonitor {
    client: Client,
    config: ZeroNetConfig,
}

impl ZeroNetMonitor {
    /// Create with explicit configuration.
    pub fn new(config: ZeroNetConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) ZeroNet Monitor")
            .build()
            .context("building ZeroNet monitor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Scan all configured ZeroNet trackers for keyword-matching zites.
    ///
    /// Aggregates per-tracker outcomes conservatively: any parse failure is
    /// propagated (so a schema change is never reported as an empty scan),
    /// then all-trackers-failed becomes a fetch failure, and only then is a
    /// successfully parsed (possibly empty) result returned.
    pub async fn scan(&self) -> ParseOutcome<ZeroNetSignal> {
        let mut all_signals = Vec::new();
        let mut any_parsed = false;
        let mut first_fetch_failure: Option<(String, Option<u16>)> = None;
        let mut first_parse_failure: Option<(String, String)> = None;

        for tracker in &self.config.trackers {
            match self.scan_tracker(tracker).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    debug!(tracker = %tracker, count = items.len(), "ZeroNet tracker scan complete");
                    any_parsed = true;
                    all_signals.extend(items);
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    warn!(tracker = %tracker, error = %error, "ZeroNet tracker scan failed");
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    warn!(tracker = %tracker, error = %error, "ZeroNet tracker parser failed");
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        all_signals.sort_by_key(|a| std::cmp::Reverse(a.observed_at));
        info!(
            total = all_signals.len(),
            "ZeroNet monitoring scan complete"
        );

        if let Some((error, redacted_sample)) = first_parse_failure {
            // A tracker whose schema changed marks the source degraded for
            // this cycle; the parser-failure metric has already been recorded.
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

    /// Scan a single ZeroNet tracker's public zite index.
    ///
    /// A non-success HTTP status is a fetch failure (previously it was
    /// silently reported as an empty zite list), and an unparseable body is a
    /// parser failure with a redacted sample.
    async fn scan_tracker(&self, tracker_url: &str) -> ParseOutcome<ZeroNetSignal> {
        let resp = match self
            .client
            .get(tracker_url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("ZeroNet tracker fetch failed: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), tracker = %tracker_url, "ZeroNet tracker returned non-success");
            let outcome = ParseOutcome::fetch_failed(
                format!("ZeroNet tracker returned HTTP {status}"),
                Some(status),
            );
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ZeroNetZite {
            address: Option<String>,
            title: Option<String>,
            description: Option<String>,
            peers: Option<i64>,
            modified: Option<i64>,
            date_added: Option<i64>,
        }

        let body = match resp.text().await {
            Ok(body) => body,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read ZeroNet tracker response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if body.is_empty() || body.trim() == "[]" {
            let outcome = ParseOutcome::parsed(Vec::new());
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let zites: Vec<ZeroNetZite> = match serde_json::from_str(&body) {
            Ok(zites) => zites,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse ZeroNet tracker JSON: {error}"),
                    &body,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let signals: Vec<ZeroNetSignal> = zites
            .into_iter()
            .filter_map(|z| {
                let address = z.address?;
                let title = z.title.unwrap_or_default();
                let description = z.description.unwrap_or_default();
                let combined = format!("{} {}", title, description).to_lowercase();

                let matched: Vec<String> = self
                    .config
                    .keywords
                    .iter()
                    .filter(|kw| combined.contains(&kw.to_lowercase()))
                    .cloned()
                    .collect();

                if matched.is_empty() {
                    return None;
                }

                let observed_at = z.modified.or(z.date_added).and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.with_timezone(&Utc))
                });

                Some(ZeroNetSignal {
                    zite_address: address.clone(),
                    title,
                    description,
                    discovery_source: format!("tracker:{}", tracker_url),
                    matched_keywords: matched,
                    observed_at,
                    tracker_url: Some(format!("{}/{}", tracker_url, address)),
                    relevance_score: 0.4,
                    is_related: true,
                })
            })
            .take(self.config.max_signals as usize)
            .collect();

        let outcome = ParseOutcome::parsed(signals);
        PARSER_METRICS.record(&outcome);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeronet_default_config_has_trackers() {
        let cfg = ZeroNetConfig::default();
        assert!(!cfg.trackers.is_empty());
        assert_eq!(cfg.max_signals, 50);
    }

    #[test]
    fn zeronet_monitor_constructs() {
        let result = ZeroNetMonitor::new(Default::default());
        assert!(result.is_ok());
    }
}
