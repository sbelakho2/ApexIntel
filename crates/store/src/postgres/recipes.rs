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
                 COALESCE(
                     r.precision_score,
                     AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL),
                     0.0
                 ) AS precision_score,
                 COALESCE(
                     CASE
                         WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                             THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'))::DOUBLE PRECISION
                                 / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                         ELSE 0.0::DOUBLE PRECISION
                     END,
                     0.0::DOUBLE PRECISION
                 ) AS false_positive_rate,
                 COALESCE(COUNT(w.id), 0) AS fired_count,
                 MAX(w.created_at) AS last_fired,
                 MIN(w.created_at) AS first_fired,
                 COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
               WHERE r.status IN ('active', 'production', 'staging')
               GROUP BY r.code, r.status, r.precision_score
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
                       AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL) AS avg_confidence
                   FROM recipe_scope rs
                   LEFT JOIN warnings w ON w.recipe_code = rs.code AND w.deleted_at IS NULL
                   GROUP BY rs.code
               )
               SELECT
                   COALESCE(ROUND((AVG(avg_confidence) * 100.0)::numeric, 0), 0)::bigint AS avg_precision_pct,
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
                       COALESCE(
                           r.precision_score,
                           AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL),
                           0.0
                       ) AS precision_observed,
                       COALESCE(COUNT(w.id), 0)::INT AS sample_size,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')), 0)::INT AS reviewed_warnings_total,
                       COALESCE(COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'), 0)::INT AS false_positive_warnings_total,
                       COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0)::INT AS active_count,
                       EXTRACT(DAY FROM (NOW() - r.created_at))::INT AS days_in_staging,
                       r.created_at
                   FROM recipes r
                   LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
                   WHERE r.status = 'staging'
                   GROUP BY r.code, r.precision_score, r.created_at
               )
               SELECT
                   recipe_code,
                   precision_observed,
                   LEAST(
                       1.0,
                       GREATEST(
                           0.0,
                           (0.7 * precision_observed)
                           + (0.3 * LEAST(1.0, sample_size::DOUBLE PRECISION / 100.0))
                       )
                   ) AS recall_observed,
                   COALESCE(
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE 0.0::DOUBLE PRECISION
                       END
                   ) AS false_positive_rate,
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
                       r.precision_score,
                       r.created_at
                   FROM recipes r
                   LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
                   WHERE r.status IN ('active', 'production')
                   GROUP BY r.code, r.precision_score, r.created_at
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
                   COALESCE(
                       snapshot_history.latest_precision,
                       recipe_warning_stats.precision_score,
                       0.0
                   )::DOUBLE PRECISION AS precision_current,
                   COALESCE(
                       snapshot_history.baseline_precision,
                       recipe_warning_stats.precision_score,
                       0.0
                   )::DOUBLE PRECISION AS precision_baseline,
                   COALESCE(
                       snapshot_history.false_positive_rate,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE 0.0::DOUBLE PRECISION
                       END
                   ) AS false_positive_rate,
                   COALESCE(
                       snapshot_history.fpr_baseline,
                       CASE
                           WHEN reviewed_warnings_total > 0
                               THEN false_positive_warnings_total::DOUBLE PRECISION / reviewed_warnings_total::DOUBLE PRECISION
                           ELSE 0.0::DOUBLE PRECISION
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
                       COALESCE(
                           AVG(w.confidence) FILTER (
                               WHERE w.confidence IS NOT NULL
                                 AND w.created_at >= $1 - INTERVAL '7 days'
                                 AND w.created_at <= $1
                           ),
                           r.precision_score,
                           0.0
                       )::DOUBLE PRECISION AS precision_score,
                       CASE
                           WHEN COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')) > 0
                               THEN (COUNT(*) FILTER (WHERE w.review_outcome = 'false_positive'))::DOUBLE PRECISION
                                   / (COUNT(*) FILTER (WHERE w.review_outcome IN ('true_positive', 'false_positive')))::DOUBLE PRECISION
                           ELSE 0.0::DOUBLE PRECISION
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
                   GROUP BY r.code, r.precision_score
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

    /// Auto-calibrate recipe `precision_score` thresholds based on persistent
    /// false-positive rates tracked in `recipe_weekly_metrics`.
    ///
    /// ## How it works
    ///
    /// 1. Query `recipe_weekly_metrics` for the last 4 weeks.
    /// 2. Compute per-recipe average false-positive rate over that window.
    /// 3. If the average exceeds **30 %** (`> 0.30`), reduce the recipe's
    ///    `precision_score` proportionally:
    ///
    ///    ```text
    ///    new_precision = current_precision × (1.0 − 0.5 × avg_fp_rate)
    ///    ```
    ///
    ///    clamped to `[0.0, current_precision]`.
    /// 4. Recipes already at `precision_score ≤ 0.001` are skipped (already
    ///    effectively silenced).
    /// 5. Every adjustment is recorded in the `audit_log` table for full
    ///    observability.
    ///
    /// ## Edge cases
    ///
    /// * **No recipes exceed threshold** → returns empty `Vec`, no-op.
    /// * **FP rate = 1.0 (100 %)** → precision becomes 0.0, recipe silenced.
    /// * **NULL precision_score** → skipped (`IS NOT NULL` guard).
    /// * **Idempotent** – running multiple times applies compounding reductions
    ///   (each week the FP rate is re-evaluated).
    ///
    /// ## Returns
    ///
    /// A `Vec<CalibrationAdjustment>` describing each adjustment made, suitable
    /// for logging, dashboard metrics, or alerting.
    pub async fn auto_calibrate_recipe_thresholds(&self) -> Result<Vec<CalibrationAdjustment>> {
        #[derive(Debug, Clone, sqlx::FromRow)]
        struct CalibrationRow {
            recipe_code: String,
            current_precision: f64,
            new_precision: f64,
            avg_fp_rate_4w: f64,
        }

        let adjustments: Vec<CalibrationRow> = sqlx::query_as::<_, CalibrationRow>(
            r#"WITH high_fp_recipes AS (
                   SELECT
                       m.recipe_code,
                       COALESCE(AVG(m.false_positive_rate), 0.0) AS avg_fp_rate_4w
                   FROM recipe_weekly_metrics m
                   WHERE m.week_start >= DATE_TRUNC('week', NOW() - INTERVAL '4 weeks')
                   GROUP BY m.recipe_code
                   HAVING COALESCE(AVG(m.false_positive_rate), 0.0) > 0.30
               ),
               calibrated AS (
                   SELECT
                       h.recipe_code,
                       r.precision_score AS current_precision,
                       GREATEST(
                           0.0,
                           r.precision_score * (1.0 - 0.5 * h.avg_fp_rate_4w)
                       ) AS new_precision,
                       h.avg_fp_rate_4w
                   FROM high_fp_recipes h
                   JOIN recipes r ON r.code = h.recipe_code
                   WHERE r.precision_score IS NOT NULL
                     AND r.precision_score > 0.001
                     AND r.status IN ('active', 'production')
               )
               UPDATE recipes r
               SET
                   precision_score = c.new_precision,
                   updated_at = NOW()
               FROM calibrated c
               WHERE r.code = c.recipe_code
               RETURNING
                   r.code                       AS recipe_code,
                   c.current_precision,
                   c.new_precision,
                   c.avg_fp_rate_4w"#,
        )
        .fetch_all(&self.pool)
        .await?;

        // Persist every adjustment to the audit trail
        for adj in &adjustments {
            #[allow(clippy::disallowed_methods)]
            let detail = serde_json::json!({
                "recipe_code": adj.recipe_code,
                "current_precision": adj.current_precision,
                "new_precision": adj.new_precision,
                "avg_fp_rate_4w": adj.avg_fp_rate_4w,
                "calibration_reason": format!(
                    "Average FP rate {:.2} exceeded 0.30 threshold over last 4 weeks",
                    adj.avg_fp_rate_4w
                ),
            });
            sqlx::query(
                "INSERT INTO audit_log (event_type, actor, detail) VALUES ($1, $2, $3)",
            )
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
                current_precision: r.current_precision,
                new_precision: r.new_precision,
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
                COALESCE(
                    r.precision_score,
                    AVG(w.confidence) FILTER (WHERE w.confidence IS NOT NULL),
                    0.0
                ) AS precision_score,
                COALESCE(COUNT(w.id), 0) AS fired_count,
                MAX(w.created_at) AS last_fired,
                MIN(w.created_at) AS first_fired,
                COALESCE(COUNT(*) FILTER (WHERE w.id IS NOT NULL AND NOT w.acknowledged), 0) AS active_count
               FROM recipes r
               LEFT JOIN warnings w ON w.recipe_code = r.code AND w.deleted_at IS NULL
               WHERE r.status = 'staging'
               GROUP BY r.code, r.status, r.precision_score
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
