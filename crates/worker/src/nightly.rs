//! Nightly pipeline — crawl cycle, pattern mining, POI refresh, feature drift check.
//!
//! All functions are pure orchestration logic operating on typed stage results.
//! No real I/O — external callers inject data; this module sequences stages,
//! tracks progress, and produces a NightlyReport.

use crate::scheduler::{JobKind, JobRun, JobStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Batch-size guard (B286)
// ────────────────────────────────────────────

/// Hard ceiling on the number of items any single nightly pipeline stage may
/// process in one run (B286).
///
/// If any stage result reports more items than this value the entire pipeline
/// is aborted immediately — before any CPU or memory is committed — and every
/// stage receives a `Failed` outcome.  This prevents a misconfigured data feed
/// from unboundedly occupying the nightly window or exhausting heap.
///
/// Rationale for 100 000: the median production nightly run processes ≤ 5 000
/// sources and ≤ 20 000 pattern candidates.  100 000 is a 5× safety margin;
/// exceeding it almost certainly indicates a runaway input bug rather than
/// legitimate growth.
pub const MAX_NIGHTLY_BATCH_SIZE: u64 = 100_000;

/// Data captured during the web-crawl stage.
///
/// Summarises how many sources were contacted, what was retrieved, and what
/// went wrong.  Used as the input to [`process_crawl_stage`].
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

impl CrawlStageResult {
    /// Validate logical consistency of crawl data (B245).
    ///
    /// Checks that `sources_succeeded` and `sources_failed` individually and
    /// collectively do not exceed `sources_attempted`.
    pub fn validate(&self) -> Result<(), String> {
        if self.sources_succeeded > self.sources_attempted {
            return Err(format!(
                "sources_succeeded ({}) > sources_attempted ({})",
                self.sources_succeeded, self.sources_attempted
            ));
        }
        if self.sources_failed > self.sources_attempted {
            return Err(format!(
                "sources_failed ({}) > sources_attempted ({})",
                self.sources_failed, self.sources_attempted
            ));
        }
        if self.sources_succeeded.saturating_add(self.sources_failed) > self.sources_attempted {
            return Err(format!(
                "sources_succeeded ({}) + sources_failed ({}) > sources_attempted ({})",
                self.sources_succeeded, self.sources_failed, self.sources_attempted
            ));
        }
        Ok(())
    }
}

/// Data captured during the pattern-mining stage.
///
/// Models the candidate funnel: every downstream count must be ≤ its upstream
/// (enforced by [`MiningStageResult::validate`]).  A zero `candidates_found`
/// causes the stage to be **Skipped** rather than Failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiningStageResult {
    pub candidates_found: u64,
    pub candidates_passed_gates: u64,
    pub hypotheses_generated: u64,
    pub recipes_staged: u64,
    pub errors: Vec<String>,
}

impl MiningStageResult {
    /// Validate mining funnel monotonicity: each downstream count ≤ upstream (B245).
    pub fn validate(&self) -> Result<(), String> {
        if self.candidates_passed_gates > self.candidates_found {
            return Err(format!(
                "candidates_passed_gates ({}) > candidates_found ({})",
                self.candidates_passed_gates, self.candidates_found
            ));
        }
        if self.hypotheses_generated > self.candidates_passed_gates {
            return Err(format!(
                "hypotheses_generated ({}) > candidates_passed_gates ({})",
                self.hypotheses_generated, self.candidates_passed_gates
            ));
        }
        if self.recipes_staged > self.hypotheses_generated {
            return Err(format!(
                "recipes_staged ({}) > hypotheses_generated ({})",
                self.recipes_staged, self.hypotheses_generated
            ));
        }
        Ok(())
    }
}

/// Data captured during the Person-of-Interest (POI) refresh stage.
///
/// Tracks how many analyst profiles were scanned, updated, and whether any
/// new POIs or role changes were discovered.  A zero `profiles_scanned`
/// causes the stage to be **Skipped**.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiRefreshStageResult {
    pub profiles_scanned: u64,
    pub profiles_updated: u64,
    pub new_pois_discovered: u64,
    pub role_changes_detected: u64,
    pub errors: Vec<String>,
}

impl PoiRefreshStageResult {
    /// Validate that profiles_updated does not exceed profiles_scanned (B245).
    pub fn validate(&self) -> Result<(), String> {
        if self.profiles_updated > self.profiles_scanned {
            return Err(format!(
                "profiles_updated ({}) > profiles_scanned ({})",
                self.profiles_updated, self.profiles_scanned
            ));
        }
        Ok(())
    }
}

