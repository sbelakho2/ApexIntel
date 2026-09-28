//! Explicit warning evidence links (audit P0-5/P0-6, migrations 083 + 085).
//!
//! A source URL is not evidence. Every `warning_evidence` link is either
//!
//!   * `resolved` — backed by a **real** object:
//!     - a real observation extracted from the source document (matched by the
//!       observation's recorded `provenance.url`), or
//!     - an explicitly referenced observation / fetched source document passed
//!       by the producer, or
//!     - a fetched `sources` document for the URL whose `content_hash` is the
//!       hash of the actual fetched body; or
//!   * `unresolved` — only a URL is known and source acquisition has not
//!     succeeded. Unresolved links carry no observation, no hash, and an
//!     explicit reason.
//!
//! The linker never fabricates an observation from the warning's own text and
//! never timestamps evidence `NOW()`: the evidence objects carry their own
//! extraction/fetch times. Migration 085 removed the fabricated
//! `WarningSourceCitation` rows this module used to create.
//!
//! The chain analysis consumes:
//!
//!   sources (fetched document + body hash) -> observations (real extract) ->
//!   warning_evidence -> warning

use super::*;
use apex_core::analysis::registrable_domain;

/// An explicit evidence reference supplied by a warning producer that already
/// knows which real object the warning is grounded in.
///
/// A bare URL is intentionally not representable here: if only a URL is
/// available, pass it as a source URL and the linker records the link as
/// `unresolved` until acquisition succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum WarningEvidenceRef {
    /// An observation row that really exists (extracted from a document).
    Observation(Uuid),
    /// A fetched `sources` document row that really exists.
    SourceDocument(Uuid),
}

/// A persisted warning evidence link with its source document reference.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct WarningEvidenceRow {
    pub id: Uuid,
    pub warning_id: Uuid,
    pub source_id: Option<Uuid>,
    pub observation_id: Option<Uuid>,
    pub source_url: Option<String>,
    pub content_hash: Option<String>,
    pub evidence_kind: String,
    pub status: String,
    pub unresolved_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The real extracted observation for a source URL, if the crawl pipeline
/// produced one. Matched strictly on the observation's recorded provenance
/// URL — never on the warning's own text.
async fn observation_for_source_url(
    conn: &mut sqlx::PgConnection,
    source_url: &str,
) -> Result<Option<(Uuid, Option<String>)>> {
    let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT id, provenance->>'content_hash' \
         FROM observations \
         WHERE provenance->>'url' = $1 \
         ORDER BY ts_utc DESC, created_at DESC, id ASC \
         LIMIT 1",
    )
    .bind(source_url)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// A fetched source document for the URL: only rows that actually hold fetched
/// content (a `content_hash`), excluding the synthetic citation rows created
/// by the old linker.
async fn fetched_source_document(
    conn: &mut sqlx::PgConnection,
    source_url: &str,
) -> Result<Option<(Uuid, String)>> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, content_hash FROM sources \
         WHERE url = $1 \
           AND content_hash IS NOT NULL AND btrim(content_hash) <> '' \
           AND COALESCE(source_kind, '') <> 'warning_citation' \
         ORDER BY fetched_at DESC NULLS LAST, id ASC \
         LIMIT 1",
    )
    .bind(source_url)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// Upsert one resolved link keyed by its real reference.
