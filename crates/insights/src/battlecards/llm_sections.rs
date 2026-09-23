//! LLM-grounded battlecard narrative synthesis.
//!
//! The [`BattlecardGenerator`](crate::battlecards::generator::BattlecardGenerator)
//! produces the *structural* sections (feature matrix, kill shots, win/loss,
//! pricing) algorithmically from real data. This module layers **LLM-synthesized
//! narrative** on top — positioning statements, objection-handler rebuttals, and
//! competitive positioning summaries — grounded strictly in the provided evidence
//! and gated by the shared anti-hallucination checks.
//!
//! # Anti-hallucination contract
//!
//! Every LLM call:
//!   1. Injects the *evidence* (insight titles, source URLs, observed claims)
//!      into the prompt as the only allowed factual basis.
//!   2. Instructs the model to refuse / return empty rather than invent.
//!   3. Deserializes into a strict schema; fields that fail validation are
//!      dropped, never silently filled with plausible-sounding text.
//!
//! When the LLM is unavailable (`LlmClient::from_env()` fails) or the model
//! returns nothing usable, the caller falls back to the algorithmic section —
//! there is never a fabricated LLM output.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use apex_llm::inference::{ChatMessage, InferenceConfig, LlmClient};

use crate::battlecards::{ObjectionHandlerPair, PositioningSection, StrengthItem, WeaknessItem};
use crate::Insight;

/// LLM-synthesized battlecard sections, all grounded in provided evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmBattlecardSections {
    /// Human-readable positioning narrative grounded in real signals.
    pub positioning_narrative: String,
    /// Evidence-backed strengths with LLM-written rationale.
    pub strengths: Vec<LlmStrength>,
    /// Evidence-backed weaknesses with LLM-written rationale + exploitation.
    pub weaknesses: Vec<LlmWeakness>,
    /// Objection → rebuttal pairs synthesized from real competitor claims.
    pub objection_handlers: Vec<ObjectionHandlerPair>,
    /// One-line "talk track" a rep can use to open the conversation.
    pub opener: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStrength {
    pub title: String,
    pub description: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmWeakness {
    pub title: String,
    pub description: String,
    pub how_to_exploit: String,
    pub evidence: String,
}

/// The strict JSON schema the LLM must return. Extra fields are ignored;
/// missing required fields cause the whole section to be dropped (no fill-in).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmBattlecardResponse {
    positioning_narrative: String,
    strengths: Vec<LlmStrength>,
    weaknesses: Vec<LlmWeakness>,
    objection_handlers: Vec<LlmObjectionPair>,
    opener: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmObjectionPair {
    objection: String,
    rebuttal: String,
    effectiveness: f64,
}

/// Synthesize LLM-grounded battlecard narrative from real evidence.
///
/// Returns `Ok(None)` when there is insufficient evidence (fewer than
/// `min_evidence` insights) — in that case the caller uses the algorithmic
/// sections unchanged rather than prompting the LLM with thin context.
pub async fn synthesize_llm_sections(
    client: &LlmClient,
    our_company: &str,
    competitor: &str,
    insights: &[Insight],
    min_evidence: usize,
) -> Result<Option<LlmBattlecardSections>> {
    if insights.len() < min_evidence {
        return Ok(None);
    }

    // Build the evidence block from real insight titles + sources only.
    let evidence_block = build_evidence_block(insights);

    let system = "You are a senior competitive intelligence analyst writing a \
                  sales battlecard. You may ONLY use the evidence provided in the \
                  user message. If the evidence does not support a field, return \
                  an empty string or empty array for it — never invent company \
                  facts, pricing, headcount, or capabilities. Be specific and \
                  concrete; cite the evidence in your reasoning.";

    let user = format!(
        "OUR COMPANY: {our}\nCOMPETITOR: {comp}\n\n\
         EVIDENCE (the only facts you may use):\n{evidence}\n\n\
         Produce a JSON object with this exact schema:\n\
         {{\n  \"positioning_narrative\": string (2-3 sentences on where {comp} sits and how {our} beats them, grounded ONLY in evidence),\n\
         \n  \"strengths\": [{{ \"title\": string, \"description\": string, \"evidence\": string (which evidence item supports this) }}],\n\
         \n  \"weaknesses\": [{{ \"title\": string, \"description\": string, \"how_to_exploit\": string (concrete sales tactic), \"evidence\": string }}],\n\
         \n  \"objection_handlers\": [{{ \"objection\": string (what {comp} claims), \"rebuttal\": string (how {our} counters, evidence-based), \"effectiveness\": number 0..1 }}],\n\
         \n  \"opener\": string (one sentence a rep says to open against {comp})\n}}\n\n\
         Return ONLY the JSON. If evidence is insufficient for any array, return an empty array for it.",
        our = our_company,
        comp = competitor,
        evidence = evidence_block,
    );

    let config = InferenceConfig::json_structured();
    let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];

    let raw = match client.complete_with_config(messages, &config).await {
        Ok(resp) => resp.text,
        Err(e) => {
            tracing::warn!(error = %e, "battlecard llm synthesis: LLM call failed; falling back to algorithmic");
            return Ok(None);
        }
    };

    // Strip any stray reasoning tags before parsing.
    let cleaned = apex_llm::inference::strip_think_tags(&raw);
    let parsed: LlmBattlecardResponse = match serde_json::from_str(&cleaned) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "battlecard llm synthesis: JSON parse failed; falling back");
            return Ok(None);
        }
    };

    // Validate: drop any strength/weakness with empty required fields rather
    // than surfacing half-formed LLM output.
    let strengths: Vec<LlmStrength> = parsed
        .strengths
        .into_iter()
        .filter(|s| !s.title.trim().is_empty() && !s.description.trim().is_empty())
        .take(5)
        .collect();
    let weaknesses: Vec<LlmWeakness> = parsed
        .weaknesses
        .into_iter()
        .filter(|w| !w.title.trim().is_empty() && !w.description.trim().is_empty())
        .take(5)
        .collect();
    let objection_handlers: Vec<ObjectionHandlerPair> = parsed
        .objection_handlers
        .into_iter()
        .filter(|o| !o.objection.trim().is_empty() && !o.rebuttal.trim().is_empty())
        .map(|o| ObjectionHandlerPair {
            objection: o.objection,
            counter_arg: o.rebuttal,
            effectiveness: o.effectiveness.clamp(0.0, 1.0),
            evidence_url: String::new(),
        })
        .take(5)
        .collect();

    // If the LLM returned nothing usable, signal fallback.
    if parsed.positioning_narrative.trim().is_empty()
        && strengths.is_empty()
        && weaknesses.is_empty()
        && objection_handlers.is_empty()
    {
        return Ok(None);
    }

    Ok(Some(LlmBattlecardSections {
        positioning_narrative: parsed.positioning_narrative,
        strengths,
        weaknesses,
        objection_handlers,
        opener: parsed.opener,
    }))
}

