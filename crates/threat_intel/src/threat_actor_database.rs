//! # Threat Actor Database Module
//!
//! 3.2.1: Comprehensive threat actor database with industry-specific threat groups,
//! attack patterns, TTPs, and historical campaign analysis.
//!
//! ## Features
//!
//! - Known threat groups with attribution details and verified public intelligence
//! - Attack pattern library with MITRE ATT&CK mapping
//! - Tactics, Techniques, and Procedures (TTPs) tracking
//! - Historical campaign analysis and timeline
//! - Industry-specific threat intelligence
//!
//! ## Data Sources
//!
//! All actor profiles draw from publicly-available, verified intelligence:
//! - MITRE ATT&CK Groups (attack.mitre.org/groups)
//! - CISA Known Exploited Vulnerabilities catalog
//! - Mandiant APT reports
//! - CrowdStrike Falcon Overwatch
//! - Recorded Future threat actor cards
//! - US-CERT alerts and joint advisories

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{Result, ThreatIntelError};
use crate::models::{
    ConfidenceLevel, GeoLocation, IndustrySector, PaginatedResponse, PaginationParams, RiskScore,
};
use crate::mitre_attck::{AttackSubTechnique, AttackTechnique, AttackTactic};

/// State of a threat actor's activity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ActorStatus {
    Active,
    Dormant,
    Suspected,
    Defunct,
    Disrupted,
}

impl ActorStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Dormant => "dormant",
            Self::Suspected => "suspected",
            Self::Defunct => "defunct",
            Self::Disrupted => "disrupted",
        }
    }
}

/// Motivation types for threat actors.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ActorMotivation {
    Financial,
    Espionage,
    Hacktivism,
    Destruction,
    Ideology,
    Coercion,
    Personal,
}

impl ActorMotivation {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Financial => "financial",
            Self::Espionage => "espionage",
            Self::Hacktivism => "hacktivism",
            Self::Destruction => "destruction",
            Self::Ideology => "ideology",
            Self::Coercion => "coercion",
            Self::Personal => "personal",
        }
    }
}

/// Known threat actor or group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatActor {
    pub id: Uuid,
    pub alias: String,
    pub aliases: Vec<String>,
    pub name: Option<String>,
    pub attributed_country: Option<String>,
    pub geo_location: Option<GeoLocation>,
    pub motivation: ActorMotivation,
    pub status: ActorStatus,
    pub target_sectors: Vec<IndustrySector>,
    pub target_regions: Vec<String>,
    pub sponsor: Option<String>,
    pub first_activity: Option<NaiveDate>,
    pub last_activity: Option<NaiveDate>,
    pub sophistication_level: u8,
    pub techniques: Vec<ActorTechnique>,
    pub campaigns: Vec<Campaign>,
    pub references: Vec<ActorReference>,
    pub description: Option<String>,
    pub risk_score: Option<RiskScore>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ThreatActor {
    pub fn new(alias: impl Into<String>, motivation: ActorMotivation, status: ActorStatus) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            alias: alias.into(),
            aliases: Vec::new(),
            name: None,
            attributed_country: None,
            geo_location: None,
            motivation,
            status,
            target_sectors: Vec::new(),
            target_regions: Vec::new(),
            sponsor: None,
            first_activity: None,
            last_activity: None,
            sophistication_level: 5,
            techniques: Vec::new(),
            campaigns: Vec::new(),
            references: Vec::new(),
            description: None,
            risk_score: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn calculate_risk_score(&self) -> RiskScore {
        let mut factors = Vec::new();
        let mut score = 0.0;
        let mut weight_sum = 0.0;

        let status_factor = match self.status {
            ActorStatus::Active => 0.4,
            ActorStatus::Suspected => 0.25,
            ActorStatus::Disrupted => 0.15,
            ActorStatus::Dormant => 0.1,
            ActorStatus::Defunct => 0.0,
        };
        factors.push(crate::models::RiskFactor {
            name: "activity_status".to_string(),
            contribution: status_factor,
            weight: 0.25,
            description: Some("Based on current operational status".to_string()),
        });
        score += status_factor * 0.25;
        weight_sum += 0.25;

        let soph_factor = self.sophistication_level as f64 / 10.0;
        factors.push(crate::models::RiskFactor {
            name: "sophistication".to_string(),
            contribution: soph_factor,
            weight: 0.20,
            description: Some("Based on technical sophistication".to_string()),
        });
        score += soph_factor * 0.20;
        weight_sum += 0.20;

        let targeting_factor = (self.target_sectors.len() as f64 / 5.0).min(1.0);
        factors.push(crate::models::RiskFactor {
            name: "targeting".to_string(),
            contribution: targeting_factor,
            weight: 0.20,
            description: Some("Based on sector targeting".to_string()),
        });
        score += targeting_factor * 0.20;
        weight_sum += 0.20;

        let recency_factor = if let Some(last) = self.last_activity {
            let days_since = (Utc::now().date_naive() - last).num_days();
            if days_since < 30 { 0.15 } else if days_since < 180 { 0.10 } else if days_since < 365 { 0.05 } else { 0.0 }
        } else { 0.05 };
        factors.push(crate::models::RiskFactor {
            name: "recency".to_string(),
            contribution: recency_factor,
            weight: 0.15,
            description: Some("Based on last observed activity".to_string()),
        });
        score += recency_factor * 0.15;
        weight_sum += 0.15;

        let motivation_factor = match self.motivation {
            ActorMotivation::Espionage => 0.25,
            ActorMotivation::Destruction => 0.25,
            ActorMotivation::Financial => 0.15,
            ActorMotivation::Hacktivism => 0.10,
            _ => 0.05,
        };
        factors.push(crate::models::RiskFactor {
            name: "motivation".to_string(),
            contribution: motivation_factor,
            weight: 0.20,
            description: Some("Based on threat actor motivation".to_string()),
        });
        score += motivation_factor * 0.20;
        weight_sum += 0.20;

        let final_score = if weight_sum > 0.0 { score / weight_sum } else { 0.0 };
        let confidence = if self.target_sectors.len() >= 3 && self.techniques.len() >= 5 {
            ConfidenceLevel::High
        } else if !self.target_sectors.is_empty() || self.techniques.len() >= 2 {
            ConfidenceLevel::Medium
        } else {
            ConfidenceLevel::Low
        };

        RiskScore::new(final_score, confidence).with_factors(factors)
    }

    pub fn targets_sector(&self, sector: &IndustrySector) -> bool {
        self.target_sectors.iter().any(|s| s == sector)
    }

    pub fn has_technique(&self, technique_id: &str) -> bool {
        self.techniques.iter().any(|t| t.technique_id == technique_id)
    }
}

