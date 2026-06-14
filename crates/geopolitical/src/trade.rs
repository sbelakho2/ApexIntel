//! # Trade Intelligence Module
//!
//! Provides comprehensive trade intelligence capabilities including:
//! - Import/export data analysis with HS codes
//! - Tariff impact assessment
//! - Trade agreement monitoring
//! - Supply chain rerouting signals

use crate::error::{GeopoliticalError, Result};
use crate::models::{
    CountryCode, GeopoliticalConfig, IntelligenceAlert, IntelligenceSource,
    Severity, TrendDirection,
};
use crate::utils::hs_code;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// HS Code classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HsCode {
    pub code: String,
    pub description: String,
    pub chapter: String,
    pub heading: String,
    pub subheading: String,
    pub level: usize,
    pub parent_codes: Vec<String>,
    pub is_sensitive: bool,
    pub restrictions: Vec<String>,
}

impl HsCode {
    /// Create from raw code
    pub fn from_code(code: &str) -> Result<Self> {
        let components = hs_code::parse_hs_code(code)
            .ok_or_else(|| GeopoliticalError::ValidationError(
                format!("Invalid HS code: {}", code)
            ))?;

        let description = hs_code::get_chapter_category(&components.chapter)
            .unwrap_or("General merchandise")
            .to_string();

        Ok(Self {
            code: code.to_string(),
            description,
            chapter: components.chapter.clone(),
            heading: components.heading.clone(),
            subheading: components.subheading.clone(),
            level: if components.additional_2.is_some() {
                10
            } else if components.additional_6.is_some() {
                8
            } else {
                6
            },
            parent_codes: vec![
                components.chapter.clone(),
                format!("{}{}", components.chapter, components.heading),
                format!("{}{}{}", components.chapter, components.heading, components.subheading),
            ],
            is_sensitive: Self::check_sensitive(&components.chapter),
            restrictions: Self::get_restrictions(&components.chapter),
        })
    }

    fn check_sensitive(chapter: &str) -> bool {
        let sensitive_chapters = ["01", "02", "03", "84", "85", "86", "87", "88", "89", "90", "91", "92", "93"];
        sensitive_chapters.contains(&chapter)
    }

    fn get_restrictions(chapter: &str) -> Vec<String> {
        let restrictions_map: HashMap<&str, Vec<&str>> = [
            ("93", vec!["Arms export license required", "End-user certificate required"]),
            ("84", vec!["Dual-use item restrictions", "Technology transfer controls"]),
            ("85", vec!["Technology transfer controls", "Semiconductor restrictions"]),
        ].into_iter().collect();

        restrictions_map.get(chapter)
            .map(|v| v.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// Get the root chapter (2 digits)
    pub fn chapter_code(&self) -> &str {
        &self.chapter
    }

    /// Get the heading (4 digits)
    pub fn heading_code(&self) -> &str {
        &self.heading
    }

    /// Check if this is a dual-use item
    pub fn is_dual_use(&self) -> bool {
        self.is_sensitive && (self.chapter == "84" || self.chapter == "85")
    }
}

/// Tariff information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffInfo {
    pub hs_code: String,
    pub country: CountryCode,
    pub partner_country: CountryCode,
    pub applied_tariff: f32,
    pub mfn_tariff: f32,
    pub preferential_tariff: Option<f32>,
    pub quota_available: Option<f32>,
    pub unit: String,
    pub effective_date: DateTime<Utc>,
    pub trade_agreement: Option<String>,
    pub additional_notes: Vec<String>,
}

impl TariffInfo {
    /// Calculate effective tariff rate
    pub fn effective_rate(&self) -> f32 {
        self.preferential_tariff.unwrap_or(self.applied_tariff)
    }

    /// Check if this is a preferential rate
    pub fn is_preferential(&self) -> bool {
        self.preferential_tariff.is_some_and(|pt| self.applied_tariff > pt)
    }

    /// Calculate tariff savings percentage
    pub fn savings_percentage(&self) -> Option<f32> {
        self.preferential_tariff.map(|pref| {
            if self.mfn_tariff > 0.0 {
                ((self.mfn_tariff - pref) / self.mfn_tariff) * 100.0
            } else {
                0.0
            }
        })
    }

