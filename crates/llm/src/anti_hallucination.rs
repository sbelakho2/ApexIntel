//! Anti-hallucination system for LLM enrichment.
//!
//! Ensures every LLM-generated field is traceable to an input source,
//! preventing fabricated job titles, company affiliations, and other
//! claims that are not supported by evidence.
//!
//! # Architecture
//!
//! 1. **SourceGroundingValidator** — validates generated fields against input sources
//! 2. **FactExtractionMode** — system prompt directive for extraction-only mode
//! 3. **FieldConfidenceScorer** — per-field confidence with minimum thresholds
//! 4. **CrossReferenceValidator** — checks generated facts against known dictionaries
//! 5. **OutputSanitizer** — strips unsourced/unverifiable claims

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ─── Source Grounding ───────────────────────────────────────────────────────

/// How a generated field was matched to a source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MatchMethod {
    /// Exact string match in source text.
    Exact,
    /// Fuzzy/token-level match in source text.
    Fuzzy {
        similarity: f64,
        matched_tokens: usize,
    },
    /// Matched against a known entity dictionary.
    Dictionary { dict_name: String },
    /// Matched against structured source field (e.g., page title, JSON-LD).
    Structured { field_name: String },
    /// Pre-extracted by parser before LLM enrichment.
    PreExtracted { extractor: String },
    /// Could not be traced to any source (hallucinated).
    Unsourced,
}

/// A single source grounding result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceGrounding {
    pub field_name: String,
    pub generated_value: String,
    pub match_method: MatchMethod,
    pub source_text: Option<String>,
    pub source_url: Option<String>,
}

/// Validates that LLM-generated fields are grounded in input sources.
pub struct SourceGroundingValidator {
    /// Input source texts to validate against.
    source_texts: Vec<String>,
    /// Source URLs corresponding to each text.
    source_urls: Vec<String>,
    /// Known dictionaries for cross-reference.
    dictionaries: HashMap<String, HashSet<String>>,
    /// Pre-extracted structured fields.
    pre_extracted: HashMap<String, Vec<String>>,
}

impl Default for SourceGroundingValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceGroundingValidator {
    pub fn new() -> Self {
        Self {
            source_texts: Vec::new(),
            source_urls: Vec::new(),
            dictionaries: HashMap::new(),
            pre_extracted: HashMap::new(),
        }
    }

    /// Add a source text for validation.
    pub fn add_source(&mut self, text: &str, url: Option<&str>) {
        self.source_texts.push(text.to_string());
        self.source_urls.push(url.unwrap_or("unknown").to_string());
    }

    /// Add a dictionary for cross-reference validation.
    pub fn add_dictionary(&mut self, name: &str, entries: HashSet<String>) {
        self.dictionaries.insert(name.to_string(), entries);
    }

    /// Add pre-extracted fields (from parser, NER, etc.).
    pub fn add_pre_extracted(&mut self, field: &str, values: Vec<String>) {
        self.pre_extracted
            .entry(field.to_string())
            .or_default()
            .extend(values);
    }

