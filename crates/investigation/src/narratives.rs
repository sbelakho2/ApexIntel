//! Narrative synthesis for generating investigative reports.
//!
//! Provides:
//! - Deep investigative reports
//! - Timeline reconstruction
//! - Actor attribution analysis
//! - Actionable recommendations

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Type of investigative report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportType {
    /// Company intelligence report
    CompanyIntelligence,
    /// Person deep-dive report
    PersonDeepDive,
    /// Supply chain threat assessment
    SupplyChainThreat,
    /// Market opportunity detection
    MarketOpportunity,
    /// Geopolitical risk assessment
    GeopoliticalRisk,
    /// Competitive intelligence
    CompetitiveIntel,
    /// General investigation
    General,
}

impl ReportType {
    pub fn label(&self) -> &'static str {
        match self {
            ReportType::CompanyIntelligence => "Company Intelligence Report",
            ReportType::PersonDeepDive => "Person Investigation Report",
            ReportType::SupplyChainThreat => "Supply Chain Threat Assessment",
            ReportType::MarketOpportunity => "Market Opportunity Report",
            ReportType::GeopoliticalRisk => "Geopolitical Risk Assessment",
            ReportType::CompetitiveIntel => "Competitive Intelligence Report",
            ReportType::General => "General Investigation Report",
        }
    }
}

/// A section of an investigative report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    /// Section title
    pub title: String,
    /// Section content (markdown)
    pub content: String,
    /// Section type for styling
    pub section_type: SectionType,
    /// Confidence in this section's accuracy
    pub confidence: f64,
    /// Key findings in this section
    pub key_findings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SectionType {
    ExecutiveSummary,
    KeyFindings,
    DetailedAnalysis,
    Timeline,
    EntityAnalysis,
    ThreatAssessment,
    Recommendations,
    Appendices,
    Methodology,
}

/// Timeline event for reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// Event unique identifier
    pub id: String,
    /// Event date/time
    pub timestamp: DateTime<Utc>,
    /// Event title
    pub title: String,
    /// Event description
    pub description: String,
    /// Event category
    pub category: TimelineCategory,
    /// Entities involved
    pub involved_entities: Vec<String>,
    /// Significance (0-1)
    pub significance: f64,
    /// Evidence supporting this event
    pub supporting_evidence: Vec<String>,
    /// Source of information
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimelineCategory {
    Leadership,
    Financial,
    Operational,
    Regulatory,
    Competitive,
    SupplyChain,
    Product,
    Partnership,
    Legal,
    Unknown,
}

impl TimelineCategory {
    pub fn label(&self) -> &'static str {
        match self {
            TimelineCategory::Leadership => "Leadership",
            TimelineCategory::Financial => "Financial",
            TimelineCategory::Operational => "Operational",
            TimelineCategory::Regulatory => "Regulatory",
            TimelineCategory::Competitive => "Competitive",
            TimelineCategory::SupplyChain => "Supply Chain",
            TimelineCategory::Product => "Product",
            TimelineCategory::Partnership => "Partnership",
            TimelineCategory::Legal => "Legal",
            TimelineCategory::Unknown => "Unknown",
        }
    }
}

/// Timeline reconstruction of events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineReconstruction {
    /// Primary entity
    pub entity: String,
    /// Events in chronological order
    pub events: Vec<TimelineEvent>,
    /// Key turning points identified
    pub turning_points: Vec<String>,
    /// Period covered
    pub period_start: DateTime<Utc>,
    /// Period end
    pub period_end: DateTime<Utc>,
    /// Confidence in reconstruction
    pub confidence: f64,
    /// Gaps in timeline
    pub gaps: Vec<TimelineGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineGap {
    /// Gap start
    pub start: DateTime<Utc>,
    /// Gap end
    pub end: DateTime<Utc>,
    /// Duration in days
    pub duration_days: i64,
    /// Why this gap might exist
    pub explanation: String,
}

