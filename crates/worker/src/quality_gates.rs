#[cfg(feature = "llm")]
use apex_core::timeline::{EntityTimeline, TimelineEvent};
#[cfg(feature = "llm")]
use apex_llm::insight_gen::validate_narrative_temporal_ordering;
#[cfg(feature = "llm")]
use chrono::{DateTime, Utc};
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
pub(super) fn weighted_phrase_score_in_normalized(normalized: &str, markers: &[(&str, f32)]) -> f32 {
    let normalized_tokens = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    markers
        .iter()
        .filter(|(marker, _)| marker_matches_normalized(&normalized_tokens, normalized, marker))
        .map(|(_, weight)| *weight)
        .sum::<f32>()
        .min(1.5)
}

#[cfg(feature = "llm")]
pub(super) fn marker_matches_normalized(normalized_tokens: &[&str], normalized: &str, marker: &str) -> bool {
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
    normalized_tokens.iter().any(|token| token.starts_with(stem))
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
            TimelineEvent::new(&signal.signal_type, observed_at, signal.relevance_score as f64)
                .with_source_observation_id(format!("signal-{index}")),
        );
    }

    (!timeline.events.is_empty()).then_some(timeline)
}

#[cfg(feature = "llm")]
pub(super) fn low_signal_security_hygiene_case(category: &str, evidence_signals: &[EvidenceSignal]) -> bool {
    if !matches!(category, "security_compliance" | "cybersecurity_threat") {
        return false;
    }

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
    let hard_incident_score = weighted_phrase_score_in_normalized(
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
    let direct_business_score = weighted_phrase_score_in_normalized(
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
    let Some(timeline) = build_temporal_timeline_from_evidence_signals(entity_name, evidence_signals)
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

    conflates_hygiene_and_qualification || unsupported_customer_impact
}

#[cfg(feature = "llm")]
pub(super) fn low_signal_certification_warning_case(evidence_signals: &[EvidenceSignal]) -> bool {
    let normalized_signals = normalized_evidence_signal_texts(evidence_signals);
    let corpus = normalized_signals.join(" ");
    let has_soft_certification_signal = has_soft_certification_signal(&normalized_signals);
    let has_non_certification_risk_markers = has_non_certification_risk_markers(&corpus);
    let has_hard_failure_markers = has_hard_certification_failure_markers(&corpus);

    has_soft_certification_signal && !has_hard_failure_markers && !has_non_certification_risk_markers
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
    weighted_phrase_score(
        corpus,
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
            ("certification warning", 0.30),
            ("compliance warning", 0.30),
            ("compliance gap", 0.30),
            ("compliance issue", 0.30),
            ("compliance issues", 0.30),
            ("certification issue", 0.30),
            ("certification issues", 0.30),
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
            ("standards non conformance", 0.25),
            ("audit finding", 0.20),
            ("regulatory exposure", 0.25),
        ],
    ) >= 0.30
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
    let unsupported_customer_impact = contains_customer_disruption_marker(&combined);
    let unsupported_competitive_displacement = contains_competitive_displacement_marker(&combined);
    let normalized_signals = normalized_evidence_signal_texts(evidence_signals);
    let evidence_corpus = normalized_signals.join(" ");
    let low_signal_certification_case = low_signal_certification_warning_case(evidence_signals);
    let mixed_soft_certification_escalation = has_soft_certification_signal(&normalized_signals)
        && !has_hard_certification_failure_markers(&evidence_corpus)
        && contains_soft_certification_pressure_marker(&combined)
        && (unsupported_qualification
            || unsupported_customer_impact
            || unsupported_competitive_displacement);

    (low_signal_certification_case
        && (unsupported_qualification
            || unsupported_customer_impact
            || unsupported_competitive_displacement))
        || mixed_soft_certification_escalation
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
    recommendation_lower.split_whitespace().collect::<Vec<_>>().windows(2).any(
        |window| {
            matches!(window[0], "q1" | "q2" | "q3" | "q4")
                && window[1].len() == 4
                && window[1].starts_with("20")
                && window[1].chars().all(|character| character.is_ascii_digit())
        },
    )
}

#[cfg(feature = "llm")]
pub(super) fn has_unnamed_customer_targeting(narrative: &str, recommendation: &str) -> bool {
    let _ = narrative;
    let recommendation_lower = normalize_gate_text(recommendation);

    if recommendation_targets_named_internal_stakeholder(recommendation) {
        return false;
    }

    weighted_phrase_score_in_normalized(
        &recommendation_lower,
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
            ("their buyer base", 0.20),
            ("their account base", 0.20),
            ("eu medical device customers", 0.25),
            ("industrial clients", 0.15),
            ("aerospace clients", 0.15),
            ("defense customers", 0.15),
            ("medical customers", 0.15),
            ("medical device customers", 0.20),
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