    /// Estimate tariff cost for given value
    pub fn estimate_cost(&self, cargo_value: f32) -> f32 {
        cargo_value * (self.effective_rate() / 100.0)
    }
}

/// Trade flow data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeFlow {
    pub id: Uuid,
    pub hs_code: String,
    pub origin_country: CountryCode,
    pub destination_country: CountryCode,
    pub export_value: f32,
    pub import_value: f32,
    pub quantity: f32,
    pub unit: String,
    pub period: String,
    pub trend: TrendDirection,
    pub year_over_year_change: f32,
    pub month_over_month_change: f32,
    pub reported_at: DateTime<Utc>,
}

impl TradeFlow {
    /// Create a new trade flow
    pub fn new(
        hs_code: String,
        origin: CountryCode,
        destination: CountryCode,
        export_value: f32,
        import_value: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            hs_code,
            origin_country: origin,
            destination_country: destination,
            export_value,
            import_value,
            quantity: 0.0,
            unit: "USD".to_string(),
            period: Utc::now().format("%Y-%m").to_string(),
            trend: TrendDirection::Stable,
            year_over_year_change: 0.0,
            month_over_month_change: 0.0,
            reported_at: Utc::now(),
        }
    }

    /// Calculate total trade value
    pub fn total_value(&self) -> f32 {
        self.export_value + self.import_value
    }

    /// Check if this is a significant trade flow
    pub fn is_significant(&self, threshold: f32) -> bool {
        self.total_value() >= threshold
    }

    /// Calculate trade balance
    pub fn trade_balance(&self) -> f32 {
        self.export_value - self.import_value
    }
}

/// Trade route information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRoute {
    pub id: Uuid,
    pub origin: CountryCode,
    pub destination: CountryCode,
    pub via_countries: Vec<CountryCode>,
    pub distance_km: u32,
    pub estimated_transit_days: u32,
    pub current_issues: Vec<String>,
    pub risk_score: f32,
    pub alternatives: Vec<Uuid>,
}

impl TradeRoute {
    /// Create a new trade route
    pub fn new(origin: CountryCode, destination: CountryCode) -> Self {
        Self {
            id: Uuid::new_v4(),
            origin,
            destination,
            via_countries: Vec::new(),
            distance_km: 0,
            estimated_transit_days: 0,
            current_issues: Vec::new(),
            risk_score: 0.0,
            alternatives: Vec::new(),
        }
    }

    /// Add transit country
    pub fn add_via(&mut self, country: CountryCode) {
        self.via_countries.push(country);
    }

    /// Add issue
    pub fn add_issue(&mut self, issue: String) {
        self.current_issues.push(issue);
    }

    /// Check if route is clear
    pub fn is_clear(&self) -> bool {
        self.current_issues.is_empty() && self.risk_score < 0.5
    }
}

/// Supply chain rerouting signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainSignal {
    pub id: Uuid,
    pub signal_type: SupplyChainSignalType,
    pub affected_routes: Vec<Uuid>,
    pub affected_hs_codes: Vec<String>,
    pub origin_region: String,
    pub destination_region: String,
    pub severity: Severity,
    pub description: String,
    pub confidence: f32,
    pub expected_impact: String,
    pub mitigation_suggestions: Vec<String>,
    pub detected_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl SupplyChainSignal {
    /// Create from detected pattern
    pub fn new(
        signal_type: SupplyChainSignalType,
        origin_region: String,
        destination_region: String,
        description: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            signal_type,
            affected_routes: Vec::new(),
            affected_hs_codes: Vec::new(),
            origin_region,
            destination_region,
            severity: Severity::Medium,
            description,
            confidence: 0.5,
            expected_impact: String::new(),
            mitigation_suggestions: Vec::new(),
            detected_at: Utc::now(),
            expires_at: None,
        }
    }
}

/// Supply chain signal types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SupplyChainSignalType {
    RouteDisruption,
    PortCongestion,
    TariffChange,
    ExportRestriction,
    ImportBan,
    SanctionsImpact,
    ClimateImpact,
    PoliticalInstability,
    DemandShift,
    SupplyShortage,
}

