//! Shared, evidence-bound warning analysis service (audit P0-3, P0-4, P1-2,
//! P1-3, P1-4, P1-5, P1-10).
//!
//! The web panel and the JSON API both run the *same* real analysis, and both
//! go through the same durable, asynchronous run pipeline:
//!
//! 1. [`enqueue_analysis`] gathers the **full bounded evidence set**, computes
//!    a digest over it, and enqueues a `warning_analysis_runs` row. Identical
//!    evidence digests deduplicate onto an in-flight or already-succeeded run
//!    (migration 079), so repeated clicks do not pay for the same work twice.
//! 2. [`spawn_analysis_executor`] runs the LLM outside the request, strictly
//!    parses its **typed JSON output** (`claims` / `impact` / `actions` /
//!    `limitations`), rejects any output that is not pure JSON, and validates
//!    every claim: unknown evidence ids, observed/inference claims without
//!    evidence, and out-of-range confidence values all fail the run.
//! 3. Validated claims are persisted through the same claim-evidence integrity
//!    system as insights (foreign keys to observations/insights + claim-kind
//!    policy), and the run stores its rendered output, model and prompt
//!    version for caching and comparison.
//!
//! Free-prose paragraph-index parsing, count-derived "source reliability" /
//! "data sufficiency" labels, and the 300-second blocking timeout no longer
//! exist here. Evidence counts are reported as counts, while quality is
//! measured by the reusable [`EvidenceQuality`] model in `apex-core`.

#![cfg(feature = "llm")]

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use apex_core::analysis::{
    assess_evidence_quality, registrable_domain, EvidenceQuality, EvidenceRecord, EvidenceStance,
};
use apex_core::claims::{AnalysisClaimRecord, ClaimKind, ClaimSection, EvidenceRef};
use apex_core::intelligence_profile::IntelligenceProfile;
use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
use apex_store::postgres::{
    NewWarningAnalysisRun, ObservationRow, PgStore, WarningAnalysisRunRow, WarningRow,
};

/// Version of the prompt/JSON contract. Bump when the contract changes so new
/// generations are comparable against (not merged with) old ones.
pub const ANALYSIS_PROMPT_VERSION: &str = "warning-analysis-v2";

/// Output schema version stored with each run.
pub const ANALYSIS_OUTPUT_SCHEMA_VERSION: u32 = 2;

/// Documented bounded-evidence caps. The prompt states how many items were
/// actually sent versus how many were available, and the run row persists both
/// counts so a result can never imply it saw more evidence than it did.
pub const MAX_EVIDENCE_OBSERVATIONS: usize = 60;
pub const MAX_EVIDENCE_INSIGHTS: usize = 10;
pub const MAX_OBSERVATION_EXCERPT_CHARS: usize = 600;
pub const MAX_INSIGHT_SUMMARY_CHARS: usize = 400;

/// Bounds on the model's structured output.
pub const MAX_CLAIMS: usize = 12;
pub const MAX_IMPACT_ITEMS: usize = 8;
pub const MAX_ACTIONS: usize = 8;
pub const MAX_LIMITATIONS: usize = 8;
pub const MAX_CLAIM_TEXT_CHARS: usize = 1200;
pub const MAX_LIMITATION_CHARS: usize = 500;

/// A queued/running run with no progress for this long is failed by the next
/// status poll or enqueue, so a process crash cannot leave it "running" forever.
/// This is longer than any supported model timeout (default 300 s) plus
/// retries, so a healthy run is never expired underneath itself.
pub const STALE_RUN_SECONDS: i64 = 1800;

/// Model configuration plus domain profile used for warning analysis, injected
/// as an axum extension by the binary when the LLM runtime is configured.
#[derive(Clone)]
pub struct WarningAnalysisModel {
    pub primary: ModelConfig,
    /// Configurable domain profile (audit P1-2); replaces the hard-coded
    /// electronics/defense/supply-chain system prompt.
    pub profile: IntelligenceProfile,
}

// ─────────────────────────────────────────────────────────────────────────────
// Model output contract (strict)
// ─────────────────────────────────────────────────────────────────────────────

