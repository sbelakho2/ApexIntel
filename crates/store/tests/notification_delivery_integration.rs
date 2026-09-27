//! Opt-in integration test for the durable notification delivery pipeline
//! against a real PostgreSQL database (migration 069).
//!
//! Proves the SQL semantics the retry processor depends on:
//! * domain event + alert outbox row + per-channel `pending` rows commit in one
//!   transaction, and the dedupe key makes repeated enqueues no-ops;
//! * the claim commits before any attempt (no row lock held afterwards) and
//!   increments `attempts` (persist before attempt);
//! * a lease blocks a second worker; retry/dead-letter/replay/backlog all work.
//!
//! `#[ignore]`d by default (CI runs DB suites with `--ignored`); reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{DeliveryChannel, NewNotificationEvent, NotificationBacklog, PgStore};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

static DB_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

fn sample_event(dedupe_key: &str) -> (NewNotificationEvent, DeliveryChannel, DeliveryChannel) {
    let alert_id = Uuid::new_v4();
    let outbox_id = Uuid::new_v4();
    let channel = DeliveryChannel {
        channel: "webhook".to_string(),
        destination: "https://example.test/hook".to_string(),
    };
    let email = DeliveryChannel {
        channel: "email".to_string(),
        destination: "ops@example.test".to_string(),
    };
    let event = NewNotificationEvent {
        dedupe_key: dedupe_key.to_string(),
        source_type: "sla_breach".to_string(),
        source_id: format!("sla-breach:{alert_id}"),
        severity: "critical".to_string(),
        category: "sla_breach".to_string(),
        title: "SLA BREACH".to_string(),
        body: "warning breached its SLA".to_string(),
        payload: serde_json::json!({"source_id": format!("sla-breach:{alert_id}")}),
        outbox_aggregate_type: "sla_alert".to_string(),
        outbox_aggregate_id: outbox_id,
        outbox_event_type: "new_warning".to_string(),
        outbox_payload: serde_json::json!({
            "id": outbox_id,
            "event_type": "new_warning",
            "title": "SLA BREACH",
            "description": "warning breached its SLA",
        }),
    };
    (event, channel, email)
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn expired_delivering_lease_is_reclaimable_after_a_crash() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    sqlx::query("DELETE FROM notification_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM event_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let dedupe_key = format!("crash-reclaim:{}", Uuid::new_v4());
    let (event, webhook, _email) = sample_event(&dedupe_key);
    let outcome = store
        .enqueue_notification_event(&event, std::slice::from_ref(&webhook))
        .await
        .unwrap();
    let outbox_id = outcome.outbox_id.expect("alert outbox row id");

    // Claim with an already-expired lease: the channel send is attempted but
    // the settlement write never lands (crash).
    let first = store
        .claim_due_notification_deliveries("crashed-worker", 0.0, 10)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].attempts, 1);
    assert_eq!(first[0].status, "delivering");

    let reclaimed = store
        .claim_due_notification_deliveries("recovering-worker", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(
        reclaimed.len(),
        1,
        "a crash must leave the delivery reclaimable after its lease expires, not stuck"
    );
    assert_eq!(reclaimed[0].attempts, 2);
    assert_eq!(reclaimed[0].delivery_key, first[0].delivery_key);
    assert_eq!(
        reclaimed[0].lease_owner.as_deref(),
        Some("recovering-worker")
    );

    // Cleanup (delivery rows cascade with the domain event).
    sqlx::query("DELETE FROM notification_events WHERE id = $1")
        .bind(outcome.notification_event_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM event_outbox WHERE id = $1")
        .bind(outbox_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn sla_alert_enqueues_outbox_and_deliveries_then_retry_lifecycle() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());
    // Remove rows left behind by previous interrupted runs.
    sqlx::query("DELETE FROM notification_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM event_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let dedupe_key = format!("sla_breach:{}", Uuid::new_v4());
    let (event, webhook, email) = sample_event(&dedupe_key);
    let channels = vec![webhook.clone(), email.clone()];

    // ONE transaction: domain event + outbox + per-channel pending rows.
    let outcome = store
        .enqueue_notification_event(&event, &channels)
        .await
        .expect("enqueue notification event");
    assert!(!outcome.already_enqueued);
    assert_eq!(outcome.deliveries_enqueued, 2);
    let outbox_id = outcome.outbox_id.expect("alert outbox row id");

    let (outbox_payload, published): (serde_json::Value, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT payload, published_at FROM event_outbox WHERE id = $1")
            .bind(outbox_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(outbox_payload["title"], serde_json::json!("SLA BREACH"));
    assert!(published.is_none(), "fresh outbox rows are unpublished");

    let (delivery_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint FROM notification_delivery_state WHERE notification_event_id = $1",
    )
    .bind(outcome.notification_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(delivery_count, 2, "one pending row per channel");

    // The persisted payload is the transport wrapper the retry processor
    // deserializes (`{"alert": ...}`), not the raw domain alert.
    let (wrapper_stored,): (bool,) = sqlx::query_as(
        "SELECT payload ? 'alert' FROM notification_delivery_state \
          WHERE notification_event_id = $1 LIMIT 1",
    )
    .bind(outcome.notification_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        wrapper_stored,
        "delivery rows must store the transport wrapper, not the raw domain alert"
    );

    // Repeated scheduler runs must not create uncontrolled duplicates.
    let repeat = store
        .enqueue_notification_event(&event, &channels)
        .await
        .unwrap();
    assert!(repeat.already_enqueued);
    assert_eq!(repeat.outbox_id, None);
    let (delivery_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint FROM notification_delivery_state WHERE notification_event_id = $1",
    )
    .bind(outcome.notification_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(delivery_count, 2, "re-enqueue writes nothing new");

    // Claim: attempts persisted, lease stamped, and NO row lock held across
    // the (future) channel send — proven with FOR UPDATE NOWAIT.
    let claimed = store
        .claim_due_notification_deliveries("worker-1", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2);
    assert!(claimed.iter().all(|row| row.attempts == 1));
    sqlx::query(
        "SELECT delivery_key FROM notification_delivery_state WHERE delivery_key = $1 FOR UPDATE NOWAIT",
    )
    .bind(&claimed[0].delivery_key)
        .fetch_one(&pool)
        .await
        .expect("claiming must not hold a row lock across the send");

    // A second worker skips the leased rows.
    let skipped = store
        .claim_due_notification_deliveries("worker-2", 120.0, 10)
        .await
        .unwrap();
    assert!(skipped.is_empty(), "leased rows are skipped");

    // Retry: failure reschedules with backoff and releases the lease.
    let retry_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(store
        .mark_notification_retry(&claimed[0].delivery_key, "worker-1", retry_at, "503")
        .await
        .unwrap());
    let reclaimed = store
        .claim_due_notification_deliveries("worker-2", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1, "a due retry is claimable again");
    assert_eq!(reclaimed[0].attempts, 2);

    // Success clears the lease and records delivery.
    assert!(store
        .mark_notification_delivered(&reclaimed[0].delivery_key, "worker-2")
        .await
        .unwrap());

    // Dead letter: terminal until an operator replay resets the budget.
    assert!(store
        .mark_notification_dead_lettered(&claimed[1].delivery_key, "worker-1", "410 gone")
        .await
        .unwrap());
    let dead = store.list_dead_lettered_notifications(10).await.unwrap();
    assert!(dead
        .iter()
        .any(|row| row.delivery_key == claimed[1].delivery_key));
    let not_claimable = store
        .claim_due_notification_deliveries("worker-3", 120.0, 10)
        .await
        .unwrap();
    assert!(
        !not_claimable
            .iter()
            .any(|row| row.delivery_key == claimed[1].delivery_key),
        "dead-lettered rows are not claimable"
    );

    let backlog = store
        .notification_delivery_backlog(chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(
        backlog,
        NotificationBacklog {
            pending: 0,
            overdue: 0,
            dead_lettered: 1,
        }
    );

    assert!(store
        .replay_dead_lettered_notification(&claimed[1].delivery_key)
        .await
        .unwrap());
    let replayed = store
        .claim_due_notification_deliveries("worker-4", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(replayed.len(), 1, "a replayed row is claimable again");
    assert_eq!(replayed[0].attempts, 1, "replay resets the attempt budget");

    // Cleanup (delivery rows cascade with the domain event).
    sqlx::query("DELETE FROM notification_events WHERE id = $1")
        .bind(outcome.notification_event_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM event_outbox WHERE id = $1")
        .bind(outbox_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
