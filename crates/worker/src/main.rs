use anyhow::Result;
use apex_core::config::AppConfig;
use apex_worker::nightly::{
    process_crawl_stage, process_mining_stage, process_poi_stage, process_drift_stage,
    CrawlStageResult, DriftCheckStageResult, MiningStageResult,
    PoiRefreshStageResult,
};
use apex_worker::scheduler::{default_scheduler, JobKind, JobRun, Scheduler};
use apex_worker::weekly::{
    run_weekly_pipeline, DeprecationPolicy, MemoInputs, PromotionPolicy, ProductionRecipe,
    StagedRecipe,
};
use chrono::Utc;
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

// ────────────────────────────────────────────────────────────────────────────
// File size guard (B293)
// ────────────────────────────────────────────────────────────────────────────

/// Maximum file size for input JSON files (B293).
///
/// If `load_nightly_inputs()` or `load_weekly_inputs()` encounters a file
/// larger than this limit, it aborts immediately **before** allocating any
/// heap memory to read the file.  This prevents a misconfigured or maliciously
/// crafted input file from causing the worker to exhaust available RAM.
///
/// Rationale for 16 MiB:
/// - Production nightly input JSON files are typically 10–50 KiB (compressed stage results).
/// - Weekly input JSON files are typically 100–500 KiB (recipe + memo metadata).
/// - 16 MiB is a 50× safety margin; exceeding it is almost certainly a bug (e.g.,
///   accidentally pointed at a database dump, a raw model checkpoint, a binary blob).
///
/// If legitimate production workloads require larger inputs, increase this value
/// after verifying the memory impact on the worker container.
pub const MAX_INPUT_FILE_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

#[derive(Debug, Deserialize)]
struct NightlyInputs {
    crawl: CrawlStageResult,
    mining: MiningStageResult,
    poi: PoiRefreshStageResult,
    drift: DriftCheckStageResult,
}

#[derive(Debug, Deserialize)]
struct WeeklyInputs {
    staged_recipes: Vec<StagedRecipe>,
    production_recipes: Vec<ProductionRecipe>,
    memo_inputs: MemoInputs,
    promotion_policy: Option<PromotionPolicy>,
    deprecation_policy: Option<DeprecationPolicy>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let log_level = std::env::var("WORKER_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| format!("{},apex_worker={}", log_level, log_level));
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::new(log_filter))
        .init();

    // B294: Validate config at startup — fail fast if env vars are misconfigured.
    // AppConfig::from_env() already checks for required fields (DATABASE_URL),
    // and validate() checks numeric ranges, URL formats, etc.
    let config = AppConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        tracing::error!(
            "Configuration validation failed ({} errors):",
            validation_errors.len()
        );
        for err in &validation_errors {
            tracing::error!("  - {}", err);
        }
        anyhow::bail!(
            "Refusing to start worker with invalid config. Fix the errors above and restart."
        );
    }
    tracing::info!("config validated successfully");

    let mut scheduler = default_scheduler();
    tracing::info!("worker started with {} jobs", scheduler.jobs.len());

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                tick_scheduler(&mut scheduler).await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received shutdown signal, exiting gracefully");
                break;
            }
        }
    }
    Ok(())
}

#[tracing::instrument(skip(scheduler))]
async fn tick_scheduler(scheduler: &mut Scheduler) {
    tracing::trace!("scheduler_tick_start");
    let now = Utc::now();
    let due = scheduler.due_jobs(now);

    if due.is_empty() {
        tracing::debug!("no jobs due at {}", now);
        return;
    }

    for kind in due {
        let run = execute_job(&kind).await;
        tracing::info!(
            job = kind.as_str(),
            status = format_status(&run),
            duration_ms = run.duration_ms(),
            "job completed"
        );
        scheduler.record_run(run);
    }
}

