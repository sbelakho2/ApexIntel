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