/// The exact JSON object the model must return. `deny_unknown_fields` rejects
/// extra prose-shaped fields; [`parse_analysis_payload`] additionally rejects
/// any text before/after the JSON object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPayload {
    pub claims: Vec<PayloadClaim>,
    pub impact: Vec<PayloadClaim>,
    pub actions: Vec<PayloadClaim>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadClaim {
    pub text: String,
    pub claim_kind: PayloadClaimKind,
    #[serde(default)]
    pub evidence_ids: Vec<Uuid>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadClaimKind {
    Observed,
    Inference,
    Recommendation,
}

impl PayloadClaimKind {
    fn to_record_kind(self) -> ClaimKind {
        match self {
            Self::Observed => ClaimKind::Observed,
            Self::Inference => ClaimKind::Inference,
            Self::Recommendation => ClaimKind::Recommendation,
        }
    }
}

/// Validated, evidence-bound analysis ready for persistence.
#[derive(Debug, Clone)]
pub struct ValidatedAnalysis {
    pub claims: Vec<AnalysisClaimRecord>,
    pub impact: Vec<AnalysisClaimRecord>,
    pub actions: Vec<AnalysisClaimRecord>,
    pub limitations: Vec<String>,
}

impl ValidatedAnalysis {
    /// All claims in one list (with their sections) for persistence.
    pub fn records(&self) -> Vec<AnalysisClaimRecord> {
        let mut records = self.claims.clone();
        records.extend(self.impact.iter().cloned());
        records.extend(self.actions.iter().cloned());
        records
    }
}

/// Parse the model output as pure JSON. Any prose before/after the object, any
/// markdown fence, and any unknown field is rejected: this output feeds a
/// claim-level integrity system, so "best effort" extraction is not acceptable.
pub fn parse_analysis_payload(raw: &str) -> Result<AnalysisPayload> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("analysis payload is empty");
    }
    serde_json::from_str::<AnalysisPayload>(trimmed)
        .with_context(|| "analysis payload is not a valid typed JSON object (prose/fences/unknown fields are rejected)")
}

/// Validate a parsed payload against the evidence the model was given.
///
/// * Every cited evidence id must resolve to a real observation or insight.
/// * `observed`/`inference` claims must cite at least one evidence id.
/// * `recommendation` claims may cite none but stay marked as recommendations.
/// * Confidence values must be finite and within `0.0..=1.0`.
/// * Output size is bounded per section.
pub fn validate_analysis_payload(
    payload: AnalysisPayload,
    observation_ids: &HashSet<Uuid>,
    insight_ids: &HashSet<Uuid>,
) -> Result<ValidatedAnalysis> {
    if payload.claims.is_empty() {
        anyhow::bail!("analysis must contain at least one claim");
    }
    if payload.claims.len() > MAX_CLAIMS {
        anyhow::bail!(
            "analysis returned {} claims; maximum is {MAX_CLAIMS}",
            payload.claims.len()
        );
    }
    if payload.impact.len() > MAX_IMPACT_ITEMS {
        anyhow::bail!(
            "analysis returned {} impact items; maximum is {MAX_IMPACT_ITEMS}",
            payload.impact.len()
        );
    }
    if payload.actions.len() > MAX_ACTIONS {
        anyhow::bail!(
            "analysis returned {} actions; maximum is {MAX_ACTIONS}",
            payload.actions.len()
        );
    }
    if payload.limitations.len() > MAX_LIMITATIONS {
        anyhow::bail!(
            "analysis returned {} limitations; maximum is {MAX_LIMITATIONS}",
            payload.limitations.len()
        );
    }

    let claims = validate_section(
        payload.claims,
        ClaimSection::Claim,
        observation_ids,
        insight_ids,
    )?;
    let impact = validate_section(
        payload.impact,
        ClaimSection::Impact,
        observation_ids,
        insight_ids,
    )?;
    let actions = validate_section(
        payload.actions,
        ClaimSection::Action,
        observation_ids,
        insight_ids,
    )?;

    let mut limitations = Vec::with_capacity(payload.limitations.len());
    for limitation in payload.limitations {
        let text = limitation.trim();
        if text.is_empty() {
            anyhow::bail!("analysis returned an empty limitation");
        }
        if text.chars().count() > MAX_LIMITATION_CHARS {
            anyhow::bail!("analysis limitation exceeds {MAX_LIMITATION_CHARS} characters");
        }
        limitations.push(text.to_string());
    }

    Ok(ValidatedAnalysis {
        claims,
        impact,
        actions,
        limitations,
    })
}

fn validate_section(
    claims: Vec<PayloadClaim>,
    section: ClaimSection,
    observation_ids: &HashSet<Uuid>,
    insight_ids: &HashSet<Uuid>,
) -> Result<Vec<AnalysisClaimRecord>> {
    let mut validated = Vec::with_capacity(claims.len());
    for claim in claims {
        let text = claim.text.trim();
        if text.is_empty() {
            anyhow::bail!("analysis returned an empty {} text", section.as_str());
        }
        if text.chars().count() > MAX_CLAIM_TEXT_CHARS {
            anyhow::bail!(
                "analysis {} text exceeds {MAX_CLAIM_TEXT_CHARS} characters",
                section.as_str()
            );
        }
        let mut evidence = Vec::with_capacity(claim.evidence_ids.len());
        for id in claim.evidence_ids {
            if observation_ids.contains(&id) {
                evidence.push(EvidenceRef::Observation(id));
            } else if insight_ids.contains(&id) {
                evidence.push(EvidenceRef::Insight(id));
            } else {
                anyhow::bail!(
                    "analysis {section} '{}' cites unknown evidence id {id}; \
                     citations must resolve to an observation or insight supplied in the prompt",
                    text,
                    section = section.as_str()
                );
            }
        }
        let record = AnalysisClaimRecord::new(
            text,
            evidence,
            claim.confidence,
            claim.claim_kind.to_record_kind(),
        )
        .with_section(section);
        record
            .validate()
            .with_context(|| format!("analysis {} '{}'", section.as_str(), text))?;
        validated.push(record);
    }
    Ok(validated)
}

