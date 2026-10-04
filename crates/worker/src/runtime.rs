use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

use crate::job_execution::execute_job;
use crate::job_execution::execute_job_with_payload;
use crate::job_execution::JobExecutionContext;
use crate::{
    format_status, JobKind, JobRun, JobStatus, PgStore, Scheduler, SchedulerProgressClock, Utc,
};
use apex_worker::scheduler::JobDef;

/// Effective per-job wall-clock timeout enforced by the scheduler: the
/// declared timeout, otherwise 7200s, clamped to the runtime's 60..=21_600s
/// hard bounds. Shared with startup so liveness budgets cannot drift from the
/// timeouts the runtime actually enforces.
pub(crate) fn effective_job_timeout_secs(def: &JobDef) -> u64 {
    def.timeout_secs.unwrap_or(7200).clamp(60, 21_600)
}

fn worker_state_from_job_def(def: &JobDef) -> apex_store::postgres::WorkerJobStateRecord {
    let (last_status, last_error, last_duration_ms) = match def.last_status.as_ref() {
        Some(JobStatus::Succeeded { duration_ms }) => (
            Some("succeeded".to_string()),
            None,
            Some(*duration_ms as i64),
        ),
        Some(JobStatus::Degraded {
            reason,
            duration_ms,
        }) => (
            Some("degraded".to_string()),
            Some(reason.clone()),
            Some(*duration_ms as i64),
        ),
        Some(JobStatus::Failed { error, duration_ms }) => (
            Some("failed".to_string()),
            Some(error.clone()),
            Some(*duration_ms as i64),
        ),
        Some(JobStatus::Skipped { reason }) => {
            (Some("skipped".to_string()), Some(reason.clone()), None)
        }
        Some(JobStatus::Running) => (
            Some("running".to_string()),
            Some("worker restarted while run was in progress".to_string()),
            None,
        ),
        Some(JobStatus::Pending) => (Some("pending".to_string()), None, None),
        None => (None, None, None),
    };

    apex_store::postgres::WorkerJobStateRecord {
        job_kind: def.kind.as_str().to_string(),
        last_run: def.last_run,
        last_status,
        last_error,
        last_duration_ms,
        consecutive_failures: def.consecutive_failures as i32,
        max_consecutive_failures: def.max_consecutive_failures as i32,
        circuit_open: def.is_circuit_broken(),
        updated_at: Utc::now(),
    }
}

fn worker_history_from_run(
    run: &JobRun,
    instance_id: &str,
) -> apex_store::postgres::WorkerJobHistoryRecord {
    let status = match &run.status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Succeeded { .. } => "succeeded",
        JobStatus::Degraded { .. } => "degraded",
        JobStatus::Failed { .. } => "failed",
        JobStatus::Skipped { .. } => "skipped",
    }
    .to_string();

    apex_store::postgres::WorkerJobHistoryRecord {
        run_id: run.run_id.clone(),
        job_kind: run.kind.as_str().to_string(),
        status,
        started_at: run.started_at,
        finished_at: run.finished_at,
        duration_ms: Some(run.duration_ms() as i64),
        items_processed: run.items_processed as i64,
        notes: run.notes.clone(),
        instance_id: instance_id.to_string(),
        created_at: Utc::now(),
    }
}

async fn persist_run_history(store: &Arc<PgStore>, run: &JobRun, instance_id: &str) {
    if let Err(error) = store
        .insert_worker_job_history(&worker_history_from_run(run, instance_id))
        .await
    {
        tracing::warn!(job = run.kind.as_str(), run_id = %run.run_id, error = %error, "failed to persist worker job history");
    }
}

async fn persist_scheduler_state(store: &Arc<PgStore>, scheduler: &Scheduler, kind: &JobKind) {
    if let Some(def) = scheduler.jobs.get(kind.as_str()) {
        if let Err(error) = store
            .upsert_worker_job_state(&worker_state_from_job_def(def))
            .await
        {
            tracing::warn!(job = kind.as_str(), error = %error, "failed to persist worker job state");
        }
    }
}

