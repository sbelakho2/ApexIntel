//! Observation replay/backfill API.
//!
//! `/api/admin/replay` endpoint that re-processes observations through
//! the recipe engine for a configurable time range, useful after recipe
//! updates to retroactively generate warnings from historical data.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ─── Request / response types ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRequest {
    /// Start date (inclusive) for replay window
    pub from_date: NaiveDate,
    /// End date (inclusive) for replay window
    pub to_date: NaiveDate,
    /// Optional: only replay specific observation types
    pub observation_types: Option<Vec<String>>,
    /// Optional: only replay for specific entity IDs
    pub entity_ids: Option<Vec<String>>,
    /// Optional: only replay through specific recipe IDs
    pub recipe_ids: Option<Vec<String>>,
    /// Whether to generate warnings (default: true, set false for dry-run)
    pub emit_warnings: Option<bool>,
    /// Max observations to process (safety limit)
    pub limit: Option<usize>,
    /// Whether to run in background (async job)
    pub background: Option<bool>,
}

impl ReplayRequest {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(10_000).min(100_000)
    }

    pub fn emit_warnings(&self) -> bool {
        self.emit_warnings.unwrap_or(true)
    }

    pub fn is_background(&self) -> bool {
        self.background.unwrap_or(false)
    }

    /// Validate the replay request.
    pub fn validate(&self) -> Result<(), String> {
        if self.from_date > self.to_date {
            return Err("from_date must be <= to_date".into());
        }
        let window = (self.to_date - self.from_date).num_days();
        if window > 365 {
            return Err("Replay window cannot exceed 365 days".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ReplayResponse {
    pub job_id: String,
    pub status: ReplayStatus,
    pub observations_queued: usize,
    pub time_range: String,
    pub estimated_duration_secs: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReplayProgress {
    pub job_id: String,
    pub status: ReplayStatus,
    pub total_observations: usize,
    pub processed: usize,
    pub warnings_generated: usize,
    pub errors: usize,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub progress_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReplayStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ReplayStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

// ─── SQL generators ─────────────────────────────────────────────────────

/// SQL to fetch observations for replay within the given window.
/// Returns (sql, param_count) where param_count is the total number
/// of `$N` placeholders used.  The LIMIT is always the last param.
pub fn replay_observations_sql(request: &ReplayRequest) -> (String, usize) {
    let mut sql = String::from(
        "SELECT id, observation_type, entity_id, value, provenance, observed_at \
         FROM observations \
         WHERE observed_at >= $1 AND observed_at <= $2",
    );

    let mut param_idx = 3;

    if let Some(ref types) = request.observation_types {
        if !types.is_empty() {
            sql.push_str(&format!(" AND observation_type = ANY(${})", param_idx));
            param_idx += 1;
        }
    }

    if let Some(ref ids) = request.entity_ids {
        if !ids.is_empty() {
            sql.push_str(&format!(" AND entity_id = ANY(${})", param_idx));
            param_idx += 1;
        }
    }

    sql.push_str(&format!(
        " ORDER BY observed_at ASC LIMIT ${}",
        param_idx
    ));
    let total_params = param_idx;

    (sql, total_params)
}

/// SQL to insert a replay job record.
pub fn insert_replay_job_sql() -> &'static str {
    r#"
    INSERT INTO audit_log (event_type, actor, detail)
    VALUES ('replay_started', 'admin', $1)
    RETURNING id
    "#
}

/// SQL to update replay job progress.
pub fn update_replay_progress_sql() -> &'static str {
    r#"
    INSERT INTO audit_log (event_type, actor, detail)
    VALUES ('replay_progress', 'system', $1)
    "#
}

// ─── Estimation helpers ─────────────────────────────────────────────────

/// Estimate replay duration based on observation count and complexity.
pub fn estimate_duration(observation_count: usize, recipe_count: usize) -> u64 {
    // Rough estimate: 10ms per observation per recipe
    let ms = observation_count as u64 * recipe_count.max(1) as u64 * 10;
    (ms / 1000).max(1)
}

/// Path constant for the replay endpoint.
pub const REPLAY_PATH: &str = "/api/admin/replay";
pub const REPLAY_STATUS_PATH: &str = "/api/admin/replay/:job_id";

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(days_back: i64) -> ReplayRequest {
        let today = Utc::now().date_naive();
        ReplayRequest {
            from_date: today - chrono::Duration::days(days_back),
            to_date: today,
            observation_types: None,
            entity_ids: None,
            recipe_ids: None,
            emit_warnings: None,
            limit: None,
            background: None,
        }
    }

    #[test]
    fn test_valid_request() {
        let req = make_request(7);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_inverted_dates_rejected() {
        let today = Utc::now().date_naive();
        let req = ReplayRequest {
            from_date: today,
            to_date: today - chrono::Duration::days(5),
            observation_types: None,
            entity_ids: None,
            recipe_ids: None,
            emit_warnings: None,
            limit: None,
            background: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_too_long_window_rejected() {
        let req = make_request(400);
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_limit_capped() {
        let mut req = make_request(7);
        req.limit = Some(500_000);
        assert_eq!(req.limit(), 100_000);
    }

    #[test]
    fn test_default_emit_warnings() {
        let req = make_request(7);
        assert!(req.emit_warnings());
    }

    #[test]
    fn test_sql_generation() {
        let req = make_request(7);
        let (sql, param_count) = replay_observations_sql(&req);
        assert!(sql.contains("observed_at >= $1"));
        assert!(sql.contains("LIMIT $3"));
        assert_eq!(param_count, 3);
    }

    #[test]
    fn test_sql_with_filters() {
        let today = Utc::now().date_naive();
        let req = ReplayRequest {
            from_date: today - chrono::Duration::days(7),
            to_date: today,
            observation_types: Some(vec!["capability_change".into()]),
            entity_ids: Some(vec!["comp-001".into()]),
            recipe_ids: None,
            emit_warnings: None,
            limit: Some(500),
            background: None,
        };
        let (sql, param_count) = replay_observations_sql(&req);
        assert!(sql.contains("observation_type = ANY"));
        assert!(sql.contains("entity_id = ANY"));
        assert!(sql.contains("LIMIT $5"));
        assert_eq!(param_count, 5);
    }

    #[test]
    fn test_estimate_duration() {
        assert_eq!(estimate_duration(1000, 10), 100);
        assert_eq!(estimate_duration(100, 5), 5);
        assert_eq!(estimate_duration(1, 1), 1); // minimum 1s
    }

    #[test]
    fn test_replay_status_values() {
        assert_ne!(ReplayStatus::Running, ReplayStatus::Completed);
        assert_eq!(ReplayStatus::Queued, ReplayStatus::Queued);
    }
}
