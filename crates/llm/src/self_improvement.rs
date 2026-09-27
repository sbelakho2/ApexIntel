//! LLM self-improvement and fine-tuning data preparation.
//!
//! This module implements a robust self-improvement loop for the LLM inference
//! layer. The core strategies are:
//!
//! 1. **Output capture**: wrap LLM calls and record (prompt, response, score) triples.
//! 2. **LLM-as-judge critique**: use the same model to score and critique its own past
//!    responses, identifying hallucinations, poor reasoning, or low-confidence outputs.
//! 3. **Training example mining**: extract high-quality prompt/response pairs as JSONL
//!    training examples suitable for supervised fine-tuning (SFT).
//! 4. **Prompt improvement proposals**: LLM proposes rewritten system prompts and few-shot
//!    examples that would have produced better outputs.
//! 5. **Hypothesis-driven improvements**: identify patterns in low-scoring outputs and
//!    generate hypotheses about what causes failures.
//!
//! # Truth semantics
//!
//! Every stage returns a [`StageResult`]. A failed model call or an unparseable
//! response becomes `Failed` with the underlying error retained — it is never
//! rewritten into "found no improvements". Likewise the average critique score
//! is a [`Measurement`]: no captures to critique is `NotMeasured`, a batch where
//! every critique failed is `Unavailable`, and only real scores are `Measured`.
//!
//! # Usage
//! ```rust,ignore
//! let mut loop_runner = SelfImprovementLoop::new(llm.clone(), config);
//! loop_runner.record_output(capture).await;
//! let report = loop_runner.run_improvement_cycle().await?;
//! let examples = report.export_training_examples();
//! ```

use anyhow::Result;
use apex_core::measurement::{FailureReason, Measurement};
use apex_core::stage::{FailureKind, StageResult, StageStatus, StructuredFailure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::LlmClient;

const STAGE_CRITIQUE: &str = "critique";
const STAGE_PROMPT_IMPROVEMENTS: &str = "prompt_improvements";
const STAGE_FAILURE_HYPOTHESES: &str = "failure_hypotheses";

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the self-improvement loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementConfig {
    /// Minimum quality score to include in training set [0, 1].
    pub min_quality_score: f64,
    /// Maximum size of the output history buffer.
    pub max_history_size: usize,
    /// Number of outputs to critique per improvement cycle.
    pub critique_batch_size: usize,
    /// Whether to generate prompt improvement proposals.
    pub generate_prompt_proposals: bool,
    /// Whether to generate failure hypothesis analysis.
    pub analyse_failures: bool,
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            min_quality_score: 0.7,
            max_history_size: 500,
            critique_batch_size: 20,
            generate_prompt_proposals: true,
            analyse_failures: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Data types
// ─────────────────────────────────────────────────────────────────────────────

/// Category of an LLM call, for tracking by task type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskCategory {
    InsightGeneration,
    RecipeHypothesis,
    PoiProfiling,
    EvidenceChain,
    GapAnalysis,
    BackgroundBrief,
    WeeklyMemo,
    Other(String),
}

impl TaskCategory {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InsightGeneration => "insight_generation",
            Self::RecipeHypothesis => "recipe_hypothesis",
            Self::PoiProfiling => "poi_profiling",
            Self::EvidenceChain => "evidence_chain",
            Self::GapAnalysis => "gap_analysis",
            Self::BackgroundBrief => "background_brief",
            Self::WeeklyMemo => "weekly_memo",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// A captured LLM input/output pair with optional quality metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputCapture {
    pub id: String,
    pub category: TaskCategory,
    pub system_prompt: String,
    pub user_prompt: String,
    pub response: String,
    /// Downstream quality metric if available (e.g. from human feedback or eval).
    pub quality_score: Option<f64>,
    /// Whether this response was used (as opposed to rejected / retried).
    pub was_used: bool,
    /// Optional LLM-assigned critique score from the improvement loop.
    pub critique_score: Option<f64>,
    /// LLM critique text.
    pub critique: Option<String>,
    pub captured_at: DateTime<Utc>,
    pub latency_ms: u64,
}

impl OutputCapture {
    pub fn new(
        category: TaskCategory,
        system_prompt: impl Into<String>,
        user_prompt: impl Into<String>,
        response: impl Into<String>,
        latency_ms: u64,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            category,
            system_prompt: system_prompt.into(),
            user_prompt: user_prompt.into(),
            response: response.into(),
            quality_score: None,
            was_used: true,
            critique_score: None,
            critique: None,
            captured_at: Utc::now(),
            latency_ms,
        }
    }

    /// The best available quality score for this capture, if any was measured.
    ///
    /// `None` means "not measured" and must never be treated as a low score
    /// (or as a passing score) by callers.
    pub fn measured_quality(&self) -> Option<f64> {
        self.critique_score.or(self.quality_score)
    }
}

