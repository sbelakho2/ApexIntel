//! FST-based autocomplete index for search suggestions.
//!
//! Uses the `fst` crate (Finite State Transducer) to provide fast prefix-based
//! lookups over entity names (companies, persons, insights, competitors, etc.).
//!
//! The FST maps each lowercase-normalized text key to a stream index, which
//! can be looked up in a parallel `keys` vector to retrieve the full entry.

use anyhow::Result;
use fst::{Automaton, IntoStreamer, Map, MapBuilder, Streamer};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

// ────────────────────────────────────────────
// Data types
// ────────────────────────────────────────────

/// An entry to be indexed in the autocomplete FST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteEntry {
    /// Display text (company name, insight title, person name, etc.)
    pub text: String,
    /// Entity type discriminator: "company", "insight", "person", "competitor"
    pub entity_type: String,
    /// UUID of the entity (link target)
    pub id: Uuid,
    /// Relevance score for ranking (higher = more relevant)
    pub score: f64,
    /// Secondary info (industry, country, role, etc.)
    pub subtext: Option<String>,
}

/// A single suggestion returned from the autocomplete index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSuggestion {
    pub text: String,
    pub entity_type: String,
    pub id: Uuid,
    pub score: f64,
    pub subtext: Option<String>,
}

// ────────────────────────────────────────────
// AutocompleteIndex
// ────────────────────────────────────────────

/// An FST-based autocomplete index.
///
/// The FST maps lowercase-normalized keys → stream index (u64). The `keys`
/// vector provides reverse lookup from stream index to the full entry data.
///
/// This approach gives O(n) prefix matching over the FST where n is the
/// number of results returned, regardless of total index size.
pub struct AutocompleteIndex {
    map: Map<Vec<u8>>,
    keys: Vec<AutocompleteEntry>,
}

// Manual Clone impl: Map<Vec<u8>> is Clone because Vec<u8>: Clone
impl Clone for AutocompleteIndex {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            keys: self.keys.clone(),
        }
    }
}

impl AutocompleteIndex {
    /// Create an empty index (no entries).
    ///
    /// Building an empty FST from constant inputs cannot fail; the final
    /// `unwrap_or_else` is a defensive fallback for a byte layout that is
    /// always valid.
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    pub fn new() -> Self {
        Self {
            map: Map::new(vec![]).unwrap_or_else(|_| {
                // Build a minimal empty FST
                let bytes = MapBuilder::new(vec![]).unwrap().into_inner().unwrap();
                Map::new(bytes).unwrap()
            }),
            keys: Vec::new(),
        }
    }

    /// Build a new index from a set of entries.
    ///
    /// This is the primary construction path. The FST is built in-memory
    /// from the provided entries, which are deduplicated by their
    /// `(lowercased_text, entity_type, id)` key.
    pub fn build(entries: &[AutocompleteEntry]) -> Result<Self> {
        // Deduplicate by (lowercase text, entity_type, id) and sort by key
        // so the FST builder produces a minimal automaton.
        let mut seen = std::collections::HashSet::new();
        let mut unique_entries: Vec<&AutocompleteEntry> = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = (
                entry.text.to_lowercase(),
                entry.entity_type.as_str(),
                entry.id,
            );
            if seen.insert(key) {
                unique_entries.push(entry);
            }
        }

        // Sort unique entries by their FST key for optimal FST construction
        unique_entries.sort_by(|a, b| {
            a.text
                .to_lowercase()
                .cmp(&b.text.to_lowercase())
                .then(a.entity_type.cmp(&b.entity_type))
                .then(a.id.cmp(&b.id))
        });

        let mut builder = MapBuilder::new(vec![])?;
        let mut keys: Vec<AutocompleteEntry> = Vec::with_capacity(unique_entries.len());

        for (stream_idx, entry) in unique_entries.iter().enumerate() {
            let key = format!(
                "{}\x00{}\x00{}",
                entry.text.to_lowercase(),
                entry.entity_type,
                entry.id
            );
            builder.insert(key, stream_idx as u64)?;
            keys.push((*entry).clone());
        }

        let fst_bytes = builder.into_inner()?;
        let map = Map::new(fst_bytes)?;

