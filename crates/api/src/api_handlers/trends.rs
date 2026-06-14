//! API handlers for historical trends data.
//!
//! Provides JSON endpoints for querying materialized trend data,
//! comparing periods, and fetching entity-level trends.

use std::sync::Arc;

use axum::{extract::Query, Extension, Json};
use chrono::NaiveDate;
use serde::Deserialize;

use apex_store::postgres::trends::{
    BucketType, TrendComparison, TrendComparisonQuery, TrendDataPoint, TrendQuery,
};
use apex_store::postgres::PgStore;

use apex_api::responses::ApiError;

// ─────────────────────────────────────────────────────────────────────────────
// Query parameter types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TrendsQueryParams {
    pub metric: Option<String>,
    pub bucket: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ComparisonQueryParams {
    pub metric: Option<String>,
    pub period: Option<String>, // "month" or "year"
    pub current_start: Option<String>,
    pub previous_start: Option<String>,
    pub days: Option<i64>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EntityTrendsQueryParams {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub metric: Option<String>,
    pub bucket: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler: GET /api/trends
// ─────────────────────────────────────────────────────────────────────────────

/// Query historical trend data.
///
/// Returns a JSON array of `{date, value}` objects sorted by date ascending.
///
/// Query parameters:
/// - `metric` (required): metric name (e.g. "warnings", "insights", "observations")
/// - `bucket` (optional, default "monthly"): bucket type ("daily", "weekly", "monthly", "quarterly", "yearly")
/// - `from` (optional): start date (ISO 8601 date string)
/// - `to` (optional): end date (ISO 8601 date string)
/// - `entity_type` (optional): entity type filter ("company", "person", "region")
/// - `entity_id` (optional): entity id filter
/// - `limit` (optional, default 1000): max number of data points
pub async fn query_trends(
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<TrendsQueryParams>,
) -> Result<Json<Vec<TrendDataPoint>>, ApiError> {
    let metric = params
        .metric
        .unwrap_or_else(|| "warnings".to_string());
    let bucket = params
        .bucket
        .unwrap_or_else(|| "monthly".to_string());

    // Validate bucket type
    if BucketType::from_str(&bucket).is_none() {
        return Err(ApiError::bad_request(format!(
            "Invalid bucket type '{}'. Must be one of: {:?}",
            bucket,
            BucketType::all_types()
        )));
    }

    let from_date = params
        .from
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let to_date = params
        .to
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let query = TrendQuery {
        bucket_type: bucket,
        metric_name: metric,
        from_date,
        to_date,
        entity_type: params.entity_type,
        entity_id: params.entity_id,
        limit: params.limit,
    };

    let data = store.query_trends(&query).await.map_err(|e| {
        tracing::error!("Failed to query trends: {e}");
        ApiError::internal("Failed to query trends")
    })?;

    Ok(Json(data))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler: GET /api/trends/comparison
// ─────────────────────────────────────────────────────────────────────────────

/// Get trend comparison between two periods (month-over-month, year-over-year).
///
/// Query parameters:
/// - `metric` (required): metric name
/// - `period` (optional, default "month"): "month" or "year"
/// - `current_start` (required): start date of current period (ISO 8601)
/// - `previous_start` (required): start date of previous period (ISO 8601)
/// - `days` (optional, default 30): duration of each period in days
/// - `entity_type` (optional): entity type filter
/// - `entity_id` (optional): entity id filter
pub async fn trend_comparison(
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<ComparisonQueryParams>,
) -> Result<Json<TrendComparison>, ApiError> {
    let metric = params
        .metric
        .unwrap_or_else(|| "warnings".to_string());

    let current_start = params
        .current_start
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .ok_or_else(|| ApiError::bad_request("Missing or invalid 'current_start' parameter (YYYY-MM-DD)"))?;

    let previous_start = params
        .previous_start
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .ok_or_else(|| ApiError::bad_request("Missing or invalid 'previous_start' parameter (YYYY-MM-DD)"))?;

    let period_duration_days = params.days.unwrap_or(30).max(1).min(3650);

    let query = TrendComparisonQuery {
        metric_name: metric,
        current_period_start: current_start,
        previous_period_start: previous_start,
        period_duration_days,
        entity_type: params.entity_type,
        entity_id: params.entity_id,
    };

    let comparison = store.get_trend_comparison(&query).await.map_err(|e| {
        tracing::error!("Failed to get trend comparison: {e}");
        ApiError::internal("Failed to get trend comparison")
    })?;

    Ok(Json(comparison))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler: GET /api/trends/entities
// ─────────────────────────────────────────────────────────────────────────────

/// Get entity-level trend data.
///
/// Query parameters:
/// - `entity_type` (optional, default "company"): entity type
/// - `entity_id` (required): entity id
/// - `metric` (optional, default "observations"): metric name
/// - `bucket` (optional, default "monthly"): bucket type
/// - `from` (optional): start date
/// - `to` (optional): end date
pub async fn entity_trends(
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<EntityTrendsQueryParams>,
) -> Result<Json<Vec<TrendDataPoint>>, ApiError> {
    let entity_type = params.entity_type.unwrap_or_else(|| "company".to_string());
    let entity_id = params
        .entity_id
        .ok_or_else(|| ApiError::bad_request("Missing required 'entity_id' parameter"))?;
    let metric = params
        .metric
        .unwrap_or_else(|| "observations".to_string());
    let bucket = params
        .bucket
        .unwrap_or_else(|| "monthly".to_string());

    if BucketType::from_str(&bucket).is_none() {
        return Err(ApiError::bad_request(format!(
            "Invalid bucket type '{}'. Must be one of: {:?}",
            bucket,
            BucketType::all_types()
        )));
    }

    let from_date = params
        .from
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let to_date = params
        .to
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let query = TrendQuery {
        bucket_type: bucket,
        metric_name: metric,
        from_date,
        to_date,
        entity_type: Some(entity_type),
        entity_id: Some(entity_id),
        limit: Some(1000),
    };

    let data: Vec<TrendDataPoint> = store.query_trends(&query).await.map_err(|e| {
        tracing::error!("Failed to query entity trends: {e}");
        ApiError::internal("Failed to query entity trends")
    })?;

    Ok(Json(data))
}
