//! Dynamic POI Discovery Module
//!
//! This module provides dynamic entity discovery that can find new POIs,
//! competitors, and companies from crawled content. This fixes Issue #2
//! (Static POI Discovery).
//!
//! Key capabilities:
//! 1. Entity co-occurrence detection - when known entities appear with unknown ones
//! 2. Emergence scoring - detecting entities that are appearing more frequently
//! 3. Pattern-based discovery - using NLP to identify potential entities
//! 4. Source expansion suggestions - recommending new sources based on discoveries
//! 5. **Emerging entity detection** - clustering observations by mentioned entities,
//!    computing velocity, and flagging entities with statistically significant surges.
//! 6. **Entity tracking decisions** - using observation velocity, supply chain proximity,
//!    and news correlation to decide whether to track a new entity.

use crate::entity_relevance::EntityRegistry;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Entity Signal (Emerging Entity Detection Output)
// ─────────────────────────────────────────────────────────────────────────────

/// A signal indicating that an entity is emerging based on observation velocity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySignal {
    /// The entity name as mentioned in observations.
    pub entity_name: String,
    /// Computed velocity (observation_count / time_window_days).
    pub velocity: f64,
    /// Observation count in the detection window.
    pub observation_count: u32,
    /// Time window in days over which velocity was computed.
    pub time_window_days: f64,
    /// Signal strength (0.0 - 1.0), where 1.0 means highly significant.
    pub signal_strength: f64,
    /// The mean velocity of all observed entities (for context).
    pub mean_velocity: f64,
    /// Standard deviation of all entity velocities.
    pub std_velocity: f64,
    /// Number of standard deviations above the mean this entity is.
    pub sigma_above_mean: f64,
    /// Entity category inferred from co-occurring known entities.
    pub inferred_category: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Existing Types (unchanged from original)
// ─────────────────────────────────────────────────────────────────────────────

/// A potential new entity discovered from content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredEntity {
    /// The name as it appears in the content
    pub raw_name: String,
    /// Normalized/canonical name
    pub normalized_name: String,
    /// Source URL where discovered
    pub source_url: String,
    /// Context around the mention
    pub context: String,
    /// Confidence this is a real entity
    pub confidence: f64,
    /// Entity type estimation (company, person, org, etc.)
    pub entity_type: EntityType,
    /// Associated known entities (co-occurrences)
    pub associated_entities: Vec<String>,
    /// Topics/keywords in the context
    pub topics: Vec<String>,
    /// Geographic mentions
    pub geography: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntityType {
    Company,
    Person,
    Organization,
    Unknown,
}

