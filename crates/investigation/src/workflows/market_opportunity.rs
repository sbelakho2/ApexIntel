//! # Market Opportunity Detection Workflow
//!
//! Production-ready workflow for market opportunity analysis including:
//! - Procurement signal aggregation
//! - Competitor weakness identification
//! - Market timing analysis
//! - Strategic entry points
//!
//! ## Features
//!
//! - **Procurement Signal Aggregation**: Public tenders, contract opportunities, demand signals
//! - **Competitor Weakness Identification**: Market share erosion, product gaps, service failures
//! - **Market Timing Analysis**: Economic indicators, seasonal patterns, lifecycle positioning
//! - **Strategic Entry Points**: Acquisition targets, partnerships, whitespace opportunities

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reasoning::EvidenceItem;

use apex_threat_intel::competitive_intelligence::CompetitiveIntelligenceEngine;

/// Complete market opportunity report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOpportunityReport {
    pub report_id: String,
    pub workflow_id: String,
    pub target_market: String,
    pub executive_summary: String,
    pub procurement_signals: Vec<ProcurementSignal>,
    pub competitor_weaknesses: Vec<CompetitorWeakness>,
    pub market_timing: MarketTiming,
    pub strategic_entry_points: Vec<StrategicEntryPoint>,
    pub overall_opportunity_score: f64,
    pub key_opportunities: Vec<String>,
    pub recommended_priorities: Vec<String>,
    pub action_plan: Vec<String>,
    pub data_gaps: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Procurement signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcurementSignal {
    pub signal_id: String,
    pub signal_type: ProcurementSignalType,
    pub title: String,
    pub description: String,
    pub organization: String,
    pub estimated_value: Option<f64>,
    pub currency: String,
    pub deadline: Option<DateTime<Utc>>,
    pub source: String,
    pub confidence: f64,
    pub relevance_score: f64,
    pub competition_level: CompetitionLevel,
}

/// Type of procurement signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcurementSignalType {
    PublicTender,
    RfpRfq,
    ContractRenewal,
    NewRequirement,
    BudgetIncrease,
    EmergencyProcurement,
    ExpansionProject,
    GovernmentContract,
}

/// Competition level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompetitionLevel {
    High,
    Medium,
    Low,
    Unknown,
}

/// Competitor weakness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorWeakness {
    pub competitor_name: String,
    pub weakness_type: WeaknessType,
    pub description: String,
    pub severity: WeaknessSeverity,
    pub opportunity_description: String,
    pub estimated_capture_potential: f64,
    pub confidence: f64,
    pub time_sensitivity: TimeSensitivity,
}

/// Type of competitor weakness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WeaknessType {
    ProductGap,
    ServiceFailure,
    PricingIssue,
    TechnologyGap,
    CustomerDissatisfaction,
    DeliveryProblems,
    QualityIssues,
    StaffTurnover,
    FinancialStress,
    RegulatoryIssues,
}

/// Weakness severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WeaknessSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Time sensitivity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeSensitivity {
    Urgent,
    NearTerm,
    MediumTerm,
    LongTerm,
}

/// Market timing analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketTiming {
    pub economic_indicators: EconomicIndicators,
    pub seasonal_factors: SeasonalFactors,
    pub lifecycle_position: LifecyclePosition,
    pub market_momentum: MarketMomentum,
    pub overall_timing_score: f64,
    pub timing_recommendation: String,
}

/// Economic indicators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicIndicators {
    pub gdp_growth: f64,
    pub industry_growth: f64,
    pub consumer_confidence: f64,
    pub unemployment_rate: f64,
    pub inflation_rate: f64,
    pub interest_rates: f64,
    pub overall_economic_score: f64,
}

/// Seasonal factors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonalFactors {
    pub buying_season: Option<BuyingSeason>,
    pub fiscal_year_impact: Option<String>,
    pub event_driven_demand: Vec<String>,
    pub historical_pattern: String,
}

/// Buying season
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuyingSeason {
    Q1Push,
    Q2Preparation,
    Q3Steady,
    Q4YearEnd,
}

/// Lifecycle position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecyclePosition {
    pub lifecycle_stage: LifecycleStage,
    pub growth_rate: f64,
    pub market_saturation: f64,
    pub innovation_frequency: String,
}

/// Lifecycle stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifecycleStage {
    Emerging,
    Growth,
    Mature,
    Declining,
    /// Used when insufficient data is available to determine the stage
    Unknown,
}

/// Market momentum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMomentum {
    pub trend_direction: TrendDirection,
    pub momentum_score: f64,
    pub acceleration_rate: f64,
    pub volatility: f64,
    pub forecast: String,
}

