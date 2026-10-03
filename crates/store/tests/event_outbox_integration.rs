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

/// These tests share one database and some of them drain all unpublished
/// events, so they must not run concurrently with each other.
static DB_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Remove rows left behind by previous interrupted runs of this test binary.
async fn clean_outbox_test_rows(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM event_outbox")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM warnings WHERE warning_type = 'outbox_test'")
        .execute(pool)
        .await
        .unwrap();
}

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
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    clean_outbox_test_rows(&pool).await;

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
            false,
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

    // TX1: claim commits before any publish. The attempt is persisted and no
    // database lock is held afterwards.
    let claim = store
        .claim_unpublished_outbox("drain-1", 120.0, 10)
        .await
        .unwrap();
    let claimed: Vec<Uuid> = claim.rows.iter().map(|row| row.id).collect();
    assert!(
        claimed.contains(&outbox_id),
        "the committed event must be claimable by the drain"
    );
    assert_eq!(
        claim.rows[0].attempts, 1,
        "the attempt is persisted at claim"
    );

    // No transaction is held across the publish: a FOR UPDATE NOWAIT probe on
    // another connection succeeds immediately instead of blocking.
    sqlx::query("SELECT id FROM event_outbox WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(outbox_id)
        .fetch_one(&pool)
        .await
        .expect("claiming must not hold a row lock across the publish");

    // A second owner skips the leased row.
    let second = store
        .claim_unpublished_outbox("drain-2", 120.0, 10)
        .await
        .unwrap();
    assert!(
        !second.rows.iter().any(|row| row.id == outbox_id),
        "a leased event must be skipped by a concurrent publisher"
    );

    // Simulate a crash after the broker accepted but before TX2 recorded it:
    // the lease is still held and the row is unpublished, so the drain can
    // reclaim it once the lease is released (or expires).
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
        "a crash before TX2 must leave the event unpublished for redelivery"
    );
    assert_eq!(attempts, 1);
    assert!(last_error.is_none());

    // Release the crashed lease (equivalent to lease expiry) and let the next
    // drain publish and stamp it.
    store
        .release_outbox_claims("drain-1", &[outbox_id])
        .await
        .unwrap();
    let claim = store
        .claim_outbox_event("drain-3", 120.0, outbox_id)
        .await
        .unwrap();
    assert_eq!(claim.rows.len(), 1);
    assert!(
        store
            .mark_outbox_published(outbox_id, "drain-3")
            .await
            .unwrap(),
        "TX2 stamps published_at for the lease owner"
    );

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

    // An already-published event is no longer claimable (no double-publish).
    assert!(
        store
            .claim_outbox_event("drain-4", 120.0, outbox_id)
            .await
            .unwrap()
            .is_empty(),
        "a published event must never be claimable again"
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
async fn locked_and_exhausted_events_are_not_claimable() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    clean_outbox_test_rows(&pool).await;

    let title = format!("claim semantics {}", Uuid::new_v4());
    let (outcome, outbox_id) = store
        .insert_warning_with_outbox(
            "outbox_test",
            &title,
            None,
            "low",
            None,
            None,
            None,
            None,
            None,
            false,
            "warning",
            "new_warning",
            |outcome| serde_json::json!({"warning_id": outcome.id}),
        )
        .await
        .expect("warning + outbox insert");

    // While one publisher holds the lease, a second claim must come back empty
    // instead of publishing it twice.
    let held = store
        .claim_outbox_event("drain-1", 120.0, outbox_id)
        .await
        .unwrap();
    assert_eq!(held.rows.len(), 1);
    {
        let second = store
            .claim_outbox_event("drain-2", 120.0, outbox_id)
            .await
            .unwrap();
        assert!(
            second.is_empty(),
            "a leased event must be skipped by a concurrent publisher"
        );
    }

    // A crashed holder's lease expires and the event becomes claimable again.
    sqlx::query("UPDATE event_outbox SET lease_until = now() - interval '1 second' WHERE id = $1")
        .bind(outbox_id)
        .execute(&pool)
        .await
        .unwrap();
    let reclaimed = store
        .claim_outbox_event("drain-2", 120.0, outbox_id)
        .await
        .unwrap();
    assert_eq!(
        reclaimed.rows.len(),
        1,
        "an expired lease must be reclaimable after a crash"
    );
    store
        .release_outbox_claims("drain-2", &[outbox_id])
        .await
        .unwrap();

    // An event that exhausted its attempts is excluded from both claims.
    sqlx::query("UPDATE event_outbox SET attempts = $2 WHERE id = $1")
        .bind(outbox_id)
        .bind(apex_store::postgres::MAX_OUTBOX_ATTEMPTS)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store
        .claim_outbox_event("drain-3", 120.0, outbox_id)
        .await
        .unwrap()
        .is_empty());
    assert!(
        !store
            .claim_unpublished_outbox("drain-3", 120.0, 10)
            .await
            .unwrap()
            .rows
            .iter()
            .any(|row| row.id == outbox_id),
        "exhausted events must not occupy the drain"
    );

    // A failed final attempt moves the row to the explicit dead-letter state;
    // it stays terminal until an operator replay resets the attempt budget.
    sqlx::query("UPDATE event_outbox SET attempts = $2 WHERE id = $1")
        .bind(outbox_id)
        .bind(apex_store::postgres::MAX_OUTBOX_ATTEMPTS - 1)
        .execute(&pool)
        .await
        .unwrap();
    let final_claim = store
        .claim_outbox_event("drain-4", 120.0, outbox_id)
        .await
        .unwrap();
    assert_eq!(final_claim.rows.len(), 1);
    assert!(
        store
            .record_outbox_failure(outbox_id, "drain-4", "simulated publish failure")
            .await
            .unwrap(),
        "the final failed attempt must dead-letter the row"
    );
    assert!(
        store
            .claim_outbox_event("drain-5", 120.0, outbox_id)
            .await
            .unwrap()
            .is_empty(),
        "a dead-lettered event must not be claimable"
    );
    let dead_letters = store.list_dead_lettered_outbox(10).await.unwrap();
    assert!(
        dead_letters.iter().any(|row| row.id == outbox_id),
        "the dead-lettered row must be visible to the admin replay listing"
    );
    assert!(
        store.replay_dead_lettered_outbox(outbox_id).await.unwrap(),
        "operator replay must clear the dead-letter state"
    );
    let replayed = store
        .claim_outbox_event("drain-6", 120.0, outbox_id)
        .await
        .unwrap();
    assert_eq!(
        replayed.rows.len(),
        1,
        "a replayed event must be claimable again"
    );
    assert_eq!(
        replayed.rows[0].attempts, 1,
        "replay resets the attempt budget"
    );

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
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    clean_outbox_test_rows(&pool).await;

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
            false,
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

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn system_broadcast_flag_persists_and_merges_with_or() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    clean_outbox_test_rows(&pool).await;

    let entity = Uuid::new_v4();
    let flag_of = |id: Uuid| {
        let pool = pool.clone();
        async move {
            let (flag,): (bool,) =
                sqlx::query_as("SELECT is_system_broadcast FROM warnings WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            flag
        }
    };

    // A deliberate system broadcast persists its flag.
    let broadcast_title = format!("broadcast {}", Uuid::new_v4());
    let broadcast = store
        .insert_warning_with_outcome(
            "outbox_test",
            &broadcast_title,
            None,
            "low",
            None,
            None,
            Some(vec![entity]),
            None,
            None,
            true,
        )
        .await
        .expect("broadcast insert");
    assert!(flag_of(broadcast.id).await);

    // A recurring non-broadcast submission merges into it and must not clear
    // the flag.
    let recurrence = store
        .insert_warning_with_outcome(
            "outbox_test",
            &broadcast_title,
            None,
            "low",
            None,
            None,
            Some(vec![entity]),
            None,
            None,
            false,
        )
        .await
        .expect("recurrence insert");
    assert!(!recurrence.created, "same signature must deduplicate");
    assert_eq!(recurrence.id, broadcast.id);
    assert!(
        flag_of(broadcast.id).await,
        "a non-broadcast recurrence must not demote an existing broadcast"
    );

    // A broadcast submission merging into a non-broadcast warning promotes it.
    let plain_title = format!("plain {}", Uuid::new_v4());
    let plain = store
        .insert_warning_with_outcome(
            "outbox_test",
            &plain_title,
            None,
            "low",
            None,
            None,
            Some(vec![entity]),
            None,
            None,
            false,
        )
        .await
        .expect("plain insert");
    assert!(!flag_of(plain.id).await);

    let promoted = store
        .insert_warning_with_outcome(
            "outbox_test",
            &plain_title,
            None,
            "low",
            None,
            None,
            Some(vec![entity]),
            None,
            None,
            true,
        )
        .await
        .expect("promoting insert");
    assert_eq!(promoted.id, plain.id);
    assert!(
        flag_of(plain.id).await,
        "a broadcast submission must promote an existing warning"
    );

    for id in [broadcast.id, plain.id] {
        sqlx::query("DELETE FROM warnings WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
}

/// Dedup merges are entity-scoped and text-similarity gated, and a recurrence
/// never buries a more severe signal: a critical recency escalates the merged
/// row instead of creating (or hiding under) a lower-severity one.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn dedup_requires_same_entity_similar_text_and_escalates_severity() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    clean_outbox_test_rows(&pool).await;

    let entity = Uuid::new_v4();
    let other_entity = Uuid::new_v4();
    let title = format!("dedup contract {}", Uuid::new_v4());

    let insert = |description: &'static str, severity: &'static str, entities: Vec<Uuid>| {
        let store = store.clone();
        let title = title.clone();
        async move {
            store
                .insert_warning_with_outcome(
                    "outbox_test",
                    &title,
                    Some(description),
                    severity,
                    None,
                    None,
                    Some(entities),
                    None,
                    None,
                    false,
                )
                .await
                .unwrap()
        }
    };

    // 1. Same entity and window, but unrelated text: no merge (the old
    //    bare entity+window branch collapsed every signal about a company).
    let first = insert("supply chain halt in Tunisia", "low", vec![entity]).await;
    assert!(first.created);
    let unrelated = insert("quarterly earnings beat expectations", "low", vec![entity]).await;
    assert!(
        unrelated.created,
        "entity and time window alone must not merge unrelated signals"
    );

    // 2. Same entity with genuinely similar text: merged.
    let recurrence = insert("supply chain halt in Tunisia.", "low", vec![entity]).await;
    assert!(!recurrence.created, "similar same-entity text must merge");
    assert_eq!(recurrence.id, first.id);

    // 3. A more severe recurrence escalates the merged row.
    let severe = insert(
        "supply chain halt in Tunisia plant",
        "critical",
        vec![entity],
    )
    .await;
    assert!(!severe.created, "the severe recurrence must merge");
    assert_eq!(severe.id, first.id);
    let (severity,): (String,) = sqlx::query_as("SELECT severity FROM warnings WHERE id = $1")
        .bind(first.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        severity, "critical",
        "a more severe recurrence must not be buried in a low-severity row"
    );

    // 4. Lexical dedup requires the same entity: identical text about a
    //    different company is a different warning.
    let other_company = insert("supply chain halt in Tunisia", "low", vec![other_entity]).await;
    assert!(
        other_company.created,
        "identical text for a different entity must not merge"
    );

    clean_outbox_test_rows(&pool).await;
    pool.close().await;
}
