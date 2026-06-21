//! # Supply Chain Threat Modeling Module
//!
//! 3.2.3: Comprehensive supply chain threat modeling including tier-n supplier risk scoring,
//! geographic concentration risk, single-manufacturer components, and disruption scenario planning.
//!
//! ## Features
//!
//! - Tier-n supplier risk scoring
//! - Geographic concentration risk analysis
//! - Single-manufacturer component identification
//! - Disruption scenario planning
//! - Supply chain resilience scoring

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::{ConfidenceLevel, GeoRegion};

/// Relationship between entities in the supply chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyRelationship {
    pub supplier_id: Uuid,
    pub component_id: Uuid,
    pub relationship_type: RelationshipType,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub contract_value: Option<i64>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelationshipType {
    Provides,
    Sources,
    Subcontracted,
    Licensed,
}

impl SupplyRelationship {
    pub fn new(supplier_id: Uuid, component_id: Uuid, relationship_type: RelationshipType) -> Self {
        Self {
            supplier_id,
            component_id,
            relationship_type,
            start_date: Some(Utc::now()),
            end_date: None,
            contract_value: None,
            is_active: true,
        }
    }
}

/// Supplier tier classification in the supply chain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SupplierTier {
    Tier0, // End customer
    Tier1, // Direct supplier
    Tier2, // Supplier of Tier 1
    Tier3, // Sub-subcontractor
    Tier4, // Raw material/component supplier
}

impl SupplierTier {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tier0 => "tier0",
            Self::Tier1 => "tier1",
            Self::Tier2 => "tier2",
            Self::Tier3 => "tier3",
            Self::Tier4 => "tier4",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Tier0,
            1 => Self::Tier1,
            2 => Self::Tier2,
            3 => Self::Tier3,
            4 => Self::Tier4,
            _ => Self::Tier4,
        }
    }
}

/// Risk score for a supplier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierRiskScore {
    pub supplier_id: Uuid,
    pub overall_score: f64,
    pub financial_risk: f64,
    pub operational_risk: f64,
    pub geopolitical_risk: f64,
    pub cyber_risk: f64,
    pub concentration_risk: f64,
    pub dependency_score: f64,
    pub confidence: ConfidenceLevel,
    pub risk_factors: Vec<RiskFactorDetail>,
    pub last_calculated: DateTime<Utc>,
}

impl SupplierRiskScore {
    pub fn new(supplier_id: Uuid) -> Self {
        Self {
            supplier_id,
            overall_score: 0.0,
            financial_risk: 0.0,
            operational_risk: 0.0,
            geopolitical_risk: 0.0,
            cyber_risk: 0.0,
            concentration_risk: 0.0,
            dependency_score: 0.0,
            confidence: ConfidenceLevel::Medium,
            risk_factors: Vec::new(),
            last_calculated: Utc::now(),
        }
    }

    /// Calculate weighted overall score.
    pub fn calculate_overall(&mut self) {
        self.overall_score = (
            self.financial_risk * 0.20
            + self.operational_risk * 0.20
            + self.geopolitical_risk * 0.25
            + self.cyber_risk * 0.15
            + self.concentration_risk * 0.10
            + self.dependency_score * 0.10
        ).min(1.0);
    }
}

/// Detailed risk factor for a supplier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactorDetail {
    pub factor_name: String,
    pub score: f64,
    pub weight: f64,
    pub description: String,
    pub evidence: Vec<String>,
}

