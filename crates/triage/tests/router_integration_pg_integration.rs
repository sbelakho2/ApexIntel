//! Opt-in integration test proving the triage fallback dispatcher writes
//! activity-feed rows through the configured severity mapping.
//!
//! The unit suite covers the pure mapping and the SQL-error surface; this test
//! runs against a real PostgreSQL and asserts the persisted
//! `activity_feed.details->>'severity'` matches
//! [`triage_score_to_alert_severity_with`] for custom thresholds (a raised
//! `critical` boundary must not still emit "critical"), and that a status
//! change notification round-trips its `new_status`.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::triage::{TriageItemType, TriageThresholds};
use apex_triage::router_integration::{
    triage_score_to_alert_severity_with, AlertDispatcher, LoggingAlertDispatcher,
    TriageAlertRequest,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

fn custom_thresholds() -> TriageThresholds {
    TriageThresholds {
        critical: 0.90,
        high: 0.60,
        medium: 0.40,
        low: 0.20,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn dispatcher_row_severity_matches_configured_mapping() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");

    let thresholds = custom_thresholds();
    let dispatcher =
        LoggingAlertDispatcher::with_db_and_thresholds(pool.clone(), thresholds.clone());
    let source_id = format!("pg-severity-{}", Uuid::new_v4());

    dispatcher
        .dispatch_triage_alert(TriageAlertRequest {
            item_type: &TriageItemType::Warning,
            source_id: &source_id,
            title: "Boundary severity item",
            description: "0.85 against a configured critical threshold of 0.90",
            composite_score: 0.85,
            entity_id: None,
            entity_name: Some("ACME"),
        })
        .await;

    let severity: Option<String> = sqlx::query_scalar(
        "SELECT details->>'severity' FROM activity_feed \
         WHERE action_type = 'triage_alert' AND entity_id = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&source_id)
    .fetch_one(&pool)
    .await
    .expect("triage_alert activity row written");

    assert_eq!(
        severity.as_deref(),
        Some(triage_score_to_alert_severity_with(0.85, &thresholds).as_str()),
        "persisted severity must come from the configured thresholds"
    );
    assert_eq!(severity.as_deref(), Some("high"));

    if let Err(error) = sqlx::query("DELETE FROM activity_feed WHERE entity_id = $1")
        .bind(&source_id)
        .execute(&pool)
        .await
    {
        panic!("failed to clean up test activity rows: {error}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn status_change_notification_round_trips_the_new_status() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");

    let dispatcher = LoggingAlertDispatcher::with_db(pool.clone());
    let item_id = Uuid::new_v4();

    dispatcher
        .notify_status_change(item_id, "resolved", "Round-trip item")
        .await;

    let row: (String, String) = sqlx::query_as(
        "SELECT action_type, details->>'new_status' FROM activity_feed \
         WHERE action_type = 'triage_status_change' AND entity_id = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(item_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("triage_status_change activity row written");

    assert_eq!(row.0, "triage_status_change");
    assert_eq!(row.1, "resolved");

    if let Err(error) = sqlx::query("DELETE FROM activity_feed WHERE entity_id = $1")
        .bind(item_id.to_string())
        .execute(&pool)
        .await
    {
        panic!("failed to clean up test activity rows: {error}");
    }
}
