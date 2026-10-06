//! Analytical depth assessment — hard, measurable sophistication scoring.
//!
//! The assessor turns a draft insight into a [`DepthReport`] with ten
//! independently-scored components, each grounded in countable textual facts
//! (marker counts, distinct actor classes, source families) rather than
//! vibes. The composite [`DepthReport::index`] and its tier are persisted per
//! insight so depth is a measured, trendable quantity.
//!
//! Design constraint: pure text measurement. No LLM is required to compute
//! depth, so the metric cannot be inflated by asking a model to grade itself
//! (the same reason the intelligence community audits products against fixed
//! tradecraft standards rather than author self-assessments).

use super::{source_families, EvidenceRecord, SourceClass};
use apex_core::claims::{ClaimKind, InsightClaim};
use serde::{Deserialize, Serialize};

/// Sophistication tiers, ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SophisticationTier {
    /// Low depth: single-source restatement.
    Shallow,
    /// Adequate desk note.
    Desk,
    /// Strategic briefing quality.
    Strategic,
    /// Multi-causal, quantified, hedged, alternative-aware strategic analysis.
    ForeignAffairs,
}

impl SophisticationTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shallow => "shallow",
            Self::Desk => "desk",
            Self::Strategic => "strategic",
            Self::ForeignAffairs => "foreign_affairs",
        }
    }

    pub fn from_index(index: f64) -> Self {
        if index >= 0.68 {
            Self::ForeignAffairs
        } else if index >= 0.50 {
            Self::Strategic
        } else if index >= 0.35 {
            Self::Desk
        } else {
            Self::Shallow
        }
    }
}

/// Weighted contribution of one depth dimension (weights sum to 1.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthComponent {
    pub name: String,
    /// Normalized 0..=1 score.
    pub score: f64,
    /// The raw countable quantity behind the score (markers, classes, …).
    pub raw_count: usize,
    pub weight: f64,
}

/// Configurable thresholds for what "deep enough" means.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthConfig {
    /// Minimum composite index the editorial board demands (default 0.50).
    pub min_index: f64,
    /// Minimum distinct source families before independence saturates.
    pub families_for_full_independence: usize,
}

impl Default for DepthConfig {
    fn default() -> Self {
        Self {
            min_index: 0.50,
            families_for_full_independence: 3,
        }
    }
}

/// Everything the assessor reads.
#[derive(Debug, Clone)]
pub struct DepthInputs<'a> {
    pub headline: &'a str,
    pub narrative: &'a str,
    pub recommendations: &'a str,
    pub claims: &'a [InsightClaim],
    pub evidence: &'a [EvidenceRecord],
    pub confidence: f64,
    /// Alternative hypotheses carried with the insight (ACH lines); the
    /// assessed count is `alternative_hypotheses` capped at 3.
    pub alternative_hypotheses: usize,
}

/// The depth verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthReport {
    pub index: f64,
    pub tier: SophisticationTier,
    pub components: Vec<DepthComponent>,
    pub word_count: usize,
    /// Filler/formulaic phrase hits (penalty already applied to `index`).
    pub filler_hits: usize,
    /// Absolute-certainty markers (proves/certainly/...) — consumed by the
    /// editorial confidence gate so certainty injections strictly lower
    /// released confidence.
    pub certainty_hits: usize,
}

const WEIGHTS: [(&str, f64); 10] = [
    ("causal_depth", 0.16),
    ("counterargument_depth", 0.14),
    ("quantification", 0.12),
    ("stakeholder_coverage", 0.10),
    ("second_order_effects", 0.10),
    ("uncertainty_discipline", 0.10),
    ("source_independence", 0.10),
    ("temporal_depth", 0.08),
    ("actionability", 0.05),
    ("specificity", 0.05),
];

const CAUSAL_MARKERS: [&str; 30] = [
    "because",
    "drove",
    "driven by",
    "driven",
    "drives",
    "leads to",
    "led to",
    "as a result",
    "thereby",
    "triggered",
    "trigger",
    "triggers",
    "forced",
    "forces",
    "forced by",
    "due to",
    "in response to",
    "pressures",
    "creates pressure",
    "results in",
    "stemming from",
    "attributable to",
    "fuels",
    "fueled",
    "spurred",
    "prompted",
    "caused",
    "constrains",
    "enables",
    "undermines",
];

