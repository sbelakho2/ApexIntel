//! # Production evidence providers
//!
//! Real network-backed [`EvidenceProvider`] implementations plus the
//! [`build_production_entity_verifier`] wiring used by worker discovery jobs.
//!
//! Providers:
//! - [`GleifProvider`] — GLEIF LEI records (registry-class identity anchor)
//! - [`SecEdgarProvider`] — SEC EDGAR full-text search (US filers only)
//! - [`RegistryAdapterProvider`] — configurable corporate-registry endpoints
//! - [`RdapProvider`] — RDAP registration data for the candidate's domain
//! - [`CorporateSiteProvider`] — structured metadata published by the
//!   candidate's own corporate site (`og:site_name`, `application-name`,
//!   schema.org JSON-LD, `<title>`)
//! - [`IndependentSourceDomainsProvider`] — corroboration from distinct crawl
//!   source domains already recorded on the candidate (never fabricated: it
//!   requires at least two real domains in the candidate metadata)
//!
//! Every provider only emits a signal when the found record plausibly matches
//! the candidate name ([`names_match`]). Network providers return `Err` on
//! transport/HTTP failures, which the verifier converts into analyst review —
//! a failing provider can never auto-register a company.
//!
//! No provider in this module is the seed directory,
//! [`EntityVerifier::seed_only`] is deliberately a separate constructor.

use crate::company_discovery::{normalize_company_name, CompanyCandidate};
use crate::entity_verifier::{
    domain_of, EntityVerifier, EvidenceProvider, EvidenceSignal, PgEvidenceStore,
    VerificationPolicy, VerificationType,
};
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use sqlx::PgPool;
use std::collections::BTreeSet;
use std::sync::OnceLock;

const GLEIF_ENDPOINT: &str = "https://api.gleif.org/api/v1/lei-records";
const SEC_EDGAR_ENDPOINT: &str = "https://efts.sec.gov/LATEST/search-index";
const RDAP_ENDPOINT_TEMPLATE: &str = "https://rdap.org/domain/{domain}";

// ─────────────────────────────────────────────────────────────────────────────
// Capabilities / builder
// ─────────────────────────────────────────────────────────────────────────────

/// Which real providers the production verifier is wired with, plus the
/// decision policy. All providers are enabled by default; registry endpoints
/// are opt-in via `ENTITY_REGISTRY_ENDPOINTS` (comma-separated URL templates
/// containing `{query}`).
#[derive(Debug, Clone)]
pub struct EntityVerifierCaps {
    pub gleif: bool,
    pub sec_edgar: bool,
    pub registry_adapters: bool,
    pub rdap: bool,
    pub corporate_site: bool,
    pub independent_sources: bool,
    /// Registry endpoints, each a URL template containing `{query}`.
    pub registry_endpoints: Vec<String>,
    /// Verification policy applied by the built verifier.
    pub policy: VerificationPolicy,
}

impl Default for EntityVerifierCaps {
    fn default() -> Self {
        Self {
            gleif: true,
            sec_edgar: true,
            registry_adapters: true,
            rdap: true,
            corporate_site: true,
            independent_sources: true,
            registry_endpoints: Vec::new(),
            policy: VerificationPolicy::default(),
        }
    }
}

impl EntityVerifierCaps {
    /// Caps with every provider enabled and no registry endpoints.
    pub fn all_enabled() -> Self {
        Self::default()
    }

    /// Read provider enable/disable flags and registry endpoints from the
    /// environment. Defaults to all providers enabled.
    pub fn from_env() -> Self {
        let mut caps = Self::default();
        caps.gleif = env_flag("ENTITY_VERIFY_GLEIF", caps.gleif);
        caps.sec_edgar = env_flag("ENTITY_VERIFY_SEC_EDGAR", caps.sec_edgar);
        caps.registry_adapters = env_flag("ENTITY_VERIFY_REGISTRY", caps.registry_adapters);
        caps.rdap = env_flag("ENTITY_VERIFY_RDAP", caps.rdap);
        caps.corporate_site = env_flag("ENTITY_VERIFY_CORPORATE_SITE", caps.corporate_site);
        caps.independent_sources = env_flag(
            "ENTITY_VERIFY_INDEPENDENT_SOURCES",
            caps.independent_sources,
        );
        if let Ok(raw) = std::env::var("ENTITY_REGISTRY_ENDPOINTS") {
            caps.registry_endpoints = raw
                .split(',')
                .map(str::trim)
                .filter(|endpoint| !endpoint.is_empty())
                .map(ToString::to_string)
                .collect();
        }
        caps.policy.auto_register =
            env_flag("ENTITY_VERIFY_AUTO_REGISTER", caps.policy.auto_register);
        caps
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| apex_core::env::parse_truthy_flag(&value))
        .unwrap_or(default)
}

