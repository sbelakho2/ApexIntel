//! Dark-web and breach intelligence: HIBP domain breach checks, paste site
//! monitoring, and credential exposure detection.
//!
//! # Sources
//! - **Have I Been Pwned (HIBP) v3 API** — domain breach lookup (`/breachedaccount`
//!   and `/breaches` by domain).  Requires `HIBP_API_KEY` env var.
//! - **HIBP k-anonymity pwned passwords** — SHA-1 prefix check for password
//!   exposure (no API key required).
//! - **Pastebin scraper** — `/api/scrape` endpoint (requires Pastebin Pro) with
//!   entity keyword matching.
//! - **Intelligence X search** — leaked database search via `INTELX_API_KEY`.
//!
//! # Usage
//! ```no_run
//! use apex_crawl::breach::BreachMonitor;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let monitor = BreachMonitor::from_env()?;
//! let events = monitor.check_domain("elbit.co.il").await?;
//! for ev in &events {
//!     println!("{}: {} accounts exposed in {}", ev.domain, ev.pwn_count, ev.breach_name);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// Severity of a detected breach or exposure event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BreachSeverity {
    /// Informational exposure (aggregate stats, non-sensitive data).
    Low,
    /// Email addresses and non-critical metadata.
    Medium,
    /// Passwords, PII, financial data present.
    High,
    /// Credentials + sensitive personal data (SSN, passport, etc.).
    Critical,
}

impl BreachSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    fn from_data_classes(classes: &[String]) -> Self {
        let s: Vec<&str> = classes.iter().map(|s| s.as_str()).collect();
        if s.iter().any(|c| {
            matches!(
                *c,
                "Passwords"
                    | "Password hints"
                    | "Credit cards"
                    | "Bank account numbers"
                    | "Government issued IDs"
                    | "Passport numbers"
            )
        }) {
            return Self::Critical;
        }
        if s.iter().any(|c| {
            matches!(
                *c,
                "Email addresses" | "Phone numbers" | "Physical addresses" | "Dates of birth"
            )
        }) {
            return Self::High;
        }
        if s.iter()
            .any(|c| matches!(*c, "Usernames" | "Names" | "Social media profiles"))
        {
            return Self::Medium;
        }
        Self::Low
    }
}

/// A breach event detected for a monitored domain or entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreachEvent {
    /// Source of the detection: `"hibp"`, `"paste"`, `"intelx"`.
    pub source: String,
    /// Breach name / paste title / dataset identifier.
    pub breach_name: String,
    /// Domain associated with the breach.
    pub domain: String,
    /// Number of compromised accounts/records (0 if unknown).
    pub pwn_count: u64,
    /// Data classes present in the breach.
    pub data_classes: Vec<String>,
    /// When the breach occurred (may be None for pastes).
    pub breach_date: Option<NaiveDate>,
    /// When the breach was published / added to source database.
    pub added_at: Option<DateTime<Utc>>,
    /// Severity assessment.
    pub severity: BreachSeverity,
    /// Whether this is a verified breach (false for unverified pastes).
    pub is_verified: bool,
    /// Whether this breach is considered sensitive (adult, etc.).
    pub is_sensitive: bool,
    /// Short description of the breach.
    pub description: Option<String>,
    /// URL to the source record.
    pub source_url: Option<String>,
    /// Raw paste content snippet (for paste events).
    pub paste_snippet: Option<String>,
}

/// Result of a paste search for an entity keyword.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteMatch {
    /// Platform: `"pastebin"`, `"github_gist"`, `"ghostbin"`, etc.
    pub platform: String,
    /// Paste identifier / URL.
    pub paste_id: String,
    /// Paste title if available.
    pub title: Option<String>,
    /// Creation date.
    pub created_at: Option<DateTime<Utc>>,
    /// Keywords from the entity that matched.
    pub matched_keywords: Vec<String>,
    /// Content snippet around the match.
    pub snippet: Option<String>,
    /// Estimated size in bytes.
    pub size_bytes: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// HIBP API response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct HibpBreach {
    name: String,
    title: String,
    domain: String,
    breach_date: Option<String>,
    added_date: Option<String>,
    modified_date: Option<String>,
    pwn_count: Option<u64>,
    description: Option<String>,
    data_classes: Option<Vec<String>>,
    is_verified: Option<bool>,
    is_fabricated: Option<bool>,
    is_sensitive: Option<bool>,
    is_retired: Option<bool>,
    is_spam_list: Option<bool>,
    logo_path: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Intelligence X response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct IntelXSearchResponse {
    id: Option<String>,
}

