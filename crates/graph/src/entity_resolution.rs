use apex_core::company_names::normalize_company_name;
use apex_core::similarity::trigram_similarity;
use std::collections::HashMap;
use tracing::debug;

/// Default similarity threshold for entity clustering (B160).
/// A Jaccard trigram similarity of 0.6 captures common spelling variations
/// while avoiding false merges between distinct entities.
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.6;
const FALSE_POSITIVE_AUDIT_THRESHOLD: f64 = 0.85;

/// Canonical form for entity name resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalName {
    pub normalized: String,
    pub original: String,
}

/// Summary metrics for entity resolution quality monitoring.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityResolutionMetrics {
    pub total_entities: usize,
    pub cluster_count: usize,
    pub merged_pairs: usize,
    pub potential_false_positive_pairs: usize,
    pub potential_false_positive_rate: f64,
}

/// Detect duplicate entity names after normalization (B331).
///
/// Returns groups of original indices where the normalized name is identical.
/// Groups are sorted by first index for deterministic output.
pub fn duplicate_entity_name_indices(names: &[String]) -> Vec<Vec<usize>> {
    let mut by_norm: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, name) in names.iter().enumerate() {
        by_norm
            .entry(normalize_company_name(name))
            .or_default()
            .push(idx);
    }

    let mut groups: Vec<Vec<usize>> = by_norm
        .into_values()
        .filter(|indices| indices.len() > 1)
        .collect();
    groups.sort_by_key(|g| g[0]);
    groups
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
    let mut rank: Vec<u8> = vec![0; n];

    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut node = i;
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }

    fn union(parent: &mut [usize], rank: &mut [u8], i: usize, j: usize) {
        let pi = find(parent, i);
        let pj = find(parent, j);
        if pi != pj {
            if rank[pi] < rank[pj] {
                parent[pi] = pj;
            } else if rank[pi] > rank[pj] {
                parent[pj] = pi;
            } else {
                parent[pj] = pi;
                rank[pi] += 1;
            }
        }
    }

    for dup_group in duplicate_entity_name_indices(names) {
        debug!(group = ?dup_group, "Detected duplicate normalized entity names");
        let leader = dup_group[0];
        for &idx in dup_group.iter().skip(1) {
            union(&mut parent, &mut rank, leader, idx);
        }
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if normalized[i] == normalized[j]
                || trigram_similarity(&normalized[i], &normalized[j]) >= threshold
            {
                debug!(
                    i,
                    j,
                    name_i = %names[i],
                    name_j = %names[j],
                    sim = %trigram_similarity(&normalized[i], &normalized[j]),
                    "Merging entities in cluster"
                );
                union(&mut parent, &mut rank, i, j);
            }
        }
    }

    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(i);
    }

    // Collect into a stable, deterministic order:
    // - Each cluster's members are already in ascending index order (0..n loop above).
    // - Clusters themselves are sorted by their smallest (canonical) member index, so
    //   the output Vec<Vec<usize>> is the same for identical inputs regardless of
    //   HashMap internal iteration order.  This is required for snapshot tests and
    //   any downstream code that compares cluster sets (B292).
    let mut result: Vec<Vec<usize>> = clusters.into_values().collect();
    result.sort_by_key(|cluster| cluster[0]); // cluster[0] is always the minimum because we iterate 0..n

    let metrics = compute_entity_resolution_metrics(names, &result);
    debug!(
        total_entities = metrics.total_entities,
        cluster_count = metrics.cluster_count,
        merged_pairs = metrics.merged_pairs,
        potential_false_positive_pairs = metrics.potential_false_positive_pairs,
        potential_false_positive_rate = metrics.potential_false_positive_rate,
        "entity_resolution_metrics"
    );

    result
}

