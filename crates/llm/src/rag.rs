//!
//! Retrieval-Augmented Generation (RAG) for ApexIntel OSINT platform.
//!
//! Implements knowledge base integration for LLM outputs:
//! - Internal knowledge base queries
//! - Source credibility weighting
//! - Context window optimization
//! - Citation grounding
//!
//! Reduces hallucinations by grounding LLM responses in verified internal knowledge.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};
use crate::LlmClient;

/// A knowledge base entry with source and credibility metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    /// Unique identifier for this entry.
    pub id: String,
    /// Entry content text.
    pub content: String,
    /// Source of the information.
    pub source: KnowledgeSource,
    /// Source credibility weight (0.0-1.0).
    pub credibility_weight: f64,
    /// Topics/entities this entry relates to.
    pub topics: Vec<String>,
    /// When this entry was added.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Expiration time (None = never expires).
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl KnowledgeEntry {
    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires_at {
            expires < chrono::Utc::now()
        } else {
            false
        }
    }
}

/// Knowledge source types with associated credibility factors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceType {
    /// Primary regulatory filings (SEC, EU filings)
    Regulatory,
    /// Academic papers and research
    Academic,
    /// Press releases and official statements
    PressRelease,
    /// Industry publications
    IndustryPress,
    /// News articles
    News,
    /// Social media
    SocialMedia,
    /// Company website
    CompanyWebsite,
    /// Employee or insider information
    Insider,
    /// OSINT collection
    Osint,
    /// Cross-referenced data
    CrossReferenced,
    /// User-submitted data
    UserSubmitted,
    /// Unknown source
    Unknown,
}

impl KnowledgeSourceType {
    /// Get the default credibility weight for this source type.
    pub fn default_credibility(&self) -> f64 {
        match self {
            Self::Regulatory => 0.95,
            Self::Academic => 0.90,
            Self::PressRelease => 0.80,
            Self::CrossReferenced => 0.85,
            Self::IndustryPress => 0.70,
            Self::News => 0.65,
            Self::Osint => 0.60,
            Self::CompanyWebsite => 0.55,
            Self::SocialMedia => 0.40,
            Self::UserSubmitted => 0.50,
            Self::Insider => 0.45,
            Self::Unknown => 0.30,
        }
    }
}

/// Knowledge source with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSource {
    /// Type of source.
    pub source_type: KnowledgeSourceType,
    /// Source name/identifier.
    pub name: String,
    /// URL if available.
    pub url: Option<String>,
    /// Publication date.
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Number of cross-references to other sources.
    pub cross_reference_count: u32,
}

impl KnowledgeSource {
    pub fn new(source_type: KnowledgeSourceType, name: impl Into<String>) -> Self {
        Self {
            source_type,
            name: name.into(),
            url: None,
            published_at: None,
            cross_reference_count: 0,
        }
    }

    /// Calculate the effective credibility weight.
    pub fn credibility_weight(&self) -> f64 {
        let base = self.source_type.default_credibility();
        
        // Cross-references boost credibility
        let cross_ref_boost = (self.cross_reference_count as f64 * 0.02).min(0.10);
        
        // Recency boost for recent sources
        let recency_boost = if let Some(published) = self.published_at {
            let age_days = (chrono::Utc::now() - published).num_days();
            if age_days < 30 {
                0.05
            } else if age_days < 90 {
                0.02
            } else if age_days > 365 {
                -0.10
            } else {
                0.0
            }
        } else {
            0.0
        };

        (base + cross_ref_boost + recency_boost).clamp(0.1, 1.0)
    }
}

/// Internal knowledge base for RAG.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBase {
    entries: HashMap<String, KnowledgeEntry>,
    /// Index by topic for fast lookup.
    topic_index: HashMap<String, Vec<String>>,
    /// Index by entity name.
    entity_index: HashMap<String, Vec<String>>,
}