/// A training example in Alpaca/chat format, ready for SFT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingExample {
    pub instruction: String,
    pub input: String,
    pub output: String,
    pub category: String,
    pub quality_score: f64,
    pub source_id: String,
}

/// A proposal for improving a system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImprovement {
    pub category: String,
    pub original_system_prompt: String,
    pub improved_system_prompt: String,
    pub rationale: String,
    pub few_shot_examples: Vec<(String, String)>,
    pub confidence: f64,
}

/// A failure hypothesis identifying a pattern in poor outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureHypothesis {
    pub pattern_description: String,
    pub affected_categories: Vec<String>,
    pub observed_failure_count: usize,
    pub root_cause_hypothesis: String,
    pub proposed_fix: String,
    pub confidence: f64,
}

/// Counts describing the critique stage, carried with its [`StageResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritiqueSummary {
    /// Captures that were critiqued successfully.
    pub captures_critiqued: usize,
    /// Captures whose critique failed.
    pub critiques_failed: usize,
}

/// The result of one improvement cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementCycleReport {
    pub cycle_id: String,
    pub ran_at: DateTime<Utc>,
    pub captures_analysed: usize,
    pub examples_qualifying: usize,
    pub prompt_improvements: StageResult<Vec<PromptImprovement>>,
    pub failure_hypotheses: StageResult<Vec<FailureHypothesis>>,
    pub critique: StageResult<CritiqueSummary>,
    /// Mean critique score over the captures that were actually scored.
    ///
    /// `NotMeasured` when the batch was empty, `Unavailable` when every
    /// critique attempt failed, `Measured` only for real scores.
    pub avg_critique_score: Measurement<f64>,
}