impl SupplyChainSignalType {
    pub fn description(&self) -> &'static str {
        match self {
            SupplyChainSignalType::RouteDisruption => "Route disruption detected",
            SupplyChainSignalType::PortCongestion => "Port congestion reported",
            SupplyChainSignalType::TariffChange => "Tariff change detected",
            SupplyChainSignalType::ExportRestriction => "Export restriction imposed",
            SupplyChainSignalType::ImportBan => "Import ban imposed",
            SupplyChainSignalType::SanctionsImpact => "Sanctions impact on trade",
            SupplyChainSignalType::ClimateImpact => "Climate-related disruption",
            SupplyChainSignalType::PoliticalInstability => "Political instability affecting trade",
            SupplyChainSignalType::DemandShift => "Demand shift detected",
            SupplyChainSignalType::SupplyShortage => "Supply shortage detected",
        }
    }
}

/// Trade agreement information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAgreement {
    pub id: Uuid,
    pub name: String,
    pub parties: Vec<CountryCode>,
    pub agreement_type: TradeAgreementType,
    pub status: TradeAgreementStatus,
    pub effective_date: Option<DateTime<Utc>>,
    pub signed_date: Option<DateTime<Utc>>,
    pub hs_codes_covered: Vec<String>,
    pub tariff_reductions: HashMap<String, f32>,
    pub url: Option<String>,
    pub description: String,
}

impl TradeAgreement {
    /// Create a new trade agreement
    pub fn new(name: String, parties: Vec<CountryCode>, agreement_type: TradeAgreementType) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parties,
            agreement_type,
            status: TradeAgreementStatus::Proposed,
            effective_date: None,
            signed_date: None,
            hs_codes_covered: Vec::new(),
            tariff_reductions: HashMap::new(),
            url: None,
            description: String::new(),
        }
    }

    /// Check if agreement is in force
    pub fn is_in_force(&self) -> bool {
        self.status == TradeAgreementStatus::InForce && 
        self.effective_date.map(|d| d <= Utc::now()).unwrap_or(false)
    }
}

/// Trade agreement types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradeAgreementType {
    Bilateral,
    Multilateral,
    Regional,
    Preferential,
    FreeTradeArea,
    CustomsUnion,
    CommonMarket,
    EconomicUnion,
}

impl TradeAgreementType {
    pub fn description(&self) -> &'static str {
        match self {
            TradeAgreementType::Bilateral => "Bilateral Trade Agreement",
            TradeAgreementType::Multilateral => "Multilateral Trade Agreement",
            TradeAgreementType::Regional => "Regional Trade Agreement",
            TradeAgreementType::Preferential => "Preferential Trade Arrangement",
            TradeAgreementType::FreeTradeArea => "Free Trade Area",
            TradeAgreementType::CustomsUnion => "Customs Union",
            TradeAgreementType::CommonMarket => "Common Market",
            TradeAgreementType::EconomicUnion => "Economic Union",
        }
    }
}

/// Agreement status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradeAgreementStatus {
    Proposed,
    Negotiating,
    Signed,
    Ratifying,
    InForce,
    Suspended,
    Terminated,
}

impl TradeAgreementStatus {
    pub fn description(&self) -> &'static str {
        match self {
            TradeAgreementStatus::Proposed => "Proposed",
            TradeAgreementStatus::Negotiating => "Under Negotiation",
            TradeAgreementStatus::Signed => "Signed (pending ratification)",
            TradeAgreementStatus::Ratifying => "Being Ratified",
            TradeAgreementStatus::InForce => "In Force",
            TradeAgreementStatus::Suspended => "Suspended",
            TradeAgreementStatus::Terminated => "Terminated",
        }
    }
}

/// Import/Export restriction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRestriction {
    pub id: Uuid,
    pub restriction_type: TradeRestrictionType,
    pub country: CountryCode,
    pub target_country: Option<CountryCode>,
    pub hs_codes: Vec<String>,
    pub description: String,
    pub effective_date: DateTime<Utc>,
    pub expiry_date: Option<DateTime<Utc>>,
    pub source: IntelligenceSource,
    pub legal_basis: String,
}

impl TradeRestriction {
    /// Check if HS code is affected
    pub fn affects_hs_code(&self, code: &str) -> bool {
        self.hs_codes.iter().any(|h| code.starts_with(h))
    }

