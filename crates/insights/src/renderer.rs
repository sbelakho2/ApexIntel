//! Narrative rendering from recipes + evidence.
//!
//! Takes InsightCandidate data (recipe template, evidence slots, entity context)
//! and produces human-readable insight cards with narratives, actions, and citations.

use apex_core::validation::normalize_url;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Generate title from recipe code and entity name.
///
/// Format: "[CODE] Category insight for Entity"
pub fn generate_title(recipe_code: &str, category: &str, entity_name: &str) -> String {
    let cat_label = match category {
        "demand" => "Demand signal",
        "supply_chain" => "Supply chain alert",
        "competitor" => "Competitor intelligence",
        "security" => "Security warning",
        "poi" => "Stakeholder insight",
        "regulatory" => "Regulatory change",
        "commodity" => "Commodity alert",
        "logistics" => "Logistics signal",
        other => other,
    };
    format!("[{}] {} for {}", recipe_code, cat_label, entity_name)
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

static RE_TRAILING_CITATION_REFS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\s*\[\d+\])+\s*$").unwrap());
static RE_CITATION_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\d+\]").unwrap());

/// Parse action template into a list of actions.
/// Actions are separated by newlines or semicolons. Empty lines are skipped.
/// B186: capped at MAX_ACTIONS to prevent huge lists.
pub fn parse_actions(action_template: &str, slots: &HashMap<String, String>) -> Vec<String> {
    let rendered = render_template(action_template, slots);
    rendered
        .split(|c: char| c == '\n' || c == ';')
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
    let mut narrative = append_citation_refs(narrative_raw.trim(), citations.len());
    if trailing_citation_ref_count(&narrative) != citations.len() {
        warn!(
            citation_count = citations.len(),
            trailing_refs = trailing_citation_ref_count(&narrative),
            "citation_reference_count_mismatch"
        );
        narrative = append_citation_refs(&narrative, citations.len());
    }
    let title = generate_title(
        &candidate.recipe_code,
        &candidate.category,
        &candidate.entity_name,
    );
    let severity = normalize_card_severity(&candidate.severity);
    let score = priority_score(candidate.impact, candidate.confidence, &severity);

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
        // Primary: priority score DESC (higher is more important)
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

/// Format a single insight card as a text block (for embedding in memos, emails, etc.).
pub fn format_card_text(card: &InsightCard) -> String {
    let mut lines = Vec::new();
    lines.push(format!("### {}", card.title));
    lines.push(format!(
        "**Severity:** {} | **Impact:** {} | **Confidence:** {:.0}%",
        card.severity,
        card.impact_label,
        card.confidence * 100.0
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

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
    fn test_generate_title() {
        assert_eq!(
            generate_title("A001", "demand", "Foxconn"),
            "[A001] Demand signal for Foxconn"
        );
        assert_eq!(
            generate_title("B012", "security", "Starz"),
            "[B012] Security warning for Starz"
        );
        assert_eq!(
            generate_title("C003", "competitor", "Jabil"),
            "[C003] Competitor intelligence for Jabil"
        );
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
        // priority = 0.7 * 0.85 * 1.5 (warning)
        assert!((card.priority_score - 0.7 * 0.85 * 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_rank_insights() {
        let c1 = InsightCandidate {
            recipe_code: "A001".to_string(),
            impact: 0.3,
            confidence: 0.5,
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
        assert!(text.contains("**Recommended Actions:**"));
        assert!(text.contains("1. Register on Foxconn supplier portal"));
        assert!(text.contains("**Sources:**"));
        assert!(text.contains("foxconn.com"));
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
}