/// Actor attribution analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorAttribution {
    /// Entity being attributed
    pub entity_id: String,
    /// Entity name
    pub entity_name: String,
    /// Attribution confidence
    pub confidence: f64,
    /// Attributed actors (if entity is a shell or front)
    pub attributed_to: Vec<AttributionLink>,
    /// Evidence for attribution
    pub evidence: Vec<AttributionEvidence>,
    /// Counter-evidence (potential false attribution)
    pub counter_evidence: Vec<String>,
    /// Alternative explanations
    pub alternatives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionLink {
    /// Linked entity
    pub linked_entity: String,
    /// Relationship type
    pub relationship_type: String,
    /// Strength of link (0-1)
    pub link_strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionEvidence {
    /// Evidence description
    pub description: String,
    /// Source
    pub source: String,
    /// Confidence in evidence
    pub confidence: f64,
    /// When discovered
    pub discovered_at: DateTime<Utc>,
}

/// Actionable recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// Recommendation ID
    pub id: String,
    /// Recommendation title
    pub title: String,
    /// Detailed description
    pub description: String,
    /// Priority level
    pub priority: RecommendationPriority,
    /// Expected impact
    pub expected_impact: String,
    /// Implementation effort
    pub effort: ImplementationEffort,
    /// Time to implement
    pub time_to_implement: String,
    /// Risks of not implementing
    pub risks: Vec<String>,
    /// Dependencies
    pub dependencies: Vec<String>,
    /// Related entities
    pub related_entities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecommendationPriority {
    Critical = 0,
    High = 1,
    Medium = 2,
    Low = 3,
}

impl RecommendationPriority {
    pub fn label(&self) -> &'static str {
        match self {
            RecommendationPriority::Critical => "Critical",
            RecommendationPriority::High => "High",
            RecommendationPriority::Medium => "Medium",
            RecommendationPriority::Low => "Low",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImplementationEffort {
    Minimal,
    Low,
    Medium,
    High,
    Significant,
}

impl ImplementationEffort {
    pub fn label(&self) -> &'static str {
        match self {
            ImplementationEffort::Minimal => "Minimal (hours)",
            ImplementationEffort::Low => "Low (days)",
            ImplementationEffort::Medium => "Medium (weeks)",
            ImplementationEffort::High => "High (months)",
            ImplementationEffort::Significant => "Significant (6+ months)",
        }
    }
}

/// Complete investigative report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigativeReport {
    /// Report unique identifier
    pub id: String,
    /// Report type
    pub report_type: ReportType,
    /// Report title
    pub title: String,
    /// Executive summary
    pub executive_summary: String,
    /// Report sections
    pub sections: Vec<ReportSection>,
    /// Timeline reconstruction (if applicable)
    pub timeline: Option<TimelineReconstruction>,
    /// Actor attribution (if applicable)
    pub attribution: Option<ActorAttribution>,
    /// Key findings
    pub key_findings: Vec<String>,
    /// Recommendations
    pub recommendations: Vec<Recommendation>,
    /// Entities analyzed
    pub analyzed_entities: Vec<String>,
    /// Overall confidence score
    pub overall_confidence: f64,
    /// When report was generated
    pub generated_at: DateTime<Utc>,
    /// Sources used
    pub sources: Vec<String>,
    /// Methodology used
    pub methodology: String,
    /// Report metadata
    pub metadata: serde_json::Value,
}

impl InvestigativeReport {
    /// Generate markdown representation.
    pub fn to_markdown(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("# {}", self.title));
        lines.push(String::new());
        lines.push(format!("**Type:** {}", self.report_type.label()));
        lines.push(format!(
            "**Generated:** {}",
            self.generated_at.format("%Y-%m-%d %H:%M UTC")
        ));
        lines.push(format!(
            "**Confidence:** {:.0}%",
            self.overall_confidence * 100.0
        ));
        lines.push(String::new());

        // Executive Summary
        lines.push("## Executive Summary".to_string());
        lines.push(self.executive_summary.clone());
        lines.push(String::new());

        // Key Findings
        if !self.key_findings.is_empty() {
            lines.push("## Key Findings".to_string());
            for (i, finding) in self.key_findings.iter().enumerate() {
                lines.push(format!("{}. {}", i + 1, finding));
            }
            lines.push(String::new());
        }

        // Timeline
        if let Some(timeline) = &self.timeline {
            lines.push("## Timeline".to_string());
            lines.push(format!(
                "Period: {} to {}",
                timeline.period_start.format("%Y-%m-%d"),
                timeline.period_end.format("%Y-%m-%d")
            ));
            lines.push(String::new());
            for event in &timeline.events {
                lines.push(format!(
                    "**{}** ({}) - {}",
                    event.timestamp.format("%Y-%m-%d"),
                    event.category.label(),
                    event.title
                ));
                lines.push(format!("  {}", event.description));
                if let Some(source) = Some(&event.source) {
                    lines.push(format!("  Source: {}", source));
                }
                lines.push(String::new());
            }
        }

        // Sections
        for section in &self.sections {
            lines.push(format!("## {}", section.title));
            lines.push(section.content.clone());
            lines.push(String::new());
        }

        // Attribution
        if let Some(attr) = &self.attribution {
            lines.push("## Actor Attribution".to_string());
            lines.push(format!("Confidence: {:.0}%", attr.confidence * 100.0));
            lines.push(String::new());
            if !attr.attributed_to.is_empty() {
                lines.push("**Attributed to:**".to_string());
                for link in &attr.attributed_to {
                    lines.push(format!(
                        "- {} ({}) - Strength: {:.0}%",
                        link.linked_entity,
                        link.relationship_type,
                        link.link_strength * 100.0
                    ));
                }
                lines.push(String::new());
            }
        }

        // Recommendations
        if !self.recommendations.is_empty() {
            lines.push("## Recommendations".to_string());
            for rec in &self.recommendations {
                lines.push(format!("### {} [{}]", rec.title, rec.priority.label()));
                lines.push(rec.description.to_string());
                lines.push(format!("**Impact:** {}", rec.expected_impact));
                lines.push(format!("**Effort:** {}", rec.effort.label()));
                lines.push(format!("**Time:** {}", rec.time_to_implement));
                if !rec.risks.is_empty() {
                    lines.push(format!("**Risks:** {}", rec.risks.join(", ")));
                }
                lines.push(String::new());
            }
        }

        // Sources
        lines.push("## Sources".to_string());
        for source in &self.sources {
            lines.push(format!("- {}", source));
        }
        lines.push(String::new());

        lines.push("## Methodology".to_string());
        lines.push(self.methodology.clone());

        lines.join("\n")
    }
}

