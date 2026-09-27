use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Supports,
    Contradicts,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub source_type: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
    pub relevance: f64,
    pub stance: EvidenceStance,
    /// Source-quality score already computed elsewhere (for example
    /// `source_reliability_stats.effective_reliability`), when available.
    /// Independent of how many records happen to cite the source.
    #[serde(default)]
    pub source_reliability: Option<f64>,
    /// Source-quality tier already computed elsewhere (for example
    /// `source_reliability_stats.tier`).
    #[serde(default)]
    pub source_reliability_tier: Option<String>,
    /// True when this record is derived intelligence (a generated insight or
    /// inference) rather than a directly observed fact.
    #[serde(default)]
    pub derived: bool,
}

impl EvidenceRecord {
    pub fn new(relevance: f64, stance: EvidenceStance) -> Self {
        Self {
            source_id: None,
            source_url: None,
            source_type: None,
            observed_at: None,
            relevance,
            stance,
            source_reliability: None,
            source_reliability_tier: None,
            derived: false,
        }
    }

    pub fn with_source_url(mut self, source_url: impl Into<String>) -> Self {
        self.source_url = Some(source_url.into());
        self
    }

    pub fn with_source_type(mut self, source_type: impl Into<String>) -> Self {
        self.source_type = Some(source_type.into());
        self
    }

    pub fn with_source_id(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }

    pub fn with_observed_at(mut self, observed_at: DateTime<Utc>) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    /// Attach source-quality data measured independently of this record's
    /// presence (never derived from corpus counts).
    pub fn with_source_reliability(
        mut self,
        reliability: Option<f64>,
        tier: Option<String>,
    ) -> Self {
        self.source_reliability = reliability;
        self.source_reliability_tier = tier;
        self
    }

    pub fn derived(mut self) -> Self {
        self.derived = true;
        self
    }
}

/// The reusable evidence-quality model (audit P0-4/P1-3).
///
/// Every field is an independent measurement: counts are never re-labelled as
/// "reliability" or "sufficiency", and independence is counted over distinct
/// registrable domains, not raw source records. `quality_label` is derived from
/// the weighted sub-scores below, not from any single count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceQuality {
    pub overall_score: f64,
    pub corroboration_score: f64,
    pub diversity_score: f64,
    pub independence_score: f64,
    pub freshness_score: f64,
    pub contradiction_penalty: f64,
    pub source_count: usize,
    /// Distinct registrable domains / origins among the cited sources.
    pub independent_source_count: usize,
    pub supporting_count: usize,
    pub contradicting_count: usize,
    /// Alias of `contradicting_count`, named for consumers that want the
    /// contradiction count on its own.
    pub contradiction_count: usize,
    /// Fraction of records whose source is a distinct registrable domain
    /// (independence ratio, 0.0–1.0).
    pub source_diversity: f64,
    /// How many records cite each source-quality tier. Records without
    /// measured source quality are counted under `unknown`. Sums to
    /// `source_count`.
    pub source_reliability_distribution: BTreeMap<String, usize>,
    /// Fraction of records that carry the minimum provenance for verification:
    /// a resolvable source group and an observation timestamp (0.0–1.0).
    pub evidence_completeness: f64,
    /// Mean recency weight of timestamped records (exp(-age_days/30)); 0.5
    /// when no record carries a timestamp.
    pub evidence_freshness: f64,
    /// Records that are directly observed facts.
    pub direct_evidence_count: usize,
    /// Records that are derived intelligence (generated insight/inference).
    pub derived_evidence_count: usize,
    pub quality_label: String,
}

impl Default for EvidenceQuality {
    fn default() -> Self {
        Self {
            overall_score: 0.0,
            corroboration_score: 0.0,
            diversity_score: 0.0,
            independence_score: 0.0,
            freshness_score: 0.0,
            contradiction_penalty: 0.0,
            source_count: 0,
            independent_source_count: 0,
            supporting_count: 0,
            contradicting_count: 0,
            contradiction_count: 0,
            source_diversity: 0.0,
            source_reliability_distribution: BTreeMap::new(),
            evidence_completeness: 0.0,
            evidence_freshness: 0.0,
            direct_evidence_count: 0,
            derived_evidence_count: 0,
            quality_label: "insufficient".to_string(),
        }
    }
}

