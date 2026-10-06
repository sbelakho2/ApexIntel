//! Analytical excellence layer — the "Foreign Affairs-grade" review pipeline.
//!
//! Research basis (2025–2026): this layer implements, as deterministic and
//! testable code, the techniques that the state of the art converges on for
//! high-stakes analytical generation:
//!
//! - **Atomic-claim composition** (ComInsight, `arXiv:2610.03525`): insights
//!   are decomposed into atomic claims that carry provenance, and composite
//!   conclusions are only as strong as their verified atoms.
//! - **Type-routed claim verification** (FinGround `arXiv:2604.23588`, CoVe
//!   `arXiv:2309.11495`, FActScore `arXiv:2305.14251`): numbers, dates,
//!   entities and causal links are each verified with the mechanism that fits
//!   them, and the supported fraction of atomic facts is the factuality score.
//! - **Argumentation-based warrants** (contestable debate, QBAF-style,
//!   `arXiv:2605.14495`): support and attack arguments with provenance resolve
//!   to a per-claim warrant, and the weakest link bounds the insight.
//! - **Independent-evidence fusion without corroboration illusion**
//!   (`arXiv:2609.22246`): duplicated or coordinated sources do not count as
//!   independent corroboration; noisy-OR fusion of genuinely independent
//!   chains is used instead of naive counting.
//! - **Calibration** (CHAIN `arXiv:2609.36689`, competence gating
//!   `arXiv:2609.12101`, TTCL `arXiv:2609.02695`): confidence is adjusted from
//!   resolved-outcome calibration curves and source competence, and stated
//!   certainty must match measured competence.
//! - **Analytic tradecraft standards** (ICD 203-style): sources characterized,
//!   uncertainty expressed, assumptions distinguished from judgments,
//!   alternatives analysed, argumentation clear and logical, change explained,
//!   judgments accurate.
//!
//! Every submodule is pure (no network, no database) so the review is
//! reproducible and unit-testable, and the worker persists the resulting
//! [`editorial::EditorialReview`] with the insight.

pub mod argumentation;
pub mod calibration;
pub mod depth;
pub mod editorial;
pub mod verification;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One piece of evidence available to the analytical review.
///
/// Deliberately decoupled from storage rows so every subsystem (worker LLM
/// path, memo path, dogfood harness) can build records from its own types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: Uuid,
    pub title: String,
    pub text: String,
    pub source_name: String,
    pub source_url: Option<String>,
    /// Signal family, e.g. "news", "certification", "warning", "registry".
    pub signal_type: String,
    pub observed_at: Option<DateTime<Utc>>,
    /// Producer-assigned reliability in `0..=1` (source tier × freshness).
    pub reliability: f64,
}

impl EvidenceRecord {
    /// Full searchable text of the record (title + body).
    pub fn searchable_text(&self) -> String {
        format!("{} {}", self.title, self.text)
    }

    /// The registrable source family: host derived from the URL when present,
    /// else the source name. Used for independence counting.
    pub fn source_family(&self) -> String {
        if let Some(url) = self.source_url.as_deref() {
            if let Some(host) = url_host(url) {
                return host;
            }
        }
        self.source_name.to_ascii_lowercase()
    }
}

/// Extract the host from a URL without pulling in a URL parser: scheme,
/// optional userinfo/port, path stripped. Returns `None` for non-URLs.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_port = rest.split(['/', '?', '#']).next()?;
    let host = host_port
        .rsplit('@')
        .next()
        .unwrap_or(host_port)
        .split(':')
        .next()
        .unwrap_or(host_port);
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Class of source, used for independence and authority scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceClass {
    /// Official registry / sanctions list / government publication.
    Official,
    /// Tier-1 or regional media.
    Media,
    /// Company-controlled channel (own site, press release, social account).
    Corporate,
    /// Social posts, forums, unverified feeds.
    Social,
    Unknown,
}

impl SourceClass {
    pub const fn authority(self) -> f64 {
        match self {
            Self::Official => 1.0,
            Self::Media => 0.8,
            Self::Corporate => 0.6,
            Self::Social => 0.35,
            Self::Unknown => 0.5,
        }
    }