        Ok(Self { map, keys })
    }

    /// Load an index from a file path.
    pub fn load(path: &Path) -> Result<Self> {
        let fst_bytes = std::fs::read(path)?;
        let map = Map::new(fst_bytes)?;
        // Keys are stored alongside in a JSON file
        let keys_path = path.with_extension("json");
        let keys: Vec<AutocompleteEntry> = if keys_path.exists() {
            let json = std::fs::read_to_string(&keys_path)?;
            serde_json::from_str(&json)?
        } else {
            Vec::new()
        };
        Ok(Self { map, keys })
    }

    /// Save the index to a file path (FST binary + keys JSON).
    pub fn save(&self, path: &Path) -> Result<()> {
        // Extract the underlying byte slice from the FST
        let bytes = self.map.as_ref().as_bytes();
        std::fs::write(path, bytes)?;
        let keys_path = path.with_extension("json");
        let json = serde_json::to_string(&self.keys)?;
        std::fs::write(keys_path, json)?;
        Ok(())
    }

    /// Insert a new entry (rebuilds the FST).
    ///
    /// This recreates the entire FST from scratch. For bulk operations,
    /// prefer using `build()` directly.
    pub fn insert(&mut self, entry: AutocompleteEntry) -> Result<()> {
        let mut all_entries: Vec<AutocompleteEntry> = self.keys.clone();
        // Replace existing entry with same (lowercase text, entity_type, id)
        all_entries.retain(|e| {
            !(e.text.to_lowercase() == entry.text.to_lowercase()
                && e.entity_type == entry.entity_type
                && e.id == entry.id)
        });
        all_entries.push(entry);

        let rebuilt = Self::build(&all_entries)?;
        self.map = rebuilt.map;
        self.keys = rebuilt.keys;
        Ok(())
    }

    /// Suggest completions for a given prefix.
    ///
    /// Returns up to `limit` suggestions, sorted by score descending.
    /// Returns empty if `prefix` is empty or has fewer than 2 characters.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<AutocompleteSuggestion> {
        if prefix.trim().len() < 2 {
            return Vec::new();
        }

        let prefix_lower = prefix.trim().to_lowercase();
        let limit = limit.min(25);

        // Use FST prefix search to find matching entries
        let automaton = fst::automaton::Str::new(&prefix_lower).starts_with();
        let mut stream = self.map.search(automaton).into_stream();

        let mut results: Vec<(u64, AutocompleteSuggestion)> = Vec::new();
        while let Some((_key, stream_idx)) = stream.next() {
            let idx = stream_idx as usize;
            if idx >= self.keys.len() {
                continue;
            }
            let entry = &self.keys[idx];
            results.push((
                (entry.score * 1000.0) as u64,
                AutocompleteSuggestion {
                    text: entry.text.clone(),
                    entity_type: entry.entity_type.clone(),
                    id: entry.id,
                    score: entry.score,
                    subtext: entry.subtext.clone(),
                },
            ));
        }

        // Sort by score descending
        results.sort_by_key(|a| std::cmp::Reverse(a.0));

        // Take top N
        results
            .into_iter()
            .take(limit)
            .map(|(_, suggestion)| suggestion)
            .collect()
    }

    /// Return the number of entries in the index.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns true if the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl Default for AutocompleteIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────
// Database seeding
// ────────────────────────────────────────────

