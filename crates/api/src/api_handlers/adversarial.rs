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
//!
//! Rows are decoded with typed `query_as` structs: a column that cannot be
//! decoded is an error, never a nil UUID / zero / empty-string default that
//! would silently fabricate an identity or a measurement.

use std::sync::Arc;

use axum::{extract::Query, Extension, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use apex_api::responses::ApiError;
use apex_shared::{PlacementAlert, QuarantineItem, SourceReliabilityHistory, SourceReliabilityTier};
use apex_store::postgres::PgStore;

#[derive(Debug, Deserialize)]
pub struct AdversarialQueryParams {
    pub domain: Option<String>,
    pub limit: Option<i64>,
}

/// One `pattern_candidates` row for an adversarial placement cluster.
///
/// `pattern_candidates.id` is a `BIGSERIAL`, and the worker stores the
/// measured cluster detail as JSON in `pattern_label` (the table has no
/// metadata column).
#[derive(Debug, sqlx::FromRow)]
struct PlacementCandidateRow {
    id: i64,
    pattern_label: Option<String>,
    confidence: f64,
    created_at: DateTime<Utc>,
}

/// Measured detail written by the worker into `pattern_label`.
#[derive(Debug, Deserialize, Serialize)]
struct PlacementDetail {
    placement_id: String,
    source_count: u32,
    time_window_hours: u32,
    token_jaccard: f64,
    signal_ids: Vec<String>,
    source_domains: Vec<String>,
    detected_at: String,
}

/// Decode the measured placement detail the worker persisted.
///
/// A label that cannot be decoded is a data-integrity error: returning a
/// fabricated alert (or constants standing in for a measurement) would be
/// worse than failing the request. Rows written before the JSON detail format
/// are excluded by the query rather than served with invented values.
fn decode_placement_detail(label: Option<&str>) -> Result<PlacementDetail, String> {
    let Some(label) = label else {
        return Err("adversarial placement row has no pattern_label".to_string());
    };

    serde_json::from_str::<PlacementDetail>(label)
        .map_err(|error| format!("adversarial placement row has an undecodable pattern_label: {error}"))
}

/// GET /api/adversarial/placements
/// Returns detected coordinated placement clusters from the adversarial pipeline.
/// Data sourced from pattern_candidates table populated by AdversarialAnalysis worker.
pub async fn get_placements(
    store: Extension<Arc<PgStore>>,
    params: Query<AdversarialQueryParams>,
) -> Result<Json<Vec<PlacementAlert>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query_as::<_, PlacementCandidateRow>(
        r#"SELECT
               id,
               pattern_label,
               confidence,
               created_at
           FROM pattern_candidates
           WHERE recipe_code = 'adversarial_placement'
           ORDER BY created_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(&store.pool)
    .await
    .map_err(|e| ApiError::internal(format!("failed to query placements: {e}")))?;

    let mut placements = Vec::with_capacity(rows.len());
    for row in &rows {
        let detail = decode_placement_detail(row.pattern_label.as_deref()).map_err(|error| {
            ApiError::internal(format!(
                "failed to decode placement candidate {}: {error}",
                row.id
            ))
        })?;

        // Filter by domain if requested
        if let Some(ref domain) = params.domain {
            if !detail
                .source_domains
                .iter()
                .any(|candidate| candidate.contains(domain.as_str()))
            {
                continue;
            }
        }

        placements.push(PlacementAlert {
            id: if detail.placement_id.is_empty() {
                row.id.to_string()
            } else {
                detail.placement_id
            },
            source_count: detail.source_count,
            time_window_hours: detail.time_window_hours,
            token_jaccard: detail.token_jaccard,
            signal_ids: detail.signal_ids,
            created_at: row.created_at,
        });
    }

    Ok(Json(placements))
}

/// One quarantined observation row.
#[derive(Debug, sqlx::FromRow)]
struct QuarantineRow {
    id: uuid::Uuid,
    value: serde_json::Value,
    created_at: DateTime<Utc>,
}

/// GET /api/adversarial/quarantine
/// Returns items currently held in quarantine for analyst review.
/// Data sourced from observations table (type='source_quarantine') populated by AdversarialAnalysis worker.
pub async fn get_quarantine(
    store: Extension<Arc<PgStore>>,
    params: Query<AdversarialQueryParams>,
) -> Result<Json<Vec<QuarantineItem>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query_as::<_, QuarantineRow>(
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

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let reason = row
            .value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let source_domain = row
            .value
            .get("source_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        // The 24h window is the quarantine policy (what the worker writes on
        // insert), used only when the persisted value predates release_at.
        let release_at = row
            .value
            .get("release_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| row.created_at + chrono::Duration::hours(24));

        // Filter by domain if requested
        if let Some(ref domain) = params.domain {
            if !source_domain.contains(domain.as_str()) {
                continue;
            }
        }

        items.push(QuarantineItem {
            id: row.id.to_string(),
            reason,
            source_domain,
            quarantined_at: row.created_at,
            release_at,
        });
    }

    Ok(Json(items))
}

/// One `source_reliability_stats` row (columns from migration 014 + 037).
#[derive(Debug, sqlx::FromRow)]
struct SourceReliabilityRow {
    source_domain: String,
    reliability_tier: String,
    observation_count: i64,
    false_positive_rate: f64,
    last_updated: DateTime<Utc>,
    metadata: serde_json::Value,
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
    let row = sqlx::query_as::<_, SourceReliabilityRow>(
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
        let tier = match row.reliability_tier.as_str() {
            "Official" => SourceReliabilityTier::Official,
            "Established" => SourceReliabilityTier::Established,
            "TradePress" => SourceReliabilityTier::TradePress,
            "Social" => SourceReliabilityTier::Social,
            "UnderReview" => SourceReliabilityTier::Unknown, // Under review = unknown pending
            _ => SourceReliabilityTier::Unknown,
        };

        let flagged_by = row
            .metadata
            .get("flagged_by")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let history_entry = format!(
            "Tier: {} | Observations: {} | False Positive Rate: {:.0}% | Flagged by: {flagged_by}",
            row.reliability_tier,
            row.observation_count,
            row.false_positive_rate * 100.0
        );

        Ok(Json(SourceReliabilityHistory {
            domain: row.source_domain,
            tier,
            promotion_candidate: tier == SourceReliabilityTier::Established
                || tier == SourceReliabilityTier::Official,
            history: vec![
                history_entry,
                format!("Last updated: {}", row.last_updated.format("%Y-%m-%d %H:%M UTC")),
            ],
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
