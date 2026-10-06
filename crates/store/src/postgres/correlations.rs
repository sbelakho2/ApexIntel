use super::*;

/// One persisted cross-signal correlation discovered by the weekly
/// correlation-mining pass (see migration 108).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct InsightCorrelationRow {
    pub id: Uuid,
    pub correlation_kind: String,
    pub signal_domain_a: String,
    pub signal_domain_b: String,
    pub entity_a: Option<String>,
    pub entity_b: Option<String>,
    pub strength: f64,
    pub p_value: Option<f64>,
    pub lag_days: Option<i32>,
    pub evidence_count: i64,
    pub status: String,
    pub derived_recipe_codes: Vec<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// Input for [`PgStore::upsert_insight_correlations`].
#[derive(Debug, Clone)]
pub struct InsightCorrelationInput {
    pub correlation_kind: String,
    pub signal_domain_a: String,
    pub signal_domain_b: String,
    pub entity_a: Option<String>,
    pub entity_b: Option<String>,
    pub strength: f64,
    pub p_value: Option<f64>,
    pub lag_days: Option<i32>,
    pub evidence_count: i64,
}

impl PgStore {
    /// Upsert discovered correlations. The identity is
    /// (correlation_kind, signal_domain_a, signal_domain_b, entity_a, entity_b);
    /// a re-discovered correlation refreshes its strength, p-value, evidence
    /// count and `last_seen_at` instead of duplicating the row.
    pub async fn upsert_insight_correlations(
        &self,
        rows: &[InsightCorrelationInput],
    ) -> Result<u64> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut inserted = 0u64;
        for row in rows {
            let result = sqlx::query(
                r#"INSERT INTO insight_correlations
                     (correlation_kind, signal_domain_a, signal_domain_b,
                      entity_a, entity_b, strength, p_value, lag_days,
                      evidence_count)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                   ON CONFLICT (correlation_kind, signal_domain_a, signal_domain_b,
                                COALESCE(entity_a, ''), COALESCE(entity_b, ''))
                   DO UPDATE SET
                     strength = EXCLUDED.strength,
                     p_value = EXCLUDED.p_value,
                     lag_days = EXCLUDED.lag_days,
                     evidence_count = EXCLUDED.evidence_count,
                     last_seen_at = now(),
                     updated_at = now()"#,
            )
            .bind(&row.correlation_kind)
            .bind(&row.signal_domain_a)
            .bind(&row.signal_domain_b)
            .bind(row.entity_a.as_deref())
            .bind(row.entity_b.as_deref())
            .bind(row.strength)
            .bind(row.p_value)
            .bind(row.lag_days)
            .bind(row.evidence_count)
            .execute(&self.pool)
            .await?;
            inserted += result.rows_affected();
        }
        Ok(inserted)
    }

    /// Strongest correlations in a state, newest activity first — the input
    /// for deepening recipes with correlation-backed insights.
    pub async fn list_top_insight_correlations(
        &self,
        status: &str,
        limit: i64,
    ) -> Result<Vec<InsightCorrelationRow>> {
        Ok(sqlx::query_as::<_, InsightCorrelationRow>(
            r#"SELECT id, correlation_kind, signal_domain_a, signal_domain_b,
                      entity_a, entity_b, strength, p_value, lag_days,
                      evidence_count, status, derived_recipe_codes,
                      first_seen_at, last_seen_at, created_at, updated_at
               FROM insight_correlations
               WHERE status = $1
               ORDER BY strength DESC, last_seen_at DESC
               LIMIT $2"#,
        )
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Mark correlations as staged and record which recipe codes were
    /// deepened from them (the audit link between a correlation and the
    /// deep-insight recipe it produced).
    pub async fn mark_correlations_staged(
        &self,
        ids: &[Uuid],
        recipe_codes: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            r#"UPDATE insight_correlations
               SET status = 'staged',
                   derived_recipe_codes = $2,
                   updated_at = now()
               WHERE id = ANY($1)
                 AND status = 'discovered'"#,
        )
        .bind(ids)
        .bind(recipe_codes)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
