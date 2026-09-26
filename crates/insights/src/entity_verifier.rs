//! # Entity Verifier — Evidence-Backed Verification
//!
//! Company candidates are only promoted into the canonical entity registry
//! when **evidence** supports them. Every check emits an [`EvidenceSignal`]
//! with the source that produced it, and the verifier decides between
//! [`VerificationOutcome::Verified`] (safe to auto-register) and
//! [`VerificationOutcome::AnalystReview`] (no canonical company is created).
//!
//! Supported signal types:
//! - Official corporate registry lookups
//! - GLEIF LEI records
//! - SEC EDGAR (US filers)
//! - Verified corporate domain (DNS/RDAP-anchored)
//! - RDAP registration data
//! - Corporate-site structured metadata (`schema.org`, `og:*`)
//! - Agreement across ≥ 2 independent source domains
//! - Address/region agreement
//!
//! All external access (network, registries, DNS) goes through the
//! [`EvidenceProvider`] trait, so tests inject fakes and never touch the
//! network. Persistence goes through the [`EvidenceStore`] trait; the
//! production implementation writes to `entity_verification_evidence`
//! (migration `052_entity_verification_evidence.sql`).

use crate::company_discovery::{normalize_company_name, CompanyCandidate};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lru::LruCache;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of evidence a signal represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    /// Official corporate registry record (e.g. OpenCorporates, Companies House).
    OfficialRegistry,
    /// GLEIF Legal Entity Identifier record.
    GleifLei,
    /// SEC EDGAR filing/filer record.
    SecEdgar,
    /// The candidate's corporate domain was independently verified.
    VerifiedCorporateDomain,
    /// RDAP registration data for the domain.
    Rdap,
    /// Structured metadata published by the corporate site itself.
    CorporateSiteMetadata,
    /// The same entity was independently reported by ≥ 2 source domains.
    IndependentSourceDomains,
    /// The entity's address/region agrees across sources.
    AddressRegionAgreement,
}

impl VerificationType {
    /// Stable string used in the database and API payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OfficialRegistry => "official_registry",
            Self::GleifLei => "gleif_lei",
            Self::SecEdgar => "sec_edgar",
            Self::VerifiedCorporateDomain => "verified_corporate_domain",
            Self::Rdap => "rdap",
            Self::CorporateSiteMetadata => "corporate_site_metadata",
            Self::IndependentSourceDomains => "independent_source_domains",
            Self::AddressRegionAgreement => "address_region_agreement",
        }
    }

    /// Parse a verification type from its stable string form.
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "official_registry" => Some(Self::OfficialRegistry),
            "gleif_lei" => Some(Self::GleifLei),
            "sec_edgar" => Some(Self::SecEdgar),
            "verified_corporate_domain" => Some(Self::VerifiedCorporateDomain),
            "rdap" => Some(Self::Rdap),
            "corporate_site_metadata" => Some(Self::CorporateSiteMetadata),
            "independent_source_domains" => Some(Self::IndependentSourceDomains),
            "address_region_agreement" => Some(Self::AddressRegionAgreement),
            _ => None,
        }
    }

    /// Registry-class identity evidence (strongest anchor).
    pub fn is_identity_anchor(&self) -> bool {
        matches!(
            self,
            Self::OfficialRegistry | Self::GleifLei | Self::SecEdgar
        )
    }

    /// Corroborating evidence that supports (but cannot alone anchor) identity.
    pub fn is_corroborating(&self) -> bool {
        matches!(
            self,
            Self::VerifiedCorporateDomain
                | Self::Rdap
                | Self::CorporateSiteMetadata
                | Self::IndependentSourceDomains
                | Self::AddressRegionAgreement
        )
    }

    /// Base weight used to aggregate signal confidence.
    pub fn weight(&self) -> f64 {
        match self {
            Self::OfficialRegistry | Self::GleifLei | Self::SecEdgar => 0.5,
            Self::VerifiedCorporateDomain => 0.3,
            Self::IndependentSourceDomains => 0.2,
            Self::Rdap | Self::CorporateSiteMetadata => 0.15,
            Self::AddressRegionAgreement => 0.1,
        }
    }
}

/// A single piece of verification evidence, persisted row-for-row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvidenceSignal {
    /// Deterministic id of the candidate this signal belongs to.
    pub candidate_id: Uuid,
    /// What kind of evidence this is.
    pub verification_type: VerificationType,
    /// URL the evidence was retrieved from, when one exists.
    pub source_url: Option<String>,
    /// Human-readable source (e.g. "SEC EDGAR", "sec.gov RDAP").
    pub source_name: String,
    /// The value that matched (legal name, LEI, domain, region, ...).
    pub matched_value: String,
    /// How confident the provider is in this signal (0.0 – 1.0).
    pub confidence: f64,
    /// When the evidence was observed.
    pub observed_at: DateTime<Utc>,
}

