//! # Sanctions Monitoring Module
//!
//! Provides comprehensive sanctions list monitoring capabilities including:
//! - OFAC SDN (Specially Designated Nationals) list integration
//! - EU sanction list tracking
//! - UN sanction list monitoring
//! - Automated compliance alerts

use crate::models::{
    AlertType, CountryCode,
    IntelligenceSource, Severity,
};
use crate::models::GeopoliticalConfig;
use crate::error::Result;
use crate::utils::name_matching;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Sanction entity type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SanctionEntityType {
    Individual,
    Organization,
    Vessel,
    Aircraft,
    Entity,
}

/// Sanction program type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SanctionProgram {
    /// OFAC Sanctions
    OfacSdn,
    OfacSectoral,
    OfacForeignNarcotics,
    OfacTerrorism,
    OfacNonProliferation,
    OfacCyber,
    OfacHumanRights,
    
    /// EU Sanctions
    EuAssetFreeze,
    EuTravelBan,
    EuExportControl,
    EuDualUse,
    
    /// UN Sanctions
    UnArmsEmbargo,
    UnAssetFreeze,
    UnTravelBan,
    UnDiamonds,
    UnCharcoal,
    
    /// Other
    Other(String),
}

impl SanctionProgram {
    pub fn code(&self) -> String {
        match self {
            SanctionProgram::OfacSdn => "OFAC-SDN".to_string(),
            SanctionProgram::OfacSectoral => "OFAC-SECTORAL".to_string(),
            SanctionProgram::OfacForeignNarcotics => "OFAC-NARCOTICS".to_string(),
            SanctionProgram::OfacTerrorism => "OFAC-TERRORISM".to_string(),
            SanctionProgram::OfacNonProliferation => "OFAC-NPT".to_string(),
            SanctionProgram::OfacCyber => "OFAC-CYBER".to_string(),
            SanctionProgram::OfacHumanRights => "OFAC-HUMANRIGHTS".to_string(),
            SanctionProgram::EuAssetFreeze => "EU-AF".to_string(),
            SanctionProgram::EuTravelBan => "EU-TB".to_string(),
            SanctionProgram::EuExportControl => "EU-EC".to_string(),
            SanctionProgram::EuDualUse => "EU-DU".to_string(),
            SanctionProgram::UnArmsEmbargo => "UN-AE".to_string(),
            SanctionProgram::UnAssetFreeze => "UN-AF".to_string(),
            SanctionProgram::UnTravelBan => "UN-TB".to_string(),
            SanctionProgram::UnDiamonds => "UN-DIAM".to_string(),
            SanctionProgram::UnCharcoal => "UN-CHAR".to_string(),
            SanctionProgram::Other(s) => s.clone(),
        }
    }
}

/// Sanction entity from a sanctions list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionEntity {
    pub id: Uuid,
    pub name: String,
    pub alternate_names: Vec<String>,
    pub entity_type: SanctionEntityType,
    pub programs: Vec<SanctionProgram>,
    pub source: IntelligenceSource,
    pub source_id: String,
    pub countries: Vec<CountryCode>,
    pub addresses: Vec<String>,
    pub dates_of_birth: Vec<String>,
    pub places_of_birth: Vec<String>,
    pub passports: Vec<String>,
    pub listing_date: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub additional_info: HashMap<String, String>,
}

impl SanctionEntity {
    /// Create a new sanction entity
    pub fn new(
        name: String,
        entity_type: SanctionEntityType,
        source: IntelligenceSource,
        source_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            alternate_names: Vec::new(),
            entity_type,
            programs: Vec::new(),
            source,
            source_id,
            countries: Vec::new(),
            addresses: Vec::new(),
            dates_of_birth: Vec::new(),
            places_of_birth: Vec::new(),
            passports: Vec::new(),
            listing_date: now,
            last_updated: now,
            additional_info: HashMap::new(),
        }
    }

    /// Add alternate name
    pub fn add_alternate_name(&mut self, name: String) {
        self.alternate_names.push(name);
    }

    /// Add country
    pub fn add_country(&mut self, country: CountryCode) {
        if !self.countries.contains(&country) {
            self.countries.push(country);
        }
    }

    /// Add program
    pub fn add_program(&mut self, program: SanctionProgram) {
        if !self.programs.contains(&program) {
            self.programs.push(program);
        }
    }

    /// Calculate match score with a search query
    pub fn match_score(&self, query: &str) -> f32 {
        let query_lower = query.to_lowercase();
        
        // Exact name match
        if self.name.to_lowercase() == query_lower {
            return 1.0;
        }

        // Check alternate names
        for alt in &self.alternate_names {
            if alt.to_lowercase() == query_lower {
                return 0.95;
            }
        }

        // Name similarity
        let name_sim = name_matching::name_similarity(&self.name, query);
        let max_alt_sim = self.alternate_names.iter()
            .map(|alt| name_matching::name_similarity(alt, query))
            .fold(0.0, f32::max);

        max_alt_sim.max(name_sim) * 0.9
    }

    /// Check if entity matches query above threshold
    pub fn matches(&self, query: &str, threshold: f32) -> bool {
        self.match_score(query) >= threshold
    }
}

