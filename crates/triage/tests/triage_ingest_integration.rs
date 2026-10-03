//! Opt-in integration tests for the merge-aware triage ingress against a real
//! PostgreSQL database.
//!
//! The unit suite exercises the pipeline with an in-memory queue; these tests
//! prove the SQL side (migration `053_triage_merge_fields.sql`, the
//! `ON CONFLICT (item_type, source_id)` guard, and evidence accumulation).
//!
//! They are `#[ignore]`d by default and read `TEST_DATABASE_URL` or
//! `DATABASE_URL`:
//!
//! ```text
//! TEST_DATABASE_URL=postgres://... cargo test -p apex-triage -- --ignored
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use apex_core::triage::{TriageItemType, TriageStatus};
use apex_triage::semantic_dedup::SemanticDedup;
use apex_triage::{QueueEnqueueRequest, TriageIngestor, TriageQueue, TriageSubmission};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn connect() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    pool
}

fn submission(
    item_type: TriageItemType,
    source_id: &str,
    observation_id: Uuid,
    url: &str,
) -> TriageSubmission {
    TriageSubmission {
        item_type,
        source_id: source_id.to_string(),
        title: format!("Integration test item {source_id}"),
        description: format!("Repeated signal for {source_id} used to verify merge bookkeeping"),
        entity_id: None,
        entity_name: Some("Integration Test Corp".to_string()),
        static_severity: Some("low".to_string()),
        dimensions: None,
        observation_ids: vec![observation_id],
        source_urls: vec![url.to_string()],
    }
}

