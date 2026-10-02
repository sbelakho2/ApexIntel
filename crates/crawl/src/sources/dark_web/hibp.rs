//! HaveIBeenPwned API Integration Module
//!
//! Integrates with the HaveIBeenPwned API for breach and paste monitoring:
//! - Account breach checking (by email)
//! - Password searching (for red team purposes)
//! - Paste monitoring (new paste alerts)
//! - Breach aggregation and reporting
//!
//! Requires a HIBP API key (k-Anonymous API available without key for breach checking).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::time::Duration;
use tracing::debug;

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

/// API mode for HIBP queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HibpApiMode {
    /// k-Anonymous API (no key required) — SHA-1 hash prefix search.
    KAnonymous,
    /// Full API (requires key) — direct email lookup.
    FullApi,
}

/// A HIBP breach record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibpBreach {
    pub name: String,
    pub title: String,
    pub domain: String,
    pub breach_date: DateTime<Utc>,
    pub added_date: DateTime<Utc>,
    pub modified_date: DateTime<Utc>,
    pub pwn_count: u64,
    pub description: String,
    pub data_classes: Vec<String>,
    pub is_verified: bool,
    pub is_fabricated: bool,
    pub is_sensitive: bool,
    pub is_retired: bool,
    pub is_spam_list: bool,
    pub logo_path: Option<String>,
}

impl HibpBreach {
    /// Whether this breach contains password data.
    pub fn has_passwords(&self) -> bool {
        self.data_classes
            .iter()
            .any(|c| c.to_lowercase().contains("password"))
    }

    /// Whether this breach contains email addresses.
    pub fn has_emails(&self) -> bool {
        self.data_classes
            .iter()
            .any(|c| c.to_lowercase().contains("email"))
    }

    /// Severity score based on data types and pwn count.
    pub fn severity_score(&self) -> f32 {
        let mut score = 0.0;
        for dc in &self.data_classes {
            let lower = dc.to_lowercase();
            score += if lower.contains("password") {
                5.0
            } else if lower.contains("credit card")
                || lower.contains("bank")
                || lower.contains("ssn")
                || lower.contains("national id")
            {
                4.0
            } else if lower.contains("email") || lower.contains("phone") {
                2.0
            } else {
                1.0
            };
        }
        score * (1.0 + (self.pwn_count as f32 / 1_000_000.0).min(5.0))
    }
}

/// A HIBP paste record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibpPaste {
    pub source: String,
    pub id: String,
    pub title: Option<String>,
    pub date: DateTime<Utc>,
    pub email_count: u64,
    pub url: Option<String>,
}

impl HibpPaste {
    /// Whether this paste is from Pastebin.
    pub fn is_pastebin(&self) -> bool {
        self.source == "Pastebin"
    }
}

/// HIBP monitoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibpMonitorConfig {
    /// API mode to use.
    pub api_mode: HibpApiMode,
    /// HIBP API key (for full API access).
    pub api_key: Option<String>,
    /// Rate limit (requests per second).
    pub rate_limit: u32,
    /// Whether to include spam lists.
    pub include_spam_lists: bool,
    /// Timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for HibpMonitorConfig {
    fn default() -> Self {
        Self {
            api_mode: HibpApiMode::KAnonymous,
            api_key: None,
            rate_limit: 2,
            include_spam_lists: false,
            timeout_secs: 30,
        }
    }
}

impl HibpMonitorConfig {
    /// Set the API key.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self.api_mode = HibpApiMode::FullApi;
        self
    }
}

/// HaveIBeenPwned monitor.
#[derive(Debug, Clone)]
pub struct HibpMonitor {
    client: Client,
    config: HibpMonitorConfig,
}

