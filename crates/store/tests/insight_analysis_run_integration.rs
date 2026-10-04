//! Opt-in integration test for durable insight-analysis runs (#169,
//! migration 104).
//!
//! Proves the durable pipeline against a real database:
//!   * `enqueue_insight_analysis` commits a queued run and a payload-carrying
//!     trigger in one transaction, and a second enqueue for the same insight
//!     deduplicates onto the in-flight run without writing a second trigger;
//!   * `pop_job_trigger_with_payload` hands the worker the payload naming the
//!     exact run, so two queued runs cannot be executed out of order;
//!   * the run lifecycle only moves `queued → running → succeeded|failed`,
//!     a claim has a single winner, and terminal rows are never overwritten;
//!   * a run abandoned by a crashed worker is expired so the in-flight
//!     partial unique index cannot block the insight forever;
//!   * the existing kind-only trigger dedup is unaffected by payload triggers.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::PgStore;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

async fn connect() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres")
}

async fn migrate(pool: &PgPool) {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .expect("migrations apply");
}

async fn new_insight(pool: &PgPool) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO insights (insight_type, title) VALUES ('test', $1) RETURNING id",
    )
    .bind(format!("insight-analysis-run-test-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert test insight")
}

/// Remove this insight's triggers and the insight itself (runs cascade).
async fn cleanup(pool: &PgPool, insight_id: Uuid) {
    sqlx::query(
        "DELETE FROM worker_trigger_queue \
         WHERE job_kind = 'insight_analysis' AND payload->>'insight_id' = $1",
    )
    .bind(insight_id.to_string())
    .execute(pool)
    .await
    .expect("delete test trigger");
    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(pool)
        .await
        .expect("delete insight cascades runs");
}

