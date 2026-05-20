#![cfg_attr(test, allow(dead_code))]

//! Template-based fallback insight generation when LLM is unavailable.
//!
//! Contains functions for building fallback summaries, goal-oriented suggestions,
//! and analytical narratives without LLM dependency.

use super::is_public_sector_entity;

/// Filter signal details to only concrete, non-aggregate entries.
pub(crate) fn concrete_signal_details(signal_details: &[String]) -> Vec<String> {
    signal_details
        .iter()
        .filter(|detail| !is_aggregate_metric_signal_detail(detail))
        .map(|detail| detail.trim())
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.trim_end_matches('.').to_string())
        .collect()
}

/// Check if a signal detail is just an aggregate metric (e.g., "5 job posting(s) observed").
pub(super) fn is_aggregate_metric_signal_detail(detail: &str) -> bool {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let aggregate_suffixes = [
        "job posting(s) observed",
        "patent(s) filed",
        "tender(s) identified",
        "certification(s) on record",
        "competitor signal(s)",
        "news article(s) referenced",
        "web change(s) detected",
        "regulatory filing(s)",
        "sanction entry(ies)",
        "trade show participation(s)",
        "person(s) of interest tracked",
    ];

    trimmed
        .chars()
        .next()
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(false)
        && aggregate_suffixes
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

/// Count concrete (non-aggregate) signal details.
pub(crate) fn count_concrete_signal_details(signal_details: &[String]) -> usize {
    concrete_signal_details(signal_details).len()
}

fn is_security_signal_detail(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("lookalike domain")
        || lower.contains("dns posture")
        || lower.contains("known exploited vulnerability")
        || lower.contains("kev-linked vulnerability")
        || lower.contains("cve-")
        || lower.contains("spoof")
}

pub(crate) fn category_relevant_signal_details(
    category: &str,
    signal_details: &[String],
) -> Vec<String> {
    let concrete = concrete_signal_details(signal_details);
    let security_category = matches!(category, "security_compliance" | "cybersecurity_threat");

    if security_category {
        return concrete;
    }

    concrete
        .into_iter()
        .filter(|detail| !is_security_signal_detail(detail))
        .collect()
}

/// Extract concrete action fragments from a rendered action hint.
pub(super) fn concrete_action_fragments(action_hint: &str) -> Vec<String> {
    action_hint
        .split([';', '.'])
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .filter(|fragment| !is_generic_action_hint(fragment))
        .map(|fragment| fragment.trim_end_matches('.').to_string())
        .take(2)
        .collect()
}

/// Check if an action hint is too generic to be useful.
pub(super) fn is_generic_action_hint(fragment: &str) -> bool {
    fn normalize_gate_text(text: &str) -> String {
        text.to_ascii_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                    character
                } else {
                    ' '
                }
            })
            .collect()
    }

    fn stemmed_starts_with(token: &str, stems: &[&str]) -> bool {
        stems.iter().any(|stem| token.starts_with(stem))
    }

    let trimmed = fragment.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = normalize_gate_text(trimmed);
    let normalized = lower.replace('-', " ");
    let first_token = normalized.split_whitespace().next().unwrap_or_default();
    let generic_starters = [
        "monitor",
        "identify",
        "assess",
        "track",
        "review",
        "evaluate",
        "analy",
        "watch",
        "map",
        "brief",
        "gather",
        "update",
        "compare",
        "convene",
        "coordinate",
        "prepare",
        "scenario",
        "escalate",
        "align",
        "document",
        "priorit",
        "initiat",
        "alert",
        "verify",
        "adjust",
        "reprioritize",
        "continu",
        "investigat",
        "explor",
        "consider",
        "examin",
        "look",
        "maintain",
        "stay",
    ];
    let starts_generic = stemmed_starts_with(first_token, &generic_starters);
    if !starts_generic {
        return false;
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let has_domain_like_marker = words.iter().any(|word| {
        let candidate = word
            .trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '.' && character != '-'
            })
            .to_ascii_lowercase();
        candidate.contains('.')
            && candidate.split('.').count() >= 2
            && candidate
                .rsplit('.')
                .next()
                .map(|suffix| {
                    suffix.len() >= 2
                        && suffix
                            .chars()
                            .all(|character| character.is_ascii_alphabetic())
                })
                .unwrap_or(false)
    });

    let has_specific_marker = trimmed.chars().any(|character| character.is_ascii_digit())
        || trimmed.contains('%')
        || trimmed.contains("http://")
        || trimmed.contains("https://")
        || lower.contains("cve-")
        || has_domain_like_marker
        || words.iter().skip(1).any(|word| {
            word.chars()
                .next()
                .map(|character| character.is_ascii_uppercase())
                .unwrap_or(false)
                && word.len() > 2
        });

    if has_specific_marker {
        return false;
    }

    let generic_business_markers = [
        "risk assessment",
        "cross functional",
        "cross-functional",
        "operational disruption",
        "scenario plan",
        "scenario-plan",
        "contingency planning",
        "leadership review",
        "stakeholder alignment",
        "customer impact",
        "topic drivers",
        "monitoring cadence",
        "suspicious domains",
        "takedown procedures",
        "customer facing teams",
        "customer-facing teams",
        "authentication scrutiny",
        "vendor security assessment",
        "data handling practices",
        "incident response procedures",
        "network segmentation",
        "security frameworks",
        "official domains",
        "procurement portals",
        "citizen facing services",
        "citizen-facing services",
    ];

    words.len() <= 8
        || generic_business_markers
            .iter()
            .any(|marker| normalized.contains(marker))
}