#[derive(Deserialize, Debug)]
struct IntelXResultResponse {
    records: Option<Vec<IntelXRecord>>,
}

#[derive(Deserialize, Debug)]
struct IntelXRecord {
    #[serde(rename = "systemid")]
    system_id: Option<String>,
    #[serde(rename = "date")]
    date: Option<String>,
    name: Option<String>,
    #[serde(rename = "type")]
    record_type: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Monitor
// ─────────────────────────────────────────────────────────────────────────────

/// Breach and dark-web exposure monitor.
#[derive(Debug, Clone)]
pub struct BreachMonitor {
    client: Client,
    hibp_api_key: Option<String>,
    intelx_api_key: Option<String>,
    pastebin_api_key: Option<String>,
}

impl BreachMonitor {
    /// Create using environment variables:
    /// - `HIBP_API_KEY` — Have I Been Pwned API key (required for HIBP lookups)
    /// - `INTELX_API_KEY` — Intelligence X API key (optional)
    /// - `PASTEBIN_API_DEV_KEY` — Pastebin developer key (optional, Pro only)
    pub fn from_env() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io)")
            .build()
            .context("Build reqwest client")?;

        Ok(Self {
            client,
            hibp_api_key: std::env::var("HIBP_API_KEY").ok(),
            intelx_api_key: std::env::var("INTELX_API_KEY").ok(),
            pastebin_api_key: std::env::var("PASTEBIN_API_DEV_KEY").ok(),
        })
    }

    pub fn new(
        hibp_api_key: Option<String>,
        intelx_api_key: Option<String>,
        pastebin_api_key: Option<String>,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io)")
            .build()
            .context("Build reqwest client")?;
        Ok(Self {
            client,
            hibp_api_key,
            intelx_api_key,
            pastebin_api_key,
        })
    }

    // ── HIBP domain breach lookup ─────────────────────────────────────────────

    /// Check all known breaches for a domain (e.g. `"elbit.co.il"`).
    ///
    /// Returns a `Vec<BreachEvent>` sorted by severity descending.
    /// Returns an empty vec if HIBP API key is not configured.
    pub async fn check_domain(&self, domain: &str) -> Result<Vec<BreachEvent>> {
        let key = match &self.hibp_api_key {
            Some(k) => k.clone(),
            None => {
                warn!(
                    "HIBP_API_KEY not set — skipping domain breach check for {}",
                    domain
                );
                return Ok(vec![]);
            }
        };

        let url = format!(
            "https://haveibeenpwned.com/api/v3/breaches?domain={}",
            urlencoding::encode(domain)
        );

        debug!(domain = %domain, "HIBP domain breach lookup");

        let resp = self
            .client
            .get(&url)
            .header("hibp-api-key", &key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("HIBP domain breach HTTP request")?;

        if resp.status() == StatusCode::NOT_FOUND {
            // No breaches found — HIBP returns 404 for clean domains
            return Ok(vec![]);
        }

        if resp.status() == StatusCode::UNAUTHORIZED {
            warn!("HIBP API key invalid or expired (401)");
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("HIBP domain breach API returned {}: {}", status, body);
        }

        let breaches: Vec<HibpBreach> = resp.json().await.context("Parse HIBP breach response")?;

        let mut events: Vec<BreachEvent> = breaches
            .into_iter()
            .filter(|b| !b.is_retired.unwrap_or(false) && !b.is_spam_list.unwrap_or(false))
            .map(|b| {
                let data_classes = b.data_classes.unwrap_or_default();
                let severity = BreachSeverity::from_data_classes(&data_classes);
                let breach_date = b
                    .breach_date
                    .as_deref()
                    .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
                let added_at = b
                    .added_date
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                BreachEvent {
                    source: "hibp".into(),
                    breach_name: b.name,
                    domain: b.domain,
                    pwn_count: b.pwn_count.unwrap_or(0),
                    data_classes,
                    breach_date,
                    added_at,
                    severity,
                    is_verified: b.is_verified.unwrap_or(false),
                    is_sensitive: b.is_sensitive.unwrap_or(false),
                    description: b.description,
                    source_url: Some(format!(
                        "https://haveibeenpwned.com/PwnedWebsites#{}",
                        urlencoding::encode(&b.title)
                    )),
                    paste_snippet: None,
                }
            })
            .collect();

        // Sort by severity descending, then by pwn_count
        events.sort_unstable_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(b.pwn_count.cmp(&a.pwn_count))
        });

        debug!(domain = %domain, count = events.len(), "HIBP domain check complete");
        Ok(events)
    }

    // ── HIBP account breach lookup ────────────────────────────────────────────

    /// Check all breaches for a specific email address.
    ///
    /// Returns truncated breach names (not full detail) per HIBP v3 spec.
    /// Returns an empty vec if HIBP API key is not configured.
    pub async fn check_email(&self, email: &str) -> Result<Vec<BreachEvent>> {
        let key = match &self.hibp_api_key {
            Some(k) => k.clone(),
            None => {
                warn!("HIBP_API_KEY not set — skipping email breach check");
                return Ok(vec![]);
            }
        };

        let url = format!(
            "https://haveibeenpwned.com/api/v3/breachedaccount/{}?truncateResponse=false",
            urlencoding::encode(email)
        );

        let resp = self
            .client
            .get(&url)
            .header("hibp-api-key", &key)
            .header("Accept", "application/json")
            .send()
            .await
            .context("HIBP email breach HTTP request")?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }

        if resp.status() == StatusCode::UNAUTHORIZED {
            warn!("HIBP API key invalid (401) for email check");
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            anyhow::bail!("HIBP email breach API returned {}", status);
        }

        let breaches: Vec<HibpBreach> = resp
            .json()
            .await
            .context("Parse HIBP email breach response")?;

        let domain = email.split('@').nth(1).unwrap_or("unknown").to_string();
        let events = breaches
            .into_iter()
            .map(|b| {
                let data_classes = b.data_classes.unwrap_or_default();
                let severity = BreachSeverity::from_data_classes(&data_classes);
                let breach_date = b
                    .breach_date
                    .as_deref()
                    .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
                BreachEvent {
                    source: "hibp".into(),
                    breach_name: b.name,
                    domain: domain.clone(),
                    pwn_count: b.pwn_count.unwrap_or(0),
                    data_classes,
                    breach_date,
                    added_at: None,
                    severity,
                    is_verified: b.is_verified.unwrap_or(false),
                    is_sensitive: b.is_sensitive.unwrap_or(false),
                    description: b.description,
                    source_url: None,
                    paste_snippet: None,
                }
            })
            .collect();

        Ok(events)
    }

    // ── HIBP k-anonymity pwned password check ────────────────────────────────

    /// Check if a SHA-1 hash prefix (5 chars) of a password has been pwned.
    ///
    /// Uses the HIBP k-anonymity Range API — no API key required.
    /// The caller should hash the password with SHA-1 and pass the first 5 hex chars.
    ///
    /// Returns the number of times the full hash appears in the response, or 0.
    pub async fn check_password_hash_prefix(&self, sha1_hash: &str) -> Result<u64> {
        if sha1_hash.len() < 5 {
            anyhow::bail!("SHA-1 prefix must be at least 5 characters");
        }
        let prefix = &sha1_hash[..5].to_uppercase();
        let suffix = &sha1_hash[5..].to_uppercase();

        let url = format!("https://api.pwnedpasswords.com/range/{}", prefix);

        let resp = self
            .client
            .get(&url)
            .header("Add-Padding", "true")
            .send()
            .await
            .context("HIBP k-anonymity request")?;

        if !resp.status().is_success() {
            anyhow::bail!("HIBP range API returned {}", resp.status());
        }

        let body = resp.text().await.context("Read HIBP range response")?;

        for line in body.lines() {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() == 2 && parts[0].trim().eq_ignore_ascii_case(suffix) {
                let count: u64 = parts[1].trim().parse().unwrap_or(0);
                return Ok(count);
            }
        }

        Ok(0)
    }

    // ── Intelligence X search ─────────────────────────────────────────────────

    /// Search Intelligence X for mentions of a domain or entity name in leaked
    /// databases, dark web pastes, and indexed breach data.
    ///
    /// Returns basic `BreachEvent` records for each match.
    /// Returns empty vec if `INTELX_API_KEY` is not configured.
    pub async fn search_intelx(&self, query: &str) -> Result<Vec<BreachEvent>> {
        let key = match &self.intelx_api_key {
            Some(k) => k.clone(),
            None => {
                debug!("INTELX_API_KEY not set — skipping IntelX search");
                return Ok(vec![]);
            }
        };

        // Step 1: submit search
        let search_url = "https://2.intelx.io/intelligent/search";
        let search_body = serde_json::json!({
            "term": query,
            "buckets": [],
            "lookuplevel": 0,
            "maxresults": 20,
            "timeout": 10,
            "datefrom": "",
            "dateto": "",
            "sort": 4,
            "media": 0,
            "terminate": []
        });

        let search_resp = self
            .client
            .post(search_url)
            .header("x-key", &key)
            .header("Content-Type", "application/json")
            .body(search_body.to_string())
            .send()
            .await
            .context("IntelX search submit")?;

        if !search_resp.status().is_success() {
            warn!("IntelX search submit returned {}", search_resp.status());
            return Ok(vec![]);
        }

        let search_result: IntelXSearchResponse = search_resp
            .json()
            .await
            .context("Parse IntelX search response")?;

        let search_id = match search_result.id {
            Some(id) if !id.is_empty() => id,
            _ => return Ok(vec![]),
        };

        // Step 2: fetch results (allow index to be ready)
        tokio::time::sleep(Duration::from_secs(2)).await;

        let results_url = format!(
            "https://2.intelx.io/intelligent/search/result?id={}&limit=20&offset=0",
            search_id
        );

        let result_resp = self
            .client
            .get(&results_url)
            .header("x-key", &key)
            .send()
            .await
            .context("IntelX fetch results")?;

        if !result_resp.status().is_success() {
            warn!("IntelX results fetch returned {}", result_resp.status());
            return Ok(vec![]);
        }

        let results: IntelXResultResponse = result_resp
            .json()
            .await
            .context("Parse IntelX results response")?;

        let records = results.records.unwrap_or_default();
        let events: Vec<BreachEvent> = records
            .into_iter()
            .filter_map(|r| {
                let name = r.name.unwrap_or_else(|| {
                    format!("intelx-{}", r.system_id.as_deref().unwrap_or("unknown"))
                });
                let added_at = r
                    .date
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                Some(BreachEvent {
                    source: "intelx".into(),
                    breach_name: name,
                    domain: query.to_string(),
                    pwn_count: 0,
                    data_classes: vec![],
                    breach_date: None,
                    added_at,
                    severity: BreachSeverity::Medium,
                    is_verified: false,
                    is_sensitive: true,
                    description: Some(format!(
                        "Intelligence X leaked record (type {})",
                        r.record_type.unwrap_or(0)
                    )),
                    source_url: r
                        .system_id
                        .map(|id| format!("https://intelx.io/?did={}", id)),
                    paste_snippet: None,
                })
            })
            .collect();

        debug!(query = %query, count = events.len(), "IntelX search complete");
        Ok(events)
    }

    // ── Pastebin paste monitoring ─────────────────────────────────────────────

    /// Scan recent public Pastebin pastes for entity keyword mentions.
    ///
    /// Uses Pastebin's `/api/api_scraping.php` endpoint (requires Pastebin Pro
    /// account; `PASTEBIN_API_DEV_KEY` must be set).
    ///
    /// Falls back to scanning the public recent pastes page if no API key.
    ///
    /// `keywords` — list of terms to search for in paste content (case-insensitive).
    /// `limit` — maximum number of recent pastes to scan (max 250).
    pub async fn scan_pastebin(&self, keywords: &[String], limit: u32) -> Result<Vec<PasteMatch>> {
        let limit = limit.min(250);

        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct PasteInfo {
            scrape_url: Option<String>,
            full_url: Option<String>,
            key: Option<String>,
            date: Option<String>,
            size: Option<String>,
            title: Option<String>,
        }

        // Choose endpoint based on API key availability
        let api_url = if let Some(ref api_key) = self.pastebin_api_key {
            format!(
                "https://scrape.pastebin.com/api_scraping.php?limit={}&api_dev_key={}",
                limit, api_key
            )
        } else {
            // Public endpoint — no key needed but capped at recent 25
            format!("https://scrape.pastebin.com/api_scraping.php?limit=25")
        };

        let list_resp = self
            .client
            .get(&api_url)
            .send()
            .await
            .context("Pastebin scrape list request")?;

        if list_resp.status() == StatusCode::FORBIDDEN {
            debug!("Pastebin scrape API requires Pro account — skipping");
            return Ok(vec![]);
        }

        if !list_resp.status().is_success() {
            warn!("Pastebin scraping API returned {}", list_resp.status());
            return Ok(vec![]);
        }

        let pastes: Vec<PasteInfo> = match list_resp.json().await {
            Ok(p) => p,
            Err(_) => return Ok(vec![]),
        };

        let mut matches = Vec::new();

        for paste in pastes.iter().take(limit as usize) {
            let scrape_url = match &paste.scrape_url {
                Some(u) => u.clone(),
                None => continue,
            };

            let content_resp = match self.client.get(&scrape_url).send().await {
                Ok(r) => r,
                Err(e) => {
                    debug!(url = %scrape_url, error = %e, "Paste content fetch failed");
                    continue;
                }
            };

            if !content_resp.status().is_success() {
                continue;
            }

            let content = match content_resp.text().await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let content_lower = content.to_lowercase();
            let matched: Vec<String> = keywords
                .iter()
                .filter(|kw| content_lower.contains(kw.to_lowercase().as_str()))
                .cloned()
                .collect();

            if !matched.is_empty() {
                // Extract a snippet around the first match
                let first_kw = matched[0].to_lowercase();
                let snippet = if let Some(pos) = content_lower.find(first_kw.as_str()) {
                    let start = pos.saturating_sub(80);
                    let end = (pos + first_kw.len() + 80).min(content.len());
                    Some(content[start..end].replace('\n', " ").trim().to_string())
                } else {
                    None
                };

                let created_at = paste
                    .date
                    .as_deref()
                    .and_then(|s| s.parse::<i64>().ok())
                    .and_then(|ts| DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc));

                let size_bytes = paste.size.as_deref().and_then(|s| s.parse::<u64>().ok());

                matches.push(PasteMatch {
                    platform: "pastebin".into(),
                    paste_id: paste.key.clone().unwrap_or_default(),
                    title: paste.title.clone(),
                    created_at,
                    matched_keywords: matched,
                    snippet,
                    size_bytes,
                });
            }
        }

        debug!(keywords = %keywords.join(","), hits = matches.len(), "Pastebin scan complete");
        Ok(matches)
    }

    // ── Convenience: full domain exposure check ───────────────────────────────

    /// Run all breach checks for a domain and compile them into a unified
    /// `Vec<BreachEvent>`.
    ///
    /// - HIBP domain breach lookup
    /// - IntelX search for the domain
    ///
    /// Errors from individual sources are logged and suppressed.
    pub async fn full_domain_exposure_check(&self, domain: &str) -> Vec<BreachEvent> {
        let mut events = Vec::new();

        match self.check_domain(domain).await {
            Ok(mut e) => events.append(&mut e),
            Err(e) => warn!(error = %e, domain = %domain, "HIBP domain check failed"),
        }

        match self.search_intelx(domain).await {
            Ok(mut e) => events.append(&mut e),
            Err(e) => warn!(error = %e, domain = %domain, "IntelX search failed"),
        }

        // Deduplicate by (source, breach_name) — must sort on these keys first
        // because dedup_by only removes *consecutive* duplicates.
        events.sort_unstable_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then(a.breach_name.cmp(&b.breach_name))
        });
        events.dedup_by(|a, b| a.source == b.source && a.breach_name == b.breach_name);

        // Final sort: severity desc
        events.sort_unstable_by(|a, b| b.severity.cmp(&a.severity));
        events
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Compute SHA-1 hash of a password (uppercase hex).
///
/// Used together with [`BreachMonitor::check_password_hash_prefix`] for k-anonymity checks.
/// Pass the **full 40-char hex hash** to `check_password_hash_prefix` — it will
/// internally split the first 5 chars as the API prefix and compare the remainder.
pub fn sha1_hex(input: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode_upper(result)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breach_severity_ordering() {
        assert!(BreachSeverity::Critical > BreachSeverity::High);
        assert!(BreachSeverity::High > BreachSeverity::Medium);
        assert!(BreachSeverity::Medium > BreachSeverity::Low);
    }

    #[test]
    fn severity_from_data_classes_critical() {
        let classes = vec!["Passwords".to_string(), "Email addresses".to_string()];
        assert_eq!(
            BreachSeverity::from_data_classes(&classes),
            BreachSeverity::Critical
        );
    }

    #[test]
    fn severity_from_data_classes_high() {
        let classes = vec!["Email addresses".to_string(), "Phone numbers".to_string()];
        assert_eq!(
            BreachSeverity::from_data_classes(&classes),
            BreachSeverity::High
        );
    }

    #[test]
    fn severity_from_data_classes_low() {
        let classes = vec!["Avatars".to_string()];
        assert_eq!(
            BreachSeverity::from_data_classes(&classes),
            BreachSeverity::Low
        );
    }

    #[test]
    fn sha1_hex_produces_hex_string() {
        let h = sha1_hex("password");
        assert!(!h.is_empty());
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn monitor_constructs_without_keys() {
        let monitor = BreachMonitor::new(None, None, None).unwrap();
        // Without HIBP key, domain check returns empty
        let result = monitor.check_domain("example.com").await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn password_range_check_bad_prefix() {
        let monitor = BreachMonitor::new(None, None, None).unwrap();
        let result = monitor.check_password_hash_prefix("abc").await;
        assert!(result.is_err()); // too short
    }
}
