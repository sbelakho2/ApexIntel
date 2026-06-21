//! Real-time alert pipeline wiring (§5.4).
//!
//! Connects the previously-isolated pieces of the streaming alert path into a
//! single end-to-end flow that runs inside the worker process:
//!
//! ```text
//! [warnings table] --poll--> DomainEvent --publish--> NATS `events.domain`
//!     --> AlertEvaluator (subscribed) --> evaluate rules --> NATS `alerts.events.*`
//!     --> (API SSE consumer) --> browser toast + bell
//! ```
//!
//! The evaluator and NATS publisher already existed as library code but were
//! never spawned; this module is the missing glue. Everything degrades
//! gracefully when NATS is unavailable (the publisher becomes a no-op and the
//! producer simply logs).

use std::sync::Arc;
use std::time::Duration;

use apex_store::postgres::PgStore;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::alert_evaluator::{AlertEvaluator, DomainEvent};
use crate::nats_stream::NatsPublisher;

/// NATS subject the producer publishes domain events to and the evaluator
/// subscribes to.
pub const DOMAIN_EVENTS_SUBJECT: &str = "events.domain";

/// Default path to the alert rules file.
const DEFAULT_ALERT_RULES_PATH: &str = "config/runtime/alert-rules.yaml";

/// How often the producer polls for newly-created warnings.
const PRODUCER_POLL_INTERVAL_SECS: u64 = 30;

/// Resolve the NATS URL from the environment (`NATS_URL`), defaulting to the
/// conventional localhost endpoint.
fn nats_url_from_env() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string())
}

/// Resolve the alert rules path from the environment (`ALERT_RULES_PATH`).
fn alert_rules_path_from_env() -> String {
    std::env::var("ALERT_RULES_PATH").unwrap_or_else(|_| DEFAULT_ALERT_RULES_PATH.to_string())
}

/// Spawn the full real-time alert pipeline.
///
/// Returns immediately; all work happens on background tokio tasks. When NATS
/// or the rules file are unavailable the pipeline logs a warning and stays
/// dormant rather than crashing the worker.
pub fn spawn(store: Arc<PgStore>) {
    // Default to enabled; set REALTIME_ALERTS_ENABLED=false to disable.
    let enabled = std::env::var("REALTIME_ALERTS_ENABLED")
        .ok()
        .map(|v| apex_core::env::parse_truthy_flag(&v))
        .unwrap_or(true);
    if !enabled {
        info!("real-time alert pipeline disabled (REALTIME_ALERTS_ENABLED=false)");
        return;
    }


    tokio::spawn(async move {
        let nats_url = nats_url_from_env();

        // 1. Connect the publisher (used by both the producer and evaluator).
        let producer_publisher = NatsPublisher::connect(&nats_url).await;
        if !producer_publisher.is_connected() {
            warn!(
                "real-time alert pipeline: NATS unavailable at {nats_url}; \
                 streaming alerts will not fire until NATS is reachable"
            );
            return;
        }

        // 2. Load rules and spawn the evaluator (subscribes to DOMAIN_EVENTS_SUBJECT).
        let rules_path = alert_rules_path_from_env();
        match AlertEvaluator::load_from_path(&rules_path) {
            Ok(evaluator) => {
                let evaluator_publisher = NatsPublisher::connect(&nats_url).await;
                evaluator.spawn_background_task(
                    evaluator_publisher,
                    vec![DOMAIN_EVENTS_SUBJECT.to_string()],
                );
                info!(
                    rules_path = %rules_path,
                    subject = DOMAIN_EVENTS_SUBJECT,
                    "real-time alert evaluator spawned"
                );
            }
            Err(e) => {
                warn!(
                    rules_path = %rules_path,
                    error = %e,
                    "failed to load alert rules; evaluator not started"
                );
                return;
            }
        }

        // 3. Run the warning→DomainEvent producer loop.
        run_warning_producer(store, producer_publisher).await;
    });
}

