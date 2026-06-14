//! # Attack Surface Analysis Module
//!
//! 3.2.2: Comprehensive attack surface analysis including external exposure detection,
//! vulnerability correlation, misconfiguration detection, and shadow IT identification.
//!
//! ## Features
//!
//! - External exposure detection and monitoring
//! - Vulnerability correlation and prioritization
//! - Security misconfiguration detection
//! - Shadow IT identification
//! - Attack surface scoring

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{Result, ThreatIntelError};
use crate::models::SeverityLevel;

/// External exposure point detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalExposure {
    pub id: Uuid,
    pub asset_id: Uuid,
    pub asset_identifier: String,
    pub exposure_type: ExposureType,
    pub severity: SeverityLevel,
    pub service: Option<String>,
    pub port: Option<u16>,
    pub protocol: Option<String>,
    pub certificate_info: Option<CertificateInfo>,
    pub dns_records: Vec<DnsRecord>,
    pub http_headers: HashMap<String, String>,
    pub risk_score: f64,
    pub description: String,
    pub recommendations: Vec<String>,
    pub discovered_at: DateTime<Utc>,
    pub last_verified: DateTime<Utc>,
}

impl ExternalExposure {
    pub fn new(asset_id: Uuid, asset_identifier: impl Into<String>, exposure_type: ExposureType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            asset_id,
            asset_identifier: asset_identifier.into(),
            exposure_type,
            severity: SeverityLevel::Medium,
            service: None,
            port: None,
            protocol: None,
            certificate_info: None,
            dns_records: Vec::new(),
            http_headers: HashMap::new(),
            risk_score: 0.5,
            description: String::new(),
            recommendations: Vec::new(),
            discovered_at: now,
            last_verified: now,
        }
    }

    /// Calculate risk score based on exposure characteristics.
    pub fn calculate_risk_score(&self) -> f64 {
        let mut score: f64 = 0.0;

        // Base severity
        score += match self.severity {
            SeverityLevel::Critical => 0.4,
            SeverityLevel::High => 0.3,
            SeverityLevel::Medium => 0.15,
            SeverityLevel::Low => 0.05,
            SeverityLevel::Info => 0.0,
        };

        // Exposure type factor
        score += match self.exposure_type {
            ExposureType::PublicApi => 0.25,
            ExposureType::RemoteAccess => 0.3,
            ExposureType::Database => 0.35,
            ExposureType::ManagementInterface => 0.3,
            ExposureType::ExposedService => 0.15,
            ExposureType::LegacySystem => 0.2,
            ExposureType::DevelopmentEnvironment => 0.1,
            ExposureType::CloudMisconfiguration => 0.25,
            ExposureType::ThirdPartyIntegration => 0.2,
        };

        // Certificate issues
        if let Some(cert) = &self.certificate_info {
            if cert.is_expired {
                score += 0.15;
            }
            if cert.is_self_signed {
                score += 0.1;
            }
            if cert.verifies_chain.is_none() {
                score += 0.1;
            }
        }

        score.min(1.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExposureType {
    PublicApi,
    RemoteAccess,
    Database,
    ManagementInterface,
    ExposedService,
    LegacySystem,
    DevelopmentEnvironment,
    CloudMisconfiguration,
    ThirdPartyIntegration,
}

impl ExposureType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::PublicApi => "public_api",
            Self::RemoteAccess => "remote_access",
            Self::Database => "database",
            Self::ManagementInterface => "management_interface",
            Self::ExposedService => "exposed_service",
            Self::LegacySystem => "legacy_system",
            Self::DevelopmentEnvironment => "development_environment",
            Self::CloudMisconfiguration => "cloud_misconfiguration",
            Self::ThirdPartyIntegration => "third_party_integration",
        }
    }
}

/// Certificate information for an exposed endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateInfo {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub valid_from: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub is_expired: bool,
    pub is_self_signed: bool,
    pub verifies_chain: Option<bool>,
    pub signature_algorithm: String,
    pub key_algorithm: String,
    pub key_length: u32,
    pub san_domains: Vec<String>,
}

