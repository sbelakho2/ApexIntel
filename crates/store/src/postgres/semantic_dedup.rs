//! Semantic dedup runtime state (migration 084).
//!
//! The worker constructs the production dedup backend at startup; the API
//! reports what was actually constructed through `/api/features` instead of
//! assuming. `semantic_dedup_state` is a single row (`id = TRUE`).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::postgres::PgStore;

/// Which backend the production worker actually constructed for semantic
/// dedup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDedupBackend {
    /// Test/dev in-memory fallback (no persistence across restarts).
    Memory,
    /// PostgreSQL + pgvector persistent store.
    PgVector,
}

impl SemanticDedupBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::PgVector => "pgvector",
        }
    }

    /// Parse a persisted backend string; unknown values degrade to `Memory`
    /// rather than assuming persistence is active.
    pub fn from_db(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "pgvector" => Self::PgVector,
            _ => Self::Memory,
        }
    }
}

/// Whether semantic lookup is fully available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDedupStatus {
    /// Embedding-based and text-based dedup both available.
    Ok,
    /// Running, but without the embedding path (text similarity only), or the
    /// worker has not recorded its backend yet.
    Degraded,
}

impl SemanticDedupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ok" => Self::Ok,
            _ => Self::Degraded,
        }
    }
}

/// The recorded dedup runtime state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDedupState {
    pub backend: SemanticDedupBackend,
    pub status: SemanticDedupStatus,
    pub detail: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl SemanticDedupState {
    /// State reported before any worker has recorded a backend: explicitly
    /// degraded in-memory fallback, never an assumed-good pgvector state.
    pub fn unrecorded() -> Self {
        Self {
            backend: SemanticDedupBackend::Memory,
            status: SemanticDedupStatus::Degraded,
            detail: Some("worker has not recorded a dedup backend yet".to_string()),
            updated_at: Utc::now(),
        }
    }
}

impl PgStore {
    /// Record which dedup backend the production worker constructed.
    ///
    /// Idempotent single-row upsert; called once at worker startup (and safe
    /// to call again on reconfiguration).
    pub async fn record_semantic_dedup_state(
        &self,
        backend: SemanticDedupBackend,
        status: SemanticDedupStatus,
        detail: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO semantic_dedup_state (id, backend, status, detail, updated_at)
               VALUES (TRUE, $1, $2, $3, NOW())
               ON CONFLICT (id) DO UPDATE SET
                   backend    = EXCLUDED.backend,
                   status     = EXCLUDED.status,
                   detail     = EXCLUDED.detail,
                   updated_at = NOW()"#,
        )
        .bind(backend.as_str())
        .bind(status.as_str())
        .bind(detail)
        .execute(&self.pool)
        .await
        .context("failed to record semantic dedup state")?;
        Ok(())
    }

    /// Read the recorded dedup state. `Ok(None)` means no row exists (the API
    /// falls back to [`SemanticDedupState::unrecorded`]); an `Err` is a real
    /// storage failure and must not be rendered as a normal state.
    pub async fn get_semantic_dedup_state(&self) -> Result<Option<SemanticDedupState>> {
        let row = sqlx::query_as::<_, (String, String, Option<String>, DateTime<Utc>)>(
            "SELECT backend, status, detail, updated_at FROM semantic_dedup_state WHERE id = TRUE",
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to read semantic dedup state")?;

        Ok(
            row.map(|(backend, status, detail, updated_at)| SemanticDedupState {
                backend: SemanticDedupBackend::from_db(&backend),
                status: SemanticDedupStatus::from_db(&status),
                detail,
                updated_at,
            }),
        )
    }
}
