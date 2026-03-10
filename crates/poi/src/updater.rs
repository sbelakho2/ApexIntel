//! POI profile updater — computes derived fields and freshness.

use crate::features;
use crate::model::*;

fn metric_changed(previous: f64, current: f64, epsilon: f64) -> bool {
    if !previous.is_finite() || !current.is_finite() {
        return previous.to_bits() != current.to_bits();
    }
    (current - previous).abs() > epsilon
}

/// Update report for a POI refresh cycle.
#[derive(Debug, Clone)]
pub struct PoiUpdateReport {
    pub person_id: String,
    pub fields_updated: Vec<String>,
    pub new_artifacts: usize,
    pub influence_delta: f64,
    pub pain_index_delta: f64,
    pub role_changed: bool,
    /// Detailed role change event, if detected.
    pub role_change_event: Option<RoleChangeEvent>,
}

/// A detected role/job change for persistent tracking.
#[derive(Debug, Clone)]
pub struct RoleChangeEvent {
    pub person_id: String,
    pub old_org: String,
    pub new_org: String,
    pub old_title: String,
    pub new_title: String,
    pub old_role_family: String,
    pub new_role_family: String,
    pub change_type: RoleChangeType,
    pub confidence: f64,
}

/// Categorize the type of role change.
#[derive(Debug, Clone, PartialEq)]
pub enum RoleChangeType {
    /// Moved to a different organization
    OrgChange,
    /// Changed title within same org
    RoleChange,
    /// Changed both org and title
    JobChange,
}

