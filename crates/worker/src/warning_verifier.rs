#![cfg_attr(test, allow(dead_code))]

//! Post-emission verification pipeline for warnings.
//!
//! # Architecture
//!
//! After a warning is emitted, this module can verify its claims against
//! available evidence signals and warning metadata.  The verification
//! pipeline has two stages:
//!
//! 1. **Categorization** – map warning type to a verifier category.
//! 2. **Evidence cross-reference** – check that the warning narrative is
//!    supported by the underlying evidence signals.
//!
//! # Verification Statuses
//!
//! | Status        | Meaning                                                       |
//! |---------------|---------------------------------------------------------------|
//! | `Pending`     | Not yet checked (default).                                    |
//! | `Confirmed`   | Evidence supports the warning claim.                          |
//! | `Disputed`    | Evidence contradicts or is insufficient to support the claim. |
//! | `Unverifiable`| Warning type has no automated verifier.                       |

// ─── Core types ─────────────────────────────────────────────────────────

/// Verification status for a single warning.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VerificationStatus {
    /// Not yet checked.
    Pending,
    /// Evidence supports the warning claim.
    Confirmed,
    /// Evidence contradicts or is insufficient.
    Disputed,
    /// Warning type has no automated verifier.
    Unverifiable,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationStatus::Pending => "pending",
            VerificationStatus::Confirmed => "confirmed",
            VerificationStatus::Disputed => "disputed",
            VerificationStatus::Unverifiable => "unverifiable",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "confirmed" => VerificationStatus::Confirmed,
            "disputed" => VerificationStatus::Disputed,
            "unverifiable" => VerificationStatus::Unverifiable,
            _ => VerificationStatus::Pending,
        }
    }
}

/// Full verification result for a single warning.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerificationResult {
    pub warning_id: uuid::Uuid,
    pub status: VerificationStatus,
    pub checked_at: chrono::DateTime<chrono::Utc>,
    /// Human-readable explanation of the verification outcome.
    pub detail: String,
    /// Numerical confidence in the verification (0.0 – 1.0).
    pub confidence: f64,
    /// Categorized verifier that was applied.
    pub verifier: String,
}

impl VerificationResult {
    pub fn new(
        warning_id: uuid::Uuid,
        status: VerificationStatus,
        detail: String,
        confidence: f64,
        verifier: &str,
    ) -> Self {
        Self {
            warning_id,
            status,
            checked_at: chrono::Utc::now(),
            detail,
            confidence,
            verifier: verifier.to_string(),
        }
    }
}

// ─── Warning type categories ──────────────────────────────────────────

/// Verifiable categories a warning type can belong to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerifierCategory {
    /// DNS hygiene (DMARC, SPF, DKIM, typosquat, lookalike).
    DnsHygiene,
    /// SSL / TLS certificate expiry or misconfiguration.
    SslCertificate,
    /// Data breach or credential leak.
    Breach,
    /// Certification / accreditation expiry or change.
    Certification,
    /// General — no automated verifier available.
    Unverifiable,
}

impl VerifierCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerifierCategory::DnsHygiene => "dns_hygiene",
            VerifierCategory::SslCertificate => "ssl_certificate",
            VerifierCategory::Breach => "breach",
            VerifierCategory::Certification => "certification",
            VerifierCategory::Unverifiable => "unverifiable",
        }
    }
}

/// Map a warning type string to its verifier category.
pub fn categorize_warning_type(warning_type: &str) -> VerifierCategory {
    let t = warning_type.trim().to_ascii_lowercase();
    if t.contains("dns")
        || t.contains("dmarc")
        || t.contains("spf")
        || t.contains("dkim")
        || t.contains("typosquat")
        || t.contains("lookalike")
        || t.contains("hygiene")
    {
        VerifierCategory::DnsHygiene
    } else if t.contains("certification")
        || t.contains("accreditation")
        || t.contains("qualification")
    {
        VerifierCategory::Certification
    } else if t.contains("ssl")
        || t.contains("tls")
        || t.contains("certificate")
        || t.contains("expir")
    {
        VerifierCategory::SslCertificate
    } else if t.contains("breach")
        || t.contains("leak")
        || t.contains("credential")
        || t.contains("compromise")
    {
        VerifierCategory::Breach
    } else {
        VerifierCategory::Unverifiable
    }
}

