//! `apex-worker healthcheck` — prove the worker is doing work.
//!
//! Container liveness must not be a process-existence check. This probe
//! connects to the database and requires a fresh `service_heartbeats` row for
//! `service = 'worker'`; an unreachable database, a deadlocked scheduler (the
//! heartbeat task stops writing when a scheduler tick exceeds its work
//! budget), or any other stall all surface as an unhealthy container.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};

use apex_store::postgres::{PgStore, ServiceHeartbeatRow};

/// Maximum accepted heartbeat age before the container reports unhealthy.
/// The worker writes a heartbeat every ~30s, so this tolerates three missed
/// writes. Mirrors the API's `WORKER_HEARTBEAT_STALE_AFTER_SECS`.
pub const WORKER_HEARTBEAT_STALE_AFTER_SECS: i64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHeartbeatHealth {
    pub instance_id: String,
    pub version: String,
    pub age_seconds: i64,
}

/// Evaluate the newest worker heartbeat against `max_age_secs`.
///
/// Fails when no heartbeat has ever been recorded or when the newest row is
/// older than the threshold; future timestamps (clock skew) are clamped to
/// zero age.
pub fn evaluate_heartbeat(
    row: Option<ServiceHeartbeatRow>,
    now: DateTime<Utc>,
    max_age_secs: i64,
) -> Result<WorkerHeartbeatHealth> {
    let Some(row) = row else {
        bail!("worker heartbeat missing: no service_heartbeats row for service 'worker'");
    };

    let age_seconds = now
        .signed_duration_since(row.last_seen_at)
        .num_seconds()
        .max(0);

    if age_seconds > max_age_secs {
        bail!(
            "worker heartbeat stale: {} old (threshold {}s), instance {} v{}",
            row.last_seen_at,
            max_age_secs,
            row.instance_id,
            row.version
        );
    }

    Ok(WorkerHeartbeatHealth {
        instance_id: row.instance_id,
        version: row.version,
        age_seconds,
    })
}

/// Connect to the database and evaluate the newest worker heartbeat.
pub async fn check_worker_heartbeat(
    database_url: &str,
    now: DateTime<Utc>,
) -> Result<WorkerHeartbeatHealth> {
    let store = PgStore::connect(database_url).await?;
    let row = store.latest_service_heartbeat("worker").await?;
    evaluate_heartbeat(row, now, WORKER_HEARTBEAT_STALE_AFTER_SECS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn heartbeat_row(last_seen_at: DateTime<Utc>) -> ServiceHeartbeatRow {
        ServiceHeartbeatRow {
            service: "worker".to_string(),
            instance_id: "worker-1".to_string(),
            version: "0.1.0".to_string(),
            last_seen_at,
        }
    }

    #[test]
    fn healthcheck_passes_for_fresh_heartbeat() {
        let now = Utc::now();
        let row = heartbeat_row(now - chrono::Duration::seconds(20));

        let health = evaluate_heartbeat(Some(row), now, WORKER_HEARTBEAT_STALE_AFTER_SECS)
            .expect("fresh heartbeat is healthy");

        assert_eq!(health.age_seconds, 20);
        assert_eq!(health.instance_id, "worker-1");
    }

    #[test]
    fn healthcheck_detects_stale_heartbeat() {
        let now = Utc::now();
        let row = heartbeat_row(now - chrono::Duration::seconds(600));

        let error = evaluate_heartbeat(Some(row), now, WORKER_HEARTBEAT_STALE_AFTER_SECS)
            .expect_err("stale heartbeat must fail");

        assert!(error.to_string().contains("worker heartbeat stale"));
    }

    #[test]
    fn healthcheck_detects_missing_heartbeat() {
        let error = evaluate_heartbeat(None, Utc::now(), WORKER_HEARTBEAT_STALE_AFTER_SECS)
            .expect_err("missing heartbeat must fail");

        assert!(error.to_string().contains("worker heartbeat missing"));
    }

    #[test]
    fn healthcheck_tolerates_clock_skew_at_the_threshold() {
        let now = Utc::now();
        let row = heartbeat_row(now - chrono::Duration::seconds(WORKER_HEARTBEAT_STALE_AFTER_SECS));

        let health = evaluate_heartbeat(Some(row), now, WORKER_HEARTBEAT_STALE_AFTER_SECS)
            .expect("exactly at the threshold is still healthy");

        assert_eq!(health.age_seconds, WORKER_HEARTBEAT_STALE_AFTER_SECS);
    }
}