async fn trigger_count(pool: &PgPool, insight_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM worker_trigger_queue \
         WHERE job_kind = 'insight_analysis' AND payload->>'insight_id' = $1 \
           AND completed_at IS NULL",
    )
    .bind(insight_id.to_string())
    .fetch_one(pool)
    .await
    .expect("count triggers")
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn enqueue_creates_a_payload_trigger_and_deduplicates() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());
    let insight_id = new_insight(&pool).await;

    let (run, deduplicated) = store
        .enqueue_insight_analysis(insight_id, Some("integration-test"))
        .await
        .expect("enqueue analysis run");
    assert!(!deduplicated, "first enqueue creates a run");
    assert_eq!(run.status, "queued");
    assert_eq!(run.insight_id, insight_id);
    assert_eq!(run.requested_by.as_deref(), Some("integration-test"));
    assert_eq!(trigger_count(&pool, insight_id).await, 1);

    let (second, deduplicated) = store
        .enqueue_insight_analysis(insight_id, Some("integration-test"))
        .await
        .expect("deduplicate analysis run");
    assert!(deduplicated, "an in-flight run must be reused");
    assert_eq!(second.id, run.id, "the same run id is returned");
    assert_eq!(
        trigger_count(&pool, insight_id).await,
        1,
        "a deduplicated enqueue must not write a second trigger"
    );

    cleanup(&pool, insight_id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn pop_returns_the_payload_for_the_exact_queued_run() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    // Isolate this test's queue from any trigger another suite left behind.
    sqlx::query("DELETE FROM worker_trigger_queue WHERE completed_at IS NULL")
        .execute(&pool)
        .await
        .expect("clear pending triggers");

    let first_insight = new_insight(&pool).await;
    let second_insight = new_insight(&pool).await;
    let (first_run, _) = store
        .enqueue_insight_analysis(first_insight, None)
        .await
        .expect("enqueue first run");
    let (second_run, _) = store
        .enqueue_insight_analysis(second_insight, None)
        .await
        .expect("enqueue second run");

    let (trigger_id, kind, payload) = store
        .pop_job_trigger_with_payload()
        .await
        .expect("pop first trigger")
        .expect("first trigger exists");
    assert_eq!(kind, "insight_analysis");
    let payload = payload.expect("payload-bound trigger carries its payload");
    assert_eq!(payload["run_id"], first_run.id.to_string());
    assert_eq!(payload["insight_id"], first_insight.to_string());

    store
        .complete_job_trigger(&trigger_id, None)
        .await
        .expect("complete first trigger");

    let (_, _, second_payload) = store
        .pop_job_trigger_with_payload()
        .await
        .expect("pop second trigger")
        .expect("second trigger exists");
    let second_payload = second_payload.expect("second payload present");
    assert_eq!(second_payload["run_id"], second_run.id.to_string());
    assert_eq!(second_payload["insight_id"], second_insight.to_string());

    cleanup(&pool, first_insight).await;
    cleanup(&pool, second_insight).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn run_lifecycle_queued_running_succeeded() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());
    let insight_id = new_insight(&pool).await;

    let (run, _) = store
        .create_insight_analysis_run(insight_id, None)
        .await
        .expect("create run");
    assert_eq!(run.status, "queued");
    assert!(run.started_at.is_none());

    let claimed = store
        .claim_insight_analysis_run(run.id)
        .await
        .expect("claim run")
        .expect("queued run is claimable");
    assert_eq!(claimed.status, "running");
    assert!(claimed.started_at.is_some());

    assert!(
        store
            .claim_insight_analysis_run(run.id)
            .await
            .expect("second claim")
            .is_none(),
        "a claimed run has exactly one winner"
    );

    let result = serde_json::json!({"analysis": {"executive_summary": "done"}});
    assert!(
        store
            .complete_insight_analysis_run(run.id, &result)
            .await
            .expect("complete run"),
        "a running run completes"
    );
    assert!(
        !store
            .complete_insight_analysis_run(run.id, &result)
            .await
            .expect("second complete"),
        "a terminal run is never overwritten"
    );

    let stored = store
        .get_insight_analysis_run(run.id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(stored.status, "succeeded");
    assert_eq!(stored.result.as_ref(), Some(&result));
    assert!(stored.error.is_none());
    assert!(stored.completed_at.is_some());

    let latest = store
        .get_latest_insight_analysis_run(insight_id)
        .await
        .expect("latest run")
        .expect("latest exists");
    assert_eq!(latest.id, run.id);

    cleanup(&pool, insight_id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn run_lifecycle_queued_running_failed() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());
    let insight_id = new_insight(&pool).await;

    let (run, _) = store
        .create_insight_analysis_run(insight_id, None)
        .await
        .expect("create run");
    store
        .claim_insight_analysis_run(run.id)
        .await
        .expect("claim run")
        .expect("queued run is claimable");

    assert!(store
        .fail_insight_analysis_run(run.id, "LLM analysis failed: timeout")
        .await
        .expect("fail run"));

    let stored = store
        .get_insight_analysis_run(run.id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(stored.status, "failed");
    assert_eq!(
        stored.error.as_deref(),
        Some("LLM analysis failed: timeout")
    );
    assert!(stored.result.is_none());
    assert!(stored.completed_at.is_some());

    cleanup(&pool, insight_id).await;
}

async fn age_run(pool: &PgPool, run_id: Uuid, age: &str) {
    // `started_at` (when set) is what the stale predicate measures for a
    // running run, so age both timestamps.
    sqlx::query(
        "UPDATE insight_analysis_runs \
         SET requested_at = NOW() - $2::interval, \
             started_at = CASE WHEN started_at IS NOT NULL \
                               THEN NOW() - $2::interval ELSE NULL END \
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(age)
    .execute(pool)
    .await
    .expect("age the run");
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn stale_runs_expire_without_racing_pending_work() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    // 1. A queued run whose trigger is still pending survives the sweep: a
    //    worker restart may leave it waiting longer than the threshold.
    let waiting_insight = new_insight(&pool).await;
    let (waiting, _) = store
        .enqueue_insight_analysis(waiting_insight, None)
        .await
        .expect("enqueue waiting run");
    age_run(&pool, waiting.id, "2 hours").await;

    // 2. A running run abandoned by a crash must expire.
    let running_insight = new_insight(&pool).await;
    let (running, _) = store
        .create_insight_analysis_run(running_insight, None)
        .await
        .expect("create running run");
    store
        .claim_insight_analysis_run(running.id)
        .await
        .expect("claim running run")
        .expect("queued run is claimable");
    age_run(&pool, running.id, "2 hours").await;

    // 3. A queued run whose trigger was completed without executing (the
    //    "already running" reconciliation path) must expire.
    let orphan_insight = new_insight(&pool).await;
    let (orphan, _) = store
        .enqueue_insight_analysis(orphan_insight, None)
        .await
        .expect("enqueue orphan run");
    let orphan_trigger: Uuid = sqlx::query_scalar(
        "SELECT id FROM worker_trigger_queue \
         WHERE job_kind = 'insight_analysis' AND payload->>'run_id' = $1",
    )
    .bind(orphan.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("orphan trigger");
    store
        .complete_job_trigger(&orphan_trigger.to_string(), Some("already running"))
        .await
        .expect("complete orphan trigger");
    age_run(&pool, orphan.id, "2 hours").await;

    let expired = store
        .expire_stale_insight_analysis_runs(60)
        .await
        .expect("expire stale runs");
    assert!(
        expired >= 2,
        "expected at least 2 expired runs, got {expired}"
    );

    let waiting = store
        .get_insight_analysis_run(waiting.id)
        .await
        .expect("get waiting run")
        .expect("waiting run exists");
    assert_eq!(
        waiting.status, "queued",
        "a queued run with a pending trigger must not be expired"
    );

    let running = store
        .get_insight_analysis_run(running.id)
        .await
        .expect("get running run")
        .expect("running run exists");
    assert_eq!(running.status, "failed");
    assert!(running
        .error
        .as_deref()
        .unwrap_or("")
        .contains("no progress"));

    let orphan = store
        .get_insight_analysis_run(orphan.id)
        .await
        .expect("get orphan run")
        .expect("orphan run exists");
    assert_eq!(orphan.status, "failed");

    // The expired insight is analyzable again: the in-flight index no longer
    // blocks a fresh run.
    let (fresh, deduplicated) = store
        .enqueue_insight_analysis(orphan_insight, None)
        .await
        .expect("re-enqueue after expiry");
    assert!(!deduplicated);
    assert_ne!(fresh.id, orphan.id);

    cleanup(&pool, waiting_insight).await;
    cleanup(&pool, running_insight).await;
    cleanup(&pool, orphan_insight).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn kind_only_trigger_dedup_is_unaffected_by_payloads() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());
    let kind = format!("plain_trigger_{}", Uuid::new_v4().simple());

    let first = store
        .queue_job_trigger(&kind)
        .await
        .expect("queue kind trigger");
    let second = store
        .queue_job_trigger(&kind)
        .await
        .expect("dedup kind trigger");
    assert_eq!(first, second, "kind-only dedup is unchanged");

    let payload = serde_json::json!({"run_id": Uuid::new_v4()});
    let payload_trigger = store
        .enqueue_job_trigger_with_payload(&kind, &payload)
        .await
        .expect("enqueue payload trigger for another kind");
    let payload_trigger_again = store
        .enqueue_job_trigger_with_payload(&kind, &payload)
        .await
        .expect("dedup payload trigger");
    assert_eq!(
        payload_trigger, payload_trigger_again,
        "same kind and payload deduplicate"
    );

    sqlx::query("DELETE FROM worker_trigger_queue WHERE job_kind = $1")
        .bind(&kind)
        .execute(&pool)
        .await
        .expect("cleanup triggers");
}
