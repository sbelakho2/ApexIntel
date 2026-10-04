//! Durable insight-analysis worker handler (#169).
//!
//! `POST /api/insights/:id/analyze` no longer runs the model inside the API
//! request: it inserts a `queued` `insight_analysis_runs` row plus a
//! payload-carrying trigger, and this executor owns the computation. The
//! trigger payload names the exact run to execute, so concurrent analysis
//! requests cannot be handed to the wrong row.
//!
//! Lifecycle: claim (`queued` → `running`), load the insight and the same
//! bounded evidence set the synchronous handler used (entities, observations,
//! warnings, related insights), run the LLM under the process-wide
//! concurrency gate, then persist the result JSON (`succeeded`) or the error
//! (`failed`). A run abandoned by a crashed worker is resolved by
//! `expire_stale_insight_analysis_runs` on the trigger poller, never by
//! leaving the insight permanently in flight.
use std::sync::Arc;

use uuid::Uuid;

#[cfg(feature = "llm")]
use apex_llm::LlmClient as _;

use crate::{JobKind, JobRun, PgStore};

/// Extract the run id from a trigger payload, or `None` when the payload is
/// absent/malformed. Pure so the "a payload-less trigger must skip, not guess"
/// rule is unit-testable.
pub(crate) fn run_id_from_payload(payload: Option<&serde_json::Value>) -> Option<Uuid> {
    payload?
        .get("run_id")?
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Execute one queued insight-analysis run.
pub(super) async fn run_insight_analysis(
    kind: &JobKind,
    store: &Arc<PgStore>,
    payload: Option<&serde_json::Value>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let Some(run_id) = run_id_from_payload(payload) else {
        run.skip("insight_analysis: trigger payload is missing run_id");
        return run;
    };

    let claimed = match store.claim_insight_analysis_run(run_id).await {
        Ok(Some(claimed)) => claimed,
        Ok(None) => {
            run.skip(&format!(
                "insight_analysis: run {run_id} was already claimed or is terminal"
            ));
            return run;
        }
        Err(error) => {
            run.fail(&format!(
                "insight_analysis: failed to claim run {run_id}: {error}"
            ));
            return run;
        }
    };

    #[cfg(not(feature = "llm"))]
    {
        let reason = "insight_analysis: this worker build has no LLM support";
        tracing::warn!(
            run_id = %run_id,
            insight_id = %claimed.insight_id,
            "insight_analysis run cannot execute without the llm feature"
        );
        if let Err(error) = store.fail_insight_analysis_run(run_id, reason).await {
            tracing::warn!(run_id = %run_id, %error, "failed to mark insight analysis run failed");
        }
        run.fail(reason);
    }

    #[cfg(feature = "llm")]
    {
        match compute_analysis(store, claimed.insight_id).await {
            Ok(result) => match store.complete_insight_analysis_run(run_id, &result).await {
                Ok(true) => run.succeed(1, &format!("insight_analysis: run {run_id} completed")),
                Ok(false) => {
                    let reason = format!(
                        "insight_analysis: run {run_id} was no longer running when the result was persisted"
                    );
                    if let Err(persist_error) =
                        store.fail_insight_analysis_run(run_id, &reason).await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            error = %persist_error,
                            "insight_analysis: failed to mark the superseded run failed"
                        );
                    }
                    run.fail(&reason);
                }
                Err(error) => {
                    let reason =
                        format!("insight_analysis: failed to persist run {run_id}: {error}");
                    if let Err(persist_error) =
                        store.fail_insight_analysis_run(run_id, &reason).await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            error = %persist_error,
                            "insight_analysis: failed to mark the failed run"
                        );
                    }
                    run.fail(&reason);
                }
            },
            Err(error) => {
                let reason = format!("insight_analysis: run {run_id} failed: {error}");
                if let Err(persist_error) = store.fail_insight_analysis_run(run_id, &reason).await {
                    tracing::warn!(
                        run_id = %run_id,
                        error = %persist_error,
                        "failed to mark insight analysis run failed"
                    );
                }
                run.fail(&reason);
            }
        }
    }

    run
}

