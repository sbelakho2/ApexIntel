//! DNS Security Posture Analysis Module
//!
//! Provides DNS record checking for security posture assessment including:
//! - SPF (Sender Policy Framework) records
//! - DKIM (DomainKeys Identified Mail) records
//! - DMARC (Domain-based Message Authentication, Reporting & Conformance) records
//! - MX record analysis
//! - Lookalike domain detection

use anyhow::Result;
use apex_core::measurement::{FailureReason, Measurement};
use chrono::{DateTime, Utc};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::{ProtoError, ProtoErrorKind};
use hickory_resolver::TokioResolver;
use hickory_resolver::{ResolveError, ResolveErrorKind};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info};

/// DNS record types we check for security posture
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DnsRecordType {
    A,
    AAAA,
    MX,
    TXT,
    SPF,
    DKIM,
    DMARC,
    NS,
    CNAME,
}

/// Result of a DNS security check for a single domain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPostureResult {
    pub domain: String,
    pub checked_at: DateTime<Utc>,

    // SPF
    pub has_spf: bool,
    pub spf_record: Option<String>,
    pub spf_all_policy: Option<String>, // -all, ~all, ?all, +all

    // DKIM
    pub has_dkim: bool,
    pub dkim_selectors_found: Vec<String>,

    // DMARC
    pub has_dmarc: bool,
    pub dmarc_record: Option<String>,
    pub dmarc_policy: Option<String>, // none, quarantine, reject
    pub dmarc_pct: Option<u8>,

    // MX
    pub has_mx: bool,
    pub mx_records: Vec<String>,

    // Posture Score (0-100). `None` when any scored component lookup was
    // indeterminate: an unknown posture must never be reported as a low one.
    pub posture_score: Option<f32>,

    // Issues found
    pub issues: Vec<DnsSecurityIssue>,

    /// Lookups that failed or timed out. When non-empty, the corresponding
    /// "missing record" conclusions were NOT proven — absence is unknown.
    #[serde(default)]
    pub lookup_failures: Vec<String>,
}

/// A specific security issue found in DNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsSecurityIssue {
    pub severity: IssueSeverity,
    pub category: String,
    pub description: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// Lookalike domain detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookalikeDomain {
    pub original_domain: String,
    pub lookalike_domain: String,
    pub similarity_score: f32, // 0.0 - 1.0
    pub technique: LookalikeType,
    pub is_registered: bool,
    pub registrar: Option<String>,
    pub registration_date: Option<DateTime<Utc>>,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LookalikeType {
    Typosquatting,  // misspelling: gooogle.com
    Homograph,      // IDN/unicode: gооgle.com (cyrillic o)
    BitFlipping,    // bit errors: goohle.com
    Combosquatting, // additions: google-login.com
    SoundSquatting, // phonetic: googel.com
    LevelSquatting, // subdomain: google.com.evil.com
}

/// DNS resolver client using DNS-over-HTTPS for reliable cross-platform resolution
pub struct DnsChecker {
    client: Client,
    doh_endpoint: String,
}

impl Default for DnsChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl DnsChecker {
    /// Create a new DNS checker with default settings
    pub fn new() -> Self {
        Self::with_doh_endpoint("https://cloudflare-dns.com/dns-query")
    }

    /// Create a DNS checker with a custom DoH endpoint
    pub fn with_doh_endpoint(endpoint: &str) -> Self {
        let client = crate::http::external_client_or_panic(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(10),
            user_agent: Some("ApexIntel-DnsChecker/1.0".to_string()),
            ..crate::http::ExternalClientOptions::default()
        });

