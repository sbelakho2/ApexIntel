use super::*;

/// One persisted analytical-quality review (migration 109).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AnalyticalQualityRow {
    pub id: Uuid,
    pub insight_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub recipe_code: Option<String>,
    pub model_id: Option<String>,
    pub prompt_version: Option<String>,
    pub depth_index: f64,
    pub depth_tier: String,
    pub depth_components: serde_json::Value,
    pub factuality_score: f64,
    pub warrant_overall: f64,
    pub warrant_weakest: f64,
    pub source_independence: f64,
    pub evidence_cap: f64,
    pub stated_confidence: f64,
    pub final_confidence: f64,
    pub hard_violations: i64,
    pub soft_flags: serde_json::Value,
    pub verdict: String,
    pub reasons: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Insert payload for [`PgStore::insert_analytical_quality_score`].
#[derive(Debug, Clone)]
pub struct NewAnalyticalQualityScore {
    pub insight_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub recipe_code: Option<String>,
    pub model_id: Option<String>,
    pub prompt_version: Option<String>,
    pub depth_index: f64,
    pub depth_tier: String,
    pub depth_components: serde_json::Value,
    pub factuality_score: f64,
    pub warrant_overall: f64,
    pub warrant_weakest: f64,
    pub source_independence: f64,
    pub evidence_cap: f64,
    pub stated_confidence: f64,
    pub final_confidence: f64,
    pub hard_violations: i64,
    pub soft_flags: serde_json::Value,
    pub verdict: String,
    pub reasons: Vec<String>,
}

/// Weekly aggregate of the analytical-quality ledger.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AnalyticalQualityAggregate {
    pub samples: i64,
    pub mean_depth: Option<f64>,
    pub median_depth: Option<f64>,
    pub mean_factuality: Option<f64>,
    pub mean_warrant: Option<f64>,
    pub published: i64,
    pub revised: i64,
    pub rejected: i64,
    pub hard_violations: i64,
}

/// One prediction awaiting (or past) resolution.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct InsightPredictionRow {
    pub id: Uuid,
    pub insight_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub recipe_code: Option<String>,
    pub statement: String,
    pub probability: f64,
    pub resolve_by: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_outcome: Option<bool>,
    pub brier: Option<f64>,
    pub expired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Insert payload for [`PgStore::insert_insight_prediction`].
#[derive(Debug, Clone)]
pub struct NewInsightPrediction {
    pub insight_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub recipe_code: Option<String>,
    pub statement: String,
    pub probability: f64,
    pub resolve_by: DateTime<Utc>,
}

