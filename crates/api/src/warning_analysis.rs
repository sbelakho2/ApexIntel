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
//!
//! Warning evidence is first-class (migrations 083 + 085, audit P0-5/P0-6): a
//! warning's source URLs are resolved at creation only against real, fetched
//! objects (`source document -> observation -> warning_evidence`); a URL with
//! no fetched source stays an explicit `unresolved` link and contributes no
//! evidence. The analysis consumes the linked observation ids as its direct
//! evidence set. Entity-derived observations are only a fallback for warnings
//! that carry no explicit links. Evidence identity uses real row identity or a
//! canonical content hash plus origin cluster (never a 120-character prefix),
//! and an empty evidence set is deterministically reported as
//! [`AnalysisStatus::InsufficientEvidence`] without calling the model.

#![cfg(feature = "llm")]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use apex_core::analysis::registrable_domain;
use apex_core::claims::{AnalysisClaimRecord, ClaimKind, ClaimSection, EvidenceRef};
use apex_core::evidence_quality::{
    assess_evidence_quality_for_claim, EvidenceItem, EvidenceQuality, EvidenceStance,
};
use apex_core::intelligence_profile::IntelligenceProfile;
use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
use apex_store::postgres::{
    NewWarningAnalysisRun, ObservationRow, PgStore, WarningAnalysisRunRow, WarningRow,
};

/// Version of the prompt/JSON contract. Bump when the contract changes so new
/// generations are comparable against (not merged with) old ones.
pub const ANALYSIS_PROMPT_VERSION: &str = "warning-analysis-v3";

/// Output schema version stored with each run.
pub const ANALYSIS_OUTPUT_SCHEMA_VERSION: u32 = 3;

/// Documented bounded-evidence caps. The prompt states how many items were
/// actually sent versus how many were available, and the run row persists both
/// counts so a result can never imply it saw more evidence than it did.
pub const MAX_EVIDENCE_OBSERVATIONS: usize = 60;
pub const MAX_EVIDENCE_INSIGHTS: usize = 10;
pub const MAX_OBSERVATION_EXCERPT_CHARS: usize = 600;
pub const MAX_INSIGHT_SUMMARY_CHARS: usize = 400;

/// Deterministic evidence-adequacy policy (claim-level).
///
/// * An `observed` claim must cite at least one **observation** (a direct
///   item); a derived insight cannot make a claim observed.
/// * An `inference` claim citing a single item must carry an explicit
///   confidence at or below [`INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP`]: the
///   platform may not state a strong inference from one uncorroborated item.
/// * An `inference` claim at or above
///   [`HIGH_CONFIDENCE_INFERENCE_THRESHOLD`] must cite at least
///   [`HIGH_CONFIDENCE_MIN_ORIGINS`] independent origins (distinct
///   registrable domains).
pub const INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP: f64 = 0.6;
pub const HIGH_CONFIDENCE_INFERENCE_THRESHOLD: f64 = 0.8;
pub const HIGH_CONFIDENCE_MIN_ORIGINS: usize = 2;

/// Bounds on the model's structured output.
pub const MAX_CLAIMS: usize = 12;
pub const MAX_IMPACT_ITEMS: usize = 8;
pub const MAX_ACTIONS: usize = 8;
pub const MAX_LIMITATIONS: usize = 8;
pub const MAX_CLAIM_TEXT_CHARS: usize = 1200;
pub const MAX_LIMITATION_CHARS: usize = 500;

/// Total prompt budget (system prompt + user prompt, characters) for one
/// analysis request. The evidence block is truncated to fit; the shipped
/// llama-server unit runs with `--ctx-size 16384` (~4 chars/token), and the
/// configured output cap is 2048 tokens, so ~20k input characters (~5k
/// tokens) stays well inside the window. Override with
/// `APEX_ANALYSIS_MAX_PROMPT_CHARS` when running a different context size.
pub const DEFAULT_MAX_PROMPT_CHARS: usize = 20_000;
pub const MAX_PROMPT_CHARS_ENV_VAR: &str = "APEX_ANALYSIS_MAX_PROMPT_CHARS";

/// The OpenAI-compatible client makes one attempt plus three retries.
const MODEL_MAX_ATTEMPTS: i64 = 4;

/// Fallback stale threshold when the model timeout is unknown.
pub const DEFAULT_STALE_RUN_SECONDS: i64 = 1800;

/// Stale threshold derived from the model timeout: a healthy run may take
/// `attempts × timeout` plus backoff, so the threshold must clear that.
pub fn stale_run_seconds(model_timeout_secs: u32) -> i64 {
    (model_timeout_secs as i64 * MODEL_MAX_ATTEMPTS + 300).max(DEFAULT_STALE_RUN_SECONDS)
}

/// Deterministic status of an analysis run.
///
/// [`AnalysisStatus::InsufficientEvidence`] is decided by the preflight before
/// any model call: when the warning has neither observations nor insights, the
/// run reports the gap instead of asking a model to write claims it cannot
/// ground. It is a completed, persisted outcome (cached like any other result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    #[default]
    Completed,
    InsufficientEvidence,
}

impl AnalysisStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }

    pub fn is_insufficient(self) -> bool {
        matches!(self, Self::InsufficientEvidence)
    }
}

/// Where the analysis's direct evidence came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    /// Explicit resolved `warning_evidence` observation links: the warning is
    /// grounded in real extracted observations.
    WarningObservations,
    /// Only resolved source-document links exist (fetched content but no
    /// extracted observation yet). Document-level evidence; not a substitute
    /// for direct observation.
    WarningDocuments,
    /// The warning has explicit `warning_evidence` links, but none are
    /// resolved — the stated source has not been acquired. Entity-wide
    /// observations are deliberately *not* substituted here: the warning's own
    /// source is unresolved, so the analysis must report insufficient
    /// evidence rather than ground itself in unrelated entity activity.
    WarningEvidenceUnresolved,
    /// Legacy fallback: no `warning_evidence` rows exist at all, so the
    /// warning's entity observations are the direct evidence.
    EntityFallback,
    /// No explicit links and no entity observations exist.
    #[default]
    None,
}

