//! Narrative rendering from recipes + evidence.
//!
//! Takes InsightCandidate data (recipe template, evidence slots, entity context)
//! and produces human-readable insight cards with narratives, actions, and citations.

use apex_core::validation::normalize_url;
use apex_stats::mutual_info;
use chrono::{DateTime, Utc};
use regex::Regex;
use crate::entity_relevance::EntityRegistry;
use crate::title_diversity::TitleGenerator;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use tracing::warn;
use uuid::Uuid;

/// Maximum number of candidates accepted in a single [`render_batch`] call (B286).
///
/// Each candidate can produce a sizable `InsightCard` with narrative text,
/// citations, and actions.  Above 1 000 cards the output Vec can grow to tens
/// of MiB per call.  Inputs exceeding this limit are truncated with a
/// `WARN`-level tracing event.
pub const MAX_RENDER_BATCH_SIZE: usize = 1_000;

// ────────────────────────────────────────────
// Insight Candidate — input to the renderer
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightCandidate {
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub entity_id: Uuid,
    pub entity_name: String,
    pub confidence: f64,
    pub impact: f64,
    pub narrative_template: String,
    pub action_template: String,
    pub evidence: Vec<EvidenceSlot>,
    pub severity: String,
    pub category: String,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSlot {
    pub slot_name: String,
    pub value: String,
    pub source_url: Option<String>,
    pub source_domain: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

// ────────────────────────────────────────────
// Insight Card — rendered output
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightCard {
    pub id: Uuid,
    pub recipe_code: String,
    pub entity_id: Uuid,
    pub entity_name: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub narrative: String,
    pub actions: Vec<String>,
    pub citations: Vec<Citation>,
    pub confidence: f64,
    pub impact: f64,
    pub impact_label: String,
    pub priority_score: f64,
    pub region: Option<String>,
    pub rendered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub index: usize,
    pub source_url: String,
    pub source_domain: String,
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceRedundancyLink {
    pub left_index: usize,
    pub right_index: usize,
    pub normalized_mutual_information: f64,
}

// ────────────────────────────────────────────
// Template rendering
// ────────────────────────────────────────────

/// Maximum byte length for a rendered template output (B280 — runaway-expansion guard).
///
/// A template with a single slot filled by a 1 MB value would otherwise produce
/// 1 MB of output per render call.  This cap prevents accidental or adversarial
/// memory blowup while remaining generous enough for the largest real-world
/// narrative templates (~50 KB).
const MAX_TEMPLATE_OUTPUT_BYTES: usize = 256 * 1024; // 256 KB

fn utf8_prefix(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Render a template string by replacing `{slot_name}` placeholders with evidence values.
/// Unknown placeholders are left as-is.
///
/// Uses single-pass replacement to prevent evidence values containing `{other_slot}`
/// patterns from being expanded (slot injection via sequential replace).
///
/// # Capacity
/// Pre-allocates `template.len() + total_slot_value_bytes` to minimise
/// re-allocations when slot values are substantially longer than the
/// placeholder names.  Output is hard-capped at [`MAX_TEMPLATE_OUTPUT_BYTES`]
/// (256 KB); any template that would exceed this limit is returned truncated
/// with `…` appended.
pub fn render_template(template: &str, slots: &HashMap<String, String>) -> String {
    // Better capacity estimate: template body + sum of all slot values (upper bound).
    let slot_total_bytes: usize = slots.values().map(String::len).sum();
    let initial_capacity = template
        .len()
        .saturating_add(slot_total_bytes)
        .min(MAX_TEMPLATE_OUTPUT_BYTES + 4); // +4 for the "…" truncation marker
    let mut result = String::with_capacity(initial_capacity);
    let mut chars = template.char_indices().peekable();

    'outer: while let Some((i, ch)) = chars.next() {
        if ch == '{' {
            // Look for closing brace to extract placeholder name
            let start = i + 1; // byte after '{'
            let mut found_end = false;
            while let Some(&(j, c2)) = chars.peek() {
                if c2 == '}' {
                    let key = &template[start..j];
                    if let Some(val) = slots.get(key) {
                        // Enforce output cap before appending slot value.
                        let remaining = MAX_TEMPLATE_OUTPUT_BYTES.saturating_sub(result.len());
                        if val.len() <= remaining {
                            result.push_str(val);
                        } else {
                            // Truncate at a valid UTF-8 boundary.
                            let safe_end = val
                                .char_indices()
                                .take_while(|(idx, _)| *idx < remaining.saturating_sub(3))
                                .last()
                                .map(|(idx, c)| idx + c.len_utf8())
                                .unwrap_or(0);
                            result.push_str(&val[..safe_end]);
                            result.push('…');
                            break 'outer;
                        }
                    } else {
                        // Unknown placeholder — keep as-is
                        let needed = 2 + key.len(); // '{' + key + '}'
                        if result.len().saturating_add(needed) <= MAX_TEMPLATE_OUTPUT_BYTES {
                            result.push('{');
                            result.push_str(key);
                            result.push('}');
                        }
                    }
                    chars.next(); // consume '}'
                    found_end = true;
                    break;
                }
                // Nested '{' means this isn't a simple placeholder — emit and bail
                if c2 == '{' {
                    if result.len() < MAX_TEMPLATE_OUTPUT_BYTES {
                        result.push('{');
                    }
                    found_end = true;
                    break;
                }
                chars.next();
            }
            if !found_end {
                // Reached end of string without closing brace
                if result.len() < MAX_TEMPLATE_OUTPUT_BYTES {
                    result.push('{');
                    let remaining = MAX_TEMPLATE_OUTPUT_BYTES.saturating_sub(result.len());
                    let tail = &template[start..];
                    if tail.len() <= remaining {
                        result.push_str(tail);
                    } else {
                        result.push_str(utf8_prefix(tail, remaining));
                        result.push('…');
                    }
                }
            }
        } else if result.len() < MAX_TEMPLATE_OUTPUT_BYTES {
            result.push(ch);
        } else {
            result.push('…');
            break;
        }
    }

    result
}

/// Build slot map from evidence slots.
/// Inserts both `slot_name` and `evidence:slot_name` keys to handle templates
/// using either `{slot_name}` or `{evidence:slot_name}` placeholder syntax.
pub fn build_slot_map(evidence: &[EvidenceSlot]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for slot in evidence {
        if !is_valid_slot_name(&slot.slot_name) {
            continue;
        }
        let key = slot.slot_name.trim().to_string();
        let value = slot.value.trim().to_string();
        // Keep first seen value to avoid silent collisions overriding prior evidence.
        map.entry(key.clone()).or_insert(value.clone());
        map.entry(format!("evidence:{}", key)).or_insert(value);
    }
    map
}

fn is_valid_slot_name(slot_name: &str) -> bool {
    let s = slot_name.trim();
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Counter for title template rotation to ensure variety across insights.
static TITLE_TEMPLATE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Title template variants for generating varied insight titles.
/// Each template provides a different narrative structure to avoid formulaic output.
const TITLE_TEMPLATES: &[(&str, &str)] = &[
    ("demand", "{entity}: Procurement activity detected"),
    ("demand", "Sourcing signal from {entity}"),
    ("demand", "{entity} shows demand indicators"),
    ("supply_chain", "Supply chain update: {entity}"),
    ("supply_chain", "{entity} supply chain alert"),
    ("supply_chain", "Logistics signal — {entity}"),
    ("competitor", "Competitive intel: {entity}"),
    ("competitor", "{entity} competitor activity"),
    ("competitor", "Market movement from {entity}"),
    ("security", "Security advisory: {entity}"),
    ("security", "{entity} security signal"),
    ("security", "Threat indicator — {entity}"),
    ("poi", "Stakeholder update: {entity}"),
    ("poi", "{entity} personnel intelligence"),
    ("poi", "Key contact movement — {entity}"),
    ("regulatory", "Regulatory update: {entity}"),
    ("regulatory", "{entity} compliance signal"),
    ("regulatory", "Policy change affecting {entity}"),
    ("commodity", "Commodity alert: {entity}"),
    ("commodity", "{entity} material sourcing"),
    ("commodity", "Supply constraint — {entity}"),
    ("logistics", "Logistics alert: {entity}"),
    ("logistics", "{entity} shipping signal"),
    ("logistics", "Transport update — {entity}"),
];

/// Generate title from recipe code and entity name with varied templates.
///
/// Uses round-robin rotation across category-specific templates to ensure
/// title variety. Falls back to structured format for unknown categories.
/// Generate a title using the `TitleGenerator` for semantic diversity.
///
/// Builds a [`SignalContext`] from the candidate's fields and delegates to
/// `TitleGenerator::generate()`. Falls back to the legacy `generate_title()`
/// if no generator is provided (backward compat).
pub fn generate_diverse_title(
    gen: &mut TitleGenerator,
    candidate: &InsightCandidate,
    registry: Option<&EntityRegistry>,
) -> String {
    use crate::entity_relevance::SignalContext;

    let context = SignalContext {
        text: candidate.narrative_template.clone(),
        entity_hint: Some(candidate.entity_name.clone()),
        category: Some(candidate.category.clone()),
        source_url: None,
        timestamp: Utc::now().timestamp(),
    };
    let diverse = gen.generate(&context, registry);
    format!("[{}] {}", candidate.recipe_code, diverse.title)
}

/// Generate a title using the legacy static template system.
pub fn generate_title(recipe_code: &str, category: &str, entity_name: &str) -> String {
    // Get templates for this category
    let category_templates: Vec<&str> = TITLE_TEMPLATES
        .iter()
        .filter(|(cat, _)| *cat == category)
        .map(|(_, template)| *template)
        .collect();

    if category_templates.is_empty() {
        // Fallback for unknown categories: use structured format
        return format!("[{}] {} for {}", recipe_code, category, entity_name);
    }

    // Rotate through templates using atomic counter
    let idx = TITLE_TEMPLATE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let template_idx = (idx as usize) % category_templates.len();
    let template = category_templates[template_idx];

    // Fill template
    let title = template.replace("{entity}", entity_name);

    // Prepend recipe code for traceability
    format!("[{}] {}", recipe_code, title)
}

/// Generate title with explicit template index (for deterministic testing).
#[cfg(test)]
pub fn generate_title_with_index(
    recipe_code: &str,
    category: &str,
    entity_name: &str,
    template_idx: usize,
) -> String {
    let category_templates: Vec<&str> = TITLE_TEMPLATES
        .iter()
        .filter(|(cat, _)| *cat == category)
        .map(|(_, template)| *template)
        .collect();

    if category_templates.is_empty() {
        return format!("[{}] {} for {}", recipe_code, category, entity_name);
    }

    let template = category_templates[template_idx % category_templates.len()];
    let title = template.replace("{entity}", entity_name);
    format!("[{}] {}", recipe_code, title)
}

/// Classify impact level from numeric value.
pub fn impact_label(impact: f64) -> &'static str {
    if impact >= 0.8 {
        "Critical"
    } else if impact >= 0.6 {
        "High"
    } else if impact >= 0.4 {
        "Medium"
    } else if impact >= 0.2 {
        "Low"
    } else {
        "Info"
    }
}

/// Compute priority score = impact * confidence * urgency_multiplier(severity).
pub fn priority_score(impact: f64, confidence: f64, severity: &str) -> f64 {
    let urgency = match severity {
        "critical" => 2.0,
        "high" | "warning" => 1.5,
        "medium" => 1.25,
        "low" | "info" => 1.0,
        _ => 1.0,
    };
    impact * confidence * urgency
}

pub fn information_gain_bits(confidence: f64, severity: &str) -> f64 {
    let prior = default_state_distribution();
    let posterior =
        posterior_state_distribution(prior, severity_target_state(severity), confidence);
    (shannon_entropy_bits(&prior) - shannon_entropy_bits(&posterior)).max(0.0)
}

pub fn card_information_gain_bits(card: &InsightCard) -> f64 {
    information_gain_bits(card.confidence, &card.severity)
}

pub fn effective_evidence_weights(evidence: &[EvidenceSlot]) -> Vec<f64> {
    let mut seen_counts: HashMap<String, usize> = HashMap::new();
    evidence
        .iter()
        .map(|slot| {
            let source_type = evidence_source_type(slot);
            let count = seen_counts.entry(source_type).or_default();
            *count += 1;
            1.0 / (*count as f64)
        })
        .collect()
}

pub fn evidence_diversity_score(evidence: &[EvidenceSlot]) -> f64 {
    if evidence.is_empty() {
        return 0.0;
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for slot in evidence {
        *counts.entry(evidence_source_type(slot)).or_default() += 1;
    }

    let total = evidence.len() as f64;
    let concentration: f64 = counts
        .values()
        .map(|count| {
            let proportion = *count as f64 / total;
            proportion * proportion
        })
        .sum();
    (1.0 - concentration).clamp(0.0, 1.0)
}

pub fn pairwise_evidence_redundancy(evidence: &[EvidenceSlot]) -> Vec<EvidenceRedundancyLink> {
    let mut links = Vec::new();
    for left_idx in 0..evidence.len() {
        for right_idx in (left_idx + 1)..evidence.len() {
            let Some(nmi) =
                evidence_pair_normalized_mi(&evidence[left_idx].value, &evidence[right_idx].value)
            else {
                continue;
            };
            links.push(EvidenceRedundancyLink {
                left_index: left_idx,
                right_index: right_idx,
                normalized_mutual_information: nmi,
            });
        }
    }
    links.sort_by(|a, b| {
        b.normalized_mutual_information
            .partial_cmp(&a.normalized_mutual_information)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.left_index.cmp(&b.left_index))
            .then_with(|| a.right_index.cmp(&b.right_index))
    });
    links
}

pub fn evidence_support_multiplier(evidence: &[EvidenceSlot]) -> f64 {
    if evidence.is_empty() {
        return 1.0;
    }

    let weights = effective_evidence_weights(evidence);
    let weight_factor = weights.iter().sum::<f64>() / evidence.len() as f64;
    let diversity_bonus = 0.75 + 0.375 * evidence_diversity_score(evidence);
    let redundant_pairs = pairwise_evidence_redundancy(evidence)
        .iter()
        .filter(|link| link.normalized_mutual_information > 0.8)
        .count() as f64;
    let redundancy_penalty = 1.0 / (1.0 + 0.25 * redundant_pairs);

    (weight_factor * diversity_bonus * redundancy_penalty).clamp(0.35, 1.15)
}

fn normalize_card_severity(severity: &str) -> String {
    match severity.trim().to_lowercase().as_str() {
        "critical" => "critical".to_string(),
        "high" | "warning" => "warning".to_string(),
        "medium" => "medium".to_string(),
        "low" | "info" => "info".to_string(),
        _ => "info".to_string(),
    }
}

/// Extract citations from evidence slots that have source URLs.
pub fn extract_citations(evidence: &[EvidenceSlot]) -> Vec<Citation> {
    const MAX_CITATIONS: usize = 25;
    let mut citations = Vec::new();
    for slot in evidence {
        if citations.len() >= MAX_CITATIONS {
            break;
        }
        if let Some(ref url) = slot.source_url {
            if !url.is_empty() && is_strict_evidence_url(url) {
                let domain = slot
                    .source_domain
                    .clone()
                    .unwrap_or_else(|| extract_domain(url));
                citations.push(Citation {
                    index: citations.len() + 1,
                    source_url: url.clone(),
                    source_domain: domain,
                    observed_at: slot.observed_at,
                });
            }
        }
    }
    citations
}

fn is_strict_evidence_url(url: &str) -> bool {
    let normalized = match normalize_url(url) {
        Some(v) => v,
        None => return false,
    };
    normalized.starts_with("http://") || normalized.starts_with("https://")
}

/// Extract domain from a URL string (best-effort).
/// B187: handles malformed URLs by returning "unknown" as fallback.
pub fn extract_domain(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    if trimmed.starts_with('?') {
        return "unknown".to_string();
    }
    // Strip scheme
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    // Take up to first /
    let domain = without_scheme.split('/').next().unwrap_or(without_scheme);
    // Strip port
    let domain = domain.split(':').next().unwrap_or(domain);
    if domain.is_empty() || domain.starts_with('?') {
        "unknown".to_string()
    } else {
        domain.to_string()
    }
}

/// Maximum number of actions to avoid unbounded growth (B186).
const MAX_ACTIONS: usize = 50;

/// Maximum number of evidence slots processed per insight (B323).
const MAX_EVIDENCE_SLOTS: usize = 100;

static RE_TRAILING_CITATION_REFS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\s*\[\d+\])+\s*$")
        .unwrap_or_else(|error| panic!("valid trailing citation regex: {error}"))
});
static RE_CITATION_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\d+\]").unwrap_or_else(|error| panic!("valid citation regex: {error}"))
});