/// Trend direction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    StrongGrowth,
    Growth,
    Stable,
    Decline,
    Volatile,
}

/// Strategic entry point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategicEntryPoint {
    pub entry_type: EntryType,
    pub target: String,
    pub description: String,
    pub investment_requirement: Option<f64>,
    pub expected_return: Option<f64>,
    pub risk_level: RiskLevel,
    pub time_to_value_months: i32,
    pub strategic_fit: f64,
    pub feasibility: FeasibilityLevel,
    pub recommendation_rank: i32,
}

/// Type of entry point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntryType {
    NewProductLaunch,
    MarketExpansion,
    AcquisitionTarget,
    PartnershipOpportunity,
    WhitespaceEntry,
    DisruptiveInnovation,
}

impl EntryType {
    pub fn description(&self) -> &'static str {
        match self {
            EntryType::NewProductLaunch => "New Product Launch",
            EntryType::MarketExpansion => "Market Expansion",
            EntryType::AcquisitionTarget => "Acquisition Target",
            EntryType::PartnershipOpportunity => "Partnership Opportunity",
            EntryType::WhitespaceEntry => "Whitespace Entry",
            EntryType::DisruptiveInnovation => "Disruptive Innovation",
        }
    }
}

/// Risk level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskLevel {
    VeryHigh,
    High,
    Medium,
    Low,
    VeryLow,
}

/// Feasibility level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeasibilityLevel {
    HighlyFeasible,
    Feasible,
    Challenging,
    Difficult,
}


impl WeaknessType {
    pub fn description(&self) -> &'static str {
        match self {
            WeaknessType::ProductGap => "Product Gap",
            WeaknessType::ServiceFailure => "Service Failure",
            WeaknessType::PricingIssue => "Pricing Issue",
            WeaknessType::TechnologyGap => "Technology Gap",
            WeaknessType::CustomerDissatisfaction => "Customer Dissatisfaction",
            WeaknessType::DeliveryProblems => "Delivery Problems",
            WeaknessType::QualityIssues => "Quality Issues",
            WeaknessType::StaffTurnover => "Staff Turnover",
            WeaknessType::FinancialStress => "Financial Stress",
            WeaknessType::RegulatoryIssues => "Regulatory Issues",
        }
    }
}

/// Market Opportunity Workflow
#[derive(Debug, Clone)]
pub struct MarketOpportunityWorkflow {
    pub workflow_id: String,
    pub config: WorkflowConfig,
    pub competitive_engine: CompetitiveIntelligenceEngine,
}

/// Workflow configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub include_procurement: bool,
    pub include_competitor_analysis: bool,
    pub include_timing_analysis: bool,
    pub include_entry_points: bool,
    pub opportunity_threshold: f64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            include_procurement: true,
            include_competitor_analysis: true,
            include_timing_analysis: true,
            include_entry_points: true,
            opportunity_threshold: 0.5,
        }
    }
}

