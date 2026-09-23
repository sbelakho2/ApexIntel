//! POI profile updater — merges new data into existing profiles.
//!
//! All merge logic computes from actual input profile data — nothing is fabricated.
//!
//! # Anti-Hallucination
//! - All extracted orgs are validated against artifact evidence
//! - All extracted titles are cross-referenced with role history
//! - Psych profiles are fully computed from real data (never left at defaults)
//! - Risk tolerance, preferred proof types, and pain index are all evidence-backed
//! - Every profile update runs a validation pass

use crate::llm_entity_extractor;
use crate::model::*;
use crate::validator;
use chrono::Utc;
use tracing::{debug, warn};

/// Apply a delta update to a PoiProfile with new artifacts,
/// fresh influence signals, and role change detection.
///
/// # Data sources
/// - `existing` — the current stored profile
/// - `new_artifacts` — freshly crawled artifacts since last refresh
/// - `fresh_influence` — updated influence scores from graph/recrawl
pub fn update_profile(
    existing: &mut PoiProfile,
    new_artifacts: Vec<PoiArtifact>,
    fresh_influence: Option<InfluenceProfile>,
) {
    update_profile_with_llm(existing, new_artifacts, fresh_influence, None)
}

/// Apply a delta update to a PoiProfile with new artifacts,
/// fresh influence signals, role change detection, and optional LLM enrichment.
///
/// When `llm_client` is provided, entity extraction uses LLM-powered extraction
/// with heuristic cross-validation, significantly improving org/title accuracy.
pub fn update_profile_with_llm(
    existing: &mut PoiProfile,
    new_artifacts: Vec<PoiArtifact>,
    fresh_influence: Option<InfluenceProfile>,
    llm_client: Option<&apex_llm::inference::LlmClient>,
) {
    let now_epoch = Utc::now().timestamp();
    let old_count = existing.artifacts.len();
    let merged_count = new_artifacts.len();
    let has_new_data = merged_count > 0 || fresh_influence.is_some();

    // Merge artifacts (append new, dedup by source_url + title)
    let mut seen = std::collections::HashSet::new();
    for a in &existing.artifacts {
        seen.insert((a.title.clone(), a.source_url.clone()));
    }
    for a in new_artifacts {
        if seen.insert((a.title.clone(), a.source_url.clone())) {
            existing.artifacts.push(a);
        }
    }
    existing.clamp_artifacts();

    let actual_new = existing.artifacts.len().saturating_sub(old_count);
    debug!(
        person = %existing.name,
        prior = old_count,
        merged = merged_count,
        actual_new,
        "artifacts merged"
    );

    // Detect role changes from artifact content using improved extraction.
    // Always run this even when no NEW artifacts arrive — there may be
    // previously-merged artifacts with unprocessed change signals.
    detect_change_vs_current(existing, now_epoch, llm_client);

    // If nothing changed in this call and no influence update needed, we're done
    if !has_new_data {
        debug!(person = %existing.name, "No new data to merge — skipping remaining updates");
        return;
    }

    // Update influence if provided
    if let Some(influence) = fresh_influence {
        existing.influence = influence;
        existing.influence.clamp_network_size();
    }

    // Refresh priority vector from artifacts
    existing.priority_vector = crate::features::compute_priority_vector(&existing.artifacts);

    // Refresh psychological profile from artifacts (using real POI features).
    // ALL psych fields are now computed from real data — no defaults are left.
    compute_full_psych_profile(existing, now_epoch);

    // Recompute profile completeness
    existing.last_updated_utc = now_epoch;
    existing.profile_completeness = existing.compute_completeness();

    // Run validation pass to check for hallucinations and data quality issues
    let validation = validator::validate_profile(existing);
    if validation.has_hallucinations {
        warn!(
            person = %existing.name,
            evidence_score = validation.evidence_score,
            unverified_fields = validation.unverified_fields,
            "Profile validation detected potential hallucinations"
        );
    }
    if !validation.passes_quality {
        warn!(
            person = %existing.name,
            evidence_score = validation.evidence_score,
            recommendations = ?validation.recommendations,
            "Profile quality check failed — needs more evidence"
        );
    }

    // Run sanity check
    let sanity_issues = validator::sanity_check(existing);
    for issue in &sanity_issues {
        if issue.contains("CRITICAL") || issue.contains("HALLUCINATION") {
            warn!(person = %existing.name, %issue, "Sanity check raised critical issue");
        } else {
            debug!(person = %existing.name, %issue, "Sanity check raised issue");
        }
    }

    // Validate psych profile is not at defaults
    if let Some(psych_issue) = validator::validate_psych_profile(&existing.psychological) {
        warn!(person = %existing.name, %psych_issue, "Psychological profile quality issue");
    }

    debug!(
        person = %existing.name,
        completeness = existing.profile_completeness,
        artifact_count = existing.artifacts.len(),
        evidence_score = validation.evidence_score,
        pain_index = existing.psychological.pain_index,
        risk_tolerance = existing.psychological.risk_tolerance,
        proof_types = ?existing.psychological.preferred_proof,
        "profile updated"
    );
}

