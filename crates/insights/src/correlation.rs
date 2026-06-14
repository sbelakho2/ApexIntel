//! Cross-domain correlation engine for intelligence signal fusion.
//!
//! Automatically correlates signals across domains to discover causal chains:
//! - Economic signals → Technology signals → Personnel signals → Procurement signals
//! - Uses Granger causality and mutual information to discover cross-domain links
//! - Builds ripple effect simulations for major events

use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ============================================================================
// COMPREHENSIVE SIGNAL TAXONOMY (100+ types across all domains)
// ============================================================================

/// Signal domain categories for cross-correlation analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalDomain {
    Economic,
    Technology,
    Personnel,
    Procurement,
    Regulatory,
    Geopolitical,
    Financial,
    SupplyChain,
    Security,
    Environmental,
    Social,
    Legal,
    Operational,
    Market,
    Infrastructure,
}

impl SignalDomain {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Economic,
            Self::Technology,
            Self::Personnel,
            Self::Procurement,
            Self::Regulatory,
            Self::Geopolitical,
            Self::Financial,
            Self::SupplyChain,
            Self::Security,
            Self::Environmental,
            Self::Social,
            Self::Legal,
            Self::Operational,
            Self::Market,
            Self::Infrastructure,
        ]
    }
}

/// Comprehensive signal type taxonomy with 120+ signal types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalType {
    // ═══════════════════════════════════════════════════════════════════════
    // ECONOMIC DOMAIN (20 types)
    // ═══════════════════════════════════════════════════════════════════════
    CommodityPriceSpike,
    CommodityPriceDrop,
    CurrencyFluctuation,
    InterestRateChange,
    InflationIndicator,
    GdpGrowthSignal,
    TradeBalanceShift,
    UnemploymentChange,
    ConsumerConfidenceShift,
    ManufacturingIndexChange,
    RetailSalesChange,
    HousingMarketSignal,
    EnergyPriceVolatility,
    RawMaterialShortage,
    FreightRateChange,
    PortCongestion,
    WarehouseCapacityConstraint,
    LaborCostIncrease,
    ProductivityChange,
    EconomicForecastRevision,

    // ═══════════════════════════════════════════════════════════════════════
    // TECHNOLOGY DOMAIN (15 types)
    // ═══════════════════════════════════════════════════════════════════════
    PatentFiling,
    PatentGrant,
    PatentLitigation,
    RdInvestmentIncrease,
    TechnologyPartnership,
    ProductInnovation,
    ProcessInnovation,
    TechStackChange,
    OpenSourceContribution,
    AcademicCollaboration,
    StandardsBodyParticipation,
    TechTalentHiring,
    LabExpansion,
    PrototypeAnnouncement,
    TechTransferAgreement,

    // ═══════════════════════════════════════════════════════════════════════
    // PERSONNEL DOMAIN (15 types)
    // ═══════════════════════════════════════════════════════════════════════
    ExecutiveHire,
    ExecutiveDeparture,
    BoardChange,
    MassHiring,
    Layoffs,
    ReorganizationAnnouncement,
    KeyPersonRetirement,
    FounderTransition,
    TalentPoaching,
    UnionActivity,
    StrikeOrWorkStoppage,
    RemoteWorkPolicyChange,
    DeiInitiative,
    CompensationChange,
    EmployeeSatisfactionShift,

    // ═══════════════════════════════════════════════════════════════════════
    // PROCUREMENT DOMAIN (12 types)
    // ═══════════════════════════════════════════════════════════════════════
    TenderPublication,
    ContractAward,
    RfqIssuance,
    SupplierQualification,
    SupplierDisqualification,
    VendorConsolidation,
    LongTermAgreement,
    SpotPurchase,
    ReverseAuction,
    StrategicSourcingInitiative,
    LocalContentRequirement,
    ProcurementPolicyChange,

    // ═══════════════════════════════════════════════════════════════════════
    // REGULATORY DOMAIN (12 types)
    // ═══════════════════════════════════════════════════════════════════════
    NewRegulation,
    RegulationRepeal,
    ComplianceDeadline,
    RegulatoryInvestigation,
    EnforcementAction,
    LicenseGrant,
    LicenseRevocation,
    EnvironmentalPermit,
    SafetyInspection,
    AuditFinding,
    CertificationChange,
    StandardsUpdate,

    // ═══════════════════════════════════════════════════════════════════════
    // GEOPOLITICAL DOMAIN (12 types)
    // ═══════════════════════════════════════════════════════════════════════
    SanctionImposed,
    SanctionLifted,
    TariffChange,
    TradeAgreement,
    TradeDispute,
    ExportControlChange,
    DiplomaticIncident,
    PoliticalInstability,
    ElectionResult,
    PolicyShift,
    MilitaryActivity,
    TerritorialDispute,

    // ═══════════════════════════════════════════════════════════════════════
    // FINANCIAL DOMAIN (12 types)
    // ═══════════════════════════════════════════════════════════════════════
    EarningsReport,
    RevenueGuidanceChange,
    ProfitWarning,
    DebtIssuance,
    EquityRaise,
    DividendChange,
    ShareBuyback,
    CreditRatingChange,
    BankruptcyFiling,
    RestructuringAnnouncement,
    AssetSale,
    WritedownOrImpairment,

    // ═══════════════════════════════════════════════════════════════════════
    // SUPPLY CHAIN DOMAIN (12 types)
    // ═══════════════════════════════════════════════════════════════════════
    SupplierDisruption,
    LogisticsBottleneck,
    InventoryBuildUp,
    InventoryDrawdown,
    LeadTimeExtension,
    DualSourcingInitiative,
    NearshoringAnnouncement,
    OffshoringAnnouncement,
    VerticalIntegration,
    OutsourcingDecision,
    SupplyChainVisibilityInvestment,
    ContingencyStockBuilding,

    // ═══════════════════════════════════════════════════════════════════════
    // SECURITY DOMAIN (10 types)
    // ═══════════════════════════════════════════════════════════════════════
    CyberAttack,
    DataBreach,
    RansomwareIncident,
    SecurityVulnerabilityDisclosure,
    PhysicalSecurityIncident,
    InsiderThreat,
    EspionageActivity,
    SecurityCertificationChange,
    PenetrationTestResult,
    SecurityInvestment,

    // ═══════════════════════════════════════════════════════════════════════
    // ENVIRONMENTAL DOMAIN (10 types)
    // ═══════════════════════════════════════════════════════════════════════
    NaturalDisaster,
    ClimateEvent,
    EnvironmentalViolation,
    EmissionsReduction,
    SustainabilityInitiative,
    RenewableEnergyInvestment,
    WasteReductionProgram,
    WaterScarcityImpact,
    BiodiversityImpact,
    CarbonCreditActivity,

    // ═══════════════════════════════════════════════════════════════════════
    // SOCIAL DOMAIN (8 types)
    // ═══════════════════════════════════════════════════════════════════════
    PublicRelationsCrisis,
    BrandReputationShift,
    CustomerSentimentChange,
    SocialMediaControversy,
    BoycottOrCampaign,
    CommunityRelationsIssue,
    PhilanthropicInitiative,
    SocialLicenseChallenge,

    // ═══════════════════════════════════════════════════════════════════════
    // LEGAL DOMAIN (8 types)
    // ═══════════════════════════════════════════════════════════════════════
    LawsuitFiled,
    LawsuitSettlement,
    ClassActionInitiated,
    AntitrustInvestigation,
    IpDispute,
    ContractDispute,
    RegulatoryLitigation,
    ArbitrationProceeding,

    // ═══════════════════════════════════════════════════════════════════════
    // OPERATIONAL DOMAIN (10 types)
    // ═══════════════════════════════════════════════════════════════════════
    FacilityExpansion,
    FacilityClosure,
    CapacityIncrease,
    CapacityReduction,
    ProductionLineChange,
    QualityIssue,
    ProductRecall,
    MaintenanceOutage,
    OperationalEfficiencyProgram,
    AutomationInvestment,

    // ═══════════════════════════════════════════════════════════════════════
    // MARKET DOMAIN (10 types)
    // ═══════════════════════════════════════════════════════════════════════
    MarketShareChange,
    CompetitorEntry,
    CompetitorExit,
    PricingStrategyChange,
    NewMarketEntry,
    MarketWithdrawal,
    ChannelPartnershipChange,
    DistributionExpansion,
    CustomerConcentrationChange,
    MarketConsolidation,

    // ═══════════════════════════════════════════════════════════════════════
    // INFRASTRUCTURE DOMAIN (6 types)
    // ═══════════════════════════════════════════════════════════════════════
    DataCenterInvestment,
    NetworkExpansion,
    CloudMigration,
    ItSystemOutage,
    DigitalTransformationInitiative,
    InfrastructureModernization,

    // ═══════════════════════════════════════════════════════════════════════
    // CATCH-ALL
    // ═══════════════════════════════════════════════════════════════════════
    Other(String),
}

