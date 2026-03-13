use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub event_type: String,
    pub observed_at: DateTime<Utc>,
    pub confidence: f64,
    pub source_observation_id: Option<String>,
}

impl TimelineEvent {
    pub fn new(
        event_type: impl Into<String>,
        observed_at: DateTime<Utc>,
        confidence: f64,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            observed_at,
            confidence: confidence.clamp(0.0, 1.0),
            source_observation_id: None,
        }
    }

    pub fn with_source_observation_id(mut self, source_observation_id: impl Into<String>) -> Self {
        self.source_observation_id = Some(source_observation_id.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityTimeline {
    pub entity_id: String,
    pub events: Vec<TimelineEvent>,
}

impl EntityTimeline {
    pub fn new(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            events: Vec::new(),
        }
    }

    pub fn add_event(&mut self, event: TimelineEvent) {
        self.events.push(event);
        self.events.sort_by_key(|candidate| candidate.observed_at);
    }

    pub fn event_time(&self, event_type: &str) -> Option<DateTime<Utc>> {
        let canonical = canonical_event_type(event_type);
        self.events
            .iter()
            .find(|event| canonical_event_type(&event.event_type) == canonical)
            .map(|event| event.observed_at)
    }

    pub fn known_event_types(&self) -> Vec<String> {
        let mut event_types = self
            .events
            .iter()
            .map(|event| canonical_event_type(&event.event_type))
            .collect::<Vec<_>>();
        event_types.sort();
        event_types.dedup();
        event_types
    }

    pub fn validate_claims(
        &self,
        claims: &[TemporalClaim],
        reference_time: DateTime<Utc>,
    ) -> TemporalValidationReport {
        let mut violations = Vec::new();

        for claim in claims {
            match claim {
                TemporalClaim::EventPrecedesReference { event_type, marker } => {
                    match self.event_time(event_type) {
                        Some(event_time) if event_time > reference_time => {
                            violations.push(TemporalViolation {
                                claim: claim.clone(),
                                reason: format!(
                                    "marker '{marker}' requires event '{event_type}' to occur before the reference time"
                                ),
                                event_time: Some(event_time),
                                reference_time: Some(reference_time),
                            });
                        }
                        None => violations.push(TemporalViolation {
                            claim: claim.clone(),
                            reason: format!("event '{event_type}' is not present in the entity timeline"),
                            event_time: None,
                            reference_time: Some(reference_time),
                        }),
                        _ => {}
                    }
                }
                TemporalClaim::OrderedEvents {
                    earlier_event_type,
                    later_event_type,
                    connector,
                } => {
                    let earlier_time = self.event_time(earlier_event_type);
                    let later_time = self.event_time(later_event_type);
                    match (earlier_time, later_time) {
                        (Some(earlier_time), Some(later_time)) if earlier_time > later_time => {
                            violations.push(TemporalViolation {
                                claim: claim.clone(),
                                reason: format!(
                                    "connector '{connector}' requires '{earlier_event_type}' to precede '{later_event_type}'"
                                ),
                                event_time: Some(earlier_time),
                                reference_time: Some(later_time),
                            });
                        }
                        (None, _) | (_, None) => violations.push(TemporalViolation {
                            claim: claim.clone(),
                            reason: format!(
                                "ordered-events claim requires both '{earlier_event_type}' and '{later_event_type}' to exist in the entity timeline"
                            ),
                            event_time: earlier_time,
                            reference_time: later_time,
                        }),
                        _ => {}
                    }
                }
            }
        }

        TemporalValidationReport {
            consistent: violations.is_empty(),
            violations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TemporalClaim {
    EventPrecedesReference {
        event_type: String,
        marker: String,
    },
    OrderedEvents {
        earlier_event_type: String,
        later_event_type: String,
        connector: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalViolation {
    pub claim: TemporalClaim,
    pub reason: String,
    pub event_time: Option<DateTime<Utc>>,
    pub reference_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TemporalValidationReport {
    pub consistent: bool,
    pub violations: Vec<TemporalViolation>,
}

pub fn canonical_event_type(event_type: &str) -> String {
    event_type
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn validate_claims_flags_event_after_reference() {
        let now = Utc::now();
        let mut timeline = EntityTimeline::new("entity-1");
        timeline.add_event(TimelineEvent::new("audit", now + Duration::days(2), 0.9));

        let report = timeline.validate_claims(
            &[TemporalClaim::EventPrecedesReference {
                event_type: "audit".into(),
                marker: "after".into(),
            }],
            now,
        );

        assert!(!report.consistent);
        assert_eq!(report.violations.len(), 1);
    }

    #[test]
    fn validate_claims_flags_wrong_event_order() {
        let now = Utc::now();
        let mut timeline = EntityTimeline::new("entity-1");
        timeline.add_event(TimelineEvent::new(
            "contract_awarded",
            now - Duration::days(2),
            0.9,
        ));
        timeline.add_event(TimelineEvent::new("patent_filed", now - Duration::days(5), 0.9));

        let report = timeline.validate_claims(
            &[TemporalClaim::OrderedEvents {
                earlier_event_type: "contract_awarded".into(),
                later_event_type: "patent_filed".into(),
                connector: "before".into(),
            }],
            now,
        );

        assert!(!report.consistent);
        assert_eq!(report.violations.len(), 1);
    }
}