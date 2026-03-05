//! LLM-powered intelligence insight generation.
//!
//! Turns raw signal clusters and statistical findings into:
//! - **Narrative intelligence summaries** — readable analyst-grade analysis
//! - **Geopolitical assessments** — diplomatic/trade/regulatory context
//! - **Competitive intelligence** — market positioning and threat analysis
//! - **Supply chain risk narratives** — multi-source correlated risk stories
//! - **Weekly executive memo sections** — ready for GM/Board briefing
//!
//! All functions take pre-computed structured data (no I/O inside this module).

use crate::inference::{ChatMessage, InferenceConfig, LlmClient};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// A fully generated intelligence insight narrative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmInsightNarrative {
    /// One-line headline (max 120 chars)
    pub headline: String,
    /// Executive-level summary (2–3 sentences)
    pub executive_summary: String,
    /// Detailed analysis paragraph (4–6 sentences)
    pub detailed_analysis: String,
    /// Specific, actionable recommendation
    pub recommendation: String,
    /// Severity: critical | warning | info
    pub severity: String,
    /// Strategic category: supply_chain | geopolitical | competitive | regulatory | financial | technology | personnel
    pub category: String,
    /// Affected regions
    pub regions: Vec<String>,
    /// Confidence in this analysis (0..1)
    pub confidence: f64,
    /// Time horizon: immediate (< 1 week) | near_term (1–4 weeks) | medium_term (1–3 months) | long_term (> 3 months)
    pub time_horizon: String,
    /// Impact magnitude: critical | high | medium | low
    pub impact_magnitude: String,
}

/// Geopolitical intelligence assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeopoliticalAssessment {
    /// Affected countries or regions
    pub affected_regions: Vec<String>,
    /// Current situation description
    pub situation: String,
    /// Key actors involved
    pub key_actors: Vec<String>,
    /// Business impact assessment
    pub business_impact: String,
    /// Specific risks for electronics/EMS supply chain
    pub supply_chain_risk: String,
    /// Recommended monitoring actions
    pub monitoring_actions: Vec<String>,
    /// Probability of escalation: low | medium | high
    pub escalation_probability: String,
    /// Overall risk level: low | medium | high | critical
    pub risk_level: String,
    /// Confidence score (0..1)
    pub confidence: f64,
}

/// Competitive intelligence summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitiveIntelSummary {
    /// Company being analyzed
    pub company: String,
    /// Competitive threat level: minimal | low | moderate | elevated | critical
    pub threat_level: String,
    /// Strategic moves observed
    pub observed_moves: Vec<String>,
    /// Likely strategic intent
    pub inferred_intent: String,
    /// Market impact assessment
    pub market_impact: String,
    /// Recommended response
    pub recommended_response: String,
    /// Time pressure: immediate | moderate | low
    pub time_pressure: String,
    /// Confidence (0..1)
    pub confidence: f64,
}

/// Supply chain risk narrative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainRiskNarrative {
    /// Overall risk level: low | moderate | elevated | critical
    pub risk_level: String,
    /// Number of affected companies
    pub affected_company_count: usize,
    /// Affected regions
    pub affected_regions: Vec<String>,
    /// Root cause analysis
    pub root_cause: String,
    /// Downstream business impact
    pub business_impact: String,
    /// Mitigation steps
    pub mitigation_steps: Vec<String>,
    /// Estimated impact duration
    pub duration_estimate: String,
    /// Confidence (0..1)
    pub confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Generator
// ─────────────────────────────────────────────────────────────────────────────

/// LLM-powered insight generator.
pub struct InsightGenerator {
    client: LlmClient,
}

impl InsightGenerator {
    pub fn new(client: LlmClient) -> Self {
        Self { client }
    }

