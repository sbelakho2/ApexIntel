//! Crash-window idempotency proof for durable notification delivery.
//!
//! Simulates exactly the audit scenario against a real PostgreSQL database and
//! a real HTTP receiver:
//!
//!   attempt 1 accepted by the receiver -> the settlement write is lost
//!   (crash) -> the lease expires -> the row is reclaimed -> attempt 2 carries
//!   the SAME stable delivery key, so the receiver deduplicates it. The
//!   attempt number travels separately as `X-Apex-Attempt`.
//!
//! `#[ignore]`d by default (CI runs DB suites with `--ignored`); reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use apex_core::alert_config::AlertScope;
use apex_store::postgres::{DeliveryChannel, PgStore};
use apex_worker::notification_delivery::{
    notification_event_for, ChannelTransport, ConfiguredChannelRouter, NotificationDelivery,
};
use apex_worker::notifications::{
    AlertSeverity, NotificationConfig, PendingAlert, WebhookConfig, WebhookFormat,
};
use sqlx::postgres::PgPoolOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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

/// The delivery identity headers recorded by the receiver for one request.
#[derive(Debug, Clone)]
struct CapturedRequest {
    idempotency_key: Option<String>,
    delivery_key: Option<String>,
    attempt: Option<String>,
    body: String,
}

fn parse_header(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Read one complete HTTP request (headers + content-length body).
async fn read_request(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if let Some(headers_end) = find_subslice(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..headers_end]).to_string();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= headers_end + 4 + content_length {
                break;
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// Minimal HTTP receiver: records the delivery identity headers and
/// deduplicates by `Idempotency-Key`, exactly like a production receiver.
async fn spawn_receiver() -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    Arc<Mutex<Vec<String>>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind receiver");
    let addr = listener.local_addr().expect("receiver addr");
    let seen: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let deduplicated: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_task = Arc::clone(&seen);
    let dedup_task = Arc::clone(&deduplicated);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let seen = Arc::clone(&seen_task);
            let dedup = Arc::clone(&dedup_task);
            tokio::spawn(async move {
                let Some(raw) = read_request(&mut socket).await else {
                    return;
                };
                let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
                let captured = CapturedRequest {
                    idempotency_key: parse_header(head, "idempotency-key"),
                    delivery_key: parse_header(head, "x-apex-delivery-key"),
                    attempt: parse_header(head, "x-apex-attempt"),
                    body: body.to_string(),
                };
                let duplicate = captured.idempotency_key.as_ref().is_some_and(|key| {
                    seen.lock()
                        .unwrap()
                        .iter()
                        .any(|previous| previous.idempotency_key.as_ref() == Some(key))
                });
                if duplicate {
                    dedup
                        .lock()
                        .unwrap()
                        .push(captured.idempotency_key.clone().unwrap_or_default());
                }
                seen.lock().unwrap().push(captured);
                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}/hook"), seen, deduplicated)
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn crash_after_send_keeps_the_delivery_key_stable_across_reclaims() {
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

    let (destination, received, deduplicated) = spawn_receiver().await;

    let mut alert = PendingAlert::new(
        "crash-window:test",
        AlertScope::entity("entity-1", Some("Acme")),
        "IDEMPOTENCY",
        "the crash window must not change the delivery key",
        AlertSeverity::Critical,
        0.95,
    );
    alert.category = "sla_breach".to_string();

    let event = notification_event_for(&alert).expect("alert serializes");
    let channel = DeliveryChannel {
        channel: "webhook".to_string(),
        destination: destination.clone(),
    };
    let enqueued = store
        .enqueue_notification_event(&event, std::slice::from_ref(&channel))
        .await
        .expect("enqueue notification event");
    assert!(!enqueued.already_enqueued);
    assert_eq!(enqueued.deliveries_enqueued, 1);

    let config = NotificationConfig {
        webhooks: vec![WebhookConfig {
            name: "webhook".to_string(),
            url: destination.clone(),
            bearer_token: None,
            // The receiver asserts the Slack body carries `text`.
            format: WebhookFormat::Slack,
            min_severity: AlertSeverity::Low,
            min_priority: 0.0,
        }],
        email: None,
        log_min_severity: AlertSeverity::Low,
    };
    let router = ConfiguredChannelRouter::new(config).expect("router builds");

    // Attempt 1: the receiver ACCEPTS the delivery; the settlement write is
    // then lost (simulated crash after the send).
    let claimed = store
        .claim_due_notification_deliveries("owner-1", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let first = NotificationDelivery::from_row(&claimed[0]).expect("row parses");
    assert_eq!(first.attempts, 1);
    router
        .deliver(&first)
        .await
        .expect("receiver accepts attempt 1");
    // No mark_* call: the crash window is open.

    // The lease expires and another worker reclaims the same row.
    sqlx::query(
        "UPDATE notification_delivery_state \
            SET lease_until = now() - interval '1 second' \
          WHERE delivery_key = $1",
    )
    .bind(&first.delivery_key)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed = store
        .claim_due_notification_deliveries("owner-2", 120.0, 10)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1, "the expired lease is reclaimable");
    let second = NotificationDelivery::from_row(&reclaimed[0]).expect("row parses");
    assert_eq!(second.attempts, 2);
    assert_eq!(
        second.idempotency_key, first.idempotency_key,
        "the stable delivery key must be identical across attempts"
    );
    router
        .deliver(&second)
        .await
        .expect("receiver accepts the redelivery");

    let requests = received.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "both attempts reached the receiver");
    let first_key = requests[0]
        .idempotency_key
        .clone()
        .expect("Idempotency-Key header is present");
    assert_eq!(
        Some(first_key.clone()),
        requests[1].idempotency_key,
        "the receiver sees one stable identity across attempts"
    );
    assert_eq!(requests[0].delivery_key, requests[1].delivery_key);
    assert_eq!(
        requests[0].delivery_key.as_deref(),
        Some(first_key.as_str()),
        "X-Apex-Delivery-Key carries the stable key"
    );
    assert_eq!(requests[0].attempt.as_deref(), Some("1"));
    assert_eq!(
        requests[1].attempt.as_deref(),
        Some("2"),
        "the attempt number travels separately"
    );
    assert_eq!(
        deduplicated.lock().unwrap().as_slice(),
        std::slice::from_ref(&first_key),
        "the receiver would deduplicate the reclaimed redelivery"
    );

    // The enqueued row only carries the alert; the router renders the webhook
    // body. A fabricated/empty payload would fail here.
    assert_eq!(requests[0].body, requests[1].body, "same redelivered body");
    let body: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("webhook body is JSON");
    assert!(
        body["text"]
            .as_str()
            .is_some_and(|text| text.contains("IDEMPOTENCY")),
        "the rendered body carries the alert title: {}",
        requests[0].body
    );

    // Settle the redelivery: the attempt log then feeds the readiness success
    // ratio.
    assert!(store
        .mark_notification_delivered(&second.delivery_key, "owner-2")
        .await
        .unwrap());
    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM notification_delivery_attempts \
          WHERE delivery_key = $1 \
          ORDER BY attempted_at DESC LIMIT 1",
    )
    .bind(&second.delivery_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "delivered");

    // Cleanup (delivery rows + attempts cascade with the domain event).
    sqlx::query("DELETE FROM notification_events WHERE id = $1")
        .bind(enqueued.notification_event_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM event_outbox WHERE id = $1")
        .bind(enqueued.outbox_id.expect("outbox id"))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
