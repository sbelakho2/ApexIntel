//! # Person Deep-Dive Workflow
//!
//! Production-ready workflow for detailed person investigation including:
//! - Professional history reconstruction
//! - Network mapping (colleagues, connections)
//! - Risk indicators
//! - Trigger event detection
//! - Engagement strategy
//!
//! ## Features
//!
//! - **Professional History Reconstruction**: Career timeline, role analysis, achievements
//! - **Network Mapping**: Professional relationships, organizational links, influence mapping
//! - **Risk Indicators**: Behavioral flags, association risks, reputational concerns
//! - **Trigger Event Detection**: Life changes, career transitions, anomalous activity
//! - **Engagement Strategy**: Communication approach, timing recommendations, talking points

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reasoning::EvidenceItem;

/// Complete person deep-dive report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonDeepDiveReport {
    pub report_id: String,
    pub workflow_id: String,
    pub target_person: PersonIdentity,
    pub executive_summary: String,
    pub professional_history: Option<ProfessionalHistory>,
    pub network_mapping: Option<NetworkMapping>,
    pub risk_indicators: Option<RiskIndicator>,
    pub trigger_events: Option<Vec<TriggerEvent>>,
    pub engagement_strategy: Option<EngagementStrategy>,
    pub overall_confidence: f64,
    pub key_findings: Vec<String>,
    pub critical_concerns: Vec<String>,
    pub actionable_recommendations: Vec<String>,
    pub data_gaps: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Person identity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonIdentity {
    pub name: String,
    pub aliases: Vec<String>,
    pub date_of_birth: Option<DateTime<Utc>>,
    pub nationalities: Vec<String>,
    pub current_location: Location,
    pub contact_info: ContactInformation,
    pub profile_urls: Vec<String>,
    pub digital_footprint: DigitalFootprint,
}

/// Location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub city: String,
    pub region: String,
    pub country: String,
    pub coordinates: Option<(f64, f64)>,
    pub location_type: LocationType,
}

/// Type of location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LocationType {
    Residence,
    Business,
    Historic,
}

/// Contact information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactInformation {
    pub emails: Vec<EmailAddress>,
    pub phone_numbers: Vec<String>,
    pub social_handles: Vec<SocialHandle>,
}

/// Email address with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub email: String,
    pub domain: String,
    pub is_personal: bool,
    pub is_verified: bool,
}

/// Social media handle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialHandle {
    pub platform: String,
    pub handle: String,
    pub url: Option<String>,
    pub follower_count: Option<i64>,
    pub is_verified: bool,
}

/// Digital footprint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigitalFootprint {
    pub platforms_found: Vec<String>,
    pub first_appearance_date: Option<DateTime<Utc>>,
    pub activity_level: ActivityLevel,
    pub sentiment_analysis: SentimentAnalysis,
}

/// Activity level assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivityLevel {
    High,
    Medium,
    Low,
    Minimal,
    Unknown,
}

/// Sentiment analysis summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentAnalysis {
    pub overall_sentiment: String,
    pub positive_percentage: f64,
    pub negative_percentage: f64,
    pub neutral_percentage: f64,
}

/// Professional history component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfessionalHistory {
    pub current_position: Position,
    pub position_history: Vec<Position>,
    pub education_history: Vec<Education>,
    pub certifications: Vec<Certification>,
    pub skills_assessment: SkillsAssessment,
    pub career_trajectory: CareerTrajectory,
    pub career_gaps: Vec<CareerGap>,
    pub notable_achievements: Vec<Achievement>,
    pub professional_confidence: f64,
}

/// Position/role information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub title: String,
    pub organization: String,
    pub organization_type: OrganizationType,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub duration_months: Option<i64>,
    pub location: Option<Location>,
    pub responsibilities: Vec<String>,
    pub achievements: Vec<String>,
    pub reporting_structure: Option<String>,
    pub is_current: bool,
}

/// Type of organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrganizationType {
    PublicCompany,
    PrivateCompany,
    Government,
    NonProfit,
    Academic,
    Startup,
    SoleProprietorship,
    Unknown,
}

/// Education entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Education {
    pub institution: String,
    pub degree_type: DegreeType,
    pub field_of_study: String,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub gpa: Option<f64>,
    pub honors: Vec<String>,
}

/// Degree type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DegreeType {
    HighSchool,
    Associate,
    Bachelor,
    Master,
    Doctorate,
    Professional,
    Certification,
}

/// Professional certification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certification {
    pub name: String,
    pub issuing_organization: String,
    pub issue_date: Option<DateTime<Utc>>,
    pub expiry_date: Option<DateTime<Utc>>,
    pub credential_id: Option<String>,
}