impl HibpMonitor {
    /// Create with configuration.
    pub fn new(config: HibpMonitorConfig) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(config.timeout_secs),
            user_agent: Some("ApexIntel/1.0 (+https://apexintel.io)".to_string()),
            ..crate::http::ExternalClientOptions::default()
        })
        .context("building HIBP HTTP client")?;
        Ok(Self { client, config })
    }

    /// Check if an email has been in any breach (k-Anonymous API).
    ///
    /// A 404 means the hash prefix is not in the corpus — a definitive
    /// "no breach" parsed success. A malformed body that contains no
    /// `HASH:COUNT` line is a parse failure, not an empty result.
    pub async fn check_breaches(&self, email: &str) -> ParseOutcome<HibpBreach> {
        // Compute SHA-1 hash of email
        let mut hasher = Sha1::new();
        hasher.update(email.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let prefix = &hash[..5];
        let suffix = &hash[5..];

        let url = format!(
            "https://api.pwnedpasswords.com/range/{}",
            urlencoding::encode(prefix)
        );

        let resp = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("HIBP k-Anonymous API request failed: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            if status == 404 {
                let outcome = ParseOutcome::parsed(Vec::new());
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
            let outcome = ParseOutcome::fetch_failed(
                format!("HIBP API returned HTTP {status}"),
                Some(status),
            );
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read HIBP response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let mut parsed_lines = 0usize;
        let mut breaches = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((line_hash, count_raw)) = line.split_once(':') else {
                continue;
            };
            // Best-effort telemetry: a single malformed line is skipped, but
            // if *no* line in the body parses as HASH:COUNT the schema changed.
            let Ok(count) = count_raw.trim().parse::<u64>() else {
                continue;
            };
            parsed_lines += 1;
            if line_hash.to_uppercase() == suffix.to_uppercase() && count > 0 {
                debug!(email = %email, count = count, "HIBP breach found via k-Anonymous API");
                breaches.push(HibpBreach {
                    name: "PwnedPassword".to_string(),
                    title: "Password found in breach".to_string(),
                    domain: "pwnedpasswords.com".to_string(),
                    breach_date: Utc::now(),
                    added_date: Utc::now(),
                    modified_date: Utc::now(),
                    pwn_count: count,
                    description: format!(
                        "The password associated with this account was found in {} data breaches.",
                        count
                    ),
                    data_classes: vec!["Passwords".to_string()],
                    is_verified: false,
                    is_fabricated: false,
                    is_sensitive: true,
                    is_retired: false,
                    is_spam_list: false,
                    logo_path: None,
                });
            }
        }

        if parsed_lines == 0 && !text.trim().is_empty() {
            let outcome =
                ParseOutcome::parse_failed("HIBP range response had no HASH:COUNT lines", &text);
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let outcome = ParseOutcome::parsed(breaches);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Get all breaches (full API).
    pub async fn all_breaches(&self) -> ParseOutcome<HibpBreach> {
        let Some(api_key) = self.config.api_key.as_ref() else {
            let outcome = ParseOutcome::fetch_failed("HIBP full API requires an API key", None);
            PARSER_METRICS.record(&outcome);
            return outcome;
        };

        let url = "https://haveibeenpwned.com/api/v3/breaches";
        let resp = match self
            .client
            .get(url)
            .header("hibp-api-key", api_key)
            .header("user-agent", "ApexIntel/1.0")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("HIBP all-breaches request failed: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            debug!(status = %resp.status(), "HIBP all breaches returned non-success");
            let outcome = ParseOutcome::fetch_failed(
                format!("HIBP all-breaches returned HTTP {status}"),
                Some(status),
            );
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        #[serde(rename_all = "PascalCase")]
        struct HibpApiBreach {
            name: String,
            title: String,
            domain: String,
            breach_date: String,
            added_date: String,
            pwn_count: u64,
            description: String,
            data_classes: Vec<String>,
            is_verified: bool,
            is_fabricated: bool,
            is_sensitive: bool,
            is_retired: bool,
            is_spam_list: bool,
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read HIBP all-breaches response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let breaches: Vec<HibpApiBreach> = match serde_json::from_str(&text) {
            Ok(breaches) => breaches,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse HIBP all-breaches JSON: {error}"),
                    &text,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        // Date parsing failure is a schema change, not a reason to silently
        // substitute "now" and keep a fabricated breach timestamp.
        let mut mapped = Vec::with_capacity(breaches.len());
        for breach in breaches {
            let breach_date = match chrono::DateTime::parse_from_rfc3339(&breach.breach_date) {
                Ok(date) => date.with_timezone(&Utc),
                Err(error) => {
                    let outcome = ParseOutcome::parse_failed(
                        format!(
                            "failed to parse HIBP breach_date '{}': {error}",
                            breach.breach_date
                        ),
                        &text,
                    );
                    PARSER_METRICS.record(&outcome);
                    return outcome;
                }
            };
            let added_date = match chrono::DateTime::parse_from_rfc3339(&breach.added_date) {
                Ok(date) => date.with_timezone(&Utc),
                Err(error) => {
                    let outcome = ParseOutcome::parse_failed(
                        format!(
                            "failed to parse HIBP added_date '{}': {error}",
                            breach.added_date
                        ),
                        &text,
                    );
                    PARSER_METRICS.record(&outcome);
                    return outcome;
                }
            };
            mapped.push(HibpBreach {
                name: breach.name,
                title: breach.title,
                domain: breach.domain,
                breach_date,
                added_date,
                modified_date: Utc::now(),
                pwn_count: breach.pwn_count,
                description: breach.description,
                data_classes: breach.data_classes,
                is_verified: breach.is_verified,
                is_fabricated: breach.is_fabricated,
                is_sensitive: breach.is_sensitive,
                is_retired: breach.is_retired,
                is_spam_list: breach.is_spam_list,
                logo_path: None,
            });
        }

        let outcome = ParseOutcome::parsed(mapped);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Check for pastes associated with an email (full API).
    ///
    /// A 404 means no pastes (parsed success, empty); other non-success
    /// statuses are fetch failures instead of a fake "no pastes".
    pub async fn check_pastes(&self, email: &str) -> ParseOutcome<HibpPaste> {
        let Some(api_key) = self.config.api_key.as_ref() else {
            let outcome = ParseOutcome::fetch_failed("HIBP paste check requires an API key", None);
            PARSER_METRICS.record(&outcome);
            return outcome;
        };

        let url = format!(
            "https://haveibeenpwned.com/api/v3/pasteaccount/{}",
            urlencoding::encode(email)
        );
        let resp = match self
            .client
            .get(&url)
            .header("hibp-api-key", api_key)
            .header("user-agent", "ApexIntel/1.0")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                let outcome =
                    ParseOutcome::fetch_failed(format!("HIBP paste request failed: {error}"), None);
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            if status == 404 {
                let outcome = ParseOutcome::parsed(Vec::new());
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
            let outcome = ParseOutcome::fetch_failed(
                format!("HIBP paste API returned HTTP {status}"),
                Some(status),
            );
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        #[serde(rename_all = "PascalCase")]
        struct HibpApiPaste {
            source: String,
            id: String,
            title: Option<String>,
            date: String,
            email_count: u64,
        }

        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                let outcome = ParseOutcome::fetch_failed(
                    format!("failed to read HIBP paste response: {error}"),
                    None,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        if text.trim().is_empty() {
            let outcome = ParseOutcome::parsed(Vec::new());
            PARSER_METRICS.record(&outcome);
            return outcome;
        }

        let pastes: Vec<HibpApiPaste> = match serde_json::from_str(&text) {
            Ok(pastes) => pastes,
            Err(error) => {
                let outcome = ParseOutcome::parse_failed(
                    format!("failed to parse HIBP paste JSON: {error}"),
                    &text,
                );
                PARSER_METRICS.record(&outcome);
                return outcome;
            }
        };

        let mut mapped = Vec::with_capacity(pastes.len());
        for paste in pastes {
            let source_lower = paste.source.to_lowercase();
            let id = paste.id.clone();
            let date = match chrono::DateTime::parse_from_rfc3339(&paste.date) {
                Ok(date) => date.with_timezone(&Utc),
                Err(error) => {
                    let outcome = ParseOutcome::parse_failed(
                        format!("failed to parse HIBP paste date '{}': {error}", paste.date),
                        &text,
                    );
                    PARSER_METRICS.record(&outcome);
                    return outcome;
                }
            };
            mapped.push(HibpPaste {
                source: paste.source,
                id: id.clone(),
                title: paste.title,
                date,
                email_count: paste.email_count,
                url: Some(format!("https://{}.com/{}", source_lower, id)),
            });
        }

        let outcome = ParseOutcome::parsed(mapped);
        PARSER_METRICS.record(&outcome);
        outcome
    }

    /// Aggregate breach results for an email.
    ///
    /// Never converts a fetch/parse failure into a "clean" result: the
    /// [`HibpCheckResult`] carries the failure states so callers cannot
    /// mistake a broken HIBP response for "no breaches".
    pub async fn full_check(&self, email: &str) -> HibpFullCheckResult {
        let breaches = self.check_breaches(email).await;
        let pastes = self.check_pastes(email).await;

        let breach_failure = breaches.failure_error().map(str::to_string);
        let paste_failure = pastes.failure_error().map(str::to_string);
        let breach_items: Vec<HibpBreach> = breaches.into_items();
        let paste_count = pastes.item_count();

        HibpFullCheckResult {
            result: HibpCheckResult {
                email: email.to_string(),
                breach_count: breach_items.len(),
                paste_count,
                total_affected: breach_items.iter().map(|b| b.pwn_count).sum(),
                highest_severity: breach_items
                    .iter()
                    .max_by(|a, b| a.pwn_count.cmp(&b.pwn_count))
                    .map(|b| b.title.clone()),
                breaches: breach_items,
                checked_at: Utc::now(),
            },
            breach_failure,
            paste_failure,
        }
    }
}

/// Full HIBP check result plus any fetch/parse failures that prevented a
/// definitive answer. A caller must not treat a non-`None` failure as "clean".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibpFullCheckResult {
    pub result: HibpCheckResult,
    pub breach_failure: Option<String>,
    pub paste_failure: Option<String>,
}

impl HibpFullCheckResult {
    /// True when every HIBP sub-check completed (a clean or breached answer).
    pub fn is_complete(&self) -> bool {
        self.breach_failure.is_none() && self.paste_failure.is_none()
    }

    /// True only when HIBP definitively reported no breach and no paste.
    pub fn is_definitively_clean(&self) -> bool {
        self.is_complete() && !self.result.is_pwned() && self.result.paste_count == 0
    }
}

/// Full HIBP check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibpCheckResult {
    pub email: String,
    pub breach_count: usize,
    pub paste_count: usize,
    pub total_affected: u64,
    pub highest_severity: Option<String>,
    pub breaches: Vec<HibpBreach>,
    pub checked_at: DateTime<Utc>,
}

impl HibpCheckResult {
    /// Whether any breach was found.
    pub fn is_pwned(&self) -> bool {
        self.breach_count > 0
    }

    /// Risk assessment.
    pub fn risk_level(&self) -> &'static str {
        if self.breach_count == 0 {
            "LOW"
        } else if self.breach_count <= 2 {
            "MEDIUM"
        } else {
            "HIGH"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hibp_breach_has_passwords() {
        let breach = HibpBreach {
            name: "LinkedIn".to_string(),
            title: "LinkedIn".to_string(),
            domain: "linkedin.com".to_string(),
            breach_date: Utc::now(),
            added_date: Utc::now(),
            modified_date: Utc::now(),
            pwn_count: 1_000_000,
            description: "LinkedIn breach".to_string(),
            data_classes: vec!["Email addresses".to_string(), "Passwords".to_string()],
            is_verified: true,
            is_fabricated: false,
            is_sensitive: false,
            is_retired: false,
            is_spam_list: false,
            logo_path: None,
        };
        assert!(breach.has_passwords());
        assert!(breach.has_emails());
    }

    #[test]
    fn hibp_breach_severity_score() {
        let breach = HibpBreach {
            name: "Test".to_string(),
            title: "Test".to_string(),
            domain: "test.com".to_string(),
            breach_date: Utc::now(),
            added_date: Utc::now(),
            modified_date: Utc::now(),
            pwn_count: 10_000_000,
            description: "Test".to_string(),
            data_classes: vec!["Passwords".to_string(), "Email addresses".to_string()],
            is_verified: true,
            is_fabricated: false,
            is_sensitive: false,
            is_retired: false,
            is_spam_list: false,
            logo_path: None,
        };
        assert!(breach.severity_score() > 0.0);
    }

    #[test]
    fn hibp_paste_is_pastebin() {
        let paste = HibpPaste {
            source: "Pastebin".to_string(),
            id: "abc123".to_string(),
            title: None,
            date: Utc::now(),
            email_count: 100,
            url: None,
        };
        assert!(paste.is_pastebin());
    }

    #[test]
    fn hibp_check_result_not_pwned() {
        let result = HibpCheckResult {
            email: "test@example.com".to_string(),
            breach_count: 0,
            paste_count: 0,
            total_affected: 0,
            highest_severity: None,
            breaches: vec![],
            checked_at: Utc::now(),
        };
        assert!(!result.is_pwned());
        assert_eq!(result.risk_level(), "LOW");
    }

    #[test]
    fn hibp_monitor_constructs() {
        let result = HibpMonitor::new(Default::default());
        assert!(result.is_ok());
    }

    #[test]
    fn hibp_full_check_treats_failures_as_not_clean() {
        let clean_result = HibpCheckResult {
            email: "test@example.com".to_string(),
            breach_count: 0,
            paste_count: 0,
            total_affected: 0,
            highest_severity: None,
            breaches: vec![],
            checked_at: Utc::now(),
        };
        let clean = HibpFullCheckResult {
            result: clean_result.clone(),
            breach_failure: None,
            paste_failure: None,
        };
        assert!(clean.is_complete());
        assert!(clean.is_definitively_clean());

        let degraded = HibpFullCheckResult {
            result: clean_result,
            breach_failure: Some("failed to parse HIBP JSON".to_string()),
            paste_failure: None,
        };
        assert!(!degraded.is_complete());
        assert!(!degraded.is_definitively_clean());
    }
}
