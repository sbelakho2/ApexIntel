//! Store methods for the canonical `app_users` identity table.
//!
//! Backs `app_users` (migrations 059 and 061). Every principal that can
//! authenticate against the product (web session, API key owner, alert
//! addressee) has a row here, so user-owned tables can reference it with a
//! real foreign key instead of repeating unconstrained `TEXT` identity
//! columns.
//!
//! Environment credentials (`APEX_ADMIN_*`, `WEB_USERS_JSON`) are bootstrap
//! only: [`PgStore::bootstrap_app_users`] seeds rows that do not yet carry a
//! password hash. Once a row has credentials the database record wins, and
//! [`PgStore::find_app_user_by_username`] is the login lookup.

use super::*;

/// One row of `app_users`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AppUserRecord {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub enabled: bool,
    pub password_hash: Option<String>,
    pub session_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// Bootstrap credential for one principal, parsed from environment
/// configuration. Only used to seed a row that has no `password_hash` yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppUserSeed {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: String,
}

const APP_USER_COLUMNS: &str = "id, username, display_name, email, role, enabled, password_hash, \
                                 session_version, created_at, updated_at, last_login_at";

impl PgStore {
    /// Ensure the canonical identity row for a principal exists without
    /// credentials, returning the stored row.
    ///
    /// This is the provisioning backstop for principals that do not go through
    /// login (API-key owners at startup, admin-targeted subscription writes),
    /// where only existence is required for the migration-059 foreign keys. It
    /// never overwrites `username`, `role`, `enabled`, `password_hash` or
    /// `last_login_at` of an existing row, so it cannot clobber a verified
    /// principal's identity or write the caller's role onto an override
    /// target.
    pub async fn ensure_app_user_exists(
        &self,
        user_id: &str,
        username: &str,
        role: &str,
    ) -> Result<AppUserRecord> {
        sqlx::query(
            r#"
            INSERT INTO app_users (id, username, display_name, role)
            VALUES ($1, $2, $2, $3)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(username)
        .bind(role)
        .execute(&self.pool)
        .await?;

        self.get_app_user(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("app_users row for '{user_id}' missing after insert"))
    }

    /// Seed environment-configured credentials into `app_users`.
    ///
    /// Rows are inserted when the canonical id is unknown, and an existing row
    /// is updated only while its `password_hash` is still `NULL` (the shape
    /// migration 059 backfill left behind). A row that already carries a
    /// password hash is never modified: after bootstrap the database record is
    /// authoritative and the environment is ignored.
    ///
    /// Returns the number of rows inserted or adopted.
    pub async fn bootstrap_app_users(&self, seeds: &[AppUserSeed]) -> Result<usize> {
        let mut applied = 0usize;
        for seed in seeds {
            let result = sqlx::query(
                r#"
                INSERT INTO app_users (id, username, display_name, role, password_hash)
                VALUES ($1, $2, $2, $3, $4)
                ON CONFLICT (id) DO UPDATE SET
                    username = EXCLUDED.username,
                    role = EXCLUDED.role,
                    password_hash = EXCLUDED.password_hash,
                    updated_at = now()
                WHERE app_users.password_hash IS NULL
                "#,
            )
            .bind(&seed.id)
            .bind(&seed.username)
            .bind(&seed.role)
            .bind(&seed.password_hash)
            .execute(&self.pool)
            .await?;
            applied += result.rows_affected() as usize;
        }
        Ok(applied)
    }

    /// Fetch one canonical identity row by its primary key.
    pub async fn get_app_user(&self, user_id: &str) -> Result<Option<AppUserRecord>> {
        let sql = format!("SELECT {APP_USER_COLUMNS} FROM app_users WHERE id = $1");
        let record: Option<AppUserRecord> = sqlx::query_as(&sql)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(record)
    }

    /// Resolve the login row for a user name.
    ///
    /// Login names are not unique on legacy backfilled data, so a row with a
    /// password hash wins; otherwise the oldest row is returned so the answer
    /// is deterministic.
    pub async fn find_app_user_by_username(&self, username: &str) -> Result<Option<AppUserRecord>> {
        let sql = format!(
            "SELECT {APP_USER_COLUMNS} FROM app_users WHERE username = $1 \
             ORDER BY (password_hash IS NOT NULL) DESC, created_at ASC, id ASC LIMIT 1"
        );
        let record: Option<AppUserRecord> = sqlx::query_as(&sql)
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;

        Ok(record)
    }

    /// List every canonical identity, oldest first.
    pub async fn list_app_users(&self) -> Result<Vec<AppUserRecord>> {
        let sql =
            format!("SELECT {APP_USER_COLUMNS} FROM app_users ORDER BY created_at ASC, id ASC");
        let records: Vec<AppUserRecord> = sqlx::query_as(&sql).fetch_all(&self.pool).await?;
        Ok(records)
    }

    /// Record a successful login and return the authoritative row. `None` when
    /// the row is missing or disabled, so a disabled account can never mint a
    /// session.
    pub async fn record_app_user_login(&self, user_id: &str) -> Result<Option<AppUserRecord>> {
        let sql = format!(
            "UPDATE app_users SET last_login_at = now(), updated_at = now() \
             WHERE id = $1 AND enabled \
             RETURNING {APP_USER_COLUMNS}"
        );
        let record: Option<AppUserRecord> = sqlx::query_as(&sql)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn app_user_seed_keeps_explicit_id_and_username() {
        let seed = AppUserSeed {
            id: "usr-1".to_string(),
            username: "alice".to_string(),
            password_hash: "hash".to_string(),
            role: "admin".to_string(),
        };
        assert_ne!(seed.id, seed.username);
        assert_eq!(seed.role, "admin");
    }
}