        Self {
            client,
            doh_endpoint: endpoint.to_string(),
        }
    }

    /// Check the DNS security posture for a domain.
    ///
    /// A failed lookup never becomes "record missing": indeterminate outcomes
    /// are recorded in [`DnsPostureResult::lookup_failures`], the related
    /// `has_*` flags stay false without a "missing record" issue, and the
    /// overall `posture_score` is `None` rather than a low score for unproven
    /// absence.
    pub async fn check_posture(&self, domain: &str) -> Result<DnsPostureResult> {
        info!(domain = %domain, "Checking DNS security posture");

        let mut issues = Vec::new();
        let mut lookup_failures = Vec::new();
        let checked_at = Utc::now();

        // Check TXT records for SPF
        let txt_outcome = self.query_txt(domain).await;
        let spf_indeterminate = Self::record_lookup_failure(
            &txt_outcome,
            "SPF",
            domain,
            &mut issues,
            &mut lookup_failures,
        );
        let (has_spf, spf_record, spf_all_policy) = if spf_indeterminate {
            (false, None, None)
        } else {
            self.analyze_spf(txt_records_or_empty(&txt_outcome), &mut issues)
        };

        // Check DMARC
        let dmarc_domain = format!("_dmarc.{}", domain);
        let dmarc_outcome = self.query_txt(&dmarc_domain).await;
        let dmarc_indeterminate = Self::record_lookup_failure(
            &dmarc_outcome,
            "DMARC",
            domain,
            &mut issues,
            &mut lookup_failures,
        );
        let (has_dmarc, dmarc_record, dmarc_policy, dmarc_pct) = if dmarc_indeterminate {
            (false, None, None, None)
        } else {
            self.analyze_dmarc(txt_records_or_empty(&dmarc_outcome), &mut issues)
        };

        // Check DKIM (common selectors)
        let (has_dkim, dkim_selectors_found, dkim_indeterminate) = self.check_dkim(domain).await;
        if dkim_indeterminate && !has_dkim {
            let detail = format!(
                "DKIM lookup for {domain} was indeterminate on at least one selector; absence not asserted"
            );
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::Low,
                category: "DKIM".to_string(),
                description: detail.clone(),
                recommendation: "Re-run the DNS posture check after resolving resolver connectivity"
                    .to_string(),
            });
            lookup_failures.push(detail);
        } else if !has_dkim {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::High,
                category: "DKIM".to_string(),
                description: "No DKIM records found for common selectors".to_string(),
                recommendation: "Configure DKIM signing for your email domain".to_string(),
            });
        }

        // Check MX records
        let mx_outcome = self.query_mx(domain).await;
        Self::record_lookup_failure(&mx_outcome, "MX", domain, &mut issues, &mut lookup_failures);
        let mx_records: Vec<String> = match &mx_outcome {
            DnsLookupOutcome::Records(records) => records.clone(),
            _ => Vec::new(),
        };
        // Only resolved MX records prove MX presence; an indeterminate lookup
        // is recorded in `lookup_failures` instead of guessing either way.
        let has_mx = !mx_records.is_empty();

        // Calculate posture score. If any scored component (SPF, DKIM, DMARC)
        // could not be resolved, the overall score is unknown rather than a
        // fabricated deduction for unproven absence.
        let posture_score = if posture_score_is_known(
            spf_indeterminate,
            dmarc_indeterminate,
            dkim_indeterminate,
            has_dkim,
        ) {
            Some(self.calculate_posture_score(
                has_spf,
                &spf_all_policy,
                has_dkim,
                has_dmarc,
                &dmarc_policy,
                dmarc_pct,
            ))
        } else {
            None
        };

        Ok(DnsPostureResult {
            domain: domain.to_string(),
            checked_at,
            has_spf,
            spf_record,
            spf_all_policy,
            has_dkim,
            dkim_selectors_found,
            has_dmarc,
            dmarc_record,
            dmarc_policy,
            dmarc_pct,
            has_mx,
            mx_records,
            posture_score,
            issues,
            lookup_failures,
        })
    }

    /// Record an indeterminate lookup as an explicit issue and failure entry.
    ///
    /// Returns `true` when the outcome failed to prove absence.
    fn record_lookup_failure(
        outcome: &DnsLookupOutcome,
        record_type: &str,
        domain: &str,
        issues: &mut Vec<DnsSecurityIssue>,
        lookup_failures: &mut Vec<String>,
    ) -> bool {
        let detail = match outcome {
            DnsLookupOutcome::Timeout => Some(format!(
                "{record_type} lookup for {domain} timed out; absence not asserted"
            )),
            DnsLookupOutcome::Failure(error) => Some(format!(
                "{record_type} lookup for {domain} failed: {error}; absence not asserted"
            )),
            _ => None,
        };

        let Some(detail) = detail else {
            return false;
        };

        issues.push(DnsSecurityIssue {
            severity: IssueSeverity::Low,
            category: "DNS".to_string(),
            description: detail.clone(),
            recommendation: "Re-run the DNS posture check after resolving resolver connectivity"
                .to_string(),
        });
        lookup_failures.push(detail);
        true
    }

    /// Generate lookalike domains for a given domain
    pub fn generate_lookalikes(&self, domain: &str) -> Vec<(String, LookalikeType)> {
        let mut lookalikes = Vec::new();

        // Extract base domain (without TLD)
        let parts: Vec<&str> = domain.split('.').collect();
        if parts.len() < 2 {
            return lookalikes;
        }
        let base = parts[0];
        let tld_parts = &parts[1..];
        let tld = tld_parts.join(".");

        // Typosquatting - character omissions
        for i in 0..base.len() {
            let mut typo = base.to_string();
            typo.remove(i);
            if !typo.is_empty() {
                lookalikes.push((format!("{}.{}", typo, tld), LookalikeType::Typosquatting));
            }
        }

        // Typosquatting - character swaps
        let chars: Vec<char> = base.chars().collect();
        for i in 0..chars.len().saturating_sub(1) {
            let mut swapped = chars.clone();
            swapped.swap(i, i + 1);
            let typo: String = swapped.into_iter().collect();
            if typo != base {
                lookalikes.push((format!("{}.{}", typo, tld), LookalikeType::Typosquatting));
            }
        }

        // Typosquatting - common keyboard adjacencies
        let keyboard_adjacent: HashMap<char, Vec<char>> = [
            ('a', vec!['s', 'q', 'z']),
            ('e', vec!['w', 'r', 'd']),
            ('i', vec!['u', 'o', 'k']),
            ('o', vec!['i', 'p', 'l']),
            ('u', vec!['y', 'i', 'j']),
        ]
        .into_iter()
        .collect();

        let base_chars: Vec<char> = base.chars().collect();

        for (i, c) in base_chars.iter().copied().enumerate() {
            if let Some(adjacent) = keyboard_adjacent.get(&c) {
                for &adj in adjacent {
                    let mut typo_chars = base_chars.clone();
                    typo_chars[i] = adj;
                    let typo: String = typo_chars.into_iter().collect();
                    lookalikes.push((format!("{}.{}", typo, tld), LookalikeType::Typosquatting));
                }
            }
        }

        // Homograph attacks - common substitutions
        let homographs: HashMap<char, Vec<char>> = [
            ('o', vec!['0', 'ο']), // zero, cyrillic
            ('l', vec!['1', 'і']), // one, cyrillic
            ('a', vec!['а']),      // cyrillic
            ('e', vec!['е']),      // cyrillic
            ('i', vec!['і', '1']), // cyrillic, one
        ]
        .into_iter()
        .collect();

        for (i, c) in base_chars.iter().copied().enumerate() {
            if let Some(subs) = homographs.get(&c) {
                for &sub in subs {
                    let mut homo_chars = base_chars.clone();
                    homo_chars[i] = sub;
                    let homo: String = homo_chars.into_iter().collect();
                    lookalikes.push((format!("{}.{}", homo, tld), LookalikeType::Homograph));
                }
            }
        }

        // Combosquatting - common prefixes/suffixes
        let prefixes = ["login-", "secure-", "account-", "mail-", "www-", "my-"];
        let suffixes = [
            "-login", "-secure", "-account", "-mail", "-portal", "-online",
        ];

        for prefix in prefixes {
            lookalikes.push((
                format!("{}{}.{}", prefix, base, tld),
                LookalikeType::Combosquatting,
            ));
        }
        for suffix in suffixes {
            lookalikes.push((
                format!("{}{}.{}", base, suffix, tld),
                LookalikeType::Combosquatting,
            ));
        }

        // Level squatting
        lookalikes.push((
            format!("{}.{}.com", domain, "login"),
            LookalikeType::LevelSquatting,
        ));
        lookalikes.push((
            format!("{}.{}.net", domain, "secure"),
            LookalikeType::LevelSquatting,
        ));

        // Deduplicate
        lookalikes.sort_by(|a, b| a.0.cmp(&b.0));
        lookalikes.dedup_by(|a, b| a.0 == b.0);

        lookalikes
    }

    /// Check whether a lookalike domain resolves at all.
    ///
    /// Returns a typed measurement: `Measured(true)` only when the name
    /// resolves, `Measured(false)` only when DNS definitively says the name or
    /// record does not exist, and `Unavailable` when the lookup failed — a
    /// failed lookup is never reported as "not registered".
    pub async fn check_lookalike_registration(&self, lookalike: &str) -> Measurement<bool> {
        match self.query_a(lookalike).await {
            DnsLookupOutcome::Records(records) => Measurement::measured(!records.is_empty()),
            DnsLookupOutcome::NxDomain | DnsLookupOutcome::NoRecords => {
                Measurement::measured(false)
            }
            DnsLookupOutcome::Timeout => Measurement::unavailable(FailureReason::new(
                "dns_timeout",
                format!("A lookup for {lookalike} timed out"),
            )),
            DnsLookupOutcome::Failure(error) => Measurement::unavailable(FailureReason::new(
                "dns_query_failed",
                format!("A lookup for {lookalike} failed: {error}"),
            )),
        }
    }

    // ─── Private Methods ────────────────────────────────────────────

    async fn query_txt(&self, domain: &str) -> DnsLookupOutcome {
        self.query_records(domain, "TXT").await
    }

    async fn query_mx(&self, domain: &str) -> DnsLookupOutcome {
        self.query_records(domain, "MX").await
    }

    async fn query_a(&self, domain: &str) -> DnsLookupOutcome {
        self.query_records(domain, "A").await
    }

    /// Structured DoH lookup. Every outcome — transport failure, non-success
    /// status, parse failure, NXDOMAIN, NODATA or records — is represented
    /// explicitly; none of them collapse into an empty record list.
    async fn query_records(&self, domain: &str, record_type: &str) -> DnsLookupOutcome {
        #[derive(Deserialize)]
        struct DohResponse {
            #[serde(rename = "Status")]
            status: Option<u32>,
            #[serde(rename = "Answer")]
            answer: Option<Vec<DohAnswer>>,
        }

        #[derive(Deserialize)]
        struct DohAnswer {
            data: String,
        }

        let context = format!("{domain} {record_type}");
        let url = format!("{}?name={}&type={}", self.doh_endpoint, domain, record_type);

        let resp = match self
            .client
            .get(&url)
            .header("Accept", "application/dns-json")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                debug!(domain = %domain, record_type = %record_type, %error, "DoH query failed");
                return DnsLookupOutcome::Failure(format!(
                    "DoH request failed for {context}: {error}"
                ));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            debug!(domain = %domain, record_type = %record_type, %status, "DoH query returned non-success");
            return DnsLookupOutcome::Failure(format!("DoH HTTP {status} for {context}"));
        }

        let doh: DohResponse = match crate::http::read_capped_json(
            resp,
            crate::http::MAX_EXTERNAL_BODY_BYTES,
        )
        .await
        {
            Ok(doh) => doh,
            Err(error) => {
                debug!(domain = %domain, record_type = %record_type, %error, "DoH response parse failed");
                return DnsLookupOutcome::Failure(format!(
                    "DoH response parse failed for {context}: {error}"
                ));
            }
        };

        let answers: Vec<String> = doh
            .answer
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.data.trim_matches('"').to_string())
            .collect();

        classify_doh_response(doh.status, answers, &context)
    }

    fn analyze_spf(
        &self,
        txt_records: &[String],
        issues: &mut Vec<DnsSecurityIssue>,
    ) -> (bool, Option<String>, Option<String>) {
        let spf_record = txt_records
            .iter()
            .find(|r| r.starts_with("v=spf1"))
            .cloned();

        let has_spf = spf_record.is_some();

        if !has_spf {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::High,
                category: "SPF".to_string(),
                description: "No SPF record found".to_string(),
                recommendation: "Add an SPF record to prevent email spoofing".to_string(),
            });
            return (false, None, None);
        }

        let Some(spf) = spf_record.as_ref() else {
            return (false, None, None);
        };
        let all_policy = if spf.contains("-all") {
            Some("-all".to_string())
        } else if spf.contains("~all") {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::Medium,
                category: "SPF".to_string(),
                description: "SPF uses soft fail (~all) instead of hard fail (-all)".to_string(),
                recommendation: "Consider using -all for stricter enforcement".to_string(),
            });
            Some("~all".to_string())
        } else if spf.contains("?all") {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::High,
                category: "SPF".to_string(),
                description: "SPF uses neutral policy (?all) which provides no protection"
                    .to_string(),
                recommendation: "Change to -all or ~all for email authentication".to_string(),
            });
            Some("?all".to_string())
        } else if spf.contains("+all") {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::Critical,
                category: "SPF".to_string(),
                description: "SPF uses +all which allows any sender - this is dangerous"
                    .to_string(),
                recommendation: "Immediately change to -all to prevent spoofing".to_string(),
            });
            Some("+all".to_string())
        } else {
            None
        };

        (has_spf, spf_record, all_policy)
    }

    fn analyze_dmarc(
        &self,
        dmarc_txt: &[String],
        issues: &mut Vec<DnsSecurityIssue>,
    ) -> (bool, Option<String>, Option<String>, Option<u8>) {
        let dmarc_record = dmarc_txt
            .iter()
            .find(|r| r.starts_with("v=DMARC1"))
            .cloned();

        let has_dmarc = dmarc_record.is_some();

        if !has_dmarc {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::High,
                category: "DMARC".to_string(),
                description: "No DMARC record found".to_string(),
                recommendation: "Add a DMARC record for email authentication reporting".to_string(),
            });
            return (false, None, None, None);
        }

        let Some(dmarc) = dmarc_record.as_ref() else {
            return (false, None, None, None);
        };

        // Parse policy
        let policy = if dmarc.contains("p=reject") {
            Some("reject".to_string())
        } else if dmarc.contains("p=quarantine") {
            Some("quarantine".to_string())
        } else if dmarc.contains("p=none") {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::Medium,
                category: "DMARC".to_string(),
                description:
                    "DMARC policy is set to 'none' - emails are monitored but not rejected"
                        .to_string(),
                recommendation: "Move to quarantine or reject policy after monitoring period"
                    .to_string(),
            });
            Some("none".to_string())
        } else {
            None
        };

        // Parse pct
        let pct = dmarc.split(';').find_map(|part| {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("pct=") {
                value.parse().ok()
            } else {
                None
            }
        });

        if let Some(p) = pct {
            if p < 100 {
                issues.push(DnsSecurityIssue {
                    severity: IssueSeverity::Low,
                    category: "DMARC".to_string(),
                    description: format!("DMARC only applies to {}% of messages", p),
                    recommendation: "Consider increasing pct to 100 for full coverage".to_string(),
                });
            }
        }

        (has_dmarc, dmarc_record, policy, pct)
    }

    async fn check_dkim(&self, domain: &str) -> (bool, Vec<String>, bool) {
        let mut found_selectors = Vec::new();
        let mut indeterminate = false;

        for &selector in COMMON_DKIM_SELECTORS {
            let dkim_domain = format!("{selector}._domainkey.{domain}");
            match self.query_txt(&dkim_domain).await {
                DnsLookupOutcome::Records(records) => {
                    if records.iter().any(|record| is_dkim_record(record)) {
                        found_selectors.push(selector.to_string());
                    }
                }
                DnsLookupOutcome::Timeout | DnsLookupOutcome::Failure(_) => {
                    indeterminate = true;
                }
                DnsLookupOutcome::NxDomain | DnsLookupOutcome::NoRecords => {}
            }
        }

        (!found_selectors.is_empty(), found_selectors, indeterminate)
    }

    fn calculate_posture_score(
        &self,
        has_spf: bool,
        spf_all_policy: &Option<String>,
        has_dkim: bool,
        has_dmarc: bool,
        dmarc_policy: &Option<String>,
        dmarc_pct: Option<u8>,
    ) -> f32 {
        let mut score: f32 = 0.0;

        // SPF: 30 points max
        if has_spf {
            score += 15.0;
            match spf_all_policy.as_deref() {
                Some("-all") => score += 15.0,
                Some("~all") => score += 10.0,
                Some("?all") => score += 5.0,
                Some("+all") => {} // Dangerous, no points
                _ => score += 5.0,
            }
        }

        // DKIM: 30 points max
        if has_dkim {
            score += 30.0;
        }

        // DMARC: 40 points max
        if has_dmarc {
            score += 15.0;
            match dmarc_policy.as_deref() {
                Some("reject") => score += 20.0,
                Some("quarantine") => score += 15.0,
                Some("none") => score += 5.0,
                _ => {}
            }
            // pct bonus
            if let Some(pct) = dmarc_pct {
                score += (pct as f32 / 100.0) * 5.0;
            } else {
                score += 5.0; // Assume 100% if not specified
            }
        }

        score.min(100.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Structured DNS resolution (hickory-resolver)
//
// Shelling out to `dig` conflated NXDOMAIN, empty answers, timeouts and
// process failures into one empty string, which made "no SPF/DKIM/DMARC" and
// "the resolver was broken" indistinguishable. These types keep the outcome
// explicit so callers can record definite absence differently from an unknown
// resolution state.
// ─────────────────────────────────────────────────────────────────────────────

/// Outcome of one structured DNS lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum DnsLookupOutcome {
    /// The name resolved and at least one record of the requested type exists.
    Records(Vec<String>),
    /// The name does not exist (`NXDOMAIN`) — a definitive absence.
    NxDomain,
    /// The name exists but has no record of the requested type (`NODATA`).
    NoRecords,
    /// The resolver timed out: absence is **unknown**, not proven.
    Timeout,
    /// Transport/resolver failure: absence is **unknown**, not proven.
    Failure(String),
}

impl DnsLookupOutcome {
    /// Records when resolution succeeded with at least one answer.
    pub fn records(&self) -> Option<&[String]> {
        match self {
            Self::Records(records) => Some(records),
            _ => None,
        }
    }

    /// True when the outcome is a definitive "no such record" (NXDOMAIN or
    /// NODATA) — the only case in which a missing DNS record may be asserted.
    pub fn is_definitive_absence(&self) -> bool {
        matches!(self, Self::NxDomain | Self::NoRecords)
    }

    /// True when resolution failed without proving absence (timeout or
    /// transport failure).
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Self::Timeout | Self::Failure(_))
    }

    /// Short machine-readable label for metrics/logs.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Records(_) => "records",
            Self::NxDomain => "nxdomain",
            Self::NoRecords => "no_records",
            Self::Timeout => "timeout",
            Self::Failure(_) => "failure",
        }
    }
}

