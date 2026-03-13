//! Storage integration layer for the worker pipeline.
//!
//! This module provides functions to build pipeline stage results from the
//! actual database state, replacing the file-based stub approach.
//!
//! # Architecture
//!
//! The worker pipeline was originally designed with pure orchestration logic
//! that operates on pre-computed stage results. This module bridges the gap
//! between the database and those pure functions by:
//!
//! 1. Querying the database for relevant data
//! 2. Constructing the appropriate stage result structs
//! 3. Returning them for processing by the pipeline stages
//!
//! This keeps the core pipeline logic testable and pure while allowing
//! real database integration in production.

use anyhow::Result;
use apex_store::postgres::{PgStore, WarningListFilters, WarningOrderBy};
use chrono::{DateTime, Duration, Utc};
use sqlx::Row;

use crate::nightly::{
    CrawlStageResult, DriftCheckStageResult, MiningStageResult, PoiRefreshStageResult,
};
use crate::weekly::{MemoInputs, ProductionRecipe, StagedRecipe};

/// Context for building pipeline inputs from storage.
pub struct StorageContext {
    pub store: PgStore,
    pub run_timestamp: DateTime<Utc>,
}

impl StorageContext {
    pub async fn new(database_url: &str) -> Result<Self> {
        let store = PgStore::connect(database_url).await?;
        Ok(Self {
            store,
            run_timestamp: Utc::now(),
        })
    }
}

// ────────────────────────────────────────────
// Nightly Pipeline - Storage Integration
// ────────────────────────────────────────────

/// Build CrawlStageResult from recent crawl activity in the database.
///
/// Queries the observations and crawl logs from the last 24 hours to
/// construct the input for the crawl processing stage.
pub async fn build_crawl_result(ctx: &StorageContext) -> Result<CrawlStageResult> {
    // Get crawl stats from the last 24 hours
    let since = ctx.run_timestamp - Duration::hours(24);

    // Query recent crawl activity
    let crawl_stats = ctx.store.get_crawl_stats(since).await?;

    Ok(CrawlStageResult {
        sources_attempted: crawl_stats.sources_attempted,
        sources_succeeded: crawl_stats.sources_succeeded,
        sources_failed: crawl_stats.sources_failed,
        new_observations: crawl_stats.new_observations,
        changed_pages: crawl_stats.changed_pages,
        bytes_fetched: crawl_stats.bytes_fetched,
        errors: crawl_stats.errors,
    })
}

/// Build MiningStageResult from recent pattern mining activity.
///
/// Queries the pattern_candidates and recipes tables to construct
/// the input for the mining stage processing.
pub async fn build_mining_result(ctx: &StorageContext) -> Result<MiningStageResult> {
    let since = ctx.run_timestamp - Duration::hours(24);

    // Query mining stats from the last 24 hours
    let mining_stats = ctx.store.get_mining_stats(since).await?;

    Ok(MiningStageResult {
        candidates_found: mining_stats.candidates_found,
        candidates_passed_gates: mining_stats.candidates_passed_gates,
        hypotheses_generated: mining_stats.hypotheses_generated,
        recipes_staged: mining_stats.recipes_staged,
        errors: mining_stats.errors,
    })
}

/// Build PoiRefreshStageResult from recent POI scanning activity.
pub async fn build_poi_result(ctx: &StorageContext) -> Result<PoiRefreshStageResult> {
    let since = ctx.run_timestamp - Duration::hours(24);

    let poi_stats = ctx.store.get_poi_stats(since).await?;

    Ok(PoiRefreshStageResult {
        profiles_scanned: poi_stats.profiles_scanned,
        profiles_updated: poi_stats.profiles_updated,
        new_pois_discovered: poi_stats.new_pois_discovered,
        role_changes_detected: poi_stats.role_changes_detected,
        errors: poi_stats.errors,
    })
}

/// Build DriftCheckStageResult from feature store drift monitoring.
pub async fn build_drift_result(ctx: &StorageContext) -> Result<DriftCheckStageResult> {
    // Get drift scores from feature store
    let drift_stats = ctx.store.get_drift_stats().await?;

    Ok(DriftCheckStageResult {
        features_checked: drift_stats.features_checked,
        features_drifted: drift_stats.features_drifted,
        drift_scores: drift_stats.drift_scores,
        alerts_raised: drift_stats.alerts_raised,
        errors: drift_stats.errors,
    })
}

// ────────────────────────────────────────────
// Weekly Pipeline - Storage Integration
// ────────────────────────────────────────────

/// Load staged recipes that are ready for promotion evaluation.
pub async fn load_staged_recipes(ctx: &StorageContext) -> Result<Vec<StagedRecipe>> {
    // Query recipes in 'staged' lifecycle state with sufficient history
    let recipes = ctx.store.get_staged_recipes_for_promotion().await?;

    Ok(recipes.into_iter().map(map_staged_recipe_row).collect())
}

/// Load production recipes for deprecation evaluation.
pub async fn load_production_recipes(ctx: &StorageContext) -> Result<Vec<ProductionRecipe>> {
    // Query recipes in 'production' lifecycle state
    let recipes = ctx.store.get_production_recipes_for_deprecation().await?;

    Ok(recipes.into_iter().map(map_production_recipe_row).collect())
}

fn map_staged_recipe_row(r: apex_store::postgres::StagedRecipeRow) -> StagedRecipe {
    let alerts_fired = r.sample_size.max(0) as u64;
    StagedRecipe {
        recipe_id: r.recipe_code,
        staged_at: r.created_at,
        weeks_in_staging: (r.days_in_staging.max(0) / 7) as u32,
        precision: r.precision_observed,
        recall: r.recall_observed,
        false_positive_rate: r.false_positive_rate,
        alerts_fired,
        true_positives: (r.precision_observed * alerts_fired as f64) as u64,
    }
}

