//! Opt-in integration test for the evidence lineage graph (audit
//! evidence-graph item, migration 079).
//!
//! Proves the real pipeline persists a traceable chain when claims are
//! written — `source_document -> extraction -> observation -> claim ->
//! insight` — and that the read API walks it upstream from an insight back to
//! the source document. Also proves re-recording is idempotent.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use apex_core::claims::{ClaimKind, InsightClaim};
use apex_core::lineage::{LineageNode, LineageStage};
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

async fn new_insight(pool: &PgPool) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO insights (insight_type, title) VALUES ('test', $1) RETURNING id",
    )
    .bind(format!("lineage-test-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert test insight")
}

async fn new_observation(pool: &PgPool, url: &str, content_hash: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO observations (observation_type, ts_utc, value, provenance, confidence) \
         VALUES ('test', NOW(), '{}'::jsonb, $1::jsonb, 0.9) RETURNING id",
    )
    .bind(serde_json::json!({
        "url": url,
        "fetch_ts": "2026-09-27T00:00:00Z",
        "content_hash": content_hash,
        "extractor_version": "test_extractor_v1",
    }))
    .fetch_one(pool)
    .await
    .expect("insert test observation")
}

/// Walk upstream from `start` through `edges` (target -> source) and return
/// the visited nodes in traversal order.
fn walk_upstream<'a>(
    start: &str,
    nodes_by_id: &HashMap<&'a str, &'a LineageNode>,
    edges: &'a [apex_core::lineage::LineageEdge],
) -> Vec<&'a LineageNode> {
    let mut path = Vec::new();
    let mut current = start.to_string();
    for _ in 0..16 {
        let Some(node) = nodes_by_id.get(current.as_str()) else {
            break;
        };
        path.push(*node);
        let Some(edge) = edges.iter().find(|edge| edge.target_node == current) else {
            break;
        };
        current = edge.source_node.clone();
    }
    path
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn lineage_read_traverses_observation_to_source_document() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let store = PgStore::from_pool(pool.clone());

    let insight_id = new_insight(&pool).await;
    let document_url = format!("https://evidence.example.com/lineage-{}", Uuid::new_v4());
    let content_hash = format!("sha256-{}", Uuid::new_v4());
    let observation_id = new_observation(&pool, &document_url, &content_hash).await;

    let claims = vec![InsightClaim::new(
        "Seeded observed claim",
        vec![observation_id],
        Some(0.9),
        ClaimKind::Observed,
    )];
    store
        .insert_insight_claims(insight_id, &claims)
        .await
        .expect("persist claims");

    // The read API must find the graph recorded by the real pipeline.
    let trace = store
        .load_lineage_trace(LineageStage::Insight, &insight_id.to_string(), 16)
        .await
        .expect("load lineage trace")
        .expect("insight lineage is recorded");

    assert!(
        trace.contains_reference(LineageStage::Observation, &observation_id.to_string()),
        "trace must include the cited observation"
    );
    assert!(
        trace
            .source_documents
            .iter()
            .any(|document| document.url == document_url
                && document.content_digest.as_deref() == Some(content_hash.as_str())),
        "trace must surface the source document behind the observation"
    );

    // Traverse insight -> claim -> observation -> source_document using the
    // edge endpoints and the node ids returned by the read API.
    let nodes_by_id: HashMap<&str, &LineageNode> = trace
        .nodes
        .iter()
        .filter_map(|node| node.id.as_deref().map(|id| (id, node)))
        .collect();
    let insight_node = trace
        .nodes
        .iter()
        .find(|node| node.stage == LineageStage::Insight)
        .expect("insight node present");
    let path = walk_upstream(
        insight_node.id.as_deref().expect("loaded node has id"),
        &nodes_by_id,
        &trace.edges,
    );
    let stages: Vec<LineageStage> = path.iter().map(|node| node.stage).collect();
    assert_eq!(
        stages,
        vec![
            LineageStage::Insight,
            LineageStage::Claim,
            LineageStage::Observation,
            LineageStage::SourceDocument,
        ],
        "lineage read must traverse insight -> claim -> observation -> source document"
    );
    let document_node = path.last().expect("document node");
    assert_eq!(document_node.reference, document_url);
    assert_eq!(
        document_node.input_digest.as_deref(),
        Some(content_hash.as_str())
    );

    // Re-recording the same claims is idempotent: no duplicated nodes/edges.
    let node_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evidence_lineage_nodes")
        .fetch_one(&pool)
        .await
        .expect("count nodes");
    store
        .insert_insight_claims(insight_id, &claims)
        .await
        .expect("re-insert claims");
    let node_count_again: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evidence_lineage_nodes")
        .fetch_one(&pool)
        .await
        .expect("count nodes again");
    assert_eq!(
        node_count, node_count_again,
        "re-recording lineage must not duplicate nodes"
    );

    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn lineage_read_returns_none_for_unrecorded_insight() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let store = PgStore::from_pool(pool.clone());
    let insight_id = new_insight(&pool).await;

    let trace = store
        .load_lineage_trace(LineageStage::Insight, &insight_id.to_string(), 16)
        .await
        .expect("load lineage trace");
    assert!(
        trace.is_none(),
        "an insight with no recorded lineage must not fabricate a graph"
    );

    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}
