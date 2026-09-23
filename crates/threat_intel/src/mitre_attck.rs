//! # MITRE ATT&CK Framework Integration
//!
//! Provides structures and utilities for mapping threat actor TTPs to the
//! MITRE ATT&CK framework.

use serde::{Deserialize, Serialize};

/// MITRE ATT&CK Tactics (the "why" of an attack - the adversary's objective).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AttackTactic {
    Reconnaissance,
    ResourceDevelopment,
    InitialAccess,
    Execution,
    Persistence,
    PrivilegeEscalation,
    DefenseEvasion,
    CredentialAccess,
    Discovery,
    LateralMovement,
    Collection,
    CommandAndControl,
    Exfiltration,
    Impact,
}

impl AttackTactic {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Reconnaissance => "reconnaissance",
            Self::ResourceDevelopment => "resource-development",
            Self::InitialAccess => "initial-access",
            Self::Execution => "execution",
            Self::Persistence => "persistence",
            Self::PrivilegeEscalation => "privilege-escalation",
            Self::DefenseEvasion => "defense-evasion",
            Self::CredentialAccess => "credential-access",
            Self::Discovery => "discovery",
            Self::LateralMovement => "lateral-movement",
            Self::Collection => "collection",
            Self::CommandAndControl => "command-and-control",
            Self::Exfiltration => "exfiltration",
            Self::Impact => "impact",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Reconnaissance => "TA0043",
            Self::ResourceDevelopment => "TA0042",
            Self::InitialAccess => "TA0001",
            Self::Execution => "TA0002",
            Self::Persistence => "TA0003",
            Self::PrivilegeEscalation => "TA0004",
            Self::DefenseEvasion => "TA0005",
            Self::CredentialAccess => "TA0006",
            Self::Discovery => "TA0007",
            Self::LateralMovement => "TA0008",
            Self::Collection => "TA0009",
            Self::CommandAndControl => "TA0011",
            Self::Exfiltration => "TA0010",
            Self::Impact => "TA0040",
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Reconnaissance => "Gather information to plan future operations",
            Self::ResourceDevelopment => "Establish resources to support operations",
            Self::InitialAccess => "Gain initial foothold in the network",
            Self::Execution => "Run malicious code",
            Self::Persistence => "Maintain presence in the network",
            Self::PrivilegeEscalation => "Gain higher-level permissions",
            Self::DefenseEvasion => "Avoid detection and security controls",
            Self::CredentialAccess => "Steal credentials like usernames and passwords",
            Self::Discovery => "Explore the environment to understand it",
            Self::LateralMovement => "Move through the environment",
            Self::Collection => "Gather data of interest",
            Self::CommandAndControl => "Communicate with compromised systems",
            Self::Exfiltration => "Steal data from the network",
            Self::Impact => "Manipulate, interrupt, or destroy systems or data",
        }
    }
}

/// MITRE ATT&CK Technique (the "how" - the method used).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AttackTechnique {
    pub id: String,
    pub name: String,
    pub tactics: Vec<AttackTactic>,
    pub description: String,
    pub detection: Option<String>,
    pub mitigation: Option<String>,
    pub data_sources: Vec<String>,
}

impl AttackTechnique {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            tactics: Vec::new(),
            description: String::new(),
            detection: None,
            mitigation: None,
            data_sources: Vec::new(),
        }
    }

    pub fn with_tactics(mut self, tactics: Vec<AttackTactic>) -> Self {
        self.tactics = tactics;
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_detection(mut self, detection: impl Into<String>) -> Self {
        self.detection = Some(detection.into());
        self
    }

    pub fn with_mitigation(mut self, mitigation: impl Into<String>) -> Self {
        self.mitigation = Some(mitigation.into());
        self
    }

    pub fn with_data_sources(mut self, sources: Vec<String>) -> Self {
        self.data_sources = sources;
        self
    }
}

/// MITRE ATT&CK Sub-Technique (more specific variants).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AttackSubTechnique {
    pub id: String,
    pub parent_id: String,
    pub name: String,
    pub tactics: Vec<AttackTactic>,
    pub description: String,
    pub platform: Option<String>,
}

