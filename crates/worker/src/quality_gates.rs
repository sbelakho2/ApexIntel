#[cfg(feature = "llm")]
use apex_core::timeline::{EntityTimeline, TimelineEvent};
#[cfg(feature = "llm")]
use apex_llm::insight_gen::validate_narrative_temporal_ordering;
#[cfg(feature = "llm")]
use chrono::{DateTime, Utc};
#[cfg(feature = "llm")]
use regex::Regex;
#[cfg(feature = "llm")]
use std::collections::HashSet;
#[cfg(feature = "llm")]
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[cfg(feature = "llm")]
use super::{
    is_promoted_business_insight_type, is_public_sector_entity,
    public_sector_procurement_or_program_case, EntityContext, EvidenceSignal,
};

#[cfg(feature = "llm")]
pub(super) fn contains_public_sector_artifact_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("procurement", 0.20),
            ("tender", 0.20),
            ("consultation", 0.20),
            ("notice", 0.15),
            ("official statement", 0.25),
            ("press release", 0.20),
            ("oversight", 0.20),
            ("approval", 0.15),
            ("directive", 0.20),
            ("regulation", 0.20),
            ("tariff", 0.20),
            ("sanction", 0.20),
            ("export control", 0.25),
            ("program", 0.10),
            ("reserve", 0.10),
            ("framework", 0.15),
            ("ministry", 0.20),
            ("commission", 0.20),
            ("agency", 0.20),
            ("department", 0.20),
            ("portal", 0.10),
            ("licensing", 0.15),
            ("permit", 0.15),
            ("review", 0.10),
            ("hearing", 0.15),
            ("decree", 0.20),
            ("guidance", 0.15),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn contains_public_sector_process_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("approval timing", 0.25),
            ("review window", 0.20),
            ("implementation window", 0.20),
            ("qualification", 0.20),
            ("stakeholder", 0.15),
            ("oversight", 0.20),
            ("committee", 0.20),
            ("procurement scrutiny", 0.25),
            ("supplier pathway", 0.25),
            ("decision timing", 0.25),
            ("account dependency", 0.20),
            ("scenario planning", 0.20),
            ("policy impact", 0.20),
            ("program delay", 0.20),
            ("formal review", 0.20),
            ("consultation", 0.15),
            ("tender", 0.20),
            ("approval owner", 0.25),
            ("stakeholder owner", 0.25),
            ("regulator", 0.15),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
