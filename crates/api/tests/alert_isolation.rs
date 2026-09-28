//! Alert isolation contract (P0): an alert targeted at user A must never be
//! observed by user B on the SSE channel, while explicit broadcasts still fan
//! out to every connected subscriber.
//!
//! This exercises the [`SseManager`] fan-out used by both the
//! `/api/v1/events/stream` SSE endpoint and the `/ws/warnings` WebSocket
//! bridge.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::alert_router::{AlertEvent, AlertEventType};
use apex_api::sse::SseManager;
use chrono::Utc;
use uuid::Uuid;

fn targeted_alert(user_ids: Vec<Uuid>, title: &str) -> AlertEvent {
    AlertEvent {
        id: Uuid::new_v4(),
        event_type: AlertEventType::NewWarning,
        severity: apex_core::alert_config::AlertSeverity::Critical,
        title: title.to_string(),
        description: format!("{title} description"),
        entity_id: None,
        entity_name: None,
        user_ids,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn alert_targeted_to_a_is_never_observed_by_b() {
    let manager = SseManager::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();

    let (_tx_a, mut rx_a) = manager.register(user_a).await;
    let (_tx_b, mut rx_b) = manager.register(user_b).await;

    for i in 0..5 {
        let alert = targeted_alert(vec![user_a], &format!("A-only {i}"));
        assert_eq!(manager.dispatch_alert(&alert).await, 1);
    }

    // A received every alert, B received none.
    let mut a_events = 0;
    while rx_a.try_recv().is_ok() {
        a_events += 1;
    }
    assert_eq!(a_events, 5, "user A must receive all targeted alerts");

    assert!(
        rx_b.try_recv().is_err(),
        "user B must never observe an alert targeted at user A"
    );
}

#[tokio::test]
async fn isolation_survives_reconnect_of_the_wrong_user() {
    let manager = SseManager::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();

    // A broadcast establishes a cursor both users can hold.
    let (_tx_a, mut rx_a) = manager.register(user_a).await;
    let (tx_b, mut rx_b) = manager.register(user_b).await;
    manager
        .dispatch_alert(&targeted_alert(Vec::new(), "Shared broadcast"))
        .await;
    let cursor = rx_a.try_recv().expect("broadcast for A").id;
    let _ = rx_b.try_recv().expect("broadcast for B");

    // B disconnects; A receives an alert addressed only to A.
    manager.unregister(user_b, &tx_b).await;
    drop(rx_b);
    let alert = targeted_alert(vec![user_a], "A-only while B is away");
    assert_eq!(manager.dispatch_alert(&alert).await, 1);
    let _ = rx_a.try_recv().expect("A receives the alert");

    // B reconnects with its valid in-window cursor: replay must not leak A's
    // alert, even though both connections are inside the replay window.
    let (_tx_b2, mut rx_b2) = manager
        .register_with_last_event_id(user_b, Some(&cursor))
        .await;
    assert!(
        rx_b2.try_recv().is_err(),
        "reconnect replay must never surface an alert targeted at another user"
    );

    // A reconnecting with the same cursor does receive its alert.
    let (_tx_a2, mut rx_a2) = manager
        .register_with_last_event_id(user_a, Some(&cursor))
        .await;
    assert!(
        rx_a2.try_recv().is_ok(),
        "the targeted user must still get the replayed alert"
    );
}

#[tokio::test]
async fn broadcast_alerts_reach_every_connected_user() {
    let manager = SseManager::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();

    let (_tx_a, mut rx_a) = manager.register(user_a).await;
    let (_tx_b, mut rx_b) = manager.register(user_b).await;

    let broadcast = targeted_alert(vec![], "System broadcast");
    assert_eq!(manager.dispatch_alert(&broadcast).await, 2);
    assert!(rx_a.try_recv().is_ok());
    assert!(rx_b.try_recv().is_ok());
}