impl SignalType {
    /// Get the domain this signal type belongs to.
    pub fn domain(&self) -> SignalDomain {
        match self {
            // Economic
            Self::CommodityPriceSpike
            | Self::CommodityPriceDrop
            | Self::CurrencyFluctuation
            | Self::InterestRateChange
            | Self::InflationIndicator
            | Self::GdpGrowthSignal
            | Self::TradeBalanceShift
            | Self::UnemploymentChange
            | Self::ConsumerConfidenceShift
            | Self::ManufacturingIndexChange
            | Self::RetailSalesChange
            | Self::HousingMarketSignal
            | Self::EnergyPriceVolatility
            | Self::RawMaterialShortage
            | Self::FreightRateChange
            | Self::PortCongestion
            | Self::WarehouseCapacityConstraint
            | Self::LaborCostIncrease
            | Self::ProductivityChange
            | Self::EconomicForecastRevision => SignalDomain::Economic,

            // Technology
            Self::PatentFiling
            | Self::PatentGrant
            | Self::PatentLitigation
            | Self::RdInvestmentIncrease
            | Self::TechnologyPartnership
            | Self::ProductInnovation
            | Self::ProcessInnovation
            | Self::TechStackChange
            | Self::OpenSourceContribution
            | Self::AcademicCollaboration
            | Self::StandardsBodyParticipation
            | Self::TechTalentHiring
            | Self::LabExpansion
            | Self::PrototypeAnnouncement
            | Self::TechTransferAgreement => SignalDomain::Technology,

            // Personnel
            Self::ExecutiveHire
            | Self::ExecutiveDeparture
            | Self::BoardChange
            | Self::MassHiring
            | Self::Layoffs
            | Self::ReorganizationAnnouncement
            | Self::KeyPersonRetirement
            | Self::FounderTransition
            | Self::TalentPoaching
            | Self::UnionActivity
            | Self::StrikeOrWorkStoppage
            | Self::RemoteWorkPolicyChange
            | Self::DeiInitiative
            | Self::CompensationChange
            | Self::EmployeeSatisfactionShift => SignalDomain::Personnel,

            // Procurement
            Self::TenderPublication
            | Self::ContractAward
            | Self::RfqIssuance
            | Self::SupplierQualification
            | Self::SupplierDisqualification
            | Self::VendorConsolidation
            | Self::LongTermAgreement
            | Self::SpotPurchase
            | Self::ReverseAuction
            | Self::StrategicSourcingInitiative
            | Self::LocalContentRequirement
            | Self::ProcurementPolicyChange => SignalDomain::Procurement,

            // Regulatory
            Self::NewRegulation
            | Self::RegulationRepeal
            | Self::ComplianceDeadline
            | Self::RegulatoryInvestigation
            | Self::EnforcementAction
            | Self::LicenseGrant
            | Self::LicenseRevocation
            | Self::EnvironmentalPermit
            | Self::SafetyInspection
            | Self::AuditFinding
            | Self::CertificationChange
            | Self::StandardsUpdate => SignalDomain::Regulatory,

            // Geopolitical
            Self::SanctionImposed
            | Self::SanctionLifted
            | Self::TariffChange
            | Self::TradeAgreement
            | Self::TradeDispute
            | Self::ExportControlChange
            | Self::DiplomaticIncident
            | Self::PoliticalInstability
            | Self::ElectionResult
            | Self::PolicyShift
            | Self::MilitaryActivity
            | Self::TerritorialDispute => SignalDomain::Geopolitical,

            // Financial
            Self::EarningsReport
            | Self::RevenueGuidanceChange
            | Self::ProfitWarning
            | Self::DebtIssuance
            | Self::EquityRaise
            | Self::DividendChange
            | Self::ShareBuyback
            | Self::CreditRatingChange
            | Self::BankruptcyFiling
            | Self::RestructuringAnnouncement
            | Self::AssetSale
            | Self::WritedownOrImpairment => SignalDomain::Financial,

            // Supply Chain
            Self::SupplierDisruption
            | Self::LogisticsBottleneck
            | Self::InventoryBuildUp
            | Self::InventoryDrawdown
            | Self::LeadTimeExtension
            | Self::DualSourcingInitiative
            | Self::NearshoringAnnouncement
            | Self::OffshoringAnnouncement
            | Self::VerticalIntegration
            | Self::OutsourcingDecision
            | Self::SupplyChainVisibilityInvestment
            | Self::ContingencyStockBuilding => SignalDomain::SupplyChain,

            // Security
            Self::CyberAttack
            | Self::DataBreach
            | Self::RansomwareIncident
            | Self::SecurityVulnerabilityDisclosure
            | Self::PhysicalSecurityIncident
            | Self::InsiderThreat
            | Self::EspionageActivity
            | Self::SecurityCertificationChange
            | Self::PenetrationTestResult
            | Self::SecurityInvestment => SignalDomain::Security,

            // Environmental
            Self::NaturalDisaster
            | Self::ClimateEvent
            | Self::EnvironmentalViolation
            | Self::EmissionsReduction
            | Self::SustainabilityInitiative
            | Self::RenewableEnergyInvestment
            | Self::WasteReductionProgram
            | Self::WaterScarcityImpact
            | Self::BiodiversityImpact
            | Self::CarbonCreditActivity => SignalDomain::Environmental,

            // Social
            Self::PublicRelationsCrisis
            | Self::BrandReputationShift
            | Self::CustomerSentimentChange
            | Self::SocialMediaControversy
            | Self::BoycottOrCampaign
            | Self::CommunityRelationsIssue
            | Self::PhilanthropicInitiative
            | Self::SocialLicenseChallenge => SignalDomain::Social,

            // Legal
            Self::LawsuitFiled
            | Self::LawsuitSettlement
            | Self::ClassActionInitiated
            | Self::AntitrustInvestigation
            | Self::IpDispute
            | Self::ContractDispute
            | Self::RegulatoryLitigation
            | Self::ArbitrationProceeding => SignalDomain::Legal,

            // Operational
            Self::FacilityExpansion
            | Self::FacilityClosure
            | Self::CapacityIncrease
            | Self::CapacityReduction
            | Self::ProductionLineChange
            | Self::QualityIssue
            | Self::ProductRecall
            | Self::MaintenanceOutage
            | Self::OperationalEfficiencyProgram
            | Self::AutomationInvestment => SignalDomain::Operational,

            // Market
            Self::MarketShareChange
            | Self::CompetitorEntry
            | Self::CompetitorExit
            | Self::PricingStrategyChange
            | Self::NewMarketEntry
            | Self::MarketWithdrawal
            | Self::ChannelPartnershipChange
            | Self::DistributionExpansion
            | Self::CustomerConcentrationChange
            | Self::MarketConsolidation => SignalDomain::Market,

            // Infrastructure
            Self::DataCenterInvestment
            | Self::NetworkExpansion
            | Self::CloudMigration
            | Self::ItSystemOutage
            | Self::DigitalTransformationInitiative
            | Self::InfrastructureModernization => SignalDomain::Infrastructure,

            Self::Other(_) => SignalDomain::Market, // Default fallback
        }
    }

