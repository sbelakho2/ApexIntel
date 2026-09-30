//! Revoked browser sessions (`revoked_sessions`, migration 093).
//!
//! Clearing the cookie at logout left a copied cookie valid until its signed
//! expiry (up to 168h). Each session carries a `jti` claim; logout inserts it
//! here and [`crate::postgres::PgStore::is_session_revoked`] is folded into the
//! session authority check, so revocation takes effect on the very next
//! request. Expired rows are purged from the status heartbeat.

use super::*;

impl PgStore {
    /// Record a session id as revoked until its signed expiry.
    pub async fn revoke_session(&self, session_id: Uuid, expires_at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "INSERT INTO revoked_sessions (jti, expires_at) VALUES ($1, $2) \
             ON CONFLICT (jti) DO NOTHING",
        )
        .bind(session_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether the session id was revoked at logout.
    pub async fn is_session_revoked(&self, session_id: Uuid) -> Result<bool> {
        let revoked: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM revoked_sessions WHERE jti = $1)")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(revoked)
    }

    /// Delete revocation rows whose sessions have expired. The set is bounded
    /// by sessions revoked within the maximum session lifetime.
    pub async fn purge_expired_revoked_sessions(&self, now: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM revoked_sessions WHERE expires_at < $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
