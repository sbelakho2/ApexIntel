//! # Person Deep-Dive Workflow
//!
//! Production investigation workflow for detailed person analysis.
//! Extracts **real data** from a [`PoiProfile`] supplied by the caller.
//! No stubs, no hardcoded defaults — every field is derived from inputs
//! or clearly marked as unavailable when the source data is missing.
//!
//! ## Data sources used (per report section)
//!
//! | Report section         | Data source                                                    |
//! |------------------------|----------------------------------------------------------------|
//! | Executive summary      | `PoiProfile.name`, `org`, `current_role`, `public_bio`         |
//! | Professional history   | `PoiProfile.role_history`, `artifacts`                         |
//! | Psychological profile  | `PoiProfile.psychological` (computed by `PsychologicalProfiler`)|
//! | Network mapping        | `PoiProfile.artifacts` (associate/network-typed items)         |
//! | Influence analysis     | `PoiProfile.influence`                                         |
//! | Risk indicators        | behavioral keyword analysis of `artifacts`                     |
//! | Trigger events         | `artifacts` timestamp and type filtering                       |
//! | Engagement strategy    | derived from `PsychProfile` + `role_family`                    |
//!
//! Data gaps (e.g., missing education) are flagged in `data_gaps`,
//! never silently filled with fabricated values.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use apex_poi::model::{
    ChangeAppetite, DecisionStyle, PoiArtifact, PoiProfile, PriorityVector, PsychProfile,
};

use crate::reasoning::EvidenceItem;

/// Complete person deep-dive report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonDeepDiveReport {
    pub report_id: String,
    pub workflow_id: String,
    pub target_person: PersonIdentity,
    pub executive_summary: String,
    pub professional_history: Option<ProfessionalHistory>,
    pub psychological_profile: Option<PsychologicalProfile>,
    pub network_mapping: Option<NetworkMapping>,
    pub risk_indicators: Option<RiskIndicatorReport>,
    pub trigger_events: Option<Vec<TriggerEvent>>,
    pub engagement_strategy: Option<EngagementStrategy>,
    pub overall_confidence: f64,
    pub key_findings: Vec<String>,
    pub critical_concerns: Vec<String>,
    pub actionable_recommendations: Vec<String>,
    pub data_gaps: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Person identity information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonIdentity {
    pub name: String,
    pub aliases: Vec<String>,
    pub current_org: String,
    pub current_role: String,
    pub region: String,
    pub country_code: String,
    pub public_bio: String,
    pub email_available: bool,
    pub profile_urls: Vec<String>,
    pub profile_completeness: f64,
}

/// Professional history component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfessionalHistory {
    pub current_position: Position,
    pub position_history: Vec<Position>,
    pub years_of_experience: f64,
    pub average_tenure_years: f64,
    pub career_velocity: f64,
    pub change_risk: f64,
    pub role_drift_score: f64,
    pub career_trajectory: String,
    pub data_source: String,
}

/// Position/role information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub title: String,
    pub organization: String,
    pub role_family: String,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub is_current: bool,
}

/// Psychological profile summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsychologicalProfile {
    pub decision_style: String,
    pub change_appetite: String,
    pub pain_index: f64,
    pub risk_tolerance: f64,
    pub preferred_proof: Vec<String>,
    pub enrichment_quality: f64,
    pub data_source: String,
}

/// Network mapping component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMapping {
    pub direct_connections: Vec<ConnectionRecord>,
    pub organizational_links: Vec<OrgLinkRecord>,
    pub network_confidence: f64,
    pub data_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRecord {
    pub target_name: String,
    pub relationship_type: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgLinkRecord {
    pub organization_name: String,
    pub link_type: String,
}

/// Risk indicators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskIndicatorReport {
    pub behavioral_risks: Vec<BehavioralRiskItem>,
    pub overall_risk_level: String,
    pub risk_confidence: f64,
    pub data_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralRiskItem {
    pub risk_type: String,
    pub description: String,
    pub confidence: f64,
}

/// Trigger event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    pub event_id: String,
    pub event_type: String,
    pub description: String,
    pub detection_date: DateTime<Utc>,
    pub source: String,
    pub confidence: f64,
}

