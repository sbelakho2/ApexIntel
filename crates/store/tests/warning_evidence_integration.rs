//! Opt-in integration test for explicit warning evidence links (audit
//! warning-evidence item, migration 082).
//!
//! Proves the upstream evidence model against a real database:
//!   * a warning whose only provenance is a `source_url` (no entity ids) gets an
//!     idempotent `warning_evidence` row resolving to a real source document
//!     and a citable observation, with the content hash preserved;
//!   * the linked observation is what analysis reads as direct evidence;
//!   * recurrence/merge re-affirms existing links instead of duplicating them;
//!   * deleting the observation cascades the link (retention-safe).
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{
    warning_evidence_content_hash, PgStore, WARNING_SOURCE_CITATION_OBSERVATION_TYPE,
};
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

async fn cleanup(pool: &PgPool, warning_id: Uuid, urls: &[&str]) {
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(pool)
        .await
        .expect("delete warning cascades evidence links");
    for url in urls {
        sqlx::query("DELETE FROM observations WHERE provenance->>'source_url' = $1")
            .bind(url)
            .execute(pool)
            .await
            .expect("delete linked observations");
        sqlx::query("DELETE FROM sources WHERE url = $1")
            .bind(url)
            .execute(pool)
            .await
            .expect("delete source documents");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn source_url_only_warning_gets_linked_citable_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };

    let url = format!("https://evidence.example.com/primary/{}", Uuid::new_v4());
    let title = format!("Source-only warning {}", Uuid::new_v4());
    let description = Some("Primary document reports a component shortage");
    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            description,
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
    assert_eq!(links.len(), 1, "one link per source URL");
    let link = &links[0];
    assert_eq!(link.source_url, url);
    assert!(link.source_id.is_some(), "source document resolved");
    assert_eq!(
        link.content_hash,
        warning_evidence_content_hash(&title, description, &url),
        "content hash must be preserved"
    );

    let hash: String = sqlx::query_scalar("SELECT content_hash FROM sources WHERE id = $1")
        .bind(link.source_id.unwrap())
        .fetch_one(&pool)
        .await
        .expect("source document row");
    assert_eq!(hash, link.content_hash);

    // The linked observation is a real citable row, even though the warning
    // has no entity ids at all.
    let observations = store
        .list_warning_evidence_observations(outcome.id)
        .await
        .expect("linked observations");
    assert_eq!(observations.len(), 1);
    let observation = &observations[0];
    assert_eq!(observation.id, link.observation_id);
    assert_eq!(
        observation.observation_type,
        WARNING_SOURCE_CITATION_OBSERVATION_TYPE
    );
    assert!(observation.entity_id.is_none());
    assert_eq!(
        observation
            .provenance
            .get("content_hash")
            .and_then(|value| value.as_str()),
        Some(link.content_hash.as_str())
    );
    assert_eq!(
        observation
            .value
            .get("title")
            .and_then(|value| value.as_str()),
        Some(title.as_str())
    );

    // Recurrence merges into the same warning and re-affirms (never duplicates)
    // the existing link.
    let repeat = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            description,
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
    assert!(!repeat.created, "identical warning must deduplicate");
    assert_eq!(repeat.id, outcome.id);
    let after = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links after merge");
    assert_eq!(after.len(), 1, "merge must not duplicate evidence links");

    // A merged-in second URL appends exactly one new link.
    let second_url = format!("https://evidence.example.com/secondary/{}", Uuid::new_v4());
    let merged = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            description,
            "high",
            None,
            None,
            None,
            Some(vec![second_url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("merge second url");
    assert!(!merged.created);
    assert_eq!(merged.id, outcome.id);
    let merged_links = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links after second url");
    assert_eq!(merged_links.len(), 2, "each source URL gets one link");

    cleanup(&pool, outcome.id, &[&url, &second_url]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn deleting_a_linked_observation_cascades_instead_of_blocking() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };

    let url = format!("https://evidence.example.com/retention/{}", Uuid::new_v4());
    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &format!("Retention warning {}", Uuid::new_v4()),
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
    let observation_id = links[0].observation_id;

    // Retention cleanup deletes observations; the link must cascade, not block.
    sqlx::query("DELETE FROM observations WHERE id = $1")
        .bind(observation_id)
        .execute(&pool)
        .await
        .expect("deleting cited observation must not be blocked");
    let remaining = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("links after delete");
    assert!(remaining.is_empty(), "link cascades with its observation");

    cleanup(&pool, outcome.id, &[&url]).await;
    pool.close().await;
}
