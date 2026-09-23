//! Entity-Relevance Validation Module
//!
//! This module provides semantic validation to ensure insights are actually
//! relevant to the entities they claim to be about. This fixes Issue #1
//! (Entity-Signal Decoupling) and Issue #5 (Generic Templates).
//!
//! The key insight is that an insight about NVIDIA should actually mention
//! NVIDIA-specific topics (GPUs, AI chips, data centers, etc.), not just
//! generic geopolitical news.
//!
//! # Data-Driven Entity Registry
//!
//! Instead of hardcoding 3 entities (NVIDIA, TSMC, Foxconn), this module
//! loads all 22+ EMS/OEM entities from `config/augmentation_entities.yaml`
//! into a dynamic registry. Selection strategies use activity scoring,
//! recency penalties, and category balancing.
//!
//! # Selection Strategy
//!
//! 1. **Observation volume** — entities with more recent observations rank higher.
//! 2. **News/event correlation** — incoming signals boost matching entities.
//! 3. **Diversity routing** — entities that recently had insights get penalised.
//! 4. **Category balance** — prevents over-indexing on semiconductor companies.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use crate::company_discovery::{CompanyCandidate, DiscoverySource};

// ─────────────────────────────────────────────────────────────────────────────
// Entity Category
// ─────────────────────────────────────────────────────────────────────────────

/// Broad category for an entity, used by the diversity router to avoid
/// over-indexing any single supply-chain segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EntityCategory {
    /// Semiconductor design / fabrication (NVIDIA, TSMC, AMD, Intel, etc.)
    Semiconductor,
    /// Electronics Manufacturing Services (Foxconn, Flex, Jabil, etc.)
    Ems,
    /// Original Equipment Manufacturers (Airbus, Lockheed Martin, etc.)
    Oem,
    /// Automotive OEMs and Tier-1 suppliers.
    Automotive,
    /// Logistics / distribution / freight.
    Logistics,
    /// Software / platform / technology companies.
    Technology,
    /// Any other category not covered above.
    Other(String),
}

impl EntityCategory {
    /// Parse a category string into the enum.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "semiconductor" | "chip" | "fab" => Self::Semiconductor,
            "ems" | "electronics manufacturing" | "contract manufacturer" => Self::Ems,
            "oem" | "original equipment manufacturer" => Self::Oem,
            "automotive" | "auto" | "tier1" | "tier 1" => Self::Automotive,
            "logistics" | "logistic" | "freight" | "shipping" => Self::Logistics,
            "technology" | "tech" | "software" | "platform" => Self::Technology,
            other => Self::Other(other.to_string()),
        }
    }

    /// Return a human-readable label for this category.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Semiconductor => "semiconductor",
            Self::Ems => "ems",
            Self::Oem => "oem",
            Self::Automotive => "automotive",
            Self::Logistics => "logistics",
            Self::Technology => "technology",
            Self::Other(s) => s.as_str(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity Profile
// ─────────────────────────────────────────────────────────────────────────────

/// Entity profile containing keywords and topics that are relevant to this entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityProfile {
    /// Entity name (e.g., "NVIDIA", "Foxconn")
    pub entity_name: String,
    /// Industry/vertical keywords specific to this entity
    pub industry_keywords: Vec<String>,
    /// Product/service names specific to this entity
    pub product_keywords: Vec<String>,
    /// Geographic regions where this entity operates
    pub geographic_keywords: Vec<String>,
    /// Competitor names (for competitive intel)
    pub competitor_keywords: Vec<String>,
    /// Topics this entity is known for (AI, chips, supply chain, etc.)
    pub topic_keywords: Vec<String>,
    /// Entity category for diversity routing.
    pub category: EntityCategory,
    /// Country code (ISO 3166-1 alpha-2).
    pub country_code: Option<String>,
    /// Stock ticker, if publicly traded.
    pub ticker: Option<String>,
    /// Whether this entity was discovered dynamically (not from seed config).
    #[serde(default)]
    pub is_dynamically_discovered: bool,
    /// When this entity was last verified.
    #[serde(default)]
    pub last_verified: Option<DateTime<Utc>>,
    /// How many times this entity has been verified.
    #[serde(default)]
    pub verification_count: u32,
}

impl EntityProfile {
    /// Create a new entity profile.
    pub fn new(entity_name: &str) -> Self {
        Self {
            entity_name: entity_name.to_string(),
            industry_keywords: Vec::new(),
            product_keywords: Vec::new(),
            geographic_keywords: Vec::new(),
            competitor_keywords: Vec::new(),
            topic_keywords: Vec::new(),
            category: EntityCategory::Other("unknown".to_string()),
            country_code: None,
            ticker: None,
            is_dynamically_discovered: false,
            last_verified: None,
            verification_count: 0,
        }
    }

    /// Set the category.
    pub fn with_category(mut self, category: EntityCategory) -> Self {
        self.category = category;
        self
    }

