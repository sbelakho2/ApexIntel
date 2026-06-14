//! # Threat Actor Database Module
//!
//! 3.2.1: Comprehensive threat actor database with industry-specific threat groups,
//! attack patterns, TTPs, and historical campaign analysis.
//!
//! ## Features
//!
//! - Known threat groups with attribution details
//! - Attack pattern library with MITRE ATT&CK mapping
//! - Tactics, Techniques, and Procedures (TTPs) tracking
//! - Historical campaign analysis and timeline
//! - Industry-specific threat intelligence

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{Result, ThreatIntelError};
use crate::models::{ConfidenceLevel, GeoLocation, IndustrySector, PaginationParams, PaginatedResponse, RiskScore};
use crate::mitre_attck::{AttackTechnique, AttackTactic};

/// State of a threat actor's activity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActorStatus {
    /// Currently active and conducting operations
    Active,
    /// Previously active but currently dormant
    Dormant,
    /// Attribution is uncertain
    Suspected,
    /// No longer operating
    Defunct,
    /// Operations disrupted but may return
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
    /// Primary identifier (e.g., "APT29", "FIN7", "Lazarus Group")
    pub alias: String,
    /// Additional known aliases
    pub aliases: Vec<String>,
    /// Short descriptive name
    pub name: Option<String>,
    /// National origin or attribution
    pub attributed_country: Option<String>,
    /// Geographic base of operations
    pub geo_location: Option<GeoLocation>,
    /// Primary motivation
    pub motivation: ActorMotivation,
    /// Current operational status
    pub status: ActorStatus,
    /// Primary target sectors
    pub target_sectors: Vec<IndustrySector>,
    /// Geographic targeting preferences
    pub target_regions: Vec<String>,
    /// Organization sponsor (state-sponsored groups)
    pub sponsor: Option<String>,
    /// When the group was first identified
    pub first_activity: Option<NaiveDate>,
    /// When the group was last observed
    pub last_activity: Option<NaiveDate>,
    /// Known capabilities and sophistication level (1-10)
    pub sophistication_level: u8,
    /// Known TTPs used by this actor
    pub techniques: Vec<ActorTechnique>,
    /// Historical campaigns attributed to this actor
    pub campaigns: Vec<Campaign>,
    /// External references and sources
    pub references: Vec<ActorReference>,
    /// Detailed description and notes
    pub description: Option<String>,
    /// Risk score based on targeting
    pub risk_score: Option<RiskScore>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ThreatActor {
    /// Create a new threat actor with required fields.
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

    /// Calculate overall risk score based on actor targeting and activity.
    pub fn calculate_risk_score(&self) -> RiskScore {
        let mut factors = Vec::new();
        let mut score = 0.0;
        let mut weight_sum = 0.0;

        // Activity status factor
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

        // Sophistication factor
        let soph_factor = self.sophistication_level as f64 / 10.0;
        factors.push(crate::models::RiskFactor {
            name: "sophistication".to_string(),
            contribution: soph_factor,
            weight: 0.20,
            description: Some("Based on technical sophistication".to_string()),
        });
        score += soph_factor * 0.20;
        weight_sum += 0.20;

        // Targeting relevance factor (how many of our sectors are targeted)
        let targeting_factor = (self.target_sectors.len() as f64 / 5.0).min(1.0);
        factors.push(crate::models::RiskFactor {
            name: "targeting".to_string(),
            contribution: targeting_factor,
            weight: 0.20,
            description: Some("Based on sector targeting".to_string()),
        });
        score += targeting_factor * 0.20;
        weight_sum += 0.20;

        // Recency factor
        let recency_factor = if let Some(last) = self.last_activity {
            let days_since = (Utc::now().date_naive() - last).num_days();
            if days_since < 30 {
                0.15
            } else if days_since < 180 {
                0.10
            } else if days_since < 365 {
                0.05
            } else {
                0.0
            }
        } else {
            0.05
        };
        factors.push(crate::models::RiskFactor {
            name: "recency".to_string(),
            contribution: recency_factor,
            weight: 0.15,
            description: Some("Based on last observed activity".to_string()),
        });
        score += recency_factor * 0.15;
        weight_sum += 0.15;

        // Motivation severity
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

        // Normalize to 0-1
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

    /// Check if this actor targets a specific sector.
    pub fn targets_sector(&self, sector: &IndustrySector) -> bool {
        self.target_sectors.iter().any(|s| s == sector)
    }

    /// Check if this actor has used a specific technique.
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

    /// Duration in days if campaign is complete.
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

/// TTP (Tactics, Techniques, Procedures) specification.
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
    /// Create a new empty threat actor database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create database with pre-populated known threat actors.
    pub fn with_known_actors() -> Self {
        let mut db = Self::default();
        db.populate_known_actors();
        db
    }

    /// Add a threat actor to the database.
    pub fn add_actor(&mut self, actor: ThreatActor) -> Result<Uuid> {
        // Check for duplicate alias
        if self.actors_by_alias.contains_key(&actor.alias) {
            return Err(ThreatIntelError::data_integrity(format!(
                "Threat actor with alias '{}' already exists",
                actor.alias
            )));
        }

        let id = actor.id;
        self.actors.insert(id, actor.clone());
        self.actors_by_alias.insert(actor.alias.clone(), id);

        // Index by sector
        for sector in &actor.target_sectors {
            self.actors_by_sector
                .entry(sector.clone())
                .or_default()
                .push(id);
        }

        Ok(id)
    }

    /// Get a threat actor by ID.
    pub fn get_actor(&self, id: Uuid) -> Option<&ThreatActor> {
        self.actors.get(&id)
    }

    /// Get a threat actor by alias.
    pub fn get_actor_by_alias(&self, alias: &str) -> Option<&ThreatActor> {
        self.actors_by_alias
            .get(alias)
            .and_then(|id| self.actors.get(id))
    }

    /// Get all actors targeting a specific sector.
    pub fn get_actors_by_sector(&self, sector: &IndustrySector) -> Vec<&ThreatActor> {
        self.actors_by_sector
            .get(sector)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.actors.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get actors by status.
    pub fn get_actors_by_status(&self, status: ActorStatus) -> Vec<&ThreatActor> {
        self.actors
            .values()
            .filter(|a| a.status == status)
            .collect()
    }

    /// Get actors by motivation.
    pub fn get_actors_by_motivation(&self, motivation: ActorMotivation) -> Vec<&ThreatActor> {
        self.actors
            .values()
            .filter(|a| a.motivation == motivation)
            .collect()
    }

    /// Search actors by alias pattern.
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

    /// Get all actors with pagination.
    pub fn list_actors(&self, params: PaginationParams) -> PaginatedResponse<ThreatActor> {
        let total = self.actors.len() as u64;
        let mut actors: Vec<_> = self.actors.values().cloned().collect();
        actors.sort_by_key(|b| std::cmp::Reverse(b.updated_at));

        let offset = params.offset as usize;
        let limit = params.limit as usize;
        let items: Vec<_> = actors.into_iter().skip(offset).take(limit).collect();

        PaginatedResponse::new(items, total, params.offset, params.limit)
    }

    /// Add an attack pattern.
    pub fn add_pattern(&mut self, pattern: AttackPattern) {
        self.patterns.insert(pattern.id.clone(), pattern);
    }

    /// Get attack pattern by ID.
    pub fn get_pattern(&self, id: &str) -> Option<&AttackPattern> {
        self.patterns.get(id)
    }

    /// Get patterns applicable to a sector.
    pub fn get_patterns_by_sector(&self, sector: &IndustrySector) -> Vec<&AttackPattern> {
        self.patterns
            .values()
            .filter(|p| p.applicable_sectors.contains(sector))
            .collect()
    }

    /// Add a TTP.
    pub fn add_ttp(&mut self, ttp: TTP) -> Uuid {
        let id = ttp.id;
        self.ttps.insert(id, ttp);
        id
    }

    /// Get TTP by ID.
    pub fn get_ttp(&self, id: Uuid) -> Option<&TTP> {
        self.ttps.get(&id)
    }

    /// Get TTPs by tactic.
    pub fn get_ttps_by_tactic(&self, tactic: AttackTactic) -> Vec<&TTP> {
        self.ttps
            .values()
            .filter(|t| t.tactic == tactic)
            .collect()
    }

    /// Add an indicator.
    pub fn add_indicator(&mut self, indicator: Indicator) -> Uuid {
        let id = indicator.id;
        self.indicators.insert(id, indicator);
        id
    }

    /// Search indicators by value pattern.
    pub fn search_indicators(&self, pattern: &str) -> Vec<&Indicator> {
        let pattern_lower = pattern.to_lowercase();
        self.indicators
            .values()
            .filter(|i| i.value.to_lowercase().contains(&pattern_lower))
            .collect()
    }

    /// Get indicators by type.
    pub fn get_indicators_by_type(&self, indicator_type: IndicatorType) -> Vec<&Indicator> {
        self.indicators
            .values()
            .filter(|i| i.indicator_type == indicator_type)
            .collect()
    }

    /// Find actors using a specific technique.
    pub fn find_actors_by_technique(&self, technique_id: &str) -> Vec<&ThreatActor> {
        self.actors
            .values()
            .filter(|a| a.has_technique(technique_id))
            .collect()
    }

    /// Get sector-specific threat summary.
    pub fn get_sector_threat_summary(&self, sector: &IndustrySector) -> SectorThreatSummary {
        let actors = self.get_actors_by_sector(sector);
        let active_count = actors.iter().filter(|a| a.status == ActorStatus::Active).count();
        let patterns = self.get_patterns_by_sector(sector);

        let techniques: Vec<_> = actors
            .iter()
            .flat_map(|a| a.techniques.iter().map(|t| t.technique_id.clone()))
            .collect();
        let mut unique_techniques = techniques.clone();
        unique_techniques.sort();
        unique_techniques.dedup();

        SectorThreatSummary {
            sector: sector.clone(),
            total_actors: actors.len(),
            active_actors: active_count,
            attack_patterns: patterns.len(),
            unique_techniques: unique_techniques.len(),
            top_actors: actors
                .iter()
                .take(5)
                .map(|a| a.alias.clone())
                .collect(),
            most_used_techniques: techniques,
            overall_risk_score: if actors.is_empty() {
                0.0
            } else {
                actors.iter().map(|a| a.calculate_risk_score().score).sum::<f64>() / actors.len() as f64
            },
        }
    }

    /// Populate with known threat actors.
    #[allow(clippy::disallowed_methods, unused_must_use)]
    fn populate_known_actors(&mut self) {
        // APT Groups (Commonly referenced)
        self.add_actor(
            ThreatActor::new("APT1", ActorMotivation::Espionage, ActorStatus::Defunct)
                .with_name("Comment Crew")
                .with_aliases(vec!["Comment Crew".to_string(), "Shanghai Raiders".to_string()])
                .with_attributed_country("CN")
                .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Aerospace])
                .with_target_regions(vec!["US".to_string(), "EU".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2006, 1, 1).unwrap())
                .with_sophistication(6)
        );

        self.add_actor(
            ThreatActor::new("APT29", ActorMotivation::Espionage, ActorStatus::Active)
                .with_name("Cozy Bear")
                .with_aliases(vec!["Cozy Bear".to_string(), "The Dukes".to_string(), "NOBELIUM".to_string()])
                .with_attributed_country("RU")
                .with_target_sectors(vec![IndustrySector::Government, IndustrySector::Healthcare, IndustrySector::Pharmaceuticals])
                .with_target_regions(vec!["US".to_string(), "EU".to_string(), "UK".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2008, 1, 1).unwrap())
                .with_sophistication(9)
        );

        self.add_actor(
            ThreatActor::new("APT41", ActorMotivation::Financial, ActorStatus::Active)
                .with_aliases(vec!["WICKED PANDA".to_string(), "WICKED SPIDER".to_string()])
                .with_attributed_country("CN")
                .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Healthcare, IndustrySector::Gaming])
                .with_target_regions(vec!["US".to_string(), "APAC".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2012, 1, 1).unwrap())
                .with_sophistication(8)
        );

        // FIN Groups (Financial)
        self.add_actor(
            ThreatActor::new("FIN7", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("Carbanak")
                .with_aliases(vec!["Carbanak".to_string(), "Carbon Spider".to_string(), "Navy Beetle".to_string()])
                .with_attributed_country("Unknown")
                .with_target_sectors(vec![IndustrySector::Financial, IndustrySector::Retail])
                .with_target_regions(vec!["US".to_string(), "EU".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2013, 1, 1).unwrap())
                .with_sophistication(7)
        );

        self.add_actor(
            ThreatActor::new("FIN12", ActorMotivation::Financial, ActorStatus::Active)
                .with_attributed_country("Unknown")
                .with_target_sectors(vec![IndustrySector::Healthcare, IndustrySector::Technology])
                .with_target_regions(vec!["US".to_string(), "CA".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2018, 1, 1).unwrap())
                .with_sophistication(7)
        );

        // Ransomware Groups
        self.add_actor(
            ThreatActor::new("RANSOMWARE-001", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("LockBit")
                .with_aliases(vec!["LockBit".to_string(), "LockBit 2.0".to_string(), "LockBit 3.0".to_string()])
                .with_attributed_country("RU")
                .with_target_sectors(vec![IndustrySector::Manufacturing, IndustrySector::Technology, IndustrySector::Healthcare])
                .with_target_regions(vec!["Global".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2019, 9, 1).unwrap())
                .with_sophistication(8)
        );

        self.add_actor(
            ThreatActor::new("RANSOMWARE-002", ActorMotivation::Financial, ActorStatus::Active)
                .with_name("Clop")
                .with_aliases(vec!["Clop".to_string(), "ClOp".to_string()])
                .with_attributed_country("RU")
                .with_target_sectors(vec![IndustrySector::Manufacturing, IndustrySector::Aerospace])
                .with_target_regions(vec!["Global".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2019, 2, 1).unwrap())
                .with_sophistication(8)
        );

        // Supply Chain Focus Groups
        self.add_actor(
            ThreatActor::new("APT10", ActorMotivation::Espionage, ActorStatus::Active)
                .with_name("Stone Panda")
                .with_aliases(vec!["Stone Panda".to_string(), "MenuPass".to_string(), "Red Apollo".to_string()])
                .with_attributed_country("CN")
                .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Aerospace, IndustrySector::Defense])
                .with_target_regions(vec!["US".to_string(), "EU".to_string(), "JP".to_string()])
                .with_first_activity(NaiveDate::from_ymd_opt(2009, 1, 1).unwrap())
                .with_sophistication(8)
        );

        // Add known attack patterns
        self.add_pattern(AttackPattern::new("PAT-001", "Supply Chain Compromise")
            .with_description("Compromising software dependencies, update mechanisms, or hardware components")
            .with_mitre_id("T1195")
            .with_mitigation(vec![
                "Implement code signing verification".to_string(),
                "Maintain SBOM inventory".to_string(),
                "Verify dependency integrity".to_string(),
            ])
        );

        self.add_pattern(AttackPattern::new("PAT-002", "Phishing with Spearphishing Attachment")
            .with_description("Targeted phishing with malicious attachments")
            .with_mitre_id("T1566.001")
            .with_mitigation(vec![
                "Email filtering".to_string(),
                "User awareness training".to_string(),
                "Attachment sandboxing".to_string(),
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
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_aliases(mut self, aliases: Vec<String>) -> Self {
        self.aliases = aliases;
        self
    }

    pub fn with_attributed_country(mut self, country: impl Into<String>) -> Self {
        self.attributed_country = Some(country.into());
        self
    }

    pub fn with_target_sectors(mut self, sectors: Vec<IndustrySector>) -> Self {
        self.target_sectors = sectors;
        self
    }

    pub fn with_target_regions(mut self, regions: Vec<String>) -> Self {
        self.target_regions = regions;
        self
    }

    pub fn with_first_activity(mut self, date: NaiveDate) -> Self {
        self.first_activity = Some(date);
        self
    }

    pub fn with_last_activity(mut self, date: NaiveDate) -> Self {
        self.last_activity = Some(date);
        self
    }

    pub fn with_sophistication(mut self, level: u8) -> Self {
        self.sophistication_level = level.min(10);
        self
    }
}

impl AttackPattern {
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_mitre_id(mut self, id: impl Into<String>) -> Self {
        self.mitre_id = Some(id.into());
        self
    }

    pub fn with_mitigation(mut self, mitigations: Vec<String>) -> Self {
        self.mitigation_strategies = mitigations;
        self
    }

    pub fn with_sectors(mut self, sectors: Vec<IndustrySector>) -> Self {
        self.applicable_sectors = sectors;
        self
    }
}

// Additional builder for Campaign
impl Campaign {
    pub fn with_end_date(mut self, date: NaiveDate) -> Self {
        self.end_date = Some(date);
        self
    }
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
            .with_aliases(vec!["Test".to_string(), "Fake".to_string()])
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
    fn test_search_actors() {
        let mut db = ThreatActorDatabase::new();
        let _ = db.add_actor(ThreatActor::new("APT1", ActorMotivation::Espionage, ActorStatus::Active)
            .with_name("First Group"));
        let _ = db.add_actor(ThreatActor::new("APT2", ActorMotivation::Espionage, ActorStatus::Active)
            .with_name("Second Group"));
        let _ = db.add_actor(ThreatActor::new("FIN1", ActorMotivation::Financial, ActorStatus::Active));

        let results = db.search_actors("APT");
        assert_eq!(results.len(), 2);
        
        let results = db.search_actors("First");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_sector_targeting() {
        let actor = ThreatActor::new("TEST", ActorMotivation::Financial, ActorStatus::Active)
            .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Healthcare]);

        assert!(actor.targets_sector(&IndustrySector::Technology));
        assert!(actor.targets_sector(&IndustrySector::Healthcare));
        assert!(!actor.targets_sector(&IndustrySector::Automotive));
    }

    #[test]
    fn test_risk_score_calculation() {
        let actor = ThreatActor::new("TEST", ActorMotivation::Espionage, ActorStatus::Active)
            .with_target_sectors(vec![
                IndustrySector::Technology,
                IndustrySector::Defense,
                IndustrySector::Aerospace,
                IndustrySector::Electronics,
                IndustrySector::Telecommunications,
            ])
            .with_sophistication(9)
            .with_first_activity(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap())
            .with_last_activity(Utc::now().date_naive());

        let score = actor.calculate_risk_score();
        assert!(score.score > 0.5); // Active, high soph, multiple sectors, recent
    }

    #[test]
    fn test_campaign_duration() {
        let campaign = Campaign::new("Test Campaign", NaiveDate::from_ymd_opt(2023, 1, 1).unwrap())
            .with_end_date(NaiveDate::from_ymd_opt(2023, 6, 1).unwrap());

        assert_eq!(campaign.duration_days(), Some(151));
    }

    #[test]
    fn test_indicator_creation() {
        let indicator = Indicator::new(IndicatorType::Domain, "malicious.example.com");
        assert_eq!(indicator.value, "malicious.example.com");
    }

    #[test]
    fn test_pagination() {
        let db = ThreatActorDatabase::new();
        let response = db.list_actors(PaginationParams::new(0, 10));
        assert_eq!(response.total, 0);
        assert!(!response.has_more);
    }
}