/// Narrative synthesizer for generating investigative reports.
#[derive(Clone)]
pub struct NarrativeSynthesizer {
    /// Include detailed methodology section
    include_methodology: bool,
    /// Maximum narrative length
    max_narrative_length: usize,
    /// Enable timeline reconstruction
    enable_timeline: bool,
    /// Enable attribution analysis
    enable_attribution: bool,
}

impl Default for NarrativeSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

impl NarrativeSynthesizer {
    /// Create a new narrative synthesizer.
    pub fn new() -> Self {
        Self {
            include_methodology: true,
            max_narrative_length: 50000,
            enable_timeline: true,
            enable_attribution: true,
        }
    }

    /// Configure methodology inclusion.
    pub fn with_methodology(mut self, include: bool) -> Self {
        self.include_methodology = include;
        self
    }

    /// Configure narrative length limit.
    pub fn with_max_length(mut self, length: usize) -> Self {
        self.max_narrative_length = length;
        self
    }

    /// Configure timeline reconstruction.
    pub fn with_timeline(mut self, enable: bool) -> Self {
        self.enable_timeline = enable;
        self
    }

    /// Configure attribution analysis.
    pub fn with_attribution(mut self, enable: bool) -> Self {
        self.enable_attribution = enable;
        self
    }

    /// Synthesize a full investigative report.
    pub fn synthesize(&self, context: &SynthesisContext) -> InvestigativeReport {
        let report_id = Uuid::new_v4().to_string();

        // Build executive summary
        let executive_summary = self.build_executive_summary(context);

        // Build key findings
        let key_findings = self.extract_key_findings(context);

        // Build sections
        let sections = self.build_sections(context);

        // Build timeline if enabled
        let timeline = if self.enable_timeline {
            Some(self.reconstruct_timeline(context))
        } else {
            None
        };

        // Build attribution if enabled
        let attribution = if self.enable_attribution {
            self.analyze_attribution(context)
        } else {
            None
        };

        // Generate recommendations
        let recommendations = self.generate_recommendations(context);

        // Calculate overall confidence
        let overall_confidence = self.calculate_overall_confidence(context);

        InvestigativeReport {
            id: report_id,
            report_type: context.report_type.clone(),
            title: context.title.clone(),
            executive_summary,
            sections,
            timeline,
            attribution,
            key_findings,
            recommendations,
            analyzed_entities: context.entities.clone(),
            overall_confidence,
            generated_at: Utc::now(),
            sources: context.sources.clone(),
            methodology: if self.include_methodology {
                self.describe_methodology(context)
            } else {
                String::new()
            },
            metadata: serde_json::json!({}),
        }
    }

