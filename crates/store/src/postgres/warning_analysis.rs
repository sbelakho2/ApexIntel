//! Durable, deduplicated warning-analysis runs and claim-level evidence
//! (audit P0-3/P0-4/P1-5, migration 079).
//!
//! An analysis run is enqueued with an `evidence_digest` computed over the
//! exact evidence set the model will see (and the warning revision that framed
//! it). Repeated clicks on identical evidence deduplicate onto the in-flight
//! or already-succeeded run instead of paying for the same work twice.
//!
//! Validated claims are persisted through the same integrity system as insight
//! claims (`insight_claim_evidence`, migration 078): a normalized citation join
//! with foreign keys to `observations` / `insights`, a claim-kind CHECK, and a
//! deferred cross-table guard that rejects observed/inference claims with no
//! real evidence at COMMIT.

use super::*;
use apex_core::claims::{AnalysisClaimRecord, EvidenceRef};

/// A request to enqueue one analysis run. The evidence digest is computed by
/// the caller over the bounded evidence set it will hand to the model.
#[derive(Debug, Clone)]
pub struct NewWarningAnalysisRun {
    pub warning_id: Uuid,
    pub requested_by: Option<String>,
    pub model: String,
    pub prompt_version: String,
    pub evidence_digest: String,
    pub observations_available: i32,
    pub observations_sent: i32,
    pub insights_available: i32,
    pub insights_sent: i32,
}

/// A persisted analysis run.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct WarningAnalysisRunRow {
    pub id: Uuid,
    pub warning_id: Uuid,
    pub status: String,
    pub requested_by: Option<String>,
    pub model: String,
    pub prompt_version: String,
    pub evidence_digest: String,
    pub observations_available: i32,
    pub observations_sent: i32,
    pub insights_available: i32,
    pub insights_sent: i32,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// A persisted analysis claim with its resolved evidence references.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WarningAnalysisClaimRow {
    pub id: Uuid,
    pub run_id: Uuid,
    /// `claim` | `impact` | `action`.
    pub section: String,
    pub claim: String,
    pub claim_kind: String,
    pub confidence: Option<f64>,
    pub evidence_count: i32,
    pub evidence: Vec<EvidenceRef>,
    pub created_at: DateTime<Utc>,
}

const RUN_COLUMNS: &str = "id, warning_id, status, requested_by, model, prompt_version, \
     evidence_digest, observations_available, observations_sent, insights_available, \
     insights_sent, output, error, created_at, started_at, finished_at, updated_at";