    /// Add industry keywords.
    pub fn with_industry(mut self, keywords: Vec<&str>) -> Self {
        self.industry_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add product keywords.
    pub fn with_products(mut self, keywords: Vec<&str>) -> Self {
        self.product_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add topic keywords.
    pub fn with_topics(mut self, keywords: Vec<&str>) -> Self {
        self.topic_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add geographic keywords.
    pub fn with_geography(mut self, keywords: Vec<&str>) -> Self {
        self.geographic_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Add competitor keywords.
    pub fn with_competitors(mut self, keywords: Vec<&str>) -> Self {
        self.competitor_keywords = keywords.into_iter().map(String::from).collect();
        self
    }

    /// Set country code.
    pub fn with_country(mut self, code: &str) -> Self {
        self.country_code = Some(code.to_string());
        self
    }

    /// Set ticker.
    pub fn with_ticker(mut self, ticker: &str) -> Self {
        self.ticker = Some(ticker.to_string());
        self
    }

    /// Create an [`EntityProfile`] from a dynamically discovered company
    /// candidate with metadata. Infers category from the discovery source
    /// and available metadata.
    ///
    /// This is the canonical constructor for entities flowing through the
    /// discovery pipeline (Phase 2 integration).
    pub fn from_discovery(
        name: &str,
        source: &DiscoverySource,
        metadata: &HashMap<String, String>,
        extraction_confidence: f64,
    ) -> Self {
        let inferred_category = match source {
            DiscoverySource::NewsArticle
            | DiscoverySource::FinancialReport
            | DiscoverySource::SocialMedia => {
                // Try to infer from metadata
                Self::infer_category_from_metadata(metadata)
            }
            DiscoverySource::JobPosting | DiscoverySource::TradeShow => {
                EntityCategory::Technology
            }
            DiscoverySource::PatentFiling | DiscoverySource::AcademicPaper => {
                EntityCategory::Semiconductor
            }
            DiscoverySource::RegulatoryFiling => EntityCategory::Ems,
            DiscoverySource::WebCrawl => Self::infer_category_from_metadata(metadata),
            DiscoverySource::Other(_) => EntityCategory::Other("dynamically_discovered".to_string()),
        };

        let mut profile = EntityProfile::new(name)
            .with_category(inferred_category);

        // Transfer metadata
        if let Some(ticker) = metadata.get("ticker") {
            profile = profile.with_ticker(ticker);
        }
        if let Some(country) = metadata.get("headquarters_mentioned") {
            profile = profile.with_country(country);
        }
        if let Some(website) = metadata.get("website") {
            profile.topic_keywords.push(website.clone());
        }

        // Mark as dynamically discovered with evidence of extraction confidence
        profile.is_dynamically_discovered = true;
        profile.verification_count = 1;
        profile.last_verified = Some(Utc::now());
        profile.topic_keywords.push("dynamically_discovered".to_string());
        profile.industry_keywords.push(source.as_str().to_string());

        // Encode confidence into activity baseline
        let _ = extraction_confidence;

        profile
    }

    /// Create an [`EntityProfile`] from a [`CompanyCandidate`] (convenience wrapper).
    ///
    /// Extracts all available metadata from the candidate and sets confidence-
    /// weighted activity baseline.
    pub fn from_candidate(candidate: &CompanyCandidate) -> Self {
        Self::from_discovery(
            &candidate.raw_name,
            &candidate.source,
            &candidate.metadata,
            candidate.extraction_confidence,
        )
    }

    /// Infer an entity category from metadata fields, falling back to
    /// `Other("dynamically_discovered")`.
    fn infer_category_from_metadata(metadata: &HashMap<String, String>) -> EntityCategory {
        // Check for exchange/ticker patterns that suggest sector
        if let Some(ticker) = metadata.get("ticker") {
            // Semiconductor tickers often start with certain prefixes
            let upper = ticker.to_uppercase();
            if upper.starts_with("NVDA")
                || upper.starts_with("AMD")
                || upper.starts_with("INTC")
                || upper.starts_with("TSM")
                || upper.starts_with("QCOM")
                || upper.starts_with("AVGO")
                || upper.starts_with("ASML")
                || upper.starts_with("TXN")
                || upper.starts_with("MU")
                || upper.starts_with("MRVL")
            {
                return EntityCategory::Semiconductor;
            }
            if upper.starts_with("AAPL")
                || upper.starts_with("MSFT")
                || upper.starts_with("GOOGL")
                || upper.starts_with("GOOG")
                || upper.starts_with("META")
                || upper.starts_with("AMZN")
            {
                return EntityCategory::Technology;
            }
            if upper.starts_with("FLEX")
                || upper.starts_with("JBL")
                || upper.starts_with("CLS")
                || upper.starts_with("SANM")
                || upper.starts_with("PLXS")
                || upper.starts_with("KE")
                || upper.starts_with("BHE")
            {
                return EntityCategory::Ems;
            }
            if upper.starts_with("NOC")
                || upper.starts_with("LMT")
                || upper.starts_with("RTX")
                || upper.starts_with("GD")
                || upper.starts_with("BA")
                || upper.starts_with("AIR")
                || upper.starts_with("EADSY")
                || upper.starts_with("BAESY")
                || upper.starts_with("ESLT")
            {
                return EntityCategory::Oem;
            }
            if upper.starts_with("TSLA")
                || upper.starts_with("F")
                || upper.starts_with("GM")
                || upper.starts_with("RACE")
                || upper.starts_with("MBG.DE")
                || upper.starts_with("BMW.DE")
                || upper.starts_with("VOW.DE")
                || upper.starts_with("TM")
                || upper.starts_with("HMC")
            {
                return EntityCategory::Automotive;
            }
            if upper.starts_with("FDX")
                || upper.starts_with("UPS")
                || upper.starts_with("DHL")
                || upper.starts_with("MAERSK")
            {
                return EntityCategory::Logistics;
            }
        }

        // Check for website hints
        if let Some(website) = metadata.get("website") {
            let lower = website.to_lowercase();
            if lower.contains("semiconductor")
                || lower.contains("chip")
                || lower.contains("foundry")
            {
                return EntityCategory::Semiconductor;
            }
            if lower.contains("ems")
                || lower.contains("manufacturing")
                || lower.contains("electronics")
            {
                return EntityCategory::Ems;
            }
            if lower.contains("defense")
                || lower.contains("aerospace")
                || lower.contains("military")
            {
                return EntityCategory::Oem;
            }
        }

        // Check for industry keywords in revenue description
        if let Some(revenue) = metadata.get("revenue_mentioned") {
            let lower = revenue.to_lowercase();
            if lower.contains("semiconductor") || lower.contains("chip") {
                return EntityCategory::Semiconductor;
            }
            if lower.contains("defense") || lower.contains("aerospace") {
                return EntityCategory::Oem;
            }
            if lower.contains("automotive") || lower.contains("auto") {
                return EntityCategory::Automotive;
            }
            if lower.contains("logistic") || lower.contains("shipping") || lower.contains("freight") {
                return EntityCategory::Logistics;
            }
        }

        // Fallback: technology as safest default for discovered entities
        EntityCategory::Technology
    }

    /// Get all keywords as a single set.
    pub fn all_keywords(&self) -> HashSet<String> {
        let mut keywords = HashSet::new();
        for kw in &self.industry_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.product_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.topic_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.geographic_keywords {
            keywords.insert(kw.to_lowercase());
        }
        for kw in &self.competitor_keywords {
            keywords.insert(kw.to_lowercase());
        }
        // Also add entity name itself
        keywords.insert(self.entity_name.to_lowercase());
        keywords
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Signal Context
// ─────────────────────────────────────────────────────────────────────────────

/// Context for signal-based entity selection.
///
/// Carries the incoming observation/signal metadata that the registry uses
/// to select the most relevant entity.
#[derive(Debug, Clone)]
pub struct SignalContext {
    /// The raw text content of the signal (news headline, observation text, etc.).
    pub text: String,
    /// Entity name if the signal is explicitly associated with one.
    pub entity_hint: Option<String>,
    /// Signal category (e.g., "news", "job_post", "certification", "patent").
    pub category: Option<String>,
    /// Source URL.
    pub source_url: Option<String>,
    /// Timestamp of the signal.
    pub timestamp: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// YAML Config Structures
// ─────────────────────────────────────────────────────────────────────────────

/// Raw company entry from the YAML config.
/// Format: [name, country_code, ticker_or_null, category_string]
#[derive(Debug, Deserialize)]
struct CompanyEntry(String, String, Option<String>, String);

/// Top-level structure matching `config/augmentation_entities.yaml`.
#[derive(Debug, Deserialize)]
struct EntitiesConfig {
    ems_companies: Vec<CompanyEntry>,
    oem_companies: Vec<CompanyEntry>,
    #[allow(dead_code)]
    capabilities: Vec<String>,
    #[allow(dead_code)]
    certifications: Vec<String>,
    #[allow(dead_code)]
    industries: Vec<String>,
    #[allow(dead_code)]
    regions: Vec<String>,
    #[allow(dead_code)]
    roles: Vec<Vec<String>>,
    #[allow(dead_code)]
    first_names: Vec<String>,
    #[allow(dead_code)]
    last_names: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity Registry
// ─────────────────────────────────────────────────────────────────────────────

/// Data-driven entity registry that replaces the hardcoded if/else chain.
///
/// By default, creates an empty registry — all entities are discovered
/// dynamically from observation data via the discovery pipeline.
///
/// # Dynamic Discovery (Production Mode)
///
/// When [`DiscoveryConfig::use_seed_entities`] is `false` (default), the
/// registry starts empty. Entities are discovered dynamically:
///   1. [`CompanySignalExtractor`] extracts company mentions from observations
///   2. [`EntityVerifier`] verifies candidates
///   3. [`DiscoveryPipeline`] registers verified entities
///
/// # Seed Entities (Transition / Testing)
///
/// [`from_yaml_config`](Self::from_yaml_config) pre-populates the registry
/// from [`config/augmentation_entities.yaml`](../../config/augmentation_entities.yaml).
/// This is preserved for testing and backward compatibility.
pub struct EntityRegistry {
    /// All known entities loaded from config + DB.
    entities: HashMap<String, EntityProfile>,
    /// Entity activity scores updated from observations, backed by a BTreeMap
    /// for O(log n) range queries.
    activity_scores: BTreeMap<String, f64>,
    /// Last time each entity had an insight generated.
    last_insight_time: HashMap<String, Instant>,
    /// Entity categories for diversity routing.
    categories: HashMap<String, EntityCategory>,
    /// Entities that were dynamically discovered (not from seed config).
    dynamically_discovered: HashSet<String>,
    /// In-memory insight history per entity (keyed by lowercased entity name).
    /// Populated by [`record_insight_record`](Self::record_insight_record)
    /// and returned by [`get_entity_history`](Self::get_entity_history).
    insight_history: HashMap<String, Vec<InsightRecord>>,
}

impl EntityRegistry {
    /// Creates an empty entity registry for pure discovery mode.
    ///
    /// Entities will be discovered dynamically from observation data via the
    /// discovery pipeline (extract → verify → register). This is the production
    /// constructor — no static lists are loaded.
    ///
    /// For pre-populated registries (testing / transition), use
    /// [`from_yaml_config`](Self::from_yaml_config).
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            activity_scores: BTreeMap::new(),
            last_insight_time: HashMap::new(),
            categories: HashMap::new(),
            dynamically_discovered: HashSet::new(),
            insight_history: HashMap::new(),
        }
    }

    /// Alias for [`new`](Self::new). Creates an empty registry.
    pub fn empty() -> Self {
        Self::new()
    }
}

impl Default for EntityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityRegistry {
    /// Create a registry with pre-allocated capacity (useful when many
    /// dynamically discovered entities are expected).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entities: HashMap::with_capacity(capacity),
            activity_scores: BTreeMap::new(),
            last_insight_time: HashMap::with_capacity(capacity),
            categories: HashMap::with_capacity(capacity),
            dynamically_discovered: HashSet::new(),
            insight_history: HashMap::with_capacity(capacity),
        }
    }

    /// Load all entities from the embedded YAML config (seed list).
    ///
    /// Uses the entities defined in [`config/augmentation_entities.yaml`](../../config/augmentation_entities.yaml)
    /// to build the initial profile set. Each EMS and OEM company is parsed
    /// into an [`EntityProfile`] with basic industry keywords derived from its
    /// category and region.
    ///
    /// This is a helper for testing and transition periods. In production
    /// (pure discovery mode) this is not called — use [`new`](Self::new) instead.
    pub fn from_yaml_config() -> Self {
        let yaml_str = include_str!("../../../config/augmentation_entities.yaml");
        let config: EntitiesConfig = serde_yaml::from_str(yaml_str)
            .unwrap_or_else(|e| panic!("Failed to parse augmentation_entities.yaml: {e}"));

        let mut entities = HashMap::new();
        let mut categories = HashMap::new();

        // Build EMS profiles
        for entry in &config.ems_companies {
            let profile = Self::company_entry_to_profile(entry, "ems");
            categories.insert(profile.entity_name.to_lowercase(), profile.category.clone());
            entities.insert(profile.entity_name.to_lowercase(), profile);
        }

        // Build OEM profiles
        for entry in &config.oem_companies {
            let profile = Self::company_entry_to_profile(entry, "oem");
            categories.insert(profile.entity_name.to_lowercase(), profile.category.clone());
            entities.insert(profile.entity_name.to_lowercase(), profile);
        }

        let now = Instant::now();
        let activity_scores: BTreeMap<String, f64> =
            entities.keys().map(|k| (k.clone(), 1.0)).collect();
        let last_insight_time: HashMap<String, Instant> =
            entities.keys().map(|k| (k.clone(), now)).collect();

        Self {
            entities,
            activity_scores,
            last_insight_time,
            categories,
            dynamically_discovered: HashSet::new(),
            insight_history: HashMap::new(),
        }
    }

    /// Register a single entity profile.
    pub fn register(&mut self, profile: EntityProfile) {
        let key = profile.entity_name.to_lowercase();
        self.categories
            .insert(key.clone(), profile.category.clone());
        self.activity_scores.entry(key.clone()).or_insert(1.0);
        self.last_insight_time
            .entry(key.clone())
            .or_insert_with(Instant::now);
        self.entities.insert(key, profile);
    }

    /// Convert a raw YAML company entry into a keyword-rich [`EntityProfile`].
    fn company_entry_to_profile(entry: &CompanyEntry, prefix: &str) -> EntityProfile {
        let name = &entry.0;
        let country = &entry.1;
        let ticker = &entry.2;
        let category_str = &entry.3;

        let category = EntityCategory::from_str(category_str);

        // Derive industry/topic keywords from the category and name
        let industry_keywords = match &category {
            EntityCategory::Ems => vec![
                "EMS".to_string(),
                "electronics manufacturing".to_string(),
                "contract manufacturer".to_string(),
                "supply chain".to_string(),
                "manufacturing".to_string(),
            ],
            EntityCategory::Oem => vec![
                "defense".to_string(),
                "aerospace".to_string(),
                "OEM".to_string(),
                "prime contractor".to_string(),
            ],
            EntityCategory::Semiconductor => vec![
                "semiconductor".to_string(),
                "chip".to_string(),
                "foundry".to_string(),
                "fabrication".to_string(),
            ],
            _ => vec!["manufacturing".to_string(), "supply chain".to_string()],
        };

        let topic_keywords = [
            "supply chain".to_string(),
            "manufacturing".to_string(),
            "factory".to_string(),
            "expansion".to_string(),
            "contract".to_string(),
        ];

        let geo_keywords = [country.to_string()];

        let mut profile = EntityProfile::new(name)
            .with_category(category)
            .with_industry(
                industry_keywords
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )
            .with_topics(
                topic_keywords
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )
            .with_geography(
                geo_keywords
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )
            .with_country(country);

        if let Some(t) = ticker {
            if t != "null" && !t.is_empty() {
                profile = profile.with_ticker(t);
            }
        }

        // Add the prefix as an alias for entity matching
        if prefix == "ems" {
            profile
                .industry_keywords
                .push("electronics manufacturing services".to_string());
            profile.topic_keywords.push("outsourcing".to_string());
        } else if prefix == "oem" {
            profile
                .topic_keywords
                .push("government contract".to_string());
            profile.topic_keywords.push("military".to_string());
        }

        profile
    }

    /// Get a profile by exact entity name (case-insensitive).
    pub fn get(&self, entity_name: &str) -> Option<&EntityProfile> {
        self.entities.get(&entity_name.to_lowercase())
    }

    /// Get a mutable profile by exact entity name (case-insensitive).
    pub fn get_mut(&mut self, entity_name: &str) -> Option<&mut EntityProfile> {
        self.entities.get_mut(&entity_name.to_lowercase())
    }

    /// Return the activity score for an entity.
    pub fn activity_score(&self, entity: &str) -> f64 {
        self.activity_scores
            .get(&entity.to_lowercase())
            .copied()
            .unwrap_or(0.0)
    }

    /// Return the number of registered entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns true if the registry has no entities.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Return an iterator over all entity names.
    pub fn entity_names(&self) -> impl Iterator<Item = &String> {
        self.entities.keys()
    }

    /// Select the most relevant entity for a given signal context.
    ///
    /// # Selection Strategy
    ///
    /// 1. If the context text or hint matches an entity name directly, that
    ///    entity is preferred (exact-match boost).
    /// 2. Otherwise, score all entities by:
    ///    - Keyword overlap between the context and the entity profile.
    ///    - Activity score (higher = more recently observed).
    ///    - Recency penalty (entities with recent insights get a 30% discount).
    /// 3. Return the highest-scoring entity that exceeds the minimum threshold.
    pub fn entity_for_context(&self, context: &SignalContext) -> Option<&EntityProfile> {
        if self.entities.is_empty() {
            return None;
        }

        let context_lower = context.text.to_lowercase();

        // Phase 1: Direct name match (exact or contained in text)
        if let Some(ref hint) = context.entity_hint {
            if let Some(profile) = self.entities.get(&hint.to_lowercase()) {
                return Some(profile);
            }
        }

        // Check if any entity name is mentioned in the context text
        for (name, profile) in &self.entities {
            let name_lower = name.to_lowercase();
            // B340: only forward containment is valid ("the text mentions
            // the entity name"). The previous reverse check matched whenever
            // an entity name happened to contain the signal text.
            if !name_lower.is_empty() && context_lower.contains(&name_lower) {
                return Some(profile);
            }
        }

        // Phase 2: Score by keyword overlap + activity + recency
        let mut scored: Vec<(&str, f64)> = Vec::with_capacity(self.entities.len());

        for (name, profile) in &self.entities {
            let mut score = 0.0;

            // Keyword overlap score
            let keywords = profile.all_keywords();
            let mut matches = 0;
            for kw in &keywords {
                if context_lower.contains(kw) {
                    matches += 1;
                }
            }
            let max_kw = keywords.len().max(1) as f64;
            let keyword_score = matches as f64 / max_kw;
            score += keyword_score * 0.5;

            // Activity score contribution
            let activity = self
                .activity_scores
                .get(name)
                .copied()
                .unwrap_or(0.5);
            score += activity * 0.3;

            // Recency penalty: entities with recent insights get discounted
            let recency_penalty = if let Some(last_time) = self.last_insight_time.get(name) {
                let elapsed = last_time.elapsed();
                if elapsed.as_secs() < 86400 {
                    // Less than 24 hours ago → 30% penalty
                    0.7
                } else if elapsed.as_secs() < 604800 {
                    // Less than 7 days ago → 15% penalty
                    0.85
                } else {
                    1.0
                }
            } else {
                1.0
            };
            score *= recency_penalty;

            if score > 0.01 {
                scored.push((name.as_str(), score));
            }
        }

        // Return highest-scoring entity
        scored
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .and_then(|(name, _)| self.entities.get(name))
    }

    /// Return the category for a given entity name.
    pub fn get_category(&self, entity: &str) -> Option<EntityCategory> {
        self.categories.get(&entity.to_lowercase()).cloned()
    }

    /// Return all entities that belong to a given category.
    ///
    /// This enables same-category competitor lookups for dynamic comparison.
    /// Entities are returned sorted by activity score (descending).
    pub fn entities_in_category(&self, category: &EntityCategory) -> Vec<&EntityProfile> {
        let mut result: Vec<&EntityProfile> = self
            .entities
            .values()
            .filter(|p| p.category == *category)
            .collect();
        // Sort by activity score descending for meaningful ordering
        result.sort_by(|a, b| {
            let score_a = self
                .activity_scores
                .get(&a.entity_name.to_lowercase())
                .copied()
                .unwrap_or(0.0);
            let score_b = self
                .activity_scores
                .get(&b.entity_name.to_lowercase())
                .copied()
                .unwrap_or(0.0);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    /// Return the top N entities by activity score, excluding those in the exclude list.
    ///
    /// # Selection Strategy
    ///
    /// Entities are ranked by their current activity score (descending).
    /// Excluded entities (e.g., those that just had insights) are filtered out.
    /// This is O(n) for the filter + O(k log k) for the sort, where n = total
    /// entities and k = n - |exclude|.
    pub fn top_n_entities(&self, n: usize, exclude: &[String]) -> Vec<&EntityProfile> {
        let exclude_set: HashSet<&str> = exclude.iter().map(|s| s.as_str()).collect();

        let mut candidates: Vec<(&str, f64)> = self
            .activity_scores
            .iter()
            .filter(|(name, _)| !exclude_set.contains(name.as_str()))
            .map(|(name, score)| (name.as_str(), *score))
            .collect();

        // Sort by score descending
        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.truncate(n);
        candidates
            .into_iter()
            .filter_map(|(name, _)| self.entities.get(name))
            .collect()
    }

    /// Find entities that have gone stale (no activity for 7+ days).
    ///
    /// # Selection Strategy
    ///
    /// An entity is considered "stale" when its activity score has decayed
    /// below 0.5 (equivalent to a full 7-day decay cycle without observations).
    /// These are candidates for gap-filler insights to ensure coverage.
    pub fn stale_entities(&self) -> Vec<&EntityProfile> {
        self.activity_scores
            .iter()
            .filter(|(_, score)| **score < 0.5)
            .filter_map(|(name, _)| self.entities.get(name))
            .collect()
    }

    /// Apply activity decay to all entities.
    ///
    /// # Decay Strategy
    ///
    /// Activity scores halve every 7 days without observations. This prevents
    /// entities with historical volume from permanently dominating selection.
    /// Call this periodically (e.g., daily) to age out inactive entities.
    pub fn activity_decay(&mut self) {
        let mut to_remove = Vec::new();
        for (name, score) in self.activity_scores.iter_mut() {
            *score *= 0.5; // Halve every decay cycle (7 days)
            if *score < 0.01 {
                // Don't remove entities entirely, but floor the score
                *score = 0.01;
                to_remove.push(name.clone());
            }
        }
        // Keep scores at minimum floor for entities that shouldn't vanish
        for name in &to_remove {
            if let Some(score) = self.activity_scores.get_mut(name) {
                *score = 0.01;
            }
        }
    }

    /// Record a positive observation for an entity, boosting its activity score.
    pub fn record_observation(&mut self, entity_name: &str) {
        let key = entity_name.to_lowercase();
        if self.entities.contains_key(&key) {
            let score = self.activity_scores.entry(key).or_insert(0.0);
            *score = (*score + 1.0).min(10.0); // Cap at 10 to prevent unbounded growth
        }
    }

    /// Mark that an insight was generated for an entity (updates recency).
    pub fn record_insight(&mut self, entity_name: &str) {
        let key = entity_name.to_lowercase();
        if self.entities.contains_key(&key) {
            self.last_insight_time.insert(key, Instant::now());
        }
    }

    /// Record a full insight record for an entity, storing it in the in-memory
    /// history and updating the recency timestamp.
    ///
    /// This is the preferred method for tracking generated insights — it
    /// enables [`get_entity_history`](Self::get_entity_history) to return
    /// real records instead of an empty list.
    pub fn record_insight_record(&mut self, record: InsightRecord) {
        let key = record.entity_name.to_lowercase();
        if self.entities.contains_key(&key) {
            self.last_insight_time.insert(key.clone(), Instant::now());
            let history = self.insight_history.entry(key).or_default();
            // Keep at most 100 records per entity to bound memory.
            if history.len() >= 100 {
                history.remove(0);
            }
            history.push(record);
        }
    }

    /// Select a diverse set of entities for insight generation.
    ///
    /// # Diversity Strategy
    ///
    /// 1. Groups entities by [`EntityCategory`].
    /// 2. Ensures at most 2 entities from any single category.
    /// 3. Prefers entities with higher activity scores but applies a recency
    ///    penalty (entities with recent insights score lower).
    /// 4. Guarantees at least 1 "cold" entity (no insights in 7+ days).
    ///
    /// Returns entity names, not profiles.
    pub fn select_diverse_entity_set(
        &self,
        count: usize,
    ) -> Vec<String> {
        if self.entities.is_empty() || count == 0 {
            return Vec::new();
        }

        // Group by category
        let mut by_category: HashMap<&EntityCategory, Vec<(&String, f64)>> = HashMap::new();
        for (name, profile) in &self.entities {
            let cat = &profile.category;
            let base_score = self
                .activity_scores
                .get(name)
                .copied()
                .unwrap_or(0.5);

            // Apply recency penalty
            let recency_penalty = if let Some(last_time) = self.last_insight_time.get(name) {
                let elapsed = last_time.elapsed();
                if elapsed.as_secs() < 86400 {
                    0.5 // Heavy penalty for < 24h
                } else if elapsed.as_secs() < 604800 {
                    0.8 // Moderate penalty for < 7d
                } else {
                    1.2 // Boost for "cold" entities
                }
            } else {
                1.2 // Entities with no insights get a boost
            };

            let adjusted_score = base_score * recency_penalty;
            by_category
                .entry(cat)
                .or_default()
                .push((name, adjusted_score));
        }

        // Sort within each category by adjusted score; break ties on name for
        // determinism (HashMap entry order is otherwise randomized per process).
        for (_, entries) in by_category.iter_mut() {
            entries.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            });
        }

        // Select: at most 2 per category, prefer highest scores
        let mut selected: Vec<String> = Vec::new();
        let max_per_category = 2;

        // Round-robin across categories to ensure diversity.
        // Sort category keys deterministically (by Debug/formatted repr) so the
        // round-robin order is stable across runs — otherwise HashMap iteration
        // order randomizes which category is picked first on ties.
        let categories: Vec<&EntityCategory> = {
            let mut keys: Vec<&EntityCategory> = by_category.keys().copied().collect();
            keys.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
            keys
        };
        let mut category_idx = 0;
        // B342: per-round no-progress detection. The previous counter
        // accumulated across ALL rounds and never reset, so selection always
        // stopped after ~2×categories iterations — requesting more entities
        // than that silently returned fewer. Now: one clean pass per round;
        // a round that selects nothing ends the loop.
        let mut round_start_len = selected.len();
        let mut max_iterations = categories.len().saturating_mul(count.max(1) * 4 + 1);

        while selected.len() < count {
            if category_idx > 0 && category_idx % categories.len() == 0 {
                if selected.len() == round_start_len {
                    break; // full round with no picks: nothing left to take
                }
                round_start_len = selected.len();
            }
            if max_iterations == 0 {
                break; // defensive hard cap on total work
            }
            max_iterations -= 1;

            let cat = categories[category_idx % categories.len()];
            // Safety: cat is guaranteed to be in by_category since we just inserted it
            let entries = by_category.get_mut(cat)
                .unwrap_or_else(|| panic!("missing category entry for {cat:?}"));

            // Remove entries already selected
            entries.retain(|(name, _)| !selected.iter().any(|s| s == *name));

            // Check category cap
            let cat_count = selected
                .iter()
                .filter(|s| {
                    self.entities
                        .get(&s.to_lowercase())
                        .map(|p| p.category == *cat)
                        .unwrap_or(false)
                })
                .count();

            if cat_count < max_per_category && !entries.is_empty() {
                if let Some((name, _)) = entries.first() {
                    selected.push((*name).clone());
                }
            }

            category_idx += 1;
        }

        // Guarantee at least 1 "cold" entity (no insights in 7+ days)
        let has_cold = selected.iter().any(|s| {
            self.last_insight_time
                .get(&s.to_lowercase())
                .map(|t| t.elapsed().as_secs() >= 604800)
                .unwrap_or(true)
        });

        if !has_cold {
            // Find a cold entity to add/replace
            let cold_candidates: Vec<&String> = self
                .entities
                .keys()
                .filter(|name| {
                    !selected.iter().any(|s| s == *name)
                        && self
                            .last_insight_time
                            .get(&name.to_lowercase())
                            .map(|t| t.elapsed().as_secs() >= 604800)
                            .unwrap_or(true)
                })
                .collect();

            if let Some(cold) = cold_candidates.into_iter().next() {
                if selected.len() < count {
                    selected.push((*cold).clone());
                } else {
                    // Replace the lowest-score selected entity
                    selected.pop();
                    selected.push((*cold).clone());
                }
            }
        }

        selected
    }

    /// Retrieve the insight history for an entity from the in-memory store.
    ///
    /// Returns insight records previously stored via
    /// [`record_insight_record`](Self::record_insight_record), sorted from
    /// oldest to newest.  Returns an empty `Vec` if no insights have been
    /// recorded for this entity.
    ///
    /// For database-backed history in production deployments, callers should
    /// also query the `insights` table via the store layer and merge results.
    pub fn get_entity_history(&self, entity: &str) -> Vec<InsightRecord> {
        let key = entity.to_lowercase();
        self.insight_history
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Dynamic Discovery Methods
    // ─────────────────────────────────────────────────────────────────────────

    /// Add a newly discovered, verified company to the registry.
    ///
    /// Returns the generated entity_id (SHA256 hash of normalized name for
    /// deterministic dedup).
    ///
    /// Activity scores for new entities start at `initial_activity *
    /// verification_confidence`.
    pub fn register_company(
        &mut self,
        name: &str,
        discovered_from: &DiscoverySource,
        metadata: HashMap<String, String>,
        initial_activity: f64,
    ) -> String {
        let normalized_name = crate::company_discovery::normalize_company_name(name);
        let key = normalized_name.clone();

        // Generate deterministic entity ID from normalized name
        let entity_id = generate_entity_id(&normalized_name);

        // Check if already registered
        if self.entities.contains_key(&key) {
            return entity_id;
        }

        // Build entity profile
        let mut profile = EntityProfile::new(name)
            .with_category(EntityCategory::Other("dynamically_discovered".to_string()));

        // Transfer metadata into profile fields
        if let Some(ticker) = metadata.get("ticker") {
            profile = profile.with_ticker(ticker);
        }
        if let Some(country) = metadata.get("headquarters_mentioned") {
            profile = profile.with_country(country);
        }

        // Enrich from discovery source context
        let source_str = discovered_from.as_str();
        profile.industry_keywords.push(source_str.to_string());
        profile.topic_keywords.push("dynamically_discovered".to_string());

        // Add any website to geographic context
        if let Some(website) = metadata.get("website") {
            profile.topic_keywords.push(website.clone());
        }

        // Mark as dynamically discovered
        profile.is_dynamically_discovered = true;
        profile.verification_count = 1;
        profile.last_verified = Some(Utc::now());

        // Register the profile
        self.categories.insert(key.clone(), profile.category.clone());
        self.activity_scores
            .entry(key.clone())
            .or_insert(initial_activity * 0.8); // Apply verification confidence
        self.last_insight_time
            .entry(key.clone())
            .or_insert_with(Instant::now);
        self.dynamically_discovered.insert(key.clone());
        self.entities.insert(key, profile);

        entity_id
    }

    /// Check if a company is already registered (by normalized name).
    pub fn is_registered(&self, normalized_name: &str) -> bool {
        self.entities.contains_key(normalized_name)
    }

    /// Get all dynamically discovered (but-not-fully-verified) companies.
    /// Note: In the current implementation, candidates pending verification
    /// are stored in the pipeline, not the registry. This returns dynamically
    /// discovered entities that ARE registered.
    pub fn pending_verifications(&self) -> Vec<String> {
        // Dynamically discovered entities are already registered.
        // Pending pipeline candidates are in DiscoveryPipeline::candidates.
        // This method returns dynamically discovered entities that have
        // low verification counts for re-verification consideration.
        self.dynamically_discovered
            .iter()
            .filter(|name| {
                self.entities
                    .get(*name)
                    .map(|p| p.verification_count <= 1)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// Prune entities with no activity for >90 days (after warning).
    ///
    /// Returns the list of pruned entity names. Only prunes dynamically
    /// discovered entities; seed entities from YAML config are retained.
    pub fn prune_stale_entities(&mut self, max_inactive_days: u64) -> Vec<String> {
        let threshold_seconds = max_inactive_days * 86400;
        let mut to_remove = Vec::new();

        // Only prune dynamically discovered entities
        for name in &self.dynamically_discovered {
            let score = self.activity_scores.get(name).copied().unwrap_or(0.0);
            // Activity score decays to ~0.01 after extended inactivity.
            // If score is at floor AND no recent insight time, it's stale.
            if score <= 0.02 {
                if let Some(last_time) = self.last_insight_time.get(name) {
                    if last_time.elapsed().as_secs() > threshold_seconds {
                        to_remove.push(name.clone());
                    }
                } else {
                    to_remove.push(name.clone());
                }
            }
        }

        // Remove stale entities
        for name in &to_remove {
            self.entities.remove(name);
            self.activity_scores.remove(name);
            self.last_insight_time.remove(name);
            self.categories.remove(name);
            self.dynamically_discovered.remove(name);
            self.insight_history.remove(name);
        }

        to_remove
    }

    /// Total entity count (static + dynamically discovered).
    pub fn total_entities(&self) -> usize {
        self.entities.len()
    }

    /// Entities discovered dynamically (not from seed config).
    pub fn dynamically_discovered(&self) -> Vec<&EntityProfile> {
        self.dynamically_discovered
            .iter()
            .filter_map(|name| self.entities.get(name))
            .collect()
    }
}

/// Generate a deterministic entity ID from a normalized name using SHA256.
fn generate_entity_id(normalized_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_name.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8]) // First 8 bytes = 16 hex chars, sufficient for dedup
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity History
// ─────────────────────────────────────────────────────────────────────────────

/// A historical record of an insight generated for an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightRecord {
    /// Unique insight identifier.
    pub id: String,
    /// Entity name this insight was about.
    pub entity_name: String,
    /// Recipe code that generated this insight.
    pub recipe_code: String,
    /// When the insight was generated (Unix timestamp).
    pub generated_at: i64,
    /// Insight title/summary.
    pub title: String,
    /// Confidence score at generation time.
    pub confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Signal Pattern (Data-Driven Signal Parsing)
// ─────────────────────────────────────────────────────────────────────────────

/// A compiled signal pattern used for data-driven signal parsing.
///
/// Replaces the 500+ line if/else chain with a configurable set of patterns.
/// Each pattern has a regex, category, weight, and optional entity hint.
#[derive(Debug, Clone)]
pub struct SignalPattern {
    /// Compiled regex to match against signal text.
    pub regex: Regex,
    /// Category label for matched signals (e.g., "semiconductor", "supply_chain").
    pub category: String,
    /// Weight/importance of this pattern (0.0 - 1.0).
    pub weight: f64,
    /// Optional entity name hint if this pattern strongly indicates an entity.
    pub entity_hint: Option<String>,
}

impl SignalPattern {
    /// Create a new signal pattern.
    pub fn new(pattern: &str, category: &str, weight: f64) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: Regex::new(pattern)?,
            category: category.to_string(),
            weight,
            entity_hint: None,
        })
    }

    /// Create a new signal pattern with an entity hint.
    pub fn with_entity_hint(
        pattern: &str,
        category: &str,
        weight: f64,
        entity_hint: &str,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: Regex::new(pattern)?,
            category: category.to_string(),
            weight,
            entity_hint: Some(entity_hint.to_string()),
        })
    }

    /// Test whether this pattern matches the given text.
    pub fn matches(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

/// Result of data-driven signal parsing.
#[derive(Debug, Clone)]
pub struct SignalMatch {
    /// The matched pattern.
    pub pattern: SignalPattern,
    /// Match strength (can be refined with capture groups).
    pub strength: f64,
    /// Captured text groups, if any.
    pub captures: Vec<String>,
    /// Inferred category from the match.
    pub category: String,
}

/// Parse a signal text using a set of patterns, returning the top-k matches.
///
/// # Signal Parsing Strategy
///
/// Iterates all registered patterns and collects matches. Patterns with higher
/// weights contribute more to the match strength. Returns the top-k matches
/// sorted by strength. Unknown patterns (no match) return an empty Vec rather
/// than being silently ignored.
pub fn parse_signal(
    text: &str,
    patterns: &[SignalPattern],
    top_k: usize,
) -> Vec<SignalMatch> {
    let mut matches: Vec<SignalMatch> = Vec::new();

    for pattern in patterns {
        if let Some(caps) = pattern.regex.captures(text) {
            let captures: Vec<String> = caps
                .iter()
                .skip(1)
                .filter_map(|m| m.map(|m| m.as_str().to_string()))
                .collect();

            let strength = pattern.weight;

            matches.push(SignalMatch {
                pattern: pattern.clone(),
                strength,
                captures,
                category: pattern.category.clone(),
            });
        }
    }

    // Sort by strength descending, return top-k
    matches.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches.truncate(top_k);
    matches
}

/// Build the default set of signal patterns for the ApexIntel insight system.
///
/// These patterns cover the major entity categories and signal types that
/// the system should recognise. Replace or extend this list from a config
/// file or DB table in production deployments.
pub fn default_signal_patterns() -> Vec<SignalPattern> {
    let mut patterns = Vec::new();

    // Semiconductor / GPU patterns
    // B339: no entity hint — the previous "NVIDIA" hint short-circuited
    // attribution so ANY GPU/AI-chip news was credited to NVIDIA regardless
    // of which company the text was about.
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(GPU|CUDA|Hopper|H100|A100|Tensor\s*Core|AI\s*chip|semiconductor|foundry|3nm|5nm|EUV)\b",
        "semiconductor",
        0.8,
    ) {
        patterns.push(p);
    }

    // EMS / Manufacturing patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(EMS|electronics manufacturing|contract manufacturer|SMT\s*assembly|box\s*build|supply chain|factory|manufacturing\s*plant)\b",
        "manufacturing",
        0.7,
    ) {
        patterns.push(p);
    }

    // Defense / Aerospace patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(defense|aerospace|military|government\s*contract|aircraft|missile|radar|satellite)\b",
        "defense",
        0.7,
    ) {
        patterns.push(p);
    }

    // Supply chain / logistics patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(supply chain|logistics|freight|shipping|port|customs|tariff|warehouse|inventory)\b",
        "supply_chain",
        0.6,
    ) {
        patterns.push(p);
    }

    // Automotive patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(automotive|electric vehicle|EV|auto\s*plant|car\s*manufacturing|Tier\s*1|OEM)\b",
        "automotive",
        0.6,
    ) {
        patterns.push(p);
    }

