//! Contact-data enrichment — verified email / phone / LinkedIn discovery.
//!
//! Closes the ZoomInfo/Apollo "contact waterfall" gap: discovers how to actually
//! *reach* a decision maker (not just *who* they are). Results are stored in the
//! `contact_methods` table via the store layer.
//!
//! # Providers
//!
//! The enricher supports a configurable provider waterfall, tried in order until
//! one yields a verified-enough contact:
//!   1. **Apollo** (if `APOLLO_API_KEY` set) — paid, highest yield.
//!   2. **Hunter** (if `HUNTER_API_KEY` set) — email finder + verifier.
//!   3. **Clearbit** (if `CLEARBIT_API_KEY` set) — enrichment.
//!   4. **Website scrape fallback** — always available; scrapes the company
//!      website for a `mailto:` link / phone pattern matching the person's name.
//!
//! Every provider degrades gracefully to `Ok(None)` on failure so a transient
//! outage or missing key never aborts the enrichment pipeline. No provider is
//! ever faked — if no key is configured and the website has no match, the
//! contact is simply not enriched.

use std::time::Duration;

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// A discovered, provenance-tracked contact method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedContact {
    pub contact_type: String,
    pub value: String,
    /// 0..1 confidence per the provider.
    pub confidence: f32,
    /// not_verified / syntax_valid / smtp_verified / guessed.
    pub verification_status: String,
    /// apollo / clearbit / hunter / website_scrape / other.
    pub source: String,
}

/// Multi-provider contact enrichment client.
pub struct ContactEnricher {
    client: Client,
    apollo_key: Option<String>,
    hunter_key: Option<String>,
    clearbit_key: Option<String>,
}

