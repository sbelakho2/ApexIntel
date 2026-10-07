//! Opt-in integration test for the truthful warning evidence chain
//! (audit P0-5/P0-6, migrations 083 + 085).
//!
//! Proves against a real database that a warning's source URLs resolve only
//! against objects that actually exist:
//!
//!   * a URL with a real extracted observation links that observation
//!     (`evidence_kind = observation`) with the observation's own content hash;
//!   * a URL with only a fetched `sources` document links the document
//!     (`evidence_kind = source_document`) with the document's body hash and
//!     yields no "direct observation" for analysis;
//!   * a URL with neither is recorded `unresolved` — no observation is
//!     synthesized from the warning text, no hash is invented;
//!   * explicit `WarningEvidenceRef` objects link real rows and dangling ids
//!     are refused;
//!   * recurrence/merge re-affirms links instead of duplicating them, and a
//!     resolved link is never downgraded.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{PgStore, WarningEvidenceRef};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

async fn connect() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres")
}

async fn migrate(pool: &PgPool) {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .expect("migrations apply");
}

/// A real fetched source document with the hash of its body.
async fn insert_source_document(pool: &PgPool, url: &str, body_hash: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sources (url, source_kind, content_hash, excerpt, fetched_at, metadata) \
         VALUES ($1, 'web_page', $2, 'fetched body excerpt', now(), '{}'::jsonb) \
         RETURNING id",
    )
    .bind(url)
    .bind(body_hash)
    .fetch_one(pool)
    .await
    .expect("insert source document")
}

/// A real extracted observation whose provenance records the source URL.
async fn insert_observation(pool: &PgPool, url: &str, content_hash: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    let provenance = serde_json::json!({
        "source": "web_change",
        "url": url,
        "content_hash": content_hash,
    });
    sqlx::query(
        "INSERT INTO observations \
             (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence, created_at) \
         VALUES ($1, 'web_change', NULL, NULL, now() - interval '3 days', $2, $3, 0.9, now())",
    )
    .bind(id)
    .bind(serde_json::json!({"change": "component shortage reported"}))
    .bind(&provenance)
    .execute(pool)
    .await
    .expect("insert observation");
    id
}

async fn cleanup(pool: &PgPool, warning_id: Uuid, urls: &[&str], observation_ids: &[Uuid]) {
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(pool)
        .await
        .expect("delete warning cascades evidence links");
    for id in observation_ids {
        sqlx::query("DELETE FROM observations WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .expect("delete observation");
    }
    for url in urls {
        sqlx::query("DELETE FROM sources WHERE url = $1")
            .bind(url)
            .execute(pool)
            .await
            .expect("delete source document");
    }
}

/// A URL with a real extracted observation links that observation; the warning
/// text itself never becomes evidence.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn real_observation_for_url_is_linked_as_direct_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    let url = format!("https://evidence.example.com/observed/{}", Uuid::new_v4());
    let doc_hash = "a".repeat(64);
    let _source_id = insert_source_document(&pool, &url, &doc_hash).await;
    let observation_id = insert_observation(&pool, &url, Some(&doc_hash)).await;

    let title = format!("Observed warning {}", Uuid::new_v4());
    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            Some("Primary document reports a component shortage"),
            "high",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("insert warning");
    assert!(outcome.created);

    let links = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("list warning evidence");
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.status, "resolved");
    assert_eq!(link.evidence_kind, "observation");
    assert_eq!(link.observation_id, Some(observation_id));
    assert_eq!(link.source_url.as_deref(), Some(url.as_str()));
    assert_eq!(
        link.content_hash.as_deref(),
        Some(doc_hash.as_str()),
        "the hash is the fetched content hash, never a hash of warning text"
    );

    // Analysis reads the real observation as direct evidence.
    let observations = store
        .list_warning_evidence_observations(outcome.id)
        .await
        .expect("linked observations");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].id, observation_id);
    assert_eq!(observations[0].observation_type, "web_change");

    // Recurrence merges and re-affirms without duplicating.
    let repeat = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            Some("Primary document reports a component shortage"),
            "high",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("repeat insert");
    assert!(!repeat.created);
    assert_eq!(repeat.id, outcome.id);
    assert_eq!(
        store
            .list_warning_evidence(outcome.id)
            .await
            .expect("links after merge")
            .len(),
        1,
        "merge must not duplicate evidence links"
    );

    cleanup(&pool, outcome.id, &[&url], &[observation_id]).await;
    pool.close().await;
}

/// A document with fetched content but no extracted observation is
/// document-level evidence: it links the source document with the body hash and
/// contributes no "direct observation" to analysis.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn fetched_document_without_observation_is_document_level_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    let url = format!("https://evidence.example.com/document/{}", Uuid::new_v4());
    let doc_hash = "b".repeat(64);
    let source_id = insert_source_document(&pool, &url, &doc_hash).await;

    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &format!("Document-only warning {}", Uuid::new_v4()),
            None,
            "medium",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            None,
            false,
        )
        .await
        .expect("insert warning");

    let links = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links");
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.status, "resolved");
    assert_eq!(link.evidence_kind, "source_document");
    assert_eq!(link.source_id, Some(source_id));
    assert_eq!(link.observation_id, None);
    assert_eq!(link.content_hash.as_deref(), Some(doc_hash.as_str()));

    assert!(
        store
            .list_warning_evidence_observations(outcome.id)
            .await
            .expect("observations")
            .is_empty(),
        "an unparsed document is not a direct observation"
    );

    cleanup(&pool, outcome.id, &[&url], &[]).await;
    pool.close().await;
}

