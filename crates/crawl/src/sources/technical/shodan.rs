//! Shodan Integration Module

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;

pub type ShodanApiKey = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanHost {
    pub ip_str: String,
    pub port: u16,
    pub transport: String,
    pub version: Option<String>,
    pub product: Option<String>,
    pub org: Option<String>,
    pub os: Option<String>,
    pub location: ShodanLocation,
    pub vulns: Option<HashMap<String, ShodanVulnerability>>,
    pub fetched_at: DateTime<Utc>,
}

impl ShodanHost {
    pub fn has_vulnerabilities(&self) -> bool {
        self.vulns.as_ref().map(|v| !v.is_empty()).unwrap_or(false)
    }
    pub fn vuln_count(&self) -> usize {
        self.vulns.as_ref().map(|v| v.len()).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanLocation {
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanVulnerability {
    pub cve: String,
    pub cvss: f32,
    pub severity: String,
    pub summary: String,
}

impl ShodanVulnerability {
    pub fn is_critical(&self) -> bool { self.cvss >= 9.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct ShodanSearchResponse {
    pub total: Option<u32>,
    pub matches: Option<Vec<ShodanHost>>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct AlertsResponse {
    pub triggers: Option<Vec<ShodanAlert>>,
}


pub struct ShodanQuery {
    pub query: String,
    pub page: u32,
    pub limit: u32,
}

impl Default for ShodanQuery {
    fn default() -> Self { Self { query: String::new(), page: 1, limit: 100 } }
}

impl ShodanQuery {
    pub fn new(query: impl Into<String>) -> Self { Self { query: query.into(), ..Default::default() } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanMonitorConfig {
    pub api_key: Option<ShodanApiKey>,
    pub timeout_secs: u64,
}

impl Default for ShodanMonitorConfig {
    fn default() -> Self { Self { api_key: None, timeout_secs: 30 } }
}

#[derive(Debug, Clone)]
pub struct ShodanClient {
    client: Client,
    config: ShodanMonitorConfig,
}

impl ShodanClient {
    pub fn new(api_key: ShodanApiKey) -> Result<Self> {
        Self::with_config(ShodanMonitorConfig { api_key: Some(api_key), ..Default::default() })
    }

    pub fn with_config(config: ShodanMonitorConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) Shodan Monitor")
            .build()
            .context("building Shodan HTTP client")?;
        Ok(Self { client, config })
    }

    fn api_key(&self) -> Result<&str> {
        self.config.api_key.as_deref().context("Shodan API key required")
    }

    pub async fn search(&self, query: &ShodanQuery) -> Result<ShodanSearchResult> {
        let url = "https://api.shodan.io/shodan/host/search";
        let resp = self.client.get(url)
            .query(&[("key", self.api_key().unwrap_or(""))])
            .query(&[("query", &query.query)])
            .query(&[("page", &query.page.to_string())])
            .query(&[("limit", &query.limit.to_string())])
            .send().await.context("Shodan search request")?;

        if !resp.status().is_success() { anyhow::bail!("Shodan search returned {}", resp.status()); }

        let search_resp: ShodanSearchResponse = resp.json().await.context("parse Shodan response")?;
        let total = search_resp.total.unwrap_or(0);
        let hosts = search_resp.matches.unwrap_or_default();

        debug!(query = %query.query, total = total, "Shodan search complete");
        Ok(ShodanSearchResult { query: query.query.clone(), total, hosts, facets: None })
    }

    pub async fn host(&self, ip: &str) -> Result<ShodanHost> {
        let url = format!("https://api.shodan.io/shodan/host/{}", urlencoding::encode(ip));
        let resp = self.client.get(&url)
            .query(&[("key", self.api_key().unwrap_or(""))])
            .send().await.context("Shodan host request")?;

        if !resp.status().is_success() { anyhow::bail!("Shodan host request returned {}", resp.status()); }
        let host: ShodanHost = resp.json().await.context("parse Shodan host response")?;
        Ok(host)
    }

    pub async fn alerts(&self) -> Result<Vec<ShodanAlert>> {
        let url = "https://api.shodan.io/shodan/alert/info";
        let resp = self.client.get(url)
            .query(&[("key", self.api_key().unwrap_or(""))])
            .send().await.context("Shodan alerts request")?;

        let alerts_resp: AlertsResponse = resp.json().await.unwrap_or(AlertsResponse { triggers: None });
        Ok(alerts_resp.triggers.unwrap_or_default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanSearchResult {
    pub query: String,
    pub total: u32,
    pub hosts: Vec<ShodanHost>,
    pub facets: Option<serde_json::Value>,
}

impl ShodanSearchResult {
    pub fn vulnerable_hosts(&self) -> Vec<&ShodanHost> {
        self.hosts.iter().filter(|h| h.has_vulnerabilities()).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShodanAlert {
    pub id: Option<String>,
    pub name: Option<String>,
    pub ip: Option<String>,
    pub created: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shodan_host_vuln_count() {
        let mut vulns = HashMap::new();
        vulns.insert("CVE-2021-44228".to_string(), ShodanVulnerability {
            cve: "CVE-2021-44228".to_string(), cvss: 10.0,
            severity: "CRITICAL".to_string(), summary: "Log4Shell".to_string(),
        });
        let host = ShodanHost {
            ip_str: "192.0.2.1".to_string(), port: 8080, transport: "tcp".to_string(),
            version: None, product: None, org: None, os: None,
            location: ShodanLocation { city: None, country_code: None, latitude: None, longitude: None },
            vulns: Some(vulns), fetched_at: Utc::now(),
        };
        assert!(host.has_vulnerabilities());
        assert_eq!(host.vuln_count(), 1);
    }

    #[test]
    fn shodan_query_builder() {
        let query = ShodanQuery::new("apache country:US");
        assert_eq!(query.query, "apache country:US");
    }

    #[test]
    fn shodan_search_result_vulnerable() {
        let result = ShodanSearchResult { query: "test".to_string(), total: 2, hosts: vec![], facets: None };
        assert!(result.vulnerable_hosts().is_empty());
    }
}
