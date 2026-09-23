//! WHOIS Data Collection Module
//!
//! Collects and parses WHOIS data for domain intelligence:
//! - Domain registration information
//! - Registrar details
//! - Name server information
//! - Registrant data (when available)
//! - Domain age and expiration tracking
//! - Historical WHOIS data

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::info;

/// A parsed WHOIS record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisRecord {
    pub domain: String,
    pub registrar: Option<String>,
    pub registrar_url: Option<String>,
    pub registration_date: Option<NaiveDate>,
    pub expiration_date: Option<NaiveDate>,
    pub updated_date: Option<NaiveDate>,
    pub name_servers: Vec<String>,
    pub status: Vec<String>,
    pub dnssec: Option<String>,
    pub registrant_name: Option<String>,
    pub registrant_org: Option<String>,
    pub registrant_country: Option<String>,
    pub registrant_state: Option<String>,
    pub registrant_city: Option<String>,
    pub admin_name: Option<String>,
    pub admin_email: Option<String>,
    pub tech_name: Option<String>,
    pub tech_email: Option<String>,
    pub raw_text: String,
    pub fetched_at: DateTime<Utc>,
}

impl WhoisRecord {
    /// Whether the domain is expired.
    pub fn is_expired(&self) -> bool {
        self.expiration_date
            .map(|d| d < Utc::now().date_naive())
            .unwrap_or(false)
    }

    /// Days until expiration.
    pub fn days_until_expiry(&self) -> Option<i64> {
        self.expiration_date
            .map(|d| (d - Utc::now().date_naive()).num_days())
    }

    /// Domain age in days.
    pub fn domain_age_days(&self) -> Option<i64> {
        self.registration_date
            .map(|r| (Utc::now().date_naive() - r).num_days())
    }

    /// Whether DNSSEC is enabled.
    pub fn has_dnssec(&self) -> bool {
        self.dnssec
            .as_ref()
            .map(|d| !d.is_empty() && d != "unsigned")
            .unwrap_or(false)
    }
}

/// WHOIS lookup configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisConfig {
    /// WHOIS server for the TLD.
    pub whois_server: Option<String>,
    /// Port for WHOIS queries.
    pub port: u16,
    /// Connection timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to follow referrals.
    pub follow_referrals: bool,
}

impl Default for WhoisConfig {
    fn default() -> Self {
        Self {
            whois_server: None,
            port: 43,
            timeout_secs: 10,
            follow_referrals: true,
        }
    }
}

/// WHOIS client for domain lookups.
#[derive(Debug, Clone)]
pub struct WhoisClient {
    config: WhoisConfig,
}

impl WhoisClient {
    /// Create with default config.
    pub fn new() -> Self {
        Self {
            config: WhoisConfig::default(),
        }
    }

    /// Create with custom config.
    pub fn with_config(config: WhoisConfig) -> Self {
        Self { config }
    }

    /// Lookup a domain.
    pub async fn lookup(&self, domain: &str) -> Result<WhoisRecord> {
        let whois_server = self
            .config
            .whois_server
            .clone()
            .or_else(|| self.guess_whois_server(domain))
            .unwrap_or_else(|| "whois.verisign-grs.com".to_string());

        info!(domain = %domain, server = %whois_server, "WHOIS lookup");
        let raw = self.query_whois(domain, &whois_server).await?;
        Ok(self.parse_whois(&raw, domain))
    }

    async fn query_whois(&self, domain: &str, server: &str) -> Result<String> {
        // Connect to WHOIS server
        let addr = format!("{}:{}", server, self.config.port);
        let mut stream = TcpStream::connect(&addr)
            .await
            .context("WHOIS TCP connection")?;

        // Send query
        let query = format!("{}\r\n", domain);
        stream
            .write_all(query.as_bytes())
            .await
            .context("WHOIS query send")?;

        // Read response
        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        let timeout_secs = self.config.timeout_secs;
        let deadline = tokio::time::Instant::now()
            .checked_add(tokio::time::Duration::from_secs(timeout_secs))
            .unwrap_or_else(|| {
                tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs)
            });

        let _ = tokio::time::timeout_at(deadline, async {
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => response.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
        })
        .await;

