//! Threat modeling for OSINT investigations.
//!
//! Provides:
//! - Attack surface assessment
//! - Supply chain risk vectors
//! - Adversary capability mapping
//! - Scenario simulation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use apex_threat_intel::threat_actor_database::ThreatActorDatabase;
use apex_threat_intel::models::IndustrySector;

/// Risk vector types for threat assessment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RiskVector {
    /// Financial risk (instability, bankruptcy)
    Financial,
    /// Supply chain disruption risk
    SupplyChain,
    /// Geopolitical/regulatory risk
    Geopolitical,
    /// Competitive threat
    Competitive,
    /// Operational risk
    Operational,
    /// Cybersecurity risk
    Cybersecurity,
    /// Reputational risk
    Reputational,
    /// Compliance/legal risk
    Compliance,
    /// Strategic risk
    Strategic,
    /// Technology/obsolescence risk
    Technology,
}

impl RiskVector {
    pub fn label(&self) -> &'static str {
        match self {
            RiskVector::Financial => "Financial Risk",
            RiskVector::SupplyChain => "Supply Chain Risk",
            RiskVector::Geopolitical => "Geopolitical Risk",
            RiskVector::Competitive => "Competitive Risk",
            RiskVector::Operational => "Operational Risk",
            RiskVector::Cybersecurity => "Cybersecurity Risk",
            RiskVector::Reputational => "Reputational Risk",
            RiskVector::Compliance => "Compliance/Legal Risk",
            RiskVector::Strategic => "Strategic Risk",
            RiskVector::Technology => "Technology Risk",
        }
    }
}

/// Known threat actor profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatActor {
    /// Unique identifier
    pub id: String,
    /// Actor name/alias
    pub name: String,
    /// Actor type
    pub actor_type: ThreatActorType,
    /// Industry focus
    pub industry_focus: Vec<String>,
    /// Geographic focus
    pub geographic_focus: Vec<String>,
    /// Known capabilities
    pub capabilities: Vec<String>,
    /// Historical TTPs (Tactics, Techniques, Procedures)
    pub ttps: Vec<String>,
    /// Threat level (0-1)
    pub threat_level: f64,
    /// Attribution confidence
    pub attribution_confidence: f64,
    /// Last observed
    pub last_observed: DateTime<Utc>,
    /// Active status
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreatActorType {
    /// Nation-state actor
    NationState,
    /// Criminal organization
    Criminal,
    /// Hacktivist group
    Hacktivist,
    /// Insider threat
    Insider,
    /// Competitor corporate espionage
    Competitor,
    /// Supply chain threat
    SupplyChain,
    /// Unknown/uncategorized
    Unknown,
}

/// Adversary capability assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryCapability {
    /// Capability category
    pub category: String,
    /// Capability name
    pub name: String,
    /// Maturity level (1-5)
    pub maturity_level: u8,
    /// Observed usage frequency
    pub usage_frequency: f64,
    /// Effectiveness rating (0-1)
    pub effectiveness: f64,
    /// Countermeasure availability
    pub countermeasure_available: bool,
    /// Description
    pub description: String,
}

impl AdversaryCapability {
    /// Get maturity description.
    pub fn maturity_description(&self) -> &'static str {
        match self.maturity_level {
            1 => "Emerging",
            2 => "Developing",
            3 => "Established",
            4 => "Advanced",
            5 => "Cutting-edge",
            _ => "Unknown",
        }
    }
}