/// DNS record information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub record_type: String,
    pub name: String,
    pub value: String,
    pub ttl: u32,
}

/// Vulnerability record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: Uuid,
    pub cve_id: Option<String>,
    pub title: String,
    pub description: String,
    pub severity: SeverityLevel,
    pub cvss_score: Option<f64>,
    pub cvss_vector: Option<String>,
    pub affected_products: Vec<AffectedProduct>,
    pub related_cwes: Vec<String>,
    pub published_date: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub exploitation_level: ExploitationLevel,
    pub patch_available: bool,
    pub patch_url: Option<String>,
    pub remediation_steps: Vec<String>,
    pub references: Vec<String>,
    pub metadata: serde_json::Value,
}

impl Vulnerability {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            cve_id: None,
            title: title.into(),
            description: String::new(),
            severity: SeverityLevel::Medium,
            cvss_score: None,
            cvss_vector: None,
            affected_products: Vec::new(),
            related_cwes: Vec::new(),
            published_date: Utc::now(),
            last_modified: Utc::now(),
            exploitation_level: ExploitationLevel::Unknown,
            patch_available: false,
            patch_url: None,
            remediation_steps: Vec::new(),
            references: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExploitationLevel {
    /// Public exploit code available
    Active,
    /// Exploit proof-of-concept available
    PoC,
    /// No public exploit, but techniques known
    Known,
    /// No known exploitation
    Unknown,
}

impl ExploitationLevel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::PoC => "poc",
            Self::Known => "known",
            Self::Unknown => "unknown",
        }
    }
}

/// Affected product information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedProduct {
    pub vendor: String,
    pub product: String,
    pub version_start: Option<String>,
    pub version_end: Option<String>,
    pub version_type: VersionType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionType {
    Including,
    Excluding,
}

/// Security misconfiguration finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Misconfiguration {
    pub id: Uuid,
    pub misconfiguration_type: MisconfigurationType,
    pub severity: SeverityLevel,
    pub title: String,
    pub description: String,
    pub affected_component: String,
    pub current_value: Option<String>,
    pub expected_value: Option<String>,
    pub remediation_steps: Vec<String>,
    pub references: Vec<String>,
    pub discovered_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl Misconfiguration {
    pub fn new(misconfiguration_type: MisconfigurationType, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            misconfiguration_type,
            severity: misconfiguration_type.severity_default(),
            title: title.into(),
            description: String::new(),
            affected_component: String::new(),
            current_value: None,
            expected_value: None,
            remediation_steps: Vec::new(),
            references: Vec::new(),
            discovered_at: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MisconfigurationType {
    /// Insecure protocol usage (HTTP instead of HTTPS)
    InsecureProtocol,
    /// Weak or missing authentication
    WeakAuthentication,
    /// Missing or misconfigured encryption
    MissingEncryption,
    /// Excessive permissions or privileges
    ExcessivePermissions,
    /// Insecure default configuration
    InsecureDefaults,
    /// Missing security headers
    MissingSecurityHeaders,
    /// Insecure cookie settings
    InsecureCookies,
    /// Open directory or file listing
    InformationDisclosure,
    /// Missing access controls
    MissingAccessControl,
    /// Insecure cloud configuration
    CloudMisconfiguration,
    /// Insecure API configuration
    ApiMisconfiguration,
    /// Legacy or deprecated protocols
    DeprecatedProtocol,
}

impl MisconfigurationType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InsecureProtocol => "insecure_protocol",
            Self::WeakAuthentication => "weak_authentication",
            Self::MissingEncryption => "missing_encryption",
            Self::ExcessivePermissions => "excessive_permissions",
            Self::InsecureDefaults => "insecure_defaults",
            Self::MissingSecurityHeaders => "missing_security_headers",
            Self::InsecureCookies => "insecure_cookies",
            Self::InformationDisclosure => "information_disclosure",
            Self::MissingAccessControl => "missing_access_control",
            Self::CloudMisconfiguration => "cloud_misconfiguration",
            Self::ApiMisconfiguration => "api_misconfiguration",
            Self::DeprecatedProtocol => "deprecated_protocol",
        }
    }

    pub fn severity_default(&self) -> SeverityLevel {
        match self {
            Self::InsecureProtocol => SeverityLevel::High,
            Self::WeakAuthentication => SeverityLevel::Critical,
            Self::MissingEncryption => SeverityLevel::Critical,
            Self::ExcessivePermissions => SeverityLevel::High,
            Self::InsecureDefaults => SeverityLevel::Medium,
            Self::MissingSecurityHeaders => SeverityLevel::Low,
            Self::InsecureCookies => SeverityLevel::Medium,
            Self::InformationDisclosure => SeverityLevel::Medium,
            Self::MissingAccessControl => SeverityLevel::High,
            Self::CloudMisconfiguration => SeverityLevel::High,
            Self::ApiMisconfiguration => SeverityLevel::High,
            Self::DeprecatedProtocol => SeverityLevel::Medium,
        }
    }
}

