//! Explicit warning evidence links (audit warning-evidence item, migration 082).
//!
//! Every source URL attached to a warning is resolved to a real, citable
//! evidence chain at warning-creation time:
//!
//!   `sources` (source document, `content_hash`) -> `observations` (observation
//!   extracted from that document) -> `warning_evidence` (link to the warning)
//!
//! Analysis then consumes the linked observation ids instead of relying on
//! `warnings.source_urls` strings, so a warning with a primary source URL and
//! no entity ids still has citable evidence. The link is idempotent per
//! `(warning_id, source_url)` and uses a deterministic observation id, so a
//! deterministically deduplicated warning that merges on recurrence
//! re-affirms its existing evidence instead of duplicating it.

use super::*;
use apex_core::analysis::registrable_domain;
use sha2::{Digest, Sha256};

/// Observation type used for the observation extracted from a warning's own
/// source document. It is a real observation row, so analysis citations to it
/// pass the same foreign-key and claim-kind integrity checks as any other
/// observation.
pub const WARNING_SOURCE_CITATION_OBSERVATION_TYPE: &str = "WarningSourceCitation";

/// A persisted warning evidence link with its source document reference.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct WarningEvidenceRow {
    pub id: Uuid,
    pub warning_id: Uuid,
    pub source_id: Option<Uuid>,
    pub observation_id: Uuid,
    pub source_url: String,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Canonical content hash preserved for one warning evidence link: sha256 over
/// the canonical citation content (warning title, description, source URL),
/// with a version prefix so the domain of the hash is explicit.
pub fn warning_evidence_content_hash(
    title: &str,
    description: Option<&str>,
    source_url: &str,
) -> String {
    let canonical = format!(
        "warning-evidence-v1\n{}\n{}\n{}",
        title.trim(),
        description.unwrap_or("").trim(),
        source_url.trim()
    );
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Deterministic observation id for the citation extracted from one source URL
/// of one warning. Stable across retries and warning merges, so the same
/// `(warning, url)` pair always resolves to the same observation row.
pub fn warning_evidence_observation_id(warning_id: Uuid, source_url: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("warning-evidence:{warning_id}:{}", source_url.trim()).as_bytes(),
    )
}

/// Ensure the warning has one `warning_evidence` link per source URL, with the
/// source document resolved (or created) and an observation extracted from it.
///
/// Runs on any connection so callers can wrap it in the warning-upsert
/// transaction (the alert path commits warning + evidence + outbox together).
/// Idempotent: re-running re-affirms existing links instead of duplicating
/// them, and preserves the content hash of the citation.
pub async fn link_warning_evidence_on(
    conn: &mut sqlx::PgConnection,
    warning_id: Uuid,
    title: &str,
    description: Option<&str>,
    entity_ids: &[Uuid],
    confidence: Option<f64>,
    source_urls: &[String],
) -> Result<usize> {
    let mut linked = 0usize;
    let entity_id = entity_ids.first().copied();
    let entity_type = entity_id.map(|_| "company".to_string());

    for source_url in source_urls {
        let source_url = source_url.trim();
        if source_url.is_empty() {
            continue;
        }
        let content_hash = warning_evidence_content_hash(title, description, source_url);
        let observation_id = warning_evidence_observation_id(warning_id, source_url);

        // Resolve the source document first (idempotent upsert by URL).
        let existing_source: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM sources WHERE url = $1 ORDER BY fetched_at DESC LIMIT 1",
        )
        .bind(source_url)
        .fetch_optional(&mut *conn)
        .await?;

        let excerpt = description
            .map(|value| value.trim().chars().take(600).collect::<String>())
            .filter(|value| !value.is_empty());
        let source_id = match existing_source {
            Some(id) => id,
            None => {
                let metadata = serde_json::json!({
                    "warning_id": warning_id,
                    "origin": "warning_evidence",
                });
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO sources (url, source_kind, content_hash, excerpt, metadata) \
                     VALUES ($1, 'warning_citation', $2, $3, $4) \
                     RETURNING id",
                )
                .bind(source_url)
                .bind(&content_hash)
                .bind(&excerpt)
                .bind(&metadata)
                .fetch_one(&mut *conn)
                .await?
            }
        };

        let domain = registrable_domain(source_url);
        let value = serde_json::json!({
            "title": title.trim(),
            "description": description.unwrap_or("").trim(),
            "excerpt": excerpt,
            "source_url": source_url,
            "content_hash": content_hash,
        });
        let provenance = serde_json::json!({
            "source_url": source_url,
            "source_domain": domain,
            "source_id": source_id,
            "content_hash": content_hash,
            "warning_id": warning_id,
            "evidence_kind": "warning_source_citation",
        });
        sqlx::query(
            "INSERT INTO observations \
                 (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at) \
             VALUES ($1, $2, $3, $4, NOW(), $5, $6, $7, NOW()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(observation_id)
        .bind(WARNING_SOURCE_CITATION_OBSERVATION_TYPE)
        .bind(entity_id)
        .bind(&entity_type)
        .bind(&value)
        .bind(&provenance)
        .bind(confidence)
        .execute(&mut *conn)
        .await?;

        let result = sqlx::query(
            "INSERT INTO warning_evidence \
                 (warning_id, source_id, observation_id, source_url, content_hash) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (warning_id, source_url) DO UPDATE SET \
                 source_id = EXCLUDED.source_id, \
                 observation_id = EXCLUDED.observation_id, \
                 content_hash = EXCLUDED.content_hash",
        )
        .bind(warning_id)
        .bind(source_id)
        .bind(observation_id)
        .bind(source_url)
        .bind(&content_hash)
        .execute(&mut *conn)
        .await?;
        linked += result.rows_affected() as usize;
    }
    Ok(linked)
}