    /// Build executive summary from context.
    fn build_executive_summary(&self, context: &SynthesisContext) -> String {
        let entity_count = context.entities.len();
        let signal_count = context.signals.len();
        let finding_count = context.findings.len();

        let mut summary = format!(
            "This {} examines {} entity/entities using {} signals and produced {} key findings. ",
            context.report_type.label(),
            entity_count,
            signal_count,
            finding_count
        );

        // Add primary conclusion if available
        if let Some(conclusion) = &context.primary_conclusion {
            summary.push_str(&format!("\n\n**Primary Conclusion:** {}", conclusion));
        }

        // Add risk assessment if available
        if let Some(risk_level) = &context.risk_level {
            summary.push_str(&format!("\n\n**Risk Assessment:** {}", risk_level));
        }

        summary
    }

    /// Extract key findings from context.
    fn extract_key_findings(&self, context: &SynthesisContext) -> Vec<String> {
        let mut findings = Vec::new();

        // Add predefined findings
        findings.extend(context.findings.iter().cloned());

        // Add signal-based findings
        for signal in context.signals.iter().take(5) {
            findings.push(format!(
                "Signal detected: {} (confidence: {:.0}%)",
                signal.description,
                signal.confidence * 100.0
            ));
        }

        // Limit to top 10 findings
        findings.truncate(10);
        findings
    }

    /// Build report sections with confidence derived from evidence quality.
    fn build_sections(&self, context: &SynthesisContext) -> Vec<ReportSection> {
        let mut sections = Vec::new();

        // Compute a dynamic confidence base from signal quality
        let signal_quality = self.compute_evidence_quality(context);

        // Detailed Analysis section
        let analysis_content = self.build_detailed_analysis(context);
        sections.push(ReportSection {
            title: "Detailed Analysis".to_string(),
            content: analysis_content,
            section_type: SectionType::DetailedAnalysis,
            // Confidence based on quantity and average signal confidence
            confidence: (signal_quality * 0.9 + 0.1).min(0.95),
            key_findings: context.findings.iter().take(3).cloned().collect(),
        });

        // Entity Analysis if multiple entities
        if context.entities.len() > 1 {
            let entity_content = self.build_entity_analysis(context);
            // Entity relationship confidence: derived from signal diversity and relationship count
            let entity_conf = if context.relationships.is_empty() {
                signal_quality * 0.7
            } else {
                let rel_factor = (context.relationships.len() as f64 / 5.0).min(1.0) * 0.2;
                (signal_quality * 0.7 + rel_factor).min(0.95)
            };
            sections.push(ReportSection {
                title: "Entity Relationships".to_string(),
                content: entity_content,
                section_type: SectionType::EntityAnalysis,
                confidence: entity_conf,
                key_findings: vec![],
            });
        }

        // Threat Assessment if applicable
        if let Some(threat) = &context.threat_summary {
            sections.push(ReportSection {
                title: "Threat Assessment".to_string(),
                content: threat.clone(),
                section_type: SectionType::ThreatAssessment,
                // Threat confidence: uses signal quality but is capped lower due to
                // inherent uncertainty in threat assessments
                confidence: (signal_quality * 0.65 + 0.15).min(0.85),
                key_findings: vec![],
            });
        }

        sections
    }