    /// Validate a generated field against sources.
    /// Returns the best match method or Unsourced if no match found.
    pub fn validate_field(&self, field_name: &str, value: &str) -> SourceGrounding {
        if value.trim().is_empty() {
            return SourceGrounding {
                field_name: field_name.to_string(),
                generated_value: value.to_string(),
                match_method: MatchMethod::Unsourced,
                source_text: None,
                source_url: None,
            };
        }

        // 1. Check pre-extracted fields first (highest confidence)
        if let Some(pre_values) = self.pre_extracted.get(field_name) {
            for pre_val in pre_values {
                if pre_val.eq_ignore_ascii_case(value) {
                    return SourceGrounding {
                        field_name: field_name.to_string(),
                        generated_value: value.to_string(),
                        match_method: MatchMethod::PreExtracted {
                            extractor: "parser".to_string(),
                        },
                        source_text: Some(pre_val.clone()),
                        source_url: None,
                    };
                }
            }
        }

        // 2. Check exact match in source texts
        let value_lower = value.to_lowercase();
        for (i, text) in self.source_texts.iter().enumerate() {
            let text_lower = text.to_lowercase();
            if text_lower.contains(&value_lower) {
                // Find the surrounding context
                let idx = text_lower.find(&value_lower).unwrap_or(0);
                let start = idx.saturating_sub(50);
                let end = (idx + value.len() + 50).min(text.len());
                let context = &text[start..end];

                return SourceGrounding {
                    field_name: field_name.to_string(),
                    generated_value: value.to_string(),
                    match_method: MatchMethod::Exact,
                    source_text: Some(context.to_string()),
                    source_url: self.source_urls.get(i).cloned(),
                };
            }
        }

        // 3. Check fuzzy match (token-level)
        let value_tokens: HashSet<&str> = value_lower
            .split_whitespace()
            .filter(|t| t.len() > 2)
            .collect();
        if !value_tokens.is_empty() {
            for (i, text) in self.source_texts.iter().enumerate() {
                let text_lower = text.to_lowercase();
                let text_tokens: HashSet<&str> = text_lower
                    .split_whitespace()
                    .filter(|t| t.len() > 2)
                    .collect();

                let matches: HashSet<_> = value_tokens.intersection(&text_tokens).collect();
                let similarity = matches.len() as f64 / value_tokens.len() as f64;

                if similarity >= 0.5 {
                    return SourceGrounding {
                        field_name: field_name.to_string(),
                        generated_value: value.to_string(),
                        match_method: MatchMethod::Fuzzy {
                            similarity,
                            matched_tokens: matches.len(),
                        },
                        source_text: None,
                        source_url: self.source_urls.get(i).cloned(),
                    };
                }
            }
        }

        // 4. Check against dictionaries
        for (dict_name, entries) in &self.dictionaries {
            if entries.contains(&value_lower)
                || entries.iter().any(|e| e.eq_ignore_ascii_case(value))
            {
                return SourceGrounding {
                    field_name: field_name.to_string(),
                    generated_value: value.to_string(),
                    match_method: MatchMethod::Dictionary {
                        dict_name: dict_name.clone(),
                    },
                    source_text: None,
                    source_url: None,
                };
            }
        }

        // 5. Unsourced
        SourceGrounding {
            field_name: field_name.to_string(),
            generated_value: value.to_string(),
            match_method: MatchMethod::Unsourced,
            source_text: None,
            source_url: None,
        }
    }

    /// Validate all fields in a map and return grounding results.
    pub fn validate_fields(&self, fields: &HashMap<String, String>) -> Vec<SourceGrounding> {
        fields
            .iter()
            .map(|(name, value)| self.validate_field(name, value))
            .collect()
    }

    /// Returns true if the field is grounded (not hallucinated).
    pub fn is_grounded(grounding: &SourceGrounding) -> bool {
        !matches!(grounding.match_method, MatchMethod::Unsourced)
    }
}

// ─── Fact Extraction Mode ───────────────────────────────────────────────────

/// Extraction strictness level for LLM prompts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ExtractionMode {
    /// Standard extraction with inference allowed.
    Standard,
    /// Strict extraction: only extract, never invent; return "unknown" for missing info.
    Strict,
    /// Source-annotated: provide source index with each extracted fact.
    SourceAnnotated,
}

impl ExtractionMode {
    /// Generate a system prompt directive for this extraction mode.
    pub fn system_prompt_directive(&self) -> &'static str {
        match self {
            ExtractionMode::Standard => {
                "Extract information from the provided text. If information is not explicitly stated, do not invent it."
            }
            ExtractionMode::Strict => {
                "EXTRACTION-ONLY MODE: Extract ONLY facts that are explicitly stated in the provided text. \
                 For any field where the information is not explicitly present, output \"unknown\". \
                 Do NOT infer, guess, or fabricate any information. \
                 Every extracted value MUST be a verbatim substring or direct paraphrase of the source text. \
                 If you are unsure, output \"unknown\"."
            }
            ExtractionMode::SourceAnnotated => {
                "Extract information from the provided text. For each extracted fact, provide the \
                 source sentence or paragraph that supports it. Use [Source: N] notation to reference \
                 the specific source document. Only extract explicitly stated facts."
            }
        }
    }

    /// Augment an existing prompt with extraction-mode directives.
    pub fn augment_prompt(&self, prompt: &str) -> String {
        format!("{}\n\n---\n{}", self.system_prompt_directive(), prompt)
    }
}

// ─── Field Confidence Scoring ───────────────────────────────────────────────

/// Per-field confidence scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldConfidenceConfig {
    /// Minimum confidence for title/role fields.
    pub min_title_confidence: f64,
    /// Minimum confidence for company/organization fields.
    pub min_company_confidence: f64,
    /// Minimum confidence for any field.
    pub min_confidence: f64,
    /// Fields that are required (must have confidence >= min).
    pub required_fields: Vec<String>,
}

impl Default for FieldConfidenceConfig {
    fn default() -> Self {
        Self {
            min_title_confidence: 0.75,
            min_company_confidence: 0.65,
            min_confidence: 0.50,
            required_fields: vec![],
        }
    }
}