const SECOND_ORDER_MARKERS: [&str; 14] = [
    "in turn",
    "second-order",
    "second order",
    "knock-on",
    "ripple",
    "which would",
    "which could",
    "which may",
    "downstream",
    "follow-on",
    "spillover",
    "compounding",
    "cascades",
    "indirect effect",
];

const COUNTER_MARKERS: [&str; 18] = [
    "alternative explanation",
    "alternatively",
    "could instead",
    "on the other hand",
    "counter-argument",
    "counterargument",
    "counterpoint",
    "but this assumes",
    "this would fail if",
    "falsified by",
    "weakens if",
    "what would change",
    "however",
    "yet the evidence",
    "dissenting",
    "contrary view",
    "steelman",
    "if the opposite",
];

const UNCERTAINTY_MARKERS: [&str; 24] = [
    "almost certainly",
    "very likely",
    "likely",
    "probably",
    "roughly even",
    "about even",
    "chances are",
    "possible",
    "unlikely",
    "very unlikely",
    "remote",
    "estimate",
    "estimated",
    "assess",
    "assessed",
    "judge",
    "judgment",
    "confidence",
    "odds",
    "probability",
    "may",
    "might",
    "could",
    "appears",
];

const CERTAINTY_MARKERS: [&str; 12] = [
    "certainly",
    "definitely",
    "proves",
    "proven",
    "undeniable",
    "undoubtedly",
    "without doubt",
    "guarantees",
    "inevitable",
    "always",
    "never",
    "conclusively",
];

const TEMPORAL_MARKERS: [&str; 20] = [
    "since",
    "over the past",
    "over the last",
    "in recent months",
    "in recent weeks",
    "last week",
    "last month",
    "last quarter",
    "year-on-year",
    "year over year",
    "quarter-on-quarter",
    "historically",
    "previously",
    "prior cycle",
    "in 20",
    "q1",
    "q2",
    "q3",
    "q4",
    "ytd",
];

const ACTION_TIMING_MARKERS: [&str; 14] = [
    "within",
    "by the end",
    "before the",
    "this week",
    "next week",
    "this month",
    "next month",
    "this quarter",
    "next quarter",
    "immediately",
    "days",
    "weeks",
    "deadline",
    "timeline",
];

const ACTION_VERBS: [&str; 20] = [
    "recommend",
    "contact",
    "engage",
    "schedule",
    "prepare",
    "monitor",
    "review",
    "audit",
    "verify",
    "secure",
    "lock",
    "negotiate",
    "alert",
    "notify",
    "brief",
    "request",
    "assess",
    "map",
    "prioritize",
    "re-forecast",
];

/// Stakeholder taxonomy: class → lexemes that indicate the class.
const STAKEHOLDER_CLASSES: [(&str, &[&str]); 11] = [
    (
        "supplier",
        &[
            "supplier",
            "vendor",
            "distributor",
            "partner",
            "subcontractor",
        ],
    ),
    (
        "customer",
        &[
            "customer",
            "client",
            "buyer",
            "oem",
            "procurement",
            "tender",
        ],
    ),
    (
        "competitor",
        &["competitor", "rival", "peer firm", "market share"],
    ),
    (
        "regulator",
        &["regulator", "regulatory", "authority", "agency", "ministry"],
    ),
    (
        "government",
        &["government", "state", "administration", "parliament"],
    ),
    (
        "financial",
        &[
            "investor",
            "lender",
            "bank",
            "credit",
            "financing",
            "funding",
        ],
    ),
    (
        "logistics",
        &[
            "port",
            "shipping",
            "freight",
            "logistics",
            "carrier",
            "route",
        ],
    ),
    (
        "labor",
        &["workforce", "labor", "union", "staffing", "hiring"],
    ),
    (
        "technology",
        &[
            "technology",
            "platform",
            "software",
            "chip",
            "semiconductor",
            "ai ",
        ],
    ),
    (
        "security",
        &["military", "defense", "security", "sanctions", "conflict"],
    ),
    (
        "media",
        &["media", "press", "news agency", "public opinion"],
    ),
];