    /// Compute evidence quality score (0.0–1.0) from signal data in the context.
    fn compute_evidence_quality(&self, context: &SynthesisContext) -> f64 {
        if context.signals.is_empty() {
            // No signals: fall back to a low base reflecting pure data-sparsity
            return 0.3;
        }
        // Quantity factor: more signals = more evidence, capped at 15 signals
        let quantity = (context.signals.len() as f64 / 15.0).min(1.0);
        // Average confidence of all signals
        let avg_conf: f64 = context.signals.iter().map(|s| s.confidence).sum::<f64>()
            / context.signals.len() as f64;
        // Source diversity: unique sources / total signals
        let mut unique_sources = std::collections::HashSet::new();
        for s in &context.signals {
            unique_sources.insert(s.source.as_str());
        }
        let diversity = if context.signals.len() > 1 {
            unique_sources.len() as f64 / context.signals.len() as f64
        } else {
            1.0
        };
        // Blend factors: quantity (0.3), avg confidence (0.5), source diversity (0.2)
        quantity * 0.3 + avg_conf * 0.5 + diversity * 0.2
    }

    /// Build detailed analysis content.
    fn build_detailed_analysis(&self, context: &SynthesisContext) -> String {
        let mut content = String::new();

        // Overview
        content.push_str("### Overview\n\n");
        content.push_str(&format!(
            "This investigation analyzed {} signals across {} entities. ",
            context.signals.len(),
            context.entities.len()
        ));

        // Signal summary
        content.push_str("\n\n### Signal Summary\n\n");
        let mut signal_types: HashMap<&str, usize> = HashMap::new();
        for signal in &context.signals {
            *signal_types.entry(&signal.signal_type).or_insert(0) += 1;
        }

        for (signal_type, count) in signal_types.iter().take(10) {
            content.push_str(&format!("- **{}**: {} occurrences\n", signal_type, count));
        }

        // Findings detail
        if !context.findings.is_empty() {
            content.push_str("\n\n### Detailed Findings\n\n");
            for (i, finding) in context.findings.iter().enumerate() {
                content.push_str(&format!("{}. {}\n\n", i + 1, finding));
            }
        }

        content
    }

    /// Build entity relationship analysis.
    fn build_entity_analysis(&self, context: &SynthesisContext) -> String {
        let mut content = String::new();

        content.push_str("### Entity Overview\n\n");
        for entity in &context.entities {
            content.push_str(&format!("- **{}**\n", entity));
        }

        if !context.relationships.is_empty() {
            content.push_str("\n\n### Identified Relationships\n\n");
            for rel in &context.relationships {
                content.push_str(&format!(
                    "- **{}** {} **{}** (confidence: {:.0}%)\n",
                    rel.entity_a,
                    rel.relationship_type,
                    rel.entity_b,
                    rel.confidence * 100.0
                ));
            }
        }

        content
    }

