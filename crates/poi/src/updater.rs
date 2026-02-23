//! POI profile updater — computes derived fields and freshness.

use crate::model::*;
use crate::features;

/// Update report for a POI refresh cycle.
#[derive(Debug, Clone)]
pub struct PoiUpdateReport {
    pub person_id: String,
    pub fields_updated: Vec<String>,
    pub new_artifacts: usize,
    pub influence_delta: f64,
    pub pain_index_delta: f64,
    pub role_changed: bool,
}

/// Recompute all derived fields on a POI profile.
pub fn refresh_profile(profile: &mut PoiProfile, now_utc: i64) -> PoiUpdateReport {
    let mut fields_updated = Vec::new();
    let old_influence = profile.influence.influence_score;
    let old_pain = profile.psychological.pain_index;

    // 1. Recompute priority vector
    let new_pv = features::compute_priority_vector(&profile.artifacts);
    if (new_pv.cost - profile.priority_vector.cost).abs() > 0.01
        || (new_pv.quality - profile.priority_vector.quality).abs() > 0.01
    {
        fields_updated.push("priority_vector".to_string());
    }
    profile.priority_vector = new_pv;

    // 2. Recompute decision style
    let new_style = features::infer_decision_style(&profile.priority_vector);
    if new_style != profile.psychological.decision_style {
        profile.psychological.decision_style = new_style;
        fields_updated.push("decision_style".to_string());
    }

    // 3. Recompute pain index
    let new_pain = features::compute_pain_index(&profile.artifacts, now_utc);
    if (new_pain - profile.psychological.pain_index).abs() > 0.01 {
        fields_updated.push("pain_index".to_string());
    }
    profile.psychological.pain_index = new_pain;

    // 4. Recompute role seniority
    let seniority = features::role_seniority_score(&profile.current_role);
    if (seniority - profile.influence.role_seniority_score).abs() > 0.01 {
        fields_updated.push("role_seniority".to_string());
    }
    profile.influence.role_seniority_score = seniority;

    // 5. Recompute influence score
    let new_influence = features::compute_influence_score(
        profile.influence.graph_centrality,
        profile.influence.role_seniority_score,
        profile.influence.public_recurrence,
    );
    if (new_influence - profile.influence.influence_score).abs() > 0.01 {
        fields_updated.push("influence_score".to_string());
    }
    profile.influence.influence_score = new_influence;

    // 6. Recompute change appetite
    let new_appetite = features::infer_change_appetite(&profile.role_history);
    if new_appetite != profile.psychological.change_appetite {
        profile.psychological.change_appetite = new_appetite;
        fields_updated.push("change_appetite".to_string());
    }

    // 7. Recompute completeness
    profile.profile_completeness = profile.compute_completeness();

    // 8. Update timestamp
    profile.last_updated_utc = now_utc;

    // 9. Check for role change
    let role_changed = detect_role_change(&profile.role_history);

    PoiUpdateReport {
        person_id: profile.person_id.clone(),
        fields_updated,
        new_artifacts: 0,
        influence_delta: new_influence - old_influence,
        pain_index_delta: new_pain - old_pain,
        role_changed,
    }
}

/// Detect if the most recent role history entry represents a change.
fn detect_role_change(history: &[RoleHistoryEntry]) -> bool {
    if history.len() < 2 {
        return false;
    }
    let last = &history[history.len() - 1];
    let prev = &history[history.len() - 2];
    last.org != prev.org || last.title != prev.title
}

/// Compute data freshness in days.
pub fn data_freshness_days(last_updated_utc: i64, now_utc: i64) -> i32 {
    ((now_utc - last_updated_utc) / 86400) as i32
}

/// Determine if a profile needs refresh based on staleness.
pub fn needs_refresh(profile: &PoiProfile, now_utc: i64, max_stale_days: i32) -> bool {
    let days = data_freshness_days(profile.last_updated_utc, now_utc);
    days >= max_stale_days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile() -> PoiProfile {
        PoiProfile {
            person_id: "poi_refresh".to_string(),
            name: "Refresh Test".to_string(),
            name_variants: vec![],
            org: "TestOrg".to_string(),
            org_id: None,
            current_role: "VP Engineering".to_string(),
            role_family: RoleFamily::Engineering,
            region: "TN".to_string(),
            country_code: "TN".to_string(),
            public_bio: "Engineer".to_string(),
            public_email: Some("test@org.com".to_string()),
            artifacts: vec![
                PoiArtifact {
                    artifact_type: "article".to_string(),
                    title: "Cost savings approach".to_string(),
                    content_summary: "Budget and price optimization".to_string(),
                    source_url: None,
                    ts_utc: 1700000000,
                },
            ],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 60.0,
                public_recurrence: 40.0,
                role_seniority_score: 0.0,
                network_size: 10,
            },
            engagement: None,
            role_history: vec![
                RoleHistoryEntry {
                    org: "OldOrg".to_string(),
                    title: "Senior Engineer".to_string(),
                    role_family: RoleFamily::Engineering,
                    start_ts: 1500000000,
                    end_ts: Some(1650000000),
                },
                RoleHistoryEntry {
                    org: "TestOrg".to_string(),
                    title: "VP Engineering".to_string(),
                    role_family: RoleFamily::Engineering,
                    start_ts: 1650000000,
                    end_ts: None,
                },
            ],
            last_updated_utc: 1690000000,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn test_refresh_profile_updates_fields() {
        let mut p = make_profile();
        let report = refresh_profile(&mut p, 1700100000);
        assert!(!report.fields_updated.is_empty());
        assert!(p.influence.influence_score > 0.0);
        assert!(p.profile_completeness > 0.0);
        assert_eq!(p.last_updated_utc, 1700100000);
    }

    #[test]
    fn test_refresh_detects_role_change() {
        let mut p = make_profile();
        let report = refresh_profile(&mut p, 1700100000);
        assert!(report.role_changed);
    }

    #[test]
    fn test_refresh_no_role_change() {
        let mut p = make_profile();
        // Same org and title in both entries
        p.role_history[0].org = "TestOrg".to_string();
        p.role_history[0].title = "VP Engineering".to_string();
        let report = refresh_profile(&mut p, 1700100000);
        assert!(!report.role_changed);
    }

    #[test]
    fn test_data_freshness_days() {
        assert_eq!(data_freshness_days(1700000000, 1700086400), 1);
        assert_eq!(data_freshness_days(1700000000, 1700000000), 0);
    }

    #[test]
    fn test_needs_refresh() {
        let p = make_profile();
        assert!(needs_refresh(&p, 1700000000, 7)); // 10M seconds > 7 days
        assert!(!needs_refresh(&p, 1690000001, 7)); // 1 second not stale
    }

    #[test]
    fn test_influence_score_computed() {
        let mut p = make_profile();
        refresh_profile(&mut p, 1700100000);
        // 0.3*60 + 0.4*seniority(VP) + 0.3*40
        // seniority for "VP Engineering" = 85.0
        // = 18 + 34 + 12 = 64.0
        assert!((p.influence.influence_score - 64.0).abs() < 1.0,
            "Expected ~64.0, got {}", p.influence.influence_score);
    }
}
