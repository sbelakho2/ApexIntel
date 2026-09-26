//! `apex-worker healthcheck` — prove this worker instance is doing work.
//!
//! Container liveness must not be a process-existence check. This probe
//! connects to the database and requires a fresh `service_heartbeats` row for
//! this container's own instance identity; an unreachable database, a
//! deadlocked scheduler (the heartbeat task stops writing when a scheduler
//! tick exceeds its work budget), or another replica masking a stalled
//! instance all surface as an unhealthy container.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use sqlx::Connection;

use apex_store::postgres::{
    latest_service_instance_heartbeat, notification_delivery_backlog_on, outbox_backlog_on,
    NotificationBacklog, OutboxBacklog, PgStore, ServiceHeartbeatRow,
};

pub use apex_store::postgres::WORKER_HEARTBEAT_STALE_AFTER_SECS;

/// Service name used for worker heartbeat rows.
pub const WORKER_SERVICE: &str = "worker";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHeartbeatHealth {
    pub instance_id: String,
    pub version: String,
    pub age_seconds: i64,
}

/// Instance identity for this worker's heartbeat row.
///
/// `APEX_INSTANCE_ID` wins; otherwise the container/host name is used so the
/// healthcheck subprocess derives the exact same identity as the heartbeat
/// writer (both share `HOSTNAME`). Deployments running several workers on one
/// host must set `APEX_INSTANCE_ID` per process.
pub fn resolve_instance_id() -> String {
    instance_id_from_env(
        std::env::var("APEX_INSTANCE_ID").ok().as_deref(),
        std::env::var("HOSTNAME").ok().as_deref(),
    )
}

fn instance_id_from_env(apex_instance_id: Option<&str>, hostname: Option<&str>) -> String {
    apex_instance_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| hostname.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or("worker")
        .to_string()
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
        bail!("worker heartbeat missing: no service_heartbeats row for this instance");
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

/// Readiness thresholds for the notification delivery backlog.
///
/// `NOTIFICATION_DELIVERY_MAX_OVERDUE` (default 250) bounds due-but-unclaimed
/// rows; `NOTIFICATION_DELIVERY_MAX_DEAD_LETTERED` (default 25) bounds terminal
/// failures awaiting operator replay. Exceeding either makes the readiness
/// probe fail so a stuck delivery pipeline is not reported healthy.
pub fn backlog_readiness_thresholds_from_env() -> (i64, i64) {
    fn parse(key: &str, default: i64) -> i64 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|value| *value >= 0)
            .unwrap_or(default)
    }
    (
        parse("NOTIFICATION_DELIVERY_MAX_OVERDUE", 250),
        parse("NOTIFICATION_DELIVERY_MAX_DEAD_LETTERED", 25),
    )
}

/// Evaluate the notification delivery backlog against readiness thresholds.
pub fn evaluate_delivery_backlog(
    backlog: NotificationBacklog,
    max_overdue: i64,
    max_dead_lettered: i64,
) -> Result<()> {
    if backlog.overdue > max_overdue {
        bail!(
            "notification delivery backlog overdue: {} due rows (threshold {}), {} pending total",
            backlog.overdue,
            max_overdue,
            backlog.pending
        );
    }
    if backlog.dead_lettered > max_dead_lettered {
        bail!(
            "notification delivery dead-lettered: {} rows await operator replay (threshold {})",
            backlog.dead_lettered,
            max_dead_lettered
        );
    }
    Ok(())
}

/// Readiness thresholds for the alert outbox backlog.
///
/// A healthy drain keeps `unpublished` near zero; `overdue` counts rows that
/// have been claimable for more than five minutes. Dead-lettered rows await an
/// operator replay (admin UI).
pub fn outbox_readiness_thresholds_from_env() -> (i64, i64) {
    fn parse(key: &str, default: i64) -> i64 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|value| *value >= 0)
            .unwrap_or(default)
    }
    (
        parse("OUTBOX_MAX_OVERDUE", 250),
        parse("OUTBOX_MAX_DEAD_LETTERED", 25),
    )
}

/// Evaluate the alert outbox backlog against readiness thresholds.
pub fn evaluate_outbox_backlog(
    backlog: OutboxBacklog,
    max_overdue: i64,
    max_dead_lettered: i64,
) -> Result<()> {
    if backlog.overdue > max_overdue {
        bail!(
            "alert outbox backlog overdue: {} events unpublished (threshold {}), {} total unpublished",
            backlog.overdue,
            max_overdue,
            backlog.unpublished
        );
    }
    if backlog.dead_lettered > max_dead_lettered {
        bail!(
            "alert outbox dead-lettered: {} events await operator replay (threshold {})",
            backlog.dead_lettered,
            max_dead_lettered
        );
    }
    Ok(())
}

