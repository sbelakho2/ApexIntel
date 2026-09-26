//! Persistence for the measurable-learning promotion gate (P0 #38, audit #6).
//!
//! Backs the tables created by `migrations/054_learning_eval_metrics.sql` and
//! `migrations/063_learning_eval_examples.sql`: frozen evaluation sets with
//! immutable examples, candidate evaluation runs, and their versioned metrics.
//! The statistical gate itself lives in `apex_learning::evaluation`; this
//! module only stores, verifies and retrieves.
//!
//! Two invariants are enforced here in addition to the database triggers:
//!
//! 1. A run may only be persisted when the frozen set actually stores
//!    `example_count` examples whose recomputed digest equals
//!    `examples_digest`; the run records that digest.
//! 2. `is_training_truth` is explicit. A positive confirmation is never
//!    automatically truth, and no dismissal/noise or workflow-convenience
//!    metric may claim it.

use super::*;

/// One frozen evaluation example to persist.
///
/// `content_hash` is computed by the database from the canonical payload text;
/// callers cannot supply or forge it.
#[derive(Debug, Clone)]
pub struct LearningEvalExampleInput {
    pub example_key: String,
    pub input_payload: Value,
    pub expected_payload: Value,
    pub provenance: Value,
}

impl LearningEvalExampleInput {
    pub fn new(
        example_key: impl Into<String>,
        input_payload: Value,
        expected_payload: Value,
    ) -> Self {
        Self {
            example_key: example_key.into(),
            input_payload,
            expected_payload,
            provenance: Value::Object(serde_json::Map::new()),
        }
    }

    pub fn with_provenance(mut self, provenance: Value) -> Self {
        self.provenance = provenance;
        self
    }
}

/// One metric row to persist with an evaluation run.
#[derive(Debug, Clone)]
pub struct LearningEvalMetricInput {
    pub metric: String,
    pub value: f64,
    pub sample_size: i32,
    pub signal_class: String,
    pub is_critical: bool,
    /// Explicit opt-in that this metric is training truth. Never inferred from
    /// `signal_class`; only `positive_confirmation` may set it.
    pub is_training_truth: bool,
    pub metadata: Value,
}

impl LearningEvalMetricInput {
    /// Convenience constructor mirroring the default database columns.
    ///
    /// `is_training_truth` defaults to `false`: a positive confirmation is not
    /// automatically training truth.
    pub fn new(metric: impl Into<String>, value: f64, sample_size: i32) -> Self {
        Self {
            metric: metric.into(),
            value,
            sample_size,
            signal_class: "positive_confirmation".to_string(),
            is_critical: false,
            is_training_truth: false,
            metadata: Value::Object(serde_json::Map::new()),
        }
    }

    pub fn with_signal_class(mut self, signal_class: impl Into<String>) -> Self {
        self.signal_class = signal_class.into();
        self
    }

    pub fn with_training_truth(mut self, is_training_truth: bool) -> Self {
        self.is_training_truth = is_training_truth;
        self
    }

    pub fn with_critical(mut self, is_critical: bool) -> Self {
        self.is_critical = is_critical;
        self
    }

    /// Reject truth claims on a signal class that can never back training
    /// truth. The database CHECK is the backstop; this gives callers a clear
    /// error before the write.
    pub fn validate(&self) -> Result<()> {
        if self.is_training_truth && self.signal_class != "positive_confirmation" {
            anyhow::bail!(
                "learning_eval metric '{}' cannot be training truth for signal class '{}'",
                self.metric,
                self.signal_class
            );
        }
        Ok(())
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
    pub eval_set_digest: String,
    pub candidate_kind: String,
    pub candidate_ref: String,
    pub candidate_version: Option<String>,
    pub candidate_artifact_hash: Option<String>,
    pub baseline_run_id: Option<Uuid>,
    pub baseline_artifact_hash: Option<String>,
    pub baseline_artifact_version: Option<String>,
    pub status: String,
    pub decision: Option<String>,
    pub decision_reason: Option<String>,
    pub metrics_version: i32,
    pub sample_size: i32,
    pub metrics_snapshot: Value,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// Everything needed to persist one evaluation run.
#[derive(Debug, Clone)]
pub struct LearningEvalRunInput<'a> {
    pub eval_set_id: Uuid,
    pub candidate_kind: &'a str,
    pub candidate_ref: &'a str,
    pub candidate_version: Option<&'a str>,
    /// Content hash of the exact candidate artefact that was evaluated.
    pub candidate_artifact_hash: &'a str,
    pub baseline_run_id: Option<Uuid>,
    /// Content hash of the baseline artefact, when there is one.
    pub baseline_artifact_hash: Option<&'a str>,
    pub baseline_artifact_version: Option<&'a str>,
    pub metrics_version: i32,
    pub metrics_snapshot: &'a Value,
    pub metrics: &'a [LearningEvalMetricInput],
}

impl PgStore {
    /// Create (or fetch) a frozen evaluation set version *without* examples.
    ///
    /// `learning_eval_sets` rows are immutable in the database: callers must
    /// create a new version instead of mutating an existing one. A repeat call
    /// for an existing `(name, version)` therefore returns the existing id
    /// rather than updating the row (which the freeze trigger would reject).
    ///
    /// Prefer [`PgStore::create_frozen_learning_eval_set`] for new sets: a
    /// set created here has no examples, so evaluation runs against it are
    /// refused until examples are inserted and finalized.
    pub async fn upsert_learning_eval_set(
        &self,
        name: &str,
        version: i32,
        description: Option<&str>,
        example_count: i32,
        metadata: &Value,
    ) -> Result<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            r#"WITH inserted AS (
                   INSERT INTO learning_eval_sets (name, version, description, example_count, metadata)
                   VALUES ($1, $2, $3, $4, $5)
                   ON CONFLICT (name, version) DO NOTHING
                   RETURNING id
               )
               SELECT id FROM inserted
               UNION ALL
               SELECT id FROM learning_eval_sets WHERE name = $1 AND version = $2
               LIMIT 1"#,
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

