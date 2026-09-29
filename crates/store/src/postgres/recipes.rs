use super::*;

fn normalize_recipe_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

impl PgStore {
    /// Get recipe signal statistics from recipes LEFT JOIN warnings
    /// so that recipes with zero warnings still appear in the list.
    pub async fn get_recipe_stats(&self) -> Result<Vec<RecipeStatRow>> {
        let rows = sqlx::query_as::<_, RecipeStatRow>(
            r#"SELECT
                 r.code AS recipe_code,
                 r.status,
                 -- Empirical precision requires reviewed outcomes:
                 -- TP / (TP + FP). Average model confidence is NOT precision
                 -- and is reported separately. No reviews => NULL.
                 CASE
                     WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                         THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive'))::DOUBLE PRECISION
                             / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                     ELSE NULL
                 END AS precision_score,
                 -- Mean model confidence is its own metric, never relabelled.
                 AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL) AS avg_model_confidence,
                 CASE
                     WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                         THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'))::DOUBLE PRECISION
                             / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                     ELSE NULL
                 END AS false_positive_rate,
                 COALESCE(COUNT(w.id), 0) AS fired_count,
                 MAX(w.created_at) AS last_fired,
                 MIN(w.created_at) AS first_fired,
                 COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
               WHERE r.status IN ('active', 'production', 'staging')
               GROUP BY r.code, r.status
               ORDER BY COALESCE(COUNT(w.id), 0) DESC, r.code ASC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Aggregate quality signals for recipe dashboard cards.
    pub async fn get_recipe_quality_summary(&self) -> Result<RecipeQualitySummaryRow> {
        let row = sqlx::query_as::<_, RecipeQualitySummaryRow>(
            r#"WITH recipe_scope AS (
                   SELECT code
                   FROM recipes
                   WHERE status IN ('active', 'production', 'staging')
               ),
               recipe_fire AS (
                   SELECT
                       rs.code,
                       COUNT(w.id) AS fired_count,
                       AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL) AS avg_confidence,
                       COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) AS reviewed_total,
                       COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive') AS reviewed_true_positive
                   FROM recipe_scope rs
                   LEFT JOIN warnings w ON w.recipe_code = rs.code AND w.deleted_at IS NULL
                   GROUP BY rs.code
               )
               SELECT
                   -- Empirical precision over reviewed outcomes; NULL when no
                   -- outcomes were reviewed. Mean confidence is separate.
                   CASE
                       WHEN COUNT(*) FILTER (WHERE reviewed_total > 0) > 0
                           THEN ROUND(
                               (
                                   100.0 * SUM(reviewed_true_positive)::numeric
                                   / NULLIF(SUM(reviewed_total), 0)
                               ),
                               0
                           )::bigint
                       ELSE NULL
                   END AS avg_precision_pct,
                   COALESCE(ROUND((AVG(avg_confidence) * 100.0)::numeric, 0), 0)::bigint AS avg_model_confidence_pct,
                   COALESCE(
                       ROUND((100.0 * COUNT(*) FILTER (WHERE fired_count > 0) / NULLIF(COUNT(*), 0))::numeric, 0),
                       0
                   )::bigint AS coverage_pct
               FROM recipe_fire"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get staged recipes ready for promotion evaluation.
    pub async fn get_staged_recipes_for_promotion(&self) -> Result<Vec<StagedRecipeRow>> {
        let rows = sqlx::query_as::<_, StagedRecipeRow>(
            r#"WITH recipe_warning_stats AS (
                   SELECT
                       r.code AS recipe_code,
                       -- Empirical precision from reviewed outcomes; NULL when
                       -- nothing was reviewed (mean confidence is a different
                       -- metric and is not used as precision here).
                       CASE
                           WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                               THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive'))::DOUBLE PRECISION
                                   / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                           ELSE NULL
                       END AS precision_observed,
                       COALESCE(COUNT(w.id), 0)::INT AS sample_size,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')), 0)::INT AS reviewed_warnings_total,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive')), 0)::INT AS reviewed_true_positives,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'), 0)::INT AS false_positive_warnings_total,
                       COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0)::INT AS active_count,
                       EXTRACT(DAY FROM (NOW() - r.created_at))::INT AS days_in_staging,
                       r.created_at
                   FROM recipes r
                   LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
                   WHERE r.status = 'staging'
                   GROUP BY r.code, r.created_at
               )
               SELECT
                   recipe_code,
                   precision_observed,
                   -- Promotion readiness heuristic over measured precision and
                   -- sample maturity. This is NOT recall: recall needs
                   -- TP / (TP + FN) against an evaluation set.
                   LEAST(
                       1.0,
                       GREATEST(
                           0.0,
                           (0.7 * COALESCE(precision_observed, 0.0))
                           + (0.3 * LEAST(1.0, sample_size::DOUBLE PRECISION / 100.0))
                       )
                   ) AS promotion_evidence_score,
                   CASE
                       WHEN reviewed_warnings_total > 0
                           THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                       ELSE NULL
                   END AS false_positive_rate,
                   sample_size,
                   days_in_staging,
                   created_at
               FROM recipe_warning_stats
               ORDER BY created_at ASC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get production recipes for deprecation evaluation.
    pub async fn get_production_recipes_for_deprecation(&self) -> Result<Vec<ProductionRecipeRow>> {
        let rows = sqlx::query_as::<_, ProductionRecipeRow>(
            r#"WITH recipe_warning_stats AS (
                   SELECT
                       r.code AS recipe_code,
                       COALESCE(COUNT(w.id) FILTER (WHERE w.created_at >= NOW() - INTERVAL '7 days'), 0)::BIGINT AS warnings_generated_last_week,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')), 0)::BIGINT AS reviewed_warnings_total,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'), 0)::BIGINT AS false_positive_warnings_total,
                       MAX(w.created_at) AS last_triggered_at,
                       r.created_at
                   FROM recipes r
                   LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
                   WHERE r.status IN ('active', 'production')
                   GROUP BY r.code, r.created_at
               ), ranked_snapshots AS (
                   SELECT
                       m.recipe_code,
                       m.week_start,
                       m.precision_score,
                       m.false_positive_rate,
                       ROW_NUMBER() OVER (PARTITION BY m.recipe_code ORDER BY m.week_start DESC) AS snapshot_rank
                   FROM recipe_weekly_metrics m
               ), snapshot_history AS (
                   SELECT
                       recipe_code,
                       MAX(precision_score) FILTER (WHERE snapshot_rank = 1) AS latest_precision,
                       MAX(precision_score) FILTER (WHERE snapshot_rank = 2) AS baseline_precision,
                       MAX(false_positive_rate) FILTER (WHERE snapshot_rank = 1) AS false_positive_rate,
                       MAX(false_positive_rate) FILTER (WHERE snapshot_rank = 2) AS fpr_baseline
                   FROM ranked_snapshots
                   WHERE snapshot_rank <= 8
                   GROUP BY recipe_code
               )
               SELECT
                   recipe_warning_stats.recipe_code,
                   -- Measured precision only: weekly snapshots or reviewed
                   -- outcomes; NULL when unmeasured. The configured minimum
                   -- precision is configuration and is never reported as
                   -- measured quality.
                   COALESCE(
                       snapshot_history.latest_precision,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN (reviewed_warnings_total - false_positive_warnings_total)::DOUBLE PRECISION
                                   / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE NULL
                       END
                   ) AS precision_current,
                   COALESCE(
                       snapshot_history.baseline_precision,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN (reviewed_warnings_total - false_positive_warnings_total)::DOUBLE PRECISION
                                   / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE NULL
                       END
                   ) AS precision_baseline,
                   COALESCE(
                       snapshot_history.false_positive_rate,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE NULL
                       END
                   ) AS false_positive_rate,
                   COALESCE(
                       snapshot_history.fpr_baseline,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE NULL
                       END
                   ) AS fpr_baseline,
                   recipe_warning_stats.warnings_generated_last_week,
                   recipe_warning_stats.last_triggered_at,
                   EXTRACT(DAY FROM (NOW() - COALESCE(recipe_warning_stats.last_triggered_at, recipe_warning_stats.created_at)))::INT AS days_inactive,
                   recipe_warning_stats.created_at
               FROM recipe_warning_stats
               LEFT JOIN snapshot_history ON snapshot_history.recipe_code = recipe_warning_stats.recipe_code
               ORDER BY last_triggered_at DESC NULLS LAST, created_at DESC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn record_recipe_weekly_metrics(&self, snapshot_time: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            r#"WITH metrics AS (
                   SELECT
                       r.code AS recipe_code,
                       DATE_TRUNC('week', $1)::DATE AS week_start,
                       -- Empirical precision (TP / (TP + FP)) over reviewed
                       -- outcomes; NULL when nothing was reviewed. Model
                       -- confidence is NOT precision and must never be
                       -- persisted in this column.
                       CASE
                           WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                               THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive'))::DOUBLE PRECISION
                                   / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                           ELSE NULL
                       END AS precision_score,
                       -- False-positive rate over reviewed outcomes; NULL when
                       -- unreviewed (0% would claim a reviewed clean week).
                       CASE
                           WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                               THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'))::DOUBLE PRECISION
                                   / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                           ELSE NULL
                       END AS false_positive_rate,
                       COALESCE(
                           COUNT(w.id) FILTER (
                               WHERE w.created_at >= $1 - INTERVAL '7 days'
                                 AND w.created_at <= $1
                           ),
                           0
                       )::BIGINT AS warnings_generated,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')), 0)::BIGINT AS reviewed_warnings,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'), 0)::BIGINT AS false_positive_warnings,
                       $1 AS snapshot_at
                   FROM recipes r
                   LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
                   WHERE r.status IN ('active', 'production')
                   GROUP BY r.code
               )
               INSERT INTO recipe_weekly_metrics (
                   recipe_code,
                   week_start,
                   precision_score,
                   false_positive_rate,
                   warnings_generated,
                   reviewed_warnings,
                   false_positive_warnings,
                   snapshot_at,
                   updated_at
               )
               SELECT
                   recipe_code,
                   week_start,
                   precision_score,
                   false_positive_rate,
                   warnings_generated,
                   reviewed_warnings,
                   false_positive_warnings,
                   snapshot_at,
                   now()
               FROM metrics
               ON CONFLICT (recipe_code, week_start) DO UPDATE
               SET precision_score = EXCLUDED.precision_score,
                   false_positive_rate = EXCLUDED.false_positive_rate,
                   warnings_generated = EXCLUDED.warnings_generated,
                   reviewed_warnings = EXCLUDED.reviewed_warnings,
                   false_positive_warnings = EXCLUDED.false_positive_warnings,
                   snapshot_at = EXCLUDED.snapshot_at,
                   updated_at = now()"#,
        )
        .bind(snapshot_time)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Auto-calibrate the runtime **activation threshold** from persistent
    /// false-positive evidence in `recipe_weekly_metrics`.
    ///
    /// ## How it works
    ///
    /// 1. Aggregate the last 4 weeks **weighted by reviewed samples**:
    ///    `SUM(false_positive_warnings) / SUM(reviewed_warnings)` — a
    ///    1-review week cannot outweigh a 100-review week.
    /// 2. Only recipes with at least **10 reviewed warnings** are eligible:
    ///    calibration needs a real sample, not one unlucky review.
    /// 3. If the weighted rate exceeds **30 %**, the activation threshold is
    ///    **raised** (the gate tightens, never loosens):
    ///
    ///    ```text
    ///    new_threshold = current_threshold × (1.0 + 0.5 × weighted_fp_rate)
    ///    ```
    ///
    ///    clamped to `[0.0, 1.0]`.
    /// 4. Recipes at `activation_threshold ≤ 0.001` are skipped (nothing to
    ///    tighten) and rows without a calibrated threshold are ignored.
    /// 5. Every adjustment is recorded in `audit_log`.
    ///
    /// ## Returns
    ///
    /// A `Vec<CalibrationAdjustment>` (thresholds, not measured precision)
    /// describing each adjustment made.
    pub async fn auto_calibrate_recipe_thresholds(&self) -> Result<Vec<CalibrationAdjustment>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct CalibrationRow {
            recipe_code: String,
            current_threshold: f64,
            new_threshold: f64,
            avg_fp_rate_4w: f64,
        }

        let adjustments: Vec<CalibrationRow> = sqlx::query_as::<_, CalibrationRow>(
            r#"WITH weekly AS (
                   -- Weighted evidence: total reviewed warnings and total false
                   -- positives, so a 1-review week cannot outweigh a 100-review
                   -- week the way an unweighted average of weekly rates would.
                   SELECT
                       m.recipe_code,
                       SUM(COALESCE(m.reviewed_warnings, 0))::BIGINT AS reviewed,
                       SUM(COALESCE(m.false_positive_warnings, 0))::BIGINT AS false_positives
                   FROM recipe_weekly_metrics m
                   WHERE m.week_start >= DATE_TRUNC('week', NOW() - INTERVAL '4 weeks')
                   GROUP BY m.recipe_code
               ),
               high_fp_recipes AS (
                   SELECT
                       recipe_code,
                       false_positives::DOUBLE PRECISION / reviewed::DOUBLE PRECISION AS avg_fp_rate_4w
                   FROM weekly
                   -- Evidence floor: calibration needs a real reviewed sample.
                   WHERE reviewed >= 10
                     AND false_positives::DOUBLE PRECISION / NULLIF(reviewed, 0) > 0.30
               ),
               calibrated AS (
                   SELECT
                       h.recipe_code,
                       r.activation_threshold AS current_threshold,
                       -- High false-positive rates must TIGHTEN the gate: the
                       -- threshold moves UP with the observed FPR (capped at
                       -- 1.0). The previous minus-correction made noisy
                       -- recipes easier to fire.
                       LEAST(
                           1.0,
                           GREATEST(
                               0.0,
                               r.activation_threshold * (1.0 + 0.5 * h.avg_fp_rate_4w)
                           )
                       ) AS new_threshold,
                       h.avg_fp_rate_4w
                   FROM high_fp_recipes h
                   JOIN recipes r ON r.code = h.recipe_code
                   WHERE r.activation_threshold IS NOT NULL
                     AND r.activation_threshold > 0.001
                     AND r.status IN ('active', 'production')
               )
               UPDATE recipes r
               SET
                   activation_threshold = c.new_threshold,
                   updated_at = NOW()
               FROM calibrated c
               WHERE r.code = c.recipe_code
               RETURNING
                   r.code                       AS recipe_code,
                   c.current_threshold,
                   c.new_threshold,
                   c.avg_fp_rate_4w"#,
        )
        .fetch_all(&self.pool)
        .await?;

        // Persist every adjustment to the audit trail
        for adj in &adjustments {
            #[allow(clippy::unwrap_used, clippy::expect_used)]
            let detail = serde_json::json!({
                "recipe_code": adj.recipe_code,
                "current_threshold": adj.current_threshold,
                "new_threshold": adj.new_threshold,
                "avg_fp_rate_4w": adj.avg_fp_rate_4w,
                "calibration_reason": format!(
                    "Average FP rate {:.2} exceeded 0.30 threshold over last 4 weeks",
                    adj.avg_fp_rate_4w
                ),
            });
            sqlx::query("INSERT INTO audit_log (event_type, actor, detail) VALUES ($1, $2, $3)")
                .bind("recipe_threshold_auto_calibrated")
                .bind("system")
                .bind(&detail)
                .execute(&self.pool)
                .await?;
        }

        let result: Vec<CalibrationAdjustment> = adjustments
            .into_iter()
            .map(|r| CalibrationAdjustment {
                recipe_code: r.recipe_code,
                current_threshold: r.current_threshold,
                new_threshold: r.new_threshold,
                avg_fp_rate_4w: r.avg_fp_rate_4w,
            })
            .collect();

        Ok(result)
    }

    pub async fn list_staging_recipes(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<RecipeStatRow>> {
        let (limit, offset) = normalize_recipe_window(limit, offset);
        let rows = sqlx::query_as::<_, RecipeStatRow>(
            r#"SELECT
                r.code AS recipe_code,
                r.status,
                -- Empirical precision (TP / (TP + FP)); NULL when nothing was
                -- reviewed. Average model confidence is NOT precision and is
                -- reported separately below.
                CASE
                    WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                        THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'true_positive'))::DOUBLE PRECISION
                            / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                    ELSE NULL
                END AS precision_score,
                AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL) AS avg_model_confidence,
                CASE
                    WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                        THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'))::DOUBLE PRECISION
                            / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                    ELSE NULL
                END AS false_positive_rate,
                COALESCE(COUNT(w.id), 0) AS fired_count,
                MAX(w.created_at) AS last_fired,
                MIN(w.created_at) AS first_fired,
                COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
               WHERE r.status = 'staging'
               GROUP BY r.code, r.status
               ORDER BY r.code ASC
               LIMIT $1 OFFSET $2"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn count_staging_recipes(&self) -> Result<i64> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE status = 'staging'")
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    pub async fn promote_recipe(&self, recipe_code: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE recipes SET status = 'production', updated_at = now() WHERE code = $1 AND status = 'staging'",
        )
        .bind(recipe_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn deprecate_recipe(&self, recipe_code: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE recipes SET status = 'deprecated', updated_at = now() WHERE code = $1 AND status IN ('staging', 'production', 'active')",
        )
        .bind(recipe_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn upsert_recipe_definition(
        &self,
        recipe_code: &str,
        name: &str,
        status: &str,
        definition: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO recipes (code, name, status, definition, created_at, updated_at)
               VALUES ($1, $2, $3, $4, now(), now())
               ON CONFLICT (code)
               DO UPDATE SET
                 name = EXCLUDED.name,
                 status = EXCLUDED.status,
                 definition = EXCLUDED.definition,
                 updated_at = now()"#,
        )
        .bind(recipe_code)
        .bind(name)
        .bind(status)
        .bind(definition)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The runtime recipe set for the execution engine (audit P0: PostgreSQL is
    /// the source of truth, not the YAML seed file).
    ///
    /// Returns every recipe the engine may evaluate, including recipes created
    /// or promoted in the database that never existed in YAML, with the
    /// lifecycle status and the calibrated `activation_threshold`. Deprecated
    /// recipes are included so the caller can exclude them explicitly (their
    /// status is authoritative); the caller must never resurrect a deprecated
    /// recipe as a seed.
    pub async fn list_recipes_for_engine(&self) -> Result<Vec<RecipeEngineRow>> {
        let rows = sqlx::query_as::<_, RecipeEngineRow>(
            r#"
            SELECT code,
                   name,
                   status,
                   definition,
                   category,
                   join_type,
                   outcome,
                   signals,
                   transforms,
                   test_config,
                   thresholds,
                   narrative_template,
                   to_jsonb(action_playbook) AS action_playbook,
                   applicability,
                   activation_threshold,
                   -- Rollout compatibility (migration 092): rows written by an
                   -- old binary during a rolling deploy only populate the
                   -- legacy column. Drop the COALESCE when the legacy column
                   -- is removed in a follow-up migration.
                   COALESCE(configured_activation_threshold, configured_min_precision)
                       AS configured_activation_threshold
            FROM recipes
            ORDER BY code
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// One month of historical recipe performance aggregated from the real
    /// persisted weekly snapshots (`recipe_weekly_metrics`).
    ///
    /// Only *measured* values are present. `precision_pct` and `fpr_pct` are
    /// `None` for a month with no **reviewed outcomes** — generated warnings
    /// alone do not measure precision — and an unmeasured rate is never
    /// reported as zero.
    pub async fn list_recipe_monthly_performance(
        &self,
        months: i32,
    ) -> Result<Vec<RecipeMonthlyPerformance>> {
        let rows = sqlx::query_as::<_, RecipeMonthlyPerformance>(
            r#"
            SELECT
                DATE_TRUNC('month', week_start)::DATE AS month_start,
                -- Weighted by reviewed samples: SUM(TP)/SUM(reviewed), never a
                -- mean of weekly percentages (a 1-review week must not weigh
                -- as much as a 100-review week). NULL when nothing was
                -- reviewed.
                (SUM(GREATEST(COALESCE(reviewed_warnings, 0) - COALESCE(false_positive_warnings, 0), 0))::DOUBLE PRECISION
                    / NULLIF(SUM(COALESCE(reviewed_warnings, 0)), 0)::DOUBLE PRECISION) * 100.0
                    AS precision_pct,
                (SUM(COALESCE(false_positive_warnings, 0))::DOUBLE PRECISION
                    / NULLIF(SUM(COALESCE(reviewed_warnings, 0)), 0)::DOUBLE PRECISION) * 100.0
                    AS fpr_pct,
                COALESCE(SUM(warnings_generated), 0)::BIGINT AS warnings_generated,
                COALESCE(SUM(reviewed_warnings), 0)::BIGINT AS reviewed_warnings
            FROM recipe_weekly_metrics
            WHERE week_start >= (DATE_TRUNC('month', now()) - make_interval(months => $1))::DATE
            GROUP BY 1
            ORDER BY 1
            "#,
        )
        .bind(months)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// A recipe row for the runtime execution engine.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct RecipeEngineRow {
    pub code: String,
    pub name: String,
    pub status: String,
    /// Legacy seed/source definition blob. Audit/reference only: the runtime
    /// reconstructs recipes from the canonical columns below when this is
    /// absent, so a DB-only recipe still executes.
    pub definition: Option<serde_json::Value>,
    /// Canonical recipe body (current-schema columns).
    pub category: Option<String>,
    pub join_type: Option<String>,
    pub outcome: Option<String>,
    pub signals: Option<serde_json::Value>,
    pub transforms: Option<serde_json::Value>,
    pub test_config: Option<serde_json::Value>,
    pub thresholds: Option<serde_json::Value>,
    pub narrative_template: Option<String>,
    pub action_playbook: Option<serde_json::Value>,
    pub applicability: Option<serde_json::Value>,
    /// Calibrated runtime threshold (migration 089); `None` = ungated.
    pub activation_threshold: Option<f64>,
    /// Configured activation threshold from the definition (configuration, not
    /// a measurement); the fallback gate only when no calibration ran.
    pub configured_activation_threshold: Option<f64>,
}

/// One aggregated month of real recipe performance history.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct RecipeMonthlyPerformance {
    pub month_start: NaiveDate,
    /// Mean measured precision over the month's weekly snapshots, `None` when
    /// no warnings were generated (nothing was measured).
    pub precision_pct: Option<f64>,
    /// Mean measured false-positive rate, `None` when no warnings were
    /// reviewed that month.
    pub fpr_pct: Option<f64>,
    pub warnings_generated: i64,
    pub reviewed_warnings: i64,
}

#[cfg(test)]
mod tests {
    use super::normalize_recipe_window;

    #[test]
    fn test_normalize_recipe_window_clamps_limit_and_offset() {
        assert_eq!(normalize_recipe_window(0, -5), (1, 0));
        assert_eq!(normalize_recipe_window(9999, -1).0, 500);
    }

    #[test]
    fn test_normalize_recipe_window_preserves_valid_values() {
        assert_eq!(normalize_recipe_window(25, 40), (25, 40));
    }
}