impl PgStore {
    /// Citable observation rows linked to a warning through `warning_evidence`,
    /// newest first. This is the analysis's direct evidence set when the
    /// warning has explicit links.
    pub async fn list_warning_evidence_observations(
        &self,
        warning_id: Uuid,
    ) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT o.id, o.observation_type, o.entity_id, o.entity_type, o.ts_utc, \
                    o.value, o.provenance, o.confidence, o.created_at \
             FROM warning_evidence e \
             JOIN observations o ON o.id = e.observation_id \
             WHERE e.warning_id = $1 \
             ORDER BY o.ts_utc DESC, e.created_at DESC, o.id ASC",
        )
        .bind(warning_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Persisted evidence links for a warning, with their source references.
    pub async fn list_warning_evidence(&self, warning_id: Uuid) -> Result<Vec<WarningEvidenceRow>> {
        let rows = sqlx::query_as::<_, WarningEvidenceRow>(
            "SELECT id, warning_id, source_id, observation_id, source_url, content_hash, created_at \
             FROM warning_evidence WHERE warning_id = $1 ORDER BY created_at ASC, id ASC",
        )
        .bind(warning_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_content_sensitive() {
        let first = warning_evidence_content_hash("Title", Some("Desc"), "https://example.com/a");
        let second = warning_evidence_content_hash("Title", Some("Desc"), "https://example.com/a");
        assert_eq!(first, second, "same citation content must hash identically");
        assert_eq!(first.len(), 64, "sha256 hex digest");

        assert_ne!(
            first,
            warning_evidence_content_hash("Title", Some("Changed"), "https://example.com/a")
        );
        assert_ne!(
            first,
            warning_evidence_content_hash("Title", Some("Desc"), "https://example.com/b")
        );
    }

    #[test]
    fn observation_id_is_deterministic_per_warning_and_url() {
        let warning = Uuid::new_v4();
        let first = warning_evidence_observation_id(warning, "https://example.com/a");
        assert_eq!(
            first,
            warning_evidence_observation_id(warning, "https://example.com/a")
        );
        assert_ne!(
            first,
            warning_evidence_observation_id(warning, "https://example.com/b")
        );
        assert_ne!(
            first,
            warning_evidence_observation_id(Uuid::new_v4(), "https://example.com/a")
        );
    }
}
