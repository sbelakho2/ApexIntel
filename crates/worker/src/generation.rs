//! LLM insight generation pipeline.

use anyhow::{Context, Result};
use apex_llm::inference::LlmClient as InferenceLlmClient;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

use crate::digest_filtering::{
    has_excessive_phrase_repetition, is_readable_and_useful_digest_text,
};
use crate::fallback_generation::format_sources_footer_from_urls;
use crate::llm_orchestration::{
    build_llm_retry_guidance, emit_quality_gate_decisions, has_confidence_boilerplate,
    quality_gate_blocker, quality_gate_passes_ensemble, quality_gate_requirement,
};
use crate::prompts::{
    inferred_supply_chain_role, is_public_sector_entity, public_sector_procurement_or_program_case,
    violates_entity_topic_alignment, violates_supply_chain_role_guidance, EntityContext,
    EvidenceSignal,
};
use crate::quality_gates::{
    has_formulaic_commercial_language, has_low_usefulness_public_sector_analysis,
    has_temporal_incoherence, has_unnamed_customer_targeting,
    has_unsupported_certification_commercialization, has_unsupported_certification_escalation,
    has_unsupported_named_target_provenance, has_unsupported_public_sector_commercialization,
    has_unsupported_security_escalation, recommendation_has_action_timing,
};

// Quality gate types and functions extracted to llm_orchestration module

/// Extract key facts from evidence text using pattern matching.
/// Returns a list of specific facts (names, dates, numbers, locations).
#[cfg(feature = "llm")]
pub(crate) fn extract_facts_from_text(text: &str) -> Vec<String> {
    use regex::Regex;
    lazy_static::lazy_static! {
        // Match monetary values
        static ref MONEY_RE: Regex = Regex::new(r"\$[\d,.]+\s*(?:M|B|million|billion|thousand)?|\d+[\d,]*\s*(?:USD|EUR|GBP)").unwrap();
        // Match percentages
        static ref PERCENT_RE: Regex = Regex::new(r"\d+(?:\.\d+)?%").unwrap();
        // Match dates
        static ref DATE_RE: Regex = Regex::new(r"(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.?\s+\d{1,2},?\s+\d{4}|\d{4})").unwrap();
        // Match certification standards
        static ref CERT_RE: Regex = Regex::new(r"(?:ISO|AS|IATF|IEC|MIL-STD|NADCAP|FDA|CE|UL|RoHS)\s*[\d:-]+(?:\s*[A-Z])?").unwrap();
        // Match employee counts
        static ref EMPLOYEE_RE: Regex = Regex::new(r"\d+[\d,]*\s*(?:employees|workers|staff|headcount)").unwrap();
        // Match acquisition/merger phrases
        static ref ACQUISITION_RE: Regex = Regex::new(r"(?:acquir(?:ed|es|ing)|merger|bought|purchas(?:ed|e))\s+(?:by\s+)?([A-Z][a-zA-Z\s&]+)").unwrap();
        // Match facility/location signals
        static ref FACILITY_RE: Regex = Regex::new(r"(?:new\s+)?(?:facility|plant|factory|site|headquarters)\s+(?:in\s+)?([A-Z][a-zA-Z\s,]+)").unwrap();
    }

    let mut facts = Vec::new();

    // Extract monetary values
    for m in MONEY_RE.find_iter(text).take(3) {
        facts.push(format!("Value: {}", m.as_str()));
    }

    // Extract percentages
    for m in PERCENT_RE.find_iter(text).take(2) {
        facts.push(format!("Change: {}", m.as_str()));
    }

    // Extract certifications
    for m in CERT_RE.find_iter(text).take(3) {
        facts.push(format!("Certification: {}", m.as_str()));
    }

    // Extract dates
    for m in DATE_RE.find_iter(text).take(2) {
        let date_str = m.as_str();
        // Skip years that are part of cert standards
        if date_str.len() > 4 || !text.contains(&format!(":{}", date_str)) {
            facts.push(format!("Date: {}", date_str));
        }
    }

    // Extract employee counts
    for m in EMPLOYEE_RE.find_iter(text).take(1) {
        facts.push(m.as_str().to_string());
    }

    // Extract acquisition mentions
    for caps in ACQUISITION_RE.captures_iter(text).take(1) {
        if let Some(m) = caps.get(1) {
            facts.push(format!("Acquisition involving {}", m.as_str().trim()));
        }
    }

    // Extract facility mentions
    for caps in FACILITY_RE.captures_iter(text).take(1) {
        if let Some(m) = caps.get(1) {
            facts.push(format!("Facility in {}", m.as_str().trim()));
        }
    }

    facts
}

#[cfg(feature = "llm")]
pub(crate) fn count_numbered_references(text: &str, max_ref: usize) -> usize {
    (1..=max_ref)
        .filter(|i| text.contains(&format!("[{}]", i)))
        .count()
}

#[cfg(feature = "llm")]
pub(crate) fn ranked_source_urls(
    evidence_signals: &[EvidenceSignal],
    max_sources: usize,
) -> Vec<String> {
    let mut sorted: Vec<_> = evidence_signals.iter().collect();
    sorted.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for signal in sorted {
        let url = signal.source_url.trim();
        if url.is_empty() {
            continue;
        }
        let key = url.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(url.to_string());
            if out.len() >= max_sources {
                break;
            }
        }
    }
    out
}

#[cfg(feature = "llm")]
pub(crate) fn format_sources_footer(
    evidence_signals: &[EvidenceSignal],
    max_sources: usize,
) -> String {
    format_sources_footer_from_urls(
        &ranked_source_urls(evidence_signals, max_sources),
        max_sources,
    )
}

#[cfg(feature = "llm")]
/// Generate insight narrative and headline using LLM with rich context.
/// Returns (headline, narrative, recommendation, confidence).
#[cfg(feature = "llm")]
/// Strip forbidden certification claims from LLM insight output. Called when
/// the model persists in claiming certifications our company does not hold
/// (AS9100, IATF 16949, ISO 13485) despite corrective retry guidance. Rather
/// than discarding the entire insight, we remove the offending phrases so the
/// rest of the (potentially valuable) analysis survives.
pub(crate) fn sanitize_certification_claims(text: &str) -> String {
    let mut out = text.to_string();
    // Replace possessive certification claim patterns with neutral language.
    let replacements = [
        ("our AS9100", "a potential AS9100 gap in"),
        ("our IATF 16949", "a potential IATF 16949 gap in"),
        ("our ISO 13485", "a potential ISO 13485 gap in"),
        ("we hold AS9100", "we do not currently hold AS9100"),
        ("we hold IATF 16949", "we do not currently hold IATF 16949"),
        ("we hold ISO 13485", "we do not currently hold ISO 13485"),
        ("we are AS9100", "we are not yet AS9100"),
        ("we are IATF 16949", "we are not yet IATF 16949"),
        ("we are ISO 13485", "we are not yet ISO 13485"),
        ("we have AS9100", "we lack AS9100"),
        ("we have IATF 16949", "we lack IATF 16949"),
        ("we have ISO 13485", "we lack ISO 13485"),
        ("with our AS9100", "noting our AS9100 gap relative to"),
        (
            "with our IATF 16949",
            "noting our IATF 16949 gap relative to",
        ),
        ("with our ISO 13485", "noting our ISO 13485 gap relative to"),
        ("AS9100 certification", "AS9100 (which we do not hold)"),
        (
            "IATF 16949 certification",
            "IATF 16949 (which we do not hold)",
        ),
        (
            "ISO 13485 certification",
            "ISO 13485 (which we do not hold)",
        ),
        ("AS9100 certified", "AS9100-adjacent (unverified)"),
        ("IATF certified", "IATF 16949-adjacent (unverified)"),
        ("13485 certified", "ISO 13485-adjacent (unverified)"),
    ];
    for (from, to) in &replacements {
        out = out.replace(from, to);
    }
    out
}

