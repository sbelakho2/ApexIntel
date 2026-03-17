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

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A potential new entity discovered from content
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

/// A candidate entity for addition to the monitoring list
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
    Emerging { previous_count: u32, current_count: u32 },
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

/// Entity co-occurrence tracker
#[derive(Debug, Clone)]
pub struct CoOccurrenceTracker {
    /// Known entities to track co-occurrences with
    known_entities: HashSet<String>,
    /// Co-occurrence counts
    cooccurrence_matrix: HashMap<(String, String), u32>,
    /// Recent mentions per entity
    entity_mentions: HashMap<String, Vec<EntityMention>>,
}

impl CoOccurrenceTracker {
    /// Check if an entity is already known
    pub fn is_known(&self, entity: &str) -> bool {
        self.known_entities.contains(&entity.to_lowercase())
    }

    /// Record a co-occurrence, filtering to only track against known entities
    pub fn record_cooccurrence(&mut self, entity: &str, associated_entity: &str) {
        let associated_lower = associated_entity.to_lowercase();
        if self.known_entities.contains(&associated_lower) {
            let key = (entity.to_lowercase(), associated_lower);
            *self.cooccurrence_matrix.entry(key).or_insert(0) += 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct EntityMention {
    pub source_url: String,
    pub context: String,
    pub timestamp: i64,
}

/// Entity emergence detector - finds entities appearing more frequently
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
    /// Record mentions of an entity
    pub fn record_mentions(&mut self, entity_name: &str, count: u32, timestamp: i64) {
        let entry = self.historical_counts
            .entry(entity_name.to_lowercase())
            .or_insert_with(Vec::new);
        entry.push((timestamp, count));
    }

    /// Check if an entity is emerging (appearing more frequently)
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

        let recent: Vec<u32> = counts.iter()
            .rev()
            .take(recent_window)
            .map(|(_, c)| *c)
            .collect();
        
        let older: Vec<u32> = counts.iter()
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
        
        // Only consider emerging if growth rate > 1.5x and meets minimum
        if growth_rate > 1.5 && recent_sum >= self.min_mentions {
            return Some((growth_rate, recent_sum, older_sum));
        }
        
        None
    }

    /// Get emergence score (0.0 - 1.0)
    pub fn emergence_score(&self, entity_name: &str) -> f64 {
        if let Some((growth_rate, recent_count, _)) = self.is_emerging(entity_name) {
            // Normalize to 0-1: growth rate of 1.5 = 0.0, growth rate of 5.0+ = 1.0
            let score = ((growth_rate - 1.5) / 3.5).min(1.0).max(0.0);
            
            // Boost by recent volume
            let volume_boost = (recent_count as f64 / 50.0).min(0.3);
            
            (score + volume_boost).min(1.0)
        } else {
            0.0
        }
    }
}

/// Pattern-based entity extractor
pub struct EntityPatternExtractor {
    /// Company name patterns
    company_patterns: Vec<CompanyPattern>,
    /// Person name patterns
    person_patterns: Vec<&'static str>,
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
                CompanyPattern { pattern: "Inc.", weight: 0.8 },
                CompanyPattern { pattern: "Corporation", weight: 0.8 },
                CompanyPattern { pattern: "Corp.", weight: 0.8 },
                CompanyPattern { pattern: "LLC", weight: 0.8 },
                CompanyPattern { pattern: "Ltd.", weight: 0.8 },
                CompanyPattern { pattern: "Co.", weight: 0.6 },
                CompanyPattern { pattern: "Group", weight: 0.5 },
                CompanyPattern { pattern: "Technologies", weight: 0.6 },
                CompanyPattern { pattern: "Semiconductor", weight: 0.7 },
                CompanyPattern { pattern: "Systems", weight: 0.5 },
                CompanyPattern { pattern: "Solutions", weight: 0.5 },
                CompanyPattern { pattern: "Holdings", weight: 0.6 },
            ],
            person_patterns: vec![
                "CEO", "CFO", "CTO", "COO", "CMO", "CIO",
                "President", "Vice President", "VP",
                "Director", "Manager", "Founder", "Co-Founder",
                "Chairman", "Board Member",
            ],
        }
    }
}

