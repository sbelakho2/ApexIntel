use apex_core::company_names::normalize_company_name;
use apex_core::similarity::{jaccard_similarity, trigram_similarity};
use std::collections::HashMap;
use std::cmp::Ordering;
use std::collections::HashSet;
use tracing::debug;

/// Default similarity threshold for entity clustering (B160).
/// A Jaccard trigram similarity of 0.6 captures common spelling variations
/// while avoiding false merges between distinct entities.
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.6;
const FALSE_POSITIVE_AUDIT_THRESHOLD: f64 = 0.85;
const SHORT_NAME_LEN: usize = 8;
const BOOTSTRAP_RESAMPLES: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct EntityMatchScore {
    pub similarity: f64,
    pub trigram_jaccard: f64,
    pub token_sort_ratio: f64,
    pub phonetic_match: f64,
    pub short_name_similarity: Option<f64>,
    pub confidence_lower: f64,
    pub confidence_upper: f64,
    pub alias_matched: bool,
}

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
            .entry(canonical_company_identity(name))
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
    let norm = canonical_company_identity(name);

    let mut best: Option<(&str, f64)> = None;

    for candidate in candidates {
        let norm_candidate = canonical_company_identity(candidate);

        // Exact match after normalization
        if norm == norm_candidate {
            return Some((candidate.as_str(), 1.0));
        }

        let sim = score_entity_match(&norm, &norm_candidate).similarity;
        if sim >= threshold {
            let dominated = match best {
                Some((_, prev_sim)) => sim > prev_sim,
                None => true,
            };
            if dominated {
                best = Some((candidate.as_str(), sim));
            }
        }
    }

    best
}

fn alias_canonical_form(normalized: &str) -> Option<&'static str> {
    match normalized {
        "stmicro" => Some("stmicroelectronics"),
        "stm" => Some("stmicroelectronics"),
        "tsmc" => Some("taiwan semiconductor manufacturing"),
        "taiwan semi" => Some("taiwan semiconductor manufacturing"),
        "taiwan semiconductor" => Some("taiwan semiconductor manufacturing"),
        "nxp" => Some("nxp semiconductors"),
        "bae" => Some("bae systems"),
        "ibm" => Some("international business machines"),
        "ge" => Some("general electric"),
        _ => None,
    }
}

fn canonical_company_identity(name: &str) -> String {
    let normalized = normalize_company_name(name);
    alias_canonical_form(&normalized)
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();

    for (i, left_char) in left.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = if left_char == *right_char { 0 } else { 1 };
            current.push(
                (previous[j + 1] + 1)
                    .min(current[j] + 1)
                    .min(previous[j] + substitution_cost),
            );
        }
        previous = current;
    }

    *previous.last().unwrap_or(&0)
}

fn normalized_levenshtein_similarity(left: &str, right: &str) -> f64 {
    let max_len = left.chars().count().max(right.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - levenshtein_distance(left, right) as f64 / max_len as f64
}

fn token_sort_ratio(left: &str, right: &str) -> f64 {
    let mut left_tokens: Vec<&str> = left.split_whitespace().collect();
    let mut right_tokens: Vec<&str> = right.split_whitespace().collect();
    left_tokens.sort_unstable();
    right_tokens.sort_unstable();
    normalized_levenshtein_similarity(&left_tokens.join(" "), &right_tokens.join(" "))
}

fn soundex_code(token: &str) -> String {
    let mut chars = token.chars();
    let first = chars.next().unwrap_or('0').to_ascii_uppercase();
    let mut code = String::from(first);
    let mut previous_digit = map_soundex_digit(first);

    for ch in chars {
        let digit = map_soundex_digit(ch.to_ascii_uppercase());
        if digit != '0' && digit != previous_digit {
            code.push(digit);
        }
        previous_digit = digit;
        if code.len() == 4 {
            break;
        }
    }

    while code.len() < 4 {
        code.push('0');
    }
    code
}

fn map_soundex_digit(ch: char) -> char {
    match ch {
        'B' | 'F' | 'P' | 'V' => '1',
        'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => '2',
        'D' | 'T' => '3',
        'L' => '4',
        'M' | 'N' => '5',
        'R' => '6',
        _ => '0',
    }
}

fn phonetic_signature(name: &str) -> Vec<String> {
    let mut signature: Vec<String> = name
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(soundex_code)
        .collect();
    signature.sort();
    signature
}

fn phonetic_match_score(left: &str, right: &str) -> f64 {
    let left_signature = phonetic_signature(left);
    let right_signature = phonetic_signature(right);
    if left_signature.is_empty() && right_signature.is_empty() {
        return 1.0;
    }
    if left_signature == right_signature {
        1.0
    } else {
        0.0
    }
}

fn trigram_set(input: &str) -> HashSet<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut set = HashSet::new();

    if chars.len() < 3 {
        let padded: Vec<char> = format!(" {} ", input).chars().collect();
        for window in padded.windows(3) {
            set.insert(window.iter().collect());
        }
        return set;
    }

    for window in chars.windows(3) {
        set.insert(window.iter().collect());
    }

    set
}