/// Regex patterns for confidence/source boilerplate that indicates
/// template-generated text rather than natural LLM output.
/// These patterns strip robotic boilerplate from the narrative when
/// the template path is used (LLM unavailable).
static RE_CONFIDENCE_BOILERPLATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        concat!(
            r"(?is)",  // case-insensitive, dot-matches-newline
            r"(?:",
            // "The current read is likely at roughly 52% confidence because..."
            r"The current read is likely at roughly \d+% confidence because[^.]+\.\s*",
            r"|",
            // "Reported by N independent sources; Includes N non-social reporting sources"
            r"Reported by \d+ independent sources;? Includes? \d+ non-social reporting sources\.?\s*",
            r"|",
            // "Coverage from ... is being compared for corroboration"
            r"Coverage from .+? is being compared for corroboration\.?\s*",
            r"|",
            // "The recurring reported themes involve ..."
            r"The recurring reported themes involve[^.]+\.\s*",
            r"|",
            // "The current read is likely at roughly X% confidence"
            r"The current read is likely at roughly \d+% confidence\.?\s*",
            r")",
        )
    ).unwrap_or_else(|error| panic!("valid confidence boilerplate regex: {error}"))
});

/// Parse action template into a list of actions.
/// Actions are separated by newlines or semicolons. Empty lines are skipped.
/// B186: capped at MAX_ACTIONS to prevent huge lists.
pub fn parse_actions(action_template: &str, slots: &HashMap<String, String>) -> Vec<String> {
    let rendered = render_template(action_template, slots);
    rendered
        .split(['\n', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(MAX_ACTIONS)
        .collect()
}

fn default_action_fallback() -> Vec<String> {
    vec!["Review evidence and define immediate next step".to_string()]
}

/// Add citation references to narrative text.
/// Appends "[1][2]..." at the end referencing all citations.
pub fn append_citation_refs(narrative: &str, citation_count: usize) -> String {
    let base = trim_trailing_citation_refs(narrative);
    if citation_count == 0 {
        return base;
    }
    let refs: Vec<String> = (1..=citation_count).map(|i| format!("[{}]", i)).collect();
    if base.is_empty() {
        refs.join("")
    } else {
        format!("{} {}", base, refs.join(""))
    }
}

fn trim_trailing_citation_refs(narrative: &str) -> String {
    RE_TRAILING_CITATION_REFS
        .replace(narrative, "")
        .trim_end()
        .to_string()
}

fn trailing_citation_ref_count(narrative: &str) -> usize {
    RE_TRAILING_CITATION_REFS
        .find(narrative)
        .map(|m| RE_CITATION_REF.find_iter(m.as_str()).count())
        .unwrap_or(0)
}

/// Strip confidence/source boilerplate patterns from template-generated narrative text.
/// This removes sentences like "The current read is likely at roughly 52% confidence because..."
/// that indicate template leakage rather than natural LLM generation.
///
/// Returns the cleaned narrative with boilerplate sentences removed.
/// If the entire narrative is boilerplate, returns an empty string (caller should handle).
pub fn strip_confidence_boilerplate(narrative: &str) -> String {
    let result = RE_CONFIDENCE_BOILERPLATE.replace_all(narrative, "");
    let trimmed = result.trim().to_string();
    // If after removal we have a trailing citation ref, clean that too
    if trimmed.ends_with('[') || trimmed.ends_with("[]") {
        trim_trailing_citation_refs(&trimmed)
    } else {
        trimmed
    }
}

// ────────────────────────────────────────────
// Main render function
// ────────────────────────────────────────────

/// Render an InsightCandidate into a full InsightCard.
pub fn render_insight(candidate: &InsightCandidate) -> InsightCard {
    let evidence: Vec<EvidenceSlot> = candidate
        .evidence
        .iter()
        .take(MAX_EVIDENCE_SLOTS)
        .cloned()
        .collect();
    let slots = build_slot_map(&evidence);
    let narrative_raw = render_template(&candidate.narrative_template, &slots);
    let mut actions = parse_actions(&candidate.action_template, &slots);
    if actions.is_empty() {
        actions = default_action_fallback();
    }
    let citations = extract_citations(&evidence);
    let narrative_trimmed = narrative_raw.trim();
    let mut narrative = append_citation_refs(narrative_trimmed, citations.len());
    if trailing_citation_ref_count(&narrative) != citations.len() {
        warn!(
            citation_count = citations.len(),
            trailing_refs = trailing_citation_ref_count(&narrative),
            "citation_reference_count_mismatch"
        );
        narrative = append_citation_refs(&narrative, citations.len());
    }
    // Clean template-generated boilerplate from narrative text.
    // This removes confidence/source enumeration sentences like
    // "The current read is likely at roughly 52% confidence because..."
    // that indicate template leakage rather than natural prose.
    let stripped = strip_confidence_boilerplate(&narrative);
    if !stripped.is_empty() {
        narrative = stripped;
    }
    let title = generate_title(
        &candidate.recipe_code,
        &candidate.category,
        &candidate.entity_name,
    );
    let severity = normalize_card_severity(&candidate.severity);
    let score = priority_score(candidate.impact, candidate.confidence, &severity)
        * evidence_support_multiplier(&evidence);

    InsightCard {
        id: Uuid::new_v4(),
        recipe_code: candidate.recipe_code.clone(),
        entity_id: candidate.entity_id,
        entity_name: candidate.entity_name.clone(),
        severity,
        category: candidate.category.clone(),
        title,
        narrative,
        actions,
        citations,
        confidence: candidate.confidence,
        impact: candidate.impact,
        impact_label: impact_label(candidate.impact).to_string(),
        priority_score: score,
        region: candidate.region.clone(),
        rendered_at: Utc::now(),
    }
}

/// Render an [`InsightCandidate`] into a full [`InsightCard`] using diverse
/// title generation via [`TitleGenerator`].
///
/// This is a drop-in replacement for [`render_insight`] that produces
/// semantically diverse titles across consecutive calls.
pub fn render_insight_with_diversity(
    gen: &mut TitleGenerator,
    candidate: &InsightCandidate,
    registry: Option<&EntityRegistry>,
) -> InsightCard {
    let evidence: Vec<EvidenceSlot> = candidate
        .evidence
        .iter()
        .take(MAX_EVIDENCE_SLOTS)
        .cloned()
        .collect();
    let slots = build_slot_map(&evidence);
    let narrative_raw = render_template(&candidate.narrative_template, &slots);
    let mut actions = parse_actions(&candidate.action_template, &slots);
    if actions.is_empty() {
        actions = default_action_fallback();
    }
    let citations = extract_citations(&evidence);
    let mut narrative = append_citation_refs(narrative_raw.trim(), citations.len());
    if trailing_citation_ref_count(&narrative) != citations.len() {
        warn!(
            citation_count = citations.len(),
            trailing_refs = trailing_citation_ref_count(&narrative),
            "citation_reference_count_mismatch"
        );
        narrative = append_citation_refs(&narrative, citations.len());
    }
    let title = generate_diverse_title(gen, candidate, registry);
    let severity = normalize_card_severity(&candidate.severity);
    let score = priority_score(candidate.impact, candidate.confidence, &severity)
        * evidence_support_multiplier(&evidence);

    InsightCard {
        id: Uuid::new_v4(),
        recipe_code: candidate.recipe_code.clone(),
        entity_id: candidate.entity_id,
        entity_name: candidate.entity_name.clone(),
        severity,
        category: candidate.category.clone(),
        title,
        narrative,
        actions,
        citations,
        confidence: candidate.confidence,
        impact: candidate.impact,
        impact_label: impact_label(candidate.impact).to_string(),
        priority_score: score,
        region: candidate.region.clone(),
        rendered_at: Utc::now(),
    }
}

/// Rank insight cards by priority score (descending).
pub fn rank_insights(cards: &mut [InsightCard]) {
    cards.sort_by(|a, b| {
        let ig_order = card_information_gain_bits(b)
            .partial_cmp(&card_information_gain_bits(a))
            .unwrap_or(std::cmp::Ordering::Equal);
        if ig_order != std::cmp::Ordering::Equal {
            return ig_order;
        }

        // Secondary: priority score DESC (higher is more important)
        let score_order = b
            .priority_score
            .partial_cmp(&a.priority_score)
            .unwrap_or_else(|| {
                // NaN priority scores sort to the end (lowest priority)
                a.priority_score.is_nan().cmp(&b.priority_score.is_nan())
            });
        if score_order != std::cmp::Ordering::Equal {
            return score_order;
        }
        // Secondary: recipe_code ASC — deterministic tiebreak for identical scores
        // (B292: required for stable snapshot output across repeated calls)
        let code_order = a.recipe_code.cmp(&b.recipe_code);
        if code_order != std::cmp::Ordering::Equal {
            return code_order;
        }
        // Tertiary: entity_id ASC — breaks ties between same recipe, same score
        a.entity_id.cmp(&b.entity_id)
    });
}

/// Filter insights by minimum confidence threshold.
pub fn filter_by_confidence(cards: &[InsightCard], min_confidence: f64) -> Vec<InsightCard> {
    cards
        .iter()
        .filter(|c| c.confidence >= min_confidence)
        .cloned()
        .collect()
}

/// Filter insights by category.
pub fn filter_by_category<'a>(cards: &'a [InsightCard], category: &str) -> Vec<&'a InsightCard> {
    cards.iter().filter(|c| c.category == category).collect()
}

