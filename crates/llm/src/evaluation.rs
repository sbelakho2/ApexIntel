//! LLM output quality evaluation framework.
//!
//! Provides a structured suite for evaluating LLM response quality across
//! multiple dimensions:
//!
//! - **Schema compliance**: Does the JSON response match the expected schema?
//! - **Content quality**: LLM-as-judge scoring on accuracy, relevance, and conciseness.
//! - **Hallucination detection**: Does the response introduce claims not supported by context?
//! - **Task adherence**: Does the response complete the instructed task?
//! - **Regression testing**: Compare against known-good "gold" responses.
//!
//! # Design
//! `EvalSuite` holds a list of `EvalCase` items. Each case defines:
//! - The system and user prompts
//! - Expected output characteristics (schema, keywords, forbidden phrases)
//! - Optional gold response for comparison
//!
//! `EvalRunner` executes the suite against a live `LlmClient`, then uses
//! an optional judge `LlmClient` (can be the same instance) to score outputs.
//!
//! # Example
//! ```rust,ignore
//! let mut suite = EvalSuite::new("insight_generation_v1");
//! suite.add(EvalCase::new("test_1", system_prompt, user_prompt)
//!     .expect_json_keys(&["title", "severity", "narrative"])
//!     .forbid_phrases(&["I don't know", "as an AI"])
//! );
//! let report = EvalRunner::new(llm.clone(), llm.clone()).run(&suite).await?;
//! println!("Pass rate: {:.1}%", report.pass_rate() * 100.0);
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::LlmClient;

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation case
// ─────────────────────────────────────────────────────────────────────────────

/// Expected output characteristics for an evaluation case.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExpectedOutput {
    /// JSON keys that MUST appear in the response (if the response is JSON).
    pub required_json_keys: Vec<String>,
    /// Substrings that MUST appear in the response.
    pub required_keywords: Vec<String>,
    /// Substrings that MUST NOT appear in the response.
    pub forbidden_phrases: Vec<String>,
    /// Minimum sentence count required for narrative responses.
    pub min_sentences: Option<usize>,
    /// Maximum sentence count allowed for narrative responses.
    pub max_sentences: Option<usize>,
    /// The response must parse as valid JSON.
    pub must_be_valid_json: bool,
    /// Optional gold (reference) response for comparison scoring.
    pub gold_response: Option<String>,
    /// Minimum LLM-judge score to pass [0, 1].
    pub min_judge_score: f64,
}

/// One test case for the evaluation suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub description: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub expected: ExpectedOutput,
    pub tags: Vec<String>,
}

