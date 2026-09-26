//! Canonical entity-admission wiring for worker discovery jobs.
//!
//! Discovery paths (crawl/nightly dynamic discovery, POI/org discovery) must
//! go through [`build_entity_admission_service`] instead of calling
//! `PgStore::insert_company` directly. The service runs the production
//! evidence-backed verifier (real GLEIF/SEC EDGAR/RDAP/corporate-site/
//! registry providers, evidence persisted to `entity_verification_evidence`)
//! and only creates a canonical company after
//! `VerificationOutcome::Verified`; everything else is queued in
//! `entity_review_queue` (migration 058).

use anyhow::Result;
use apex_core::entities::Company;
use apex_insights::entity_admission::{AdmissionStore, EntityAdmissionService, EntityReviewEntry};
use apex_insights::entity_providers::{build_production_entity_verifier, EntityVerifierCaps};
use apex_store::postgres::{EntityReviewRow, PgStore};
use async_trait::async_trait;
use uuid::Uuid;

pub use apex_insights::entity_admission::EntityAdmissionResult;

/// Minimum candidate confidence worth *verifying*. This is a pre-filter only:
/// it can never admit a company.
pub const ENTITY_ADMISSION_MIN_CONFIDENCE: f64 = 0.45;

/// [`AdmissionStore`] over `PgStore` (`companies` + `entity_review_queue`).
pub struct PgAdmissionStore {
    store: PgStore,
}

impl PgAdmissionStore {
    /// Build from a pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            store: PgStore { pool },
        }
    }

    /// Build from a shared store (clones the pool handle).
    pub fn from_store(store: &PgStore) -> Self {
        Self {
            store: PgStore {
                pool: store.pool.clone(),
            },
        }
    }
}

#[async_trait]
impl AdmissionStore for PgAdmissionStore {
    async fn find_company_id_by_name(&self, name: &str) -> Result<Option<Uuid>> {
        Ok(self
            .store
            .get_company_by_name_ci(name)
            .await?
            .map(|company| company.id))
    }

    async fn insert_verified_company(&self, company: &Company) -> Result<()> {
        self.store.insert_company(company).await
    }

    async fn enqueue_entity_review(&self, entry: &EntityReviewEntry) -> Result<Uuid> {
        let row = EntityReviewRow {
            candidate_id: entry.candidate_id,
            candidate_name: entry.candidate_name.clone(),
            normalized_name: entry.normalized_name.clone(),
            confidence: entry.confidence,
            outcome: entry.outcome.clone(),
            review_reasons: entry.review_reasons.clone(),
            evidence: serde_json::to_value(&entry.evidence)?,
            metadata: entry.metadata.clone(),
            source: entry.source.clone(),
        };
        self.store.enqueue_entity_review(&row).await
    }
}

/// Build the production admission service used by discovery jobs.
///
/// Uses [`build_production_entity_verifier`] with the process environment's
/// capability flags (`ENTITY_VERIFY_*`, `ENTITY_REGISTRY_ENDPOINTS`) and
/// persists evidence through the `PgStore` pool.
pub fn build_entity_admission_service(
    store: &PgStore,
    admission_source: &str,
    company_type_label: &str,
) -> EntityAdmissionService<PgAdmissionStore> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent("ApexIntelBot/1.0 (+https://apex-intel.io/bot)")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let verifier =
        build_production_entity_verifier(store.pool.clone(), http, EntityVerifierCaps::from_env());
    EntityAdmissionService::new(verifier, PgAdmissionStore::from_store(store))
        .with_min_confidence(ENTITY_ADMISSION_MIN_CONFIDENCE)
        .with_source(admission_source)
        .with_company_type_label(company_type_label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    async fn production_service_uses_non_seed_providers() {
        let pool = match PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://apex:apex@127.0.0.1:5432/apex_entity_admission_test")
        {
            Ok(pool) => pool,
            Err(error) => panic!("lazy pool construction must not connect: {error}"),
        };
        let store = PgStore { pool };
        let service = build_entity_admission_service(&store, "test_discovery", "test_discovered");
        let names = service.verifier().provider_names();
        for expected in [
            "gleif",
            "sec_edgar",
            "rdap",
            "corporate_site",
            "independent_sources",
        ] {
            assert!(
                names.contains(&expected),
                "production admission service is missing provider {expected}: {names:?}"
            );
        }
        assert!(
            !names.contains(&"seed_directory"),
            "production admission service must not use the seed-only verifier: {names:?}"
        );
    }
}
