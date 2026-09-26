//! # Entity Admission — the only path from a discovered candidate to a
//! canonical company
//!
//! [`EntityAdmissionService`] is the single admission authority for discovered
//! company candidates. Every candidate flows through the evidence-backed
//! [`EntityVerifier`]; the service then:
//!
//! - returns [`EntityAdmissionResult::Existing`] when the canonical company
//!   already exists (no verification, no write),
//! - persists evidence and creates the canonical company **only** on
//!   [`VerificationOutcome::Verified`],
//! - enqueues an `entity_review_queue` row (migration 058) for anything that
//!   is ambiguous, contradictory, below the verification bar, or blocked by a
//!   provider failure — and creates no company,
//! - rejects candidates below the "worth verifying" confidence threshold.
//!
//! Discovery jobs must route through this service instead of calling
//! `store.insert_company` directly. The confidence threshold (0.45 by
//! default) is a *pre-filter*: it decides what is worth verifying, never what
//! gets admitted.

use crate::company_discovery::{normalize_company_name, CompanyCandidate};
use crate::entity_verifier::{
    candidate_id_for, domain_of, EntityVerifier, VerificationOutcome, VerificationResult,
};
use anyhow::Result;
use apex_core::entities::{Company, CompanyType};
use async_trait::async_trait;
use uuid::Uuid;

/// What admission decided for a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityAdmissionResult {
    /// A canonical company with this name already exists.
    Existing(Uuid),
    /// A verified candidate was inserted as a canonical company.
    Created(Uuid),
    /// The candidate was queued for analyst review; no company was created.
    /// The id is the review-queue row id.
    ReviewRequired(Uuid),
    /// The candidate was below the "worth verifying" threshold (or had no
    /// usable name); nothing was written.
    Rejected,
}

/// A pending entity-review row produced by admission.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityReviewEntry {
    /// Deterministic id of the candidate this review belongs to.
    pub candidate_id: Uuid,
    /// Display name of the candidate.
    pub candidate_name: String,
    /// Normalized candidate name.
    pub normalized_name: String,
    /// Aggregated evidence confidence at the time of review.
    pub confidence: f64,
    /// Verifier outcome (always `analyst_review` today, kept for audit).
    pub outcome: String,
    /// Why the candidate could not be auto-admitted.
    pub review_reasons: Vec<String>,
    /// Every evidence signal observed, persisted row-for-row alongside.
    pub evidence: Vec<crate::entity_verifier::EvidenceSignal>,
    /// Discovery metadata carried by the candidate.
    pub metadata: serde_json::Value,
    /// Which discovery path produced the candidate.
    pub source: String,
}

/// The persistence operations admission needs.
///
/// Implemented for the production `PgStore` by the worker's
/// `PgAdmissionStore` adapter; tests use [`InMemoryAdmissionStore`].
#[async_trait]
pub trait AdmissionStore: Send + Sync {
    /// Find an existing canonical company id by (case-insensitive) name.
    async fn find_company_id_by_name(&self, name: &str) -> Result<Option<Uuid>>;

    /// Insert a company that has passed verification. Implementations must
    /// be idempotent for the same company id.
    async fn insert_verified_company(&self, company: &Company) -> Result<()>;

    /// Enqueue (or return the existing pending) review-queue row.
    async fn enqueue_entity_review(&self, entry: &EntityReviewEntry) -> Result<Uuid>;
}

/// Admission configuration.
#[derive(Debug, Clone)]
pub struct EntityAdmissionConfig {
    /// Minimum candidate extraction confidence worth verifying. Below this
    /// the candidate is [`EntityAdmissionResult::Rejected`] — it is never
    /// verified and never inserted.
    pub minimum_candidate_confidence: f64,
    /// Value stored in `discovered_via` metadata / review `source`.
    pub admission_source: String,
    /// `company_type` label used for admitted companies.
    pub company_type_label: String,
}

impl Default for EntityAdmissionConfig {
    fn default() -> Self {
        Self {
            minimum_candidate_confidence: 0.45,
            admission_source: "entity_admission".to_string(),
            company_type_label: "entity_admission".to_string(),
        }
    }
}

