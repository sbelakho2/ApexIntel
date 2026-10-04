//! Triage processing job — scores queued items using the LLM triage scorer.
//!
//! This job runs every 5 minutes by default, picking up unscored items from
//! the triage queue, running them through [`apex_triage::TriageScorer`], and
//! persisting the resulting dimension scores and composite score.

use std::sync::Arc;

use apex_core::triage::{TriageDimensions, TriageStats, TriageThresholds};
use apex_store::postgres::PgStore;
use apex_triage::router_integration::{LoggingAlertDispatcher, RouterIntegration};
use apex_triage::{TriageConfig, TriageQueue, TriageScorer};

use crate::{JobKind, JobRun};

/// Maximum triage items claimed per run (#107).
///
/// The batch is deliberately small: every item costs one LLM call, and the
/// scheduler timeout is 900s. A bounded batch keeps one run comfortably inside
/// its timeout even on a slow local model, instead of one run trying to score
/// the whole queue.
const TRIAGE_BATCH_SIZE: usize = 25;

/// Process unscored triage items through the LLM triage scorer.
///
/// 1. Load config (defaults or from environment).
/// 2. Create a [`TriageQueue`] and [`TriageScorer`].
/// 3. Fetch up to [`TRIAGE_BATCH_SIZE`] unscored items.
/// 4. Score them in batch via the LLM.
/// 5. Persist the scores back to the queue.
#[tracing::instrument(skip(store))]
pub(crate) async fn run_triage_processing(_kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::TriageProcessing);

    // TriageConfig::from_env() returns Self, not Result
    let config = TriageConfig::from_env();

    // Configured weights/thresholds must reach the scoring queue: `new`
    // silently used the defaults regardless of TRIAGE_* configuration.
    let queue = TriageQueue::with_config(
        store.pool.clone(),
        config.weights.clone(),
        config.thresholds.clone(),
    );

    // Get unscored items (bounded batch)
    let unscored = match queue.unscored_items(TRIAGE_BATCH_SIZE).await {
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
        // Captured before `config` moves into the scorer; the notification
        // dispatcher maps severity with the same configured thresholds.
        let thresholds = config.thresholds.clone();
        let scorer = TriageScorer::new(llm, config);

        // #92: the triage batch borrows one process-wide LLM permit for the
        // whole scoring pass (the scorer issues sequential model calls).
        let _llm_slot = apex_worker::llm_concurrency::acquire_llm_slot().await;
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
                notify_scored_queue(store, &queue, &thresholds).await;
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

/// Send a queue-update notification through the given dispatcher.
///
/// A serialization failure is logged with `error!`; a failed activity-feed
/// write is logged by the dispatcher and never fails the job.
async fn emit_queue_update(integration: &RouterIntegration, stats: &TriageStats) {
    let stats_json = match serde_json::to_value(stats) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                error = %error,
                "triage: failed to serialize queue stats for notification"
            );
            return;
        }
    };
    integration.notify_queue_update(stats_json).await;
}

/// Production queue-update path: after a scoring pass the queue contents
/// changed, so notify the dispatcher with the fresh stats. The scoring job is
/// the only bulk writer of triage rows, which is why the queue-update event
/// belongs here rather than on read-only stats endpoints.
async fn notify_scored_queue(
    store: &Arc<PgStore>,
    queue: &TriageQueue,
    thresholds: &TriageThresholds,
) {
    let stats = match queue.stats().await {
        Ok(stats) => stats,
        Err(error) => {
            tracing::error!(
                error = %error,
                "triage: failed to read queue stats for notification"
            );
            return;
        }
    };
    let integration = RouterIntegration::new(Box::new(
        LoggingAlertDispatcher::with_db_and_thresholds(store.pool.clone(), thresholds.clone()),
    ));
    emit_queue_update(&integration, &stats).await;
}

/// Build a lightweight LLM client for triage scoring.
///
/// Triage dimension scoring is a small-model task (audit P0 #24). The model is
/// resolved by tier (`LLM_SMALL_MODEL`, defaulting to the small local model),
/// not by `LLM_MODEL`, so triage never silently runs on the 30B tier.
#[cfg(feature = "llm")]
fn build_triage_llm_client() -> Box<dyn apex_llm::LlmClient> {
    let mut llm_config = apex_llm::ModelConfig::llamacpp_lightweight();
    llm_config.max_tokens = 1024;
    llm_config.temperature = 0.1;
    if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
        llm_config.base_url = base_url;
    }
    let tiered = apex_llm::tiering::TieredModels::from_env();
    if let Some(model) = tiered.model_for(apex_llm::tiering::Workflow::TriageDimensions) {
        llm_config.model_name = model.to_string();
    }
    llm_config.api_key = std::env::var("LLM_API_KEY")
        .ok()
        .map(apex_llm::ApiKeySecret::from);
    Box::new(apex_llm::OpenAiCompatibleClient::new(llm_config))
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use apex_triage::router_integration::{AlertDispatcher, TriageAlertRequest};

    #[derive(Clone, Default)]
    struct RecordingDispatcher {
        queue_updates: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl AlertDispatcher for RecordingDispatcher {
        async fn dispatch_triage_alert(&self, _request: TriageAlertRequest<'_>) -> Vec<uuid::Uuid> {
            Vec::new()
        }

        async fn notify_queue_update(&self, stats_json: &str) {
            self.queue_updates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(stats_json.to_string());
        }

        async fn notify_status_change(
            &self,
            _queue_item_id: uuid::Uuid,
            _new_status: &str,
            _title: &str,
        ) {
        }
    }

    fn sample_stats() -> TriageStats {
        TriageStats {
            total: 7,
            pending: 1,
            triaged: 2,
            acknowledged: 1,
            resolved: 2,
            dismissed: 1,
            critical_count: 1,
            high_count: 2,
            medium_count: 2,
            low_count: 2,
            avg_urgency: 0.4,
            avg_impact: 0.5,
            avg_actionability: 0.6,
            avg_novelty: 0.3,
            avg_confidence: 0.7,
            avg_composite: 0.5,
            overridden_count: 0,
            override_rate: 0.0,
            resolution_rate: 0.4,
        }
    }

    /// Regression: the scoring job is the production queue-update path and
    /// must call the notification helper after a successful scoring pass.
    /// Before wiring, nothing notified the dispatcher from this job.
    #[test]
    fn scoring_pass_wires_queue_update_notification() {
        let source = include_str!("triage.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            production.contains("notify_scored_queue(store, &queue, &thresholds).await"),
            "the successful scoring branch must emit a queue update notification"
        );
        assert!(
            production.contains("emit_queue_update(&integration, &stats).await"),
            "the queue update must go through the dispatcher"
        );
    }

    /// Regression: a scoring pass that changed queue contents must notify the
    /// dispatcher with the fresh queue stats.
    #[tokio::test]
    async fn scoring_pass_notifies_dispatcher_with_queue_stats() {
        let dispatcher = RecordingDispatcher::default();
        let recorded = dispatcher.queue_updates.clone();
        let integration = RouterIntegration::new(Box::new(dispatcher));

        emit_queue_update(&integration, &sample_stats()).await;

        let recorded = recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0].contains("\"total\":7"),
            "queue stats payload must round-trip: {}",
            recorded[0]
        );
    }
}