/// Confidence score for a single field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldConfidenceScore {
    pub field_name: String,
    pub value: String,
    pub confidence: f64,
    pub status: ConfidenceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConfidenceStatus {
    /// Confidence meets threshold.
    Clean,
    /// Confidence below threshold but above absolute minimum (0.3).
    Flagged,
    /// Confidence too low, value should be discarded.
    Rejected,
}

/// Score the confidence of a generated field based on source grounding.
pub fn score_field_confidence(
    grounding: &SourceGrounding,
    config: &FieldConfidenceConfig,
) -> FieldConfidenceScore {
    let base_confidence = match &grounding.match_method {
        MatchMethod::Exact => 0.95,
        MatchMethod::Fuzzy { similarity, .. } => 0.5 + (*similarity * 0.4).min(0.45),
        MatchMethod::Dictionary { .. } => 0.80,
        MatchMethod::Structured { .. } => 0.85,
        MatchMethod::PreExtracted { .. } => 0.90,
        MatchMethod::Unsourced => 0.0,
    };

    // Apply field-specific minimum thresholds
    let field_lower = grounding.field_name.to_lowercase();
    let min_threshold = if field_lower.contains("title") || field_lower.contains("role") {
        config.min_title_confidence
    } else if field_lower.contains("company") || field_lower.contains("org") {
        config.min_company_confidence
    } else {
        config.min_confidence
    };

    let status = if base_confidence >= min_threshold {
        ConfidenceStatus::Clean
    } else if base_confidence >= 0.3 {
        ConfidenceStatus::Flagged
    } else {
        ConfidenceStatus::Rejected
    };

    FieldConfidenceScore {
        field_name: grounding.field_name.clone(),
        value: grounding.generated_value.clone(),
        confidence: base_confidence,
        status,
    }
}

// ─── Cross-Reference Validator ──────────────────────────────────────────────

/// Validates generated facts against known dictionaries.
pub struct CrossReferenceValidator {
    /// Known valid job titles.
    known_titles: HashSet<String>,
    /// Known valid company names.
    known_companies: HashSet<String>,
    /// Known valid locations.
    known_locations: HashSet<String>,
    /// Blacklisted/impossible values (definitely hallucinated).
    blacklist: HashSet<String>,
}

impl Default for CrossReferenceValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl CrossReferenceValidator {
    pub fn new() -> Self {
        Self {
            known_titles: HashSet::new(),
            known_companies: HashSet::new(),
            known_locations: HashSet::new(),
            blacklist: HashSet::new(),
        }
    }

    /// Load known titles from a list.
    pub fn load_known_titles(&mut self, titles: &[&str]) {
        for t in titles {
            self.known_titles.insert(t.to_lowercase());
        }
    }

    /// Load known companies from a list.
    pub fn load_known_companies(&mut self, companies: &[&str]) {
        for c in companies {
            self.known_companies.insert(c.to_lowercase());
        }
    }

    /// Load known locations from a list.
    pub fn load_known_locations(&mut self, locations: &[&str]) {
        for l in locations {
            self.known_locations.insert(l.to_lowercase());
        }
    }

    /// Add blacklisted values that should never appear.
    pub fn add_blacklist(&mut self, values: &[&str]) {
        for v in values {
            self.blacklist.insert(v.to_lowercase());
        }
    }

    /// Check if a generated title is plausible.
    pub fn validate_title(&self, title: &str) -> TitleValidation {
        let lower = title.to_lowercase().trim().to_string();

        if lower.is_empty() || lower == "unknown" {
            return TitleValidation::Missing;
        }
        if self.blacklist.contains(&lower) {
            return TitleValidation::Rejected("blacklisted".to_string());
        }
        if self.known_titles.contains(&lower) {
            return TitleValidation::Known;
        }
        // Check partial match: at least one known title keyword present
        let tokens: HashSet<&str> = lower.split_whitespace().collect();
        let has_keyword = tokens.iter().any(|t| {
            self.known_titles
                .iter()
                .any(|kt| kt.contains(*t) || t.contains(kt.as_str()))
        });
        if has_keyword {
            TitleValidation::Plausible
        } else if title.chars().any(|c| c.is_ascii_uppercase()) {
            // Has proper-case words — might be a real title not in our dictionary
            TitleValidation::Unverified
        } else {
            TitleValidation::Suspicious
        }
    }