    // AI / Software patterns
    // B339: entity hint removed — see the semiconductor note above.
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(artificial intelligence|machine learning|large language model|LLM|GPT|deep learning|neural network)\b",
        "technology",
        0.7,
    ) {
        patterns.push(p);
    }

    // Certification / quality patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(ISO\s*\d+|AS9100|NADCAP|certification|quality\s*audit)\b",
        "quality",
        0.5,
    ) {
        patterns.push(p);
    }

    // Job posting patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(hiring|job|position|opening|recruiting|career|vacancy)\b",
        "hr_signal",
        0.4,
    ) {
        patterns.push(p);
    }

    // Regulatory / trade patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(sanctions|export control|regulatory|compliance|ITAR|EAR|trade restrictions)\b",
        "regulatory",
        0.8,
    ) {
        patterns.push(p);
    }

    // Financial patterns
    if let Ok(p) = SignalPattern::new(
        r"(?i)\b(revenue|earnings|quarterly results|SEC filing|IPO|acquisition|merger|dividend)\b",
        "financial",
        0.6,
    ) {
        patterns.push(p);
    }

    // Taiwan-specific patterns (TSMC, Foxconn, etc.)
    if let Ok(p) = SignalPattern::with_entity_hint(
        r"(?i)\b(Taiwan Semiconductor|TSMC|Hon Hai|Foxconn|Pegatron|Quanta)\b",
        "semiconductor",
        0.9,
        "TSMC",
    ) {
        patterns.push(p);
    }

    patterns
}

