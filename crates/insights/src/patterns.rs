//!
//! Pattern Detection for Intelligence Insights.
//!
//! Detects various patterns in OSINT data:
//! - Temporal patterns (recurring events, trends)
//! - Entity patterns (shared connections, behavioral patterns)
//! - Network patterns (relationship clusters, influence)
//! - Anomaly patterns (deviations from baseline)
//!
//! Part of Phase 2.1: LLM Integration for ApexIntel OSINT platform.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{Insight, InsightSeverity};

/// Pattern types detected by the analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// Temporal patterns (recurring events, cycles).
    Temporal,
    /// Entity co-occurrence patterns.
    CoOccurrence,
    /// Beneficial ownership patterns.
    Ownership,
    /// Shell company indicators.
    ShellCompany,
    /// Money laundering indicators.
    MoneyLaundering,
    /// Sanctions evasion patterns.
    SanctionsEvasion,
    /// Supply chain dependencies.
    SupplyChain,
    /// Corporate structure patterns.
    CorporateStructure,
    /// Sentiment/trend changes.
    SentimentShift,
    /// Anomalous behavior.
    Anomaly,
    /// Geographic clustering.
    GeographicCluster,
    /// Time-based correlation.
    TemporalCorrelation,
}

impl PatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Temporal => "temporal",
            Self::CoOccurrence => "co_occurrence",
            Self::Ownership => "ownership",
            Self::ShellCompany => "shell_company",
            Self::MoneyLaundering => "money_laundering",
            Self::SanctionsEvasion => "sanctions_evasion",
            Self::SupplyChain => "supply_chain",
            Self::CorporateStructure => "corporate_structure",
            Self::SentimentShift => "sentiment_shift",
            Self::Anomaly => "anomaly",
            Self::GeographicCluster => "geographic_cluster",
            Self::TemporalCorrelation => "temporal_correlation",
        }
    }

    pub fn default_severity(&self) -> InsightSeverity {
        match self {
            Self::MoneyLaundering | Self::SanctionsEvasion => InsightSeverity::Critical,
            Self::ShellCompany | Self::Anomaly => InsightSeverity::High,
            Self::Ownership | Self::SupplyChain | Self::SentimentShift => InsightSeverity::Medium,
            _ => InsightSeverity::Low,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Temporal => "Recurring events or cyclical patterns detected over time.",
            Self::CoOccurrence => "Entities frequently appearing together across sources.",
            Self::Ownership => "Beneficial ownership chains or structures detected.",
            Self::ShellCompany => {
                "Shell company indicators (single-member, same address, nominee director)."
            }
            Self::MoneyLaundering => "Potential money laundering indicators detected.",
            Self::SanctionsEvasion => "Potential sanctions evasion patterns detected.",
            Self::SupplyChain => "Supply chain dependencies or concentrations identified.",
            Self::CorporateStructure => "Complex corporate structures or unusual hierarchies.",
            Self::SentimentShift => "Significant shift in sentiment or perception.",
            Self::Anomaly => "Anomalous behavior or deviation from baseline.",
            Self::GeographicCluster => "Geographic clustering of related entities.",
            Self::TemporalCorrelation => "Events correlated in timing.",
        }
    }
}

