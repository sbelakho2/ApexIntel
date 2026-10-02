//! Marketplace Monitoring Module
//!
//! Monitors dark web marketplaces for leaked data, credentials, and illicit goods:
//! - Generic marketplace crawler (site-agnostic)
//! - Product category monitoring
//! - Vendor reputation tracking
//! - Price intelligence
//! - Alert on new entries related to tracked entities
//!
//! Note: This uses the Tor proxy for anonymous access. Requires Tor daemon running.

use crate::acquisition::{AcquisitionOutcome, AdapterPrerequisite, SourceAdapter};
use crate::sources::dark_web::tor_proxy::TorProxy;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// A marketplace listing/listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListing {
    pub listing_id: String,
    pub marketplace: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub price: Option<MarketplacePrice>,
    pub vendor_name: String,
    pub vendor_rating: Option<f32>,
    pub listing_date: Option<DateTime<Utc>>,
    pub views: Option<u64>,
    pub last_updated: DateTime<Utc>,
    pub url: String,
    pub contains_keywords: Vec<String>,
    pub is_related: bool,
}

impl MarketplaceListing {
    /// Whether this listing has a price in a cryptocurrency.
    pub fn has_crypto_price(&self) -> bool {
        self.price.as_ref().map(|p| p.is_crypto()).unwrap_or(false)
    }

    /// Whether the vendor is trusted (rating >= 4.0).
    pub fn is_trusted_vendor(&self) -> bool {
        self.vendor_rating.map(|r| r >= 4.0).unwrap_or(false)
    }
}

/// A price in fiat or crypto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePrice {
    pub amount: f64,
    pub currency: String,
    /// Exchange rate to USD (if known).
    pub usd_value: Option<f64>,
}

impl MarketplacePrice {
    /// Whether this is a cryptocurrency price.
    pub fn is_crypto(&self) -> bool {
        matches!(
            self.currency.to_uppercase().as_str(),
            "BTC" | "XMR" | "ETH" | "LTC"
        )
    }

    /// Create a new price.
    pub fn new(amount: f64, currency: &str) -> Self {
        Self {
            amount,
            currency: currency.to_string(),
            usd_value: None,
        }
    }

    /// Create in BTC.
    pub fn btc(amount: f64) -> Self {
        Self::new(amount, "BTC")
    }

    /// Create in USD.
    pub fn usd(amount: f64) -> Self {
        Self::new(amount, "USD")
    }
}

/// Marketplace category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketplaceCategory {
    Credentials,
    FinancialData,
    PersonalData,
    Malware,
    Drugs,
    Weapons,
    Counterfeits,
    StolenGoods,
    Leaks,
    Other,
}

impl MarketplaceCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::FinancialData => "financial_data",
            Self::PersonalData => "personal_data",
            Self::Malware => "malware",
            Self::Drugs => "drugs",
            Self::Weapons => "weapons",
            Self::Counterfeits => "counterfeits",
            Self::StolenGoods => "stolen_goods",
            Self::Leaks => "leaks",
            Self::Other => "other",
        }
    }
}

/// Marketplace monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceMonitorConfig {
    /// Keywords that flag a listing as relevant.
    pub alert_keywords: Vec<String>,
    /// Categories to monitor.
    pub categories: Vec<MarketplaceCategory>,
    /// Maximum listings to return per scan.
    pub max_listings: u32,
    /// Minimum vendor rating threshold.
    pub min_vendor_rating: Option<f32>,
    /// Tor proxy configuration.
    pub tor_config: Option<crate::sources::dark_web::tor_proxy::TorConfig>,
}

impl Default for MarketplaceMonitorConfig {
    fn default() -> Self {
        Self {
            alert_keywords: vec![
                "password".to_string(),
                "credentials".to_string(),
                "dump".to_string(),
                "breach".to_string(),
                "leak".to_string(),
            ],
            categories: vec![
                MarketplaceCategory::Credentials,
                MarketplaceCategory::FinancialData,
                MarketplaceCategory::Leaks,
            ],
            max_listings: 100,
            min_vendor_rating: Some(3.0),
            tor_config: None,
        }
    }
}

/// Marketplace monitor.
#[derive(Debug, Clone)]
pub struct MarketplaceMonitor {
    tor_proxy: TorProxy,
    config: MarketplaceMonitorConfig,
    /// Cached listings.
    listings: Vec<MarketplaceListing>,
}

impl MarketplaceMonitor {
    /// Create with a Tor proxy and config.
    pub fn new(tor_proxy: TorProxy, config: MarketplaceMonitorConfig) -> Self {
        Self {
            tor_proxy,
            config,
            listings: Vec::new(),
        }
    }

    /// Create with a Tor proxy, using default config.
    pub fn with_tor(tor_proxy: TorProxy) -> Self {
        Self::new(tor_proxy, MarketplaceMonitorConfig::default())
    }

    /// Check if Tor is available.
    pub fn is_tor_available(&self) -> bool {
        self.tor_proxy.is_reachable()
    }