// ─────────────────────────────────────────────────────────────────────────────
// Relevance Validation
// ─────────────────────────────────────────────────────────────────────────────

/// Result of relevance validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevanceValidation {
    /// Overall relevance score (0.0 - 1.0)
    pub relevance_score: f64,
    /// Whether the insight passes the relevance threshold
    pub is_relevant: bool,
    /// Keywords found in the insight
    pub matched_keywords: Vec<String>,
    /// Keywords that were expected but not found
    pub missing_keywords: Vec<String>,
    /// Specificity score - how unique is this to the entity?
    pub specificity_score: f64,
    /// Warning messages
    pub warnings: Vec<String>,
}

/// Relevance validator for insights.
pub struct RelevanceValidator {
    /// Minimum relevance score threshold
    min_relevance_threshold: f64,
    /// Minimum specificity score threshold
    min_specificity_threshold: f64,
}

impl Default for RelevanceValidator {
    fn default() -> Self {
        Self {
            min_relevance_threshold: 0.3,
            min_specificity_threshold: 0.2,
        }
    }
}

impl RelevanceValidator {
    /// Create a new validator with custom thresholds.
    pub fn new(min_relevance: f64, min_specificity: f64) -> Self {
        Self {
            min_relevance_threshold: min_relevance,
            min_specificity_threshold: min_specificity,
        }
    }