/// A detected pattern with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedPattern {
    pub pattern_type: PatternType,
    pub description: String,
    pub confidence: f64,
    pub severity: InsightSeverity,
    pub entities_involved: Vec<String>,
    pub evidence: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl DetectedPattern {
    pub fn new(pattern_type: PatternType, description: &str) -> Self {
        Self {
            pattern_type,
            description: description.to_string(),
            confidence: 0.5,
            severity: pattern_type.default_severity(),
            entities_involved: vec![],
            evidence: vec![],
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_entities(mut self, entities: Vec<String>) -> Self {
        self.entities_involved = entities;
        self
    }

    pub fn with_evidence(mut self, evidence: Vec<String>) -> Self {
        self.evidence = evidence;
        self
    }

    pub fn to_insight(&self, title_suffix: &str) -> Insight {
        Insight::new(
            &format!("{}: {}", self.pattern_type.as_str(), title_suffix),
            &self.description,
        )
        .with_severity(self.severity)
        .with_confidence(self.confidence)
        .with_entities(self.entities_involved.clone())
        .with_sources(self.evidence.clone())
        .with_tags(vec![self.pattern_type.as_str().to_string()])
    }
}

/// Configuration for pattern detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternDetectorConfig {
    pub enable_temporal: bool,
    pub enable_ownership: bool,
    pub enable_sanctions: bool,
    pub enable_anomaly: bool,
    pub min_confidence: f64,
    pub lookback_days: i64,
}

impl Default for PatternDetectorConfig {
    fn default() -> Self {
        Self {
            enable_temporal: true,
            enable_ownership: true,
            enable_sanctions: true,
            enable_anomaly: true,
            min_confidence: 0.6,
            lookback_days: 90,
        }
    }
}

/// Pattern detector for OSINT data.
pub struct PatternDetector {
    config: PatternDetectorConfig,
}

impl PatternDetector {
    pub fn new(config: PatternDetectorConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(PatternDetectorConfig::default())
    }

    /// Detect all patterns in the given data.
    pub fn detect_all(&self, data: &PatternData) -> Vec<DetectedPattern> {
        let mut patterns = Vec::new();

        if self.config.enable_temporal {
            patterns.extend(self.detect_temporal_patterns(data));
        }

        if self.config.enable_ownership {
            patterns.extend(self.detect_ownership_patterns(data));
        }

        if self.config.enable_sanctions {
            patterns.extend(self.detect_sanctions_patterns(data));
        }

        if self.config.enable_anomaly {
            patterns.extend(self.detect_anomalies(data));
        }

        // Filter by minimum confidence
        patterns.retain(|p| p.confidence >= self.config.min_confidence);

        patterns
    }

    /// Detect temporal patterns.
    fn detect_temporal_patterns(&self, data: &PatternData) -> Vec<DetectedPattern> {
        let mut patterns = Vec::new();

        // Look for entities that appear at regular intervals
        let mut date_counts: HashMap<&str, usize> = HashMap::new();
        for entity in &data.entity_names {
            *date_counts.entry(entity).or_insert(0) += 1;
        }

        for (entity, count) in &date_counts {
            if *count >= 3 {
                patterns.push(
                    DetectedPattern::new(
                        PatternType::Temporal,
                        &format!(
                            "Entity '{}' appears {} times, suggesting regular activity pattern",
                            entity, count
                        ),
                    )
                    .with_confidence(0.7)
                    .with_entities(vec![entity.to_string()]),
                );
            }
        }

        // Detect rapid succession events
        if data.events.windows(2).any(|w| {
            let diff = (w[1].timestamp - w[0].timestamp).num_hours();
            diff < 24
        }) {
            patterns.push(
                DetectedPattern::new(
                    PatternType::TemporalCorrelation,
                    "Multiple events occurred within 24 hours of each other",
                )
                .with_confidence(0.8),
            );
        }

        patterns
    }

    /// Detect ownership patterns.
    fn detect_ownership_patterns(&self, data: &PatternData) -> Vec<DetectedPattern> {
        let mut patterns = Vec::new();

        // Look for circular ownership
        for cycle in &data.ownership_cycles {
            patterns.push(
                DetectedPattern::new(
                    PatternType::Ownership,
                    &format!("Circular ownership detected: {}", cycle.join(" → ")),
                )
                .with_confidence(0.85)
                .with_entities(cycle.clone()),
            );
        }

        // Look for beneficial owners with multiple entities
        for (owner, entities) in &data.beneficial_owners {
            if entities.len() >= 3 {
                patterns.push(
                    DetectedPattern::new(
                        PatternType::Ownership,
                        &format!(
                            "Beneficial owner '{}' controls {} entities",
                            owner,
                            entities.len()
                        ),
                    )
                    .with_confidence(0.75)
                    .with_entities(
                        std::iter::once(owner.clone())
                            .chain(entities.iter().cloned())
                            .collect(),
                    ),
                );
            }
        }

        patterns
    }

    /// Detect sanctions evasion patterns.
    fn detect_sanctions_patterns(&self, data: &PatternData) -> Vec<DetectedPattern> {
        let mut patterns = Vec::new();

        // Look for entities in sanctioned jurisdictions
        for entity in &data.entities_in_sanctioned {
            if data.high_risk_countries.contains(&entity.1) {
                patterns.push(
                    DetectedPattern::new(
                        PatternType::SanctionsEvasion,
                        &format!(
                            "Entity '{}' linked to sanctioned country '{}'",
                            entity.0, entity.1
                        ),
                    )
                    .with_confidence(0.8)
                    .with_entities(vec![entity.0.clone()]),
                );
            }
        }

        // Look for shell company indicators
        for entity in &data.potential_shells {
            patterns.push(
                DetectedPattern::new(
                    PatternType::ShellCompany,
                    &format!("Shell company indicators for '{}': {}", entity.0, entity.1),
                )
                .with_confidence(0.7)
                .with_entities(vec![entity.0.clone()]),
            );
        }

        patterns
    }

    /// Detect anomalies.
    fn detect_anomalies(&self, data: &PatternData) -> Vec<DetectedPattern> {
        let mut patterns = Vec::new();

        // Look for unusual transaction amounts
        if let Some(stats) = &data.transaction_stats {
            for (entity, amount) in &stats.unusual_amounts {
                patterns.push(
                    DetectedPattern::new(
                        PatternType::Anomaly,
                        &format!(
                            "Unusual transaction amount for '{}': {} (z-score: {:.1})",
                            entity, amount.0, amount.1
                        ),
                    )
                    .with_confidence(0.75)
                    .with_entities(vec![entity.clone()]),
                );
            }
        }

        // Look for velocity anomalies
        for (entity, rate) in &data.activity_velocities {
            if *rate > 10.0 {
                // Arbitrary threshold
                patterns.push(
                    DetectedPattern::new(
                        PatternType::Anomaly,
                        &format!(
                            "High activity velocity for '{}': {:.1} events/day",
                            entity, rate
                        ),
                    )
                    .with_confidence(0.65)
                    .with_entities(vec![entity.clone()]),
                );
            }
        }

        patterns
    }
}

/// Data input for pattern detection.
#[derive(Debug, Clone, Default)]
pub struct PatternData {
    pub entity_names: Vec<String>,
    pub events: Vec<PatternEvent>,
    pub ownership_cycles: Vec<Vec<String>>,
    pub beneficial_owners: HashMap<String, Vec<String>>,
    pub entities_in_sanctioned: Vec<(String, String)>,
    pub potential_shells: Vec<(String, String)>,
    pub transaction_stats: Option<TransactionStats>,
    pub activity_velocities: HashMap<String, f64>,
    pub high_risk_countries: Vec<String>,
}

/// An event for temporal analysis.
#[derive(Debug, Clone)]
pub struct PatternEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub description: String,
    pub entity: Option<String>,
}