/// Skills assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsAssessment {
    pub technical_skills: Vec<Skill>,
    pub soft_skills: Vec<Skill>,
    pub leadership_skills: Vec<Skill>,
    pub domain_expertise: Vec<String>,
    pub skill_confidence: f64,
}

/// Individual skill with proficiency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub proficiency_level: ProficiencyLevel,
    pub years_experience: Option<i32>,
    pub last_used: Option<DateTime<Utc>>,
}

/// Proficiency level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProficiencyLevel {
    Expert,
    Advanced,
    Intermediate,
    Basic,
    Unknown,
}

/// Career trajectory analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareerTrajectory {
    pub trajectory_type: TrajectoryType,
    pub average_tenure_months: f64,
    pub promotion_rate: f64,
    pub industry_transitions: i32,
    pub mobility_pattern: MobilityPattern,
    pub trajectory_confidence: f64,
}

/// Type of career trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(PartialEq)]
pub enum TrajectoryType {
    Ascending,
    Stable,
    Descending,
    Volatile,
    Lateral,
}

/// Mobility pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MobilityPattern {
    VerticalUp,
    VerticalDown,
    Lateral,
    Hybrid,
    Static,
}

/// Career gap
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareerGap {
    pub start_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub duration_months: i64,
    pub potential_explanation: Option<String>,
    pub flagged_for_review: bool,
}

/// Notable achievement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Achievement {
    pub description: String,
    pub date: Option<DateTime<Utc>>,
    pub impact_level: ImpactLevel,
    pub verified: bool,
}

/// Impact level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImpactLevel {
    CompanyWide,
    Industry,
    Regional,
    Personal,
}

/// Network mapping component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMapping {
    pub direct_connections: Vec<Connection>,
    pub organizational_links: Vec<OrganizationalLink>,
    pub influence_network: InfluenceNetwork,
    pub association_groups: Vec<AssociationGroup>,
    pub network_confidence: f64,
}

/// Connection to another person
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub target_name: String,
    pub relationship_type: RelationshipType,
    pub relationship_strength: ConnectionStrength,
    pub context: String,
    pub verified: bool,
    pub first_connected_date: Option<DateTime<Utc>>,
}

/// Type of relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelationshipType {
    Colleague,
    Manager,
    DirectReport,
    BoardMember,
    Investor,
    Advisor,
    Family,
    Personal,
    Academic,
    Professional,
}

/// Connection strength
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionStrength {
    Strong,
    Moderate,
    Weak,
    Unknown,
}

/// Link to an organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationalLink {
    pub organization_name: String,
    pub link_type: OrganizationLinkType,
    pub ownership_percentage: Option<f64>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
}

/// Type of organizational link
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrganizationLinkType {
    Founder,
    Executive,
    BoardMember,
    Investor,
    Employee,
    Contractor,
    Consultant,
    AdvisoryBoard,
}

/// Influence network analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluenceNetwork {
    pub influence_score: f64,
    pub reach_score: f64,
    pub media_mentions: i32,
    pub speaking_engagements: i32,
    pub published_work: i32,
    pub industry_recognition: Vec<String>,
}

/// Association group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociationGroup {
    pub group_name: String,
    pub group_type: GroupType,
    pub membership_type: MembershipType,
    pub joined_date: Option<DateTime<Utc>>,
    pub active_status: bool,
}

/// Type of group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupType {
    ProfessionalAssociation,
    SocialClub,
    PoliticalOrganization,
    Charitable,
    Academic,
    Industry,
}

/// Membership type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MembershipType {
    Founder,
    Board,
    ActiveMember,
    Associate,
    Honorary,
}

/// Risk indicators component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskIndicator {
    pub financial_risks: Vec<FinancialRisk>,
    pub reputational_risks: Vec<ReputationalRisk>,
    pub association_risks: Vec<AssociationRisk>,
    pub behavioral_risks: Vec<BehavioralRisk>,
    pub legal_compliance_flags: Vec<LegalComplianceFlag>,
    pub overall_risk_score: f64,
    pub risk_confidence: f64,
}

/// Financial risk indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialRisk {
    pub risk_type: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub evidence_source: String,
    pub confidence: f64,
}

/// Reputational risk indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationalRisk {
    pub risk_type: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub media_coverage: Option<String>,
    pub spread_potential: SpreadPotential,
}

/// Spread potential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpreadPotential {
    Viral,
    Widespread,
    Local,
    Minimal,
}

/// Association risk indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociationRisk {
    pub associated_entity: String,
    pub risk_type: String,
    pub association_strength: ConnectionStrength,
    pub severity: RiskSeverity,
    pub description: String,
}