/// Attack surface assessment for an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackSurfaceAssessment {
    /// Entity being assessed
    pub entity_id: String,
    /// Assessment timestamp
    pub assessed_at: DateTime<Utc>,
    /// Overall attack surface score (0-1, higher = more exposed)
    pub overall_score: f64,
    /// External exposure points
    pub external_exposure: Vec<ExposurePoint>,
    /// Vulnerabilities identified
    pub vulnerabilities: Vec<Vulnerability>,
    /// Misconfigurations
    pub misconfigurations: Vec<Misconfiguration>,
    /// Shadow IT detected
    pub shadow_it: Vec<ShadowITAsset>,
    /// Recommendations
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposurePoint {
    pub asset_type: String,
    pub asset_name: String,
    pub exposure_type: ExposureType,
    pub severity: String,
    pub description: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExposureType {
    /// Public-facing web presence
    Web,
    /// Network exposure
    Network,
    /// Cloud misconfiguration
    Cloud,
    /// Third-party exposure
    ThirdParty,
    /// Physical security
    Physical,
    /// Social engineering target
    SocialEngineering,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: String,
    pub cve_id: Option<String>,
    pub title: String,
    pub severity: String,
    pub cvss_score: Option<f64>,
    pub affected_systems: Vec<String>,
    pub patch_available: bool,
    pub exploitation_observed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Misconfiguration {
    pub system: String,
    pub issue_type: String,
    pub severity: String,
    pub description: String,
    pub remediation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowITAsset {
    pub asset_name: String,
    pub service_type: String,
    pub discovered_at: DateTime<Utc>,
    pub owner_unknown: bool,
    pub risk_level: String,
}

/// Supply chain threat model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainThreatModel {
    /// Entity being modeled
    pub entity_id: String,
    /// Tier-1 suppliers
    pub tier1_suppliers: Vec<SupplierRisk>,
    /// Tier-2+ suppliers
    pub tier2_suppliers: Vec<SupplierRisk>,
    /// Geographic concentration risk
    pub geographic_concentration: GeographicRisk,
    /// Single-manufacturer components
    pub single_source_components: Vec<SingleSourceRisk>,
    /// Disruption scenarios
    pub disruption_scenarios: Vec<DisruptionScenario>,
    /// Overall supply chain risk score
    pub overall_risk_score: f64,
    /// Mitigation recommendations
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierRisk {
    pub supplier_name: String,
    pub supplier_id: String,
    pub tier: u8,
    pub risk_score: f64,
    pub risk_factors: Vec<String>,
    pub financial_health: Option<f64>,
    pub geographic_risk: Option<String>,
    pub concentration_risk: String,
    pub diversification_options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicRisk {
    pub country_code: String,
    pub country_name: String,
    pub risk_factors: Vec<String>,
    pub political_stability: f64,
    pub trade_risk: f64,
    pub natural_disaster_risk: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleSourceRisk {
    pub component_name: String,
    pub sole_manufacturer: String,
    pub manufacturer_country: String,
    pub alternative_availability: bool,
    pub alternative_count: usize,
    pub switchover_time_months: usize,
    pub inventory_buffer_months: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisruptionScenario {
    pub scenario_name: String,
    pub probability: f64,
    pub impact_severity: String,
    pub affected_components: Vec<String>,
    pub estimated_recovery_time_days: usize,
    pub financial_impact_estimate: Option<String>,
    pub mitigation_actions: Vec<String>,
}

/// Scenario simulation for threat modeling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSimulation {
    /// Scenario identifier
    pub id: String,
    /// Scenario name
    pub name: String,
    /// Scenario type
    pub scenario_type: ScenarioType,
    /// Description
    pub description: String,
    /// Trigger conditions
    pub trigger_conditions: Vec<String>,
    /// Probability (0-1)
    pub probability: f64,
    /// Impact severity (0-1)
    pub impact_severity: f64,
    /// Risk score (probability * impact)
    pub risk_score: f64,
    /// Affected entities
    pub affected_entities: Vec<String>,
    /// Recommended responses
    pub recommended_responses: Vec<String>,
    /// Early warning indicators
    pub early_warning_indicators: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScenarioType {
    /// Supply chain disruption
    SupplyChainDisruption,
    /// Cyber attack
    CyberAttack,
    /// Financial distress
    FinancialDistress,
    /// Geopolitical event
    GeopoliticalEvent,
    /// Regulatory change
    RegulatoryChange,
    /// Competitive move
    CompetitiveMove,
    /// Leadership change
    LeadershipChange,
    /// Market shift
    MarketShift,
    /// Natural disaster
    NaturalDisaster,
}

impl ScenarioType {
    pub fn label(&self) -> &'static str {
        match self {
            ScenarioType::SupplyChainDisruption => "Supply Chain Disruption",
            ScenarioType::CyberAttack => "Cyber Attack",
            ScenarioType::FinancialDistress => "Financial Distress",
            ScenarioType::GeopoliticalEvent => "Geopolitical Event",
            ScenarioType::RegulatoryChange => "Regulatory Change",
            ScenarioType::CompetitiveMove => "Competitive Move",
            ScenarioType::LeadershipChange => "Leadership Change",
            ScenarioType::MarketShift => "Market Shift",
            ScenarioType::NaturalDisaster => "Natural Disaster",
        }
    }
}

/// Threat modeling engine.
#[derive(Clone)]
pub struct ThreatModeling {
    /// Threat Actor Database from threat-intel crate
    actor_database: ThreatActorDatabase,
    /// Derived adversary capabilities
    #[allow(dead_code)]
    adversary_capabilities: Vec<AdversaryCapability>,
    /// Industry context
    #[allow(dead_code)]
    industry_context: String,
    /// Cached converted threat actors
    #[allow(dead_code)]
    threat_actors: Vec<ThreatActor>,
}

impl Default for ThreatModeling {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreatModeling {
    /// Create a new threat modeling engine backed by the threat intelligence database.
    pub fn new() -> Self {
        let db = ThreatActorDatabase::with_known_actors();

        // Derive adversary capabilities from known actors
        let adversary_capabilities = vec![
            AdversaryCapability {
                category: "Supply Chain".to_string(),
                name: "Supply Chain Infiltration".to_string(),
                maturity_level: 4,
                usage_frequency: 0.3,
                effectiveness: 0.9,
                countermeasure_available: true,
                description: "Ability to compromise upstream suppliers".to_string(),
            },
            AdversaryCapability {
                category: "Cyber".to_string(),
                name: "Ransomware".to_string(),
                maturity_level: 5,
                usage_frequency: 0.8,
                effectiveness: 0.7,
                countermeasure_available: true,
                description: "Deploy ransomware for financial gain".to_string(),
            },
            AdversaryCapability {
                category: "Competitive".to_string(),
                name: "Corporate Espionage".to_string(),
                maturity_level: 3,
                usage_frequency: 0.4,
                effectiveness: 0.6,
                countermeasure_available: true,
                description: "Intelligence gathering on competitors".to_string(),
            },
        ];

        // Convert all known actors from the database into investigation's ThreatActor type
        let params = apex_threat_intel::models::PaginationParams::new(0, 1000);
        let all_actors = db.list_actors(params);
        let threat_actors: Vec<ThreatActor> = all_actors.items.iter().map(convert_threat_actor).collect();

        Self {
            actor_database: db,
            threat_actors,
            adversary_capabilities,
            industry_context: "Manufacturing/Electronics".to_string(),
        }
    }

    /// Get a reference to the underlying threat actor database.
    pub fn database(&self) -> &ThreatActorDatabase {
        &self.actor_database
    }

    /// Get threat actors targeting a specific industry sector.
    pub fn get_actors_for_industry(&self, industry: &str) -> Vec<ThreatActor> {
        let sector = IndustrySector::from_str(industry);
        self.actor_database
            .get_actors_by_sector(&sector)
            .into_iter()
            .map(convert_threat_actor)
            .collect()
    }

    /// Get a mutable reference to the underlying threat actor database.
    pub fn database_mut(&mut self) -> &mut ThreatActorDatabase {
        &mut self.actor_database
    }

    /// Assess attack surface for an entity.
    pub fn assess_attack_surface(&self, entity_id: &str, external_assets: &[ExternalAsset]) -> AttackSurfaceAssessment {
        let mut vulnerabilities = Vec::new();
        let mut misconfigurations = Vec::new();
        let mut shadow_it = Vec::new();
        let mut recommendations = Vec::new();

        // Analyze external assets
        for asset in external_assets {
            match asset.asset_type.as_str() {
                // Web exposure
                "web" => {
                    if asset.has_known_vulnerabilities {
                        vulnerabilities.push(Vulnerability {
                            id: Uuid::new_v4().to_string(),
                            cve_id: None,
                            title: "Potentially vulnerable web application".to_string(),
                            severity: "Medium".to_string(),
                            cvss_score: Some(5.0),
                            affected_systems: vec![asset.name.clone()],
                            patch_available: true,
                            exploitation_observed: false,
                        });
                    }
                    if asset.has_security_headers.is_none_or(|h| !h) {
                        misconfigurations.push(Misconfiguration {
                            system: asset.name.clone(),
                            issue_type: "Missing security headers".to_string(),
                            severity: "Medium".to_string(),
                            description: "Web application missing security headers".to_string(),
                            remediation: "Configure security headers (CSP, HSTS, etc.)".to_string(),
                        });
                    }
                }

                // Cloud misconfigurations
                "cloud" => {
                    if asset.is_publicly_accessible.unwrap_or(false) {
                        shadow_it.push(ShadowITAsset {
                            asset_name: asset.name.clone(),
                            service_type: "Cloud Storage".to_string(),
                            discovered_at: Utc::now(),
                            owner_unknown: true,
                            risk_level: "High".to_string(),
                        });
                        recommendations.push(format!("Review and secure cloud asset: {}", asset.name));
                    }
                }

                // Mobile application exposure
                "mobile" => {
                    vulnerabilities.push(Vulnerability {
                        id: Uuid::new_v4().to_string(),
                        cve_id: None,
                        title: "Mobile application exposure".to_string(),
                        severity: "Medium".to_string(),
                        cvss_score: Some(4.5),
                        affected_systems: vec![asset.name.clone()],
                        patch_available: true,
                        exploitation_observed: false,
                    });
                    misconfigurations.push(Misconfiguration {
                        system: asset.name.clone(),
                        issue_type: "Mobile app security posture".to_string(),
                        severity: "Medium".to_string(),
                        description: "Mobile application may expose backend APIs or sensitive data".to_string(),
                        remediation: "Review mobile app certificate pinning, API auth, and data storage".to_string(),
                    });
                }

                // IoT device exposure
                "iot" => {
                    vulnerabilities.push(Vulnerability {
                        id: Uuid::new_v4().to_string(),
                        cve_id: None,
                        title: "IoT device exposure".to_string(),
                        severity: "High".to_string(),
                        cvss_score: Some(7.0),
                        affected_systems: vec![asset.name.clone()],
                        patch_available: false,
                        exploitation_observed: false,
                    });
                    recommendations.push(format!("Conduct IoT firmware and communication security review for: {}", asset.name));
                }

                // Desktop/endpoint exposure
                "desktop" => {
                    if asset.has_known_vulnerabilities {
                        vulnerabilities.push(Vulnerability {
                            id: Uuid::new_v4().to_string(),
                            cve_id: None,
                            title: "Vulnerable desktop/endpoint software".to_string(),
                            severity: "Medium".to_string(),
                            cvss_score: Some(5.5),
                            affected_systems: vec![asset.name.clone()],
                            patch_available: true,
                            exploitation_observed: false,
                        });
                    }
                    misconfigurations.push(Misconfiguration {
                        system: asset.name.clone(),
                        issue_type: "Endpoint security posture".to_string(),
                        severity: "Low".to_string(),
                        description: "Desktop endpoint may have unpatched software or weak configurations".to_string(),
                        remediation: "Ensure endpoint protection, patch management, and EDR coverage".to_string(),
                    });
                }

                // Operational Technology exposure
                "ot" => {
                    vulnerabilities.push(Vulnerability {
                        id: Uuid::new_v4().to_string(),
                        cve_id: None,
                        title: "OT/SCADA system exposure".to_string(),
                        severity: "Critical".to_string(),
                        cvss_score: Some(9.0),
                        affected_systems: vec![asset.name.clone()],
                        patch_available: false,
                        exploitation_observed: false,
                    });
                    recommendations.push(format!("Isolate OT network and perform ICS security assessment for: {}", asset.name));
                }

                // Default: unknown asset type — still generate a basic exposure point
                _ => {
                    vulnerabilities.push(Vulnerability {
                        id: Uuid::new_v4().to_string(),
                        cve_id: None,
                        title: format!("Unclassified asset exposure: {}", asset.asset_type),
                        severity: "Low".to_string(),
                        cvss_score: Some(3.0),
                        affected_systems: vec![asset.name.clone()],
                        patch_available: false,
                        exploitation_observed: false,
                    });
                    recommendations.push(format!("Catalog and assess unknown asset type '{}': {}", asset.asset_type, asset.name));
                }
            }
        }

        // Calculate overall score
        let vuln_score = (vulnerabilities.len() as f64 * 0.1).min(0.4);
        let misconfig_score = (misconfigurations.len() as f64 * 0.05).min(0.3);
        let shadow_score = if shadow_it.is_empty() { 0.0 } else { 0.3 };
        let overall_score = (vuln_score + misconfig_score + shadow_score).min(1.0);

        // Add recommendations
        if !vulnerabilities.is_empty() {
            recommendations.push("Review and address identified vulnerabilities".to_string());
        }
        if !misconfigurations.is_empty() {
            recommendations.push("Fix security misconfigurations".to_string());
        }
        if !shadow_it.is_empty() {
            recommendations.push("Review and secure shadow IT assets".to_string());
        }

        AttackSurfaceAssessment {
            entity_id: entity_id.to_string(),
            assessed_at: Utc::now(),
            overall_score,
            external_exposure: Vec::new(),
            vulnerabilities,
            misconfigurations,
            shadow_it,
            recommendations,
        }
    }

    /// Model supply chain threats.
    pub fn model_supply_chain(&self, entity_id: &str, suppliers: &[Supplier]) -> SupplyChainThreatModel {
        let mut tier1_risks = Vec::new();
        let mut tier2_risks = Vec::new();
        let mut single_source = Vec::new();
        let mut scenarios = Vec::new();
        let mut recommendations = Vec::new();

        // Pre-compute supplier distribution for concentration risk
        // Count how many suppliers share each country to estimate geographic concentration
        let mut country_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for s in suppliers {
            if let Some(ref c) = s.country {
                *country_counts.entry(c.as_str()).or_insert(0) += 1;
            }
        }
        let total_suppliers = suppliers.len();

        fn compute_concentration(country: &Option<String>, country_counts: &std::collections::HashMap<&str, usize>, total: usize) -> String {
            match country {
                Some(c) => {
                    let count = country_counts.get(c.as_str()).copied().unwrap_or(0);
                    let ratio = if total > 0 { count as f64 / total as f64 } else { 0.0 };
                    if ratio >= 0.5 {
                        "High".to_string()
                    } else if ratio >= 0.25 {
                        "Medium".to_string()
                    } else {
                        "Low".to_string()
                    }
                }
                None => "Unknown".to_string(),
            }
        }

        // Analyze each supplier
        for supplier in suppliers {
            let risk_factors = self.calculate_supplier_risk_factors(supplier);
            let risk_score = self.calculate_supplier_risk_score(&risk_factors);
            let concentration = compute_concentration(&supplier.country, &country_counts, total_suppliers);

            if supplier.tier == 1 {
                tier1_risks.push(SupplierRisk {
                    supplier_name: supplier.name.clone(),
                    supplier_id: supplier.id.clone(),
                    tier: supplier.tier,
                    risk_score,
                    risk_factors: risk_factors.clone(),
                    financial_health: supplier.financial_score,
                    geographic_risk: supplier.country.clone(),
                    concentration_risk: concentration,
                    diversification_options: vec![
                        "Identify alternative suppliers in different regions".to_string(),
                        "Develop second-source qualification".to_string(),
                    ],
                });
            } else {
                tier2_risks.push(SupplierRisk {
                    supplier_name: supplier.name.clone(),
                    supplier_id: supplier.id.clone(),
                    tier: supplier.tier,
                    risk_score,
                    risk_factors,
                    financial_health: supplier.financial_score,
                    geographic_risk: supplier.country.clone(),
                    concentration_risk: concentration,
                    diversification_options: vec![],
                });
            }

            // Check for single-source dependencies
            if supplier.is_single_source {
                single_source.push(SingleSourceRisk {
                    component_name: supplier.component.clone(),
                    sole_manufacturer: supplier.name.clone(),
                    manufacturer_country: supplier.country.clone().unwrap_or_default(),
                    alternative_availability: false,
                    alternative_count: 0,
                    switchover_time_months: 12,
                    inventory_buffer_months: 3,
                });
            }
        }

        // Generate disruption scenarios
        if !tier1_risks.is_empty() {
            let top_risk = tier1_risks.iter().max_by_key(|s| (s.risk_score * 100.0) as i32);
            if let Some(risk) = top_risk {
                scenarios.push(DisruptionScenario {
                    scenario_name: format!("Loss of {} as supplier", risk.supplier_name),
                    probability: risk.risk_score,
                    impact_severity: "Critical".to_string(),
                    affected_components: vec![risk.supplier_name.clone()],
                    estimated_recovery_time_days: 90,
                    financial_impact_estimate: Some("$10-50M depending on component".to_string()),
                    mitigation_actions: vec![
                        "Qualify alternative supplier".to_string(),
                        "Increase safety stock".to_string(),
                    ],
                });
            }
        }

        // Calculate overall risk
        let avg_tier1_risk: f64 = if tier1_risks.is_empty() {
            0.0
        } else {
            tier1_risks.iter().map(|s| s.risk_score).sum::<f64>() / tier1_risks.len() as f64
        };
        let single_source_factor = (single_source.len() as f64 * 0.1).min(0.3);
        let overall_risk_score = (avg_tier1_risk + single_source_factor).min(1.0);

        // Recommendations
        if tier1_risks.len() > 5 {
            recommendations.push("Consider supplier diversification program".to_string());
        }
        if !single_source.is_empty() {
            recommendations.push("Develop second-source qualification for single-source components".to_string());
        }

        SupplyChainThreatModel {
            entity_id: entity_id.to_string(),
            tier1_suppliers: tier1_risks,
            tier2_suppliers: tier2_risks,
            geographic_concentration: GeographicRisk {
                country_code: "MULTI".to_string(),
                country_name: "Multiple Countries".to_string(),
                risk_factors: vec!["Geographic diversification present".to_string()],
                political_stability: 0.7,
                trade_risk: 0.3,
                natural_disaster_risk: 0.2,
            },
            single_source_components: single_source,
            disruption_scenarios: scenarios,
            overall_risk_score,
            recommendations,
        }
    }

    /// Calculate risk factors for a supplier.
    fn calculate_supplier_risk_factors(&self, supplier: &Supplier) -> Vec<String> {
        let mut factors = Vec::new();

        if supplier.financial_score.unwrap_or(0.5) < 0.4 {
            factors.push("Poor financial health indicator".to_string());
        }
        if let Some(country) = &supplier.country {
            if ["RU", "CN", "IR", "KP"].contains(&country.as_str()) {
                factors.push("Geopolitical risk country".to_string());
            }
        }
        if supplier.on_time_delivery.unwrap_or(0.9) < 0.8 {
            factors.push("Historical delivery issues".to_string());
        }
        if supplier.quality_score.unwrap_or(0.9) < 0.8 {
            factors.push("Quality concerns".to_string());
        }

        if factors.is_empty() {
            factors.push("No significant risk factors identified".to_string());
        }

        factors
    }

    /// Calculate overall risk score for a supplier.
    fn calculate_supplier_risk_score(&self, factors: &[String]) -> f64 {
        let mut score: f64 = 0.3; // Base score

        for factor in factors {
            if factor.contains("financial") {
                score += 0.2;
            }
            if factor.contains("Geopolitical") {
                score += 0.15;
            }
            if factor.contains("delivery") {
                score += 0.1;
            }
            if factor.contains("Quality") {
                score += 0.1;
            }
        }

        score.min(1.0)
    }

    /// Simulate threat scenarios.
    pub fn simulate_scenarios(&self, entity_id: &str, context: &SimulationContext) -> Vec<ScenarioSimulation> {
        let mut scenarios = Vec::new();

        // Supply chain disruption scenario
        if context.supply_chain_risk_score > 0.3 {
            scenarios.push(ScenarioSimulation {
                id: Uuid::new_v4().to_string(),
                name: "Major Supply Chain Disruption".to_string(),
                scenario_type: ScenarioType::SupplyChainDisruption,
                description: "A critical supplier experiences a disruption affecting production".to_string(),
                trigger_conditions: vec![
                    "Supplier financial distress".to_string(),
                    "Natural disaster at supplier location".to_string(),
                    "Geopolitical event".to_string(),
                ],
                probability: context.supply_chain_risk_score,
                impact_severity: 0.8,
                risk_score: context.supply_chain_risk_score * 0.8,
                affected_entities: vec![entity_id.to_string()],
                recommended_responses: vec![
                    "Activate alternative supplier".to_string(),
                    "Use strategic inventory buffer".to_string(),
                ],
                early_warning_indicators: vec![
                    "Supplier financial health decline".to_string(),
                    "Quality issues increase".to_string(),
                    "Delivery delays".to_string(),
                ],
            });
        }

        // Competitive threat scenario
        if context.competitive_intensity > 0.7 {
            scenarios.push(ScenarioSimulation {
                id: Uuid::new_v4().to_string(),
                name: "Aggressive Competitive Move".to_string(),
                scenario_type: ScenarioType::CompetitiveMove,
                description: "Competitor makes aggressive market move (price cut, new product)".to_string(),
                trigger_conditions: vec![
                    "Competitor hiring surge".to_string(),
                    "New capacity announcements".to_string(),
                    "Patent filing activity".to_string(),
                ],
                probability: context.competitive_intensity * 0.5,
                impact_severity: 0.6,
                risk_score: context.competitive_intensity * 0.3,
                affected_entities: vec![entity_id.to_string()],
                recommended_responses: vec![
                    "Accelerate product roadmap".to_string(),
                    "Review pricing strategy".to_string(),
                ],
                early_warning_indicators: vec![
                    "Competitor job postings".to_string(),
                    "Supply chain changes".to_string(),
                ],
            });
        }

        scenarios
    }
}

// ============================================================================
// Conversion layer: apex-threat-intel -> apex-investigation types
// ============================================================================

/// Convert a `threat_intel::ThreatActor` (owned) to an `investigation::ThreatActor`.
#[allow(clippy::disallowed_methods)]
fn convert_threat_actor(actor: &apex_threat_intel::threat_actor_database::ThreatActor) -> ThreatActor {
    let actor_type = convert_actor_motivation(actor.motivation);
    let capabilities: Vec<String> = actor.techniques.iter().map(|t| t.technique_name.clone()).collect();
    let ttps: Vec<String> = actor.techniques.iter().map(|t| {
        format!("{} - {}", t.technique_id, t.technique_name)
    }).collect();
    let industry_focus: Vec<String> = actor.target_sectors.iter().map(|s| s.as_str().to_string()).collect();
    let threat_level = actor.sophistication_level as f64 / 10.0;
    let attribution_confidence = match &actor.risk_score {
        Some(rs) => match rs.confidence {
            apex_threat_intel::models::ConfidenceLevel::High => 0.9,
            apex_threat_intel::models::ConfidenceLevel::Medium => 0.6,
            apex_threat_intel::models::ConfidenceLevel::Low => 0.3,
            apex_threat_intel::models::ConfidenceLevel::Unknown => 0.1,
        },
        None => 0.5,
    };
    let last_observed = actor.last_activity
        .map(|d| {
            let naive = chrono::NaiveDateTime::new(d, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
        })
        .unwrap_or_else(|| {
            let now = Utc::now().date_naive();
            let naive = chrono::NaiveDateTime::new(now, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
        });

    ThreatActor {
        id: actor.id.to_string(),
        name: actor.name.clone().unwrap_or_else(|| actor.alias.clone()),
        actor_type,
        industry_focus,
        geographic_focus: actor.target_regions.clone(),
        capabilities,
        ttps,
        threat_level,
        attribution_confidence,
        last_observed,
        is_active: actor.status == apex_threat_intel::threat_actor_database::ActorStatus::Active,
    }
}

/// Map threat_intel motivation to investigation ThreatActorType.
fn convert_actor_motivation(motivation: apex_threat_intel::threat_actor_database::ActorMotivation) -> ThreatActorType {
    use apex_threat_intel::threat_actor_database::ActorMotivation;
    match motivation {
        ActorMotivation::Espionage => ThreatActorType::NationState,
        ActorMotivation::Financial => ThreatActorType::Criminal,
        ActorMotivation::Hacktivism => ThreatActorType::Hacktivist,
        ActorMotivation::Destruction => ThreatActorType::NationState,
        ActorMotivation::Ideology => ThreatActorType::Hacktivist,
        ActorMotivation::Coercion => ThreatActorType::Criminal,
        ActorMotivation::Personal => ThreatActorType::Insider,
    }
}

/// External asset for attack surface analysis.
#[derive(Debug, Clone)]
pub struct ExternalAsset {
    pub asset_type: String,
    pub name: String,
    pub has_known_vulnerabilities: bool,
    pub has_security_headers: Option<bool>,
    pub is_publicly_accessible: Option<bool>,
}

/// Supplier for supply chain modeling.
#[derive(Debug, Clone)]
pub struct Supplier {
    pub id: String,
    pub name: String,
    pub tier: u8,
    pub country: Option<String>,
    pub financial_score: Option<f64>,
    pub on_time_delivery: Option<f64>,
    pub quality_score: Option<f64>,
    pub is_single_source: bool,
    pub component: String,
}

/// Context for scenario simulation.
#[derive(Debug, Clone)]
pub struct SimulationContext {
    pub supply_chain_risk_score: f64,
    pub competitive_intensity: f64,
    pub financial_health: f64,
    pub geopolitical_exposure: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threat_modeling_creation() {
        let tm = ThreatModeling::new();
        assert!(!tm.threat_actors.is_empty());
        assert!(!tm.adversary_capabilities.is_empty());
    }

    #[test]
    fn test_attack_surface_assessment() {
        let tm = ThreatModeling::new();
        let assets = vec![
            ExternalAsset {
                asset_type: "web".to_string(),
                name: "www.example.com".to_string(),
                has_known_vulnerabilities: true,
                has_security_headers: Some(false),
                is_publicly_accessible: Some(true),
            },
        ];

        let assessment = tm.assess_attack_surface("entity1", &assets);
        assert!(assessment.overall_score > 0.0);
        assert!(!assessment.recommendations.is_empty());
    }

    #[test]
    fn test_scenario_simulation() {
        let tm = ThreatModeling::new();
        let context = SimulationContext {
            supply_chain_risk_score: 0.5,
            competitive_intensity: 0.8,
            financial_health: 0.7,
            geopolitical_exposure: 0.3,
        };

        let scenarios = tm.simulate_scenarios("entity1", &context);
        assert!(!scenarios.is_empty());
    }
}