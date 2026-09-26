//! Store methods for the canonical `app_users` identity table.
//!
//! Backs `app_users` (migration 059). Every principal that can authenticate
//! against the product (web session, API key owner, alert addressee) is
//! expected to have a row here, so user-owned tables can reference it with a
//! real foreign key instead of repeating unconstrained `TEXT` identity columns.

use super::*;

/// One row of `app_users`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AppUserRecord {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl PgStore {
    /// Ensure the canonical identity row for a principal exists and refresh its
    /// mutable attributes, returning the stored row.
    ///
    /// Called on successful login so a principal is present in `app_users`
    /// before any user-owned write references it. The `id` comes from the
    /// signed session (or API key owner) and never from request input.
    pub async fn ensure_app_user(
        &self,
        user_id: &str,
        username: &str,
        role: &str,
    ) -> Result<AppUserRecord> {
        let record: AppUserRecord = sqlx::query_as(
            r#"
            INSERT INTO app_users (id, username, display_name, role, last_login_at, updated_at)
            VALUES ($1, $2, $2, $3, now(), now())
            ON CONFLICT (id)
            DO UPDATE SET
                username = EXCLUDED.username,
                role = EXCLUDED.role,
                last_login_at = now(),
                updated_at = now()
            RETURNING id, username, display_name, email, role, is_active,
                      created_at, updated_at, last_login_at
            "#,
        )
        .bind(user_id)
        .bind(username)
        .bind(role)
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    /// Fetch one canonical identity row by its primary key.
    pub async fn get_app_user(&self, user_id: &str) -> Result<Option<AppUserRecord>> {
        let record: Option<AppUserRecord> = sqlx::query_as(
            r#"
            SELECT id, username, display_name, email, role, is_active,
                   created_at, updated_at, last_login_at
            FROM app_users
            WHERE id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record)
    }

    /// Ensure an identity row exists **without modifying an existing one**,
    /// returning the stored row.
    ///
    /// This is the provisioning backstop for principals that do not go through
    /// login (API-key owners at startup, the alert-subscription PUT), where
    /// only existence is required for the migration-059 foreign keys. Unlike
    /// [`Self::ensure_app_user`] it never overwrites `username`, `role` or
    /// `last_login_at`, so it cannot clobber a verified principal's identity or
    /// write the caller's role onto an override target.
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
}
