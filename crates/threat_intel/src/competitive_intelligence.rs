//! # Competitive Threat Intelligence Module
//!
//! 3.2.4: Competitive threat intelligence including market share analysis,
//! technology positioning, pricing intelligence, and strategic move prediction.
//!
//! ## Features
//!
//! - Market share analysis and trends
//! - Technology positioning assessment
//! - Pricing intelligence tracking
//! - Strategic move prediction
//! - Competitive threat assessment

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::models::{ConfidenceLevel, GeoRegion, IndustrySector, TimeWindow};

/// Competitive entity (company, product, service).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Competitor {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<GeoRegion>,
    pub industry: IndustrySector,
    pub market_segments: Vec<String>,
    pub founded_year: Option<u16>,
    pub employee_count: Option<i32>,
    pub revenue_estimate: Option<i64>,
    pub website: Option<String>,
    pub description: Option<String>,
    pub capabilities: Vec<String>,
    pub technologies: Vec<TechnologyStack>,
    pub market_position: MarketPosition,
    pub financial_health: Option<FinancialHealth>,
    pub strategic_moves: Vec<StrategicMove>,
    pub threat_indicators: Vec<ThreatIndicator>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Competitor {
    pub fn new(name: impl Into<String>, industry: IndustrySector) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            legal_name: None,
            country_code: None,
            region: None,
            industry,
            market_segments: Vec::new(),
            founded_year: None,
            employee_count: None,
            revenue_estimate: None,
            website: None,
            description: None,
            capabilities: Vec::new(),
            technologies: Vec::new(),
            market_position: MarketPosition::default(),
            financial_health: None,
            strategic_moves: Vec::new(),
            threat_indicators: Vec::new(),
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Market position of a competitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPosition {
    pub market_share_percent: f64,
    pub market_rank: u32,
    pub relative_strength: f64,
    pub growth_rate: f64,
    pub innovation_index: f64,
    pub customer_satisfaction: Option<f64>,
    pub brand_recognition: Option<f64>,
}

impl Default for MarketPosition {
    fn default() -> Self {
        Self {
            market_share_percent: 0.0,
            market_rank: 0,
            relative_strength: 0.5,
            growth_rate: 0.0,
            innovation_index: 0.5,
            customer_satisfaction: None,
            brand_recognition: None,
        }
    }
}

/// Financial health indicators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialHealth {
    pub credit_rating: Option<String>,
    pub revenue_growth_yoy: f64,
    pub profit_margin: f64,
    pub debt_to_equity: Option<f64>,
    pub cash_reserves: Option<i64>,
    pub burn_rate: Option<i64>,
    pub funding_rounds: Vec<FundingRound>,
    pub profitability_status: ProfitabilityStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProfitabilityStatus {
    Profitable,
    BreakEven,
    PreProfit,
    CashBurn,
    Distressed,
}

impl FinancialHealth {
    pub fn overall_score(&self) -> f64 {
        let mut score: f64 = 0.0;

        // Revenue growth contribution
        score += if self.revenue_growth_yoy > 0.2 {
            0.25
        } else if self.revenue_growth_yoy > 0.0 {
            0.15
        } else if self.revenue_growth_yoy > -0.1 {
            0.05
        } else {
            0.0
        };

        // Profit margin contribution
        score += if self.profit_margin > 0.2 {
            0.25
        } else if self.profit_margin > 0.1 {
            0.15
        } else if self.profit_margin > 0.0 {
            0.05
        } else {
            0.0
        };

        // Status contribution
        score += match self.profitability_status {
            ProfitabilityStatus::Profitable => 0.25,
            ProfitabilityStatus::BreakEven => 0.15,
            ProfitabilityStatus::PreProfit => 0.1,
            ProfitabilityStatus::CashBurn => 0.05,
            ProfitabilityStatus::Distressed => 0.0,
        };

        score.min(1.0)
    }
}

/// Funding round information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRound {
    pub round_type: FundingRoundType,
    pub amount: i64,
    pub date: DateTime<Utc>,
    pub investors: Vec<String>,
    pub valuation: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FundingRoundType {
    Seed,
    SeriesA,
    SeriesB,
    SeriesC,
    SeriesD,
    SeriesE,
    PreIPO,
    IPO,
    Debt,
    Grant,
}

/// Technology stack of a competitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnologyStack {
    pub category: TechnologyCategory,
    pub technologies: Vec<TechnologyItem>,
    pub maturity_level: TechnologyMaturity,
    pub investment_level: InvestmentLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TechnologyCategory {
    Hardware,
    Software,
    #[allow(non_camel_case_types)]
    AI_ML,
    Cloud,
    Security,
    Manufacturing,
    Materials,
    Robotics,
    IoT,
    Blockchain,
    Quantum,
    Other,
}

impl TechnologyCategory {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Hardware => "hardware",
            Self::Software => "software",
            Self::AI_ML => "ai_ml",
            Self::Cloud => "cloud",
            Self::Security => "security",
            Self::Manufacturing => "manufacturing",
            Self::Materials => "materials",
            Self::Robotics => "robotics",
            Self::IoT => "iot",
            Self::Blockchain => "blockchain",
            Self::Quantum => "quantum",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TechnologyMaturity {
    Research,
    Development,
    EarlyStage,
    Growth,
    Mature,
    Legacy,
}

impl TechnologyMaturity {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Research => "research",
            Self::Development => "development",
            Self::EarlyStage => "early_stage",
            Self::Growth => "growth",
            Self::Mature => "mature",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvestmentLevel {
    Low,
    Medium,
    High,
    Strategic,
}

impl InvestmentLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Strategic => "strategic",
        }
    }
}