/// Group insights by category.
pub fn group_by_category(cards: &[InsightCard]) -> HashMap<String, Vec<&InsightCard>> {
    let mut groups: HashMap<String, Vec<&InsightCard>> = HashMap::new();
    for card in cards {
        groups.entry(card.category.clone()).or_default().push(card);
    }
    groups
}

/// Group insights by region.
pub fn group_by_region(cards: &[InsightCard]) -> HashMap<String, Vec<&InsightCard>> {
    let mut groups: HashMap<String, Vec<&InsightCard>> = HashMap::new();
    for card in cards {
        let region = card.region.clone().unwrap_or_else(|| "global".to_string());
        groups.entry(region).or_default().push(card);
    }
    groups
}

/// Render a batch of candidates into ranked insight cards.
///
/// # Batch size limit (B286)
/// Inputs larger than [`MAX_RENDER_BATCH_SIZE`] are truncated before
/// rendering.  The truncation is logged at `WARN` level so that operators can
/// detect runaway callers and shard their workloads accordingly.
pub fn render_batch(candidates: &[InsightCandidate]) -> Vec<InsightCard> {
    // B295: Deduplicate by (recipe_code, entity_id) composite key before truncation
    let mut seen_keys = std::collections::HashSet::new();
    let mut unique_candidates = Vec::new();
    let mut dup_count = 0;
    for candidate in candidates {
        let key = (&candidate.recipe_code, &candidate.entity_id);
        if seen_keys.insert(key) {
            unique_candidates.push(candidate);
        } else {
            dup_count += 1;
        }
    }
    if dup_count > 0 {
        warn!(
            duplicate_count = dup_count,
            "render_batch: dropped duplicate (recipe_code, entity_id) pairs before processing"
        );
    }

    let candidates: Vec<&InsightCandidate> = if unique_candidates.len() > MAX_RENDER_BATCH_SIZE {
        warn!(
            input_len = unique_candidates.len(),
            limit = MAX_RENDER_BATCH_SIZE,
            "render_batch: input exceeds MAX_RENDER_BATCH_SIZE — truncating to limit"
        );
        unique_candidates
            .into_iter()
            .take(MAX_RENDER_BATCH_SIZE)
            .collect()
    } else {
        unique_candidates
    };
    let mut cards: Vec<InsightCard> = candidates.iter().map(|c| render_insight(c)).collect();
    rank_insights(&mut cards);
    cards
}