impl MarketOpportunityWorkflow {
    /// Create a new market opportunity workflow
    pub fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config: WorkflowConfig::default(),
            competitive_engine: CompetitiveIntelligenceEngine::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: WorkflowConfig) -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            config,
            competitive_engine: CompetitiveIntelligenceEngine::new(),
        }
    }

    /// Run the complete market opportunity detection workflow
    pub fn run(&self, target_market: &str, signals: Vec<EvidenceItem>) -> MarketOpportunityReport {
        let mut report = MarketOpportunityReport {
            report_id: Uuid::new_v4().to_string(),
            workflow_id: self.workflow_id.clone(),
            target_market: target_market.to_string(),
            executive_summary: String::new(),
            procurement_signals: Vec::new(),
            competitor_weaknesses: Vec::new(),
            market_timing: MarketTiming {
                economic_indicators: EconomicIndicators {
                    gdp_growth: 0.0,
                    industry_growth: 0.0,
                    consumer_confidence: 0.0,
                    unemployment_rate: 0.0,
                    inflation_rate: 0.0,
                    interest_rates: 0.0,
                    overall_economic_score: 0.5,
                },
                seasonal_factors: SeasonalFactors {
                    buying_season: None,
                    fiscal_year_impact: None,
                    event_driven_demand: Vec::new(),
                    historical_pattern: String::new(),
                },
                lifecycle_position: LifecyclePosition {
                    lifecycle_stage: LifecycleStage::Unknown,
                    growth_rate: 0.0,
                    market_saturation: 0.0,
                    innovation_frequency: String::new(),
                },
                market_momentum: MarketMomentum {
                    trend_direction: TrendDirection::Stable,
                    momentum_score: 0.5,
                    acceleration_rate: 0.0,
                    volatility: 0.0,
                    forecast: String::new(),
                },
                overall_timing_score: 0.5,
                timing_recommendation: String::new(),
            },
            strategic_entry_points: Vec::new(),
            overall_opportunity_score: 0.0,
            key_opportunities: Vec::new(),
            recommended_priorities: Vec::new(),
            action_plan: Vec::new(),
            data_gaps: Vec::new(),
            generated_at: Utc::now(),
        };

        // Run analysis components based on configuration
        if self.config.include_procurement {
            report.procurement_signals = self.aggregate_procurement_signals(&signals);
        }

        if self.config.include_competitor_analysis {
            report.competitor_weaknesses = self.identify_competitor_weaknesses(&signals);
        }

        if self.config.include_timing_analysis {
            report.market_timing = self.analyze_market_timing(&signals);
        }

        if self.config.include_entry_points {
            report.strategic_entry_points = self.identify_strategic_entry_points(&report);
        }

        // Calculate overall opportunity score
        let mut score_components: Vec<f64> = Vec::new();

        if !report.procurement_signals.is_empty() {
            let procurement_score = report
                .procurement_signals
                .iter()
                .map(|p| p.relevance_score * p.confidence)
                .sum::<f64>()
                / report.procurement_signals.len() as f64;
            score_components.push(procurement_score);
        }

        if !report.competitor_weaknesses.is_empty() {
            let competitor_score = report
                .competitor_weaknesses
                .iter()
                .map(|c| c.estimated_capture_potential * c.confidence)
                .sum::<f64>()
                / report.competitor_weaknesses.len() as f64;
            score_components.push(competitor_score);
        }

        score_components.push(report.market_timing.overall_timing_score);

        if !report.strategic_entry_points.is_empty() {
            let entry_score = report
                .strategic_entry_points
                .iter()
                .map(|e| e.strategic_fit)
                .sum::<f64>()
                / report.strategic_entry_points.len() as f64;
            score_components.push(entry_score);
        }

        report.overall_opportunity_score = if score_components.is_empty() {
            0.5
        } else {
            score_components.iter().sum::<f64>() / score_components.len() as f64
        };

        // Generate executive summary
        report.executive_summary = self.generate_executive_summary(&report);

        // Extract key opportunities
        report.key_opportunities = self.extract_key_opportunities(&report);

        // Generate recommendations
        report.recommended_priorities = self.generate_recommendations(&report);

        // Generate action plan
        report.action_plan = self.generate_action_plan(&report);

        // Identify data gaps
        report.data_gaps = self.identify_data_gaps(&report, &signals);

        report
    }

    /// Extract an estimated monetary value from signal `raw_data`, if present.
    fn extract_value_from_raw_data(raw_data: &serde_json::Value) -> Option<f64> {
        // Try common field names used across different signal sources
        raw_data
            .get("estimated_value")
            .and_then(|v| v.as_f64())
            .or_else(|| raw_data.get("value").and_then(|v| v.as_f64()))
            .or_else(|| raw_data.get("amount").and_then(|v| v.as_f64()))
            .or_else(|| raw_data.get("contract_value").and_then(|v| v.as_f64()))
            .or_else(|| raw_data.get("budget").and_then(|v| v.as_f64()))
    }

    /// Aggregate procurement signals
    fn aggregate_procurement_signals(&self, signals: &[EvidenceItem]) -> Vec<ProcurementSignal> {
        let mut procurement_signals = Vec::new();

        for signal in signals {
            let estimated_value = Self::extract_value_from_raw_data(&signal.raw_data);

            match signal.evidence_type.as_str() {
                "public_tender" | "tender" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::PublicTender,
                        title: signal.description.clone(),
                        description: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence * 0.9,
                        competition_level: CompetitionLevel::Medium,
                    });
                }
                "rfp" | "rfq" | "procurement" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::RfpRfq,
                        title: signal.description.clone(),
                        description: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence,
                        competition_level: CompetitionLevel::High,
                    });
                }
                "contract_renewal" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::ContractRenewal,
                        title: signal.description.clone(),
                        description: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence * 0.8,
                        competition_level: CompetitionLevel::Low,
                    });
                }
                "expansion_project" | "new_requirement" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::ExpansionProject,
                        title: signal.description.clone(),
                        description: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence,
                        competition_level: CompetitionLevel::Medium,
                    });
                }
                "budget_increase" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::BudgetIncrease,
                        title: signal.description.clone(),
                        description: signal.description.clone(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence * 0.7,
                        competition_level: CompetitionLevel::Unknown,
                    });
                }
                "emergency_procurement" => {
                    procurement_signals.push(ProcurementSignal {
                        signal_id: signal.id.clone(),
                        signal_type: ProcurementSignalType::EmergencyProcurement,
                        title: signal.description.clone(),
                        description: "Urgent procurement opportunity".to_string(),
                        organization: signal.entity_id.clone(),
                        estimated_value,
                        currency: "USD".to_string(),
                        deadline: Some(signal.timestamp),
                        source: signal.source.clone(),
                        confidence: signal.confidence,
                        relevance_score: signal.confidence,
                        competition_level: CompetitionLevel::Low,
                    });
                }
                _ => {}
            }
        }

        procurement_signals
    }

    /// Identify competitor weaknesses
    fn identify_competitor_weaknesses(&self, signals: &[EvidenceItem]) -> Vec<CompetitorWeakness> {
        let mut weaknesses = Vec::new();

        for signal in signals {
            match signal.evidence_type.as_str() {
                "competitor_product_gap" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::ProductGap,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::High,
                        opportunity_description: "Unmet customer need".to_string(),
                        estimated_capture_potential: 0.7,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::NearTerm,
                    });
                }
                "competitor_service_failure" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::ServiceFailure,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::Medium,
                        opportunity_description: "Service quality gap".to_string(),
                        estimated_capture_potential: 0.5,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::Urgent,
                    });
                }
                "competitor_pricing" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::PricingIssue,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::High,
                        opportunity_description: "Price-sensitive customer segment".to_string(),
                        estimated_capture_potential: 0.6,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::MediumTerm,
                    });
                }
                "competitor_technology_gap" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::TechnologyGap,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::Medium,
                        opportunity_description: "Technology-based differentiation".to_string(),
                        estimated_capture_potential: 0.8,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::LongTerm,
                    });
                }
                "competitor_customer_complaint" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::CustomerDissatisfaction,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::High,
                        opportunity_description: "Disgruntled customer base".to_string(),
                        estimated_capture_potential: 0.7,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::Urgent,
                    });
                }
                "competitor_quality_issue" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::QualityIssues,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::High,
                        opportunity_description: "Quality-conscious customers".to_string(),
                        estimated_capture_potential: 0.6,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::NearTerm,
                    });
                }
                "competitor_financial_stress" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::FinancialStress,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::Critical,
                        opportunity_description: "Competitor capacity reduction".to_string(),
                        estimated_capture_potential: 0.9,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::Urgent,
                    });
                }
                "competitor_regulatory_issue" => {
                    weaknesses.push(CompetitorWeakness {
                        competitor_name: signal.entity_id.clone(),
                        weakness_type: WeaknessType::RegulatoryIssues,
                        description: signal.description.clone(),
                        severity: WeaknessSeverity::High,
                        opportunity_description: "Compliance vacuum".to_string(),
                        estimated_capture_potential: 0.75,
                        confidence: signal.confidence,
                        time_sensitivity: TimeSensitivity::NearTerm,
                    });
                }
                _ => {}
            }
        }

        weaknesses
    }

    /// Analyze market timing using CompetitiveIntelligenceEngine for lifecycle data
    fn analyze_market_timing(&self, signals: &[EvidenceItem]) -> MarketTiming {
        // Collect signal-derived lifecycle indicators before building defaults
        let mut signal_derived_lifecycle: Option<LifecycleStage> = None;
        let mut signal_derived_growth_rate: f64 = 0.0;
        let mut signal_derived_trend: Option<TrendDirection> = None;
        let mut signal_derived_volatility: f64 = 0.0;

        // Accumulate economic signal contributions
        let mut gdp_from_signals: f64 = 0.0;
        let mut industry_growth_from_signals: f64 = 0.0;
        let mut momentum_from_signals: f64 = 0.0;

        for signal in signals {
            match signal.evidence_type.as_str() {
                "market_growth" | "industry_growth" => {
                    industry_growth_from_signals = signal.confidence * 0.1;
                    momentum_from_signals = signal.confidence;
                }
                "emerging_market" => {
                    signal_derived_lifecycle = Some(LifecycleStage::Emerging);
                    signal_derived_growth_rate = signal.confidence;
                }
                "declining_market" => {
                    signal_derived_lifecycle = Some(LifecycleStage::Declining);
                    signal_derived_trend = Some(TrendDirection::Decline);
                }
                "seasonal_demand" => {
                    signal_derived_volatility = signal.confidence * 0.3;
                }
                "economic_boom" => {
                    gdp_from_signals = signal.confidence;
                    signal_derived_trend = Some(TrendDirection::StrongGrowth);
                }
                "recession_risk" => {
                    signal_derived_trend = Some(TrendDirection::Decline);
                    momentum_from_signals *= 0.7;
                }
                "market_disruption" => {
                    signal_derived_volatility = 0.5;
                }
                _ => {}
            }
        }

        // Build EconomicIndicators from signal data; fall back to 0.0 (f64, not Option)
        let economic_indicators = EconomicIndicators {
            gdp_growth: gdp_from_signals,
            industry_growth: industry_growth_from_signals,
            consumer_confidence: 0.0,
            unemployment_rate: 0.0,
            inflation_rate: 0.0,
            interest_rates: 0.0,
            overall_economic_score: momentum_from_signals,
        };

        // Derive lifecycle position – prefer signal data, then competitive engine, else Unknown
        let tech_positioning = self.competitive_engine.analyze_technology_positioning();
        let lifecycle_stage = signal_derived_lifecycle.unwrap_or_else(|| {
            if tech_positioning.is_empty() {
                LifecycleStage::Unknown // no data available
            } else {
                // Derive from average technology maturity
                let has_emerging = tech_positioning.iter().any(|tp| matches!(
                    tp.maturity,
                    apex_threat_intel::competitive_intelligence::TechnologyMaturity::Research
                        | apex_threat_intel::competitive_intelligence::TechnologyMaturity::Development
                        | apex_threat_intel::competitive_intelligence::TechnologyMaturity::EarlyStage
                ));
                let has_growth = tech_positioning.iter().any(|tp| matches!(
                    tp.maturity,
                    apex_threat_intel::competitive_intelligence::TechnologyMaturity::Growth
                ));
                let has_legacy = tech_positioning.iter().any(|tp| matches!(
                    tp.maturity,
                    apex_threat_intel::competitive_intelligence::TechnologyMaturity::Legacy
                ));
                if has_emerging {
                    LifecycleStage::Emerging
                } else if has_growth {
                    LifecycleStage::Growth
                } else if has_legacy {
                    LifecycleStage::Declining
                } else {
                    LifecycleStage::Mature
                }
            }
        });

        let lifecycle_position = LifecyclePosition {
            lifecycle_stage,
            growth_rate: signal_derived_growth_rate,
            market_saturation: 0.0,
            innovation_frequency: String::new(),
        };

        let trend_direction = signal_derived_trend.unwrap_or(TrendDirection::Stable);
        let forecast = match trend_direction {
            TrendDirection::StrongGrowth => "Strong growth forecast".to_string(),
            TrendDirection::Growth => "Moderate growth expected".to_string(),
            TrendDirection::Stable => "Stable market conditions".to_string(),
            TrendDirection::Decline => "Market contraction expected".to_string(),
            TrendDirection::Volatile => "Volatile market conditions".to_string(),
        };
        let market_momentum = MarketMomentum {
            trend_direction,
            momentum_score: momentum_from_signals.max(0.5),
            acceleration_rate: 0.0,
            volatility: signal_derived_volatility,
            forecast,
        };

        // Calculate overall timing score
        let overall_economic = economic_indicators.overall_economic_score;
        let timing_score = (overall_economic + market_momentum.momentum_score) / 2.0;

        let timing_recommendation = if timing_score > 0.7 {
            "Highly favorable timing for market entry".to_string()
        } else if timing_score > 0.5 {
            "Moderate timing - consider tactical entry".to_string()
        } else {
            "Challenging timing - focus on preparation".to_string()
        };

        MarketTiming {
            economic_indicators,
            seasonal_factors: SeasonalFactors {
                buying_season: None,
                fiscal_year_impact: None,
                event_driven_demand: Vec::new(),
                historical_pattern: String::new(),
            },
            lifecycle_position,
            market_momentum,
            overall_timing_score: timing_score,
            timing_recommendation,
        }
    }

    /// Identify strategic entry points
    fn identify_strategic_entry_points(&self, report: &MarketOpportunityReport) -> Vec<StrategicEntryPoint> {
        let mut entry_points = Vec::new();
        let mut rank = 1;

        // Generate entry points from procurement signals
        for signal in &report.procurement_signals {
            if signal.relevance_score > 0.6 {
                entry_points.push(StrategicEntryPoint {
                    entry_type: EntryType::NewProductLaunch,
                    target: signal.organization.clone(),
                    description: format!(
                        "Opportunity with {} - {}",
                        signal.organization, signal.title
                    ),
                    investment_requirement: signal.estimated_value,
                    expected_return: signal.estimated_value.map(|v| v * 0.3),
                    risk_level: RiskLevel::Medium,
                    time_to_value_months: 3,
                    strategic_fit: signal.relevance_score,
                    feasibility: FeasibilityLevel::Feasible,
                    recommendation_rank: rank,
                });
                rank += 1;
            }
        }

        // Generate entry points from competitor weaknesses
        for weakness in &report.competitor_weaknesses {
            if weakness.estimated_capture_potential > 0.6 {
                entry_points.push(StrategicEntryPoint {
                    entry_type: EntryType::WhitespaceEntry,
                    target: weakness.competitor_name.clone(),
                    description: format!(
                        "Target {} customers affected by {}",
                        weakness.competitor_name, weakness.weakness_type.description()
                    ),
                    investment_requirement: None,
                    expected_return: None,
                    risk_level: if matches!(weakness.severity, WeaknessSeverity::Critical | WeaknessSeverity::High) {
                        RiskLevel::Low
                    } else {
                        RiskLevel::Medium
                    },
                    time_to_value_months: 6,
                    strategic_fit: weakness.estimated_capture_potential,
                    feasibility: FeasibilityLevel::Feasible,
                    recommendation_rank: rank,
                });
                rank += 1;
            }
        }

        // Add acquisition opportunities if market timing is favorable
        if report.market_timing.overall_timing_score > 0.6 {
            entry_points.push(StrategicEntryPoint {
                entry_type: EntryType::AcquisitionTarget,
                target: "Market Consolidation".to_string(),
                description: "Favorable conditions for strategic acquisitions".to_string(),
                investment_requirement: None,
                expected_return: None,
                risk_level: RiskLevel::High,
                time_to_value_months: 12,
                strategic_fit: 0.8,
                feasibility: FeasibilityLevel::Challenging,
                recommendation_rank: rank,
            });
        }

        entry_points.sort_by(|a, b| b.strategic_fit.partial_cmp(&a.strategic_fit).unwrap_or(std::cmp::Ordering::Equal));
        for (i, point) in entry_points.iter_mut().enumerate() {
            point.recommendation_rank = (i + 1) as i32;
        }

        entry_points
    }

    /// Generate executive summary
    fn generate_executive_summary(&self, report: &MarketOpportunityReport) -> String {
        let mut summary = format!("Market Opportunity Report for {}\n\n", report.target_market);

        summary.push_str(&format!(
            "Overall Opportunity Score: {:.0}%\n\n",
            report.overall_opportunity_score * 100.0
        ));

        summary.push_str(&format!(
            "## Procurement Signals\nOpportunities: {}\n",
            report.procurement_signals.len()
        ));

        let high_value_procurement: Vec<_> = report
            .procurement_signals
            .iter()
            .filter(|p| p.estimated_value.is_some())
            .collect();
        if !high_value_procurement.is_empty() {
            summary.push_str(&format!(
                "High-Value Targets: {}\n",
                high_value_procurement.len()
            ));
        }

        summary.push_str(&format!(
            "## Competitor Weaknesses\nIdentified: {}\n",
            report.competitor_weaknesses.len()
        ));

        let urgent_weaknesses: Vec<_> = report
            .competitor_weaknesses
            .iter()
            .filter(|w| matches!(w.time_sensitivity, TimeSensitivity::Urgent))
            .collect();
        if !urgent_weaknesses.is_empty() {
            summary.push_str(&format!(
                "Urgent Opportunities: {}\n",
                urgent_weaknesses.len()
            ));
        }

        summary.push_str(&format!(
            "## Market Timing\nTiming Score: {:.0}%\n",
            report.market_timing.overall_timing_score * 100.0
        ));
        summary.push_str(&format!(
            "Recommendation: {}\n",
            report.market_timing.timing_recommendation
        ));

        summary.push_str(&format!(
            "## Strategic Entry Points\nIdentified: {}\n",
            report.strategic_entry_points.len()
        ));

        summary
    }

    /// Extract key opportunities
    fn extract_key_opportunities(&self, report: &MarketOpportunityReport) -> Vec<String> {
        let mut opportunities = Vec::new();

        // High-value procurement opportunities
        let high_value: Vec<_> = report
            .procurement_signals
            .iter()
            .filter(|p| p.estimated_value.map(|v| v > 100000.0).unwrap_or(false))
            .collect();
        if !high_value.is_empty() {
            opportunities.push(format!(
                "{} high-value procurement opportunities identified",
                high_value.len()
            ));
        }

        // Critical competitor weaknesses
        let critical_weaknesses: Vec<_> = report
            .competitor_weaknesses
            .iter()
            .filter(|w| matches!(w.severity, WeaknessSeverity::Critical | WeaknessSeverity::High))
            .collect();
        if !critical_weaknesses.is_empty() {
            opportunities.push(format!(
                "{} critical competitor weaknesses identified",
                critical_weaknesses.len()
            ));
        }

        // Favorable timing
        if report.market_timing.overall_timing_score > 0.7 {
            opportunities.push("Favorable market timing for aggressive entry".to_string());
        }

        // High-potential entry points
        let high_fit_entries: Vec<_> = report
            .strategic_entry_points
            .iter()
            .filter(|e| e.strategic_fit > 0.7)
            .collect();
        if !high_fit_entries.is_empty() {
            opportunities.push(format!(
                "{} high-strategic-fit entry points identified",
                high_fit_entries.len()
            ));
        }

        opportunities
    }

    /// Generate recommendations
    fn generate_recommendations(&self, report: &MarketOpportunityReport) -> Vec<String> {
        let mut recommendations = Vec::new();

        // Procurement recommendations
        let urgent_procurement: Vec<_> = report
            .procurement_signals
            .iter()
            .filter(|p| matches!(p.competition_level, CompetitionLevel::Low))
            .collect();
        if !urgent_procurement.is_empty() {
            recommendations.push(
                "Priority: Low-competition procurement opportunities available".to_string(),
            );
        }

        // Competitor recommendations
        let urgent_weaknesses: Vec<_> = report
            .competitor_weaknesses
            .iter()
            .filter(|w| matches!(w.time_sensitivity, TimeSensitivity::Urgent))
            .collect();
        if !urgent_weaknesses.is_empty() {
            recommendations.push(
                "Immediate action: Target customers affected by urgent competitor issues".to_string(),
            );
        }

        // Timing recommendations
        if report.market_timing.overall_timing_score > 0.6 {
            recommendations.push(
                "Timing is favorable - accelerate market entry plans".to_string(),
            );
        } else {
            recommendations.push(
                "Timing is challenging - focus on preparation and positioning".to_string(),
            );
        }

        // Entry point recommendations
        if let Some(top_entry) = report.strategic_entry_points.first() {
            recommendations.push(format!(
                "Top priority: {} - {}",
                top_entry.entry_type.description(),
                top_entry.description
            ));
        }

        recommendations
    }

    /// Generate action plan
    fn generate_action_plan(&self, report: &MarketOpportunityReport) -> Vec<String> {
        let mut action_plan = Vec::new();

        // Immediate actions (this week)
        let urgent_procurement: Vec<_> = report
            .procurement_signals
            .iter()
            .filter(|p| p.estimated_value.map(|v| v > 50000.0).unwrap_or(false))
            .take(3)
            .collect();

        if !urgent_procurement.is_empty() {
            action_plan.push("This week: Review and prepare bids for top procurement opportunities".to_string());
        }

        let urgent_weaknesses: Vec<_> = report
            .competitor_weaknesses
            .iter()
            .filter(|w| matches!(w.time_sensitivity, TimeSensitivity::Urgent))
            .take(3)
            .collect();

        if !urgent_weaknesses.is_empty() {
            action_plan.push("This week: Develop customer acquisition campaign targeting competitor gaps".to_string());
        }

        // Short-term actions (this month)
        if !report.strategic_entry_points.is_empty() {
            action_plan.push("This month: Initiate contact with top strategic entry targets".to_string());
        }

        // Medium-term actions (next quarter)
        if report.market_timing.overall_timing_score > 0.6 {
            action_plan.push("Next quarter: Scale market presence based on favorable timing".to_string());
        }

        // Add monitoring actions
        action_plan.push("Ongoing: Monitor competitor movements and market signals".to_string());

        action_plan
    }

    /// Identify data gaps
    fn identify_data_gaps(&self, report: &MarketOpportunityReport, signals: &[EvidenceItem]) -> Vec<String> {
        let mut gaps = Vec::new();

        if report.procurement_signals.is_empty() {
            gaps.push("No procurement opportunities identified".to_string());
        }

        if report.competitor_weaknesses.is_empty() {
            gaps.push("Limited competitor weakness data".to_string());
        }

        if report.market_timing.overall_timing_score < 0.4 {
            gaps.push("Insufficient market timing data".to_string());
        }

        if report.strategic_entry_points.is_empty() {
            gaps.push("No clear strategic entry points identified".to_string());
        }

        if signals.len() < 10 {
            gaps.push("Limited signal data for comprehensive analysis".to_string());
        }

        gaps
    }
}

