//! Certificate Transparency Log Monitoring Module
//!
//! Monitors CT logs for:
//! - Newly issued certificates for monitored domains
//! - Lookalike domain detection via certificate issuance
//! - Subdomain discovery
//! - Certificate misuse detection

use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration as StdDuration;
use tracing::info;

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// A certificate entry from CT logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtCertificate {
    /// SHA256 hash of the certificate
    pub cert_hash: String,
    /// Domain names in the certificate (CN + SANs)
    pub domains: Vec<String>,
    /// Certificate issuer
    pub issuer: String,
    /// Not-before date as stated by the certificate, when parseable.
    pub not_before: Option<DateTime<Utc>>,
    /// Not-after date as stated by the certificate, when parseable.
    pub not_after: Option<DateTime<Utc>>,
    /// When this crawl observed the entry. The crt.sh row does not carry a
    /// log-entry timestamp here, so the honest fact is the observation time.
    pub observed_at: DateTime<Utc>,
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
        let client = crate::http::external_client_or_panic(crate::http::ExternalClientOptions {
            timeout: StdDuration::from_secs(30),
            user_agent: Some("ApexIntel-CtMonitor/1.0".to_string()),
            ..crate::http::ExternalClientOptions::default()
        });

        Self {
            client,
            config,
            crt_sh_endpoint: "https://crt.sh".to_string(),
        }
    }

    /// Search for certificates for a domain.
    ///
    /// Returns a [`ParseOutcome`]: a crt.sh outage is `FetchFailed`, an
    /// unparseable body is `ParseFailed` (with a redacted sample), and only a
    /// successfully deserialized response — empty or not — is
    /// `ParsedSuccessfully`.
    pub async fn search_certificates(
        &self,
        domain: &str,
        include_expired: bool,
    ) -> ParseOutcome<CtCertificate> {
        info!(domain = %domain, "Searching CT logs for certificates");

        let url = format!(
            "{}/?q={}&output=json{}",
            self.crt_sh_endpoint,
            urlencoding::encode(domain),
            if include_expired {
                ""
            } else {
                "&exclude=expired"
            }
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                let outcome =
                    ParseOutcome::fetch_failed(format!("crt.sh request failed: {error}"), None);
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let outcome =
                ParseOutcome::fetch_failed(format!("crt.sh returned HTTP {status}"), Some(status));
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read crt.sh response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        // Handle empty response
        if text.is_empty() || text.trim() == "[]" {
            let outcome = ParseOutcome::parsed(Vec::new());
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let entries: Vec<CrtShEntry> = match serde_json::from_str(&text) {
            Ok(entries) => entries,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse crt.sh JSON: {error}"),
                    &text,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let outcome =
            ParseOutcome::parsed(entries.into_iter().map(|e| e.into_certificate()).collect());
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Monitor domains and generate alerts.
    ///
    /// Fails the caller when any domain's fetch or parse failed: an empty
    /// alert list must only mean "no alerts", never "the parser broke".
    pub async fn monitor(&self) -> Result<Vec<CtAlert>> {
        let mut alerts = Vec::new();

        for domain in &self.config.domains.clone() {
            let certs = match self.search_certificates(domain, false).await {
                ParseOutcome::ParsedSuccessfully { items } => items,
                ParseOutcome::FetchFailed { error, .. } => {
                    anyhow::bail!("crt.sh fetch failed for {domain}: {error}")
                }
                ParseOutcome::ParseFailed { error, .. } => {
                    anyhow::bail!("crt.sh parser failed for {domain}: {error}")
                }
            };

            for cert in certs {
                alerts.extend(self.analyze_certificate(&cert, domain));
            }
        }

        Ok(alerts)
    }

    /// Search for lookalike certificates.
    ///
    /// Parse failures are returned as [`ParseOutcome::ParseFailed`] instead of
    /// being converted into "no lookalikes found".
    pub async fn search_lookalikes(&self, base_domain: &str) -> ParseOutcome<CtCertificate> {
        info!(domain = %base_domain, "Searching for lookalike certificates");

        // Use wildcard search to find similar domains
        let base = base_domain.split('.').next().unwrap_or(base_domain);
        let url = format!(
            "{}/?q=%25{}%25&output=json&exclude=expired",
            self.crt_sh_endpoint,
            urlencoding::encode(base)
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("crt.sh lookalike search failed: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let outcome =
                ParseOutcome::fetch_failed(format!("crt.sh returned HTTP {status}"), Some(status));
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read crt.sh response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if text.is_empty() || text.trim() == "[]" {
            let outcome = ParseOutcome::parsed(Vec::new());
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let entries: Vec<CrtShEntry> = match serde_json::from_str(&text) {
            Ok(entries) => entries,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse crt.sh lookalike JSON: {error}"),
                    &text,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        // Filter to lookalikes (not exact matches)
        let lookalikes = entries
            .into_iter()
            .filter(|e| {
                e.common_name
                    .as_ref()
                    .map(|cn| !cn.ends_with(base_domain) && cn.contains(base))
                    .unwrap_or(false)
            })
            .map(|e| e.into_certificate())
            .collect();

        let outcome = ParseOutcome::parsed(lookalikes);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Get subdomains discovered via CT logs.
    pub async fn discover_subdomains(&self, domain: &str) -> Result<Vec<String>> {
        let certs = match self
            .search_certificates(&format!("%.{}", domain), true)
            .await
        {
            ParseOutcome::ParsedSuccessfully { items } => items,
            ParseOutcome::FetchFailed { error, .. } => {
                anyhow::bail!("crt.sh fetch failed for {domain}: {error}")
            }
            ParseOutcome::ParseFailed { error, .. } => {
                anyhow::bail!("crt.sh parser failed for {domain}: {error}")
            }
        };

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
            let issuer_allowed = self
                .config
                .allowed_issuers
                .iter()
                .any(|allowed| cert.issuer.contains(allowed));

            if !issuer_allowed {
                alerts.push(CtAlert {
                    alert_type: CtAlertType::UnexpectedIssuer,
                    certificate: cert.clone(),
                    monitored_domain: monitored_domain.to_string(),
                    detected_at: now,
                    severity: AlertSeverity::High,
                    description: format!(
                        "Certificate issued by unexpected issuer: {}",
                        cert.issuer
                    ),
                });
            }
        }

        // Check for new subdomain discovery
        for domain in &cert.domains {
            if domain.ends_with(monitored_domain) && domain != monitored_domain {
                let subdomain = domain
                    .strip_suffix(monitored_domain)
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

        // Check for short validity (less than 30 days - potentially
        // suspicious). Both dates must be real: an unparseable validity window
        // is not evidence of a short one.
        if let (Some(not_before), Some(not_after)) = (cert.not_before, cert.not_after) {
            let validity_days = (not_after - not_before).num_days();
            if validity_days < 30 && validity_days > 0 {
                alerts.push(CtAlert {
                    alert_type: CtAlertType::ShortValidity,
                    certificate: cert.clone(),
                    monitored_domain: monitored_domain.to_string(),
                    detected_at: now,
                    severity: AlertSeverity::Medium,
                    description: format!(
                        "Certificate has unusually short validity period: {} days",
                        validity_days
                    ),
                });
            }
        }

        // Check for expiring certificates — only when the certificate states
        // its expiry.
        let days_until_expiry = cert.not_after.map(|not_after| (not_after - now).num_days());
        if let Some(days_until_expiry) = days_until_expiry {
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
        }

        // Check for lookalike domains in SANs
        if self.config.alert_on_lookalikes {
            let base = monitored_domain
                .split('.')
                .next()
                .unwrap_or(monitored_domain);
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
        let domains: Vec<String> = self
            .name_value
            .map(|nv| nv.lines().map(ToString::to_string).collect())
            .unwrap_or_else(|| self.common_name.iter().cloned().collect());

        // Parse dates
        let not_before = self.not_before.as_deref().and_then(parse_crtsh_datetime);
        let not_after = self.not_after.as_deref().and_then(parse_crtsh_datetime);

        CtCertificate {
            cert_hash: self.serial_number.unwrap_or_default(),
            domains,
            issuer: self.issuer_name.unwrap_or_else(|| "Unknown".to_string()),
            not_before,
            not_after,
            observed_at: Utc::now(),
            log_source: "crt.sh".to_string(),
        }
    }
}

fn parse_crtsh_datetime(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            DateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S%z").map(|dt| dt.with_timezone(&Utc))
        })
        .or_else(|_| {
            DateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%z").map(|dt| dt.with_timezone(&Utc))
        })
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

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for (index, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
        row[0] = index;
    }
    for (index, cell) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
        *cell = index;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
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
