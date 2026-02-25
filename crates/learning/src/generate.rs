//! LLM-backed hypothesis generation — connects mined pattern candidates
//! to the LLM client via the hypothesis prompt builders.
//!
//! This module bridges:
//! - `hypothesis::build_system_prompt()` / `build_user_prompt()` — prompt construction
//! - `apex_llm::LlmClient::generate_json()` — LLM inference
//! - `hypothesis::parse_hypothesis_response()` / `validate_hypothesis()` — parse & validate
//!
//! # Usage
//!
//! ```rust,ignore
//! let client = OpenAiCompatibleClient::new(ModelConfig::llamacpp_default());
//! let candidates = miner::mine_patterns(&observations, &config);
//! let hypotheses = generate_hypotheses(&client, &candidates, &existing_ids).await;
//! ```

use crate::hypothesis::{
    build_system_prompt, build_user_prompt, parse_hypothesis_response, validate_hypothesis,
    RecipeHypothesis,
};
use crate::miner::PatternCandidate;
use apex_llm::LlmClient;
use tracing;

/// Maximum number of retries for a single hypothesis generation attempt.
const MAX_RETRIES: usize = 2;

/// Result of a single hypothesis generation attempt.
#[derive(Debug)]
pub enum HypothesisResult {
    /// Successfully generated and validated hypothesis.
    Success(RecipeHypothesis),
    /// LLM returned a response but it failed validation.
    ValidationFailed {
        candidate_outcome: String,
        issues: Vec<String>,
    },
    /// LLM call failed after retries.
    LlmError {
        candidate_outcome: String,
        error: String,
    },
}

/// Generate a single recipe hypothesis from a pattern candidate.
///
/// Retries up to `MAX_RETRIES` times on LLM errors or validation failures.
/// Returns `HypothesisResult` indicating success or the failure mode.
pub async fn generate_hypothesis(
    client: &dyn LlmClient,
    candidate: &PatternCandidate,
    existing_ids: &[String],
) -> HypothesisResult {
    let system = build_system_prompt();

    for attempt in 0..=MAX_RETRIES {
        let user = build_user_prompt(candidate, existing_ids);

        let raw = match client.generate_json(&system, &user).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    attempt,
                    outcome = %candidate.outcome,
                    error = %e,
                    "generate_hypothesis: LLM call failed"
                );
                if attempt < MAX_RETRIES {
                    continue;
                }
                return HypothesisResult::LlmError {
                    candidate_outcome: candidate.outcome.clone(),
                    error: e.to_string(),
                };
            }
        };

        // Parse the response
        let hyp = match parse_hypothesis_response(&raw) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    attempt,
                    outcome = %candidate.outcome,
                    error = %e,
                    "generate_hypothesis: parse failed"
                );
                if attempt < MAX_RETRIES {
                    continue;
                }
                return HypothesisResult::ValidationFailed {
                    candidate_outcome: candidate.outcome.clone(),
                    issues: vec![format!("Parse error: {}", e)],
                };
            }
        };

        // Validate against candidate stats
        let issues = validate_hypothesis(&hyp, candidate, existing_ids);
        if issues.is_empty() {
            tracing::info!(
                id = %hyp.id,
                outcome = %candidate.outcome,
                signals = hyp.signals.len(),
                "generate_hypothesis: success"
            );
            return HypothesisResult::Success(hyp);
        }

        tracing::warn!(
            attempt,
            outcome = %candidate.outcome,
            issues = ?issues,
            "generate_hypothesis: validation issues"
        );

        if attempt < MAX_RETRIES {
            continue;
        }

        return HypothesisResult::ValidationFailed {
            candidate_outcome: candidate.outcome.clone(),
            issues,
        };
    }

    unreachable!("loop always returns")
}

