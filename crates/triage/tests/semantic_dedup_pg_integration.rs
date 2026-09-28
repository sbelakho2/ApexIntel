//! Opt-in integration test proving semantic dedup state is persistent
//! (audit P0 #9).
//!
//! The unit suite uses the in-memory store; this test runs against a real
//! PostgreSQL with `vector` and `pg_trgm` and asserts that items stored by one
//! [`PgSemanticDedupStore`] instance are found by a brand-new instance over a
//! brand-new pool — the "survives a worker restart" property the in-memory
//! fallback never had. It also proves the recorded dedup state round-trips.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::triage::TriageItemType;
use apex_store::postgres::{PgStore, SemanticDedupBackend, SemanticDedupStatus};
use apex_triage::semantic_dedup::{DedupStore, PgSemanticDedupStore};
use sqlx::postgres::PgPoolOptions;

async fn connect() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres")
}

fn unit_vector() -> Vec<f64> {
    // 384-dimensional basis vector matching the production embedding width
    // (bge-small-en-v1.5, see 033_embedding_dimension_384.sql).
    let mut vector = vec![0.0f64; 384];
    vector[0] = 1.0;
    vector
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn dedup_items_survive_a_process_restart() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");

    let item_type = TriageItemType::Warning;
    let item_id = format!("restart-{}", uuid::Uuid::new_v4());
    let title = "Foxconn quality crisis";
    let text = "Quality defect recall at Foxconn Tunisia manufacturing plant";

    // ── First "worker process": store the item with its embedding.
    {
        let store = PgSemanticDedupStore::new(pool.clone());
        store
            .store_item_with_vector(&item_type, &item_id, title, text, Some(&unit_vector()))
            .await
            .expect("store item with vector");
    }

    // ── "Restart": a brand-new store over a brand-new pool. Nothing is
    // re-stored; the item must still be found.
    let restarted_pool = connect().await;
    let restarted = PgSemanticDedupStore::new(restarted_pool.clone());

    let vector_hits = restarted
        .find_similar_by_vector(&item_type, &unit_vector(), 5)
        .await
        .expect("vector lookup after restart");
    let vector_hit = vector_hits
        .iter()
        .find(|hit| hit.id == item_id)
        .expect("item stored before the restart must be found by vector search");
    assert!(
        vector_hit.similarity > 0.99,
        "identical vector must score ~1.0, got {}",
        vector_hit.similarity
    );

    let text_hits = restarted
        .find_similar(&item_type, text, 5)
        .await
        .expect("text lookup after restart");
    assert!(
        text_hits.iter().any(|hit| hit.id == item_id),
        "item stored before the restart must be found by text similarity"
    );

    // A different item type must not match (partitioning by item_type).
    let other_type_hits = restarted
        .find_similar_by_vector(&TriageItemType::Insight, &unit_vector(), 5)
        .await
        .expect("cross-type lookup");
    assert!(
        !other_type_hits.iter().any(|hit| hit.id == item_id),
        "items must be scoped to their item type"
    );

    sqlx::query("DELETE FROM semantic_dedup_items WHERE item_type = $1 AND item_id = $2")
        .bind(item_type.as_str())
        .bind(&item_id)
        .execute(&restarted_pool)
        .await
        .expect("cleanup");
    restarted_pool.close().await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn recorded_backend_state_round_trips_and_rejects_unknown_backends() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let store = PgStore { pool: pool.clone() };

    store
        .record_semantic_dedup_state(
            SemanticDedupBackend::PgVector,
            SemanticDedupStatus::Ok,
            Some("integration test"),
        )
        .await
        .expect("record state");

    let state = store
        .get_semantic_dedup_state()
        .await
        .expect("read state")
        .expect("state row exists after recording");
    assert_eq!(state.backend, SemanticDedupBackend::PgVector);
    assert_eq!(state.status, SemanticDedupStatus::Ok);
    assert_eq!(state.detail.as_deref(), Some("integration test"));

    // An unknown backend string parses to the degraded memory fallback rather
    // than assuming persistence is active.
    sqlx::query("UPDATE semantic_dedup_state SET backend = 'memory', status = 'degraded' WHERE id")
        .execute(&pool)
        .await
        .expect("reset state");
    let state = store
        .get_semantic_dedup_state()
        .await
        .expect("read state")
        .expect("state row exists");
    assert_eq!(state.backend, SemanticDedupBackend::Memory);
    assert_eq!(state.status, SemanticDedupStatus::Degraded);

    // The CHECK constraint only allows the two real backends.
    let bogus = sqlx::query("UPDATE semantic_dedup_state SET backend = 'nonsense' WHERE id")
        .execute(&pool)
        .await;
    assert!(bogus.is_err(), "unknown backend must be rejected by the DB");

    pool.close().await;
}