pub fn assess_evidence_quality(records: &[EvidenceRecord], now: DateTime<Utc>) -> EvidenceQuality {
    if records.is_empty() {
        return EvidenceQuality::default();
    }

    let mut independent_sources = HashSet::new();
    let mut source_types = HashSet::new();
    let mut freshness_total = 0.0;
    let mut freshness_count = 0usize;
    let mut support_weight = 0.0;
    let mut contradict_weight = 0.0;
    let mut support_count = 0usize;
    let mut contradict_count = 0usize;
    let mut completeness_count = 0usize;
    let mut direct_count = 0usize;
    let mut derived_count = 0usize;
    let mut reliability_distribution: BTreeMap<String, usize> = BTreeMap::new();

    for record in records {
        let relevance = record.relevance.clamp(0.0, 1.0);
        match record.stance {
            EvidenceStance::Supports => {
                support_weight += relevance.max(0.2);
                support_count += 1;
            }
            EvidenceStance::Contradicts => {
                contradict_weight += relevance.max(0.2);
                contradict_count += 1;
            }
            EvidenceStance::Neutral => {
                support_weight += relevance * 0.35;
            }
        }

        let source_group = evidence_source_group(record);
        if let Some(group) = source_group.as_ref() {
            independent_sources.insert(group.clone());
        }
        if let Some(source_type) = record.source_type.as_deref() {
            let normalized = source_type.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                source_types.insert(normalized);
            }
        }
        if source_group.is_some() && record.observed_at.is_some() {
            completeness_count += 1;
        }
        if let Some(observed_at) = record.observed_at {
            let age_days = (now - observed_at).num_days().max(0) as f64;
            freshness_total += (-age_days / 30.0).exp();
            freshness_count += 1;
        }
        if record.derived {
            derived_count += 1;
        } else {
            direct_count += 1;
        }
        let tier = record
            .source_reliability_tier
            .as_deref()
            .map(str::trim)
            .filter(|tier| !tier.is_empty())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        *reliability_distribution.entry(tier).or_default() += 1;
    }

    let source_count = records.len();
    // Do not inflate independence: records with no source id/url contribute no
    // independent source (previously `.max(1)` gave a single unsourced record
    // full independence credit).
    let independent_source_count = independent_sources.len();
    let corroboration_score = if support_count == 0 {
        0.0
    } else {
        let independent_support = (independent_source_count.min(support_count) as f64
            / source_count as f64)
            .clamp(0.0, 1.0);
        let support_strength = (support_weight / support_count as f64).clamp(0.0, 1.0);
        (0.55 * independent_support + 0.45 * support_strength).clamp(0.0, 1.0)
    };
    let source_diversity = (independent_source_count as f64 / source_count as f64).clamp(0.0, 1.0);
    let diversity_score = {
        let type_diversity = if source_types.is_empty() {
            source_diversity
        } else {
            (source_types.len() as f64 / source_count as f64).clamp(0.0, 1.0)
        };
        (0.6 * source_diversity + 0.4 * type_diversity).clamp(0.0, 1.0)
    };
    let independence_score = source_diversity;
    let freshness_score = if freshness_count == 0 {
        0.5
    } else {
        (freshness_total / freshness_count as f64).clamp(0.0, 1.0)
    };
    let contradiction_penalty = if support_weight + contradict_weight <= f64::EPSILON {
        0.0
    } else {
        (contradict_weight / (support_weight + contradict_weight)).clamp(0.0, 1.0)
    };
    let evidence_completeness = (completeness_count as f64 / source_count as f64).clamp(0.0, 1.0);

    let raw = 0.35 * corroboration_score
        + 0.20 * diversity_score
        + 0.25 * independence_score
        + 0.20 * freshness_score;
    let overall_score = (raw * (1.0 - 0.6 * contradiction_penalty)).clamp(0.0, 1.0);
    let quality_label = match overall_score {
        score if score >= 0.8 && contradiction_penalty < 0.15 => "high",
        score if score >= 0.6 => "moderate",
        score if score >= 0.35 => "emerging",
        _ => "insufficient",
    }
    .to_string();

    EvidenceQuality {
        overall_score,
        corroboration_score,
        diversity_score,
        independence_score,
        freshness_score,
        contradiction_penalty,
        source_count,
        independent_source_count,
        supporting_count: support_count,
        contradicting_count: contradict_count,
        contradiction_count: contradict_count,
        source_diversity,
        source_reliability_distribution: reliability_distribution,
        evidence_completeness,
        evidence_freshness: freshness_score,
        direct_evidence_count: direct_count,
        derived_evidence_count: derived_count,
        quality_label,
    }
}