fn map_production_recipe_row(r: apex_store::postgres::ProductionRecipeRow) -> ProductionRecipe {
    ProductionRecipe {
        recipe_id: r.recipe_code,
        promoted_at: r.created_at,
        weeks_in_production: (r.days_inactive.max(0) / 7) as u32,
        precision_history: vec![r.precision_baseline, r.precision_current],
        recall_history: vec![],
        false_positive_rate: r.false_positive_rate,
        alerts_fired_total: r.warnings_generated_last_week.max(0) as u64,
    }
}

/// Build memo inputs from recent activity summaries.
pub async fn build_memo_inputs(ctx: &StorageContext) -> Result<MemoInputs> {
    let since = ctx.run_timestamp - Duration::days(7);

    let _stats = ctx.store.get_weekly_summary_stats(since).await?;
    let drift = ctx.store.get_drift_stats().await.unwrap_or_default();

    let warning_filters = WarningListFilters {
        date_from: Some(since),
        ..WarningListFilters::default()
    };
    let warning_rows = ctx
        .store
        .list_warnings(&warning_filters, Some(WarningOrderBy::Severity), true, 5, 0)
        .await
        .unwrap_or_default();
    let top_warnings = warning_rows
        .into_iter()
        .map(|row| crate::weekly::MemoWarning {
            id: row.id.to_string(),
            headline: row.title,
            impact: row.description.unwrap_or_else(|| row.warning_type),
            confidence: row.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        })
        .collect();

    let new_recipes_staged: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recipes WHERE status = 'staging' AND created_at >= $1",
    )
    .bind(since)
    .fetch_one(&ctx.store.pool)
    .await
    .unwrap_or(0);

    let recipes_promoted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recipes WHERE status IN ('active', 'production') AND updated_at >= $1",
    )
    .bind(since)
    .fetch_one(&ctx.store.pool)
    .await
    .unwrap_or(0);

    let recipes_deprecated: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recipes WHERE status = 'deprecated' AND updated_at >= $1",
    )
    .bind(since)
    .fetch_one(&ctx.store.pool)
    .await
    .unwrap_or(0);

    let poi_changes_rows = sqlx::query(
        r#"SELECT
                COALESCE(p.name, pc.person_id::text) AS person_name,
                pc.change_type,
                COALESCE(NULLIF(pc.new_value, ''), NULLIF(pc.old_value, ''), 'change detected') AS details
           FROM person_changes pc
           LEFT JOIN persons p ON p.id = pc.person_id
           WHERE pc.detected_at >= $1
           ORDER BY pc.detected_at DESC
           LIMIT 10"#,
    )
    .bind(since)
    .fetch_all(&ctx.store.pool)
    .await
    .unwrap_or_default();
    let poi_changes = poi_changes_rows
        .into_iter()
        .map(|row| crate::weekly::PoiChange {
            person_name: row
                .try_get::<String, _>("person_name")
                .unwrap_or_else(|_| "Unknown".to_string()),
            change_type: row
                .try_get::<String, _>("change_type")
                .unwrap_or_else(|_| "change".to_string()),
            details: row
                .try_get::<String, _>("details")
                .unwrap_or_else(|_| "change detected".to_string()),
        })
        .collect();

    let mut top_drift_features = drift.drift_scores;
    top_drift_features.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top_drift_features.truncate(5);

    let pipeline_health_pct = if drift.features_checked == 0 {
        1.0
    } else {
        (1.0 - (drift.features_drifted as f64 / drift.features_checked as f64)).clamp(0.0, 1.0)
    };

    Ok(MemoInputs {
        top_warnings,
        new_recipes_staged: new_recipes_staged.max(0) as u32,
        recipes_promoted: recipes_promoted.max(0) as u32,
        recipes_deprecated: recipes_deprecated.max(0) as u32,
        pipeline_health_pct,
        top_drift_features,
        poi_changes,
        period_start: since,
        period_end: ctx.run_timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_store::postgres::{ProductionRecipeRow, StagedRecipeRow};
    use uuid::Uuid;

    #[test]
    fn staged_recipe_mapping_uses_recipe_code_and_clamps_counts() {
        let row = StagedRecipeRow {
            id: Uuid::new_v4(),
            recipe_code: "R-STAGE-1".to_string(),
            precision_observed: 0.9,
            recall_observed: 0.55,
            false_positive_rate: 0.04,
            sample_size: 5,
            days_in_staging: 29,
            created_at: Utc::now(),
        };

        let recipe = map_staged_recipe_row(row);
        assert_eq!(recipe.recipe_id, "R-STAGE-1");
        assert_eq!(recipe.weeks_in_staging, 4);
        assert_eq!(recipe.alerts_fired, 5);
        assert_eq!(recipe.true_positives, 4);
    }

    #[test]
    fn production_recipe_mapping_uses_recent_warning_volume() {
        let row = ProductionRecipeRow {
            id: Uuid::new_v4(),
            recipe_code: "R-PROD-1".to_string(),
            precision_current: 0.78,
            precision_baseline: 0.75,
            false_positive_rate: 0.11,
            fpr_baseline: 0.09,
            warnings_generated_last_week: 7,
            last_triggered_at: Some(Utc::now()),
            days_inactive: 13,
            created_at: Utc::now(),
        };

        let recipe = map_production_recipe_row(row);
        assert_eq!(recipe.recipe_id, "R-PROD-1");
        assert_eq!(recipe.weeks_in_production, 1);
        assert_eq!(recipe.precision_history, vec![0.75, 0.78]);
        assert_eq!(recipe.alerts_fired_total, 7);
    }
}