// ─────────────────────────────────────────────────────────────────────────────
// Evidence gathering (full bounded set, with sent-vs-available counts)
// ─────────────────────────────────────────────────────────────────────────────

/// The bounded evidence set handed to the model. `*_available` counts the
/// real corpus, `observations`/`insights` are what was actually sent after the
/// documented caps — the two are never conflated.
#[derive(Debug, Clone)]
pub struct EvidenceBundle {
    pub observations: Vec<ObservationRow>,
    pub insights: Vec<apex_store::postgres::InsightRow>,
    pub observations_available: usize,
    pub insights_available: usize,
    pub entity_names: Vec<String>,
    /// Distinct registrable domains across the warning's own sources and the
    /// observation corpus.
    pub source_domains: Vec<String>,
    /// `(domain, tier, effective_reliability)` from `source_reliability_stats`.
    pub source_reliability: Vec<(String, String, f64)>,
}

impl EvidenceBundle {
    pub fn observation_ids(&self) -> HashSet<Uuid> {
        self.observations.iter().map(|row| row.id).collect()
    }

    pub fn insight_ids(&self) -> HashSet<Uuid> {
        self.insights.iter().map(|row| row.id).collect()
    }
}

/// Gather the full bounded evidence set: up to
/// [`MAX_EVIDENCE_OBSERVATIONS`] deduplicated observations and
/// [`MAX_EVIDENCE_INSIGHTS`] related insights, newest first.
pub async fn gather_evidence(store: &PgStore, warning: &WarningRow) -> Result<EvidenceBundle> {
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
            .get_observations_by_entity(*entity_id, 60)
            .await
            .with_context(|| {
                format!("warning analysis: failed to load observations for entity {entity_id}")
            })?;
        all_observations.extend(observations);
    }
    all_observations.sort_by_key(|observation| std::cmp::Reverse(observation.ts_utc));
    {
        let mut seen = HashSet::new();
        all_observations.retain(|observation| {
            let key = format!(
                "{}:{}",
                observation.observation_type,
                observation
                    .value
                    .to_string()
                    .chars()
                    .take(120)
                    .collect::<String>()
            );
            seen.insert(key)
        });
    }
    let observations_available = all_observations.len();
    all_observations.truncate(MAX_EVIDENCE_OBSERVATIONS);

    let all_insights = store
        .get_insights_by_entity_ids(&entity_ids, 50)
        .await
        .context("warning analysis: failed to load related insights")?;
    let insights_available = all_insights.len();
    let insights: Vec<_> = all_insights
        .into_iter()
        .take(MAX_EVIDENCE_INSIGHTS)
        .collect();

    let mut domains: Vec<String> = Vec::new();
    for url in warning.source_urls.as_deref().unwrap_or_default() {
        if let Some(domain) = registrable_domain(url) {
            domains.push(domain);
        }
    }
    for observation in &all_observations {
        if let Some(domain) = observation_source_domain(observation) {
            domains.push(domain);
        }
    }
    domains.sort();
    domains.dedup();

    let source_reliability = store
        .get_source_reliability_stats_for_domains(&domains)
        .await
        .context("warning analysis: failed to load source reliability stats")?;

    Ok(EvidenceBundle {
        observations: all_observations,
        insights,
        observations_available,
        insights_available,
        entity_names,
        source_domains: domains,
        source_reliability,
    })
}

