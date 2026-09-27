use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{BooleanQuery, QueryParser, TermQuery};
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use uuid::Uuid;

/// File name (inside the index directory) holding the last successful commit
/// checkpoint. Readiness compares its high-water mark against the database's
/// newest observation, so an empty or lagging index cannot report `ok`.
pub const INDEX_CHECKPOINT_FILE: &str = "apex_index_checkpoint.json";

/// Truncate a string to at most `max_chars` characters (safe for multi-byte UTF-8).
fn truncate_snippet(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Proof that the index was successfully committed, and the source-data
/// high-water mark that commit covered.
///
/// Written by whoever commits documents (`SearchIndex::record_checkpoint`) and
/// read by readiness probes. A missing checkpoint means the index has never
/// recorded a successful commit, which is not the same as "healthy".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexCheckpoint {
    /// When the commit that produced this checkpoint succeeded.
    pub last_commit_at: DateTime<Utc>,
    /// Newest source observation timestamp included in that commit (`None`
    /// when the indexer has never seen source data).
    pub high_water_ts: Option<DateTime<Utc>>,
    /// Id of the newest source observation included in that commit. Together
    /// with `high_water_ts` this is a keyset cursor: many observations can
    /// share one timestamp, so paging on the timestamp alone cannot advance.
    #[serde(default)]
    pub high_water_id: Option<Uuid>,
    /// Number of documents committed in that commit.
    pub indexed_documents: u64,
}

/// Read a checkpoint file, returning `None` when it is missing or unreadable.
///
/// An unreadable checkpoint must never fail index open (the read path stays
/// available) but also must never be invented: readiness treats `None` as
/// "never committed".
fn read_checkpoint_file(path: &Path) -> Option<IndexCheckpoint> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Full-text search index for observations, companies, and documents.
pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    #[allow(dead_code)]
    schema: Schema,
    /// Where the commit checkpoint lives (`None` for in-memory indexes).
    checkpoint_path: Option<PathBuf>,
    /// Last checkpoint read from / written to `checkpoint_path`.
    checkpoint: Mutex<Option<IndexCheckpoint>>,
    // Field handles
    pub id_field: Field,
    pub entity_type_field: Field,
    pub entity_id_field: Field,
    pub title_field: Field,
    pub body_field: Field,
    pub url_field: Field,
    pub region_field: Field,
    pub tags_field: Field,
    pub timestamp_field: Field,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub title: String,
    pub snippet: String,
    pub url: String,
    pub region: String,
    pub score: f32,
    /// Unix timestamp (seconds since epoch) from the indexed document.
    pub timestamp: i64,
}

