//! Triage processing job — scores queued items using the LLM triage scorer.
//!
//! This job runs every 5 minutes by default, picking up unscored items from
//! the triage queue, running them through [`apex_triage::TriageScorer`], and
//! persisting the resulting dimension scores and composite score.

use std::sync::Arc;

use apex_core::triage::TriageDimensions;
use apex_store::postgres::PgStore;
use apex_triage::{TriageConfig, TriageQueue, TriageScorer};

use crate::{JobKind, JobRun};

/// Process unscored triage items through the LLM triage scorer.
///
/// 1. Load config (defaults or from environment).
/// 2. Create a [`TriageQueue`] and [`TriageScorer`].
/// 3. Fetch up to 50 unscored items.
/// 4. Score them in batch via the LLM.
/// 5. Persist the scores back to the queue.
#[tracing::instrument(skip(store))]
pub(crate) async fn run_triage_processing(_kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::TriageProcessing);

    // TriageConfig::from_env() returns Self, not Result
    let config = TriageConfig::from_env();

    let queue = TriageQueue::new(store.pool.clone());

    // Get unscored items (up to 50 per batch)
    let unscored = match queue.unscored_items(50).await {
        Ok(items) => items,
        Err(e) => {
            run.fail(&format!("failed to fetch unscored items: {e}"));
            return run;
        }
    };

    if unscored.is_empty() {
        run.skip("no unscored items to process");
        return run;
    }

    run.start();

    // ── LLM scoring (requires `llm` feature) ──────────────────────────────
    #[cfg(feature = "llm")]
    {
        let llm = build_triage_llm_client();
        let scorer = TriageScorer::new(llm, config);

        let dimensions = match scorer.score_batch(&unscored).await {
            Ok(dims) => dims,
            Err(e) => {
                run.fail(&format!("LLM scoring failed: {e}"));
                return run;
            }
        };

        // Build score entries — batch_update_scores expects Vec<(Uuid, TriageDimensions)>
        let score_entries: Vec<(uuid::Uuid, TriageDimensions)> = dimensions
            .into_iter()
            .zip(unscored.iter())
            .map(|(d, item)| (item.id, d))
            .collect();

        // Persist scores (composite_score is computed internally by batch_update_scores)
        match queue.batch_update_scores(score_entries).await {
            Ok(count) => {
                run.succeed(count, &format!("scored {} items", unscored.len()));
            }
            Err(e) => {
                run.fail(&format!("failed to persist scores: {e}"));
            }
        }
    }

    #[cfg(not(feature = "llm"))]
    {
        run.skip("triage processing requires the `llm` feature");
    }

    run
}

/// Build a lightweight LLM client for triage scoring.
#[cfg(feature = "llm")]
fn build_triage_llm_client() -> Box<dyn apex_llm::LlmClient> {
    let mut llm_config = apex_llm::ModelConfig::llamacpp_lightweight();
    llm_config.max_tokens = 1024;
    llm_config.temperature = 0.1;
    if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
        llm_config.base_url = base_url;
    }
    if let Ok(model) = std::env::var("LLM_MODEL") {
        if !model.trim().is_empty() {
            llm_config.model_name = model;
        }
    }
    llm_config.api_key = std::env::var("LLM_API_KEY")
        .ok()
        .map(apex_llm::ApiKeySecret::from);
    Box::new(apex_llm::OpenAiCompatibleClient::new(llm_config))
}