/// Extract the source URL recorded in an observation's provenance, mirroring
/// the `normalized_observations` view (`source_domain`, `domain`,
/// `source_url`, `url`).
pub fn observation_source_url(observation: &ObservationRow) -> Option<String> {
    for key in ["source_url", "url", "canonical_url"] {
        if let Some(url) = observation.provenance.get(key).and_then(|v| v.as_str()) {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn observation_source_domain(observation: &ObservationRow) -> Option<String> {
    for key in ["source_domain", "domain"] {
        if let Some(domain) = observation.provenance.get(key).and_then(|v| v.as_str()) {
            if let Some(domain) = registrable_domain(domain) {
                return Some(domain);
            }
        }
    }
    observation_source_url(observation).and_then(|url| registrable_domain(&url))
}

fn source_reliability_for(
    bundle: &EvidenceBundle,
    domain: Option<&str>,
) -> (Option<f64>, Option<String>) {
    let Some(domain) = domain else {
        return (None, None);
    };
    bundle
        .source_reliability
        .iter()
        .find(|(known_domain, _, _)| known_domain == domain)
        .map(|(_, tier, reliability)| (Some(*reliability), Some(tier.clone())))
        .unwrap_or((None, None))
}

/// Reusable evidence-quality assessment over the exact evidence set the model
/// saw: observations are direct evidence, related insights are derived.
pub fn assess_bundle_quality(bundle: &EvidenceBundle, now: DateTime<Utc>) -> EvidenceQuality {
    let mut records: Vec<EvidenceRecord> = Vec::new();
    for observation in &bundle.observations {
        let domain = observation_source_domain(observation);
        let (reliability, tier) = source_reliability_for(bundle, domain.as_deref());
        let mut record = EvidenceRecord::new(
            observation.confidence.unwrap_or(0.6),
            EvidenceStance::Supports,
        )
        .with_source_type(observation.observation_type.clone())
        .with_observed_at(observation.ts_utc)
        .with_source_reliability(reliability, tier);
        if let Some(url) = observation_source_url(observation) {
            record = record.with_source_url(url);
        } else if let Some(domain) = domain {
            record = record.with_source_url(domain);
        }
        records.push(record);
    }
    for insight in &bundle.insights {
        let mut record =
            EvidenceRecord::new(insight.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                .with_source_type(format!(
                    "insight:{}",
                    insight.insight_type.as_deref().unwrap_or("insight")
                ))
                .derived();
        if let Some(created_at) = insight.created_at {
            record = record.with_observed_at(created_at);
        }
        records.push(record);
    }
    assess_evidence_quality(&records, now)
}

// ─────────────────────────────────────────────────────────────────────────────
// Evidence digest (dedupe/caching key)
// ─────────────────────────────────────────────────────────────────────────────

/// Stable digest of the exact evidence set and warning revision. Two requests
/// with the same digest may reuse the same run; any evidence or warning change
/// produces a different digest.
pub fn evidence_digest(warning: &WarningRow, bundle: &EvidenceBundle) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"warning-analysis-digest-v1\n");
    hasher.update(format!(
        "warning|{}|{}|{}|{}|{}|{}\n",
        warning.id,
        warning.warning_type,
        warning.severity,
        warning.title,
        warning.description.as_deref().unwrap_or(""),
        warning
            .updated_at
            .or(warning.created_at)
            .map(|ts| ts.timestamp())
            .unwrap_or_default(),
    ));
    for url in warning.source_urls.as_deref().unwrap_or_default() {
        hasher.update(format!("source|{url}\n"));
    }
    for observation in &bundle.observations {
        hasher.update(format!(
            "observation|{}|{}|{}\n",
            observation.id,
            observation.ts_utc.timestamp(),
            observation.observation_type
        ));
    }
    for insight in &bundle.insights {
        hasher.update(format!(
            "insight|{}|{}|{}\n",
            insight.id,
            insight
                .updated_at
                .or(insight.created_at)
                .map(|ts| ts.timestamp())
                .unwrap_or_default(),
            insight.insight_type.as_deref().unwrap_or("insight")
        ));
    }
    hex::encode(hasher.finalize())
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt construction
// ─────────────────────────────────────────────────────────────────────────────

fn single_line(text: &str, max_chars: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn observation_excerpt(observation: &ObservationRow) -> String {
    let text = observation
        .value
        .get("excerpt")
        .or_else(|| observation.value.get("text"))
        .or_else(|| observation.value.get("summary"))
        .or_else(|| observation.value.get("title"))
        .or_else(|| observation.value.get("content"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| observation.value.to_string());
    single_line(&text, MAX_OBSERVATION_EXCERPT_CHARS)
}

/// Render the evidence block: every sent observation with its full bounded
/// context (id, type, time, source, excerpt) and every sent insight. The header
/// states sent-vs-available so the model can calibrate coverage honestly.
fn evidence_block(bundle: &EvidenceBundle) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "OBSERVATIONS (direct evidence; showing {} of {} available):",
        bundle.observations.len(),
        bundle.observations_available
    ));
    for (index, observation) in bundle.observations.iter().enumerate() {
        let domain =
            observation_source_domain(observation).unwrap_or_else(|| "unknown".to_string());
        let url = observation_source_url(observation).unwrap_or_default();
        let confidence = observation
            .confidence
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "n/a".to_string());
        parts.push(format!(
            "[O{}] id={} type={} observed_at={} source_domain={} source_url={} confidence={}\n      {}",
            index + 1,
            observation.id,
            observation.observation_type,
            observation.ts_utc.to_rfc3339(),
            domain,
            url,
            confidence,
            observation_excerpt(observation),
        ));
    }
    if bundle.observations.is_empty() {
        parts.push("(none available)".to_string());
    }
    parts.push(format!(
        "RELATED INSIGHTS (derived intelligence; showing {} of {} available):",
        bundle.insights.len(),
        bundle.insights_available
    ));
    for (index, insight) in bundle.insights.iter().enumerate() {
        parts.push(format!(
            "[I{}] id={} type={} created_at={} title={} summary={}",
            index + 1,
            insight.id,
            insight.insight_type.as_deref().unwrap_or("insight"),
            insight
                .created_at
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string()),
            single_line(&insight.title, 200),
            single_line(&insight.summary, MAX_INSIGHT_SUMMARY_CHARS),
        ));
    }
    if bundle.insights.is_empty() {
        parts.push("(none available)".to_string());
    }
    if !bundle.source_domains.is_empty() {
        parts.push(format!(
            "KNOWN SOURCE DOMAINS (registrable domains across the evidence): {}",
            bundle.source_domains.join(", ")
        ));
    }
    parts.join("\n")
}

