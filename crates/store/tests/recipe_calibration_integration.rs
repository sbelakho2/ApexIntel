//! Opt-in integration test for the recipe activation-threshold calibration
//! contract (audit P0: high false-positive rates must tighten the gate, never
//! loosen it).
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service through `scripts/ci/run_pg_integration_suites.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

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
             configured_activation_threshold, activation_threshold,
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
    sqlx::query(
        "DELETE FROM audit_log WHERE event_type = 'recipe_threshold_auto_calibrated' \
         AND detail->>'recipe_code' = $1",
    )
    .bind(&code)
    .execute(&pool)
    .await
    .expect("cleanup audit rows");
}

/// Calibration is idempotent within a calendar week: the worker retries a
/// failed stage, and each retry must not multiply the threshold again.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn calibration_applies_at_most_once_per_recipe_per_week() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let code = format!("CALONCE{}", Uuid::new_v4().simple());
    let baseline = 0.50_f64;

    sqlx::query(
        "INSERT INTO recipes (
             code, name, status, category, join_type, outcome,
             signals, transforms, test_config, thresholds,
             narrative_template, action_playbook, applicability,
             configured_activation_threshold, activation_threshold,
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
    .expect("insert recipe fixture");

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

    let first = store
        .auto_calibrate_recipe_thresholds()
        .await
        .expect("first calibration runs");
    assert!(
        first.iter().any(|a| a.recipe_code == code),
        "the first calibration in the week must adjust the recipe"
    );

    let after_first: f64 =
        sqlx::query_scalar("SELECT activation_threshold FROM recipes WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .expect("read threshold after first run");

    let second = store
        .auto_calibrate_recipe_thresholds()
        .await
        .expect("second calibration runs");
    assert!(
        second.iter().all(|a| a.recipe_code != code),
        "a retry inside the same week must not recalibrate the same recipe"
    );

    let after_second: f64 =
        sqlx::query_scalar("SELECT activation_threshold FROM recipes WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .expect("read threshold after second run");
    assert!(
        (after_first - after_second).abs() < 1e-12,
        "a retry must not compound the threshold: {after_first} -> {after_second}"
    );

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
    sqlx::query(
        "DELETE FROM audit_log WHERE event_type = 'recipe_threshold_auto_calibrated' \
         AND detail->>'recipe_code' = $1",
    )
    .bind(&code)
    .execute(&pool)
    .await
    .expect("cleanup audit rows");
}

/// The canonical writer used by UI recipe creation must persist the canonical
/// columns alongside `definition`, record `created_by`, start new recipes in
/// staging and never resurrect a deprecated recipe on re-save.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn canonical_recipe_writer_persists_columns_and_author() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let code = format!("CANON{}", Uuid::new_v4().simple());
    let signals = serde_json::json!([{"kind": "webchange", "entity": "company"}]);
    let transforms = serde_json::json!([{"op": "normalize"}]);
    let thresholds = serde_json::json!({"activation_threshold": 0.62});
    let playbook = serde_json::json!([{"step": "escalate"}]);

    store
        .insert_recipe_canonical(
            &code,
            "Canonical Recipe",
            "demand",
            "company",
            "signal",
            &signals,
            &transforms,
            &thresholds,
            "Narrative {{entity}}",
            &playbook,
            "user-42",
        )
        .await
        .expect("canonical insert");

    let (
        name,
        category,
        join_type,
        outcome,
        db_signals,
        db_thresholds,
        template,
        db_playbook,
        definition_name,
        created_by,
        status,
    ): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        serde_json::Value,
        serde_json::Value,
        Option<String>,
        serde_json::Value,
        Option<String>,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT name, category, join_type, outcome, signals, thresholds, \
                narrative_template, to_jsonb(action_playbook), definition->>'name', \
                created_by, status \
           FROM recipes WHERE code = $1",
    )
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("read canonical recipe");

    assert_eq!(name, "Canonical Recipe");
    assert_eq!(category.as_deref(), Some("demand"));
    assert_eq!(join_type.as_deref(), Some("company"));
    assert_eq!(outcome.as_deref(), Some("signal"));
    assert_eq!(db_signals, signals);
    assert_eq!(db_thresholds, thresholds);
    assert_eq!(template.as_deref(), Some("Narrative {{entity}}"));
    assert_eq!(db_playbook, playbook);
    assert_eq!(
        definition_name.as_deref(),
        Some("Canonical Recipe"),
        "definition must stay in sync with the canonical columns"
    );
    assert_eq!(created_by.as_deref(), Some("user-42"));
    assert_eq!(status, "staging", "new UI recipes start in staging");

    // Re-saving must not resurrect a deprecated recipe.
    sqlx::query("UPDATE recipes SET status = 'deprecated' WHERE code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .expect("deprecate fixture");
    store
        .insert_recipe_canonical(
            &code,
            "Canonical Recipe v2",
            "demand",
            "company",
            "signal",
            &signals,
            &transforms,
            &thresholds,
            "Narrative v2",
            &playbook,
            "user-43",
        )
        .await
        .expect("canonical re-save");
    let (status_after, name_after, author_after): (String, String, Option<String>) =
        sqlx::query_as("SELECT status, name, created_by FROM recipes WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .expect("read re-saved recipe");
    assert_eq!(
        status_after, "deprecated",
        "a re-save must not change lifecycle status"
    );
    assert_eq!(name_after, "Canonical Recipe v2");
    assert_eq!(author_after.as_deref(), Some("user-43"));

    sqlx::query("DELETE FROM recipes WHERE code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .expect("cleanup recipe");
}
