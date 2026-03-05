//! Certificate Transparency Log Monitoring Module
//!
//! Monitors CT logs for:
//! - Newly issued certificates for monitored domains
//! - Lookalike domain detection via certificate issuance
//! - Subdomain discovery
//! - Certificate misuse detection

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration as StdDuration;
use tracing::{info, warn};

/// A certificate entry from CT logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtCertificate {
    /// SHA256 hash of the certificate
    pub cert_hash: String,
    /// Domain names in the certificate (CN + SANs)
    pub domains: Vec<String>,
    /// Certificate issuer
    pub issuer: String,
    /// Not before date
    pub not_before: DateTime<Utc>,
    /// Not after date
    pub not_after: DateTime<Utc>,
    /// When this was logged to CT
    pub logged_at: DateTime<Utc>,
    /// CT log source
    pub log_source: String,
}

/// Certificate monitoring alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtAlert {
    pub alert_type: CtAlertType,
    pub certificate: CtCertificate,
    pub monitored_domain: String,
    pub detected_at: DateTime<Utc>,
    pub severity: AlertSeverity,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CtAlertType {
    /// New certificate for monitored domain
    NewCertificate,
    /// Certificate for lookalike domain
    LookalikeCertificate,
    /// Unexpected subdomain discovered
    SubdomainDiscovery,
    /// Certificate from unexpected issuer
    UnexpectedIssuer,
    /// Wildcard certificate issued
    WildcardIssued,
    /// Short validity certificate (potentially suspicious)
    ShortValidity,
    /// Certificate expiring soon
    ExpiringCertificate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// CT log monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtMonitorConfig {
    /// Domains to monitor
    pub domains: Vec<String>,
    /// Allowed issuers (empty = allow all)
    pub allowed_issuers: Vec<String>,
    /// Known subdomains (for discovery alerts)
    pub known_subdomains: HashSet<String>,
    /// Alert on lookalike certificates
    pub alert_on_lookalikes: bool,
    /// Alert on wildcard certificates
    pub alert_on_wildcards: bool,
    /// Days before expiry to alert
    pub expiry_warning_days: i64,
}

impl Default for CtMonitorConfig {
    fn default() -> Self {
        Self {
            domains: Vec::new(),
            allowed_issuers: Vec::new(),
            known_subdomains: HashSet::new(),
            alert_on_lookalikes: true,
            alert_on_wildcards: true,
            expiry_warning_days: 30,
        }
    }
}

/// Certificate Transparency log monitor
pub struct CtMonitor {
    client: Client,
    config: CtMonitorConfig,
    crt_sh_endpoint: String,
}

impl CtMonitor {
    /// Create a new CT monitor
    pub fn new(config: CtMonitorConfig) -> Self {
        let client = Client::builder()
            .timeout(StdDuration::from_secs(30))
            .user_agent("ApexIntel-CtMonitor/1.0")
            .build()
            .expect("Failed to build HTTP client");
        
        Self {
            client,
            config,
            crt_sh_endpoint: "https://crt.sh".to_string(),
        }
    }
    
    /// Search for certificates for a domain
    pub async fn search_certificates(&self, domain: &str, include_expired: bool) -> Result<Vec<CtCertificate>> {
        info!(domain = %domain, "Searching CT logs for certificates");
        
        let url = format!(
            "{}/?q={}&output=json{}",
            self.crt_sh_endpoint,
            urlencoding::encode(domain),
            if include_expired { "" } else { "&exclude=expired" }
        );
        
        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("crt.sh request failed")?;
        
        if !resp.status().is_success() {
            warn!(status = %resp.status(), "crt.sh returned non-success");
            return Ok(Vec::new());
        }
        
        let text = resp.text().await.context("Failed to read crt.sh response")?;
        
        // Handle empty response
        if text.is_empty() || text == "[]" {
            return Ok(Vec::new());
        }
        
        let entries: Vec<CrtShEntry> = serde_json::from_str(&text)
            .context("Failed to parse crt.sh JSON")?;
        
        let certs = entries
            .into_iter()
            .map(|e| e.into_certificate())
            .collect();
        
        Ok(certs)
    }
    
    /// Monitor domains and generate alerts
    pub async fn monitor(&self) -> Result<Vec<CtAlert>> {
        let mut alerts = Vec::new();
        
        for domain in &self.config.domains.clone() {
            let certs = self.search_certificates(domain, false).await?;
            
            for cert in certs {
                alerts.extend(self.analyze_certificate(&cert, domain));
            }
        }
        
        Ok(alerts)
    }
    
