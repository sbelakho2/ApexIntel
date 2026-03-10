//! Self-improvement pipeline — weekly feedback loop orchestration.
//!
//! This module orchestrates the self-improvement stages that run weekly
//! after the promotion/deprecation cycle:
//!
//! 1. **Source Scoring** — re-rank crawl sources by yield, freshness, novelty.
//! 2. **Cross-Domain Mining** — discover synergistic multi-signal combinations.
//! 3. **Outcome Tracking** — match predictions against observed outcomes.
//! 4. **Meta-Learning** — extract insights from promotion/deprecation history.
//! 5. **Recommendation Generation** — produce actionable collection strategy updates.
//!
//! All functions are pure orchestration logic — no I/O.
//! External callers inject data; this module sequences stages and produces
//! a `SelfImprovementReport`.

use crate::scheduler::{JobKind, JobRun, JobStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Stage results
// ────────────────────────────────────────────

/// Data captured during the source scoring stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceScoringStageResult {
    pub sources_scored: u64,
    pub sources_upgraded: u64,
    pub sources_downgraded: u64,
    pub coverage_gaps_found: u64,
    pub errors: Vec<String>,
}

impl SourceScoringStageResult {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .sources_upgraded
            .saturating_add(self.sources_downgraded)
            > self.sources_scored
        {
            return Err(format!(
                "upgraded ({}) + downgraded ({}) > scored ({})",
                self.sources_upgraded, self.sources_downgraded, self.sources_scored
            ));
        }
        Ok(())
    }
}

/// Data captured during the cross-domain mining stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossDomainStageResult {
    pub pairs_evaluated: u64,
    pub synergies_found: u64,
    pub new_candidates_generated: u64,
    pub errors: Vec<String>,
}

impl CrossDomainStageResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.synergies_found > self.pairs_evaluated {
            return Err(format!(
                "synergies_found ({}) > pairs_evaluated ({})",
                self.synergies_found, self.pairs_evaluated
            ));
        }
        if self.new_candidates_generated > self.synergies_found {
            return Err(format!(
                "new_candidates_generated ({}) > synergies_found ({})",
                self.new_candidates_generated, self.synergies_found
            ));
        }
        Ok(())
    }
}

/// Data captured during the outcome tracking stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeTrackingStageResult {
    pub predictions_checked: u64,
    pub predictions_confirmed: u64,
    pub predictions_expired: u64,
    pub recipes_with_updated_accuracy: u64,
    pub overconfident_recipes: u64,
    pub poor_accuracy_recipes: u64,
    pub errors: Vec<String>,
}

impl OutcomeTrackingStageResult {
    pub fn validate(&self) -> Result<(), String> {
        let resolved = self.predictions_confirmed + self.predictions_expired;
        if resolved > self.predictions_checked {
            return Err(format!(
                "confirmed ({}) + expired ({}) > checked ({})",
                self.predictions_confirmed, self.predictions_expired, self.predictions_checked
            ));
        }
        Ok(())
    }
}

/// Data captured during the meta-learning stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaLearningStageResult {
    pub recipes_analysed: u64,
    pub threshold_adjustments_suggested: u64,
    pub high_value_signal_types_found: u64,
    pub collection_recommendations_generated: u64,
    pub errors: Vec<String>,
}

impl MetaLearningStageResult {
    pub fn validate(&self) -> Result<(), String> {
        Ok(()) // All fields are independent — nothing to cross-validate.
    }
}

// ────────────────────────────────────────────
// Pipeline stages
// ────────────────────────────────────────────

/// Identifies one stage in the self-improvement pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelfImprovementStage {
    SourceScoring,
    CrossDomainMining,
    OutcomeTracking,
    MetaLearning,
}

impl SelfImprovementStage {
    pub fn all() -> &'static [SelfImprovementStage] {
        &[
            SelfImprovementStage::SourceScoring,
            SelfImprovementStage::CrossDomainMining,
            SelfImprovementStage::OutcomeTracking,
            SelfImprovementStage::MetaLearning,
        ]
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::SourceScoring => "source_scoring",
            Self::CrossDomainMining => "cross_domain_mining",
            Self::OutcomeTracking => "outcome_tracking",
            Self::MetaLearning => "meta_learning",
        }
    }
}

/// Outcome record per stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementStageOutcome {
    pub stage: SelfImprovementStage,
    pub run: JobRun,
    pub items: u64,
    pub error_count: u64,
}

/// Aggregated report from the self-improvement pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementReport {
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stages: Vec<ImprovementStageOutcome>,
    pub overall_success: bool,
}

