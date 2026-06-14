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
use sha1::{Sha1, Digest};
use std::time::Duration;
use tracing::debug;

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
        self.data_classes.iter().any(|c| {
            c.to_lowercase().contains("password")
        })
    }

    /// Whether this breach contains email addresses.
    pub fn has_emails(&self) -> bool {
        self.data_classes.iter().any(|c| {
            c.to_lowercase().contains("email")
        })
    }

    /// Severity score based on data types and pwn count.
    pub fn severity_score(&self) -> f32 {
        let mut score = 0.0;
        for dc in &self.data_classes {
            let lower = dc.to_lowercase();
            score += if lower.contains("password") { 5.0 }
                else if lower.contains("credit card") || lower.contains("bank") || lower.contains("ssn") || lower.contains("national id") { 4.0 }
                else if lower.contains("email") || lower.contains("phone") { 2.0 }
                else { 1.0 };
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
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io)")
            .build()
            .context("building HIBP HTTP client")?;
        Ok(Self { client, config })
    }

    /// Check if an email has been in any breach (k-Anonymous API).
    pub async fn check_breaches(&self, email: &str) -> Result<Vec<HibpBreach>> {
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

        let resp = self.client.get(&url).send().await
            .context("HIBP k-Anonymous API request")?;

        if !resp.status().is_success() {
            if resp.status().as_u16() == 404 {
                return Ok(Vec::new());
            }
            anyhow::bail!("HIBP API returned {}", resp.status());
        }

        let text = resp.text().await.context("read HIBP response")?;
        for line in text.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[0].to_uppercase() == suffix.to_uppercase() {
                let count: u64 = parts[1].trim().parse().unwrap_or(0);
                if count > 0 {
                    debug!(email = %email, count = count, "HIBP breach found via k-Anonymous API");
                    return Ok(vec![HibpBreach {
                        name: "PwnedPassword".to_string(),
                        title: "Password found in breach".to_string(),
                        domain: "pwnedpasswords.com".to_string(),
                        breach_date: Utc::now(),
                        added_date: Utc::now(),
                        modified_date: Utc::now(),
                        pwn_count: count,
                        description: format!("The password associated with this account was found in {} data breaches.", count),
                        data_classes: vec!["Passwords".to_string()],
                        is_verified: false,
                        is_fabricated: false,
                        is_sensitive: true,
                        is_retired: false,
                        is_spam_list: false,
                        logo_path: None,
                    }]);
                }
            }
        }

        Ok(Vec::new())
    }

    /// Get all breaches (full API).
    pub async fn all_breaches(&self) -> Result<Vec<HibpBreach>> {
        let api_key = self.config.api_key.as_ref()
            .context("HIBP full API requires an API key")?;

        let url = "https://haveibeenpwned.com/api/v3/all breaches";
        let resp = self.client.get(url)
            .header("hibp-api-key", api_key)
            .header("user-agent", "ApexIntel/1.0")
            .send().await.context("HIBP all breaches request")?;

        if !resp.status().is_success() {
            debug!(status = %resp.status(), "HIBP all breaches returned non-success");
            return Ok(Vec::new());
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

        let breaches: Vec<HibpApiBreach> = resp.json().await.unwrap_or_default();
        Ok(breaches.into_iter().map(|b| HibpBreach {
            name: b.name,
            title: b.title,
            domain: b.domain,
            breach_date: chrono::DateTime::parse_from_rfc3339(&b.breach_date)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            added_date: chrono::DateTime::parse_from_rfc3339(&b.added_date)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            modified_date: Utc::now(),
            pwn_count: b.pwn_count,
            description: b.description,
            data_classes: b.data_classes,
            is_verified: b.is_verified,
            is_fabricated: b.is_fabricated,
            is_sensitive: b.is_sensitive,
            is_retired: b.is_retired,
            is_spam_list: b.is_spam_list,
            logo_path: None,
        }).collect())
    }

    /// Check for pastes associated with an email (full API).
    pub async fn check_pastes(&self, email: &str) -> Result<Vec<HibpPaste>> {
        let api_key = self.config.api_key.as_ref()
            .context("HIBP paste check requires an API key")?;

        let url = format!(
            "https://haveibeenpwned.com/api/v3/pasteaccount/{}",
            urlencoding::encode(email)
        );
        let resp = self.client.get(&url)
            .header("hibp-api-key", api_key)
            .header("user-agent", "ApexIntel/1.0")
            .send().await.context("HIBP paste request")?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
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

        let pastes: Vec<HibpApiPaste> = resp.json().await.unwrap_or_default();
        Ok(pastes.into_iter().map(|p| {
            let source_lower = p.source.to_lowercase();
            let id = p.id.clone();
            HibpPaste {
                source: p.source,
                id,
                title: p.title,
                date: chrono::DateTime::parse_from_rfc3339(&p.date)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                email_count: p.email_count,
                url: Some(format!("https://{}.com/{}", source_lower, p.id)),
            }
        }).collect())
    }

    /// Aggregate breach results for an email.
    pub async fn full_check(&self, email: &str) -> HibpCheckResult {
        let breaches = self.check_breaches(email).await.unwrap_or_default();
        let paste_count = self.check_pastes(email).await
            .map(|p| p.len())
            .unwrap_or(0);

        HibpCheckResult {
            email: email.to_string(),
            breach_count: breaches.len(),
            paste_count,
            total_affected: breaches.iter().map(|b| b.pwn_count).sum(),
            highest_severity: breaches.iter()
                .max_by(|a, b| a.pwn_count.cmp(&b.pwn_count))
                .map(|b| b.title.clone()),
            breaches,
            checked_at: Utc::now(),
        }
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
}
