//! Nightly pipeline — crawl cycle, pattern mining, POI refresh, feature drift check.
//!
//! All functions are pure orchestration logic operating on typed stage results.
//! No real I/O — external callers inject data; this module sequences stages,
//! tracks progress, and produces a NightlyReport.

use crate::scheduler::{JobKind, JobRun, JobStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Stage results (inputs to the pipeline)
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStageResult {
    pub sources_attempted: u64,
    pub sources_succeeded: u64,
    pub sources_failed: u64,
    pub new_observations: u64,
    pub changed_pages: u64,
    pub bytes_fetched: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiningStageResult {
    pub candidates_found: u64,
    pub candidates_passed_gates: u64,
    pub hypotheses_generated: u64,
    pub recipes_staged: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiRefreshStageResult {
    pub profiles_scanned: u64,
    pub profiles_updated: u64,
    pub new_pois_discovered: u64,
    pub role_changes_detected: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftCheckStageResult {
    pub features_checked: u64,
    pub features_drifted: u64,
    pub drift_scores: Vec<(String, f64)>,
    pub alerts_raised: u64,
    pub errors: Vec<String>,
}

// ────────────────────────────────────────────
// Nightly pipeline
// ────────────────────────────────────────────

/// Pipeline stages in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NightlyStage {
    Crawl,
    Mining,
    PoiRefresh,
    DriftCheck,
}

impl NightlyStage {
    pub fn all() -> &'static [NightlyStage] {
        &[
            NightlyStage::Crawl,
            NightlyStage::Mining,
            NightlyStage::PoiRefresh,
            NightlyStage::DriftCheck,
        ]
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Crawl => "crawl",
            Self::Mining => "mining",
            Self::PoiRefresh => "poi_refresh",
            Self::DriftCheck => "drift_check",
        }
    }

    pub fn to_job_kind(&self) -> JobKind {
        match self {
            Self::Crawl => JobKind::CrawlCycle,
            Self::Mining => JobKind::PatternMining,
            Self::PoiRefresh => JobKind::PoiRefresh,
            Self::DriftCheck => JobKind::FeatureDriftCheck,
        }
    }
}

/// Result of a single stage execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageOutcome {
    pub stage: NightlyStage,
    pub run: JobRun,
    pub items: u64,
    pub error_count: u64,
}

/// Full nightly report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightlyReport {
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stages: Vec<StageOutcome>,
    pub overall_success: bool,
}

impl NightlyReport {
    pub fn new() -> Self {
        Self {
            started_at: Utc::now(),
            finished_at: None,
            stages: Vec::new(),
            overall_success: true,
        }
    }

    pub fn add_stage(&mut self, outcome: StageOutcome) {
        if !matches!(outcome.run.status, JobStatus::Succeeded { .. }) {
            self.overall_success = false;
        }
        self.stages.push(outcome);
    }

    pub fn finish(&mut self) {
        self.finished_at = Some(Utc::now());
    }

    pub fn total_items(&self) -> u64 {
        self.stages.iter().map(|s| s.items).sum()
    }

    pub fn total_errors(&self) -> u64 {
        self.stages.iter().map(|s| s.error_count).sum()
    }

    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    pub fn succeeded_stages(&self) -> usize {
        self.stages
            .iter()
            .filter(|s| matches!(s.run.status, JobStatus::Succeeded { .. }))
            .count()
    }

    pub fn failed_stages(&self) -> Vec<&StageOutcome> {
        self.stages
            .iter()
            .filter(|s| matches!(s.run.status, JobStatus::Failed { .. }))
            .collect()
    }

    /// Summary string for logging.
    pub fn summary(&self) -> String {
        let status = if self.overall_success {
            "SUCCESS"
        } else {
            "PARTIAL FAILURE"
        };
        format!(
            "Nightly [{}]: {}/{} stages OK, {} items, {} errors",
            status,
            self.succeeded_stages(),
            self.stage_count(),
            self.total_items(),
            self.total_errors()
        )
    }
}

