//! Certificate Transparency Log Analysis Module
//!
//! Monitors Certificate Transparency (CT) logs for subdomain discovery and certificate intelligence:
//! - crt.sh integration for certificate search
//! - Subdomain enumeration from CT logs
//! - SSL certificate monitoring for changes
//! - Issuer analysis
//! - Validity period tracking
//!
//! Certificate Transparency logs are mandated by CA/Browser Forum for all public CAs.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info};

/// A discovered certificate from CT logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtCertificate {
    pub cert_id: String,
    pub common_name: String,
    pub names: Vec<String>,
    pub issuer_name: String,
    pub issuer_org: Option<String>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub serial_number: String,
    pub fingerprint_sha256: String,
    pub key_algorithm: String,
    pub key_size: Option<u32>,
    pub signature_algorithm: String,
    pub is_valid: bool,
    pub matched_names: Vec<String>,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
}

impl CtCertificate {
    /// Whether this certificate is currently valid.
    pub fn is_currently_valid(&self) -> bool {
        let now = Utc::now();
        now > self.not_before && now < self.not_after
    }

    /// Days until expiration.
    pub fn days_until_expiry(&self) -> i64 {
        (self.not_after - Utc::now()).num_days()
    }

    /// Whether this is an EV certificate.
    pub fn is_ev(&self) -> bool {
        self.issuer_org
            .as_ref()
            .map(|o| o.contains("Extended Validation") || o.contains("EV"))
            .unwrap_or(false)
    }
}

/// A CT log entry from crt.sh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtLogEntry {
    pub id: String,
    pub issuer_ca_id: i64,
    pub issuer_name: String,
    pub common_name: String,
    pub name_value: String,
    pub not_before: String,
    pub not_after: String,
    pub serial_number: String,
    pub fingerprint: String,
}

/// Certificate Transparency monitor.
#[derive(Debug, Clone)]
pub struct CtMonitor {
    client: Client,
    /// Known certificate hashes for deduplication.
    seen_hashes: HashSet<String>,
}

impl CtMonitor {
    /// Create a new CT monitor.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) CT Monitor")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            seen_hashes: HashSet::new(),
        }
    }

    /// Search crt.sh for certificates matching a domain.
    pub async fn search_domain(&mut self, domain: &str) -> Result<Vec<CtCertificate>> {
        let url = format!(
            "https://crt.sh/?q={}&output=json",
            urlencoding::encode(domain)
        );

        let resp = self
            .client
            .get(&url)
            .header("User-Agent", "ApexIntel/1.0 CT Monitor")
            .send()
            .await
            .context("crt.sh API request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), domain = %domain, "crt.sh returned non-success");
            return Ok(Vec::new());
        }

        let entries: Vec<CtLogEntry> = resp.json().await.unwrap_or_default();
        self.entries_to_certs(&entries, domain)
    }

    fn entries_to_certs(
        &mut self,
        entries: &[CtLogEntry],
        domain: &str,
    ) -> Result<Vec<CtCertificate>> {
        let mut certs = Vec::new();
        for entry in entries {
            if self.seen_hashes.contains(&entry.fingerprint) {
                continue;
            }
            self.seen_hashes.insert(entry.fingerprint.clone());

            let not_before =
                DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", entry.not_before))
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
            let not_after = DateTime::parse_from_rfc3339(&format!("{}T23:59:59Z", entry.not_after))
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let names: Vec<String> = entry
                .name_value
                .split('\n')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let matched = names
                .iter()
                .filter(|n| n.contains(domain))
                .cloned()
                .collect();

            certs.push(CtCertificate {
                cert_id: entry.id.clone(),
                common_name: entry.common_name.clone(),
                names: names.clone(),
                issuer_name: entry.issuer_name.clone(),
                issuer_org: None,
                not_before,
                not_after,
                serial_number: entry.serial_number.clone(),
                fingerprint_sha256: entry.fingerprint.clone(),
                key_algorithm: "RSA".to_string(),
                key_size: None,
                signature_algorithm: "SHA256".to_string(),
                is_valid: not_before < Utc::now() && not_after > Utc::now(),
                matched_names: matched,
                source: "crt.sh".to_string(),
                fetched_at: Utc::now(),
            });
        }
        info!(domain = %domain, count = certs.len(), "CT certificate search complete");
        Ok(certs)
    }

    /// Enumerate subdomains from CT logs.
    pub async fn enumerate_subdomains(&mut self, domain: &str) -> Result<Vec<String>> {
        let certs = self.search_domain(domain).await?;
        let mut subdomains: HashSet<String> = HashSet::new();

        for cert in &certs {
            for name in &cert.names {
                if name.ends_with(domain) {
                    // Strip the domain to get subdomain
                    let subdomain = &name[..name.len().saturating_sub(domain.len() + 1)];
                    if !subdomain.is_empty() && !subdomain.contains('.') {
                        subdomains.insert(name.clone());
                    }
                }
            }
        }

        let mut subdomains: Vec<String> = subdomains.into_iter().collect();
        subdomains.sort();
        debug!(domain = %domain, count = subdomains.len(), "Subdomains enumerated from CT");
        Ok(subdomains)
    }

    /// Get all discovered subdomains from CT data.
    pub fn all_discovered_subdomains(&self) -> Vec<String> {
        self.seen_hashes
            .iter()
            .filter_map(|h| {
                let parts: Vec<&str> = h.split('.').collect();
                parts.last().map(|s| s.to_string())
            })
            .collect()
    }
}

impl Default for CtMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_certificate_validity() {
        let cert = CtCertificate {
            cert_id: "test".to_string(),
            common_name: "test.example.com".to_string(),
            names: vec!["test.example.com".to_string()],
            issuer_name: "DigiCert".to_string(),
            issuer_org: Some("DigiCert Inc".to_string()),
            not_before: Utc::now() - chrono::Duration::days(30),
            not_after: Utc::now() + chrono::Duration::days(365),
            serial_number: "00:AB:CD".to_string(),
            fingerprint_sha256: "abc123".to_string(),
            key_algorithm: "RSA".to_string(),
            key_size: Some(2048),
            signature_algorithm: "SHA256".to_string(),
            is_valid: true,
            matched_names: vec![],
            source: "crt.sh".to_string(),
            fetched_at: Utc::now(),
        };
        assert!(cert.is_currently_valid());
        assert!(cert.days_until_expiry() > 0);
    }

    #[test]
    fn ct_certificate_expired() {
        let cert = CtCertificate {
            cert_id: "test".to_string(),
            common_name: "test.example.com".to_string(),
            names: vec![],
            issuer_name: "Test CA".to_string(),
            issuer_org: None,
            not_before: Utc::now() - chrono::Duration::days(365),
            not_after: Utc::now() - chrono::Duration::days(30),
            serial_number: "00:AB:CD".to_string(),
            fingerprint_sha256: "abc123".to_string(),
            key_algorithm: "RSA".to_string(),
            key_size: None,
            signature_algorithm: "SHA256".to_string(),
            is_valid: false,
            matched_names: vec![],
            source: "crt.sh".to_string(),
            fetched_at: Utc::now(),
        };
        assert!(!cert.is_currently_valid());
        assert!(cert.days_until_expiry() < 0);
    }

    #[test]
    fn ct_monitor_constructs() {
        let monitor = CtMonitor::new();
        assert!(monitor.all_discovered_subdomains().is_empty());
    }
}