fn public_sector_recommendation_is_concrete(recommendation: &str) -> bool {
    let normalized = normalize_gate_text(recommendation);
    weighted_phrase_score_in_normalized(
        &normalized,
        &[
            ("review the consultation notice", 0.35),
            ("review the notice", 0.30),
            ("map exposed bids", 0.35),
            ("map stakeholder owners", 0.35),
            ("map account dependency", 0.30),
            ("update qualification planning", 0.35),
            ("update qualification plan", 0.35),
            ("verify procurement path", 0.30),
            ("brief account teams", 0.30),
            ("brief teams", 0.20),
            ("identify approval owner", 0.35),
            ("track the review window", 0.30),
            ("scenario planning", 0.25),
            ("stakeholder outreach", 0.20),
            ("policy impact mapping", 0.25),
            ("qualification planning", 0.25),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn has_generic_public_sector_recommendation(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("monitor the situation", 0.30),
            ("continue monitoring", 0.25),
            ("watch for developments", 0.30),
            ("stay close to", 0.20),
            ("explore the opportunity", 0.30),
            ("pursue the opportunity", 0.35),
            ("broad outreach", 0.25),
            ("reach out now", 0.25),
            ("engage broadly", 0.25),
            ("position a manufacturing response", 0.35),
            ("position a response", 0.20),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
pub(super) fn has_formulaic_commercial_language(
    headline: &str,
    narrative: &str,
    recommendation: &str,
) -> bool {
    let headline_normalized = normalize_gate_text(headline);
    let combined_normalized =
        normalize_gate_text(&format!("{} {} {}", headline, narrative, recommendation));
    let explicit_formulaic_headline = headline_normalized.contains("nearshore ems opportunit")
        || headline_normalized.contains("nearshore manufacturing opportunit")
        || headline_normalized.contains("ems outsourcing opportunit");
    let explicit_formulaic_pitch = combined_normalized
        .contains("opportunities for competitors to target")
        || combined_normalized.contains("position us to target")
        || combined_normalized.contains("positioning us to target")
        || combined_normalized.contains("direct alternative for clients");

    let headline_template_score = weighted_phrase_score_in_normalized(
        &headline_normalized,
        &[
            ("strategic outreach opportunity", 0.45),
            ("strategic outreach opportunities", 0.45),
            ("outreach opportunity", 0.30),
            ("outreach opportunities", 0.30),
            ("qualification opportunity", 0.35),
            ("qualification opportunities", 0.35),
            ("nearshore ems opportunity", 0.45),
            ("nearshore ems opportunities", 0.45),
            ("nearshore manufacturing opportunity", 0.40),
            ("nearshore manufacturing opportunities", 0.40),
            ("commercial opening", 0.35),
            ("strategic pivot", 0.35),
            ("strategic shift", 0.30),
            ("presents", 0.10),
            ("offers", 0.10),
        ],
    );
    let narrative_template_score = weighted_phrase_score_in_normalized(
        &combined_normalized,
        &[
            ("signals a strategic pivot", 0.40),
            ("suggests a strategic pivot", 0.40),
            ("reflects a strategic pivot", 0.40),
            ("signals a strategic shift", 0.35),
            ("suggests a strategic shift", 0.35),
            ("reflects a strategic shift", 0.35),
            ("could signal a shift", 0.25),
            ("critical juncture", 0.20),
            ("commercial opening", 0.30),
            ("direct pitch angle", 0.35),
            ("targeted ems partnerships", 0.35),
            ("opportunities for competitors to target", 0.40),
            ("creates opportunities for competitors to target", 0.30),
            ("ems partnerships", 0.20),
            ("aligns with our", 0.25),
            ("position us to", 0.25),
            ("position us to target", 0.20),
            ("positioning us to", 0.25),
            ("positioning us to target", 0.20),
            ("direct alternative for clients", 0.35),
        ],
    );

    explicit_formulaic_headline
        || explicit_formulaic_pitch
        || headline_template_score >= 0.45
        || (headline_template_score >= 0.35
            && (headline_normalized.contains("presents") || headline_normalized.contains("offers")))
        || narrative_template_score >= 0.60
}

#[cfg(feature = "llm")]
pub(super) fn normalize_gate_text(text: &str) -> String {
    let mut normalized = String::new();
    for character in text.nfd() {
        if is_combining_mark(character) {
            continue;
        }

        let folded = match character {
            'ı' | 'İ' => 'i',
            'ſ' => 's',
            'С' | 'с' => 'c',
            'А' | 'а' => 'a',
            'Е' | 'е' => 'e',
            'О' | 'о' => 'o',
            'Р' | 'р' => 'p',
            'Т' | 'т' => 't',
            'Н' | 'н' => 'h',
            'К' | 'к' => 'k',
            'М' | 'м' => 'm',
            'Β' | 'β' => 'b',
            'Ο' | 'ο' => 'o',
            _ => character,
        };

        for lower in folded.to_lowercase() {
            normalized.push(lower);
        }
    }
    normalized
}

#[cfg(feature = "llm")]
pub(super) fn weighted_phrase_score(text: &str, markers: &[(&str, f32)]) -> f32 {
    let normalized = normalize_gate_text(text);
    weighted_phrase_score_in_normalized(&normalized, markers)
}

#[cfg(feature = "llm")]
pub(super) fn weighted_phrase_score_in_normalized(
    normalized: &str,
    markers: &[(&str, f32)],
) -> f32 {
    let normalized_tokens = normalized_token_slices(normalized);

    markers
        .iter()
        .filter(|(marker, _)| marker_matches_normalized(&normalized_tokens, normalized, marker))
        .map(|(_, weight)| *weight)
        .sum::<f32>()
        .min(1.5)
}

#[cfg(feature = "llm")]
fn normalized_token_slices(normalized: &str) -> Vec<&str> {
    normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
}

#[cfg(feature = "llm")]
pub(super) fn marker_matches_normalized(
    normalized_tokens: &[&str],
    normalized: &str,
    marker: &str,
) -> bool {
    if normalized.contains(marker) {
        return true;
    }

    if marker.contains(' ') {
        return false;
    }

    let trimmed = marker.trim();
    if trimmed.is_empty() {
        return false;
    }

    let stem = marker_stem(trimmed);
    normalized_tokens
        .iter()
        .any(|token| token.starts_with(stem))
}

#[cfg(feature = "llm")]
pub(super) fn marker_stem(marker: &str) -> &str {
    if marker.len() <= 4 {
        return marker;
    }

    let stem_len = if marker.len() >= 10 {
        6
    } else if marker.len() >= 7 {
        5
    } else {
        4
    };

    &marker[..stem_len]
}

#[cfg(feature = "llm")]
fn token_matches_marker_token(token: &str, marker_token: &str) -> bool {
    if marker_token.is_empty() {
        return false;
    }

    let stem = marker_stem(marker_token);
    token == marker_token || token.starts_with(stem)
}

#[cfg(feature = "llm")]
fn has_recent_negation(normalized_tokens: &[&str], match_start: usize) -> bool {
    let window_start = match_start.saturating_sub(4);
    let window = &normalized_tokens[window_start..match_start];

    window.iter().any(|token| {
        matches!(
            *token,
            "no" | "not" | "without" | "absent" | "lacks" | "lacking"
        )
    }) || window
        .windows(2)
        .any(|tokens| matches!(tokens, ["lack", "of"] | ["absence", "of"]))
}

#[cfg(feature = "llm")]
fn marker_matches_without_recent_negation(normalized_tokens: &[&str], marker: &str) -> bool {
    let normalized_marker = normalize_gate_text(marker);
    let marker_tokens = normalized_token_slices(&normalized_marker);
    if marker_tokens.is_empty() || normalized_tokens.len() < marker_tokens.len() {
        return false;
    }

    normalized_tokens
        .windows(marker_tokens.len())
        .enumerate()
        .any(|(start, window)| {
            window
                .iter()
                .zip(marker_tokens.iter())
                .all(|(token, marker_token)| token_matches_marker_token(token, marker_token))
                && !has_recent_negation(normalized_tokens, start)
        })
}

#[cfg(feature = "llm")]
fn weighted_phrase_score_without_recent_negation_in_normalized(
    normalized: &str,
    markers: &[(&str, f32)],
) -> f32 {
    let normalized_tokens = normalized_token_slices(normalized);

    markers
        .iter()
        .filter(|(marker, _)| marker_matches_without_recent_negation(&normalized_tokens, marker))
        .map(|(_, weight)| *weight)
        .sum::<f32>()
        .min(1.5)
}

#[cfg(feature = "llm")]
fn parse_temporal_signal_timestamp(date_context: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(date_context)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(date_context, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|naive| naive.and_utc())
        })
}

#[cfg(feature = "llm")]
fn build_temporal_timeline_from_evidence_signals(
    entity_name: &str,
    evidence_signals: &[EvidenceSignal],
) -> Option<EntityTimeline> {
    let mut timeline = EntityTimeline::new(entity_name.to_string());

    for (index, signal) in evidence_signals.iter().enumerate() {
        let Some(date_context) = signal.date_context.as_deref() else {
            continue;
        };
        let Some(observed_at) = parse_temporal_signal_timestamp(date_context) else {
            continue;
        };

        timeline.add_event(
            TimelineEvent::new(
                &signal.signal_type,
                observed_at,
                signal.relevance_score as f64,
            )
            .with_source_observation_id(format!("signal-{index}")),
        );
    }

    (!timeline.events.is_empty()).then_some(timeline)
}

#[cfg(feature = "llm")]
pub(super) fn low_signal_security_hygiene_case(
    category: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    let _ = category;

    let corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    let normalized = normalize_gate_text(&corpus);
    let hygiene_score = weighted_phrase_score_in_normalized(
        &normalized,
        &[
            ("dns posture", 0.35),
            ("dkim", 0.20),
            ("dmarc", 0.20),
            ("spf", 0.20),
            ("lookalike", 0.25),
            ("typosquat", 0.25),
            ("spoof", 0.20),
            ("spoofing", 0.20),
        ],
    );
    let hard_incident_score = weighted_phrase_score_without_recent_negation_in_normalized(
        &normalized,
        &[
            ("breach", 0.35),
            ("compromise", 0.30),
            ("compromised", 0.30),
            ("ransomware", 0.35),
            ("malware", 0.30),
            ("incident", 0.20),
            ("outage", 0.25),
            ("exfiltrat", 0.35),
            ("unauthorized access", 0.35),
            ("account takeover", 0.35),
        ],
    );
    let direct_business_score = weighted_phrase_score_without_recent_negation_in_normalized(
        &normalized,
        &[
            ("audit finding", 0.25),
            ("nonconformance", 0.25),
            ("tender exclusion", 0.35),
            ("contract loss", 0.35),
            ("customer complaint", 0.25),
            ("regulator action", 0.30),
            ("program delay", 0.25),
            ("production halt", 0.35),
            ("supplier removal", 0.35),
            ("export control action", 0.30),
            ("disqualified", 0.35),
        ],
    );

    hygiene_score >= 0.20 && hard_incident_score < 0.20 && direct_business_score < 0.20
}

#[cfg(feature = "llm")]
pub(super) fn contains_causal_link(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("because", 0.45),
            ("therefore", 0.45),
            ("which means", 0.35),
            ("leads to", 0.35),
            ("resulting in", 0.35),
            ("create", 0.25),
            ("creates", 0.25),
            ("creating", 0.25),
            ("undermines", 0.35),
            ("expose", 0.25),
            ("exposes", 0.25),
            ("causing", 0.25),
            ("could face", 0.30),
            ("pushing them to", 0.30),
            ("contributing to", 0.35),
            ("triggering", 0.35),
            ("precipitating", 0.35),
            ("culminating in", 0.35),
            ("giving rise to", 0.35),
            ("on account of", 0.30),
            ("enabling", 0.25),
        ],
    ) >= 0.35
}

