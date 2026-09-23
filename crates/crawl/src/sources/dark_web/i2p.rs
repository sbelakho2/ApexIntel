//! I2P (Invisible Internet Project) Intelligence Module
//!
//! Monitors I2P eepsites (hidden services) for OSINT signals relevant to
//! supply-chain intelligence and competitive analysis. I2P provides
//! garlic-routed anonymous hosting, an alternative dark web to Tor.
//!
//! # Capabilities
//! - I2P eepsite discovery via public registries
//! - Keyword-based content search
//! - Hosted service cataloging

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// An I2P eepsite discovery signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2pSignal {
    /// Eepsite hostname (base32 or base64 .b32.i2p / .i2p address).
    pub eepsite_address: String,
    /// Eepsite name or title.
    pub name: String,
    /// Content description.
    pub description: String,
    /// Discovery source (registry, crawl, etc.).
    pub discovery_source: String,
    /// Keywords matched in content.
    pub matched_keywords: Vec<String>,
    /// When the eepsite was observed.
    pub observed_at: Option<DateTime<Utc>>,
    /// Public registry URL.
    pub registry_url: Option<String>,
    /// Relevance score [0, 1].
    pub relevance_score: f32,
    /// Whether it matches target entities.
    pub is_related: bool,
}

/// I2P monitor configuration.
#[derive(Debug, Clone)]
pub struct I2pConfig {
    /// I2P jump services or registries (HTTP proxies to I2P network).
    pub registries: Vec<String>,
    /// Target keywords.
    pub keywords: Vec<String>,
    /// Max signals per scan.
    pub max_signals: u32,
    /// Request timeout.
    pub timeout_secs: u64,
}

impl Default for I2pConfig {
    fn default() -> Self {
        Self {
            registries: vec![
                "http://identiguy.i2p".to_string(),
                "http://stats.i2p".to_string(),
            ],
            keywords: vec![
                "electronics".to_string(),
                "manufacturing".to_string(),
                "supply chain".to_string(),
                "hardware".to_string(),
                "semiconductor".to_string(),
            ],
            max_signals: 50,
            timeout_secs: 30,
        }
    }
}

/// I2P network monitor.
#[derive(Debug, Clone)]
pub struct I2pMonitor {
    client: Client,
    config: I2pConfig,
}

impl I2pMonitor {
    /// Create with explicit configuration.
    pub fn new(config: I2pConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) I2P Monitor")
            .build()
            .context("building I2P monitor HTTP client")?;

        Ok(Self { client, config })
    }

    /// Scan all configured I2P registries for keyword-matching eepsites.
    pub async fn scan(&self) -> Vec<I2pSignal> {
        let mut all_signals = Vec::new();

        for registry in &self.config.registries {
            match self.scan_registry(registry).await {
                Ok(signals) => {
                    debug!(registry = %registry, count = signals.len(), "I2P registry scan complete");
                    all_signals.extend(signals);
                }
                Err(e) => {
                    warn!(registry = %registry, error = %e, "I2P registry scan failed");
                }
            }
        }

        all_signals.sort_by_key(|a| std::cmp::Reverse(a.observed_at));
        info!(total = all_signals.len(), "I2P monitoring scan complete");
        all_signals
    }

    /// Scan a single I2P registry for eepsite listings.
    async fn scan_registry(&self, registry_url: &str) -> Result<Vec<I2pSignal>> {
        let resp = self
            .client
            .get(registry_url)
            .header("Accept", "text/html, application/json")
            .send()
            .await
            .context("I2P registry fetch")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), registry = %registry_url, "I2P registry returned non-success");
            return Ok(Vec::new());
        }

        let body = resp.text().await.context("read I2P registry body")?;

        // Attempt JSON parsing for structured registry responses
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct I2pEepsite {
            address: Option<String>,
            name: Option<String>,
            description: Option<String>,
            last_seen: Option<i64>,
        }

        let signals = if let Ok(eepsites) = serde_json::from_str::<Vec<I2pEepsite>>(&body) {
            eepsites
                .into_iter()
                .filter_map(|e| {
                    let address = e.address?;
                    let name = e.name.unwrap_or_default();
                    let description = e.description.unwrap_or_default();
                    let combined = format!("{} {}", name, description).to_lowercase();

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

                    let observed_at = e.last_seen.and_then(|ts| {
                        DateTime::from_timestamp(ts, 0).map(|dt| dt.with_timezone(&Utc))
                    });

                    Some(I2pSignal {
                        eepsite_address: address.clone(),
                        name,
                        description,
                        discovery_source: format!("registry:{}", registry_url),
                        matched_keywords: matched,
                        observed_at,
                        registry_url: Some(format!("{}/{}", registry_url, address)),
                        relevance_score: 0.35,
                        is_related: true,
                    })
                })
                .take(self.config.max_signals as usize)
                .collect()
        } else {
            // Fallback: plain-text / HTML scanning
            let lower = body.to_lowercase();
            let matched: Vec<String> = self
                .config
                .keywords
                .iter()
                .filter(|kw| lower.contains(&kw.to_lowercase()))
                .cloned()
                .collect();

            if matched.is_empty() {
                return Ok(Vec::new());
            }

            vec![I2pSignal {
                eepsite_address: registry_url.to_string(),
                name: format!("I2P registry: {}", registry_url),
                description: format!(
                    "Keyword match on I2P registry at {}. Matched: {}",
                    registry_url,
                    matched.join(", ")
                ),
                discovery_source: format!("registry:{}", registry_url),
                matched_keywords: matched,
                observed_at: Some(Utc::now()),
                registry_url: Some(registry_url.to_string()),
                relevance_score: 0.3,
                is_related: true,
            }]
        };

        Ok(signals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2p_default_config_has_registries() {
        let cfg = I2pConfig::default();
        assert!(!cfg.registries.is_empty());
        assert_eq!(cfg.max_signals, 50);
    }

    #[test]
    fn i2p_monitor_constructs() {
        let result = I2pMonitor::new(Default::default());
        assert!(result.is_ok());
    }
}