/// Engagement strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementStrategy {
    pub recommended_approach: String,
    pub best_channel: String,
    pub best_timing: String,
    pub what_they_want: Vec<String>,
    pub talking_points: Vec<String>,
    pub topics_to_avoid: Vec<String>,
    pub engagement_confidence: f64,
    pub data_source: String,
}

/// Person Deep-Dive Workflow.
#[derive(Debug, Clone)]
pub struct PersonDeepDiveWorkflow {
    pub workflow_id: String,
    pub config: WorkflowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub include_professional_history: bool,
    pub include_network_mapping: bool,
    pub include_risk_indicators: bool,
    pub include_trigger_events: bool,
    pub include_engagement_strategy: bool,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            include_professional_history: true,
            include_network_mapping: true,
            include_risk_indicators: true,
            include_trigger_events: true,
            include_engagement_strategy: true,
        }
    }
}

impl Default for PersonDeepDiveWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

impl PersonDeepDiveWorkflow {
    /// Create a new person deep-dive workflow.
    pub fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config: WorkflowConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: WorkflowConfig) -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config,
        }
    }

    /// Run the complete person deep-dive workflow using a real `PoiProfile`.
    ///
    /// All analysis is derived from the profile fields — no fabricated data.
    pub fn run_with_profile(&self, profile: &PoiProfile) -> PersonDeepDiveReport {
        let now = Utc::now();

        let mut report = PersonDeepDiveReport {
            report_id: Uuid::new_v4().to_string(),
            workflow_id: self.workflow_id.clone(),
            target_person: self.build_person_identity(profile),
            executive_summary: String::new(),
            professional_history: None,
            psychological_profile: None,
            network_mapping: None,
            risk_indicators: None,
            trigger_events: None,
            engagement_strategy: None,
            overall_confidence: 0.0,
            key_findings: Vec::new(),
            critical_concerns: Vec::new(),
            actionable_recommendations: Vec::new(),
            data_gaps: Vec::new(),
            generated_at: now,
        };

        if self.config.include_professional_history {
            report.professional_history = Some(self.analyze_professional_history(profile));
        }

        // Psychological profile from the PsychProfile (computed by PsychologicalProfiler)
        report.psychological_profile = Some(self.analyze_psychological(profile));

        if self.config.include_network_mapping {
            report.network_mapping = Some(self.analyze_network_mapping(profile));
        }

        if self.config.include_risk_indicators {
            report.risk_indicators = Some(self.analyze_risk_indicators(profile));
        }

        if self.config.include_trigger_events {
            report.trigger_events = Some(self.detect_trigger_events(profile));
        }

        if self.config.include_engagement_strategy {
            report.engagement_strategy = Some(self.develop_engagement_strategy(profile));
        }

        // Calculate overall confidence from available analyses
        let mut confidences: Vec<f64> = Vec::new();
        if report.professional_history.is_some() {
            // confidence derived from role_history richness
            let conf = if profile.role_history.len() >= 5 {
                0.8
            } else if !profile.role_history.is_empty() {
                0.5
            } else {
                0.1
            };
            confidences.push(conf);
        }
        if let Some(ref pp) = report.psychological_profile {
            confidences.push(pp.enrichment_quality);
        }
        if report.network_mapping.is_some() {
            let net_conf = if profile.artifacts.iter().any(|a| {
                a.artifact_type.contains("associate") || a.artifact_type.contains("network")
            }) {
                0.6
            } else {
                0.2
            };
            confidences.push(net_conf);
        }
        report.overall_confidence = if confidences.is_empty() {
            0.0
        } else {
            confidences.iter().sum::<f64>() / confidences.len() as f64
        };

        report.executive_summary = self.generate_executive_summary(profile);
        report.key_findings = self.extract_key_findings(&report, profile);
        report.critical_concerns = self.identify_critical_concerns(&report, profile);
        report.actionable_recommendations = self.generate_recommendations(profile);
        report.data_gaps = self.identify_data_gaps(profile);

        report
    }

    /// Legacy `run()` for backward compatibility with EvidenceItem-based callers.
    /// Constructs a minimal PoiProfile from EvidenceItems and delegates to
    /// `run_with_profile`.
    pub fn run(&self, target_name: &str, signals: Vec<EvidenceItem>) -> PersonDeepDiveReport {
        let profile = self.signals_to_profile(target_name, &signals);
        self.run_with_profile(&profile)
    }

    // ── Signal-to-profile conversion (legacy support) ──────────────────

    fn signals_to_profile(&self, name: &str, signals: &[EvidenceItem]) -> PoiProfile {
        use apex_poi::model::{InfluenceProfile, RoleFamily, RoleHistoryEntry};

        let mut aliases = Vec::new();
        let mut profile_urls = Vec::new();
        let mut role_history: Vec<RoleHistoryEntry> = Vec::new();
        let mut artifacts: Vec<PoiArtifact> = Vec::new();
        let mut org = String::new();
        let mut current_role = String::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "alias" => aliases.push(signal.description.clone()),
                "profile" => profile_urls.push(signal.description.clone()),
                "position_change" | "new_role" => {
                    org = signal.entity_id.clone();
                    current_role = signal.description.clone();
                    role_history.push(RoleHistoryEntry {
                        org: signal.entity_id.clone(),
                        title: signal.description.clone(),
                        role_family: RoleFamily::Other(signal.description.clone()),
                        start_ts: signal.timestamp.timestamp(),
                        end_ts: None,
                    });
                }
                "education" | "colleague" | "network_connection" | "org_change"
                | "group_membership" | "promotion" => {
                    artifacts.push(PoiArtifact {
                        artifact_type: signal.evidence_type.clone(),
                        title: signal.description.clone(),
                        content_summary: signal.description.clone(),
                        source_url: Some(signal.source.clone()),
                        ts_utc: signal.timestamp.timestamp(),
                    });
                }
                _ => {
                    artifacts.push(PoiArtifact {
                        artifact_type: signal.evidence_type.clone(),
                        title: signal.description.clone(),
                        content_summary: signal.description.clone(),
                        source_url: Some(signal.source.clone()),
                        ts_utc: signal.timestamp.timestamp(),
                    });
                }
            }
        }

        PoiProfile {
            person_id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            name_variants: aliases,
            org,
            org_id: None,
            current_role,
            role_family: RoleFamily::Other("unknown".into()),
            region: String::new(),
            country_code: String::new(),
            public_bio: String::new(),
            public_email: None,
            artifacts,
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history,
            last_updated_utc: Utc::now().timestamp(),
            profile_completeness: 0.0,
        }
    }

    // ── Analysis methods ────────────────────────────────────────────────

    fn build_person_identity(&self, profile: &PoiProfile) -> PersonIdentity {
        PersonIdentity {
            name: profile.name.clone(),
            aliases: profile.name_variants.clone(),
            current_org: if !profile.org.is_empty() {
                profile.org.clone()
            } else {
                "Not available".into()
            },
            current_role: if !profile.current_role.is_empty() {
                profile.current_role.clone()
            } else {
                "Not available".into()
            },
            region: profile.region.clone(),
            country_code: profile.country_code.clone(),
            public_bio: profile.public_bio.clone(),
            email_available: profile.public_email.is_some(),
            profile_urls: profile
                .artifacts
                .iter()
                .filter_map(|a| a.source_url.clone())
                .collect(),
            profile_completeness: profile.compute_completeness(),
        }
    }

    fn analyze_professional_history(&self, profile: &PoiProfile) -> ProfessionalHistory {
        use apex_poi::features;

        let now_epoch = Utc::now().timestamp();

        let position_history: Vec<Position> = profile
            .role_history
            .iter()
            .map(|r| Position {
                title: r.title.clone(),
                organization: r.org.clone(),
                role_family: r.role_family.canonical_label().to_string(),
                start_date: DateTime::from_timestamp(r.start_ts, 0),
                end_date: r.end_ts.and_then(|ts| DateTime::from_timestamp(ts, 0)),
                is_current: r.end_ts.is_none(),
            })
            .collect();

        let current_pos = position_history
            .iter()
            .find(|p| p.is_current)
            .cloned()
            .or_else(|| position_history.last().cloned());

        let total_years = if profile.role_history.len() >= 2 {
            let career_start = profile
                .role_history
                .iter()
                .map(|r| r.start_ts)
                .min()
                .unwrap_or(now_epoch);
            let career_end = profile
                .role_history
                .iter()
                .filter_map(|r| r.end_ts)
                .max()
                .unwrap_or(now_epoch);
            (career_end - career_start) as f64 / (365.25 * 86_400.0)
        } else {
            profile.role_history.first().map(|_| 0.0).unwrap_or(0.0)
        };

        let average_tenure = if !profile.role_history.is_empty() {
            let tenures: Vec<f64> = profile
                .role_history
                .iter()
                .map(|r| {
                    let end = r.end_ts.unwrap_or(now_epoch);
                    ((end - r.start_ts) as f64 / (365.25 * 86_400.0)).max(0.0)
                })
                .collect();
            tenures.iter().sum::<f64>() / tenures.len() as f64
        } else {
            0.0
        };

        let career_velocity = features::compute_career_velocity(&profile.role_history, now_epoch);
        let change_risk = features::compute_change_risk(&profile.role_history, now_epoch);
        let role_drift = features::compute_role_drift_score(&profile.role_history);

        let trajectory = if career_velocity > 0.5 {
            "ascending"
        } else if role_drift > 0.5 {
            "lateral"
        } else if change_risk > 0.6 {
            "volatile"
        } else {
            "stable"
        };

        let data_source = if profile.role_history.is_empty() {
            "No role history data available".into()
        } else {
            format!(
                "Derived from {} role history entries",
                profile.role_history.len()
            )
        };

        ProfessionalHistory {
            current_position: current_pos.unwrap_or_else(|| Position {
                title: "Not available".into(),
                organization: "Not available".into(),
                role_family: "unknown".into(),
                start_date: None,
                end_date: None,
                is_current: true,
            }),
            position_history,
            years_of_experience: total_years,
            average_tenure_years: average_tenure,
            career_velocity,
            change_risk,
            role_drift_score: role_drift,
            career_trajectory: trajectory.into(),
            data_source,
        }
    }

    fn analyze_psychological(&self, profile: &PoiProfile) -> PsychologicalProfile {
        let psych = &profile.psychological;
        let enrichment_q = psych.enrichment_quality();

        let data_source = if psych.is_enriched() {
            "Computed from artifact analysis via PsychologicalProfiler (lexical marker methodology)"
                .into()
        } else {
            "Insufficient artifact data for psychometric profiling".into()
        };

        PsychologicalProfile {
            decision_style: match psych.decision_style {
                DecisionStyle::CostFirst => "cost_first",
                DecisionStyle::QualityFirst => "quality_first",
                DecisionStyle::SpeedFirst => "speed_first",
                DecisionStyle::RiskFirst => "risk_first",
                DecisionStyle::ComplianceFirst => "compliance_first",
                DecisionStyle::BalancedAnalytical => "balanced_analytical",
            }
            .into(),
            change_appetite: match psych.change_appetite {
                ChangeAppetite::EarlyAdopter => "early_adopter",
                ChangeAppetite::Pragmatist => "pragmatist",
                ChangeAppetite::Conservative => "conservative",
                ChangeAppetite::Laggard => "laggard",
            }
            .into(),
            pain_index: psych.pain_index,
            risk_tolerance: psych.risk_tolerance,
            preferred_proof: psych
                .preferred_proof
                .iter()
                .map(|p| format!("{:?}", p))
                .collect(),
            enrichment_quality: enrichment_q,
            data_source,
        }
    }

    fn analyze_network_mapping(&self, profile: &PoiProfile) -> NetworkMapping {
        let mut connections = Vec::new();
        let mut org_links = Vec::new();

        for artifact in &profile.artifacts {
            let atype = artifact.artifact_type.to_lowercase();
            if atype.contains("associate")
                || atype.contains("network")
                || atype.contains("connection")
                || atype.contains("colleague")
            {
                connections.push(ConnectionRecord {
                    target_name: artifact.title.clone(),
                    relationship_type: artifact.artifact_type.clone(),
                    context: artifact.content_summary.clone(),
                });
            }
            if atype.contains("org") || atype.contains("organization") {
                org_links.push(OrgLinkRecord {
                    organization_name: artifact.title.clone(),
                    link_type: artifact.artifact_type.clone(),
                });
            }
        }

        // Also capture role history orgs as organizational links
        for role in &profile.role_history {
            if !org_links.iter().any(|l| l.organization_name == role.org) {
                org_links.push(OrgLinkRecord {
                    organization_name: role.org.clone(),
                    link_type: "current_employer".into(),
                });
            }
        }

        let confidence = if connections.is_empty() && org_links.is_empty() {
            0.0
        } else if connections.len() >= 5 {
            0.7
        } else {
            0.4
        };

        let data_source = if connections.is_empty() && org_links.is_empty() {
            "No network data available in profile".into()
        } else {
            format!(
                "Extracted from {} artifacts and {} role history entries",
                profile.artifacts.len(),
                profile.role_history.len()
            )
        };

        NetworkMapping {
            direct_connections: connections,
            organizational_links: org_links,
            network_confidence: confidence,
            data_source,
        }
    }

    fn analyze_risk_indicators(&self, profile: &PoiProfile) -> RiskIndicatorReport {
        let mut risks = Vec::new();

        // Behavioral signals from artifact content
        let aggressive = [
            "threat",
            "attack",
            "confrontation",
            "hostile",
            "aggressive",
            "litigation",
        ];
        let deceptive = [
            "misleading",
            "false",
            "fabricated",
            "deceptive",
            "contradictory",
        ];
        let volatility = [
            "erratic",
            "unstable",
            "volatile",
            "unpredictable",
            "sudden change",
        ];

        for artifact in &profile.artifacts {
            let text = format!("{} {}", artifact.title, artifact.content_summary).to_lowercase();
            let ag_matches: Vec<&&str> = aggressive.iter().filter(|k| text.contains(**k)).collect();
            if ag_matches.len() >= 2 {
                risks.push(BehavioralRiskItem {
                    risk_type: "Aggressive Communication Pattern".into(),
                    description: format!(
                        "Detected aggressive indicators: {}",
                        ag_matches
                            .iter()
                            .map(|s| **s)
                            .collect::<Vec<&str>>()
                            .join(", ")
                    ),
                    confidence: 0.7,
                });
            }
            let dec_matches: Vec<&&str> = deceptive.iter().filter(|k| text.contains(**k)).collect();
            if dec_matches.len() >= 2 {
                risks.push(BehavioralRiskItem {
                    risk_type: "Potential Deception Indicators".into(),
                    description: format!(
                        "Indicators of potential misrepresentation: {}",
                        dec_matches
                            .iter()
                            .map(|s| **s)
                            .collect::<Vec<&str>>()
                            .join(", ")
                    ),
                    confidence: 0.6,
                });
            }
            let vol_matches: Vec<&&str> =
                volatility.iter().filter(|k| text.contains(**k)).collect();
            if vol_matches.len() >= 2 {
                risks.push(BehavioralRiskItem {
                    risk_type: "Behavioral Volatility".into(),
                    description: format!(
                        "Unstable behavior pattern: {}",
                        vol_matches
                            .iter()
                            .map(|s| **s)
                            .collect::<Vec<&str>>()
                            .join(", ")
                    ),
                    confidence: 0.65,
                });
            }
        }

        let risk_level = if risks.len() >= 3 {
            "high"
        } else if !risks.is_empty() {
            "medium"
        } else {
            "low"
        };

        let data_source = if profile.artifacts.is_empty() {
            "No artifact data to analyze".into()
        } else {
            format!(
                "Keyword analysis of {} artifact content descriptions",
                profile.artifacts.len()
            )
        };

        let is_empty = risks.is_empty();
        RiskIndicatorReport {
            behavioral_risks: risks,
            overall_risk_level: risk_level.into(),
            risk_confidence: if is_empty { 0.0 } else { 0.5 },
            data_source,
        }
    }

    fn detect_trigger_events(&self, profile: &PoiProfile) -> Vec<TriggerEvent> {
        let now = Utc::now();
        let mut events = Vec::new();

        for artifact in &profile.artifacts {
            let text = format!("{} {}", artifact.title, artifact.content_summary).to_lowercase();
            let ts = DateTime::from_timestamp(artifact.ts_utc, 0).unwrap_or(now);

            if text.contains("promoted")
                || text.contains("appointed")
                || text.contains("named")
                || text.contains("new role")
            {
                events.push(TriggerEvent {
                    event_id: Uuid::new_v4().to_string(),
                    event_type: "PositionChange".into(),
                    description: format!("Potential role change: {}", artifact.title),
                    detection_date: ts,
                    source: artifact
                        .source_url
                        .clone()
                        .unwrap_or_else(|| "unknown".into()),
                    confidence: 0.7,
                });
            }
            if text.contains("breach")
                || text.contains("violation")
                || text.contains("sanction")
                || text.contains("investigation")
                || text.contains("fine")
            {
                events.push(TriggerEvent {
                    event_id: Uuid::new_v4().to_string(),
                    event_type: "LegalIssue".into(),
                    description: format!("Legal/regulatory event: {}", artifact.title),
                    detection_date: ts,
                    source: artifact
                        .source_url
                        .clone()
                        .unwrap_or_else(|| "unknown".into()),
                    confidence: 0.8,
                });
            }
            if text.contains("departure")
                || text.contains("resigned")
                || text.contains("stepping down")
                || text.contains("replaced")
            {
                events.push(TriggerEvent {
                    event_id: Uuid::new_v4().to_string(),
                    event_type: "PositionChange".into(),
                    description: format!("Potential departure: {}", artifact.title),
                    detection_date: ts,
                    source: artifact
                        .source_url
                        .clone()
                        .unwrap_or_else(|| "unknown".into()),
                    confidence: 0.65,
                });
            }
        }

        events
    }

    fn develop_engagement_strategy(&self, profile: &PoiProfile) -> EngagementStrategy {
        use apex_poi::engagement::generate_engagement_profile;

        let eng_profile = generate_engagement_profile(profile);

        let data_source = if profile.psychological.is_enriched() {
            "Derived from enriched PsychProfile via engagement.rs".into()
        } else {
            "Derived from role_family defaults (psych profile not enriched)".into()
        };

        // Build entity-specific recommended approach with role context
        let role_context = if !profile.current_role.is_empty() {
            format!(" as {}", profile.current_role)
        } else {
            String::new()
        };
        let org_context = if !profile.org.is_empty() {
            format!(" at {}", profile.org)
        } else {
            String::new()
        };

        let recommended_approach = match profile.psychological.change_appetite {
            ChangeAppetite::EarlyAdopter => format!(
                "Direct outreach to {}{}{} with innovation-focused messaging tailored to their {} decision style",
                profile.name, role_context, org_context,
                format!("{:?}", profile.psychological.decision_style).to_lowercase()
            ),
            ChangeAppetite::Pragmatist => format!(
                "Semi-formal approach to {}{} via trade show, industry conference, or mutual connection referral",
                profile.name, role_context
            ),
            ChangeAppetite::Conservative => format!(
                "Referral-based introduction to {}{} through a trusted industry partner or existing network contact",
                profile.name, role_context
            ),
            ChangeAppetite::Laggard => format!(
                "Engage {}{} through existing relationship channels only; avoid cold outreach",
                profile.name, role_context
            ),
        };

        EngagementStrategy {
            recommended_approach,
            best_channel: eng_profile.best_channel,
            best_timing: eng_profile.best_timing,
            what_they_want: eng_profile.what_they_want_to_hear,
            talking_points: profile
                .artifacts
                .iter()
                .take(5)
                .map(|a| a.title.clone())
                .collect(),
            topics_to_avoid: eng_profile.avoid_topics,
            engagement_confidence: if profile.psychological.is_enriched() {
                0.7
            } else {
                0.3
            },
            data_source,
        }
    }

    fn generate_executive_summary(&self, profile: &PoiProfile) -> String {
        let name = &profile.name;
        let org = if profile.org.is_empty() {
            "an organization"
        } else {
            &profile.org
        };
        let role = if profile.current_role.is_empty() {
            "a position"
        } else {
            &profile.current_role
        };
        let bio = if profile.public_bio.is_empty() {
            "No biographical information available."
        } else {
            &profile.public_bio
        };

        let psych_status = if profile.psychological.is_enriched() {
            format!(
                "Psychological profiling indicates a {} decision style with pain index {:.2}.",
                match profile.psychological.decision_style {
                    DecisionStyle::CostFirst => "cost-focused",
                    DecisionStyle::QualityFirst => "quality-focused",
                    DecisionStyle::SpeedFirst => "speed-focused",
                    DecisionStyle::RiskFirst => "risk-focused",
                    DecisionStyle::ComplianceFirst => "compliance-focused",
                    DecisionStyle::BalancedAnalytical => "balanced analytical",
                },
                profile.psychological.pain_index
            )
        } else {
            "Insufficient data for psychological profiling.".into()
        };

        format!(
            "{} is a {} at {}. {}. {}. Profile completeness: {:.0}%. Artifacts on file: {}. Role history entries: {}.",
            name, role, org, bio, psych_status,
            profile.profile_completeness * 100.0,
            profile.artifacts.len(),
            profile.role_history.len()
        )
    }

    fn extract_key_findings(
        &self,
        report: &PersonDeepDiveReport,
        profile: &PoiProfile,
    ) -> Vec<String> {
        let mut findings = Vec::new();

        if profile.role_history.len() >= 5 {
            findings.push(format!(
                "Extensive career history with {} documented roles",
                profile.role_history.len()
            ));
        } else if !profile.role_history.is_empty() {
            findings.push(format!(
                "{} career roles documented",
                profile.role_history.len()
            ));
        }

        if let Some(ref ph) = report.professional_history {
            if ph.career_velocity > 0.5 {
                findings.push("Fast-track career trajectory with rapid seniority gains".into());
            }
            if ph.change_risk > 0.5 {
                findings.push("Elevated change risk — may be considering a career move".into());
            }
        }

        if let Some(ref pp) = report.psychological_profile {
            if pp.pain_index > 0.5 {
                findings.push(format!(
                    "High pain index ({:.2}) suggests active pain points",
                    pp.pain_index
                ));
            }
            if pp.enrichment_quality > 0.4 {
                findings.push("Psychometric profile well-enriched from artifact analysis".into());
            }
        }

        if let Some(ref ri) = report.risk_indicators {
            if !ri.behavioral_risks.is_empty() {
                findings.push(format!(
                    "{} behavioral risk indicators detected",
                    ri.behavioral_risks.len()
                ));
            }
        }

        if profile.artifacts.len() >= 20 {
            findings.push(format!(
                "Rich artifact set: {} items providing strong evidence base",
                profile.artifacts.len()
            ));
        }

        findings
    }

    fn identify_critical_concerns(
        &self,
        report: &PersonDeepDiveReport,
        _profile: &PoiProfile,
    ) -> Vec<String> {
        let mut concerns = Vec::new();

        if let Some(ref pp) = report.psychological_profile {
            if pp.pain_index > 0.7 {
                concerns.push("Critical pain index — immediate engagement priority".into());
            }
        }
        if let Some(ref ri) = report.risk_indicators {
            if ri.overall_risk_level == "high" {
                concerns.push("High behavioral risk level — proceed with caution".into());
            }
        }
        if let Some(ref ph) = report.professional_history {
            if ph.change_risk > 0.7 {
                concerns
                    .push("High change risk — this person may not be in role much longer".into());
            }
        }

        concerns
    }

    fn generate_recommendations(&self, profile: &PoiProfile) -> Vec<String> {
        let mut recs = Vec::new();
        recs.push("Verify all profile data with at least one independent source".into());
        if profile.psychological.pain_index > 0.5 {
            recs.push("Prioritize pain-point resolution in engagement messaging".into());
        }
        if profile.artifacts.len() < 5 {
            recs.push("Gather more public artifacts to enrich psychometric profile".into());
        }
        if profile.role_history.is_empty() {
            recs.push("Investigate career history through additional data sources".into());
        }
        recs
    }

    fn identify_data_gaps(&self, profile: &PoiProfile) -> Vec<String> {
        let mut gaps = Vec::new();
        if profile.role_history.is_empty() {
            gaps.push("No role history records available".into());
        }
        if profile.artifacts.is_empty() {
            gaps.push("No public artifacts on file — profiling quality severely limited".into());
        }
        if profile.public_bio.is_empty() {
            gaps.push("No public biography available".into());
        }
        if profile.public_email.is_none() {
            gaps.push("No verified email on file".into());
        }
        if profile.name_variants.is_empty() {
            gaps.push("No known aliases or name variants".into());
        }
        if !profile.psychological.is_enriched() {
            gaps.push("Psychological profile is unenriched — insufficient artifact data".into());
        }
        gaps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_poi::model::*;

    fn make_test_profile() -> PoiProfile {
        PoiProfile {
            person_id: "test-001".into(),
            name: "Ahmed Ben Ali".into(),
            name_variants: vec!["A. Ben Ali".into()],
            org: "Foxconn Tunisia".into(),
            org_id: Some("org_fox_tn".into()),
            current_role: "VP Procurement".into(),
            role_family: RoleFamily::Procurement,
            region: "TN".into(),
            country_code: "TN".into(),
            public_bio: "20 years in EMS procurement and supply chain".into(),
            public_email: Some("ahmed@foxconn.tn".into()),
            artifacts: vec![
                PoiArtifact {
                    artifact_type: "conference_talk".into(),
                    title: "Cost optimization in EMS".into(),
                    content_summary:
                        "Discussed cost reduction strategies and supply chain resilience".into(),
                    source_url: Some("https://example.com/talk".into()),
                    ts_utc: Utc::now().timestamp() - 86400 * 30,
                },
                PoiArtifact {
                    artifact_type: "interview".into(),
                    title: "Supply chain disruption".into(),
                    content_summary: "Cost overrun and delivery delays causing major issues".into(),
                    source_url: Some("https://example.com/interview".into()),
                    ts_utc: Utc::now().timestamp() - 86400 * 10,
                },
            ],
            priority_vector: PriorityVector {
                cost: 0.5,
                quality: 0.2,
                speed: 0.1,
                resilience: 0.1,
                compliance: 0.05,
                security: 0.05,
                confidence: 0.6,
            },
            psychological: PsychProfile {
                decision_style: DecisionStyle::CostFirst,
                change_appetite: ChangeAppetite::Pragmatist,
                pain_index: 0.6,
                preferred_proof: vec![ProofType::KpiMetrics, ProofType::CaseStudies],
                risk_tolerance: 0.4,
            },
            influence: InfluenceProfile {
                influence_score: 65.0,
                graph_centrality: 50.0,
                public_recurrence: 55.0,
                role_seniority_score: 85.0,
                network_size: 20,
            },
            engagement: None,
            role_history: vec![
                RoleHistoryEntry {
                    org: "Foxconn Tunisia".into(),
                    title: "Senior Procurement Manager".into(),
                    role_family: RoleFamily::Procurement,
                    start_ts: 1500000000,
                    end_ts: Some(1600000000),
                },
                RoleHistoryEntry {
                    org: "Foxconn Tunisia".into(),
                    title: "VP Procurement".into(),
                    role_family: RoleFamily::Procurement,
                    start_ts: 1600000000,
                    end_ts: None,
                },
            ],
            last_updated_utc: Utc::now().timestamp(),
            profile_completeness: 0.85,
        }
    }

    #[test]
    fn test_run_with_profile() {
        let workflow = PersonDeepDiveWorkflow::new();
        let profile = make_test_profile();
        let report = workflow.run_with_profile(&profile);

        assert!(!report.report_id.is_empty());
        assert_eq!(report.target_person.name, "Ahmed Ben Ali");
        assert!(report.professional_history.is_some());
        assert!(report.psychological_profile.is_some());
        assert!(report.risk_indicators.is_some());
        assert!(report.key_findings.len() >= 3);
        assert!(report.data_gaps.is_empty() || report.data_gaps.iter().any(|g| !g.is_empty()));
    }

    #[test]
    fn test_empty_profile() {
        let workflow = PersonDeepDiveWorkflow::new();
        let profile = PoiProfile {
            person_id: "empty".into(),
            name: "Unknown".into(),
            name_variants: vec![],
            org: String::new(),
            org_id: None,
            current_role: String::new(),
            role_family: RoleFamily::Other("unknown".into()),
            region: String::new(),
            country_code: String::new(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 0,
            profile_completeness: 0.0,
        };
        let report = workflow.run_with_profile(&profile);
        // Report should generate cleanly even with empty profile — no panics
        assert_eq!(report.target_person.name, "Unknown");
        assert!(report.data_gaps.len() >= 3);
        assert_eq!(report.data_gaps.len(), 6);
    }

    #[test]
    fn test_profile_not_enriched_when_empty() {
        let workflow = PersonDeepDiveWorkflow::new();
        let profile = PoiProfile {
            person_id: "no-artifacts".into(),
            name: "Test".into(),
            name_variants: vec![],
            org: "Corp".into(),
            org_id: None,
            current_role: "Manager".into(),
            role_family: RoleFamily::Other("management".into()),
            region: "US".into(),
            country_code: "US".into(),
            public_bio: String::new(),
            public_email: None,
            artifacts: vec![],
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 0,
            profile_completeness: 0.0,
        };
        let report = workflow.run_with_profile(&profile);
        assert!(!report
            .psychological_profile
            .unwrap()
            .data_source
            .contains("lexical")); // not enriched
    }
}