/// Canonical admission service: `verifier` + persistence `store`.
pub struct EntityAdmissionService<S: AdmissionStore> {
    verifier: EntityVerifier,
    store: S,
    config: EntityAdmissionConfig,
}

impl<S: AdmissionStore> EntityAdmissionService<S> {
    /// Build a service from a verifier and a store.
    pub fn new(verifier: EntityVerifier, store: S) -> Self {
        Self {
            verifier,
            store,
            config: EntityAdmissionConfig::default(),
        }
    }

    /// Set the minimum "worth verifying" confidence.
    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.config.minimum_candidate_confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Set the discovery source label recorded on reviews and companies.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.config.admission_source = source.into();
        self
    }

    /// Set the `company_type` label used for admitted companies.
    pub fn with_company_type_label(mut self, label: impl Into<String>) -> Self {
        self.config.company_type_label = label.into();
        self
    }

    /// The verifier's policy (read-only).
    pub fn verifier(&self) -> &EntityVerifier {
        &self.verifier
    }

    /// Evaluate one candidate end-to-end.
    pub async fn evaluate_company_candidate(
        &mut self,
        candidate: &CompanyCandidate,
    ) -> Result<EntityAdmissionResult> {
        let display_name = candidate.raw_name.trim();
        if display_name.is_empty() || candidate.normalized_name.trim().is_empty() {
            return Ok(EntityAdmissionResult::Rejected);
        }
        if candidate.extraction_confidence < self.config.minimum_candidate_confidence {
            return Ok(EntityAdmissionResult::Rejected);
        }

        if let Some(existing) = self.store.find_company_id_by_name(display_name).await? {
            return Ok(EntityAdmissionResult::Existing(existing));
        }

        let verification = self.verifier.verify(candidate).await;
        if verification.outcome == VerificationOutcome::Verified {
            let company = canonical_company(candidate, &verification, &self.config);
            self.store.insert_verified_company(&company).await?;
            return Ok(EntityAdmissionResult::Created(company.id));
        }

        let entry = review_entry(candidate, &verification, &self.config);
        let review_id = self.store.enqueue_entity_review(&entry).await?;
        Ok(EntityAdmissionResult::ReviewRequired(review_id))
    }
}