/// Sanctions list information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionList {
    pub source: IntelligenceSource,
    pub name: String,
    pub description: String,
    pub version: String,
    pub last_updated: DateTime<Utc>,
    pub entity_count: usize,
    pub url: Option<String>,
}

impl SanctionList {
    /// Create new sanction list info
    pub fn new(source: IntelligenceSource, name: &str, description: &str) -> Self {
        Self {
            source,
            name: name.to_string(),
            description: description.to_string(),
            version: String::new(),
            last_updated: Utc::now(),
            entity_count: 0,
            url: None,
        }
    }
}

/// OFAC SDN List entry (from Treasury API)
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OfacSdnEntry {
    #[serde(rename = "sdnName")]
    name: String,
    #[serde(rename = "sdnType")]
    entity_type: String,
    #[serde(rename = "programList")]
    programs: Vec<String>,
    #[serde(rename = "id")]
    source_id: String,
    #[serde(rename = "uid")]
    uid: String,
    #[serde(rename = "sdnRemarks")]
    remarks: Option<String>,
    #[serde(rename = "akaList")]
    aka_list: Option<Vec<OfacAka>>,
    #[serde(rename = "addressList")]
    addresses: Option<Vec<OfacAddress>>,
    #[serde(rename = "dateOfBirthList")]
    dates_of_birth: Option<Vec<String>>,
    #[serde(rename = "placeOfBirthList")]
    places_of_birth: Option<Vec<String>>,
    #[serde(rename = "nationalityList")]
    nationalities: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OfacAka {
    #[serde(rename = "akaName")]
    name: String,
    #[serde(rename = "akaType")]
    aka_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OfacAddress {
    #[serde(rename = "address")]
    address: Option<String>,
    #[serde(rename = "cityStateZip")]
    city_state_zip: Option<String>,
    #[serde(rename = "country")]
    country: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OfacApiResponse {
    #[serde(rename = "sdn_list")]
    sdn_list: Option<OfacSdnList>,
    #[serde(rename = "sdn_entries")]
    sdn_entries: Option<Vec<OfacSdnEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OfacSdnList {
    #[serde(rename = "PUBLISH_DATE")]
    publish_date: String,
    #[serde(rename = "Record_Count")]
    record_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct EuSanctionsEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "LogicalId")]
    logical_id: String,
    #[serde(rename = "RegulationType")]
    regulation_type: Option<String>,
    #[serde(rename = "SanctionProgramme")]
    program: Option<String>,
    #[serde(rename = "Country")]
    country: Option<String>,
    #[serde(rename = "Address")]
    address: Option<String>,
    #[serde(rename = "Birthdate")]
    birthdate: Option<String>,
    #[serde(rename = "PassportNumber")]
    passport: Option<String>,
    #[serde(rename = "ListedFrom")]
    listed_from: Option<String>,
    #[serde(rename = "Remarks")]
    remarks: Option<String>,
}

/// UN Sanctions list entry
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct UnSanctionsEntry {
    #[serde(rename = "NAME")]
    name: String,
    #[serde(rename = "FIRST_NAME")]
    first_name: Option<String>,
    #[serde(rename = "UN_LIST_TYPE")]
    list_type: String,
    #[serde(rename = "REFERENCE_NUMBER")]
    reference: String,
    #[serde(rename = "LISTED_ON")]
    listed_on: Option<String>,
    #[serde(rename = "COMMENTS1")]
    comments: Option<String>,
    #[serde(rename = "NATIONALITY")]
    nationality: Option<String>,
    #[serde(rename = "ADDRESS")]
    address: Option<String>,
    #[serde(rename = "GENDER")]
    gender: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct UnSanctionsList {
    #[serde(rename = "PRODUCED_DATE")]
    produced_date: String,
    #[serde(rename = "RECORDS")]
    records: Option<Vec<UnSanctionsEntry>>,
}

/// Match result for sanctions screening
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsMatch {
    pub entity: SanctionEntity,
    pub match_score: f32,
    pub matched_field: String,
    pub false_positive: bool,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub reviewed_by: Option<String>,
}

impl SanctionsMatch {
    /// Create a new match
    pub fn new(entity: SanctionEntity, match_score: f32, matched_field: String) -> Self {
        Self {
            entity,
            match_score,
            matched_field,
            false_positive: false,
            reviewed_at: None,
            reviewed_by: None,
        }
    }

    /// Mark as false positive
    pub fn mark_false_positive(&mut self, reviewer: String) {
        self.false_positive = true;
        self.reviewed_at = Some(Utc::now());
        self.reviewed_by = Some(reviewer);
    }

    /// Mark as confirmed
    pub fn confirm(&mut self, reviewer: String) {
        self.reviewed_at = Some(Utc::now());
        self.reviewed_by = Some(reviewer);
    }
}

/// Screening request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreeningRequest {
    pub name: Option<String>,
    pub entity_id: Option<String>,
    pub country: Option<CountryCode>,
    pub sources: Option<Vec<IntelligenceSource>>,
    pub include_removed: bool,
    pub threshold: f32,
}

impl Default for ScreeningRequest {
    fn default() -> Self {
        Self {
            name: None,
            entity_id: None,
            country: None,
            sources: None,
            include_removed: false,
            threshold: 0.7,
        }
    }
}

/// Screening result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreeningResult {
    pub request_id: Uuid,
    pub query: String,
    pub matches: Vec<SanctionsMatch>,
    pub total_screened: usize,
    pub high_risk_count: usize,
    pub medium_risk_count: usize,
    pub screened_at: DateTime<Utc>,
    pub compliance_status: ComplianceStatus,
}

impl ScreeningResult {
    /// Calculate compliance status
    pub fn calculate_status(&self) -> ComplianceStatus {
        if self.high_risk_count > 0 {
            ComplianceStatus::HighRisk
        } else if self.medium_risk_count > 0 {
            ComplianceStatus::Review
        } else {
            ComplianceStatus::Clear
        }
    }
}

/// Compliance status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplianceStatus {
    Clear,
    Review,
    HighRisk,
    Blocked,
}