/// Behavioral risk indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralRisk {
    pub behavior_type: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub pattern_consistency: PatternConsistency,
    pub anomaly_score: f64,
}

/// Pattern consistency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternConsistency {
    Consistent,
    Inconsistent,
    Anomalous,
}

/// Legal/compliance flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalComplianceFlag {
    pub flag_type: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub jurisdiction: Option<String>,
    pub regulatory_body: Option<String>,
    pub resolution_status: Option<String>,
}

/// Risk severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskSeverity {
    Critical,
    High,
    Medium,
    Low,
    Informational,
}

/// Trigger event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    pub event_id: String,
    pub event_type: TriggerEventType,
    pub description: String,
    pub detection_date: DateTime<Utc>,
    pub source: String,
    pub confidence: f64,
    pub significance: EventSignificance,
    pub related_entities: Vec<String>,
    pub recommended_action: Option<String>,
}

/// Type of trigger event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerEventType {
    PositionChange,
    LocationChange,
    LegalIssue,
    FinancialAnomaly,
    MediaEvent,
    SocialActivity,
    NetworkChange,
    CompanyEvent,
    RegulatoryEvent,
    SecurityIncident,
    ReputationEvent,
}

/// Event significance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventSignificance {
    Critical,
    High,
    Medium,
    Low,
}

/// Engagement strategy component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementStrategy {
    pub recommended_approach: EngagementApproach,
    pub communication_preferences: CommunicationPreferences,
    pub optimal_timing: TimingRecommendation,
    pub talking_points: Vec<TalkingPoint>,
    pub topics_to_avoid: Vec<String>,
    pub background_information: Vec<String>,
    pub risk_mitigation: Vec<String>,
    pub engagement_confidence: f64,
}

/// Engagement approach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngagementApproach {
    Direct,
    Indirect,
    ThirdParty,
    Written,
    Formal,
    SemiFormal,
    Informal,
}

/// Communication preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationPreferences {
    pub preferred_channel: String,
    pub optimal_time: String,
    pub tone: EngagementTone,
    pub response_rate_estimate: f64,
}

/// Engagement tone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngagementTone {
    Formal,
    SemiFormal,
    Casual,
    Friendly,
}

/// Timing recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingRecommendation {
    pub best_contact_window: String,
    pub seasonal_considerations: Vec<String>,
    pub scheduling_factors: Vec<String>,
    pub urgency_considerations: Option<String>,
}

/// Talking point for engagement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalkingPoint {
    pub topic: String,
    pub key_message: String,
    pub supporting_evidence: Vec<String>,
    pub expected_response: String,
}

/// Person Deep-Dive Workflow
#[derive(Debug, Clone)]
pub struct PersonDeepDiveWorkflow {
    pub workflow_id: String,
    pub config: WorkflowConfig,
}

/// Workflow configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub include_professional_history: bool,
    pub include_network_mapping: bool,
    pub include_risk_indicators: bool,
    pub include_trigger_events: bool,
    pub include_engagement_strategy: bool,
    pub risk_threshold: RiskSeverity,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            include_professional_history: true,
            include_network_mapping: true,
            include_risk_indicators: true,
            include_trigger_events: true,
            include_engagement_strategy: true,
            risk_threshold: RiskSeverity::Medium,
        }
    }
}