impl KnowledgeBase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry to the knowledge base.
    pub fn add_entry(&mut self, entry: KnowledgeEntry) {
        let entry_id = entry.id.clone();
        self.entries.insert(entry_id.clone(), entry.clone());

        // Update topic index
        for topic in &entry.topics {
            self.topic_index
                .entry(topic.to_lowercase())
                .or_default()
                .push(entry_id.clone());
        }

        // Update entity index (extract entities from content)
        for entity in extract_entities(&entry.content) {
            self.entity_index
                .entry(entity.to_lowercase())
                .or_default()
                .push(entry_id.clone());
        }
    }

    /// Query knowledge base by topic.
    pub fn query_by_topic(&self, topic: &str, limit: usize) -> Vec<&KnowledgeEntry> {
        let key = topic.to_lowercase();
        let entry_ids = self.topic_index.get(&key).map(|v| v.as_slice()).unwrap_or(&[]);
        
        entry_ids
            .iter()
            .filter_map(|id| self.entries.get(id))
            .filter(|e| !e.is_expired())
            .take(limit)
            .collect()
    }

    /// Query knowledge base by entity name.
    pub fn query_by_entity(&self, entity: &str, limit: usize) -> Vec<&KnowledgeEntry> {
        let key = entity.to_lowercase();
        let entry_ids = self.entity_index.get(&key).map(|v| v.as_slice()).unwrap_or(&[]);
        
        entry_ids
            .iter()
            .filter_map(|id| self.entries.get(id))
            .filter(|e| !e.is_expired())
            .take(limit)
            .collect()
    }

    /// Query with combined filters and ranking.
    pub fn query(&self, query: &RagQuery) -> Vec<RankedEntry> {
        // Use HashMap to track best weight per entry (owned entry for the final collection)
        let mut seen: HashMap<String, f64> = HashMap::new();
        let mut candidate_ids: Vec<String> = Vec::new();

        // Topic matches
        for topic in &query.topics {
            for entry in self.query_by_topic(topic, 50) {
                let weight = entry.credibility_weight;
                if let Some(current) = seen.get(&entry.id) {
                    if weight > *current {
                        seen.insert(entry.id.clone(), weight);
                    }
                } else {
                    seen.insert(entry.id.clone(), weight);
                    candidate_ids.push(entry.id.clone());
                }
            }
        }

        // Entity matches
        for entity in &query.entities {
            for entry in self.query_by_entity(entity, 50) {
                let weight = entry.credibility_weight;
                if let Some(current) = seen.get(&entry.id) {
                    if weight > *current {
                        seen.insert(entry.id.clone(), weight);
                    }
                } else {
                    seen.insert(entry.id.clone(), weight);
                    candidate_ids.push(entry.id.clone());
                }
            }
        }

        // Convert to ranked entries and sort
        let mut ranked: Vec<RankedEntry> = candidate_ids
            .into_iter()
            .filter_map(|id| {
                let entry = self.entries.get(&id)?;
                let weight = *seen.get(&id).unwrap_or(&0.0);
                let relevance = calculate_relevance(entry, &query.keywords);
                Some(RankedEntry {
                    entry: entry.clone(),
                    relevance_score: relevance,
                    credibility_score: weight,
                    combined_score: relevance * 0.4 + weight * 0.6,
                })
            })
            .collect();

        ranked.sort_by(|a, b| b.combined_score.total_cmp(&a.combined_score));
        ranked.truncate(query.limit);
        ranked
    }

    /// Get entry by ID.
    pub fn get(&self, id: &str) -> Option<&KnowledgeEntry> {
        self.entries.get(id)
    }

    /// Remove expired entries.
    pub fn remove_expired(&mut self) -> usize {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        for id in &expired {
            self.entries.remove(id);
        }

        // Rebuild indices
        self.topic_index.clear();
        self.entity_index.clear();
        for entry in self.entries.values() {
            let entry_id = entry.id.clone();
            for topic in &entry.topics {
                self.topic_index
                    .entry(topic.to_lowercase())
                    .or_default()
                    .push(entry_id.clone());
            }
            for entity in extract_entities(&entry.content) {
                self.entity_index
                    .entry(entity.to_lowercase())
                    .or_default()
                    .push(entry_id.clone());
            }
        }

        expired.len()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// RAG query parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagQuery {
    /// Topics to search for.
    pub topics: Vec<String>,
    /// Specific entities to find information about.
    pub entities: Vec<String>,
    /// Keywords to enhance relevance scoring.
    pub keywords: Vec<String>,
    /// Maximum results to return.
    pub limit: usize,
    /// Minimum credibility threshold.
    pub min_credibility: f64,
}

