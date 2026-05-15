//! Persons route — request/response types and logic for POI endpoints.

use crate::config::PriorityWeights;
use apex_core::validation::{clamp_ratio, validate_uuid};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing persons/POIs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListPersonsQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub regions: Option<String>,
    pub search: Option<String>,
    pub min_priority: Option<f64>,
    pub roles: Option<String>,
    pub sort_by: Option<PersonSortField>,
    /// Named tier filter: "critical" | "high" | "medium" | "low"
    /// Converts to min/max priority bounds server-side.
    pub tier: Option<String>,
    /// Legacy list filter used by pre-migration UI: "A" | "B" | "C".
    pub priority: Option<String>,
    /// Legacy single region filter alias used by pre-migration list page.
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PersonSortField {
    Name,
    Priority,
    Region,
    UpdatedAt,
}

impl PersonSortField {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "name" => Some(Self::Name),
            "priority" | "score" => Some(Self::Priority),
            "region" => Some(Self::Region),
            "updated_at" | "updated" => Some(Self::UpdatedAt),
            _ => None,
        }
    }
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Person list item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonListItem {
    pub id: String,
    pub name: String,
    pub role: String,
    pub role_family: String,
    pub organization: String,
    pub region: String,
    pub country: String,
    pub priority_score: f64,
    pub pain_index: f64,
    pub change_risk: f64,
    pub role_drift_score: f64,
    /// Legacy 0-100 influence score expected by pre-migration cards/charts.
    pub influence_score: i64,
    /// Legacy priority band used by pre-migration filters.
    pub priority: String,
    pub influence_tier: String,
    pub engagement_status: String,
    pub tags: Vec<String>,
    pub last_signal: String,
    pub updated_at: DateTime<Utc>,
}

/// Detailed person/POI profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonDetail {
    pub id: String,
    pub name: String,
    pub name_alt: Vec<String>,
    pub role: String,
    pub role_family: String,
    pub organization: String,
    pub org_id: Option<String>,
    pub region: String,
    pub country: String,
    pub bio: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin: Option<String>,
    pub priority_score: f64,
    pub influence_score: i64,
    pub priority: String,
    pub priority_vector: PriorityVector,
    pub influence_tier: String,
    pub engagement_status: String,
    pub engagement_readiness: f64,
    pub data_completeness: f64,
    pub tags: Vec<String>,
    pub trigger_topics: Vec<String>,
    pub decision_style: Option<String>,
    pub risk_tolerance: Option<String>,
    pub change_appetite: Option<String>,
    pub communication_style: Option<String>,
    pub decision_mode: Option<String>,
    pub preferred_proof_type: Option<String>,
    pub pain_index: Option<f64>,
    pub change_risk: Option<f64>,
    pub role_drift_score: Option<f64>,
    pub buying_center_role: String,
    pub affiliations: Vec<Affiliation>,
    pub timeline: Vec<PersonEvent>,
    pub role_history: Vec<RoleHistoryEntry>,
    pub peers: Vec<PeerSummary>,
    pub warning_count: i64,
    pub insight_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityVector {
    pub decision_power: f64,
    pub domain_relevance: f64,
    pub network_centrality: f64,
    pub engagement_potential: f64,
    pub intelligence_value: f64,
}

impl PriorityVector {
    /// Compute a weighted composite score.
    pub fn composite(&self) -> f64 {
        self.composite_with_weights(&PriorityWeights::default())
    }