fn deterministic_seed(left: &str, right: &str) -> u64 {
    let mut hash = 1469598103934665603u64;
    for byte in left.bytes().chain([0u8]).chain(right.bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211u64);
    }
    hash.max(1)
}

fn sample_with_replacement(values: &[String], seed: &mut u64) -> HashSet<String> {
    if values.is_empty() {
        return HashSet::new();
    }

    let mut sample = HashSet::new();
    for _ in 0..values.len() {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let index = (*seed as usize) % values.len();
        sample.insert(values[index].clone());
    }
    sample
}

fn bootstrap_similarity_band(left: &str, right: &str, baseline: f64) -> (f64, f64) {
    let left_trigrams: Vec<String> = trigram_set(left).into_iter().collect();
    let right_trigrams: Vec<String> = trigram_set(right).into_iter().collect();
    if left_trigrams.is_empty() || right_trigrams.is_empty() {
        return (baseline, baseline);
    }

    let mut seed = deterministic_seed(left, right);
    let mut samples = Vec::with_capacity(BOOTSTRAP_RESAMPLES);
    for _ in 0..BOOTSTRAP_RESAMPLES {
        let sampled_left = sample_with_replacement(&left_trigrams, &mut seed);
        let sampled_right = sample_with_replacement(&right_trigrams, &mut seed);
        samples.push(jaccard_similarity(&sampled_left, &sampled_right));
    }
    samples.sort_by(|left, right| left.total_cmp(right));

    let lower_index = ((BOOTSTRAP_RESAMPLES - 1) as f64 * 0.05).round() as usize;
    let upper_index = ((BOOTSTRAP_RESAMPLES - 1) as f64 * 0.95).round() as usize;
    (samples[lower_index], samples[upper_index])
}

pub fn score_entity_match(left: &str, right: &str) -> EntityMatchScore {
    let left = canonical_company_identity(left);
    let right = canonical_company_identity(right);
    let alias_matched = left == right;

    if alias_matched {
        return EntityMatchScore {
            similarity: 1.0,
            trigram_jaccard: 1.0,
            token_sort_ratio: 1.0,
            phonetic_match: 1.0,
            short_name_similarity: Some(1.0),
            confidence_lower: 1.0,
            confidence_upper: 1.0,
            alias_matched: true,
        };
    }

    let trigram_jaccard = trigram_similarity(&left, &right);
    let token_sort = token_sort_ratio(&left, &right);
    let phonetic_match = phonetic_match_score(&left, &right);
    let short_name_similarity = if left.len() < SHORT_NAME_LEN || right.len() < SHORT_NAME_LEN {
        Some(normalized_levenshtein_similarity(&left, &right))
    } else {
        None
    };

    let mut similarity = 0.6 * trigram_jaccard + 0.3 * token_sort + 0.1 * phonetic_match;
    if let Some(short_score) = short_name_similarity {
        similarity = similarity.max(0.5 * similarity + 0.5 * short_score);
    }
    let (confidence_lower, confidence_upper) = bootstrap_similarity_band(&left, &right, similarity);

    EntityMatchScore {
        similarity,
        trigram_jaccard,
        token_sort_ratio: token_sort,
        phonetic_match,
        short_name_similarity,
        confidence_lower,
        confidence_upper,
        alias_matched: false,
    }
}

fn canonical_entity_key(name: &str, original_index: usize) -> (String, String, usize) {
    (
        canonical_company_identity(name),
        name.to_string(),
        original_index,
    )
}

fn compare_entity_keys(
    left: &(String, String, usize),
    right: &(String, String, usize),
) -> Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(&right.2))
}

