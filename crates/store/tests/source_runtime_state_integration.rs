//! Opt-in integration test for the source-runtime scheduler state
//! (migration 047 + `PgStore` record/load methods).
//!
//! `#[ignore]`d by default like `migrations_integration.rs`; reads
//! `TEST_DATABASE_URL` or `DATABASE_URL` and is intended to run against a
//! disposable PostgreSQL database.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::PgStore;
use chrono::{Duration, Utc};
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

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn source_runtime_state_failure_backoff_success_reset_and_index() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    let slug = "integration_scheduler_feed";

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(slug)
        .execute(&pool)
        .await
        .unwrap();

    let index_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_source_runtime_due')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(index_exists, "idx_source_runtime_due is missing");

    let base = Utc::now();
    let interval = Duration::hours(24);
    for (attempt, expected_minutes) in [30_i64, 60, 120, 240, 480, 480].into_iter().enumerate() {
        let now = base + Duration::minutes(attempt as i64 * 1000);
        let row = store
            .record_source_attempt_failure(slug, "boom", Some(500), interval, now)
            .await
            .unwrap();
        assert_eq!(row.consecutive_failures, attempt as i32 + 1);
        assert_eq!(row.next_due_at, now + Duration::minutes(expected_minutes));
        assert_eq!(row.circuit_open_until, Some(row.next_due_at));
        assert_eq!(row.last_error.as_deref(), Some("boom"));
        assert_eq!(row.last_http_status, Some(500));
    }

    let now = base + Duration::days(30);
    let row = store
        .record_source_success(slug, Duration::minutes(45), Some(120.0), Some(200), now)
        .await
        .unwrap();
    assert_eq!(row.consecutive_failures, 0);
    assert_eq!(row.next_due_at, now + Duration::minutes(45));
    assert_eq!(row.last_success_at, Some(now));
    assert!(row.circuit_open_until.is_none());
    assert!(row.last_error.is_none());
    assert_eq!(row.rolling_latency_ms, Some(120.0));

    let loaded = store
        .load_source_runtime_states()
        .await
        .unwrap()
        .into_iter()
        .find(|state| state.source_slug == slug)
        .expect("persisted row is loaded back");
    assert_eq!(loaded.consecutive_failures, 0);
    assert_eq!(loaded.next_due_at, now + Duration::minutes(45));

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(slug)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