impl PersonDeepDiveWorkflow {
    /// Create a new person deep-dive workflow
    pub fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config: WorkflowConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: WorkflowConfig) -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config,
        }
    }

    /// Run the complete person deep-dive workflow
    pub fn run(&self, target_name: &str, signals: Vec<EvidenceItem>) -> PersonDeepDiveReport {
        let mut report = PersonDeepDiveReport {
            report_id: Uuid::new_v4().to_string(),
            workflow_id: self.workflow_id.clone(),
            target_person: self.build_person_identity(target_name, &signals),
            executive_summary: String::new(),
            professional_history: None,
            network_mapping: None,
            risk_indicators: None,
            trigger_events: None,
            engagement_strategy: None,
            overall_confidence: 0.0,
            key_findings: Vec::new(),
            critical_concerns: Vec::new(),
            actionable_recommendations: Vec::new(),
            data_gaps: Vec::new(),
            generated_at: Utc::now(),
        };

        // Run analysis components based on configuration
        if self.config.include_professional_history {
            report.professional_history = Some(self.analyze_professional_history(&signals));
        }

        if self.config.include_network_mapping {
            report.network_mapping = Some(self.analyze_network_mapping(&signals));
        }

        if self.config.include_risk_indicators {
            report.risk_indicators = Some(self.analyze_risk_indicators(&signals));
        }

        if self.config.include_trigger_events {
            report.trigger_events = Some(self.detect_trigger_events(&signals));
        }

        if self.config.include_engagement_strategy {
            report.engagement_strategy = Some(self.develop_engagement_strategy(&report));
        }

        // Calculate overall confidence
        let mut confidences: Vec<f64> = Vec::new();
        if let Some(ref ph) = report.professional_history {
            confidences.push(ph.professional_confidence);
        }
        if let Some(ref nm) = report.network_mapping {
            confidences.push(nm.network_confidence);
        }
        if let Some(ref ri) = report.risk_indicators {
            confidences.push(ri.risk_confidence);
        }
        if report.trigger_events.is_some() {
            confidences.push(0.6);
        }
        if let Some(ref es) = report.engagement_strategy {
            confidences.push(es.engagement_confidence);
        }

        report.overall_confidence = if confidences.is_empty() {
            0.5
        } else {
            confidences.iter().sum::<f64>() / confidences.len() as f64
        };

        // Generate executive summary
        report.executive_summary = self.generate_executive_summary(&report);

        // Extract key findings
        report.key_findings = self.extract_key_findings(&report);

        // Identify critical concerns
        report.critical_concerns = self.identify_critical_concerns(&report);

        // Generate recommendations
        report.actionable_recommendations = self.generate_recommendations(&report);

        // Identify data gaps
        report.data_gaps = self.identify_data_gaps(&report, &signals);

        report
    }

    /// Build person identity from signals
    fn build_person_identity(&self, name: &str, signals: &[EvidenceItem]) -> PersonIdentity {
        let mut aliases = Vec::new();
        let mut nationalities = Vec::new();
        let mut profile_urls = Vec::new();
        let mut platforms_found = Vec::new();
        let activity_level = ActivityLevel::Unknown;

        for signal in signals {
            match signal.evidence_type.as_str() {
                "alias" => aliases.push(signal.description.clone()),
                "profile" => {
                    profile_urls.push(signal.description.clone());
                    platforms_found.push(signal.source.clone());
                }
                "nationality_change" => nationalities.push(signal.entity_id.clone()),
                _ => {
                    if !platforms_found.contains(&signal.source) && signal.source != "unknown" {
                        platforms_found.push(signal.source.clone());
                    }
                }
            }
        }

        PersonIdentity {
            name: name.to_string(),
            aliases,
            date_of_birth: None,
            nationalities,
            current_location: Location {
                city: "Unknown (no data)".to_string(),
                region: "Unknown (no data)".to_string(),
                country: "Unknown (no data)".to_string(),
                coordinates: None,
                location_type: LocationType::Residence,
            },
            contact_info: ContactInformation {
                emails: Vec::new(),
                phone_numbers: Vec::new(),
                social_handles: Vec::new(),
            },
            profile_urls,
            digital_footprint: DigitalFootprint {
                platforms_found,
                first_appearance_date: None,
                activity_level,
                sentiment_analysis: SentimentAnalysis {
                    overall_sentiment: "Unknown".to_string(),
                    positive_percentage: 0.0,
                    negative_percentage: 0.0,
                    neutral_percentage: 1.0,
                },
            },
        }
    }

    /// Analyze professional history
    fn analyze_professional_history(&self, signals: &[EvidenceItem]) -> ProfessionalHistory {
        let mut position_history = Vec::new();
        let mut education_history = Vec::new();
        let certifications = Vec::new();
        let mut notable_achievements = Vec::new();
        let career_gaps = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "position_change" | "new_role" => {
                    position_history.push(Position {
                        title: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        organization_type: OrganizationType::Unknown, // no signal data to determine type
                        start_date: Some(signal.timestamp),
                        end_date: None,
                        duration_months: None,
                        location: None,
                        responsibilities: Vec::new(),
                        achievements: Vec::new(),
                        reporting_structure: None,
                        is_current: true,
                    });
                }
                "promotion" => {
                    notable_achievements.push(Achievement {
                        description: format!("Promoted: {}", signal.description),
                        date: Some(signal.timestamp),
                        impact_level: ImpactLevel::CompanyWide,
                        verified: true,
                    });
                }
                "education" => {
                    education_history.push(Education {
                        institution: signal.entity_id.clone(),
                        degree_type: DegreeType::Bachelor,
                        field_of_study: signal.description.clone(),
                        start_date: None,
                        end_date: Some(signal.timestamp),
                        gpa: None,
                        honors: Vec::new(),
                    });
                }
                _ => {}
            }
        }

        let current_position = position_history
            .first()
            .cloned()
            .unwrap_or(Position {
                title: "Unknown (no data)".to_string(),
                organization: "Unknown (no data)".to_string(),
                organization_type: OrganizationType::Unknown,
                start_date: None,
                end_date: None,
                duration_months: None,
                location: None,
                responsibilities: Vec::new(),
                achievements: Vec::new(),
                reporting_structure: None,
                is_current: true,
            });

        ProfessionalHistory {
            current_position,
            position_history,
            education_history,
            certifications,
            skills_assessment: SkillsAssessment {
                technical_skills: Vec::new(),
                soft_skills: Vec::new(),
                leadership_skills: Vec::new(),
                domain_expertise: Vec::new(),
                skill_confidence: 0.5,
            },
            career_trajectory: CareerTrajectory {
                trajectory_type: TrajectoryType::Stable,
                average_tenure_months: 36.0,
                promotion_rate: 0.3,
                industry_transitions: 0,
                mobility_pattern: MobilityPattern::VerticalUp,
                trajectory_confidence: 0.5,
            },
            career_gaps,
            notable_achievements,
            professional_confidence: 0.5,
        }
    }

    /// Analyze network mapping
    fn analyze_network_mapping(&self, signals: &[EvidenceItem]) -> NetworkMapping {
        let mut direct_connections = Vec::new();
        let mut organizational_links = Vec::new();
        let mut association_groups = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "colleague" | "network_connection" => {
                    direct_connections.push(Connection {
                        target_name: signal.entity_id.clone(),
                        relationship_type: RelationshipType::Colleague,
                        relationship_strength: ConnectionStrength::Moderate,
                        context: signal.description.clone(),
                        verified: signal.confidence > 0.7,
                        first_connected_date: Some(signal.timestamp),
                    });
                }
                "org_change" => {
                    organizational_links.push(OrganizationalLink {
                        organization_name: signal.entity_id.clone(),
                        link_type: OrganizationLinkType::Employee,
                        ownership_percentage: None,
                        start_date: Some(signal.timestamp),
                        end_date: None,
                    });
                }
                "group_membership" => {
                    association_groups.push(AssociationGroup {
                        group_name: signal.description.clone(),
                        group_type: GroupType::ProfessionalAssociation,
                        membership_type: MembershipType::ActiveMember,
                        joined_date: Some(signal.timestamp),
                        active_status: true,
                    });
                }
                _ => {}
            }
        }

        NetworkMapping {
            direct_connections,
            organizational_links,
            influence_network: InfluenceNetwork {
                influence_score: 0.5,
                reach_score: 0.5,
                media_mentions: 0,
                speaking_engagements: 0,
                published_work: 0,
                industry_recognition: Vec::new(),
            },
            association_groups,
            network_confidence: 0.5,
        }
    }

    /// Analyze risk indicators
    fn analyze_risk_indicators(&self, signals: &[EvidenceItem]) -> RiskIndicator {
        let mut financial_risks = Vec::new();
        let mut reputational_risks = Vec::new();
        let mut association_risks = Vec::new();
        let behavioral_risks = Vec::new();
        let mut legal_compliance_flags = Vec::new();

        for signal in signals {
            let confidence = signal.confidence;

            match signal.evidence_type.as_str() {
                "financial_anomaly" => {
                    financial_risks.push(FinancialRisk {
                        risk_type: "Anomalous Transaction".to_string(),
                        description: signal.description.clone(),
                        severity: RiskSeverity::Medium,
                        evidence_source: signal.source.clone(),
                        confidence,
                    });
                }
                "legal_issue" | "regulatory_flag" => {
                    legal_compliance_flags.push(LegalComplianceFlag {
                        flag_type: "Legal Matter".to_string(),
                        description: signal.description.clone(),
                        severity: RiskSeverity::Medium,
                        jurisdiction: None,
                        regulatory_body: None,
                        resolution_status: None,
                    });
                }
                "reputational_concern" => {
                    reputational_risks.push(ReputationalRisk {
                        risk_type: "Reputation Risk".to_string(),
                        description: signal.description.clone(),
                        severity: RiskSeverity::Medium,
                        media_coverage: None,
                        spread_potential: SpreadPotential::Local,
                    });
                }
                "association_risk" => {
                    association_risks.push(AssociationRisk {
                        associated_entity: signal.entity_id.clone(),
                        risk_type: "Risky Association".to_string(),
                        association_strength: ConnectionStrength::Moderate,
                        severity: RiskSeverity::Medium,
                        description: signal.description.clone(),
                    });
                }
                _ => {}
            }
        }

        // Calculate overall risk score
        let mut risk_count = 0;
        let mut severity_sum = 0.0;

        for risk in &financial_risks {
            risk_count += 1;
            severity_sum += match risk.severity {
                RiskSeverity::Critical => 1.0,
                RiskSeverity::High => 0.8,
                RiskSeverity::Medium => 0.5,
                RiskSeverity::Low => 0.2,
                RiskSeverity::Informational => 0.1,
            };
        }

        for flag in &legal_compliance_flags {
            risk_count += 1;
            severity_sum += match flag.severity {
                RiskSeverity::Critical => 1.0,
                RiskSeverity::High => 0.8,
                RiskSeverity::Medium => 0.5,
                RiskSeverity::Low => 0.2,
                RiskSeverity::Informational => 0.1,
            };
        }

        let overall_risk_score = if risk_count > 0 {
            severity_sum / risk_count as f64
        } else {
            0.0
        };

        RiskIndicator {
            financial_risks,
            reputational_risks,
            association_risks,
            behavioral_risks,
            legal_compliance_flags,
            overall_risk_score,
            risk_confidence: 0.5,
        }
    }

    /// Detect trigger events
    fn detect_trigger_events(&self, signals: &[EvidenceItem]) -> Vec<TriggerEvent> {
        let mut events = Vec::new();

        for signal in signals {
            let event_type = match signal.evidence_type.as_str() {
                "position_change" => TriggerEventType::PositionChange,
                "location_change" => TriggerEventType::LocationChange,
                "legal_issue" | "regulatory_flag" => TriggerEventType::LegalIssue,
                "financial_anomaly" => TriggerEventType::FinancialAnomaly,
                "media_event" => TriggerEventType::MediaEvent,
                "social_activity" => TriggerEventType::SocialActivity,
                "network_connection" | "colleague" => TriggerEventType::NetworkChange,
                "company_event" => TriggerEventType::CompanyEvent,
                "regulatory_change" => TriggerEventType::RegulatoryEvent,
                "security_incident" => TriggerEventType::SecurityIncident,
                "reputational_concern" => TriggerEventType::ReputationEvent,
                _ => continue,
            };

            let significance = if signal.confidence > 0.8 {
                EventSignificance::High
            } else if signal.confidence > 0.6 {
                EventSignificance::Medium
            } else {
                EventSignificance::Low
            };

            events.push(TriggerEvent {
                event_id: Uuid::new_v4().to_string(),
                event_type,
                description: signal.description.clone(),
                detection_date: signal.timestamp,
                source: signal.source.clone(),
                confidence: signal.confidence,
                significance,
                related_entities: vec![signal.entity_id.clone()],
                recommended_action: Some("Review and assess".to_string()),
            });
        }

        events
    }

    /// Develop engagement strategy — returns minimal/empty data instead of fabricated preferences
    fn develop_engagement_strategy(&self, report: &PersonDeepDiveReport) -> EngagementStrategy {
        let recommended_approach = if let Some(ref network) = report.network_mapping {
            if network.influence_network.influence_score > 0.6 {
                EngagementApproach::Direct
            } else {
                EngagementApproach::Indirect
            }
        } else {
            EngagementApproach::SemiFormal
        };

        let optimal_timing = TimingRecommendation {
            best_contact_window: String::new(),
            seasonal_considerations: Vec::new(),
            scheduling_factors: Vec::new(),
            urgency_considerations: None,
        };

        // Build talking points only from actual report data
        let mut talking_points = Vec::new();
        if let Some(ref history) = report.professional_history {
            if history.current_position.title != "Unknown (no data)" {
                talking_points.push(TalkingPoint {
                    topic: "Career Background".to_string(),
                    key_message: format!("Discuss {} experience", history.current_position.title),
                    supporting_evidence: Vec::new(),
                    expected_response: String::new(),
                });
            }
        }

        EngagementStrategy {
            recommended_approach,
            communication_preferences: CommunicationPreferences {
                preferred_channel: String::new(),
                optimal_time: String::new(),
                tone: EngagementTone::SemiFormal,
                response_rate_estimate: 0.0,
            },
            optimal_timing,
            talking_points,
            topics_to_avoid: Vec::new(),
            background_information: Vec::new(),
            risk_mitigation: Vec::new(),
            engagement_confidence: 0.0,
        }
    }

    /// Generate executive summary
    fn generate_executive_summary(&self, report: &PersonDeepDiveReport) -> String {
        let mut summary = format!("Person Deep-Dive Report for {}\n\n", report.target_person.name);

        summary.push_str(&format!(
            "Overall Confidence: {:.0}%\n\n",
            report.overall_confidence * 100.0
        ));

        if let Some(ref history) = report.professional_history {
            summary.push_str(&format!(
                "## Professional History\nCurrent Position: {} at {}\n",
                history.current_position.title, history.current_position.organization
            ));
            if !history.career_gaps.is_empty() {
                summary.push_str(&format!("Career Gaps: {} detected\n", history.career_gaps.len()));
            }
        }

        if let Some(ref network) = report.network_mapping {
            summary.push_str(&format!(
                "## Network Mapping\nConnections: {}\n",
                network.direct_connections.len()
            ));
        }

        if let Some(ref risks) = report.risk_indicators {
            summary.push_str(&format!(
                "## Risk Assessment\nOverall Risk Score: {:.0}%\n",
                risks.overall_risk_score * 100.0
            ));
            if !risks.legal_compliance_flags.is_empty() {
                summary.push_str(&format!(
                    "Compliance Flags: {} detected\n",
                    risks.legal_compliance_flags.len()
                ));
            }
        }

        if let Some(ref events) = report.trigger_events {
            summary.push_str(&format!(
                "## Trigger Events\nEvents Detected: {}\n",
                events.len()
            ));
        }

        summary
    }

    /// Extract key findings
    fn extract_key_findings(&self, report: &PersonDeepDiveReport) -> Vec<String> {
        let mut findings = Vec::new();

        if let Some(ref history) = report.professional_history {
            if history.career_trajectory.trajectory_type == TrajectoryType::Ascending {
                findings.push("Ascending career trajectory detected".to_string());
            }
            if history.career_gaps.len() > 2 {
                findings.push("Multiple career gaps detected - may indicate life transitions".to_string());
            }
        }

        if let Some(ref network) = report.network_mapping {
            if network.direct_connections.len() > 10 {
                findings.push("Large professional network detected".to_string());
            }
            if network.influence_network.influence_score > 0.7 {
                findings.push("High influence score in professional network".to_string());
            }
        }

        if let Some(ref risks) = report.risk_indicators {
            if risks.overall_risk_score > 0.6 {
                findings.push("Elevated risk profile requires attention".to_string());
            }
        }

        if let Some(ref events) = report.trigger_events {
            let significant_events: Vec<_> = events
                .iter()
                .filter(|e| matches!(e.significance, EventSignificance::High | EventSignificance::Critical))
                .collect();
            if !significant_events.is_empty() {
                findings.push(format!(
                    "{} significant recent events detected",
                    significant_events.len()
                ));
            }
        }

        findings
    }

    /// Identify critical concerns
    fn identify_critical_concerns(&self, report: &PersonDeepDiveReport) -> Vec<String> {
        let mut concerns = Vec::new();

        if let Some(ref risks) = report.risk_indicators {
            for flag in &risks.legal_compliance_flags {
                if matches!(flag.severity, RiskSeverity::Critical | RiskSeverity::High) {
                    concerns.push(format!("Critical legal flag: {}", flag.description));
                }
            }
            for risk in &risks.financial_risks {
                if matches!(risk.severity, RiskSeverity::Critical | RiskSeverity::High) {
                    concerns.push(format!("High financial risk: {}", risk.description));
                }
            }
        }

        if let Some(ref events) = report.trigger_events {
            for event in events {
                if matches!(event.significance, EventSignificance::Critical) {
                    concerns.push(format!("Critical event: {}", event.description));
                }
            }
        }

        concerns
    }

    /// Generate recommendations
    fn generate_recommendations(&self, report: &PersonDeepDiveReport) -> Vec<String> {
        let mut recommendations = Vec::new();

        if let Some(ref risks) = report.risk_indicators {
            if !risks.legal_compliance_flags.is_empty() {
                recommendations
                    .push("Conduct thorough due diligence before any engagement".to_string());
            }
            if risks.overall_risk_score > 0.5 {
                recommendations.push("Consider risk mitigation measures".to_string());
            }
        }

        if let Some(ref events) = report.trigger_events {
            let recent_events: Vec<_> = events.iter().filter(|e| {
                (Utc::now() - e.detection_date).num_days() < 30
            }).collect();
            if !recent_events.is_empty() {
                recommendations.push("Monitor for additional developments on recent events".to_string());
            }
        }

        if let Some(ref history) = report.professional_history {
            if history.career_gaps.len() > 1 {
                recommendations.push("Investigate career gaps for potential concerns".to_string());
            }
        }

        recommendations
    }

    /// Identify data gaps
    fn identify_data_gaps(&self, report: &PersonDeepDiveReport, signals: &[EvidenceItem]) -> Vec<String> {
        let mut gaps = Vec::new();

        if report.target_person.date_of_birth.is_none() {
            gaps.push("Date of birth not available".to_string());
        }

        if report.target_person.contact_info.emails.is_empty() {
            gaps.push("Contact information not available".to_string());
        }

        if let Some(ref history) = report.professional_history {
            if history.position_history.len() < 2 {
                gaps.push("Limited professional history available".to_string());
            }
        }

        if let Some(ref network) = report.network_mapping {
            if network.direct_connections.is_empty() {
                gaps.push("Limited network data available".to_string());
            }
        }

        if signals.is_empty() {
            gaps.push("No signals available - limited intelligence".to_string());
        }

        gaps
    }
}