const FILLER_PHRASES: [&str; 16] = [
    "in today's dynamic",
    "it is important to note",
    "in conclusion",
    "game changer",
    "cutting-edge solutions",
    "leverage our",
    "synergies",
    "one-stop shop",
    "value-added services",
    "state-of-the-art",
    "robust solutions",
    "we are well-positioned",
    "significant opportunity",
    "exciting opportunity",
    "rapidly evolving landscape",
    "comprehensive range of services",
];

const HEADLINE_STOPWORDS: [&str; 30] = [
    "the", "a", "an", "and", "or", "but", "for", "with", "from", "into", "over", "under", "after",
    "before", "new", "this", "that", "these", "those", "its", "his", "her", "their", "our", "as",
    "at", "in", "on", "by", "to",
];

/// Saturating transform: monotone, 0 → 0, ∞ → 1, smooth (Michaelis–Menten).
fn saturate(value: f64, half: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    value / (value + half)
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

fn count_marker_hits(text_lower: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .map(|marker| count_word_occurrences(text_lower, marker))
        .sum()
}

/// Count occurrences of `needle` that are not embedded inside a longer word
/// ("cause" must not match inside "because").
fn count_word_occurrences(haystack: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(position) = haystack[start..].find(needle) {
        let absolute = start + position;
        let before_ok = absolute == 0
            || !haystack[..absolute]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        let end = absolute + needle.len();
        let after_ok = end >= haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if before_ok && after_ok {
            count += 1;
        }
        start = absolute + needle.len();
    }
    count
}

/// Proper-noun-ish tokens: capitalized words that are not sentence-initial
/// function words. Used only as a specificity signal, so false positives are
/// acceptable; false negatives (all-caps entities) are counted too.
fn specific_tokens(text: &str) -> usize {
    let mut count = 0;
    let mut sentence_start = true;
    for token in text.split_whitespace() {
        let stripped: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '&' || *c == '.')
            .collect();
        let first = stripped.chars().next();
        let is_capitalized = first.is_some_and(|c| c.is_uppercase());
        let is_acronym = stripped.len() >= 2
            && stripped.len() <= 6
            && stripped.chars().all(|c| c.is_ascii_uppercase());
        let lower = stripped.to_ascii_lowercase();
        let stopword = HEADLINE_STOPWORDS.contains(&lower.as_str());
        if (is_capitalized || is_acronym)
            && stripped.chars().count() >= 2
            && !(sentence_start && stopword)
        {
            count += 1;
        }
        sentence_start = token.ends_with('.') || token.ends_with('!') || token.ends_with('?');
    }
    count
}

