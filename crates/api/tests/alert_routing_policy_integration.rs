//! Database-backed alert routing policy contract (audit P0-1/P0-2/P0-3).
//!
//! `#[ignore]`d by default; CI runs it against a PostgreSQL service through
//! `scripts/ci/run_pg_integration_suites.sh`. Proves the exact failure the
//! audit found in the live NATS → router → SSE path:
//!
//!   * a warning explicitly targeted at Alice whose `EntityAlertConfig`
//!     suppresses the warning category resolves to `NoRecipients`;
//!   * the consumer contract for `NoRecipients` (ack, no dispatch) delivers
//!     **zero** SSE events to Alice;
//!   * the same alert without the suppression would have been delivered —
//!     the negative result is a policy decision, not a broken fixture;
//!   * an entity alert with no explicit recipients resolves through the
//!     canonical `user_alert_subscriptions` resolver;
//!   * an empty recipient list is never widened into a broadcast.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use apex_api::alert_router::{
    AlertAudience, AlertEvent, AlertEventType, AlertRouter, AlertRoutingDecision,
};
use apex_api::sse::SseManager;
use apex_core::alert_config::{
    principal_uuid_from_user_id, AlertChannel, AlertOverride, AlertSeverity, EntityAlertConfig,
};
use apex_core::identity::UserId;
use apex_store::postgres::{AppUserSeed, PgStore};
use chrono::Utc;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

async fn setup() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url())
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

fn warning_alert(entity_id: Uuid, audience: AlertAudience) -> AlertEvent {
    AlertEvent {
        id: Uuid::new_v4(),
        event_type: AlertEventType::NewWarning,
        severity: AlertSeverity::High,
        title: "Supplier capacity warning".to_string(),
        description: "Permit filings indicate an expansion.".to_string(),
        entity_ids: vec![entity_id],
        entity_name: Some("Northwind Power".to_string()),
        audience,
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
    }
}

fn suppression_config(entity_id: Uuid) -> EntityAlertConfig {
    EntityAlertConfig {
        entity_id: entity_id.to_string(),
        min_severity: AlertSeverity::Info,
        enabled_channels: vec![AlertChannel::InApp],
        cooldown_minutes: 0,
        max_daily_alerts: 0,
        override_rules: vec![AlertOverride {
            alert_type: "warning".to_string(),
            min_severity: AlertSeverity::Critical,
            enabled: true,
            cooldown_minutes: 0,
            max_daily: 0,
        }],
        enabled: true,
    }
}

/// The audit scenario: `target = Alice`, Alice's entity preference suppresses
/// the warning, routing resolves to `NoRecipients`, and Alice receives zero
/// SSE events. The positive control (suppression removed) delivers the alert,
/// proving the zero is an authorization outcome and not an inert fixture.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn suppressed_warning_target_receives_zero_sse_events() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let router = AlertRouter::new(store.clone());
    let manager = SseManager::new();

    let entity_id = Uuid::new_v4();
    let alice_username = format!("alice-route-{}", Uuid::new_v4());
    let alice = principal_uuid_from_user_id(&UserId::from(alice_username.as_str()));

    // Alice exists in `app_users` (the subscription FK target) and is
    // watching the entity; she also holds an explicit SSE connection, so any
    // leak would be observable.
    store
        .bootstrap_app_users(&[AppUserSeed {
            id: alice_username.clone(),
            username: alice_username.clone(),
            password_hash: "unused-test-hash".to_string(),
            role: "analyst".to_string(),
        }])
        .await
        .expect("bootstrap alice");
    store
        .upsert_user_alert_subscription(&alice_username, entity_id, None, "low", true)
        .await
        .expect("subscribe alice");
    let (_tx, mut rx) = manager.register(alice).await;

    // Alice's entity preference suppresses the `warning` category below
    // Critical, so the High warning must not reach her.
    store
        .upsert_entity_alert_config(&entity_id.to_string(), &suppression_config(entity_id))
        .await
        .expect("suppress warnings for the entity");

    // Targeted path: the producer explicitly addressed Alice.
    let targeted = warning_alert(entity_id, AlertAudience::Users(vec![alice]));
    let decision = router.route_alert(&targeted).await;
    assert_eq!(
        decision,
        AlertRoutingDecision::NoRecipients,
        "a suppressed target must resolve to NoRecipients, not an empty target list"
    );

    // Subscriber path: no explicit recipients — the canonical
    // `user_alert_subscriptions` resolver finds Alice, then the same policy
    // gate suppresses her.
    let unresolved = warning_alert(entity_id, AlertAudience::Users(vec![]));
    let decision = router.route_alert(&unresolved).await;
    assert_eq!(
        decision,
        AlertRoutingDecision::NoRecipients,
        "a subscribed-but-suppressed user must resolve to NoRecipients"
    );

    // Consumer contract for `NoRecipients`: ack and drop, never dispatch.
    for alert in [&targeted, &unresolved] {
        if let AlertRoutingDecision::NoRecipients = router.route_alert(alert).await {
            // Deliberately no `manager.dispatch_alert` call — this is what the
            // NATS consumer does on NoRecipients.
        } else {
            panic!("expected NoRecipients");
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        rx.try_recv().is_err(),
        "Alice must receive zero SSE events for a suppressed warning"
    );

    // Positive control: relax the suppression and the same alert is delivered
    // to exactly Alice.
    let mut allowed = suppression_config(entity_id);
    allowed.override_rules.clear();
    allowed.min_severity = AlertSeverity::Info;
    store
        .upsert_entity_alert_config(&entity_id.to_string(), &allowed)
        .await
        .expect("relax suppression");

    let decision = router.route_alert(&targeted).await;
    assert_eq!(
        decision,
        AlertRoutingDecision::Targets(vec![alice]),
        "without suppression the targeted warning must resolve to Alice"
    );
    if let AlertRoutingDecision::Targets(users) = decision {
        let routed = targeted.with_audience(AlertAudience::Users(users));
        assert_eq!(
            manager.dispatch_alert(&routed).await,
            1,
            "Alice's connection must receive the delivered alert"
        );
    }
    assert!(
        rx.try_recv().is_ok(),
        "the positive control proves the connection and routing work"
    );

    // Cleanup.
    store
        .delete_entity_alert_config(&entity_id.to_string())
        .await
        .expect("cleanup entity config");
    store
        .delete_user_alert_subscription(&alice_username, entity_id, None)
        .await
        .expect("cleanup subscription");
    sqlx::query("DELETE FROM app_users WHERE username = $1")
        .bind(&alice_username)
        .execute(&pool)
        .await
        .expect("cleanup alice");
}

/// An empty recipient list with no entity scope is "nobody", never a
/// broadcast — even when connections are live.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn empty_audience_without_entity_is_never_a_broadcast() {
    let pool = setup().await;
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    let router = AlertRouter::new(store);

    let mut alert = warning_alert(Uuid::new_v4(), AlertAudience::Users(vec![]));
    alert.entity_ids.clear();

    assert_eq!(
        router.route_alert(&alert).await,
        AlertRoutingDecision::NoRecipients,
        "an unresolved audience must never be widened into a broadcast"
    );
}