#[cfg(feature = "llm")]
pub(super) fn has_temporal_incoherence(
    entity_name: &str,
    narrative: &str,
    reference_time: DateTime<Utc>,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    let Some(timeline) =
        build_temporal_timeline_from_evidence_signals(entity_name, evidence_signals)
    else {
        return false;
    };

    !validate_narrative_temporal_ordering(narrative, &timeline, reference_time).consistent
}

#[cfg(feature = "llm")]
pub(super) fn contains_security_hygiene_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("dns posture", 0.35),
            ("dkim", 0.20),
            ("dmarc", 0.20),
            ("spf", 0.20),
            ("lookalike", 0.25),
            ("spoof", 0.20),
            ("typosquat", 0.25),
            ("security posture weakness", 0.20),
            ("digital risk indicator", 0.20),
            ("vulnerability surface", 0.20),
            ("email authentication gap", 0.25),
            ("mail authentication weakness", 0.25),
            ("domain spoofing exposure", 0.25),
            ("brand impersonation risk", 0.25),
            ("look alike domain", 0.20),
            ("look alike domains", 0.20),
            ("typosquatting campaign", 0.25),
            ("dns hygiene issue", 0.25),
            ("spoofing susceptibility", 0.25),
            ("identity surface exposure", 0.25),
            ("domain trust weakness", 0.20),
            ("email trust weakness", 0.20),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn contains_qualification_or_program_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("as9100", 0.30),
            ("iso 13485", 0.30),
            ("iatf 16949", 0.30),
            ("defense program", 0.25),
            ("eu defense", 0.25),
            ("medical device", 0.25),
            ("regulated program", 0.25),
            ("regulated programs", 0.25),
            ("sensitive program", 0.25),
            ("sensitive programs", 0.25),
            ("program eligibility", 0.30),
            ("qualification", 0.20),
            ("vendor assurance", 0.20),
            ("compliance delays", 0.25),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn contains_customer_disruption_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("supply chain disruption", 0.30),
            ("supply chain disruptions", 0.30),
            ("production interruption", 0.35),
            ("production halt", 0.35),
            ("customer audit cascade", 0.30),
            ("customer concern", 0.20),
            ("customer risk", 0.25),
            ("client risk", 0.25),
            ("clients may seek", 0.25),
            ("client confusion", 0.20),
            ("sourcing review", 0.20),
            ("sourcing reviews", 0.20),
            ("supplier review", 0.20),
            ("supplier reviews", 0.20),
            ("seek ems providers", 0.25),
            ("capture clients", 0.25),
            ("downstream customers", 0.20),
            ("downstream oem accounts", 0.20),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn contains_competitive_displacement_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("opens door", 0.20),
            ("open the door", 0.20),
            ("opportunity for", 0.15),
            ("opportunities for", 0.15),
            ("target their", 0.25),
            ("seek alternatives", 0.25),
            ("safer alternatives", 0.25),
            ("alternative supplier", 0.25),
            ("alternative-supplier", 0.25),
            ("supplier screening", 0.20),
            ("account base", 0.20),
            ("buyer base", 0.20),
            ("customer roster", 0.20),
            ("switch suppliers", 0.30),
            ("switch campaign", 0.30),
            ("customers at risk", 0.20),
            ("nearshore shift", 0.25),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn contains_procurement_or_supplier_access_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("supplier portal", 0.35),
            ("portal registration", 0.35),
            ("supplier registration", 0.35),
            ("supplier onboarding", 0.30),
            ("supplier qualification", 0.30),
            ("qualification package", 0.30),
            ("procurement opportunity", 0.30),
            ("procurement opportunities", 0.30),
            ("procurement requirements", 0.25),
            ("procurement process", 0.25),
            ("procurement processes", 0.25),
            ("register on", 0.20),
            ("ppap", 0.35),
            ("imds", 0.35),
            ("supplier portal registration", 0.40),
        ],
    ) >= 0.30
}

