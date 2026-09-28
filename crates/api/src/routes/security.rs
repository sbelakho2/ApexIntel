//! Security route — request/response types and logic for security endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use apex_store::postgres::{PgStore, SourceRuntimeStateRow, WorkerJobStateRecord};

/// Negative-state taxonomy for a security-source scan.
///
/// A security surface must never render a bare "0 findings": zero is only a
/// statement when a scan actually ran. Every other situation has its own
/// state so an operator can tell "nothing found" from "nothing happened".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySourceState {
    /// Findings were reported by the source.
    FindingsReported,
    /// The source was scanned successfully and reported no findings.
    NoFindingsAfterSuccessfulScan,
    /// No scan has been recorded for this source in this deployment.
    NotScanned,
    /// The last scan failed.
    ScanFailed,
    /// The source requires credentials the deployment does not have.
    AuthenticationUnavailable,
    /// The upstream rejected the scan with a rate limit.
    RateLimited,
    /// The source endpoint is unreachable or missing its capability (e.g. no
    /// Tor/I2P proxy).
    SourceUnavailable,
    /// Some part of the scan completed and some part did not.
    PartialScan,
    /// The scan succeeded but the finding count is not measurable here.
    ScanSucceeded,
}

impl SecuritySourceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FindingsReported => "findings_reported",
            Self::NoFindingsAfterSuccessfulScan => "no_findings_after_successful_scan",
            Self::NotScanned => "not_scanned",
            Self::ScanFailed => "scan_failed",
            Self::AuthenticationUnavailable => "authentication_unavailable",
            Self::RateLimited => "rate_limited",
            Self::SourceUnavailable => "source_unavailable",
            Self::PartialScan => "partial_scan",
            Self::ScanSucceeded => "scan_succeeded",
        }
    }

    /// True for the states that must never be presented as a clean result.
    pub fn is_negative(self) -> bool {
        matches!(
            self,
            Self::NotScanned
                | Self::ScanFailed
                | Self::AuthenticationUnavailable
                | Self::RateLimited
                | Self::SourceUnavailable
                | Self::PartialScan
        )
    }
}

/// One security source's measured status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecuritySourceStatus {
    /// Stable machine id (e.g. `cve`, `dark_web`).
    pub id: String,
    pub label: String,
    pub state: SecuritySourceState,
    /// Human-readable classification, never a bare "0 findings".
    pub detail: String,
    /// Findings reported by the last successful measurement, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<String>,
}

/// Classify one scan from its last recorded status/error and finding count.
///
/// `last_status` is the worker job's terminal status (`succeeded`, `failed`,
/// `degraded`, `skipped`) or the source runtime row's condition.
pub fn classify_scan_state(
    last_status: Option<&str>,
    last_error: Option<&str>,
    findings: Option<u64>,
) -> SecuritySourceState {
    let Some(status) = last_status.map(str::trim).filter(|s| !s.is_empty()) else {
        return SecuritySourceState::NotScanned;
    };
    let error = last_error.unwrap_or_default().to_ascii_lowercase();
    match status.to_ascii_lowercase().as_str() {
        "skipped" => SecuritySourceState::NotScanned,
        "failed" | "error" => classify_failure(&error),
        "degraded" | "partial" => SecuritySourceState::PartialScan,
        "succeeded" | "success" | "ok" | "status_ok" => match findings {
            Some(0) => SecuritySourceState::NoFindingsAfterSuccessfulScan,
            Some(_) => SecuritySourceState::FindingsReported,
            None => SecuritySourceState::ScanSucceeded,
        },
        _ => classify_failure(&error),
    }
}

fn classify_failure(error: &str) -> SecuritySourceState {
    let mentions = |needles: &[&str]| needles.iter().any(|needle| error.contains(needle));
    if mentions(&[
        "401",
        "403",
        "authentication",
        "unauthorized",
        "forbidden",
        "credential",
        "api key",
        "api_key",
        "token",
        "login",
    ]) {
        SecuritySourceState::AuthenticationUnavailable
    } else if mentions(&[
        "429",
        "rate limit",
        "rate-limit",
        "too many requests",
        "quota",
    ]) {
        SecuritySourceState::RateLimited
    } else if mentions(&[
        "timeout",
        "timed out",
        "connect",
        "connection",
        "dns",
        "resolve",
        "unreachable",
        "refused",
        "proxy",
        "socks",
        "tor",
        "i2p",
        "network",
        "unavailable",
    ]) {
        SecuritySourceState::SourceUnavailable
    } else {
        SecuritySourceState::ScanFailed
    }
}