fn evidence_source_group(record: &EvidenceRecord) -> Option<String> {
    // Prefer the registrable domain of the source URL: two articles from the
    // same publisher (or the same publisher's subdomains) are not independent
    // corroboration. Only fall back to the raw source id when no URL is known.
    if let Some(source_url) = record.source_url.as_deref() {
        if let Some(domain) = registrable_domain(source_url) {
            return Some(domain);
        }
    }
    if let Some(source_id) = record.source_id.as_deref() {
        let normalized = source_id.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    None
}

/// Registrable domain (eTLD+1 approximation) for a URL or bare host.
///
/// Independence must count distinct organisations, not distinct URLs or
/// subdomains: `news.example.com` and `blog.example.com` are one origin. This
/// strips the scheme/path, lowercases, removes `www.`, and reduces the host to
/// its last label plus the public suffix when the suffix is a known multi-label
/// one (for example `co.uk`); otherwise it keeps the last two labels. IP
/// addresses and single-label hosts are returned as-is (lowercased), because
/// there is no registrable domain to reduce them to.
pub fn registrable_domain(url_or_host: &str) -> Option<String> {
    let raw = url_or_host.trim();
    if raw.is_empty() {
        return None;
    }
    let host = match Url::parse(raw) {
        Ok(parsed) => parsed.host_str().map(str::to_string),
        Err(_) => {
            // Bare host (possibly with port/path): strip everything after the
            // first `/`, `?`, or `#`, then the port.
            let cut = raw
                .find(['/', '?', '#'])
                .map(|index| &raw[..index])
                .unwrap_or(raw);
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
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host);
    }
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    if labels.len() < 2 {
        return Some(host);
    }
    let suffix_len = multi_label_suffix_len(&labels);
    let keep = suffix_len + 1;
    Some(labels[labels.len() - keep..].join("."))
}

/// Number of trailing labels forming a known multi-label public suffix.
fn multi_label_suffix_len(labels: &[&str]) -> usize {
    const MULTI_LABEL_SUFFIXES: &[&[&str]] = &[
        &["co", "uk"],
        &["org", "uk"],
        &["ac", "uk"],
        &["gov", "uk"],
        &["co", "jp"],
        &["or", "jp"],
        &["ne", "jp"],
        &["com", "au"],
        &["net", "au"],
        &["org", "au"],
        &["co", "nz"],
        &["com", "br"],
        &["com", "cn"],
        &["com", "hk"],
        &["com", "sg"],
        &["com", "tw"],
        &["co", "in"],
        &["com", "mx"],
        &["co", "za"],
        &["co", "kr"],
    ];
    if labels.len() < 3 {
        return 1;
    }
    let last_two = &labels[labels.len() - 2..];
    if MULTI_LABEL_SUFFIXES.contains(&last_two) {
        2
    } else {
        1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationReport {
    pub base_confidence: f64,
    pub calibrated_confidence: f64,
    pub posterior_precision: f64,
    pub sample_size: u64,
    pub evidence_quality: f64,
    pub calibration_delta: f64,
    pub confidence_interval_low: f64,
    pub confidence_interval_high: f64,
    pub confidence_interval_half_width: f64,
    pub confidence_band: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryBrierObservation {
    pub category: String,
    pub predicted_probability: f64,
    pub actual_outcome: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryWeekBrierScore {
    pub category: String,
    pub week_start: DateTime<Utc>,
    pub sample_count: usize,
    pub brier_score: f64,
}

fn start_of_utc_week(observed_at: DateTime<Utc>) -> DateTime<Utc> {
    let date = observed_at.date_naive();
    let weekday = date.weekday().num_days_from_monday() as i64;
    let week_start = date - chrono::Duration::days(weekday);
    DateTime::<Utc>::from_naive_utc_and_offset(
        week_start.and_hms_opt(0, 0, 0).unwrap_or_default(),
        Utc,
    )
}

pub fn brier_score_per_category_week(
    observations: &[CategoryBrierObservation],
) -> Vec<CategoryWeekBrierScore> {
    let mut grouped: BTreeMap<(String, DateTime<Utc>), (f64, usize)> = BTreeMap::new();
    for observation in observations {
        let category = observation.category.trim();
        if category.is_empty() {
            continue;
        }
        let week_start = start_of_utc_week(observation.observed_at);
        let entry = grouped
            .entry((category.to_string(), week_start))
            .or_insert((0.0, 0));
        let actual = if observation.actual_outcome { 1.0 } else { 0.0 };
        entry.0 += (observation.predicted_probability.clamp(0.0, 1.0) - actual).powi(2);
        entry.1 += 1;
    }

    grouped
        .into_iter()
        .map(
            |((category, week_start), (sum, count))| CategoryWeekBrierScore {
                category,
                week_start,
                sample_count: count,
                brier_score: if count > 0 { sum / count as f64 } else { 0.0 },
            },
        )
        .collect()
}

pub fn calibrate_confidence(
    base_confidence: f64,
    true_positive_count: u64,
    false_positive_count: u64,
    evidence_quality: Option<f64>,
) -> CalibrationReport {
    let base_confidence = base_confidence.clamp(0.0, 1.0);
    let successes = true_positive_count as f64;
    let failures = false_positive_count as f64;
    let posterior_precision = (1.0 + successes) / (2.0 + successes + failures);
    let sample_size = true_positive_count.saturating_add(false_positive_count);
    let sample_weight = 1.0 - (-(sample_size as f64) / 12.0).exp();
    let evidence_quality = evidence_quality.unwrap_or(0.5).clamp(0.0, 1.0);

    let historical_component = 0.67 * posterior_precision + 0.33 * evidence_quality;
    let calibrated_confidence = ((1.0 - sample_weight) * base_confidence
        + sample_weight * historical_component)
        .clamp(0.0, 1.0);
    let calibration_delta = calibrated_confidence - base_confidence;
    let confidence_interval_half_width = if sample_size == 0 {
        0.5
    } else {
        (1.96
            * ((calibrated_confidence * (1.0 - calibrated_confidence)) / sample_size as f64).sqrt())
        .clamp(0.0, 0.5)
    };
    let confidence_interval_low =
        (calibrated_confidence - confidence_interval_half_width).clamp(0.0, 1.0);
    let confidence_interval_high =
        (calibrated_confidence + confidence_interval_half_width).clamp(0.0, 1.0);
    let confidence_band = match confidence_interval_half_width {
        width if width <= 0.05 => "tight",
        width if width <= 0.15 => "moderate",
        _ => "wide",
    }
    .to_string();

    CalibrationReport {
        base_confidence,
        calibrated_confidence,
        posterior_precision,
        sample_size,
        evidence_quality,
        calibration_delta,
        confidence_interval_low,
        confidence_interval_high,
        confidence_interval_half_width,
        confidence_band,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaDirection {
    Up,
    Down,
    Flat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalDelta {
    pub current_value: f64,
    pub previous_value: f64,
    pub absolute_change: f64,
    pub relative_change: f64,
    pub direction: DeltaDirection,
    pub label: String,
}

pub fn compare_temporal_windows(current_value: f64, previous_value: f64) -> TemporalDelta {
    let absolute_change = current_value - previous_value;
    let relative_change = if previous_value.abs() <= f64::EPSILON {
        if current_value.abs() <= f64::EPSILON {
            0.0
        } else {
            1.0
        }
    } else {
        absolute_change / previous_value.abs()
    };
    let direction = if absolute_change > 0.001 {
        DeltaDirection::Up
    } else if absolute_change < -0.001 {
        DeltaDirection::Down
    } else {
        DeltaDirection::Flat
    };
    let label = match direction {
        DeltaDirection::Up if relative_change >= 0.25 => "accelerating",
        DeltaDirection::Up => "rising",
        DeltaDirection::Down if relative_change <= -0.25 => "cooling",
        DeltaDirection::Down => "easing",
        DeltaDirection::Flat => "stable",
    }
    .to_string();

    TemporalDelta {
        current_value,
        previous_value,
        absolute_change,
        relative_change,
        direction,
        label,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalFrame {
    pub id: String,
    pub theme: String,
    pub category: Option<String>,
    pub region: Option<String>,
    pub entity: Option<String>,
    pub confidence: f64,
    pub impact: f64,
    pub source_group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeakSignalCluster {
    pub theme: String,
    pub region: Option<String>,
    pub signal_ids: Vec<String>,
    pub signal_count: usize,
    pub independent_source_count: usize,
    pub combined_score: f64,
    pub label: String,
}

pub fn fuse_weak_signals(signals: &[SignalFrame]) -> Vec<WeakSignalCluster> {
    let weak_signals: Vec<&SignalFrame> = signals
        .iter()
        .filter(|signal| signal.confidence <= 0.65)
        .collect();

    let mut grouped: BTreeMap<(String, String), Vec<&SignalFrame>> = BTreeMap::new();
    for signal in weak_signals {
        let region = signal
            .region
            .as_deref()
            .unwrap_or("global")
            .to_ascii_lowercase();
        let theme = if let Some(entity) = signal.entity.as_deref() {
            format!(
                "{}::{}",
                signal.theme.trim().to_ascii_lowercase(),
                entity.trim().to_ascii_lowercase()
            )
        } else {
            signal.theme.trim().to_ascii_lowercase()
        };
        grouped.entry((region, theme)).or_default().push(signal);
    }

    let mut clusters = Vec::new();
    for ((region, theme), items) in grouped {
        if items.len() < 2 {
            continue;
        }

        let mut source_groups = HashSet::new();
        let mut combined_score = 0.0;
        let mut ids = Vec::new();
        for item in items {
            if let Some(source_group) = item.source_group.as_deref() {
                let normalized = source_group.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    source_groups.insert(normalized);
                }
            }
            combined_score = 1.0
                - (1.0 - combined_score) * (1.0 - (item.confidence * item.impact).clamp(0.0, 1.0));
            ids.push(item.id.clone());
        }

        let independent_source_count = source_groups.len();
        if independent_source_count < 2 {
            continue;
        }

        let label = if combined_score >= 0.75 {
            "converging"
        } else if combined_score >= 0.55 {
            "emerging"
        } else {
            "speculative"
        }
        .to_string();

        let signal_count = ids.len();
        clusters.push(WeakSignalCluster {
            theme,
            region: Some(region),
            signal_ids: ids,
            signal_count,
            independent_source_count,
            combined_score,
            label,
        });
    }

    clusters.sort_by(|a, b| {
        b.combined_score
            .partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clusters
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisInput {
    pub hypothesis: String,
    pub support_score: f64,
    pub contradiction_score: f64,
    pub prior: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisScorecard {
    pub hypothesis: String,
    pub support_score: f64,
    pub contradiction_score: f64,
    pub net_score: f64,
    pub posterior: f64,
    pub assessment: String,
}

pub fn score_competing_hypotheses(inputs: &[HypothesisInput]) -> Vec<HypothesisScorecard> {
    let mut cards: Vec<HypothesisScorecard> = inputs
        .iter()
        .map(|input| {
            let support = input.support_score.max(0.0);
            let contradiction = input.contradiction_score.max(0.0);
            let net_score = (support - contradiction).clamp(-1.0, 1.0);
            let posterior = (input.prior.clamp(0.0, 1.0) + 0.45 * support - 0.35 * contradiction)
                .clamp(0.0, 1.0);
            let assessment = if posterior >= 0.75 && net_score > 0.15 {
                "favored"
            } else if posterior <= 0.35 || net_score < -0.1 {
                "disfavored"
            } else {
                "contested"
            }
            .to_string();

            HypothesisScorecard {
                hypothesis: input.hypothesis.clone(),
                support_score: support,
                contradiction_score: contradiction,
                net_score,
                posterior,
                assessment,
            }
        })
        .collect();

    cards.sort_by(|a, b| {
        b.posterior
            .partial_cmp(&a.posterior)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cards
}

pub fn source_group_from_url(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .host_str()
                .map(|host| host.trim().to_ascii_lowercase())
        })
        .filter(|host| !host.is_empty())
}

pub fn top_source_groups(urls: &[String]) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for url in urls {
        if let Some(group) = source_group_from_url(url) {
            *counts.entry(group).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().map(|(group, _)| group).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn evidence_quality_rewards_independent_corroboration() {
        let now = Utc::now();
        let strong = assess_evidence_quality(
            &[
                EvidenceRecord::new(0.9, EvidenceStance::Supports)
                    .with_source_url("https://alpha.example.com/report")
                    .with_source_type("news")
                    .with_observed_at(now - Duration::days(1)),
                EvidenceRecord::new(0.8, EvidenceStance::Supports)
                    .with_source_url("https://beta.example.org/report")
                    .with_source_type("filing")
                    .with_observed_at(now - Duration::days(2)),
            ],
            now,
        );
        let weak = assess_evidence_quality(
            &[EvidenceRecord::new(0.8, EvidenceStance::Supports)
                .with_source_url("https://alpha.example.com/report")
                .with_source_type("news")
                .with_observed_at(now - Duration::days(10))],
            now,
        );

        assert!(strong.overall_score > weak.overall_score);
        assert_eq!(strong.independent_source_count, 2);
    }

    #[test]
    fn evidence_quality_penalizes_contradictions() {
        let now = Utc::now();
        let quality = assess_evidence_quality(
            &[
                EvidenceRecord::new(0.9, EvidenceStance::Supports)
                    .with_source_url("https://alpha.example.com/a"),
                EvidenceRecord::new(0.8, EvidenceStance::Contradicts)
                    .with_source_url("https://beta.example.com/b"),
            ],
            now,
        );

        assert!(quality.contradiction_penalty > 0.0);
        assert!(quality.overall_score < 0.6);
    }

    #[test]
    fn calibration_disciplines_low_precision_histories() {
        let high_quality = calibrate_confidence(0.75, 40, 4, Some(0.8));
        let weak_history = calibrate_confidence(0.75, 4, 20, Some(0.8));

        assert!(high_quality.calibrated_confidence > weak_history.calibrated_confidence);
        assert!(weak_history.posterior_precision < 0.5);
    }

    #[test]
    fn dynamic_confidence_data_dominates() {
        let report = calibrate_confidence(0.55, 100, 0, Some(0.95));

        assert!(report.calibrated_confidence > 0.90);
        assert_eq!(report.confidence_band, "tight");
        assert!(report.confidence_interval_half_width < 0.10);
    }

    #[test]
    fn low_sample_confidence_band_is_wide() {
        let report = calibrate_confidence(0.75, 1, 0, Some(0.8));

        assert_eq!(report.confidence_band, "wide");
        assert!(report.confidence_interval_half_width >= 0.15);
    }

    #[test]
    fn brier_score_per_category() {
        let base = DateTime::<Utc>::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
                .unwrap_or_default()
                .and_hms_opt(12, 0, 0)
                .unwrap_or_default(),
            Utc,
        );
        let observations = vec![
            CategoryBrierObservation {
                category: "supply_chain".to_string(),
                predicted_probability: 0.9,
                actual_outcome: true,
                observed_at: base,
            },
            CategoryBrierObservation {
                category: "supply_chain".to_string(),
                predicted_probability: 0.8,
                actual_outcome: true,
                observed_at: base + chrono::Duration::days(1),
            },
            CategoryBrierObservation {
                category: "regulatory".to_string(),
                predicted_probability: 0.2,
                actual_outcome: false,
                observed_at: base,
            },
            CategoryBrierObservation {
                category: "regulatory".to_string(),
                predicted_probability: 0.3,
                actual_outcome: false,
                observed_at: base + chrono::Duration::days(2),
            },
        ];

        let scores = brier_score_per_category_week(&observations);

        assert_eq!(scores.len(), 2);
        assert!(scores.iter().all(|score| score.brier_score < 0.25));
    }

    #[test]
    fn temporal_delta_computes_direction() {
        let delta = compare_temporal_windows(12.0, 8.0);
        assert_eq!(delta.direction, DeltaDirection::Up);
        assert!(delta.relative_change > 0.0);
    }

    #[test]
    fn weak_signal_fusion_requires_independent_sources() {
        let clusters = fuse_weak_signals(&[
            SignalFrame {
                id: "a".to_string(),
                theme: "expansion".to_string(),
                category: Some("demand".to_string()),
                region: Some("eu".to_string()),
                entity: Some("acme".to_string()),
                confidence: 0.55,
                impact: 0.6,
                source_group: Some("alpha.example.com".to_string()),
            },
            SignalFrame {
                id: "b".to_string(),
                theme: "expansion".to_string(),
                category: Some("demand".to_string()),
                region: Some("eu".to_string()),
                entity: Some("acme".to_string()),
                confidence: 0.58,
                impact: 0.65,
                source_group: Some("beta.example.com".to_string()),
            },
        ]);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].independent_source_count, 2);
    }

    #[test]
    fn competing_hypotheses_rank_favored_first() {
        let scorecards = score_competing_hypotheses(&[
            HypothesisInput {
                hypothesis: "expansion".to_string(),
                support_score: 0.8,
                contradiction_score: 0.1,
                prior: 0.5,
            },
            HypothesisInput {
                hypothesis: "routine_noise".to_string(),
                support_score: 0.2,
                contradiction_score: 0.5,
                prior: 0.5,
            },
        ]);

        assert_eq!(scorecards[0].hypothesis, "expansion");
        assert_eq!(scorecards[0].assessment, "favored");
    }

    #[test]
    fn independence_counts_distinct_domains_not_raw_sources() {
        let now = Utc::now();
        // Ten articles, but only two organisations (and three hosts, two of
        // which belong to the same registrable domain).
        let mut records = Vec::new();
        for index in 0..6 {
            records.push(
                EvidenceRecord::new(0.8, EvidenceStance::Supports)
                    .with_source_url(format!("https://news.example.com/article-{index}"))
                    .with_observed_at(now - Duration::days(1)),
            );
        }
        for index in 0..3 {
            records.push(
                EvidenceRecord::new(0.8, EvidenceStance::Supports)
                    .with_source_url(format!("https://blog.example.com/post-{index}"))
                    .with_observed_at(now - Duration::days(1)),
            );
        }
        records.push(
            EvidenceRecord::new(0.8, EvidenceStance::Supports)
                .with_source_url("https://other.example.org/report")
                .with_observed_at(now - Duration::days(1)),
        );

        let quality = assess_evidence_quality(&records, now);

        assert_eq!(quality.source_count, 10);
        assert_eq!(
            quality.independent_source_count, 2,
            "subdomains of one registrable domain are not independent corroboration"
        );
        assert!(quality.source_diversity < 0.25);
    }

    #[test]
    fn registrable_domain_reduces_hosts_to_etld_plus_one() {
        assert_eq!(
            registrable_domain("https://news.example.com/story").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("www.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            registrable_domain("https://sub.example.com:8443/path").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("http://192.168.0.1/x").as_deref(),
            Some("192.168.0.1")
        );
        assert_eq!(
            registrable_domain("localhost").as_deref(),
            Some("localhost")
        );
        assert_eq!(registrable_domain(""), None);
    }

    #[test]
    fn evidence_quality_reports_independent_dimensions() {
        let now = Utc::now();
        let quality = assess_evidence_quality(
            &[
                EvidenceRecord::new(0.9, EvidenceStance::Supports)
                    .with_source_url("https://alpha.example.com/a")
                    .with_source_type("news")
                    .with_observed_at(now - Duration::days(1))
                    .with_source_reliability(Some(0.9), Some("High".to_string())),
                EvidenceRecord::new(0.7, EvidenceStance::Contradicts)
                    .with_source_url("https://beta.example.org/b")
                    .with_source_type("filing")
                    .with_observed_at(now - Duration::days(40)),
                EvidenceRecord::new(0.6, EvidenceStance::Supports)
                    .with_source_url("https://gamma.example.net/c")
                    .with_source_type("news")
                    .with_observed_at(now - Duration::days(2))
                    .derived(),
            ],
            now,
        );

        assert_eq!(quality.source_count, 3);
        assert_eq!(quality.independent_source_count, 3);
        assert_eq!(quality.contradiction_count, 1);
        assert_eq!(quality.direct_evidence_count, 2);
        assert_eq!(quality.derived_evidence_count, 1);
        assert!((quality.evidence_completeness - 1.0).abs() < 1e-9);
        assert!(quality.evidence_freshness > 0.0 && quality.evidence_freshness < 1.0);
        assert_eq!(
            quality
                .source_reliability_distribution
                .get("high")
                .copied()
                .unwrap_or_default(),
            1
        );
        assert_eq!(
            quality
                .source_reliability_distribution
                .get("unknown")
                .copied()
                .unwrap_or_default(),
            2
        );
        assert_eq!(
            quality
                .source_reliability_distribution
                .values()
                .sum::<usize>(),
            3
        );
    }

    #[test]
    fn evidence_completeness_requires_source_and_timestamp() {
        let now = Utc::now();
        let quality = assess_evidence_quality(
            &[
                EvidenceRecord::new(0.9, EvidenceStance::Supports)
                    .with_source_url("https://alpha.example.com/a")
                    .with_observed_at(now),
                EvidenceRecord::new(0.9, EvidenceStance::Supports)
                    .with_source_url("https://alpha.example.com/b"),
            ],
            now,
        );
        assert!((quality.evidence_completeness - 0.5).abs() < 1e-9);
    }
}
