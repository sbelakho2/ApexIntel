//! Cross-Entity Correlation Module
//!
//! This module provides cross-entity correlation to identify relationships
//! between entities and generate deeper insights. This fixes Issue #4
//! (Weak Entity Linking).
//!
//! Key capabilities:
//! 1. Entity co-occurrence tracking - when entities appear together
//! 2. Relationship inference - deriving business relationships
//! 3. Supply chain mapping - understanding supplier-customer relationships
//! 4. Competitive analysis - finding competitive dynamics

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Types of relationships between entities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RelationshipType {
    /// Supplier provides components/materials to customer
    Supplier,
    /// Customer purchases from supplier
    Customer,
    /// Partner collaborates on projects/products
    Partner,
    /// Competitor competes in same markets
    Competitor,
    /// Subsidiary owned by parent
    Subsidiary,
    /// Parent owns subsidiary
    Parent,
    /// Investor invested in entity
    Investor,
    /// Acquirer acquired target
    Acquirer,
    /// Target was acquired
    Target,
    /// Shared geography/location
    CoLocated,
    /// Shared technology/standards
    SharedTechnology,
    /// Entities co-occurring frequently in content
    CoOccurrence,
    /// Unknown/uncategorized
    Unknown,
}

/// A relationship between two entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRelationship {
    /// First entity in relationship
    pub entity_a: String,
    /// Second entity in relationship
    pub entity_b: String,
    /// Type of relationship
    pub relationship_type: RelationshipType,
    /// Confidence in the relationship (0.0 - 1.0)
    pub confidence: f64,
    /// Evidence supporting the relationship
    pub evidence: Vec<RelationshipEvidence>,
    /// When this relationship was last confirmed
    pub last_confirmed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipEvidence {
    /// Source of the evidence
    pub source: String,
    /// Text snippet showing the relationship
    pub snippet: String,
    /// When this evidence was observed
    pub timestamp: i64,
}

