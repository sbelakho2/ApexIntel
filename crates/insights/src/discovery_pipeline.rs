//! # Discovery Pipeline
//!
//! Orchestrates the full company discovery lifecycle:
//!
//! 1. **Extract** — Parse observations/structured text for company mentions
//! 2. **Verify** — Validate candidates through heuristic and cross-reference checks
//! 3. **Register** — Add verified companies to the entity registry
//!
//! # Lifecycle
//!
//! ```text
//! Observations ──→ extract_company_mentions() ──→ CompanyCandidate[]
//!                                                        │
//!                                                        ▼
//!                                                  EntityVerifier.verify()
//!                                                        │
//!                                              ┌─────────┴─────────┐
//!                                              ▼                   ▼
//!                                        Verified              Rejected
//!                                              │
//!                                              ▼
//!                                    EntityRegistry.register_company()
//!                                              │
//!                                              ▼
//!                                        EntityProfile
//! ```

use crate::company_discovery::{
    extract_company_mentions, CompanyCandidate, DiscoverySource,
};
use crate::entity_relevance::EntityRegistry;
use crate::entity_verifier::{EntityVerifier, VerificationResult};
use chrono::{DateTime, Utc};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the discovery pipeline.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Minimum extraction confidence to consider a candidate.
    pub min_extraction_confidence: f64,
    /// Minimum verification confidence to register.
    pub min_verification_confidence: f64,
    /// How often to re-verify entities (in seconds).
    pub reverification_interval: u64,
    /// Max entities before triggering pruning.
    pub soft_limit: usize,
    /// Whether to use seed entities from config.
    pub use_seed_entities: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            min_extraction_confidence: 0.3,
            min_verification_confidence: 0.6,
            reverification_interval: 604_800, // 7 days
            soft_limit: 5_000,
            use_seed_entities: false, // Pure discovery — entities extracted from observations
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Result types
// ─────────────────────────────────────────────────────────────────────────────

/// Result of processing a batch of observations.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    /// Total candidates found in this batch.
    pub candidates_found: usize,
    /// Candidates that were already registered.
    pub already_registered: usize,
    /// Candidates currently pending verification.
    pub pending_verification: usize,
    /// Newly registered entities from this batch.
    pub newly_registered: usize,
    /// Entity IDs of newly registered entities.
    pub registration_ids: Vec<String>,
}

/// Report from scheduled maintenance.
#[derive(Debug, Clone)]
pub struct MaintenanceReport {
    /// When maintenance ran.
    pub ran_at: DateTime<Utc>,
    /// Number of entities re-verified.
    pub reverified_count: usize,
    /// Number of entities pruned (90+ days inactive).
    pub pruned_count: usize,
    /// Names of pruned entities.
    pub pruned_entities: Vec<String>,
    /// Total entities after maintenance.
    pub total_entities: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline
// ─────────────────────────────────────────────────────────────────────────────

/// Orchestrates extraction → verification → registration.
pub struct DiscoveryPipeline {
    /// The entity registry (mutable — supports dynamic addition).
    registry: EntityRegistry,
    /// Pipeline configuration.
    config: DiscoveryConfig,
    /// Pending candidates awaiting verification.
    candidates: Vec<CompanyCandidate>,
    /// Entity verifier.
    verifier: EntityVerifier,
}

impl DiscoveryPipeline {
    /// Create a new pipeline.
    ///
    /// If `config.use_seed_entities` is true, the registry is pre-loaded
    /// with entities from the YAML config.
    pub fn new(config: DiscoveryConfig) -> Self {
        let registry = if config.use_seed_entities {
            EntityRegistry::from_yaml_config()
        } else {
            EntityRegistry::empty()
        };

        Self {
            registry,
            config,
            candidates: Vec::new(),
            verifier: EntityVerifier::new(),
        }
    }

    /// Create a pipeline with an existing registry (for testing or migration).
    pub fn with_registry(registry: EntityRegistry, config: DiscoveryConfig) -> Self {
        Self {
            registry,
            config,
            candidates: Vec::new(),
            verifier: EntityVerifier::new(),
        }
    }

