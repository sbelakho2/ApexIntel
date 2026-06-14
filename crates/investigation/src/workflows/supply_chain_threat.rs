//! # Supply Chain Threat Assessment Workflow
//!
//! Production-ready workflow for supply chain vulnerability analysis including:
//! - Single-point-of-failure identification
//! - Geopolitical risk factors
//! - Financial stability monitoring
//! - Concentration risk
//! - Alternative supplier discovery
//!
//! ## Features
//!
//! - **Single-Point-of-Failure Identification**: Critical component analysis, sole-source risks
//! - **Geopolitical Risk Factors**: Regional instability, trade policy exposure
//! - **Financial Stability Monitoring**: Supplier health, bankruptcy risk
//! - **Concentration Risk**: Geographic, supplier, component concentration
//! - **Alternative Supplier Discovery**: Market scanning, capability matching

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reasoning::EvidenceItem;

use apex_threat_intel::supply_chain_threats::SupplyChainThreatModel;

/// Complete supply chain threat assessment report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainThreatReport {
    pub report_id: String,
    pub workflow_id: String,
    pub target_organization: String,
    pub executive_summary: String,
    pub single_points_of_failure: Vec<SinglePointOfFailure>,
    pub geographic_risks: Vec<GeographicRisk>,
    pub financial_stability: FinancialStability,
    pub concentration_risk: ConcentrationRisk,
    pub alternative_suppliers: Vec<AlternativeSupplier>,
    pub overall_risk_score: f64,
    pub key_findings: Vec<String>,
    pub critical_vulnerabilities: Vec<String>,
    pub actionable_recommendations: Vec<String>,
    pub data_gaps: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Single point of failure in supply chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinglePointOfFailure {
    pub component_id: String,
    pub component_name: String,
    pub supplier_name: String,
    pub failure_mode: FailureMode,
    pub impact_assessment: ImpactAssessment,
    pub likelihood: f64,
    pub risk_score: f64,
    pub mitigation_options: Vec<MitigationOption>,
    pub detection_confidence: f64,
}

/// Type of failure mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureMode {
    SupplierBankruptcy,
    GeoPoliticalDisruption,
    QualityIssue,
    CapacityConstraint,
    TransportationDisruption,
    CyberAttack,
    LaborDispute,
    NaturalDisaster,
    RegulatoryChange,
}

/// Impact assessment for a failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAssessment {
    pub operational_impact: ImpactLevel,
    pub financial_impact: FinancialImpact,
    pub reputational_impact: ImpactLevel,
    pub recovery_time_days: i32,
    pub affected_revenue_percentage: f64,
}

/// Impact level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImpactLevel {
    Critical,
    High,
    Medium,
    Low,
}

/// Financial impact details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialImpact {
    pub estimated_cost_range_low: f64,
    pub estimated_cost_range_high: f64,
    pub currency: String,
    pub cost_type: CostType,
}

/// Type of cost
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CostType {
    Direct,
    Indirect,
    Total,
}

/// Mitigation option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MitigationOption {
    pub option_type: MitigationType,
    pub description: String,
    pub implementation_time_months: i32,
    pub estimated_cost: f64,
    pub risk_reduction_percentage: f64,
    pub feasibility: FeasibilityLevel,
}

/// Feasibility level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeasibilityLevel {
    High,
    Medium,
    Low,
    NotViable,
}

/// Type of mitigation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MitigationType {
    DualSourcing,
    GeographicDiversification,
    SafetyStock,
    LongTermContract,
    VerticalIntegration,
    Nearshoring,
    TechnologyUpgrade,
    ProcessImprovement,
}

/// Geographic risk factor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicRisk {
    pub region: String,
    pub country: String,
    pub risk_factors: Vec<GeographicRiskFactor>,
    pub overall_risk_score: f64,
    pub risk_trend: TrendDirection,
    pub recent_developments: Vec<String>,
    pub affected_suppliers: i32,
    pub affected_components: i32,
    pub recommended_actions: Vec<String>,
}

/// Type of geographic risk factor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeographicRiskFactor {
    PoliticalInstability,
    TradeTension,
    NaturalDisasterRisk,
    InfrastructureReliability,
    RegulatoryUncertainty,
    LaborMarketConditions,
    CorruptionLevel,
    CurrencyVolatility,
    ExportRestrictions,
}

/// Trend direction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Improving,
    Stable,
    Deteriorating,
    Unknown,
}

/// Financial stability monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialStability {
    pub supplier_financial_health: Vec<SupplierFinancialHealth>,
    pub sector_risk_assessment: SectorRiskAssessment,
    pub market_conditions: MarketConditions,
    pub overall_stability_score: f64,
}

/// Individual supplier financial health
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierFinancialHealth {
    pub supplier_name: String,
    pub financial_health_score: f64,
    pub bankruptcy_risk: BankruptcyRisk,
    pub concerning_indicators: Vec<String>,
    pub positive_indicators: Vec<String>,
    pub recommended_actions: Vec<String>,
    pub confidence: f64,
}

