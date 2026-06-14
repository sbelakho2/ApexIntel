//! # Regulatory Intelligence Module
//!
//! Provides comprehensive regulatory intelligence capabilities including:
//! - Industry regulation tracking
//! - Environmental compliance monitoring
//! - Labor law changes detection
//! - Tax policy updates

use crate::models::{
    AlertType, CountryCode, GeopoliticalConfig, IntelligenceAlert,
    IntelligenceSource, Severity,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Regulation information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Regulation {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub regulation_type: RegulationType,
    pub jurisdiction: CountryCode,
    pub regulatory_body: String,
    pub effective_date: DateTime<Utc>,
    pub compliance_deadline: Option<DateTime<Utc>>,
    pub affected_sectors: Vec<String>,
    pub affected_activities: Vec<String>,
    pub requirements: Vec<ComplianceRequirement>,
    pub penalties: Vec<Penalty>,
    pub exemptions: Vec<Exemption>,
    pub related_regulations: Vec<Uuid>,
    pub source_url: Option<String>,
    pub last_reviewed: DateTime<Utc>,
    pub next_review: Option<DateTime<Utc>>,
    pub status: RegulationStatus,
}

impl Regulation {
    /// Create a new regulation
    pub fn new(
        title: String,
        regulation_type: RegulationType,
        jurisdiction: CountryCode,
        regulatory_body: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title,
            description: String::new(),
            regulation_type,
            jurisdiction,
            regulatory_body,
            effective_date: now,
            compliance_deadline: None,
            affected_sectors: Vec::new(),
            affected_activities: Vec::new(),
            requirements: Vec::new(),
            penalties: Vec::new(),
            exemptions: Vec::new(),
            related_regulations: Vec::new(),
            source_url: None,
            last_reviewed: now,
            next_review: None,
            status: RegulationStatus::Proposed,
        }
    }

    /// Check if regulation is active
    pub fn is_active(&self) -> bool {
        self.status == RegulationStatus::Active && self.effective_date <= Utc::now()
    }

    /// Check if compliance deadline is approaching
    pub fn is_deadline_approaching(&self, days_threshold: i64) -> bool {
        self.compliance_deadline
            .map(|d| {
                let diff = (d - Utc::now()).num_days();
                diff >= 0 && diff <= days_threshold
            })
            .unwrap_or(false)
    }

    /// Get compliance status for a requirement
    pub fn check_compliance(&self, requirement_id: &Uuid) -> Option<ComplianceStatus> {
        self.requirements.iter()
            .find(|r| &r.id == requirement_id)
            .map(|r| r.status)
    }

    /// Add a compliance requirement
    pub fn add_requirement(&mut self, req: ComplianceRequirement) {
        self.requirements.push(req);
    }
}

/// Regulation types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegulationType {
    Environmental,
    Labor,
    Tax,
    Financial,
    Healthcare,
    FoodSafety,
    ProductSafety,
    DataPrivacy,
    Cybersecurity,
    Antitrust,
    ConsumerProtection,
    ExportControl,
    ImportControl,
    Sanctions,
    #[serde(rename = "AML_KYC")]
    AmlKyc,
    ESG,
    CarbonEmissions,
    WasteManagement,
    WaterUsage,
    EnergyEfficiency,
    OccupationalHealth,
    Employment,
    Immigration,
    CorporateGovernance,
    Securities,
    Insurance,
    Telecommunications,
    Transportation,
    Agriculture,
    Manufacturing,
    Construction,
    Mining,
    Other,
}

impl RegulationType {
    pub fn category(&self) -> &'static str {
        match self {
            RegulationType::Environmental | RegulationType::CarbonEmissions |
            RegulationType::WasteManagement | RegulationType::WaterUsage |
            RegulationType::EnergyEfficiency => "Environmental",
            
            RegulationType::Labor | RegulationType::OccupationalHealth |
            RegulationType::Employment | RegulationType::Immigration => "Labor",
            
            RegulationType::Tax => "Tax",
            
            RegulationType::Financial | RegulationType::AmlKyc |
            RegulationType::Securities | RegulationType::Insurance => "Financial",
            
            RegulationType::DataPrivacy | RegulationType::Cybersecurity => "Digital",
            
            RegulationType::Healthcare | RegulationType::FoodSafety => "Health & Safety",
            
            _ => "General",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            RegulationType::Environmental => "Environmental Regulation",
            RegulationType::Labor => "Labor Law",
            RegulationType::Tax => "Tax Regulation",
            RegulationType::Financial => "Financial Regulation",
            RegulationType::Healthcare => "Healthcare Regulation",
            RegulationType::FoodSafety => "Food Safety Regulation",
            RegulationType::ProductSafety => "Product Safety Regulation",
            RegulationType::DataPrivacy => "Data Privacy Regulation",
            RegulationType::Cybersecurity => "Cybersecurity Regulation",
            RegulationType::Antitrust => "Antitrust/Competition Law",
            RegulationType::ConsumerProtection => "Consumer Protection",
            RegulationType::ExportControl => "Export Control",
            RegulationType::ImportControl => "Import Control",
            RegulationType::Sanctions => "Sanctions Regulations",
            RegulationType::AmlKyc => "AML/KYC Requirements",
            RegulationType::ESG => "ESG Reporting",
            RegulationType::CarbonEmissions => "Carbon Emissions Standards",
            RegulationType::WasteManagement => "Waste Management",
            RegulationType::WaterUsage => "Water Usage Regulations",
            RegulationType::EnergyEfficiency => "Energy Efficiency Standards",
            RegulationType::OccupationalHealth => "Occupational Health & Safety",
            RegulationType::Employment => "Employment Law",
            RegulationType::Immigration => "Immigration Law",
            RegulationType::CorporateGovernance => "Corporate Governance",
            RegulationType::Securities => "Securities Regulation",
            RegulationType::Insurance => "Insurance Regulation",
            RegulationType::Telecommunications => "Telecommunications",
            RegulationType::Transportation => "Transportation",
            RegulationType::Agriculture => "Agriculture",
            RegulationType::Manufacturing => "Manufacturing",
            RegulationType::Construction => "Construction",
            RegulationType::Mining => "Mining",
            RegulationType::Other => "Other",
        }
    }
}

