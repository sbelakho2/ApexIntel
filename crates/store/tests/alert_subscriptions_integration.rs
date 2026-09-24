//! Opt-in integration test for `PgStore::find_subscribed_users` against a real
//! PostgreSQL database.
//!
//! The unit suite never touches a database, so this is the test that proves the
//! migration and the matching query agree. It is `#[ignore]`d by default (CI
//! runs it with `--ignored` against a Postgres service) and reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::alert_config::{user_principal_id, AlertSeverity};
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

async fn insert_subscription(
    pool: &sqlx::PgPool,
    user_id: &str,
    entity_id: Uuid,
    category: Option<&str>,
    min_severity: &str,
    enabled: bool,
) {
    sqlx::query(
        "INSERT INTO user_alert_subscriptions \
         (user_id, entity_id, category, min_severity, enabled) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(entity_id)
    .bind(category)
    .bind(min_severity)
    .bind(enabled)
    .execute(pool)
    .await
    .expect("insert subscription");
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn find_subscribed_users_returns_matching_subscribers_only() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    // Isolate the test from any other rows in a shared CI database.
    let entity_id = Uuid::new_v4();
    let other_entity = Uuid::new_v4();
    let marker = format!("audit-sub-{}", Uuid::new_v4());
    let user_low = format!("{marker}-low");
    let user_high = format!("{marker}-high");
    let user_critical = format!("{marker}-critical");
    let user_wrong_category = format!("{marker}-wrong-category");
    let user_disabled = format!("{marker}-disabled");
    let user_other_entity = format!("{marker}-other-entity");

    // Matching: 'warning' subscription at low severity.
    insert_subscription(&pool, &user_low, entity_id, Some("warning"), "low", true).await;
    // Matching: 'warning' subscription at high severity (alert is High).
    insert_subscription(&pool, &user_high, entity_id, Some("warning"), "high", true).await;
    // Not matching: threshold above the alert severity.
    insert_subscription(
        &pool,
        &user_critical,
        entity_id,
        Some("warning"),
        "critical",
        true,
    )
    .await;
    // Not matching: category filter.
    insert_subscription(
        &pool,
        &user_wrong_category,
        entity_id,
        Some("insight"),
        "low",
        true,
    )
    .await;
    // Not matching: disabled.
    insert_subscription(
        &pool,
        &user_disabled,
        entity_id,
        Some("warning"),
        "low",
        false,
    )
    .await;
    // Not matching: a different entity.
    insert_subscription(
        &pool,
        &user_other_entity,
        other_entity,
        Some("warning"),
        "low",
        true,
    )
    .await;
    // Matching: category-less subscription covers every category.
    insert_subscription(&pool, &user_high, entity_id, None, "medium", true).await;

    let matched = store
        .find_subscribed_users(entity_id, "warning", AlertSeverity::High)
        .await
        .expect("subscription lookup");

    let mut expected = vec![user_principal_id(&user_low), user_principal_id(&user_high)];
    expected.sort();
    let mut actual = matched.clone();
    actual.sort();
    assert_eq!(actual, expected, "unexpected subscriber set: {matched:?}");

    // No matching rows must be an empty list — the router turns that into
    // `Users([])`, i.e. nobody, never an implicit broadcast.
    let none = store
        .find_subscribed_users(entity_id, "warning", AlertSeverity::Info)
        .await
        .expect("subscription lookup");
    assert!(
        none.is_empty(),
        "Info alert is below every threshold; expected nobody, got {none:?}"
    );

    // Cleanup.
    sqlx::query(
        "DELETE FROM user_alert_subscriptions WHERE entity_id IN ($1, $2) OR user_id LIKE $3",
    )
    .bind(entity_id)
    .bind(other_entity)
    .bind(format!("{marker}%"))
    .execute(&pool)
    .await
    .unwrap();

    pool.close().await;
}
