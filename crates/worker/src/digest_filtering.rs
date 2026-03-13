//! Digest deduplication and quality checks.
//!
//! Contains functions for digest token extraction, deduplication,
//! and quality filtering for email digests.

use apex_core::similarity::jaccard_similarity;
use std::collections::{HashMap, HashSet};

use crate::fallback_generation::{count_concrete_signal_details, extract_fallback_signal_details};

/// Stopwords to filter out during tokenization.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "over", "under", "into",
    "onto", "after", "before", "about", "their", "there", "they", "them", "were", "have",
    "has", "been", "being", "will", "would", "could", "should", "a", "an", "of", "to", "in",
    "on", "by", "at", "as", "is", "are", "or",
];

use crate::config::{DEDUP_TITLE_THRESHOLD, DEDUP_SUMMARY_THRESHOLD};

/// Generate a canonical key from raw text for deduplication.
pub(super) fn canonical_digest_key(raw: &str, max_words: usize) -> String {
    let normalized = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>();

    normalized
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tokenize text for similarity comparison, removing stopwords.
pub(super) fn digest_tokens(raw: &str, max_tokens: usize) -> Vec<String> {
    raw.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .take(max_tokens)
        .map(|w| w.to_string())
        .collect()
}

/// Compute Jaccard similarity between token sets.
pub(super) fn token_jaccard_similarity(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_set: HashSet<&str> = left.iter().map(String::as_str).collect();
    let right_set: HashSet<&str> = right.iter().map(String::as_str).collect();
    jaccard_similarity(&left_set, &right_set)
}

/// Detect excessive phrase repetition indicating templated/mangled output.
pub(super) fn has_excessive_phrase_repetition(text: &str) -> bool {
    let tokens = digest_tokens(text, 220);
    if tokens.len() < 10 {
        return false;
    }

    // Flag repeated 4-token windows that indicate templated/mangled outputs.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for window in tokens.windows(4) {
        let phrase = window.join(" ");
        let entry = counts.entry(phrase).or_insert(0);
        *entry += 1;
        if *entry >= 3 {
            return true;
        }
    }
    false
}

/// Check if an insight type is a promoted business insight type.
pub(super) fn is_promoted_business_insight_type(insight_type: Option<&str>) -> bool {
    matches!(
        insight_type,
        Some(
            "demand_procurement"
                | "competitor_market"
                | "supply_chain_risk"
                | "regulatory_policy"
                | "pricing_market"
                | "geopolitical_analysis"
        )
    )
}

/// Check if summary text has sufficient promoted-business specificity.
pub(super) fn has_concrete_promoted_business_summary(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    let sentence_fragments = summary
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .collect::<Vec<_>>();

    if !(4..=12).contains(&sentence_fragments.len()) {
        return false;
    }

    let citation_count = summary.matches('[').count();
    if citation_count < 2 {
        return false;
    }

    let action_lane_count = sentence_fragments
        .iter()
        .filter(|fragment| {
            let fragment_lower = fragment.to_ascii_lowercase();
            [
                "approach ",
                "contact ",
                "target ",
                "engage ",
                "map ",
                "align ",
                "review ",
                "prepare ",
                "propose ",
                "should ",
            ]
            .iter()
            .any(|marker| fragment_lower.starts_with(marker) || fragment_lower.contains(marker))
        })
        .count();
    if action_lane_count < 2 {
        return false;
    }

    let business_marker_count = [
        "customer",
        "procurement",
        "qualification",
        "program",
        "supply chain",
        "tariff",
        "policy",
        "stakeholder",
        "pricing",
        "compliance",
        "risk",
    ]
    .iter()
    .filter(|marker| lower.contains(**marker))
    .count();

    business_marker_count >= 3
}