/// Tri-state DKIM observation.
///
/// A boolean could not distinguish "no DKIM on the selectors we know" from
/// "we could not determine this because DNS failed". Warning generation uses
/// the distinction so an indeterminate lookup never becomes a missing-DKIM
/// finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum DkimStatus {
    /// At least one known selector returned a usable DKIM public key record.
    ConfirmedPresent { selectors: Vec<String> },
    /// Every known selector definitively resolved without a DKIM record.
    NotObservedOnKnownSelectors,
    /// At least one selector lookup failed; DKIM state cannot be asserted.
    Unknown { reason: String },
}

impl DkimStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConfirmedPresent { .. } => "confirmed_present",
            Self::NotObservedOnKnownSelectors => "not_observed_on_known_selectors",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::ConfirmedPresent { .. })
    }

    /// Only a definitive all-selectors miss may be reported as a DKIM gap.
    pub fn is_confirmed_absent(&self) -> bool {
        matches!(self, Self::NotObservedOnKnownSelectors)
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }
}

/// Common DKIM selectors probed when no provider-specific selector is known.
pub const COMMON_DKIM_SELECTORS: &[&str] = &[
    "default",
    "selector1",
    "selector2",
    "google",
    "k1",
    "s1",
    "s2",
    "mail",
    "email",
    "dkim",
    "smtp",
];