    /// Check if restriction is active
    pub fn is_active(&self) -> bool {
        self.expiry_date.map(|e| e > Utc::now()).unwrap_or(true)
    }
}

/// Trade restriction types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradeRestrictionType {
    ImportBan,
    ExportBan,
    ImportQuota,
    ExportQuota,
    ImportLicense,
    ExportLicense,
    Safeguard,
    AntiDumping,
    Countervailing,
    Sanction,
}

impl TradeRestrictionType {
    pub fn description(&self) -> &'static str {
        match self {
            TradeRestrictionType::ImportBan => "Import Ban",
            TradeRestrictionType::ExportBan => "Export Ban",
            TradeRestrictionType::ImportQuota => "Import Quota",
            TradeRestrictionType::ExportQuota => "Export Quota",
            TradeRestrictionType::ImportLicense => "Import License Required",
            TradeRestrictionType::ExportLicense => "Export License Required",
            TradeRestrictionType::Safeguard => "Safeguard Measure",
            TradeRestrictionType::AntiDumping => "Anti-Dumping Duty",
            TradeRestrictionType::Countervailing => "Countervailing Duty",
            TradeRestrictionType::Sanction => "Sanctions-based Restriction",
        }
    }
}

/// Trade analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAnalysis {
    pub id: Uuid,
    pub origin: CountryCode,
    pub destination: CountryCode,
    pub hs_codes: Vec<String>,
    pub analysis_type: TradeAnalysisType,
    pub results: HashMap<String, f32>,
    pub tariff_impact: f32,
    pub rerouting_recommended: bool,
    pub alternative_routes: Vec<Uuid>,
    pub estimated_cost_change: f32,
    pub confidence: f32,
    pub generated_at: DateTime<Utc>,
}

impl TradeAnalysis {
    /// Calculate total tariff impact
    pub fn total_tariff_impact(&self, cargo_values: &[f32]) -> f32 {
        cargo_values.iter().zip(self.results.values())
            .map(|(value, tariff_rate)| value * (tariff_rate / 100.0))
            .sum()
    }
}

/// Trade analysis types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradeAnalysisType {
    TariffImpact,
    RouteOptimization,
    SupplyChainRisk,
    MarketAccess,
    ComplianceCheck,
}

impl TradeAnalysisType {
    pub fn description(&self) -> &'static str {
        match self {
            TradeAnalysisType::TariffImpact => "Tariff Impact Analysis",
            TradeAnalysisType::RouteOptimization => "Route Optimization Analysis",
            TradeAnalysisType::SupplyChainRisk => "Supply Chain Risk Analysis",
            TradeAnalysisType::MarketAccess => "Market Access Analysis",
            TradeAnalysisType::ComplianceCheck => "Trade Compliance Check",
        }
    }
}

/// Trade intelligence client
pub struct TradeIntelligenceClient {
    _http_client: Client,
    _config: GeopoliticalConfig,
    cached_agreements: Vec<TradeAgreement>,
    cached_restrictions: Vec<TradeRestriction>,
}

impl TradeIntelligenceClient {
    /// Create a new trade intelligence client
    pub fn new(http_client: Client, config: GeopoliticalConfig) -> Self {
        Self {
            _http_client: http_client,
            _config: config,
            cached_agreements: Self::get_default_agreements(),
            cached_restrictions: Vec::new(),
        }
    }

