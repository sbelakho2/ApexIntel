//! LLM-based triage scoring — prompts, single-item and batch scoring, composite score.

use anyhow::{Context, Result};
use apex_core::triage::{TriageDimensions, TriageQueueItem};
use apex_llm::LlmClient;

use crate::config::TriageConfig;

/// Build the system prompt for triage scoring.
///
/// The prompt instructs the LLM to respond with raw JSON only — no
/// `<think>` or `<no_think>` blocks — and provides scoring guidelines.
pub fn build_triage_system_prompt() -> String {
    r#"You are an AI triage analyst for a supply-chain intelligence platform.
Your task is to score each intelligence item across five dimensions.

Respond with valid JSON only, using this exact schema:
{
    "urgency": 0.0-1.0,
    "impact": 0.0-1.0,
    "actionability": 0.0-1.0,
    "novelty": 0.0-1.0,
    "confidence": 0.0-1.0,
    "reasoning": "brief explanation"
}

Scoring guidelines:
- urgency: Is there a time constraint? 1.0 = "act within hours", 0.0 = "no time pressure"
- impact: Business/financial/reputational damage potential. 1.0 = "could lose major contract"
- actionability: Can the user do something? 1.0 = "call supplier, update contract"
- novelty: Fresh information vs. known pattern. 1.0 = "never seen before"
- confidence: How certain are you in this assessment? 1.0 = "clear evidence"

Be conservative. Default to 0.3-0.5 range unless you have strong signals.
/no_think"#.to_string()
}

/// LLM-based triage scorer.
pub struct TriageScorer {
    llm: Box<dyn LlmClient>,
    config: TriageConfig,
}

impl TriageScorer {
    /// Create a new triage scorer.
    pub fn new(llm: Box<dyn LlmClient>, config: TriageConfig) -> Self {
        Self { llm, config }
    }

    /// Score a single item using the LLM.
    ///
    /// Returns [`TriageDimensions`] parsed from the LLM's JSON response.
    /// The dimensions are clamped to [0.0, 1.0] after parsing.
    pub async fn score_item(&self, item: &TriageQueueItem) -> Result<TriageDimensions> {
        let entity_context = if let Some(ref entity_name) = item.entity_name {
            format!("\nRelated entity: {}", entity_name)
        } else {
            String::new()
        };

        let user_prompt = format!(
            "Title: {}\nDescription: {}\nType: {}{}\n\nScore this item.",
            item.title, item.description, item.item_type.as_str(), entity_context,
        );

        let response = self
            .llm
            .generate_json(&build_triage_system_prompt(), &user_prompt)
            .await?;

        // Attempt to parse the response as TriageDimensions
        // The LLM may include a "reasoning" field which we ignore
        let parsed: serde_json::Value =
            serde_json::from_str(&response).context("Failed to parse triage LLM response as JSON")?;

        let mut dims = TriageDimensions {
            urgency: parsed
                .get("urgency")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5),
            impact: parsed
                .get("impact")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5),
            actionability: parsed
                .get("actionability")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5),
            novelty: parsed
                .get("novelty")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5),
            confidence: parsed
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5),
        };
        dims.clamp();
        Ok(dims)
    }

    /// Score multiple items in a single LLM call.
    ///
    /// Sends all items in one prompt and expects a JSON array response.
    /// Falls back to individual scoring if batch fails.
    pub async fn score_batch(&self, items: &[TriageQueueItem]) -> Result<Vec<TriageDimensions>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        if items.len() == 1 {
            let dims = self.score_item(&items[0]).await?;
            return Ok(vec![dims]);
        }

        let mut batch_prompt = String::from("Score each of the following items:\n\n");
        for (i, item) in items.iter().enumerate() {
            batch_prompt.push_str(&format!(
                "[{}] Title: {}\n    Description: {}\n    Type: {}\n\n",
                i,
                item.title,
                item.description,
                item.item_type.as_str()
            ));
        }
        batch_prompt.push_str(
            "Respond with a JSON array of objects: [{\"urgency\": 0.0-1.0, \"impact\": 0.0-1.0, \"actionability\": 0.0-1.0, \"novelty\": 0.0-1.0, \"confidence\": 0.0-1.0}, ...]"
        );

        let response = self
            .llm
            .generate_json(&build_triage_system_prompt(), &batch_prompt)
            .await;

        match response {
            Ok(resp) => {
                let parsed: Vec<serde_json::Value> = serde_json::from_str(&resp)
                    .context("Failed to parse batch triage response")?;

                let scores: Vec<TriageDimensions> = parsed
                    .into_iter()
                    .map(|v| {
                        let mut dims = TriageDimensions {
                            urgency: v.get("urgency").and_then(|v| v.as_f64()).unwrap_or(0.5),
                            impact: v.get("impact").and_then(|v| v.as_f64()).unwrap_or(0.5),
                            actionability: v
                                .get("actionability")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.5),
                            novelty: v.get("novelty").and_then(|v| v.as_f64()).unwrap_or(0.5),
                            confidence: v.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5),
                        };
                        dims.clamp();
                        dims
                    })
                    .collect();

                if scores.len() != items.len() {
                    anyhow::bail!(
                        "Batch triage returned {} scores for {} items",
                        scores.len(),
                        items.len()
                    );
                }
                Ok(scores)
            }
            Err(_) => {
                // Fall back to individual scoring
                let mut scores = Vec::with_capacity(items.len());
                for item in items {
                    let dims = self.score_item(item).await?;
                    scores.push(dims);
                }
                Ok(scores)
            }
        }
    }

    /// Calculate composite scores for a slice of dimensions.
    pub fn calculate_composite_scores(
        &self,
        dimensions: &[TriageDimensions],
    ) -> Vec<f64> {
        dimensions
            .iter()
            .map(|d| composite_score(d, &self.config.weights))
            .collect()
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &TriageConfig {
        &self.config
    }
}