/// One registered model (eval-gated onboarding for model swaps).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct LlmModelRegistryRow {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub context_window: Option<i64>,
    pub status: String,
    pub eval_scores: serde_json::Value,
    pub calibration: Option<serde_json::Value>,
    pub activated_at: Option<DateTime<Utc>>,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PgStore {
    /// Persist one analytical-quality review.
    pub async fn insert_analytical_quality_score(
        &self,
        score: &NewAnalyticalQualityScore,
    ) -> Result<Uuid> {
        let id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO analytical_quality_scores (
                   insight_id, entity_id, recipe_code, model_id, prompt_version,
                   depth_index, depth_tier, depth_components,
                   factuality_score, warrant_overall, warrant_weakest,
                   source_independence, evidence_cap, stated_confidence,
                   final_confidence, hard_violations, soft_flags, verdict, reasons)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
               RETURNING id"#,
        )
        .bind(score.insight_id)
        .bind(score.entity_id)
        .bind(score.recipe_code.as_deref())
        .bind(score.model_id.as_deref())
        .bind(score.prompt_version.as_deref())
        .bind(score.depth_index)
        .bind(&score.depth_tier)
        .bind(&score.depth_components)
        .bind(score.factuality_score)
        .bind(score.warrant_overall)
        .bind(score.warrant_weakest)
        .bind(score.source_independence)
        .bind(score.evidence_cap)
        .bind(score.stated_confidence)
        .bind(score.final_confidence)
        .bind(score.hard_violations)
        .bind(&score.soft_flags)
        .bind(&score.verdict)
        .bind(&score.reasons)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Aggregate the ledger over a window — the measured "is it getting
    /// better?" readout for depth, truth and warrant.
    pub async fn aggregate_analytical_quality(
        &self,
        since: DateTime<Utc>,
    ) -> Result<AnalyticalQualityAggregate> {
        Ok(sqlx::query_as::<_, AnalyticalQualityAggregate>(
            r#"SELECT
                 COUNT(*)::BIGINT AS samples,
                 AVG(depth_index) AS mean_depth,
                 percentile_cont(0.5) WITHIN GROUP (ORDER BY depth_index) AS median_depth,
                 AVG(factuality_score) AS mean_factuality,
                 AVG(warrant_overall) AS mean_warrant,
                 COUNT(*) FILTER (WHERE verdict = 'publish')::BIGINT AS published,
                 COUNT(*) FILTER (WHERE verdict = 'revise')::BIGINT AS revised,
                 COUNT(*) FILTER (WHERE verdict = 'reject')::BIGINT AS rejected,
                 COALESCE(SUM(hard_violations), 0)::BIGINT AS hard_violations
               FROM analytical_quality_scores
               WHERE created_at >= $1"#,
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Recent per-product reviews (admin/trend views).
    pub async fn list_recent_analytical_quality(
        &self,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AnalyticalQualityRow>> {
        Ok(sqlx::query_as::<_, AnalyticalQualityRow>(
            r#"SELECT id, insight_id, entity_id, recipe_code, model_id, prompt_version,
                      depth_index, depth_tier, depth_components, factuality_score,
                      warrant_overall, warrant_weakest, source_independence, evidence_cap,
                      stated_confidence, final_confidence, hard_violations, soft_flags,
                      verdict, reasons, created_at
               FROM analytical_quality_scores
               WHERE created_at >= $1
               ORDER BY created_at DESC
               LIMIT $2"#,
        )
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Mean measured quality for one model — the comparison basis when
    /// swapping models.
    pub async fn model_quality_summary(
        &self,
        model_id: &str,
    ) -> Result<AnalyticalQualityAggregate> {
        Ok(sqlx::query_as::<_, AnalyticalQualityAggregate>(
            r#"SELECT
                 COUNT(*)::BIGINT AS samples,
                 AVG(depth_index) AS mean_depth,
                 percentile_cont(0.5) WITHIN GROUP (ORDER BY depth_index) AS median_depth,
                 AVG(factuality_score) AS mean_factuality,
                 AVG(warrant_overall) AS mean_warrant,
                 COUNT(*) FILTER (WHERE verdict = 'publish')::BIGINT AS published,
                 COUNT(*) FILTER (WHERE verdict = 'revise')::BIGINT AS revised,
                 COUNT(*) FILTER (WHERE verdict = 'reject')::BIGINT AS rejected,
                 COALESCE(SUM(hard_violations), 0)::BIGINT AS hard_violations
               FROM analytical_quality_scores
               WHERE model_id = $1"#,
        )
        .bind(model_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Register a prediction for the calibration ledger.
    pub async fn insert_insight_prediction(
        &self,
        prediction: &NewInsightPrediction,
    ) -> Result<Uuid> {
        let id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO insight_predictions
                   (insight_id, entity_id, recipe_code, statement, probability, resolve_by)
               VALUES ($1,$2,$3,$4,$5,$6)
               RETURNING id"#,
        )
        .bind(prediction.insight_id)
        .bind(prediction.entity_id)
        .bind(prediction.recipe_code.as_deref())
        .bind(&prediction.statement)
        .bind(prediction.probability.clamp(0.0, 1.0))
        .bind(prediction.resolve_by)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Predictions whose horizon has passed and that are still unresolved.
    pub async fn list_due_insight_predictions(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<InsightPredictionRow>> {
        Ok(sqlx::query_as::<_, InsightPredictionRow>(
            r#"SELECT id, insight_id, entity_id, recipe_code, statement, probability,
                      resolve_by, resolved_at, resolved_outcome, brier, expired_at, created_at
               FROM insight_predictions
               WHERE resolve_by <= $1
                 AND resolved_outcome IS NULL
                 AND expired_at IS NULL
               ORDER BY resolve_by ASC
               LIMIT $2"#,
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Resolve a prediction with its observed outcome (Brier computed here so
    /// every resolution is scored identically).
    pub async fn resolve_insight_prediction(
        &self,
        id: Uuid,
        outcome: bool,
        resolved_at: DateTime<Utc>,
    ) -> Result<bool> {
        let probability: Option<f64> =
            sqlx::query_scalar("SELECT probability FROM insight_predictions WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(probability) = probability else {
            return Ok(false);
        };
        let actual = if outcome { 1.0 } else { 0.0 };
        let brier = (probability - actual).powi(2);
        let result = sqlx::query(
            r#"UPDATE insight_predictions
               SET resolved_at = $2, resolved_outcome = $3, brier = $4
               WHERE id = $1 AND resolved_outcome IS NULL"#,
        )
        .bind(id)
        .bind(resolved_at)
        .bind(outcome)
        .bind(brier)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Mark a past-horizon prediction unresolvable (excluded from calibration).
    pub async fn expire_insight_prediction(&self, id: Uuid, at: DateTime<Utc>) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE insight_predictions
               SET expired_at = $2
               WHERE id = $1 AND resolved_outcome IS NULL AND expired_at IS NULL"#,
        )
        .bind(id)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Resolved calibration pairs (stated, outcome) for the given model scope.
    pub async fn list_calibration_pairs(
        &self,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<(f64, bool)>> {
        let rows = sqlx::query_as::<_, (f64, bool)>(
            r#"SELECT probability, resolved_outcome
               FROM insight_predictions
               WHERE resolved_outcome IS NOT NULL
                 AND resolved_at >= $1
               ORDER BY resolved_at DESC
               LIMIT $2"#,
        )
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Best Brier among models with at least `min_samples` resolved
    /// predictions — used in onboarding reports.
    pub async fn model_brier_summary(&self) -> Result<Vec<(String, i64, f64)>> {
        let rows = sqlx::query_as::<_, (String, i64, f64)>(
            r#"SELECT COALESCE(s.model_id, 'unattributed') AS model_id,
                      COUNT(*)::BIGINT,
                      AVG(p.brier)
               FROM insight_predictions p
               JOIN insights i ON i.id = p.insight_id
               LEFT JOIN analytical_quality_scores s ON s.insight_id = i.id
               WHERE p.resolved_outcome IS NOT NULL
               GROUP BY 1
               ORDER BY 3 ASC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Upsert the weekly quality snapshot (idempotent per period).
    pub async fn upsert_analytical_quality_snapshot(
        &self,
        period_start: chrono::NaiveDate,
        sample_size: i64,
        snapshot: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO analytical_quality_snapshots (period_start, sample_size, snapshot)
               VALUES ($1,$2,$3)
               ON CONFLICT (period_start)
               DO UPDATE SET sample_size = EXCLUDED.sample_size,
                             snapshot = EXCLUDED.snapshot,
                             created_at = now()"#,
        )
        .bind(period_start)
        .bind(sample_size)
        .bind(snapshot)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Most recent snapshot strictly before `period_start` (the regression
    /// baseline).
    pub async fn previous_analytical_quality_snapshot(
        &self,
        period_start: chrono::NaiveDate,
    ) -> Result<Option<(chrono::NaiveDate, serde_json::Value)>> {
        Ok(sqlx::query_as::<_, (chrono::NaiveDate, serde_json::Value)>(
            r#"SELECT period_start, snapshot
               FROM analytical_quality_snapshots
               WHERE period_start < $1
               ORDER BY period_start DESC
               LIMIT 1"#,
        )
        .bind(period_start)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Register or refresh a model in the registry (onboarding writes the
    /// candidate; activation flips status).
    pub async fn upsert_llm_model_registry(
        &self,
        id: &str,
        provider: &str,
        display_name: &str,
        context_window: Option<i64>,
        status: &str,
        eval_scores: &serde_json::Value,
        calibration: Option<&serde_json::Value>,
        notes: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO llm_model_registry
                   (id, provider, display_name, context_window, status, eval_scores,
                    calibration, notes)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
               ON CONFLICT (id)
               DO UPDATE SET provider = EXCLUDED.provider,
                             display_name = EXCLUDED.display_name,
                             context_window = EXCLUDED.context_window,
                             status = EXCLUDED.status,
                             eval_scores = EXCLUDED.eval_scores,
                             calibration = EXCLUDED.calibration,
                             notes = EXCLUDED.notes,
                             updated_at = now()"#,
        )
        .bind(id)
        .bind(provider)
        .bind(display_name)
        .bind(context_window)
        .bind(status)
        .bind(eval_scores)
        .bind(calibration)
        .bind(notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Promote a candidate to active and mark every other active model as
    /// deprecated (one active model at a time).
    pub async fn activate_llm_model(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"UPDATE llm_model_registry
               SET status = 'deprecated', deprecated_at = now(), updated_at = now()
               WHERE status = 'active' AND id <> $1"#,
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        let result = sqlx::query(
            r#"UPDATE llm_model_registry
               SET status = 'active', activated_at = now(), updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// List registered models, newest activity first.
    pub async fn list_llm_models(&self) -> Result<Vec<LlmModelRegistryRow>> {
        Ok(sqlx::query_as::<_, LlmModelRegistryRow>(
            r#"SELECT id, provider, display_name, context_window, status, eval_scores,
                      calibration, activated_at, deprecated_at, notes, created_at, updated_at
               FROM llm_model_registry
               ORDER BY updated_at DESC"#,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

impl PgStore {
    /// Analyst-verified outcomes for an entity+recipe within a window:
    /// `(true_positive, false_positive)` counts. Used to resolve predictions
    /// against hard analyst labels — never against speculative inference.
    pub async fn reviewed_warning_outcomes_for_entity(
        &self,
        entity_id: Uuid,
        recipe_code: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<(i64, i64)> {
        let row = sqlx::query_as::<_, (i64, i64)>(
            r#"SELECT
                 COUNT(*) FILTER (WHERE review_outcome = 'true_positive')::BIGINT,
                 COUNT(*) FILTER (WHERE review_outcome = 'false_positive')::BIGINT
               FROM warnings
               WHERE entity_id = $1
                 AND recipe_id = $2
                 AND reviewed_at >= $3
                 AND reviewed_at <= $4
                 AND review_outcome IN ('true_positive', 'false_positive')"#,
        )
        .bind(entity_id)
        .bind(recipe_code)
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Store the fitted calibration curve for a model (model-portability:
    /// each model carries its own measured calibration).
    pub async fn update_model_calibration(
        &self,
        model_id: &str,
        calibration: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE llm_model_registry
               SET calibration = $2, updated_at = now()
               WHERE id = $1"#,
        )
        .bind(model_id)
        .bind(calibration)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch the stored calibration curve JSON for a model.
    pub async fn get_model_calibration(&self, model_id: &str) -> Result<Option<serde_json::Value>> {
        Ok(sqlx::query_scalar::<_, Option<serde_json::Value>>(
            "SELECT calibration FROM llm_model_registry WHERE id = $1",
        )
        .bind(model_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }
}

impl PgStore {
    /// Resolved calibration pairs attributed to one model via the quality
    /// ledger join.
    pub async fn list_calibration_pairs_for_model(
        &self,
        model_id: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<(f64, bool)>> {
        let rows = sqlx::query_as::<_, (f64, bool)>(
            r#"SELECT p.probability, p.resolved_outcome
               FROM insight_predictions p
               JOIN analytical_quality_scores s ON s.insight_id = p.insight_id
               WHERE p.resolved_outcome IS NOT NULL
                 AND p.resolved_at >= $1
                 AND s.model_id = $2
               ORDER BY p.resolved_at DESC
               LIMIT $3"#,
        )
        .bind(since)
        .bind(model_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

impl PgStore {
    /// Prune raw analytical-quality rows older than `max_age_days` (weekly
    /// snapshots keep the long-horizon trend). Safe to call repeatedly.
    pub async fn prune_analytical_quality_scores(&self, max_age_days: i32) -> Result<i64> {
        let deleted: i64 = sqlx::query_scalar("SELECT prune_analytical_quality_scores($1)")
            .bind(max_age_days)
            .fetch_one(&self.pool)
            .await?;
        Ok(deleted)
    }
}