/// Data captured during the feature drift-check stage.
///
/// Tracks which model features shifted beyond their control thresholds.
/// `drift_scores` is a list of `(feature_name, score)` pairs (score ∈ [0, 1]).
/// A zero `features_checked` causes the stage to be **Skipped**.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftCheckStageResult {
    pub features_checked: u64,
    pub features_drifted: u64,
    pub drift_scores: Vec<(String, f64)>,
    pub alerts_raised: u64,
    pub errors: Vec<String>,
}

impl DriftCheckStageResult {
    /// Validate that drifted ≤ checked and alerts ≤ drifted (B245).
    pub fn validate(&self) -> Result<(), String> {
        if self.features_drifted > self.features_checked {
            return Err(format!(
                "features_drifted ({}) > features_checked ({})",
                self.features_drifted, self.features_checked
            ));
        }
        if self.alerts_raised > self.features_drifted {
            return Err(format!(
                "alerts_raised ({}) > features_drifted ({})",
                self.alerts_raised, self.features_drifted
            ));
        }
        Ok(())
    }
}

/// Data captured during the LLM-backed hypothesis generation stage.
///
/// This stage takes the candidates that passed statistical gates from Mining
/// and sends them through the LLM to produce structured recipe hypotheses.
/// Requires the `llm` feature flag; when disabled, this stage is automatically
/// skipped in the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisGenerationStageResult {
    /// Number of candidates that were submitted to the LLM.
    pub candidates_submitted: u64,
    /// Number of hypotheses successfully generated and validated.
    pub hypotheses_generated: u64,
    /// Number of hypotheses that failed LLM generation or validation.
    pub hypotheses_failed: u64,
    /// Number of generated hypotheses staged as candidate recipes.
    pub recipes_staged: u64,
    pub errors: Vec<String>,
}

impl HypothesisGenerationStageResult {
    /// Validate funnel monotonicity (B245).
    pub fn validate(&self) -> Result<(), String> {
        if self.hypotheses_generated > self.candidates_submitted {
            return Err(format!(
                "hypotheses_generated ({}) > candidates_submitted ({})",
                self.hypotheses_generated, self.candidates_submitted
            ));
        }
        let total_outcomes = self.hypotheses_generated + self.hypotheses_failed;
        if total_outcomes > self.candidates_submitted {
            return Err(format!(
                "hypotheses_generated ({}) + hypotheses_failed ({}) > candidates_submitted ({})",
                self.hypotheses_generated, self.hypotheses_failed, self.candidates_submitted
            ));
        }
        if self.recipes_staged > self.hypotheses_generated {
            return Err(format!(
                "recipes_staged ({}) > hypotheses_generated ({})",
                self.recipes_staged, self.hypotheses_generated
            ));
        }
        Ok(())
    }
}

// ────────────────────────────────────────────
// Nightly pipeline
// ────────────────────────────────────────────

/// Results from the POI network-expansion discovery stage.
///
/// The expansion engine queries multiple OSINT sources (org leadership pages,
/// GDELT co-mentions, OpenCorporates boards, conference speaker directories,
/// and optionally Tor onion sources) using existing seed POIs as starting points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryStageResult {
    /// Number of existing seed POIs that were used as starting points.
    pub seeds_processed: u64,
    /// Number of candidate new POIs found.
    pub candidates_found: u64,
    /// Number of candidates inserted or upserted into the database.
    pub new_pois_inserted: u64,
    /// Number of existing persons whose contact details were enriched.
    pub contacts_enriched: u64,
    /// Whether the Tor dark-web sources were included in this run.
    pub tor_sources_used: bool,
    pub errors: Vec<String>,
}

impl DiscoveryStageResult {
    pub fn empty() -> Self {
        Self {
            seeds_processed: 0,
            candidates_found: 0,
            new_pois_inserted: 0,
            contacts_enriched: 0,
            tor_sources_used: false,
            errors: vec![],
        }
    }
}

/// Identifies one stage in the nightly pipeline.
///
/// Used as a discriminant in [`StageOutcome`] and as the argument to
/// [`should_proceed`] to query whether a stage's prerequisites are met.
/// Stages run in their `all()` order:
///   Crawl → Mining → HypothesisGeneration → PoiRefresh → DriftCheck.
///
/// `HypothesisGeneration` is gated behind the `llm` feature flag.
/// When the feature is disabled, the stage is skipped automatically and
/// the pipeline falls back to the 4-stage sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NightlyStage {
    Crawl,
    Mining,
    /// LLM-backed hypothesis generation from mined pattern candidates.
    /// Requires the `llm` feature flag.
    HypothesisGeneration,
    PoiRefresh,
    DriftCheck,
    /// Network-expansion discovery — finds new POI candidates from existing seeds.
    PoiDiscovery,
}

