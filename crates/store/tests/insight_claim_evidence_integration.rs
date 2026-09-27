//! Opt-in integration test for claim-level evidence integrity
//! (audit P0 #23, migration 061).
//!
//! Proves the database rejects fabricated provenance:
//!   * an `observed` claim with no evidence link,
//!   * an evidence link to an observation id that does not exist,
//!   * an `observed` claim that never receives links (deferred guard).
//!
//! It also proves the application path validates ids, downgrades claims whose
//! cited evidence does not exist, and round-trips real evidence through the
//! normalized join table.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::claims::{ClaimKind, InsightClaim};
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
    .bind(format!("claim-integrity-test-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert test insight")
}

async fn new_observation(pool: &PgPool) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO observations (observation_type, ts_utc, value) \
         VALUES ('test', NOW(), '{}'::jsonb) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert test observation")
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db) => db.code().map(|code| code.into_owned()),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn observed_claim_without_evidence_is_rejected() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let insight_id = new_insight(&pool).await;

    let error = sqlx::query(
        "INSERT INTO insight_claims \
             (insight_id, claim, evidence_ids, confidence, claim_kind, claim_hash, evidence_count) \
         VALUES ($1, 'observed fact with no evidence', '{}'::uuid[], 0.9, 'observed', md5($2), 0)",
    )
    .bind(insight_id)
    .bind("observed fact with no evidence")
    .execute(&pool)
    .await
    .expect_err("CHECK must reject an observed claim with zero evidence");

    assert_eq!(
        sqlstate(&error).as_deref(),
        Some("23514"),
        "expected check_violation, got {error}"
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
async fn imaginary_evidence_id_is_rejected_by_foreign_key() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let insight_id = new_insight(&pool).await;

    let claim_id: Uuid = sqlx::query_scalar(
        "INSERT INTO insight_claims \
             (insight_id, claim, evidence_ids, confidence, claim_kind, claim_hash, evidence_count) \
         VALUES ($1, 'claim pending evidence', '{}'::uuid[], NULL, 'unknown', md5($2), 0) \
         RETURNING id",
    )
    .bind(insight_id)
    .bind("claim pending evidence")
    .fetch_one(&pool)
    .await
    .expect("insert unknown claim");

    let imaginary = Uuid::new_v4();
    let error =
        sqlx::query("INSERT INTO insight_claim_evidence (claim_id, evidence_id) VALUES ($1, $2)")
            .bind(claim_id)
            .bind(imaginary)
            .execute(&pool)
            .await
            .expect_err("FK must reject an evidence id that is not an observation");

    assert_eq!(
        sqlstate(&error).as_deref(),
        Some("23503"),
        "expected foreign_key_violation, got {error}"
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
async fn observed_claim_that_never_receives_links_is_rejected_at_commit() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let insight_id = new_insight(&pool).await;

    let mut tx = pool.begin().await.expect("begin tx");
    // evidence_count deliberately claims evidence while no join row exists:
    // the CHECK passes, so the deferred cross-table guard must catch it at
    // COMMIT.
    sqlx::query(
        "INSERT INTO insight_claims \
             (insight_id, claim, evidence_ids, confidence, claim_kind, claim_hash, evidence_count) \
         VALUES ($1, 'phantom evidence', '{}'::uuid[], 0.9, 'observed', md5($2), 3)",
    )
    .bind(insight_id)
    .bind("phantom evidence")
    .execute(&mut *tx)
    .await
    .expect("row-level insert passes the CHECK");

    let error = tx
        .commit()
        .await
        .expect_err("deferred guard must reject the commit");
    assert_eq!(
        sqlstate(&error).as_deref(),
        Some("23514"),
        "expected check_violation from the deferred guard, got {error}"
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
async fn application_path_links_real_evidence_and_drops_imaginary_ids() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let store = PgStore { pool: pool.clone() };
    let insight_id = new_insight(&pool).await;
    let observation_id = new_observation(&pool).await;

    let claims = vec![
        InsightClaim::new(
            "Real observed fact",
            vec![observation_id],
            Some(0.9),
            ClaimKind::Observed,
        ),
        InsightClaim::new(
            "Claim citing imaginary evidence",
            vec![Uuid::new_v4()],
            Some(0.8),
            ClaimKind::Observed,
        ),
        InsightClaim::new("Do this next", Vec::new(), None, ClaimKind::Recommendation),
    ];

    let inserted = store
        .insert_insight_claims(insight_id, &claims)
        .await
        .expect("insert claims");
    assert_eq!(inserted, 3);

    let rows = store
        .list_insight_claims(insight_id)
        .await
        .expect("list claims");
    assert_eq!(rows.len(), 3);

    let observed = rows
        .iter()
        .find(|row| row.claim == "Real observed fact")
        .expect("observed claim persisted");
    assert_eq!(observed.claim_kind, "observed");
    assert_eq!(observed.evidence_ids, vec![observation_id]);
    assert_eq!(observed.evidence_count, 1);

    let downgraded = rows
        .iter()
        .find(|row| row.claim == "Claim citing imaginary evidence")
        .expect("downgraded claim persisted");
    assert_eq!(
        downgraded.claim_kind, "unknown",
        "a claim whose only cited evidence does not exist must be downgraded, not fabricated"
    );
    assert!(downgraded.evidence_ids.is_empty());

    let recommendation = rows
        .iter()
        .find(|row| row.claim == "Do this next")
        .expect("recommendation persisted");
    assert_eq!(recommendation.claim_kind, "recommendation");
    assert!(recommendation.evidence_ids.is_empty());

    // Re-running is idempotent: no new claim rows, links stay intact.
    let inserted_again = store
        .insert_insight_claims(insight_id, &claims)
        .await
        .expect("re-insert claims");
    assert_eq!(inserted_again, 0);
    let rows_again = store
        .list_insight_claims(insight_id)
        .await
        .expect("list claims again");
    let observed_again = rows_again
        .iter()
        .find(|row| row.claim == "Real observed fact")
        .expect("observed claim still present");
    assert_eq!(observed_again.evidence_ids, vec![observation_id]);

    // The join table, not the legacy array, is authoritative.
    let join_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM insight_claim_evidence WHERE claim_id = $1 AND evidence_id = $2",
    )
    .bind(observed.id)
    .bind(observation_id)
    .fetch_one(&pool)
    .await
    .expect("join row exists");
    assert_eq!(join_count, 1);

    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn unknown_claim_with_evidence_is_sanitized_to_no_links() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    let store = PgStore { pool: pool.clone() };
    let insight_id = new_insight(&pool).await;
    let observation_id = new_observation(&pool).await;

    let claims = vec![InsightClaim::new(
        "Unverifiable assertion",
        vec![observation_id],
        None,
        ClaimKind::Unknown,
    )];
    store
        .insert_insight_claims(insight_id, &claims)
        .await
        .expect("insert claims");

    let rows = store
        .list_insight_claims(insight_id)
        .await
        .expect("list claims");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].claim_kind, "unknown");
    assert!(
        rows[0].evidence_ids.is_empty(),
        "unknown claims must not cite evidence"
    );

    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}