/// Determine if a fallback insight should be emitted.
pub(super) fn should_emit_fallback_insight(
    category: &str,
    signal_details: &[String],
    rendered_action: &str,
    confidence: f64,
    evidence_count: usize,
) -> bool {
    let concrete_signals = category_relevant_signal_details(category, signal_details).len();
    let concrete_actions = concrete_action_fragments(rendered_action);

    // Aggregate counters alone do not justify a user-facing fallback insight.
    if concrete_signals == 0 {
        return false;
    }

    if concrete_actions.is_empty() && confidence < 0.60 && evidence_count < 3 {
        return false;
    }

    true
}

/// Extract fallback signal details from a summary string.
pub(crate) fn extract_fallback_signal_details(summary: &str) -> Vec<String> {
    let lower_summary = summary.to_ascii_lowercase();
    let (start, marker_len) = if let Some(start) = lower_summary.find("our monitoring detected:") {
        (start, "our monitoring detected:".len())
    } else if let Some(start) = lower_summary.find("observed signals include:") {
        (start, "observed signals include:".len())
    } else {
        return Vec::new();
    };

    let detected = &summary[start + marker_len..];
    let first_sentence = detected.split('\n').next().unwrap_or(detected);
    let first_sentence = first_sentence.split(". ").next().unwrap_or(first_sentence);

    first_sentence
        .split(';')
        .map(str::trim)
        .map(|detail| detail.trim_end_matches('.').trim())
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.to_string())
        .collect()
}

/// Format a sources footer from a list of URLs.
fn source_domain_from_url(url: &str) -> String {
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

pub(super) fn format_sources_footer_from_urls(urls: &[String], max_sources: usize) -> String {
    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for url in urls
        .iter()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
    {
        let key = url.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(url.to_string());
            if deduped.len() >= max_sources {
                break;
            }
        }
    }

    if deduped.is_empty() {
        return String::new();
    }

    let formatted: Vec<String> = deduped
        .iter()
        .enumerate()
        .map(|(idx, url)| format!("[{}] {} — {}", idx + 1, source_domain_from_url(url), url))
        .collect();

    format!("Sources:\n{}", formatted.join("\n"))
}