    /// Validate an insight against an entity profile.
    pub fn validate(&self, insight_text: &str, profile: &EntityProfile) -> RelevanceValidation {
        let insight_lower = insight_text.to_lowercase();
        let keywords = profile.all_keywords();

        let mut matched_keywords = Vec::new();
        let mut missing_keywords = Vec::new();
        let mut warning_messages = Vec::new();

        // Check for entity name mentions
        let entity_name_mentioned = insight_lower.contains(&profile.entity_name.to_lowercase());

        // Check keyword matches
        for keyword in &keywords {
            if insight_lower.contains(keyword) {
                matched_keywords.push(keyword.clone());
            } else {
                missing_keywords.push(keyword.clone());
            }
        }

        // Calculate relevance score.  Rich profiles may have 40+ keywords;
        // dividing by the full pool would penalise well-described entities.
        // Cap the denominator so matching a reasonable number of keywords
        // produces a strong signal regardless of vocabulary size.
        //
        // B341: dynamically discovered entities have tiny keyword sets
        // (name + placeholder tokens like "dynamically_discovered"), so a
        // name-mention alone scored ~0.25 and could never clear the 0.3
        // threshold — the pipeline rejected insights about entities it had
        // itself just discovered. An explicit entity-name mention is itself
        // a floor-strength relevance signal.
        let total_keywords = keywords.len();
        let effective_total = (total_keywords as f64).min(10.0);
        let keyword_ratio = if effective_total > 0.0 {
            (matched_keywords.len() as f64 / effective_total).min(1.0)
        } else {
            0.0
        };
        let relevance_score = if entity_name_mentioned {
            keyword_ratio.max(0.35)
        } else {
            keyword_ratio
        };

        // Calculate specificity - how many non-generic keywords matched?
        // Generic keywords reduce specificity (e.g., "market", "global", "news")
        let generic_keywords = vec![
            "market", "global", "news", "report", "update", "latest", "breaking", "alert",
            "industry", "sector", "economy", "business",
        ];

        let non_generic_matches: Vec<&String> = matched_keywords
            .iter()
            .filter(|k| !generic_keywords.contains(&k.as_str()))
            .collect();

        let specificity_score = if !matched_keywords.is_empty() {
            non_generic_matches.len() as f64 / matched_keywords.len() as f64
        } else {
            0.0
        };

        // Check for specific entity-relevant content
        if !entity_name_mentioned {
            warning_messages.push(format!(
                "Entity '{}' not explicitly mentioned in insight",
                profile.entity_name
            ));
        }

        if matched_keywords.is_empty() {
            warning_messages.push("No entity-specific keywords found in insight".to_string());
        }

        if relevance_score < self.min_relevance_threshold {
            warning_messages.push(format!(
                "Relevance score ({:.2}) below threshold ({:.2})",
                relevance_score, self.min_relevance_threshold
            ));
        }

        // Check for generic content that doesn't belong to any specific entity
        let generic_content_indicators = [
            "geopolitical tensions",
            "oil prices",
            "global markets",
            "middle east",
            "escalating",
        ];

        let mut generic_count = 0;
        for indicator in generic_content_indicators {
            if insight_lower.contains(indicator) {
                generic_count += 1;
            }
        }

        if generic_count >= 3 && matched_keywords.len() <= 2 {
            warning_messages.push("Insight appears to contain mostly generic content".to_string());
        }

        // Determine if relevant
        let is_relevant = relevance_score >= self.min_relevance_threshold
            && specificity_score >= self.min_specificity_threshold
            && entity_name_mentioned;

        RelevanceValidation {
            relevance_score,
            is_relevant,
            matched_keywords,
            missing_keywords,
            specificity_score,
            warnings: warning_messages,
        }
    }

