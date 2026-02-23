//! POI resolver — match and merge person entities from different sources.

use crate::model::*;
use regex::Regex;

/// A candidate match between two POI records.
#[derive(Debug, Clone)]
pub struct MatchCandidate {
    pub profile_a_id: String,
    pub profile_b_id: String,
    pub confidence: f64,
    pub match_reasons: Vec<String>,
}

/// Resolve whether two POI profiles refer to the same person.
pub fn match_profiles(a: &PoiProfile, b: &PoiProfile) -> Option<MatchCandidate> {
    let mut score: f64 = 0.0;
    let mut reasons = Vec::new();

    // 1. Name similarity
    let name_sim = name_similarity(&a.name, &b.name);
    if name_sim > 0.8 {
        score += 0.4;
        reasons.push("name_exact_match".to_string());
    } else if name_sim > 0.5 {
        score += 0.2;
        reasons.push("name_fuzzy_match".to_string());
    }

    // 2. Check name variants
    for va in &a.name_variants {
        for vb in &b.name_variants {
            if name_similarity(va, vb) > 0.8 {
                score += 0.15;
                reasons.push("variant_match".to_string());
            }
        }
    }

    // 3. Same organization
    if !a.org.is_empty() && !b.org.is_empty() && org_similarity(&a.org, &b.org) > 0.7 {
        score += 0.2;
        reasons.push("same_org".to_string());
    }

    // 4. Same email
    if let (Some(ea), Some(eb)) = (&a.public_email, &b.public_email) {
        if normalize_email(ea) == normalize_email(eb) {
            score += 0.4;
            reasons.push("email_match".to_string());
        }
    }

    // 5. Same role
    if a.role_family == b.role_family {
        score += 0.05;
        reasons.push("same_role_family".to_string());
    }

    // 6. Same country
    if !a.country_code.is_empty() && a.country_code == b.country_code {
        score += 0.05;
        reasons.push("same_country".to_string());
    }

    if score >= 0.5 {
        Some(MatchCandidate {
            profile_a_id: a.person_id.clone(),
            profile_b_id: b.person_id.clone(),
            confidence: score.min(1.0),
            match_reasons: reasons,
        })
    } else {
        None
    }
}

/// Resolve a batch of profiles into clusters of duplicates.
pub fn resolve_batch(profiles: &[PoiProfile], threshold: f64) -> Vec<Vec<usize>> {
    let n = profiles.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    fn union(parent: &mut [usize], i: usize, j: usize) {
        let pi = find(parent, i);
        let pj = find(parent, j);
        if pi != pj {
            parent[pi] = pj;
        }
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if let Some(m) = match_profiles(&profiles[i], &profiles[j]) {
                if m.confidence >= threshold {
                    union(&mut parent, i, j);
                }
            }
        }
    }

    let mut clusters: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(i);
    }

    clusters.into_values().collect()
}

/// Normalize a name for comparison: lowercase, trim, collapse whitespace.
fn normalize_name(name: &str) -> String {
    let re = Regex::new(r"\s+").unwrap();
    re.replace_all(name.trim(), " ").to_lowercase()
}

/// Compute name similarity using character trigrams (Jaccard).
fn name_similarity(a: &str, b: &str) -> f64 {
    let na = normalize_name(a);
    let nb = normalize_name(b);

    if na == nb {
        return 1.0;
    }
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }

    let trig_a = trigrams(&na);
    let trig_b = trigrams(&nb);

    let intersection = trig_a.intersection(&trig_b).count();
    let union = trig_a.union(&trig_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn trigrams(s: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut set = std::collections::HashSet::new();
    if chars.len() < 3 {
        set.insert(s.to_string());
        return set;
    }
    for w in chars.windows(3) {
        set.insert(w.iter().collect());
    }
    set
}

/// Compute organization name similarity.
fn org_similarity(a: &str, b: &str) -> f64 {
    name_similarity(a, b)
}

/// Normalize an email for comparison.
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_poi(id: &str, name: &str, org: &str, email: Option<&str>) -> PoiProfile {
        PoiProfile {
            person_id: id.to_string(),
            name: name.to_string(),
            name_variants: vec![],
            org: org.to_string(),
            org_id: None,
            current_role: "Engineer".to_string(),
            role_family: RoleFamily::Engineering,
            region: "TN".to_string(),
            country_code: "TN".to_string(),
            public_bio: String::new(),
            public_email: email.map(|e| e.to_string()),
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 0,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn test_match_same_person_exact() {
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", Some("ahmed@foxconn.tn"));
        let b = make_poi("p2", "Ahmed Ben Ali", "Foxconn Tunisia", Some("ahmed@foxconn.tn"));
        let m = match_profiles(&a, &b);
        assert!(m.is_some());
        assert!(m.unwrap().confidence > 0.8);
    }

    #[test]
    fn test_match_different_people() {
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn", Some("ahmed@foxconn.tn"));
        let b = make_poi("p2", "John Smith", "Samsung", Some("john@samsung.com"));
        let m = match_profiles(&a, &b);
        assert!(m.is_none());
    }

    #[test]
    fn test_match_fuzzy_name() {
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", None);
        let b = make_poi("p2", "Ahmed Benali", "Foxconn Tunisia", None);
        let m = match_profiles(&a, &b);
        assert!(m.is_some(), "Fuzzy name match should succeed");
    }

    #[test]
    fn test_match_email_only() {
        let a = make_poi("p1", "A. Ben Ali", "Foxconn", Some("ahmed.benali@company.com"));
        let b = make_poi("p2", "Ahmed Benali", "Other Corp", Some("ahmed.benali@company.com"));
        let m = match_profiles(&a, &b);
        assert!(m.is_some());
        assert!(m.unwrap().match_reasons.contains(&"email_match".to_string()));
    }

    #[test]
    fn test_name_similarity_identical() {
        assert!((name_similarity("Ahmed Ben Ali", "Ahmed Ben Ali") - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_name_similarity_different() {
        assert!(name_similarity("Ahmed Ben Ali", "John Smith") < 0.3);
    }

    #[test]
    fn test_resolve_batch() {
        let profiles = vec![
            make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", Some("ahmed@foxconn.tn")),
            make_poi("p2", "Ahmed Ben Ali", "Foxconn Tunisia", Some("ahmed@foxconn.tn")),
            make_poi("p3", "John Smith", "Samsung Korea", Some("john@samsung.com")),
        ];
        let clusters = resolve_batch(&profiles, 0.5);
        // p1 and p2 should cluster, p3 alone
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn test_resolve_batch_no_matches() {
        let profiles = vec![
            make_poi("p1", "Alice", "CompA", None),
            make_poi("p2", "Bob", "CompB", None),
            make_poi("p3", "Charlie", "CompC", None),
        ];
        let clusters = resolve_batch(&profiles, 0.8);
        assert_eq!(clusters.len(), 3);
    }

    #[test]
    fn test_match_with_variants() {
        let mut a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", None);
        a.name_variants = vec!["أحمد بن علي".to_string()];
        let mut b = make_poi("p2", "Ahmed Benali", "Foxconn Tunisia", None);
        b.name_variants = vec!["أحمد بن علي".to_string()];
        let m = match_profiles(&a, &b);
        assert!(m.is_some(), "Variant match should work");
    }
}