/// Build goal-oriented suggestions for fallback insights.
pub(super) fn build_goal_oriented_suggestions(
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    signal_details: &[String],
    action_hint: Option<&str>,
) -> Vec<String> {
    let flags =
        detect_strategy_signal_flags(category, signal_details, action_hint.unwrap_or_default());
    let is_public_sector = is_public_sector_entity(entity_label, entity_type);
    let entity = if entity_label.is_empty() {
        "this account"
    } else {
        entity_label
    };
    let region_note = if entity_region.is_empty() {
        String::new()
    } else {
        format!(" in {}", entity_region)
    };

    let mut suggestions = Vec::new();

    match (is_public_sector, category) {
        (true, "brand_sentiment") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Treat sentiment around {} as actionable only if it starts to change tender scrutiny, policy credibility, oversight pressure, or approval timing{}.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Separate a short media cycle from a real institutional issue by checking whether the discussion around {} is leading to formal review, parliamentary attention, procurement caution, or stakeholder messaging changes.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Brief account teams only on bids, regulated customers, or public-sector programs that depend on decisions from {} and could slow down if scrutiny hardens.",
                entity
            ));
        }
        (true, "regulatory_policy") | (true, "geopolitical_analysis") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Treat the signal around {}{} as material only when it resolves into a concrete policy artifact such as an official statement, tender amendment, export-control move, or oversight action.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Escalate only when the evidence names the affected institution, program, border measure, or approval flow tied to {} rather than translating broad geopolitical noise into an operations playbook.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Use the next review to confirm what specific rule, corridor, or procurement process could move next at {} before changing routing, qualification, or supply assumptions.",
                entity
            ));
        }
        (true, "security_compliance")
        | (true, "cybersecurity_threat")
        | (true, "quality_compliance") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Check whether the signal around {} reaches official domains, citizen-facing services, procurement portals, or supplier access paths before escalating it as a government-wide incident.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, "If a concrete domain, host, CVE, or ministry system is involved, route that exact artifact into takedown, remediation, or access-control work rather than relying on a generic cyber playbook.".to_string());
            push_unique_suggestion(&mut suggestions, format!(
                "Brief stakeholders only on the specific public services, agencies, or vendor workflows that would be affected if the security signal around {} is confirmed.",
                entity
            ));
        }
        (true, _) => {
            push_unique_suggestion(&mut suggestions, format!(
                "Use this signal around {} to test whether procurement scrutiny, approval timing, stakeholder access, or policy posture is actually changing rather than assuming it is a direct sales trigger.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Map which bids, regulated customers, or public programs are exposed to decisions from {}{} and prioritize only those with real timing or compliance consequences.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Brief teams on what concrete institutional decision could move next at {}, who owns that decision, and what evidence would justify escalation beyond background monitoring.",
                entity
            ));
        }
        (false, "demand_procurement") | (false, "customer_rfq") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is revenue capture, use the current signal window to get in front of {} with a capability-led offer{} before the sourcing shortlist hardens.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is qualification readiness, line up the audit pack, certification evidence, and sector-specific onboarding material that {} is likely to request.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is footprint positioning, test whether an EU or North Africa manufacturing angle gives {} a better resilience, tariff, or lead-time story than incumbent supply.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is early design influence, use prototype, NPI, or engineering-support language rather than waiting for a fully formal RFQ cycle at {}.",
                entity
            ));
        }
        (false, "supply_chain_risk") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is continuity protection, map the single-source and long-lead exposures around {} first and decide where dual-source qualification or inventory buffers matter most.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is customer assurance, turn the signal into a concrete fallback story: what can be rerouted, requalified, or migrated before schedules slip for programs touching {}{}?",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is commercial upside, use the disruption around {} to open conversations where incumbents cannot currently promise supply continuity or timeline confidence.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the issue is structural rather than temporary, consider design migration, alternative technology qualification, and contractual reset paths rather than only short-term firefighting around {}.",
                entity
            ));
        }
        (false, "competitor_market") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is competitive displacement, identify which accounts near {} are most exposed to missed qualifications, slower ramps, or weaker service and build a targeted rescue narrative around that pain.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is account defense, review where {} overlaps with your highest-value customers and prepare a sharper proof set on responsiveness, certification depth, and execution reliability.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is pricing leverage, decide whether {} is signaling margin pressure, aggressive expansion, or a tactical quote reset, then match the response to total-cost value rather than list price alone.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is regional strategy, look for places where {} is thin on local footprint, sector credibility, or customer intimacy and press that angle directly.",
                entity
            ));
        }
        (false, "regulatory_policy") | (false, "geopolitical_analysis") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is regulatory posture, turn this into a concrete checklist for export control, certification scope, contract language, and customer communication affecting {}{}.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is routing and footprint, model whether Morocco, Tunisia, or EU-qualified alternatives now solve a policy or trade problem more cleanly than the current setup around {}.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is executive planning, brief leadership on which programs around {} could need re-sourcing, repricing, or customer reassurance first rather than treating this as a generic macro event.",
                entity
            ));
        }
        (false, "strategic_poi") | (false, "talent_ip") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is relationship leverage, update the power map around {} and decide which individual or team now controls project timing, supplier qualification, or partnership appetite.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, "If the goal is early project entry, use the current personnel or project signal to anchor a discussion around capability alignment, design support, or fast-start execution rather than a generic intro.".to_string());
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is competitive positioning, look for dissatisfaction, transition risk, or new mandate areas where the POI signal around {} creates permission for a differentiated pitch.",
                entity
            ));
        }
        (false, "security_compliance")
        | (false, "cybersecurity_threat")
        | (false, "quality_compliance") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is assurance, gather the evidence that would reassure auditors, customers, and procurement teams that {} still clears the relevant security or quality gate.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is supplier governance, update scorecards, audit timing, and exception handling for programs exposed to {}{} rather than waiting for a formal customer escalation.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is commercial positioning, use the compliance gap around {} to show why stronger process control, cert depth, or cyber hygiene changes supplier choice today.",
                entity
            ));
        }
        (false, _) => {
            push_unique_suggestion(&mut suggestions, format!(
                "Map this {} signal around {} to an active decision: is the priority pipeline capture, relationship defense, competitive displacement, or early qualification? Choose one and build the outreach around it.",
                category, entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Translate the current {} evidence into concrete sourcing or qualification choices for {} rather than treating it as a general market signal.",
                category, entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Brief stakeholders on which accounts or programs tied to {} the {} signal affects most and what specific decision each team needs to make next.",
                entity, category
            ));
        }
    }

    if flags.pricing {
        push_unique_suggestion(&mut suggestions, format!(
            "If pricing or cost pressure is part of the pattern, compare margin, lead-time, and resilience trade-offs explicitly before {} resets expectations in the market.",
            entity
        ));
    }
    if flags.innovation || flags.expansion {
        push_unique_suggestion(&mut suggestions, format!(
            "If technology or capacity build is underway, treat this as a timing problem: the best opening is usually before the new capability at {} becomes fully standardized and crowded.",
            entity
        ));
    }
    if flags.shortage || flags.regulatory || flags.geopolitical {
        push_unique_suggestion(&mut suggestions, format!(
            "If regional exposure is rising, pressure-test alternative sites, suppliers, and customer commitments now rather than assuming the current footprint around {} remains stable.",
            entity
        ));
    }
    if flags.personnel {
        push_unique_suggestion(&mut suggestions, format!(
            "If the people signal is real, refresh stakeholder maps and project ownership before deciding who to brief, who to sell to, and who may block progress around {}.",
            entity
        ));
    }

    if let Some(action_hint) = action_hint {
        for fragment in action_hint
            .split(';')
            .map(str::trim)
            .filter(|fragment| !fragment.is_empty())
            .take(3)
        {
            if is_generic_action_hint(fragment) {
                continue;
            }
            push_unique_suggestion(
                &mut suggestions,
                format!(
                    "One concrete lane worth testing is this: {}.",
                    fragment.trim_end_matches('.')
                ),
            );
        }
    }

    for suggestion in &mut suggestions {
        *suggestion = normalize_goal_oriented_suggestion(suggestion);
    }

    suggestions.truncate(5);

    suggestions
}

