use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::job_execution::execute_job;
use crate::{format_status, JobKind, JobRun, JobStatus, PgStore, Scheduler, Utc};
use apex_worker::scheduler::JobDef;

fn worker_state_from_job_def(def: &JobDef) -> apex_store::postgres::WorkerJobStateRecord {
    let (last_status, last_error, last_duration_ms) = match def.last_status.as_ref() {
        Some(JobStatus::Succeeded { duration_ms }) => (
            Some("succeeded".to_string()),
            None,
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

fn worker_history_from_run(run: &JobRun) -> apex_store::postgres::WorkerJobHistoryRecord {
    let status = match &run.status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Succeeded { .. } => "succeeded",
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
        created_at: Utc::now(),
    }
}

async fn persist_run_history(store: &Arc<PgStore>, run: &JobRun) {
    if let Err(error) = store
        .insert_worker_job_history(&worker_history_from_run(run))
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
            def.last_status = match state.last_status.as_deref() {
                Some("succeeded") => Some(JobStatus::Succeeded {
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
                Some("pending") => Some(JobStatus::Pending),
                _ => None,
            };
        }
    }
}

#[tracing::instrument(skip(scheduler, store))]
pub(crate) async fn tick_scheduler(scheduler: &mut Scheduler, store: &Arc<PgStore>) {
    tracing::trace!("scheduler_tick_start");
    let now = Utc::now();
    let due = scheduler.due_jobs(now);

    if due.is_empty() {
        tracing::debug!("no jobs due at {}", now);
        return;
    }

    // B320: execute due jobs concurrently under a global semaphore. The
    // previous strictly-sequential loop let one long job (e.g. an unbounded
    // POI LLM pass) starve SLA enforcement, triage processing, and digest
    // delivery for hours.
    let max_concurrent = std::env::var("WORKER_MAX_CONCURRENT_JOBS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4)
        .clamp(1, 16);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    let mut handles = Vec::with_capacity(due.len());
    for kind in due {
        // B321: lease claim — a second worker replica must not double-fire
        // the same schedule, and a crashed run must not block the job
        // forever. Lease covers 2x the declared timeout plus slack.
        let declared_timeout = scheduler
            .jobs
            .get(kind.as_str())
            .and_then(|def| def.timeout_secs)
            .unwrap_or(1800);
        let lease_secs = (declared_timeout as i64 * 2 + 60).max(120);
        match store
            .try_claim_scheduled_job(kind.as_str(), lease_secs)
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

        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => continue,
        };
        let store = Arc::clone(store);
        let timeout = std::time::Duration::from_secs(
            scheduler
                .jobs
                .get(kind.as_str())
                .and_then(|def| def.timeout_secs)
                // B322: hard ceiling so a job with no declared timeout can
                // never wedge the scheduler indefinitely.
                .unwrap_or(7200)
                .clamp(60, 21_600),
        );
        let run_kind = kind.clone();
        let fail_kind = kind.clone();
        handles.push((
            kind,
            tokio::spawn(async move {
                let _permit = permit;
                // B323: timeouts declared in JobDef are now enforced — a
                // hung LLM/HTTP/DB call records a failure instead of
                // blocking forever. The inner task also contains panics:
                // a JoinError becomes a Failed run rather than silently
                // vanishing (which previously left no history record).
                //
                // The JoinHandle is retained and aborted on timeout: dropping
                // a Tokio JoinHandle only *detaches* the task, so a timed-out
                // job previously kept running (and could be started again).
                let mut job_handle =
                    tokio::spawn(async move { execute_job(&run_kind, &store).await });
                match tokio::time::timeout(timeout, &mut job_handle).await {
                    Ok(Ok(run)) => run,
                    Ok(Err(join_error)) => {
                        let mut run = JobRun::new(fail_kind);
                        run.fail(&format!("job panicked: {join_error}"));
                        run
                    }
                    Err(_) => {
                        job_handle.abort();
                        let mut run = JobRun::new(fail_kind);
                        run.fail(&format!(
                            "job timed out after {}s (enforced by scheduler)",
                            timeout.as_secs()
                        ));
                        run
                    }
                }
            }),
        ));
    }

    for (kind, handle) in handles {
        let run = match handle.await {
            Ok(run) => run,
            Err(join_error) => {
                tracing::error!(job = kind.as_str(), %join_error, "scheduler task panicked");
                let mut run = JobRun::new(kind.clone());
                run.fail(&format!("scheduler task panicked: {join_error}"));
                run
            }
        };
        tracing::info!(
            job = kind.as_str(),
            status = format_status(&run),
            duration_ms = run.duration_ms(),
            "job completed"
        );
        persist_run_history(store, &run).await;
        scheduler.record_run(run);
        persist_scheduler_state(store, scheduler, &kind).await;
    }
}

pub(crate) async fn poll_trigger_queue(
    store: &Arc<PgStore>,
    manual_trigger_semaphore: &Arc<Semaphore>,
    max_claims_per_poll: usize,
    manual_trigger_timeout_secs: i64,
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

    let mut claimed_this_poll: usize = 0;
    loop {
        if claimed_this_poll >= max_claims_per_poll {
            break;
        }

        let permit = match Arc::clone(manual_trigger_semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => break,
        };

        match store.pop_job_trigger().await {
            Ok(Some((trigger_id, job_kind_str))) => {
                claimed_this_poll += 1;
                let kind = JobKind::from_str(&job_kind_str);
                tracing::info!(trigger_id = %trigger_id, job = %job_kind_str, "manual trigger: executing job");

                let store = Arc::clone(store);
                tokio::spawn(async move {
                    let _permit = permit;
                    let run = execute_job(&kind, &store).await;
                    persist_run_history(&store, &run).await;
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