/// Assess a draft against the ten depth dimensions.
pub fn assess_depth(inputs: &DepthInputs<'_>, config: &DepthConfig) -> DepthReport {
    let narrative = inputs.narrative;
    let recommendations = inputs.recommendations;
    let combined = format!("{} {} {}", inputs.headline, narrative, recommendations);
    let lower = combined.to_ascii_lowercase();
    let word_count = words(&combined).len().max(1);
    let per_100_words = |count: usize| (count as f64) * 100.0 / word_count as f64;

    let causal_hits = count_marker_hits(&lower, &CAUSAL_MARKERS);
    let causal_score = saturate(causal_hits as f64, 6.0);

    let temporal_hits =
        count_marker_hits(&lower, &TEMPORAL_MARKERS) + combined.matches("20").count().min(6);
    let temporal_score = saturate(temporal_hits as f64, 5.0);

    let numeric_hits = count_numeric_facts(&combined);
    let quantification_score = saturate(per_100_words(numeric_hits), 3.0);

    let stakeholder_classes = STAKEHOLDER_CLASSES
        .iter()
        .filter(|(_, lexemes)| lexemes.iter().any(|lexeme| lower.contains(lexeme)))
        .count();
    let stakeholder_score = saturate(stakeholder_classes as f64, 4.0);

    let second_order_hits = count_marker_hits(&lower, &SECOND_ORDER_MARKERS);
    let second_order_score = saturate(second_order_hits as f64, 2.0);

    let counter_hits = count_marker_hits(&lower, &COUNTER_MARKERS);
    let counterarg_score = saturate(
        (counter_hits + inputs.alternative_hypotheses.min(3)) as f64,
        2.0,
    );

    let uncertainty_hits = count_marker_hits(&lower, &UNCERTAINTY_MARKERS);
    let certainty_hits = count_marker_hits(&lower, &CERTAINTY_MARKERS);
    let families = source_families(inputs.evidence);
    let family_count = families.len();
    // Certainty without independent corroboration is penalized hard; certainty
    // with strong official corroboration is acceptable.
    let certainty_penalty = if family_count >= 2 {
        (certainty_hits as f64 * 0.05).min(0.25)
    } else {
        (certainty_hits as f64 * 0.15).min(0.6)
    };
    let uncertainty_base =
        if uncertainty_hits > 0 { 0.6 } else { 0.2 } + saturate(uncertainty_hits as f64, 4.0) * 0.4;
    let uncertainty_score = (uncertainty_base - certainty_penalty).clamp(0.0, 1.0);

    let independence_score = if families.is_empty() {
        0.0
    } else {
        let breadth = saturate(
            family_count as f64,
            config.families_for_full_independence as f64,
        );
        let class_mix = families
            .iter()
            .map(|family| family.class.authority())
            .fold(0.0_f64, f64::max);
        let official_bonus = if families
            .iter()
            .any(|family| family.class == SourceClass::Official)
        {
            0.15
        } else {
            0.0
        };
        (breadth * (0.5 + 0.5 * class_mix) + official_bonus).clamp(0.0, 1.0)
    };

    let recommendation_words: Vec<&str> = words(recommendations);
    let recommendation_sentences = recommendations
        .split(['.', '!', '?'])
        .filter(|sentence| sentence.split_whitespace().count() >= 4)
        .count()
        .max(1);
    let action_with_timing = recommendations
        .split(['.', '!', '?'])
        .filter(|sentence| {
            let sentence_lower = sentence.to_ascii_lowercase();
            ACTION_VERBS
                .iter()
                .any(|verb| sentence_lower.contains(verb))
                && ACTION_TIMING_MARKERS
                    .iter()
                    .any(|marker| sentence_lower.contains(marker))
        })
        .count();
    let actionability_score = if recommendation_words.len() < 4 {
        0.0
    } else {
        saturate(
            action_with_timing as f64 / recommendation_sentences as f64,
            0.5,
        )
    };

    let proper_hits = specific_tokens(&combined);
    let specificity_score = saturate(per_100_words(proper_hits), 8.0);

    let observed_ratio = if inputs.claims.is_empty() {
        0.0
    } else {
        inputs
            .claims
            .iter()
            .filter(|claim| claim.kind == ClaimKind::Observed)
            .count() as f64
            / inputs.claims.len() as f64
    };
    // Claims that are neither observed nor inferences (unknown/unsupported
    // statements) cap the counterargument adjustment: an unknown claim cannot
    // be "balanced".
    let known_ratio = if inputs.claims.is_empty() {
        0.5
    } else {
        inputs
            .claims
            .iter()
            .filter(|claim| matches!(claim.kind, ClaimKind::Observed | ClaimKind::Inference))
            .count() as f64
            / inputs.claims.len() as f64
    };
    let _ = observed_ratio;

    let filler_hits = FILLER_PHRASES
        .iter()
        .map(|phrase| lower.matches(phrase).count())
        .sum::<usize>();

    let mut components = Vec::with_capacity(WEIGHTS.len());
    let raw_by_name = |name: &str| -> (f64, usize) {
        match name {
            "causal_depth" => (causal_score, causal_hits),
            "temporal_depth" => (temporal_score, temporal_hits),
            "quantification" => (quantification_score, numeric_hits),
            "stakeholder_coverage" => (stakeholder_score, stakeholder_classes),
            "second_order_effects" => (second_order_score, second_order_hits),
            "counterargument_depth" => (
                counterarg_score.min(0.4 + 0.6 * known_ratio),
                counter_hits + inputs.alternative_hypotheses.min(3),
            ),
            "uncertainty_discipline" => (uncertainty_score, uncertainty_hits),
            "source_independence" => (independence_score, family_count),
            "actionability" => (actionability_score, action_with_timing),
            "specificity" => (specificity_score, proper_hits),
            other => unreachable!("unknown depth component {other}"),
        }
    };

    let mut index = 0.0;
    for (name, weight) in WEIGHTS {
        let (score, raw_count) = raw_by_name(name);
        index += weight * score;
        components.push(DepthComponent {
            name: name.to_string(),
            score,
            raw_count,
            weight,
        });
    }
    index = (index - (filler_hits as f64 * 0.01).min(0.1)).clamp(0.0, 1.0);

    DepthReport {
        index,
        tier: SophisticationTier::from_index(index),
        components,
        word_count,
        filler_hits,
        certainty_hits,
    }
}