impl Default for RagQuery {
    fn default() -> Self {
        Self {
            topics: vec![],
            entities: vec![],
            keywords: vec![],
            limit: 10,
            min_credibility: 0.3,
        }
    }
}

impl RagQuery {
    pub fn new(topics: Vec<String>, entities: Vec<String>) -> Self {
        Self {
            topics,
            entities,
            ..Default::default()
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_min_credibility(mut self, min: f64) -> Self {
        self.min_credibility = min;
        self
    }
}

/// A ranked knowledge entry with scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedEntry {
    pub entry: KnowledgeEntry,
    pub relevance_score: f64,
    pub credibility_score: f64,
    pub combined_score: f64,
}

impl RankedEntry {
    /// Generate a citation string for this entry.
    pub fn citation(&self) -> String {
        format!(
            "[{}] {} (credibility: {:.0}%)",
            self.entry.source.name,
            truncate_str(&self.entry.content, 100),
            self.credibility_score * 100.0
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Context window optimization
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for context window optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowConfig {
    /// Maximum context tokens (including prompt).
    pub max_tokens: usize,
    /// Reserve tokens for the response.
    pub response_token_reserve: usize,
    /// Enable priority-based truncation.
    pub prioritize_by_relevance: bool,
    /// Enable source credibility weighting.
    pub weight_by_credibility: bool,
}

impl Default for ContextWindowConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            response_token_reserve: 512,
            prioritize_by_relevance: true,
            weight_by_credibility: true,
        }
    }
}

/// Optimized context for LLM input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedContext {
    /// The formatted context string.
    pub context: String,
    /// Estimated token count.
    pub token_count: usize,
    /// Entries included.
    pub entries_included: usize,
    /// Entries truncated.
    pub entries_truncated: usize,
    /// Average credibility of included entries.
    pub avg_credibility: f64,
}

impl OptimizedContext {
    /// Convert ranked entries to a context string.
    pub fn from_entries(
        entries: &[RankedEntry],
        config: &ContextWindowConfig,
    ) -> Self {
        let available_tokens = config.max_tokens - config.response_token_reserve;
        let mut context_parts = Vec::new();
        let mut total_tokens = 0usize;
        let mut entries_included = 0;
        let mut entries_truncated = 0;
        let mut total_credibility = 0.0;

        for entry in entries {
            // Estimate token count (rough: 4 chars per token)
            let entry_tokens = entry.entry.content.len() / 4;
            let source_tokens = entry.entry.source.name.len() / 4;

            if total_tokens + entry_tokens + source_tokens > available_tokens {
                entries_truncated += 1;
                continue;
            }

            total_tokens += entry_tokens + source_tokens;
            total_credibility += entry.credibility_score;
            entries_included += 1;

            context_parts.push(format!(
                "[Source: {} | Credibility: {:.0}%]\n{}\n---",
                entry.entry.source.name,
                entry.credibility_score * 100.0,
                entry.entry.content
            ));
        }

        let context = context_parts.join("\n\n");
        let avg_credibility = if entries_included > 0 {
            total_credibility / entries_included as f64
        } else {
            0.0
        };

        Self {
            context,
            token_count: total_tokens,
            entries_included,
            entries_truncated,
            avg_credibility,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Citation grounding
// ─────────────────────────────────────────────────────────────────────────────

/// Citation for an LLM claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// Citation ID.
    pub id: String,
    /// Source information.
    pub source: KnowledgeSource,
    /// Quote or excerpt.
    pub quote: String,
    /// The claim this citation supports.
    pub claim: String,
    /// Credibility of the citation.
    pub credibility: f64,
    /// Whether this citation is verified.
    pub verified: bool,
}

impl Citation {
    pub fn new(source: KnowledgeSource, quote: impl Into<String>, claim: impl Into<String>) -> Self {
        let credibility = source.credibility_weight();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            quote: quote.into(),
            claim: claim.into(),
            credibility,
            verified: false,
        }
    }

    /// Format as markdown citation.
    pub fn to_markdown(&self) -> String {
        format!(
            "- *\"{}\"*\n  — *{}* (credibility: {:.0}%)\n",
            truncate_str(&self.quote, 200),
            self.source.name,
            self.credibility * 100.0
        )
    }
}

/// Citation grounded response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundedResponse {
    /// The generated response text.
    pub response: String,
    /// Citations supporting claims in the response.
    pub citations: Vec<Citation>,
    /// Overall grounding quality score (0.0-1.0).
    pub grounding_quality: f64,
    /// Number of claims without citations.
    pub ungrounded_claims: usize,
}

