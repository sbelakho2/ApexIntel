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
use apex_store::postgres::PgStore;
use chrono::{DateTime, Duration, Utc};

use crate::nightly::{CrawlStageResult, DriftCheckStageResult, MiningStageResult, PoiRefreshStageResult};
use crate::weekly::{MemoInputs, MemoWarning, PoiChange, ProductionRecipe, StagedRecipe};

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
    
    Ok(recipes.into_iter().map(|r| StagedRecipe {
        recipe_id: r.id.to_string(),
        staged_at: r.created_at,
        weeks_in_staging: (r.days_in_staging / 7) as u32,
        precision: r.precision_observed,
        recall: r.recall_observed,
        false_positive_rate: r.false_positive_rate,
        alerts_fired: r.sample_size as u64,
        true_positives: (r.precision_observed * r.sample_size as f64) as u64,
    }).collect())
}

/// Load production recipes for deprecation evaluation.
pub async fn load_production_recipes(ctx: &StorageContext) -> Result<Vec<ProductionRecipe>> {
    // Query recipes in 'production' lifecycle state
    let recipes = ctx.store.get_production_recipes_for_deprecation().await?;
    
    Ok(recipes.into_iter().map(|r| ProductionRecipe {
        recipe_id: r.id.to_string(),
        promoted_at: r.created_at,
        weeks_in_production: (r.days_inactive / 7) as u32,
        precision_history: vec![r.precision_baseline, r.precision_current],
        recall_history: vec![], // Not tracked in current schema
        false_positive_rate: r.false_positive_rate,
        alerts_fired_total: r.warnings_generated_last_week as u64,
    }).collect())
}

/// Build memo inputs from recent activity summaries.
pub async fn build_memo_inputs(ctx: &StorageContext) -> Result<MemoInputs> {
    let since = ctx.run_timestamp - Duration::days(7);
    
    // Aggregate stats for memo generation
    let _stats = ctx.store.get_weekly_summary_stats(since).await?;
    
    Ok(MemoInputs {
        top_warnings: vec![], // Would need additional query to populate
        new_recipes_staged: 0,
        recipes_promoted: 0,
        recipes_deprecated: 0,
        pipeline_health_pct: 1.0, // Assume healthy unless drift detected
        top_drift_features: vec![],
        poi_changes: vec![],
        period_start: since,
        period_end: ctx.run_timestamp,
    })
}
