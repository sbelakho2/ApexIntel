use std::collections::HashMap;
use regex::Regex;

/// Canonical form for entity name resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalName {
    pub normalized: String,
    pub original: String,
}

/// Normalize a company name for fuzzy matching.
pub fn normalize_company_name(name: &str) -> String {
    let lower = name.to_lowercase().trim().to_string();

    // Remove common suffixes
    let suffixes = [
        " inc.", " inc", " ltd.", " ltd", " llc", " corp.", " corp",
        " s.a.", " sa", " sarl", " s.a.r.l.", " gmbh", " ag", " sas",
        " co.", " co", " plc", " group", " holdings", " international",
        " technologies", " technology", " electronics", " manufacturing",
    ];

    let mut result = lower;
    for suffix in &suffixes {
        if result.ends_with(suffix) {
            result = result[..result.len() - suffix.len()].to_string();
        }
    }

    // Normalize whitespace + punctuation
    let re = Regex::new(r"[^\w\s]").unwrap();
    result = re.replace_all(&result, "").to_string();
    let re_ws = Regex::new(r"\s+").unwrap();
    result = re_ws.replace_all(&result, " ").trim().to_string();

    result
}

/// Compute string similarity using trigram overlap (Jaccard).
pub fn trigram_similarity(a: &str, b: &str) -> f64 {
    let trig_a = trigrams(a);
    let trig_b = trigrams(b);

    if trig_a.is_empty() && trig_b.is_empty() {
        return 0.0;
    }

    let intersection = trig_a.intersection(&trig_b).count();
    let union = trig_a.union(&trig_b).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
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

/// Match a name against a list of known entities and return the best match.
pub fn find_best_match<'a>(
    name: &str,
    candidates: &'a [String],
    threshold: f64,
) -> Option<(&'a str, f64)> {
    let norm = normalize_company_name(name);

    let mut best: Option<(&str, f64)> = None;

    for candidate in candidates {
        let norm_candidate = normalize_company_name(candidate);

        // Exact match after normalization
        if norm == norm_candidate {
            return Some((candidate.as_str(), 1.0));
        }

        let sim = trigram_similarity(&norm, &norm_candidate);
        if sim >= threshold {
            if best.is_none() || sim > best.unwrap().1 {
                best = Some((candidate.as_str(), sim));
            }
        }
    }

    best
}

/// Resolve entities: given a batch of names, group them into clusters.
pub fn cluster_entities(names: &[String], threshold: f64) -> Vec<Vec<usize>> {
    let normalized: Vec<String> = names.iter().map(|n| normalize_company_name(n)).collect();
    let n = names.len();
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
            if normalized[i] == normalized[j]
                || trigram_similarity(&normalized[i], &normalized[j]) >= threshold
            {
                union(&mut parent, i, j);
            }
        }
    }

    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(i);
    }

    clusters.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_company_name() {
        assert_eq!(normalize_company_name("Starz Electronics SARL"), "starz");
        assert_eq!(normalize_company_name("Foxconn Technology Group"), "foxconn");
        assert_eq!(normalize_company_name("Jabil Inc."), "jabil");
    }

    #[test]
    fn test_normalize_strips_punctuation() {
        assert_eq!(normalize_company_name("A.B.C. Corp."), "abc");
    }

    #[test]
    fn test_trigram_similarity_identical() {
        let sim = trigram_similarity("foxconn", "foxconn");
        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_trigram_similarity_similar() {
        let sim = trigram_similarity("starz electronics", "starz electronik");
        assert!(sim > 0.5);
    }

    #[test]
    fn test_trigram_similarity_different() {
        let sim = trigram_similarity("foxconn", "samsung");
        assert!(sim < 0.2);
    }

    #[test]
    fn test_find_best_match_exact() {
        let candidates = vec![
            "Foxconn Technology Group".to_string(),
            "Jabil Inc.".to_string(),
            "Starz Electronics SARL".to_string(),
        ];
        let (match_name, score) = find_best_match("Starz Electronics", &candidates, 0.5).unwrap();
        assert_eq!(match_name, "Starz Electronics SARL");
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_best_match_fuzzy() {
        let candidates = vec![
            "Foxconn Technology Group".to_string(),
            "Jabil Inc.".to_string(),
        ];
        let result = find_best_match("Foxconn Tech", &candidates, 0.3);
        assert!(result.is_some());
        assert!(result.unwrap().0.contains("Foxconn"));
    }

    #[test]
    fn test_find_best_match_none() {
        let candidates = vec!["Foxconn Technology Group".to_string()];
        let result = find_best_match("Samsung Electronics", &candidates, 0.8);
        assert!(result.is_none());
    }

    #[test]
    fn test_cluster_entities() {
        let names = vec![
            "Starz Electronics SARL".to_string(),
            "Starz Electronics".to_string(),
            "Foxconn Technology Group".to_string(),
            "Foxconn Technology".to_string(),
            "Samsung Electronics".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.6);

        // Starz variants should cluster, Foxconn variants should cluster, Samsung separate
        assert!(clusters.len() >= 2);

        // Find the Starz cluster
        let starz_cluster = clusters
            .iter()
            .find(|c| c.contains(&0))
            .unwrap();
        assert!(starz_cluster.contains(&1));
    }

    #[test]
    fn test_cluster_no_merges() {
        let names = vec![
            "Apple Inc".to_string(),
            "Microsoft Corp".to_string(),
            "Google LLC".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.8);
        assert_eq!(clusters.len(), 3);
    }
}