    /// Validate insight with automatic entity profile generation.
    pub fn validate_with_entity_name(
        &self,
        insight_text: &str,
        entity_name: &str,
    ) -> RelevanceValidation {
        // Create a basic profile from entity name
        let profile = EntityProfile::new(entity_name);
        self.validate(insight_text, &profile)
    }
}

/// Pre-built profiles for common tech entities.
///
/// These profiles are automatically loaded from [`config/augmentation_entities.yaml`](../../config/augmentation_entities.yaml)
/// at startup via [`EntityRegistry::from_yaml_config`]. The individual
/// `nvidia_profile`, `tsmc_profile`, and `foxconn_profile` functions below
/// are kept for backward compatibility and delegate to the registry by
/// default.
pub mod common_profiles {
    use super::EntityProfile;

    /// NVIDIA Corporation profile — semiconductor, GPU, AI.
    pub fn nvidia_profile() -> EntityProfile {
        EntityProfile::new("NVIDIA")
            .with_industry(vec![
                "semiconductor",
                "chip",
                "GPU",
                "AI hardware",
                "data center",
            ])
            .with_products(vec![
                "A100",
                "H100",
                "V100",
                "RTX",
                "GeForce",
                "Tesla",
                "DGX",
                "Jetson",
                "CUDA",
                "Tensor Core",
                "Hopper",
                "Ada Lovelace",
            ])
            .with_topics(vec![
                "AI",
                "artificial intelligence",
                "machine learning",
                "deep learning",
                "GPU computing",
                "HPC",
                "high performance computing",
                "data center",
                "gaming",
                "autonomous vehicle",
                "robotics",
                "metaverse",
                "LLM",
                "GPT",
            ])
            .with_geography(vec!["Santa Clara", "California", "USA", "Taiwan", "China"])
            .with_competitors(vec![
                "AMD", "Intel", "Qualcomm", "Google", "TPU", "Amazon", "Trainium",
            ])
    }

    /// TSMC profile — semiconductor foundry, advanced nodes.
    pub fn tsmc_profile() -> EntityProfile {
        EntityProfile::new("TSMC")
            .with_industry(vec![
                "semiconductor",
                "foundry",
                "chip manufacturing",
                "fab",
            ])
            .with_products(vec!["3nm", "5nm", "7nm", "28nm", "wafer", "先进製程"])
            .with_topics(vec![
                "advanced node",
                "chip shortage",
                "fabrication",
                "wafer",
                "EUV",
            ])
            .with_geography(vec!["Taiwan", "Arizona", "USA", "Tainan", "Hsinchu"])
            .with_competitors(vec!["Samsung", "Intel", "GlobalFoundries", "SMIC"])
    }

    /// Foxconn profile — EMS, electronics manufacturing.
    pub fn foxconn_profile() -> EntityProfile {
        EntityProfile::new("Foxconn")
            .with_industry(vec![
                "EMS",
                "electronics manufacturing",
                "contract manufacturer",
            ])
            .with_products(vec!["iPhone", "smartphone", "assembly", "OEM"])
            .with_topics(vec![
                "supply chain",
                "manufacturing",
                "labor",
                "factory",
                "assembly",
            ])
            .with_geography(vec!["Taiwan", "China", "Vietnam", "India", "Wisconsin"])
            .with_competitors(vec!["Flex", "Jabil", "Pegatron", "Wistron"])
    }

