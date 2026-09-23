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
    pub async fn scan(&self) -> Vec<ZeroNetSignal> {
        let mut all_signals = Vec::new();

        for tracker in &self.config.trackers {
            match self.scan_tracker(tracker).await {
                Ok(signals) => {
                    debug!(tracker = %tracker, count = signals.len(), "ZeroNet tracker scan complete");
                    all_signals.extend(signals);
                }
                Err(e) => {
                    warn!(tracker = %tracker, error = %e, "ZeroNet tracker scan failed");
                }
            }
        }

        all_signals.sort_by_key(|a| std::cmp::Reverse(a.observed_at));
        info!(
            total = all_signals.len(),
            "ZeroNet monitoring scan complete"
        );
        all_signals
    }

    /// Scan a single ZeroNet tracker's public zite index.
    async fn scan_tracker(&self, tracker_url: &str) -> Result<Vec<ZeroNetSignal>> {
        let resp = self
            .client
            .get(tracker_url)
            .header("Accept", "application/json")
            .send()
            .await
            .context("ZeroNet tracker fetch")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), tracker = %tracker_url, "ZeroNet tracker returned non-success");
            return Ok(Vec::new());
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

        let zites: Vec<ZeroNetZite> = resp.json().await.unwrap_or_default();

        let signals = zites
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

        Ok(signals)
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
