//! Narrative rendering from recipes + evidence.
//!
//! Takes InsightCandidate data (recipe template, evidence slots, entity context)
//! and produces human-readable insight cards with narratives, actions, and citations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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

/// Render a template string by replacing `{slot_name}` placeholders with evidence values.
/// Unknown placeholders are left as-is.
pub fn render_template(template: &str, slots: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, val) in slots {
        let placeholder = format!("{{{}}}", key);
        result = result.replace(&placeholder, val);
    }
    result
}

/// Build slot map from evidence slots.
pub fn build_slot_map(evidence: &[EvidenceSlot]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for slot in evidence {
        map.insert(slot.slot_name.clone(), slot.value.clone());
    }
    map
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
        "warning" => 1.5,
        "info" => 1.0,
        _ => 1.0,
    };
    impact * confidence * urgency
}

/// Extract citations from evidence slots that have source URLs.
pub fn extract_citations(evidence: &[EvidenceSlot]) -> Vec<Citation> {
    let mut citations = Vec::new();
    for slot in evidence {
        if let Some(ref url) = slot.source_url {
            if !url.is_empty() {
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

/// Extract domain from a URL string (best-effort).
pub fn extract_domain(url: &str) -> String {
    // Strip scheme
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    // Take up to first /
    let domain = without_scheme.split('/').next().unwrap_or(without_scheme);
    // Strip port
    let domain = domain.split(':').next().unwrap_or(domain);
    domain.to_string()
}

/// Parse action template into a list of actions.
/// Actions are separated by newlines or semicolons. Empty lines are skipped.
pub fn parse_actions(action_template: &str, slots: &HashMap<String, String>) -> Vec<String> {
    let rendered = render_template(action_template, slots);
    rendered
        .split(|c: char| c == '\n' || c == ';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Add citation references to narrative text.
/// Appends "[1][2]..." at the end referencing all citations.
pub fn append_citation_refs(narrative: &str, citation_count: usize) -> String {
    if citation_count == 0 {
        return narrative.to_string();
    }
    let refs: Vec<String> = (1..=citation_count).map(|i| format!("[{}]", i)).collect();
    format!("{} {}", narrative.trim(), refs.join(""))
}

// ────────────────────────────────────────────
// Main render function
// ────────────────────────────────────────────

/// Render an InsightCandidate into a full InsightCard.
pub fn render_insight(candidate: &InsightCandidate) -> InsightCard {
    let slots = build_slot_map(&candidate.evidence);
    let narrative_raw = render_template(&candidate.narrative_template, &slots);
    let actions = parse_actions(&candidate.action_template, &slots);
    let citations = extract_citations(&candidate.evidence);
    let narrative = append_citation_refs(&narrative_raw, citations.len());
    let title = generate_title(
        &candidate.recipe_code,
        &candidate.category,
        &candidate.entity_name,
    );
    let score = priority_score(candidate.impact, candidate.confidence, &candidate.severity);

    InsightCard {
        id: Uuid::new_v4(),
        recipe_code: candidate.recipe_code.clone(),
        entity_id: candidate.entity_id,
        entity_name: candidate.entity_name.clone(),
        severity: candidate.severity.clone(),
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
        b.priority_score
            .partial_cmp(&a.priority_score)
            .unwrap_or(std::cmp::Ordering::Equal)
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
    cards
        .iter()
        .filter(|c| c.category == category)
        .collect()
}

/// Group insights by category.
pub fn group_by_category(cards: &[InsightCard]) -> HashMap<String, Vec<&InsightCard>> {
    let mut groups: HashMap<String, Vec<&InsightCard>> = HashMap::new();
    for card in cards {
        groups
            .entry(card.category.clone())
            .or_default()
            .push(card);
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
pub fn render_batch(candidates: &[InsightCandidate]) -> Vec<InsightCard> {
    let mut cards: Vec<InsightCard> = candidates.iter().map(render_insight).collect();
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
            lines.push(format!("[{}] {} ({}{})", cite.index, cite.source_url, cite.source_domain, ts));
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
        assert_eq!(map.get("signal").unwrap(), "3 new procurement job postings in 7 days");
        assert_eq!(map.get("region").unwrap(), "Tunisia");
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
    fn test_priority_score() {
        let score = priority_score(0.8, 0.9, "critical");
        assert!((score - 0.8 * 0.9 * 2.0).abs() < 1e-10);

        let score_warn = priority_score(0.5, 0.7, "warning");
        assert!((score_warn - 0.5 * 0.7 * 1.5).abs() < 1e-10);

        let score_info = priority_score(0.3, 0.6, "info");
        assert!((score_info - 0.3 * 0.6 * 1.0).abs() < 1e-10);
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
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://www.foxconn.com/press"), "www.foxconn.com");
        assert_eq!(extract_domain("http://example.com:8080/path"), "example.com");
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
        assert!(card.narrative.contains("Foxconn shows signs of a new sourcing cycle"));
        assert!(card.narrative.contains("3 new procurement job postings in 7 days"));
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
}