/// Poll for new warnings and publish them as domain events to NATS.
async fn run_warning_producer(store: Arc<PgStore>, publisher: NatsPublisher) {
    let Some(client) = publisher.client().cloned() else {
        warn!("alert producer: NATS client unavailable; producer exiting");
        return;
    };

    // Start from "now" so we don't replay historical warnings on boot.
    let mut last_seen: DateTime<Utc> = Utc::now();
    let mut interval = tokio::time::interval(Duration::from_secs(PRODUCER_POLL_INTERVAL_SECS));
    info!(
        interval_secs = PRODUCER_POLL_INTERVAL_SECS,
        "real-time alert producer started"
    );

    loop {
        interval.tick().await;

        let warnings = match store.list_recent_warnings_since(last_seen, 200).await {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %e, "alert producer: failed to query recent warnings");
                continue;
            }
        };

        if warnings.is_empty() {
            continue;
        }

        // Advance the cursor to the newest warning we observed.
        if let Some(newest) = warnings
            .iter()
            .filter_map(|w| w.created_at.or(Some(w.ts_utc)))
            .max()
        {
            last_seen = newest;
        }

        let mut published = 0usize;
        for w in &warnings {
            let event = warning_to_domain_event(w);
            let payload = match serde_json::to_vec(&event) {
                Ok(bytes) => bytes,
                Err(e) => {
                    warn!(error = %e, "alert producer: failed to serialize domain event");
                    continue;
                }
            };
            if let Err(e) = client
                .publish(DOMAIN_EVENTS_SUBJECT.to_string(), payload.into())
                .await
            {
                warn!(error = %e, "alert producer: failed to publish domain event");
            } else {
                published += 1;
            }
        }

        if published > 0 {
            info!(count = published, "alert producer: published warning domain events");
        }
    }
}

/// Convert a stored warning row into a [`DomainEvent`] for the evaluator.
fn warning_to_domain_event(w: &apex_store::postgres::WarningRow) -> DomainEvent {
    let mut event = DomainEvent::new(
        // Use a `warning`-prefixed source so warning-scoped rules match.
        format!("warning_{}", w.warning_type),
        w.severity.clone(),
        w.title.clone(),
        w.description.clone().unwrap_or_default(),
    );

    if let Some(entity_id) = w.entity_ids.as_ref().and_then(|ids| ids.first()).copied() {
        let name = w.region.clone().unwrap_or_default();
        event = event.with_entity(entity_id, name);
    }

    event = event.with_metadata(serde_json::json!({
        "warning_id": w.id,
        "warning_type": w.warning_type,
        "recipe_code": w.recipe_code,
        "region": w.region,
        "confidence": w.confidence,
    }));

    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_warning() -> apex_store::postgres::WarningRow {
        apex_store::postgres::WarningRow {
            id: uuid::Uuid::new_v4(),
            recipe_code: Some("C123".to_string()),
            warning_type: "competitor_change".to_string(),
            title: "Competitor launched new product".to_string(),
            description: Some("A competitor announced a new battery line".to_string()),
            severity: "high".to_string(),
            region: Some("TN".to_string()),
            source_urls: None,
            entity_ids: Some(vec![uuid::Uuid::nil()]),
            confidence: Some(0.8),
            ts_utc: Utc::now(),
            acknowledged: false,
            acknowledged_by: None,
            acknowledged_at: None,
            acknowledged_note: None,
            review_outcome: None,
            deleted_at: None,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn warning_maps_to_domain_event() {
        let w = sample_warning();
        let event = warning_to_domain_event(&w);
        assert_eq!(event.source, "warning_competitor_change");
        assert_eq!(event.severity, "high");
        assert_eq!(event.title, "Competitor launched new product");
        assert!(event.entity_id.is_some());
        assert_eq!(event.metadata["warning_type"], "competitor_change");
    }

    #[test]
    fn nats_url_defaults_to_localhost() {
        std::env::remove_var("NATS_URL");
        assert_eq!(nats_url_from_env(), "nats://127.0.0.1:4222");
    }

    #[test]
    fn rules_path_defaults() {
        std::env::remove_var("ALERT_RULES_PATH");
        assert_eq!(alert_rules_path_from_env(), DEFAULT_ALERT_RULES_PATH);
    }
}
