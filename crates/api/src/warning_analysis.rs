//! Shared warning analysis service (audit P0 #40).
//!
//! The web panel and the JSON API both run the *same* real analysis: the
//! evidence-gathering and LLM call live here so neither surface can fall back
//! to a placeholder panel. The model configuration is injected by the binary
//! (see `WarningAnalysisModel`), never read from ambient state here.

#![cfg(feature = "llm")]

use anyhow::{Context, Result};
use uuid::Uuid;

use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
use apex_store::postgres::{PgStore, WarningRow};

/// Model configuration used for warning analysis, injected as an axum
/// extension by the binary when the LLM runtime is configured.
#[derive(Clone)]
pub struct WarningAnalysisModel {
    pub primary: ModelConfig,
}

/// The real, structured result of one warning analysis run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WarningAnalysisOutput {
    /// First generated paragraph: what the threat is and who is affected.
    pub threat_assessment: String,
    /// Second generated paragraph: the evidence supporting the assessment.
    pub severity_justification: String,
    /// Third generated paragraph: business/operational impact.
    pub impact_assessment: String,
    /// Fourth generated paragraph: mitigation/escalation actions.
    pub response_plan: String,
    /// Any extra paragraphs the model produced, rendered as limitations.
    pub key_assumptions: Vec<String>,
    pub source_count: usize,
    pub observation_count: usize,
    pub related_insight_count: usize,
    pub entity_count: usize,
    /// Warning confidence from the producing pipeline (0.0–1.0).
    pub overall_confidence: f64,
    /// `strong` | `adequate` | `limited` — derived from real source volume.
    pub data_sufficiency: String,
    /// `high` | `medium` | `low` — derived from real source volume.
    pub source_reliability: String,
    /// Names of the entities the analysis was grounded in (may be empty).
    pub entity_names: Vec<String>,
}

impl WarningAnalysisOutput {
    /// Key indicators derived from the generated evidence paragraph. These are
    /// the analysis claims: each is grounded in the observation corpus the
    /// model saw, with the real observation/source counts as its evidence.
    pub fn key_indicators(&self) -> Vec<AnalysisClaim> {
        let indicator: String = self.severity_justification.chars().take(240).collect();
        if indicator.trim().is_empty() {
            return Vec::new();
        }
        vec![AnalysisClaim {
            indicator,
            evidence: format!(
                "{} observations, {} sources",
                self.observation_count, self.source_count
            ),
            severity_contribution: String::new(),
        }]
    }

    /// Limitations the analysis is explicit about: model paragraphs beyond the
    /// four requested (assumptions) plus a data-sufficiency statement.
    pub fn limitations(&self) -> Vec<String> {
        let mut limitations = self.key_assumptions.clone();
        limitations.push(format!(
            "Data sufficiency: {} ({} sources, {} observations, {} related insights)",
            self.data_sufficiency,
            self.source_count,
            self.observation_count,
            self.related_insight_count
        ));
        if self.entity_count == 0 {
            limitations.push(
                "No linked entities were available; entity-specific impact is not assessed."
                    .to_string(),
            );
        }
        limitations
    }

    /// Recommended actions from the generated response paragraph.
    pub fn recommendations(&self) -> Vec<String> {
        vec![self.response_plan.clone()]
            .into_iter()
            .filter(|action| !action.trim().is_empty())
            .collect()
    }
}

/// One claim in the rendered analysis panel.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnalysisClaim {
    pub indicator: String,
    pub evidence: String,
    pub severity_contribution: String,
}

