//! Multidimensional source-coverage readiness (audit P1-9).
//!
//! The old readiness gate compared the deployment's total operational source
//! count against a single minimum. That number is blind to *what* the sources
//! cover: 30 operational news feeds satisfy it while procurement, patents and
//! hiring have zero. This module replaces it with a policy-driven matrix over
//! capability families. Every family declares its own requirement:
//!
//! * whether the family is required at all,
//! * minimum operational sources,
//! * maximum age of the newest successful fetch (freshness),
//! * minimum successful-fetch ratio and parser-success ratio,
//! * minimum independent domains,
//!
//! plus a deployment-level priority-company coverage dimension. The resolved
//! matrix is published in the `source_coverage` capability detail so an
//! operator can see exactly which family failed and why.
//!
//! The aggregate evidence base is summarised through the shared
//! [`apex_core::evidence_quality::EvidenceQuality`] model so the same
//! semantics drive coverage reporting and the intelligence features.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::acquisition::AcquisitionDisposition;
use crate::sources_registry::{
    effective_capability, source_is_validated, Category, DeploymentCapabilities, Source,
    SourceCapability,
};
use apex_core::config::ConfigErrors;
use apex_core::evidence_quality::{
    assess_evidence_quality_for_claim, EvidenceItem, EvidenceQuality, EvidenceStance,
};
use apex_store::postgres::SourceRuntimeStateRow;

/// Minimum number of attempted sources before a family's fetch/parser ratios
/// count as a measurement. Below this the family is degraded regardless of the
/// ratio: a single successful fetch out of one attempt is not production
/// evidence, and an unmeasured family must never satisfy a production gate.
pub const MIN_COVERAGE_SAMPLE: usize = 5;

/// Capability families the coverage matrix reasons about. These are the
/// intel domains the product promises to cover, independent of how the
/// registry happens to tag sources.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum CoverageFamily {
    #[default]
    Procurement,
    Patents,
    Regulatory,
    Hiring,
    Financial,
    Tenders,
    Certifications,
    ExecutiveChanges,
    TradeCustoms,
    ProductCompetitive,
    SupplyChainFactories,
}

impl CoverageFamily {
    pub const ALL: [Self; 11] = [
        Self::Procurement,
        Self::Patents,
        Self::Regulatory,
        Self::Hiring,
        Self::Financial,
        Self::Tenders,
        Self::Certifications,
        Self::ExecutiveChanges,
        Self::TradeCustoms,
        Self::ProductCompetitive,
        Self::SupplyChainFactories,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Procurement => "procurement",
            Self::Patents => "patents",
            Self::Regulatory => "regulatory",
            Self::Hiring => "hiring",
            Self::Financial => "financial",
            Self::Tenders => "tenders",
            Self::Certifications => "certifications",
            Self::ExecutiveChanges => "executive_changes",
            Self::TradeCustoms => "trade_customs",
            Self::ProductCompetitive => "product_competitive",
            Self::SupplyChainFactories => "supply_chain_factories",
        }
    }

    /// Upper-case token used in environment variable names.
    fn env_token(self) -> &'static str {
        match self {
            Self::Procurement => "PROCUREMENT",
            Self::Patents => "PATENTS",
            Self::Regulatory => "REGULATORY",
            Self::Hiring => "HIRING",
            Self::Financial => "FINANCIAL",
            Self::Tenders => "TENDERS",
            Self::Certifications => "CERTIFICATIONS",
            Self::ExecutiveChanges => "EXECUTIVE_CHANGES",
            Self::TradeCustoms => "TRADE_CUSTOMS",
            Self::ProductCompetitive => "PRODUCT_COMPETITIVE",
            Self::SupplyChainFactories => "SUPPLY_CHAIN_FACTORIES",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.as_str() == value.trim().to_ascii_lowercase())
    }
}

impl std::fmt::Display for CoverageFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Keyword augmentations applied on top of the source's declared category.
const FAMILY_KEYWORDS: &[(CoverageFamily, &[&str])] = &[
    (
        CoverageFamily::Tenders,
        &["tender", "contract notice", "award notice"],
    ),
    (
        CoverageFamily::Hiring,
        &[
            "hiring",
            "job board",
            "job_board",
            "jobs",
            "careers",
            "recruit",
        ],
    ),
    (
        CoverageFamily::Certifications,
        &[
            // Certification-information registers only. NIST/NVD are
            // vulnerability feeds, not certification registers, so "nist" is
            // deliberately absent: with certifications now required, a
            // keyword match on NVD must not satisfy the family.
            "certification",
            "accredit",
            " iso",
            "iso-",
            "iso_",
            "ansi",
        ],
    ),
    (
        CoverageFamily::ExecutiveChanges,
        &[
            "executive",
            "appointment",
            "leadership",
            "ceo",
            "cfo",
            "board",
            "insider",
        ],
    ),
    (
        CoverageFamily::TradeCustoms,
        &[
            "customs",
            "tariff",
            "panjiva",
            "importgenius",
            "import genius",
            "comtrade",
            "trademap",
        ],
    ),
    (
        CoverageFamily::SupplyChainFactories,
        &[
            "factory",
            "factories",
            "manufactur",
            "assembly",
            "production line",
        ],
    ),
    (
        CoverageFamily::ProductCompetitive,
        &["product", "competitive", "competitor"],
    ),
    (
        CoverageFamily::Regulatory,
        &["regulat", "compliance", "sanction"],
    ),
];

