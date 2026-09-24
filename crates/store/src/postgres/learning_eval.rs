//! Persistence for the measurable-learning promotion gate (P0 #38).
//!
//! Backs the tables created by `migrations/051_learning_eval_metrics.sql`:
//! frozen evaluation sets, candidate evaluation runs, and their versioned
//! metrics. The statistical gate itself lives in
//! `apex_learning::evaluation`; this module only stores and retrieves.

use super::*;

/// One metric row to persist with an evaluation run.
#[derive(Debug, Clone)]
pub struct LearningEvalMetricInput {
    pub metric: String,
    pub value: f64,
    pub sample_size: i32,
    pub signal_class: String,
    pub is_critical: bool,
    pub metadata: Value,
}

impl LearningEvalMetricInput {
    /// Convenience constructor mirroring the default database columns.
    pub fn new(metric: impl Into<String>, value: f64, sample_size: i32) -> Self {
        Self {
            metric: metric.into(),
            value,
            sample_size,
            signal_class: "positive_confirmation".to_string(),
            is_critical: false,
            metadata: Value::Object(serde_json::Map::new()),
        }
    }
}

/// A persisted metric row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LearningEvalMetricRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub metric: String,
    pub value: f64,
    pub sample_size: i32,
    pub signal_class: String,
    pub is_critical: bool,
    pub is_training_truth: bool,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

/// A persisted evaluation run summary.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LearningEvalRunRow {
    pub id: Uuid,
    pub eval_set_id: Uuid,
    pub candidate_kind: String,
    pub candidate_ref: String,
    pub candidate_version: Option<String>,
    pub baseline_run_id: Option<Uuid>,
    pub status: String,
    pub decision: Option<String>,
    pub decision_reason: Option<String>,
    pub metrics_version: i32,
    pub sample_size: i32,
    pub metrics_snapshot: Value,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

impl PgStore {
    /// Create (or fetch) a frozen evaluation set version.
    ///
    /// `learning_eval_sets` rows are immutable in the database: callers must
    /// create a new version instead of mutating an existing one.
    pub async fn upsert_learning_eval_set(
        &self,
        name: &str,
        version: i32,
        description: Option<&str>,
        example_count: i32,
        metadata: &Value,
    ) -> Result<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO learning_eval_sets (name, version, description, example_count, metadata)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (name, version) DO UPDATE SET name = EXCLUDED.name
               RETURNING id"#,
        )
        .bind(name.trim())
        .bind(version)
        .bind(description)
        .bind(example_count)
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Persist an evaluation run together with its metrics, atomically.
    pub async fn insert_learning_eval_run(
        &self,
        eval_set_id: Uuid,
        candidate_kind: &str,
        candidate_ref: &str,
        candidate_version: Option<&str>,
        baseline_run_id: Option<Uuid>,
        metrics_version: i32,
        metrics_snapshot: &Value,
        metrics: &[LearningEvalMetricInput],
    ) -> Result<Uuid> {
        let sample_size = metrics
            .iter()
            .map(|metric| metric.sample_size)
            .max()
            .unwrap_or(0);
        let mut tx = self.pool.begin().await?;
        let run_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO learning_eval_runs
                 (eval_set_id, candidate_kind, candidate_ref, candidate_version,
                  baseline_run_id, metrics_version, sample_size, metrics_snapshot)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id"#,
        )
        .bind(eval_set_id)
        .bind(candidate_kind)
        .bind(candidate_ref)
        .bind(candidate_version)
        .bind(baseline_run_id)
        .bind(metrics_version)
        .bind(sample_size)
        .bind(metrics_snapshot)
        .fetch_one(&mut *tx)
        .await?;

        for metric in metrics {
            sqlx::query(
                r#"INSERT INTO learning_eval_metrics
                     (run_id, metric, value, sample_size, signal_class, is_critical,
                      is_training_truth, metadata)
                   VALUES ($1, $2, $3, $4, $5, $6,
                           ($5 = 'positive_confirmation'), $7)
                   ON CONFLICT (run_id, metric, signal_class) DO UPDATE SET
                     value = EXCLUDED.value,
                     sample_size = EXCLUDED.sample_size,
                     is_critical = EXCLUDED.is_critical,
                     metadata = EXCLUDED.metadata"#,
            )
            .bind(run_id)
            .bind(&metric.metric)
            .bind(metric.value)
            .bind(metric.sample_size)
            .bind(&metric.signal_class)
            .bind(metric.is_critical)
            .bind(&metric.metadata)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(run_id)
    }

    /// Record the promotion decision for an evaluation run.
    pub async fn decide_learning_eval_run(
        &self,
        run_id: Uuid,
        decision: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let status = match decision {
            "promote" => "promoted",
            _ => "rejected",
        };
        sqlx::query(
            r#"UPDATE learning_eval_runs
               SET status = $2, decision = $3, decision_reason = $4, decided_at = now()
               WHERE id = $1"#,
        )
        .bind(run_id)
        .bind(status)
        .bind(decision)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load the metrics recorded for one evaluation run, oldest first.
    pub async fn list_learning_eval_metrics(
        &self,
        run_id: Uuid,
    ) -> Result<Vec<LearningEvalMetricRow>> {
        Ok(sqlx::query_as::<_, LearningEvalMetricRow>(
            r#"SELECT id, run_id, metric, value, sample_size, signal_class, is_critical,
                      is_training_truth, metadata, created_at
               FROM learning_eval_metrics
               WHERE run_id = $1
               ORDER BY metric, signal_class"#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Load the most recent evaluation run for a candidate on a frozen set.
    pub async fn latest_learning_eval_run(
        &self,
        eval_set_id: Uuid,
        candidate_kind: &str,
        candidate_ref: &str,
    ) -> Result<Option<LearningEvalRunRow>> {
        Ok(sqlx::query_as::<_, LearningEvalRunRow>(
            r#"SELECT id, eval_set_id, candidate_kind, candidate_ref, candidate_version,
                      baseline_run_id, status, decision, decision_reason, metrics_version,
                      sample_size, metrics_snapshot, created_at, decided_at
               FROM learning_eval_runs
               WHERE eval_set_id = $1 AND candidate_kind = $2 AND candidate_ref = $3
               ORDER BY created_at DESC, id DESC
               LIMIT 1"#,
        )
        .bind(eval_set_id)
        .bind(candidate_kind)
        .bind(candidate_ref)
        .fetch_optional(&self.pool)
        .await?)
    }
}
