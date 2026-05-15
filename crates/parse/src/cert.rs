use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::validation::normalize_url;

static CERT_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(ISO\s*\d{4,5}(?::\d{4})?)",
        r"(?i)(IATF\s*16949(?::\d{4})?)",
        r"(?i)(AS\s*9100\w?)",
        r"(?i)(Nadcap(?:\s+Electronics)?)",
        r"(?i)(IEC\s*61340(?:[-\s]\d+[-\s]\d+)?)",
        r"(?i)(IPC[-\s]*A[-\s]*610\w?)",
        r"(?i)(IPC\s*J[-\s]*STD[-\s]*001\w?)",
        r"(?i)\b(UL\s*\d*)\b",
        r"(?i)\b(CE)\b(?:\s*mark)?",
        r"(?i)\b(RoHS)\b",
        r"(?i)\b(REACH)\b",
        r"(?i)\b(ITAR)\b",
    ]
    .iter()
    .map(|p| {
        RegexBuilder::new(p)
            .size_limit(200_000)
            .dfa_size_limit(200_000)
            .build()
            .unwrap_or_else(|error| panic!("invalid certification pattern regex `{p}`: {error}"))
    })
    .collect()
});

static RE_HOLDER: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:holder|certified company|organisation|company name)[:\s]+([^\n.;]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid certification holder regex: {error}"))
});

static RE_CERT_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:certificate|cert\.?\s*(?:no|#|number))[:\s]*([A-Z0-9][\w\-/]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid certification number regex: {error}"))
});

static RE_ISSUER: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:issued by|certifying body|issuer|organisme)[:\s]+([^\n.;]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid certification issuer regex: {error}"))
});

static RE_ISSUE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:issue date|date of issue|valid from)[:\s]+(\d{4}[-/]\d{2}[-/]\d{2})")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid certification issue-date regex: {error}"))
});

static RE_EXPIRY_DATE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r"(?i)(?:expiry|expiration|valid until|valid to)[:\s]+(\d{4}[-/]\d{2}[-/]\d{2})",
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap_or_else(|error| panic!("invalid certification expiry-date regex: {error}"))
});

static RE_SCOPE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"(?i)(?:scope|champ d'application)[:\s]+([^\n]+)")
        .size_limit(200_000)
        .dfa_size_limit(200_000)
        .build()
        .unwrap_or_else(|error| panic!("invalid certification scope regex: {error}"))
});

/// Known certification standards relevant to EMS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertStandard {
    Iso9001,
    Iso14001,
    Iso13485,
    Iso45001,
    Iatf16949,
    As9100,
    Nadcap,
    Iec61340,
    Ul,
    Ce,
    RoHS,
    Reach,
    Itar,
    NadcapElectronics,
    IpcA610,
    #[allow(non_camel_case_types)]
    IpcJ_Std_001,
    Other(String),
}

impl CertStandard {
    pub fn from_text(text: &str) -> Self {
        let t = text.to_uppercase().replace([' ', '-'], "");
        if t.contains("ISO9001") {
            return CertStandard::Iso9001;
        }
        if t.contains("ISO14001") {
            return CertStandard::Iso14001;
        }
        if t.contains("ISO13485") {
            return CertStandard::Iso13485;
        }
        if t.contains("ISO45001") {
            return CertStandard::Iso45001;
        }
        if t.contains("IATF16949") || t.contains("16949") {
            return CertStandard::Iatf16949;
        }
        if t.contains("AS9100") {
            return CertStandard::As9100;
        }
        if t.contains("NADCAPELECTRONICS") {
            return CertStandard::NadcapElectronics;
        }
        if t.contains("NADCAP") {
            return CertStandard::Nadcap;
        }
        if t.contains("61340") {
            return CertStandard::Iec61340;
        }
        if t.contains("IPCA610") || t.contains("A610") {
            return CertStandard::IpcA610;
        }
        if t.contains("JSTD001") {
            return CertStandard::IpcJ_Std_001;
        }
        if t.contains("ITAR") {
            return CertStandard::Itar;
        }
        if t == "UL" || t.contains("UL94") || t.contains("ULCERTIF") {
            return CertStandard::Ul;
        }
        if t == "CE" || t.contains("CEMARK") {
            return CertStandard::Ce;
        }
        if t.contains("ROHS") {
            return CertStandard::RoHS;
        }
        if t.contains("REACH") {
            return CertStandard::Reach;
        }
        CertStandard::Other(text.to_string())
    }

