//! Historical trends aggregation — materialized rollup tables for efficient
//! monthly, quarterly, yearly, and multi-year trend queries.
//!
//! Provides upsert, query, comparison, and automated aggregation methods
//! for pre-computed metric buckets stored in `trend_rollups`.

use super::*;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Data types
// ─────────────────────────────────────────────────────────────────────────────

/// A single trend rollup row returned from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendRollupRow {
    pub id: uuid::Uuid,
    pub bucket_date: NaiveDate,
    pub bucket_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub metric_name: String,
    pub metric_value: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A simplified trend data point for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendDataPoint {
    pub date: NaiveDate,
    pub value: i64,
}

/// Trend comparison result between two periods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendComparison {
    pub current_period_label: String,
    pub previous_period_label: String,
    pub current_total: i64,
    pub previous_total: i64,
    pub absolute_change: i64,
    pub percent_change: f64,
    pub direction: String, // "up", "down", "flat"
}

/// Summary statistics for a set of trend data points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendSummary {
    pub total: i64,
    pub average: f64,
    pub min: i64,
    pub max: i64,
    pub data_points: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Bucket type helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Supported rollup bucket types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BucketType {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

impl BucketType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::Yearly => "yearly",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "quarterly" => Some(Self::Quarterly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }

    /// Compute the bucket start date for a given date.
    /// The day/month components used below are always valid calendar values
    /// (day 1, month 1..=12), so the construction cannot fail.
    #[allow(clippy::unwrap_used)]
    pub fn bucket_start(&self, date: NaiveDate) -> NaiveDate {
        match self {
            Self::Daily => date,
            Self::Weekly => {
                // ISO week: Monday as start
                let weekday = date.weekday().num_days_from_monday();
                date - chrono::Duration::days(weekday as i64)
            }
            Self::Monthly => NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap(),
            Self::Quarterly => {
                let q_start_month = ((date.month() - 1) / 3) * 3 + 1;
                NaiveDate::from_ymd_opt(date.year(), q_start_month, 1).unwrap()
            }
            Self::Yearly => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
        }
    }

    /// Return the next bucket start date after the given one.
    #[allow(clippy::unwrap_used)]
    pub fn next_bucket_start(&self, bucket_start: NaiveDate) -> NaiveDate {
        match self {
            Self::Daily => bucket_start + chrono::Duration::days(1),
            Self::Weekly => bucket_start + chrono::Duration::days(7),
            Self::Monthly => {
                let mut m = bucket_start.month();
                let mut y = bucket_start.year();
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
                NaiveDate::from_ymd_opt(y, m, 1).unwrap()
            }
            Self::Quarterly => {
                let mut m = bucket_start.month();
                let mut y = bucket_start.year();
                m += 3;
                if m > 12 {
                    m -= 12;
                    y += 1;
                }
                NaiveDate::from_ymd_opt(y, m, 1).unwrap()
            }
            Self::Yearly => NaiveDate::from_ymd_opt(bucket_start.year() + 1, 1, 1).unwrap(),
        }
    }

    /// Return all supported bucket types as string slices.
    pub fn all_types() -> &'static [&'static str] {
        &["daily", "weekly", "monthly", "quarterly", "yearly"]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Query parameter types
// ─────────────────────────────────────────────────────────────────────────────

/// Parameters for querying trend data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendQuery {
    pub bucket_type: String,
    pub metric_name: String,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub limit: Option<i64>,
}

/// Parameters for a trend comparison query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendComparisonQuery {
    pub metric_name: String,
    pub current_period_start: NaiveDate,
    pub previous_period_start: NaiveDate,
    pub period_duration_days: i64,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Implementation
// ─────────────────────────────────────────────────────────────────────────────