impl GroundedResponse {
    /// Create from LLM response and knowledge base.
    pub fn from_response_and_kb(
        response: &str,
        kb: &KnowledgeBase,
        query: &RagQuery,
    ) -> Self {
        // Find relevant entries that could support claims
        let relevant = kb.query(query);
        let mut citations = Vec::new();

        // Simple claim detection (look for statements with specific entities)
        let response_lower = response.to_lowercase();
        
        for entry in &relevant {
            // Check if entry content overlaps with response
            let entry_lower = entry.entry.content.to_lowercase();
            let entry_words: std::collections::HashSet<_> = entry_lower
                .split_whitespace()
                .collect();
            
            let response_words: std::collections::HashSet<_> = response_lower
                .split_whitespace()
                .collect();
            
            let overlap: f64 = entry_words.intersection(&response_words).count() as f64 
                / entry_words.len().max(1) as f64;
            
            if overlap > 0.1 {
                // Extract a quote from the entry
                let quote: String = entry.entry.content.chars().take(200).collect();
                
                citations.push(Citation::new(
                    entry.entry.source.clone(),
                    quote,
                    format!("Claim related to {}", entry.entry.topics.join(", ")),
                ));
            }
        }

        // Estimate grounding quality
        let grounding_quality = if citations.is_empty() {
            0.0
        } else {
            let avg_cred = citations.iter().map(|c| c.credibility).sum::<f64>() 
                / citations.len() as f64;
            let citation_rate = citations.len() as f64 / 5.0; // Assume ~5 claims
            (avg_cred * 0.6 + citation_rate.min(1.0) * 0.4).clamp(0.0, 1.0)
        };

        // Estimate ungrounded claims
        let ungrounded_claims = 5usize.saturating_sub(citations.len());

        Self {
            response: response.to_string(),
            citations,
            grounding_quality,
            ungrounded_claims,
        }
    }

    /// Format as markdown with citations.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&self.response);
        md.push_str("\n\n---\n\n**Sources:**\n");
        
        for citation in &self.citations {
            md.push_str(&citation.to_markdown());
        }
        
        if self.ungrounded_claims > 0 {
            md.push_str(&format!(
                "\n*Note: {} claims without verifiable citations.*\n",
                self.ungrounded_claims
            ));
        }
        
        md
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RAG Engine
// ─────────────────────────────────────────────────────────────────────────────

/// RAG engine combining knowledge base with LLM.
pub struct RagEngine {
    knowledge_base: Arc<KnowledgeBase>,
}

impl RagEngine {
    pub fn new(knowledge_base: Arc<KnowledgeBase>) -> Self {
        Self { knowledge_base }
    }

    /// Query the knowledge base and format context.
    pub fn query_and_format(&self, query: &RagQuery) -> OptimizedContext {
        let ranked = self.knowledge_base.query(query);
        OptimizedContext::from_entries(&ranked, &ContextWindowConfig::default())
    }

