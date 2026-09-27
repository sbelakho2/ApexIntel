//! Opt-in integration tests for the auth-hardening audit items:
//!
//!   * migration 069 — the login throttle is durable and shared across store
//!     instances (simulated replicas and restarts);
//!   * migration 070 — credential-bearing login names are unique and the
//!     migration refuses ambiguous duplicates instead of guessing;
//!   * migration 071 — `app_users.role` is constrained to the four known roles.
//!
//! `#[ignore]`d by default (CI runs them with `--ignored` against a PostgreSQL
//! service) and reads `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::login_throttle::LOGIN_ADMIN_LOCK_SECS;
use apex_store::postgres::PgStore;
use chrono::{Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

async fn connect(url: &str) -> sqlx::PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .expect("connect to postgres")
}

async fn migrated_pool(url: &str) -> sqlx::PgPool {
    let pool = connect(url).await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Execute a migration file on a fresh connection.
///
/// A migration that refuses (RAISE EXCEPTION inside its transaction) leaves
/// the session in an aborted transaction; running it through the shared pool
/// would poison the connection for every later test. A dedicated connection
/// is reset and closed instead.
async fn run_migration_sql(url: &str, sql: &str) -> Result<(), sqlx::Error> {
    use sqlx::Connection as _;

    let mut connection = sqlx::PgConnection::connect(url)
        .await
        .expect("connect for migration SQL");
    let result = sqlx::raw_sql(sql)
        .execute(&mut connection)
        .await
        .map(|_| ());
    if result.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
    }
    let _ = connection.close().await;
    result
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn durable_login_throttle_blocks_and_survives_restart() {
    let url = database_url();
    let pool_a = migrated_pool(&url).await;
    let store_a = PgStore::from_pool(pool_a.clone());
    let key = format!("throttle-it-{}", Uuid::new_v4());
    let now = Utc::now();

    // Progressive backoff is observable through the durable backend.
    let first = store_a
        .login_throttle_record_failure(&key, now)
        .await
        .expect("record first failure");
    assert!(!first.allowed);
    assert_eq!(first.retry_after_secs, 1);

    let second = store_a
        .login_throttle_record_failure(&key, now + Duration::seconds(1))
        .await
        .expect("record second failure");
    assert_eq!(second.retry_after_secs, 2);

    // A second store instance (separate pool, same database) shares the active
    // backoff: the second failure's deadline is now + 3s.
    let pool_b = connect(&url).await;
    let store_b = PgStore::from_pool(pool_b.clone());
    let shared = store_b
        .login_throttle_evaluate(&key, now + Duration::seconds(2))
        .await
        .expect("evaluate through second instance");
    assert!(
        !shared.allowed,
        "the backoff must be visible to every replica"
    );
    assert_eq!(shared.retry_after_secs, 1);

    // Lock after 10 failures in the 10-minute window.
    for offset in 2..10 {
        store_a
            .login_throttle_record_failure(&key, now + Duration::seconds(offset))
            .await
            .expect("record failure");
    }
    let locked = store_b
        .login_throttle_evaluate(&key, now + Duration::seconds(10))
        .await
        .expect("evaluate lockout");
    assert!(!locked.allowed);
    assert!(locked.retry_after_secs >= 590);
    assert_eq!(locked.failure_count_10m, 10);

    // "Restart": close the first pool, open a fresh pool and observe the lock.
    pool_a.close().await;
    let pool_c = connect(&url).await;
    let store_c = PgStore::from_pool(pool_c.clone());
    let after_restart = store_c
        .login_throttle_evaluate(&key, now + Duration::seconds(11))
        .await
        .expect("evaluate after restart");
    assert!(
        !after_restart.allowed,
        "a durable lockout must survive a process restart"
    );
    assert_eq!(after_restart.failure_count_10m, 10);

    // The window decays after 10 minutes and a success clears everything.
    assert!(
        store_c
            .login_throttle_evaluate(&key, now + Duration::minutes(11))
            .await
            .expect("evaluate after window")
            .allowed,
        "the 10-minute window must decay"
    );
    store_c
        .login_throttle_record_failure(&key, now + Duration::minutes(11))
        .await
        .expect("record after window");
    store_c
        .login_throttle_record_success(&key)
        .await
        .expect("clear on success");
    let cleared = store_b
        .login_throttle_evaluate(&key, now + Duration::minutes(12))
        .await
        .expect("evaluate cleared state");
    assert!(cleared.allowed);
    assert_eq!(cleared.failure_count_10m, 0);

    sqlx::query("DELETE FROM login_attempt_throttle WHERE attempt_key = $1")
        .bind(&key)
        .execute(&pool_c)
        .await
        .unwrap();
    pool_b.close().await;
    pool_c.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn durable_login_throttle_requires_admin_unlock() {
    let url = database_url();
    let pool = migrated_pool(&url).await;
    let store = PgStore::from_pool(pool.clone());
    let key = format!("throttle-admin-it-{}", Uuid::new_v4());
    let now = Utc::now();

    for offset in 0..20 {
        store
            .login_throttle_record_failure(&key, now + Duration::minutes(offset))
            .await
            .expect("record failure");
    }
    let locked = store
        .login_throttle_evaluate(&key, now + Duration::minutes(20))
        .await
        .expect("evaluate admin lock");
    assert!(locked.admin_unlock_required);
    assert!(!locked.allowed);

    // Still locked after the windows would have decayed: only an admin clears it.
    assert!(
        !store
            .login_throttle_evaluate(&key, now + Duration::hours(3))
            .await
            .expect("evaluate long after lock")
            .allowed
    );
    assert!(store
        .login_throttle_clear_lock(&key)
        .await
        .expect("clear admin lock"));
    assert!(
        store
            .login_throttle_evaluate(&key, now + Duration::hours(3))
            .await
            .expect("evaluate after unlock")
            .allowed
    );
    assert!(
        !store
            .login_throttle_clear_lock(&key)
            .await
            .expect("clearing absent state is not an error"),
        "clear_lock reports whether state existed"
    );

    // The strongest lock is bounded: it releases on its own and its row is
    // swept, so unauthenticated failures cannot mint permanent rows.
    let expiring_key = format!("throttle-admin-expiry-it-{}", Uuid::new_v4());
    for offset in 0..20 {
        store
            .login_throttle_record_failure(&expiring_key, now + Duration::minutes(offset))
            .await
            .expect("record failure");
    }
    let expiry = now + Duration::minutes(19) + Duration::seconds(LOGIN_ADMIN_LOCK_SECS);
    assert!(
        !store
            .login_throttle_evaluate(&expiring_key, expiry - Duration::seconds(60))
            .await
            .expect("evaluate just before the deadline")
            .allowed
    );
    assert!(
        store
            .login_throttle_evaluate(&expiring_key, expiry + Duration::seconds(1))
            .await
            .expect("evaluate after the deadline")
            .allowed,
        "the bounded admin lock must release without manual intervention"
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM login_attempt_throttle WHERE attempt_key = $1")
            .bind(&expiring_key)
            .fetch_one(&pool)
            .await
            .expect("count expired admin rows");
    assert_eq!(remaining, 0, "the expired admin row must be swept");

    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn unique_login_name_index_rejects_duplicate_credentials() {
    let url = database_url();
    let pool = migrated_pool(&url).await;
    let suffix = Uuid::new_v4();
    let username = format!("canon-{suffix}");
    let first_id = format!("canon-first-{suffix}");
    let second_id = format!("canon-second-{suffix}");

    sqlx::query("INSERT INTO app_users (id, username, password_hash) VALUES ($1, $2, 'hash')")
        .bind(&first_id)
        .bind(&username)
        .execute(&pool)
        .await
        .expect("first credential row is accepted");

    let duplicate =
        sqlx::query("INSERT INTO app_users (id, username, password_hash) VALUES ($1, $2, 'hash')")
            .bind(&second_id)
            .bind(username.to_uppercase())
            .execute(&pool)
            .await;
    let error = duplicate.expect_err("duplicate login name must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("uq_app_users_username_ci") || message.contains("duplicate key"),
        "unexpected error: {message}"
    );

    sqlx::query("DELETE FROM app_users WHERE id IN ($1, $2)")
        .bind(&first_id)
        .bind(&second_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn find_app_user_by_username_refuses_ambiguous_rows() {
    let url = database_url();
    let pool = migrated_pool(&url).await;
    let store = PgStore::from_pool(pool.clone());
    let suffix = Uuid::new_v4();
    let username = format!("ambig-lookup-{suffix}");
    let first_id = format!("ambig-lookup-a-{suffix}");
    let second_id = format!("ambig-lookup-b-{suffix}");

    // The unique index prevents this state in production; drop it to prove the
    // lookup itself fails closed if a schema bypassed migration 070.
    sqlx::query("DROP INDEX IF EXISTS uq_app_users_username_ci")
        .execute(&pool)
        .await
        .unwrap();
    for id in [&first_id, &second_id] {
        sqlx::query("INSERT INTO app_users (id, username, password_hash) VALUES ($1, $2, 'hash')")
            .bind(id)
            .bind(&username)
            .execute(&pool)
            .await
            .unwrap();
    }

    let error = store
        .find_app_user_by_username(&username)
        .await
        .expect_err("ambiguous login name must not resolve");
    assert!(
        error.to_string().contains("ambiguous login name"),
        "unexpected error: {error}"
    );

    // A missing name still resolves to None (0 rows is valid).
    assert!(store
        .find_app_user_by_username(&format!("absent-{suffix}"))
        .await
        .expect("absent login name is not an error")
        .is_none());

    sqlx::query("DELETE FROM app_users WHERE id IN ($1, $2)")
        .bind(&first_id)
        .bind(&second_id)
        .execute(&pool)
        .await
        .unwrap();
    run_migration_sql(
        &url,
        include_str!("../../../migrations/070_app_users_unique_username.sql"),
    )
    .await
    .expect("restore the unique login-name index");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn migration_070_refuses_ambiguity_and_renames_resolvable_duplicates() {
    let url = database_url();
    let pool = migrated_pool(&url).await;
    let migration_sql = include_str!("../../../migrations/070_app_users_unique_username.sql");
    let suffix = Uuid::new_v4();

    // Stage a credential row plus two non-credential duplicates: the
    // credential row keeps the name, the placeholders are renamed
    // deterministically as `<username>~<id>`.
    sqlx::query("DROP INDEX IF EXISTS uq_app_users_username_ci")
        .execute(&pool)
        .await
        .unwrap();
    let canonical = format!("Ren-{suffix}");
    let credential_id = format!("ren-cred-{suffix}");
    let null_lower_id = format!("ren-null-lower-{suffix}");
    let null_upper_id = format!("ren-null-upper-{suffix}");
    sqlx::query("INSERT INTO app_users (id, username, password_hash) VALUES ($1, $2, 'hash')")
        .bind(&credential_id)
        .bind(&canonical)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_users (id, username) VALUES ($1, $2)")
        .bind(&null_lower_id)
        .bind(canonical.to_lowercase())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_users (id, username) VALUES ($1, $2)")
        .bind(&null_upper_id)
        .bind(canonical.to_uppercase())
        .execute(&pool)
        .await
        .unwrap();

    run_migration_sql(&url, migration_sql)
        .await
        .expect("resolvable duplicates are renamed deterministically");

    let credential_name: String =
        sqlx::query_scalar("SELECT username FROM app_users WHERE id = $1")
            .bind(&credential_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(credential_name, canonical, "the credential row wins");
    let matching: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM app_users WHERE lower(username) = lower($1)")
            .bind(&canonical)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        matching, 1,
        "only the credential row keeps the canonical name"
    );

    let renamed_lower: String = sqlx::query_scalar("SELECT username FROM app_users WHERE id = $1")
        .bind(&null_lower_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        renamed_lower,
        format!("{}~{}", canonical.to_lowercase(), null_lower_id)
    );
    let renamed_upper: String = sqlx::query_scalar("SELECT username FROM app_users WHERE id = $1")
        .bind(&null_upper_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        renamed_upper,
        format!("{}~{}", canonical.to_uppercase(), null_upper_id)
    );

    // Stage two credential rows with the same normalised name: the migration
    // must refuse rather than pick one.
    sqlx::query("DROP INDEX IF EXISTS uq_app_users_username_ci")
        .execute(&pool)
        .await
        .unwrap();
    let ambiguous = format!("Ambig-{suffix}");
    let ambiguous_a = format!("ambig-a-{suffix}");
    let ambiguous_b = format!("ambig-b-{suffix}");
    for id in [&ambiguous_a, &ambiguous_b] {
        sqlx::query("INSERT INTO app_users (id, username, password_hash) VALUES ($1, $2, 'hash')")
            .bind(id)
            .bind(&ambiguous)
            .execute(&pool)
            .await
            .unwrap();
    }
    let error = run_migration_sql(&url, migration_sql)
        .await
        .expect_err("ambiguous credential duplicates must refuse the migration");
    assert!(
        error
            .to_string()
            .contains("ambiguous duplicate login names"),
        "unexpected error: {error}"
    );

    sqlx::query("DELETE FROM app_users WHERE id IN ($1, $2, $3, $4)")
        .bind(&credential_id)
        .bind(&null_lower_id)
        .bind(&null_upper_id)
        .bind(&ambiguous_a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM app_users WHERE id = $1")
        .bind(&ambiguous_b)
        .execute(&pool)
        .await
        .unwrap();
    run_migration_sql(&url, migration_sql)
        .await
        .expect("migration succeeds once the ambiguity is resolved");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn migration_071_downgrades_unknown_roles_and_enforces_check() {
    let url = database_url();
    let pool = migrated_pool(&url).await;
    let store = PgStore::from_pool(pool.clone());
    let suffix = Uuid::new_v4();
    let id = format!("role-it-{suffix}");
    let padded_id = format!("role-it-padded-{suffix}");
    let unknown_before = store
        .count_app_users_with_unknown_roles()
        .await
        .expect("count unknown roles before staging");

    // Simulate a schema migrated out-of-band (no CHECK) so an unknown role can
    // exist at all; the runtime guard and admin warning must still work.
    sqlx::query("ALTER TABLE app_users DROP CONSTRAINT IF EXISTS app_users_role_check")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_users (id, username, role) VALUES ($1, $2, 'superuser')")
        .bind(&id)
        .bind(format!("role-it-name-{suffix}"))
        .execute(&pool)
        .await
        .unwrap();
    // A padded but known role: the runtime trims and would honor it, so the
    // migration must canonicalise it instead of demoting it to viewer.
    sqlx::query("INSERT INTO app_users (id, username, role) VALUES ($1, $2, 'admin ')")
        .bind(&padded_id)
        .bind(format!("role-it-padded-name-{suffix}"))
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        store
            .count_app_users_with_unknown_roles()
            .await
            .expect("count unknown roles"),
        unknown_before + 2,
        "the admin health warning must see both staged roles"
    );

    run_migration_sql(
        &url,
        include_str!("../../../migrations/071_app_users_role_check.sql"),
    )
    .await
    .expect("migration 071 applies");

    let role: String = sqlx::query_scalar("SELECT role FROM app_users WHERE id = $1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(role, "viewer", "unknown roles downgrade to least privilege");
    let padded_role: String = sqlx::query_scalar("SELECT role FROM app_users WHERE id = $1")
        .bind(&padded_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        padded_role, "admin",
        "a padded known role must be canonicalised, not downgraded"
    );
    assert_eq!(
        store
            .count_app_users_with_unknown_roles()
            .await
            .expect("count unknown roles after migration"),
        unknown_before
    );

    let rejected =
        sqlx::query("INSERT INTO app_users (id, username, role) VALUES ($1, $2, 'superuser')")
            .bind(format!("role-it-b-{suffix}"))
            .bind(format!("role-it-name-b-{suffix}"))
            .execute(&pool)
            .await;
    assert!(
        rejected.is_err(),
        "the CHECK constraint must reject unknown roles"
    );

    sqlx::query("DELETE FROM app_users WHERE id IN ($1, $2)")
        .bind(&id)
        .bind(&padded_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