impl PgStore {
    /// Upsert a single metric value into the trend_rollups table.
    ///
    /// Inserts a new row or updates the existing one if the combination of
    /// (bucket_date, bucket_type, entity_type, entity_id, metric_name) already exists.
    #[tracing::instrument(skip(self))]
    pub async fn upsert_trend_rollup(
        &self,
        bucket_date: NaiveDate,
        bucket_type: &str,
        entity_type: Option<&str>,
        entity_id: Option<&str>,
        metric_name: &str,
        metric_value: i64,
    ) -> anyhow::Result<()> {
        let entity_type = normalize_optional_text(entity_type);
        let entity_id = normalize_optional_text(entity_id);

        sqlx::query(
            r#"INSERT INTO trend_rollups (bucket_date, bucket_type, entity_type, entity_id, metric_name, metric_value)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (bucket_date, bucket_type, entity_type, entity_id, metric_name)
               DO UPDATE SET metric_value = EXCLUDED.metric_value, updated_at = NOW()"#,
        )
        .bind(bucket_date)
        .bind(bucket_type)
        .bind(entity_type)
        .bind(entity_id)
        .bind(metric_name)
        .bind(metric_value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Compute and store all metrics for a given time bucket.
    ///
    /// This aggregates global metrics (warnings, insights, observations,
    /// companies, persons, recipes) for the specified date range and
    /// upserts the results into `trend_rollups`.
    #[tracing::instrument(skip(self))]
    pub async fn aggregate_trend_bucket(
        &self,
        bucket_type: &str,
        bucket_date: NaiveDate,
    ) -> anyhow::Result<AggregationResult> {
        let bucket_end = match BucketType::from_str(bucket_type) {
            Some(bt) => bt.next_bucket_start(bucket_date),
            None => {
                // Default to 1-day bucket
                bucket_date + chrono::Duration::days(1)
            }
        };

        let since = bucket_date
            .and_hms_opt(0, 0, 0)
            .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let until = bucket_end
            .and_hms_opt(23, 59, 59)
            .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        let mut metrics_computed = 0u64;

        // ── Global metrics ────────────────────────────────────────────────

        // Warnings count
        let warnings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM warnings WHERE deleted_at IS NULL AND created_at >= $1 AND created_at <= $2",
        )
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(bucket_date, bucket_type, None, None, "warnings", warnings)
            .await?;
        metrics_computed += 1;

        // New warnings count (created in this period)
        let new_warnings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM warnings WHERE deleted_at IS NULL AND created_at >= $1 AND created_at <= $2",
        )
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "new_warnings",
            new_warnings,
        )
        .await?;
        metrics_computed += 1;