/// Regulation status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegulationStatus {
    Proposed,
    InEffect,
    Active,
    UnderReview,
    Amended,
    Superseded,
    Revoked,
    Expired,
}

impl RegulationStatus {
    pub fn description(&self) -> &'static str {
        match self {
            RegulationStatus::Proposed => "Proposed",
            RegulationStatus::InEffect => "In Effect",
            RegulationStatus::Active => "Active",
            RegulationStatus::UnderReview => "Under Review",
            RegulationStatus::Amended => "Amended",
            RegulationStatus::Superseded => "Superseded",
            RegulationStatus::Revoked => "Revoked",
            RegulationStatus::Expired => "Expired",
        }
    }
}

/// Compliance requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceRequirement {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub requirement_type: RequirementType,
    pub mandatory: bool,
    pub status: ComplianceStatus,
    pub evidence_required: Vec<String>,
    pub reporting_frequency: Option<ReportingFrequency>,
    pub next_due_date: Option<DateTime<Utc>>,
    pub completion_percentage: f32,
    pub notes: Vec<String>,
}

impl ComplianceRequirement {
    /// Create a new requirement
    pub fn new(title: String, requirement_type: RequirementType) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            description: String::new(),
            requirement_type,
            mandatory: true,
            status: ComplianceStatus::NotStarted,
            evidence_required: Vec::new(),
            reporting_frequency: None,
            next_due_date: None,
            completion_percentage: 0.0,
            notes: Vec::new(),
        }
    }

    /// Update completion
    pub fn update_completion(&mut self, percentage: f32) {
        self.completion_percentage = percentage.clamp(0.0, 100.0);
        self.status = match self.completion_percentage as u32 {
            0 => ComplianceStatus::NotStarted,
            1..=49 => ComplianceStatus::InProgress,
            50..=99 => ComplianceStatus::PartialCompliance,
            _ => ComplianceStatus::Compliant,
        };
    }
}

/// Requirement types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RequirementType {
    Reporting,
    Certification,
    License,
    Permit,
    Inspection,
    Audit,
    Training,
    Documentation,
    Disclosure,
    Registration,
    Notification,
    RecordKeeping,
    Testing,
    Monitoring,
    Assessment,
}

impl RequirementType {
    pub fn description(&self) -> &'static str {
        match self {
            RequirementType::Reporting => "Reporting Requirement",
            RequirementType::Certification => "Certification Requirement",
            RequirementType::License => "License Requirement",
            RequirementType::Permit => "Permit Requirement",
            RequirementType::Inspection => "Inspection Requirement",
            RequirementType::Audit => "Audit Requirement",
            RequirementType::Training => "Training Requirement",
            RequirementType::Documentation => "Documentation Requirement",
            RequirementType::Disclosure => "Disclosure Requirement",
            RequirementType::Registration => "Registration Requirement",
            RequirementType::Notification => "Notification Requirement",
            RequirementType::RecordKeeping => "Record Keeping",
            RequirementType::Testing => "Testing Requirement",
            RequirementType::Monitoring => "Monitoring Requirement",
            RequirementType::Assessment => "Assessment Requirement",
        }
    }
}

/// Compliance status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplianceStatus {
    Compliant,
    NonCompliant,
    PartialCompliance,
    InProgress,
    NotStarted,
    NotApplicable,
    UnderReview,
    Expired,
}

impl ComplianceStatus {
    pub fn is_compliant(&self) -> bool {
        matches!(self, ComplianceStatus::Compliant | ComplianceStatus::NotApplicable)
    }