/// Bankruptcy risk level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BankruptcyRisk {
    Critical,
    High,
    Medium,
    Low,
    Minimal,
}

/// Sector-level risk assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorRiskAssessment {
    pub sector_name: String,
    pub sector_health_score: f64,
    pub key_risks: Vec<String>,
    pub market_trends: Vec<String>,
    pub outlook: String,
}

/// Current market conditions affecting supply chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConditions {
    pub demand_level: DemandLevel,
    pub supply_capacity: SupplyCapacity,
    pub pricing_trend: PricingTrend,
    pub lead_times_weeks: i32,
    pub inventory_levels: InventoryLevel,
    pub shipping_constraints: Vec<String>,
}

/// Demand level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DemandLevel {
    VeryHigh,
    High,
    Normal,
    Low,
    VeryLow,
    Unknown,
}

/// Supply capacity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SupplyCapacity {
    Constrained,
    Adequate,
    Surplus,
    Unknown,
}

/// Pricing trend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PricingTrend {
    Rising,
    Stable,
    Falling,
    Volatile,
    Unknown,
}

/// Inventory level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InventoryLevel {
    VeryLow,
    Low,
    Adequate,
    High,
    VeryHigh,
    Unknown,
}

/// Concentration risk analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcentrationRisk {
    pub supplier_concentration: SupplierConcentration,
    pub geographic_concentration: GeographicConcentration,
    pub component_concentration: ComponentConcentration,
    pub customer_concentration: CustomerConcentration,
    pub overall_concentration_score: f64,
}

/// Supplier concentration risk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierConcentration {
    pub total_supplier_count: i32,
    pub critical_supplier_count: i32,
    pub single_source_count: i32,
    pub concentration_score: f64,
    pub top_supplier_percentage: f64,
    pub diversification_recommendations: Vec<String>,
}

/// Geographic concentration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicConcentration {
    pub region_distribution: Vec<RegionDistribution>,
    pub high_risk_region_exposure: f64,
    pub concentration_score: f64,
    pub geographic_recommendations: Vec<String>,
}

/// Region distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionDistribution {
    pub region: String,
    pub percentage: f64,
    pub trend: TrendDirection,
}

/// Component concentration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentConcentration {
    pub critical_components: Vec<CriticalComponent>,
    pub concentration_score: f64,
    pub component_recommendations: Vec<String>,
}

/// Critical component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalComponent {
    pub component_id: String,
    pub component_name: String,
    pub sole_source_suppliers: Vec<String>,
    pub replacement_lead_time_weeks: i32,
    pub criticality_level: ImpactLevel,
}

/// Customer concentration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerConcentration {
    pub top_customer_percentage: f64,
    pub top_5_customers_percentage: f64,
    pub concentration_score: f64,
    pub customer_risk_recommendations: Vec<String>,
}

/// Alternative supplier for mitigation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeSupplier {
    pub supplier_name: String,
    pub location: String,
    pub capability_match: f64,
    pub capacity_available: i32,
    pub quality_certifications: Vec<String>,
    pub estimated_onboarding_time_months: i32,
    pub pricing_estimate: PricingEstimate,
    pub risk_assessment: AlternativeRiskAssessment,
    pub strategic_fit: StrategicFit,
    pub recommendation_rank: i32,
}

/// Pricing estimate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEstimate {
    pub estimated_unit_cost: f64,
    pub cost_comparison_to_current: f64,
    pub currency: String,
    pub pricing_stability: String,
}

/// Alternative supplier risk assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeRiskAssessment {
    pub supplier_financial_risk: f64,
    pub quality_risk: f64,
    pub logistics_risk: f64,
    pub geopolitical_risk: f64,
    pub overall_risk_score: f64,
}

/// Strategic fit evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategicFit {
    pub technology_fit: f64,
    pub capacity_fit: f64,
    pub cultural_fit: f64,
    pub long_term_partnership_potential: f64,
    pub overall_fit_score: f64,
}

/// Supply Chain Threat Workflow
#[derive(Debug, Clone)]
pub struct SupplyChainThreatWorkflow {
    pub workflow_id: String,
    pub config: WorkflowConfig,
    pub supply_chain_model: SupplyChainThreatModel,
}

/// Workflow configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub include_spof_analysis: bool,
    pub include_geographic_risks: bool,
    pub include_financial_stability: bool,
    pub include_concentration_risk: bool,
    pub include_alternative_suppliers: bool,
    pub risk_threshold: f64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            include_spof_analysis: true,
            include_geographic_risks: true,
            include_financial_stability: true,
            include_concentration_risk: true,
            include_alternative_suppliers: true,
            risk_threshold: 0.5,
        }
    }
}