/// Supplier entity in the supply chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Supplier {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub country_code: String,
    pub region: Option<GeoRegion>,
    pub tier: SupplierTier,
    pub category: String,
    pub capabilities: Vec<String>,
    pub certifications: Vec<String>,
    pub annual_revenue_estimate: Option<i64>,
    pub employee_count: Option<i32>,
    pub financial_health_score: Option<f64>,
    pub criticality_score: f64,
    pub substitutability: SubstitutabilityLevel,
    pub capacity: SupplierCapacity,
    pub contacts: Vec<SupplierContact>,
    pub risk_score: Option<SupplierRiskScore>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Supplier {
    pub fn new(name: impl Into<String>, country_code: impl Into<String>, tier: SupplierTier, capacity: SupplierCapacity) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            legal_name: None,
            country_code: country_code.into().to_uppercase(),
            region: None,
            tier,
            category: String::new(),
            capabilities: Vec::new(),
            certifications: Vec::new(),
            annual_revenue_estimate: None,
            employee_count: None,
            financial_health_score: None,
            criticality_score: 0.5,
            substitutability: SubstitutabilityLevel::Medium,
            capacity,
            contacts: Vec::new(),
            risk_score: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubstitutabilityLevel {
    /// Multiple alternatives available
    High,
    /// Limited alternatives
    Medium,
    /// Few or single source
    Low,
    /// Single point of failure
    SingleSource,
}

impl SubstitutabilityLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::SingleSource => "single_source",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierCapacity {
    pub current_utilization: f64,
    pub max_capacity: f64,
    pub lead_time_days: u32,
    pub flex_capacity_percent: f64,
}

impl Default for SupplierCapacity {
    /// Creates a zero/unknown capacity — all fields set to neutral values
    /// indicating "no data available". This ensures risk scores propagate
    /// `0.0` for capacity-driven calculations until real data is populated
    /// from supplier assessments, ERP integrations, or manual entry.
    ///
    /// In production, override these via:
    /// - `current_utilization`: supplier production reports or ERP pull
    /// - `max_capacity`: manufacturing capability assessments
    /// - `lead_time_days`: historical order-to-delivery data
    /// - `flex_capacity_percent`: contractual surge provisions
    fn default() -> Self {
        Self {
            current_utilization: 0.0,
            max_capacity: 0.0,
            lead_time_days: 0,
            flex_capacity_percent: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierContact {
    pub name: String,
    pub role: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub is_primary: bool,
}

/// Component in the supply chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: Uuid,
    pub part_number: String,
    pub name: String,
    pub category: String,
    pub manufacturer: Option<Uuid>,
    pub alternate_manufacturers: Vec<Uuid>,
    pub tier: SupplierTier,
    pub risk_level: ComponentRiskLevel,
    pub lead_time_days: u32,
    pub unit_cost: Option<f64>,
    pub life_cycle_status: LifeCycleStatus,
    pub obsolescence_date: Option<DateTime<Utc>>,
    pub substitutes: Vec<String>,
    pub compliance_info: ComplianceInfo,
    pub metadata: serde_json::Value,
}

impl Component {
    pub fn new(part_number: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            part_number: part_number.into(),
            name: name.into(),
            category: String::new(),
            manufacturer: None,
            alternate_manufacturers: Vec::new(),
            tier: SupplierTier::Tier2,
            risk_level: ComponentRiskLevel::Medium,
            lead_time_days: 90,
            unit_cost: None,
            life_cycle_status: LifeCycleStatus::Active,
            obsolescence_date: None,
            substitutes: Vec::new(),
            compliance_info: ComplianceInfo::default(),
            metadata: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComponentRiskLevel {
    Critical,
    High,
    Medium,
    Low,
}

impl ComponentRiskLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LifeCycleStatus {
    Active,
    Introduction,
    Growth,
    Mature,
    Decline,
    EndOfLife,
    Obsolete,
}

impl LifeCycleStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Introduction => "introduction",
            Self::Growth => "growth",
            Self::Mature => "mature",
            Self::Decline => "decline",
            Self::EndOfLife => "end_of_life",
            Self::Obsolete => "obsolete",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplianceInfo {
    pub rohs_compliant: bool,
    pub reach_compliant: bool,
    pub conflict_minerals: Option<bool>,
    pub country_of_origin: Option<String>,
    pub customs_tariff_code: Option<String>,
}

/// Geographic concentration risk for a supply chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoConcentrationRisk {
    pub region: GeoRegion,
    pub country_code: Option<String>,
    pub supplier_count: usize,
    pub component_count: usize,
    pub revenue_exposure_percent: f64,
    pub risk_score: f64,
    pub contributing_factors: Vec<String>,
    pub mitigation_options: Vec<String>,
}

/// Single-manufacturer component analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleManufacturerRisk {
    pub component_id: Uuid,
    pub part_number: String,
    pub manufacturer_name: String,
    pub manufacturer_country: String,
    pub manufacturer_risk_score: f64,
    pub available_alternates: usize,
    pub alternate_viability: Vec<AlternateViability>,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternateViability {
    pub manufacturer_id: Uuid,
    pub manufacturer_name: String,
    pub qualification_status: QualificationStatus,
    pub lead_time_days: u32,
    pub cost_delta_percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum QualificationStatus {
    Qualified,
    InProgress,
    Planned,
    NotQualified,
}

/// Disruption scenario for supply chain planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisruptionScenario {
    pub id: Uuid,
    pub scenario_name: String,
    pub scenario_type: ScenarioType,
    pub probability: f64,
    pub impact_severity: Severity,
    pub affected_tiers: Vec<SupplierTier>,
    pub affected_regions: Vec<GeoRegion>,
    pub affected_categories: Vec<String>,
    pub duration_days: u32,
    pub financial_impact_min: Option<i64>,
    pub financial_impact_max: Option<i64>,
    pub mitigation_strategies: Vec<MitigationStrategy>,
    pub recovery_options: Vec<RecoveryOption>,
    pub created_at: DateTime<Utc>,
}

impl DisruptionScenario {
    pub fn new(scenario_name: impl Into<String>, scenario_type: ScenarioType) -> Self {
        Self {
            id: Uuid::new_v4(),
            scenario_name: scenario_name.into(),
            scenario_type,
            probability: 0.5,
            impact_severity: Severity::Medium,
            affected_tiers: Vec::new(),
            affected_regions: Vec::new(),
            affected_categories: Vec::new(),
            duration_days: 30,
            financial_impact_min: None,
            financial_impact_max: None,
            mitigation_strategies: Vec::new(),
            recovery_options: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Calculate risk exposure (probability * impact).
    pub fn risk_exposure(&self) -> f64 {
        let impact_score = match self.impact_severity {
            Severity::Critical => 1.0,
            Severity::High => 0.75,
            Severity::Medium => 0.5,
            Severity::Low => 0.25,
            Severity::Minimal => 0.1,
        };
        self.probability * impact_score
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScenarioType {
    SupplierBankruptcy,
    NaturalDisaster,
    PoliticalInstability,
    TradeWar,
    CyberAttack,
    Pandemic,
    TransportationDisruption,
    QualityIncident,
    CapacityShortage,
    RawMaterialShortage,
    RegulatoryChange,
    LaborDispute,
    QualityFraud,
    CounterfeitComponents,
}

impl ScenarioType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SupplierBankruptcy => "supplier_bankruptcy",
            Self::NaturalDisaster => "natural_disaster",
            Self::PoliticalInstability => "political_instability",
            Self::TradeWar => "trade_war",
            Self::CyberAttack => "cyber_attack",
            Self::Pandemic => "pandemic",
            Self::TransportationDisruption => "transportation_disruption",
            Self::QualityIncident => "quality_incident",
            Self::CapacityShortage => "capacity_shortage",
            Self::RawMaterialShortage => "raw_material_shortage",
            Self::RegulatoryChange => "regulatory_change",
            Self::LaborDispute => "labor_dispute",
            Self::QualityFraud => "quality_fraud",
            Self::CounterfeitComponents => "counterfeit_components",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Minimal,
}

/// Mitigation strategy for a disruption scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MitigationStrategy {
    pub strategy_name: String,
    pub implementation_cost: Option<i64>,
    pub implementation_time_months: u32,
    pub effectiveness_percent: f64,
    pub preconditions: Vec<String>,
    pub trade_offs: Vec<String>,
}

/// Recovery option after a disruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryOption {
    pub option_name: String,
    pub activation_time_hours: u32,
    pub estimated_cost: Option<i64>,
    pub constraints: Vec<String>,
}

/// Supply chain resilience score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainResilience {
    pub overall_score: f64,
    pub geographic_diversity_score: f64,
    pub supplier_diversity_score: f64,
    pub inventory_depth_score: f64,
    pub visibility_score: f64,
    pub flexibility_score: f64,
    pub bottlenecks: Vec<Bottleneck>,
    pub recommendations: Vec<String>,
}

/// Identified bottleneck in the supply chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub bottleneck_type: BottleneckType,
    pub affected_components: Vec<String>,
    pub risk_score: f64,
    pub mitigation_priority: Priority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BottleneckType {
    SingleSource,
    GeographicConcentration,
    CapacityConstraint,
    QualityIssue,
    LogisticsConstraint,
    FinancialConstraint,
}

impl BottleneckType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SingleSource => "single_source",
            Self::GeographicConcentration => "geographic_concentration",
            Self::CapacityConstraint => "capacity_constraint",
            Self::QualityIssue => "quality_issue",
            Self::LogisticsConstraint => "logistics_constraint",
            Self::FinancialConstraint => "financial_constraint",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

/// Supply Chain Threat Model.
#[derive(Debug, Clone, Default)]
pub struct SupplyChainThreatModel {
    suppliers: HashMap<Uuid, Supplier>,
    components: HashMap<Uuid, Component>,
    #[allow(dead_code)]
    relationships: HashMap<Uuid, Vec<SupplyRelationship>>,
    scenarios: Vec<DisruptionScenario>,
}

impl SupplyChainThreatModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all disruption scenarios.
    pub fn scenarios(&self) -> &[DisruptionScenario] {
        &self.scenarios
    }

    /// Add a supplier to the model.
    pub fn add_supplier(&mut self, supplier: Supplier) -> Uuid {
        let id = supplier.id;
        self.suppliers.insert(id, supplier);
        id
    }

    /// Get a supplier by ID.
    pub fn get_supplier(&self, id: Uuid) -> Option<&Supplier> {
        self.suppliers.get(&id)
    }

    /// Get a supplier by ID (mutable).
    pub fn get_supplier_mut(&mut self, id: Uuid) -> Option<&mut Supplier> {
        self.suppliers.get_mut(&id)
    }

    /// Add a component to the model.
    pub fn add_component(&mut self, component: Component) -> Uuid {
        let id = component.id;
        self.components.insert(id, component);
        id
    }

    /// Get a component by ID.
    pub fn get_component(&self, id: Uuid) -> Option<&Component> {
        self.components.get(&id)
    }

    /// Get all suppliers by tier.
    pub fn get_suppliers_by_tier(&self, tier: SupplierTier) -> Vec<&Supplier> {
        self.suppliers
            .values()
            .filter(|s| s.tier == tier)
            .collect()
    }

    /// Get all suppliers in a region.
    pub fn get_suppliers_by_region(&self, region: &GeoRegion) -> Vec<&Supplier> {
        self.suppliers
            .values()
            .filter(|s| s.region.as_ref() == Some(region))
            .collect()
    }

    /// Get all suppliers in a country.
    pub fn get_suppliers_by_country(&self, country_code: &str) -> Vec<&Supplier> {
        self.suppliers
            .values()
            .filter(|s| s.country_code == country_code.to_uppercase())
            .collect()
    }

    /// Calculate geographic concentration risk.
    pub fn calculate_geo_concentration(&self) -> Vec<GeoConcentrationRisk> {
        let mut region_map: HashMap<GeoRegion, (usize, usize, f64)> = HashMap::new();
        let mut country_map: HashMap<String, (usize, usize, f64)> = HashMap::new();

        for supplier in self.suppliers.values() {
            // By region
            if let Some(region) = &supplier.region {
                let entry = region_map.entry(region.clone()).or_insert((0, 0, 0.0));
                entry.0 += 1;
                entry.2 += supplier.criticality_score;
            }

            // By country
            let entry = country_map.entry(supplier.country_code.clone()).or_insert((0, 0, 0.0));
            entry.0 += 1;
            entry.2 += supplier.criticality_score;
        }

        // Convert to risk scores
        let total_suppliers = self.suppliers.len();
        let mut risks = Vec::new();

        for (region, (count, _, criticality_sum)) in region_map {
            if count == 0 {
                continue;
            }
            let concentration = count as f64 / total_suppliers as f64;
            let avg_criticality = criticality_sum / count as f64;
            let risk_score = (concentration * 0.6 + avg_criticality * 0.4).min(1.0);

            risks.push(GeoConcentrationRisk {
                region,
                country_code: None,
                supplier_count: count,
                component_count: 0,
                revenue_exposure_percent: concentration * 100.0,
                risk_score,
                contributing_factors: vec![
                    format!("{} suppliers in region ({}% of total)", count, (concentration * 100.0) as u32)
                ],
                mitigation_options: vec![
                    "Diversify supplier base across regions".to_string(),
                    "Establish backup suppliers in other regions".to_string(),
                ],
            });
        }

        // Sort by risk score descending
        risks.sort_by(|a, b| b.risk_score.partial_cmp(&a.risk_score).unwrap_or(std::cmp::Ordering::Equal));
        risks
    }

    /// Identify single-manufacturer components.
    pub fn identify_single_manufacturer(&self) -> Vec<SingleManufacturerRisk> {
        let mut risks = Vec::new();

        for component in self.components.values() {
            // Check if single manufacturer
            if component.alternate_manufacturers.is_empty() {
                let mfr_id = component.manufacturer;
                let supplier_ref = mfr_id.as_ref().and_then(|id| self.suppliers.get(id));
                let manufacturer_name = supplier_ref
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let manufacturer_country = supplier_ref
                    .map(|s| s.country_code.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                risks.push(SingleManufacturerRisk {
                    component_id: component.id,
                    part_number: component.part_number.clone(),
                    manufacturer_name,
                    manufacturer_country,
                    manufacturer_risk_score: 0.8, // High risk for single source
                    available_alternates: 0,
                    alternate_viability: Vec::new(),
                    recommendation: "Identify and qualify alternate manufacturers".to_string(),
                });
            }
        }

        risks
    }

    /// Add a disruption scenario.
    pub fn add_scenario(&mut self, scenario: DisruptionScenario) -> Uuid {
        let id = scenario.id;
        self.scenarios.push(scenario);
        id
    }

    /// Get scenarios by type.
    pub fn get_scenarios_by_type(&self, scenario_type: ScenarioType) -> Vec<&DisruptionScenario> {
        self.scenarios
            .iter()
            .filter(|s| s.scenario_type == scenario_type)
            .collect()
    }

    /// Get scenarios by severity.
    pub fn get_scenarios_by_severity(&self, severity: Severity) -> Vec<&DisruptionScenario> {
        self.scenarios
            .iter()
            .filter(|s| s.impact_severity == severity)
            .collect()
    }

    /// Get highest risk scenarios.
    pub fn get_high_risk_scenarios(&self, threshold: f64) -> Vec<&DisruptionScenario> {
        self.scenarios
            .iter()
            .filter(|s| s.risk_exposure() >= threshold)
            .collect()
    }

    /// Calculate supply chain resilience score.
    pub fn calculate_resilience(&self) -> SupplyChainResilience {
        let total_suppliers = self.suppliers.len();
        if total_suppliers == 0 {
            return SupplyChainResilience {
                overall_score: 0.0,
                geographic_diversity_score: 0.0,
                supplier_diversity_score: 0.0,
                inventory_depth_score: 0.5,
                visibility_score: 0.5,
                flexibility_score: 0.5,
                bottlenecks: Vec::new(),
                recommendations: vec!["Add suppliers to calculate resilience".to_string()],
            };
        }

        // Geographic diversity
        let unique_regions: std::collections::HashSet<_> = self.suppliers
            .values()
            .filter_map(|s| s.region.clone())
            .collect();
        let geographic_diversity_score = (unique_regions.len() as f64 / 6.0).min(1.0); // 6 major regions

        // Supplier diversity (by tier and category)
        let unique_tiers: std::collections::HashSet<_> = self.suppliers
            .values()
            .map(|s| s.tier)
            .collect();
        let unique_categories: std::collections::HashSet<_> = self.suppliers
            .values()
            .map(|s| s.category.clone())
            .collect();
        let supplier_diversity_score = ((unique_tiers.len() as f64 / 5.0) * 0.5
            + (unique_categories.len().min(20) as f64 / 20.0) * 0.5).min(1.0);

        // Inventory depth (based on substitutes available)
        let avg_substitutes = if self.components.is_empty() {
            0.5
        } else {
            self.components.values()
                .map(|c| c.substitutes.len() as f64)
                .sum::<f64>() / self.components.len() as f64
        };
        let inventory_depth_score = (avg_substitutes / 5.0).min(1.0);

        // Visibility (based on contact information completeness)
        let avg_contacts = self.suppliers.values()
            .map(|s| s.contacts.len() as f64)
            .sum::<f64>() / total_suppliers as f64;
        let visibility_score = (avg_contacts / 3.0).min(1.0);

        // Flexibility (based on capacity)
        let avg_flex = self.suppliers.values()
            .map(|s| s.capacity.flex_capacity_percent)
            .sum::<f64>() / total_suppliers as f64;
        let flexibility_score = (avg_flex / 0.5).min(1.0);

        // Calculate overall
        let overall_score = (geographic_diversity_score * 0.2
            + supplier_diversity_score * 0.25
            + inventory_depth_score * 0.2
            + visibility_score * 0.15
            + flexibility_score * 0.2).min(1.0);

        // Identify bottlenecks
        let mut bottlenecks = Vec::new();

        // Single source bottlenecks
        for risk in self.identify_single_manufacturer() {
            bottlenecks.push(Bottleneck {
                bottleneck_type: BottleneckType::SingleSource,
                affected_components: vec![risk.part_number],
                risk_score: risk.manufacturer_risk_score,
                mitigation_priority: Priority::High,
            });
        }

        // Geographic concentration bottlenecks
        for geo_risk in self.calculate_geo_concentration() {
            if geo_risk.risk_score > 0.5 {
                bottlenecks.push(Bottleneck {
                    bottleneck_type: BottleneckType::GeographicConcentration,
                    affected_components: Vec::new(),
                    risk_score: geo_risk.risk_score,
                    mitigation_priority: if geo_risk.risk_score > 0.7 {
                        Priority::Critical
                    } else {
                        Priority::High
                    },
                });
            }
        }

        let recommendations = Self::generate_recommendations(overall_score, &bottlenecks);
        SupplyChainResilience {
            overall_score,
            geographic_diversity_score,
            supplier_diversity_score,
            inventory_depth_score,
            visibility_score,
            flexibility_score,
            bottlenecks,
            recommendations,
        }
    }

    fn generate_recommendations(score: f64, bottlenecks: &[Bottleneck]) -> Vec<String> {
        let mut recommendations = Vec::new();

        if score < 0.4 {
            recommendations.push("Critical: Supply chain resilience is low. Immediate action required.".to_string());
        } else if score < 0.6 {
            recommendations.push("Moderate resilience. Address identified bottlenecks.".to_string());
        }

        for bottleneck in bottlenecks {
            match bottleneck.bottleneck_type {
                BottleneckType::SingleSource => {
                    recommendations.push(format!(
                        "Single source identified for components. Risk: {:.0}%. Priority: {:?}. Recommend qualifying alternate manufacturers.",
                        bottleneck.risk_score * 100.0,
                        bottleneck.mitigation_priority
                    ));
                }
                BottleneckType::GeographicConcentration => {
                    recommendations.push(format!(
                        "Geographic concentration risk identified. Risk: {:.0}%. Priority: {:?}. Consider diversifying to other regions.",
                        bottleneck.risk_score * 100.0,
                        bottleneck.mitigation_priority
                    ));
                }
                _ => {}
            }
        }

        if recommendations.is_empty() {
            recommendations.push("Supply chain appears resilient. Continue monitoring.".to_string());
        }

        recommendations
    }

    /// Generate standard disruption scenarios.
    pub fn generate_standard_scenarios(&mut self) {
        // Natural disaster scenarios by region
        let scenarios = vec![
            DisruptionScenario::new("Taiwan Earthquake", ScenarioType::NaturalDisaster)
                .with_probability(0.15)
                .with_impact(Severity::Critical)
                .with_regions(vec![GeoRegion::EastAsia])
                .with_duration(90)
                .with_financial_impact(500_000_000, 2_000_000_000)
                .with_mitigation(
                    "Maintain 60-day safety stock of critical components",
                    5_000_000,
                    12,
                    0.8,
                )
                .with_recovery("Activate alternate manufacturers from Korea/Japan", 72),

            DisruptionScenario::new("South China Sea Tensions", ScenarioType::PoliticalInstability)
                .with_probability(0.25)
                .with_impact(Severity::High)
                .with_regions(vec![GeoRegion::EastAsia, GeoRegion::SoutheastAsia])
                .with_duration(180)
                .with_mitigation(
                    "Diversify assembly to Vietnam/Malaysia facilities",
                    10_000_000,
                    18,
                    0.9,
                ),

            DisruptionScenario::new("Semiconductor Capacity Shortage", ScenarioType::CapacityShortage)
                .with_probability(0.35)
                .with_impact(Severity::High)
                .with_categories(vec!["Semiconductors".to_string(), "Electronics".to_string()])
                .with_duration(120)
                .with_mitigation(
                    "Long-term supply agreements with buffer allocation",
                    2_000_000,
                    6,
                    0.7,
                ),

            DisruptionScenario::new("Cyber Attack on Tier-1 Supplier", ScenarioType::CyberAttack)
                .with_probability(0.20)
                .with_impact(Severity::Critical)
                .with_tiers(vec![SupplierTier::Tier1])
                .with_duration(60)
                .with_mitigation(
                    "Cyber insurance and incident response plan",
                    1_000_000,
                    3,
                    0.6,
                ),

            DisruptionScenario::new("Supplier Bankruptcy", ScenarioType::SupplierBankruptcy)
                .with_probability(0.15)
                .with_impact(Severity::High)
                .with_duration(120)
                .with_mitigation(
                    "Monitor supplier financial health and maintain backup sources",
                    500_000,
                    6,
                    0.75,
                ),
        ];

        for scenario in scenarios {
            self.add_scenario(scenario);
        }
    }
}

// Builder pattern extensions
impl Supplier {
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_criticality(mut self, criticality: f64) -> Self {
        self.criticality_score = criticality.clamp(0.0, 1.0);
        self
    }

    pub fn with_region(mut self, region: GeoRegion) -> Self {
        self.region = Some(region);
        self
    }
}

impl Component {
    pub fn with_risk_level(mut self, risk_level: ComponentRiskLevel) -> Self {
        self.risk_level = risk_level;
        self
    }

    pub fn with_manufacturer(mut self, manufacturer_id: Uuid) -> Self {
        self.manufacturer = Some(manufacturer_id);
        self
    }
}

impl DisruptionScenario {
    pub fn with_probability(mut self, probability: f64) -> Self {
        self.probability = probability.clamp(0.0, 1.0);
        self
    }

    pub fn with_impact(mut self, severity: Severity) -> Self {
        self.impact_severity = severity;
        self
    }

    pub fn with_tiers(mut self, tiers: Vec<SupplierTier>) -> Self {
        self.affected_tiers = tiers;
        self
    }

    pub fn with_regions(mut self, regions: Vec<GeoRegion>) -> Self {
        self.affected_regions = regions;
        self
    }

    pub fn with_categories(mut self, categories: Vec<String>) -> Self {
        self.affected_categories = categories;
        self
    }

    pub fn with_duration(mut self, days: u32) -> Self {
        self.duration_days = days;
        self
    }

    pub fn with_financial_impact(mut self, min: i64, max: i64) -> Self {
        self.financial_impact_min = Some(min);
        self.financial_impact_max = Some(max);
        self
    }

    pub fn with_mitigation(
        mut self,
        name: &str,
        cost: i64,
        months: u32,
        effectiveness: f64,
    ) -> Self {
        self.mitigation_strategies.push(MitigationStrategy {
            strategy_name: name.to_string(),
            implementation_cost: Some(cost),
            implementation_time_months: months,
            effectiveness_percent: effectiveness.clamp(0.0, 1.0),
            preconditions: Vec::new(),
            trade_offs: Vec::new(),
        });
        self
    }

    pub fn with_recovery(mut self, name: &str, hours: u32) -> Self {
        self.recovery_options.push(RecoveryOption {
            option_name: name.to_string(),
            activation_time_hours: hours,
            estimated_cost: None,
            constraints: Vec::new(),
        });
        self
    }
}

// ============================================================================
// Supply Chain Risk Summary (for threat_intel_refresh worker integration)
// ============================================================================

/// Summarized supply chain risk assessment produced by the threat intel worker.
/// Provides a lightweight overview suitable for storage and API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainRiskSummary {
    /// Overall resilience score (0-100).
    pub resilience_score: f64,
    /// Human-readable tier label ("Resilient", "Moderate", "Vulnerable").
    pub tier_label: String,
    /// Geographic concentration risk factor (0.0-1.0).
    pub geographic_concentration_risk: f64,
    /// Part numbers / names of single-source components.
    pub single_source_components: Vec<String>,
    /// Names of high-risk disruption scenarios identified.
    pub disruption_scenarios: Vec<String>,
    /// Actionable recommendations for risk mitigation.
    pub recommended_actions: Vec<String>,
}

impl SupplyChainRiskSummary {
    /// Create a new empty summary.
    pub fn new() -> Self {
        Self {
            resilience_score: 0.0,
            tier_label: "Unknown".to_string(),
            geographic_concentration_risk: 0.0,
            single_source_components: Vec::new(),
            disruption_scenarios: Vec::new(),
            recommended_actions: Vec::new(),
        }
    }

    /// Human-readable tier label based on resilience score.
    pub fn tier_label(&self) -> &str {
        &self.tier_label
    }
}

impl Default for SupplyChainRiskSummary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_supplier_creation() {
        let supplier = Supplier::new("Acme Corp", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_category("Electronics")
            .with_criticality(0.8);

        assert_eq!(supplier.name, "Acme Corp");
        assert_eq!(supplier.country_code, "US");
        assert_eq!(supplier.tier, SupplierTier::Tier1);
        assert_eq!(supplier.criticality_score, 0.8);
    }

    #[test]
    fn test_component_creation() {
        let component = Component::new("PART-001", "Microcontroller")
            .with_risk_level(ComponentRiskLevel::Critical);

        assert_eq!(component.part_number, "PART-001");
        assert_eq!(component.risk_level, ComponentRiskLevel::Critical);
    }

    #[test]
    fn test_scenario_risk_exposure() {
        let scenario = DisruptionScenario::new("Test Scenario", ScenarioType::NaturalDisaster)
            .with_probability(0.5)
            .with_impact(Severity::Critical);

        assert!((scenario.risk_exposure() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_geo_concentration_calculation() {
        let mut model = SupplyChainThreatModel::new();

        // Add suppliers from different regions
        model.add_supplier(Supplier::new("US Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::NorthAmerica));
        model.add_supplier(Supplier::new("China Supplier 1", "CN", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::EastAsia));
        model.add_supplier(Supplier::new("China Supplier 2", "CN", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::EastAsia));

        let risks = model.calculate_geo_concentration();
        // China/East Asia should have higher concentration (2/3 = 66%)
        assert!(!risks.is_empty());
    }

    #[test]
    fn test_single_manufacturer_identification() {
        let mut model = SupplyChainThreatModel::new();

        let _supplier_id = model.add_supplier(
            Supplier::new("Primary Manufacturer", "CN", SupplierTier::Tier2, SupplierCapacity::default())
        );

        model.add_component(
            Component::new("PART-001", "Critical Chip")
                .with_risk_level(ComponentRiskLevel::Critical)
        );

        let risks = model.identify_single_manufacturer();
        assert_eq!(risks.len(), 1); // No alternates
    }

    #[test]
    fn test_resilience_calculation() {
        let mut model = SupplyChainThreatModel::new();

        // Add diverse suppliers from different regions
        model.add_supplier(Supplier::new("US Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::NorthAmerica));
        model.add_supplier(Supplier::new("EU Supplier", "DE", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::Europe));
        model.add_supplier(Supplier::new("APAC Supplier", "JP", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::AsiaPacific));

        let resilience = model.calculate_resilience();
        assert!(resilience.geographic_diversity_score > 0.0);
        assert!(resilience.overall_score > 0.0);
    }

    #[test]
    fn test_scenario_generation() {
        let mut model = SupplyChainThreatModel::new();
        model.generate_standard_scenarios();

        assert!(!model.scenarios.is_empty());
        
        let natural_disasters = model.get_scenarios_by_type(ScenarioType::NaturalDisaster);
        assert!(!natural_disasters.is_empty());
    }

    #[test]
    fn test_tier_filtering() {
        let mut model = SupplyChainThreatModel::new();

        model.add_supplier(Supplier::new("Tier 1 Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default()));
        model.add_supplier(Supplier::new("Tier 2 Supplier", "CN", SupplierTier::Tier2, SupplierCapacity::default()));
        model.add_supplier(Supplier::new("Another Tier 1", "DE", SupplierTier::Tier1, SupplierCapacity::default()));

        let tier1 = model.get_suppliers_by_tier(SupplierTier::Tier1);
        assert_eq!(tier1.len(), 2);

        let tier2 = model.get_suppliers_by_tier(SupplierTier::Tier2);
        assert_eq!(tier2.len(), 1);
    }

    #[test]
    fn test_supplier_risk_score_calculation() {
        let mut risk = SupplierRiskScore::new(Uuid::new_v4());
        risk.financial_risk = 0.6;
        risk.operational_risk = 0.7;
        risk.geopolitical_risk = 0.5;
        risk.cyber_risk = 0.3;
        risk.concentration_risk = 0.4;
        risk.dependency_score = 0.6;

        risk.calculate_overall();

        // Check weighted calculation
        let expected = 0.6 * 0.20 + 0.7 * 0.20 + 0.5 * 0.25 + 0.3 * 0.15 + 0.4 * 0.10 + 0.6 * 0.10;
        assert!((risk.overall_score - expected).abs() < 0.01);
    }

    #[test]
    fn test_life_cycle_status() {
        let component = Component::new("EOL-PART", "End of Life Component");
        // Default status is Active, let's create one in EOL
        assert_eq!(component.life_cycle_status, LifeCycleStatus::Active);
    }

    #[test]
    fn test_bottleneck_identification() {
        let mut model = SupplyChainThreatModel::new();

        // Add high-risk supplier
        let supplier_id = model.add_supplier(
            Supplier::new("Single Source Supplier", "TW", SupplierTier::Tier1, SupplierCapacity::default())
                .with_criticality(1.0)
                .with_region(GeoRegion::EastAsia)
        );

        // Add a component that depends on this supplier with no alternates
        model.add_component(
            Component::new("PART-001", "Critical Chip")
                .with_risk_level(ComponentRiskLevel::Critical)
                .with_manufacturer(supplier_id)
        );

        let resilience = model.calculate_resilience();
        assert!(!resilience.bottlenecks.is_empty());
    }
}