    pub fn severity(&self) -> Severity {
        match self {
            ComplianceStatus::Compliant => Severity::Low,
            ComplianceStatus::NotApplicable => Severity::Low,
            ComplianceStatus::PartialCompliance => Severity::Medium,
            ComplianceStatus::InProgress => Severity::Medium,
            ComplianceStatus::NotStarted => Severity::High,
            ComplianceStatus::NonCompliant => Severity::Critical,
            ComplianceStatus::UnderReview => Severity::Medium,
            ComplianceStatus::Expired => Severity::High,
        }
    }
}

/// Reporting frequency
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportingFrequency {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    SemiAnnually,
    Annually,
    Biennially,
    OnDemand,
}

impl ReportingFrequency {
    pub fn description(&self) -> &'static str {
        match self {
            ReportingFrequency::Daily => "Daily",
            ReportingFrequency::Weekly => "Weekly",
            ReportingFrequency::Monthly => "Monthly",
            ReportingFrequency::Quarterly => "Quarterly",
            ReportingFrequency::SemiAnnually => "Semi-Annually",
            ReportingFrequency::Annually => "Annually",
            ReportingFrequency::Biennially => "Biennially",
            ReportingFrequency::OnDemand => "On Demand",
        }
    }
}

/// Penalty information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Penalty {
    pub penalty_type: PenaltyType,
    pub description: String,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub percentage: Option<f32>,
    pub imprisonment: Option<u32>,
    pub additional_sanctions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PenaltyType {
    Fine,
    Imprisonment,
    LicenseRevocation,
    AssetSeizure,
    ExportBan,
    ImportBan,
    Debarment,
    Blacklisting,
    CeaseAndDesist,
    Injunction,
    ComplianceOrder,
    PublicDisclosure,
    Reputational,
}

impl PenaltyType {
    pub fn description(&self) -> &'static str {
        match self {
            PenaltyType::Fine => "Monetary Fine",
            PenaltyType::Imprisonment => "Imprisonment",
            PenaltyType::LicenseRevocation => "License Revocation",
            PenaltyType::AssetSeizure => "Asset Seizure",
            PenaltyType::ExportBan => "Export Ban",
            PenaltyType::ImportBan => "Import Ban",
            PenaltyType::Debarment => "Debarment from Government Contracts",
            PenaltyType::Blacklisting => "Blacklisting",
            PenaltyType::CeaseAndDesist => "Cease and Desist Order",
            PenaltyType::Injunction => "Injunction",
            PenaltyType::ComplianceOrder => "Compliance Order",
            PenaltyType::PublicDisclosure => "Public Disclosure Requirement",
            PenaltyType::Reputational => "Reputational Damage",
        }
    }
}

/// Exemption information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exemption {
    pub title: String,
    pub description: String,
    pub eligibility_criteria: Vec<String>,
    pub application_required: bool,
    pub approval_authority: Option<String>,
}

/// Tax policy information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxPolicy {
    pub id: Uuid,
    pub country: CountryCode,
    pub tax_type: TaxType,
    pub title: String,
    pub description: String,
    pub rate: Option<f32>,
    pub rate_type: RateType,
    pub effective_date: DateTime<Utc>,
    pub affected_sectors: Vec<String>,
    pub affected_activities: Vec<String>,
    pub exemptions: Vec<TaxExemption>,
    pub deductions: Vec<TaxDeduction>,
    pub credits: Vec<TaxCredit>,
    pub source: IntelligenceSource,
    pub last_updated: DateTime<Utc>,
}

impl TaxPolicy {
    /// Calculate tax liability
    pub fn calculate_liability(&self, taxable_amount: f64) -> f64 {
        match self.rate {
            Some(rate) => taxable_amount * (rate as f64 / 100.0),
            None => 0.0,
        }
    }

    /// Check if sector is affected
    pub fn affects_sector(&self, sector: &str) -> bool {
        self.affected_sectors.iter().any(|s| s.to_lowercase() == sector.to_lowercase())
    }
}

/// Tax types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaxType {
    CorporateIncome,
    PersonalIncome,
    ValueAdded,
    GoodsServices,
    Excise,
    Property,
    CapitalGains,
    Withholding,
    Payroll,
    Environmental,
    DigitalServices,
    Luxury,
    Inheritance,
    Transfer,
    Stamp,
    Custom,
    CarbonTax,
    FinancialTransaction,
    Other,
}

