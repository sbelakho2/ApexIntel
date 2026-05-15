#![cfg_attr(test, allow(dead_code))]

//! Title and headline construction logic for insights.

use super::is_public_sector_entity;
#[cfg(feature = "llm")]
use super::EvidenceSignal;
#[cfg(feature = "llm")]
use crate::evidence_scoring::clean_signal_title;
use crate::fallback_generation::category_relevant_signal_details;
#[cfg(feature = "llm")]
use crate::truncate_text;

pub(crate) fn build_analytical_title(
    entity_label: &str,
    entity_region: &str,
    category: &str,
    signal_details: &[String],
    entity_type: Option<&str>,
) -> String {
    let is_government = is_public_sector_entity(entity_label, entity_type);

    let action = if is_government {
        match category {
            "competitor_market" => "regulatory activity detected",
            "demand_procurement" => "tender/solicitation identified",
            "supply_chain_risk" => "policy risk flagged",
            "security_compliance" => "regulatory update detected",
            "regulatory_policy" => "policy change detected",
            "strategic_poi" => "official stakeholder activity surfaced",
            "pricing_market" => "economic policy shift detected",
            "customer_rfq" => "procurement signal emerging",
            "geopolitical_analysis" => "policy and trade exposure shift",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "official appointment detected",
            "technology_innovation" => "research initiative detected",
            "ma_partnerships" => "bilateral agreement detected",
            "market_expansion" => "expansion signal with sourcing implications",
            "cybersecurity_threat" => "security advisory detected",
            "quality_compliance" => "standards update detected",
            "brand_sentiment" => "public sentiment signal detected",
            _ => "notable development detected",
        }
    } else {
        match category {
            "competitor_market" => "competitive activity detected",
            "demand_procurement" => "procurement opportunity identified",
            "supply_chain_risk" => "supply chain risk flagged",
            "security_compliance" => "compliance gap identified",
            "regulatory_policy" => "regulatory change detected",
            "strategic_poi" => "stakeholder activity surfaced",
            "pricing_market" => "market pricing shift detected",
            "customer_rfq" => "procurement signal emerging",
            "geopolitical_analysis" => "geopolitical exposure assessment required",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "talent/IP activity detected",
            "technology_innovation" => "R&D activity detected",
            "ma_partnerships" => "M&A/partnership activity detected",
            "market_expansion" => "capacity or market expansion signal",
            "cybersecurity_threat" => "cybersecurity threat detected",
            "quality_compliance" => "quality certification change detected",
            "brand_sentiment" => "brand sentiment signal detected",
            _ => "notable development detected",
        }
    };

    let region_tag = if !entity_region.is_empty() {
        format!(" ({})", entity_region)
    } else {
        String::new()
    };

    // Prefer evidence-specific detail as the lead when available,
    // falling back to category-based action verb.
    let relevant_details = category_relevant_signal_details(category, signal_details);
    let title = if let Some(lead_detail) = relevant_details.first() {
        if lead_detail.len() > 20 {
            // Evidence is specific enough to lead the title
            format!("{}{}: {}", entity_label, region_tag, lead_detail)
        } else {
            // Evidence is too short — use category verb + detail suffix
            let detail_suffix = format!(". {}", lead_detail);
            format!(
                "{}{}: {}{}",
                entity_label, region_tag, action, detail_suffix
            )
        }
    } else {
        format!("{}{}: {}", entity_label, region_tag, action)
    };
    if title.chars().count() > 200 {
        let truncated: String = title.chars().take(197).collect();
        format!("{truncated}...")
    } else {
        title
    }
}