impl EvidenceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WarningObservations => "warning_observations",
            Self::WarningDocuments => "warning_documents",
            Self::WarningEvidenceUnresolved => "warning_evidence_unresolved",
            Self::EntityFallback => "entity_fallback",
            Self::None => "none",
        }
    }

    /// True when the scope grounds the warning in its own stated source rather
    /// than unrelated entity activity.
    pub fn is_explicit(self) -> bool {
        matches!(
            self,
            Self::WarningObservations | Self::WarningDocuments | Self::WarningEvidenceUnresolved
        )
    }

    /// True when direct observations are available for citing.
    pub fn has_direct_observations(self) -> bool {
        matches!(self, Self::WarningObservations | Self::EntityFallback)
    }
}

/// Deterministic preflight: an analysis with no observations and no insights
/// has nothing the model could cite, so it is reported as
/// [`AnalysisStatus::InsufficientEvidence`] without calling the model.
///
/// The check is over the evidence set that would be sent: `available` counts
/// can only be zero when nothing was sent, and a prompt budget that dropped
/// everything leaves the model equally unable to ground a claim.
pub fn preflight_status(bundle: &EvidenceBundle) -> AnalysisStatus {
    // The warning names an explicit source that has not been acquired. Related
    // derived insights cannot replace the missing primary evidence, so this is
    // deterministically insufficient rather than a model call.
    if bundle.evidence_scope == EvidenceScope::WarningEvidenceUnresolved {
        return AnalysisStatus::InsufficientEvidence;
    }
    if bundle.observations.is_empty() && bundle.insights.is_empty() {
        AnalysisStatus::InsufficientEvidence
    } else {
        AnalysisStatus::Completed
    }
}

/// Best-effort stale-run housekeeping shared by enqueue and both status
/// surfaces: a crash must not leave a run "in progress" forever, but a
/// housekeeping failure must never block enqueueing or reporting.
pub async fn expire_stale_runs(store: &PgStore, stale_seconds: i64) {
    if let Err(error) = store
        .expire_stale_warning_analysis_runs(stale_seconds)
        .await
    {
        tracing::warn!(
            %error,
            "warning analysis: failed to expire stale runs; run statuses unchanged"
        );
    }
}

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

/// Everything the validator needs to know about the evidence the model saw:
/// which ids resolve, and which origin each observation belongs to.
#[derive(Debug, Clone, Default)]
pub struct EvidenceIndex {
    pub observation_ids: HashSet<Uuid>,
    pub insight_ids: HashSet<Uuid>,
    /// Canonical origin record per observation, clustered through
    /// [`apex_core::origin_cluster`]. `None` means the observation has no
    /// resolvable origin (it cannot establish independence).
    pub observation_origins: HashMap<Uuid, Option<String>>,
}

impl EvidenceIndex {
    pub fn from_bundle(bundle: &EvidenceBundle) -> Self {
        let observation_ids = bundle.observation_ids();
        let insight_ids = bundle.insight_ids();
        let mut observation_origins = HashMap::with_capacity(bundle.observations.len());
        for observation in &bundle.observations {
            observation_origins.insert(observation.id, observation_source_domain(observation));
        }
        Self {
            observation_ids,
            insight_ids,
            observation_origins,
        }
    }

    /// Distinct independent origins among the cited observations, through the
    /// canonical [`apex_core::origin_cluster`] service (registrable-domain
    /// fallback, content-hash/publisher/syndication signals). Derived
    /// insights do not contribute origins.
    ///
    /// This is the same definition of "independent evidence" used by corpus
    /// quality, so the validator can never accept a claim as independently
    /// corroborated while the corpus model counts it as one origin.
    fn independent_origin_count(&self, evidence: &[EvidenceRef]) -> usize {
        let records: Vec<apex_core::origin_cluster::OriginRecord> = evidence
            .iter()
            .filter_map(|reference| match reference {
                EvidenceRef::Observation(id) => Some(id),
                EvidenceRef::Insight(_) => None,
            })
            .map(|id| apex_core::origin_cluster::OriginRecord {
                origin: self.observation_origins.get(id).cloned().flatten(),
                content_hash: None,
                canonical_publisher: None,
                syndication_of: None,
                title: None,
                body: None,
                observed_at: None,
            })
            .collect();
        apex_core::origin_cluster::independent_origin_count(&records)
    }
}

