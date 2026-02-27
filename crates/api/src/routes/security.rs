//! Security route — request/response types and logic for security endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureResponse {
    pub domain: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub score: f64,
    pub checked_at: DateTime<Utc>,
}

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

/// Summary payload for security overview endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySummary {
    pub dns_posture: Vec<DnsPostureResponse>,
    pub lookalikes: Vec<LookalikeDomain>,
    pub kev: Vec<KevRelevance>,
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
        let v = DnsPostureResponse {
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