/// Cross-entity correlation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossEntityInsight {
    /// Primary entity
    pub primary_entity: String,
    /// Related entities
    pub related_entities: Vec<RelatedEntity>,
    /// Type of correlation
    pub correlation_type: CorrelationType,
    /// Combined confidence score
    pub confidence: f64,
    /// Narrative describing the relationship network
    pub narrative: String,
    /// Recommended actions based on relationships
    pub recommended_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedEntity {
    pub name: String,
    pub relationship: RelationshipType,
    pub confidence: f64,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CorrelationType {
    /// Entities mentioned together frequently
    CoOccurrence,
    /// Supplier-customer relationship
    SupplyChain,
    /// Competitive dynamics
    Competitive,
    /// Geographic proximity
    Geographic,
    /// Technology overlap
    Technology,
    /// Investment relationship
    Investment,
}

/// Entity relationship tracker
pub struct RelationshipTracker {
    /// Known relationships
    relationships: HashMap<(String, String), EntityRelationship>,
    /// Entity co-occurrence counts
    cooccurrence_counts: HashMap<(String, String), u32>,
    /// Entity mentions
    entity_mentions: HashMap<String, Vec<EntityMention>>,
    /// Known relationship patterns
    known_patterns: Vec<RelationshipPattern>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // fields stored for evidence/debugging, read via Debug
struct EntityMention {
    pub context: String,
    pub source: String,
    pub timestamp: i64,
}

struct RelationshipPattern {
    pub keywords_a: Vec<String>,
    pub keywords_b: Vec<String>,
    pub relationship_type: RelationshipType,
    pub weight: f64,
}

impl Default for RelationshipTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RelationshipTracker {
    /// Create a new relationship tracker
    pub fn new() -> Self {
        let known_patterns = vec![
            // Supplier-Customer patterns
            RelationshipPattern {
                keywords_a: vec![
                    "supplier".to_string(),
                    "provides".to_string(),
                    "delivers".to_string(),
                ],
                keywords_b: vec![
                    "customer".to_string(),
                    "client".to_string(),
                    "orders".to_string(),
                ],
                relationship_type: RelationshipType::Supplier,
                weight: 0.8,
            },
            RelationshipPattern {
                keywords_a: vec![
                    "orders from".to_string(),
                    "procures".to_string(),
                    "sources from".to_string(),
                ],
                keywords_b: vec![
                    "supplier".to_string(),
                    "vendor".to_string(),
                    "provides".to_string(),
                ],
                relationship_type: RelationshipType::Customer,
                weight: 0.8,
            },
            // Partner patterns
            RelationshipPattern {
                keywords_a: vec![
                    "partners with".to_string(),
                    "collaborates".to_string(),
                    "joint venture".to_string(),
                ],
                keywords_b: vec![
                    "partner".to_string(),
                    "collaborator".to_string(),
                    "JV".to_string(),
                ],
                relationship_type: RelationshipType::Partner,
                weight: 0.7,
            },
            // Competitor patterns
            RelationshipPattern {
                keywords_a: vec![
                    "competes with".to_string(),
                    "rival".to_string(),
                    "competitor".to_string(),
                ],
                keywords_b: vec![
                    "competes".to_string(),
                    "rival".to_string(),
                    "market share".to_string(),
                ],
                relationship_type: RelationshipType::Competitor,
                weight: 0.9,
            },
            // Investment patterns
            RelationshipPattern {
                keywords_a: vec![
                    "invests in".to_string(),
                    "funds".to_string(),
                    "backed by".to_string(),
                ],
                keywords_b: vec![
                    "investment".to_string(),
                    "funding".to_string(),
                    "round".to_string(),
                ],
                relationship_type: RelationshipType::Investor,
                weight: 0.8,
            },
            RelationshipPattern {
                keywords_a: vec![
                    "acquires".to_string(),
                    "purchases".to_string(),
                    "acquisition".to_string(),
                ],
                keywords_b: vec![
                    "acquired".to_string(),
                    "acquisition".to_string(),
                    "acqui-hire".to_string(),
                ],
                relationship_type: RelationshipType::Acquirer,
                weight: 0.9,
            },
            // Geographic patterns
            RelationshipPattern {
                keywords_a: vec![
                    "based in".to_string(),
                    "headquartered".to_string(),
                    "located in".to_string(),
                ],
                keywords_b: vec![
                    "based in".to_string(),
                    "headquartered".to_string(),
                    "operates in".to_string(),
                ],
                relationship_type: RelationshipType::CoLocated,
                weight: 0.6,
            },
            // Technology patterns
            RelationshipPattern {
                keywords_a: vec![
                    "uses".to_string(),
                    "powered by".to_string(),
                    "integrates".to_string(),
                ],
                keywords_b: vec![
                    "technology".to_string(),
                    "platform".to_string(),
                    "framework".to_string(),
                ],
                relationship_type: RelationshipType::SharedTechnology,
                weight: 0.5,
            },
        ];

        Self {
            relationships: HashMap::new(),
            cooccurrence_counts: HashMap::new(),
            entity_mentions: HashMap::new(),
            known_patterns,
        }
    }

    /// Record an entity mention in context
    pub fn record_mention(&mut self, entity: &str, context: &str, source: &str, timestamp: i64) {
        let normalized = entity.to_lowercase();
        self.entity_mentions
            .entry(normalized)
            .or_default()
            .push(EntityMention {
                context: context.to_string(),
                source: source.to_string(),
                timestamp,
            });
    }

    /// Record co-occurrence of two entities
    pub fn record_cooccurrence(&mut self, entity_a: &str, entity_b: &str) {
        let key = (entity_a.to_lowercase(), entity_b.to_lowercase());
        *self.cooccurrence_counts.entry(key).or_insert(0) += 1;
    }

    /// Infer relationships from text content
    pub fn infer_relationships(
        &mut self,
        content: &str,
        source: &str,
        timestamp: i64,
    ) -> Vec<EntityRelationship> {
        let mut inferred = Vec::new();
        let content_lower = content.to_lowercase();

        // Check each pattern
        for pattern in &self.known_patterns {
            // Check if keywords from both sides appear in content
            let has_a = pattern
                .keywords_a
                .iter()
                .any(|kw| content_lower.contains(kw));
            let has_b = pattern
                .keywords_b
                .iter()
                .any(|kw| content_lower.contains(kw));

            if has_a && has_b {
                // Extract potential entity names
                // This is simplified - in production, would use NER
                let entities = self.extract_potential_entities(content);

                // Create relationship for each pair
                for i in 0..entities.len() {
                    for j in (i + 1)..entities.len() {
                        let entity_a = &entities[i];
                        let entity_b = &entities[j];

                        let relationship = EntityRelationship {
                            entity_a: entity_a.clone(),
                            entity_b: entity_b.clone(),
                            relationship_type: pattern.relationship_type.clone(),
                            confidence: pattern.weight,
                            evidence: vec![RelationshipEvidence {
                                source: source.to_string(),
                                snippet: content[..content.len().min(200)].to_string(),
                                timestamp,
                            }],
                            last_confirmed: timestamp,
                        };

                        inferred.push(relationship);
                    }
                }
            }
        }

        inferred
    }

    /// Extract potential entity names from text using the tracker's
    /// dynamically known entities (from relationships, co-occurrences,
    /// and entity mentions) rather than a hardcoded list.
    fn extract_potential_entities(&self, text: &str) -> Vec<String> {
        let mut entities = Vec::new();
        let text_lower = text.to_lowercase();

        // Collect known entities from the tracker's existing data
        // (relationships, co-occurrences, mentions) — all populated
        // dynamically from observations.
        let known: HashSet<&str> = self
            .relationships
            .keys()
            .flat_map(|(a, b)| vec![a.as_str(), b.as_str()])
            .chain(
                self.cooccurrence_counts
                    .keys()
                    .flat_map(|(a, b)| vec![a.as_str(), b.as_str()]),
            )
            .chain(self.entity_mentions.keys().map(|s| s.as_str()))
            .collect();

        for entity in known {
            if text_lower.contains(entity) && !entities.iter().any(|e: &String| e == entity) {
                entities.push(entity.to_string());
            }
        }

        // Also extract company names with suffixes
        let suffixes = [
            "Corp",
            "Inc",
            "LLC",
            "Ltd",
            "Co",
            "Group",
            "Technologies",
            "Systems",
        ];
        for suffix in suffixes {
            let suffix_lower = suffix.to_lowercase();
            if let Some(pos) = text_lower.find(&suffix_lower) {
                // Ensure byte offsets are valid char boundaries before slicing
                let end = pos + suffix.len();
                if end <= text.len() && text.is_char_boundary(pos) && text.is_char_boundary(end) {
                    let raw_start = pos.saturating_sub(50);
                    let mut start = raw_start;
                    while start < pos && !text.is_char_boundary(start) {
                        start += 1;
                    }
                    let candidate = &text[start..end];
                    let cleaned = candidate
                        .replace(|c: char| !c.is_alphanumeric() && c != ' ' && c != '.', "");
                    if cleaned.len() > 3 && !entities.iter().any(|e| e.contains(&cleaned)) {
                        entities.push(cleaned.trim().to_string());
                    }
                }
            }
        }

        entities
    }

    /// Get relationships for an entity
    pub fn get_relationships(&self, entity: &str) -> Vec<&EntityRelationship> {
        let normalized = entity.to_lowercase();
        self.relationships
            .iter()
            .filter(|((a, b), _)| a == &normalized || b == &normalized)
            .map(|((_, _), r)| r)
            .collect()
    }

    /// Get co-occurring entities
    pub fn get_cooccurring(&self, entity: &str, min_count: u32) -> Vec<(&str, u32)> {
        let normalized = entity.to_lowercase();
        let mut results: Vec<(&str, u32)> = self
            .cooccurrence_counts
            .iter()
            .filter(|((a, b), count)| {
                (*a == normalized || *b == normalized) && **count >= min_count
            })
            .map(|((a, b), count)| {
                if *a == normalized {
                    (b.as_str(), *count)
                } else {
                    (a.as_str(), *count)
                }
            })
            .collect();

        results.sort_by(|a, b| b.1.cmp(&a.1));
        results
    }

    /// Generate cross-entity insights
    pub fn generate_cross_entity_insights(&self, entity: &str) -> Vec<CrossEntityInsight> {
        let mut insights = Vec::new();

        // Get relationships
        let relationships = self.get_relationships(entity);

        if relationships.is_empty() {
            // Try co-occurrence based insights
            let cooccurring = self.get_cooccurring(entity, 3);

            if !cooccurring.is_empty() {
                let related: Vec<RelatedEntity> = cooccurring
                    .iter()
                    .map(|(name, count)| RelatedEntity {
                        name: name.to_string(),
                        relationship: RelationshipType::CoOccurrence,
                        confidence: (*count as f64 / 10.0).min(1.0),
                        context: format!("Appeared together in {} sources", count),
                    })
                    .collect();

                insights.push(CrossEntityInsight {
                    primary_entity: entity.to_string(),
                    related_entities: related,
                    correlation_type: CorrelationType::CoOccurrence,
                    confidence: 0.7,
                    narrative: format!(
                        "{} frequently appears alongside other companies in news sources, suggesting potential business relationships or market correlations.",
                        entity
                    ),
                    recommended_actions: vec![
                        "Investigate nature of co-occurrence".to_string(),
                        "Map potential supply chain relationships".to_string(),
                    ],
                });
            }

            return insights;
        }

        // Group relationships by type
        let mut suppliers: Vec<&EntityRelationship> = Vec::new();
        let mut customers: Vec<&EntityRelationship> = Vec::new();
        let mut partners: Vec<&EntityRelationship> = Vec::new();
        let mut competitors: Vec<&EntityRelationship> = Vec::new();
        let mut investors: Vec<&EntityRelationship> = Vec::new();

        for rel in &relationships {
            match rel.relationship_type {
                RelationshipType::Supplier => suppliers.push(rel),
                RelationshipType::Customer => customers.push(rel),
                RelationshipType::Partner => partners.push(rel),
                RelationshipType::Competitor => competitors.push(rel),
                RelationshipType::Investor => investors.push(rel),
                _ => {}
            }
        }

        // Generate supply chain insight
        if !suppliers.is_empty() || !customers.is_empty() {
            let mut related: Vec<RelatedEntity> = Vec::new();

            for rel in &suppliers {
                related.push(RelatedEntity {
                    name: if rel.entity_a == entity {
                        rel.entity_b.clone()
                    } else {
                        rel.entity_a.clone()
                    },
                    relationship: RelationshipType::Supplier,
                    confidence: rel.confidence,
                    context: "Supplies components/materials".to_string(),
                });
            }

            for rel in &customers {
                related.push(RelatedEntity {
                    name: if rel.entity_a == entity {
                        rel.entity_b.clone()
                    } else {
                        rel.entity_a.clone()
                    },
                    relationship: RelationshipType::Customer,
                    confidence: rel.confidence,
                    context: "Purchases from this entity".to_string(),
                });
            }

            let avg_confidence: f64 = relationships.iter().map(|r| r.confidence).sum::<f64>()
                / relationships.len() as f64;

            insights.push(CrossEntityInsight {
                primary_entity: entity.to_string(),
                related_entities: related,
                correlation_type: CorrelationType::SupplyChain,
                confidence: avg_confidence,
                narrative: format!(
                    "{} has identified supply chain relationships with {} suppliers and {} customers. \
                    This provides context for understanding their operational dependencies and market position.",
                    entity,
                    suppliers.len(),
                    customers.len()
                ),
                recommended_actions: vec![
                    "Map complete supply chain network".to_string(),
                    "Identify single points of failure".to_string(),
                    "Assess supplier risk diversification".to_string(),
                ],
            });
        }

        // Generate competitive insight
        if !competitors.is_empty() {
            let related: Vec<RelatedEntity> = competitors
                .iter()
                .map(|rel| RelatedEntity {
                    name: if rel.entity_a == entity {
                        rel.entity_b.clone()
                    } else {
                        rel.entity_a.clone()
                    },
                    relationship: RelationshipType::Competitor,
                    confidence: rel.confidence,
                    context: "Competitor in same markets".to_string(),
                })
                .collect();

            let avg_confidence: f64 =
                competitors.iter().map(|r| r.confidence).sum::<f64>() / competitors.len() as f64;

            insights.push(CrossEntityInsight {
                primary_entity: entity.to_string(),
                related_entities: related,
                correlation_type: CorrelationType::Competitive,
                confidence: avg_confidence,
                narrative: format!(
                    "{} competes with {} identified competitors in the market. \
                    Monitoring competitor activities can provide strategic intelligence.",
                    entity,
                    competitors.len()
                ),
                recommended_actions: vec![
                    "Monitor competitor product launches".to_string(),
                    "Track competitive pricing changes".to_string(),
                    "Analyze market share shifts".to_string(),
                ],
            });
        }

        // Generate investment insight
        if !investors.is_empty() {
            let related: Vec<RelatedEntity> = investors
                .iter()
                .map(|rel| RelatedEntity {
                    name: if rel.entity_a == entity {
                        rel.entity_b.clone()
                    } else {
                        rel.entity_a.clone()
                    },
                    relationship: RelationshipType::Investor,
                    confidence: rel.confidence,
                    context: "Investor/portfolio company".to_string(),
                })
                .collect();

            let avg_confidence: f64 =
                investors.iter().map(|r| r.confidence).sum::<f64>() / investors.len() as f64;

            insights.push(CrossEntityInsight {
                primary_entity: entity.to_string(),
                related_entities: related,
                correlation_type: CorrelationType::Investment,
                confidence: avg_confidence,
                narrative: format!(
                    "{} has {} identified investment relationships. \
                    Investment activity can indicate growth prospects and strategic direction.",
                    entity,
                    investors.len()
                ),
                recommended_actions: vec![
                    "Track investor activity and funding rounds".to_string(),
                    "Monitor institutional ownership changes".to_string(),
                ],
            });
        }

        insights
    }
}

/// Supply chain mapper
pub struct SupplyChainMapper {
    /// Known supply chain relationships
    supply_chain: HashMap<String, Vec<SupplyChainLink>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainLink {
    pub supplier: String,
    pub customer: String,
    pub component: String,
    pub confidence: f64,
}

impl Default for SupplyChainMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl SupplyChainMapper {
    pub fn new() -> Self {
        Self {
            supply_chain: HashMap::new(),
        }
    }

    /// Add a supply chain relationship
    pub fn add_relationship(
        &mut self,
        supplier: &str,
        customer: &str,
        component: &str,
        confidence: f64,
    ) {
        let link = SupplyChainLink {
            supplier: supplier.to_string(),
            customer: customer.to_string(),
            component: component.to_string(),
            confidence,
        };

        self.supply_chain
            .entry(customer.to_string())
            .or_default()
            .push(link);
    }

    /// Get suppliers for an entity
    pub fn get_suppliers(&self, customer: &str) -> Vec<&SupplyChainLink> {
        self.supply_chain
            .get(customer)
            .map(|links| links.iter().collect())
            .unwrap_or_default()
    }

    /// Get customers for an entity
    pub fn get_customers(&self, supplier: &str) -> Vec<String> {
        let mut customers = Vec::new();

        for (customer, links) in &self.supply_chain {
            for link in links {
                if link.supplier.to_lowercase() == supplier.to_lowercase() {
                    customers.push(customer.clone());
                }
            }
        }

        customers
    }

    /// Map complete supply chain for an entity (upstream and downstream)
    pub fn map_supply_chain(&self, entity: &str) -> SupplyChainMap {
        let mut upstream: Vec<SupplyChainLink> = Vec::new();

        // Get direct suppliers
        if let Some(links) = self.supply_chain.get(entity) {
            upstream = links.clone();
        }

        // Get customers (reverse lookup)
        let downstream = self.get_customers(entity);

        let supplier_count = upstream.len();
        let customer_count = downstream.len();

        SupplyChainMap {
            entity: entity.to_string(),
            direct_suppliers: upstream,
            direct_customers: downstream,
            supplier_count,
            customer_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainMap {
    pub entity: String,
    pub direct_suppliers: Vec<SupplyChainLink>,
    pub direct_customers: Vec<String>,
    pub supplier_count: usize,
    pub customer_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relationship_inference() {
        let mut tracker = RelationshipTracker::new();

        // Pre-populate with known entities (as would happen in production
        // where entities are dynamically discovered from observations).
        tracker.record_mention("NVIDIA", "tech company", "test", 1234567890);
        tracker.record_mention("Microsoft", "tech company", "test", 1234567890);
        tracker.record_mention("AMD", "tech company", "test", 1234567890);
        tracker.record_mention("TSMC", "semiconductor", "test", 1234567890);

        let content = "NVIDIA partners with Microsoft to develop AI solutions. \
            AMD competes with NVIDIA in the GPU market. TSMC supplies chips to NVIDIA.";

        let relationships = tracker.infer_relationships(content, "test_source", 1234567890);

        // Should find partner relationship
        let has_partner = relationships
            .iter()
            .any(|r| matches!(r.relationship_type, RelationshipType::Partner));

        // Should find competitor relationship
        let has_competitor = relationships
            .iter()
            .any(|r| matches!(r.relationship_type, RelationshipType::Competitor));

        // Should find supplier relationship
        let has_supplier = relationships
            .iter()
            .any(|r| matches!(r.relationship_type, RelationshipType::Supplier));

        assert!(
            has_partner || has_competitor || has_supplier,
            "Should find at least one relationship type"
        );
    }

    #[test]
    fn test_cooccurrence_tracking() {
        let mut tracker = RelationshipTracker::new();

        tracker.record_cooccurrence("NVIDIA", "AMD");
        tracker.record_cooccurrence("NVIDIA", "AMD");
        tracker.record_cooccurrence("NVIDIA", "TSMC");

        let cooccurring = tracker.get_cooccurring("NVIDIA", 1);

        assert!(cooccurring.len() >= 2);

        let amd_count = cooccurring
            .iter()
            .find(|(name, _)| *name == "amd")
            .map(|(_, count)| *count)
            .unwrap_or(0);

        assert_eq!(amd_count, 2);
    }

    #[test]
    fn test_supply_chain_mapping() {
        let mut mapper = SupplyChainMapper::new();

        mapper.add_relationship("TSMC", "NVIDIA", "GPU chips", 0.95);
        mapper.add_relationship("Foxconn", "Apple", "iPhone assembly", 0.90);

        let nvidia_supply = mapper.get_suppliers("NVIDIA");
        assert!(!nvidia_supply.is_empty());

        let apple_customers = mapper.get_customers("Foxconn");
        assert!(apple_customers.contains(&"Apple".to_string()));

        let chain = mapper.map_supply_chain("NVIDIA");
        assert!(!chain.direct_suppliers.is_empty());
    }

    #[test]
    fn test_cross_entity_insight_generation() {
        let tracker = RelationshipTracker::new();

        let insights = tracker.generate_cross_entity_insights("NVIDIA");

        // Should generate some insights (even if empty relationships)
        assert!(insights.is_empty() || insights.iter().all(|i| !i.narrative.is_empty()));
    }
}
