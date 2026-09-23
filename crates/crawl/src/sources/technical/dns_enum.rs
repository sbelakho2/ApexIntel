//! DNS Enumeration and Subdomain Discovery Module

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};
use tracing::{debug, info};

pub const COMMON_SUBDOMAINS: &[&str] = &[
    "www", "mail", "ftp", "admin", "test", "dev", "staging", "api", "app", "mobile", "web", "blog",
    "shop", "cdn", "static", "assets", "images", "dns", "mx", "ns1", "ns2", "smtp", "vpn", "ssh",
    "remote", "git", "ci", "build", "demo", "sandbox", "corp", "intranet", "portal", "oauth",
    "auth", "login", "sso", "status", "monitor", "metrics",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub record_type: DnsRecordType,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsRecordType {
    A,
    Aaaa,
    Cname,
    Mx,
    Ns,
    Txt,
    Unknown,
}

impl DnsRecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Cname => "CNAME",
            Self::Mx => "MX",
            Self::Ns => "NS",
            Self::Txt => "TXT",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsEnumerationResult {
    pub domain: String,
    pub subdomains: Vec<SubdomainEntry>,
    pub scanned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubdomainEntry {
    pub subdomain: String,
    pub full_name: String,
    pub resolved_ips: Vec<IpAddr>,
    pub has_web_service: bool,
    pub first_discovered: DateTime<Utc>,
}

impl SubdomainEntry {
    pub fn fqdn(&self) -> &str {
        &self.full_name
    }
}

pub struct DnsEnumerator {
    wordlist: Vec<String>,
}

impl DnsEnumerator {
    pub fn new() -> Self {
        Self {
            wordlist: COMMON_SUBDOMAINS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn with_wordlist(wordlist: Vec<String>) -> Self {
        Self { wordlist }
    }

    pub async fn enumerate(&self, domain: &str) -> DnsEnumerationResult {
        let mut subdomains = Vec::new();
        let mut seen = HashSet::new();

        info!(domain = %domain, "Starting DNS enumeration");

        for word in &self.wordlist {
            let subdomain = format!("{}.{}", word, domain);
            if seen.contains(&subdomain) {
                continue;
            }
            seen.insert(subdomain.clone());

            // Check if subdomain resolves
            let resolved = tokio::task::spawn_blocking({
                let sd = subdomain.clone();
                move || sd.as_str().to_socket_addrs()
            })
            .await;

            if let Ok(Ok(addrs)) = resolved {
                let ips: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
                if !ips.is_empty() {
                    let has_web = ips.iter().any(|a| a.is_ipv4() || a.is_ipv6());
                    subdomains.push(SubdomainEntry {
                        subdomain: word.clone(),
                        full_name: subdomain.clone(),
                        resolved_ips: ips,
                        has_web_service: has_web,
                        first_discovered: Utc::now(),
                    });
                    debug!(subdomain = %subdomain, "subdomain resolved");
                }
            }
        }

        info!(domain = %domain, subdomains = subdomains.len(), "DNS enumeration complete");
        DnsEnumerationResult {
            domain: domain.to_string(),
            subdomains,
            scanned_at: Utc::now(),
        }
    }
}

impl Default for DnsEnumerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_enumerator_constructs() {
        let e = DnsEnumerator::new();
        assert!(!e.wordlist.is_empty());
    }

    #[test]
    fn subdomain_entry_fqdn() {
        let e = SubdomainEntry {
            subdomain: "www".to_string(),
            full_name: "www.example.com".to_string(),
            resolved_ips: vec![],
            has_web_service: true,
            first_discovered: Utc::now(),
        };
        assert_eq!(e.fqdn(), "www.example.com");
    }

    #[test]
    fn common_subdomains_included() {
        let e = DnsEnumerator::new();
        assert!(e.wordlist.contains(&"www".to_string()));
        assert!(e.wordlist.contains(&"mail".to_string()));
        assert!(e.wordlist.contains(&"api".to_string()));
    }

    #[test]
    fn dns_record_type_as_str() {
        assert_eq!(DnsRecordType::A.as_str(), "A");
        assert_eq!(DnsRecordType::Mx.as_str(), "MX");
    }
}
