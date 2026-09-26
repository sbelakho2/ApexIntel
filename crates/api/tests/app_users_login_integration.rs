//! Opt-in integration test for the canonical `app_users` login contract.
//!
//! `#[ignore]`d by default (CI runs it against a PostgreSQL service), reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`, and uses the real migrations.
//!
//! Proves the audit contract end to end:
//!   * environment credentials bootstrap the initial admin into `app_users`;
//!   * afterwards the database record is authoritative — rotating the
//!     environment password neither unlocks the old password nor replaces the
//!     stored hash;
//!   * disabled rows can never authenticate;
//!   * every successful login records `last_login_at`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::auth::ApiRole;
use apex_api::web::auth::{hash_password, resolve_login};
use apex_store::postgres::PgStore;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn bootstrap_admin_then_database_authoritative_login() {
    let url = database_url();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    let store = PgStore::from_pool(pool.clone());

    let marker = format!("login-it-{}", Uuid::new_v4());
    let username = format!("{marker}-admin");
    let password = "correct horse battery staple";
    let bootstrap_hash = hash_password(password).expect("hash password");

    let bootstrap_users = json!([{
        "id": marker,
        "username": username,
        "password_hash": bootstrap_hash,
        "role": "admin",
    }]);
    std::env::set_var("WEB_USERS_JSON", bootstrap_users.to_string());

    // The environment bootstraps the row: the caller is provisioned as admin
    // and can log in with the bootstrap password.
    let principal = resolve_login(Some(&store), &username, password)
        .await
        .expect("bootstrap admin must be able to log in");
    assert_eq!(principal.user_id.as_str(), marker);
    assert_eq!(principal.username.as_str(), username);
    assert_eq!(principal.role, ApiRole::Admin);

    // Rotating the environment password must not change the database record:
    // the old password still works and the rotated one does not.
    let rotated_hash = hash_password("rotated-env-password").expect("hash rotated password");
    std::env::set_var(
        "WEB_USERS_JSON",
        json!([{
            "id": marker,
            "username": username,
            "password_hash": rotated_hash,
            "role": "admin",
        }])
        .to_string(),
    );
    assert!(
        resolve_login(Some(&store), &username, "rotated-env-password")
            .await
            .is_none(),
        "the environment must not overwrite the stored credential"
    );
    assert!(
        resolve_login(Some(&store), &username, password)
            .await
            .is_some(),
        "the database credential stays authoritative"
    );

    // A disabled row can never mint a session.
    sqlx::query("UPDATE app_users SET enabled = FALSE WHERE id = $1")
        .bind(&marker)
        .execute(&pool)
        .await
        .expect("disable app user");
    assert!(resolve_login(Some(&store), &username, password)
        .await
        .is_none());

    // Re-enabling restores login and records `last_login_at`.
    sqlx::query("UPDATE app_users SET enabled = TRUE, last_login_at = NULL WHERE id = $1")
        .bind(&marker)
        .execute(&pool)
        .await
        .expect("re-enable app user");
    assert!(resolve_login(Some(&store), &username, password)
        .await
        .is_some());
    let last_login: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT last_login_at FROM app_users WHERE id = $1")
            .bind(&marker)
            .fetch_one(&pool)
            .await
            .expect("read last_login_at");
    assert!(last_login.is_some(), "successful login records the time");

    // Unknown principals fail closed even while environment users exist.
    assert!(
        resolve_login(Some(&store), "not-a-configured-user", "irrelevant")
            .await
            .is_none()
    );

    sqlx::query("DELETE FROM app_users WHERE id = $1")
        .bind(&marker)
        .execute(&pool)
        .await
        .expect("cleanup app user");
    std::env::remove_var("WEB_USERS_JSON");
    pool.close().await;
}
