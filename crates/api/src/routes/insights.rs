//! Insights route — request/response types and logic for insight endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing insights.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListInsightsQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub regions: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub search: Option<String>,
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Single insight in list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightResponse {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub insight_type: String,
    pub region: String,
    pub confidence: f64,
    pub evidence_urls: Vec<String>,
    pub entity_ids: Vec<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Weekly strategy memo response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyMemoResponse {
    pub id: String,
    pub week_label: String,
    pub executive_summary: String,
    pub sections: Vec<MemoSection>,
    pub key_metrics: MemoMetrics,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoSection {
    pub title: String,
    pub content: String,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoMetrics {
    pub total_warnings: u32,
    pub critical_warnings: u32,
    pub new_insights: u32,
    pub recipe_promotions: u32,
    pub recipe_deprecations: u32,
    pub poi_updates: u32,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Rank insights by confidence × recency.
pub fn rank_insights(insights: &mut [InsightResponse]) {
    let now = Utc::now();
    insights.sort_by(|a, b| {
        let score_a = insight_score(a, &now);
        let score_b = insight_score(b, &now);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn insight_score(insight: &InsightResponse, now: &DateTime<Utc>) -> f64 {
    let age_hours = (*now - insight.created_at).num_hours().max(1) as f64;
    let recency = 1.0 / (1.0 + (age_hours / 24.0).ln().max(0.0));
    insight.confidence * recency
}

/// Group insights by region.
pub fn group_by_region(insights: &[InsightResponse]) -> Vec<(String, Vec<&InsightResponse>)> {
    let mut map: std::collections::HashMap<String, Vec<&InsightResponse>> =
        std::collections::HashMap::new();
    for i in insights {
        map.entry(i.region.clone()).or_default().push(i);
    }
    let mut result: Vec<_> = map.into_iter().collect();
    result.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    result
}

/// Filter insights by tag.
pub fn filter_by_tag<'a>(insights: &'a [InsightResponse], tag: &str) -> Vec<&'a InsightResponse> {
    let tag_lower = tag.to_lowercase();
    insights
        .iter()
        .filter(|i| i.tags.iter().any(|t| t.to_lowercase() == tag_lower))
        .collect()
}

/// Build a memo metrics summary from raw counts.
pub fn build_memo_metrics(
    total_warnings: u32,
    critical_warnings: u32,
    new_insights: u32,
    recipe_promotions: u32,
    recipe_deprecations: u32,
    poi_updates: u32,
) -> MemoMetrics {
    MemoMetrics {
        total_warnings,
        critical_warnings,
        new_insights,
        recipe_promotions,
        recipe_deprecations,
        poi_updates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_insight(region: &str, confidence: f64, hours_ago: i64, tags: Vec<&str>) -> InsightResponse {
        InsightResponse {
            id: uuid::Uuid::new_v4().to_string(),
            title: format!("Insight in {}", region),
            summary: "Test insight".to_string(),
            insight_type: "supply_chain".to_string(),
            region: region.to_string(),
            confidence,
            evidence_urls: vec![],
            entity_ids: vec![],
            tags: tags.into_iter().map(|t| t.to_string()).collect(),
            created_at: Utc::now() - chrono::Duration::hours(hours_ago),
        }
    }

    #[test]
    fn test_rank_insights_by_score() {
        let mut insights = vec![
            make_insight("TN", 0.5, 48, vec![]),  // old, low conf
            make_insight("MA", 0.95, 1, vec![]),   // recent, high conf
            make_insight("EU", 0.8, 12, vec![]),
        ];
        rank_insights(&mut insights);
        // best score first (high confidence + recent)
        assert_eq!(insights[0].region, "MA");
    }

    #[test]
    fn test_group_by_region() {
        let insights = vec![
            make_insight("TN", 0.9, 1, vec![]),
            make_insight("TN", 0.8, 2, vec![]),
            make_insight("MA", 0.7, 3, vec![]),
        ];
        let groups = group_by_region(&insights);
        // TN has 2, MA has 1 → TN first
        assert_eq!(groups[0].0, "TN");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "MA");
    }

    #[test]
    fn test_filter_by_tag() {
        let insights = vec![
            make_insight("TN", 0.9, 1, vec!["supply_chain", "critical"]),
            make_insight("MA", 0.8, 2, vec!["competitive"]),
            make_insight("EU", 0.7, 3, vec!["supply_chain"]),
        ];
        let filtered = filter_by_tag(&insights, "supply_chain");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_tag_case_insensitive() {
        let insights = vec![
            make_insight("TN", 0.9, 1, vec!["Supply_Chain"]),
        ];
        let filtered = filter_by_tag(&insights, "supply_chain");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_by_tag_no_match() {
        let insights = vec![make_insight("TN", 0.9, 1, vec!["competitive"])];
        let filtered = filter_by_tag(&insights, "security");
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_build_memo_metrics() {
        let m = build_memo_metrics(50, 5, 12, 3, 1, 8);
        assert_eq!(m.total_warnings, 50);
        assert_eq!(m.critical_warnings, 5);
        assert_eq!(m.new_insights, 12);
        assert_eq!(m.recipe_promotions, 3);
    }

    #[test]
    fn test_memo_response_serialization() {
        let memo = WeeklyMemoResponse {
            id: "memo-1".to_string(),
            week_label: "2024-W25".to_string(),
            executive_summary: "Key developments this week...".to_string(),
            sections: vec![MemoSection {
                title: "Supply Chain".to_string(),
                content: "No disruptions.".to_string(),
                priority: 1,
            }],
            key_metrics: build_memo_metrics(40, 3, 10, 2, 0, 5),
            generated_at: Utc::now(),
        };
        let json = serde_json::to_string(&memo).unwrap();
        assert!(json.contains("2024-W25"));
        assert!(json.contains("Supply Chain"));
    }

    #[test]
    fn test_insight_response_serialization() {
        let insight = make_insight("TN", 0.85, 5, vec!["tag1"]);
        let json = serde_json::to_string(&insight).unwrap();
        let back: InsightResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.region, "TN");
        assert!((back.confidence - 0.85).abs() < 0.001);
    }
}