#[tracing::instrument(skip(kind), fields(job = %kind.as_str()))]
async fn execute_job(kind: &JobKind) -> JobRun {
    tracing::debug!(job = %kind.as_str(), "job_start");
    match kind {
        JobKind::CrawlCycle => {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            match load_nightly_inputs().await {
                Ok(inputs) => {
                    let stage = process_crawl_stage(&inputs.crawl);
                    match stage.run.status {
                        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                            run.succeed(stage.items, &format!("crawl completed: {}", stage.run.notes));
                        }
                        apex_worker::scheduler::JobStatus::Failed { .. } => {
                            run.fail(&format!("crawl failed: {}", stage.run.notes));
                        }
                        apex_worker::scheduler::JobStatus::Skipped { .. } => {
                            run.skip(&format!("crawl skipped: {}", stage.run.notes));
                        }
                        _ => {
                            run.skip("crawl stage not terminal");
                        }
                    }
                }
                Err(err) => {
                    run.skip(&format!("crawl inputs unavailable: {}", err));
                }
            }
            run
        }
        JobKind::PatternMining => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            match load_nightly_inputs().await {
                Ok(inputs) => {
                    let stage = process_mining_stage(&inputs.mining);
                    match stage.run.status {
                        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                            run.succeed(stage.items, &format!("mining completed: {}", stage.run.notes));
                        }
                        apex_worker::scheduler::JobStatus::Failed { .. } => {
                            run.fail(&format!("mining failed: {}", stage.run.notes));
                        }
                        _ => {
                            run.skip(&format!("mining stage not terminal: {}", stage.run.notes));
                        }
                    }
                }
                Err(err) => {
                    run.skip(&format!("nightly inputs unavailable: {}", err));
                }
            }
            run
        }
        JobKind::PoiRefresh => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            match load_nightly_inputs().await {
                Ok(inputs) => {
                    let stage = process_poi_stage(&inputs.poi);
                    match stage.run.status {
                        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                            run.succeed(stage.items, &format!("poi refresh completed: {}", stage.run.notes));
                        }
                        apex_worker::scheduler::JobStatus::Failed { .. } => {
                            run.fail(&format!("poi refresh failed: {}", stage.run.notes));
                        }
                        _ => {
                            run.skip(&format!("poi stage not terminal: {}", stage.run.notes));
                        }
                    }
                }
                Err(err) => {
                    run.skip(&format!("nightly inputs unavailable: {}", err));
                }
            }
            run
        }
        JobKind::FeatureDriftCheck => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            match load_nightly_inputs().await {
                Ok(inputs) => {
                    let stage = process_drift_stage(&inputs.drift);
                    match stage.run.status {
                        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                            run.succeed(stage.items, &format!("drift check completed: {}", stage.run.notes));
                        }
                        apex_worker::scheduler::JobStatus::Failed { .. } => {
                            run.fail(&format!("drift check failed: {}", stage.run.notes));
                        }
                        _ => {
                            run.skip(&format!("drift stage not terminal: {}", stage.run.notes));
                        }
                    }
                }
                Err(err) => {
                    run.skip(&format!("nightly inputs unavailable: {}", err));
                }
            }
            run
        }
        JobKind::PromotionBoard | JobKind::RecipeDeprecation | JobKind::StrategyMemo => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            match load_weekly_inputs().await {
                Ok(inputs) => {
                    let report = run_weekly_pipeline(
                        &inputs.staged_recipes,
                        &inputs.production_recipes,
                        &inputs.memo_inputs,
                        &inputs.promotion_policy.unwrap_or_default(),
                        &inputs.deprecation_policy.unwrap_or_default(),
                    );
                    if report.overall_success {
                        // Report the relevant metric for this specific job kind
                        let items = match kind {
                            JobKind::PromotionBoard => report
                                .promotion_result
                                .as_ref()
                                .map(|r| r.promoted.len() as u64)
                                .unwrap_or(0),
                            JobKind::RecipeDeprecation => report
                                .deprecation_result
                                .as_ref()
                                .map(|r| r.deprecated.len() as u64)
                                .unwrap_or(0),
                            JobKind::StrategyMemo => report
                                .memo
                                .as_ref()
                                .map(|m| m.sections.len() as u64)
                                .unwrap_or(0),
                            _ => 0,
                        };
                        run.succeed(items, &report.summary());
                    } else {
                        run.fail(&report.summary());
                    }
                }
                Err(err) => {
                    run.skip(&format!("weekly inputs unavailable: {}", err));
                }
            }
            run
        }
        JobKind::Custom(name) => {
            let mut run = JobRun::new(JobKind::Custom(name.clone()));
            run.start();
            let key = format!("CUSTOM_JOB_COMMAND_{}", name.to_uppercase());
            match std::env::var(&key) {
                Ok(command) if !command.trim().is_empty() => {
                    let timeout = std::time::Duration::from_secs(
                        std::env::var("CUSTOM_JOB_TIMEOUT_SECS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(300),
                    );
                    let status = tokio::time::timeout(
                        timeout,
                        tokio::process::Command::new("sh")
                            .arg("-c")
                            .arg(&command)
                            .status(),
                    )
                    .await;
                    match status {
                        Ok(Ok(exit)) if exit.success() => {
                            run.succeed(1, &format!("custom command succeeded: {}", key));
                        }
                        Ok(Ok(exit)) => {
                            run.fail(&format!("custom command exited with status: {}", exit));
                        }
                        Ok(Err(err)) => {
                            run.fail(&format!("custom command execution failed: {}", err));
                        }
                        Err(_) => {
                            run.fail(&format!("custom command timed out after {}s", timeout.as_secs()));
                        }
                    }
                }
                _ => {
                    run.skip(&format!("custom command env missing: {}", key));
                }
            }
            run
        }
    }
}

