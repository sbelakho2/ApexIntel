use chrono::{DateTime, Utc};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceQuality {
    pub overall_score: f64,
    pub corroboration_score: f64,
    pub diversity_score: f64,
    pub independence_score: f64,
    pub freshness_score: f64,
    pub contradiction_penalty: f64,
    pub source_count: usize,
    pub independent_source_count: usize,
    pub supporting_count: usize,
    pub contradicting_count: usize,
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

        if let Some(group) = evidence_source_group(record) {
            independent_sources.insert(group);
        }
        if let Some(source_type) = record.source_type.as_deref() {
            let normalized = source_type.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                source_types.insert(normalized);
            }
        }
        if let Some(observed_at) = record.observed_at {
            let age_days = (now - observed_at).num_days().max(0) as f64;
            freshness_total += (-age_days / 30.0).exp();
            freshness_count += 1;
        }
    }

    let source_count = records.len();
    let independent_source_count = independent_sources.len().max(1);
    let corroboration_score = if support_count == 0 {
        0.0
    } else {
        let independent_support = (independent_source_count.min(support_count) as f64
            / source_count as f64)
            .clamp(0.0, 1.0);
        let support_strength = (support_weight / support_count as f64).clamp(0.0, 1.0);
        (0.55 * independent_support + 0.45 * support_strength).clamp(0.0, 1.0)
    };
    let diversity_score = {
        let source_diversity = independent_source_count as f64 / source_count as f64;
        let type_diversity = if source_types.is_empty() {
            source_diversity
        } else {
            (source_types.len() as f64 / source_count as f64).clamp(0.0, 1.0)
        };
        (0.6 * source_diversity + 0.4 * type_diversity).clamp(0.0, 1.0)
    };
    let independence_score =
        (independent_source_count as f64 / source_count as f64).clamp(0.0, 1.0);
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
        quality_label,
    }
}

fn evidence_source_group(record: &EvidenceRecord) -> Option<String> {
    if let Some(source_id) = record.source_id.as_deref() {
        let normalized = source_id.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    if let Some(source_url) = record.source_url.as_deref() {
        if let Ok(url) = Url::parse(source_url) {
            if let Some(host) = url.host_str() {
                let normalized = host.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationReport {
    pub base_confidence: f64,
    pub calibrated_confidence: f64,
    pub posterior_precision: f64,
    pub sample_size: u64,
    pub evidence_quality: f64,
    pub calibration_delta: f64,
    pub confidence_band: String,
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

    let historical_component =
        base_confidence * (1.0 - sample_weight) + posterior_precision * sample_weight;
    let calibrated_confidence =
        (0.55 * base_confidence + 0.30 * historical_component + 0.15 * evidence_quality)
            .clamp(0.0, 1.0);
    let calibration_delta = calibrated_confidence - base_confidence;
    let confidence_band = match calibrated_confidence {
        score if score >= 0.8 => "high",
        score if score >= 0.6 => "elevated",
        score if score >= 0.4 => "watch",
        _ => "low",
    }
    .to_string();

    CalibrationReport {
        base_confidence,
        calibrated_confidence,
        posterior_precision,
        sample_size,
        evidence_quality,
        calibration_delta,
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
}
