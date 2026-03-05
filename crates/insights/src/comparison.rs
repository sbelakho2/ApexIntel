//! Competitive comparison matrix generator.
//!
//! Builds a structured comparison of Starz capabilities vs. competitor
//! capabilities including certification gaps, regional advantages, and
//! proof-grade analysis.
//!
//! Used by: weekly strategy memo, dossier generation, `/api/insights/comparison`.

use serde::Serialize;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ComparisonMatrix {
    pub starz_capabilities: Vec<String>,
    pub competitors: Vec<CompetitorColumn>,
    pub capability_rows: Vec<CapabilityRow>,
    pub summary: ComparisonSummary,
}

#[derive(Debug, Serialize)]
pub struct CompetitorColumn {
    pub name: String,
    pub region: String,
    pub threat_score: f64,
    pub overlap_score: f64,
}

#[derive(Debug, Serialize)]
pub struct CapabilityRow {
    pub capability: String,
    pub starz_has: bool,
    pub starz_proof_grade: String,
    pub competitor_status: HashMap<String, CapStatus>,
}

#[derive(Debug, Serialize)]
pub struct CapStatus {
    pub has_capability: bool,
    pub proof_grade: String,
    pub certified: bool,
}

#[derive(Debug, Serialize)]
pub struct ComparisonSummary {
    pub starz_unique_capabilities: Vec<String>,
    pub common_capabilities: Vec<String>,
    pub competitor_unique_capabilities: Vec<String>,
    pub starz_cert_advantage: Vec<String>,
    pub starz_cert_gap: Vec<String>,
    pub regional_advantages: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Build a comparison matrix from Starz capabilities and competitor data.
///
/// # Arguments
/// * `starz_caps` — (capability_name, proof_grade, is_certified) tuples
/// * `competitors` — (name, region, threat_score, overlap_score, capabilities) tuples
pub fn build_comparison_matrix(
    starz_caps: &[(String, String, bool)],
    competitors: &[(String, String, f64, f64, Vec<(String, String, bool)>)],
) -> ComparisonMatrix {
    // Collect all unique capabilities
    let all_capabilities: Vec<String> = {
        let mut caps: Vec<String> = starz_caps.iter().map(|c| c.0.clone()).collect();
        for (_, _, _, _, comp_caps) in competitors {
            for (cap, _, _) in comp_caps {
                if !caps.contains(cap) {
                    caps.push(cap.clone());
                }
            }
        }
        caps.sort();
        caps
    };

    let competitor_columns: Vec<CompetitorColumn> = competitors
        .iter()
        .map(|(name, region, threat, overlap, _)| CompetitorColumn {
            name: name.clone(),
            region: region.clone(),
            threat_score: *threat,
            overlap_score: *overlap,
        })
        .collect();

    let mut capability_rows = Vec::new();
    let mut starz_unique = Vec::new();
    let mut common = Vec::new();
    let mut comp_unique = Vec::new();
    let mut cert_advantage = Vec::new();
    let mut cert_gap = Vec::new();

    for cap in &all_capabilities {
        let starz = starz_caps.iter().find(|c| &c.0 == cap);
        let starz_has = starz.is_some();
        let starz_grade = starz.map(|c| c.1.clone()).unwrap_or_default();
        let starz_certified = starz.map(|c| c.2).unwrap_or(false);

        let mut comp_status = HashMap::new();
        let mut any_competitor_has = false;
        let mut any_competitor_certified = false;

        for (name, _, _, _, comp_caps) in competitors {
            let comp = comp_caps.iter().find(|c| &c.0 == cap);
            let has = comp.is_some();
            if has {
                any_competitor_has = true;
            }
            let certified = comp.map(|c| c.2).unwrap_or(false);
            if certified {
                any_competitor_certified = true;
            }
            comp_status.insert(
                name.clone(),
                CapStatus {
                    has_capability: has,
                    proof_grade: comp.map(|c| c.1.clone()).unwrap_or_default(),
                    certified,
                },
            );
        }

        if starz_has && !any_competitor_has {
            starz_unique.push(cap.clone());
        } else if starz_has && any_competitor_has {
            common.push(cap.clone());
        } else if !starz_has && any_competitor_has {
            comp_unique.push(cap.clone());
        }

        if starz_certified && !any_competitor_certified {
            cert_advantage.push(cap.clone());
        }
        if !starz_certified && any_competitor_certified {
            cert_gap.push(cap.clone());
        }

        capability_rows.push(CapabilityRow {
            capability: cap.clone(),
            starz_has,
            starz_proof_grade: starz_grade,
            competitor_status: comp_status,
        });
    }

    ComparisonMatrix {
        starz_capabilities: starz_caps.iter().map(|c| c.0.clone()).collect(),
        competitors: competitor_columns,
        capability_rows,
        summary: ComparisonSummary {
            starz_unique_capabilities: starz_unique,
            common_capabilities: common,
            competitor_unique_capabilities: comp_unique,
            starz_cert_advantage: cert_advantage,
            starz_cert_gap: cert_gap,
            regional_advantages: vec![
                "Tunisia/Morocco nearshore proximity to EU".into(),
                "Multi-region presence (TN+MA) for dual-source".into(),
                "Free-zone tax advantages".into(),
            ],
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_matrix_generation() {
        let starz = vec![
            ("SMT Assembly".into(), "A".into(), true),
            ("Wire Bonding".into(), "B".into(), false),
            ("Conformal Coating".into(), "A".into(), true),
        ];
        let competitors = vec![(
            "CompetitorX".into(),
            "CN".into(),
            0.85,
            0.6,
            vec![
                ("SMT Assembly".into(), "B".into(), true),
                ("Die Attach".into(), "A".into(), true),
            ],
        )];

        let matrix = build_comparison_matrix(&starz, &competitors);

        assert_eq!(matrix.competitors.len(), 1);
        assert_eq!(matrix.capability_rows.len(), 4); // 3 starz + 1 comp-unique
        assert!(matrix
            .summary
            .starz_unique_capabilities
            .contains(&"Conformal Coating".into()));
        assert!(matrix
            .summary
            .starz_unique_capabilities
            .contains(&"Wire Bonding".into()));
        assert!(matrix
            .summary
            .competitor_unique_capabilities
            .contains(&"Die Attach".into()));
        assert!(matrix
            .summary
            .common_capabilities
            .contains(&"SMT Assembly".into()));
    }

    #[test]
    fn empty_competitors() {
        let starz = vec![("PCB Fabrication".into(), "A".into(), true)];
        let matrix = build_comparison_matrix(&starz, &[]);
        assert_eq!(matrix.capability_rows.len(), 1);
        assert!(matrix.summary.starz_unique_capabilities.contains(&"PCB Fabrication".into()));
    }

    #[test]
    fn cert_advantage_detection() {
        let starz = vec![("AOI Testing".into(), "A".into(), true)];
        let competitors = vec![(
            "Rival".into(),
            "EU".into(),
            0.5,
            0.3,
            vec![("AOI Testing".into(), "C".into(), false)],
        )];
        let matrix = build_comparison_matrix(&starz, &competitors);
        assert!(matrix.summary.starz_cert_advantage.contains(&"AOI Testing".into()));
    }

    #[test]
    fn cert_gap_detection() {
        let starz = vec![("X-Ray Inspection".into(), "B".into(), false)];
        let competitors = vec![(
            "Rival".into(),
            "JP".into(),
            0.7,
            0.5,
            vec![("X-Ray Inspection".into(), "A".into(), true)],
        )];
        let matrix = build_comparison_matrix(&starz, &competitors);
        assert!(matrix.summary.starz_cert_gap.contains(&"X-Ray Inspection".into()));
    }
}