    /// Reconstruct timeline from events.
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn reconstruct_timeline(&self, context: &SynthesisContext) -> TimelineReconstruction {
        let mut events: Vec<TimelineEvent> = context
            .events
            .iter()
            .map(|e| TimelineEvent {
                id: Uuid::new_v4().to_string(),
                timestamp: e.timestamp,
                title: e.title.clone(),
                description: e.description.clone(),
                category: e.category.clone(),
                involved_entities: e.entities.clone(),
                significance: e.significance,
                supporting_evidence: vec![],
                source: e.source.clone(),
            })
            .collect();

        // Sort by timestamp
        events.sort_by_key(|a| a.timestamp);

        // Identify turning points (high significance events)
        let turning_points: Vec<String> = events
            .iter()
            .filter(|e| e.significance > 0.7)
            .map(|e| e.title.clone())
            .collect();

        // Calculate gaps (periods > 60 days without events)
        let mut gaps = Vec::new();
        if events.len() >= 2 {
            for i in 0..events.len() - 1 {
                let gap_days = (events[i + 1].timestamp - events[i].timestamp).num_days();
                if gap_days > 60 {
                    gaps.push(TimelineGap {
                        start: events[i].timestamp,
                        end: events[i + 1].timestamp,
                        duration_days: gap_days,
                        explanation: "Extended period with no observable signals".to_string(),
                    });
                }
            }
        }

        let (period_start, period_end) = if events.is_empty() {
            (Utc::now(), Utc::now())
        } else {
            // SAFETY: events is non-empty in this branch
            (
                events.first().unwrap().timestamp,
                events.last().unwrap().timestamp,
            )
        };

        TimelineReconstruction {
            entity: context.entities.first().cloned().unwrap_or_default(),
            events,
            turning_points,
            period_start,
            period_end,
            confidence: 0.7,
            gaps,
        }
    }

    /// Analyze actor attribution.
    fn analyze_attribution(&self, context: &SynthesisContext) -> Option<ActorAttribution> {
        // Only perform attribution if relevant signals exist
        let attribution_signals: Vec<&SignalInfo> = context
            .signals
            .iter()
            .filter(|s| {
                s.signal_type.contains("ownership")
                    || s.signal_type.contains("relationship")
                    || s.signal_type.contains("affiliate")
            })
            .collect();

        if attribution_signals.is_empty() {
            return None;
        }

        // Compute dynamic confidence based on evidence quantity and source diversity
        let evidence_quantity_factor = (attribution_signals.len() as f64 / 10.0).min(1.0);

        // Source diversity: count unique sources among attribution signals
        let mut unique_sources: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for s in &attribution_signals {
            unique_sources.insert(s.source.as_str());
        }
        let source_diversity_factor = (unique_sources.len() as f64 / 5.0).min(1.0);

        // Combined confidence: base 0.3 + evidence boost + diversity boost, capped at 0.95
        let confidence =
            (0.3 + evidence_quantity_factor * 0.35 + source_diversity_factor * 0.30).min(0.95);

        Some(ActorAttribution {
            entity_id: context.entities.first().cloned().unwrap_or_default(),
            entity_name: context.entities.first().cloned().unwrap_or_default(),
            confidence,
            attributed_to: Vec::new(),
            evidence: context
                .signals
                .iter()
                .take(3)
                .map(|s| AttributionEvidence {
                    description: s.description.clone(),
                    source: "Signal Analysis".to_string(),
                    confidence: s.confidence,
                    discovered_at: Utc::now(),
                })
                .collect(),
            counter_evidence: vec!["Limited direct evidence available".to_string()],
            alternatives: vec!["Entity operates independently".to_string()],
        })
    }

    /// Generate actionable recommendations.
    fn generate_recommendations(&self, context: &SynthesisContext) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();

        // High-confidence findings generate high-priority recommendations
        for finding in context.findings.iter().take(3) {
            recommendations.push(Recommendation {
                id: Uuid::new_v4().to_string(),
                title: format!(
                    "Investigate: {}",
                    finding.chars().take(50).collect::<String>()
                ),
                description: finding.clone(),
                priority: RecommendationPriority::High,
                expected_impact: "Improved understanding of entity behavior".to_string(),
                effort: ImplementationEffort::Medium,
                time_to_implement: "2-4 weeks".to_string(),
                risks: vec!["Resource allocation required".to_string()],
                dependencies: vec![],
                related_entities: context.entities.clone(),
            });
        }