        // Insights count
        let insights: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM insights WHERE created_at >= $1 AND created_at <= $2",
        )
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(bucket_date, bucket_type, None, None, "insights", insights)
            .await?;
        metrics_computed += 1;

        // Observations count
        let observations: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM observations WHERE ts_utc >= $1 AND ts_utc <= $2",
        )
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "observations",
            observations,
        )
        .await?;
        metrics_computed += 1;

        // Companies tracked
        let companies: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM companies")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "companies_tracked",
            companies,
        )
        .await?;
        metrics_computed += 1;

        // Persons tracked
        let persons: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM persons")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "persons_tracked",
            persons,
        )
        .await?;
        metrics_computed += 1;

        // Active recipes
        let active_recipes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM recipes WHERE status IN ('active', 'production')",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "active_recipes",
            active_recipes,
        )
        .await?;
        metrics_computed += 1;

        // Unacknowledged warnings
        let unacked_warnings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM warnings WHERE deleted_at IS NULL AND acknowledged = false",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        self.upsert_trend_rollup(
            bucket_date,
            bucket_type,
            None,
            None,
            "unacknowledged_warnings",
            unacked_warnings,
        )
        .await?;
        metrics_computed += 1;

        // ── Entity-level metrics ──────────────────────────────────────────

        // Per-company observation counts
        let company_obs_rows = sqlx::query_as::<_, (uuid::Uuid, i64)>(
            r#"SELECT entity_id, COUNT(*)::BIGINT AS cnt
               FROM observations
               WHERE entity_id IS NOT NULL AND ts_utc >= $1 AND ts_utc <= $2
               GROUP BY entity_id"#,
        )
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for (entity_id, cnt) in &company_obs_rows {
            self.upsert_trend_rollup(
                bucket_date,
                bucket_type,
                Some("company"),
                Some(&entity_id.to_string()),
                "observations",
                *cnt,
            )
            .await?;
            metrics_computed += 1;
        }

        // Per-company warning counts
        let company_warn_rows = sqlx::query_as::<_, (uuid::Uuid, i64)>(
            r#"SELECT unnest(entity_ids) AS entity_id, COUNT(*)::BIGINT AS cnt
               FROM warnings
               WHERE entity_ids IS NOT NULL
                 AND deleted_at IS NULL
                 AND array_length(entity_ids, 1) > 0
                 AND COALESCE(created_at, ts_utc) >= $1
                 AND COALESCE(created_at, ts_utc) <= $2
               GROUP BY entity_id"#,
        )
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for (entity_id, cnt) in &company_warn_rows {
            self.upsert_trend_rollup(
                bucket_date,
                bucket_type,
                Some("company"),
                Some(&entity_id.to_string()),
                "warnings",
                *cnt,
            )
            .await?;
            metrics_computed += 1;
        }

        tracing::info!(
            bucket_type = bucket_type,
            bucket_date = %bucket_date,
            metrics_computed = metrics_computed,
            "trend_bucket_aggregated"
        );

        Ok(AggregationResult {
            bucket_date,
            bucket_type: bucket_type.to_string(),
            metrics_computed,
        })
    }

    /// Query historical trend data with optional filtering.
    ///
    /// Returns a list of trend data points sorted by date ascending.
    #[tracing::instrument(skip(self))]
    pub async fn query_trends(&self, query: &TrendQuery) -> anyhow::Result<Vec<TrendDataPoint>> {
        let limit = query.limit.unwrap_or(1000).clamp(1, 10000);
        let entity_type = normalize_optional_text(query.entity_type.as_deref());
        let entity_id = normalize_optional_text(query.entity_id.as_deref());

        let rows = sqlx::query_as::<_, (NaiveDate, i64)>(
            r#"SELECT bucket_date, metric_value
               FROM trend_rollups
               WHERE bucket_type = $1
                 AND metric_name = $2
                 AND ($3::DATE IS NULL OR bucket_date >= $3)
                 AND ($4::DATE IS NULL OR bucket_date <= $4)
                 AND ($5::VARCHAR IS NULL OR entity_type IS NOT DISTINCT FROM $5)
                 AND ($6::VARCHAR IS NULL OR entity_id IS NOT DISTINCT FROM $6)
               ORDER BY bucket_date ASC
               LIMIT $7"#,
        )
        .bind(&query.bucket_type)
        .bind(&query.metric_name)
        .bind(query.from_date)
        .bind(query.to_date)
        .bind(entity_type)
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(date, value)| TrendDataPoint { date, value })
            .collect())
    }

    /// Compute month-over-month or year-over-year comparison for a metric.
    ///
    /// `current_period_start` and `previous_period_start` define the start dates
    /// of the two periods being compared. `period_duration_days` defines the length
    /// of each period.
    #[tracing::instrument(skip(self))]
    pub async fn get_trend_comparison(
        &self,
        query: &TrendComparisonQuery,
    ) -> anyhow::Result<TrendComparison> {
        let entity_type = normalize_optional_text(query.entity_type.as_deref());
        let entity_id = normalize_optional_text(query.entity_id.as_deref());

        let current_end =
            query.current_period_start + chrono::Duration::days(query.period_duration_days);
        let previous_end =
            query.previous_period_start + chrono::Duration::days(query.period_duration_days);

        // Fetch current period data
        let current_rows = sqlx::query_as::<_, (i64,)>(
            r#"SELECT COALESCE(SUM(metric_value), 0)::BIGINT
               FROM trend_rollups
               WHERE metric_name = $1
                 AND bucket_date >= $2
                 AND bucket_date < $3
                 AND ($4::VARCHAR IS NULL OR entity_type IS NOT DISTINCT FROM $4)
                 AND ($5::VARCHAR IS NULL OR entity_id IS NOT DISTINCT FROM $5)"#,
        )
        .bind(&query.metric_name)
        .bind(query.current_period_start)
        .bind(current_end)
        .bind(entity_type.as_deref())
        .bind(entity_id.as_deref())
        .fetch_one(&self.pool)
        .await?;

        // Fetch previous period data
        let previous_rows = sqlx::query_as::<_, (i64,)>(
            r#"SELECT COALESCE(SUM(metric_value), 0)::BIGINT
               FROM trend_rollups
               WHERE metric_name = $1
                 AND bucket_date >= $2
                 AND bucket_date < $3
                 AND ($4::VARCHAR IS NULL OR entity_type IS NOT DISTINCT FROM $4)
                 AND ($5::VARCHAR IS NULL OR entity_id IS NOT DISTINCT FROM $5)"#,
        )
        .bind(&query.metric_name)
        .bind(query.previous_period_start)
        .bind(previous_end)
        .bind(entity_type.as_deref())
        .bind(entity_id.as_deref())
        .fetch_one(&self.pool)
        .await?;

        let current_total = current_rows.0;
        let previous_total = previous_rows.0;

        let (absolute_change, percent_change, direction) = if previous_total == 0 {
            if current_total > 0 {
                (current_total, 100.0, "up".to_string())
            } else {
                (0, 0.0, "flat".to_string())
            }
        } else {
            let change = current_total - previous_total;
            let pct = (change as f64 / previous_total as f64) * 100.0;
            let dir = if change > 0 {
                "up"
            } else if change < 0 {
                "down"
            } else {
                "flat"
            };
            (change, pct, dir.to_string())
        };

        let current_label = format!("{}", query.current_period_start);
        let previous_label = format!("{}", query.previous_period_start);

        Ok(TrendComparison {
            current_period_label: current_label,
            previous_period_label: previous_label,
            current_total,
            previous_total,
            absolute_change,
            percent_change,
            direction,
        })
    }

    /// Run a full trend aggregation job — aggregates all pending buckets
    /// for daily → weekly → monthly → quarterly → yearly rollups.
    ///
    /// This method is designed to be called by the scheduled worker job.
    /// It only computes buckets that haven't been aggregated yet.
    #[tracing::instrument(skip(self))]
    pub async fn run_trend_aggregation(&self) -> anyhow::Result<AggregationSummary> {
        let now = chrono::Utc::now().date_naive();
        let mut total_buckets = 0u64;
        let mut total_metrics = 0u64;
        let mut errors: Vec<String> = Vec::new();

        // Aggregate daily buckets for the last 90 days
        for day_offset in (0..90).rev() {
            let bucket_date = now - chrono::Duration::days(day_offset);
            match self.aggregate_trend_bucket("daily", bucket_date).await {
                Ok(result) => {
                    total_buckets += 1;
                    total_metrics += result.metrics_computed;
                }
                Err(e) => {
                    errors.push(format!("daily {}: {}", bucket_date, e));
                    tracing::error!(bucket_date = %bucket_date, error = %e, "daily aggregation failed");
                }
            }
        }

        // Aggregate weekly buckets for the last 52 weeks
        let weekly_type = BucketType::Weekly;
        for week_offset in (0..52).rev() {
            let bucket_date =
                weekly_type.bucket_start(now - chrono::Duration::days(week_offset * 7));
            // Check if already aggregated
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM trend_rollups WHERE bucket_type = 'weekly' AND bucket_date = $1 LIMIT 1",
            )
            .bind(bucket_date)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if existing > 0 {
                continue;
            }
            match self.aggregate_trend_bucket("weekly", bucket_date).await {
                Ok(result) => {
                    total_buckets += 1;
                    total_metrics += result.metrics_computed;
                }
                Err(e) => {
                    errors.push(format!("weekly {}: {}", bucket_date, e));
                    tracing::error!(bucket_date = %bucket_date, error = %e, "weekly aggregation failed");
                }
            }
        }

        // Aggregate monthly buckets for the last 24 months
        let monthly_type = BucketType::Monthly;
        for month_offset in (0..24).rev() {
            let dt = now - chrono::Duration::days(month_offset * 30);
            let bucket_date = monthly_type.bucket_start(dt);
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM trend_rollups WHERE bucket_type = 'monthly' AND bucket_date = $1 LIMIT 1",
            )
            .bind(bucket_date)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if existing > 0 {
                continue;
            }
            match self.aggregate_trend_bucket("monthly", bucket_date).await {
                Ok(result) => {
                    total_buckets += 1;
                    total_metrics += result.metrics_computed;
                }
                Err(e) => {
                    errors.push(format!("monthly {}: {}", bucket_date, e));
                    tracing::error!(bucket_date = %bucket_date, error = %e, "monthly aggregation failed");
                }
            }
        }

        // Aggregate quarterly buckets for the last 8 quarters
        let quarterly_type = BucketType::Quarterly;
        for q_offset in (0..8).rev() {
            let dt = now - chrono::Duration::days(q_offset * 90);
            let bucket_date = quarterly_type.bucket_start(dt);
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM trend_rollups WHERE bucket_type = 'quarterly' AND bucket_date = $1 LIMIT 1",
            )
            .bind(bucket_date)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if existing > 0 {
                continue;
            }
            match self.aggregate_trend_bucket("quarterly", bucket_date).await {
                Ok(result) => {
                    total_buckets += 1;
                    total_metrics += result.metrics_computed;
                }
                Err(e) => {
                    errors.push(format!("quarterly {}: {}", bucket_date, e));
                    tracing::error!(bucket_date = %bucket_date, error = %e, "quarterly aggregation failed");
                }
            }
        }

        // Aggregate yearly buckets for the last 5 years
        let yearly_type = BucketType::Yearly;
        for year_offset in (0..5).rev() {
            let dt = now - chrono::Duration::days(year_offset * 365);
            let bucket_date = yearly_type.bucket_start(dt);
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT FROM trend_rollups WHERE bucket_type = 'yearly' AND bucket_date = $1 LIMIT 1",
            )
            .bind(bucket_date)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if existing > 0 {
                continue;
            }
            match self.aggregate_trend_bucket("yearly", bucket_date).await {
                Ok(result) => {
                    total_buckets += 1;
                    total_metrics += result.metrics_computed;
                }
                Err(e) => {
                    errors.push(format!("yearly {}: {}", bucket_date, e));
                    tracing::error!(bucket_date = %bucket_date, error = %e, "yearly aggregation failed");
                }
            }
        }

        tracing::info!(
            total_buckets = total_buckets,
            total_metrics = total_metrics,
            errors = errors.len(),
            "trend_aggregation_complete"
        );

        Ok(AggregationSummary {
            total_buckets,
            total_metrics,
            errors,
        })
    }

    /// Compute summary statistics for a set of trend data points.
    pub async fn get_trend_summary(&self, query: &TrendQuery) -> anyhow::Result<TrendSummary> {
        let data_points = self.query_trends(query).await?;

        if data_points.is_empty() {
            return Ok(TrendSummary {
                total: 0,
                average: 0.0,
                min: 0,
                max: 0,
                data_points: 0,
            });
        }

        let total: i64 = data_points.iter().map(|dp| dp.value).sum();
        let min = data_points.iter().map(|dp| dp.value).min().unwrap_or(0);
        let max = data_points.iter().map(|dp| dp.value).max().unwrap_or(0);
        let average = total as f64 / data_points.len() as f64;
        let data_points_count = data_points.len();

        Ok(TrendSummary {
            total,
            average,
            min,
            max,
            data_points: data_points_count,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Result types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of a single trend bucket aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationResult {
    pub bucket_date: NaiveDate,
    pub bucket_type: String,
    pub metrics_computed: u64,
}

/// Summary of a full trend aggregation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationSummary {
    pub total_buckets: u64,
    pub total_metrics: u64,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_bucket_type_from_str() {
        assert_eq!(BucketType::from_str("daily"), Some(BucketType::Daily));
        assert_eq!(BucketType::from_str("weekly"), Some(BucketType::Weekly));
        assert_eq!(BucketType::from_str("monthly"), Some(BucketType::Monthly));
        assert_eq!(
            BucketType::from_str("quarterly"),
            Some(BucketType::Quarterly)
        );
        assert_eq!(BucketType::from_str("yearly"), Some(BucketType::Yearly));
        assert_eq!(BucketType::from_str("unknown"), None);
    }

    #[test]
    fn test_bucket_type_as_str() {
        assert_eq!(BucketType::Daily.as_str(), "daily");
        assert_eq!(BucketType::Weekly.as_str(), "weekly");
        assert_eq!(BucketType::Monthly.as_str(), "monthly");
        assert_eq!(BucketType::Quarterly.as_str(), "quarterly");
        assert_eq!(BucketType::Yearly.as_str(), "yearly");
    }

    #[test]
    fn test_bucket_start_daily() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert_eq!(BucketType::Daily.bucket_start(date), date);
    }

    #[test]
    fn test_bucket_start_weekly() {
        // 2026-06-15 is a Monday
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert_eq!(
            BucketType::Weekly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
        );

        // 2026-06-17 is a Wednesday — should still start from Monday
        let wed = NaiveDate::from_ymd_opt(2026, 6, 17).unwrap();
        assert_eq!(
            BucketType::Weekly.bucket_start(wed),
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
        );
    }

    #[test]
    fn test_bucket_start_monthly() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert_eq!(
            BucketType::Monthly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
    }

    #[test]
    fn test_bucket_start_quarterly() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 15).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        );
    }

    #[test]
    fn test_bucket_start_yearly() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert_eq!(
            BucketType::Yearly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
    }

    #[test]
    fn test_next_bucket_start_daily() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let next = BucketType::Daily.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 6, 16).unwrap());
    }

    #[test]
    fn test_next_bucket_start_weekly() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(); // Monday
        let next = BucketType::Weekly.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 6, 22).unwrap());
    }

    #[test]
    fn test_next_bucket_start_monthly() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let next = BucketType::Monthly.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());

        // December → January of next year
        let dec = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let next_dec = BucketType::Monthly.next_bucket_start(dec);
        assert_eq!(next_dec, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn test_next_bucket_start_quarterly() {
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(); // Q2
        let next = BucketType::Quarterly.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()); // Q3
    }

    #[test]
    fn test_next_bucket_start_yearly() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let next = BucketType::Yearly.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn test_bucket_type_all_types() {
        let all = BucketType::all_types();
        assert_eq!(all.len(), 5);
        assert!(all.contains(&"daily"));
        assert!(all.contains(&"weekly"));
        assert!(all.contains(&"monthly"));
        assert!(all.contains(&"quarterly"));
        assert!(all.contains(&"yearly"));
    }

    #[test]
    fn test_bucket_type_roundtrip() {
        for expected in &[
            BucketType::Daily,
            BucketType::Weekly,
            BucketType::Monthly,
            BucketType::Quarterly,
            BucketType::Yearly,
        ] {
            let s = expected.as_str();
            let back = BucketType::from_str(s);
            assert_eq!(back, Some(*expected), "roundtrip failed for {:?}", expected);
        }
    }

    #[test]
    fn test_bucket_start_quarterly_q1() {
        // January → Q1 (Jan 1)
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
        // February → Q1 (Jan 1)
        let feb = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(feb),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
        // March → Q1 (Jan 1)
        let mar = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(mar),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
    }

    #[test]
    fn test_bucket_start_quarterly_q4() {
        // October → Q4 (Oct 1)
        let date = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(date),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );
        // December → Q4 (Oct 1)
        let dec = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        assert_eq!(
            BucketType::Quarterly.bucket_start(dec),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );
    }

    #[test]
    fn test_next_bucket_start_quarterly_q1() {
        let q1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let next = BucketType::Quarterly.next_bucket_start(q1);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap());
    }

    #[test]
    fn test_next_bucket_start_quarterly_q4_to_next_year() {
        let q4 = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let next = BucketType::Quarterly.next_bucket_start(q4);
        assert_eq!(next, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn test_next_bucket_start_yearly_edge() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(); // leap year
        let next = BucketType::Yearly.next_bucket_start(date);
        assert_eq!(next, NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
    }

    #[test]
    fn test_weekly_bucket_start_sunday() {
        // 2026-06-14 is a Sunday — the week should start on the previous Monday (2026-06-08)
        let sun = NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
        let start = BucketType::Weekly.bucket_start(sun);
        // Monday of that week is 2026-06-08
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());
    }

    #[test]
    fn test_weekly_bucket_start_saturday() {
        // 2026-06-20 is a Saturday — Monday is 2026-06-15
        let sat = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        let start = BucketType::Weekly.bucket_start(sat);
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
    }

    #[test]
    fn test_trend_data_point_serialization() {
        let point = TrendDataPoint {
            date: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            value: 42,
        };
        let json = serde_json::to_string(&point).unwrap();
        assert!(json.contains("\"date\""));
        assert!(json.contains("\"value\""));
        assert!(json.contains("2026-06-15"));
        assert!(json.contains("42"));

        // Deserialize back
        let back: TrendDataPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back.date, point.date);
        assert_eq!(back.value, point.value);
    }

    #[test]
    fn test_trend_comparison_serialization() {
        let comp = TrendComparison {
            current_period_label: "Current".to_string(),
            previous_period_label: "Previous".to_string(),
            current_total: 100,
            previous_total: 80,
            percent_change: 25.0,
            absolute_change: 20,
            direction: "up".to_string(),
        };
        let json = serde_json::to_string(&comp).unwrap();
        assert!(json.contains("\"direction\":\"up\""));
        assert!(json.contains("\"percent_change\":25.0"));

        let back: TrendComparison = serde_json::from_str(&json).unwrap();
        assert_eq!(back.current_total, 100);
        assert_eq!(back.previous_total, 80);
        assert_eq!(back.direction, "up");
    }

    #[test]
    fn test_trend_summary_default() {
        // TrendSummary is created via constructor/struct literal, not Default
        let summary = TrendSummary {
            total: 0,
            average: 0.0,
            min: 0,
            max: 0,
            data_points: 0,
        };
        assert_eq!(summary.total, 0);
        assert_eq!(summary.average, 0.0);
        assert_eq!(summary.data_points, 0);
    }

    #[test]
    fn test_trend_rollup_row_serialization() {
        let row = TrendRollupRow {
            id: uuid::Uuid::nil(),
            bucket_date: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            bucket_type: "monthly".to_string(),
            entity_type: Some("company".to_string()),
            entity_id: Some("uuid-123".to_string()),
            metric_name: "warnings".to_string(),
            metric_value: 42,
            created_at: chrono::DateTime::UNIX_EPOCH,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&row).unwrap();
        assert!(json.contains("\"bucket_type\":\"monthly\""));
        assert!(json.contains("\"metric_name\":\"warnings\""));
        assert!(json.contains("\"metric_value\":42"));

        let back: TrendRollupRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bucket_type, "monthly");
        assert_eq!(back.metric_name, "warnings");
        assert_eq!(back.metric_value, 42);
    }

    #[test]
    fn test_trend_query_defaults() {
        let query = TrendQuery {
            bucket_type: "monthly".to_string(),
            metric_name: "warnings".to_string(),
            from_date: None,
            to_date: None,
            entity_type: None,
            entity_id: None,
            limit: Some(100),
        };
        assert_eq!(query.bucket_type, "monthly");
        assert_eq!(query.metric_name, "warnings");
        assert!(query.from_date.is_none());
        assert_eq!(query.limit, Some(100));
    }

    #[test]
    fn test_aggregation_result_and_summary() {
        let result = AggregationResult {
            bucket_type: "daily".to_string(),
            bucket_date: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            metrics_computed: 8,
        };
        assert_eq!(result.bucket_type, "daily");
        assert_eq!(result.metrics_computed, 8);

        let summary = AggregationSummary {
            total_buckets: 5,
            total_metrics: 40,
            errors: vec![],
        };
        assert_eq!(summary.total_buckets, 5);
        assert_eq!(summary.total_metrics, 40);
        assert!(summary.errors.is_empty());
    }

    #[test]
    fn test_trend_comparison_directions() {
        let up = TrendComparison {
            current_period_label: "Current".to_string(),
            previous_period_label: "Previous".to_string(),
            current_total: 100,
            previous_total: 50,
            percent_change: 100.0,
            absolute_change: 50,
            direction: "up".to_string(),
        };
        assert_eq!(up.direction, "up");
        assert!(up.percent_change > 0.0);

        let down = TrendComparison {
            current_period_label: "Current".to_string(),
            previous_period_label: "Previous".to_string(),
            current_total: 30,
            previous_total: 90,
            percent_change: -66.67,
            absolute_change: -60,
            direction: "down".to_string(),
        };
        assert_eq!(down.direction, "down");
        assert!(down.percent_change < 0.0);

        let flat = TrendComparison {
            current_period_label: "Current".to_string(),
            previous_period_label: "Previous".to_string(),
            current_total: 50,
            previous_total: 50,
            percent_change: 0.0,
            absolute_change: 0,
            direction: "flat".to_string(),
        };
        assert_eq!(flat.direction, "flat");
        assert_eq!(flat.percent_change, 0.0);
    }

    #[test]
    fn bucket_start_rollovers_and_idempotence() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();

        // Monthly: any day maps to the first of that month, including year end.
        assert_eq!(
            BucketType::Monthly.bucket_start(d(2026, 12, 31)),
            d(2026, 12, 1)
        );

        // Quarterly boundaries (Jan/Apr/Jul/Oct).
        assert_eq!(
            BucketType::Quarterly.bucket_start(d(2026, 1, 1)),
            d(2026, 1, 1)
        );
        assert_eq!(
            BucketType::Quarterly.bucket_start(d(2026, 3, 31)),
            d(2026, 1, 1)
        );
        assert_eq!(
            BucketType::Quarterly.bucket_start(d(2026, 4, 1)),
            d(2026, 4, 1)
        );
        assert_eq!(
            BucketType::Quarterly.bucket_start(d(2026, 12, 31)),
            d(2026, 10, 1)
        );

        // Yearly.
        assert_eq!(
            BucketType::Yearly.bucket_start(d(2026, 7, 4)),
            d(2026, 1, 1)
        );

        // Idempotence: bucketing a bucket start is a no-op.
        for bucket in [
            BucketType::Daily,
            BucketType::Weekly,
            BucketType::Monthly,
            BucketType::Quarterly,
            BucketType::Yearly,
        ] {
            let start = bucket.bucket_start(d(2026, 8, 17));
            assert_eq!(
                bucket.bucket_start(start),
                start,
                "not idempotent for {bucket:?}"
            );
        }
    }

    #[test]
    fn next_bucket_start_crosses_year_boundaries() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(
            BucketType::Monthly.next_bucket_start(d(2026, 12, 1)),
            d(2027, 1, 1)
        );
        assert_eq!(
            BucketType::Quarterly.next_bucket_start(d(2026, 10, 1)),
            d(2027, 1, 1)
        );
        assert_eq!(
            BucketType::Yearly.next_bucket_start(d(2026, 1, 1)),
            d(2027, 1, 1)
        );
        assert_eq!(
            BucketType::Daily.next_bucket_start(d(2026, 2, 28)),
            d(2026, 3, 1)
        );
        assert_eq!(
            BucketType::Weekly.next_bucket_start(d(2026, 12, 28)),
            d(2027, 1, 4)
        );
    }

    #[test]
    fn weekly_bucket_start_is_a_monday_not_after_the_date() {
        use chrono::{Datelike, Weekday};
        let date = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap(); // Thursday
        let start = BucketType::Weekly.bucket_start(date);
        assert_eq!(start.weekday(), Weekday::Mon);
        assert!(start <= date);
        assert_eq!((date - start).num_days(), 3);
    }
}
