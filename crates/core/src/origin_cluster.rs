//! Origin clustering for evidence independence (audit P0-11).
//!
//! Two evidence records are not independent corroboration merely because their
//! URLs have different hosts: the same story is routinely syndicated across
//! dozens of sites, republished by the same publisher under different
//! subdomains, or quoted from a single upstream wire service. Independence is
//! therefore computed over *origin clusters* built from:
//!
//! * the content hash (exact duplicates),
//! * near-duplicate title/body text within a bounded time window (timestamp),
//! * the canonical publisher,
//! * a declared syndication/quoted-upstream publisher,
//! * and, as a last resort, the record's own origin host.
//!
//! [`independent_origin_count`] is the number of clusters that carry an
//! identity. Records with no origin, publisher or content identity contribute
//! no independent origin — they can never inflate corroboration.

use std::collections::HashMap;

use crate::analysis::registrable_domain;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Maximum age gap for two near-duplicate texts to be treated as the same
/// published story.
const NEAR_DUPLICATE_WINDOW: Duration = Duration::hours(72);
/// Title token-overlap threshold for near-duplicate detection.
const TITLE_NEAR_DUPLICATE_THRESHOLD: f64 = 0.8;
/// Body token-overlap threshold for near-duplicate detection.
const BODY_NEAR_DUPLICATE_THRESHOLD: f64 = 0.6;

/// One record fed to origin clustering.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginRecord {
    /// Source id or URL for this record.
    #[serde(default)]
    pub origin: Option<String>,
    /// Canonical publisher of the record (for example `reuters` or
    /// `https://www.reuters.com`).
    #[serde(default)]
    pub canonical_publisher: Option<String>,
    /// Upstream publisher/wire this record was syndicated from or quotes
    /// (for example `reuters`).
    #[serde(default)]
    pub syndication_of: Option<String>,
    /// Content hash of the record body (exact duplicate detection).
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Title text used for near-duplicate detection.
    #[serde(default)]
    pub title: Option<String>,
    /// Body text used for near-duplicate detection.
    #[serde(default)]
    pub body: Option<String>,
    /// Observation timestamp, bounding near-duplicate matches.
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
}

impl OriginRecord {
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: Some(origin.into()),
            ..Self::default()
        }
    }

    pub fn with_canonical_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.canonical_publisher = Some(publisher.into());
        self
    }

    pub fn with_syndication_of(mut self, upstream: impl Into<String>) -> Self {
        self.syndication_of = Some(upstream.into());
        self
    }

    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }

    pub fn with_text(mut self, title: impl Into<String>, body: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self.body = Some(body.into());
        self
    }

    pub fn with_observed_at(mut self, observed_at: DateTime<Utc>) -> Self {
        self.observed_at = Some(observed_at);
        self
    }
}

/// A group of records that share one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginCluster {
    /// Cluster identity: canonical publisher, syndication upstream, content
    /// hash or origin host. `None` for a record with no identity at all.
    pub key: Option<String>,
    /// Indices into the input slice, in ascending order.
    pub record_indices: Vec<usize>,
}

impl OriginCluster {
    pub fn len(&self) -> usize {
        self.record_indices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.record_indices.is_empty()
    }
}

/// Normalize a publisher value (`Reuters`, `https://www.reuters.com/x`,
/// `reuters.com`) into a comparable key.
pub fn normalize_publisher(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let host = match url::Url::parse(trimmed) {
        Ok(parsed) => parsed.host_str().map(str::to_string),
        Err(_) => {
            let cut = trimmed
                .find(['/', '?', '#'])
                .map(|index| &trimmed[..index])
                .unwrap_or(trimmed);
            Some(
                cut.rsplit_once(':')
                    .map(|(host, _)| host)
                    .unwrap_or(cut)
                    .to_string(),
            )
        }
    }?;
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// Normalize an origin (URL or source id) into a comparable host/id key.
pub fn normalize_origin(value: &str) -> Option<String> {
    normalize_publisher(value)
}

fn token_set(text: &str) -> std::collections::HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn jaccard(
    left: &std::collections::HashSet<String>,
    right: &std::collections::HashSet<String>,
) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = left.union(right).count() as f64;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn within_window(left: Option<DateTime<Utc>>, right: Option<DateTime<Utc>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() <= NEAR_DUPLICATE_WINDOW,
        // Timestamps are supporting evidence: when one is missing the
        // content/publisher signals still decide.
        _ => true,
    }
}