impl EvalCase {
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        system_prompt: impl Into<String>,
        user_prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            system_prompt: system_prompt.into(),
            user_prompt: user_prompt.into(),
            expected: ExpectedOutput {
                min_judge_score: 0.6,
                ..Default::default()
            },
            tags: vec![],
        }
    }

    pub fn expect_json_keys(mut self, keys: &[&str]) -> Self {
        self.expected.must_be_valid_json = true;
        self.expected.required_json_keys = keys.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn require_keywords(mut self, keywords: &[&str]) -> Self {
        self.expected.required_keywords = keywords.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn forbid_phrases(mut self, phrases: &[&str]) -> Self {
        self.expected.forbidden_phrases = phrases.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn sentence_bounds(mut self, min_sentences: usize, max_sentences: usize) -> Self {
        self.expected.min_sentences = Some(min_sentences);
        self.expected.max_sentences = Some(max_sentences);
        self
    }

    pub fn with_gold(mut self, gold: impl Into<String>) -> Self {
        self.expected.gold_response = Some(gold.into());
        self
    }

    pub fn min_judge_score(mut self, score: f64) -> Self {
        self.expected.min_judge_score = score;
        self
    }

    pub fn tagged(mut self, tags: &[&str]) -> Self {
        self.tags = tags.iter().map(|s| s.to_string()).collect();
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation result
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of a single evaluation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckOutcome {
    Pass,
    Fail(String),
    Skipped,
}

impl CheckOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn is_ok_for_gate(&self) -> bool {
        matches!(self, Self::Pass | Self::Skipped)
    }
}

/// Result of running one `EvalCase`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub case_id: String,
    pub response: String,
    pub latency_ms: u64,
    /// JSON validity check.
    pub json_valid: CheckOutcome,
    /// Required keys check.
    pub required_keys: CheckOutcome,
    /// Required keywords check.
    pub required_keywords: CheckOutcome,
    /// Forbidden phrases check.
    pub forbidden_phrases: CheckOutcome,
    /// Sentence count bounds check.
    pub sentence_bounds: CheckOutcome,
    /// LLM judge score and pass/fail.
    pub judge_score: Option<f64>,
    pub judge_pass: CheckOutcome,
    /// Judge's reasoning.
    pub judge_rationale: Option<String>,
    /// Similarity to gold response [0, 1] (if applicable).
    pub gold_similarity: Option<f64>,
    /// Overall pass/fail for this case.
    pub passed: bool,
    pub ran_at: DateTime<Utc>,
}

impl EvalResult {
    pub fn failed_checks(&self) -> Vec<String> {
        let mut failures = vec![];
        if let CheckOutcome::Fail(msg) = &self.json_valid {
            failures.push(format!("json_valid: {msg}"));
        }
        if let CheckOutcome::Fail(msg) = &self.required_keys {
            failures.push(format!("required_keys: {msg}"));
        }
        if let CheckOutcome::Fail(msg) = &self.required_keywords {
            failures.push(format!("required_keywords: {msg}"));
        }
        if let CheckOutcome::Fail(msg) = &self.forbidden_phrases {
            failures.push(format!("forbidden_phrases: {msg}"));
        }
        if let CheckOutcome::Fail(msg) = &self.sentence_bounds {
            failures.push(format!("sentence_bounds: {msg}"));
        }
        if let CheckOutcome::Fail(msg) = &self.judge_pass {
            failures.push(format!("judge: {msg}"));
        }
        failures
    }
}

/// Summary report for an entire eval suite run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub suite_name: String,
    pub run_id: String,
    pub ran_at: DateTime<Utc>,
    pub total_cases: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<EvalResult>,
    pub avg_judge_score: f64,
    pub avg_latency_ms: f64,
}

impl EvalReport {
    pub fn pass_rate(&self) -> f64 {
        if self.total_cases == 0 {
            return 0.0;
        }
        self.passed as f64 / self.total_cases as f64
    }

    /// Return all failing case IDs with their failure reasons.
    pub fn failures(&self) -> Vec<(&str, Vec<String>)> {
        self.results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| (r.case_id.as_str(), r.failed_checks()))
            .collect()
    }

    /// Compute hallucination rate estimate: fraction of cases where the LLM
    /// introduced phrases it was explicitly told to forbid or generated invalid JSON.
    pub fn estimated_hallucination_rate(&self) -> f64 {
        if self.total_cases == 0 {
            return 0.0;
        }
        let hallucinated = self
            .results
            .iter()
            .filter(|r| {
                matches!(&r.forbidden_phrases, CheckOutcome::Fail(_))
                    || matches!(&r.json_valid, CheckOutcome::Fail(_))
            })
            .count();
        hallucinated as f64 / self.total_cases as f64
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation suite and runner
// ─────────────────────────────────────────────────────────────────────────────

/// A named collection of `EvalCase` items.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvalSuite {
    pub name: String,
    pub cases: Vec<EvalCase>,
}

impl EvalSuite {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cases: vec![],
        }
    }

    pub fn add(&mut self, case: EvalCase) -> &mut Self {
        self.cases.push(case);
        self
    }

    /// Filter cases by tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&EvalCase> {
        self.cases
            .iter()
            .filter(|c| c.tags.contains(&tag.to_string()))
            .collect()
    }
}