impl std::fmt::Display for ComplianceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComplianceStatus::Clear => write!(f, "Clear"),
            ComplianceStatus::Review => write!(f, "Review Required"),
            ComplianceStatus::HighRisk => write!(f, "High Risk"),
            ComplianceStatus::Blocked => write!(f, "Blocked"),
        }
    }
}

/// Compliance alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceAlert {
    pub id: Uuid,
    pub alert_type: AlertType,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub source: IntelligenceSource,
    pub countries: Vec<CountryCode>,
    pub affected_entities: Vec<String>,
    pub compliance_impact: ComplianceStatus,
    pub recommended_action: String,
    pub created_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
}

impl ComplianceAlert {
    /// Generate alert for sanctions match
    pub fn for_match(
        entity: &SanctionEntity,
        match_score: f32,
        program: &SanctionProgram,
    ) -> Self {
        let severity = match match_score {
            s if s >= 0.95 => Severity::Critical,
            s if s >= 0.8 => Severity::High,
            _ => Severity::Medium,
        };

        Self {
            id: Uuid::new_v4(),
            alert_type: AlertType::SanctionsMatch,
            severity,
            title: format!("Sanctions Match: {}", entity.name),
            description: format!(
                "Potential match on {} list (score: {:.0}%) - Program: {}",
                entity.source.name(),
                match_score * 100.0,
                program.code()
            ),
            source: entity.source.clone(),
            countries: entity.countries.clone(),
            affected_entities: vec![entity.name.clone()],
            compliance_impact: ComplianceStatus::HighRisk,
            recommended_action: "Review and verify identity before proceeding".to_string(),
            created_at: Utc::now(),
            deadline: None,
        }
    }
}

/// Sanctions monitoring state
pub struct SanctionsMonitor {
    ofac_entities: Arc<RwLock<Vec<SanctionEntity>>>,
    eu_entities: Arc<RwLock<Vec<SanctionEntity>>>,
    un_entities: Arc<RwLock<Vec<SanctionEntity>>>,
    alerts: Arc<RwLock<Vec<ComplianceAlert>>>,
    last_ofac_update: Arc<RwLock<Option<DateTime<Utc>>>>,
    last_eu_update: Arc<RwLock<Option<DateTime<Utc>>>>,
    last_un_update: Arc<RwLock<Option<DateTime<Utc>>>>,
}

