//! NVD CVE vulnerability fetcher.
//!
//! Pulls recently published CVEs from the NVD (National Vulnerability
//! Database) JSON 2.0 feed.  The feed is free and requires no API key — the
//! optional key only raises the per-window rate limit, which we avoid by
//! requesting modest page sizes.
//!
//! Each CVE is projected onto a strongly-typed [`CveVulnerability`] carrying
//! the description, CVSS v3.1 score/severity, affected CPE products, and
//! reference URLs. Every non-success state (network error, HTTP error, rate
//! limit, parse failure) is returned as its explicit
//! [`AcquisitionOutcome`](crate::acquisition::AcquisitionOutcome) variant — an
//! empty result is only ever a genuine successful zero-findings run.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::acquisition::{
    http_failure, retry_after_secs, AcquisitionOutcome, AdapterPrerequisite, SourceAdapter,
};
use apex_core::entities::{Observation, ObservationType};

/// NVD CVE 2.0 JSON endpoint.
const NVD_CVE_API: &str = "https://services.nvd.nist.gov/rest/json/cves/2.0";
/// Hard cap on the number of CVEs returned per call.
const MAX_PAGE_SIZE: u32 = 50;

/// A single CVE projected from the NVD 2.0 feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CveVulnerability {
    /// CVE identifier (e.g. `"CVE-2024-12345"`).
    pub cve_id: String,
    /// English (en) description from `descriptions[]`.
    pub description: String,
    /// CVSS v3.1 base score, when available.
    pub cvss_score: Option<f64>,
    /// CVSS v3.1 base severity label (`LOW`/`MEDIUM`/`HIGH`/`CRITICAL`).
    pub cvss_severity: Option<String>,
    /// Publication timestamp string.
    pub published: Option<String>,
    /// Last-modified timestamp string.
    pub last_modified: Option<String>,
    /// Affected CPE product criteria strings.
    pub affected_products: Vec<String>,
    /// Reference URLs attached to the CVE.
    pub reference_urls: Vec<String>,
}

impl CveVulnerability {
    /// Wrap this CVE into a crawl [`Observation`] of type
    /// [`ObservationType::VulnNotice`].
    pub fn to_observation(&self, entity_id: Option<Uuid>) -> Observation {
        let mut obs = Observation::new(
            ObservationType::VulnNotice,
            Utc::now(),
            serde_json::to_value(self).unwrap_or(json!(null)),
            json!({
                "source": "nvd",
                "url": format!("https://nvd.nist.gov/vuln/detail/{}", self.cve_id),
            }),
        );
        obs.entity_id = entity_id;
        obs
    }
}

/// NVD CVE client.  Owns a `reqwest::Client` with a generous timeout (NVD
/// can be slow under load).
pub struct CveClient {
    client: Client,
}