    /// Check if a generated company name is plausible.
    pub fn validate_company(&self, company: &str) -> CompanyValidation {
        let lower = company.to_lowercase().trim().to_string();

        if lower.is_empty() || lower == "unknown" {
            return CompanyValidation::Missing;
        }
        if self.blacklist.contains(&lower) {
            return CompanyValidation::Rejected("blacklisted".to_string());
        }
        if self.known_companies.contains(&lower) {
            return CompanyValidation::Known;
        }
        // Check for legal suffixes as a plausibility signal
        let has_suffix = lower.contains("inc")
            || lower.contains("ltd")
            || lower.contains("llc")
            || lower.contains("corp")
            || lower.contains("gmbh")
            || lower.contains("sa")
            || lower.contains("sas")
            || lower.contains("bv")
            || lower.contains("ag")
            || lower.contains("group")
            || lower.contains("technologies")
            || lower.contains("electronics");
        if has_suffix || company.chars().any(|c| c.is_ascii_uppercase()) {
            CompanyValidation::Unverified
        } else {
            CompanyValidation::Suspicious
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TitleValidation {
    Known,
    Plausible,
    Unverified,
    Suspicious,
    Missing,
    Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompanyValidation {
    Known,
    Unverified,
    Suspicious,
    Missing,
    Rejected(String),
}

// ─── Output Sanitizer ───────────────────────────────────────────────────────

/// Sanitizes LLM output by stripping unsourced claims.
pub struct OutputSanitizer {
    min_confidence: f64,
}

impl OutputSanitizer {
    pub fn new(min_confidence: f64) -> Self {
        Self { min_confidence }
    }

    /// Sanitize a set of field groundings, removing or marking unsourced values.
    /// Returns (clean_fields, flagged_fields, rejected_fields).
    pub fn sanitize(
        &self,
        groundings: &[SourceGrounding],
        config: &FieldConfidenceConfig,
    ) -> (HashMap<String, String>, Vec<String>, Vec<String>) {
        let mut clean = HashMap::new();
        let mut flagged = Vec::new();
        let mut rejected = Vec::new();

        for grounding in groundings {
            let score = score_field_confidence(grounding, config);
            match score.status {
                ConfidenceStatus::Clean => {
                    clean.insert(
                        grounding.field_name.clone(),
                        grounding.generated_value.clone(),
                    );
                }
                ConfidenceStatus::Flagged => {
                    flagged.push(format!(
                        "{}: '{}' (confidence: {:.2})",
                        grounding.field_name, grounding.generated_value, score.confidence
                    ));
                    // Still include flagged fields but mark them
                    clean.insert(
                        format!("{}_flagged", grounding.field_name),
                        grounding.generated_value.clone(),
                    );
                }
                ConfidenceStatus::Rejected => {
                    rejected.push(format!(
                        "{}: '{}' - REJECTED (confidence: {:.2})",
                        grounding.field_name, grounding.generated_value, score.confidence
                    ));
                    // Replace with empty/unknown
                    clean.insert(grounding.field_name.clone(), String::new());
                }
            }
        }

        (clean, flagged, rejected)
    }

    /// Quick check: does the output contain any unsourced claims?
    pub fn has_unsourced_claims(groundings: &[SourceGrounding]) -> bool {
        groundings
            .iter()
            .any(|g| matches!(g.match_method, MatchMethod::Unsourced))
    }

    /// Get a summary of grounding quality.
    pub fn grounding_summary(groundings: &[SourceGrounding]) -> GroundingSummary {
        let total = groundings.len();
        let sourced = groundings
            .iter()
            .filter(|g| SourceGroundingValidator::is_grounded(g))
            .count();
        GroundingSummary {
            total_fields: total,
            sourced_fields: sourced,
            unsourced_fields: total - sourced,
            grounding_ratio: if total > 0 {
                sourced as f64 / total as f64
            } else {
                1.0
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingSummary {
    pub total_fields: usize,
    pub sourced_fields: usize,
    pub unsourced_fields: usize,
    pub grounding_ratio: f64,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_grounding_exact_match() {
        let mut validator = SourceGroundingValidator::new();
        validator.add_source(
            "John Smith is the Supply Chain Manager at Foxconn Technology Group.",
            Some("https://example.com"),
        );

        let result = validator.validate_field("title", "Supply Chain Manager");
        assert!(matches!(result.match_method, MatchMethod::Exact));
        assert!(SourceGroundingValidator::is_grounded(&result));
    }

    #[test]
    fn test_source_grounding_pre_extracted() {
        let mut validator = SourceGroundingValidator::new();
        validator.add_source("Some text about the company.", None);
        validator.add_pre_extracted("title", vec!["VP Procurement".to_string()]);

        let result = validator.validate_field("title", "VP Procurement");
        assert!(matches!(
            result.match_method,
            MatchMethod::PreExtracted { .. }
        ));
        assert!(SourceGroundingValidator::is_grounded(&result));
    }

    #[test]
    fn test_source_grounding_unsourced() {
        let validator = SourceGroundingValidator::new();
        let result = validator.validate_field("title", "Completely Fabricated Title");
        assert!(matches!(result.match_method, MatchMethod::Unsourced));
        assert!(!SourceGroundingValidator::is_grounded(&result));
    }

    #[test]
    fn test_source_grounding_dictionary() {
        let mut validator = SourceGroundingValidator::new();
        let mut dict = HashSet::new();
        dict.insert("supply chain manager".to_string());
        validator.add_dictionary("titles", dict);

        let result = validator.validate_field("title", "Supply Chain Manager");
        assert!(matches!(
            result.match_method,
            MatchMethod::Dictionary { .. }
        ));
        assert!(SourceGroundingValidator::is_grounded(&result));
    }

    #[test]
    fn test_field_confidence_exact_is_high() {
        let grounding = SourceGrounding {
            field_name: "title".to_string(),
            generated_value: "CEO".to_string(),
            match_method: MatchMethod::Exact,
            source_text: Some("CEO John Smith".to_string()),
            source_url: None,
        };
        let config = FieldConfidenceConfig::default();
        let score = score_field_confidence(&grounding, &config);
        assert_eq!(score.status, ConfidenceStatus::Clean);
        assert!(score.confidence > 0.9);
    }

    #[test]
    fn test_field_confidence_unsourced_is_rejected() {
        let grounding = SourceGrounding {
            field_name: "title".to_string(),
            generated_value: "Fake Title".to_string(),
            match_method: MatchMethod::Unsourced,
            source_text: None,
            source_url: None,
        };
        let config = FieldConfidenceConfig::default();
        let score = score_field_confidence(&grounding, &config);
        assert_eq!(score.status, ConfidenceStatus::Rejected);
        assert_eq!(score.confidence, 0.0);
    }

    #[test]
    fn test_extraction_mode_strict_prompt() {
        let prompt = ExtractionMode::Strict.system_prompt_directive();
        assert!(prompt.contains("EXTRACTION-ONLY"));
        assert!(prompt.contains("unknown"));
        assert!(prompt.contains("Do NOT infer")); // explicitly forbids inference
    }

    #[test]
    fn test_cross_reference_validator_title() {
        let mut validator = CrossReferenceValidator::new();
        validator.load_known_titles(&[
            "Chief Executive Officer",
            "Supply Chain Manager",
            "VP Procurement",
        ]);

        assert_eq!(
            validator.validate_title("Supply Chain Manager"),
            TitleValidation::Known
        );
        // "CEO" doesn't keyword-match "Chief Executive Officer" (no shared token),
        // but it has uppercase chars so it's Unverified rather than Suspicious.
        assert_eq!(validator.validate_title("CEO"), TitleValidation::Unverified);
        assert_eq!(validator.validate_title(""), TitleValidation::Missing);
    }

    #[test]
    fn test_output_sanitizer() {
        let groundings = vec![
            SourceGrounding {
                field_name: "title".to_string(),
                generated_value: "Supply Chain Manager".to_string(),
                match_method: MatchMethod::Exact,
                source_text: Some("Supply Chain Manager".to_string()),
                source_url: None,
            },
            SourceGrounding {
                field_name: "company".to_string(),
                generated_value: "Fake Corp".to_string(),
                match_method: MatchMethod::Unsourced,
                source_text: None,
                source_url: None,
            },
        ];

        let sanitizer = OutputSanitizer::new(0.5);
        let config = FieldConfidenceConfig::default();
        let (clean, _flagged, rejected) = sanitizer.sanitize(&groundings, &config);

        assert!(clean.contains_key("title"));
        assert_eq!(clean.get("company").unwrap(), "");
        assert!(!rejected.is_empty());
    }

    #[test]
    fn test_grounding_summary() {
        let groundings = vec![
            SourceGrounding {
                field_name: "name".to_string(),
                generated_value: "John".to_string(),
                match_method: MatchMethod::Exact,
                source_text: None,
                source_url: None,
            },
            SourceGrounding {
                field_name: "title".to_string(),
                generated_value: "Fake".to_string(),
                match_method: MatchMethod::Unsourced,
                source_text: None,
                source_url: None,
            },
        ];

        let summary = OutputSanitizer::grounding_summary(&groundings);
        assert_eq!(summary.total_fields, 2);
        assert_eq!(summary.sourced_fields, 1);
        assert_eq!(summary.unsourced_fields, 1);
        assert!((summary.grounding_ratio - 0.5).abs() < 0.01);
    }
}