/// Reference to an external source about the threat actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorReference {
    pub source: String,
    pub url: Option<String>,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

/// Technique used by a specific threat actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorTechnique {
    pub technique_id: String,
    pub technique_name: String,
    pub confidence: ConfidenceLevel,
    pub first_used: Option<NaiveDate>,
    pub last_used: Option<NaiveDate>,
    pub usage_count: u32,
    pub associated_campaigns: Vec<String>,
    pub evidence: String,
}

impl ActorTechnique {
    pub fn new(technique_id: impl Into<String>, technique_name: impl Into<String>) -> Self {
        Self {
            technique_id: technique_id.into(),
            technique_name: technique_name.into(),
            confidence: ConfidenceLevel::Medium,
            first_used: None,
            last_used: None,
            usage_count: 0,
            associated_campaigns: Vec::new(),
            evidence: String::new(),
        }
    }
}

/// Historical campaign attributed to a threat actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Campaign {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub target_sectors: Vec<IndustrySector>,
    pub target_regions: Vec<String>,
    pub targeted_organizations: Vec<String>,
    pub techniques_used: Vec<String>,
    pub impact_summary: Option<String>,
    pub estimated_damage: Option<String>,
    pub references: Vec<ActorReference>,
    pub attribution_confidence: ConfidenceLevel,
}

impl Campaign {
    pub fn new(name: impl Into<String>, start_date: NaiveDate) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: String::new(),
            start_date,
            end_date: None,
            target_sectors: Vec::new(),
            target_regions: Vec::new(),
            targeted_organizations: Vec::new(),
            techniques_used: Vec::new(),
            impact_summary: None,
            estimated_damage: None,
            references: Vec::new(),
            attribution_confidence: ConfidenceLevel::Medium,
        }
    }

    pub fn duration_days(&self) -> Option<i64> {
        self.end_date.map(|end| (end - self.start_date).num_days())
    }
}

/// Attack pattern in the MITRE ATT&CK style.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackPattern {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mitre_id: Option<String>,
    pub related_techniques: Vec<String>,
    pub detection_rules: Vec<DetectionRule>,
    pub mitigation_strategies: Vec<String>,
    pub applicable_sectors: Vec<IndustrySector>,
    pub risk_factors: Vec<PatternRiskFactor>,
}

impl AttackPattern {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            mitre_id: None,
            related_techniques: Vec::new(),
            detection_rules: Vec::new(),
            mitigation_strategies: Vec::new(),
            applicable_sectors: Vec::new(),
            risk_factors: Vec::new(),
        }
    }
}

/// Detection rule for identifying an attack pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionRule {
    pub id: String,
    pub name: String,
    pub rule_content: String,
    pub platform: String,
    pub false_positive_rate: Option<f64>,
}

/// Risk factor for an attack pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRiskFactor {
    pub factor: String,
    pub weight: f64,
    pub description: String,
}

/// TTP specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTP {
    pub id: Uuid,
    pub name: String,
    pub tactic: AttackTactic,
    pub technique: Option<AttackTechnique>,
    pub sub_technique_id: Option<String>,
    pub description: String,
    pub procedures: Vec<Procedure>,
    pub indicators: Vec<Indicator>,
    pub related_malware: Vec<String>,
    pub related_tools: Vec<String>,
    pub applicable_sectors: Vec<IndustrySector>,
    pub complexity: ComplexityLevel,
    pub effectiveness: f64,
    pub metadata: serde_json::Value,
}

impl TTP {
    pub fn new(name: impl Into<String>, tactic: AttackTactic) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            tactic,
            technique: None,
            sub_technique_id: None,
            description: String::new(),
            procedures: Vec::new(),
            indicators: Vec::new(),
            related_malware: Vec::new(),
            related_tools: Vec::new(),
            applicable_sectors: Vec::new(),
            complexity: ComplexityLevel::Medium,
            effectiveness: 0.5,
            metadata: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplexityLevel {
    Low,
    Medium,
    High,
    VeryHigh,
}

/// Indicator of compromise or attack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Indicator {
    pub id: Uuid,
    pub indicator_type: IndicatorType,
    pub value: String,
    pub context: Option<String>,
    pub first_observed: DateTime<Utc>,
    pub last_observed: Option<DateTime<Utc>>,
    pub confidence: ConfidenceLevel,
    pub linked_actors: Vec<String>,
    pub linked_campaigns: Vec<Uuid>,
}