/// MITRE ATT&CK Matrix containing all known techniques.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttckMatrix {
    pub version: String,
    pub techniques: Vec<AttackTechnique>,
    pub sub_techniques: Vec<AttackSubTechnique>,
}

impl AttckMatrix {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            techniques: Self::default_techniques(),
            sub_techniques: Vec::new(),
        }
    }

    /// Returns common techniques for major attack categories.
    pub fn default_techniques() -> Vec<AttackTechnique> {
        vec![
            // Initial Access
            AttackTechnique::new("T1190", "Exploit Public-Facing Application")
                .with_tactics(vec![AttackTactic::InitialAccess])
                .with_description("Exploiting vulnerabilities in internet-facing applications")
                .with_detection("Web application firewall logs, IDS/IPS alerts")
                .with_mitigation("Patch management, WAF deployment, input validation")
                .with_data_sources(vec![
                    "Web logs".to_string(),
                    "Network traffic".to_string(),
                    "IDS/IPS".to_string(),
                ]),
            AttackTechnique::new("T1133", "External Remote Services")
                .with_tactics(vec![AttackTactic::InitialAccess, AttackTactic::Persistence])
                .with_description("Exploiting external remote services like VPN, RDP")
                .with_detection("Authentication logs, VPN connection attempts")
                .with_mitigation("Multi-factor authentication, network segmentation")
                .with_data_sources(vec![
                    "VPN logs".to_string(),
                    "Authentication logs".to_string(),
                ]),
            AttackTechnique::new("T1566", "Phishing")
                .with_tactics(vec![AttackTactic::InitialAccess])
                .with_description("Social engineering via email or other communication")
                .with_data_sources(vec!["Email logs".to_string(), "Proxy logs".to_string()]),
            // Execution
            AttackTechnique::new("T1059", "Command and Scripting Interpreter")
                .with_tactics(vec![AttackTactic::Execution])
                .with_description("Execution through PowerShell, Python, Bash, etc.")
                .with_data_sources(vec![
                    "Process execution logs".to_string(),
                    "PowerShell logs".to_string(),
                ]),
            // Persistence
            AttackTechnique::new("T1547", "Boot or Logon Autostart Execution")
                .with_tactics(vec![AttackTactic::Persistence])
                .with_description("Registry keys, startup folders, services")
                .with_data_sources(vec![
                    "Registry monitoring".to_string(),
                    "File monitoring".to_string(),
                ]),
            AttackTechnique::new("T1543", "Create/Modify System Process")
                .with_tactics(vec![
                    AttackTactic::Persistence,
                    AttackTactic::PrivilegeEscalation,
                ])
                .with_description("Creating or modifying system services")
                .with_data_sources(vec![
                    "Windows event logs".to_string(),
                    "Process monitoring".to_string(),
                ]),
            // Privilege Escalation
            AttackTechnique::new("T1068", "Exploitation for Privilege Escalation")
                .with_tactics(vec![AttackTactic::PrivilegeEscalation])
                .with_description("Exploiting vulnerabilities to gain higher privileges")
                .with_data_sources(vec![
                    "Vulnerability scanner".to_string(),
                    "Patch management".to_string(),
                ]),
            // Defense Evasion
            AttackTechnique::new("T1027", "Obfuscated Files or Information")
                .with_tactics(vec![AttackTactic::DefenseEvasion])
                .with_description("Encoding, encryption, or compression of payloads")
                .with_data_sources(vec![
                    "File analysis".to_string(),
                    "Network traffic analysis".to_string(),
                ]),
            AttackTechnique::new("T1070", "Indicator Removal on Host")
                .with_tactics(vec![AttackTactic::DefenseEvasion])
                .with_description("Clearing logs, deleting files, modifying artifacts")
                .with_data_sources(vec![
                    "Log analysis".to_string(),
                    "File integrity monitoring".to_string(),
                ]),
            // Credential Access
            AttackTechnique::new("T1110", "Brute Force")
                .with_tactics(vec![AttackTactic::CredentialAccess])
                .with_description("Credential guessing, credential brute force")
                .with_mitigation("Account lockout, MFA, password complexity")
                .with_data_sources(vec![
                    "Authentication logs".to_string(),
                    "Network traffic".to_string(),
                ]),
            AttackTechnique::new("T1003", "OS Credential Dumping")
                .with_tactics(vec![AttackTactic::CredentialAccess])
                .with_description("Extracting credentials from memory or storage")
                .with_data_sources(vec![
                    "LSASS access".to_string(),
                    "Memory analysis".to_string(),
                ]),
            // Discovery
            AttackTechnique::new("T1087", "Account Discovery")
                .with_tactics(vec![AttackTactic::Discovery])
                .with_description("Identifying user accounts and groups")
                .with_data_sources(vec![
                    "Directory queries".to_string(),
                    "Process monitoring".to_string(),
                ]),
            AttackTechnique::new("T1046", "Network Service Discovery")
                .with_tactics(vec![AttackTactic::Discovery])
                .with_description("Scanning for services, ports, and hosts")
                .with_data_sources(vec![
                    "Network traffic".to_string(),
                    "Firewall logs".to_string(),
                ]),
            // Lateral Movement
            AttackTechnique::new("T1021", "Remote Services")
                .with_tactics(vec![AttackTactic::LateralMovement])
                .with_description("Using remote desktop, SSH, VNC, etc.")
                .with_data_sources(vec![
                    "Remote access logs".to_string(),
                    "Authentication logs".to_string(),
                ]),
            // Collection
            AttackTechnique::new("T1005", "Data from Local System")
                .with_tactics(vec![AttackTactic::Collection])
                .with_description("Collecting data from local storage")
                .with_data_sources(vec![
                    "File access logs".to_string(),
                    "DLP alerts".to_string(),
                ]),
            // Command and Control
            AttackTechnique::new("T1071", "Application Layer Protocol")
                .with_tactics(vec![AttackTactic::CommandAndControl])
                .with_description("Using HTTP, HTTPS, DNS for C2 communication")
                .with_data_sources(vec![
                    "Network traffic".to_string(),
                    "Proxy logs".to_string(),
                    "DNS logs".to_string(),
                ]),
            AttackTechnique::new("T1573", "Encrypted Channel")
                .with_tactics(vec![AttackTactic::CommandAndControl])
                .with_description("Using encryption to hide C2 traffic")
                .with_data_sources(vec![
                    "Network traffic analysis".to_string(),
                    "TLS inspection".to_string(),
                ]),
            // Exfiltration
            AttackTechnique::new("T1041", "Exfiltration Over C2 Channel")
                .with_tactics(vec![AttackTactic::Exfiltration])
                .with_description("Data exfiltration via existing C2 channel")
                .with_data_sources(vec![
                    "Network traffic".to_string(),
                    "DLP alerts".to_string(),
                ]),
            AttackTechnique::new("T1567", "Exfiltration Over Web Service")
                .with_tactics(vec![AttackTactic::Exfiltration])
                .with_description("Using cloud storage or web services for exfil")
                .with_data_sources(vec!["Cloud logs".to_string(), "Proxy logs".to_string()]),
            // Impact
            AttackTechnique::new("T1486", "Data Encrypted for Impact")
                .with_tactics(vec![AttackTactic::Impact])
                .with_description("Ransomware encryption of data")
                .with_mitigation("Backup strategy, EDR deployment")
                .with_data_sources(vec![
                    "File encryption events".to_string(),
                    "Ransomware notes".to_string(),
                ]),
            AttackTechnique::new("T1489", "Service Stop")
                .with_tactics(vec![AttackTactic::Impact])
                .with_description("Stopping services to enable impact or evasion")
                .with_data_sources(vec!["Service control manager logs".to_string()]),
            // Supply Chain specific techniques
            AttackTechnique::new("T1195", "Supply Chain Compromise")
                .with_tactics(vec![
                    AttackTactic::InitialAccess,
                    AttackTactic::Execution,
                    AttackTactic::Persistence,
                ])
                .with_description(
                    "Compromising software dependencies, update mechanisms, or hardware",
                )
                .with_detection("Software composition analysis, hash verification")
                .with_mitigation("Code signing, SBOM analysis, vendor assessment")
                .with_data_sources(vec![
                    "Package manager logs".to_string(),
                    "Software inventory".to_string(),
                ]),
            AttackTechnique::new("T1195.001", "Software Development Tools Compromise")
                .with_tactics(vec![AttackTactic::InitialAccess])
                .with_description("Compromising development tools or build pipelines")
                .with_data_sources(vec![
                    "CI/CD logs".to_string(),
                    "Build artifact analysis".to_string(),
                ]),
            AttackTechnique::new("T1195.002", "Software Supply Compromise")
                .with_tactics(vec![AttackTactic::InitialAccess])
                .with_description("Compromising software dependencies or libraries")
                .with_data_sources(vec![
                    "Dependency scanning".to_string(),
                    "SBOM analysis".to_string(),
                ]),
            // Reconnaissance for supply chain
            AttackTechnique::new("T1596", "Search Open Technical Databases")
                .with_tactics(vec![AttackTactic::Reconnaissance])
                .with_description("Using WHOIS, DNS, certificate transparency, Shodan")
                .with_data_sources(vec![
                    "DNS logs".to_string(),
                    "Certificate logs".to_string(),
                    "WHOIS data".to_string(),
                ]),
            AttackTechnique::new("T1591", "Gather Victim Org Information")
                .with_tactics(vec![AttackTactic::Reconnaissance])
                .with_description("Gathering info about target organization structure")
                .with_data_sources(vec![
                    "OSINT".to_string(),
                    "Social media".to_string(),
                    "Public records".to_string(),
                ]),
        ]
    }

    /// Find techniques by tactic.
    pub fn find_by_tactic(&self, tactic: AttackTactic) -> Vec<&AttackTechnique> {
        self.techniques
            .iter()
            .filter(|t| t.tactics.contains(&tactic))
            .collect()
    }

    /// Find techniques by ID pattern (e.g., "T1195" matches all sub-techniques).
    pub fn find_by_id_pattern(&self, pattern: &str) -> Vec<&AttackTechnique> {
        self.techniques
            .iter()
            .filter(|t| t.id.starts_with(pattern))
            .collect()
    }

    /// Find technique by exact ID.
    pub fn find_by_id(&self, id: &str) -> Option<&AttackTechnique> {
        self.techniques.iter().find(|t| t.id == id)
    }
}