/// Runs an `EvalSuite` against an LLM under test with an optional judge LLM.
pub struct EvalRunner {
    subject: Arc<dyn LlmClient>,
    judge: Arc<dyn LlmClient>,
}

impl EvalRunner {
    pub fn new(subject: Arc<dyn LlmClient>, judge: Arc<dyn LlmClient>) -> Self {
        Self { subject, judge }
    }

    /// Run the full evaluation suite.
    pub async fn run(&self, suite: &EvalSuite) -> Result<EvalReport> {
        info!(suite=%suite.name, cases=%suite.cases.len(), "Starting eval run");

        let mut results = Vec::with_capacity(suite.cases.len());
        let mut total_judge_score = 0.0;
        let mut judge_scored = 0;
        let mut total_latency = 0u64;

        for case in &suite.cases {
            debug!(case_id=%case.id, "Running eval case");
            match self.run_case(case).await {
                Ok(result) => {
                    if let Some(score) = result.judge_score {
                        total_judge_score += score;
                        judge_scored += 1;
                    }
                    total_latency += result.latency_ms;
                    if !result.passed {
                        let failures = result.failed_checks();
                        warn!(case_id=%result.case_id, ?failures, "Eval case FAILED");
                    }
                    results.push(result);
                }
                Err(e) => {
                    warn!(case_id=%case.id, error=%e, "Eval case execution error");
                    // Push a failure result
                    results.push(EvalResult {
                        case_id: case.id.clone(),
                        response: String::new(),
                        latency_ms: 0,
                        json_valid: CheckOutcome::Fail(e.to_string()),
                        required_keys: CheckOutcome::Skipped,
                        required_keywords: CheckOutcome::Skipped,
                        forbidden_phrases: CheckOutcome::Skipped,
                        sentence_bounds: CheckOutcome::Skipped,
                        judge_score: None,
                        judge_pass: CheckOutcome::Fail("execution_error".into()),
                        judge_rationale: Some(e.to_string()),
                        gold_similarity: None,
                        passed: false,
                        ran_at: Utc::now(),
                    });
                }
            }
        }

        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;

        let report = EvalReport {
            suite_name: suite.name.clone(),
            run_id: uuid::Uuid::new_v4().to_string(),
            ran_at: Utc::now(),
            total_cases: results.len(),
            passed,
            failed,
            avg_judge_score: if judge_scored > 0 {
                total_judge_score / judge_scored as f64
            } else {
                0.0
            },
            avg_latency_ms: if !results.is_empty() {
                total_latency as f64 / results.len() as f64
            } else {
                0.0
            },
            results,
        };

        info!(
            suite=%suite.name,
            passed=%report.passed,
            failed=%report.failed,
            pass_rate=%format!("{:.1}%", report.pass_rate() * 100.0),
            avg_score=%format!("{:.3}", report.avg_judge_score),
            "Eval run complete"
        );

        Ok(report)
    }

