//! LLM-powered POI psychometric profiling and engagement copy generation.
//!
//! Uses the local Qwen3 inference backend to produce:
//! - **Psychometric profiles** — decision style, risk appetite, pain index, preferred proof
//! - **Priority vectors** — what matters most to this person (cost, quality, speed…)
//! - **"What They Want To Hear"** engagement copy — personalised outreach messages
//! - **Background summaries** — synthesized narrative from artifact corpus
//!
//! All functions accept pre-loaded artifact text to avoid DB I/O being performed
//! inside this module (no I/O here — pure LLM inference wrappers).

use crate::inference::{ChatMessage, InferenceConfig, LlmClient};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// Full psychometric profile for a POI as inferred by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPsychProfile {
    /// Primary decision-making style: CostFirst | QualityFirst | SpeedFirst
    /// | RiskFirst | ComplianceFirst | BalancedAnalytical
    pub decision_style: String,
    /// Risk appetite: EarlyAdopter | Pragmatist | Conservative | Laggard
    pub change_appetite: String,
    /// Pain index in [0, 1] — how urgently they need a solution right now
    pub pain_index: f64,
    /// Dominant priority: cost | quality | speed | resilience | compliance | security
    pub dominant_priority: String,
    /// Priority vector weights (each in [0,1], should roughly sum to 1)
    pub priority_vector: PriorityVectorLlm,
    /// Most convincing proof types for this person
    pub preferred_proof_types: Vec<String>,
    /// Key communication style notes
    pub communication_style: String,
    /// Key trigger topics that will resonate
    pub trigger_topics: Vec<String>,
    /// Risk tolerance in [0,1]
    pub risk_tolerance: f64,
    /// Confidence in this profile (0..1), based on artifact richness
    pub profile_confidence: f64,
    /// Artifact richness score (0..1) — how much source material was available.
    /// Distinct from profile_confidence which measures inference quality.
    #[serde(default)]
    pub artifact_richness: f64,
    /// Short reasoning chain explaining the profile
    pub reasoning: String,
}

/// Priority vector with individual dimension weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityVectorLlm {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
    pub resilience: f64,
    pub compliance: f64,
    pub security: f64,
}

impl PriorityVectorLlm {
    /// Validate that all dimensions are in [0,1].
    pub fn validate_clamped(&self) -> Self {
        let clamp = |v: f64| v.clamp(0.0, 1.0);
        PriorityVectorLlm {
            cost: clamp(self.cost),
            quality: clamp(self.quality),
            speed: clamp(self.speed),
            resilience: clamp(self.resilience),
            compliance: clamp(self.compliance),
            security: clamp(self.security),
        }
    }
}

/// Engagement copy tailored to this specific POI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementCopy {
    /// Short one-liner subject for email/InMail
    pub subject_line: String,
    /// Opening paragraph (2–3 sentences, referencing their specific pain points)
    pub opening_paragraph: String,
    /// Core value proposition paragraph pitched to their dominant priority
    pub value_proposition: String,
    /// Call to action tailored to their decision style
    pub call_to_action: String,
    /// PS note referencing recent news about their company (if any)
    pub ps_note: Option<String>,
    /// Recommended tone: formal | conversational | technical | executive
    pub tone: String,
    /// Full assembled message
    pub full_message: String,
}

/// Background intelligence summary synthesized from artifact corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiBackgroundSummary {
    /// Executive biography paragraph
    pub bio: String,
    /// Key career milestones
    pub career_milestones: Vec<String>,
    /// Identified professional pain points
    pub pain_points: Vec<String>,
    /// Recent public statements or known positions
    pub known_positions: Vec<String>,
    /// Red flags or concerns (sanctions exposure, reputational issues)
    pub red_flags: Vec<String>,
    /// Network connections of interest
    pub notable_connections: Vec<String>,
    /// Confidence in summary (0..1)
    pub confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Profiler
// ─────────────────────────────────────────────────────────────────────────────

