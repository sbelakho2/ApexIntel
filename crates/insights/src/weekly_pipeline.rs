//! End-to-end weekly intelligence memo pipeline.
//!
//! Orchestrates the full journey from raw [`InsightCard`]s to a polished,
//! LLM-narrated [`WeeklyMemo`] ready for email delivery to the GM/Board.
//!
//! # Pipeline stages
//! 1. **Collect** — receive pre-computed insight cards
//! 2. **Rank** — sort by signal strength and recency
//! 3. **Cluster** — group into regional / thematic sections
//! 4. **Narrate** — call LLM to enrich executive summary and section leads
//! 5. **Render** — convert to structured [`WeeklyMemo`]
//!
//! # Fallback
//! If the LLM is unavailable, the pipeline produces a rule-based memo
//! without narrative enrichment (degrades gracefully).

use crate::memo::{generate_weekly_memo, WeeklyMemo};
use crate::renderer::{card_information_gain_bits, group_by_region, rank_insights, InsightCard};
use anyhow::{Context, Result};
use apex_llm::LlmClient;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the weekly pipeline runner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyPipelineConfig {
    /// Maximum tier-1 priority cards to feature in the executive summary.
    pub max_executive_cards: usize,
    /// Maximum total cards to include in the memo body.
    pub max_body_cards: usize,
    /// Whether to call the LLM for narrative enrichment.
    pub enable_llm_narration: bool,
    /// Minimum confidence threshold (0–1) for including a card.
    pub min_confidence: f64,
    /// Minimum priority_score to include in the executive spotlight.
    pub min_exec_priority: f64,
}