impl EntityPatternExtractor {
    /// Extract potential entities from text
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
            let cleaned = candidate.replace(|c: char| !c.is_alphanumeric() && c != ' ' && c != '.' && c != ',' , "");
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
        // Known entities to look for co-occurrences
        let known = vec![
            "NVIDIA", "TSMC", "Foxconn", "Samsung", "Intel", "AMD", "Qualcomm",
            "Apple", "Google", "Microsoft", "Amazon", "Tesla", "Meta", "OpenAI",
            "IBM", "Oracle", "Cisco", "Broadcom", "Micron", "SK Hynix",
        ];
        
        let mut found = Vec::new();
        let text_lower = text.to_lowercase();
        
        for entity in known {
            if text_lower.contains(&entity.to_lowercase()) {
                found.push(entity.to_string());
            }
        }
        
        found
    }

    fn extract_topics(&self, text: &str) -> Vec<String> {
        let topic_keywords = vec![
            ("AI", vec!["ai", "artificial intelligence", "machine learning", "ml"]),
            ("chips", vec!["chip", "semiconductor", "gpu", "processor"]),
            ("supply_chain", vec!["supply chain", "supplier", "manufacturing"]),
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

/// Main POI discovery service
pub struct DynamicPoiDiscovery {
    cooccurrence_tracker: CoOccurrenceTracker,
    emergence_detector: EmergenceDetector,
    pattern_extractor: EntityPatternExtractor,
}

impl DynamicPoiDiscovery {
    /// Create a new discovery service
    pub fn new(known_entities: Vec<String>) -> Self {
        let known_set: HashSet<String> = known_entities
            .into_iter()
            .map(|e| e.to_lowercase())
            .collect();
        
        Self {
            cooccurrence_tracker: CoOccurrenceTracker {
                known_entities: known_set,
                cooccurrence_matrix: HashMap::new(),
                entity_mentions: HashMap::new(),
            },
            emergence_detector: EmergenceDetector::default(),
            pattern_extractor: EntityPatternExtractor::default(),
        }
    }

    /// Process content and discover potential new entities
    pub fn process_content(&mut self, content: &str, source_url: &str, timestamp: i64) -> Vec<DiscoveredEntity> {
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
                self.cooccurrence_tracker.record_cooccurrence(&normalized, associated);
            }
            
            // Record mention for emergence detection
            self.cooccurrence_tracker.entity_mentions
                .entry(normalized.clone())
                .or_insert_with(Vec::new)
                .push(EntityMention {
                    source_url: source_url.to_string(),
                    context: entity.context.clone(),
                    timestamp,
            });
            
            // Update emergence detector
            let count = self.cooccurrence_tracker.entity_mentions
                .get(&normalized)
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            self.emergence_detector.record_mentions(&normalized, count, timestamp);
            
            // Adjust confidence based on emergence
            let emergence_score = self.emergence_detector.emergence_score(&normalized);
            let mut adjusted_entity = entity;
            adjusted_entity.confidence = (adjusted_entity.confidence + emergence_score * 0.3).min(1.0);
            
            discovered.push(adjusted_entity);
        }
        
        discovered
    }

    /// Generate POI candidates from discovered entities
    pub fn generate_candidates(&self, discovered: &[DiscoveredEntity]) -> Vec<PoiCandidate> {
        let mut candidates = Vec::new();
        
        for entity in discovered {
            // Skip if confidence too low
            if entity.confidence < 0.3 {
                continue;
            }
            
            // Calculate metrics
            let emergence_score = self.emergence_detector.emergence_score(&entity.normalized_name);
            
            // Get co-occurrence count
            let co_occurrence_count = entity.associated_entities.len() as u32;
            
            // Get source diversity (unique source URLs)
            let source_diversity = 1; // Simplified
            
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
                    known_entity: entity.associated_entities.first().cloned().unwrap_or_default()
                }
            } else if emergence_score > 0.3 {
                DiscoveryReason::Emerging {
                    previous_count: 0,
                    current_count: co_occurrence_count,
                }
            } else {
                DiscoveryReason::PatternMatch {
                    pattern: "company_suffix".to_string()
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
        
        // Sort by combined score
        candidates.sort_by(|a, b| {
            let score_a = a.confidence * 0.4 + a.emergence_score * 0.4 + (a.co_occurrence_count as f64) * 0.2;
            let score_b = b.confidence * 0.4 + b.emergence_score * 0.4 + (b.co_occurrence_count as f64) * 0.2;
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
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
        let combined_score = confidence * 0.3 + emergence_score * 0.4 + (co_occurrence_count as f64) * 0.2 + (source_diversity as f64) * 0.1;
        
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

/// Source expansion recommendation
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

/// Generate source recommendations based on discovered entities
pub fn suggest_sources(discovered: &[DiscoveredEntity]) -> Vec<SourceRecommendation> {
    let mut topic_sources: HashMap<String, Vec<SourceRecommendation>> = HashMap::new();
    
    for entity in discovered {
        for topic in &entity.topics {
            let entry = topic_sources.entry(topic.clone()).or_insert_with(Vec::new);
            
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
                "chips" => vec![
                    SourceRecommendation {
                        url: "https://www.semiconductorengineering.com/".to_string(),
                        reason: "Semiconductor industry news".to_string(),
                        expected_topics: vec!["semiconductor".to_string(), "chips".to_string()],
                        confidence: 0.9,
                    },
                ],
                "supply_chain" => vec![
                    SourceRecommendation {
                        url: "https://www.supplychaindive.com/".to_string(),
                        reason: "Supply chain industry news".to_string(),
                        expected_topics: vec!["supply chain".to_string()],
                        confidence: 0.8,
                    },
                ],
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
    
    all_sources.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    
    all_sources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emergence_detection() {
        let mut detector = EmergenceDetector::default();
        
        // Record increasing mentions
        for i in 0..10 {
            detector.record_mentions("NewCompany", 1, 1000000 + i * 86400);
        }
        
        let emerging = detector.is_emerging("NewCompany");
        assert!(emerging.is_some(), "NewCompany should be flagged as emerging");
        
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
        let names: Vec<&str> = entities.iter().map(|e| e.normalized_name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("NVIDIA")), "Should find NVIDIA");
        assert!(names.iter().any(|n| n.contains("Microsoft")), "Should find Microsoft");
    }

    #[test]
    fn test_poi_discovery() {
        let known = vec!["NVIDIA".to_string(), "Microsoft".to_string()];
        let mut discovery = DynamicPoiDiscovery::new(known);
        
        let content = "AMD Inc. is expanding their AI chip offerings alongside NVIDIA. New startup Cerebras Technologies announced a breakthrough.";
        
        let discovered = discovery.process_content(content, "https://example.com", 1234567890);
        
        // Should find AMD (co-occurring with known NVIDIA)
        // Should NOT find NVIDIA (already known)
        let names: Vec<&str> = discovered.iter().map(|e| e.normalized_name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("AMD") || n.contains("Cerebras")), 
            "Should find new entities");
    }

    #[test]
    fn test_candidate_generation() {
        let known = vec!["NVIDIA".to_string()];
        let discovery = DynamicPoiDiscovery::new(known);
        
        let discovered = vec![
            DiscoveredEntity {
                raw_name: "AMD".to_string(),
                normalized_name: "AMD".to_string(),
                source_url: "https://example.com".to_string(),
                context: "AMD announced new chips".to_string(),
                confidence: 0.7,
                entity_type: EntityType::Company,
                associated_entities: vec!["NVIDIA".to_string()],
                topics: vec!["chips".to_string(), "AI".to_string()],
                geography: vec![],
            },
        ];
        
        let candidates = discovery.generate_candidates(&discovered);
        
        assert!(!candidates.is_empty(), "Should generate candidates");
        
        // AMD should be recommended for review or addition
        let amd = candidates.iter().find(|c| c.name.contains("AMD")).unwrap();
        assert!(matches!(amd.recommended_action, Recommendation::ReviewRequired | Recommendation::AddNow));
    }

    #[test]
    fn test_source_suggestion() {
        let discovered = vec![
            DiscoveredEntity {
                raw_name: "Test".to_string(),
                normalized_name: "Test".to_string(),
                source_url: "".to_string(),
                context: "".to_string(),
                confidence: 0.5,
                entity_type: EntityType::Company,
                associated_entities: vec![],
                topics: vec!["AI".to_string()],
                geography: vec![],
            },
        ];
        
        let sources = suggest_sources(&discovered);
        
        // Should suggest AI-related sources
        assert!(!sources.is_empty(), "Should suggest sources");
    }
}