fn near_duplicate(left: &OriginRecord, right: &OriginRecord) -> bool {
    if !within_window(left.observed_at, right.observed_at) {
        return false;
    }
    let title_similarity = match (left.title.as_deref(), right.title.as_deref()) {
        (Some(left_title), Some(right_title)) => {
            jaccard(&token_set(left_title), &token_set(right_title))
        }
        _ => 0.0,
    };
    if title_similarity >= TITLE_NEAR_DUPLICATE_THRESHOLD {
        return true;
    }
    match (left.body.as_deref(), right.body.as_deref()) {
        (Some(left_body), Some(right_body)) => {
            jaccard(&token_set(left_body), &token_set(right_body)) >= BODY_NEAR_DUPLICATE_THRESHOLD
        }
        _ => false,
    }
}

/// Same-publisher identity at the registrable-domain level: the fallback
/// identity is the eTLD+1 of the origin host, so subdomains of one publisher
/// (news.example.com, investor.example.com, blog.example.com) are one origin
/// unless a stronger signal separates them.
fn same_origin(left: &OriginRecord, right: &OriginRecord) -> bool {
    match (
        left.origin.as_deref().and_then(normalize_origin),
        right.origin.as_deref().and_then(normalize_origin),
    ) {
        (Some(left), Some(right)) => {
            let left_domain = registrable_domain(&left).unwrap_or(left);
            let right_domain = registrable_domain(&right).unwrap_or(right);
            left_domain == right_domain
        }
        _ => false,
    }
}

/// Syndication/quoted-upstream link: one record's declared upstream publisher
/// is the other record's canonical publisher or origin host (or vice versa).
fn syndication_link(left: &OriginRecord, right: &OriginRecord) -> bool {
    let left_upstream = left.syndication_of.as_deref().and_then(normalize_publisher);
    let right_upstream = right
        .syndication_of
        .as_deref()
        .and_then(normalize_publisher);
    let left_publisher = left
        .canonical_publisher
        .as_deref()
        .and_then(normalize_publisher);
    let right_publisher = right
        .canonical_publisher
        .as_deref()
        .and_then(normalize_publisher);
    let left_origin = left.origin.as_deref().and_then(normalize_origin);
    let right_origin = right.origin.as_deref().and_then(normalize_origin);
    let matches = |upstream: &Option<String>, target: &Option<String>| matches!((upstream, target), (Some(upstream), Some(target)) if upstream == target);
    matches(&left_upstream, &right_publisher)
        || matches(&left_upstream, &right_origin)
        || matches(&right_upstream, &left_publisher)
        || matches(&right_upstream, &left_origin)
}

/// True when two records belong to the same origin cluster.
pub fn same_cluster(left: &OriginRecord, right: &OriginRecord) -> bool {
    let same_hash = match (
        left.content_hash.as_deref().map(str::trim),
        right.content_hash.as_deref().map(str::trim),
    ) {
        (Some(left_hash), Some(right_hash)) => {
            !left_hash.is_empty() && left_hash.eq_ignore_ascii_case(right_hash)
        }
        _ => false,
    };
    let same_publisher = match (
        left.canonical_publisher
            .as_deref()
            .and_then(normalize_publisher),
        right
            .canonical_publisher
            .as_deref()
            .and_then(normalize_publisher),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    };
    same_hash
        || same_publisher
        || syndication_link(left, right)
        || near_duplicate(left, right)
        || same_origin(left, right)
}

/// Cluster records by origin. The result preserves first-seen order.
pub fn cluster_origins(records: &[OriginRecord]) -> Vec<OriginCluster> {
    let mut parent: Vec<usize> = (0..records.len()).collect();

    fn find(parent: &mut [usize], index: usize) -> usize {
        let mut root = index;
        while parent[root] != root {
            root = parent[root];
        }
        let mut current = index;
        while parent[current] != root {
            let next = parent[current];
            parent[current] = root;
            current = next;
        }
        root
    }

    for left in 0..records.len() {
        for right in (left + 1)..records.len() {
            if same_cluster(&records[left], &records[right]) {
                let left_root = find(&mut parent, left);
                let right_root = find(&mut parent, right);
                if left_root != right_root {
                    parent[right_root] = left_root;
                }
            }
        }
    }

    let mut clusters: Vec<OriginCluster> = Vec::new();
    let mut cluster_index: HashMap<usize, usize> = HashMap::new();
    for index in 0..records.len() {
        let root = find(&mut parent, index);
        let position = match cluster_index.get(&root) {
            Some(position) => *position,
            None => {
                let position = clusters.len();
                clusters.push(OriginCluster {
                    key: None,
                    record_indices: Vec::new(),
                });
                cluster_index.insert(root, position);
                position
            }
        };
        clusters[position].record_indices.push(index);
    }

    for cluster in &mut clusters {
        cluster.key = cluster
            .record_indices
            .iter()
            .find_map(|&index| cluster_key(&records[index]));
    }
    clusters
}

