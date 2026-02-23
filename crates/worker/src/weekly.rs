//! Weekly pipeline — promotion board, recipe deprecation, strategy memo generation.
//!
//! Pure orchestration logic. External callers provide the data;
//! this module sequences stages, applies promotion/deprecation policies,
//! and produces a WeeklyReport.

use crate::scheduler::{JobKind, JobRun, JobStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Stage inputs
// ────────────────────────────────────────────

/// A staged recipe awaiting promotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedRecipe {
    pub recipe_id: String,
    pub staged_at: DateTime<Utc>,
    pub weeks_in_staging: u32,
    pub precision: f64,
    pub recall: f64,
    pub false_positive_rate: f64,
    pub alerts_fired: u64,
    pub true_positives: u64,
}

/// A production recipe being monitored for degradation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionRecipe {
    pub recipe_id: String,
    pub promoted_at: DateTime<Utc>,
    pub weeks_in_production: u32,
    pub precision_history: Vec<f64>,
    pub recall_history: Vec<f64>,
    pub false_positive_rate: f64,
    pub alerts_fired_total: u64,
}

/// Promotion policy thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionPolicy {
    pub min_weeks_staged: u32,
    pub min_precision: f64,
    pub min_recall: f64,
    pub max_false_positive_rate: f64,
    pub min_alerts_fired: u64,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            min_weeks_staged: 4,
            min_precision: 0.85,
            min_recall: 0.3,
            max_false_positive_rate: 0.05,
            min_alerts_fired: 3,
        }
    }
}

/// Deprecation policy thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationPolicy {
    pub precision_threshold: f64,
    pub min_weeks_declining: u32,
    pub max_false_positive_rate: f64,
    pub inactivity_weeks: u32,
}

impl Default for DeprecationPolicy {
    fn default() -> Self {
        Self {
            precision_threshold: 0.5,
            min_weeks_declining: 3,
            max_false_positive_rate: 0.15,
            inactivity_weeks: 8,
        }
    }
}

// ────────────────────────────────────────────
// Weekly stages
// ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeeklyStage {
    PromotionBoard,
    RecipeDeprecation,
    StrategyMemo,
}

impl WeeklyStage {
    pub fn all() -> &'static [WeeklyStage] {
        &[
            WeeklyStage::PromotionBoard,
            WeeklyStage::RecipeDeprecation,
            WeeklyStage::StrategyMemo,
        ]
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::PromotionBoard => "promotion_board",
            Self::RecipeDeprecation => "recipe_deprecation",
            Self::StrategyMemo => "strategy_memo",
        }
    }

    pub fn to_job_kind(&self) -> JobKind {
        match self {
            Self::PromotionBoard => JobKind::PromotionBoard,
            Self::RecipeDeprecation => JobKind::RecipeDeprecation,
            Self::StrategyMemo => JobKind::StrategyMemo,
        }
    }
}

// ────────────────────────────────────────────
// Promotion logic (pure)
// ────────────────────────────────────────────

/// Decision for a single staged recipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PromotionDecision {
    Promote { reason: String },
    Keep { reason: String },
    Reject { reason: String },
}

/// Evaluate whether a staged recipe should be promoted.
pub fn evaluate_promotion(recipe: &StagedRecipe, policy: &PromotionPolicy) -> PromotionDecision {
    if recipe.weeks_in_staging < policy.min_weeks_staged {
        return PromotionDecision::Keep {
            reason: format!(
                "only {}/{} weeks staged",
                recipe.weeks_in_staging, policy.min_weeks_staged
            ),
        };
    }

    if recipe.alerts_fired < policy.min_alerts_fired {
        return PromotionDecision::Keep {
            reason: format!(
                "only {}/{} alerts fired — insufficient data",
                recipe.alerts_fired, policy.min_alerts_fired
            ),
        };
    }

    if recipe.precision < policy.min_precision {
        return PromotionDecision::Reject {
            reason: format!(
                "precision {:.2} < {:.2}",
                recipe.precision, policy.min_precision
            ),
        };
    }

    if recipe.recall < policy.min_recall {
        return PromotionDecision::Reject {
            reason: format!(
                "recall {:.2} < {:.2}",
                recipe.recall, policy.min_recall
            ),
        };
    }

    if recipe.false_positive_rate > policy.max_false_positive_rate {
        return PromotionDecision::Reject {
            reason: format!(
                "FPR {:.3} > {:.3}",
                recipe.false_positive_rate, policy.max_false_positive_rate
            ),
        };
    }

    PromotionDecision::Promote {
        reason: format!(
            "precision={:.2}, recall={:.2}, FPR={:.3}, {} alerts over {} weeks",
            recipe.precision,
            recipe.recall,
            recipe.false_positive_rate,
            recipe.alerts_fired,
            recipe.weeks_in_staging,
        ),
    }
}

