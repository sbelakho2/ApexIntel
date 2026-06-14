//! Certificate Transparency Log Analysis Module
//!
//! Wraps the existing `CtMonitor` from `crate::ct` and adds structured
//! CT log analysis helpers for ApexIntel entities:
//! - New certificate alerts for monitored domains
//! - Lookalike domain detection
//! - Subdomain discovery via CT logs
//! - Certificate misuse / unexpected issuer alerts

use crate::ct::{CtAlert, CtAlertType, CtCertificate, CtMonitor, CtMonitorConfig};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// High-level CT log querier for ApexIntel entities.
#[derive(Debug, Clone)]
pub struct CtLogQuerier {
    monitor: CtMonitor,
    /// Domains being actively monitored.
    monitored_domains: HashSet<String>,
}

impl CtLogQuerier {
    /// Create with default configuration.
    pub fn new() -> Self {
        let config = CtMonitorConfig::default();
        let monitor = CtMonitor::new(config);
        Self {
            monitor,
            monitored_domains: HashSet::new(),
        }
    }

    /// Create with custom monitor configuration.
    pub fn with_config(config: CtMonitorConfig) -> Self {
        let monitor = CtMonitor::new(config);
        Self {
            monitor,
            monitored_domains: HashSet::new(),
        }
    }

    /// Register a domain for ongoing monitoring.
    pub fn monitor_domain(&mut self, domain: impl Into<String>) {
        self.monitored_domains.insert(domain.into());
    }

    /// Search for certificates for a domain.
    pub async fn search(&self, domain: &str) -> Result<Vec<CtCertificate>> {
        self.monitor.search_certificates(domain, false).await
    }

    /// Search for lookalike certificates for a base domain.
    pub async fn search_lookalikes(&self, base_domain: &str) -> Result<Vec<CtCertificate>> {
        self.monitor.search_lookalikes(base_domain).await
    }

    /// Discover subdomains for a domain via CT logs.
    pub async fn discover_subdomains(&self, domain: &str) -> Result<Vec<String>> {
        self.monitor.discover_subdomains(domain).await
    }

    /// Run all monitored domains and collect alerts.
    pub async fn run_monitoring(&self) -> Result<Vec<CtAlert>> {
        self.monitor.monitor().await
    }
}

/// A subdomain discovery result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubdomainDiscoveryResult {
    pub domain: String,
    pub discovered_subdomains: Vec<String>,
    pub discovered_at: DateTime<Utc>,
    pub scan_id: String,
}

impl SubdomainDiscoveryResult {
    /// Create a new discovery result.
    pub fn new(domain: String, subdomains: Vec<String>) -> Self {
        let scan_id = format!("ct-sub-{}-{}", domain, Utc::now().timestamp());
        Self {
            domain,
            discovered_subdomains: subdomains,
            discovered_at: Utc::now(),
            scan_id,
        }
    }

    /// Return the count of new subdomains.
    pub fn count(&self) -> usize {
        self.discovered_subdomains.len()
    }

    /// Return subdomains that appear to be staging/preview environments.
    pub fn staging_candidates(&self) -> Vec<&String> {
        let patterns = [
            "staging", "preview", "dev", "test", "qa", "beta", "demo", "stage",
        ];
        self.discovered_subdomains
            .iter()
            .filter(|s| {
                let lower = s.to_lowercase();
                patterns.iter().any(|p| lower.contains(p))
            })
            .collect()
    }

    /// Return subdomains that appear to be cloud infrastructure.
    pub fn cloud_candidates(&self) -> Vec<&String> {
        let patterns = [
            "aws", "ec2", "s3", "azure", "gcp", "google", "cloud", "cdn",
            "fastly", "akamai", "cloudfront",
        ];
        self.discovered_subdomains
            .iter()
            .filter(|s| {
                let lower = s.to_lowercase();
                patterns.iter().any(|p| lower.contains(p))
            })
            .collect()
    }
}

/// Lookalike certificate alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeAlert {
    pub original_domain: String,
    pub lookalike_domain: String,
    pub similarity_score: f32,
    pub certificate_issuer: String,
    pub certificate_log_source: String,
    pub detected_at: DateTime<Utc>,
    pub severity: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_log_querier_constructs() {
        let querier = CtLogQuerier::new();
        assert!(querier.monitored_domains.is_empty());
    }

    #[test]
    fn subdomain_discovery_result_staging() {
        let result = SubdomainDiscoveryResult::new(
            "example.com".to_string(),
            vec![
                "www.example.com".to_string(),
                "staging.example.com".to_string(),
                "preview.example.com".to_string(),
                "api.example.com".to_string(),
            ],
        );
        assert_eq!(result.count(), 4);
        assert_eq!(result.staging_candidates().len(), 2);
        assert!(result.cloud_candidates().is_empty());
    }

    #[test]
    fn subdomain_discovery_result_cloud() {
        let result = SubdomainDiscoveryResult::new(
            "example.com".to_string(),
            vec![
                "cdn.example.com".to_string(),
                "aws-lb.example.com".to_string(),
                "www.example.com".to_string(),
            ],
        );
        assert_eq!(result.cloud_candidates().len(), 2);
    }

    #[test]
    fn lookalike_alert_debug() {
        let alert = LookalikeAlert {
            original_domain: "google.com".to_string(),
            lookalike_domain: "g00gle.com".to_string(),
            similarity_score: 0.85,
            certificate_issuer: "DigiCert".to_string(),
            certificate_log_source: "crt.sh".to_string(),
            detected_at: Utc::now(),
            severity: "high".to_string(),
        };
        assert_eq!(alert.original_domain, "google.com");
        assert!(alert.similarity_score > 0.8);
    }
}