#[cfg(feature = "llm")]
fn contains_certification_commercialization_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("nearshore", 0.25),
            ("nearshoring", 0.25),
            ("outsourcing", 0.25),
            ("outsourcing opportunit", 0.30),
            ("ems opportunit", 0.30),
            ("manufacturing opportunit", 0.30),
            ("qualification opportunit", 0.25),
            ("position our services", 0.25),
            ("offer nearshore manufacturing", 0.30),
            ("target nearshoring", 0.30),
            ("strategic outreach opportunity", 0.25),
            ("direct alternative", 0.25),
            ("alternative provider", 0.25),
            ("alternative providers", 0.25),
            ("alternative ems provider", 0.30),
            ("alternative ems providers", 0.30),
            ("window for competitors", 0.30),
            ("requalify suppliers", 0.25),
            ("re qualify suppliers", 0.25),
            ("switching to a provider", 0.25),
            ("offer a direct alternative", 0.30),
            ("our iso 13485", 0.25),
            ("our iso 9001", 0.25),
            ("our as9100", 0.25),
            ("our facilities", 0.20),
            ("approach ", 0.15),
            ("contact ", 0.15),
            ("engage ", 0.15),
            ("target ", 0.20),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
fn normalize_company_reference(text: &str) -> String {
    let stripped = text.replace("'s", " ").replace("’s", " ");
    let normalized = normalize_gate_text(&stripped);
    normalized
        .split_whitespace()
        .filter(|token| {
            !matches!(
                *token,
                "the"
                    | "a"
                    | "an"
                    | "and"
                    | "or"
                    | "co"
                    | "company"
                    | "companies"
                    | "corp"
                    | "corporation"
                    | "inc"
                    | "ltd"
                    | "limited"
                    | "llc"
                    | "plc"
                    | "gmbh"
                    | "sa"
                    | "ag"
                    | "nv"
                    | "bv"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(feature = "llm")]
fn extract_output_named_company_candidates(text: &str) -> Vec<String> {
    lazy_static::lazy_static! {
        static ref ACTION_TARGET_RE: Regex = Regex::new(
            r"(?:(?i:approach|contact|engage|target|pursue|focus on|work with|for example,|similarly,|like))\s+([A-Z][A-Za-z0-9&.'-]+(?:\s+[A-Z][A-Za-z0-9&.'-]+){1,3})"
        ).unwrap();
        static ref CUSTOMER_APPOSITIVE_RE: Regex = Regex::new(
            r"([A-Z][A-Za-z0-9&.'-]+(?:\s+[A-Z][A-Za-z0-9&.'-]+){1,3})\s*(?:—|,)\s*(?:an?\s+)?(?:[^.\n;]{0,40})\b(?:customer|client|oem|buyer|supplier)\b"
        ).unwrap();
    }

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for regex in [&*ACTION_TARGET_RE, &*CUSTOMER_APPOSITIVE_RE] {
        for caps in regex.captures_iter(text) {
            let Some(raw_candidate) = caps.get(1).map(|m| m.as_str()) else {
                continue;
            };
            let normalized = normalize_company_reference(raw_candidate);
            if normalized.split_whitespace().count() < 2 {
                continue;
            }
            if seen.insert(normalized.clone()) {
                candidates.push(normalized);
            }
        }
    }

    candidates
}

#[cfg(feature = "llm")]
fn has_concrete_commercial_trigger(text: &str) -> bool {
    let normalized = normalize_gate_text(text);
    normalized.contains("failed audit")
        || normalized.contains("revoked")
        || normalized.contains("suspended")
        || normalized.contains("withdrawn")
        || normalized.contains("supplier removed")
        || normalized.contains("disqualified")
        || normalized.contains("tender exclusion")
        || normalized.contains("major nonconformance")
        || normalized.contains("major non-conformance")
        || normalized.contains("warning letter")
        || normalized.contains("regulator action")
}

#[cfg(feature = "llm")]
fn has_soft_or_generic_certification_context(text: &str) -> bool {
    let normalized = normalize_gate_text(text);
    let has_certification_marker = normalized.contains("certif")
        || normalized.contains("certificate")
        || normalized.contains("accredit")
        || normalized.contains("iso ")
        || normalized.contains("as9100")
        || normalized.contains("iatf");
    let has_soft_context_marker = normalized.contains("compliance")
        || normalized.contains("renewal")
        || normalized.contains("renewed")
        || normalized.contains("update")
        || normalized.contains("updated")
        || normalized.contains("deadline")
        || normalized.contains("expire")
        || normalized.contains("expires")
        || normalized.contains("valid until")
        || normalized.contains("warning")
        || normalized.contains("flagged");

    has_certification_marker && has_soft_context_marker
}

#[cfg(feature = "llm")]
pub(super) fn has_unsupported_security_escalation(
    category: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !low_signal_security_hygiene_case(category, evidence_signals) {
        return false;
    }

    let combined = normalize_gate_text(&format!("{} {}", narrative, recommendation));
    let conflates_hygiene_and_qualification = contains_security_hygiene_marker(&combined)
        && contains_qualification_or_program_marker(&combined)
        && contains_causal_link(&combined);
    let unsupported_customer_impact = contains_security_hygiene_marker(&combined)
        && contains_customer_disruption_marker(&combined);
    let unsupported_competitive_displacement = contains_security_hygiene_marker(&combined)
        && contains_competitive_displacement_marker(&combined);
    let unsupported_procurement_conversion =
        matches!(category, "security_compliance" | "cybersecurity_threat")
            && contains_security_hygiene_marker(&combined)
            && contains_procurement_or_supplier_access_marker(&combined);

    conflates_hygiene_and_qualification
        || unsupported_customer_impact
        || unsupported_competitive_displacement
        || unsupported_procurement_conversion
}

#[cfg(feature = "llm")]
pub(super) fn low_signal_certification_warning_case(evidence_signals: &[EvidenceSignal]) -> bool {
    let normalized_signals = normalized_evidence_signal_texts(evidence_signals);
    // Battery/energy-storage safety certifications (UN 38.3, IEC 62619, IEC 62133,
    // UL 1973, UL 9540, EN 50604, ...) are first-class commercial intelligence for
    // the BESS line of business, not low-signal compliance noise: a competitor
    // obtaining them signals market entry, and a prospect requiring them signals a
    // qualification window. Never treat warnings anchored to them as low-signal.
    if has_battery_safety_certification_signal(&normalized_signals) {
        return false;
    }
    let corpus = normalized_signals.join(" ");
    let has_soft_certification_signal = has_soft_certification_signal(&normalized_signals);
    let has_non_certification_risk_markers = has_non_certification_risk_markers(&corpus);
    let has_hard_failure_markers = has_hard_certification_failure_markers(&corpus);

    has_soft_certification_signal
        && !has_hard_failure_markers
        && !has_non_certification_risk_markers
}

/// Returns true when any evidence signal references a battery/energy-storage
/// safety or performance certification. These standards are highly specific, so
/// substring matching on the lowercased (digit-preserving) normalized text is
/// both precise and robust to spacing/punctuation variants.
#[cfg(feature = "llm")]
fn has_battery_safety_certification_signal(normalized_signals: &[String]) -> bool {
    const BATTERY_CERT_MARKERS: &[&str] = &[
        "un38.3", "un 38.3", "un 38 3", "un383",
        "iec 62619", "iec62619",
        "iec 62133", "iec62133",
        "iec 61427", "iec61427",
        "iec 62660", "iec62660",
        "ul 1973", "ul1973",
        "ul 9540", "ul9540",
        "en 50604", "en50604",
    ];
    normalized_signals.iter().any(|signal| {
        BATTERY_CERT_MARKERS
            .iter()
            .any(|marker| signal.contains(marker))
    })
}

#[cfg(feature = "llm")]
fn normalized_evidence_signal_texts(evidence_signals: &[EvidenceSignal]) -> Vec<String> {
    evidence_signals
        .iter()
        .map(|signal| {
            let combined = format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            );
            normalize_gate_text(&combined)
        })
        .collect::<Vec<_>>()
}

#[cfg(feature = "llm")]
fn has_soft_certification_signal(normalized_signals: &[String]) -> bool {
    normalized_signals.iter().any(|signal| {
        let certification_score = weighted_phrase_score_in_normalized(
            signal,
            &[
                ("certificate", 0.20),
                ("certification", 0.25),
                ("accreditation", 0.25),
                ("iso ", 0.25),
                ("as9100", 0.30),
                ("iatf", 0.25),
            ],
        );
        let soft_warning_score = weighted_phrase_score_in_normalized(
            signal,
            &[
                ("warning", 0.25),
                ("flagged", 0.20),
                ("outdated", 0.20),
                ("update", 0.15),
                ("updated", 0.15),
                ("reaffirmation", 0.20),
                ("reaffirmed", 0.20),
                ("renewal", 0.20),
                ("renewed", 0.20),
                ("valid until", 0.20),
                ("detected", 0.15),
            ],
        );

        certification_score >= 0.20 && soft_warning_score >= 0.15
    })
}

#[cfg(feature = "llm")]
fn has_non_certification_risk_markers(corpus: &str) -> bool {
    weighted_phrase_score(
        corpus,
        &[
            ("supply chain", 0.20),
            ("shortage", 0.20),
            ("tariff", 0.20),
            ("cybersecurity", 0.20),
            ("dns", 0.15),
            ("lookalike", 0.20),
            ("acquisition", 0.20),
            ("merger", 0.20),
            ("pricing", 0.15),
            ("hiring", 0.15),
            ("procurement", 0.15),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
fn has_hard_certification_failure_markers(corpus: &str) -> bool {
    let normalized = normalize_gate_text(corpus);
    weighted_phrase_score_without_recent_negation_in_normalized(
        &normalized,
        &[
            ("revoked", 0.35),
            ("suspended", 0.35),
            ("withdrawn", 0.35),
            ("decertified", 0.35),
            ("failed audit", 0.35),
            ("audit finding", 0.20),
            ("major nonconformance", 0.35),
            ("major non-conformance", 0.35),
            ("nonconformity", 0.25),
            ("non-conformity", 0.25),
            ("tender exclusion", 0.35),
            ("regulator action", 0.30),
            ("warning letter", 0.35),
            ("certificate expired", 0.35),
            ("certification expired", 0.35),
            ("expired on", 0.20),
            ("supplier removal", 0.35),
            ("disqualified", 0.35),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
pub(super) fn contains_soft_certification_pressure_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("certification gap", 0.30),
            ("certification gaps", 0.30),
            ("certification warning", 0.30),
            ("compliance warning", 0.30),
            ("compliance gap", 0.30),
            ("compliance issue", 0.30),
            ("compliance issues", 0.30),
            ("certification issue", 0.30),
            ("certification issues", 0.30),
            ("iatf gap", 0.35),
            ("iatf gaps", 0.35),
            ("certification notice", 0.25),
            ("certification update", 0.25),
            ("certification risk", 0.30),
            ("certification risks", 0.30),
            ("compliance risk", 0.30),
            ("compliance risks", 0.30),
            ("regulatory concern", 0.25),
            ("standards lapse", 0.25),
            ("accreditation gap", 0.25),
            ("certification deficiency", 0.25),
            ("compliance shortfall", 0.25),
            ("nearing expiration", 0.25),
            ("expiration on", 0.25),
            ("standards non conformance", 0.25),
            ("audit finding", 0.20),
            ("regulatory exposure", 0.25),
            ("certification deadline", 0.35),
            ("compliance deadline", 0.30),
            ("iso 9001 deadline", 0.35),
            ("as9100 deadline", 0.35),
            ("iatf deadline", 0.35),
            ("expiration timeline", 0.25),
            ("renewal timeline", 0.25),
            ("expires in", 0.25),
            ("expires on", 0.25),
        ],
    ) >= 0.30
}

#[cfg(feature = "llm")]
fn contains_bom_or_delivery_disruption_marker(text: &str) -> bool {
    weighted_phrase_score(
        text,
        &[
            ("supply chain risk", 0.25),
            ("supply chain risks", 0.25),
            ("delivery disruption", 0.30),
            ("delivery disruptions", 0.30),
            ("delivery risk", 0.25),
            ("delivery risks", 0.25),
            ("bom impact", 0.25),
            ("bom impacts", 0.25),
            ("bom revision", 0.30),
            ("bom revisions", 0.30),
            ("alternate part qualification", 0.35),
            ("alternate-part qualification", 0.35),
            ("alternate part", 0.20),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
pub(super) fn has_unsupported_certification_escalation(
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    let combined = normalize_gate_text(&format!("{} {}", narrative, recommendation));
    let unsupported_qualification =
        contains_qualification_or_program_marker(&combined) && contains_causal_link(&combined);
    let unsupported_customer_impact = contains_customer_disruption_marker(&combined)
        || contains_bom_or_delivery_disruption_marker(&combined);
    let unsupported_competitive_displacement = contains_competitive_displacement_marker(&combined);
    let normalized_signals = normalized_evidence_signal_texts(evidence_signals);
    let evidence_corpus = normalized_signals.join(" ");
    let low_signal_certification_case = low_signal_certification_warning_case(evidence_signals);
    let mixed_or_output_soft_certification_escalation =
        !has_hard_certification_failure_markers(&evidence_corpus)
            && contains_soft_certification_pressure_marker(&combined)
            && (unsupported_qualification
                || unsupported_customer_impact
                || unsupported_competitive_displacement);

    (low_signal_certification_case
        && (unsupported_qualification
            || unsupported_customer_impact
            || unsupported_competitive_displacement))
        || mixed_or_output_soft_certification_escalation
}

#[cfg(feature = "llm")]
pub(super) fn has_unsupported_certification_commercialization(
    headline: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    let evidence_corpus = normalize_gate_text(
        &evidence_signals
            .iter()
            .map(|signal| {
                format!(
                    "{} {} {} {}",
                    signal.title,
                    signal.description,
                    signal.signal_type,
                    signal.extracted_facts.join(" ")
                )
            })
            .collect::<Vec<_>>()
            .join(" "),
    );
    let combined = normalize_gate_text(&format!("{} {} {}", headline, narrative, recommendation));
    let output_has_soft_certification_context =
        contains_soft_certification_pressure_marker(&combined)
            && (combined.contains("certif")
                || combined.contains("certificate")
                || combined.contains("accredit")
                || combined.contains("iso ")
                || combined.contains("as9100")
                || combined.contains("iatf"));

    if !(has_soft_or_generic_certification_context(&evidence_corpus)
        || output_has_soft_certification_context)
        || has_concrete_commercial_trigger(&evidence_corpus)
    {
        return false;
    }

    contains_certification_commercialization_marker(&combined)
}

#[cfg(feature = "llm")]
pub(super) fn has_unsupported_named_target_provenance(
    entity_ctx: &EntityContext,
    headline: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    let support_corpus = normalize_company_reference(&format!(
        "{} {} {} {} {}",
        entity_ctx.name,
        entity_ctx.competitor_names.join(" "),
        entity_ctx.recent_changes.join(" "),
        entity_ctx.sites_summary.join(" "),
        evidence_signals
            .iter()
            .map(|signal| {
                format!(
                    "{} {} {} {}",
                    signal.title,
                    signal.description,
                    signal.signal_type,
                    signal.extracted_facts.join(" ")
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    ));
    let entity_name = normalize_company_reference(&entity_ctx.name);
    let output_text = format!("{} {} {}", headline, narrative, recommendation);

    extract_output_named_company_candidates(&output_text)
        .into_iter()
        .any(|candidate| candidate != entity_name && !support_corpus.contains(&candidate))
}

#[cfg(feature = "llm")]
fn recommendation_targets_named_internal_stakeholder(recommendation: &str) -> bool {
    let recommendation_lower = normalize_gate_text(recommendation);

    let internal_role_score = weighted_phrase_score_in_normalized(
        &recommendation_lower,
        &[
            ("procurement lead", 0.30),
            ("procurement team", 0.25),
            ("sourcing team", 0.25),
            ("supply chain team", 0.25),
            ("operations team", 0.20),
            ("engineering team", 0.20),
            ("program team", 0.20),
            ("automotive division", 0.25),
            ("medical division", 0.25),
            ("industrial division", 0.25),
            ("defense division", 0.25),
            ("procurement manager", 0.30),
            ("category manager", 0.25),
            ("buyer", 0.15),
        ],
    );

    let named_target_shape = [" at ", "'s ", " team ", " division"]
        .iter()
        .filter(|marker| recommendation_lower.contains(**marker))
        .count() as f32;

    let direct_action_score = weighted_phrase_score_in_normalized(
        &recommendation_lower,
        &[
            ("contact ", 0.25),
            ("approach ", 0.25),
            ("engage ", 0.20),
            ("propose ", 0.20),
            ("schedule ", 0.20),
        ],
    );

    internal_role_score >= 0.15 && named_target_shape >= 1.0 && direct_action_score >= 0.20
}

#[cfg(feature = "llm")]
fn recommendation_mentions_quarter_shorthand(recommendation_lower: &str) -> bool {
    recommendation_lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|window| {
            matches!(window[0], "q1" | "q2" | "q3" | "q4")
                && window[1].len() == 4
                && window[1].starts_with("20")
                && window[1]
                    .chars()
                    .all(|character| character.is_ascii_digit())
        })
}

#[cfg(feature = "llm")]
pub(super) fn has_unnamed_customer_targeting(narrative: &str, recommendation: &str) -> bool {
    let combined = format!("{} {}", narrative, recommendation);
    let targeting_lower = normalize_gate_text(&combined);

    if recommendation_targets_named_internal_stakeholder(recommendation) {
        return false;
    }

    weighted_phrase_score_in_normalized(
        &targeting_lower,
        &[
            ("target their ", 0.35),
            ("target its ", 0.35),
            ("go after their ", 0.35),
            ("pursue their ", 0.35),
            ("focus on their ", 0.30),
            ("their medical clients", 0.25),
            ("their industrial clients", 0.25),
            ("their aerospace clients", 0.25),
            ("their defense clients", 0.25),
            ("their automotive customers", 0.25),
            ("their medical device customers", 0.30),
            ("their medical device clients", 0.30),
            ("their buyer base", 0.20),
            ("their account base", 0.20),
            ("aerospace and medical customers", 0.30),
            ("medical and aerospace customers", 0.30),
            ("eu medical device customers", 0.25),
            ("industrial clients", 0.15),
            ("aerospace clients", 0.15),
            ("defense customers", 0.15),
            ("medical customers", 0.15),
            ("medical device customers", 0.20),
            ("medical device clients", 0.20),
            ("medical device manufacturers", 0.20),
            ("automotive clients", 0.20),
            ("industrial customers", 0.20),
            ("automotive customers", 0.15),
            ("customers at risk", 0.20),
            ("underserved customers", 0.20),
            ("downstream oem accounts", 0.25),
            ("client portfolio", 0.20),
            ("end customer base", 0.20),
            ("downstream buyer set", 0.20),
            ("customer roster", 0.20),
            ("account portfolio", 0.20),
            ("undisclosed buyer", 0.25),
            ("unnamed accounts", 0.25),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
pub(super) fn recommendation_has_action_timing(category: &str, recommendation: &str) -> bool {
    let recommendation_lower = normalize_gate_text(recommendation);
    if weighted_phrase_score_in_normalized(
        &recommendation_lower,
        &[(" by ", 0.25), (" within ", 0.25), (" before ", 0.25)],
    ) >= 0.25
    {
        return true;
    }

    if recommendation_mentions_quarter_shorthand(&recommendation_lower) {
        return true;
    }

    is_promoted_business_insight_type(Some(category))
        && weighted_phrase_score_in_normalized(
            &recommendation_lower,
            &[
                ("this week", 0.20),
                ("this month", 0.20),
                ("this quarter", 0.25),
                ("next quarter", 0.25),
                ("next sourcing cycle", 0.25),
                ("next supplier review", 0.25),
                ("renewal cycle", 0.20),
                ("before renewal", 0.25),
                ("ahead of renewal", 0.25),
                ("immediately", 0.20),
                ("right now", 0.20),
                ("ahead of", 0.20),
                ("in the next 30 days", 0.25),
                ("in the next 60 days", 0.25),
            ],
        ) >= 0.20
}

#[cfg(feature = "llm")]
pub(super) fn has_unsupported_public_sector_commercialization(
    entity_ctx: &EntityContext,
    category: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref()) {
        return false;
    }

    if !matches!(
        category,
        "geopolitical_analysis" | "regulatory_policy" | "brand_sentiment"
    ) {
        return false;
    }

    if public_sector_procurement_or_program_case(evidence_signals) {
        return false;
    }

    let combined = format!("{} {}", narrative, recommendation);
    weighted_phrase_score(
        &combined,
        &[
            ("pcba", 0.30),
            ("pcb assembly", 0.30),
            ("box build", 0.30),
            ("electronics manufacturing services", 0.30),
            ("qualified to supply", 0.30),
            ("submit qualification package", 0.35),
            ("procurement team", 0.20),
            ("offer pcba", 0.30),
            ("offer supply chain management", 0.25),
            ("offer manufacturing", 0.25),
            ("defense-adjacent", 0.25),
            ("north african eu trade corridors", 0.25),
            ("north african eu trade corridor", 0.25),
            ("local manufacturing", 0.20),
            ("manufacturing response", 0.25),
            ("direct outreach", 0.25),
            ("supplier outreach", 0.25),
            ("offer contract manufacturing", 0.30),
            ("offer electronics assembly", 0.30),
            ("position our services", 0.25),
            ("lead with manufacturing", 0.25),
        ],
    ) >= 0.20
}

#[cfg(feature = "llm")]
pub(super) fn has_low_usefulness_public_sector_analysis(
    entity_ctx: &EntityContext,
    category: &str,
    headline: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref()) {
        return false;
    }

    if !matches!(
        category,
        "geopolitical_analysis" | "regulatory_policy" | "brand_sentiment"
    ) {
        return false;
    }

    if public_sector_procurement_or_program_case(evidence_signals) {
        return false;
    }

    let evidence_corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let output_corpus = format!("{} {} {}", headline, narrative, recommendation);
    let recommendation_lower = normalize_gate_text(recommendation);

    let evidence_has_artifact = contains_public_sector_artifact_marker(&evidence_corpus);
    let output_has_artifact = contains_public_sector_artifact_marker(&output_corpus)
        || weighted_phrase_score_in_normalized(
            &recommendation_lower,
            &[
                ("account dependency", 0.20),
                ("stakeholder", 0.15),
                ("qualification planning", 0.25),
                ("policy impact", 0.20),
                ("approval timing", 0.20),
                ("procurement scrutiny", 0.20),
            ],
        ) >= 0.15;
    let output_has_process = contains_public_sector_process_marker(&output_corpus);
    let narrative_has_causal_link = contains_causal_link(narrative);
    let recommendation_is_concrete = public_sector_recommendation_is_concrete(recommendation);

    let generic_opportunity_language = weighted_phrase_score(
        &output_corpus,
        &[
            ("strategic outreach opportunity", 0.30),
            ("nearshoring opportunity", 0.30),
            ("opens ems opportunities", 0.35),
            ("ems opportunities", 0.30),
            ("open for ems partnerships", 0.35),
            ("supply chain partnerships", 0.25),
            ("partners for defense supply chain resilience", 0.35),
            ("opens nearshoring", 0.30),
            ("opportunity for eu/na ems", 0.35),
            ("manufacturing response", 0.30),
            ("direct supplier opportunity", 0.30),
        ],
    ) >= 0.25;
    let generic_recommendation_language = has_generic_public_sector_recommendation(recommendation);

    !evidence_has_artifact
        || !output_has_artifact
        || !output_has_process
        || !narrative_has_causal_link
        || !recommendation_is_concrete
        || generic_opportunity_language
        || generic_recommendation_language
}