fn canonical_company(
    candidate: &CompanyCandidate,
    verification: &VerificationResult,
    config: &EntityAdmissionConfig,
) -> Company {
    let name = candidate.raw_name.trim();
    let mut company = Company::new(name, CompanyType::Other(config.company_type_label.clone()));
    company.id = candidate_id_for(&candidate.normalized_name);

    let metadata_domain = candidate
        .metadata
        .get("website")
        .or_else(|| candidate.metadata.get("domain"))
        .and_then(|value| domain_of(value));
    company.domain = metadata_domain.clone();
    company.legal_name = candidate
        .metadata
        .get("legal_name")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    company.country_code = candidate
        .metadata
        .get("country_code")
        .cloned()
        .filter(|value| !value.trim().is_empty());
    company.region = candidate
        .metadata
        .get("region")
        .cloned()
        .filter(|value| !value.trim().is_empty());

    let mut metadata = serde_json::Map::new();
    for (key, value) in &candidate.metadata {
        metadata.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    metadata.insert(
        "discovered_via".to_string(),
        serde_json::Value::String(config.admission_source.clone()),
    );
    metadata.insert(
        "admission_decision".to_string(),
        serde_json::Value::String("verified".to_string()),
    );
    metadata.insert(
        "verification_outcome".to_string(),
        serde_json::Value::String(verification.outcome.as_str().to_string()),
    );
    metadata.insert(
        "verification_confidence".to_string(),
        serde_json::json!(verification.confidence),
    );
    metadata.insert(
        "verification_methods".to_string(),
        serde_json::json!(verification.verification_methods),
    );
    metadata.insert(
        "verified_at".to_string(),
        serde_json::Value::String(verification.verified_at.to_rfc3339()),
    );
    company.metadata = serde_json::Value::Object(metadata);

    company.created_at = verification.verified_at;
    company.updated_at = verification.verified_at;
    company
}

fn review_entry(
    candidate: &CompanyCandidate,
    verification: &VerificationResult,
    config: &EntityAdmissionConfig,
) -> EntityReviewEntry {
    let mut metadata = serde_json::Map::new();
    for (key, value) in &candidate.metadata {
        metadata.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    metadata.insert(
        "candidate_extraction_confidence".to_string(),
        serde_json::json!(candidate.extraction_confidence),
    );

    EntityReviewEntry {
        candidate_id: candidate_id_for(&candidate.normalized_name),
        candidate_name: candidate.raw_name.trim().to_string(),
        normalized_name: candidate.normalized_name.clone(),
        confidence: verification.confidence,
        outcome: verification.outcome.as_str().to_string(),
        review_reasons: verification.review_reasons.clone(),
        evidence: verification.evidence.clone(),
        metadata: serde_json::Value::Object(metadata),
        source: config.admission_source.clone(),
    }
}

/// In-memory [`AdmissionStore`] for unit tests. Implements the same
/// semantics as the Postgres adapter: name lookup is case-insensitive,
/// re-inserting the same company id is an upsert, and review enqueue returns
/// the existing pending row for a candidate.
#[derive(Debug, Default)]
pub struct InMemoryAdmissionStore {
    state: std::sync::Mutex<InMemoryAdmissionState>,
}

#[derive(Debug, Default)]
struct InMemoryAdmissionState {
    companies: Vec<Company>,
    review_queue: Vec<(Uuid, EntityReviewEntry)>,
}

impl InMemoryAdmissionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of companies actually inserted (upserts included).
    pub fn insert_calls(&self) -> usize {
        self.lock().companies.len()
    }

    /// Snapshot of admitted companies.
    pub fn companies(&self) -> Vec<Company> {
        self.lock().companies.clone()
    }

    /// Snapshot of review-queue entries.
    pub fn review_queue(&self) -> Vec<EntityReviewEntry> {
        self.lock()
            .review_queue
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, InMemoryAdmissionState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[async_trait]
impl AdmissionStore for InMemoryAdmissionStore {
    async fn find_company_id_by_name(&self, name: &str) -> Result<Option<Uuid>> {
        let needle = normalize_company_name(name);
        Ok(self
            .lock()
            .companies
            .iter()
            .find(|company| normalize_company_name(&company.name) == needle)
            .map(|company| company.id))
    }

    async fn insert_verified_company(&self, company: &Company) -> Result<()> {
        let mut state = self.lock();
        if let Some(existing) = state
            .companies
            .iter_mut()
            .find(|existing| existing.id == company.id)
        {
            *existing = company.clone();
        } else {
            state.companies.push(company.clone());
        }
        Ok(())
    }

    async fn enqueue_entity_review(&self, entry: &EntityReviewEntry) -> Result<Uuid> {
        let mut state = self.lock();
        if let Some((id, _)) = state
            .review_queue
            .iter()
            .find(|(_, queued)| queued.candidate_id == entry.candidate_id)
        {
            return Ok(*id);
        }
        let id = Uuid::new_v4();
        state.review_queue.push((id, entry.clone()));
        Ok(id)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company_discovery::DiscoverySource;
    use crate::entity_verifier::{EvidenceSignal, StaticEvidenceProvider, VerificationType};
    use std::collections::HashMap;

    fn make_candidate(name: &str, confidence: f64) -> CompanyCandidate {
        CompanyCandidate {
            raw_name: name.to_string(),
            normalized_name: normalize_company_name(name),
            source: DiscoverySource::WebCrawl,
            extraction_confidence: confidence,
            context_snippet: String::new(),
            metadata: HashMap::new(),
        }
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

    fn independent_signal() -> EvidenceSignal {
        EvidenceSignal::new(
            VerificationType::IndependentSourceDomains,
            "Multi-source corroboration",
            Some("https://news-a.example/story"),
            "news-a.example, news-b.example",
            0.85,
        )
    }

    fn verified_service() -> EntityAdmissionService<InMemoryAdmissionStore> {
        let verifier = EntityVerifier::without_providers().with_provider(Box::new(
            StaticEvidenceProvider::new(
                "fake",
                vec![
                    registry_signal("Acme Corp"),
                    domain_signal("Acme Corp"),
                    independent_signal(),
                ],
            ),
        ));
        EntityAdmissionService::new(verifier, InMemoryAdmissionStore::new())
            .with_source("test_discovery")
            .with_company_type_label("test_discovered")
    }

    #[tokio::test]
    async fn below_threshold_is_rejected_without_insert_or_review() {
        let mut service = verified_service();
        let candidate = make_candidate("Acme Corp", 0.30);
        let result = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        assert_eq!(result, EntityAdmissionResult::Rejected);
        assert_eq!(service.store.insert_calls(), 0);
        assert!(service.store.review_queue().is_empty());
    }

    #[tokio::test]
    async fn verified_candidate_inserts_once_and_is_idempotent() {
        let mut service = verified_service();
        let candidate = make_candidate("Acme Corp", 0.8);

        let first = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        let EntityAdmissionResult::Created(company_id) = first else {
            panic!("verified candidate must be created, got {first:?}");
        };
        assert_eq!(service.store.insert_calls(), 1);
        let companies = service.store.companies();
        assert_eq!(companies.len(), 1);
        assert_eq!(companies[0].id, company_id);
        assert_eq!(companies[0].name, "Acme Corp");
        assert!(companies[0]
            .metadata
            .get("verification_confidence")
            .is_some());

        let second = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        assert_eq!(second, EntityAdmissionResult::Existing(company_id));
        assert_eq!(
            service.store.insert_calls(),
            1,
            "second pass must not insert"
        );
        assert!(service.store.review_queue().is_empty());
    }

    #[tokio::test]
    async fn unverified_candidate_goes_to_review_queue_without_company() {
        let verifier = EntityVerifier::without_providers();
        let mut service = EntityAdmissionService::new(verifier, InMemoryAdmissionStore::new())
            .with_source("test_discovery");
        let candidate = make_candidate("Ghost Company Inc", 0.8);

        let first = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        let EntityAdmissionResult::ReviewRequired(_) = first else {
            panic!("unverified candidate must be queued, got {first:?}");
        };
        assert_eq!(service.store.insert_calls(), 0);
        assert_eq!(service.store.review_queue().len(), 1);

        // Re-evaluating the same candidate reuses the pending review row.
        let second = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        assert_eq!(second, first);
        assert_eq!(service.store.review_queue().len(), 1);
    }

    #[tokio::test]
    async fn ambiguous_contradictory_candidate_goes_to_review_with_reasons() {
        let verifier = EntityVerifier::without_providers().with_provider(Box::new(
            StaticEvidenceProvider::new(
                "contradictory",
                vec![
                    registry_signal("Acme Corporation"),
                    EvidenceSignal::new(
                        VerificationType::GleifLei,
                        "GLEIF",
                        Some("https://gleif.example/lei/1"),
                        "Zenith Holdings GmbH",
                        0.95,
                    ),
                    domain_signal("Acme Corp"),
                    independent_signal(),
                ],
            ),
        ));
        let mut service = EntityAdmissionService::new(verifier, InMemoryAdmissionStore::new());
        let candidate = make_candidate("Acme Corp", 0.8);

        let result = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        assert!(matches!(result, EntityAdmissionResult::ReviewRequired(_)));
        assert_eq!(service.store.insert_calls(), 0);

        let queued = service.store.review_queue();
        assert_eq!(queued.len(), 1);
        assert!(
            queued[0]
                .review_reasons
                .iter()
                .any(|reason| reason.contains("contradictory")),
            "review reasons must explain the contradiction: {:?}",
            queued[0].review_reasons
        );
        assert!(!queued[0].evidence.is_empty());
    }

    #[tokio::test]
    async fn nameless_candidate_is_rejected() {
        let mut service = verified_service();
        let mut candidate = make_candidate("Acme Corp", 0.9);
        candidate.raw_name = "   ".to_string();
        let result = match service.evaluate_company_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => panic!("admission must not fail: {error}"),
        };
        assert_eq!(result, EntityAdmissionResult::Rejected);
        assert_eq!(service.store.insert_calls(), 0);
        assert!(service.store.review_queue().is_empty());
    }
}
