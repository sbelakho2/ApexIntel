//! POI resolver — match and merge person entities from different sources.

use crate::model::*;
use apex_core::validation::normalize_email as normalize_email_core;
use regex::Regex;
use std::sync::LazyLock;
use tracing::{debug, warn};

/// Maximum number of profiles accepted in a single [`resolve_batch`] call.
///
/// `resolve_batch` runs an O(n²) pairwise comparison.  At `n = 10 000` that is
/// 50 million comparisons — already expensive.  Inputs beyond this ceiling are
/// silently truncated after emitting a `WARN`-level tracing event so that a
/// runaway caller cannot lock the process for minutes or exhaust the heap.
pub const MAX_RESOLVE_BATCH_SIZE: usize = 10_000;

/// Pre-compiled whitespace regex to avoid O(n²) recompilation in `normalize_name`.
static WHITESPACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s+").unwrap_or_else(|error| panic!("valid whitespace regex: {error}"))
});

/// A candidate match between two POI records.
#[derive(Debug, Clone)]
pub struct MatchCandidate {
    pub profile_a_id: String,
    pub profile_b_id: String,
    pub confidence: f64,
    pub match_reasons: Vec<String>,
}

/// A planned merge of corroborated duplicate profiles.
///
/// `survivor_id` is the profile the merged ids are rewritten onto; `merged_ids`
/// are the duplicate person ids that must not survive in warning/insight
/// entity arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonMergePlan {
    pub survivor_id: String,
    pub merged_ids: Vec<String>,
    pub confidence: f64,
    pub match_reasons: Vec<String>,
}

/// Minimum primary-name similarity for two records to be considered a name
/// match.  Below this the pair is unresolved regardless of any other signal.
const NAME_COMPATIBILITY_THRESHOLD: f64 = 0.5;

/// Minimum organization-name similarity for the employer+role corroborator.
const EMPLOYER_SIMILARITY_THRESHOLD: f64 = 0.7;

/// Minimum confidence for [`match_profiles`] to emit a candidate.
const MIN_MATCH_CONFIDENCE: f64 = 0.5;

/// Path markers that identify a URL as an individual person-profile page
/// (rather than a generic article or company page).
const PROFILE_URL_MARKERS: [&str; 7] = [
    "linkedin.com/in/",
    "linkedin.com/pub/",
    "/people/",
    "/profile/",
    "/team/",
    "/members/",
    "/author/",
];

/// Canonicalize a profile URL for cross-source comparison: lowercase, drop the
/// query string and fragment, and trim a trailing slash.  Returns `None` when
/// the URL is empty or is not a person-profile URL.
fn profile_url_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lowered = trimmed.to_ascii_lowercase();
    let without_fragment = lowered.split('#').next().unwrap_or(&lowered);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let canonical = without_query.trim_end_matches('/');
    if PROFILE_URL_MARKERS
        .iter()
        .any(|marker| canonical.contains(marker))
    {
        Some(canonical.to_string())
    } else {
        None
    }
}

/// Return a profile URL carried by both artifacts of `a` and `b`.
fn shared_profile_url(a: &PoiProfile, b: &PoiProfile) -> Option<String> {
    let a_urls: std::collections::HashSet<String> = a
        .artifacts
        .iter()
        .filter_map(|artifact| artifact.source_url.as_deref())
        .filter_map(profile_url_key)
        .collect();
    if a_urls.is_empty() {
        return None;
    }
    b.artifacts
        .iter()
        .filter_map(|artifact| artifact.source_url.as_deref())
        .filter_map(profile_url_key)
        .find(|url| a_urls.contains(url))
}

/// Whether two role titles/families are compatible for the employer+role
/// corroborator.  `RoleFamily::Other` is deliberately excluded: an unknown or
/// placeholder family is not evidence of a shared role.
fn roles_compatible(a: &PoiProfile, b: &PoiProfile) -> bool {
    let role_a = a.current_role.trim().to_lowercase();
    let role_b = b.current_role.trim().to_lowercase();
    if !role_a.is_empty() && role_a == role_b {
        return true;
    }
    !matches!(&a.role_family, RoleFamily::Other(_)) && a.role_family == b.role_family
}