impl CveClient {
    /// Build a client with a 20s timeout and a descriptive User-Agent.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("ApexIntel-CVE/1.0")
            .build()
            .unwrap_or_else(|error| {
                warn!(error = %error, "cve: failed to build HTTP client; using default");
                Client::new()
            });
        Self { client }
    }

    /// Fetch CVEs published within the last `since_days` days.
    ///
    /// Network, rate-limit, HTTP and parse failures are returned as their
    /// explicit [`AcquisitionOutcome`] variants — never as an empty success.
    pub async fn fetch_recent(&self, since_days: u32) -> AcquisitionOutcome<CveVulnerability> {
        let page_size = MAX_PAGE_SIZE;
        let now = Utc::now();
        // Guard against pathological inputs: clamp to at least 1 day so we
        // always produce a valid, non-empty window.
        let since = now - ChronoDuration::days(since_days.max(1) as i64);

        let url = format!(
            "{NVD_CVE_API}?resultsPerPage={page_size}\
             &pubStartDate={start}T00:00:00.000\
             &pubEndDate={end}T23:59:59.999",
            start = since.format("%Y-%m-%d"),
            end = now.format("%Y-%m-%d")
        );

        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                warn!(error = %error, "cve: network error");
                return AcquisitionOutcome::fetch_failed(
                    format!("cve: network error: {error}"),
                    None,
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after = retry_after_secs(response.headers());
            warn!(status = status.as_u16(), "cve: acquisition failed");
            return http_failure(status.as_u16(), retry_after, "cve");
        }

        let body: serde_json::Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                warn!(error = %error, "cve: failed to parse JSON");
                return AcquisitionOutcome::parse_failed(
                    format!("cve: failed to parse NVD JSON: {error}"),
                    "",
                );
            }
        };

        let vulnerabilities = body
            .get("vulnerabilities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let cves: Vec<CveVulnerability> = vulnerabilities
            .iter()
            .filter_map(|entry| entry.get("cve").map(Self::parse_cve))
            .collect();

        AcquisitionOutcome::success_now(cves)
    }

    /// Project a single raw NVD `cve` object onto [`CveVulnerability`].
    fn parse_cve(cve: &serde_json::Value) -> CveVulnerability {
        let cve_id = cve
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // First English description.
        let description = cve
            .get("descriptions")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|d| {
                    let is_en = d.get("lang").and_then(|v| v.as_str()) == Some("en");
                    if is_en {
                        d.get("value")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        // CVSS v3.1, falling back to v3.0, then v2.
        let (cvss_score, cvss_severity) = Self::extract_cvss(cve.get("metrics"));

        let published = cve
            .get("published")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let last_modified = cve
            .get("lastModified")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let affected_products = Self::extract_affected_products(cve.get("configurations"));
        let reference_urls = cve
            .get("references")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| {
                        r.get("url")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        CveVulnerability {
            cve_id,
            description,
            cvss_score,
            cvss_severity,
            published,
            last_modified,
            affected_products,
            reference_urls,
        }
    }

    /// Extract `(baseScore, baseSeverity)` from the `metrics` block,
    /// preferring CVSS v3.1, then v3.0, then v2.
    fn extract_cvss(metrics: Option<&serde_json::Value>) -> (Option<f64>, Option<String>) {
        let Some(metrics) = metrics else {
            return (None, None);
        };

        for key in ["cvssMetricV31", "cvssMetricV30", "cvssMetricV2"] {
            if let Some(arr) = metrics.get(key).and_then(|v| v.as_array()) {
                if let Some(first) = arr.first() {
                    let data = first.get("cvssData").or(Some(first));
                    let score = data
                        .and_then(|d| d.get("baseScore"))
                        .and_then(|v| v.as_f64());
                    // V2 stores severity on the metric, V3 on cvssData.
                    let severity = first
                        .get("baseSeverity")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .or_else(|| {
                            data.and_then(|d| d.get("baseSeverity"))
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        });
                    if score.is_some() {
                        return (score, severity);
                    }
                }
            }
        }
        (None, None)
    }

    /// Walk `configurations[].nodes[].cpeMatch[].criteria` for affected
    /// products.  Deduplicates while preserving first-seen order.
    fn extract_affected_products(configurations: Option<&serde_json::Value>) -> Vec<String> {
        let Some(configs) = configurations.and_then(|v| v.as_array()) else {
            return Vec::new();
        };

        let mut products = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for config in configs {
            let Some(nodes) = config.get("nodes").and_then(|v| v.as_array()) else {
                continue;
            };
            for node in nodes {
                let Some(matches) = node.get("cpeMatch").and_then(|v| v.as_array()) else {
                    continue;
                };
                for m in matches {
                    if let Some(criteria) = m.get("criteria").and_then(|v| v.as_str()) {
                        if seen.insert(criteria.to_string()) {
                            products.push(criteria.to_string());
                        }
                    }
                }
            }
        }
        products
    }
}

impl Default for CveClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Request for one NVD acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CveRequest {
    pub since_days: u32,
}

#[async_trait]
impl SourceAdapter for CveClient {
    type Item = CveVulnerability;
    type Request = CveRequest;

    fn adapter_id(&self) -> &'static str {
        "cve"
    }

    fn prerequisite(&self) -> AdapterPrerequisite {
        AdapterPrerequisite::NONE
    }

    async fn acquire(&self, request: CveRequest) -> AcquisitionOutcome<CveVulnerability> {
        self.fetch_recent(request.since_days).await
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal but structurally faithful NVD 2.0 response containing one
    /// CVE with an English description, CVSS v3.1 metrics, one affected
    /// product, and a reference URL.
    const SAMPLE_NVD: &str = r#"{
      "resultsPerPage": 1,
      "startIndex": 0,
      "totalResults": 1,
      "vulnerabilities": [
        {
          "cve": {
            "id": "CVE-2024-99999",
            "published": "2024-06-01T12:34:56.000",
            "lastModified": "2024-06-02T09:00:00.000",
            "descriptions": [
              {"lang": "en", "value": "A critical buffer overflow in the demo component."},
              {"lang": "es", "value": "Una descripción en español."}
            ],
            "metrics": {
              "cvssMetricV31": [
                {
                  "source": "nvd@nist.gov",
                  "type": "Primary",
                  "cvssData": {
                    "version": "3.1",
                    "baseScore": 9.8,
                    "baseSeverity": "CRITICAL"
                  }
                }
              ]
            },
            "configurations": [
              {
                "nodes": [
                  {
                    "operator": "OR",
                    "cpeMatch": [
                      {"criteria": "cpe:2.3:a:example:demo:1.0:*:*:*:*:*:*:*"},
                      {"criteria": "cpe:2.3:a:example:demo:1.0:*:*:*:*:*:*:*"}
                    ]
                  }
                ]
              }
            ],
            "references": [
              {"url": "https://example.org/advisory"},
              {"url": "https://example.org/patch"}
            ]
          }
        }
      ]
    }"#;

    #[test]
    fn cve_adapter_declares_its_prerequisite() {
        let client = CveClient::new();
        assert_eq!(client.adapter_id(), "cve");
        assert!(!client.prerequisite().requires_credentials);
        assert!(
            crate::acquisition::adapter_descriptor(client.adapter_id()).is_some(),
            "every SourceAdapter implementation must publish its prerequisite"
        );
    }

    #[test]
    fn parses_sample_nvd_cve() {
        let value: serde_json::Value = serde_json::from_str(SAMPLE_NVD).unwrap();
        let vulns = value
            .get("vulnerabilities")
            .and_then(|v| v.as_array())
            .unwrap();
        let cve = CveClient::parse_cve(&vulns[0]["cve"]);

        assert_eq!(cve.cve_id, "CVE-2024-99999");
        assert_eq!(
            cve.description,
            "A critical buffer overflow in the demo component."
        );
        assert_eq!(cve.cvss_score, Some(9.8));
        assert_eq!(cve.cvss_severity.as_deref(), Some("CRITICAL"));
        assert_eq!(cve.published.as_deref(), Some("2024-06-01T12:34:56.000"));
        assert_eq!(
            cve.last_modified.as_deref(),
            Some("2024-06-02T09:00:00.000")
        );
        // Duplicate CPE must be deduplicated.
        assert_eq!(cve.affected_products.len(), 1);
        assert_eq!(
            cve.affected_products[0],
            "cpe:2.3:a:example:demo:1.0:*:*:*:*:*:*:*"
        );
        assert_eq!(cve.reference_urls.len(), 2);
    }

    #[test]
    fn parse_cve_is_defensive_about_missing_fields() {
        let raw: serde_json::Value = serde_json::json!({"id": "CVE-2024-0001"});
        let cve = CveClient::parse_cve(&raw);
        assert_eq!(cve.cve_id, "CVE-2024-0001");
        assert!(cve.description.is_empty());
        assert!(cve.cvss_score.is_none());
        assert!(cve.cvss_severity.is_none());
        assert!(cve.affected_products.is_empty());
        assert!(cve.reference_urls.is_empty());
    }

    #[test]
    fn extract_cvss_prefers_v31_then_v30_then_v2() {
        // v31 present.
        let metrics = serde_json::json!({
            "cvssMetricV31": [{"cvssData": {"baseScore": 9.8, "baseSeverity": "CRITICAL"}}]
        });
        let (score, sev) = CveClient::extract_cvss(Some(&metrics));
        assert_eq!(score, Some(9.8));
        assert_eq!(sev.as_deref(), Some("CRITICAL"));

        // Only v2 — severity lives on the metric, not cvssData, for v2.
        let metrics = serde_json::json!({
            "cvssMetricV2": [{"baseSeverity": "HIGH", "cvssData": {"baseScore": 7.5}}]
        });
        let (score, sev) = CveClient::extract_cvss(Some(&metrics));
        assert_eq!(score, Some(7.5));
        assert_eq!(sev.as_deref(), Some("HIGH"));
    }

    #[test]
    fn to_observation_is_vuln_notice() {
        let cve = CveVulnerability {
            cve_id: "CVE-2024-99999".to_string(),
            description: "demo".to_string(),
            cvss_score: Some(9.8),
            cvss_severity: Some("CRITICAL".to_string()),
            published: None,
            last_modified: None,
            affected_products: vec![],
            reference_urls: vec![],
        };
        let entity_id = Uuid::new_v4();
        let obs = cve.to_observation(Some(entity_id));
        assert_eq!(obs.observation_type, ObservationType::VulnNotice);
        assert_eq!(obs.entity_id, Some(entity_id));
        assert_eq!(
            obs.provenance.get("source").and_then(|v| v.as_str()),
            Some("nvd")
        );
        assert!(obs
            .provenance
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap()
            .ends_with("CVE-2024-99999"));
    }
}