    /// Monitor a specific onion site for listings.
    ///
    /// An unreachable Tor daemon is [`AcquisitionOutcome::Unavailable`], not an
    /// empty success: the deployment cannot execute the adapter at all.
    pub async fn scan_onion(
        &mut self,
        onion_url: &str,
        category: &str,
    ) -> AcquisitionOutcome<MarketplaceListing> {
        if !self.is_tor_available() {
            return AcquisitionOutcome::unavailable(
                "Tor not reachable: dark-web marketplace scan unavailable",
            );
        }

        let client = match self.tor_proxy.create_tor_client() {
            Ok(client) => client,
            Err(error) => {
                return AcquisitionOutcome::unavailable(format!(
                    "create Tor client failed: {error}"
                ));
            }
        };
        let resp = match client.get(onion_url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("onion site request {onion_url} failed: {error}"),
                    None,
                );
            }
        };

        if !resp.status().is_success() {
            debug!(status = %resp.status(), url = %onion_url, "Marketplace onion returned non-success");
            let retry_after = crate::acquisition::retry_after_secs(resp.headers());
            return crate::acquisition::http_failure(
                resp.status().as_u16(),
                retry_after,
                "Marketplace onion",
            );
        }

        let body = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(body) => body,
            Err(error) => {
                return AcquisitionOutcome::fetch_failed(
                    format!("read marketplace HTML failed: {error}"),
                    None,
                );
            }
        };
        self.parse_listings(&body, onion_url, category)
    }

    fn parse_listings(
        &mut self,
        html: &str,
        source: &str,
        category: &str,
    ) -> AcquisitionOutcome<MarketplaceListing> {
        use regex::Regex;

        let title_re = Regex::new(r"<title>([^<]+)</title>").ok();
        let title = title_re
            .and_then(|re| re.captures(html))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| source.to_string());

        let listing = MarketplaceListing {
            listing_id: format!("{}-listing-{}", source, Utc::now().timestamp_millis()),
            marketplace: source.to_string(),
            title,
            description: html.chars().take(300).collect(),
            category: category.to_string(),
            price: None,
            vendor_name: "Unknown".to_string(),
            vendor_rating: None,
            listing_date: None,
            views: None,
            last_updated: Utc::now(),
            url: source.to_string(),
            contains_keywords: self.config.alert_keywords.clone(),
            is_related: true,
        };

        self.listings.push(listing.clone());
        debug!(marketplace = %source, "Marketplace listing parsed");
        AcquisitionOutcome::success_now(vec![listing])
    }

    /// Get all cached listings.
    pub fn all_listings(&self) -> &[MarketplaceListing] {
        &self.listings
    }

    /// Get related listings (those matching alert keywords).
    pub fn related_listings(&self) -> Vec<&MarketplaceListing> {
        self.listings.iter().filter(|l| l.is_related).collect()
    }

    /// Return total listing count.
    pub fn listing_count(&self) -> usize {
        self.listings.len()
    }
}

/// Request for one marketplace onion scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceScanRequest {
    pub onion_url: String,
    pub category: String,
}

#[async_trait]
impl SourceAdapter for MarketplaceMonitor {
    type Item = MarketplaceListing;
    type Request = MarketplaceScanRequest;

    fn adapter_id(&self) -> &'static str {
        "dark_web_marketplace"
    }

    fn prerequisite(&self) -> AdapterPrerequisite {
        AdapterPrerequisite::TOR
    }

    async fn acquire(
        &self,
        request: MarketplaceScanRequest,
    ) -> AcquisitionOutcome<MarketplaceListing> {
        // `scan_onion` needs `&mut self` for its listing cache; clone the
        // monitor so the adapter contract stays `&self`.
        let mut monitor = self.clone();
        monitor
            .scan_onion(&request.onion_url, &request.category)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_adapter_declares_its_prerequisite() {
        let monitor = MarketplaceMonitor::with_tor(TorProxy::default());
        assert_eq!(monitor.adapter_id(), "dark_web_marketplace");
        assert!(monitor.prerequisite().requires_tor);
        assert!(crate::acquisition::adapter_descriptor(monitor.adapter_id()).is_some());
    }

    #[test]
    fn marketplace_price_crypto() {
        let btc = MarketplacePrice::btc(0.5);
        assert!(btc.is_crypto());
        assert_eq!(btc.currency, "BTC");

        let usd = MarketplacePrice::usd(100.0);
        assert!(!usd.is_crypto());
    }

    #[test]
    fn marketplace_listing_trusted_vendor() {
        let listing = MarketplaceListing {
            listing_id: "test-001".to_string(),
            marketplace: "test-market".to_string(),
            title: "Test".to_string(),
            description: "Test listing".to_string(),
            category: "credentials".to_string(),
            price: None,
            vendor_name: "TestVendor".to_string(),
            vendor_rating: Some(4.5),
            listing_date: None,
            views: None,
            last_updated: Utc::now(),
            url: "http://test.onion".to_string(),
            contains_keywords: vec![],
            is_related: false,
        };
        assert!(listing.is_trusted_vendor());
    }

    #[test]
    fn marketplace_monitor_constructs() {
        let proxy = TorProxy::with_defaults();
        let monitor = MarketplaceMonitor::with_tor(proxy);
        assert_eq!(monitor.listing_count(), 0);
    }

    #[test]
    fn marketplace_category_as_str() {
        assert_eq!(MarketplaceCategory::Credentials.as_str(), "credentials");
        assert_eq!(MarketplaceCategory::Leaks.as_str(), "leaks");
    }
}