/// Validate a parsed payload against the evidence the model was given.
///
/// * Every cited evidence id must resolve to a real observation or insight.
/// * `observed` claims must cite at least one observation (a direct item);
///   a derived insight cannot make a claim observed.
/// * `inference` claims must cite at least two items, or exactly one with an
///   explicit confidence at or below
///   [`INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP`]. At or above
///   [`HIGH_CONFIDENCE_INFERENCE_THRESHOLD`] they must cite at least
///   [`HIGH_CONFIDENCE_MIN_ORIGINS`] independent origins.
/// * `recommendation` claims may cite none but stay marked as recommendations.
/// * Confidence values must be finite and within `0.0..=1.0`.
/// * Output size is bounded per section.
pub fn validate_analysis_payload(
    payload: AnalysisPayload,
    index: &EvidenceIndex,
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

    let claims = validate_section(payload.claims, ClaimSection::Claim, index)?;
    let impact = validate_section(payload.impact, ClaimSection::Impact, index)?;
    let actions = validate_section(payload.actions, ClaimSection::Action, index)?;

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
    index: &EvidenceIndex,
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
        let mut direct_items = 0usize;
        for id in claim.evidence_ids {
            if index.observation_ids.contains(&id) {
                direct_items += 1;
                evidence.push(EvidenceRef::Observation(id));
            } else if index.insight_ids.contains(&id) {
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

        // Claim-kind policy beyond "has evidence": direct items for observed
        // claims, and confidence/origin requirements for inference.
        match claim.claim_kind {
            PayloadClaimKind::Observed => {
                if direct_items == 0 {
                    anyhow::bail!(
                        "analysis observed claim '{}' cites no evidence resolving to an observation; \
                         observed claims must cite at least one direct observation",
                        text
                    );
                }
            }
            PayloadClaimKind::Inference => {
                if evidence.len() < 2 {
                    let capped = claim.confidence.is_some_and(|confidence| {
                        confidence <= INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP
                    });
                    if !capped {
                        anyhow::bail!(
                            "analysis inference claim '{}' cites a single evidence item and no \
                             explicit confidence <= {INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP}; a \
                             single-item inference must cap its confidence",
                            text
                        );
                    }
                }
                if claim
                    .confidence
                    .is_some_and(|confidence| confidence >= HIGH_CONFIDENCE_INFERENCE_THRESHOLD)
                {
                    let origins = index.independent_origin_count(&evidence);
                    if origins < HIGH_CONFIDENCE_MIN_ORIGINS {
                        anyhow::bail!(
                            "analysis high-confidence inference claim '{}' cites {origins} \
                             independent origin(s); confidence >= {HIGH_CONFIDENCE_INFERENCE_THRESHOLD} \
                             requires {HIGH_CONFIDENCE_MIN_ORIGINS}",
                            text
                        );
                    }
                }
            }
            PayloadClaimKind::Recommendation => {}
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
    /// `(domain, tier)` from `source_reliability_stats`.
    pub source_reliability: Vec<(String, String)>,
    /// Total explicit `warning_evidence` links found for the warning
    /// (resolved observations, resolved documents and unresolved URLs).
    pub warning_evidence_count: usize,
    /// Links resolved to a real extracted observation.
    pub resolved_observation_links: usize,
    /// Links resolved to a fetched source document without an extracted
    /// observation.
    pub resolved_document_links: usize,
    /// Links whose URL has no fetched source yet.
    pub unresolved_links: usize,
    /// Where the direct evidence came from.
    pub evidence_scope: EvidenceScope,
}

impl EvidenceBundle {
    pub fn observation_ids(&self) -> HashSet<Uuid> {
        self.observations.iter().map(|row| row.id).collect()
    }

    pub fn insight_ids(&self) -> HashSet<Uuid> {
        self.insights.iter().map(|row| row.id).collect()
    }

    /// Everything the validator needs: resolvable ids plus per-observation
    /// origins for the independence rules.
    pub fn evidence_index(&self) -> EvidenceIndex {
        EvidenceIndex::from_bundle(self)
    }
}

/// Canonical JSON rendering: object keys sorted recursively so logically
/// identical content hashes identically regardless of serialization order.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        serde_json::Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", rendered.join(","))
        }
        other => other.to_string(),
    }
}

