//! Shared NATS JetStream helpers.
//!
//! The `alerts` stream definition lives here and nowhere else. Both the API
//! SSE bridge (`apex-api` `sse.rs`) and the worker alert publisher
//! (`apex-worker` `nats_stream.rs`) call [`ensure_alerts_stream`]; before this
//! helper each side created the stream with its own config, so whichever
//! process started first won and the loser silently inherited the broker's
//! defaults (notably a 2-minute duplicate window instead of 2 hours, audit
//! #78). The helper both creates a missing stream and reconciles an existing
//! stream whose mutable config drifted.

use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::jetstream::stream::{Config, RetentionPolicy, StorageType};
use tracing::{info, warn};

/// Name of the JetStream stream carrying alert events.
pub const ALERTS_STREAM: &str = "alerts";

/// Subjects captured by [`ALERTS_STREAM`]. `alerts.>` also captures the
/// `alerts.dead_letter.*` subjects used by the SSE bridge, so dead-lettered
/// payloads remain inspectable instead of being discarded.
pub const ALERTS_STREAM_SUBJECT: &str = "alerts.>";

/// Duplicate-tracking window. The outbox republishes unacknowledged rows with
/// the stable outbox row id as `Nats-Msg-Id`; this window must comfortably
/// exceed any realistic retry cadence (the drain polls every 5s with backoff
/// spanning hours).
pub const ALERTS_DUPLICATE_WINDOW: Duration = Duration::from_secs(2 * 60 * 60);

/// Maximum age of a retained alert message.
pub const ALERTS_MAX_AGE: Duration = Duration::from_secs(2 * 60 * 60);

/// The canonical configuration for the `alerts` stream.
fn alerts_stream_config() -> Config {
    Config {
        name: ALERTS_STREAM.to_string(),
        subjects: vec![ALERTS_STREAM_SUBJECT.to_string()],
        max_age: ALERTS_MAX_AGE,
        storage: StorageType::File,
        retention: RetentionPolicy::Limits,
        duplicate_window: ALERTS_DUPLICATE_WINDOW,
        ..Config::default()
    }
}

/// Ensure the `alerts` JetStream stream exists with the canonical config,
/// updating an existing stream when its mutable config differs.
///
/// # Retention and delivery guarantees
///
/// The stream uses [`RetentionPolicy::Limits`] with a two-hour `max_age`, not
/// [`RetentionPolicy::Interest`]. This matters: with `Interest`, a publish
/// that has **no matching consumer at publish time is discarded** — the
/// SSE bridge filters on `alerts.events.>`, so the bridge's
/// `alerts.dead_letter.malformed` publishes were deleted on arrival. `Limits`
/// retains messages up to `max_age` regardless of consumer acknowledgement.
///
/// Realtime delivery is still **best-effort**: the API's `sse_bridge`
/// consumer uses `DeliverPolicy::New` and acks after routing, so an alert
/// published while the bridge is down or redeploying is not replayed to SSE
/// clients. Clients are expected to restore canonical state from the REST API
/// (the database outbox is the durable record); JetStream is the realtime
/// fan-out, not the source of truth.
pub async fn ensure_alerts_stream(jetstream: &async_nats::jetstream::Context) -> Result<()> {
    match jetstream.get_stream(ALERTS_STREAM).await {
        Ok(mut stream) => reconcile_alerts_stream(jetstream, &mut stream).await,
        Err(_) => {
            let desired = alerts_stream_config();
            match jetstream.create_stream(desired).await {
                Ok(_) => {
                    info!(
                        stream = ALERTS_STREAM,
                        "JetStream stream created with the canonical alert-stream config"
                    );
                    Ok(())
                }
                Err(create_error) => {
                    // Lost a create race against the API/worker: re-read and
                    // reconcile instead of failing startup.
                    match jetstream.get_stream(ALERTS_STREAM).await {
                        Ok(mut stream) => reconcile_alerts_stream(jetstream, &mut stream)
                            .await
                            .context(
                            "stream 'alerts' appeared during create but could not be reconciled",
                        ),
                        Err(_) => {
                            Err(create_error).context("failed to create JetStream stream 'alerts'")
                        }
                    }
                }
            }
        }
    }
}