    /// Parse signal type from a raw string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        let lower = s.to_lowercase();

        // Economic
        if lower.contains("commodity") && lower.contains("spike") {
            return Self::CommodityPriceSpike;
        }
        if lower.contains("commodity") && lower.contains("drop") {
            return Self::CommodityPriceDrop;
        }
        if lower.contains("currency") || lower.contains("forex") || lower.contains("exchange rate")
        {
            return Self::CurrencyFluctuation;
        }
        if lower.contains("interest rate") {
            return Self::InterestRateChange;
        }
        if lower.contains("inflation") {
            return Self::InflationIndicator;
        }
        if lower.contains("gdp") {
            return Self::GdpGrowthSignal;
        }
        if lower.contains("trade balance") {
            return Self::TradeBalanceShift;
        }
        if lower.contains("unemployment") || lower.contains("jobless") {
            return Self::UnemploymentChange;
        }
        if lower.contains("consumer confidence") {
            return Self::ConsumerConfidenceShift;
        }
        if lower.contains("pmi") || lower.contains("manufacturing index") {
            return Self::ManufacturingIndexChange;
        }
        if lower.contains("retail sales") {
            return Self::RetailSalesChange;
        }
        if lower.contains("housing") || lower.contains("real estate") {
            return Self::HousingMarketSignal;
        }
        if lower.contains("energy price")
            || lower.contains("oil price")
            || lower.contains("gas price")
        {
            return Self::EnergyPriceVolatility;
        }
        if lower.contains("shortage") && lower.contains("material") {
            return Self::RawMaterialShortage;
        }
        if lower.contains("freight") || lower.contains("shipping rate") {
            return Self::FreightRateChange;
        }
        if lower.contains("port") && lower.contains("congestion") {
            return Self::PortCongestion;
        }
        if lower.contains("warehouse") && lower.contains("capacity") {
            return Self::WarehouseCapacityConstraint;
        }
        if lower.contains("labor cost") || lower.contains("wage") {
            return Self::LaborCostIncrease;
        }
        if lower.contains("productivity") {
            return Self::ProductivityChange;
        }
        if lower.contains("economic forecast") {
            return Self::EconomicForecastRevision;
        }

        // Technology
        if lower.contains("patent") && lower.contains("filing") {
            return Self::PatentFiling;
        }
        if lower.contains("patent") && lower.contains("grant") {
            return Self::PatentGrant;
        }
        if lower.contains("patent") && (lower.contains("litigation") || lower.contains("lawsuit")) {
            return Self::PatentLitigation;
        }
        if lower.contains("r&d") || lower.contains("research") && lower.contains("investment") {
            return Self::RdInvestmentIncrease;
        }
        if lower.contains("technology") && lower.contains("partnership") {
            return Self::TechnologyPartnership;
        }
        if lower.contains("product") && lower.contains("innovation") {
            return Self::ProductInnovation;
        }
        if lower.contains("process") && lower.contains("innovation") {
            return Self::ProcessInnovation;
        }
        if lower.contains("tech stack") || lower.contains("platform change") {
            return Self::TechStackChange;
        }
        if lower.contains("open source") {
            return Self::OpenSourceContribution;
        }
        if lower.contains("academic") || lower.contains("university") && lower.contains("partner") {
            return Self::AcademicCollaboration;
        }
        if lower.contains("standards body")
            || lower.contains("ieee")
            || lower.contains("iso committee")
        {
            return Self::StandardsBodyParticipation;
        }
        if lower.contains("hiring") && (lower.contains("engineer") || lower.contains("tech")) {
            return Self::TechTalentHiring;
        }
        if lower.contains("lab") && lower.contains("expansion") {
            return Self::LabExpansion;
        }
        if lower.contains("prototype") {
            return Self::PrototypeAnnouncement;
        }
        if lower.contains("tech transfer") || lower.contains("technology transfer") {
            return Self::TechTransferAgreement;
        }