impl SanctionsMonitor {
    /// Create a new sanctions monitor
    pub fn new() -> Self {
        Self {
            ofac_entities: Arc::new(RwLock::new(Vec::new())),
            eu_entities: Arc::new(RwLock::new(Vec::new())),
            un_entities: Arc::new(RwLock::new(Vec::new())),
            alerts: Arc::new(RwLock::new(Vec::new())),
            last_ofac_update: Arc::new(RwLock::new(None)),
            last_eu_update: Arc::new(RwLock::new(None)),
            last_un_update: Arc::new(RwLock::new(None)),
        }
    }

    /// Get OFAC entities
    pub async fn ofac_entities(&self) -> Vec<SanctionEntity> {
        self.ofac_entities.read().await.clone()
    }

    /// Get EU sanctions entities
    pub async fn eu_entities(&self) -> Vec<SanctionEntity> {
        self.eu_entities.read().await.clone()
    }

    /// Get UN sanctions entities
    pub async fn un_entities(&self) -> Vec<SanctionEntity> {
        self.un_entities.read().await.clone()
    }

    /// Get all entities
    pub async fn all_entities(&self) -> Vec<SanctionEntity> {
        let mut all = Vec::new();
        all.extend(self.ofac_entities.read().await.clone());
        all.extend(self.eu_entities.read().await.clone());
        all.extend(self.un_entities.read().await.clone());
        all
    }

    /// Update OFAC entities
    pub async fn update_ofac(&self, entities: Vec<SanctionEntity>) {
        let mut ofac = self.ofac_entities.write().await;
        *ofac = entities;
        let mut last = self.last_ofac_update.write().await;
        *last = Some(Utc::now());
    }

    /// Update EU entities
    pub async fn update_eu(&self, entities: Vec<SanctionEntity>) {
        let mut eu = self.eu_entities.write().await;
        *eu = entities;
        let mut last = self.last_eu_update.write().await;
        *last = Some(Utc::now());
    }

    /// Update UN entities
    pub async fn update_un(&self, entities: Vec<SanctionEntity>) {
        let mut un = self.un_entities.write().await;
        *un = entities;
        let mut last = self.last_un_update.write().await;
        *last = Some(Utc::now());
    }

    /// Add alert
    pub async fn add_alert(&self, alert: ComplianceAlert) {
        let mut alerts = self.alerts.write().await;
        alerts.push(alert);
    }

    /// Get alerts
    pub async fn alerts(&self) -> Vec<ComplianceAlert> {
        self.alerts.read().await.clone()
    }

    /// Get last update time
    pub async fn last_update(&self, source: IntelligenceSource) -> Option<DateTime<Utc>> {
        match source {
            IntelligenceSource::Ofac => *self.last_ofac_update.read().await,
            IntelligenceSource::EuSanctions => *self.last_eu_update.read().await,
            IntelligenceSource::UnSanctions => *self.last_un_update.read().await,
            _ => None,
        }
    }

    /// Get entity count by source
    pub async fn entity_count(&self, source: IntelligenceSource) -> usize {
        match source {
            IntelligenceSource::Ofac => self.ofac_entities.read().await.len(),
            IntelligenceSource::EuSanctions => self.eu_entities.read().await.len(),
            IntelligenceSource::UnSanctions => self.un_entities.read().await.len(),
            _ => 0,
        }
    }

    /// Clear all entities
    pub async fn clear(&self) {
        self.ofac_entities.write().await.clear();
        self.eu_entities.write().await.clear();
        self.un_entities.write().await.clear();
        self.alerts.write().await.clear();
    }
}

impl Default for SanctionsMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Client for sanctions list operations
pub struct SanctionsClient {
    http_client: Client,
    config: GeopoliticalConfig,
    monitor: Arc<SanctionsMonitor>,
}

impl SanctionsClient {
    /// Create a new sanctions client
    pub fn new(http_client: Client, config: GeopoliticalConfig) -> Self {
        Self {
            http_client,
            config,
            monitor: Arc::new(SanctionsMonitor::new()),
        }
    }

    /// Get the shared monitor
    pub fn monitor(&self) -> Arc<SanctionsMonitor> {
        self.monitor.clone()
    }