#[allow(clippy::too_many_arguments)]
async fn upsert_resolved_link(
    conn: &mut sqlx::PgConnection,
    warning_id: Uuid,
    source_id: Option<Uuid>,
    observation_id: Option<Uuid>,
    source_url: Option<&str>,
    content_hash: Option<&str>,
    evidence_kind: &str,
) -> Result<()> {
    // `ON CONFLICT` needs a single arbiter; the reference-scoped unique
    // indexes each cover one column, so resolve by explicit lookup first.
    let existing: Option<Uuid> = if let Some(observation_id) = observation_id {
        sqlx::query_scalar(
            "SELECT id FROM warning_evidence WHERE warning_id = $1 AND observation_id = $2 LIMIT 1",
        )
        .bind(warning_id)
        .bind(observation_id)
        .fetch_optional(&mut *conn)
        .await?
    } else if let Some(source_id) = source_id {
        sqlx::query_scalar(
            "SELECT id FROM warning_evidence WHERE warning_id = $1 AND source_id = $2 LIMIT 1",
        )
        .bind(warning_id)
        .bind(source_id)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        None
    };

    // A URL previously recorded as `unresolved` is upgraded in place when the
    // real object appears; the unique index on (warning_id, source_url) would
    // otherwise reject the resolved insert.
    let existing = match existing {
        Some(id) => Some(id),
        None => match source_url {
            Some(source_url) => {
                sqlx::query_scalar(
                    "SELECT id FROM warning_evidence \
                 WHERE warning_id = $1 AND source_url = $2 LIMIT 1",
                )
                .bind(warning_id)
                .bind(source_url)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        },
    };

    match existing {
        Some(id) => {
            // One link per (warning, real reference). When a URL resolves to
            // both an observation and its source document, the row carries
            // both and keeps the stronger `observation` kind.
            sqlx::query(
                "UPDATE warning_evidence SET \
                     source_id = COALESCE($2, source_id), \
                     observation_id = COALESCE($3, observation_id), \
                     source_url = COALESCE($4, source_url), \
                     content_hash = COALESCE($5, content_hash), \
                     evidence_kind = CASE \
                         WHEN COALESCE($3, observation_id) IS NOT NULL THEN 'observation' \
                         ELSE 'source_document' END, \
                     status = 'resolved', \
                     unresolved_reason = NULL \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(source_id)
            .bind(observation_id)
            .bind(source_url)
            .bind(content_hash)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO warning_evidence \
                     (warning_id, source_id, observation_id, source_url, content_hash, \
                      evidence_kind, status) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'resolved')",
            )
            .bind(warning_id)
            .bind(source_id)
            .bind(observation_id)
            .bind(source_url)
            .bind(content_hash)
            .bind(evidence_kind)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// Record an `unresolved` link for a URL whose source acquisition has not
/// succeeded. Idempotent per `(warning_id, source_url)`; it never overwrites a
/// resolved link for the same URL.
async fn upsert_unresolved_link(
    conn: &mut sqlx::PgConnection,
    warning_id: Uuid,
    source_url: &str,
    reason: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO warning_evidence \
             (warning_id, source_url, evidence_kind, status, unresolved_reason) \
         VALUES ($1, $2, 'source_document', 'unresolved', $3) \
         ON CONFLICT (warning_id, source_url) WHERE source_url IS NOT NULL DO UPDATE SET \
             unresolved_reason = EXCLUDED.unresolved_reason \
         WHERE warning_evidence.status <> 'resolved'",
    )
    .bind(warning_id)
    .bind(source_url)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Ensure the warning has truthful `warning_evidence` links.
///
/// Explicit references (real observations / fetched documents the producer
/// already holds) are linked first. Each `source_urls` entry is then resolved
/// against objects that actually exist: a real observation extracted from the
/// URL, or a fetched `sources` document for the URL. Anything else is recorded
/// as `unresolved` — **no** observation is synthesized from the warning text
/// and no timestamp is invented.
///
/// Runs on any connection so callers can wrap it in the warning-upsert
/// transaction. Idempotent: re-running re-affirms existing links, a resolved
/// link is never downgraded to unresolved, and no fabricated rows are written.
pub async fn link_warning_evidence_on(
    conn: &mut sqlx::PgConnection,
    warning_id: Uuid,
    explicit_refs: &[WarningEvidenceRef],
    source_urls: &[String],
) -> Result<usize> {
    let mut linked = 0usize;

    for evidence_ref in explicit_refs {
        match evidence_ref {
            WarningEvidenceRef::Observation(observation_id) => {
                // The reference must point at a real row; a dangling id must
                // not be recorded as evidence.
                let exists: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                    "SELECT provenance->>'url', provenance->>'content_hash' \
                     FROM observations WHERE id = $1",
                )
                .bind(observation_id)
                .fetch_optional(&mut *conn)
                .await?;
                if let Some((url, content_hash)) = exists {
                    upsert_resolved_link(
                        conn,
                        warning_id,
                        None,
                        Some(*observation_id),
                        url.as_deref(),
                        content_hash.as_deref(),
                        "observation",
                    )
                    .await?;
                    linked += 1;
                }
            }
            WarningEvidenceRef::SourceDocument(source_id) => {
                let exists: Option<(String, Option<String>)> =
                    sqlx::query_as("SELECT url, content_hash FROM sources WHERE id = $1")
                        .bind(source_id)
                        .fetch_optional(&mut *conn)
                        .await?;
                if let Some((url, content_hash)) = exists {
                    upsert_resolved_link(
                        conn,
                        warning_id,
                        Some(*source_id),
                        None,
                        Some(url.as_str()),
                        content_hash.as_deref(),
                        "source_document",
                    )
                    .await?;
                    linked += 1;
                }
            }
        }
    }

    for source_url in source_urls {
        let source_url = source_url.trim();
        if source_url.is_empty() {
            continue;
        }

        // 1. A real observation extracted from this URL is the strongest link.
        if let Some((observation_id, content_hash)) =
            observation_for_source_url(conn, source_url).await?
        {
            upsert_resolved_link(
                conn,
                warning_id,
                None,
                Some(observation_id),
                Some(source_url),
                content_hash.as_deref(),
                "observation",
            )
            .await?;
            linked += 1;
            continue;
        }

        // 2. A fetched source document with real content is document-level
        //    evidence (content hash of the fetched body).
        if let Some((source_id, content_hash)) = fetched_source_document(conn, source_url).await? {
            upsert_resolved_link(
                conn,
                warning_id,
                Some(source_id),
                None,
                Some(source_url),
                Some(content_hash.as_str()),
                "source_document",
            )
            .await?;
            linked += 1;
            continue;
        }

        // 3. Only a URL is known: record that truthfully.
        upsert_unresolved_link(
            conn,
            warning_id,
            source_url,
            "no fetched source document or extracted observation for URL",
        )
        .await?;
    }

    Ok(linked)
}

impl PgStore {
    /// Citable observation rows linked to a warning through `warning_evidence`,
    /// newest first. Only `resolved` links backed by a **real** observation are
    /// returned: a source document that has not been parsed yet is not a direct
    /// observation, and an unresolved URL is not evidence at all.
    ///
    /// This is the analysis's direct evidence set when the warning has explicit
    /// links.
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
               AND e.status = 'resolved' \
               AND e.observation_id IS NOT NULL \
             ORDER BY o.ts_utc DESC, e.created_at DESC, o.id ASC",
        )
        .bind(warning_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Persisted evidence links for a warning, with their source references and
    /// resolution status. Includes unresolved links so callers can surface
    /// "evidence not yet acquired" instead of pretending the warning is
    /// grounded.
    pub async fn list_warning_evidence(&self, warning_id: Uuid) -> Result<Vec<WarningEvidenceRow>> {
        let rows = sqlx::query_as::<_, WarningEvidenceRow>(
            "SELECT id, warning_id, source_id, observation_id, source_url, content_hash, \
                    evidence_kind, status, unresolved_reason, created_at \
             FROM warning_evidence WHERE warning_id = $1 ORDER BY created_at ASC, id ASC",
        )
        .bind(warning_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Traceability label for one URL resolved outside a warning insert (used
    /// by admin/debug surfaces): the registrable domain of a source URL.
    pub fn source_url_domain(source_url: &str) -> Option<String> {
        registrable_domain(source_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The linker's public types make a bare URL unrepresentable as evidence.
    #[test]
    fn evidence_refs_are_only_real_objects() {
        let observation = WarningEvidenceRef::Observation(Uuid::new_v4());
        let document = WarningEvidenceRef::SourceDocument(Uuid::new_v4());
        let serialized = serde_json::to_value(observation).unwrap();
        assert_eq!(serialized["kind"], "observation");
        assert!(serialized.get("id").is_some());
        assert_ne!(
            serde_json::to_value(document).unwrap()["kind"],
            serde_json::to_value(observation).unwrap()["kind"]
        );
    }

    /// There is deliberately no content-hash helper over warning text: the
    /// only acceptable hash is one recorded on the fetched source or the real
    /// observation.
    #[test]
    fn module_exposes_no_warning_text_hash() {
        let source = include_str!("warning_evidence.rs");
        // Needles are assembled so this test's own source cannot satisfy its
        // assertions.
        let content_hash_helper = ["warning_evidence", "content_hash"].join("_");
        let citation_type = ["WARNING", "SOURCE", "CITATION", "OBSERVATION", "TYPE"].join("_");
        assert!(
            !source.contains(&content_hash_helper),
            "hashing warning title/description as 'content' is fabrication"
        );
        assert!(
            !source.contains(&citation_type),
            "the synthetic citation observation type must not exist"
        );
    }
}
