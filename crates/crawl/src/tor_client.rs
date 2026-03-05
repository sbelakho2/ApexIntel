//! Tor SOCKS5 client for scraping onion / dark-web intelligence sources.
//!
//! Uses the locally-running `tor` daemon (127.0.0.1:9050) as a SOCKS5-over-Hostname
//! proxy so that `.onion` DNS is resolved inside Tor (via `socks5h://`).
//!
//! # Prerequisites
//! A `tor` daemon must be running and listening on port 9050.  The binary
//! is `tor` on most Linux systems and can be configured in `/etc/tor/torrc`.
//!
//! # Security note
//! Only ever use this for passive intelligence collection against public
//! onion sites (leak archives, breach compilations, dark-web executive
//! directories).  Never attempt to DDOS or exploit any service.

use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;
use tracing::{debug, warn};
use urlencoding::encode as urlencode;

// ─────────────────────────────────────────────────────────────────────────────
// Regex helpers
// ─────────────────────────────────────────────────────────────────────────────

static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap()
});
static RE_PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\+?[\d\s\-\(\)]{8,20}").unwrap()
});

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// A credential / contact record found in a breach or leak.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreachRecord {
    /// Email address found in the breach.
    pub email: String,
    /// Breached password (hashed or plaintext as stored).
    pub password_hash: Option<String>,
    /// Source breach label (e.g. `"pwndb"`, `"exposed_vc"`, `"dread_forum"`).
    pub source: String,
    /// Domain extracted from the email.
    pub domain: String,
    /// Approximate date the data was posted (ISO-8601 or empty).
    pub date_posted: Option<String>,
}

/// A scraped contact detail from an executive leak or OSINT onion source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnionContactRecord {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin: Option<String>,
    pub org: Option<String>,
    pub source_url: String,
    pub confidence: f32,
    pub ts_scraped: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// Wrapper around a `reqwest::Client` pre-configured to route all traffic
/// through the local Tor SOCKS5 proxy.
pub struct TorClient {
    client: reqwest::Client,
    available: bool,
}

impl TorClient {
    /// Build a new `TorClient`.  If the Tor proxy is unreachable the client
    /// is still created but `available` is `false`; all scrape methods will
    /// return empty results instead of errors.
    pub async fn new() -> Self {
        match build_tor_client() {
            Ok(client) => {
                // Quick connectivity test to the check.torproject.org onion equivalent.
                let available = client
                    .get("https://check.torproject.org/api/ip")
                    .timeout(Duration::from_secs(15))
                    .send()
                    .await
                    .is_ok();
                if !available {
                    warn!("tor_client: SOCKS5 proxy reachable but Tor circuit not established");
                }
                Self { client, available }
            }
            Err(e) => {
                warn!("tor_client: failed to build SOCKS5 client: {e:#}");
                Self {
                    client: reqwest::Client::new(),
                    available: false,
                }
            }
        }
    }

    /// Returns `true` if Tor is reachable and circuits are up.
    pub fn is_available(&self) -> bool {
        self.available
    }

    // ─── PwnDB ───────────────────────────────────────────────────────────

    /// Query the PwnDB onion leak database for credentials belonging to a
    /// given email domain.
    ///
    /// `pwndb2am4tzkvold.onion` exposes a simple PHP query interface.
    pub async fn query_pwndb_by_domain(&self, domain: &str) -> Vec<BreachRecord> {
        if !self.available {
            return vec![];
        }
        let url = format!(
            "http://pwndb2am4tzkvold.onion/?luser=%25&domain={}&lusers=Search",
            urlencode(domain)
        );
        let html = match self.get_text(&url, 45).await {
            Ok(v) => v,
            Err(e) => {
                warn!("pwndb query failed for {domain}: {e:#}");
                return vec![];
            }
        };
        parse_pwndb_html(&html, domain)
    }

