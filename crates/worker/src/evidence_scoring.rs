#![cfg_attr(test, allow(dead_code))]

#[cfg(feature = "llm")]
use super::EvidenceSignal;
#[cfg(feature = "llm")]
use super::quality_gates::{
    marker_matches_normalized, normalize_gate_text, weighted_phrase_score,
    weighted_phrase_score_in_normalized,
};

#[cfg(feature = "llm")]
pub(super) fn noisy_or(scores: &[f32]) -> f32 {
    1.0 - scores.iter().fold(1.0, |accumulator, score| {
        accumulator * (1.0 - score.clamp(0.0, 1.0))
    })
}

#[cfg(feature = "llm")]
pub(super) fn min_max_scale_weight(weight: f32, min_weight: f32, max_weight: f32) -> f32 {
    if (max_weight - min_weight).abs() < f32::EPSILON {
        1.0
    } else {
        ((weight - min_weight) / (max_weight - min_weight)).clamp(0.0, 1.0)
    }
}

#[cfg(feature = "llm")]
pub(super) fn signal_type_relevance_score(signal_type: &str, category: &str) -> f32 {
    match signal_type {
        "certification" if category.contains("compliance") || category.contains("security") => 0.70,
        "capability" if category.contains("procurement") || category.contains("competitor") => 0.60,
        "poi" if category.contains("strategic_poi") || category.contains("talent") => 0.75,
        "poi" if category.contains("competitor") => 0.55,
        "facility" if category.contains("supply_chain") || category.contains("procurement") => 0.60,
        "facility" if category.contains("competitor") => 0.50,
        "TenderNotice" if category.contains("demand") || category.contains("procurement") => 0.85,
        "PatentPublication" if category.contains("technology") || category.contains("competitor") => 0.80,
        "RegulatoryFiling" if category.contains("regulatory") || category.contains("compliance") => 0.80,
        "FinancialDisclosure" if category.contains("competitor") || category.contains("ma_") => 0.75,
        "PersonMove" if category.contains("poi") || category.contains("talent") => 0.85,
        "PersonMove" if category.contains("competitor") => 0.65,
        "CompetitorEvent" if category.contains("competitor") => 0.75,
        "SocialPost" if category.contains("brand") || category.contains("sentiment") => 0.70,
        "warning" => 0.45,
        "news" => 0.35,
        _ => 0.20,
    }
}

#[cfg(feature = "llm")]
fn lexical_specificity_score(text: &str) -> f32 {
    let normalized = normalize_gate_text(text);
    let has_digits = normalized.chars().any(|character| character.is_ascii_digit());
    let cert_signal = weighted_phrase_score_in_normalized(
        &normalized,
        &[
            ("iso", 0.30),
            ("as9100", 0.35),
            ("iatf", 0.35),
            ("rfq", 0.30),
            ("tender", 0.30),
            ("tariff", 0.25),
            ("sanction", 0.25),
            ("ransomware", 0.35),
        ],
    )
    .min(0.35);

    noisy_or(&[
        if has_digits { 0.25 } else { 0.0 },
        cert_signal,
        if normalized.contains("http://") || normalized.contains("https://") {
            0.15
        } else {
            0.0
        },
    ])
}

#[cfg(feature = "llm")]
pub(super) fn signal_diversity_multiplier(distinct_signal_types: usize) -> f32 {
    1.0 + 0.1 * distinct_signal_types.saturating_sub(1) as f32
}