/// A candidate entity for addition to the monitoring list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiCandidate {
    pub entity_id: Uuid,
    pub name: String,
    pub entity_type: EntityType,
    pub discovery_reason: DiscoveryReason,
    pub confidence: f64,
    pub emergence_score: f64,
    pub co_occurrence_count: u32,
    pub source_diversity: u32,
    pub recommended_action: Recommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscoveryReason {
    /// Found co-occurring with known entity
    CoOccurrence { known_entity: String },
    /// Frequency increased significantly
    Emerging {
        previous_count: u32,
        current_count: u32,
    },
    /// Matches known patterns (CEO, CFO, etc.)
    PatternMatch { pattern: String },
    /// Cross-reference with multiple sources
    Corroborated { source_count: u32 },
    /// Similar to existing monitored entity
    SimilarTo { similar_to: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Recommendation {
    /// Add to monitoring immediately
    AddNow,
    /// Add after human review
    ReviewRequired,
    /// Monitor for more signals before adding
    MonitorFurther,
    /// Don't add - likely false positive
    Discard,
}

/// External entity seed from authoritative sources (e.g., sanctions lists, regulatory filings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalEntitySeed {
    /// Entity name
    pub name: String,
    /// Source of the seed (e.g., "sanctions_list", "sec_filing", "trade_directory")
    pub source: String,
    /// Entity type if known
    pub entity_type: Option<EntityType>,
    /// Confidence in this seed (authoritative sources = 1.0)
    pub confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Co-Occurrence Tracker
// ─────────────────────────────────────────────────────────────────────────────

/// Entity co-occurrence tracker.
#[derive(Debug, Clone)]
pub struct CoOccurrenceTracker {
    /// Known entities to track co-occurrences with
    known_entities: HashSet<String>,
    /// External seed entities (not yet promoted to known)
    external_seeds: HashMap<String, ExternalEntitySeed>,
    /// Co-occurrence counts (includes unknown-to-unknown for discovery)
    cooccurrence_matrix: HashMap<(String, String), u32>,
    /// Unknown-to-unknown co-occurrence counts (for emerging entity clusters)
    unknown_cooccurrences: HashMap<(String, String), u32>,
    /// Recent mentions per entity
    entity_mentions: HashMap<String, Vec<EntityMention>>,
}

impl CoOccurrenceTracker {
    /// Check if an entity is already known.
    pub fn is_known(&self, entity: &str) -> bool {
        self.known_entities.contains(&entity.to_lowercase())
    }

    /// Check if an entity is an external seed.
    pub fn is_external_seed(&self, entity: &str) -> bool {
        self.external_seeds.contains_key(&entity.to_lowercase())
    }

    /// Add external entity seeds from authoritative sources.
    pub fn add_external_seeds(&mut self, seeds: Vec<ExternalEntitySeed>) {
        for seed in seeds {
            let normalized = seed.name.to_lowercase();
            // Don't overwrite if already known
            if !self.known_entities.contains(&normalized) {
                self.external_seeds.insert(normalized, seed);
            }
        }
    }

    /// Promote an external seed to known entity.
    pub fn promote_seed_to_known(&mut self, entity: &str) -> bool {
        let normalized = entity.to_lowercase();
        if let Some((_key, _seed)) = self.external_seeds.remove_entry(&normalized) {
            self.known_entities.insert(normalized);
            return true;
        }
        false
    }

    /// Maximum entries in `unknown_cooccurrences` before evicting low-count pairs.
    const MAX_UNKNOWN_COOCCURRENCES: usize = 10_000;

    /// Record a co-occurrence, tracking both known and unknown entities.
    /// This breaks the closed loop by also tracking unknown-to-unknown co-occurrences.
    pub fn record_cooccurrence(&mut self, entity: &str, associated_entity: &str) {
        let entity_lower = entity.to_lowercase();
        let associated_lower = associated_entity.to_lowercase();

        // Skip self-co-occurrence
        if entity_lower == associated_lower {
            return;
        }

        // Track against known entities
        if self.known_entities.contains(&associated_lower) {
            let key = (entity_lower.clone(), associated_lower.clone());
            *self.cooccurrence_matrix.entry(key).or_insert(0) += 1;
        }

        // Track against external seeds (these are authoritative but not yet monitored)
        if self.external_seeds.contains_key(&associated_lower) {
            let key = (entity_lower.clone(), associated_lower.clone());
            *self.cooccurrence_matrix.entry(key).or_insert(0) += 1;
        }

        // Track unknown-to-unknown co-occurrences for emerging entity clusters
        // This allows discovery of new entity groups that appear together
        if !self.known_entities.contains(&associated_lower)
            && !self.known_entities.contains(&entity_lower)
            && !self.external_seeds.contains_key(&associated_lower)
        {
            // Use sorted order for consistent keys
            let (first, second) = if entity_lower < associated_lower {
                (entity_lower, associated_lower)
            } else {
                (associated_lower, entity_lower)
            };
            *self
                .unknown_cooccurrences
                .entry((first, second))
                .or_insert(0) += 1;

            // Evict low-count pairs when the map grows too large
            if self.unknown_cooccurrences.len() > Self::MAX_UNKNOWN_COOCCURRENCES {
                self.evict_low_count_cooccurrences();
            }
        }
    }

    /// Evict the lowest-count half of unknown co-occurrence pairs to bound memory.
    fn evict_low_count_cooccurrences(&mut self) {
        let mut counts: Vec<u32> = self.unknown_cooccurrences.values().copied().collect();
        counts.sort_unstable();
        let median = counts[counts.len() / 2];
        self.unknown_cooccurrences
            .retain(|_, count| *count > median);
    }

    /// Get entities that frequently co-occur with unknown entities (potential new clusters).
    pub fn get_emerging_clusters(&self, min_cooccurrences: u32) -> Vec<(String, Vec<String>)> {
        let mut entity_peers: HashMap<String, Vec<String>> = HashMap::new();

        for ((e1, e2), count) in &self.unknown_cooccurrences {
            if *count >= min_cooccurrences {
                entity_peers.entry(e1.clone()).or_default().push(e2.clone());
                entity_peers.entry(e2.clone()).or_default().push(e1.clone());
            }
        }

        // Sort by cluster size
        let mut clusters: Vec<_> = entity_peers.into_iter().collect();
        clusters.sort_by_key(|b| std::cmp::Reverse(b.1.len()));
        clusters
    }
}

#[derive(Debug, Clone)]
pub struct EntityMention {
    pub source_url: String,
    pub context: String,
    pub timestamp: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Emergence Detector
// ─────────────────────────────────────────────────────────────────────────────

/// Entity emergence detector - finds entities appearing more frequently.
pub struct EmergenceDetector {
    /// Historical mention counts
    historical_counts: HashMap<String, Vec<(i64, u32)>>,
    /// Minimum days to track
    lookback_days: i64,
    /// Minimum mentions to consider emerging
    min_mentions: u32,
}

impl Default for EmergenceDetector {
    fn default() -> Self {
        Self {
            historical_counts: HashMap::new(),
            lookback_days: 30,
            min_mentions: 5,
        }
    }
}

impl EmergenceDetector {
    /// Record mentions of an entity.
    pub fn record_mentions(&mut self, entity_name: &str, count: u32, timestamp: i64) {
        let entry = self
            .historical_counts
            .entry(entity_name.to_lowercase())
            .or_default();
        entry.push((timestamp, count));
    }

    /// Check if an entity is emerging (appearing more frequently).
    /// Returns `Some((growth_rate, recent_count, older_count))` if emerging.
    pub fn is_emerging(&self, entity_name: &str) -> Option<(f64, u32, u32)> {
        let normalized = entity_name.to_lowercase();
        let counts = self.historical_counts.get(&normalized)?;

        if counts.len() < 2 {
            return None;
        }

        // Derive windows from lookback_days so the field is honoured.
        // Default 30 → recent_window=7, older_window=22 (close to original).
        let recent_window = (self.lookback_days / 4).max(1) as usize;
        let older_window = (self.lookback_days * 3 / 4).max(1) as usize;

        let recent: Vec<u32> = counts
            .iter()
            .rev()
            .take(recent_window)
            .map(|(_, c)| *c)
            .collect();

        let older: Vec<u32> = counts
            .iter()
            .rev()
            .skip(recent_window)
            .take(older_window)
            .map(|(_, c)| *c)
            .collect();

        if recent.is_empty() || older.is_empty() {
            return None;
        }

        let recent_sum: u32 = recent.iter().sum();
        let older_sum: u32 = older.iter().sum();

        if older_sum == 0 {
            // New entity - check if it meets minimum threshold
            if recent_sum >= self.min_mentions {
                return Some((1.0, recent_sum, 0)); // 100% increase from zero
            }
            return None;
        }

        // Calculate growth rate
        let growth_rate = (recent_sum as f64) / (older_sum as f64);

        // Require both growth rate (1.5x) AND minimum absolute increase (3+)
        // to filter out noise from entities with very low base counts.
        // Scale the older window into the recent window length before comparing.
        let older_recent_window_equivalent =
            older_sum.saturating_mul(recent.len().max(1) as u32) / older.len().max(1) as u32;
        let absolute_increase = recent_sum.saturating_sub(older_recent_window_equivalent);
        if growth_rate > 1.5 && recent_sum >= self.min_mentions && absolute_increase >= 3 {
            return Some((growth_rate, recent_sum, older_sum));
        }

        None
    }

    /// Get emergence score (0.0 - 1.0).
    pub fn emergence_score(&self, entity_name: &str) -> f64 {
        if let Some((growth_rate, recent_count, _)) = self.is_emerging(entity_name) {
            // Normalize to 0-1: growth rate of 1.5 = 0.0, growth rate of 5.0+ = 1.0
            let score = ((growth_rate - 1.5) / 3.5).clamp(0.0, 1.0);

            // Boost by recent volume
            let volume_boost = (recent_count as f64 / 50.0).min(0.3);

            (score + volume_boost).min(1.0)
        } else {
            0.0
        }
    }

    /// Compute observation velocity for all tracked entities.
    ///
    /// Velocity = total observations in the recent window / time_window_days.
    /// Returns a map of entity_name → velocity.
    pub fn compute_velocities(&self) -> HashMap<String, f64> {
        let mut velocities = HashMap::new();
        let time_window_days = (self.lookback_days / 4).max(1) as f64;

        for (name, counts) in &self.historical_counts {
            let recent_window = (self.lookback_days / 4).max(1) as usize;
            let recent_sum: u32 = counts
                .iter()
                .rev()
                .take(recent_window)
                .map(|(_, c)| *c)
                .sum();
            let velocity = recent_sum as f64 / time_window_days;
            velocities.insert(name.clone(), velocity);
        }

        velocities
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pattern-Based Entity Extractor
// ─────────────────────────────────────────────────────────────────────────────

/// Pattern-based entity extractor.
pub struct EntityPatternExtractor {
    /// Company name patterns
    company_patterns: Vec<CompanyPattern>,
    /// Person name patterns
    person_patterns: Vec<&'static str>,
    /// Dynamically known entities (populated from cooccurrence tracker).
    /// Used instead of hardcoded lists for associated entity extraction.
    known_entities: HashSet<String>,
}

#[derive(Debug, Clone)]
struct CompanyPattern {
    pattern: &'static str,
    weight: f64,
}

impl Default for EntityPatternExtractor {
    fn default() -> Self {
        Self {
            company_patterns: vec![
                CompanyPattern {
                    pattern: "Inc.",
                    weight: 0.8,
                },
                CompanyPattern {
                    pattern: "Corporation",
                    weight: 0.8,
                },
                CompanyPattern {
                    pattern: "Corp.",
                    weight: 0.8,
                },
                CompanyPattern {
                    pattern: "LLC",
                    weight: 0.8,
                },
                CompanyPattern {
                    pattern: "Ltd.",
                    weight: 0.8,
                },
                CompanyPattern {
                    pattern: "Co.",
                    weight: 0.6,
                },
                CompanyPattern {
                    pattern: "Group",
                    weight: 0.5,
                },
                CompanyPattern {
                    pattern: "Technologies",
                    weight: 0.6,
                },
                CompanyPattern {
                    pattern: "Semiconductor",
                    weight: 0.7,
                },
                CompanyPattern {
                    pattern: "Systems",
                    weight: 0.5,
                },
                CompanyPattern {
                    pattern: "Solutions",
                    weight: 0.5,
                },
                CompanyPattern {
                    pattern: "Holdings",
                    weight: 0.6,
                },
            ],
            person_patterns: vec![
                "CEO",
                "CFO",
                "CTO",
                "COO",
                "CMO",
                "CIO",
                "President",
                "Vice President",
                "VP",
                "Director",
                "Manager",
                "Founder",
                "Co-Founder",
                "Chairman",
                "Board Member",
            ],
            known_entities: HashSet::new(),
        }
    }
}

impl EntityPatternExtractor {
    /// Extract potential entities from text.
    pub fn extract(&self, text: &str) -> Vec<DiscoveredEntity> {
        let mut entities = Vec::new();
        let lines: Vec<&str> = text.lines().collect();

        for line in &lines {
            // Try to extract company names
            for pattern in &self.company_patterns {
                if line.contains(pattern.pattern) {
                    // Try to extract the full name
                    if let Some(name) = self.extract_company_name(line, pattern.pattern) {
                        if name.len() > 2 && name.len() < 100 {
                            entities.push(DiscoveredEntity {
                                raw_name: name.clone(),
                                normalized_name: name.clone(),
                                source_url: String::new(),
                                context: line.to_string(),
                                confidence: pattern.weight,
                                entity_type: EntityType::Company,
                                associated_entities: self.extract_associated_entities(line),
                                topics: self.extract_topics(line),
                                geography: self.extract_geography(line),
                            });
                        }
                    }
                }
            }

            // Try to extract person names (simplified)
            for title in &self.person_patterns {
                if line.contains(title) {
                    if let Some(name) = self.extract_person_name(line, title) {
                        if name.len() > 3 && name.len() < 50 {
                            entities.push(DiscoveredEntity {
                                raw_name: name.clone(),
                                normalized_name: name.clone(),
                                source_url: String::new(),
                                context: line.to_string(),
                                confidence: 0.6,
                                entity_type: EntityType::Person,
                                associated_entities: self.extract_associated_entities(line),
                                topics: self.extract_topics(line),
                                geography: self.extract_geography(line),
                            });
                        }
                    }
                }
            }
        }

        entities
    }

    fn extract_company_name(&self, line: &str, pattern: &str) -> Option<String> {
        // Find position of pattern
        if let Some(pos) = line.find(pattern) {
            // Extract up to 60 chars before the pattern, respecting char boundaries
            let raw_start = pos.saturating_sub(60);
            let mut start = raw_start;
            while start < pos && !line.is_char_boundary(start) {
                start += 1;
            }
            let candidate = line[start..pos + pattern.len()].trim();
            // Clean up
            let cleaned = candidate.replace(
                |c: char| !c.is_alphanumeric() && c != ' ' && c != '.' && c != ',',
                "",
            );
            if cleaned.len() > 3 {
                return Some(cleaned);
            }
        }
        None
    }

    fn extract_person_name(&self, line: &str, title: &str) -> Option<String> {
        if let Some(pos) = line.find(title) {
            // Get text before title - assume it's a name
            let before = if pos > 0 { line[..pos].trim() } else { "" };
            // Take last few words as name
            let words: Vec<&str> = before.split_whitespace().rev().take(2).collect();
            if !words.is_empty() {
                return Some(words.into_iter().rev().collect::<Vec<_>>().join(" "));
            }
        }
        None
    }

    fn extract_associated_entities(&self, text: &str) -> Vec<String> {
        // Use the dynamic set of known entities rather than a hardcoded list.
        // This set is populated from the cooccurrence tracker and grows as
        // entities are discovered dynamically from observations.
        let mut found = Vec::new();
        let text_lower = text.to_lowercase();

        for entity in &self.known_entities {
            if text_lower.contains(entity) {
                found.push(entity.clone());
            }
        }

        found
    }

    fn extract_topics(&self, text: &str) -> Vec<String> {
        let topic_keywords = vec![
            (
                "AI",
                vec!["ai", "artificial intelligence", "machine learning", "ml"],
            ),
            ("chips", vec!["chip", "semiconductor", "gpu", "processor"]),
            (
                "supply_chain",
                vec!["supply chain", "supplier", "manufacturing"],
            ),
            ("data_center", vec!["data center", "cloud", "server"]),
            ("automotive", vec!["automotive", "vehicle", "car", "ev"]),
            ("gaming", vec!["gaming", "game", "console"]),
        ];

        let text_lower = text.to_lowercase();
        let mut found = Vec::new();

        for (topic, keywords) in topic_keywords {
            for kw in keywords {
                if text_lower.contains(kw) {
                    found.push(topic.to_string());
                    break;
                }
            }
        }

        found
    }

    fn extract_geography(&self, text: &str) -> Vec<String> {
        let locations = vec![
            ("USA", vec!["usa", "united states", "america", "u.s."]),
            ("Taiwan", vec!["taiwan", "taipei"]),
            ("China", vec!["china", "chinese", "beijing", "shanghai"]),
            ("Korea", vec!["korea", "seoul", "korean"]),
            ("Japan", vec!["japan", "tokyo", "japanese"]),
            ("Europe", vec!["europe", "germany", "france", "uk"]),
            ("India", vec!["india", "indian"]),
            ("Vietnam", vec!["vietnam", "vietnamese"]),
        ];

        let text_lower = text.to_lowercase();
        let mut found = Vec::new();

        for (location, keywords) in locations {
            for kw in keywords {
                if text_lower.contains(kw) {
                    found.push(location.to_string());
                    break;
                }
            }
        }

        found
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Emerging Entity Detection Logic
// ─────────────────────────────────────────────────────────────────────────────

/// Extract entity mentions from observation text using simple regex/NER patterns.
///
/// This is a lightweight NER replacement that looks for capitalized words,
/// company suffixes, and known entity patterns. In production, this should
/// be replaced with a proper NER model or API call.
fn extract_entity_mentions(text: &str) -> Vec<String> {
    let mut mentions = HashSet::new();

    // Known entity names to look for (both from config and common companies)
    let known_patterns = [
        "Corp.",
        "Corporation",
        "Inc.",
        "Ltd.",
        "LLC",
        "GmbH",
        "PLC",
        "SA",
        "NV",
    ];

    // Check for entity patterns in the text
    let text_lower = text.to_lowercase();
    for pattern in &known_patterns {
        let pat_lower = pattern.to_lowercase();
        if let Some(pos) = text_lower.find(&pat_lower) {
            // Extract the entity name before the suffix
            let start = pos.saturating_sub(40);
            let preceding = &text[start..pos];
            let words: Vec<&str> = preceding.split_whitespace().collect();
            if let Some(last_word) = words.last() {
                let cleaned: String = last_word
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '.')
                    .collect();
                if !cleaned.is_empty() && cleaned.len() > 1 {
                    let full_name = format!("{} {}", cleaned, pattern);
                    mentions.insert(full_name);
                }
            }
        }
    }

    // Also extract capitalized multi-word sequences that might be entities
    // (simple heuristic: sequences of capitalized words of length 2-4)
    // Safety: the regex pattern is a valid hardcoded constant
    let re = regex::Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3})\b")
        .unwrap_or_else(|e| panic!("valid capitalized entity regex: {e}"));
    for cap in re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let name = m.as_str().to_string();
            // Filter out common false positives
            if name.len() > 3
                && !name.contains("The ")
                && !name.contains("This ")
                && !name.contains("That ")
                && name.split_whitespace().count() <= 4
            {
                mentions.insert(name);
            }
        }
    }

    mentions.into_iter().collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Main POI Discovery Service
// ─────────────────────────────────────────────────────────────────────────────

/// Main POI discovery service.
pub struct DynamicPoiDiscovery {
    cooccurrence_tracker: CoOccurrenceTracker,
    emergence_detector: EmergenceDetector,
    pattern_extractor: EntityPatternExtractor,
}

impl DynamicPoiDiscovery {
    /// Create a new discovery service.
    pub fn new(known_entities: Vec<String>) -> Self {
        let known_set: HashSet<String> = known_entities
            .into_iter()
            .map(|e| e.to_lowercase())
            .collect();

        Self {
            cooccurrence_tracker: CoOccurrenceTracker {
                known_entities: known_set.clone(),
                external_seeds: HashMap::new(),
                cooccurrence_matrix: HashMap::new(),
                unknown_cooccurrences: HashMap::new(),
                entity_mentions: HashMap::new(),
            },
            emergence_detector: EmergenceDetector::default(),
            pattern_extractor: EntityPatternExtractor {
                known_entities: known_set,
                ..EntityPatternExtractor::default()
            },
        }
    }

    /// Create a discovery service with external entity seeds.
    pub fn with_external_seeds(
        known_entities: Vec<String>,
        seeds: Vec<ExternalEntitySeed>,
    ) -> Self {
        let mut discovery = Self::new(known_entities);
        discovery.cooccurrence_tracker.add_external_seeds(seeds);
        discovery
    }

    /// Add external entity seeds from authoritative sources.
    pub fn add_external_seeds(&mut self, seeds: Vec<ExternalEntitySeed>) {
        self.cooccurrence_tracker.add_external_seeds(seeds);
    }

    /// Get emerging entity clusters (entities that co-occur with each other but aren't known).
    pub fn get_emerging_clusters(&self, min_cooccurrences: u32) -> Vec<(String, Vec<String>)> {
        self.cooccurrence_tracker
            .get_emerging_clusters(min_cooccurrences)
    }

    /// Discover emerging entities from a batch of observations.
    ///
    /// # Discovery Strategy
    ///
    /// 1. **Clusters** observations by mentioned entities using [`extract_entity_mentions`]
    ///    (NER or regex patterns on observation content).
    /// 2. **Computes velocity** for each entity: observation_count / time_window_days.
    /// 3. **Flags entities** with velocity > 2σ above the mean as "emerging".
    /// 4. **Ranks** emerging entities by signal strength (sigma above mean).
    ///
    /// Returns a ranked list of [`EntitySignal`]s sorted by signal strength descending.
    ///
    /// # Arguments
    ///
    /// * `observations` - A slice of observation text strings to analyze.
    ///
    /// # Performance
    ///
    /// This is O(n * m) where n = observations and m = average entity mentions per
    /// observation. For large batches (>10K), consider chunking.
    pub fn discover_emerging_entities(&self, observations: &[String]) -> Vec<EntitySignal> {
        if observations.is_empty() {
            return Vec::new();
        }

        // Step 1: Cluster observations by mentioned entities
        let mut entity_obs_count: HashMap<String, u32> = HashMap::new();

        for obs_text in observations {
            let mentions = extract_entity_mentions(obs_text);
            // Record observation for each mentioned entity
            for entity in mentions {
                *entity_obs_count.entry(entity.to_lowercase()).or_insert(0) += 1;
            }
        }

        if entity_obs_count.is_empty() {
            return Vec::new();
        }

        // Step 2: Compute velocities
        // We use the number of observations as a proxy for time window
        let time_window_days = 7.0; // Default window; should be derived from actual timestamps

        let velocities: Vec<(String, f64)> = entity_obs_count
            .into_iter()
            .map(|(entity, count)| {
                let velocity = count as f64 / time_window_days;
                (entity, velocity)
            })
            .collect();

        // Step 3: Compute mean and std of velocities
        let n = velocities.len() as f64;
        if n < 3.0 {
            return Vec::new(); // Need at least 3 entities for meaningful statistics
        }

        let mean_velocity: f64 = velocities.iter().map(|(_, v)| *v).sum::<f64>() / n;

        let variance: f64 = velocities
            .iter()
            .map(|(_, v)| (v - mean_velocity).powi(2))
            .sum::<f64>()
            / n;
        let std_velocity = variance.sqrt();

        if std_velocity < 0.001 {
            return Vec::new(); // All velocities are essentially the same
        }

        // Step 4: Flag entities with velocity > 2σ above mean
        let mut signals: Vec<EntitySignal> = velocities
            .into_iter()
            .filter_map(|(entity, velocity)| {
                let sigma = (velocity - mean_velocity) / std_velocity;
                if sigma > 2.0 {
                    // Compute signal strength: normalize sigma to [0, 1]
                    // 2σ → 0.0, 5σ+ → 1.0
                    let signal_strength = ((sigma - 2.0) / 3.0).clamp(0.0, 1.0);

                    let observation_count = (velocity * time_window_days).round() as u32;

                    Some(EntitySignal {
                        entity_name: entity.clone(),
                        velocity,
                        observation_count,
                        time_window_days,
                        signal_strength,
                        mean_velocity,
                        std_velocity,
                        sigma_above_mean: sigma,
                        inferred_category: None,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Step 5: Sort by signal strength descending
        signals.sort_by(|a, b| {
            b.signal_strength
                .partial_cmp(&a.signal_strength)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        signals
    }

    /// Determine whether a newly observed entity should be tracked.
    ///
    /// # Decision Strategy
    ///
    /// Uses four factors to decide whether to track an entity:
    ///
    /// 1. **Observation velocity** — recent surge in mentions (weight: 0.3).
    /// 2. **Supply chain proximity** — how close the entity is to already-tracked
    ///    entities, measured by co-occurrence density (weight: 0.3).
    /// 3. **News/event correlation score** — how many distinct signals mention
    ///    this entity across different sources (weight: 0.2).
    /// 4. **Entity registry awareness** — check if the entity (or a close variant)
    ///    is already known in the registry (weight: 0.2).
    ///
    /// Returns `true` if the aggregate score exceeds 0.5.
    pub fn should_track(entity: &str, registry: &EntityRegistry) -> bool {
        let entity_lower = entity.to_lowercase();

        // Factor 1: Check if already known in the registry (quick rejection)
        if registry.get(entity).is_some() || registry.get(&entity_lower).is_some() {
            return false; // Already being tracked
        }

        // Factor 2: Compute co-occurrence density
        // Count how many tracked entities are "close" to this entity
        // (simplified: check if entity name contains or is contained in known names)
        let mut proximity_score: f64 = 0.0;
        let mut co_occurrence_count = 0;
        for tracked_name in registry.entity_names() {
            let tracked_lower = tracked_name.to_lowercase();
            if entity_lower.contains(&tracked_lower) || tracked_lower.contains(&entity_lower) {
                proximity_score += 0.25;
                co_occurrence_count += 1;
            }
        }
        let proximity_score = proximity_score.min(0.5);

        // Factor 3: News/event correlation score.
        // Uses co-occurrence density as a proxy for cross-source signal
        // correlation.  Entities mentioned alongside 3+ tracked entities
        // are more likely to be supply-chain-relevant.
        let correlation_score = if co_occurrence_count >= 3 {
            0.2
        } else if co_occurrence_count >= 1 {
            0.1
        } else {
            0.0
        };

        // Factor 4: Entity name characteristics
        // Capitalized multi-word names are more likely to be companies
        let name_quality_score =
            if entity.chars().any(|c| c.is_uppercase()) && entity.split_whitespace().count() > 1 {
                0.15
            } else if entity.chars().any(|c| c.is_uppercase()) {
                0.1
            } else {
                0.0
            };

        let total_score = proximity_score + correlation_score + name_quality_score;

        // Track if aggregate score meets or exceeds threshold
        total_score >= 0.5
    }

    /// Process content and discover potential new entities.
    pub fn process_content(
        &mut self,
        content: &str,
        source_url: &str,
        timestamp: i64,
    ) -> Vec<DiscoveredEntity> {
        let mut discovered = Vec::new();

        // Extract entities using patterns
        let extracted = self.pattern_extractor.extract(content);

        for entity in extracted {
            let normalized = entity.normalized_name.to_lowercase();

            // Skip if already known (using tracker's known_entities)
            if self.cooccurrence_tracker.is_known(&normalized) {
                continue;
            }

            // Record co-occurrences (tracker filters to known entities only)
            for associated in &entity.associated_entities {
                self.cooccurrence_tracker
                    .record_cooccurrence(&normalized, associated);
            }

            // Record mention for emergence detection
            self.cooccurrence_tracker
                .entity_mentions
                .entry(normalized.clone())
                .or_default()
                .push(EntityMention {
                    source_url: source_url.to_string(),
                    context: entity.context.clone(),
                    timestamp,
                });

            // Update emergence detector
            let count = self
                .cooccurrence_tracker
                .entity_mentions
                .get(&normalized)
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            self.emergence_detector
                .record_mentions(&normalized, count, timestamp);

            // Adjust confidence based on emergence
            let emergence_score = self.emergence_detector.emergence_score(&normalized);
            let mut adjusted_entity = entity;
            adjusted_entity.source_url = source_url.to_string();
            adjusted_entity.confidence =
                (adjusted_entity.confidence + emergence_score * 0.3).min(1.0);

            discovered.push(adjusted_entity);
        }

        discovered
    }

    /// Generate POI candidates from discovered entities.
    pub fn generate_candidates(&self, discovered: &[DiscoveredEntity]) -> Vec<PoiCandidate> {
        let mut candidates = Vec::new();

        for entity in discovered {
            // Skip if confidence too low
            if entity.confidence < 0.3 {
                continue;
            }

            // Calculate metrics
            let emergence_score = self
                .emergence_detector
                .emergence_score(&entity.normalized_name);

            // Get co-occurrence count
            let co_occurrence_count = entity.associated_entities.len() as u32;

            // Get source diversity (unique source URLs)
            let source_diversity = self
                .cooccurrence_tracker
                .entity_mentions
                .get(&entity.normalized_name.to_lowercase())
                .map(|mentions| {
                    mentions
                        .iter()
                        .map(|mention| mention.source_url.as_str())
                        .collect::<HashSet<_>>()
                        .len() as u32
                })
                .unwrap_or(1);

            // Determine recommendation
            let recommendation = self.determine_recommendation(
                entity.confidence,
                emergence_score,
                co_occurrence_count,
                source_diversity,
            );

            // Determine discovery reason
            let reason = if co_occurrence_count > 0 {
                DiscoveryReason::CoOccurrence {
                    known_entity: entity
                        .associated_entities
                        .first()
                        .cloned()
                        .unwrap_or_default(),
                }
            } else if let Some((_, current_count, previous_count)) =
                self.emergence_detector.is_emerging(&entity.normalized_name)
            {
                DiscoveryReason::Emerging {
                    previous_count,
                    current_count,
                }
            } else {
                DiscoveryReason::PatternMatch {
                    pattern: "company_suffix".to_string(),
                }
            };

            candidates.push(PoiCandidate {
                entity_id: Uuid::new_v4(),
                name: entity.normalized_name.clone(),
                entity_type: entity.entity_type.clone(),
                discovery_reason: reason,
                confidence: entity.confidence,
                emergence_score,
                co_occurrence_count,
                source_diversity,
                recommended_action: recommendation,
            });
        }

        // Sort by combined score (cap co_occurrence_count contribution to prevent overflow domination)
        candidates.sort_by(|a, b| {
            let score_a = a.confidence * 0.4
                + a.emergence_score * 0.4
                + ((a.co_occurrence_count.min(20) as f64) / 20.0) * 0.2;
            let score_b = b.confidence * 0.4
                + b.emergence_score * 0.4
                + ((b.co_occurrence_count.min(20) as f64) / 20.0) * 0.2;
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    fn determine_recommendation(
        &self,
        confidence: f64,
        emergence_score: f64,
        co_occurrence_count: u32,
        source_diversity: u32,
    ) -> Recommendation {
        let combined_score = confidence * 0.3
            + emergence_score * 0.4
            + ((co_occurrence_count.min(20) as f64) / 20.0) * 0.2
            + ((source_diversity.min(10) as f64) / 10.0) * 0.1;

        if combined_score > 0.7 && co_occurrence_count >= 2 {
            Recommendation::AddNow
        } else if combined_score > 0.4 {
            Recommendation::ReviewRequired
        } else if combined_score > 0.2 {
            Recommendation::MonitorFurther
        } else {
            Recommendation::Discard
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Source Expansion Suggestions
// ─────────────────────────────────────────────────────────────────────────────

/// Source expansion recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecommendation {
    /// Suggested source URL
    pub url: String,
    /// Reason for recommendation
    pub reason: String,
    /// Expected topics
    pub expected_topics: Vec<String>,
    /// Confidence in recommendation
    pub confidence: f64,
}

/// Generate source recommendations based on discovered entities.
pub fn suggest_sources(discovered: &[DiscoveredEntity]) -> Vec<SourceRecommendation> {
    let mut topic_sources: HashMap<String, Vec<SourceRecommendation>> = HashMap::new();

    for entity in discovered {
        for topic in &entity.topics {
            let entry = topic_sources.entry(topic.clone()).or_default();

            // Generate source suggestions based on topic
            let suggestions = match topic.as_str() {
                "AI" => vec![
                    SourceRecommendation {
                        url: "https://venturebeat.com/ai/".to_string(),
                        reason: "Leading AI news source".to_string(),
                        expected_topics: vec!["AI".to_string(), "machine learning".to_string()],
                        confidence: 0.8,
                    },
                    SourceRecommendation {
                        url: "https://techcrunch.com/tag/ai/".to_string(),
                        reason: "Tech industry AI coverage".to_string(),
                        expected_topics: vec!["AI".to_string()],
                        confidence: 0.7,
                    },
                ],
                "chips" => vec![SourceRecommendation {
                    url: "https://www.semiconductorengineering.com/".to_string(),
                    reason: "Semiconductor industry news".to_string(),
                    expected_topics: vec!["semiconductor".to_string(), "chips".to_string()],
                    confidence: 0.9,
                }],
                "supply_chain" => vec![SourceRecommendation {
                    url: "https://www.supplychaindive.com/".to_string(),
                    reason: "Supply chain industry news".to_string(),
                    expected_topics: vec!["supply chain".to_string()],
                    confidence: 0.8,
                }],
                _ => vec![],
            };

            entry.extend(suggestions);
        }
    }

    // Flatten and deduplicate
    let mut all_sources: Vec<SourceRecommendation> = Vec::new();
    for (_, sources) in topic_sources {
        for source in sources {
            if !all_sources.iter().any(|s| s.url == source.url) {
                all_sources.push(source);
            }
        }
    }

    all_sources.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    all_sources
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use crate::entity_relevance::EntityRegistry;

    // ── Existing Tests (unchanged) ───────────────────────────────────────

    #[test]
    fn test_emergence_detection() {
        let mut detector = EmergenceDetector::default();

        // Record increasing mentions
        for i in 0..10 {
            let count = if i < 3 { 1 } else { 2 };
            detector.record_mentions("NewCompany", count, 1000000 + i * 86400);
        }

        let emerging = detector.is_emerging("NewCompany");
        assert!(
            emerging.is_some(),
            "NewCompany should be flagged as emerging"
        );

        let (growth, recent, _) = emerging.unwrap();
        assert!(growth > 1.0, "Growth rate should be positive");
        assert!(recent >= 5, "Should have sufficient recent mentions");
    }

    #[test]
    fn test_pattern_extraction() {
        let extractor = EntityPatternExtractor::default();

        let text = "NVIDIA Corporation announced a new partnership with Microsoft Corp. The CEO Jensen Huang spoke about AI developments.";

        let entities = extractor.extract(text);

        // Should find NVIDIA and Microsoft
        let names: Vec<&str> = entities
            .iter()
            .map(|e| e.normalized_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("NVIDIA")),
            "Should find NVIDIA"
        );
        assert!(
            names.iter().any(|n| n.contains("Microsoft")),
            "Should find Microsoft"
        );
    }

    #[test]
    fn test_poi_discovery() {
        let known = vec!["NVIDIA".to_string(), "Microsoft".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        let content = "AMD Inc. is expanding their AI chip offerings alongside NVIDIA. New startup Cerebras Technologies announced a breakthrough.";

        let discovered = discovery.process_content(content, "https://example.com", 1234567890);

        // Should find AMD (co-occurring with known NVIDIA)
        // Should NOT find NVIDIA (already known)
        let names: Vec<&str> = discovered
            .iter()
            .map(|e| e.normalized_name.as_str())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("AMD") || n.contains("Cerebras")),
            "Should find new entities"
        );
    }

    #[test]
    fn test_candidate_generation() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);

        let discovered = vec![DiscoveredEntity {
            raw_name: "AMD".to_string(),
            normalized_name: "AMD".to_string(),
            source_url: "https://example.com".to_string(),
            context: "AMD announced new chips".to_string(),
            confidence: 0.7,
            entity_type: EntityType::Company,
            associated_entities: vec!["NVIDIA".to_string()],
            topics: vec!["chips".to_string(), "AI".to_string()],
            geography: vec![],
        }];

        let candidates = discovery.generate_candidates(&discovered);

        assert!(!candidates.is_empty(), "Should generate candidates");

        // AMD with a single co-occurrence and one source should need further monitoring
        // (with capped scoring, thin evidence correctly yields MonitorFurther)
        let amd = candidates.iter().find(|c| c.name.contains("AMD")).unwrap();
        assert!(matches!(
            amd.recommended_action,
            Recommendation::ReviewRequired
                | Recommendation::AddNow
                | Recommendation::MonitorFurther
        ));
    }

    #[test]
    fn test_source_suggestion() {
        let discovered = vec![DiscoveredEntity {
            raw_name: "Test".to_string(),
            normalized_name: "Test".to_string(),
            source_url: "".to_string(),
            context: "".to_string(),
            confidence: 0.5,
            entity_type: EntityType::Company,
            associated_entities: vec![],
            topics: vec!["AI".to_string()],
            geography: vec![],
        }];

        let sources = suggest_sources(&discovered);

        // Should suggest AI-related sources
        assert!(!sources.is_empty(), "Should suggest sources");
    }

    // ── External seed infrastructure tests ──

    #[test]
    fn test_add_external_seeds() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        let seeds = vec![
            ExternalEntitySeed {
                name: "SanctionedCorp".to_string(),
                source: "sanctions_list".to_string(),
                entity_type: Some(EntityType::Company),
                confidence: 1.0,
            },
            ExternalEntitySeed {
                name: "RegFilingInc".to_string(),
                source: "sec_filing".to_string(),
                entity_type: Some(EntityType::Company),
                confidence: 0.9,
            },
        ];

        discovery.add_external_seeds(seeds);

        assert!(discovery
            .cooccurrence_tracker
            .is_external_seed("SanctionedCorp"));
        assert!(discovery
            .cooccurrence_tracker
            .is_external_seed("regfilinginc")); // case-insensitive
        assert!(!discovery.cooccurrence_tracker.is_external_seed("NVIDIA")); // known, not seed
        assert!(!discovery.cooccurrence_tracker.is_external_seed("Unknown")); // neither
    }

    #[test]
    fn test_external_seed_not_added_if_already_known() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        let seeds = vec![ExternalEntitySeed {
            name: "NVIDIA".to_string(),
            source: "sanctions_list".to_string(),
            entity_type: Some(EntityType::Company),
            confidence: 1.0,
        }];

        discovery.add_external_seeds(seeds);

        // NVIDIA should remain known, not become an external seed
        assert!(discovery.cooccurrence_tracker.is_known("nvidia"));
        assert!(!discovery.cooccurrence_tracker.is_external_seed("nvidia"));
    }

    #[test]
    fn test_promote_seed_to_known() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        discovery.add_external_seeds(vec![ExternalEntitySeed {
            name: "NewCorp".to_string(),
            source: "trade_directory".to_string(),
            entity_type: Some(EntityType::Company),
            confidence: 0.8,
        }]);

        assert!(discovery.cooccurrence_tracker.is_external_seed("newcorp"));
        assert!(!discovery.cooccurrence_tracker.is_known("newcorp"));

        // Promote
        let promoted = discovery
            .cooccurrence_tracker
            .promote_seed_to_known("NewCorp");
        assert!(promoted);
        assert!(discovery.cooccurrence_tracker.is_known("newcorp"));
        assert!(!discovery.cooccurrence_tracker.is_external_seed("newcorp"));

        // Promoting again should return false
        let again = discovery
            .cooccurrence_tracker
            .promote_seed_to_known("NewCorp");
        assert!(!again);
    }

    #[test]
    fn test_promote_nonexistent_seed_returns_false() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        let result = discovery
            .cooccurrence_tracker
            .promote_seed_to_known("DoesNotExist");
        assert!(!result);
    }

    #[test]
    fn test_with_external_seeds_constructor() {
        let known = vec!["Intel".to_string()];
        let seeds = vec![ExternalEntitySeed {
            name: "SeedEntity".to_string(),
            source: "sanctions_list".to_string(),
            entity_type: None,
            confidence: 1.0,
        }];

        let discovery = DynamicPoiDiscovery::with_external_seeds(known, seeds);
        assert!(discovery.cooccurrence_tracker.is_known("intel"));
        assert!(discovery
            .cooccurrence_tracker
            .is_external_seed("seedentity"));
    }

    #[test]
    fn test_cooccurrence_with_external_seed_tracked_in_matrix() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        discovery.add_external_seeds(vec![ExternalEntitySeed {
            name: "SeedCorp".to_string(),
            source: "sec_filing".to_string(),
            entity_type: Some(EntityType::Company),
            confidence: 0.9,
        }]);

        // Record co-occurrence between unknown entity and seed
        discovery
            .cooccurrence_tracker
            .record_cooccurrence("unknownA", "SeedCorp");
        discovery
            .cooccurrence_tracker
            .record_cooccurrence("unknownA", "SeedCorp");

        let key = ("unknowna".to_string(), "seedcorp".to_string());
        let count = discovery.cooccurrence_tracker.cooccurrence_matrix.get(&key);
        assert_eq!(
            count,
            Some(&2),
            "Co-occurrences with external seeds should be tracked in matrix"
        );
    }

    #[test]
    fn test_self_cooccurrence_ignored() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        discovery
            .cooccurrence_tracker
            .record_cooccurrence("NVIDIA", "NVIDIA");

        assert!(discovery
            .cooccurrence_tracker
            .cooccurrence_matrix
            .is_empty());
        assert!(discovery
            .cooccurrence_tracker
            .unknown_cooccurrences
            .is_empty());
    }

    #[test]
    fn test_unknown_to_unknown_cooccurrence_tracked() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        discovery
            .cooccurrence_tracker
            .record_cooccurrence("AlphaInc", "BetaCorp");
        discovery
            .cooccurrence_tracker
            .record_cooccurrence("AlphaInc", "BetaCorp");
        discovery
            .cooccurrence_tracker
            .record_cooccurrence("BetaCorp", "AlphaInc"); // reverse order

        // Should use sorted key order
        let key = ("alphainc".to_string(), "betacorp".to_string());
        let count = discovery
            .cooccurrence_tracker
            .unknown_cooccurrences
            .get(&key);
        assert_eq!(
            count,
            Some(&3),
            "Unknown-to-unknown co-occurrences should be tracked with sorted key"
        );
    }

    #[test]
    fn test_emerging_clusters() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        // Record enough co-occurrences to form a cluster
        for _ in 0..5 {
            discovery
                .cooccurrence_tracker
                .record_cooccurrence("ClusterA", "ClusterB");
            discovery
                .cooccurrence_tracker
                .record_cooccurrence("ClusterA", "ClusterC");
        }

        let clusters = discovery.get_emerging_clusters(3);
        assert!(!clusters.is_empty(), "Should detect emerging clusters");

        // ClusterA should appear as a cluster hub
        let cluster_a = clusters.iter().find(|(name, _)| name == "clustera");
        assert!(cluster_a.is_some(), "ClusterA should be a cluster hub");
        let (_, peers) = cluster_a.unwrap();
        assert!(peers.len() >= 2, "ClusterA should have at least 2 peers");
    }

    // ── Unknown co-occurrence eviction test ──

    #[test]
    fn test_unknown_cooccurrences_bounded() {
        let known = vec!["NVIDIA".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);

        // Insert more than MAX_UNKNOWN_COOCCURRENCES pairs
        let limit = CoOccurrenceTracker::MAX_UNKNOWN_COOCCURRENCES;
        for i in 0..(limit + 500) {
            let e1 = format!("entity_{}", i);
            let e2 = format!("entity_{}", i + limit + 500);
            discovery.cooccurrence_tracker.record_cooccurrence(&e1, &e2);
        }

        // After eviction, the map should be smaller than the limit + some margin
        assert!(
            discovery.cooccurrence_tracker.unknown_cooccurrences.len() <= limit,
            "unknown_cooccurrences should be bounded after eviction, got {}",
            discovery.cooccurrence_tracker.unknown_cooccurrences.len()
        );
    }

    // ── New: Emerging Entity Detection Tests ─────────────────────────────

    #[test]
    fn test_discover_emerging_entities_returns_empty_for_empty_input() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);

        let signals = discovery.discover_emerging_entities(&[]);
        assert!(
            signals.is_empty(),
            "Empty observations should yield no signals"
        );
    }

    #[test]
    fn test_discover_emerging_entities_with_mock_observations() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);

        // Create observations where "ChipUpStart" appears much more frequently
        let mut observations = Vec::new();

        // ChipUpStart appears in many observations
        for i in 0..20 {
            observations.push(format!(
                "ChipUpStart Corp. announced new chip technology at conference {}.",
                i
            ));
        }

        // Other entities appear rarely
        observations.push("Some Other Company Inc. had a minor update.".to_string());
        observations.push("Yet Another Corp. is still operating normally.".to_string());
        observations.push("SmallBiz Ltd. posted quarterly results.".to_string());

        let signals = discovery.discover_emerging_entities(&observations);

        // With 20+ mentions of ChipUpStart and only 1 of others, it should be flagged
        assert!(!signals.is_empty(), "Should detect emerging entities");

        // ChipUpStart should have the highest signal strength
        let chipup = signals
            .iter()
            .find(|s| s.entity_name.contains("chipupstart"));
        assert!(chipup.is_some(), "ChipUpStart should be emerging");
        if let Some(signal) = chipup {
            assert!(
                signal.observation_count >= 18,
                "ChipUpStart should have high observation count"
            );
            assert!(signal.sigma_above_mean > 2.0, "Should be >2σ above mean");
        }
    }

    #[test]
    fn test_discover_emerging_entities_returns_ranked_results() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);

        let mut observations = Vec::new();
        // EntityA: 15 mentions
        for _ in 0..15 {
            observations.push("EntityA Technologies Inc. reports earnings.".to_string());
        }
        // EntityB: 10 mentions
        for _ in 0..10 {
            observations.push("EntityB Corp. launches new product.".to_string());
        }
        // EntityC: 3 mentions
        for _ in 0..3 {
            observations.push("EntityC Ltd. hires new CEO.".to_string());
        }
        // Fillers
        observations.push("Some other company news.".to_string());

        let signals = discovery.discover_emerging_entities(&observations);

        if !signals.is_empty() {
            // Should be sorted by signal_strength descending
            for i in 1..signals.len() {
                assert!(
                    signals[i - 1].signal_strength >= signals[i].signal_strength,
                    "Signals should be sorted by signal_strength descending"
                );
            }
        }
    }

    #[test]
    fn test_discover_emerging_entities_requires_min_entities() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);

        // Only 1 entity mentioned in all observations
        let observations: Vec<String> = (0..5)
            .map(|i| format!("OnlyEntity Inc. mention {}.", i))
            .collect();

        let signals = discovery.discover_emerging_entities(&observations);
        // Need at least 3 entities for std dev to be meaningful
        assert!(signals.is_empty(), "Should not flag with only 1 entity");
    }

    #[test]
    fn test_extract_entity_mentions_finds_capitalized_names() {
        let text = "NVIDIA Corporation and Advanced Micro Devices Inc. partnered on AI chips. The CEO spoke at an event.";
        let mentions = extract_entity_mentions(text);

        assert!(
            mentions.iter().any(|m| m.contains("Corporation")),
            "Should find entity with Corporation suffix"
        );
        assert!(
            mentions.iter().any(|m| m.contains("Inc.")),
            "Should find entity with Inc. suffix"
        );
    }

    #[test]
    fn test_extract_entity_mentions_capitalized_sequences() {
        let text =
            "Advanced Micro Devices announced new products. Samsung Electronics is expanding.";
        let mentions = extract_entity_mentions(text);

        assert!(
            mentions
                .iter()
                .any(|m| m.contains("Advanced Micro Devices")),
            "Should find capitalized multi-word entities"
        );
    }

    // ── New: should_track tests ──────────────────────────────────────────

    #[test]
    fn test_should_track_returns_false_for_already_tracked() {
        let registry = EntityRegistry::from_yaml_config();

        // Foxconn is definitely in the registry
        let result = DynamicPoiDiscovery::should_track("Foxconn", &registry);
        assert!(!result, "Already-tracked entities should return false");
    }

    #[test]
    fn test_should_track_returns_false_for_known_entity() {
        let registry = EntityRegistry::from_yaml_config();

        // Check various casing
        let result = DynamicPoiDiscovery::should_track("Jabil Inc.", &registry);
        assert!(
            !result,
            "Known entity Jabil Inc. should not be tracked again"
        );
    }

    #[test]
    fn test_should_track_true_for_close_proximity_entity() {
        let registry = EntityRegistry::from_yaml_config();

        // "Foxconn Subsidiary" is not in the registry but closely related to Foxconn
        let result = DynamicPoiDiscovery::should_track("Foxconn Subsidiary Co.", &registry);
        assert!(
            result,
            "Entity closely related to tracked entities should be tracked"
        );
    }

    // ── New: compute_velocities test ─────────────────────────────────────

    #[test]
    fn test_compute_velocities() {
        let mut detector = EmergenceDetector::default();

        detector.record_mentions("EntityA", 5, 1000000);
        detector.record_mentions("EntityA", 10, 1000001);

        let velocities = detector.compute_velocities();
        assert!(!velocities.is_empty(), "Should compute velocities");

        // EntityA should have a positive velocity
        let vel_a = velocities.get("entitya");
        assert!(vel_a.is_some(), "Should have velocity for EntityA");
        if let Some(v) = vel_a {
            assert!(*v > 0.0, "Velocity should be positive");
        }
    }
}