/// Helper to add a suggestion only if not already present.
fn push_unique_suggestion(suggestions: &mut Vec<String>, suggestion: String) {
    if !suggestions.iter().any(|s| s == &suggestion) {
        suggestions.push(suggestion);
    }
}

fn capitalize_first_fragment(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn normalize_goal_oriented_suggestion(suggestion: &str) -> String {
    let trimmed = suggestion.trim();

    for prefix in ["If the goal is ", "If the priority is "] {
        let Some(remainder) = trimmed.strip_prefix(prefix) else {
            continue;
        };
        let Some((objective, action)) = remainder.split_once(',') else {
            continue;
        };

        let objective = objective.trim().trim_end_matches('.');
        let action = action.trim().trim_end_matches('.');
        if objective.is_empty() || action.is_empty() {
            continue;
        }

        return format!(
            "{} to support {}.",
            capitalize_first_fragment(action),
            objective
        );
    }

    trimmed.to_string()
}

/// Flags detected in signal details for strategy suggestions.
#[derive(Default)]
pub(super) struct StrategySignalFlags {
    pub(super) procurement: bool,
    pub(super) qualification: bool,
    pub(super) shortage: bool,
    pub(super) regulatory: bool,
    pub(super) innovation: bool,
    pub(super) expansion: bool,
    pub(super) pricing: bool,
    pub(super) competitor: bool,
    pub(super) personnel: bool,
    pub(super) security: bool,
    pub(super) geopolitical: bool,
}

/// Detect strategy-relevant flags from signal details.
pub(super) fn detect_strategy_signal_flags(
    category: &str,
    signal_details: &[String],
    action_hint: &str,
) -> StrategySignalFlags {
    let corpus =
        format!("{} {} {}", category, signal_details.join(" "), action_hint).to_ascii_lowercase();

    StrategySignalFlags {
        procurement: corpus.contains("procurement")
            || corpus.contains("supplier portal")
            || corpus.contains("tender")
            || corpus.contains("rfq")
            || corpus.contains("sourcing")
            || corpus.contains("sqe"),
        qualification: corpus.contains("qualification")
            || corpus.contains("cert")
            || corpus.contains("audit")
            || corpus.contains("ppap")
            || corpus.contains("imds")
            || corpus.contains("iatf")
            || corpus.contains("as9100")
            || corpus.contains("iso "),
        shortage: corpus.contains("shortage")
            || corpus.contains("allocation")
            || corpus.contains("lead time")
            || corpus.contains("delay")
            || corpus.contains("disruption")
            || corpus.contains("supply chain")
            || corpus.contains("force majeure"),
        regulatory: corpus.contains("regulation")
            || corpus.contains("policy")
            || corpus.contains("tariff")
            || corpus.contains("sanction")
            || corpus.contains("export control")
            || corpus.contains("embargo")
            || corpus.contains("compliance"),
        innovation: corpus.contains("patent")
            || corpus.contains("innovation")
            || corpus.contains("r&d")
            || corpus.contains("npi")
            || corpus.contains("prototype")
            || corpus.contains("engineering")
            || corpus.contains("technology"),
        expansion: corpus.contains("expansion")
            || corpus.contains("capacity")
            || corpus.contains("facility")
            || corpus.contains("new plant")
            || corpus.contains("new site")
            || corpus.contains("trade show"),
        pricing: corpus.contains("pricing")
            || corpus.contains("price")
            || corpus.contains("margin")
            || corpus.contains("landed cost")
            || corpus.contains("cost"),
        competitor: corpus.contains("competitor")
            || corpus.contains("market")
            || corpus.contains("displaced")
            || corpus.contains("replacement opportunity")
            || corpus.contains("customer overlap"),
        personnel: corpus.contains("poi")
            || corpus.contains("leadership")
            || corpus.contains("executive")
            || corpus.contains("project")
            || corpus.contains("hiring")
            || corpus.contains("job posting")
            || corpus.contains("appointment"),
        security: corpus.contains("security")
            || corpus.contains("cyber")
            || corpus.contains("breach")
            || corpus.contains("incident")
            || corpus.contains("cmmc")
            || corpus.contains("iso 27001"),
        geopolitical: corpus.contains("geopolitical")
            || corpus.contains("trade lane")
            || corpus.contains("nearshor")
            || corpus.contains("morocco")
            || corpus.contains("tunisia")
            || corpus.contains("country")
            || corpus.contains("routing"),
    }
}

/// Build template-based fallback summary when LLM is unavailable.
pub(super) fn build_fallback_summary(
    analytical_narrative: &str,
    rendered_action: &str,
    signal_details: &[String],
    evidence_urls: &[String],
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    _severity: &str,
    _confidence: f64,
    _evidence_count: usize,
) -> String {
    let mut summary_parts: Vec<String> = Vec::new();
    let concrete_details = category_relevant_signal_details(category, signal_details);
    let concrete_signals = concrete_details.len();
    let lower_narrative = analytical_narrative.to_ascii_lowercase();
    let narrative_already_carries_evidence = lower_narrative.contains("observed signals include:")
        || lower_narrative.contains("most specific current evidence:");

    summary_parts.push(analytical_narrative.to_string());

    if !concrete_details.is_empty() && !narrative_already_carries_evidence {
        summary_parts.push(format!(
            "Key evidence: {}.",
            concrete_details
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    if concrete_signals > 0 {
        let suggestion_sentences = build_goal_oriented_suggestions(
            entity_label,
            entity_region,
            entity_type,
            category,
            &concrete_details,
            Some(rendered_action).filter(|s| !s.trim().is_empty()),
        );
        if !suggestion_sentences.is_empty() {
            summary_parts.push(
                suggestion_sentences
                    .into_iter()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }

    let direct_actions = concrete_action_fragments(rendered_action);
    let suppress_direct_actions = is_public_sector_entity(entity_label, entity_type)
        && matches!(
            category,
            "regulatory_policy" | "geopolitical_analysis" | "brand_sentiment"
        );
    if !direct_actions.is_empty() && !suppress_direct_actions {
        summary_parts.push(format!(
            "Immediate next step: {}.",
            direct_actions.join(". ")
        ));
    }

    let sources_footer = format_sources_footer_from_urls(evidence_urls, 6);
    if !sources_footer.is_empty() {
        summary_parts.push(sources_footer);
    }

    summary_parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_metric_detected() {
        assert!(is_aggregate_metric_signal_detail(
            "5 job posting(s) observed"
        ));
        assert!(is_aggregate_metric_signal_detail("3 patent(s) filed"));
        assert!(!is_aggregate_metric_signal_detail(
            "AS9100 certification expiring"
        ));
    }

    #[test]
    fn concrete_details_filters_aggregates() {
        let details = vec![
            "5 job posting(s) observed".to_string(),
            "AS9100 certification update".to_string(),
            "3 news article(s) referenced".to_string(),
        ];
        let concrete = concrete_signal_details(&details);
        assert_eq!(concrete.len(), 1);
        assert!(concrete[0].contains("AS9100"));
    }

    #[test]
    fn generic_action_hints_detected() {
        assert!(is_generic_action_hint("continue to monitor"));
        assert!(is_generic_action_hint("stay informed"));
        assert!(!is_generic_action_hint(
            "Contact procurement lead at Company X about Q2 RFQ"
        ));
    }

    #[test]
    fn should_emit_requires_concrete_signals() {
        let aggregate_only = vec!["5 job posting(s) observed".to_string()];
        assert!(!should_emit_fallback_insight(
            "demand_procurement",
            &aggregate_only,
            "",
            0.5,
            1,
        ));

        let concrete = vec!["AS9100 certification expiring".to_string()];
        assert!(should_emit_fallback_insight(
            "demand_procurement",
            &concrete,
            "Contact vendor",
            0.7,
            2,
        ));
    }

    #[test]
    fn extract_fallback_details_parses_correctly() {
        let summary = "Our monitoring detected: certification update; facility expansion; leadership change. Source: example.com";
        let details = extract_fallback_signal_details(summary);
        assert_eq!(details.len(), 3);
        assert!(details[0].contains("certification"));
    }

    #[test]
    fn nonsecurity_categories_ignore_security_hygiene_as_concrete_signal() {
        let details = vec![
            "8 lookalike domain(s) detected".to_string(),
            "DNS posture degradation detected".to_string(),
            "Named RFQ issued for avionics subassembly".to_string(),
        ];

        let filtered = category_relevant_signal_details("demand_procurement", &details);
        assert_eq!(
            filtered,
            vec!["Named RFQ issued for avionics subassembly".to_string()]
        );
    }

    #[test]
    fn nonsecurity_fallback_requires_category_relevant_concrete_signal() {
        let security_only = vec![
            "8 lookalike domain(s) detected".to_string(),
            "DNS posture degradation detected".to_string(),
        ];

        assert!(!should_emit_fallback_insight(
            "demand_procurement",
            &security_only,
            "",
            0.88,
            4,
        ));
        assert!(should_emit_fallback_insight(
            "cybersecurity_threat",
            &security_only,
            "",
            0.88,
            4,
        ));
    }

    #[test]
    fn extract_fallback_details_parses_new_observed_signals_prefix() {
        let summary = "Observed signals include: certification update; facility expansion; leadership change. Sources pending.";
        let details = extract_fallback_signal_details(summary);
        assert_eq!(details.len(), 3);
        assert!(details[1].contains("facility expansion"));
    }
}
