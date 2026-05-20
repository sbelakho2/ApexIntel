use std::collections::HashMap;
#[cfg(feature = "llm")]
use std::collections::HashSet;
use std::sync::Arc;

use apex_core::analysis::{assess_evidence_quality, EvidenceRecord, EvidenceStance};
use apex_core::quality_score::{build_source_reliability_stats, SourceReliability};
#[cfg(feature = "llm")]
use apex_insights::insight_feedback::{
    FatigueAction, InsightFeedback as TrackerInsightFeedback, InsightFeedbackRecord,
    InsightFeedbackTracker,
};
use apex_recipes::stats_enrichment::{
    alert_score_from_features, alert_score_rank_correlation, analyze_with_calibration,
    calibration_curve, fit_best_alert_calibration_model, CalibrationSample, StatsEnrichmentInput,
};
#[cfg(feature = "llm")]
use apex_store::postgres::InsightRow;
use uuid::Uuid;

use crate::*;

#[cfg(feature = "llm")]
fn evidence_quality_from_signals(
    evidence_signals: &[EvidenceSignal],
    source_reliability_scores: &HashMap<String, f64>,
) -> f64 {
    let now = Utc::now();
    let records: Vec<EvidenceRecord> = evidence_signals
        .iter()
        .map(|signal| {
            let relevance = blended_evidence_relevance(
                signal.relevance_score as f64,
                source_reliability_for_url(&signal.source_url, source_reliability_scores),
            );
            let mut record = EvidenceRecord::new(relevance, EvidenceStance::Supports)
                .with_source_url(signal.source_url.clone())
                .with_source_type(signal.signal_type.clone());
            if let Some(date_context) = signal.date_context.as_deref() {
                if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(date_context) {
                    record = record.with_observed_at(parsed.with_timezone(&Utc));
                }
            }
            record
        })
        .collect();
    assess_evidence_quality(&records, now).overall_score
}

#[cfg(feature = "llm")]
fn is_grounding_evidence_signal(signal: &EvidenceSignal) -> bool {
    let signal_type = signal.signal_type.trim().to_ascii_lowercase();
    if signal_type == "warning"
        || signal_type == "capability"
        || signal_type == "facility"
        || signal_type == "poi"
        || signal_type.starts_with("graph_")
    {
        return false;
    }

    !signal.source_url.trim().is_empty() || signal.date_context.is_some()
}

#[cfg(feature = "llm")]
fn grounding_evidence_count(evidence_signals: &[EvidenceSignal]) -> usize {
    evidence_signals
        .iter()
        .filter(|signal| is_grounding_evidence_signal(signal))
        .count()
}

#[cfg(feature = "llm")]
fn should_emit_llm_warning(
    warning_severity: &str,
    warning_action: &str,
    evidence_signals: &[EvidenceSignal],
    evidence_quality: f64,
    insight_inserted: bool,
) -> bool {
    let grounding_count = grounding_evidence_count(evidence_signals);
    let reference_count = count_numbered_references(warning_action, 12);
    if grounding_count == 0 || reference_count == 0 {
        return false;
    }

    let normalized_severity = warning_severity.trim().to_ascii_lowercase();
    if normalized_severity != "critical" && !insight_inserted {
        // Even without LLM insight, allow warnings through if the evidence
        // contains strong concrete signals (tender notices, regulatory filings,
        // security incidents, POI moves, social mentions with high engagement).
        // This prevents quality gates from suppressing all warnings when the
        // LLM insight was rejected but the underlying evidence is solid.
        if !has_concrete_evidence(evidence_signals) {
            return false;
        }
    }

    if low_signal_certification_warning_case(evidence_signals) && normalized_severity != "critical"
    {
        return false;
    }

    if normalized_severity == "critical" {
        return evidence_quality >= 0.45;
    }

    grounding_count >= 2 && evidence_quality >= 0.55
}

/// Checks whether the evidence signals contain concrete, verifiable business
/// activity that should produce a warning even when the LLM insight was
/// rejected by quality gates.
///
/// Concrete evidence types include: tender/contract awards, regulatory filings,
/// security incidents, POI movements, social mentions, news articles, business
/// deals, investments, government policy changes, and trade actions.
///
/// The function requires either:
/// - At least 2 concrete signals with relevance_score >= 0.6, or
/// - At least 1 concrete signal with relevance_score >= 0.8
#[cfg(feature = "llm")]
fn has_concrete_evidence(signals: &[EvidenceSignal]) -> bool {
    // Normalized signal type patterns that represent concrete, verifiable events.
    // Each entry is the signal_type lowered and with underscores/special chars removed,
    // so both "tender_notice" and "TenderNotice" match "tendernotice".
    fn normalized_type(signal_type: &str) -> String {
        signal_type
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    }

    let concrete_normalized: [&str; 28] = [
        "tendernotice", "tender",
        "regulatoryfiling", "regulatory",
        "contractaward", "contract",
        "securityincident", "security", "breach",
        "personmove", "person", "leadershipchange", "hire",
        "socialmention", "social", "socialpost",
        "newsarticle", "news",
        "deal", "partnership", "investment",
        "governmentpolicy", "government", "policy",
        "tradeaction", "trade", "sanction",
        "certification",
    ];

    let concrete_count = signals
        .iter()
        .filter(|s| {
            let norm = normalized_type(&s.signal_type);
            concrete_normalized.contains(&norm.as_str())
        })
        .filter(|s| s.relevance_score >= 0.6)
        .count();

    // Require at least 2 concrete evidence signals or 1 very strong one
    concrete_count >= 2
        || signals.iter().any(|s| {
            let norm = normalized_type(&s.signal_type);
            concrete_normalized.contains(&norm.as_str()) && s.relevance_score >= 0.8
        })
}

#[allow(dead_code)]
fn evidence_quality_from_urls(
    urls: &[String],
    source_reliability_scores: &HashMap<String, f64>,
) -> f64 {
    let now = Utc::now();
    let records: Vec<EvidenceRecord> = urls
        .iter()
        .map(|url| {
            let relevance = blended_evidence_relevance(
                0.6,
                source_reliability_for_url(url, source_reliability_scores),
            );
            EvidenceRecord::new(relevance, EvidenceStance::Supports).with_source_url(url.clone())
        })
        .collect();
    assess_evidence_quality(&records, now).overall_score
}

fn normalized_source_domain(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed.as_str());
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .split('?')
        .next()
        .unwrap_or(without_scheme)
        .split('#')
        .next()
        .unwrap_or(without_scheme)
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .trim_start_matches("www.")
        .trim();

    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn source_reliability_for_url(
    value: &str,
    source_reliability_scores: &HashMap<String, f64>,
) -> Option<f64> {
    normalized_source_domain(value)
        .and_then(|domain| source_reliability_scores.get(&domain).copied())
}

fn blended_evidence_relevance(base_relevance: f64, source_reliability: Option<f64>) -> f64 {
    match source_reliability {
        Some(source_reliability) => (0.65 * base_relevance.clamp(0.0, 1.0)
            + 0.35 * source_reliability.clamp(0.0, 1.0))
        .clamp(0.0, 1.0),
        None => base_relevance.clamp(0.0, 1.0),
    }
}

#[cfg(feature = "llm")]
fn hypothesis_posterior_margin(summary: &apex_insights::hypothesis::HypothesisSummary) -> f64 {
    let leading = summary
        .all_posteriors
        .first()
        .map(|(_, probability)| *probability)
        .unwrap_or(0.0);
    let runner_up = summary
        .all_posteriors
        .get(1)
        .map(|(_, probability)| *probability)
        .unwrap_or(0.0);
    (leading - runner_up).max(0.0)
}

#[cfg(feature = "llm")]
fn ach_signal_highlight(signal: &EvidenceSignal) -> Option<String> {
    let highlight = signal
        .extracted_facts
        .iter()
        .map(|fact| fact.trim())
        .find(|fact| fact.len() >= 18)
        .unwrap_or_else(|| signal.title.trim());

    if highlight.len() < 18 {
        return None;
    }

    Some(format!(
        "{} [{}]",
        crate::truncate_text(highlight, 110),
        signal.signal_type.trim().replace('_', " ")
    ))
}

#[cfg(feature = "llm")]
fn ach_evidence_highlights(evidence_signals: &[EvidenceSignal], limit: usize) -> Vec<String> {
    let mut ranked: Vec<&EvidenceSignal> = evidence_signals.iter().collect();
    ranked.sort_by(|left, right| {
        right
            .relevance_score
            .partial_cmp(&left.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.title.len().cmp(&left.title.len()))
    });

    let mut highlights = Vec::new();
    let mut seen = HashSet::new();
    for signal in ranked {
        let Some(highlight) = ach_signal_highlight(signal) else {
            continue;
        };
        if !seen.insert(highlight.to_ascii_lowercase()) {
            continue;
        }
        highlights.push(highlight);
        if highlights.len() >= limit {
            break;
        }
    }

    highlights
}

#[cfg(feature = "llm")]
fn build_hypothesis_ach_candidate(
    summary: &apex_insights::hypothesis::HypothesisSummary,
    evidence_signals: &[EvidenceSignal],
) -> Option<(String, String, f64, u32)> {
    let leading = summary.leading_hypothesis.as_deref()?.trim();
    let leading_posterior = summary.leading_posterior.unwrap_or(0.0);
    let posterior_margin = hypothesis_posterior_margin(summary);
    let evidence_highlights = ach_evidence_highlights(evidence_signals, 3);
    let (runner_up_label, runner_up_probability) = summary
        .all_posteriors
        .get(1)
        .map(|(label, probability)| (label.as_str(), *probability))
        .unwrap_or(("the next alternative", 0.0));

    if summary.total_evidence < 6
        || leading_posterior < 0.72
        || posterior_margin < 0.14
        || evidence_highlights.len() < 2
    {
        return None;
    }

    let leading_lower = leading.to_ascii_lowercase();
    let title_anchor = evidence_highlights[0]
        .split(" [")
        .next()
        .unwrap_or(evidence_highlights[0].as_str());
    let title = format!(
        "Hypothesis check: {} leans {} on {}",
        summary.entity_name,
        leading_lower,
        crate::truncate_text(title_anchor, 72)
    );

    let confidence_note = if summary.is_potentially_anchored() {
        summary
            .anchoring_warning()
            .unwrap_or_default()
            .trim_start_matches("⚠️ ")
            .to_string()
    } else {
        format!(
            "The lead over the runner-up is {:.0} points, so treat this as a directional read rather than a settled conclusion.",
            posterior_margin * 100.0
        )
    };
    let diagnostic_note = summary
        .top_diagnostic
        .as_deref()
        .unwrap_or("collect more disconfirming evidence before escalating this view")
        .trim_end_matches('.');
    let insight_summary = format!(
        "{} currently leans {} based on {} evidence signals. Most diagnostic evidence so far: {}. This view carries {:.0}% posterior probability versus {} at {:.0}%. {} Next discriminating evidence: {}.",
        summary.entity_name,
        leading_lower,
        summary.total_evidence,
        evidence_highlights.join("; "),
        leading_posterior * 100.0,
        runner_up_label,
        runner_up_probability * 100.0,
        confidence_note,
        diagnostic_note,
    );

    Some((
        title,
        insight_summary,
        leading_posterior,
        summary.total_evidence,
    ))
}

#[cfg(feature = "llm")]
struct BiasMitigationAdjustment {
    confidence_reduction: f64,
    tag: &'static str,
}

#[cfg(feature = "llm")]
fn apply_bias_mitigation(
    entity_name: &str,
    title: &str,
    summary: &str,
    evidence_signals: &[EvidenceSignal],
    stored_confidence: f64,
) -> Option<BiasMitigationAdjustment> {
    if evidence_signals.len() < 3 || stored_confidence < 0.55 {
        return None;
    }

    let bias_evidence: Vec<BiasEvidenceItem> = evidence_signals
        .iter()
        .map(|sig| BiasEvidenceItem {
            id: Uuid::new_v4(),
            description: format!("{}: {}", sig.signal_type, sig.title),
            source: sig.source_url.clone(),
            timestamp: chrono::Utc::now(),
            supports_claim: true,
            strength: sig.relevance_score as f64,
        })
        .collect();

    let claim = if title.trim().is_empty() {
        format!("Strategic assessment for {entity_name}")
    } else {
        title.trim().to_string()
    };
    let conclusion = summary
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(summary)
        .trim();

    let mut da_config = DevilsAdvocateConfig::default();
    da_config.generate_counter_narrative = false;

    let result = generate_devils_advocate(
        &claim,
        conclusion,
        &bias_evidence,
        Severity::High,
        &da_config,
    )?;

    let tag = if result.confidence_reduction >= 0.15 {
        "bias:strong_counter"
    } else {
        "bias:challenged"
    };

    Some(BiasMitigationAdjustment {
        confidence_reduction: result.confidence_reduction.clamp(0.05, 0.30),
        tag,
    })
}

#[cfg(feature = "llm")]
fn feedback_similarity_tokens(raw: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "into", "over", "under", "after",
        "before", "about", "their", "there", "have", "will", "would", "could", "should", "also",
        "just", "only", "more", "most", "been", "being", "same", "into", "onto",
    ];

    raw.to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|token| token.len() > 2 && !STOPWORDS.contains(token))
        .take(48)
        .map(|token| token.to_string())
        .collect()
}

#[cfg(feature = "llm")]
fn feedback_jaccard_similarity(left: &str, right: &str) -> f64 {
    let left_tokens: HashSet<String> = feedback_similarity_tokens(left).into_iter().collect();
    let right_tokens: HashSet<String> = feedback_similarity_tokens(right).into_iter().collect();
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let overlap = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union <= f64::EPSILON {
        0.0
    } else {
        overlap / union
    }
}

#[cfg(feature = "llm")]
fn parse_tracker_feedback(feedback_type: &str) -> Option<TrackerInsightFeedback> {
    match feedback_type.trim().to_ascii_lowercase().as_str() {
        "bookmarked" => Some(TrackerInsightFeedback::Bookmarked),
        "actioned" => Some(TrackerInsightFeedback::Actioned),
        "dismissed" => Some(TrackerInsightFeedback::Dismissed),
        "false_positive" => Some(TrackerInsightFeedback::FalsePositive),
        "false_negative" => Some(TrackerInsightFeedback::FalseNegative),
        "true_positive" => Some(TrackerInsightFeedback::TruePositive),
        "relevant" => Some(TrackerInsightFeedback::Relevant),
        "irrelevant" => Some(TrackerInsightFeedback::Irrelevant),
        "viewed" => Some(TrackerInsightFeedback::Viewed),
        _ => None,
    }
}

#[cfg(feature = "llm")]
fn is_semantic_duplicate_recent_insight(
    recent_rows: &[InsightRow],
    recipe_code: &str,
    category: &str,
    title: &str,
    summary: &str,
) -> bool {
    recent_rows.iter().any(|recent| {
        let same_recipe = recent
            .tags
            .as_ref()
            .map(|tags| tags.iter().any(|tag| tag.eq_ignore_ascii_case(recipe_code)))
            .unwrap_or(false);
        let same_category = recent
            .insight_type
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case(category))
            .unwrap_or(false);
        if !same_recipe && !same_category {
            return false;
        }

        let title_similarity = feedback_jaccard_similarity(&recent.title, title);
        let summary_similarity = feedback_jaccard_similarity(&recent.summary, summary);
        title_similarity >= *crate::config::DEDUP_TITLE_THRESHOLD
            && summary_similarity >= *crate::config::DEDUP_SUMMARY_THRESHOLD
    })
}