/// Connect to the database with a single short-lived connection and evaluate
/// this instance's newest worker heartbeat plus the notification delivery
/// backlog readiness thresholds.
pub async fn check_worker_heartbeat(
    database_url: &str,
    now: DateTime<Utc>,
) -> Result<WorkerHeartbeatHealth> {
    let instance_id = resolve_instance_id();
    let mut conn = sqlx::postgres::PgConnection::connect(database_url).await?;
    PgStore::assume_service_identity(&mut conn).await?;
    let row = latest_service_instance_heartbeat(&mut conn, WORKER_SERVICE, &instance_id).await?;
    let health = evaluate_heartbeat(row, now, WORKER_HEARTBEAT_STALE_AFTER_SECS)
        .with_context(|| format!("worker heartbeat check failed for instance '{instance_id}'"))?;

    let backlog = notification_delivery_backlog_on(&mut conn, now)
        .await
        .context("failed to read the notification delivery backlog")?;
    let (max_overdue, max_dead_lettered) = backlog_readiness_thresholds_from_env();
    evaluate_delivery_backlog(backlog, max_overdue, max_dead_lettered)
        .context("notification delivery readiness check failed")?;

    // The alert outbox: an unpublished row older than five minutes means the
    // canonical publisher is not keeping up (or is dead).
    let outbox = outbox_backlog_on(&mut conn, now - chrono::Duration::minutes(5))
        .await
        .context("failed to read the alert outbox backlog")?;
    let (max_overdue, max_dead_lettered) = outbox_readiness_thresholds_from_env();
    evaluate_outbox_backlog(outbox, max_overdue, max_dead_lettered)
        .context("alert outbox readiness check failed")?;

    Ok(health)
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

    #[test]
    fn instance_id_prefers_apex_override() {
        assert_eq!(
            instance_id_from_env(Some("  worker-a "), Some("host-1")),
            "worker-a"
        );
    }

    #[test]
    fn instance_id_matches_the_heartbeat_writer_without_a_override() {
        assert_eq!(
            instance_id_from_env(None, Some("host-1")),
            instance_id_from_env(Some("   "), Some("host-1")),
        );
        assert_eq!(instance_id_from_env(None, Some("host-1")), "host-1");
    }

    #[test]
    fn instance_id_defaults_when_nothing_is_set() {
        assert_eq!(instance_id_from_env(None, None), "worker");
    }

    #[test]
    fn delivery_backlog_within_thresholds_is_ready() {
        let backlog = NotificationBacklog {
            pending: 10,
            overdue: 2,
            dead_lettered: 1,
        };
        assert!(evaluate_delivery_backlog(backlog, 250, 25).is_ok());
    }

    #[test]
    fn delivery_backlog_over_threshold_is_not_ready() {
        let overdue = NotificationBacklog {
            pending: 1000,
            overdue: 400,
            dead_lettered: 0,
        };
        let error = evaluate_delivery_backlog(overdue, 250, 25)
            .expect_err("an overdue backlog must fail readiness");
        assert!(error.to_string().contains("backlog overdue"));

        let dead = NotificationBacklog {
            pending: 0,
            overdue: 0,
            dead_lettered: 30,
        };
        let error = evaluate_delivery_backlog(dead, 250, 25)
            .expect_err("dead letters above threshold must fail readiness");
        assert!(error.to_string().contains("dead-lettered"));
    }

    #[test]
    fn outbox_backlog_over_threshold_is_not_ready() {
        let healthy = OutboxBacklog {
            unpublished: 3,
            overdue: 1,
            dead_lettered: 0,
        };
        assert!(evaluate_outbox_backlog(healthy, 250, 25).is_ok());

        let overdue = OutboxBacklog {
            unpublished: 900,
            overdue: 300,
            dead_lettered: 0,
        };
        let error = evaluate_outbox_backlog(overdue, 250, 25)
            .expect_err("an overdue outbox must fail readiness");
        assert!(error.to_string().contains("outbox backlog overdue"));

        let dead = OutboxBacklog {
            unpublished: 0,
            overdue: 0,
            dead_lettered: 26,
        };
        let error = evaluate_outbox_backlog(dead, 250, 25)
            .expect_err("outbox dead letters above threshold must fail readiness");
        assert!(error.to_string().contains("outbox dead-lettered"));
    }
}
