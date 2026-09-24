use super::*;

impl PgStore {
    /// Look up cached LLM output by cache key.
    ///
    /// `cache_key` is the SHA-256 of `(workflow, model_version, prompt_version,
    /// sorted evidence ids)` — see `apex_llm::cache::cache_key`. On a hit the
    /// hits counter is bumped so cache effectiveness is measurable. A miss
    /// returns `None`; callers then run the model exactly once and store the
    /// result via [`PgStore::put_llm_cache`].
    pub async fn get_llm_cache(&self, cache_key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"UPDATE llm_cache
                  SET hits = hits + 1, last_hit_at = NOW()
                WHERE cache_key = $1
            RETURNING response"#,
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(response,)| response))
    }

    /// Store LLM output under `cache_key`. Idempotent: an already-populated
    /// key is left untouched (identical evidence never re-runs the model).
    pub async fn put_llm_cache(
        &self,
        cache_key: &str,
        workflow: &str,
        model_version: &str,
        prompt_version: &str,
        evidence_ids: &[Uuid],
        response: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO llm_cache
                   (cache_key, workflow, model_version, prompt_version, evidence_ids, response)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (cache_key) DO NOTHING"#,
        )
        .bind(cache_key)
        .bind(workflow)
        .bind(model_version)
        .bind(prompt_version)
        .bind(evidence_ids)
        .bind(response)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Number of cached entries for a workflow (test/diagnostic helper).
    pub async fn count_llm_cache_entries(&self, workflow: &str) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*)::bigint FROM llm_cache WHERE workflow = $1")
                .bind(workflow)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }
}
