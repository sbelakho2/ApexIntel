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

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::parse_outcome::{ParseOutcome, PARSER_METRICS};

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
        let client = crate::http::external_client_or_panic(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(20),
            user_agent: Some(crate::fetch_policy::BROWSER_USER_AGENT.to_string()),
            ..crate::http::ExternalClientOptions::default()
        });
        Self {
            client,
            apollo_key: std::env::var("APOLLO_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            hunter_key: std::env::var("HUNTER_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            clearbit_key: std::env::var("CLEARBIT_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    /// Discover contact methods for a person at a company domain.
    ///
    /// Tries each configured provider in waterfall order, then the website
    /// fallback. Returns a [`ParseOutcome`]: a provider schema change is a
    /// parse failure (never an empty "not found"), and an empty
    /// `ParsedSuccessfully` means a provider answered cleanly and genuinely
    /// had no match.
    pub async fn enrich(
        &self,
        full_name: &str,
        company_domain: &str,
    ) -> ParseOutcome<EnrichedContact> {
        let mut all: Vec<EnrichedContact> = Vec::new();
        let mut first_fetch_failure: Option<(String, Option<u16>)> = None;
        let mut first_parse_failure: Option<(String, String)> = None;
        let mut any_parsed = false;

        if let Some(key) = &self.apollo_key {
            match self.apollo_lookup(key, full_name, company_domain).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    if !items.is_empty() {
                        debug!(name = full_name, n = items.len(), "contact: apollo hit");
                        all.extend(items);
                    } else {
                        debug!(name = full_name, "contact: apollo miss");
                    }
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    warn!(error = %error, "contact: apollo error");
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    warn!(error = %error, "contact: apollo parser error");
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        if all.is_empty() {
            if let Some(key) = &self.hunter_key {
                match self.hunter_lookup(key, full_name, company_domain).await {
                    ParseOutcome::ParsedSuccessfully { items } => {
                        any_parsed = true;
                        if !items.is_empty() {
                            debug!(name = full_name, n = items.len(), "contact: hunter hit");
                            all.extend(items);
                        } else {
                            debug!(name = full_name, "contact: hunter miss");
                        }
                    }
                    ParseOutcome::FetchFailed { error, http_status } => {
                        warn!(error = %error, "contact: hunter error");
                        first_fetch_failure.get_or_insert((error, http_status));
                    }
                    ParseOutcome::ParseFailed {
                        error,
                        redacted_sample,
                    } => {
                        warn!(error = %error, "contact: hunter parser error");
                        first_parse_failure.get_or_insert((error, redacted_sample));
                    }
                }
            }
        }

        if all.is_empty() {
            if let Some(key) = &self.clearbit_key {
                match self.clearbit_lookup(key, full_name, company_domain).await {
                    ParseOutcome::ParsedSuccessfully { items } => {
                        any_parsed = true;
                        if !items.is_empty() {
                            debug!(name = full_name, n = items.len(), "contact: clearbit hit");
                            all.extend(items);
                        } else {
                            debug!(name = full_name, "contact: clearbit miss");
                        }
                    }
                    ParseOutcome::FetchFailed { error, http_status } => {
                        warn!(error = %error, "contact: clearbit error");
                        first_fetch_failure.get_or_insert((error, http_status));
                    }
                    ParseOutcome::ParseFailed {
                        error,
                        redacted_sample,
                    } => {
                        warn!(error = %error, "contact: clearbit parser error");
                        first_parse_failure.get_or_insert((error, redacted_sample));
                    }
                }
            }
        }

        // Always-available website fallback: scrape the company site for a
        // matching mailto/phone. Low confidence, real provenance.
        if all.is_empty() {
            match self.website_scrape(full_name, company_domain).await {
                ParseOutcome::ParsedSuccessfully { items } => {
                    any_parsed = true;
                    if !items.is_empty() {
                        debug!(name = full_name, n = items.len(), "contact: website hit");
                        all.extend(items);
                    } else {
                        debug!(name = full_name, "contact: website miss");
                    }
                }
                ParseOutcome::FetchFailed { error, http_status } => {
                    warn!(error = %error, "contact: website error");
                    first_fetch_failure.get_or_insert((error, http_status));
                }
                ParseOutcome::ParseFailed {
                    error,
                    redacted_sample,
                } => {
                    warn!(error = %error, "contact: website parser error");
                    first_parse_failure.get_or_insert((error, redacted_sample));
                }
            }
        }

        let outcome = if let Some((error, redacted_sample)) = first_parse_failure {
            ParseOutcome::ParseFailed {
                error,
                redacted_sample,
            }
        } else if !any_parsed && all.is_empty() {
            match first_fetch_failure {
                Some((error, http_status)) => ParseOutcome::FetchFailed { error, http_status },
                None => ParseOutcome::parsed(Vec::new()),
            }
        } else {
            ParseOutcome::parsed(all)
        };
        PARSER_METRICS.record(&outcome);
        outcome
    }

    // ── Apollo ──────────────────────────────────────────────────────────────

    async fn apollo_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> ParseOutcome<EnrichedContact> {
        let (first, last) = split_name(full_name);
        let body = serde_json::json!({
            "first_name": first,
            "last_name": last,
            "organization_domains": [domain],
        });
        let resp = match self
            .client
            .post("https://api.apollo.io/api/v1/people/match")
            .header("X-Api-Key", key)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(format!("apollo request failed: {error}"), None)
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            tracing::warn!(status = %resp.status(), "apollo_lookup: request failed (check API key/quota)");
            return ParseOutcome::fetch_failed(
                format!("apollo returned HTTP {status}"),
                Some(status),
            );
        }
        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read apollo response: {error}"),
                    None,
                )
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(json) => json,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse apollo JSON: {error}"),
                    &text,
                )
            }
        };
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
                verification_status: if verified {
                    "smtp_verified"
                } else {
                    "syntax_valid"
                }
                .into(),
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
        ParseOutcome::parsed(out)
    }

    // ── Hunter ──────────────────────────────────────────────────────────────

    async fn hunter_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> ParseOutcome<EnrichedContact> {
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
        let resp = match self.client.get(&url).header("X-Api-Key", key).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(format!("hunter request failed: {error}"), None)
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            tracing::warn!(status = %resp.status(), "hunter_lookup: request failed (check API key/quota)");
            return ParseOutcome::fetch_failed(
                format!("hunter returned HTTP {status}"),
                Some(status),
            );
        }
        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read hunter response: {error}"),
                    None,
                )
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(json) => json,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse hunter JSON: {error}"),
                    &text,
                )
            }
        };
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
        ParseOutcome::parsed(out)
    }

    // ── Clearbit ────────────────────────────────────────────────────────────

    async fn clearbit_lookup(
        &self,
        key: &str,
        full_name: &str,
        domain: &str,
    ) -> ParseOutcome<EnrichedContact> {
        let (first, last) = split_name(full_name);
        let url = format!(
            "https://person.clearbit.com/v1/people/find?domain={domain}&first_name={first}&last_name={last}"
        );
        let resp = match self.client.get(&url).bearer_auth(key).send().await {
            Ok(resp) => resp,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("clearbit request failed: {error}"),
                    None,
                )
            }
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return ParseOutcome::fetch_failed(
                format!("clearbit returned HTTP {status}"),
                Some(status),
            );
        }
        let text = match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await
        {
            Ok(text) => text,
            Err(error) => {
                return ParseOutcome::fetch_failed(
                    format!("failed to read clearbit response: {error}"),
                    None,
                )
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(json) => json,
            Err(error) => {
                return ParseOutcome::parse_failed(
                    format!("failed to parse clearbit JSON: {error}"),
                    &text,
                )
            }
        };
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
        ParseOutcome::parsed(out)
    }

    // ── Website scrape fallback (always available) ──────────────────────────

    async fn website_scrape(&self, full_name: &str, domain: &str) -> ParseOutcome<EnrichedContact> {
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
            return ParseOutcome::parsed(Vec::new());
        }

        let mut last_error: Option<String> = None;
        let mut fetched_any_page = false;
        for url in &candidates {
            let resp = match self.client.get(url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    last_error = Some(format!("{url} returned HTTP {}", r.status()));
                    continue;
                }
                Err(error) => {
                    last_error = Some(format!("{url} request failed: {error}"));
                    continue;
                }
            };
            fetched_any_page = true;
            let html =
                match crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await {
                    Ok(t) => t,
                    Err(error) => {
                        last_error = Some(format!("failed to read {url}: {error}"));
                        continue;
                    }
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
                    if let Some(phone) = extract_phone(line) {
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
                return ParseOutcome::parsed(out);
            }
        }

        if !fetched_any_page {
            // Every candidate page failed to fetch: this is a fetch failure,
            // not evidence that the person has no public contact method.
            return ParseOutcome::fetch_failed(
                last_error.unwrap_or_else(|| {
                    "no website contact page could be fetched (all candidates failed)".to_string()
                }),
                None,
            );
        }
        ParseOutcome::parsed(Vec::new())
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
    let end =
        rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>');
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