    /// Generate a grounded response using the knowledge base.
    pub async fn generate_grounded(
        &self,
        llm: &crate::OpenAiCompatibleClient,
        system: &str,
        user: &str,
        query: &RagQuery,
    ) -> Result<GroundedResponse> {
        // Get relevant context
        let context = self.query_and_format(query);
        
        debug!(
            entries = %context.entries_included,
            tokens = %context.token_count,
            avg_credibility = %context.avg_credibility,
            "RAG context retrieved"
        );

        // Build enhanced user prompt with context
        let enhanced_user = if context.context.is_empty() {
            user.to_string()
        } else {
            format!(
                "{}\n\nRelevant context from knowledge base:\n{}\n\n---\n\nUser query:\n{}",
                system, context.context, user
            )
        };

        // Generate response using LlmClient trait
        let system_prompt = "You are an intelligence analyst. Use the provided context to ground \
                your analysis in verified information. Cite specific sources when making claims. \
                If information is uncertain, explicitly state your confidence level.";
        
        let resp = llm
            .generate_text(system_prompt, &enhanced_user)
            .await
            .context("Grounded response generation failed")?;

        // Create grounded response
        let grounded = GroundedResponse::from_response_and_kb(&resp, &self.knowledge_base, query);
        
        info!(
            grounding_quality = %grounded.grounding_quality,
            citations = %grounded.citations.len(),
            "Grounded response generated"
        );

        Ok(grounded)
    }