/// Recompute all derived fields on a POI profile.
pub fn refresh_profile(profile: &mut PoiProfile, now_utc: i64) -> PoiUpdateReport {
    let mut fields_updated = Vec::new();
    let old_influence = profile.influence.influence_score;
    let old_pain = profile.psychological.pain_index;

    // 1. Recompute priority vector — compare all 7 dimensions
    let new_pv = features::compute_priority_vector(&profile.artifacts);
    let pv_changed = metric_changed(profile.priority_vector.cost, new_pv.cost, 0.01)
        || metric_changed(profile.priority_vector.quality, new_pv.quality, 0.01)
        || metric_changed(profile.priority_vector.speed, new_pv.speed, 0.01)
        || metric_changed(profile.priority_vector.resilience, new_pv.resilience, 0.01)
        || metric_changed(profile.priority_vector.compliance, new_pv.compliance, 0.01)
        || metric_changed(profile.priority_vector.security, new_pv.security, 0.01)
        || metric_changed(profile.priority_vector.confidence, new_pv.confidence, 0.01);
    if pv_changed {
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
    if metric_changed(profile.psychological.pain_index, new_pain, 0.01) {
        fields_updated.push("pain_index".to_string());
    }
    profile.psychological.pain_index = new_pain;

    // 4. Recompute role seniority
    let seniority = features::role_seniority_score(&profile.current_role);
    if metric_changed(profile.influence.role_seniority_score, seniority, 0.01) {
        fields_updated.push("role_seniority".to_string());
    }
    profile.influence.role_seniority_score = seniority;

    // 5. Recompute influence score
    let new_influence = features::compute_influence_score(
        profile.influence.graph_centrality,
        profile.influence.role_seniority_score,
        profile.influence.public_recurrence,
    );
    if metric_changed(profile.influence.influence_score, new_influence, 0.01) {
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
    let role_change_event = if role_changed {
        build_role_change_event(&profile.person_id, &profile.role_history)
    } else {
        None
    };

    PoiUpdateReport {
        person_id: profile.person_id.clone(),
        fields_updated,
        new_artifacts: 0,
        influence_delta: new_influence - old_influence,
        pain_index_delta: new_pain - old_pain,
        role_changed,
        role_change_event,
    }
}

/// Detect if the most recent role history entry represents a change.
pub fn detect_role_change(history: &[RoleHistoryEntry]) -> bool {
    if history.len() < 2 {
        return false;
    }
    let last = &history[history.len() - 1];
    let prev = &history[history.len() - 2];
    last.org != prev.org || last.title != prev.title
}

/// Build a detailed role change event from the last two history entries.
fn build_role_change_event(
    person_id: &str,
    history: &[RoleHistoryEntry],
) -> Option<RoleChangeEvent> {
    if history.len() < 2 {
        return None;
    }
    let last = &history[history.len() - 1];
    let prev = &history[history.len() - 2];

    let org_changed = last.org != prev.org;
    let title_changed = last.title != prev.title;

    if !org_changed && !title_changed {
        return None;
    }

    let change_type = match (org_changed, title_changed) {
        (true, true) => RoleChangeType::JobChange,
        (true, false) => RoleChangeType::OrgChange,
        (false, true) => RoleChangeType::RoleChange,
        (false, false) => unreachable!(),
    };

    Some(RoleChangeEvent {
        person_id: person_id.to_string(),
        old_org: prev.org.clone(),
        new_org: last.org.clone(),
        old_title: prev.title.clone(),
        new_title: last.title.clone(),
        old_role_family: prev.role_family.canonical_label().to_string(),
        new_role_family: last.role_family.canonical_label().to_string(),
        change_type,
        confidence: 0.8,
    })
}

/// Compare a person's current DB role against their profile to detect a job change
/// that hasn't been recorded yet. This is for the worker pipeline to call.
pub fn detect_change_vs_current(
    current_org: &str,
    current_title: &str,
    new_org: &str,
    new_title: &str,
    person_id: &str,
) -> Option<RoleChangeEvent> {
    let org_changed = current_org != new_org && !new_org.is_empty();
    let title_changed = current_title != new_title && !new_title.is_empty();

    if !org_changed && !title_changed {
        return None;
    }

    let change_type = match (org_changed, title_changed) {
        (true, true) => RoleChangeType::JobChange,
        (true, false) => RoleChangeType::OrgChange,
        (false, true) => RoleChangeType::RoleChange,
        (false, false) => unreachable!(),
    };

    Some(RoleChangeEvent {
        person_id: person_id.to_string(),
        old_org: current_org.to_string(),
        new_org: new_org.to_string(),
        old_title: current_title.to_string(),
        new_title: new_title.to_string(),
        old_role_family: String::new(),
        new_role_family: String::new(),
        change_type,
        confidence: 0.7,
    })
}

/// Compute data freshness in days.
pub fn data_freshness_days(last_updated_utc: i64, now_utc: i64) -> i32 {
    ((now_utc - last_updated_utc) / 86400).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Determine if a profile needs refresh based on staleness.
pub fn needs_refresh(profile: &PoiProfile, now_utc: i64, max_stale_days: i32) -> bool {
    if max_stale_days < 0 {
        return false;
    }
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
            artifacts: vec![PoiArtifact {
                artifact_type: "article".to_string(),
                title: "Cost savings approach".to_string(),
                content_summary: "Budget and price optimization".to_string(),
                source_url: None,
                ts_utc: 1700000000,
            }],
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
        assert!(report.role_change_event.is_some());
        let evt = report.role_change_event.unwrap();
        assert_eq!(evt.old_org, "OldOrg");
        assert_eq!(evt.new_org, "TestOrg");
        assert_eq!(evt.change_type, RoleChangeType::JobChange);
    }

    #[test]
    fn test_refresh_no_role_change() {
        let mut p = make_profile();
        // Same org and title in both entries
        p.role_history[0].org = "TestOrg".to_string();
        p.role_history[0].title = "VP Engineering".to_string();
        let report = refresh_profile(&mut p, 1700100000);
        assert!(!report.role_changed);
        assert!(report.role_change_event.is_none());
    }

    #[test]
    fn test_detect_change_vs_current_org_change() {
        let evt = detect_change_vs_current(
            "Foxconn",
            "VP Procurement",
            "Jabil",
            "VP Procurement",
            "poi_001",
        );
        assert!(evt.is_some());
        assert_eq!(evt.unwrap().change_type, RoleChangeType::OrgChange);
    }

    #[test]
    fn test_detect_change_vs_current_role_change() {
        let evt = detect_change_vs_current(
            "Foxconn",
            "VP Procurement",
            "Foxconn",
            "SVP Supply Chain",
            "poi_001",
        );
        assert!(evt.is_some());
        assert_eq!(evt.unwrap().change_type, RoleChangeType::RoleChange);
    }

    #[test]
    fn test_detect_change_vs_current_no_change() {
        let evt = detect_change_vs_current(
            "Foxconn",
            "VP Procurement",
            "Foxconn",
            "VP Procurement",
            "poi_001",
        );
        assert!(evt.is_none());
    }

    #[test]
    fn test_data_freshness_days() {
        assert_eq!(data_freshness_days(1700000000, 1700086400), 1);
        assert_eq!(data_freshness_days(1700000000, 1700000000), 0);
    }

    #[test]
    fn test_data_freshness_days_large_span_clamped() {
        let days = data_freshness_days(i64::MIN / 2, i64::MAX / 2);
        assert_eq!(days, i32::MAX);
    }

    #[test]
    fn test_needs_refresh() {
        let p = make_profile();
        assert!(needs_refresh(&p, 1700000000, 7)); // 10M seconds > 7 days
        assert!(!needs_refresh(&p, 1690000001, 7)); // 1 second not stale
    }

    #[test]
    fn test_needs_refresh_negative_max_days_is_false() {
        let p = make_profile();
        assert!(!needs_refresh(&p, 1700000000, -1));
    }

    #[test]
    fn test_influence_score_computed() {
        let mut p = make_profile();
        refresh_profile(&mut p, 1700100000);
        // 0.3*60 + 0.4*seniority(VP) + 0.3*40
        // seniority for "VP Engineering" = 85.0
        // = 18 + 34 + 12 = 64.0
        assert!(
            (p.influence.influence_score - 64.0).abs() < 1.0,
            "Expected ~64.0, got {}",
            p.influence.influence_score
        );
    }
}