/// Compute resolution quality metrics for monitoring potential false positives.
///
/// A merged pair is flagged as a potential false positive when its trigram
/// similarity falls below `FALSE_POSITIVE_AUDIT_THRESHOLD`, even if it passed
/// the configured merge threshold.
pub fn compute_entity_resolution_metrics(
    names: &[String],
    clusters: &[Vec<usize>],
) -> EntityResolutionMetrics {
    let normalized: Vec<String> = names.iter().map(|n| normalize_company_name(n)).collect();
    let mut merged_pairs = 0usize;
    let mut potential_false_positive_pairs = 0usize;

    for cluster in clusters {
        if cluster.len() < 2 {
            continue;
        }
        for i in 0..cluster.len() {
            for j in (i + 1)..cluster.len() {
                let left = cluster[i];
                let right = cluster[j];
                if left >= normalized.len() || right >= normalized.len() {
                    continue;
                }
                merged_pairs += 1;
                let sim = trigram_similarity(&normalized[left], &normalized[right]);
                if sim < FALSE_POSITIVE_AUDIT_THRESHOLD {
                    potential_false_positive_pairs += 1;
                }
            }
        }
    }

    let potential_false_positive_rate = if merged_pairs == 0 {
        0.0
    } else {
        potential_false_positive_pairs as f64 / merged_pairs as f64
    };

    EntityResolutionMetrics {
        total_entities: names.len(),
        cluster_count: clusters.len(),
        merged_pairs,
        potential_false_positive_pairs,
        potential_false_positive_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_company_name() {
        // Now we only strip legal suffixes, preserving descriptive words
        assert_eq!(
            normalize_company_name("Starz Electronics SARL"),
            "starz electronics"
        );
        assert_eq!(
            normalize_company_name("Foxconn Technology Group"),
            "foxconn technology group"
        );
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
        let starz_cluster = clusters.iter().find(|c| c.contains(&0)).unwrap();
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

    #[test]
    fn test_duplicate_entity_name_indices_normalized_groups() {
        let names = vec![
            "Acme Inc.".to_string(),
            "ACME".to_string(),
            "Foxconn Technology".to_string(),
            "Foxconn Technology Group".to_string(),
            "Unrelated Corp".to_string(),
        ];
        let groups = duplicate_entity_name_indices(&names);
        assert_eq!(groups, vec![vec![0, 1]]);
    }

    // ── B159: additional normalization tests ──

    #[test]
    fn test_normalize_strips_multiple_suffixes() {
        // e.g. "Foo Inc. Corp." — both should be stripped
        assert_eq!(normalize_company_name("Foo Inc. Corp."), "foo");
    }

    #[test]
    fn test_normalize_preserves_core_name() {
        assert_eq!(
            normalize_company_name("Samsung Electronics"),
            "samsung electronics"
        );
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_company_name("  Starz   SARL  "), "starz");
    }

    // ── B160: default threshold constant ──

    #[test]
    fn test_default_similarity_threshold() {
        assert!((DEFAULT_SIMILARITY_THRESHOLD - 0.6).abs() < f64::EPSILON);
        // Using the default should cluster obvious duplicates
        let names = vec![
            "Starz Electronics SARL".to_string(),
            "Starz Electronics".to_string(),
        ];
        let clusters = cluster_entities(&names, DEFAULT_SIMILARITY_THRESHOLD);
        assert_eq!(clusters.len(), 1);
    }

    // ── B161: Unicode trigram similarity ──

    #[test]
    fn test_trigram_similarity_unicode_chinese() {
        let sim = trigram_similarity("华为技术有限公司", "华为技术公司");
        assert!(
            sim > 0.2,
            "Chinese names should have partial overlap: {}",
            sim
        );
    }

    #[test]
    fn test_trigram_similarity_unicode_korean() {
        let sim = trigram_similarity("삼성전자", "삼성전자주식회사");
        assert!(
            sim > 0.2,
            "Korean names should have partial overlap: {}",
            sim
        );
    }

    #[test]
    fn test_trigram_similarity_unicode_accented() {
        let sim = trigram_similarity("café electronics", "cafe electronics");
        // One character difference, should still be quite similar
        assert!(sim > 0.5, "Accented vs plain should be similar: {}", sim);
    }

    #[test]
    fn test_normalize_company_name_with_accents() {
        assert_eq!(normalize_company_name("Café Société S.A."), "cafe societe");
    }

    #[test]
    fn test_normalize_company_name_mixed_script_confusables() {
        let mixed = "A\u{0421}\u{041c}E Corp."; // uses Cyrillic С and М in ACME
        assert_eq!(normalize_company_name(mixed), "acme");
    }

    // ── B169: large-set performance test ──

    #[test]
    fn test_cluster_entities_large_set() {
        let mut names = Vec::new();
        for i in 0..200 {
            names.push(format!("Company_{:04}", i));
        }
        // Add some duplicates
        names.push("Company_0001".to_string());
        names.push("Company_0002".to_string());

        let clusters = cluster_entities(&names, 0.8);
        // Should complete without panic and produce reasonable clusters
        assert!(!clusters.is_empty());
        // Total indices across all clusters should equal input length
        let total: usize = clusters.iter().map(|c| c.len()).sum();
        assert_eq!(total, names.len());
    }

    // ── B170: punctuation normalization before trigrams ──

    #[test]
    fn test_punctuation_normalized_before_trigrams() {
        // normalize_company_name strips punctuation, so "A.B.C." → "abc"
        let a = normalize_company_name("A.B.C. Corp.");
        let b = normalize_company_name("ABC Corp.");
        assert_eq!(a, b, "Punctuation should be stripped before comparison");

        // Verify trigram similarity reflects this
        let sim = trigram_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < f64::EPSILON,
            "Identical after normalization"
        );
    }

    #[test]
    fn test_punctuation_in_find_best_match() {
        let candidates = vec!["A.B.C. Electronics Inc.".to_string()];
        let result = find_best_match("ABC Electronics", &candidates, 0.5);
        assert!(
            result.is_some(),
            "Should match despite punctuation differences"
        );
        assert!((result.unwrap().1 - 1.0).abs() < f64::EPSILON);
    }

    // ── B287: empty input tests ──

    #[test]
    fn test_normalize_company_name_empty_string() {
        // Empty string must return empty string, not panic
        assert_eq!(normalize_company_name(""), "");
    }

    #[test]
    fn test_trigram_similarity_empty_strings() {
        // Both empty → identical → 1.0 or both-empty special case → 0.0 is also acceptable,
        // but the important invariant is: no panic.
        let sim = trigram_similarity("", "");
        assert!((0.0..=1.0).contains(&sim));
    }

    #[test]
    fn test_trigram_similarity_one_empty_string() {
        // One empty, one not → no shared trigrams → 0.0 (or at most near 0)
        let sim = trigram_similarity("", "foxconn");
        assert!(
            sim < 0.3,
            "similarity against empty string should be near 0; got {sim}"
        );
    }

    #[test]
    fn test_trigram_similarity_spaces_only() {
        let sim = trigram_similarity("   ", "\t \n");
        assert!(sim.is_finite());
        assert!((0.0..=1.0).contains(&sim));
    }

    #[test]
    fn test_find_best_match_empty_candidates() {
        // No candidates → must return None without panic
        let result = find_best_match("Foxconn", &[], 0.5);
        assert!(result.is_none(), "empty candidate list must return None");
    }

    #[test]
    fn test_cluster_entities_empty_input() {
        // Empty name list → empty clusters, no panic
        let clusters = cluster_entities(&[], 0.6);
        assert!(
            clusters.is_empty(),
            "cluster_entities([]) must return empty vec"
        );
    }

    // B292: cluster_entities deterministic output ordering
    #[test]
    fn test_cluster_entities_output_order_is_deterministic() {
        // Build a name list that produces 3 non-overlapping clusters.
        // Calling twice from the same input must produce the same cluster order.
        let names: Vec<String> = vec![
            "Foxconn Technology".to_string(),  // cluster A
            "Samsung Electronics".to_string(), // cluster B
            "Apple Inc".to_string(),           // cluster C
        ];
        let clusters1 = cluster_entities(&names, 0.9);
        let clusters2 = cluster_entities(&names, 0.9);
        assert_eq!(
            clusters1, clusters2,
            "cluster_entities must return the same order across repeated calls"
        );
    }

    #[test]
    fn test_cluster_entities_first_cluster_has_smallest_index() {
        // After sorting by min element, the first cluster must start at index 0
        let names: Vec<String> = vec![
            "Name-A".to_string(),
            "Name-B".to_string(),
            "Name-C".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.999); // high threshold → no merges
                                                        // With no merges, every name is its own cluster, sorted by index
        let first_min = clusters.first().map(|c| c[0]).unwrap_or(0);
        assert_eq!(
            first_min, 0,
            "first cluster must have smallest canonical index"
        );
    }

    #[test]
    fn test_cluster_entities_all_identical_names() {
        let names = vec![
            "ACME Inc.".to_string(),
            "Acme".to_string(),
            "acme corp".to_string(),
            "ACME LLC".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.95);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), names.len());
    }

    #[test]
    fn test_cluster_entities_threshold_zero_merges_all() {
        let names = vec![
            "Alpha Labs".to_string(),
            "Beta Manufacturing".to_string(),
            "Gamma Holdings".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
    }

    #[test]
    fn test_cluster_entities_threshold_one_requires_exact() {
        let names = vec![
            "Acme Inc.".to_string(),
            "ACME".to_string(),
            "Acne".to_string(),
        ];
        let clusters = cluster_entities(&names, 1.0);
        assert_eq!(clusters.len(), 2);
        assert!(clusters.iter().any(|c| c == &vec![0, 1]));
        assert!(clusters.iter().any(|c| c == &vec![2]));
    }

    #[test]
    fn test_compute_entity_resolution_metrics_flags_potential_false_positives() {
        let names = vec![
            "Starz Electronics".to_string(),
            "Starz Electronik".to_string(),
            "Samsung Heavy Industries".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.5);
        let metrics = compute_entity_resolution_metrics(&names, &clusters);

        assert_eq!(metrics.total_entities, 3);
        assert!(metrics.merged_pairs >= 1);
        assert!(metrics.potential_false_positive_pairs >= 1);
        assert!(
            (0.0..=1.0).contains(&metrics.potential_false_positive_rate),
            "rate should be bounded"
        );
    }

    #[test]
    fn test_compute_entity_resolution_metrics_exact_duplicates_not_flagged() {
        let names = vec![
            "ACME INC.".to_string(),
            "Acme Inc".to_string(),
            "Unrelated Corp".to_string(),
        ];
        let clusters = cluster_entities(&names, 0.9);
        let metrics = compute_entity_resolution_metrics(&names, &clusters);

        assert_eq!(metrics.merged_pairs, 1);
        assert_eq!(metrics.potential_false_positive_pairs, 0);
        assert_eq!(metrics.potential_false_positive_rate, 0.0);
    }
}