    /// Search for lookalike certificates
    pub async fn search_lookalikes(&self, base_domain: &str) -> Result<Vec<CtCertificate>> {
        info!(domain = %base_domain, "Searching for lookalike certificates");
        
        // Use wildcard search to find similar domains
        let base = base_domain.split('.').next().unwrap_or(base_domain);
        let url = format!(
            "{}/?q=%25{}%25&output=json&exclude=expired",
            self.crt_sh_endpoint,
            urlencoding::encode(base)
        );
        
        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("crt.sh lookalike search failed")?;
        
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        
        let text = resp.text().await?;
        if text.is_empty() || text == "[]" {
            return Ok(Vec::new());
        }
        
        let entries: Vec<CrtShEntry> = serde_json::from_str(&text).unwrap_or_default();
        
        // Filter to lookalikes (not exact matches)
        let lookalikes = entries
            .into_iter()
            .filter(|e| {
                e.common_name.as_ref()
                    .map(|cn| !cn.ends_with(base_domain) && cn.contains(base))
                    .unwrap_or(false)
            })
            .map(|e| e.into_certificate())
            .collect();
        
        Ok(lookalikes)
    }
    
    /// Get subdomains discovered via CT logs
    pub async fn discover_subdomains(&self, domain: &str) -> Result<Vec<String>> {
        let certs = self.search_certificates(&format!("%.{}", domain), true).await?;
        
        let mut subdomains: HashSet<String> = HashSet::new();
        
        for cert in certs {
            for d in cert.domains {
                if d.ends_with(domain) && d != domain {
                    subdomains.insert(d);
                }
            }
        }
        
        let mut result: Vec<_> = subdomains.into_iter().collect();
        result.sort();
        Ok(result)
    }
    
    // ─── Private Methods ────────────────────────────────────────────
    
    fn analyze_certificate(&self, cert: &CtCertificate, monitored_domain: &str) -> Vec<CtAlert> {
        let mut alerts = Vec::new();
        let now = Utc::now();
        
        // Check for wildcard certificates
        if self.config.alert_on_wildcards {
            for domain in &cert.domains {
                if domain.starts_with("*.") {
                    alerts.push(CtAlert {
                        alert_type: CtAlertType::WildcardIssued,
                        certificate: cert.clone(),
                        monitored_domain: monitored_domain.to_string(),
                        detected_at: now,
                        severity: AlertSeverity::Medium,
                        description: format!("Wildcard certificate issued for {}", domain),
                    });
                }
            }
        }
        
        // Check for unexpected issuers
        if !self.config.allowed_issuers.is_empty() {
            let issuer_allowed = self.config.allowed_issuers.iter()
                .any(|allowed| cert.issuer.contains(allowed));
            
            if !issuer_allowed {
                alerts.push(CtAlert {
                    alert_type: CtAlertType::UnexpectedIssuer,
                    certificate: cert.clone(),
                    monitored_domain: monitored_domain.to_string(),
                    detected_at: now,
                    severity: AlertSeverity::High,
                    description: format!("Certificate issued by unexpected issuer: {}", cert.issuer),
                });
            }
        }
        
        // Check for new subdomain discovery
        for domain in &cert.domains {
            if domain.ends_with(monitored_domain) && domain != monitored_domain {
                let subdomain = domain.strip_suffix(monitored_domain)
                    .unwrap_or("")
                    .trim_end_matches('.');
                
                if !subdomain.is_empty() && !self.config.known_subdomains.contains(subdomain) {
                    alerts.push(CtAlert {
                        alert_type: CtAlertType::SubdomainDiscovery,
                        certificate: cert.clone(),
                        monitored_domain: monitored_domain.to_string(),
                        detected_at: now,
                        severity: AlertSeverity::Info,
                        description: format!("New subdomain discovered: {}", domain),
                    });
                }
            }
        }
        
        // Check for short validity (less than 30 days - potentially suspicious)
        let validity_days = (cert.not_after - cert.not_before).num_days();
        if validity_days < 30 && validity_days > 0 {
            alerts.push(CtAlert {
                alert_type: CtAlertType::ShortValidity,
                certificate: cert.clone(),
                monitored_domain: monitored_domain.to_string(),
                detected_at: now,
                severity: AlertSeverity::Medium,
                description: format!("Certificate has unusually short validity period: {} days", validity_days),
            });
        }
        
        // Check for expiring certificates
        let days_until_expiry = (cert.not_after - now).num_days();
        if days_until_expiry > 0 && days_until_expiry <= self.config.expiry_warning_days {
            alerts.push(CtAlert {
                alert_type: CtAlertType::ExpiringCertificate,
                certificate: cert.clone(),
                monitored_domain: monitored_domain.to_string(),
                detected_at: now,
                severity: if days_until_expiry <= 7 {
                    AlertSeverity::High
                } else {
                    AlertSeverity::Medium
                },
                description: format!("Certificate expires in {} days", days_until_expiry),
            });
        }
        
        // Check for lookalike domains in SANs
        if self.config.alert_on_lookalikes {
            let base = monitored_domain.split('.').next().unwrap_or(monitored_domain);
            for domain in &cert.domains {
                if !domain.ends_with(monitored_domain) && domain.contains(base) {
                    let similarity = self.calculate_similarity(domain, monitored_domain);
                    if similarity > 0.7 {
                        alerts.push(CtAlert {
                            alert_type: CtAlertType::LookalikeCertificate,
                            certificate: cert.clone(),
                            monitored_domain: monitored_domain.to_string(),
                            detected_at: now,
                            severity: AlertSeverity::High,
                            description: format!(
                                "Certificate issued for lookalike domain: {} ({}% similar)",
                                domain,
                                (similarity * 100.0) as u8
                            ),
                        });
                    }
                }
            }
        }
        
        alerts
    }
    
