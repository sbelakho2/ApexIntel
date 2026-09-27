use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use url::Url;

// The evidence-quality model lives in `crate::evidence_quality` (canonical
// module, audit P1 measurement). `analysis` re-exports it so existing
// `apex_core::analysis::...` imports keep working, but there is exactly one
// implementation.
pub use crate::evidence_quality::{
    registrable_domain, EvidenceItem, EvidenceQuality, EvidenceStance,
};

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
}