impl NightlyStage {
    pub fn all() -> &'static [NightlyStage] {
        &[
            NightlyStage::Crawl,
            NightlyStage::Mining,
            NightlyStage::HypothesisGeneration,
            NightlyStage::PoiRefresh,
            NightlyStage::DriftCheck,
            NightlyStage::PoiDiscovery,
        ]
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Crawl => "crawl",
            Self::Mining => "mining",
            Self::HypothesisGeneration => "hypothesis_generation",
            Self::PoiRefresh => "poi_refresh",
            Self::DriftCheck => "drift_check",
            Self::PoiDiscovery => "poi_discovery",
        }
    }

    pub fn to_job_kind(&self) -> JobKind {
        match self {
            Self::Crawl => JobKind::CrawlCycle,
            Self::Mining => JobKind::PatternMining,
            Self::HypothesisGeneration => JobKind::HypothesisGeneration,
            Self::PoiRefresh => JobKind::PoiRefresh,
            Self::DriftCheck => JobKind::FeatureDriftCheck,
            Self::PoiDiscovery => JobKind::PoiDiscovery,
        }
    }
}

/// Outcome record produced by processing one pipeline stage.
///
/// Bundles the raw [`JobRun`] (containing run-id, timing, and status) with
/// aggregated counters `items` and `error_count` for quick roll-up in
/// [`NightlyReport`] methods such as [`total_items`](NightlyReport::total_items).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageOutcome {
    pub stage: NightlyStage,
    pub run: JobRun,
    pub items: u64,
    pub error_count: u64,
}

/// Aggregated report produced by [`run_nightly_pipeline`].
///
/// Contains:
/// - Wall-clock timestamps bracketing the full pipeline run.
/// - One [`StageOutcome`] per stage, in execution order.
/// - `overall_success`: `true` only when **every** stage succeeded.
///
/// Use the helper methods ([`total_items`](Self::total_items),
/// [`skipped_stages`](Self::skipped_stages), [`pipeline_health`](super::nightly::pipeline_health), …)
/// rather than iterating `stages` directly to avoid coupling to internal layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightlyReport {
    pub schema_version: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stages: Vec<StageOutcome>,
    pub overall_success: bool,
}