    pub fn display_name(&self) -> &str {
        match self {
            CertStandard::Iso9001 => "ISO 9001",
            CertStandard::Iso14001 => "ISO 14001",
            CertStandard::Iso13485 => "ISO 13485",
            CertStandard::Iso45001 => "ISO 45001",
            CertStandard::Iatf16949 => "IATF 16949",
            CertStandard::As9100 => "AS9100",
            CertStandard::Nadcap => "Nadcap",
            CertStandard::NadcapElectronics => "Nadcap Electronics",
            CertStandard::Iec61340 => "IEC 61340",
            CertStandard::Ul => "UL",
            CertStandard::Ce => "CE",
            CertStandard::RoHS => "RoHS",
            CertStandard::Reach => "REACH",
            CertStandard::Itar => "ITAR",
            CertStandard::IpcA610 => "IPC-A-610",
            CertStandard::IpcJ_Std_001 => "IPC J-STD-001",
            CertStandard::Other(s) => s.as_str(),
        }
    }
}

/// Extracted certification entry from a registry or company page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertExtract {
    pub standard: String,
    pub parsed_standard: CertStandard,
    pub holder: Option<String>,
    pub certificate_number: Option<String>,
    pub issuer: Option<String>,
    pub issue_date: Option<String>,
    pub expiry_date: Option<String>,
    pub scope: Option<String>,
    pub status: CertExtractionStatus,
    pub url: String,
    pub extracted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertExtractionStatus {
    Active,
    Expired,
    Suspended,
    Unknown,
}

/// Extract all certification mentions from page text.
pub fn extract_certifications(body_text: &str, url: &str) -> Vec<CertExtract> {
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
    let holder = extract_holder(body_text);
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for re in CERT_PATTERNS.iter() {
        for caps in re.captures_iter(body_text) {
            let Some(raw_match) = caps.get(1) else {
                continue;
            };
            let raw = raw_match.as_str();
            let standard = CertStandard::from_text(raw);
            let name = standard.display_name().to_string();

            if seen.contains(&name) {
                continue;
            }
            seen.insert(name.clone());

            let cert_number = caps
                .get(0)
                .and_then(|m| extract_cert_number_near(body_text, m.end()));
            let issuer = extract_issuer(body_text);
            let (issue_date, expiry_date) = extract_cert_dates(body_text);
            let scope = extract_scope(body_text);
            let status = detect_cert_status(body_text);

            results.push(CertExtract {
                standard: normalizer::normalize_whitespace(raw),
                parsed_standard: standard,
                holder: holder.clone(),
                certificate_number: cert_number,
                issuer,
                issue_date,
                expiry_date,
                scope,
                status,
                url: normalized_url.clone(),
                extracted_at: Utc::now(),
            });
        }
    }

    results
}