/// Render a batch of candidates using [`TitleGenerator`] for diverse titles.
///
/// Same semantics as [`render_batch`] but produces semantically diverse titles.
/// The [`TitleGenerator`] accumulates history across the batch so titles
/// become increasingly diverse as more cards are rendered.
pub fn render_batch_with_diversity(
    gen: &mut TitleGenerator,
    candidates: &[InsightCandidate],
    registry: Option<&EntityRegistry>,
) -> Vec<InsightCard> {
    // B295: Deduplicate by (recipe_code, entity_id) composite key before truncation
    let mut seen_keys = std::collections::HashSet::new();
    let mut unique_candidates = Vec::new();
    let mut dup_count = 0;
    for candidate in candidates {
        let key = (&candidate.recipe_code, &candidate.entity_id);
        if seen_keys.insert(key) {
            unique_candidates.push(candidate);
        } else {
            dup_count += 1;
        }
    }
    if dup_count > 0 {
        warn!(
            duplicate_count = dup_count,
            "render_batch_with_diversity: dropped duplicate (recipe_code, entity_id) pairs"
        );
    }

    let candidates: Vec<&InsightCandidate> = if unique_candidates.len() > MAX_RENDER_BATCH_SIZE {
        warn!(
            input_len = unique_candidates.len(),
            limit = MAX_RENDER_BATCH_SIZE,
            "render_batch_with_diversity: input exceeds MAX_RENDER_BATCH_SIZE — truncating"
        );
        unique_candidates
            .into_iter()
            .take(MAX_RENDER_BATCH_SIZE)
            .collect()
    } else {
        unique_candidates
    };
    let mut cards: Vec<InsightCard> = candidates
        .iter()
        .map(|c| render_insight_with_diversity(gen, c, registry))
        .collect();
    rank_insights(&mut cards);
    cards
}

/// Format a single insight card as a text block (for embedding in memos, emails, etc.).
pub fn format_card_text(card: &InsightCard) -> String {
    let mut lines = Vec::new();
    lines.push(format!("### {}", card.title));
    lines.push(format!(
        "**Severity:** {} | **Impact:** {} | **Confidence:** {:.0}% | **Information Gain:** {:.2} bits",
        card.severity,
        card.impact_label,
        card.confidence * 100.0,
        card_information_gain_bits(card)
    ));
    lines.push(String::new());
    lines.push(card.narrative.clone());
    lines.push(String::new());

    if !card.actions.is_empty() {
        lines.push("**Recommended Actions:**".to_string());
        for (i, action) in card.actions.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, action));
        }
        lines.push(String::new());
    }

    if !card.citations.is_empty() {
        lines.push("**Sources:**".to_string());
        for cite in &card.citations {
            let ts = cite
                .observed_at
                .map(|t| format!(", {}", t.format("%Y-%m-%d")))
                .unwrap_or_default();
            lines.push(format!(
                "[{}] {} ({}{})",
                cite.index, cite.source_url, cite.source_domain, ts
            ));
        }
    }

    lines.join("\n")
}

fn default_state_distribution() -> [f64; 3] {
    [0.70, 0.20, 0.10]
}

fn severity_target_state(severity: &str) -> usize {
    match severity.trim().to_lowercase().as_str() {
        "critical" | "high" => 2,
        "warning" | "medium" => 1,
        _ => 0,
    }
}

fn posterior_state_distribution(prior: [f64; 3], target_state: usize, confidence: f64) -> [f64; 3] {
    let confidence = confidence.clamp(0.0, 1.0);
    let mut posterior = [0.0; 3];
    for (idx, probability) in prior.iter().enumerate() {
        posterior[idx] = probability * (1.0 - confidence);
    }
    posterior[target_state] += confidence;
    let total: f64 = posterior.iter().sum();
    if total > 0.0 {
        for probability in &mut posterior {
            *probability /= total;
        }
    }
    posterior
}

fn shannon_entropy_bits(distribution: &[f64]) -> f64 {
    distribution
        .iter()
        .copied()
        .filter(|probability| *probability > 0.0)
        .map(|probability| -probability * probability.log2())
        .sum()
}

fn evidence_source_type(slot: &EvidenceSlot) -> String {
    if let Some(domain) = slot
        .source_domain
        .as_ref()
        .filter(|domain| !domain.trim().is_empty())
    {
        return domain.trim().to_lowercase();
    }
    if let Some(url) = slot
        .source_url
        .as_ref()
        .filter(|url| !url.trim().is_empty())
    {
        return extract_domain(url).to_lowercase();
    }
    slot.slot_name.trim().to_lowercase()
}

fn evidence_pair_normalized_mi(left: &str, right: &str) -> Option<f64> {
    let left_tokens = tokenize_signal(left);
    let right_tokens = tokenize_signal(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return None;
    }
    if left_tokens == right_tokens {
        return Some(1.0);
    }

    let mut vocabulary: Vec<String> = left_tokens
        .iter()
        .chain(right_tokens.iter())
        .cloned()
        .collect();
    vocabulary.sort();
    vocabulary.dedup();

    let token_to_id: HashMap<String, usize> = vocabulary
        .into_iter()
        .enumerate()
        .map(|(idx, token)| (token, idx + 1))
        .collect();

    let mut left_ids: Vec<f64> = left_tokens
        .into_iter()
        .filter_map(|token| token_to_id.get(&token).copied())
        .map(|idx| idx as f64)
        .collect();
    let mut right_ids: Vec<f64> = right_tokens
        .into_iter()
        .filter_map(|token| token_to_id.get(&token).copied())
        .map(|idx| idx as f64)
        .collect();

    left_ids.sort_by(|a, b| a.total_cmp(b));
    right_ids.sort_by(|a, b| a.total_cmp(b));
    let target_len = left_ids.len().max(right_ids.len());
    if target_len < 2 {
        return None;
    }
    left_ids.resize(target_len, 0.0);
    right_ids.resize(target_len, 0.0);

    Some(mutual_info::normalized_mi_adaptive(&left_ids, &right_ids).clamp(0.0, 1.0))
}

fn tokenize_signal(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            tokens.push(trimmed.to_lowercase());
        }
    }
    tokens
}