/// Test whether a TXT record is a usable DKIM key record.
pub fn is_dkim_record(record: &str) -> bool {
    let lower = record.to_lowercase();
    lower.contains("v=dkim1") || lower.contains("p=")
}

/// Collapse per-selector lookup outcomes into a [`DkimStatus`].
///
/// * any selector with a DKIM record → `ConfirmedPresent`
/// * no DKIM record, every lookup definitive → `NotObservedOnKnownSelectors`
/// * any timeout/failure → `Unknown` (never a false "missing" finding)
pub fn classify_dkim_outcomes(
    outcomes: impl IntoIterator<Item = (String, DnsLookupOutcome)>,
) -> DkimStatus {
    let mut found_selectors = Vec::new();
    let mut had_indeterminate = false;
    let mut indeterminate_reason: Option<String> = None;

    for (selector, outcome) in outcomes {
        match outcome {
            DnsLookupOutcome::Records(records) => {
                if records.iter().any(|record| is_dkim_record(record)) {
                    found_selectors.push(selector);
                }
            }
            DnsLookupOutcome::Timeout => {
                had_indeterminate = true;
                indeterminate_reason
                    .get_or_insert_with(|| format!("selector '{selector}' timed out"));
            }
            DnsLookupOutcome::Failure(error) => {
                had_indeterminate = true;
                indeterminate_reason
                    .get_or_insert_with(|| format!("selector '{selector}' lookup failed: {error}"));
            }
            DnsLookupOutcome::NxDomain | DnsLookupOutcome::NoRecords => {}
        }
    }

    if !found_selectors.is_empty() {
        found_selectors.sort();
        return DkimStatus::ConfirmedPresent {
            selectors: found_selectors,
        };
    }
    match indeterminate_reason {
        Some(reason) if had_indeterminate => DkimStatus::Unknown { reason },
        _ => DkimStatus::NotObservedOnKnownSelectors,
    }
}