/// Resolve entities: given a batch of names, group them into clusters.
pub fn cluster_entities(names: &[String], threshold: f64) -> Vec<Vec<usize>> {
    let mut canonical_entities: Vec<(usize, String, String)> = names
        .iter()
        .enumerate()
        .map(|(original_index, name)| {
            (
                original_index,
                canonical_company_identity(name),
                name.clone(),
            )
        })
        .collect();
    canonical_entities.sort_by(|left, right| {
        compare_entity_keys(
            &(left.1.clone(), left.2.clone(), left.0),
            &(right.1.clone(), right.2.clone(), right.0),
        )
    });

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

    let mut duplicate_groups: Vec<Vec<usize>> = Vec::new();
    let mut start = 0usize;
    while start < canonical_entities.len() {
        let mut end = start + 1;
        while end < canonical_entities.len() && canonical_entities[end].1 == canonical_entities[start].1 {
            end += 1;
        }
        if end - start > 1 {
            duplicate_groups.push((start..end).collect());
        }
        start = end;
    }

    for dup_group in duplicate_groups {
        debug!(group = ?dup_group, "Detected duplicate normalized entity names");
        let leader = dup_group[0];
        for &idx in dup_group.iter().skip(1) {
            union(&mut parent, &mut rank, leader, idx);
        }
    }

    for i in 0..n {
        for j in (i + 1)..n {
            let normalized_i = &canonical_entities[i].1;
            let normalized_j = &canonical_entities[j].1;
            let match_score = score_entity_match(normalized_i, normalized_j);
            if normalized_i == normalized_j || match_score.similarity >= threshold {
                debug!(
                    i,
                    j,
                    name_i = %canonical_entities[i].2,
                    name_j = %canonical_entities[j].2,
                    sim = %match_score.similarity,
                    ci_low = %match_score.confidence_lower,
                    ci_high = %match_score.confidence_upper,
                    "Merging entities in cluster"
                );
                union(&mut parent, &mut rank, i, j);
            }
        }
    }

    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters
            .entry(root)
            .or_default()
            .push(canonical_entities[i].0);
    }

    // Collect into a stable, deterministic order using canonicalized member identities.
    let mut result: Vec<Vec<usize>> = clusters.into_values().collect();
    for cluster in &mut result {
        cluster.sort_unstable();
    }
    result.sort_by(|left, right| {
        let left_key = left
            .iter()
            .map(|index| canonical_entity_key(&names[*index], *index))
            .min_by(compare_entity_keys);
        let right_key = right
            .iter()
            .map(|index| canonical_entity_key(&names[*index], *index))
            .min_by(compare_entity_keys);
        match (left_key, right_key) {
            (Some(left_key), Some(right_key)) => compare_entity_keys(&left_key, &right_key),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    });

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
                let match_score = score_entity_match(&normalized[left], &normalized[right]);
                if match_score.confidence_lower < FALSE_POSITIVE_AUDIT_THRESHOLD {
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

    fn canonical_cluster_labels(names: &[String], clusters: &[Vec<usize>]) -> Vec<Vec<String>> {
        let mut labels: Vec<Vec<String>> = clusters
            .iter()
            .map(|cluster| {
                let mut members: Vec<String> = cluster
                    .iter()
                    .map(|index| normalize_company_name(&names[*index]))
                    .collect();
                members.sort();
                members
            })
            .collect();
        labels.sort();
        labels
    }

    fn pseudo_shuffle_names(names: &[String], iteration: usize) -> Vec<String> {
        let mut keyed: Vec<(u64, String)> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let key = ((index as u64 + 1) * 1_103_515_245u64)
                    .wrapping_add((iteration as u64 + 7) * 12_345u64)
                    % 4_294_967_291u64;
                (key, name.clone())
            })
            .collect();
        keyed.sort_by_key(|(key, name)| (*key, name.clone()));
        keyed.into_iter().map(|(_, name)| name).collect()
    }

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
    fn test_score_entity_match_short_names_use_levenshtein() {
        let score = score_entity_match("NXP", "NXP Semiconductors");
        assert!(score.short_name_similarity.is_some());
        assert!(score.similarity > 0.6);
    }

    #[test]
    fn test_score_entity_match_token_sort_handles_reordering() {
        let score = score_entity_match("Samsung Electronics", "Electronics Samsung");
        assert!(score.token_sort_ratio > 0.99);
        assert!(score.similarity > 0.7);
    }

    #[test]
    fn test_score_entity_match_alias_short_circuit() {
        let score = score_entity_match("STMicro", "STMicroelectronics");
        assert!(score.alias_matched);
        assert!((score.similarity - 1.0).abs() < 1e-10);
    }

    #[test]
    fn short_name_confusion_prevention() {
        let bae_bea = score_entity_match("BAE", "BEA");
        let nxp = score_entity_match("NXP", "NXP Semiconductors");
        assert!(bae_bea.similarity < DEFAULT_SIMILARITY_THRESHOLD);
        assert!(nxp.similarity >= DEFAULT_SIMILARITY_THRESHOLD);
    }

    #[test]
    fn token_reorder_invariance() {
        let score = score_entity_match("Samsung Electronics", "Electronics Samsung");
        assert!(score.token_sort_ratio > 0.99);
        assert!(score.similarity >= DEFAULT_SIMILARITY_THRESHOLD);
    }

    #[test]
    fn full_homoglyph_table() {
        assert_eq!(normalize_company_name("Rоsatом"), "rosatom");
    }

    #[test]
    fn alias_table_resolution() {
        assert!(score_entity_match("STMicro", "STMicroelectronics").alias_matched);
        assert!(score_entity_match("TSMC", "Taiwan Semiconductor").alias_matched);
    }

    #[test]
    fn union_find_valid_partition() {
        let names = vec![
            "STMicro".to_string(),
            "STMicroelectronics".to_string(),
            "TSMC".to_string(),
            "Taiwan Semiconductor".to_string(),
            "NXP".to_string(),
        ];
        let clusters = cluster_entities(&names, DEFAULT_SIMILARITY_THRESHOLD);
        let mut seen = std::collections::HashSet::new();
        for cluster in &clusters {
            for index in cluster {
                assert!(seen.insert(*index), "duplicate index in partition: {index}");
            }
        }
        assert_eq!(seen.len(), names.len());
    }

    #[test]
    fn test_score_entity_match_emits_confidence_band() {
        let score = score_entity_match("Foxconn Technology Group", "Foxconn Technology");
        assert!(score.confidence_lower <= score.confidence_upper);
        assert!((0.0..=1.0).contains(&score.confidence_lower));
        assert!((0.0..=1.0).contains(&score.confidence_upper));
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

    #[test]
    fn test_normalize_company_name_extended_homoglyphs() {
        assert_eq!(normalize_company_name("οmicron Corp"), "omicron");
        assert_eq!(normalize_company_name("Τesla"), "tesla");
        assert_eq!(normalize_company_name("Νokia"), "nokia");
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
    fn entity_resolution_ordering_invariance() {
        let names = vec![
            "Starz Electronics SARL".to_string(),
            "Foxconn Technology Group".to_string(),
            "Starz Electronics".to_string(),
            "Foxconn Technology".to_string(),
            "Samsung Electronics".to_string(),
            "ACME INC.".to_string(),
            "Acme".to_string(),
        ];
        let baseline = canonical_cluster_labels(&names, &cluster_entities(&names, 0.6));

        for iteration in 0..10 {
            let shuffled = pseudo_shuffle_names(&names, iteration);
            let shuffled_clusters = cluster_entities(&shuffled, 0.6);
            assert_eq!(
                canonical_cluster_labels(&shuffled, &shuffled_clusters),
                baseline,
                "cluster membership should be invariant across deterministic shuffles"
            );
        }
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
        assert!((metrics.potential_false_positive_rate - 0.0).abs() < 1e-10);
    }

    #[test]
    fn find_best_match_empty_candidates_returns_none() {
        let result = find_best_match("ACME Corp", &[], 0.8);
        assert!(result.is_none());
    }

    #[test]
    fn find_best_match_all_below_threshold_returns_none() {
        let candidates = vec!["Totally Different Name".to_string()];
        let result = find_best_match("ACME Corp", &candidates, 0.99);
        assert!(result.is_none());
    }

    #[test]
    fn cluster_entities_empty_input() {
        let names: Vec<String> = vec![];
        let clusters = cluster_entities(&names, 0.9);
        assert!(clusters.is_empty());
    }

    #[test]
    fn cluster_entities_single_name() {
        let names = vec!["Sole Corp".to_string()];
        let clusters = cluster_entities(&names, 0.9);
        assert_eq!(clusters.len(), 1);
    }
}