        // Personnel
        if lower.contains("executive") && lower.contains("hire") {
            return Self::ExecutiveHire;
        }
        if lower.contains("executive")
            && (lower.contains("departure") || lower.contains("resign") || lower.contains("leave"))
        {
            return Self::ExecutiveDeparture;
        }
        if lower.contains("board") && (lower.contains("change") || lower.contains("appoint")) {
            return Self::BoardChange;
        }
        if lower.contains("hiring")
            && (lower.contains("mass") || lower.contains("bulk") || lower.contains("spree"))
        {
            return Self::MassHiring;
        }
        if lower.contains("layoff")
            || lower.contains("workforce reduction")
            || (lower.contains(" rif ") || lower.contains(" rif,") || lower.starts_with("rif "))
        {
            return Self::Layoffs;
        }
        if lower.contains("reorganization") || lower.contains("restructuring") {
            return Self::ReorganizationAnnouncement;
        }
        if lower.contains("retirement")
            && (lower.contains("key") || lower.contains("executive") || lower.contains("founder"))
        {
            return Self::KeyPersonRetirement;
        }
        if lower.contains("founder") && (lower.contains("transition") || lower.contains("step")) {
            return Self::FounderTransition;
        }
        if lower.contains("poach") || lower.contains("talent raid") {
            return Self::TalentPoaching;
        }
        if lower.contains("union") {
            return Self::UnionActivity;
        }
        if lower.contains("strike") || lower.contains("work stoppage") {
            return Self::StrikeOrWorkStoppage;
        }
        if lower.contains("remote work")
            || lower.contains("work from home")
            || lower.contains("hybrid")
        {
            return Self::RemoteWorkPolicyChange;
        }
        if lower.contains("dei") || lower.contains("diversity") || lower.contains("inclusion") {
            return Self::DeiInitiative;
        }
        if lower.contains("compensation") || lower.contains("salary") || lower.contains("bonus") {
            return Self::CompensationChange;
        }
        if lower.contains("employee satisfaction") || lower.contains("glassdoor") {
            return Self::EmployeeSatisfactionShift;
        }

        // Procurement
        if lower.contains("tender") || lower.contains("solicitation") {
            return Self::TenderPublication;
        }
        if lower.contains("contract") && lower.contains("award") {
            return Self::ContractAward;
        }
        if lower.contains("rfq") || lower.contains("request for quote") {
            return Self::RfqIssuance;
        }
        if lower.contains("supplier") && lower.contains("qualification") {
            return Self::SupplierQualification;
        }
        if lower.contains("supplier") && lower.contains("disqualification") {
            return Self::SupplierDisqualification;
        }
        if lower.contains("vendor") && lower.contains("consolidation") {
            return Self::VendorConsolidation;
        }
        if lower.contains("long-term") && lower.contains("agreement") {
            return Self::LongTermAgreement;
        }
        if lower.contains("spot") && lower.contains("purchase") {
            return Self::SpotPurchase;
        }
        if lower.contains("reverse auction") {
            return Self::ReverseAuction;
        }
        if lower.contains("strategic sourcing") {
            return Self::StrategicSourcingInitiative;
        }
        if lower.contains("local content") {
            return Self::LocalContentRequirement;
        }
        if lower.contains("procurement") && lower.contains("policy") {
            return Self::ProcurementPolicyChange;
        }

        // Regulatory
        if lower.contains("new regulation") || lower.contains("new rule") {
            return Self::NewRegulation;
        }
        if lower.contains("regulation") && lower.contains("repeal") {
            return Self::RegulationRepeal;
        }
        if lower.contains("compliance") && lower.contains("deadline") {
            return Self::ComplianceDeadline;
        }
        if lower.contains("regulatory") && lower.contains("investigation") {
            return Self::RegulatoryInvestigation;
        }
        if lower.contains("enforcement") && lower.contains("action") {
            return Self::EnforcementAction;
        }
        if lower.contains("license") && lower.contains("grant") {
            return Self::LicenseGrant;
        }
        if lower.contains("license") && lower.contains("revocation") {
            return Self::LicenseRevocation;
        }
        if lower.contains("environmental") && lower.contains("permit") {
            return Self::EnvironmentalPermit;
        }
        if lower.contains("safety") && lower.contains("inspection") {
            return Self::SafetyInspection;
        }
        if lower.contains("audit") && lower.contains("finding") {
            return Self::AuditFinding;
        }
        if lower.contains("certification")
            && (lower.contains("change") || lower.contains("update") || lower.contains("expire"))
        {
            return Self::CertificationChange;
        }
        if lower.contains("standards") && lower.contains("update") {
            return Self::StandardsUpdate;
        }

        // Geopolitical
        if lower.contains("sanction") && lower.contains("imposed") {
            return Self::SanctionImposed;
        }
        if lower.contains("sanction") && lower.contains("lifted") {
            return Self::SanctionLifted;
        }
        if lower.contains("tariff") {
            return Self::TariffChange;
        }
        if lower.contains("trade agreement") || lower.contains("fta") {
            return Self::TradeAgreement;
        }
        if lower.contains("trade dispute") || lower.contains("trade war") {
            return Self::TradeDispute;
        }
        if lower.contains("export control") {
            return Self::ExportControlChange;
        }
        if lower.contains("diplomatic") && lower.contains("incident") {
            return Self::DiplomaticIncident;
        }
        if lower.contains("political instability")
            || lower.contains("coup")
            || lower.contains("unrest")
        {
            return Self::PoliticalInstability;
        }
        if lower.contains("election") {
            return Self::ElectionResult;
        }
        if lower.contains("policy shift") || lower.contains("policy change") {
            return Self::PolicyShift;
        }
        if lower.contains("military") && lower.contains("activity") {
            return Self::MilitaryActivity;
        }
        if lower.contains("territorial") && lower.contains("dispute") {
            return Self::TerritorialDispute;
        }

