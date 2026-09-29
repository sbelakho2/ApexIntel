//! Recipes route — request/response types and logic for recipe management endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Request types
// ────────────────────────────────────────────

/// Query parameters for listing recipes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRecipesQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub min_precision: Option<f64>,
    pub region: Option<String>,
    pub sort_by: Option<RecipeSortField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipeSortField {
    Name,
    Precision,
    Recall,
    CreatedAt,
    FiredCount,
}

impl RecipeSortField {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "name" => Some(Self::Name),
            "precision" => Some(Self::Precision),
            "recall" => Some(Self::Recall),
            "created_at" | "created" | "date" => Some(Self::CreatedAt),
            "fired" | "fired_count" | "alerts" => Some(Self::FiredCount),
            _ => None,
        }
    }
}

/// Promote recipe request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteRequest {
    pub promoted_by: String,
    pub reason: Option<String>,
}

// ────────────────────────────────────────────
// Response types
// ────────────────────────────────────────────

/// Recipe list item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeListItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: RecipeStatus,
    pub region: Option<String>,
    /// Empirical precision from reviewed outcomes; `None` = not measured.
    pub precision: Option<f64>,
    /// Promotion-evidence heuristic, not recall (there is no evaluation-set
    /// recall to report yet).
    pub promotion_evidence_score: f64,
    /// False-positive rate from reviewed outcomes; `None` = not measured.
    pub false_positive_rate: Option<f64>,
    pub fired_count: u32,
    pub last_fired: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipeStatus {
    Staging,
    Production,
    Deprecated,
}

impl RecipeStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Production => "production",
            Self::Deprecated => "deprecated",
        }
    }
}

/// Promote response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteResponse {
    pub recipe_id: String,
    pub previous_status: RecipeStatus,
    pub new_status: RecipeStatus,
    pub promoted_by: String,
    pub promoted_at: DateTime<Utc>,
}

/// Recipe performance detail (for admin endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePerformanceDetail {
    pub recipe_id: String,
    pub recipe_name: String,
    pub precision_trend: Vec<f64>,
    pub recall_trend: Vec<f64>,
    pub weekly_fires: Vec<u32>,
    pub avg_precision: f64,
    pub avg_recall: f64,
}

// ────────────────────────────────────────────
// Logic
// ────────────────────────────────────────────