/// Canonical content hash of an observation: sha256 over the canonicalized
/// `(observation_type, value)` payload (sorted keys, stable across
/// serialization order).
pub fn canonical_observation_content_hash(observation: &ObservationRow) -> String {
    let canonical = canonical_json(&serde_json::json!({
        "observation_type": observation.observation_type,
        "value": observation.value,
    }));
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// The origin cluster an observation belongs to: its registrable source
/// domain when resolvable. `None` means the origin cannot be established, so
/// the observation keeps its real row identity instead of being suppressed.
pub fn observation_origin_cluster(observation: &ObservationRow) -> Option<String> {
    observation_source_domain(observation)
}

/// Real dedup identity for one observation.
///
/// * When an origin is resolvable, the identity is the canonical content hash
///   plus the origin cluster: byte-identical content from the same origin is
///   the same signal (semantic suppression), while content that only shares a
///   long prefix is *not* collapsed.
/// * Without a resolvable origin the identity is the real row id: origin
///   cannot be established, so no semantic suppression is allowed.
pub fn observation_dedup_identity(observation: &ObservationRow) -> String {
    match observation_origin_cluster(observation) {
        Some(origin) => format!(
            "content:{}:{origin}",
            canonical_observation_content_hash(observation)
        ),
        None => format!("id:{}", observation.id),
    }
}

/// Deduplicate observations by real identity (id, or canonical content hash +
/// origin cluster) instead of a truncated JSON prefix. Newest-first order is
/// preserved.
pub fn dedup_observations(observations: Vec<ObservationRow>) -> Vec<ObservationRow> {
    let mut seen = HashSet::new();
    observations
        .into_iter()
        .filter(|observation| seen.insert(observation_dedup_identity(observation)))
        .collect()
}

/// Gather the full bounded evidence set: up to
/// [`MAX_EVIDENCE_OBSERVATIONS`] deduplicated observations and
/// [`MAX_EVIDENCE_INSIGHTS`] related insights, newest first.
///
/// Direct evidence comes from the warning's explicit `warning_evidence` links
/// (migrations 083 + 085). Entity observations are only a fallback for warnings
/// with no explicit links, so a warning with a real fetched source and no
/// entity ids still has citable evidence.
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

    // Every persisted link, resolved or not. The *presence* of explicit links
    // (even unresolved ones) is what decides whether unrelated entity
    // observations may be used: a warning whose stated source is unresolved
    // must not be grounded in generic entity activity.
    let links = store
        .list_warning_evidence(warning.id)
        .await
        .context("warning analysis: failed to load warning evidence links")?;
    let linked_observations = store
        .list_warning_evidence_observations(warning.id)
        .await
        .context("warning analysis: failed to load linked warning observations")?;

    let warning_evidence_count = links.len();
    let resolved_observation_links = linked_observations.len();
    let resolved_document_links = links
        .iter()
        .filter(|link| link.status == "resolved" && link.evidence_kind == "source_document")
        .count();
    let unresolved_links = links
        .iter()
        .filter(|link| link.status != "resolved")
        .count();

    let (mut all_observations, evidence_scope) = if !linked_observations.is_empty() {
        (linked_observations, EvidenceScope::WarningObservations)
    } else if warning_evidence_count > 0 {
        // Explicit links exist but none resolved to an observation. Never
        // substitute unrelated entity observations here.
        let scope = if resolved_document_links > 0 {
            EvidenceScope::WarningDocuments
        } else {
            EvidenceScope::WarningEvidenceUnresolved
        };
        (Vec::new(), scope)
    } else {
        let mut observations = Vec::new();
        for entity_id in &entity_ids {
            let entity_observations = store
                .get_observations_by_entity(*entity_id, 60)
                .await
                .with_context(|| {
                    format!("warning analysis: failed to load observations for entity {entity_id}")
                })?;
            observations.extend(entity_observations);
        }
        let scope = if entity_ids.is_empty() {
            EvidenceScope::None
        } else {
            EvidenceScope::EntityFallback
        };
        (observations, scope)
    };
    all_observations.sort_by_key(|observation| std::cmp::Reverse(observation.ts_utc));
    let all_observations = dedup_observations(all_observations);
    let observations_available = all_observations.len();
    let mut all_observations = all_observations;
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
        warning_evidence_count,
        resolved_observation_links,
        resolved_document_links,
        unresolved_links,
        evidence_scope,
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

fn source_reliability_tier(bundle: &EvidenceBundle, domain: Option<&str>) -> Option<String> {
    let domain = domain?;
    bundle
        .source_reliability
        .iter()
        .find(|(known_domain, _)| known_domain == domain)
        .map(|(_, tier)| tier.clone())
}

/// Reusable evidence-quality assessment over the exact evidence set the model
/// saw, evaluated against the warning title as the claim.
///
/// No claim relation has been established for the gathered evidence: nothing
/// classifies an observation as supporting or contradicting the warning, so
/// every record enters as [`EvidenceStance::Neutral`]. Corpus dimensions
/// (count, origins, freshness, provenance, parser confidence, coverage) are
/// still measured exactly; claim corroboration reads `NotMeasured` until a
/// stance classifier exists. Marking evidence `Supports` by default would
/// fabricate semantic corroboration.
///
/// Derived insights are context, never factual support: they are marked
/// `Neutral` and `.derived()` so they cannot inflate independent corroboration
/// of the warning's own claim.
///
/// A missing observation/insight confidence stays missing: the record still
/// counts for corpus dimensions, but it is never assigned a synthesized 0.6
/// relevance weight.
pub fn assess_bundle_quality(
    bundle: &EvidenceBundle,
    claim: &str,
    now: DateTime<Utc>,
) -> EvidenceQuality {
    let mut items: Vec<EvidenceItem> = Vec::new();
    for observation in &bundle.observations {
        let domain = observation_source_domain(observation);
        let tier = source_reliability_tier(bundle, domain.as_deref());
        let mut item = EvidenceItem::new_optional(observation.confidence, EvidenceStance::Neutral)
            .with_source_type(observation.observation_type.clone())
            .with_observed_at(observation.ts_utc)
            .with_source_reliability(tier);
        if let Some(url) = observation_source_url(observation) {
            item = item.with_source_url(url);
        } else if let Some(domain) = domain {
            item = item.with_source_url(domain);
        }
        items.push(item);
    }
    for insight in &bundle.insights {
        let mut item = EvidenceItem::new_optional(insight.confidence, EvidenceStance::Neutral)
            .with_source_type(format!(
                "insight:{}",
                insight.insight_type.as_deref().unwrap_or("insight")
            ))
            .derived();
        if let Some(created_at) = insight.created_at {
            item = item.with_observed_at(created_at);
        }
        items.push(item);
    }
    assess_evidence_quality_for_claim(&items, &[], claim, now)
}

// ─────────────────────────────────────────────────────────────────────────────
// Evidence digest (dedupe/caching key)
// ─────────────────────────────────────────────────────────────────────────────

/// Stable digest of the exact evidence set and the model-visible warning
/// fields. Two requests with the same digest may reuse the same run; any
/// evidence or prompt-content change produces a different digest.
///
/// Deliberately excludes bookkeeping timestamps (`warnings.updated_at` is
/// bumped by acknowledge/review and by no-op merges), which do not change what
/// the model sees and must not defeat dedupe on repeated clicks.
pub fn evidence_digest(warning: &WarningRow, bundle: &EvidenceBundle) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"warning-analysis-digest-v3\n");
    hasher.update(format!(
        "warning|{}|{}|{}|{}|{}|{}|{:.6}\n",
        warning.id,
        warning.warning_type,
        warning.severity,
        warning.title,
        warning.description.as_deref().unwrap_or(""),
        warning.region.as_deref().unwrap_or(""),
        warning.confidence.unwrap_or(0.0),
    ));
    hasher.update(format!(
        "evidence-scope|{}|{}|{}\n",
        bundle.evidence_scope.as_str(),
        bundle.warning_evidence_count,
        bundle.observations_available,
    ));
    for url in warning.source_urls.as_deref().unwrap_or_default() {
        hasher.update(format!("source|{url}\n"));
    }
    let mut entity_ids: Vec<Uuid> = warning.entity_ids.clone().unwrap_or_default();
    entity_ids.sort();
    entity_ids.dedup();
    for entity_id in entity_ids {
        hasher.update(format!("entity|{entity_id}\n"));
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
        "OBSERVATIONS (direct evidence; source: {}; showing {} of {} available):",
        bundle.evidence_scope.as_str(),
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
    system.push_str(&format!(
        "\nReturn exactly one JSON object and nothing else — no prose, no markdown, no code \
         fences, no fields beyond the schema.\n\
         Schema:\n\
         {{\n\
           \"claims\":    [{{\"text\": string, \"claim_kind\": \"observed\"|\"inference\"|\"recommendation\", \"evidence_ids\": [uuid], \"confidence\": number}}],\n\
           \"impact\":    [same claim shape],\n\
           \"actions\":   [same claim shape],\n\
           \"limitations\": [string]\n\
         }}\n\
         Rules:\n\
         - A claim labelled \"observed\" MUST cite at least one observation id from the \
           OBSERVATIONS list; a derived RELATED INSIGHT cannot make a claim observed.\n\
         - A claim labelled \"inference\" MUST cite at least one evidence_id taken verbatim \
           from the evidence lists. With exactly one cited item it MUST carry an explicit \
           confidence of {single_item_cap} or lower; confidence >= {high_threshold} requires \
           cited observations from at least {min_origins} independent source domains.\n\
         - Never invent evidence_ids: every id must appear in the evidence lists.\n\
         - \"recommendation\" claims (suggested actions) may cite zero evidence_ids but must be \
           labelled \"recommendation\".\n\
         - confidence is 0.0–1.0; omit it rather than guessing.\n\
         - If the evidence is insufficient for a claim, put the gap in limitations instead of \
           asserting it.",
        single_item_cap = INFERENCE_SINGLE_ITEM_CONFIDENCE_CAP,
        high_threshold = HIGH_CONFIDENCE_INFERENCE_THRESHOLD,
        min_origins = HIGH_CONFIDENCE_MIN_ORIGINS,
    ));

    let entity_names_str = if bundle.entity_names.is_empty() {
        "unspecified entities".to_string()
    } else {
        single_line(&bundle.entity_names.join(", "), 600)
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
        title = single_line(&warning.title, 300),
        severity = warning.severity,
        warning_type = warning.warning_type,
        region = region,
        confidence = confidence_pct,
        entities = entity_names_str,
        description = single_line(warning.description.as_deref().unwrap_or("(none)"), 2000),
        evidence = evidence_block(bundle),
        max_claims = MAX_CLAIMS,
        max_impact = MAX_IMPACT_ITEMS,
        max_actions = MAX_ACTIONS,
        max_limitations = MAX_LIMITATIONS,
    );
    (system, user)
}