/// Load the insight, its bounded evidence set, run the model, and assemble the
/// persisted analysis JSON. Mirrors the evidence loading the synchronous API
/// handler used so the worker path produces the same result shape.
#[cfg(feature = "llm")]
async fn compute_analysis(store: &PgStore, insight_id: Uuid) -> Result<serde_json::Value, String> {
    let insight = store
        .get_insight(insight_id)
        .await
        .map_err(|error| format!("failed to load insight: {error}"))?
        .ok_or_else(|| format!("insight {insight_id} no longer exists"))?;

    let entity_ids: Vec<Uuid> = insight.entity_ids.clone().unwrap_or_default();

    let company_names = store
        .get_company_names_by_ids(&entity_ids)
        .await
        .map_err(|error| format!("failed to load insight entities: {error}"))?;
    let entity_names: Vec<String> = company_names
        .iter()
        .map(|(_, name, _, _)| name.clone())
        .collect();

    let mut all_observations = Vec::new();
    for entity_id in &entity_ids {
        let observations = store
            .get_observations_by_entity(*entity_id, 30)
            .await
            .map_err(|error| format!("failed to load insight evidence: {error}"))?;
        all_observations.extend(observations);
    }
    all_observations.sort_by_key(|observation| std::cmp::Reverse(observation.ts_utc));
    all_observations.truncate(40);

    {
        let mut seen_types = std::collections::HashSet::new();
        all_observations.retain(|observation| {
            let key = format!(
                "{}:{}",
                observation.observation_type,
                observation
                    .value
                    .to_string()
                    .chars()
                    .take(80)
                    .collect::<String>()
            );
            seen_types.insert(key)
        });
    }

    let related_warnings = store
        .get_warnings_by_entity_ids(&entity_ids, 10)
        .await
        .map_err(|error| format!("failed to load related warnings: {error}"))?;
    let related_insights = store
        .get_related_insights(&entity_ids, insight_id, 5)
        .await
        .map_err(|error| format!("failed to load related insights: {error}"))?;
    let evidence_urls = insight.evidence_urls.clone().unwrap_or_default();
    let source_count = evidence_urls.len();

    let mut context_parts: Vec<String> = Vec::new();
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (index, observation) in all_observations.iter().take(6).enumerate() {
            let text = observation
                .value
                .get("excerpt")
                .or(observation.value.get("text"))
                .or(observation.value.get("summary"))
                .or(observation.value.get("title"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                index + 1,
                observation.observation_type,
                truncated
            ));
        }
    }

    for (index, warning) in related_warnings.iter().take(3).enumerate() {
        context_parts.push(format!(
            "[W{}] {} ({})",
            index + 1,
            warning.title,
            warning.severity
        ));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        entity_names.join(", ")
    };
    let region = insight.region.as_deref().unwrap_or("Global");
    let insight_type = insight.insight_type.as_deref().unwrap_or("general");
    let system_prompt = concat!(
        "You are a senior OSINT intelligence analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief analytical report. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [W1] where relevant."
    );

    let summary_truncated: String = insight.summary.chars().take(400).collect();
    let user_prompt = format!(
        r#"Write a 4-paragraph intelligence analysis.

SUBJECT: {entities} ({insight_type}, {region})
CONFIDENCE: {confidence:.0}%

BRIEFING: {summary}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. SITUATION: What is happening and why it matters (3-4 sentences)
2. ANALYSIS: What the data tells us — correlate observations, identify patterns (3-4 sentences)
3. RISK: What could go wrong, which sectors are affected, timeline (2-3 sentences)
4. ACTION: Specific recommendations and what to monitor (2-3 sentences)"#,
        entities = entity_names_str,
        insight_type = insight_type,
        region = region,
        confidence = insight.confidence.unwrap_or(0.0) * 100.0,
        summary = summary_truncated,
        context = context_block,
    );

    let client = build_llm_client();
    // Respect the process-wide model gate (#92): at most N model calls are in
    // flight across every worker job.
    let raw_analysis = {
        let _llm_slot = apex_worker::llm_concurrency::acquire_llm_slot().await;
        client
            .generate_text(system_prompt, &user_prompt)
            .await
            .map_err(|error| format!("LLM analysis failed: {error}"))?
    };

    let paragraphs: Vec<String> = {
        let double_split: Vec<&str> = raw_analysis
            .split("\n\n")
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .collect();
        if double_split.len() >= 4 {
            double_split
                .into_iter()
                .map(|part| part.replace('\n', " "))
                .collect()
        } else {
            raw_analysis
                .split('\n')
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        }
    };

    let exec_summary = strip_analysis_label(
        paragraphs
            .first()
            .map(|part| part.as_str())
            .unwrap_or("Analysis unavailable."),
    );
    let detailed = strip_analysis_label(paragraphs.get(1).map(|part| part.as_str()).unwrap_or(""));
    let risk_assessment_text =
        strip_analysis_label(paragraphs.get(2).map(|part| part.as_str()).unwrap_or(""));
    let recommendations_text =
        strip_analysis_label(paragraphs.get(3).map(|part| part.as_str()).unwrap_or(""));

    let risk_lower = risk_assessment_text.to_lowercase();
    let risk_level = if risk_lower.contains("critical") || risk_lower.contains("severe") {
        "critical"
    } else if risk_lower.contains("high") || risk_lower.contains("significant") {
        "high"
    } else if risk_lower.contains("low") || risk_lower.contains("minimal") {
        "low"
    } else {
        "medium"
    };

    let source_diversity = if source_count >= 8 {
        "excellent"
    } else if source_count >= 5 {
        "good"
    } else if source_count >= 3 {
        "moderate"
    } else {
        "limited"
    };

    let data_sufficiency = if source_count >= 6 && all_observations.len() >= 5 {
        "strong"
    } else if source_count >= 3 {
        "adequate"
    } else if source_count >= 1 {
        "limited"
    } else {
        "insufficient"
    };

    let overall_confidence = insight.confidence.unwrap_or(0.0);

    let analysis = serde_json::json!({
        "executive_summary": exec_summary,
        "key_findings": [{
            "finding": detailed.chars().take(200).collect::<String>(),
            "evidence": format!("{} observations, {} sources", all_observations.len(), source_count),
            "confidence": overall_confidence,
            "impact": risk_level,
        }],
        "detailed_analysis": detailed,
        "source_analysis": {
            "total_sources": source_count,
            "observation_signals": all_observations.len(),
            "corroborating_sources": std::cmp::max(1, source_count.saturating_sub(1)),
            "contradicting_signals": 0,
            "source_diversity_assessment": source_diversity,
        },
        "risk_assessment": {
            "overall_risk": risk_level,
            "probability": overall_confidence,
            "time_horizon": "near_term",
            "affected_sectors": entity_names,
            "escalation_potential": risk_assessment_text,
        },
        "correlations": if detailed.len() > 10 { vec![detailed.clone()] } else { vec![] },
        "recommendations": [{
            "action": recommendations_text.chars().take(200).collect::<String>(),
            "priority": if risk_level == "critical" || risk_level == "high" { "high" } else { "medium" },
            "rationale": format!("Based on {} observations from {} sources at {:.0}% confidence",
                all_observations.len(), source_count, overall_confidence * 100.0),
        }],
        "monitoring_indicators": if !recommendations_text.is_empty() {
            vec![recommendations_text.clone()]
        } else {
            vec![format!("Monitor {} for further developments", entity_names_str)]
        },
        "analytical_confidence": {
            "overall": overall_confidence,
            "data_sufficiency": data_sufficiency,
            "key_uncertainties": [],
        },
    });

    Ok(serde_json::json!({
        "insight_id": insight.id,
        "insight_title": insight.title,
        "analysis": analysis,
        "context_used": {
            "source_count": source_count,
            "observation_count": all_observations.len(),
            "warning_count": related_warnings.len(),
            "related_insight_count": related_insights.len(),
            "entity_count": entity_ids.len(),
        }
    }))
}