#[cfg(feature = "llm")]
async fn hydrate_feedback_tracker(
    store: &PgStore,
    now: DateTime<Utc>,
) -> (
    InsightFeedbackTracker,
    HashMap<String, f64>,
    HashMap<String, f64>,
) {
    let mut tracker = InsightFeedbackTracker::new();

    match store
        .list_recent_insight_feedback_events(now - chrono::Duration::days(90))
        .await
    {
        Ok(events) => {
            for event in events {
                let Some(entity_id) = event.entity_id else {
                    continue;
                };
                let Some(recipe_code) = event.recipe_code.clone() else {
                    continue;
                };
                let Some(feedback) = parse_tracker_feedback(&event.feedback_type) else {
                    continue;
                };
                tracker.record_feedback(InsightFeedbackRecord {
                    insight_id: event.insight_id.to_string(),
                    entity_id: entity_id.to_string(),
                    recipe_code,
                    feedback,
                    timestamp: event.created_at.timestamp(),
                    user_id: Some(event.user_id),
                    notes: event.notes,
                });
            }
        }
        Err(error) => {
            tracing::warn!(%error, "recipe_fire: failed to load insight feedback history")
        }
    }

    match store
        .list_recent_insight_firings(now - chrono::Duration::days(30))
        .await
    {
        Ok(firings) => {
            for firing in firings {
                tracker.record_firing(
                    &firing.entity_id.to_string(),
                    &firing.recipe_code,
                    &firing.insight_hash,
                );
            }
        }
        Err(error) => tracing::warn!(%error, "recipe_fire: failed to load insight firing history"),
    }

    let mut adaptive_thresholds = HashMap::new();
    for recommendation in tracker.recommend_threshold_adjustments() {
        adaptive_thresholds.insert(
            recommendation.recipe_code.to_ascii_uppercase(),
            recommendation.recommended_threshold.clamp(0.05, 0.60),
        );
    }

    let mut recipe_quality_scores = HashMap::new();
    for performance in tracker.get_all_performances() {
        recipe_quality_scores.insert(
            performance.recipe_code.to_ascii_uppercase(),
            performance.f1_score().clamp(0.0, 1.0),
        );
    }

    tracing::info!(
        feedback_recipes = recipe_quality_scores.len(),
        adaptive_thresholds = adaptive_thresholds.len(),
        fatigued_pairs = tracker.get_fatigued_combinations().len(),
        "recipe_fire: hydrated insight feedback tracker"
    );

    (tracker, adaptive_thresholds, recipe_quality_scores)
}

fn build_daily_entity_series(
    rows: &[(Uuid, i64, i64)],
    total_days: usize,
) -> HashMap<Uuid, Vec<f64>> {
    let mut series_by_entity: HashMap<Uuid, Vec<f64>> = HashMap::new();
    for (entity_id, day_offset, count) in rows {
        if *day_offset < 0 {
            continue;
        }
        let index = *day_offset as usize;
        if index >= total_days {
            continue;
        }
        let series = series_by_entity
            .entry(*entity_id)
            .or_insert_with(|| vec![0.0; total_days]);
        series[index] += *count as f64;
    }
    series_by_entity
}

fn parse_feature_vector_map(value: &serde_json::Value) -> HashMap<String, f64> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| value.as_f64().map(|score| (key.clone(), score)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default()
}

fn recipe_warning_severity(
    recipe_code: &str,
    category: &str,
    candidate_severity: &str,
    confidence: f64,
    impact: f64,
) -> Option<&'static str> {
    let normalized_category = category.trim().to_ascii_lowercase();
    let normalized_candidate_severity = candidate_severity.trim().to_ascii_lowercase();
    let normalized_recipe_code = recipe_code.trim().to_ascii_uppercase();
    let is_dns_hygiene_recipe = matches!(normalized_recipe_code.as_str(), "N001" | "N002");

    // Reject impossible severity values
    if !matches!(
        normalized_candidate_severity.as_str(),
        "critical" | "warning" | "high" | "medium" | "low" | "info"
    ) {
        tracing::warn!(
            recipe_code,
            severity = candidate_severity,
            "recipe_warning_severity: unrecognized severity, rejecting"
        );
        return None;
    }

    if normalized_candidate_severity == "critical" && confidence >= 0.82 {
        return Some("critical");
    }

    if normalized_candidate_severity == "warning" {
        if is_dns_hygiene_recipe {
            return (confidence >= 0.92 && impact >= 0.75).then_some("medium");
        }

        let severity = if confidence >= 0.9 || impact >= 1.1 {
            "high"
        } else {
            "medium"
        };
        return (confidence >= 0.75).then_some(severity);
    }

    let promoted_category = matches!(
        normalized_category.as_str(),
        "demand_procurement"
            | "competitor_market"
            | "supply_chain_risk"
            | "regulatory_policy"
            | "cybersecurity_threat"
            | "competitive_comparison"
            | "talent_movement"
            | "market_intelligence"
            | "technology_innovation"
            | "geopolitical_risk"
    ) || normalized_category.contains("compliance")
        || normalized_category.contains("sanction")
        || normalized_category.contains("security")
        || normalized_category.contains("competitive")
        || normalized_category.contains("supply_chain")
        || normalized_category.contains("procurement");

    if !promoted_category {
        return None;
    }

    let min_confidence = match normalized_category.as_str() {
        "regulatory_policy" | "supply_chain_risk" => 0.62,
        "demand_procurement" | "competitor_market" => 0.65,
        "cybersecurity_threat" => 0.70,
        _ => 0.60,
    };
    let min_impact = match normalized_category.as_str() {
        "demand_procurement" | "competitor_market" => 0.40,
        "regulatory_policy" | "supply_chain_risk" => 0.35,
        "cybersecurity_threat" => 0.55,
        _ => 0.35,
    };

    if confidence < min_confidence || impact < min_impact {
        return None;
    }

    Some(match normalized_category.as_str() {
        "regulatory_policy" | "supply_chain_risk" | "cybersecurity_threat" => "high",
        _ => "medium",
    })
}