    /// Search PwnDB for a specific email address.
    pub async fn query_pwndb_by_email(&self, email: &str) -> Vec<BreachRecord> {
        if !self.available {
            return vec![];
        }
        let (luser, domain) = match email.split_once('@') {
            Some(p) => p,
            None => return vec![],
        };
        let url = format!(
            "http://pwndb2am4tzkvold.onion/?luser={}&domain={}&lusers=Search",
            urlencode(luser),
            urlencode(domain)
        );
        let html = match self.get_text(&url, 45).await {
            Ok(v) => v,
            Err(e) => {
                warn!("pwndb email query failed for {email}: {e:#}");
                return vec![];
            }
        };
        parse_pwndb_html(&html, domain)
    }

    // ─── Exposed.vc / IntelX-style breach listings ───────────────────────

    /// Query the Exposed.vc onion mirror for name mentions.
    /// Returns raw email addresses found alongside this name.
    pub async fn query_exposed_vc(&self, full_name: &str) -> Vec<OnionContactRecord> {
        if !self.available {
            return vec![];
        }
        // The Exposed.vc onion address changes; this is the currently known mirror.
        // If unavailable the Tor client returns empty gracefully.
        let url = format!(
            "http://exposedboq3tpnmxx.onion/search?query={}",
            urlencode(full_name)
        );
        let html = match self.get_text(&url, 60).await {
            Ok(v) => v,
            Err(e) => {
                debug!("exposed_vc query skipped for {full_name}: {e:#}");
                return vec![];
            }
        };
        extract_onion_contacts(&html, &url, 0.4)
    }

    // ─── Ransomware / leak-site executive data ────────────────────────────

    /// Search known ransomware leak-site archives for mentions of a company
    /// or executive name.
    ///
    /// These sites publish stolen corporate data (annual reports, HR files,
    /// org charts) that contain executive rosters with contact details.
    pub async fn scrape_leak_sites_for_org(&self, org_name: &str) -> Vec<OnionContactRecord> {
        if !self.available {
            return vec![];
        }
        // Aggregated leak index (known working as of 2024-Q4).
        let sites = [
            format!(
                "http://ransomwarebugsctmseqejbm7dlgm4lol2eahrr2r2iyctba2d6vlxxad.onion/search?q={}",
                urlencode(org_name)
            ),
        ];
        let mut results = vec![];
        for url in &sites {
            if let Ok(html) = self.get_text(url, 60).await {
                results.extend(extract_onion_contacts(&html, url, 0.3));
            }
        }
        results
    }

    // ─── Haystack / Dread OSINT threads ──────────────────────────────────

    /// Scrape the Dread forum's OSINT/doxxing sub-boards for mentions of
    /// a person by name.  Returns contact-like records.
    pub async fn scrape_dread_osint(&self, full_name: &str) -> Vec<OnionContactRecord> {
        if !self.available {
            return vec![];
        }
        let url = format!(
            "http://dreadytofatroptsdj6io7l3xptbet6onoyno2yv7jicoxknyazubrad.onion/search?q={}",
            urlencode(full_name)
        );
        let html = match self.get_text(&url, 60).await {
            Ok(v) => v,
            Err(e) => {
                debug!("dread_osint query skipped for {full_name}: {e:#}");
                return vec![];
            }
        };
        extract_onion_contacts(&html, &url, 0.25)
    }

    /// Aggregate all dark-web contact intelligence for a person.
    ///
    /// Runs all enabled scrapers in sequence (Tor circuits serialised to
    /// reduce the chance of mid-session circuit rotation).
    pub async fn aggregate_dark_web_contacts(
        &self,
        full_name: &str,
        email_domain: Option<&str>,
    ) -> DarkWebPersonIntel {
        let now = Utc::now().timestamp();
        let mut breach_records = vec![];
        let mut contact_records = vec![];

        if let Some(domain) = email_domain {
            breach_records.extend(self.query_pwndb_by_domain(domain).await);
        }
        contact_records.extend(self.query_exposed_vc(full_name).await);
        contact_records.extend(self.scrape_dread_osint(full_name).await);

        // Extract any email addresses from breach records that match the name pattern.
        let name_tokens: Vec<String> = full_name
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .collect();

        let matched_emails: Vec<String> = breach_records
            .iter()
            .filter(|r| {
                let local = r.email.split('@').next().unwrap_or("").to_lowercase();
                name_tokens.iter().any(|tok| local.contains(tok.as_str()))
            })
            .map(|r| r.email.clone())
            .collect();

        DarkWebPersonIntel {
            full_name: full_name.to_string(),
            breach_records,
            contact_records,
            matched_emails,
            ts_scraped: now,
        }
    }