/// Generate hypotheses for a batch of pattern candidates.
///
/// Processes candidates sequentially (to avoid overwhelming the LLM server).
/// Returns results for each candidate in order.
///
/// `existing_ids` is updated as new hypotheses are generated to prevent
/// duplicate IDs within a batch.
pub async fn generate_hypotheses_batch(
    client: &dyn LlmClient,
    candidates: &[PatternCandidate],
    initial_existing_ids: &[String],
) -> Vec<HypothesisResult> {
    let mut existing_ids: Vec<String> = initial_existing_ids.to_vec();
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        tracing::info!(
            index = i,
            total = candidates.len(),
            outcome = %candidate.outcome,
            "generate_hypotheses_batch: processing candidate"
        );

        let result = generate_hypothesis(client, candidate, &existing_ids).await;

        // Track generated IDs to prevent duplicates
        if let HypothesisResult::Success(ref hyp) = result {
            existing_ids.push(hyp.id.clone());
        }

        results.push(result);
    }

    let success_count = results
        .iter()
        .filter(|r| matches!(r, HypothesisResult::Success(_)))
        .count();

    tracing::info!(
        total = candidates.len(),
        successes = success_count,
        failures = candidates.len() - success_count,
        "generate_hypotheses_batch: complete"
    );

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;

    /// Mock LLM client that returns a canned valid hypothesis JSON.
    struct MockLlmClient {
        response: String,
    }

    impl MockLlmClient {
        fn valid() -> Self {
            Self {
                response: r#"{
                    "id": "mock_recipe_01",
                    "join": "Entity",
                    "outcome": "supplier_distress",
                    "signals": ["late_filing", "layoff_announcement"],
                    "transforms": [{"type": "Lag", "days": 30}],
                    "test": {"type": "FisherExact"},
                    "thresholds": {"min_effect": 2.0, "max_p_value": 0.01, "min_stability": 0.7, "max_false_alarm_rate": 0.05},
                    "narrative_template": "Supplier shows distress: {{evidence:late_filing}} and {{evidence:layoff_announcement}}",
                    "action_playbook": ["Review supplier contract", "Identify backup supplier"],
                    "applicability": {"geos": ["TN"], "industries": ["EMS"], "notes": ""}
                }"#
                .to_string(),
            }
        }

        fn invalid_json() -> Self {
            Self {
                response: "This is not JSON at all".to_string(),
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn generate_json(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }
        async fn generate_text(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    /// Mock that always errors.
    struct ErrorLlmClient;

    #[async_trait]
    impl LlmClient for ErrorLlmClient {
        async fn generate_json(&self, _system: &str, _user: &str) -> Result<String> {
            anyhow::bail!("Connection refused")
        }
        async fn generate_text(&self, _system: &str, _user: &str) -> Result<String> {
            anyhow::bail!("Connection refused")
        }
    }

    fn sample_candidate() -> PatternCandidate {
        PatternCandidate {
            outcome: "supplier_distress".to_string(),
            signals: vec!["late_filing".to_string(), "layoff_announcement".to_string()],
            best_lag_days: 30,
            effect_size: 3.5,
            p_value: 0.003,
            q_value: 0.01,
            stability: 0.85,
            entity_coverage: 0.6,
            segments: vec!["TN".to_string()],
            contingency: (15, 5, 3, 30),
        }
    }

    #[tokio::test]
    async fn test_generate_hypothesis_success() {
        let client = MockLlmClient::valid();
        let candidate = sample_candidate();
        let result = generate_hypothesis(&client, &candidate, &[]).await;
        match result {
            HypothesisResult::Success(hyp) => {
                assert_eq!(hyp.id, "mock_recipe_01");
                assert_eq!(hyp.signals.len(), 2);
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_generate_hypothesis_invalid_json() {
        let client = MockLlmClient::invalid_json();
        let candidate = sample_candidate();
        let result = generate_hypothesis(&client, &candidate, &[]).await;
        assert!(matches!(result, HypothesisResult::ValidationFailed { .. }));
    }

    #[tokio::test]
    async fn test_generate_hypothesis_llm_error() {
        let client = ErrorLlmClient;
        let candidate = sample_candidate();
        let result = generate_hypothesis(&client, &candidate, &[]).await;
        assert!(matches!(result, HypothesisResult::LlmError { .. }));
    }

    #[tokio::test]
    async fn test_generate_hypotheses_batch() {
        let client = MockLlmClient::valid();
        let candidates = vec![sample_candidate(), sample_candidate()];
        let results = generate_hypotheses_batch(&client, &candidates, &[]).await;
        assert_eq!(results.len(), 2);
        // First should succeed
        assert!(matches!(&results[0], HypothesisResult::Success(_)));
        // Second should fail validation (duplicate ID "mock_recipe_01")
        assert!(matches!(&results[1], HypothesisResult::ValidationFailed { .. }));
    }
}