async fn delete_fixture(pool: &sqlx::PgPool, item_type: &TriageItemType, source_id: &str) {
    sqlx::query("DELETE FROM triage_queue WHERE item_type = $1 AND source_id = $2::uuid")
        .bind(item_type.as_str())
        .bind(source_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Remove rows left behind by aborted earlier runs so similarity stages cannot
/// merge into stale fixtures. Scoped to one item type: the DB tests run in
/// parallel and must not delete each other's rows.
async fn purge_stale_fixtures(pool: &sqlx::PgPool, item_type: &TriageItemType) {
    sqlx::query(
        "DELETE FROM triage_queue WHERE item_type = $1 \
         AND (title LIKE 'Integration test item%' \
              OR title IN ('Foxconn quality crisis', 'Samsung fab investment'))",
    )
    .bind(item_type.as_str())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn duplicate_submits_merge_in_the_database() {
    let pool = connect().await;
    let ingestor = TriageIngestor::new(
        TriageQueue::new(pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    );

    let item_type = TriageItemType::Warning;
    purge_stale_fixtures(&pool, &item_type).await;

    let source_id = Uuid::new_v4().to_string();
    let obs_a = Uuid::new_v4();
    let obs_b = Uuid::new_v4();

    let first = ingestor
        .submit(submission(
            item_type.clone(),
            &source_id,
            obs_a,
            "https://news-a.example/story",
        ))
        .await
        .unwrap();
    assert!(!first.merged(), "first submission must insert");
    assert_eq!(first.item().occurrence_count, 1);

    let second = ingestor
        .submit(submission(
            item_type.clone(),
            &source_id,
            obs_b,
            "https://news-b.example/story",
        ))
        .await
        .unwrap();
    assert!(second.merged(), "repeat submission must merge");
    assert_eq!(second.item().occurrence_count, 2);

    let rows: Vec<(i64, Vec<Uuid>, Vec<String>)> = sqlx::query_as(
        "SELECT occurrence_count::bigint, merged_observation_ids, merged_source_urls \
         FROM triage_queue WHERE item_type = $1 AND source_id = $2::uuid",
    )
    .bind(item_type.as_str())
    .bind(&source_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 1, "duplicate submits must not double-insert");
    let (count, observations, urls) = &rows[0];
    assert_eq!(*count, 2);
    assert_eq!(observations.len(), 2, "observation evidence merged");
    assert!(observations.contains(&obs_a));
    assert!(observations.contains(&obs_b));
    assert_eq!(urls.len(), 2, "source urls merged");
    assert!(urls.iter().any(|u| u.contains("news-a.example")));
    assert!(urls.iter().any(|u| u.contains("news-b.example")));

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn concurrent_duplicate_submits_single_row_in_the_database() {
    let pool = connect().await;
    let ingestor = Arc::new(TriageIngestor::new(
        TriageQueue::new(pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    ));

    let item_type = TriageItemType::Alert;
    purge_stale_fixtures(&pool, &item_type).await;

    let source_id = Uuid::new_v4().to_string();

    let mut handles = Vec::new();
    for i in 0..4 {
        let ingestor = Arc::clone(&ingestor);
        let source_id = source_id.clone();
        let item_type = item_type.clone();
        handles.push(tokio::spawn(async move {
            let obs = Uuid::new_v4();
            ingestor
                .submit(submission(
                    item_type,
                    &source_id,
                    obs,
                    &format!("https://news-{i}.example/story"),
                ))
                .await
        }));
    }

    let mut merged = 0;
    for handle in handles {
        let outcome = handle.await.unwrap().unwrap();
        if outcome.merged() {
            merged += 1;
        }
    }

    let rows: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT occurrence_count::bigint, \
                COALESCE(array_length(merged_observation_ids, 1), 0)::bigint, \
                COALESCE(static_severity, '') \
         FROM triage_queue WHERE item_type = $1 AND source_id = $2::uuid",
    )
    .bind(item_type.as_str())
    .bind(&source_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 1, "concurrent duplicates must insert once");
    assert_eq!(rows[0].0, 4, "every submission must be counted");
    assert_eq!(rows[0].1, 4, "every observation id must be merged");
    assert_eq!(
        rows[0].2, "high",
        "4 repeats must atomically escalate low -> high"
    );
    assert_eq!(merged, 3, "all but the first submission merge");

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn distinct_items_enqueue_separately_in_the_database() {
    let pool = connect().await;
    let ingestor = TriageIngestor::new(
        TriageQueue::new(pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    );

    let item_type = TriageItemType::Insight;
    purge_stale_fixtures(&pool, &item_type).await;

    let source_a = Uuid::new_v4().to_string();
    let source_b = Uuid::new_v4().to_string();

    let mut a = submission(
        item_type.clone(),
        &source_a,
        Uuid::new_v4(),
        "https://news-a.example/1",
    );
    a.title = "Foxconn quality crisis".to_string();
    a.description = "Recall at the Tunisia plant".to_string();

    let mut b = submission(
        item_type.clone(),
        &source_b,
        Uuid::new_v4(),
        "https://news-b.example/2",
    );
    b.title = "Samsung fab investment".to_string();
    b.description = "New semiconductor investment in Korea".to_string();

    assert!(!ingestor.submit(a).await.unwrap().merged());
    assert!(!ingestor.submit(b).await.unwrap().merged());

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM triage_queue \
         WHERE item_type = $1 AND source_id IN ($2::uuid, $3::uuid)",
    )
    .bind(item_type.as_str())
    .bind(&source_a)
    .bind(&source_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 2);

    delete_fixture(&pool, &item_type, &source_a).await;
    delete_fixture(&pool, &item_type, &source_b).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn resolved_items_are_not_merge_targets_in_the_database() {
    let pool = connect().await;
    let ingestor = TriageIngestor::new(
        TriageQueue::new(pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    );

    let item_type = TriageItemType::Warning;
    purge_stale_fixtures(&pool, &item_type).await;
    let entity_id = Uuid::new_v4();

    let source_a = Uuid::new_v4().to_string();
    let mut first = submission(
        item_type.clone(),
        &source_a,
        Uuid::new_v4(),
        "https://news-a.example/story",
    );
    first.entity_id = Some(entity_id);
    first.title = "Initial signal for the entity".to_string();
    assert!(!ingestor.submit(first).await.unwrap().merged());

    let (row_id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM triage_queue WHERE item_type = $1 AND source_id = $2::uuid")
            .bind(item_type.as_str())
            .bind(&source_a)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE triage_queue SET status = 'resolved' WHERE id = $1")
        .bind(row_id)
        .execute(&pool)
        .await
        .unwrap();

    // Same entity within the window, clearly different text: the resolved row
    // must not absorb the new signal.
    let source_b = Uuid::new_v4().to_string();
    let mut second = submission(
        item_type.clone(),
        &source_b,
        Uuid::new_v4(),
        "https://news-b.example/story",
    );
    second.entity_id = Some(entity_id);
    second.title = "Entirely different headline wording".to_string();
    second.description = "Unrelated sentence sharing almost no trigrams".to_string();

    let outcome = ingestor.submit(second).await.unwrap();
    assert!(
        !outcome.merged(),
        "resolved rows must not absorb new signals"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM triage_queue \
         WHERE item_type = $1 AND source_id IN ($2::uuid, $3::uuid)",
    )
    .bind(item_type.as_str())
    .bind(&source_a)
    .bind(&source_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "the new signal must enqueue as its own row");

    delete_fixture(&pool, &item_type, &source_a).await;
    delete_fixture(&pool, &item_type, &source_b).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn enqueue_records_repeats_like_the_ingress_in_the_database() {
    // Both write paths must share merge bookkeeping: calling `enqueue` for an
    // already-queued (item_type, source_id) increments the occurrence count and
    // escalates severity instead of silently overwriting the row.
    let pool = connect().await;
    let queue = TriageQueue::new(pool.clone());

    let item_type = TriageItemType::Insight;
    purge_stale_fixtures(&pool, &item_type).await;
    let source_id = Uuid::new_v4().to_string();

    for _ in 0..3 {
        queue
            .enqueue(QueueEnqueueRequest {
                item_type: item_type.clone(),
                source_id: &source_id,
                title: "Enqueue repeat check",
                description: "body",
                entity_id: None,
                entity_name: None,
                static_severity: Some("low"),
                dimensions: None,
            })
            .await
            .unwrap();
    }

    let (count, severity): (i64, String) = sqlx::query_as(
        "SELECT occurrence_count::bigint, COALESCE(static_severity, '') \
         FROM triage_queue WHERE item_type = $1 AND source_id = $2::uuid",
    )
    .bind(item_type.as_str())
    .bind(&source_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 3, "enqueue must count repeats");
    assert_eq!(severity, "high", "3 repeats must escalate low -> high");

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}

/// `list`/`count` back the `/triage` page. `triage_queue.status` is TEXT; a
/// cast to a nonexistent `triage_status` type made both fail on every call.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn list_and_count_run_against_the_real_schema() {
    let pool = connect().await;
    let queue = TriageQueue::new(pool.clone());
    let ingestor = TriageIngestor::new(
        TriageQueue::new(pool.clone()),
        SemanticDedup::with_in_memory_fallback(),
    );

    let item_type = TriageItemType::Insight;
    purge_stale_fixtures(&pool, &item_type).await;
    let source_id = Uuid::new_v4().to_string();
    ingestor
        .submit(submission(
            item_type.clone(),
            &source_id,
            Uuid::new_v4(),
            "https://list-count.example/story",
        ))
        .await
        .unwrap();

    let all = queue.list(None, 500, 0).await.expect("unfiltered list");
    assert!(all.iter().any(|item| item.source_id == source_id));
    let pending = queue
        .list(Some(TriageStatus::Pending), 500, 0)
        .await
        .expect("status-filtered list");
    assert!(pending.iter().any(|item| item.source_id == source_id));
    assert!(pending
        .iter()
        .all(|item| item.status == TriageStatus::Pending));

    assert!(queue.count(None).await.expect("unfiltered count") >= 1);
    assert!(
        queue
            .count(Some(TriageStatus::Pending))
            .await
            .expect("status-filtered count")
            >= 1
    );
    assert_eq!(
        queue
            .count(Some(TriageStatus::Dismissed))
            .await
            .expect("count for an empty status"),
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM triage_queue WHERE status = 'dismissed'"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    );

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn enqueue_without_dimensions_preserves_scores_and_reopens_closed_rows() {
    let pool = connect().await;
    let queue = TriageQueue::new(pool.clone());

    let item_type = TriageItemType::Warning;
    purge_stale_fixtures(&pool, &item_type).await;
    let source_id = Uuid::new_v4().to_string();

    let dims = apex_core::triage::TriageDimensions {
        urgency: 0.8,
        impact: 0.9,
        actionability: 0.6,
        novelty: 0.4,
        confidence: 0.7,
    };

    let first = queue
        .enqueue(QueueEnqueueRequest {
            item_type: item_type.clone(),
            source_id: &source_id,
            title: "Initial sighting",
            description: "First observation of the signal",
            entity_id: None,
            entity_name: Some("Integration Test Corp"),
            static_severity: Some("low"),
            dimensions: Some(&dims),
        })
        .await
        .unwrap();
    assert_eq!(first.status, TriageStatus::Pending);
    let original_dimensions = first.dimensions.clone().expect("dimensions stored");
    let original_score = first.composite_score;

    // Re-enqueue the same sighting with no dimensions: the stored score must
    // not be zeroed out (#111).
    let repeated = queue
        .enqueue(QueueEnqueueRequest {
            item_type: item_type.clone(),
            source_id: &source_id,
            title: "Repeat sighting",
            description: "Second observation of the signal",
            entity_id: None,
            entity_name: Some("Integration Test Corp"),
            static_severity: Some("low"),
            dimensions: None,
        })
        .await
        .unwrap();
    let repeated_dimensions = repeated.dimensions.clone().expect("dimensions kept");
    assert!(
        (repeated_dimensions.urgency - original_dimensions.urgency).abs() < 1e-9,
        "urgency must be preserved when no new dimensions are supplied"
    );
    assert!(
        (repeated.composite_score - original_score).abs() < 1e-9,
        "composite score must be preserved when no new dimensions are supplied"
    );
    assert_eq!(repeated.status, TriageStatus::Pending);

    // Close the row, then enqueue the same sighting again: it must reopen
    // (#112) with its score intact.
    queue.resolve(repeated.id).await.unwrap();
    let reopened = queue
        .enqueue(QueueEnqueueRequest {
            item_type: item_type.clone(),
            source_id: &source_id,
            title: "Third sighting",
            description: "Third observation of the signal",
            entity_id: None,
            entity_name: Some("Integration Test Corp"),
            static_severity: Some("low"),
            dimensions: None,
        })
        .await
        .unwrap();
    assert_eq!(
        reopened.status,
        TriageStatus::Triaged,
        "a new sighting must reopen a resolved row"
    );
    assert!(
        (reopened.composite_score - original_score).abs() < 1e-9,
        "reopening without new dimensions keeps the stored score"
    );

    // New dimensions still overwrite the stored ones.
    let new_dims = apex_core::triage::TriageDimensions {
        urgency: 0.1,
        impact: 0.2,
        actionability: 0.3,
        novelty: 0.4,
        confidence: 0.5,
    };
    let rescored = queue
        .enqueue(QueueEnqueueRequest {
            item_type: item_type.clone(),
            source_id: &source_id,
            title: "Fourth sighting",
            description: "Fourth observation with fresh scoring",
            entity_id: None,
            entity_name: Some("Integration Test Corp"),
            static_severity: Some("low"),
            dimensions: Some(&new_dims),
        })
        .await
        .unwrap();
    let rescored_dimensions = rescored.dimensions.clone().expect("dimensions stored");
    assert!(
        (rescored_dimensions.urgency - 0.1).abs() < 1e-9,
        "supplied dimensions must overwrite the stored score"
    );
    assert_eq!(rescored.status, TriageStatus::Triaged);

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn resubmitting_a_resolved_sighting_reopens_it() {
    let pool = connect().await;
    let queue = TriageQueue::new(pool.clone());
    let ingestor = TriageIngestor::new(queue, SemanticDedup::with_in_memory_fallback());

    let item_type = TriageItemType::Alert;
    purge_stale_fixtures(&pool, &item_type).await;
    let source_id = Uuid::new_v4().to_string();

    let first = ingestor
        .submit(submission(
            item_type.clone(),
            &source_id,
            Uuid::new_v4(),
            "https://news-a.example/reopen",
        ))
        .await
        .unwrap();
    assert!(!first.merged());
    let id = first.item().id;

    TriageQueue::new(pool.clone()).resolve(id).await.unwrap();

    // The same source producing a new sighting must reopen the closed row,
    // not merge into it invisibly (#112).
    let second = ingestor
        .submit(submission(
            item_type.clone(),
            &source_id,
            Uuid::new_v4(),
            "https://news-b.example/reopen",
        ))
        .await
        .unwrap();
    assert!(second.merged());

    let (status, occurrences): (String, i64) = sqlx::query_as(
        "SELECT status::text, occurrence_count::bigint FROM triage_queue WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "pending", "a new sighting must reopen the row");
    assert_eq!(occurrences, 2);

    delete_fixture(&pool, &item_type, &source_id).await;
    pool.close().await;
}
