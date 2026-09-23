//! Certification gap analyzer.
//!
//! Compares a company's certifications against sector requirements to
//! identify gaps as business opportunities or risks.
//!
//! Sector profiles: Automotive (IATF 16949), Aerospace (AS9100),
//! Medical (ISO 13485), Defense (ITAR/EAR, NADCAP), General (ISO 9001).

use serde::Serialize;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CertGapAnalysis {
    pub company_name: String,
    pub target_sector: String,
    pub held_certifications: Vec<String>,
    pub required_certifications: Vec<SectorRequirement>,
    pub gaps: Vec<CertGap>,
    pub coverage_pct: f64,
    pub assessment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectorRequirement {
    pub standard: String,
    pub description: String,
    pub mandatory: bool,
    pub typical_timeline_months: u32,
    pub estimated_cost_usd: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertGap {
    pub standard: String,
    pub description: String,
    pub mandatory: bool,
    pub gap_type: GapType,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum GapType {
    /// Missing a mandatory certification — blocks market entry
    BlockingGap,
    /// Missing a recommended certification — competitive disadvantage
    CompetitiveGap,
    /// Certification is held but may be expiring or outdated
    ExpiryRisk,
    /// Company has cert the competitor doesn't — competitive advantage
    CompetitiveAdvantage,
}

// ─────────────────────────────────────────────────────────────────────────────
// Sector requirements database
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_sector_requirements(sector: &str) -> Vec<SectorRequirement> {
    match sector.to_lowercase().as_str() {
        "automotive" => vec![
            req(
                "IATF 16949",
                "Automotive quality management",
                true,
                12,
                50000,
            ),
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req("ISO 14001", "Environmental management", false, 6, 12000),
            req("ISO 45001", "Occupational health & safety", false, 6, 12000),
            req("VDA 6.3", "Process audit standard", false, 3, 8000),
            req("AIAG CQI-9", "Heat treat system assessment", false, 2, 5000),
            req(
                "IPC-A-610",
                "Acceptability of electronic assemblies",
                true,
                1,
                3000,
            ),
            req(
                "AEC-Q100",
                "Automotive IC qualification (if applicable)",
                false,
                6,
                25000,
            ),
        ],
        "aerospace" | "defense_aero" => vec![
            req("AS9100", "Aerospace quality management", true, 12, 60000),
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req(
                "NADCAP",
                "National Aerospace & Defense Accreditation",
                true,
                12,
                40000,
            ),
            req("ISO 14001", "Environmental management", false, 6, 12000),
            req(
                "IPC J-STD-001 Space",
                "Space-level soldering",
                false,
                3,
                8000,
            ),
            req(
                "IPC-A-620",
                "Wire and cable harness acceptability",
                false,
                1,
                3000,
            ),
            req(
                "ITAR Compliance",
                "International Traffic in Arms Regulations",
                true,
                6,
                20000,
            ),
        ],
        "medical" => vec![
            req(
                "ISO 13485",
                "Medical device quality management",
                true,
                12,
                55000,
            ),
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req(
                "FDA 21 CFR 820",
                "FDA Quality System Regulation",
                true,
                9,
                30000,
            ),
            req(
                "IEC 60601",
                "Medical electrical equipment safety",
                true,
                6,
                20000,
            ),
            req(
                "ISO 14971",
                "Risk management for medical devices",
                true,
                3,
                10000,
            ),
            req("ISO 14001", "Environmental management", false, 6, 12000),
            req(
                "IPC-A-610 Class 3",
                "High-reliability assembly",
                false,
                1,
                3000,
            ),
        ],
        "defense" | "military" => vec![
            req(
                "ITAR Compliance",
                "International Traffic in Arms Regulations",
                true,
                6,
                20000,
            ),
            req(
                "MIL-STD-883",
                "Test methods for microelectronics",
                true,
                6,
                15000,
            ),
            req(
                "MIL-PRF-38534",
                "Hybrid microcircuit general spec",
                false,
                9,
                25000,
            ),
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req(
                "NADCAP",
                "National Aerospace & Defense Accreditation",
                true,
                12,
                40000,
            ),
            req("AS9100", "Aerospace quality management", false, 12, 60000),
            req(
                "NIST SP 800-171",
                "Cybersecurity for controlled info",
                true,
                6,
                30000,
            ),
        ],
        "telecom" => vec![
            req("TL 9000", "Telecom quality management", true, 9, 35000),
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req("ISO 14001", "Environmental management", false, 6, 12000),
            req(
                "IPC-A-610",
                "Electronic assembly acceptability",
                true,
                1,
                3000,
            ),
        ],
        _ => vec![
            // General electronics manufacturing
            req("ISO 9001", "Quality management system", true, 6, 15000),
            req("ISO 14001", "Environmental management", false, 6, 12000),
            req("ISO 45001", "Occupational health & safety", false, 6, 12000),
            req(
                "IPC-A-610",
                "Electronic assembly acceptability",
                true,
                1,
                3000,
            ),
            req("IPC J-STD-001", "Soldering requirements", true, 1, 3000),
            req(
                "UL Certification",
                "Product safety listing",
                false,
                3,
                10000,
            ),
        ],
    }
}

fn req(standard: &str, desc: &str, mandatory: bool, months: u32, cost: u64) -> SectorRequirement {
    SectorRequirement {
        standard: standard.into(),
        description: desc.into(),
        mandatory,
        typical_timeline_months: months,
        estimated_cost_usd: cost,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gap analysis engine
// ─────────────────────────────────────────────────────────────────────────────

/// Analyze certification gaps for a company targeting a specific sector.
pub fn analyze_cert_gaps(
    company_name: &str,
    held_certs: &[String],
    target_sector: &str,
) -> CertGapAnalysis {
    let requirements = get_sector_requirements(target_sector);
    let held_lower: Vec<String> = held_certs.iter().map(|c| c.to_lowercase()).collect();

    let mut gaps = Vec::new();
    let mut met_count = 0;

    for req in &requirements {
        let is_held = held_lower.iter().any(|h| {
            h.contains(&req.standard.to_lowercase()) || req.standard.to_lowercase().contains(h)
        });

        if is_held {
            met_count += 1;
        } else {
            let (gap_type, recommendation) = if req.mandatory {
                (
                    GapType::BlockingGap,
                    format!(
                        "CRITICAL: Obtain {} certification. Estimated timeline: {} months, cost: ~${}",
                        req.standard, req.typical_timeline_months, req.estimated_cost_usd
                    ),
                )
            } else {
                (
                    GapType::CompetitiveGap,
                    format!(
                        "Recommended: {} would strengthen competitive position. Timeline: {} months",
                        req.standard, req.typical_timeline_months
                    ),
                )
            };

            gaps.push(CertGap {
                standard: req.standard.clone(),
                description: req.description.clone(),
                mandatory: req.mandatory,
                gap_type,
                recommendation,
            });
        }
    }

    let coverage = if requirements.is_empty() {
        100.0
    } else {
        (met_count as f64 / requirements.len() as f64) * 100.0
    };

    let blocking = gaps
        .iter()
        .filter(|g| g.gap_type == GapType::BlockingGap)
        .count();
    let assessment = if blocking == 0 && coverage >= 80.0 {
        format!(
            "Strong position for {} sector. {:.0}% coverage, no blocking gaps.",
            target_sector, coverage
        )
    } else if blocking == 0 {
        format!(
            "Ready for {} sector with {:.0}% coverage. Recommended certifications would improve competitiveness.",
            target_sector, coverage
        )
    } else {
        format!(
            "NOT READY for {} sector. {} blocking certification gap(s) must be resolved. Current coverage: {:.0}%.",
            target_sector, blocking, coverage
        )
    };

    CertGapAnalysis {
        company_name: company_name.into(),
        target_sector: target_sector.into(),
        held_certifications: held_certs.to_vec(),
        required_certifications: requirements,
        gaps,
        coverage_pct: (coverage * 10.0).round() / 10.0,
        assessment,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    #[test]
    fn automotive_fully_certified() {
        let held = vec![
            "IATF 16949".into(),
            "ISO 9001".into(),
            "ISO 14001".into(),
            "ISO 45001".into(),
            "VDA 6.3".into(),
            "IPC-A-610".into(),
        ];
        let result = analyze_cert_gaps("StarzEMS", &held, "automotive");
        assert!(result.coverage_pct > 60.0);
        assert!(!result.gaps.iter().any(|g| g.standard == "IATF 16949"));
    }

    #[test]
    fn automotive_missing_iatf() {
        let held = vec!["ISO 9001".into()];
        let result = analyze_cert_gaps("NewCo", &held, "automotive");
        let iatf_gap = result.gaps.iter().find(|g| g.standard == "IATF 16949");
        assert!(iatf_gap.is_some());
        assert_eq!(iatf_gap.unwrap().gap_type, GapType::BlockingGap);
        assert!(result.assessment.contains("NOT READY"));
    }

    #[test]
    fn aerospace_requirements() {
        let reqs = get_sector_requirements("aerospace");
        assert!(reqs.iter().any(|r| r.standard == "AS9100"));
        assert!(reqs.iter().any(|r| r.standard == "NADCAP"));
    }

    #[test]
    fn medical_requirements() {
        let reqs = get_sector_requirements("medical");
        assert!(reqs.iter().any(|r| r.standard == "ISO 13485"));
    }

    #[test]
    fn general_sector_fallback() {
        let reqs = get_sector_requirements("unknown");
        assert!(reqs.iter().any(|r| r.standard == "ISO 9001"));
    }

    #[test]
    fn coverage_calculation() {
        let held = vec!["ISO 9001".into(), "ISO 14001".into()];
        let result = analyze_cert_gaps("TestCo", &held, "general");
        assert!(result.coverage_pct > 0.0 && result.coverage_pct < 100.0);
    }
}
