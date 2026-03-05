//! Security route — request/response types and logic for security endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Internal record used by the legacy list_security aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureRecord {
    pub domain: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub score: f64,
    pub checked_at: DateTime<Utc>,
}

/// DNS posture item returned by GET /api/security/dns-posture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureItem {
    pub domain: String,
    pub company_id: String,
    pub company_name: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub dmarc_policy: Option<String>,
    pub posture_score: f64,
    pub last_checked: String,
}

/// Response body for GET /api/security/dns-posture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureOverview {
    pub items: Vec<DnsPostureItem>,
    pub overall_score: f64,
    pub domains_checked: usize,
}

/// Lookalike domain entry returned by GET /api/security/lookalike-domains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeDomainItem {
    pub id: String,
    pub original_domain: String,
    pub lookalike_domain: String,
    pub distance: i64,
    pub threat_type: String,
    pub detected_at: String,
    pub active: bool,
    pub registrar: Option<String>,
    pub registration_date: Option<String>,
}

/// KEV entry returned by GET /api/security/kev-relevance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KevItem {
    pub cve_id: String,
    pub vendor: String,
    pub product: String,
    pub vulnerability_name: String,
    pub date_added: String,
    pub due_date: String,
    pub relevance_score: f64,
    pub affected_companies: Vec<String>,
    pub notes: Option<String>,
}

/// Internal types kept for potential future use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeDomain {
    pub domain: String,
    pub similarity: f64,
    pub risk: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KevRelevance {
    pub cve_id: String,
    pub vendor: String,
    pub product: String,
    pub relevance_score: f64,
    pub rationale: String,
}

/// Scalar summary returned by GET /api/security.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecuritySummary {
    pub dns_posture_score: f64,
    pub lookalike_domains_detected: u64,
    pub kev_matches: u64,
    pub last_scan_at: Option<String>,
    pub domains_monitored: u64,
}

pub fn dns_score(has_spf: bool, has_dkim: bool, has_dmarc: bool) -> f64 {
    let mut score: f64 = 0.0;
    if has_spf {
        score += 0.3;
    }
    if has_dkim {
        score += 0.3;
    }
    if has_dmarc {
        score += 0.4;
    }
    score.min(1.0)
}

pub fn classify_lookalike_risk(similarity: f64) -> &'static str {
    if similarity >= 0.9 {
        "critical"
    } else if similarity >= 0.8 {
        "high"
    } else if similarity >= 0.7 {
        "medium"
    } else {
        "low"
    }
}

pub fn relevance_tier(score: f64) -> &'static str {
    if score >= 0.8 {
        "high"
    } else if score >= 0.5 {
        "medium"
    } else {
        "low"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_score() {
        assert!((dns_score(true, true, true) - 1.0).abs() < 0.001);
        assert!((dns_score(true, false, false) - 0.3).abs() < 0.001);
        assert!((dns_score(false, false, false) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_classify_lookalike_risk() {
        assert_eq!(classify_lookalike_risk(0.92), "critical");
        assert_eq!(classify_lookalike_risk(0.82), "high");
        assert_eq!(classify_lookalike_risk(0.71), "medium");
        assert_eq!(classify_lookalike_risk(0.5), "low");
    }

    #[test]
    fn test_relevance_tier() {
        assert_eq!(relevance_tier(0.9), "high");
        assert_eq!(relevance_tier(0.6), "medium");
        assert_eq!(relevance_tier(0.2), "low");
    }

    #[test]
    fn test_dns_posture_serialization() {
        let v = DnsPostureRecord {
            domain: "example.com".to_string(),
            has_spf: true,
            has_dkim: true,
            has_dmarc: false,
            score: 0.6,
            checked_at: Utc::now(),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("example.com"));
    }
}