pub(crate) fn restore_scheduler_state(
    scheduler: &mut Scheduler,
    states: &[apex_store::postgres::WorkerJobStateRecord],
) {
    for state in states {
        if let Some(def) = scheduler.jobs.get_mut(&state.job_kind) {
            def.last_run = state.last_run;
            def.consecutive_failures = state.consecutive_failures.max(0) as u32;
            def.max_consecutive_failures = state.max_consecutive_failures.max(1) as u32;
            def.enabled = !state.circuit_open;
            // #106: an open circuit restored from the database must still be
            // able to probe. The state's `updated_at` is when the tripping run
            // was persisted, so the probe cooldown is preserved across restarts.
            def.circuit_opened_at = if state.circuit_open {
                Some(state.updated_at)
            } else {
                None
            };
            def.last_status = match state.last_status.as_deref() {
                Some("succeeded") => Some(JobStatus::Succeeded {
                    duration_ms: state.last_duration_ms.unwrap_or_default().max(0) as u64,
                }),
                Some("degraded") => Some(JobStatus::Degraded {
                    reason: state
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "previous run was degraded".to_string()),
                    duration_ms: state.last_duration_ms.unwrap_or_default().max(0) as u64,
                }),
                Some("failed") => Some(JobStatus::Failed {
                    error: state
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "previous run failed".to_string()),
                    duration_ms: state.last_duration_ms.unwrap_or_default().max(0) as u64,
                }),
                Some("skipped") => Some(JobStatus::Skipped {
                    reason: state
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "previous run skipped".to_string()),
                }),
                Some("running") => Some(JobStatus::Skipped {
                    reason: "previous run recovered after worker restart".to_string(),
                }),
                Some("interrupted") => Some(JobStatus::Skipped {
                    reason: "previous run was interrupted by worker shutdown".to_string(),
                }),
                Some("pending") => Some(JobStatus::Pending),
                _ => None,
            };
        }
    }
}

