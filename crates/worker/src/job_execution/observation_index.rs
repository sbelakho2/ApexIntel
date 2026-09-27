//! Observation → search-index job.
//!
//! The search index is only trustworthy if something actually commits
//! documents and records how far it got. This job indexes observations newer
//! than the persisted commit checkpoint (one bounded batch per run), commits
//! them, and advances the checkpoint's observation high-water mark. Readiness
//! compares that checkpoint with the database's newest observation, so index
//! lag is a measurement rather than an assumption.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use apex_store::postgres::{ObservationRow, PgStore};
use apex_store::tantivy_index::{IndexCheckpoint, SearchIndex};

use apex_worker::scheduler::{JobKind, JobRun};

/// Observations indexed per run. Bounds memory, writer commit size, and how
/// long a single run can hold the tantivy writer lock.
const INDEX_BATCH_LIMIT: i64 = 1_000;

/// Indexed observation bodies are provenance for search, not the canonical
/// record, so the JSON is truncated to keep the index small.
const MAX_BODY_CHARS: usize = 4_000;

/// Writer heap for the per-run index writer.
const WRITER_HEAP_BYTES: usize = 50_000_000;

/// Incrementally index new observations into the tantivy index.
pub(crate) async fn run_observation_index(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::ObservationIndex);
    run.start();

    let index_path =
        std::env::var("SEARCH_INDEX_PATH").unwrap_or_else(|_| "data/search".to_string());
    let index = match SearchIndex::open(Path::new(&index_path)) {
        Ok(index) => index,
        Err(error) => {
            run.fail(&format!("opening search index at {index_path}: {error}"));
            return run;
        }
    };

    let after = index
        .checkpoint()
        .and_then(|checkpoint| checkpoint.high_water_ts);
    let rows = match store
        .observations_for_indexing(after, INDEX_BATCH_LIMIT)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            run.fail(&format!("loading observations for indexing: {error}"));
            return run;
        }
    };
    if rows.is_empty() {
        run.skip("search index is current");
        return run;
    }

    match index_observation_batch(&index, &rows) {
        Ok(checkpoint) => {
            let high_water = checkpoint
                .high_water_ts
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| "none".to_string());
            run.succeed(
                checkpoint.indexed_documents,
                &format!(
                    "indexed {} observations (high-water {high_water})",
                    checkpoint.indexed_documents
                ),
            );
        }
        Err(error) => run.fail(&format!("indexing observations: {error}")),
    }
    run
}

/// Index one bounded batch, commit it, and persist the commit checkpoint.
///
/// The checkpoint's high-water mark is the newest observation timestamp in the
/// batch (rows are ordered oldest-first), which readiness then compares against
/// `observations.MAX(ts_utc)`.
pub(crate) fn index_observation_batch(
    index: &SearchIndex,
    rows: &[ObservationRow],
) -> Result<IndexCheckpoint> {
    let mut writer = index.writer(WRITER_HEAP_BYTES)?;
    let mut high_water: Option<DateTime<Utc>> = None;
    for row in rows {
        let entity_id = row
            .entity_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| row.id.to_string());
        let body: String = row.value.to_string().chars().take(MAX_BODY_CHARS).collect();
        index.index_document(
            &writer,
            &row.id.to_string(),
            "observation",
            &entity_id,
            &row.observation_type,
            &body,
            "",
            "",
            &[],
            row.ts_utc.timestamp(),
        )?;
        high_water = Some(match high_water {
            Some(previous) => previous.max(row.ts_utc),
            None => row.ts_utc,
        });
    }
    writer.commit().context("committing search index batch")?;
    // Reload so the handle that produced the commit observes it immediately
    // (in-memory indexes use a manual reload policy; on-disk indexes reload on
    // commit with delay, but an explicit reload keeps callers deterministic).
    index
        .reload()
        .context("reloading search index after commit")?;

    let checkpoint = IndexCheckpoint {
        last_commit_at: Utc::now(),
        high_water_ts: high_water,
        indexed_documents: rows.len() as u64,
    };
    index.record_checkpoint(&checkpoint)?;
    Ok(checkpoint)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn observation_row(ts: DateTime<Utc>) -> ObservationRow {
        ObservationRow {
            id: Uuid::new_v4(),
            observation_type: "news".to_string(),
            entity_id: Some(Uuid::new_v4()),
            entity_type: Some("company".to_string()),
            ts_utc: ts,
            value: json!({"content": "hello index"}),
            provenance: json!({}),
            confidence: Some(1.0),
            created_at: None,
        }
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid timestamp")
    }

    #[test]
    fn batch_commit_records_high_water_checkpoint() {
        let index = SearchIndex::in_memory().unwrap();
        let rows = vec![
            observation_row(ts(1_700_000_000)),
            observation_row(ts(1_700_000_100)),
        ];

        let checkpoint = index_observation_batch(&index, &rows).unwrap();

        assert_eq!(index.num_docs(), 2);
        assert_eq!(checkpoint.indexed_documents, 2);
        assert_eq!(checkpoint.high_water_ts, Some(ts(1_700_000_100)));
        assert_eq!(
            index.checkpoint().and_then(|cp| cp.high_water_ts),
            Some(ts(1_700_000_100)),
            "the committed checkpoint must advance the high-water mark"
        );
    }

    #[test]
    fn reindexing_the_boundary_row_does_not_duplicate_documents() {
        let index = SearchIndex::in_memory().unwrap();
        let rows = vec![
            observation_row(ts(1_700_000_000)),
            observation_row(ts(1_700_000_100)),
        ];

        index_observation_batch(&index, &rows).unwrap();
        index_observation_batch(&index, &rows).unwrap();

        assert_eq!(index.num_docs(), 2, "same ids must replace, not duplicate");
    }

    #[test]
    fn empty_batch_is_not_committed() {
        let index = SearchIndex::in_memory().unwrap();
        let checkpoint = index_observation_batch(&index, &[]).unwrap();
        assert_eq!(index.num_docs(), 0);
        assert_eq!(checkpoint.high_water_ts, None);
    }
}