impl Default for MarketOpportunityWorkflow {
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
        let workflow = MarketOpportunityWorkflow::new();
        assert!(!workflow.workflow_id.is_empty());
        assert!(workflow.config.include_procurement);
    }

    #[test]
    fn test_workflow_with_config() {
        let config = WorkflowConfig {
            include_procurement: true,
            include_competitor_analysis: false,
            include_timing_analysis: true,
            include_entry_points: false,
            opportunity_threshold: 0.7,
        };
        let workflow = MarketOpportunityWorkflow::with_config(config);
        assert!(workflow.config.include_procurement);
        assert!(!workflow.config.include_competitor_analysis);
    }

    #[test]
    fn test_workflow_run() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Organization A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "tender".to_string(),
                description: "IT Services RFP".to_string(),
                source: "Government Portal".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Competitor X".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_customer_complaint".to_string(),
                description: "Multiple customer complaints".to_string(),
                source: "Social Media".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Technology Services", signals);
        assert_eq!(report.target_market, "Technology Services");
        assert!(!report.procurement_signals.is_empty());
    }

    #[test]
    fn test_procurement_signal_aggregation() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "rfp".to_string(),
                description: "Software Development RFP".to_string(),
                source: "Procurement Portal".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Company B".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "contract_renewal".to_string(),
                description: "Annual contract renewal".to_string(),
                source: "Industry News".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig3".to_string(),
                entity_id: "Company C".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "expansion_project".to_string(),
                description: "New facility construction".to_string(),
                source: "News".to_string(),
                confidence: 0.8,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert_eq!(report.procurement_signals.len(), 3);
    }

    #[test]
    fn test_competitor_weakness_identification() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Competitor A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_financial_stress".to_string(),
                description: "Revenue decline reported".to_string(),
                source: "Financial Reports".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Competitor B".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_product_gap".to_string(),
                description: "Missing AI capabilities".to_string(),
                source: "Product Analysis".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert!(!report.competitor_weaknesses.is_empty());
        assert!(report.competitor_weaknesses.iter().any(|w| matches!(w.severity, WeaknessSeverity::Critical)));
    }

    #[test]
    fn test_market_timing_analysis() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Market".to_string(),
                entity_type: "market".to_string(),
                evidence_type: "market_growth".to_string(),
                description: "Industry showing strong growth".to_string(),
                source: "Industry Report".to_string(),
                confidence: 0.85,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert!(report.market_timing.overall_timing_score > 0.5);
        assert!(!report.market_timing.timing_recommendation.is_empty());
    }

    #[test]
    fn test_strategic_entry_points() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "rfp".to_string(),
                description: "Large RFP opportunity".to_string(),
                source: "Portal".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Competitor X".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_customer_complaint".to_string(),
                description: "Major service failure".to_string(),
                source: "Social Media".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert!(!report.strategic_entry_points.is_empty());
        assert!(report.strategic_entry_points[0].recommendation_rank <= report.strategic_entry_points.last().map(|e| e.recommendation_rank).unwrap_or(1));
    }

    #[test]
    fn test_overall_opportunity_score() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "tender".to_string(),
                description: "High-value opportunity".to_string(),
                source: "Portal".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Competitor A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_financial_stress".to_string(),
                description: "Major weakness detected".to_string(),
                source: "News".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert!(report.overall_opportunity_score > 0.0);
        assert!(report.overall_opportunity_score <= 1.0);
    }

    #[test]
    fn test_action_plan_generation() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![
            EvidenceItem {
                id: "sig1".to_string(),
                entity_id: "Company A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "rfp".to_string(),
                description: "Urgent RFP".to_string(),
                source: "Portal".to_string(),
                confidence: 0.95,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
            EvidenceItem {
                id: "sig2".to_string(),
                entity_id: "Competitor A".to_string(),
                entity_type: "company".to_string(),
                evidence_type: "competitor_customer_complaint".to_string(),
                description: "Urgent customer issue".to_string(),
                source: "Social".to_string(),
                confidence: 0.9,
                timestamp: Utc::now(),
                raw_data: serde_json::json!({}),
            },
        ];

        let report = workflow.run("Test Market", signals);
        assert!(!report.action_plan.is_empty());
    }

    #[test]
    fn test_data_gaps_identification() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Market", signals);
        assert!(!report.data_gaps.is_empty());
    }

    #[test]
    fn test_report_serialization() {
        let workflow = MarketOpportunityWorkflow::new();
        let signals = vec![];

        let report = workflow.run("Test Market", signals);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Test Market"));
        assert!(json.contains("workflow_id"));
    }
}