/// Calculate a composite priority score from dimensions and weights.
///
/// Convenience function re-exported from `apex_core::triage`.
pub use apex_core::triage::composite_score;

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTriageLlm {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmClient for MockTriageLlm {
        async fn generate_json(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }

        async fn generate_text(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    fn make_test_item(title: &str, description: &str) -> TriageQueueItem {
        TriageQueueItem {
            id: uuid::Uuid::new_v4(),
            item_type: apex_core::triage::TriageItemType::Insight,
            source_id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            description: description.to_string(),
            entity_id: None,
            entity_name: None,
            static_severity: Some("high".to_string()),
            dimensions: None,
            composite_score: 0.0,
            is_overridden: false,
            override_score: None,
            status: apex_core::triage::TriageStatus::Pending,
            created_at: chrono::Utc::now(),
            triaged_at: None,
            acknowledged_at: None,
            score_band: String::new(),
            score_band_color: String::new(),
        }
    }

    #[tokio::test]
    async fn test_score_item_parses_response() {
        let mock = MockTriageLlm {
            response: r#"{"urgency": 0.8, "impact": 0.9, "actionability": 0.6, "novelty": 0.4, "confidence": 0.7, "reasoning": "test"}"#.to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let item = make_test_item("Test", "Description");
        let dims = scorer.score_item(&item).await.unwrap();
        assert!((dims.urgency - 0.8).abs() < 1e-9);
        assert!((dims.impact - 0.9).abs() < 1e-9);
        assert!((dims.actionability - 0.6).abs() < 1e-9);
        assert!((dims.novelty - 0.4).abs() < 1e-9);
        assert!((dims.confidence - 0.7).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_score_item_clamps_out_of_range() {
        let mock = MockTriageLlm {
            response: r#"{"urgency": -0.5, "impact": 1.5, "actionability": 0.5, "novelty": 0.0, "confidence": 1.0}"#.to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let item = make_test_item("Test", "Description");
        let dims = scorer.score_item(&item).await.unwrap();
        assert!((dims.urgency - 0.0).abs() < 1e-9);
        assert!((dims.impact - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_score_batch_returns_correct_count() {
        let mock = MockTriageLlm {
            response: r#"[{"urgency": 0.8, "impact": 0.9, "actionability": 0.6, "novelty": 0.4, "confidence": 0.7}, {"urgency": 0.2, "impact": 0.3, "actionability": 0.4, "novelty": 0.5, "confidence": 0.6}]"#.to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let items = vec![
            make_test_item("Item 1", "First"),
            make_test_item("Item 2", "Second"),
        ];
        let scores = scorer.score_batch(&items).await.unwrap();
        assert_eq!(scores.len(), 2);
        assert!((scores[0].urgency - 0.8).abs() < 1e-9);
        assert!((scores[1].urgency - 0.2).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_score_batch_empty() {
        let mock = MockTriageLlm {
            response: "[]".to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let scores = scorer.score_batch(&[]).await.unwrap();
        assert!(scores.is_empty());
    }

    #[tokio::test]
    async fn test_score_batch_single_item() {
        let mock = MockTriageLlm {
            response: r#"{"urgency": 0.5, "impact": 0.5, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5, "reasoning": "test"}"#.to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let items = vec![make_test_item("Single", "Item")];
        let scores = scorer.score_batch(&items).await.unwrap();
        assert_eq!(scores.len(), 1);
    }

    #[test]
    fn test_calculate_composite_scores() {
        let scorer = TriageScorer::new(
            Box::new(MockTriageLlm {
                response: String::new(),
            }),
            TriageConfig::default(),
        );
        let dims = vec![
            TriageDimensions {
                urgency: 1.0,
                impact: 1.0,
                actionability: 1.0,
                novelty: 1.0,
                confidence: 1.0,
            },
        ];
        let scores = scorer.calculate_composite_scores(&dims);
        assert_eq!(scores.len(), 1);
        // With default weights: 0.30 + 0.35 + 0.15 + 0.10 + 0.10 = 1.0
        assert!((scores[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_build_triage_system_prompt_contains_no_think() {
        let prompt = build_triage_system_prompt();
        assert!(prompt.contains("/no_think"));
        assert!(prompt.contains("JSON"));
        assert!(prompt.contains("urgency"));
        assert!(prompt.contains("impact"));
        assert!(prompt.contains("actionability"));
        assert!(prompt.contains("novelty"));
        assert!(prompt.contains("confidence"));
    }
}