impl Indicator {
    pub fn new(indicator_type: IndicatorType, value: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            indicator_type,
            value: value.into(),
            context: None,
            first_observed: Utc::now(),
            last_observed: None,
            confidence: ConfidenceLevel::Medium,
            linked_actors: Vec::new(),
            linked_campaigns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IndicatorType {
    IPv4,
    IPv6,
    Domain,
    URL,
    FileHash,
    FileName,
    Registry,
    Mutex,
    Certificate,
    Email,
    UserAgent,
    Custom,
}

/// Individual procedure within a TTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    pub step: u32,
    pub description: String,
    pub command: Option<String>,
    pub tool_required: Option<String>,
    pub privileges_required: Option<String>,
    pub os_target: OperatingSystem,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Linux,
    MacOS,
    Android,
    IOS,
    CrossPlatform,
}

/// Threat Actor Database - main interface.
#[derive(Debug, Clone, Default)]
pub struct ThreatActorDatabase {
    actors: HashMap<Uuid, ThreatActor>,
    actors_by_alias: HashMap<String, Uuid>,
    actors_by_sector: HashMap<IndustrySector, Vec<Uuid>>,
    patterns: HashMap<String, AttackPattern>,
    ttps: HashMap<Uuid, TTP>,
    indicators: HashMap<Uuid, Indicator>,
}

impl ThreatActorDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create database with pre-populated known threat actors from verified intelligence.
    pub fn with_known_actors() -> Self {
        let mut db = Self::default();
        db.populate_known_actors();
        db
    }

    /// Iterate over every threat actor in the database.
    ///
    /// Used by downstream consumers (e.g. the worker threat-intel refresh job)
    /// to match tracked companies against the full set of known actors by
    /// industry sector and/or target geography.
    pub fn iter_actors(&self) -> impl Iterator<Item = &ThreatActor> {
        self.actors.values()
    }

    /// Total number of threat actors held in the database.
    pub fn len(&self) -> usize {
        self.actors.len()
    }

    /// Returns `true` if the database holds no threat actors.
    pub fn is_empty(&self) -> bool {
        self.actors.is_empty()
    }

    pub fn add_actor(&mut self, actor: ThreatActor) -> Result<Uuid> {
        if self.actors_by_alias.contains_key(&actor.alias) {
            return Err(ThreatIntelError::data_integrity(format!(
                "Threat actor with alias '{}' already exists",
                actor.alias
            )));
        }
        let id = actor.id;
        self.actors.insert(id, actor.clone());
        self.actors_by_alias.insert(actor.alias.clone(), id);
        for sector in &actor.target_sectors {
            self.actors_by_sector.entry(sector.clone()).or_default().push(id);
        }
        Ok(id)
    }

    pub fn get_actor(&self, id: Uuid) -> Option<&ThreatActor> {
        self.actors.get(&id)
    }

    pub fn get_actor_by_alias(&self, alias: &str) -> Option<&ThreatActor> {
        self.actors_by_alias.get(alias).and_then(|id| self.actors.get(id))
    }

    pub fn get_actors_by_sector(&self, sector: &IndustrySector) -> Vec<&ThreatActor> {
        self.actors_by_sector
            .get(sector)
            .map(|ids| ids.iter().filter_map(|id| self.actors.get(id)).collect())
            .unwrap_or_default()
    }

    pub fn get_actors_by_status(&self, status: ActorStatus) -> Vec<&ThreatActor> {
        self.actors.values().filter(|a| a.status == status).collect()
    }

    pub fn get_actors_by_motivation(&self, motivation: ActorMotivation) -> Vec<&ThreatActor> {
        self.actors.values().filter(|a| a.motivation == motivation).collect()
    }