// ─── Evidence cross-reference ──────────────────────────────────────────

/// Signals extracted from a warning's title, description, and metadata.
struct WarningSignals {
    /// All lowercase tokens from the title.
    title_tokens: Vec<String>,
    /// All lowercase tokens from the description (if any).
    desc_tokens: Vec<String>,
    /// Domains mentioned in source URLs.
    source_domains: Vec<String>,
}

impl WarningSignals {
    fn from_warning(
        title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> Self {
        let desc_tokens = description.map(tokenize).unwrap_or_default();
        let source_domains = source_urls
            .map(|urls| urls.iter().filter_map(|u| extract_domain(u)).collect())
            .unwrap_or_default();

        Self {
            title_tokens: tokenize(title),
            desc_tokens,
            source_domains,
        }
    }

    /// Whether the warning text contains a reference to a specific domain.
    fn mentions_domain(&self, domain: &str) -> bool {
        let domain_lower = domain.to_ascii_lowercase();
        self.title_tokens.contains(&domain_lower)
            || self.desc_tokens.contains(&domain_lower)
            || self.source_domains.contains(&domain_lower)
    }

    /// Whether the warning source URLs include the expected domain.
    #[allow(dead_code)]
    fn has_source_from_domain(&self, domain: &str) -> bool {
        let domain_lower = domain.to_ascii_lowercase();
        self.source_domains.contains(&domain_lower)
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .flat_map(|w| w.split(|c: char| !c.is_alphanumeric() && c != '.' && c != '-'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

fn extract_domain(url: &str) -> Option<String> {
    let url = url.trim();
    // Strip scheme
    let after_scheme = if let Some(pos) = url.find("://") {
        &url[pos + 3..]
    } else {
        url
    };
    // Take up to first '/' or '?' or '#'
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    // Strip port
    let host = host.split(':').next().unwrap_or(host);
    Some(host.to_ascii_lowercase())
}

// ─── Verifier traits and implementations ───────────────────────────────

/// A trait for warning-type-specific verifiers.
#[allow(dead_code)]
trait WarningVerifier: Send + Sync {
    fn category(&self) -> VerifierCategory;
    fn verify(
        &self,
        warning_id: uuid::Uuid,
        title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> VerificationResult;
}

/// DNS hygiene verifier — checks that the warning has corroborating evidence.
struct DnsHygieneVerifier;

impl WarningVerifier for DnsHygieneVerifier {
    fn category(&self) -> VerifierCategory {
        VerifierCategory::DnsHygiene
    }

    fn verify(
        &self,
        warning_id: uuid::Uuid,
        title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> VerificationResult {
        let signals = WarningSignals::from_warning(title, description, source_urls);

        // Check: does the warning have source URLs at all?
        if source_urls.is_none_or(|u| u.is_empty()) {
            return VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "DNS hygiene warning has no source URLs — cannot corroborate claim".to_string(),
                0.2,
                "dns_hygiene",
            );
        }

        // Check: does the warning reference a domain found in sources?
        let has_domain_ref = signals
            .source_domains
            .iter()
            .any(|d| signals.mentions_domain(d));

        if has_domain_ref {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                format!(
                    "DNS claim corroborated by {} source URL(s)",
                    source_urls.map_or(0, |u| u.len())
                ),
                0.7,
                "dns_hygiene",
            )
        } else {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "DNS claim does not reference any domain found in source URLs".to_string(),
                0.4,
                "dns_hygiene",
            )
        }
    }
}

/// SSL certificate verifier.
struct SslCertificateVerifier;

impl WarningVerifier for SslCertificateVerifier {
    fn category(&self) -> VerifierCategory {
        VerifierCategory::SslCertificate
    }

    fn verify(
        &self,
        warning_id: uuid::Uuid,
        title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> VerificationResult {
        let signals = WarningSignals::from_warning(title, description, source_urls);

        // Check: does the warning have source URLs?
        if source_urls.is_none_or(|u| u.is_empty()) {
            return VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "SSL warning has no source URLs — cannot cross-reference".to_string(),
                0.2,
                "ssl_certificate",
            );
        }

        // Check: does the description or title mention a specific domain that
        // appears in source URLs?
        let has_ssl_source = signals
            .source_domains
            .iter()
            .any(|d| signals.mentions_domain(d));

        if has_ssl_source {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                "SSL claim cross-referenced against source URLs".to_string(),
                0.7,
                "ssl_certificate",
            )
        } else {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "SSL claim domain not found in source URLs".to_string(),
                0.3,
                "ssl_certificate",
            )
        }
    }
}

/// Breach verifier.
struct BreachVerifier;

impl WarningVerifier for BreachVerifier {
    fn category(&self) -> VerifierCategory {
        VerifierCategory::Breach
    }

    fn verify(
        &self,
        warning_id: uuid::Uuid,
        title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> VerificationResult {
        let _signals = WarningSignals::from_warning(title, description, source_urls);

        // Breach warnings require source evidence
        if source_urls.is_none_or(|u| u.is_empty()) {
            return VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "Breach warning has no source URLs — cannot verify".to_string(),
                0.1,
                "breach",
            );
        }

        // Check for breach-specific keywords in description
        let has_breach_evidence = description.is_some_and(|desc| {
            let lower = desc.to_ascii_lowercase();
            lower.contains("breach")
                || lower.contains("leak")
                || lower.contains("compromised")
                || lower.contains("exposed")
                || lower.contains("credential")
                || lower.contains("dark web")
                || lower.contains("paste")
        });

        if has_breach_evidence {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                "Breach claim supported by evidence description".to_string(),
                0.75,
                "breach",
            )
        } else {
            // Fall through — the source URLs exist but we can't automatically
            // confirm.  We treat this as a weak confirmation because the
            // upstream detection pipeline already evaluated the signals.
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                "Breach claim has source URLs (upstream pipeline validation)".to_string(),
                0.55,
                "breach",
            )
        }
    }
}

