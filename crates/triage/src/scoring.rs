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
/no_think"#
        .to_string()
}

/// Build the system prompt for **batch** triage scoring.
///
/// Batch responses are keyed by `index`: the model must echo the input index
/// for every item, which lets the caller map results back to inputs even when
/// the model returns them out of order. Missing/duplicate/out-of-range indexes
/// are schema failures that trigger the per-item fallback.
pub fn build_triage_batch_system_prompt() -> String {
    r#"You are an AI triage analyst for a supply-chain intelligence platform.
You will receive a JSON object with an "items" array. Score every item across
five dimensions.

Respond with valid JSON only, using this exact schema:
{
    "items": [
        {"index": 0, "urgency": 0.0-1.0, "impact": 0.0-1.0,
         "actionability": 0.0-1.0, "novelty": 0.0-1.0, "confidence": 0.0-1.0},
        ...
    ]
}

Rules:
- Return exactly one result object per input item.
- Copy each input item's "index" unchanged into its result.
- Never add, omit, renumber, or reorder items.
- Include no extra top-level keys.

Scoring guidelines:
- urgency: Is there a time constraint? 1.0 = "act within hours", 0.0 = "no time pressure"
- impact: Business/financial/reputational damage potential. 1.0 = "could lose major contract"
- actionability: Can the user do something? 1.0 = "call supplier, update contract"
- novelty: Fresh information vs. known pattern. 1.0 = "never seen before"
- confidence: How certain are you in this assessment? 1.0 = "clear evidence"

Be conservative. Default to 0.3-0.5 range unless you have strong signals.
/no_think"#
        .to_string()
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
            item.title,
            item.description,
            item.item_type.as_str(),
            entity_context,
        );

        let response = self
            .llm
            .generate_json(&build_triage_system_prompt(), &user_prompt)
            .await?;

        // Attempt to parse the response as TriageDimensions
        // The LLM may include a "reasoning" field which we ignore
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .context("Failed to parse triage LLM response as JSON")?;

        let wire: TriageDimensionsWire = serde_json::from_value(parsed)
            .context("Triage LLM response is missing one or more required dimension fields")?;
        TriageDimensions::try_from(wire)
    }

    /// Score multiple items in a single LLM call.
    ///
    /// Sends `{"items":[{"index":n,…}]}` with a batch-specific system prompt
    /// and maps results back by index. Any batch failure — transport error,
    /// invalid JSON, missing/duplicate/out-of-range index, missing dimension
    /// fields, or a wrong result count — falls back to scoring the items
    /// individually. The batch path never abandons scoring via `?`.
    pub async fn score_batch(&self, items: &[TriageQueueItem]) -> Result<Vec<TriageDimensions>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        if items.len() == 1 {
            let dims = self.score_item(&items[0]).await?;
            return Ok(vec![dims]);
        }

        match self.try_score_batch(items).await {
            Ok(scores) => Ok(scores),
            Err(error) => {
                tracing::warn!(
                    %error,
                    item_count = items.len(),
                    "batch triage scoring failed; retrying items individually"
                );
                self.score_items_individually(items).await
            }
        }
    }

    /// Attempt a single batch call. Every failure is returned as `Err` so the
    /// caller can fall back; this helper never aborts the outer batch flow.
    async fn try_score_batch(&self, items: &[TriageQueueItem]) -> Result<Vec<TriageDimensions>> {
        let batch_request = serde_json::json!({
            "items": items
                .iter()
                .enumerate()
                .map(|(index, item)| serde_json::json!({
                    "index": index,
                    "title": item.title,
                    "description": item.description,
                    "type": item.item_type.as_str(),
                    "entity_name": item.entity_name,
                }))
                .collect::<Vec<_>>(),
        });
        let batch_prompt = format!(
            "Score each item in the following JSON payload.\n\n{batch_request}\n\n\
             Respond with a JSON object of the form \
             {{\"items\":[{{\"index\": <copied index>, \"urgency\": 0.0-1.0, \
             \"impact\": 0.0-1.0, \"actionability\": 0.0-1.0, \"novelty\": 0.0-1.0, \
             \"confidence\": 0.0-1.0}}, ...]}} containing exactly one result per item."
        );

        let response = self
            .llm
            .generate_json(&build_triage_batch_system_prompt(), &batch_prompt)
            .await
            .context("batch triage LLM call failed")?;

        let parsed: BatchTriageResponse = serde_json::from_str(&response)
            .context("Failed to parse batch triage response as JSON")?;

        map_batch_scores(parsed, items.len())
    }

    /// Fallback: score each item on its own call.
    async fn score_items_individually(
        &self,
        items: &[TriageQueueItem],
    ) -> Result<Vec<TriageDimensions>> {
        let mut scores = Vec::with_capacity(items.len());
        for item in items {
            scores.push(self.score_item(item).await?);
        }
        Ok(scores)
    }

    /// Calculate composite scores for a slice of dimensions.
    pub fn calculate_composite_scores(&self, dimensions: &[TriageDimensions]) -> Vec<f64> {
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

/// Strict wire schema for the LLM's triage dimensions.
///
/// Every field is **required**: an omitted field is a schema failure, not a
/// neutral 0.5. Out-of-range values are clamped to 0..=1 (the documented
/// tolerance), but a missing field means the response is unusable.
#[derive(Debug, serde::Deserialize)]
struct TriageDimensionsWire {
    urgency: f64,
    impact: f64,
    actionability: f64,
    novelty: f64,
    confidence: f64,
}

/// Strict wire schema for a batch triage response.
///
/// `index` is required in addition to the five dimensions so results can be
/// mapped to inputs positionally. Extra fields (e.g. `reasoning`) are ignored.
#[derive(Debug, serde::Deserialize)]
struct BatchTriageResponse {
    items: Vec<BatchTriageItem>,
}

#[derive(Debug, serde::Deserialize)]
struct BatchTriageItem {
    index: usize,
    urgency: f64,
    impact: f64,
    actionability: f64,
    novelty: f64,
    confidence: f64,
}

/// Map a parsed batch response onto `count` input positions.
///
/// Fails — so the caller can fall back to per-item scoring — when the result
/// count differs, when an index is out of range, when an index repeats, or
/// when any input position is left uncovered.
fn map_batch_scores(parsed: BatchTriageResponse, count: usize) -> Result<Vec<TriageDimensions>> {
    if parsed.items.len() != count {
        anyhow::bail!(
            "batch triage returned {} results for {} items",
            parsed.items.len(),
            count
        );
    }

    let mut slots: Vec<Option<TriageDimensions>> = vec![None; count];
    for item in parsed.items {
        if item.index >= count {
            anyhow::bail!(
                "batch triage result index {} is out of range for {} items",
                item.index,
                count
            );
        }
        if slots[item.index].is_some() {
            anyhow::bail!(
                "batch triage returned duplicate result index {}",
                item.index
            );
        }
        slots[item.index] = Some(TriageDimensions::try_from(TriageDimensionsWire {
            urgency: item.urgency,
            impact: item.impact,
            actionability: item.actionability,
            novelty: item.novelty,
            confidence: item.confidence,
        })?);
    }

    slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            slot.ok_or_else(|| {
                anyhow::anyhow!("batch triage omitted result for item index {index}")
            })
        })
        .collect()
}

