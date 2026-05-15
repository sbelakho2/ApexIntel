//! LLM-powered enrichment for intelligence artifacts.
//!
//! Wraps the `apex-llm` crate's [`InsightGenerator`] and exposes high-level
//! async helpers that take existing [`WeeklyMemo`] / dossier data and layer in
//! LLM-generated narratives, executive summaries, and geopolitical assessments.
//!
//! # Usage pattern
//! ```no_run
//! use apex_insights::llm_enricher::InsightsLlmEnricher;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let enricher = InsightsLlmEnricher::from_env()?;
//! let signals = vec![
//!     ("Procurement slowdown".to_string(), "Q2 2025 orders fell 18%".to_string(), "https://example.com".to_string()),
//! ];
//! let entities = vec!["Elbit Systems".to_string()];
//! let enriched_summary = enricher
//!     .enrich_executive_summary("supply_chain_disruption", &signals, &entities, "MENA")
//!     .await?;
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing;

use apex_llm::{
    inference::LlmClient,
    insight_gen::{
        CompetitiveIntelSummary, GeopoliticalAssessment, InsightGenerator, LlmInsightNarrative,
        SupplyChainRiskNarrative,
    },
};

// ────────────────────────────────────────────
// Enriched output types
// ────────────────────────────────────────────

/// LLM-enriched executive summary section for a weekly memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedExecSummary {
    /// Original template-rendered summary.
    pub original: String,
    /// LLM-refined / extended narrative.
    pub llm_narrative: String,
    /// Key risks extracted by the LLM (bullet points).
    pub key_risks: Vec<String>,
    /// Recommended immediate actions.
    pub recommended_actions: Vec<String>,
    /// Confidence of the LLM output (0-1).
    pub confidence: f64,
}

/// LLM-enriched regional intelligence section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedRegionalSection {
    pub region: String,
    pub llm_narrative: String,
    pub geopolitical_assessment: Option<GeopoliticalAssessment>,
}

/// LLM-generated competitive intelligence card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitiveIntelCard {
    pub target_company: String,
    pub summary: CompetitiveIntelSummary,
}

/// LLM-generated supply chain risk card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainRiskCard {
    pub entity_id: String,
    pub narrative: SupplyChainRiskNarrative,
}

// ────────────────────────────────────────────
// Main enricher struct
// ────────────────────────────────────────────

/// High-level LLM enricher for insights artifacts.
///
/// Thin wrapper around [`InsightGenerator`] that adds error-handling,
/// caching keys, and insights-domain-specific prompts.
pub struct InsightsLlmEnricher {
    gen: InsightGenerator,
}

impl InsightsLlmEnricher {
    /// Build from environment variables.
    ///
    /// Reads `LLM_BASE_URL` (defaults to `http://localhost:8080`),
    /// `LLM_API_KEY` (optional), `LLM_MODEL` (optional).
    pub fn from_env() -> Result<Self> {
        let client = LlmClient::from_env()?;
        Ok(Self {
            gen: InsightGenerator::new(client),
        })
    }

    /// Build with a pre-constructed client.
    pub fn new(client: LlmClient) -> Self {
        Self {
            gen: InsightGenerator::new(client),
        }
    }

    // ── Executive summary ──────────────────────────────────────────

    /// Enrich a weekly memo's executive summary using LLM insight narrative.
    ///
    /// * `signal_type` — thematic category, e.g. `"supply_chain_disruption"`.
    /// * `signals` — `(title, description, source_url)` triples.
    /// * `entity_names` — company/POI names relevant to this insight.
    /// * `region` — geographic region code, e.g. `"MENA"`.
    pub async fn enrich_executive_summary(
        &self,
        signal_type: &str,
        signals: &[(String, String, String)],
        entity_names: &[String],
        region: &str,
    ) -> Result<EnrichedExecSummary> {
        let narrative: LlmInsightNarrative = self
            .gen
            .generate_insight_narrative(signal_type, signals, entity_names, region)
            .await?;

        Ok(EnrichedExecSummary {
            original: signal_type.to_string(),
            llm_narrative: narrative.executive_summary.clone(),
            key_risks: vec![narrative.detailed_analysis.clone()],
            recommended_actions: vec![narrative.recommendation.clone()],
            confidence: narrative.confidence,
        })
    }

    // ── Geopolitical assessment ────────────────────────────────────

