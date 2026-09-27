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
#[cfg(test)]
use chrono::DateTime;
use chrono::Utc;

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

/// Wall-clock budget for one catch-up run; keeps the writer lock bounded.
const INDEX_RUN_BUDGET_SECS: u64 = 25;

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

    let batch_limit: i64 = std::env::var("INDEX_BATCH_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(INDEX_BATCH_LIMIT)
        .clamp(1, 50_000);

    // Catch up in bounded batches inside one run: a large backlog (or a batch
    // that shares a timestamp) converges instead of advancing 1 000 rows per
    // five-minute tick. The wall-clock budget keeps the writer lock bounded.
    let started = std::time::Instant::now();
    let budget = std::time::Duration::from_secs(INDEX_RUN_BUDGET_SECS);
    let mut total_indexed: u64 = 0;
    let mut batches: u32 = 0;
    let mut high_water_label = "none".to_string();

    loop {
        let checkpoint = index.checkpoint();
        let after_ts = checkpoint.as_ref().and_then(|cp| cp.high_water_ts);
        let after_id = checkpoint.as_ref().and_then(|cp| cp.high_water_id);
        let rows = match store
            .observations_for_indexing(after_ts, after_id, batch_limit)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                run.fail(&format!("loading observations for indexing: {error}"));
                return run;
            }
        };
        if rows.is_empty() {
            break;
        }
        match index_observation_batch(&index, &rows) {
            Ok(committed) => {
                total_indexed += committed.indexed_documents;
                batches += 1;
                if let Some(ts) = committed.high_water_ts {
                    high_water_label = ts.to_rfc3339();
                }
                if committed.indexed_documents < batch_limit as u64 {
                    break; // caught up
                }
            }
            Err(error) => {
                run.fail(&format!("indexing observations: {error}"));
                return run;
            }
        }
        if started.elapsed() >= budget {
            break;
        }
    }

    if total_indexed == 0 {
        run.skip("search index is current");
        return run;
    }
    run.succeed(
        total_indexed,
        &format!(
            "indexed {total_indexed} observations in {batches} batch(es) (high-water {high_water_label})"
        ),
    );
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
    }
    writer.commit().context("committing search index batch")?;
    // Reload so the handle that produced the commit observes it immediately
    // (in-memory indexes use a manual reload policy; on-disk indexes reload on
    // commit with delay, but an explicit reload keeps callers deterministic).
    index
        .reload()
        .context("reloading search index after commit")?;

    // The cursor is the *last* row's keyset (rows are ordered oldest-first),
    // which is correct even when many observations share one timestamp.
    let last = rows.last();
    let checkpoint = IndexCheckpoint {
        last_commit_at: Utc::now(),
        high_water_ts: last.map(|row| row.ts_utc),
        high_water_id: last.map(|row| row.id),
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
