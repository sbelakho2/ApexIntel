//! Prompt construction and rendering helpers for LLM insight generation.
//!
//! These helpers build and clean the entity profile, evidence text, and
//! narrative templates that feed `generation::generate_llm_insight`. They were
//! extracted from `main.rs` (audit P0 #28) so the binary root only wires
//! configuration, dependencies, scheduler, and service lifecycle.

use std::collections::HashMap;

use crate::fallback_generation::{category_relevant_signal_details, detect_strategy_signal_flags};
use crate::quality_gates::weighted_phrase_score;

// ────────────────────────────────────────────────────────────────────────────
// Recipe-fire helpers
// ────────────────────────────────────────────────────────────────────────────

/// Resolve `{{evidence:KEY}}` placeholders in a template string.
/// Replaces each `{{evidence:key}}` with the corresponding value from `slots`,
/// or with a contextual fallback (e.g. "(entity)" for company_name, "N/A" for others).
#[allow(dead_code)]
pub(crate) fn resolve_evidence_placeholders(
    template: &str,
    slots: &HashMap<String, String>,
) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{evidence:") {
        let prefix = &rest[..start];
        let after = &rest[start + 11..];
        if let Some(end) = after.find("}}") {
            let key = &after[..end];
            if let Some(val) = slots.get(key) {
                result.push_str(prefix);
                result.push_str(val);
            } else {
                let trimmed = prefix.trim_end();
                let lower = trimmed.to_ascii_lowercase();
                let strip_prep = lower.ends_with(" for")
                    || lower.ends_with(" from")
                    || lower.ends_with(" at")
                    || lower.ends_with(" in")
                    || lower.ends_with(" via")
                    || lower.ends_with(" of")
                    || lower.ends_with(" to")
                    || lower.ends_with(" by")
                    || lower.ends_with(" with")
                    || lower.ends_with(" toward")
                    || lower.ends_with(" towards")
                    || lower.ends_with(" against")
                    || lower.ends_with(" between")
                    || lower.ends_with(" up")
                    || lower.ends_with(" down")
                    || lower.ends_with(" about")
                    || lower.ends_with(" over")
                    || trimmed.ends_with(':');
                if strip_prep {
                    let cut = if trimmed.ends_with(':') {
                        trimmed.rfind(':').unwrap_or(trimmed.len())
                    } else {
                        trimmed.rfind(' ').unwrap_or(0)
                    };
                    result.push_str(&trimmed[..cut]);
                } else {
                    result.push_str(prefix);
                }
            }
            rest = &after[end + 2..];
            if slots.get(key).is_none() && rest.starts_with('%') {
                rest = &rest[1..];
            }
        } else {
            result.push_str(prefix);
            result.push_str(&rest[start..]);
            rest = "";
        }
    }
    result.push_str(rest);
    result
}

pub(crate) fn clean_rendered_text(s: &str) -> String {
    let mut text = s.to_string();
    for pattern in &["()", "( )", "[]", "[ ]"] {
        text = text.replace(pattern, "");
    }
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    for _ in 0..3 {
        text = text
            .replace(", .", ".")
            .replace(",,", ",")
            .replace(", ,", ",");
        text = text.replace(". .", ".").replace("..", ".");
        text = text.replace(" .", ".").replace(" ,", ",");
        text = text
            .replace(":.", ".")
            .replace(": .", ".")
            .replace(":,", ",");
        text = text.replace("( ", "(").replace(" )", ")");
        text = text.replace(". . ", ". ");
    }
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    while text.starts_with(". ") {
        text = text[2..].to_string();
    }
    if text == "." {
        text.clear();
    }
    text.trim().to_string()
}

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub(crate) fn is_low_quality_narrative(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 20 {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();

    let dash_count = trimmed.matches('—').count();
    let word_count = trimmed.split_whitespace().count();
    if word_count > 0 && dash_count as f64 / word_count as f64 > 0.25 {
        return true;
    }

    let short_tokens = trimmed
        .split(". ")
        .filter(|segment| segment.split_whitespace().count() <= 2)
        .count();
    let total_sentences = trimmed.split(". ").count();
    if total_sentences >= 3 && short_tokens as f64 / total_sentences as f64 > 0.5 {
        return true;
    }

    let double_prep_patterns = [
        "from under",
        "from via",
        "from through",
        "from by",
        "via via",
        "via by",
        "via from",
        "via through",
        "by by",
        "by from",
        "by via",
        "in in",
        "to to",
        "at at",
        "for for",
        "of of",
        "with with",
        "between between",
        "supply from under",
        "showing conflict",
        "showing including",
    ];
    if double_prep_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return true;
    }

    let vague_alarm_patterns = [
        ("resource weaponization", "which resource"),
        ("conflict escalation", "conflict indicators"),
        ("sanctions cascade", "sanction"),
        ("technology theft", "technology"),
    ];
    for (alarm, must_have) in &vague_alarm_patterns {
        if lower.contains(alarm) {
            let has_number = trimmed.chars().any(|c| c.is_ascii_digit());
            let has_specific = lower.contains(must_have) || has_number;
            if !has_specific {
                return true;
            }
        }
    }

    let broken_grammar = [
        ". supply from",
        ". including.",
        ": .",
        " via .",
        " from .",
        " at .",
        " by .",
        " to .",
        " in .",
        " of .",
        " with .",
    ];
    broken_grammar.iter().any(|pattern| lower.contains(pattern))
}