/// A security source family tracked by the taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecuritySourceSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Worker job kind backing this source, when one exists.
    pub job_kind: Option<&'static str>,
    /// Registered crawl source slug backing this source, when one exists.
    pub source_slug: Option<&'static str>,
    /// Environment variables that must all be configured for the source to
    /// be reachable (empty when the source needs no credentials).
    pub credentials: &'static [&'static str],
}

/// The security source families surfaced by the API and UI.
pub const SECURITY_SOURCE_SPECS: &[SecuritySourceSpec] = &[
    SecuritySourceSpec {
        id: "cve",
        label: "CVE / CISA KEV",
        job_kind: Some("kev_catalog_fetch"),
        source_slug: Some("cisa_advisories"),
        credentials: &[],
    },
    SecuritySourceSpec {
        id: "dark_web",
        label: "Dark web forums",
        job_kind: Some("dark_web_scan"),
        source_slug: None,
        credentials: &[],
    },
    SecuritySourceSpec {
        id: "censys",
        label: "Censys attack surface",
        job_kind: None,
        source_slug: Some("censys_attack_surface"),
        credentials: &["CENSYS_API_ID", "CENSYS_API_SECRET"],
    },
    SecuritySourceSpec {
        id: "github_code_exposure",
        label: "GitHub code exposure",
        job_kind: None,
        // Code-exposure monitoring is not a registered crawl source yet; the
        // public `github_security_advisories` feed is *advisories*, not code
        // exposure, so it must not be reported under this row. Without the
        // token the honest state is authentication unavailable; with it the
        // row is not scanned until a real code-exposure source exists.
        source_slug: None,
        credentials: &["GITHUB_TOKEN"],
    },
    SecuritySourceSpec {
        id: "i2p",
        label: "I2P hidden services",
        job_kind: Some("dark_web_scan"),
        source_slug: None,
        credentials: &["DARKWEB_TOR_PROXY"],
    },
    SecuritySourceSpec {
        id: "marketplaces",
        label: "Dark web marketplaces",
        job_kind: Some("dark_web_scan"),
        source_slug: None,
        credentials: &[],
    },
];

/// Build the per-source status report from the latest worker job states and
/// source runtime rows. `configured` answers whether a credential env var is
/// present; `findings` supplies the count for a source when the caller could
/// measure it.
pub fn build_security_source_statuses<F>(
    job_states: &[WorkerJobStateRecord],
    source_states: &[SourceRuntimeStateRow],
    configured: F,
    findings: &dyn Fn(&str) -> Option<u64>,
) -> Vec<SecuritySourceStatus>
where
    F: Fn(&str) -> bool,
{
    SECURITY_SOURCE_SPECS
        .iter()
        .map(|spec| {
            let missing: Vec<&str> = spec
                .credentials
                .iter()
                .copied()
                .filter(|key| !configured(key))
                .collect();
            let job = spec
                .job_kind
                .and_then(|kind| job_states.iter().find(|state| state.job_kind == kind));
            let source = spec
                .source_slug
                .and_then(|slug| source_states.iter().find(|state| state.source_slug == slug));
            let count = findings(spec.id);

            let (state, detail) = if !missing.is_empty() {
                (
                    SecuritySourceState::AuthenticationUnavailable,
                    format!(
                        "required credentials are not configured: {}",
                        missing.join(", ")
                    ),
                )
            } else if let Some(job) = job {
                let state = classify_scan_state(
                    job.last_status.as_deref(),
                    job.last_error.as_deref(),
                    count,
                );
                (state, job_detail(job, state, count))
            } else if let Some(source) = source {
                let status = source_status_label(source);
                let state = classify_scan_state(Some(status), source.last_error.as_deref(), count);
                (state, source_detail(source, state, count))
            } else {
                (
                    SecuritySourceState::NotScanned,
                    "no scan recorded for this source in this deployment".to_string(),
                )
            };

            SecuritySourceStatus {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                state,
                detail,
                findings: count,
                last_scan_at: job
                    .and_then(|job| job.last_run)
                    .or_else(|| source.and_then(|source| source.last_attempt_at))
                    .map(|ts| ts.to_rfc3339()),
            }
        })
        .collect()
}