/// Build the autocomplete index from database data.
///
/// Reads from companies, persons, and insights tables to create
/// a comprehensive autocomplete index. This is the production path
/// for seeding/rebuilding the index.
pub async fn build_from_database(store: &crate::postgres::PgStore) -> Result<AutocompleteIndex> {
    let mut entries: Vec<AutocompleteEntry> = Vec::new();

    // 1. Company names
    if let Ok(companies) = store
        .list_companies(
            &crate::postgres::CompanyListFilters::default(),
            None,
            false,
            10_000,
            0,
        )
        .await
    {
        for company in &companies {
            let score = company
                .threat_score
                .or(company.overlap_score)
                .unwrap_or(0.5)
                .clamp(0.0, 10.0);
            let subtext = company
                .region
                .as_deref()
                .or(company.country_code.as_deref())
                .map(|s| s.to_string());

            entries.push(AutocompleteEntry {
                text: company.name.clone(),
                entity_type: "company".to_string(),
                id: company.id,
                score,
                subtext,
            });

            // Also index legal names
            if let Some(ref legal_name) = company.legal_name {
                if !legal_name.trim().is_empty() && legal_name.trim() != company.name.trim() {
                    entries.push(AutocompleteEntry {
                        text: legal_name.trim().to_string(),
                        entity_type: "company".to_string(),
                        id: company.id,
                        score: score * 0.9, // slightly lower weight for legal name
                        subtext: company.region.clone(),
                    });
                }
            }
        }
    }

    // 2. Person names
    if let Ok(persons) = store
        .list_persons(
            &crate::postgres::PersonListFilters::default(),
            None,
            false,
            10_000,
            0,
        )
        .await
    {
        for person in &persons {
            let score = (person.priority_score * 0.1).clamp(0.0, 10.0);
            let subtext = if person.organization != "Independent" {
                Some(person.organization.clone())
            } else {
                Some(person.role.clone())
            };

            entries.push(AutocompleteEntry {
                text: person.name.clone(),
                entity_type: "person".to_string(),
                id: person.id,
                score,
                subtext,
            });
        }
    }

    // 3. Insight titles
    if let Ok(insights) = store
        .list_insights(&crate::postgres::InsightListFilters::default(), 10_000, 0)
        .await
    {
        for insight in &insights {
            let score = insight.confidence.unwrap_or(0.5).clamp(0.0, 10.0);
            let subtext = insight.insight_type.clone();

            entries.push(AutocompleteEntry {
                text: insight.title.clone(),
                entity_type: "insight".to_string(),
                id: insight.id,
                score,
                subtext,
            });
        }
    }

    // 4. Competitor names from competitive intelligence engine
    if let Ok(competitors) = store.list_competitors(10_000, 0).await {
        for competitor in &competitors {
            let score = competitor.threat_score.unwrap_or(5.0).clamp(0.0, 10.0);
            entries.push(AutocompleteEntry {
                text: competitor.name.clone(),
                entity_type: "competitor".to_string(),
                id: competitor.id,
                score,
                subtext: competitor.region.clone(),
            });
        }
    }

    tracing::info!(
        "Building autocomplete index from database: {} entries ({} companies, {} persons, {} insights, {} competitors)",
        entries.len(),
        entries.iter().filter(|e| e.entity_type == "company").count(),
        entries.iter().filter(|e| e.entity_type == "person").count(),
        entries.iter().filter(|e| e.entity_type == "insight").count(),
        entries.iter().filter(|e| e.entity_type == "competitor").count(),
    );

    AutocompleteIndex::build(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<AutocompleteEntry> {
        vec![
            AutocompleteEntry {
                text: "Apple Inc.".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
                score: 9.5,
                subtext: Some("Technology".to_string()),
            },
            AutocompleteEntry {
                text: "Apple Store".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").unwrap(),
                score: 4.0,
                subtext: Some("Retail".to_string()),
            },
            AutocompleteEntry {
                text: "Taiwan Semiconductor Manufacturing Company".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440003").unwrap(),
                score: 8.0,
                subtext: Some("Semiconductors".to_string()),
            },
            AutocompleteEntry {
                text: "Taiwan Semiconductor Co.".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440004").unwrap(),
                score: 3.5,
                subtext: Some("Semiconductors".to_string()),
            },
            AutocompleteEntry {
                text: "John Appleseed".to_string(),
                entity_type: "person".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440005").unwrap(),
                score: 7.0,
                subtext: Some("CEO".to_string()),
            },
            AutocompleteEntry {
                text: "AP Moller-Maersk".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440006").unwrap(),
                score: 6.0,
                subtext: Some("Logistics".to_string()),
            },
            AutocompleteEntry {
                text: "Supply Chain Insights Q1".to_string(),
                entity_type: "insight".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440007").unwrap(),
                score: 8.5,
                subtext: Some("supply_chain".to_string()),
            },
            AutocompleteEntry {
                text: "Apple Supplier Report".to_string(),
                entity_type: "insight".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440008").unwrap(),
                score: 6.0,
                subtext: Some("competitive".to_string()),
            },
        ]
    }

    #[test]
    fn test_build_empty_index() {
        let idx = AutocompleteIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_build_with_entries() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        assert!(!idx.is_empty());
        assert_eq!(idx.len(), 8);
    }

    #[test]
    fn test_suggest_prefix_ap_returns_apple() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("ap", 10);
        assert!(!results.is_empty(), "Should find results for 'ap'");
        let texts: Vec<&str> = results.iter().map(|r| r.text.as_str()).collect();
        assert!(
            texts.contains(&"Apple Inc."),
            "Should contain 'Apple Inc.' — got {:?}",
            texts
        );
        assert!(
            texts.contains(&"Apple Store"),
            "Should contain 'Apple Store'"
        );
    }

    #[test]
    fn test_suggest_prefix_tai_returns_tsmc() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("tai", 10);
        assert!(!results.is_empty(), "Should find results for 'tai'");
        let texts: Vec<&str> = results.iter().map(|r| r.text.as_str()).collect();
        assert!(
            texts.contains(&"Taiwan Semiconductor Manufacturing Company"),
            "Should contain 'Taiwan Semiconductor Manufacturing Company' — got {:?}",
            texts
        );
        assert!(
            texts.contains(&"Taiwan Semiconductor Co."),
            "Should contain 'Taiwan Semiconductor Co.'"
        );
    }

    #[test]
    fn test_empty_prefix_returns_empty() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("", 10);
        assert!(results.is_empty());
        let results = idx.suggest("a", 10);
        assert!(results.is_empty(), "Single char should return empty");
    }

    #[test]
    fn test_limit_parameter() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("ap", 2);
        assert!(results.len() <= 2, "Should limit to 2 results");
    }

    #[test]
    fn test_special_characters() {
        let entries = vec![
            AutocompleteEntry {
                text: "Côte d'Ivoire Telecom".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::new_v4(),
                score: 5.0,
                subtext: None,
            },
            AutocompleteEntry {
                text: "Össur hf.".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::new_v4(),
                score: 4.0,
                subtext: None,
            },
        ];
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("côte", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "Côte d'Ivoire Telecom");

        let results = idx.suggest("össur", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "Össur hf.");
    }

    #[test]
    fn test_case_insensitive() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let upper = idx.suggest("AP", 10);
        let lower = idx.suggest("ap", 10);
        let mixed = idx.suggest("Ap", 10);
        assert_eq!(upper.len(), lower.len());
        assert_eq!(lower.len(), mixed.len());
    }

    #[test]
    fn test_suggestions_include_entity_type() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("ap", 10);
        for result in &results {
            assert!(
                !result.entity_type.is_empty(),
                "Entity type should not be empty"
            );
            assert!(
                ["company", "person", "insight", "competitor"]
                    .contains(&result.entity_type.as_str()),
                "Unexpected entity type: {}",
                result.entity_type
            );
        }
    }

    #[test]
    fn test_score_ordering() {
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        let results = idx.suggest("ap", 10);
        // Results should be ordered by score descending
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "Results should be sorted by score descending: {} ({}) >= {} ({})",
                window[0].text,
                window[0].score,
                window[1].text,
                window[1].score
            );
        }
    }

    #[test]
    fn test_duplicate_dedup() {
        let entries = vec![
            AutocompleteEntry {
                text: "Apple Inc.".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
                score: 9.5,
                subtext: None,
            },
            // Duplicate (same lowercase text, same entity_type, same id)
            AutocompleteEntry {
                text: "Apple Inc.".to_string(),
                entity_type: "company".to_string(),
                id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
                score: 9.5,
                subtext: None,
            },
        ];
        let idx = AutocompleteIndex::build(&entries).unwrap();
        assert_eq!(idx.len(), 1, "Duplicates should be deduplicated");
    }

    #[test]
    fn test_insert_after_build() {
        let entries = sample_entries();
        let mut idx = AutocompleteIndex::build(&entries).unwrap();
        assert_eq!(idx.len(), 8);

        idx.insert(AutocompleteEntry {
            text: "Apple Vision Pro".to_string(),
            entity_type: "product".to_string(),
            id: Uuid::new_v4(),
            score: 7.0,
            subtext: None,
        })
        .unwrap();

        assert_eq!(idx.len(), 9, "After insert, should have 9 entries");
        let results = idx.suggest("apple", 10);
        assert!(
            results.iter().any(|r| r.text == "Apple Vision Pro"),
            "New entry should be searchable"
        );
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("autocomplete.fst");
        let entries = sample_entries();
        let idx = AutocompleteIndex::build(&entries).unwrap();
        idx.save(&path).unwrap();

        let loaded = AutocompleteIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), idx.len());
        let results = loaded.suggest("ap", 10);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.text == "Apple Inc."));
    }
}
