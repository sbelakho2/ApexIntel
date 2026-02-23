use anyhow::Result;
use apex_worker::nightly::{
    process_crawl_stage, run_nightly_pipeline, CrawlStageResult, DriftCheckStageResult, MiningStageResult,
    PoiRefreshStageResult,
};
use apex_worker::scheduler::{default_scheduler, JobKind, JobRun, Scheduler};
use apex_worker::weekly::{
    run_weekly_pipeline, DeprecationPolicy, MemoInputs, PromotionPolicy, ProductionRecipe,
    StagedRecipe,
};
use chrono::Utc;
use serde::Deserialize;

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
    tracing_subscriber::fmt().with_target(false).init();

    let mut scheduler = default_scheduler();
    tracing::info!("worker started with {} jobs", scheduler.jobs.len());

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        tick_scheduler(&mut scheduler).await;
    }
}

async fn tick_scheduler(scheduler: &mut Scheduler) {
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

async fn execute_job(kind: &JobKind) -> JobRun {
    match kind {
        JobKind::CrawlCycle => {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            match load_nightly_inputs() {
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
        JobKind::PatternMining | JobKind::PoiRefresh | JobKind::FeatureDriftCheck => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            match load_nightly_inputs() {
                Ok(inputs) => {
                    let report = run_nightly_pipeline(
                        &inputs.crawl,
                        &inputs.mining,
                        &inputs.poi,
                        &inputs.drift,
                    );
                    if report.overall_success {
                        run.succeed(report.total_items(), &report.summary());
                    } else {
                        run.fail(&report.summary());
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
            match load_weekly_inputs() {
                Ok(inputs) => {
                    let report = run_weekly_pipeline(
                        &inputs.staged_recipes,
                        &inputs.production_recipes,
                        &inputs.memo_inputs,
                        &inputs.promotion_policy.unwrap_or_default(),
                        &inputs.deprecation_policy.unwrap_or_default(),
                    );
                    if report.overall_success {
                        run.succeed(
                            report
                                .promotion_result
                                .as_ref()
                                .map(|r| r.promoted.len() as u64)
                                .unwrap_or(0),
                            &report.summary(),
                        );
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
                    let status = tokio::process::Command::new("sh")
                        .arg("-c")
                        .arg(&command)
                        .status()
                        .await;
                    match status {
                        Ok(exit) if exit.success() => {
                            run.succeed(1, &format!("custom command succeeded: {}", key));
                        }
                        Ok(exit) => {
                            run.fail(&format!("custom command exited with status: {}", exit));
                        }
                        Err(err) => {
                            run.fail(&format!("custom command execution failed: {}", err));
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

fn load_nightly_inputs() -> Result<NightlyInputs> {
    let path = std::env::var("NIGHTLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/nightly_inputs.json".to_string());
    let content = std::fs::read_to_string(&path)?;
    let payload = serde_json::from_str::<NightlyInputs>(&content)?;
    Ok(payload)
}

fn load_weekly_inputs() -> Result<WeeklyInputs> {
    let path = std::env::var("WEEKLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/weekly_inputs.json".to_string());
    let content = std::fs::read_to_string(&path)?;
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