impl ImprovementCycleReport {
    /// Export all qualifying captures as training examples.
    ///
    /// Only captures with a *measured* score at or above `min_quality` are
    /// exported; an unmeasured capture is not training data.
    pub fn export_training_examples(
        captures: &[OutputCapture],
        min_quality: f64,
    ) -> Vec<TrainingExample> {
        captures
            .iter()
            .filter_map(|c| {
                let score = c.measured_quality()?;
                if score >= min_quality && c.was_used {
                    Some(TrainingExample {
                        instruction: c.system_prompt.clone(),
                        input: c.user_prompt.clone(),
                        output: c.response.clone(),
                        category: c.category.as_str().to_string(),
                        quality_score: score,
                        source_id: c.id.clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Serialize training examples to JSONL string.
    pub fn to_jsonl(examples: &[TrainingExample]) -> String {
        examples
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Number of prompt improvements produced, if the stage produced any.
    pub fn prompt_improvements_count(&self) -> usize {
        self.prompt_improvements.value_ref().map_or(0, Vec::len)
    }

    /// Number of failure hypotheses produced, if the stage produced any.
    pub fn failure_hypotheses_count(&self) -> usize {
        self.failure_hypotheses.value_ref().map_or(0, Vec::len)
    }

    /// Every structured stage failure recorded in this report.
    pub fn stage_failures(&self) -> Vec<&StructuredFailure> {
        [
            self.critique.failure_ref(),
            self.prompt_improvements.failure_ref(),
            self.failure_hypotheses.failure_ref(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// Failures from stages that terminally failed (`Failed`).
    ///
    /// Unlike [`Self::stage_failures`], this excludes `Partial` stages whose
    /// value was still produced, so a single failed critique in a batch does
    /// not read as a whole-cycle failure.
    pub fn failed_stages(&self) -> Vec<&StructuredFailure> {
        [
            (self.critique.status, self.critique.failure_ref()),
            (
                self.prompt_improvements.status,
                self.prompt_improvements.failure_ref(),
            ),
            (
                self.failure_hypotheses.status,
                self.failure_hypotheses.failure_ref(),
            ),
        ]
        .into_iter()
        .filter(|(status, _)| matches!(status, StageStatus::Failed))
        .filter_map(|(_, failure)| failure)
        .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Self-improvement loop
// ─────────────────────────────────────────────────────────────────────────────

/// Orchestrates the LLM self-improvement cycle.
pub struct SelfImprovementLoop {
    llm: Arc<dyn LlmClient>,
    config: SelfImprovementConfig,
    history: Vec<OutputCapture>,
}

impl SelfImprovementLoop {
    pub fn new(llm: Arc<dyn LlmClient>, config: SelfImprovementConfig) -> Self {
        Self {
            llm,
            config,
            history: Vec::new(),
        }
    }

    /// Record a captured LLM output into the history buffer.
    pub fn record(&mut self, capture: OutputCapture) {
        if self.history.len() >= self.config.max_history_size {
            self.history.remove(0); // FIFO eviction
        }
        self.history.push(capture);
    }

    /// Run one improvement cycle: critique batch, mine training examples,
    /// generate prompt improvements and failure hypotheses.
    ///
    /// Stage failures are captured inside the returned report rather than
    /// aborting the cycle, but they are never rewritten as success or as
    /// "nothing found": each stage carries its own [`StageResult`].
    pub async fn run_cycle(&mut self) -> Result<ImprovementCycleReport> {
        info!(
            history_size = self.history.len(),
            "Starting self-improvement cycle"
        );

        // Select uncritiqued captures for this batch
        let batch: Vec<usize> = self
            .history
            .iter()
            .enumerate()
            .filter(|(_, c)| c.critique_score.is_none())
            .take(self.config.critique_batch_size)
            .map(|(i, _)| i)
            .collect();

        // Critique each capture
        let mut total_score = 0.0;
        let mut scored = 0usize;
        let mut critiques_failed = 0usize;
        let mut first_failure: Option<StructuredFailure> = None;
        for &idx in &batch {
            match self.critique_capture(idx).await {
                Ok((score, text)) => {
                    self.history[idx].critique_score = Some(score);
                    self.history[idx].critique = Some(text);
                    total_score += score;
                    scored += 1;
                }
                Err(failure) => {
                    critiques_failed += 1;
                    warn!(
                        stage = %failure.stage,
                        kind = %failure.kind.as_str(),
                        error = %failure.message,
                        capture_id = %self.history[idx].id,
                        "Critique failed"
                    );
                    if first_failure.is_none() {
                        first_failure = Some(failure);
                    }
                }
            }
        }

        let (critique, avg_critique_score) = Self::summarize_critique(
            batch.len(),
            scored,
            critiques_failed,
            total_score,
            first_failure,
        );

        // Same predicate as `export_training_examples` (measured score >=
        // threshold and `was_used`), so the reported count always matches the
        // exported dataset size.
        let examples_qualifying = self.export_training_examples().len();

        // Prompt improvement proposals
        let prompt_improvements = if self.config.generate_prompt_proposals {
            self.generate_prompt_improvements().await
        } else {
            StageResult::empty()
        };

        // Failure analysis
        let failure_hypotheses = if self.config.analyse_failures {
            self.analyse_failures().await
        } else {
            StageResult::empty()
        };

        let report = ImprovementCycleReport {
            cycle_id: uuid::Uuid::new_v4().to_string(),
            ran_at: Utc::now(),
            captures_analysed: batch.len(),
            examples_qualifying,
            prompt_improvements,
            failure_hypotheses,
            critique,
            avg_critique_score,
        };

        info!(
            cycle_id = %report.cycle_id,
            analysed = report.captures_analysed,
            qualifying = report.examples_qualifying,
            critique_status = report.critique.status.as_str(),
            avg_score = %report.avg_critique_score.display_fixed(3),
            prompt_improvement_status = report.prompt_improvements.status.as_str(),
            failure_hypothesis_status = report.failure_hypotheses.status.as_str(),
            "Self-improvement cycle complete"
        );

        Ok(report)
    }

    /// Export training examples from current history.
    pub fn export_training_examples(&self) -> Vec<TrainingExample> {
        ImprovementCycleReport::export_training_examples(
            &self.history,
            self.config.min_quality_score,
        )
    }

    // ── Internal methods ──────────────────────────────────────────────────────

    /// Collapse the per-capture critique outcomes into an honest stage result.
    fn summarize_critique(
        batch_len: usize,
        scored: usize,
        critiques_failed: usize,
        total_score: f64,
        first_failure: Option<StructuredFailure>,
    ) -> (StageResult<CritiqueSummary>, Measurement<f64>) {
        if batch_len == 0 {
            return (StageResult::empty(), Measurement::not_measured());
        }

        let summary = CritiqueSummary {
            captures_critiqued: scored,
            critiques_failed,
        };

        if scored == 0 {
            // Every critique in the batch failed: this is not "no
            // improvements found", it is a failed measurement.
            let failure = first_failure.unwrap_or_else(|| {
                StructuredFailure::new(
                    STAGE_CRITIQUE,
                    FailureKind::Internal,
                    "no critique in the batch succeeded",
                )
            });
            let reason = FailureReason::new("critique_failed", failure.message.clone());
            return (
                StageResult::failed(failure),
                Measurement::unavailable(reason),
            );
        }

        let avg = total_score / scored as f64;
        match first_failure {
            Some(failure) => (
                StageResult::partial(summary, failure),
                Measurement::measured(avg),
            ),
            None => (StageResult::success(summary), Measurement::measured(avg)),
        }
    }

    async fn critique_capture(&self, idx: usize) -> Result<(f64, String), StructuredFailure> {
        let capture = &self.history[idx];

        let system =
            "You are a strict LLM output quality reviewer for an OSINT intelligence system. \
            Rate the following response on: (1) Factual accuracy/coherence [0-1], \
            (2) Task adherence [0-1], (3) Conciseness [0-1], (4) OSINT relevance [0-1]. \
            Return JSON: { \"score\": float (0-1 average), \"critique\": \"brief rationale\" }";

        let user = format!(
            "=== SYSTEM PROMPT ===\n{}\n\n=== USER PROMPT ===\n{}\n\n=== RESPONSE ===\n{}",
            crate::truncate_utf8(&capture.system_prompt, 400),
            crate::truncate_utf8(&capture.user_prompt, 600),
            crate::truncate_utf8(&capture.response, 800),
        );

        let json = self
            .llm
            .generate_json(system, &user)
            .await
            .map_err(|error| {
                StructuredFailure::new(
                    STAGE_CRITIQUE,
                    FailureKind::ModelCallFailed,
                    format!("critique LLM call failed: {error}"),
                )
            })?;

        let v = parse_model_json(&json, STAGE_CRITIQUE)?;

        let Some(score) = v.get("score").and_then(|s| s.as_f64()) else {
            return Err(StructuredFailure::new(
                STAGE_CRITIQUE,
                FailureKind::MissingField,
                format!(
                    "critique response is missing a numeric 'score' (raw: {})",
                    crate::truncate_utf8(&json, 200)
                ),
            ));
        };
        let score = score.clamp(0.0, 1.0);
        let text = v
            .get("critique")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        debug!(capture_id=%capture.id, score=%score, "Critique complete");
        Ok((score, text))
    }

    async fn generate_prompt_improvements(&self) -> StageResult<Vec<PromptImprovement>> {
        // Group low-scoring captures by category. Only *measured* low scores
        // qualify: an unmeasured capture is not evidence of low quality.
        let low_quality: Vec<&OutputCapture> = self
            .history
            .iter()
            .filter(|c| c.measured_quality().is_some_and(|score| score < 0.5))
            .take(5)
            .collect();

        if low_quality.is_empty() {
            return StageResult::empty();
        }

        let examples_text = low_quality
            .iter()
            .map(|c| {
                let score = c
                    .measured_quality()
                    .map_or_else(|| "unmeasured".to_string(), |score| format!("{score:.2}"));
                format!(
                    "[{}] Score {}\nSystem: {}...\nResponse: {}...",
                    c.category.as_str(),
                    score,
                    crate::truncate_utf8(&c.system_prompt, 200),
                    crate::truncate_utf8(&c.response, 300),
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let system = "You are a prompt engineering expert. Analyse these low-quality LLM outputs \
            and propose an improved system prompt that would produce better results. \
            Return JSON: { \"improved_system_prompt\": str, \"rationale\": str, \"confidence\": float }";

        let json = match self.llm.generate_json(system, &examples_text).await {
            Ok(json) => json,
            Err(error) => {
                return StageResult::failed(StructuredFailure::new(
                    STAGE_PROMPT_IMPROVEMENTS,
                    FailureKind::ModelCallFailed,
                    format!("prompt-improvement LLM call failed: {error}"),
                ))
            }
        };

        let v = match parse_model_json(&json, STAGE_PROMPT_IMPROVEMENTS) {
            Ok(value) => value,
            Err(failure) => return StageResult::failed(failure),
        };

        let improved_system_prompt = v
            .get("improved_system_prompt")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if improved_system_prompt.is_empty() {
            return StageResult::failed(StructuredFailure::new(
                STAGE_PROMPT_IMPROVEMENTS,
                FailureKind::MissingField,
                format!(
                    "response is missing a non-empty 'improved_system_prompt' (raw: {})",
                    crate::truncate_utf8(&json, 200)
                ),
            ));
        }

        let Some(confidence) = v.get("confidence").and_then(|s| s.as_f64()) else {
            return StageResult::failed(StructuredFailure::new(
                STAGE_PROMPT_IMPROVEMENTS,
                FailureKind::MissingField,
                format!(
                    "response is missing a numeric 'confidence' (raw: {})",
                    crate::truncate_utf8(&json, 200)
                ),
            ));
        };

        let rationale = v
            .get("rationale")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let improvement = PromptImprovement {
            category: low_quality[0].category.as_str().to_string(),
            original_system_prompt: low_quality[0].system_prompt.clone(),
            improved_system_prompt,
            rationale,
            few_shot_examples: vec![],
            confidence: confidence.clamp(0.0, 1.0),
        };

        if improvement.rationale.is_empty() {
            // The proposal itself is usable, but the rationale is missing:
            // keep the value and record the partial failure.
            return StageResult::partial(
                vec![improvement],
                StructuredFailure::new(
                    STAGE_PROMPT_IMPROVEMENTS,
                    FailureKind::MissingField,
                    "response is missing 'rationale'; proposal kept without rationale",
                ),
            );
        }

        StageResult::success(vec![improvement])
    }

    async fn analyse_failures(&self) -> StageResult<Vec<FailureHypothesis>> {
        // Only measured poor scores count as observed failures.
        let failures: Vec<&OutputCapture> = self
            .history
            .iter()
            .filter(|c| c.measured_quality().is_some_and(|score| score < 0.4))
            .take(10)
            .collect();

        if failures.len() < 3 {
            return StageResult::empty(); // Not enough data to hypothesise
        }

        let failure_text = failures
            .iter()
            .map(|c| {
                let score = c
                    .measured_quality()
                    .map_or_else(|| "unmeasured".to_string(), |score| format!("{score:.2}"));
                format!(
                    "Category: {} | Score: {} | Critique: {}",
                    c.category.as_str(),
                    score,
                    c.critique.as_deref().unwrap_or("no critique"),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You are a machine learning failure analyst. Identify the root cause pattern \
            behind these LLM output failures for an OSINT system. \
            Return JSON: { \"pattern\": str, \"root_cause\": str, \"proposed_fix\": str, \"confidence\": float }";

        let json = match self.llm.generate_json(system, &failure_text).await {
            Ok(json) => json,
            Err(error) => {
                return StageResult::failed(StructuredFailure::new(
                    STAGE_FAILURE_HYPOTHESES,
                    FailureKind::ModelCallFailed,
                    format!("failure-analysis LLM call failed: {error}"),
                ))
            }
        };

        let v = match parse_model_json(&json, STAGE_FAILURE_HYPOTHESES) {
            Ok(value) => value,
            Err(failure) => return StageResult::failed(failure),
        };

        let pattern_description = v
            .get("pattern")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let root_cause_hypothesis = v
            .get("root_cause")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let proposed_fix = v
            .get("proposed_fix")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let confidence = v.get("confidence").and_then(|s| s.as_f64());

        let mut missing = Vec::new();
        if pattern_description.is_empty() {
            missing.push("pattern");
        }
        if root_cause_hypothesis.is_empty() {
            missing.push("root_cause");
        }
        if proposed_fix.is_empty() {
            missing.push("proposed_fix");
        }
        if confidence.is_none() {
            missing.push("confidence");
        }
        if !missing.is_empty() {
            return StageResult::failed(StructuredFailure::new(
                STAGE_FAILURE_HYPOTHESES,
                FailureKind::MissingField,
                format!(
                    "failure analysis is missing required field(s): {} (raw: {})",
                    missing.join(", "),
                    crate::truncate_utf8(&json, 200)
                ),
            ));
        }
        let confidence = match confidence {
            Some(value) => value.clamp(0.0, 1.0),
            // Guarded by the `missing` check above; kept explicit so no
            // default confidence can ever be fabricated here.
            None => {
                return StageResult::failed(StructuredFailure::new(
                    STAGE_FAILURE_HYPOTHESES,
                    FailureKind::MissingField,
                    "failure analysis is missing a numeric 'confidence'",
                ))
            }
        };

        let hypothesis = FailureHypothesis {
            pattern_description,
            affected_categories: failures
                .iter()
                .map(|c| c.category.as_str().to_string())
                .collect(),
            observed_failure_count: failures.len(),
            root_cause_hypothesis,
            proposed_fix,
            confidence,
        };

        StageResult::success(vec![hypothesis])
    }
}

/// Parse a model JSON response, retaining the parse error on failure.
fn parse_model_json(raw: &str, stage: &str) -> Result<serde_json::Value, StructuredFailure> {
    serde_json::from_str(raw).map_err(|error| {
        StructuredFailure::new(
            stage,
            FailureKind::InvalidResponse,
            format!(
                "invalid JSON from model: {error} (raw: {})",
                crate::truncate_utf8(raw, 200)
            ),
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sample_capture(score: f64) -> OutputCapture {
        let mut c = OutputCapture::new(
            TaskCategory::InsightGeneration,
            "You are an OSINT analyst.",
            "Summarise the threat from supplier X.",
            "Supplier X presents medium risk due to geopolitical exposure.",
            150,
        );
        c.quality_score = Some(score);
        c.critique_score = Some(score);
        c
    }

    /// Stub that returns a fixed JSON body for every call.
    struct FixedLlm {
        body: Result<String, String>,
    }

    #[async_trait::async_trait]
    impl LlmClient for FixedLlm {
        async fn generate_json(&self, _sys: &str, _usr: &str) -> Result<String> {
            self.body.clone().map_err(|error| anyhow::anyhow!(error))
        }
        async fn generate_text(&self, _sys: &str, _usr: &str) -> Result<String> {
            Ok(String::new())
        }
    }

    /// Stub whose JSON responses are scripted per call; extra calls reuse the
    /// last scripted response.
    struct ScriptedLlm {
        responses: Vec<Result<String, String>>,
        calls: AtomicUsize,
    }

    impl ScriptedLlm {
        fn new(responses: Vec<Result<String, String>>) -> Self {
            Self {
                responses,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for ScriptedLlm {
        async fn generate_json(&self, _sys: &str, _usr: &str) -> Result<String> {
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            let scripted = self
                .responses
                .get(index)
                .or_else(|| self.responses.last())
                .cloned()
                .unwrap_or_else(|| Ok("{}".to_string()));
            scripted.map_err(|error| anyhow::anyhow!(error))
        }
        async fn generate_text(&self, _sys: &str, _usr: &str) -> Result<String> {
            Ok(String::new())
        }
    }

    fn low_quality_loop(llm: Arc<dyn LlmClient>) -> SelfImprovementLoop {
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 50,
                ..Default::default()
            },
        );
        for _ in 0..3 {
            loop_runner.record(sample_capture(0.2));
        }
        loop_runner
    }

    #[test]
    fn training_example_export_respects_min_quality() {
        let captures = vec![
            sample_capture(0.9),
            sample_capture(0.3),
            sample_capture(0.75),
        ];

        let examples = ImprovementCycleReport::export_training_examples(&captures, 0.7);
        assert_eq!(examples.len(), 2);
        for ex in &examples {
            assert!(ex.quality_score >= 0.7);
        }
    }

    #[test]
    fn unmeasured_capture_is_not_training_data() {
        let mut capture = OutputCapture::new(
            TaskCategory::InsightGeneration,
            "system",
            "user",
            "response",
            1,
        );
        capture.quality_score = None;
        capture.critique_score = None;

        let examples = ImprovementCycleReport::export_training_examples(&[capture], 0.0);
        assert!(
            examples.is_empty(),
            "an unmeasured capture must never be exported at quality 0.5"
        );
    }

    #[test]
    fn jsonl_export_is_valid() {
        let examples = vec![TrainingExample {
            instruction: "You are an analyst.".into(),
            input: "Assess threat.".into(),
            output: "Threat is high.".into(),
            category: "insight_generation".into(),
            quality_score: 0.9,
            source_id: "test-001".into(),
        }];

        let jsonl = ImprovementCycleReport::to_jsonl(&examples);
        let _: serde_json::Value = serde_json::from_str(&jsonl)
            .unwrap_or_else(|error| panic!("JSONL line should be valid JSON: {error}"));
    }

    #[test]
    fn history_buffer_evicts_on_overflow() {
        let mut loop_runner = SelfImprovementLoop::new(
            Arc::new(FixedLlm {
                body: Ok("{}".into()),
            }),
            SelfImprovementConfig {
                max_history_size: 3,
                ..Default::default()
            },
        );

        for i in 0..5 {
            loop_runner.record(sample_capture(0.5 + 0.1 * i as f64));
        }

        assert_eq!(loop_runner.history.len(), 3);
    }

    #[test]
    fn task_category_as_str_works() {
        assert_eq!(
            TaskCategory::InsightGeneration.as_str(),
            "insight_generation"
        );
        assert_eq!(TaskCategory::Other("custom".into()).as_str(), "custom");
    }

    #[tokio::test]
    async fn failed_model_call_yields_failed_not_empty() {
        let llm = Arc::new(FixedLlm {
            body: Err("model unavailable".into()),
        });
        let mut loop_runner = low_quality_loop(llm);

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.prompt_improvements.is_failed());
        assert!(!report.prompt_improvements.is_empty());
        assert!(report.prompt_improvements.value_ref().is_none());
        let failure = report
            .prompt_improvements
            .failure_ref()
            .expect("failure retained");
        assert_eq!(failure.kind, FailureKind::ModelCallFailed);
        assert!(failure.message.contains("model unavailable"));

        assert!(report.failure_hypotheses.is_failed());
        assert!(!report.failure_hypotheses.is_empty());
        assert_eq!(
            report.failed_stages().len(),
            2,
            "terminal failures must be reported as failed stages"
        );
    }

    #[tokio::test]
    async fn invalid_json_yields_failed_with_parse_error() {
        let llm = Arc::new(FixedLlm {
            body: Ok("this is not JSON".into()),
        });
        let mut loop_runner = low_quality_loop(llm);

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.prompt_improvements.is_failed());
        let failure = report
            .prompt_improvements
            .failure_ref()
            .expect("failure retained");
        assert_eq!(failure.kind, FailureKind::InvalidResponse);
        assert!(
            failure.message.contains("invalid JSON"),
            "parse error must be retained: {failure:?}"
        );
        // The critical regression: invalid JSON must never become an empty
        // PromptImprovement.
        assert!(report.prompt_improvements.value_ref().is_none());
        assert!(report.failure_hypotheses.is_failed());
    }

    #[tokio::test]
    async fn missing_confidence_is_failed_not_default_0_5() {
        let llm = Arc::new(FixedLlm {
            body: Ok(
                r#"{"improved_system_prompt": "better prompt", "rationale": "why"}"#.to_string(),
            ),
        });
        let mut loop_runner = low_quality_loop(llm);

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.prompt_improvements.is_failed());
        let failure = report
            .prompt_improvements
            .failure_ref()
            .expect("failure retained");
        assert_eq!(failure.kind, FailureKind::MissingField);
        assert!(failure.message.contains("confidence"));
        assert!(report.prompt_improvements.value_ref().is_none());
    }

    #[tokio::test]
    async fn empty_batch_has_no_prompt_stage_work() {
        // Captures already critiqued: nothing to do this cycle.
        let llm = Arc::new(FixedLlm {
            body: Ok(r#"{"score": 0.9, "critique": "fine"}"#.to_string()),
        });
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 10,
                ..Default::default()
            },
        );
        for _ in 0..3 {
            loop_runner.record(sample_capture(0.9));
        }

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.critique.is_empty());
        assert_eq!(report.avg_critique_score, Measurement::not_measured());
        assert!(report.prompt_improvements.is_empty());
        assert!(report.failure_hypotheses.is_empty());
    }

    #[tokio::test]
    async fn not_evaluated_critique_is_not_zero() {
        let llm = Arc::new(FixedLlm {
            body: Ok("{}".to_string()),
        });
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 10,
                ..Default::default()
            },
        );

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.critique.is_empty());
        assert_eq!(report.avg_critique_score, Measurement::not_measured());
        assert_ne!(
            report.avg_critique_score.value_copied(),
            Some(0.0),
            "no evaluation must not be encoded as 0.0"
        );
    }

    #[tokio::test]
    async fn all_critiques_failed_is_unavailable_not_zero() {
        let llm = Arc::new(FixedLlm {
            body: Err("connection refused".into()),
        });
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 10,
                ..Default::default()
            },
        );
        for _ in 0..2 {
            loop_runner.record(sample_capture(0.0));
        }
        // Force the critique batch to run: clear the pre-seeded critique score.
        for capture in loop_runner.history.iter_mut() {
            capture.critique_score = None;
        }

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.critique.is_failed());
        assert!(report.avg_critique_score.is_unavailable());
        assert!(report
            .avg_critique_score
            .failure_reason()
            .is_some_and(|reason| reason.message.contains("connection refused")));
        assert_ne!(report.avg_critique_score.value_copied(), Some(0.0));
    }

    #[tokio::test]
    async fn partially_failed_critique_keeps_measured_average() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            Ok(r#"{"score": 0.8, "critique": "good"}"#.to_string()),
            Err("transient failure".to_string()),
        ]));
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 10,
                ..Default::default()
            },
        );
        for _ in 0..2 {
            loop_runner.record(sample_capture(0.5));
        }
        for capture in loop_runner.history.iter_mut() {
            capture.critique_score = None;
        }

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.critique.is_partial());
        assert_eq!(
            report.critique.value_ref().map(|c| c.captures_critiqued),
            Some(1)
        );
        assert_eq!(
            report.critique.value_ref().map(|c| c.critiques_failed),
            Some(1)
        );
        assert_eq!(report.avg_critique_score.value_copied(), Some(0.8));
        assert_eq!(report.stage_failures().len(), 1);
        assert!(
            report.failed_stages().is_empty(),
            "a partial stage that produced a value must not fail the whole cycle"
        );
    }

    #[tokio::test]
    async fn missing_critique_score_is_failed_not_default_0_5() {
        let llm = Arc::new(FixedLlm {
            body: Ok(r#"{"critique": "no score field"}"#.to_string()),
        });
        let mut loop_runner = SelfImprovementLoop::new(
            llm,
            SelfImprovementConfig {
                max_history_size: 10,
                ..Default::default()
            },
        );
        loop_runner.record(sample_capture(0.5));
        loop_runner.history[0].critique_score = None;

        let report = loop_runner.run_cycle().await.expect("cycle runs");

        assert!(report.critique.is_failed());
        let failure = report.critique.failure_ref().expect("failure");
        assert_eq!(failure.kind, FailureKind::MissingField);
        assert!(report.avg_critique_score.is_unavailable());
    }

    #[test]
    fn parsed_valid_improvement_is_kept() {
        // Direct check that a valid payload still succeeds.
        let value: serde_json::Value = serde_json::from_str(
            r#"{"improved_system_prompt": "better", "rationale": "because", "confidence": 0.8}"#,
        )
        .expect("valid json");
        assert_eq!(
            value.get("improved_system_prompt").and_then(|v| v.as_str()),
            Some("better")
        );
    }
}