#[cfg(feature = "llm")]
pub(super) fn calculate_relevance(title: &str, description: &str, signal_type: &str, category: &str) -> f32 {
    let text = normalize_gate_text(&format!("{} {} {}", title, description, signal_type));

    let keywords: &[(&str, f32)] = match category {
        "demand_procurement" => &[
            ("rfq", 1.0),
            ("tender", 1.0),
            ("procurement", 0.9),
            ("sourcing", 0.8),
            ("bid", 0.8),
            ("contract", 0.7),
            ("supplier", 0.6),
            ("vendor", 0.6),
        ],
        "supply_chain_risk" => &[
            ("shortage", 1.0),
            ("delay", 0.9),
            ("disruption", 0.9),
            ("risk", 0.8),
            ("constraint", 0.8),
            ("lead time", 0.7),
            ("allocation", 0.7),
            ("single source", 0.9),
        ],
        "competitor_market" => &[
            ("competitor", 1.0),
            ("market share", 0.9),
            ("pricing", 0.8),
            ("win", 0.8),
            ("lost", 0.8),
            ("expansion", 0.7),
            ("capability", 0.6),
            ("capacity", 0.6),
        ],
        "security_compliance" => &[
            ("certification", 1.0),
            ("iso", 0.9),
            ("as9100", 1.0),
            ("iatf", 1.0),
            ("compliance", 0.9),
            ("audit", 0.8),
            ("accreditation", 0.9),
            ("security", 0.7),
        ],
        "regulatory_policy" => &[
            ("regulation", 1.0),
            ("tariff", 0.9),
            ("sanction", 1.0),
            ("export control", 1.0),
            ("policy", 0.8),
            ("legislation", 0.8),
            ("compliance", 0.7),
            ("itar", 1.0),
        ],
        "strategic_poi" => &[
            ("ceo", 1.0),
            ("cto", 1.0),
            ("cfo", 1.0),
            ("executive", 0.9),
            ("appointed", 0.9),
            ("resigned", 0.9),
            ("leadership", 0.8),
            ("vp", 0.7),
        ],
        "ma_partnerships" => &[
            ("acquisition", 1.0),
            ("merger", 1.0),
            ("partnership", 0.9),
            ("joint venture", 0.9),
            ("acquired", 1.0),
            ("divest", 0.9),
            ("spin-off", 0.8),
            ("alliance", 0.7),
        ],
        "technology_innovation" => &[
            ("patent", 1.0),
            ("r&d", 0.9),
            ("innovation", 0.9),
            ("breakthrough", 0.9),
            ("technology", 0.7),
            ("launch", 0.7),
            ("product", 0.6),
            ("development", 0.6),
        ],
        "cybersecurity_threat" => &[
            ("breach", 1.0),
            ("vulnerability", 1.0),
            ("cyber", 0.9),
            ("attack", 0.9),
            ("security", 0.7),
            ("malware", 1.0),
            ("ransomware", 1.0),
            ("incident", 0.8),
        ],
        _ => &[("signal", 0.5), ("detected", 0.5), ("update", 0.4)],
    };

    let min_weight = keywords
        .iter()
        .map(|(_, weight)| *weight)
        .fold(f32::INFINITY, f32::min);
    let max_weight = keywords
        .iter()
        .map(|(_, weight)| *weight)
        .fold(f32::NEG_INFINITY, f32::max);

    let normalized_tokens = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let keyword_dimensions = keywords
        .iter()
        .filter(|(keyword, _)| marker_matches_normalized(&normalized_tokens, &text, keyword))
        .map(|(_, weight)| min_max_scale_weight(*weight, min_weight, max_weight))
        .collect::<Vec<_>>();
    let keyword_score = noisy_or(&keyword_dimensions);
    let title_focus_score = weighted_phrase_score(title, keywords).min(1.0);
    let signal_type_score = signal_type_relevance_score(signal_type, category);
    let specificity_score = lexical_specificity_score(&text);

    noisy_or(&[
        keyword_score,
        title_focus_score,
        signal_type_score,
        specificity_score,
    ])
}

#[allow(dead_code)]
pub(super) fn extract_article_titles(description: &str) -> Vec<String> {
    let mut articles = Vec::new();
    if let Some(idx) = description
        .find("Articles:")
        .or_else(|| description.find("articles:"))
    {
        let after = &description[idx..];
        let end = after
            .find(". Source:")
            .or_else(|| after.find(". See:"))
            .or_else(|| after.find(". Page"))
            .unwrap_or(after.len());
        let articles_text = &after[after.find(':').map(|i| i + 1).unwrap_or(0)..end].trim();
        for title in articles_text.split(';') {
            let t = title.trim();
            if t.len() > 10 && !t.starts_with("http") {
                articles.push(t.to_string());
            }
        }
    }
    articles
}

#[allow(dead_code)]
pub(crate) fn clean_signal_title(title: &str, entity_name: &str) -> String {
    let mut cleaned = title.to_string();
    if let Some(rest) = cleaned.strip_prefix(&format!("{}: ", entity_name)) {
        cleaned = rest.to_string();
    }
    if cleaned.ends_with(" detected") && cleaned.len() < 40 {
        return String::new();
    }
    let lower = cleaned.to_lowercase();
    if lower.contains("update") && lower.len() < 30 {
        return String::new();
    }
    if lower == "lookalike_domain" || lower.starts_with("dns_") || lower.starts_with("webchange") {
        return String::new();
    }
    cleaned
}

#[cfg(feature = "llm")]
pub(super) fn dedup_signals<'a>(signals: &[&'a EvidenceSignal]) -> Vec<&'a EvidenceSignal> {
    let mut seen_titles: Vec<String> = Vec::new();
    let mut result = Vec::new();
    for sig in signals {
        let lower = sig.title.to_lowercase();
        let dominated = seen_titles.iter().any(|seen| {
            seen.contains(&lower)
                || lower.contains(seen.as_str())
                || (lower.len() > 15 && seen.len() > 15 && lower[..15] == seen[..15])
        });
        if !dominated {
            seen_titles.push(lower);
            result.push(*sig);
        }
    }
    result
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::*;

    #[test]
    fn diminishing_returns_noisy_or() {
        let weak = noisy_or(&[0.15; 5]);
        let strong = noisy_or(&[0.7]);
        assert!(weak < strong, "expected weak aggregate {weak} < strong {strong}");
    }

    #[test]
    fn signal_diversity_bonus() {
        let same_type = noisy_or(&[
            signal_type_relevance_score("warning", "supply_chain_risk"),
            signal_type_relevance_score("warning", "supply_chain_risk"),
            signal_type_relevance_score("warning", "supply_chain_risk"),
        ]) * signal_diversity_multiplier(1);
        let mixed_type = noisy_or(&[
            signal_type_relevance_score("certification", "security_compliance"),
            signal_type_relevance_score("capability", "competitor_market"),
            signal_type_relevance_score("poi", "strategic_poi"),
        ]) * signal_diversity_multiplier(3);

        assert!(mixed_type > same_type);
    }

    #[test]
    fn relevance_score_bounded() {
        let relevance = calculate_relevance(
            "AS9100 certification awarded after audit",
            "The supplier reported AS9100 accreditation and audit completion.",
            "certification",
            "security_compliance",
        );
        assert!((0.0..=1.0).contains(&relevance));
    }
}