/// Build the production, evidence-backed verifier.
///
/// Wires the real providers selected by `caps` and persists every collected
/// signal through [`PgEvidenceStore`] (`entity_verification_evidence`,
/// migration 052). The returned verifier is the **only** sanctioned way for
/// production jobs to verify company candidates; the seed-directory-only
/// constructor is explicitly named [`EntityVerifier::seed_only`].
pub fn build_production_entity_verifier(
    store: PgPool,
    http: reqwest::Client,
    caps: EntityVerifierCaps,
) -> EntityVerifier {
    let mut verifier = EntityVerifier::without_providers().with_policy(caps.policy.clone());

    if caps.gleif {
        verifier = verifier.with_provider(Box::new(GleifProvider::new(http.clone())));
    }
    if caps.sec_edgar {
        verifier = verifier.with_provider(Box::new(SecEdgarProvider::new(http.clone())));
    }
    if caps.registry_adapters {
        verifier = verifier.with_provider(Box::new(RegistryAdapterProvider::new(
            http.clone(),
            caps.registry_endpoints.clone(),
        )));
    }
    if caps.rdap {
        verifier = verifier.with_provider(Box::new(RdapProvider::new(http.clone())));
    }
    if caps.corporate_site {
        verifier = verifier.with_provider(Box::new(CorporateSiteProvider::new(http)));
    }
    if caps.independent_sources {
        verifier = verifier.with_provider(Box::new(IndependentSourceDomainsProvider::new()));
    }

    verifier.with_store(Box::new(PgEvidenceStore::new(store)))
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared parsing helpers (pure, unit-tested without network)
// ─────────────────────────────────────────────────────────────────────────────

/// Percent-encode a query component (RFC 3986 unreserved set kept as-is).
pub fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push_str("%20"),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Do a candidate name and a record's legal name refer to the same entity?
///
/// Compares normalized names for equality or whole-word containment, so
/// "Acme Corp" matches "Acme Corporation Holdings" but "Acme" alone (too
/// short) never matches.
pub fn names_match(candidate: &str, found: &str) -> bool {
    let candidate_raw = candidate.trim();
    let found_raw = found.trim();
    if candidate_raw.len() < 3 || found_raw.len() < 3 {
        return false;
    }
    // Names shorter than five characters on both sides are too generic to
    // anchor an identity (e.g. "Acme" vs "ACME").
    if candidate_raw.len() < 5 && found_raw.len() < 5 {
        return false;
    }
    let candidate = normalize_company_name(candidate_raw);
    let found = normalize_company_name(found_raw);
    if candidate.len() < 3 || found.len() < 3 {
        return false;
    }
    if candidate == found {
        return true;
    }
    let (long, short) = if candidate.len() >= found.len() {
        (candidate, found)
    } else {
        (found, candidate)
    };
    short.len() >= 4 && format!(" {long} ").contains(&format!(" {short} "))
}

/// Normalize a bare domain or URL to a lowercase host without `www.`.
pub fn normalize_domain(value: &str) -> Option<String> {
    domain_of(value).filter(|domain| domain.contains('.'))
}

/// Candidate domains taken from `metadata["domain"]` / `metadata["website"]`.
pub fn candidate_domains(candidate: &CompanyCandidate) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for key in ["domain", "website"] {
        if let Some(value) = candidate.metadata.get(key) {
            if let Some(domain) = normalize_domain(value) {
                if !domains.contains(&domain) {
                    domains.push(domain);
                }
            }
        }
    }
    domains
}