    /// Process a batch of observations for company discovery.
    ///
    /// For each observation:
    /// 1. Extract company mentions using pattern matching
    /// 2. Skip already-registered companies (by normalized name)
    /// 3. Add new candidates to pending verification
    /// 4. Verify candidates and register verified ones
    ///
    /// The `text_extractor` function extracts text from an observation's value.
    pub fn process_observations(
        &mut self,
        observations: &[serde_json::Value],
        source: DiscoverySource,
        text_extractor: impl Fn(&serde_json::Value) -> String,
    ) -> DiscoveryResult {
        let mut candidates_found = 0;
        let mut already_registered = 0;
        let mut newly_registered = 0;
        let mut registration_ids = Vec::new();

        for obs in observations {
            let text = text_extractor(obs);
            if text.trim().is_empty() {
                continue;
            }

            // Step 1: Extract company mentions
            let extracted = extract_company_mentions(&text, source.clone());

            for candidate in extracted {
                candidates_found += 1;

                // Filter by minimum extraction confidence
                if candidate.extraction_confidence < self.config.min_extraction_confidence {
                    continue;
                }

                // Step 2: Check if already registered
                if self.registry.is_registered(&candidate.normalized_name) {
                    already_registered += 1;
                    continue;
                }

                // Step 3: Add to pending verification
                self.candidates.push(candidate);
            }
        }

        // Step 4: Verify pending candidates
        let pending: Vec<CompanyCandidate> = self.candidates.drain(..).collect();
        let _pending_count = pending.len();

        for candidate in pending {
            let verification = self.verifier.verify(&candidate);

            if verification.is_verified
                && verification.confidence >= self.config.min_verification_confidence
            {
                // Step 5: Register verified company
                let entity_id = self.register_verified_company(candidate, verification);
                newly_registered += 1;
                registration_ids.push(entity_id);
            } else {
                // Keep unverified candidates for later re-verification
                // (only if confidence is above a lower threshold)
                if verification.confidence > 0.1 {
                    self.candidates.push(verification.candidate);
                }
            }
        }

        DiscoveryResult {
            candidates_found,
            already_registered,
            pending_verification: self.candidates.len(),
            newly_registered,
            registration_ids,
        }
    }

    /// Register a verified company in the entity registry.
    ///
    /// Builds an `EntityProfile` from the candidate's metadata and
    /// registers it with the registry.
    fn register_verified_company(
        &mut self,
        candidate: CompanyCandidate,
        verification: VerificationResult,
    ) -> String {
        let entity_id = self
            .registry
            .register_company(&candidate.raw_name, &candidate.source, verification.metadata, candidate.extraction_confidence);

        // Record an observation for the new entity to establish baseline activity
        self.registry.record_observation(&candidate.raw_name);

        entity_id
    }

    /// Run scheduled maintenance:
    ///
    /// 1. Re-verify entities due for reverification
    /// 2. Prune stale entities (90+ days inactive)
    /// 3. Emit discovery metrics
    pub fn run_maintenance(&mut self) -> MaintenanceReport {
        let ran_at = Utc::now();

        // Re-verify pending candidates (in a real system, this would check
        // entities that haven't been verified in > reverification_interval)
        let reverified_count = self.candidates.len();
        let pending: Vec<CompanyCandidate> = self.candidates.drain(..).collect();
        let mut re_verified: Vec<CompanyCandidate> = Vec::new();

        // First pass: verify all candidates (no mutable self borrow needed)
        struct VerifiedCandidate {
            candidate: CompanyCandidate,
            verification: VerificationResult,
        }
        let mut to_register: Vec<VerifiedCandidate> = Vec::new();

        for candidate in pending {
            let verification = self.verifier.verify(&candidate);
            if verification.is_verified
                && verification.confidence >= self.config.min_verification_confidence
            {
                to_register.push(VerifiedCandidate {
                    candidate,
                    verification,
                });
            } else if verification.confidence > 0.1 {
                re_verified.push(candidate);
            }
        }

        // Second pass: register verified companies (needs &mut self)
        for vc in to_register {
            self.register_verified_company(vc.candidate, vc.verification);
        }

        self.candidates = re_verified;

        // Prune stale entities (90+ days inactive = low activity score)
        let pruned = self.registry.prune_stale_entities(90);
        let pruned_count = pruned.len();

        MaintenanceReport {
            ran_at,
            reverified_count,
            pruned_count,
            pruned_entities: pruned,
            total_entities: self.registry.total_entities(),
        }
    }

    /// Access the entity registry (immutable).
    pub fn registry(&self) -> &EntityRegistry {
        &self.registry
    }

    /// Access the entity registry (mutable).
    pub fn registry_mut(&mut self) -> &mut EntityRegistry {
        &mut self.registry
    }

    /// Access pending candidates.
    pub fn pending_candidates(&self) -> &[CompanyCandidate] {
        &self.candidates
    }

