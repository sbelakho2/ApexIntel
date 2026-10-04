//! Durable insight-analysis runs (migration 104).
//!
//! Insight analysis used to run synchronously inside the API request: a client
//! disconnect discarded the work and nothing was persisted. A run is now a
//! `insight_analysis_runs` row plus a payload-carrying
//! `worker_trigger_queue` entry inserted in the same transaction, so the API
//! only enqueues and the worker owns the model call.
//!
//! Lifecycle: `queued` → `running` → `succeeded` | `failed`. At most one
//! queued/running run exists per insight (partial unique index), so repeated
//! requests deduplicate onto the in-flight run. A run abandoned by a crashed
//! process is resolved by [`PgStore::expire_stale_insight_analysis_runs`], not
//! by client polls.

use super::*;

/// A persisted insight-analysis run.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct InsightAnalysisRunRow {
    pub id: Uuid,
    pub insight_id: Uuid,
    pub status: String,
    pub requested_by: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

const RUN_COLUMNS: &str = "id, insight_id, status, requested_by, requested_at, \
     started_at, completed_at, result, error";

/// The partial unique index predicate that deduplicates in-flight runs. The
/// `ON CONFLICT` inference below must name the same predicate.
const INFLIGHT_PREDICATE: &str = "status IN ('queued', 'running')";