    /// Get default trade agreements
    #[allow(clippy::disallowed_methods)]
    fn get_default_agreements() -> Vec<TradeAgreement> {
        vec![
            {
                let mut agreement = TradeAgreement::new(
                    "USMCA".to_string(),
                    vec![CountryCode::new("US"), CountryCode::new("CA"), CountryCode::new("MX")],
                    TradeAgreementType::FreeTradeArea,
                );
                agreement.status = TradeAgreementStatus::InForce;
                agreement.effective_date = Some(DateTime::parse_from_rfc3339("2020-07-01T00:00:00Z").unwrap().with_timezone(&Utc));
                agreement.signed_date = Some(DateTime::parse_from_rfc3339("2018-11-30T00:00:00Z").unwrap().with_timezone(&Utc));
                agreement
            },
            {
                let mut agreement = TradeAgreement::new(
                    "EU Single Market".to_string(),
                    vec![CountryCode::new("DE"), CountryCode::new("FR"), CountryCode::new("IT"), CountryCode::new("ES")],
                    TradeAgreementType::CommonMarket,
                );
                agreement.status = TradeAgreementStatus::InForce;
                agreement.effective_date = Some(DateTime::parse_from_rfc3339("1993-01-01T00:00:00Z").unwrap().with_timezone(&Utc));
                agreement
            },
            {
                let mut agreement = TradeAgreement::new(
                    "RCEP".to_string(),
                    vec![CountryCode::new("CN"), CountryCode::new("JP"), CountryCode::new("KR"), CountryCode::new("AU")],
                    TradeAgreementType::FreeTradeArea,
                );
                agreement.status = TradeAgreementStatus::InForce;
                agreement.effective_date = Some(DateTime::parse_from_rfc3339("2022-01-01T00:00:00Z").unwrap().with_timezone(&Utc));
                agreement
            },
            {
                let mut agreement = TradeAgreement::new(
                    "CPTPP".to_string(),
                    vec![CountryCode::new("CA"), CountryCode::new("AU"), CountryCode::new("JP")],
                    TradeAgreementType::FreeTradeArea,
                );
                agreement.status = TradeAgreementStatus::InForce;
                agreement.effective_date = Some(DateTime::parse_from_rfc3339("2018-12-30T00:00:00Z").unwrap().with_timezone(&Utc));
                agreement
            },
        ]
    }

    /// Parse HS code
    pub fn parse_hs_code(&self, code: &str) -> Result<HsCode> {
        HsCode::from_code(code)
    }

    /// Lookup tariff rate
    pub async fn lookup_tariff(
        &self,
        hs_code: &str,
        importing_country: &CountryCode,
        exporting_country: &CountryCode,
    ) -> Result<TariffInfo> {
        // First try to parse the HS code
        let hs = HsCode::from_code(hs_code)?;

        // Look for applicable trade agreement
        let agreement = self.find_applicable_agreement(importing_country, exporting_country);
        
        // Calculate tariff rates
        let mfn_rate = self.get_mfn_rate(&hs.chapter);
        let applied_rate = match &agreement {
            Some(agg) if agg.is_in_force() => {
                agg.tariff_reductions.get(hs_code)
                    .copied()
                    .unwrap_or(mfn_rate * 0.8) // Assume 20% reduction under FTA
            }
            _ => mfn_rate,
        };

        let preferential_rate = if agreement.is_some() {
            Some(applied_rate * 0.5) // Further reduction under preferential treatment
        } else {
            None
        };

        Ok(TariffInfo {
            hs_code: hs_code.to_string(),
            country: importing_country.clone(),
            partner_country: exporting_country.clone(),
            applied_tariff: applied_rate,
            mfn_tariff: mfn_rate,
            preferential_tariff: preferential_rate,
            quota_available: None,
            unit: "percent".to_string(),
            effective_date: Utc::now(),
            trade_agreement: agreement.map(|a| a.name.clone()),
            additional_notes: hs.restrictions.clone(),
        })
    }

    /// Get MFN (Most Favored Nation) tariff rate by chapter
    fn get_mfn_rate(&self, chapter: &str) -> f32 {
        let chapter_num: u32 = chapter.parse().unwrap_or(0);
        
        match chapter_num {
            1..=24 => 15.0, // Agricultural products
            25..=27 => 5.0, // Minerals
            28..=38 => 8.0, // Chemicals
            39..=40 => 10.0, // Plastics
            41..=43 => 12.0, // Leather
            44..=49 => 8.0, // Wood
            50..=63 => 15.0, // Textiles
            64..=67 => 20.0, // Footwear
            68..=70 => 10.0, // Stone, cement
            71 => 2.0, // Precious metals
            72..=83 => 10.0, // Base metals
            84..=85 => 5.0, // Machinery
            86..=89 => 8.0, // Vehicles
            90..=92 => 5.0, // Instruments
            93 => 30.0, // Arms
            _ => 10.0,
        }
    }