// ────────────────────────────────────────────
// Tests
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

    fn sample_evidence() -> Vec<EvidenceSlot> {
        vec![
            EvidenceSlot {
                slot_name: "company".to_string(),
                value: "Foxconn".to_string(),
                source_url: Some("https://www.foxconn.com/press/expansion".to_string()),
                source_domain: Some("foxconn.com".to_string()),
                observed_at: Some(Utc::now()),
            },
            EvidenceSlot {
                slot_name: "signal".to_string(),
                value: "3 new procurement job postings in 7 days".to_string(),
                source_url: Some("https://jobs.example.com/foxconn".to_string()),
                source_domain: None,
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "region".to_string(),
                value: "Tunisia".to_string(),
                source_url: None,
                source_domain: None,
                observed_at: None,
            },
        ]
    }

    fn sample_candidate() -> InsightCandidate {
        InsightCandidate {
            recipe_id: Uuid::new_v4(),
            recipe_code: "A001".to_string(),
            entity_id: Uuid::new_v4(),
            entity_name: "Foxconn".to_string(),
            confidence: 0.85,
            impact: 0.7,
            narrative_template: "{company} shows signs of a new sourcing cycle: {signal}. Region: {region}.".to_string(),
            action_template: "Register on {company} supplier portal\nSend capabilities deck to procurement team\nSchedule meeting within 14 days".to_string(),
            evidence: sample_evidence(),
            severity: "warning".to_string(),
            category: "demand".to_string(),
            region: Some("TN".to_string()),
        }
    }

    #[test]
    fn test_render_template_basic() {
        let mut slots = HashMap::new();
        slots.insert("name".to_string(), "Starz".to_string());
        slots.insert("target".to_string(), "Foxconn".to_string());
        let result = render_template("{name} targets {target} for engagement", &slots);
        assert_eq!(result, "Starz targets Foxconn for engagement");
    }

    #[test]
    fn test_render_template_unknown_slot() {
        let slots = HashMap::new();
        let result = render_template("Hello {unknown}", &slots);
        assert_eq!(result, "Hello {unknown}");
    }

    #[test]
    fn test_build_slot_map() {
        let evidence = sample_evidence();
        let map = build_slot_map(&evidence);
        assert_eq!(map.get("company").unwrap(), "Foxconn");
        assert_eq!(
            map.get("signal").unwrap(),
            "3 new procurement job postings in 7 days"
        );
        assert_eq!(map.get("region").unwrap(), "Tunisia");
    }

    #[test]
    fn test_build_slot_map_avoids_collisions_keeps_first() {
        let evidence = vec![
            EvidenceSlot {
                slot_name: "company".to_string(),
                value: "FirstValue".to_string(),
                source_url: None,
                source_domain: None,
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "company".to_string(),
                value: "SecondValue".to_string(),
                source_url: None,
                source_domain: None,
                observed_at: None,
            },
        ];
        let map = build_slot_map(&evidence);
        assert_eq!(map.get("company").unwrap(), "FirstValue");
        assert_eq!(map.get("evidence:company").unwrap(), "FirstValue");
    }

    #[test]
    fn test_build_slot_map_skips_invalid_slot_names() {
        let evidence = vec![
            EvidenceSlot {
                slot_name: " ".to_string(),
                value: "bad".to_string(),
                source_url: None,
                source_domain: None,
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "ok_name".to_string(),
                value: "good".to_string(),
                source_url: None,
                source_domain: None,
                observed_at: None,
            },
        ];
        let map = build_slot_map(&evidence);
        assert!(!map.contains_key(""));
        assert_eq!(map.get("ok_name").unwrap(), "good");
    }

    #[test]
    fn test_generate_title_uses_varied_templates() {
        // Test that titles use varied templates, not fixed format
        let title1 = generate_title("A001", "demand", "Foxconn");
        assert!(title1.starts_with("[A001]"));
        assert!(title1.contains("Foxconn"));
        // Should NOT be the old fixed format "Demand signal for Foxconn"
        // Instead should be one of the varied templates

        let title2 = generate_title("B012", "security", "Starz");
        assert!(title2.starts_with("[B012]"));
        assert!(title2.contains("Starz"));

        let title3 = generate_title("C003", "competitor", "Jabil");
        assert!(title3.starts_with("[C003]"));
        assert!(title3.contains("Jabil"));
    }

    #[test]
    fn test_generate_title_templates_rotate() {
        // Reset counter for deterministic test
        // Generate multiple titles for same category - should vary
        let titles: Vec<String> = (0..6)
            .map(|i| generate_title_with_index("X", "demand", "TestCorp", i))
            .collect();

        // All should contain entity name
        for title in &titles {
            assert!(title.contains("TestCorp"));
        }

        // Titles should differ (rotation working)
        let unique_titles: std::collections::HashSet<_> = titles.iter().collect();
        assert!(unique_titles.len() > 1, "Title templates should rotate");
    }

    #[test]
    fn test_generate_title_unknown_category_fallback() {
        let title = generate_title("Z999", "unknown_cat", "SomeEntity");
        // Unknown category should use fallback format
        assert_eq!(title, "[Z999] unknown_cat for SomeEntity");
    }

    #[test]
    fn test_impact_label() {
        assert_eq!(impact_label(0.9), "Critical");
        assert_eq!(impact_label(0.8), "Critical");
        assert_eq!(impact_label(0.7), "High");
        assert_eq!(impact_label(0.5), "Medium");
        assert_eq!(impact_label(0.3), "Low");
        assert_eq!(impact_label(0.1), "Info");
    }

    #[test]
    fn test_impact_label_consistent_with_severity_mapping() {
        // High impact should not map to "Info" severity normalization path.
        let candidate = InsightCandidate {
            impact: 0.85,
            severity: "high".to_string(),
            ..sample_candidate()
        };
        let card = render_insight(&candidate);
        assert_eq!(card.impact_label, "Critical");
        assert_eq!(card.severity, "warning");
    }

    #[test]
    fn test_priority_score() {
        let score = priority_score(0.8, 0.9, "critical");
        assert!((score - 0.8 * 0.9 * 2.0).abs() < 1e-10);

        let score_warn = priority_score(0.5, 0.7, "warning");
        assert!((score_warn - 0.5 * 0.7 * 1.5).abs() < 1e-10);

        let score_info = priority_score(0.3, 0.6, "info");
        assert!((score_info - 0.3 * 0.6 * 1.0).abs() < 1e-10);

        // Regression: "high" and "medium" must get proper urgency, not default 1.0
        let score_high = priority_score(0.5, 0.7, "high");
        assert!((score_high - 0.5 * 0.7 * 1.5).abs() < 1e-10);

        let score_med = priority_score(0.5, 0.7, "medium");
        assert!((score_med - 0.5 * 0.7 * 1.25).abs() < 1e-10);

        let score_low = priority_score(0.5, 0.7, "low");
        assert!((score_low - 0.5 * 0.7 * 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_extract_citations() {
        let evidence = sample_evidence();
        let citations = extract_citations(&evidence);
        assert_eq!(citations.len(), 2); // only 2 have source_url
        assert_eq!(citations[0].index, 1);
        assert_eq!(citations[0].source_domain, "foxconn.com");
        assert_eq!(citations[1].index, 2);
        assert_eq!(citations[1].source_domain, "jobs.example.com");
    }

    #[test]
    fn test_extract_citations_rejects_non_http_and_malformed_urls() {
        let evidence = vec![
            EvidenceSlot {
                slot_name: "a".to_string(),
                value: "v".to_string(),
                source_url: Some("ftp://example.com/file".to_string()),
                source_domain: None,
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "b".to_string(),
                value: "v".to_string(),
                source_url: Some("not a url".to_string()),
                source_domain: None,
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "c".to_string(),
                value: "v".to_string(),
                source_url: Some("https://valid.example/path".to_string()),
                source_domain: None,
                observed_at: None,
            },
        ];
        let citations = extract_citations(&evidence);
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].source_domain, "valid.example");
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://www.foxconn.com/press"),
            "www.foxconn.com"
        );
        assert_eq!(
            extract_domain("http://example.com:8080/path"),
            "example.com"
        );
        assert_eq!(extract_domain("example.com/foo"), "example.com");
    }

    #[test]
    fn test_parse_actions() {
        let mut slots = HashMap::new();
        slots.insert("company".to_string(), "Foxconn".to_string());
        let actions = parse_actions("Register on {company} portal\nSend deck; Follow up", &slots);
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0], "Register on Foxconn portal");
        assert_eq!(actions[1], "Send deck");
        assert_eq!(actions[2], "Follow up");
    }

    #[test]
    fn test_append_citation_refs() {
        assert_eq!(append_citation_refs("Some text", 0), "Some text");
        assert_eq!(append_citation_refs("Some text", 2), "Some text [1][2]");
        assert_eq!(append_citation_refs("Some text ", 3), "Some text [1][2][3]");
    }

    #[test]
    fn test_render_insight_full() {
        let candidate = sample_candidate();
        let card = render_insight(&candidate);

        assert_eq!(card.recipe_code, "A001");
        assert_eq!(card.entity_name, "Foxconn");
        assert_eq!(card.severity, "warning");
        assert_eq!(card.category, "demand");
        assert_eq!(card.impact_label, "High");
        assert!(card
            .narrative
            .contains("Foxconn shows signs of a new sourcing cycle"));
        assert!(card
            .narrative
            .contains("3 new procurement job postings in 7 days"));
        assert!(card.narrative.contains("[1][2]")); // citations appended
        assert_eq!(card.actions.len(), 3);
        assert!(card.actions[0].contains("Register on Foxconn supplier portal"));
        assert_eq!(card.citations.len(), 2);
        assert_eq!(card.region, Some("TN".to_string()));
        // With three independent evidence types, the support multiplier stays neutral.
        assert!((card.priority_score - 0.7 * 0.85 * 1.5).abs() < 1e-10);
        assert!(card_information_gain_bits(&card) > 0.1);
    }

    #[test]
    fn test_rank_insights() {
        let c1 = InsightCandidate {
            recipe_code: "A001".to_string(),
            impact: 0.3,
            confidence: 0.2,
            severity: "info".to_string(),
            ..sample_candidate()
        };
        let c2 = InsightCandidate {
            recipe_code: "A002".to_string(),
            impact: 0.9,
            confidence: 0.9,
            severity: "critical".to_string(),
            ..sample_candidate()
        };
        let c3 = InsightCandidate {
            recipe_code: "A003".to_string(),
            impact: 0.6,
            confidence: 0.7,
            severity: "warning".to_string(),
            ..sample_candidate()
        };
        let cards = render_batch(&[c1, c2, c3]);
        // Should be ranked by priority_score descending
        assert_eq!(cards[0].recipe_code, "A002"); // highest
        assert_eq!(cards[1].recipe_code, "A003");
        assert_eq!(cards[2].recipe_code, "A001"); // lowest
    }

    #[test]
    fn test_filter_by_confidence() {
        let cards = render_batch(&[sample_candidate()]);
        let filtered = filter_by_confidence(&cards, 0.9);
        assert!(filtered.is_empty()); // confidence is 0.85, below 0.9

        let filtered2 = filter_by_confidence(&cards, 0.8);
        assert_eq!(filtered2.len(), 1);
    }

    #[test]
    fn test_filter_by_category() {
        let cards = render_batch(&[sample_candidate()]);
        let demand = filter_by_category(&cards, "demand");
        assert_eq!(demand.len(), 1);
        let security = filter_by_category(&cards, "security");
        assert!(security.is_empty());
    }

    #[test]
    fn test_group_by_category() {
        let c1 = sample_candidate();
        let c2 = InsightCandidate {
            category: "security".to_string(),
            recipe_code: "B001".to_string(),
            ..sample_candidate()
        };
        let cards = render_batch(&[c1, c2]);
        let groups = group_by_category(&cards);
        assert!(groups.contains_key("demand"));
        assert!(groups.contains_key("security"));
        assert_eq!(groups["demand"].len(), 1);
        assert_eq!(groups["security"].len(), 1);
    }

    #[test]
    fn test_group_by_region() {
        let c1 = sample_candidate();
        let c2 = InsightCandidate {
            region: Some("MA".to_string()),
            recipe_code: "A002".to_string(),
            ..sample_candidate()
        };
        let c3 = InsightCandidate {
            region: None,
            recipe_code: "A003".to_string(),
            ..sample_candidate()
        };
        let cards = render_batch(&[c1, c2, c3]);
        let groups = group_by_region(&cards);
        assert!(groups.contains_key("TN"));
        assert!(groups.contains_key("MA"));
        assert!(groups.contains_key("global"));
    }

    #[test]
    fn test_format_card_text() {
        let card = render_insight(&sample_candidate());
        let text = format_card_text(&card);
        assert!(text.contains("### [A001]"));
        assert!(text.contains("**Severity:** warning"));
        assert!(text.contains("**Impact:** High"));
        assert!(text.contains("**Confidence:** 85%"));
        assert!(text.contains("**Information Gain:**"));
        assert!(text.contains("**Recommended Actions:**"));
        assert!(text.contains("1. Register on Foxconn supplier portal"));
        assert!(text.contains("**Sources:**"));
        assert!(text.contains("foxconn.com"));
    }

    #[test]
    fn test_information_gain_increases_with_confidence() {
        let low = information_gain_bits(0.35, "warning");
        let high = information_gain_bits(0.85, "warning");
        assert!(
            high > low,
            "higher confidence should increase information gain"
        );
        assert!(
            high > 0.1,
            "high-confidence signals should clear the noise floor"
        );
    }

    #[test]
    fn test_redundant_evidence_reduces_priority_score() {
        let repeated = InsightCandidate {
            evidence: vec![
                EvidenceSlot {
                    slot_name: "signal_a".to_string(),
                    value: "supplier expansion capacity increase".to_string(),
                    source_url: Some("https://same.example/a".to_string()),
                    source_domain: Some("same.example".to_string()),
                    observed_at: None,
                },
                EvidenceSlot {
                    slot_name: "signal_b".to_string(),
                    value: "supplier expansion capacity increase".to_string(),
                    source_url: Some("https://same.example/b".to_string()),
                    source_domain: Some("same.example".to_string()),
                    observed_at: None,
                },
                EvidenceSlot {
                    slot_name: "signal_c".to_string(),
                    value: "supplier expansion capacity increase".to_string(),
                    source_url: Some("https://same.example/c".to_string()),
                    source_domain: Some("same.example".to_string()),
                    observed_at: None,
                },
            ],
            ..sample_candidate()
        };
        let diversified = sample_candidate();
        let repeated_card = render_insight(&repeated);
        let diversified_card = render_insight(&diversified);
        assert!(repeated_card.priority_score < diversified_card.priority_score);
        assert!(
            evidence_diversity_score(&repeated.evidence)
                < evidence_diversity_score(&diversified.evidence)
        );
    }

    #[test]
    fn test_pairwise_evidence_redundancy_detects_duplicate_signals() {
        let evidence = vec![
            EvidenceSlot {
                slot_name: "a".to_string(),
                value: "new procurement jobs in mexico".to_string(),
                source_url: None,
                source_domain: Some("jobs.example.com".to_string()),
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "b".to_string(),
                value: "new procurement jobs in mexico".to_string(),
                source_url: None,
                source_domain: Some("mirror.example.com".to_string()),
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "c".to_string(),
                value: "export permit delay in tunisia".to_string(),
                source_url: None,
                source_domain: Some("permits.example.com".to_string()),
                observed_at: None,
            },
        ];
        let links = pairwise_evidence_redundancy(&evidence);
        assert!(!links.is_empty());
        assert!(
            links[0].normalized_mutual_information > 0.8,
            "expected duplicated signals to be redundant"
        );
    }

    #[test]
    fn insight_information_gain_ordering() {
        let flip = information_gain_bits(0.95, "critical");
        let confirm = information_gain_bits(0.55, "info");
        assert!(flip > confirm);
    }

    #[test]
    fn evidence_redundancy_detection() {
        let evidence = vec![
            EvidenceSlot {
                slot_name: "a".to_string(),
                value: "same rss feed duplicate signal".to_string(),
                source_url: None,
                source_domain: Some("rss.example.com".to_string()),
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "b".to_string(),
                value: "same rss feed duplicate signal".to_string(),
                source_url: None,
                source_domain: Some("rss.example.com".to_string()),
                observed_at: None,
            },
            EvidenceSlot {
                slot_name: "c".to_string(),
                value: "same rss feed duplicate signal".to_string(),
                source_url: None,
                source_domain: Some("rss.example.com".to_string()),
                observed_at: None,
            },
        ];
        let links = pairwise_evidence_redundancy(&evidence);
        assert!(links
            .iter()
            .any(|link| link.normalized_mutual_information > 0.8));
    }

    #[test]
    fn test_rank_insights_prefers_higher_information_gain() {
        use chrono::Utc;
        let entity_a = Uuid::new_v4();
        let entity_b = Uuid::new_v4();
        let mut cards = vec![
            InsightCard {
                id: Uuid::new_v4(),
                recipe_code: "LOW-IG".to_string(),
                entity_id: entity_a,
                entity_name: "Entity A".to_string(),
                severity: "warning".to_string(),
                category: "demand".to_string(),
                title: "Low IG".to_string(),
                narrative: String::new(),
                actions: vec![],
                citations: vec![],
                confidence: 0.45,
                impact: 0.7,
                impact_label: "High".to_string(),
                priority_score: 0.75,
                region: None,
                rendered_at: Utc::now(),
            },
            InsightCard {
                id: Uuid::new_v4(),
                recipe_code: "HIGH-IG".to_string(),
                entity_id: entity_b,
                entity_name: "Entity B".to_string(),
                severity: "warning".to_string(),
                category: "demand".to_string(),
                title: "High IG".to_string(),
                narrative: String::new(),
                actions: vec![],
                citations: vec![],
                confidence: 0.9,
                impact: 0.5,
                impact_label: "Medium".to_string(),
                priority_score: 0.70,
                region: None,
                rendered_at: Utc::now(),
            },
        ];
        rank_insights(&mut cards);
        assert_eq!(cards[0].recipe_code, "HIGH-IG");
    }

    // ── B184: generate_title with unknown category ──

    #[test]
    fn test_generate_title_unknown_category() {
        let title = generate_title("Z999", "unknown_cat", "SomeEntity");
        // unknown category should be passed through as-is
        assert_eq!(title, "[Z999] unknown_cat for SomeEntity");
    }

    #[test]
    fn test_generate_title_empty_category() {
        let title = generate_title("X", "", "Ent");
        assert_eq!(title, "[X]  for Ent");
    }

    // ── B185: render_template with nested braces ──

    #[test]
    fn test_render_template_nested_braces() {
        let mut slots = HashMap::new();
        slots.insert("x".to_string(), "val".to_string());
        // Nested `{` should stop the placeholder scan
        let result = render_template("{{x}} and {x}", &slots);
        // First `{` hits another `{`, is emitted literally; then `x}` is remainder
        assert!(
            result.contains("val"),
            "Outer {{x}} should still be replaced: {}",
            result
        );
    }

    #[test]
    fn test_render_template_unclosed_brace() {
        let slots = HashMap::new();
        let result = render_template("start {unclosed", &slots);
        assert_eq!(result, "start {unclosed");
    }

    // ── B186: parse_actions bounded ──

    #[test]
    fn test_parse_actions_bounded() {
        let huge_template: String = (0..200)
            .map(|i| format!("Action {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let actions = parse_actions(&huge_template, &HashMap::new());
        assert!(
            actions.len() <= 50,
            "parse_actions should cap at MAX_ACTIONS=50, got {}",
            actions.len()
        );
    }

    // ── B187: extract_domain with malformed URLs ──

    #[test]
    fn test_extract_domain_malformed() {
        assert_eq!(extract_domain(""), "unknown");
        assert_eq!(extract_domain("   "), "unknown");
        assert_eq!(extract_domain("https://"), "unknown");
        assert_eq!(extract_domain("not-a-url"), "not-a-url");
        assert_eq!(extract_domain("?q=abc"), "unknown");
    }

    #[test]
    fn test_append_citation_refs_replaces_existing_trailing_refs() {
        let narrative = "Alert observed [1][2]";
        let updated = append_citation_refs(narrative, 3);
        assert_eq!(updated, "Alert observed [1][2][3]");
    }

    #[test]
    fn test_append_citation_refs_count_matches_requested() {
        let updated = append_citation_refs("Signal detected", 4);
        assert_eq!(trailing_citation_ref_count(&updated), 4);
    }

    // ── B188: citation ordering stability ──

    #[test]
    fn test_citation_ordering_stable() {
        let evidence = sample_evidence();
        let c1 = extract_citations(&evidence);
        let c2 = extract_citations(&evidence);
        assert_eq!(c1.len(), c2.len());
        for (a, b) in c1.iter().zip(c2.iter()) {
            assert_eq!(a.index, b.index);
            assert_eq!(a.source_url, b.source_url);
        }
    }

    // ── B189: empty evidence list handling ──

    #[test]
    fn test_render_insight_empty_evidence() {
        let mut candidate = sample_candidate();
        candidate.evidence = vec![];
        let card = render_insight(&candidate);
        assert!(card.citations.is_empty());
        // Template placeholders left as-is since no slots
        assert!(card.narrative.contains("{company}") || card.narrative.contains("{signal}"));
    }

    // ── B190: priority_score with unknown severity ──

    #[test]
    fn test_priority_score_unknown_severity() {
        let score = priority_score(0.5, 0.7, "banana");
        // Unknown severity should use default multiplier 1.0
        assert!((score - 0.5 * 0.7 * 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_priority_score_empty_severity() {
        let score = priority_score(0.5, 0.7, "");
        assert!((score - 0.5 * 0.7 * 1.0).abs() < 1e-10);
    }

    // ── B280: render_template output cap (runaway-expansion guard) ────────────

    #[test]
    fn test_render_template_small_slot_expansion_is_accurate() {
        // Slot value is larger than its placeholder but within the 256 KB cap
        let mut slots = HashMap::new();
        slots.insert("val".to_string(), "x".repeat(1024));
        let result = render_template("{val}", &slots);
        assert_eq!(result.len(), 1024);
        assert!(result.chars().all(|c| c == 'x'));
    }

    #[test]
    fn test_render_template_exceeding_cap_emits_truncation_marker() {
        // A single slot value of 300 KB should be truncated and end with …
        let mut slots = HashMap::new();
        slots.insert("big".to_string(), "a".repeat(300 * 1024));
        let result = render_template("{big}", &slots);
        // Output must be ≤ MAX cap + a few bytes for the … marker
        assert!(
            result.len() <= MAX_TEMPLATE_OUTPUT_BYTES + 4,
            "output was {} bytes, expected <= {}",
            result.len(),
            MAX_TEMPLATE_OUTPUT_BYTES + 4
        );
        assert!(result.ends_with('…'), "truncated output must end with …");
    }

    #[test]
    fn test_render_template_many_small_slots_stay_within_cap() {
        // 1000 slots of 300 bytes each = 300 KB total > 256 KB cap
        let mut slots = HashMap::new();
        let template: String = (0..1000).map(|i| format!("{{{i}}}")).collect();
        for i in 0..1000usize {
            slots.insert(i.to_string(), "z".repeat(300));
        }
        let result = render_template(&template, &slots);
        assert!(
            result.len() <= MAX_TEMPLATE_OUTPUT_BYTES + 4,
            "output was {} bytes, should be capped",
            result.len()
        );
    }

    #[test]
    fn test_render_template_capacity_hint_matches_output_no_reallocation() {
        // Regression: capacity should equal or exceed result length so no re-alloc
        let mut slots = HashMap::new();
        slots.insert("company".to_string(), "Acme Corp Ltd".to_string());
        slots.insert("signal".to_string(), "procurement spike".to_string());
        let template = "Alert for {company}: {signal} detected.";
        let result = render_template(template, &slots);
        // Exact string match, no truncation
        assert_eq!(
            result,
            "Alert for Acme Corp Ltd: procurement spike detected."
        );
        assert!(result.len() <= template.len() + 13 + 17 + 10); // upper bound check
    }

    // B286: render_batch must not exceed MAX_RENDER_BATCH_SIZE
    #[test]
    fn test_render_batch_truncates_at_max_batch_size() {
        let over_limit = MAX_RENDER_BATCH_SIZE + 5;
        let candidates: Vec<InsightCandidate> = (0..over_limit)
            .map(|i| InsightCandidate {
                recipe_id: Uuid::new_v4(),
                recipe_code: format!("R{i:04}"),
                entity_id: Uuid::new_v4(),
                entity_name: format!("Entity{i}"),
                confidence: 0.8,
                impact: 0.7,
                narrative_template: "Alert: {signal} detected.".to_string(),
                action_template: "Investigate immediately.".to_string(),
                evidence: vec![],
                severity: "warning".to_string(),
                category: "supply_chain".to_string(),
                region: None,
            })
            .collect();
        let cards = render_batch(&candidates);
        assert!(
            cards.len() <= MAX_RENDER_BATCH_SIZE,
            "render_batch returned {} cards; expected ≤ {MAX_RENDER_BATCH_SIZE}",
            cards.len()
        );
    }

    // B286: render_batch at exactly the limit is not truncated
    #[test]
    fn test_render_batch_at_exact_limit_passes_all() {
        let candidates: Vec<InsightCandidate> = (0..MAX_RENDER_BATCH_SIZE)
            .map(|i| InsightCandidate {
                recipe_id: Uuid::new_v4(),
                recipe_code: format!("R{i:04}"),
                entity_id: Uuid::new_v4(),
                entity_name: format!("Ent{i}"),
                confidence: 0.5,
                impact: 0.5,
                narrative_template: "Alert: threshold crossed.".to_string(),
                action_template: "Review the metrics.".to_string(),
                evidence: vec![],
                severity: "info".to_string(),
                category: "finance".to_string(),
                region: None,
            })
            .collect();
        let cards = render_batch(&candidates);
        assert_eq!(
            cards.len(),
            MAX_RENDER_BATCH_SIZE,
            "render_batch at exact limit must return all cards"
        );
    }

    // ── B287: empty input tests ──

    #[test]
    fn test_render_batch_empty_input_returns_empty_vec() {
        // render_batch([]) must return [] without panic
        let cards = render_batch(&[]);
        assert!(cards.is_empty(), "render_batch([]) must return empty vec");
    }

    #[test]
    fn test_render_template_empty_template_returns_empty() {
        // An empty template string must produce an empty result
        let result = render_template("", &HashMap::new());
        assert!(
            result.is_empty(),
            "render_template('', {{}}) must return empty string"
        );
    }

    #[test]
    fn test_render_template_empty_slot_map_preserves_placeholders() {
        // Non-empty template, but slot map is empty → placeholders are kept verbatim
        let result = render_template("Hello {name} world", &HashMap::new());
        assert_eq!(result, "Hello {name} world");
    }

    // B292: rank_insights deterministic tie-breaking
    #[test]
    fn test_rank_insights_equal_scores_stable_by_recipe_code() {
        use chrono::Utc;
        use uuid::Uuid;
        let make_card = |recipe_code: &str, entity_id: Uuid, score: f64| InsightCard {
            id: Uuid::new_v4(),
            recipe_code: recipe_code.to_string(),
            entity_id,
            entity_name: "Test Entity".to_string(),
            region: None,
            severity: "medium".to_string(),
            category: "test".to_string(),
            title: format!("Title {recipe_code}"),
            narrative: String::new(),
            actions: vec![],
            citations: vec![],
            confidence: 0.8,
            impact: 0.8,
            impact_label: "medium".to_string(),
            priority_score: score,
            rendered_at: Utc::now(),
        };
        let id1 = Uuid::nil();
        let id2 = Uuid::new_v4();
        let mut cards = vec![
            make_card("ZZZ-recipe", id1, 0.5),
            make_card("AAA-recipe", id2, 0.5), // same score, should sort before ZZZ
        ];
        rank_insights(&mut cards);
        assert_eq!(
            cards[0].recipe_code, "AAA-recipe",
            "when priority scores are equal, lower recipe_code must come first"
        );
        // Determinism: calling again produces same order
        let mut cards2 = vec![
            make_card("ZZZ-recipe", id1, 0.5),
            make_card("AAA-recipe", id2, 0.5),
        ];
        rank_insights(&mut cards2);
        assert_eq!(cards[0].recipe_code, cards2[0].recipe_code);
    }

    #[test]
    fn test_render_batch_drops_duplicate_recipe_entity_pairs() {
        // B295: Verify that duplicate (recipe_code, entity_id) pairs are dropped
        use uuid::Uuid;
        let dup_eid = Uuid::new_v4();
        let unique_eid = Uuid::new_v4();
        let recipe_code = "TEST-001";
        let recipe_id = Uuid::new_v4();

        let make_candidate = |entity_id: Uuid| InsightCandidate {
            recipe_id,
            recipe_code: recipe_code.to_string(),
            entity_id,
            entity_name: "Test Entity".to_string(),
            confidence: 0.8,
            impact: 0.7,
            narrative_template: "Test narrative".to_string(),
            action_template: "Test action".to_string(),
            evidence: vec![],
            severity: "medium".to_string(),
            category: "test".to_string(),
            region: None,
        };

        let candidates = vec![
            make_candidate(dup_eid),
            make_candidate(dup_eid), // duplicate pair
            make_candidate(unique_eid),
        ];

        let result = render_batch(&candidates);
        // Should have 2 entries: first occurrence of dup + unique
        assert_eq!(
            result.len(),
            2,
            "render_batch must drop duplicate (recipe_code, entity_id) pairs"
        );
        // Verify both unique pairs are present
        let has_dup = result.iter().any(|c| c.entity_id == dup_eid);
        let has_unique = result.iter().any(|c| c.entity_id == unique_eid);
        assert!(has_dup, "first occurrence of duplicate must be kept");
        assert!(has_unique, "unique pair must be kept");
    }

    #[test]
    fn test_extract_citations_capped_at_max_external_links() {
        let evidence: Vec<EvidenceSlot> = (0..40)
            .map(|i| EvidenceSlot {
                slot_name: format!("slot_{i}"),
                value: format!("value {i}"),
                source_url: Some(format!("https://example.com/{i}")),
                source_domain: Some("example.com".to_string()),
                observed_at: None,
            })
            .collect();

        let citations = extract_citations(&evidence);
        assert_eq!(citations.len(), 25, "citations must be capped per record");
        assert_eq!(citations[0].index, 1);
        assert_eq!(citations[24].index, 25);
    }

    #[test]
    fn test_render_insight_uses_fallback_action_when_template_empty() {
        let candidate = InsightCandidate {
            recipe_id: Uuid::new_v4(),
            recipe_code: "R-FALLBACK".to_string(),
            entity_id: Uuid::new_v4(),
            entity_name: "Entity".to_string(),
            confidence: 0.8,
            impact: 0.7,
            narrative_template: "alert".to_string(),
            action_template: "   ; ;\n".to_string(),
            evidence: vec![],
            severity: "info".to_string(),
            category: "ops".to_string(),
            region: None,
        };
        let card = render_insight(&candidate);
        assert_eq!(card.actions.len(), 1);
        assert!(card.actions[0].contains("Review evidence"));
    }

    #[test]
    fn test_render_insight_caps_actions_per_card() {
        let action_template = (0..120)
            .map(|i| format!("Action {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let candidate = InsightCandidate {
            recipe_id: Uuid::new_v4(),
            recipe_code: "R-ACTIONS".to_string(),
            entity_id: Uuid::new_v4(),
            entity_name: "Entity".to_string(),
            confidence: 0.8,
            impact: 0.7,
            narrative_template: "alert".to_string(),
            action_template,
            evidence: vec![],
            severity: "warning".to_string(),
            category: "ops".to_string(),
            region: None,
        };
        let card = render_insight(&candidate);
        assert!(card.actions.len() <= 50);
    }

    #[test]
    fn test_parse_actions_with_semicolons() {
        let actions = parse_actions(
            "Investigate supplier; Notify legal ;  Escalate to CISO",
            &HashMap::new(),
        );
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0], "Investigate supplier");
        assert_eq!(actions[1], "Notify legal");
        assert_eq!(actions[2], "Escalate to CISO");
    }

    #[test]
    fn test_render_insight_unknown_severity_defaults_to_info() {
        let mut candidate = sample_candidate();
        candidate.severity = "SEVERE".to_string();
        let card = render_insight(&candidate);
        assert_eq!(card.severity, "info");
    }

    #[test]
    fn test_render_insight_caps_evidence_slots() {
        let evidence: Vec<EvidenceSlot> = (0..130)
            .map(|i| EvidenceSlot {
                slot_name: format!("k{i}"),
                value: format!("v{i}"),
                source_url: Some(format!("https://example.com/{i}")),
                source_domain: Some("example.com".to_string()),
                observed_at: None,
            })
            .collect();
        let candidate = InsightCandidate {
            recipe_id: Uuid::new_v4(),
            recipe_code: "R-EVID".to_string(),
            entity_id: Uuid::new_v4(),
            entity_name: "Entity".to_string(),
            confidence: 0.9,
            impact: 0.9,
            narrative_template: "{k129}".to_string(),
            action_template: "do x".to_string(),
            evidence,
            severity: "critical".to_string(),
            category: "security".to_string(),
            region: None,
        };
        let card = render_insight(&candidate);
        assert!(card.narrative.starts_with("{k129} "));
        assert!(card.narrative.contains("[25]"));
    }

    // ── Template placeholder validation tests ──

    #[test]
    fn test_title_templates_all_contain_entity_placeholder() {
        for (category, template) in TITLE_TEMPLATES {
            assert!(
                template.contains("{entity}"),
                "Template for category '{}' is missing {{entity}} placeholder: '{}'",
                category,
                template
            );
        }
    }

    #[test]
    fn test_title_templates_categories_all_have_at_least_two_variants() {
        let mut category_counts: HashMap<&str, usize> = HashMap::new();
        for (cat, _) in TITLE_TEMPLATES {
            *category_counts.entry(cat).or_insert(0) += 1;
        }
        for (cat, count) in &category_counts {
            assert!(
                *count >= 2,
                "Category '{}' has only {} template(s), need at least 2 for variety",
                cat,
                count
            );
        }
    }

    #[test]
    fn test_title_templates_no_duplicate_templates() {
        let mut seen = std::collections::HashSet::new();
        for (cat, template) in TITLE_TEMPLATES {
            let key = format!("{}:{}", cat, template);
            assert!(
                seen.insert(key.clone()),
                "Duplicate template found: '{}'",
                key
            );
        }
    }
}