/// Load the full security-source report from worker job states, source
/// runtime rows and the dark-web warning count.
///
/// `/api/security` and `/security` both call this so the two surfaces cannot
/// drift on the same source's state or credentials. `cve_findings` is the
/// caller-measured KEV count; `None` means the measurement failed or was not
/// taken and must never render as a clean zero.
pub async fn load_security_source_statuses(
    store: &PgStore,
    cve_findings: Option<u64>,
) -> Vec<SecuritySourceStatus> {
    let job_states = store
        .list_worker_job_states()
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "security source states: worker job state query failed");
            vec![]
        });
    let source_states = store
        .load_source_runtime_states()
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "security source states: source runtime query failed");
            vec![]
        });
    let dark_web_findings = store
        .count_warnings_by_type("dark_web")
        .await
        .ok()
        .map(|count| count.max(0) as u64);
    build_security_source_statuses(
        &job_states,
        &source_states,
        |key| {
            std::env::var(key)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        },
        &|id| match id {
            "cve" => cve_findings,
            "dark_web" | "i2p" | "marketplaces" => dark_web_findings,
            _ => None,
        },
    )
}

fn job_detail(
    job: &WorkerJobStateRecord,
    state: SecuritySourceState,
    findings: Option<u64>,
) -> String {
    let error = job.last_error.as_deref().unwrap_or("").trim();
    let suffix = if error.is_empty() {
        String::new()
    } else {
        format!(": {error}")
    };
    match state {
        SecuritySourceState::FindingsReported => {
            format!("scan succeeded with {}{suffix}", count_label(findings))
        }
        SecuritySourceState::NoFindingsAfterSuccessfulScan => {
            format!("last scan succeeded and reported no findings{suffix}")
        }
        SecuritySourceState::ScanSucceeded => {
            format!("last scan succeeded; finding count not measured here{suffix}")
        }
        SecuritySourceState::NotScanned => {
            "last run was skipped; treated as not scanned".to_string()
        }
        _ => format!(
            "last run status '{}'{suffix}",
            job.last_status.as_deref().unwrap_or("unknown")
        ),
    }
}

fn source_detail(
    source: &SourceRuntimeStateRow,
    state: SecuritySourceState,
    findings: Option<u64>,
) -> String {
    let error = source.last_error.as_deref().unwrap_or("").trim();
    let http = source
        .last_http_status
        .map(|status| format!(" (HTTP {status})"))
        .unwrap_or_default();
    match state {
        SecuritySourceState::FindingsReported => {
            format!("source is operational with {}{http}", count_label(findings))
        }
        SecuritySourceState::NoFindingsAfterSuccessfulScan => {
            format!("source is operational and reported no findings{http}")
        }
        SecuritySourceState::NotScanned => {
            "source has never produced a successful, validated fetch".to_string()
        }
        _ if !error.is_empty() => format!("source state: {error}{http}"),
        _ => format!("source state does not prove a successful security scan{http}"),
    }
}

fn count_label(findings: Option<u64>) -> String {
    match findings {
        Some(count) => format!("{count} finding(s)"),
        None => "an unmeasured number of findings".to_string(),
    }
}

fn source_status_label(source: &SourceRuntimeStateRow) -> &'static str {
    if source
        .circuit_open_until
        .map(|until| until > Utc::now())
        .unwrap_or(false)
    {
        "failed"
    } else if source.last_success_at.is_some() && source.last_error.is_none() {
        "succeeded"
    } else if source.last_error.is_some() {
        "failed"
    } else {
        "skipped"
    }
}

/// Internal record used by the legacy list_security aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureRecord {
    pub domain: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub score: f64,
    pub checked_at: DateTime<Utc>,
}

/// DNS posture item returned by GET /api/security/dns-posture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureItem {
    pub domain: String,
    pub company_id: String,
    pub company_name: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub dmarc_policy: Option<String>,
    pub posture_score: f64,
    pub last_checked: String,
}

/// Response body for GET /api/security/dns-posture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureOverview {
    pub items: Vec<DnsPostureItem>,
    pub overall_score: f64,
    pub domains_checked: usize,
}

/// Lookalike domain entry returned by GET /api/security/lookalike-domains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeDomainItem {
    pub id: String,
    pub original_domain: String,
    pub lookalike_domain: String,
    pub distance: i64,
    pub threat_type: String,
    pub detected_at: String,
    pub active: bool,
    pub registrar: Option<String>,
    pub registration_date: Option<String>,
}

/// KEV entry returned by GET /api/security/kev-relevance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KevItem {
    pub cve_id: String,
    pub vendor: String,
    pub product: String,
    pub vulnerability_name: String,
    pub date_added: String,
    pub due_date: String,
    pub relevance_score: f64,
    pub affected_companies: Vec<String>,
    pub notes: Option<String>,
}

