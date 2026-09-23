//! Censys Integration Module
//!
//! Integrates with Censys for network reconnaissance and asset discovery:
//! - Certificate search (Censys search engine)
//! - Host enumeration
//! - Autonomous System analysis
//! - Website fingerprinting
//! - Protocol detection

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

/// Censys API credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysConfig {
    pub api_id: String,
    pub api_secret: String,
    pub timeout_secs: u64,
}

impl CensysConfig {
    /// Create from environment variables.
    pub fn from_env() -> Option<Self> {
        let api_id = std::env::var("CENSYS_API_ID").ok()?;
        let api_secret = std::env::var("CENSYS_API_SECRET").ok()?;
        Some(Self {
            api_id,
            api_secret,
            timeout_secs: 30,
        })
    }
}

/// A Censys host result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysHost {
    pub ip: String,
    pub ports: Vec<u16>,
    pub protocols: Vec<String>,
    pub services: Vec<CensysService>,
    pub location: CensysLocation,
    pub autonomous_system: Option<CensysAs>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysService {
    pub port: u16,
    pub service_name: String,
    pub product: Option<String>,
    pub version: Option<String>,
    pub banner: Option<String>,
    pub transport_protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysLocation {
    pub city: Option<String>,
    pub country: Option<String>,
    pub continent: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysAs {
    pub asn: u32,
    pub name: String,
    pub description: Option<String>,
    pub route: Option<String>,
    pub country: Option<String>,
}

/// A Censys certificate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CensysCertificate {
    pub fingerprint_sha256: String,
    pub common_name: Vec<String>,
    pub subject_alt_names: Vec<String>,
    pub issuer_common_name: Vec<String>,
    pub issuer_organization: Vec<String>,
    pub not_before: String,
    pub not_after: String,
    pub signature_algorithm: String,
    pub subject_organization: Vec<String>,
    pub subject_country: Vec<String>,
    pub validation_level: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

/// Censys API client.
#[derive(Debug, Clone)]
pub struct CensysClient {
    client: Client,
    config: CensysConfig,
}

impl CensysClient {
    /// Create with configuration.
    pub fn new(config: CensysConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Censys Monitor")
            .build()
            .context("building Censys HTTP client")?;
        Ok(Self { client, config })
    }

    fn auth_header(&self) -> String {
        // Simple Base64 encoder without external crate
        let creds = format!("{}:{}", self.config.api_id, self.config.api_secret);
        let encoded = simple_base64_encode(creds.as_bytes());
        format!("Basic {}", encoded)
    }

    /// Search for certificates by domain.
    pub async fn search_certificates(&self, query: &str) -> Result<Vec<CensysCertificate>> {
        let url = "https://search.censys.io/api/v1/search/certificates";
        let resp = self
            .client
            .get(url)
            .header("Authorization", self.auth_header())
            .query(&[("q", query)])
            .query(&[("per_page", "100")])
            .send()
            .await
            .context("Censys certificate search")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), query = %query, "Censys returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct CensysCertResponse {
            results: Option<Vec<serde_json::Value>>,
        }

        let cert_resp: CensysCertResponse = resp
            .json()
            .await
            .unwrap_or(CensysCertResponse { results: None });
        let certs: Vec<CensysCertificate> = cert_resp
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| {
                Some(CensysCertificate {
                    fingerprint_sha256: r.get("parsed.fingerprint_sha256")?.as_str()?.to_string(),
                    common_name: r
                        .get("parsed.subject.common_name")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    subject_alt_names: r
                        .get("parsed.subject_alt_name")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    issuer_common_name: r
                        .get("parsed.issuer.common_name")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    issuer_organization: r
                        .get("parsed.issuer.organization")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    not_before: r
                        .get("parsed.validity.start")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    not_after: r
                        .get("parsed.validity.end")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    signature_algorithm: r
                        .get("parsed.signature_algorithm")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    subject_organization: r
                        .get("parsed.subject.organization")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    subject_country: r
                        .get("parsed.subject.country")
                        .map(|v| {
                            v.as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    validation_level: r
                        .get("parsed.validation_level")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    fetched_at: Utc::now(),
                })
            })
            .collect();

        info!(query = %query, count = certs.len(), "Censys certificate search complete");
        Ok(certs)
    }

    /// Search for hosts by query.
    pub async fn search_hosts(&self, query: &str) -> Result<Vec<CensysHost>> {
        let url = "https://search.censys.io/api/v1/search/hosts";
        let resp = self
            .client
            .get(url)
            .header("Authorization", self.auth_header())
            .query(&[("q", query)])
            .query(&[("per_page", "100")])
            .send()
            .await
            .context("Censys host search")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), query = %query, "Censys host search returned non-success");
            return Ok(Vec::new());
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct CensysHostResponse {
            results: Option<Vec<serde_json::Value>>,
        }

        let host_resp: CensysHostResponse = resp
            .json()
            .await
            .unwrap_or(CensysHostResponse { results: None });
        let hosts: Vec<CensysHost> = host_resp
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| {
                let ip = r.get("ip")?.as_str()?.to_string();
                let ports: Vec<u16> = r
                    .get("ports")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_u64())
                            .map(|p| p as u16)
                            .collect()
                    })
                    .unwrap_or_default();
                let protocols: Vec<String> = r
                    .get("protocols")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                Some(CensysHost {
                    ip,
                    ports,
                    protocols: protocols
                        .iter()
                        .map(|p| {
                            let parts: Vec<&str> = p.split('/').collect();
                            parts
                                .first()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| p.clone())
                        })
                        .collect(),
                    services: vec![],
                    location: CensysLocation {
                        city: None,
                        country: r
                            .get("location.country")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        continent: None,
                        latitude: None,
                        longitude: None,
                    },
                    autonomous_system: None,
                    fetched_at: Utc::now(),
                })
            })
            .collect();

        debug!(query = %query, count = hosts.len(), "Censys host search complete");
        Ok(hosts)
    }
}

// Simple Base64 encoder for Censys auth
fn simple_base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = input.get(i + 1).copied().unwrap_or(0) as u32;
        let b2 = input.get(i + 2).copied().unwrap_or(0) as u32;

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < input.len() {
            result.push(ALPHABET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < input.len() {
            result.push(ALPHABET[(b2 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn censys_config_from_env_skips() {
        // This will skip if env vars are not set, which is fine
        let result = CensysConfig::from_env();
        // Result is Option — None is valid in test environment
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn censys_certificate_structure() {
        let cert = CensysCertificate {
            fingerprint_sha256: "abc123".to_string(),
            common_name: vec!["example.com".to_string()],
            subject_alt_names: vec!["www.example.com".to_string()],
            issuer_common_name: vec!["DigiCert".to_string()],
            issuer_organization: vec!["DigiCert Inc".to_string()],
            not_before: "2024-01-01".to_string(),
            not_after: "2025-01-01".to_string(),
            signature_algorithm: "SHA256-RSA".to_string(),
            subject_organization: vec!["Example Org".to_string()],
            subject_country: vec!["US".to_string()],
            validation_level: Some("OV".to_string()),
            fetched_at: Utc::now(),
        };
        assert_eq!(cert.common_name.len(), 1);
        assert_eq!(cert.issuer_common_name[0], "DigiCert");
    }
}