impl NightlyReport {
    pub fn new() -> Self {
        Self {
            schema_version: "v1".to_string(),
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

    pub fn skipped_stages(&self) -> usize {
        self.stages
            .iter()
            .filter(|s| matches!(s.run.status, JobStatus::Skipped { .. }))
            .count()
    }

    pub fn failed_stages(&self) -> Vec<&StageOutcome> {
        self.stages
            .iter()
            .filter(|s| matches!(s.run.status, JobStatus::Failed { .. }))
            .collect()
    }

    /// Summary string for logging (B244: includes skipped-stage count).
    pub fn summary(&self) -> String {
        let status = if self.overall_success {
            "SUCCESS"
        } else {
            "PARTIAL FAILURE"
        };
        let skipped = self.skipped_stages();
        if skipped > 0 {
            format!(
                "Nightly [{}]: {}/{} stages OK, {} skipped, {} items, {} errors",
                status,
                self.succeeded_stages(),
                self.stage_count(),
                skipped,
                self.total_items(),
                self.total_errors()
            )
        } else {
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
        run.fail(&format!("all updates failed: {}", result.errors.join("; ")));
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

/// Process hypothesis-generation stage result into a StageOutcome.
pub fn process_hypothesis_generation_stage(
    result: &HypothesisGenerationStageResult,
) -> StageOutcome {
    let mut run = JobRun::new(JobKind::HypothesisGeneration);
    run.start();

    let items = result.recipes_staged;
    let error_count = result.errors.len() as u64;

    if result.candidates_submitted == 0 {
        run.skip("no candidates to generate hypotheses for");
    } else if result.hypotheses_generated == 0 && !result.errors.is_empty() {
        run.fail(&format!(
            "all hypothesis generations failed: {}",
            result.errors.join("; ")
        ));
    } else {
        let msg = format!(
            "{} submitted → {} generated, {} failed → {} staged",
            result.candidates_submitted,
            result.hypotheses_generated,
            result.hypotheses_failed,
            result.recipes_staged
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::HypothesisGeneration,
        run,
        items,
        error_count,
    }
}

/// Process a POI discovery stage result into a StageOutcome.
pub fn process_discovery_stage(result: &DiscoveryStageResult) -> StageOutcome {
    let mut run = JobRun::new(JobKind::PoiDiscovery);
    run.start();

    let items = result.new_pois_inserted + result.contacts_enriched;
    let error_count = result.errors.len() as u64;

    if result.seeds_processed == 0 {
        run.skip("no seed POIs available for expansion");
    } else if !result.errors.is_empty() && result.candidates_found == 0 {
        run.fail(&format!("discovery failed: {}", result.errors.join("; ")));
    } else {
        let tor_note = if result.tor_sources_used {
            " (tor enabled)"
        } else {
            ""
        };
        let msg = format!(
            "{} seeds → {} candidates → {} inserted, {} contacts enriched{}",
            result.seeds_processed,
            result.candidates_found,
            result.new_pois_inserted,
            result.contacts_enriched,
            tor_note
        );
        run.succeed(items, &msg);
    }

    StageOutcome {
        stage: NightlyStage::PoiDiscovery,
        run,
        items,
        error_count,
    }
}

/// Run the full nightly pipeline from pre-computed stage results.
/// This is the pure orchestration function — no I/O.
///
/// The `hypothesis` parameter is optional: when the `llm` feature is disabled
/// (or no candidates passed mining), pass `None` and the stage will be recorded
/// as Skipped.
///
/// The `discovery` parameter is optional: pass `None` to skip the POI
/// network-expansion stage (e.g. lightweight runs or when the crawl stage failed).
///
/// Emits structured tracing events at each stage boundary with batch-size
/// and error-count fields to enable downstream alerting and dashboards (B285).
#[tracing::instrument(skip(crawl, mining, hypothesis, poi, drift))]
pub fn run_nightly_pipeline(
    crawl: &CrawlStageResult,
    mining: &MiningStageResult,
    hypothesis: Option<&HypothesisGenerationStageResult>,
    poi: &PoiRefreshStageResult,
    drift: &DriftCheckStageResult,
) -> NightlyReport {
    run_nightly_pipeline_full(crawl, mining, hypothesis, poi, drift, None)
}

/// Extended variant that also runs the optional POI network-expansion discovery stage.
pub fn run_nightly_pipeline_full(
    crawl: &CrawlStageResult,
    mining: &MiningStageResult,
    hypothesis: Option<&HypothesisGenerationStageResult>,
    poi: &PoiRefreshStageResult,
    drift: &DriftCheckStageResult,
    discovery: Option<&DiscoveryStageResult>,
) -> NightlyReport {
    let mut report = NightlyReport::new();

    // ── Stage 1: Crawl ──
    tracing::info!(
        stage = "crawl",
        sources_attempted = crawl.sources_attempted,
        sources_succeeded = crawl.sources_succeeded,
        sources_failed = crawl.sources_failed,
        bytes_fetched = crawl.bytes_fetched,
        error_count = crawl.errors.len(),
        "nightly_stage_begin"
    );
    let crawl_outcome = process_crawl_stage(crawl);
    tracing::info!(
        stage = "crawl",
        items = crawl_outcome.items,
        error_count = crawl_outcome.error_count,
        status = ?crawl_outcome.run.status,
        "nightly_stage_complete"
    );
    report.add_stage(crawl_outcome);

    // ── Stage 2: Pattern Mining ──
    tracing::info!(
        stage = "mining",
        candidates_found = mining.candidates_found,
        candidates_passed_gates = mining.candidates_passed_gates,
        hypotheses_generated = mining.hypotheses_generated,
        recipes_staged = mining.recipes_staged,
        error_count = mining.errors.len(),
        "nightly_stage_begin"
    );
    let mining_outcome = process_mining_stage(mining);
    tracing::info!(
        stage = "mining",
        items = mining_outcome.items,
        error_count = mining_outcome.error_count,
        status = ?mining_outcome.run.status,
        "nightly_stage_complete"
    );
    report.add_stage(mining_outcome);

    // ── Stage 3: Hypothesis Generation (LLM) ──
    if let Some(hyp) = hypothesis {
        tracing::info!(
            stage = "hypothesis_generation",
            candidates_submitted = hyp.candidates_submitted,
            hypotheses_generated = hyp.hypotheses_generated,
            hypotheses_failed = hyp.hypotheses_failed,
            recipes_staged = hyp.recipes_staged,
            error_count = hyp.errors.len(),
            "nightly_stage_begin"
        );
        let hyp_outcome = process_hypothesis_generation_stage(hyp);
        tracing::info!(
            stage = "hypothesis_generation",
            items = hyp_outcome.items,
            error_count = hyp_outcome.error_count,
            status = ?hyp_outcome.run.status,
            "nightly_stage_complete"
        );
        report.add_stage(hyp_outcome);
    } else {
        // LLM feature disabled — omit stage from report entirely.
        // This preserves backward-compatible overall_success semantics
        // (a 4-stage pipeline without LLM behaves identically to before).
        tracing::debug!(
            stage = "hypothesis_generation",
            "skipped: hypothesis generation not available (llm feature disabled)"
        );
    }

    // ── Stage 4: POI Refresh ──
    tracing::info!(
        stage = "poi_refresh",
        profiles_scanned = poi.profiles_scanned,
        profiles_updated = poi.profiles_updated,
        new_pois_discovered = poi.new_pois_discovered,
        role_changes_detected = poi.role_changes_detected,
        error_count = poi.errors.len(),
        "nightly_stage_begin"
    );
    let poi_outcome = process_poi_stage(poi);
    tracing::info!(
        stage = "poi_refresh",
        items = poi_outcome.items,
        error_count = poi_outcome.error_count,
        status = ?poi_outcome.run.status,
        "nightly_stage_complete"
    );
    report.add_stage(poi_outcome);

    // ── Stage 5: Feature Drift ──
    tracing::info!(
        stage = "drift_check",
        features_checked = drift.features_checked,
        features_drifted = drift.features_drifted,
        alerts_raised = drift.alerts_raised,
        drift_scores_count = drift.drift_scores.len(),
        error_count = drift.errors.len(),
        "nightly_stage_begin"
    );
    let drift_outcome = process_drift_stage(drift);
    tracing::info!(
        stage = "drift_check",
        items = drift_outcome.items,
        error_count = drift_outcome.error_count,
        status = ?drift_outcome.run.status,
        "nightly_stage_complete"
    );
    report.add_stage(drift_outcome);

    // ── Stage 6: POI Discovery (optional) ──
    if let Some(disc) = discovery {
        tracing::info!(
            stage = "poi_discovery",
            seeds_processed = disc.seeds_processed,
            candidates_found = disc.candidates_found,
            new_pois_inserted = disc.new_pois_inserted,
            contacts_enriched = disc.contacts_enriched,
            tor_sources_used = disc.tor_sources_used,
            error_count = disc.errors.len(),
            "nightly_stage_begin"
        );
        let disc_outcome = process_discovery_stage(disc);
        tracing::info!(
            stage = "poi_discovery",
            items = disc_outcome.items,
            error_count = disc_outcome.error_count,
            status = ?disc_outcome.run.status,
            "nightly_stage_complete"
        );
        report.add_stage(disc_outcome);
    }

    report.finish();

    // ── Pipeline summary ──
    let health = pipeline_health(&report);
    tracing::info!(
        total_items = report.total_items(),
        total_errors = report.total_errors(),
        succeeded_stages = report.succeeded_stages(),
        skipped_stages = report.skipped_stages(),
        overall_success = report.overall_success,
        health_score = health,
        // Estimated memory footprint from crawl data (bytes)
        estimated_bytes = crawl.bytes_fetched,
        "nightly_pipeline_complete"
    );

    report
}

/// Decide whether the pipeline should proceed to the next stage given previous failures.
/// Policy: crawl failure blocks mining (no fresh data), mining failure blocks
/// hypothesis generation, but POI and drift are independent.
pub fn should_proceed(report: &NightlyReport, next_stage: NightlyStage) -> bool {
    match next_stage {
        NightlyStage::Crawl => true,
        NightlyStage::Mining => {
            // Mining needs crawl data
            report.stages.iter().any(|s| {
                s.stage == NightlyStage::Crawl
                    && matches!(s.run.status, JobStatus::Succeeded { .. })
            })
        }
        NightlyStage::HypothesisGeneration => {
            // Hypothesis generation needs mining candidates
            report.stages.iter().any(|s| {
                s.stage == NightlyStage::Mining
                    && matches!(s.run.status, JobStatus::Succeeded { .. })
            })
        }
        NightlyStage::PoiRefresh => true,   // independent
        NightlyStage::DriftCheck => true,   // independent
        NightlyStage::PoiDiscovery => true, // independent; runs after POI refresh if available
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
    #![allow(clippy::disallowed_methods, clippy::field_reassign_with_default)]

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
    fn test_crawl_stage_zero_attempts() {
        let result = CrawlStageResult {
            sources_attempted: 0,
            sources_succeeded: 0,
            sources_failed: 0,
            new_observations: 0,
            changed_pages: 0,
            bytes_fetched: 0,
            errors: vec![],
        };
        let outcome = process_crawl_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 0);
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
    fn test_mining_stage_errors_with_staged_recipes_still_succeeds() {
        let result = MiningStageResult {
            candidates_found: 20,
            candidates_passed_gates: 8,
            hypotheses_generated: 4,
            recipes_staged: 2,
            errors: vec!["partial llm timeout".to_string()],
        };
        let outcome = process_mining_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 2);
        assert_eq!(outcome.error_count, 1);
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

    #[test]
    fn test_drift_stage_no_drift_is_success() {
        let result = DriftCheckStageResult {
            features_checked: 10,
            features_drifted: 0,
            drift_scores: vec![],
            alerts_raised: 0,
            errors: vec![],
        };
        let outcome = process_drift_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert!(outcome.run.notes.contains("0/10 features drifted"));
    }

    // ── Full pipeline ──

    #[test]
    fn test_full_pipeline_all_good() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
                errors: vec![
                    "e1".to_string(),
                    "e2".to_string(),
                    "e3".to_string(),
                    "e4".to_string(),
                    "e5".to_string(),
                ],
            },
        );
        assert!((pipeline_health(&report) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_pipeline_health_mixed() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            None,
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
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0], NightlyStage::Crawl);
        assert_eq!(stages[4], NightlyStage::DriftCheck);
        assert_eq!(stages[5], NightlyStage::PoiDiscovery);
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
            None,
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
            None,
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
            None,
            &good_poi(),
            &good_drift(),
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: NightlyReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.stage_count(), 4);
        assert!(back.overall_success);
        assert_eq!(back.schema_version, "v1");
    }

    // ── B244: summary includes skipped stages ──

    #[test]
    fn test_nightly_summary_includes_skipped_count() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            None,
            &good_poi(),
            &good_drift(),
        );
        let summary = report.summary();
        assert!(
            summary.contains("1 skipped"),
            "summary should mention skipped stages: {}",
            summary
        );
        assert!(summary.contains("PARTIAL FAILURE"));
    }

    #[test]
    fn test_nightly_summary_no_skipped_section_when_zero() {
        // When all stages succeed, the "N skipped" text should be absent
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        let summary = report.summary();
        assert!(
            !summary.contains("skipped"),
            "no skipped text when all succeed: {}",
            summary
        );
    }

    #[test]
    fn test_skipped_stages_count_correct() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            None,
            &empty_poi(), // skipped
            &good_drift(),
        );
        assert_eq!(report.skipped_stages(), 2);
    }

    // ── B245: validate() methods on stage result structs ──

    #[test]
    fn test_crawl_validate_ok() {
        assert!(good_crawl().validate().is_ok());
    }

    #[test]
    fn test_crawl_validate_succeeded_exceeds_attempted() {
        let mut r = good_crawl();
        r.sources_succeeded = 100; // > sources_attempted (50)
        let e = r.validate().unwrap_err();
        assert!(e.contains("sources_succeeded") && e.contains("sources_attempted"));
    }

    #[test]
    fn test_crawl_validate_failed_exceeds_attempted() {
        let mut r = good_crawl();
        r.sources_failed = 100;
        let e = r.validate().unwrap_err();
        assert!(e.contains("sources_failed"));
    }

    #[test]
    fn test_crawl_validate_sum_exceeds_attempted() {
        let r = CrawlStageResult {
            sources_attempted: 10,
            sources_succeeded: 7,
            sources_failed: 5, // 7+5 = 12 > 10
            new_observations: 0,
            changed_pages: 0,
            bytes_fetched: 0,
            errors: vec![],
        };
        let e = r.validate().unwrap_err();
        assert!(e.contains("sources_succeeded") && e.contains("sources_failed"));
    }

    #[test]
    fn test_mining_validate_ok() {
        assert!(good_mining().validate().is_ok());
    }

    #[test]
    fn test_mining_validate_funnel_inversion() {
        let mut r = good_mining();
        r.candidates_passed_gates = r.candidates_found + 1;
        let e = r.validate().unwrap_err();
        assert!(e.contains("candidates_passed_gates"));
    }

    #[test]
    fn test_mining_validate_staged_exceeds_hypotheses() {
        let mut r = good_mining();
        r.recipes_staged = r.hypotheses_generated + 99;
        let e = r.validate().unwrap_err();
        assert!(e.contains("recipes_staged"));
    }

    #[test]
    fn test_poi_validate_ok() {
        assert!(good_poi().validate().is_ok());
    }

    #[test]
    fn test_poi_validate_updated_exceeds_scanned() {
        let mut r = good_poi();
        r.profiles_updated = r.profiles_scanned + 1;
        let e = r.validate().unwrap_err();
        assert!(e.contains("profiles_updated"));
    }

    #[test]
    fn test_drift_validate_ok() {
        assert!(good_drift().validate().is_ok());
    }

    #[test]
    fn test_drift_validate_drifted_exceeds_checked() {
        let mut r = good_drift();
        r.features_drifted = r.features_checked + 1;
        let e = r.validate().unwrap_err();
        assert!(e.contains("features_drifted"));
    }

    #[test]
    fn test_drift_validate_alerts_exceed_drifted() {
        let mut r = good_drift();
        r.alerts_raised = r.features_drifted + 10;
        let e = r.validate().unwrap_err();
        assert!(e.contains("alerts_raised"));
    }

    // ── B266: Golden JSON regression tests ──────────────────────────────────

    /// Serialize a full NightlyReport and verify the JSON has the expected
    /// top-level keys and structural invariants.  This guards against
    /// accidental renames, removed fields, or serde attribute changes.
    #[test]
    fn golden_nightly_report_json_has_required_keys() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        let json = serde_json::to_string(&report).expect("nightly report must serialize");

        // Top-level fields
        assert!(json.contains("\"started_at\""), "missing started_at");
        assert!(json.contains("\"finished_at\""), "missing finished_at");
        assert!(json.contains("\"stages\""), "missing stages");
        assert!(
            json.contains("\"overall_success\""),
            "missing overall_success"
        );
    }

    #[test]
    fn golden_nightly_report_roundtrips_losslessly() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        let json = serde_json::to_string(&report).expect("serialize");
        let back: NightlyReport = serde_json::from_str(&json).expect("deserialize");
        // Key invariants preserved through round-trip
        assert_eq!(back.overall_success, report.overall_success);
        assert_eq!(back.stages.len(), report.stages.len());
        assert_eq!(back.total_items(), report.total_items());
        assert_eq!(back.total_errors(), report.total_errors());
    }

    #[test]
    fn golden_nightly_stage_outcomes_have_run_fields() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        let json = serde_json::to_string(&report).expect("serialize");
        // Each stage outcome must have run with run_id
        assert!(
            json.contains("\"run_id\""),
            "missing run_id in stage outomes"
        );
        assert!(json.contains("\"kind\""), "missing kind in job run");
        assert!(json.contains("\"status\""), "missing status in job run");
    }

    #[test]
    fn golden_nightly_report_failed_pipeline_serializes() {
        let report = run_nightly_pipeline(
            &failed_crawl(),
            &empty_mining(),
            None,
            &good_poi(),
            &empty_drift(),
        );
        assert!(!report.overall_success);
        let json = serde_json::to_string(&report).expect("failed report must still serialize");
        let back: NightlyReport =
            serde_json::from_str(&json).expect("failed report must deserialize");
        assert!(!back.overall_success);
    }

    #[test]
    fn golden_crawl_stage_result_roundtrip() {
        let c = good_crawl();
        let json = serde_json::to_string(&c).expect("serialize CrawlStageResult");
        let back: CrawlStageResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.sources_attempted, c.sources_attempted);
        assert_eq!(back.bytes_fetched, c.bytes_fetched);
        assert_eq!(back.errors, c.errors);
    }

    #[test]
    fn golden_stage_count_equals_four() {
        // NightlyStage::all() now includes PoiDiscovery in addition to the
        // earlier stages, so the full stage registry has 6 entries.
        assert_eq!(NightlyStage::all().len(), 6);
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        // This fixture path omits both the hypothesis and POI discovery stages.
        assert_eq!(report.stages.len(), 4);
        let json = serde_json::to_string(&report).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["stages"].as_array().unwrap().len(), 4);
    }

    // ── B269: End-to-end pipeline integration tests ──────────────────────────
    //
    // These tests exercise the nightly pipeline as an integrated system and
    // verify cross-stage invariants, decision policies, and data flow.

    #[test]
    fn e2e_happy_path_items_and_errors_are_summed_correctly() {
        let crawl = CrawlStageResult {
            sources_attempted: 100,
            sources_succeeded: 95,
            sources_failed: 5,
            new_observations: 400,
            changed_pages: 50,
            bytes_fetched: 10_000_000,
            errors: vec!["err1".to_string(); 5],
        };
        let mining = MiningStageResult {
            candidates_found: 50,
            candidates_passed_gates: 15,
            hypotheses_generated: 10,
            recipes_staged: 5,
            errors: vec![],
        };
        let poi = PoiRefreshStageResult {
            profiles_scanned: 200,
            profiles_updated: 20,
            new_pois_discovered: 4,
            role_changes_detected: 3,
            errors: vec![],
        };
        let drift = DriftCheckStageResult {
            features_checked: 80,
            features_drifted: 6,
            drift_scores: (0..6).map(|i| (format!("f{i}"), 0.1 * i as f64)).collect(),
            alerts_raised: 2,
            errors: vec![],
        };

        let report = run_nightly_pipeline(&crawl, &mining, None, &poi, &drift);
        assert!(report.overall_success);
        assert_eq!(report.total_errors(), 5); // only crawl had errors
                                              // Items: crawl=(400+50)=450, mining=5 (recipes_staged), poi=(20+4)=24, drift=80
        assert_eq!(report.total_items(), 450 + 5 + 24 + 80);
    }

    #[test]
    fn e2e_should_proceed_blocks_mining_on_crawl_failure() {
        // In a guarded execution model, when crawl fails, mining should not run
        let mut report = NightlyReport::new();
        report.add_stage(process_crawl_stage(&failed_crawl()));

        // Verify that should_proceed returns false for mining
        assert!(
            !should_proceed(&report, NightlyStage::Mining),
            "mining must be blocked when crawl fails"
        );
        // But POI and DriftCheck are still allowed
        assert!(should_proceed(&report, NightlyStage::PoiRefresh));
        assert!(should_proceed(&report, NightlyStage::DriftCheck));
    }

    #[test]
    fn e2e_pipeline_health_reflects_all_stage_outcomes() {
        // 4 stages: 2 succeed + 1 skip + 1 fail
        let poi_failure = PoiRefreshStageResult {
            profiles_scanned: 50,
            profiles_updated: 0,
            new_pois_discovered: 0,
            role_changes_detected: 0,
            errors: (0..50).map(|i| format!("err_{i}")).collect(),
        };
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            None,
            &poi_failure, // fail
            &good_drift(),
        );
        // Health = (2 succeed + 0.5 skip + 0 fail) / 4 = 2.5/4 = 0.625
        let health = pipeline_health(&report);
        assert!(
            (health - 0.625).abs() < 0.01,
            "expected health 0.625 but got {health}"
        );
    }

    #[test]
    fn e2e_run_ids_are_unique_across_all_stages() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        let ids: Vec<&str> = report
            .stages
            .iter()
            .map(|s| s.run.run_id.as_str())
            .collect();
        let unique: std::collections::HashSet<&str> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), unique.len(), "all stage run_ids must be unique");
    }

    #[test]
    fn e2e_finished_at_is_set_after_pipeline_completes() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &good_mining(),
            None,
            &good_poi(),
            &good_drift(),
        );
        assert!(
            report.finished_at.is_some(),
            "finished_at must be set after pipeline completes"
        );
        // finished_at must be >= started_at
        assert!(
            report.finished_at.unwrap() >= report.started_at,
            "finished_at must not precede started_at"
        );
    }

    #[test]
    fn e2e_all_stages_skipped_overall_success_is_false() {
        let report = run_nightly_pipeline(
            &good_crawl(),
            &empty_mining(), // skipped
            None,
            &empty_poi(),   // skipped
            &empty_drift(), // skipped
        );
        // A skipped stage is not a success — at least mining and poi are skipped
        assert!(
            !report.overall_success,
            "all-skipped should not be overall success"
        );
        assert_eq!(report.skipped_stages(), 3);
    }

    #[test]
    fn e2e_full_pipeline_with_maximum_realistic_error_counts() {
        let crawl = CrawlStageResult {
            sources_attempted: 1000,
            sources_succeeded: 0, // everything failed
            sources_failed: 1000,
            new_observations: 0,
            changed_pages: 0,
            bytes_fetched: 0,
            errors: (0..1000)
                .map(|i| format!("timeout on source {i}"))
                .collect(),
        };
        let report = run_nightly_pipeline(&crawl, &good_mining(), None, &good_poi(), &good_drift());
        assert!(!report.overall_success);
        assert_eq!(report.failed_stages().len(), 1);
        assert_eq!(report.failed_stages()[0].stage, NightlyStage::Crawl);
        // The overall pipeline can still summarize cleanly
        let summary = report.summary();
        assert!(summary.contains("PARTIAL FAILURE"));
    }
}