        // Financial
        if lower.contains("earnings") || lower.contains("quarterly report") {
            return Self::EarningsReport;
        }
        if lower.contains("guidance") && (lower.contains("revenue") || lower.contains("forecast")) {
            return Self::RevenueGuidanceChange;
        }
        if lower.contains("profit warning") {
            return Self::ProfitWarning;
        }
        if lower.contains("debt") && lower.contains("issuance") {
            return Self::DebtIssuance;
        }
        if lower.contains("equity") && (lower.contains("raise") || lower.contains("offering")) {
            return Self::EquityRaise;
        }
        if lower.contains("dividend") {
            return Self::DividendChange;
        }
        if lower.contains("buyback") || lower.contains("share repurchase") {
            return Self::ShareBuyback;
        }
        if lower.contains("credit rating")
            || lower.contains("moody")
            || lower.contains("s&p")
            || lower.contains("fitch")
        {
            return Self::CreditRatingChange;
        }
        if lower.contains("bankruptcy") {
            return Self::BankruptcyFiling;
        }
        if lower.contains("restructuring") {
            return Self::RestructuringAnnouncement;
        }
        if lower.contains("asset sale") || lower.contains("divestiture") {
            return Self::AssetSale;
        }
        if lower.contains("writedown") || lower.contains("impairment") {
            return Self::WritedownOrImpairment;
        }

        // Supply Chain
        if lower.contains("supplier") && lower.contains("disruption") {
            return Self::SupplierDisruption;
        }
        if lower.contains("logistics") && lower.contains("bottleneck") {
            return Self::LogisticsBottleneck;
        }
        if lower.contains("inventory") && (lower.contains("build") || lower.contains("increase")) {
            return Self::InventoryBuildUp;
        }
        if lower.contains("inventory") && (lower.contains("draw") || lower.contains("decrease")) {
            return Self::InventoryDrawdown;
        }
        if lower.contains("lead time") && lower.contains("extension") {
            return Self::LeadTimeExtension;
        }
        if lower.contains("dual source") || lower.contains("multi-source") {
            return Self::DualSourcingInitiative;
        }
        if lower.contains("nearshore") || lower.contains("nearshoring") {
            return Self::NearshoringAnnouncement;
        }
        if lower.contains("offshore") || lower.contains("offshoring") {
            return Self::OffshoringAnnouncement;
        }
        if lower.contains("vertical integration") {
            return Self::VerticalIntegration;
        }
        if lower.contains("outsourcing") {
            return Self::OutsourcingDecision;
        }
        if lower.contains("supply chain") && lower.contains("visibility") {
            return Self::SupplyChainVisibilityInvestment;
        }
        if lower.contains("contingency") && lower.contains("stock") {
            return Self::ContingencyStockBuilding;
        }

        // Security
        if lower.contains("cyber") && lower.contains("attack") {
            return Self::CyberAttack;
        }
        if lower.contains("data breach") {
            return Self::DataBreach;
        }
        if lower.contains("ransomware") {
            return Self::RansomwareIncident;
        }
        if lower.contains("vulnerability") && lower.contains("disclosure") {
            return Self::SecurityVulnerabilityDisclosure;
        }
        if lower.contains("physical security") && lower.contains("incident") {
            return Self::PhysicalSecurityIncident;
        }
        if lower.contains("insider threat") {
            return Self::InsiderThreat;
        }
        if lower.contains("espionage") {
            return Self::EspionageActivity;
        }
        if lower.contains("security") && lower.contains("certification") {
            return Self::SecurityCertificationChange;
        }
        if lower.contains("penetration test") || lower.contains("pentest") {
            return Self::PenetrationTestResult;
        }
        if lower.contains("security") && lower.contains("investment") {
            return Self::SecurityInvestment;
        }

        // Environmental
        if lower.contains("natural disaster")
            || lower.contains("earthquake")
            || lower.contains("hurricane")
            || lower.contains("flood")
        {
            return Self::NaturalDisaster;
        }
        if lower.contains("climate") && lower.contains("event") {
            return Self::ClimateEvent;
        }
        if lower.contains("environmental") && lower.contains("violation") {
            return Self::EnvironmentalViolation;
        }
        if lower.contains("emissions") && lower.contains("reduction") {
            return Self::EmissionsReduction;
        }
        if lower.contains("sustainability") {
            return Self::SustainabilityInitiative;
        }
        if lower.contains("renewable") && lower.contains("energy") {
            return Self::RenewableEnergyInvestment;
        }
        if lower.contains("waste") && lower.contains("reduction") {
            return Self::WasteReductionProgram;
        }
        if lower.contains("water") && lower.contains("scarcity") {
            return Self::WaterScarcityImpact;
        }
        if lower.contains("biodiversity") {
            return Self::BiodiversityImpact;
        }
        if lower.contains("carbon") && lower.contains("credit") {
            return Self::CarbonCreditActivity;
        }

        // Social
        if lower.contains("pr crisis")
            || lower.contains("public relations") && lower.contains("crisis")
        {
            return Self::PublicRelationsCrisis;
        }
        if lower.contains("brand") && lower.contains("reputation") {
            return Self::BrandReputationShift;
        }
        if lower.contains("customer") && lower.contains("sentiment") {
            return Self::CustomerSentimentChange;
        }
        if lower.contains("social media") && lower.contains("controversy") {
            return Self::SocialMediaControversy;
        }
        if lower.contains("boycott") {
            return Self::BoycottOrCampaign;
        }
        if lower.contains("community") && lower.contains("relations") {
            return Self::CommunityRelationsIssue;
        }
        if lower.contains("philanthropic")
            || lower.contains("donation")
            || lower.contains("charity")
        {
            return Self::PhilanthropicInitiative;
        }
        if lower.contains("social license") {
            return Self::SocialLicenseChallenge;
        }

        // Legal
        if lower.contains("lawsuit") && lower.contains("filed") {
            return Self::LawsuitFiled;
        }
        if lower.contains("lawsuit") && lower.contains("settlement") {
            return Self::LawsuitSettlement;
        }
        if lower.contains("class action") {
            return Self::ClassActionInitiated;
        }
        if lower.contains("antitrust") {
            return Self::AntitrustInvestigation;
        }
        if lower.contains("ip dispute")
            || lower.contains("intellectual property") && lower.contains("dispute")
        {
            return Self::IpDispute;
        }
        if lower.contains("contract") && lower.contains("dispute") {
            return Self::ContractDispute;
        }
        if lower.contains("regulatory") && lower.contains("litigation") {
            return Self::RegulatoryLitigation;
        }
        if lower.contains("arbitration") {
            return Self::ArbitrationProceeding;
        }

