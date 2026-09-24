#![cfg_attr(test, allow(dead_code))]

//! Title and headline construction logic for insights.

use super::is_public_sector_entity;
use crate::fallback_generation::category_relevant_signal_details;

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
