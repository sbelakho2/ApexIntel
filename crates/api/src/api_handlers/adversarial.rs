//! API handlers for adversarial detection & defense.
//!
//! Provides endpoints for:
//! - Coordinated placement detection
//! - Quarantine queue management
//! - Source reliability tracking
//!
//! These endpoints surface data from the adversarial pipeline
//! which analyzes source entropy, token similarity, and placement patterns.
//! Data is populated by the worker AdversarialAnalysis job.

use std::sync::Arc;

use axum::{extract::Query, Extension, Json};
use serde::Deserialize;

use apex_api::responses::ApiError;
use apex_shared::{PlacementAlert, QuarantineItem, SourceReliabilityHistory, SourceReliabilityTier};
use apex_store::postgres::PgStore;
use chrono::Utc;

#[derive(Debug, Deserialize)]
pub struct AdversarialQueryParams {
    pub domain: Option<String>,
    pub limit: Option<i64>,
}

/// GET /api/adversarial/placements
/// Returns detected coordinated placement clusters from the adversarial pipeline.
/// Data sourced from pattern_candidates table populated by AdversarialAnalysis worker.
pub async fn get_placements(
    store: Extension<Arc<PgStore>>,
    params: Query<AdversarialQueryParams>,
) -> Result<Json<Vec<PlacementAlert>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query(
        r#"SELECT
               id,
               metadata,
               confidence,
               created_at
           FROM pattern_candidates
           WHERE pattern_type = 'adversarial_placement'
           ORDER BY created_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(&store.pool)
    .await
    .map_err(|e| ApiError::internal(format!("failed to query placements: {e}")))?;

    let mut placements = Vec::new();
    for row in &rows {
        use sqlx::Row;
        let id: uuid::Uuid = row.try_get("id").unwrap_or_default();
        let metadata: serde_json::Value = row.try_get("metadata").unwrap_or_default();
        let confidence: f64 = row.try_get("confidence").unwrap_or(0.0);
        let created_at: chrono::DateTime<Utc> = row.try_get("created_at").unwrap_or_else(|_| Utc::now());

        let source_count = metadata
            .get("source_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let time_window_hours = metadata
            .get("time_window_hours")
            .and_then(|v| v.as_u64())
            .unwrap_or(6) as u32;
        let token_jaccard = metadata
            .get("token_jaccard")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let signal_ids: Vec<String> = metadata
            .get("signal_ids")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        // Filter by domain if requested
        if let Some(ref domain) = params.domain {
            let source_domains: Vec<String> = metadata
                .get("source_domains")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            if !source_domains.iter().any(|d| d.contains(domain.as_str())) {
                continue;
            }
        }

        placements.push(PlacementAlert {
            id: id.to_string(),
            source_count,
            time_window_hours,
            token_jaccard,
            signal_ids,
            created_at,
        });
    }

    Ok(Json(placements))
}

/// GET /api/adversarial/quarantine
/// Returns items currently held in quarantine for analyst review.
/// Data sourced from observations table (type='source_quarantine') populated by AdversarialAnalysis worker.
pub async fn get_quarantine(
    store: Extension<Arc<PgStore>>,
    params: Query<AdversarialQueryParams>,
) -> Result<Json<Vec<QuarantineItem>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query(
        r#"SELECT
               id,
               value,
               created_at
           FROM observations
           WHERE observation_type = 'source_quarantine'
             AND (value->>'status' IS NULL OR value->>'status' = 'quarantined')
           ORDER BY created_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(&store.pool)
    .await
    .map_err(|e| ApiError::internal(format!("failed to query quarantine: {e}")))?;

    let mut items = Vec::new();
    for row in &rows {
        use sqlx::Row;
        let id: uuid::Uuid = row.try_get("id").unwrap_or_default();
        let value: serde_json::Value = row.try_get("value").unwrap_or_default();
        let created_at: chrono::DateTime<Utc> = row.try_get("created_at").unwrap_or_else(|_| Utc::now());

        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let source_domain = value
            .get("source_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let release_at = value
            .get("release_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| created_at + chrono::Duration::hours(24));

        // Filter by domain if requested
        if let Some(ref domain) = params.domain {
            if !source_domain.contains(domain.as_str()) {
                continue;
            }
        }

        items.push(QuarantineItem {
            id: id.to_string(),
            reason,
            source_domain,
            quarantined_at: created_at,
            release_at,
        });
    }

    Ok(Json(items))
}

/// GET /api/adversarial/source-reliability
/// Returns source reliability history for a given domain.
/// Data sourced from source_reliability_stats table populated by AdversarialAnalysis worker.
pub async fn get_source_reliability(
    store: Extension<Arc<PgStore>>,
    params: Query<AdversarialQueryParams>,
) -> Result<Json<SourceReliabilityHistory>, ApiError> {
    let domain = params.domain.as_deref().unwrap_or("aggregate");

    // Try to get stats for the specific domain
    let row = sqlx::query(
        r#"SELECT
               source_domain,
               reliability_tier,
               observation_count,
               false_positive_rate,
               last_updated,
               metadata
           FROM source_reliability_stats
           WHERE source_domain = $1
           LIMIT 1"#,
    )
    .bind(domain)
    .fetch_optional(&store.pool)
    .await
    .map_err(|e| ApiError::internal(format!("failed to query source reliability: {e}")))?;

    if let Some(row) = row {
        use sqlx::Row;
        let source_domain: String = row
            .try_get("source_domain")
            .unwrap_or_else(|_| domain.to_string());
        let tier_str: String = row
            .try_get("reliability_tier")
            .unwrap_or_else(|_| "Unknown".to_string());
        let obs_count: i32 = row.try_get("observation_count").unwrap_or(0);
        let false_pos_rate: f64 = row.try_get("false_positive_rate").unwrap_or(0.0);
        let last_updated: chrono::DateTime<Utc> = row
            .try_get("last_updated")
            .unwrap_or_else(|_| Utc::now());
        let metadata: serde_json::Value = row.try_get("metadata").unwrap_or_default();

        let tier = match tier_str.as_str() {
            "Official" => SourceReliabilityTier::Official,
            "Established" => SourceReliabilityTier::Established,
            "TradePress" => SourceReliabilityTier::TradePress,
            "Social" => SourceReliabilityTier::Social,
            "UnderReview" => SourceReliabilityTier::Unknown, // Under review = unknown pending
            _ => SourceReliabilityTier::Unknown,
        };

        let flagged_by = metadata
            .get("flagged_by")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let history_entry = format!(
            "Tier: {tier_str} | Observations: {obs_count} | False Positive Rate: {:.0}% | Flagged by: {flagged_by}",
            false_pos_rate * 100.0
        );

        Ok(Json(SourceReliabilityHistory {
            domain: source_domain,
            tier,
            promotion_candidate: tier == SourceReliabilityTier::Established || tier == SourceReliabilityTier::Official,
            history: vec![history_entry, format!("Last updated: {}", last_updated.format("%Y-%m-%d %H:%M UTC"))],
        }))
    } else {
        // No stats for this domain - return unknown
        Ok(Json(SourceReliabilityHistory {
            domain: domain.to_string(),
            tier: SourceReliabilityTier::Unknown,
            promotion_candidate: false,
            history: vec!["No reliability data available for this domain".to_string()],
        }))
    }
}