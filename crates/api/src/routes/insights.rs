//! Insights route — request/response types and logic for insight endpoints.

use apex_core::validation::clamp_ratio;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing insights.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInsightsQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub regions: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub search: Option<String>,
    pub insight_type: Option<String>,
    /// When "true", only return bookmarked insights.
    pub bookmarked: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightFeedbackRequest {
    pub feedback_type: String,
    pub notes: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_gain_bits: Option<f64>,
    #[serde(default)]
    pub information_gain_sparkline: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diversity_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diversity_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causal_flag: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookmarked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
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

    diversify_ranked_insights(insights);
}

fn insight_score(insight: &InsightResponse, now: &DateTime<Utc>) -> f64 {
    let age_hours = (*now - insight.created_at).num_hours().max(1) as f64;
    let recency = 1.0 / (1.0 + age_hours / 24.0);
    let quality = insight.quality_score.unwrap_or(0.5);
    clamp_ratio((0.60 * insight.confidence + 0.40 * quality) * (0.75 + 0.25 * recency))
}

fn diversify_ranked_insights(insights: &mut [InsightResponse]) {
    if insights.len() < 3 {
        return;
    }

    let mut diversified = Vec::with_capacity(insights.len());
    let mut remaining: Vec<InsightResponse> = insights.to_vec();
    let mut last_type: Option<String> = None;
    let mut consecutive = 0usize;

    while !remaining.is_empty() {
        let selected_idx = remaining
            .iter()
            .position(|candidate| {
                let candidate_type = candidate.insight_type.to_ascii_lowercase();
                match last_type.as_deref() {
                    Some(previous) if previous == candidate_type => consecutive < 2,
                    _ => true,
                }
            })
            .unwrap_or(0);

        let selected = remaining.remove(selected_idx);
        let selected_type = selected.insight_type.to_ascii_lowercase();
        if last_type.as_deref() == Some(selected_type.as_str()) {
            consecutive += 1;
        } else {
            last_type = Some(selected_type);
            consecutive = 1;
        }
        diversified.push(selected);
    }

    insights.clone_from_slice(&diversified);
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

    fn make_insight(
        region: &str,
        confidence: f64,
        hours_ago: i64,
        tags: Vec<&str>,
    ) -> InsightResponse {
        let ts = Utc::now() - chrono::Duration::hours(hours_ago);
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
            information_gain_bits: Some(0.12),
            information_gain_sparkline: vec![0.02, 0.07, 0.12],
            diversity_score: Some(0.66),
            diversity_label: Some("diverse".to_string()),
            causal_flag: None,
            created_at: ts,
            updated_at: ts,
            bookmarked: None,
            quality_score: Some(0.5),
        }
    }

    #[test]
    fn test_rank_insights_by_score() {
        let mut insights = vec![
            make_insight("TN", 0.5, 48, vec![]), // old, low conf
            make_insight("MA", 0.95, 1, vec![]), // recent, high conf
            make_insight("EU", 0.8, 12, vec![]),
        ];
        rank_insights(&mut insights);
        // best score first (high confidence + recent)
        assert_eq!(insights[0].region, "MA");
    }

    #[test]
    fn test_rank_insights_diversifies_after_two_of_same_type() {
        let ts = Utc::now() - chrono::Duration::hours(1);
        let mut insights = vec![
            InsightResponse {
                id: uuid::Uuid::new_v4().to_string(),
                title: "A1".to_string(),
                summary: "Test".to_string(),
                insight_type: "arbitrage_cost_window".to_string(),
                region: "CN".to_string(),
                confidence: 0.99,
                evidence_urls: vec![],
                entity_ids: vec![],
                tags: vec![],
                information_gain_bits: None,
                information_gain_sparkline: vec![],
                diversity_score: None,
                diversity_label: None,
                causal_flag: None,
                created_at: ts,
                updated_at: ts,
                bookmarked: None,
                quality_score: Some(0.5),
            },
            InsightResponse {
                id: uuid::Uuid::new_v4().to_string(),
                title: "A2".to_string(),
                summary: "Test".to_string(),
                insight_type: "arbitrage_cost_window".to_string(),
                region: "MA".to_string(),
                confidence: 0.98,
                evidence_urls: vec![],
                entity_ids: vec![],
                tags: vec![],
                information_gain_bits: None,
                information_gain_sparkline: vec![],
                diversity_score: None,
                diversity_label: None,
                causal_flag: None,
                created_at: ts,
                updated_at: ts,
                bookmarked: None,
                quality_score: Some(0.5),
            },
            InsightResponse {
                id: uuid::Uuid::new_v4().to_string(),
                title: "A3".to_string(),
                summary: "Test".to_string(),
                insight_type: "arbitrage_cost_window".to_string(),
                region: "TN".to_string(),
                confidence: 0.97,
                evidence_urls: vec![],
                entity_ids: vec![],
                tags: vec![],
                information_gain_bits: None,
                information_gain_sparkline: vec![],
                diversity_score: None,
                diversity_label: None,
                causal_flag: None,
                created_at: ts,
                updated_at: ts,
                bookmarked: None,
                quality_score: Some(0.5),
            },
            InsightResponse {
                id: uuid::Uuid::new_v4().to_string(),
                title: "H1".to_string(),
                summary: "Test".to_string(),
                insight_type: "hypothesis_ach".to_string(),
                region: "EU".to_string(),
                confidence: 0.70,
                evidence_urls: vec![],
                entity_ids: vec![],
                tags: vec![],
                information_gain_bits: None,
                information_gain_sparkline: vec![],
                diversity_score: None,
                diversity_label: None,
                causal_flag: None,
                created_at: ts,
                updated_at: ts,
                bookmarked: None,
                quality_score: Some(0.5),
            },
        ];

        rank_insights(&mut insights);

        assert_eq!(insights[0].insight_type, "arbitrage_cost_window");
        assert_eq!(insights[1].insight_type, "arbitrage_cost_window");
        assert_eq!(insights[2].insight_type, "hypothesis_ach");
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
        let insights = vec![make_insight("TN", 0.9, 1, vec!["Supply_Chain"])];
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