    /// Find applicable trade agreement
    fn find_applicable_agreement(
        &self,
        country1: &CountryCode,
        country2: &CountryCode,
    ) -> Option<&TradeAgreement> {
        self.cached_agreements.iter().find(|a| {
            a.parties.contains(country1) && a.parties.contains(country2) && a.is_in_force()
        })
    }

    /// Analyze tariff impact
    pub async fn analyze_tariff_impact(
        &self,
        hs_codes: &[String],
        origin: &CountryCode,
        destination: &CountryCode,
        cargo_values: &[f32],
    ) -> Result<TradeAnalysis> {
        let mut tariff_impacts = HashMap::new();
        let mut total_impact = 0.0f32;

        for (i, hs_code) in hs_codes.iter().enumerate() {
            let tariff = self.lookup_tariff(hs_code, destination, origin).await?;
            let value = cargo_values.get(i).copied().unwrap_or(0.0);
            let impact = tariff.effective_rate();
            
            tariff_impacts.insert(hs_code.clone(), impact);
            total_impact += value * (impact / 100.0);
        }

        let avg_impact = if !hs_codes.is_empty() {
            tariff_impacts.values().sum::<f32>() / hs_codes.len() as f32
        } else {
            0.0
        };

        // Check for rerouting opportunities
        let rerouting_recommended = avg_impact > 15.0;

        Ok(TradeAnalysis {
            id: Uuid::new_v4(),
            origin: origin.clone(),
            destination: destination.clone(),
            hs_codes: hs_codes.to_vec(),
            analysis_type: TradeAnalysisType::TariffImpact,
            results: tariff_impacts,
            tariff_impact: avg_impact,
            rerouting_recommended,
            alternative_routes: Vec::new(),
            estimated_cost_change: total_impact,
            confidence: 0.8,
            generated_at: Utc::now(),
        })
    }

    /// Get trade flows for a country pair
    pub async fn get_trade_flows(
        &self,
        origin: &CountryCode,
        destination: &CountryCode,
        period: Option<&str>,
    ) -> Result<Vec<TradeFlow>> {
        // Fetch from external API or return mock data
        Ok(self.get_mock_trade_flows(origin, destination, period))
    }

    /// Generate mock trade flows
    fn get_mock_trade_flows(
        &self,
        origin: &CountryCode,
        destination: &CountryCode,
        _period: Option<&str>,
    ) -> Vec<TradeFlow> {
        // Generate some realistic mock data
        let chapters = ["84", "85", "87", "90"];
        let mut flows = Vec::new();

        for (i, chapter) in chapters.iter().enumerate() {
            let base_value = 100_000.0 + i as f32 * 50_000.0;
            let export_value = base_value;
            let import_value = base_value * 0.9;

            let mut flow = TradeFlow::new(
                format!("{}000000", chapter),
                origin.clone(),
                destination.clone(),
                export_value,
                import_value,
            );
            flow.trend = TrendDirection::Improving;
            flow.year_over_year_change = 0.1;
            flows.push(flow);
        }

        flows
    }

    /// Analyze supply chain risk
    pub async fn analyze_supply_chain(
        &self,
        origin: &CountryCode,
        destination: &CountryCode,
        via_countries: &[CountryCode],
    ) -> Result<Vec<SupplyChainSignal>> {
        let mut signals = Vec::new();

        // Check for sanctions impact
        let sanctioned_countries = ["RU", "IR", "KP", "SY"];
        for via in via_countries {
            if sanctioned_countries.contains(&via.0.as_str()) {
                signals.push(SupplyChainSignal::new(
                    SupplyChainSignalType::SanctionsImpact,
                    origin.to_string(),
                    destination.to_string(),
                    format!("Route through {} may be affected by sanctions", via),
                ));
            }
        }

        // Check for tariff changes affecting route
        if via_countries.len() > 2 {
            signals.push(SupplyChainSignal::new(
                SupplyChainSignalType::RouteDisruption,
                origin.to_string(),
                destination.to_string(),
                "Complex routing may increase transit time and costs".to_string(),
            ));
        }

        // Add risk scores
        for signal in &mut signals {
            signal.confidence = 0.7;
            signal.severity = Severity::High;
        }

        Ok(signals)
    }