/// LLM-powered POI profiler.
pub struct PoiProfiler {
    client: LlmClient,
}

impl PoiProfiler {
    pub fn new(client: LlmClient) -> Self {
        Self { client }
    }

    /// Infer psychometric profile from a collection of artifact texts.
    ///
    /// `artifacts` — list of (title, content_excerpt) tuples scraped about the POI.
    /// `poi_name` — full name for context.
    /// `poi_role` — current role / title for context.
    pub async fn infer_psych_profile(
        &self,
        poi_name: &str,
        poi_role: &str,
        artifacts: &[(String, String)],
    ) -> Result<LlmPsychProfile> {
        let artifacts_text = format_artifacts(artifacts, 6000);

        let system = concat!(
            "You are a senior B2B sales intelligence analyst specializing in psychographic profiling ",
            "of procurement, quality, and engineering executives in the electronics and defense manufacturing ",
            "sector. Your analysis is precise, evidence-based, and actionable.\n/no_think"
        );

        let user = format!(
            r#"Analyze the following collected intelligence about {poi_name} ({poi_role}) and produce a structured psychometric profile.

ARTIFACTS:
{artifacts_text}

Respond ONLY with valid JSON matching this exact schema:
{{
  "decision_style": "<CostFirst|QualityFirst|SpeedFirst|RiskFirst|ComplianceFirst|BalancedAnalytical>",
  "change_appetite": "<EarlyAdopter|Pragmatist|Conservative|Laggard>",
  "pain_index": <0.0 to 1.0>,
  "dominant_priority": "<cost|quality|speed|resilience|compliance|security>",
  "priority_vector": {{
    "cost": <0.0-1.0>,
    "quality": <0.0-1.0>,
    "speed": <0.0-1.0>,
    "resilience": <0.0-1.0>,
    "compliance": <0.0-1.0>,
    "security": <0.0-1.0>
  }},
  "preferred_proof_types": ["<KpiMetrics|Certifications|CaseStudies|AuditReadiness|TechDemos|CostTransparency>"],
  "communication_style": "<brief description of how to communicate with them>",
  "trigger_topics": ["<topic1>", "<topic2>"],
  "risk_tolerance": <0.0 to 1.0>,
  "profile_confidence": <0.0 to 1.0>,
  "reasoning": "<1-2 sentence explanation of profile>"
}}"#,
            poi_name = poi_name,
            poi_role = poi_role,
            artifacts_text = artifacts_text,
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self
            .client
            .complete_with_config(messages, &config)
            .await
            .with_context(|| format!("LLM psych profile failed for {}", poi_name))?;

        let mut profile: LlmPsychProfile = resp
            .parse_json()
            .with_context(|| "Failed to parse psych profile JSON")?;

        // Clamp numeric values to valid ranges, logging when values were out of bounds
        if profile.pain_index < 0.0 || profile.pain_index > 1.0 {
            tracing::warn!(
                poi = %poi_name,
                field = "pain_index",
                raw_value = profile.pain_index,
                "POI profiler: clamping out-of-range value to [0, 1]"
            );
            profile.pain_index = profile.pain_index.clamp(0.0, 1.0);
        }
        if profile.risk_tolerance < 0.0 || profile.risk_tolerance > 1.0 {
            tracing::warn!(
                poi = %poi_name,
                field = "risk_tolerance",
                raw_value = profile.risk_tolerance,
                "POI profiler: clamping out-of-range value to [0, 1]"
            );
            profile.risk_tolerance = profile.risk_tolerance.clamp(0.0, 1.0);
        }
        if profile.profile_confidence < 0.0 || profile.profile_confidence > 1.0 {
            tracing::warn!(
                poi = %poi_name,
                field = "profile_confidence",
                raw_value = profile.profile_confidence,
                "POI profiler: clamping out-of-range value to [0, 1]"
            );
            profile.profile_confidence = profile.profile_confidence.clamp(0.0, 1.0);
        }
        profile.priority_vector = profile.priority_vector.validate_clamped();

        // Compute artifact richness separately from profile confidence.
        // Richness is based on the volume of source material available.
        let artifact_count = artifacts.len();
        profile.artifact_richness = match artifact_count {
            0 => 0.0,
            1..=2 => 0.2,
            3..=5 => 0.5,
            6..=10 => 0.75,
            _ => 1.0,
        };

        Ok(profile)
    }

    /// Generate personalized "What They Want To Hear" engagement copy.
    ///
    /// `company_name` — the POI's employer (for personalisation).
    /// `product_or_service` — what you're pitching.
    /// `recent_news` — optional recent news about their company to reference.
    pub async fn generate_engagement_copy(
        &self,
        poi_name: &str,
        poi_role: &str,
        company_name: &str,
        psych_profile: &LlmPsychProfile,
        product_or_service: &str,
        recent_news: Option<&str>,
    ) -> Result<EngagementCopy> {
        let news_section = match recent_news {
            Some(n) => format!(
                "\nRECENT NEWS ABOUT THEIR COMPANY:\n{}",
                crate::truncate_utf8(n, 500)
            ),
            None => String::new(),
        };

        let system = concat!(
            "You are an expert enterprise B2B copywriter. ",
            "You write concise, compelling outreach that is highly personalized to the specific role and responsibilities of the recipient. ",
            "Tailor the message to their actual function:\n",
            "- For procurement/supply chain/purchasing: emphasize TCO, supply security, lead times, compliance\n",
            "- For quality/compliance: emphasize certifications, audit readiness, process capability, traceability\n",
            "- For engineering/R&D: emphasize DFM support, prototyping speed, technical collaboration, BOM optimization\n",
            "- For operations/manufacturing: emphasize line stability, capacity flexibility, OTD, escalation paths\n",
            "- For executives/C-suite: emphasize strategic partnership, growth, regional advantage, innovation\n",
            "- For security/IT: emphasize zero-trust, incident response, vendor risk management\n",
            "- For finance: emphasize cost transparency, ROI, margin impact\n",
            "Match the tone to their seniority: technical depth for engineers, strategic framing for executives, operational specifics for managers.\n",
            "IMPORTANT: Never default to CEO/executive framing when the recipient is a functional buyer or manager. ",
            "A procurement manager needs cost and supply details, not strategic vision.\n/no_think"
        );

        let user = format!(
            r#"Write personalized B2B outreach for {poi_name}, {poi_role} at {company_name}.

THEIR PSYCHOGRAPHIC PROFILE:
- Decision style: {decision_style}
- Change appetite: {change_appetite}
- Pain index: {pain_index:.1} / 1.0
- Dominant priority: {dominant_priority}
- Preferred proof: {proof_types}
- Communication style: {comm_style}
- Trigger topics: {triggers}{news_section}

WE ARE PITCHING: {product_or_service}

Respond ONLY with valid JSON:
{{
  "subject_line": "<compelling email subject under 60 chars>",
  "opening_paragraph": "<2-3 sentences referencing their specific context>",
  "value_proposition": "<1 paragraph pitched to their {dominant_priority} priority>",
  "call_to_action": "<1 sentence CTA matching their {decision_style} style and {change_appetite} appetite>",
  "ps_note": "<optional 1-sentence PS referencing recent news, or null>",
  "tone": "<formal|conversational|technical|executive>",
  "full_message": "<complete assembled message combining all sections>"
}}"#,
            poi_name = poi_name,
            poi_role = poi_role,
            company_name = company_name,
            decision_style = psych_profile.decision_style,
            change_appetite = psych_profile.change_appetite,
            pain_index = psych_profile.pain_index,
            dominant_priority = psych_profile.dominant_priority,
            proof_types = psych_profile.preferred_proof_types.join(", "),
            comm_style = psych_profile.communication_style,
            triggers = psych_profile.trigger_topics.join(", "),
            news_section = news_section,
            product_or_service = product_or_service,
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self
            .client
            .complete_with_config(messages, &config)
            .await
            .with_context(|| format!("LLM engagement copy failed for {}", poi_name))?;

        resp.parse_json()
            .with_context(|| "Failed to parse engagement copy JSON")
    }

    /// Synthesize background intelligence summary from artifact corpus.
    pub async fn synthesize_background(
        &self,
        poi_name: &str,
        poi_role: &str,
        artifacts: &[(String, String)],
    ) -> Result<PoiBackgroundSummary> {
        let artifacts_text = format_artifacts(artifacts, 8000);

        let system = concat!(
            "You are an OSINT analyst compiling background intelligence dossiers on business executives. ",
            "You are precise, fact-based, and flag uncertainty when evidence is thin.\n/no_think"
        );

        let user = format!(
            r#"Compile a background intelligence summary for {poi_name} ({poi_role}) from the following collected artifacts.

ARTIFACTS:
{artifacts_text}

Respond ONLY with valid JSON:
{{
  "bio": "<2-3 sentence professional biography>",
  "career_milestones": ["<milestone1>", "<milestone2>"],
  "pain_points": ["<pain1>", "<pain2>"],
  "known_positions": ["<public statement or known position>"],
  "red_flags": ["<concern1 if any, else empty array>"],
  "notable_connections": ["<person/org of interest>"],
  "confidence": <0.0 to 1.0 based on artifact richness>
}}"#,
            poi_name = poi_name,
            poi_role = poi_role,
            artifacts_text = artifacts_text,
        );

        let config = InferenceConfig::json_structured();
        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let resp = self
            .client
            .complete_with_config(messages, &config)
            .await
            .with_context(|| format!("LLM background synthesis failed for {}", poi_name))?;

        let mut summary: PoiBackgroundSummary = resp
            .parse_json()
            .with_context(|| "Failed to parse background summary JSON")?;

        if summary.confidence < 0.0 || summary.confidence > 1.0 {
            tracing::warn!(
                poi = %poi_name,
                field = "background_confidence",
                raw_value = summary.confidence,
                "POI profiler: clamping out-of-range background confidence to [0, 1]"
            );
            summary.confidence = summary.confidence.clamp(0.0, 1.0);
        }
        Ok(summary)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Format artifacts into a compact text block for LLM consumption.
/// Limits total length to `max_chars` to stay within token budget.
fn format_artifacts(artifacts: &[(String, String)], max_chars: usize) -> String {
    let mut out = String::new();
    for (i, (title, content)) in artifacts.iter().enumerate() {
        let entry = format!(
            "[{}] {}\n{}\n\n",
            i + 1,
            title,
            crate::truncate_utf8(content, 800)
        );
        if out.len() + entry.len() > max_chars {
            out.push_str("[... additional artifacts truncated for token budget ...]\n");
            break;
        }
        out.push_str(&entry);
    }
    if out.is_empty() {
        out.push_str("[No artifacts collected yet]\n");
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_artifacts_truncates() {
        let arts: Vec<_> = (0..100)
            .map(|i| (format!("Article {}", i), "x".repeat(500)))
            .collect();
        let result = format_artifacts(&arts, 2000);
        assert!(
            result.len() <= 2000 + 100,
            "Should be bounded near max_chars"
        );
        assert!(result.contains("truncated"), "Should mention truncation");
    }

    #[test]
    fn format_artifacts_empty() {
        let result = format_artifacts(&[], 2000);
        assert!(result.contains("No artifacts"));
    }

    #[test]
    fn priority_vector_clamp() {
        let pv = PriorityVectorLlm {
            cost: 1.5,
            quality: -0.2,
            speed: 0.5,
            resilience: 2.0,
            compliance: 0.0,
            security: 0.8,
        };
        let clamped = pv.validate_clamped();
        assert_eq!(clamped.cost, 1.0);
        assert_eq!(clamped.quality, 0.0);
        assert_eq!(clamped.resilience, 1.0);
        assert_eq!(clamped.speed, 0.5);
    }
}