/// Individual technology item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnologyItem {
    pub name: String,
    pub version: Option<String>,
    pub vendor: Option<String>,
    pub adoption_status: AdoptionStatus,
    pub integration_depth: IntegrationDepth,
    pub strategic_importance: StrategicImportance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdoptionStatus {
    Planned,
    InDevelopment,
    LimitedUse,
    WideAdoption,
    Core,
}

impl AdoptionStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Planned => "planned",
            Self::InDevelopment => "in_development",
            Self::LimitedUse => "limited_use",
            Self::WideAdoption => "wide_adoption",
            Self::Core => "core",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntegrationDepth {
    Experimental,
    Partial,
    Full,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StrategicImportance {
    Low,
    Medium,
    High,
    Critical,
}

/// Strategic move by a competitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategicMove {
    pub id: Uuid,
    pub move_type: StrategicMoveType,
    pub title: String,
    pub description: String,
    pub date_announced: DateTime<Utc>,
    pub date_effective: Option<DateTime<Utc>>,
    pub confidence: ConfidenceLevel,
    pub impact_assessment: ImpactAssessment,
    pub source_reliability: SourceReliability,
    pub related_indicators: Vec<String>,
}

impl StrategicMove {
    pub fn new(move_type: StrategicMoveType, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            move_type,
            title: title.into(),
            description: String::new(),
            date_announced: Utc::now(),
            date_effective: None,
            confidence: ConfidenceLevel::Medium,
            impact_assessment: ImpactAssessment::default(),
            source_reliability: SourceReliability::default(),
            related_indicators: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StrategicMoveType {
    Acquisition,
    Merger,
    Partnership,
    Divestiture,
    MarketExpansion,
    ProductLaunch,
    PriceChange,
    TechnologyInvestment,
    TalentAcquisition,
    RegulatoryAction,
    Financing,
    ExecutiveChange,
}

impl StrategicMoveType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Acquisition => "acquisition",
            Self::Merger => "merger",
            Self::Partnership => "partnership",
            Self::Divestiture => "divestiture",
            Self::MarketExpansion => "market_expansion",
            Self::ProductLaunch => "product_launch",
            Self::PriceChange => "price_change",
            Self::TechnologyInvestment => "technology_investment",
            Self::TalentAcquisition => "talent_acquisition",
            Self::RegulatoryAction => "regulatory_action",
            Self::Financing => "financing",
            Self::ExecutiveChange => "executive_change",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAssessment {
    pub competitive_impact: f64,
    pub market_impact: f64,
    pub timeline: TimeHorizon,
    pub affected_segments: Vec<String>,
    pub threat_level: ThreatLevel,
}

impl Default for ImpactAssessment {
    fn default() -> Self {
        Self {
            competitive_impact: 0.5,
            market_impact: 0.5,
            timeline: TimeHorizon::Medium,
            affected_segments: Vec::new(),
            threat_level: ThreatLevel::Moderate,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeHorizon {
    Immediate,
    Short,
    Medium,
    Long,
}

impl TimeHorizon {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Immediate => "immediate",
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreatLevel {
    Minimal,
    Low,
    Moderate,
    High,
    Severe,
}

impl ThreatLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Severe => "severe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReliability {
    pub reliability_score: f64,
    pub source_type: SourceType,
    pub verification_level: VerificationLevel,
}

impl Default for SourceReliability {
    fn default() -> Self {
        Self {
            reliability_score: 0.5,
            source_type: SourceType::Inferred,
            verification_level: VerificationLevel::Unverified,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceType {
    Primary,
    Secondary,
    Inferred,
    Rumor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationLevel {
    Confirmed,
    Probable,
    Possible,
    Unverified,
}

/// Threat indicator from competitive intelligence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatIndicator {
    pub id: Uuid,
    pub indicator_type: ThreatIndicatorType,
    pub title: String,
    pub description: String,
    pub severity: ThreatLevel,
    pub confidence: ConfidenceLevel,
    pub source: String,
    pub date_observed: DateTime<Utc>,
    pub relevant_segments: Vec<String>,
    pub recommended_response: String,
}

impl ThreatIndicator {
    pub fn new(indicator_type: ThreatIndicatorType, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            indicator_type,
            title: title.into(),
            description: String::new(),
            severity: ThreatLevel::Moderate,
            confidence: ConfidenceLevel::Medium,
            source: String::new(),
            date_observed: Utc::now(),
            relevant_segments: Vec::new(),
            recommended_response: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreatIndicatorType {
    MarketShareGain,
    PriceAggression,
    ProductAdvantage,
    TalentPoaching,
    PartnershipExpansion,
    TechnologyBreakthrough,
    CustomerWin,
    RegulatoryAdvantage,
    FundingIncrease,
    ExecutiveMove,
}

impl ThreatIndicatorType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::MarketShareGain => "market_share_gain",
            Self::PriceAggression => "price_aggression",
            Self::ProductAdvantage => "product_advantage",
            Self::TalentPoaching => "talent_poaching",
            Self::PartnershipExpansion => "partnership_expansion",
            Self::TechnologyBreakthrough => "technology_breakthrough",
            Self::CustomerWin => "customer_win",
            Self::RegulatoryAdvantage => "regulatory_advantage",
            Self::FundingIncrease => "funding_increase",
            Self::ExecutiveMove => "executive_move",
        }
    }
}

/// Market share data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketShareData {
    pub competitor_id: Uuid,
    pub competitor_name: String,
    pub time_period: TimeWindow,
    pub market_segment: String,
    pub share_percent: f64,
    pub revenue: Option<i64>,
    pub volume: Option<i64>,
    pub trend: TrendDirection,
    pub data_source: String,
}

/// Pricing intelligence data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingIntelligence {
    pub product_id: Option<Uuid>,
    pub product_name: String,
    pub competitor_id: Option<Uuid>,
    pub competitor_name: Option<String>,
    pub price: f64,
    pub currency: String,
    pub unit: String,
    pub effective_date: DateTime<Utc>,
    pub price_type: PriceType,
    pub region: Option<GeoRegion>,
    pub discount_available: Option<f64>,
    pub volume_tier_pricing: Option<Vec<VolumeTier>>,
    pub confidence: ConfidenceLevel,
    pub source: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PriceType {
    List,
    Wholesale,
    Contract,
    Promotional,
    Subscription,
    Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeTier {
    pub min_quantity: i64,
    pub max_quantity: Option<i64>,
    pub price_per_unit: f64,
    pub discount_percent: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrendDirection {
    Increasing,
    Stable,
    Decreasing,
    Volatile,
}

/// Strategic prediction based on analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategicPrediction {
    pub id: Uuid,
    pub prediction_type: PredictionType,
    pub title: String,
    pub description: String,
    pub probability: f64,
    pub confidence: ConfidenceLevel,
    pub time_horizon: TimeHorizon,
    pub impact: ImpactAssessment,
    pub supporting_evidence: Vec<String>,
    pub contradicting_evidence: Vec<String>,
    pub recommended_counter_moves: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl StrategicPrediction {
    pub fn new(prediction_type: PredictionType, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            prediction_type,
            title: title.into(),
            description: String::new(),
            probability: 0.5,
            confidence: ConfidenceLevel::Medium,
            time_horizon: TimeHorizon::Medium,
            impact: ImpactAssessment::default(),
            supporting_evidence: Vec::new(),
            contradicting_evidence: Vec::new(),
            recommended_counter_moves: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PredictionType {
    MarketEntry,
    Acquisition,
    ProductLaunch,
    PriceWar,
    Partnership,
    TechnologyPivot,
    Exit,
    Consolidation,
}

/// Competitive Threat Assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitiveThreatAssessment {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub assessment_date: DateTime<Utc>,
    pub competitors: Vec<CompetitorSummary>,
    pub market_trends: Vec<MarketTrend>,
    pub key_threats: Vec<KeyThreat>,
    pub opportunities: Vec<Opportunity>,
    pub recommendations: Vec<CompetitiveRecommendation>,
}

impl CompetitiveThreatAssessment {
    pub fn new(organization_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            organization_id,
            assessment_date: Utc::now(),
            competitors: Vec::new(),
            market_trends: Vec::new(),
            key_threats: Vec::new(),
            opportunities: Vec::new(),
            recommendations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorSummary {
    pub competitor_id: Uuid,
    pub name: String,
    pub threat_level: ThreatLevel,
    pub market_share: f64,
    pub growth_rate: f64,
    pub key_capabilities: Vec<String>,
    pub primary_weakness: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketTrend {
    pub trend_name: String,
    pub description: String,
    pub impact_direction: ImpactDirection,
    pub affected_segments: Vec<String>,
    pub confidence: ConfidenceLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImpactDirection {
    Positive,
    Neutral,
    Negative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyThreat {
    pub threat_id: Uuid,
    pub threat_type: String,
    pub source_competitor: Option<String>,
    pub severity: ThreatLevel,
    pub description: String,
    pub timeline: TimeHorizon,
    pub mitigation_priority: Priority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub opportunity_id: Uuid,
    pub opportunity_type: String,
    pub description: String,
    pub estimated_value: Option<i64>,
    pub confidence: ConfidenceLevel,
    pub time_window: TimeHorizon,
    pub action_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitiveRecommendation {
    pub category: RecommendationCategory,
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub estimated_impact: String,
    pub implementation_effort: String,
    pub time_to_value: TimeHorizon,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecommendationCategory {
    MarketDefense,
    CompetitiveAdvantage,
    StrategicPositioning,
    CustomerRetention,
    Innovation,
    Pricing,
    Partnership,
    Talent,
}

/// Competitive Intelligence Engine.
#[derive(Debug, Clone, Default)]
pub struct CompetitiveIntelligenceEngine {
    competitors: HashMap<Uuid, Competitor>,
    market_data: Vec<MarketShareData>,
    pricing_data: Vec<PricingIntelligence>,
    predictions: Vec<StrategicPrediction>,
}

impl CompetitiveIntelligenceEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a competitor.
    pub fn add_competitor(&mut self, competitor: Competitor) -> Uuid {
        let id = competitor.id;
        self.competitors.insert(id, competitor);
        id
    }

    /// Get a competitor by ID.
    pub fn get_competitor(&self, id: Uuid) -> Option<&Competitor> {
        self.competitors.get(&id)
    }

    /// Get all competitors.
    pub fn get_all_competitors(&self) -> Vec<&Competitor> {
        self.competitors.values().collect()
    }

    /// Get competitors by industry.
    pub fn get_competitors_by_industry(&self, industry: &IndustrySector) -> Vec<&Competitor> {
        self.competitors
            .values()
            .filter(|c| c.industry == *industry)
            .collect()
    }

    /// Get top competitors by market share.
    pub fn get_top_competitors(&self, limit: usize) -> Vec<&Competitor> {
        let mut sorted: Vec<_> = self.competitors.values().collect();
        sorted.sort_by(|a, b| {
            b.market_position.market_share_percent
                .partial_cmp(&a.market_position.market_share_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.into_iter().take(limit).collect()
    }

    /// Get most threatening competitors.
    pub fn get_most_threatening(&self, limit: usize) -> Vec<&Competitor> {
        let mut competitors: Vec<_> = self.competitors.values().collect();
        
        // Sort by combined threat score (growth + innovation + market position)
        competitors.sort_by(|a, b| {
            let threat_a = a.market_position.growth_rate 
                + a.market_position.innovation_index 
                + a.market_position.relative_strength;
            let threat_b = b.market_position.growth_rate 
                + b.market_position.innovation_index 
                + b.market_position.relative_strength;
            threat_b.partial_cmp(&threat_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        competitors.into_iter().take(limit).collect()
    }

    /// Add market share data.
    pub fn add_market_share_data(&mut self, data: MarketShareData) {
        self.market_data.push(data);
    }

    /// Get market share for a competitor over time.
    pub fn get_market_share_history(
        &self,
        competitor_id: Uuid,
    ) -> Vec<&MarketShareData> {
        self.market_data
            .iter()
            .filter(|d| d.competitor_id == competitor_id)
            .collect()
    }

    /// Calculate market share trends.
    pub fn calculate_market_share_trend(&self, competitor_id: Uuid) -> Option<TrendDirection> {
        let history = self.get_market_share_history(competitor_id);
        if history.len() < 2 {
            return None;
        }

        let shares: Vec<f64> = history.iter()
            .map(|d| d.share_percent)
            .collect();

        let first = shares.first()?;
        let last = shares.last()?;

        let change = (last - first) / first * 100.0;
        if change > 5.0 {
            Some(TrendDirection::Increasing)
        } else if change < -5.0 {
            Some(TrendDirection::Decreasing)
        } else {
            Some(TrendDirection::Stable)
        }
    }

    /// Add pricing intelligence.
    pub fn add_pricing_intelligence(&mut self, pricing: PricingIntelligence) {
        self.pricing_data.push(pricing);
    }

    /// Get pricing for a product.
    pub fn get_pricing(&self, product_name: &str) -> Vec<&PricingIntelligence> {
        self.pricing_data
            .iter()
            .filter(|p| p.product_name.to_lowercase().contains(&product_name.to_lowercase()))
            .collect()
    }

    /// Get average price for a product.
    pub fn get_average_price(&self, product_name: &str) -> Option<f64> {
        let prices: Vec<f64> = self.get_pricing(product_name)
            .iter()
            .map(|p| p.price)
            .collect();

        if prices.is_empty() {
            return None;
        }

        Some(prices.iter().sum::<f64>() / prices.len() as f64)
    }

    /// Add a strategic prediction.
    pub fn add_prediction(&mut self, prediction: StrategicPrediction) -> Uuid {
        let id = prediction.id;
        self.predictions.push(prediction);
        id
    }

    /// Get predictions by type.
    pub fn get_predictions_by_type(&self, prediction_type: PredictionType) -> Vec<&StrategicPrediction> {
        self.predictions
            .iter()
            .filter(|p| p.prediction_type == prediction_type)
            .collect()
    }

    /// Get high probability predictions.
    pub fn get_confident_predictions(&self, min_probability: f64) -> Vec<&StrategicPrediction> {
        self.predictions
            .iter()
            .filter(|p| p.probability >= min_probability)
            .collect()
    }

    /// Generate threat assessment.
    pub fn generate_threat_assessment(&self, organization_id: Uuid) -> CompetitiveThreatAssessment {
        let mut assessment = CompetitiveThreatAssessment::new(organization_id);

        // Add competitor summaries
        for competitor in self.get_all_competitors() {
            let threat_level = self.calculate_threat_level(competitor);
            
            // Identify primary weakness from financial health and market position
            let primary_weakness = self.identify_primary_weakness(competitor);
            
            assessment.competitors.push(CompetitorSummary {
                competitor_id: competitor.id,
                name: competitor.name.clone(),
                threat_level,
                market_share: competitor.market_position.market_share_percent,
                growth_rate: competitor.market_position.growth_rate,
                key_capabilities: competitor.capabilities.clone(),
                primary_weakness,
            });
        }

        // Add key threats based on indicators
        for competitor in self.competitors.values() {
            for indicator in &competitor.threat_indicators {
                if indicator.severity == ThreatLevel::High || indicator.severity == ThreatLevel::Severe {
                    assessment.key_threats.push(KeyThreat {
                        threat_id: indicator.id,
                        threat_type: indicator.indicator_type.as_str().to_string(),
                        source_competitor: Some(competitor.name.clone()),
                        severity: indicator.severity,
                        description: indicator.description.clone(),
                        timeline: TimeHorizon::Short,
                        mitigation_priority: Priority::High,
                    });
                }
            }
        }

        // Sort threats by severity
        assessment.key_threats.sort_by(|a, b| {
            let severity_order = |t: &ThreatLevel| match t {
                ThreatLevel::Severe => 0,
                ThreatLevel::High => 1,
                ThreatLevel::Moderate => 2,
                ThreatLevel::Low => 3,
                ThreatLevel::Minimal => 4,
            };
            severity_order(&a.severity).cmp(&severity_order(&b.severity))
        });

        assessment
    }

    fn calculate_threat_level(&self, competitor: &Competitor) -> ThreatLevel {
        let threat_score = 
            competitor.market_position.market_share_percent * 0.3
            + (competitor.market_position.growth_rate.max(0.0) / 0.5) * 0.3 // Normalized growth
            + competitor.market_position.relative_strength * 0.2
            + competitor.market_position.innovation_index * 0.2;

        if threat_score > 0.8 {
            ThreatLevel::Severe
        } else if threat_score > 0.6 {
            ThreatLevel::High
        } else if threat_score > 0.4 {
            ThreatLevel::Moderate
        } else if threat_score > 0.2 {
            ThreatLevel::Low
        } else {
            ThreatLevel::Minimal
        }
    }

    /// Analyze technology positioning.
    pub fn analyze_technology_positioning(&self) -> Vec<TechnologyPositionAnalysis> {
        let mut analyses = Vec::new();

        for competitor in self.competitors.values() {
            for stack in &competitor.technologies {
                analyses.push(TechnologyPositionAnalysis {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    category: stack.category,
                    maturity: stack.maturity_level,
                    investment: stack.investment_level,
                    competitive_advantage: self.calculate_tech_advantage(&stack.technologies),
                });
            }
        }

        analyses
    }

    fn calculate_tech_advantage(&self, technologies: &[TechnologyItem]) -> f64 {
        let mut score = 0.0;
        let count = technologies.len();

        if count == 0 {
            return 0.5;
        }

        for tech in technologies {
            score += match tech.adoption_status {
                AdoptionStatus::Core => 1.0,
                AdoptionStatus::WideAdoption => 0.8,
                AdoptionStatus::LimitedUse => 0.5,
                AdoptionStatus::InDevelopment => 0.3,
                AdoptionStatus::Planned => 0.1,
            };
        }

        (score / count as f64).min(1.0)
    }

    /// Identify the most significant weakness for a competitor.
    fn identify_primary_weakness(&self, competitor: &Competitor) -> Option<String> {
        // Check financial health indicators
        if let Some(financial) = &competitor.financial_health {
            if financial.profitability_status == ProfitabilityStatus::Distressed {
                return Some("Distressed financial position - high burn rate or insolvency risk".to_string());
            }
            if financial.profitability_status == ProfitabilityStatus::CashBurn {
                return Some("High cash burn rate without sustainable revenue".to_string());
            }
            if financial.revenue_growth_yoy < -0.1 {
                return Some(format!("Declining revenue ({}% YoY decline)", (financial.revenue_growth_yoy * 100.0) as i32));
            }
            if financial.profit_margin < 0.0 {
                return Some(format!("Negative profit margins ({:.1}%)", financial.profit_margin * 100.0));
            }
            if let Some(de) = financial.debt_to_equity {
                if de > 2.0 {
                    return Some(format!("High debt-to-equity ratio ({:.1})", de));
                }
            }
        }

        // Check market position weaknesses
        if competitor.market_position.market_share_percent < 2.0 {
            return Some("Very low market share - limited competitive presence".to_string());
        }
        if competitor.market_position.growth_rate < -0.05 {
            return Some(format!("Negative growth rate ({:.1}%)", competitor.market_position.growth_rate * 100.0));
        }
        if competitor.market_position.relative_strength < 0.3 {
            return Some("Weak relative competitive position".to_string());
        }
        if competitor.market_position.innovation_index < 0.3 {
            return Some("Low innovation output relative to peers".to_string());
        }
        if competitor.capabilities.is_empty() {
            return Some("Limited or undefined core capabilities".to_string());
        }

        None
    }

    /// Predict competitive moves based on patterns.
    pub fn predict_competitive_moves(&self) -> Vec<MovePrediction> {
        let mut predictions = Vec::new();

        for competitor in self.competitors.values() {
            let market_share = competitor.market_position.market_share_percent;
            let growth = competitor.market_position.growth_rate;
            let relative_strength = competitor.market_position.relative_strength;
            let innovation = competitor.market_position.innovation_index;
            let has_financial_data = competitor.financial_health.is_some();
            let is_financially_healthy = competitor.financial_health.as_ref()
                .map(|f| f.profitability_status == ProfitabilityStatus::Profitable)
                .unwrap_or(false);
            let revenue_growth = competitor.financial_health.as_ref()
                .map(|f| f.revenue_growth_yoy)
                .unwrap_or(0.0);

            // Assess data sufficiency
            let data_points = competitor.strategic_moves.len()
                + if has_financial_data { 1 } else { 0 }
                + if market_share > 0.0 { 1 } else { 0 };
            
            if data_points < 2 {
                // Insufficient data to make predictions
                continue;
            }

            // Rule 1: High growth + strong innovation + recent product launches → likely ProductLaunch
            if growth > 0.1 && innovation > 0.6 {
                let confidence = if revenue_growth > 0.2 && is_financially_healthy {
                    ConfidenceLevel::High
                } else if revenue_growth > 0.0 {
                    ConfidenceLevel::Medium
                } else {
                    ConfidenceLevel::Low
                };
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::ProductLaunch,
                    confidence,
                    reasoning: format!(
                        "Strong growth ({:.1}%) and high innovation ({:.1}) suggest upcoming product launches",
                        growth * 100.0, innovation
                    ),
                    recommended_response: "Monitor product pipeline and prepare competitive analysis".to_string(),
                });
            }

            // Rule 2: High market share + slow growth → likely MarketExpansion into new segments
            if market_share > 15.0 && growth < 0.05 {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::MarketExpansion,
                    confidence: ConfidenceLevel::Medium,
                    reasoning: format!(
                        "High market share ({:.1}%) with slowing growth ({:.1}%) suggests seeking new markets",
                        market_share, growth * 100.0
                    ),
                    recommended_response: "Identify potential expansion targets and strengthen existing relationships".to_string(),
                });
            }

            // Rule 3: Strong financial health + low market rank → likely Acquisition/Partnership
            if is_financially_healthy && competitor.market_position.market_rank > 3 && market_share < 10.0 {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::Acquisition,
                    confidence: ConfidenceLevel::Medium,
                    reasoning: format!(
                        "Strong financial position with relatively low market share ({:.1}%) suggests acquisition strategy",
                        market_share
                    ),
                    recommended_response: "Monitor M&A activity and identify potential acquisition targets".to_string(),
                });
            }

            // Rule 4: Negative growth + financial stress → likely Divestiture/Restructuring
            if growth < -0.05 || competitor.financial_health.as_ref().map(|f| f.profit_margin < 0.0).unwrap_or(false) {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::Divestiture,
                    confidence: ConfidenceLevel::Low,
                    reasoning: format!(
                        "Negative growth ({:.1}%) or margin pressure may force divestiture of non-core assets",
                        growth * 100.0
                    ),
                    recommended_response: "Prepare to capture divested assets or market share".to_string(),
                });
            }

            // Rule 5: High innovation + low market share → likely TechnologyInvestment/Partnership
            if innovation > 0.7 && market_share < 8.0 {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::TechnologyInvestment,
                    confidence: if innovation > 0.8 { ConfidenceLevel::High } else { ConfidenceLevel::Medium },
                    reasoning: format!(
                        "High innovation ({:.1}) with low market share ({:.1}%) indicates R&D investment focus",
                        innovation, market_share
                    ),
                    recommended_response: "Track technology patents and partnership announcements".to_string(),
                });
            }

            // Rule 6: Recent history of talent/executive changes → further ExecutiveChange likely
            let recent_exec_changes = competitor.strategic_moves.iter()
                .filter(|m| matches!(m.move_type, StrategicMoveType::ExecutiveChange))
                .count();
            if recent_exec_changes > 0 {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::ExecutiveChange,
                    confidence: if recent_exec_changes > 2 { ConfidenceLevel::High } else { ConfidenceLevel::Low },
                    reasoning: format!(
                        "Recent executive changes ({}) signal ongoing organizational restructuring",
                        recent_exec_changes
                    ),
                    recommended_response: "Monitor leadership changes and assess impact on strategy".to_string(),
                });
            }

            // Rule 7: Strong growth in market share + customer wins → likely PriceChange
            if growth > 0.15 && relative_strength > 0.7 {
                predictions.push(MovePrediction {
                    competitor_id: competitor.id,
                    competitor_name: competitor.name.clone(),
                    predicted_move: StrategicMoveType::PriceChange,
                    confidence: ConfidenceLevel::Medium,
                    reasoning: format!(
                        "Growing market influence ({:.1}% growth, strength {:.1}) may enable pricing power adjustments",
                        growth * 100.0, relative_strength
                    ),
                    recommended_response: "Review pricing strategy and value proposition".to_string(),
                });
            }
        }

        predictions
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnologyPositionAnalysis {
    pub competitor_id: Uuid,
    pub competitor_name: String,
    pub category: TechnologyCategory,
    pub maturity: TechnologyMaturity,
    pub investment: InvestmentLevel,
    pub competitive_advantage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovePrediction {
    pub competitor_id: Uuid,
    pub competitor_name: String,
    pub predicted_move: StrategicMoveType,
    pub confidence: ConfidenceLevel,
    pub reasoning: String,
    pub recommended_response: String,
}

// Builder extensions
impl Competitor {
    pub fn with_market_share(mut self, share: f64) -> Self {
        self.market_position.market_share_percent = share.clamp(0.0, 100.0);
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_region(mut self, region: GeoRegion) -> Self {
        self.region = Some(region);
        self
    }
}

impl StrategicMove {
    pub fn with_confidence(mut self, confidence: ConfidenceLevel) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_impact(mut self, impact: f64) -> Self {
        self.impact_assessment.competitive_impact = impact.clamp(0.0, 1.0);
        self
    }
}

impl StrategicPrediction {
    pub fn with_probability(mut self, probability: f64) -> Self {
        self.probability = probability.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_competitor_creation() {
        let competitor = Competitor::new("Test Corp", IndustrySector::Technology)
            .with_market_share(15.5);

        assert_eq!(competitor.name, "Test Corp");
        assert_eq!(competitor.industry, IndustrySector::Technology);
        assert_eq!(competitor.market_position.market_share_percent, 15.5);
    }

    #[test]
    fn test_market_position_default() {
        let position = MarketPosition::default();
        assert_eq!(position.market_share_percent, 0.0);
        assert_eq!(position.relative_strength, 0.5);
    }

    #[test]
    fn test_financial_health_score() {
        let health = FinancialHealth {
            credit_rating: Some("AAA".to_string()),
            revenue_growth_yoy: 0.25,
            profit_margin: 0.15,
            debt_to_equity: None,
            cash_reserves: Some(100_000_000),
            burn_rate: None,
            funding_rounds: Vec::new(),
            profitability_status: ProfitabilityStatus::Profitable,
        };

        let score = health.overall_score();
        assert!(score > 0.6);
    }

    #[test]
    fn test_strategic_move_creation() {
        let move_ = StrategicMove::new(StrategicMoveType::Acquisition, "Acquisition of TechCo")
            .with_confidence(ConfidenceLevel::High)
            .with_impact(0.8);

        assert_eq!(move_.move_type, StrategicMoveType::Acquisition);
        assert_eq!(move_.confidence, ConfidenceLevel::High);
    }

    #[test]
    fn test_threat_indicator_creation() {
        let indicator = ThreatIndicator::new(ThreatIndicatorType::MarketShareGain, "Market Share Increase");
        assert_eq!(indicator.indicator_type, ThreatIndicatorType::MarketShareGain);
    }

    #[test]
    fn test_engine_add_competitor() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        let id = engine.add_competitor(
            Competitor::new("Competitor A", IndustrySector::Electronics)
                .with_market_share(20.0)
        );

        assert!(engine.get_competitor(id).is_some());
    }

    #[test]
    fn test_top_competitors() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_competitor(
            Competitor::new("Small Player", IndustrySector::Technology)
                .with_market_share(5.0)
        );
        engine.add_competitor(
            Competitor::new("Large Player", IndustrySector::Technology)
                .with_market_share(30.0)
        );
        engine.add_competitor(
            Competitor::new("Medium Player", IndustrySector::Technology)
                .with_market_share(15.0)
        );

        let top = engine.get_top_competitors(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "Large Player");
        assert_eq!(top[1].name, "Medium Player");
    }

    #[test]
    fn test_pricing_intelligence() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_pricing_intelligence(PricingIntelligence {
            product_id: None,
            product_name: "Widget A".to_string(),
            competitor_id: None,
            competitor_name: Some("Competitor X".to_string()),
            price: 99.99,
            currency: "USD".to_string(),
            unit: "unit".to_string(),
            effective_date: Utc::now(),
            price_type: PriceType::List,
            region: None,
            discount_available: Some(10.0),
            volume_tier_pricing: None,
            confidence: ConfidenceLevel::High,
            source: "Web scraping".to_string(),
        });

        let avg = engine.get_average_price("Widget A");
        assert!(avg.is_some());
        assert!((avg.unwrap() - 99.99).abs() < 0.01);
    }

    #[test]
    fn test_threat_level_calculation() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        let id = engine.add_competitor(
            Competitor::new("Aggressive Competitor", IndustrySector::Technology)
                .with_market_share(25.0)
        );

        let competitor = engine.get_competitor(id).unwrap();
        let threat_level = engine.calculate_threat_level(competitor);
        
        // High market share and assumed good metrics should result in higher threat
        assert!(matches!(threat_level, ThreatLevel::High | ThreatLevel::Severe | ThreatLevel::Moderate));
    }

    #[test]
    fn test_prediction_creation() {
        let prediction = StrategicPrediction::new(PredictionType::MarketEntry, "Competitor X entering market")
            .with_probability(0.75);

        assert_eq!(prediction.prediction_type, PredictionType::MarketEntry);
        assert!((prediction.probability - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_strategic_move_type() {
        assert_eq!(StrategicMoveType::Acquisition.as_str(), "acquisition");
        assert_eq!(StrategicMoveType::PriceChange.as_str(), "price_change");
    }

    #[test]
    fn test_threat_assessment_generation() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_competitor(
            Competitor::new("Threat Competitor", IndustrySector::Technology)
                .with_market_share(30.0)
        );

        let assessment = engine.generate_threat_assessment(Uuid::new_v4());
        
        assert!(!assessment.competitors.is_empty());
    }

    #[test]
    fn test_technology_positioning() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        let mut competitor = Competitor::new("Tech Leader", IndustrySector::Technology);
        competitor.technologies.push(TechnologyStack {
            category: TechnologyCategory::AI_ML,
            technologies: vec![
                TechnologyItem {
                    name: "Advanced AI".to_string(),
                    version: None,
                    vendor: None,
                    adoption_status: AdoptionStatus::Core,
                    integration_depth: IntegrationDepth::Critical,
                    strategic_importance: StrategicImportance::Critical,
                }
            ],
            maturity_level: TechnologyMaturity::Growth,
            investment_level: InvestmentLevel::High,
        });

        engine.add_competitor(competitor);

        let analysis = engine.analyze_technology_positioning();
        assert!(!analysis.is_empty());
    }
}