/// Build an analytical narrative paragraph from structured signal data.
#[allow(dead_code)]
pub(crate) fn build_analytical_narrative(
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    signal_details: &[String],
    _confidence: f64,
    _evidence_ids: &[String],
) -> String {
    let category_desc = match category {
        "competitor_market" => "competitive market activity",
        "demand_procurement" => "demand and procurement developments",
        "supply_chain_risk" => "supply chain risk indicators",
        "security_compliance" => "security and compliance concerns",
        "regulatory_policy" => "regulatory and policy changes",
        "strategic_poi" => "strategic developments involving key personnel",
        "pricing_market" => "pricing and market dynamics",
        "customer_rfq" => "a potential customer procurement opportunity",
        "geopolitical_analysis" => "geopolitical developments affecting operations",
        "veracity_analysis" => "cross-referenced intelligence reporting",
        "talent_ip" => "talent movement and intellectual property activity",
        "technology_innovation" => "technology and R&D developments",
        "ma_partnerships" => "merger, acquisition, or partnership activity",
        "market_expansion" => "market expansion and capacity investments",
        "cybersecurity_threat" => "cybersecurity threats and vulnerabilities",
        "quality_compliance" => "quality certification and compliance changes",
        "brand_sentiment" => "brand sentiment and reputation signals",
        _ => "notable developments",
    };

    let entity_ctx = if !entity_label.is_empty() {
        if !entity_region.is_empty() {
            format!("{} ({})", entity_label, entity_region)
        } else {
            entity_label.to_string()
        }
    } else {
        "An entity under monitoring".to_string()
    };

    let mut parts = Vec::new();
    let relevant_details = category_relevant_signal_details(category, signal_details);
    let flags = detect_strategy_signal_flags(category, &relevant_details, "");
    let is_public_sector = is_public_sector_entity(entity_label, entity_type);
    let concrete_details = relevant_details;

    parts.push(format!("{entity_ctx} shows {category_desc}."));

    if !concrete_details.is_empty() {
        parts.push(format!(
            "Observed signals include: {}.",
            concrete_details.join("; ")
        ));
    }
    if !concrete_details.is_empty() {
        parts.push(format!(
            "Most specific current evidence: {}.",
            concrete_details
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    let mut implications: Vec<String> = Vec::new();
    if flags.procurement {
        implications.push(
            "The signal mix points to an active supplier-evaluation window rather than routine background noise, which means early positioning can shape shortlist, prototype, or qualification scope before it becomes a price-only contest.".to_string()
        );
    }
    if flags.qualification {
        implications.push(format!(
            "Qualification and certification clues tighten the supplier envelope around {entity_ctx}; in regulated sectors, audit readiness and process proof usually matter before commercial terms are finalized."
        ));
    }
    if flags.shortage {
        implications.push(
            "Supply disruption indicators shift the conversation from optimization to continuity: if the constraint persists, customers will look for dual-source coverage, buffer stock, migration paths, or faster escalation before schedules move.".to_string()
        );
    }
    if flags.regulatory || flags.geopolitical {
        implications.push(
            "Policy and trade exposure can quickly make footprint, routing, and export eligibility more decisive than nominal unit cost, especially where EU and North Africa positioning changes the compliance or resilience story.".to_string()
        );
    }
    if flags.competitor {
        implications.push(
            "Competitive movement here creates both a defense problem and a displacement opportunity; the key question is which accounts will feel pain first and what proof would make them switch.".to_string()
        );
    }
    if flags.innovation || flags.expansion {
        implications.push(format!(
            "Capacity, innovation, or program-build signals around {entity_ctx} usually surface months before operations stabilize, so the real leverage is in early influence, allocation access, and partnership framing rather than late reactive outreach."
        ));
    }
    if flags.personnel {
        implications.push(
            "POI and hiring clues add a power-mapping dimension: they often reveal who is building budget, launching a program, or quietly changing supplier criteria before the organization says so publicly.".to_string()
        );
    }
    if flags.pricing {
        implications.push(
            "Pricing signals imply a quoting or margin reset is underway, which can be used either to protect current business or to present a stronger total-cost and resilience case to buyers under pressure.".to_string()
        );
    }
    if flags.security && is_public_sector {
        implications.push(
            "For a government entity, the security question is whether the signal touches official domains, procurement portals, citizen-facing services, or supplier access paths; that determines whether this is a real operational issue or just background cyber noise.".to_string()
        );
    }
    if flags.security && !is_public_sector {
        implications.push(
            "Security and assurance issues rarely stay isolated; they spill into vendor eligibility, customer audits, and executive risk discussions much faster than routine operational changes.".to_string()
        );
    }
    if implications.is_empty() && is_public_sector && category == "brand_sentiment" {
        implications.push(
            "For a public-sector body, sentiment is only actionable if it starts to change institutional credibility, oversight pressure, procurement scrutiny, or decision timing; otherwise it should be treated as background noise rather than a direct commercial trigger.".to_string()
        );
    }
    if implications.is_empty() {
        implications.push(
            "Taken together, these signals matter because they change commercial timing, supplier choice, or executive priorities rather than representing isolated informational noise.".to_string()
        );
    }
    parts.push(
        implications
            .into_iter()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
    );

    parts.join(" ")
}

/// Rich evidence signal with category and structured data.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(crate) struct EvidenceSignal {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) source_url: String,
    pub(crate) signal_type: String, // e.g., "certification", "capability", "news", "poi", "warning"
    pub(crate) extracted_facts: Vec<String>, // Key facts extracted from the evidence
    pub(crate) date_context: Option<String>, // When this happened/detected
    pub(crate) relevance_score: f32, // How relevant to the insight category
}

/// Entity context with rich structured data about the company/org.
#[cfg(feature = "llm")]
#[derive(Clone)]
pub(crate) struct EntityContext {
    pub(crate) name: String,
    pub(crate) region: String,
    pub(crate) entity_type: Option<String>,
    /// True = this entity is a direct EMS competitor; False = customer/partner/prospect
    pub(crate) is_competitor: bool,
    pub(crate) industry_tags: Vec<String>,
    pub(crate) certifications: Vec<String>, // e.g., "AS9100D (valid until 2027-03)", "ISO 13485"
    pub(crate) capabilities: Vec<String>, // e.g., "High-Volume SMT", "Medical Device Manufacturing"
    pub(crate) key_persons: Vec<String>,  // e.g., "CEO: Jensen Huang (C-Suite, influential)"
    pub(crate) recent_changes: Vec<String>, // e.g., "New facility detected", "Leadership change"
    // Rich competitive context
    pub(crate) threat_score: Option<f64>,
    pub(crate) overlap_score: Option<f64>,
    pub(crate) strategic_relevance: Option<f64>,
    pub(crate) revenue_estimate_usd: Option<i64>,
    pub(crate) employee_estimate: Option<i32>,
    pub(crate) competitor_names: Vec<String>, // Linked competitors via graph edges
    pub(crate) sites_summary: Vec<String>,    // e.g., "Manufacturing plant in Tunis, Tunisia"
    pub(crate) competitor_events: Vec<String>, // Recent competitor moves
    pub(crate) domain: Option<String>,
}

/// Returns the supply chain role for non-EMS, non-government entities.
/// These entities need differentiated LLM prompting — not the default
/// "customer/prospect" framing that leads to nonsensical partnership proposals.
#[cfg(feature = "llm")]
pub(crate) fn supply_chain_role(entity_type: Option<&str>) -> Option<&'static str> {
    match entity_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "distributor" => Some("DISTRIBUTOR"),
        "oem" => Some("OEM"),
        "defense" | "defence" => Some("DEFENSE_PRIME"),
        "semiconductor" => Some("SEMICONDUCTOR"),
        "pcb" => Some("PCB_MANUFACTURER"),
        "t&m" | "test_measurement" => Some("TEST_MEASUREMENT"),
        "trade_association" => Some("TRADE_ASSOCIATION"),
        _ => None,
    }
}