impl Default for WeeklyPipelineConfig {
    fn default() -> Self {
        Self {
            max_executive_cards: 5,
            max_body_cards: 30,
            enable_llm_narration: true,
            min_confidence: 0.40,
            min_exec_priority: 0.70,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline output
// ─────────────────────────────────────────────────────────────────────────────

/// Output of one weekly pipeline run.
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// The fully constructed memo.
    pub memo: WeeklyMemo,
    /// Number of cards received before filtering.
    pub cards_received: usize,
    /// Number of cards discarded (below confidence threshold).
    pub cards_discarded: usize,
    /// Whether LLM narration was applied.
    pub llm_narrated: bool,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Weekly information-gain summaries per entity, including crawl priority recommendations.
    pub entity_information_gain: Vec<EntityInformationGainSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityInformationGainSummary {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub cumulative_information_gain_bits: f64,
    pub mean_information_gain_bits: f64,
    pub insight_count: usize,
    pub plateaued: bool,
    pub crawl_priority_multiplier: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline runner
// ─────────────────────────────────────────────────────────────────────────────

/// Orchestrates the weekly intelligence memo generation pipeline.
pub struct WeeklyPipelineRunner {
    config: WeeklyPipelineConfig,
    llm: Option<Arc<dyn LlmClient>>,
}

impl WeeklyPipelineRunner {
    /// Create a runner with an LLM client.
    pub fn new(config: WeeklyPipelineConfig, llm: Arc<dyn LlmClient>) -> Self {
        Self {
            config,
            llm: Some(llm),
        }
    }

    /// Create a runner without LLM (rule-based narration only).
    pub fn headless(config: WeeklyPipelineConfig) -> Self {
        Self { config, llm: None }
    }

    /// Execute the full pipeline.
    pub async fn run(&self, cards: Vec<InsightCard>) -> Result<PipelineOutput> {
        let start = std::time::Instant::now();
        let total = cards.len();

        // Stage 1: Filter by confidence
        let (qualified, discarded): (Vec<_>, Vec<_>) = cards
            .into_iter()
            .partition(|c| c.confidence >= self.config.min_confidence);

        let cards_discarded = discarded.len();

        // Stage 2: Sort by score descending (highest priority first)
        let mut ranked = qualified;
        rank_insights(&mut ranked);
        ranked.truncate(self.config.max_body_cards);

        // Stage 3: Extract top executive-level cards
        let exec_cards: Vec<&InsightCard> = ranked
            .iter()
            .take(self.config.max_executive_cards)
            .collect();

        // Stage 4: Generate executive summary
        let exec_summary = if self.config.enable_llm_narration {
            if let Some(ref llm) = self.llm {
                match self.llm_executive_summary(llm.as_ref(), &exec_cards).await {
                    Ok(summary) => summary,
                    Err(e) => {
                        warn!(error=%e, "LLM executive summary failed; using rule-based fallback");
                        self.rule_based_exec_summary(&exec_cards)
                    }
                }
            } else {
                self.rule_based_exec_summary(&exec_cards)
            }
        } else {
            self.rule_based_exec_summary(&exec_cards)
        };

        // Stage 5: LLM-enrich regional section leads
        let mut enriched_cards = ranked.clone();
        let llm_narrated = if self.config.enable_llm_narration {
            if let Some(ref llm) = self.llm {
                match self
                    .enrich_section_leads(llm.as_ref(), &mut enriched_cards)
                    .await
                {
                    Ok(count) => {
                        info!(enriched_sections = count, "LLM section leads generated");
                        true
                    }
                    Err(e) => {
                        warn!(error=%e, "LLM section enrichment failed");
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        };

        // Stage 6: Build WeeklyMemo
        let mut memo = generate_weekly_memo(&enriched_cards);
        memo.executive_summary = exec_summary;

        let duration_ms = start.elapsed().as_millis() as u64;
        let entity_information_gain = summarize_entity_information_gain(&enriched_cards);
        let plateaued_entities = entity_information_gain
            .iter()
            .filter(|summary| summary.plateaued)
            .count();

        info!(
            cards_processed = total,
            cards_discarded,
            llm_narrated,
            duration_ms,
            plateaued_entities,
            "Weekly pipeline complete"
        );

        Ok(PipelineOutput {
            memo,
            cards_received: total,
            cards_discarded,
            llm_narrated,
            duration_ms,
            entity_information_gain,
        })
    }

    // ── LLM stages ───────────────────────────────────────────────

    async fn llm_executive_summary(
        &self,
        llm: &dyn LlmClient,
        cards: &[&InsightCard],
    ) -> Result<String> {
        let card_text = cards
            .iter()
            .map(|c| {
                format!(
                    "- [{}] {}: {} (confidence: {:.0}%)",
                    c.region.as_deref().unwrap_or("GLB"),
                    c.entity_name,
                    c.title,
                    c.confidence * 100.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let now = Utc::now();
        let system = "You are a senior intelligence analyst writing a weekly executive briefing. \
            Write in a concise, direct, authoritative style. No fluff. Pure signal.";

        let user = format!(
            "Write a 3-sentence executive summary for the week of {}. \
            These are the top intelligence signals:\n{}\n\
            Cover: key risks, key opportunities, recommended focus.",
            now.format("%B %d, %Y"),
            card_text
        );

        llm.generate_text(system, &user)
            .await
            .context("LLM executive summary generation failed")
    }

    async fn enrich_section_leads(
        &self,
        llm: &dyn LlmClient,
        cards: &mut Vec<InsightCard>,
    ) -> Result<usize> {
        let by_region = group_by_region(cards.as_slice());
        let region_inputs: Vec<(String, String)> = by_region
            .iter()
            .filter_map(|(region, region_cards)| {
                if region_cards.is_empty() {
                    return None;
                }
                let headlines = region_cards
                    .iter()
                    .take(5)
                    .map(|c| format!("• {}: {}", c.entity_name, c.title))
                    .collect::<Vec<_>>()
                    .join("\n");
                Some((region.clone(), headlines))
            })
            .collect();
        let mut enriched_count = 0;

        for (region, headlines) in region_inputs {
            let system = "You are an intelligence analyst. Write a single concise paragraph \
                (2-3 sentences) summarizing the regional intelligence picture.";

            let user = format!(
                "Summarise the {} intelligence picture based on these signals:\n{}",
                region, headlines
            );

            match llm.generate_text(system, &user).await {
                Ok(lead) => {
                    if let Some(first_card) = cards
                        .iter_mut()
                        .find(|card| card.region.as_deref().unwrap_or("global") == region)
                    {
                        let lead = lead.trim();
                        if !lead.is_empty() && !first_card.narrative.starts_with(lead) {
                            first_card.narrative = format!("{}\n\n{}", lead, first_card.narrative);
                        }
                    }
                    enriched_count += 1;
                }
                Err(e) => warn!(region=%region, error=%e, "Region lead generation failed"),
            }
        }

        Ok(enriched_count)
    }

    // ── Rule-based fallback ───────────────────────────────────────

    fn rule_based_exec_summary(&self, cards: &[&InsightCard]) -> String {
        let now = Utc::now();
        let critical: Vec<_> = cards.iter().filter(|c| c.priority_score >= 0.8).collect();
        let high: Vec<_> = cards
            .iter()
            .filter(|c| c.priority_score >= 0.6 && c.priority_score < 0.8)
            .collect();

        let mut summary = format!(
            "Weekly Intelligence Briefing — {}. ",
            now.format("%B %d, %Y")
        );

        if !critical.is_empty() {
            let names: Vec<&str> = critical.iter().map(|c| c.entity_name.as_str()).collect();
            summary += &format!(
                "{} critical signal(s) detected for: {}. ",
                critical.len(),
                names.join(", ")
            );
        }

        if !high.is_empty() {
            summary += &format!(
                "{} high-priority signal(s) require attention this week. ",
                high.len()
            );
        }

        if cards.is_empty() {
            summary += "No significant signals detected this week.";
        } else {
            summary += "See regional sections for full detail.";
        }

        summary
    }
}

pub fn summarize_entity_information_gain(
    cards: &[InsightCard],
) -> Vec<EntityInformationGainSummary> {
    let mut grouped: std::collections::HashMap<Uuid, (String, f64, usize)> =
        std::collections::HashMap::new();
    for card in cards {
        let entry = grouped
            .entry(card.entity_id)
            .or_insert_with(|| (card.entity_name.clone(), 0.0, 0));
        entry.1 += card_information_gain_bits(card);
        entry.2 += 1;
    }

    let mut summaries: Vec<EntityInformationGainSummary> = grouped
        .into_iter()
        .map(
            |(entity_id, (entity_name, cumulative_information_gain_bits, insight_count))| {
                let mean_information_gain_bits = if insight_count == 0 {
                    0.0
                } else {
                    cumulative_information_gain_bits / insight_count as f64
                };
                let plateaued = insight_count >= 2 && mean_information_gain_bits < 0.1;
                EntityInformationGainSummary {
                    entity_id,
                    entity_name,
                    cumulative_information_gain_bits,
                    mean_information_gain_bits,
                    insight_count,
                    plateaued,
                    crawl_priority_multiplier: if plateaued { 0.75 } else { 1.0 },
                }
            },
        )
        .collect();

    summaries.sort_by(|a, b| {
        a.crawl_priority_multiplier
            .partial_cmp(&b.crawl_priority_multiplier)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entity_name.cmp(&b.entity_name))
    });
    summaries
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use crate::renderer::InsightCard;

    fn make_card(entity: &str, title: &str, priority_score: f64) -> InsightCard {
        InsightCard {
            id: uuid::Uuid::new_v4(),
            recipe_code: "TEST_001".to_string(),
            entity_id: uuid::Uuid::new_v4(),
            entity_name: entity.to_string(),
            severity: "Medium".to_string(),
            category: "Risk".to_string(),
            title: title.to_string(),
            narrative: "Detail text.".to_string(),
            actions: vec![],
            citations: vec![],
            confidence: 0.75,
            impact: priority_score,
            impact_label: "Medium".to_string(),
            priority_score,
            region: Some("IL".to_string()),
            rendered_at: Utc::now(),
        }
    }

    #[test]
    fn config_defaults_sane() {
        let cfg = WeeklyPipelineConfig::default();
        assert!(cfg.max_body_cards > cfg.max_executive_cards);
        assert!(cfg.min_confidence > 0.0 && cfg.min_confidence < 1.0);
    }

    #[tokio::test]
    async fn headless_pipeline_runs() {
        let cfg = WeeklyPipelineConfig::default();
        let runner = WeeklyPipelineRunner::headless(cfg);
        let cards: Vec<InsightCard> = (0..10)
            .map(|i| make_card("Acme", &format!("Signal {}", i), 0.5 + i as f64 * 0.04))
            .collect();
        let result = runner.run(cards).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.memo.executive_summary.is_empty());
        assert!(!output.llm_narrated);
        assert_eq!(output.cards_received, 10);
        assert!(!output.entity_information_gain.is_empty());
    }

    #[tokio::test]
    async fn cards_below_confidence_discarded() {
        let mut cfg = WeeklyPipelineConfig::default();
        cfg.min_confidence = 0.8;
        let runner = WeeklyPipelineRunner::headless(cfg);
        // Both cards have confidence=0.75 (from make_card). With min_confidence=0.8, both are discarded.
        let cards = vec![
            make_card("A", "Card below threshold", 0.9),
            make_card("B", "Also below threshold", 0.3),
        ];
        let output = runner.run(cards).await.unwrap();
        assert_eq!(output.cards_discarded, 2);
    }

    #[test]
    fn entity_information_gain_plateau_recommends_lower_crawl_priority() {
        let entity_id = uuid::Uuid::new_v4();
        let cards = vec![
            InsightCard {
                id: uuid::Uuid::new_v4(),
                recipe_code: "LOW-A".to_string(),
                entity_id,
                entity_name: "Plateau Co".to_string(),
                severity: "info".to_string(),
                category: "ops".to_string(),
                title: "Low IG A".to_string(),
                narrative: String::new(),
                actions: vec![],
                citations: vec![],
                confidence: 0.05,
                impact: 0.2,
                impact_label: "Info".to_string(),
                priority_score: 0.1,
                region: None,
                rendered_at: Utc::now(),
            },
            InsightCard {
                id: uuid::Uuid::new_v4(),
                recipe_code: "LOW-B".to_string(),
                entity_id,
                entity_name: "Plateau Co".to_string(),
                severity: "info".to_string(),
                category: "ops".to_string(),
                title: "Low IG B".to_string(),
                narrative: String::new(),
                actions: vec![],
                citations: vec![],
                confidence: 0.08,
                impact: 0.2,
                impact_label: "Info".to_string(),
                priority_score: 0.1,
                region: None,
                rendered_at: Utc::now(),
            },
        ];

        let summary = summarize_entity_information_gain(&cards);
        assert_eq!(summary.len(), 1);
        assert!(summary[0].plateaued);
        assert!(summary[0].crawl_priority_multiplier < 1.0);
    }
}