    pub fn composite_with_weights(&self, weights: &PriorityWeights) -> f64 {
        let values = [
            self.decision_power,
            self.domain_relevance,
            self.network_centrality,
            self.engagement_potential,
            self.intelligence_value,
        ];
        let weight_values = [
            weights.decision_power,
            weights.domain_relevance,
            weights.network_centrality,
            weights.engagement_potential,
            weights.intelligence_value,
        ];
        let total_weight: f64 = weight_values.iter().sum();
        let normalized = if total_weight <= f64::EPSILON {
            [0.25, 0.20, 0.20, 0.15, 0.20]
        } else {
            [
                weight_values[0] / total_weight,
                weight_values[1] / total_weight,
                weight_values[2] / total_weight,
                weight_values[3] / total_weight,
                weight_values[4] / total_weight,
            ]
        };
        let score: f64 = normalized
            .iter()
            .zip(values.iter())
            .map(|(w, v)| w * v)
            .sum();
        clamp_ratio(score)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Affiliation {
    pub organization: String,
    pub role: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonEvent {
    pub event_type: String,
    pub description: String,
    pub date: DateTime<Utc>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleHistoryEntry {
    pub organization: String,
    pub role: String,
    pub role_family: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub is_current: bool,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSummary {
    pub id: String,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub priority_score: f64,
    pub influence_tier: String,
}

/// Engagement guide response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementGuide {
    pub person_id: String,
    pub person_name: String,
    pub recommended_approach: String,
    pub talking_points: Vec<String>,
    pub common_interests: Vec<String>,
    pub risk_factors: Vec<String>,
    pub optimal_timing: Option<String>,
    pub communication_preference: Option<String>,
}

/// Validate a person ID.
pub fn validate_person_id(id: &str) -> Result<Uuid, String> {
    validate_uuid(id, "person_id").map_err(|e| e.to_string())?;
    Uuid::parse_str(id.trim()).map_err(|_| format!("Invalid person ID: '{}'", id))
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Sort persons in-place.
pub fn sort_persons(items: &mut [PersonListItem], field: &PersonSortField, desc: bool) {
    items.sort_by(|a, b| {
        let cmp = match field {
            PersonSortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            PersonSortField::Priority => a
                .priority_score
                .partial_cmp(&b.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal),
            PersonSortField::Region => a.region.cmp(&b.region),
            PersonSortField::UpdatedAt => a.updated_at.cmp(&b.updated_at),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Filter persons by minimum priority.
pub fn filter_by_priority(items: &[PersonListItem], min: f64) -> Vec<&PersonListItem> {
    items.iter().filter(|p| p.priority_score >= min).collect()
}

/// Filter persons by role (comma-separated roles string).
pub fn filter_by_roles<'a>(items: &'a [PersonListItem], roles: &str) -> Vec<&'a PersonListItem> {
    let role_set: Vec<String> = roles
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if role_set.is_empty() {
        return items.iter().collect();
    }
    items
        .iter()
        .filter(|p| role_set.contains(&p.role.to_lowercase()))
        .collect()
}

/// Search persons by name or organization.
pub fn search_persons<'a>(items: &'a [PersonListItem], query: &str) -> Vec<&'a PersonListItem> {
    let q = query.to_lowercase();
    items
        .iter()
        .filter(|p| {
            p.name.to_lowercase().contains(&q) || p.organization.to_lowercase().contains(&q)
        })
        .collect()
}

/// Priority tier classification.
pub fn priority_tier(score: f64) -> &'static str {
    if score >= 0.8 {
        "critical"
    } else if score >= 0.6 {
        "high"
    } else if score >= 0.4 {
        "medium"
    } else {
        "low"
    }
}

/// Count persons by region.
pub fn count_by_region(items: &[PersonListItem]) -> Vec<(String, usize)> {
    let mut map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in items {
        *map.entry(p.region.clone()).or_insert(0) += 1;
    }
    let mut result: Vec<_> = map.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_person(name: &str, role: &str, region: &str, priority: f64) -> PersonListItem {
        PersonListItem {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            role: role.to_string(),
            role_family: "C-Suite".to_string(),
            organization: "Test Org".to_string(),
            region: region.to_string(),
            country: "US".to_string(),
            priority_score: priority,
            pain_index: 0.0,
            change_risk: 0.0,
            role_drift_score: 0.0,
            influence_score: (priority.clamp(0.0, 1.0) * 100.0).round() as i64,
            priority: if priority >= 0.8 {
                "A".to_string()
            } else if priority >= 0.6 {
                "B".to_string()
            } else {
                "C".to_string()
            },
            influence_tier: super::priority_tier(priority).to_string(),
            engagement_status: "new".to_string(),
            tags: vec![],
            last_signal: "".to_string(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_sort_persons_by_priority_desc() {
        let mut items = vec![
            make_person("Alice", "CEO", "TN", 0.6),
            make_person("Bob", "CTO", "MA", 0.9),
            make_person("Carol", "VP", "EU", 0.3),
        ];
        sort_persons(&mut items, &PersonSortField::Priority, true);
        assert_eq!(items[0].name, "Bob");
        assert_eq!(items[2].name, "Carol");
    }

    #[test]
    fn test_sort_persons_by_name_asc() {
        let mut items = vec![
            make_person("Zara", "CEO", "TN", 0.5),
            make_person("Alice", "CTO", "MA", 0.7),
        ];
        sort_persons(&mut items, &PersonSortField::Name, false);
        assert_eq!(items[0].name, "Alice");
    }

    #[test]
    fn test_filter_by_priority() {
        let items = vec![
            make_person("A", "CEO", "TN", 0.3),
            make_person("B", "CTO", "MA", 0.7),
            make_person("C", "VP", "EU", 0.9),
        ];
        let filtered = filter_by_priority(&items, 0.5);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_roles() {
        let items = vec![
            make_person("A", "CEO", "TN", 0.5),
            make_person("B", "CTO", "MA", 0.7),
            make_person("C", "CEO", "EU", 0.6),
        ];
        let filtered = filter_by_roles(&items, "ceo");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_roles_multiple() {
        let items = vec![
            make_person("A", "CEO", "TN", 0.5),
            make_person("B", "CTO", "MA", 0.7),
            make_person("C", "VP", "EU", 0.6),
        ];
        let filtered = filter_by_roles(&items, "ceo,cto");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_roles_empty() {
        let items = vec![make_person("A", "CEO", "TN", 0.5)];
        let filtered = filter_by_roles(&items, "");
        assert_eq!(filtered.len(), 1); // empty = no filter
    }

    #[test]
    fn test_search_persons_by_name() {
        let items = vec![
            make_person("Ahmed Ben Ali", "CEO", "TN", 0.8),
            make_person("Sarah Johnson", "CTO", "US", 0.6),
        ];
        let results = search_persons(&items, "ahmed");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_priority_tier() {
        assert_eq!(priority_tier(0.9), "critical");
        assert_eq!(priority_tier(0.8), "critical");
        assert_eq!(priority_tier(0.7), "high");
        assert_eq!(priority_tier(0.5), "medium");
        assert_eq!(priority_tier(0.2), "low");
    }

    #[test]
    fn test_count_by_region() {
        let items = vec![
            make_person("A", "CEO", "TN", 0.5),
            make_person("B", "CTO", "TN", 0.7),
            make_person("C", "VP", "MA", 0.6),
        ];
        let counts = count_by_region(&items);
        assert_eq!(counts[0].0, "TN");
        assert_eq!(counts[0].1, 2);
    }

    #[test]
    fn test_priority_vector_composite() {
        let pv = PriorityVector {
            decision_power: 0.8,
            domain_relevance: 0.9,
            network_centrality: 0.7,
            engagement_potential: 0.6,
            intelligence_value: 0.5,
        };
        let score = pv.composite();
        // 0.25*0.8 + 0.20*0.9 + 0.20*0.7 + 0.15*0.6 + 0.20*0.5
        // = 0.20 + 0.18 + 0.14 + 0.09 + 0.10 = 0.71
        assert!((score - 0.71).abs() < 0.01);
    }

    #[test]
    fn test_priority_vector_clamped() {
        let pv = PriorityVector {
            decision_power: 1.0,
            domain_relevance: 1.0,
            network_centrality: 1.0,
            engagement_potential: 1.0,
            intelligence_value: 1.0,
        };
        assert!((pv.composite() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_priority_vector_composite_with_custom_weights() {
        let pv = PriorityVector {
            decision_power: 0.9,
            domain_relevance: 0.2,
            network_centrality: 0.2,
            engagement_potential: 0.2,
            intelligence_value: 0.2,
        };
        let weights = PriorityWeights {
            decision_power: 1.0,
            domain_relevance: 0.0,
            network_centrality: 0.0,
            engagement_potential: 0.0,
            intelligence_value: 0.0,
        };

        assert!((pv.composite_with_weights(&weights) - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_engagement_guide_serialization() {
        let guide = EngagementGuide {
            person_id: "p-1".to_string(),
            person_name: "Ahmed Ben Ali".to_string(),
            recommended_approach: "Professional conference introduction".to_string(),
            talking_points: vec!["EMS market trends".to_string()],
            common_interests: vec!["PCB manufacturing".to_string()],
            risk_factors: vec![],
            optimal_timing: Some("Q3 trade shows".to_string()),
            communication_preference: Some("email".to_string()),
        };
        let json = serde_json::to_string(&guide).unwrap();
        assert!(json.contains("Ahmed Ben Ali"));
        assert!(json.contains("trade shows"));
    }

    #[test]
    fn test_person_sort_field_from_str() {
        assert_eq!(
            PersonSortField::from_str_loose("priority"),
            Some(PersonSortField::Priority)
        );
        assert_eq!(
            PersonSortField::from_str_loose("score"),
            Some(PersonSortField::Priority)
        );
        assert_eq!(PersonSortField::from_str_loose("xyz"), None);
    }
}