/// Run the real warning analysis: gather evidence, call the LLM, and return
/// the structured result.
///
/// Any storage or LLM failure is returned as `Err`; callers must render an
/// explicit failure/degraded state, never a placeholder that implies analysis
/// is under way.
pub async fn analyze_warning(
    store: &PgStore,
    model_config: &ModelConfig,
    warning: &WarningRow,
) -> Result<WarningAnalysisOutput> {
    let entity_ids: Vec<Uuid> = warning.entity_ids.clone().unwrap_or_default();

    let company_names = store
        .get_company_names_by_ids(&entity_ids)
        .await
        .context("warning analysis: failed to load entity names")?;
    let entity_names: Vec<String> = company_names
        .iter()
        .map(|(_, name, _, _)| name.clone())
        .collect();

    let mut all_observations = Vec::new();
    for entity_id in &entity_ids {
        let observations = store
            .get_observations_by_entity(*entity_id, 30)
            .await
            .with_context(|| {
                format!("warning analysis: failed to load observations for entity {entity_id}")
            })?;
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

    let related_insights = store
        .get_insights_by_entity_ids(&entity_ids, 10)
        .await
        .context("warning analysis: failed to load related insights")?;
    let source_urls = warning.source_urls.clone().unwrap_or_default();
    let source_count = source_urls.len();

    let mut context_parts: Vec<String> = Vec::new();
    if !all_observations.is_empty() {
        context_parts.push(format!("DATA ({} observations):", all_observations.len()));
        for (i, observation) in all_observations.iter().take(6).enumerate() {
            let text = observation
                .value
                .get("excerpt")
                .or(observation.value.get("text"))
                .or(observation.value.get("summary"))
                .or(observation.value.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let truncated: String = text.chars().take(100).collect();
            context_parts.push(format!(
                "[O{}] {} — {}",
                i + 1,
                observation.observation_type,
                truncated
            ));
        }
    }

    for (i, related_insight) in related_insights.iter().take(3).enumerate() {
        context_parts.push(format!("[I{}] {}", i + 1, related_insight.title));
    }

    let context_block = context_parts.join("\n");
    let entity_names_str = if entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        entity_names.join(", ")
    };
    let region = warning.region.as_deref().unwrap_or("Global");
    let system_prompt = concat!(
        "You are a senior threat analyst specializing in electronics, defense, and supply chains. ",
        "Write a brief threat assessment. Be specific — name companies, products, events, and dates. ",
        "Cite data references like [O1], [I1] where relevant."
    );

    let desc_truncated: String = warning
        .description
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect();
    let user_prompt = format!(
        r#"Write a 4-paragraph threat assessment.

WARNING: {title} ({severity} severity)
Type: {warning_type} | Region: {region} | Confidence: {confidence:.0}%
Entities: {entities}

DESCRIPTION: {description}

{context}

Write EXACTLY 4 paragraphs, each on a new line:
1. THREAT: What the threat is and who is affected (3-4 sentences)
2. EVIDENCE: What data supports this assessment, citing observations (3-4 sentences)
3. IMPACT: Business, operational, and financial consequences with timeline (2-3 sentences)
4. RESPONSE: Specific mitigation actions and escalation triggers (2-3 sentences)"#,
        title = warning.title,
        warning_type = warning.warning_type,
        severity = warning.severity,
        region = region,
        confidence = warning.confidence.unwrap_or(0.0) * 100.0,
        description = desc_truncated,
        entities = entity_names_str,
        context = context_block,
    );

    let mut model_config = model_config.clone();
    model_config.temperature = 0.5;
    model_config.max_tokens = 1024;
    model_config.timeout_seconds = 300;
    let client = OpenAiCompatibleClient::new(model_config);

    let raw_analysis = client
        .generate_text(system_prompt, &user_prompt)
        .await
        .context("warning analysis: LLM call failed")?;

    let paragraphs: Vec<&str> = raw_analysis
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let threat_text = strip_warning_label(paragraphs.first().copied().unwrap_or(""));
    let evidence_text = strip_warning_label(paragraphs.get(1).copied().unwrap_or(""));
    let impact_text = strip_warning_label(paragraphs.get(2).copied().unwrap_or(""));
    let response_text = strip_warning_label(paragraphs.get(3).copied().unwrap_or(""));

    // An empty generation is a failure, not a completed analysis: callers must
    // render an explicit failed state instead of a panel with empty sections.
    if threat_text.trim().is_empty()
        && evidence_text.trim().is_empty()
        && impact_text.trim().is_empty()
        && response_text.trim().is_empty()
    {
        anyhow::bail!("warning analysis: LLM returned an empty analysis");
    }

    let source_reliability = if source_count >= 5 {
        "high"
    } else if source_count >= 2 {
        "medium"
    } else {
        "low"
    };
    let data_sufficiency = if source_count >= 4 && all_observations.len() >= 3 {
        "strong"
    } else if source_count >= 2 {
        "adequate"
    } else {
        "limited"
    };

    Ok(WarningAnalysisOutput {
        threat_assessment: threat_text,
        severity_justification: evidence_text,
        impact_assessment: impact_text,
        response_plan: response_text,
        key_assumptions: paragraphs
            .get(4..)
            .map(|extra| extra.iter().map(|line| strip_warning_label(line)).collect())
            .unwrap_or_default(),
        source_count,
        observation_count: all_observations.len(),
        related_insight_count: related_insights.len(),
        entity_count: entity_ids.len(),
        overall_confidence: warning.confidence.unwrap_or(0.0),
        data_sufficiency: data_sufficiency.to_string(),
        source_reliability: source_reliability.to_string(),
        entity_names,
    })
}

/// Strip the model's paragraph labels (`THREAT:`, `1.`, …) from generated text.
pub fn strip_warning_label(s: &str) -> String {
    let patterns = [
        "THREAT:",
        "EVIDENCE:",
        "IMPACT:",
        "RESPONSE:",
        "1.",
        "2.",
        "3.",
        "4.",
    ];
    let mut result = s.to_string();
    for pattern in patterns {
        if let Some(stripped) = result.strip_prefix(pattern) {
            result = stripped.trim().to_string();
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_warning_label_removes_one_section_prefix() {
        assert_eq!(
            strip_warning_label("THREAT: A real threat"),
            "A real threat"
        );
        assert_eq!(strip_warning_label("2. Evidence line"), "Evidence line");
        assert_eq!(strip_warning_label("unchanged"), "unchanged");
    }

    fn sample_output() -> WarningAnalysisOutput {
        WarningAnalysisOutput {
            threat_assessment: "Threat".to_string(),
            severity_justification: "Evidence".to_string(),
            impact_assessment: "Impact".to_string(),
            response_plan: "Respond".to_string(),
            key_assumptions: vec!["Assumption".to_string()],
            source_count: 3,
            observation_count: 5,
            related_insight_count: 1,
            entity_count: 0,
            overall_confidence: 0.7,
            data_sufficiency: "adequate".to_string(),
            source_reliability: "medium".to_string(),
            entity_names: Vec::new(),
        }
    }

    #[test]
    fn limitations_include_sufficiency_and_missing_entity_warning() {
        let output = sample_output();
        let limitations = output.limitations();
        assert!(limitations.iter().any(|l| l.contains("Assumption")));
        assert!(limitations
            .iter()
            .any(|l| l.contains("Data sufficiency: adequate")));
        assert!(
            limitations.iter().any(|l| l.contains("No linked entities")),
            "zero entity coverage must be stated as a limitation"
        );
    }

    #[test]
    fn recommendations_are_empty_when_the_model_returned_no_response_paragraph() {
        let mut output = sample_output();
        output.response_plan = "   ".to_string();
        assert!(output.recommendations().is_empty());
        output.response_plan = "Do the thing".to_string();
        assert_eq!(output.recommendations(), vec!["Do the thing".to_string()]);
    }
}
