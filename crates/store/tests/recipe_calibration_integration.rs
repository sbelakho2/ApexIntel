//! Opt-in integration test for the recipe activation-threshold calibration
//! contract (audit P0: high false-positive rates must tighten the gate, never
//! loosen it).
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service through `scripts/ci/run_pg_integration_suites.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::PgStore;
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

/// A 40% false-positive month must move the threshold UP, not down.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn high_false_positive_rate_tightens_the_activation_threshold() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let code = format!("CAL{}", Uuid::new_v4().simple());
    let baseline = 0.80_f64;

    // Minimal recipe row: only the columns the calibrator reads plus the
    // schema's required defaults.
    sqlx::query(
        "INSERT INTO recipes (
             code, name, status, category, join_type, outcome,
             signals, transforms, test_config, thresholds,
             narrative_template, action_playbook, applicability,
             configured_min_precision, activation_threshold,
             created_at, updated_at
         ) VALUES (
             $1, $1, 'production', 'demand', 'company', 'signal',
             '[]'::jsonb, '[]'::jsonb, '{}'::jsonb, '{}'::jsonb,
             'template', '[]'::jsonb, '{}'::jsonb,
             $2, $2, NOW(), NOW()
         )
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(&code)
    .bind(baseline)
    .execute(&pool)
    .await
    .expect("insert calibration fixture recipe");

    // A reviewed month with a 40% false-positive rate.
    sqlx::query(
        "INSERT INTO recipe_weekly_metrics (
             recipe_code, week_start, warnings_generated,
             reviewed_warnings, false_positive_warnings,
             precision_score, false_positive_rate
         ) VALUES (
             $1, DATE_TRUNC('week', NOW() - INTERVAL '1 week')::DATE,
             50, 10, 4, 0.6, 0.4
         )",
    )
    .bind(&code)
    .execute(&pool)
    .await
    .expect("insert weekly metrics fixture");

    let adjustments = store
        .auto_calibrate_recipe_thresholds()
        .await
        .expect("calibration runs");
    assert!(
        adjustments
            .iter()
            .any(|adjustment| adjustment.recipe_code == code),
        "the noisy recipe must be calibrated"
    );

    let (new_threshold,): (f64,) =
        sqlx::query_as("SELECT activation_threshold FROM recipes WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .expect("read calibrated threshold");

    assert!(
        new_threshold >= baseline,
        "high FPR must tighten the gate: {new_threshold} < {baseline}"
    );
    // 0.80 * (1 + 0.5 * 0.40) = 0.96
    assert!((new_threshold - 0.96).abs() < 1e-9, "got {new_threshold}");

    // Cleanup.
    sqlx::query("DELETE FROM recipe_weekly_metrics WHERE recipe_code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .expect("cleanup metrics");
    sqlx::query("DELETE FROM recipes WHERE code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .expect("cleanup recipe");
}