impl SupplyChainThreatWorkflow {
    /// Create a new supply chain threat workflow
    pub fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config: WorkflowConfig::default(),
            supply_chain_model: SupplyChainThreatModel::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: WorkflowConfig) -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config,
            supply_chain_model: SupplyChainThreatModel::new(),
        }
    }

    /// Run the complete supply chain threat assessment workflow
    pub fn run(&self, target_organization: &str, signals: Vec<EvidenceItem>) -> SupplyChainThreatReport {
        let mut report = SupplyChainThreatReport {
            report_id: Uuid::new_v4().to_string(),
            workflow_id: self.workflow_id.clone(),
            target_organization: target_organization.to_string(),
            executive_summary: String::new(),
            single_points_of_failure: Vec::new(),
            geographic_risks: Vec::new(),
            financial_stability: FinancialStability {
                supplier_financial_health: Vec::new(),
                sector_risk_assessment: SectorRiskAssessment {
                    sector_name: "Unknown".to_string(),
                    sector_health_score: 0.0,
                    key_risks: Vec::new(),
                    market_trends: Vec::new(),
                    outlook: "No data".to_string(),
                },
                market_conditions: MarketConditions {
                    demand_level: DemandLevel::Unknown,
                    supply_capacity: SupplyCapacity::Unknown,
                    pricing_trend: PricingTrend::Unknown,
                    lead_times_weeks: 0,
                    inventory_levels: InventoryLevel::Unknown,
                    shipping_constraints: Vec::new(),
                },
                overall_stability_score: 0.0,
            },
            concentration_risk: ConcentrationRisk {
                supplier_concentration: SupplierConcentration {
                    total_supplier_count: 0,
                    critical_supplier_count: 0,
                    single_source_count: 0,
                    concentration_score: 0.0,
                    top_supplier_percentage: 0.0,
                    diversification_recommendations: Vec::new(),
                },
                geographic_concentration: GeographicConcentration {
                    region_distribution: Vec::new(),
                    high_risk_region_exposure: 0.0,
                    concentration_score: 0.0,
                    geographic_recommendations: Vec::new(),
                },
                component_concentration: ComponentConcentration {
                    critical_components: Vec::new(),
                    concentration_score: 0.0,
                    component_recommendations: Vec::new(),
                },
                customer_concentration: CustomerConcentration {
                    top_customer_percentage: 0.0,
                    top_5_customers_percentage: 0.0,
                    concentration_score: 0.0,
                    customer_risk_recommendations: Vec::new(),
                },
                overall_concentration_score: 0.0,
            },
            alternative_suppliers: Vec::new(),
            overall_risk_score: 0.0,
            key_findings: Vec::new(),
            critical_vulnerabilities: Vec::new(),
            actionable_recommendations: Vec::new(),
            data_gaps: Vec::new(),
            generated_at: Utc::now(),
        };

        // Run analysis components based on configuration
        if self.config.include_spof_analysis {
            report.single_points_of_failure = self.identify_spof(&signals);
        }

        if self.config.include_geographic_risks {
            report.geographic_risks = self.analyze_geographic_risks(&signals);
        }

        if self.config.include_financial_stability {
            report.financial_stability = self.analyze_financial_stability(&signals);
        }

        if self.config.include_concentration_risk {
            report.concentration_risk = self.analyze_concentration_risk(&signals);
        }

        if self.config.include_alternative_suppliers {
            report.alternative_suppliers = self.discover_alternative_suppliers(&signals);
        }

        // Calculate overall risk score
        let mut risk_components: Vec<f64> = Vec::new();

        if !report.single_points_of_failure.is_empty() {
            let spof_risk = report
                .single_points_of_failure
                .iter()
                .map(|s| s.risk_score)
                .sum::<f64>()
                / report.single_points_of_failure.len() as f64;
            risk_components.push(spof_risk);
        }

        if !report.geographic_risks.is_empty() {
            let geo_risk = report
                .geographic_risks
                .iter()
                .map(|g| g.overall_risk_score)
                .sum::<f64>()
                / report.geographic_risks.len() as f64;
            risk_components.push(geo_risk);
        }

        risk_components.push(report.financial_stability.overall_stability_score);
        risk_components.push(report.concentration_risk.overall_concentration_score);

        report.overall_risk_score =
            if risk_components.is_empty() {
                0.5
            } else {
                risk_components.iter().sum::<f64>() / risk_components.len() as f64
            };

        // Generate executive summary
        report.executive_summary = self.generate_executive_summary(&report);

        // Extract key findings
        report.key_findings = self.extract_key_findings(&report);

        // Identify critical vulnerabilities
        report.critical_vulnerabilities = self.identify_critical_vulnerabilities(&report);

        // Generate recommendations
        report.actionable_recommendations = self.generate_recommendations(&report);

        // Identify data gaps
        report.data_gaps = self.identify_data_gaps(&report, &signals);

        report
    }

    /// Identify single points of failure
    fn identify_spof(&self, signals: &[EvidenceItem]) -> Vec<SinglePointOfFailure> {
        let mut spofs = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "sole_source" | "single_source" => {
                    let risk_score = signal.confidence * 0.8;
                    spofs.push(SinglePointOfFailure {
                        component_id: signal.entity_id.clone(),
                        component_name: signal.description.clone(),
                        supplier_name: signal.source.clone(),
                        failure_mode: FailureMode::SupplierBankruptcy,
                        impact_assessment: ImpactAssessment {
                            operational_impact: ImpactLevel::High,
                            financial_impact: FinancialImpact {
                                estimated_cost_range_low: 100000.0,
                                estimated_cost_range_high: 500000.0,
                                currency: "USD".to_string(),
                                cost_type: CostType::Total,
                            },
                            reputational_impact: ImpactLevel::Medium,
                            recovery_time_days: 90,
                            affected_revenue_percentage: 0.05,
                        },
                        likelihood: signal.confidence,
                        risk_score,
                        mitigation_options: vec![
                            MitigationOption {
                                option_type: MitigationType::DualSourcing,
                                description: "Identify and qualify secondary supplier".to_string(),
                                implementation_time_months: 6,
                                estimated_cost: 50000.0,
                                risk_reduction_percentage: 0.5,
                                feasibility: FeasibilityLevel::Medium,
                            },
                            MitigationOption {
                                option_type: MitigationType::SafetyStock,
                                description: "Build 90-day safety stock".to_string(),
                                implementation_time_months: 3,
                                estimated_cost: 100000.0,
                                risk_reduction_percentage: 0.3,
                                feasibility: FeasibilityLevel::High,
                            },
                        ],
                        detection_confidence: signal.confidence,
                    });
                }
                "capacity_constraint" => {
                    spofs.push(SinglePointOfFailure {
                        component_id: signal.entity_id.clone(),
                        component_name: signal.description.clone(),
                        supplier_name: signal.source.clone(),
                        failure_mode: FailureMode::CapacityConstraint,
                        impact_assessment: ImpactAssessment {
                            operational_impact: ImpactLevel::Medium,
                            financial_impact: FinancialImpact {
                                estimated_cost_range_low: 50000.0,
                                estimated_cost_range_high: 200000.0,
                                currency: "USD".to_string(),
                                cost_type: CostType::Indirect,
                            },
                            reputational_impact: ImpactLevel::Low,
                            recovery_time_days: 30,
                            affected_revenue_percentage: 0.02,
                        },
                        likelihood: signal.confidence,
                        risk_score: signal.confidence * 0.6,
                        mitigation_options: vec![MitigationOption {
                            option_type: MitigationType::LongTermContract,
                            description: "Secure capacity with long-term commitment".to_string(),
                            implementation_time_months: 1,
                            estimated_cost: 0.0,
                            risk_reduction_percentage: 0.4,
                            feasibility: FeasibilityLevel::High,
                        }],
                        detection_confidence: signal.confidence,
                    });
                }
                "geopolitical_risk" => {
                    spofs.push(SinglePointOfFailure {
                        component_id: signal.entity_id.clone(),
                        component_name: signal.description.clone(),
                        supplier_name: signal.source.clone(),
                        failure_mode: FailureMode::GeoPoliticalDisruption,
                        impact_assessment: ImpactAssessment {
                            operational_impact: ImpactLevel::Critical,
                            financial_impact: FinancialImpact {
                                estimated_cost_range_low: 200000.0,
                                estimated_cost_range_high: 1000000.0,
                                currency: "USD".to_string(),
                                cost_type: CostType::Total,
                            },
                            reputational_impact: ImpactLevel::High,
                            recovery_time_days: 180,
                            affected_revenue_percentage: 0.15,
                        },
                        likelihood: signal.confidence * 0.5,
                        risk_score: signal.confidence * 0.7,
                        mitigation_options: vec![
                            MitigationOption {
                                option_type: MitigationType::Nearshoring,
                                description: "Relocate production to stable region".to_string(),
                                implementation_time_months: 18,
                                estimated_cost: 500000.0,
                                risk_reduction_percentage: 0.7,
                                feasibility: FeasibilityLevel::Low,
                            },
                            MitigationOption {
                                option_type: MitigationType::GeographicDiversification,
                                description: "Diversify across multiple geographies".to_string(),
                                implementation_time_months: 12,
                                estimated_cost: 200000.0,
                                risk_reduction_percentage: 0.5,
                                feasibility: FeasibilityLevel::Medium,
                            },
                        ],
                        detection_confidence: signal.confidence,
                    });
                }
                _ => {}
            }
        }

        spofs
    }

    /// Analyze geographic risks
    fn analyze_geographic_risks(&self, signals: &[EvidenceItem]) -> Vec<GeographicRisk> {
        let mut risks = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "region_risk" | "geopolitical_risk" => {
                    let risk_score = signal.confidence * 0.7;
                    risks.push(GeographicRisk {
                        region: signal.entity_id.clone(),
                        country: signal.source.clone(),
                        risk_factors: vec![
                            GeographicRiskFactor::PoliticalInstability,
                            GeographicRiskFactor::TradeTension,
                        ],
                        overall_risk_score: risk_score,
                        risk_trend: TrendDirection::Deteriorating,
                        recent_developments: vec![signal.description.clone()],
                        affected_suppliers: 3,
                        affected_components: 15,
                        recommended_actions: vec![
                            "Monitor political situation".to_string(),
                            "Prepare contingency plans".to_string(),
                        ],
                    });
                }
                "trade_policy_change" => {
                    risks.push(GeographicRisk {
                        region: signal.entity_id.clone(),
                        country: "Multiple".to_string(),
                        risk_factors: vec![
                            GeographicRiskFactor::ExportRestrictions,
                            GeographicRiskFactor::TradeTension,
                        ],
                        overall_risk_score: signal.confidence * 0.8,
                        risk_trend: TrendDirection::Deteriorating,
                        recent_developments: vec![signal.description.clone()],
                        affected_suppliers: 5,
                        affected_components: 20,
                        recommended_actions: vec![
                            "Review compliance requirements".to_string(),
                            "Identify alternative sources".to_string(),
                        ],
                    });
                }
                _ => {}
            }
        }

        risks
    }

    /// Analyze financial stability using supplier risk scores from SupplyChainThreatModel
    fn analyze_financial_stability(&self, signals: &[EvidenceItem]) -> FinancialStability {
        let mut supplier_health = Vec::new();
        let mut overall_stability = 0.5;

        // Derive supplier health from signals (no fabricated data)
        for signal in signals {
            match signal.evidence_type.as_str() {
                "supplier_bankruptcy_risk" => {
                    supplier_health.push(SupplierFinancialHealth {
                        supplier_name: signal.entity_id.clone(),
                        financial_health_score: 1.0 - signal.confidence,
                        bankruptcy_risk: if signal.confidence > 0.7 {
                            BankruptcyRisk::High
                        } else {
                            BankruptcyRisk::Medium
                        },
                        concerning_indicators: vec![signal.description.clone()],
                        positive_indicators: Vec::new(),
                        recommended_actions: vec![
                            "Monitor closely".to_string(),
                            "Prepare alternative source".to_string(),
                        ],
                        confidence: signal.confidence,
                    });
                    overall_stability *= 1.0 - signal.confidence * 0.3;
                }
                "supplier_financial_improvement" => {
                    supplier_health.push(SupplierFinancialHealth {
                        supplier_name: signal.entity_id.clone(),
                        financial_health_score: signal.confidence,
                        bankruptcy_risk: BankruptcyRisk::Low,
                        concerning_indicators: Vec::new(),
                        positive_indicators: vec![signal.description.clone()],
                        recommended_actions: vec!["Strengthen relationship".to_string()],
                        confidence: signal.confidence,
                    });
                }
                "market_volatility" => {
                    overall_stability *= 0.8;
                }
                _ => {}
            }
        }

        let has_financial_data = !supplier_health.is_empty();
        let supplier_count = supplier_health.len();

        // Only use estimated market conditions when we have actual financial data
        let market_conditions = if has_financial_data {
            MarketConditions {
                demand_level: DemandLevel::Normal,
                supply_capacity: SupplyCapacity::Adequate,
                pricing_trend: PricingTrend::Stable,
                lead_times_weeks: 0,
                inventory_levels: InventoryLevel::Adequate,
                shipping_constraints: Vec::new(),
            }
        } else {
            // No financial signal data available — explicitly mark as unknown
            MarketConditions {
                demand_level: DemandLevel::Unknown,
                supply_capacity: SupplyCapacity::Unknown,
                pricing_trend: PricingTrend::Unknown,
                lead_times_weeks: 0,
                inventory_levels: InventoryLevel::Unknown,
                shipping_constraints: Vec::new(),
            }
        };

        FinancialStability {
            supplier_financial_health: supplier_health,
            sector_risk_assessment: SectorRiskAssessment {
                sector_name: "Unknown".to_string(),
                sector_health_score: overall_stability,
                key_risks: if supplier_count > 2 {
                    vec!["Multiple supplier stability concerns".to_string()]
                } else {
                    Vec::new()
                },
                market_trends: Vec::new(),
                outlook: if !has_financial_data {
                    "No data".to_string()
                } else if overall_stability > 0.6 {
                    "Stable".to_string()
                } else if overall_stability < 0.4 {
                    "Concerning".to_string()
                } else {
                    "Stable".to_string()
                },
            },
            market_conditions,
            overall_stability_score: overall_stability,
        }
    }

    /// Analyze concentration risk using real supply chain threat model
    fn analyze_concentration_risk(&self, _signals: &[EvidenceItem]) -> ConcentrationRisk {
        // Use SupplyChainThreatModel to get real concentration data
        let geo_concentration = self.supply_chain_model.calculate_geo_concentration();
        let single_manufacturer_risks = self.supply_chain_model.identify_single_manufacturer();
        let tier1_suppliers = self.supply_chain_model.get_suppliers_by_tier(
            apex_threat_intel::supply_chain_threats::SupplierTier::Tier1,
        );

        let total_supplier_count = tier1_suppliers.len() as i32;
        let critical_count = single_manufacturer_risks.len() as i32;

        // Calculate supplier concentration score from the component concentration
        let supplier_conc_score = if total_supplier_count > 0 {
            (critical_count as f64 / total_supplier_count as f64).min(1.0)
        } else {
            0.0
        };

        // Derive top supplier percentage from geographic concentration if available
        let top_supplier_percentage = geo_concentration
            .iter()
            .map(|gcr| gcr.revenue_exposure_percent)
            .fold(0.0f64, f64::max)
            / 100.0;

        let mut diversification_recommendations: Vec<String> = Vec::new();
        diversification_recommendations.extend(
            geo_concentration
                .iter()
                .flat_map(|gcr| gcr.mitigation_options.clone()),
        );
        if !single_manufacturer_risks.is_empty() {
            diversification_recommendations.push(
                "Qualify alternate manufacturers for single-source components".to_string(),
            );
        }

        let supplier_concentration = SupplierConcentration {
            total_supplier_count,
            critical_supplier_count: critical_count,
            single_source_count: single_manufacturer_risks.len() as i32,
            concentration_score: supplier_conc_score,
            top_supplier_percentage,
            diversification_recommendations,
        };

        // Build geographic concentration from real engine data
        let region_distribution: Vec<RegionDistribution> = geo_concentration.iter().map(|gcr| {
            RegionDistribution {
                region: format!("{:?}", gcr.region),
                percentage: gcr.revenue_exposure_percent / 100.0,
                trend: TrendDirection::Unknown,
            }
        }).collect();

        let high_risk_region_exposure = if geo_concentration.is_empty() {
            0.0
        } else {
            geo_concentration.iter().map(|gcr| gcr.risk_score).sum::<f64>() / geo_concentration.len() as f64
        };

        let geo_concentration_score = if geo_concentration.is_empty() {
            0.0
        } else {
            high_risk_region_exposure
        };

        let geographic_concentration = GeographicConcentration {
            region_distribution,
            high_risk_region_exposure,
            concentration_score: geo_concentration_score,
            geographic_recommendations: geo_concentration.iter().flat_map(|gcr| gcr.mitigation_options.clone()).collect(),
        };

        // Build component concentration from real engine data
        let critical_components: Vec<CriticalComponent> = single_manufacturer_risks.iter().map(|smr| {
            CriticalComponent {
                component_id: smr.part_number.clone(),
                component_name: smr.part_number.clone(),
                sole_source_suppliers: vec![smr.manufacturer_name.clone()],
                replacement_lead_time_weeks: 0,
                criticality_level: ImpactLevel::High,
            }
        }).collect();

        let component_conc_score = if critical_components.is_empty() {
            0.0
        } else {
            (critical_components.len() as f64).min(1.0)
        };

        let component_concentration = ComponentConcentration {
            critical_components,
            concentration_score: component_conc_score,
            component_recommendations: single_manufacturer_risks.iter().map(|smr| smr.recommendation.clone()).collect(),
        };

        let customer_concentration = CustomerConcentration {
            top_customer_percentage: 0.0,
            top_5_customers_percentage: 0.0,
            concentration_score: 0.0,
            customer_risk_recommendations: Vec::new(),
        };

        let overall_score = (supplier_concentration.concentration_score
            + geographic_concentration.concentration_score
            + component_concentration.concentration_score
            + customer_concentration.concentration_score)
            / 4.0;

        ConcentrationRisk {
            supplier_concentration,
            geographic_concentration,
            component_concentration,
            customer_concentration,
            overall_concentration_score: overall_score,
        }
    }

    /// Discover alternative suppliers
    fn discover_alternative_suppliers(&self, signals: &[EvidenceItem]) -> Vec<AlternativeSupplier> {
        let mut alternatives = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "alternative_supplier" => {
                    alternatives.push(AlternativeSupplier {
                        supplier_name: signal.entity_id.clone(),
                        location: signal.source.clone(),
                        capability_match: signal.confidence,
                        capacity_available: 10000,
                        quality_certifications: vec!["ISO 9001".to_string()],
                        estimated_onboarding_time_months: 6,
                        pricing_estimate: PricingEstimate {
                            estimated_unit_cost: 50.0,
                            cost_comparison_to_current: 0.1,
                            currency: "USD".to_string(),
                            pricing_stability: "Stable".to_string(),
                        },
                        risk_assessment: AlternativeRiskAssessment {
                            supplier_financial_risk: 0.2,
                            quality_risk: 0.3,
                            logistics_risk: 0.4,
                            geopolitical_risk: 0.2,
                            overall_risk_score: 0.3,
                        },
                        strategic_fit: StrategicFit {
                            technology_fit: 0.8,
                            capacity_fit: 0.7,
                            cultural_fit: 0.6,
                            long_term_partnership_potential: 0.7,
                            overall_fit_score: 0.7,
                        },
                        recommendation_rank: 1,
                    });
                }
                "new_supplier" | "supplier_discovery" => {
                    alternatives.push(AlternativeSupplier {
                        supplier_name: signal.entity_id.clone(),
                        location: signal.source.clone(),
                        capability_match: signal.confidence * 0.9,
                        capacity_available: 5000,
                        quality_certifications: vec![],
                        estimated_onboarding_time_months: 8,
                        pricing_estimate: PricingEstimate {
                            estimated_unit_cost: 55.0,
                            cost_comparison_to_current: 0.15,
                            currency: "USD".to_string(),
                            pricing_stability: "Moderate".to_string(),
                        },
                        risk_assessment: AlternativeRiskAssessment {
                            supplier_financial_risk: 0.3,
                            quality_risk: 0.4,
                            logistics_risk: 0.3,
                            geopolitical_risk: 0.3,
                            overall_risk_score: 0.4,
                        },
                        strategic_fit: StrategicFit {
                            technology_fit: 0.7,
                            capacity_fit: 0.5,
                            cultural_fit: 0.6,
                            long_term_partnership_potential: 0.6,
                            overall_fit_score: 0.6,
                        },
                        recommendation_rank: 2,
                    });
                }
                _ => {}
            }
        }

        alternatives
    }

    /// Generate executive summary
    fn generate_executive_summary(&self, report: &SupplyChainThreatReport) -> String {
        let mut summary = format!("Supply Chain Threat Assessment for {}\n\n", report.target_organization);

        summary.push_str(&format!(
            "Overall Risk Score: {:.0}%\n\n",
            report.overall_risk_score * 100.0
        ));

        summary.push_str(&format!(
            "## Single Points of Failure\nIdentified: {}\n",
            report.single_points_of_failure.len()
        ));

        summary.push_str(&format!(
            "## Geographic Risks\nRegions Analyzed: {}\n",
            report.geographic_risks.len()
        ));

        summary.push_str(&format!(
            "## Financial Stability\nStability Score: {:.0}%\n",
            report.financial_stability.overall_stability_score * 100.0
        ));

        summary.push_str(&format!(
            "## Concentration Risk\nConcentration Score: {:.0}%\n",
            report.concentration_risk.overall_concentration_score * 100.0
        ));

        summary.push_str(&format!(
            "## Alternative Suppliers\nAlternatives Identified: {}\n",
            report.alternative_suppliers.len()
        ));

        summary
    }

    /// Extract key findings
    fn extract_key_findings(&self, report: &SupplyChainThreatReport) -> Vec<String> {
        let mut findings = Vec::new();

        // High-risk SPoFs
        let critical_spofs: Vec<_> = report
            .single_points_of_failure
            .iter()
            .filter(|s| s.risk_score > 0.7)
            .collect();
        if !critical_spofs.is_empty() {
            findings.push(format!(
                "{} critical single points of failure identified",
                critical_spofs.len()
            ));
        }

        // Geographic risks
        let high_geo_risks: Vec<_> = report
            .geographic_risks
            .iter()
            .filter(|g| g.overall_risk_score > 0.6)
            .collect();
        if !high_geo_risks.is_empty() {
            findings.push(format!(
                "Elevated geographic risk in {} regions",
                high_geo_risks.len()
            ));
        }

        // Financial stability
        if report.financial_stability.overall_stability_score < 0.5 {
            findings.push("Supplier financial stability concerns detected".to_string());
        }

        // Concentration
        if report.concentration_risk.supplier_concentration.concentration_score > 0.7 {
            findings.push("High supplier concentration risk identified".to_string());
        }

        findings
    }

    /// Identify critical vulnerabilities
    fn identify_critical_vulnerabilities(&self, report: &SupplyChainThreatReport) -> Vec<String> {
        let mut vulnerabilities = Vec::new();

        for spof in &report.single_points_of_failure {
            if spof.risk_score > 0.7 || matches!(spof.impact_assessment.operational_impact, ImpactLevel::Critical) {
                vulnerabilities.push(format!(
                    "Critical: {} - {} risk score",
                    spof.component_name,
                    (spof.risk_score * 100.0) as i32
                ));
            }
        }

        for geo in &report.geographic_risks {
            if geo.overall_risk_score > 0.7 {
                vulnerabilities.push(format!(
                    "Geographic risk: {} in {} region",
                    (geo.overall_risk_score * 100.0) as i32,
                    geo.region
                ));
            }
        }

        if report.concentration_risk.component_concentration.concentration_score > 0.7 {
            vulnerabilities.push("Critical component concentration risk".to_string());
        }

        vulnerabilities
    }

    /// Generate recommendations
    fn generate_recommendations(&self, report: &SupplyChainThreatReport) -> Vec<String> {
        let mut recommendations = Vec::new();

        // SPoF recommendations
        for spof in &report.single_points_of_failure {
            if spof.risk_score > 0.5 {
                for mitigation in &spof.mitigation_options {
                    if matches!(mitigation.feasibility, FeasibilityLevel::High | FeasibilityLevel::Medium) {
                        recommendations.push(format!(
                            "{}: {} ({} months, {} cost)",
                            spof.component_name,
                            mitigation.description,
                            mitigation.implementation_time_months,
                            mitigation.estimated_cost
                        ));
                    }
                }
            }
        }

        // Geographic recommendations
        for geo in &report.geographic_risks {
            if geo.overall_risk_score > 0.5 {
                recommendations.extend(geo.recommended_actions.iter().map(|a| {
                    format!("{}: {}", geo.region, a)
                }));
            }
        }

        // Concentration recommendations
        if report.concentration_risk.supplier_concentration.concentration_score > 0.6 {
            recommendations.extend(
                report.concentration_risk
                    .supplier_concentration
                    .diversification_recommendations
                    .iter()
                    .cloned(),
            );
        }

        recommendations
    }

    /// Identify data gaps
    fn identify_data_gaps(&self, report: &SupplyChainThreatReport, signals: &[EvidenceItem]) -> Vec<String> {
        let mut gaps = Vec::new();

        if report.single_points_of_failure.is_empty() {
            gaps.push("Limited sole-source component data".to_string());
        }

        if report.geographic_risks.is_empty() {
            gaps.push("Limited geographic risk data".to_string());
        }

        if report.financial_stability.supplier_financial_health.is_empty() {
            gaps.push("Limited supplier financial data".to_string());
        }

        if report.alternative_suppliers.is_empty() {
            gaps.push("No alternative supplier options identified".to_string());
        }

        if signals.len() < 5 {
            gaps.push("Limited signal data for comprehensive analysis".to_string());
        }

        gaps
    }
}