/// Internal types kept for potential future use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeDomain {
    pub domain: String,
    pub similarity: f64,
    pub risk: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KevRelevance {
    pub cve_id: String,
    pub vendor: String,
    pub product: String,
    pub relevance_score: f64,
    pub rationale: String,
}

/// Scalar summary returned by GET /api/security.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecuritySummary {
    pub dns_posture_score: f64,
    pub lookalike_domains_detected: u64,
    pub kev_matches: u64,
    pub last_scan_at: Option<String>,
    pub domains_monitored: u64,
    /// Per-source negative-state taxonomy. Never a bare "0 findings": every
    /// source reports its measured state (not scanned, failed, auth
    /// unavailable, rate limited, unavailable, partial, or a real zero).
    #[serde(default)]
    pub source_states: Vec<SecuritySourceStatus>,
}

pub fn dns_score(has_spf: bool, has_dkim: bool, has_dmarc: bool) -> f64 {
    let mut score: f64 = 0.0;
    if has_spf {
        score += 0.3;
    }
    if has_dkim {
        score += 0.3;
    }
    if has_dmarc {
        score += 0.4;
    }
    score.min(1.0)
}

pub fn classify_lookalike_risk(similarity: f64) -> &'static str {
    if similarity >= 0.9 {
        "critical"
    } else if similarity >= 0.8 {
        "high"
    } else if similarity >= 0.7 {
        "medium"
    } else {
        "low"
    }
}