impl PgStore {
    /// Insert a `queued` run, deduplicating onto any queued/running run of the
    /// same insight. Returns the row and whether this call deduplicated.
    ///
    /// No trigger is written here; callers that want the worker to execute the
    /// run use [`PgStore::enqueue_insight_analysis`] so the run and its
    /// trigger are committed together.
    pub async fn create_insight_analysis_run(
        &self,
        insight_id: Uuid,
        requested_by: Option<&str>,
    ) -> Result<(InsightAnalysisRunRow, bool)> {
        let inserted = sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "INSERT INTO insight_analysis_runs (insight_id, status, requested_by) \
             VALUES ($1, 'queued', $2) \
             ON CONFLICT (insight_id) WHERE {INFLIGHT_PREDICATE} DO NOTHING \
             RETURNING {RUN_COLUMNS}"
        ))
        .bind(insight_id)
        .bind(requested_by)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = inserted {
            return Ok((row, false));
        }

        let existing = sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM insight_analysis_runs \
             WHERE insight_id = $1 AND {INFLIGHT_PREDICATE} \
             ORDER BY requested_at DESC, id DESC LIMIT 1"
        ))
        .bind(insight_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "insight analysis run insert conflicted but no in-flight run was found \
                 (insight {insight_id})"
            )
        })?;
        Ok((existing, true))
    }

    /// Enqueue an analysis run and its worker trigger atomically.
    ///
    /// Returns the run plus whether it deduplicated onto an existing
    /// queued/running run. A deduplicated call writes no trigger: the
    /// in-flight run already has one.
    pub async fn enqueue_insight_analysis(
        &self,
        insight_id: Uuid,
        requested_by: Option<&str>,
    ) -> Result<(InsightAnalysisRunRow, bool)> {
        let mut tx = self.pool.begin().await?;

        let inserted = sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "INSERT INTO insight_analysis_runs (insight_id, status, requested_by) \
             VALUES ($1, 'queued', $2) \
             ON CONFLICT (insight_id) WHERE {INFLIGHT_PREDICATE} DO NOTHING \
             RETURNING {RUN_COLUMNS}"
        ))
        .bind(insight_id)
        .bind(requested_by)
        .fetch_optional(&mut *tx)
        .await?;

        let (row, deduplicated) = match inserted {
            Some(row) => (row, false),
            None => {
                let existing = sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
                    "SELECT {RUN_COLUMNS} FROM insight_analysis_runs \
                     WHERE insight_id = $1 AND {INFLIGHT_PREDICATE} \
                     ORDER BY requested_at DESC, id DESC LIMIT 1"
                ))
                .bind(insight_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "insight analysis run insert conflicted but no in-flight run was \
                         found (insight {insight_id})"
                    )
                })?;
                (existing, true)
            }
        };

        if !deduplicated {
            let payload = serde_json::json!({
                "run_id": row.id,
                "insight_id": row.insight_id,
            });
            sqlx::query(
                "INSERT INTO worker_trigger_queue (job_kind, payload) \
                 VALUES ('insight_analysis', $1)",
            )
            .bind(&payload)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok((row, deduplicated))
    }

    /// Claim a queued run for execution. Returns the row when this caller won
    /// the `queued` → `running` transition, `None` when the run was already
    /// claimed or is terminal.
    pub async fn claim_insight_analysis_run(
        &self,
        run_id: Uuid,
    ) -> Result<Option<InsightAnalysisRunRow>> {
        Ok(sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "UPDATE insight_analysis_runs \
             SET status = 'running', started_at = NOW() \
             WHERE id = $1 AND status = 'queued' \
             RETURNING {RUN_COLUMNS}"
        ))
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Persist the result of a running run. A terminal run is never
    /// overwritten; returns false when the run was not running.
    pub async fn complete_insight_analysis_run(
        &self,
        run_id: Uuid,
        result: &serde_json::Value,
    ) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE insight_analysis_runs \
             SET status = 'succeeded', result = $2, error = NULL, completed_at = NOW() \
             WHERE id = $1 AND status = 'running'",
        )
        .bind(run_id)
        .bind(result)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated == 1)
    }

    /// Mark a queued/running run failed with an explicit reason. Succeeded
    /// runs are never overwritten.
    pub async fn fail_insight_analysis_run(&self, run_id: Uuid, error: &str) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE insight_analysis_runs \
             SET status = 'failed', error = $2, completed_at = NOW() \
             WHERE id = $1 AND status IN ('queued', 'running')",
        )
        .bind(run_id)
        .bind(error)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated == 1)
    }

    /// Fail runs abandoned by a crashed worker so the in-flight partial unique
    /// index cannot block every new run for an insight:
    ///
    /// * a `running` run whose last progress is older than `stale_secs` was
    ///   claimed and never finished;
    /// * a `queued` run older than `stale_secs` whose trigger is gone (completed
    ///   without executing, or never written) will never run.
    ///
    /// A queued run whose trigger is still pending is deliberately not expired:
    /// a worker restart may sit on the queue for longer than the threshold
    /// before polling, and the run is still legitimately waiting. Returns how
    /// many runs were failed.
    pub async fn expire_stale_insight_analysis_runs(&self, stale_secs: i64) -> Result<u64> {
        let stale_secs = stale_secs.max(60);
        let updated = sqlx::query(
            "UPDATE insight_analysis_runs AS run \
             SET status = 'failed', \
                 error = 'insight analysis run made no progress within ' || $1 || \
                         ' seconds; no result was persisted (the worker may have restarted)', \
                 completed_at = NOW() \
             WHERE (run.status = 'running' \
                    OR (run.status = 'queued' AND NOT EXISTS ( \
                        SELECT 1 FROM worker_trigger_queue AS trigger \
                        WHERE trigger.job_kind = 'insight_analysis' \
                          AND trigger.payload->>'run_id' = run.id::text \
                          AND trigger.completed_at IS NULL))) \
               AND COALESCE(run.started_at, run.requested_at) < \
                   NOW() - make_interval(secs => $1::double precision)",
        )
        .bind(stale_secs)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated)
    }

    pub async fn get_insight_analysis_run(
        &self,
        run_id: Uuid,
    ) -> Result<Option<InsightAnalysisRunRow>> {
        Ok(sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM insight_analysis_runs WHERE id = $1"
        ))
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Most recent run for an insight, whatever its status.
    pub async fn get_latest_insight_analysis_run(
        &self,
        insight_id: Uuid,
    ) -> Result<Option<InsightAnalysisRunRow>> {
        Ok(sqlx::query_as::<_, InsightAnalysisRunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM insight_analysis_runs \
             WHERE insight_id = $1 \
             ORDER BY requested_at DESC, id DESC LIMIT 1"
        ))
        .bind(insight_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_columns_cover_the_lifecycle_fields() {
        for column in [
            "insight_id",
            "status",
            "requested_by",
            "requested_at",
            "started_at",
            "completed_at",
            "result",
            "error",
        ] {
            assert!(RUN_COLUMNS.contains(column), "missing {column}");
        }
    }

    #[test]
    fn inflight_predicate_matches_the_partial_index() {
        assert!(INFLIGHT_PREDICATE.contains("queued"));
        assert!(INFLIGHT_PREDICATE.contains("running"));
    }
}