/// Collect the independent identifiers that corroborate a compatible name.
///
/// The documented corroborators are:
/// - `email_match` — normalized public emails are equal and non-empty;
/// - `org_id_match` — both profiles carry the same non-empty organization id;
/// - `profile_url_match` — both profiles carry the same person-profile URL
///   (e.g. a LinkedIn `/in/` URL) among their artifacts;
/// - `employer_role_match` — organization names are similar AND the role
///   title/family is compatible.
///
/// Name similarity, name variants, country, and role family alone are NOT
/// corroborators: they are name/attribute evidence, not identifiers.
fn independent_corroborators(a: &PoiProfile, b: &PoiProfile) -> Vec<&'static str> {
    let mut corroborators = Vec::new();

    if let (Some(ea), Some(eb)) = (&a.public_email, &b.public_email) {
        let na = normalize_email(ea);
        let nb = normalize_email(eb);
        if !na.is_empty() && na == nb {
            corroborators.push("email_match");
        }
    }

    if let (Some(oa), Some(ob)) = (a.normalized_org_id(), b.normalized_org_id()) {
        if oa == ob {
            corroborators.push("org_id_match");
        }
    }

    if shared_profile_url(a, b).is_some() {
        corroborators.push("profile_url_match");
    }

    if !a.org.is_empty()
        && !b.org.is_empty()
        && org_similarity(&a.org, &b.org) > EMPLOYER_SIMILARITY_THRESHOLD
        && roles_compatible(a, b)
    {
        corroborators.push("employer_role_match");
    }

    corroborators
}