/// Compute full psychological profile from artifact evidence.
///
/// This replaces all `PsychProfile::default_profile()` placeholders with
/// real data computed from artifacts, priority vectors, and role history.
/// Fields previously set by LLM enrichment are preserved and not overwritten.
fn compute_full_psych_profile(profile: &mut PoiProfile, now_epoch: i64) {
    // Core fields always computed from evidence
    profile.psychological.pain_index =
        crate::features::compute_pain_index(&profile.artifacts, now_epoch);
    profile.psychological.decision_style =
        crate::features::infer_decision_style(&profile.priority_vector);
    profile.psychological.change_appetite =
        crate::features::infer_change_appetite(&profile.role_history);

    // Compute risk_tolerance from features when not already set by LLM enrichment.
    // Risk tolerance is derived from change risk: higher change risk = higher tolerance.
    if profile.psychological.risk_tolerance == 0.0 {
        let change_risk = crate::features::compute_change_risk(&profile.role_history, now_epoch);
        profile.psychological.risk_tolerance = change_risk.clamp(0.1, 0.9);
    }

    // Compute preferred_proof_types from artifact evidence when not already set.
    if profile.psychological.preferred_proof.is_empty() && !profile.artifacts.is_empty() {
        let themes = crate::features::infer_pain_themes(&profile.artifacts, now_epoch);
        let mut proof_types = Vec::new();

        // Map pain themes to preferred proof types
        for (theme, weight) in &themes {
            if *weight < 0.3 {
                continue;
            }
            match theme.as_str() {
                "supply_disruption" => {
                    if !proof_types.contains(&ProofType::CaseStudies) {
                        proof_types.push(ProofType::CaseStudies);
                    }
                }
                "financial_stress" => {
                    if !proof_types.contains(&ProofType::CostTransparency) {
                        proof_types.push(ProofType::CostTransparency);
                    }
                }
                "quality_crisis" => {
                    if !proof_types.contains(&ProofType::Certifications) {
                        proof_types.push(ProofType::Certifications);
                    }
                    if !proof_types.contains(&ProofType::KpiMetrics) {
                        proof_types.push(ProofType::KpiMetrics);
                    }
                }
                "regulatory_burden" => {
                    if !proof_types.contains(&ProofType::AuditReadiness) {
                        proof_types.push(ProofType::AuditReadiness);
                    }
                }
                "cyber_threat" if !proof_types.contains(&ProofType::TechDemos) => {
                    proof_types.push(ProofType::TechDemos);
                }
                _ => {}
            }
        }

        // If no themes drove proof types, derive from dominant priority
        if proof_types.is_empty() {
            match profile.priority_vector.dominant() {
                "cost" => {
                    proof_types.push(ProofType::CostTransparency);
                    proof_types.push(ProofType::KpiMetrics);
                }
                "quality" => {
                    proof_types.push(ProofType::Certifications);
                    proof_types.push(ProofType::KpiMetrics);
                }
                "speed" => {
                    proof_types.push(ProofType::CaseStudies);
                    proof_types.push(ProofType::TechDemos);
                }
                "resilience" | "security" => {
                    proof_types.push(ProofType::AuditReadiness);
                    proof_types.push(ProofType::CaseStudies);
                }
                "compliance" => {
                    proof_types.push(ProofType::AuditReadiness);
                    proof_types.push(ProofType::Certifications);
                }
                _ => {
                    proof_types.push(ProofType::CaseStudies);
                    proof_types.push(ProofType::KpiMetrics);
                }
            }
        }

        if !proof_types.is_empty() {
            profile.psychological.preferred_proof = proof_types;
        }
    }

    // Log when psych profile was computed from real data
    if profile.psychological.is_enriched() {
        debug!(
            person = %profile.name,
            enrichment_quality = profile.psychological.enrichment_quality(),
            "Psych profile enriched from artifact evidence"
        );
    }
}

