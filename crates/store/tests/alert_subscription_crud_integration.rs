//! Opt-in integration test for the alert-subscription CRUD store methods and
//! the `app_users` identity guarantees added by migration 059.
//!
//! `#[ignore]`d by default (CI runs it with `--ignored` against a Postgres
//! service), reads `TEST_DATABASE_URL` or `DATABASE_URL`, and follows the same
//! pattern as `migrations_integration.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{AppUserSeed, PgStore};
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
async fn subscription_crud_round_trip() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let marker = format!("audit-crud-{}", Uuid::new_v4());
    let entity_id = Uuid::new_v4();
    let other_entity = Uuid::new_v4();

    // The canonical identity must exist before a subscription can reference it.
    // Environment credentials bootstrap the row; the database then wins.
    let seed = AppUserSeed {
        id: marker.clone(),
        username: format!("{marker}-name"),
        password_hash: "bootstrap-hash".to_string(),
        role: "analyst".to_string(),
    };
    let applied = store
        .bootstrap_app_users(std::slice::from_ref(&seed))
        .await
        .expect("bootstrap_app_users creates the canonical identity");
    assert_eq!(applied, 1);
    let app_user = store
        .get_app_user(&marker)
        .await
        .expect("get_app_user")
        .expect("identity exists");
    assert_eq!(app_user.id, marker);
    assert!(app_user.enabled);
    assert_eq!(app_user.password_hash.as_deref(), Some("bootstrap-hash"));

    // Migrations can be re-applied without error and without duplicating the
    // identity (idempotency of 059/061).
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let seeded_again = AppUserSeed {
        password_hash: "changed-env-hash".to_string(),
        role: "admin".to_string(),
        ..seed.clone()
    };
    let applied = store
        .bootstrap_app_users(std::slice::from_ref(&seeded_again))
        .await
        .expect("bootstrap_app_users is idempotent");
    assert_eq!(
        applied, 0,
        "a row that already carries credentials must not be re-bootstrapped"
    );

    // Database-authoritative: a successful login records `last_login_at`
    // without letting the environment rewrite the stored credentials.
    let logged_in = store
        .record_app_user_login(&marker)
        .await
        .expect("record_app_user_login")
        .expect("login row");
    assert!(logged_in.last_login_at.is_some());
    assert_eq!(logged_in.password_hash.as_deref(), Some("bootstrap-hash"));
    assert_eq!(logged_in.role, "analyst");

    // `ensure_app_user_exists` is insert-only: it must not overwrite an
    // existing verified identity (the PUT backstop and API-key provisioning
    // use it instead of the login upsert).
    store
        .ensure_app_user_exists(&marker, "spoofed-username", "admin")
        .await
        .expect("ensure_app_user_exists on an existing row");
    let unchanged = store
        .get_app_user(&marker)
        .await
        .expect("get_app_user")
        .expect("identity exists");
    assert_eq!(unchanged.username, format!("{marker}-name"));
    assert_eq!(unchanged.role, "analyst");
    let provisioned_other = format!("{marker}-provisioned");
    store
        .ensure_app_user_exists(&provisioned_other, "Provisioned User", "viewer")
        .await
        .expect("ensure_app_user_exists inserts a missing row");
    assert_eq!(
        store
            .get_app_user(&provisioned_other)
            .await
            .expect("get_app_user")
            .expect("identity exists")
            .role,
        "viewer"
    );

    // Create.
    let created = store
        .upsert_user_alert_subscription(&marker, entity_id, Some("Warning"), "HIGH", true)
        .await
        .expect("create subscription");
    assert_eq!(created.user_id, marker);
    assert_eq!(created.entity_id, entity_id);
    assert_eq!(created.category.as_deref(), Some("warning"));
    assert_eq!(created.min_severity, "high");
    assert!(created.enabled);

    // Read back.
    let fetched = store
        .get_user_alert_subscription(&marker, entity_id, Some("WARNING"))
        .await
        .expect("get subscription")
        .expect("subscription exists");
    assert_eq!(fetched.id, created.id);

    // Upsert on the natural key updates in place (case-insensitive category).
    let updated = store
        .upsert_user_alert_subscription(&marker, entity_id, Some("warning"), "critical", false)
        .await
        .expect("update subscription");
    assert_eq!(updated.id, created.id, "upsert must not duplicate the row");
    assert_eq!(updated.min_severity, "critical");
    assert!(!updated.enabled);

    // The "all categories" row is a distinct natural key.
    let all_categories = store
        .upsert_user_alert_subscription(&marker, entity_id, None, "medium", true)
        .await
        .expect("create all-categories subscription");
    assert!(all_categories.category.is_none());
    assert_ne!(all_categories.id, created.id);

    // List returns every row for the user, and only that user's rows.
    let listed = store
        .list_user_alert_subscriptions(&marker)
        .await
        .expect("list subscriptions");
    assert_eq!(listed.len(), 2);

    // The entity-scoped listing powers the per-entity GET without reading the
    // user's whole subscription set.
    let entity_rows = store
        .list_user_alert_subscriptions_for_entity(&marker, entity_id)
        .await
        .expect("list subscriptions for entity");
    assert_eq!(entity_rows.len(), 2);
    assert!(entity_rows.iter().all(|row| row.entity_id == entity_id));
    assert!(store
        .list_user_alert_subscriptions_for_entity(&marker, other_entity)
        .await
        .expect("list subscriptions for other entity")
        .is_empty());

    // Delete is natural-key scoped.
    assert!(store
        .delete_user_alert_subscription(&marker, entity_id, Some("warning"))
        .await
        .expect("delete category subscription"));
    assert!(!store
        .delete_user_alert_subscription(&marker, entity_id, Some("warning"))
        .await
        .expect("second delete is a no-op"));
    assert!(store
        .delete_user_alert_subscription(&marker, entity_id, None)
        .await
        .expect("delete all-categories subscription"));
    assert!(store
        .list_user_alert_subscriptions(&marker)
        .await
        .expect("list after delete")
        .is_empty());

    // Identity integrity: unknown principals are rejected by the FK ...
    let unknown = format!("{marker}-unknown");
    let orphan = store
        .upsert_user_alert_subscription(&unknown, entity_id, None, "medium", true)
        .await;
    assert!(
        orphan.is_err(),
        "migration 059 must reject a subscription for an unknown principal"
    );

    // ... and deleting the canonical identity cascades to its subscriptions.
    store
        .upsert_user_alert_subscription(&marker, other_entity, None, "low", true)
        .await
        .expect("re-create a subscription for cascade test");
    sqlx::query("DELETE FROM app_users WHERE id = $1")
        .bind(&marker)
        .execute(&pool)
        .await
        .expect("delete canonical identity");
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_alert_subscriptions WHERE user_id = $1")
            .bind(&marker)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0, "ON DELETE CASCADE must remove subscriptions");

    // Cleanup the provisioned-only identity used by the insert-only check.
    sqlx::query("DELETE FROM app_users WHERE id = $1")
        .bind(&provisioned_other)
        .execute(&pool)
        .await
        .expect("delete provisioned identity");

    pool.close().await;
}