/// Build the system/user prompts. The domain persona comes from the
/// configurable [`IntelligenceProfile`]; the JSON contract is added by the
/// service and versioned via [`ANALYSIS_PROMPT_VERSION`].
pub fn build_prompts(
    profile: &IntelligenceProfile,
    warning: &WarningRow,
    bundle: &EvidenceBundle,
) -> (String, String) {
    let mut system = profile.system_prompt.trim().to_string();
    if !profile.focus_areas.is_empty() {
        system.push_str("\n\nDomain focus areas:\n");
        for area in &profile.focus_areas {
            system.push_str(&format!("- {}\n", area.trim()));
        }
    }
    system.push_str(
        "\nReturn exactly one JSON object and nothing else — no prose, no markdown, no code \
         fences, no fields beyond the schema.\n\
         Schema:\n\
         {\n\
           \"claims\":    [{\"text\": string, \"claim_kind\": \"observed\"|\"inference\"|\"recommendation\", \"evidence_ids\": [uuid], \"confidence\": number}],\n\
           \"impact\":    [same claim shape],\n\
           \"actions\":   [same claim shape],\n\
           \"limitations\": [string]\n\
         }\n\
         Rules:\n\
         - A claim labelled \"observed\" or \"inference\" MUST cite at least one evidence_id taken \
           verbatim from the OBSERVATIONS/RELATED INSIGHTS lists.\n\
         - Never invent evidence_ids: every id must appear in the evidence lists.\n\
         - \"recommendation\" claims (suggested actions) may cite zero evidence_ids but must be \
           labelled \"recommendation\".\n\
         - confidence is 0.0–1.0; omit it rather than guessing.\n\
         - If the evidence is insufficient for a claim, put the gap in limitations instead of \
           asserting it.",
    );

    let entity_names_str = if bundle.entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        bundle.entity_names.join(", ")
    };
    let region = warning.region.as_deref().unwrap_or("Global");
    let confidence_pct = (warning.confidence.unwrap_or(0.0) * 100.0).round();
    let user = format!(
        "Analyse this warning and return the JSON object.\n\n\
         WARNING: {title} ({severity} severity)\n\
         Type: {warning_type} | Region: {region} | Pipeline confidence: {confidence:.0}%\n\
         Entities: {entities}\n\
         Description: {description}\n\n\
         {evidence}\n\n\
         Produce 3-{max_claims} claims grounded in the evidence above, up to {max_impact} impact \
         items, up to {max_actions} recommended actions, and up to {max_limitations} limitations. \
         Prefer specific facts (companies, products, dates) exactly as they appear in the evidence.",
        title = warning.title,
        severity = warning.severity,
        warning_type = warning.warning_type,
        region = region,
        confidence = confidence_pct,
        entities = entity_names_str,
        description = warning.description.as_deref().unwrap_or("(none)"),
        evidence = evidence_block(bundle),
        max_claims = MAX_CLAIMS,
        max_impact = MAX_IMPACT_ITEMS,
        max_actions = MAX_ACTIONS,
        max_limitations = MAX_LIMITATIONS,
    );
    (system, user)
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendered output (persisted in the run and served to every surface)
// ─────────────────────────────────────────────────────────────────────────────

/// One evidence citation, resolved to a human-readable label so consumers do
/// not have to re-derive source text from generated prose.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderedEvidence {
    pub evidence_id: Uuid,
    pub evidence_kind: String,
    pub label: String,
}

/// One claim in the rendered analysis panel, with its real per-claim evidence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderedClaim {
    pub text: String,
    pub claim_kind: String,
    pub section: String,
    pub confidence: Option<f64>,
    pub evidence: Vec<RenderedEvidence>,
}

/// The rendered, persisted result of one analysis run. Counts are counts;
/// quality is measured by the reusable [`EvidenceQuality`] model; there are no
/// count-derived `source_reliability` / `data_sufficiency` labels.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WarningAnalysisOutput {
    pub schema_version: u32,
    pub claims: Vec<RenderedClaim>,
    pub impact: Vec<RenderedClaim>,
    pub actions: Vec<RenderedClaim>,
    pub limitations: Vec<String>,
    pub evidence_quality: EvidenceQuality,
    /// Number of source URLs attached to the warning itself.
    pub warning_source_count: usize,
    pub source_domains: Vec<String>,
    /// Observations after dedup before the cap (the real corpus size).
    pub observation_count_available: usize,
    /// Observations actually sent to the model.
    pub observation_count_sent: usize,
    pub insight_count_available: usize,
    pub insight_count_sent: usize,
    pub entity_count: usize,
    pub entity_names: Vec<String>,
    pub warning_confidence: f64,
    pub model: String,
    pub prompt_version: String,
    pub evidence_digest: String,
}