/// Extract the SPF record from TXT answers, if present.
pub fn extract_spf_record(records: &[String]) -> Option<String> {
    records
        .iter()
        .find(|record| record.to_lowercase().contains("v=spf1"))
        .cloned()
}

/// Extract the DMARC record and its `p=` policy from TXT answers.
pub fn extract_dmarc_record(records: &[String]) -> Option<(String, Option<String>)> {
    let record = records
        .iter()
        .find(|record| record.to_lowercase().contains("v=dmarc1"))?
        .clone();
    let policy = record
        .to_lowercase()
        .split(';')
        .find_map(|part| part.trim().strip_prefix("p=").map(|p| p.trim().to_string()));
    Some((record, policy))
}

/// Whether the posture score can be asserted.
///
/// SPF and DMARC contribute to the score whenever they are not confirmed, so
/// any indeterminate lookup there makes the total unknown. A DKIM selector
/// failure only matters when no selector confirmed DKIM: a confirmed presence
/// keeps the DKIM component known.
fn posture_score_is_known(
    spf_indeterminate: bool,
    dmarc_indeterminate: bool,
    dkim_indeterminate: bool,
    has_dkim: bool,
) -> bool {
    !(spf_indeterminate || dmarc_indeterminate || (dkim_indeterminate && !has_dkim))
}