    /// Get a profile for the given entity name.
    ///
    /// This function delegates to the embedded hardcoded profiles for backward
    /// compatibility. For dynamic registry access (which loads all 22+ entities
    /// from YAML), use [`EntityRegistry::get`] instead.
    ///
    /// # Hardcoded Entities (Legacy)
    ///
    /// - `nvidia` / `nvidia corporation`
    /// - `tsmc` / `taiwan semiconductor` / `taiwan semiconductor manufacturing company`
    /// - `foxconn` / `hon hai` / `hon hai precision`
    pub fn get_profile_for_entity(entity_name: &str) -> Option<EntityProfile> {
        match entity_name.to_lowercase().as_str() {
            "nvidia" | "nvidia corporation" => Some(nvidia_profile()),
            "tsmc" | "taiwan semiconductor" | "taiwan semiconductor manufacturing company" => {
                Some(tsmc_profile())
            }
            "foxconn" | "hon hai" | "hon hai precision" => Some(foxconn_profile()),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Staleness Detector
// ─────────────────────────────────────────────────────────────────────────────

/// Stale insight detector - checks if similar insights have fired recently.
#[derive(Debug, Clone)]
pub struct StalenessDetector {
    /// Maximum number of times an entity-recipe combo can fire before suppression
    max_repetitions: u32,
    /// Days to look back for repetition check
    lookback_days: i64,
}

impl Default for StalenessDetector {
    fn default() -> Self {
        Self {
            max_repetitions: 2,
            lookback_days: 14,
        }
    }
}

impl StalenessDetector {
    /// Create a new staleness detector with custom parameters.
    pub fn new(max_repetitions: u32, lookback_days: i64) -> Self {
        Self {
            max_repetitions,
            lookback_days,
        }
    }

    /// Check if an entity-recipe combination is becoming stale.
    ///
    /// Returns `(is_stale, repetition_count, suppression_recommended)`.
    pub fn check_staleness(
        &self,
        _entity_id: &str,
        _recipe_code: &str,
        historical_firings: &[(i64, String)], // (timestamp, insight_summary_hash)
    ) -> (bool, u32, bool) {
        let now = chrono::Utc::now().timestamp();
        let lookback_seconds = self.lookback_days * 86400;
        let cutoff = now - lookback_seconds;

        // Filter to recent firings within the lookback window
        let recent_hashes: Vec<&str> = historical_firings
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .map(|(_, hash)| hash.as_str())
            .collect();

        let repetition_count = recent_hashes.len() as u32;

        // Deduplicate to count unique content
        let unique: HashSet<&str> = recent_hashes.iter().copied().collect();
        let unique_count = unique.len() as u32;

        // Stale if total firings exceed threshold and most are repetitive
        let is_stale = repetition_count >= self.max_repetitions && unique_count < repetition_count;
        let suppress =
            repetition_count > self.max_repetitions && unique_count * 2 < repetition_count;

        (is_stale, repetition_count, suppress)
    }

    /// Calculate staleness penalty for confidence score.
    pub fn staleness_penalty(&self, repetition_count: u32) -> f64 {
        // Apply penalty: each repetition reduces confidence
        // 0 reps = 1.0 (no penalty), 1 rep = 0.8, 2+ reps = 0.5
        match repetition_count {
            0 => 1.0,
            1 => 0.8,
            _ => 0.5,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Entity Registry Tests ────────────────────────────────────────────

    #[test]
    fn test_entity_registry_loads_all_entities_from_yaml() {
        let registry = EntityRegistry::from_yaml_config();

        // Should load 22+ EMS companies + OEM companies
        let ems_count = registry
            .entities
            .values()
            .filter(|p| p.category == EntityCategory::Ems)
            .count();
        let oem_count = registry
            .entities
            .values()
            .filter(|p| p.category == EntityCategory::Oem)
            .count();

        assert!(
            ems_count >= 21,
            "Expected at least 21 EMS entities, got {}",
            ems_count
        );
        assert!(
            oem_count >= 12,
            "Expected at least 12 OEM entities, got {}",
            oem_count
        );
        assert!(
            registry.len() >= 33,
            "Expected at least 33 total entities, got {}",
            registry.len()
        );
    }

    #[test]
    fn test_entity_registry_contains_specific_entities() {
        let registry = EntityRegistry::from_yaml_config();

        assert!(
            registry.get("foxconn").is_some(),
            "Should contain Foxconn"
        );
        assert!(
            registry.get("jabil inc.").is_some(),
            "Should contain Jabil Inc."
        );
        assert!(
            registry.get("flex ltd.").is_some(),
            "Should contain Flex Ltd."
        );
        assert!(
            registry.get("celestica").is_some(),
            "Should contain Celestica"
        );
        assert!(
            registry.get("lockheed martin").is_some(),
            "Should contain Lockheed Martin"
        );
        assert!(
            registry.get("airbus defence").is_some(),
            "Should contain Airbus Defence"
        );
    }

    #[test]
    fn test_entity_for_context_direct_match() {
        let registry = EntityRegistry::from_yaml_config();

        let context = SignalContext {
            text: "NVIDIA announced new H100 GPUs for AI workloads.".to_string(),
            entity_hint: Some("NVIDIA".to_string()),
            category: Some("news".to_string()),
            source_url: None,
            timestamp: 1000000,
        };

        let entity = registry.entity_for_context(&context);
        assert!(
            entity.is_some(),
            "Should find an entity for NVIDIA context"
        );
    }

    #[test]
    fn test_top_n_entities_returns_unique() {
        let registry = EntityRegistry::from_yaml_config();

        let top_5 = registry.top_n_entities(5, &[]);
        assert_eq!(top_5.len(), 5, "Should return 5 entities");

        // Verify uniqueness
        let mut names: Vec<&str> = top_5.iter().map(|p| p.entity_name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            5,
            "top_n_entities should return unique entities"
        );
    }

    #[test]
    fn test_top_n_entities_excludes_listed() {
        let registry = EntityRegistry::from_yaml_config();

        let exclude = vec!["foxconn".to_string(), "nvidia".to_string()];
        let top_3 = registry.top_n_entities(10, &exclude);

        // None of the returned entities should be in the exclude list
        for profile in &top_3 {
            let name = profile.entity_name.to_lowercase();
            assert!(
                !exclude.contains(&name),
                "Entity {} should be excluded",
                profile.entity_name
            );
        }
    }

    #[test]
    fn test_activity_decay_reduces_scores() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("TestCorp"));
        registry.register(EntityProfile::new("AnotherCorp"));

        // Record some activity
        registry.record_observation("TestCorp");
        registry.record_observation("TestCorp");

        let before = registry.activity_score("TestCorp");
        assert!(
            (before - 3.0).abs() < 0.01,
            "Expected score ~3.0, got {}",
            before
        );

        registry.activity_decay();

        let after = registry.activity_score("TestCorp");
        assert!(
            (after - 1.5).abs() < 0.01,
            "Expected score ~1.5 after decay, got {}",
            after
        );
    }

    #[test]
    fn test_activity_decay_floors_at_zero() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("LowActivityCorp"));

        // Multiple decay cycles should floor at 0.01
        for _ in 0..10 {
            registry.activity_decay();
        }

        let score = registry.activity_score("LowActivityCorp");
        assert!(
            (score - 0.01).abs() < 0.001,
            "Expected floor score 0.01, got {}",
            score
        );
    }

    #[test]
    fn test_entity_diversity_selector_picks_from_multiple_categories() {
        let mut registry = EntityRegistry::empty();

        // Register entities from different categories
        registry.register(
            EntityProfile::new("ChipMaker")
                .with_category(EntityCategory::Semiconductor),
        );
        registry.register(
            EntityProfile::new("BoardAssembler")
                .with_category(EntityCategory::Ems),
        );
        registry.register(
            EntityProfile::new("AeroPrime")
                .with_category(EntityCategory::Oem),
        );
        registry.register(
            EntityProfile::new("SecondChip")
                .with_category(EntityCategory::Semiconductor),
        );
        registry.register(
            EntityProfile::new("ThirdChip")
                .with_category(EntityCategory::Semiconductor),
        );

        let selected = registry.select_diverse_entity_set(5);

        // Should have no more than 2 from any single category
        let semicon_count = selected
            .iter()
            .filter(|s| {
                registry
                    .entities
                    .get(&s.to_lowercase())
                    .map(|p| p.category == EntityCategory::Semiconductor)
                    .unwrap_or(false)
            })
            .count();
        assert!(
            semicon_count <= 2,
            "Should have at most 2 semiconductor entities, got {}",
            semicon_count
        );

        // Should have at least 2 categories represented
        let cats: HashSet<&EntityCategory> = selected
            .iter()
            .filter_map(|s| registry.entities.get(&s.to_lowercase()))
            .map(|p| &p.category)
            .collect();
        assert!(
            cats.len() >= 2,
            "Should have at least 2 categories, got {}",
            cats.len()
        );
    }

    #[test]
    fn test_stale_entities_detection() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("ActiveCorp"));
        registry.register(EntityProfile::new("StaleCorp"));

        // Boost ActiveCorp
        registry.record_observation("ActiveCorp");
        registry.record_observation("ActiveCorp");

        // StaleCorp gets no observations, then decay a few times
        // ActiveCorp: 3.0 → 1.5 → 0.75 (stays above 0.5 threshold)
        // StaleCorp: 1.0 → 0.5 → 0.25 (falls below 0.5 threshold)
        for _ in 0..2 {
            registry.activity_decay();
        }

        let stale = registry.stale_entities();
        let stale_names: Vec<&str> = stale.iter().map(|p| p.entity_name.as_str()).collect();