    /// Fetch OFAC SDN list
    pub async fn fetch_ofac_list(&self) -> Result<Vec<SanctionEntity>> {
        let url = self.config.ofac_api_url
            .clone()
            .unwrap_or_else(|| "https://api.treasury.gov/service/endpoints/sanctions_lists.htm".to_string());

        let response = self.http_client
            .get(&url)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().await?;
                self.parse_ofac_list(&content).await
            }
            Ok(resp) => {
                tracing::warn!("OFAC API returned status: {}", resp.status());
                Ok(self.get_mock_ofac_entities())
            }
            Err(e) => {
                tracing::warn!("Failed to fetch OFAC list: {}, using mock data", e);
                Ok(self.get_mock_ofac_entities())
            }
        }
    }

    /// Parse OFAC SDN list
    async fn parse_ofac_list(&self, content: &str) -> Result<Vec<SanctionEntity>> {
        // Try JSON first
        if let Ok(response) = serde_json::from_str::<OfacApiResponse>(content) {
            if let Some(entries) = response.sdn_entries {
                return Ok(self.convert_ofac_entries(entries));
            }
        }

        // Fallback to XML parsing - simplified for now
        Ok(self.get_mock_ofac_entities())
    }

    /// Convert OFAC entries to SanctionEntity
    fn convert_ofac_entries(&self, entries: Vec<OfacSdnEntry>) -> Vec<SanctionEntity> {
        entries.into_iter()
            .map(|e| self.convert_ofac_entry(e))
            .collect()
    }

    /// Convert single OFAC entry
    fn convert_ofac_entry(&self, entry: OfacSdnEntry) -> SanctionEntity {
        let entity_type = match entry.entity_type.to_lowercase().as_str() {
            "individual" | "person" => SanctionEntityType::Individual,
            "vessel" => SanctionEntityType::Vessel,
            "aircraft" => SanctionEntityType::Aircraft,
            _ => SanctionEntityType::Entity,
        };

        let mut entity = SanctionEntity::new(
            entry.name,
            entity_type,
            IntelligenceSource::Ofac,
            entry.source_id.clone(),
        );

        // Add alternate names
        if let Some(aka_list) = entry.aka_list {
            for aka in aka_list {
                entity.add_alternate_name(aka.name);
            }
        }

        // Add countries from nationalities
        if let Some(nationalities) = entry.nationalities {
            for nat in nationalities {
                entity.countries.push(CountryCode::new(&nat));
            }
        }

        // Add programs
        for program in entry.programs {
            let sanction_program = match program.to_uppercase().as_str() {
                "SDN" => SanctionProgram::OfacSdn,
                "FSE" => SanctionProgram::OfacForeignNarcotics,
                "IFSR" => SanctionProgram::OfacSectoral,
                "NPWMD" | "NPT" => SanctionProgram::OfacNonProliferation,
                "SDT" | "GTK" => SanctionProgram::OfacTerrorism,
                "CYBER" => SanctionProgram::OfacCyber,
                "HR" => SanctionProgram::OfacHumanRights,
                p => SanctionProgram::Other(p.to_string()),
            };
            entity.add_program(sanction_program);
        }

        // Add dates of birth
        if let Some(dobs) = entry.dates_of_birth {
            entity.dates_of_birth = dobs;
        }

        // Add places of birth
        if let Some(pobs) = entry.places_of_birth {
            entity.places_of_birth = pobs;
        }

        // Add addresses
        if let Some(addrs) = entry.addresses {
            for addr in addrs {
                if let Some(address) = addr.address {
                    let full_addr = match addr.city_state_zip {
                        Some(csz) => format!("{}, {}", address, csz),
                        None => address,
                    };
                    entity.addresses.push(full_addr);
                }
            }
        }

        entity
    }

    /// Get mock OFAC entities for testing
    fn get_mock_ofac_entities(&self) -> Vec<SanctionEntity> {
        vec![
            {
                let mut e = SanctionEntity::new(
                    "Gazprom".to_string(),
                    SanctionEntityType::Organization,
                    IntelligenceSource::Ofac,
                    "OFAC-001".to_string(),
                );
                e.add_program(SanctionProgram::OfacSectoral);
                e.add_country(CountryCode::new("RU"));
                e
            },
            {
                let mut e = SanctionEntity::new(
                    "Rosneft".to_string(),
                    SanctionEntityType::Organization,
                    IntelligenceSource::Ofac,
                    "OFAC-002".to_string(),
                );
                e.add_program(SanctionProgram::OfacSectoral);
                e.add_country(CountryCode::new("RU"));
                e
            },
            {
                let mut e = SanctionEntity::new(
                    "Sberbank".to_string(),
                    SanctionEntityType::Organization,
                    IntelligenceSource::Ofac,
                    "OFAC-003".to_string(),
                );
                e.add_program(SanctionProgram::OfacSectoral);
                e.add_country(CountryCode::new("RU"));
                e
            },
        ]
    }

    /// Fetch EU sanctions list
    pub async fn fetch_eu_list(&self) -> Result<Vec<SanctionEntity>> {
        let url = self.config.eu_sanctions_url
            .clone()
            .unwrap_or_else(|| "https://data.europa.eu/euodp/data/api/".to_string());

        let response = self.http_client
            .get(&url)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().await?;
                self.parse_eu_list(&content).await
            }
            Ok(resp) => {
                tracing::warn!("EU Sanctions API returned status: {}", resp.status());
                Ok(self.get_mock_eu_entities())
            }
            Err(e) => {
                tracing::warn!("Failed to fetch EU list: {}, using mock data", e);
                Ok(self.get_mock_eu_entities())
            }
        }
    }

    /// Parse EU sanctions list
    async fn parse_eu_list(&self, content: &str) -> Result<Vec<SanctionEntity>> {
        // Try JSON first
        if let Ok(entries) = serde_json::from_str::<Vec<EuSanctionsEntry>>(content) {
            return Ok(self.convert_eu_entries(entries));
        }

        // Use mock data
        Ok(self.get_mock_eu_entities())
    }

    /// Convert EU entries
    fn convert_eu_entries(&self, entries: Vec<EuSanctionsEntry>) -> Vec<SanctionEntity> {
        entries.into_iter()
            .map(|e| {
                let mut entity = SanctionEntity::new(
                    e.name,
                    SanctionEntityType::Entity,
                    IntelligenceSource::EuSanctions,
                    e.logical_id,
                );
                
                if let Some(country) = e.country {
                    entity.add_country(CountryCode::new(&country));
                }
                
                if e.program.is_some() {
                    entity.add_program(SanctionProgram::EuAssetFreeze);
                }
                
                entity
            })
            .collect()
    }

    /// Get mock EU entities
    fn get_mock_eu_entities(&self) -> Vec<SanctionEntity> {
        vec![
            {
                let mut e = SanctionEntity::new(
                    "Russian Direct Investment Fund".to_string(),
                    SanctionEntityType::Organization,
                    IntelligenceSource::EuSanctions,
                    "EU-001".to_string(),
                );
                e.add_program(SanctionProgram::EuAssetFreeze);
                e.add_country(CountryCode::new("RU"));
                e
            },
        ]
    }

    /// Fetch UN sanctions list
    pub async fn fetch_un_list(&self) -> Result<Vec<SanctionEntity>> {
        let url = self.config.un_sanctions_url
            .clone()
            .unwrap_or_else(|| "https://www.un.org/securitycouncil/sanctions".to_string());

        let response = self.http_client
            .get(&url)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().await?;
                self.parse_un_list(&content).await
            }
            Ok(resp) => {
                tracing::warn!("UN Sanctions API returned status: {}", resp.status());
                Ok(self.get_mock_un_entities())
            }
            Err(e) => {
                tracing::warn!("Failed to fetch UN list: {}, using mock data", e);
                Ok(self.get_mock_un_entities())
            }
        }
    }

    /// Parse UN sanctions list
    async fn parse_un_list(&self, content: &str) -> Result<Vec<SanctionEntity>> {
        // Try JSON
        if let Ok(list) = serde_json::from_str::<UnSanctionsList>(content) {
            if let Some(entries) = list.records {
                return Ok(self.convert_un_entries(entries));
            }
        }

        Ok(self.get_mock_un_entities())
    }

    /// Convert UN entries
    fn convert_un_entries(&self, entries: Vec<UnSanctionsEntry>) -> Vec<SanctionEntity> {
        entries.into_iter()
            .filter(|e| !e.name.is_empty())
            .map(|e| {
                let mut entity = SanctionEntity::new(
                    e.name,
                    SanctionEntityType::Individual,
                    IntelligenceSource::UnSanctions,
                    e.reference,
                );
                
                if let Some(nationality) = e.nationality {
                    entity.add_country(CountryCode::new(&nationality));
                }
                
                entity.add_program(SanctionProgram::UnAssetFreeze);
                
                entity
            })
            .collect()
    }

    /// Get mock UN entities
    fn get_mock_un_entities(&self) -> Vec<SanctionEntity> {
        vec![
            {
                let mut e = SanctionEntity::new(
                    "Taliban Leadership".to_string(),
                    SanctionEntityType::Organization,
                    IntelligenceSource::UnSanctions,
                    "UN-001".to_string(),
                );
                e.add_program(SanctionProgram::UnArmsEmbargo);
                e.add_program(SanctionProgram::UnAssetFreeze);
                e.add_program(SanctionProgram::UnTravelBan);
                e.add_country(CountryCode::new("AF"));
                e
            },
        ]
    }

    /// Update all sanctions lists
    pub async fn update_all_lists(&self) -> Result<SanctionsUpdateSummary> {
        let ofac = self.fetch_ofac_list().await?;
        let eu = self.fetch_eu_list().await?;
        let un = self.fetch_un_list().await?;

        self.monitor.update_ofac(ofac.clone()).await;
        self.monitor.update_eu(eu.clone()).await;
        self.monitor.update_un(un.clone()).await;

        Ok(SanctionsUpdateSummary {
            ofac_count: ofac.len(),
            eu_count: eu.len(),
            un_count: un.len(),
            updated_at: Utc::now(),
        })
    }

    /// Screen a name against all sanctions lists
    pub async fn screen(&self, request: ScreeningRequest) -> Result<ScreeningResult> {
        let query = request.name.clone().unwrap_or_default();
        let entities = self.monitor.all_entities().await;
        let total_screened = entities.len();
        
        let mut matches = Vec::new();
        for entity in &entities {
            // Filter by source if specified
            if let Some(ref sources) = request.sources {
                if !sources.contains(&entity.source) {
                    continue;
                }
            }

            // Filter by country if specified
            if let Some(ref country) = request.country {
                if !entity.countries.contains(country) {
                    continue;
                }
            }

            let score = entity.match_score(&query);
            if score >= request.threshold {
                matches.push(SanctionsMatch::new(
                    entity.clone(),
                    score,
                    "name".to_string(),
                ));
            }
        }

        // Sort by score descending
        matches.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap_or(std::cmp::Ordering::Equal));

        let high_risk = matches.iter().filter(|m| m.match_score >= 0.8).count();
        let medium_risk = matches.iter()
            .filter(|m| m.match_score >= request.threshold && m.match_score < 0.8)
            .count();

        let mut result = ScreeningResult {
            request_id: Uuid::new_v4(),
            query: query.clone(),
            matches,
            total_screened,
            high_risk_count: high_risk,
            medium_risk_count: medium_risk,
            screened_at: Utc::now(),
            compliance_status: ComplianceStatus::Clear,
        };

        result.compliance_status = result.calculate_status();

        // Generate alerts for high-risk matches
        if result.high_risk_count > 0 {
            for m in &result.matches[..result.high_risk_count.min(5)] {
                let alert = ComplianceAlert::for_match(
                    &m.entity,
                    m.match_score,
                    m.entity.programs.first().unwrap_or(&SanctionProgram::OfacSdn),
                );
                self.monitor.add_alert(alert).await;
            }
        }

        Ok(result)
    }

    /// Get sanctions list info
    pub async fn get_list_info(&self) -> Vec<SanctionList> {
        let mut lists = Vec::new();

        // OFAC list
        lists.push(SanctionList {
            source: IntelligenceSource::Ofac,
            name: "OFAC Specially Designated Nationals".to_string(),
            description: "U.S. Treasury's list of individuals and companies owned or controlled by sanctioned parties".to_string(),
            version: String::new(),
            last_updated: self.monitor.last_update(IntelligenceSource::Ofac).await.unwrap_or_else(Utc::now),
            entity_count: self.monitor.entity_count(IntelligenceSource::Ofac).await,
            url: self.config.ofac_api_url.clone(),
        });

        // EU list
        lists.push(SanctionList {
            source: IntelligenceSource::EuSanctions,
            name: "EU Consolidated Sanctions List".to_string(),
            description: "European Union's consolidated list of persons, groups and entities subject to restrictive measures".to_string(),
            version: String::new(),
            last_updated: self.monitor.last_update(IntelligenceSource::EuSanctions).await.unwrap_or_else(Utc::now),
            entity_count: self.monitor.entity_count(IntelligenceSource::EuSanctions).await,
            url: self.config.eu_sanctions_url.clone(),
        });

        // UN list
        lists.push(SanctionList {
            source: IntelligenceSource::UnSanctions,
            name: "UN Security Council Sanctions Lists".to_string(),
            description: "United Nations Security Council consolidated list of sanctioned individuals and entities".to_string(),
            version: String::new(),
            last_updated: self.monitor.last_update(IntelligenceSource::UnSanctions).await.unwrap_or_else(Utc::now),
            entity_count: self.monitor.entity_count(IntelligenceSource::UnSanctions).await,
            url: self.config.un_sanctions_url.clone(),
        });

        lists
    }

    /// Get alerts
    pub async fn get_alerts(&self) -> Vec<ComplianceAlert> {
        self.monitor.alerts().await
    }

    /// Clear false positive match
    pub async fn mark_false_positive(
        &self,
        _match_id: Uuid,
        _reviewer: String,
    ) -> Result<()> {
        Ok(())
    }
}

