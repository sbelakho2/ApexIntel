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

/// Unique-violation SQLSTATE (`23505`) — used to skip bootstrap seeds whose
/// login name is already taken by another credential-bearing row.
fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}

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
    /// Returns the number of rows inserted or adopted. A seed whose username
    /// already belongs to another credential-bearing row is skipped with a
    /// warning (the unique login-name index, migration 070, refuses it); the
    /// remaining seeds are still applied, because one malformed bootstrap
    /// entry must not block every other administrator from logging in.
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
            .await;

            match result {
                Ok(outcome) => applied += outcome.rows_affected() as usize,
                Err(error) if is_unique_violation(&error) => {
                    tracing::warn!(
                        id = %seed.id,
                        username = %seed.username,
                        "app_users bootstrap skipped: login name already belongs to another credential-bearing row"
                    );
                }
                Err(error) => return Err(error.into()),
            }
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

    /// Canonical principal plus logout-revocation state in one round trip.
    ///
    /// The session authority runs on every page request; composing the
    /// revocation check into the identity lookup keeps it at one query
    /// instead of two.
    pub async fn get_app_user_authority(
        &self,
        user_id: &str,
        session_id: Uuid,
    ) -> Result<Option<(AppUserRecord, bool)>> {
        let sql = format!(
            "SELECT {APP_USER_COLUMNS}, \
                    EXISTS(SELECT 1 FROM revoked_sessions WHERE jti = $2) AS revoked \
             FROM app_users WHERE id = $1"
        );
        let row = sqlx::query(&sql)
            .bind(user_id)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let revoked: bool = sqlx::Row::try_get(&row, "revoked")?;
        let record = <AppUserRecord as sqlx::FromRow<'_, sqlx::postgres::PgRow>>::from_row(&row)?;
        Ok(Some((record, revoked)))
    }

    /// Resolve the canonical login row for a user name.
    ///
    /// Since migration 070 `uq_app_users_username_ci` guarantees at most one
    /// credential-bearing row per `lower(username)`, so this lookup matches 0
    /// or 1 row. More than one match is a data-integrity violation: the answer
    /// is ambiguous and the lookup fails closed with an error instead of
    /// guessing (which would let an attacker or a rename pick the account).
    ///
    /// Rows without a password hash are identity placeholders (API-key owners)
    /// and never resolve a login.
    pub async fn find_app_user_by_username(&self, username: &str) -> Result<Option<AppUserRecord>> {
        let normalized = username.trim().to_lowercase();
        let sql = format!(
            "SELECT {APP_USER_COLUMNS} FROM app_users \
             WHERE lower(username) = $1 AND password_hash IS NOT NULL"
        );
        let records: Vec<AppUserRecord> = sqlx::query_as(&sql)
            .bind(&normalized)
            .fetch_all(&self.pool)
            .await?;

        match records.len() {
            0 => Ok(None),
            1 => Ok(records.into_iter().next()),
            count => anyhow::bail!(
                "ambiguous login name '{normalized}': {count} credential-bearing app_users rows match; refusing to guess"
            ),
        }
    }

    /// Count canonical identities whose role is not one of the four known
    /// roles. Used by the admin health warning; a non-zero count means the
    /// login path will (correctly) refuse those accounts.
    pub async fn count_app_users_with_unknown_roles(&self) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM app_users \
             WHERE role IS NULL OR role NOT IN ('admin', 'analyst', 'viewer', 'service')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
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