fn cluster_key(record: &OriginRecord) -> Option<String> {
    if let Some(publisher) = record
        .canonical_publisher
        .as_deref()
        .and_then(normalize_publisher)
    {
        return Some(format!("publisher:{publisher}"));
    }
    if let Some(upstream) = record
        .syndication_of
        .as_deref()
        .and_then(normalize_publisher)
    {
        return Some(format!("upstream:{upstream}"));
    }
    if let Some(hash) = record
        .content_hash
        .as_deref()
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
    {
        return Some(format!("content:{}", hash.to_ascii_lowercase()));
    }
    record
        .origin
        .as_deref()
        .and_then(normalize_origin)
        .map(|origin| format!("origin:{}", registrable_domain(&origin).unwrap_or(origin)))
}

/// Number of independent origin clusters that carry an identity.
///
/// Unsourced records (no origin, publisher or content identity) contribute
/// nothing: they can never inflate corroboration.
pub fn independent_origin_count(records: &[OriginRecord]) -> usize {
    cluster_origins(records)
        .into_iter()
        .filter(|cluster| cluster.key.is_some())
        .count()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reuters_via_three_sites_counts_as_one_origin() {
        let now = Utc::now();
        let records = [
            OriginRecord::new("https://news.example.com/tech/reuters-story")
                .with_canonical_publisher("reuters")
                .with_syndication_of("reuters")
                .with_text(
                    "Chipmaker unveils new plant in Arizona",
                    "The chipmaker said it will invest five billion dollars in a new fabrication plant.",
                )
                .with_observed_at(now),
            OriginRecord::new("https://daily.example.org/business/reuters-story")
                .with_canonical_publisher("reuters")
                .with_text(
                    "Chipmaker unveils new plant in Arizona",
                    "The chipmaker said it will invest five billion dollars in a new fabrication plant.",
                )
                .with_observed_at(now + Duration::minutes(30)),
            OriginRecord::new("https://portal.example.net/wires/reuters-story")
                .with_syndication_of("reuters")
                .with_text(
                    "Chipmaker unveils new plant in Arizona",
                    "The chipmaker said it will invest five billion dollars in a new fabrication plant.",
                )
                .with_observed_at(now + Duration::hours(2)),
        ];

        let clusters = cluster_origins(&records);
        assert_eq!(
            clusters.len(),
            1,
            "three sites republishing one Reuters story are one origin"
        );
        assert_eq!(clusters[0].record_indices, vec![0, 1, 2]);
        assert_eq!(independent_origin_count(&records), 1);
    }

    #[test]
    fn syndication_link_clusters_even_without_publisher_labels() {
        let now = Utc::now();
        let records = [
            OriginRecord::new("https://a.example/one")
                .with_syndication_of("https://www.reuters.com/world/story")
                .with_observed_at(now),
            OriginRecord::new("https://b.example/two")
                .with_canonical_publisher("https://www.reuters.com")
                .with_observed_at(now + Duration::hours(1)),
        ];
        assert_eq!(independent_origin_count(&records), 1);
    }

    #[test]
    fn near_duplicate_content_outside_the_window_is_not_clustered() {
        let now = Utc::now();
        let records = [
            OriginRecord::new("https://a.example/one")
                .with_text(
                    "Same headline about a factory",
                    "same body text with details",
                )
                .with_observed_at(now - Duration::days(10)),
            OriginRecord::new("https://b.example/two")
                .with_text(
                    "Same headline about a factory",
                    "same body text with details",
                )
                .with_observed_at(now),
        ];
        assert_eq!(independent_origin_count(&records), 2);
    }

    #[test]
    fn identical_content_hash_clusters_regardless_of_origin() {
        let records = [
            OriginRecord::new("https://a.example/one").with_content_hash("deadbeef"),
            OriginRecord::new("https://b.example/two").with_content_hash("DEADBEEF"),
        ];
        assert_eq!(independent_origin_count(&records), 1);
    }

    #[test]
    fn distinct_origins_stay_independent() {
        let records = [
            OriginRecord::new("https://alpha.example/report"),
            OriginRecord::new("https://beta.example/report"),
            OriginRecord::new("https://gamma.example/report"),
        ];
        assert_eq!(independent_origin_count(&records), 3);
        assert_eq!(cluster_origins(&records).len(), 3);
    }

    #[test]
    fn unsourced_records_do_not_earn_independence() {
        let records = [OriginRecord::default(), OriginRecord::default()];
        assert_eq!(independent_origin_count(&records), 0);
        assert_eq!(cluster_origins(&records).len(), 2);
    }

    #[test]
    fn same_host_records_share_one_origin() {
        let records = [
            OriginRecord::new("https://news.example.com/a"),
            OriginRecord::new("https://news.example.com/b"),
        ];
        assert_eq!(independent_origin_count(&records), 1);
    }
}