fn prompt_budget_chars() -> usize {
    std::env::var(MAX_PROMPT_CHARS_ENV_VAR)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= 4_000)
        .unwrap_or(DEFAULT_MAX_PROMPT_CHARS)
}

/// Fit the evidence set to the prompt budget, newest evidence first.
///
/// The documented evidence caps bound the *count* of items; this bounds the
/// rendered *size* against the model's context window, so the prompt can never
/// be silently truncated by the server (which would drop the system prompt and
/// citation rules). Observations and insights dropped here are reflected in
/// the run's sent counts and the prompt header ("showing X of Y available"),
/// and the `send only newest` order means the freshest evidence survives.
pub fn apply_prompt_budget(
    profile: &IntelligenceProfile,
    warning: &WarningRow,
    bundle: EvidenceBundle,
) -> EvidenceBundle {
    apply_prompt_budget_with(profile, warning, bundle, prompt_budget_chars())
}

fn apply_prompt_budget_with(
    profile: &IntelligenceProfile,
    warning: &WarningRow,
    mut bundle: EvidenceBundle,
    budget: usize,
) -> EvidenceBundle {
    let (system, user) = build_prompts(profile, warning, &bundle);
    if system.len() + user.len() <= budget {
        return bundle;
    }

    // Drop oldest observations in chunks first (the list is newest-first).
    while !bundle.observations.is_empty() {
        let drop = bundle.observations.len().div_ceil(10).max(1);
        bundle
            .observations
            .truncate(bundle.observations.len() - drop);
        let (system, user) = build_prompts(profile, warning, &bundle);
        if system.len() + user.len() <= budget {
            return bundle;
        }
    }

    // Then shed derived insights, least-recent first (also newest-first).
    while !bundle.insights.is_empty() {
        bundle.insights.truncate(bundle.insights.len() - 1);
        let (system, user) = build_prompts(profile, warning, &bundle);
        if system.len() + user.len() <= budget {
            return bundle;
        }
    }

    bundle
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
    /// Deterministic run status ([`AnalysisStatus::InsufficientEvidence`] when
    /// the preflight found nothing citable).
    #[serde(default)]
    pub analysis_status: AnalysisStatus,
    /// Where the direct evidence came from: explicit `warning_evidence` links
    /// or the entity-observation fallback.
    #[serde(default)]
    pub evidence_scope: EvidenceScope,
    /// Explicit `warning_evidence` rows available for the warning (resolved
    /// observations, resolved documents and unresolved URLs).
    #[serde(default)]
    pub warning_evidence_count: usize,
    /// Links resolved to a real extracted observation.
    #[serde(default)]
    pub resolved_observation_links: usize,
    /// Links resolved to a fetched source document with no extracted
    /// observation yet.
    #[serde(default)]
    pub resolved_document_links: usize,
    /// Links whose stated source has not been acquired.
    #[serde(default)]
    pub unresolved_links: usize,
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
    // Best-effort: housekeeping failure must not block new analysis. The
    // threshold covers the configured timeout plus retries.
    expire_stale_runs(store, stale_run_seconds(model.primary.timeout_seconds)).await;

    // Bound the evidence to the prompt budget before hashing: the digest and
    // the persisted sent-vs-available counts must describe exactly what the
    // model will see.
    let bundle = apply_prompt_budget(
        &model.profile,
        warning,
        gather_evidence(store, warning).await?,
    );
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
/// failure — model, validation, serialization, or persistence — marks the run
/// failed with an explicit reason; never a silent success or a run stuck in
/// `running`.
pub async fn execute_analysis_run(store: &PgStore, context: &AnalysisRunContext) -> Result<()> {
    let claimed = store
        .start_warning_analysis_run(context.run_id)
        .await
        .context("warning analysis: failed to claim run")?;
    if !claimed {
        // Another executor owns it, or it is already terminal.
        return Ok(());
    }

    let finished = match run_analysis(context).await {
        Ok(finished) => finished,
        Err(error) => {
            return fail_run(store, context, &error).await;
        }
    };

    let persist = async {
        let output_json = serde_json::to_value(&finished.output)
            .context("warning analysis: failed to serialize output")?;
        store
            .complete_warning_analysis_run(context.run_id, &output_json, &finished.records)
            .await
            .context("warning analysis: failed to persist result")?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(error) = persist {
        return fail_run(store, context, &error).await;
    }

    tracing::info!(
        run_id = %context.run_id,
        warning_id = %context.warning.id,
        claims = finished.output.claims.len(),
        "warning analysis completed"
    );
    Ok(())
}

async fn fail_run(
    store: &PgStore,
    context: &AnalysisRunContext,
    error: &anyhow::Error,
) -> Result<()> {
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

struct FinishedAnalysis {
    output: WarningAnalysisOutput,
    records: Vec<AnalysisClaimRecord>,
}

/// Deterministic limitation recorded for an insufficient-evidence run.
pub const INSUFFICIENT_EVIDENCE_LIMITATION: &str =
    "No observations or related insights were available for this warning, so no analysis was generated.";

fn base_output(
    context: &AnalysisRunContext,
    status: AnalysisStatus,
    claims: Vec<RenderedClaim>,
    impact: Vec<RenderedClaim>,
    actions: Vec<RenderedClaim>,
    limitations: Vec<String>,
) -> WarningAnalysisOutput {
    WarningAnalysisOutput {
        schema_version: ANALYSIS_OUTPUT_SCHEMA_VERSION,
        analysis_status: status,
        evidence_scope: context.bundle.evidence_scope,
        warning_evidence_count: context.bundle.warning_evidence_count,
        resolved_observation_links: context.bundle.resolved_observation_links,
        resolved_document_links: context.bundle.resolved_document_links,
        unresolved_links: context.bundle.unresolved_links,
        claims,
        impact,
        actions,
        limitations,
        evidence_quality: assess_bundle_quality(
            &context.bundle,
            &context.warning.title,
            Utc::now(),
        ),
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
    }
}

async fn run_analysis(context: &AnalysisRunContext) -> Result<FinishedAnalysis> {
    // Deterministic preflight (audit item 3): with no observations and no
    // insights there is nothing the model could cite, so the run reports
    // InsufficientEvidence without paying for an LLM call.
    if preflight_status(&context.bundle).is_insufficient() {
        let output = base_output(
            context,
            AnalysisStatus::InsufficientEvidence,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![INSUFFICIENT_EVIDENCE_LIMITATION.to_string()],
        );
        return Ok(FinishedAnalysis {
            output,
            records: Vec::new(),
        });
    }

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
    let index = context.bundle.evidence_index();
    let validated = validate_analysis_payload(payload, &index)?;

    let output = base_output(
        context,
        AnalysisStatus::Completed,
        validated
            .claims
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        validated
            .impact
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        validated
            .actions
            .iter()
            .map(|claim| render_claim(&context.bundle, claim))
            .collect(),
        validated.limitations.clone(),
    );

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

    /// The evidence index for the unit-test payloads: one observation with a
    /// resolvable origin and one related insight.
    fn evidence_index() -> EvidenceIndex {
        let mut observation_origins = HashMap::new();
        observation_origins.insert(Uuid::nil(), Some("example.com".to_string()));
        EvidenceIndex {
            observation_ids: observation_ids(),
            insight_ids: insight_ids(),
            observation_origins,
        }
    }

    fn valid_payload_json(evidence_id: Uuid) -> String {
        json!({
            "claims": [
                {"text": "Plant X opened in March", "claim_kind": "observed", "evidence_ids": [evidence_id.to_string()], "confidence": 0.9},
                {"text": "Capacity may tighten", "claim_kind": "inference", "evidence_ids": [evidence_id.to_string()], "confidence": 0.6}
            ],
            "impact": [
                {"text": "Lead times could extend", "claim_kind": "inference", "evidence_ids": [evidence_id.to_string()], "confidence": 0.5}
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
        validate_analysis_payload(payload, &evidence_index())
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
            "claims": [{"text": "Cross-referenced conclusion", "claim_kind": "inference", "evidence_ids": [insight.to_string()], "confidence": 0.5}],
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
    fn observed_claim_citing_only_a_derived_insight_is_rejected() {
        let insight = Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap();
        let raw = json!({
            "claims": [{"text": "Stated as a fact", "claim_kind": "observed", "evidence_ids": [insight.to_string()], "confidence": 0.8}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        let error = validate(&raw).expect_err("a derived insight cannot make a claim observed");
        assert!(
            format!("{error:#}").contains("at least one direct observation"),
            "{error:#}"
        );
    }

    #[test]
    fn inference_with_one_item_requires_an_explicit_confidence_cap() {
        let id = Uuid::nil();
        let uncapped = json!({
            "claims": [{"text": "Single-source inference", "claim_kind": "inference", "evidence_ids": [id.to_string()]}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        let error = validate(&uncapped).expect_err("single-item inference without a cap");
        assert!(
            format!("{error:#}").contains("single-item inference"),
            "{error:#}"
        );

        let too_sure = json!({
            "claims": [{"text": "Single-source inference", "claim_kind": "inference", "evidence_ids": [id.to_string()], "confidence": 0.75}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        assert!(validate(&too_sure).is_err(), "above-cap single item");

        let capped = json!({
            "claims": [{"text": "Single-source inference", "claim_kind": "inference", "evidence_ids": [id.to_string()], "confidence": 0.5}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        validate(&capped).expect("capped single-item inference is allowed");
    }

    #[test]
    fn high_confidence_inference_requires_two_independent_origins() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut index = EvidenceIndex {
            observation_ids: [first, second].into_iter().collect(),
            ..EvidenceIndex::default()
        };
        index
            .observation_origins
            .insert(first, Some("example.com".to_string()));
        index
            .observation_origins
            .insert(second, Some("example.com".to_string()));

        let same_origin = json!({
            "claims": [{"text": "Confident claim", "claim_kind": "inference", "evidence_ids": [first.to_string(), second.to_string()], "confidence": 0.9}],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string();
        let payload = parse_analysis_payload(&same_origin).unwrap();
        let error = validate_analysis_payload(payload, &index)
            .expect_err("one origin cannot support high-confidence inference");
        assert!(
            format!("{error:#}").contains("independent origin"),
            "{error:#}"
        );

        index
            .observation_origins
            .insert(second, Some("other.example.org".to_string()));
        let payload = parse_analysis_payload(&same_origin).unwrap();
        validate_analysis_payload(payload, &index)
            .expect("two independent origins support high-confidence inference");
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
    fn digest_ignores_bookkeeping_changes_but_tracks_prompt_content() {
        let warning = test_warning();
        let bundle = test_bundle(vec![test_observation(Uuid::new_v4())], 1);
        let first = evidence_digest(&warning, &bundle);

        // Acknowledge/review bump `updated_at` without changing what the model
        // sees: repeated clicks must still deduplicate.
        let mut bookkeeping = warning.clone();
        bookkeeping.updated_at = warning.updated_at.map(|ts| ts + chrono::Duration::hours(1));
        bookkeeping.acknowledged = true;
        bookkeeping.review_outcome = Some("reviewed".to_string());
        assert_eq!(
            first,
            evidence_digest(&bookkeeping, &bundle),
            "bookkeeping-only changes must not defeat dedupe"
        );

        // Prompt-visible content still changes the digest.
        let mut content = warning.clone();
        content.title = "Different title".to_string();
        assert_ne!(first, evidence_digest(&content, &bundle));
    }

    #[test]
    fn stale_threshold_covers_retries_of_configured_timeout() {
        assert_eq!(stale_run_seconds(300), DEFAULT_STALE_RUN_SECONDS);
        assert_eq!(stale_run_seconds(600), 2700);
        assert!(stale_run_seconds(900) > 4 * 900);
    }

    #[test]
    fn prompt_budget_truncates_evidence_and_counts_only_what_is_sent() {
        let profile = IntelligenceProfile {
            name: "Test".to_string(),
            system_prompt: "You are a test analyst.".to_string(),
            focus_areas: vec!["supply chains".to_string()],
        };
        let observations: Vec<ObservationRow> = (0..40)
            .map(|index| {
                test_observation_with(
                    Uuid::new_v4(),
                    &format!("observation {index}"),
                    "https://example.com/a",
                )
            })
            .collect();
        let newest_id = observations[0].id;
        let bundle = test_bundle(observations, 40);

        let bounded = apply_prompt_budget_with(&profile, &test_warning(), bundle, 6_000);
        let (system, user) = build_prompts(&profile, &test_warning(), &bounded);

        assert!(
            system.len() + user.len() <= 6_000,
            "prompt must fit the budget ({} chars)",
            system.len() + user.len()
        );
        assert!(
            bounded.observations.len() < 40,
            "budget must have dropped evidence"
        );
        assert_eq!(
            bounded
                .observations
                .first()
                .map(|observation| observation.id),
            Some(newest_id),
            "newest evidence is kept first"
        );
        assert!(
            user.contains(&format!(
                "showing {} of {} available",
                bounded.observations.len(),
                40
            )),
            "prompt header must reflect the truncated sent count"
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

    #[test]
    fn prompt_names_the_evidence_scope_and_claim_rules() {
        let profile = IntelligenceProfile {
            name: "Test".to_string(),
            system_prompt: "You are a test analyst.".to_string(),
            focus_areas: vec![],
        };
        let mut bundle = test_bundle(vec![test_observation(Uuid::nil())], 1);
        bundle.evidence_scope = EvidenceScope::WarningObservations;
        bundle.warning_evidence_count = 1;
        let (system, user) = build_prompts(&profile, &test_warning(), &bundle);
        assert!(
            user.contains("source: warning_observations"),
            "prompt must name the evidence scope: {user}"
        );
        assert!(system.contains("at least one observation id"));
        assert!(system.contains("exactly one cited item"));
        assert!(system.contains("independent source domains"));
    }

    #[test]
    fn preflight_reports_insufficient_evidence_without_a_model_call() {
        let empty = test_bundle(Vec::new(), 0);
        assert_eq!(
            preflight_status(&empty),
            AnalysisStatus::InsufficientEvidence
        );

        let with_observation = test_bundle(vec![test_observation(Uuid::new_v4())], 1);
        assert_eq!(
            preflight_status(&with_observation),
            AnalysisStatus::Completed
        );

        let with_insight = EvidenceBundle {
            insights: vec![test_insight()],
            insights_available: 1,
            ..test_bundle(Vec::new(), 0)
        };
        assert_eq!(preflight_status(&with_insight), AnalysisStatus::Completed);
    }

    #[tokio::test]
    async fn insufficient_evidence_short_circuits_before_the_model() {
        // The model config points at a dead endpoint: if the executor called
        // the model this test would fail (or hang until the timeout), so a
        // fast Ok proves the preflight skipped the call.
        let context = AnalysisRunContext {
            run_id: Uuid::new_v4(),
            warning: test_warning(),
            bundle: test_bundle(Vec::new(), 0),
            model: ModelConfig {
                model_name: "unreachable".to_string(),
                provider: apex_llm::LlmProvider::OpenAi,
                base_url: "http://127.0.0.1:9".to_string(),
                api_key: None,
                max_tokens: 16,
                temperature: 0.0,
                timeout_seconds: 1,
            },
            profile: IntelligenceProfile {
                name: "Test".to_string(),
                system_prompt: "You are a test analyst.".to_string(),
                focus_areas: Vec::new(),
            },
        };

        let finished = run_analysis(&context)
            .await
            .expect("preflight must succeed");
        assert_eq!(
            finished.output.analysis_status,
            AnalysisStatus::InsufficientEvidence
        );
        assert!(finished.records.is_empty());
        assert!(finished.output.claims.is_empty());
        assert!(finished
            .output
            .limitations
            .iter()
            .any(|limitation| limitation == INSUFFICIENT_EVIDENCE_LIMITATION));
    }

    #[test]
    fn dedup_keeps_observations_that_only_share_a_long_prefix() {
        let prefix = "x".repeat(200);
        let first = test_observation_with(
            Uuid::new_v4(),
            &format!("{prefix} alpha tail"),
            "https://example.com/a",
        );
        let second = test_observation_with(
            Uuid::new_v4(),
            &format!("{prefix} beta tail"),
            "https://example.com/a",
        );
        let deduped = dedup_observations(vec![first, second]);
        assert_eq!(
            deduped.len(),
            2,
            "content that only shares a 120-char prefix must not collapse"
        );
    }

    #[test]
    fn dedup_suppresses_identical_content_from_the_same_origin() {
        let first =
            test_observation_with(Uuid::new_v4(), "identical payload", "https://example.com/a");
        let second =
            test_observation_with(Uuid::new_v4(), "identical payload", "https://example.com/b");
        // Different URLs but the same registrable origin: semantic duplicate.
        assert_eq!(dedup_observations(vec![first, second]).len(), 1);
    }

    #[test]
    fn dedup_keeps_identical_content_from_different_origins() {
        let first =
            test_observation_with(Uuid::new_v4(), "identical payload", "https://example.com/a");
        let second = test_observation_with(
            Uuid::new_v4(),
            "identical payload",
            "https://other.example.org/b",
        );
        assert_eq!(dedup_observations(vec![first, second]).len(), 2);
    }

    #[test]
    fn canonical_content_hash_ignores_json_key_order() {
        let mut first = test_observation_with(Uuid::new_v4(), "payload", "https://example.com/a");
        first.value = json!({"a": 2, "b": 1});
        let mut second = test_observation_with(Uuid::new_v4(), "payload", "https://example.com/a");
        second.value = json!({"b": 1, "a": 2});
        assert_eq!(
            canonical_observation_content_hash(&first),
            canonical_observation_content_hash(&second)
        );
    }

    #[test]
    fn digest_tracks_the_evidence_scope() {
        let warning = test_warning();
        let bundle = test_bundle(vec![test_observation(Uuid::new_v4())], 1);
        let first = evidence_digest(&warning, &bundle);

        let mut linked = bundle.clone();
        linked.evidence_scope = EvidenceScope::WarningObservations;
        linked.warning_evidence_count = 1;
        assert_ne!(
            first,
            evidence_digest(&warning, &linked),
            "linked warning evidence must change the digest"
        );
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
        test_observation_with(id, "A real observation", "https://example.com/a")
    }

    fn test_observation_with(id: Uuid, text: &str, source_url: &str) -> ObservationRow {
        ObservationRow {
            id,
            observation_type: "signal".to_string(),
            entity_id: None,
            entity_type: None,
            ts_utc: Utc::now(),
            value: json!({"text": text}),
            provenance: json!({"source_url": source_url}),
            confidence: Some(0.7),
            created_at: Some(Utc::now()),
        }
    }

    fn test_insight() -> apex_store::postgres::InsightRow {
        apex_store::postgres::InsightRow {
            id: Uuid::new_v4(),
            title: "Related insight".to_string(),
            summary: "Derived finding".to_string(),
            insight_type: Some("supply_chain".to_string()),
            region: None,
            confidence: Some(0.6),
            evidence_urls: None,
            entity_ids: None,
            tags: None,
            metadata: None,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
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
            warning_evidence_count: 0,
            resolved_observation_links: 0,
            resolved_document_links: 0,
            unresolved_links: 0,
            evidence_scope: EvidenceScope::EntityFallback,
        }
    }

    /// The audit scenario: a warning whose stated source is unresolved must
    /// not be grounded in unrelated entity observations. Even with plenty of
    /// entity observations available, the bundle's direct observations stay
    /// empty, the scope is `WarningEvidenceUnresolved`, and the preflight is
    /// deterministically InsufficientEvidence.
    #[test]
    fn unresolved_warning_source_never_substitutes_entity_observations() {
        let bundle = EvidenceBundle {
            observations: Vec::new(),
            insights: Vec::new(),
            observations_available: 0,
            insights_available: 40,
            entity_names: vec!["Acme".to_string()],
            source_domains: Vec::new(),
            source_reliability: Vec::new(),
            warning_evidence_count: 1,
            resolved_observation_links: 0,
            resolved_document_links: 0,
            unresolved_links: 1,
            evidence_scope: EvidenceScope::WarningEvidenceUnresolved,
        };
        assert!(bundle.observations.is_empty());
        assert_eq!(
            preflight_status(&bundle),
            AnalysisStatus::InsufficientEvidence,
            "an unresolved explicit source must never fall back to the model"
        );
        assert!(!bundle.evidence_scope.has_direct_observations());
        assert!(bundle.evidence_scope.is_explicit());
    }

    /// A warning with no `warning_evidence` rows at all keeps the legacy
    /// entity-observation fallback.
    #[test]
    fn no_links_keeps_entity_fallback() {
        let bundle = EvidenceBundle {
            observations: vec![test_observation(Uuid::new_v4())],
            insights: Vec::new(),
            observations_available: 1,
            insights_available: 0,
            entity_names: vec!["Acme".to_string()],
            source_domains: Vec::new(),
            source_reliability: Vec::new(),
            warning_evidence_count: 0,
            resolved_observation_links: 0,
            resolved_document_links: 0,
            unresolved_links: 0,
            evidence_scope: EvidenceScope::EntityFallback,
        };
        assert!(bundle.evidence_scope.has_direct_observations());
        assert!(!bundle.evidence_scope.is_explicit());
        assert_eq!(preflight_status(&bundle), AnalysisStatus::Completed);
    }
}
