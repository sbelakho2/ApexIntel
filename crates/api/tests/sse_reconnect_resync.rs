//! SSE reconnect + canonical-state resync contract (P0).
//!
//! A reconnecting `EventSource` presents the last event id it saw. When the
//! server still retains the intervening events they are replayed; when the
//! cursor falls outside the bounded replay window the server emits a `resync`
//! event so the client refetches canonical state (`GET /warnings/unread-count`)
//! instead of silently missing alerts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::alert_router::{AlertEvent, AlertEventType};
use apex_api::sse::{SseEvent, SseManager, SSE_RESYNC_EVENT};
use chrono::Utc;
use uuid::Uuid;

fn alert_for(user_id: Uuid, title: &str) -> AlertEvent {
    AlertEvent {
        id: Uuid::new_v4(),
        event_type: AlertEventType::NewWarning,
        severity: apex_core::alert_config::AlertSeverity::High,
        title: title.to_string(),
        description: "resync fixture".to_string(),
        entity_id: None,
        entity_name: None,
        user_ids: vec![user_id],
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
    }
}

fn drain(rx: &mut tokio::sync::mpsc::Receiver<SseEvent>) -> Vec<SseEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn reconnect_replays_events_missed_while_disconnected() {
    let manager = SseManager::new();
    let user = Uuid::new_v4();

    // Tab 1 + tab 2 for the same user; tab 1 sees "before".
    let (tx1, mut rx1) = manager.register(user).await;
    let (_tx2, mut rx2) = manager.register(user).await;
    manager
        .dispatch_alert(&alert_for(user, "before-disconnect"))
        .await;
    let seen = drain(&mut rx1);
    let _ = drain(&mut rx2);
    assert_eq!(seen.len(), 1, "tab 1 observes the first alert");

    // Tab 1 drops. Two alerts arrive while it is away (still delivered to tab 2
    // and therefore retained for replay).
    manager.unregister(user, &tx1).await;
    drop(rx1);
    manager.dispatch_alert(&alert_for(user, "missed-1")).await;
    manager.dispatch_alert(&alert_for(user, "missed-2")).await;
    let _ = drain(&mut rx2);

    // Tab 1 reconnects with its cursor: both missed alerts replay and nothing
    // already seen is re-delivered.
    let (_tx3, mut rx3) = manager
        .register_with_last_event_id(user, Some(seen[0].id.as_str()))
        .await;
    let replayed = drain(&mut rx3);
    assert_eq!(replayed.len(), 2, "both missed alerts must replay");
    assert!(replayed.iter().all(|event| event.id != seen[0].id));
    assert_eq!(replayed[0].event, "new_warning");
}

#[tokio::test]
async fn reconnect_beyond_replay_window_requests_canonical_resync() {
    let manager = SseManager::new();
    let user = Uuid::new_v4();

    let (_tx, mut rx) = manager.register(user).await;
    manager.dispatch_alert(&alert_for(user, "current")).await;
    let _ = drain(&mut rx);

    let (_tx2, mut rx2) = manager
        .register_with_last_event_id(user, Some("cursor-from-a-previous-process"))
        .await;
    let events = drain(&mut rx2);
    assert_eq!(events.len(), 1, "resync emits exactly one marker event");
    assert_eq!(events[0].event, SSE_RESYNC_EVENT);
    let payload: serde_json::Value =
        serde_json::from_str(&events[0].data).expect("resync payload json");
    assert_eq!(payload["reason"], "replay_window_exhausted");
}

#[tokio::test]
async fn targeted_alert_missed_while_offline_replays_on_reconnect() {
    let manager = SseManager::new();
    let user = Uuid::new_v4();
    let other = Uuid::new_v4();

    // Broadcast cursor, established while both users are connected.
    let (tx_user, mut rx_user) = manager.register(user).await;
    let (_tx_other, mut rx_other) = manager.register(other).await;
    let broadcast = AlertEvent {
        user_ids: Vec::new(),
        ..alert_for(user, "broadcast")
    };
    manager.dispatch_alert(&broadcast).await;
    let cursor = rx_user.try_recv().expect("broadcast for user").id;
    let _ = rx_other.try_recv().expect("broadcast for other");

    // The user disconnects, then an alert addressed only to them is dispatched.
    manager.unregister(user, &tx_user).await;
    drop(rx_user);
    manager.dispatch_alert(&alert_for(user, "offline")).await;

    // The other user's reconnect with the same cursor sees nothing.
    let (_tx_other2, mut rx_other2) = manager
        .register_with_last_event_id(other, Some(&cursor))
        .await;
    assert!(rx_other2.try_recv().is_err(), "no cross-user replay");

    // The addressed user reconnects and resumes with the missed alert.
    let (_tx_user2, mut rx_user2) = manager
        .register_with_last_event_id(user, Some(&cursor))
        .await;
    let replayed = rx_user2.try_recv().expect("offline alert replays");
    assert_eq!(replayed.event, "new_warning");
}

#[tokio::test]
async fn first_connection_never_replays_stale_events() {
    let manager = SseManager::new();
    let other = Uuid::new_v4();
    let (_tx, mut rx) = manager.register(other).await;
    manager
        .dispatch_alert(&alert_for(other, "already-happened"))
        .await;
    let _ = drain(&mut rx);

    let (_tx, mut fresh) = manager.register(Uuid::new_v4()).await;
    assert!(
        fresh.try_recv().is_err(),
        "a first connection has no cursor and must not replay"
    );
}
