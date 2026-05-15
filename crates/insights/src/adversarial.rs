use apex_core::similarity::jaccard_similarity;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversarialSignal {
    pub signal_id: String,
    pub entity_id: String,
    pub source_id: String,
    pub source_type: String,
    pub observed_at: DateTime<Utc>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementClusterAlert {
    pub entity_id: String,
    pub signal_ids: Vec<String>,
    pub distinct_source_count: usize,
    pub mean_token_jaccard: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InformationOriginAssessment {
    pub origin: String,
    pub requires_corroboration: bool,
    pub first_seen_at: DateTime<Utc>,
    pub first_official_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualConsistencyReport {
    pub consistent: bool,
    pub contradicting_signal_ids: Vec<String>,
    pub requires_corroboration: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationQuarantineDecision {
    pub quarantined: bool,
    pub quarantine_until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationConsensusDecision {
    pub allowed: bool,
    pub requires_consensus: bool,
    pub corroborating_source_count: usize,
}

pub fn detect_coordinated_placement(
    signals: &[AdversarialSignal],
    max_window_hours: i64,
    min_sources: usize,
    min_mean_jaccard: f64,
) -> Vec<PlacementClusterAlert> {
    let mut alerts = Vec::new();
    let mut grouped = HashMap::<String, Vec<&AdversarialSignal>>::new();

    for signal in signals {
        grouped
            .entry(signal.entity_id.clone())
            .or_default()
            .push(signal);
    }

    for (entity_id, mut entity_signals) in grouped {
        entity_signals.sort_by_key(|signal| signal.observed_at);
        for start in 0..entity_signals.len() {
            let window_end = entity_signals[start].observed_at + Duration::hours(max_window_hours);
            let window = entity_signals[start..]
                .iter()
                .copied()
                .take_while(|signal| signal.observed_at <= window_end)
                .collect::<Vec<_>>();
            let distinct_sources = window
                .iter()
                .map(|signal| signal.source_id.as_str())
                .collect::<HashSet<_>>();
            if distinct_sources.len() < min_sources {
                continue;
            }
            let mean_jaccard = mean_pairwise_jaccard(&window);
            if mean_jaccard >= min_mean_jaccard {
                let mut signal_ids = window
                    .iter()
                    .map(|signal| signal.signal_id.clone())
                    .collect::<Vec<_>>();
                signal_ids.sort();
                signal_ids.dedup();
                alerts.push(PlacementClusterAlert {
                    entity_id: entity_id.clone(),
                    signal_ids,
                    distinct_source_count: distinct_sources.len(),
                    mean_token_jaccard: mean_jaccard,
                });
                break;
            }
        }
    }

    alerts
}

pub fn assess_information_origin(
    signals: &[AdversarialSignal],
) -> Option<InformationOriginAssessment> {
    let mut ordered = signals.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|signal| signal.observed_at);
    let first = ordered.first()?;
    let first_official_seen_at = ordered
        .iter()
        .find(|signal| signal.source_type.eq_ignore_ascii_case("official"))
        .map(|signal| signal.observed_at);
    let requires_corroboration = first.source_type.eq_ignore_ascii_case("social")
        && first_official_seen_at
            .map(|official| official > first.observed_at)
            .unwrap_or(true);

    Some(InformationOriginAssessment {
        origin: first.source_type.to_ascii_lowercase(),
        requires_corroboration,
        first_seen_at: first.observed_at,
        first_official_seen_at,
    })
}

pub fn counterfactual_consistency_check(
    supporting_signals: &[AdversarialSignal],
    contradicting_signals: &[AdversarialSignal],
) -> CounterfactualConsistencyReport {
    let mut contradicting_signal_ids = contradicting_signals
        .iter()
        .map(|signal| signal.signal_id.clone())
        .collect::<Vec<_>>();
    contradicting_signal_ids.sort();
    let requires_corroboration = assess_information_origin(supporting_signals)
        .map(|assessment| assessment.requires_corroboration)
        .unwrap_or(false);

    CounterfactualConsistencyReport {
        consistent: contradicting_signal_ids.is_empty(),
        contradicting_signal_ids,
        requires_corroboration,
    }
}

pub fn quarantine_observation(
    source_previously_seen: bool,
    first_seen_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ObservationQuarantineDecision {
    let quarantine_until = first_seen_at + Duration::hours(24);
    ObservationQuarantineDecision {
        quarantined: !source_previously_seen && now < quarantine_until,
        quarantine_until,
    }
}

pub fn consensus_required_for_escalation(
    prior_level: &str,
    proposed_level: &str,
    corroborating_source_ids: &[String],
) -> EscalationConsensusDecision {
    let corroborating_source_count = corroborating_source_ids
        .iter()
        .map(|source_id| source_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let requires_consensus =
        prior_level.eq_ignore_ascii_case("low") && proposed_level.eq_ignore_ascii_case("high");
    let allowed = !requires_consensus || corroborating_source_count >= 2;

    EscalationConsensusDecision {
        allowed,
        requires_consensus,
        corroborating_source_count,
    }
}

fn mean_pairwise_jaccard(signals: &[&AdversarialSignal]) -> f64 {
    if signals.len() < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    let mut count = 0usize;
    for left in 0..signals.len() {
        for right in (left + 1)..signals.len() {
            let left_tokens = tokenize(&signals[left].text);
            let right_tokens = tokenize(&signals[right].text);
            total += jaccard_similarity(&left_tokens, &right_tokens);
            count += 1;
        }
    }
    total / count as f64
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    fn signal(
        signal_id: &str,
        source_id: &str,
        source_type: &str,
        observed_at: DateTime<Utc>,
        text: &str,
    ) -> AdversarialSignal {
        AdversarialSignal {
            signal_id: signal_id.to_string(),
            entity_id: "entity-x".to_string(),
            source_id: source_id.to_string(),
            source_type: source_type.to_string(),
            observed_at,
            text: text.to_string(),
        }
    }

    #[test]
    fn coordinated_placement_detection() {
        let now = Utc::now();
        let signals = vec![
            signal(
                "s1",
                "source-a",
                "news",
                now,
                "Factory outage disrupts output and supplier allocations",
            ),
            signal(
                "s2",
                "source-b",
                "news",
                now + Duration::minutes(40),
                "Factory outage disrupts output and supplier allocation plans",
            ),
            signal(
                "s3",
                "source-c",
                "news",
                now + Duration::minutes(80),
                "Factory outage disrupts output and supplier allocation planning",
            ),
            signal(
                "s4",
                "source-d",
                "news",
                now + Duration::minutes(110),
                "Factory outage disrupts output and supplier allocation routes",
            ),
        ];

        let alerts = detect_coordinated_placement(&signals, 6, 4, 0.6);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].mean_token_jaccard >= 0.6);
    }

    #[test]
    fn social_first_origin_requires_corroboration() {
        let now = Utc::now();
        let signals = vec![
            signal("s1", "social-a", "social", now, "Rumor of a contract award"),
            signal(
                "s2",
                "official-a",
                "official",
                now + Duration::hours(8),
                "Official contract notice",
            ),
        ];
        let assessment = assess_information_origin(&signals).unwrap();
        assert_eq!(assessment.origin, "social");
        assert!(assessment.requires_corroboration);
    }

    #[test]
    fn counterfactual_check_flags_contradictions() {
        let now = Utc::now();
        let support = vec![signal(
            "s1",
            "social-a",
            "social",
            now,
            "Rumor of a contract award",
        )];
        let contradictions = vec![signal(
            "s2",
            "official-a",
            "official",
            now + Duration::hours(2),
            "Official statement says no award was made",
        )];

        let report = counterfactual_consistency_check(&support, &contradictions);
        assert!(!report.consistent);
        assert!(report.requires_corroboration);
        assert_eq!(report.contradicting_signal_ids, vec!["s2".to_string()]);
    }

    #[test]
    fn previously_unseen_source_is_quarantined_for_24_hours() {
        let now = Utc::now();
        let decision = quarantine_observation(false, now - Duration::hours(6), now);
        assert!(decision.quarantined);
        assert_eq!(decision.quarantine_until, now + Duration::hours(18));
    }

    #[test]
    fn low_to_high_escalation_requires_two_sources() {
        let blocked = consensus_required_for_escalation("low", "high", &["source-a".to_string()]);
        assert!(blocked.requires_consensus);
        assert!(!blocked.allowed);

        let allowed = consensus_required_for_escalation(
            "low",
            "high",
            &["source-a".to_string(), "source-b".to_string()],
        );
        assert!(allowed.allowed);
        assert_eq!(allowed.corroborating_source_count, 2);
    }
}
