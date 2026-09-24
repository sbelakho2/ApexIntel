//! Opt-in integration test proving RLS is bound to the application identity
//! set by `PgStore::begin_scoped` (audit P0 #7), and that migration 051 forces
//! row-level security on the user-private tables.
//!
//! `#[ignore]`d by default like `migrations_integration.rs`; reads
//! `TEST_DATABASE_URL` or `DATABASE_URL` and is intended to run against a
//! disposable PostgreSQL database owned by an administrative role.
//!
//! Scoped access is exercised through a dedicated, non-superuser,
//! non-owner role so the policies actually apply: `SET ROLE` in the pool's
//! `after_connect` makes every scoped connection run as that role. The
//! unscoped service path is exercised on the administrative connection.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{PgStore, UserSettingsPrefs};
use serde_json::json;
use sqlx::postgres::{PgPool, PgPoolOptions};

const RLS_ROLE: &str = "apexintel_rls_test";

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
        .connect(url)
        .await
        .expect("connect to postgres")
}

async fn scoped_store(admin: &PgPool, url: &str) -> PgStore {
    let role_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(RLS_ROLE)
            .fetch_one(admin)
            .await
            .unwrap();
    if !role_exists {
        sqlx::query(&format!("CREATE ROLE {RLS_ROLE}"))
            .execute(admin)
            .await
            .expect("create scoped test role (needs CREATEROLE)");
    }
    for table in ["user_preferences", "watchlists"] {
        sqlx::query(&format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {table} TO {RLS_ROLE}"
        ))
        .execute(admin)
        .await
        .expect("grant DML on user-private table");
    }

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET ROLE apexintel_rls_test")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .expect("connect scoped pool");
    PgStore::from_pool(pool)
}

