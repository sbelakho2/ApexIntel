#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::absurd_extreme_comparisons
)]

use super::*;
#[cfg(feature = "llm")]
use crate::llm_orchestration::{summarize_quality_gate_reviews, QualityGateReview};

#[cfg(feature = "llm")]
#[test]
fn classify_role_family_detects_supply_chain_buyers() {
    assert_eq!(
        classify_role_family(Some("Senior Buyer")),
        RoleFamily::Procurement
    );
    assert_eq!(
        classify_role_family(Some("Head of Supply Chain")),
        RoleFamily::Procurement
    );
    assert_eq!(
        classify_role_family(Some("Category Manager, Packaging")),
        RoleFamily::Procurement
    );
    assert_eq!(
        classify_role_family(Some("Responsable Achats")),
        RoleFamily::Procurement
    );
}

#[cfg(feature = "llm")]
fn matrix_entity_context(public_sector: bool) -> EntityContext {
    EntityContext {
        name: if public_sector {
            "European Commission".to_string()
        } else {
            "Key Tronic Corporation".to_string()
        },
        region: if public_sector {
            "Europe".to_string()
        } else {
            "North America".to_string()
        },
        entity_type: Some(if public_sector {
            "Government".to_string()
        } else {
            "EMS".to_string()
        }),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec!["AS9100".to_string(), "ISO 13485".to_string()],
        capabilities: vec!["PCBA".to_string()],
        key_persons: vec!["Procurement Lead: Jane Doe".to_string()],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some(if public_sector {
            "ec.europa.eu".to_string()
        } else {
            "keytronic.com".to_string()
        }),
    }
}

#[cfg(feature = "llm")]
#[test]
fn inferred_supply_chain_role_detects_semiconductor_from_context() {
    let entity_ctx = EntityContext {
        name: "Microchip Technology".to_string(),
        region: "US".to_string(),
        entity_type: Some("OEM".to_string()),
        is_competitor: false,
        industry_tags: vec!["semiconductor".to_string(), "automotive".to_string()],
        certifications: vec![],
        capabilities: vec![
            "MCU portfolio".to_string(),
            "power management ICs".to_string(),
        ],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: None,
    };

    assert_eq!(
        inferred_supply_chain_role(&entity_ctx),
        Some("SEMICONDUCTOR")
    );
}