fn split_metadata_list(candidate: &CompanyCandidate, key: &str) -> Vec<String> {
    candidate
        .metadata
        .get(key)
        .map(|raw| {
            raw.split([',', ';', '\n', ' '])
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn fetch_json(http: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    let response = http.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {url}");
    }
    Ok(response.json::<serde_json::Value>().await?)
}

async fn fetch_text(http: &reqwest::Client, url: &str) -> Result<String> {
    let response = http.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {url}");
    }
    Ok(response.text().await?)
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToString::to_string))
        .unwrap_or_else(|| url.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// GLEIF
// ─────────────────────────────────────────────────────────────────────────────

/// GLEIF LEI record lookups.
pub struct GleifProvider {
    http: reqwest::Client,
    endpoint: String,
}

impl GleifProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint: GLEIF_ENDPOINT.to_string(),
        }
    }

    pub fn with_endpoint(http: reqwest::Client, endpoint: impl Into<String>) -> Self {
        Self {
            http,
            endpoint: endpoint.into(),
        }
    }
}

/// Extract matching GLEIF signals from a `lei-records` payload.
pub fn gleif_signals(payload: &serde_json::Value, candidate_name: &str) -> Vec<EvidenceSignal> {
    let Some(records) = payload.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let mut signals = Vec::new();
    for record in records {
        let legal_name = record
            .pointer("/attributes/entity/legalName/name")
            .or_else(|| record.pointer("/attributes/entity/legalName"))
            .and_then(serde_json::Value::as_str);
        let lei = record
            .get("id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                record
                    .pointer("/attributes/lei")
                    .and_then(serde_json::Value::as_str)
            });
        if let (Some(legal_name), Some(lei)) = (legal_name, lei) {
            if names_match(candidate_name, legal_name) {
                signals.push(EvidenceSignal::new(
                    VerificationType::GleifLei,
                    "GLEIF",
                    Some(format!("https://api.gleif.org/api/v1/lei-records/{lei}")),
                    legal_name,
                    0.9,
                ));
            }
        }
    }
    signals
}

#[async_trait]
impl EvidenceProvider for GleifProvider {
    fn name(&self) -> &str {
        "gleif"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let name = candidate.raw_name.trim();
        if name.len() < 3 {
            return Ok(Vec::new());
        }
        let mut url = reqwest::Url::parse(&self.endpoint)?;
        url.query_pairs_mut()
            .append_pair("filter[entity.legalName]", name)
            .append_pair("page[size]", "5");
        let payload = fetch_json(&self.http, url.as_str()).await?;
        Ok(gleif_signals(&payload, name))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SEC EDGAR
// ─────────────────────────────────────────────────────────────────────────────

/// SEC EDGAR full-text search (US filers). Skipped when the candidate is
/// explicitly tagged with a non-US `country_code`.
pub struct SecEdgarProvider {
    http: reqwest::Client,
    endpoint: String,
}

impl SecEdgarProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint: SEC_EDGAR_ENDPOINT.to_string(),
        }
    }

    pub fn with_endpoint(http: reqwest::Client, endpoint: impl Into<String>) -> Self {
        Self {
            http,
            endpoint: endpoint.into(),
        }
    }
}

/// Should EDGAR be consulted for this candidate? Only explicit non-US
/// country codes exclude it; crawl candidates without country data are
/// checked (EDGAR itself is the authority on whether a filer exists).
pub fn is_us_candidate(candidate: &CompanyCandidate) -> bool {
    match candidate
        .metadata
        .get("country_code")
        .map(|code| code.trim().to_ascii_uppercase())
    {
        Some(code) => code == "US" || code == "USA",
        None => true,
    }
}

