//! DNS Security Posture Analysis Module
//!
//! Provides DNS record checking for security posture assessment including:
//! - SPF (Sender Policy Framework) records
//! - DKIM (DomainKeys Identified Mail) records
//! - DMARC (Domain-based Message Authentication, Reporting & Conformance) records
//! - MX record analysis
//! - Lookalike domain detection

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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

    // Posture Score (0-100)
    pub posture_score: f32,

    // Issues found
    pub issues: Vec<DnsSecurityIssue>,
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
    common_dkim_selectors: Vec<String>,
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
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("ApexIntel-DnsChecker/1.0")
            .build()
            .unwrap_or_else(|error| panic!("failed to build DNS checker HTTP client: {error}"));

        Self {
            client,
            doh_endpoint: endpoint.to_string(),
            common_dkim_selectors: vec![
                "default".to_string(),
                "selector1".to_string(),
                "selector2".to_string(),
                "google".to_string(),
                "k1".to_string(),
                "s1".to_string(),
                "s2".to_string(),
                "mail".to_string(),
                "email".to_string(),
                "dkim".to_string(),
                "smtp".to_string(),
            ],
        }
    }

    /// Check the DNS security posture for a domain
    pub async fn check_posture(&self, domain: &str) -> Result<DnsPostureResult> {
        info!(domain = %domain, "Checking DNS security posture");

        let mut issues = Vec::new();
        let checked_at = Utc::now();

        // Check TXT records for SPF
        let txt_records = self.query_txt(domain).await.unwrap_or_default();
        let (has_spf, spf_record, spf_all_policy) = self.analyze_spf(&txt_records, &mut issues);

        // Check DMARC
        let dmarc_domain = format!("_dmarc.{}", domain);
        let dmarc_txt = self.query_txt(&dmarc_domain).await.unwrap_or_default();
        let (has_dmarc, dmarc_record, dmarc_policy, dmarc_pct) =
            self.analyze_dmarc(&dmarc_txt, &mut issues);

        // Check DKIM (common selectors)
        let (has_dkim, dkim_selectors_found) = self.check_dkim(domain).await;
        if !has_dkim {
            issues.push(DnsSecurityIssue {
                severity: IssueSeverity::High,
                category: "DKIM".to_string(),
                description: "No DKIM records found for common selectors".to_string(),
                recommendation: "Configure DKIM signing for your email domain".to_string(),
            });
        }

        // Check MX records
        let mx_records = self.query_mx(domain).await.unwrap_or_default();
        let has_mx = !mx_records.is_empty();

        // Calculate posture score
        let posture_score = self.calculate_posture_score(
            has_spf,
            &spf_all_policy,
            has_dkim,
            has_dmarc,
            &dmarc_policy,
            dmarc_pct,
        );

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
        })
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

    /// Check if a lookalike domain is registered
    pub async fn check_lookalike_registration(&self, lookalike: &str) -> Result<bool> {
        // Try to resolve A record
        let records = self.query_a(lookalike).await.unwrap_or_default();
        Ok(!records.is_empty())
    }

    // ─── Private Methods ────────────────────────────────────────────

    async fn query_txt(&self, domain: &str) -> Result<Vec<String>> {
        self.query_records(domain, "TXT").await
    }

    async fn query_mx(&self, domain: &str) -> Result<Vec<String>> {
        self.query_records(domain, "MX").await
    }

    async fn query_a(&self, domain: &str) -> Result<Vec<String>> {
        self.query_records(domain, "A").await
    }

    async fn query_records(&self, domain: &str, record_type: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct DohResponse {
            #[serde(rename = "Answer")]
            answer: Option<Vec<DohAnswer>>,
        }

        #[derive(Deserialize)]
        struct DohAnswer {
            data: String,
        }

        let url = format!("{}?name={}&type={}", self.doh_endpoint, domain, record_type);

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/dns-json")
            .send()
            .await
            .context("DoH request failed")?;

        if !resp.status().is_success() {
            debug!(domain = %domain, record_type = %record_type, status = %resp.status(), "DoH query returned non-success");
            return Ok(Vec::new());
        }

        let doh: DohResponse = resp.json().await.context("Failed to parse DoH response")?;

        Ok(doh
            .answer
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.data.trim_matches('"').to_string())
            .collect())
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

    async fn check_dkim(&self, domain: &str) -> (bool, Vec<String>) {
        let mut found_selectors = Vec::new();

        for selector in &self.common_dkim_selectors {
            let dkim_domain = format!("{}._domainkey.{}", selector, domain);
            if let Ok(records) = self.query_txt(&dkim_domain).await {
                if records
                    .iter()
                    .any(|r| r.contains("v=DKIM1") || r.contains("k=rsa"))
                {
                    found_selectors.push(selector.clone());
                }
            }
        }

        (!found_selectors.is_empty(), found_selectors)
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

#[cfg(test)]
mod tests {
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
}
