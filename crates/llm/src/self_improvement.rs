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
//! # Usage
//! ```rust,ignore
//! let mut loop_runner = SelfImprovementLoop::new(llm.clone(), config);
//! loop_runner.record_output(capture).await;
//! let report = loop_runner.run_improvement_cycle().await?;
//! let examples = report.export_training_examples();
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::LlmClient;

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

/// The result of one improvement cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementCycleReport {
    pub cycle_id: String,
    pub ran_at: DateTime<Utc>,
    pub captures_analysed: usize,
    pub examples_qualifying: usize,
    pub prompt_improvements: Vec<PromptImprovement>,
    pub failure_hypotheses: Vec<FailureHypothesis>,
    pub avg_critique_score: f64,
}

impl ImprovementCycleReport {
    /// Export all qualifying captures as training examples.
    pub fn export_training_examples(
        captures: &[OutputCapture],
        min_quality: f64,
    ) -> Vec<TrainingExample> {
        captures
            .iter()
            .filter(|c| {
                let score = c.critique_score.or(c.quality_score).unwrap_or(0.0);
                score >= min_quality && c.was_used
            })
            .map(|c| TrainingExample {
                instruction: c.system_prompt.clone(),
                input: c.user_prompt.clone(),
                output: c.response.clone(),
                category: c.category.as_str().to_string(),
                quality_score: c.critique_score.or(c.quality_score).unwrap_or(0.5),
                source_id: c.id.clone(),
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
        let mut scored = 0;
        for &idx in &batch {
            match self.critique_capture(idx).await {
                Ok((score, text)) => {
                    self.history[idx].critique_score = Some(score);
                    self.history[idx].critique = Some(text);
                    total_score += score;
                    scored += 1;
                }
                Err(e) => {
                    warn!(error=%e, capture_id=%self.history[idx].id, "Critique failed");
                }
            }
        }

        let avg_critique_score = if scored > 0 {
            total_score / scored as f64
        } else {
            0.0
        };

        let examples_qualifying = self
            .history
            .iter()
            .filter(|c| {
                c.critique_score.or(c.quality_score).unwrap_or(0.0) >= self.config.min_quality_score
            })
            .count();

        // Prompt improvement proposals
        let prompt_improvements = if self.config.generate_prompt_proposals {
            self.generate_prompt_improvements()
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // Failure analysis
        let failure_hypotheses = if self.config.analyse_failures {
            self.analyse_failures().await.unwrap_or_default()
        } else {
            vec![]
        };

        let report = ImprovementCycleReport {
            cycle_id: uuid::Uuid::new_v4().to_string(),
            ran_at: Utc::now(),
            captures_analysed: batch.len(),
            examples_qualifying,
            prompt_improvements,
            failure_hypotheses,
            avg_critique_score,
        };

        info!(
            cycle_id = %report.cycle_id,
            analysed = report.captures_analysed,
            qualifying = report.examples_qualifying,
            avg_score = %format!("{:.3}", avg_critique_score),
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

    async fn critique_capture(&self, idx: usize) -> Result<(f64, String)> {
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
            .context("Critique LLM call failed")?;

        let v: serde_json::Value =
            serde_json::from_str(&json).context("Failed to parse critique JSON")?;

        let score = v
            .get("score")
            .and_then(|s| s.as_f64())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let text = v
            .get("critique")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        debug!(capture_id=%capture.id, score=%score, "Critique complete");
        Ok((score, text))
    }

    async fn generate_prompt_improvements(&self) -> Result<Vec<PromptImprovement>> {
        // Group low-scoring captures by category
        let low_quality: Vec<&OutputCapture> = self
            .history
            .iter()
            .filter(|c| c.critique_score.or(c.quality_score).unwrap_or(1.0) < 0.5)
            .take(5)
            .collect();

        if low_quality.is_empty() {
            return Ok(vec![]);
        }

        let examples_text = low_quality
            .iter()
            .map(|c| {
                format!(
                    "[{}] Score {:.2}\nSystem: {}...\nResponse: {}...",
                    c.category.as_str(),
                    c.critique_score.unwrap_or(0.0),
                    crate::truncate_utf8(&c.system_prompt, 200),
                    crate::truncate_utf8(&c.response, 300),
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let system = "You are a prompt engineering expert. Analyse these low-quality LLM outputs \
            and propose an improved system prompt that would produce better results. \
            Return JSON: { \"improved_system_prompt\": str, \"rationale\": str, \"confidence\": float }";

        let json = self.llm.generate_json(system, &examples_text).await?;
        let v: serde_json::Value = match serde_json::from_str(&json) {
            Ok(val) => val,
            Err(e) => {
                warn!(
                    "LLM prompt improvement JSON parse failed: {}. Raw response (first 200 chars): {}",
                    e,
                    &crate::truncate_utf8(&json, 200)
                );
                serde_json::Value::Object(serde_json::Map::new())
            }
        };

        let improvement = PromptImprovement {
            category: low_quality[0].category.as_str().to_string(),
            original_system_prompt: low_quality[0].system_prompt.clone(),
            improved_system_prompt: v
                .get("improved_system_prompt")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            rationale: v
                .get("rationale")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            few_shot_examples: vec![],
            confidence: v.get("confidence").and_then(|s| s.as_f64()).unwrap_or(0.5),
        };

        Ok(vec![improvement])
    }

    async fn analyse_failures(&self) -> Result<Vec<FailureHypothesis>> {
        let failures: Vec<&OutputCapture> = self
            .history
            .iter()
            .filter(|c| c.critique_score.or(c.quality_score).unwrap_or(1.0) < 0.4)
            .take(10)
            .collect();

        if failures.len() < 3 {
            return Ok(vec![]); // Not enough data to hypothesise
        }

        let failure_text = failures
            .iter()
            .map(|c| {
                format!(
                    "Category: {} | Score: {:.2} | Critique: {}",
                    c.category.as_str(),
                    c.critique_score.unwrap_or(0.0),
                    c.critique.as_deref().unwrap_or("no critique"),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You are a machine learning failure analyst. Identify the root cause pattern \
            behind these LLM output failures for an OSINT system. \
            Return JSON: { \"pattern\": str, \"root_cause\": str, \"proposed_fix\": str, \"confidence\": float }";

        let json = self.llm.generate_json(system, &failure_text).await?;
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();

        let hypothesis = FailureHypothesis {
            pattern_description: v
                .get("pattern")
                .and_then(|s| s.as_str())
                .unwrap_or("Unidentified pattern")
                .to_string(),
            affected_categories: failures
                .iter()
                .map(|c| c.category.as_str().to_string())
                .collect(),
            observed_failure_count: failures.len(),
            root_cause_hypothesis: v
                .get("root_cause")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            proposed_fix: v
                .get("proposed_fix")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            confidence: v.get("confidence").and_then(|s| s.as_f64()).unwrap_or(0.5),
        };

        Ok(vec![hypothesis])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        // Use a stub LLM — we're only testing the buffer
        struct StubLlm;
        #[async_trait::async_trait]
        impl LlmClient for StubLlm {
            async fn generate_json(&self, _sys: &str, _usr: &str) -> Result<String> {
                Ok("{}".into())
            }
            async fn generate_text(&self, _sys: &str, _usr: &str) -> Result<String> {
                Ok("".into())
            }
        }

        let mut loop_runner = SelfImprovementLoop::new(
            Arc::new(StubLlm),
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
}