/// Borrow records from an outcome, or an empty slice for definitive absence.
///
/// Callers must only use this for outcomes where absence is proven
/// (`NxDomain`/`NoRecords`); an indeterminate outcome has no records and must
/// be handled as unknown before reaching record analysis.
fn txt_records_or_empty(outcome: &DnsLookupOutcome) -> &[String] {
    match outcome {
        DnsLookupOutcome::Records(records) => records,
        _ => &[],
    }
}

/// Classify a DNS-over-HTTPS JSON answer into a structured outcome.
///
/// `Status` 3 (NXDOMAIN) is definitive absence; any other non-zero status is a
/// query failure, never an absence; an empty answer with status 0 is NODATA.
/// A response with neither status nor answers is malformed and therefore a
/// failure, not evidence of absence.
pub fn classify_doh_response(
    status: Option<u32>,
    answers: Vec<String>,
    context: &str,
) -> DnsLookupOutcome {
    match status {
        Some(3) => DnsLookupOutcome::NxDomain,
        Some(code) if code != 0 => {
            DnsLookupOutcome::Failure(format!("DoH response status {code} for {context}"))
        }
        None if answers.is_empty() => DnsLookupOutcome::Failure(format!(
            "DoH response missing both Status and Answer for {context}"
        )),
        _ => {
            if answers.is_empty() {
                DnsLookupOutcome::NoRecords
            } else {
                DnsLookupOutcome::Records(answers)
            }
        }
    }
}

/// Structured resolver built on hickory-resolver (system configuration).
///
/// Construction never panics: when the system resolver configuration cannot
/// be loaded the resolver reports [`DnsLookupOutcome::Failure`] for every
/// lookup so callers can degrade instead of asserting DNS facts.
pub struct StructuredDnsResolver {
    resolver: Option<TokioResolver>,
}

impl Default for StructuredDnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl StructuredDnsResolver {
    pub fn new() -> Self {
        match TokioResolver::builder_tokio() {
            Ok(builder) => Self {
                resolver: Some(builder.build()),
            },
            Err(error) => {
                tracing::warn!(
                    %error,
                    "StructuredDnsResolver: system resolver config unavailable; DNS lookups will report failure"
                );
                Self { resolver: None }
            }
        }
    }

    pub fn is_available(&self) -> bool {
        self.resolver.is_some()
    }

