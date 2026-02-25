//! LLM route — request/response types and validation for LLM-powered endpoints.
//!
//! Endpoints:
//! - `POST /api/llm/extract-entities` — extract named entities from freeform text
//! - `POST /api/llm/generate-recipe`  — generate a recipe hypothesis from a pattern description
//! - `POST /api/llm/synthesize-poi`   — synthesize a person-of-interest dossier from fragments
//! - `POST /api/llm/generate-memo`    — generate a strategic intelligence memo
//!
//! All endpoints require authentication (`min_role: analyst`).
//! When the `llm` feature is disabled, the handler layer should return 501.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Shared types
// ────────────────────────────────────────────

/// Maximum allowed input text length in characters (B350).
/// Prevents accidentally sending multi-megabyte payloads to the LLM.
pub const MAX_INPUT_TEXT_CHARS: usize = 50_000;

/// Maximum number of items in a batch request (B351).
pub const MAX_BATCH_SIZE: usize = 20;

/// LLM task identifier for routing decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LlmTask {
    EntityExtraction,
    RecipeHypothesis,
    PoiSynthesis,
    MemoGeneration,
}

impl LlmTask {
    pub fn as_str(&self) -> &str {
        match self {
            Self::EntityExtraction => "entity_extraction",
            Self::RecipeHypothesis => "recipe_hypothesis",
            Self::PoiSynthesis => "poi_synthesis",
            Self::MemoGeneration => "memo_generation",
        }
    }
}

// ────────────────────────────────────────────
// Entity extraction
// ────────────────────────────────────────────

/// Request body for entity extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractEntitiesRequest {
    /// Freeform text to extract entities from.
    pub text: String,
    /// Optional hint about the document type (e.g. "news_article", "patent", "filing").
    pub doc_type: Option<String>,
    /// Optional list of entity types to focus on (e.g. ["company", "person", "location"]).
    pub entity_types: Option<Vec<String>>,
}

impl ExtractEntitiesRequest {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.text.trim().is_empty() {
            issues.push("text must not be empty".to_string());
        }
        if self.text.len() > MAX_INPUT_TEXT_CHARS {
            issues.push(format!(
                "text length {} exceeds maximum {} characters",
                self.text.len(),
                MAX_INPUT_TEXT_CHARS
            ));
        }
        if let Some(ref types) = self.entity_types {
            if types.len() > 20 {
                issues.push("entity_types list too long (max 20)".to_string());
            }
        }
        issues
    }
}

/// A single extracted entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    pub entity_type: String,
    pub confidence: f64,
    /// Character offset in the original text where the entity was found.
    pub span_start: Option<usize>,
    pub span_end: Option<usize>,
    /// Normalized/canonical form of the entity name.
    pub canonical: Option<String>,
}

/// Response from entity extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractEntitiesResponse {
    pub entities: Vec<ExtractedEntity>,
    pub task: LlmTask,
    pub model_used: String,
    pub processing_ms: u64,
}

// ────────────────────────────────────────────
// Recipe hypothesis generation
// ────────────────────────────────────────────

/// Request body for recipe hypothesis generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateRecipeRequest {
    /// Natural language description of the pattern to generate a recipe for.
    pub pattern_description: String,
    /// Target outcome (e.g. "supplier_distress", "customs_delay").
    pub outcome: String,
    /// Known signals (e.g. ["late_filing", "layoff_announcement"]).
    pub signals: Vec<String>,
    /// Existing recipe IDs to avoid duplicates.
    #[serde(default)]
    pub existing_recipe_ids: Vec<String>,
    /// Geographic scope (e.g. ["TN", "MA", "IL"]).
    #[serde(default)]
    pub regions: Vec<String>,
}

impl GenerateRecipeRequest {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.pattern_description.trim().is_empty() {
            issues.push("pattern_description must not be empty".to_string());
        }
        if self.pattern_description.len() > MAX_INPUT_TEXT_CHARS {
            issues.push(format!(
                "pattern_description length {} exceeds maximum {}",
                self.pattern_description.len(),
                MAX_INPUT_TEXT_CHARS
            ));
        }
        if self.outcome.trim().is_empty() {
            issues.push("outcome must not be empty".to_string());
        }
        if self.signals.is_empty() {
            issues.push("signals must not be empty".to_string());
        }
        if self.signals.len() > 50 {
            issues.push("too many signals (max 50)".to_string());
        }
        issues
    }
}

/// Response from recipe hypothesis generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRecipeResponse {
    pub recipe_json: serde_json::Value,
    pub task: LlmTask,
    pub model_used: String,
    pub processing_ms: u64,
}

