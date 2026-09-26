//! Opt-in integration test proving RLS is bound to the application identity
//! set by `PgStore::begin_scoped` (audit P0 #7), and that migrations 057/058
//! force row-level security on the user-private tables.
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
    for table in [
        "user_preferences",
        "watchlists",
        "annotations",
        "insight_bookmarks",
        "app_users",
        // Annotation writes replace their tag assignments in the same
        // transaction, so the scoped role needs these tag tables too.
        "tags",
        "tag_assignments",
    ] {
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
        sqlx::query("DELETE FROM annotations WHERE user_id = $1")
            .bind(user)
            .execute(admin)
            .await
            .unwrap();
        sqlx::query("DELETE FROM insight_bookmarks WHERE user_id = $1")
            .bind(user)
            .execute(admin)
            .await
            .unwrap();
        sqlx::query("DELETE FROM app_users WHERE id = $1")
            .bind(user)
            .execute(admin)
            .await
            .unwrap();
    }
}

/// Migration 059 gives `user_preferences`/`watchlists` an
/// `app_users(id) ON DELETE CASCADE` foreign key, so direct inserts in this
/// test must have a canonical identity row first.
async fn seed_app_users(admin: &PgPool, users: &[&str]) {
    for user in users {
        sqlx::query(
            "INSERT INTO app_users (id, username, display_name, role) \
             VALUES ($1, $1, $1, 'analyst') ON CONFLICT (id) DO NOTHING",
        )
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
    seed_app_users(&admin, &[user_a, user_b, user_c]).await;

    for user in [user_a, user_b, user_c] {
        sqlx::query(
            "INSERT INTO analyst_users (id, display_name, role) VALUES ($1, $1, 'analyst') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(user)
        .execute(&admin)
        .await
        .unwrap();
    }
    let insight_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO insights (id, insight_type, title) \
         VALUES ($1, 'test_signal', 'RLS scoped test insight')",
    )
    .bind(insight_id)
    .execute(&admin)
    .await
    .unwrap();

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

    // Annotations are force-scoped by migration 058 (reconciled from the
    // legacy author_id/content shape).
    scoped
        .upsert_annotation_scoped(
            user_a,
            "analyst",
            None,
            "warning",
            "w-1",
            "note A",
            &[],
            "private",
        )
        .await
        .unwrap();
    scoped
        .upsert_annotation_scoped(
            user_b,
            "analyst",
            None,
            "warning",
            "w-1",
            "note B",
            &[],
            "private",
        )
        .await
        .unwrap();
    let a_notes = scoped
        .list_annotations_scoped(user_a, "analyst", Some("warning"), Some("w-1"))
        .await
        .unwrap();
    assert!(
        a_notes.iter().all(|note| note.user_id == user_a),
        "user A must not see user B's private annotation"
    );
    assert!(a_notes.iter().any(|note| note.body == "note A"));
    assert!(!a_notes.iter().any(|note| note.body == "note B"));

    let mut tx = scoped.begin_scoped(user_a, "analyst").await.unwrap();
    let cross_user_note = sqlx::query(
        "INSERT INTO annotations (user_id, entity_type, entity_id, body) \
         VALUES ($1, 'warning', 'w-1', 'hack')",
    )
    .bind(user_b)
    .execute(&mut *tx)
    .await;
    assert!(
        cross_user_note.is_err(),
        "RLS must reject an annotation written for another user"
    );
    tx.rollback().await.unwrap();

    // Insight bookmarks are force-scoped by migration 058.
    assert!(scoped
        .bookmark_insight_scoped(insight_id, user_a, "analyst", None)
        .await
        .unwrap());
    assert!(scoped
        .is_insight_bookmarked_scoped(insight_id, user_a, "analyst")
        .await
        .unwrap());
    assert!(!scoped
        .is_insight_bookmarked_scoped(insight_id, user_b, "analyst")
        .await
        .unwrap());
    assert_eq!(
        scoped
            .get_bookmarked_insight_ids_scoped(user_a, "analyst", &[insight_id])
            .await
            .unwrap(),
        vec![insight_id]
    );
    assert!(scoped
        .get_bookmarked_insight_ids_scoped(user_b, "analyst", &[insight_id])
        .await
        .unwrap()
        .is_empty());

    let mut tx = scoped.begin_scoped(user_a, "analyst").await.unwrap();
    let visible: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM insight_bookmarks ORDER BY user_id")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(visible, vec![user_a.to_string()]);
    let cross_user_bookmark =
        sqlx::query("INSERT INTO insight_bookmarks (insight_id, user_id) VALUES ($1, $2)")
            .bind(insight_id)
            .bind(user_b)
            .execute(&mut *tx)
            .await;
    assert!(
        cross_user_bookmark.is_err(),
        "RLS must reject a bookmark written for another user"
    );
    tx.rollback().await.unwrap();

    assert!(scoped
        .unbookmark_insight_scoped(insight_id, user_a, "analyst")
        .await
        .unwrap());
    assert!(!scoped
        .is_insight_bookmarked_scoped(insight_id, user_a, "analyst")
        .await
        .unwrap());

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
        "annotations",
        "insight_bookmarks",
        "export_history",
        "analyst_notifications",
        "insight_feedback_events",
        "insight_bookmark_collections",
        "user_alert_subscriptions",
        "priority_queue",
        "daily_priority_queue",
        "alert_preferences",
        "notification_preferences",
        "bookmarks",
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
        assert!(
            forced,
            "{table} must have FORCE ROW LEVEL SECURITY (service paths carry identity)"
        );
    }

    // Shared/system tables whose `user_id` is an actor reference rather than a
    // privacy boundary stay out of scope (migration 058 classification).
    for table in [
        "workspace_assignments",
        "api_key_owners",
        "analyst_user_roles",
        "weekly_memo_recipients",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = $1)",
        )
        .bind(table)
        .fetch_one(&admin)
        .await
        .unwrap();
        assert!(exists, "{table} must exist after migrations");
    }

    admin.close().await;
}