pub(crate) async fn generate_llm_insight(
    llm_client: &InferenceLlmClient,
    entity_ctx: &EntityContext,
    category: &str,
    evidence_signals: &[EvidenceSignal],
) -> Result<(String, String, String, f64, serde_json::Value)> {
    use apex_llm::inference::{ChatMessage, InferenceConfig};

    if evidence_signals.is_empty() {
        anyhow::bail!("No evidence signals provided for LLM insight generation");
    }

    let is_public_sector =
        is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref());
    let has_public_sector_procurement_program =
        public_sector_procurement_or_program_case(evidence_signals);

    // ── Build rich entity profile with competitive context ──
    let mut profile_parts: Vec<String> = Vec::new();
    let type_str = entity_ctx.entity_type.as_deref().unwrap_or("company");
    if type_str.to_lowercase().contains("government") {
        profile_parts.push(format!(
            "{} is a government/public sector entity in {}.",
            entity_ctx.name, entity_ctx.region
        ));
    } else {
        let mut desc = format!(
            "{} is a {} based in {}.",
            entity_ctx.name, type_str, entity_ctx.region
        );
        if let Some(rev) = entity_ctx.revenue_estimate_usd {
            desc.push_str(&format!(
                " Estimated revenue: ~${:.0}M.",
                rev as f64 / 1_000_000.0
            ));
        }
        if let Some(emp) = entity_ctx.employee_estimate {
            desc.push_str(&format!(" ~{} employees.", emp));
        }
        profile_parts.push(desc);
    }
    if let Some(domain) = &entity_ctx.domain {
        profile_parts.push(format!("Domain: {}", domain));
    }
    if !entity_ctx.industry_tags.is_empty() {
        profile_parts.push(format!(
            "Industry focus: {}.",
            entity_ctx.industry_tags.join(", ")
        ));
    }
    if !entity_ctx.certifications.is_empty() {
        let cert_str = entity_ctx
            .certifications
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Certifications: {}", cert_str));
    }
    if !entity_ctx.capabilities.is_empty() {
        let cap_str = entity_ctx
            .capabilities
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Capabilities: {}", cap_str));
    }
    if !entity_ctx.key_persons.is_empty() {
        let poi_str = entity_ctx
            .key_persons
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Key personnel: {}", poi_str));
    }
    if !entity_ctx.sites_summary.is_empty() {
        let sites_str = entity_ctx
            .sites_summary
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Sites/Facilities: {}", sites_str));
    }

    // Competitive positioning
    let mut comp_parts: Vec<String> = Vec::new();
    if let Some(ts) = entity_ctx.threat_score {
        comp_parts.push(format!("Threat score: {:.2}", ts));
    }
    if let Some(os) = entity_ctx.overlap_score {
        comp_parts.push(format!("Market overlap: {:.2}", os));
    }
    if let Some(sr) = entity_ctx.strategic_relevance {
        comp_parts.push(format!("Strategic relevance: {:.2}", sr));
    }
    if !entity_ctx.competitor_names.is_empty() {
        comp_parts.push(format!(
            "Linked competitors: {}",
            entity_ctx.competitor_names.join(", ")
        ));
    }
    if !comp_parts.is_empty() {
        profile_parts.push(format!("Competitive profile: {}", comp_parts.join(". ")));
    }
    if !entity_ctx.recent_changes.is_empty() {
        let changes_str = entity_ctx
            .recent_changes
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Recent changes: {}", changes_str));
    }
    if !entity_ctx.competitor_events.is_empty() {
        let events_str = entity_ctx
            .competitor_events
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Competitor intelligence: {}", events_str));
    }
    // Competitor classification — tells the LLM how to treat this entity
    let sc_role = inferred_supply_chain_role(entity_ctx);
    if entity_ctx.is_competitor {
        profile_parts.push(
            "⚠️ ENTITY CLASSIFICATION: DIRECT EMS COMPETITOR — Do NOT recommend offering our services \
to this company. Analyse their weaknesses, identify which of their customers are underserved, \
and recommend approaching those customers instead.".to_string()
        );
    } else if is_public_sector {
        profile_parts.push(
            "🏛️ ENTITY CLASSIFICATION: PUBLIC-SECTOR BODY — treat this as an institutional account, not a default manufacturing prospect. Direct supplier outreach is only justified when the evidence explicitly names a procurement, tender, supplier qualification event, or hardware/equipment program.".to_string()
        );
    } else if let Some(role) = sc_role {
        let classification_text = match role {
            "DISTRIBUTOR" => format!(
                "📦 ENTITY CLASSIFICATION: COMPONENT DISTRIBUTOR — {} sells electronic components, they do NOT manufacture products. \
Do NOT recommend partnering with or selling EMS services to this distributor. \
Instead, analyse how their distribution changes (pricing, availability, new products, logistics) \
affect OEM and EMS procurement. Recommend how to leverage or hedge against these distribution shifts \
for our existing and prospective customers.",
                entity_ctx.name
            ),
            "OEM" => format!(
                "🏭 ENTITY CLASSIFICATION: OEM / END CUSTOMER — {} designs and sells end products. \
They are a potential customer for EMS services. Analyse their outsourcing needs, product roadmap signals, \
supply chain vulnerabilities, and qualification requirements. Recommend specific engagement strategies \
to win or expand manufacturing contracts with them.",
                entity_ctx.name
            ),
            "DEFENSE_PRIME" => format!(
                "🛡️ ENTITY CLASSIFICATION: DEFENSE PRIME CONTRACTOR — {} is a large defense/aerospace prime. \
They subcontract EMS work. Analyse their program timelines, subcontractor needs, compliance requirements, \
and supply chain gaps. Recommend qualification paths and specific programs where our capabilities \
(nearshore, AS9100, ITAR-free) create an advantage.",
                entity_ctx.name
            ),
            "SEMICONDUCTOR" => format!(
                "🔬 ENTITY CLASSIFICATION: SEMICONDUCTOR COMPANY — {} designs or manufactures chips/ICs. \
They are NOT an EMS prospect. Do NOT recommend selling assembly services to them. \
    Do NOT frame them as a consulting, compliance-support, proposal, or facility-pitch target either. \
    Instead, analyse how their product launches, shortages, EOL notices, or pricing changes \
    affect our customers' BOMs and procurement. Recommend supply chain actions for our customer base.",
                entity_ctx.name
            ),
            "PCB_MANUFACTURER" => format!(
                "🟢 ENTITY CLASSIFICATION: PCB MANUFACTURER — {} makes bare printed circuit boards. \
They are an upstream supplier, not an EMS prospect. Analyse their capacity, lead times, quality, \
and regional footprint to assess impact on our supply chain and our customers' PCB sourcing. \
Recommend sourcing diversification or qualification actions.",
                entity_ctx.name
            ),
            "TEST_MEASUREMENT" => format!(
                "🔧 ENTITY CLASSIFICATION: TEST & MEASUREMENT COMPANY — {} makes test equipment. \
They are a tooling vendor, not an EMS prospect. Analyse how their product updates \
or pricing affect our test capabilities and our competitiveness. \
Recommend equipment investment or qualification actions.",
                entity_ctx.name
            ),
            "TRADE_ASSOCIATION" => format!(
                "🤝 ENTITY CLASSIFICATION: TRADE ASSOCIATION / INDUSTRY BODY — {} is an industry organization. \
They do NOT buy EMS services. Analyse their policy positions, member activities, trade events, \
and regulatory advocacy for their impact on our market. Recommend engagement for visibility \
and business development, not direct sales.",
                entity_ctx.name
            ),
            _ => "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity.".to_string(),
        };
        profile_parts.push(classification_text);
    } else {
        profile_parts.push(
            "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity."
                .to_string(),
        );
    }
    let entity_profile = profile_parts.join("\n");

    // ── Build sorted evidence text ──
    let mut sorted_evidence: Vec<_> = evidence_signals.iter().collect();
    sorted_evidence.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let evidence_text: String = sorted_evidence
        .iter()
        .take(12)
        .enumerate()
        .map(|(i, sig)| {
            let mut parts = vec![format!("[{}] {} ({})", i + 1, sig.title, sig.signal_type)];
            if !sig.extracted_facts.is_empty() {
                parts.push(format!("   Key facts: {}", sig.extracted_facts.join("; ")));
            }
            if !sig.description.is_empty() {
                parts.push(format!(
                    "   Detail: {}",
                    crate::truncate_text(&sig.description, 420)
                ));
            }
            if let Some(date) = &sig.date_context {
                parts.push(format!("   When: {}", date));
            }
            if !sig.source_url.is_empty() {
                parts.push(format!("   Source: {}", sig.source_url));
            }
            parts.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // ── Category-specific guidance tuned for actionable intelligence ──
    let (category_label, analysis_focus, action_focus) = if entity_ctx.is_competitor {
        // For competitors: always focus on exploiting their weaknesses via their customers
        (
            "Competitive Intelligence",
            "Identify specific competitive vulnerabilities in this rival: lost certifications, \
delayed projects, leadership gaps, supply problems, customer complaints, capability gaps. \
Flag which of their customers are likely dissatisfied or underserved and ready to switch. \
For North African and EU competitors, identify market entry/exit signals, M&A activity, \
and regions where their coverage is thin.",
            "Name 2-3 specific companies that are CUSTOMERS OF THIS COMPETITOR and explain why \
they are now vulnerable to switching. Format: '<Customer company name from evidence> — \
<reason they are underserved by competitor right now> — <what we offer them> — by <deadline>'. \
NEVER address the competitor itself as a target.",
        )
    } else if is_public_sector {
        match category {
        "brand_sentiment" => (
            "Public-Sector Sentiment Brief",
            "Assess whether the signal changes institutional credibility, oversight pressure, procurement scrutiny, stakeholder messaging, or decision timing. Distinguish a media cycle from a formal review, public inquiry, procurement caution, or program delay. Do not infer direct electronics demand or supplier fit unless the evidence explicitly names a procurement or hardware program.",
            "Recommend account-planning, dependency mapping, evidence verification, or scenario-planning actions. Only propose direct outreach if the evidence explicitly names a procurement, tender, hardware requirement, or supplier qualification event tied to this entity.",
        ),
        "geopolitical_analysis" | "regulatory_policy" => (
            "Public-Sector Policy Impact Brief",
            "Explain what concrete policy, institutional, or trade artifact changed and which approvals, tenders, supplier pathways, or customer programs it could affect. Do not convert macro policy or innovation signals into a manufacturing sales pitch unless the evidence explicitly names procurement, equipment need, tender language, or supplier qualification requirements.",
            "Recommend concrete next steps such as policy-impact mapping, account dependency review, qualification planning, stakeholder outreach to named institutional roles, or scenario planning. If there is no explicit procurement or hardware program in evidence, do not propose PCBA, box build, or generic EMS outreach.",
        ),
        _ => (
            "Public-Sector Intelligence Brief",
            "Identify what institutional decision, review, procurement path, or stakeholder process is actually moving and what that changes for account planning. Keep direct evidence separate from commercial inference and avoid treating public-sector activity as a sales trigger without explicit buying or program evidence.",
            "Recommend evidence-based account actions, named stakeholder follow-up, or qualification planning. Direct supplier outreach requires explicit procurement or program evidence.",
        ),
    }
    } else {
        match category {
        "competitor_market" => (
            "Competitive Intelligence",
            "Identify specific competitive vulnerabilities: lost certifications, delayed projects, leadership gaps, supply problems, customer complaints. Flag which of their customers are likely dissatisfied or underserved. For North African and EU competitors, identify market entry/exit signals, M&A activity, and capability gaps we can exploit.",
            "Name 2-3 specific companies or contacts to call, with the exact pitch angle (e.g., 'their AS9100 lapsed — offer compliance consulting as a door-opener'). Include competitive displacement actions targeting specific accounts.",
        ),
        "demand_procurement" => (
            "Procurement Opportunity",
            "Identify what this entity is buying, building, or expanding. Map their procurement signals to products/services we can offer. Assess: buying timeline, budget signals (from revenue/hiring data), qualification requirements (from cert signals), and decision-makers (from POI data). Look for RFQ/tender signals in the evidence.",
            "Name the exact person or role to call, what to offer, and by when. If they posted jobs in engineering or procurement, infer what capability they're building and propose a supply partnership. Give a specific dollar-value opportunity estimate if evidence supports it.",
        ),
        "supply_chain_risk" => (
            "Supply Chain Opportunity",
            "Map this entity's supply chain vulnerabilities to our opportunities: their supplier delays become our pitch for alternative sourcing; their shortage signals become our capacity-availability play; their tariff exposure becomes our nearshoring pitch for North African production sites. Quantify disruption impact using any revenue/employee data available.",
            "Specify which of their supply chain gaps we can fill, which facility/site to propose, and which buyer to contact. If this is a competitor's problem, explain how to approach their downstream customers with reliability messaging.",
        ),
        "security_compliance" | "cybersecurity_threat" => (
            "Security Assurance Intelligence",
            "Separate observed security facts from commercial inference. Low-severity hygiene findings such as missing DKIM/SPF/DMARC records, low DNS posture scores, or isolated lookalike domains are vendor-assurance and fraud-risk signals, not proof of customer churn, defense-program exclusion, medical-device ineligibility, or supply disruption. Only escalate to audit failure, program eligibility, contract loss, or downstream operational impact if the evidence explicitly names a breach, outage, regulator action, tender requirement, customer response, or failed certification.",
            "Recommend measured actions tied to the evidence: assurance questions, remediation requests, or targeted outreach only where a named customer, program, or qualification requirement appears in the evidence. Every recommendation must cite the evidence it depends on.",
        ),
        "geopolitical_analysis" => (
            "Geopolitical Opportunity",
            "Translate geopolitical/regulatory shifts into commercial actions: new tariffs → nearshoring opportunity in Morocco/Tunisia; export control changes → qualification opportunity for EU-based alternatives; sanctions → market gap to fill. Focus on North African and EU angles. Map affected trade lanes to specific entities and their likely procurement pivots.",
            "Name the specific companies that will need to re-source and when. Specify which North African or EU production site to position. Include regulatory deadlines and qualification windows.",
        ),
        "regulatory_policy" | "pricing_market" => (
            "Market Intelligence",
            "Extract actionable business intelligence: explicit failed, revoked, or time-bound certification changes can create qualification windows; pricing signals reveal margin pressure at competitors; regulatory shifts create compliance consulting opportunities. Generic website certification notices, stale dates, or unspecified warning markers are not enough on their own to claim qualification failure, customer loss, or a switch opportunity. Cross-reference with known competitor capabilities and recent changes to identify where rivals are weak.",
            "Identify 2-3 specific commercial actions: companies to approach with compliance offers, pricing advantages to highlight, or capability gaps to fill. Name the buyer role and a specific deadline tied to the regulatory change.",
        ),
        _ => (
            "Business Intelligence",
            "Produce case-specific competitive intelligence. Identify who is winning, losing, hiring, cutting, expanding, or retreating. Cross-reference evidence to find exploitable patterns: a company hiring procurement staff likely has upcoming RFQs; a company with explicit certification loss or failed audit may have a compliance gap we can fill. Do not treat generic certification warnings, brochure dates, or accreditation mentions as proof of qualification failure.",
            "Name 2-3 specific actions with company names, contact roles, pitch angles, and deadlines. Every recommendation must answer: 'who do we call, what do we say, and by when?'",
        ),
    }
    };

    // ── System prompt: competitive intelligence operator, not passive analyst ──
    // Load our company profile from env so the model knows what we offer.
    let our_profile = std::env::var("COMPANY_PROFILE").unwrap_or_else(|_| {
        "Starz Electronics is an electronics manufacturer (EMS heritage: PCBA assembly, box build, \
test & inspection, supply chain management) whose primary growth focus is battery energy storage \
systems (BESS): it designs and manufactures residential and commercial/industrial battery packs \
(5, 10 and 15 kWh) with an in-house battery management system (BMS) and custom pack design, \
sourcing lithium cells from global suppliers. Production facilities are in North Africa (Tunisia, \
Morocco free zones) and Europe. Certifications: ISO 9001:2015 and IPC (Institute for Printed \
Circuits) ONLY. We DO NOT hold AS9100, ISO 13485, or IATF 16949 certifications. Battery-pack sales \
focus: PRIMARILY Morocco, Tunisia and Egypt; SECONDARILY (smaller focus) the European Union; we do \
NOT sell battery packs in any other market. Cell suppliers and competing pack/BMS makers are \
tracked globally regardless of their location."
            .to_string()
    });

    let system = format!("You are a competitive intelligence analyst at an OSINT firm. Your job is to read raw signal evidence and report what it actually shows — nothing more, nothing less. You write for C-suite executives and procurement leadership who will act on your words, so accuracy matters more than narrative flair.

--- OUR COMPANY ---
{our_profile}
-------------------

{competitor_mode_instruction}

{public_sector_mode_instruction}

{brief_outcome_instruction}

EVIDENCE DISCIPLINE — THESE RULES OVERRIDE ALL OTHER INSTRUCTIONS:
- Calibrate your claims to the evidence strength. One weak or single source can only support a tentative observation. A firm conclusion requires two or more independent, corroborating sources. If the evidence is thin, say so plainly and keep the insight narrow.
- Distinguish OBSERVED FACTS (directly stated in the evidence, cited as [1], [2]) from your INFERENCES. Mark inferences with words like 'suggests', 'may', 'appears to'. Never present an inference as a fact.
- Do NOT invent. Every proper noun in your output — company names, people, part numbers, customer relationships, locations, dollar figures, deadlines — MUST appear in the evidence signals or the entity profile above. If you cannot find a real name in the evidence, do not name one. Write 'the procurement lead' or 'an engineering contact' rather than inventing 'the VP of Procurement at Siemens Munich'.
- Do NOT extrapolate a commercial consequence unless the causal chain is supported by the evidence. A single social-media post about a topic does not establish a 'strategic pivot', a 'BOM risk', a 'redesign urgency', or a supply disruption. State what was observed and, at most, what it MIGHT imply — clearly marked as speculative.
- Do NOT force a causal or counterfactual structure. Write naturally. Use 'because' only when the evidence genuinely shows causation. Use 'if' only when exploring a real risk, and only when the evidence makes it plausible. Do not pad with mandatory cause-effect or counterfactual sentences.
- It is correct and expected to conclude 'insufficient evidence to recommend a specific action' when the evidence does not support one. A restrained, honest insight is more valuable than a confident-sounding fabrication.
- Every claim must cite a numbered evidence reference [1], [2], etc. that actually supports THAT claim — not a reference to a different topic.
- Our company profile is context, not proof of fit. Do not claim our certifications, footprint, or capabilities are relevant unless the evidence explicitly names a matching procurement, hardware/equipment program, supplier qualification need, or manufacturing requirement.
- Keep direct evidence separate from inference. Do not turn minor hygiene findings such as DNS posture, missing DKIM/SPF/DMARC, or isolated lookalike domains into claims about defense-program exclusion, medical-device qualification failure, customer churn, or supply disruption unless the evidence explicitly links them.
- Do not turn generic certification or accreditation warnings, stale certificate dates, reaffirmation notices, or unspecified compliance page updates into claims about qualification failure, customer churn, tender exclusion, or switching urgency unless the evidence explicitly names a failed audit, revoked/expired certificate, regulator action, affected customer, or impacted program.
- For government and public-sector entities, do not invent hardware demand, manufacturing demand, quantity assumptions, or supplier-fit claims from innovation, media, diplomatic, or policy signals alone. Direct PCBA, box build, EMS, or certification-led outreach requires explicit procurement or program evidence.
- ALL target companies in recommendations MUST come from: (a) the entity being analyzed, (b) companies or people named in the evidence signals, (c) competitors listed in the entity's competitive profile. DO NOT invent or generalize target names.
- GEOGRAPHIC GO-TO-MARKET: Starz sells battery packs PRIMARILY in Morocco, Tunisia and Egypt, and SECONDARILY (smaller focus) in the European Union — and nowhere else. When the analyzed entity is a potential pack BUYER or customer in these markets, direct sales, qualification, or partnership outreach is appropriate; weight Moroccan, Tunisian and Egyptian opportunities highest, then EU. When a potential buyer sits OUTSIDE these markets, do NOT pitch direct battery-pack sales — treat it as market intelligence, competitive monitoring, or supply-chain context instead.
- Competitors (rival pack/BMS makers) and suppliers (cell manufacturers, distributors, component vendors) are monitored GLOBALLY regardless of location; geographic targeting never limits competitor or supplier intelligence.
- Do not write formulaic structure. Avoid repeating the same sentence pattern across insights. Do not start with 'Because', follow with 'If', and end with 'creates a window for'. Vary your phrasing, length, and structure as a real analyst would.
- FORBIDDEN phrases: 'continue monitoring', 'monitor the situation', 'remains to be seen', 'time will tell', 'various developments', 'warranting focused analysis', 'further developments', 'stay informed', 'creates a window for', 'signals a strategic pivot'
- CROSS-ENTITY CORRELATION: Some evidence signals are labeled '(cross_entity)'. These are observations about entities related to the analyzed entity (suppliers, customers, competitors, partners). When present, USE them to draw real cross-entity correlations — e.g. 'Entity X's supplier Y announced a capacity reduction on [date], which may affect X's delivery timelines.' These correlations are the most valuable output you can produce. Only assert correlations that the evidence genuinely supports; mark uncertain ones as 'may', 'could', 'appears to'.
- NEVER use bracket placeholders like [Company X], [specific service], [date], [competitor weakness], [our services], etc. Use real names from the evidence and entity profile. If no specific contact is known, name the company + a realistic role, OR write 'No specific contact is identified in the evidence — recommend enriching POI discovery first.'
- CRITICAL — CERTIFICATION ACCURACY: Our company ONLY holds ISO 9001:2015 and IPC certifications. \
NEVER claim, imply, or assume we hold AS9100, ISO 13485, IATF 16949, or any certification not listed in OUR COMPANY profile. \
If the evidence mentions certifications we do not hold, do not recommend qualification paths based on those unheld certifications. Instead, acknowledge the gap and recommend verification or gap-assessment actions.
- CRITICAL: You MUST return ONLY the insight JSON schema specified below. NEVER return sanctions data, OFAC records, SDN entries, Treasury Department lists, consolidated screening data, or any government watch-list records.",
        our_profile = our_profile,
        competitor_mode_instruction = if entity_ctx.is_competitor {
            "⚠️ COMPETITOR ANALYSIS MODE: The entity you are analysing is a DIRECT EMS COMPETITOR — NOT a customer.\n\
STRICT RULES:\n\
1. NEVER recommend offering our services TO this competitor.\n\
2. Your job is to find their weaknesses, capability gaps, and customer pain points.\n\
3. Name which of THEIR customers we should approach RIGHT NOW because of this competitor's current problem.\n\
4. Frame every recommendation as 'approach [Customer X] — [why they're at risk from this competitor's problem]'.\n\
5. Any recommendation that addresses the competitor directly is WRONG — address their downstream customers instead."
        } else if sc_role == Some("DISTRIBUTOR") {
            "📦 DISTRIBUTOR INTELLIGENCE MODE: This entity DISTRIBUTES electronic components — they are NOT an EMS prospect.\n\
STRICT RULES:\n\
1. NEVER recommend selling assembly services, proposing partnerships, or pitching qualification packages to this distributor.\n\
2. Analyse how their distribution changes (pricing, inventory, new product introductions, logistics shifts) affect OEMs and EMS companies.\n\
3. Recommend procurement hedging, alternate sourcing, or inventory actions for OUR customers based on this distributor's moves.\n\
4. Any recommendation that pitches our services TO this distributor is WRONG."
        } else if sc_role == Some("SEMICONDUCTOR") {
            "🔬 SEMICONDUCTOR INTELLIGENCE MODE: This entity designs/manufactures chips — NOT an EMS prospect.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this semiconductor company.\n\
2. NEVER pitch our certifications, facilities, consulting support, compliance packages, proposals, or generic manufacturing capabilities to this semiconductor company or treat it as a direct services lead.\n\
3. Analyse how their product launches, shortages, EOL notices, or pricing shifts affect our customers' BOMs.\n\
4. Recommend supply chain actions (alternate parts, redesign triggers, pre-buy strategies) for our customer base.\n\
5. Any recommendation that pitches assembly, qualification support, compliance support, or manufacturing to this chip company is WRONG."
        } else if sc_role == Some("PCB_MANUFACTURER") {
            "🟢 PCB MANUFACTURER INTELLIGENCE MODE: This entity makes bare PCBs — they are an upstream supplier.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this PCB manufacturer.\n\
2. Analyse their capacity, quality, lead times, and pricing for impact on our supply chain.\n\
3. Recommend sourcing qualification, dual-source strategies, or supply chain de-risking actions.\n\
4. Any recommendation that pitches our services TO this PCB company is WRONG."
        } else if sc_role == Some("TEST_MEASUREMENT") {
            "🔧 TEST EQUIPMENT INTELLIGENCE MODE: This entity makes test & measurement equipment — they are a tooling vendor.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this T&M company.\n\
2. Analyse how their product updates or pricing affect our test capabilities and manufacturing competitiveness.\n\
3. Recommend equipment investment, qualification, or capability-building actions."
        } else if sc_role == Some("TRADE_ASSOCIATION") {
            "🤝 INDUSTRY BODY INTELLIGENCE MODE: This is a trade association or industry organization — NOT a customer.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this organization.\n\
2. Analyse their policy positions, events, member activities, and regulatory advocacy for market impact.\n\
3. Recommend engagement for visibility, networking, and business development — not direct sales."
        } else if sc_role == Some("DEFENSE_PRIME") {
            "🛡️ DEFENSE PRIME MODE: This is a large defense/aerospace prime contractor that subcontracts EMS work.\n\
STRICT RULES:\n\
1. Do NOT frame this entity as a generic EMS sales prospect or as an 'outsourcing opportunity'.\n\
2. Do NOT use headline language such as 'nearshore EMS opportunities' or 'EMS outsourcing opportunities'.\n\
3. Analyse program timelines, subcontractor needs, compliance requirements, offset obligations, and supply chain gaps.\n\
4. Recommend qualification paths and named programs where our capabilities (nearshore, AS9100, ITAR-free) create an advantage.\n\
5. Frame recommendations around winning Tier-2/Tier-3 subcontracting or supplier-qualification positions."
        } else if sc_role == Some("OEM") {
            "🏭 OEM / END CUSTOMER MODE: This entity designs end products and may outsource manufacturing.\n\
Analyse their outsourcing needs, product roadmap signals, supply chain vulnerabilities, and qualification requirements.\n\
Recommend specific engagement strategies to win or expand EMS contracts with them."
        } else {
            "✅ CUSTOMER/PROSPECT MODE: This entity is a potential customer or partner. Recommend direct outreach, \
service proposals, qualification bids, and strategic partnership opportunities with this entity."
        },
        public_sector_mode_instruction = if is_public_sector {
            if has_public_sector_procurement_program {
                "🏛️ PUBLIC-SECTOR MODE: procurement or program evidence is present, so direct outreach is allowed only if it maps to the named tender, hardware/equipment need, or supplier qualification path in the evidence."
            } else {
                "🏛️ PUBLIC-SECTOR MODE: no explicit procurement or hardware program evidence is present. Do not turn this into a direct EMS sales pitch, qualification package, or manufacturing offer. Keep recommendations on policy impact, account exposure, stakeholder mapping, qualification planning, or scenario planning."
            }
        } else {
            ""
        },
        brief_outcome_instruction = if is_public_sector {
            "Every brief you write must lead to a specific account-planning, qualification, policy-response, or stakeholder action. Direct commercial outreach is allowed only when the evidence explicitly names a procurement, tender, supplier qualification event, or hardware/equipment program."
        } else {
            "Every brief you write must lead to a specific commercial action — a call to make, a bid to prepare, a competitor's customer to approach, or a market gap to fill."
        }
    );

    let user = format!(
        r#"Write a {category_label} brief about {entity_name}.

{entity_profile}

Evidence:
{evidence_text}

Analysis guidance: {analysis_focus}
Action guidance: {action_focus}
Strategic suggestion lanes to consider: {suggestion_axes}

Respond with valid JSON only:
{{
  "headline": "A factual, specific headline (<=140 chars) naming {entity_name} and stating WHAT WAS OBSERVED. Do not write an action verb ('Act on', 'Seize', 'Capture') in the headline — describe the finding.",
        "narrative": "80-280 words of evidence-grounded analysis in natural prose, written as a real analyst would. Cite evidence as [1], [2], [3]. Report what the evidence actually shows. Calibrate every claim: a single weak source supports only a tentative observation; two+ corroborating sources support a firmer conclusion; if the evidence is thin, say so. Mark inferences clearly ('suggests', 'may', 'appears'). Do NOT pad with a mandatory cause-effect or counterfactual sentence. Do NOT use section labels, template headings, or the phrases 'Because... therefore', 'If... would/could', or 'creates a window for'. Vary your sentence structure.",
        "recommendation": "1-3 suggestions in plain prose. Each must be justified by specific evidence you cite as [1], [2]. If the evidence does not support a specific commercial action, write 'Insufficient evidence to recommend a specific commercial action at this time; recommend enriching data collection on this entity before outreach.' Name ONLY companies, people, and programs that appear in the evidence or entity profile. Do not invent target names, part numbers, locations, or deadlines.",
    "confidence": 0.0,
    "severity": "<critical|high|medium|low based on likely business impact>"
}}

REQUIREMENTS:
- Reference at least 2 evidence items as [1], [2], etc.
- Each recommendation must cite at least 1 supporting evidence item as [1], [2], etc.
- Every proper noun (company, person, part number, location, standard, dollar figure, date) in your output MUST exist in the evidence or entity profile. Inventing names or figures is the most serious error you can make.
- Calibrate confidence to the actual evidence strength. One source = at most 0.4. Two corroborating sources = up to 0.6. Three or more independent sources = up to 0.85. Never claim confidence above what the evidence supports.
- Write plain business prose with complete sentences. No templates, no boilerplate labels, no heading prefixes.
- Every sentence must add new information (fact, implication, or action); do not repeat the same claim with paraphrases.
- If the evidence genuinely supports a strong commercial action, make it. If it does not, restraint is the correct answer — say so.
"#,
        category_label = category_label,
        entity_name = entity_ctx.name,
        entity_profile = entity_profile,
        evidence_text = evidence_text,
        analysis_focus = analysis_focus,
        action_focus = action_focus,
        suggestion_axes = {
            // Supply chain roles get role-specific axes regardless of category
            if entity_ctx.is_competitor {
                "competitive displacement, customer rescue, qualification wedge, pricing wedge, regional footprint positioning, executive account planning"
            } else if sc_role == Some("DISTRIBUTOR") {
                "procurement hedging, alternate sourcing, inventory pre-buy strategy, BOM cost impact, supply continuity, customer advisory"
            } else if sc_role == Some("SEMICONDUCTOR") {
                "BOM impact assessment, alternate part qualification, end-of-life migration, customer supply advisory, design-in risk, pricing trend analysis"
            } else if sc_role == Some("PCB_MANUFACTURER") {
                "PCB sourcing diversification, lead time risk mitigation, quality benchmark comparison, dual-source qualification, supply chain de-risking"
            } else if sc_role == Some("TEST_MEASUREMENT") {
                "test capability investment, equipment qualification, manufacturing competitiveness, throughput improvement, technology roadmap alignment"
            } else if sc_role == Some("TRADE_ASSOCIATION") {
                "event engagement, policy monitoring, member network mapping, standards participation, visibility building, regulatory intelligence"
            } else if is_public_sector {
                match category {
                    "brand_sentiment" => "institutional credibility assessment, procurement scrutiny mapping, stakeholder messaging review, account dependency review, scenario planning, executive briefing",
                    "regulatory_policy" | "geopolitical_analysis" => "policy impact mapping, qualification planning, account dependency review, stakeholder outreach, executive scenario planning, procurement path verification",
                    "security_compliance" | "cybersecurity_threat" | "quality_compliance" => "official-surface validation, supplier access review, containment planning, stakeholder brief, assurance planning, executive escalation",
                    _ => "account planning, stakeholder mapping, institutional process review, evidence verification, qualification planning, executive briefing",
                }
            } else if sc_role == Some("DEFENSE_PRIME") {
                "program qualification, Tier-2/Tier-3 subcontracting, offset obligations, compliance readiness, supply chain gap-fill, executive program engagement"
            } else if sc_role == Some("OEM") {
                "outsourcing capture, NPI engagement, qualification bid, design-for-manufacturing advisory, regional footprint advantage, executive sponsor mapping"
            } else {
                match category {
                    "demand_procurement" | "customer_rfq" => "revenue capture, qualification readiness, prototype or NPI entry, pricing leverage, regional footprint positioning, executive sponsor mapping",
                    "supply_chain_risk" => "continuity protection, dual-source qualification, customer assurance, design migration, regional rerouting, executive risk briefing",
                    "regulatory_policy" | "geopolitical_analysis" => "regulatory posture, export-control routing, nearshoring, customer communication, qualification planning, executive scenario planning",
                    "strategic_poi" | "talent_ip" => "POI mapping, early project engagement, competitive positioning, executive outreach, stakeholder timing",
                    "security_compliance" | "cybersecurity_threat" | "quality_compliance" => "audit readiness, security assurance, supplier governance, containment planning, customer reassurance, executive escalation",
                    _ => "revenue capture, resilience, competitive positioning, pricing leverage, regional expansion, executive planning",
                }
            }
        },
    );

    let config = InferenceConfig {
        temperature: 0.3,
        max_tokens: 3072,
        json_mode: true,
        suppress_thinking: false,
        timeout: crate::config::llm_timeout(),
        ..Default::default()
    };

    #[derive(serde::Deserialize)]
    struct LlmInsightResponse {
        headline: String,
        narrative: String,
        #[serde(default, deserialize_with = "deserialize_recommendation")]
        recommendation: Option<String>,
        confidence: f64,
        #[serde(default)]
        severity: String,
    }

    /// Accept recommendation as string, array of strings, or array of objects.
    fn deserialize_recommendation<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::Deserialize;
        let val: Option<serde_json::Value> = Option::deserialize(deserializer)?;
        match val {
            Some(serde_json::Value::String(s)) => Ok(Some(s)),
            Some(serde_json::Value::Array(arr)) => {
                let items: Vec<String> = arr
                    .into_iter()
                    .filter_map(|v| match v {
                        serde_json::Value::String(s) => Some(s),
                        serde_json::Value::Object(map) => {
                            // Handle {"action":"...", "owner":"...", "deadline":"..."}
                            let parts: Vec<String> =
                                ["action", "owner", "deadline", "description", "task"]
                                    .iter()
                                    .filter_map(|key| {
                                        map.get(*key)
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .collect();
                            if parts.is_empty() {
                                // Fallback: join all string values
                                let all_strings: Vec<String> = map
                                    .values()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                                if all_strings.is_empty() {
                                    None
                                } else {
                                    Some(all_strings.join(" — "))
                                }
                            } else {
                                Some(parts.join(" — "))
                            }
                        }
                        _ => None,
                    })
                    .collect();
                if items.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(items.join(". ")))
                }
            }
            Some(serde_json::Value::Object(map)) => {
                // Single object recommendation
                let parts: Vec<String> = map
                    .values()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if parts.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(parts.join(" — ")))
                }
            }
            Some(serde_json::Value::Null) | None => Ok(None),
            _ => Ok(None),
        }
    }

    fn normalize_assessment_severity(raw: &str, confidence: f64) -> &'static str {
        match raw.trim().to_ascii_lowercase().as_str() {
            "critical" => "critical",
            "high" => "high",
            "warning" | "medium" => "medium",
            "info" | "low" => "low",
            _ if confidence >= 0.8 => "critical",
            _ if confidence >= 0.7 => "high",
            _ if confidence >= 0.4 => "medium",
            _ => "low",
        }
    }

    // Ban truly formulaic/filler phrases that indicate passive analysis.
    let generic_phrases: &[&str] = &[
        "continue monitoring",
        "various developments",
        "warranting focused analysis",
        "further developments",
        "remains to be seen",
        "time will tell",
        "developments warrant attention",
        "stay informed",
        "keep an eye on",
        "in conclusion",
        "it is important to note",
        "overall",
    ];

    let malformed_fragments: &[&str] = &[
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
        "[object object]",
        "undefined",
        "{{",
        "}}",
    ];

    let mut previous_failure_reasons: Vec<&'static str> = Vec::new();
    for attempt in 1..=*crate::config::LLM_MAX_RETRIES {
        // Exponential backoff: 0s on first attempt, 1s, 2s, 4s...
        if attempt > 1 {
            let backoff_ms = 1000u64 * (1u64 << (attempt - 2).min(4));
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        }
        let retry_guidance =
            build_llm_retry_guidance(entity_ctx, category, &previous_failure_reasons);
        let mut messages = vec![
            ChatMessage::system(system.as_str()),
            ChatMessage::user(&user),
        ];
        if let Some(guidance) = retry_guidance.as_deref() {
            messages.push(ChatMessage::user(guidance));
        }
        let mut resp = llm_client
            .complete_with_config(messages, &config)
            .await
            .with_context(|| format!("LLM insight generation failed for {}", entity_ctx.name))?;

        tracing::info!(
            entity = %entity_ctx.name,
            attempt,
            raw_len = resp.text.len(),
            raw_preview = %crate::truncate_text(&resp.text, 300),
            "LLM raw response"
        );

        // ── SDN/OFAC contamination guard ──
        // The fine-tuned model may regurgitate sanctions data instead of insight JSON.
        // Detect this early and retry with corrective guidance.
        let raw_lower = resp.text.to_ascii_lowercase();
        let is_sdn_contaminated = raw_lower.contains("specially designated nationals")
            || raw_lower.contains("\"source\": \"specially designated")
            || raw_lower.contains("ofac")
            || raw_lower.contains("treasury department")
            || raw_lower.contains("entity_number")
            || raw_lower.contains("sdn list")
            || raw_lower.contains("consolidated screening")
            || (raw_lower.contains("\"_id\"") && raw_lower.contains("\"programs\""));

        if is_sdn_contaminated {
            tracing::warn!(
                entity = %entity_ctx.name,
                attempt,
                raw_preview = %crate::truncate_text(&resp.text, 200),
                "LLM SDN/OFAC contamination detected — model returned sanctions data instead of insight JSON"
            );
            previous_failure_reasons.push("sdn_contamination");
            crate::observability::WORKER_METRICS.record_llm_retry();
            continue;
        }

        // ── Certification invention guard ──
        // The model sometimes claims certifications (AS9100, IATF 16949, ISO 13485)
        // that our company does NOT hold. Detect this in the raw response and retry.
        // Our company ONLY holds ISO 9001:2015 and IPC.
        let raw_text = resp.text.to_ascii_lowercase();
        let is_cert_invented = {
            // Check for possessive claims: "our [forbidden_cert]", "we hold [forbidden_cert]", etc.
            let has_forbidden_cert = [
                "as9100",
                "iatf 16949",
                "iatf16949",
                "iso 13485",
                "13485 certification",
            ]
            .iter()
            .any(|c| raw_text.contains(c));
            let has_possessive_pattern = [
                "our as9100",
                "our iatf",
                "our iso 13485",
                "as9100 certification",
                "iatf 16949 certification",
                "iso 13485 certification",
                "as9100 certified",
                "iatf certified",
                "13485 certified",
                "we hold as9100",
                "we hold iatf",
                "we hold iso 13485",
                "we are as9100",
                "we are iatf",
                "we are iso 13485",
                "we have as9100",
                "we have iatf",
                "we have iso 13485",
                "our facility is as9100",
                "our facility is iatf",
                "as9100 and iso 13485",
                "iatf 16949 and ",
                "with our as9100",
                "with our iatf",
                "with our iso 13485",
            ]
            .iter()
            .any(|p| raw_text.contains(p));
            has_forbidden_cert && has_possessive_pattern
        };

        if is_cert_invented {
            if attempt < *crate::config::LLM_MAX_RETRIES {
                // First violation: give the model one corrective retry.
                tracing::warn!(
                    entity = %entity_ctx.name,
                    attempt,
                    raw_preview = %crate::truncate_text(&resp.text, 200),
                    "LLM certification invention detected — retrying with corrective guidance"
                );
                previous_failure_reasons.push("certification_invention");
                crate::observability::WORKER_METRICS.record_llm_retry();
                continue;
            } else {
                // Final attempt still violates: sanitize the forbidden
                // certification claims from the response text and proceed,
                // rather than discarding the entire insight. This avoids the
                // wasteful retry loop that blocked recipe-fire throughput on
                // entities (e.g. Sanmina) where the LLM persistently invented
                // certification claims despite corrective guidance.
                tracing::warn!(
                    entity = %entity_ctx.name,
                    attempt,
                    "LLM certification invention persists after retries — sanitizing forbidden claims and accepting output"
                );
                resp.text = sanitize_certification_claims(&resp.text);
            }
        }

        let parsed: LlmInsightResponse = match resp.parse_json() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(entity = %entity_ctx.name, attempt, error = %e, "LLM JSON parse failed");
                crate::observability::WORKER_METRICS.record_llm_retry();
                continue;
            }
        };

        let recommendation = parsed.recommendation.unwrap_or_default().trim().to_string();
        let narrative = parsed.narrative.trim().to_string();
        let headline = parsed.headline.trim().to_string();
        let role_guidance_violation =
            violates_supply_chain_role_guidance(sc_role, &headline, &narrative, &recommendation);
        let topic_alignment_violation = violates_entity_topic_alignment(
            entity_ctx,
            evidence_signals,
            &headline,
            &narrative,
            &recommendation,
        );

        let headline_lower = headline.to_lowercase();
        let narrative_lower = narrative.to_lowercase();
        let recommendation_lower = recommendation.to_lowercase();
        let is_generic =
            generic_phrases.iter().any(|p| {
                headline_lower.contains(p)
                    || narrative_lower.contains(p)
                    || recommendation_lower.contains(p)
            }) || has_formulaic_commercial_language(&headline, &narrative, &recommendation);

        let words = narrative.split_whitespace().count();
        let reference_count = count_numbered_references(&narrative, 12);
        let has_refs = reference_count >= 2;
        let recommendation_reference_count = count_numbered_references(&recommendation, 12);
        let recommendation_has_refs = recommendation_reference_count >= 1;
        let has_digits = narrative.chars().any(|c| c.is_ascii_digit());
        let has_recommendation = recommendation.split_whitespace().count() >= 12;
        let is_headline_ok = !headline.is_empty() && headline.len() <= 160;
        let has_causal_language = [
            "because",
            "therefore",
            "as a result",
            "which means",
            "implies",
            "drives",
            "leads to",
        ]
        .iter()
        .any(|p| narrative_lower.contains(p));
        let has_counterfactual = narrative_lower.contains("if ")
            && (narrative_lower.contains(" would ") || narrative_lower.contains(" could "));
        let has_reasoning_depth = has_causal_language || has_counterfactual;
        let recommendation_has_deadline =
            recommendation_has_action_timing(category, &recommendation);
        let malformed = malformed_fragments.iter().any(|f| {
            headline.to_ascii_lowercase().contains(f)
                || narrative_lower.contains(f)
                || recommendation_lower.contains(f)
        });

        let readable_narrative = is_readable_and_useful_digest_text(&narrative)
            && !has_excessive_phrase_repetition(&recommendation);
        let readable_recommendation = recommendation
            .split(['.', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .count()
            >= 2;
        let unsupported_security_escalation = has_unsupported_security_escalation(
            category,
            &narrative,
            &recommendation,
            evidence_signals,
        );
        let unsupported_certification_escalation =
            has_unsupported_certification_escalation(&narrative, &recommendation, evidence_signals);
        let unsupported_certification_commercialization =
            has_unsupported_certification_commercialization(
                &headline,
                &narrative,
                &recommendation,
                evidence_signals,
            );
        let unsupported_public_sector_commercialization =
            has_unsupported_public_sector_commercialization(
                entity_ctx,
                category,
                &narrative,
                &recommendation,
                evidence_signals,
            );
        let low_usefulness_public_sector_analysis = has_low_usefulness_public_sector_analysis(
            entity_ctx,
            category,
            &headline,
            &narrative,
            &recommendation,
            evidence_signals,
        );
        let temporal_incoherence =
            has_temporal_incoherence(&entity_ctx.name, &narrative, Utc::now(), evidence_signals);
        let unnamed_customer_targeting =
            has_unnamed_customer_targeting(&narrative, &recommendation);
        let unsupported_named_target_provenance = has_unsupported_named_target_provenance(
            entity_ctx,
            &headline,
            &narrative,
            &recommendation,
            evidence_signals,
        );

        // Reject if recommendation contains bracket placeholders like [Company X], [specific service], [date]
        let placeholder_patterns: &[&str] = &[
            "[company ",
            "[specific ",
            "[date]",
            "[competitor ",
            "[our ",
            "[their ",
            "[service",
            "[product",
            "[client ",
            "[customer ",
            "[contact ",
        ];
        let has_placeholders = placeholder_patterns
            .iter()
            .any(|p| recommendation_lower.contains(p));

        // Check for confidence/source boilerplate patterns (template leakage)
        let narrative_has_confidence_boilerplate = has_confidence_boilerplate(&narrative)
            || has_confidence_boilerplate(&recommendation)
            || has_confidence_boilerplate(&headline);

        let gate_decisions = vec![
            quality_gate_blocker("generic_language", is_generic, false),
            quality_gate_blocker("malformed_output", malformed, false),
            quality_gate_blocker("placeholder_output", has_placeholders, false),
            quality_gate_blocker(
                "confidence_boilerplate",
                narrative_has_confidence_boilerplate,
                true,
            ),
            quality_gate_requirement("narrative_words", words as f32, 85.0),
            quality_gate_requirement("narrative_references", reference_count as f32, 2.0),
            quality_gate_requirement(
                "recommendation_references",
                recommendation_reference_count as f32,
                1.0,
            ),
            quality_gate_requirement(
                "narrative_has_quantification",
                if has_digits { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_depth",
                recommendation.split_whitespace().count() as f32,
                12.0,
            ),
            quality_gate_requirement(
                "reasoning_depth",
                if has_reasoning_depth { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_timing",
                if recommendation_has_deadline {
                    1.0
                } else {
                    0.0
                },
                0.5,
            ),
            quality_gate_requirement(
                "narrative_readability",
                if readable_narrative { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_readability",
                if readable_recommendation { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_blocker("security_escalation", unsupported_security_escalation, true),
            quality_gate_blocker(
                "certification_escalation",
                unsupported_certification_escalation,
                false,
            ),
            quality_gate_blocker(
                "certification_commercialization",
                unsupported_certification_commercialization,
                true,
            ),
            quality_gate_blocker(
                "public_sector_commercialization",
                unsupported_public_sector_commercialization,
                true,
            ),
            quality_gate_blocker(
                "public_sector_low_usefulness",
                low_usefulness_public_sector_analysis,
                true,
            ),
            quality_gate_blocker("temporal_incoherence", temporal_incoherence, true),
            quality_gate_blocker(
                "unnamed_customer_targeting",
                unnamed_customer_targeting,
                true,
            ),
            quality_gate_blocker(
                "named_target_provenance",
                unsupported_named_target_provenance,
                true,
            ),
            quality_gate_blocker("topic_alignment", topic_alignment_violation, true),
            quality_gate_blocker("role_guidance", role_guidance_violation, true),
            quality_gate_requirement(
                "headline_length",
                if is_headline_ok { 1.0 } else { 0.0 },
                0.5,
            ),
        ];
        emit_quality_gate_decisions(
            &entity_ctx.name,
            category,
            attempt as usize,
            &gate_decisions,
        );

        let passes = quality_gate_passes_ensemble(&gate_decisions);
        let ensemble_failures = gate_decisions
            .iter()
            .filter(|decision| decision.failed && !decision.veto)
            .count();
        let veto_rejected = gate_decisions
            .iter()
            .any(|decision| decision.failed && decision.veto);

        if passes {
            crate::observability::WORKER_METRICS.record_llm_success();
            crate::observability::WORKER_METRICS.record_insight_accepted();
            let parsed_confidence = parsed.confidence.clamp(0.0, 1.0);
            let assessment_severity =
                normalize_assessment_severity(&parsed.severity, parsed_confidence);
            let mut consensus_reached = true;
            let mut dissenting_opinions = Vec::new();

            if assessment_severity == "critical" {
                for sample_idx in 1..=2 {
                    let sample_instruction = format!(
                        "Independent review sample {}. Reassess the evidence from scratch, remain evidence-bound, and return the same JSON schema.",
                        sample_idx
                    );
                    let sample_messages = vec![
                        ChatMessage::system(system.as_str()),
                        ChatMessage::user(&user),
                        ChatMessage::user(sample_instruction),
                    ];

                    match llm_client
                        .complete_with_config(sample_messages, &config)
                        .await
                    {
                        Ok(sample_resp) => match sample_resp.parse_json::<LlmInsightResponse>() {
                            Ok(sample) => {
                                let sample_confidence = sample.confidence.clamp(0.0, 1.0);
                                let sample_severity = normalize_assessment_severity(
                                    &sample.severity,
                                    sample_confidence,
                                );
                                if sample_severity != assessment_severity {
                                    consensus_reached = false;
                                    dissenting_opinions.push(serde_json::json!({
                                        "severity": sample_severity,
                                        "category": category,
                                        "confidence": sample_confidence,
                                        "rationale_summary": crate::truncate_text(
                                            sample.narrative.trim(),
                                            200,
                                        ),
                                    }));
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    entity = %entity_ctx.name,
                                    sample_idx,
                                    %error,
                                    "LLM consensus sample JSON parse failed"
                                );
                            }
                        },
                        Err(error) => {
                            tracing::warn!(
                                entity = %entity_ctx.name,
                                sample_idx,
                                %error,
                                "LLM consensus sample request failed"
                            );
                        }
                    }
                }
            }

            let metadata = serde_json::json!({
                "assessment_severity": assessment_severity,
                "assessment_category": category,
                "consensus_reached": consensus_reached,
                "dissenting_opinions": dissenting_opinions,
            });
            return Ok((
                headline,
                narrative,
                recommendation,
                parsed_confidence,
                metadata,
            ));
        }

        previous_failure_reasons.clear();
        if !has_reasoning_depth {
            previous_failure_reasons.push("reasoning");
        }
        if !recommendation_has_deadline {
            previous_failure_reasons.push("timing");
        }
        if !readable_narrative || !readable_recommendation || is_generic || malformed {
            previous_failure_reasons.push("readability");
        }
        if unnamed_customer_targeting {
            previous_failure_reasons.push("unnamed_customer_targeting");
        }
        if unsupported_named_target_provenance {
            previous_failure_reasons.push("named_target_provenance");
        }
        if unsupported_public_sector_commercialization {
            previous_failure_reasons.push("public_sector_commercialization");
        }
        if low_usefulness_public_sector_analysis {
            previous_failure_reasons.push("public_sector_low_usefulness");
        }
        if topic_alignment_violation {
            previous_failure_reasons.push("topic_alignment");
        }
        if role_guidance_violation {
            previous_failure_reasons.push("role_guidance");
        }
        if unsupported_certification_escalation {
            previous_failure_reasons.push("certification_escalation");
        }
        if unsupported_certification_commercialization {
            previous_failure_reasons.push("certification_commercialization");
        }
        if unsupported_security_escalation {
            previous_failure_reasons.push("security_escalation");
        }

        tracing::warn!(
            entity = %entity_ctx.name,
            attempt,
            words,
            reference_count,
            has_refs,
            recommendation_reference_count,
            recommendation_has_refs,
            has_digits,
            has_recommendation,
            has_reasoning_depth,
            recommendation_has_deadline,
            readable_narrative,
            readable_recommendation,
            unsupported_security_escalation,
            unsupported_certification_escalation,
            unsupported_certification_commercialization,
            unsupported_public_sector_commercialization,
            low_usefulness_public_sector_analysis,
            unnamed_customer_targeting,
            unsupported_named_target_provenance,
            topic_alignment_violation,
            role_guidance_violation,
            ensemble_failures,
            veto_rejected,
            recommendation_words = recommendation.split_whitespace().count(),
            recommendation_preview = %crate::truncate_text(&recommendation, 120),
            generic = is_generic,
            has_placeholders,
            malformed,
            headline = %headline,
            narrative_preview = %crate::truncate_text(&narrative, 180),
            "LLM quality gate: rejected"
        );
    }

    crate::observability::WORKER_METRICS.record_llm_failure();
    crate::observability::WORKER_METRICS.record_insight_rejected();
    anyhow::bail!(
        "LLM failed quality checks after retries for {}",
        entity_ctx.name
    )
}

#[allow(dead_code)]
pub(crate) fn push_entity_source_url(
    urls_by_entity: &mut HashMap<String, Vec<String>>,
    entity_id: Uuid,
    url: &str,
) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }

    let entry = urls_by_entity.entry(entity_id.to_string()).or_default();
    if entry
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        return;
    }

    entry.push(trimmed.to_string());
}

#[cfg(not(feature = "llm"))]
pub(crate) async fn collect_entity_evidence_urls(
    store: &PgStore,
    entity_ids: &[Uuid],
) -> HashMap<String, Vec<String>> {
    let unique_entity_ids: std::collections::HashSet<Uuid> = entity_ids.iter().copied().collect();
    let query_entity_ids: Vec<Uuid> = unique_entity_ids.iter().copied().collect();
    let mut urls_by_entity: HashMap<String, Vec<String>> = HashMap::new();

    if let Ok(rows) = store
        .get_warnings_by_entity_ids(&query_entity_ids, 500)
        .await
    {
        for warning in rows {
            let Some(entity_ids) = warning.entity_ids.as_ref() else {
                continue;
            };
            let Some(source_urls) = warning.source_urls.as_ref() else {
                continue;
            };
            for entity_id in entity_ids {
                for url in source_urls.iter().take(3) {
                    push_entity_source_url(&mut urls_by_entity, *entity_id, url);
                }
            }
        }
    }

    for entity_id in query_entity_ids {
        if let Ok(certs) = store.get_certifications_for_company(entity_id).await {
            for cert in certs.iter().take(8) {
                if let Some(url) = cert.evidence_url.as_deref() {
                    push_entity_source_url(&mut urls_by_entity, entity_id, url);
                }
            }
        }

        if let Ok(capabilities) = store.list_capabilities(Some(entity_id), 12, 0).await {
            for capability in capabilities.iter().take(8) {
                if let Some(urls) = capability.evidence_urls.as_ref() {
                    for url in urls.iter().take(2) {
                        push_entity_source_url(&mut urls_by_entity, entity_id, url);
                    }
                }
            }
        }

        if let Ok(observations) = store.get_observations_by_entity(entity_id, 10).await {
            for observation in observations {
                if let Some(url) = observation
                    .provenance
                    .as_object()
                    .and_then(|provenance| {
                        provenance
                            .get("source_url")
                            .or_else(|| provenance.get("url"))
                    })
                    .and_then(|value| value.as_str())
                {
                    push_entity_source_url(&mut urls_by_entity, entity_id, url);
                }
            }
        }
    }

    urls_by_entity
}