impl SelfImprovementReport {
    pub fn new() -> Self {
        Self {
            started_at: Utc::now(),
            finished_at: None,
            stages: Vec::new(),
            overall_success: true,
        }
    }

    pub fn add_stage(&mut self, outcome: ImprovementStageOutcome) {
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
}

// ────────────────────────────────────────────
// Stage processors
// ────────────────────────────────────────────

/// Batch-size guard — same rationale as nightly (B286).
const MAX_IMPROVEMENT_BATCH_SIZE: u64 = 50_000;

pub fn process_source_scoring_stage(result: &SourceScoringStageResult) -> ImprovementStageOutcome {
    let mut run = JobRun::new(JobKind::SourceScoring);
    run.start();

    if result.sources_scored > MAX_IMPROVEMENT_BATCH_SIZE {
        run.status = JobStatus::Failed {
            error: format!(
                "sources_scored {} exceeds batch limit {}",
                result.sources_scored, MAX_IMPROVEMENT_BATCH_SIZE
            ),
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::SourceScoring,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if let Err(e) = result.validate() {
        run.status = JobStatus::Failed {
            error: e,
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::SourceScoring,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if result.sources_scored == 0 {
        run.status = JobStatus::Skipped {
            reason: "No sources to score".into(),
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::SourceScoring,
            run,
            items: 0,
            error_count: 0,
        };
    }

    run.succeed(
        result.sources_scored,
        &format!(
            "Scored {} sources: {} upgraded, {} downgraded, {} coverage gaps",
            result.sources_scored,
            result.sources_upgraded,
            result.sources_downgraded,
            result.coverage_gaps_found
        ),
    );
    ImprovementStageOutcome {
        stage: SelfImprovementStage::SourceScoring,
        run,
        items: result.sources_scored,
        error_count: result.errors.len() as u64,
    }
}

pub fn process_cross_domain_stage(result: &CrossDomainStageResult) -> ImprovementStageOutcome {
    let mut run = JobRun::new(JobKind::CrossDomainMining);
    run.start();

    if result.pairs_evaluated > MAX_IMPROVEMENT_BATCH_SIZE {
        run.status = JobStatus::Failed {
            error: format!(
                "pairs_evaluated {} exceeds batch limit {}",
                result.pairs_evaluated, MAX_IMPROVEMENT_BATCH_SIZE
            ),
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::CrossDomainMining,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if let Err(e) = result.validate() {
        run.status = JobStatus::Failed {
            error: e,
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::CrossDomainMining,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if result.pairs_evaluated == 0 {
        run.status = JobStatus::Skipped {
            reason: "No signal pairs to evaluate".into(),
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::CrossDomainMining,
            run,
            items: 0,
            error_count: 0,
        };
    }

    run.succeed(
        result.pairs_evaluated,
        &format!(
            "Evaluated {} pairs: {} synergies found, {} new candidates",
            result.pairs_evaluated, result.synergies_found, result.new_candidates_generated
        ),
    );
    ImprovementStageOutcome {
        stage: SelfImprovementStage::CrossDomainMining,
        run,
        items: result.pairs_evaluated,
        error_count: result.errors.len() as u64,
    }
}

pub fn process_outcome_tracking_stage(
    result: &OutcomeTrackingStageResult,
) -> ImprovementStageOutcome {
    let mut run = JobRun::new(JobKind::OutcomeTracking);
    run.start();

    if result.predictions_checked > MAX_IMPROVEMENT_BATCH_SIZE {
        run.status = JobStatus::Failed {
            error: format!(
                "predictions_checked {} exceeds batch limit {}",
                result.predictions_checked, MAX_IMPROVEMENT_BATCH_SIZE
            ),
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::OutcomeTracking,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if let Err(e) = result.validate() {
        run.status = JobStatus::Failed {
            error: e,
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::OutcomeTracking,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if result.predictions_checked == 0 {
        run.status = JobStatus::Skipped {
            reason: "No predictions to track".into(),
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::OutcomeTracking,
            run,
            items: 0,
            error_count: 0,
        };
    }

    run.succeed(
        result.predictions_checked,
        &format!(
            "Checked {} predictions: {} confirmed, {} expired, {} overconfident, {} poor accuracy",
            result.predictions_checked,
            result.predictions_confirmed,
            result.predictions_expired,
            result.overconfident_recipes,
            result.poor_accuracy_recipes
        ),
    );
    ImprovementStageOutcome {
        stage: SelfImprovementStage::OutcomeTracking,
        run,
        items: result.predictions_checked,
        error_count: result.errors.len() as u64,
    }
}

pub fn process_meta_learning_stage(result: &MetaLearningStageResult) -> ImprovementStageOutcome {
    let mut run = JobRun::new(JobKind::Custom("meta_learning".into()));
    run.start();

    if let Err(e) = result.validate() {
        run.status = JobStatus::Failed {
            error: e,
            duration_ms: 0,
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::MetaLearning,
            run,
            items: 0,
            error_count: 1,
        };
    }

    if result.recipes_analysed == 0 {
        run.status = JobStatus::Skipped {
            reason: "No recipe history for meta-learning".into(),
        };
        return ImprovementStageOutcome {
            stage: SelfImprovementStage::MetaLearning,
            run,
            items: 0,
            error_count: 0,
        };
    }

    run.succeed(
        result.recipes_analysed,
        &format!(
        "Analysed {} recipes: {} threshold adjustments, {} high-value signals, {} recommendations",
        result.recipes_analysed, result.threshold_adjustments_suggested,
        result.high_value_signal_types_found, result.collection_recommendations_generated
    ),
    );
    ImprovementStageOutcome {
        stage: SelfImprovementStage::MetaLearning,
        run,
        items: result.recipes_analysed,
        error_count: result.errors.len() as u64,
    }
}

// ────────────────────────────────────────────
// Pipeline orchestration
// ────────────────────────────────────────────

/// Run the weekly self-improvement pipeline from pre-computed stage results.
///
/// All parameters are optional — if a stage has no data to process, pass `None`
/// and it will be recorded as Skipped.
#[tracing::instrument(skip(source_scoring, cross_domain, outcome_tracking, meta_learning))]
pub fn run_self_improvement_pipeline(
    source_scoring: Option<&SourceScoringStageResult>,
    cross_domain: Option<&CrossDomainStageResult>,
    outcome_tracking: Option<&OutcomeTrackingStageResult>,
    meta_learning: Option<&MetaLearningStageResult>,
) -> SelfImprovementReport {
    let mut report = SelfImprovementReport::new();

    // ── Stage 1: Source Scoring ──
    if let Some(ss) = source_scoring {
        tracing::info!(
            stage = "source_scoring",
            sources_scored = ss.sources_scored,
            "self_improvement_stage_begin"
        );
        let outcome = process_source_scoring_stage(ss);
        tracing::info!(
            stage = "source_scoring",
            items = outcome.items,
            status = ?outcome.run.status,
            "self_improvement_stage_complete"
        );
        report.add_stage(outcome);
    }

    // ── Stage 2: Cross-Domain Mining ──
    if let Some(cd) = cross_domain {
        tracing::info!(
            stage = "cross_domain_mining",
            pairs_evaluated = cd.pairs_evaluated,
            "self_improvement_stage_begin"
        );
        let outcome = process_cross_domain_stage(cd);
        tracing::info!(
            stage = "cross_domain_mining",
            items = outcome.items,
            status = ?outcome.run.status,
            "self_improvement_stage_complete"
        );
        report.add_stage(outcome);
    }

    // ── Stage 3: Outcome Tracking ──
    if let Some(ot) = outcome_tracking {
        tracing::info!(
            stage = "outcome_tracking",
            predictions_checked = ot.predictions_checked,
            "self_improvement_stage_begin"
        );
        let outcome = process_outcome_tracking_stage(ot);
        tracing::info!(
            stage = "outcome_tracking",
            items = outcome.items,
            status = ?outcome.run.status,
            "self_improvement_stage_complete"
        );
        report.add_stage(outcome);
    }

    // ── Stage 4: Meta-Learning ──
    if let Some(ml) = meta_learning {
        tracing::info!(
            stage = "meta_learning",
            recipes_analysed = ml.recipes_analysed,
            "self_improvement_stage_begin"
        );
        let outcome = process_meta_learning_stage(ml);
        tracing::info!(
            stage = "meta_learning",
            items = outcome.items,
            status = ?outcome.run.status,
            "self_improvement_stage_complete"
        );
        report.add_stage(outcome);
    }

    report.finish();

    tracing::info!(
        total_items = report.total_items(),
        total_errors = report.total_errors(),
        overall_success = report.overall_success,
        stages_run = report.stages.len(),
        "self_improvement_pipeline_complete"
    );

    report
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn good_source_scoring() -> SourceScoringStageResult {
        SourceScoringStageResult {
            sources_scored: 50,
            sources_upgraded: 8,
            sources_downgraded: 5,
            coverage_gaps_found: 3,
            errors: vec![],
        }
    }

    fn good_cross_domain() -> CrossDomainStageResult {
        CrossDomainStageResult {
            pairs_evaluated: 120,
            synergies_found: 5,
            new_candidates_generated: 3,
            errors: vec![],
        }
    }

    fn good_outcome_tracking() -> OutcomeTrackingStageResult {
        OutcomeTrackingStageResult {
            predictions_checked: 200,
            predictions_confirmed: 140,
            predictions_expired: 40,
            recipes_with_updated_accuracy: 25,
            overconfident_recipes: 2,
            poor_accuracy_recipes: 1,
            errors: vec![],
        }
    }

    fn good_meta_learning() -> MetaLearningStageResult {
        MetaLearningStageResult {
            recipes_analysed: 80,
            threshold_adjustments_suggested: 2,
            high_value_signal_types_found: 4,
            collection_recommendations_generated: 7,
            errors: vec![],
        }
    }

    #[test]
    fn test_full_pipeline_success() {
        let report = run_self_improvement_pipeline(
            Some(&good_source_scoring()),
            Some(&good_cross_domain()),
            Some(&good_outcome_tracking()),
            Some(&good_meta_learning()),
        );
        assert!(report.overall_success);
        assert_eq!(report.stages.len(), 4);
        assert!(report.total_items() > 0);
        assert_eq!(report.total_errors(), 0);
        assert!(report.finished_at.is_some());
    }

    #[test]
    fn test_pipeline_partial() {
        let report = run_self_improvement_pipeline(
            Some(&good_source_scoring()),
            None, // skip cross-domain
            None, // skip outcome tracking
            Some(&good_meta_learning()),
        );
        assert!(report.overall_success);
        assert_eq!(report.stages.len(), 2);
    }

    #[test]
    fn test_pipeline_empty() {
        let report = run_self_improvement_pipeline(None, None, None, None);
        assert!(report.overall_success);
        assert_eq!(report.stages.len(), 0);
    }

    #[test]
    fn test_source_scoring_validation_failure() {
        let bad = SourceScoringStageResult {
            sources_scored: 10,
            sources_upgraded: 8,
            sources_downgraded: 8,
            coverage_gaps_found: 0,
            errors: vec![],
        };
        let outcome = process_source_scoring_stage(&bad);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_source_scoring_batch_guard() {
        let huge = SourceScoringStageResult {
            sources_scored: 100_000,
            sources_upgraded: 0,
            sources_downgraded: 0,
            coverage_gaps_found: 0,
            errors: vec![],
        };
        let outcome = process_source_scoring_stage(&huge);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_source_scoring_skipped() {
        let empty = SourceScoringStageResult {
            sources_scored: 0,
            sources_upgraded: 0,
            sources_downgraded: 0,
            coverage_gaps_found: 0,
            errors: vec![],
        };
        let outcome = process_source_scoring_stage(&empty);
        assert!(matches!(outcome.run.status, JobStatus::Skipped { .. }));
    }

    #[test]
    fn test_cross_domain_validation() {
        let bad = CrossDomainStageResult {
            pairs_evaluated: 5,
            synergies_found: 10, // more than pairs
            new_candidates_generated: 0,
            errors: vec![],
        };
        let outcome = process_cross_domain_stage(&bad);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_outcome_tracking_validation() {
        let bad = OutcomeTrackingStageResult {
            predictions_checked: 10,
            predictions_confirmed: 8,
            predictions_expired: 8, // 8+8 > 10
            recipes_with_updated_accuracy: 0,
            overconfident_recipes: 0,
            poor_accuracy_recipes: 0,
            errors: vec![],
        };
        let outcome = process_outcome_tracking_stage(&bad);
        assert!(matches!(outcome.run.status, JobStatus::Failed { .. }));
    }

    #[test]
    fn test_outcome_tracking_success() {
        let result = good_outcome_tracking();
        let outcome = process_outcome_tracking_stage(&result);
        assert!(matches!(outcome.run.status, JobStatus::Succeeded { .. }));
        assert_eq!(outcome.items, 200);
    }

    #[test]
    fn test_meta_learning_empty() {
        let empty = MetaLearningStageResult {
            recipes_analysed: 0,
            threshold_adjustments_suggested: 0,
            high_value_signal_types_found: 0,
            collection_recommendations_generated: 0,
            errors: vec![],
        };
        let outcome = process_meta_learning_stage(&empty);
        assert!(matches!(outcome.run.status, JobStatus::Skipped { .. }));
    }

    #[test]
    fn test_stage_enum() {
        let all = SelfImprovementStage::all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].as_str(), "source_scoring");
        assert_eq!(all[3].as_str(), "meta_learning");
    }

    #[test]
    fn test_report_methods() {
        let mut report = SelfImprovementReport::new();
        assert_eq!(report.total_items(), 0);
        assert_eq!(report.total_errors(), 0);
        assert!(report.overall_success);
        report.finish();
        assert!(report.finished_at.is_some());
    }
}