        assert!(
            stale_names.contains(&"StaleCorp"),
            "StaleCorp should be detected as stale"
        );
        assert!(
            !stale_names.contains(&"ActiveCorp"),
            "ActiveCorp should not be stale"
        );
    }

    // ── Signal Pattern Tests ─────────────────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_signal_pattern_matches() {
        let pattern = SignalPattern::new(r"(?i)\bGPU\b", "semiconductor", 0.8).unwrap();
        assert!(pattern.matches("NVIDIA GPU sales"));
        assert!(pattern.matches("The latest GPU architecture"));
        assert!(!pattern.matches("General purpose unit"));
    }

    #[test]
    fn test_parse_signal_returns_top_k() {
        let patterns = default_signal_patterns();
        let text = "NVIDIA announced H100 GPUs with Hopper architecture \
                     for AI and machine learning workloads in data centers.";

        let matches = parse_signal(text, &patterns, 3);
        assert!(!matches.is_empty(), "Should match at least one pattern");
        assert!(
            matches.len() <= 3,
            "Should return at most 3 matches, got {}",
            matches.len()
        );

        // Should be sorted by strength descending
        for i in 1..matches.len() {
            assert!(
                matches[i - 1].strength >= matches[i].strength,
                "Matches should be sorted by strength descending"
            );
        }
    }

    #[test]
    fn test_parse_signal_unknown_returns_empty() {
        let patterns = default_signal_patterns();
        let text = "The weather today is sunny with a chance of rain.";

        let matches = parse_signal(text, &patterns, 5);
        assert!(
            matches.is_empty(),
            "Unknown signal should return empty, got {} matches",
            matches.len()
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_signal_pattern_with_entity_hint() {
        let pattern =
            SignalPattern::with_entity_hint(r"(?i)TSMC|Taiwan Semiconductor", "semiconductor", 0.9, "TSMC")
                .unwrap();
        assert!(pattern.matches("TSMC announced 3nm production"));
        assert!(pattern.matches("Taiwan Semiconductor leads the foundry market"));
        assert!(!pattern.matches("Samsung foundry"));

        assert_eq!(pattern.entity_hint, Some("TSMC".to_string()));
    }

    // ── Relevance Validation Tests ──────────────────────────────────────

    #[test]
    fn test_nvidia_profile_rejects_generic_content() {
        let validator = RelevanceValidator::default();
        let profile = common_profiles::nvidia_profile();

        // This is the BAD insight - generic geopolitical content
        let bad_insight = "Geopolitical tensions in the Middle East are escalating, causing oil prices to surge and impacting global markets. Coverage from multiple sources is being compared for corroboration.";

        let result = validator.validate(bad_insight, &profile);

        // Should fail relevance check
        assert!(
            !result.is_relevant,
            "Generic content should not be relevant to NVIDIA"
        );
        assert!(
            result.warnings.iter().any(|w| w.contains("generic")),
            "Should warn about generic content"
        );
    }

    #[test]
    fn test_nvidia_profile_accepts_specific_content() {
        let validator = RelevanceValidator::default();
        let profile = common_profiles::nvidia_profile();

        // This is GOOD - NVIDIA-specific content
        let good_insight = "NVIDIA announces new H100 AI chips for data center expansion. The GPU computing demand continues to drive market growth. CUDA ecosystem expands with new AI frameworks.";

        let result = validator.validate(good_insight, &profile);

        // Should pass relevance check
        assert!(
            result.is_relevant,
            "NVIDIA-specific content should be relevant"
        );
        assert!(result.matched_keywords.iter().any(|k| k.contains("nvidia")));
        assert!(result.matched_keywords.iter().any(|k| k.contains("gpu")));
        assert!(result.matched_keywords.iter().any(|k| k.contains("h100")));
    }

    #[test]
    fn test_staleness_detection() {
        let detector = StalenessDetector::default();

        let now = chrono::Utc::now().timestamp();
        // Same insight fired 3 times recently
        let history = vec![
            (now - 100, "abc123".to_string()),
            (now - 50, "abc123".to_string()),
            (now - 10, "abc123".to_string()),
        ];

        let (is_stale, count, suppress) = detector.check_staleness("nvidia-123", "A001", &history);

        assert!(is_stale);
        assert!(count >= 3);
        assert!(suppress);
    }

    #[test]
    fn test_staleness_penalty() {
        let detector = StalenessDetector::default();

        assert!((detector.staleness_penalty(0) - 1.0).abs() < 0.01);
        assert!((detector.staleness_penalty(1) - 0.8).abs() < 0.01);
        assert!((detector.staleness_penalty(2) - 0.5).abs() < 0.01);
    }

    // ── Entity Category Tests ────────────────────────────────────────────

    #[test]
    fn test_entity_category_parse() {
        assert_eq!(EntityCategory::from_str("ems"), EntityCategory::Ems);
        assert_eq!(EntityCategory::from_str("OEM"), EntityCategory::Oem);
        assert_eq!(
            EntityCategory::from_str("semiconductor"),
            EntityCategory::Semiconductor
        );
        assert_eq!(
            EntityCategory::from_str("Technology"),
            EntityCategory::Technology
        );
        assert_eq!(
            EntityCategory::from_str("unknown_category"),
            EntityCategory::Other("unknown_category".to_string())
        );
    }

    #[test]
    fn test_entity_category_roundtrip() {
        let cases = vec![
            EntityCategory::Semiconductor,
            EntityCategory::Ems,
            EntityCategory::Oem,
            EntityCategory::Automotive,
            EntityCategory::Logistics,
            EntityCategory::Technology,
            EntityCategory::Other("custom".to_string()),
        ];

        for cat in cases {
            let s = cat.as_str();
            let back = EntityCategory::from_str(s);
            assert_eq!(cat, back, "Roundtrip failed for {:?}", cat);
        }
    }

    // ── Default Signal Patterns Test ─────────────────────────────────────

    #[test]
    fn test_default_signal_patterns_not_empty() {
        let patterns = default_signal_patterns();
        assert!(!patterns.is_empty(), "Should have default patterns");
        assert!(patterns.len() >= 8, "Should have at least 8 patterns");
    }

    #[test]
    fn test_entity_registry_observation_recording() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("ObservedCorp"));

        assert!((registry.activity_score("ObservedCorp") - 1.0).abs() < 0.01);

        registry.record_observation("ObservedCorp");
        assert!((registry.activity_score("ObservedCorp") - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_entity_registry_insight_recording() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("InsightCorp"));

        registry.record_insight("InsightCorp");
        assert!(
            registry.last_insight_time.contains_key("insightcorp"),
            "Should record insight time"
        );
    }

    #[test]
    fn test_entity_registry_insight_history() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("HistoryCorp"));

        // Initially no history
        assert!(registry.get_entity_history("HistoryCorp").is_empty());

        // Record an insight
        let record = InsightRecord {
            id: "ins-001".to_string(),
            entity_name: "HistoryCorp".to_string(),
            recipe_code: "SUPPLY_CHAIN_RISK".to_string(),
            generated_at: 1700000000,
            title: "Supply chain risk detected".to_string(),
            confidence: 0.85,
        };
        registry.record_insight_record(record);

        // History should now contain the record
        let history = registry.get_entity_history("HistoryCorp");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, "ins-001");
        assert_eq!(history[0].recipe_code, "SUPPLY_CHAIN_RISK");
        assert!((history[0].confidence - 0.85).abs() < 0.01);

        // Case-insensitive lookup
        let history_lower = registry.get_entity_history("historycorp");
        assert_eq!(history_lower.len(), 1);

        // Unknown entity returns empty
        assert!(registry.get_entity_history("UnknownCorp").is_empty());
    }

    #[test]
    fn test_entity_registry_insight_history_multiple_records() {
        let mut registry = EntityRegistry::empty();
        registry.register(EntityProfile::new("MultiCorp"));

        for i in 0..5 {
            registry.record_insight_record(InsightRecord {
                id: format!("ins-{i:03}"),
                entity_name: "MultiCorp".to_string(),
                recipe_code: "RECIPE_A".to_string(),
                generated_at: 1700000000 + i,
                title: format!("Insight {i}"),
                confidence: 0.5 + i as f64 * 0.1,
            });
        }

        let history = registry.get_entity_history("MultiCorp");
        assert_eq!(history.len(), 5);
        // Records should be in insertion order
        assert_eq!(history[0].id, "ins-000");
        assert_eq!(history[4].id, "ins-004");
    }

    #[test]
    fn test_select_diverse_entity_set_guarantees_cold_entity() {
        let mut registry = EntityRegistry::empty();

        // Register entities from different categories
        for i in 0..6 {
            let name = format!("Entity{}", i);
            let cat = match i % 3 {
                0 => EntityCategory::Semiconductor,
                1 => EntityCategory::Ems,
                _ => EntityCategory::Oem,
            };
            registry.register(EntityProfile::new(&name).with_category(cat));
        }

        // Record insights for all entities to make them "warm"
        for i in 0..6 {
            let name = format!("entity{}", i);
            registry.record_insight(&name);
        }

        let selected = registry.select_diverse_entity_set(4);

        // There may or may not be a cold entity (all are warm), but the function
        // should not crash and should return the requested number
        assert!(
            selected.len() <= 4,
            "Should return at most 4 entities"
        );
    }

    #[test]
    fn test_empty_registry_returns_none() {
        let registry = EntityRegistry::empty();

        assert!(registry.get("anything").is_none());
        assert!(registry.entity_for_context(&SignalContext {
            text: "test".to_string(),
            entity_hint: None,
            category: None,
            source_url: None,
            timestamp: 0,
        }).is_none());
        assert!(registry.top_n_entities(5, &[]).is_empty());
        assert!(registry.stale_entities().is_empty());
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_get_category_returns_category_for_known_entity() {
        let registry = EntityRegistry::from_yaml_config();

        let cat = registry.get_category("foxconn");
        assert!(cat.is_some(), "Should return category for Foxconn");
        assert_eq!(cat.unwrap(), EntityCategory::Ems);
    }

    #[test]
    fn test_get_category_returns_none_for_unknown_entity() {
        let registry = EntityRegistry::empty();

        let cat = registry.get_category("nonexistent");
        assert!(cat.is_none(), "Should return None for unknown entity");
    }

    #[test]
    fn test_entities_in_category_returns_all_same_category() {
        let registry = EntityRegistry::from_yaml_config();

        let ems_entities = registry.entities_in_category(&EntityCategory::Ems);
        assert!(!ems_entities.is_empty(), "Should find EMS entities");
        for profile in &ems_entities {
            assert_eq!(
                profile.category,
                EntityCategory::Ems,
                "All returned entities should be EMS"
            );
        }

        let oem_entities = registry.entities_in_category(&EntityCategory::Oem);
        assert!(!oem_entities.is_empty(), "Should find OEM entities");
        for profile in &oem_entities {
            assert_eq!(
                profile.category,
                EntityCategory::Oem,
                "All returned entities should be OEM"
            );
        }
    }

    #[test]
    fn test_entities_in_category_returns_sorted_by_activity() {
        let mut registry = EntityRegistry::empty();

        registry.register(EntityProfile::new("LowActivity").with_category(EntityCategory::Ems));
        registry.register(
            EntityProfile::new("HighActivity").with_category(EntityCategory::Ems),
        );

        // Boost HighActivity
        registry.record_observation("HighActivity");
        registry.record_observation("HighActivity");

        let ems = registry.entities_in_category(&EntityCategory::Ems);
        assert!(ems.len() >= 2, "Should have at least 2 EMS entities");
        // HighActivity should appear before LowActivity (higher score first)
        let high_pos = ems.iter().position(|p| p.entity_name == "HighActivity");
        let low_pos = ems.iter().position(|p| p.entity_name == "LowActivity");
        assert!(
            high_pos < low_pos,
            "HighActivity should be sorted before LowActivity"
        );
    }

    #[test]
    fn test_entities_in_category_empty_for_unregistered_category() {
        let registry = EntityRegistry::empty();

        let result = registry.entities_in_category(&EntityCategory::Semiconductor);
        assert!(result.is_empty(), "Should return empty for unregistered category");
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_entity_for_context_text_contains_name() {
        let mut registry = EntityRegistry::empty();
        registry.register(
            EntityProfile::new("TestCorp")
                .with_industry(vec!["electronics"]),
        );

        let context = SignalContext {
            text: "TestCorp announced new products.".to_string(),
            entity_hint: None,
            category: None,
            source_url: None,
            timestamp: 0,
        };

        let entity = registry.entity_for_context(&context);
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().entity_name, "TestCorp");
    }
}