impl Source {
    /// Capability families this source contributes to. Category assignment is
    /// authoritative; keywords extend it so a hiring board registered under
    /// `Technology` still counts as hiring coverage.
    pub fn coverage_families(&self) -> BTreeSet<CoverageFamily> {
        let mut families = BTreeSet::new();
        match self.category {
            Category::Procurement => {
                families.insert(CoverageFamily::Procurement);
            }
            Category::Patents => {
                families.insert(CoverageFamily::Patents);
            }
            Category::LegalRegulatory | Category::Sanctions => {
                families.insert(CoverageFamily::Regulatory);
            }
            Category::Finance => {
                families.insert(CoverageFamily::Financial);
            }
            Category::Trade => {
                families.insert(CoverageFamily::TradeCustoms);
            }
            Category::SupplyChain => {
                families.insert(CoverageFamily::SupplyChainFactories);
            }
            Category::Technology => {
                families.insert(CoverageFamily::ProductCompetitive);
            }
            Category::GovernmentRegistry => {
                families.insert(CoverageFamily::ExecutiveChanges);
                families.insert(CoverageFamily::Regulatory);
            }
            Category::CertificationRegistry => {
                families.insert(CoverageFamily::Certifications);
            }
            _ => {}
        }

        let haystack = format!(
            "{} {} {} {}",
            self.slug,
            self.name,
            self.url,
            self.notes.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        for (family, needles) in FAMILY_KEYWORDS {
            if needles.iter().any(|needle| haystack.contains(needle)) {
                families.insert(*family);
            }
        }
        families
    }
}

/// One family's requirement row in the coverage matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageFamilyRequirement {
    pub family: CoverageFamily,
    pub required: bool,
    pub min_operational_sources: usize,
    pub max_freshness_age_secs: i64,
    /// Minimum successful-fetch ratio, in percent.
    pub min_fetch_success_pct: u8,
    /// Minimum parser-success ratio, in percent.
    pub min_parser_success_pct: u8,
    pub min_independent_domains: usize,
}

impl CoverageFamilyRequirement {
    fn row(family: CoverageFamily, required: bool, min_operational_sources: usize) -> Self {
        Self {
            family,
            required,
            min_operational_sources,
            max_freshness_age_secs: 7 * 24 * 60 * 60,
            min_fetch_success_pct: 50,
            min_parser_success_pct: 50,
            min_independent_domains: 1,
        }
    }
}

/// The full policy-driven coverage matrix, published in readiness responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveragePolicy {
    pub families: Vec<CoverageFamilyRequirement>,
    /// Minimum share (percent) of priority companies with recent observations.
    pub min_priority_company_coverage_pct: u8,
    /// Window in which a priority company observation counts as coverage.
    pub priority_company_window_secs: i64,
}

impl Default for CoveragePolicy {
    fn default() -> Self {
        use CoverageFamily::*;
        Self {
            families: vec![
                CoverageFamilyRequirement::row(Procurement, true, 3),
                CoverageFamilyRequirement::row(Patents, true, 1),
                CoverageFamilyRequirement::row(Regulatory, true, 2),
                CoverageFamilyRequirement::row(Hiring, true, 1),
                CoverageFamilyRequirement::row(Financial, true, 3),
                CoverageFamilyRequirement::row(Tenders, true, 1),
                // Certification-information sources now exist (IAF CertSearch,
                // IAQG OASIS, ANAB, UKAS, openFDA device registration), so the
                // family is required: a deployment whose certification sources
                // are not operational cannot claim certification coverage.
                CoverageFamilyRequirement::row(Certifications, true, 1),
                CoverageFamilyRequirement::row(ExecutiveChanges, true, 1),
                CoverageFamilyRequirement::row(TradeCustoms, true, 2),
                CoverageFamilyRequirement::row(ProductCompetitive, true, 2),
                CoverageFamilyRequirement::row(SupplyChainFactories, true, 2),
            ],
            min_priority_company_coverage_pct: 50,
            priority_company_window_secs: 30 * 24 * 60 * 60,
        }
    }
}

impl CoveragePolicy {
    pub fn requirement(&self, family: CoverageFamily) -> Option<&CoverageFamilyRequirement> {
        self.families.iter().find(|row| row.family == family)
    }

    /// Resolve per-family overrides from `APEX_COVERAGE_*`.
    ///
    /// Absent variables keep their default. A present-but-malformed value
    /// (for example `APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT=banana`) is a
    /// configuration error: silently substituting the default would change a
    /// production readiness gate without telling the operator. All errors are
    /// collected so one resolution pass reports every problem at once.
    pub fn from_env() -> std::result::Result<Self, ConfigErrors> {
        let mut errors = ConfigErrors::new();
        let mut policy = Self::default();
        for row in &mut policy.families {
            let token = row.family.env_token();
            row.required = env_bool(
                &format!("APEX_COVERAGE_{token}_REQUIRED"),
                row.required,
                &mut errors,
            );
            row.min_operational_sources = env_usize(
                &format!("APEX_COVERAGE_{token}_MIN"),
                row.min_operational_sources,
                &mut errors,
            );
            row.max_freshness_age_secs = env_i64(
                &format!("APEX_COVERAGE_{token}_FRESHNESS_SECS"),
                row.max_freshness_age_secs,
                &mut errors,
            );
            row.min_fetch_success_pct = env_percentage(
                &format!("APEX_COVERAGE_{token}_FETCH_SUCCESS_PCT"),
                row.min_fetch_success_pct,
                &mut errors,
            );
            row.min_parser_success_pct = env_percentage(
                &format!("APEX_COVERAGE_{token}_PARSER_SUCCESS_PCT"),
                row.min_parser_success_pct,
                &mut errors,
            );
            row.min_independent_domains = env_usize(
                &format!("APEX_COVERAGE_{token}_MIN_DOMAINS"),
                row.min_independent_domains,
                &mut errors,
            );
        }
        policy.min_priority_company_coverage_pct = env_percentage(
            "APEX_COVERAGE_PRIORITY_COMPANY_PCT",
            policy.min_priority_company_coverage_pct,
            &mut errors,
        );
        policy.priority_company_window_secs = env_i64(
            "APEX_COVERAGE_PRIORITY_COMPANY_WINDOW_SECS",
            policy.priority_company_window_secs,
            &mut errors,
        );
        errors.into_result().map(|()| policy)
    }
}

fn env_bool(name: &str, default: bool, errors: &mut ConfigErrors) -> bool {
    match std::env::var(name) {
        // A present-but-blank value is treated as unset: placeholder lines in
        // .env/compose files must not abort startup.
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            other => {
                errors.push(name, other, "one of true/false/1/0/yes/no/on/off");
                default
            }
        },
    }
}