/// The worker builds its own pool and runs unscoped service paths. After
/// migrations 057/058 FORCE RLS, such a pool must assume the `service`
/// identity via `PgStore::assume_service_identity`, or every newly forced
/// table silently returns zero rows / rejects writes.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn forced_worker_tables_require_a_service_identity() {
    let url = database_url();
    let admin = connect(&url).await;
    sqlx::migrate!("../../migrations")
        .run(&admin)
        .await
        .unwrap();

    let role_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(RLS_ROLE)
            .fetch_one(&admin)
            .await
            .unwrap();
    if !role_exists {
        sqlx::query(&format!("CREATE ROLE {RLS_ROLE}"))
            .execute(&admin)
            .await
            .expect("create scoped test role (needs CREATEROLE)");
    }
    for table in ["analyst_notifications", "insight_feedback_events"] {
        sqlx::query(&format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {table} TO {RLS_ROLE}"
        ))
        .execute(&admin)
        .await
        .expect("grant DML on forced table");
    }

    let worker_user = "rls-worker-service-user";

    // A non-owner role with no identity is denied by the FORCEd RLS.
    let no_identity = PgStore::from_pool(
        PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("SET ROLE apexintel_rls_test")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap(),
    );
    assert!(
        no_identity
            .create_notification(
                worker_user,
                "rls_test",
                "no identity",
                "must be rejected",
                None,
                None,
                None,
            )
            .await
            .is_err(),
        "a connection with no identity must not write a forced table"
    );

    // The same role with the worker's service identity can write and scan.
    let service = PgStore::from_pool(
        PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("SET ROLE apexintel_rls_test")
                        .execute(&mut *conn)
                        .await?;
                    PgStore::assume_service_identity(&mut *conn).await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap(),
    );
    service
        .create_notification(
            worker_user,
            "rls_test",
            "service identity",
            "must be accepted",
            None,
            None,
            None,
        )
        .await
        .expect("service identity must satisfy the forced service policy");
    service
        .list_recent_insight_feedback_events(chrono::Utc::now() - chrono::Duration::days(90))
        .await
        .expect("service identity must be able to scan forced feedback events");

    sqlx::query("DELETE FROM analyst_notifications WHERE user_id = $1")
        .bind(worker_user)
        .execute(&admin)
        .await
        .unwrap();

    admin.close().await;
}
