//! Companies route — request/response types and logic for company endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use apex_core::validation::{clamp_ratio, validate_uuid};

use super::warnings::SortDirection;

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing companies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListCompaniesQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub regions: Option<String>,
    pub search: Option<String>,
    pub is_competitor: Option<bool>,
    pub sort_by: Option<CompanySortField>,
    pub sort_dir: Option<SortDirection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompanySortField {
    Name,
    Region,
    ThreatScore,
    UpdatedAt,
}

impl CompanySortField {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "name" => Some(Self::Name),
            "region" => Some(Self::Region),
            "threat" | "threat_score" => Some(Self::ThreatScore),
            "updated_at" | "updated" => Some(Self::UpdatedAt),
            _ => None,
        }
    }
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Company list item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyListItem {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub region: String,
    pub country: String,
    pub entity_type: String,
    pub is_competitor: bool,
    pub threat_score: Option<f64>,
    pub capabilities: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Detailed company profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyDetail {
    pub id: String,
    pub name: String,
    pub legal_name: Option<String>,
    pub region: String,
    pub country: String,
    pub city: Option<String>,
    pub website: Option<String>,
    pub entity_type: String,
    pub is_competitor: bool,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub capabilities: Vec<String>,
    pub certifications: Vec<String>,
    pub sites: Vec<CompanySite>,
    pub key_persons: Vec<CompanyKeyPerson>,
    pub recent_events: Vec<CompanyEvent>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanySite {
    pub name: String,
    pub location: String,
    pub site_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyKeyPerson {
    pub person_id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyEvent {
    pub event_type: String,
    pub description: String,
    pub date: DateTime<Utc>,
    pub source_url: Option<String>,
}

/// Competitor change event for the competitor changes endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorChange {
    pub competitor_id: String,
    pub competitor_name: String,
    pub change_type: String,
    pub description: String,
    pub significance: f64,
    pub detected_at: DateTime<Utc>,
    pub source_url: Option<String>,
}

/// Validate a company ID.
pub fn validate_company_id(id: &str) -> Result<Uuid, String> {
    validate_uuid(id, "company_id").map_err(|e| e.to_string())?;
    Uuid::parse_str(id.trim()).map_err(|_| format!("Invalid company ID: '{}'", id))
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Sort companies in-place.
pub fn sort_companies(items: &mut [CompanyListItem], field: &CompanySortField, desc: bool) {
    items.sort_by(|a, b| {
        let cmp = match field {
            CompanySortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            CompanySortField::Region => a.region.cmp(&b.region),
            CompanySortField::ThreatScore => {
                let sa = a.threat_score.unwrap_or(0.0);
                let sb = b.threat_score.unwrap_or(0.0);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            }
            CompanySortField::UpdatedAt => a.updated_at.cmp(&b.updated_at),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Filter companies by search text (matches name or capabilities).
pub fn search_companies<'a>(items: &'a [CompanyListItem], query: &str) -> Vec<&'a CompanyListItem> {
    let q = query.to_lowercase();
    items
        .iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&q)
                || c.capabilities.iter().any(|cap| cap.to_lowercase().contains(&q))
        })
        .collect()
}

/// Filter to competitors only.
pub fn filter_competitors(items: &[CompanyListItem]) -> Vec<&CompanyListItem> {
    items.iter().filter(|c| c.is_competitor).collect()
}

/// Rank competitor changes by significance.
pub fn rank_changes(changes: &mut [CompetitorChange]) {
    changes.sort_by(|a, b| {
        b.significance
            .partial_cmp(&a.significance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Compute a simple competitive threat summary.
pub fn threat_summary(companies: &[CompanyListItem]) -> CompetitorSummary {
    let competitors: Vec<_> = companies.iter().filter(|c| c.is_competitor).collect();
    let total = competitors.len();
    let scored: Vec<f64> = competitors.iter().filter_map(|c| c.threat_score).collect();
    let avg_threat = if !scored.is_empty() {
        clamp_ratio(scored.iter().sum::<f64>() / scored.len() as f64)
    } else {
        0.0
    };
    let high_threat = competitors
        .iter()
        .filter(|c| c.threat_score.unwrap_or(0.0) >= 0.7)
        .count();

    CompetitorSummary {
        total_competitors: total,
        avg_threat_score: avg_threat,
        high_threat_count: high_threat,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorSummary {
    pub total_competitors: usize,
    pub avg_threat_score: f64,
    pub high_threat_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_company(name: &str, region: &str, competitor: bool, threat: Option<f64>) -> CompanyListItem {
        CompanyListItem {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            domain: None,
            region: region.to_string(),
            country: region.to_string(),
            entity_type: "manufacturer".to_string(),
            is_competitor: competitor,
            threat_score: threat,
            capabilities: vec!["pcb_assembly".to_string(), "smt".to_string()],
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_sort_companies_by_name() {
        let mut items = vec![
            make_company("Zeta Corp", "TN", false, None),
            make_company("Alpha Inc", "MA", false, None),
            make_company("Mid Tech", "EU", false, None),
        ];
        sort_companies(&mut items, &CompanySortField::Name, false);
        assert_eq!(items[0].name, "Alpha Inc");
        assert_eq!(items[2].name, "Zeta Corp");
    }

    #[test]
    fn test_sort_companies_by_threat_desc() {
        let mut items = vec![
            make_company("A", "TN", true, Some(0.3)),
            make_company("B", "MA", true, Some(0.9)),
            make_company("C", "EU", true, Some(0.5)),
        ];
        sort_companies(&mut items, &CompanySortField::ThreatScore, true);
        assert_eq!(items[0].name, "B"); // 0.9 first
        assert_eq!(items[2].name, "A"); // 0.3 last
    }

    #[test]
    fn test_search_companies_by_name() {
        let items = vec![
            make_company("Foxconn", "CN", true, Some(0.8)),
            make_company("Celestica", "US", true, Some(0.7)),
            make_company("Starz Electronics", "TN", false, None),
        ];
        let results = search_companies(&items, "fox");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Foxconn");
    }

    #[test]
    fn test_search_companies_by_capability() {
        let items = vec![make_company("Test Corp", "TN", false, None)];
        let results = search_companies(&items, "smt");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_filter_competitors() {
        let items = vec![
            make_company("A", "TN", true, Some(0.5)),
            make_company("B", "MA", false, None),
            make_company("C", "EU", true, Some(0.8)),
        ];
        let comps = filter_competitors(&items);
        assert_eq!(comps.len(), 2);
    }

    #[test]
    fn test_rank_changes() {
        let mut changes = vec![
            CompetitorChange {
                competitor_id: "1".to_string(),
                competitor_name: "A".to_string(),
                change_type: "expansion".to_string(),
                description: "New facility".to_string(),
                significance: 0.3,
                detected_at: Utc::now(),
                source_url: None,
            },
            CompetitorChange {
                competitor_id: "2".to_string(),
                competitor_name: "B".to_string(),
                change_type: "acquisition".to_string(),
                description: "Major acquisition".to_string(),
                significance: 0.95,
                detected_at: Utc::now(),
                source_url: None,
            },
        ];
        rank_changes(&mut changes);
        assert_eq!(changes[0].competitor_name, "B");
    }

    #[test]
    fn test_threat_summary() {
        let companies = vec![
            make_company("A", "TN", true, Some(0.8)),
            make_company("B", "MA", true, Some(0.4)),
            make_company("C", "EU", false, None),
            make_company("D", "CN", true, Some(0.9)),
        ];
        let summary = threat_summary(&companies);
        assert_eq!(summary.total_competitors, 3);
        assert_eq!(summary.high_threat_count, 2); // 0.8 and 0.9
        assert!((summary.avg_threat_score - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_threat_summary_no_competitors() {
        let companies = vec![make_company("A", "TN", false, None)];
        let summary = threat_summary(&companies);
        assert_eq!(summary.total_competitors, 0);
        assert_eq!(summary.avg_threat_score, 0.0);
    }

    #[test]
    fn test_company_detail_serialization() {
        let detail = CompanyDetail {
            id: "c-1".to_string(),
            name: "Starz Electronics".to_string(),
            legal_name: Some("Starz Electronics SARL".to_string()),
            region: "TN".to_string(),
            country: "Tunisia".to_string(),
            city: Some("Tunis".to_string()),
            website: Some("https://starzelectronics.site".to_string()),
            entity_type: "ems".to_string(),
            is_competitor: false,
            threat_score: None,
            overlap_score: None,
            capabilities: vec!["pcb_assembly".to_string()],
            certifications: vec!["ISO9001".to_string()],
            sites: vec![CompanySite {
                name: "HQ".to_string(),
                location: "Tunis".to_string(),
                site_type: "factory".to_string(),
            }],
            key_persons: vec![],
            recent_events: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("Starz Electronics"));
        assert!(json.contains("ISO9001"));
    }

    #[test]
    fn test_company_sort_field_from_str() {
        assert_eq!(
            CompanySortField::from_str_loose("threat"),
            Some(CompanySortField::ThreatScore)
        );
        assert_eq!(
            CompanySortField::from_str_loose("name"),
            Some(CompanySortField::Name)
        );
        assert_eq!(CompanySortField::from_str_loose("xyz"), None);
    }
}
