//! Trend aggregation scheduled job.
//!
//! Runs the `run_trend_aggregation()` method on the store to compute and
//! store materialized rollup metrics for daily, weekly, monthly, quarterly,
//! and yearly time buckets.
//!
//! Designed to be invoked by the scheduler at 01:00 UTC daily.

use std::sync::Arc;

use crate::scheduler::{JobKind, JobRun};
use apex_store::postgres::PgStore;

/// Run the trend aggregation job.
///
/// Computes and stores materialized rollup metrics for all pending time
/// buckets (daily, weekly, monthly, quarterly, yearly). Only aggregates
/// buckets that have not yet been computed.
///
/// Returns a `JobRun` with status indicating success, failure, or skip.
#[tracing::instrument(skip(kind, store), fields(job = %kind.as_str()))]
pub async fn run_trend_aggregation(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());

    let result = store.run_trend_aggregation().await;

    match result {
        Ok(summary) => {
            tracing::info!(
                total_buckets = summary.total_buckets,
                total_metrics = summary.total_metrics,
                errors = summary.errors.len(),
                "trend_aggregation_succeeded"
            );

            let notes = if summary.errors.is_empty() {
                format!(
                    "Aggregated {} buckets, {} metrics computed",
                    summary.total_buckets, summary.total_metrics
                )
            } else {
                format!(
                    "Aggregated {} buckets, {} metrics computed, {} errors: {}",
                    summary.total_buckets,
                    summary.total_metrics,
                    summary.errors.len(),
                    summary.errors.join("; ")
                )
            };

            run.succeed(summary.total_metrics as u64, &notes);
        }
        Err(e) => {
            tracing::error!(error = %e, "trend_aggregation_failed");
            run.fail(&format!("Trend aggregation failed: {}", e));
        }
    }

    run
}