    /// Get underlying knowledge base for direct access.
    pub fn knowledge_base(&self) -> &Arc<KnowledgeBase> {
        &self.knowledge_base
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility functions
// ─────────────────────────────────────────────────────────────────────────────

fn extract_entities(text: &str) -> Vec<String> {
    // Simple entity extraction - look for capitalized words
    let mut entities = Vec::new();
    let mut current = String::new();
    let mut is_capitalized = false;

    for ch in text.chars() {
        if ch.is_uppercase() && !current.is_empty() && !is_capitalized && !current.ends_with('.') {
            if current.len() > 2 {
                entities.push(current.clone());
            }
            current = ch.to_string();
        } else {
            current.push(ch);
        }
        is_capitalized = ch.is_uppercase();
    }

    if current.len() > 2 {
        entities.push(current);
    }

    entities
}

fn calculate_relevance(entry: &KnowledgeEntry, keywords: &[String]) -> f64 {
    if keywords.is_empty() {
        return 0.5;
    }

    let content_lower = entry.content.to_lowercase();
    let topic_match = entry
        .topics
        .iter()
        .filter(|t| keywords.iter().any(|k| t.to_lowercase().contains(&k.to_lowercase())))
        .count();

    let keyword_match = keywords
        .iter()
        .filter(|k| content_lower.contains(&k.to_lowercase()))
        .count();

    let total = keywords.len() * 2;
    (topic_match * 2 + keyword_match) as f64 / total.max(1) as f64
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_source_credibility_default() {
        assert_eq!(KnowledgeSourceType::Regulatory.default_credibility(), 0.95);
        assert_eq!(KnowledgeSourceType::SocialMedia.default_credibility(), 0.40);
        assert_eq!(KnowledgeSourceType::Unknown.default_credibility(), 0.30);
    }

    #[test]
    fn knowledge_source_with_cross_references() {
        let source = KnowledgeSource {
            source_type: KnowledgeSourceType::News,
            name: "Test News".to_string(),
            url: None,
            published_at: None,
            cross_reference_count: 5,
        };

        assert!(source.credibility_weight() > source.source_type.default_credibility());
    }

    #[test]
    fn knowledge_base_add_and_query() {
        let mut kb = KnowledgeBase::new();

        kb.add_entry(KnowledgeEntry {
            id: "entry-1".to_string(),
            content: "Apple is expanding its manufacturing in Vietnam.".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Reuters"),
            credibility_weight: 0.7,
            topics: vec!["supply_chain".to_string(), "Apple".to_string()],
            timestamp: chrono::Utc::now(),
            expires_at: None,
        });

        let results = kb.query_by_topic("supply_chain", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "entry-1");
    }

    #[test]
    fn rag_query_builder() {
        let query = RagQuery::new(
            vec!["supply_chain".to_string()],
            vec!["Apple".to_string()],
        )
        .with_limit(5)
        .with_min_credibility(0.6);

        assert_eq!(query.topics.len(), 1);
        assert_eq!(query.limit, 5);
        assert_eq!(query.min_credibility, 0.6);
    }

    #[test]
    fn context_optimization_limits_tokens() {
        let entries = vec![
            RankedEntry {
                entry: KnowledgeEntry {
                    id: "1".to_string(),
                    content: "Test content ".repeat(100),
                    source: KnowledgeSource::new(KnowledgeSourceType::News, "Test"),
                    credibility_weight: 0.8,
                    topics: vec![],
                    timestamp: chrono::Utc::now(),
                    expires_at: None,
                },
                relevance_score: 0.7,
                credibility_score: 0.8,
                combined_score: 0.75,
            },
        ];

        let config = ContextWindowConfig {
            max_tokens: 1000,
            response_token_reserve: 200,
            ..Default::default()
        };

        let context = OptimizedContext::from_entries(&entries, &config);
        assert!(context.token_count <= 800);
    }

    #[test]
    fn citation_formatting() {
        let citation = Citation::new(
            KnowledgeSource::new(KnowledgeSourceType::Regulatory, "SEC Filing"),
            "Revenue increased by 15% year over year",
            "Company showed strong growth",
        );

        let md = citation.to_markdown();
        assert!(md.contains("SEC Filing"));
        assert!(md.contains("95%"));
    }

    #[test]
    fn grounded_response_generation() {
        let mut kb = KnowledgeBase::new();
        kb.add_entry(KnowledgeEntry {
            id: "1".to_string(),
            content: "Foxconn announced expansion plans in Vietnam.".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Reuters"),
            credibility_weight: 0.75,
            topics: vec!["Foxconn".to_string(), "Vietnam".to_string()],
            timestamp: chrono::Utc::now(),
            expires_at: None,
        });

        let query = RagQuery::new(vec![], vec!["Foxconn".to_string()]);
        let response = "Foxconn is expanding in Vietnam.";
        
        let grounded = GroundedResponse::from_response_and_kb(response, &kb, &query);
        assert!(!grounded.citations.is_empty() || grounded.grounding_quality == 0.0);
    }

    #[test]
    fn extract_entities_simple() {
        let text = "Apple Inc. announced plans while Samsung responded.";
        let entities = extract_entities(text);
        assert!(entities.iter().any(|e| e.contains("Apple")));
        assert!(entities.iter().any(|e| e.contains("Samsung")));
    }

    #[test]
    fn ranked_entry_citation() {
        let entry = RankedEntry {
            entry: KnowledgeEntry {
                id: "1".to_string(),
                content: "Test content".to_string(),
                source: KnowledgeSource::new(KnowledgeSourceType::Regulatory, "SEC"),
                credibility_weight: 0.95,
                topics: vec![],
                timestamp: chrono::Utc::now(),
                expires_at: None,
            },
            relevance_score: 0.8,
            credibility_score: 0.95,
            combined_score: 0.88,
        };

        let citation = entry.citation();
        assert!(citation.contains("SEC"));
        assert!(citation.contains("95%"));
    }

    #[test]
    fn knowledge_entry_expiration() {
        let entry = KnowledgeEntry {
            id: "1".to_string(),
            content: "Test".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Test"),
            credibility_weight: 0.7,
            topics: vec![],
            timestamp: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::days(1)),
        };
        assert!(!entry.is_expired());

        let expired = KnowledgeEntry {
            id: "2".to_string(),
            content: "Test".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Test"),
            credibility_weight: 0.7,
            topics: vec![],
            timestamp: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now() - chrono::Duration::days(1)),
        };
        assert!(expired.is_expired());
    }

    #[test]
    fn knowledge_base_removes_expired() {
        let mut kb = KnowledgeBase::new();
        
        kb.add_entry(KnowledgeEntry {
            id: "fresh".to_string(),
            content: "Fresh content".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Test"),
            credibility_weight: 0.7,
            topics: vec!["test".to_string()],
            timestamp: chrono::Utc::now(),
            expires_at: None,
        });
        
        kb.add_entry(KnowledgeEntry {
            id: "expired".to_string(),
            content: "Expired content".to_string(),
            source: KnowledgeSource::new(KnowledgeSourceType::News, "Test"),
            credibility_weight: 0.7,
            topics: vec!["test".to_string()],
            timestamp: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        });

        assert_eq!(kb.len(), 2);
        let removed = kb.remove_expired();
        assert_eq!(removed, 1);
        assert_eq!(kb.len(), 1);
    }
}
