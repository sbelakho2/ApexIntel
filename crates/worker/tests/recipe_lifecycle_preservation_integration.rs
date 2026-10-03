//! Opt-in integration test: startup seed bootstrap must never overwrite
//! authoritative recipe lifecycle state (audit P1).
//!
//! The recipes table is shared by the YAML seed bootstrap (idempotent inserts
//! on worker start) and the database lifecycle (staging / production /
//! deprecated / calibrated thresholds). Now that the database is
//! authoritative, a seed re-run must not reset a deprecated recipe back to
//! seed, nor clobber a calibrated activation threshold.
//!
//! `#[ignore]`d by default; CI runs it through
//! `scripts/ci/run_pg_integration_suites.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_worker::recipe_loader::SeedRecipe;
use sqlx::postgres::PgPoolOptions;

async fn setup() -> sqlx::PgPool {
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

fn seed(id: &str) -> SeedRecipe {
    SeedRecipe {
        id: id.to_string(),
        name: format!("Seed {id}"),
        category: "demand".to_string(),
        join: vec!["company".to_string()],
        outcome: "signal".to_string(),
        signals: vec![serde_yaml::Value::String("count.company".to_string())],
        transforms: vec![],
        test: serde_yaml::Value::Null,
        thresholds: serde_yaml::Value::Null,
        narrative_template: "note".to_string(),
        action_playbook: vec!["act".to_string()],
        applicability: serde_yaml::Value::Null,
    }
}

async fn insert_fixture(pool: &sqlx::PgPool, code: &str, status: &str, threshold: f64) {
    sqlx::query(
        "INSERT INTO recipes (
             code, name, status, category, join_type, outcome,
             signals, transforms, test_config, thresholds,
             narrative_template, action_playbook, applicability,
             configured_activation_threshold, activation_threshold,
             created_at, updated_at
         ) VALUES (
             $1, $1, $2, 'demand', 'company', 'signal',
             '[]'::jsonb, '[]'::jsonb, '{}'::jsonb, '{}'::jsonb,
             'template', '[]'::jsonb, '{}'::jsonb,
             $3, $3, NOW(), NOW()
         )
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(code)
    .bind(status)
    .bind(threshold)
    .execute(pool)
    .await
    .expect("insert lifecycle fixture");
}

async fn read_state(pool: &sqlx::PgPool, code: &str) -> (String, Option<f64>) {
    sqlx::query_as("SELECT status, activation_threshold FROM recipes WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("read recipe state")
}

/// #158: the lifecycle decisions the weekly promotion board / deprecation
/// audit derive must actually change the persisted `recipes.status` — the
/// promotion used to only compute and log its decision, leaving the row in
/// `staging` forever and a deprecated recipe still firing.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn promotion_and_deprecation_change_the_persisted_status() {
    let pool = setup().await;
    let store = apex_store::postgres::PgStore::from_pool(pool.clone());

    let code = format!("LIF{}", uuid::Uuid::new_v4().simple());
    insert_fixture(&pool, &code, "staging", 0.8).await;

    assert!(
        store.promote_recipe(&code).await.expect("promote_recipe"),
        "a staging recipe must be promoted"
    );
    let (status, _) = read_state(&pool, &code).await;
    assert_eq!(
        status, "production",
        "promotion must change the persisted status"
    );
    // Idempotent: a second promotion changes no row.
    assert!(
        !store.promote_recipe(&code).await.expect("second promotion"),
        "promoting an already-promoted recipe must report no row changed"
    );

    assert!(
        store
            .deprecate_recipe(&code)
            .await
            .expect("deprecate_recipe"),
        "a production recipe must be deprecated"
    );
    let (status, _) = read_state(&pool, &code).await;
    assert_eq!(
        status, "deprecated",
        "deprecation must change the persisted status"
    );
    assert!(
        !store
            .deprecate_recipe(&code)
            .await
            .expect("second deprecation"),
        "deprecating an already-deprecated recipe must report no row changed"
    );

    sqlx::query("DELETE FROM recipes WHERE code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .expect("cleanup");
}

/// The seed bootstrap is idempotent and lifecycle-preserving.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn seed_bootstrap_preserves_authoritative_lifecycle() {
    let pool = setup().await;

    let deprecated_code = format!("DEP{}", uuid::Uuid::new_v4().simple());
    let promoted_code = format!("PRO{}", uuid::Uuid::new_v4().simple());

    // Authoritative state before the bootstrap runs.
    insert_fixture(&pool, &deprecated_code, "deprecated", 0.8).await;
    insert_fixture(&pool, &promoted_code, "production", 0.96).await;

    // A worker start re-runs the YAML seed inserts for the same codes.
    let seeds = vec![seed(&deprecated_code), seed(&promoted_code)];
    apex_worker::recipe_loader::insert_seed_recipes(&pool, &seeds)
        .await
        .expect("seed bootstrap runs");

    let (deprecated_status, deprecated_threshold) = read_state(&pool, &deprecated_code).await;
    assert_eq!(
        deprecated_status, "deprecated",
        "a seed re-run must not resurrect a deprecated recipe"
    );
    assert_eq!(deprecated_threshold, Some(0.8));

    let (promoted_status, promoted_threshold) = read_state(&pool, &promoted_code).await;
    assert_eq!(
        promoted_status, "production",
        "a seed re-run must not demote a production recipe"
    );
    assert_eq!(
        promoted_threshold,
        Some(0.96),
        "a seed re-run must not clobber a calibrated activation threshold"
    );

    for code in [&deprecated_code, &promoted_code] {
        sqlx::query("DELETE FROM recipes WHERE code = $1")
            .bind(code)
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}
