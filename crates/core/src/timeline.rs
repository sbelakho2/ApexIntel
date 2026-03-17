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
                TemporalClaim::EventPrecedesReference { event_type, marker, requirement } => {
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
                        None => {
                            // Only report missing event as violation if claim is Required
                            if matches!(requirement, ClaimRequirement::Required) {
                                violations.push(TemporalViolation {
                                    claim: claim.clone(),
                                    reason: format!("event '{event_type}' is not present in the entity timeline"),
                                    event_time: None,
                                    reference_time: Some(reference_time),
                                });
                            }
                            // Optional claims don't report missing events as violations
                        }
                        _ => {}
                    }
                }
                TemporalClaim::OrderedEvents {
                    earlier_event_type,
                    later_event_type,
                    connector,
                    requirement,
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
                        (None, _) | (_, None) => {
                            // Only report missing events as violation if claim is Required
                            if matches!(requirement, ClaimRequirement::Required) {
                                violations.push(TemporalViolation {
                                    claim: claim.clone(),
                                    reason: format!(
                                        "ordered-events claim requires both '{earlier_event_type}' and '{later_event_type}' to exist in the entity timeline"
                                    ),
                                    event_time: earlier_time,
                                    reference_time: later_time,
                                });
                            }
                            // Optional claims don't report missing events as violations
                        }
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

/// Whether a temporal claim is required or optional for validation purposes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum ClaimRequirement {
    /// The claim must be satisfied; missing events are violations.
    #[default]
    Required,
    /// The claim is only validated if all referenced events exist.
    /// Missing events do not constitute violations.
    Optional,
    /// The claim is only validated for entities of a specific type.
    /// The predicate receives the entity type and returns true if the claim applies.
    /// Note: Function pointers are not comparable, so PartialEq/Eq are not derived.
    #[serde(skip)]
    EntityTypeFilter(fn(&str) -> bool),
}

// Manual PartialEq implementation that only compares the variant, not the function pointer
impl PartialEq for ClaimRequirement {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Required, Self::Required) => true,
            (Self::Optional, Self::Optional) => true,
            (Self::EntityTypeFilter(_), Self::EntityTypeFilter(_)) => {
                // Function pointers are not meaningfully comparable
                // We consider two EntityTypeFilter variants equal by variant only
                true
            }
            _ => false,
        }
    }
}

impl Eq for ClaimRequirement {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TemporalClaim {
    /// Asserts that an event occurs before a reference time (e.g., "now" or a deadline).
    EventPrecedesReference {
        event_type: String,
        marker: String,
        #[serde(default)]
        requirement: ClaimRequirement,
    },
    /// Asserts that one event type must occur before another.
    OrderedEvents {
        earlier_event_type: String,
        later_event_type: String,
        connector: String,
        #[serde(default)]
        requirement: ClaimRequirement,
    },
}

impl TemporalClaim {
    /// Create a required event-precedes-reference claim.
    pub fn event_before_reference(event_type: impl Into<String>, marker: impl Into<String>) -> Self {
        Self::EventPrecedesReference {
            event_type: event_type.into(),
            marker: marker.into(),
            requirement: ClaimRequirement::Required,
        }
    }

    /// Create an optional event-precedes-reference claim.
    pub fn optional_event_before_reference(event_type: impl Into<String>, marker: impl Into<String>) -> Self {
        Self::EventPrecedesReference {
            event_type: event_type.into(),
            marker: marker.into(),
            requirement: ClaimRequirement::Optional,
        }
    }

    /// Create a required ordered-events claim.
    pub fn ordered_events(
        earlier_event_type: impl Into<String>,
        later_event_type: impl Into<String>,
        connector: impl Into<String>,
    ) -> Self {
        Self::OrderedEvents {
            earlier_event_type: earlier_event_type.into(),
            later_event_type: later_event_type.into(),
            connector: connector.into(),
            requirement: ClaimRequirement::Required,
        }
    }

    /// Create an optional ordered-events claim.
    pub fn optional_ordered_events(
        earlier_event_type: impl Into<String>,
        later_event_type: impl Into<String>,
        connector: impl Into<String>,
    ) -> Self {
        Self::OrderedEvents {
            earlier_event_type: earlier_event_type.into(),
            later_event_type: later_event_type.into(),
            connector: connector.into(),
            requirement: ClaimRequirement::Optional,
        }
    }

    /// Check if this claim applies to the given entity type.
    pub fn applies_to_entity(&self, entity_type: &str) -> bool {
        let requirement = match self {
            Self::EventPrecedesReference { requirement, .. } => requirement,
            Self::OrderedEvents { requirement, .. } => requirement,
        };
        if let ClaimRequirement::EntityTypeFilter(f) = requirement {
            f(entity_type)
        } else {
            true // Required and Optional apply to all entity types
        }
    }
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
                requirement: ClaimRequirement::Required,
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
                requirement: ClaimRequirement::Required,
            }],
            now,
        );

        assert!(!report.consistent);
        assert_eq!(report.violations.len(), 1);
    }
}