impl SearchIndex {
    /// Create or open an index at the given directory path.
    pub fn open(index_path: &Path) -> Result<Self> {
        let mut schema_builder = Schema::builder();

        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let entity_type_field = schema_builder.add_text_field("entity_type", STRING | STORED);
        let entity_id_field = schema_builder.add_text_field("entity_id", STRING | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let region_field = schema_builder.add_text_field("region", STRING | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let timestamp_field = schema_builder.add_i64_field("timestamp", INDEXED | STORED);

        let schema = schema_builder.build();

        let index = if index_path.exists() && index_path.join("meta.json").exists() {
            Index::open_in_dir(index_path)?
        } else {
            std::fs::create_dir_all(index_path)?;
            Index::create_in_dir(index_path, schema.clone())?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        let checkpoint_path = Some(index_path.join(INDEX_CHECKPOINT_FILE));
        let checkpoint = checkpoint_path.as_deref().and_then(read_checkpoint_file);

        Ok(Self {
            index,
            reader,
            schema,
            checkpoint_path,
            checkpoint: Mutex::new(checkpoint),
            id_field,
            entity_type_field,
            entity_id_field,
            title_field,
            body_field,
            url_field,
            region_field,
            tags_field,
            timestamp_field,
        })
    }

    /// Create a temporary in-memory (RAM) index for testing.
    pub fn in_memory() -> Result<Self> {
        let mut schema_builder = Schema::builder();

        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let entity_type_field = schema_builder.add_text_field("entity_type", STRING | STORED);
        let entity_id_field = schema_builder.add_text_field("entity_id", STRING | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let region_field = schema_builder.add_text_field("region", STRING | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let timestamp_field = schema_builder.add_i64_field("timestamp", INDEXED | STORED);

        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema.clone());

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            schema,
            checkpoint_path: None,
            checkpoint: Mutex::new(None),
            id_field,
            entity_type_field,
            entity_id_field,
            title_field,
            body_field,
            url_field,
            region_field,
            tags_field,
            timestamp_field,
        })
    }

    /// Last successful commit checkpoint, if any was recorded.
    ///
    /// For on-disk indexes the checkpoint file is the source of truth and is
    /// re-read on every call, so the API observes commits recorded by the
    /// worker's indexer without restarting. A `None` result means this index
    /// has never recorded a commit — an empty or freshly recreated index.
    /// Readiness must not treat that as healthy.
    pub fn checkpoint(&self) -> Option<IndexCheckpoint> {
        let mut guard = match self.checkpoint.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(path) = self.checkpoint_path.as_deref() {
            let fresh = read_checkpoint_file(path);
            *guard = fresh.clone();
            return fresh;
        }
        guard.clone()
    }

    /// Persist a commit checkpoint after a successful `IndexWriter::commit`.
    ///
    /// The file is written atomically (temp file + rename) so a crashing
    /// indexer can never leave a half-written checkpoint that a subsequent
    /// readiness probe would misread as fresh.
    pub fn record_checkpoint(&self, checkpoint: &IndexCheckpoint) -> Result<()> {
        let Some(path) = self.checkpoint_path.as_deref() else {
            // In-memory indexes keep the checkpoint in memory only.
            match self.checkpoint.lock() {
                Ok(mut guard) => *guard = Some(checkpoint.clone()),
                Err(poisoned) => *poisoned.into_inner() = Some(checkpoint.clone()),
            }
            return Ok(());
        };

        let serialized = serde_json::to_vec_pretty(checkpoint)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &serialized)?;
        std::fs::rename(&tmp_path, path)?;
        match self.checkpoint.lock() {
            Ok(mut guard) => *guard = Some(checkpoint.clone()),
            Err(poisoned) => *poisoned.into_inner() = Some(checkpoint.clone()),
        }
        Ok(())
    }

    /// Get a writer with specified heap size.
    pub fn writer(&self, heap_size: usize) -> Result<IndexWriter> {
        Ok(self.index.writer(heap_size)?)
    }

    /// Index a document. Deletes any existing document with the same ID first
    /// to prevent duplicates on re-index.
    pub fn index_document(
        &self,
        writer: &IndexWriter,
        id: &str,
        entity_type: &str,
        entity_id: &str,
        title: &str,
        body: &str,
        url: &str,
        region: &str,
        tags: &[String],
        timestamp: i64,
    ) -> Result<()> {
        // Remove existing doc with this ID to prevent duplicates
        writer.delete_term(Term::from_field_text(self.id_field, id));

        let tags_str = tags.join(" ");
        writer.add_document(doc!(
            self.id_field => id,
            self.entity_type_field => entity_type,
            self.entity_id_field => entity_id,
            self.title_field => title,
            self.body_field => body,
            self.url_field => url,
            self.region_field => region,
            self.tags_field => tags_str,
            self.timestamp_field => timestamp,
        ))?;
        Ok(())
    }

    /// Search across title and body fields.
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        Ok(self.search_with_total(query_str, limit, 0)?.0)
    }