/// Count numeric facts: percentages, currency amounts, and standalone
/// integers ≥ 2 digits. Years (1900–2100) count as temporal, not numeric.
pub(crate) fn count_numeric_facts(text: &str) -> usize {
    use std::sync::OnceLock;
    static PERCENT: OnceLock<regex::Regex> = OnceLock::new();
    static CURRENCY: OnceLock<regex::Regex> = OnceLock::new();
    static INTEGER: OnceLock<regex::Regex> = OnceLock::new();
    let percent = PERCENT.get_or_init(|| {
        regex::Regex::new(r"\d+(?:[.,]\d+)?\s*%")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    });
    let currency = CURRENCY.get_or_init(|| {
        regex::Regex::new(r"[$€£]\s?\d+(?:[.,]\d+)?\s?(?:k|m|bn|billion|million|thousand)?")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    });
    let integer = INTEGER.get_or_init(|| {
        regex::Regex::new(r"\b\d{2,}\b")
            .unwrap_or_else(|error| panic!("valid static regex: {error}"))
    });

    let percent_hits = percent.find_iter(text).count();
    let currency_hits = currency.find_iter(text).count();
    // Blank out percent/currency spans so their digits are not double-counted
    // by the integer pass.
    let mut remainder = String::with_capacity(text.len());
    let mut cut = 0usize;
    let mut spans: Vec<(usize, usize)> = percent
        .find_iter(text)
        .chain(currency.find_iter(text))
        .map(|m| (m.start(), m.end()))
        .collect();
    spans.sort_unstable();
    for (start, end) in spans {
        if start >= cut {
            remainder.push_str(&text[cut..start]);
            cut = end;
        }
    }
    remainder.push_str(&text[cut..]);
    let integer_hits = integer
        .find_iter(&remainder)
        .filter(|m| {
            let value: Option<u32> = m.as_str().parse().ok();
            !value.is_some_and(|value| (1900..=2100).contains(&value))
        })
        .count();
    percent_hits + currency_hits + integer_hits
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::analytical::EvidenceRecord;
    use uuid::Uuid;

    fn claim(text: &str, kind: ClaimKind) -> InsightClaim {
        InsightClaim::new(text.to_string(), Vec::new(), Some(0.7), kind)
    }

    fn evidence(n: usize) -> Vec<EvidenceRecord> {
        (0..n)
            .map(|i| EvidenceRecord {
                evidence_id: Uuid::from_u128(i as u128 + 1),
                title: format!("Source {i} report"),
                text: "Trade ministry filing shows a 12% increase since 2024.".into(),
                source_name: format!("publisher{i}.example"),
                source_url: Some(format!("https://publisher{i}.example/a")),
                signal_type: "government_registry".into(),
                observed_at: None,
                reliability: 0.8,
            })
            .collect()
    }

    fn deep_inputs<'a>(
        narrative: &'a str,
        recommendations: &'a str,
        claims: &'a [InsightClaim],
        evidence: &'a [EvidenceRecord],
    ) -> DepthInputs<'a> {
        DepthInputs {
            headline: "Tariff pressure reshapes North African electronics sourcing",
            narrative,
            recommendations,
            claims,
            evidence,
            confidence: 0.7,
            alternative_hypotheses: 2,
        }
    }

    #[test]
    fn shallow_restatement_scores_lowly() {
        let claims = vec![claim(
            "Something happened here recently.",
            ClaimKind::Unknown,
        )];
        let evidence = evidence(1);
        let inputs = deep_inputs(
            "The company did a thing. It was notable.",
            "",
            &claims,
            &evidence,
        );
        let report = assess_depth(&inputs, &DepthConfig::default());
        assert!(report.index < 0.35, "index was {}", report.index);
        assert_eq!(report.tier, SophisticationTier::Shallow);
    }

    #[test]
    fn deep_multicausal_analysis_scores_high() {
        let narrative = "Tariff increases on HS 8534 drove a 21% landed-cost rise in Tunisia since 2024, \
            because ministry filings show duty rates rising in Q1 and Q3. This forced buyers to re-quote; \
            in turn, nearshore suppliers gained share. However, an alternative explanation is that freight \
            rates, not tariffs, drove the shift. The evidence suggests roughly even odds that both factors \
            compounded. Second-order effects would include supplier consolidation which could reduce choice.";
        let recommendations =
            "We recommend contacting the top three buyers within two weeks to lock pricing. \
            Monitor regulator gazette updates this month and re-forecast margins next quarter.";
        let claims = vec![
            claim(
                "Tariffs drove a 21% landed-cost rise [1].",
                ClaimKind::Observed,
            ),
            claim(
                "Buyers likely shifted to nearshore suppliers [2].",
                ClaimKind::Inference,
            ),
            claim(
                "Alternative explanation: freight rates.",
                ClaimKind::Inference,
            ),
        ];
        let evidence = evidence(4);
        let inputs = deep_inputs(narrative, recommendations, &claims, &evidence);
        let report = assess_depth(&inputs, &DepthConfig::default());
        assert!(report.index >= 0.5, "index was {}", report.index);
        assert!(
            matches!(
                report.tier,
                SophisticationTier::Strategic | SophisticationTier::ForeignAffairs
            ),
            "tier was {:?}",
            report.tier
        );
        let causal = report
            .components
            .iter()
            .find(|component| component.name == "causal_depth")
            .unwrap();
        assert!(causal.raw_count >= 3, "causal raw {}", causal.raw_count);
    }

    #[test]
    fn certainty_without_corroboration_is_penalized() {
        let claims = vec![claim(
            "It is proven that the deal happened.",
            ClaimKind::Observed,
        )];
        let single = evidence(1);
        let narrative =
            "This certainly proves the supplier will expand. Undoubtedly the market is inevitable.";
        let inputs = deep_inputs(narrative, "", &claims, &single);
        let report = assess_depth(&inputs, &DepthConfig::default());
        let uncertainty = report
            .components
            .iter()
            .find(|component| component.name == "uncertainty_discipline")
            .unwrap();
        assert!(uncertainty.score < 0.4, "score {}", uncertainty.score);
    }

    #[test]
    fn filler_phrases_penalize_index() {
        let claims = vec![claim("A statement about the market.", ClaimKind::Unknown)];
        let evidence = evidence(1);
        let clean = assess_depth(
            &deep_inputs(
                "Tariff pressure drove a 12% cost rise since 2024.",
                "",
                &claims,
                &evidence,
            ),
            &DepthConfig::default(),
        );
        let filler = assess_depth(
            &deep_inputs(
                "In today's dynamic landscape, tariff pressure drove a 12% cost rise since 2024. \
                 It is important to note this is a game changer. Synergies and robust solutions.",
                "",
                &claims,
                &evidence,
            ),
            &DepthConfig::default(),
        );
        assert!(
            filler.index < clean.index,
            "filler {} vs clean {}",
            filler.index,
            clean.index
        );
        assert!(filler.filler_hits >= 3);
    }

    #[test]
    fn numeric_fact_counter_skips_years() {
        assert_eq!(
            count_numeric_facts("In 2024 revenue rose 12% to $3.5m across 34 plants"),
            3
        );
    }
}