// ────────────────────────────────────────────
// POI synthesis
// ────────────────────────────────────────────

/// Request body for person-of-interest synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesizePoiRequest {
    /// Name of the person.
    pub person_name: String,
    /// Known titles/roles.
    #[serde(default)]
    pub known_titles: Vec<String>,
    /// Raw text fragments from different sources to synthesize.
    pub fragments: Vec<PoiFragment>,
}

/// A single text fragment from a source about a POI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiFragment {
    pub source_url: Option<String>,
    pub source_type: Option<String>,
    pub text: String,
    pub date: Option<String>,
}

impl SynthesizePoiRequest {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.person_name.trim().is_empty() {
            issues.push("person_name must not be empty".to_string());
        }
        if self.fragments.is_empty() {
            issues.push("fragments must not be empty".to_string());
        }
        if self.fragments.len() > MAX_BATCH_SIZE {
            issues.push(format!(
                "too many fragments ({}, max {})",
                self.fragments.len(),
                MAX_BATCH_SIZE
            ));
        }
        let total_chars: usize = self.fragments.iter().map(|f| f.text.len()).sum();
        if total_chars > MAX_INPUT_TEXT_CHARS {
            issues.push(format!(
                "total fragment text length {} exceeds maximum {}",
                total_chars, MAX_INPUT_TEXT_CHARS
            ));
        }
        issues
    }
}

/// Synthesized POI profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizePoiResponse {
    pub person_name: String,
    pub summary: String,
    pub roles: Vec<String>,
    pub affiliations: Vec<String>,
    pub key_facts: Vec<String>,
    pub risk_indicators: Vec<String>,
    pub source_count: usize,
    pub task: LlmTask,
    pub model_used: String,
    pub processing_ms: u64,
}

// ────────────────────────────────────────────
// Memo generation
// ────────────────────────────────────────────

/// Request body for strategic memo generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateMemoRequest {
    /// Topic or focus area for the memo.
    pub topic: String,
    /// Supporting context/evidence to incorporate.
    #[serde(default)]
    pub context_items: Vec<MemoContextItem>,
    /// Target audience (e.g. "executive", "analyst", "procurement").
    pub audience: Option<String>,
    /// Maximum desired word count for the memo.
    pub max_words: Option<u32>,
}

/// A piece of context to include in the memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoContextItem {
    pub title: String,
    pub content: String,
    pub source: Option<String>,
    pub date: Option<String>,
}

impl GenerateMemoRequest {
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.topic.trim().is_empty() {
            issues.push("topic must not be empty".to_string());
        }
        if self.context_items.len() > MAX_BATCH_SIZE {
            issues.push(format!(
                "too many context_items ({}, max {})",
                self.context_items.len(),
                MAX_BATCH_SIZE
            ));
        }
        let total_chars: usize = self
            .context_items
            .iter()
            .map(|c| c.content.len() + c.title.len())
            .sum();
        if total_chars > MAX_INPUT_TEXT_CHARS {
            issues.push(format!(
                "total context text length {} exceeds maximum {}",
                total_chars, MAX_INPUT_TEXT_CHARS
            ));
        }
        if let Some(max) = self.max_words {
            if max == 0 || max > 10_000 {
                issues.push(format!("max_words must be 1–10000, got {}", max));
            }
        }
        issues
    }
}

/// Generated strategic memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateMemoResponse {
    pub title: String,
    pub executive_summary: String,
    pub sections: Vec<MemoSection>,
    pub recommendations: Vec<String>,
    pub task: LlmTask,
    pub model_used: String,
    pub processing_ms: u64,
}

/// A section within a generated memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoSection {
    pub heading: String,
    pub content: String,
}

// ────────────────────────────────────────────
// Path constants
// ────────────────────────────────────────────

pub mod paths {
    pub const LLM_PREFIX: &str = "/api/llm";
    pub const EXTRACT_ENTITIES: &str = "/api/llm/extract-entities";
    pub const GENERATE_RECIPE: &str = "/api/llm/generate-recipe";
    pub const SYNTHESIZE_POI: &str = "/api/llm/synthesize-poi";
    pub const GENERATE_MEMO: &str = "/api/llm/generate-memo";
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_entities_validation_empty_text() {
        let req = ExtractEntitiesRequest {
            text: "".to_string(),
            doc_type: None,
            entity_types: None,
        };
        let issues = req.validate();
        assert!(!issues.is_empty());
        assert!(issues[0].contains("empty"));
    }