/// Run the promotion board for all staged recipes.
pub fn run_promotion_board(
    staged: &[StagedRecipe],
    policy: &PromotionPolicy,
) -> PromotionBoardResult {
    let mut promoted = Vec::new();
    let mut kept = Vec::new();
    let mut rejected = Vec::new();

    for recipe in staged {
        match evaluate_promotion(recipe, policy) {
            PromotionDecision::Promote { reason } => {
                promoted.push((recipe.recipe_id.clone(), reason));
            }
            PromotionDecision::Keep { reason } => {
                kept.push((recipe.recipe_id.clone(), reason));
            }
            PromotionDecision::Reject { reason } => {
                rejected.push((recipe.recipe_id.clone(), reason));
            }
        }
    }

    PromotionBoardResult {
        promoted,
        kept,
        rejected,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionBoardResult {
    pub promoted: Vec<(String, String)>,
    pub kept: Vec<(String, String)>,
    pub rejected: Vec<(String, String)>,
}

// ────────────────────────────────────────────
// Deprecation logic (pure)
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DeprecationDecision {
    Deprecate { reason: String },
    Keep,
}

/// Evaluate whether a production recipe should be deprecated.
pub fn evaluate_deprecation(
    recipe: &ProductionRecipe,
    policy: &DeprecationPolicy,
) -> DeprecationDecision {
    // Inactivity: no alerts for too long
    if recipe.alerts_fired_total == 0 && recipe.weeks_in_production >= policy.inactivity_weeks {
        return DeprecationDecision::Deprecate {
            reason: format!(
                "inactive for {} weeks (threshold: {})",
                recipe.weeks_in_production, policy.inactivity_weeks
            ),
        };
    }

    // High FPR
    if recipe.false_positive_rate > policy.max_false_positive_rate {
        return DeprecationDecision::Deprecate {
            reason: format!(
                "FPR {:.3} > {:.3}",
                recipe.false_positive_rate, policy.max_false_positive_rate
            ),
        };
    }

    // Precision declining for N consecutive weeks
    if is_declining(&recipe.precision_history, policy.min_weeks_declining) {
        let recent = recipe.precision_history.last().copied().unwrap_or(0.0);
        if recent < policy.precision_threshold {
            return DeprecationDecision::Deprecate {
                reason: format!(
                    "precision declining for {}+ weeks, now {:.2} < {:.2}",
                    policy.min_weeks_declining, recent, policy.precision_threshold
                ),
            };
        }
    }

    DeprecationDecision::Keep
}

/// Check if a metric has been declining for at least `n` consecutive weeks.
pub fn is_declining(history: &[f64], n: u32) -> bool {
    if history.len() < n as usize + 1 {
        return false;
    }
    let tail = &history[history.len() - (n as usize + 1)..];
    tail.windows(2).all(|w| w[1] < w[0])
}

/// Run deprecation check for all production recipes.
pub fn run_deprecation_check(
    recipes: &[ProductionRecipe],
    policy: &DeprecationPolicy,
) -> DeprecationResult {
    let mut deprecated = Vec::new();
    let mut kept = Vec::new();

    for recipe in recipes {
        match evaluate_deprecation(recipe, policy) {
            DeprecationDecision::Deprecate { reason } => {
                deprecated.push((recipe.recipe_id.clone(), reason));
            }
            DeprecationDecision::Keep => {
                kept.push(recipe.recipe_id.clone());
            }
        }
    }

    DeprecationResult { deprecated, kept }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationResult {
    pub deprecated: Vec<(String, String)>,
    pub kept: Vec<String>,
}

// ────────────────────────────────────────────
// Strategy memo scaffolding
// ────────────────────────────────────────────

/// Inputs needed to generate a weekly strategy memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoInputs {
    pub top_warnings: Vec<MemoWarning>,
    pub new_recipes_staged: u32,
    pub recipes_promoted: u32,
    pub recipes_deprecated: u32,
    pub pipeline_health_pct: f64,
    pub top_drift_features: Vec<(String, f64)>,
    pub poi_changes: Vec<PoiChange>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoWarning {
    pub id: String,
    pub headline: String,
    pub impact: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiChange {
    pub person_name: String,
    pub change_type: String,
    pub details: String,
}

/// Build the markdown structure for a weekly strategy memo.
pub fn build_memo_structure(inputs: &MemoInputs) -> MemoStructure {
    let sections = vec![
        MemoSection {
            title: "Executive Summary".to_string(),
            content: build_executive_summary(inputs),
        },
        MemoSection {
            title: "Top Warnings".to_string(),
            content: build_warnings_section(&inputs.top_warnings),
        },
        MemoSection {
            title: "People of Interest Changes".to_string(),
            content: build_poi_section(&inputs.poi_changes),
        },
        MemoSection {
            title: "Recipe Pipeline".to_string(),
            content: build_recipe_section(inputs),
        },
        MemoSection {
            title: "System Health".to_string(),
            content: build_health_section(inputs),
        },
    ];

    MemoStructure {
        title: format!(
            "Weekly Intelligence Brief — {} to {}",
            inputs.period_start.format("%Y-%m-%d"),
            inputs.period_end.format("%Y-%m-%d"),
        ),
        sections,
        generated_at: Utc::now(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoStructure {
    pub title: String,
    pub sections: Vec<MemoSection>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoSection {
    pub title: String,
    pub content: String,
}

fn build_executive_summary(inputs: &MemoInputs) -> String {
    let warning_count = inputs.top_warnings.len();
    let high_conf = inputs
        .top_warnings
        .iter()
        .filter(|w| w.confidence > 0.8)
        .count();
    format!(
        "This week produced **{} actionable warnings** ({} high-confidence). \
         {} new recipes were staged, {} promoted to production, and {} deprecated. \
         System pipeline health: **{:.0}%**.",
        warning_count,
        high_conf,
        inputs.new_recipes_staged,
        inputs.recipes_promoted,
        inputs.recipes_deprecated,
        inputs.pipeline_health_pct * 100.0,
    )
}

fn build_warnings_section(warnings: &[MemoWarning]) -> String {
    if warnings.is_empty() {
        return "No actionable warnings this period.".to_string();
    }
    let mut out = String::new();
    for (i, w) in warnings.iter().enumerate() {
        out.push_str(&format!(
            "{}. **{}** (confidence: {:.0}%)\n   Impact: {}\n\n",
            i + 1,
            w.headline,
            w.confidence * 100.0,
            w.impact,
        ));
    }
    out
}

fn build_poi_section(changes: &[PoiChange]) -> String {
    if changes.is_empty() {
        return "No significant POI changes detected.".to_string();
    }
    let mut out = String::new();
    for c in changes {
        out.push_str(&format!(
            "- **{}**: {} — {}\n",
            c.person_name, c.change_type, c.details
        ));
    }
    out
}

fn build_recipe_section(inputs: &MemoInputs) -> String {
    format!(
        "- Staged: {}\n- Promoted: {}\n- Deprecated: {}",
        inputs.new_recipes_staged, inputs.recipes_promoted, inputs.recipes_deprecated,
    )
}

fn build_health_section(inputs: &MemoInputs) -> String {
    let mut out = format!("Pipeline health: {:.0}%\n", inputs.pipeline_health_pct * 100.0);
    if !inputs.top_drift_features.is_empty() {
        out.push_str("\nDrifted features:\n");
        for (feat, score) in &inputs.top_drift_features {
            out.push_str(&format!("- {}: KL={:.4}\n", feat, score));
        }
    }
    out
}

// ────────────────────────────────────────────
// Full weekly pipeline
// ────────────────────────────────────────────

/// Stage outcomes for the weekly pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyStageOutcome {
    pub stage: WeeklyStage,
    pub run: JobRun,
    pub details: String,
}

/// Full weekly report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyReport {
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stages: Vec<WeeklyStageOutcome>,
    pub promotion_result: Option<PromotionBoardResult>,
    pub deprecation_result: Option<DeprecationResult>,
    pub memo: Option<MemoStructure>,
    pub overall_success: bool,
}

impl WeeklyReport {
    pub fn new() -> Self {
        Self {
            started_at: Utc::now(),
            finished_at: None,
            stages: Vec::new(),
            promotion_result: None,
            deprecation_result: None,
            memo: None,
            overall_success: true,
        }
    }

    pub fn finish(&mut self) {
        self.finished_at = Some(Utc::now());
    }

    pub fn summary(&self) -> String {
        let succeeded = self
            .stages
            .iter()
            .filter(|s| matches!(s.run.status, JobStatus::Succeeded { .. }))
            .count();
        let total = self.stages.len();
        let status = if self.overall_success {
            "SUCCESS"
        } else {
            "PARTIAL FAILURE"
        };
        format!(
            "Weekly [{}]: {}/{} stages OK",
            status, succeeded, total
        )
    }
}

impl Default for WeeklyReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the full weekly pipeline from pre-computed inputs.
pub fn run_weekly_pipeline(
    staged_recipes: &[StagedRecipe],
    production_recipes: &[ProductionRecipe],
    memo_inputs: &MemoInputs,
    promotion_policy: &PromotionPolicy,
    deprecation_policy: &DeprecationPolicy,
) -> WeeklyReport {
    let mut report = WeeklyReport::new();

    // Stage 1: Promotion board
    let promo_result = run_promotion_board(staged_recipes, promotion_policy);
    let mut promo_run = JobRun::new(JobKind::PromotionBoard);
    promo_run.start();
    let promo_items = promo_result.promoted.len() as u64;
    promo_run.succeed(
        promo_items,
        &format!(
            "{} promoted, {} kept, {} rejected",
            promo_result.promoted.len(),
            promo_result.kept.len(),
            promo_result.rejected.len(),
        ),
    );
    report.stages.push(WeeklyStageOutcome {
        stage: WeeklyStage::PromotionBoard,
        run: promo_run,
        details: format!("{} promoted", promo_result.promoted.len()),
    });
    report.promotion_result = Some(promo_result);

    // Stage 2: Deprecation
    let dep_result = run_deprecation_check(production_recipes, deprecation_policy);
    let mut dep_run = JobRun::new(JobKind::RecipeDeprecation);
    dep_run.start();
    let dep_items = dep_result.deprecated.len() as u64;
    dep_run.succeed(
        dep_items,
        &format!(
            "{} deprecated, {} kept",
            dep_result.deprecated.len(),
            dep_result.kept.len(),
        ),
    );
    report.stages.push(WeeklyStageOutcome {
        stage: WeeklyStage::RecipeDeprecation,
        run: dep_run,
        details: format!("{} deprecated", dep_result.deprecated.len()),
    });
    report.deprecation_result = Some(dep_result);

    // Stage 3: Strategy memo
    let memo = build_memo_structure(memo_inputs);
    let mut memo_run = JobRun::new(JobKind::StrategyMemo);
    memo_run.start();
    memo_run.succeed(
        memo.sections.len() as u64,
        &format!("{} sections generated", memo.sections.len()),
    );
    report.stages.push(WeeklyStageOutcome {
        stage: WeeklyStage::StrategyMemo,
        run: memo_run,
        details: format!("{} sections", memo.sections.len()),
    });
    report.memo = Some(memo);

    report.finish();
    report
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
    }

    fn sample_staged_good() -> StagedRecipe {
        StagedRecipe {
            recipe_id: "R001".to_string(),
            staged_at: utc(2026, 1, 1, 0, 0, 0),
            weeks_in_staging: 6,
            precision: 0.92,
            recall: 0.45,
            false_positive_rate: 0.02,
            alerts_fired: 10,
            true_positives: 9,
        }
    }

    fn sample_staged_too_young() -> StagedRecipe {
        StagedRecipe {
            recipe_id: "R002".to_string(),
            staged_at: utc(2026, 2, 10, 0, 0, 0),
            weeks_in_staging: 2,
            precision: 0.95,
            recall: 0.50,
            false_positive_rate: 0.01,
            alerts_fired: 5,
            true_positives: 5,
        }
    }

    fn sample_staged_low_precision() -> StagedRecipe {
        StagedRecipe {
            recipe_id: "R003".to_string(),
            staged_at: utc(2026, 1, 1, 0, 0, 0),
            weeks_in_staging: 5,
            precision: 0.40,
            recall: 0.60,
            false_positive_rate: 0.08,
            alerts_fired: 15,
            true_positives: 6,
        }
    }

    fn sample_staged_few_alerts() -> StagedRecipe {
        StagedRecipe {
            recipe_id: "R004".to_string(),
            staged_at: utc(2026, 1, 1, 0, 0, 0),
            weeks_in_staging: 5,
            precision: 1.0,
            recall: 1.0,
            false_positive_rate: 0.0,
            alerts_fired: 1,
            true_positives: 1,
        }
    }

    fn sample_prod_healthy() -> ProductionRecipe {
        ProductionRecipe {
            recipe_id: "P001".to_string(),
            promoted_at: utc(2025, 6, 1, 0, 0, 0),
            weeks_in_production: 30,
            precision_history: vec![0.90, 0.88, 0.91, 0.89, 0.90],
            recall_history: vec![0.45, 0.47, 0.44, 0.46, 0.45],
            false_positive_rate: 0.03,
            alerts_fired_total: 150,
        }
    }

    fn sample_prod_declining() -> ProductionRecipe {
        ProductionRecipe {
            recipe_id: "P002".to_string(),
            promoted_at: utc(2025, 6, 1, 0, 0, 0),
            weeks_in_production: 20,
            precision_history: vec![0.85, 0.70, 0.55, 0.42],
            recall_history: vec![0.40, 0.35, 0.30, 0.25],
            false_positive_rate: 0.08,
            alerts_fired_total: 80,
        }
    }

    fn sample_prod_inactive() -> ProductionRecipe {
        ProductionRecipe {
            recipe_id: "P003".to_string(),
            promoted_at: utc(2025, 1, 1, 0, 0, 0),
            weeks_in_production: 10,
            precision_history: vec![],
            recall_history: vec![],
            false_positive_rate: 0.0,
            alerts_fired_total: 0,
        }
    }

    fn sample_prod_high_fpr() -> ProductionRecipe {
        ProductionRecipe {
            recipe_id: "P004".to_string(),
            promoted_at: utc(2025, 6, 1, 0, 0, 0),
            weeks_in_production: 15,
            precision_history: vec![0.60, 0.55, 0.50],
            recall_history: vec![0.50, 0.48, 0.45],
            false_positive_rate: 0.20,
            alerts_fired_total: 100,
        }
    }

    fn sample_memo_inputs() -> MemoInputs {
        MemoInputs {
            top_warnings: vec![
                MemoWarning {
                    id: "W001".to_string(),
                    headline: "Supplier X late filing detected".to_string(),
                    impact: "High".to_string(),
                    confidence: 0.92,
                },
                MemoWarning {
                    id: "W002".to_string(),
                    headline: "Competitor Y expanding in Morocco".to_string(),
                    impact: "Medium".to_string(),
                    confidence: 0.75,
                },
            ],
            new_recipes_staged: 3,
            recipes_promoted: 1,
            recipes_deprecated: 2,
            pipeline_health_pct: 0.875,
            top_drift_features: vec![("commodity_vol".to_string(), 0.18)],
            poi_changes: vec![PoiChange {
                person_name: "Ahmed Ben Ali".to_string(),
                change_type: "Role change".to_string(),
                details: "New CPO at OEM Corp".to_string(),
            }],
            period_start: utc(2026, 2, 16, 0, 0, 0),
            period_end: utc(2026, 2, 23, 0, 0, 0),
        }
    }

    // ── Promotion logic ──

    #[test]
    fn test_promote_good_recipe() {
        let recipe = sample_staged_good();
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Promote { .. }));
    }

    #[test]
    fn test_keep_young_recipe() {
        let recipe = sample_staged_too_young();
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Keep { .. }));
        if let PromotionDecision::Keep { reason } = decision {
            assert!(reason.contains("weeks staged"));
        }
    }

    #[test]
    fn test_reject_low_precision() {
        let recipe = sample_staged_low_precision();
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Reject { .. }));
        if let PromotionDecision::Reject { reason } = decision {
            assert!(reason.contains("precision"));
        }
    }

    #[test]
    fn test_keep_few_alerts() {
        let recipe = sample_staged_few_alerts();
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Keep { .. }));
        if let PromotionDecision::Keep { reason } = decision {
            assert!(reason.contains("alerts"));
        }
    }

    #[test]
    fn test_reject_high_fpr() {
        let mut recipe = sample_staged_good();
        recipe.false_positive_rate = 0.10;
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Reject { .. }));
        if let PromotionDecision::Reject { reason } = decision {
            assert!(reason.contains("FPR"));
        }
    }

    #[test]
    fn test_reject_low_recall() {
        let mut recipe = sample_staged_good();
        recipe.recall = 0.1;
        let policy = PromotionPolicy::default();
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Reject { .. }));
        if let PromotionDecision::Reject { reason } = decision {
            assert!(reason.contains("recall"));
        }
    }

    #[test]
    fn test_promotion_board_mixed() {
        let staged = vec![
            sample_staged_good(),
            sample_staged_too_young(),
            sample_staged_low_precision(),
        ];
        let policy = PromotionPolicy::default();
        let result = run_promotion_board(&staged, &policy);
        assert_eq!(result.promoted.len(), 1);
        assert_eq!(result.promoted[0].0, "R001");
        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0].0, "R002");
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].0, "R003");
    }

    #[test]
    fn test_promotion_board_empty() {
        let result = run_promotion_board(&[], &PromotionPolicy::default());
        assert!(result.promoted.is_empty());
        assert!(result.kept.is_empty());
        assert!(result.rejected.is_empty());
    }

    #[test]
    fn test_custom_promotion_policy() {
        let recipe = sample_staged_too_young(); // 2 weeks
        let policy = PromotionPolicy {
            min_weeks_staged: 1, // lower threshold
            min_precision: 0.80,
            min_recall: 0.20,
            max_false_positive_rate: 0.10,
            min_alerts_fired: 3,
        };
        let decision = evaluate_promotion(&recipe, &policy);
        assert!(matches!(decision, PromotionDecision::Promote { .. }));
    }

    // ── Deprecation logic ──

    #[test]
    fn test_keep_healthy_recipe() {
        let recipe = sample_prod_healthy();
        let policy = DeprecationPolicy::default();
        let decision = evaluate_deprecation(&recipe, &policy);
        assert_eq!(decision, DeprecationDecision::Keep);
    }

    #[test]
    fn test_deprecate_declining() {
        let recipe = sample_prod_declining();
        let policy = DeprecationPolicy::default();
        let decision = evaluate_deprecation(&recipe, &policy);
        assert!(matches!(decision, DeprecationDecision::Deprecate { .. }));
        if let DeprecationDecision::Deprecate { reason } = decision {
            assert!(reason.contains("precision declining"));
        }
    }

    #[test]
    fn test_deprecate_inactive() {
        let recipe = sample_prod_inactive();
        let policy = DeprecationPolicy::default();
        let decision = evaluate_deprecation(&recipe, &policy);
        assert!(matches!(decision, DeprecationDecision::Deprecate { .. }));
        if let DeprecationDecision::Deprecate { reason } = decision {
            assert!(reason.contains("inactive"));
        }
    }

    #[test]
    fn test_deprecate_high_fpr() {
        let recipe = sample_prod_high_fpr();
        let policy = DeprecationPolicy::default();
        let decision = evaluate_deprecation(&recipe, &policy);
        assert!(matches!(decision, DeprecationDecision::Deprecate { .. }));
        if let DeprecationDecision::Deprecate { reason } = decision {
            assert!(reason.contains("FPR"));
        }
    }

    #[test]
    fn test_deprecation_check_mixed() {
        let recipes = vec![
            sample_prod_healthy(),
            sample_prod_declining(),
            sample_prod_inactive(),
        ];
        let policy = DeprecationPolicy::default();
        let result = run_deprecation_check(&recipes, &policy);
        assert_eq!(result.deprecated.len(), 2);
        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0], "P001");
    }

    #[test]
    fn test_deprecation_check_empty() {
        let result = run_deprecation_check(&[], &DeprecationPolicy::default());
        assert!(result.deprecated.is_empty());
        assert!(result.kept.is_empty());
    }

    // ── is_declining ──

    #[test]
    fn test_is_declining_true() {
        let history = vec![0.90, 0.85, 0.75, 0.60];
        assert!(is_declining(&history, 3));
    }

    #[test]
    fn test_is_declining_false_not_enough() {
        let history = vec![0.90, 0.85];
        assert!(!is_declining(&history, 3));
    }

    #[test]
    fn test_is_declining_false_recovery() {
        let history = vec![0.90, 0.80, 0.70, 0.85]; // recovered
        assert!(!is_declining(&history, 3));
    }

    #[test]
    fn test_is_declining_flat() {
        let history = vec![0.80, 0.80, 0.80, 0.80];
        assert!(!is_declining(&history, 3));
    }

    // ── Memo generation ──

    #[test]
    fn test_build_memo_structure() {
        let inputs = sample_memo_inputs();
        let memo = build_memo_structure(&inputs);
        assert!(memo.title.contains("2026-02-16"));
        assert!(memo.title.contains("2026-02-23"));
        assert_eq!(memo.sections.len(), 5);
    }

    #[test]
    fn test_executive_summary_content() {
        let inputs = sample_memo_inputs();
        let summary = build_executive_summary(&inputs);
        assert!(summary.contains("2 actionable warnings"));
        assert!(summary.contains("1 high-confidence"));
        assert!(summary.contains("3 new recipes"));
        assert!(summary.contains("88%")); // 0.875 * 100 rounds to 88
    }

    #[test]
    fn test_warnings_section_content() {
        let inputs = sample_memo_inputs();
        let section = build_warnings_section(&inputs.top_warnings);
        assert!(section.contains("Supplier X"));
        assert!(section.contains("92%"));
        assert!(section.contains("Competitor Y"));
    }

    #[test]
    fn test_warnings_section_empty() {
        let section = build_warnings_section(&[]);
        assert!(section.contains("No actionable"));
    }

    #[test]
    fn test_poi_section_content() {
        let inputs = sample_memo_inputs();
        let section = build_poi_section(&inputs.poi_changes);
        assert!(section.contains("Ahmed Ben Ali"));
        assert!(section.contains("New CPO"));
    }

    #[test]
    fn test_poi_section_empty() {
        let section = build_poi_section(&[]);
        assert!(section.contains("No significant"));
    }

    #[test]
    fn test_health_section_drift() {
        let inputs = sample_memo_inputs();
        let section = build_health_section(&inputs);
        assert!(section.contains("commodity_vol"));
        assert!(section.contains("KL="));
    }

    // ── WeeklyStage ──

    #[test]
    fn test_weekly_stage_all() {
        assert_eq!(WeeklyStage::all().len(), 3);
    }

    #[test]
    fn test_weekly_stage_as_str() {
        assert_eq!(WeeklyStage::PromotionBoard.as_str(), "promotion_board");
        assert_eq!(WeeklyStage::RecipeDeprecation.as_str(), "recipe_deprecation");
        assert_eq!(WeeklyStage::StrategyMemo.as_str(), "strategy_memo");
    }

    #[test]
    fn test_weekly_stage_to_job_kind() {
        assert_eq!(
            WeeklyStage::PromotionBoard.to_job_kind(),
            JobKind::PromotionBoard
        );
        assert_eq!(
            WeeklyStage::RecipeDeprecation.to_job_kind(),
            JobKind::RecipeDeprecation
        );
        assert_eq!(
            WeeklyStage::StrategyMemo.to_job_kind(),
            JobKind::StrategyMemo
        );
    }

    // ── Full weekly pipeline ──

    #[test]
    fn test_full_weekly_pipeline() {
        let staged = vec![sample_staged_good(), sample_staged_too_young()];
        let production = vec![sample_prod_healthy(), sample_prod_declining()];
        let memo_inputs = sample_memo_inputs();
        let promo_policy = PromotionPolicy::default();
        let dep_policy = DeprecationPolicy::default();

        let report = run_weekly_pipeline(
            &staged,
            &production,
            &memo_inputs,
            &promo_policy,
            &dep_policy,
        );

        assert!(report.overall_success);
        assert_eq!(report.stages.len(), 3);
        assert!(report.finished_at.is_some());

        // Promotion: R001 promoted, R002 kept
        let promo = report.promotion_result.as_ref().unwrap();
        assert_eq!(promo.promoted.len(), 1);
        assert_eq!(promo.kept.len(), 1);

        // Deprecation: P002 deprecated, P001 kept
        let dep = report.deprecation_result.as_ref().unwrap();
        assert_eq!(dep.deprecated.len(), 1);
        assert_eq!(dep.kept.len(), 1);

        // Memo generated
        let memo = report.memo.as_ref().unwrap();
        assert_eq!(memo.sections.len(), 5);
    }

    #[test]
    fn test_weekly_pipeline_empty() {
        let memo_inputs = MemoInputs {
            top_warnings: vec![],
            new_recipes_staged: 0,
            recipes_promoted: 0,
            recipes_deprecated: 0,
            pipeline_health_pct: 1.0,
            top_drift_features: vec![],
            poi_changes: vec![],
            period_start: utc(2026, 2, 16, 0, 0, 0),
            period_end: utc(2026, 2, 23, 0, 0, 0),
        };

        let report = run_weekly_pipeline(
            &[],
            &[],
            &memo_inputs,
            &PromotionPolicy::default(),
            &DeprecationPolicy::default(),
        );

        assert!(report.overall_success);
        assert_eq!(report.stages.len(), 3);
        assert!(report.promotion_result.as_ref().unwrap().promoted.is_empty());
    }

    #[test]
    fn test_weekly_report_summary() {
        let memo_inputs = sample_memo_inputs();
        let report = run_weekly_pipeline(
            &[sample_staged_good()],
            &[sample_prod_healthy()],
            &memo_inputs,
            &PromotionPolicy::default(),
            &DeprecationPolicy::default(),
        );
        let summary = report.summary();
        assert!(summary.contains("SUCCESS"));
        assert!(summary.contains("3/3"));
    }

    // ── Serialization ──

    #[test]
    fn test_promotion_policy_serialization() {
        let policy = PromotionPolicy::default();
        let json = serde_json::to_string(&policy).unwrap();
        let back: PromotionPolicy = serde_json::from_str(&json).unwrap();
        assert!((back.min_precision - 0.85).abs() < 0.01);
    }

    #[test]
    fn test_deprecation_policy_serialization() {
        let policy = DeprecationPolicy::default();
        let json = serde_json::to_string(&policy).unwrap();
        let back: DeprecationPolicy = serde_json::from_str(&json).unwrap();
        assert!((back.precision_threshold - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_staged_recipe_serialization() {
        let recipe = sample_staged_good();
        let json = serde_json::to_string(&recipe).unwrap();
        let back: StagedRecipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back.recipe_id, "R001");
    }

    #[test]
    fn test_weekly_report_serialization() {
        let memo_inputs = sample_memo_inputs();
        let report = run_weekly_pipeline(
            &[sample_staged_good()],
            &[sample_prod_healthy()],
            &memo_inputs,
            &PromotionPolicy::default(),
            &DeprecationPolicy::default(),
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: WeeklyReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.stages.len(), 3);
        assert!(back.overall_success);
    }
}