async fn cleanup(admin: &PgPool, users: &[&str]) {
    for user in users {
        sqlx::query("DELETE FROM user_preferences WHERE user_id = $1")
            .bind(user)
            .execute(admin)
            .await
            .unwrap();
        sqlx::query("DELETE FROM watchlists WHERE user_id = $1")
            .bind(user)
            .execute(admin)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn scoped_access_isolates_users_but_service_path_still_works() {
    let url = database_url();
    let admin = connect(&url).await;
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .unwrap();

    let user_a = "rls-scoped-user-a";
    let user_b = "rls-scoped-user-b";
    let user_c = "rls-scoped-user-c";
    cleanup(&admin, &[user_a, user_b, user_c]).await;

    sqlx::query(
        "INSERT INTO user_preferences (user_id, theme, locale, preferences) VALUES \
         ($1, 'dark-a', 'en', '{\"settings_page\":{\"email_digest_enabled\":true}}'::jsonb), \
         ($2, 'dark-b', 'fr', '{\"settings_page\":{\"email_digest_enabled\":false}}'::jsonb)",
    )
    .bind(user_a)
    .bind(user_b)
    .execute(&admin)
    .await
    .unwrap();

    let scoped = scoped_store(&admin, &url).await;

    // Each principal reads its own preferences record.
    let a = scoped
        .get_user_preferences_record_scoped(user_a, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.theme, "dark-a");
    let b = scoped
        .get_user_preferences_record_scoped(user_b, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.theme, "dark-b");
    assert!(scoped
        .get_user_preferences_record_scoped(user_c, "analyst")
        .await
        .unwrap()
        .is_none());

    // Inside user A's scope the database itself hides every other user's row,
    // and cross-user writes are refused even when the SQL targets them.
    let mut tx = scoped.begin_scoped(user_a, "analyst").await.unwrap();
    let visible: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM user_preferences ORDER BY user_id")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(visible, vec![user_a.to_string()]);

    let updated = sqlx::query("UPDATE user_preferences SET theme = 'hacked' WHERE user_id = $1")
        .bind(user_b)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(updated.rows_affected(), 0);

    let deleted = sqlx::query("DELETE FROM user_preferences WHERE user_id = $1")
        .bind(user_b)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(deleted.rows_affected(), 0);

    let inserted_other =
        sqlx::query("INSERT INTO user_preferences (user_id, theme) VALUES ($1, 'hacked')")
            .bind(user_c)
            .execute(&mut *tx)
            .await;
    assert!(
        inserted_other.is_err(),
        "RLS must reject inserting a preferences row for another user"
    );
    tx.rollback().await.unwrap();

    // Scoped writes stay on the caller's own row.
    scoped
        .upsert_user_preferences_record_scoped(
            user_a,
            "analyst",
            "light-a",
            "de",
            &json!({"table_columns": {"warnings": ["severity"]}}),
        )
        .await
        .unwrap();
    let a = scoped
        .get_user_preferences_record_scoped(user_a, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.theme, "light-a");
    assert_eq!(a.locale, "de");
    let b = scoped
        .get_user_preferences_record_scoped(user_b, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.theme, "dark-b", "user B's row must be untouched");

    let settings = UserSettingsPrefs {
        email_digest_enabled: true,
        ..UserSettingsPrefs::default()
    };
    scoped
        .upsert_user_settings_prefs_scoped(user_a, "analyst", "system", "en", &settings)
        .await
        .unwrap();
    let loaded = scoped
        .get_user_settings_prefs_scoped(user_a, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.email_digest_enabled);
    let b_settings = scoped
        .get_user_settings_prefs_scoped(user_b, "analyst")
        .await
        .unwrap()
        .unwrap();
    assert!(!b_settings.email_digest_enabled);

    // Watchlists behave the same way.
    let watch_a = scoped
        .upsert_watchlist_scoped(
            user_a,
            "analyst",
            None,
            "A list",
            &json!([{"type": "company", "id": "acme"}]),
            None,
        )
        .await
        .unwrap();
    let watch_b = scoped
        .upsert_watchlist_scoped(user_b, "analyst", None, "B list", &json!([]), None)
        .await
        .unwrap();
    assert_ne!(watch_a.id, watch_b.id);

    let a_lists = scoped
        .list_watchlists_scoped(user_a, "analyst")
        .await
        .unwrap();
    assert_eq!(a_lists.len(), 1);
    assert_eq!(a_lists[0].id, watch_a.id);
    assert!(a_lists.iter().all(|w| w.user_id == user_a));

    let mut tx = scoped.begin_scoped(user_a, "analyst").await.unwrap();
    let visible: Vec<String> = sqlx::query_scalar("SELECT user_id FROM watchlists")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(visible, vec![user_a.to_string()]);
    let inserted_other = sqlx::query(
        "INSERT INTO watchlists (user_id, name, entities) VALUES ($1, 'hack', '[]'::jsonb)",
    )
    .bind(user_b)
    .execute(&mut *tx)
    .await;
    assert!(
        inserted_other.is_err(),
        "RLS must reject inserting a watchlist for another user"
    );
    tx.rollback().await.unwrap();

    assert!(!scoped
        .delete_watchlist_scoped(user_a, "analyst", watch_b.id)
        .await
        .unwrap());
    assert!(scoped
        .delete_watchlist_scoped(user_a, "analyst", watch_a.id)
        .await
        .unwrap());
    let b_lists = scoped
        .list_watchlists_scoped(user_b, "analyst")
        .await
        .unwrap();
    assert_eq!(b_lists.len(), 1);
    assert_eq!(b_lists[0].id, watch_b.id);

    // The unscoped service path (administrative connection) still reads and
    // writes across users, for the worker/digest flows that set no identity.
    let service = PgStore::from_pool(admin.clone());
    let b_record = service.get_user_preferences_record(user_b).await.unwrap();
    assert_eq!(b_record.unwrap().theme, "dark-b");
    assert_eq!(service.list_watchlists(user_b).await.unwrap().len(), 1);
    service
        .upsert_watchlist(None, user_c, "C list", &json!([]), None)
        .await
        .unwrap();
    assert_eq!(service.list_watchlists(user_c).await.unwrap().len(), 1);
    service
        .upsert_user_preferences_record(user_c, "system", "en", &json!({}))
        .await
        .unwrap();
    assert!(service
        .get_user_preferences_record(user_c)
        .await
        .unwrap()
        .is_some());

    // Ownership is part of the write itself: even on a connection that
    // bypasses RLS (superuser/admin/service), user A cannot upsert or update
    // a watchlist id owned by user B.
    let bypass = PgStore::from_pool(admin.clone());
    let overwrite = bypass
        .upsert_watchlist_scoped(
            user_a,
            "analyst",
            Some(watch_b.id),
            "hacked",
            &json!([]),
            None,
        )
        .await;
    assert!(
        overwrite.is_err(),
        "cross-user watchlist upsert must be rejected even without RLS"
    );
    let updated_other = bypass
        .update_watchlist_scoped(user_a, "analyst", watch_b.id, "hacked", &json!([]), None)
        .await
        .unwrap();
    assert!(
        updated_other.is_none(),
        "updating another user's watchlist must not match a row"
    );
    let b_lists = service.list_watchlists(user_b).await.unwrap();
    assert_eq!(b_lists.len(), 1);
    assert_eq!(b_lists[0].name, "B list");

    cleanup(&admin, &[user_a, user_b, user_c]).await;
    scoped.pool.close().await;
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn user_private_tables_force_row_level_security() {
    let url = database_url();
    let admin = connect(&url).await;
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .unwrap();

    for table in [
        "user_preferences",
        "watchlists",
        "saved_searches",
        "notifications",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = $1)",
        )
        .bind(table)
        .fetch_one(&admin)
        .await
        .unwrap();
        assert!(exists, "{table} must exist after migrations");
        let (enabled, forced): (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE relname = $1",
        )
        .bind(table)
        .fetch_one(&admin)
        .await
        .unwrap();
        assert!(enabled, "{table} must have RLS enabled");
        assert!(forced, "{table} must have FORCE ROW LEVEL SECURITY");
    }

    // These stay ENABLE-only until their unscoped call sites are migrated to
    // `begin_scoped`; forcing them now would deny the owner the live web paths.
    for table in ["annotations", "insight_bookmarks"] {
        let (enabled, forced): (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE relname = $1",
        )
        .bind(table)
        .fetch_one(&admin)
        .await
        .unwrap();
        assert!(enabled, "{table} must have RLS enabled");
        assert!(
            !forced,
            "{table} must not be forced until its call sites are scoped"
        );
    }

    admin.close().await;
}