/// A URL with neither a fetched document nor an observation is recorded as
/// unresolved: no synthesized observation, no invented hash, no `NOW()`.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn url_without_real_source_is_unresolved_and_creates_no_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    let url = format!("https://evidence.example.com/unfetched/{}", Uuid::new_v4());
    let title = format!("Unresolved warning {}", Uuid::new_v4());
    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            Some("warning text must never become evidence"),
            "high",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("insert warning");

    let links = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links");
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.status, "unresolved");
    assert!(link.source_id.is_none());
    assert!(link.observation_id.is_none());
    assert!(link.content_hash.is_none());
    assert!(
        link.unresolved_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no fetched source document")),
        "unresolved links must state why"
    );

    // Crucially: no observation was synthesized from the warning title.
    let synthesized: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM observations WHERE provenance->>'warning_id' = $1",
    )
    .bind(outcome.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count synthesized observations");
    assert_eq!(synthesized, 0, "warning text must never become evidence");

    assert!(store
        .list_warning_evidence_observations(outcome.id)
        .await
        .expect("observations")
        .is_empty());

    // Recurrence re-affirms the unresolved link (still exactly one row) and an
    // unresolved link is never downgraded/duplicated.
    let repeat = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            Some("warning text must never become evidence"),
            "high",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("repeat insert");
    assert!(!repeat.created);
    assert_eq!(
        store
            .list_warning_evidence(outcome.id)
            .await
            .expect("links after repeat")
            .len(),
        1
    );

    cleanup(&pool, outcome.id, &[&url], &[]).await;
    pool.close().await;
}

/// Explicit references link real objects directly, and dangling ids are
/// refused rather than recorded as evidence.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn explicit_refs_link_real_objects_and_refuse_dangling_ids() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    let doc_url = format!(
        "https://evidence.example.com/explicit-doc/{}",
        Uuid::new_v4()
    );
    let obs_url = format!(
        "https://evidence.example.com/explicit-obs/{}",
        Uuid::new_v4()
    );
    let doc_hash = "c".repeat(64);
    let source_id = insert_source_document(&pool, &doc_url, &doc_hash).await;
    let observation_id = insert_observation(&pool, &obs_url, Some(&doc_hash)).await;

    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &format!("Explicit evidence {}", Uuid::new_v4()),
            None,
            "medium",
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .await
        .expect("insert warning");

    let mut conn = pool.acquire().await.expect("acquire connection");
    let linked = apex_store::postgres::link_warning_evidence_on(
        &mut conn,
        outcome.id,
        &[
            WarningEvidenceRef::Observation(observation_id),
            WarningEvidenceRef::SourceDocument(source_id),
            WarningEvidenceRef::Observation(Uuid::new_v4()),
            WarningEvidenceRef::SourceDocument(Uuid::new_v4()),
        ],
        &[],
    )
    .await
    .expect("link explicit evidence");
    assert_eq!(linked, 2, "only real references are linked");
    drop(conn);

    let links = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links");
    assert_eq!(links.len(), 2);
    assert!(links
        .iter()
        .any(|link| link.observation_id == Some(observation_id)
            && link.evidence_kind == "observation"));
    assert!(links
        .iter()
        .any(|link| link.source_id == Some(source_id) && link.evidence_kind == "source_document"));

    cleanup(&pool, outcome.id, &[&doc_url, &obs_url], &[observation_id]).await;
    pool.close().await;
}

/// Bulk dismissal acknowledges every unacknowledged warning, leaves
/// already-acknowledged rows alone, and never fabricates a review verdict
/// (review_outcome is untouched — dismissing the board is not a
/// false-positive call on every warning).
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn acknowledge_all_open_warnings_transitions_only_unacknowledged_rows() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());
    let open_a = Uuid::new_v4();
    let open_b = Uuid::new_v4();
    let already = Uuid::new_v4();
    for (id, acknowledged) in [(open_a, false), (open_b, false), (already, true)] {
        sqlx::query(
            "INSERT INTO warnings (id, warning_type, severity, title, is_system_broadcast,
                                   acknowledged, review_outcome)
             VALUES ($1, 'bulk_ack_test', 'low', $2, false, $3, 'true_positive')",
        )
        .bind(id)
        .bind(format!("bulk ack fixture {id}"))
        .bind(acknowledged)
        .execute(&pool)
        .await
        .expect("insert warning fixture");
    }

    let transitioned = store
        .acknowledge_all_open_warnings("tester", "bulk dismissal test")
        .await
        .expect("bulk acknowledge");
    assert!(transitioned >= 2, "expected at least the two open fixtures");

    let rows = sqlx::query_as::<_, (Uuid, bool, Option<String>)>(
        "SELECT id, acknowledged, review_outcome::text FROM warnings WHERE id = ANY($1)",
    )
    .bind(vec![open_a, open_b, already])
    .fetch_all(&pool)
    .await
    .expect("load fixtures");
    assert_eq!(rows.len(), 3);
    for (_, acknowledged, review_outcome) in rows {
        assert!(
            acknowledged,
            "every fixture is acknowledged after bulk dismissal"
        );
        assert_eq!(
            review_outcome.as_deref(),
            Some("true_positive"),
            "bulk dismissal must not overwrite review verdicts"
        );
    }

    sqlx::query("DELETE FROM warnings WHERE id = ANY($1)")
        .bind(vec![open_a, open_b, already])
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}