impl ContactEnricher {
    /// Build from environment. Reads optional provider API keys; the website
    /// fallback is always available.
    pub fn from_env() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("ApexIntel-Contacts/1.0")
            .build()
            .unwrap_or_else(|e| {
                warn!(error = %e, "contact_enrichment: client build failed; default");
                Client::new()
            });
        Self {
            client,
            apollo_key: std::env::var("APOLLO_API_KEY").ok().filter(|s| !s.is_empty()),
            hunter_key: std::env::var("HUNTER_API_KEY").ok().filter(|s| !s.is_empty()),
            clearbit_key: std::env::var("CLEARBIT_API_KEY").ok().filter(|s| !s.is_empty()),
        }
    }

    /// Discover contact methods for a person at a company domain.
    ///
    /// Tries each configured provider in waterfall order, then the website
    /// fallback. Returns the first non-empty set of results (providers may
    /// return multiple contact methods). Never returns an empty vec from a
    /// "faked success" — empty means genuinely nothing was found.
    pub async fn enrich(
        &self,
        full_name: &str,
        company_domain: &str,
    ) -> Result<Vec<EnrichedContact>> {
        let mut all: Vec<EnrichedContact> = Vec::new();

        if let Some(key) = &self.apollo_key {
            match self.apollo_lookup(key, full_name, company_domain).await {
                Ok(contacts) if !contacts.is_empty() => {
                    debug!(name = full_name, n = contacts.len(), "contact: apollo hit");
                    all.extend(contacts);
                }
                Ok(_) => debug!(name = full_name, "contact: apollo miss"),
                Err(e) => warn!(error = %e, "contact: apollo error"),
            }
        }

        if all.is_empty() {
            if let Some(key) = &self.hunter_key {
                match self.hunter_lookup(key, full_name, company_domain).await {
                    Ok(contacts) if !contacts.is_empty() => {
                        debug!(name = full_name, n = contacts.len(), "contact: hunter hit");
                        all.extend(contacts);
                    }
                    Ok(_) => debug!(name = full_name, "contact: hunter miss"),
                    Err(e) => warn!(error = %e, "contact: hunter error"),
                }
            }
        }

        if all.is_empty() {
            if let Some(key) = &self.clearbit_key {
                match self.clearbit_lookup(key, full_name, company_domain).await {
                    Ok(contacts) if !contacts.is_empty() => {
                        debug!(name = full_name, n = contacts.len(), "contact: clearbit hit");
                        all.extend(contacts);
                    }
                    Ok(_) => debug!(name = full_name, "contact: clearbit miss"),
                    Err(e) => warn!(error = %e, "contact: clearbit error"),
                }
            }
        }

        // Always-available website fallback: scrape the company site for a
        // matching mailto/phone. Low confidence, real provenance.
        if all.is_empty() {
            match self.website_scrape(full_name, company_domain).await {
                Ok(contacts) if !contacts.is_empty() => {
                    debug!(name = full_name, n = contacts.len(), "contact: website hit");
                    all.extend(contacts);
                }
                Ok(_) => debug!(name = full_name, "contact: website miss"),
                Err(e) => warn!(error = %e, "contact: website error"),
            }
        }

        Ok(all)
    }

    // ── Apollo ──────────────────────────────────────────────────────────────

    async fn apollo_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> Result<Vec<EnrichedContact>> {
        let (first, last) = split_name(full_name);
        let body = serde_json::json!({
            "first_name": first,
            "last_name": last,
            "organization_domains": [domain],
        });
        let resp = self
            .client
            .post("https://api.apollo.io/api/v1/people/match")
            .header("X-Api-Key", key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "apollo_lookup: request failed (check API key/quota)");
            return Ok(Vec::new());
        }
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let person = &json["person"];
        let mut out = Vec::new();
        if let Some(email) = person["email"].as_str().filter(|s| !s.is_empty()) {
            let verified = person["email_status"]
                .as_str()
                .map(|s| s == "verified")
                .unwrap_or(false);
            out.push(EnrichedContact {
                contact_type: "email".into(),
                value: email.to_string(),
                confidence: if verified { 0.95 } else { 0.7 },
                verification_status: if verified { "smtp_verified" } else { "syntax_valid" }.into(),
                source: "apollo".into(),
            });
        }
        if let Some(phone) = person["sanitized_phone"].as_str().filter(|s| !s.is_empty()) {
            out.push(EnrichedContact {
                contact_type: "phone".into(),
                value: phone.to_string(),
                confidence: 0.7,
                verification_status: "not_verified".into(),
                source: "apollo".into(),
            });
        }
        if let Some(li) = person["linkedin_url"].as_str().filter(|s| !s.is_empty()) {
            out.push(EnrichedContact {
                contact_type: "linkedin".into(),
                value: li.to_string(),
                confidence: 0.9,
                verification_status: "manual_confirmed".into(),
                source: "apollo".into(),
            });
        }
        Ok(out)
    }

    // ── Hunter ──────────────────────────────────────────────────────────────

    async fn hunter_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> Result<Vec<EnrichedContact>> {
        let (first, last) = split_name(full_name);
        // B331: encode name parts (spaces/unicode previously produced an
        // unparseable URL → silent miss) and pass the key via header instead
        // of the query string (URLs leak into logs and proxies).
        let url = format!(
            "https://api.hunter.io/v2/email-finder?domain={}&first_name={}&last_name={}",
            urlencoding::encode(domain),
            urlencoding::encode(&first),
            urlencoding::encode(&last),
        );
        let resp = self.client.get(&url).header("X-Api-Key", key).send().await?;
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "hunter_lookup: request failed (check API key/quota)");
            return Ok(Vec::new());
        }
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let data = &json["data"];
        let mut out = Vec::new();
        if let Some(email) = data["email"].as_str().filter(|s| !s.is_empty()) {
            let score = data["score"].as_i64().unwrap_or(50) as f32 / 100.0;
            let status = data["verification"]["status"]
                .as_str()
                .unwrap_or("not_verified");
            let mapped = match status {
                "valid" | "accept_all" => "smtp_verified",
                "invalid" => "bounced",
                _ => "guessed",
            };
            out.push(EnrichedContact {
                contact_type: "email".into(),
                value: email.to_string(),
                confidence: score.max(0.3),
                verification_status: mapped.into(),
                source: "hunter".into(),
            });
        }
        Ok(out)
    }

    // ── Clearbit ────────────────────────────────────────────────────────────

    async fn clearbit_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> Result<Vec<EnrichedContact>> {
        let (first, last) = split_name(full_name);
        let url = format!(
            "https://person.clearbit.com/v1/people/find?domain={domain}&first_name={first}&last_name={last}"
        );
        let resp = self.client.get(&url).bearer_auth(key).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let mut out = Vec::new();
        if let Some(email) = json["email"].as_str().filter(|s| !s.is_empty()) {
            out.push(EnrichedContact {
                contact_type: "email".into(),
                value: email.to_string(),
                confidence: 0.75,
                verification_status: "syntax_valid".into(),
                source: "clearbit".into(),
            });
        }
        if let Some(phone) = json["phone"].as_str().filter(|s| !s.is_empty()) {
            out.push(EnrichedContact {
                contact_type: "phone".into(),
                value: phone.to_string(),
                confidence: 0.6,
                verification_status: "not_verified".into(),
                source: "clearbit".into(),
            });
        }
        Ok(out)
    }

    // ── Website scrape fallback (always available) ──────────────────────────

    async fn website_scrape(
        &self,
        full_name: &str,
        domain: &str,
    ) -> Result<Vec<EnrichedContact>> {
        let domain = domain.trim().trim_start_matches("www.");
        // Try common contact/about pages.
        let candidates = [
            format!("https://{domain}/contact"),
            format!("https://{domain}/about"),
            format!("https://{domain}/team"),
            format!("https://{domain}/"),
        ];
        let (first, last) = split_name(full_name);
        let last_lower = last.to_lowercase();
        // B331: single-token names ("Cher") have no last name — `contains("")`
        // is always true, so the first mailto/tel on ANY page was attached to
        // the person. Require a real token to anchor on.
        let anchor = if last_lower.is_empty() {
            first.to_lowercase()
        } else {
            last_lower
        };
        if anchor.is_empty() {
            return Ok(Vec::new());
        }

        for url in &candidates {
            let resp = match self.client.get(url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };
            let html = match resp.text().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            // Look for a mailto: near the person's last name.
            let mut out = Vec::new();
            for line in html.lines() {
                let lower = line.to_lowercase();
                if lower.contains(&anchor) {
                    if let Some(email) = extract_mailto(&lower) {
                        out.push(EnrichedContact {
                            contact_type: "email".into(),
                            value: email,
                            confidence: 0.5,
                            verification_status: "guessed".into(),
                            source: "website_scrape".into(),
                        });
                    }
                    if let Some(phone) = extract_phone(&line) {
                        let _ = first;
                        out.push(EnrichedContact {
                            contact_type: "phone".into(),
                            value: phone,
                            confidence: 0.4,
                            verification_status: "not_verified".into(),
                            source: "website_scrape".into(),
                        });
                    }
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
        Ok(Vec::new())
    }
}

/// Split a full name into (first, last). Falls back to (full, "") on failure.
fn split_name(full: &str) -> (String, String) {
    let mut parts = full.split_whitespace();
    let first = parts.next().unwrap_or("").to_string();
    let last = parts.last().unwrap_or("").to_string();
    (first, last)
}

/// Extract an email address from a `mailto:` link in a lowercased HTML line.
fn extract_mailto(lower_line: &str) -> Option<String> {
    let idx = lower_line.find("mailto:")?;
    let rest = &lower_line[idx + 7..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>');
    let email = &rest[..end.unwrap_or(rest.len())];
    if email.contains('@') && email.contains('.') {
        Some(email.to_string())
    } else {
        None
    }
}

/// Extract an E.164-ish phone number from a text line.
fn extract_phone(line: &str) -> Option<String> {
    // Match +<digits and separators>, at least 7 digits.
    let tel = line.find("tel:")?;
    let rest = &line[tel + 4..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'');
    let phone = &rest[..end.unwrap_or(rest.len())];
    let digit_count = phone.chars().filter(|c| c.is_ascii_digit()).count();
    if digit_count >= 7 {
        Some(phone.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_name_basic() {
        assert_eq!(split_name("Jane Doe"), ("Jane".into(), "Doe".into()));
        assert_eq!(split_name("Jane Marie Doe"), ("Jane".into(), "Doe".into()));
        assert_eq!(split_name("Singleton"), ("Singleton".into(), "".into()));
    }

    #[test]
    fn extract_mailto_finds_email() {
        let e = extract_mailto("contact: <a href=\"mailto:jane.doe@example.com\">");
        assert_eq!(e.as_deref(), Some("jane.doe@example.com"));
    }

    #[test]
    fn extract_mailto_rejects_non_email() {
        assert!(extract_mailto("mailto:not-an-email").is_none());
    }
}