        Ok(String::from_utf8_lossy(&response).into_owned())
    }

    fn parse_whois(&self, raw: &str, domain: &str) -> WhoisRecord {
        use regex::Regex;

        let get = |pattern: &str| -> Option<String> {
            let re = Regex::new(pattern).ok()?;
            re.captures(raw)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
        };

        let get_all = |pattern: &str| -> Vec<String> {
            let re = match Regex::new(pattern) {
                Ok(r) => r,
                Err(_) => return Vec::new(),
            };
            re.captures_iter(raw)
                .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                .collect()
        };

        let parse_date = |s: &str| -> Option<NaiveDate> {
            NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
                .ok()
                .or_else(|| NaiveDate::parse_from_str(s.trim(), "%d-%b-%Y").ok())
        };

        WhoisRecord {
            domain: domain.to_string(),
            registrar: get(r"(?i)Registrar:\s*(.+)"),
            registrar_url: get(r"(?i)Registrar URL:\s*(.+)"),
            registration_date: get(r"(?i)Created\s*Date:\s*(.+)").and_then(|s| parse_date(&s)),
            expiration_date: get(r"(?i)(?:Expir(?:y|ation)\s*Date|Registry\s*Expiry):\s*(.+)")
                .and_then(|s| parse_date(&s)),
            updated_date: get(r"(?i)Updated\s*Date:\s*(.+)").and_then(|s| parse_date(&s)),
            name_servers: get_all(r"(?i)Name Server:\s*(.+)")
                .into_iter()
                .filter(|ns| ns != "whois.verisign-grs.com" && !ns.is_empty())
                .collect(),
            status: get_all(r"(?i)Status:\s*(.+)"),
            dnssec: get(r"(?i)DNSSEC:\s*(.+)"),
            registrant_name: get(r"(?i)Registrant Name:\s*(.+)"),
            registrant_org: get(r"(?i)Registrant Organization:\s*(.+)"),
            registrant_country: get(r"(?i)Registrant Country:\s*(.+)"),
            registrant_state: get(r"(?i)Registrant State/Province:\s*(.+)"),
            registrant_city: get(r"(?i)Registrant City:\s*(.+)"),
            admin_name: get(r"(?i)Admin Name:\s*(.+)"),
            admin_email: get(r"(?i)Admin Email:\s*(.+)"),
            tech_name: get(r"(?i)Tech Name:\s*(.+)"),
            tech_email: get(r"(?i)Tech Email:\s*(.+)"),
            raw_text: raw.to_string(),
            fetched_at: Utc::now(),
        }
    }

    fn guess_whois_server(&self, domain: &str) -> Option<String> {
        let tld = domain.split('.').next_back()?;
        let server = match tld {
            "com" | "net" | "cc" | "tv" => "whois.verisign-grs.com",
            "org" => "whois.pir.org",
            "io" => "whois.nic.io",
            "co" => "whois.nic.co",
            "biz" => "whois.neulevel.biz",
            "info" => "whois.nic.info",
            "me" => "whois.nic.me",
            "ru" => "whois.tcinet.ru",
            "cn" => "whois.cnnic.cn",
            "de" => "whois.denic.de",
            "uk" => "whois.nic.uk",
            "au" => "whois.auda.org.au",
            "jp" => "whois.jprs.jp",
            _ => "whois.verisign-grs.com",
        };
        Some(server.to_string())
    }
}

impl Default for WhoisClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn whois_record_is_expired() {
        let record = WhoisRecord {
            domain: "example.com".to_string(),
            registrar: None,
            registrar_url: None,
            registration_date: Some(Utc::now().date_naive() - chrono::Duration::days(365 * 2)),
            expiration_date: Some(Utc::now().date_naive() - chrono::Duration::days(30)),
            updated_date: None,
            name_servers: vec![],
            status: vec![],
            dnssec: None,
            registrant_name: None,
            registrant_org: None,
            registrant_country: None,
            registrant_state: None,
            registrant_city: None,
            admin_name: None,
            admin_email: None,
            tech_name: None,
            tech_email: None,
            raw_text: "".to_string(),
            fetched_at: Utc::now(),
        };
        assert!(record.is_expired());
        assert!(record.days_until_expiry().is_some_and(|d| d < 0));
    }

    #[test]
    fn whois_record_domain_age() {
        let record = WhoisRecord {
            domain: "example.com".to_string(),
            registrar: None,
            registrar_url: None,
            registration_date: Some(Utc::now().date_naive() - chrono::Duration::days(365)),
            expiration_date: None,
            updated_date: None,
            name_servers: vec![],
            status: vec![],
            dnssec: None,
            registrant_name: None,
            registrant_org: None,
            registrant_country: None,
            registrant_state: None,
            registrant_city: None,
            admin_name: None,
            admin_email: None,
            tech_name: None,
            tech_email: None,
            raw_text: "".to_string(),
            fetched_at: Utc::now(),
        };
        assert!(record
            .domain_age_days()
            .is_some_and(|d| (d - 365).abs() <= 1));
    }

    #[test]
    fn whois_client_constructs() {
        let client = WhoisClient::new();
        let server = client.guess_whois_server("example.com");
        assert!(server.is_some());
        assert!(server.unwrap().contains("verisign"));
    }

    #[test]
    fn whois_client_tld_servers() {
        let client = WhoisClient::new();
        assert_eq!(
            client.guess_whois_server("example.org").unwrap(),
            "whois.pir.org"
        );
        assert_eq!(
            client.guess_whois_server("example.io").unwrap(),
            "whois.nic.io"
        );
        assert_eq!(
            client.guess_whois_server("example.co").unwrap(),
            "whois.nic.co"
        );
    }
}