    /// Generate a geopolitical risk assessment for an event.
    ///
    /// * `event_description` — plain-text description of the trigger event.
    /// * `affected_countries` — ISO country codes or full names.
    /// * `industry_context` — relevant industry context note.
    pub async fn assess_geopolitical_risk(
        &self,
        event_description: &str,
        affected_countries: &[String],
        industry_context: &str,
    ) -> Result<GeopoliticalAssessment> {
        self.gen
            .assess_geopolitical_risk(event_description, affected_countries, industry_context)
            .await
    }

    // ── Competitive intelligence ───────────────────────────────────

    /// Summarize competitive intelligence for a target company.
    ///
    /// * `company_name` — e.g. `"Airbus Defence & Space"`.
    /// * `company_region` — region where the company operates.
    /// * `observed_signals` — `(signal_type, description)` pairs.
    pub async fn analyze_competitive_intel(
        &self,
        company_name: &str,
        company_region: &str,
        observed_signals: &[(String, String)],
    ) -> Result<CompetitiveIntelCard> {
        let summary = self
            .gen
            .analyze_competitive_intel(company_name, company_region, observed_signals)
            .await?;
        Ok(CompetitiveIntelCard {
            target_company: company_name.to_string(),
            summary,
        })
    }

    // ── Supply chain ───────────────────────────────────────────────

    /// Narrate supply chain risk.
    ///
    /// * `affected_companies` — company names involved.
    /// * `affected_regions` — region codes.
    /// * `signal_descriptions` — textual descriptions of the risk signals.
    pub async fn narrate_supply_chain_risk(
        &self,
        affected_companies: &[String],
        affected_regions: &[String],
        signal_descriptions: &[String],
    ) -> Result<SupplyChainRiskCard> {
        let narrative = self
            .gen
            .narrate_supply_chain_risk(affected_companies, affected_regions, signal_descriptions)
            .await?;
        Ok(SupplyChainRiskCard {
            entity_id: affected_companies.first().cloned().unwrap_or_default(),
            narrative,
        })
    }

    // ── Memo section writer ────────────────────────────────────────

    /// Generate a weekly memo section for a region.
    ///
    /// * `top_insights` — `(title, summary, severity)` triples.
    pub async fn generate_memo_section(
        &self,
        region: &str,
        top_insights: &[(String, String, String)],
        week_number: u32,
        year: i32,
    ) -> Result<String> {
        self.gen
            .generate_exec_memo_section(region, top_insights, week_number, year)
            .await
    }

    // ── Batch enrichment helper ────────────────────────────────────

    /// Enrich multiple regional sections concurrently.
    ///
    /// Returns one [`EnrichedRegionalSection`] per `(region, snippets)` pair.
    /// Individual failures are logged and replaced with a plain pass-through
    /// entry so that a single LLM timeout does not abort the full batch.
    pub async fn enrich_regional_sections(
        &self,
        sections: &[(&str, Vec<String>, &str)],
    ) -> Vec<EnrichedRegionalSection> {
        let mut results = Vec::with_capacity(sections.len());

        for (region, country_list, industry_context) in sections {
            let geo = match self
                .assess_geopolitical_risk(region, country_list, industry_context)
                .await
            {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::warn!(region = %region, error = %e, "Geo assessment failed");
                    None
                }
            };

            let narrative_result = self
                .gen
                .generate_exec_memo_section(
                    region,
                    &[(
                        region.to_string(),
                        industry_context.to_string(),
                        "info".to_string(),
                    )],
                    0u32,
                    0i32,
                )
                .await;

            let llm_narrative = match narrative_result {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(region = %region, error = %e, "Memo section generation failed");
                    format!("No LLM narrative available for {region}.")
                }
            };

            results.push(EnrichedRegionalSection {
                region: region.to_string(),
                llm_narrative,
                geopolitical_assessment: geo,
            });
        }

        results
    }
}

// ────────────────────────────────────────────
// Unit tests (pure / mock-friendly)
// ────────────────────────────────────────────

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

    /// Verify that `EnrichedExecSummary` serialises cleanly to JSON.
    #[test]
    fn enriched_exec_summary_serializes() {
        let s = EnrichedExecSummary {
            original: "orig".to_string(),
            llm_narrative: "narrative".to_string(),
            key_risks: vec!["r1".to_string()],
            recommended_actions: vec!["a1".to_string()],
            confidence: 0.88,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("narrative"));
    }
}