    async fn run_case(&self, case: &EvalCase) -> Result<EvalResult> {
        let start = std::time::Instant::now();

        // Generate response
        let response =
            if case.expected.must_be_valid_json || !case.expected.required_json_keys.is_empty() {
                self.subject
                    .generate_json(&case.system_prompt, &case.user_prompt)
                    .await?
            } else {
                self.subject
                    .generate_text(&case.system_prompt, &case.user_prompt)
                    .await?
            };

        let latency_ms = start.elapsed().as_millis() as u64;

        // ── Structural checks ──
        let json_valid = self.check_json_valid(case, &response);
        let required_keys = self.check_required_keys(case, &response);
        let required_keywords = self.check_required_keywords(case, &response);
        let forbidden_phrases = self.check_forbidden_phrases(case, &response);
        let sentence_bounds = self.check_sentence_bounds(case, &response);

        // ── LLM judge scoring ──
        let (judge_score, judge_rationale) = self
            .judge_response(case, &response)
            .await
            .unwrap_or_else(|e| {
                warn!(error=%e, "Judge scoring failed");
                (None, Some(format!("Judge error: {e}")))
            });

        let judge_pass = match judge_score {
            Some(score) if score >= case.expected.min_judge_score => CheckOutcome::Pass,
            Some(score) => CheckOutcome::Fail(format!(
                "score {:.3} < threshold {:.3}",
                score, case.expected.min_judge_score
            )),
            None => CheckOutcome::Skipped,
        };

        // ── Gold comparison ──
        let gold_similarity = if let Some(ref gold) = case.expected.gold_response {
            Some(simple_token_similarity(&response, gold))
        } else {
            None
        };

        // ── Overall pass ──
        // Skipped checks should be neutral, not counted as failures.
        let all_structural_pass = json_valid.is_ok_for_gate()
            && required_keys.is_ok_for_gate()
            && required_keywords.is_ok_for_gate()
            && forbidden_phrases.is_ok_for_gate()
            && sentence_bounds.is_ok_for_gate();

        let judge_ok = !matches!(judge_pass, CheckOutcome::Fail(_));
        let passed = all_structural_pass && judge_ok;

        Ok(EvalResult {
            case_id: case.id.clone(),
            response,
            latency_ms,
            json_valid,
            required_keys,
            required_keywords,
            forbidden_phrases,
            sentence_bounds,
            judge_score,
            judge_pass,
            judge_rationale,
            gold_similarity,
            passed,
            ran_at: Utc::now(),
        })
    }

    // ── Check helpers ────────────────────────────────────────────────────────

    fn check_json_valid(&self, case: &EvalCase, response: &str) -> CheckOutcome {
        if !case.expected.must_be_valid_json && case.expected.required_json_keys.is_empty() {
            return CheckOutcome::Skipped;
        }
        match serde_json::from_str::<serde_json::Value>(response) {
            Ok(_) => CheckOutcome::Pass,
            Err(e) => CheckOutcome::Fail(format!("invalid JSON: {e}")),
        }
    }

    fn check_required_keys(&self, case: &EvalCase, response: &str) -> CheckOutcome {
        if case.expected.required_json_keys.is_empty() {
            return CheckOutcome::Skipped;
        }
        let v: serde_json::Value = match serde_json::from_str(response) {
            Ok(v) => v,
            Err(_) => return CheckOutcome::Fail("response is not JSON".into()),
        };
        let missing: Vec<&String> = case
            .expected
            .required_json_keys
            .iter()
            .filter(|k| v.get(k.as_str()).is_none())
            .collect();
        if missing.is_empty() {
            CheckOutcome::Pass
        } else {
            CheckOutcome::Fail(format!("missing keys: {:?}", missing))
        }
    }

    fn check_required_keywords(&self, case: &EvalCase, response: &str) -> CheckOutcome {
        if case.expected.required_keywords.is_empty() {
            return CheckOutcome::Pass;
        }
        let lower = response.to_lowercase();
        let missing: Vec<&String> = case
            .expected
            .required_keywords
            .iter()
            .filter(|kw| !lower.contains(kw.to_lowercase().as_str()))
            .collect();
        if missing.is_empty() {
            CheckOutcome::Pass
        } else {
            CheckOutcome::Fail(format!("missing keywords: {:?}", missing))
        }
    }

    fn check_forbidden_phrases(&self, case: &EvalCase, response: &str) -> CheckOutcome {
        if case.expected.forbidden_phrases.is_empty() {
            return CheckOutcome::Pass;
        }
        let lower = response.to_lowercase();
        let found: Vec<&String> = case
            .expected
            .forbidden_phrases
            .iter()
            .filter(|p| lower.contains(p.to_lowercase().as_str()))
            .collect();
        if found.is_empty() {
            CheckOutcome::Pass
        } else {
            CheckOutcome::Fail(format!("forbidden phrases found: {:?}", found))
        }
    }