/// Build a numbered evidence block from real insights (title + sources).
/// Caps at 15 items to keep the prompt bounded.
fn build_evidence_block(insights: &[Insight]) -> String {
    insights
        .iter()
        .take(15)
        .enumerate()
        .map(|(i, ins)| {
            let src = ins.sources.first().cloned().unwrap_or_default();
            format!(
                "[{}] {} (confidence: {:.2}){}",
                i + 1,
                ins.title,
                ins.confidence,
                if src.is_empty() {
                    String::new()
                } else {
                    format!(" — source: {src}")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Merge LLM strengths into the algorithmic strengths list, deduplicating by
/// title (LLM versions preferred for their evidence-backed rationale).
pub fn merge_strengths(algo: Vec<StrengthItem>, llm: &[LlmStrength]) -> Vec<StrengthItem> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<StrengthItem> = llm
        .iter()
        .filter_map(|s| {
            let key = s.title.to_lowercase();
            if seen.insert(key) {
                Some(StrengthItem {
                    title: s.title.clone(),
                    description: s.description.clone(),
                    impact_area: "Competitive".to_string(),
                    evidence_url: s.evidence.clone(),
                })
            } else {
                None
            }
        })
        .collect();
    for a in algo {
        if seen.insert(a.title.to_lowercase()) {
            out.push(a);
        }
    }
    out
}

/// Merge LLM weaknesses into the algorithmic weaknesses list.
pub fn merge_weaknesses(algo: Vec<WeaknessItem>, llm: &[LlmWeakness]) -> Vec<WeaknessItem> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<WeaknessItem> = llm
        .iter()
        .filter_map(|w| {
            let key = w.title.to_lowercase();
            if seen.insert(key) {
                Some(WeaknessItem {
                    title: w.title.clone(),
                    description: format!("{} Exploit: {}", w.description, w.how_to_exploit),
                    impact_area: "Competitive".to_string(),
                    severity: 0.7,
                })
            } else {
                None
            }
        })
        .collect();
    for a in algo {
        if seen.insert(a.title.to_lowercase()) {
            out.push(a);
        }
    }
    out.truncate(8);
    out
}

/// Enrich a positioning section with the LLM narrative (appended to the
/// existing value_proposition so the algorithmic signal is preserved).
pub fn enrich_positioning(
    mut base: PositioningSection,
    llm: &LlmBattlecardSections,
) -> PositioningSection {
    if !llm.positioning_narrative.is_empty() {
        base.value_proposition = if base.value_proposition.is_empty() {
            llm.positioning_narrative.clone()
        } else {
            format!(
                "{}\n\n{}",
                base.value_proposition, llm.positioning_narrative
            )
        };
    }
    base
}