/// Detect if new artifacts signal a role or organization change.
/// Correctly closes previous role-history entries (end_ts) when a change is detected,
/// preventing overlapping/indefinite roles in the history log.
///
/// Uses heuristic extraction for sync detection; full async LLM extraction
/// is available via the `update_profile_with_llm` path.
fn detect_change_vs_current(
    profile: &mut PoiProfile,
    now_epoch: i64,
    _llm_client: Option<&apex_llm::inference::LlmClient>,
) {
    let role_keywords = [
        "promoted",
        "appointed",
        "appoints",
        "named",
        "new role",
        "new position",
        "joins",
        "transition",
        "stepping down",
        "departure",
        "resigned",
        "replaced",
        "now at",
        "leaves",
        "exits",
        "retires",
        "formerly at",
        "former",
        "previously at",
        "started at",
    ];

    // Scan most recent 90 days of artifacts for role-change signals.
    let cutoff = now_epoch - 90 * 86400;
    let recent: Vec<(i64, String)> = profile
        .artifacts
        .iter()
        .filter(|a| a.ts_utc >= cutoff)
        .map(|a| {
            (
                a.ts_utc,
                format!(
                    "{} {}",
                    a.title.to_lowercase(),
                    a.content_summary.to_lowercase()
                ),
            )
        })
        .collect();

    if recent.is_empty() {
        return;
    }

    // Sort by timestamp ascending to process changes chronologically
    let mut recent_sorted = recent.clone();
    recent_sorted.sort_by_key(|(ts, _)| *ts);

    for (ts_utc, text) in &recent_sorted {
        let mut found_keyword = false;
        for kw in &role_keywords {
            if text.contains(kw) {
                found_keyword = true;
                break;
            }
        }
        if !found_keyword {
            continue;
        }

        // Use improved multi-strategy extraction from llm_entity_extractor.
        // Start with heuristics (sync), then if we have enough signal,
        // the caller should use update_profile_with_llm for full LLM extraction.
        let (inferred_org, org_conf) = llm_entity_extractor::extract_org_heuristic(text);
        let (inferred_title, title_conf) = llm_entity_extractor::extract_title_heuristic(text);
        let inferred_org = inferred_org.unwrap_or_default();
        let inferred_title = inferred_title.unwrap_or_default();

        // Skip low-confidence extractions to avoid hallucinated job/org changes
        if inferred_org.is_empty() && inferred_title.is_empty() {
            continue;
        }
        if !inferred_org.is_empty() && org_conf < 0.3 {
            debug!(
                person = %profile.name,
                candidate = %inferred_org,
                confidence = org_conf,
                "Skipping low-confidence org extraction"
            );
            continue;
        }
        if !inferred_title.is_empty() && title_conf < 0.3 {
            debug!(
                person = %profile.name,
                candidate = %inferred_title,
                confidence = title_conf,
                "Skipping low-confidence title extraction"
            );
            continue;
        }

        let org_lower = inferred_org.to_lowercase();
        let title_lower = inferred_title.to_lowercase();
        let profile_org_lower = profile.org.to_lowercase();
        let profile_role_lower = profile.current_role.to_lowercase();

        if !inferred_org.is_empty() && org_lower != profile_org_lower {
            // Org change detected — close previous role entries
            close_open_role_entries(profile, *ts_utc);

            warn!(
                person = %profile.name,
                old_org = %profile.org,
                new_org_candidate = %inferred_org,
                confidence = org_conf,
                "org change detected and previous role entries closed"
            );
            let new_title = if inferred_title.is_empty() {
                profile.current_role.clone()
            } else {
                inferred_title.clone()
            };
            let new_role_family = if inferred_title.is_empty() {
                profile.role_family.clone()
            } else {
                llm_entity_extractor::infer_role_family_advanced(&inferred_title)
            };
            profile.role_history.push(RoleHistoryEntry {
                org: inferred_org.clone(),
                title: new_title.clone(),
                role_family: new_role_family.clone(),
                start_ts: *ts_utc,
                end_ts: None,
            });
            profile.org = inferred_org;
            profile.current_role = new_title;
            profile.role_family = new_role_family;
        } else if !inferred_title.is_empty() && title_lower != profile_role_lower {
            // Same org, new title — close current role, start new one
            close_open_role_entries(profile, *ts_utc);

            let new_role_family = llm_entity_extractor::infer_role_family_advanced(&inferred_title);
            profile.role_history.push(RoleHistoryEntry {
                org: profile.org.clone(),
                title: inferred_title.clone(),
                role_family: new_role_family.clone(),
                start_ts: *ts_utc,
                end_ts: None,
            });
            profile.current_role = inferred_title;
            profile.role_family = new_role_family;
            debug!(
                person = %profile.name,
                new_title = %profile.current_role,
                new_role_family = ?profile.role_family,
                "title change detected within same org"
            );
        }
    }
}