    pub fn search_actors(&self, query: &str) -> Vec<&ThreatActor> {
        let query_lower = query.to_lowercase();
        self.actors
            .values()
            .filter(|a| {
                a.alias.to_lowercase().contains(&query_lower)
                    || a.aliases.iter().any(|alias| alias.to_lowercase().contains(&query_lower))
                    || a.name.as_ref().is_some_and(|n| n.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    pub fn list_actors(&self, params: PaginationParams) -> PaginatedResponse<ThreatActor> {
        let total = self.actors.len() as u64;
        let mut actors: Vec<_> = self.actors.values().cloned().collect();
        actors.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        let offset = params.offset as usize;
        let limit = params.limit as usize;
        let items: Vec<_> = actors.into_iter().skip(offset).take(limit).collect();
        PaginatedResponse::new(items, total, params.offset, params.limit)
    }

    pub fn add_pattern(&mut self, pattern: AttackPattern) {
        self.patterns.insert(pattern.id.clone(), pattern);
    }

    pub fn get_pattern(&self, id: &str) -> Option<&AttackPattern> {
        self.patterns.get(id)
    }

    pub fn get_patterns_by_sector(&self, sector: &IndustrySector) -> Vec<&AttackPattern> {
        self.patterns.values().filter(|p| p.applicable_sectors.contains(sector)).collect()
    }

    pub fn add_ttp(&mut self, ttp: TTP) -> Uuid {
        let id = ttp.id;
        self.ttps.insert(id, ttp);
        id
    }

    pub fn get_ttp(&self, id: Uuid) -> Option<&TTP> {
        self.ttps.get(&id)
    }

    pub fn get_ttps_by_tactic(&self, tactic: AttackTactic) -> Vec<&TTP> {
        self.ttps.values().filter(|t| t.tactic == tactic).collect()
    }

    pub fn add_indicator(&mut self, indicator: Indicator) -> Uuid {
        let id = indicator.id;
        self.indicators.insert(id, indicator);
        id
    }

    pub fn search_indicators(&self, pattern: &str) -> Vec<&Indicator> {
        let pattern_lower = pattern.to_lowercase();
        self.indicators.values().filter(|i| i.value.to_lowercase().contains(&pattern_lower)).collect()
    }

    pub fn get_indicators_by_type(&self, indicator_type: IndicatorType) -> Vec<&Indicator> {
        self.indicators.values().filter(|i| i.indicator_type == indicator_type).collect()
    }

    pub fn find_actors_by_technique(&self, technique_id: &str) -> Vec<&ThreatActor> {
        self.actors.values().filter(|a| a.has_technique(technique_id)).collect()
    }

    pub fn get_sector_threat_summary(&self, sector: &IndustrySector) -> SectorThreatSummary {
        let actors = self.get_actors_by_sector(sector);
        let active_count = actors.iter().filter(|a| a.status == ActorStatus::Active).count();
        let patterns = self.get_patterns_by_sector(sector);
        let techniques: Vec<_> = actors.iter().flat_map(|a| a.techniques.iter().map(|t| t.technique_id.clone())).collect();
        let mut unique_techniques = techniques.clone();
        unique_techniques.sort();
        unique_techniques.dedup();
        SectorThreatSummary {
            sector: sector.clone(),
            total_actors: actors.len(),
            active_actors: active_count,
            attack_patterns: patterns.len(),
            unique_techniques: unique_techniques.len(),
            top_actors: actors.iter().take(5).map(|a| a.alias.clone()).collect(),
            most_used_techniques: techniques,
            overall_risk_score: if actors.is_empty() {
                0.0
            } else {
                actors.iter().map(|a| a.calculate_risk_score().score).sum::<f64>() / actors.len() as f64
            },
        }
    }

    // ── Population of verified threat actors ────────────────────────────────
    // All data sourced from: MITRE ATT&CK, CISA advisories, Mandiant reports,
    // CrowdStrike Global Threat Report, joint government advisories.

    #[allow(clippy::disallowed_methods, unused_must_use)]
    fn populate_known_actors(&mut self) {
        // ── APT28 — Fancy Bear (GRU Unit 26165, Russia) ──
        self.add_actor(
            ThreatActor::new("APT28", ActorMotivation::Espionage, ActorStatus::Active)
                .with_name("Fancy Bear")
                .with_aliases(vec!["Fancy Bear".into(), "Sofacy".into(), "Pawn Storm".into(),
                    "Sednit".into(), "STRONTIUM".into(), "Tsar Team".into()])
                .with_attributed_country("RU")
                .with_sponsor("GRU Unit 26165")
                .with_target_sectors(vec![IndustrySector::Government, IndustrySector::Defense,
                    IndustrySector::Aerospace, IndustrySector::Energy, IndustrySector::Technology])
                .with_target_regions(vec!["US".into(), "EU".into(), "UK".into(), "NATO".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2004, 1, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
                .with_sophistication(8)
                .with_description("GRU-affiliated espionage group targeting governments, militaries, and security organizations since 2004. Conducted intrusions against DNC, Bundestag, WADA, and NATO entities.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1566".into(), technique_name: "Phishing".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Spear-phishing emails with malicious Office documents and links to credential harvesting sites".into() },
                    ActorTechnique { technique_id: "T1566.001".into(), technique_name: "Spearphishing Attachment".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Malicious macros in Word documents exploiting CVE-2017-0199".into() },
                    ActorTechnique { technique_id: "T1003".into(), technique_name: "OS Credential Dumping".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Mimikatz usage for credential extraction from LSASS memory".into() },
                    ActorTechnique { technique_id: "T1059".into(), technique_name: "Command and Scripting Interpreter".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "PowerShell Empire post-exploitation framework deployment".into() },
                    ActorTechnique { technique_id: "T1071".into(), technique_name: "Application Layer Protocol".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "X-Agent malware using HTTP/S for C2 with custom encryption".into() },
                    ActorTechnique { technique_id: "T1027".into(), technique_name: "Obfuscated Files or Information".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Base64-encoded PowerShell payloads and encrypted configuration files".into() },
                    ActorTechnique { technique_id: "T1041".into(), technique_name: "Exfiltration Over C2 Channel".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Data exfiltration via encrypted HTTP POST requests to attacker-controlled servers".into() },
                    ActorTechnique { technique_id: "T1210".into(), technique_name: "Exploitation of Remote Services".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "EternalBlue (MS17-010) exploitation for lateral movement, inherited from Equation Group tools".into() },
                ])
                .with_campaigns(vec![
                    Campaign {
                        id: Uuid::new_v4(),
                        name: "Pawn Storm".into(),
                        description: "Long-running multi-platform espionage campaign targeting government, military, and media organizations globally".into(),
                        start_date: NaiveDate::from_ymd_opt(2014, 1, 1).unwrap(),
                        end_date: None,
                        target_sectors: vec![IndustrySector::Government, IndustrySector::Defense, IndustrySector::Aerospace],
                        target_regions: vec!["Global".into()],
                        targeted_organizations: vec!["NATO".into(), "DNC".into(), "Bundestag".into(), "WADA".into()],
                        techniques_used: vec!["T1566".into(), "T1003".into(), "T1059".into()],
                        impact_summary: Some("Breach of DNC email systems, Bundestag network compromise, theft of Olympic athlete medical data".into()),
                        estimated_damage: Some("Hundreds of millions in remediation costs and diplomatic fallout".into()),
                        references: vec![],
                        attribution_confidence: ConfidenceLevel::High,
                    },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0007/".into()), description: Some("APT28 group profile".into()), published_at: None },
                    ActorReference { source: "CISA".into(), url: Some("https://www.cisa.gov/uscert/ncas/alerts/aa22-110a".into()), description: Some("Joint advisory on GRU cyber operations".into()), published_at: None },
                    ActorReference { source: "CrowdStrike".into(), url: Some("https://www.crowdstrike.com/blog/who-is-fancy-bear/".into()), description: Some("Fancy Bear actor profile".into()), published_at: None },
                ])
        );

        // ── APT29 — Cozy Bear (SVR, Russia) ──
        self.add_actor(
            ThreatActor::new("APT29", ActorMotivation::Espionage, ActorStatus::Active)
                .with_name("Cozy Bear")
                .with_aliases(vec!["Cozy Bear".into(), "The Dukes".into(), "NOBELIUM".into(),
                    "Midnight Blizzard".into(), "CozyDuke".into(), "Ytterbium".into()])
                .with_attributed_country("RU")
                .with_sponsor("SVR (Foreign Intelligence Service)")
                .with_target_sectors(vec![IndustrySector::Government, IndustrySector::Healthcare,
                    IndustrySector::Pharmaceuticals, IndustrySector::Technology,
                    IndustrySector::Education, IndustrySector::Financial])
                .with_target_regions(vec!["US".into(), "EU".into(), "UK".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2008, 1, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
                .with_sophistication(9)
                .with_description("SVR-affiliated threat actor known for sophisticated supply chain attacks (SolarWinds), COVID-19 vaccine research targeting, and long-term stealth operations against Western government and technology targets.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1195".into(), technique_name: "Supply Chain Compromise".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "SolarWinds Orion build system compromise injecting SUNBURST backdoor (2020)".into() },
                    ActorTechnique { technique_id: "T1190".into(), technique_name: "Exploit Public-Facing Application".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Exploitation of vulnerable public-facing web applications for initial access".into() },
                    ActorTechnique { technique_id: "T1003".into(), technique_name: "OS Credential Dumping".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "NTDS.dit extraction and LSASS memory dumping for domain credential theft".into() },
                    ActorTechnique { technique_id: "T1078".into(), technique_name: "Valid Accounts".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Use of stolen valid credentials for lateral movement and persistence".into() },
                    ActorTechnique { technique_id: "T1059".into(), technique_name: "Command and Scripting Interpreter".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "PowerShell and Python scripts for execution and C2 via Cobalt Strike".into() },
                    ActorTechnique { technique_id: "T1573".into(), technique_name: "Encrypted Channel".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "TLS-encrypted C2 traffic mimicking legitimate Microsoft and Amazon cloud services".into() },
                    ActorTechnique { technique_id: "T1027".into(), technique_name: "Obfuscated Files or Information".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "TEARDROP and RAINDROP loaders with custom encoding and memory-only execution".into() },
                    ActorTechnique { technique_id: "T1547".into(), technique_name: "Boot or Logon Autostart Execution".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Scheduled tasks and registry Run keys for persistence after SUNBURST phase".into() },
                ])
                .with_campaigns(vec![
                    Campaign {
                        id: Uuid::new_v4(),
                        name: "SolarWinds Supply Chain".into(),
                        description: "Landmark supply chain attack compromising SolarWinds Orion platform to distribute SUNBURST backdoor to ~18,000 organizations; focused follow-on operations against ~100 high-value targets".into(),
                        start_date: NaiveDate::from_ymd_opt(2019, 9, 1).unwrap(),
                        end_date: Some(NaiveDate::from_ymd_opt(2021, 6, 1).unwrap()),
                        target_sectors: vec![IndustrySector::Government, IndustrySector::Technology, IndustrySector::Financial],
                        target_regions: vec!["US".into(), "EU".into(), "UK".into()],
                        targeted_organizations: vec!["US Treasury".into(), "US DHS".into(), "Microsoft".into(), "FireEye".into()],
                        techniques_used: vec!["T1195".into(), "T1078".into(), "T1573".into()],
                        impact_summary: Some("Massive supply chain compromise affecting thousands of organizations, theft of sensitive government and corporate data".into()),
                        estimated_damage: Some("Multi-billion dollar remediation effort, extensive diplomatic consequences".into()),
                        references: vec![],
                        attribution_confidence: ConfidenceLevel::High,
                    },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0016/".into()), description: Some("APT29 group profile".into()), published_at: None },
                    ActorReference { source: "CISA SolarWinds Advisory".into(), url: Some("https://www.cisa.gov/solarwinds".into()), description: Some("SolarWinds compromise response".into()), published_at: None },
                ])
        );

        // ── APT33 (Elfin, Iran) ──
        self.add_actor(
            ThreatActor::new("APT33", ActorMotivation::Espionage, ActorStatus::Active)
                .with_name("Elfin")
                .with_aliases(vec!["Elfin".into(), "Refined Kitten".into(), "MAGNALLIUM".into()])
                .with_attributed_country("IR")
                .with_sponsor("IRGC")
                .with_target_sectors(vec![IndustrySector::Aerospace, IndustrySector::Energy,
                    IndustrySector::Defense, IndustrySector::Manufacturing])
                .with_target_regions(vec!["US".into(), "ME".into(), "EU".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2013, 1, 1).unwrap())
                .with_sophistication(7)
                .with_description("Iranian state-sponsored group targeting aerospace, energy, and defense sectors. Known for destructive wiper attacks (Shamoon) alongside espionage operations.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1566".into(), technique_name: "Phishing".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Spear-phishing with job recruitment lures targeting aviation and energy engineers".into() },
                    ActorTechnique { technique_id: "T1110".into(), technique_name: "Brute Force".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Password spraying against Office 365 and VPN portals".into() },
                    ActorTechnique { technique_id: "T1485".into(), technique_name: "Data Destruction".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Shamoon wiper malware destroying master boot records and file systems".into() },
                    ActorTechnique { technique_id: "T1078".into(), technique_name: "Valid Accounts".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Compromised valid domain credentials for lateral movement".into() },
                    ActorTechnique { technique_id: "T1090".into(), technique_name: "Proxy".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Use of TOR and commercial VPN services for operational security".into() },
                    ActorTechnique { technique_id: "T1105".into(), technique_name: "Ingress Tool Transfer".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "DropBox and custom FTP tools for tool staging and data exfiltration".into() },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0064/".into()), description: Some("APT33 group profile".into()), published_at: None },
                ])
        );

        // ── Lazarus Group (APT38, North Korea) ──
        self.add_actor(
            ThreatActor::new("Lazarus Group", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("APT38")
                .with_aliases(vec!["Lazarus Group".into(), "APT38".into(), "HIDDEN COBRA".into(),
                    "ZINC".into(), "Stardust Chollima".into(), "Appleworm".into()])
                .with_attributed_country("KP")
                .with_sponsor("RGB (Reconnaissance General Bureau)")
                .with_target_sectors(vec![IndustrySector::Financial, IndustrySector::Technology,
                    IndustrySector::Gaming, IndustrySector::Healthcare, IndustrySector::Pharmaceuticals])
                .with_target_regions(vec!["Global".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2009, 1, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
                .with_sophistication(9)
                .with_description("North Korean state-sponsored threat actor responsible for some of the most destructive and financially motivated cyber operations in history, including SWIFT banking heists, cryptocurrency theft, and destructive wiper attacks.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1566".into(), technique_name: "Phishing".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Spear-phishing targeting cryptocurrency exchanges, financial institutions, and defense contractors".into() },
                    ActorTechnique { technique_id: "T1496".into(), technique_name: "Resource Hijacking".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Cryptojacking operations to generate revenue for the regime".into() },
                    ActorTechnique { technique_id: "T1486".into(), technique_name: "Data Encrypted for Impact".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "WannaCry ransomware and VHD ransomware deployment".into() },
                    ActorTechnique { technique_id: "T1485".into(), technique_name: "Data Destruction".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Destructive wiper attacks against Sony Pictures Entertainment (2014)".into() },
                    ActorTechnique { technique_id: "T1560".into(), technique_name: "Archive Collected Data".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Data staging and compression before exfiltration using custom packers".into() },
                    ActorTechnique { technique_id: "T1059".into(), technique_name: "Command and Scripting Interpreter".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Custom backdoors (Dtrack, Hoplight) and PowerShell scripts for post-exploitation".into() },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0032/".into()), description: Some("Lazarus Group profile".into()), published_at: None },
                    ActorReference { source: "CISA HIDDEN COBRA".into(), url: Some("https://www.cisa.gov/topics/cyber-threats-and-advisories/advanced-persistent-threats/north-korea".into()), description: Some("North Korea cyber threat overview".into()), published_at: None },
                ])
        );

        // ── FIN7 (Carbanak) ──
        self.add_actor(
            ThreatActor::new("FIN7", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("Carbanak Group")
                .with_aliases(vec!["Carbanak".into(), "Carbon Spider".into(), "Navy Beetle".into(),
                    "JokerStash".into(), "Anunak".into()])
                .with_attributed_country("RU")
                .with_target_sectors(vec![IndustrySector::Financial, IndustrySector::Retail,
                    IndustrySector::Technology, IndustrySector::Gaming])
                .with_target_regions(vec!["US".into(), "EU".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2013, 1, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
                .with_sophistication(7)
                .with_description("Financially motivated organized crime group that has stolen over $1 billion from financial institutions and retailers globally using sophisticated point-of-sale and ATM malware, as well as ransomware operations.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1566".into(), technique_name: "Phishing".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Spear-phishing with malicious Office documents using social engineering lures tailored to targets".into() },
                    ActorTechnique { technique_id: "T1203".into(), technique_name: "Exploitation for Client Execution".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Exploitation of browser and application vulnerabilities for initial access".into() },
                    ActorTechnique { technique_id: "T1090".into(), technique_name: "Proxy".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "SOCKS proxy usage over SSH tunnels for C2 communication".into() },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0046/".into()), description: Some("FIN7 group profile".into()), published_at: None },
                ])
        );

        // ── LockBit Ransomware ──
        self.add_actor(
            ThreatActor::new("LockBit", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("LockBit Ransomware Group")
                .with_aliases(vec!["LockBit".into(), "LockBit 2.0".into(), "LockBit 3.0".into(),
                    "LockBit Green".into(), "LockBit Black".into()])
                .with_attributed_country("RU")
                .with_target_sectors(vec![IndustrySector::Manufacturing, IndustrySector::Technology,
                    IndustrySector::Healthcare, IndustrySector::Education, IndustrySector::Government])
                .with_target_regions(vec!["Global".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2019, 9, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
                .with_sophistication(8)
                .with_description("Ransomware-as-a-Service (RaaS) operation that became one of the most prolific ransomware families globally, known for double extortion tactics (encryption + data leak) and the continuous development of new variants.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1486".into(), technique_name: "Data Encrypted for Impact".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "LockBit ransomware encrypting files with AES + RSA encryption, appending .lockbit extension".into() },
                    ActorTechnique { technique_id: "T1041".into(), technique_name: "Exfiltration Over C2 Channel".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "StealBit data exfiltration tool uploading victim data before encryption for double extortion".into() },
                    ActorTechnique { technique_id: "T1562".into(), technique_name: "Impair Defenses".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Disabling Windows Defender, deleting shadow copies via vssadmin, terminating security services".into() },
                ])
                .with_references(vec![
                    ActorReference { source: "CISA LockBit Advisory".into(), url: Some("https://www.cisa.gov/news-events/cybersecurity-advisories/aa23-165a".into()), description: Some("Understanding Ransomware Threat Actors: LockBit".into()), published_at: None },
                ])
        );

        // ── APT41 (Wicked Panda, China) ──
        self.add_actor(
            ThreatActor::new("APT41", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("Wicked Panda")
                .with_aliases(vec!["Wicked Panda".into(), "Wicked Spider".into(), "BARIUM".into(),
                    "Caster".into(), "Winnti Group".into()])
                .with_attributed_country("CN")
                .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Healthcare,
                    IndustrySector::Gaming, IndustrySector::Pharmaceuticals, IndustrySector::Telecommunications])
                .with_target_regions(vec!["US".into(), "APAC".into(), "EU".into()])
                .with_first_activity(NaiveDate::from_ymd_opt(2012, 1, 1).unwrap())
                .with_last_activity(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
                .with_sophistication(8)
                .with_description("Dual-use group conducting both state-sponsored espionage and financially motivated operations. Known for supply chain compromises, gaming industry targeting, and theft of intellectual property.")
                .with_techniques(vec![
                    ActorTechnique { technique_id: "T1190".into(), technique_name: "Exploit Public-Facing Application".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Exploitation of Citrix (CVE-2019-19781) and Pulse Secure (CVE-2019-11510) VPN appliances".into() },
                    ActorTechnique { technique_id: "T1566".into(), technique_name: "Phishing".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Spear-phishing targeting gaming companies with fake job offers and CV lures".into() },
                    ActorTechnique { technique_id: "T1003".into(), technique_name: "OS Credential Dumping".into(), confidence: ConfidenceLevel::High, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "Mimikatz and custom credential harvesting tools for domain privilege escalation".into() },
                    ActorTechnique { technique_id: "T1195".into(), technique_name: "Supply Chain Compromise".into(), confidence: ConfidenceLevel::Medium, first_used: None, last_used: None, usage_count: 0, associated_campaigns: vec![], evidence: "CCleaner supply chain attack compromising software update mechanism".into() },
                ])
                .with_references(vec![
                    ActorReference { source: "MITRE ATT&CK".into(), url: Some("https://attack.mitre.org/groups/G0096/".into()), description: Some("APT41 group profile".into()), published_at: None },
                ])
        );

        // ── Add attack patterns ──
        self.add_pattern(AttackPattern::new("PAT-001", "Supply Chain Compromise")
            .with_description("Compromising software dependencies, update mechanisms, or hardware components")
            .with_mitre_id("T1195")
            .with_mitigation(vec![
                "Implement code signing verification".into(),
                "Maintain SBOM inventory".into(),
                "Verify dependency integrity".into(),
                "Monitor build pipeline for unauthorized changes".into(),
            ])
        );

        self.add_pattern(AttackPattern::new("PAT-002", "Phishing with Spearphishing Attachment")
            .with_description("Targeted phishing with malicious attachments")
            .with_mitre_id("T1566.001")
            .with_mitigation(vec![
                "Email filtering with sandbox analysis".into(),
                "User awareness training".into(),
                "Attachment sandboxing".into(),
                "Disable macros in documents from external sources".into(),
            ])
        );

        self.add_pattern(AttackPattern::new("PAT-003", "Ransomware Double Extortion")
            .with_description("Encrypting data for ransom while exfiltrating copies to pressure payment through leak threats")
            .with_mitre_id("T1486")
            .with_mitigation(vec![
                "Offline encrypted backups".into(),
                "Network segmentation".into(),
                "EDR deployment".into(),
                "Egress filtering and DLP".into(),
                "Credential hardening and MFA".into(),
            ])
        );
    }
}

/// Summary of threats for a specific sector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorThreatSummary {
    pub sector: IndustrySector,
    pub total_actors: usize,
    pub active_actors: usize,
    pub attack_patterns: usize,
    pub unique_techniques: usize,
    pub top_actors: Vec<String>,
    pub most_used_techniques: Vec<String>,
    pub overall_risk_score: f64,
}

// Builder pattern extensions
impl ThreatActor {
    pub fn with_name(mut self, name: impl Into<String>) -> Self { self.name = Some(name.into()); self }
    pub fn with_aliases(mut self, aliases: Vec<String>) -> Self { self.aliases = aliases; self }
    pub fn with_attributed_country(mut self, country: impl Into<String>) -> Self { self.attributed_country = Some(country.into()); self }
    pub fn with_sponsor(mut self, sponsor: impl Into<String>) -> Self { self.sponsor = Some(sponsor.into()); self }
    pub fn with_target_sectors(mut self, sectors: Vec<IndustrySector>) -> Self { self.target_sectors = sectors; self }
    pub fn with_target_regions(mut self, regions: Vec<String>) -> Self { self.target_regions = regions; self }
    pub fn with_first_activity(mut self, date: NaiveDate) -> Self { self.first_activity = Some(date); self }
    pub fn with_last_activity(mut self, date: NaiveDate) -> Self { self.last_activity = Some(date); self }
    pub fn with_sophistication(mut self, level: u8) -> Self { self.sophistication_level = level.min(10); self }
    pub fn with_techniques(mut self, techniques: Vec<ActorTechnique>) -> Self { self.techniques = techniques; self }
    pub fn with_campaigns(mut self, campaigns: Vec<Campaign>) -> Self { self.campaigns = campaigns; self }
    pub fn with_references(mut self, references: Vec<ActorReference>) -> Self { self.references = references; self }
    pub fn with_description(mut self, desc: impl Into<String>) -> Self { self.description = Some(desc.into()); self }
}

impl AttackPattern {
    pub fn with_description(mut self, desc: impl Into<String>) -> Self { self.description = desc.into(); self }
    pub fn with_mitre_id(mut self, id: impl Into<String>) -> Self { self.mitre_id = Some(id.into()); self }
    pub fn with_mitigation(mut self, mitigations: Vec<String>) -> Self { self.mitigation_strategies = mitigations; self }
    pub fn with_sectors(mut self, sectors: Vec<IndustrySector>) -> Self { self.applicable_sectors = sectors; self }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_creation() {
        let actor = ThreatActor::new("TEST-ACTOR", ActorMotivation::Financial, ActorStatus::Active);
        assert_eq!(actor.alias, "TEST-ACTOR");
        assert_eq!(actor.motivation, ActorMotivation::Financial);
        assert_eq!(actor.status, ActorStatus::Active);
    }

    #[test]
    fn test_actor_builder() {
        let actor = ThreatActor::new("APT-TEST", ActorMotivation::Espionage, ActorStatus::Active)
            .with_name("Test APT Group")
            .with_aliases(vec!["Test".into(), "Fake".into()])
            .with_attributed_country("US")
            .with_target_sectors(vec![IndustrySector::Technology])
            .with_sophistication(8);
        assert_eq!(actor.name, Some("Test APT Group".to_string()));
        assert_eq!(actor.attributed_country, Some("US".to_string()));
        assert_eq!(actor.target_sectors.len(), 1);
        assert_eq!(actor.sophistication_level, 8);
    }

    #[test]
    fn test_database_operations() {
        let mut db = ThreatActorDatabase::new();
        let id = db.add_actor(ThreatActor::new("TEST", ActorMotivation::Financial, ActorStatus::Active)).unwrap();
        assert!(db.get_actor(id).is_some());
        assert!(db.get_actor_by_alias("TEST").is_some());
        assert!(db.get_actor_by_alias("UNKNOWN").is_none());
    }

    #[test]
    fn test_duplicate_alias_rejected() {
        let mut db = ThreatActorDatabase::new();
        db.add_actor(ThreatActor::new("TEST", ActorMotivation::Financial, ActorStatus::Active)).unwrap();
        let result = db.add_actor(ThreatActor::new("TEST", ActorMotivation::Espionage, ActorStatus::Active));
        assert!(result.is_err());
    }

    #[test]
    fn test_known_actors_populated() {
        let db = ThreatActorDatabase::with_known_actors();
        assert!(db.get_actor_by_alias("APT28").is_some());
        assert!(db.get_actor_by_alias("APT29").is_some());
        assert!(db.get_actor_by_alias("Lazarus Group").is_some());
        assert!(db.get_actor_by_alias("FIN7").is_some());
        assert!(db.get_actor_by_alias("LockBit").is_some());
        assert!(db.actors.len() >= 7);
    }

    #[test]
    fn test_sector_threat_summary() {
        let db = ThreatActorDatabase::with_known_actors();
        let summary = db.get_sector_threat_summary(&IndustrySector::Government);
        assert!(summary.total_actors >= 2);
        assert!(summary.active_actors >= 2);
        assert!(summary.overall_risk_score > 0.0);
    }
}