    fn calculate_similarity(&self, a: &str, b: &str) -> f32 {
        // Simple Levenshtein-based similarity
        let max_len = a.len().max(b.len());
        if max_len == 0 {
            return 1.0;
        }
        
        let distance = levenshtein_distance(a, b);
        1.0 - (distance as f32 / max_len as f32)
    }
}

/// crt.sh JSON response entry
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CrtShEntry {
    id: Option<i64>,
    issuer_ca_id: Option<i64>,
    issuer_name: Option<String>,
    common_name: Option<String>,
    name_value: Option<String>,
    not_before: Option<String>,
    not_after: Option<String>,
    serial_number: Option<String>,
}

impl CrtShEntry {
    fn into_certificate(self) -> CtCertificate {
        // Parse domains from name_value (newline separated)
        let domains: Vec<String> = self.name_value
            .map(|nv| nv.lines().map(ToString::to_string).collect())
            .unwrap_or_else(|| {
                self.common_name.iter().cloned().collect()
            });
        
        // Parse dates
        let not_before = self.not_before
            .as_deref()
            .and_then(parse_crtsh_datetime)
            .unwrap_or_else(Utc::now);
        
        let not_after = self.not_after
            .as_deref()
            .and_then(parse_crtsh_datetime)
            .unwrap_or_else(Utc::now);
        
        CtCertificate {
            cert_hash: self.serial_number.unwrap_or_default(),
            domains,
            issuer: self.issuer_name.unwrap_or_else(|| "Unknown".to_string()),
            not_before,
            not_after,
            logged_at: Utc::now(),
            log_source: "crt.sh".to_string(),
        }
    }
}

fn parse_crtsh_datetime(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| DateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S%z").map(|dt| dt.with_timezone(&Utc)))
        .or_else(|_| DateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%z").map(|dt| dt.with_timezone(&Utc)))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
        .or_else(|_| {
            NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
        .ok()
}

/// Calculate Levenshtein distance between two strings
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    
    if a_len == 0 { return b_len; }
    if b_len == 0 { return a_len; }
    
    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];
    
    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }
    
    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }
    
    matrix[a_len][b_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "abd"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }
    
    #[test]
    fn test_crt_sh_entry_parse() {
        let entry = CrtShEntry {
            id: Some(123),
            issuer_ca_id: Some(456),
            issuer_name: Some("Let's Encrypt".to_string()),
            common_name: Some("example.com".to_string()),
            name_value: Some("example.com\nwww.example.com".to_string()),
            not_before: None,
            not_after: None,
            serial_number: Some("abc123".to_string()),
        };
        
        let cert = entry.into_certificate();
        assert_eq!(cert.domains.len(), 2);
        assert!(cert.domains.contains(&"example.com".to_string()));
        assert!(cert.domains.contains(&"www.example.com".to_string()));
    }
}