/// Resolve whether two POI profiles refer to the same person.
///
/// # Corroboration rule (#163)
///
/// A pair is eligible to merge only when BOTH conditions hold:
///
/// 1. the primary names (or a documented name variant) are compatible — name
///    similarity above [`NAME_COMPATIBILITY_THRESHOLD`]; and
/// 2. at least one independent identifier matches (see
///    [`independent_corroborators`]).
///
/// Name-only matches — identical or fuzzy names with no independent
/// corroborator — return `None` ("unresolved"), so two different people who
/// share a common name are never merged on name evidence alone.  The
/// confidence score is still computed and thresholded for callers that want a
/// stricter bar.
pub fn match_profiles(a: &PoiProfile, b: &PoiProfile) -> Option<MatchCandidate> {
    let mut score: f64 = 0.0;
    let mut reasons = Vec::new();

    // 1. Name similarity
    let name_sim = name_similarity(&a.name, &b.name);
    let mut name_compatible = false;
    if name_sim > 0.8 {
        score += 0.4;
        reasons.push("name_exact_match".to_string());
        name_compatible = true;
    } else if name_sim > NAME_COMPATIBILITY_THRESHOLD {
        score += 0.2;
        reasons.push("name_fuzzy_match".to_string());
        name_compatible = true;
    }

    // 2. Check name variants — cap total contribution to prevent false positives from many variants
    let mut variant_score: f64 = 0.0;
    for va in &a.name_variants {
        for vb in &b.name_variants {
            if name_similarity(va, vb) > 0.8 {
                variant_score += 0.15;
            }
        }
    }
    let variant_contribution = variant_score.min(0.3);
    if variant_contribution > 0.0 {
        score += variant_contribution;
        reasons.push("variant_match".to_string());
        name_compatible = true;
    }

    if !name_compatible {
        return None;
    }

    // #163: independent corroboration gate.  A compatible name alone must never
    // resolve or merge two people.
    let corroborators = independent_corroborators(a, b);
    if corroborators.is_empty() {
        debug!(
            profile_a = %a.person_id,
            profile_b = %b.person_id,
            name_similarity = name_sim,
            "POI unresolved: compatible name without independent corroboration"
        );
        return None;
    }

    // 3. Same organization
    if !a.org.is_empty() && !b.org.is_empty() && org_similarity(&a.org, &b.org) > 0.7 {
        score += 0.2;
        reasons.push("same_org".to_string());
    }

    // 4. Same email (B114: normalize before comparison)
    if let (Some(ea), Some(eb)) = (&a.public_email, &b.public_email) {
        let na = normalize_email(ea);
        let nb = normalize_email(eb);
        if na == nb && !na.is_empty() {
            score += 0.4;
            reasons.push("email_match".to_string());
        } else if !na.is_empty() && !nb.is_empty() {
            // Explicitly handle conflicting emails to reduce false merges.
            score = (score - 0.2).max(0.0);
            reasons.push("email_mismatch".to_string());
        }
    }

    // 4b. Same organization id — a strong independent identifier
    if let (Some(oa), Some(ob)) = (a.normalized_org_id(), b.normalized_org_id()) {
        if oa == ob {
            score += 0.3;
            reasons.push("org_id_match".to_string());
        }
    }

    // 4c. Shared person-profile URL — a strong independent identifier
    if shared_profile_url(a, b).is_some() {
        score += 0.35;
        reasons.push("profile_url_match".to_string());
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

    if score >= MIN_MATCH_CONFIDENCE {
        // B125: Log merge decisions for observability
        debug!(
            profile_a = %a.person_id,
            profile_b = %b.person_id,
            confidence = score.min(1.0),
            corroborators = ?corroborators,
            reasons = ?reasons,
            "POI merge candidate identified"
        );
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

/// Plan person merges for a batch of profiles.
///
/// Runs the corroborated resolver over the batch and, for every cluster with
/// more than one profile, emits one [`PersonMergePlan`] whose survivor is the
/// lowest-indexed profile in the cluster (callers should feed profiles in a
/// deterministic order).  Only pairs that passed [`match_profiles`] can appear
/// in a cluster, so a planned merge always carries independent corroboration.
///
/// Plans are sorted by survivor id so callers execute them deterministically.
pub fn plan_person_merges(profiles: &[PoiProfile], threshold: f64) -> Vec<PersonMergePlan> {
    let clusters = resolve_batch(profiles, threshold);
    let mut plans = Vec::new();

    for cluster in clusters {
        if cluster.len() < 2 {
            continue;
        }
        let survivor_idx = cluster[0];
        let merged_ids: Vec<String> = cluster[1..]
            .iter()
            .map(|index| profiles[*index].person_id.clone())
            .collect();

        let mut confidence: f64 = 0.0;
        let mut reasons: std::collections::HashSet<String> = std::collections::HashSet::new();
        for left in 0..cluster.len() {
            for right in (left + 1)..cluster.len() {
                if let Some(candidate) =
                    match_profiles(&profiles[cluster[left]], &profiles[cluster[right]])
                {
                    confidence = confidence.max(candidate.confidence);
                    reasons.extend(candidate.match_reasons);
                }
            }
        }
        let mut match_reasons: Vec<String> = reasons.into_iter().collect();
        match_reasons.sort();

        plans.push(PersonMergePlan {
            survivor_id: profiles[survivor_idx].person_id.clone(),
            merged_ids,
            confidence: confidence.min(1.0),
            match_reasons,
        });
    }

    plans.sort_by(|left, right| left.survivor_id.cmp(&right.survivor_id));
    plans
}

/// Resolve a batch of profiles into clusters of duplicates.
///
/// Uses iterative union-find with union-by-rank and path halving to prevent
/// stack overflows on large inputs.
///
/// # Batch size limit (B286)
/// Inputs larger than [`MAX_RESOLVE_BATCH_SIZE`] are truncated: the first
/// `MAX_RESOLVE_BATCH_SIZE` profiles are processed and a `WARN` tracing event
/// is emitted.  Callers that need to process more profiles should split the
/// input into shards and merge the resulting clusters themselves.
pub fn resolve_batch(profiles: &[PoiProfile], threshold: f64) -> Vec<Vec<usize>> {
    resolve_batch_with_limit(profiles, threshold, MAX_RESOLVE_BATCH_SIZE)
}

/// Internal implementation that accepts an explicit batch size ceiling (B286).
///
/// Extracted from [`resolve_batch`] so that unit tests can exercise the
/// truncation path without spawning 10 000+ profiles (which would incur an
/// O(n²) Levenshtein scan unacceptable in CI).
fn resolve_batch_with_limit(
    profiles: &[PoiProfile],
    threshold: f64,
    max_batch_size: usize,
) -> Vec<Vec<usize>> {
    // B295: Deduplicate by person_id before truncation or processing
    let mut seen_ids = std::collections::HashSet::new();
    let mut unique_profiles: Vec<(usize, &PoiProfile)> = Vec::new();
    let mut dup_count = 0;
    for (original_idx, profile) in profiles.iter().enumerate() {
        if seen_ids.insert(profile.person_id.clone()) {
            unique_profiles.push((original_idx, profile));
        } else {
            dup_count += 1;
        }
    }
    if dup_count > 0 {
        warn!(
            duplicate_count = dup_count,
            "resolve_batch: dropped duplicate person_ids before processing"
        );
    }

    let profiles: Vec<(usize, &PoiProfile)> = if unique_profiles.len() > max_batch_size {
        warn!(
            input_len = unique_profiles.len(),
            limit = max_batch_size,
            "resolve_batch: input exceeds batch limit — truncating to limit"
        );
        unique_profiles.into_iter().take(max_batch_size).collect()
    } else {
        unique_profiles
    };
    let n = profiles.len();
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = vec![0; n];

    for i in 0..n {
        for j in (i + 1)..n {
            if let Some(m) = match_profiles(profiles[i].1, profiles[j].1) {
                if m.confidence >= threshold {
                    // Iterative path-halving find for node i
                    let mut pi = i;
                    while parent[pi] != pi {
                        parent[pi] = parent[parent[pi]];
                        pi = parent[pi];
                    }
                    // Iterative path-halving find for node j
                    let mut pj = j;
                    while parent[pj] != pj {
                        parent[pj] = parent[parent[pj]];
                        pj = parent[pj];
                    }
                    // Union by rank
                    if pi != pj {
                        match rank[pi].cmp(&rank[pj]) {
                            std::cmp::Ordering::Less => parent[pi] = pj,
                            std::cmp::Ordering::Greater => parent[pj] = pi,
                            std::cmp::Ordering::Equal => {
                                parent[pj] = pi;
                                rank[pi] += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Final full path compression pass
    for i in 0..n {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = i;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
    }

    let mut clusters: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        clusters.entry(parent[i]).or_default().push(profiles[i].0);
    }

    clusters.into_values().collect()
}

/// Normalize a name for comparison: lowercase, trim, collapse whitespace.
fn normalize_name(name: &str) -> String {
    WHITESPACE_RE.replace_all(name.trim(), " ").to_lowercase()
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
        // Pad short strings with spaces to produce valid trigrams instead of
        // inserting the whole string (which yields 0.0 Jaccard vs real trigrams).
        let padded: Vec<char> = format!(" {} ", s).chars().collect();
        for w in padded.windows(3) {
            set.insert(w.iter().collect());
        }
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
    normalize_email_core(email).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::assertions_on_constants
    )]

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
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("ahmed@foxconn.tn"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("ahmed@foxconn.tn"),
        );
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
        // Compatible (fuzzy) name + matching email, different employers: the
        // email is the sole independent corroborator and the pair must merge.
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn",
            Some("ahmed.benali@company.com"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Benali",
            "Other Corp",
            Some("ahmed.benali@company.com"),
        );
        let m = match_profiles(&a, &b);
        assert!(m.is_some());
        assert!(m
            .unwrap()
            .match_reasons
            .contains(&"email_match".to_string()));
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
            make_poi(
                "p1",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
            make_poi(
                "p2",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
            make_poi(
                "p3",
                "John Smith",
                "Samsung Korea",
                Some("john@samsung.com"),
            ),
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

    // B121: resolve_batch with large N
    #[test]
    fn test_resolve_batch_large() {
        // Use very distinct names to avoid trigram overlap
        let names = [
            "Ahmed",
            "Brigitte",
            "Chen",
            "Dmitri",
            "Esperanza",
            "François",
            "Greta",
            "Hiroshi",
            "Ingrid",
            "Javier",
            "Karim",
            "Leila",
            "Marco",
            "Nadia",
            "Oscar",
            "Priya",
            "Qasim",
            "Rosa",
            "Sven",
            "Tariq",
            "Ulrike",
            "Viktor",
            "Wendy",
            "Xiang",
            "Yuki",
            "Zara",
            "Boris",
            "Carla",
            "Dieter",
            "Elena",
            "Felix",
            "Gloria",
            "Hugo",
            "Irene",
            "Jorge",
            "Keira",
            "Ludwig",
            "Maria",
            "Niklas",
            "Olga",
            "Pablo",
            "Quinn",
            "Renata",
            "Stefan",
            "Tanya",
            "Umberto",
            "Vera",
            "Walter",
            "Xena",
            "Yusuf",
        ];
        let profiles: Vec<PoiProfile> = names
            .iter()
            .enumerate()
            .map(|(i, name)| make_poi(&format!("p{}", i), name, &format!("UniqueOrg{}", i), None))
            .collect();
        let clusters = resolve_batch(&profiles, 0.5);
        // All different → 50 clusters
        assert_eq!(clusters.len(), 50);
    }

    // B122: Org name normalization (similarity handles case + whitespace)
    #[test]
    fn test_org_similarity_normalization() {
        let a = make_poi("p1", "Ahmed", "  Foxconn  Tunisia  ", None);
        let b = make_poi("p2", "Ahmed", "foxconn tunisia", None);
        let m = match_profiles(&a, &b);
        assert!(m.is_some(), "Org name normalization should match");
    }

    // B126: Email normalization with plus tags
    #[test]
    fn test_match_email_plus_tags() {
        // Plus-tag handling depends on normalize_email_core implementation
        // At minimum, both should be lowercased and trimmed
        let na = normalize_email("Ahmed@Company.COM");
        assert_eq!(na, normalize_email("ahmed@company.com"));
    }

    #[test]
    fn test_name_similarity_short_strings() {
        assert!((name_similarity("Al", "Al") - 1.0).abs() < 1e-10);
        assert!(name_similarity("Al", "Bo") < 0.5);
    }

    #[test]
    fn test_variant_contribution_is_capped() {
        // Same employer + role corroborates the name so the variant-cap check
        // measures confidence, not the corroboration gate.
        let mut a = make_poi("p1", "same", "OrgA", None);
        let mut b = make_poi("p2", "same", "OrgA", None);
        a.name_variants = vec!["Same Variant".to_string(); 20];
        b.name_variants = vec!["Same Variant".to_string(); 20];

        let base_a = make_poi("p3", "same", "OrgA", None);
        let base_b = make_poi("p4", "same", "OrgA", None);
        let baseline = match_profiles(&base_a, &base_b).expect("baseline candidate expected");

        let m = match_profiles(&a, &b).expect("profiles should match");
        // Variant contribution should increase confidence by at most +0.3.
        let delta = m.confidence - baseline.confidence;
        assert!(
            delta <= 0.300_001,
            "variant contribution should be capped; delta={delta}"
        );
    }

    #[test]
    fn test_match_identical_profiles_mismatched_emails_penalized() {
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("ahmed@foxconn.tn"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("different@foxconn.tn"),
        );
        let m = match_profiles(&a, &b).expect("still similar enough to produce a candidate");
        assert!(m.match_reasons.contains(&"email_mismatch".to_string()));
        assert!(m.confidence < 0.8);
    }

    #[test]
    fn test_resolve_batch_threshold_above_one_no_merges() {
        let profiles = vec![
            make_poi(
                "p1",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
            make_poi(
                "p2",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
        ];
        let clusters = resolve_batch(&profiles, 1.1);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn test_resolve_batch_threshold_below_zero_merges_all_candidates() {
        let profiles = vec![
            make_poi(
                "p1",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
            make_poi(
                "p2",
                "Ahmed Ben Ali",
                "Foxconn Tunisia",
                Some("ahmed@foxconn.tn"),
            ),
        ];
        let clusters = resolve_batch(&profiles, -0.1);
        assert_eq!(clusters.len(), 1);
    }

    // B286: resolve_batch_with_limit truncates inputs that exceed the ceiling
    #[test]
    fn test_resolve_batch_truncates_at_custom_limit() {
        // 5 distinct profiles, limit = 3 → only first 3 are processed → 3 clusters
        let profiles: Vec<PoiProfile> = ["Ana", "Boris", "Carla", "Dmitri", "Eva"]
            .iter()
            .enumerate()
            .map(|(i, name)| make_poi(&format!("p{i}"), name, &format!("OrgUnique{i}"), None))
            .collect();
        let clusters = resolve_batch_with_limit(&profiles, 0.99, 3);
        // 3 singletons (all distinct names, threshold 0.99 ensures no merges)
        assert_eq!(
            clusters.len(),
            3,
            "truncation to limit=3 must produce exactly 3 singleton clusters"
        );
    }

    // B286: resolve_batch_with_limit at exactly the limit processes all entries
    #[test]
    fn test_resolve_batch_at_exact_limit_no_truncation() {
        let profiles: Vec<PoiProfile> = ["Ana", "Boris", "Carla"]
            .iter()
            .enumerate()
            .map(|(i, name)| make_poi(&format!("q{i}"), name, &format!("OrgQ{i}"), None))
            .collect();
        // limit = 3 = profiles.len() → no truncation, all three processed
        let clusters = resolve_batch_with_limit(&profiles, 0.99, 3);
        assert_eq!(clusters.len(), 3);
    }

    // B286: resolve_batch public API delegates to the same limit logic
    #[test]
    fn test_resolve_batch_max_resolve_batch_size_constant_is_published() {
        // The constant must be accessible and have a sane minimum value
        assert!(
            MAX_RESOLVE_BATCH_SIZE >= 1_000,
            "MAX_RESOLVE_BATCH_SIZE should be at least 1 000; got {MAX_RESOLVE_BATCH_SIZE}"
        );
    }

    // ── B287: empty input tests ──

    #[test]
    fn test_resolve_batch_empty_input_returns_empty_vec() {
        // resolve_batch([]) must return an empty cluster vec, not panic
        let clusters = resolve_batch(&[], 0.5);
        assert!(
            clusters.is_empty(),
            "resolve_batch([]) must return empty vec"
        );
    }

    // ── B288: boundary condition tests ──

    #[test]
    fn test_resolve_batch_confidence_exactly_at_threshold_merges() {
        // resolve_batch uses `>= threshold` — so confidence == threshold should merge
        // Two identical profiles → confidence == 1.0 ≥ any reasonable threshold
        // We check with threshold 1.0 (max possible) — identical profiles must still merge
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn", Some("ahmed@foxconn.tn"));
        let b = make_poi("p2", "Ahmed Ben Ali", "Foxconn", Some("ahmed@foxconn.tn"));
        let clusters = resolve_batch(&[a, b], 1.0);
        // Both profiles are identical → confidence = 1.0 ≥ threshold 1.0 → 1 cluster
        assert_eq!(
            clusters.len(),
            1,
            "identical profiles at threshold=1.0 must merge into one cluster"
        );
    }

    #[test]
    fn test_resolve_batch_confidence_just_below_threshold_does_not_merge() {
        // Use profiles that match below threshold → should NOT merge
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn", None); // no email
        let b = make_poi("p2", "Boris Petrov", "Foxconn", None); // completely different name
                                                                 // With threshold 0.99, no match should be above it → 2 clusters
        let clusters = resolve_batch(&[a, b], 0.99);
        assert_eq!(
            clusters.len(),
            2,
            "very distinct profiles below threshold must stay separate"
        );
    }

    #[test]
    fn test_resolve_batch_drops_duplicate_person_ids() {
        // B295: Verify that duplicate person_ids are dropped
        let dup_person1 = make_poi("dup123", "Alice Smith", "OrgA", Some("alice1@example.com"));
        let mut dup_person2 = make_poi("dup123", "Alice Jones", "OrgB", Some("alice2@example.com"));
        let unique_person = make_poi("unique456", "Bob Brown", "OrgC", Some("bob@example.com"));

        // Make sure dup_person2 has different name to avoid natural matching
        dup_person2.name = "Completely Different Name".to_string();

        let batch = vec![dup_person1, dup_person2, unique_person];
        // Use threshold that would NOT match them by name (0.99999)
        let clusters = resolve_batch(&batch, 0.99999);
        // Should have 2 clusters because dup is dropped, leaving only 2 unique profiles
        assert_eq!(
            clusters.len(),
            2,
            "resolve_batch must drop duplicate person_ids before clustering"
        );
    }

    // ── #163: corroboration rule ──

    #[test]
    fn test_same_name_different_company_stays_separate() {
        // Same name, same role family, same country, no email / no profile URL:
        // name-only evidence must never resolve these two people.
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", None);
        let b = make_poi("p2", "Ahmed Ben Ali", "Samsung Tunisia", None);

        assert!(
            match_profiles(&a, &b).is_none(),
            "identical names at different employers without an independent identifier must stay unresolved"
        );

        let clusters = resolve_batch(&[a, b], 0.5);
        assert_eq!(
            clusters.len(),
            2,
            "resolve_batch must not cluster name-only duplicates"
        );
    }

    #[test]
    fn test_same_name_matching_email_merges() {
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("Ahmed.BenAli@Example.com"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Ben Ali",
            "Foxconn Europe",
            Some("ahmed.benali@example.com"),
        );

        let candidate = match_profiles(&a, &b).expect("matching email must corroborate");
        assert!(candidate.match_reasons.contains(&"email_match".to_string()));
    }

    #[test]
    fn test_same_name_matching_profile_url_merges() {
        let profile_url = "https://www.linkedin.com/in/ahmed-ben-ali?trk=public_profile";
        let mut a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", None);
        a.artifacts.push(PoiArtifact {
            artifact_type: "profile".to_string(),
            title: "Company profile".to_string(),
            content_summary: String::new(),
            source_url: Some(profile_url.to_string()),
            ts_utc: 1_700_000_000,
        });
        let mut b = make_poi("p2", "Ahmed Ben Ali", "Foxconn Europe", None);
        b.artifacts.push(PoiArtifact {
            artifact_type: "profile".to_string(),
            title: "LinkedIn".to_string(),
            content_summary: String::new(),
            source_url: Some("https://www.linkedin.com/in/ahmed-ben-ali#experience".to_string()),
            ts_utc: 1_700_000_100,
        });

        let candidate =
            match_profiles(&a, &b).expect("shared profile URL must corroborate the name");
        assert!(candidate
            .match_reasons
            .contains(&"profile_url_match".to_string()));
    }

    #[test]
    fn test_missing_corroborators_stays_unresolved() {
        // Identical names but no employer, no email, no artifacts, and an
        // unknown role family: nothing independent corroborates the match.
        let mut a = make_poi("p1", "Maria Garcia", "", None);
        a.current_role = String::new();
        a.role_family = RoleFamily::Other("Unknown".to_string());
        let mut b = make_poi("p2", "Maria Garcia", "", None);
        b.current_role = String::new();
        b.role_family = RoleFamily::Other("Unknown".to_string());

        assert!(
            match_profiles(&a, &b).is_none(),
            "a missing corroborator set must leave the pair unresolved"
        );
    }

    #[test]
    fn test_conflicting_emails_are_not_a_corroborator() {
        // Same name but different, explicitly conflicting emails and different
        // employers: the emails veto rather than corroborate.
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("ahmed@a.com"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Ben Ali",
            "Samsung Tunisia",
            Some("ahmed@b.com"),
        );

        assert!(
            match_profiles(&a, &b).is_none(),
            "conflicting emails must not corroborate a name-only match"
        );
    }

    #[test]
    fn test_matching_org_id_corroborates() {
        let mut a = make_poi("p1", "Ahmed Ben Ali", "Foxconn", None);
        a.org_id = Some("org-42".to_string());
        let mut b = make_poi("p2", "Ahmed Ben Ali", "Foxconn", None);
        b.org_id = Some("org-42".to_string());

        let candidate = match_profiles(&a, &b).expect("shared org id must corroborate");
        assert!(candidate
            .match_reasons
            .contains(&"org_id_match".to_string()));
    }

    #[test]
    fn test_plan_person_merges_emits_corroborated_cluster() {
        let a = make_poi(
            "p1",
            "Ahmed Ben Ali",
            "Foxconn Tunisia",
            Some("ahmed@example.com"),
        );
        let b = make_poi(
            "p2",
            "Ahmed Ben Ali",
            "Foxconn Europe",
            Some("ahmed@example.com"),
        );
        let c = make_poi("p3", "John Smith", "Samsung", Some("john@example.com"));

        let plans = plan_person_merges(&[a, b, c], 0.7);
        assert_eq!(plans.len(), 1, "only the corroborated pair may merge");
        assert_eq!(plans[0].survivor_id, "p1");
        assert_eq!(plans[0].merged_ids, vec!["p2".to_string()]);
        assert!(plans[0].confidence >= 0.7);
    }

    #[test]
    fn test_plan_person_merges_skips_name_only_duplicates() {
        let a = make_poi("p1", "Ahmed Ben Ali", "Foxconn Tunisia", None);
        let b = make_poi("p2", "Ahmed Ben Ali", "Samsung Tunisia", None);

        let plans = plan_person_merges(&[a, b], 0.5);
        assert!(
            plans.is_empty(),
            "name-only duplicates must never produce a merge plan"
        );
    }
}