    /// Get active trade restrictions
    pub fn get_restrictions(&self) -> Vec<TradeRestriction> {
        self.cached_restrictions.clone()
    }

    /// Add restriction
    pub fn add_restriction(&mut self, restriction: TradeRestriction) {
        self.cached_restrictions.push(restriction);
    }

    /// Check HS code for restrictions
    pub fn check_restrictions(&self, hs_code: &str, country: &CountryCode) -> Vec<&TradeRestriction> {
        self.cached_restrictions.iter()
            .filter(|r| {
                r.affects_hs_code(hs_code) && 
                r.is_active() &&
                (r.country == *country || r.target_country.as_ref() == Some(country))
            })
            .collect()
    }

    /// Get trade agreements
    pub fn get_agreements(&self) -> &[TradeAgreement] {
        &self.cached_agreements
    }

    /// Get active agreements by status
    pub fn get_active_agreements(&self) -> Vec<&TradeAgreement> {
        self.cached_agreements.iter()
            .filter(|a| a.is_in_force())
            .collect()
    }

    /// Search trade agreements
    pub fn search_agreements(
        &self,
        country: Option<&CountryCode>,
        agreement_type: Option<TradeAgreementType>,
    ) -> Vec<&TradeAgreement> {
        self.cached_agreements.iter()
            .filter(|a| {
                let country_match = country.map(|c| a.parties.contains(c)).unwrap_or(true);
                let type_match = agreement_type.map(|t| a.agreement_type == t).unwrap_or(true);
                country_match && type_match
            })
            .collect()
    }

    /// Calculate optimal routing
    pub async fn calculate_routing(
        &self,
        origin: &CountryCode,
        destination: &CountryCode,
        _hs_codes: &[String],
    ) -> Result<Vec<TradeRoute>> {
        let mut routes = Vec::new();

        // Direct route
        let mut direct = TradeRoute::new(origin.clone(), destination.clone());
        direct.distance_km = self.estimate_distance(origin, destination);
        direct.estimated_transit_days = (direct.distance_km as f32 / 500.0) as u32;
        routes.push(direct);

        // Check for potential issues
        let sanctioned_countries = ["RU", "IR", "KP", "SY"];
        for route in &mut routes {
            if sanctioned_countries.contains(&route.origin.0.as_str()) ||
               sanctioned_countries.contains(&route.destination.0.as_str()) {
                route.add_issue("Sanctions risk".to_string());
                route.risk_score = 0.8;
            }
        }

        Ok(routes)
    }

    /// Estimate distance between countries (simplified)
    fn estimate_distance(&self, origin: &CountryCode, _destination: &CountryCode) -> u32 {
        // Simplified distance estimates in km
        let distances: HashMap<&str, HashMap<&str, u32>> = [
            ("US", [("CN", 11000), ("DE", 8000), ("JP", 11000)].into()),
            ("CN", [("US", 11000), ("DE", 8000), ("JP", 3000)].into()),
            ("DE", [("US", 8000), ("CN", 8000), ("JP", 9000)].into()),
        ].into_iter().collect();

        distances.get(&origin.0.as_str())
            .and_then(|d| d.get(&_destination.0.as_str()).copied())
            .unwrap_or(5000)
    }

