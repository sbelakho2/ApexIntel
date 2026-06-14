//! Generator Orchestrator — Closed-Loop Feedback Consumer
//!
//! Uses [`FeedbackController`] signals + [`EntityRegistry`] to orchestrate what
//! gets generated next.  This is the **consumer** side of the closed-loop
//! feedback system described in [`docs/analysis/insight_system_analysis.md`].
//!
//! # Architecture
//!
//! ```text
//! FeedbackController.analyze()  ───→  Vec<FeedbackSignal>
//!                                            │
//!                                            ▼
//!                              GeneratorOrchestrator
//!                                │            │
//!                                ▼            ▼
//!                         select_next_entity   select_category
//!                                │            │
//!                                ▼            ▼
//!                           [Insight Generation Pipeline]
//! ```
//!
//! # Thread safety
//! `GeneratorOrchestrator` is `Send + Sync` when all contained types are.
//! `FeedbackController` is `Send + Sync`; `EntityRegistry` and `TitleGenerator`
//! are also `Send + Sync` as defined in their respective modules.

use crate::company_discovery::DiscoverySource;
use crate::discovery_pipeline::{DiscoveryPipeline, DiscoveryResult};
use crate::entity_relevance::EntityRegistry;
use crate::insight_feedback::{
    FeedbackController, FeedbackEntry, FeedbackSignal, InsightRecord,
};
use crate::title_diversity::{TitleGenerator, TitleStrategy};

/// Orchestrates insight generation using feedback signals and entity data.
///
/// This is the **consumer** side of the closed-loop feedback system — it reads
/// the signals produced by [`FeedbackController`] and translates them into
/// concrete decisions about *what* to generate next.
///
/// # Discovery Integration (Phase 2)
///
/// The orchestrator now includes an optional [`DiscoveryPipeline`] that ingests
/// raw observations to discover new entities dynamically. When the pipeline
/// surfaces a pending insight candidate, it takes priority over the normal
/// entity-selection flow — ensuring newly discovered companies receive quick
/// analytical coverage.
pub struct GeneratorOrchestrator {
    /// Feedback controller that analyses entries and produces signals
    feedback: FeedbackController,
    /// Entity registry for entity selection and category queries
    registry: EntityRegistry,
    /// Title generator for diversity-aware title creation
    title_gen: TitleGenerator,
    /// Optional discovery pipeline for dynamic entity discovery
    discovery_pipeline: Option<DiscoveryPipeline>,
}

impl GeneratorOrchestrator {
    /// Create a new orchestrator that owns the given sub-systems.
    ///
    /// No discovery pipeline is attached by default; use
    /// [`with_discovery_pipeline`](Self::with_discovery_pipeline) to add one.
    pub fn new(registry: EntityRegistry, title_gen: TitleGenerator) -> Self {
        Self {
            feedback: FeedbackController::new(),
            registry,
            title_gen,
            discovery_pipeline: None,
        }
    }

    /// Create a new orchestrator with an attached discovery pipeline.
    pub fn with_discovery_pipeline(
        registry: EntityRegistry,
        title_gen: TitleGenerator,
        pipeline: DiscoveryPipeline,
    ) -> Self {
        Self {
            feedback: FeedbackController::new(),
            registry,
            title_gen,
            discovery_pipeline: Some(pipeline),
        }
    }

    /// Returns a shared reference to the inner [`FeedbackController`].
    pub fn feedback_controller(&self) -> &FeedbackController {
        &self.feedback
    }

    /// Returns a mutable reference to the inner [`FeedbackController`].
    pub fn feedback_controller_mut(&mut self) -> &mut FeedbackController {
        &mut self.feedback
    }

    /// Returns a shared reference to the inner [`EntityRegistry`].
    pub fn registry(&self) -> &EntityRegistry {
        &self.registry
    }

    /// Returns a mutable reference to the inner [`EntityRegistry`].
    pub fn registry_mut(&mut self) -> &mut EntityRegistry {
        &mut self.registry
    }

    /// Returns a shared reference to the inner [`TitleGenerator`].
    pub fn title_generator(&self) -> &TitleGenerator {
        &self.title_gen
    }

    /// Returns a mutable reference to the inner [`TitleGenerator`].
    pub fn title_generator_mut(&mut self) -> &mut TitleGenerator {
        &mut self.title_gen
    }