/// Tracks detached job tasks so shutdown can wait for (or abort) them (#64).
#[derive(Clone, Default)]
pub(crate) struct RunTracker {
    in_flight: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl RunTracker {
    fn track(&self, handle: JoinHandle<()>) {
        let mut in_flight = self.in_flight.lock().unwrap();
        // Drop finished handles so the vector cannot grow without bound.
        in_flight.retain(|handle| !handle.is_finished());
        in_flight.push(handle);
    }

    /// Number of detached job tasks still running.
    pub(crate) fn in_flight(&self) -> usize {
        let mut in_flight = self.in_flight.lock().unwrap();
        in_flight.retain(|handle| !handle.is_finished());
        in_flight.len()
    }

    /// Abort every in-flight job task (shutdown past the drain deadline).
    pub(crate) fn abort_all(&self) {
        let in_flight = self.in_flight.lock().unwrap();
        for handle in in_flight.iter() {
            handle.abort();
        }
    }

    /// Wait until every tracked task finished, or `timeout` elapses.
    ///
    /// Returns `true` when all tasks are done.
    pub(crate) async fn wait_for_all(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.in_flight() == 0 {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// A completed run handed back from a detached job task.
struct CompletedRun {
    kind: JobKind,
    run: JobRun,
}

/// Long-lived scheduler dispatch state (#104).
///
/// * one process-wide job semaphore, acquired with `try_acquire_owned` so a
///   saturated pool leaves jobs due for the next tick instead of blocking the
///   dispatcher;
/// * dedicated lanes for the control-plane jobs that must never wait behind a
///   long crawl/LLM job;
/// * an unbounded channel of completed runs that each tick drains before
///   dispatching new work.
pub(crate) struct Dispatcher {
    completed_tx: mpsc::UnboundedSender<CompletedRun>,
    completed_rx: mpsc::UnboundedReceiver<CompletedRun>,
    global: Arc<Semaphore>,
    lanes: Vec<(&'static str, Arc<Semaphore>)>,
    tracker: RunTracker,
}

impl Dispatcher {
    pub(crate) fn from_env(tracker: RunTracker) -> Self {
        let max_concurrent = std::env::var("WORKER_MAX_CONCURRENT_JOBS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 16);
        let lane_concurrency = std::env::var("WORKER_LANE_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 8);
        // Dedicated lanes: these jobs are user-visible safety nets and must
        // never queue behind a long crawl / LLM pass (#104).
        let lanes = [
            "notification_delivery",
            "sla_enforcement",
            "triage_processing",
            "observation_index",
        ]
        .into_iter()
        .map(|kind| (kind, Arc::new(Semaphore::new(lane_concurrency))))
        .collect();
        let (completed_tx, completed_rx) = mpsc::unbounded_channel();
        tracing::info!(
            max_concurrent,
            lane_concurrency,
            "scheduler dispatcher initialized (long-lived pool + dedicated control-plane lanes)"
        );
        Self {
            completed_tx,
            completed_rx,
            global: Arc::new(Semaphore::new(max_concurrent)),
            lanes,
            tracker,
        }
    }

    /// A permit for `kind`: its dedicated lane, or the global pool.
    fn try_permit(&self, kind: &JobKind) -> Option<OwnedSemaphorePermit> {
        if let Some((_, lane)) = self.lanes.iter().find(|(name, _)| *name == kind.as_str()) {
            return Arc::clone(lane).try_acquire_owned().ok();
        }
        Arc::clone(&self.global).try_acquire_owned().ok()
    }

    /// Drain every completed run: persist history, record it in the scheduler
    /// and persist the scheduler state. Runs before any new dispatch so a
    /// finished job frees its slot and updates its state in the same tick.
    async fn drain_completed(
        &mut self,
        scheduler: &mut Scheduler,
        store: &Arc<PgStore>,
        progress: &SchedulerProgressClock,
        instance_id: &str,
    ) {
        while let Ok(completed) = self.completed_rx.try_recv() {
            progress.record_progress();
            tracing::info!(
                job = completed.kind.as_str(),
                status = format_status(&completed.run),
                duration_ms = completed.run.duration_ms(),
                "job completed"
            );
            persist_run_history(store, &completed.run, instance_id).await;
            scheduler.record_run(completed.run);
            persist_scheduler_state(store, scheduler, &completed.kind).await;
        }
    }

    /// Final drain at shutdown: persist every run that completed after the
    /// last tick so a green run is never reported as interrupted (#64/#104).
    pub(crate) async fn drain_pending(
        &mut self,
        scheduler: &mut Scheduler,
        store: &Arc<PgStore>,
        progress: &SchedulerProgressClock,
        instance_id: &str,
    ) {
        self.drain_completed(scheduler, store, progress, instance_id)
            .await;
    }
}

/// One scheduler tick: drain completed runs first, then dispatch due jobs
/// without awaiting them (#104).
#[tracing::instrument(skip(scheduler, store, ctx, progress, dispatcher))]
pub(crate) async fn tick_scheduler(
    scheduler: &mut Scheduler,
    store: &Arc<PgStore>,
    ctx: &JobExecutionContext,
    progress: &SchedulerProgressClock,
    instance_id: &str,
    dispatcher: &mut Dispatcher,
) {
    tracing::trace!("scheduler_tick_start");
    let now = Utc::now();

    // 1. Persist and record everything that finished since the last tick.
    dispatcher
        .drain_completed(scheduler, store, progress, instance_id)
        .await;

    // 2. Dispatch due jobs. The scheduler lock is held for this pass only:
    // each job runs in its own detached task and reports back over the
    // completed channel.
    for kind in scheduler.due_jobs(now) {
        let Some(permit) = dispatcher.try_permit(&kind) else {
            tracing::debug!(
                job = kind.as_str(),
                "concurrency pool saturated; leaving job due for the next tick"
            );
            continue;
        };

        // #92: only claim an LLM-heavy run while a model slot is free, so a
        // saturated gate leaves the run queued instead of starting blocked
        // work. The permit itself is released immediately: the run's model
        // calls each acquire the gate (holding a permit for a whole run while
        // its own calls acquire more would deadlock the gate).
        if apex_worker::scheduler::job_may_use_llm(&kind) {
            match apex_worker::llm_concurrency::try_acquire_llm_slot() {
                Some(slot) => drop(slot),
                None => {
                    tracing::info!(
                        job = kind.as_str(),
                        capacity = apex_worker::llm_concurrency::llm_gate_capacity(),
                        "LLM concurrency gate saturated; leaving job queued for the next tick"
                    );
                    continue;
                }
            }
        }

        // B321: lease claim — a second worker replica (or a manual trigger
        // racing the schedule) must not double-fire the same run.
        let declared_timeout = scheduler
            .jobs
            .get(kind.as_str())
            .and_then(|def| def.timeout_secs)
            .unwrap_or(1800);
        let lease_secs = (declared_timeout as i64 * 2 + 60).max(120);
        match store
            .try_claim_scheduled_job(kind.as_str(), instance_id, lease_secs)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(job = kind.as_str(), "job leased elsewhere; skipping");
                scheduler.record_run(JobRun {
                    status: JobStatus::Skipped {
                        reason: "leased by another worker".to_string(),
                    },
                    ..JobRun::new(kind)
                });
                continue;
            }
            Err(error) => {
                tracing::warn!(job = kind.as_str(), %error, "lease claim failed; executing anyway");
            }
        }

        // Mark the run Running before it starts: `due_jobs` must not dispatch
        // it again while the detached task is in flight, and the persisted
        // state lets startup recovery distinguish a crash from a schedule.
        scheduler.mark_dispatched(&kind);
        persist_scheduler_state(store, scheduler, &kind).await;

        let timeout = Duration::from_secs(
            scheduler
                .jobs
                .get(kind.as_str())
                .map(effective_job_timeout_secs)
                // B322: hard ceiling so a job with no declared timeout can
                // never wedge the dispatcher indefinitely.
                .unwrap_or(7200),
        );
        let store = Arc::clone(store);
        let job_context = ctx.clone();
        let run_kind = kind.clone();
        let fail_kind = kind.clone();
        let completed_tx = dispatcher.completed_tx.clone();
        let handle = tokio::spawn(async move {
            let _permit = permit;
            // B323: per-job timeouts abort the inner task; a JoinError becomes
            // a Failed run instead of silently vanishing.
            let mut job_handle =
                tokio::spawn(async move { execute_job(&run_kind, &store, &job_context).await });
            let run = match tokio::time::timeout(timeout, &mut job_handle).await {
                Ok(Ok(run)) => run,
                Ok(Err(join_error)) => {
                    let mut run = JobRun::new(fail_kind.clone());
                    run.fail(&format!("job panicked: {join_error}"));
                    run
                }
                Err(_) => {
                    job_handle.abort();
                    let mut run = JobRun::new(fail_kind.clone());
                    run.fail(&format!(
                        "job timed out after {}s (enforced by scheduler)",
                        timeout.as_secs()
                    ));
                    run
                }
            };
            // The receiver may have been dropped during shutdown; the run was
            // already persisted by its own handler path where applicable and
            // shutdown reconciliation marks interrupted rows.
            let _ = completed_tx.send(CompletedRun {
                kind: fail_kind,
                run,
            });
        });
        dispatcher.tracker.track(handle);
    }
}

/// Claim the oldest queued manual trigger and execute it under the same lease,
/// timeout and scheduler-state bookkeeping as a scheduled run (#105).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn poll_trigger_queue(
    store: &Arc<PgStore>,
    scheduler: &Arc<tokio::sync::Mutex<Scheduler>>,
    manual_trigger_semaphore: &Arc<Semaphore>,
    max_claims_per_poll: usize,
    manual_trigger_timeout_secs: i64,
    ctx: &JobExecutionContext,
    instance_id: &str,
    tracker: &RunTracker,
) {
    match store
        .timeout_stale_job_triggers(manual_trigger_timeout_secs)
        .await
    {
        Ok(timed_out) if timed_out > 0 => tracing::warn!(
            timed_out,
            manual_trigger_timeout_secs,
            "poll_trigger_queue: marked stale trigger(s) as timed out"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(
            error = %e,
            "poll_trigger_queue: failed to mark stale trigger(s) as timed out"
        ),
    }

    // #169: a crash mid-analysis leaves its run `running`; the in-flight
    // partial unique index would then block every new run for that insight.
    // Reconcile abandoned runs on the same cadence as abandoned triggers.
    match store
        .expire_stale_insight_analysis_runs(manual_trigger_timeout_secs)
        .await
    {
        Ok(expired) if expired > 0 => tracing::warn!(
            expired,
            manual_trigger_timeout_secs,
            "poll_trigger_queue: failed stale insight analysis run(s)"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(
            error = %e,
            "poll_trigger_queue: failed to expire stale insight analysis run(s)"
        ),
    }

    let mut claimed_this_poll: usize = 0;
    loop {
        if claimed_this_poll >= max_claims_per_poll {
            break;
        }

        let permit = match Arc::clone(manual_trigger_semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => break,
        };

        match store.pop_job_trigger_with_payload().await {
            Ok(Some((trigger_id, job_kind_str, payload))) => {
                claimed_this_poll += 1;
                let kind = JobKind::from_str(&job_kind_str);
                tracing::info!(
                    trigger_id = %trigger_id,
                    job = %job_kind_str,
                    has_payload = payload.is_some(),
                    "manual trigger: executing job"
                );

                // Resolve the same timeout the scheduler would enforce and a
                // lease that covers 2x it plus slack (mirrors tick_scheduler).
                let (timeout_secs, lease_secs, registered) = {
                    let scheduler = scheduler.lock().await;
                    let def = scheduler.jobs.get(kind.as_str());
                    let declared = def.and_then(|def| def.timeout_secs).unwrap_or(1800);
                    (
                        def.map(effective_job_timeout_secs).unwrap_or(7200),
                        (declared as i64 * 2 + 60).max(120),
                        def.is_some(),
                    )
                };

                // #105: a manual trigger of a SCHEDULED job claims the SAME
                // lease as the scheduled run, so a trigger racing the schedule
                // (or another replica's trigger) is answered "already running"
                // instead of executing the job twice. Custom/unregistered kinds
                // have no scheduled run to race and no scheduler state to
                // settle, so they skip the lease (the trigger queue itself
                // already serializes same-kind claims).
                if registered {
                    match store
                        .try_claim_scheduled_job(kind.as_str(), instance_id, lease_secs)
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::info!(
                                trigger_id = %trigger_id,
                                job = %job_kind_str,
                                "manual trigger: job already running; refusing duplicate execution"
                            );
                            if let Err(error) = store
                                .complete_job_trigger(&trigger_id, Some("already running"))
                                .await
                            {
                                tracing::warn!(
                                    trigger_id = %trigger_id,
                                    %error,
                                    "failed to mark duplicate trigger complete"
                                );
                            }
                            drop(permit);
                            continue;
                        }
                        Err(error) => {
                            tracing::warn!(
                                trigger_id = %trigger_id,
                                job = %job_kind_str,
                                %error,
                                "manual trigger: lease claim failed; executing anyway"
                            );
                        }
                    }
                } else {
                    tracing::debug!(
                        trigger_id = %trigger_id,
                        job = %job_kind_str,
                        "manual trigger: kind is not scheduled; skipping scheduled-run lease"
                    );
                }

                // Mark the run in the scheduler before dispatch so a scheduled
                // tick cannot double-dispatch the same kind while it runs.
                {
                    let mut scheduler = scheduler.lock().await;
                    scheduler.mark_dispatched(&kind);
                    persist_scheduler_state(store, &scheduler, &kind).await;
                }

                let store = Arc::clone(store);
                let job_context = ctx.clone();
                let instance_id = instance_id.to_string();
                let scheduler = Arc::clone(scheduler);
                let handle = tokio::spawn(async move {
                    let _permit = permit;
                    // #105: the manual run is bounded by the same effective
                    // timeout as the scheduled run; on expiry the inner task is
                    // ABORTED rather than detached.
                    let job_store = Arc::clone(&store);
                    let mut job_handle = tokio::spawn(async move {
                        execute_job_with_payload(&kind, &job_store, &job_context, payload.as_ref())
                            .await
                    });
                    let run = match tokio::time::timeout(
                        Duration::from_secs(timeout_secs),
                        &mut job_handle,
                    )
                    .await
                    {
                        Ok(Ok(run)) => run,
                        Ok(Err(join_error)) => {
                            let mut run = JobRun::new(JobKind::from_str(&job_kind_str));
                            run.fail(&format!("job panicked: {join_error}"));
                            run
                        }
                        Err(_) => {
                            job_handle.abort();
                            let mut run = JobRun::new(JobKind::from_str(&job_kind_str));
                            run.fail(&format!(
                                "job timed out after {timeout_secs}s (enforced by scheduler)"
                            ));
                            run
                        }
                    };

                    persist_run_history(&store, &run, &instance_id).await;

                    // #105: persist scheduler state after a manual run, so the
                    // trigger counts as the job's last run and the circuit
                    // breaker / next-due computation see it.
                    {
                        let mut scheduler = scheduler.lock().await;
                        scheduler.record_run(run.clone());
                        persist_scheduler_state(&store, &scheduler, &run.kind).await;
                    }

                    let error = if matches!(run.status, JobStatus::Failed { .. }) {
                        Some(run.notes.as_str())
                    } else {
                        None
                    };
                    if let Err(e) = store.complete_job_trigger(&trigger_id, error).await {
                        tracing::warn!(trigger_id = %trigger_id, "failed to mark trigger complete: {e}");
                    }
                    tracing::info!(
                        trigger_id = %trigger_id,
                        job = job_kind_str,
                        status = format_status(&run),
                        "manual trigger: job completed"
                    );
                });
                tracker.track(handle);
            }
            Ok(None) => {
                drop(permit);
                break;
            }
            Err(e) => {
                drop(permit);
                tracing::warn!("poll_trigger_queue: DB error: {e}");
                break;
            }
        }
    }

    if claimed_this_poll > 0 {
        tracing::info!(
            claimed = claimed_this_poll,
            max_claims_per_poll,
            "poll_trigger_queue: claimed trigger(s)"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[tokio::test]
    async fn run_tracker_waits_for_and_aborts_detached_tasks() {
        let tracker = RunTracker::default();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        tracker.track(tokio::spawn(async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
        }));

        started_rx.await.unwrap();
        assert_eq!(tracker.in_flight(), 1);
        assert!(
            !tracker.wait_for_all(Duration::from_millis(50)).await,
            "a still-running task must not report drained"
        );

        release_tx.send(()).unwrap();
        assert!(
            tracker.wait_for_all(Duration::from_secs(2)).await,
            "task must drain once released"
        );
        assert_eq!(tracker.in_flight(), 0);
    }

    #[tokio::test]
    async fn control_plane_kinds_get_dedicated_lanes() {
        let dispatcher = Dispatcher::from_env(RunTracker::default());

        // A lane permit is independent of the global pool.
        let lane = dispatcher
            .try_permit(&JobKind::NotificationDelivery)
            .expect("dedicated lane must hand out a permit");
        // The global pool is still fully available (4 by default); taking a
        // lane never consumes it.
        let global = dispatcher
            .try_permit(&JobKind::CrawlCycle)
            .expect("global pool permit expected");
        drop(lane);
        drop(global);

        // The lane itself is bounded.
        let first = dispatcher
            .try_permit(&JobKind::SlaEnforcement)
            .expect("first lane permit");
        let second = dispatcher.try_permit(&JobKind::SlaEnforcement);
        if std::env::var("WORKER_LANE_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            == 1
        {
            assert!(
                second.is_none(),
                "a lane at capacity must refuse additional permits"
            );
        }
        drop(first);
        drop(second);
    }

    fn worker_state(
        circuit_open: bool,
        updated_at: chrono::DateTime<Utc>,
    ) -> apex_store::postgres::WorkerJobStateRecord {
        apex_store::postgres::WorkerJobStateRecord {
            job_kind: "crawl_cycle".to_string(),
            last_run: Some(updated_at),
            last_status: Some(if circuit_open {
                "failed".to_string()
            } else {
                "succeeded".to_string()
            }),
            last_error: if circuit_open {
                Some("boom".to_string())
            } else {
                None
            },
            last_duration_ms: Some(1),
            consecutive_failures: if circuit_open { 5 } else { 0 },
            max_consecutive_failures: 5,
            circuit_open,
            updated_at,
        }
    }

    #[test]
    fn restored_open_circuit_keeps_its_probe_cooldown() {
        let mut scheduler = Scheduler::new();
        scheduler.register(JobDef::new(
            JobKind::CrawlCycle,
            apex_worker::scheduler::Schedule::IntervalSecs(60),
        ));
        let updated_at = Utc::now();
        restore_scheduler_state(&mut scheduler, &[worker_state(true, updated_at)]);

        let def = &scheduler.jobs["crawl_cycle"];
        assert!(!def.enabled);
        assert!(def.is_circuit_broken());
        assert_eq!(def.circuit_opened_at, Some(updated_at));
        assert!(!def.probe_allowed(updated_at + chrono::Duration::seconds(10)));
        assert!(def.probe_allowed(
            updated_at
                + chrono::Duration::seconds(
                    apex_worker::scheduler::CIRCUIT_PROBE_COOLDOWN_SECS + 1
                )
        ));
    }

    #[test]
    fn restored_closed_circuit_clears_the_probe_timestamp() {
        let mut scheduler = Scheduler::new();
        scheduler.register(JobDef::new(
            JobKind::CrawlCycle,
            apex_worker::scheduler::Schedule::IntervalSecs(60),
        ));
        restore_scheduler_state(&mut scheduler, &[worker_state(false, Utc::now())]);

        let def = &scheduler.jobs["crawl_cycle"];
        assert!(def.enabled);
        assert!(def.circuit_opened_at.is_none());
    }
}