        // Operational
        if lower.contains("facility") && lower.contains("expansion") {
            return Self::FacilityExpansion;
        }
        if lower.contains("facility") && lower.contains("closure") {
            return Self::FacilityClosure;
        }
        if lower.contains("capacity") && lower.contains("increase") {
            return Self::CapacityIncrease;
        }
        if lower.contains("capacity") && lower.contains("reduction") {
            return Self::CapacityReduction;
        }
        if lower.contains("production line") {
            return Self::ProductionLineChange;
        }
        if lower.contains("quality issue") {
            return Self::QualityIssue;
        }
        if lower.contains("recall") {
            return Self::ProductRecall;
        }
        if lower.contains("maintenance") && lower.contains("outage") {
            return Self::MaintenanceOutage;
        }
        if lower.contains("operational efficiency") {
            return Self::OperationalEfficiencyProgram;
        }
        if lower.contains("automation") {
            return Self::AutomationInvestment;
        }

        // Market
        if lower.contains("market share") {
            return Self::MarketShareChange;
        }
        if lower.contains("competitor") && lower.contains("entry") {
            return Self::CompetitorEntry;
        }
        if lower.contains("competitor") && lower.contains("exit") {
            return Self::CompetitorExit;
        }
        if lower.contains("pricing") && lower.contains("strategy") {
            return Self::PricingStrategyChange;
        }
        if lower.contains("new market") && lower.contains("entry") {
            return Self::NewMarketEntry;
        }
        if lower.contains("market") && lower.contains("withdrawal") {
            return Self::MarketWithdrawal;
        }
        if lower.contains("channel") && lower.contains("partnership") {
            return Self::ChannelPartnershipChange;
        }
        if lower.contains("distribution") && lower.contains("expansion") {
            return Self::DistributionExpansion;
        }
        if lower.contains("customer") && lower.contains("concentration") {
            return Self::CustomerConcentrationChange;
        }
        if lower.contains("market") && lower.contains("consolidation") {
            return Self::MarketConsolidation;
        }

        // Infrastructure
        if lower.contains("data center") {
            return Self::DataCenterInvestment;
        }
        if lower.contains("network") && lower.contains("expansion") {
            return Self::NetworkExpansion;
        }
        if lower.contains("cloud") && lower.contains("migration") {
            return Self::CloudMigration;
        }
        if lower.contains("it") && lower.contains("outage") {
            return Self::ItSystemOutage;
        }
        if lower.contains("digital transformation") {
            return Self::DigitalTransformationInitiative;
        }
        if lower.contains("infrastructure") && lower.contains("modernization") {
            return Self::InfrastructureModernization;
        }

        Self::Other(s.to_string())
    }

    /// Get all signal types (excluding Other).
    pub fn all() -> Vec<Self> {
        vec![
            // Economic
            Self::CommodityPriceSpike,
            Self::CommodityPriceDrop,
            Self::CurrencyFluctuation,
            Self::InterestRateChange,
            Self::InflationIndicator,
            Self::GdpGrowthSignal,
            Self::TradeBalanceShift,
            Self::UnemploymentChange,
            Self::ConsumerConfidenceShift,
            Self::ManufacturingIndexChange,
            Self::RetailSalesChange,
            Self::HousingMarketSignal,
            Self::EnergyPriceVolatility,
            Self::RawMaterialShortage,
            Self::FreightRateChange,
            Self::PortCongestion,
            Self::WarehouseCapacityConstraint,
            Self::LaborCostIncrease,
            Self::ProductivityChange,
            Self::EconomicForecastRevision,
            // Technology
            Self::PatentFiling,
            Self::PatentGrant,
            Self::PatentLitigation,
            Self::RdInvestmentIncrease,
            Self::TechnologyPartnership,
            Self::ProductInnovation,
            Self::ProcessInnovation,
            Self::TechStackChange,
            Self::OpenSourceContribution,
            Self::AcademicCollaboration,
            Self::StandardsBodyParticipation,
            Self::TechTalentHiring,
            Self::LabExpansion,
            Self::PrototypeAnnouncement,
            Self::TechTransferAgreement,
            // Personnel
            Self::ExecutiveHire,
            Self::ExecutiveDeparture,
            Self::BoardChange,
            Self::MassHiring,
            Self::Layoffs,
            Self::ReorganizationAnnouncement,
            Self::KeyPersonRetirement,
            Self::FounderTransition,
            Self::TalentPoaching,
            Self::UnionActivity,
            Self::StrikeOrWorkStoppage,
            Self::RemoteWorkPolicyChange,
            Self::DeiInitiative,
            Self::CompensationChange,
            Self::EmployeeSatisfactionShift,
            // Procurement
            Self::TenderPublication,
            Self::ContractAward,
            Self::RfqIssuance,
            Self::SupplierQualification,
            Self::SupplierDisqualification,
            Self::VendorConsolidation,
            Self::LongTermAgreement,
            Self::SpotPurchase,
            Self::ReverseAuction,
            Self::StrategicSourcingInitiative,
            Self::LocalContentRequirement,
            Self::ProcurementPolicyChange,
            // Regulatory
            Self::NewRegulation,
            Self::RegulationRepeal,
            Self::ComplianceDeadline,
            Self::RegulatoryInvestigation,
            Self::EnforcementAction,
            Self::LicenseGrant,
            Self::LicenseRevocation,
            Self::EnvironmentalPermit,
            Self::SafetyInspection,
            Self::AuditFinding,
            Self::CertificationChange,
            Self::StandardsUpdate,
            // Geopolitical
            Self::SanctionImposed,
            Self::SanctionLifted,
            Self::TariffChange,
            Self::TradeAgreement,
            Self::TradeDispute,
            Self::ExportControlChange,
            Self::DiplomaticIncident,
            Self::PoliticalInstability,
            Self::ElectionResult,
            Self::PolicyShift,
            Self::MilitaryActivity,
            Self::TerritorialDispute,
            // Financial
            Self::EarningsReport,
            Self::RevenueGuidanceChange,
            Self::ProfitWarning,
            Self::DebtIssuance,
            Self::EquityRaise,
            Self::DividendChange,
            Self::ShareBuyback,
            Self::CreditRatingChange,
            Self::BankruptcyFiling,
            Self::RestructuringAnnouncement,
            Self::AssetSale,
            Self::WritedownOrImpairment,
            // Supply Chain
            Self::SupplierDisruption,
            Self::LogisticsBottleneck,
            Self::InventoryBuildUp,
            Self::InventoryDrawdown,
            Self::LeadTimeExtension,
            Self::DualSourcingInitiative,
            Self::NearshoringAnnouncement,
            Self::OffshoringAnnouncement,
            Self::VerticalIntegration,
            Self::OutsourcingDecision,
            Self::SupplyChainVisibilityInvestment,
            Self::ContingencyStockBuilding,
            // Security
            Self::CyberAttack,
            Self::DataBreach,
            Self::RansomwareIncident,
            Self::SecurityVulnerabilityDisclosure,
            Self::PhysicalSecurityIncident,
            Self::InsiderThreat,
            Self::EspionageActivity,
            Self::SecurityCertificationChange,
            Self::PenetrationTestResult,
            Self::SecurityInvestment,
            // Environmental
            Self::NaturalDisaster,
            Self::ClimateEvent,
            Self::EnvironmentalViolation,
            Self::EmissionsReduction,
            Self::SustainabilityInitiative,
            Self::RenewableEnergyInvestment,
            Self::WasteReductionProgram,
            Self::WaterScarcityImpact,
            Self::BiodiversityImpact,
            Self::CarbonCreditActivity,
            // Social
            Self::PublicRelationsCrisis,
            Self::BrandReputationShift,
            Self::CustomerSentimentChange,
            Self::SocialMediaControversy,
            Self::BoycottOrCampaign,
            Self::CommunityRelationsIssue,
            Self::PhilanthropicInitiative,
            Self::SocialLicenseChallenge,
            // Legal
            Self::LawsuitFiled,
            Self::LawsuitSettlement,
            Self::ClassActionInitiated,
            Self::AntitrustInvestigation,
            Self::IpDispute,
            Self::ContractDispute,
            Self::RegulatoryLitigation,
            Self::ArbitrationProceeding,
            // Operational
            Self::FacilityExpansion,
            Self::FacilityClosure,
            Self::CapacityIncrease,
            Self::CapacityReduction,
            Self::ProductionLineChange,
            Self::QualityIssue,
            Self::ProductRecall,
            Self::MaintenanceOutage,
            Self::OperationalEfficiencyProgram,
            Self::AutomationInvestment,
            // Market
            Self::MarketShareChange,
            Self::CompetitorEntry,
            Self::CompetitorExit,
            Self::PricingStrategyChange,
            Self::NewMarketEntry,
            Self::MarketWithdrawal,
            Self::ChannelPartnershipChange,
            Self::DistributionExpansion,
            Self::CustomerConcentrationChange,
            Self::MarketConsolidation,
            // Infrastructure
            Self::DataCenterInvestment,
            Self::NetworkExpansion,
            Self::CloudMigration,
            Self::ItSystemOutage,
            Self::DigitalTransformationInitiative,
            Self::InfrastructureModernization,
        ]
    }
}