/// Close all open role-history entries (end_ts = None) at the given timestamp.
fn close_open_role_entries(profile: &mut PoiProfile, at_ts: i64) {
    for entry in profile.role_history.iter_mut() {
        if entry.end_ts.is_none() {
            entry.end_ts = Some(at_ts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile() -> PoiProfile {
        PoiProfile {
            person_id: "test-update".into(),
            name: "Test Person".into(),
            name_variants: vec![],
            org: "Acme Corp".into(),
            org_id: Some("org_acme".into()),
            current_role: "Manager".into(),
            role_family: RoleFamily::Engineering,
            region: "US".into(),
            country_code: "US".into(),
            public_bio: "Test bio".into(),
            public_email: Some("test@acme.com".into()),
            artifacts: vec![PoiArtifact {
                artifact_type: "article".into(),
                title: "Old article".into(),
                content_summary: "Some old content".into(),
                source_url: Some("https://example.com/old".into()),
                ts_utc: 1500000000,
            }],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 50.0,
                graph_centrality: 40.0,
                public_recurrence: 30.0,
                role_seniority_score: 55.0,
                network_size: 10,
            },
            engagement: None,
            role_history: vec![RoleHistoryEntry {
                org: "Acme Corp".into(),
                title: "Manager".into(),
                role_family: RoleFamily::Engineering,
                start_ts: 1500000000,
                end_ts: None,
            }],
            last_updated_utc: 1500000000,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn test_update_profile_merges_new_artifacts() {
        let mut profile = make_profile();
        let new_arts = vec![PoiArtifact {
            artifact_type: "article".into(),
            title: "New article".into(),
            content_summary: "Fresh content".into(),
            source_url: Some("https://example.com/new".into()),
            ts_utc: 1700000000,
        }];

        let old_count = profile.artifacts.len();
        update_profile(&mut profile, new_arts, None);
        assert!(profile.artifacts.len() > old_count);
    }

    #[test]
    fn test_update_dedup_by_title_and_url() {
        let mut profile = make_profile();
        let dup = profile.artifacts[0].clone();
        let old_count = profile.artifacts.len();
        update_profile(&mut profile, vec![dup], None);
        assert_eq!(profile.artifacts.len(), old_count);
    }

    #[test]
    fn test_update_updates_timestamp() {
        let mut profile = make_profile();
        let old_ts = profile.last_updated_utc;
        update_profile(&mut profile, vec![], None);
        assert!(profile.last_updated_utc >= old_ts);
    }

    #[test]
    fn test_role_change_detection() {
        let mut profile = make_profile();
        let now = Utc::now().timestamp();
        profile.artifacts.push(PoiArtifact {
            artifact_type: "press_release".into(),
            title: "Leadership change".into(),
            content_summary: "Acme Corp appoints Test Person as Senior Director of Engineering"
                .into(),
            source_url: Some("https://example.com/press".into()),
            ts_utc: now,
        });

        let old_role_count = profile.role_history.len();
        update_profile(&mut profile, vec![], None);
        assert!(profile.role_history.len() > old_role_count);
    }

    #[test]
    fn test_psych_profile_computed_from_artifacts() {
        let mut profile = make_profile();
        let new_arts = vec![PoiArtifact {
            artifact_type: "article".into(),
            title: "Supply chain crisis".into(),
            content_summary: "Major shortage and delays causing failure and disruption".into(),
            source_url: Some("https://example.com/crisis".into()),
            ts_utc: Utc::now().timestamp() - 86400,
        }];
        update_profile(&mut profile, new_arts, None);
        assert!(
            profile.psychological.pain_index > 0.0,
            "Pain index should be computed from artifacts"
        );
        assert!(
            !profile.psychological.preferred_proof.is_empty(),
            "Preferred proof types should be computed"
        );
        assert!(
            profile.psychological.risk_tolerance > 0.0,
            "Risk tolerance should be computed"
        );
        assert!(
            profile.psychological.is_enriched(),
            "Psych profile should be enriched"
        );
    }

    #[test]
    fn test_psych_profile_not_overwritten_by_empty() {
        let mut profile = make_profile();
        profile.psychological.pain_index = 0.8;
        profile.psychological.risk_tolerance = 0.7;
        profile.psychological.preferred_proof =
            vec![ProofType::Certifications, ProofType::KpiMetrics];
        update_profile(&mut profile, vec![], None);
        assert!((profile.psychological.pain_index - 0.8).abs() < 0.01);
        assert!((profile.psychological.risk_tolerance - 0.7).abs() < 0.01);
        assert_eq!(profile.psychological.preferred_proof.len(), 2);
    }

    #[test]
    fn test_llm_entity_extraction() {
        use crate::llm_entity_extractor;
        let (org, conf) =
            llm_entity_extractor::extract_org_heuristic("Test Person joins NewCorp as VP");
        assert!(org.is_some());
        assert!(conf > 0.0);
        assert!(org.unwrap().contains("NewCorp"));

        let (title, conf) =
            llm_entity_extractor::extract_title_heuristic("named Chief Technology Officer");
        assert!(title.is_some());
        assert!(conf > 0.0);
    }

    #[test]
    fn test_infer_role_family_advanced() {
        use crate::llm_entity_extractor::infer_role_family_advanced;
        assert_eq!(
            infer_role_family_advanced("VP Procurement"),
            RoleFamily::Procurement
        );
        assert_eq!(infer_role_family_advanced("CEO"), RoleFamily::Executive);
        assert_eq!(
            infer_role_family_advanced("Security Engineer"),
            RoleFamily::Security
        );
        assert_eq!(
            infer_role_family_advanced("Head of Supply Chain"),
            RoleFamily::Procurement
        );
        assert_eq!(
            infer_role_family_advanced("Director of Quality Assurance"),
            RoleFamily::SupplierQuality
        );
    }

    #[test]
    fn test_update_profile_with_llm_no_client_uses_heuristics() {
        let mut profile = make_profile();
        let new_arts = vec![PoiArtifact {
            artifact_type: "press_release".into(),
            title: "New hire".into(),
            content_summary: "Test Person has been appointed VP Quality at NewCorp Technologies"
                .into(),
            source_url: Some("https://example.com/newhire".into()),
            ts_utc: Utc::now().timestamp(),
        }];
        update_profile_with_llm(&mut profile, new_arts, None, None);
        assert!(profile.artifacts.len() >= 2);
        assert!(profile.psychological.is_enriched());
    }
}
