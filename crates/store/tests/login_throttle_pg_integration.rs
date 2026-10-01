//! Opt-in PostgreSQL test: concurrent login-throttle reservations must not
//! lose updates.
//!
//! `SELECT ... FOR UPDATE` locks no row for a key that does not exist yet, so
//! concurrent first attempts on the same key used to read the default state
//! and overwrite each other. The reservation is serialized on the attempt key
//! via a transaction advisory lock; exactly one concurrent attempt may be
//! reserved and the stored counter must reflect it.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a PostgreSQL
//! service through `scripts/ci/run_pg_integration_suites.sh`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use apex_store::postgres::PgStore;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    pool
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn concurrent_reserves_serialize_on_the_attempt_key() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let attempt_key = format!("throttle-race-{}", Uuid::new_v4().simple());
    let now = chrono::Utc::now();

    // Five simultaneous first attempts. The advisory lock serializes them:
    // the first records a failure and starts the backoff; the rest observe the
    // recorded state and are refused.
    let mut handles = Vec::new();
    for _ in 0..5 {
        let store = Arc::clone(&store);
        let attempt_key = attempt_key.clone();
        handles.push(tokio::spawn(async move {
            store.login_throttle_reserve(&attempt_key, now).await
        }));
    }

    let mut allowed = 0usize;
    for handle in handles {
        let status = handle
            .await
            .expect("reserve task joins")
            .expect("reserve query succeeds");
        if status.allowed {
            allowed += 1;
        }
    }
    assert_eq!(
        allowed, 1,
        "exactly one concurrent first attempt may be reserved"
    );

    let stored: i32 = sqlx::query_scalar(
        "SELECT failure_count_10m FROM login_attempt_throttle WHERE attempt_key = $1",
    )
    .bind(&attempt_key)
    .fetch_one(&pool)
    .await
    .expect("throttle row exists");
    assert_eq!(
        stored, 1,
        "a losing reservation must not overwrite the winner's counter"
    );

    sqlx::query("DELETE FROM login_attempt_throttle WHERE attempt_key = $1")
        .bind(&attempt_key)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}