impl TryFrom<TriageDimensionsWire> for TriageDimensions {
    type Error = anyhow::Error;

    fn try_from(wire: TriageDimensionsWire) -> Result<Self> {
        let mut dims = TriageDimensions {
            urgency: wire.urgency,
            impact: wire.impact,
            actionability: wire.actionability,
            novelty: wire.novelty,
            confidence: wire.confidence,
        };
        dims.clamp();
        Ok(dims)
    }
}

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

    /// Mock that replays a fixed sequence of responses (one per call) and
    /// records the system/user prompts it saw, so batch fallback and request
    /// shape can be asserted without a network.
    struct SequencedTriageLlm {
        responses: std::sync::Mutex<std::collections::VecDeque<String>>,
        prompts: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl SequencedTriageLlm {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
                prompts: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for SequencedTriageLlm {
        async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
            self.prompts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((system.to_string(), user.to_string()));
            let mut responses = self.responses.lock().unwrap_or_else(|e| e.into_inner());
            responses
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("mock LLM response queue exhausted"))
        }

        async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
            self.generate_json(system, user).await
        }
    }

    fn single_item_json(urgency: f64) -> String {
        format!(
            r#"{{"urgency": {urgency}, "impact": 0.5, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5}}"#
        )
    }

    fn batch_item_json(index: usize, urgency: f64) -> String {
        format!(
            r#"{{"index": {index}, "urgency": {urgency}, "impact": 0.5, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5}}"#
        )
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

    /// The audit contract: a response missing required fields is a schema
    /// failure, never a fabricated neutral 0.5 score.
    #[tokio::test]
    async fn test_score_item_missing_required_field_fails() {
        let mock = MockTriageLlm {
            response: r#"{"urgency": 0.8, "impact": 0.9}"#.to_string(),
        };
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let item = make_test_item("Test", "Description");
        let result = scorer.score_item(&item).await;
        assert!(
            result.is_err(),
            "missing required dimensions must fail, got {result:?}"
        );
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
    async fn test_score_batch_maps_results_by_index() {
        let response = format!(
            r#"{{"items": [{}, {}]}}"#,
            batch_item_json(1, 0.2),
            batch_item_json(0, 0.8)
        );
        let mock = SequencedTriageLlm::new(vec![response]);
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let items = vec![
            make_test_item("Item 1", "First"),
            make_test_item("Item 2", "Second"),
        ];
        let scores = scorer.score_batch(&items).await.unwrap();
        assert_eq!(scores.len(), 2);
        // Results are mapped back by index, not by response order.
        assert!((scores[0].urgency - 0.8).abs() < 1e-9);
        assert!((scores[1].urgency - 0.2).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_score_batch_prompt_contains_indexed_request() {
        let response = format!(
            r#"{{"items": [{}, {}]}}"#,
            batch_item_json(0, 0.5),
            batch_item_json(1, 0.5)
        );
        let mock = std::sync::Arc::new(SequencedTriageLlm::new(vec![response]));
        struct SharedMock(std::sync::Arc<SequencedTriageLlm>);
        #[async_trait::async_trait]
        impl LlmClient for SharedMock {
            async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
                self.0.generate_json(system, user).await
            }
            async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
                self.0.generate_text(system, user).await
            }
        }
        let scorer = TriageScorer::new(Box::new(SharedMock(mock.clone())), TriageConfig::default());
        let items = vec![
            make_test_item("Item 1", "First"),
            make_test_item("Item 2", "Second"),
        ];
        scorer.score_batch(&items).await.unwrap();
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let (system, user) = &calls[0];
        assert!(
            system.contains("\"items\""),
            "batch system prompt: {system}"
        );
        assert!(
            system.contains("\"index\""),
            "batch system prompt: {system}"
        );
        assert!(user.contains("\"items\""), "batch user prompt: {user}");
        assert!(user.contains("\"index\":0"), "batch user prompt: {user}");
        assert!(user.contains("\"index\":1"), "batch user prompt: {user}");
    }

    async fn assert_batch_falls_back(batch_response: &str) {
        let mock = SequencedTriageLlm::new(vec![
            batch_response.to_string(),
            single_item_json(0.11),
            single_item_json(0.22),
        ]);
        let scorer = TriageScorer::new(Box::new(mock), TriageConfig::default());
        let items = vec![
            make_test_item("Item 1", "First"),
            make_test_item("Item 2", "Second"),
        ];
        let scores = scorer
            .score_batch(&items)
            .await
            .expect("fallback must produce scores");
        assert_eq!(scores.len(), 2);
        assert!((scores[0].urgency - 0.11).abs() < 1e-9);
        assert!((scores[1].urgency - 0.22).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_malformed_json() {
        assert_batch_falls_back("this is not json").await;
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_count_mismatch() {
        let response = format!(r#"{{"items": [{}]}}"#, batch_item_json(0, 0.9));
        assert_batch_falls_back(&response).await;
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_duplicate_index() {
        let response = format!(
            r#"{{"items": [{}, {}]}}"#,
            batch_item_json(0, 0.9),
            batch_item_json(0, 0.8)
        );
        assert_batch_falls_back(&response).await;
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_out_of_range_index() {
        let response = format!(
            r#"{{"items": [{}, {}]}}"#,
            batch_item_json(0, 0.9),
            batch_item_json(5, 0.8)
        );
        assert_batch_falls_back(&response).await;
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_missing_dimension_field() {
        let response = r#"{"items": [{"index": 0, "urgency": 0.9, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5}, {"index": 1, "urgency": 0.8, "impact": 0.5, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5}]}"#;
        assert_batch_falls_back(response).await;
    }

    #[tokio::test]
    async fn test_score_batch_falls_back_on_missing_index() {
        let response = format!(
            r#"{{"items": [{}, {}]}}"#,
            batch_item_json(0, 0.9),
            r#"{"urgency": 0.8, "impact": 0.5, "actionability": 0.5, "novelty": 0.5, "confidence": 0.5}"#
        );
        assert_batch_falls_back(&response).await;
    }

    #[test]
    fn test_build_triage_batch_system_prompt_schema() {
        let prompt = build_triage_batch_system_prompt();
        assert!(prompt.contains("/no_think"));
        assert!(prompt.contains("\"items\""));
        assert!(prompt.contains("\"index\""));
        assert!(prompt.contains("urgency"));
        assert!(prompt.contains("confidence"));
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
        let dims = vec![TriageDimensions {
            urgency: 1.0,
            impact: 1.0,
            actionability: 1.0,
            novelty: 1.0,
            confidence: 1.0,
        }];
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