    #[test]
    fn test_extract_entities_validation_too_long() {
        let req = ExtractEntitiesRequest {
            text: "x".repeat(MAX_INPUT_TEXT_CHARS + 1),
            doc_type: None,
            entity_types: None,
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("exceeds")));
    }

    #[test]
    fn test_extract_entities_validation_ok() {
        let req = ExtractEntitiesRequest {
            text: "Starz Electronics announced a new factory in Tangier.".to_string(),
            doc_type: Some("news_article".to_string()),
            entity_types: Some(vec!["company".to_string(), "location".to_string()]),
        };
        assert!(req.validate().is_empty());
    }

    #[test]
    fn test_generate_recipe_validation_empty_outcome() {
        let req = GenerateRecipeRequest {
            pattern_description: "Supplier shows distress signals".to_string(),
            outcome: "".to_string(),
            signals: vec!["late_filing".to_string()],
            existing_recipe_ids: vec![],
            regions: vec![],
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("outcome")));
    }

    #[test]
    fn test_generate_recipe_validation_no_signals() {
        let req = GenerateRecipeRequest {
            pattern_description: "Supplier shows distress signals".to_string(),
            outcome: "supplier_distress".to_string(),
            signals: vec![],
            existing_recipe_ids: vec![],
            regions: vec![],
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("signals")));
    }

    #[test]
    fn test_generate_recipe_validation_ok() {
        let req = GenerateRecipeRequest {
            pattern_description: "Supplier shows distress signals".to_string(),
            outcome: "supplier_distress".to_string(),
            signals: vec!["late_filing".to_string(), "layoff_announcement".to_string()],
            existing_recipe_ids: vec!["recipe_01".to_string()],
            regions: vec!["TN".to_string()],
        };
        assert!(req.validate().is_empty());
    }

    #[test]
    fn test_synthesize_poi_validation_empty_fragments() {
        let req = SynthesizePoiRequest {
            person_name: "John Doe".to_string(),
            known_titles: vec![],
            fragments: vec![],
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("fragments")));
    }

    #[test]
    fn test_synthesize_poi_validation_ok() {
        let req = SynthesizePoiRequest {
            person_name: "John Doe".to_string(),
            known_titles: vec!["VP Supply Chain".to_string()],
            fragments: vec![PoiFragment {
                source_url: Some("https://example.com".to_string()),
                source_type: Some("linkedin".to_string()),
                text: "John Doe is VP of Supply Chain at Acme Corp.".to_string(),
                date: Some("2024-01-15".to_string()),
            }],
        };
        assert!(req.validate().is_empty());
    }

    #[test]
    fn test_generate_memo_validation_empty_topic() {
        let req = GenerateMemoRequest {
            topic: "".to_string(),
            context_items: vec![],
            audience: None,
            max_words: None,
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("topic")));
    }

    #[test]
    fn test_generate_memo_validation_ok() {
        let req = GenerateMemoRequest {
            topic: "Morocco EMS market Q3 outlook".to_string(),
            context_items: vec![MemoContextItem {
                title: "GDP growth".to_string(),
                content: "Morocco GDP grew 3.2% in H1 2024.".to_string(),
                source: Some("World Bank".to_string()),
                date: Some("2024-07-01".to_string()),
            }],
            audience: Some("executive".to_string()),
            max_words: Some(500),
        };
        assert!(req.validate().is_empty());
    }

    #[test]
    fn test_generate_memo_validation_bad_max_words() {
        let req = GenerateMemoRequest {
            topic: "Test".to_string(),
            context_items: vec![],
            audience: None,
            max_words: Some(0),
        };
        let issues = req.validate();
        assert!(issues.iter().any(|i| i.contains("max_words")));
    }

    #[test]
    fn test_llm_task_as_str() {
        assert_eq!(LlmTask::EntityExtraction.as_str(), "entity_extraction");
        assert_eq!(LlmTask::RecipeHypothesis.as_str(), "recipe_hypothesis");
        assert_eq!(LlmTask::PoiSynthesis.as_str(), "poi_synthesis");
        assert_eq!(LlmTask::MemoGeneration.as_str(), "memo_generation");
    }

    #[test]
    fn test_path_constants() {
        assert!(paths::EXTRACT_ENTITIES.starts_with("/api/llm"));
        assert!(paths::GENERATE_RECIPE.starts_with("/api/llm"));
        assert!(paths::SYNTHESIZE_POI.starts_with("/api/llm"));
        assert!(paths::GENERATE_MEMO.starts_with("/api/llm"));
    }
}