/// Ascending comparison that keeps unmeasured (`None`) values last.
fn cmp_measured_ascending<T: PartialOrd>(a: &Option<T>, b: &Option<T>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Sort recipes.
pub fn sort_recipes(items: &mut [RecipeListItem], field: &RecipeSortField, desc: bool) {
    items.sort_by(|a, b| {
        let cmp = match field {
            RecipeSortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            // Ascending by measured value; unmeasured values sort last.
            RecipeSortField::Precision => cmp_measured_ascending(&a.precision, &b.precision),
            RecipeSortField::Recall => a
                .promotion_evidence_score
                .partial_cmp(&b.promotion_evidence_score)
                .unwrap_or(std::cmp::Ordering::Equal),
            RecipeSortField::CreatedAt => a.created_at.cmp(&b.created_at),
            RecipeSortField::FiredCount => a.fired_count.cmp(&b.fired_count),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Filter recipes by status.
pub fn filter_by_status<'a>(
    items: &'a [RecipeListItem],
    status: &RecipeStatus,
) -> Vec<&'a RecipeListItem> {
    items.iter().filter(|r| r.status == *status).collect()
}

/// Filter recipes by minimum precision.
pub fn filter_by_precision(items: &[RecipeListItem], min: f64) -> Vec<&RecipeListItem> {
    // Unmeasured precision never passes a numeric threshold.
    items
        .iter()
        .filter(|r| r.precision.is_some_and(|precision| precision >= min))
        .collect()
}

/// Validate a promote request.
pub fn validate_promote(req: &PromoteRequest) -> Result<(), String> {
    if req.promoted_by.trim().is_empty() {
        return Err("promoted_by is required".to_string());
    }
    if let Some(reason) = &req.reason {
        if reason.chars().count() > 500 {
            return Err("reason must be <= 500 characters".to_string());
        }
    }
    Ok(())
}

/// Check if a recipe is eligible for promotion (must be staging + meet min thresholds).
pub fn is_promotable(
    recipe: &RecipeListItem,
    min_precision: f64,
    min_promotion_evidence: f64,
) -> bool {
    recipe.status == RecipeStatus::Staging
        && recipe
            .precision
            .is_some_and(|precision| precision >= min_precision)
        && recipe.promotion_evidence_score >= min_promotion_evidence
        // An unmeasured FPR cannot satisfy a verifiable threshold.
        && recipe
            .false_positive_rate
            .is_some_and(|fpr| fpr <= 1.0)
}

/// Recipe health score combining measured precision, promotion evidence and
/// activity. Unmeasured precision/FPR contribute nothing rather than zeros.
pub fn recipe_health(recipe: &RecipeListItem) -> f64 {
    let precision_score = recipe.precision.unwrap_or(0.0);
    let recall_score = recipe.promotion_evidence_score;
    let fpr_penalty = recipe.false_positive_rate.unwrap_or(0.0);
    let activity_bonus = if recipe.fired_count > 0 { 0.1 } else { 0.0 };

    let score: f64 =
        0.4 * precision_score + 0.3 * recall_score - 0.2 * fpr_penalty + activity_bonus;
    score.clamp(0.0, 1.0)
}

/// Compute aggregate recipe stats.
pub fn recipe_stats(items: &[RecipeListItem]) -> RecipeAggregateStats {
    let total = items.len();
    let production = items
        .iter()
        .filter(|r| r.status == RecipeStatus::Production)
        .count();
    let staging = items
        .iter()
        .filter(|r| r.status == RecipeStatus::Staging)
        .count();
    let deprecated = items
        .iter()
        .filter(|r| r.status == RecipeStatus::Deprecated)
        .count();

    let active: Vec<_> = items
        .iter()
        .filter(|r| r.status == RecipeStatus::Production)
        .collect();
    // Averages over measured values only; nothing measured is `None`, never a
    // synthesized zero.
    let measured_precision: Vec<f64> = active.iter().filter_map(|r| r.precision).collect();
    let avg_precision = if measured_precision.is_empty() {
        None
    } else {
        Some(measured_precision.iter().sum::<f64>() / measured_precision.len() as f64)
    };
    let avg_promotion_evidence = if active.is_empty() {
        0.0
    } else {
        active
            .iter()
            .map(|r| r.promotion_evidence_score)
            .sum::<f64>()
            / active.len() as f64
    };

    RecipeAggregateStats {
        total,
        production,
        staging,
        deprecated,
        avg_precision,
        avg_promotion_evidence,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeAggregateStats {
    pub total: usize,
    pub production: usize,
    pub staging: usize,
    pub deprecated: usize,
    /// Mean measured precision; `None` when nothing is measured.
    pub avg_precision: Option<f64>,
    /// Mean promotion-evidence heuristic (not recall; no evaluation-set recall
    /// exists yet).
    pub avg_promotion_evidence: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_recipe(
        name: &str,
        status: RecipeStatus,
        precision: f64,
        promotion_evidence: f64,
        fpr: f64,
        fired: u32,
    ) -> RecipeListItem {
        RecipeListItem {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: format!("{} desc", name),
            status,
            region: Some("TN".to_string()),
            precision: Some(precision),
            promotion_evidence_score: promotion_evidence,
            false_positive_rate: Some(fpr),
            fired_count: fired,
            last_fired: if fired > 0 { Some(Utc::now()) } else { None },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_sort_recipes_by_precision_desc() {
        let mut items = vec![
            make_recipe("A", RecipeStatus::Production, 0.7, 0.5, 0.05, 10),
            make_recipe("B", RecipeStatus::Production, 0.95, 0.6, 0.02, 20),
            make_recipe("C", RecipeStatus::Staging, 0.8, 0.4, 0.1, 3),
        ];
        sort_recipes(&mut items, &RecipeSortField::Precision, true);
        assert_eq!(items[0].name, "B");
        assert_eq!(items[2].name, "A");
    }

    #[test]
    fn test_sort_recipes_by_name_asc() {
        let mut items = vec![
            make_recipe("Zeta", RecipeStatus::Production, 0.8, 0.5, 0.05, 10),
            make_recipe("Alpha", RecipeStatus::Production, 0.9, 0.6, 0.02, 20),
        ];
        sort_recipes(&mut items, &RecipeSortField::Name, false);
        assert_eq!(items[0].name, "Alpha");
    }

    #[test]
    fn test_filter_by_status() {
        let items = vec![
            make_recipe("A", RecipeStatus::Production, 0.8, 0.5, 0.05, 10),
            make_recipe("B", RecipeStatus::Staging, 0.7, 0.4, 0.1, 3),
            make_recipe("C", RecipeStatus::Production, 0.9, 0.6, 0.02, 20),
        ];
        let prod = filter_by_status(&items, &RecipeStatus::Production);
        assert_eq!(prod.len(), 2);
        let staging = filter_by_status(&items, &RecipeStatus::Staging);
        assert_eq!(staging.len(), 1);
    }

    #[test]
    fn test_filter_by_precision() {
        let items = vec![
            make_recipe("A", RecipeStatus::Production, 0.8, 0.5, 0.05, 10),
            make_recipe("B", RecipeStatus::Production, 0.6, 0.4, 0.1, 5),
        ];
        let filtered = filter_by_precision(&items, 0.7);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "A");
    }

    #[test]
    fn test_validate_promote_ok() {
        let req = PromoteRequest {
            promoted_by: "admin".to_string(),
            reason: Some("Meets all thresholds".to_string()),
        };
        assert!(validate_promote(&req).is_ok());
    }

    #[test]
    fn test_validate_promote_empty_user() {
        let req = PromoteRequest {
            promoted_by: "  ".to_string(),
            reason: None,
        };
        assert!(validate_promote(&req).is_err());
    }

    #[test]
    fn test_validate_promote_long_reason() {
        let req = PromoteRequest {
            promoted_by: "admin".to_string(),
            reason: Some("x".repeat(501)),
        };
        assert!(validate_promote(&req).is_err());
    }

    #[test]
    fn test_is_promotable() {
        let staging = make_recipe("A", RecipeStatus::Staging, 0.9, 0.5, 0.02, 5);
        assert!(is_promotable(&staging, 0.85, 0.3));
        assert!(!is_promotable(&staging, 0.95, 0.3)); // precision too low

        let prod = make_recipe("B", RecipeStatus::Production, 0.95, 0.7, 0.01, 50);
        assert!(!is_promotable(&prod, 0.85, 0.3)); // already production
    }

    #[test]
    fn test_recipe_health() {
        let r = make_recipe("A", RecipeStatus::Production, 0.9, 0.7, 0.05, 10);
        let h = recipe_health(&r);
        // 0.4*0.9 + 0.3*0.7 - 0.2*0.05 + 0.1 = 0.36 + 0.21 - 0.01 + 0.1 = 0.66
        assert!((h - 0.66).abs() < 0.01);
    }

    #[test]
    fn test_recipe_health_clamped() {
        let r = make_recipe("A", RecipeStatus::Production, 0.0, 0.0, 1.0, 0);
        let h = recipe_health(&r);
        assert!(h >= 0.0);
    }

    #[test]
    fn test_recipe_stats() {
        let items = vec![
            make_recipe("A", RecipeStatus::Production, 0.9, 0.7, 0.05, 10),
            make_recipe("B", RecipeStatus::Production, 0.8, 0.5, 0.03, 20),
            make_recipe("C", RecipeStatus::Staging, 0.7, 0.4, 0.1, 3),
            make_recipe("D", RecipeStatus::Deprecated, 0.3, 0.1, 0.2, 0),
        ];
        let stats = recipe_stats(&items);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.production, 2);
        assert_eq!(stats.staging, 1);
        assert_eq!(stats.deprecated, 1);
        assert!((stats.avg_precision.unwrap() - 0.85).abs() < 0.01); // (0.9+0.8)/2
        assert!((stats.avg_promotion_evidence - 0.6).abs() < 0.01); // (0.7+0.5)/2
    }

    #[test]
    fn test_recipe_stats_empty() {
        let stats = recipe_stats(&[]);
        assert_eq!(stats.total, 0);
        assert_eq!(
            stats.avg_precision, None,
            "nothing measured must be None, not 0"
        );
    }

    #[test]
    fn test_recipe_sort_field_from_str() {
        assert_eq!(
            RecipeSortField::from_str_loose("precision"),
            Some(RecipeSortField::Precision)
        );
        assert_eq!(
            RecipeSortField::from_str_loose("alerts"),
            Some(RecipeSortField::FiredCount)
        );
        assert_eq!(RecipeSortField::from_str_loose("xyz"), None);
    }

    #[test]
    fn test_recipe_status_label() {
        assert_eq!(RecipeStatus::Staging.label(), "staging");
        assert_eq!(RecipeStatus::Production.label(), "production");
        assert_eq!(RecipeStatus::Deprecated.label(), "deprecated");
    }

    #[test]
    fn test_recipe_list_item_serialization() {
        let r = make_recipe(
            "Test Recipe",
            RecipeStatus::Production,
            0.88,
            0.65,
            0.03,
            15,
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("Test Recipe"));
        assert!(json.contains("Production"));
    }
}