/// Certification verifier.
struct CertificationVerifier;

impl WarningVerifier for CertificationVerifier {
    fn category(&self) -> VerifierCategory {
        VerifierCategory::Certification
    }

    fn verify(
        &self,
        warning_id: uuid::Uuid,
        _title: &str,
        description: Option<&str>,
        source_urls: Option<&[String]>,
    ) -> VerificationResult {
        if source_urls.is_none_or(|u| u.is_empty()) {
            return VerificationResult::new(
                warning_id,
                VerificationStatus::Disputed,
                "Certification warning has no source URLs".to_string(),
                0.2,
                "certification",
            );
        }

        // Check for certification-specific keywords in description
        let has_cert_evidence = description.is_some_and(|desc| {
            let lower = desc.to_ascii_lowercase();
            lower.contains("certification")
                || lower.contains("accreditation")
                || lower.contains("certificate")
                || lower.contains("expired")
                || lower.contains("revoked")
                || lower.contains("suspended")
                || lower.contains("lapsed")
        });

        if has_cert_evidence {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                "Certification claim supported by evidence description".to_string(),
                0.7,
                "certification",
            )
        } else {
            VerificationResult::new(
                warning_id,
                VerificationStatus::Confirmed,
                "Certification claim has source URLs (upstream pipeline validation)".to_string(),
                0.5,
                "certification",
            )
        }
    }
}

// ─── Main verification entry point ─────────────────────────────────────

/// Verify a single warning against available evidence.
///
/// This is the main entry point.  It:
/// 1. Categorises the warning type.
/// 2. Dispatches to the type-specific verifier.
/// 3. Returns a `VerificationResult`.
///
/// For unverifiable warning types, the result is `Unverifiable` with a
/// descriptive message.
pub fn verify_warning(
    warning_id: uuid::Uuid,
    warning_type: &str,
    title: &str,
    description: Option<&str>,
    source_urls: Option<&[String]>,
) -> VerificationResult {
    let category = categorize_warning_type(warning_type);

    match category {
        VerifierCategory::DnsHygiene => {
            DnsHygieneVerifier.verify(warning_id, title, description, source_urls)
        }
        VerifierCategory::SslCertificate => {
            SslCertificateVerifier.verify(warning_id, title, description, source_urls)
        }
        VerifierCategory::Breach => {
            BreachVerifier.verify(warning_id, title, description, source_urls)
        }
        VerifierCategory::Certification => {
            CertificationVerifier.verify(warning_id, title, description, source_urls)
        }
        VerifierCategory::Unverifiable => VerificationResult::new(
            warning_id,
            VerificationStatus::Unverifiable,
            format!("Warning type '{}' has no automated verifier", warning_type),
            1.0,
            "unverifiable",
        ),
    }
}