/// Transaction statistics for anomaly detection.
#[derive(Debug, Clone)]
pub struct TransactionStats {
    pub unusual_amounts: Vec<(String, (String, f64))>, // (entity, (amount, z-score))
    pub avg_amount: f64,
    pub std_dev: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn pattern_type_severity() {
        assert_eq!(
            PatternType::MoneyLaundering.default_severity(),
            InsightSeverity::Critical
        );
        assert_eq!(
            PatternType::SanctionsEvasion.default_severity(),
            InsightSeverity::Critical
        );
        assert_eq!(
            PatternType::ShellCompany.default_severity(),
            InsightSeverity::High
        );
        assert_eq!(
            PatternType::Temporal.default_severity(),
            InsightSeverity::Low
        );
    }

    #[test]
    fn pattern_detector_config() {
        let config = PatternDetectorConfig::default();
        assert!(config.enable_temporal);
        assert!(config.enable_ownership);
        assert!(config.enable_sanctions);
        assert!(config.enable_anomaly);
        assert_eq!(config.min_confidence, 0.6);
    }

    #[test]
    fn detected_pattern_builder() {
        let pattern = DetectedPattern::new(PatternType::Ownership, "Test ownership pattern")
            .with_confidence(0.85)
            .with_entities(vec!["Entity A".to_string(), "Entity B".to_string()])
            .with_evidence(vec!["Source 1".to_string(), "Source 2".to_string()]);

        assert_eq!(pattern.pattern_type, PatternType::Ownership);
        assert_eq!(pattern.confidence, 0.85);
        assert_eq!(pattern.entities_involved.len(), 2);
        assert_eq!(pattern.evidence.len(), 2);
    }