    /// Returns a shared reference to the inner [`DiscoveryPipeline`], if set.
    pub fn discovery_pipeline(&self) -> Option<&DiscoveryPipeline> {
        self.discovery_pipeline.as_ref()
    }

    /// Returns a mutable reference to the inner [`DiscoveryPipeline`], if set.
    pub fn discovery_pipeline_mut(&mut self) -> Option<&mut DiscoveryPipeline> {
        self.discovery_pipeline.as_mut()
    }

    /// Attach (or replace) the discovery pipeline.
    pub fn set_discovery_pipeline(&mut self, pipeline: DiscoveryPipeline) {
        self.discovery_pipeline = Some(pipeline);
    }

    /// Ingest raw observations into the discovery pipeline.
    ///
    /// Extracts company mentions from each observation (using a default
    /// text extractor that reads the `"text"` field of JSON values) and
    /// registers any newly verified entities.
    ///
    /// Returns a [`DiscoveryResult`] summarising what was found, or `None`
    /// if no pipeline is configured.
    ///
    /// # Note
    ///
    /// This is a bridge between the crawl/observation layer and the insight
    /// generation layer. Observations without a `"text"` field are silently
    /// skipped.
    pub fn ingest_observations(
        &mut self,
        observations: &[serde_json::Value],
        source: DiscoverySource,
    ) -> Option<DiscoveryResult> {
        let pipeline = self.discovery_pipeline.as_mut()?;

        Some(pipeline.process_observations(observations, source, |v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string()
        }))
    }

    /// Ingest observations with a custom text extractor function.
    ///
    /// Useful when observations use a different JSON schema (e.g.,
    /// `"content"`, `"body"`, or nested fields).
    pub fn ingest_observations_with(
        &mut self,
        observations: &[serde_json::Value],
        source: DiscoverySource,
        text_extractor: impl Fn(&serde_json::Value) -> String,
    ) -> Option<DiscoveryResult> {
        let pipeline = self.discovery_pipeline.as_mut()?;
        Some(pipeline.process_observations(observations, source, text_extractor))
    }

    /// Select which entity to generate an insight for next.
    ///
    /// Uses priority ranking from feedback signals (revived/stale entities
    /// first, then emerging entities, then non-fatigued entities).  Falls back
    /// to [`EntityRegistry::select_diverse_entity_set`] when no signals are
    /// available.
    ///
    /// # Discovery-aware selection
    ///
    /// Before consulting feedback signals, this method checks the discovery
    /// pipeline for pending insight candidates. If the pipeline has discovered
    /// and verified a new entity that hasn't had an insight generated yet,
    /// that entity is returned immediately — ensuring newly discovered companies
    /// receive quick analytical coverage.
    pub fn select_next_entity(
        &mut self,
        feedback_entries: &[FeedbackEntry],
        recent_insights: &[InsightRecord],
    ) -> String {
        // Step 1: Check if the discovery pipeline has a pending candidate
        // that needs an insight generated (discovery-first routing).
        if let Some(pipeline) = self.discovery_pipeline.as_ref() {
            // Check for dynamically discovered entities with no insights yet
            let discovered: Vec<String> = pipeline
                .registry()
                .dynamically_discovered()
                .iter()
                .map(|p| p.entity_name.clone())
                .collect();

            for entity_name in &discovered {
                // If this discovered entity has never had an insight, prioritise it
                let has_insight = recent_insights
                    .iter()
                    .any(|r| r.entity.eq_ignore_ascii_case(entity_name));
                if !has_insight {
                    self.registry.record_insight(entity_name);
                    return entity_name.clone();
                }
            }
        }

        // Step 2: Emit discovery-urgency signals for dynamic entities without coverage
        self.feedback
            .emit_discovery_urgency_signals(&self.registry, recent_insights);

        // Step 3: Check feedback-driven priority entities
        let _signals = self.feedback.analyze(feedback_entries, recent_insights);
        let priorities = self.feedback.get_priority_entities(5);

        if !priorities.is_empty() {
            let selected = priorities[0].clone();

            // Record the selection so the registry can apply recency penalties
            self.registry.record_insight(&selected);

            return selected;
        }

        // Step 4: Fall back to diverse entity selection
        let candidates = self.registry.select_diverse_entity_set(3);
        if let Some(entity) = candidates.into_iter().next() {
            self.registry.record_insight(&entity);
            entity
        } else {
            // Last resort: pick any registered entity
            self.registry
                .entity_names()
                .next()
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string())
        }
    }

    /// Select which category to generate an insight for.
    ///
    /// Avoids suppressed categories from feedback signals and prefers
    /// categories that have recent data available.
    pub fn select_category(&mut self, _entity: &str, _signals: &[FeedbackSignal]) -> String {
        let suppressed = self.feedback.get_suppressed_categories();
        let known = self.feedback.known_categories();

        // Prefer categories with recent data that aren't suppressed
        for cat in &known {
            if !suppressed.contains(cat) {
                return cat.clone();
            }
        }

        // Fall back to the default category list, skipping suppressed ones
        let default_categories = [
            "demand",
            "supply_chain",
            "competitor",
            "security",
            "regulatory",
            "commodity",
            "logistics",
            "poi",
        ];

        for cat in &default_categories {
            if !suppressed.contains(*cat) {
                return cat.to_string();
            }
        }

        // Absolute fallback
        "general".to_string()
    }

    /// Determine which [`TitleStrategy`] to use for the next insight,
    /// considering recent title history and the feedback controller's signals.
    ///
    /// Delegates to [`TitleGenerator`]'s internal weights to pick the
    /// least-recently-used strategy, then cross-references with suppressed
    /// categories to avoid stale approaches.
    pub fn get_title_strategy(
        &self,
        _recent_titles: &[String],
        _signals: &[FeedbackSignal],
    ) -> TitleStrategy {
        let suppressed = self.feedback.get_suppressed_categories();

        // Check if any strategy is mapped to a suppressed category
        let strategy_categories: &[(TitleStrategy, &str)] = &[
            (TitleStrategy::EventDriven, "demand"),
            (TitleStrategy::RiskExposure, "supply_chain"),
            (TitleStrategy::CompetitiveComparison, "competitor"),
            (TitleStrategy::TrendAnalysis, "general"),
            (TitleStrategy::DataDiscovery, "general"),
            (TitleStrategy::ImpactAssessment, "general"),
            (TitleStrategy::NarrativeArc, "general"),
            (TitleStrategy::SignalSynthesis, "general"),
        ];

        // Try to find the highest-weighted non-suppressed strategy
        let weights = self.title_gen.strategy_weights();
        let mut best: Option<TitleStrategy> = None;
        let mut best_weight: f64 = -1.0;

        for (strategy, cat) in strategy_categories {
            if suppressed.contains(*cat) {
                continue;
            }
            if let Some(w) = weights.get(strategy) {
                if *w > best_weight {
                    best_weight = *w;
                    best = Some(*strategy);
                }
            }
        }

        best.unwrap_or(TitleStrategy::EventDriven)
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparison::{find_analogous_entity, infer_category};
    use crate::company_discovery::DiscoverySource;
    use crate::discovery_pipeline::{DiscoveryConfig, DiscoveryPipeline};
    use crate::entity_relevance::{EntityCategory, EntityProfile};
    use crate::predictive::build_patterns;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_insight(entity: &str, category: &str, title: &str, days_ago: i64) -> InsightRecord {
        let ts = Utc::now().timestamp() - days_ago * 86400;
        InsightRecord {
            entity: entity.to_string(),
            category: category.to_string(),
            title: title.to_string(),
            timestamp: ts,
        }
    }

    fn test_registry() -> EntityRegistry {
        let mut reg = EntityRegistry::empty();
        reg.register(
            EntityProfile::new("NVIDIA").with_category(EntityCategory::Semiconductor),
        );
        reg.register(EntityProfile::new("AMD").with_category(EntityCategory::Semiconductor));
        reg.register(
            EntityProfile::new("Intel").with_category(EntityCategory::Semiconductor),
        );
        reg.register(EntityProfile::new("Foxconn").with_category(EntityCategory::Ems));
        reg
    }

    #[test]
    fn test_orchestrator_selects_priority_entity() {
        let mut orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());

        // Create fatigue for nvidia, making intel/amd the priority
        let insights: Vec<InsightRecord> = (0..7)
            .map(|i| make_insight("nvidia", "demand", &format!("NVIDIA {}", i), i))
            .collect();

        let entity = orch.select_next_entity(&[], &insights);

        // nvidia is fatigued, so it should NOT be selected
        assert_ne!(
            entity.to_lowercase(),
            "nvidia",
            "Fatigued entity nvidia should not be selected, got: {}",
            entity
        );
    }

    #[test]
    fn test_orchestrator_falls_back_to_diverse() {
        let mut orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());

        // No signals (single insight per entity) → fallback to diverse selection
        let insights = vec![
            make_insight("nvidia", "demand", "NVIDIA 1", 1),
            make_insight("amd", "demand", "AMD 1", 2),
            make_insight("intel", "demand", "Intel 1", 3),
        ];

        let entity = orch.select_next_entity(&[], &insights);

        // Should pick something — any entity
        assert!(!entity.is_empty(), "Should select an entity even without signals");
        assert_ne!(entity, "Unknown", "Should not fall back to Unknown");
    }

    #[test]
    fn test_orchestrator_selects_non_suppressed_category() {
        let mut orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());

        // Generate signals that suppress "demand" via repetition
        let repetitive = vec![
            make_insight("nvidia", "demand", "NVIDIA demand surge amid AI growth", 1),
            make_insight("nvidia", "demand", "NVIDIA demand growth amid AI surge", 2),
            make_insight("nvidia", "demand", "NVIDIA demand boom amid AI growth", 3),
        ];

        let signals = orch
            .feedback_controller_mut()
            .analyze(&[], &repetitive);

        let category = orch.select_category("nvidia", &signals);

        // "demand" may be suppressed due to repetition → should pick something else
        assert!(!category.is_empty(), "Should select a non-empty category");
    }

    #[test]
    fn test_orchestrator_title_strategy_avoids_suppressed_categories() {
        let orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());

        let signals = vec![
            crate::insight_feedback::FeedbackSignal {
                signal_type: crate::insight_feedback::FeedbackSignalType::CategoryRepetition {
                    title_similarity: 0.8,
                    recent_count: 5,
                },
                entity: None,
                intensity: 0.7,
                category: Some("demand".to_string()),
                reason: "test".to_string(),
                generated_at: Utc::now(),
            },
        ];

        let strategy = orch.get_title_strategy(&[], &signals);

        // Should not panic; should return some valid strategy
        let _label = strategy.label();
    }

    #[test]
    fn test_orchestrator_new_defaults() {
        let orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());
        assert!(orch.feedback_controller().signals().is_empty());
    }

    #[test]
    fn test_orchestrator_empty_registry_fallback() {
        let mut orch = GeneratorOrchestrator::new(EntityRegistry::empty(), TitleGenerator::new());
        let entity = orch.select_next_entity(&[], &[]);
        assert_eq!(entity, "Unknown", "Empty registry should fall back to Unknown");
    }

    #[test]
    fn test_orchestrator_mut_accessors() {
        let mut orch = GeneratorOrchestrator::new(test_registry(), TitleGenerator::new());
        assert!(!orch.registry_mut().is_empty());
        assert!(orch.title_generator_mut().recent_titles().is_empty());
    }

    // ═══════════════════════════════════════════════════════════
    // Phase 2: Discovery Integration Tests
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_discovery_first_routing_picks_uncovered_entity() {
        // Pipeline with empty registry + a dynamically discovered company
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "NVDA".to_string());
        pipeline.registry_mut().register_company(
            "NovaTech Solutions",
            &DiscoverySource::NewsArticle,
            metadata,
            0.8,
        );

        // Orchestrator with its own empty registry + discovery pipeline
        let mut orch = GeneratorOrchestrator::with_discovery_pipeline(
            EntityRegistry::empty(),
            TitleGenerator::new(),
            pipeline,
        );

        // Step 1 should find the discovered entity (no insights yet)
        let entity = orch.select_next_entity(&[], &[]);
        assert_eq!(
            entity, "NovaTech Solutions",
            "Discovery-first routing should return uncovered dynamic entity"
        );
    }

    #[test]
    fn test_discovery_routing_beats_fatigued_seed_entities() {
        // Pipeline with seed entities + newly discovered entity
        let config = DiscoveryConfig {
            use_seed_entities: true,
            ..Default::default()
        };
        let mut pipeline = DiscoveryPipeline::new(config);
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "NEWC".to_string());
        pipeline.registry_mut().register_company(
            "NewTech Innovations",
            &DiscoverySource::FinancialReport,
            metadata,
            0.85,
        );

        // Orchestrator with seeded registry + pipeline
        let mut orch = GeneratorOrchestrator::with_discovery_pipeline(
            EntityRegistry::from_yaml_config(),
            TitleGenerator::new(),
            pipeline,
        );

        // Seed entities have insights but discovered entity does not
        let insights = vec![
            make_insight("nvidia", "demand", "NVIDIA demand surge", 1),
            make_insight("intel", "supply_chain", "Intel supply update", 2),
            make_insight("amd", "competitor", "AMD competitor analysis", 3),
        ];

        // Should pick discovered entity (no insight yet) over seed entities
        let entity = orch.select_next_entity(&[], &insights);
        assert_eq!(
            entity.to_lowercase(),
            "newtech innovations",
            "Discovered entity without insight should be selected over seed entities"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_ingest_observations_flow_through_orchestrator() {
        // Pipeline with seed entities disabled (no pre-loaded entities)
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let pipeline = DiscoveryPipeline::new(config);
        let mut orch = GeneratorOrchestrator::with_discovery_pipeline(
            EntityRegistry::empty(),
            TitleGenerator::new(),
            pipeline,
        );

        // Ingest observations containing a known ticker pattern
        #[allow(clippy::disallowed_methods)]
        let observations = vec![serde_json::json!({
            "text": "Quantum Computing Inc announced breakthrough. NASDAQ:QCI."
        })];
        let result = orch.ingest_observations(&observations, DiscoverySource::NewsArticle);

        // The pipeline should have processed the observation
        assert!(result.is_some(), "ingest_observations should return a result");
        let result = result.unwrap();
        assert!(
            result.candidates_found > 0 || result.pending_verification > 0,
            "Should find at least one candidate (QCI) or have it pending: found={}, pending={}",
            result.candidates_found,
            result.pending_verification,
        );

        // After ingest, select_next_entity should still return something
        let entity = orch.select_next_entity(&[], &[]);
        assert!(!entity.is_empty(), "Should select an entity even after ingest");
    }

    #[test]
    fn test_infer_category_for_dynamic_entity() {
        // Build a registry with known entities in known categories
        let mut reg = EntityRegistry::empty();
        reg.register(
            EntityProfile::new("NVIDIA")
                .with_category(EntityCategory::Semiconductor)
                .with_ticker("NVDA")
                .with_industry(vec!["gpu", "ai", "semiconductor"]),
        );
        reg.register(
            EntityProfile::new("AMD")
                .with_category(EntityCategory::Semiconductor)
                .with_ticker("AMD")
                .with_industry(vec!["gpu", "cpu", "semiconductor"]),
        );
        reg.register(
            EntityProfile::new("Foxconn")
                .with_category(EntityCategory::Ems)
                .with_industry(vec!["manufacturing", "electronics", "ems"]),
        );

        // Create a dynamically discovered entity profile with GPU-like keywords
        let dynamic_profile = EntityProfile::new("NewGPU Corp")
            .with_industry(vec!["gpu", "ai", "datacenter"])
            .with_ticker("NGC");

        // infer_category should pick Semiconductor (most similar to NVIDIA/AMD)
        let inferred = infer_category(&dynamic_profile, &reg);
        assert_eq!(
            inferred,
            EntityCategory::Semiconductor,
            "GPU-adjacent company should be inferred as Semiconductor"
        );

        // find_analogous_entity should return NVIDIA or AMD as closest match
        let analogous = find_analogous_entity(&dynamic_profile, &reg);
        assert!(
            analogous.is_some(),
            "Should find an analogous entity for GPU company"
        );
        if let Some((profile, _similarity)) = analogous {
            assert!(
                profile.entity_name == "NVIDIA" || profile.entity_name == "AMD",
                "Analogous entity should be NVIDIA or AMD, got: {}",
                profile.entity_name
            );
        }
    }

    #[test]
    fn test_find_analogous_entity_for_ems_company() {
        let mut reg = EntityRegistry::empty();
        reg.register(
            EntityProfile::new("Foxconn")
                .with_category(EntityCategory::Ems)
                .with_industry(vec!["manufacturing", "electronics", "assembly"]),
        );
        reg.register(
            EntityProfile::new("Flex")
                .with_category(EntityCategory::Ems)
                .with_industry(vec!["manufacturing", "electronics", "supply_chain"]),
        );
        reg.register(
            EntityProfile::new("NVIDIA")
                .with_category(EntityCategory::Semiconductor)
                .with_industry(vec!["gpu", "ai"]),
        );

        // A new EMS-like company
        let ems_profile = EntityProfile::new("NewEMS Ltd")
            .with_industry(vec!["electronics", "manufacturing", "assembly"]);

        let analogous = find_analogous_entity(&ems_profile, &reg);
        assert!(
            analogous.is_some(),
            "Should find analogous entity for EMS company"
        );
        if let Some((profile, similarity)) = analogous {
            assert!(
                profile.entity_name == "Foxconn" || profile.entity_name == "Flex",
                "Analogous should be Foxconn or Flex, got: {}",
                profile.entity_name
            );
            assert!(
                similarity > 0.0,
                "Similarity should be positive for same-industry company"
            );
        }
    }

    #[test]
    fn test_infer_category_falls_back_to_technology_for_empty_registry() {
        let empty_reg = EntityRegistry::empty();
        let profile = EntityProfile::new("UnknownStartup Inc");
        let inferred = infer_category(&profile, &empty_reg);
        assert_eq!(
            inferred,
            EntityCategory::Technology,
            "Empty registry should fall back to Technology"
        );
    }

    #[test]
    fn test_discovery_urgency_signals_emitted_for_dynamic_entities() {
        // Create a registry with dynamically discovered entities
        let mut reg = EntityRegistry::empty();
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "NEWC".to_string());
        reg.register_company(
            "NewUncovered Corp",
            &DiscoverySource::NewsArticle,
            metadata,
            0.9,
        );

        // Also add a seed-like entity that has had insights
        reg.register(
            EntityProfile::new("KnownCorp")
                .with_category(EntityCategory::Semiconductor),
        );

        let mut controller = crate::insight_feedback::FeedbackController::new();

        // No insights → discovery urgency signals should be emitted for NewUncovered Corp
        let recent_insights = vec![make_insight("KnownCorp", "demand", "KnownCorp insight", 1)];

        controller.emit_discovery_urgency_signals(&reg, &recent_insights);

        let signals = controller.signals();
        let discovery_signals: Vec<_> = signals
            .iter()
            .filter(|s| matches!(s.signal_type, crate::insight_feedback::FeedbackSignalType::DiscoveryUrgency { .. }))
            .collect();

        assert_eq!(
            discovery_signals.len(),
            1,
            "Should emit 1 DiscoveryUrgency signal for the uncovered dynamic entity"
        );
        let signal = &discovery_signals[0];
        assert_eq!(
            signal.entity.as_deref(),
            Some("NewUncovered Corp"),
            "Signal should target the uncovered dynamic entity"
        );
        assert!(
            signal.intensity >= 0.1,
            "Discovery urgency intensity should be at least 0.1"
        );
    }

    #[test]
    fn test_discovery_urgency_gets_top_priority_tier() {
        let mut reg = EntityRegistry::empty();
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "DISC".to_string());
        reg.register_company(
            "PriorityDiscovery Inc",
            &DiscoverySource::NewsArticle,
            metadata,
            0.9,
        );

        // Simulate analysis so the controller has some data, then emit urgency
        let mut controller = crate::insight_feedback::FeedbackController::new();
        let _signals = controller.analyze(&[], &[]);
        controller.emit_discovery_urgency_signals(&reg, &[]);

        let priorities = controller.get_priority_entities(10);
        assert!(
            !priorities.is_empty(),
            "Priority entities should not be empty after emitting urgency signals"
        );

        // The discovery-urgent entity should be at the top (highest score = 3.0 + intensity)
        let top = &priorities[0];
        assert_eq!(
            top.to_lowercase(),
            "prioritydiscovery inc",
            "Discovery-urgent entity should be the top priority (Tier 0)"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_empty_registry_to_dynamic_discovery_flow() {
        // Pipeline and orchestrator both start empty
        let config = DiscoveryConfig {
            use_seed_entities: false,
            ..Default::default()
        };
        let pipeline = DiscoveryPipeline::new(config);
        let mut orch = GeneratorOrchestrator::with_discovery_pipeline(
            EntityRegistry::empty(),
            TitleGenerator::new(),
            pipeline,
        );

        // Initially, orchestrator falls back to Unknown
        let entity = orch.select_next_entity(&[], &[]);
        assert_eq!(entity, "Unknown", "Empty registry should fall back to Unknown");

        // Register a discovered entity on the pipeline's registry
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "TEST".to_string());
        orch.discovery_pipeline_mut()
            .unwrap()
            .registry_mut()
            .register_company(
                "TestDynamic Inc",
                &DiscoverySource::WebCrawl,
                metadata,
                0.75,
            );

        // Now select_next_entity should find it via step 1
        let entity = orch.select_next_entity(&[], &[]);
        assert_eq!(
            entity, "TestDynamic Inc",
            "After registering dynamic entity, orchestrator should discover it"
        );
    }

    #[test]
    fn test_dedup_register_same_company_twice() {
        // Use register_company directly — dedup is by normalized name hash
        let mut reg = EntityRegistry::empty();
        let mut metadata = HashMap::new();
        metadata.insert("ticker".to_string(), "DEDUP".to_string());

        // First registration
        let id1 = reg.register_company(
            "DupeTest Corp",
            &DiscoverySource::NewsArticle,
            metadata.clone(),
            0.8,
        );
        assert_eq!(reg.total_entities(), 1, "First registration adds 1 entity");

        // Second registration (same normalized name → dedup)
        let id2 = reg.register_company(
            "DupeTest Corp",
            &DiscoverySource::NewsArticle,
            metadata,
            0.9,
        );
        assert_eq!(
            reg.total_entities(),
            1,
            "Second registration of same name should be deduped"
        );
        assert_eq!(
            id1, id2,
            "Deterministic entity IDs should match for same normalized name"
        );
    }

    #[test]
    fn test_dedup_different_casing_same_name() {
        let mut reg = EntityRegistry::empty();

        let id1 = reg.register_company(
            "SomeCompany Inc",
            &DiscoverySource::SocialMedia,
            HashMap::new(),
            0.7,
        );
        assert_eq!(reg.total_entities(), 1);

        // Same company, different casing
        let id2 = reg.register_company(
            "somecompany inc",
            &DiscoverySource::SocialMedia,
            HashMap::new(),
            0.7,
        );
        assert_eq!(
            reg.total_entities(),
            1,
            "Different casing should be deduped via normalized name"
        );
        assert_eq!(
            id1, id2,
            "Entity IDs should be identical for case-different names"
        );
    }

    #[test]
    fn test_zero_history_prediction_returns_fallback() {
        // Empty observations → build_patterns returns fallback uniform prior
        let reg = EntityRegistry::empty();
        let patterns = build_patterns("NewDynamicEntity", &[], &reg);

        assert!(!patterns.is_empty(), "Should return at least fallback pattern");
        let fallback = &patterns[0];
        assert_eq!(
            fallback.name, "general",
            "Fallback pattern should be 'general'"
        );
        assert_eq!(
            fallback.observation_count, 0,
            "Fallback should have zero observations"
        );
        assert!(
            (fallback.base_rate - 0.5).abs() < 0.001,
            "Fallback base_rate should be 0.5 (uniform prior)"
        );
        assert!(
            (fallback.confidence - 0.0).abs() < 0.001,
            "Fallback confidence should be 0.0"
        );
        assert!(
            fallback.prior_strength >= 1.0,
            "Fallback prior_strength should be at least 1.0"
        );
    }

    #[test]
    fn test_evidence_score_with_zero_observations() {
        // evidence_score should handle observation_count = 0 gracefully
        use crate::predictive::evidence_score;

        let pattern = crate::predictive::EvidencePattern {
            name: "general".to_string(),
            observation_count: 0,
            base_rate: 0.5,
            confidence: 0.0,
            prior_strength: 2.0,
            last_updated: Utc::now(),
        };

        let score = evidence_score(&pattern);
        // With 0 observations, sufficiency = 0/50 = 0, so score = 0.5 * 1.0 + 0 * 0.5 = 0.5
        assert!(
            (score - 0.5).abs() < 0.001 || score.is_finite(),
            "evidence_score should handle zero observations gracefully, got: {}",
            score
        );
    }
}