impl TaxType {
    pub fn description(&self) -> &'static str {
        match self {
            TaxType::CorporateIncome => "Corporate Income Tax",
            TaxType::PersonalIncome => "Personal Income Tax",
            TaxType::ValueAdded => "Value Added Tax (VAT)",
            TaxType::GoodsServices => "Goods and Services Tax (GST)",
            TaxType::Excise => "Excise Duty",
            TaxType::Property => "Property Tax",
            TaxType::CapitalGains => "Capital Gains Tax",
            TaxType::Withholding => "Withholding Tax",
            TaxType::Payroll => "Payroll Tax",
            TaxType::Environmental => "Environmental Tax",
            TaxType::DigitalServices => "Digital Services Tax",
            TaxType::Luxury => "Luxury Tax",
            TaxType::Inheritance => "Inheritance Tax",
            TaxType::Transfer => "Transfer Tax",
            TaxType::Stamp => "Stamp Duty",
            TaxType::Custom => "Customs Duty",
            TaxType::CarbonTax => "Carbon Tax",
            TaxType::FinancialTransaction => "Financial Transaction Tax",
            TaxType::Other => "Other Tax",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RateType {
    Flat,
    Progressive,
    Marginal,
    Fixed,
    Percentage,
    UnitBased,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxExemption {
    pub title: String,
    pub description: String,
    pub threshold: Option<f64>,
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxDeduction {
    pub title: String,
    pub description: String,
    pub max_amount: Option<f64>,
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxCredit {
    pub title: String,
    pub description: String,
    pub credit_amount: f64,
    pub refundable: bool,
    pub conditions: Vec<String>,
}

/// Environmental compliance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentalCompliance {
    pub id: Uuid,
    pub country: CountryCode,
    pub regulation_type: RegulationType,
    pub title: String,
    pub description: String,
    pub emission_limits: Vec<EmissionLimit>,
    pub reporting_requirements: Vec<String>,
    pub monitoring_requirements: Vec<MonitoringRequirement>,
    pub permits_required: Vec<String>,
    pub compliance_deadlines: Vec<ComplianceDeadline>,
    pub penalties: Vec<Penalty>,
    pub last_updated: DateTime<Utc>,
}

impl EnvironmentalCompliance {
    /// Check if emissions are within limits
    pub fn check_emissions(&self, emission_type: &str, value: f64) -> Option<ComplianceStatus> {
        for limit in &self.emission_limits {
            if limit.emission_type.to_lowercase() == emission_type.to_lowercase() {
                return Some(if value <= limit.limit_value as f64 {
                    ComplianceStatus::Compliant
                } else {
                    ComplianceStatus::NonCompliant
                });
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionLimit {
    pub emission_type: String,
    pub limit_value: f32,
    pub unit: String,
    pub averaging_period: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringRequirement {
    pub parameter: String,
    pub frequency: MonitoringFrequency,
    pub method: String,
    pub equipment_required: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MonitoringFrequency {
    Continuous,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Annually,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceDeadline {
    pub milestone: String,
    pub deadline: DateTime<Utc>,
    pub description: String,
    pub status: ComplianceStatus,
}

/// Labor law change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaborLawChange {
    pub id: Uuid,
    pub country: CountryCode,
    pub change_type: LaborLawType,
    pub title: String,
    pub description: String,
    pub previous_requirement: Option<String>,
    pub new_requirement: String,
    pub effective_date: DateTime<Utc>,
    pub transition_period_end: Option<DateTime<Utc>>,
    pub affected_employers: Vec<String>,
    pub affected_workers: u64,
    pub employer_costs: Option<f64>,
    pub impact_assessment: LaborImpactAssessment,
    pub source: IntelligenceSource,
    pub last_updated: DateTime<Utc>,
}

impl LaborLawChange {
    /// Check if change is imminent
    pub fn is_imminent(&self, days_threshold: i64) -> bool {
        let diff = (self.effective_date - Utc::now()).num_days();
        diff >= 0 && diff <= days_threshold
    }

    /// Check if in transition period
    #[allow(dead_code)]
    pub fn in_transition_period(&self) -> bool {
        self.transition_period_end
            .map(|end| Utc::now() < end && Utc::now() > self.effective_date)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LaborLawType {
    MinimumWage,
    WorkingHours,
    Overtime,
    Leave,
    Benefits,
    HealthSafety,
    Discrimination,
    Termination,
    Contract,
    CollectiveBargaining,
    SocialSecurity,
    Pension,
    Healthcare,
    Training,
    RemoteWork,
    GigEconomy,
    UnionRights,
    StrikeAction,
    ChildLabor,
    ForcedLabor,
}

impl LaborLawType {
    pub fn description(&self) -> &'static str {
        match self {
            LaborLawType::MinimumWage => "Minimum Wage",
            LaborLawType::WorkingHours => "Working Hours",
            LaborLawType::Overtime => "Overtime Regulations",
            LaborLawType::Leave => "Leave Entitlements",
            LaborLawType::Benefits => "Employee Benefits",
            LaborLawType::HealthSafety => "Health & Safety",
            LaborLawType::Discrimination => "Anti-Discrimination",
            LaborLawType::Termination => "Termination Regulations",
            LaborLawType::Contract => "Contract Requirements",
            LaborLawType::CollectiveBargaining => "Collective Bargaining",
            LaborLawType::SocialSecurity => "Social Security",
            LaborLawType::Pension => "Pension Requirements",
            LaborLawType::Healthcare => "Healthcare Requirements",
            LaborLawType::Training => "Training Requirements",
            LaborLawType::RemoteWork => "Remote Work Regulations",
            LaborLawType::GigEconomy => "Gig Economy Regulations",
            LaborLawType::UnionRights => "Union Rights",
            LaborLawType::StrikeAction => "Strike Action Rules",
            LaborLawType::ChildLabor => "Child Labor Laws",
            LaborLawType::ForcedLabor => "Forced Labor Prevention",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaborImpactAssessment {
    pub employer_impact: f32,
    pub worker_impact: f32,
    pub economic_impact: f32,
    pub timeline: String,
    pub affected_sectors: Vec<String>,
    pub recommendations: Vec<String>,
}

/// Compliance tracking report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub id: Uuid,
    pub organization: String,
    pub jurisdiction: CountryCode,
    pub report_date: DateTime<Utc>,
    pub regulations_tracked: u32,
    pub requirements_tracked: u32,
    pub overall_compliance_score: f32,
    pub compliant_count: u32,
    pub non_compliant_count: u32,
    pub in_progress_count: u32,
    pub upcoming_deadlines: Vec<UpcomingDeadline>,
    pub risks: Vec<ComplianceRisk>,
    pub recommendations: Vec<String>,
    pub generated_by: String,
}

impl ComplianceReport {
    /// Calculate overall compliance score
    pub fn calculate_score(&mut self) {
        if self.requirements_tracked > 0 {
            self.overall_compliance_score = 
                (self.compliant_count as f32 / self.requirements_tracked as f32) * 100.0;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpcomingDeadline {
    pub regulation_id: Uuid,
    pub requirement_id: Uuid,
    pub title: String,
    pub deadline: DateTime<Utc>,
    pub days_remaining: i64,
    pub status: ComplianceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceRisk {
    pub risk_id: Uuid,
    pub regulation_id: Uuid,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub probability: f32,
    pub impact: f32,
    pub mitigation: Vec<String>,
}

/// Regulatory change update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegulatoryUpdate {
    pub id: Uuid,
    pub regulation_id: Uuid,
    pub update_type: UpdateType,
    pub title: String,
    pub description: String,
    pub changes_summary: Vec<String>,
    pub effective_date: Option<DateTime<Utc>>,
    pub source: IntelligenceSource,
    pub source_url: Option<String>,
    pub published_date: DateTime<Utc>,
    pub verified: bool,
    pub impact_score: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UpdateType {
    NewRegulation,
    Amendment,
    Repeal,
    Guidance,
    Interpretation,
    Enforcement,
    Exemption,
    Consultation,
    Review,
}

/// Regulatory intelligence client
pub struct RegulatoryClient {
    _http_client: Client,
    _config: GeopoliticalConfig,
    tracked_regulations: Vec<Regulation>,
    tax_policies: Vec<TaxPolicy>,
    environmental_compliance: Vec<EnvironmentalCompliance>,
    labor_law_changes: Vec<LaborLawChange>,
    regulatory_updates: Vec<RegulatoryUpdate>,
}

impl RegulatoryClient {
    /// Create a new regulatory client
    pub fn new(http_client: Client, config: GeopoliticalConfig) -> Self {
        Self {
            _http_client: http_client,
            _config: config,
            tracked_regulations: Self::get_default_regulations(),
            tax_policies: Self::get_default_tax_policies(),
            environmental_compliance: Vec::new(),
            labor_law_changes: Self::get_default_labor_changes(),
            regulatory_updates: Vec::new(),
        }
    }

    /// Get default tracked regulations
    #[allow(clippy::disallowed_methods)]
    fn get_default_regulations() -> Vec<Regulation> {
        vec![
            {
                let mut reg = Regulation::new(
                    "EU General Data Protection Regulation".to_string(),
                    RegulationType::DataPrivacy,
                    CountryCode::new("EU"),
                    "European Commission".to_string(),
                );
                reg.effective_date = DateTime::parse_from_rfc3339("2018-05-25T00:00:00Z")
                    .unwrap().with_timezone(&Utc);
                reg.status = RegulationStatus::Active;
                reg.description = "Comprehensive data protection framework for EU residents".to_string();
                reg.affected_sectors = vec!["Technology".to_string(), "Finance".to_string(), "Healthcare".to_string()];
                reg.add_requirement(ComplianceRequirement::new(
                    "Data Protection Officer Appointment".to_string(),
                    RequirementType::Documentation,
                ));
                reg
            },
            {
                let mut reg = Regulation::new(
                    "US Foreign Corrupt Practices Act".to_string(),
                    RegulationType::Antitrust,
                    CountryCode::new("US"),
                    "Department of Justice".to_string(),
                );
                reg.effective_date = DateTime::parse_from_rfc3339("1977-12-19T00:00:00Z")
                    .unwrap().with_timezone(&Utc);
                reg.status = RegulationStatus::Active;
                reg.description = "Anti-bribery legislation for US companies".to_string();
                reg
            },
            {
                let mut reg = Regulation::new(
                    "EU Carbon Border Adjustment Mechanism".to_string(),
                    RegulationType::CarbonEmissions,
                    CountryCode::new("EU"),
                    "European Commission".to_string(),
                );
                reg.effective_date = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                    .unwrap().with_timezone(&Utc);
                reg.status = RegulationStatus::Active;
                reg.compliance_deadline = Some(DateTime::parse_from_rfc3339("2025-12-31T00:00:00Z")
                    .unwrap().with_timezone(&Utc));
                reg.description = "Carbon pricing on imports to prevent carbon leakage".to_string();
                reg.affected_sectors = vec!["Steel".to_string(), "Cement".to_string(), "Aluminum".to_string()];
                reg
            },
        ]
    }

    /// Get default tax policies
    #[allow(clippy::disallowed_methods)]
    fn get_default_tax_policies() -> Vec<TaxPolicy> {
        vec![
            TaxPolicy {
                id: Uuid::new_v4(),
                country: CountryCode::new("US"),
                tax_type: TaxType::CorporateIncome,
                title: "US Federal Corporate Tax".to_string(),
                description: "Federal corporate income tax rates".to_string(),
                rate: Some(21.0),
                rate_type: RateType::Flat,
                effective_date: DateTime::parse_from_rfc3339("2018-01-01T00:00:00Z")
                    .unwrap().with_timezone(&Utc),
                affected_sectors: vec!["All".to_string()],
                affected_activities: vec!["Corporate Profits".to_string()],
                exemptions: Vec::new(),
                deductions: Vec::new(),
                credits: Vec::new(),
                source: IntelligenceSource::Government,
                last_updated: Utc::now(),
            },
            TaxPolicy {
                id: Uuid::new_v4(),
                country: CountryCode::new("EU"),
                tax_type: TaxType::ValueAdded,
                title: "EU Standard VAT Rate".to_string(),
                description: "EU Value Added Tax rates".to_string(),
                rate: Some(21.0),
                rate_type: RateType::Progressive,
                effective_date: DateTime::parse_from_rfc3339("1993-01-01T00:00:00Z")
                    .unwrap().with_timezone(&Utc),
                affected_sectors: vec!["All".to_string()],
                affected_activities: vec!["Sale of Goods".to_string(), "Services".to_string()],
                exemptions: vec![TaxExemption {
                    title: "Medical".to_string(),
                    description: "Healthcare services".to_string(),
                    threshold: None,
                    conditions: vec![],
                }],
                deductions: Vec::new(),
                credits: Vec::new(),
                source: IntelligenceSource::Government,
                last_updated: Utc::now(),
            },
        ]
    }

    /// Get default labor law changes
    #[allow(clippy::disallowed_methods)]
    fn get_default_labor_changes() -> Vec<LaborLawChange> {
        vec![
            LaborLawChange {
                id: Uuid::new_v4(),
                country: CountryCode::new("EU"),
                change_type: LaborLawType::RemoteWork,
                title: "EU Remote Work Directive".to_string(),
                description: "Framework for remote work arrangements".to_string(),
                previous_requirement: Some("No specific framework".to_string()),
                new_requirement: "Employers must establish remote work policies".to_string(),
                effective_date: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .unwrap().with_timezone(&Utc),
                transition_period_end: None,
                affected_employers: vec!["All Employers".to_string()],
                affected_workers: 150_000_000,
                employer_costs: Some(500.0),
                impact_assessment: LaborImpactAssessment::default(),
                source: IntelligenceSource::Government,
                last_updated: Utc::now(),
            },
        ]
    }

    /// Get tracked regulations
    pub fn get_regulations(&self) -> &[Regulation] {
        &self.tracked_regulations
    }

    /// Get regulation by ID
    pub fn get_regulation(&self, id: &Uuid) -> Option<&Regulation> {
        self.tracked_regulations.iter().find(|r| &r.id == id)
    }

    /// Get regulations by country
    pub fn get_regulations_by_country(&self, country: &CountryCode) -> Vec<&Regulation> {
        self.tracked_regulations.iter()
            .filter(|r| r.jurisdiction == *country)
            .collect()
    }

    /// Get regulations by type
    pub fn get_regulations_by_type(&self, reg_type: RegulationType) -> Vec<&Regulation> {
        self.tracked_regulations.iter()
            .filter(|r| r.regulation_type == reg_type)
            .collect()
    }

    /// Add regulation to track
    pub fn track_regulation(&mut self, regulation: Regulation) {
        self.tracked_regulations.push(regulation);
    }

    /// Get tax policies
    pub fn get_tax_policies(&self) -> &[TaxPolicy] {
        &self.tax_policies
    }

    /// Get tax policy by country
    pub fn get_tax_policies_by_country(&self, country: &CountryCode) -> Vec<&TaxPolicy> {
        self.tax_policies.iter()
            .filter(|p| p.country == *country)
            .collect()
    }

    /// Get environmental compliance data
    pub fn get_environmental_compliance(&self) -> &[EnvironmentalCompliance] {
        &self.environmental_compliance
    }

    /// Get labor law changes
    pub fn get_labor_law_changes(&self) -> &[LaborLawChange] {
        &self.labor_law_changes
    }

    /// Get labor law changes by country
    pub fn get_labor_changes_by_country(&self, country: &CountryCode) -> Vec<&LaborLawChange> {
        self.labor_law_changes.iter()
            .filter(|c| c.country == *country)
            .collect()
    }

    /// Add regulatory update
    pub fn add_update(&mut self, update: RegulatoryUpdate) {
        self.regulatory_updates.insert(0, update);
        self.regulatory_updates.truncate(1000);
    }

    /// Get recent updates
    pub fn get_recent_updates(&self, limit: usize) -> &[RegulatoryUpdate] {
        &self.regulatory_updates[..limit.min(self.regulatory_updates.len())]
    }

    /// Get upcoming deadlines
    pub fn get_upcoming_deadlines(&self, days: i64) -> Vec<UpcomingDeadline> {
        let cutoff = Utc::now() + chrono::Duration::days(days);
        let mut deadlines = Vec::new();

        for reg in &self.tracked_regulations {
            if let Some(deadline) = reg.compliance_deadline {
                if deadline <= cutoff && deadline > Utc::now() {
                    let days_remaining = (deadline - Utc::now()).num_days();
                    deadlines.push(UpcomingDeadline {
                        regulation_id: reg.id,
                        requirement_id: Uuid::new_v4(),
                        title: reg.title.clone(),
                        deadline,
                        days_remaining,
                        status: ComplianceStatus::InProgress,
                    });
                }
            }

            for req in &reg.requirements {
                if let Some(due) = req.next_due_date {
                    if due <= cutoff && due > Utc::now() {
                        let days_remaining = (due - Utc::now()).num_days();
                        deadlines.push(UpcomingDeadline {
                            regulation_id: reg.id,
                            requirement_id: req.id,
                            title: format!("{} - {}", reg.title, req.title),
                            deadline: due,
                            days_remaining,
                            status: req.status,
                        });
                    }
                }
            }
        }

        deadlines.sort_by_key(|d| d.deadline);
        deadlines
    }

    /// Check compliance for a sector
    #[allow(dead_code)]
    pub fn check_sector_compliance(
        &self,
        country: &CountryCode,
        sector: &str,
    ) -> Vec<(Regulation, ComplianceStatus)> {
        self.tracked_regulations.iter()
            .filter(|r| r.jurisdiction == *country && r.affected_sectors.iter().any(|s| s.to_lowercase() == sector.to_lowercase()))
            .map(|r| {
                let status = if r.is_active() {
                    ComplianceStatus::Compliant
                } else {
                    ComplianceStatus::UnderReview
                };
                (r.clone(), status)
            })
            .collect()
    }

    /// Generate compliance alert
    pub fn generate_compliance_alert(&self, regulation: &Regulation, severity: Severity) -> IntelligenceAlert {
        IntelligenceAlert::new(
            AlertType::RegulatoryChange,
            severity,
            format!("Compliance Alert: {}", regulation.title),
            regulation.description.clone(),
            IntelligenceSource::Government,
            vec![regulation.jurisdiction.clone()],
        )
    }

    /// Search regulations
    pub fn search_regulations(
        &self,
        query: &str,
        country: Option<&CountryCode>,
        reg_type: Option<RegulationType>,
    ) -> Vec<&Regulation> {
        let query_lower = query.to_lowercase();
        
        self.tracked_regulations.iter()
            .filter(|r| {
                let matches_query = r.title.to_lowercase().contains(&query_lower) ||
                    r.description.to_lowercase().contains(&query_lower);
                
                let matches_country = country.map(|c| r.jurisdiction == *c).unwrap_or(true);
                let matches_type = reg_type.map(|t| r.regulation_type == t).unwrap_or(true);
                
                matches_query && matches_country && matches_type
            })
            .collect()
    }

    /// Generate compliance report
    pub fn generate_compliance_report(
        &self,
        organization: &str,
        jurisdiction: &CountryCode,
    ) -> ComplianceReport {
        let regulations = self.get_regulations_by_country(jurisdiction);
        
        let mut compliant = 0u32;
        let mut non_compliant = 0u32;
        let mut in_progress = 0u32;
        let mut total_requirements = 0u32;
        
        for reg in &regulations {
            for req in &reg.requirements {
                total_requirements += 1;
                match req.status {
                    ComplianceStatus::Compliant => compliant += 1,
                    ComplianceStatus::NonCompliant => non_compliant += 1,
                    ComplianceStatus::InProgress | ComplianceStatus::PartialCompliance => in_progress += 1,
                    _ => {}
                }
            }
        }

        let upcoming_deadlines = self.get_upcoming_deadlines(30);
        
        let mut report = ComplianceReport {
            id: Uuid::new_v4(),
            organization: organization.to_string(),
            jurisdiction: jurisdiction.clone(),
            report_date: Utc::now(),
            regulations_tracked: regulations.len() as u32,
            requirements_tracked: total_requirements,
            overall_compliance_score: 0.0,
            compliant_count: compliant,
            non_compliant_count: non_compliant,
            in_progress_count: in_progress,
            upcoming_deadlines,
            risks: Vec::new(),
            recommendations: Vec::new(),
            generated_by: "Regulatory Intelligence Module".to_string(),
        };
        
        report.calculate_score();
        
        if non_compliant > 0 {
            report.recommendations.push(
                format!("Address {} non-compliant requirements immediately", non_compliant)
            );
        }
        
        if !report.upcoming_deadlines.is_empty() {
            report.recommendations.push(
                format!("Prepare for {} upcoming deadlines", report.upcoming_deadlines.len())
            );
        }
        
        report
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn test_regulation_status() {
        let reg = Regulation::new(
            "Test Regulation".to_string(),
            RegulationType::DataPrivacy,
            CountryCode::new("US"),
            "Test Agency".to_string(),
        );

        assert!(!reg.is_active());
    }

    #[test]
    fn test_compliance_requirement() {
        let mut req = ComplianceRequirement::new(
            "Test Requirement".to_string(),
            RequirementType::Reporting,
        );

        req.update_completion(49.0);
        assert_eq!(req.status, ComplianceStatus::InProgress);

        req.update_completion(100.0);
        assert_eq!(req.status, ComplianceStatus::Compliant);
    }

    #[test]
    fn test_tax_policy_calculation() {
        let policy = TaxPolicy {
            id: Uuid::new_v4(),
            country: CountryCode::new("US"),
            tax_type: TaxType::CorporateIncome,
            title: "Test Tax".to_string(),
            description: "Test".to_string(),
            rate: Some(21.0),
            rate_type: RateType::Flat,
            effective_date: Utc::now(),
            affected_sectors: vec!["All".to_string()],
            affected_activities: vec![],
            exemptions: vec![],
            deductions: vec![],
            credits: vec![],
            source: IntelligenceSource::Government,
            last_updated: Utc::now(),
        };

        let liability = policy.calculate_liability(1000000.0);
        assert!((liability - 210000.0).abs() < 0.01);
    }

    #[test]
    fn test_labor_law_change() {
        let change = LaborLawChange {
            id: Uuid::new_v4(),
            country: CountryCode::new("EU"),
            change_type: LaborLawType::MinimumWage,
            title: "Minimum Wage Increase".to_string(),
            description: "Test".to_string(),
            previous_requirement: Some("10 EUR/hour".to_string()),
            new_requirement: "12 EUR/hour".to_string(),
            effective_date: Utc::now() + chrono::Duration::days(30),
            transition_period_end: None,
            affected_employers: vec!["All".to_string()],
            affected_workers: 1000000,
            employer_costs: Some(1000000.0),
            impact_assessment: LaborImpactAssessment::default(),
            source: IntelligenceSource::Government,
            last_updated: Utc::now(),
        };

        assert!(change.is_imminent(60));
        assert!(!change.is_imminent(10));
    }

    #[test]
    fn test_regulation_types() {
        assert_eq!(
            RegulationType::CarbonEmissions.category(),
            "Environmental"
        );
        assert_eq!(
            RegulationType::Labor.category(),
            "Labor"
        );
        assert_eq!(
            RegulationType::DataPrivacy.category(),
            "Digital"
        );
    }

    #[test]
    fn test_compliance_status_severity() {
        assert_eq!(ComplianceStatus::Compliant.severity(), Severity::Low);
        assert_eq!(ComplianceStatus::NonCompliant.severity(), Severity::Critical);
        assert_eq!(ComplianceStatus::InProgress.severity(), Severity::Medium);
    }

    #[test]
    fn test_search_regulations() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let regulatory = RegulatoryClient::new(client, config);

        let results = regulatory.search_regulations(
            "data",
            None,
            Some(RegulationType::DataPrivacy),
        );

        assert!(!results.is_empty());
    }

    #[test]
    fn test_upcoming_deadlines() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let regulatory = RegulatoryClient::new(client, config);

        let _deadlines = regulatory.get_upcoming_deadlines(365);
        // May have some mock deadlines
    }

    #[test]
    fn test_compliance_report() {
        let config = GeopoliticalConfig::default();
        let client = Client::new();
        let regulatory = RegulatoryClient::new(client, config);

        let report = regulatory.generate_compliance_report(
            "Test Corp",
            &CountryCode::new("US"),
        );

        assert_eq!(report.organization, "Test Corp");
    }

    #[test]
    fn test_penalty_types() {
        assert_eq!(PenaltyType::Fine.description(), "Monetary Fine");
        assert_eq!(PenaltyType::Debarment.description(), "Debarment from Government Contracts");
    }

    #[test]
    fn test_tax_types() {
        assert_eq!(TaxType::CorporateIncome.description(), "Corporate Income Tax");
        assert_eq!(TaxType::CarbonTax.description(), "Carbon Tax");
    }

    #[test]
    fn test_reporting_frequency() {
        assert_eq!(ReportingFrequency::Quarterly.description(), "Quarterly");
        assert_eq!(ReportingFrequency::Annually.description(), "Annually");
    }
}