/// Bring an existing `alerts` stream in line with the canonical mutable config.
///
/// Only mutable fields are updated (`subjects`, `retention`, `max_age`,
/// `duplicate_window`); immutable fields such as `storage` are read from the
/// server's own config so an update never attempts to change them. Cloning the
/// server config also preserves every other limit the operator set.
async fn reconcile_alerts_stream(
    jetstream: &async_nats::jetstream::Context,
    stream: &mut async_nats::jetstream::stream::Stream,
) -> Result<()> {
    let desired = alerts_stream_config();
    let current = stream
        .info()
        .await
        .context("failed to read JetStream stream 'alerts' info")?
        .config
        .clone();

    if current.storage != desired.storage {
        // Storage is immutable in JetStream; report instead of failing
        // startup on a stream we cannot fix.
        warn!(
            stream = ALERTS_STREAM,
            current = ?current.storage,
            desired = ?desired.storage,
            "JetStream stream 'alerts' uses a different storage type; leaving it untouched"
        );
    }

    let mut updated = current.clone();
    let mut changed = false;
    if current.duplicate_window != desired.duplicate_window {
        updated.duplicate_window = desired.duplicate_window;
        changed = true;
    }
    if current.retention != desired.retention {
        updated.retention = desired.retention;
        changed = true;
    }
    if current.max_age != desired.max_age {
        updated.max_age = desired.max_age;
        changed = true;
    }
    if current.subjects != desired.subjects {
        updated.subjects = desired.subjects.clone();
        changed = true;
    }

    if !changed {
        info!(
            stream = ALERTS_STREAM,
            "JetStream stream already matches the canonical alert-stream config"
        );
        return Ok(());
    }

    jetstream
        .update_stream(updated)
        .await
        .context("failed to update JetStream stream 'alerts' to the canonical config")?;
    info!(
        stream = ALERTS_STREAM,
        previous_duplicate_window_secs = current.duplicate_window.as_secs(),
        duplicate_window_secs = desired.duplicate_window.as_secs(),
        previous_retention = ?current.retention,
        retention = ?desired.retention,
        max_age_secs = desired.max_age.as_secs(),
        "JetStream stream config reconciled to the canonical alert-stream definition"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit #78: both the worker publisher and the API SSE bridge consume
    /// this ONE definition, so the values below are the contract that keeps
    /// their views of the `alerts` stream identical.
    #[test]
    fn canonical_alerts_stream_config_is_pinned() {
        let config = alerts_stream_config();
        assert_eq!(config.name, ALERTS_STREAM);
        assert_eq!(config.name, "alerts");
        assert_eq!(config.subjects, vec![ALERTS_STREAM_SUBJECT.to_string()]);
        assert_eq!(config.subjects, vec!["alerts.>".to_string()]);
        assert_eq!(config.retention, RetentionPolicy::Limits);
        assert_eq!(config.storage, StorageType::File);
        assert_eq!(config.duplicate_window, ALERTS_DUPLICATE_WINDOW);
        assert_eq!(config.max_age, ALERTS_MAX_AGE);
    }

    /// The outbox republishes unacknowledged rows with a stable `Nats-Msg-Id`;
    /// the duplicate window must comfortably exceed the drain's retry cadence
    /// and never shrink below the retention horizon.
    #[test]
    fn duplicate_window_outlives_the_retry_cadence() {
        assert_eq!(ALERTS_DUPLICATE_WINDOW, Duration::from_secs(2 * 60 * 60));
        assert_eq!(ALERTS_MAX_AGE, Duration::from_secs(2 * 60 * 60));
        assert!(ALERTS_DUPLICATE_WINDOW >= Duration::from_secs(60 * 60));
        assert!(ALERTS_MAX_AGE >= ALERTS_DUPLICATE_WINDOW);
    }
}