/// Number of EDGAR hits in an `efts.sec.gov` response.
pub fn sec_edgar_hits(payload: &serde_json::Value) -> u64 {
    payload
        .pointer("/hits/total/value")
        .or_else(|| payload.pointer("/hits/total"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .unwrap_or(0)
}

#[async_trait]
impl EvidenceProvider for SecEdgarProvider {
    fn name(&self) -> &str {
        "sec_edgar"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let name = candidate.raw_name.trim();
        if name.len() < 3 || !is_us_candidate(candidate) {
            return Ok(Vec::new());
        }
        let query = encode_query_component(&format!("\"{name}\""));
        let url = format!("{}?q={}&forms=10-K,10-Q,8-K", self.endpoint, query);
        let payload = fetch_json(&self.http, &url).await?;
        let hits = sec_edgar_hits(&payload);
        if hits == 0 {
            return Ok(Vec::new());
        }
        let browse = format!(
            "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&company={}&type=10-K&owner=include&count=10",
            encode_query_component(name)
        );
        Ok(vec![EvidenceSignal::new(
            VerificationType::SecEdgar,
            "SEC EDGAR",
            Some(browse),
            name,
            0.85,
        )])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Registry adapters
// ─────────────────────────────────────────────────────────────────────────────

/// Configurable corporate-registry adapters. Each endpoint is a URL template
/// containing `{query}`; responses may follow the OpenCorporates
/// (`results.companies[].company.name`) or a generic
/// `data[].attributes.name` / `[].name` shape.
pub struct RegistryAdapterProvider {
    http: reqwest::Client,
    endpoints: Vec<String>,
}

impl RegistryAdapterProvider {
    pub fn new(http: reqwest::Client, endpoints: Vec<String>) -> Self {
        Self { http, endpoints }
    }
}

/// Extract candidate legal names from a registry JSON response.
pub fn registry_names(payload: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    for pointer in ["/results/companies", "/data", "/companies", "/results"] {
        if let Some(entries) = payload
            .pointer(pointer)
            .and_then(serde_json::Value::as_array)
        {
            for entry in entries {
                let name = entry
                    .pointer("/company/name")
                    .or_else(|| entry.pointer("/attributes/name"))
                    .or_else(|| entry.get("name"))
                    .and_then(serde_json::Value::as_str);
                if let Some(name) = name {
                    names.push(name.to_string());
                }
                if names.len() >= 5 {
                    return names;
                }
            }
        }
    }
    names
}

#[async_trait]
impl EvidenceProvider for RegistryAdapterProvider {
    fn name(&self) -> &str {
        "registry_adapter"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let query = encode_query_component(candidate.raw_name.trim());
        let mut signals = Vec::new();
        for endpoint in &self.endpoints {
            let url = endpoint.replace("{query}", &query);
            let payload = fetch_json(&self.http, &url).await?;
            for name in registry_names(&payload) {
                if names_match(&candidate.raw_name, &name) {
                    signals.push(EvidenceSignal::new(
                        VerificationType::OfficialRegistry,
                        format!("registry:{}", host_of(&url)),
                        Some(url.clone()),
                        name,
                        0.8,
                    ));
                }
            }
        }
        Ok(signals)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RDAP
// ─────────────────────────────────────────────────────────────────────────────

/// RDAP registration data for the candidate's corporate domain.
pub struct RdapProvider {
    http: reqwest::Client,
    endpoint_template: String,
}

impl RdapProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint_template: RDAP_ENDPOINT_TEMPLATE.to_string(),
        }
    }

    pub fn with_endpoint_template(http: reqwest::Client, template: impl Into<String>) -> Self {
        Self {
            http,
            endpoint_template: template.into(),
        }
    }
}

/// The `ldhName` of an RDAP domain response, lowercased.
pub fn rdap_domain_name(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("ldhName")
        .and_then(serde_json::Value::as_str)
        .map(|name| name.to_ascii_lowercase())
}

#[async_trait]
impl EvidenceProvider for RdapProvider {
    fn name(&self) -> &str {
        "rdap"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let mut signals = Vec::new();
        for domain in candidate_domains(candidate).into_iter().take(3) {
            let url = self.endpoint_template.replace("{domain}", &domain);
            let payload = fetch_json(&self.http, &url).await?;
            if let Some(ldh_name) = rdap_domain_name(&payload) {
                if ldh_name == domain || ldh_name.ends_with(&format!(".{domain}")) {
                    signals.push(EvidenceSignal::new(
                        VerificationType::Rdap,
                        "RDAP",
                        Some(url),
                        ldh_name,
                        0.8,
                    ));
                }
            }
        }
        Ok(signals)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Corporate site metadata
// ─────────────────────────────────────────────────────────────────────────────

/// Structured metadata published by the candidate's own corporate site.
pub struct CorporateSiteProvider {
    http: reqwest::Client,
    scheme: String,
}

impl CorporateSiteProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            scheme: "https".to_string(),
        }
    }

    pub fn with_scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = scheme.into();
        self
    }
}

fn meta_tag_regex() -> &'static Regex {
    static META_TAG_RE: OnceLock<Regex> = OnceLock::new();
    META_TAG_RE.get_or_init(|| {
        Regex::new(r"(?is)<meta\s[^>]*>")
            .unwrap_or_else(|error| panic!("valid meta tag regex: {error}"))
    })
}