#[cfg(feature = "llm")]
pub(crate) fn build_rich_headline(
    name: &str,
    region: &str,
    category: &str,
    top_signals: &[&EvidenceSignal],
    article_titles: &[String],
    certs: &[&str],
    caps: &[&str],
    has_acquisition: bool,
    has_expansion: bool,
    has_innovation: bool,
    has_tariff: bool,
    has_shortage: bool,
    has_financial: bool,
    has_certification: bool,
) -> String {
    if let Some(best_article) = article_titles.first() {
        let short = truncate_text(best_article, 80);
        return format!("{name}: {short}");
    }

    for sig in top_signals {
        let cleaned = clean_signal_title(&sig.title, name);
        if cleaned.len() > 15 {
            return format!("{name}: {}", truncate_text(&cleaned, 80));
        }
    }

    if has_acquisition {
        return format!("{name}: acquisition activity signals competitive repositioning");
    }
    if has_shortage {
        return format!("{name}: supply allocation constraints — assess component availability");
    }
    if has_tariff {
        return format!("{name}: trade policy exposure requires sourcing review in {region}");
    }
    if has_expansion {
        if !caps.is_empty() {
            return format!(
                "{name}: capacity expansion in {} manufacturing",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            );
        }
        return format!("{name}: facility expansion underway in {region}");
    }
    if has_innovation {
        if !caps.is_empty() {
            return format!(
                "{name}: technology evolution in {} — early engagement window",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            );
        }
        return format!("{name}: innovation and R&D signals indicate technology pivot");
    }
    if has_financial {
        return format!("{name}: financial activity signals — review business trajectory");
    }
    if has_certification && !certs.is_empty() {
        return format!(
            "{name}: {} certification update — verify qualification status",
            certs[0].split('(').next().unwrap_or(certs[0]).trim()
        );
    }

    match category {
        "demand_procurement" => {
            if !caps.is_empty() {
                format!(
                    "{name}: procurement signals in {} — sourcing opportunity",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name} ({region}): procurement activity indicates emerging sourcing opportunity")
            }
        }
        "supply_chain_risk" => {
            format!("{name}: supply chain risk indicators — mitigation planning required")
        }
        "competitor_market" => {
            format!("{name}: competitive positioning shift detected in {region}")
        }
        "security_compliance" => {
            if !certs.is_empty() {
                format!(
                    "{name}: {} compliance update — supplier qualification impact",
                    certs[0].split('(').next().unwrap_or(certs[0]).trim()
                )
            } else {
                format!("{name}: security and compliance posture update")
            }
        }
        "regulatory_policy" => format!("{name}: regulatory changes affect operations in {region}"),
        "strategic_poi" => {
            if !caps.is_empty() {
                format!(
                    "{name}: strategic signals in {} domain",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name}: strategic activity signals direction change in {region}")
            }
        }
        "technology_innovation" => {
            if !caps.is_empty() {
                format!(
                    "{name}: {} technology evolution — capability development signals",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name}: technology investment signals emerging capability")
            }
        }
        "ma_partnerships" => {
            format!("{name}: M&A or partnership activity reshaping competitive landscape")
        }
        "market_expansion" => {
            if !certs.is_empty() {
                format!(
                    "{name}: market expansion with {} qualification in {region}",
                    certs[0].split('(').next().unwrap_or(certs[0]).trim()
                )
            } else {
                format!("{name}: market expansion activity detected in {region}")
            }
        }
        "quality_compliance" => {
            format!("{name}: quality system update — supplier requalification required")
        }
        "talent_ip" => {
            format!("{name}: talent and IP investment signals strategic capability build")
        }
        "customer_rfq" => format!("{name}: RFQ or customer engagement activity in {region}"),
        "geopolitical_analysis" => {
            format!("{name}: geopolitical exposure assessment required for {region}")
        }
        "pricing_market" => format!("{name}: pricing signals indicate market dynamics shift"),
        "cybersecurity_threat" => {
            format!("{name}: cybersecurity posture change — vendor risk review")
        }
        "brand_sentiment" => format!("{name}: brand and reputation signals require monitoring"),
        _ => format!("{name} ({region}): new intelligence signals require assessment"),
    }
}

#[cfg(feature = "llm")]
pub(crate) fn describe_entity_type_with_article(entity_type: Option<&str>) -> &'static str {
    match entity_type {
        Some(t) if t.to_lowercase().contains("ems") => {
            "an electronics manufacturing services (EMS) provider"
        }
        Some(t) if t.to_lowercase().contains("oem") => "an original equipment manufacturer (OEM)",
        Some(t) if t.to_lowercase().contains("semiconductor") => "a semiconductor manufacturer",
        Some(t) if t.to_lowercase().contains("defense") || t.to_lowercase().contains("defence") => {
            "a defense and aerospace company"
        }
        Some(t) if t.to_lowercase().contains("government") => "a government entity",
        Some(t) if t.to_lowercase().contains("automotive") => "an automotive tier supplier",
        Some(t) if t.to_lowercase().contains("distributor") => {
            "an electronic components distributor"
        }
        _ => "a company",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytical_title_uses_concrete_detail() {
        let title = build_analytical_title(
            "Acme",
            "Morocco",
            "demand_procurement",
            &[
                "3 tender(s) identified".to_string(),
                "AS9100 recertification required".to_string(),
            ],
            Some("EMS"),
        );

        assert!(title.contains("AS9100 recertification required"));
        assert!(!title.contains("3 tender(s) identified"));
    }

    #[test]
    fn analytical_title_ignores_security_hygiene_for_nonsecurity_categories() {
        let title = build_analytical_title(
            "Acme",
            "Morocco",
            "demand_procurement",
            &[
                "8 lookalike domain(s) detected".to_string(),
                "Supplier portal opened for Q3 RFQ".to_string(),
            ],
            Some("EMS"),
        );

        assert!(title.contains("Supplier portal opened for Q3 RFQ"));
        assert!(!title.contains("lookalike domain"));
    }
}