    // ─── Internal helpers ─────────────────────────────────────────────────

    async fn get_text(&self, url: &str, timeout_secs: u64) -> Result<String> {
        let resp = self
            .client
            .get(url)
            .timeout(Duration::from_secs(timeout_secs))
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0")
            .send()
            .await
            .context("tor GET failed")?;
        let text = resp.text().await.context("tor response body")?;
        Ok(text)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Aggregated output
// ─────────────────────────────────────────────────────────────────────────────

/// Full dark-web intelligence package for one person.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DarkWebPersonIntel {
    pub full_name: String,
    /// Raw credential records found in breach databases.
    pub breach_records: Vec<BreachRecord>,
    /// Contact details extracted from onion sources.
    pub contact_records: Vec<OnionContactRecord>,
    /// Email addresses from breach records that match the person's name tokens.
    pub matched_emails: Vec<String>,
    pub ts_scraped: i64,
}

impl DarkWebPersonIntel {
    /// Best email guess: first matched breach email, then first contact email.
    pub fn best_email(&self) -> Option<&str> {
        self.matched_emails
            .first()
            .map(String::as_str)
            .or_else(|| {
                self.contact_records
                    .iter()
                    .filter_map(|r| r.email.as_deref())
                    .next()
            })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

fn build_tor_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("socks5h://127.0.0.1:9050")?)
        .timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0")
        .danger_accept_invalid_certs(true) // many .onion sites have self-signed certs
        .build()
        .context("build tor reqwest client")
}

/// Parse the PwnDB HTML response and extract `BreachRecord` entries.
fn parse_pwndb_html(html: &str, domain: &str) -> Vec<BreachRecord> {
    // PwnDB returns an HTML table with rows like: luser | domain | password
    // We do a lightweight regex parse rather than a full HTML parser.
    let row_re = Regex::new(
        r"<li>luser:\s*(.+?)\s*</li>\s*<li>domain:\s*(.+?)\s*</li>(?:\s*<li>password:\s*(.+?)\s*</li>)?"
    ).unwrap();

    row_re
        .captures_iter(html)
        .filter_map(|cap| {
            let luser = cap.get(1)?.as_str().trim().to_string();
            let dom = cap.get(2).map_or(domain, |m| m.as_str()).trim().to_string();
            let password_hash = cap.get(3).map(|m| m.as_str().trim().to_string());
            if luser.is_empty() || luser.len() > 64 {
                return None;
            }
            Some(BreachRecord {
                email: format!("{luser}@{dom}"),
                password_hash,
                source: "pwndb".to_string(),
                domain: dom,
                date_posted: None,
            })
        })
        .collect()
}

/// Generic extractor: scan `html` for email addresses and phone numbers,
/// return as `OnionContactRecord` items.
fn extract_onion_contacts(html: &str, source_url: &str, confidence: f32) -> Vec<OnionContactRecord> {
    let now = Utc::now().timestamp();
    let emails: Vec<String> = RE_EMAIL
        .find_iter(html)
        .map(|m| m.as_str().to_lowercase())
        .filter(|e| !e.ends_with(".png") && !e.ends_with(".jpg"))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let phones: Vec<String> = RE_PHONE
        .find_iter(html)
        .map(|m| m.as_str().trim().to_string())
        .filter(|p| p.chars().filter(|c| c.is_ascii_digit()).count() >= 7)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if emails.is_empty() && phones.is_empty() {
        return vec![];
    }

    // Zip emails and phones into contact records (one record per email, attach a phone if available).
    let phone_ref = phones.first().cloned();
    emails
        .into_iter()
        .map(|email| OnionContactRecord {
            name: None,
            email: Some(email),
            phone: phone_ref.clone(),
            linkedin: None,
            org: None,
            source_url: source_url.to_string(),
            confidence,
            ts_scraped: now,
        })
        .collect()
}

// No internal urlencoding module needed — the `urlencoding` workspace crate is used directly.