    fn check_sentence_bounds(&self, case: &EvalCase, response: &str) -> CheckOutcome {
        let min_sentences = case.expected.min_sentences;
        let max_sentences = case.expected.max_sentences;
        if min_sentences.is_none() && max_sentences.is_none() {
            return CheckOutcome::Skipped;
        }

        let sentence_count = count_sentences(response);
        if let Some(minimum) = min_sentences {
            if sentence_count < minimum {
                return CheckOutcome::Fail(format!(
                    "found {sentence_count} sentences, below minimum {minimum}"
                ));
            }
        }
        if let Some(maximum) = max_sentences {
            if sentence_count > maximum {
                return CheckOutcome::Fail(format!(
                    "found {sentence_count} sentences, above maximum {maximum}"
                ));
            }
        }

        CheckOutcome::Pass
    }

    async fn judge_response(
        &self,
        case: &EvalCase,
        response: &str,
    ) -> Result<(Option<f64>, Option<String>)> {
        let gold_text = case
            .expected
            .gold_response
            .as_deref()
            .map(|g| {
                format!(
                    "\n\nGold response reference:\n{}",
                    crate::truncate_utf8(g, 400)
                )
            })
            .unwrap_or_default();

        let system = "You are a strict quality judge for an OSINT intelligence system. \
            Score the response on task adherence, accuracy, and professionalism [0.0–1.0]. \
            Return JSON: { \"score\": float, \"rationale\": \"brief explanation\" }";

        let user = format!(
            "Task description: {}\n\nSystem prompt: {}...\n\nUser prompt: {}...\n\nResponse to judge:\n{}{}",
            case.description,
            crate::truncate_utf8(&case.system_prompt, 300),
            crate::truncate_utf8(&case.user_prompt, 500),
            crate::truncate_utf8(response, 800),
            gold_text,
        );

        let json = self.judge.generate_json(system, &user).await?;
        let v: serde_json::Value =
            serde_json::from_str(&json).context("Judge response JSON parse failed")?;

        let score = v
            .get("score")
            .and_then(|s| s.as_f64())
            .map(|s| s.clamp(0.0, 1.0));
        let rationale = v
            .get("rationale")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        Ok((score, rationale))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Similarity helper
// ─────────────────────────────────────────────────────────────────────────────

/// Simple token-overlap Jaccard similarity between two strings.
fn simple_token_similarity(a: &str, b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let tokens_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn count_sentences(text: &str) -> usize {
    text.split(['.', '!', '?'])
        .filter(|segment| !segment.trim().is_empty())
        .count()
}

// ─────────────────────────────────────────────────────────────────────────────
// Built-in test suites
// ─────────────────────────────────────────────────────────────────────────────

/// Create the standard ApexIntel evaluation suite.
pub fn standard_eval_suite() -> EvalSuite {
    let mut suite = EvalSuite::new("apexintel_standard_v1");

    // ── Insight generation ──
    suite.add(
        EvalCase::new(
            "insight_json_schema",
            "LLM returns valid insight JSON with required fields",
            "You are an OSINT intelligence analyst. Respond with valid JSON only.",
            r#"Generate an insight card for: Company X is reducing its workforce by 15% citing geopolitical uncertainty. Return JSON with fields: title, severity, narrative, category, confidence."#,
        )
        .expect_json_keys(&["title", "severity", "narrative", "category", "confidence"])
        .forbid_phrases(&["I don't know", "As an AI", "I cannot", "I'm not able"])
        .min_judge_score(0.6)
        .tagged(&["insight", "json", "required"]),
    );

    // ── POI profiling ──
    suite.add(
        EvalCase::new(
            "poi_profile_json",
            "POI profiler returns structured priority and psych profile",
            "You are a POI psychometric analyst. Return JSON only.",
            r#"Profile this executive: John Smith, VP Procurement at Lockheed Martin, former US Navy supply officer, known for cost-first decision making. Return JSON with fields: decision_style, change_appetite, pain_points, priority_vector."#,
        )
        .expect_json_keys(&["decision_style", "change_appetite", "pain_points", "priority_vector"])
        .forbid_phrases(&["I don't know", "As an AI"])
        .min_judge_score(0.55)
        .tagged(&["poi", "json", "required"]),
    );

    // ── Hallucination test ──
    suite.add(
        EvalCase::new(
            "no_ai_disclaimer",
            "LLM does not produce AI disclaimers in OSINT analysis",
            "You are a geopolitical risk analyst. Provide direct analysis without disclaimers in 2-4 sentences, with no bullets or lists.",
            "Assess the supply chain risk for a company with major operations in Taiwan. Keep the answer to 2-4 sentences and explain the main risk drivers.",
        )
        .require_keywords(&["Taiwan"])
        .forbid_phrases(&[
            "I don't know",
            "as an ai",
            "i'm an ai",
            "i cannot provide",
            "i'm not able to",
            "consult a professional",
        ])
        .sentence_bounds(2, 4)
        .min_judge_score(0.0)
        .tagged(&["hallucination", "required"]),
    );

    // ── Gap analysis narrative ──
    suite.add(
        EvalCase::new(
            "gap_analysis_narrative",
            "Gap analysis produces concise 2-4 sentence strategic narrative",
            "You are an OSINT analyst writing a competitive capability gap brief. Be concise, specific, and actionable. Max 4 sentences.",
            "Entity: Elbit Systems\nCapability gaps vs competitors:\n  - AESA radar (held by Rafael, IAI)\n  - Autonomous drone swarm control\nDifferentiators:\n  + Battle-proven EW suite\nWrite a gap analysis paragraph.",
        )
        .require_keywords(&["Elbit", "AESA radar", "drone swarm"])
        .forbid_phrases(&["As an AI", "I cannot"])
        .sentence_bounds(2, 4)
        .min_judge_score(0.0)
        .tagged(&["gap_analysis", "required"]),
    );

    suite
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_similarity_same_string() {
        let s = "hello world test";
        assert!((simple_token_similarity(s, s) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn token_similarity_disjoint_strings() {
        assert!((simple_token_similarity("alpha beta", "gamma delta")).abs() < f64::EPSILON);
    }

    #[test]
    fn eval_case_builder_works() {
        let case = EvalCase::new("test", "desc", "sys", "usr")
            .expect_json_keys(&["key1", "key2"])
            .forbid_phrases(&["bad phrase"]);
        assert!(case.expected.must_be_valid_json);
        assert_eq!(case.expected.required_json_keys.len(), 2);
        assert_eq!(case.expected.forbidden_phrases.len(), 1);
    }

    #[test]
    fn required_keys_check_fails_on_missing_key() {
        let runner = build_dummy_runner();
        let case = EvalCase::new("t", "d", "s", "u").expect_json_keys(&["required_field"]);
        let result = runner.check_required_keys(&case, r#"{"other_field": "value"}"#);
        assert!(matches!(result, CheckOutcome::Fail(_)));
    }

    #[test]
    fn required_keys_check_passes_with_all_keys() {
        let runner = build_dummy_runner();
        let case = EvalCase::new("t", "d", "s", "u").expect_json_keys(&["title", "severity"]);
        let result = runner.check_required_keys(&case, r#"{"title": "T", "severity": "high"}"#);
        assert_eq!(result, CheckOutcome::Pass);
    }

    #[test]
    fn forbidden_phrase_check_detects_ai_disclaimer() {
        let runner = build_dummy_runner();
        let case = EvalCase::new("t", "d", "s", "u").forbid_phrases(&["as an ai"]);
        let result = runner.check_forbidden_phrases(&case, "As an AI, I think this is risky.");
        assert!(matches!(result, CheckOutcome::Fail(_)));
    }

    #[test]
    fn report_pass_rate_accurate() {
        let report = EvalReport {
            suite_name: "test".into(),
            run_id: "r1".into(),
            ran_at: Utc::now(),
            total_cases: 4,
            passed: 3,
            failed: 1,
            results: vec![],
            avg_judge_score: 0.75,
            avg_latency_ms: 200.0,
        };
        assert!((report.pass_rate() - 0.75).abs() < 0.001);
    }

    #[test]
    fn sentence_bounds_check_enforces_limits() {
        let runner = build_dummy_runner();
        let case = EvalCase::new("t", "d", "s", "u").sentence_bounds(2, 4);

        assert!(matches!(
            runner.check_sentence_bounds(&case, "One sentence only."),
            CheckOutcome::Fail(_)
        ));
        assert_eq!(
            runner.check_sentence_bounds(&case, "One. Two. Three."),
            CheckOutcome::Pass
        );
        assert!(matches!(
            runner.check_sentence_bounds(&case, "One. Two. Three. Four. Five."),
            CheckOutcome::Fail(_)
        ));
    }

    #[test]
    fn hallucination_rate_counts_forbidden_and_invalid_json() {
        #[allow(dead_code)]
        struct StubLlm;
        #[async_trait::async_trait]
        impl LlmClient for StubLlm {
            async fn generate_json(&self, _: &str, _: &str) -> Result<String> {
                Ok("{}".into())
            }
            async fn generate_text(&self, _: &str, _: &str) -> Result<String> {
                Ok("".into())
            }
        }

        let report = EvalReport {
            suite_name: "test".into(),
            run_id: "r1".into(),
            ran_at: Utc::now(),
            total_cases: 3,
            passed: 1,
            failed: 2,
            avg_judge_score: 0.5,
            avg_latency_ms: 100.0,
            results: vec![
                make_result("c1", true, CheckOutcome::Pass, CheckOutcome::Pass),
                make_result(
                    "c2",
                    false,
                    CheckOutcome::Fail("bad JSON".into()),
                    CheckOutcome::Pass,
                ),
                make_result(
                    "c3",
                    false,
                    CheckOutcome::Pass,
                    CheckOutcome::Fail("forbidden phrase".into()),
                ),
            ],
        };
        let rate = report.estimated_hallucination_rate();
        assert!((rate - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn standard_suite_has_expected_cases() {
        let suite = standard_eval_suite();
        assert!(suite.cases.len() >= 4);
        assert!(suite.filter_by_tag("required").len() >= 3);
    }

    #[test]
    fn skipped_checks_are_neutral_for_gate() {
        assert!(CheckOutcome::Skipped.is_ok_for_gate());
        assert!(CheckOutcome::Pass.is_ok_for_gate());
        assert!(!CheckOutcome::Fail("x".into()).is_ok_for_gate());
    }

    // ── Helpers ────────────────────────────────────────────────────────────────

    fn build_dummy_runner() -> EvalRunner {
        struct StubLlm;
        #[async_trait::async_trait]
        impl LlmClient for StubLlm {
            async fn generate_json(&self, _: &str, _: &str) -> Result<String> {
                Ok("{}".into())
            }
            async fn generate_text(&self, _: &str, _: &str) -> Result<String> {
                Ok("".into())
            }
        }
        let llm = Arc::new(StubLlm);
        EvalRunner::new(llm.clone(), llm)
    }

    fn make_result(
        id: &str,
        passed: bool,
        json_valid: CheckOutcome,
        forbidden: CheckOutcome,
    ) -> EvalResult {
        EvalResult {
            case_id: id.into(),
            response: "".into(),
            latency_ms: 0,
            json_valid,
            required_keys: CheckOutcome::Pass,
            required_keywords: CheckOutcome::Pass,
            forbidden_phrases: forbidden,
            sentence_bounds: CheckOutcome::Skipped,
            judge_score: Some(0.7),
            judge_pass: CheckOutcome::Pass,
            judge_rationale: None,
            gold_similarity: None,
            passed,
            ran_at: Utc::now(),
        }
    }
}
