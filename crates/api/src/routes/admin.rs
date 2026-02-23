//! Admin route — request/response types and logic for admin/monitoring endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Crawl status for admin dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStatusResponse {
    pub total_sources: u32,
    pub active_sources: u32,
    pub failed_sources: u32,
    pub disabled_sources: u32,
    pub last_cycle_at: Option<DateTime<Utc>>,
    pub last_cycle_duration_secs: Option<u64>,
    pub avg_success_rate: f64,
    pub sources_by_region: HashMap<String, u32>,
    pub recent_failures: Vec<CrawlFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlFailure {
    pub source_url: String,
    pub error: String,
    pub failed_at: DateTime<Utc>,
    pub consecutive_failures: u32,
}

/// Recipe performance for admin dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePerformanceResponse {
    pub total_recipes: u32,
    pub production_count: u32,
    pub staging_count: u32,
    pub deprecated_count: u32,
    pub avg_precision: f64,
    pub avg_recall: f64,
    pub avg_fpr: f64,
    pub recipes_needing_attention: Vec<RecipeAlert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeAlert {
    pub recipe_id: String,
    pub recipe_name: String,
    pub alert_type: RecipeAlertType,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipeAlertType {
    LowPrecision,
    HighFpr,
    NoRecentFires,
    Declining,
}

impl RecipeAlertType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::LowPrecision => "low_precision",
            Self::HighFpr => "high_fpr",
            Self::NoRecentFires => "no_recent_fires",
            Self::Declining => "declining",
        }
    }
}