    #[test]
    fn pattern_to_insight() {
        let pattern = DetectedPattern::new(PatternType::Anomaly, "Unusual activity detected")
            .with_confidence(0.8)
            .with_entities(vec!["Company X".to_string()]);

        let insight = pattern.to_insight("Activity Spike");

        assert!(insight.title.contains("anomaly"));
        assert!(insight.title.contains("Activity Spike"));
        assert!(insight.description.contains("Unusual activity"));
        assert_eq!(insight.severity, InsightSeverity::High);
    }

    #[test]
    fn temporal_pattern_detection() {
        let detector = PatternDetector::with_default_config();
        let data = PatternData {
            entity_names: vec![
                "Company A".to_string(),
                "Company A".to_string(),
                "Company A".to_string(),
                "Company B".to_string(),
            ],
            ..Default::default()
        };

        let patterns = detector.detect_all(&data);
        assert!(!patterns.is_empty());
    }

    #[test]
    fn ownership_cycle_detection() {
        let detector = PatternDetector::with_default_config();
        let mut data = PatternData::default();

        // Add circular ownership
        data.ownership_cycles.push(vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "A".to_string(),
        ]);

        let patterns = detector.detect_all(&data);
        assert!(patterns.iter().any(|p| {
            p.pattern_type == PatternType::Ownership && p.description.contains("Circular ownership")
        }));
    }

    #[test]
    fn sanctions_pattern_detection() {
        let detector = PatternDetector::with_default_config();
        let data = PatternData {
            high_risk_countries: vec!["Russia".to_string(), "Iran".to_string()],
            entities_in_sanctioned: vec![
                ("Entity X".to_string(), "Russia".to_string()),
                ("Entity Y".to_string(), "Unknown".to_string()),
            ],
            ..Default::default()
        };

        let patterns = detector.detect_all(&data);
        assert!(patterns
            .iter()
            .any(|p| p.pattern_type == PatternType::SanctionsEvasion));
    }

    #[test]
    fn shell_company_detection() {
        let detector = PatternDetector::with_default_config();
        let data = PatternData {
            potential_shells: vec![(
                "Shell Corp".to_string(),
                "Single director, same address as 20 other companies".to_string(),
            )],
            ..Default::default()
        };

        let patterns = detector.detect_all(&data);
        assert!(patterns
            .iter()
            .any(|p| p.pattern_type == PatternType::ShellCompany));
    }

    #[test]
    fn confidence_filtering() {
        let config = PatternDetectorConfig {
            min_confidence: 0.8,
            ..Default::default()
        };
        let detector = PatternDetector::new(config);
        let mut data = PatternData::default();

        data.ownership_cycles
            .push(vec!["A".to_string(), "B".to_string(), "A".to_string()]);

        let patterns = detector.detect_all(&data);
        assert!(patterns.iter().all(|p| p.confidence >= 0.8));
    }
}