#[cfg(feature = "llm")]
#[test]
fn role_guidance_rejects_semiconductor_ems_pitch_language() {
    assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "Microchip Technology's CEO shift opens EMS outsourcing opportunities",
            "Microchip Technology is a chip supplier, but this creates nearshore EMS opportunities for us [1].",
            "Offer assembly services to Microchip procurement [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn role_guidance_rejects_semiconductor_proposal_pitch_language() {
    assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NXP Semiconductors faces supply chain risks due to regulatory changes",
            "NXP Semiconductors is navigating regulatory shifts that could affect procurement, creating an opportunity for our company to position itself as a reliable partner in maintaining compliance.",
            "Reach out to NXP's operations leadership within 30 days and prepare a detailed proposal highlighting our ISO 14001 certification and green manufacturing capabilities.",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn role_guidance_rejects_semiconductor_qualification_support_pitch_language() {
    assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NVIDIA's AS9100 gap risks aerospace contracts",
            "NVIDIA's AS9100 compliance gap creates a direct opportunity for our aerospace-focused EMS services because defense-adjacent buyers may need alternative qualification paths.",
            "Offer expedited qualification support using our Tunisia and Morocco facilities before Q3 2026 [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn role_guidance_allows_semiconductor_customer_supply_advisory() {
    assert!(!violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NXP regulatory update may affect automotive sourcing windows",
            "The regulatory shift could narrow documentation timing for automotive OEMs that depend on NXP MCUs [1].",
            "Advise affected automotive customers to review buffer stock, alternate-part qualification, and redesign triggers before the next sourcing cycle [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn inferred_supply_chain_role_detects_defense_prime_from_context() {
    let entity_ctx = EntityContext {
        name: "BAE Systems".to_string(),
        region: "UK".to_string(),
        entity_type: Some("company".to_string()),
        is_competitor: false,
        industry_tags: vec!["aerospace and defense".to_string()],
        certifications: vec!["AS9100D".to_string()],
        capabilities: vec!["munitions systems".to_string()],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec!["Glascoed munitions facility".to_string()],
        competitor_events: vec![],
        domain: None,
    };

    assert_eq!(
        inferred_supply_chain_role(&entity_ctx),
        Some("DEFENSE_PRIME")
    );
}

#[cfg(feature = "llm")]
#[test]
fn role_guidance_rejects_defense_prime_generic_ems_opportunity_language() {
    assert!(violates_supply_chain_role_guidance(
        Some("DEFENSE_PRIME"),
        "BAE Systems' munitions factory delay opens nearshore EMS opportunities",
        "The delay creates an EMS opportunity for direct outreach to BAE Systems [1].",
        "Position nearshore EMS capacity to BAE Systems immediately [1].",
    ));
}

#[cfg(feature = "llm")]
#[test]
fn topic_alignment_rejects_unsupported_oil_narrative() {
    let entity_ctx = EntityContext {
        name: "NVIDIA".to_string(),
        region: "US".to_string(),
        entity_type: Some("semiconductor".to_string()),
        is_competitor: false,
        industry_tags: vec!["semiconductor".to_string(), "ai".to_string()],
        certifications: vec![],
        capabilities: vec![
            "GPU platforms".to_string(),
            "data center accelerators".to_string(),
        ],
        key_persons: vec![],
        recent_changes: vec!["Blackwell server launch".to_string()],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: None,
    };
    let evidence_signals = vec![EvidenceSignal {
            title: "NVIDIA expands AI server program".to_string(),
            description:
                "Hyperscaler demand is lifting GPU server deployments and data center capacity planning."
                    .to_string(),
            source_url: "https://example.com/nvidia".to_string(),
            signal_type: "news".to_string(),
            extracted_facts: vec!["GPU server".to_string(), "data center".to_string()],
            date_context: Some("2026-03-18".to_string()),
            relevance_score: 1.0,
        }];

    assert!(violates_entity_topic_alignment(
            &entity_ctx,
            &evidence_signals,
            "Middle East oil surge changes NVIDIA demand",
            "NVIDIA now sits in the path of a Middle East oil surge and refinery spending wave that will reshape its sales mix [1] [2].",
            "Brief the account team on oil price and refinery demand exposure within 30 days [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn topic_alignment_allows_supported_semiconductor_narrative() {
    let entity_ctx = EntityContext {
        name: "NVIDIA".to_string(),
        region: "US".to_string(),
        entity_type: Some("semiconductor".to_string()),
        is_competitor: false,
        industry_tags: vec!["semiconductor".to_string(), "ai".to_string()],
        certifications: vec![],
        capabilities: vec![
            "GPU platforms".to_string(),
            "data center accelerators".to_string(),
        ],
        key_persons: vec![],
        recent_changes: vec!["Blackwell server launch".to_string()],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: None,
    };
    let evidence_signals = vec![EvidenceSignal {
            title: "NVIDIA expands AI server program".to_string(),
            description:
                "Hyperscaler demand is lifting GPU server deployments and data center capacity planning."
                    .to_string(),
            source_url: "https://example.com/nvidia".to_string(),
            signal_type: "news".to_string(),
            extracted_facts: vec!["GPU server".to_string(), "data center".to_string()],
            date_context: Some("2026-03-18".to_string()),
            relevance_score: 1.0,
        }];

    assert!(!violates_entity_topic_alignment(
            &entity_ctx,
            &evidence_signals,
            "AI server demand lifts NVIDIA planning urgency",
            "NVIDIA is seeing stronger data center demand because hyperscalers are accelerating GPU server deployment windows [1] [2].",
            "Prepare a response plan for the next data center qualification cycle within 30 days [1].",
        ));
}

#[cfg(feature = "llm")]
fn matrix_signal(category: &str, public_sector: bool) -> Vec<EvidenceSignal> {
    if public_sector {
        return vec![EvidenceSignal {
                title: "Trade defense consultation notice".to_string(),
                description: "The European Commission published a consultation notice affecting supplier qualification timing, stakeholder review, and account dependency planning for impacted programs.".to_string(),
                source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
                signal_type: "regulatory".to_string(),
                extracted_facts: vec![
                    "consultation notice".to_string(),
                    "supplier qualification".to_string(),
                    "approval timing".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            }];
    }

    let (title, description, signal_type, facts) = match category {
            "demand_procurement" => (
                "OEM sourcing review launched",
                "The OEM opened a sourcing review for medical and industrial assemblies with qualification timing noted for Q2 2026.",
                "procurement",
                vec!["Q2 2026".to_string(), "sourcing review".to_string()],
            ),
            "supply_chain_risk" => (
                "Supplier delay hits production schedule",
                "A component delay is extending lead times by 14 days and creating continuity pressure for regulated programs.",
                "warning",
                vec!["14 days".to_string(), "continuity pressure".to_string()],
            ),
            "security_compliance" => (
                "Certification warning posted",
                "A compliance warning and certification issue were posted on the capabilities page without a named audit failure.",
                "certification",
                vec!["compliance warning".to_string(), "certification issue".to_string()],
            ),
            "competitor_market" => (
                "Competitor margin pressure rises",
                "Pricing pressure and program delays are creating customer risk across the competitor account base in Q2 2026.",
                "news",
                vec!["Q2 2026".to_string(), "program delays".to_string()],
            ),
            _ => (
                "Tariff notice issued",
                "A tariff notice affects export-control timing and supplier qualification planning for the next sourcing cycle.",
                "regulatory",
                vec!["tariff notice".to_string(), "next sourcing cycle".to_string()],
            ),
        };

    vec![EvidenceSignal {
        title: title.to_string(),
        description: description.to_string(),
        source_url: "https://example.com/source".to_string(),
        signal_type: signal_type.to_string(),
        extracted_facts: facts,
        date_context: Some("2026-03-07".to_string()),
        relevance_score: 1.0,
    }]
}

#[cfg(feature = "llm")]
fn matrix_payload(
    category: &str,
    public_sector: bool,
    quality: &str,
) -> (String, String, String, bool) {
    match (public_sector, quality) {
            (false, "good") => (
                format!("{} program shift creates a timely account move", category),
                "Key Tronic Corporation is facing a documented operating shift in 2026 because the latest evidence names sourcing timing, program exposure, and concrete planning windows [1] [2]. That matters now because procurement teams typically lock qualification paths before the next review cycle, therefore a delayed response would reduce influence over line allocation and supplier shortlist design. If the current pressure deepens, buyers could widen the review scope and demand stronger continuity proof, which would favor suppliers that move before the Q2 2026 window closes. The evidence points to a specific timing problem rather than a generic market observation.".to_string(),
                "Approach Key Tronic Corporation's procurement team within 30 days with a qualification-led continuity proposal tied to the named 2026 review window and the sourcing pressure in the evidence [1].".to_string(),
                true,
            ),
            (false, "borderline") => (
                format!("{} timing window deserves action", category),
                "Key Tronic Corporation is facing a documented operating shift because the latest evidence names sourcing timing, program exposure, and concrete planning windows [1] [2]. That matters now because procurement teams typically lock qualification paths before the next review cycle, therefore a delayed response would reduce influence over line allocation and supplier shortlist design. If the current pressure deepens, buyers could widen the review scope and demand stronger continuity proof, which would favor suppliers that move before the current review closes. The evidence points to a specific timing problem rather than a generic market observation.".to_string(),
                "Approach Key Tronic Corporation's procurement team within 30 days with a qualification-led continuity proposal tied to the named review window and the sourcing pressure in the evidence [1].".to_string(),
                true,
            ),
            (false, _) => (
                format!("{} update", category),
                "Various developments warrant focused analysis and it is important to note that the situation should continue monitoring.".to_string(),
                "Monitor the situation when possible.".to_string(),
                false,
            ),
            (true, "good") => (
                "Policy artifact changes supplier planning".to_string(),
                "The European Commission published a named consultation notice on 2026-03-07 that changes qualification timing because affected suppliers may need to adjust account plans before the next review window [1] [2]. That matters commercially because institutional approval timing can constrain which suppliers remain viable during a formal policy cycle, therefore account teams need to map dependency and stakeholder exposure now instead of waiting for a later clarification. If the review scope widens, customers with open qualification paths could face slower approvals, which would make early scenario planning materially more valuable.".to_string(),
                "Brief the named account team this quarter on policy impact, stakeholder dependencies, and qualification timing before the next Commission review window closes [1].".to_string(),
                true,
            ),
            (true, "borderline") => (
                "Policy artifact changes supplier planning".to_string(),
                "The European Commission published a named consultation notice that changes qualification timing because affected suppliers may need to adjust account plans before the next review window [1] [2]. That matters commercially because institutional approval timing can constrain which suppliers remain viable during a formal policy cycle, therefore account teams need to map dependency and stakeholder exposure now instead of waiting for a later clarification. If the review scope widens, customers with open qualification paths could face slower approvals, which would make early scenario planning materially more valuable for the account.".to_string(),
                "Brief the named account team this quarter on policy impact, stakeholder dependencies, and qualification timing before the next Commission review window closes [1].".to_string(),
                true,
            ),
            (true, _) => (
                "Nearshoring opportunity emerging".to_string(),
                "The latest public-sector developments suggest a direct electronics manufacturing opportunity and broader EMS potential for North Africa.".to_string(),
                "Offer PCBA and box build services to the procurement team and submit a qualification package immediately [1].".to_string(),
                false,
            ),
        }
}

#[cfg(feature = "llm")]
fn matrix_case_passes(category: &str, public_sector: bool, quality: &str) -> bool {
    let entity_ctx = matrix_entity_context(public_sector);
    let evidence_signals = matrix_signal(category, public_sector);
    let (headline, narrative, recommendation, _) = matrix_payload(category, public_sector, quality);

    let headline_lower = headline.to_lowercase();
    let narrative_lower = narrative.to_lowercase();
    let recommendation_lower = recommendation.to_lowercase();
    let generic_phrases = [
        "continue monitoring",
        "monitor the situation",
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
    let placeholder_patterns = [
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

    let is_generic =
        generic_phrases.iter().any(|phrase| {
            headline_lower.contains(phrase)
                || narrative_lower.contains(phrase)
                || recommendation_lower.contains(phrase)
        }) || has_formulaic_commercial_language(&headline, &narrative, &recommendation);
    let malformed = malformed_fragments.iter().any(|fragment| {
        headline.to_ascii_lowercase().contains(fragment)
            || narrative_lower.contains(fragment)
            || recommendation_lower.contains(fragment)
    });
    let words = narrative.split_whitespace().count();
    let reference_count = count_numbered_references(&narrative, 12);
    let recommendation_reference_count = count_numbered_references(&recommendation, 12);
    let has_digits = narrative
        .chars()
        .any(|character| character.is_ascii_digit());
    let _has_recommendation = recommendation.split_whitespace().count() >= 12;
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
    .any(|phrase| narrative_lower.contains(phrase));
    let has_counterfactual = narrative_lower.contains("if ")
        && (narrative_lower.contains(" would ") || narrative_lower.contains(" could "));
    let recommendation_has_deadline = recommendation_has_action_timing(category, &recommendation);
    let readable_narrative = is_readable_and_useful_digest_text(&narrative)
        && !has_excessive_phrase_repetition(&recommendation);
    let readable_recommendation = recommendation
        .split(['.', ';'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .count()
        >= 1;
    let unsupported_security_escalation = has_unsupported_security_escalation(
        category,
        &narrative,
        &recommendation,
        &evidence_signals,
    );
    let unsupported_certification_escalation =
        has_unsupported_certification_escalation(&narrative, &recommendation, &evidence_signals);
    let unsupported_certification_commercialization =
        has_unsupported_certification_commercialization(
            &headline,
            &narrative,
            &recommendation,
            &evidence_signals,
        );
    let unsupported_public_sector_commercialization =
        has_unsupported_public_sector_commercialization(
            &entity_ctx,
            category,
            &narrative,
            &recommendation,
            &evidence_signals,
        );
    let low_usefulness_public_sector_analysis = has_low_usefulness_public_sector_analysis(
        &entity_ctx,
        category,
        &headline,
        &narrative,
        &recommendation,
        &evidence_signals,
    );
    let topic_alignment_violation = violates_entity_topic_alignment(
        &entity_ctx,
        &evidence_signals,
        &headline,
        &narrative,
        &recommendation,
    );
    let unnamed_customer_targeting = has_unnamed_customer_targeting(&narrative, &recommendation);
    let has_placeholders = placeholder_patterns
        .iter()
        .any(|pattern| recommendation_lower.contains(pattern));

    let decisions = vec![
        quality_gate_blocker("generic_language", is_generic, false),
        quality_gate_blocker("malformed_output", malformed, false),
        quality_gate_blocker("placeholder_output", has_placeholders, false),
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
            if has_causal_language || has_counterfactual {
                1.0
            } else {
                0.0
            },
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
            false,
        ),
        quality_gate_blocker(
            "public_sector_low_usefulness",
            low_usefulness_public_sector_analysis,
            true,
        ),
        quality_gate_blocker(
            "unnamed_customer_targeting",
            unnamed_customer_targeting,
            false,
        ),
        quality_gate_blocker("topic_alignment", topic_alignment_violation, true),
        quality_gate_requirement(
            "headline_length",
            if is_headline_ok { 1.0 } else { 0.0 },
            0.5,
        ),
    ];

    quality_gate_passes_ensemble(&decisions)
}

#[cfg(feature = "llm")]
#[test]
fn causal_ordering_validation() {
    let evidence_signals = vec![
        EvidenceSignal {
            title: "Patent filed".to_string(),
            description: "Patent filed for the new platform.".to_string(),
            source_url: "https://example.com/patent".to_string(),
            signal_type: "patent_filed".to_string(),
            extracted_facts: vec!["Patent filed on 2026-03-01".to_string()],
            date_context: Some("2026-03-01".to_string()),
            relevance_score: 1.0,
        },
        EvidenceSignal {
            title: "Contract awarded".to_string(),
            description: "Contract awarded after the filing window closed.".to_string(),
            source_url: "https://example.com/contract".to_string(),
            signal_type: "contract_awarded".to_string(),
            extracted_facts: vec!["Contract awarded on 2026-03-15".to_string()],
            date_context: Some("2026-03-15".to_string()),
            relevance_score: 1.0,
        },
    ];

    assert!(has_temporal_incoherence(
        "entity-1",
        "The contract awarded before patent filed sequence accelerated customer demand.",
        Utc::now(),
        &evidence_signals,
    ));
}

#[test]
fn shared_quality_gate_accepts_natural_veracity_summary() {
    let title = "NVIDIA: likely gpu and procurement development";
    let summary = "NVIDIA is appearing in 4 recent reports across 3 independent sources. The reported development centers on gpu and procurement activity, and source coverage is being compared for corroboration. The current read is likely because reported hiring and supply-chain evidence are showing up across multiple articles. If this matters commercially, keep it on an active watchlist while stronger confirmation arrives.";

    assert!(passes_shared_insight_quality_gate(
        title,
        summary,
        Some("veracity_analysis")
    ));
}

#[cfg(feature = "llm")]
#[test]
fn end_to_end_gate_matrix_is_deterministic_across_30_payloads() {
    let categories = [
        "demand_procurement",
        "supply_chain_risk",
        "security_compliance",
        "competitor_market",
        "geopolitical_analysis",
    ];
    let qualities = ["good", "borderline", "bad"];

    let mut expected = Vec::new();
    for public_sector in [false, true] {
        for category in categories {
            for quality in qualities {
                let pass = matrix_case_passes(category, public_sector, quality);
                expected.push((category, public_sector, quality, pass));
            }
        }
    }

    assert_eq!(expected.len(), 30);

    for _ in 0..3 {
        for (category, public_sector, quality, pass) in &expected {
            assert_eq!(
                    matrix_case_passes(category, *public_sector, quality),
                    *pass,
                    "matrix case changed for category={category}, public_sector={public_sector}, quality={quality}"
                );
        }
    }

    let pass_count = expected.iter().filter(|(_, _, _, pass)| *pass).count();
    let reject_count = expected.len() - pass_count;
    assert!(pass_count > 0);
    assert!(reject_count > 0);
}

#[cfg(feature = "llm")]
#[test]
fn adversarial_evolution_five_levels_are_caught_for_high_risk_gates() {
    let certification_evidence = vec![
            EvidenceSignal {
                title: "Kimball certification warning".to_string(),
                description: "Certification warning detected on the capabilities page, with only a generic compliance notice and no named customer or program impact.".to_string(),
                source_url: "https://example.com/kimball-warning".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "Certification warning detected".to_string(),
                    "Capabilities page update".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Kimball certifications".to_string(),
                description: "Observed certifications include ISO 13485 and IATF 16949.".to_string(),
                source_url: "https://example.com/kimball-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec!["ISO 13485".to_string(), "IATF 16949".to_string()],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.9,
            },
        ];
    let security_evidence = vec![
        EvidenceSignal {
            title: "El Sewedy DNS posture degradation".to_string(),
            description:
                "DNS posture score at 70 with missing DKIM record and elevated spoofing risk."
                    .to_string(),
            source_url: "https://example.com/dns".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "30% DNS posture score deficiency".to_string(),
                "Missing DKIM record".to_string(),
            ],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        },
        EvidenceSignal {
            title: "El Sewedy certifications".to_string(),
            description: "Observed certification scope includes ISO 9001 only.".to_string(),
            source_url: "https://example.com/certs".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec!["ISO 9001".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 0.9,
        },
    ];

    let certification_variants = [
            (
                "The certification warning means medical device buyers may switch suppliers because qualification confidence is weakening.",
                "Target their medical device customers with a switch campaign by Q2 2026 [1].",
            ),
            (
                "The regulatory concern is contributing to sourcing reviews and giving rise to alternative-supplier screening.",
                "Pursue their account base before the next sourcing cycle with a continuity offer [1].",
            ),
            (
                "If the standards lapse persists, buyers would widen qualification reviews and look for safer alternatives.",
                "Focus on downstream OEM accounts with a qualification-led message this quarter [1].",
            ),
            (
                "The accreditation gap is precipitating a compliance shortfall narrative across regulated programs.",
                "Work the customer roster with a dual-source pitch before renewal [1].",
            ),
            (
                "La certificati\u{301}on shortfall is giving rise to supplier switches in regulated accounts.",
                "Go after their buyer base with a near-term switch campaign [1].",
            ),
        ];
    for (narrative, recommendation) in certification_variants {
        assert!(
            has_unsupported_certification_escalation(
                narrative,
                recommendation,
                &certification_evidence,
            ),
            "certification variant slipped: {narrative} || {recommendation}"
        );
    }

    let security_variants = [
            (
                "The DNS posture weakness means customers could face qualification delays because trust controls are eroding.",
                "Target their medical device customers for a switch campaign by Q2 2026 [1].",
            ),
            (
                "The email authentication gap is resulting in customer concern and contributing to supplier reviews.",
                "Pursue their account base with an assurance-led switch pitch this quarter [1].",
            ),
            (
                "If domain spoofing exposure persists, buyers would widen sourcing reviews and seek safer alternatives.",
                "Focus on downstream OEM accounts before the next sourcing cycle [1].",
            ),
            (
                "Identity surface exposure is precipitating vendor-assurance scrutiny across regulated programs.",
                "Work the customer roster with a trust-and-continuity offer before renewal [1].",
            ),
            (
                "Spoof\u{131}ng susceptibility is giving rise to supplier switches in sensitive programs.",
                "Go after their buyer base with a near-term continuity campaign [1].",
            ),
        ];
    for (narrative, recommendation) in security_variants {
        assert!(
            has_unsupported_security_escalation(
                "security_compliance",
                narrative,
                recommendation,
                &security_evidence,
            ),
            "security variant slipped: {narrative} || {recommendation}"
        );
    }

    let targeting_variants = [
        "Target their medical device customers for a switch campaign by Q2 2026 [1].",
        "Pursue their account base before the next sourcing cycle with a continuity offer [1].",
        "If delays continue, work the customer roster with a dual-source message this quarter [1].",
        "Focus on downstream OEM accounts ahead of renewal with an assurance-led pitch [1].",
        "Go after their buyer base with a near-term switch campaign [1].",
    ];
    for recommendation in targeting_variants {
        assert!(
            has_unnamed_customer_targeting("Generic risk narrative.", recommendation,),
            "targeting variant slipped: {recommendation}"
        );
    }
}

#[test]
fn shared_quality_gate_rejects_template_markers() {
    let title = "Intelligence Veracity: NVIDIA";
    let summary = "Additional source reporting: source one. Signal themes detected: gpu, procurement. Assessment: MODERATE-HIGH CONFIDENCE. Actionable: move now.";

    assert!(!passes_shared_insight_quality_gate(
        title,
        summary,
        Some("veracity_analysis")
    ));
}

#[test]
fn shared_quality_gate_allows_internal_llm_reports() {
    assert!(passes_shared_insight_quality_gate(
        "LLM Eval Gate Report",
        "Assessment: internal evaluator output is intentionally structured for debugging.",
        Some("llm_eval_report")
    ));
}

#[test]
fn generic_action_hints_are_not_treated_as_concrete_lanes() {
    assert!(is_generic_action_hint("Monitor sentiment trajectory"));
    assert!(is_generic_action_hint("Identify topic drivers"));
    assert!(is_generic_action_hint("Investigate partner sentiment"));
    assert!(is_generic_action_hint("Explore demand drivers"));
    assert!(is_generic_action_hint(
        "Convene cross-functional risk assessment"
    ));
    assert!(is_generic_action_hint(
        "Scenario-plan for operational disruption"
    ));
    assert!(is_generic_action_hint(
        "Document and classify suspicious domains"
    ));
    assert!(is_generic_action_hint("Initiate takedown procedures"));
    assert!(!is_generic_action_hint(
        "Review DG GROW supplier notice from 2026-03-07"
    ));
    assert!(!is_generic_action_hint(
        "Initiate takedown for tunisia-gov-alert.com"
    ));
}

#[test]
fn token_jaccard_similarity_ignores_duplicate_tokens() {
    let left = vec!["nvidia".to_string(), "gpu".to_string(), "gpu".to_string()];
    let right = vec![
        "nvidia".to_string(),
        "supply".to_string(),
        "gpu".to_string(),
    ];

    let similarity = token_jaccard_similarity(&left, &right);

    assert!((similarity - (2.0 / 3.0)).abs() < f64::EPSILON);
}

#[test]
fn env_flag_uses_shared_truthy_parser() {
    std::env::set_var("APEX_TEST_ENV_FLAG", " yes ");

    assert!(env_flag("APEX_TEST_ENV_FLAG"));

    std::env::remove_var("APEX_TEST_ENV_FLAG");
}

#[test]
fn truncate_text_uses_shared_utf8_boundary_logic() {
    assert_eq!(truncate_text("hello 😀", 7), "hello ");
}

#[test]
fn aggregate_count_signal_details_are_not_treated_as_concrete_evidence() {
    let signal_details = vec![
        "26 job posting(s) observed".to_string(),
        "2 patent(s) filed".to_string(),
        "16 certification(s) on record".to_string(),
        "1 competitor signal(s)".to_string(),
        "45 web change(s) detected".to_string(),
    ];

    assert_eq!(count_concrete_signal_details(&signal_details), 0);
    assert!(!should_emit_fallback_insight(
        "security_compliance",
        &signal_details,
        "Monitor sentiment trajectory; Assess customer impact",
        0.48,
        2,
    ));
    assert!(!should_emit_fallback_insight(
        "security_compliance",
        &signal_details,
        "Convene cross-functional risk assessment; Scenario-plan for operational disruption",
        0.82,
        4,
    ));
}

#[test]
fn shared_quality_gate_rejects_generic_count_only_fallback_summary() {
    let summary = "Arrow Electronics (North America) has been flagged for security and compliance concerns. Our monitoring detected: 26 job posting(s) observed; 2 patent(s) filed; 16 certification(s) on record; 1 competitor signal(s); 45 web change(s) detected. If the priority is assurance, gather the evidence that would reassure auditors, customers, and procurement teams that Arrow Electronics still clears the relevant security or quality gate. This deserves action inside the current planning cycle rather than being left as passive watch-list material. Overall confidence is preliminary (48%), grounded in 2 supporting signal(s).";

    assert!(!passes_shared_insight_quality_gate(
        "Arrow Electronics: security and compliance concerns",
        summary,
        Some("security_compliance"),
    ));
}

#[test]
fn shared_quality_gate_rejects_public_sector_single_count_fallback_summary() {
    let summary = "Government of Czech Republic (Europe) has been flagged for geopolitical developments affecting operations. Our monitoring detected: 1 web change(s) detected. Policy and trade exposure can quickly make footprint, routing, and export eligibility more decisive than nominal unit cost, especially where EU and North Africa positioning changes the compliance or resilience story. Immediate next step: Convene cross-functional risk assessment. Scenario-plan for operational disruption.";

    assert!(!passes_shared_insight_quality_gate(
        "Government of Czech Republic: geopolitical developments affecting operations",
        summary,
        Some("geopolitical_analysis"),
    ));
}

#[test]
fn shared_quality_gate_allows_richer_promoted_business_summary() {
    let summary = "A named tariff review now affects qualification timing for a defense-adjacent account. Because the supplier notice explicitly changes approval sequencing, the customer could compress sourcing decisions this quarter. The evidence ties the shift to export routing and tender preparation, which creates a nearshoring opportunity for a Morocco or Tunisia production path. The account team should map exposed bids and stakeholder owners before the next sourcing cycle. Procurement should align qualification evidence and pricing for the impacted program this quarter. If the review widens, customer communication would need to move faster, and if it narrows, the same qualification work would still improve bid speed.";

    assert!(passes_shared_insight_quality_gate(
        "European defense supplier faces tariff-driven qualification squeeze",
        summary,
        Some("regulatory_policy"),
    ));
}

#[test]
fn shared_quality_gate_allows_cited_multi_lane_promoted_business_summary() {
    let summary = "Key Tronic Corporation's recent supply chain risk flags and cybersecurity threats [6][7] create a critical vulnerability for its customers, particularly in defense-adjacent and medical sectors where compliance and data security are paramount. The company's 2026 filings show increased focus on risk mitigation [6], but the detected cybersecurity weaknesses [7] suggest potential gaps in protecting sensitive manufacturing data. This positions our ISO 9001 and AS9100-certified North African and EU manufacturing footprint as a direct alternative for clients needing secure, tariff-free production. If Key Tronic's compliance issues worsen, its defense and medical customers could face qualification delays, making our nearshore capabilities a strategic replacement. Approach ABB Ltd. [4] before the next sourcing cycle to position secure, qualified capacity against Key Tronic's risk profile. Contact Airbus Defence & Space [6] this quarter with a defense-program continuity plan tied to qualification speed and data-security controls.";

    assert!(passes_shared_insight_quality_gate(
        "Key Tronic's supply chain risks open door for EMS switch",
        summary,
        Some("demand_procurement"),
    ));
}

#[test]
fn nonsecurity_titles_skip_aggregate_counts_and_legacy_phrases() {
    let title = build_analytical_title(
        "Government of Netherlands",
        "Europe",
        "geopolitical_analysis",
        &["86 web change(s) detected".to_string()],
        Some("Government"),
    );

    assert!(title.contains("policy and trade exposure shift"));
    assert!(!title.contains("diplomatic development flagged"));
    assert!(!title.contains("86 web change(s) detected"));
}

#[test]
fn customer_rfq_titles_use_emerging_procurement_language() {
    let title = build_analytical_title(
        "Jabil",
        "North America",
        "customer_rfq",
        &["18 job posting(s) observed".to_string()],
        Some("Company"),
    );

    assert!(title.contains("procurement signal emerging"));
    assert!(!title.contains("RFQ or customer engagement activity"));
    assert!(!title.contains("18 job posting(s) observed"));
}

#[test]
fn business_fallback_summary_passes_gate_and_ignores_security_hygiene_noise() {
    let signal_details = vec![
        "8 lookalike domain(s) detected".to_string(),
        "Named RFQ issued for avionics subassembly".to_string(),
        "Supplier portal opened for Q3 RFQ".to_string(),
    ];

    let title = build_analytical_title(
        "Acme EMS",
        "Europe",
        "demand_procurement",
        &signal_details,
        Some("EMS"),
    );
    let analytical = build_analytical_narrative(
        "Acme EMS",
        "Europe",
        Some("EMS"),
        "demand_procurement",
        &signal_details,
        0.78,
        &[],
    );
    let summary = build_fallback_summary(FallbackSummaryRequest {
        analytical_narrative: &analytical,
        rendered_action: "",
        signal_details: &signal_details,
        evidence_urls: &[],
        entity_label: "Acme EMS",
        entity_region: "Europe",
        entity_type: Some("EMS"),
        category: "demand_procurement",
    });

    assert!(
        title.contains("Named RFQ issued for avionics subassembly")
            || title.contains("Supplier portal opened for Q3 RFQ")
    );
    assert!(!title.contains("lookalike domain"));
    assert!(!summary
        .to_ascii_lowercase()
        .contains("has been flagged for"));
    assert!(!summary
        .to_ascii_lowercase()
        .contains("our monitoring detected:"));
    assert!(
        passes_shared_insight_quality_gate(&title, &summary, Some("demand_procurement"),),
        "title={title}\nsummary={summary}"
    );
}

#[test]
fn public_sector_brand_sentiment_avoids_commercial_template_language() {
    let summary = build_fallback_summary(FallbackSummaryRequest {
            analytical_narrative: "European Commission (Europe) has been flagged for brand sentiment and reputation signals.",
            rendered_action: "Monitor sentiment trajectory; Identify topic drivers; Assess customer impact",
            signal_details: &["Press coverage increased after an EU policy dispute".to_string()],
            evidence_urls: &[],
            entity_label: "European Commission",
            entity_region: "Europe",
            entity_type: Some("Government"),
            category: "brand_sentiment",
        });

    let lower = summary.to_ascii_lowercase();
    assert!(!lower.contains("pipeline capture"));
    assert!(!lower.contains("commercial upside"));
    assert!(!lower.contains("one concrete lane worth testing is this"));
    assert!(
        lower.contains("tender scrutiny")
            || lower.contains("oversight")
            || lower.contains("approval timing")
    );
}

#[test]
fn public_sector_security_fallback_uses_specific_artifacts_not_generic_playbook() {
    let summary = build_fallback_summary(FallbackSummaryRequest {
            analytical_narrative: "Government of Tunisia (MENA) has been flagged for cybersecurity threats and vulnerabilities.",
            rendered_action: "Document and classify suspicious domains; Initiate takedown procedures",
            signal_details: &[
                "3 lookalike domain(s) detected".to_string(),
                "DNS posture degradation detected".to_string(),
            ],
            evidence_urls: &[],
            entity_label: "Government of Tunisia",
            entity_region: "MENA",
            entity_type: Some("Government"),
            category: "cybersecurity_threat",
        });

    let lower = summary.to_ascii_lowercase();
    assert!(lower.contains(
        "key evidence: 3 lookalike domain(s) detected; dns posture degradation detected"
    ));
    assert!(
        lower.contains("official domains")
            || lower.contains("procurement portals")
            || lower.contains("citizen-facing services")
    );
    assert!(!lower.contains("vendor eligibility"));
    assert!(!lower.contains("customer audits"));
    assert!(!lower.contains("immediate next step:"));
}

#[test]
fn public_sector_geopolitical_fallback_avoids_generic_supply_chain_actions() {
    let summary = build_fallback_summary(FallbackSummaryRequest {
            analytical_narrative: "European Commission (Europe) has been flagged for geopolitical developments affecting operations.",
            rendered_action: "Adjust supply chain monitoring priorities; Review partner exposure",
            signal_details: &["European Commission published an updated trade defense consultation notice".to_string()],
            evidence_urls: &[],
            entity_label: "European Commission",
            entity_region: "Europe",
            entity_type: Some("Government"),
            category: "geopolitical_analysis",
        });

    let lower = summary.to_ascii_lowercase();
    assert!(!lower.contains("immediate next step:"));
    assert!(!lower.contains("adjust supply chain monitoring priorities"));
    assert!(
        lower.contains("official statement")
            || lower.contains("tender amendment")
            || lower.contains("oversight action")
    );
}

#[test]
fn fallback_summary_appends_numbered_sources_footer() {
    let summary = build_fallback_summary(FallbackSummaryRequest {
            analytical_narrative: "European Commission (Europe) has been flagged for geopolitical developments affecting operations.",
            rendered_action: "Review consultation scope",
            signal_details: &["European Commission published an updated trade defense consultation notice".to_string()],
            evidence_urls: &[
                "https://ec.europa.eu/commission/presscorner/detail/en/ip_26_1234".to_string(),
                "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            ],
            entity_label: "European Commission",
            entity_region: "Europe",
            entity_type: Some("Government"),
            category: "geopolitical_analysis",
        });

    assert!(summary.contains("Sources:"));
    assert!(summary.contains(
        "[1] ec.europa.eu — https://ec.europa.eu/commission/presscorner/detail/en/ip_26_1234"
    ));
    assert!(summary
        .contains("[2] trade.ec.europa.eu — https://trade.ec.europa.eu/doclib/notice-2026-03-07"));
}

#[cfg(feature = "llm")]
#[test]
fn low_signal_security_hygiene_rejects_overstated_program_impact() {
    let evidence_signals = vec![
        EvidenceSignal {
            title: "El Sewedy DNS posture degradation".to_string(),
            description:
                "DNS posture score at 70 with missing DKIM record and elevated spoofing risk."
                    .to_string(),
            source_url: "https://example.com/dns".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "30% DNS posture score deficiency".to_string(),
                "Missing DKIM record".to_string(),
            ],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        },
        EvidenceSignal {
            title: "El Sewedy certifications".to_string(),
            description: "Observed certification scope includes ISO 9001 only.".to_string(),
            source_url: "https://example.com/certs".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec!["ISO 9001".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 0.9,
        },
    ];

    let narrative = "El Sewedy's DNS posture and missing DKIM undermine its ability to qualify for EU defense programs and medical device manufacturing because customers could face compliance delays.";
    let recommendation = "Approach named customers and position for supply chain disruption response by Q2 2026 [1].";

    assert!(has_unsupported_security_escalation(
        "security_compliance",
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn low_signal_certification_warning_rejects_switching_claims() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Kimball Electronics certification warning".to_string(),
                description: "Certification warning detected on the capabilities page, with only a generic compliance notice and no named customer or program impact.".to_string(),
                source_url: "https://example.com/kimball-warning".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "Certification warning detected".to_string(),
                    "Capabilities page update".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Kimball certifications".to_string(),
                description: "Observed certifications include ISO 13485 and IATF 16949.".to_string(),
                source_url: "https://example.com/kimball-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec!["ISO 13485".to_string(), "IATF 16949".to_string()],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.9,
            },
        ];

    let narrative = "Kimball's certification warning means medical device customers could face qualification delays and may switch suppliers because the company appears to be struggling with compliance.";
    let recommendation = "Target Kimball's medical and automotive customers for a nearshore switch campaign by Q2 2026 [1].";

    assert!(has_unsupported_certification_escalation(
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn mixed_supply_chain_and_certification_evidence_is_not_treated_as_low_signal_cert_warning() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic tariff pressure".to_string(),
                description: "New tariff exposure is increasing supply-chain costs for medical and industrial programs.".to_string(),
                source_url: "https://example.com/keytronic-tariffs".to_string(),
                signal_type: "supply_chain_risk".to_string(),
                extracted_facts: vec![
                    "Tariff exposure raised landed costs".to_string(),
                    "Medical device programs face margin pressure".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows an updated ISO 13485 certification notice.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 13485 certification updated".to_string(),
                    "Capabilities page refreshed".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.8,
            },
        ];

    let narrative = "Key Tronic's tariff pressure creates a dual-source opening for Medtronic because rising landed costs threaten continuity on regulated device programs.";
    let recommendation = "Approach Medtronic's sourcing team this quarter with a continuity-focused EMS proposal tied to Key Tronic's tariff exposure [1].";

    assert!(!low_signal_certification_warning_case(&evidence_signals));
    assert!(!has_unsupported_certification_escalation(
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn mixed_evidence_still_rejects_soft_certification_switch_claims() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic cyber weaknesses".to_string(),
                description: "Cybersecurity threats and weak controls were observed on a public-facing surface.".to_string(),
                source_url: "https://example.com/keytronic-cyber".to_string(),
                signal_type: "security_compliance".to_string(),
                extracted_facts: vec![
                    "Cybersecurity weaknesses detected".to_string(),
                    "Public-facing controls need remediation".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows an updated ISO 13485 certification notice and generic compliance warning.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 13485 certification updated".to_string(),
                    "Generic compliance warning detected".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 0.8,
            },
        ];

    let narrative = "Key Tronic's compliance risks and cybersecurity threats create a vulnerability for defense-adjacent and medical customers because qualification confidence could weaken.";
    let recommendation = "Approach ABB Ltd. before the next sourcing cycle to position qualified capacity if Key Tronic's compliance issues worsen [1].";

    assert!(!low_signal_certification_warning_case(&evidence_signals));
    assert!(has_unsupported_certification_escalation(
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn mixed_evidence_rejects_flagged_certifications_that_create_switch_risk() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic cyber weaknesses".to_string(),
                description: "Cybersecurity threats and weak controls were observed on a public-facing surface.".to_string(),
                source_url: "https://example.com/keytronic-cyber".to_string(),
                signal_type: "security_compliance".to_string(),
                extracted_facts: vec![
                    "Cybersecurity weaknesses detected".to_string(),
                    "Public-facing controls need remediation".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows flagged ISO 13485 and AS9100 certifications with a generic compliance notice.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "Flagged ISO 13485 certification".to_string(),
                    "Flagged AS9100 certification".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 0.8,
            },
        ];

    let narrative = "Key Tronic Corporation's compliance gaps, highlighted by flagged ISO 13485 and AS9100 certifications, create immediate risks for its defense-adjacent and medical clients.";
    let recommendation = "Approach ABB Ltd. by Q2 2026 by highlighting our AS9100-certified capacity as a safer alternative if Key Tronic's certification risks deepen [1].";

    assert!(has_unsupported_certification_escalation(
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn soft_certification_opportunity_headline_is_rejected() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Celestica certifications page updated".to_string(),
                description: "The capabilities page shows ISO 9001 and ISO 13485 certifications with a generic renewal notice and no explicit downstream impact.".to_string(),
                source_url: "https://example.com/celestica-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 9001 certification renewed".to_string(),
                    "ISO 13485 certification renewed".to_string(),
                    "Generic renewal notice posted".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.9,
            },
            EvidenceSignal {
                title: "Celestica compliance notice".to_string(),
                description: "A generic compliance update was detected without named audit action or downstream qualification changes.".to_string(),
                source_url: "https://example.com/celestica-compliance".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec!["Generic compliance update".to_string()],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.8,
            },
        ];

    assert!(has_unsupported_certification_commercialization(
            "Celestica's ISO 9001/13485 Certifications Expire in 2026 - Target Nearshoring Opportunities",
            "Celestica's certification timeline suggests regulated buyers may revisit confidence, creating a strategic outreach opportunity for our North Africa footprint.",
            "Target nearshoring opportunities with EU industrial customers this quarter and position our services as a safer manufacturing option [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn live_brand_sentiment_formulaic_pitch_is_rejected() {
    assert!(has_formulaic_commercial_language(
            "Plexus Corp's AI API gateway expansion: Strategic outreach opportunities",
            "Plexus Corp's recent announcement of a unified API gateway for multiple AI providers [1] signals a strategic pivot toward AI-integrated electronics solutions. This development, coupled with their existing aerospace and defense industry focus [3], creates a commercial opening for targeted EMS partnerships. The AI gateway initiative likely requires robust hardware integration capabilities, which aligns with our ISO 9001 and AS9100-certified facilities in North Africa and Europe [3].",
            "Reach out to Plexus's procurement team by April 5, 2026, emphasizing our defense-qualified facilities and AI hardware integration experience [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn formulaic_headline_is_rejected_even_when_body_is_concrete() {
    assert!(has_formulaic_commercial_language(
            "Government of India's NDC 3.0 strategy presents climate-tech qualification opportunities",
            "The March 2026 policy update [1] names a concrete planning artifact, and the current evidence still needs procurement-path verification before any commercial move.",
            "Brief the account team this week on the named policy update and confirm whether a tender or qualification pathway is actually present before proposing outreach [1].",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn live_flex_nearshore_ems_pitch_is_rejected() {
    assert!(has_formulaic_commercial_language(
            "Flex Ltd faces nearshoring pressure; nearshore EMS opportunities emerge",
            "Flex Ltd's compliance gaps and procurement risks create nearshore EMS opportunities. This matters because our ISO 13485 and AS9100-certified facilities in Tunisia and Morocco position us to target Flex's medical and aerospace clients if those qualification concerns widen.",
            "",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn live_benchmark_competitor_targeting_pitch_is_rejected() {
    assert!(has_formulaic_commercial_language(
            "Benchmark Electronics' DNS risks and R&D activity signal customer vulnerabilities",
            "Benchmark Electronics faces heightened supply chain risks due to its insecure DNS posture, and this dual vulnerability creates opportunities for competitors to target Benchmark's aerospace clients, particularly those with strict compliance requirements.",
            "",
        ));
}

#[cfg(feature = "llm")]
#[test]
fn soft_certification_nearshore_manufacturing_pitch_is_rejected() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Venture certifications profile".to_string(),
                description: "Venture's public certifications page lists ISO standards and a generic compliance status update with no explicit qualification disruption.".to_string(),
                source_url: "https://example.com/venture-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO certification listed".to_string(),
                    "Generic compliance status update".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.9,
            },
        ];

    assert!(has_unsupported_certification_commercialization(
            "Venture Corporation's Asia-Pacific EMS network offers nearshore manufacturing opportunities",
            "The certification posture can be framed as a manufacturing opportunity because some buyers may want more resilient options.",
            "Offer nearshore manufacturing to industrial customer opportunities before the next quarter [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn hard_certification_failure_does_not_trigger_commercialization_gate() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Supplier removed after failed audit".to_string(),
                description: "The supplier was removed from an approved vendor list after a failed audit and certificate suspension affecting a named medical device program.".to_string(),
                source_url: "https://example.com/audit-failure".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "Failed audit".to_string(),
                    "Certificate suspended".to_string(),
                    "Supplier removed from approved list".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 1.0,
            },
        ];

    assert!(!has_unsupported_certification_commercialization(
            "Supplier audit failure disrupts named medical program",
            "The failed audit and supplier removal create a concrete qualification gap for the named device program because procurement now has to re-source an approved build path.",
            "Meet the program sourcing lead this week with an approved alternative package tied to the removed supplier event [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn live_jabil_certification_client_targeting_pitch_is_rejected_with_sparse_cert_evidence() {
    let evidence_signals = vec![EvidenceSignal {
            title: "Jabil medical certifications page updated".to_string(),
            description: "Jabil refreshed its medical manufacturing certifications page and capabilities overview with a generic renewal update and no named customer disruption.".to_string(),
            source_url: "https://example.com/jabil-iso13485".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "ISO 13485 certification listed".to_string(),
                "medical manufacturing capabilities page updated".to_string(),
                "generic renewal update".to_string(),
            ],
            date_context: Some("2026-04-12".to_string()),
            relevance_score: 0.89,
        }];

    assert!(has_unsupported_certification_commercialization(
            "Jabil's ISO 13485 gaps threaten medical device clients",
            "Jabil's ISO 13485 certification for medical device manufacturing is nearing expiration on 2026-08-31, creating urgent risks for its healthcare clients. This certification gap directly impacts clients like Medtronic and Boston Scientific, who rely on Jabil's verified SMT and additive manufacturing services. Our ISO 13485-certified SMT and additive manufacturing services offer a direct alternative, with proven capabilities in 01005 SMT and EU-compliant supply chains.",
            "Approach Medtronic with our ISO 13485-certified SMT services by 2026-08-31. Engage Boston Scientific with our 01005 SMT expertise to mitigate potential delays. Target Siemens Healthineers with our EU-compliant supply chain management by 2026-08-31.",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn live_blue_solutions_nearshore_pitch_is_rejected_with_sparse_cert_evidence() {
    let evidence_signals = vec![EvidenceSignal {
            title: "Blue Solutions IATF certificate page updated".to_string(),
            description: "Blue Solutions published an IATF 16949 certificate status page and generic compliance documentation with a renewal timeline and no named customer disruption.".to_string(),
            source_url: "https://example.com/blue-solutions-iatf".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "IATF 16949 certificate listed".to_string(),
                "generic compliance documentation updated".to_string(),
                "renewal timeline".to_string(),
            ],
            date_context: Some("2026-04-12".to_string()),
            relevance_score: 0.88,
        }];

    assert!(has_unsupported_certification_commercialization(
            "Blue Solutions' IATF gaps threaten automotive clients",
            "Blue Solutions' IATF 16949 certification gaps create immediate risks for automotive clients relying on their compliance. If Blue's compliance issues persist, automotive clients could switch to nearshore EMS providers with verified EU compliance. This creates a window to approach AutoTech Systems' procurement lead with our rapid qualification process before their April 2026 deadline.",
            "Approach AutoTech Systems' procurement lead before April 2026 with our IATF 16949-certified facilities. Engage EuroMotive Motors' supply chain manager with our secure, audit-ready manufacturing. Target DigiDrive Automotive's engineering team with our aerospace-qualified facilities in Tunisia.",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn low_signal_security_hygiene_displacement_is_rejected_outside_security_categories() {
    let evidence_signals = vec![EvidenceSignal {
            title: "GPV Group lookalike domains detected".to_string(),
            description: "Monitoring found multiple lookalike domains and missing DMARC enforcement alongside routine email-authentication hygiene drift.".to_string(),
            source_url: "https://example.com/gpv-lookalikes".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "lookalike domains".to_string(),
                "missing DMARC enforcement".to_string(),
                "email authentication gap".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.92,
        }];

    assert!(has_unsupported_security_escalation(
            "quality_compliance",
            "GPV Group's lookalike domains and email authentication gap now create customer risk because affected buyers may view the supplier as a weaker option.",
            "Offer a direct alternative to industrial customers this quarter before they switch providers [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn negated_hard_failure_security_markers_still_count_as_low_signal_hygiene() {
    let evidence_signals = vec![EvidenceSignal {
            title: "Mouser lookalike domains detected".to_string(),
            description: "Monitoring found multiple lookalike domains with no confirmed compromise or supplier removal, only routine email-authentication hygiene drift.".to_string(),
            source_url: "https://example.com/mouser-lookalikes".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "lookalike domains".to_string(),
                "missing DMARC enforcement".to_string(),
                "routine email-authentication hygiene drift".to_string(),
            ],
            date_context: Some("2026-04-06".to_string()),
            relevance_score: 0.9,
        }];

    assert!(quality_gates::low_signal_security_hygiene_case(
        "cybersecurity_threat",
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn security_hygiene_procurement_portal_conversion_is_rejected_in_security_category() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "Mouser lookalike domains detected".to_string(),
                description: "Monitoring found eight lookalike domains around Mouser's supplier-facing infrastructure alongside routine email authentication hygiene gaps.".to_string(),
                source_url: "https://example.com/mouser-lookalikes".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "8 lookalike domains".to_string(),
                    "email authentication gap".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.95,
            },
            EvidenceSignal {
                title: "Mouser supplier portal notice".to_string(),
                description: "A supplier portal page references an onboarding process for distributor partners in March 2026.".to_string(),
                source_url: "https://example.com/mouser-supplier-portal".to_string(),
                signal_type: "procurement".to_string(),
                extracted_facts: vec![
                    "supplier portal onboarding".to_string(),
                    "March 2026 notice".to_string(),
                ],
                date_context: Some("2026-03-15".to_string()),
                relevance_score: 0.72,
            },
        ];

    assert!(has_unsupported_security_escalation(
            "cybersecurity_threat",
            "Mouser's lookalike domains suggest heightened fraud risk around supplier access and portal credentials [1].",
            "Register on Mouser's supplier portal within 30 days, prepare PPAP and IMDS documentation, and use the portal notice to accelerate qualification [2].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn soft_certification_deadline_bom_disruption_is_rejected() {
    let evidence_signals = vec![
            EvidenceSignal {
                title: "STMicroelectronics certification page updated".to_string(),
                description: "STMicroelectronics lists ISO 9001 on a public site with a generic validity window and no explicit disruption or supplier removal.".to_string(),
                source_url: "https://example.com/st-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 9001 listed".to_string(),
                    "valid until 2027".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.84,
            },
            EvidenceSignal {
                title: "STMicroelectronics R&D update".to_string(),
                description: "The company highlighted 300mm wafer work and SiC/FDSOI roadmap investment in Crolles with no direct customer impact described.".to_string(),
                source_url: "https://example.com/st-rd".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec![
                    "300mm wafer capabilities".to_string(),
                    "SiC roadmap".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.79,
            },
        ];

    assert!(has_unsupported_certification_escalation(
            "STMicroelectronics' ISO 9001 deadline could disrupt deliveries if compliance transitions slip, forcing urgent BOM revisions for automotive customers.",
            "Alert customers to begin alternate part qualification before the 2027 certification deadline creates delivery risk [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn soft_certification_marker_is_unicode_normalized() {
    assert!(contains_soft_certification_pressure_marker(
        "A certificati\u{301}on warning was posted after the latest compliance review."
    ));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_is_rejected() {
    let narrative = "Key Tronic's tariff and compliance warnings create a vulnerability for EU medical device customers because rising cross-border costs could pressure delivery commitments.";
    let recommendation =
        "Target their medical device customers for a nearshore switch campaign by Q2 2026 [1].";

    assert!(has_unnamed_customer_targeting(narrative, recommendation));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_ignores_generic_narrative_when_recommendation_is_specific() {
    let narrative = "Key Tronic's recent supply chain instability creates risk for their medical customers and industrial buyers.";
    let recommendation =
            "Approach Medtronic's sourcing team this quarter with a dual-source continuity pitch tied to Key Tronic's supply-chain volatility [1].";

    assert!(!has_unnamed_customer_targeting(narrative, recommendation));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_detects_unicode_and_downstream_account_language() {
    assert!(has_unnamed_customer_targeting(
            "Tariff pressure is building across the account base.",
            "Ta\u{301}rget their medical device customers for a nearshore switch campaign by Q2 2026 [1]."
        ));
    assert!(has_unnamed_customer_targeting(
            "The supplier is under margin pressure.",
            "Pursue downstream OEM accounts before the next sourcing cycle with a continuity-led offer [1]."
        ));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_allows_named_internal_stakeholder_outreach() {
    let narrative = "Flex Ltd faces multiple supply chain risks that could disrupt automotive and industrial programs.";
    let recommendation = "Contact Flex Ltd's procurement lead Revathi Advaithi within 30 days to propose a capacity audit for their automotive and industrial programs [1].";

    assert!(!has_unnamed_customer_targeting(narrative, recommendation));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_in_narrative_is_rejected() {
    let narrative = "NOTE AB's lookalike domains suggest aggressive competitive outreach, and their aerospace clients may now be open to a switch.";
    let recommendation =
        "Prioritize a displacement plan before the next sourcing cycle closes [1].";

    assert!(has_unnamed_customer_targeting(narrative, recommendation));
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_rejects_generic_customer_cohorts() {
    assert!(has_unnamed_customer_targeting(
        "Digi-Key's Part-DB update may unsettle qualification timelines for regulated buyers.",
        "Act on aerospace and medical customers before the next sourcing cycle [1].",
    ));
    assert!(has_unnamed_customer_targeting(
        "Jabil's ISO 13485 renewal timeline could raise review questions.",
        "Approach their medical device clients with a direct alternative this quarter [1].",
    ));
}

#[cfg(feature = "llm")]
#[test]
fn named_target_provenance_rejects_hallucinated_downstream_company() {
    let entity_ctx = EntityContext {
        name: "Jabil".to_string(),
        region: "North America".to_string(),
        entity_type: Some("company".to_string()),
        is_competitor: false,
        industry_tags: vec!["electronics manufacturing services".to_string()],
        certifications: vec!["ISO 13485".to_string()],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec!["Flex".to_string()],
        sites_summary: vec!["St. Petersburg medical manufacturing campus".to_string()],
        competitor_events: vec![],
        domain: Some("jabil.com".to_string()),
    };
    let evidence_signals = vec![EvidenceSignal {
            title: "Jabil ISO 13485 page updated".to_string(),
            description: "Jabil updated its medical manufacturing certifications page with an ISO 13485 renewal note and no named customer impact.".to_string(),
            source_url: "https://example.com/jabil-iso13485".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "ISO 13485 renewal note".to_string(),
                "medical manufacturing campus".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.86,
        }];

    assert!(has_unsupported_named_target_provenance(
            &entity_ctx,
            "Jabil's ISO 13485 renewal may unsettle regulated sourcing reviews",
            "The renewal timing could create questions for regulated builds if buyers want a backup path [1].",
            "Approach MedTech Innovators Ltd. before Q3 sourcing reviews begin [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn named_target_provenance_allows_named_company_present_in_evidence() {
    let entity_ctx = EntityContext {
        name: "Digi-Key".to_string(),
        region: "North America".to_string(),
        entity_type: Some("distributor".to_string()),
        is_competitor: false,
        industry_tags: vec!["electronics distribution".to_string()],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("digikey.com".to_string()),
    };
    let evidence_signals = vec![EvidenceSignal {
            title: "Digi-Key names Acme Medical Systems in rollout update".to_string(),
            description: "The update names Acme Medical Systems as a launch customer for the new workflow and ties the rollout to regulated sourcing reviews.".to_string(),
            source_url: "https://example.com/partdb-acme".to_string(),
            signal_type: "product_update".to_string(),
            extracted_facts: vec![
                "Acme Medical Systems launch customer".to_string(),
                "regulated sourcing reviews".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.91,
        }];

    assert!(!has_unsupported_named_target_provenance(
            &entity_ctx,
            "Digi-Key rollout could affect regulated sourcing timing",
            "Because Acme Medical Systems is explicitly named in the rollout evidence, the account impact can be discussed directly [1].",
            "Approach Acme Medical Systems this quarter [1].",
            &evidence_signals,
        ));
}

#[cfg(feature = "llm")]
#[test]
fn strategic_recommendation_timing_accepts_this_quarter() {
    assert!(recommendation_has_action_timing(
        "demand_procurement",
        "Approach the procurement lead this quarter with a qualification-led EMS proposal [1]."
    ));
    assert!(!recommendation_has_action_timing(
        "demand_procurement",
        "Approach the procurement lead when possible with a qualification-led EMS proposal [1]."
    ));
}

#[cfg(feature = "llm")]
#[test]
fn strategic_recommendation_timing_accepts_quarter_shorthand_and_sourcing_cycle() {
    assert!(recommendation_has_action_timing(
        "demand_procurement",
        "Contact the sourcing team by Q2 2026 with a qualification-led EMS proposal [1]."
    ));
    assert!(recommendation_has_action_timing(
            "demand_procurement",
            "Approach the procurement lead ahead of the next sourcing cycle with a continuity-led EMS proposal [1]."
        ));
}

#[cfg(feature = "llm")]
#[test]
fn causal_link_accepts_expanded_synonyms() {
    assert!(contains_causal_link(
            "The certification lapse is precipitating customer concern and contributing to sourcing reviews."
        ));
    assert!(contains_causal_link(
        "The warning is resulting in qualification risk and giving rise to supplier switches."
    ));
}

#[cfg(feature = "llm")]
#[test]
fn weighted_scorer_matches_stemmed_single_token_variants() {
    assert!(weighted_phrase_score(
            "The supplier was certifying new lines after prior certified output and legacy certifications.",
            &[("certification", 0.30)]
        ) >= 0.30);
    assert!(
        weighted_phrase_score(
            "The compliance team qualified the plant after qualification work on adjacent cells.",
            &[("qualification", 0.30)]
        ) >= 0.30
    );
}

#[cfg(feature = "llm")]
#[test]
fn quality_gate_ensemble_allows_single_non_veto_failure() {
    let decisions = vec![
        quality_gate_blocker("generic_language", true, false),
        quality_gate_requirement("narrative_words", 96.0, 85.0),
        quality_gate_blocker("security_escalation", false, true),
    ];

    assert!(quality_gate_passes_ensemble(&decisions));
}

#[cfg(feature = "llm")]
#[test]
fn quality_gate_ensemble_rejects_three_non_veto_failures() {
    let decisions = vec![
        quality_gate_blocker("generic_language", true, false),
        quality_gate_requirement("narrative_words", 72.0, 85.0),
        quality_gate_requirement("reasoning_depth", 0.0, 0.5),
        quality_gate_blocker("security_escalation", false, true),
    ];

    assert!(!quality_gate_passes_ensemble(&decisions));
}

#[cfg(feature = "llm")]
#[test]
fn quality_gate_ensemble_passes_two_non_veto_failures() {
    let decisions = vec![
        quality_gate_blocker("generic_language", true, false),
        quality_gate_requirement("narrative_words", 72.0, 85.0),
        quality_gate_blocker("security_escalation", false, true),
    ];

    assert!(quality_gate_passes_ensemble(&decisions));
}

#[cfg(feature = "llm")]
#[test]
fn quality_gate_ensemble_rejects_single_veto_failure() {
    let decisions = vec![
        quality_gate_blocker("security_escalation", true, true),
        quality_gate_requirement("narrative_words", 96.0, 85.0),
    ];

    assert!(!quality_gate_passes_ensemble(&decisions));
}

#[cfg(feature = "llm")]
#[test]
fn weekly_quality_gate_metrics_flag_low_precision_and_recall() {
    let reviewed_at = DateTime::<Utc>::from_naive_utc_and_offset(
        chrono::NaiveDate::from_ymd_opt(2026, 3, 10)
            .unwrap_or_default()
            .and_hms_opt(12, 0, 0)
            .unwrap_or_default(),
        Utc,
    );
    let metrics = summarize_quality_gate_reviews(&[
        QualityGateReview {
            gate_name: "certification_escalation",
            blocked: true,
            human_confirmed_block: true,
            reviewed_at,
        },
        QualityGateReview {
            gate_name: "certification_escalation",
            blocked: true,
            human_confirmed_block: false,
            reviewed_at,
        },
        QualityGateReview {
            gate_name: "certification_escalation",
            blocked: false,
            human_confirmed_block: true,
            reviewed_at,
        },
    ]);

    assert_eq!(metrics.len(), 1);
    assert!(metrics[0].alert_precision);
    assert!(metrics[0].alert_recall);
    assert!((metrics[0].precision_observed - 0.5).abs() < f64::EPSILON);
    assert!((metrics[0].recall_observed - 0.5).abs() < f64::EPSILON);
}

#[cfg(feature = "llm")]
#[test]
fn noisy_or_favors_single_strong_signal_over_many_weak_signals() {
    let weak_stack = noisy_or(&[0.15, 0.15, 0.15, 0.15, 0.15]);
    let strong_single = noisy_or(&[0.70]);

    assert!(weak_stack < strong_single);
}

#[cfg(feature = "llm")]
#[test]
fn calculate_relevance_prefers_strong_category_alignment() {
    let strong = calculate_relevance(
            "AS9100 qualification risk emerges in defense program review",
            "Medical device and defense program eligibility could tighten after the latest audit finding.",
            "certification",
            "security_compliance",
        );
    let weak = calculate_relevance(
            "General company update posted",
            "The supplier shared a broad business update with no concrete procurement or compliance signal.",
            "news",
            "security_compliance",
        );

    assert!(strong > weak);
    assert!(strong <= 1.0);
}

#[cfg(feature = "llm")]
#[test]
fn signal_diversity_multiplier_rewards_multiple_types() {
    assert_eq!(signal_diversity_multiplier(1), 1.0);
    assert_eq!(signal_diversity_multiplier(3), 1.2);
}

#[cfg(feature = "llm")]
#[test]
fn golden_set_regression_accepts_at_least_ninety_five_percent_agreement() {
    let reviewed_at = Utc::now();
    let mut examples = Vec::new();

    for index in 0..19 {
        examples.push(QualityGateGoldenSetExample {
                source_id: Uuid::new_v4(),
                source_kind: "warning".to_string(),
                content_type: "supplier_disruption".to_string(),
                historical_label: HistoricalQualityGateLabel::Accepted,
                title: format!("Supplier disruption signal {}", index),
                body: "Multiple customs delays, named customer escalations, and documented sourcing reviews now threaten program continuity across two audited production lines [1].".to_string(),
                region: Some("MENA".to_string()),
                confidence: Some(0.84),
                source_urls: vec!["https://example.com/1".to_string()],
                reviewed_at,
                metadata: serde_json::json!({"review_outcome": "true_positive"}),
            });
    }

    examples.push(QualityGateGoldenSetExample {
            source_id: Uuid::new_v4(),
            source_kind: "warning".to_string(),
            content_type: "supplier_disruption".to_string(),
            historical_label: HistoricalQualityGateLabel::Rejected,
            title: "Generic market commentary".to_string(),
            body: "This may matter eventually, so watch the situation and keep an eye on possible changes if priorities shift later.".to_string(),
            region: Some("Global".to_string()),
            confidence: Some(0.35),
            source_urls: vec!["https://example.com/2".to_string()],
            reviewed_at,
            metadata: serde_json::json!({"review_outcome": "false_positive"}),
        });

    let regression = evaluate_quality_gate_golden_set(&examples);

    assert_eq!(regression.total_examples, 20);
    assert!(regression.disagreements.is_empty());
    assert!(regression.agreement >= 0.95);
}

#[cfg(feature = "llm")]
#[test]
fn golden_set_regression_flags_disagreements_below_threshold() {
    let reviewed_at = Utc::now();
    let mut examples = Vec::new();

    for index in 0..18 {
        examples.push(QualityGateGoldenSetExample {
                source_id: Uuid::new_v4(),
                source_kind: "warning".to_string(),
                content_type: "security_compliance".to_string(),
                historical_label: HistoricalQualityGateLabel::Accepted,
                title: format!("Compliance disruption case {}", index),
                body: "Named audit findings, customer escalation, and sourcing-review evidence indicate immediate qualification risk for active regulated programs [1].".to_string(),
                region: Some("EU".to_string()),
                confidence: Some(0.89),
                source_urls: vec!["https://example.com/a".to_string()],
                reviewed_at,
                metadata: serde_json::json!({"review_outcome": "true_positive"}),
            });
    }

    for index in 0..2 {
        examples.push(QualityGateGoldenSetExample {
            source_id: Uuid::new_v4(),
            source_kind: "warning".to_string(),
            content_type: "security_compliance".to_string(),
            historical_label: HistoricalQualityGateLabel::Accepted,
            title: format!("Borderline accepted case {}", index),
            body: "Stay alert for updates later.".to_string(),
            region: Some("EU".to_string()),
            confidence: Some(0.51),
            source_urls: vec!["https://example.com/b".to_string()],
            reviewed_at,
            metadata: serde_json::json!({"review_outcome": "true_positive"}),
        });
    }

    let regression = evaluate_quality_gate_golden_set(&examples);

    assert_eq!(regression.total_examples, 20);
    assert_eq!(regression.disagreements.len(), 2);
    assert!(regression.agreement < 0.95);
}

#[cfg(feature = "llm")]
#[test]
fn security_hygiene_marker_accepts_adversarial_synonyms() {
    let variants = [
        "The supplier shows an email authentication gap across customer-facing domains.",
        "Monitoring found domain spoofing exposure and a weak mail trust posture.",
        "The brand now faces impersonation risk from look alike domains.",
        "A typosquatting campaign is exploiting the firm's DNS hygiene issue.",
        "The company has spoofing susceptibility across its identity surface.",
    ];

    for variant in variants {
        assert!(
            contains_security_hygiene_marker(variant),
            "missed variant: {variant}"
        );
    }
}

#[cfg(feature = "llm")]
#[test]
fn unnamed_customer_targeting_accepts_adversarial_synonyms() {
    let variants = [
        "Go after their buyer base before the next sourcing cycle [1].",
        "Pursue their account base with a continuity pitch this quarter [1].",
        "Focus on their downstream buyer set for a switch campaign [1].",
        "Engage the customer roster they may lose if delays worsen [1].",
        "Work the account portfolio with a dual-source message [1].",
    ];

    for variant in variants {
        assert!(
            has_unnamed_customer_targeting("Generic risk narrative.", variant),
            "missed variant: {variant}"
        );
    }
}

#[cfg(feature = "llm")]
#[test]
fn public_sector_macro_signal_without_procurement_rejects_hardware_sales_pitch() {
    let entity_ctx = EntityContext {
        name: "Government of UAE".to_string(),
        region: "MENA".to_string(),
        entity_type: Some("Government".to_string()),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("government.ae".to_string()),
    };

    let evidence_signals = vec![
            EvidenceSignal {
                title: "government.ae innovation update".to_string(),
                description: "Innovation signals were detected on government.ae, highlighting strategic investment in media infrastructure and digital transformation.".to_string(),
                source_url: "https://government.ae/en/media/innovation".to_string(),
                signal_type: "web_change".to_string(),
                extracted_facts: vec!["government.ae".to_string(), "innovation".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "New Media Academy announced".to_string(),
                description: "The Government of UAE announced the New Media Academy to train media professionals and social media content creators.".to_string(),
                source_url: "https://government.ae/en/news/new-media-academy".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec!["New Media Academy".to_string(), "media professionals".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 0.9,
            },
        ];

    let narrative = "These developments suggest growing demand for electronics manufacturing services to support UAE's digital transformation. The New Media Academy implies need for advanced hardware, including cameras, streaming devices, and AI-driven analytics tools. Given our ISO 9001 and AS9100 certifications, we are qualified to supply reliable electronics for government projects.";
    let recommendation = "Target the New Media Academy's procurement team to offer PCBA assembly and box build services for media equipment, and submit a qualification package for defense-adjacent electronics manufacturing [1].";

    assert!(has_unsupported_public_sector_commercialization(
        &entity_ctx,
        "geopolitical_analysis",
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn public_sector_explicit_tender_for_hardware_does_not_trigger_sales_pitch_guard() {
    let entity_ctx = EntityContext {
        name: "Government of UAE".to_string(),
        region: "MENA".to_string(),
        entity_type: Some("Government".to_string()),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("government.ae".to_string()),
    };

    let evidence_signals = vec![EvidenceSignal {
            title: "Ministry tender for broadcast equipment".to_string(),
            description: "The ministry procurement portal published a tender for broadcast cameras, streaming devices, and control-room electronics for the New Media Academy campus.".to_string(),
            source_url: "https://procurement.gov.ae/tenders/media-equipment".to_string(),
            signal_type: "tender".to_string(),
            extracted_facts: vec!["tender".to_string(), "broadcast cameras".to_string(), "control-room electronics".to_string()],
            date_context: Some("2026-02-28".to_string()),
            relevance_score: 1.0,
        }];

    let narrative = "A named ministry tender now creates an explicit equipment procurement path for the New Media Academy campus [1].";
    let recommendation = "Approach the named tender contact with a qualification-led hardware manufacturing response [1].";

    assert!(!has_unsupported_public_sector_commercialization(
        &entity_ctx,
        "regulatory_policy",
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn public_sector_generic_opportunity_language_is_rejected_as_low_usefulness() {
    let entity_ctx = EntityContext {
        name: "Government of Canada".to_string(),
        region: "North America".to_string(),
        entity_type: Some("Government".to_string()),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("canada.ca".to_string()),
    };

    let evidence_signals = vec![
            EvidenceSignal {
                title: "Government tariff update".to_string(),
                description: "A government tariff update was posted on the official site together with general hiring signals.".to_string(),
                source_url: "https://canada.ca/trade/tariff-update".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec!["tariff update".to_string(), "hiring".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 1.0,
            },
        ];

    let headline =
        "Government of Canada's Tariff & Hiring Signals: Nearshoring Opportunity in North Africa";
    let narrative = "The tariff update and hiring signals point to a nearshoring opportunity in North Africa for Starz because public-sector buyers may need resilient supply alternatives.";
    let recommendation = "Pursue the nearshoring opportunity and position a North Africa manufacturing response this quarter [1].";

    assert!(has_low_usefulness_public_sector_analysis(
        &entity_ctx,
        "geopolitical_analysis",
        headline,
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn public_sector_concrete_policy_artifact_and_account_action_remain_allowed() {
    let entity_ctx = EntityContext {
        name: "European Commission".to_string(),
        region: "Europe".to_string(),
        entity_type: Some("Government".to_string()),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("ec.europa.eu".to_string()),
    };

    let evidence_signals = vec![EvidenceSignal {
            title: "Trade defense consultation notice".to_string(),
            description: "The European Commission published a trade defense consultation notice that could affect supplier qualification and approval timing for related programs.".to_string(),
            source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            signal_type: "regulatory".to_string(),
            extracted_facts: vec!["consultation notice".to_string(), "supplier qualification".to_string(), "approval timing".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        }];

    let headline =
        "European Commission trade defense consultation could affect supplier qualification timing";
    let narrative = "Because the consultation notice explicitly affects supplier qualification and approval timing, regulated bids tied to the Commission may face a narrower response window [1].";
    let recommendation = "Review the consultation notice, map exposed bids and stakeholder owners, and update qualification planning before approval timing moves [1].";

    assert!(!has_low_usefulness_public_sector_analysis(
        &entity_ctx,
        "regulatory_policy",
        headline,
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn public_sector_output_without_process_or_concrete_action_is_rejected() {
    let entity_ctx = EntityContext {
        name: "European Commission".to_string(),
        region: "Europe".to_string(),
        entity_type: Some("Government".to_string()),
        is_competitor: false,
        industry_tags: vec![],
        certifications: vec![],
        capabilities: vec![],
        key_persons: vec![],
        recent_changes: vec![],
        threat_score: None,
        overlap_score: None,
        strategic_relevance: None,
        revenue_estimate_usd: None,
        employee_estimate: None,
        competitor_names: vec![],
        sites_summary: vec![],
        competitor_events: vec![],
        domain: Some("ec.europa.eu".to_string()),
    };

    let evidence_signals = vec![EvidenceSignal {
            title: "Trade defense consultation notice".to_string(),
            description: "The European Commission published a trade defense consultation notice tied to supplier qualification timing for related programs.".to_string(),
            source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            signal_type: "regulatory".to_string(),
            extracted_facts: vec!["consultation notice".to_string(), "supplier qualification".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        }];

    let headline = "European Commission consultation may matter for suppliers";
    let narrative = "The consultation notice matters for suppliers and could affect activity around the Commission [1].";
    let recommendation = "Continue monitoring and watch for developments before acting [1].";

    assert!(has_low_usefulness_public_sector_analysis(
        &entity_ctx,
        "regulatory_policy",
        headline,
        narrative,
        recommendation,
        &evidence_signals,
    ));
}

#[cfg(feature = "llm")]
#[test]
fn sources_footer_lists_ranked_evidence_urls() {
    let evidence_signals = vec![
        EvidenceSignal {
            title: "Most relevant".to_string(),
            description: "Primary evidence".to_string(),
            source_url: "https://alpha.example.com/report".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![],
            date_context: None,
            relevance_score: 1.0,
        },
        EvidenceSignal {
            title: "Secondary".to_string(),
            description: "Backup evidence".to_string(),
            source_url: "https://beta.example.com/article".to_string(),
            signal_type: "news".to_string(),
            extracted_facts: vec![],
            date_context: None,
            relevance_score: 0.7,
        },
    ];

    let footer = format_sources_footer(&evidence_signals, 4);
    assert!(footer.contains("Sources:"));
    assert!(footer.contains("[1] alpha.example.com — https://alpha.example.com/report"));
    assert!(footer.contains("[2] beta.example.com — https://beta.example.com/article"));
}

#[cfg(feature = "llm")]
#[test]
fn company_name_matching_normalizes_punctuation() {
    assert!(company_name_matches_seed(
        "STMicroelectronics N.V.",
        "STMicroelectronics NV"
    ));
    assert!(company_name_matches_seed(
        "Young Poong Electronics Co., Ltd.",
        "Young Poong Electronics Co Ltd"
    ));
    assert!(!company_name_matches_seed("NVIDIA", "AMD"));
}

#[cfg(feature = "llm")]
#[test]
fn company_name_matching_uses_shared_canonicalization() {
    let mixed = format!("{}cme", '\u{0410}');
    assert!(company_name_matches_seed(
        "Café Société S.A.",
        "Cafe Societe SA"
    ));
    assert!(company_name_matches_seed(&mixed, "Acme"));
}

#[cfg(feature = "llm")]
#[test]
fn candidate_company_domain_prefers_corporate_email() {
    let disc = DiscoveredPoi {
        name: "Jane Doe".to_string(),
        inferred_role: Some("VP Supply Chain".to_string()),
        inferred_org: Some("Acme Electronics".to_string()),
        source_url: "https://news.example.com/article".to_string(),
        discovery_method: "gdelt_co_mention".to_string(),
        contact_email: Some("jane.doe@acme-electronics.com".to_string()),
        contact_linkedin: None,
        confidence: 0.84,
        seed_person_id: "seed-1".to_string(),
        ts_discovered: 0,
    };

    assert_eq!(
        extract_candidate_company_domain(&disc).as_deref(),
        Some("acme-electronics.com")
    );
}

#[cfg(feature = "llm")]
#[test]
fn candidate_company_domain_ignores_public_mailboxes() {
    let disc = DiscoveredPoi {
        name: "Jane Doe".to_string(),
        inferred_role: Some("VP Supply Chain".to_string()),
        inferred_org: Some("Acme Electronics".to_string()),
        source_url: "https://www.acme-electronics.com/team".to_string(),
        discovery_method: "org_leadership".to_string(),
        contact_email: Some("janedoe@gmail.com".to_string()),
        contact_linkedin: None,
        confidence: 0.84,
        seed_person_id: "seed-1".to_string(),
        ts_discovered: 0,
    };

    assert_eq!(
        extract_candidate_company_domain(&disc).as_deref(),
        Some("acme-electronics.com")
    );
}

#[cfg(feature = "llm")]
#[test]
fn sanitize_validated_role_rejects_company_name_garbage() {
    let disc = DiscoveredPoi {
        name: "Sujatha Chandrasekaran".to_string(),
        inferred_role: Some("Chief Operating Officer".to_string()),
        inferred_org: Some("Jabil".to_string()),
        source_url: "https://www.jabil.com/about-us/senior-leadership.html".to_string(),
        discovery_method: "org_leadership".to_string(),
        contact_email: None,
        contact_linkedin: None,
        confidence: 0.75,
        seed_person_id: "seed-1".to_string(),
        ts_discovered: 0,
    };

    assert_eq!(
        sanitize_validated_role(&disc, Some("ApexIntel".to_string()), Some("Jabil")),
        None
    );
    assert_eq!(
        sanitize_validated_role(&disc, Some("Jabil".to_string()), Some("Jabil")),
        None
    );
    assert_eq!(
        sanitize_validated_role(
            &disc,
            Some("Chief Operating Officer".to_string()),
            Some("Jabil")
        )
        .as_deref(),
        Some("Chief Operating Officer")
    );
}

#[cfg(feature = "llm")]
#[test]
fn sanitize_validated_org_rejects_role_text() {
    let disc = DiscoveredPoi {
        name: "Jane Doe".to_string(),
        inferred_role: Some("VP Supply Chain".to_string()),
        inferred_org: Some("Acme Electronics".to_string()),
        source_url: "https://www.acme-electronics.com/team".to_string(),
        discovery_method: "org_leadership".to_string(),
        contact_email: None,
        contact_linkedin: None,
        confidence: 0.84,
        seed_person_id: "seed-1".to_string(),
        ts_discovered: 0,
    };

    assert_eq!(
        sanitize_validated_org(&disc, Some("VP Supply Chain".to_string())),
        None
    );
    assert_eq!(
        sanitize_validated_org(&disc, Some("ApexIntel".to_string())),
        None
    );
    assert_eq!(
        sanitize_validated_org(&disc, Some("Acme Electronics".to_string())).as_deref(),
        Some("Acme Electronics")
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Startup schema gate, subcommand parsing, and scheduler liveness (audit P0
// startup/readiness: no worker may run against an unknown schema, and the
// container healthcheck must prove the scheduler is making progress).
// ────────────────────────────────────────────────────────────────────────────

struct PassingMigrationStore;

#[async_trait::async_trait]
impl SchemaMigrationStore for PassingMigrationStore {
    async fn run_migrations(&self) -> Result<()> {
        Ok(())
    }
}

struct FailingMigrationStore;

#[async_trait::async_trait]
impl SchemaMigrationStore for FailingMigrationStore {
    async fn run_migrations(&self) -> Result<()> {
        Err(anyhow::anyhow!(
            "database schema is stale: latest applied migration is 052, embedded latest is 053"
        ))
    }
}

#[tokio::test]
async fn worker_startup_fails_on_schema_mismatch() {
    let error = ensure_database_schema(&FailingMigrationStore)
        .await
        .expect_err("schema mismatch must abort startup");
    let report = format!("{error:#}");

    assert!(
        report.contains("database schema is not current"),
        "startup error must name the schema gate, got: {report}"
    );
    assert!(
        report.contains("latest applied migration is 052"),
        "startup error must preserve the store failure detail, got: {report}"
    );
}

#[tokio::test]
async fn worker_startup_succeeds_when_schema_is_current() {
    ensure_database_schema(&PassingMigrationStore)
        .await
        .expect("current schema must start");
}

#[test]
fn worker_command_defaults_to_running_the_scheduler() {
    assert_eq!(
        parse_worker_command(Vec::<String>::new()).expect("no args"),
        WorkerCommand::Run
    );
}

#[test]
fn worker_command_parses_healthcheck() {
    assert_eq!(
        parse_worker_command(vec!["healthcheck".to_string()]).expect("healthcheck"),
        WorkerCommand::Healthcheck
    );
}

#[test]
fn worker_command_rejects_unknown_subcommands() {
    let error = parse_worker_command(vec!["healtcheck".to_string()])
        .expect_err("typos must not start a second scheduler");
    assert!(error.to_string().contains("unknown worker subcommand"));
}

#[test]
fn scheduler_progress_clock_flags_stall_only_past_budget() {
    let clock = SchedulerProgressClock::new_at(1_000);

    assert!(!clock.is_stalled(1_000, 900), "fresh clock is not stalled");
    assert!(
        !clock.is_stalled(1_900, 900),
        "exactly at the budget is not stalled"
    );
    assert!(
        clock.is_stalled(1_901, 900),
        "no progress past the budget is a stall"
    );

    clock.record_progress_at(2_000);
    assert!(
        !clock.is_stalled(2_500, 900),
        "each progress record resets the stall window"
    );
}