fn env_i64(name: &str, default: i64, errors: &mut ConfigErrors) -> i64 {
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                errors.push(name, raw.trim(), "an integer");
                default
            }
        },
    }
}

fn env_percentage(name: &str, default: u8, errors: &mut ConfigErrors) -> u8 {
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<u8>() {
            Ok(value) if value <= 100 => value,
            Ok(value) => {
                errors.push(name, value.to_string(), "an integer percentage in 0..=100");
                default
            }
            Err(_) => {
                errors.push(name, raw.trim(), "an integer percentage in 0..=100");
                default
            }
        },
    }
}

fn env_usize(name: &str, default: usize, errors: &mut ConfigErrors) -> usize {
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) if raw.trim().is_empty() => default,
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(value) => value,
            Err(_) => {
                errors.push(name, raw.trim(), "a non-negative integer");
                default
            }
        },
    }
}

/// Per-family snapshot of the source universe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FamilyCoverage {
    pub family: CoverageFamily,
    pub declared: usize,
    pub registered: usize,
    pub operational: usize,
    pub validated: usize,
    pub temporarily_degraded: usize,
    pub never_crawled: usize,
    /// Distinct domains among operational sources.
    pub independent_domains: usize,
    /// Sources with at least one recorded attempt.
    pub attempted: usize,
    /// Share (percent) of attempted sources whose last attempt succeeded
    /// (fetch + parser contract); `None` when nothing was attempted.
    pub parser_success_pct: Option<u8>,
    /// Mean rolling fetch-success rate (percent) over attempted sources.
    pub fetch_success_pct: Option<u8>,
    pub latest_success_at: Option<DateTime<Utc>>,
    /// Sources whose most recent persisted outcome was a rate limit (429).
    /// They are never counted as operational successes.
    pub rate_limited: usize,
    /// Sources blocked because their adapter requires credentials this
    /// deployment does not have.
    pub authentication_blocked: usize,
    /// Sources the deployment cannot execute at all (missing capability,
    /// missing proxy, adapter unavailable).
    pub unavailable: usize,
}

/// Deployment-level priority-company coverage, computed from the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PriorityCompanyCoverage {
    pub total: usize,
    pub covered: usize,
}

/// One family's evaluated status against its requirement row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageFamilyEvaluation {
    pub family: CoverageFamily,
    pub required: bool,
    /// `ok` | `degraded` | `not_required`.
    pub status: String,
    pub reasons: Vec<String>,
    pub operational: usize,
    pub min_operational_sources: usize,
    /// Sources with at least one recorded attempt.
    pub attempted: usize,
    /// Minimum attempts before the fetch/parser ratios are a measurement.
    pub min_coverage_sample: usize,
    pub independent_domains: usize,
    pub min_independent_domains: usize,
    pub freshness_age_secs: Option<i64>,
    pub max_freshness_age_secs: i64,
    pub fetch_success_pct: Option<u8>,
    pub min_fetch_success_pct: u8,
    pub parser_success_pct: Option<u8>,
    pub min_parser_success_pct: u8,
    /// Sources whose last persisted outcome was a rate limit.
    pub rate_limited: usize,
    /// Sources blocked on adapter credentials.
    pub authentication_blocked: usize,
    /// Sources the deployment cannot execute.
    pub unavailable: usize,
}

/// Full coverage evaluation published in the `source_coverage` capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCoverageReport {
    /// `ok` | `degraded`.
    pub status: String,
    pub detail: String,
    pub families: Vec<CoverageFamilyEvaluation>,
    pub required_families: usize,
    pub satisfied_required_families: usize,
    pub priority_companies_total: usize,
    pub priority_companies_covered: usize,
    pub priority_company_pct: Option<u8>,
    pub min_priority_company_coverage_pct: u8,
    /// Shared evidence-quality summary of the operational evidence base.
    pub evidence_quality: EvidenceQuality,
}

/// Classify one source using exactly the rules of
/// [`crate::sources_registry::source_coverage_summary`], so the family
/// breakdown can never disagree with the aggregate metric.
fn classify_source(
    source: &Source,
    runtime: Option<&SourceRuntimeStateRow>,
    deployment_caps: &DeploymentCapabilities,
    now: DateTime<Utc>,
) -> (bool, bool) {
    let effective = effective_capability(source, runtime, deployment_caps);
    let validated = source_is_validated(runtime)
        && matches!(
            effective,
            SourceCapability::Operational | SourceCapability::TemporarilyFailed
        );
    let degraded = runtime
        .map(|row| {
            row.consecutive_failures > 0
                || row
                    .circuit_open_until
                    .map(|until| until > now)
                    .unwrap_or(false)
        })
        .unwrap_or(false);
    let operational = validated && !(degraded || effective == SourceCapability::TemporarilyFailed);
    (validated, operational)
}

fn pct(numerator: usize, denominator: usize) -> u8 {
    if denominator == 0 {
        return 0;
    }
    ((numerator as f64 * 100.0 / denominator as f64).round()).clamp(0.0, 100.0) as u8
}

/// Classify a persisted runtime state by its most recent outcome.
///
/// The store records failure kinds as `last_error` prefixes
/// (`rate_limited:`, `authentication_required:`, `unavailable:`,
/// `parser_failure:`), so the coverage/readiness matrix can distinguish a 429
/// or an authentication gap from a transport failure without re-fetching.
pub fn persisted_outcome_disposition(row: &SourceRuntimeStateRow) -> AcquisitionDisposition {
    let error = row.last_error.as_deref().unwrap_or("");
    if error.starts_with("rate_limited") {
        return AcquisitionDisposition::RateLimited;
    }
    if error.starts_with("authentication_required") {
        return AcquisitionDisposition::AuthenticationBlocked;
    }
    if error.starts_with("unavailable") {
        return AcquisitionDisposition::Unavailable;
    }
    if error.starts_with("parser_failure") {
        return AcquisitionDisposition::Degraded;
    }
    match (row.last_success_at, row.last_attempt_at) {
        (Some(success), Some(attempt)) if success >= attempt => AcquisitionDisposition::Operational,
        (Some(_), None) => AcquisitionDisposition::Operational,
        (_, Some(_)) => AcquisitionDisposition::Degraded,
        _ => AcquisitionDisposition::NotApplicable,
    }
}