    /// Create a frozen evaluation set together with its immutable examples.
    ///
    /// The set declares `examples.len()` and is finalized in the same
    /// transaction: `examples_digest` is the sha256 over the sorted content
    /// hashes of the stored examples. `(name, version)` collisions are an
    /// error: changing a set's examples requires a new version.
    pub async fn create_frozen_learning_eval_set(
        &self,
        name: &str,
        version: i32,
        description: Option<&str>,
        metadata: &Value,
        examples: &[LearningEvalExampleInput],
    ) -> Result<Uuid> {
        if examples.is_empty() {
            anyhow::bail!("a frozen evaluation set requires at least one example");
        }
        let example_count = i32::try_from(examples.len())
            .map_err(|_| anyhow::anyhow!("too many evaluation examples"))?;
        let mut tx = self.pool.begin().await?;
        let set_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO learning_eval_sets
                 (name, version, description, example_count, metadata)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id"#,
        )
        .bind(name.trim())
        .bind(version)
        .bind(description)
        .bind(example_count)
        .bind(metadata)
        .fetch_one(&mut *tx)
        .await?;

        Self::insert_learning_eval_examples_tx(&mut tx, set_id, examples).await?;

        sqlx::query_scalar::<_, String>("SELECT learning_eval_set_finalize_examples($1)")
            .bind(set_id)
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(set_id)
    }

    /// Append the immutable examples of a set, computing their content hashes.
    ///
    /// Insertion is the only mutation the freeze guard permits; call
    /// [`PgStore::finalize_learning_eval_set`] afterwards so the set digest
    /// covers them, otherwise evaluation runs are refused.
    pub async fn insert_learning_eval_examples(
        &self,
        eval_set_id: Uuid,
        examples: &[LearningEvalExampleInput],
    ) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let inserted =
            Self::insert_learning_eval_examples_tx(&mut tx, eval_set_id, examples).await?;
        tx.commit().await?;
        Ok(inserted)
    }

    async fn insert_learning_eval_examples_tx(
        tx: &mut Transaction<'_, Postgres>,
        eval_set_id: Uuid,
        examples: &[LearningEvalExampleInput],
    ) -> Result<u64> {
        let mut inserted = 0u64;
        for example in examples {
            sqlx::query(
                r#"INSERT INTO learning_eval_examples
                     (eval_set_id, example_key, input_payload, expected_payload, provenance)
                   VALUES ($1, $2, $3, $4, $5)"#,
            )
            .bind(eval_set_id)
            .bind(example.example_key.trim())
            .bind(&example.input_payload)
            .bind(&example.expected_payload)
            .bind(&example.provenance)
            .execute(&mut **tx)
            .await?;
            inserted += 1;
        }
        Ok(inserted)
    }

    /// Finalize (or refresh) the digest of a frozen set over its stored
    /// examples. Fails when the stored count does not match `example_count`.
    pub async fn finalize_learning_eval_set(&self, eval_set_id: Uuid) -> Result<String> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT learning_eval_set_finalize_examples($1)")
                .bind(eval_set_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Persist an evaluation run together with its metrics, atomically.
    ///
    /// Refuses execution unless the frozen set is complete and consistent:
    /// the stored example count must equal `example_count`, the digest
    /// recomputed from the stored content hashes must equal `examples_digest`,
    /// and the run records that same digest plus the candidate/baseline
    /// artifact hashes. Every metric must pass
    /// [`LearningEvalMetricInput::validate`] (explicit truth only).
    pub async fn insert_learning_eval_run(&self, run: LearningEvalRunInput<'_>) -> Result<Uuid> {
        if run.candidate_artifact_hash.trim().is_empty() {
            anyhow::bail!("candidate artifact hash is required for an evaluation run");
        }
        for metric in run.metrics {
            metric.validate()?;
        }

        let sample_size = run
            .metrics
            .iter()
            .map(|metric| metric.sample_size)
            .max()
            .unwrap_or(0);

        let mut tx = self.pool.begin().await?;

        let verification = sqlx::query(
            r#"SELECT s.example_count,
                      s.examples_digest,
                      (SELECT COUNT(*) FROM learning_eval_examples e
                        WHERE e.eval_set_id = s.id) AS stored_examples,
                      learning_eval_examples_digest(s.id) AS recomputed_digest
               FROM learning_eval_sets s
               WHERE s.id = $1"#,
        )
        .bind(run.eval_set_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(verification) = verification else {
            anyhow::bail!(
                "evaluation refused: learning_eval_set {} does not exist",
                run.eval_set_id
            );
        };
        let example_count: i32 = verification.try_get("example_count")?;
        let examples_digest: String = verification.try_get("examples_digest")?;
        let stored_examples: i64 = verification.try_get("stored_examples")?;
        let recomputed_digest: String = verification.try_get("recomputed_digest")?;

        if example_count <= 0 {
            anyhow::bail!(
                "evaluation refused: frozen set {} declares no examples",
                run.eval_set_id
            );
        }
        if stored_examples != i64::from(example_count) {
            anyhow::bail!(
                "evaluation refused: frozen set {} stores {} examples but declares {}",
                run.eval_set_id,
                stored_examples,
                example_count
            );
        }
        if recomputed_digest != examples_digest {
            anyhow::bail!(
                "evaluation refused: frozen set {} digest mismatch (recorded {}, recomputed {})",
                run.eval_set_id,
                examples_digest,
                recomputed_digest
            );
        }

        let run_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO learning_eval_runs
                 (eval_set_id, eval_set_digest, candidate_kind, candidate_ref,
                  candidate_version, candidate_artifact_hash, baseline_run_id,
                  baseline_artifact_hash, baseline_artifact_version,
                  metrics_version, sample_size, metrics_snapshot)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id"#,
        )
        .bind(run.eval_set_id)
        .bind(&examples_digest)
        .bind(run.candidate_kind)
        .bind(run.candidate_ref)
        .bind(run.candidate_version)
        .bind(run.candidate_artifact_hash)
        .bind(run.baseline_run_id)
        .bind(run.baseline_artifact_hash)
        .bind(run.baseline_artifact_version)
        .bind(run.metrics_version)
        .bind(sample_size)
        .bind(run.metrics_snapshot)
        .fetch_one(&mut *tx)
        .await?;

        for metric in run.metrics {
            sqlx::query(
                r#"INSERT INTO learning_eval_metrics
                     (run_id, metric, value, sample_size, signal_class, is_critical,
                      is_training_truth, metadata)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                   ON CONFLICT (run_id, metric, signal_class) DO UPDATE SET
                     value = EXCLUDED.value,
                     sample_size = EXCLUDED.sample_size,
                     is_critical = EXCLUDED.is_critical,
                     is_training_truth = EXCLUDED.is_training_truth,
                     metadata = EXCLUDED.metadata"#,
            )
            .bind(run_id)
            .bind(&metric.metric)
            .bind(metric.value)
            .bind(metric.sample_size)
            .bind(&metric.signal_class)
            .bind(metric.is_critical)
            .bind(metric.is_training_truth)
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
            r#"SELECT id, eval_set_id, eval_set_digest, candidate_kind, candidate_ref,
                      candidate_version, candidate_artifact_hash, baseline_run_id,
                      baseline_artifact_hash, baseline_artifact_version, status,
                      decision, decision_reason, metrics_version, sample_size,
                      metrics_snapshot, created_at, decided_at
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_input_defaults_to_not_training_truth() {
        let metric = LearningEvalMetricInput::new("precision", 0.7, 100);
        assert_eq!(metric.signal_class, "positive_confirmation");
        assert!(
            !metric.is_training_truth,
            "positive confirmation is not automatically training truth"
        );
        metric
            .validate()
            .expect("confirmation without opt-in is valid");
    }

    #[test]
    fn metric_input_rejects_truth_on_dismissal_or_noise() {
        let noise = LearningEvalMetricInput::new("source_yield", 0.4, 100)
            .with_signal_class("dismissal_noise")
            .with_training_truth(true);
        assert!(noise.validate().is_err());

        let convenience = LearningEvalMetricInput::new("time_to_action_hours", 12.0, 100)
            .with_signal_class("workflow_convenience")
            .with_training_truth(true);
        assert!(convenience.validate().is_err());

        let confirmed =
            LearningEvalMetricInput::new("precision", 0.7, 100).with_training_truth(true);
        confirmed
            .validate()
            .expect("explicit truth opt-in on a confirmation is valid");
    }
}
