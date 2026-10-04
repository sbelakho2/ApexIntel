use super::*;

use chrono::Duration;
use sha2::{Digest, Sha256};

/// Default cache lifetime for entries written without an explicit TTL.
const DEFAULT_LLM_CACHE_TTL: Duration = Duration::days(7);

/// SHA-256 of the exact rendered prompt, recorded on every cache entry so a
/// cached response is bound to the prompt text that produced it.
fn hash_llm_prompt(rendered_prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"apex-llm-prompt\x1f");
    hasher.update(rendered_prompt.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Effective cache key for a rendered prompt: the caller's evidence-scoped key
/// hashed together with the prompt text (domain-separated), so two different
/// renderings of the same evidence set can never collide on one cache row.
fn llm_prompt_cache_key(base_cache_key: &str, rendered_prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"apex-llm-cache-v2\x1f");
    hasher.update(base_cache_key.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(rendered_prompt.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl PgStore {
    /// Look up cached LLM output by cache key.
    ///
    /// `cache_key` is the SHA-256 of `(workflow, model_version, prompt_version,
    /// sorted evidence ids)` — see `apex_llm::cache::cache_key` — plus, for
    /// entries written by [`PgStore::put_llm_cache_grounded`], the rendered
    /// prompt. On a hit the hits counter is bumped so cache effectiveness is
    /// measurable. Expired entries are misses. A miss returns `None`; callers
    /// then run the model exactly once and store the result via
    /// [`PgStore::put_llm_cache`].
    pub async fn get_llm_cache(&self, cache_key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"UPDATE llm_cache
                  SET hits = hits + 1, last_hit_at = NOW()
                WHERE cache_key = $1
                  AND expires_at > NOW()
            RETURNING response"#,
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(response,)| response))
    }

    /// Look up the cached output for a caller's base key and rendered prompt,
    /// applying the same prompt-hashed key derivation as
    /// [`PgStore::put_llm_cache_grounded`].
    pub async fn get_llm_cache_for_prompt(
        &self,
        base_cache_key: &str,
        rendered_prompt: &str,
    ) -> Result<Option<String>> {
        let key = llm_prompt_cache_key(base_cache_key, rendered_prompt);
        self.get_llm_cache(&key).await
    }

    /// Store LLM output under `cache_key`. Idempotent: an already-populated
    /// key is left untouched (identical evidence never re-runs the model).
    ///
    /// Entries written here carry no prompt hash (legacy contract) and expire
    /// after [`DEFAULT_LLM_CACHE_TTL`]; use
    /// [`PgStore::put_llm_cache_grounded`] to bind the exact rendered prompt
    /// and control the TTL.
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
                   (cache_key, workflow, model_version, prompt_version, evidence_ids,
                    response, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, NOW() + make_interval(secs => $7))
               ON CONFLICT (cache_key) DO NOTHING"#,
        )
        .bind(cache_key)
        .bind(workflow)
        .bind(model_version)
        .bind(prompt_version)
        .bind(evidence_ids)
        .bind(response)
        .bind(DEFAULT_LLM_CACHE_TTL.num_seconds() as f64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store LLM output only when the grounding check passed, binding the
    /// exact rendered prompt into the effective cache key and recording its
    /// hash, with an explicit expiry.
    ///
    /// Returns `false` (and writes nothing) when `grounded` is false: an
    /// ungrounded response must never become a cache hit for later runs.
    #[allow(clippy::too_many_arguments)]
    pub async fn put_llm_cache_grounded(
        &self,
        base_cache_key: &str,
        workflow: &str,
        model_version: &str,
        prompt_version: &str,
        evidence_ids: &[Uuid],
        rendered_prompt: &str,
        grounded: bool,
        ttl: Duration,
        response: &str,
    ) -> Result<bool> {
        if !grounded {
            return Ok(false);
        }
        let cache_key = llm_prompt_cache_key(base_cache_key, rendered_prompt);
        let prompt_hash = hash_llm_prompt(rendered_prompt);
        let ttl_secs = ttl.num_seconds().max(1) as f64;

        let result = sqlx::query(
            r#"INSERT INTO llm_cache
                   (cache_key, workflow, model_version, prompt_version, evidence_ids,
                    prompt_hash, response, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW() + make_interval(secs => $8))
               ON CONFLICT (cache_key) DO NOTHING"#,
        )
        .bind(&cache_key)
        .bind(workflow)
        .bind(model_version)
        .bind(prompt_version)
        .bind(evidence_ids)
        .bind(&prompt_hash)
        .bind(response)
        .bind(ttl_secs)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
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

    /// Number of entries for a workflow whose expiry has passed
    /// (test/diagnostic helper).
    pub async fn count_expired_llm_cache_entries(&self, workflow: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM llm_cache WHERE workflow = $1 AND expires_at <= NOW()",
        )
        .bind(workflow)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_llm_prompt, llm_prompt_cache_key};

    #[test]
    fn prompt_hash_is_deterministic_and_prompt_specific() {
        let a = hash_llm_prompt("prompt one");
        assert_eq!(a, hash_llm_prompt("prompt one"));
        assert_ne!(a, hash_llm_prompt("prompt two"));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn cache_key_binds_the_rendered_prompt() {
        let base = "evidence-key";
        let key = llm_prompt_cache_key(base, "rendered A");
        assert_eq!(key, llm_prompt_cache_key(base, "rendered A"));
        assert_ne!(
            key,
            llm_prompt_cache_key(base, "rendered B"),
            "different rendered prompts of the same evidence must not share a cache row"
        );
        assert_ne!(key, llm_prompt_cache_key("other", "rendered A"));
        assert_ne!(key, base, "the effective key must not be the bare base key");
    }
}