fn jsonld_regex() -> &'static Regex {
    static JSONLD_RE: OnceLock<Regex> = OnceLock::new();
    JSONLD_RE.get_or_init(|| {
        Regex::new(r#"(?is)<script[^>]+application/ld\+json[^>]*>(.*?)</script>"#)
            .unwrap_or_else(|error| panic!("valid json-ld regex: {error}"))
    })
}

fn title_regex() -> &'static Regex {
    static TITLE_RE: OnceLock<Regex> = OnceLock::new();
    TITLE_RE.get_or_init(|| {
        Regex::new(r"(?is)<title[^>]*>(.*?)</title>")
            .unwrap_or_else(|error| panic!("valid title regex: {error}"))
    })
}

fn html_attribute(tag: &str, attribute: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attribute}=");
    let position = lower.find(&needle)? + needle.len();
    let rest = tag[position..].trim_start();
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let inner = &rest[1..];
        inner.find(quote).map(|end| inner[..end].to_string())
    } else {
        rest.split([' ', '>', '/']).next().map(ToString::to_string)
    }
}

fn jsonld_org_name(value: &serde_json::Value) -> Option<String> {
    for pointer in ["/name", "/legalName", "/publisher/name"] {
        if let Some(name) = value.pointer(pointer).and_then(serde_json::Value::as_str) {
            if !name.trim().is_empty() {
                return Some(name.trim().to_string());
            }
        }
    }
    if let Some(graph) = value.get("@graph").and_then(serde_json::Value::as_array) {
        for node in graph {
            if let Some(name) = node.get("name").and_then(serde_json::Value::as_str) {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    None
}

/// Extract the organization name a corporate page publishes about itself:
/// `og:site_name` / `application-name`, then schema.org JSON-LD, then the
/// first segment of `<title>`.
pub fn extract_corporate_site_name(html: &str) -> Option<String> {
    for tag in meta_tag_regex().find_iter(html) {
        let tag = tag.as_str();
        let lower = tag.to_ascii_lowercase();
        if lower.contains("og:site_name") || lower.contains("application-name") {
            if let Some(content) = html_attribute(tag, "content") {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    for captures in jsonld_regex().captures_iter(html) {
        if let Some(raw) = captures.get(1) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.as_str()) {
                if let Some(name) = jsonld_org_name(&value) {
                    return Some(name);
                }
            }
        }
    }

    if let Some(captures) = title_regex().captures(html) {
        if let Some(title) = captures.get(1) {
            let title = title.as_str().trim();
            let name = title
                .split(['|', '–', '—'])
                .next()
                .unwrap_or(title)
                .split(" - ")
                .next()
                .unwrap_or(title)
                .trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    None
}

#[async_trait]
impl EvidenceProvider for CorporateSiteProvider {
    fn name(&self) -> &str {
        "corporate_site"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let mut signals = Vec::new();
        for domain in candidate_domains(candidate).into_iter().take(2) {
            let url = format!("{}://{domain}/", self.scheme);
            let html = match fetch_text(&self.http, &url).await {
                Ok(html) => html,
                Err(error) => {
                    tracing::debug!(domain = %domain, error = %error, "corporate site fetch failed");
                    continue;
                }
            };
            let Some(site_name) = extract_corporate_site_name(&html) else {
                continue;
            };
            if names_match(&candidate.raw_name, &site_name) {
                signals.push(EvidenceSignal::new(
                    VerificationType::CorporateSiteMetadata,
                    "corporate site",
                    Some(url),
                    site_name,
                    0.7,
                ));
            }
        }
        Ok(signals)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Independent source domains
// ─────────────────────────────────────────────────────────────────────────────

/// Corroboration from ≥ 2 distinct source domains already observed on the
/// candidate (`metadata["source_domains"]` / `metadata["source_urls"]`).
/// Emits nothing when fewer than two real domains are recorded — evidence is
/// never fabricated from a single mention.
pub struct IndependentSourceDomainsProvider {
    min_domains: usize,
}

impl IndependentSourceDomainsProvider {
    pub fn new() -> Self {
        Self { min_domains: 2 }
    }

    pub fn with_min_domains(mut self, min_domains: usize) -> Self {
        self.min_domains = min_domains.max(2);
        self
    }
}

impl Default for IndependentSourceDomainsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EvidenceProvider for IndependentSourceDomainsProvider {
    fn name(&self) -> &str {
        "independent_sources"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let mut domains: BTreeSet<String> = BTreeSet::new();
        for value in split_metadata_list(candidate, "source_domains") {
            if let Some(domain) = normalize_domain(&value) {
                domains.insert(domain);
            }
        }
        for value in split_metadata_list(candidate, "source_urls") {
            if let Some(domain) = normalize_domain(&value) {
                domains.insert(domain);
            }
        }
        if domains.len() < self.min_domains {
            return Ok(Vec::new());
        }
        let matched = domains.iter().cloned().collect::<Vec<_>>().join(", ");
        Ok(vec![EvidenceSignal::new(
            VerificationType::IndependentSourceDomains,
            "independent crawl source domains",
            None::<String>,
            matched,
            0.75,
        )])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company_discovery::{normalize_company_name, DiscoverySource};
    use std::collections::HashMap;

    fn candidate(name: &str, metadata: &[(&str, &str)]) -> CompanyCandidate {
        let mut map = HashMap::new();
        for (key, value) in metadata {
            map.insert(key.to_string(), value.to_string());
        }
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: normalize_company_name(name),
            source: DiscoverySource::WebCrawl,
            extraction_confidence: 0.8,
            context_snippet: String::new(),
            metadata: map,
        }
    }

    fn lazy_pool() -> PgPool {
        match sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://apex:apex@127.0.0.1:5432/apex_entity_admission_test")
        {
            Ok(pool) => pool,
            Err(error) => panic!("lazy pool construction must not connect: {error}"),
        }
    }

    #[tokio::test]
    async fn production_builder_wires_non_seed_providers() {
        let verifier = build_production_entity_verifier(
            lazy_pool(),
            reqwest::Client::new(),
            EntityVerifierCaps::all_enabled(),
        );
        let names = verifier.provider_names();
        for expected in [
            "gleif",
            "sec_edgar",
            "registry_adapter",
            "rdap",
            "corporate_site",
            "independent_sources",
        ] {
            assert!(
                names.contains(&expected),
                "production verifier is missing provider {expected}: {names:?}"
            );
        }
        assert!(
            !names.contains(&"seed_directory"),
            "production verifier must not be seed-only: {names:?}"
        );
    }

    #[tokio::test]
    async fn production_builder_respects_capability_switches() {
        let caps = EntityVerifierCaps {
            gleif: false,
            sec_edgar: false,
            registry_adapters: false,
            rdap: false,
            corporate_site: false,
            independent_sources: false,
            ..EntityVerifierCaps::default()
        };
        let verifier = build_production_entity_verifier(lazy_pool(), reqwest::Client::new(), caps);
        assert!(verifier.provider_names().is_empty());
    }

    #[test]
    fn names_match_requires_close_identity() {
        assert!(names_match("Acme Corp", "Acme Corporation"));
        assert!(names_match("Acme Corp", "ACME CORP HOLDINGS"));
        assert!(!names_match("Acme Corp", "Zenith Holdings"));
        assert!(!names_match("Acme", "ACME"));
    }

    #[test]
    fn gleif_parser_keeps_only_matching_records() {
        let payload = serde_json::json!({
            "data": [
                {
                    "id": "529900T8BM49AURSDO55",
                    "attributes": { "entity": { "legalName": { "name": "Acme Corporation" } } }
                },
                {
                    "id": "5493001KJTIIGC8Y1R12",
                    "attributes": { "entity": { "legalName": { "name": "Zenith Holdings GmbH" } } }
                }
            ]
        });
        let signals = gleif_signals(&payload, "Acme Corp");
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].matched_value, "Acme Corporation");
        assert_eq!(signals[0].verification_type, VerificationType::GleifLei);
    }

    #[test]
    fn sec_edgar_parser_reads_hit_counts() {
        let payload = serde_json::json!({ "hits": { "total": { "value": 4 } } });
        assert_eq!(sec_edgar_hits(&payload), 4);
        let string_payload = serde_json::json!({ "hits": { "total": "7" } });
        assert_eq!(sec_edgar_hits(&string_payload), 7);
        assert_eq!(sec_edgar_hits(&serde_json::json!({})), 0);
    }

    #[test]
    fn sec_edgar_skips_explicitly_non_us_candidates() {
        let de = candidate("Siemens AG", &[("country_code", "DE")]);
        let us = candidate("Acme Corp", &[("country_code", "US")]);
        let unknown = candidate("Acme Corp", &[]);
        assert!(!is_us_candidate(&de));
        assert!(is_us_candidate(&us));
        assert!(is_us_candidate(&unknown));
    }

    #[test]
    fn registry_parser_handles_opencorporates_and_generic_shapes() {
        let opencorporates = serde_json::json!({
            "results": { "companies": [ { "company": { "name": "Acme Corp" } } ] }
        });
        assert_eq!(
            registry_names(&opencorporates),
            vec!["Acme Corp".to_string()]
        );

        let generic = serde_json::json!({
            "data": [ { "attributes": { "name": "Zenith GmbH" } } ]
        });
        assert_eq!(registry_names(&generic), vec!["Zenith GmbH".to_string()]);
        assert!(registry_names(&serde_json::json!({ "ok": true })).is_empty());
    }

    #[test]
    fn corporate_site_extracts_structured_names() {
        let og = r#"<html><head><meta property="og:site_name" content="Acme Corp"/></head></html>"#;
        assert_eq!(
            extract_corporate_site_name(og).as_deref(),
            Some("Acme Corp")
        );

        let jsonld = r#"<html><head><script type="application/ld+json">{"@type":"Organization","legalName":"Acme Corporation"}</script></head></html>"#;
        assert_eq!(
            extract_corporate_site_name(jsonld).as_deref(),
            Some("Acme Corporation")
        );

        let title = "<html><head><title>Acme Corp | Supply Chain</title></head></html>";
        assert_eq!(
            extract_corporate_site_name(title).as_deref(),
            Some("Acme Corp")
        );
        assert_eq!(extract_corporate_site_name("<html></html>"), None);
    }

    #[tokio::test]
    async fn independent_sources_needs_two_real_domains() {
        let provider = IndependentSourceDomainsProvider::new();
        let single = candidate(
            "Acme Corp",
            &[("source_url", "https://news-a.example/story")],
        );
        let single_signals = match provider.collect(&single).await {
            Ok(signals) => signals,
            Err(error) => panic!("pure provider must not fail: {error}"),
        };
        assert!(
            single_signals.is_empty(),
            "a single source must not corroborate itself"
        );

        let multi = candidate(
            "Acme Corp",
            &[("source_domains", "news-a.example, news-b.example")],
        );
        let multi_signals = match provider.collect(&multi).await {
            Ok(signals) => signals,
            Err(error) => panic!("pure provider must not fail: {error}"),
        };
        assert_eq!(multi_signals.len(), 1);
        assert_eq!(
            multi_signals[0].verification_type,
            VerificationType::IndependentSourceDomains
        );
        assert!(multi_signals[0].matched_value.contains("news-a.example"));
    }

    #[test]
    fn domain_helpers_normalize_urls_and_bare_domains() {
        assert_eq!(
            normalize_domain("https://www.Acme.example/path").as_deref(),
            Some("acme.example")
        );
        assert_eq!(
            normalize_domain("acme.example").as_deref(),
            Some("acme.example")
        );
        assert_eq!(normalize_domain("not a domain"), None);
        let with_metadata = candidate(
            "Acme Corp",
            &[
                ("website", "https://www.acme.example"),
                ("domain", "acme.example"),
            ],
        );
        assert_eq!(
            candidate_domains(&with_metadata),
            vec!["acme.example".to_string()]
        );
    }

    #[test]
    fn query_encoding_escapes_reserved_characters() {
        assert_eq!(encode_query_component("Acme Corp"), "Acme%20Corp");
        assert_eq!(encode_query_component("a&b"), "a%26b");
    }
}