/// Shadow IT asset detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowITAsset {
    pub id: Uuid,
    pub asset_type: ShadowITType,
    pub identifier: String,
    pub domain: Option<String>,
    pub ip_addresses: Vec<String>,
    pub service_provider: Option<String>,
    pub business_function: Option<String>,
    pub department: Option<String>,
    pub owner: Option<String>,
    pub data_sensitivity: DataSensitivity,
    pub risk_score: f64,
    pub compliance_impact: Vec<ComplianceFramework>,
    pub approval_status: ApprovalStatus,
    pub discovered_at: DateTime<Utc>,
    pub last_verified: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl ShadowITAsset {
    pub fn new(asset_type: ShadowITType, identifier: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            asset_type,
            identifier: identifier.into(),
            domain: None,
            ip_addresses: Vec::new(),
            service_provider: None,
            business_function: None,
            department: None,
            owner: None,
            data_sensitivity: DataSensitivity::Internal,
            risk_score: 0.5,
            compliance_impact: Vec::new(),
            approval_status: ApprovalStatus::Unknown,
            discovered_at: now,
            last_verified: now,
            metadata: serde_json::json!({}),
        }
    }

    pub fn calculate_risk_score(&self) -> f64 {
        let mut score = 0.0;

        // Data sensitivity
        score += match self.data_sensitivity {
            DataSensitivity::Public => 0.0,
            DataSensitivity::Internal => 0.2,
            DataSensitivity::Confidential => 0.4,
            DataSensitivity::Restricted => 0.6,
        };

        // Compliance impact
        score += (self.compliance_impact.len() as f64 * 0.1).min(0.3);

        // Approval status
        score += match self.approval_status {
            ApprovalStatus::Approved => 0.0,
            ApprovalStatus::PendingReview => 0.2,
            ApprovalStatus::Unauthorized => 0.3,
            ApprovalStatus::Unknown => 0.15,
        };

        // Asset type risk
        score += match self.asset_type {
            ShadowITType::CloudStorage => 0.15,
            ShadowITType::SaaSApplication => 0.15,
            ShadowITType::CommunicationTool => 0.1,
            ShadowITType::DevelopmentTool => 0.2,
            ShadowITType::PersonalDevice => 0.25,
            ShadowITType::UnmonitoredEndpoint => 0.2,
            ShadowITType::UnmanagedServer => 0.25,
            ShadowITType::BYODNetwork => 0.2,
        };

        score.min(1.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShadowITType {
    CloudStorage,
    SaaSApplication,
    CommunicationTool,
    DevelopmentTool,
    PersonalDevice,
    UnmonitoredEndpoint,
    UnmanagedServer,
    BYODNetwork,
}

impl ShadowITType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::CloudStorage => "cloud_storage",
            Self::SaaSApplication => "saas_application",
            Self::CommunicationTool => "communication_tool",
            Self::DevelopmentTool => "development_tool",
            Self::PersonalDevice => "personal_device",
            Self::UnmonitoredEndpoint => "unmonitored_endpoint",
            Self::UnmanagedServer => "unmanaged_server",
            Self::BYODNetwork => "byod_network",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DataSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

impl DataSensitivity {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Confidential => "confidential",
            Self::Restricted => "restricted",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalStatus {
    Approved,
    PendingReview,
    Unauthorized,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ComplianceFramework {
    SOC2,
    ISO27001,
    HIPAA,
    #[allow(non_camel_case_types)]
    PCI_DSS,
    GDPR,
    FedRAMP,
    NIST,
    CIS,
}

impl ComplianceFramework {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SOC2 => "SOC2",
            Self::ISO27001 => "ISO27001",
            Self::HIPAA => "HIPAA",
            Self::PCI_DSS => "PCI_DSS",
            Self::GDPR => "GDPR",
            Self::FedRAMP => "FedRAMP",
            Self::NIST => "NIST",
            Self::CIS => "CIS",
        }
    }
}

/// Attack surface assessment result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackSurfaceAssessment {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub assessment_date: DateTime<Utc>,
    pub exposures: Vec<ExternalExposure>,
    pub vulnerabilities: Vec<Vulnerability>,
    pub misconfigurations: Vec<Misconfiguration>,
    pub shadow_it: Vec<ShadowITAsset>,
    pub overall_score: f64,
    pub risk_distribution: RiskDistribution,
    pub recommendations: Vec<SecurityRecommendation>,
    pub trends: Option<AttackSurfaceTrend>,
}

impl AttackSurfaceAssessment {
    pub fn new(organization_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            organization_id,
            assessment_date: Utc::now(),
            exposures: Vec::new(),
            vulnerabilities: Vec::new(),
            misconfigurations: Vec::new(),
            shadow_it: Vec::new(),
            overall_score: 0.5,
            risk_distribution: RiskDistribution::default(),
            recommendations: Vec::new(),
            trends: None,
        }
    }

    /// Calculate overall attack surface score using a weighted compositing model.
    ///
    /// ## Weight Rationale
    ///
    /// | Component          | Weight | Rationale                                                                    |
    /// |--------------------|--------|------------------------------------------------------------------------------|
    /// | Exposure (0.30)   | 0.30   | External exposure is the primary attack vector; it determines the breadth of |
    /// |                    |        | the attack surface visible to adversaries. High weight reflects that exposed |
    /// |                    |        | services are directly exploitable without needing prior access.              |
    /// | Vulnerability     | 0.35   | Vulnerabilities carry the highest weight because known CVEs with active      |
    /// | (0.35)            |        | exploitation constitute the most urgent and measurable risk. Severity is     |
    /// |                    |        | further modulated by exploitation level (active, PoC, known, unknown).       |
    /// | Misconfiguration  | 0.20   | Misconfigurations are prevalent but typically lower-severity than unpatched  |
    /// | (0.20)            |        | vulnerabilities. They are important hygiene indicators but less directly      |
    /// |                    |        | exploitable without additional chaining.                                     |
    /// | Shadow IT (0.15)  | 0.15   | Shadow IT represents unknown assets with unknown security postures. While    |
    /// |                    |        | potentially high-impact, detection coverage is often incomplete, so a lower  |
    /// |                    |        | weight prevents detection gaps from dominating the overall score.            |
    pub fn calculate_overall_score(&mut self) {
        let exposure_score = self.calculate_exposure_score();
        let vuln_score = self.calculate_vulnerability_score();
        let config_score = self.calculate_misconfiguration_score();
        let shadow_it_score = self.calculate_shadow_it_score();

        // Weighted compositing: 0.30 exposure, 0.35 vulnerability, 0.20 misconfiguration, 0.15 shadow IT
        self.overall_score = (exposure_score * 0.3
            + vuln_score * 0.35
            + config_score * 0.2
            + shadow_it_score * 0.15).min(1.0);

        self.update_risk_distribution();
    }

    fn calculate_exposure_score(&self) -> f64 {
        if self.exposures.is_empty() {
            return 0.0;
        }

        let total: f64 = self.exposures.iter()
            .map(|e| {
                match e.severity {
                    SeverityLevel::Critical => 1.0,
                    SeverityLevel::High => 0.75,
                    SeverityLevel::Medium => 0.5,
                    SeverityLevel::Low => 0.25,
                    SeverityLevel::Info => 0.1,
                }
            })
            .sum();

        (total / self.exposures.len() as f64).min(1.0)
    }

    fn calculate_vulnerability_score(&self) -> f64 {
        if self.vulnerabilities.is_empty() {
            return 0.0;
        }

        let total: f64 = self.vulnerabilities.iter()
            .map(|v| {
                let sev_score: f64 = match v.severity {
                    SeverityLevel::Critical => 1.0,
                    SeverityLevel::High => 0.75,
                    SeverityLevel::Medium => 0.5,
                    SeverityLevel::Low => 0.25,
                    SeverityLevel::Info => 0.1,
                };

                let exploit_multiplier = match v.exploitation_level {
                    ExploitationLevel::Active => 1.5,
                    ExploitationLevel::PoC => 1.25,
                    ExploitationLevel::Known => 1.0,
                    ExploitationLevel::Unknown => 0.75,
                };

                (sev_score * exploit_multiplier).min(1.0)
            })
            .sum();

        (total / self.vulnerabilities.len() as f64).min(1.0)
    }

    fn calculate_misconfiguration_score(&self) -> f64 {
        if self.misconfigurations.is_empty() {
            return 0.0;
        }

        let total: f64 = self.misconfigurations.iter()
            .map(|m| {
                match m.severity {
                    SeverityLevel::Critical => 1.0,
                    SeverityLevel::High => 0.75,
                    SeverityLevel::Medium => 0.5,
                    SeverityLevel::Low => 0.25,
                    SeverityLevel::Info => 0.1,
                }
            })
            .sum();

        (total / self.misconfigurations.len() as f64).min(1.0)
    }

    fn calculate_shadow_it_score(&self) -> f64 {
        if self.shadow_it.is_empty() {
            return 0.0;
        }

        let total: f64 = self.shadow_it.iter()
            .map(|s| s.risk_score)
            .sum();

        (total / self.shadow_it.len() as f64).min(1.0)
    }

    fn update_risk_distribution(&mut self) {
        let mut critical = 0;
        let mut high = 0;
        let mut medium = 0;
        let mut low = 0;

        for e in &self.exposures {
            match e.severity {
                SeverityLevel::Critical => critical += 1,
                SeverityLevel::High => high += 1,
                SeverityLevel::Medium => medium += 1,
                SeverityLevel::Low => low += 1,
                SeverityLevel::Info => {}
            }
        }

        for v in &self.vulnerabilities {
            match v.severity {
                SeverityLevel::Critical => critical += 1,
                SeverityLevel::High => high += 1,
                SeverityLevel::Medium => medium += 1,
                SeverityLevel::Low => low += 1,
                SeverityLevel::Info => {}
            }
        }

        for m in &self.misconfigurations {
            match m.severity {
                SeverityLevel::Critical => critical += 1,
                SeverityLevel::High => high += 1,
                SeverityLevel::Medium => medium += 1,
                SeverityLevel::Low => low += 1,
                SeverityLevel::Info => {}
            }
        }

        self.risk_distribution = RiskDistribution { critical, high, medium, low };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskDistribution {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRecommendation {
    pub priority: Priority,
    pub category: RecommendationCategory,
    pub title: String,
    pub description: String,
    pub estimated_effort: String,
    pub business_impact: String,
    pub related_findings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecommendationCategory {
    ExposureReduction,
    VulnerabilityManagement,
    ConfigurationHardening,
    ShadowITGovernance,
    Monitoring,
    AccessControl,
    Encryption,
    NetworkSegmentation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackSurfaceTrend {
    pub metric: String,
    pub current_value: f64,
    pub previous_value: f64,
    pub change_percentage: f64,
    pub trend_direction: TrendDirection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrendDirection {
    Improving,
    Stable,
    Worsening,
}

/// Attack Surface Analyzer.
#[derive(Debug, Clone, Default)]
pub struct AttackSurfaceAnalyzer {
    assessments: HashMap<Uuid, AttackSurfaceAssessment>,
}

impl AttackSurfaceAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new assessment.
    pub fn create_assessment(&mut self, organization_id: Uuid) -> Uuid {
        let assessment = AttackSurfaceAssessment::new(organization_id);
        let id = assessment.id;
        self.assessments.insert(id, assessment);
        id
    }

    /// Get an assessment by ID.
    pub fn get_assessment(&self, id: Uuid) -> Option<&AttackSurfaceAssessment> {
        self.assessments.get(&id)
    }

    /// Add an exposure to an assessment.
    pub fn add_exposure(&mut self, assessment_id: Uuid, exposure: ExternalExposure) -> Result<()> {
        let assessment = self.assessments
            .get_mut(&assessment_id)
            .ok_or_else(|| ThreatIntelError::attack_surface("Assessment not found"))?;
        
        assessment.exposures.push(exposure);
        assessment.calculate_overall_score();
        Ok(())
    }

    /// Add a vulnerability to an assessment.
    pub fn add_vulnerability(&mut self, assessment_id: Uuid, vulnerability: Vulnerability) -> Result<()> {
        let assessment = self.assessments
            .get_mut(&assessment_id)
            .ok_or_else(|| ThreatIntelError::attack_surface("Assessment not found"))?;
        
        assessment.vulnerabilities.push(vulnerability);
        assessment.calculate_overall_score();
        Ok(())
    }

    /// Add a misconfiguration to an assessment.
    pub fn add_misconfiguration(&mut self, assessment_id: Uuid, misconfiguration: Misconfiguration) -> Result<()> {
        let assessment = self.assessments
            .get_mut(&assessment_id)
            .ok_or_else(|| ThreatIntelError::attack_surface("Assessment not found"))?;
        
        assessment.misconfigurations.push(misconfiguration);
        assessment.calculate_overall_score();
        Ok(())
    }

    /// Add a shadow IT asset to an assessment.
    pub fn add_shadow_it(&mut self, assessment_id: Uuid, asset: ShadowITAsset) -> Result<()> {
        let assessment = self.assessments
            .get_mut(&assessment_id)
            .ok_or_else(|| ThreatIntelError::attack_surface("Assessment not found"))?;
        
        assessment.shadow_it.push(asset);
        assessment.calculate_overall_score();
        Ok(())
    }

    /// Get vulnerabilities by severity.
    pub fn get_vulnerabilities_by_severity(&self, assessment_id: Uuid, severity: SeverityLevel) -> Vec<&Vulnerability> {
        self.assessments
            .get(&assessment_id)
            .map(|a| a.vulnerabilities.iter().filter(|v| v.severity == severity).collect())
            .unwrap_or_default()
    }

    /// Get exposures by type.
    pub fn get_exposures_by_type(&self, assessment_id: Uuid, exposure_type: ExposureType) -> Vec<&ExternalExposure> {
        self.assessments
            .get(&assessment_id)
            .map(|a| a.exposures.iter().filter(|e| e.exposure_type == exposure_type).collect())
            .unwrap_or_default()
    }

    /// Get critical findings across all assessments.
    pub fn get_all_critical_findings(&self) -> Vec<CriticalFinding> {
        let mut findings = Vec::new();

        for assessment in self.assessments.values() {
            for vuln in &assessment.vulnerabilities {
                if vuln.severity == SeverityLevel::Critical {
                    findings.push(CriticalFinding {
                        finding_type: FindingType::Vulnerability(vuln.clone()),
                        severity: SeverityLevel::Critical,
                        assessment_id: assessment.id,
                    });
                }
            }

            for exposure in &assessment.exposures {
                if exposure.severity == SeverityLevel::Critical {
                    findings.push(CriticalFinding {
                        finding_type: FindingType::Exposure(exposure.clone()),
                        severity: SeverityLevel::Critical,
                        assessment_id: assessment.id,
                    });
                }
            }
        }

        findings
    }

    /// Calculate remediation priority score.
    pub fn calculate_remediation_priority(&self, assessment_id: Uuid) -> Vec<RemediationItem> {
        let assessment = match self.assessments.get(&assessment_id) {
            Some(a) => a,
            None => return Vec::new(),
        };

        let mut items = Vec::new();

        // Vulnerabilities
        for vuln in &assessment.vulnerabilities {
            let priority_score = self.vulnerability_priority_score(vuln);
            items.push(RemediationItem {
                item_id: vuln.id.to_string(),
                item_type: "vulnerability".to_string(),
                title: vuln.title.clone(),
                severity: vuln.severity,
                priority_score,
                effort_estimate: self.estimate_remediation_effort(vuln),
                risk_reduction: self.estimate_risk_reduction(vuln),
            });
        }

        // Exposures
        for exposure in &assessment.exposures {
            let priority_score = exposure.risk_score;
            items.push(RemediationItem {
                item_id: exposure.id.to_string(),
                item_type: "exposure".to_string(),
                title: exposure.description.clone(),
                severity: exposure.severity,
                priority_score,
                effort_estimate: "Low".to_string(),
                risk_reduction: exposure.risk_score,
            });
        }

        // Sort by priority score descending
        items.sort_by(|a, b| b.priority_score.partial_cmp(&a.priority_score).unwrap_or(std::cmp::Ordering::Equal));
        items
    }

    fn vulnerability_priority_score(&self, vuln: &Vulnerability) -> f64 {
        let mut score = vuln.cvss_score.unwrap_or(5.0) / 10.0;

        score *= match vuln.exploitation_level {
            ExploitationLevel::Active => 1.5,
            ExploitationLevel::PoC => 1.25,
            ExploitationLevel::Known => 1.0,
            ExploitationLevel::Unknown => 0.75,
        };

        if vuln.patch_available {
            score *= 0.8; // Lower priority if patch is available
        }

        score.min(1.0)
    }

    fn estimate_remediation_effort(&self, vuln: &Vulnerability) -> String {
        if vuln.patch_available {
            "Low".to_string()
        } else {
            match vuln.severity {
                SeverityLevel::Critical | SeverityLevel::High => "High".to_string(),
                SeverityLevel::Medium => "Medium".to_string(),
                _ => "Low".to_string(),
            }
        }
    }

    fn estimate_risk_reduction(&self, vuln: &Vulnerability) -> f64 {
        match vuln.severity {
            SeverityLevel::Critical => 0.4,
            SeverityLevel::High => 0.25,
            SeverityLevel::Medium => 0.15,
            SeverityLevel::Low => 0.05,
            SeverityLevel::Info => 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FindingType {
    Vulnerability(Vulnerability),
    Exposure(ExternalExposure),
    Misconfiguration(Misconfiguration),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalFinding {
    pub finding_type: FindingType,
    pub severity: SeverityLevel,
    pub assessment_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationItem {
    pub item_id: String,
    pub item_type: String,
    pub title: String,
    pub severity: SeverityLevel,
    pub priority_score: f64,
    pub effort_estimate: String,
    pub risk_reduction: f64,
}

// Builder extensions
impl ExternalExposure {
    pub fn with_severity(mut self, severity: SeverityLevel) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

impl Vulnerability {
    pub fn with_cve(mut self, cve_id: impl Into<String>) -> Self {
        self.cve_id = Some(cve_id.into());
        self
    }

    pub fn with_cvss(mut self, score: f64) -> Self {
        self.cvss_score = Some(score);
        self.severity = SeverityLevel::from_cvss(score);
        self
    }

    pub fn with_exploitation_level(mut self, level: ExploitationLevel) -> Self {
        self.exploitation_level = level;
        self
    }
}

impl Misconfiguration {
    pub fn with_severity(mut self, severity: SeverityLevel) -> Self {
        self.severity = severity;
        self
    }
}

impl ShadowITAsset {
    pub fn with_sensitivity(mut self, sensitivity: DataSensitivity) -> Self {
        self.data_sensitivity = sensitivity;
        self
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_exposure_creation() {
        let exposure = ExternalExposure::new(
            Uuid::new_v4(),
            "api.example.com",
            ExposureType::PublicApi,
        )
        .with_severity(SeverityLevel::High)
        .with_port(443);

        assert_eq!(exposure.asset_identifier, "api.example.com");
        assert_eq!(exposure.exposure_type, ExposureType::PublicApi);
        assert_eq!(exposure.severity, SeverityLevel::High);
        assert_eq!(exposure.port, Some(443));
    }

    #[test]
    fn test_exposure_risk_score() {
        let exposure = ExternalExposure::new(
            Uuid::new_v4(),
            "vulnerable.example.com",
            ExposureType::Database,
        )
        .with_severity(SeverityLevel::Critical);

        let score = exposure.calculate_risk_score();
        assert!(score > 0.5); // High risk due to database + critical severity
    }

    #[test]
    fn test_vulnerability_cvss_severity() {
        let vuln = Vulnerability::new("Test Vulnerability")
            .with_cvss(9.5);

        assert_eq!(vuln.severity, SeverityLevel::Critical);
        assert!(vuln.cvss_score.is_some());
    }

    #[test]
    fn test_misconfiguration_types() {
        let misconfig = Misconfiguration::new(
            MisconfigurationType::WeakAuthentication,
            "Missing MFA on admin portal",
        );

        assert_eq!(misconfig.misconfiguration_type, MisconfigurationType::WeakAuthentication);
        assert_eq!(misconfig.severity, MisconfigurationType::WeakAuthentication.severity_default());
    }

    #[test]
    fn test_shadow_it_risk_score() {
        let asset = ShadowITAsset::new(ShadowITType::CloudStorage, "Dropbox")
            .with_sensitivity(DataSensitivity::Confidential);

        let score = asset.calculate_risk_score();
        assert!(score > 0.4); // High risk due to confidential data
    }

    #[test]
    fn test_attack_surface_assessment() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        // Add a critical vulnerability with active exploitation
        let vuln = Vulnerability::new("Critical CVE")
            .with_cvss(9.8)
            .with_exploitation_level(ExploitationLevel::Active);
        analyzer.add_vulnerability(assessment_id, vuln).unwrap();

        // Add a high exposure
        let exposure = ExternalExposure::new(
            Uuid::new_v4(),
            "admin.example.com",
            ExposureType::ManagementInterface,
        )
        .with_severity(SeverityLevel::High);
        analyzer.add_exposure(assessment_id, exposure).unwrap();

        let assessment = analyzer.get_assessment(assessment_id).unwrap();
        assert!(!assessment.vulnerabilities.is_empty());
        assert!(!assessment.exposures.is_empty());
        assert!(assessment.overall_score > 0.5);
    }

    #[test]
    fn test_remediation_priority() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        // Add multiple vulnerabilities
        let vuln1 = Vulnerability::new("Critical CVE")
            .with_cvss(9.8);
        let vuln2 = Vulnerability::new("Medium CVE")
            .with_cvss(5.5);

        analyzer.add_vulnerability(assessment_id, vuln1).unwrap();
        analyzer.add_vulnerability(assessment_id, vuln2).unwrap();

        let priorities = analyzer.calculate_remediation_priority(assessment_id);
        assert!(!priorities.is_empty());
        assert!(priorities[0].priority_score >= priorities[1].priority_score);
    }

    #[test]
    fn test_critical_findings() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        let vuln = Vulnerability::new("Critical")
            .with_cvss(9.5);
        analyzer.add_vulnerability(assessment_id, vuln).unwrap();

        let critical = analyzer.get_all_critical_findings();
        assert!(!critical.is_empty());
    }

    #[test]
    fn test_exposure_type_conversion() {
        assert_eq!(ExposureType::PublicApi.as_str(), "public_api");
        assert_eq!(ExposureType::CloudMisconfiguration.as_str(), "cloud_misconfiguration");
    }

    #[test]
    fn test_data_sensitivity_conversion() {
        assert_eq!(DataSensitivity::Confidential.as_str(), "confidential");
        assert_eq!(DataSensitivity::Restricted.as_str(), "restricted");
    }
}