/// Per-family coverage snapshot for the given registry + runtime state.
pub fn family_coverage(
    sources: &[Source],
    states: &[SourceRuntimeStateRow],
    deployment_caps: &DeploymentCapabilities,
    now: DateTime<Utc>,
) -> Vec<FamilyCoverage> {
    let state_by_slug: HashMap<&str, &SourceRuntimeStateRow> = states
        .iter()
        .map(|row| (row.source_slug.as_str(), row))
        .collect();

    let mut by_family: HashMap<CoverageFamily, FamilyCoverage> = CoverageFamily::ALL
        .into_iter()
        .map(|family| {
            (
                family,
                FamilyCoverage {
                    family,
                    ..FamilyCoverage::default()
                },
            )
        })
        .collect();

    for source in sources {
        let families = source.coverage_families();
        if families.is_empty() {
            continue;
        }
        let runtime = state_by_slug.get(source.slug.as_str()).copied();
        let (validated, operational) = classify_source(source, runtime, deployment_caps, now);
        let effective = effective_capability(source, runtime, deployment_caps);
        let disposition = runtime
            .map(persisted_outcome_disposition)
            .unwrap_or(AcquisitionDisposition::NotApplicable);
        let authentication_blocked = effective == SourceCapability::UnavailableMissingCredentials
            || disposition == AcquisitionDisposition::AuthenticationBlocked;
        let unavailable = matches!(
            effective,
            SourceCapability::UnavailableMissingCapability
                | SourceCapability::UnavailableMissingProxy
        ) || disposition == AcquisitionDisposition::Unavailable;
        for family in families {
            let Some(entry) = by_family.get_mut(&family) else {
                continue;
            };
            entry.declared += 1;
            if !source.enabled {
                continue;
            }
            entry.registered += 1;
            if authentication_blocked {
                entry.authentication_blocked += 1;
            }
            if unavailable {
                entry.unavailable += 1;
            }
            if disposition == AcquisitionDisposition::RateLimited {
                entry.rate_limited += 1;
            }
            if operational {
                entry.operational += 1;
            } else if validated {
                entry.temporarily_degraded += 1;
            } else if !(authentication_blocked
                || unavailable
                || disposition == AcquisitionDisposition::RateLimited)
            {
                entry.never_crawled += 1;
            }
        }
    }

    // Second pass for per-source measurements (attempts, ratios, freshness,
    // domains) so each family aggregates its own source subset.
    let mut parser_success: HashMap<CoverageFamily, (usize, usize)> = HashMap::new();
    let mut fetch_success: HashMap<CoverageFamily, (f64, usize)> = HashMap::new();
    let mut domains: HashMap<CoverageFamily, BTreeSet<String>> = HashMap::new();
    for source in sources {
        let families = source.coverage_families();
        if families.is_empty() || !source.enabled {
            continue;
        }
        let runtime = state_by_slug.get(source.slug.as_str()).copied();
        let (_, operational) = classify_source(source, runtime, deployment_caps, now);
        if !operational {
            continue;
        }
        if let Some(domain) = source.domain() {
            for family in &families {
                domains.entry(*family).or_default().insert(domain.clone());
            }
        }
        let Some(runtime) = runtime else {
            continue;
        };
        let Some(attempted_at) = runtime.last_attempt_at else {
            continue;
        };
        let success = runtime
            .last_success_at
            .map(|success_at| success_at >= attempted_at)
            .unwrap_or(false);
        let success_at = runtime.last_success_at.unwrap_or(attempted_at);
        let rate = runtime.rolling_success_rate.unwrap_or(0.0).clamp(0.0, 1.0);
        for family in &families {
            let entry = parser_success.entry(*family).or_insert((0, 0));
            entry.0 += usize::from(success);
            entry.1 += 1;
            let fetch = fetch_success.entry(*family).or_insert((0.0, 0));
            fetch.0 += rate;
            fetch.1 += 1;
            if success {
                if let Some(entry) = by_family.get_mut(family) {
                    entry.latest_success_at = Some(
                        entry
                            .latest_success_at
                            .map(|current| current.max(success_at))
                            .unwrap_or(success_at),
                    );
                }
            }
        }
    }

    let mut result: Vec<FamilyCoverage> = CoverageFamily::ALL
        .into_iter()
        .filter_map(|family| by_family.remove(&family))
        .collect();
    for entry in &mut result {
        entry.independent_domains = domains.get(&entry.family).map(BTreeSet::len).unwrap_or(0);
        if let Some((successes, attempts)) = parser_success.get(&entry.family) {
            entry.attempted = *attempts;
            entry.parser_success_pct = Some(pct(*successes, *attempts));
        }
        if let Some((total, count)) = fetch_success.get(&entry.family) {
            if *count > 0 {
                entry.fetch_success_pct =
                    Some((((total / *count as f64) * 100.0).round()).clamp(0.0, 100.0) as u8);
            }
        }
    }
    result
}

