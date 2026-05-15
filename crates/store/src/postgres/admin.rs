use super::*;

fn average_artifacts_per_person(total_persons: i64, total_artifacts: i64) -> f64 {
    if total_persons > 0 {
        total_artifacts as f64 / total_persons as f64
    } else {
        0.0
    }
}

fn trigger_row_to_result(row: Option<(Uuid, String)>) -> Option<(String, String)> {
    row.map(|(id, kind)| (id.to_string(), kind))
}

impl PgStore {
    pub async fn get_admin_crawl_status(&self) -> Result<AdminCrawlStatus> {
        let (total_fp,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM page_fingerprints")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));
        let (domains,): (i64,) =
            sqlx::query_as("SELECT COUNT(DISTINCT split_part(url, '/', 3)) FROM page_fingerprints")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));
        let latest: Option<(DateTime<Utc>,)> =
            sqlx::query_as("SELECT MAX(ts) FROM page_fingerprints")
                .fetch_optional(&self.pool)
                .await
                .ok()
                .flatten();
        let latest_ts = latest.map(|value| value.0);
        let crawl_stats = self
            .get_crawl_stats(Utc::now() - chrono::Duration::days(7))
            .await
            .unwrap_or_default();
        Ok(AdminCrawlStatus {
            total_fingerprints: total_fp,
            domains_tracked: domains,
            latest_crawl_ts: latest_ts,
            crawl_stats,
        })
    }

    pub async fn get_admin_recipe_performance(&self) -> Result<AdminRecipePerformance> {
        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));
        let (prod,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status IN ('active', 'production')")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));
        let (staging,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'staging'")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));
        let (deprecated,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'deprecated'")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));
        let recipes = self.get_recipe_stats().await.unwrap_or_default();
        Ok(AdminRecipePerformance {
            total_recipes: total,
            production_count: prod,
            staging_count: staging,
            deprecated_count: deprecated,
            recipes,
        })
    }

    pub async fn get_admin_poi_coverage(&self) -> Result<AdminPoiCoverage> {
        let (total_persons,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM persons")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));
        let (with_artifacts,): (i64,) =
            sqlx::query_as("SELECT COUNT(DISTINCT person_id) FROM poi_artifacts")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));
        let (total_artifacts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM poi_artifacts")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));
        let avg = average_artifacts_per_person(total_persons, total_artifacts);
        let poi_stats = self
            .get_poi_stats(Utc::now() - chrono::Duration::days(30))
            .await
            .unwrap_or_default();
        Ok(AdminPoiCoverage {
            total_persons,
            with_artifacts,
            avg_artifacts_per_person: avg,
            total_artifacts,
            poi_stats,
        })
    }

    /// Enqueue a manual job trigger from the API.
    pub async fn queue_job_trigger(&self, job_kind: &str) -> Result<String> {
        let existing: Option<(Uuid,)> = sqlx::query_as(
            r#"SELECT id
               FROM worker_trigger_queue
               WHERE job_kind = $1
                 AND completed_at IS NULL
                 AND (
                     claimed_at IS NOT NULL
                     OR recovered_at IS NULL
                 )
               ORDER BY claimed_at DESC NULLS LAST, requested_at ASC
               LIMIT 1"#,
        )
        .bind(job_kind)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((id,)) = existing {
            return Ok(id.to_string());
        }

        let (id,): (Uuid,) =
            sqlx::query_as("INSERT INTO worker_trigger_queue (job_kind) VALUES ($1) RETURNING id")
                .bind(job_kind)
                .fetch_one(&self.pool)
                .await?;
        Ok(id.to_string())
    }

    /// Claim the oldest unclaimed job trigger (atomic via UPDATE ... RETURNING).
    /// Returns `(trigger_id, job_kind)` or `None` if the queue is empty.
    pub async fn pop_job_trigger(&self) -> Result<Option<(String, String)>> {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            r#"WITH candidate AS (
                   SELECT queue.id
                   FROM worker_trigger_queue AS queue
                   WHERE queue.claimed_at IS NULL
                     AND queue.completed_at IS NULL
                     AND NOT EXISTS (
                         SELECT 1
                         FROM worker_trigger_queue AS active
                         WHERE active.job_kind = queue.job_kind
                           AND active.claimed_at IS NOT NULL
                           AND active.completed_at IS NULL
                     )
                     AND pg_try_advisory_xact_lock(hashtext(queue.job_kind), 0)
                   ORDER BY queue.recovered_at NULLS FIRST, queue.requested_at
                   FOR UPDATE SKIP LOCKED
                   LIMIT 1
               )
               UPDATE worker_trigger_queue AS queue
               SET claimed_at = now()
               FROM candidate
               WHERE queue.id = candidate.id
               RETURNING queue.id, queue.job_kind"#,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(trigger_row_to_result(row))
    }

    pub async fn complete_job_trigger(&self, trigger_id: &str, error: Option<&str>) -> Result<()> {
        let uid = Uuid::parse_str(trigger_id)?;
        sqlx::query(
            "UPDATE worker_trigger_queue SET completed_at = now(), error = $2 WHERE id = $1",
        )
        .bind(uid)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn timeout_stale_job_triggers(&self, timeout_secs: i64) -> Result<u64> {
        let result = sqlx::query(
            r#"UPDATE worker_trigger_queue
               SET claimed_at = NULL,
                   recovered_at = now(),
                   recovery_count = recovery_count + 1,
                   error = COALESCE(error, 'manual trigger recovered after abandoned claim')
               WHERE completed_at IS NULL
                 AND claimed_at IS NOT NULL
                 AND claimed_at < (now() - make_interval(secs => $1))"#,
        )
        .bind(timeout_secs)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn upsert_worker_job_state(&self, record: &WorkerJobStateRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO worker_job_state (
                   job_kind, last_run, last_status, last_error, last_duration_ms,
                   consecutive_failures, max_consecutive_failures, circuit_open
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (job_kind) DO UPDATE SET
                   last_run = EXCLUDED.last_run,
                   last_status = EXCLUDED.last_status,
                   last_error = EXCLUDED.last_error,
                   last_duration_ms = EXCLUDED.last_duration_ms,
                   consecutive_failures = EXCLUDED.consecutive_failures,
                   max_consecutive_failures = EXCLUDED.max_consecutive_failures,
                   circuit_open = EXCLUDED.circuit_open,
                   updated_at = now()"#,
        )
        .bind(&record.job_kind)
        .bind(record.last_run)
        .bind(&record.last_status)
        .bind(&record.last_error)
        .bind(record.last_duration_ms)
        .bind(record.consecutive_failures)
        .bind(record.max_consecutive_failures)
        .bind(record.circuit_open)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_worker_job_states(&self) -> Result<Vec<WorkerJobStateRecord>> {
        Ok(sqlx::query_as::<_, WorkerJobStateRecord>(
            r#"SELECT job_kind, last_run, last_status, last_error, last_duration_ms,
                      consecutive_failures, max_consecutive_failures, circuit_open, updated_at
               FROM worker_job_state
               ORDER BY job_kind ASC"#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn insert_worker_job_history(&self, run: &WorkerJobHistoryRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO worker_job_history (
                   run_id, job_kind, status, started_at, finished_at, duration_ms,
                   items_processed, notes
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (run_id) DO NOTHING"#,
        )
        .bind(&run.run_id)
        .bind(&run.job_kind)
        .bind(&run.status)
        .bind(run.started_at)
        .bind(run.finished_at)
        .bind(run.duration_ms)
        .bind(run.items_processed)
        .bind(&run.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{average_artifacts_per_person, trigger_row_to_result};
    use uuid::Uuid;

    #[test]
    fn test_average_artifacts_per_person_handles_zero_people() {
        assert_eq!(average_artifacts_per_person(0, 12), 0.0);
    }

    #[test]
    fn test_trigger_row_to_result_formats_uuid() {
        let id = Uuid::new_v4();

        let result = trigger_row_to_result(Some((id, "dns_posture_scan".to_string())));

        assert_eq!(
            result,
            Some((id.to_string(), "dns_posture_scan".to_string()))
        );
    }
}