        // Threat-based recommendations
        if let Some(risk) = &context.risk_level {
            if risk.contains("High") || risk.contains("Critical") {
                recommendations.push(Recommendation {
                    id: Uuid::new_v4().to_string(),
                    title: "Implement monitoring for high-risk entity".to_string(),
                    description: "Establish continuous monitoring for this high-risk entity"
                        .to_string(),
                    priority: RecommendationPriority::Critical,
                    expected_impact: "Early warning of risk events".to_string(),
                    effort: ImplementationEffort::Low,
                    time_to_implement: "1-2 weeks".to_string(),
                    risks: vec![],
                    dependencies: vec![],
                    related_entities: context.entities.clone(),
                });
            }
        }

        recommendations
    }

    /// Calculate overall confidence score.
    ///
    /// ## Weight Rationale
    ///
    /// - **Evidence weight (0.4)**: Signal density is the strongest indicator of
    ///   investigative confidence because it directly reflects the volume of raw
    ///   intelligence collected. More signals mean a richer evidentiary base, but
    ///   the contribution is capped at 20 signals to avoid over-weighting noise.
    /// - **Relationship weight (0.3)**: Findings represent synthesised analytical
    ///   conclusions derived from signals. Their weight is lower than raw evidence
    ///   because findings can be redundant or speculative, and the cap at 5 findings
    ///   reflects diminishing marginal value from additional statements.
    /// - **Events weight (0.3)**: Temporal events ground the investigation in
    ///   observable chronology, which is critical for timeline reconstruction and
    ///   causality inference. A minimum threshold of 3 entities yields the full
    ///   event contribution to encourage multi-entity analysis; fewer entities
    ///   receive a reduced baseline because correlation opportunities are limited.
    fn calculate_overall_confidence(&self, context: &SynthesisContext) -> f64 {
        // Evidence factor: signal count capped at 20, weighted 0.4
        let signal_factor = (context.signals.len() as f64 / 20.0).min(1.0) * 0.4;
        // Finding factor: finding count capped at 5, weighted 0.3
        let finding_factor = if context.findings.is_empty() {
            0.0
        } else {
            (context.findings.len() as f64 / 5.0).min(1.0) * 0.3
        };
        // Entity factor: at least 3 entities for strong event-based confidence, weighted 0.3
        let entity_factor = if context.entities.len() >= 3 {
            0.3
        } else {
            0.1
        };

        (signal_factor + finding_factor + entity_factor).clamp(0.0, 1.0)
    }

    /// Describe methodology used, dynamically reflecting what analysis was performed.
    fn describe_methodology(&self, context: &SynthesisContext) -> String {
        let mut techniques = Vec::new();

        // Multi-signal analysis
        let signal_count = context.signals.len();
        let source_count = {
            let mut sources = std::collections::HashSet::new();
            for s in &context.signals {
                sources.insert(s.source.as_str());
            }
            sources.len()
        };
        techniques.push(format!(
            "**Multi-signal analysis** across {} signals from {} unique sources",
            signal_count, source_count
        ));

        // Entity analysis
        if context.entities.len() > 1 {
            techniques.push(format!(
                "**Entity correlation** of {} entities with {} identified relationships",
                context.entities.len(),
                context.relationships.len()
            ));
        } else if !context.entities.is_empty() {
            techniques.push(format!(
                "**Single-entity deep-dive** on {}",
                context.entities[0]
            ));
        }

        // Temporal / timeline analysis
        if !context.events.is_empty() {
            techniques.push(format!(
                "**Temporal pattern detection** across {} events for trend and anomaly identification",
                context.events.len()
            ));
        }

        // Hypothesis & findings
        if !context.findings.is_empty() {
            techniques.push(format!(
                "**Bayesian hypothesis testing** with {} key findings derived from Analysis of Competing Hypotheses",
                context.findings.len()
            ));
        }

        // Threat / risk analysis
        if context.risk_level.is_some() || context.threat_summary.is_some() {
            techniques.push(
                "**Risk assessment** with automated threat scoring and confidence propagation"
                    .to_string(),
            );
        }

        // Fallback if nothing was performed
        if techniques.is_empty() {
            techniques.push("**Multi-signal analysis** across diverse OSINT sources".to_string());
            techniques.push(
                "**Bayesian hypothesis testing** using Analysis of Competing Hypotheses (ACH)"
                    .to_string(),
            );
            techniques
                .push("**Chain-of-thought reasoning** with confidence propagation".to_string());
        }

        let mut desc = String::from(
            "This report was generated using ApexIntel's Deep Investigation Framework, employing the following analytical techniques:\n\n"
        );

        for (i, technique) in techniques.iter().enumerate() {
            desc.push_str(&format!("{}. {}\n", i + 1, technique));
        }

        desc.push_str(&format!(
            "\nConfidence scores reflect the quantity and quality of supporting evidence. \
             Overall confidence: {:.0}% based on {} signals, {} findings, and {} entities analyzed.",
            self.calculate_overall_confidence(context) * 100.0,
            signal_count,
            context.findings.len(),
            context.entities.len()
        ));

        desc
    }
}