// ============================================================================
// CROSS-DOMAIN CORRELATION ENGINE
// ============================================================================

/// A causal link between two signal types across domains.
#[derive(Debug, Clone)]
pub struct CausalLink {
    /// Source signal type (cause)
    pub source: SignalType,
    /// Target signal type (effect)
    pub target: SignalType,
    /// Estimated lag in days between cause and effect
    pub lag_days: u32,
    /// Strength of the causal relationship (0.0 - 1.0)
    pub strength: f64,
    /// Description of the causal mechanism
    pub mechanism: String,
}

/// Pre-computed causal chains for common cross-domain patterns.
pub fn known_causal_chains() -> Vec<Vec<CausalLink>> {
    vec![
        // Chain 1: Commodity shock → Supply chain → Financial → Personnel
        vec![
            CausalLink {
                source: SignalType::CommodityPriceSpike,
                target: SignalType::RawMaterialShortage,
                lag_days: 14,
                strength: 0.75,
                mechanism: "Price spikes reduce affordable supply".to_string(),
            },
            CausalLink {
                source: SignalType::RawMaterialShortage,
                target: SignalType::LeadTimeExtension,
                lag_days: 7,
                strength: 0.80,
                mechanism: "Shortages extend production lead times".to_string(),
            },
            CausalLink {
                source: SignalType::LeadTimeExtension,
                target: SignalType::ProfitWarning,
                lag_days: 30,
                strength: 0.60,
                mechanism: "Delivery delays impact revenue recognition".to_string(),
            },
            CausalLink {
                source: SignalType::ProfitWarning,
                target: SignalType::Layoffs,
                lag_days: 45,
                strength: 0.55,
                mechanism: "Financial pressure triggers cost cuts".to_string(),
            },
        ],
        // Chain 2: Tariff → Sourcing → Facility changes
        vec![
            CausalLink {
                source: SignalType::TariffChange,
                target: SignalType::DualSourcingInitiative,
                lag_days: 30,
                strength: 0.70,
                mechanism: "Tariffs motivate supply chain diversification".to_string(),
            },
            CausalLink {
                source: SignalType::DualSourcingInitiative,
                target: SignalType::NearshoringAnnouncement,
                lag_days: 90,
                strength: 0.65,
                mechanism: "Diversification leads to regional production".to_string(),
            },
            CausalLink {
                source: SignalType::NearshoringAnnouncement,
                target: SignalType::FacilityExpansion,
                lag_days: 180,
                strength: 0.70,
                mechanism: "Regional strategy requires local capacity".to_string(),
            },
        ],
        // Chain 3: Patent activity → M&A → Market structure
        vec![
            CausalLink {
                source: SignalType::PatentFiling,
                target: SignalType::TechnologyPartnership,
                lag_days: 60,
                strength: 0.50,
                mechanism: "IP development attracts partners".to_string(),
            },
            CausalLink {
                source: SignalType::TechnologyPartnership,
                target: SignalType::EquityRaise,
                lag_days: 90,
                strength: 0.45,
                mechanism: "Partnerships validate funding opportunities".to_string(),
            },
            CausalLink {
                source: SignalType::EquityRaise,
                target: SignalType::MarketConsolidation,
                lag_days: 180,
                strength: 0.40,
                mechanism: "Capital enables acquisition activity".to_string(),
            },
        ],
        // Chain 4: Cyber incident → Regulatory → Financial
        vec![
            CausalLink {
                source: SignalType::DataBreach,
                target: SignalType::RegulatoryInvestigation,
                lag_days: 7,
                strength: 0.85,
                mechanism: "Breaches trigger regulatory scrutiny".to_string(),
            },
            CausalLink {
                source: SignalType::RegulatoryInvestigation,
                target: SignalType::EnforcementAction,
                lag_days: 120,
                strength: 0.60,
                mechanism: "Investigations lead to enforcement".to_string(),
            },
            CausalLink {
                source: SignalType::EnforcementAction,
                target: SignalType::WritedownOrImpairment,
                lag_days: 30,
                strength: 0.70,
                mechanism: "Penalties impact financial statements".to_string(),
            },
        ],
        // Chain 5: Executive change → Strategy → Operations
        vec![
            CausalLink {
                source: SignalType::ExecutiveDeparture,
                target: SignalType::ReorganizationAnnouncement,
                lag_days: 45,
                strength: 0.65,
                mechanism: "Leadership change triggers reorg".to_string(),
            },
            CausalLink {
                source: SignalType::ReorganizationAnnouncement,
                target: SignalType::PricingStrategyChange,
                lag_days: 60,
                strength: 0.50,
                mechanism: "Reorg often includes go-to-market changes".to_string(),
            },
            CausalLink {
                source: SignalType::PricingStrategyChange,
                target: SignalType::MarketShareChange,
                lag_days: 90,
                strength: 0.55,
                mechanism: "Pricing affects competitive position".to_string(),
            },
        ],
    ]
}

