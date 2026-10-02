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

    // PostgreSQL timestamps have microsecond precision; capture the test
    // clock at the same precision so round-tripped equality holds on every
    // platform (Linux `Utc::now()` carries nanoseconds).
    let base = {
        let captured = Utc::now();
        captured - Duration::nanoseconds(i64::from(captured.timestamp_subsec_nanos() % 1_000))
    };
    let interval = Duration::hours(24);
    for (attempt, expected_minutes) in [1440_i64, 1440, 1440, 1440, 1440, 1440]
        .into_iter()
        .enumerate()
    {
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
        assert_eq!(row.rolling_success_rate, Some(0.0));
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
    assert!((row.rolling_success_rate.unwrap() - 0.15).abs() < 1e-9);

    let later = now + Duration::minutes(45);
    let row = store
        .record_source_success(slug, Duration::minutes(45), Some(180.0), Some(200), later)
        .await
        .unwrap();
    assert!((row.rolling_latency_ms.unwrap() - 129.0).abs() < 1e-9);
    assert!((row.rolling_success_rate.unwrap() - 0.2775).abs() < 1e-9);

    let loaded = store
        .load_source_runtime_states()
        .await
        .unwrap()
        .into_iter()
        .find(|state| state.source_slug == slug)
        .expect("persisted row is loaded back");
    assert_eq!(loaded.consecutive_failures, 0);
    assert_eq!(loaded.next_due_at, later + Duration::minutes(45));

    // A missing deployment capability is not a crawl attempt: the circuit
    // opens, the reason is recorded, and attempt/failure counters stay put.
    let unavailable_at = now + Duration::hours(1);
    let row = store
        .mark_source_unavailable(
            slug,
            "source requires the headless browser renderer (ENABLE_HEADLESS_BROWSER)",
            Duration::minutes(30),
            unavailable_at,
        )
        .await
        .unwrap();
    assert_eq!(
        row.circuit_open_until,
        Some(unavailable_at + Duration::minutes(30))
    );
    assert_eq!(row.next_due_at, unavailable_at + Duration::minutes(30));
    assert_eq!(row.consecutive_failures, 0);
    // Not an attempt: the previous success timestamps are preserved untouched.
    // The previous success was recorded at `later`, so both timestamps are
    // `later` — never the earlier `now` success.
    assert_eq!(row.last_attempt_at, Some(later));
    assert_eq!(row.last_success_at, Some(later));
    assert!(
        row.last_error
            .as_deref()
            .is_some_and(|error| error.contains("headless browser")),
        "capability gap must be recorded in last_error: {:?}",
        row.last_error
    );

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(slug)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Two workers recording concurrent attempts must merge inside the database:
/// the previous read-then-upsert pair could both read "no row" and overwrite
/// each other's increment, losing a failure and an EWMA step.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn concurrent_failure_updates_do_not_lose_counters() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    let slug = format!("integration_concurrent_{}", uuid::Uuid::new_v4());
    // PostgreSQL timestamps are microsecond-precision; truncate the test clock
    // so the round-tripped `next_due_at` compares equal.
    let now = {
        let captured = Utc::now();
        captured - Duration::nanoseconds(i64::from(captured.timestamp_subsec_nanos() % 1_000))
    };
    let interval = Duration::hours(24);

    let (first, second) = tokio::join!(
        store.record_source_attempt_failure(&slug, "boom-1", Some(500), interval, now),
        store.record_source_attempt_failure(&slug, "boom-2", Some(503), interval, now),
    );
    first.unwrap();
    second.unwrap();

    let (failures, next_due_at): (i32, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT consecutive_failures, next_due_at FROM source_runtime_state WHERE source_slug = $1",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failures, 2, "both concurrent failures must be counted");
    // The 24h interval floors the capped ladder on the second failure too.
    assert_eq!(next_due_at, now + interval);

    // Failure EWMA seeds at 0.0 and stays 0.0 under failure decay: the second
    // merge must decay the stored estimate, not re-seed from a stale read.
    let rate: Option<f64> = sqlx::query_scalar(
        "SELECT rolling_success_rate FROM source_runtime_state WHERE source_slug = $1",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rate, Some(0.0));

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(&slug)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Parser failures (fetch succeeded, deserialization did not) must degrade the
/// source and preserve `last_success_at`: a schema change is not a successful
/// empty parse, and the last known-good validation timestamp stays intact.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn source_parse_failure_preserves_last_success_and_degrades() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    let slug = "integration_parser_contract_feed";

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(slug)
        .execute(&pool)
        .await
        .unwrap();

    // Microsecond precision, matching PostgreSQL's stored timestamps.
    let base = {
        let captured = Utc::now();
        captured - Duration::nanoseconds(i64::from(captured.timestamp_subsec_nanos() % 1_000))
    };
    let interval = Duration::hours(24);
    let success_at = base - Duration::hours(2);
    let row = store
        .record_source_success(slug, interval, Some(90.0), Some(200), success_at)
        .await
        .unwrap();
    assert_eq!(row.last_success_at, Some(success_at));
    assert_eq!(row.consecutive_failures, 0);

    let failure_at = base;
    let row = store
        .record_source_parse_failure(
            slug,
            "failed to parse source JSON: missing field `items`",
            Some("[{\"unexpected\": true}]"),
            Some(200),
            interval,
            failure_at,
        )
        .await
        .unwrap();

    assert_eq!(
        row.last_success_at,
        Some(success_at),
        "a parser failure must NOT update last_success_at"
    );
    assert_eq!(
        row.consecutive_failures, 1,
        "a parser failure must mark the source degraded (backoff counter advances)"
    );
    assert_eq!(row.last_attempt_at, Some(failure_at));
    assert!(
        row.last_error.as_deref().is_some_and(|error| {
            error.starts_with("parser_failure:")
                && error.contains("missing field `items`")
                && error.contains("sample:")
        }),
        "parser failure metadata (error + redacted sample) must be preserved: {:?}",
        row.last_error
    );

    sqlx::query("DELETE FROM source_runtime_state WHERE source_slug = $1")
        .bind(slug)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// `get_crawl_stats` feeds the admin crawl panel and the worker storage stage.
/// `SUM(bigint)` yields NUMERIC; without a cast back to BIGINT the row failed
/// to decode into `i64` whenever the query ran.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn crawl_stats_aggregates_decode_as_bigint() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    // Far-future timestamps isolate the fixture rows from any other data.
    let since = Utc::now() + Duration::days(36_500);
    let marker = "https://crawl-stats-integration.example/";

    sqlx::query("DELETE FROM crawl_logs WHERE source_url = $1")
        .bind(marker)
        .execute(&pool)
        .await
        .unwrap();
    for (status, new_obs, changed, bytes) in
        [("success", 3_i64, 2_i64, 1_000_i64), ("failed", 4, 1, 24)]
    {
        sqlx::query(
            "INSERT INTO crawl_logs (source_url, status, new_observations, changed_pages, bytes_fetched, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(marker)
        .bind(status)
        .bind(new_obs)
        .bind(changed)
        .bind(bytes)
        .bind(since + Duration::minutes(1))
        .execute(&pool)
        .await
        .unwrap();
    }

    let stats = store
        .get_crawl_stats(since)
        .await
        .expect("crawl stats decode");
    assert_eq!(stats.sources_attempted, 2);
    assert_eq!(stats.sources_succeeded, 1);
    assert_eq!(stats.sources_failed, 1);
    assert_eq!(stats.new_observations, 7);
    assert_eq!(stats.changed_pages, 3);
    assert_eq!(stats.bytes_fetched, 1_024);

    sqlx::query("DELETE FROM crawl_logs WHERE source_url = $1")
        .bind(marker)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