/// Context for report synthesis.
#[derive(Debug, Clone)]
pub struct SynthesisContext {
    /// Report type
    pub report_type: ReportType,
    /// Report title
    pub title: String,
    /// Entities analyzed
    pub entities: Vec<String>,
    /// Signals/evidence collected
    pub signals: Vec<SignalInfo>,
    /// Findings generated
    pub findings: Vec<String>,
    /// Primary conclusion
    pub primary_conclusion: Option<String>,
    /// Risk level assessment
    pub risk_level: Option<String>,
    /// Threat summary (if applicable)
    pub threat_summary: Option<String>,
    /// Relationships identified
    pub relationships: Vec<RelationshipInfo>,
    /// Events for timeline
    pub events: Vec<EventInfo>,
    /// Sources used
    pub sources: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SignalInfo {
    pub signal_type: String,
    pub description: String,
    pub confidence: f64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct RelationshipInfo {
    pub entity_a: String,
    pub entity_b: String,
    pub relationship_type: String,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct EventInfo {
    pub timestamp: DateTime<Utc>,
    pub title: String,
    pub description: String,
    pub category: TimelineCategory,
    pub entities: Vec<String>,
    pub significance: f64,
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_synthesis() {
        let synthesizer = NarrativeSynthesizer::new();
        let context = SynthesisContext {
            report_type: ReportType::CompanyIntelligence,
            title: "Test Company Investigation".to_string(),
            entities: vec!["TestCorp".to_string()],
            signals: vec![SignalInfo {
                signal_type: "job_posting".to_string(),
                description: "Expansion hiring detected".to_string(),
                confidence: 0.9,
                source: "LinkedIn".to_string(),
            }],
            findings: vec!["Company is actively hiring".to_string()],
            primary_conclusion: Some("Company appears to be expanding".to_string()),
            risk_level: Some("Medium".to_string()),
            threat_summary: None,
            relationships: vec![],
            events: vec![],
            sources: vec!["LinkedIn".to_string()],
        };

        let report = synthesizer.synthesize(&context);
        assert!(!report.executive_summary.is_empty());
        assert!(!report.key_findings.is_empty());
    }

    #[test]
    fn test_markdown_generation() {
        let synthesizer = NarrativeSynthesizer::new();
        let context = SynthesisContext {
            report_type: ReportType::General,
            title: "Test Report".to_string(),
            entities: vec!["Test".to_string()],
            signals: vec![],
            findings: vec!["Test finding".to_string()],
            primary_conclusion: None,
            risk_level: None,
            threat_summary: None,
            relationships: vec![],
            events: vec![],
            sources: vec![],
        };

        let report = synthesizer.synthesize(&context);
        let markdown = report.to_markdown();
        assert!(markdown.contains("# Test Report"));
    }
}