/// Read a local file to string with a file-size guard (B293).
///
/// **Never use `tokio::fs::read_to_string` directly on user-controlled or
/// environment-controlled paths.**  This function checks the file size before
/// attempting to read, preventing unbounded memory allocation.
///
/// # Errors
/// - Returns `Err` if the file does not exist, is not readable, or exceeds
///   `MAX_INPUT_FILE_BYTES`.
/// - Error messages are safe for logging (do not expose file content).
///
/// # Example
/// ```ignore
/// let content = read_file_with_size_check("runtime/job_input.json").await?;
/// let data: MyStruct = serde_json::from_str(&content)?;
/// ```
async fn read_file_with_size_check(path: &str) -> Result<String> {
    // Fetch file metadata before attempting to read
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        anyhow::anyhow!("failed to stat file '{}': {}", path, err)
    })?;

    let file_size = metadata.len();
    if file_size > MAX_INPUT_FILE_BYTES {
        anyhow::bail!(
            "file '{}' is {} bytes, which exceeds MAX_INPUT_FILE_BYTES ({}). \
             Refusing to read to prevent memory exhaustion.",
            path,
            file_size,
            MAX_INPUT_FILE_BYTES
        );
    }

    // Size is within limit; proceed with the read
    let content = tokio::fs::read_to_string(path).await.map_err(|err| {
        anyhow::anyhow!("failed to read file '{}': {}", path, err)
    })?;

    Ok(content)
}

async fn load_nightly_inputs() -> Result<NightlyInputs> {
    let path = std::env::var("NIGHTLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/nightly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<NightlyInputs>(&content)?;
    Ok(payload)
}

async fn load_weekly_inputs() -> Result<WeeklyInputs> {
    let path = std::env::var("WEEKLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/weekly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<WeeklyInputs>(&content)?;
    Ok(payload)
}

fn format_status(run: &JobRun) -> &'static str {
    match run.status {
        apex_worker::scheduler::JobStatus::Succeeded { .. } => "succeeded",
        apex_worker::scheduler::JobStatus::Failed { .. } => "failed",
        apex_worker::scheduler::JobStatus::Skipped { .. } => "skipped",
        apex_worker::scheduler::JobStatus::Running => "running",
        apex_worker::scheduler::JobStatus::Pending => "pending",
    }
}