pub fn relevance_tier(score: f64) -> &'static str {
    if score >= 0.8 {
        "high"
    } else if score >= 0.5 {
        "medium"
    } else {
        "low"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_score() {
        assert!((dns_score(true, true, true) - 1.0).abs() < 0.001);
        assert!((dns_score(true, false, false) - 0.3).abs() < 0.001);
        assert!((dns_score(false, false, false) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_classify_lookalike_risk() {
        assert_eq!(classify_lookalike_risk(0.92), "critical");
        assert_eq!(classify_lookalike_risk(0.82), "high");
        assert_eq!(classify_lookalike_risk(0.71), "medium");
        assert_eq!(classify_lookalike_risk(0.5), "low");
    }

    #[test]
    fn test_relevance_tier() {
        assert_eq!(relevance_tier(0.9), "high");
        assert_eq!(relevance_tier(0.6), "medium");
        assert_eq!(relevance_tier(0.2), "low");
    }

    #[test]
    fn test_dns_posture_serialization() {
        let v = DnsPostureRecord {
            domain: "example.com".to_string(),
            has_spf: true,
            has_dkim: true,
            has_dmarc: false,
            score: 0.6,
            checked_at: Utc::now(),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("example.com"));
    }

    fn job(
        kind: &str,
        status: &str,
        error: Option<&str>,
        run_at: Option<DateTime<Utc>>,
    ) -> WorkerJobStateRecord {
        WorkerJobStateRecord {
            job_kind: kind.to_string(),
            last_run: run_at,
            last_status: Some(status.to_string()),
            last_error: error.map(str::to_string),
            last_duration_ms: None,
            consecutive_failures: 0,
            max_consecutive_failures: 5,
            circuit_open: false,
            updated_at: Utc::now(),
        }
    }

    fn configured(_: &str) -> bool {
        true
    }

    fn no_findings(_: &str) -> Option<u64> {
        None
    }

    #[test]
    fn scan_state_classifier_covers_every_negative_state() {
        assert_eq!(
            classify_scan_state(None, None, None),
            SecuritySourceState::NotScanned
        );
        assert_eq!(
            classify_scan_state(Some("skipped"), None, None),
            SecuritySourceState::NotScanned
        );
        assert_eq!(
            classify_scan_state(Some("failed"), Some("HTTP 500 from CISA"), None),
            SecuritySourceState::ScanFailed
        );
        assert_eq!(
            classify_scan_state(Some("failed"), Some("HTTP 401 unauthorized"), None),
            SecuritySourceState::AuthenticationUnavailable
        );
        assert_eq!(
            classify_scan_state(Some("failed"), Some("HTTP 429 too many requests"), None),
            SecuritySourceState::RateLimited
        );
        assert_eq!(
            classify_scan_state(Some("failed"), Some("connect timeout to upstream"), None),
            SecuritySourceState::SourceUnavailable
        );
        assert_eq!(
            classify_scan_state(Some("degraded"), None, None),
            SecuritySourceState::PartialScan
        );
        assert_eq!(
            classify_scan_state(Some("succeeded"), None, Some(0)),
            SecuritySourceState::NoFindingsAfterSuccessfulScan
        );
        assert_eq!(
            classify_scan_state(Some("succeeded"), None, Some(7)),
            SecuritySourceState::FindingsReported
        );
        assert_eq!(
            classify_scan_state(Some("succeeded"), None, None),
            SecuritySourceState::ScanSucceeded
        );
    }

    #[test]
    fn github_code_exposure_does_not_claim_the_public_advisories_feed() {
        let spec = SECURITY_SOURCE_SPECS
            .iter()
            .find(|spec| spec.id == "github_code_exposure")
            .expect("github code exposure spec");
        assert!(
            spec.source_slug.is_none(),
            "the public github_security_advisories feed is not code-exposure coverage"
        );
        assert_eq!(spec.credentials, &["GITHUB_TOKEN"]);
    }

    #[test]
    fn only_findings_states_are_non_negative() {
        assert!(!SecuritySourceState::FindingsReported.is_negative());
        assert!(!SecuritySourceState::NoFindingsAfterSuccessfulScan.is_negative());
        assert!(!SecuritySourceState::ScanSucceeded.is_negative());
        for state in [
            SecuritySourceState::NotScanned,
            SecuritySourceState::ScanFailed,
            SecuritySourceState::AuthenticationUnavailable,
            SecuritySourceState::RateLimited,
            SecuritySourceState::SourceUnavailable,
            SecuritySourceState::PartialScan,
        ] {
            assert!(state.is_negative(), "{state:?} must be a negative state");
        }
    }

    #[test]
    fn security_source_states_serialize_to_distinct_tokens() {
        let mut tokens = std::collections::BTreeSet::new();
        for state in [
            SecuritySourceState::FindingsReported,
            SecuritySourceState::NoFindingsAfterSuccessfulScan,
            SecuritySourceState::NotScanned,
            SecuritySourceState::ScanFailed,
            SecuritySourceState::AuthenticationUnavailable,
            SecuritySourceState::RateLimited,
            SecuritySourceState::SourceUnavailable,
            SecuritySourceState::PartialScan,
            SecuritySourceState::ScanSucceeded,
        ] {
            let json = serde_json::to_value(state).expect("state serializes");
            assert_eq!(json, state.as_str());
            assert!(tokens.insert(state.as_str()), "duplicate state token");
        }
    }

    #[test]
    fn builder_reports_every_family_and_never_silently_zero() {
        let run_at = Utc::now();
        let jobs = vec![job("kev_catalog_fetch", "succeeded", None, Some(run_at))];
        let statuses = build_security_source_statuses(&jobs, &[], configured, &|id| match id {
            "cve" => Some(0),
            _ => None,
        });

        assert_eq!(statuses.len(), SECURITY_SOURCE_SPECS.len());
        let cve = statuses.iter().find(|s| s.id == "cve").expect("cve row");
        assert_eq!(
            cve.state,
            SecuritySourceState::NoFindingsAfterSuccessfulScan
        );
        assert!(cve.detail.contains("no findings"));

        let censys = statuses
            .iter()
            .find(|s| s.id == "censys")
            .expect("censys row");
        assert_eq!(censys.state, SecuritySourceState::NotScanned);

        // Every family is reported: none is silently treated as covered.
        for spec in SECURITY_SOURCE_SPECS {
            assert!(statuses.iter().any(|status| status.id == spec.id));
        }
    }

    #[test]
    fn builder_flags_missing_credentials_and_rate_limits() {
        let jobs = vec![job(
            "dark_web_scan",
            "failed",
            Some("HTTP 429 rate limit exceeded"),
            None,
        )];
        let statuses = build_security_source_statuses(
            &jobs,
            &[],
            |key| key != "DARKWEB_TOR_PROXY",
            &no_findings,
        );

        let i2p = statuses.iter().find(|s| s.id == "i2p").expect("i2p row");
        assert_eq!(
            i2p.state,
            SecuritySourceState::AuthenticationUnavailable,
            "missing Tor/I2P proxy is an authentication/capability gap"
        );

        let dark_web = statuses
            .iter()
            .find(|s| s.id == "dark_web")
            .expect("dark web row");
        assert_eq!(dark_web.state, SecuritySourceState::RateLimited);
    }

    #[test]
    fn builder_degrades_not_scanned_without_fabricating_findings() {
        let statuses = build_security_source_statuses(&[], &[], configured, &no_findings);
        for status in &statuses {
            assert!(status.findings.is_none());
            assert_eq!(status.state, SecuritySourceState::NotScanned);
            assert!(!status.detail.contains("0 findings"));
        }
    }
}