fn extract_holder(text: &str) -> Option<String> {
    RE_HOLDER.captures(text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn extract_cert_number_near(text: &str, offset: usize) -> Option<String> {
    let remaining = &text[offset..];
    RE_CERT_NUMBER
        .captures(remaining)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn extract_issuer(text: &str) -> Option<String> {
    RE_ISSUER.captures(text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn extract_cert_dates(text: &str) -> (Option<String>, Option<String>) {
    let issue = RE_ISSUE_DATE
        .captures(text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .filter(|raw| normalizer::is_valid_date(raw));
    let expiry = RE_EXPIRY_DATE
        .captures(text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .filter(|raw| normalizer::is_valid_date(raw));

    (issue, expiry)
}

fn extract_scope(text: &str) -> Option<String> {
    RE_SCOPE.captures(text).and_then(|c| {
        c.get(1)
            .map(|m| normalizer::normalize_whitespace(m.as_str()))
    })
}

fn detect_cert_status(text: &str) -> CertExtractionStatus {
    let lower = text.to_lowercase();
    if lower.contains("expired") || lower.contains("expiré") {
        CertExtractionStatus::Expired
    } else if lower.contains("suspended") || lower.contains("suspendu") {
        CertExtractionStatus::Suspended
    } else if lower.contains("active") || lower.contains("valid") || lower.contains("actif") {
        CertExtractionStatus::Active
    } else {
        CertExtractionStatus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cert_standard_from_text() {
        assert_eq!(
            CertStandard::from_text("ISO 9001:2015"),
            CertStandard::Iso9001
        );
        assert_eq!(
            CertStandard::from_text("IATF 16949"),
            CertStandard::Iatf16949
        );
        assert_eq!(CertStandard::from_text("AS9100D"), CertStandard::As9100);
        assert_eq!(CertStandard::from_text("IPC-A-610"), CertStandard::IpcA610);
        assert_eq!(CertStandard::from_text("RoHS"), CertStandard::RoHS);
    }

    #[test]
    fn test_cert_standard_display() {
        assert_eq!(CertStandard::Iso9001.display_name(), "ISO 9001");
        assert_eq!(CertStandard::Iatf16949.display_name(), "IATF 16949");
        assert_eq!(CertStandard::IpcA610.display_name(), "IPC-A-610");
    }

    #[test]
    fn test_extract_certifications_multiple() {
        let body = "Our company holds ISO 9001:2015, ISO 14001:2015, and IATF 16949 certifications. \
                    Status: active. Issued by: TUV SUD. Issue date: 2023-01-15. Expiry: 2026-01-15.";
        let certs = extract_certifications(body, "https://example.com/certs");

        assert!(certs.len() >= 3);
        let standards: Vec<String> = certs.iter().map(|c| c.standard.clone()).collect();
        assert!(standards.iter().any(|s| s.contains("9001")));
        assert!(standards.iter().any(|s| s.contains("14001")));
        assert!(standards.iter().any(|s| s.contains("16949")));
    }

    #[test]
    fn test_extract_certifications_with_details() {
        let body = "Certificate holder: Company name: Starz Electronics. \
                    ISO 9001:2015 Certificate no: QMS-2024-001. \
                    Issued by: Bureau Veritas. \
                    Issue date: 2024-01-01. Valid until: 2027-01-01. \
                    Scope: Design and manufacture of electronic assemblies. \
                    Status: active.";
        let certs = extract_certifications(body, "https://example.com");

        assert!(!certs.is_empty());
        let cert = &certs[0];
        assert_eq!(cert.parsed_standard, CertStandard::Iso9001);
        assert_eq!(cert.status, CertExtractionStatus::Active);
        assert!(cert.issuer.is_some());
        assert!(cert.issue_date.is_some());
        assert!(cert.expiry_date.is_some());
        assert!(cert.scope.is_some());
    }

    #[test]
    fn test_detect_cert_status() {
        assert_eq!(
            detect_cert_status("This certificate is active"),
            CertExtractionStatus::Active
        );
        assert_eq!(
            detect_cert_status("Certificate expired on 2023-12-31"),
            CertExtractionStatus::Expired
        );
        assert_eq!(
            detect_cert_status("Certification has been suspended"),
            CertExtractionStatus::Suspended
        );
        assert_eq!(
            detect_cert_status("No status info"),
            CertExtractionStatus::Unknown
        );
    }

    #[test]
    fn test_extract_ems_specific_certs() {
        let body =
            "Certifications: IPC-A-610 Class 3, IPC J-STD-001, Nadcap Electronics. All valid.";
        let certs = extract_certifications(body, "https://example.com");

        let standards: Vec<&CertStandard> = certs.iter().map(|c| &c.parsed_standard).collect();
        assert!(standards.contains(&&CertStandard::IpcA610));
        assert!(standards.contains(&&CertStandard::IpcJ_Std_001));
        assert!(standards.contains(&&CertStandard::NadcapElectronics));
    }

    #[test]
    fn test_extract_holder() {
        let text = "Certified company: Foxconn Technology Group. Certificate details below.";
        let holder = extract_holder(text);
        assert!(holder.is_some());
        assert!(matches!(holder.as_deref(), Some(value) if value.contains("Foxconn")));
    }

    #[test]
    fn test_extract_rohs_reach() {
        let body = "Our products comply with RoHS and REACH regulations.";
        let certs = extract_certifications(body, "https://example.com");
        let standards: Vec<&CertStandard> = certs.iter().map(|c| &c.parsed_standard).collect();
        assert!(standards.contains(&&CertStandard::RoHS));
        assert!(standards.contains(&&CertStandard::Reach));
    }

    #[test]
    fn fuzz_cert_parse_no_panic() {
        let inputs = [
            "",
            "Certificate no: ???",
            "ISO 9001\0\0\0 weird bytes rendered",
            "شهادة ISO 14001 صالحة",
        ];
        for input in inputs {
            let _ = extract_certifications(input, "https://example.com");
        }
    }
}