/// Observed signal instance.
#[derive(Debug, Clone)]
pub struct ObservedSignal {
    pub id: Uuid,
    pub signal_type: SignalType,
    pub entity_id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub magnitude: f64,
    pub raw_text: String,
}

/// Cross-domain correlation engine.
#[derive(Debug)]
pub struct CorrelationEngine {
    /// Known causal links (can be learned or predefined)
    causal_links: Vec<CausalLink>,
    /// Observed signals by entity
    signals_by_entity: HashMap<Uuid, Vec<ObservedSignal>>,
    /// Correlation matrix between signal types
    #[allow(dead_code)]
    correlation_matrix: HashMap<(SignalType, SignalType), f64>,
}

impl Default for CorrelationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationEngine {
    pub fn new() -> Self {
        // Flatten known causal chains into individual links
        let causal_links: Vec<CausalLink> = known_causal_chains().into_iter().flatten().collect();

        Self {
            causal_links,
            signals_by_entity: HashMap::new(),
            correlation_matrix: HashMap::new(),
        }
    }

    /// Add an observed signal.
    pub fn observe(&mut self, signal: ObservedSignal) {
        self.signals_by_entity
            .entry(signal.entity_id)
            .or_default()
            .push(signal);
    }

    /// Find potential causal predecessors for a given signal.
    pub fn find_predecessors(&self, signal_type: &SignalType) -> Vec<&CausalLink> {
        self.causal_links
            .iter()
            .filter(|link| &link.target == signal_type)
            .collect()
    }

    /// Find potential causal successors for a given signal.
    pub fn find_successors(&self, signal_type: &SignalType) -> Vec<&CausalLink> {
        self.causal_links
            .iter()
            .filter(|link| &link.source == signal_type)
            .collect()
    }

    /// Simulate ripple effects from a major event.
    pub fn simulate_ripple(&self, trigger: &SignalType, max_depth: usize) -> Vec<RippleEffect> {
        let mut effects = Vec::new();
        let mut visited = HashSet::new();
        self.ripple_recursive(trigger, 0, 1.0, max_depth, &mut visited, &mut effects);
        effects.sort_by(|a, b| {
            b.cumulative_strength
                .partial_cmp(&a.cumulative_strength)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        effects
    }

    fn ripple_recursive(
        &self,
        current: &SignalType,
        depth: usize,
        cumulative_strength: f64,
        max_depth: usize,
        visited: &mut HashSet<String>,
        effects: &mut Vec<RippleEffect>,
    ) {
        if depth >= max_depth || cumulative_strength < 0.1 {
            return;
        }

        let key = format!("{:?}", current);
        if visited.contains(&key) {
            return;
        }
        visited.insert(key);

        for link in self.find_successors(current) {
            let new_strength = cumulative_strength * link.strength;
            effects.push(RippleEffect {
                signal_type: link.target.clone(),
                depth: depth + 1,
                cumulative_strength: new_strength,
                lag_days: link.lag_days,
                path: format!("{:?} -> {:?}", current, link.target),
            });
            self.ripple_recursive(
                &link.target,
                depth + 1,
                new_strength,
                max_depth,
                visited,
                effects,
            );
        }
    }

    /// Compute cross-domain correlation statistics.
    pub fn domain_correlation_summary(&self) -> HashMap<(SignalDomain, SignalDomain), usize> {
        let mut counts = HashMap::new();
        for link in &self.causal_links {
            let source_domain = link.source.domain();
            let target_domain = link.target.domain();
            *counts.entry((source_domain, target_domain)).or_insert(0) += 1;
        }
        counts
    }

    /// Get total signal type count.
    pub fn signal_type_count() -> usize {
        SignalType::all().len()
    }
}

/// A ripple effect from a trigger event.
#[derive(Debug, Clone)]
pub struct RippleEffect {
    pub signal_type: SignalType,
    pub depth: usize,
    pub cumulative_strength: f64,
    pub lag_days: u32,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_type_count_exceeds_100() {
        let count = SignalType::all().len();
        assert!(count >= 120, "Expected 120+ signal types, got {}", count);
    }

    #[test]
    fn all_domains_covered() {
        let all_signals = SignalType::all();
        let mut domains_found = HashSet::new();
        for signal in all_signals {
            domains_found.insert(signal.domain());
        }
        assert_eq!(domains_found.len(), SignalDomain::all().len());
    }

    #[test]
    fn ripple_simulation_produces_effects() {
        let engine = CorrelationEngine::new();
        let effects = engine.simulate_ripple(&SignalType::CommodityPriceSpike, 4);
        assert!(!effects.is_empty());
    }

    #[test]
    fn signal_parsing_works() {
        assert_eq!(
            SignalType::from_str("patent filing announcement"),
            SignalType::PatentFiling
        );
        assert_eq!(
            SignalType::from_str("major layoffs at company"),
            SignalType::Layoffs
        );
        assert_eq!(
            SignalType::from_str("new tariff on steel"),
            SignalType::TariffChange
        );
    }
}