impl Default for NightlyReport {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────
// Stage processing (pure functions)
// ────────────────────────────────────────────

/// Process crawl stage result into a StageOutcome.
pub fn process_crawl_stage(result: &CrawlStageResult) -> StageOutcome {
    let mut run = JobRun::new(JobKind::CrawlCycle);
    run.start();

    let error_count = result.errors.len() as u64;
    let items = result.new_observations + result.changed_pages;

    if result.sources_succeeded == 0 && result.sources_attempted > 0 {
        run.fail("all sources failed");
    } else if error_count > 0 {
        let msg = format!(
            "{}/{} sources OK, {} new obs, {} changed, {} errors",
            result.sources_succeeded,
            result.sources_attempted,
            result.new_observations,
            result.changed_pages,
            error_count
        );
        run.succeed(items, &msg);
    } else {
        let msg = format!(
            "{}/{} sources OK, {} new obs, {} changed",
            result.sources_succeeded,
            result.sources_attempted,
            result.new_observations,
            result.changed_pages
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::Crawl,
        run,
        items,
        error_count,
    }
}

/// Process mining stage result into a StageOutcome.
pub fn process_mining_stage(result: &MiningStageResult) -> StageOutcome {
    let mut run = JobRun::new(JobKind::PatternMining);
    run.start();

    let items = result.recipes_staged;
    let error_count = result.errors.len() as u64;

    if result.candidates_found == 0 {
        run.skip("no candidates found");
    } else if !result.errors.is_empty()
        && result.recipes_staged == 0
        && result.candidates_passed_gates > 0
    {
        run.fail(&format!(
            "hypothesis generation failed: {}",
            result.errors.join("; ")
        ));
    } else {
        let msg = format!(
            "{} candidates → {} passed gates → {} hypotheses → {} staged",
            result.candidates_found,
            result.candidates_passed_gates,
            result.hypotheses_generated,
            result.recipes_staged
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::Mining,
        run,
        items,
        error_count,
    }
}

/// Process POI refresh result into a StageOutcome.
pub fn process_poi_stage(result: &PoiRefreshStageResult) -> StageOutcome {
    let mut run = JobRun::new(JobKind::PoiRefresh);
    run.start();

    let items = result.profiles_updated + result.new_pois_discovered;
    let error_count = result.errors.len() as u64;

    if result.profiles_scanned == 0 {
        run.skip("no profiles to scan");
    } else if !result.errors.is_empty() && result.profiles_updated == 0 {
        run.fail(&format!(
            "all updates failed: {}",
            result.errors.join("; ")
        ));
    } else {
        let msg = format!(
            "{}/{} updated, {} new POIs, {} role changes",
            result.profiles_updated,
            result.profiles_scanned,
            result.new_pois_discovered,
            result.role_changes_detected
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::PoiRefresh,
        run,
        items,
        error_count,
    }
}

/// Process drift check result into a StageOutcome.
pub fn process_drift_stage(result: &DriftCheckStageResult) -> StageOutcome {
    let mut run = JobRun::new(JobKind::FeatureDriftCheck);
    run.start();

    let items = result.features_checked;
    let error_count = result.errors.len() as u64;

    if result.features_checked == 0 {
        run.skip("no features to check");
    } else if !result.errors.is_empty() && result.features_checked == result.errors.len() as u64 {
        run.fail("all feature checks failed");
    } else {
        let msg = format!(
            "{}/{} features drifted, {} alerts",
            result.features_drifted, result.features_checked, result.alerts_raised
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::DriftCheck,
        run,
        items,
        error_count,
    }
}

/// Run the full nightly pipeline from pre-computed stage results.
/// This is the pure orchestration function — no I/O.
pub fn run_nightly_pipeline(
    crawl: &CrawlStageResult,
    mining: &MiningStageResult,
    poi: &PoiRefreshStageResult,
    drift: &DriftCheckStageResult,
) -> NightlyReport {
    let mut report = NightlyReport::new();

    report.add_stage(process_crawl_stage(crawl));
    report.add_stage(process_mining_stage(mining));
    report.add_stage(process_poi_stage(poi));
    report.add_stage(process_drift_stage(drift));

    report.finish();
    report
}

/// Decide whether the pipeline should proceed to the next stage given previous failures.
/// Policy: crawl failure blocks mining (no fresh data), but POI and drift are independent.
pub fn should_proceed(report: &NightlyReport, next_stage: NightlyStage) -> bool {
    match next_stage {
        NightlyStage::Crawl => true,
        NightlyStage::Mining => {
            // Mining needs crawl data
            report
                .stages
                .iter()
                .any(|s| s.stage == NightlyStage::Crawl && matches!(s.run.status, JobStatus::Succeeded { .. }))
        }
        NightlyStage::PoiRefresh => true, // independent
        NightlyStage::DriftCheck => true,  // independent
    }
}

/// Compute a health score for the nightly pipeline (0.0-1.0).
pub fn pipeline_health(report: &NightlyReport) -> f64 {
    if report.stages.is_empty() {
        return 0.0;
    }
    let total = report.stages.len() as f64;
    let ok = report.succeeded_stages() as f64;
    let skipped = report
        .stages
        .iter()
        .filter(|s| matches!(s.run.status, JobStatus::Skipped { .. }))
        .count() as f64;
    // Skipped counts as 0.5 — it's not a failure but not a success either
    (ok + skipped * 0.5) / total
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn good_crawl() -> CrawlStageResult {
        CrawlStageResult {
            sources_attempted: 50,
            sources_succeeded: 48,
            sources_failed: 2,
            new_observations: 320,
            changed_pages: 15,
            bytes_fetched: 5_000_000,
            errors: vec!["timeout on src1".to_string(), "404 on src2".to_string()],
        }
    }

    fn failed_crawl() -> CrawlStageResult {
        CrawlStageResult {
            sources_attempted: 50,
            sources_succeeded: 0,
            sources_failed: 50,
            new_observations: 0,
            changed_pages: 0,
            bytes_fetched: 0,
            errors: (0..50).map(|i| format!("failed {}", i)).collect(),
        }
    }

    fn good_mining() -> MiningStageResult {
        MiningStageResult {
            candidates_found: 25,
            candidates_passed_gates: 8,
            hypotheses_generated: 5,
            recipes_staged: 3,
            errors: vec![],
        }
    }

    fn empty_mining() -> MiningStageResult {
        MiningStageResult {
            candidates_found: 0,
            candidates_passed_gates: 0,
            hypotheses_generated: 0,
            recipes_staged: 0,
            errors: vec![],
        }
    }

    fn failed_mining() -> MiningStageResult {
        MiningStageResult {
            candidates_found: 10,
            candidates_passed_gates: 3,
            hypotheses_generated: 0,
            recipes_staged: 0,
            errors: vec!["LLM timeout".to_string()],
        }
    }

    fn good_poi() -> PoiRefreshStageResult {
        PoiRefreshStageResult {
            profiles_scanned: 100,
            profiles_updated: 12,
            new_pois_discovered: 3,
            role_changes_detected: 2,
            errors: vec![],
        }
    }

    fn empty_poi() -> PoiRefreshStageResult {
        PoiRefreshStageResult {
            profiles_scanned: 0,
            profiles_updated: 0,
            new_pois_discovered: 0,
            role_changes_detected: 0,
            errors: vec![],
        }
    }

    fn good_drift() -> DriftCheckStageResult {
        DriftCheckStageResult {
            features_checked: 40,
            features_drifted: 2,
            drift_scores: vec![
                ("supplier_risk".to_string(), 0.15),
                ("commodity_vol".to_string(), 0.22),
            ],
            alerts_raised: 1,
            errors: vec![],
        }
    }

    fn empty_drift() -> DriftCheckStageResult {
        DriftCheckStageResult {
            features_checked: 0,
            features_drifted: 0,
            drift_scores: vec![],
            alerts_raised: 0,
            errors: vec![],
        }
    }

    // ── Stage processing ──

    #[test]
    fn test_crawl_stage_success() {
        let result = good_crawl();
        let outcome = process_crawl_stage(&result);
        assert_eq!(outcome.stage, NightlyStage::Crawl);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 335); // 320 + 15
        assert_eq!(outcome.error_count, 2);
    }

    #[test]
    fn test_crawl_stage_all_fail() {
        let result = failed_crawl();
        let outcome = process_crawl_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_crawl_stage_perfect() {
        let result = CrawlStageResult {
            sources_attempted: 10,
            sources_succeeded: 10,
            sources_failed: 0,
            new_observations: 100,
            changed_pages: 5,
            bytes_fetched: 1_000_000,
            errors: vec![],
        };
        let outcome = process_crawl_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.error_count, 0);
    }

    #[test]
    fn test_mining_stage_success() {
        let result = good_mining();
        let outcome = process_mining_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 3);
        assert!(outcome.run.notes.contains("25 candidates"));
    }

    #[test]
    fn test_mining_stage_no_candidates() {
        let result = empty_mining();
        let outcome = process_mining_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Skipped { .. }));
    }

    #[test]
    fn test_mining_stage_fail() {
        let result = failed_mining();
        let outcome = process_mining_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_poi_stage_success() {
        let result = good_poi();
        let outcome = process_poi_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 15); // 12 + 3
        assert!(outcome.run.notes.contains("role changes"));
    }

    #[test]
    fn test_poi_stage_no_profiles() {
        let result = empty_poi();
        let outcome = process_poi_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Skipped { .. }));
    }

    #[test]
    fn test_poi_stage_all_fail() {
        let result = PoiRefreshStageResult {
            profiles_scanned: 10,
            profiles_updated: 0,
            new_pois_discovered: 0,
            role_changes_detected: 0,
            errors: vec!["db error".to_string()],
        };
        let outcome = process_poi_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_drift_stage_success() {
        let result = good_drift();
        let outcome = process_drift_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 40);
    }

    #[test]
    fn test_drift_stage_no_features() {
        let result = empty_drift();
        let outcome = process_drift_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Skipped { .. }));
    }

    // ── Full pipeline ──

    #[test]
    fn test_full_pipeline_all_good() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        assert!(report.overall_success);
        assert_eq!(report.stage_count(), 4);
        assert_eq!(report.succeeded_stages(), 4);
        assert!(report.finished_at.is_some());
        assert!(report.total_items() > 0);
    }

    #[test]
    fn test_full_pipeline_crawl_fails() {
        let report = run_nightly_pipeline(
            &failed_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        assert!(!report.overall_success);
        assert_eq!(report.failed_stages().len(), 1);
        assert_eq!(report.failed_stages()[0].stage, NightlyStage::Crawl);
    }

    #[test]
    fn test_full_pipeline_mining_skipped() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(),
            &good_poi(),
            &good_drift(),
        );
        // Skipped is not a success
        assert!(!report.overall_success);
        assert_eq!(report.succeeded_stages(), 3);
    }

    #[test]
    fn test_full_pipeline_multiple_failures() {
        let report = run_nightly_pipeline(
            &failed_crawl(),
            &failed_mining(),
            &empty_poi(),
            &empty_drift(),
        );
        assert!(!report.overall_success);
        assert_eq!(report.failed_stages().len(), 2);
    }

    #[test]
    fn test_nightly_report_summary_success() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        let summary = report.summary();
        assert!(summary.contains("SUCCESS"));
        assert!(summary.contains("4/4"));
    }

    #[test]
    fn test_nightly_report_summary_failure() {
        let report = run_nightly_pipeline(
            &failed_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        let summary = report.summary();
        assert!(summary.contains("PARTIAL FAILURE"));
        assert!(summary.contains("3/4"));
    }

    // ── should_proceed ──

    #[test]
    fn test_should_proceed_crawl_always() {
        let report = NightlyReport::new();
        assert!(should_proceed(&report, NightlyStage::Crawl));
    }

    #[test]
    fn test_should_proceed_mining_after_crawl_success() {
        let mut report = NightlyReport::new();
        report.add_stage(process_crawl_stage(&good_crawl()));
        assert!(should_proceed(&report, NightlyStage::Mining));
    }

    #[test]
    fn test_should_proceed_mining_blocked_after_crawl_fail() {
        let mut report = NightlyReport::new();
        report.add_stage(process_crawl_stage(&failed_crawl()));
        assert!(!should_proceed(&report, NightlyStage::Mining));
    }

    #[test]
    fn test_should_proceed_poi_always() {
        let mut report = NightlyReport::new();
        report.add_stage(process_crawl_stage(&failed_crawl()));
        assert!(should_proceed(&report, NightlyStage::PoiRefresh));
    }

    #[test]
    fn test_should_proceed_drift_always() {
        let report = NightlyReport::new();
        assert!(should_proceed(&report, NightlyStage::DriftCheck));
    }

    // ── pipeline_health ──

    #[test]
    fn test_pipeline_health_all_good() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        assert!((pipeline_health(&report) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pipeline_health_all_fail() {
        let report = run_nightly_pipeline(
            &failed_crawl(),
            &failed_mining(),
            &PoiRefreshStageResult {
                profiles_scanned: 10,
                profiles_updated: 0,
                new_pois_discovered: 0,
                role_changes_detected: 0,
                errors: vec!["err".to_string()],
            },
            &DriftCheckStageResult {
                features_checked: 5,
                features_drifted: 0,
                drift_scores: vec![],
                alerts_raised: 0,
                errors: vec!["e1".to_string(), "e2".to_string(), "e3".to_string(), "e4".to_string(), "e5".to_string()],
            },
        );
        assert!((pipeline_health(&report) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_pipeline_health_mixed() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            &good_poi(),
            &good_drift(),
        );
        // 3 success + 1 skipped (0.5) = 3.5/4 = 0.875
        assert!((pipeline_health(&report) - 0.875).abs() < 0.01);
    }

    #[test]
    fn test_pipeline_health_empty() {
        let report = NightlyReport::new();
        assert_eq!(pipeline_health(&report), 0.0);
    }

    // ── NightlyStage ──

    #[test]
    fn test_nightly_stage_all() {
        let stages = NightlyStage::all();
        assert_eq!(stages.len(), 4);
        assert_eq!(stages[0], NightlyStage::Crawl);
        assert_eq!(stages[3], NightlyStage::DriftCheck);
    }

    #[test]
    fn test_nightly_stage_as_str() {
        assert_eq!(NightlyStage::Crawl.as_str(), "crawl");
        assert_eq!(NightlyStage::Mining.as_str(), "mining");
        assert_eq!(NightlyStage::PoiRefresh.as_str(), "poi_refresh");
        assert_eq!(NightlyStage::DriftCheck.as_str(), "drift_check");
    }

    #[test]
    fn test_nightly_stage_to_job_kind() {
        assert_eq!(NightlyStage::Crawl.to_job_kind(), JobKind::CrawlCycle);
        assert_eq!(NightlyStage::Mining.to_job_kind(), JobKind::PatternMining);
        assert_eq!(NightlyStage::PoiRefresh.to_job_kind(), JobKind::PoiRefresh);
        assert_eq!(
            NightlyStage::DriftCheck.to_job_kind(),
            JobKind::FeatureDriftCheck
        );
    }

    // ── NightlyReport ──

    #[test]
    fn test_report_total_items() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        // crawl: 320+15=335, mining: 3, poi: 12+3=15, drift: 40
        assert_eq!(report.total_items(), 335 + 3 + 15 + 40);
    }

    #[test]
    fn test_report_total_errors() {
        let report = run_nightly_pipeline(
            &good_crawl(), // 2 errors
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        assert_eq!(report.total_errors(), 2);
    }

    #[test]
    fn test_stage_result_serialization() {
        let result = good_crawl();
        let json = serde_json::to_string(&result).unwrap();
        let back: CrawlStageResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sources_attempted, 50);
        assert_eq!(back.sources_succeeded, 48);
    }

    #[test]
    fn test_nightly_report_serialization() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            &good_poi(),
            &good_drift(),
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: NightlyReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.stage_count(), 4);
        assert!(back.overall_success);
    }
}
