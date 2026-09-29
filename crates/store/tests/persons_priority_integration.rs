//! Opt-in integration test for the canonical POI priority contract (audit P0).
//!
//! Priority is the weighted composite of the stored five-dimension
//! `priority_vector`, mirrored into the nullable `persons.priority_score`
//! column; influence is a separate measurement. Filtering and ordering by
//! "priority" must use the canonical composite, and an unmeasured priority is
//! `NULL` — never a zero score and never derived from influence.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service through `scripts/ci/run_pg_integration_suites.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::entities::{Person, RoleFamily};
use apex_store::postgres::{PersonListFilters, PersonOrderBy, PgStore};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

async fn setup() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    pool
}

/// Distinct per-dimension values, so the assertion pins the canonical weights
/// and dimension order rather than being invariant to re-weighting.
fn full_vector() -> serde_json::Value {
    serde_json::json!({
        "decision_power": 0.8,
        "domain_relevance": 0.4,
        "network_centrality": 0.6,
        "engagement_potential": 0.2,
        "intelligence_value": 1.0,
    })
}

fn expected_priority_score() -> f64 {
    apex_core::priority::priority_score_from_json(&full_vector())
        .expect("canonical vector parses in the single Rust implementation")
}

async fn insert_person(store: &PgStore, name: &str, influence: f64) -> Uuid {
    let mut person = Person::new(name, RoleFamily::Procurement);
    person.influence_score = influence;
    store.insert_person(&person).await.expect("insert person");
    person.id
}

async fn stored_priority_score(pool: &PgPool, id: Uuid) -> Option<f64> {
    sqlx::query_scalar("SELECT priority_score FROM persons WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read priority_score")
}

/// The SQL mirror computes the same composite as `apex_core::priority`, and a
/// missing/partial vector stays `NULL`.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn priority_score_is_synced_from_full_vectors_only() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let prefix = format!("PRIO{}", Uuid::new_v4().simple());

    let full_name = format!("{prefix} Full");
    let legacy_name = format!("{prefix} Legacy");
    let partial_name = format!("{prefix} Partial");

    let full_id = insert_person(&store, &full_name, 0.95).await;
    let legacy_id = insert_person(&store, &legacy_name, 0.95).await;
    let partial_id = insert_person(&store, &partial_name, 0.95).await;

    store
        .update_person_psych_profile(
            full_id,
            Some(&full_vector().to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("write full priority vector");
    // The legacy six-dimension vector is not the canonical model.
    store
        .update_person_psych_profile(
            legacy_id,
            Some(&serde_json::json!({"cost": 0.5, "quality": 0.5}).to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("write legacy vector");
    store
        .update_person_psych_profile(
            partial_id,
            Some(&serde_json::json!({"decision_power": 1.0}).to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("write partial vector");

    let full_score = stored_priority_score(&pool, full_id)
        .await
        .expect("full vector must produce a canonical composite");
    let expected = expected_priority_score();
    assert!(
        (full_score - expected).abs() < 1e-9,
        "the stored score must equal the canonical Rust composite {expected}, got {full_score}"
    );
    assert_eq!(
        stored_priority_score(&pool, legacy_id).await,
        None,
        "a non-canonical vector is unmeasured, not zero"
    );
    assert_eq!(
        stored_priority_score(&pool, partial_id).await,
        None,
        "a partial vector is unmeasured, not zero-padded"
    );

    // The engagement guide reads the same canonical composite — never the
    // influence measurement, never a zero-filled default.
    let engagement = store
        .get_person_engagement(full_id)
        .await
        .expect("engagement query")
        .expect("person exists");
    let engagement_score = engagement
        .priority_score
        .expect("full vector yields a priority score");
    assert!((engagement_score - expected).abs() < 1e-9);
    let legacy_engagement = store
        .get_person_engagement(legacy_id)
        .await
        .expect("engagement query")
        .expect("person exists");
    assert_eq!(
        legacy_engagement.priority_score, None,
        "no canonical vector means unmeasured priority, not influence"
    );

    // Replacing a canonical vector with a non-canonical one re-synchronizes the
    // score to NULL instead of leaving a stale composite behind.
    store
        .update_person_psych_profile(
            full_id,
            Some(&serde_json::json!({"cost": 0.5}).to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("overwrite with legacy vector");
    assert_eq!(
        stored_priority_score(&pool, full_id).await,
        None,
        "the score must mirror the stored vector, never a stale value"
    );
    // Restore the canonical vector so the filter/ordering test data is intact.
    store
        .update_person_psych_profile(
            full_id,
            Some(&full_vector().to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("restore canonical vector");

    for id in [full_id, legacy_id, partial_id] {
        sqlx::query("DELETE FROM persons WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}

/// Priority filters and ordering use `priority_score`; unmeasured priority
/// never passes a numeric range and always sorts last.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn priority_filters_and_ordering_use_the_canonical_score() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let prefix = format!("PRIOORD{}", Uuid::new_v4().simple());

    let measured_name = format!("{prefix} Measured");
    let unmeasured_name = format!("{prefix} Unmeasured");

    let measured_id = insert_person(&store, &measured_name, 0.95).await;
    let unmeasured_id = insert_person(&store, &unmeasured_name, 0.95).await;
    store
        .update_person_psych_profile(
            measured_id,
            Some(&full_vector().to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("write full priority vector");

    let with_min = PersonListFilters {
        search: Some(prefix.clone()),
        min_priority: Some(0.5),
        ..Default::default()
    };
    let rows = store
        .list_persons(&with_min, Some(PersonOrderBy::Priority), true, 50, 0)
        .await
        .expect("list with min priority");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, measured_id);

    // An unmeasured row must not pass a numeric range — even `>= 0.0`.
    let zero_floor = PersonListFilters {
        search: Some(prefix.clone()),
        min_priority: Some(0.0),
        ..Default::default()
    };
    let rows = store
        .list_persons(&zero_floor, None, true, 50, 0)
        .await
        .expect("list with zero floor");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, measured_id);

    // Ordering: measured first, unmeasured last, in both directions.
    let all = PersonListFilters {
        search: Some(prefix.clone()),
        ..Default::default()
    };
    for desc in [true, false] {
        let rows = store
            .list_persons(&all, Some(PersonOrderBy::Priority), desc, 50, 0)
            .await
            .expect("list ordered by priority");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].id, measured_id,
            "measured priority must lead (desc={desc})"
        );
        assert_eq!(
            rows[1].id, unmeasured_id,
            "unmeasured priority sorts last (desc={desc})"
        );
    }

    for id in [measured_id, unmeasured_id] {
        sqlx::query("DELETE FROM persons WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}
