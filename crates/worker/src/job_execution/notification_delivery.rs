//! Notification delivery retry processor job.
//!
//! Runs every minute: claims due `notification_delivery_state` rows with a
//! lease (TX1, committed before any send), attempts each channel delivery
//! outside any transaction, and settles each row as delivered, retried with
//! exponential backoff + jitter, or dead-lettered.
//!
//! The job never publishes alerts directly and never touches NATS: real-time
//! alert publication belongs to the canonical outbox drain
//! (`crate::alert_pipeline`).

use std::sync::Arc;

use apex_store::postgres::PgStore;
use apex_worker::notification_delivery::{
    delivery_claim_owner, log_cycle, process_due_notifications, ConfiguredChannelRouter,
    DEFAULT_DELIVERY_BATCH,
};

use crate::observability::WORKER_METRICS;
use crate::{JobKind, JobRun};

#[tracing::instrument(skip(store))]
pub(super) async fn run_notification_delivery(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let owner = delivery_claim_owner();
    let router = ConfiguredChannelRouter::from_env();

    let outcome = match process_due_notifications(
        store.as_ref(),
        &router,
        &owner,
        DEFAULT_DELIVERY_BATCH,
        chrono::Utc::now(),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            run.fail(&format!("notification_delivery: cycle failed: {error}"));
            return run;
        }
    };

    WORKER_METRICS.record_notification_delivery_cycle(&outcome);
    log_cycle(&outcome);

    let notes = format!(
        "claimed {} deliveries: {} delivered, {} retried with backoff, {} dead-lettered",
        outcome.claimed, outcome.delivered, outcome.retried, outcome.dead_lettered
    );
    if outcome.dead_lettered > 0 {
        // Dead-lettering needs operator attention (admin replay) but the cycle
        // itself worked: report degraded rather than failed.
        run.degrade(outcome.delivered as u64, &notes);
    } else {
        run.succeed(outcome.delivered as u64, &notes);
    }
    run
}