/// POI coverage for admin dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiCoverageResponse {
    pub total_pois: u32,
    pub by_region: HashMap<String, u32>,
    pub by_priority_tier: HashMap<String, u32>,
    pub avg_priority: f64,
    pub stale_count: u32,
    pub stale_threshold_days: u32,
    pub coverage_gaps: Vec<CoverageGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageGap {
    pub region: String,
    pub description: String,
    pub severity: String,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Classify a crawl source health status.
pub fn source_health(consecutive_failures: u32, last_success_hours: Option<u64>) -> SourceHealth {
    if consecutive_failures >= 5 {
        SourceHealth::Critical
    } else if consecutive_failures >= 3 {
        SourceHealth::Degraded
    } else if let Some(hours) = last_success_hours {
        if hours > 72 {
            SourceHealth::Stale
        } else {
            SourceHealth::Healthy
        }
    } else {
        SourceHealth::Healthy
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SourceHealth {
    Healthy,
    Stale,
    Degraded,
    Critical,
}

impl SourceHealth {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Stale => "stale",
            Self::Degraded => "degraded",
            Self::Critical => "critical",
        }
    }
}

/// Check if a recipe needs attention based on thresholds.
pub fn check_recipe_alert(
    recipe_id: &str,
    recipe_name: &str,
    precision: f64,
    fpr: f64,
    last_fired_days_ago: Option<u32>,
    precision_trend_declining: bool,
) -> Vec<RecipeAlert> {
    let mut alerts = Vec::new();

    if precision < 0.5 {
        alerts.push(RecipeAlert {
            recipe_id: recipe_id.to_string(),
            recipe_name: recipe_name.to_string(),
            alert_type: RecipeAlertType::LowPrecision,
            message: format!("Precision {:.0}% below 50% threshold", precision * 100.0),
        });
    }

    if fpr > 0.15 {
        alerts.push(RecipeAlert {
            recipe_id: recipe_id.to_string(),
            recipe_name: recipe_name.to_string(),
            alert_type: RecipeAlertType::HighFpr,
            message: format!("FPR {:.0}% exceeds 15% threshold", fpr * 100.0),
        });
    }

    if let Some(days) = last_fired_days_ago {
        if days > 30 {
            alerts.push(RecipeAlert {
                recipe_id: recipe_id.to_string(),
                recipe_name: recipe_name.to_string(),
                alert_type: RecipeAlertType::NoRecentFires,
                message: format!("No fires in {} days", days),
            });
        }
    }

    if precision_trend_declining {
        alerts.push(RecipeAlert {
            recipe_id: recipe_id.to_string(),
            recipe_name: recipe_name.to_string(),
            alert_type: RecipeAlertType::Declining,
            message: "Precision trend is declining".to_string(),
        });
    }

    alerts
}

/// Compute coverage score: tracked POIs / expected_minimum_per_region.
pub fn coverage_score(pois_by_region: &HashMap<String, u32>, expected_min: u32) -> f64 {
    if pois_by_region.is_empty() {
        return 0.0;
    }
    let total_regions = pois_by_region.len();
    let meeting_min = pois_by_region
        .values()
        .filter(|&&v| v >= expected_min)
        .count();
    meeting_min as f64 / total_regions as f64
}

/// Identify coverage gaps where region has fewer POIs than threshold.
pub fn identify_gaps(pois_by_region: &HashMap<String, u32>, threshold: u32) -> Vec<CoverageGap> {
    let mut gaps = Vec::new();
    for (region, count) in pois_by_region {
        if *count < threshold {
            let severity = if *count == 0 {
                "critical"
            } else if *count < threshold / 2 {
                "high"
            } else {
                "medium"
            };
            gaps.push(CoverageGap {
                region: region.clone(),
                description: format!("{} POIs tracked (minimum {})", count, threshold),
                severity: severity.to_string(),
            });
        }
    }
    gaps.sort_by(|a, b| a.region.cmp(&b.region));
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_health_healthy() {
        assert_eq!(source_health(0, Some(10)), SourceHealth::Healthy);
        assert_eq!(source_health(1, Some(24)), SourceHealth::Healthy);
    }

    #[test]
    fn test_source_health_stale() {
        assert_eq!(source_health(0, Some(100)), SourceHealth::Stale);
    }

    #[test]
    fn test_source_health_degraded() {
        assert_eq!(source_health(3, Some(10)), SourceHealth::Degraded);
        assert_eq!(source_health(4, Some(10)), SourceHealth::Degraded);
    }

    #[test]
    fn test_source_health_critical() {
        assert_eq!(source_health(5, Some(10)), SourceHealth::Critical);
        assert_eq!(source_health(10, None), SourceHealth::Critical);
    }

    #[test]
    fn test_check_recipe_alert_none() {
        let alerts = check_recipe_alert("r-1", "Good Recipe", 0.9, 0.05, Some(3), false);
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_check_recipe_alert_low_precision() {
        let alerts = check_recipe_alert("r-1", "Bad Recipe", 0.4, 0.05, Some(3), false);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert_type, RecipeAlertType::LowPrecision);
    }

    #[test]
    fn test_check_recipe_alert_high_fpr() {
        let alerts = check_recipe_alert("r-1", "Noisy Recipe", 0.8, 0.20, Some(3), false);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert_type, RecipeAlertType::HighFpr);
    }

    #[test]
    fn test_check_recipe_alert_no_fires() {
        let alerts = check_recipe_alert("r-1", "Dead Recipe", 0.8, 0.05, Some(45), false);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert_type, RecipeAlertType::NoRecentFires);
    }

    #[test]
    fn test_check_recipe_alert_declining() {
        let alerts = check_recipe_alert("r-1", "Declining Recipe", 0.7, 0.05, Some(3), true);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert_type, RecipeAlertType::Declining);
    }

    #[test]
    fn test_check_recipe_alert_multiple() {
        let alerts = check_recipe_alert("r-1", "Terrible", 0.3, 0.25, Some(60), true);
        assert_eq!(alerts.len(), 4); // all 4 alerts
    }

    #[test]
    fn test_coverage_score() {
        let mut pois = HashMap::new();
        pois.insert("TN".to_string(), 10);
        pois.insert("MA".to_string(), 5);
        pois.insert("EU".to_string(), 3);
        // expected_min = 5 → TN (10>=5) ✓, MA (5>=5) ✓, EU (3<5) ✗
        let score = coverage_score(&pois, 5);
        assert!((score - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_coverage_score_empty() {
        assert_eq!(coverage_score(&HashMap::new(), 5), 0.0);
    }

    #[test]
    fn test_identify_gaps() {
        let mut pois = HashMap::new();
        pois.insert("TN".to_string(), 10);
        pois.insert("MA".to_string(), 3);
        pois.insert("EA".to_string(), 0);
        let gaps = identify_gaps(&pois, 5);
        assert_eq!(gaps.len(), 2); // MA and EA
        // sorted alphabetically
        assert_eq!(gaps[0].region, "EA");
        assert_eq!(gaps[0].severity, "critical"); // 0 POIs
        assert_eq!(gaps[1].region, "MA");
        assert_eq!(gaps[1].severity, "medium"); // 3 >= 5/2=2, so medium
    }

    #[test]
    fn test_identify_gaps_severity() {
        let mut pois = HashMap::new();
        pois.insert("TN".to_string(), 0);  // critical (0)
        pois.insert("MA".to_string(), 2);  // high (< 10/2 = 5)
        pois.insert("EU".to_string(), 7);  // medium (>= 5, < 10)
        let gaps = identify_gaps(&pois, 10);
        assert_eq!(gaps.len(), 3);
        let tn = gaps.iter().find(|g| g.region == "TN").unwrap();
        assert_eq!(tn.severity, "critical");
        let ma = gaps.iter().find(|g| g.region == "MA").unwrap();
        assert_eq!(ma.severity, "high");
        let eu = gaps.iter().find(|g| g.region == "EU").unwrap();
        assert_eq!(eu.severity, "medium");
    }

    #[test]
    fn test_identify_gaps_none() {
        let mut pois = HashMap::new();
        pois.insert("TN".to_string(), 20);
        let gaps = identify_gaps(&pois, 5);
        assert!(gaps.is_empty());
    }

    #[test]
    fn test_recipe_alert_type_label() {
        assert_eq!(RecipeAlertType::LowPrecision.label(), "low_precision");
        assert_eq!(RecipeAlertType::HighFpr.label(), "high_fpr");
    }

    #[test]
    fn test_source_health_label() {
        assert_eq!(SourceHealth::Healthy.label(), "healthy");
        assert_eq!(SourceHealth::Critical.label(), "critical");
    }

    #[test]
    fn test_crawl_status_response_serialization() {
        let mut by_region = HashMap::new();
        by_region.insert("TN".to_string(), 50);
        let resp = CrawlStatusResponse {
            total_sources: 612,
            active_sources: 580,
            failed_sources: 20,
            disabled_sources: 12,
            last_cycle_at: Some(Utc::now()),
            last_cycle_duration_secs: Some(300),
            avg_success_rate: 0.967,
            sources_by_region: by_region,
            recent_failures: vec![CrawlFailure {
                source_url: "https://example.com".to_string(),
                error: "timeout".to_string(),
                failed_at: Utc::now(),
                consecutive_failures: 3,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("612"));
        assert!(json.contains("timeout"));
    }

    #[test]
    fn test_poi_coverage_response_serialization() {
        let mut by_region = HashMap::new();
        by_region.insert("TN".to_string(), 35);
        let mut by_tier = HashMap::new();
        by_tier.insert("critical".to_string(), 10);
        let resp = PoiCoverageResponse {
            total_pois: 120,
            by_region,
            by_priority_tier: by_tier,
            avg_priority: 0.65,
            stale_count: 5,
            stale_threshold_days: 30,
            coverage_gaps: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("120"));
    }
}