    /// Search across title and body fields with pagination and total hits.
    pub fn search_with_total(
        &self,
        query_str: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchResult>, u64)> {
        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.body_field]);
        let query = query_parser.parse_query(query_str)?;
        let total_hits = searcher.search(&query, &Count)? as u64;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).and_offset(offset))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;

            let id = doc
                .get_first(self.id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entity_type = doc
                .get_first(self.entity_type_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entity_id = doc
                .get_first(self.entity_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get_first(self.title_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let body = doc
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = doc
                .get_first(self.url_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let region = doc
                .get_first(self.region_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let timestamp = doc
                .get_first(self.timestamp_field)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let snippet = truncate_snippet(&body, 200);

            results.push(SearchResult {
                id,
                entity_type,
                entity_id,
                title,
                snippet,
                url,
                region,
                score,
                timestamp,
            });
        }

        Ok((results, total_hits))
    }

    /// Search filtered by entity_type.
    pub fn search_entity_type(
        &self,
        query_str: &str,
        entity_type: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        Ok(self
            .search_entity_type_with_total(query_str, entity_type, limit, 0)?
            .0)
    }

    pub fn search_entity_type_with_total(
        &self,
        query_str: &str,
        entity_type: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchResult>, u64)> {
        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.body_field]);
        let text_query = query_parser.parse_query(query_str)?;
        // Use BooleanQuery to combine text query with entity_type filter safely
        let type_term = Term::from_field_text(self.entity_type_field, entity_type);
        let type_query = TermQuery::new(type_term, IndexRecordOption::Basic);
        let query = BooleanQuery::new(vec![
            (tantivy::query::Occur::Must, text_query),
            (tantivy::query::Occur::Must, Box::new(type_query)),
        ]);
        let total_hits = searcher.search(&query, &Count)? as u64;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).and_offset(offset))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;

            let id = doc
                .get_first(self.id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let et = doc
                .get_first(self.entity_type_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let eid = doc
                .get_first(self.entity_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get_first(self.title_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let body = doc
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = doc
                .get_first(self.url_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let region = doc
                .get_first(self.region_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let timestamp = doc
                .get_first(self.timestamp_field)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let snippet = truncate_snippet(&body, 200);

            results.push(SearchResult {
                id,
                entity_type: et,
                entity_id: eid,
                title,
                snippet,
                url,
                region,
                score,
                timestamp,
            });
        }

        Ok((results, total_hits))
    }

    /// Reload the reader to pick up committed changes.
    pub fn reload(&self) -> Result<()> {
        self.reader.reload()?;
        Ok(())
    }

    /// Get total number of documents in the index.
    pub fn num_docs(&self) -> u64 {
        let searcher = self.reader.searcher();
        searcher.num_docs()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use uuid::Uuid;

    fn create_test_index() -> SearchIndex {
        SearchIndex::in_memory().unwrap()
    }

    #[test]
    fn test_create_in_memory_index() {
        let idx = create_test_index();
        assert_eq!(idx.num_docs(), 0);
        assert!(idx.checkpoint().is_none(), "fresh index has no checkpoint");
    }

    #[test]
    fn checkpoint_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let idx = SearchIndex::open(dir.path()).unwrap();
        assert!(idx.checkpoint().is_none());

        let checkpoint = IndexCheckpoint {
            last_commit_at: Utc::now(),
            high_water_ts: Some(Utc::now() - chrono::Duration::minutes(5)),
            high_water_id: None,
            indexed_documents: 7,
        };
        idx.record_checkpoint(&checkpoint).unwrap();
        assert_eq!(idx.checkpoint().as_ref(), Some(&checkpoint));

        let reopened = SearchIndex::open(dir.path()).unwrap();
        assert_eq!(reopened.checkpoint().as_ref(), Some(&checkpoint));
    }

    #[test]
    fn checkpoint_is_observed_across_independent_handles() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SearchIndex::open(dir.path()).unwrap();
        let reader = SearchIndex::open(dir.path()).unwrap();

        let checkpoint = IndexCheckpoint {
            last_commit_at: Utc::now(),
            high_water_ts: Some(Utc::now()),
            high_water_id: None,
            indexed_documents: 3,
        };
        writer.record_checkpoint(&checkpoint).unwrap();

        assert_eq!(
            reader.checkpoint().as_ref(),
            Some(&checkpoint),
            "a reader handle must observe commits recorded by another process"
        );
    }

    #[test]
    fn corrupt_checkpoint_reads_as_absent_instead_of_failing_open() {
        let dir = tempfile::tempdir().unwrap();
        let idx = SearchIndex::open(dir.path()).unwrap();
        idx.record_checkpoint(&IndexCheckpoint {
            last_commit_at: Utc::now(),
            high_water_ts: None,
            high_water_id: None,
            indexed_documents: 1,
        })
        .unwrap();
        std::fs::write(dir.path().join(INDEX_CHECKPOINT_FILE), b"{not json").unwrap();

        let reopened = SearchIndex::open(dir.path()).unwrap();
        assert!(
            reopened.checkpoint().is_none(),
            "a corrupt checkpoint must read as 'never committed', not as stale health"
        );
    }

    #[test]
    fn test_index_and_search_single_doc() {
        let idx = create_test_index();
        let mut writer = idx.writer(15_000_000).unwrap();

        idx.index_document(
            &writer,
            &Uuid::new_v4().to_string(),
            "company",
            &Uuid::new_v4().to_string(),
            "Starz Electronics SARL",
            "EMS manufacturer specializing in automotive PCB assembly in Tunisia",
            "https://starz-electronics.com",
            "TN",
            &["ems".into(), "automotive".into(), "pcb".into()],
            1700000000,
        )
        .unwrap();

        writer.commit().unwrap();
        idx.reload().unwrap();

        assert_eq!(idx.num_docs(), 1);

        let results = idx.search("automotive", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("Starz"));
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_search_multiple_docs() {
        let idx = create_test_index();
        let mut writer = idx.writer(15_000_000).unwrap();

        idx.index_document(
            &writer,
            &Uuid::new_v4().to_string(),
            "company",
            &Uuid::new_v4().to_string(),
            "Foxconn Technology Group",
            "World's largest EMS provider based in Taiwan, manufacturing for Apple",
            "https://foxconn.com",
            "TW",
            &["ems".into(), "consumer_electronics".into()],
            1700000000,
        )
        .unwrap();

        idx.index_document(
            &writer,
            &Uuid::new_v4().to_string(),
            "company",
            &Uuid::new_v4().to_string(),
            "Starz Electronics",
            "Tunisian EMS company for automotive clients",
            "https://starz-electronics.com",
            "TN",
            &["ems".into(), "automotive".into()],
            1700000000,
        )
        .unwrap();

        idx.index_document(
            &writer,
            &Uuid::new_v4().to_string(),
            "observation",
            &Uuid::new_v4().to_string(),
            "Job Post: Senior Quality Engineer",
            "Seeking SQE with automotive IATF experience for Sousse plant",
            "https://jobs.example.com/123",
            "TN",
            &["job_post".into(), "quality".into()],
            1700000001,
        )
        .unwrap();

        writer.commit().unwrap();
        idx.reload().unwrap();

        assert_eq!(idx.num_docs(), 3);

        // Search for EMS - should find both companies
        let results = idx.search("EMS", 10).unwrap();
        assert_eq!(results.len(), 2);

        // Search for automotive - should find Starz + job post
        let results = idx.search("automotive", 10).unwrap();
        assert!(results.len() >= 2);

        // Search for IATF - should find job post
        let results = idx.search("IATF", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("Quality"));
    }

    #[test]
    fn test_search_no_results() {
        let idx = create_test_index();
        let mut writer = idx.writer(15_000_000).unwrap();

        idx.index_document(
            &writer,
            &Uuid::new_v4().to_string(),
            "company",
            &Uuid::new_v4().to_string(),
            "Starz Electronics",
            "Tunisian EMS company",
            "https://starz.com",
            "TN",
            &["ems".into()],
            1700000000,
        )
        .unwrap();

        writer.commit().unwrap();
        idx.reload().unwrap();

        let results = idx.search("cryptocurrency", 10).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            id: Uuid::new_v4().to_string(),
            entity_type: "company".into(),
            entity_id: Uuid::new_v4().to_string(),
            title: "Test Company".into(),
            snippet: "A test company snippet".into(),
            url: "https://example.com".into(),
            region: "TN".into(),
            score: 1.5,
            timestamp: 1700000000,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.title, "Test Company");
        assert_eq!(deser.region, "TN");
    }

    #[test]
    fn test_open_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let idx = SearchIndex::open(tmp.path()).unwrap();
        let mut writer = idx.writer(15_000_000).unwrap();

        idx.index_document(
            &writer,
            "doc-1",
            "company",
            "entity-1",
            "Persistent Test",
            "This document should persist on disk",
            "https://example.com",
            "EU",
            &["test".into()],
            1700000000,
        )
        .unwrap();
        writer.commit().unwrap();
        idx.reload().unwrap();
        assert_eq!(idx.num_docs(), 1);

        // Re-open and verify
        drop(idx);
        let idx2 = SearchIndex::open(tmp.path()).unwrap();
        assert_eq!(idx2.num_docs(), 1);
        let results = idx2.search("persistent", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_index_many_docs() {
        let idx = create_test_index();
        let mut writer = idx.writer(15_000_000).unwrap();

        for i in 0..50 {
            idx.index_document(
                &writer,
                &format!("doc-{}", i),
                "observation",
                &Uuid::new_v4().to_string(),
                &format!("Observation {}", i),
                &format!(
                    "Body content for observation number {} about electronics manufacturing",
                    i
                ),
                &format!("https://example.com/{}", i),
                if i % 2 == 0 { "TN" } else { "MA" },
                &["observation".into()],
                1700000000 + i,
            )
            .unwrap();
        }

        writer.commit().unwrap();
        idx.reload().unwrap();

        assert_eq!(idx.num_docs(), 50);

        let results = idx.search("electronics manufacturing", 100).unwrap();
        assert_eq!(results.len(), 50);

        // Limit results
        let results = idx.search("electronics", 5).unwrap();
        assert_eq!(results.len(), 5);
    }
}
