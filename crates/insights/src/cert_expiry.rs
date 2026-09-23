//! Certificate expiry warning integration.
//!
//! If a tracked company's IATF/AS9100/ISO certification is expiring
//! within 90 days, auto-generate a warning (type=cert_expiry_approaching)
//! and flag as competitor opportunity or own-risk.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ─── Configuration ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CertExpiryConfig {
    /// Days before expiry to generate a warning
    pub warning_days: i64,
    /// Days before expiry for critical severity
    pub critical_days: i64,
    /// Days before expiry for high severity
    pub high_days: i64,
    /// Whether to check competitor certs (opportunity signals)
    pub check_competitors: bool,
    /// Whether to check own certs (risk signals)
    pub check_own: bool,
}

impl Default for CertExpiryConfig {
    fn default() -> Self {
        Self {
            warning_days: 90,
            critical_days: 30,
            high_days: 60,
            check_competitors: true,
            check_own: true,
        }
    }
}

// ─── Certification record ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRecord {
    pub company_id: String,
    pub company_name: String,
    pub cert_type: String,
    pub cert_body: String,
    pub issued_date: NaiveDate,
    pub expiry_date: NaiveDate,
    pub scope: String,
    /// Whether this is our own company or a competitor
    pub is_own: bool,
    /// Region/country
    pub region: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CertExpiryWarning {
    pub company_id: String,
    pub company_name: String,
    pub cert_type: String,
    pub expiry_date: NaiveDate,
    pub days_until_expiry: i64,
    pub severity: String,
    pub warning_type: String,
    pub title: String,
    pub description: String,
    pub signal_type: CertSignalType,
    pub region: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum CertSignalType {
    /// Our cert is expiring — action required
    OwnRisk,
    /// Competitor cert expiring — potential opportunity
    CompetitorOpportunity,
}

// ─── Expiry checker ─────────────────────────────────────────────────────

pub struct CertExpiryChecker {
    config: CertExpiryConfig,
}

impl CertExpiryChecker {
    pub fn new(config: CertExpiryConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(CertExpiryConfig::default())
    }

    /// Check a single cert for expiry warnings.
    pub fn check_cert(&self, cert: &CertRecord, today: NaiveDate) -> Option<CertExpiryWarning> {
        let days_until = (cert.expiry_date - today).num_days();

        // Skip already-expired or far-future certs
        if days_until < 0 || days_until > self.config.warning_days {
            return None;
        }

        // Skip based on own/competitor config
        if cert.is_own && !self.config.check_own {
            return None;
        }
        if !cert.is_own && !self.config.check_competitors {
            return None;
        }

        let severity = if days_until <= self.config.critical_days {
            "critical"
        } else if days_until <= self.config.high_days {
            "high"
        } else {
            "medium"
        };

        let signal_type = if cert.is_own {
            CertSignalType::OwnRisk
        } else {
            CertSignalType::CompetitorOpportunity
        };

        let title = if cert.is_own {
            format!(
                "Own {} certification expiring in {} days",
                cert.cert_type, days_until
            )
        } else {
            format!(
                "Competitor {} {} cert expires in {} days — potential opportunity",
                cert.company_name, cert.cert_type, days_until
            )
        };

        let description = if cert.is_own {
            format!(
                "The {} certification (issued by {}) for {} expires on {}. \
                 Scope: {}. Renewal action required to avoid business disruption.",
                cert.cert_type, cert.cert_body, cert.company_name, cert.expiry_date, cert.scope
            )
        } else {
            format!(
                "{}'s {} certification (issued by {}) expires on {}. \
                 Scope: {}. This creates a potential opportunity if they fail to renew — \
                 their customers may seek certified alternatives.",
                cert.company_name, cert.cert_type, cert.cert_body, cert.expiry_date, cert.scope
            )
        };

        Some(CertExpiryWarning {
            company_id: cert.company_id.clone(),
            company_name: cert.company_name.clone(),
            cert_type: cert.cert_type.clone(),
            expiry_date: cert.expiry_date,
            days_until_expiry: days_until,
            severity: severity.into(),
            warning_type: "cert_expiry_approaching".into(),
            title,
            description,
            signal_type,
            region: cert.region.clone(),
        })
    }

    /// Batch-check all certs and return warnings.
    pub fn check_all(&self, certs: &[CertRecord], today: NaiveDate) -> Vec<CertExpiryWarning> {
        certs
            .iter()
            .filter_map(|cert| self.check_cert(cert, today))
            .collect()
    }

    /// Get just the critical warnings (for immediate notification).
    pub fn critical_warnings(
        &self,
        certs: &[CertRecord],
        today: NaiveDate,
    ) -> Vec<CertExpiryWarning> {
        self.check_all(certs, today)
            .into_iter()
            .filter(|w| w.severity == "critical")
            .collect()
    }

    /// SQL query to find certs expiring within the warning window.
    pub fn expiring_certs_sql(&self) -> String {
        format!(
            "SELECT c.id, c.canonical_name, cert.cert_type, cert.issuing_body, \
             cert.issued_date, cert.expiry_date, cert.scope, \
             (c.id = ANY($1)) as is_own, c.region \
             FROM certifications cert \
             JOIN companies c ON c.id = cert.company_id \
             WHERE cert.expiry_date BETWEEN CURRENT_DATE AND CURRENT_DATE + INTERVAL '{} days' \
             AND cert.status = 'active'",
            self.config.warning_days
        )
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use chrono::Utc;

    fn make_cert(cert_type: &str, days_until_expiry: i64, is_own: bool) -> CertRecord {
        let today = Utc::now().date_naive();
        CertRecord {
            company_id: "comp-001".into(),
            company_name: if is_own {
                "Starz Electronics".into()
            } else {
                "RivalCorp".into()
            },
            cert_type: cert_type.into(),
            cert_body: "TUV".into(),
            issued_date: today - chrono::Duration::days(365),
            expiry_date: today + chrono::Duration::days(days_until_expiry),
            scope: "PCB assembly".into(),
            is_own,
            region: "TN".into(),
        }
    }

    #[test]
    fn test_cert_within_warning_window() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("IATF 16949", 45, false);
        let today = Utc::now().date_naive();
        let warning = checker.check_cert(&cert, today);
        assert!(warning.is_some());
        let w = warning.unwrap();
        assert_eq!(w.severity, "high");
        assert_eq!(w.signal_type, CertSignalType::CompetitorOpportunity);
    }

    #[test]
    fn test_cert_critical() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("AS9100", 15, true);
        let today = Utc::now().date_naive();
        let warning = checker.check_cert(&cert, today);
        assert!(warning.is_some());
        let w = warning.unwrap();
        assert_eq!(w.severity, "critical");
        assert_eq!(w.signal_type, CertSignalType::OwnRisk);
    }

    #[test]
    fn test_cert_far_future_ignored() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("ISO 9001", 200, false);
        let today = Utc::now().date_naive();
        assert!(checker.check_cert(&cert, today).is_none());
    }

    #[test]
    fn test_cert_already_expired_ignored() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("ISO 14001", -10, false);
        let today = Utc::now().date_naive();
        assert!(checker.check_cert(&cert, today).is_none());
    }

    #[test]
    fn test_batch_check() {
        let checker = CertExpiryChecker::with_defaults();
        let today = Utc::now().date_naive();
        let certs = vec![
            make_cert("IATF 16949", 20, true),
            make_cert("AS9100", 100, false),
            make_cert("ISO 9001", 60, false),
        ];
        let warnings = checker.check_all(&certs, today);
        assert_eq!(warnings.len(), 2); // 20 days (own) + 60 days (competitor)
    }

    #[test]
    fn test_own_cert_description() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("IATF 16949", 30, true);
        let today = Utc::now().date_naive();
        let warning = checker.check_cert(&cert, today).unwrap();
        assert!(warning.description.contains("Renewal action required"));
    }

    #[test]
    fn test_competitor_cert_description() {
        let checker = CertExpiryChecker::with_defaults();
        let cert = make_cert("ISO 9001", 60, false);
        let today = Utc::now().date_naive();
        let warning = checker.check_cert(&cert, today).unwrap();
        assert!(warning.description.contains("potential opportunity"));
    }

    #[test]
    fn test_sql_generation() {
        let checker = CertExpiryChecker::with_defaults();
        let sql = checker.expiring_certs_sql();
        assert!(sql.contains("90 days"));
        assert!(sql.contains("certifications"));
    }
}