impl PgStore {
    /// Enqueue an analysis run, deduplicating identical evidence.
    ///
    /// Returns the run row plus whether this call deduplicated onto an existing
    /// run. Dedupe rules (migration 079 partial unique indexes):
    /// * one queued/running run per (warning, evidence digest), any prompt
    ///   version;
    /// * one succeeded run per (warning, evidence digest, prompt version), so a
    ///   new prompt version creates a new comparable run.
    pub async fn enqueue_warning_analysis_run(
        &self,
        request: &NewWarningAnalysisRun,
    ) -> Result<(WarningAnalysisRunRow, bool)> {
        // Cache check first: a succeeded run for the same evidence and prompt
        // version is reused. The insert below cannot rely on the succeeded
        // partial unique index for this, because the candidate row's status
        // ('queued') does not satisfy that index's predicate.
        let inserted = sqlx::query_as::<_, WarningAnalysisRunRow>(&format!(
            "INSERT INTO warning_analysis_runs \
                 (warning_id, status, requested_by, model, prompt_version, evidence_digest, \
                  observations_available, observations_sent, insights_available, insights_sent) \
             SELECT $1, 'queued', $2, $3, $4, $5, $6, $7, $8, $9 \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM warning_analysis_runs \
                 WHERE warning_id = $1 AND evidence_digest = $5 \
                   AND prompt_version = $4 AND status = 'succeeded' \
             ) \
             ON CONFLICT DO NOTHING \
             RETURNING {RUN_COLUMNS}"
        ))
        .bind(request.warning_id)
        .bind(&request.requested_by)
        .bind(&request.model)
        .bind(&request.prompt_version)
        .bind(&request.evidence_digest)
        .bind(request.observations_available)
        .bind(request.observations_sent)
        .bind(request.insights_available)
        .bind(request.insights_sent)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = inserted {
            return Ok((row, false));
        }

        let existing = sqlx::query_as::<_, WarningAnalysisRunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM warning_analysis_runs \
             WHERE warning_id = $1 AND evidence_digest = $2 \
               AND (status IN ('queued', 'running') \
                    OR (status = 'succeeded' AND prompt_version = $3)) \
             ORDER BY CASE WHEN status = 'succeeded' THEN 0 ELSE 1 END, \
                      created_at DESC, id DESC LIMIT 1"
        ))
        .bind(request.warning_id)
        .bind(&request.evidence_digest)
        .bind(&request.prompt_version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "analysis run insert conflicted but no matching run was found \
                 (warning {} digest {})",
                request.warning_id,
                request.evidence_digest
            )
        })?;
        Ok((existing, true))
    }

    pub async fn get_warning_analysis_run(
        &self,
        run_id: Uuid,
    ) -> Result<Option<WarningAnalysisRunRow>> {
        Ok(sqlx::query_as::<_, WarningAnalysisRunRow>(&format!(
            "SELECT {RUN_COLUMNS} FROM warning_analysis_runs WHERE id = $1"
        ))
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Transition a queued run to running. Returns false when the run is not
    /// queued (another executor claimed it, or it is already terminal).
    pub async fn start_warning_analysis_run(&self, run_id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE warning_analysis_runs \
             SET status = 'running', started_at = NOW(), updated_at = NOW() \
             WHERE id = $1 AND status = 'queued'",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Mark a queued/running run failed with an explicit reason. Succeeded
    /// runs are never overwritten.
    pub async fn fail_warning_analysis_run(&self, run_id: Uuid, error: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE warning_analysis_runs \
             SET status = 'failed', error = $2, finished_at = NOW(), updated_at = NOW() \
             WHERE id = $1 AND status IN ('queued', 'running')",
        )
        .bind(run_id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Fail queued/running runs that have not progressed for `stale_secs`, so a
    /// process crash cannot leave a run "running" forever. Returns how many
    /// runs were failed.
    pub async fn expire_stale_warning_analysis_runs(&self, stale_secs: i64) -> Result<u64> {
        let stale_secs = stale_secs.max(60);
        let result = sqlx::query(
            "UPDATE warning_analysis_runs \
             SET status = 'failed', \
                 error = 'analysis run made no progress within ' || $1 || ' seconds; \
                          no result was persisted (the process may have restarted)', \
                 finished_at = NOW(), updated_at = NOW() \
             WHERE status IN ('queued', 'running') \
               AND updated_at < NOW() - make_interval(secs => $1::double precision)",
        )
        .bind(stale_secs)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Persist the validated claims and the rendered output for a running run
    /// in one transaction. Claims are policy-validated and every cited evidence
    /// id must resolve to a real observation or insight; otherwise the whole
    /// completion is rejected and the run can be failed explicitly.
    pub async fn complete_warning_analysis_run(
        &self,
        run_id: Uuid,
        output: &serde_json::Value,
        claims: &[AnalysisClaimRecord],
    ) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM warning_analysis_runs WHERE id = $1 FOR UPDATE")
                .bind(run_id)
                .fetch_optional(&mut *tx)
                .await?;
        match status.as_deref() {
            Some("running") => {}
            Some(other) => {
                anyhow::bail!("analysis run {run_id} is '{other}'; only a running run can complete")
            }
            None => anyhow::bail!("analysis run {run_id} not found"),
        }

        let inserted = Self::insert_warning_analysis_claims_on(&mut tx, run_id, claims).await?;

        sqlx::query(
            "UPDATE warning_analysis_runs \
             SET status = 'succeeded', output = $2, error = NULL, \
                 finished_at = NOW(), updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(run_id)
        .bind(output)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(inserted)
    }

    /// Insert validated claims for a run as part of the run's completion
    /// transaction (private entry point used by
    /// [`PgStore::complete_warning_analysis_run`]). Idempotent per
    /// `(run_id, section, claim)`.
    async fn insert_warning_analysis_claims_on(
        conn: &mut sqlx::PgConnection,
        run_id: Uuid,
        claims: &[AnalysisClaimRecord],
    ) -> Result<u64> {
        let mut inserted = 0u64;
        for claim in claims {
            claim
                .validate()
                .map_err(|error| anyhow::anyhow!("invalid analysis claim: {error}"))?;

            // Strict: every cited id must resolve to a real observation or
            // insight. The database FK enforces this too; checking here gives a
            // descriptive error and lets the caller fail the run cleanly.
            for evidence in &claim.evidence {
                let exists: bool = match evidence {
                    EvidenceRef::Observation(id) => {
                        sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM observations WHERE id = $1)",
                        )
                        .bind(id)
                        .fetch_one(&mut *conn)
                        .await?
                    }
                    EvidenceRef::Insight(id) => {
                        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM insights WHERE id = $1)")
                            .bind(id)
                            .fetch_one(&mut *conn)
                            .await?
                    }
                };
                if !exists {
                    anyhow::bail!(
                        "analysis claim '{}' cites missing {} {}",
                        claim.claim.trim(),
                        evidence.kind_str(),
                        evidence.id()
                    );
                }
            }

            let kind = claim.kind.as_str();
            let text = claim.claim.trim();
            let section = claim.section.as_str();
            // Claim identity includes the section: the same wording can appear
            // in two sections (e.g. an impact statement and an action), and a
            // text-only key would collapse them with last-writer-wins.
            // New rows start as `unknown` with zero evidence so they are
            // policy-valid before links exist; the row is promoted to its real
            // kind once the join table state is settled.
            let claim_id: Uuid = match sqlx::query_scalar::<_, Uuid>(
                r#"INSERT INTO warning_analysis_claims
                       (run_id, section, claim, confidence, claim_kind, claim_hash)
                   VALUES ($1, $2, $3, $4, 'unknown', md5($2 || ':' || $3))
                   ON CONFLICT (run_id, claim_hash) DO NOTHING
                   RETURNING id"#,
            )
            .bind(run_id)
            .bind(section)
            .bind(text)
            .bind(claim.confidence)
            .fetch_optional(&mut *conn)
            .await?
            {
                Some(id) => {
                    inserted += 1;
                    id
                }
                None => {
                    sqlx::query_scalar::<_, Uuid>(
                        "SELECT id FROM warning_analysis_claims \
                     WHERE run_id = $1 AND claim_hash = md5($2 || ':' || $3)",
                    )
                    .bind(run_id)
                    .bind(section)
                    .bind(text)
                    .fetch_one(&mut *conn)
                    .await?
                }
            };

            sqlx::query(
                r#"UPDATE warning_analysis_claims
                   SET claim_kind = $1,
                       section = $2,
                       evidence_count = $3,
                       confidence = COALESCE($4, confidence)
                   WHERE id = $5"#,
            )
            .bind(kind)
            .bind(claim.section.as_str())
            .bind(claim.evidence.len() as i32)
            .bind(claim.confidence)
            .bind(claim_id)
            .execute(&mut *conn)
            .await?;

            for evidence in &claim.evidence {
                match evidence {
                    EvidenceRef::Observation(observation_id) => {
                        sqlx::query(
                            r#"INSERT INTO warning_analysis_claim_evidence
                                   (claim_id, observation_id)
                               VALUES ($1, $2)
                               ON CONFLICT (claim_id, evidence_key) DO NOTHING"#,
                        )
                        .bind(claim_id)
                        .bind(observation_id)
                        .execute(&mut *conn)
                        .await?;
                    }
                    EvidenceRef::Insight(insight_id) => {
                        sqlx::query(
                            r#"INSERT INTO warning_analysis_claim_evidence
                                   (claim_id, insight_id)
                               VALUES ($1, $2)
                               ON CONFLICT (claim_id, evidence_key) DO NOTHING"#,
                        )
                        .bind(claim_id)
                        .bind(insight_id)
                        .execute(&mut *conn)
                        .await?;
                    }
                }
            }

            // Drop links that are no longer cited (idempotent re-runs).
            sqlx::query(
                r#"DELETE FROM warning_analysis_claim_evidence
                   WHERE claim_id = $1
                     AND evidence_key <> ALL($2::text[])"#,
            )
            .bind(claim_id)
            .bind(
                claim
                    .evidence
                    .iter()
                    .map(|evidence| format!("{}:{}", evidence.kind_str(), evidence.id()))
                    .collect::<Vec<String>>(),
            )
            .execute(&mut *conn)
            .await?;
        }
        Ok(inserted)
    }

    /// Persisted claims for a run, with their resolved evidence references.
    pub async fn list_warning_analysis_claims(
        &self,
        run_id: Uuid,
    ) -> Result<Vec<WarningAnalysisClaimRow>> {
        let rows = sqlx::query(
            r#"SELECT c.id, c.run_id, c.section, c.claim, c.claim_kind, c.confidence,
                      c.evidence_count, c.created_at,
                      e.observation_id, e.insight_id
               FROM warning_analysis_claims c
               LEFT JOIN warning_analysis_claim_evidence e ON e.claim_id = c.id
               WHERE c.run_id = $1
               ORDER BY c.created_at ASC, c.id ASC, e.evidence_key ASC"#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        let mut claims: Vec<WarningAnalysisClaimRow> = Vec::new();
        for row in rows {
            let claim_id: Uuid = row.try_get("id")?;
            let observation_id: Option<Uuid> = row.try_get("observation_id")?;
            let insight_id: Option<Uuid> = row.try_get("insight_id")?;
            let evidence = match (observation_id, insight_id) {
                (Some(id), _) => Some(EvidenceRef::Observation(id)),
                (_, Some(id)) => Some(EvidenceRef::Insight(id)),
                _ => None,
            };
            match claims.iter_mut().find(|existing| existing.id == claim_id) {
                Some(existing) => {
                    if let Some(evidence) = evidence {
                        existing.evidence.push(evidence);
                    }
                }
                None => claims.push(WarningAnalysisClaimRow {
                    id: claim_id,
                    run_id: row.try_get("run_id")?,
                    section: row.try_get("section")?,
                    claim: row.try_get("claim")?,
                    claim_kind: row.try_get("claim_kind")?,
                    confidence: row.try_get("confidence")?,
                    evidence_count: row.try_get("evidence_count")?,
                    evidence: evidence.into_iter().collect(),
                    created_at: row.try_get("created_at")?,
                }),
            }
        }
        Ok(claims)
    }

    /// Source-quality data measured elsewhere (migration 014), keyed by
    /// registrable domain: `(source_domain, tier)`. Used to expose reliability
    /// as independent data rather than deriving it from how many sources
    /// happen to be present.
    pub async fn get_source_reliability_stats_for_domains(
        &self,
        domains: &[String],
    ) -> Result<Vec<(String, String)>> {
        if domains.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT source_domain, tier \
             FROM source_reliability_stats WHERE source_domain = ANY($1)",
        )
        .bind(domains)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_columns_are_qualified_for_shared_sql() {
        assert!(RUN_COLUMNS.contains("evidence_digest"));
        assert!(RUN_COLUMNS.contains("prompt_version"));
    }
}