fn render_evidence(bundle: &EvidenceBundle, reference: &EvidenceRef) -> RenderedEvidence {
    match reference {
        EvidenceRef::Observation(id) => {
            let label = bundle
                .observations
                .iter()
                .find(|observation| observation.id == *id)
                .map(|observation| {
                    let domain = observation_source_domain(observation)
                        .unwrap_or_else(|| "unknown".to_string());
                    format!(
                        "{} observation from {} @ {}",
                        observation.observation_type,
                        domain,
                        observation.ts_utc.to_rfc3339()
                    )
                })
                .unwrap_or_else(|| "observation no longer in the evidence set".to_string());
            RenderedEvidence {
                evidence_id: *id,
                evidence_kind: "observation".to_string(),
                label: single_line(&label, 200),
            }
        }
        EvidenceRef::Insight(id) => {
            let label = bundle
                .insights
                .iter()
                .find(|insight| insight.id == *id)
                .map(|insight| format!("related insight: {}", single_line(&insight.title, 160)))
                .unwrap_or_else(|| "related insight no longer in the evidence set".to_string());
            RenderedEvidence {
                evidence_id: *id,
                evidence_kind: "insight".to_string(),
                label,
            }
        }
    }
}

fn render_claim(bundle: &EvidenceBundle, claim: &AnalysisClaimRecord) -> RenderedClaim {
    RenderedClaim {
        text: claim.claim.clone(),
        claim_kind: claim.kind.as_str().to_string(),
        section: claim.section.as_str().to_string(),
        confidence: claim.confidence,
        evidence: claim
            .evidence
            .iter()
            .map(|reference| render_evidence(bundle, reference))
            .collect(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Enqueue + asynchronous execution
// ─────────────────────────────────────────────────────────────────────────────

/// Everything the executor needs for one run.
#[derive(Clone)]
pub struct AnalysisRunContext {
    pub run_id: Uuid,
    pub warning: WarningRow,
    pub bundle: EvidenceBundle,
    pub model: ModelConfig,
    pub profile: IntelligenceProfile,
}

/// Enqueue an analysis run for a warning (deduplicating identical evidence)
/// and return the run row plus the context an executor needs if it is queued.
pub async fn enqueue_analysis(
    store: &PgStore,
    model: &WarningAnalysisModel,
    warning: &WarningRow,
    requested_by: Option<&str>,
) -> Result<(WarningAnalysisRunRow, bool, AnalysisRunContext)> {
    // A crashed process must not leave a run looking "in progress" forever.
    store
        .expire_stale_warning_analysis_runs(STALE_RUN_SECONDS)
        .await
        .context("warning analysis: failed to expire stale runs")?;

    let bundle = gather_evidence(store, warning).await?;
    let digest = evidence_digest(warning, &bundle);

    let (run, deduplicated) = store
        .enqueue_warning_analysis_run(&NewWarningAnalysisRun {
            warning_id: warning.id,
            requested_by: requested_by.map(str::to_string),
            model: model.primary.model_name.clone(),
            prompt_version: ANALYSIS_PROMPT_VERSION.to_string(),
            evidence_digest: digest.clone(),
            observations_available: bundle.observations_available as i32,
            observations_sent: bundle.observations.len() as i32,
            insights_available: bundle.insights_available as i32,
            insights_sent: bundle.insights.len() as i32,
        })
        .await
        .context("warning analysis: failed to enqueue run")?;

    let context = AnalysisRunContext {
        run_id: run.id,
        warning: warning.clone(),
        bundle,
        model: model.primary.clone(),
        profile: model.profile.clone(),
    };
    Ok((run, deduplicated, context))
}

/// Spawn the executor for a queued run. The request returns immediately; the
/// run's status/output are persisted so the panel and API can poll them.
pub fn spawn_analysis_executor(store: Arc<PgStore>, context: AnalysisRunContext) {
    tokio::spawn(async move {
        if let Err(error) = execute_analysis_run(&store, &context).await {
            tracing::error!(
                run_id = %context.run_id,
                error = %format!("{error:#}"),
                "warning analysis executor failed"
            );
        }
    });
}

/// Run the LLM outside the request and persist the validated result. Any
/// failure marks the run failed with an explicit reason — never a silent
/// success.
pub async fn execute_analysis_run(store: &PgStore, context: &AnalysisRunContext) -> Result<()> {
    let claimed = store
        .start_warning_analysis_run(context.run_id)
        .await
        .context("warning analysis: failed to claim run")?;
    if !claimed {
        // Another executor owns it, or it is already terminal.
        return Ok(());
    }

    match run_analysis(context).await {
        Ok(finished) => {
            let output_json = serde_json::to_value(&finished.output)
                .context("warning analysis: failed to serialize output")?;
            store
                .complete_warning_analysis_run(context.run_id, &output_json, &finished.records)
                .await
                .context("warning analysis: failed to persist result")?;
            tracing::info!(
                run_id = %context.run_id,
                warning_id = %context.warning.id,
                claims = finished.output.claims.len(),
                "warning analysis completed"
            );
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::error!(
                run_id = %context.run_id,
                warning_id = %context.warning.id,
                error = %message,
                "warning analysis run failed"
            );
            store
                .fail_warning_analysis_run(context.run_id, &message)
                .await
                .context("warning analysis: failed to record failure")?;
            Ok(())
        }
    }
}

struct FinishedAnalysis {
    output: WarningAnalysisOutput,
    records: Vec<AnalysisClaimRecord>,
}

async fn run_analysis(context: &AnalysisRunContext) -> Result<FinishedAnalysis> {
    let (system_prompt, user_prompt) =
        build_prompts(&context.profile, &context.warning, &context.bundle);

    let mut model_config = context.model.clone();
    // Deterministic-ish structured extraction. No timeout override: the run is
    // asynchronous, so the configured model timeout applies as-is.
    model_config.temperature = 0.2;
    model_config.max_tokens = 2048;
    let client = OpenAiCompatibleClient::new(model_config);

    let raw = client
        .generate_json(&system_prompt, &user_prompt)
        .await
        .context("warning analysis: LLM call failed")?;

    let payload = parse_analysis_payload(&raw)?;
    let validated = validate_analysis_payload(
        payload,
        &context.bundle.observation_ids(),
        &context.bundle.insight_ids(),
    )?;

    let quality = assess_bundle_quality(&context.bundle, Utc::now());
    let output = WarningAnalysisOutput {
        schema_version: ANALYSIS_OUTPUT_SCHEMA_VERSION,
        claims: validated
            .claims
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        impact: validated
            .impact
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        actions: validated
            .actions
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        limitations: validated.limitations.clone(),
        evidence_quality: quality,
        warning_source_count: context
            .warning
            .source_urls
            .as_deref()
            .unwrap_or_default()
            .len(),
        source_domains: context.bundle.source_domains.clone(),
        observation_count_available: context.bundle.observations_available,
        observation_count_sent: context.bundle.observations.len(),
        insight_count_available: context.bundle.insights_available,
        insight_count_sent: context.bundle.insights.len(),
        entity_count: context
            .warning
            .entity_ids
            .as_deref()
            .unwrap_or_default()
            .len(),
        entity_names: context.bundle.entity_names.clone(),
        warning_confidence: context.warning.confidence.unwrap_or(0.0),
        model: context.model.model_name.clone(),
        prompt_version: ANALYSIS_PROMPT_VERSION.to_string(),
        evidence_digest: evidence_digest(&context.warning, &context.bundle),
    };

    Ok(FinishedAnalysis {
        output,
        records: validated.records(),
    })
}

/// Parse a persisted run output back into the rendered output type (used by
/// the panel and JSON status endpoint to avoid trusting untyped JSON).
pub fn output_from_run(run: &WarningAnalysisRunRow) -> Result<Option<WarningAnalysisOutput>> {
    match run.output.as_ref() {
        Some(value) => Ok(Some(
            serde_json::from_value(value.clone())
                .context("stored analysis output does not match the current schema")?,
        )),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation_ids() -> HashSet<Uuid> {
        [Uuid::nil()].into_iter().collect()
    }

    fn insight_ids() -> HashSet<Uuid> {
        [Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap()]
            .into_iter()
            .collect()
    }

    fn valid_payload_json(evidence_id: Uuid) -> String {
        json!({
            "claims": [
                {"text": "Plant X opened in March", "claim_kind": "observed", "evidence_ids": [evidence_id.to_string()], "confidence": 0.9},
                {"text": "Capacity may tighten", "claim_kind": "inference", "evidence_ids": [evidence_id.to_string()], "confidence": 0.6}
            ],
            "impact": [
                {"text": "Lead times could extend", "claim_kind": "inference", "evidence_ids": [evidence_id.to_string()]}
            ],
            "actions": [
                {"text": "Qualify a second supplier", "claim_kind": "recommendation"}
            ],
            "limitations": ["Only one source domain was available."]
        })
        .to_string()
    }

    fn validate(json: &str) -> Result<ValidatedAnalysis> {
        let payload = parse_analysis_payload(json)?;
        validate_analysis_payload(payload, &observation_ids(), &insight_ids())
    }

    #[test]
    fn strict_parse_rejects_prose_around_the_json_object() {
        let id = Uuid::nil();
        let raw = format!("Here is the analysis:\n{}", valid_payload_json(id));
        let error = parse_analysis_payload(&raw).expect_err("prose must be rejected");
        assert!(error.to_string().contains("typed JSON"), "{error}");
    }

    #[test]
    fn strict_parse_rejects_markdown_fences() {
        let id = Uuid::nil();
        let raw = format!("```json\n{}\n```", valid_payload_json(id));
        assert!(parse_analysis_payload(&raw).is_err());
    }

    #[test]
    fn strict_parse_rejects_unknown_fields() {
        let id = Uuid::nil();
        let raw = json!({
            "claims": [{"text": "x", "claim_kind": "observed", "evidence_ids": [id.to_string()], "narrative": "extra prose"}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        assert!(parse_analysis_payload(&raw).is_err());
    }

    #[test]
    fn unknown_evidence_id_is_rejected() {
        let raw = valid_payload_json(Uuid::new_v4());
        let error = validate(&raw).expect_err("unknown evidence id must be rejected");
        assert!(error.to_string().contains("unknown evidence id"), "{error}");
    }

    #[test]
    fn observed_claim_without_evidence_is_rejected() {
        let raw = json!({
            "claims": [{"text": "Plant opened", "claim_kind": "observed", "evidence_ids": []}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        let error = validate(&raw).expect_err("observed claim without evidence must be rejected");
        assert!(
            format!("{error:#}").contains("cites no evidence"),
            "{error:#}"
        );
    }

    #[test]
    fn recommendations_may_omit_evidence_but_stay_marked() {
        let validated = validate(&valid_payload_json(Uuid::nil())).expect("valid payload");
        let actions = validated.actions.first().expect("action persisted");
        assert_eq!(actions.kind, ClaimKind::Recommendation);
        assert!(actions.evidence.is_empty());
        assert_eq!(actions.section, ClaimSection::Action);
        assert!(validated
            .records()
            .iter()
            .any(|record| record.kind == ClaimKind::Recommendation));
    }

    #[test]
    fn citations_may_resolve_to_related_insights() {
        let insight = Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap();
        let raw = json!({
            "claims": [{"text": "Cross-referenced conclusion", "claim_kind": "inference", "evidence_ids": [insight.to_string()]}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        let validated = validate(&raw).expect("insight citation resolves");
        assert_eq!(
            validated.claims[0].evidence,
            vec![EvidenceRef::Insight(insight)]
        );
    }

    #[test]
    fn confidence_outside_unit_interval_is_rejected() {
        let id = Uuid::nil();
        let raw = json!({
            "claims": [{"text": "Too sure", "claim_kind": "observed", "evidence_ids": [id.to_string()], "confidence": 1.5}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        assert!(validate(&raw).is_err());
    }

    #[test]
    fn empty_claims_are_rejected() {
        let raw = json!({"claims": [], "impact": [], "actions": [], "limitations": []}).to_string();
        assert!(validate(&raw).is_err());
    }

    #[test]
    fn section_caps_are_enforced() {
        let id = Uuid::nil();
        let claims: Vec<serde_json::Value> = (0..MAX_CLAIMS + 1)
            .map(|index| {
                json!({"text": format!("claim {index}"), "claim_kind": "observed", "evidence_ids": [id.to_string()]})
            })
            .collect();
        let raw =
            json!({"claims": claims, "impact": [], "actions": [], "limitations": []}).to_string();
        assert!(validate(&raw).is_err());
    }

    #[test]
    fn digest_is_stable_and_evidence_sensitive() {
        let warning = test_warning();
        let observation_id = Uuid::new_v4();
        let bundle = test_bundle(vec![test_observation(observation_id)], 1);
        let first = evidence_digest(&warning, &bundle);
        let second = evidence_digest(&warning, &bundle);
        assert_eq!(first, second, "same evidence must produce the same digest");

        let changed = test_bundle(vec![test_observation(Uuid::new_v4())], 1);
        assert_ne!(
            first,
            evidence_digest(&warning, &changed),
            "different evidence must produce a different digest"
        );
    }

    #[test]
    fn prompt_states_sent_vs_available_evidence_counts() {
        let profile = IntelligenceProfile {
            name: "Test".to_string(),
            system_prompt: "You are a test analyst.".to_string(),
            focus_areas: vec!["supply chains".to_string()],
        };
        let bundle = test_bundle(
            vec![
                test_observation(Uuid::new_v4()),
                test_observation(Uuid::new_v4()),
            ],
            3,
        );
        let (system, user) = build_prompts(&profile, &test_warning(), &bundle);
        assert!(system.contains("You are a test analyst."));
        assert!(system.contains("supply chains"));
        assert!(
            user.contains("showing 2 of 3 available"),
            "prompt must state sent vs available evidence: {user}"
        );
        assert!(system.contains("\"recommendation\""));
    }

    fn test_warning() -> WarningRow {
        WarningRow {
            id: Uuid::new_v4(),
            recipe_code: None,
            warning_type: "supply_chain".to_string(),
            title: "Component shortage".to_string(),
            description: Some("Lead times extend".to_string()),
            severity: "high".to_string(),
            region: Some("US".to_string()),
            source_urls: Some(vec!["https://example.com/a".to_string()]),
            entity_ids: Some(vec![Uuid::new_v4()]),
            confidence: Some(0.8),
            impact: None,
            actions: None,
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

    fn test_observation(id: Uuid) -> ObservationRow {
        ObservationRow {
            id,
            observation_type: "signal".to_string(),
            entity_id: None,
            entity_type: None,
            ts_utc: Utc::now(),
            value: json!({"text": "A real observation"}),
            provenance: json!({"source_url": "https://example.com/a"}),
            confidence: Some(0.7),
            created_at: Some(Utc::now()),
        }
    }

    fn test_bundle(observations: Vec<ObservationRow>, available: usize) -> EvidenceBundle {
        EvidenceBundle {
            observations,
            insights: Vec::new(),
            observations_available: available,
            insights_available: 0,
            entity_names: Vec::new(),
            source_domains: vec!["example.com".to_string()],
            source_reliability: Vec::new(),
        }
    }
}