    /// Generate a narrative intelligence insight from a cluster of raw signals.
    ///
    /// `signal_type` — e.g. "supply_chain_disruption", "expansion", "geopolitical_risk"
    /// `signals` — list of (title, description, source_url) tuples
    /// `entity_names` — companies / people affected
    /// `region` — primary geographic focus
    pub async fn generate_insight_narrative(
        &self,
        signal_type: &str,
        signals: &[(String, String, String)],
        entity_names: &[String],
        region: &str,
    ) -> Result<LlmInsightNarrative> {
        let signals_text = signals
            .iter()
            .enumerate()
            .map(|(i, (title, desc, url))| {
                format!(
                    "[{}] {}\n  {}\n  Source: {}\n",
                    i + 1,
                    title,
                    crate::truncate_utf8(desc, 400),
                    url
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let entities_text = entity_names.join(", ");

        let system = concat!(
            "You are a senior intelligence analyst at an OSINT firm specializing in the electronics, ",
            "defense manufacturing, and supply chain sectors. You produce precise, actionable intelligence ",
            "reports for C-suite executives and procurement leadership.\n/no_think"
        );

        let user = format!(
            r#"Analyze the following intelligence signals of type "{signal_type}" affecting {entities_text} in {region} and produce a structured intelligence insight.

SIGNALS:
{signals_text}

Respond ONLY with valid JSON:
{{
  "headline": "<one-line headline max 120 chars>",
  "executive_summary": "<2-3 sentence executive summary>",
  "detailed_analysis": "<4-6 sentence detailed analysis with context and implications>",
  "recommendation": "<specific actionable recommendation>",
  "severity": "<critical|warning|info>",
  "category": "<supply_chain|geopolitical|competitive|regulatory|financial|technology|personnel>",
  "regions": ["<region1>"],
  "confidence": <0.0-1.0>,
  "time_horizon": "<immediate|near_term|medium_term|long_term>",
  "impact_magnitude": "<critical|high|medium|low>"
}}"#,
            signal_type = signal_type,
            entities_text = entities_text,
            region = region,
            signals_text = signals_text,
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.client.complete_with_config(messages, &config).await
            .with_context(|| format!("LLM insight narrative failed for type {}", signal_type))?;

        let mut narrative: LlmInsightNarrative = resp.parse_json()
            .with_context(|| "Failed to parse insight narrative JSON")?;

        narrative.confidence = narrative.confidence.clamp(0.0, 1.0);
        Ok(narrative)
    }

    /// Generate a geopolitical risk assessment.
    ///
    /// `event_description` — description of the geopolitical event(s)
    /// `affected_countries` — list of country codes
    /// `industry_context` — e.g. "electronics manufacturing, CHIPS Act compliance"
    pub async fn assess_geopolitical_risk(
        &self,
        event_description: &str,
        affected_countries: &[String],
        industry_context: &str,
    ) -> Result<GeopoliticalAssessment> {
        let countries_text = affected_countries.join(", ");

        let system = concat!(
            "You are a geopolitical risk analyst specializing in the intersection of global supply chains, ",
            "trade policy, and the electronics manufacturing industry. You assess risks with precision and ",
            "provide actionable intelligence for business leaders.\n/no_think"
        );

        let user = format!(
            r#"Assess the geopolitical risk of the following events for companies in the {industry_context} sector.

AFFECTED COUNTRIES: {countries_text}

EVENT DESCRIPTION:
{event_description}

Respond ONLY with valid JSON:
{{
  "affected_regions": ["<region1>"],
  "situation": "<current situation description>",
  "key_actors": ["<actor1>", "<actor2>"],
  "business_impact": "<direct business impact assessment>",
  "supply_chain_risk": "<specific electronics/EMS supply chain risk>",
  "monitoring_actions": ["<action1>", "<action2>"],
  "escalation_probability": "<low|medium|high>",
  "risk_level": "<low|medium|high|critical>",
  "confidence": <0.0-1.0>
}}"#,
            industry_context = industry_context,
            countries_text = countries_text,
            event_description = crate::truncate_utf8(event_description, 2000),
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.client.complete_with_config(messages, &config).await
            .with_context(|| "LLM geopolitical assessment failed")?;

        let mut assessment: GeopoliticalAssessment = resp.parse_json()
            .with_context(|| "Failed to parse geopolitical assessment JSON")?;

        assessment.confidence = assessment.confidence.clamp(0.0, 1.0);
        Ok(assessment)
    }

    /// Analyze competitive intelligence for a specific company.
    pub async fn analyze_competitive_intel(
        &self,
        company_name: &str,
        company_region: &str,
        observed_signals: &[(String, String)],
    ) -> Result<CompetitiveIntelSummary> {
        let signals_text = observed_signals
            .iter()
            .map(|(signal_type, description)| format!("- {}: {}", signal_type, description))
            .collect::<Vec<_>>()
            .join("\n");

        let system = concat!(
            "You are a competitive intelligence analyst for the electronics manufacturing and EMS sector. ",
            "You analyze competitor activity to identify strategic threats and opportunities.\n/no_think"
        );

        let user = format!(
            r#"Analyze competitive intelligence signals for {company_name} ({company_region}).

OBSERVED SIGNALS:
{signals_text}

Respond ONLY with valid JSON:
{{
  "company": "{company_name}",
  "threat_level": "<minimal|low|moderate|elevated|critical>",
  "observed_moves": ["<move1>", "<move2>"],
  "inferred_intent": "<likely strategic intent>",
  "market_impact": "<market impact on your competitive position>",
  "recommended_response": "<recommended defensive or offensive action>",
  "time_pressure": "<immediate|moderate|low>",
  "confidence": <0.0-1.0>
}}"#,
            company_name = company_name,
            company_region = company_region,
            signals_text = signals_text,
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.client.complete_with_config(messages, &config).await
            .with_context(|| format!("LLM competitive intel failed for {}", company_name))?;

        let mut summary: CompetitiveIntelSummary = resp.parse_json()
            .with_context(|| "Failed to parse competitive intel JSON")?;

        summary.confidence = summary.confidence.clamp(0.0, 1.0);
        Ok(summary)
    }

    /// Generate a supply chain risk narrative from correlated disruption signals.
    pub async fn narrate_supply_chain_risk(
        &self,
        affected_companies: &[String],
        affected_regions: &[String],
        signal_descriptions: &[String],
    ) -> Result<SupplyChainRiskNarrative> {
        let companies_text = affected_companies.join(", ");
        let regions_text = affected_regions.join(", ");
        let signals_text = signal_descriptions
            .iter()
            .map(|s| format!("- {}", s))
            .collect::<Vec<_>>()
            .join("\n");

        let system = concat!(
            "You are a supply chain risk analyst with deep expertise in electronics, semiconductors, and EMS. ",
            "You identify root causes, quantify impacts, and provide actionable mitigation strategies.\n/no_think"
        );

        let user = format!(
            r#"Analyze the following correlated supply chain risk signals.

AFFECTED COMPANIES: {companies_text}
AFFECTED REGIONS: {regions_text}

SIGNALS:
{signals_text}

Respond ONLY with valid JSON:
{{
  "risk_level": "<low|moderate|elevated|critical>",
  "affected_company_count": {company_count},
  "affected_regions": [{regions_json}],
  "root_cause": "<root cause analysis in 2-3 sentences>",
  "business_impact": "<downstream business impact>",
  "mitigation_steps": ["<step1>", "<step2>", "<step3>"],
  "duration_estimate": "<estimated impact duration>",
  "confidence": <0.0-1.0>
}}"#,
            companies_text = companies_text,
            regions_text = regions_text,
            signals_text = signals_text,
            company_count = affected_companies.len(),
            regions_json = affected_regions
                .iter()
                .map(|r| format!("\"{}\"", r))
                .collect::<Vec<_>>()
                .join(", "),
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.client.complete_with_config(messages, &config).await
            .with_context(|| "LLM supply chain risk narrative failed")?;

        let mut narrative: SupplyChainRiskNarrative = resp.parse_json()
            .with_context(|| "Failed to parse supply chain risk JSON")?;

        narrative.confidence = narrative.confidence.clamp(0.0, 1.0);
        Ok(narrative)
    }

    /// Generate a weekly executive memo section for a specific region.
    ///
    /// Returns a formatted markdown section ready for inclusion in the weekly memo.
    pub async fn generate_exec_memo_section(
        &self,
        region: &str,
        top_insights: &[(String, String, String)], // (title, summary, severity)
        week_number: u32,
        year: i32,
    ) -> Result<String> {
        let insights_text = top_insights
            .iter()
            .enumerate()
            .map(|(i, (title, summary, severity))| {
                format!(
                    "[{}] [{severity}] {}\n{}\n",
                    i + 1,
                    title,
                    crate::truncate_utf8(summary, 300)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = concat!(
            "You are writing the weekly intelligence memo for a C-suite executive in the electronics ",
            "manufacturing sector. Be concise, precise, and directly actionable. Each section should be ",
            "readable in under 2 minutes.\n/no_think"
        );

        let user = format!(
            r#"Write the {region} section of the Week {week_number} {year} executive intelligence memo.

TOP INTELLIGENCE ITEMS:
{insights_text}

Write a professional memo section in markdown format. Include:
1. A 2-sentence regional situation summary
2. Bullet points for each key intelligence item (SIGNAL → IMPLICATION → ACTION)
3. A "Watch List" of 2-3 items to monitor next week

Keep the entire section under 350 words. Use professional intelligence memo language."#,
            region = region,
            week_number = week_number,
            year = year,
            insights_text = insights_text,
        );

        let config = InferenceConfig::narrative();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self.client.complete_with_config(messages, &config).await
            .with_context(|| format!("LLM memo section failed for {}", region))?;

        Ok(resp.text)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_insight_narrative_clamps_confidence() {
        let mut n = LlmInsightNarrative {
            headline: "test".into(),
            executive_summary: "s".into(),
            detailed_analysis: "d".into(),
            recommendation: "r".into(),
            severity: "warning".into(),
            category: "supply_chain".into(),
            regions: vec!["TN".into()],
            confidence: 1.5,
            time_horizon: "immediate".into(),
            impact_magnitude: "high".into(),
        };
        n.confidence = n.confidence.clamp(0.0, 1.0);
        assert_eq!(n.confidence, 1.0);
    }
}