/// Check if digest text is readable and useful.
pub(super) fn is_readable_and_useful_digest_text(summary: &str) -> bool {
    if has_excessive_phrase_repetition(summary) {
        return false;
    }

    let sentence_count = summary
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .count();
    if sentence_count == 0 || sentence_count > 8 {
        return false;
    }

    let tokens = digest_tokens(summary, 240);
    if tokens.len() < 12 {
        return false;
    }
    let unique: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let unique_ratio = unique.len() as f64 / tokens.len() as f64;
    if unique_ratio < 0.48 {
        return false;
    }

    let lower = summary.to_ascii_lowercase();
    let usefulness_markers = [
        "impact",
        "risk",
        "opportunity",
        "because",
        "therefore",
        "drives",
        "leads to",
        "supply",
        "pricing",
        "compliance",
        "customer",
        "action",
    ];
    usefulness_markers
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Check if digest text is readable and useful, with type-specific thresholds.
pub(super) fn is_readable_and_useful_digest_text_for_type(
    summary: &str,
    insight_type: Option<&str>,
) -> bool {
    if has_excessive_phrase_repetition(summary) {
        return false;
    }

    let sentence_count = summary
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .count();
    let max_sentences = if is_promoted_business_insight_type(insight_type) {
        10
    } else {
        8
    };
    if sentence_count == 0 || sentence_count > max_sentences {
        return false;
    }

    let tokens = digest_tokens(summary, 240);
    if tokens.len() < 12 {
        return false;
    }
    let unique: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let unique_ratio = unique.len() as f64 / tokens.len() as f64;
    if unique_ratio < 0.48 {
        return false;
    }

    let lower = summary.to_ascii_lowercase();
    let usefulness_markers: &[&str] = if is_promoted_business_insight_type(insight_type) {
        &[
            "impact",
            "risk",
            "opportunity",
            "because",
            "therefore",
            "drives",
            "leads to",
            "supply",
            "pricing",
            "compliance",
            "customer",
            "action",
            "procurement",
            "tender",
            "qualification",
            "stakeholder",
            "policy",
            "tariff",
            "export",
            "nearshoring",
        ]
    } else {
        &[
            "impact",
            "risk",
            "opportunity",
            "because",
            "therefore",
            "drives",
            "leads to",
            "supply",
            "pricing",
            "compliance",
            "customer",
            "action",
        ]
    };

    usefulness_markers
        .iter()
        .any(|marker| lower.contains(marker))
}

pub(super) fn is_internal_insight_type(insight_type: Option<&str>) -> bool {
    insight_type
        .map(|insight_type| {
            let normalized = insight_type.trim().to_ascii_lowercase();
            normalized.starts_with("llm_")
                || matches!(normalized.as_str(), "llm_eval_report" | "llm_self_improvement")
        })
        .unwrap_or(false)
}

pub(super) fn count_template_markers(text: &str) -> usize {
    const TEMPLATE_MARKERS: &[&str] = &[
        "assessment:",
        "recommended action:",
        "additional source reporting:",
        "signal themes detected:",
        "actionable:",
        "watch closely:",
        "early signal:",
        "low confidence:",
        "analysis:",
        "impact:",
        "recommendation:",
    ];

    let lower = text.to_ascii_lowercase();
    TEMPLATE_MARKERS
        .iter()
        .filter(|marker| lower.contains(**marker))
        .count()
}

pub(crate) fn passes_shared_insight_quality_gate(
    title: &str,
    summary: &str,
    insight_type: Option<&str>,
) -> bool {
    if is_internal_insight_type(insight_type) {
        return true;
    }
    if title.trim().len() < 12 || summary.trim().len() < 80 {
        return false;
    }
    if crate::is_low_quality_narrative(summary) {
        return false;
    }
    if has_excessive_phrase_repetition(summary) {
        return false;
    }

    let lower_title = title.to_ascii_lowercase();
    let lower_summary = summary.to_ascii_lowercase();
    let malformed_fragments = [
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
        "[object object]",
        "undefined",
        "{{",
        "}}",
    ];
    if malformed_fragments
        .iter()
        .any(|fragment| lower_title.contains(fragment) || lower_summary.contains(fragment))
    {
        return false;
    }

    if count_template_markers(summary) >= 2 {
        return false;
    }

    let generic_fallback_markers = [
        "if the priority is ",
        "if the goal is ",
        "this deserves action inside the current planning cycle",
        "overall confidence is ",
        "grounded in ",
    ];
    let generic_fallback_count = generic_fallback_markers
        .iter()
        .filter(|marker| lower_summary.contains(**marker))
        .count();

    let fallback_signal_details = extract_fallback_signal_details(summary);
    if lower_summary.contains("our monitoring detected:")
        && lower_summary.contains("has been flagged for")
        && !fallback_signal_details.is_empty()
        && count_concrete_signal_details(&fallback_signal_details) == 0
    {
        return false;
    }

    if lower_summary.contains("our monitoring detected:")
        && lower_summary.contains("has been flagged for")
        && generic_fallback_count >= 2
    {
        return false;
    }

    if is_promoted_business_insight_type(insight_type)
        && has_concrete_promoted_business_summary(summary)
    {
        return true;
    }

    if matches!(insight_type, Some("veracity_analysis")) {
        return is_readable_and_useful_digest_text_for_type(summary, insight_type)
            && lower_summary.contains("source")
            && (lower_summary.contains("evidence")
                || lower_summary.contains("corroborat")
                || lower_summary.contains("reported"));
    }

    is_readable_and_useful_digest_text_for_type(summary, insight_type)
}

/// Check if an insight meets digest quality standards.
pub(crate) fn is_digest_insight_quality(title: &str, summary: &str) -> bool {
    // Title must be sufficiently descriptive
    if title.len() < 12 {
        return false;
    }

    if crate::is_low_quality_narrative(summary) {
        return false;
    }

    if !is_readable_and_useful_digest_text(summary) {
        return false;
    }

    if summary.len() < 80 {
        return false;
    }

    let bad_fragments = [
        "{{",
        "}}",
        "[object object]",
        "undefined",
        "null null",
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
    ];
    let lower_title = title.to_ascii_lowercase();
    let lower_summary = summary.to_ascii_lowercase();
    !bad_fragments
        .iter()
        .any(|fragment| lower_title.contains(fragment) || lower_summary.contains(fragment))
}

/// Deduplicate insights by title and summary similarity.
#[allow(dead_code)]
pub(super) fn dedup_insights_for_digest<T: Clone>(
    insights: &[T],
    get_title: impl Fn(&T) -> &str,
    get_summary: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut result = Vec::new();
    let mut seen_titles: Vec<Vec<String>> = Vec::new();
    let mut seen_summaries: Vec<Vec<String>> = Vec::new();

    for insight in insights {
        let title = get_title(insight);
        let summary = get_summary(insight);

        let title_tokens = digest_tokens(title, 30);
        let summary_tokens = digest_tokens(summary, 80);

        // Check for near-duplicate by title
        let title_duplicate = seen_titles.iter().any(|seen| {
            token_jaccard_similarity(&title_tokens, seen) >= *DEDUP_TITLE_THRESHOLD
        });

        // Check for near-duplicate by summary
        let summary_duplicate = seen_summaries.iter().any(|seen| {
            token_jaccard_similarity(&summary_tokens, seen) >= *DEDUP_SUMMARY_THRESHOLD
        });

        if !title_duplicate && !summary_duplicate {
            seen_titles.push(title_tokens);
            seen_summaries.push(summary_tokens);
            result.push(insight.clone());
        }
    }

    result
}

/// Expand digest category aliases to full insight type names.
pub(super) fn expand_digest_categories(categories: &[String]) -> Vec<String> {
    fn push_unique(out: &mut Vec<String>, value: &str) {
        if !out.iter().any(|v| v == value) {
            out.push(value.to_string());
        }
    }

    let mut out = Vec::new();
    for category in categories {
        match category.trim().to_ascii_lowercase().as_str() {
            "demand_signal" => {
                push_unique(&mut out, "demand_procurement");
                push_unique(&mut out, "customer_rfq");
                push_unique(&mut out, "pricing_market");
            }
            "supply_risk" => {
                push_unique(&mut out, "supply_chain_risk");
                push_unique(&mut out, "quality_compliance");
            }
            "competitive_intel" => {
                push_unique(&mut out, "competitor_market");
                push_unique(&mut out, "ma_partnerships");
                push_unique(&mut out, "market_expansion");
            }
            "security_posture" => {
                push_unique(&mut out, "cybersecurity_threat");
                push_unique(&mut out, "security_compliance");
            }
            "macro_shift" => {
                push_unique(&mut out, "geopolitical_analysis");
                push_unique(&mut out, "regulatory_policy");
                push_unique(&mut out, "brand_sentiment");
            }
            "poi_movement" => {
                push_unique(&mut out, "strategic_poi");
                push_unique(&mut out, "talent_ip");
            }
            // Backward compatible passthrough for direct insight_type values.
            other if !other.is_empty() => push_unique(&mut out, other),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_normalizes_text() {
        assert_eq!(
            canonical_digest_key("Hello, World! This is a Test.", 5),
            "hello world this test"
        );
    }

    #[test]
    fn digest_tokens_filters_stopwords() {
        let tokens = digest_tokens("The quick brown fox jumps over the lazy dog", 10);
        assert!(!tokens.contains(&"the".to_string()));
        assert!(tokens.contains(&"quick".to_string()));
    }

    #[test]
    fn jaccard_similarity_computed_correctly() {
        let a = vec!["quick".to_string(), "brown".to_string(), "fox".to_string()];
        let b = vec!["quick".to_string(), "brown".to_string(), "dog".to_string()];
        let sim = token_jaccard_similarity(&a, &b);
        assert!(sim > 0.4 && sim < 0.7); // 2/4 intersection/union
    }

    #[test]
    fn excessive_repetition_detected() {
        let repeated = "action plan action plan action plan action plan action plan";
        assert!(has_excessive_phrase_repetition(repeated));

        let varied = "This is a varied text with different words and phrases throughout.";
        assert!(!has_excessive_phrase_repetition(varied));
    }

    #[test]
    fn promoted_types_identified() {
        assert!(is_promoted_business_insight_type(Some("demand_procurement")));
        assert!(is_promoted_business_insight_type(Some("supply_chain_risk")));
        assert!(!is_promoted_business_insight_type(Some("random_type")));
        assert!(!is_promoted_business_insight_type(None));
    }

    #[test]
    fn category_expansion_works() {
        let cats = vec!["demand_signal".to_string()];
        let expanded = expand_digest_categories(&cats);
        assert!(expanded.contains(&"demand_procurement".to_string()));
        assert!(expanded.contains(&"customer_rfq".to_string()));
    }
}