/// Build the analysis client. Endpoint/model/key come from the same
/// `LLM_BASE_URL` / `LLM_MODEL` / `LLM_API_KEY` environment as the other
/// worker clients; budgets match the synchronous handler this replaces.
#[cfg(feature = "llm")]
fn build_llm_client() -> apex_llm::OpenAiCompatibleClient {
    let mut config = apex_llm::ModelConfig::llamacpp_default();
    if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
        if !base_url.trim().is_empty() {
            config.base_url = base_url;
        }
    }
    if let Ok(model) = std::env::var("LLM_MODEL") {
        if !model.trim().is_empty() {
            config.model_name = model;
        }
    }
    config.api_key = std::env::var("LLM_API_KEY")
        .ok()
        .map(apex_llm::ApiKeySecret::from);
    config.temperature = 0.5;
    config.max_tokens = 1024;
    config.timeout_seconds = 300;
    apex_llm::OpenAiCompatibleClient::new(config)
}

/// Strip the leading numbered / labelled marker the model adds to a
/// paragraph (e.g. `1. SITUATION:` or `**ACTION:**`). Ported from the
/// synchronous handler so the persisted sections keep the same shape.
fn strip_analysis_label(value: &str) -> String {
    let mut result = value.to_string();
    if result.len() > 2 && result.as_bytes()[0].is_ascii_digit() && result.as_bytes()[1] == b'.' {
        result = result[2..].trim().to_string();
    }
    let labels = [
        "SITUATION:",
        "ANALYSIS:",
        "RISK:",
        "ACTION:",
        "THREAT:",
        "EVIDENCE:",
        "IMPACT:",
        "RESPONSE:",
        "**SITUATION:**",
        "**ANALYSIS:**",
        "**RISK:**",
        "**ACTION:**",
    ];
    for label in labels {
        if result.starts_with(label) {
            result = result[label.len()..].trim().to_string();
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{run_id_from_payload, strip_analysis_label};

    #[test]
    fn payload_run_id_is_parsed_only_from_a_valid_uuid() {
        let run_id = uuid::Uuid::new_v4();
        let payload =
            serde_json::json!({"run_id": run_id.to_string(), "insight_id": uuid::Uuid::new_v4()});
        assert_eq!(run_id_from_payload(Some(&payload)), Some(run_id));

        assert_eq!(run_id_from_payload(None), None);
        assert_eq!(run_id_from_payload(Some(&serde_json::json!({}))), None);
        assert_eq!(
            run_id_from_payload(Some(&serde_json::json!({"run_id": "not-a-uuid"}))),
            None
        );
        assert_eq!(
            run_id_from_payload(Some(&serde_json::json!({"run_id": 42}))),
            None,
            "a non-string run id must not be coerced"
        );
    }

    #[test]
    fn analysis_labels_are_stripped_from_paragraphs() {
        assert_eq!(
            strip_analysis_label("1. SITUATION: Supply risk rising"),
            "Supply risk rising"
        );
        assert_eq!(
            strip_analysis_label("**ACTION:** Monitor inventory"),
            "Monitor inventory"
        );
        assert_eq!(strip_analysis_label("plain analysis"), "plain analysis");
    }
}