    /// Classify from a signal type / source name string. Conservative: only
    /// clearly-official markers grant the official class.
    pub fn classify(signal_type: &str, source_name: &str) -> Self {
        let probe = format!("{signal_type} {source_name}").to_ascii_lowercase();
        const OFFICIAL: [&str; 12] = [
            "registry",
            "sanction",
            "ofac",
            "government",
            "ministry",
            "gazette",
            "regulator",
            "official",
            "sec_edgar",
            "edgar",
            "gleif",
            "court",
        ];
        const SOCIAL: [&str; 9] = [
            "social", "telegram", "reddit", "forum", "twitter", "mastodon", "bluesky", "post",
            "comment",
        ];
        const CORPORATE: [&str; 7] = [
            "press release",
            "press_release",
            "company site",
            "corporate",
            "job post",
            "careers",
            "website",
        ];
        if OFFICIAL.iter().any(|needle| probe.contains(needle)) {
            return Self::Official;
        }
        if SOCIAL.iter().any(|needle| probe.contains(needle)) {
            return Self::Social;
        }
        if CORPORATE.iter().any(|needle| probe.contains(needle)) {
            return Self::Corporate;
        }
        if probe.contains("news") || probe.contains("media") || probe.contains("wire") {
            return Self::Media;
        }
        Self::Unknown
    }
}

/// A grouped, independent source family with its aggregate weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFamily {
    pub family: String,
    pub class: SourceClass,
    pub evidence_count: usize,
    /// Best reliability among the family's records.
    pub best_reliability: f64,
}

/// Group evidence by source family, collapsing duplicates of the same
/// publisher. Families are sorted by best reliability (descending).
pub fn source_families(evidence: &[EvidenceRecord]) -> Vec<SourceFamily> {
    use std::collections::BTreeMap;

    let mut families: BTreeMap<String, SourceFamily> = BTreeMap::new();
    for record in evidence {
        let family = record.source_family();
        // Pick a representative class per family: Official > Media > Corporate
        // > Unknown > Social (a family holding an official artifact is
        // official).
        let class = SourceClass::classify(&record.signal_type, &record.source_name);
        let entry = families.entry(family.clone()).or_insert(SourceFamily {
            family,
            class,
            evidence_count: 0,
            best_reliability: 0.0,
        });
        entry.evidence_count += 1;
        entry.best_reliability = entry
            .best_reliability
            .max(record.reliability.clamp(0.0, 1.0));
        if class_rank(class) > class_rank(entry.class) {
            entry.class = class;
        }
    }
    let mut out: Vec<SourceFamily> = families.into_values().collect();
    out.sort_by(|a, b| {
        b.best_reliability
            .partial_cmp(&a.best_reliability)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.family.cmp(&b.family))
    });
    out
}

/// Higher = more authoritative class (used for family-class promotion).
pub const fn class_rank(class: SourceClass) -> u8 {
    match class {
        SourceClass::Official => 4,
        SourceClass::Media => 3,
        SourceClass::Corporate => 2,
        SourceClass::Unknown => 1,
        SourceClass::Social => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(source: &str, url: Option<&str>) -> EvidenceRecord {
        EvidenceRecord {
            evidence_id: Uuid::new_v4(),
            title: "t".into(),
            text: "b".into(),
            source_name: source.into(),
            source_url: url.map(str::to_string),
            signal_type: "news".into(),
            observed_at: None,
            reliability: 0.7,
        }
    }

    #[test]
    fn host_extraction_handles_ports_and_paths() {
        assert_eq!(
            url_host("https://www.reuters.com/world/x?y=1"),
            Some("reuters.com".into())
        );
        assert_eq!(
            url_host("http://user@Example.GOV:8080/path"),
            Some("example.gov".into())
        );
        assert_eq!(url_host("not a url"), None);
    }

    #[test]
    fn families_collapse_same_publisher_and_take_best_reliability() {
        let mut a = record("Reuters", Some("https://www.reuters.com/a"));
        a.reliability = 0.9;
        let mut b = record("Reuters", Some("https://reuters.com/b"));
        b.reliability = 0.6;
        let families = source_families(&[a, b]);
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].evidence_count, 2);
        assert!((families[0].best_reliability - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn official_class_wins_over_social_within_a_family() {
        let mut official = record("Ministry of Trade", Some("https://trade.gov/a"));
        official.signal_type = "government_registry".into();
        let social = record("Ministry of Trade (social)", Some("https://trade.gov/b"));
        let families = source_families(&[official, social]);
        assert_eq!(families[0].class, SourceClass::Official);
    }
}