    /// Generate trade alert
    pub async fn generate_trade_alert(
        &self,
        alert_type: SupplyChainSignalType,
        countries: Vec<CountryCode>,
        description: String,
        severity: Severity,
    ) -> IntelligenceAlert {
        IntelligenceAlert::new(
            crate::models::AlertType::TradeRestriction,
            severity,
            format!("Trade Alert: {}", alert_type.description()),
            description,
            IntelligenceSource::Wto,
            countries,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn test_hs_code_parsing() {
        let result = HsCode::from_code("8471300000");
        assert!(result.is_ok());
        
        let hs = result.unwrap();
        assert_eq!(hs.chapter, "84");
        assert_eq!(hs.heading, "71");
        assert_eq!(hs.subheading, "30");
        assert_eq!(hs.level, 10);
    }

    #[test]
    fn test_hs_code_validation() {
        let result = HsCode::from_code("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_tariff_info() {
        let tariff = TariffInfo {
            hs_code: "847130".to_string(),
            country: CountryCode::new("US"),
            partner_country: CountryCode::new("CN"),
            applied_tariff: 5.0,
            mfn_tariff: 10.0,
            preferential_tariff: Some(2.5),
            quota_available: None,
            unit: "percent".to_string(),
            effective_date: Utc::now(),
            trade_agreement: Some("USMCA".to_string()),
            additional_notes: Vec::new(),
        };

        assert!(tariff.is_preferential());
        assert_eq!(tariff.effective_rate(), 2.5);
        assert!((tariff.savings_percentage().unwrap() - 75.0).abs() < 0.01);
        
        let cost = tariff.estimate_cost(10000.0);
        assert!((cost - 250.0).abs() < 0.01);
    }

    #[test]
    fn test_trade_flow() {
        let flow = TradeFlow::new(
            "847130".to_string(),
            CountryCode::new("US"),
            CountryCode::new("CN"),
            1000000.0,
            800000.0,
        );

        assert_eq!(flow.total_value(), 1800000.0);
        assert_eq!(flow.trade_balance(), 200000.0);
        assert!(flow.is_significant(100000.0));
    }

    #[test]
    fn test_trade_route() {
        let mut route = TradeRoute::new(
            CountryCode::new("US"),
            CountryCode::new("CN"),
        );
        
        route.add_via(CountryCode::new("JP"));
        route.add_issue("Port congestion".to_string());
        
        assert!(!route.is_clear());
        assert_eq!(route.via_countries.len(), 1);
    }

    #[test]
    fn test_supply_chain_signal() {
        let signal = SupplyChainSignal::new(
            SupplyChainSignalType::TariffChange,
            "US".to_string(),
            "CN".to_string(),
            "New tariffs detected".to_string(),
        );

        assert_eq!(signal.signal_type, SupplyChainSignalType::TariffChange);
        assert_eq!(signal.confidence, 0.5);
    }

    #[test]
    fn test_trade_agreement() {
        let mut agreement = TradeAgreement::new(
            "Test Agreement".to_string(),
            vec![CountryCode::new("US"), CountryCode::new("CA")],
            TradeAgreementType::FreeTradeArea,
        );
        agreement.status = TradeAgreementStatus::InForce;
        agreement.effective_date = Some(Utc::now());

        assert!(agreement.is_in_force());
    }

    #[test]
    fn test_trade_restriction() {
        let restriction = TradeRestriction {
            id: Uuid::new_v4(),
            restriction_type: TradeRestrictionType::ExportLicense,
            country: CountryCode::new("US"),
            target_country: Some(CountryCode::new("RU")),
            hs_codes: vec!["8471".to_string()],
            description: "Export license required for technology".to_string(),
            effective_date: Utc::now(),
            expiry_date: None,
            source: IntelligenceSource::Government,
            legal_basis: "EAR".to_string(),
        };

        assert!(restriction.affects_hs_code("847130"));
        assert!(!restriction.affects_hs_code("8501"));
        assert!(restriction.is_active());
    }

    #[tokio::test]
    async fn test_tariff_lookup() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let trade = TradeIntelligenceClient::new(client, config);

        let tariff = trade.lookup_tariff(
            "847130",
            &CountryCode::new("US"),
            &CountryCode::new("CN"),
        ).await.unwrap();

        assert!(tariff.mfn_tariff > 0.0);
        assert!(tariff.applied_tariff <= tariff.mfn_tariff);
    }

    #[tokio::test]
    async fn test_tariff_impact_analysis() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let trade = TradeIntelligenceClient::new(client, config);

        let analysis = trade.analyze_tariff_impact(
            &["847130".to_string(), "847141".to_string()],
            &CountryCode::new("US"),
            &CountryCode::new("CN"),
            &[10000.0, 5000.0],
        ).await.unwrap();

        assert!(!analysis.results.is_empty());
        assert!(analysis.confidence > 0.0);
    }

    #[test]
    fn test_sensitive_hs_codes() {
        let sensitive = ["8471", "8501", "9300"];
        for code in sensitive {
            let hs = HsCode::from_code(&format!("{}0000", code)).unwrap();
            assert!(hs.is_sensitive);
        }

        let normal = HsCode::from_code("123456").unwrap();
        assert!(!normal.is_sensitive);
    }
}