impl EvidenceSignal {
    /// Build a signal with `candidate_id` left nil — the verifier stamps the
    /// owning candidate id before persistence.
    pub fn new(
        verification_type: VerificationType,
        source_name: impl Into<String>,
        source_url: Option<impl Into<String>>,
        matched_value: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Self {
            candidate_id: Uuid::nil(),
            verification_type,
            source_url: source_url.map(Into::into),
            source_name: source_name.into(),
            matched_value: matched_value.into(),
            confidence: confidence.clamp(0.0, 1.0),
            observed_at: Utc::now(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider / store traits (network + DB behind injectable fakes)
// ─────────────────────────────────────────────────────────────────────────────

/// A source of verification evidence.
///
/// Production implementations perform the network calls (registry APIs,
/// GLEIF, EDGAR, RDAP, DNS, HTTP fetches of corporate-site metadata).
/// Tests inject [`StaticEvidenceProvider`] so no network is touched.
#[async_trait]
pub trait EvidenceProvider: Send + Sync {
    /// Stable provider name (used in logs and contradictions).
    fn name(&self) -> &str;

    /// Collect evidence for a candidate. Providers must not fail the whole
    /// verification on a transient error: return `Err` and the verifier
    /// records the failure and routes the candidate to analyst review.
    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>>;
}

/// Persists evidence rows. Every collected signal is persisted, including
/// weak/contradictory ones, so analysts can audit the decision.
#[async_trait]
pub trait EvidenceStore: Send + Sync {
    /// Persist signals, returning the number of newly inserted rows.
    async fn persist_signals(&self, signals: &[EvidenceSignal]) -> Result<u64>;
}

/// `entity_verification_evidence` store (migration 052).
#[derive(Debug, Clone)]
pub struct PgEvidenceStore {
    pool: PgPool,
}

impl PgEvidenceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl EvidenceStore for PgEvidenceStore {
    async fn persist_signals(&self, signals: &[EvidenceSignal]) -> Result<u64> {
        let mut inserted = 0u64;
        for signal in signals {
            let result = sqlx::query(
                r#"
                INSERT INTO entity_verification_evidence
                    (candidate_id, verification_type, source_url, source_name,
                     matched_value, confidence, observed_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (candidate_id, verification_type, source_url, matched_value)
                DO NOTHING
                "#,
            )
            .bind(signal.candidate_id)
            .bind(signal.verification_type.as_str())
            .bind(signal.source_url.clone().unwrap_or_default())
            .bind(&signal.source_name)
            .bind(&signal.matched_value)
            .bind(signal.confidence)
            .bind(signal.observed_at)
            .execute(&self.pool)
            .await?;
            inserted += result.rows_affected();
        }
        Ok(inserted)
    }
}

/// In-memory evidence store for unit tests (records every signal).
#[derive(Debug, Default, Clone)]
pub struct InMemoryEvidenceStore {
    rows: std::sync::Arc<std::sync::Mutex<Vec<EvidenceSignal>>>,
}

impl InMemoryEvidenceStore {
    pub fn new() -> Self {
        Self {
            rows: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Snapshot of the persisted rows.
    pub fn rows(&self) -> Vec<EvidenceSignal> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn len(&self) -> usize {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl EvidenceStore for InMemoryEvidenceStore {
    async fn persist_signals(&self, signals: &[EvidenceSignal]) -> Result<u64> {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut inserted = 0u64;
        for signal in signals {
            let duplicate = rows.iter().any(|r| {
                r.candidate_id == signal.candidate_id
                    && r.verification_type == signal.verification_type
                    && r.source_url == signal.source_url
                    && r.matched_value == signal.matched_value
            });
            if !duplicate {
                rows.push(signal.clone());
                inserted += 1;
            }
        }
        Ok(inserted)
    }
}

/// Provider that returns a fixed set of signals — used by tests and by
/// callers that already hold structured evidence.
pub struct StaticEvidenceProvider {
    name: String,
    signals: Vec<EvidenceSignal>,
}

impl StaticEvidenceProvider {
    pub fn new(name: impl Into<String>, signals: Vec<EvidenceSignal>) -> Self {
        Self {
            name: name.into(),
            signals,
        }
    }

    /// A provider that never finds anything.
    pub fn empty() -> Self {
        Self {
            name: "static_empty".to_string(),
            signals: Vec::new(),
        }
    }
}

#[async_trait]
impl EvidenceProvider for StaticEvidenceProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn collect(&self, _candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        Ok(self.signals.clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Policy / outcome
// ─────────────────────────────────────────────────────────────────────────────

/// What the verifier decided for a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationOutcome {
    /// Sufficient, non-contradictory evidence — safe to auto-register.
    Verified,
    /// Insufficient, weak, or contradictory evidence — an analyst must
    /// review; no canonical company is created.
    AnalystReview,
}

impl VerificationOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::AnalystReview => "analyst_review",
        }
    }
}

/// Thresholds for deciding whether evidence is sufficient to auto-register.
#[derive(Debug, Clone)]
pub struct VerificationPolicy {
    /// Minimum confidence for a signal to count towards sufficiency.
    pub min_signal_confidence: f64,
    /// Minimum number of distinct corroborating signal types.
    pub min_corroborating_signals: usize,
    /// Minimum number of distinct evidence source domains.
    pub min_independent_domains: usize,
    /// Require a registry-class identity anchor (registry / GLEIF / EDGAR).
    pub require_registry_anchor: bool,
    /// Master switch: when false, nothing ever auto-registers.
    pub auto_register: bool,
}

impl Default for VerificationPolicy {
    fn default() -> Self {
        Self {
            min_signal_confidence: 0.6,
            min_corroborating_signals: 2,
            min_independent_domains: 2,
            require_registry_anchor: true,
            auto_register: true,
        }
    }
}

/// Result of applying a policy to a set of signals.
#[derive(Debug, Clone)]
pub struct EvidenceAssessment {
    pub outcome: VerificationOutcome,
    /// Aggregated confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Human-readable methods that contributed.
    pub methods: Vec<String>,
    /// Reasons the candidate must be reviewed (empty when verified).
    pub review_reasons: Vec<String>,
    /// Number of signals that qualified (confidence + non-empty value).
    pub qualifying_signals: usize,
}

impl VerificationPolicy {
    /// Assess a set of evidence signals.
    pub fn assess(&self, signals: &[EvidenceSignal]) -> EvidenceAssessment {
        let mut review_reasons: Vec<String> = Vec::new();
        let mut methods: Vec<String> = Vec::new();

        let mut confidence = 0.0f64;
        let mut qualifying: Vec<&EvidenceSignal> = Vec::new();
        let mut registry_identities: HashSet<String> = HashSet::new();
        let mut registry_names: Vec<String> = Vec::new();

        for signal in signals {
            let matched = signal.matched_value.trim();
            methods.push(format!(
                "{}:{}:{:.2}",
                signal.verification_type.as_str(),
                signal.source_name,
                signal.confidence
            ));

            if matched.is_empty() {
                review_reasons.push(format!(
                    "ambiguous evidence: empty matched value from {}",
                    signal.source_name
                ));
                continue;
            }

            // Explicit disagreement signal (e.g. address/region mismatch) is
            // checked before the confidence gate: a contradiction must route
            // the candidate to review even when the provider is not confident
            // enough for the signal to count as positive evidence.
            if signal.verification_type == VerificationType::AddressRegionAgreement {
                let lower = matched.to_lowercase();
                if lower.contains("mismatch")
                    || lower.contains("conflict")
                    || lower.contains("disagree")
                {
                    review_reasons.push(format!(
                        "contradictory address/region evidence from {}: {}",
                        signal.source_name, matched
                    ));
                    continue;
                }
            }

            if signal.confidence < self.min_signal_confidence {
                if signal.verification_type.is_identity_anchor() {
                    review_reasons.push(format!(
                        "ambiguous registry evidence from {} ({:.2} < {:.2})",
                        signal.source_name, signal.confidence, self.min_signal_confidence
                    ));
                }
                continue;
            }

            confidence += signal.verification_type.weight() * signal.confidence;

            if signal.verification_type.is_identity_anchor() {
                registry_names.push(format!("{} ({})", matched, signal.source_name));
                registry_identities.insert(normalize_company_name(matched));
            }

            qualifying.push(signal);
        }

        // Contradiction 1: registry-class sources disagree on the identity.
        if registry_identities.len() > 1 {
            registry_names.sort();
            registry_names.dedup();
            review_reasons.push(format!(
                "contradictory registry identities: {}",
                registry_names.join(" vs ")
            ));
        }

        let confidence = confidence.min(1.0);

        let has_anchor = qualifying
            .iter()
            .any(|s| s.verification_type.is_identity_anchor());

        let mut corroborating_types: HashSet<VerificationType> = HashSet::new();
        let mut source_domains: HashSet<String> = HashSet::new();
        let mut has_independent_sources_signal = false;

        for signal in &qualifying {
            if signal.verification_type == VerificationType::IndependentSourceDomains {
                has_independent_sources_signal = true;
            }
            if signal.verification_type.is_corroborating() {
                corroborating_types.insert(signal.verification_type);
            }
            if let Some(url) = &signal.source_url {
                if let Some(domain) = domain_of(url) {
                    source_domains.insert(domain);
                }
            }
        }

        let corroborating_count = corroborating_types.len();
        let independent_domains_ok =
            has_independent_sources_signal || source_domains.len() >= self.min_independent_domains;

        if self.require_registry_anchor && !has_anchor {
            review_reasons.push("missing registry-class identity anchor".to_string());
        }
        if corroborating_count < self.min_corroborating_signals {
            review_reasons.push(format!(
                "insufficient corroboration: {} of {} required signal types",
                corroborating_count, self.min_corroborating_signals
            ));
        }
        if !independent_domains_ok {
            review_reasons.push(format!(
                "insufficient independent source domains: {} of {} required",
                source_domains.len(),
                self.min_independent_domains
            ));
        }

        review_reasons.sort();
        review_reasons.dedup();

        let sufficient = review_reasons.is_empty();
        let outcome = if sufficient && self.auto_register {
            VerificationOutcome::Verified
        } else {
            VerificationOutcome::AnalystReview
        };

        EvidenceAssessment {
            outcome,
            confidence,
            methods,
            review_reasons,
            qualifying_signals: qualifying.len(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verifier
// ─────────────────────────────────────────────────────────────────────────────

/// Result of verifying a company candidate.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// The candidate that was verified.
    pub candidate: CompanyCandidate,
    /// Whether verification passed (equivalent to `outcome == Verified`).
    pub is_verified: bool,
    /// Full decision: auto-register vs analyst review.
    pub outcome: VerificationOutcome,
    /// Overall confidence (0.0 – 1.0).
    pub confidence: f64,
    /// Methods used for verification.
    pub verification_methods: Vec<String>,
    /// Every evidence signal collected (persisted row-for-row).
    pub evidence: Vec<EvidenceSignal>,
    /// Why the candidate was routed to analyst review (empty when verified).
    pub review_reasons: Vec<String>,
    /// When verification was performed.
    pub verified_at: DateTime<Utc>,
    /// Additional metadata discovered during verification.
    pub metadata: HashMap<String, String>,
}

/// Deterministic candidate id derived from the normalized company name.
pub fn candidate_id_for(normalized_name: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, normalized_name.as_bytes())
}

/// Evidence-backed entity verifier.
///
/// Collects signals from all registered [`EvidenceProvider`]s, persists them
/// through the optional [`EvidenceStore`], and applies a
/// [`VerificationPolicy`]. Candidates only auto-register when a sufficient,
/// non-contradictory evidence combination is present; everything else is
/// returned as [`VerificationOutcome::AnalystReview`].
pub struct EntityVerifier {
    providers: Vec<Box<dyn EvidenceProvider>>,
    store: Option<Box<dyn EvidenceStore>>,
    policy: VerificationPolicy,
    verification_cache: LruCache<String, VerificationResult>,
}

impl EntityVerifier {
    /// Create a verifier with the local, network-free seed directory provider.
    ///
    /// This constructor is **seed-only**: it can never perform a real
    /// registry/GLEIF/EDGAR/RDAP lookup. Production code must use
    /// [`crate::entity_providers::build_production_entity_verifier`]; the
    /// explicit name keeps the seed directory from being wired by accident.
    pub fn seed_only() -> Self {
        Self {
            providers: vec![Box::new(SeedDirectoryProvider::new())],
            store: None,
            policy: VerificationPolicy::default(),
            verification_cache: LruCache::new(NonZeroUsize::new(1000).unwrap_or(NonZeroUsize::MIN)),
        }
    }

    /// Create a verifier with no evidence providers (everything is reviewed).
    pub fn without_providers() -> Self {
        Self {
            providers: Vec::new(),
            store: None,
            policy: VerificationPolicy::default(),
            verification_cache: LruCache::new(NonZeroUsize::new(1000).unwrap_or(NonZeroUsize::MIN)),
        }
    }

    /// Replace the policy.
    pub fn with_policy(mut self, policy: VerificationPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Replace the provider list.
    pub fn with_providers(mut self, providers: Vec<Box<dyn EvidenceProvider>>) -> Self {
        self.providers = providers;
        self
    }

    /// Add a provider.
    pub fn with_provider(mut self, provider: Box<dyn EvidenceProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Attach an evidence store (signals are persisted on every verify).
    pub fn with_store(mut self, store: Box<dyn EvidenceStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// The active policy.
    pub fn policy(&self) -> &VerificationPolicy {
        &self.policy
    }

    /// Names of the configured evidence providers, in evaluation order.
    ///
    /// Used by tests and startup logging to prove the production verifier is
    /// not backed by the seed directory alone.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers
            .iter()
            .map(|provider| provider.name())
            .collect()
    }

    /// Verify a company candidate using all available evidence providers.
    ///
    /// Every collected signal is persisted (when a store is configured) and
    /// included in the result. The candidate auto-registers only when the
    /// policy finds a sufficient, non-contradictory combination; otherwise
    /// the result carries [`VerificationOutcome::AnalystReview`] and no
    /// canonical company should be created.
    pub async fn verify(&mut self, candidate: &CompanyCandidate) -> VerificationResult {
        let cache_key = candidate.normalized_name.clone();

        if let Some(cached) = self.verification_cache.get(&cache_key) {
            return cached.clone();
        }

        let candidate_id = candidate_id_for(&candidate.normalized_name);
        let mut evidence: Vec<EvidenceSignal> = Vec::new();
        let mut provider_failures: Vec<String> = Vec::new();

        for provider in &self.providers {
            match provider.collect(candidate).await {
                Ok(mut signals) => {
                    for signal in &mut signals {
                        signal.candidate_id = candidate_id;
                    }
                    evidence.extend(signals);
                }
                Err(e) => {
                    provider_failures.push(format!("{}: {e}", provider.name()));
                }
            }
        }

        if let Some(store) = &self.store {
            if let Err(e) = store.persist_signals(&evidence).await {
                tracing::warn!(
                    candidate = %candidate.normalized_name,
                    error = %e,
                    "failed to persist entity verification evidence"
                );
            }
        }

        let mut assessment = self.policy.assess(&evidence);
        for failure in &provider_failures {
            assessment
                .review_reasons
                .push(format!("provider unavailable: {failure}"));
        }
        if !provider_failures.is_empty() {
            assessment.outcome = VerificationOutcome::AnalystReview;
        }
        assessment.review_reasons.sort();
        assessment.review_reasons.dedup();

        let outcome = assessment.outcome;
        let is_verified = outcome == VerificationOutcome::Verified;

        let mut metadata = candidate.metadata.clone();
        metadata.insert(
            "verification_confidence".to_string(),
            format!("{:.4}", assessment.confidence),
        );
        metadata.insert(
            "verification_outcome".to_string(),
            outcome.as_str().to_string(),
        );
        if !assessment.review_reasons.is_empty() {
            metadata.insert(
                "verification_review_reasons".to_string(),
                assessment.review_reasons.join("; "),
            );
        }

        let result = VerificationResult {
            candidate: candidate.clone(),
            is_verified,
            outcome,
            confidence: assessment.confidence,
            verification_methods: assessment.methods,
            evidence,
            review_reasons: assessment.review_reasons,
            verified_at: Utc::now(),
            metadata,
        };

        // Only cache stable, fully-evaluated successes. Review outcomes (and
        // anything collected while a provider was failing) must be
        // re-evaluated on the next pass, otherwise a transient failure or new
        // evidence can never change the verdict.
        if is_verified && provider_failures.is_empty() {
            self.verification_cache.put(cache_key, result.clone());
        }
        result
    }

    /// Invalidate the cached verification for a candidate, forcing the next
    /// call to re-collect evidence.
    pub fn invalidate_cache(&mut self, normalized_name: &str) {
        self.verification_cache.pop(normalized_name);
    }

    /// Check if a website/domain looks valid.
    ///
    /// This is a shape check used by providers, not evidence by itself.
    pub fn check_website(&self, url: &str) -> bool {
        let domain = domain_of(url).unwrap_or_default();
        if domain.is_empty() {
            return false;
        }

        if let Some(dot_pos) = domain.rfind('.') {
            let tld = &domain[dot_pos + 1..];
            let valid_tlds = [
                "com", "org", "net", "io", "ai", "co", "de", "fr", "jp", "cn", "sg", "tw", "uk",
                "eu", "gov", "edu",
            ];
            valid_tlds.contains(&tld) && domain.len() > 4
        } else {
            false
        }
    }

    /// Quick heuristic: does the name look like a real company?
    ///
    /// This is a *shape* check only — it never contributes evidence and can
    /// never verify a candidate on its own.
    pub fn heuristic_check(&self, name: &str) -> f64 {
        let name = name.trim();

        if name.len() <= 3 || name.len() >= 100 {
            return 0.0;
        }

        if !name.chars().any(|c| c.is_alphabetic()) {
            return 0.0;
        }

        let lower = name.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        let stopwords: HashSet<&str> = [
            "the", "this", "that", "these", "those", "there", "their", "they", "have", "has",
            "had", "been", "being", "some", "any", "each", "every", "both", "few", "more", "most",
            "other", "into", "over", "such", "only", "own", "same", "than", "very", "just", "also",
            "about", "above", "below", "between", "through", "during", "before", "after", "where",
            "which", "what", "when", "why", "how", "who", "whom", "with", "without",
        ]
        .into();

        if !words.is_empty() && words.iter().all(|w| stopwords.contains(w)) {
            return 0.0;
        }

        let exclusion_list: HashSet<&str> = [
            "new york",
            "los angeles",
            "chicago",
            "houston",
            "london",
            "paris",
            "tokyo",
            "beijing",
            "shanghai",
            "hong kong",
            "singapore",
            "dubai",
            "san francisco",
            "washington",
            "boston",
            "seattle",
            "miami",
            "dallas",
            "berlin",
            "munich",
            "milan",
            "rome",
            "madrid",
            "toronto",
            "sydney",
            "melbourne",
            "mumbai",
            "delhi",
            "bangalore",
        ]
        .into();
        if exclusion_list.contains(lower.as_str()) {
            return 0.0;
        }

        let mut score: f64 = 0.3;

        let suffixes = [
            "inc",
            "corp",
            "ltd",
            "llc",
            "plc",
            "gmbh",
            "sarl",
            "ag",
            "kg",
            "limited",
            "incorporated",
            "corporation",
            "company",
            "co",
        ];
        let has_suffix = words
            .iter()
            .any(|w| suffixes.contains(&w.trim_end_matches('.')));
        if has_suffix {
            score += 0.3;
        }

        let titlecase_words: Vec<&str> = name
            .split_whitespace()
            .filter(|w| {
                let chars: Vec<char> = w.chars().collect();
                chars.len() > 1
                    && chars[0].is_uppercase()
                    && chars[1..]
                        .iter()
                        .all(|c| c.is_lowercase() || !c.is_alphabetic())
            })
            .collect();
        if titlecase_words.len() >= 2 {
            score += 0.2;
        }

        if name.chars().filter(|c| c.is_uppercase()).count() >= 2 && name.len() > 5 {
            score += 0.1;
        }

        if name.len() < 5 {
            score -= 0.2;
        }

        score.clamp(0.0, 1.0)
    }
}

/// Local, network-free provider over the curated ticker/domain seed data.
///
/// Emits `OfficialRegistry` evidence for a known listing and
/// `VerifiedCorporateDomain` for a known corporate domain. Because these are
/// single-source signals, they can never auto-verify a candidate by
/// themselves — they merely anchor/ corroborate richer evidence.
pub struct SeedDirectoryProvider {
    known_tickers: HashMap<String, String>,
    known_domains: HashMap<String, String>,
}

impl SeedDirectoryProvider {
    pub fn new() -> Self {
        let seed_tickers: &[(&str, &str)] = &[
            ("AAPL", "Apple"),
            ("MSFT", "Microsoft"),
            ("GOOGL", "Alphabet"),
            ("GOOG", "Alphabet"),
            ("AMZN", "Amazon"),
            ("NVDA", "NVIDIA"),
            ("META", "Meta"),
            ("TSLA", "Tesla"),
            ("TSM", "TSMC"),
            ("INTC", "Intel"),
            ("AMD", "AMD"),
            ("QCOM", "Qualcomm"),
            ("AVGO", "Broadcom"),
            ("ASML", "ASML"),
            ("TXN", "Texas Instruments"),
            ("MU", "Micron"),
            ("CRM", "Salesforce"),
            ("ORCL", "Oracle"),
            ("IBM", "IBM"),
            ("CSCO", "Cisco"),
            ("NOC", "Northrop Grumman"),
            ("LMT", "Lockheed Martin"),
            ("RTX", "Raytheon Technologies"),
            ("GD", "General Dynamics"),
            ("BA", "Boeing"),
            ("AIR", "Airbus"),
            ("EADSY", "Airbus"),
            ("BAESY", "BAE Systems"),
            ("ESLT", "Elbit Systems"),
            ("RHM.DE", "Rheinmetall"),
            ("SAAB-B.ST", "Saab"),
            ("LDO.MI", "Leonardo"),
            ("HO.PA", "Thales"),
            ("JBL", "Jabil"),
            ("FLEX", "Flex"),
            ("CLS", "Celestica"),
            ("SANM", "Sanmina"),
            ("PLXS", "Plexus"),
            ("KE", "Kimball Electronics"),
            ("BHE", "Benchmark Electronics"),
        ];
        let mut known_tickers = HashMap::new();
        for (ticker, name) in seed_tickers {
            known_tickers.insert(ticker.to_string(), name.to_string());
        }

        let seed_domains: &[(&str, &str)] = &[
            ("apple.com", "Apple"),
            ("microsoft.com", "Microsoft"),
            ("nvidia.com", "NVIDIA"),
            ("tsmc.com", "TSMC"),
            ("intel.com", "Intel"),
            ("amd.com", "AMD"),
            ("ibm.com", "IBM"),
            ("oracle.com", "Oracle"),
            ("cisco.com", "Cisco"),
            ("lockheedmartin.com", "Lockheed Martin"),
            ("northropgrumman.com", "Northrop Grumman"),
            ("raytheon.com", "Raytheon Technologies"),
            ("gdeb.com", "General Dynamics"),
            ("boeing.com", "Boeing"),
            ("airbus.com", "Airbus"),
            ("baesystems.com", "BAE Systems"),
            ("elbitsystems.com", "Elbit Systems"),
            ("rheinmetall.com", "Rheinmetall"),
            ("saab.com", "Saab"),
            ("leonardo.com", "Leonardo"),
            ("thalesgroup.com", "Thales"),
            ("jabil.com", "Jabil"),
            ("flex.com", "Flex"),
            ("celestica.com", "Celestica"),
            ("foxconn.com", "Foxconn"),
        ];
        let mut known_domains = HashMap::new();
        for (domain, name) in seed_domains {
            known_domains.insert(domain.to_string(), name.to_string());
        }

        Self {
            known_tickers,
            known_domains,
        }
    }

    fn ticker_signal(&self, candidate: &CompanyCandidate) -> Option<EvidenceSignal> {
        let ticker = candidate.metadata.get("ticker")?;
        let exchange = candidate.metadata.get("exchange");
        let name = exchange
            .and_then(|ex| self.known_tickers.get(&format!("{}:{}", ex, ticker)))
            .or_else(|| self.known_tickers.get(ticker))?;

        Some(EvidenceSignal::new(
            VerificationType::OfficialRegistry,
            "curated_ticker_directory",
            None::<String>,
            name,
            0.7,
        ))
    }

    fn domain_signal(&self, candidate: &CompanyCandidate) -> Option<EvidenceSignal> {
        let website = candidate.metadata.get("website")?;
        let domain = domain_of(website)?;
        let name = self.known_domains.get(&domain)?;

        Some(EvidenceSignal::new(
            VerificationType::VerifiedCorporateDomain,
            "curated_domain_directory",
            Some(format!("https://{domain}")),
            name,
            0.9,
        ))
    }
}

impl Default for SeedDirectoryProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EvidenceProvider for SeedDirectoryProvider {
    fn name(&self) -> &str {
        "seed_directory"
    }

    async fn collect(&self, candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
        let mut signals = Vec::new();
        if let Some(signal) = self.ticker_signal(candidate) {
            signals.push(signal);
        }
        if let Some(signal) = self.domain_signal(candidate) {
            signals.push(signal);
        }
        Ok(signals)
    }
}

/// Extract the host portion of a URL or bare domain, lowercased and without
/// a leading `www.`.
pub(crate) fn domain_of(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.contains('.') && !host.contains(' ') {
        Some(host)
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company_discovery::{normalize_company_name, DiscoverySource};

    fn make_candidate(name: &str) -> CompanyCandidate {
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: normalize_company_name(name),
            source: DiscoverySource::NewsArticle,
            extraction_confidence: 0.8,
            context_snippet: String::new(),
            metadata: HashMap::new(),
        }
    }

    fn candidate_with_metadata(name: &str, metadata: &[(&str, &str)]) -> CompanyCandidate {
        let mut map = HashMap::new();
        for (k, v) in metadata {
            map.insert(k.to_string(), v.to_string());
        }
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: normalize_company_name(name),
            source: DiscoverySource::NewsArticle,
            extraction_confidence: 0.8,
            context_snippet: String::new(),
            metadata: map,
        }
    }

    fn static_verifier(signals: Vec<EvidenceSignal>) -> EntityVerifier {
        EntityVerifier::without_providers()
            .with_provider(Box::new(StaticEvidenceProvider::new("fake", signals)))
    }

    fn registry_signal(name: &str) -> EvidenceSignal {
        EvidenceSignal::new(
            VerificationType::OfficialRegistry,
            "Fake Registry",
            Some("https://registry.example/company/1"),
            name,
            0.95,
        )
    }

    fn domain_signal(name: &str) -> EvidenceSignal {
        EvidenceSignal::new(
            VerificationType::VerifiedCorporateDomain,
            "Corporate Domain",
            Some("https://acme.example"),
            name,
            0.9,
        )
    }

    fn independent_sources_signal(domains: &str) -> EvidenceSignal {
        EvidenceSignal::new(
            VerificationType::IndependentSourceDomains,
            "Multi-source corroboration",
            Some("https://news-a.example/story"),
            domains,
            0.85,
        )
    }

    // ── Heuristic shape checks ────────────────────────────────────────────

    #[test]
    fn test_heuristic_check_passes_good_name() {
        let verifier = EntityVerifier::seed_only();
        let score = verifier.heuristic_check("NVIDIA Corporation");
        assert!(
            score > 0.5,
            "NVIDIA Corporation should score > 0.5, got {:.2}",
            score
        );
    }

    #[test]
    fn test_heuristic_check_rejects_short_name() {
        let verifier = EntityVerifier::seed_only();
        assert_eq!(verifier.heuristic_check("AB"), 0.0);
    }

    #[test]
    fn test_heuristic_check_rejects_stopwords() {
        let verifier = EntityVerifier::seed_only();
        assert_eq!(verifier.heuristic_check("the this that"), 0.0);
    }

    #[test]
    fn test_heuristic_check_rejects_location() {
        let verifier = EntityVerifier::seed_only();
        assert_eq!(verifier.heuristic_check("New York"), 0.0);
    }

    #[tokio::test]
    async fn heuristic_is_not_evidence() {
        let mut verifier = EntityVerifier::without_providers();
        let candidate = make_candidate("UnknownStartupXYZ Corp");
        let result = verifier.verify(&candidate).await;
        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(
            result.evidence.is_empty(),
            "heuristics must not fabricate evidence"
        );
    }

    #[test]
    fn test_check_website_valid() {
        let verifier = EntityVerifier::seed_only();
        assert!(verifier.check_website("https://www.apple.com"));
        assert!(verifier.check_website("https://nvidia.com"));
        assert!(verifier.check_website("http://example.org"));
        assert!(!verifier.check_website("not-a-url"));
    }

    #[test]
    fn test_candidate_id_is_deterministic() {
        let a = candidate_id_for(&normalize_company_name("Acme Corp"));
        let b = candidate_id_for(&normalize_company_name("Acme Corp"));
        let c = candidate_id_for(&normalize_company_name("Other Ltd"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // ── Evidence-backed decisions ─────────────────────────────────────────

    #[tokio::test]
    async fn no_signals_routes_to_analyst_review_not_registration() {
        let mut verifier = static_verifier(Vec::new());
        let candidate = make_candidate("Ghost Company Inc");
        let result = verifier.verify(&candidate).await;

        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(
            !result.is_verified,
            "must not auto-register without evidence"
        );
        assert!(result.evidence.is_empty());
        assert!(!result.review_reasons.is_empty());
    }

    #[tokio::test]
    async fn single_weak_signal_routes_to_analyst_review() {
        let weak = EvidenceSignal::new(
            VerificationType::OfficialRegistry,
            "Weak Registry",
            Some("https://registry.example/x"),
            "Acme Corp",
            0.3,
        );
        let mut verifier = static_verifier(vec![weak]);
        let candidate = make_candidate("Acme Corp");
        let result = verifier.verify(&candidate).await;

        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(!result.is_verified);
        assert!(
            result
                .review_reasons
                .iter()
                .any(|r| r.contains("ambiguous")),
            "weak signal should be flagged ambiguous, got {:?}",
            result.review_reasons
        );
    }

    #[tokio::test]
    async fn registry_plus_domain_plus_independent_sources_verifies() {
        let signals = vec![
            registry_signal("Acme Corp"),
            domain_signal("Acme Corp"),
            EvidenceSignal::new(
                VerificationType::IndependentSourceDomains,
                "Three independent sources",
                Some("https://news-a.example/story"),
                "news-a.example, news-b.example",
                0.9,
            ),
            EvidenceSignal::new(
                VerificationType::Rdap,
                "RDAP",
                Some("https://rdap.example/acme"),
                "Acme Corp",
                0.8,
            ),
        ];
        let mut verifier = static_verifier(signals);
        let candidate = make_candidate("Acme Corp");
        let result = verifier.verify(&candidate).await;

        assert_eq!(
            result.outcome,
            VerificationOutcome::Verified,
            "reasons: {:?}",
            result.review_reasons
        );
        assert!(result.is_verified);
        assert!(result.confidence > 0.5);
        assert!(result.review_reasons.is_empty());
        assert_eq!(result.evidence.len(), 4);
    }

    #[tokio::test]
    async fn ambiguous_or_contradictory_evidence_routes_to_review() {
        // Two registry-class sources disagree about the legal identity.
        let signals = vec![
            registry_signal("Acme Corporation"),
            EvidenceSignal::new(
                VerificationType::GleifLei,
                "GLEIF",
                Some("https://gleif.example/lei/1"),
                "Zenith Holdings GmbH",
                0.95,
            ),
            domain_signal("Acme Corp"),
            EvidenceSignal::new(
                VerificationType::IndependentSourceDomains,
                "Two independent sources",
                Some("https://news-a.example/story"),
                "news-a.example, news-b.example",
                0.9,
            ),
        ];
        let mut verifier = static_verifier(signals);
        let candidate = make_candidate("Acme Corp");
        let result = verifier.verify(&candidate).await;

        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(
            result
                .review_reasons
                .iter()
                .any(|r| r.contains("contradictory registry identities")),
            "expected contradiction, got {:?}",
            result.review_reasons
        );
    }

    #[tokio::test]
    async fn address_region_mismatch_is_contradictory() {
        let signals = vec![
            registry_signal("Acme Corp"),
            domain_signal("Acme Corp"),
            EvidenceSignal::new(
                VerificationType::AddressRegionAgreement,
                "Region check",
                Some("https://registry.example/acme"),
                "mismatch: HQ region US vs registry DE",
                0.9,
            ),
            independent_sources_signal("news-a.example, news-b.example"),
        ];
        let mut verifier = static_verifier(signals);
        let result = verifier.verify(&make_candidate("Acme Corp")).await;

        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(result
            .review_reasons
            .iter()
            .any(|r| r.contains("contradictory address/region")));
    }

    #[tokio::test]
    async fn low_confidence_region_mismatch_still_blocks_verification() {
        // The mismatch signal is below the positive-evidence confidence gate;
        // explicit negative evidence must still route the candidate to review.
        let signals = vec![
            registry_signal("Acme Corp"),
            domain_signal("Acme Corp"),
            independent_sources_signal("news-a.example, news-b.example"),
            EvidenceSignal::new(
                VerificationType::AddressRegionAgreement,
                "Region check",
                Some("https://registry.example/acme"),
                "mismatch: HQ region US vs registry DE",
                0.2,
            ),
        ];
        let mut verifier = static_verifier(signals);
        let result = verifier.verify(&make_candidate("Acme Corp")).await;

        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert!(result
            .review_reasons
            .iter()
            .any(|r| r.contains("contradictory address/region")));
    }

    /// Provider that fails on the first call and succeeds afterwards.
    struct FlakyProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl EvidenceProvider for FlakyProvider {
        fn name(&self) -> &str {
            "flaky"
        }

        async fn collect(&self, _candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                anyhow::bail!("transient provider outage");
            }
            Ok(vec![
                registry_signal("Acme Corp"),
                domain_signal("Acme Corp"),
                independent_sources_signal("news-a.example, news-b.example"),
            ])
        }
    }

    #[tokio::test]
    async fn provider_failure_is_not_cached_and_recovers_on_retry() {
        let mut verifier =
            EntityVerifier::without_providers().with_provider(Box::new(FlakyProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }));
        let candidate = make_candidate("Acme Corp");

        let first = verifier.verify(&candidate).await;
        assert_eq!(first.outcome, VerificationOutcome::AnalystReview);
        assert!(first
            .review_reasons
            .iter()
            .any(|r| r.contains("provider unavailable")));

        // A review outcome must never be cached: the retry re-collects
        // evidence and can flip the verdict.
        let second = verifier.verify(&candidate).await;
        assert_eq!(second.outcome, VerificationOutcome::Verified);
    }

    #[tokio::test]
    async fn verified_outcome_is_cached() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct CountingProvider {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl EvidenceProvider for CountingProvider {
            fn name(&self) -> &str {
                "counting"
            }

            async fn collect(&self, _candidate: &CompanyCandidate) -> Result<Vec<EvidenceSignal>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![
                    registry_signal("Acme Corp"),
                    domain_signal("Acme Corp"),
                    independent_sources_signal("news-a.example, news-b.example"),
                ])
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mut verifier =
            EntityVerifier::without_providers().with_provider(Box::new(CountingProvider {
                calls: Arc::clone(&calls),
            }));
        let candidate = make_candidate("Acme Corp");

        let first = verifier.verify(&candidate).await;
        assert_eq!(first.outcome, VerificationOutcome::Verified);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Stable successes are cached: the second call must not re-collect.
        let second = verifier.verify(&candidate).await;
        assert_eq!(second.outcome, VerificationOutcome::Verified);
        assert_eq!(second.evidence.len(), first.evidence.len());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "verified outcome should be served from cache"
        );
    }

    #[tokio::test]
    async fn every_collected_signal_is_persisted_even_when_review() {
        let store = InMemoryEvidenceStore::new();
        let signals = vec![
            registry_signal("Acme Corp"),
            EvidenceSignal::new(
                VerificationType::OfficialRegistry,
                "Weak Registry",
                Some("https://registry.example/weak"),
                "Acme Corp",
                0.2,
            ),
        ];
        let mut verifier = static_verifier(signals).with_store(Box::new(store.clone()));

        let result = verifier.verify(&make_candidate("Acme Corp")).await;
        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
        assert_eq!(store.len(), 2, "weak signals must be persisted too");
        assert_eq!(store.rows().len(), result.evidence.len());
        assert!(store.rows().iter().all(|r| r.candidate_id != Uuid::nil()));
    }

    #[tokio::test]
    async fn seed_provider_emits_known_domain_evidence_but_not_auto_verify() {
        let mut verifier = EntityVerifier::seed_only();
        let candidate = candidate_with_metadata(
            "NVIDIA Corporation",
            &[
                ("ticker", "NVDA"),
                ("exchange", "NASDAQ"),
                ("website", "https://nvidia.com"),
            ],
        );
        let result = verifier.verify(&candidate).await;

        assert_eq!(result.evidence.len(), 2);
        assert!(result
            .evidence
            .iter()
            .any(|s| s.verification_type == VerificationType::OfficialRegistry));
        assert!(result
            .evidence
            .iter()
            .any(|s| s.verification_type == VerificationType::VerifiedCorporateDomain));
        // No corroboration beyond the single domain → analyst review.
        assert_eq!(result.outcome, VerificationOutcome::AnalystReview);
    }

    #[tokio::test]
    async fn verification_cache_returns_identical_result() {
        let mut verifier = static_verifier(vec![registry_signal("Acme Corp")]);
        let candidate = make_candidate("Acme Corp");
        let first = verifier.verify(&candidate).await;
        let second = verifier.verify(&candidate).await;
        assert_eq!(first.confidence, second.confidence);
        assert_eq!(first.evidence.len(), second.evidence.len());
        assert_eq!(first.outcome, second.outcome);
    }

    /// Opt-in persistence test against a real PostgreSQL database.
    ///
    /// Run with: `TEST_DATABASE_URL=... cargo test -p apex-insights -- --ignored`
    #[tokio::test]
    #[ignore = "requires PostgreSQL; run with --ignored"]
    async fn evidence_rows_persist_to_postgres() {
        let url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect to postgres");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrations apply");

        let candidate_id = Uuid::new_v4();
        let store = PgEvidenceStore::new(pool.clone());
        let signal = EvidenceSignal::new(
            VerificationType::SecEdgar,
            "SEC EDGAR",
            Some("https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany"),
            "Acme Corp",
            0.95,
        );
        let mut signal = signal;
        signal.candidate_id = candidate_id;

        let inserted = store
            .persist_signals(std::slice::from_ref(&signal))
            .await
            .expect("persist evidence");
        assert_eq!(inserted, 1);

        // Re-persisting the same signal is idempotent.
        let again = store
            .persist_signals(std::slice::from_ref(&signal))
            .await
            .expect("persist evidence again");
        assert_eq!(again, 0, "duplicate evidence rows must not be inserted");

        let rows: Vec<(String, String, f64)> = sqlx::query_as(
            "SELECT verification_type, matched_value, confidence \
             FROM entity_verification_evidence WHERE candidate_id = $1",
        )
        .bind(candidate_id)
        .fetch_all(&pool)
        .await
        .expect("query evidence");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "sec_edgar");
        assert_eq!(rows[0].1, "Acme Corp");
        assert!((rows[0].2 - 0.95).abs() < 1e-9);

        sqlx::query("DELETE FROM entity_verification_evidence WHERE candidate_id = $1")
            .bind(candidate_id)
            .execute(&pool)
            .await
            .expect("cleanup");
        pool.close().await;
    }
}
