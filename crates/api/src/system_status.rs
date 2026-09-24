//! Process-wide system status snapshot for the UI status strip.
//!
//! `base.html` previously hard-coded "System Online" / "Data Fresh". Those
//! claims are now derived from measured facts: the database probe, the newest
//! `service_heartbeats` row for the worker, and the newest observation
//! timestamp. A background task (started at API boot) refreshes the snapshot
//! every ~30 seconds; templates read the cached value synchronously.

use std::sync::{OnceLock, RwLock};

use chrono::Duration;

/// A worker heartbeat older than this is considered offline (heartbeats are
/// written every ~30s, so this tolerates two missed beats plus jitter).
pub const WORKER_HEARTBEAT_STALE_AFTER_SECS: i64 = 120;

/// Observations older than this are surfaced as stale data.
pub const DATA_FRESH_WITHIN_SECS: i64 = 6 * 60 * 60;

/// Values for the desktop status strip in `base.html`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusStrip {
    pub system_status: String,
    pub system_ok: bool,
    pub data_freshness: String,
    pub data_fresh: bool,
}

impl StatusStrip {
    /// Honest placeholder used before the first probe completes: never claims
    /// the system is healthy.
    pub fn unknown() -> Self {
        Self {
            system_status: "System status unavailable".to_string(),
            system_ok: false,
            data_freshness: "Data freshness unknown".to_string(),
            data_fresh: false,
        }
    }

    /// Derive the strip from measured probe results.
    pub fn from_parts(
        database_ok: bool,
        worker_heartbeat_age: Option<Duration>,
        newest_observation_age: Option<Duration>,
    ) -> Self {
        let (system_status, system_ok) = if !database_ok {
            ("System degraded · database unreachable".to_string(), false)
        } else {
            match worker_heartbeat_age {
                None => (
                    "System degraded · worker heartbeat missing".to_string(),
                    false,
                ),
                Some(age) if age.num_seconds() > WORKER_HEARTBEAT_STALE_AFTER_SECS => (
                    format!("System degraded · worker heartbeat {} ago", format_age(age)),
                    false,
                ),
                Some(_) => ("System Online".to_string(), true),
            }
        };

        let (data_freshness, data_fresh) = match newest_observation_age {
            None => (
                "Data freshness unknown · no observations recorded".to_string(),
                false,
            ),
            Some(age) if age.num_seconds() <= DATA_FRESH_WITHIN_SECS => (
                format!("Data current · newest observation {} ago", format_age(age)),
                true,
            ),
            Some(age) => (
                format!("Data stale · newest observation {} ago", format_age(age)),
                false,
            ),
        };

        Self {
            system_status,
            system_ok,
            data_freshness,
            data_fresh,
        }
    }

    /// Latest published snapshot (falls back to an honest "unknown").
    pub fn current() -> Self {
        match cache().read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Publish a freshly measured snapshot for all subsequent page renders.
    pub fn publish(strip: Self) {
        match cache().write() {
            Ok(mut guard) => *guard = strip,
            Err(poisoned) => *poisoned.into_inner() = strip,
        }
    }
}

/// Compact age rendering: `42s`, `4m`, `9h 14m`, `3d 2h`.
pub fn format_age(age: Duration) -> String {
    let secs = age.num_seconds().max(0);
    let minutes = secs / 60;
    let hours = minutes / 60;
    let days = hours / 24;

    if days > 0 {
        let rem_hours = hours % 24;
        if rem_hours > 0 {
            format!("{days}d {rem_hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        let rem_minutes = minutes % 60;
        if rem_minutes > 0 {
            format!("{hours}h {rem_minutes}m")
        } else {
            format!("{hours}h")
        }
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{secs}s")
    }
}

fn cache() -> &'static RwLock<StatusStrip> {
    static CACHE: OnceLock<RwLock<StatusStrip>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(StatusStrip::unknown()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_age_is_compact_and_human_readable() {
        assert_eq!(format_age(Duration::seconds(42)), "42s");
        assert_eq!(format_age(Duration::minutes(4)), "4m");
        assert_eq!(format_age(Duration::minutes(554)), "9h 14m");
        assert_eq!(format_age(Duration::hours(9)), "9h");
        assert_eq!(format_age(Duration::hours(74)), "3d 2h");
        assert_eq!(format_age(Duration::seconds(-5)), "0s");
    }

    #[test]
    fn healthy_parts_render_online_and_current() {
        let strip = StatusStrip::from_parts(
            true,
            Some(Duration::seconds(12)),
            Some(Duration::minutes(4)),
        );

        assert!(strip.system_ok);
        assert_eq!(strip.system_status, "System Online");
        assert!(strip.data_fresh);
        assert_eq!(
            strip.data_freshness,
            "Data current · newest observation 4m ago"
        );
    }

    #[test]
    fn stale_observation_renders_stale_marker() {
        let strip = StatusStrip::from_parts(
            true,
            Some(Duration::seconds(30)),
            Some(Duration::minutes(554)),
        );

        assert!(!strip.data_fresh);
        assert_eq!(
            strip.data_freshness,
            "Data stale · newest observation 9h 14m ago"
        );
    }

    #[test]
    fn missing_worker_heartbeat_is_degraded_not_online() {
        let strip = StatusStrip::from_parts(true, None, Some(Duration::minutes(1)));

        assert!(!strip.system_ok);
        assert!(strip.system_status.contains("worker heartbeat missing"));
    }

    #[test]
    fn stale_worker_heartbeat_is_degraded() {
        let strip = StatusStrip::from_parts(true, Some(Duration::minutes(5)), None);

        assert!(!strip.system_ok);
        assert!(strip.system_status.contains("worker heartbeat 5m ago"));
        assert!(!strip.data_fresh);
        assert!(strip.data_freshness.contains("no observations recorded"));
    }

    #[test]
    fn database_failure_is_degraded() {
        let strip = StatusStrip::from_parts(false, None, None);

        assert!(!strip.system_ok);
        assert!(strip.system_status.contains("database unreachable"));
    }

    #[test]
    fn unknown_never_claims_health() {
        let strip = StatusStrip::unknown();

        assert!(!strip.system_ok);
        assert!(!strip.data_fresh);
    }
}