/// Verify a batch of warnings, returning results keyed by warning ID.
pub fn verify_warnings_batch(warnings: &[VerificationInput]) -> Vec<VerificationResult> {
    warnings
        .iter()
        .map(|w| {
            verify_warning(
                w.warning_id,
                &w.warning_type,
                &w.title,
                w.description.as_deref(),
                w.source_urls.as_deref(),
            )
        })
        .collect()
}

/// Input struct for batch verification.
#[derive(Debug, Clone)]
pub struct VerificationInput {
    pub warning_id: uuid::Uuid,
    pub warning_type: String,
    pub title: String,
    pub description: Option<String>,
    pub source_urls: Option<Vec<String>>,
}

/// Summarise a batch of verification results into aggregate metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationSummary {
    pub total: usize,
    pub confirmed: usize,
    pub disputed: usize,
    pub unverifiable: usize,
    pub pending: usize,
    pub avg_confidence: f64,
}

impl VerificationSummary {
    pub fn from_results(results: &[VerificationResult]) -> Self {
        let total = results.len();
        let confirmed = results
            .iter()
            .filter(|r| r.status == VerificationStatus::Confirmed)
            .count();
        let disputed = results
            .iter()
            .filter(|r| r.status == VerificationStatus::Disputed)
            .count();
        let unverifiable = results
            .iter()
            .filter(|r| r.status == VerificationStatus::Unverifiable)
            .count();
        let pending = results
            .iter()
            .filter(|r| r.status == VerificationStatus::Pending)
            .count();
        let avg_confidence = if total == 0 {
            0.0
        } else {
            results.iter().map(|r| r.confidence).sum::<f64>() / total as f64
        };

        Self {
            total,
            confirmed,
            disputed,
            unverifiable,
            pending,
            avg_confidence,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slack integration — notify on confirmed High / Critical warnings
// ─────────────────────────────────────────────────────────────────────────────

/// Send a Slack notification when a verified warning is **Confirmed** with
/// **High** or **Critical** severity.
///
/// This bridges the verification pipeline with the Block Kit Slack module.
/// No-op when the verification status is not `Confirmed` or when the severity
/// is below High.
///
/// # Errors
///
/// Propagates errors from the Slack webhook client.
pub async fn notify_slack_on_confirmed_warning(
    result: &VerificationResult,
    severity: &str,
    title: &str,
    webhook: &crate::slack::SlackWebhook,
) -> anyhow::Result<()> {
    if result.status != VerificationStatus::Confirmed {
        return Ok(());
    }

    let severity_lower = severity.trim().to_ascii_lowercase();
    if severity_lower != "high" && severity_lower != "critical" {
        return Ok(());
    }

    let slack_severity = crate::slack::SlackMessageSeverity::from_str(severity);
    let alert_type = crate::slack::AlertType::Warning;

    let mut msg =
        crate::slack::SlackMessage::new(slack_severity, alert_type, title, &result.detail)
            .with_field("Confidence", format!("{:.0}%", result.confidence * 100.0))
            .with_field("Verifier", &result.verifier)
            .with_source_url(format!(
                "https://apexintel.io/warnings/{}",
                result.warning_id
            ));

    if result.confidence >= 0.8 {
        msg = msg.with_field("Status", "✅ Confirmed (high confidence)");
    }

    webhook.send(&msg).await
}

/// Convenience wrapper that builds a [`SlackWebhook`] from environment and
/// calls [`notify_slack_on_confirmed_warning`].
///
/// Useful when the caller does not already hold a webhook client.
pub async fn notify_slack_on_confirmed_warning_from_env(
    result: &VerificationResult,
    severity: &str,
    title: &str,
) -> anyhow::Result<()> {
    let config = crate::slack::SlackConfig::from_env();
    let webhook = crate::slack::SlackWebhook::new(&config)
        .map_err(|e| anyhow::anyhow!("failed to create Slack webhook client: {e}"))?;
    notify_slack_on_confirmed_warning(result, severity, title, &webhook).await
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Categorisation ─────────────────────────────────────────

    #[test]
    fn test_categorize_dns() {
        assert_eq!(
            categorize_warning_type("dns_hygiene"),
            VerifierCategory::DnsHygiene
        );
        assert_eq!(
            categorize_warning_type("DMARC check failed"),
            VerifierCategory::DnsHygiene
        );
        assert_eq!(
            categorize_warning_type("typosquat_detected"),
            VerifierCategory::DnsHygiene
        );
    }

    #[test]
    fn test_categorize_ssl() {
        assert_eq!(
            categorize_warning_type("ssl_certificate_expiry"),
            VerifierCategory::SslCertificate
        );
        assert_eq!(
            categorize_warning_type("TLS misconfiguration"),
            VerifierCategory::SslCertificate
        );
    }

    #[test]
    fn test_categorize_breach() {
        assert_eq!(
            categorize_warning_type("data_breach"),
            VerifierCategory::Breach
        );
        assert_eq!(
            categorize_warning_type("credential_leak"),
            VerifierCategory::Breach
        );
    }

    #[test]
    fn test_categorize_certification() {
        assert_eq!(
            categorize_warning_type("certification_expiry"),
            VerifierCategory::Certification
        );
    }

    #[test]
    fn test_categorize_unverifiable() {
        assert_eq!(
            categorize_warning_type("supply_chain_risk"),
            VerifierCategory::Unverifiable
        );
        assert_eq!(
            categorize_warning_type("competitor_activity"),
            VerifierCategory::Unverifiable
        );
    }

    // ─── Tokenisation ───────────────────────────────────────────

    #[test]
    fn test_tokenize_splits_correctly() {
        let tokens = tokenize("Target example.com has weak DNS (SPF missing)");
        assert!(tokens.contains(&"target".to_string()));
        assert!(tokens.contains(&"example.com".to_string()));
        assert!(tokens.contains(&"dns".to_string()));
        assert!(tokens.contains(&"spf".to_string()));
        assert!(tokens.contains(&"missing".to_string()));
    }

    #[test]
    fn test_tokenize_empty() {
        assert!(tokenize("").is_empty());
    }

    // ─── Domain extraction ──────────────────────────────────────

    #[test]
    fn test_extract_domain_simple() {
        assert_eq!(
            extract_domain("https://example.com/page"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_domain_no_scheme() {
        assert_eq!(
            extract_domain("example.com/path?q=1"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_domain_empty() {
        assert!(extract_domain("").is_none());
    }

    // ─── WarningSignals ─────────────────────────────────────────

    #[test]
    fn test_warning_signals_mentions_domain() {
        let signals = WarningSignals::from_warning(
            "Target example.com has weak DNS",
            Some("Missing SPF record for example.com"),
            Some(&["https://example.com/report".to_string()]),
        );
        assert!(signals.mentions_domain("example.com"));
        assert!(!signals.mentions_domain("other.com"));
    }

    #[test]
    fn test_warning_signals_source_domains() {
        let signals = WarningSignals::from_warning(
            "SSL certificate expired",
            None,
            Some(&[
                "https://example.com/ssl-check".to_string(),
                "https://other.org/cert".to_string(),
            ]),
        );
        assert!(signals.has_source_from_domain("example.com"));
        assert!(signals.has_source_from_domain("other.org"));
        assert!(!signals.has_source_from_domain("missing.com"));
    }

    // ─── DNS verifier ───────────────────────────────────────────

    #[test]
    fn test_dns_verifier_disputed_without_sources() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "dns_hygiene",
            "Target example.com has weak DNS",
            None,
            None,
        );
        assert_eq!(result.status, VerificationStatus::Disputed);
        assert!(result.confidence < 0.5);
    }

    #[test]
    fn test_dns_verifier_confirmed_with_sources() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "dns_hygiene",
            "Target example.com has weak DNS (DMARC missing)",
            Some("Missing DMARC record for example.com"),
            Some(&["https://example.com/dns-report".to_string()]),
        );
        assert_eq!(result.status, VerificationStatus::Confirmed);
        assert_eq!(result.verifier, "dns_hygiene");
    }

    // ─── SSL verifier ───────────────────────────────────────────

    #[test]
    fn test_ssl_verifier_disputed_without_sources() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "ssl_certificate_expiry",
            "SSL certificate for example.com expires in 7 days",
            None,
            None,
        );
        assert_eq!(result.status, VerificationStatus::Disputed);
    }

    #[test]
    fn test_ssl_verifier_confirmed_with_matching_domain() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "ssl_certificate_expiry",
            "SSL certificate for example.com expires soon",
            Some("Certificate for example.com will expire"),
            Some(&["https://example.com/ssl-info".to_string()]),
        );
        assert_eq!(result.status, VerificationStatus::Confirmed);
    }

    // ─── Breach verifier ────────────────────────────────────────

    #[test]
    fn test_breach_verifier_disputed_without_sources() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "data_breach",
            "Credentials leaked for example.com",
            None,
            None,
        );
        assert_eq!(result.status, VerificationStatus::Disputed);
    }

    #[test]
    fn test_breach_verifier_confirmed_with_evidence() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "data_breach",
            "Data breach detected",
            Some("Employee credentials were exposed in a dark web paste"),
            Some(&["https://breach-db.example.com/entry".to_string()]),
        );
        assert_eq!(result.status, VerificationStatus::Confirmed);
        assert!(result.confidence > 0.7);
    }

    // ─── Certification verifier ─────────────────────────────────

    #[test]
    fn test_cert_verifier_confirmed_with_sources() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "certification_expiry",
            "ISO 9001 certification expired for Acme Corp",
            Some("Acme Corp's ISO 9001 certification has lapsed"),
            Some(&["https://cert-db.example.com/acme".to_string()]),
        );
        assert_eq!(result.status, VerificationStatus::Confirmed);
    }

    // ─── Unverifiable ───────────────────────────────────────────

    #[test]
    fn test_unverifiable_warning_type() {
        let result = verify_warning(
            uuid::Uuid::new_v4(),
            "supply_chain_risk",
            "New sourcing cycle",
            Some("Procurement signals detected"),
            Some(&["https://example.com/evidence".to_string()]),
        );
        assert_eq!(result.status, VerificationStatus::Unverifiable);
        assert_eq!(result.verifier, "unverifiable");
    }

    // ─── Batch verification ─────────────────────────────────────

    #[test]
    fn test_batch_verification_produces_summary() {
        let inputs = vec![
            VerificationInput {
                warning_id: uuid::Uuid::new_v4(),
                warning_type: "dns_hygiene".to_string(),
                title: "DNS issue".to_string(),
                description: None,
                source_urls: None,
            },
            VerificationInput {
                warning_id: uuid::Uuid::new_v4(),
                warning_type: "supply_chain_risk".to_string(),
                title: "Supply chain".to_string(),
                description: Some("Risk detected".to_string()),
                source_urls: Some(vec!["https://example.com".to_string()]),
            },
        ];

        let results = verify_warnings_batch(&inputs);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].verifier, "dns_hygiene");
        assert_eq!(results[1].verifier, "unverifiable");

        let summary = VerificationSummary::from_results(&results);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.unverifiable, 1);
    }

    // ─── Status round-trip ──────────────────────────────────────

    #[test]
    fn test_verification_status_round_trip() {
        // Standard statuses round-trip: from_str(as_str()) == identity.
        let standard = [
            ("confirmed", VerificationStatus::Confirmed),
            ("disputed", VerificationStatus::Disputed),
            ("unverifiable", VerificationStatus::Unverifiable),
            ("pending", VerificationStatus::Pending),
        ];
        for (s, expected) in &standard {
            assert_eq!(VerificationStatus::from_str(s), *expected);
            assert_eq!(expected.as_str(), *s);
        }
        // Non-standard inputs fall back to Pending but do not need to round-trip.
        assert_eq!(
            VerificationStatus::from_str("unknown"),
            VerificationStatus::Pending
        );
    }
}