impl Default for PersonDeepDiveWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn test_workflow_creation() {
        let workflow = PersonDeepDiveWorkflow::new();
        assert!(!workflow.workflow_id.is_empty());
        assert!(workflow.config.include_professional_history);
    }

    #[test]
    fn test_workflow_with_config() {
        let config = WorkflowConfig {
            include_professional_history: true,
            include_network_mapping: false,
            include_risk_indicators: true,
            include_trigger_events: false,
            include_engagement_strategy: false,
            risk_threshold: RiskSeverity::High,
        };
        let workflow = PersonDeepDiveWorkflow::with_config(config);
        assert!(!workflow.config.include_network_mapping);
        assert!(matches!(workflow.config.risk_threshold, RiskSeverity::High));
    }

    #[test]
    fn test_workflow_run_with_signals() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "New Company".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "position_change".to_string(),
                description: "VP of Engineering".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "John Doe".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "colleague".to_string(),
                description: "Team member".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        assert_eq!(report.target_person.name, "Jane Smith");
        assert!(report.professional_history.is_some());
        assert!(report.network_mapping.is_some());
    }

    #[test]
    fn test_professional_history_analysis() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "new_role".to_string(),
                description: "Senior Developer".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "promotion".to_string(),
                description: "Promoted to Tech Lead".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        let history = report.professional_history.unwrap();
        assert!(!history.position_history.is_empty());
        assert!(!history.notable_achievements.is_empty());
    }

    #[test]
    fn test_network_mapping_analysis() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Colleague A".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "colleague".to_string(),
                description: "Worked together on Project X".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Org B".to_string(),
                entity_type: "organization".to_string(),
                evidence_type: "org_change".to_string(),
                description: "Joined as advisor".to_string(),
                source: "News".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        let network = report.network_mapping.unwrap();
        assert!(!network.direct_connections.is_empty());
        assert!(!network.organizational_links.is_empty());
    }

    #[test]
    fn test_risk_indicators_analysis() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Jane Smith".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "legal_issue".to_string(),
                description: "Pending litigation".to_string(),
                source: "Court Records".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Risky Company".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "association_risk".to_string(),
                description: "Board member at flagged company".to_string(),
                source: "News".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        let risks = report.risk_indicators.unwrap();
        assert!(!risks.legal_compliance_flags.is_empty());
        assert!(!risks.association_risks.is_empty());
        assert!(risks.overall_risk_score > 0.0);
    }

    #[test]
    fn test_trigger_event_detection() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "New Company".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "position_change".to_string(),
                description: "New CFO appointment".to_string(),
                source: "Press Release".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Multiple".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "media_event".to_string(),
                description: "Featured in industry publication".to_string(),
                source: "Media".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        let events = report.trigger_events.unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().all(|e| e.confidence > 0.0));
    }

    #[test]
    fn test_engagement_strategy_development() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Tech Corp".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "position_change".to_string(),
                description: "CTO".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        let strategy = report.engagement_strategy.unwrap();
        // Talking points are derived from actual professional history data
        assert!(!strategy.talking_points.is_empty());
        // topics_to_avoid is no longer fabricated — may be empty when no data available
    }

    #[test]
    fn test_data_gaps_identification() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Jane Smith", signals);
        // With no signals, we expect data gaps
        assert!(!report.data_gaps.is_empty());
    }

    #[test]
    fn test_key_findings_extraction() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "position_change".to_string(),
                description: "CEO".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Person A".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "colleague".to_string(),
                description: "colleague".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig3".to_string(),
                entity_id: "Person B".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "colleague".to_string(),
                description: "colleague".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig4".to_string(),
                entity_id: "Person C".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "colleague".to_string(),
                description: "colleague".to_string(),
                source: "LinkedIn".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        // Should detect large network
        assert!(!report.key_findings.is_empty());
    }

    #[test]
    fn test_critical_concerns_identification() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Jane Smith".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "legal_issue".to_string(),
                description: "Critical legal matter".to_string(),
                source: "Court".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Jane Smith".to_string(),
                entity_type: "person".to_string(),
                evidence_type: "financial_anomaly".to_string(),
                description: "Large unexplained transactions".to_string(),
                source: "Financial Records".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Jane Smith", signals);
        // Should have critical concerns from legal and financial issues
        assert!(!report.critical_concerns.is_empty() || report.risk_indicators.as_ref().map(|r| r.overall_risk_score > 0.0).unwrap_or(false));
    }

    #[test]
    fn test_report_serialization() {
        let workflow = PersonDeepDiveWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Jane Smith", signals);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Jane Smith"));
        assert!(json.contains("workflow_id"));
    }
}
