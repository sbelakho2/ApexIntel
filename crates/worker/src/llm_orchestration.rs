#![cfg_attr(test, allow(dead_code))]

//! LLM Orchestration Module
//!
//! Centralizes LLM interaction patterns including:
//! - Quality gate decision types and ensemble logic
//! - Prompt construction for insight generation
//! - Response validation and parsing
//! - Retry guidance generation

#[cfg(feature = "llm")]
use chrono::{DateTime, Utc};
#[cfg(feature = "llm")]
use std::collections::HashMap;
#[cfg(feature = "llm")]
use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Quality Gate Types
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(super) struct QualityGateDecision {
    pub gate_name: &'static str,
    pub score: f32,
    pub threshold: f32,
    pub failed: bool,
    pub veto: bool,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(super) struct QualityGateReview {
    pub gate_name: &'static str,
    pub blocked: bool,
    pub human_confirmed_block: bool,
    pub reviewed_at: DateTime<Utc>,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct QualityGateWeeklyMetrics {
    pub gate_name: String,
    pub week_start: DateTime<Utc>,
    pub total_reviews: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision_observed: f64,
    pub recall_observed: f64,
    pub alert_precision: bool,
    pub alert_recall: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Quality Gate Decision Functions
// ─────────────────────────────────────────────────────────────────────────────

/// Create a quality gate requirement decision (soft gate).
#[cfg(feature = "llm")]
pub(super) fn quality_gate_requirement(
    gate_name: &'static str,
    observed: f32,
    minimum: f32,
) -> QualityGateDecision {
    QualityGateDecision {
        gate_name,
        score: observed,
        threshold: minimum,
        failed: observed < minimum,
        veto: false,
    }
}

/// Create a quality gate blocker decision (can be veto or soft).
#[cfg(feature = "llm")]
pub(super) fn quality_gate_blocker(
    gate_name: &'static str,
    triggered: bool,
    veto: bool,
) -> QualityGateDecision {
    QualityGateDecision {
        gate_name,
        score: if triggered { 1.0 } else { 0.0 },
        threshold: 0.5,
        failed: triggered,
        veto,
    }
}

/// Emit structured tracing events for quality gate decisions.
#[cfg(feature = "llm")]
pub(super) fn emit_quality_gate_decisions(
    entity_name: &str,
    category: &str,
    attempt: usize,
    decisions: &[QualityGateDecision],
) {
    let timestamp = Utc::now().to_rfc3339();
    for decision in decisions {
        crate::observability::WORKER_METRICS.record_gate_evaluation(
            decision.gate_name,
            !decision.failed,
        );
        tracing::info!(
            entity = %entity_name,
            category,
            attempt,
            gate_name = decision.gate_name,
            score = decision.score,
            threshold = decision.threshold,
            decision = if decision.failed { "reject" } else { "pass" },
            veto = decision.veto,
            timestamp = %timestamp,
            "LLM quality gate decision"
        );
    }
}

/// Ensemble logic: pass if no veto and fewer than 2 soft failures.
#[cfg(feature = "llm")]
pub(super) fn quality_gate_passes_ensemble(decisions: &[QualityGateDecision]) -> bool {
    let veto_rejected = decisions.iter().any(|d| d.failed && d.veto);
    let ensemble_rejections = decisions.iter().filter(|d| d.failed && !d.veto).count();
    !veto_rejected && ensemble_rejections < 3
}

/// Summarize quality gate reviews into weekly metrics with alerting.
#[cfg(feature = "llm")]
pub(super) fn summarize_quality_gate_reviews(
    reviews: &[QualityGateReview],
) -> Vec<QualityGateWeeklyMetrics> {
    let mut grouped: HashMap<(String, DateTime<Utc>), Vec<&QualityGateReview>> = HashMap::new();
    for review in reviews {
        let week_start = start_of_review_week(review.reviewed_at);
        grouped
            .entry((review.gate_name.to_string(), week_start))
            .or_default()
            .push(review);
    }

    let mut metrics: Vec<QualityGateWeeklyMetrics> = grouped
        .into_iter()
        .map(|((gate_name, week_start), items)| {
            let true_positives = items
                .iter()
                .filter(|r| r.blocked && r.human_confirmed_block)
                .count();
            let false_positives = items
                .iter()
                .filter(|r| r.blocked && !r.human_confirmed_block)
                .count();
            let false_negatives = items
                .iter()
                .filter(|r| !r.blocked && r.human_confirmed_block)
                .count();

            let precision_observed = if true_positives + false_positives == 0 {
                1.0
            } else {
                true_positives as f64 / (true_positives + false_positives) as f64
            };
            let recall_observed = if true_positives + false_negatives == 0 {
                1.0
            } else {
                true_positives as f64 / (true_positives + false_negatives) as f64
            };

            QualityGateWeeklyMetrics {
                gate_name,
                week_start,
                total_reviews: items.len(),
                true_positives,
                false_positives,
                false_negatives,
                precision_observed,
                recall_observed,
                alert_precision: precision_observed < 0.70,
                alert_recall: recall_observed < 0.70,
            }
        })
        .collect();

    metrics.sort_by(|a, b| {
        a.gate_name
            .cmp(&b.gate_name)
            .then(a.week_start.cmp(&b.week_start))
    });
    metrics
}

/// Get the start of the week for a review timestamp (Monday 00:00 UTC).
#[cfg(feature = "llm")]
fn start_of_review_week(ts: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Datelike, Duration, TimeZone};
    let date = ts.date_naive();
    let weekday = date.weekday().num_days_from_monday();
    let monday = date - Duration::days(weekday as i64);
    Utc.from_utc_datetime(&monday.and_hms_opt(0, 0, 0).unwrap())
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM Response Types
// ─────────────────────────────────────────────────────────────────────────────

/// Parsed response from LLM insight generation.
#[cfg(feature = "llm")]
#[derive(serde::Deserialize, Debug)]
pub(super) struct LlmInsightResponse {
    pub headline: String,
    pub narrative: String,
    #[serde(default, deserialize_with = "deserialize_recommendation")]
    pub recommendation: Option<String>,
    pub confidence: f64,
}

/// Accept recommendation as string, array of strings, or array of objects.
#[cfg(feature = "llm")]
fn deserialize_recommendation<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let val: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match val {
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Array(arr)) => {
            let items: Vec<String> = arr
                .into_iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(s),
                    serde_json::Value::Object(map) => {
                        let parts: Vec<String> =
                            ["action", "owner", "deadline", "description", "task"]
                                .iter()
                                .filter_map(|key| {
                                    map.get(*key)
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                })
                                .collect();
                        if parts.is_empty() {
                            let all_strings: Vec<String> = map
                                .values()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect();
                            if all_strings.is_empty() {
                                None
                            } else {
                                Some(all_strings.join(" — "))
                            }
                        } else {
                            Some(parts.join(" — "))
                        }
                    }
                    _ => None,
                })
                .collect();
            if items.is_empty() {
                Ok(None)
            } else {
                Ok(Some(items.join(". ")))
            }
        }
        Some(serde_json::Value::Object(map)) => {
            let parts: Vec<String> = map
                .values()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if parts.is_empty() {
                Ok(None)
            } else {
                Ok(Some(parts.join(" — ")))
            }
        }
        Some(serde_json::Value::Null) | None => Ok(None),
        _ => Ok(None),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt Construction
// ─────────────────────────────────────────────────────────────────────────────

/// Build rich entity profile string with competitive context.
#[cfg(feature = "llm")]
pub(super) fn build_entity_profile(entity_ctx: &EntityContext, is_public_sector: bool) -> String {
    let mut profile_parts: Vec<String> = Vec::new();
    let type_str = entity_ctx.entity_type.as_deref().unwrap_or("company");

    if type_str.to_lowercase().contains("government") {
        profile_parts.push(format!(
            "{} is a government/public sector entity in {}.",
            entity_ctx.name, entity_ctx.region
        ));
    } else {
        let mut desc = format!(
            "{} is a {} based in {}.",
            entity_ctx.name, type_str, entity_ctx.region
        );
        if let Some(rev) = entity_ctx.revenue_estimate_usd {
            desc.push_str(&format!(
                " Estimated revenue: ~${:.0}M.",
                rev as f64 / 1_000_000.0
            ));
        }
        if let Some(emp) = entity_ctx.employee_estimate {
            desc.push_str(&format!(" ~{} employees.", emp));
        }
        profile_parts.push(desc);
    }

    if let Some(domain) = &entity_ctx.domain {
        profile_parts.push(format!("Domain: {}", domain));
    }
    if !entity_ctx.industry_tags.is_empty() {
        profile_parts.push(format!(
            "Industry focus: {}.",
            entity_ctx.industry_tags.join(", ")
        ));
    }
    if !entity_ctx.certifications.is_empty() {
        let cert_str = entity_ctx
            .certifications
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Certifications: {}", cert_str));
    }
    if !entity_ctx.capabilities.is_empty() {
        let cap_str = entity_ctx
            .capabilities
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Capabilities: {}", cap_str));
    }
    if !entity_ctx.key_persons.is_empty() {
        let poi_str = entity_ctx
            .key_persons
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Key personnel: {}", poi_str));
    }
    if !entity_ctx.sites_summary.is_empty() {
        let sites_str = entity_ctx
            .sites_summary
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Sites/Facilities: {}", sites_str));
    }

    // Competitive positioning
    let mut comp_parts: Vec<String> = Vec::new();
    if let Some(ts) = entity_ctx.threat_score {
        comp_parts.push(format!("Threat score: {:.2}", ts));
    }
    if let Some(os) = entity_ctx.overlap_score {
        comp_parts.push(format!("Market overlap: {:.2}", os));
    }
    if let Some(sr) = entity_ctx.strategic_relevance {
        comp_parts.push(format!("Strategic relevance: {:.2}", sr));
    }
    if !entity_ctx.competitor_names.is_empty() {
        comp_parts.push(format!(
            "Linked competitors: {}",
            entity_ctx.competitor_names.join(", ")
        ));
    }
    if !comp_parts.is_empty() {
        profile_parts.push(format!("Competitive profile: {}", comp_parts.join(". ")));
    }

    if !entity_ctx.recent_changes.is_empty() {
        let changes_str = entity_ctx
            .recent_changes
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Recent changes: {}", changes_str));
    }
    if !entity_ctx.competitor_events.is_empty() {
        let events_str = entity_ctx
            .competitor_events
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Competitor intelligence: {}", events_str));
    }

    // Entity classification
    if entity_ctx.is_competitor {
        profile_parts.push(
            "⚠️ ENTITY CLASSIFICATION: DIRECT EMS COMPETITOR — Do NOT recommend offering our services \
to this company. Analyse their weaknesses, identify which of their customers are underserved, \
and recommend approaching those customers instead.".to_string()
        );
    } else if is_public_sector {
        profile_parts.push(
            "🏛️ ENTITY CLASSIFICATION: PUBLIC-SECTOR BODY — treat this as an institutional account, not a default manufacturing prospect. Direct supplier outreach is only justified when the evidence explicitly names a procurement, tender, supplier qualification event, or hardware/equipment program.".to_string()
        );
    } else {
        profile_parts.push(
            "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity."
                .to_string(),
        );
    }

    profile_parts.join("\n")
}

/// Build formatted evidence text from signals.
#[cfg(feature = "llm")]
pub(super) fn build_evidence_text(evidence_signals: &[EvidenceSignal], max_items: usize) -> String {
    let mut sorted_evidence: Vec<_> = evidence_signals.iter().collect();
    sorted_evidence.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted_evidence
        .iter()
        .take(max_items)
        .enumerate()
        .map(|(i, sig)| {
            let mut parts = vec![format!("[{}] {} ({})", i + 1, sig.title, sig.signal_type)];
            if !sig.extracted_facts.is_empty() {
                parts.push(format!("   Key facts: {}", sig.extracted_facts.join("; ")));
            }
            if !sig.description.is_empty() {
                let desc_preview = if sig.description.len() > 400 {
                    format!("{}...", &sig.description[..400])
                } else {
                    sig.description.clone()
                };
                parts.push(format!("   Summary: {}", desc_preview));
            }
            parts.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Get suggestion axes based on entity type and category.
#[cfg(feature = "llm")]
pub(super) fn get_suggestion_axes(
    is_competitor: bool,
    is_public_sector: bool,
    category: &str,
) -> &'static str {
    match (is_competitor, is_public_sector, category) {
        (true, _, _) => "competitive displacement, customer rescue, qualification wedge, pricing wedge, regional footprint positioning, executive account planning",
        (_, true, "brand_sentiment") => "institutional credibility assessment, procurement scrutiny mapping, stakeholder messaging review, account dependency review, scenario planning, executive briefing",
        (_, true, "regulatory_policy") | (_, true, "geopolitical_analysis") => "policy impact mapping, qualification planning, account dependency review, stakeholder outreach, executive scenario planning, procurement path verification",
        (_, true, "security_compliance") | (_, true, "cybersecurity_threat") | (_, true, "quality_compliance") => "official-surface validation, supplier access review, containment planning, stakeholder brief, assurance planning, executive escalation",
        (_, true, _) => "account planning, stakeholder mapping, institutional process review, evidence verification, qualification planning, executive briefing",
        (_, false, "demand_procurement") | (_, false, "customer_rfq") => "revenue capture, qualification readiness, prototype or NPI entry, pricing leverage, regional footprint positioning, executive sponsor mapping",
        (_, false, "supply_chain_risk") => "continuity protection, dual-source qualification, customer assurance, design migration, regional rerouting, executive risk briefing",
        (_, false, "regulatory_policy") | (_, false, "geopolitical_analysis") => "regulatory posture, export-control routing, nearshoring, customer communication, qualification planning, executive scenario planning",
        (_, false, "strategic_poi") | (_, false, "talent_ip") => "POI mapping, early project engagement, partnership proposal, competitive positioning, executive outreach, stakeholder timing",
        (_, false, "security_compliance") | (_, false, "cybersecurity_threat") | (_, false, "quality_compliance") => "audit readiness, security assurance, supplier governance, containment planning, customer reassurance, executive escalation",
        _ => "revenue capture, resilience, competitive positioning, pricing leverage, regional expansion, executive planning",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Response Validation
// ─────────────────────────────────────────────────────────────────────────────

/// Banned generic phrases that indicate passive analysis.
#[cfg(feature = "llm")]
pub(super) const GENERIC_PHRASES: &[&str] = &[
    "continue monitoring",
    "monitor the situation",
    "various developments",
    "warranting focused analysis",
    "further developments",
    "remains to be seen",
    "time will tell",
    "developments warrant attention",
    "stay informed",
    "keep an eye on",
    "in conclusion",
    "it is important to note",
    "overall",
];

/// Malformed output fragments that indicate template leakage.
#[cfg(feature = "llm")]
pub(super) const MALFORMED_FRAGMENTS: &[&str] = &[
    "intelligence veracity:",
    "additional source reporting:",
    "signal themes detected:",
    "assessment: moderate-high confidenc",
    "[object object]",
    "undefined",
    "{{",
    "}}",
];

/// Placeholder patterns that indicate incomplete generation.
#[cfg(feature = "llm")]
pub(super) const PLACEHOLDER_PATTERNS: &[&str] = &[
    "[company ",
    "[specific ",
    "[date]",
    "[competitor ",
    "[our ",
    "[their ",
    "[service",
    "[product",
    "[client ",
    "[customer ",
    "[contact ",
];

/// Check if text contains causal language indicators.
#[cfg(feature = "llm")]
pub(super) fn has_causal_language(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["because", "therefore", "as a result", "which means", "implies", "drives", "leads to"]
        .iter()
        .any(|p| lower.contains(p))
}

/// Check if text contains counterfactual reasoning.
#[cfg(feature = "llm")]
pub(super) fn has_counterfactual(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("if ") && (lower.contains(" would ") || lower.contains(" could "))
}

/// Determine failure reasons for retry guidance.
#[cfg(feature = "llm")]
pub(super) fn determine_failure_reasons(
    has_reasoning_depth: bool,
    recommendation_has_deadline: bool,
    readable_narrative: bool,
    readable_recommendation: bool,
    is_generic: bool,
    malformed: bool,
    unnamed_customer_targeting: bool,
    unsupported_certification_escalation: bool,
    unsupported_security_escalation: bool,
) -> Vec<&'static str> {
    let mut reasons = Vec::new();

    if !has_reasoning_depth {
        reasons.push("reasoning");
    }
    if !recommendation_has_deadline {
        reasons.push("timing");
    }
    if !readable_narrative || !readable_recommendation || is_generic || malformed {
        reasons.push("readability");
    }
    if unnamed_customer_targeting {
        reasons.push("unnamed_customer_targeting");
    }
    if unsupported_certification_escalation {
        reasons.push("certification_escalation");
    }
    if unsupported_security_escalation {
        reasons.push("security_escalation");
    }

    reasons
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry Guidance
// ─────────────────────────────────────────────────────────────────────────────

/// Build retry guidance message for LLM based on previous failure reasons.
#[cfg(feature = "llm")]
pub(super) fn build_llm_retry_guidance(
    entity_ctx: &EntityContext,
    category: &str,
    previous_failure_reasons: &[&'static str],
) -> Option<String> {
    if previous_failure_reasons.is_empty() {
        return None;
    }

    let mut guidance: Vec<String> = Vec::new();
    guidance.push(format!(
        "The previous draft for {} in category {} failed quality checks. Rewrite it and fix every issue below.",
        entity_ctx.name, category
    ));

    for reason in previous_failure_reasons {
        match *reason {
            "timing" => guidance.push(
                "Recommendation timing was too vague. Include an explicit timing anchor such as 'this quarter', 'within 30 days', 'before <date>', or another concrete window supported by the evidence.".to_string(),
            ),
            "unnamed_customer_targeting" => guidance.push(
                "Do not target unnamed customer cohorts like 'their medical customers'. Either recommend direct action on the analyzed entity or name only companies, roles, facilities, or programs explicitly present in the evidence or entity profile.".to_string(),
            ),
            "certification_escalation" => guidance.push(
                "Do not turn generic certification notices or stale dates into contract-loss, qualification-failure, or customer-switch claims unless the evidence explicitly says that happened.".to_string(),
            ),
            "security_escalation" => guidance.push(
                "Do not turn DNS posture, missing SPF/DKIM/DMARC, or lookalike-domain findings into breach, customer-loss, or program-exclusion claims unless the evidence explicitly links them.".to_string(),
            ),
            "readability" => guidance.push(
                "Write plain business prose with no template phrasing, no labels, and no repetitive restatements. Every sentence should add a new fact, implication, or action.".to_string(),
            ),
            "reasoning" => guidance.push(
                "Make the causal chain explicit with clear 'because/therefore' logic or a concrete counterfactual based on the evidence.".to_string(),
            ),
            _ => guidance.push(
                "Keep the output concrete, evidence-cited, and commercially actionable.".to_string(),
            ),
        }
    }

    Some(guidance.join("\n- ").replacen('\n', "\n- ", 1))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::*;

    #[test]
    fn test_quality_gate_requirement_passes() {
        let decision = quality_gate_requirement("word_count", 100.0, 85.0);
        assert!(!decision.failed);
        assert!(!decision.veto);
    }

    #[test]
    fn test_quality_gate_requirement_fails() {
        let decision = quality_gate_requirement("word_count", 50.0, 85.0);
        assert!(decision.failed);
        assert!(!decision.veto);
    }

    #[test]
    fn test_quality_gate_blocker_veto() {
        let decision = quality_gate_blocker("security_escalation", true, true);
        assert!(decision.failed);
        assert!(decision.veto);
    }

    #[test]
    fn test_ensemble_passes_no_failures() {
        let decisions = vec![
            quality_gate_requirement("test1", 1.0, 0.5),
            quality_gate_requirement("test2", 1.0, 0.5),
        ];
        assert!(quality_gate_passes_ensemble(&decisions));
    }

    #[test]
    fn test_ensemble_fails_with_veto() {
        let decisions = vec![
            quality_gate_requirement("test1", 1.0, 0.5),
            quality_gate_blocker("veto_gate", true, true),
        ];
        assert!(!quality_gate_passes_ensemble(&decisions));
    }

    #[test]
    fn test_ensemble_fails_with_three_soft_failures() {
        let decisions = vec![
            quality_gate_requirement("test1", 0.0, 0.5),
            quality_gate_requirement("test2", 0.0, 0.5),
            quality_gate_requirement("test3", 0.0, 0.5),
        ];
        assert!(!quality_gate_passes_ensemble(&decisions));
    }

    #[test]
    fn test_ensemble_passes_with_two_soft_failures() {
        let decisions = vec![
            quality_gate_requirement("test1", 0.0, 0.5),
            quality_gate_requirement("test2", 0.0, 0.5),
            quality_gate_requirement("test3", 1.0, 0.5),
        ];
        assert!(quality_gate_passes_ensemble(&decisions));
    }

    #[test]
    fn test_ensemble_passes_with_one_soft_failure() {
        let decisions = vec![
            quality_gate_requirement("test1", 0.0, 0.5),
            quality_gate_requirement("test2", 1.0, 0.5),
        ];
        assert!(quality_gate_passes_ensemble(&decisions));
    }

    #[test]
    fn test_has_causal_language() {
        assert!(has_causal_language("Revenue dropped because of supply issues"));
        assert!(has_causal_language("Therefore, we recommend action"));
        assert!(!has_causal_language("The company is growing"));
    }

    #[test]
    fn test_has_counterfactual() {
        assert!(has_counterfactual("If they lose the contract, they would face layoffs"));
        assert!(has_counterfactual("If supply disrupts, margins could drop"));
        assert!(!has_counterfactual("The contract is at risk"));
    }

    #[test]
    fn test_retry_guidance_empty() {
        let ctx = EntityContext {
            name: "Test Corp".to_string(),
            region: "US".to_string(),
            is_competitor: false,
            entity_type: None,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: None,
        };
        assert!(build_llm_retry_guidance(&ctx, "test", &[]).is_none());
    }

    #[test]
    fn test_retry_guidance_with_timing() {
        let ctx = EntityContext {
            name: "Test Corp".to_string(),
            region: "US".to_string(),
            is_competitor: false,
            entity_type: None,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: None,
        };
        let guidance = build_llm_retry_guidance(&ctx, "test", &["timing"]).unwrap();
        assert!(guidance.contains("timing"));
        assert!(guidance.contains("this quarter"));
    }

    #[test]
    fn test_suggestion_axes_competitor() {
        let axes = get_suggestion_axes(true, false, "any");
        assert!(axes.contains("competitive displacement"));
        assert!(axes.contains("customer rescue"));
    }

    #[test]
    fn test_suggestion_axes_public_sector() {
        let axes = get_suggestion_axes(false, true, "regulatory_policy");
        assert!(axes.contains("policy impact"));
        assert!(axes.contains("qualification planning"));
    }
}