/// Software or malware used by threat actors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttckSoftware {
    pub id: String,
    pub name: String,
    pub type_: SoftwareType,
    pub description: String,
    pub techniques: Vec<String>, // Technique IDs
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SoftwareType {
    Malware,
    Tool,
    Ransomware,
    Loader,
    Backdoor,
}

/// Mapping between threat actors and their associated techniques.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorTechniqueMapping {
    pub actor_id: String,
    pub technique_id: String,
    pub confidence: f64,
    pub evidence: String,
    pub last_observed: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_tactic_properties() {
        let tactic = AttackTactic::InitialAccess;
        assert_eq!(tactic.as_str(), "initial-access");
        assert_eq!(tactic.id(), "TA0001");
        assert!(tactic.description().contains("foothold"));
    }

    #[test]
    fn test_attck_matrix() {
        let matrix = AttckMatrix::new("v14.0");
        assert_eq!(matrix.version, "v14.0");
        assert!(!matrix.techniques.is_empty());

        // Find by tactic
        let phishing = matrix.find_by_tactic(AttackTactic::InitialAccess);
        assert!(!phishing.is_empty());

        // Find by ID
        let supply_chain = matrix.find_by_id("T1195");
        assert!(supply_chain.is_some());
        assert_eq!(supply_chain.unwrap().name, "Supply Chain Compromise");
    }

    #[test]
    fn test_attck_matrix_id_pattern() {
        let matrix = AttckMatrix::new("v14.0");
        let supply_chain_variants = matrix.find_by_id_pattern("T1195");
        assert!(!supply_chain_variants.is_empty());
    }
}
