//! Opt-in integration test that applies the embedded migrations to a real
//! PostgreSQL database and asserts the schema the application depends on
//! exists.
//!
//! The unit suite uses in-memory fakes and never touches a database, so this
//! is the only test that proves a fresh deployment can bootstrap. It is
//! `#[ignore]`d by default (CI runs it with `--ignored` against a Postgres
//! service) and reads `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use sqlx::{postgres::PgPoolOptions, Row};

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

async fn table_exists(pool: &sqlx::PgPool, table: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = $1)",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn column_exists(pool: &sqlx::PgPool, table: &str, column: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn migrations_apply_cleanly_to_a_fresh_database() {
    let pool = connect().await;

    // Applying the embedded migrations must succeed on an empty database and
    // be idempotent (a second run is a no-op).
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply to a fresh database");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations are idempotent");

    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        applied >= 40,
        "expected >= 40 applied migrations, got {applied}"
    );

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn core_and_feature_tables_exist() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

    for table in [
        "companies",
        "persons",
        "observations",
        "warnings",
        "insights",
        "recipes",
        "sources",
        "triage_queue",
        "graph_edges",
        "engagement_profiles",
        "competitor_pricing",
        "contact_methods",
        "engagement_events",
        "buying_centers",
        "buying_center_members",
        "crawl_metrics",
        "closed_deals",
        "crm_sync_state",
        "trend_rollups",
        "user_alert_subscriptions",
        "source_runtime_state",
    ] {
        assert!(table_exists(&pool, table).await, "missing table {table}");
    }

    // Columns introduced by later migrations that the code now reads/writes.
    for (table, column) in [
        ("engagement_profiles", "metadata"),  // 044
        ("companies", "tech_stack"),          // 043
        ("companies", "intent_signal_score"), // 043
        ("companies", "funding_stage"),
        ("companies", "headcount_growth_pct"),
        ("closed_deals", "won"),
        ("buying_center_members", "influence_score"),
    ] {
        assert!(
            column_exists(&pool, table, column).await,
            "missing column {table}.{column}"
        );
    }

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn freshly_migrated_schema_contains_no_fixture_intelligence() {
    // migration 004 inserted deterministic demo rows into intelligence tables;
    // migration 046 removes them. A fresh migration run must contain none.
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

    for (table, prefix) in [
        (
            "strategic_opportunities",
            "11111111-1111-1111-1111-1111111111",
        ),
        ("critical_threats", "22222222-2222-2222-2222-2222222222"),
        (
            "investigation_workspaces",
            "33333333-3333-3333-3333-3333333333",
        ),
        ("priority_queue", "44444444-4444-4444-4444-4444444444"),
        ("supplier_risk", "55555555-5555-5555-5555-5555555555"),
        (
            "pipeline_opportunities",
            "66666666-6666-6666-6666-6666666666",
        ),
        ("team_assignments", "77777777-7777-7777-7777-7777777777"),
        ("source_evidence", "88888888-8888-8888-8888-8888888888"),
    ] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE id::text LIKE $1 || '%'"
        ))
        .bind(prefix)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "{table} still contains {count} fixture row(s)");
    }

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn sales_layer_inserts_round_trip() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

    // Prove the types the store layer binds actually round-trip (this is where
    // the f32-vs-float8 defect class would have been caught).
    let company_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO companies (id, name, is_competitor) VALUES ($1, $2, FALSE) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(company_id)
    .bind("Audit Co")
    .execute(&pool)
    .await
    .expect("insert company");

    // A person must exist for the contact-method FK.
    let person_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO persons (id, name) VALUES ($1, $2)")
        .bind(person_id)
        .bind("Audit Person")
        .execute(&pool)
        .await
        .expect("insert person");

    let conf: f64 = 0.42;
    sqlx::query(
        "INSERT INTO contact_methods (id, person_id, contact_type, value, confidence, verification_status, source) \
         VALUES ($1, $2, 'email', 'a@b.test', $3, 'smtp_verified', 'apollo')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(person_id)
    .bind(conf)
    .execute(&pool)
    .await
    .expect("insert contact_methods with f64 confidence");

    // The DB must reject an out-of-range confidence (the `<= 1.0` CHECK that the
    // sales layer relies on).
    let rejected = sqlx::query(
        "INSERT INTO contact_methods (id, person_id, contact_type, value, confidence, verification_status, source) \
         VALUES ($1, $2, 'email', 'bad@b.test', 1.5, 'smtp_verified', 'apollo')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(person_id)
    .execute(&pool)
    .await;
    assert!(
        rejected.is_err(),
        "out-of-range confidence should violate the CHECK constraint"
    );

    let read_back: f64 =
        sqlx::query("SELECT confidence FROM contact_methods WHERE value = 'a@b.test' LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get("confidence")
            .unwrap();
    assert!((read_back - conf).abs() < 1e-9);

    sqlx::query("DELETE FROM contact_methods WHERE value IN ('a@b.test', 'bad@b.test')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM persons WHERE id = $1")
        .bind(person_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM companies WHERE id = $1")
        .bind(company_id)
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn learning_eval_metrics_schema_round_trips() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

    assert!(table_exists(&pool, "learning_eval_sets").await);
    assert!(table_exists(&pool, "learning_eval_runs").await);
    assert!(table_exists(&pool, "learning_eval_metrics").await);

    let view_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.views \
         WHERE table_schema = 'public' AND table_name = 'learning_training_truth_metrics')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(view_exists, "training-truth view must exist");

    let set_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO learning_eval_sets (id, name, version, example_count) VALUES ($1, $2, 1, 500)",
    )
    .bind(set_id)
    .bind(format!("golden_set_{set_id}"))
    .execute(&pool)
    .await
    .expect("insert frozen evaluation set");

    // Frozen sets are append-only: updates must be rejected by the trigger.
    let update = sqlx::query("UPDATE learning_eval_sets SET example_count = 999 WHERE id = $1")
        .bind(set_id)
        .execute(&pool)
        .await;
    assert!(
        update.is_err(),
        "frozen evaluation sets must reject updates"
    );

    let run_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO learning_eval_runs (id, eval_set_id, candidate_kind, candidate_ref, metrics_version) \
         VALUES ($1, $2, 'prompt', 'insight_prompt', 1)",
    )
    .bind(run_id)
    .bind(set_id)
    .execute(&pool)
    .await
    .expect("insert evaluation run");

    // Positive confirmation is training truth.
    sqlx::query(
        "INSERT INTO learning_eval_metrics (run_id, metric, value, sample_size, signal_class, is_training_truth) \
         VALUES ($1, 'precision', 0.72, 200, 'positive_confirmation', TRUE)",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .expect("insert confirmed precision metric");

    // Workflow convenience must never be marked as training truth.
    let forged_truth = sqlx::query(
        "INSERT INTO learning_eval_metrics (run_id, metric, value, sample_size, signal_class, is_training_truth) \
         VALUES ($1, 'source_yield', 0.40, 200, 'workflow_convenience', TRUE)",
    )
    .bind(run_id)
    .execute(&pool)
    .await;
    assert!(
        forged_truth.is_err(),
        "non-confirmed analyst signals must not be stored as training truth"
    );

    // One metric per (run, metric, signal class).
    let duplicate = sqlx::query(
        "INSERT INTO learning_eval_metrics (run_id, metric, value, sample_size, signal_class) \
         VALUES ($1, 'precision', 0.70, 180, 'positive_confirmation')",
    )
    .bind(run_id)
    .execute(&pool)
    .await;
    assert!(duplicate.is_err(), "duplicate metric rows must be rejected");

    let truth_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM learning_training_truth_metrics WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(truth_rows, 1);

    // The store helper must fetch an existing frozen version on a repeat call
    // instead of tripping the freeze trigger with an UPDATE arm.
    let store = apex_store::postgres::PgStore::from_pool(pool.clone());
    let store_set_id = store
        .upsert_learning_eval_set(
            "store_round_trip_set",
            1,
            None,
            10,
            &serde_json::json!({"source": "integration_test"}),
        )
        .await
        .expect("first upsert creates the frozen set");
    let repeat_id = store
        .upsert_learning_eval_set(
            "store_round_trip_set",
            1,
            None,
            10,
            &serde_json::json!({"source": "integration_test"}),
        )
        .await
        .expect("repeat upsert fetches the existing frozen set");
    assert_eq!(store_set_id, repeat_id);

    // Cleanup: runs cascade to metrics. Frozen set rows are intentionally
    // immutable and are left behind (the CI database is ephemeral).
    sqlx::query("DELETE FROM learning_eval_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;
}