#[cfg(feature = "llm")]
pub(crate) fn inferred_supply_chain_role(entity_ctx: &EntityContext) -> Option<&'static str> {
    let corpus = format!(
        "{} {} {} {} {}",
        entity_ctx.name,
        entity_ctx.entity_type.as_deref().unwrap_or_default(),
        entity_ctx.industry_tags.join(" "),
        entity_ctx.capabilities.join(" "),
        entity_ctx.sites_summary.join(" "),
    )
    .to_ascii_lowercase();

    let has_any = |markers: &[&str]| markers.iter().any(|marker| corpus.contains(marker));

    if has_any(&[
        "trade association",
        "industry association",
        "industry body",
        "standards body",
        "chamber of commerce",
        "industry council",
    ]) {
        return Some("TRADE_ASSOCIATION");
    }

    if has_any(&[
        "semiconductor",
        "microcontroller",
        "mcu",
        "analog chip",
        "power management ic",
        "sensor ic",
        "chipmaker",
        "fabless",
        "wafer",
    ]) {
        return Some("SEMICONDUCTOR");
    }

    if has_any(&[
        "defense prime",
        "defence prime",
        "munitions",
        "missile",
        "radar",
        "electronic warfare",
        "defense contractor",
        "defence contractor",
        "aerospace and defense",
    ]) {
        return Some("DEFENSE_PRIME");
    }

    if has_any(&[
        "distributor",
        "electronics distribution",
        "component distributor",
        "authorized distributor",
        "broadline distribution",
    ]) {
        return Some("DISTRIBUTOR");
    }

    if has_any(&[
        "printed circuit board",
        "pcb manufacturer",
        "pcb fabrication",
        "bare board",
        "bare pcb",
    ]) {
        return Some("PCB_MANUFACTURER");
    }

    if has_any(&[
        "test and measurement",
        "test & measurement",
        "oscilloscope",
        "metrology",
        "signal analyzer",
    ]) {
        return Some("TEST_MEASUREMENT");
    }

    if has_any(&[
        " oem",
        "original equipment manufacturer",
        "medical device manufacturer",
        "automotive oem",
        "industrial oem",
    ]) {
        return Some("OEM");
    }

    supply_chain_role(entity_ctx.entity_type.as_deref())
}