/// Evaluate the coverage matrix. A required family with zero operational
/// sources fails regardless of the deployment-wide total.
pub fn evaluate_coverage(
    summary: &crate::sources_registry::SourceCoverageSummary,
    policy: &CoveragePolicy,
    priority_companies: PriorityCompanyCoverage,
    now: DateTime<Utc>,
) -> SourceCoverageReport {
    let by_family: HashMap<CoverageFamily, &FamilyCoverage> = summary
        .families
        .iter()
        .map(|entry| (entry.family, entry))
        .collect();

    let mut evaluations = Vec::with_capacity(policy.families.len());
    let mut required_families = 0usize;
    let mut satisfied_required_families = 0usize;

    for requirement in &policy.families {
        let snapshot = by_family.get(&requirement.family).copied();
        let mut reasons = Vec::new();
        let mut status = if requirement.required {
            required_families += 1;
            "ok"
        } else {
            "not_required"
        };

        let (
            operational,
            independent_domains,
            attempted,
            freshness_age_secs,
            fetch_success_pct,
            parser_success_pct,
            latest_success_at,
            rate_limited,
            authentication_blocked,
            unavailable,
        ) = match snapshot {
            Some(snapshot) => (
                snapshot.operational,
                snapshot.independent_domains,
                snapshot.attempted,
                snapshot
                    .latest_success_at
                    .map(|latest| (now - latest).num_seconds().max(0)),
                snapshot.fetch_success_pct,
                snapshot.parser_success_pct,
                snapshot.latest_success_at,
                snapshot.rate_limited,
                snapshot.authentication_blocked,
                snapshot.unavailable,
            ),
            None => (0, 0, 0, None, None, None, None, 0, 0, 0),
        };

        if requirement.required {
            if snapshot.is_none() {
                reasons.push(format!(
                    "no registered source maps to the '{}' family",
                    requirement.family
                ));
            }
            if operational < requirement.min_operational_sources {
                reasons.push(format!(
                    "{} operational sources, minimum {}",
                    operational, requirement.min_operational_sources
                ));
            }
            // A family whose fetch/parser ratios rest on fewer than
            // `MIN_COVERAGE_SAMPLE` attempts is unmeasured, not healthy: an
            // unmeasured family can never satisfy the production readiness
            // gate. (Families with no operational source already fail on the
            // operational count; this reason targets families that look
            // present but have no measured fetch/parser history.)
            if operational > 0 && attempted < MIN_COVERAGE_SAMPLE {
                reasons.push(format!(
                    "{} attempted source(s), minimum coverage sample {}",
                    attempted, MIN_COVERAGE_SAMPLE
                ));
            }
            if independent_domains < requirement.min_independent_domains {
                reasons.push(format!(
                    "{} independent domains, minimum {}",
                    independent_domains, requirement.min_independent_domains
                ));
            }
            if authentication_blocked > 0 {
                reasons.push(format!(
                    "{} source(s) blocked on adapter credentials",
                    authentication_blocked
                ));
            }
            if rate_limited > 0 {
                reasons.push(format!(
                    "{} source(s) rate limited by upstream",
                    rate_limited
                ));
            }
            if unavailable > 0 {
                reasons.push(format!(
                    "{} source(s) unavailable in this deployment",
                    unavailable
                ));
            }
            if operational > 0 {
                match (latest_success_at, requirement.max_freshness_age_secs) {
                    (Some(latest), max_age) => {
                        let age = (now - latest).num_seconds().max(0);
                        if age > max_age {
                            reasons.push(format!(
                                "newest successful fetch is {}s old, maximum {}s",
                                age, max_age
                            ));
                        }
                    }
                    (None, _) => reasons
                        .push("no successful fetch recorded for a required family".to_string()),
                }
            }
            if let Some(fetch_pct) = fetch_success_pct {
                if fetch_pct < requirement.min_fetch_success_pct {
                    reasons.push(format!(
                        "successful-fetch ratio {}%, minimum {}%",
                        fetch_pct, requirement.min_fetch_success_pct
                    ));
                }
            }
            if let Some(parser_pct) = parser_success_pct {
                if parser_pct < requirement.min_parser_success_pct {
                    reasons.push(format!(
                        "parser-success ratio {}%, minimum {}%",
                        parser_pct, requirement.min_parser_success_pct
                    ));
                }
            }
            if reasons.is_empty() {
                satisfied_required_families += 1;
            } else {
                status = "degraded";
            }
        }

        evaluations.push(CoverageFamilyEvaluation {
            family: requirement.family,
            required: requirement.required,
            status: status.to_string(),
            reasons,
            operational,
            min_operational_sources: requirement.min_operational_sources,
            attempted,
            min_coverage_sample: MIN_COVERAGE_SAMPLE,
            independent_domains,
            min_independent_domains: requirement.min_independent_domains,
            freshness_age_secs,
            max_freshness_age_secs: requirement.max_freshness_age_secs,
            fetch_success_pct,
            min_fetch_success_pct: requirement.min_fetch_success_pct,
            parser_success_pct,
            min_parser_success_pct: requirement.min_parser_success_pct,
            rate_limited,
            authentication_blocked,
            unavailable,
        });
    }

    let failing: Vec<&CoverageFamilyEvaluation> = evaluations
        .iter()
        .filter(|evaluation| evaluation.status == "degraded")
        .collect();

    let priority_company_pct = if priority_companies.total == 0 {
        None
    } else {
        Some(pct(priority_companies.covered, priority_companies.total))
    };
    let priority_company_degraded = priority_company_pct
        .map(|share| share < policy.min_priority_company_coverage_pct)
        .unwrap_or(false);

    let status = if failing.is_empty() && !priority_company_degraded {
        "ok"
    } else {
        "degraded"
    };
    let mut problems: Vec<String> = failing
        .iter()
        .map(|evaluation| format!("{}: {}", evaluation.family, evaluation.reasons.join("; ")))
        .collect();
    if priority_company_degraded {
        problems.push(format!(
            "priority-company coverage {}%, minimum {}%",
            priority_company_pct.unwrap_or(0),
            policy.min_priority_company_coverage_pct
        ));
    }

    let detail = if problems.is_empty() {
        format!(
            "{} operational of {} registered across {} required families | {}",
            summary.operational,
            summary.registered,
            required_families,
            coverage_matrix_summary(&evaluations)
        )
    } else {
        format!(
            "{} operational of {} registered | {}",
            summary.operational,
            summary.registered,
            problems.join(" | ")
        )
    };

    let evidence_quality = coverage_evidence_quality(summary, policy, now);

    SourceCoverageReport {
        status: status.to_string(),
        detail,
        families: evaluations,
        required_families,
        satisfied_required_families,
        priority_companies_total: priority_companies.total,
        priority_companies_covered: priority_companies.covered,
        priority_company_pct,
        min_priority_company_coverage_pct: policy.min_priority_company_coverage_pct,
        evidence_quality,
    }
}