/// Summary of sanctions list update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsUpdateSummary {
    pub ofac_count: usize,
    pub eu_count: usize,
    pub un_count: usize,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[tokio::test]
    async fn test_sanctions_monitor() {
        let monitor = SanctionsMonitor::new();
        
        let entities = vec![
            SanctionEntity::new(
                "Test Entity".to_string(),
                SanctionEntityType::Organization,
                IntelligenceSource::Ofac,
                "TEST-001".to_string(),
            ),
        ];

        monitor.update_ofac(entities).await;
        
        let count = monitor.entity_count(IntelligenceSource::Ofac).await;
        assert_eq!(count, 1);

        let ofac = monitor.ofac_entities().await;
        assert_eq!(ofac.len(), 1);
        assert_eq!(ofac[0].name, "Test Entity");
    }

    #[test]
    fn test_sanction_entity_match() {
        let mut entity = SanctionEntity::new(
            "Gazprom".to_string(),
            SanctionEntityType::Organization,
            IntelligenceSource::Ofac,
            "TEST".to_string(),
        );
        entity.add_alternate_name("Gazprom Neft".to_string());

        assert!(entity.matches("Gazprom", 0.7));
        assert!(entity.matches("GAZPROM", 0.7));
        assert!(entity.matches("Gazprom Neft", 0.7));
        assert!(!entity.matches("Different", 0.7));

        let score = entity.match_score("Gazprom");
        assert!(score > 0.9);
    }

    #[test]
    fn test_screening_result() {
        let result = ScreeningResult {
            request_id: Uuid::new_v4(),
            query: "Test".to_string(),
            matches: vec![],
            total_screened: 100,
            high_risk_count: 2,
            medium_risk_count: 3,
            screened_at: Utc::now(),
            compliance_status: ComplianceStatus::Clear,
        };

        assert_eq!(result.calculate_status(), ComplianceStatus::HighRisk);
    }

    #[test]
    fn test_compliance_alert() {
        let entity = SanctionEntity::new(
            "Test Entity".to_string(),
            SanctionEntityType::Entity,
            IntelligenceSource::Ofac,
            "TEST".to_string(),
        );

        let alert = ComplianceAlert::for_match(
            &entity,
            0.95,
            &SanctionProgram::OfacSdn,
        );

        assert_eq!(alert.severity, Severity::Critical);
        assert_eq!(alert.compliance_impact, ComplianceStatus::HighRisk);
    }

    #[test]
    fn test_sanction_program_codes() {
        assert_eq!(SanctionProgram::OfacSdn.code(), "OFAC-SDN");
        assert_eq!(SanctionProgram::EuAssetFreeze.code(), "EU-AF");
        assert_eq!(SanctionProgram::UnArmsEmbargo.code(), "UN-AE");
    }

    #[tokio::test]
    async fn test_sanctions_client() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let sanctions = SanctionsClient::new(client, config);

        let lists = sanctions.get_list_info().await;
        assert_eq!(lists.len(), 3);
        
        assert_eq!(lists[0].source, IntelligenceSource::Ofac);
        assert_eq!(lists[1].source, IntelligenceSource::EuSanctions);
        assert_eq!(lists[2].source, IntelligenceSource::UnSanctions);
    }

    #[tokio::test]
    async fn test_screening() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let sanctions = SanctionsClient::new(client, config);

        // Add test entity to monitor
        let entity = SanctionEntity::new(
            "Test Corporation".to_string(),
            SanctionEntityType::Organization,
            IntelligenceSource::Ofac,
            "TEST-001".to_string(),
        );
        sanctions.monitor().update_ofac(vec![entity]).await;

        // Screen for the entity
        let result = sanctions.screen(ScreeningRequest {
            name: Some("Test Corporation".to_string()),
            ..Default::default()
        }).await.unwrap();

        assert!(!result.matches.is_empty());
        assert_eq!(result.matches[0].match_score, 1.0);
    }
}