    /// Look up TXT records with structured error semantics.
    pub async fn txt(&self, domain: &str) -> DnsLookupOutcome {
        let Some(resolver) = self.resolver.as_ref() else {
            return DnsLookupOutcome::Failure(
                "structured DNS resolver unavailable (no system resolver configuration)".into(),
            );
        };
        match resolver.txt_lookup(domain).await {
            Ok(lookup) => {
                let records: Vec<String> = lookup
                    .iter()
                    .map(|txt| {
                        txt.txt_data()
                            .iter()
                            .map(|chunk| String::from_utf8_lossy(chunk).to_string())
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .collect();
                if records.is_empty() {
                    DnsLookupOutcome::NoRecords
                } else {
                    DnsLookupOutcome::Records(records)
                }
            }
            Err(error) => classify_resolve_error(&error),
        }
    }

    /// Look up MX records with structured error semantics.
    pub async fn mx(&self, domain: &str) -> DnsLookupOutcome {
        let Some(resolver) = self.resolver.as_ref() else {
            return DnsLookupOutcome::Failure(
                "structured DNS resolver unavailable (no system resolver configuration)".into(),
            );
        };
        match resolver.mx_lookup(domain).await {
            Ok(lookup) => {
                let records: Vec<String> = lookup
                    .iter()
                    .map(|mx| format!("{} {}", mx.preference(), mx.exchange()))
                    .collect();
                if records.is_empty() {
                    DnsLookupOutcome::NoRecords
                } else {
                    DnsLookupOutcome::Records(records)
                }
            }
            Err(error) => classify_resolve_error(&error),
        }
    }

    /// Resolve DKIM state across the common selectors as a tri-state value.
    pub async fn dkim_status(&self, domain: &str) -> DkimStatus {
        self.dkim_status_for_selectors(domain, COMMON_DKIM_SELECTORS)
            .await
    }

    /// Resolve DKIM state across an explicit selector list.
    pub async fn dkim_status_for_selectors(&self, domain: &str, selectors: &[&str]) -> DkimStatus {
        let mut outcomes = Vec::with_capacity(selectors.len());
        for selector in selectors {
            let dkim_domain = format!("{selector}._domainkey.{domain}");
            let outcome = self.txt(&dkim_domain).await;
            outcomes.push(((*selector).to_string(), outcome));
        }
        classify_dkim_outcomes(outcomes)
    }
}

/// Map a hickory resolution error to the structured outcome.
///
/// `NXDOMAIN` and `NODATA` are definitive absence; timeouts and transport
/// errors are indeterminate.
pub fn classify_resolve_error(error: &ResolveError) -> DnsLookupOutcome {
    if let ResolveErrorKind::Proto(proto) = error.kind() {
        return classify_proto_error(proto);
    }
    DnsLookupOutcome::Failure(error.to_string())
}

fn classify_proto_error(proto: &ProtoError) -> DnsLookupOutcome {
    match proto.kind() {
        ProtoErrorKind::NoRecordsFound { response_code, .. } => {
            if *response_code == ResponseCode::NXDomain {
                DnsLookupOutcome::NxDomain
            } else {
                DnsLookupOutcome::NoRecords
            }
        }
        ProtoErrorKind::Timeout => DnsLookupOutcome::Timeout,
        other => DnsLookupOutcome::Failure(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_generate_lookalikes() {
        let checker = DnsChecker::new();
        let lookalikes = checker.generate_lookalikes("example.com");

        assert!(!lookalikes.is_empty());

        // Should include typosquatting
        assert!(lookalikes
            .iter()
            .any(|(_, t)| *t == LookalikeType::Typosquatting));

        // Should include combosquatting
        assert!(lookalikes
            .iter()
            .any(|(d, t)| *t == LookalikeType::Combosquatting && d.contains("-login")));
    }

    #[test]
    fn test_posture_score_calculation() {
        let checker = DnsChecker::new();

        // Perfect score
        let score = checker.calculate_posture_score(
            true,
            &Some("-all".to_string()),
            true,
            true,
            &Some("reject".to_string()),
            Some(100),
        );
        assert!((score - 100.0).abs() < 0.01);

        // No protection
        let score = checker.calculate_posture_score(false, &None, false, false, &None, None);
        assert!(score < 1.0);
    }

    #[test]
    fn test_extract_spf_record() {
        let records = vec![
            "google-site-verification=abc".to_string(),
            "v=spf1 include:_spf.example.com -all".to_string(),
        ];
        assert_eq!(
            extract_spf_record(&records).as_deref(),
            Some("v=spf1 include:_spf.example.com -all")
        );
        assert!(extract_spf_record(&[]).is_none());
    }

    #[test]
    fn test_extract_dmarc_record_and_policy() {
        let records = vec!["v=DMARC1; p=quarantine; rua=mailto:dmarc@example.com".to_string()];
        let (record, policy) = extract_dmarc_record(&records).expect("dmarc record");
        assert!(record.starts_with("v=DMARC1"));
        assert_eq!(policy.as_deref(), Some("quarantine"));

        assert!(extract_dmarc_record(&["v=spf1 -all".to_string()]).is_none());
    }

    #[test]
    fn test_doh_query_failure_is_not_no_record() {
        let failed = classify_doh_response(Some(2), Vec::new(), "example.com TXT");
        assert!(matches!(failed, DnsLookupOutcome::Failure(_)));
        assert_ne!(failed, DnsLookupOutcome::NoRecords);
        assert!(!failed.is_definitive_absence());
        assert!(failed.is_indeterminate());

        let nxdomain = classify_doh_response(Some(3), Vec::new(), "example.com TXT");
        assert_eq!(nxdomain, DnsLookupOutcome::NxDomain);
        assert!(nxdomain.is_definitive_absence());

        let nodata = classify_doh_response(Some(0), Vec::new(), "example.com TXT");
        assert_eq!(nodata, DnsLookupOutcome::NoRecords);
        assert!(nodata.is_definitive_absence());

        let records =
            classify_doh_response(Some(0), vec!["v=spf1 -all".to_string()], "example.com TXT");
        assert!(matches!(records, DnsLookupOutcome::Records(_)));

        // A malformed response is a failure, never a definitive absence.
        let malformed = classify_doh_response(None, Vec::new(), "example.com TXT");
        assert!(matches!(malformed, DnsLookupOutcome::Failure(_)));
        assert!(!malformed.is_definitive_absence());
    }

    #[test]
    fn test_indeterminate_lookup_is_not_missing_record_evidence() {
        assert_ne!(
            DnsLookupOutcome::Failure("connection refused".to_string()),
            DnsLookupOutcome::NxDomain
        );
        assert_ne!(DnsLookupOutcome::Timeout, DnsLookupOutcome::NoRecords);
        assert!(!DnsLookupOutcome::Timeout.is_definitive_absence());
        assert!(DnsLookupOutcome::Timeout.is_indeterminate());
    }

    #[test]
    fn test_record_lookup_failure_marks_absence_unproven() {
        let mut issues = Vec::new();
        let mut failures = Vec::new();
        let indeterminate = DnsChecker::record_lookup_failure(
            &DnsLookupOutcome::Failure("resolver down".to_string()),
            "SPF",
            "example.com",
            &mut issues,
            &mut failures,
        );
        assert!(indeterminate);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("absence not asserted"));
        assert!(issues.iter().any(|issue| issue.category == "DNS"));
        assert!(!issues
            .iter()
            .any(|issue| issue.description.contains("No SPF record found")));

        let mut issues = Vec::new();
        let mut failures = Vec::new();
        let definitive = DnsChecker::record_lookup_failure(
            &DnsLookupOutcome::NxDomain,
            "SPF",
            "example.com",
            &mut issues,
            &mut failures,
        );
        assert!(!definitive);
        assert!(failures.is_empty());
        assert!(issues.is_empty());
    }

    #[test]
    fn test_lookalike_lookup_failure_is_not_unregistered() {
        let failure = DnsLookupOutcome::Failure("connection refused".to_string());
        assert_ne!(failure, DnsLookupOutcome::NoRecords);
        assert!(!failure.is_definitive_absence());
    }

    #[test]
    fn test_posture_score_is_unknown_when_a_scored_component_failed() {
        // Indeterminate SPF/DMARC always invalidates the total.
        assert!(!posture_score_is_known(true, false, false, false));
        assert!(!posture_score_is_known(false, true, false, false));
        // Indeterminate DKIM matters only when DKIM was not confirmed.
        assert!(!posture_score_is_known(false, false, true, false));
        assert!(posture_score_is_known(false, false, true, true));
        // All components resolved: the score is assertable.
        assert!(posture_score_is_known(false, false, false, false));
    }

    #[test]
    fn test_dkim_tristate_confirmed_present() {
        let status = classify_dkim_outcomes(vec![
            (
                "default".to_string(),
                DnsLookupOutcome::Records(vec!["v=DKIM1; k=rsa; p=abc".to_string()]),
            ),
            ("google".to_string(), DnsLookupOutcome::NxDomain),
        ]);
        assert!(status.is_confirmed());
        assert_eq!(status.as_str(), "confirmed_present");
        match status {
            DkimStatus::ConfirmedPresent { selectors } => assert_eq!(selectors, vec!["default"]),
            other => panic!("expected ConfirmedPresent, got {other:?}"),
        }
    }

    #[test]
    fn test_dkim_tristate_not_observed_requires_definitive_absence() {
        let status = classify_dkim_outcomes(vec![
            ("default".to_string(), DnsLookupOutcome::NxDomain),
            ("google".to_string(), DnsLookupOutcome::NoRecords),
            (
                "k1".to_string(),
                DnsLookupOutcome::Records(vec!["some other txt".to_string()]),
            ),
        ]);
        assert!(status.is_confirmed_absent());
        assert!(!status.is_confirmed());
        assert_eq!(status.as_str(), "not_observed_on_known_selectors");
    }

    #[test]
    fn test_dkim_tristate_unknown_on_timeout_or_failure() {
        let status = classify_dkim_outcomes(vec![
            ("default".to_string(), DnsLookupOutcome::NxDomain),
            ("google".to_string(), DnsLookupOutcome::Timeout),
        ]);
        assert!(status.is_unknown());
        assert!(!status.is_confirmed_absent());
        assert_eq!(status.as_str(), "unknown");

        let status = classify_dkim_outcomes(vec![
            ("default".to_string(), DnsLookupOutcome::NxDomain),
            (
                "google".to_string(),
                DnsLookupOutcome::Failure("connection refused".to_string()),
            ),
        ]);
        assert!(status.is_unknown());
        match status {
            DkimStatus::Unknown { reason } => assert!(reason.contains("google")),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn test_dkim_status_serde_roundtrip() {
        for status in [
            DkimStatus::ConfirmedPresent {
                selectors: vec!["default".to_string()],
            },
            DkimStatus::NotObservedOnKnownSelectors,
            DkimStatus::Unknown {
                reason: "timeout".to_string(),
            },
        ] {
            let json = serde_json::to_string(&status).expect("serialize dkim status");
            let decoded: DkimStatus = serde_json::from_str(&json).expect("deserialize dkim status");
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn test_lookup_outcome_absence_semantics() {
        assert!(DnsLookupOutcome::NxDomain.is_definitive_absence());
        assert!(DnsLookupOutcome::NoRecords.is_definitive_absence());
        assert!(!DnsLookupOutcome::Timeout.is_definitive_absence());
        assert!(DnsLookupOutcome::Timeout.is_indeterminate());
        assert!(DnsLookupOutcome::Failure("boom".into()).is_indeterminate());
        assert!(!DnsLookupOutcome::Records(vec!["x".into()]).is_definitive_absence());
        assert_eq!(
            DnsLookupOutcome::Records(vec!["x".into()]).records(),
            Some(["x".to_string()].as_slice())
        );
    }
}