#[cfg(feature = "llm")]
pub(crate) fn contains_semiconductor_sales_pitch(corpus: &str) -> bool {
    weighted_phrase_score(
        corpus,
        &[
            ("our company", 0.20),
            ("our services", 0.25),
            ("our facility", 0.20),
            ("our facilities", 0.20),
            ("our capability", 0.20),
            ("our capabilities", 0.20),
            ("our as9100", 0.25),
            ("our iatf", 0.25),
            ("our iso 14001", 0.30),
            ("our iso 9001", 0.25),
            ("our iso 13485", 0.25),
            ("our north africa", 0.20),
            ("our tunisia", 0.20),
            ("our morocco", 0.20),
            ("our european manufacturing", 0.25),
            ("manufacturing capabilities", 0.25),
            ("green manufacturing", 0.25),
            ("reliable partner", 0.20),
            ("detailed proposal", 0.25),
            ("qualification support", 0.25),
            ("compliance support", 0.25),
            ("supply chain optimization", 0.25),
            ("opportunity for our company", 0.30),
            ("immediate qualification support", 0.25),
            ("aerospace focused ems services", 0.30),
            ("ems services", 0.25),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
pub(crate) fn violates_supply_chain_role_guidance(
    supply_chain_role: Option<&str>,
    headline: &str,
    narrative: &str,
    recommendation: &str,
) -> bool {
    let Some(role) = supply_chain_role else {
        return false;
    };

    let corpus = format!("{}\n{}\n{}", headline, narrative, recommendation).to_ascii_lowercase();

    match role {
        "SEMICONDUCTOR" => {
            [
                "outsourcing opportunit",
                "ems opportunit",
                "nearshore ems",
                "sell assembly",
                "assembly services",
                "manufacturing services",
                "ems partner",
                "partner with microchip",
                "partner with nxp",
            ]
            .iter()
            .any(|phrase| corpus.contains(phrase))
                || contains_semiconductor_sales_pitch(&corpus)
        }
        "DEFENSE_PRIME" => [
            "outsourcing opportunit",
            "ems opportunit",
            "nearshore ems",
            "direct outreach to bae",
            "direct outreach to l3harris",
            "sell assembly services",
            "offer our services to bae",
            "offer our services to l3harris",
        ]
        .iter()
        .any(|phrase| corpus.contains(phrase)),
        _ => false,
    }
}

#[cfg(feature = "llm")]
pub(crate) fn topic_marker_hits(corpus: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .filter(|marker| corpus.contains(**marker))
        .count()
}

#[cfg(feature = "llm")]
pub(crate) fn violates_entity_topic_alignment(
    entity_ctx: &EntityContext,
    evidence_signals: &[EvidenceSignal],
    headline: &str,
    narrative: &str,
    recommendation: &str,
) -> bool {
    let support_corpus = format!(
        "{} {} {} {} {} {} {}",
        entity_ctx.name,
        entity_ctx.entity_type.as_deref().unwrap_or_default(),
        entity_ctx.industry_tags.join(" "),
        entity_ctx.capabilities.join(" "),
        entity_ctx.sites_summary.join(" "),
        entity_ctx.recent_changes.join(" "),
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
    )
    .to_ascii_lowercase();
    let output_corpus =
        format!("{} {} {}", headline, narrative, recommendation).to_ascii_lowercase();

    let unsupported_bundles: [&[&str]; 4] = [
        &[
            "middle east oil",
            "oil surge",
            "oil price",
            "brent",
            "crude",
            "opec",
            "petrochemical",
            "refinery",
            "lng",
            "gas field",
        ],
        &[
            "mining",
            "ore",
            "smelter",
            "lithium",
            "nickel",
            "copper concentrate",
            "rare earth",
        ],
        &[
            "agriculture",
            "crop",
            "harvest",
            "grain",
            "fertilizer",
            "farm",
            "food processing",
        ],
        &[
            "retail chain",
            "store rollout",
            "consumer packaged",
            "apparel",
            "fashion",
        ],
    ];

    unsupported_bundles.iter().any(|markers| {
        let output_hits = topic_marker_hits(&output_corpus, markers);
        let support_hits = topic_marker_hits(&support_corpus, markers);
        output_hits >= 2 && support_hits == 0
    })
}

pub(crate) fn is_public_sector_entity(entity_label: &str, entity_type: Option<&str>) -> bool {
    let haystack = format!(
        "{} {}",
        entity_type.unwrap_or_default().to_ascii_lowercase(),
        entity_label.to_ascii_lowercase()
    );

    [
        "government",
        "public sector",
        "public-sector",
        "ministry",
        "commission",
        "parliament",
        "council",
        "agency",
        "authority",
        "department",
        "municipality",
        "state ",
        "embassy",
        "consulate",
        "regulator",
    ]
    .iter()
    .any(|marker| haystack.contains(marker))
}

#[cfg(feature = "llm")]
pub(crate) fn public_sector_procurement_or_program_case(
    evidence_signals: &[EvidenceSignal],
) -> bool {
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
        .join(" ")
        .to_ascii_lowercase();

    let has_procurement_marker = [
        "procurement",
        "tender",
        "rfq",
        "rfp",
        "solicitation",
        "bid",
        "framework agreement",
        "supplier registration",
        "buyer",
        "sourcing",
        "purchase",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_hardware_or_program_marker = [
        "equipment",
        "hardware",
        "device",
        "devices",
        "electronics",
        "camera",
        "cameras",
        "sensor",
        "sensors",
        "streaming",
        "broadcast",
        "control room",
        "media system",
        "manufacturing",
        "assembly",
        "pcba",
        "pcb",
        "box build",
        "program",
        "platform",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    has_procurement_marker && has_hardware_or_program_marker
}