/// Compact per-family summary line published alongside the detail.
fn coverage_matrix_summary(evaluations: &[CoverageFamilyEvaluation]) -> String {
    evaluations
        .iter()
        .map(|evaluation| {
            format!(
                "{}={}/{}",
                evaluation.family, evaluation.operational, evaluation.min_operational_sources
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Summarise the operational evidence base through the shared
/// [`EvidenceQuality`] model (audit P1-10 consumer: source coverage).
///
/// Operational sources are supporting evidence for the claim "the deployment
/// covers the required families"; validated-but-degraded sources contradict
/// that claim, so `contradiction_ratio` is measured against a real claim.
/// Expected coverage is the set of required families, so
/// `coverage_completeness` measures the matrix directly.
///
/// Fetch/parser ratios are measurements, not defaults: a family with no
/// recorded attempt keeps the corresponding dimension `NotMeasured` instead of
/// a synthetic 50% prior.
fn coverage_evidence_quality(
    summary: &crate::sources_registry::SourceCoverageSummary,
    policy: &CoveragePolicy,
    now: DateTime<Utc>,
) -> EvidenceQuality {
    let mut items = Vec::new();
    for entry in &summary.families {
        let requirement = policy.requirement(entry.family);
        let required = requirement.map(|row| row.required).unwrap_or(false);
        if entry.operational > 0 {
            let mut item = EvidenceItem::new_optional(
                entry
                    .fetch_success_pct
                    .map(|pct| f64::from(pct.min(100)) / 100.0),
                EvidenceStance::Supports,
            )
            .with_source_type(entry.family.as_str())
            .with_coverage_tag(entry.family.as_str())
            .primary();
            if let Some(parser_pct) = entry.parser_success_pct {
                item = item.with_parser_confidence(f64::from(parser_pct.min(100)) / 100.0);
            }
            if let Some(latest) = entry.latest_success_at {
                item = item.with_observed_at(latest);
            }
            items.push(item);
        }
        if entry.temporarily_degraded > 0 {
            items.push(
                EvidenceItem::new(0.5, EvidenceStance::Contradicts)
                    .with_source_type(entry.family.as_str())
                    .with_coverage_tag(entry.family.as_str()),
            );
        }
        if required && entry.operational == 0 && entry.declared == 0 {
            // A required family with no declared source is itself a
            // contradiction: the deployment promises coverage it cannot
            // substantiate.
            items.push(
                EvidenceItem::new(0.9, EvidenceStance::Contradicts)
                    .with_source_type(entry.family.as_str()),
            );
        }
    }
    let expected: Vec<String> = policy
        .families
        .iter()
        .filter(|row| row.required)
        .map(|row| row.family.as_str().to_string())
        .collect();
    assess_evidence_quality_for_claim(
        &items,
        &expected,
        "the deployment covers the required source families",
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operational_row(slug: &str, now: DateTime<Utc>) -> SourceRuntimeStateRow {
        SourceRuntimeStateRow {
            source_slug: slug.to_string(),
            last_attempt_at: Some(now),
            last_success_at: Some(now),
            next_due_at: now,
            consecutive_failures: 0,
            rolling_success_rate: Some(1.0),
            rolling_latency_ms: Some(50.0),
            last_http_status: Some(200),
            circuit_open_until: None,
            etag: None,
            last_modified: None,
            last_error: None,
            updated_at: now,
        }
    }

    fn source(slug: &str, category: Category, region: crate::sources_registry::Region) -> Source {
        Source {
            slug: slug.to_string(),
            name: slug.to_string(),
            url: format!("https://{slug}.example.com"),
            search_param: None,
            region,
            category,
            tier: 2,
            needs_proxy: false,
            rss_url: None,
            enabled: true,
            min_interval_minutes: 60,
            fetch_strategy: None,
            capability: SourceCapability::Unvalidated,
            notes: None,
        }
    }

    fn summary_with(
        families: Vec<FamilyCoverage>,
        operational: usize,
    ) -> crate::sources_registry::SourceCoverageSummary {
        crate::sources_registry::SourceCoverageSummary {
            registered: 500,
            operational,
            families,
            ..crate::sources_registry::SourceCoverageSummary::default()
        }
    }

    #[test]
    fn family_coverage_reflects_non_success_acquisition_outcomes() {
        let now = Utc::now();
        let sources = vec![
            source(
                "linkedin_company",
                Category::Procurement,
                crate::sources_registry::Region::Global,
            ),
            source(
                "rate_limited_feed",
                Category::Procurement,
                crate::sources_registry::Region::Global,
            ),
        ];
        // Even a coincidentally recorded success row cannot make an
        // unauthenticated adapter source operational.
        let linkedin_row = operational_row("linkedin_company", now);
        let mut rate_row = operational_row("rate_limited_feed", now);
        rate_row.last_error = Some("rate_limited: upstream returned HTTP 429".to_string());
        rate_row.last_success_at = None;
        rate_row.rolling_success_rate = Some(0.0);

        let coverage = family_coverage(
            &sources,
            &[linkedin_row, rate_row],
            &DeploymentCapabilities {
                browser: true,
                proxy: true,
                credentialed_api_adapters: Default::default(),
            },
            now,
        );
        let procurement = coverage
            .iter()
            .find(|entry| entry.family == CoverageFamily::Procurement)
            .expect("procurement row");

        assert_eq!(procurement.operational, 0);
        assert_eq!(procurement.authentication_blocked, 1);
        assert_eq!(procurement.rate_limited, 1);
        assert_eq!(procurement.never_crawled, 0);
    }

    #[test]
    fn required_family_with_zero_operational_sources_fails_despite_high_total() {
        let now = Utc::now();
        let mut families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: 5,
                independent_domains: 5,
                attempted: 5,
                parser_success_pct: Some(100),
                fetch_success_pct: Some(100),
                latest_success_at: Some(now),
                ..FamilyCoverage::default()
            })
            .collect();
        // 500 operational sources overall, but procurement has none.
        let procurement = families
            .iter_mut()
            .find(|entry| entry.family == CoverageFamily::Procurement)
            .expect("procurement row");
        procurement.operational = 0;

        let report = evaluate_coverage(
            &summary_with(families, 500),
            &CoveragePolicy::default(),
            PriorityCompanyCoverage::default(),
            now,
        );

        assert_eq!(report.status, "degraded");
        let procurement_eval = report
            .families
            .iter()
            .find(|row| row.family == CoverageFamily::Procurement)
            .expect("procurement evaluation");
        assert_eq!(procurement_eval.status, "degraded");
        assert!(report.detail.contains("procurement"));
    }

    #[test]
    fn all_families_met_is_ok_and_publishes_the_matrix() {
        let now = Utc::now();
        let families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: 5,
                independent_domains: 5,
                attempted: 5,
                parser_success_pct: Some(100),
                fetch_success_pct: Some(100),
                latest_success_at: Some(now),
                ..FamilyCoverage::default()
            })
            .collect();

        let report = evaluate_coverage(
            &summary_with(families, 55),
            &CoveragePolicy::default(),
            PriorityCompanyCoverage {
                total: 10,
                covered: 8,
            },
            now,
        );

        assert_eq!(report.status, "ok");
        assert_eq!(report.families.len(), CoverageFamily::ALL.len());
        assert_eq!(report.required_families, 11);
        assert_eq!(report.satisfied_required_families, 11);
        assert_eq!(report.priority_company_pct, Some(80));
        assert!(
            (report
                .evidence_quality
                .corpus
                .coverage_completeness
                .value_copied()
                .unwrap_or_default()
                - 1.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn stale_family_and_low_ratios_fail_their_dimensions() {
        let now = Utc::now();
        let stale = now - chrono::Duration::days(30);
        let families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: 5,
                independent_domains: 5,
                attempted: 5,
                parser_success_pct: Some(10),
                fetch_success_pct: Some(10),
                latest_success_at: Some(stale),
                ..FamilyCoverage::default()
            })
            .collect();

        let report = evaluate_coverage(
            &summary_with(families, 55),
            &CoveragePolicy::default(),
            PriorityCompanyCoverage::default(),
            now,
        );

        assert_eq!(report.status, "degraded");
        let procurement = report
            .families
            .iter()
            .find(|row| row.family == CoverageFamily::Procurement)
            .expect("procurement evaluation");
        assert!(procurement
            .reasons
            .iter()
            .any(|reason| reason.contains("newest successful fetch")));
        assert!(procurement
            .reasons
            .iter()
            .any(|reason| reason.contains("successful-fetch ratio")));
        assert!(procurement
            .reasons
            .iter()
            .any(|reason| reason.contains("parser-success ratio")));
    }

    #[test]
    fn priorities_companies_below_threshold_degrade_even_when_families_pass() {
        let now = Utc::now();
        let families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: 5,
                independent_domains: 5,
                attempted: 5,
                parser_success_pct: Some(100),
                fetch_success_pct: Some(100),
                latest_success_at: Some(now),
                ..FamilyCoverage::default()
            })
            .collect();

        let report = evaluate_coverage(
            &summary_with(families, 55),
            &CoveragePolicy::default(),
            PriorityCompanyCoverage {
                total: 10,
                covered: 1,
            },
            now,
        );

        assert_eq!(report.status, "degraded");
        assert_eq!(report.priority_company_pct, Some(10));
        assert!(report.detail.contains("priority-company coverage"));
    }

    #[test]
    fn default_policy_requires_certification_coverage() {
        let policy = CoveragePolicy::default();
        let certifications = policy
            .requirement(CoverageFamily::Certifications)
            .expect("certifications row is always published");
        assert!(
            certifications.required,
            "certifications must be a required family now that lawful sources exist"
        );
        assert_eq!(
            certifications.min_operational_sources, 1,
            "at least one operational certification source must be proven"
        );

        // A deployment with zero operational certification sources is
        // degraded, even when every other family is satisfied.
        let now = Utc::now();
        let families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: if family == CoverageFamily::Certifications {
                    0
                } else {
                    5
                },
                independent_domains: 5,
                attempted: 5,
                parser_success_pct: Some(100),
                fetch_success_pct: Some(100),
                latest_success_at: Some(now),
                ..FamilyCoverage::default()
            })
            .collect();
        let report = evaluate_coverage(
            &summary_with(families, 50),
            &policy,
            PriorityCompanyCoverage::default(),
            now,
        );
        assert_eq!(report.status, "degraded");
        let certifications_eval = report
            .families
            .iter()
            .find(|row| row.family == CoverageFamily::Certifications)
            .expect("certifications evaluation");
        assert_eq!(certifications_eval.status, "degraded");
        assert!(certifications_eval
            .reasons
            .iter()
            .any(|reason| reason.contains("operational sources, minimum 1")));
    }

    #[test]
    fn registered_certification_sources_map_to_the_certification_family() {
        let registry_sources = crate::sources_registry::all_sources();
        let certification_sources: Vec<&crate::sources_registry::Source> = registry_sources
            .iter()
            .filter(|source| {
                source
                    .coverage_families()
                    .contains(&CoverageFamily::Certifications)
            })
            .collect();
        assert!(
            !certification_sources.is_empty(),
            "the registry must declare at least one certification-information source"
        );
        assert!(
            certification_sources
                .iter()
                .any(|source| source.slug == "iaf_certsearch"
                    || source.slug == "fda_device_registration"
                    || source.slug == "iaqg_oasis"),
            "expected IAF/IAQG/openFDA certification registers in the registry"
        );
    }

    #[test]
    fn vulnerability_feeds_do_not_satisfy_the_certification_family() {
        let registry_sources = crate::sources_registry::all_sources();
        let nvd = registry_sources
            .iter()
            .find(|source| source.slug == "nvd_nist_vuln")
            .expect("NVD source is registered");
        assert!(
            !nvd.coverage_families()
                .contains(&CoverageFamily::Certifications),
            "NVD/NIST is a vulnerability feed and must not count as certification coverage"
        );
    }

    #[test]
    fn family_classification_uses_category_and_keywords() {
        let hiring = source(
            "greenhouse_job_board",
            Category::Technology,
            crate::sources_registry::Region::Global,
        );
        let families = hiring.coverage_families();
        assert!(families.contains(&CoverageFamily::Hiring));
        assert!(families.contains(&CoverageFamily::ProductCompetitive));

        let registry = source(
            "sedar_plus_ca",
            Category::GovernmentRegistry,
            crate::sources_registry::Region::NorthAmerica,
        );
        let families = registry.coverage_families();
        assert!(families.contains(&CoverageFamily::ExecutiveChanges));

        let ct = source(
            "certificate_transparency",
            Category::Cybersecurity,
            crate::sources_registry::Region::Global,
        );
        assert!(
            !ct.coverage_families()
                .contains(&CoverageFamily::Certifications),
            "TLS certificate transparency is not a certifications-intel source"
        );
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn invalid_coverage_threshold_fails_loudly_and_names_the_variable() {
        let _guard = env_lock();

        std::env::set_var("APEX_COVERAGE_PRIORITY_COMPANY_PCT", "150");
        let error = CoveragePolicy::from_env()
            .expect_err("an out-of-range percentage must fail loudly")
            .to_string();
        std::env::remove_var("APEX_COVERAGE_PRIORITY_COMPANY_PCT");

        assert!(
            error.contains("APEX_COVERAGE_PRIORITY_COMPANY_PCT"),
            "the error must name the variable: {error}"
        );
        assert!(
            error.contains("150"),
            "the error must name the bad value: {error}"
        );
    }

    #[test]
    fn blank_coverage_threshold_falls_back_to_default() {
        let _guard = env_lock();

        std::env::set_var("APEX_COVERAGE_PRIORITY_COMPANY_PCT", "");
        let policy = CoveragePolicy::from_env().expect("a blank threshold is treated as unset");
        std::env::remove_var("APEX_COVERAGE_PRIORITY_COMPANY_PCT");

        assert_eq!(
            policy.min_priority_company_coverage_pct,
            CoveragePolicy::default().min_priority_company_coverage_pct
        );
    }

    #[test]
    fn unset_coverage_thresholds_keep_their_defaults() {
        let _guard = env_lock();

        std::env::remove_var("APEX_COVERAGE_PRIORITY_COMPANY_PCT");
        let policy = CoveragePolicy::from_env().expect("unset thresholds use defaults");
        assert_eq!(
            policy.min_priority_company_coverage_pct,
            CoveragePolicy::default().min_priority_company_coverage_pct
        );
    }

    #[test]
    fn family_coverage_counts_attempt_ratios_and_domains() {
        let now = Utc::now();
        let sources = vec![
            source(
                "alpha_procurement",
                Category::Procurement,
                crate::sources_registry::Region::Global,
            ),
            source(
                "beta_procurement",
                Category::Procurement,
                crate::sources_registry::Region::Europe,
            ),
        ];
        let states = vec![operational_row("alpha_procurement", now)];
        let coverage = family_coverage(
            &sources,
            &states,
            &DeploymentCapabilities {
                browser: true,
                proxy: true,
                credentialed_api_adapters: Default::default(),
            },
            now,
        );
        let procurement = coverage
            .iter()
            .find(|entry| entry.family == CoverageFamily::Procurement)
            .expect("procurement row");

        assert_eq!(procurement.declared, 2);
        assert_eq!(procurement.operational, 1);
        assert_eq!(procurement.never_crawled, 1);
        assert_eq!(procurement.attempted, 1);
        assert_eq!(procurement.parser_success_pct, Some(100));
        assert_eq!(procurement.fetch_success_pct, Some(100));
        assert_eq!(procurement.latest_success_at, Some(now));
        assert_eq!(procurement.independent_domains, 1);
    }

    #[test]
    fn unmeasured_family_cannot_satisfy_readiness() {
        let now = Utc::now();
        // Every family looks operational with a fresh success, but no source
        // has ever been attempted: the fetch/parser ratios are unmeasured, so
        // the readiness gate must fail instead of treating the families as
        // healthy on a synthetic 50%.
        let families: Vec<FamilyCoverage> = CoverageFamily::ALL
            .into_iter()
            .map(|family| FamilyCoverage {
                family,
                declared: 5,
                registered: 5,
                operational: 5,
                independent_domains: 5,
                attempted: 0,
                parser_success_pct: None,
                fetch_success_pct: None,
                latest_success_at: Some(now),
                ..FamilyCoverage::default()
            })
            .collect();

        let report = evaluate_coverage(
            &summary_with(families, 55),
            &CoveragePolicy::default(),
            PriorityCompanyCoverage::default(),
            now,
        );

        assert_eq!(report.status, "degraded");
        assert_eq!(report.satisfied_required_families, 0);
        let procurement = report
            .families
            .iter()
            .find(|row| row.family == CoverageFamily::Procurement)
            .expect("procurement evaluation");
        assert_eq!(procurement.attempted, 0);
        assert_eq!(procurement.min_coverage_sample, MIN_COVERAGE_SAMPLE);
        assert!(procurement
            .reasons
            .iter()
            .any(|reason| reason.contains("minimum coverage sample")));
        assert_eq!(
            report.evidence_quality.corpus.parser_confidence,
            apex_core::measurement::Measurement::not_measured(),
            "no parser results must stay NotMeasured, not 0.5"
        );
        assert!(report
            .evidence_quality
            .completeness
            .missing_dimensions
            .iter()
            .any(|name| name == "parser_confidence"));
    }

    #[test]
    fn malformed_coverage_threshold_is_a_configuration_error() {
        let _guard = env_lock();

        let name = "APEX_COVERAGE_PROCUREMENT_FETCH_SUCCESS_PCT";
        std::env::set_var(name, "banana");
        let result = CoveragePolicy::from_env();
        std::env::remove_var(name);

        let errors = result.expect_err("banana is not a percentage");
        assert_eq!(errors.errors.len(), 1);
        assert_eq!(errors.errors[0].variable, name);
        assert_eq!(errors.errors[0].value, "banana");
    }

    #[test]
    fn out_of_range_coverage_threshold_is_a_configuration_error() {
        let _guard = env_lock();

        let name = "APEX_COVERAGE_PATENTS_PARSER_SUCCESS_PCT";
        std::env::set_var(name, "150");
        let result = CoveragePolicy::from_env();
        std::env::remove_var(name);

        let errors = result.expect_err("150 is outside 0..=100");
        assert!(errors.errors.iter().any(|error| error.variable == name));
    }
}