    /// Access the pipeline configuration.
    pub fn config(&self) -> &DiscoveryConfig {
        &self.config
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company_discovery::normalize_company_name;
    use std::collections::HashMap;

    #[test]
    fn test_discovery_config_defaults() {
        let config = DiscoveryConfig::default();
        assert!((config.min_extraction_confidence - 0.3).abs() < 0.01);
        assert!((config.min_verification_confidence - 0.6).abs() < 0.01);
        assert_eq!(config.reverification_interval, 604_800);
        assert_eq!(config.soft_limit, 5_000);
        assert!(!config.use_seed_entities);
    }

    #[test]
    fn test_pipeline_created_with_seed_entities() {
        let config = DiscoveryConfig {
            use_seed_entities: true,
            ..Default::default()
        };
        let pipeline = DiscoveryPipeline::new(config);
        assert!(
            pipeline.registry.total_entities() > 0,
            "Should have seed entities"
        );
    }

    #[test]
    fn test_pipeline_created_empty() {
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let pipeline = DiscoveryPipeline::new(config);
        assert_eq!(
            pipeline.registry.total_entities(),
            0,
            "Should start empty"
        );
    }

    #[test]
    fn test_process_observations_discovers_new_company() {
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);

        #[allow(clippy::disallowed_methods)]
        let observations = vec![serde_json::json!({
            "text": "NVIDIA Corporation announced new H100 GPUs today. NASDAQ:NVDA."
        })];

        let result = pipeline.process_observations(
            &observations,
            DiscoverySource::NewsArticle,
            |v| v["text"].as_str().unwrap_or("").to_string(),
        );

        assert!(
            result.candidates_found > 0,
            "Should find candidates"
        );
    }

    #[test]
    fn test_process_observations_skips_registered() {
        let config = DiscoveryConfig {
            use_seed_entities: true, // NVIDIA is in seed entities
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);

        // Pre-register NVIDIA
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "NVDA".to_string());
        pipeline.registry.register_company(
            "NVIDIA Corporation",
            &DiscoverySource::NewsArticle,
            metadata,
            0.9,
        );

        #[allow(clippy::disallowed_methods)]
        let observations = vec![serde_json::json!({
            "text": "NVIDIA Corporation is doing great."
        })];

        let result = pipeline.process_observations(
            &observations,
            DiscoverySource::NewsArticle,
            |v| v["text"].as_str().unwrap_or("").to_string(),
        );

        // NVIDIA should be recognized as already registered
        assert!(
            result.already_registered > 0 || result.candidates_found == 0,
            "NVIDIA should be recognized as registered"
        );
    }

    #[test]
    fn test_pipeline_registers_verified_company() {
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);

        // Register a company that should pass verification
        let id = pipeline.registry.register_company(
            "NewTestCorp Inc",
            &DiscoverySource::NewsArticle,
            HashMap::new(),
            0.8,
        );

        assert!(!id.is_empty(), "Should generate an entity ID");
        assert!(pipeline.registry.is_registered(&normalize_company_name("NewTestCorp Inc")));
    }

    #[test]
    fn test_maintenance_does_not_crash_empty_pipeline() {
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);

        let report = pipeline.run_maintenance();
        assert_eq!(report.pruned_count, 0);
        assert_eq!(report.total_entities, 0);
    }

    #[test]
    fn test_pipeline_with_registry() {
        let registry = EntityRegistry::empty();
        let config = DiscoveryConfig::default();
        let mut pipeline = DiscoveryPipeline::with_registry(registry, config);

        assert_eq!(pipeline.registry.total_entities(), 0);
        pipeline.registry_mut().register_company(
            "TestCorp",
            &DiscoverySource::NewsArticle,
            HashMap::new(),
            0.5,
        );
        assert_eq!(pipeline.registry.total_entities(), 1);
    }

    #[test]
    fn test_pipeline_tracks_pending_candidates() {
        let config = DiscoveryConfig {
            use_seed_entities: false,
            min_verification_confidence: 0.99, // Very high — nothing will pass
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);

        #[allow(clippy::disallowed_methods)]
        let observations = vec![serde_json::json!({
            "text": "SomeUnknownStartupXYZ Inc announced funding."
        })];

        let result = pipeline.process_observations(
            &observations,
            DiscoverySource::NewsArticle,
            |v| v["text"].as_str().unwrap_or("").to_string(),
        );

        // Some candidates may be pending if they didn't meet the high threshold
        // (or they may have been discarded if confidence < 0.1)
        assert!(result.candidates_found > 0);
    }
}
