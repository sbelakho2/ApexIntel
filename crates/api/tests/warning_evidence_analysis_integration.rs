//! Opt-in integration test: warning analysis consumes explicit
//! `warning_evidence` links (audit warning-evidence item, migration 082).
//!
//! Proves the end-to-end analysis contract against a real database:
//!   * a warning whose only provenance is a `source_url` (no entity ids) yields
//!     a warning-evidence bundle whose linked observation is the direct evidence
//!     the prompt exposes and claims can cite;
//!   * a warning with no links and no entity observations is deterministically
//!     reported as `InsufficientEvidence` by the preflight.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored --features llm` against
//! a Postgres service, reading `TEST_DATABASE_URL` or `DATABASE_URL`.
#![cfg(feature = "llm")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_api::warning_analysis::{
    build_prompts, gather_evidence, parse_analysis_payload, preflight_status,
    validate_analysis_payload, AnalysisStatus, EvidenceScope,
};
use apex_core::intelligence_profile::IntelligenceProfile;
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

async fn cleanup(pool: &PgPool, warning_id: Uuid, urls: &[&str]) {
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(pool)
        .await
        .expect("delete warning cascades evidence");
    for url in urls {
        sqlx::query("DELETE FROM observations WHERE provenance->>'source_url' = $1")
            .bind(url)
            .execute(pool)
            .await
            .expect("delete linked observations");
        sqlx::query("DELETE FROM sources WHERE url = $1")
            .bind(url)
            .execute(pool)
            .await
            .expect("delete source documents");
    }
}

fn test_profile() -> IntelligenceProfile {
    IntelligenceProfile {
        name: "Integration".to_string(),
        system_prompt: "You are a test analyst.".to_string(),
        focus_areas: vec!["supply chains".to_string()],
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn source_url_only_warning_analysis_cites_the_linked_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };

    let url = format!("https://analysis.example.com/doc/{}", Uuid::new_v4());
    let title = format!("Analysis evidence warning {}", Uuid::new_v4());
    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &title,
            Some("Primary document reports a shortage"),
            "high",
            None,
            None,
            None,
            Some(vec![url.clone()]),
            Some(0.8),
            false,
        )
        .await
        .expect("insert warning");
    let warning = store
        .get_warning(outcome.id)
        .await
        .expect("load warning")
        .expect("warning exists");

    let bundle = gather_evidence(&store, &warning)
        .await
        .expect("gather evidence");
    assert_eq!(bundle.evidence_scope, EvidenceScope::WarningEvidence);
    assert_eq!(bundle.warning_evidence_count, 1);
    assert_eq!(
        bundle.observations.len(),
        1,
        "the linked observation is the direct evidence set"
    );
    assert_eq!(
        bundle.observations_available, 1,
        "available count reflects the linked corpus"
    );

    let link = store
        .list_warning_evidence(outcome.id)
        .await
        .expect("load links")
        .pop()
        .expect("one link");
    assert_eq!(bundle.observations[0].id, link.observation_id);

    // The prompt exposes the linked id: the model can cite exactly this row.
    let (system, user) = build_prompts(&test_profile(), &warning, &bundle);
    assert!(
        user.contains(&link.observation_id.to_string()),
        "prompt must expose the linked observation id"
    );
    assert!(user.contains("source: warning_evidence"));
    assert!(system.contains("observation id"));

    // A claim citing the linked observation validates against the bundle.
    let payload = parse_analysis_payload(
        &serde_json::json!({
            "claims": [{
                "text": "The primary document reports a shortage",
                "claim_kind": "observed",
                "evidence_ids": [link.observation_id.to_string()],
                "confidence": 0.8
            }],
            "impact": [],
            "actions": [],
            "limitations": []
        })
        .to_string(),
    )
    .expect("payload parses");
    let validated = validate_analysis_payload(payload, &bundle.evidence_index())
        .expect("linked evidence citation validates");
    assert_eq!(validated.claims.len(), 1);
    assert_eq!(
        validated.claims[0].evidence,
        vec![apex_core::claims::EvidenceRef::Observation(
            link.observation_id
        )]
    );
    assert_eq!(preflight_status(&bundle), AnalysisStatus::Completed);

    cleanup(&pool, outcome.id, &[&url]).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn warning_without_links_or_entities_is_insufficient_evidence() {
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore { pool: pool.clone() };

    let outcome = store
        .insert_warning_with_outcome(
            "supply_chain",
            &format!("No evidence warning {}", Uuid::new_v4()),
            Some("No provenance attached"),
            "medium",
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .await
        .expect("insert warning");
    let warning = store
        .get_warning(outcome.id)
        .await
        .expect("load warning")
        .expect("warning exists");

    let bundle = gather_evidence(&store, &warning)
        .await
        .expect("gather evidence");
    assert_eq!(bundle.evidence_scope, EvidenceScope::None);
    assert_eq!(bundle.warning_evidence_count, 0);
    assert!(bundle.observations.is_empty());
    assert!(bundle.insights.is_empty());
    assert_eq!(
        preflight_status(&bundle),
        AnalysisStatus::InsufficientEvidence,
        "empty evidence must be reported deterministically without a model call"
    );

    cleanup(&pool, outcome.id, &[]).await;
    pool.close().await;
}
