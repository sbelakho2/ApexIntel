//! Opt-in integration test for the transactional event outbox against a real
//! PostgreSQL database (migration 061).
//!
//! The unit suite never touches a database; this proves the SQL semantics the
//! alert pipeline depends on: warning + outbox commit together, a dropped
//! (crashed) batch leaves the event unpublished, and only a committed
//! `mark_published` stamps it.
//!
//! It is `#[ignore]`d by default (CI runs it with `--ignored` against a
//! Postgres service) and reads `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::PgStore;
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

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn warning_and_outbox_commit_together_and_crash_recovery_drains_later() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let title = format!("outbox integration {}", Uuid::new_v4());
    let event_id = Uuid::new_v4();
    let (outcome, outbox_id) = store
        .insert_warning_with_outbox(
            "outbox_test",
            &title,
            Some("committed with its outbox event"),
            "high",
            None,
            None,
            Some(vec![Uuid::new_v4()]),
            None,
            Some(0.9),
            "warning",
            "new_warning",
            |outcome| {
                serde_json::json!({
                    "id": outcome.id,
                    "warning_id": outcome.id,
                    "event_type": "new_warning",
                    "marker": event_id,
                })
            },
        )
        .await
        .expect("warning + outbox insert");
    assert!(outcome.created, "first insert must create the warning");

    // The outbox row is committed but unpublished.
    let (published_at, payload): (Option<chrono::DateTime<chrono::Utc>>, serde_json::Value) =
        sqlx::query_as("SELECT published_at, payload FROM event_outbox WHERE id = $1")
            .bind(outbox_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        published_at.is_none(),
        "fresh outbox events are unpublished"
    );
    assert_eq!(payload["warning_id"], serde_json::json!(outcome.id));
    assert_eq!(payload["marker"], serde_json::json!(event_id));

    // Simulate a crash between commit and publish: lock the batch, mark it,
    // then drop it WITHOUT committing. Nothing may be stamped and the row must
    // still be claimable by the next drain.
    {
        let mut crashed = store.lock_unpublished_outbox(10).await.unwrap();
        let locked: Vec<Uuid> = crashed.events().iter().map(|row| row.id).collect();
        assert!(
            locked.contains(&outbox_id),
            "the committed event must be claimable by the drain"
        );
        crashed.mark_published(outbox_id).await.unwrap();
        // `crashed` drops here without commit: the simulated process death.
    }

    let (published_at, attempts, last_error): (
        Option<chrono::DateTime<chrono::Utc>>,
        i32,
        Option<String>,
    ) = sqlx::query_as("SELECT published_at, attempts, last_error FROM event_outbox WHERE id = $1")
        .bind(outbox_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        published_at.is_none(),
        "a crashed batch must leave the event unpublished for redelivery"
    );
    assert_eq!(attempts, 0);
    assert!(last_error.is_none());

    // The next drain publishes and stamps it.
    let mut batch = store.lock_unpublished_outbox(10).await.unwrap();
    let locked: Vec<Uuid> = batch.events().iter().map(|row| row.id).collect();
    assert!(locked.contains(&outbox_id));
    batch.mark_published(outbox_id).await.unwrap();
    batch.commit().await.unwrap();

    let (published_at, marked): (Option<chrono::DateTime<chrono::Utc>>, bool) = sqlx::query_as(
        "SELECT published_at, published_at IS NOT NULL FROM event_outbox WHERE id = $1",
    )
    .bind(outbox_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        published_at.is_some(),
        "the healthy drain stamps published_at"
    );
    assert!(marked);

    // A second mark is idempotent (no double-publish bookkeeping).
    assert!(
        !store.mark_outbox_published(outbox_id).await.unwrap(),
        "an already published event must not be re-stamped"
    );

    // Cleanup.
    sqlx::query("DELETE FROM event_outbox WHERE id = $1")
        .bind(outbox_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(outcome.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn plain_warning_insert_writes_no_outbox_event() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let title = format!("no outbox {}", Uuid::new_v4());
    let outcome = store
        .insert_warning_with_outcome(
            "outbox_test",
            &title,
            None,
            "low",
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("plain warning insert");
    assert!(outcome.created);

    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*)::bigint FROM event_outbox WHERE aggregate_id = $1")
            .bind(outcome.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        count, 0,
        "the non-outbox path must not enqueue alert events"
    );

    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(outcome.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