#[allow(clippy::disallowed_methods)]
pub(super) async fn run_recipe_fire(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let now = Utc::now();
    let mut source_reliability_scores = HashMap::new();

    #[cfg(feature = "llm")]
    let insight_llm_client = {
        let base_url =
            std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
        let api_key = std::env::var("LLM_API_KEY").ok();
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
        let mut config = apex_llm::inference::InferenceConfig::default();
        config.model = model;
        config.max_tokens = 2048;
        config.timeout = crate::config::llm_timeout();
        InferenceLlmClient::new(base_url, api_key, config)
    };

    let seed_recipes = load_default_seed_recipes().unwrap_or_default();
    let engine_recipes: Vec<Recipe> = seed_recipes
        .iter()
        .filter(|sr| !sr.narrative_template.is_empty() && !sr.signals.is_empty())
        .map(seed_recipe_to_engine_recipe)
        .collect();

    if engine_recipes.is_empty() {
        run.skip("recipe_fire: no seed recipes with signals/templates available");
        return run;
    }

    let engine = RecipeEngine::load(engine_recipes);

    let since = now - chrono::Duration::days(30);
    if let Err(error) = store.resolve_stats_alert_calibration_events(now).await {
        tracing::warn!(%error, "recipe_fire: failed to resolve mature stats alert calibration events");
    }
    let resolved_calibration_rows = store
        .list_resolved_stats_alert_calibration_samples(
            Some(now - chrono::Duration::days(365)),
            5_000,
        )
        .await
        .unwrap_or_default();
    let calibration_samples = resolved_calibration_rows
        .iter()
        .map(|row| CalibrationSample {
            raw_score: alert_score_from_features(&parse_feature_vector_map(&row.feature_vector)),
            actual_outcome: row.actual_outcome_within_30d.unwrap_or(false),
        })
        .collect::<Vec<_>>();
    let alert_calibration_model = fit_best_alert_calibration_model(&calibration_samples);
    if !calibration_samples.is_empty() {
        let reliability_bins =
            calibration_curve(&calibration_samples, &alert_calibration_model, 10);
        let rank_correlation =
            alert_score_rank_correlation(&calibration_samples, &alert_calibration_model);
        let detail = serde_json::json!({
            "week_start": now.date_naive().to_string(),
            "sample_count": calibration_samples.len(),
            "calibration_method": alert_calibration_model.method_name(),
            "rank_correlation": rank_correlation,
            "bins": reliability_bins,
        });
        if let Err(error) = store
            .record_audit_event("system", "stats_alert_calibration_curve_published", &detail)
            .await
        {
            tracing::warn!(%error, "recipe_fire: failed to publish calibration curve artifact");
        }
    }
    let source_reliability_aggregates = store
        .aggregate_source_reliability_outcomes(None)
        .await
        .unwrap_or_default();
    for aggregate in source_reliability_aggregates {
        let observation_count = aggregate.observation_count.max(0) as u64;
        let confirmed_count = aggregate.confirmed_count.max(0) as u64;
        let tier = SourceReliability::from_url(&aggregate.source_domain);
        let stats = build_source_reliability_stats(
            aggregate.source_domain.clone(),
            tier,
            observation_count,
            confirmed_count,
        );
        if let Err(error) = store
            .upsert_source_reliability_stat(
                &stats.source_domain,
                stats.tier.as_str(),
                stats.observation_count as i64,
                stats.confirmed_count as i64,
                stats.observed_reliability,
                stats.effective_reliability,
                stats.promotion_recommended,
                now,
            )
            .await
        {
            tracing::warn!(
                %error,
                source_domain = %stats.source_domain,
                "recipe_fire: failed to persist source reliability stat"
            );
            continue;
        }
        source_reliability_scores.insert(stats.source_domain.clone(), stats.effective_reliability);
    }
    let pending_source_promotions = store
        .list_pending_source_reliability_promotions(50)
        .await
        .unwrap_or_default();
    if !pending_source_promotions.is_empty() {
        let analyst_users = store
            .list_analyst_users()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|user| user.is_active)
            .collect::<Vec<_>>();
        for promotion in pending_source_promotions {
            let detail = serde_json::json!({
                "source_domain": promotion.source_domain,
                "tier": promotion.tier,
                "observation_count": promotion.observation_count,
                "confirmed_count": promotion.confirmed_count,
                "observed_reliability": promotion.observed_reliability,
                "effective_reliability": promotion.effective_reliability,
            });
            if let Err(error) = store
                .record_audit_event("system", "source_reliability_promotion_candidate", &detail)
                .await
            {
                tracing::warn!(
                    %error,
                    source_domain = %promotion.source_domain,
                    "recipe_fire: failed to publish source promotion audit event"
                );
            }
            for analyst in &analyst_users {
                let title = format!("Promote source candidate: {}", promotion.source_domain);
                let body = format!(
                    "Observed reliability {:.2} over {} observations ({} confirmed) exceeds the promotion threshold for unknown sources.",
                    promotion.observed_reliability,
                    promotion.observation_count,
                    promotion.confirmed_count
                );
                if let Err(error) = store
                    .create_notification(
                        &analyst.id,
                        "source_reliability",
                        &title,
                        &body,
                        None,
                        None,
                        None,
                    )
                    .await
                {
                    tracing::warn!(
                        %error,
                        user_id = %analyst.id,
                        source_domain = %promotion.source_domain,
                        "recipe_fire: failed to create source promotion notification"
                    );
                }
            }
            if let Err(error) = store
                .mark_source_reliability_promotion_alerted(&promotion.source_domain, now)
                .await
            {
                tracing::warn!(
                    %error,
                    source_domain = %promotion.source_domain,
                    "recipe_fire: failed to mark source promotion alert"
                );
            }
        }
    }
    let obs_counts = match store.get_obs_type_counts_per_entity(since).await {
        Ok(v) => v,
        Err(e) => {
            run.fail(&format!(
                "recipe_fire: failed to fetch observation counts: {e}"
            ));
            return run;
        }
    };

    let warn_counts = store
        .get_warning_type_counts_per_entity(since)
        .await
        .unwrap_or_default();
    let ce_features = store
        .get_competitor_event_features(since)
        .await
        .unwrap_or_default();
    let wc_features = store
        .get_webchange_jsonb_features(since)
        .await
        .unwrap_or_default();
    let wc_kw_features = store
        .get_webchange_keyword_features(since)
        .await
        .unwrap_or_default();
    let person_feats = store
        .get_person_features_per_company()
        .await
        .unwrap_or_default();
    let cert_feats = store
        .get_certification_features_per_company(since)
        .await
        .unwrap_or_default();
    let cap_feats = store
        .get_capability_features_per_company()
        .await
        .unwrap_or_default();
    let site_feats = store
        .get_site_features_per_company()
        .await
        .unwrap_or_default();
    let graph_feats = store.get_graph_edge_features().await.unwrap_or_default();
    let job_post_feats = store
        .get_job_post_features_per_entity(since)
        .await
        .unwrap_or_default();
    let commodity_fx_feats = store
        .get_commodity_fx_features_per_entity(since)
        .await
        .unwrap_or_default();
    let poi_artifact_feats = store
        .get_poi_artifact_features_per_company(since)
        .await
        .unwrap_or_default();
    let daily_obs_series = store
        .get_daily_observation_counts_per_entity(since)
        .await
        .unwrap_or_default();
    let daily_warning_series = store
        .get_daily_warning_counts_per_entity(since)
        .await
        .unwrap_or_default();
    let observation_series_by_entity = build_daily_entity_series(&daily_obs_series, 31);
    let warning_series_by_entity = build_daily_entity_series(&daily_warning_series, 31);

    if obs_counts.is_empty() && warn_counts.is_empty() {
        run.skip("recipe_fire: no observations or warnings in last 30 days");
        return run;
    }

    let mut entity_maps: HashMap<Uuid, FeatureMap> = HashMap::new();

    for (entity_id, obs_type, count) in &obs_counts {
        let fm = entity_maps.entry(*entity_id).or_default();
        let count_f = *count as f64;
        fm.insert(format!("{obs_type}.count"), count_f);
        fm.insert(format!("{obs_type}.any"), count_f);

        match obs_type.as_str() {
            "lookalike_domain" => {
                for k in &[
                    "LookalikeDomain.count",
                    "LookalikeDomain.active",
                    "LookalikeDomain.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
                if count_f >= 2.0 {
                    *fm.entry("Security.risk".into()).or_default() += 1.0;
                }
            }
            "dns_posture" => {
                for k in &["DNSPosture.degraded", "DNSPosture.count"] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "kev_match" => {
                for k in &[
                    "KEV.match",
                    "KEV.count",
                    "Security.vulnerability",
                    "Security.risk",
                    "Security.count",
                    "Compliance.risk",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "SocialPost" => {
                for k in &[
                    "SocialPost.count",
                    "SocialPost.sentiment",
                    "SocialPost.any",
                    "News.count",
                    "News.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "PersonMove" => {
                for k in &[
                    "PersonMention.role_change",
                    "PersonMention.count",
                    "RoleChange.count",
                    "RoleChange.any",
                    "SocialSignal.leadership_change",
                    "JobPost.executive.new_function",
                    "POI.role_change.imminent",
                    "POI.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "TenderNotice" => {
                for k in &[
                    "Tender.count",
                    "Tender.any",
                    "Tender.public.posted",
                    "Procurement.count",
                    "Procurement.any",
                    "Contract.count",
                    "Demand.count",
                    "Demand.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "PatentPublication" => {
                for k in &[
                    "Patent.count",
                    "Patent.any",
                    "Patent.recent",
                    "PatentPublished.competitor.cluster",
                    "PatentPublished.technology_overlap",
                    "IP.count",
                    "IP.any",
                    "Technology.count",
                    "Technology.any",
                    "Innovation.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "RegulatoryFiling" => {
                for k in &[
                    "Regulatory.count",
                    "Regulatory.change",
                    "Compliance.count",
                    "Compliance.any",
                    "Compliance.risk",
                    "Filing.count",
                    "Filing.any",
                    "Policy.count",
                    "Policy.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "FinancialDisclosure" => {
                for k in &[
                    "Filing.count",
                    "Filing.any",
                    "Company.filing.new",
                    "Company.earnings.call",
                    "CompanyProfile.count",
                    "Industry.count",
                    "Industry.trend",
                    "Market.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "CompetitorEvent" => {
                for k in &[
                    "Competitor.count",
                    "Competitor.activity",
                    "CompetitorEvent.count",
                    "Industry.count",
                    "Industry.trend",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            // ── Observation types previously unmapped ──────────────────
            "JobPost" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "JobPost.volume.anomaly",
                    "Demand.hiring",
                    "Demand.count",
                    "Demand.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "CertificationUpdate" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "Certification.count",
                    "Certification.any",
                    "Compliance.certification",
                    "Compliance.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "WebChange" => {
                for k in &["WebChange.count", "WebChange.any", "News.count", "News.any"] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "CommodityPrice" => {
                for k in &[
                    "CommodityPrice.shift",
                    "CommodityPrice.significant_move",
                    "Commodity.price.volatile",
                    "Commodity.count",
                    "Commodity.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "FxRate" => {
                for k in &[
                    "FxRate.significant_move",
                    "FxRate.volatility.high",
                    "FxRate.any",
                    "FxRate.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "PortMetric" => {
                for k in &[
                    "PortMetric.delay_increase",
                    "SupplyChain.disruption",
                    "SupplyChain.count",
                    "Supplier.risk",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "NewDomain" => {
                for k in &[
                    "DNS.typosquat.new",
                    "LookalikeDomain.count",
                    "LookalikeDomain.active",
                    "Security.risk",
                    "Security.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "VulnNotice" => {
                for k in &[
                    "VulnNotice.data_breach",
                    "Security.vulnerability",
                    "Security.risk",
                    "Security.count",
                    "Compliance.risk",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "PersonMention" => {
                for k in &[
                    "PersonMention.count",
                    "POI.media.presence",
                    "POI.visibility.high",
                    "POI.count",
                    "News.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "RoleChange" => {
                for k in &[
                    "RoleChange.count",
                    "RoleChange.any",
                    "RoleChange.competitor_destination",
                    "POI.role_change.imminent",
                    "PersonMention.role_change",
                    "SocialSignal.leadership_change",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "SpeakerAppearance" => {
                for k in &[
                    "TradeShow.speaker.poi_match",
                    "TradeShow.presence.increase",
                    "ConferenceAgenda.count",
                    "POI.conference.speaker",
                    "POI.thought_leadership",
                    "POI.visibility.high",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "ProcurementSignal" => {
                for k in &[
                    "Procurement.count",
                    "Procurement.any",
                    "SocialSignal.procurement_announcement",
                    "Demand.count",
                    "Demand.any",
                    "Tender.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            _ => {}
        }
    }

    for (entity_id, wtype, count) in &warn_counts {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        fm.insert(format!("Warning.{wtype}"), c);

        match wtype.as_str() {
            "certification_update" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "Certification.count",
                    "Certification.any",
                    "Compliance.certification",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "hiring_signal" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "Demand.hiring",
                    "Demand.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "ma_activity" => {
                for k in &[
                    "Competitor.ma_activity",
                    "Competitor.acquisition",
                    "Company.acquisition",
                    "Company.M_A",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "expansion" => {
                for k in &[
                    "Company.expansion",
                    "Company.investment",
                    "Facility.new",
                    "Facility.count",
                    "Production.site.change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Patent.count",
                    "Patent.any",
                    "Innovation.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply_chain_disruption" | "supply_chain" => {
                for k in &[
                    "SupplyChain.disruption",
                    "SupplyChain.count",
                    "Supplier.risk",
                    "Supplier.count",
                    "Material.shortage",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical_risk" | "geopolitical" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Sanctions.count",
                    "Sanctions.risk",
                    "Trade.restriction",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "competitive" => {
                for k in &["Competitor.count", "Competitor.activity", "Industry.trend"] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "compliance" => {
                for k in &[
                    "Compliance.count",
                    "Compliance.risk",
                    "Regulatory.count",
                    "Regulatory.change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "market_intelligence" => {
                for k in &[
                    "Market.count",
                    "Market.intelligence",
                    "Industry.count",
                    "Industry.trend",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "procurement" => {
                for k in &[
                    "Procurement.count",
                    "Procurement.any",
                    "Tender.count",
                    "Tender.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "relationship" => {
                for k in &["Relationship.count", "Relationship.any", "Connection.count"] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "talent_movement" | "talent_migration" | "key_hire" => {
                for k in &[
                    "PersonMention.role_change",
                    "PersonMention.count",
                    "RoleChange.competitor_destination",
                    "RoleChange.count",
                    "SocialSignal.leadership_change",
                    "JobPost.executive.new_function",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "patent" | "patent_filing" | "ip_filing" => {
                for k in &[
                    "PatentPublished.competitor.cluster",
                    "PatentPublished.technology_overlap",
                    "PatentPublished.litigation.filed",
                    "PatentPublished.university_collab",
                    "Patent.count",
                    "Patent.any",
                    "IP.count",
                    "IP.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "leadership_change" | "executive_change" => {
                for k in &[
                    "SocialSignal.leadership_change",
                    "SocialSignal.ip_dispute",
                    "PersonMention.role_change",
                    "RoleChange.competitor_destination",
                    "JobPost.executive.new_function",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            other => {
                let capitalized = capitalize_first(other);
                *fm.entry(format!("{capitalized}.count")).or_default() += c;
                *fm.entry(format!("{capitalized}.any")).or_default() += c;
            }
        }
    }

    for (entity_id, signal_type, keyword, count) in &ce_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        if !signal_type.is_empty() {
            *fm.entry(format!("Competitor.{signal_type}")).or_default() += c;
            *fm.entry("Competitor.count".into()).or_default() += c;
        }
        if !keyword.is_empty() {
            *fm.entry(format!("Competitor.{keyword}")).or_default() += c;
        }
    }

    for (entity_id, source_id, signal_type, count) in &wc_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;

        match source_id.as_str() {
            "ofac_sanctions" | "eu_sanctions" | "un_sanctions" => {
                for k in &[
                    "Sanctions.count",
                    "Sanctions.any",
                    "Sanctions.list",
                    "Sanctions.screening.match",
                    "Sanctions.risk",
                    "OFAC.count",
                    "OFAC.any",
                    "OFAC.match",
                    "Compliance.sanctions",
                    "Trade.count",
                    "Trade.any",
                    "Embargo.count",
                    "Embargo.any",
                    "ExportControl.count",
                    "ExportControl.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "uspto_patents" | "epo_patents" | "wipo_patents" => {
                for k in &[
                    "Patent.count",
                    "Patent.any",
                    "Patent.recent",
                    "Patent.filing",
                    "Patent.competitor",
                    "Technology.patent",
                    "Technology.count",
                    "Technology.any",
                    "IP.count",
                    "IP.any",
                    "PatentPublished.competitor.cluster",
                    "PatentPublished.technology_overlap",
                    "PatentPublished.litigation.filed",
                    "PatentPublished.university_collab",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sam_gov" | "ted_eu" | "dgmarket" => {
                for k in &[
                    "Tender.count",
                    "Tender.any",
                    "Tender.public",
                    "Tender.posted",
                    "Tender.public.posted",
                    "Tender.framework_agreement",
                    "Tender.sector",
                    "Tender.region",
                    "Procurement.count",
                    "Procurement.any",
                    "Contract.count",
                    "Contract.any",
                    "Government.count",
                    "Government.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sec_edgar" | "sec_filings" => {
                for k in &[
                    "Filing.count",
                    "Filing.any",
                    "Filing.recent",
                    "Company.filing",
                    "Company.count",
                    "Company.any",
                    "CompanyProfile.count",
                    "CompanyProfile.any",
                    "CompanyProfile.revenue",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "defense_news" | "jane_defence" | "janes_defence" => {
                for k in &[
                    "Defense.count",
                    "Defense.any",
                    "Security.defense",
                    "Security.count",
                    "Security.any",
                    "News.count",
                    "News.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "bloomberg_global" | "ft_global" | "nyt_us" | "axios_us" | "politico_us"
            | "the_hill" | "afp_global" => {
                for k in &[
                    "News.count",
                    "News.any",
                    "News.geopolitical",
                    "News.industry",
                    "News.competitor",
                    "PressRelease.count",
                    "PressRelease.any",
                    "Industry.count",
                    "Industry.any",
                    "Market.count",
                    "Market.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "foreign_affairs" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Geopolitical.any",
                    "News.geopolitical",
                    "News.count",
                    "News.any",
                    "Trade.count",
                    "Trade.any",
                    "Regional.risk.high",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {
                if !source_id.is_empty() {
                    *fm.entry(format!("WebChange.{source_id}")).or_default() += c;
                }
            }
        }

        match signal_type.as_str() {
            "hiring_signal" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "JobPost.role_family",
                    "Demand.hiring",
                    "JobPost.executive.new_function",
                    "SocialSignal.leadership_change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "certification_update" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "CertificationUpdate.new",
                    "Certification.count",
                    "Certification.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Innovation.count",
                    "Innovation.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply_chain_disruption" => {
                for k in &[
                    "SupplyChain.count",
                    "SupplyChain.disruption",
                    "Supplier.risk",
                    "Supplier.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical_risk" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Security.risk",
                    "Security.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {}
        }
    }

    for (entity_id, keyword, count) in &wc_kw_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        match keyword.as_str() {
            "patent" => {
                for k in &[
                    "Patent.count",
                    "Patent.any",
                    "Patent.recent",
                    "IP.count",
                    "IP.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "innovation" | "breakthrough" | "next-generation" | "new technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Technology.emerging",
                    "Innovation.count",
                    "Innovation.any",
                    "Industry40.count",
                    "Industry40.any",
                    "Digital.count",
                    "Digital.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "r&d" | "research and development" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Engineering.count",
                    "Engineering.any",
                    "Competitor.R_D",
                    "Product.development.early",
                    "SocialSignal.research_partnership",
                    "PatentPublished.university_collab",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "career" | "hiring" | "job opening" | "join our team" | "open position" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "JobPost.volume",
                    "Demand.hiring",
                    "Demand.count",
                    "Demand.any",
                    "JobPost.executive.new_function",
                    "SocialSignal.leadership_change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "certification" | "accreditation" | "iso 9001" | "iso 14001" | "iso 13485"
            | "iso 27001" | "as9100" | "iatf 16949" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "CertificationUpdate.new",
                    "Certification.count",
                    "Certification.any",
                    "Certification.new",
                    "Compliance.count",
                    "Compliance.any",
                    "Compliance.certification",
                    "Audit.count",
                    "Audit.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
                match keyword.as_str() {
                    "iatf 16949" => {
                        *fm.entry("CertificationUpdate.new.IATF_16949".into())
                            .or_default() += c;
                    }
                    "as9100" => {
                        *fm.entry("CertificationUpdate.new.AS9100".into())
                            .or_default() += c;
                    }
                    "iso 13485" => {
                        *fm.entry("CertificationUpdate.new.ISO_13485".into())
                            .or_default() += c;
                    }
                    "iso 27001" => {
                        *fm.entry("CertificationUpdate.new.ISO_27001".into())
                            .or_default() += c;
                    }
                    _ => {}
                }
            }
            "compliance" | "audit" => {
                for k in &[
                    "Compliance.count",
                    "Compliance.any",
                    "Compliance.risk",
                    "Regulatory.count",
                    "Regulatory.any",
                    "Audit.count",
                    "Audit.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sanctions" => {
                for k in &[
                    "Sanctions.count",
                    "Sanctions.any",
                    "Sanctions.list",
                    "OFAC.count",
                    "OFAC.any",
                    "Compliance.sanctions",
                    "Trade.count",
                    "Trade.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "tariff" => {
                for k in &[
                    "Tariff.count",
                    "Tariff.any",
                    "Tariff.change.announced",
                    "Tariff.reduction",
                    "Trade.count",
                    "Trade.any",
                    "Trade.restriction",
                    "Customs.count",
                    "Customs.any",
                    "Import.count",
                    "Import.any",
                    "ImportData.count",
                    "ImportData.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "shortage" | "allocation" | "lead time" => {
                for k in &[
                    "SupplyChain.count",
                    "SupplyChain.any",
                    "SupplyChain.lead_time.increase",
                    "Supplier.lead_time.increase",
                    "Supplier.capacity.reduced",
                    "Material.shortage",
                    "Material.count",
                    "Material.any",
                    "Commodity.shortage",
                    "Commodity.count",
                    "Commodity.any",
                    "Semiconductor.lead_time.surge",
                    "Inventory.count",
                    "Inventory.any",
                    "Component.count",
                    "Component.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply chain disruption" => {
                for k in &[
                    "SupplyChain.disruption",
                    "SupplyChain.count",
                    "SupplyChain.any",
                    "Supplier.risk",
                    "Supplier.count",
                    "Supplier.any",
                    "Logistics.disruption",
                    "Logistics.count",
                    "Logistics.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Geopolitical.any",
                    "Security.risk",
                    "Security.count",
                    "Security.any",
                    "Regional.risk.high",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "acquisition" | "acquired" | "joint venture" | "strategic partnership" => {
                for k in &[
                    "Company.acquisition",
                    "Company.count",
                    "Company.any",
                    "Competitor.acquisition",
                    "Competitor.ma_activity",
                    "Post_MA.integration",
                    "Post_MA.integration.issues",
                    "Partnership.strategic",
                    "PressRelease.acquisition",
                    "PressRelease.JV_announced",
                    "NewEntrant.count",
                    "NewEntrant.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "investment in" | "new facility" | "new manufacturing" | "grand opening"
            | "groundbreaking" => {
                for k in &[
                    "Company.expansion",
                    "Company.investment",
                    "Company.count",
                    "Company.any",
                    "Competitor.expansion",
                    "Competitor.factory",
                    "Facility.new",
                    "Facility.count",
                    "Facility.any",
                    "Production.site.change",
                    "Production.count",
                    "Production.any",
                    "PressRelease.expansion",
                    "Infrastructure.count",
                    "Infrastructure.any",
                    "Growth.count",
                    "Growth.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "product launch" => {
                for k in &[
                    "Product.count",
                    "Product.any",
                    "Product.development.early",
                    "Product.supply_chain.new",
                    "Competitor.product",
                    "Competitor.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {
                if !keyword.is_empty() {
                    let kw_clean = keyword.replace(' ', "_");
                    *fm.entry(format!("WebChange.kw_{kw_clean}")).or_default() += c;
                }
            }
        }
    }

    for (company_id, role_family, influence, pain, change_risk, count) in &person_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("POI.count".into()).or_default() += c;
        *fm.entry("POI.any".into()).or_default() += c;

        if *influence > 0.7 {
            for k in &[
                "POI.influence.broad",
                "POI.influence.expanding",
                "POI.influence.external",
                "POI.influence.chain",
                "POI.strategic.influence",
                "POI.visibility.high",
                "POI.spec.influence",
                "POI.stakeholder.map.complete",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }

        if *pain > 0.6 {
            for k in &[
                "POI.pain_index.high",
                "POI.pain.cost.expressed",
                "POI.pain.delivery.expressed",
                "POI.pain.quality.expressed",
                "POI.pain.flexibility.expressed",
                "POI.frustration.supplier",
                "POI.frustration.internal",
                "POI.lead_time.concern",
                "POI.reliability.priority",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }
        if *pain > 0.8 {
            *fm.entry("POI.pain_index.very_high".into()).or_default() += c;
        }

        if *change_risk > 0.5 {
            for k in &[
                "POI.role_change.CPO.new",
                "POI.scope.expanded",
                "POI.promotion.detected",
                "POI.milestone.career",
                "POI.project.new",
                "POI.team.building",
                "Decision.count",
                "Decision.any",
                "Decision.imminent",
                "Decision.budget.allocated",
                "Decision.committee.formed",
                "PersonMention.role_change",
                "PersonMention.count",
                "RoleChange.competitor_destination",
                "RoleChange.count",
                "SocialSignal.leadership_change",
                "JobPost.executive.new_function",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }

        match role_family.as_str() {
            "C-Suite" => {
                for k in &[
                    "POI.strategic.influence",
                    "POI.visibility.high",
                    "POI.thought_leadership",
                    "POI.media.presence",
                    "POI.network.broad",
                    "POI.social.active",
                    "Decision.executive",
                    "Succession.identified",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "Government" | "Agency Head" => {
                for k in &[
                    "POI.region.visiting",
                    "Government.count",
                    "Government.any",
                    "Regulatory.count",
                    "Regulatory.any",
                    "POI.knowledge.gap",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "Industry Association" | "Industry Analyst" => {
                for k in &[
                    "POI.thought_leadership",
                    "POI.media.presence",
                    "POI.network.broad",
                    "Industry.count",
                    "Industry.any",
                    "ConferenceAgenda.count",
                    "ConferenceAgenda.any",
                    "Event.count",
                    "Event.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {}
        }

        for k in &[
            "POI.location.nearby",
            "POI.operations.experience",
            "Multi_POI.count",
            "Multi_POI.any",
            "Trigger.count",
            "Trigger.any",
        ] {
            *fm.entry(k.to_string()).or_default() += c;
        }
    }

    for (company_id, standard, count) in &cert_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("Certification.count".into()).or_default() += c;
        *fm.entry("Certification.any".into()).or_default() += c;
        *fm.entry("CertificationUpdate.count".into()).or_default() += c;
        *fm.entry("CertificationUpdate.any".into()).or_default() += c;

        let std_upper = standard.to_uppercase();
        if std_upper.contains("IATF") || std_upper.contains("16949") {
            *fm.entry("CertificationUpdate.new.IATF_16949".into())
                .or_default() += c;
            *fm.entry("Tender.sector=automotive".into()).or_default() += c;
        }
        if std_upper.contains("AS9100")
            || std_upper.contains("AS 9100")
            || std_upper.contains("EN 9100")
        {
            *fm.entry("CertificationUpdate.new.AS9100".into())
                .or_default() += c;
            *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
        }
        if std_upper.contains("13485") {
            *fm.entry("CertificationUpdate.new.ISO_13485".into())
                .or_default() += c;
            *fm.entry("Tender.sector=medical".into()).or_default() += c;
        }
        if std_upper.contains("14001") {
            *fm.entry("Environmental.count".into()).or_default() += c;
            *fm.entry("ESG.count".into()).or_default() += c;
        }
        if std_upper.contains("27001") {
            *fm.entry("CertificationUpdate.new.ISO_27001".into())
                .or_default() += c;
            *fm.entry("Security.count".into()).or_default() += c;
            *fm.entry("Compliance.cybersecurity".into()).or_default() += c;
        }
    }

    for (company_id, capability, count) in &cap_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("Capability.count".into()).or_default() += c;
        *fm.entry("Capability.any".into()).or_default() += c;
        *fm.entry("Technology.count".into()).or_default() += c;

        let cap_lower = capability.to_lowercase();
        if cap_lower.contains("smt") || cap_lower.contains("pcb") {
            *fm.entry("Competitor.careers.SMT".into()).or_default() += c;
            *fm.entry("Competitor.capability_page.changed".into())
                .or_default() += c;
        }
        if cap_lower.contains("medical") {
            *fm.entry("Tender.sector=medical".into()).or_default() += c;
        }
        if cap_lower.contains("automotive") {
            *fm.entry("Tender.sector=automotive".into()).or_default() += c;
        }
        if cap_lower.contains("aerospace")
            || cap_lower.contains("avionics")
            || cap_lower.contains("satellite")
        {
            *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
            *fm.entry("Defense.count".into()).or_default() += c;
        }
        if cap_lower.contains("iot") || cap_lower.contains("embedded") {
            *fm.entry("Industry40.count".into()).or_default() += c;
            *fm.entry("Digital.count".into()).or_default() += c;
        }
        if cap_lower.contains("prototype") || cap_lower.contains("testing") {
            *fm.entry("Product.development.early".into()).or_default() += c;
        }
    }

    for (company_id, country_code, site_type, count) in &site_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;

        *fm.entry("Production.count".into()).or_default() += c;
        *fm.entry("Production.any".into()).or_default() += c;
        *fm.entry("Facility.count".into()).or_default() += c;
        *fm.entry("Facility.any".into()).or_default() += c;

        if !country_code.is_empty() {
            *fm.entry(format!("Geographic.{country_code}")).or_default() += c;
            *fm.entry("SupplyChain.geo.concentrated".into()).or_default() += c;
        }
        if !site_type.is_empty() {
            *fm.entry(format!("Facility.{site_type}")).or_default() += c;
            *fm.entry("Production.site.change".into()).or_default() += c;
        }
    }

    for (source_id, edge_type, count) in &graph_feats {
        let fm = entity_maps.entry(*source_id).or_default();
        let c = *count as f64;

        *fm.entry("Connection.count".into()).or_default() += c;
        *fm.entry("Relationship.count".into()).or_default() += c;
        *fm.entry("Ecosystem.count".into()).or_default() += c;

        match edge_type.as_str() {
            "CompanyCompany" => {
                *fm.entry("Competitor.count".into()).or_default() += c;
                *fm.entry("Vendor.count".into()).or_default() += c;
                *fm.entry("Vendor.any".into()).or_default() += c;
                *fm.entry("Supplier.count".into()).or_default() += c;
                *fm.entry("Customer.count".into()).or_default() += c;
                *fm.entry("Customer.any".into()).or_default() += c;
                *fm.entry("Partnership.strategic".into()).or_default() += c;
                *fm.entry("Distributor.count".into()).or_default() += c;
                *fm.entry("Distributor.any".into()).or_default() += c;
            }
            "CompanyPerson" => {
                *fm.entry("POI.count".into()).or_default() += c;
                *fm.entry("POI.any".into()).or_default() += c;
                *fm.entry("Alumni.count".into()).or_default() += c;
                *fm.entry("Alumni.any".into()).or_default() += c;
                *fm.entry("Alumni.connection".into()).or_default() += c;
                *fm.entry("Connection.mutual".into()).or_default() += c;
            }
            "leads" => {
                *fm.entry("POI.influence.chain".into()).or_default() += c;
                *fm.entry("Relationship.new".into()).or_default() += c;
                *fm.entry("Relationship.strengthened".into()).or_default() += c;
            }
            "CompanySite" => {
                *fm.entry("Facility.count".into()).or_default() += c;
                *fm.entry("Production.count".into()).or_default() += c;
            }
            "SiteLogistics" => {
                *fm.entry("SupplyChain.count".into()).or_default() += c;
                *fm.entry("PortMetric.delay_increase".into()).or_default() += c;
            }
            "CompanyCapability" => {
                *fm.entry("Capability.count".into()).or_default() += c;
                *fm.entry("Technology.count".into()).or_default() += c;
            }
            "VulnProduct" => {
                *fm.entry("Security.vulnerability".into()).or_default() += c;
                *fm.entry("Product.risk".into()).or_default() += c;
            }
            "CompanyRegulation" => {
                *fm.entry("Regulatory.count".into()).or_default() += c;
                *fm.entry("Compliance.count".into()).or_default() += c;
            }
            "PersonPatent" => {
                *fm.entry("Patent.count".into()).or_default() += c;
                *fm.entry("IP.count".into()).or_default() += c;
            }
            "PersonEvent" => {
                *fm.entry("ConferenceAgenda.count".into()).or_default() += c;
                *fm.entry("TradeShow.presence.increase".into()).or_default() += c;
            }
            _ => {}
        }
    }

    // ── Job-post payload feature enrichment ───────────────────────
    // Recipes reference JobPost.role_family, JobPost.bilingual.*,
    // JobPost.competitor.decrease, JobPost.new_region — all need
    // structured data from the job post JSONB payloads.
    for (entity_id, role_family, seniority, count) in &job_post_feats {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        *fm.entry("JobPost.count".into()).or_default() += c;
        *fm.entry("JobPost.any".into()).or_default() += c;
        *fm.entry("Demand.hiring".into()).or_default() += c;
        *fm.entry("Demand.count".into()).or_default() += c;
        if !role_family.is_empty() {
            *fm.entry("JobPost.role_family".to_string()).or_default() += c;
            *fm.entry(format!("JobPost.role_family.{role_family}"))
                .or_default() += c;
            let rf_lower = role_family.to_lowercase();
            if rf_lower.contains("procurement") || rf_lower.contains("sourcing") {
                *fm.entry("Demand.hiring".into()).or_default() += c;
                *fm.entry("SocialSignal.procurement_announcement".into())
                    .or_default() += c;
            }
            if rf_lower.contains("executive") || rf_lower.contains("clevel") {
                *fm.entry("JobPost.executive.new_function".into())
                    .or_default() += c;
                *fm.entry("SocialSignal.leadership_change".into())
                    .or_default() += c;
            }
            if rf_lower.contains("quality") || rf_lower.contains("audit") {
                *fm.entry("Compliance.count".into()).or_default() += c;
            }
            if rf_lower.contains("engineer") || rf_lower.contains("r&d") {
                *fm.entry("Technology.count".into()).or_default() += c;
                *fm.entry("Innovation.count".into()).or_default() += c;
            }
        }
        if !seniority.is_empty() {
            let sen_lower = seniority.to_lowercase();
            if sen_lower.contains("director")
                || sen_lower.contains("vp")
                || sen_lower.contains("clevel")
            {
                *fm.entry("JobPost.executive.new_function".into())
                    .or_default() += c;
            }
        }
    }

    // ── Commodity/FX feature enrichment ───────────────────────────
    // Recipes reference CommodityPrice.shift/significant_move,
    // FxRate.EUR_MAD.stable, FxRate.volatility.high, Commodity.*
    for (entity_id, obs_type, item, count) in &commodity_fx_feats {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        match obs_type.as_str() {
            "CommodityPrice" => {
                *fm.entry("CommodityPrice.shift".into()).or_default() += c;
                *fm.entry("CommodityPrice.significant_move".into())
                    .or_default() += c;
                *fm.entry("Commodity.count".into()).or_default() += c;
                *fm.entry("Commodity.price.volatile".into()).or_default() += c;
                if !item.is_empty() {
                    let item_upper = item.to_uppercase();
                    // Map specific commodities to recipe keys
                    if item_upper.contains("COPPER") || item_upper.contains("CU") {
                        *fm.entry("Commodity.Cu.increase_5pct".into()).or_default() += c;
                        *fm.entry("Commodity.copper.increase_5pct".into())
                            .or_default() += c;
                    }
                    if item_upper.contains("TIN") || item_upper.contains("SN") {
                        *fm.entry("Commodity.Sn.increase_5pct".into()).or_default() += c;
                    }
                }
            }
            "FxRate" => {
                *fm.entry("FxRate.significant_move".into()).or_default() += c;
                *fm.entry("FxRate.count".into()).or_default() += c;
                *fm.entry("FxRate.any".into()).or_default() += c;
                if !item.is_empty() {
                    *fm.entry(format!("FxRate.{item}")).or_default() += c;
                    let pair = item.to_uppercase();
                    // Detect specific pairs referenced in recipes
                    if pair.contains("EUR") && pair.contains("MAD") {
                        *fm.entry("FxRate.EUR_MAD.stable".into()).or_default() += c;
                    }
                    if pair.contains("EUR") && pair.contains("TND") {
                        *fm.entry("FxRate.EUR_TND.increase".into()).or_default() += c;
                    }
                    if pair.contains("GBP") && pair.contains("EUR") {
                        *fm.entry("FxRate.GBP_EUR.decrease".into()).or_default() += c;
                    }
                }
            }
            _ => {}
        }
    }

    // ── POI artifact feature enrichment ───────────────────────────
    // POI artifacts (articles, speaker appearances, publications) feed
    // the massive POI recipe category (93 refs). Map artifact types to
    // the rich POI.* feature keys that recipes expect.
    for (company_id, artifact_type, count) in &poi_artifact_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("POI.count".into()).or_default() += c;
        *fm.entry("POI.visibility.high".into()).or_default() += c;
        let at_lower = artifact_type.to_lowercase();
        if at_lower.contains("article") || at_lower.contains("publication") {
            *fm.entry("POI.article.published".into()).or_default() += c;
            *fm.entry("POI.thought_leadership".into()).or_default() += c;
            *fm.entry("PressRelease.count".into()).or_default() += c;
            *fm.entry("PressRelease.any".into()).or_default() += c;
            *fm.entry("News.count".into()).or_default() += c;
        }
        if at_lower.contains("speaker")
            || at_lower.contains("conference")
            || at_lower.contains("panel")
        {
            *fm.entry("POI.conference.speaker".into()).or_default() += c;
            *fm.entry("ConferenceAgenda.count".into()).or_default() += c;
            *fm.entry("TradeShow.speaker.poi_match".into()).or_default() += c;
            *fm.entry("TradeShow.presence.increase".into()).or_default() += c;
        }
        if at_lower.contains("award") || at_lower.contains("recognition") {
            *fm.entry("POI.achievement.professional".into()).or_default() += c;
            *fm.entry("PressRelease.quality_award".into()).or_default() += c;
        }
        if at_lower.contains("social") || at_lower.contains("post") || at_lower.contains("linkedin")
        {
            *fm.entry("POI.social.active".into()).or_default() += c;
            *fm.entry("POI.content.engagement".into()).or_default() += c;
            *fm.entry("SocialPost.count".into()).or_default() += c;
        }
        if at_lower.contains("patent") {
            *fm.entry("Patent.count".into()).or_default() += c;
            *fm.entry("POI.technical.expert".into()).or_default() += c;
        }
    }

    for (entity_id, feature_map) in entity_maps.iter_mut() {
        let stats_input = StatsEnrichmentInput {
            entity_id: entity_id.to_string(),
            primary_series: observation_series_by_entity
                .get(entity_id)
                .cloned()
                .unwrap_or_else(|| vec![0.0; 31]),
            secondary_series: Some(
                warning_series_by_entity
                    .get(entity_id)
                    .cloned()
                    .unwrap_or_else(|| vec![0.0; 31]),
            ),
            ..Default::default()
        };
        let stats_result = analyze_with_calibration(&stats_input, Some(&alert_calibration_model));
        for (key, value) in &stats_result.features {
            feature_map.insert(key.clone(), *value);
        }

        if stats_result.alert_level.to_string() != "none" {
            let feature_vector = serde_json::to_value(&stats_result.features)
                .unwrap_or_else(|_| serde_json::json!({}));
            let metadata = serde_json::json!({
                "window_days": 30,
                "stage_warning_count": stats_result.warnings.len(),
                "alert_score": stats_result.alert_score,
                "alert_probability": stats_result.alert_probability,
                "calibration_method": stats_result.calibration_method,
                "observation_series_total": stats_input.primary_series.iter().sum::<f64>(),
                "warning_series_total": stats_input
                    .secondary_series
                    .as_ref()
                    .map(|series| series.iter().sum::<f64>())
                    .unwrap_or(0.0),
            });
            if let Err(error) = store
                .record_stats_alert_calibration_event(
                    *entity_id,
                    &feature_vector,
                    &stats_result.alert_level.to_string(),
                    now,
                    now + chrono::Duration::days(30),
                    &metadata,
                )
                .await
            {
                tracing::warn!(
                    %error,
                    entity_id = %entity_id,
                    alert_level = %stats_result.alert_level,
                    "recipe_fire: failed to persist stats alert calibration tuple"
                );
            }
        }
    }

    tracing::info!(
        entities = entity_maps.len(),
        obs_rows = obs_counts.len(),
        warn_rows = warn_counts.len(),
        ce_rows = ce_features.len(),
        wc_rows = wc_features.len(),
        kw_rows = wc_kw_features.len(),
        person_rows = person_feats.len(),
        cert_rows = cert_feats.len(),
        cap_rows = cap_feats.len(),
        site_rows = site_feats.len(),
        graph_rows = graph_feats.len(),
        job_post_rows = job_post_feats.len(),
        commodity_fx_rows = commodity_fx_feats.len(),
        poi_artifact_rows = poi_artifact_feats.len(),
        "recipe_fire: built feature maps"
    );

    let entity_id_strs: Vec<(String, FeatureMap)> = entity_maps
        .into_iter()
        .map(|(id, fm)| (id.to_string(), fm))
        .collect();
    let entity_refs: Vec<(&str, &FeatureMap)> = entity_id_strs
        .iter()
        .map(|(id, fm)| (id.as_str(), fm))
        .collect();

    #[cfg(feature = "llm")]
    let mut candidates = engine.evaluate_batch(&entity_refs);
    #[cfg(not(feature = "llm"))]
    let candidates = engine.evaluate_batch(&entity_refs);

    #[cfg(feature = "llm")]
    let (feedback_tracker, adaptive_thresholds, recipe_quality_scores) =
        hydrate_feedback_tracker(store.as_ref(), now).await;

    #[cfg(feature = "llm")]
    candidates.sort_by(|left, right| {
        let left_quality = recipe_quality_scores
            .get(&left.recipe_code.to_ascii_uppercase())
            .copied()
            .unwrap_or(0.5);
        let right_quality = recipe_quality_scores
            .get(&right.recipe_code.to_ascii_uppercase())
            .copied()
            .unwrap_or(0.5);
        let left_score = left.confidence * left.impact * (0.75 + 0.5 * left_quality);
        let right_score = right.confidence * right.impact * (0.75 + 0.5 * right_quality);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let all_entity_uuids: Vec<Uuid> = candidates
        .iter()
        .filter_map(|c| Uuid::parse_str(&c.entity_id).ok())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    #[cfg(feature = "llm")]
    let recent_insight_limit = ((all_entity_uuids.len() as i64).saturating_mul(6)).clamp(300, 1500);

    #[cfg(feature = "llm")]
    let recent_insights_by_entity: HashMap<String, Vec<InsightRow>> = match store
        .get_recent_insights_for_entities(
            &all_entity_uuids,
            now - chrono::Duration::days(21),
            recent_insight_limit,
        )
        .await
    {
        Ok(rows) => {
            let mut grouped: HashMap<String, Vec<InsightRow>> = HashMap::new();
            for row in rows {
                for entity_id in row.entity_ids.clone().unwrap_or_default() {
                    grouped
                        .entry(entity_id.to_string())
                        .or_default()
                        .push(row.clone());
                }
            }
            grouped
        }
        Err(error) => {
            tracing::warn!(%error, "recipe_fire: failed to load recent insights for semantic dedup");
            HashMap::new()
        }
    };

    let company_names: HashMap<String, (String, Option<String>, Option<String>)> = match store
        .get_company_names_by_ids(&all_entity_uuids)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|(id, name, region, company_type)| (id.to_string(), (name, region, company_type)))
            .collect(),
        Err(e) => {
            tracing::warn!("recipe_fire: failed to load company names: {e}");
            HashMap::new()
        }
    };

    let poi_names: HashMap<String, String> = match store
        .get_person_names_by_company_ids(&all_entity_uuids)
        .await
    {
        Ok(rows) => {
            let mut m: HashMap<String, String> = HashMap::new();
            for (org_id, name) in rows {
                m.entry(org_id.to_string()).or_insert(name);
            }
            m
        }
        Err(e) => {
            tracing::warn!("recipe_fire: failed to load POI names: {e}");
            HashMap::new()
        }
    };

    #[cfg(feature = "llm")]
    let (entity_evidence, entity_contexts): (
        HashMap<String, Vec<EvidenceSignal>>,
        HashMap<String, EntityContext>,
    ) = {
        let mut evidence_map: HashMap<String, Vec<EvidenceSignal>> = HashMap::new();
        let mut context_map: HashMap<String, EntityContext> = HashMap::new();

        for (entity_id, (name, region, company_type)) in &company_names {
            let industry_tags: Vec<String> = Vec::new();
            context_map.insert(
                entity_id.clone(),
                EntityContext {
                    name: name.clone(),
                    region: region.clone().unwrap_or_default(),
                    entity_type: company_type.clone(),
                    is_competitor: false,
                    industry_tags,
                    certifications: Vec::new(),
                    capabilities: Vec::new(),
                    key_persons: Vec::new(),
                    recent_changes: Vec::new(),
                    threat_score: None,
                    overlap_score: None,
                    strategic_relevance: None,
                    revenue_estimate_usd: None,
                    employee_estimate: None,
                    competitor_names: Vec::new(),
                    sites_summary: Vec::new(),
                    competitor_events: Vec::new(),
                    domain: None,
                },
            );
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(Some(row)) = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>, Option<i64>, Option<i32>, Option<String>, Option<Vec<String>>, Option<bool>)>(
                "SELECT threat_score, overlap_score, strategic_relevance, revenue_estimate_usd, employee_estimate, domain, industry_tags, (metadata->>'is_competitor')::boolean FROM companies WHERE id = $1"
            )
            .bind(entity_uuid)
            .fetch_optional(&store.pool)
            .await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    ctx.threat_score = row.0;
                    ctx.overlap_score = row.1;
                    ctx.strategic_relevance = row.2;
                    ctx.revenue_estimate_usd = row.3;
                    ctx.employee_estimate = row.4;
                    ctx.domain = row.5;
                    if let Some(tags) = row.6 {
                        ctx.industry_tags = tags;
                    }
                    ctx.is_competitor = row.7.unwrap_or(false);
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(edges) = store.get_edges_from(*entity_uuid, "company").await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for edge in edges.iter().take(8) {
                        if let Some((name, region, _ctype)) =
                            company_names.get(&edge.target_id.to_string())
                        {
                            let region_str = region.as_deref().unwrap_or("");
                            ctx.competitor_names
                                .push(format!("{} ({})", name, region_str));
                        }
                    }
                }
            }
            if let Ok(edges) = store.get_edges_to(*entity_uuid, "company").await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for edge in edges.iter().take(8) {
                        if let Some((name, region, _ctype)) =
                            company_names.get(&edge.source_id.to_string())
                        {
                            let region_str = region.as_deref().unwrap_or("");
                            let entry = format!("{} ({})", name, region_str);
                            if !ctx.competitor_names.contains(&entry) {
                                ctx.competitor_names.push(entry);
                            }
                        }
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(sites) = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<Vec<String>>)>(
                "SELECT name, city, country_code, site_type, capabilities FROM sites WHERE company_id = $1 LIMIT 6"
            )
            .bind(entity_uuid)
            .fetch_all(&store.pool)
            .await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for (sname, city, cc, stype, caps) in sites {
                        let location = city.unwrap_or_else(|| cc.unwrap_or_default());
                        let type_str = stype.unwrap_or_default();
                        let caps_str = caps.map(|c| c.join(", ")).unwrap_or_default();
                        let summary = if caps_str.is_empty() {
                            format!("{} ({}) in {}", sname, type_str, location)
                        } else {
                            format!("{} ({}) in {} – capabilities: {}", sname, type_str, location, caps_str)
                        };
                        ctx.sites_summary.push(summary);
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(obs) = store.get_observations_by_entity(*entity_uuid, 5).await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for o in obs {
                        if o.observation_type == "CompetitorEvent"
                            || o.observation_type == "JobPost"
                            || o.observation_type == "SocialPost"
                        {
                            let excerpt = o
                                .value
                                .as_object()
                                .and_then(|obj| {
                                    obj.get("excerpt").or(obj.get("title")).or(obj.get("text"))
                                })
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let source = o
                                .value
                                .as_object()
                                .and_then(|obj| obj.get("source").or(obj.get("url")))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !excerpt.is_empty() {
                                let evt_line = crate::truncate_text(
                                    &format!("{}: {}", o.observation_type, &excerpt),
                                    200,
                                )
                                .to_string();
                                let sig = EvidenceSignal {
                                    title: format!(
                                        "{}: {}",
                                        o.observation_type,
                                        crate::truncate_text(&excerpt, 80)
                                    ),
                                    description: excerpt,
                                    source_url: source,
                                    signal_type: o.observation_type.clone(),
                                    extracted_facts: extract_facts_from_text(&o.value.to_string()),
                                    date_context: Some(o.ts_utc.format("%Y-%m-%d").to_string()),
                                    relevance_score: 0.75,
                                };
                                evidence_map
                                    .entry(entity_id_str.clone())
                                    .or_default()
                                    .push(sig);
                                ctx.competitor_events.push(evt_line);
                            }
                        }
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            if let Ok(certs) = store.get_certifications_for_company(*entity_uuid).await {
                let entity_id_str = entity_uuid.to_string();
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for cert in certs.iter().take(10) {
                        let valid_info = cert
                            .valid_until
                            .map(|d| format!(" (valid until {})", d))
                            .unwrap_or_default();
                        let cert_str = format!("{}{}", cert.standard, valid_info);
                        ctx.certifications.push(cert_str.clone());

                        let sig = EvidenceSignal {
                            title: format!("{} Certification", cert.standard),
                            description: format!(
                                "Holds {} certification{}. Issuing body: {}. Scope: {}",
                                cert.standard,
                                valid_info,
                                cert.issuing_body.as_deref().unwrap_or("Unknown"),
                                cert.scope.as_deref().unwrap_or("General")
                            ),
                            source_url: cert.evidence_url.clone().unwrap_or_default(),
                            signal_type: "certification".to_string(),
                            extracted_facts: vec![
                                format!("Standard: {}", cert.standard),
                                cert.valid_until
                                    .map(|d| format!("Valid until: {}", d))
                                    .unwrap_or_default(),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect(),
                            date_context: cert.valid_until.map(|d| d.to_string()),
                            relevance_score: 0.5,
                        };
                        evidence_map
                            .entry(entity_id_str.clone())
                            .or_default()
                            .push(sig);
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            if let Ok(caps) = store.list_capabilities(Some(*entity_uuid), 15, 0).await {
                let entity_id_str = entity_uuid.to_string();
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for cap in caps.iter().take(10) {
                        let proof_info = cap
                            .proof_grade
                            .as_deref()
                            .map(|g| format!(" ({})", g))
                            .unwrap_or_default();
                        ctx.capabilities
                            .push(format!("{}{}", cap.capability, proof_info));

                        let sig = EvidenceSignal {
                            title: format!("Capability: {}", cap.capability),
                            description: format!(
                                "Manufacturing capability: {}. Proof level: {}",
                                cap.capability,
                                cap.proof_grade.as_deref().unwrap_or("Claimed")
                            ),
                            source_url: cap
                                .evidence_urls
                                .as_ref()
                                .and_then(|u| u.first().cloned())
                                .unwrap_or_default(),
                            signal_type: "capability".to_string(),
                            extracted_facts: vec![format!("Capability: {}", cap.capability)],
                            date_context: cap
                                .last_confirmed
                                .map(|d| d.format("%Y-%m-%d").to_string()),
                            relevance_score: 0.4,
                        };
                        evidence_map
                            .entry(entity_id_str.clone())
                            .or_default()
                            .push(sig);
                    }
                }
            }
        }

        for (entity_id, poi_name) in &poi_names {
            if let Some(ctx) = context_map.get_mut(entity_id) {
                ctx.key_persons.push(poi_name.clone());
            }
        }

        // ── POI evidence signals ──────────────────────────────────────
        // Persons-of-interest are the #1 recipe category (93 recipe
        // conditions) but previously had zero evidence signals, making
        // LLM-generated POI insights vague.  Load key persons per
        // entity as evidence so recipes in the strategic_poi, talent_ip,
        // and personnel categories have concrete data to reason about.
        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(persons) = store.list_persons_by_org(*entity_uuid).await {
                for person in persons.iter().take(6) {
                    let role = person.current_role.as_deref().unwrap_or("Unknown role");
                    let influence = person.influence_score.unwrap_or(0.0);
                    let bio_excerpt = person
                        .public_bio
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect::<String>();
                    let topics = person
                        .trigger_topics
                        .as_ref()
                        .map(|t| t.join(", "))
                        .unwrap_or_default();

                    let mut facts = vec![
                        format!("Role: {}", role),
                        format!("Influence: {:.2}", influence),
                    ];
                    if !topics.is_empty() {
                        facts.push(format!("Trigger topics: {}", topics));
                    }
                    if let Some(style) = &person.decision_style {
                        facts.push(format!("Decision style: {}", style));
                    }

                    let relevance = if influence > 0.7 {
                        0.75
                    } else if influence > 0.5 {
                        0.6
                    } else {
                        0.45
                    };

                    let sig = EvidenceSignal {
                        title: format!("{} – {}", person.name, role),
                        description: if bio_excerpt.is_empty() {
                            format!(
                                "{} serves as {} with influence score {:.2}",
                                person.name, role, influence
                            )
                        } else {
                            format!("{}: {}…", role, bio_excerpt)
                        },
                        source_url: String::new(),
                        signal_type: "poi".to_string(),
                        extracted_facts: facts,
                        date_context: person.updated_at.map(|d| d.format("%Y-%m-%d").to_string()),
                        relevance_score: relevance,
                    };
                    evidence_map
                        .entry(entity_id_str.clone())
                        .or_default()
                        .push(sig);
                }
            }
        }

        // ── Site / facility evidence signals ──────────────────────────
        // Facility/production features (37 Supplier refs, 4 Production
        // refs, 4 Facility refs) fire recipes but had no corresponding
        // evidence signals for the LLM to reference.
        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(sites) = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<Vec<String>>)>(
                "SELECT name, city, country_code, site_type, capabilities FROM sites WHERE company_id = $1 LIMIT 6"
            )
            .bind(entity_uuid)
            .fetch_all(&store.pool)
            .await {
                for (sname, city, cc, stype, caps) in &sites {
                    let location = city.as_deref().unwrap_or(cc.as_deref().unwrap_or("Unknown"));
                    let type_str = stype.as_deref().unwrap_or("facility");
                    let caps_str = caps.as_ref().map(|c| c.join(", ")).unwrap_or_default();

                    let mut facts = vec![
                        format!("Site: {}", sname),
                        format!("Type: {}", type_str),
                        format!("Location: {}", location),
                    ];
                    if !caps_str.is_empty() {
                        facts.push(format!("Capabilities: {}", caps_str));
                    }

                    let sig = EvidenceSignal {
                        title: format!("Facility: {} ({})", sname, location),
                        description: if caps_str.is_empty() {
                            format!("{} {} in {}", type_str, sname, location)
                        } else {
                            format!("{} {} in {} — capabilities: {}", type_str, sname, location, caps_str)
                        },
                        source_url: String::new(),
                        signal_type: "facility".to_string(),
                        extracted_facts: facts,
                        date_context: None,
                        relevance_score: 0.45,
                    };
                    evidence_map
                        .entry(entity_id_str.clone())
                        .or_default()
                        .push(sig);
                }
            }
        }

        // ── Graph relationship evidence signals ───────────────────────
        // Supplier/customer/partnership relationships are tracked in
        // graph_edges but never surfaced as evidence. This gives the LLM
        // concrete relationship data for 37 Supplier recipe refs, 10
        // Customer refs, and 5 Partnership refs.
        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(edges) = store.get_graph_edge_evidence(*entity_uuid).await {
                for (edge_type, target_type, target_name, weight, confidence) in
                    edges.iter().take(8)
                {
                    let label = match edge_type.as_str() {
                        "CompanyCompany" => "Business relationship",
                        "CompanyPerson" => "Key personnel",
                        "CompanySite" => "Facility",
                        "SiteLogistics" => "Logistics link",
                        "CompanyCapability" => "Capability",
                        "VulnProduct" => "Vulnerability exposure",
                        "CompanyRegulation" => "Regulatory exposure",
                        "PersonPatent" => "Patent holder",
                        "PersonEvent" => "Event participation",
                        _ => "Relationship",
                    };

                    let facts = vec![
                        format!("Relationship: {}", edge_type),
                        format!("Target: {} ({})", target_name, target_type),
                        format!("Weight: {:.2}, Confidence: {:.2}", weight, confidence),
                    ];

                    let sig = EvidenceSignal {
                        title: format!("{}: {}", label, target_name),
                        description: format!(
                            "{} link to {} {} (weight {:.2}, confidence {:.2})",
                            edge_type, target_type, target_name, weight, confidence
                        ),
                        source_url: String::new(),
                        signal_type: format!("graph_{}", edge_type),
                        extracted_facts: facts,
                        date_context: None,
                        relevance_score: (*weight as f32 * 0.5).clamp(0.3, 0.7),
                    };
                    evidence_map
                        .entry(entity_id_str.clone())
                        .or_default()
                        .push(sig);
                }
            }
        }

        // Do not feed analyst/generated warnings back into the LLM evidence set.
        // Recursive warning->insight reuse amplifies stale narratives and can turn
        // prior generated copy into apparent source evidence.

        let unique_entity_uuids: std::collections::HashSet<Uuid> =
            all_entity_uuids.iter().cloned().collect();

        // ── Type-stratified observation evidence ──────────────────────
        // Instead of loading the 10 most-recent observations (biased
        // towards high-volume types like WebChange), load a balanced
        // sample across observation types so that tender notices,
        // patent publications, regulatory filings, financial disclosures,
        // and person moves all get fair representation as evidence.
        let diverse_obs_types = [
            "TenderNotice",
            "TenderPosted",
            "PatentPublication",
            "PatentPublished",
            "RegulatoryFiling",
            "FinancialDisclosure",
            "PersonMove",
            "PersonMention",
            "RoleChange",
            "SpeakerAppearance",
            "JobPost",
            "CertificationUpdate",
            "CommodityPrice",
            "FxRate",
            "PortMetric",
            "NewDomain",
            "VulnNotice",
            "ProcurementSignal",
            "CompetitorEvent",
            "WebChange",
            "SocialPost",
        ];
        for entity_uuid in unique_entity_uuids.iter() {
            // Track how many signals we've added per type so we can
            // cap the total while preserving diversity.
            let mut type_counts: HashMap<String, u32> = HashMap::new();
            let obs_rows = match store.get_observations_by_entity(*entity_uuid, 50).await {
                Ok(rows) => rows,
                Err(_) => continue,
            };

            // First pass: prioritize rare/high-value types
            for obs_type in &diverse_obs_types {
                for obs in obs_rows.iter().filter(|o| o.observation_type == *obs_type) {
                    let count = type_counts.entry(obs_type.to_string()).or_default();
                    if *count >= 3 {
                        break;
                    }

                    let (title, description) = if let Some(obj) = obs.value.as_object() {
                        let title = obj
                            .get("title")
                            .or_else(|| obj.get("headline"))
                            .or_else(|| obj.get("role"))
                            .or_else(|| obj.get("commodity"))
                            .or_else(|| obj.get("pair"))
                            .or_else(|| obj.get("cve_id"))
                            .or_else(|| obj.get("domain"))
                            .or_else(|| obj.get("patent_id"))
                            .or_else(|| obj.get("standard"))
                            .or_else(|| obj.get("event"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("{} update", obs.observation_type));
                        let desc = obj
                            .get("description")
                            .or_else(|| obj.get("text"))
                            .or_else(|| obj.get("summary"))
                            .or_else(|| obj.get("content"))
                            .or_else(|| obj.get("detail"))
                            .or_else(|| obj.get("context"))
                            .or_else(|| obj.get("diff_summary"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        (title, desc)
                    } else if let Some(s) = obs.value.as_str() {
                        (obs.observation_type.clone(), s.to_string())
                    } else {
                        continue;
                    };

                    if description.is_empty() && !title.contains(':') {
                        continue;
                    }

                    let source_url = obs
                        .provenance
                        .as_object()
                        .and_then(|p| p.get("source_url").or_else(|| p.get("url")))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    let extracted = extract_facts_from_text(&format!("{} {}", title, description));

                    // Assign higher relevance to rarer, more actionable types
                    let relevance = match *obs_type {
                        "TenderNotice" | "TenderPosted" => 0.80,
                        "PatentPublication" | "PatentPublished" => 0.70,
                        "RegulatoryFiling" => 0.70,
                        "FinancialDisclosure" => 0.65,
                        "PersonMove" | "RoleChange" => 0.75,
                        "PersonMention" | "SpeakerAppearance" => 0.65,
                        "JobPost" => 0.70,
                        "CertificationUpdate" => 0.60,
                        "CommodityPrice" => 0.75,
                        "FxRate" => 0.70,
                        "PortMetric" => 0.80,
                        "NewDomain" | "VulnNotice" => 0.75,
                        "ProcurementSignal" => 0.70,
                        "CompetitorEvent" => 0.70,
                        "SocialPost" => 0.55,
                        _ => 0.50,
                    };

                    let sig = EvidenceSignal {
                        title,
                        description,
                        source_url,
                        signal_type: obs.observation_type.clone(),
                        extracted_facts: extracted,
                        date_context: Some(obs.ts_utc.format("%Y-%m-%d").to_string()),
                        relevance_score: relevance,
                    };
                    evidence_map
                        .entry(entity_uuid.to_string())
                        .or_default()
                        .push(sig);
                    *count += 1;
                }
            }

            // Second pass: fill remaining slots with any type not yet seen
            for obs in &obs_rows {
                let type_count = *type_counts.get(&obs.observation_type).unwrap_or(&0);
                if type_count >= 3 {
                    continue;
                }
                let total: u32 = type_counts.values().sum();
                if total >= 25 {
                    break;
                }

                let (title, description) = if let Some(obj) = obs.value.as_object() {
                    let title = obj
                        .get("title")
                        .or_else(|| obj.get("headline"))
                        .or_else(|| obj.get("role"))
                        .or_else(|| obj.get("commodity"))
                        .or_else(|| obj.get("pair"))
                        .or_else(|| obj.get("cve_id"))
                        .or_else(|| obj.get("domain"))
                        .or_else(|| obj.get("patent_id"))
                        .or_else(|| obj.get("standard"))
                        .or_else(|| obj.get("event"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("{} update", obs.observation_type));
                    let desc = obj
                        .get("description")
                        .or_else(|| obj.get("text"))
                        .or_else(|| obj.get("summary"))
                        .or_else(|| obj.get("content"))
                        .or_else(|| obj.get("detail"))
                        .or_else(|| obj.get("context"))
                        .or_else(|| obj.get("diff_summary"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    (title, desc)
                } else if let Some(s) = obs.value.as_str() {
                    (obs.observation_type.clone(), s.to_string())
                } else {
                    continue;
                };

                if description.is_empty() && !title.contains(':') {
                    continue;
                }

                let source_url = obs
                    .provenance
                    .as_object()
                    .and_then(|p| p.get("source_url").or_else(|| p.get("url")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let extracted = extract_facts_from_text(&format!("{} {}", title, description));

                let sig = EvidenceSignal {
                    title,
                    description,
                    source_url,
                    signal_type: obs.observation_type.clone(),
                    extracted_facts: extracted,
                    date_context: Some(obs.ts_utc.format("%Y-%m-%d").to_string()),
                    relevance_score: 0.5,
                };
                evidence_map
                    .entry(entity_uuid.to_string())
                    .or_default()
                    .push(sig);
                *type_counts.entry(obs.observation_type.clone()).or_default() += 1;
            }
        }

        tracing::info!(
            unique_entities = unique_entity_uuids.len(),
            entities_with_evidence = evidence_map.len(),
            total_evidence_items = evidence_map.values().map(|v| v.len()).sum::<usize>(),
            entities_with_context = context_map.len(),
            "recipe_fire: loaded enriched evidence for LLM"
        );

        (evidence_map, context_map)
    };

    #[cfg(not(feature = "llm"))]
    let entity_evidence_urls = collect_entity_evidence_urls(store, &all_entity_uuids).await;

    let mut dedup_map: HashMap<(String, String), usize> = HashMap::new();
    for (idx, c) in candidates.iter().enumerate() {
        let key = (c.entity_id.clone(), c.category.clone());
        let dominated = match dedup_map.get(&key) {
            Some(&prev_idx) => {
                let prev = &candidates[prev_idx];
                c.confidence * c.impact > prev.confidence * prev.impact
            }
            None => true,
        };
        if dominated {
            dedup_map.insert(key, idx);
        }
    }
    let deduped_idxs: std::collections::HashSet<usize> = dedup_map.values().copied().collect();

    let total_candidates = candidates.len();
    let deduped_count = total_candidates - deduped_idxs.len();
    if deduped_count > 0 {
        tracing::info!(
            total = total_candidates,
            deduped = deduped_count,
            "recipe_fire: per-run dedup removed duplicate candidates"
        );
    }

    let mut insights_inserted: u64 = 0;
    let mut warnings_inserted: u64 = 0;
    let mut skipped_low_conf: u64 = 0;
    let mut skipped_dedup: u64 = 0;
    let mut skipped_cross_run: u64 = 0;

    let cross_run_dedup: std::collections::HashSet<(String, String)> = {
        let rows = sqlx::query(
            "SELECT DISTINCT t.tag AS recipe_code, e.entity_id::text AS eid \
             FROM insights i \
             CROSS JOIN LATERAL unnest(COALESCE(i.tags, ARRAY[]::text[])) AS t(tag) \
             CROSS JOIN LATERAL unnest(COALESCE(i.entity_ids, ARRAY[]::uuid[])) AS e(entity_id) \
             WHERE i.created_at > NOW() - INTERVAL '48 hours' \
               AND t.tag ~ '^[A-Z][0-9]{3,}'",
        )
        .fetch_all(&store.pool)
        .await;
        match rows {
            Ok(rows) => {
                use sqlx::Row as _;
                rows.into_iter()
                    .filter_map(|r| {
                        let tag: String = r.try_get("recipe_code").ok()?;
                        let eid: String = r.try_get("eid").ok()?;
                        Some((tag, eid))
                    })
                    .collect()
            }
            Err(e) => {
                tracing::warn!("recipe_fire: failed to load cross-run dedup set: {e}");
                std::collections::HashSet::new()
            }
        }
    };

    #[cfg(feature = "llm")]
    let mut llm_entity_cat_done: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    #[cfg(feature = "llm")]
    let mut entity_run_count: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for (idx, c) in candidates.iter().enumerate() {
        if !deduped_idxs.contains(&idx) {
            skipped_dedup += 1;
            continue;
        }

        #[cfg(feature = "llm")]
        if let Some(analysis) = feedback_tracker.detect_fatigue(&c.entity_id, &c.recipe_code) {
            if analysis.is_fatigued && analysis.recommended_action == FatigueAction::Suppress {
                tracing::info!(
                    entity_id = %c.entity_id,
                    recipe = %c.recipe_code,
                    firing_count = analysis.firing_count,
                    repetition_rate = analysis.repetition_rate,
                    "recipe_fire: suppressed fatigued entity-recipe pair"
                );
                skipped_cross_run += 1;
                continue;
            }
        }

        #[cfg(feature = "llm")]
        let fatigue_penalty = feedback_tracker
            .staleness_penalty(&c.entity_id, &c.recipe_code)
            .clamp(0.1, 1.0);
        #[cfg(not(feature = "llm"))]
        let fatigue_penalty = 1.0;

        let effective_candidate_confidence = (c.confidence * fatigue_penalty).clamp(0.0, 1.0);

        #[cfg(feature = "llm")]
        let adaptive_min_confidence = adaptive_thresholds
            .get(&c.recipe_code.to_ascii_uppercase())
            .copied()
            .unwrap_or(0.30);
        #[cfg(not(feature = "llm"))]
        let adaptive_min_confidence = 0.30;

        if effective_candidate_confidence < adaptive_min_confidence {
            skipped_low_conf += 1;
            continue;
        }

        if cross_run_dedup.contains(&(c.recipe_code.clone(), c.entity_id.clone())) {
            skipped_cross_run += 1;
            continue;
        }

        let entity_uuid = Uuid::parse_str(&c.entity_id).ok();
        let entity_ids = entity_uuid.map(|u| vec![u]);
        let mut tags = vec![c.category.clone(), c.recipe_code.clone()];

        let mut slots: HashMap<String, String> = HashMap::new();
        let mut entity_label = String::new();
        let mut entity_region = String::new();
        let mut entity_type: Option<String> = None;
        if let Some((name, region, company_type)) = company_names.get(&c.entity_id) {
            entity_label = name.clone();
            entity_type = company_type.clone();
            slots.insert("company_name".into(), name.clone());
            slots.insert("supplier_name".into(), name.clone());
            slots.insert("vendor_name".into(), name.clone());
            slots.insert("customer_name".into(), name.clone());
            slots.insert("target_company".into(), name.clone());
            slots.insert("entity_name".into(), name.clone());
            slots.insert("competitor_name".into(), name.clone());
            if let Some(r) = region {
                entity_region = r.clone();
                slots.insert("region".into(), r.clone());
                slots.insert("location".into(), r.clone());
                slots.insert("country".into(), r.clone());
            }
        }
        if let Some(poi) = poi_names.get(&c.entity_id) {
            slots.insert("poi_name".into(), poi.clone());
            slots.insert("mentor_name".into(), poi.clone());
        }

        let mut signal_details: Vec<String> = Vec::new();
        if let Some((_, fm)) = entity_id_strs.iter().find(|(id, _)| id == &c.entity_id) {
            if let Some(v) = fm.get("POI.count") {
                slots.insert("poi_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} person(s) of interest tracked", *v as i64));
                }
            }
            if let Some(v) = fm.get("JobPost.count") {
                slots.insert("job_count".into(), (*v as i64).to_string());
                slots.insert("job_delta".into(), format!("{:.0}", v));
                slots.insert("npi_jobs".into(), (*v as i64).to_string());
                slots.insert("eng_jobs".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} job posting(s) observed", *v as i64));
                }
            }
            if let Some(v) = fm.get("Patent.count") {
                slots.insert("patent_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} patent(s) filed", *v as i64));
                }
            }
            if let Some(v) = fm.get("Tender.count") {
                slots.insert("tender_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} tender(s) identified", *v as i64));
                }
            }
            if let Some(v) = fm.get("Certification.count") {
                slots.insert("cert_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} certification(s) on record", *v as i64));
                }
            }
            if let Some(v) = fm.get("Competitor.count") {
                slots.insert("competitor_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} competitor signal(s)", *v as i64));
                }
            }
            if let Some(v) = fm.get("NewsArticle.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} news article(s) referenced", *v as i64));
                }
            }
            if let Some(v) = fm.get("WebChange.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} web change(s) detected", *v as i64));
                }
            }
            if let Some(v) = fm.get("LookalikeDomain.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} lookalike domain(s) detected", *v as i64));
                }
            }
            if let Some(v) = fm.get("DNSPosture.degraded") {
                if *v > 0.0 {
                    signal_details.push("DNS posture degradation detected".to_string());
                }
            }
            if let Some(v) = fm.get("KEV.match") {
                if *v > 0.0 {
                    signal_details.push("known exploited vulnerability match detected".to_string());
                }
            }
            if let Some(v) = fm.get("KEV.count") {
                if *v > 0.0 {
                    signal_details.push(format!(
                        "{} KEV-linked vulnerability match(es) detected",
                        *v as i64
                    ));
                }
            }
            if let Some(v) = fm.get("Filing.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} regulatory filing(s)", *v as i64));
                }
            }
            if let Some(v) = fm.get("SanctionEntry.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} sanction entry(ies)", *v as i64));
                }
            }
            if let Some(v) = fm.get("TradeShow.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} trade show participation(s)", *v as i64));
                }
            }
            let signal_count = c.evidence_ids.len();
            slots.insert("signal_count".into(), signal_count.to_string());
        }

        #[cfg(not(feature = "llm"))]
        let _rendered_narrative = clean_rendered_text(&resolve_evidence_placeholders(
            &c.narrative_template,
            &slots,
        ));
        #[cfg(not(feature = "llm"))]
        let warning_action =
            clean_rendered_text(&resolve_evidence_placeholders(&c.action_template, &slots));

        #[cfg(not(feature = "llm"))]
        let analytical_narrative = build_analytical_narrative(
            &entity_label,
            &entity_region,
            entity_type.as_deref(),
            &c.category,
            &signal_details,
            effective_candidate_confidence,
            &c.evidence_ids,
        );

        #[cfg(feature = "llm")]
        {
            let ec_key = (c.entity_id.clone(), c.category.clone());
            if !llm_entity_cat_done.insert(ec_key) {
                tracing::debug!(
                    entity = %entity_label,
                    category = %c.category,
                    recipe = %c.recipe_code,
                    "recipe_fire: skipping duplicate entity+category LLM call"
                );
                skipped_dedup += 1;
                continue;
            }

            let run_count = entity_run_count.entry(c.entity_id.clone()).or_insert(0);
            if *run_count >= 2 {
                tracing::debug!(
                    entity = %entity_label,
                    category = %c.category,
                    "recipe_fire: per-run entity cap reached (F9), skipping"
                );
                skipped_dedup += 1;
                continue;
            }
            *run_count += 1;
        }

        #[cfg(feature = "llm")]
        let evidence_signals: Vec<EvidenceSignal> = {
            let mut evidence_signals: Vec<EvidenceSignal> = entity_evidence
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();

            let distinct_signal_types = evidence_signals
                .iter()
                .map(|signal| normalize_gate_text(&signal.signal_type))
                .collect::<HashSet<_>>()
                .len();
            let diversity_multiplier = signal_diversity_multiplier(distinct_signal_types);

            for sig in &mut evidence_signals {
                sig.relevance_score = calculate_relevance(
                    &sig.title,
                    &sig.description,
                    &sig.signal_type,
                    &c.category,
                ) * diversity_multiplier;
            }

            evidence_signals.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            evidence_signals.truncate(*crate::config::LLM_MAX_EVIDENCE_SIGNALS);
            evidence_signals
        };

        #[cfg(feature = "llm")]
        let llm_output = {
            let entity_ctx = entity_contexts
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_else(|| EntityContext {
                    name: entity_label.clone(),
                    region: entity_region.clone(),
                    entity_type: entity_type.clone(),
                    is_competitor: false,
                    industry_tags: Vec::new(),
                    certifications: Vec::new(),
                    capabilities: Vec::new(),
                    key_persons: poi_names.get(&c.entity_id).cloned().into_iter().collect(),
                    recent_changes: Vec::new(),
                    threat_score: None,
                    overlap_score: None,
                    strategic_relevance: None,
                    revenue_estimate_usd: None,
                    employee_estimate: None,
                    competitor_names: Vec::new(),
                    sites_summary: Vec::new(),
                    competitor_events: Vec::new(),
                    domain: None,
                });

            tracing::info!(
                entity = %entity_label,
                category = %c.category,
                evidence_count = evidence_signals.len(),
                certs = entity_ctx.certifications.len(),
                caps = entity_ctx.capabilities.len(),
                "recipe_fire: generating LLM insight"
            );

            match generate_llm_insight(
                &insight_llm_client,
                &entity_ctx,
                &c.category,
                &evidence_signals,
            )
            .await
            {
                Ok((headline, narrative, recommendation, llm_confidence, insight_metadata)) => {
                    let source_urls = ranked_source_urls(&evidence_signals, 6);
                    let sources_footer = format_sources_footer(&evidence_signals, 6);
                    let summary = if sources_footer.is_empty() {
                        format!("{}\n\n{}", narrative, recommendation)
                    } else {
                        format!("{}\n\n{}\n\n{}", narrative, recommendation, sources_footer)
                    };
                    Some((
                        headline,
                        summary,
                        recommendation,
                        llm_confidence,
                        source_urls,
                        evidence_signals.clone(),
                        insight_metadata,
                    ))
                }
                Err(e) => {
                    tracing::warn!(
                        entity = %entity_label,
                        category = %c.category,
                        error = %e,
                        "recipe_fire: LLM insight generation failed; skipping insight (no template fallback)"
                    );
                    None
                }
            }
        };

        #[cfg(feature = "llm")]
        let (
            title,
            summary,
            warning_action,
            mut stored_confidence,
            llm_source_urls,
            evidence_quality,
            warning_evidence_signals,
            llm_insight_metadata,
        ) = match llm_output {
            Some((
                title,
                summary,
                warning_action,
                llm_confidence,
                llm_source_urls,
                evidence_signals,
                insight_metadata,
            )) => {
                let evidence_quality =
                    evidence_quality_from_signals(&evidence_signals, &source_reliability_scores);
                let stored_confidence = (0.75 * effective_candidate_confidence
                    + 0.15 * llm_confidence
                    + 0.10 * evidence_quality)
                    .clamp(0.0, 1.0);
                (
                    title,
                    summary,
                    warning_action,
                    stored_confidence,
                    llm_source_urls,
                    evidence_quality,
                    evidence_signals,
                    insight_metadata,
                )
            }
            None => {
                if !has_concrete_evidence(&evidence_signals) {
                    continue;
                }
                // LLM generation failed but concrete evidence exists —
                // emit a fallback warning derived from evidence signals directly.
                let fallback_evidence_quality =
                    evidence_quality_from_signals(&evidence_signals, &source_reliability_scores);
                let fallback_confidence = (0.75 * effective_candidate_confidence
                    + 0.10 * fallback_evidence_quality)
                    .clamp(0.0, 1.0);
                let fallback_title = build_analytical_title(
                    &entity_label,
                    &entity_region,
                    &c.category,
                    &signal_details,
                    entity_type.as_deref(),
                );
                let fallback_urls = ranked_source_urls(&evidence_signals, 6);
                let evidence_items: Vec<&str> = evidence_signals
                    .iter()
                    .take(3)
                    .map(|s| s.title.as_str())
                    .collect();
                let fallback_summary = if evidence_items.is_empty() {
                    format!(
                        "Concrete evidence detected for {} — review signals for details.",
                        entity_label
                    )
                } else {
                    format!(
                        "Evidence signals detected for {}: {}.",
                        entity_label,
                        evidence_items.join("; ")
                    )
                };
                crate::observability::WORKER_METRICS.record_insight_fallback();
                tracing::info!(
                    entity = %entity_label,
                    category = %c.category,
                    evidence_count = evidence_signals.len(),
                    "recipe_fire: LLM generation failed; emitting evidence-based fallback warning"
                );
                (
                    fallback_title,
                    fallback_summary,
                    String::new(),
                    fallback_confidence,
                    fallback_urls,
                    fallback_evidence_quality,
                    evidence_signals.clone(),
                    serde_json::Value::Null,
                )
            }
        };

        #[cfg(feature = "llm")]
        if let Some(bias_adjustment) = apply_bias_mitigation(
            &entity_label,
            &title,
            &summary,
            &warning_evidence_signals,
            stored_confidence,
        ) {
            stored_confidence =
                (stored_confidence * (1.0 - bias_adjustment.confidence_reduction)).clamp(0.0, 1.0);
            tags.push(bias_adjustment.tag.to_string());
            tracing::info!(
                entity = %entity_label,
                recipe = %c.recipe_code,
                reduction = bias_adjustment.confidence_reduction,
                adjusted_confidence = stored_confidence,
                "recipe_fire: devil's-advocate challenge adjusted insight confidence"
            );
        }

        #[cfg(feature = "llm")]
        let maybe_evidence_urls = (!llm_source_urls.is_empty()).then_some(llm_source_urls);

        #[cfg(feature = "llm")]
        let insight_metadata = Some(llm_insight_metadata);

        #[cfg(not(feature = "llm"))]
        let insight_metadata: Option<serde_json::Value> = None;

        #[cfg(not(feature = "llm"))]
        let stored_confidence = {
            let evidence_urls = entity_evidence_urls
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();
            let evidence_quality =
                evidence_quality_from_urls(&evidence_urls, &source_reliability_scores);
            (0.85 * effective_candidate_confidence + 0.15 * evidence_quality).clamp(0.0, 1.0)
        };
        #[cfg(not(feature = "llm"))]
        let (title, summary) = {
            if !should_emit_fallback_insight(
                &c.category,
                &signal_details,
                &warning_action,
                effective_candidate_confidence,
                c.evidence_ids.len(),
            ) {
                tracing::warn!(
                    recipe = %c.recipe_code,
                    category = %c.category,
                    entity = %entity_label,
                    confidence = effective_candidate_confidence,
                    evidence_count = c.evidence_ids.len(),
                    signal_details = ?signal_details,
                    "recipe_fire: skipping low-signal fallback insight"
                );
                continue;
            }

            let title = build_analytical_title(
                &entity_label,
                &entity_region,
                &c.category,
                &signal_details,
                entity_type.as_deref(),
            );
            let evidence_urls = entity_evidence_urls
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();
            let summary = build_fallback_summary(
                &analytical_narrative,
                &warning_action,
                &signal_details,
                &evidence_urls,
                &entity_label,
                &entity_region,
                entity_type.as_deref(),
                &c.category,
                &c.severity,
                effective_candidate_confidence,
                c.evidence_ids.len(),
            );
            crate::observability::WORKER_METRICS.record_insight_fallback();
            (title, summary)
        };

        #[cfg(not(feature = "llm"))]
        let maybe_evidence_urls = entity_evidence_urls
            .get(&c.entity_id)
            .cloned()
            .filter(|urls| !urls.is_empty());

        #[cfg(feature = "llm")]
        tags.push(format!(
            "evidence:{}",
            if evidence_quality >= 0.75 {
                "high"
            } else if evidence_quality >= 0.5 {
                "moderate"
            } else {
                "emerging"
            }
        ));

        #[cfg(not(feature = "llm"))]
        if let Some(urls) = maybe_evidence_urls.as_ref() {
            let evidence_quality = evidence_quality_from_urls(urls, &source_reliability_scores);
            tags.push(format!(
                "evidence:{}",
                if evidence_quality >= 0.75 {
                    "high"
                } else if evidence_quality >= 0.5 {
                    "moderate"
                } else {
                    "emerging"
                }
            ));
        }

        // LLM-enriched insights already pass 15+ individual quality gates
        // (narrative_words, references, readability, etc.) inside generate_llm_insight.
        // The shared gate's 8-sentence cap rejects the longer LLM narratives, so
        // only apply it to the non-LLM fallback path.
        // NOTE: the gate only controls insight insertion; warnings are always
        // generated so the analyst warnings page stays populated.
        #[cfg(not(feature = "llm"))]
        let insight_gate_passed =
            passes_shared_insight_quality_gate(&title, &summary, Some(c.category.as_str()));
        #[cfg(feature = "llm")]
        let insight_gate_passed = true;

        let region_param = if entity_region.is_empty() {
            None
        } else {
            Some(entity_region.as_str())
        };

        let warning_evidence_urls = maybe_evidence_urls.clone();

        #[cfg(feature = "llm")]
        if let Some(recent_rows) = recent_insights_by_entity.get(&c.entity_id) {
            if is_semantic_duplicate_recent_insight(
                recent_rows,
                &c.recipe_code,
                &c.category,
                &title,
                &summary,
            ) {
                tracing::info!(
                    entity_id = %c.entity_id,
                    recipe = %c.recipe_code,
                    category = %c.category,
                    "recipe_fire: skipped semantically similar recent insight"
                );
                skipped_cross_run += 1;
                continue;
            }
        }

        #[cfg(feature = "llm")]
        let mut insight_inserted = false;
        if insight_gate_passed {
            match store
                .insert_insight(
                    &title,
                    &summary,
                    Some(c.category.as_str()),
                    region_param,
                    Some(stored_confidence),
                    maybe_evidence_urls,
                    entity_ids.clone(),
                    Some(tags),
                    insight_metadata,
                )
                .await
            {
                Ok(insight_id) => {
                    insights_inserted += 1;
                    #[cfg(feature = "llm")]
                    {
                        insight_inserted = true;
                    }
                    if let Some(entity_uuid) = entity_uuid {
                        if let Err(error) = store
                            .record_insight_firing(
                                insight_id,
                                entity_uuid,
                                &c.recipe_code,
                                Some(c.category.as_str()),
                                &title,
                                &summary,
                                Some(stored_confidence),
                            )
                            .await
                        {
                            tracing::warn!(
                                recipe = %c.recipe_code,
                                entity_id = %c.entity_id,
                                %error,
                                "recipe_fire: failed to record insight firing"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(recipe = %c.recipe_code, "recipe_fire: insert_insight failed: {e}")
                }
            }
        } else {
            tracing::warn!(
                recipe = %c.recipe_code,
                category = %c.category,
                title = %title,
                "recipe_fire: shared insight quality gate rejected summary"
            );
        }

        #[cfg(feature = "llm")]
        if let Some(warning_severity) = recipe_warning_severity(
            &c.recipe_code,
            &c.category,
            &c.severity,
            stored_confidence,
            c.impact,
        ) {
            if !should_emit_llm_warning(
                warning_severity,
                &warning_action,
                &warning_evidence_signals,
                evidence_quality,
                insight_inserted,
            ) {
                tracing::info!(
                    recipe = %c.recipe_code,
                    category = %c.category,
                    grounding_evidence = grounding_evidence_count(&warning_evidence_signals),
                    evidence_quality,
                    insight_inserted,
                    "recipe_fire: suppressed low-grounding warning"
                );
                continue;
            }
            let diversified_title = super::template_variation::diversify_title(
                &title,
                &c.recipe_code,
                &c.entity_id,
            );
            let diversified_action = super::template_variation::diversify_action(
                &warning_action,
                &c.recipe_code,
                &c.entity_id,
            );
            let warn_title = format!("[{}] {}", c.recipe_code, diversified_title);
            let warn_region = if entity_region.is_empty() {
                None
            } else {
                Some(entity_region.as_str())
            };
            let warn_urls = warning_evidence_urls.clone();
            let _ = store
                .insert_warning(
                    &c.category,
                    &warn_title,
                    Some(&diversified_action),
                    warning_severity,
                    warn_region,
                    Some(&c.recipe_code),
                    entity_ids.clone(),
                    warn_urls,
                    Some(stored_confidence),
                )
                .await;
            warnings_inserted += 1;
        }

        #[cfg(not(feature = "llm"))]
        if let Some(warning_severity) = recipe_warning_severity(
            &c.recipe_code,
            &c.category,
            &c.severity,
            effective_candidate_confidence,
            c.impact,
        ) {
            let diversified_title = super::template_variation::diversify_title(
                &title,
                &c.recipe_code,
                &c.entity_id,
            );
            let diversified_action = super::template_variation::diversify_action(
                &warning_action,
                &c.recipe_code,
                &c.entity_id,
            );
            let warn_title = format!("[{}] {}", c.recipe_code, diversified_title);
            let warn_region = if entity_region.is_empty() {
                None
            } else {
                Some(entity_region.as_str())
            };
            let warn_urls = warning_evidence_urls.clone();
            let _ = store
                .insert_warning(
                    &c.category,
                    &warn_title,
                    Some(&diversified_action),
                    warning_severity,
                    warn_region,
                    Some(&c.recipe_code),
                    entity_ids.clone(),
                    warn_urls,
                    Some(c.confidence),
                )
                .await;
            warnings_inserted += 1;
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ADVANCED GENERATORS — diversify beyond recipe-only cert-gap insights
    // ═══════════════════════════════════════════════════════════════════════

    #[cfg(feature = "llm")]
    {
        // ── 1. Predictive forward-looking insights ──────────────────────
        let patterns = predefined_patterns();
        let mut predictive_count: u64 = 0;
        for entity_uuid in &all_entity_uuids {
            let eid = entity_uuid.to_string();
            let signals: Vec<&str> = entity_evidence
                .get(&eid)
                .map(|sigs| sigs.iter().map(|s| s.signal_type.as_str()).collect())
                .unwrap_or_default();
            if signals.is_empty() {
                continue;
            }
            let (entity_name, entity_region, _entity_type) = company_names
                .get(&eid)
                .cloned()
                .unwrap_or_else(|| ("Unknown".into(), None, None));

            for pattern in &patterns {
                let triggers_present = pattern
                    .triggers
                    .iter()
                    .all(|t| signals.iter().any(|s| s.contains(t.as_str())));
                if !triggers_present {
                    continue;
                }
                let prediction = pattern.predict(*entity_uuid, &entity_name);
                let title = format!(
                    "Predictive: {} likely within {} days for {}",
                    prediction.predicted_outcome, prediction.prediction_window_days, entity_name
                );
                let summary = format!(
                    "{}.\n\nPrediction: {} is {:.0}% likely to experience '{}' within {} days \
                     (95% CI: {:.0}%–{:.0}%), based on {} historical observations.\n\n\
                     Pattern: {}",
                    pattern.prediction_statement(),
                    entity_name,
                    prediction.probability * 100.0,
                    prediction.predicted_outcome,
                    prediction.prediction_window_days,
                    prediction.probability_ci_lower * 100.0,
                    prediction.probability_ci_upper * 100.0,
                    pattern.observation_count,
                    pattern.description,
                );
                let region_param = entity_region.as_deref();
                let _ = store
                    .insert_insight(
                        &title,
                        &summary,
                        Some("predictive_forward"),
                        region_param,
                        Some(prediction.probability),
                        None,
                        Some(vec![*entity_uuid]),
                        Some(vec!["predictive".to_string(), pattern.id.clone()]),
                        None,
                    )
                    .await;
                predictive_count += 1;
            }
        }
        if predictive_count > 0 {
            tracing::info!(
                count = predictive_count,
                "recipe_fire: predictive insights inserted"
            );
            insights_inserted += predictive_count;
        }

        // ── 2. Hypothesis ACH scoring per entity ────────────────────────
        let mut hypothesis_candidates = Vec::new();
        for entity_uuid in &all_entity_uuids {
            let eid = entity_uuid.to_string();
            let evidence_signals = match entity_evidence.get(&eid) {
                Some(sigs) if sigs.len() >= 2 => sigs,
                _ => continue,
            };
            let (entity_name, entity_region, _) = company_names
                .get(&eid)
                .cloned()
                .unwrap_or_else(|| ("Unknown".into(), None, None));

            let mut tracker = EntityHypothesisTracker::new(*entity_uuid, entity_name.clone());
            for sig in evidence_signals {
                let ev_type = HypothesisEvidenceType::from_signal_type(&sig.signal_type);
                tracker.update_with_evidence(ev_type);
            }
            let summary = tracker.summary();
            let Some((title, insight_summary, leading_posterior, total_evidence)) =
                build_hypothesis_ach_candidate(&summary, evidence_signals)
            else {
                continue;
            };
            if !passes_shared_insight_quality_gate(&title, &insight_summary, Some("hypothesis_ach"))
            {
                continue;
            }
            hypothesis_candidates.push((
                leading_posterior,
                total_evidence,
                title,
                insight_summary,
                entity_region,
                *entity_uuid,
            ));
        }

        hypothesis_candidates.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.cmp(&a.1))
        });

        let mut hypothesis_count: u64 = 0;
        for (
            leading_posterior,
            _total_evidence,
            title,
            insight_summary,
            entity_region,
            entity_uuid,
        ) in hypothesis_candidates.into_iter().take(8)
        {
            let region_param = entity_region.as_deref();
            let _ = store
                .insert_insight(
                    &title,
                    &insight_summary,
                    Some("hypothesis_ach"),
                    region_param,
                    Some(leading_posterior),
                    None,
                    Some(vec![entity_uuid]),
                    Some(vec!["hypothesis".to_string(), "ach".to_string()]),
                    None,
                )
                .await;
            hypothesis_count += 1;
        }
        if hypothesis_count > 0 {
            tracing::info!(
                count = hypothesis_count,
                "recipe_fire: hypothesis ACH insights inserted"
            );
            insights_inserted += hypothesis_count;
        }

        // ── 3. Cross-region arbitrage detection ─────────────────────────
        let detector = ArbitrageDetector::new(20.0);
        let profiles = arbitrage_default_profiles();
        let mut opportunities = detector.scan_all(&profiles);
        opportunities.sort_by(|a, b| {
            b.cost_advantage_pct
                .partial_cmp(&a.cost_advantage_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seen_advantaged_regions = std::collections::HashSet::new();
        let mut arbitrage_count: u64 = 0;
        for opp in opportunities
            .into_iter()
            .filter(|opp| seen_advantaged_regions.insert(opp.advantaged_region.clone()))
            .take(4)
        {
            let _ = store
                .insert_insight(
                    &opp.title,
                    &opp.description,
                    Some("arbitrage_cost_window"),
                    Some(&opp.advantaged_region),
                    Some((opp.cost_advantage_pct / 100.0).min(0.99)),
                    None,
                    None,
                    Some(vec![
                        "arbitrage".to_string(),
                        format!("region:{}", opp.advantaged_region),
                        format!("market:{}", opp.target_market),
                    ]),
                    None,
                )
                .await;
            arbitrage_count += 1;
        }
        if arbitrage_count > 0 {
            tracing::info!(
                count = arbitrage_count,
                "recipe_fire: arbitrage insights inserted"
            );
            insights_inserted += arbitrage_count;
        }

        // ── 4. Competitive comparison matrix ────────────────────────────
        // Build Starz (self-company) capabilities from cap_feats and cert_feats
        let starz_uuid = all_entity_uuids.iter().find(|id| {
            company_names
                .get(&id.to_string())
                .and_then(|(_name, _region, ctype)| ctype.as_ref())
                .map(|t| t == "self" || t == "starz")
                .unwrap_or(false)
        });
        if let Some(starz_id) = starz_uuid {
            let starz_eid = starz_id.to_string();
            let starz_caps: Vec<(String, String, bool)> = cap_feats
                .iter()
                .filter(|(eid, _, _)| eid == starz_id)
                .map(|(_, cap, count)| (cap.clone(), "production".to_string(), *count > 0))
                .collect();

            let competitor_data: Vec<(String, String, f64, f64, Vec<(String, String, bool)>)> =
                company_names
                    .iter()
                    .filter(|(eid, _)| *eid != &starz_eid)
                    .take(10)
                    .map(|(eid, (name, region, _))| {
                        let caps: Vec<(String, String, bool)> = cap_feats
                            .iter()
                            .filter(|(e, _, _)| e.to_string() == *eid)
                            .map(|(_, cap, count)| (cap.clone(), "claimed".to_string(), *count > 0))
                            .collect();
                        let ctx = entity_contexts.get(eid);
                        let threat = ctx.and_then(|c| c.threat_score).unwrap_or(0.5);
                        let overlap = ctx.and_then(|c| c.overlap_score).unwrap_or(0.3);
                        (
                            name.clone(),
                            region.clone().unwrap_or_default(),
                            threat,
                            overlap,
                            caps,
                        )
                    })
                    .collect();

            if !starz_caps.is_empty() && !competitor_data.is_empty() {
                let matrix = build_comparison_matrix(&starz_caps, &competitor_data);
                let unique_str = matrix.summary.starz_unique_capabilities.join(", ");
                let gaps_str = matrix.summary.starz_cert_gap.join(", ");
                let advantages_str = matrix.summary.starz_cert_advantage.join(", ");
                let regional_str = matrix.summary.regional_advantages.join("; ");
                let title = format!(
                    "Competitive Landscape: {} unique strengths vs {} competitors",
                    matrix.summary.starz_unique_capabilities.len(),
                    matrix.competitors.len()
                );
                let summary = format!(
                    "Competitive comparison across {} capabilities and {} competitors.\n\n\
                     Unique Starz capabilities: {}\n\
                     Certification advantages: {}\n\
                     Certification gaps to close: {}\n\
                     Regional advantages: {}\n\n\
                     Shared capabilities: {} | Competitor-only: {}",
                    matrix.capability_rows.len(),
                    matrix.competitors.len(),
                    if unique_str.is_empty() {
                        "none"
                    } else {
                        &unique_str
                    },
                    if advantages_str.is_empty() {
                        "none"
                    } else {
                        &advantages_str
                    },
                    if gaps_str.is_empty() {
                        "none"
                    } else {
                        &gaps_str
                    },
                    if regional_str.is_empty() {
                        "none identified"
                    } else {
                        &regional_str
                    },
                    matrix.summary.common_capabilities.len(),
                    matrix.summary.competitor_unique_capabilities.len(),
                );
                let _ = store
                    .insert_insight(
                        &title,
                        &summary,
                        Some("competitive_comparison"),
                        None,
                        Some(0.85),
                        None,
                        Some(vec![*starz_id]),
                        Some(vec!["comparison".to_string(), "competitive".to_string()]),
                        None,
                    )
                    .await;
                insights_inserted += 1;
                tracing::info!("recipe_fire: competitive comparison insight inserted");
            }
        }
    }

    tracing::info!(
        total_candidates = total_candidates,
        skipped_low_conf = skipped_low_conf,
        skipped_dedup = skipped_dedup,
        skipped_cross_run = skipped_cross_run,
        inserted = insights_inserted,
        warnings = warnings_inserted,
        "recipe_fire: run complete"
    );

    run.succeed(
        insights_inserted,
        &format!(
            "recipe_fire: {} candidate(s), inserted {} insight(s), {} warning(s) (skipped {} low-conf, {} dedup, {} cross-run)",
            total_candidates,
            insights_inserted,
            warnings_inserted,
            skipped_low_conf,
            skipped_dedup,
            skipped_cross_run,
        ),
    );
    run
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods, clippy::field_reassign_with_default)]

    use super::recipe_warning_severity;

    #[cfg(feature = "llm")]
    use super::{
        apply_bias_mitigation, build_hypothesis_ach_candidate, should_emit_llm_warning,
        EvidenceSignal,
    };
    #[cfg(feature = "llm")]
    use apex_insights::hypothesis::HypothesisSummary;
    #[cfg(feature = "llm")]
    use chrono::Utc;
    #[cfg(feature = "llm")]
    use uuid::Uuid;

    #[test]
    fn suppresses_low_signal_dns_hygiene_recipe_alerts() {
        assert_eq!(
            recipe_warning_severity("N002", "cybersecurity_threat", "warning", 0.88, 0.7),
            None
        );
        assert_eq!(
            recipe_warning_severity("N002", "cybersecurity_threat", "warning", 0.93, 0.8),
            Some("medium")
        );
    }

    #[test]
    fn promotes_high_signal_business_categories() {
        assert_eq!(
            recipe_warning_severity("A001", "demand_procurement", "info", 0.8, 0.65),
            Some("medium")
        );
        assert_eq!(
            recipe_warning_severity("G001", "regulatory_policy", "info", 0.78, 0.6),
            Some("high")
        );
    }

    #[cfg(feature = "llm")]
    fn make_signal(signal_type: &str, title: &str, source_url: &str) -> EvidenceSignal {
        EvidenceSignal {
            title: title.to_string(),
            description: format!("{} description", title),
            source_url: source_url.to_string(),
            signal_type: signal_type.to_string(),
            extracted_facts: vec![title.to_string()],
            date_context: Some("2026-03-18".to_string()),
            relevance_score: 0.7,
        }
    }

    #[cfg(feature = "llm")]
    fn make_hypothesis_summary(
        leading: &str,
        leading_posterior: f64,
        runner_up: &str,
        runner_up_posterior: f64,
        total_evidence: u32,
    ) -> HypothesisSummary {
        HypothesisSummary {
            entity_id: Uuid::new_v4(),
            entity_name: "Flex Ltd".to_string(),
            leading_hypothesis: Some(leading.to_string()),
            leading_posterior: Some(leading_posterior),
            all_posteriors: vec![
                (leading.to_string(), leading_posterior),
                (runner_up.to_string(), runner_up_posterior),
                (
                    "Contracting".to_string(),
                    (1.0 - leading_posterior - runner_up_posterior).max(0.0),
                ),
            ],
            total_evidence,
            top_diagnostic: Some(
                "collect supplier-change evidence to distinguish pivoting from expansion"
                    .to_string(),
            ),
            updated_at: Utc::now(),
        }
    }

    #[cfg(feature = "llm")]
    #[test]
    fn llm_warning_requires_grounding_evidence() {
        let evidence = vec![
            make_signal("warning", "Derived warning", "https://example.com/warning"),
            make_signal("capability", "Capability", ""),
        ];

        assert!(!should_emit_llm_warning(
            "high",
            "Approach the buyer by Q2 2026 [1].",
            &evidence,
            0.82,
            true,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn llm_warning_accepts_grounded_non_critical_warning_when_insight_inserted() {
        let evidence = vec![
            make_signal(
                "TenderNotice",
                "Tender announced",
                "https://example.com/tender",
            ),
            make_signal(
                "WebChange",
                "Supplier qualification update",
                "https://example.com/update",
            ),
        ];

        assert!(should_emit_llm_warning(
            "high",
            "Approach the procurement lead before supplier review [1].",
            &evidence,
            0.74,
            true,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn llm_warning_rejects_non_critical_warning_without_visible_insight() {
        let evidence = vec![
            make_signal(
                "TenderNotice",
                "Tender announced",
                "https://example.com/tender",
            ),
            make_signal(
                "WebChange",
                "Supplier qualification update",
                "https://example.com/update",
            ),
        ];

        assert!(!should_emit_llm_warning(
            "high",
            "Approach the procurement lead before supplier review [1].",
            &evidence,
            0.74,
            false,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn hypothesis_ach_candidate_rejects_low_separation() {
        let summary = make_hypothesis_summary("Pivoting", 0.69, "Expanding capability", 0.58, 8);
        let evidence = vec![
            make_signal(
                "LeadershipChange",
                "Appointed new EV systems vice president",
                "https://example.com/leadership",
            ),
            make_signal(
                "SupplyChainChange",
                "Shifted sourcing for power modules to new suppliers",
                "https://example.com/sourcing",
            ),
            make_signal(
                "NewCertification",
                "Secured rail-electronics certification for new product lines",
                "https://example.com/cert",
            ),
        ];

        assert!(build_hypothesis_ach_candidate(&summary, &evidence).is_none());
    }

    #[cfg(feature = "llm")]
    #[test]
    fn hypothesis_ach_candidate_uses_concrete_evidence_in_summary() {
        let summary = make_hypothesis_summary("Pivoting", 0.78, "Expanding capability", 0.44, 9);
        let evidence = vec![
            make_signal(
                "LeadershipChange",
                "Appointed new EV systems vice president",
                "https://example.com/leadership",
            ),
            make_signal(
                "SupplyChainChange",
                "Shifted sourcing for power modules to new suppliers",
                "https://example.com/sourcing",
            ),
            make_signal(
                "NewCertification",
                "Secured rail-electronics certification for new product lines",
                "https://example.com/cert",
            ),
        ];

        let (title, body, confidence, total_evidence) =
            build_hypothesis_ach_candidate(&summary, &evidence).expect("expected ACH candidate");

        assert!(title.contains("leans pivoting"));
        assert!(body.contains("Appointed new EV systems vice president"));
        assert!(body.contains("Shifted sourcing for power modules to new suppliers"));
        assert!(body.contains("Next discriminating evidence"));
        assert!(!body.contains("Hypothesis Posteriors:"));
        assert_eq!(confidence, 0.78);
        assert_eq!(total_evidence, 9);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn bias_mitigation_reduces_confidence_without_emitting_explicit_insight() {
        let evidence = vec![
            make_signal(
                "TenderNotice",
                "Tender announced",
                "https://example.com/tender",
            ),
            make_signal(
                "WebChange",
                "Supplier qualification update",
                "https://example.com/update",
            ),
            make_signal(
                "TradeShow",
                "Trade show participation",
                "https://example.com/show",
            ),
        ];

        let adjustment = apply_bias_mitigation(
            "Digi-Key",
            "Digi-Key expands sourcing program",
            "Digi-Key appears to be expanding sourcing activity based on recent supplier and demand signals.",
            &evidence,
            0.82,
        )
        .expect("expected bias adjustment");

        assert!(adjustment.confidence_reduction >= 0.05);
        assert!(matches!(
            adjustment.tag,
            "bias:challenged" | "bias:strong_counter"
        ));
    }
}
