//! POI conflict-of-interest detector.
//!
//! Cross-references person directorships, shareholdings, and advisory roles
//! across companies to flag hidden relationships — e.g., a procurement POI
//! at one company who is a director/shareholder at a competitor.
//!
//! Feeds into: warning generation (type = `poi_conflict_of_interest`),
//! dossier red-flag sections, and graph edge annotations.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tracing::info;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// A person's affiliation with a company.
#[derive(Debug, Clone)]
pub struct Affiliation {
    pub person_id: String,
    pub person_name: String,
    pub company_id: String,
    pub company_name: String,
    pub role: String,
    pub is_competitor: bool,
}

/// A detected conflict of interest.
#[derive(Debug, Clone, Serialize)]
pub struct ConflictOfInterest {
    pub person_id: String,
    pub person_name: String,
    pub primary_company: String,
    pub primary_role: String,
    pub conflicting_company: String,
    pub conflicting_role: String,
    pub conflict_type: ConflictType,
    pub severity: ConflictSeverity,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ConflictType {
    /// Person holds roles at two competing companies
    CompetitorDualRole,
    /// Person is a decision-maker at buyer and supplier
    BuyerSupplierConflict,
    /// Person is board member at multiple tracked companies
    BoardInterlocking,
    /// Person has ownership stake in a competitor
    OwnershipConflict,
}

#[derive(Debug, Clone, Serialize, PartialEq, PartialOrd)]
pub enum ConflictSeverity {
    Low,
    Medium,
    High,
    Critical,
}

// ─────────────────────────────────────────────────────────────────────────────
// Detection engine
// ─────────────────────────────────────────────────────────────────────────────

/// Detect conflicts of interest from a list of affiliations.
pub fn detect_conflicts(affiliations: &[Affiliation]) -> Vec<ConflictOfInterest> {
    let mut conflicts = Vec::new();

    // Group affiliations by person
    let mut by_person: HashMap<&str, Vec<&Affiliation>> = HashMap::new();
    for aff in affiliations {
        by_person.entry(&aff.person_id).or_default().push(aff);
    }

    for (person_id, affs) in &by_person {
        if affs.len() < 2 {
            continue;
        }

        let company_ids: HashSet<&str> = affs.iter().map(|a| a.company_id.as_str()).collect();
        if company_ids.len() < 2 {
            continue;
        }

        // Check every pair of affiliations
        for i in 0..affs.len() {
            for j in (i + 1)..affs.len() {
                let a = affs[i];
                let b = affs[j];
                if a.company_id == b.company_id {
                    continue;
                }

                // Competitor dual-role
                if a.is_competitor && b.is_competitor {
                    let severity = classify_severity(&a.role, &b.role);
                    conflicts.push(ConflictOfInterest {
                        person_id: person_id.to_string(),
                        person_name: a.person_name.clone(),
                        primary_company: a.company_name.clone(),
                        primary_role: a.role.clone(),
                        conflicting_company: b.company_name.clone(),
                        conflicting_role: b.role.clone(),
                        conflict_type: ConflictType::CompetitorDualRole,
                        severity: severity.clone(),
                        description: format!(
                            "{} holds '{}' at {} AND '{}' at {} — both competitors",
                            a.person_name, a.role, a.company_name, b.role, b.company_name
                        ),
                    });
                }

                // Board interlocking
                if is_board_role(&a.role) && is_board_role(&b.role) {
                    conflicts.push(ConflictOfInterest {
                        person_id: person_id.to_string(),
                        person_name: a.person_name.clone(),
                        primary_company: a.company_name.clone(),
                        primary_role: a.role.clone(),
                        conflicting_company: b.company_name.clone(),
                        conflicting_role: b.role.clone(),
                        conflict_type: ConflictType::BoardInterlocking,
                        severity: ConflictSeverity::Medium,
                        description: format!(
                            "{} serves on boards of both {} and {}",
                            a.person_name, a.company_name, b.company_name
                        ),
                    });
                }

                // Ownership conflict
                if is_ownership_role(&a.role) && a.is_competitor != b.is_competitor {
                    conflicts.push(ConflictOfInterest {
                        person_id: person_id.to_string(),
                        person_name: a.person_name.clone(),
                        primary_company: b.company_name.clone(),
                        primary_role: b.role.clone(),
                        conflicting_company: a.company_name.clone(),
                        conflicting_role: a.role.clone(),
                        conflict_type: ConflictType::OwnershipConflict,
                        severity: ConflictSeverity::High,
                        description: format!(
                            "{} has ownership role '{}' at {} while holding '{}' at {}",
                            a.person_name, a.role, a.company_name, b.role, b.company_name
                        ),
                    });
                }
            }
        }
    }

    // Deduplicate by (person, company-pair)
    conflicts.sort_by(|a, b| {
        a.person_id
            .cmp(&b.person_id)
            .then(a.primary_company.cmp(&b.primary_company))
            .then(a.severity.partial_cmp(&b.severity).unwrap().reverse())
    });
    conflicts.dedup_by(|a, b| {
        a.person_id == b.person_id
            && ((a.primary_company == b.primary_company
                && a.conflicting_company == b.conflicting_company)
                || (a.primary_company == b.conflicting_company
                    && a.conflicting_company == b.primary_company))
    });

    info!(
        conflicts = conflicts.len(),
        persons = by_person.len(),
        "POI conflict detection complete"
    );
    conflicts
}

fn is_board_role(role: &str) -> bool {
    let r = role.to_lowercase();
    r.contains("director")
        || r.contains("board")
        || r.contains("chairman")
        || r.contains("trustee")
        || r.contains("governor")
}

fn is_ownership_role(role: &str) -> bool {
    let r = role.to_lowercase();
    r.contains("shareholder")
        || r.contains("owner")
        || r.contains("partner")
        || r.contains("founder")
        || r.contains("investor")
}

fn is_decision_maker(role: &str) -> bool {
    let r = role.to_lowercase();
    r.contains("ceo")
        || r.contains("cto")
        || r.contains("cfo")
        || r.contains("vp")
        || r.contains("director")
        || r.contains("head")
        || r.contains("manager")
        || r.contains("procurement")
}

fn classify_severity(role_a: &str, role_b: &str) -> ConflictSeverity {
    if is_decision_maker(role_a) && is_decision_maker(role_b) {
        ConflictSeverity::Critical
    } else if is_decision_maker(role_a) || is_decision_maker(role_b) {
        ConflictSeverity::High
    } else if is_board_role(role_a) || is_board_role(role_b) {
        ConflictSeverity::Medium
    } else {
        ConflictSeverity::Low
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_aff(
        person_id: &str,
        name: &str,
        company_id: &str,
        company: &str,
        role: &str,
        competitor: bool,
    ) -> Affiliation {
        Affiliation {
            person_id: person_id.into(),
            person_name: name.into(),
            company_id: company_id.into(),
            company_name: company.into(),
            role: role.into(),
            is_competitor: competitor,
        }
    }

    #[test]
    fn detect_competitor_dual_role() {
        let affs = vec![
            make_aff("p1", "John", "c1", "CompA", "VP Sales", true),
            make_aff("p1", "John", "c2", "CompB", "Director", true),
        ];
        let conflicts = detect_conflicts(&affs);
        assert!(!conflicts.is_empty());
        assert_eq!(conflicts[0].conflict_type, ConflictType::CompetitorDualRole);
    }

    #[test]
    fn no_conflict_single_company() {
        let affs = vec![
            make_aff("p1", "Jane", "c1", "CompA", "CEO", true),
            make_aff("p1", "Jane", "c1", "CompA", "Board Member", true),
        ];
        let conflicts = detect_conflicts(&affs);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn board_interlocking() {
        let affs = vec![
            make_aff("p1", "Ali", "c1", "CompA", "Board Director", false),
            make_aff("p1", "Ali", "c2", "CompB", "Board Chairman", false),
        ];
        let conflicts = detect_conflicts(&affs);
        assert!(conflicts
            .iter()
            .any(|c| c.conflict_type == ConflictType::BoardInterlocking));
    }

    #[test]
    fn severity_classification() {
        assert_eq!(
            classify_severity("CEO", "CTO"),
            ConflictSeverity::Critical
        );
        assert_eq!(
            classify_severity("CEO", "Engineer"),
            ConflictSeverity::High
        );
        assert_eq!(
            classify_severity("Board Member", "Consultant"),
            ConflictSeverity::Medium
        );
        assert_eq!(
            classify_severity("Consultant", "Advisor"),
            ConflictSeverity::Low
        );
    }
}
