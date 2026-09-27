//! Opt-in integration test for durable warning-analysis runs (audit P1-5,
//! migration 079).
//!
//! Proves the run pipeline's persistence invariants against a real database:
//!   * identical evidence digests deduplicate (one in-flight run, one
//!     succeeded run per prompt version),
//!   * a different digest or a bumped prompt version creates a comparable run,
//!   * validated claims persist through the claim-evidence integrity system
//!     (observed claims cite real observations, recommendations cite none),
//!   * fabricated provenance is rejected at the database level,
//!   * stale queued/running runs expire explicitly instead of hanging forever.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_core::claims::{AnalysisClaimRecord, ClaimKind, ClaimSection, EvidenceRef};
use apex_store::postgres::{NewWarningAnalysisRun, PgStore};
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

async fn new_warning(pool: &PgPool) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO warnings (warning_type, title, severity, ts_utc) \
         VALUES ('test', $1, 'high', NOW()) RETURNING id",
    )
    .bind(format!("analysis-run-test-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert test warning")
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

async fn cleanup(pool: &PgPool, warning_id: Uuid, observations: &[Uuid]) {
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(pool)
        .await
        .expect("delete warning cascades runs and claims");
    for observation_id in observations {
        sqlx::query("DELETE FROM observations WHERE id = $1")
            .bind(observation_id)
            .execute(pool)
            .await
            .expect("delete observation");
    }
}

fn request(warning_id: Uuid, digest: &str, prompt_version: &str) -> NewWarningAnalysisRun {
    NewWarningAnalysisRun {
        warning_id,
        requested_by: Some("integration-test".to_string()),
        model: "test-model".to_string(),
        prompt_version: prompt_version.to_string(),
        evidence_digest: digest.to_string(),
        observations_available: 3,
        observations_sent: 2,
        insights_available: 1,
        insights_sent: 1,
    }
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db) => db.code().map(|code| code.into_owned()),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn identical_evidence_digest_deduplicates_runs() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;

    let first = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-a", "v2"))
        .await
        .expect("enqueue first run");
    assert!(!first.1, "first enqueue must create a run");
    assert_eq!(first.0.status, "queued");

    let second = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-a", "v2"))
        .await
        .expect("enqueue duplicate run");
    assert!(second.1, "identical evidence must deduplicate");
    assert_eq!(first.0.id, second.0.id, "dedupe must reuse the same run");
    assert_eq!(second.0.observations_sent, 2);

    let different = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-b", "v2"))
        .await
        .expect("enqueue different-evidence run");
    assert!(!different.1);
    assert_ne!(first.0.id, different.0.id);

    cleanup(&pool, warning_id, &[]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn completed_runs_cache_by_prompt_version_and_persist_claims() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;
    let observation_id = new_observation(&pool).await;

    let (run, _) = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-c", "v2"))
        .await
        .expect("enqueue");
    assert!(store
        .start_warning_analysis_run(run.id)
        .await
        .expect("start run"));

    let claims = vec![
        AnalysisClaimRecord::new(
            "Observed component shortage",
            vec![EvidenceRef::Observation(observation_id)],
            Some(0.9),
            ClaimKind::Observed,
        )
        .with_section(ClaimSection::Claim),
        AnalysisClaimRecord::new(
            "Qualify a second supplier",
            Vec::new(),
            None,
            ClaimKind::Recommendation,
        )
        .with_section(ClaimSection::Action),
    ];
    let output = serde_json::json!({"schema_version": 2, "claims": []});
    let inserted = store
        .complete_warning_analysis_run(run.id, &output, &claims)
        .await
        .expect("complete run");
    assert_eq!(inserted, 2);

    let stored = store
        .get_warning_analysis_run(run.id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(stored.status, "succeeded");
    assert!(stored.output.is_some());

    let rows = store
        .list_warning_analysis_claims(run.id)
        .await
        .expect("list claims");
    assert_eq!(rows.len(), 2);
    let observed = rows
        .iter()
        .find(|row| row.claim == "Observed component shortage")
        .expect("observed claim persisted");
    assert_eq!(observed.claim_kind, "observed");
    assert_eq!(observed.section, "claim");
    assert_eq!(
        observed.evidence,
        vec![EvidenceRef::Observation(observation_id)]
    );
    let recommendation = rows
        .iter()
        .find(|row| row.claim == "Qualify a second supplier")
        .expect("recommendation persisted");
    assert_eq!(recommendation.claim_kind, "recommendation");
    assert_eq!(recommendation.section, "action");
    assert!(recommendation.evidence.is_empty());

    // Completing an already-succeeded run must not overwrite it.
    assert!(store
        .complete_warning_analysis_run(run.id, &output, &claims)
        .await
        .is_err());

    // Same evidence + same prompt version → cached succeeded run.
    let cached = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-c", "v2"))
        .await
        .expect("enqueue cached");
    assert!(cached.1);
    assert_eq!(cached.0.id, run.id);
    assert_eq!(cached.0.status, "succeeded");

    // Same evidence + new prompt version → a new comparable run.
    let bumped = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-c", "v3"))
        .await
        .expect("enqueue bumped prompt");
    assert!(!bumped.1);
    assert_ne!(bumped.0.id, run.id);

    cleanup(&pool, warning_id, &[observation_id]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn fabricated_provenance_is_rejected_and_fails_the_run() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;
    let observation_id = new_observation(&pool).await;

    let (run, _) = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-d", "v2"))
        .await
        .expect("enqueue");
    store
        .start_warning_analysis_run(run.id)
        .await
        .expect("start run");

    // Application path: a claim citing a non-existent observation is rejected
    // (never downgraded silently) and the run stays running until failed.
    let imaginary = vec![AnalysisClaimRecord::new(
        "Claim citing imaginary evidence",
        vec![EvidenceRef::Observation(Uuid::new_v4())],
        None,
        ClaimKind::Observed,
    )];
    let error = store
        .complete_warning_analysis_run(run.id, &serde_json::json!({}), &imaginary)
        .await
        .expect_err("missing evidence must be rejected");
    assert!(
        format!("{error:#}").contains("missing observation"),
        "{error:#}"
    );

    // Observed claims with no evidence are rejected by the application policy.
    let evidence_less = vec![AnalysisClaimRecord::new(
        "Observed fact without evidence",
        Vec::new(),
        None,
        ClaimKind::Observed,
    )];
    assert!(store
        .complete_warning_analysis_run(run.id, &serde_json::json!({}), &evidence_less)
        .await
        .is_err());

    store
        .fail_warning_analysis_run(run.id, "fabricated provenance rejected")
        .await
        .expect("fail run");
    let stored = store
        .get_warning_analysis_run(run.id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(stored.status, "failed");
    assert!(stored.output.is_none());
    assert!(stored.error.is_some());

    // Database level: an observed claim with a zero evidence count violates the
    // table CHECK even for a writer that bypasses the application.
    let error = sqlx::query(
        "INSERT INTO warning_analysis_claims \
             (run_id, section, claim, claim_kind, claim_hash, evidence_count) \
         VALUES ($1, 'claim', 'observed with no evidence', 'observed', md5($2), 0)",
    )
    .bind(run.id)
    .bind("observed with no evidence")
    .execute(&pool)
    .await
    .expect_err("CHECK must reject an observed claim with zero evidence");
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"), "{error}");

    // Database level: citing an observation that does not exist violates the FK.
    let claim_id: Uuid = sqlx::query_scalar(
        "INSERT INTO warning_analysis_claims \
             (run_id, section, claim, claim_kind, claim_hash, evidence_count) \
         VALUES ($1, 'claim', 'claim pending evidence', 'unknown', md5($2), 0) \
         RETURNING id",
    )
    .bind(run.id)
    .bind("claim pending evidence")
    .fetch_one(&pool)
    .await
    .expect("insert unknown claim");
    let error = sqlx::query(
        "INSERT INTO warning_analysis_claim_evidence (claim_id, observation_id) VALUES ($1, $2)",
    )
    .bind(claim_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect_err("FK must reject an imaginary observation");
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"), "{error}");

    // Database level: a real observation link is accepted. The claim is
    // promoted to `observed` (with its count) before linking, so the immediate
    // count-refresh trigger never writes a count that violates the policy
    // CHECK; the deferred guard then validates the final state at COMMIT.
    let mut tx = pool.begin().await.expect("begin tx");
    sqlx::query(
        "UPDATE warning_analysis_claims \
         SET claim_kind = 'observed', evidence_count = 1 WHERE id = $1",
    )
    .bind(claim_id)
    .execute(&mut *tx)
    .await
    .expect("promote claim");
    sqlx::query(
        "INSERT INTO warning_analysis_claim_evidence (claim_id, observation_id) VALUES ($1, $2)",
    )
    .bind(claim_id)
    .bind(observation_id)
    .execute(&mut *tx)
    .await
    .expect("real observation link accepted");
    tx.commit().await.expect("commit valid claim state");

    let count: i32 =
        sqlx::query_scalar("SELECT evidence_count FROM warning_analysis_claims WHERE id = $1")
            .bind(claim_id)
            .fetch_one(&pool)
            .await
            .expect("count refreshed by trigger");
    assert_eq!(count, 1);

    cleanup(&pool, warning_id, &[observation_id]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn same_wording_in_two_sections_persists_two_claims() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;
    let observation_id = new_observation(&pool).await;

    let (run, _) = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-sections", "v2"))
        .await
        .expect("enqueue");
    store
        .start_warning_analysis_run(run.id)
        .await
        .expect("start run");

    // The same wording is deliberately used in two sections; a text-only
    // claim identity would collapse these into one row with last-writer-wins.
    let shared = "Qualify a second supplier";
    let claims = vec![
        AnalysisClaimRecord::new(
            shared,
            vec![EvidenceRef::Observation(observation_id)],
            Some(0.7),
            ClaimKind::Inference,
        )
        .with_section(ClaimSection::Claim),
        AnalysisClaimRecord::new(shared, Vec::new(), Some(0.6), ClaimKind::Recommendation)
            .with_section(ClaimSection::Action),
    ];
    let inserted = store
        .complete_warning_analysis_run(run.id, &serde_json::json!({}), &claims)
        .await
        .expect("complete run");
    assert_eq!(inserted, 2, "both sections must persist independently");

    let rows = store
        .list_warning_analysis_claims(run.id)
        .await
        .expect("list claims");
    assert_eq!(rows.len(), 2);
    let claim = rows
        .iter()
        .find(|row| row.section == "claim")
        .expect("claim section row");
    assert_eq!(claim.claim_kind, "inference");
    assert_eq!(
        claim.evidence,
        vec![EvidenceRef::Observation(observation_id)]
    );
    let action = rows
        .iter()
        .find(|row| row.section == "action")
        .expect("action section row");
    assert_eq!(action.claim_kind, "recommendation");
    assert!(action.evidence.is_empty());

    cleanup(&pool, warning_id, &[observation_id]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn deleting_cited_evidence_downgrades_claim_instead_of_blocking() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;
    let observation_id = new_observation(&pool).await;

    let (run, _) = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-delete", "v2"))
        .await
        .expect("enqueue");
    store
        .start_warning_analysis_run(run.id)
        .await
        .expect("start run");
    let claims = vec![AnalysisClaimRecord::new(
        "Observed fact tied to a deletable observation",
        vec![EvidenceRef::Observation(observation_id)],
        None,
        ClaimKind::Observed,
    )];
    store
        .complete_warning_analysis_run(run.id, &serde_json::json!({}), &claims)
        .await
        .expect("complete run");

    // Production cleanup (DNS-posture scans, retention) deletes observations.
    // That must not be blocked by citations; the claim is downgraded to an
    // explicit "provenance no longer verifiable" state instead.
    sqlx::query("DELETE FROM observations WHERE id = $1")
        .bind(observation_id)
        .execute(&pool)
        .await
        .expect("deleting cited evidence must not be blocked");

    let rows = store
        .list_warning_analysis_claims(run.id)
        .await
        .expect("list claims");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].claim_kind, "unknown");
    assert_eq!(rows[0].evidence_count, 0);
    assert!(rows[0].evidence.is_empty());

    // The run's result stays available and the claim text is preserved.
    let stored = store
        .get_warning_analysis_run(run.id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(stored.status, "succeeded");

    cleanup(&pool, warning_id, &[]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn stale_runs_expire_instead_of_hanging() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };
    let warning_id = new_warning(&pool).await;

    let (run, _) = store
        .enqueue_warning_analysis_run(&request(warning_id, "digest-e", "v2"))
        .await
        .expect("enqueue");
    sqlx::query(
        "UPDATE warning_analysis_runs SET updated_at = NOW() - INTERVAL '2 hours' WHERE id = $1",
    )
    .bind(run.id)
    .execute(&pool)
    .await
    .expect("age the run");

    let expired = store
        .expire_stale_warning_analysis_runs(300)
        .await
        .expect("expire stale runs");
    assert_eq!(expired, 1);

    let stored = store
        .get_warning_analysis_run(run.id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(stored.status, "failed");
    assert!(stored
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("no progress"));

    cleanup(&pool, warning_id, &[]).await;
    pool.close().await;
}