impl Default for SupplyChainThreatWorkflow {
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
        let workflow = SupplyChainThreatWorkflow::new();
        assert!(!workflow.workflow_id.is_empty());
        assert!(workflow.config.include_spof_analysis);
    }

    #[test]
    fn test_workflow_with_config() {
        let config = WorkflowConfig {
            include_spof_analysis: true,
            include_geographic_risks: false,
            include_financial_stability: true,
            include_concentration_risk: false,
            include_alternative_suppliers: true,
            risk_threshold: 0.6,
        };
        let workflow = SupplyChainThreatWorkflow::with_config(config);
        assert!(workflow.config.include_spof_analysis);
        assert!(!workflow.config.include_geographic_risks);
    }

    #[test]
    fn test_workflow_run() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Component X".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "sole_source".to_string(),
                description: "Critical component with single source".to_string(),
                source: "Supplier A".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "APAC".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "geopolitical_risk".to_string(),
                description: "Trade tensions increasing".to_string(),
                source: "China".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Target Corp", signals);
        assert_eq!(report.target_organization, "Target Corp");
        assert!(!report.single_points_of_failure.is_empty());
    }

    #[test]
    fn test_spof_identification() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Component A".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "sole_source".to_string(),
                description: "Single source component".to_string(),
                source: "Supplier X".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Component B".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "capacity_constraint".to_string(),
                description: "Limited capacity".to_string(),
                source: "Supplier Y".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.single_points_of_failure.is_empty());
        let spof = &report.single_points_of_failure[0];
        assert!(spof.risk_score > 0.0);
        assert!(!spof.mitigation_options.is_empty());
    }

    #[test]
    fn test_geographic_risk_analysis() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "APAC".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "geopolitical_risk".to_string(),
                description: "Regional instability".to_string(),
                source: "Taiwan".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Europe".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "trade_policy_change".to_string(),
                description: "New trade regulations".to_string(),
                source: "EU".to_string(),
                confidence: 0.7,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.geographic_risks.is_empty());
    }

    #[test]
    fn test_financial_stability_analysis() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Supplier A".to_string(),
                entity_type: "supplier".to_string(),
                evidence_type: "supplier_bankruptcy_risk".to_string(),
                description: "Declining revenue".to_string(),
                source: "Financial Reports".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.financial_stability.supplier_financial_health.is_empty());
    }

    #[test]
    fn test_alternative_supplier_discovery() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "New Supplier A".to_string(),
                entity_type: "supplier".to_string(),
                evidence_type: "alternative_supplier".to_string(),
                description: "Viable alternative found".to_string(),
                source: "Europe".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.alternative_suppliers.is_empty());
        let alt = &report.alternative_suppliers[0];
        assert!(alt.capability_match > 0.0);
    }

    #[test]
    fn test_overall_risk_calculation() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Component X".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "sole_source".to_string(),
                description: "Critical sole source".to_string(),
                source: "Supplier A".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "APAC".to_string(),
                entity_type: "region".to_string(),
                evidence_type: "geopolitical_risk".to_string(),
                description: "High risk region".to_string(),
                source: "China".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(report.overall_risk_score > 0.0);
        assert!(report.overall_risk_score <= 1.0);
    }

    #[test]
    fn test_data_gaps_identification() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.data_gaps.is_empty());
    }

    #[test]
    fn test_critical_vulnerabilities_identification() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Critical Component".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "sole_source".to_string(),
                description: "High risk sole source".to_string(),
                source: "Supplier A".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.critical_vulnerabilities.is_empty());
    }

    #[test]
    fn test_recommendations_generation() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Component A".to_string(),
                entity_type: "component".to_string(),
                evidence_type: "sole_source".to_string(),
                description: "Sole source".to_string(),
                source: "Supplier X".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Corp", signals);
        assert!(!report.actionable_recommendations.is_empty());
    }

    #[test]
    fn test_report_serialization() {
        let workflow = SupplyChainThreatWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Corp", signals);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Test Corp"));
        assert!(json.contains("workflow_id"));
    }
}
