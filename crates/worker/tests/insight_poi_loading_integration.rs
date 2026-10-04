//! PostgreSQL integration tests for the insight-generation data loaders.
//!
//! Covers four confirmed defects at HEAD:
//!
//! * **N6** — `load_company_pois` used `COALESCE(p.\"current_role\", ...)`
//!   inside a raw string; raw strings do not process escapes, so PostgreSQL
//!   received a literal backslash and every company load failed. The person
//!   row below can only be returned when the identifier is spelled
//!   `p."current_role"`.
//! * **#99** — evidence was selected by `SELECT DISTINCT ON (o.id) ... LIMIT
//!   5000`, then `.take(12)`, so a noisy company could crowd out every other
//!   company and the kept rows were not the newest per company. The loader now
//!   keeps the newest 12 rows per company via `ROW_NUMBER()`.
//! * **#100** — the LLM cache key ignored the rendered prompt and the output
//!   was cached before the grounding check. `put_llm_cache_grounded` must bind
//!   the prompt (different prompt => different key) and must never write when
//!   grounding failed.
//!
//! `#[ignore]`d by default (CI runs DB suites with `--ignored`); reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`.
//!
//! The production module lives in the `apex-worker` *binary* (declared in
//! `src/main.rs`), not the library, so it cannot be imported by an integration
//! test directly. It is included here via `#[path]` together with minimal
//! shims for the binary-crate paths it resolves against
//! (`crate::EntityContext`, `crate::EvidenceSignal`, `crate::config`,
//! `crate::generate_llm_insight`); the loader code under test is the exact
//! production source.
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports)]

use apex_store::postgres::PgStore;
use apex_worker::scheduler::{JobKind, JobRun};
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ─── Shims for the binary-crate paths `insights.rs` resolves against ─────────

#[cfg(feature = "llm")]
pub use apex_llm::inference::LlmClient as InferenceLlmClient;

/// The binary's `crate::config::llm_timeout`; never called by the loaders.
#[cfg(feature = "llm")]
mod config {
    pub(crate) fn llm_timeout() -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }
}

/// Mirror of the binary's `crate::EntityContext` (fields read/constructed by
/// `insights.rs`).
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub struct EntityContext {
    pub name: String,
    pub region: String,
    pub entity_type: Option<String>,
    pub is_competitor: bool,
    pub industry_tags: Vec<String>,
    pub certifications: Vec<String>,
    pub capabilities: Vec<String>,
    pub key_persons: Vec<String>,
    pub recent_changes: Vec<String>,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub strategic_relevance: Option<f64>,
    pub revenue_estimate_usd: Option<i64>,
    pub employee_estimate: Option<i32>,
    pub competitor_names: Vec<String>,
    pub sites_summary: Vec<String>,
    pub competitor_events: Vec<String>,
    pub domain: Option<String>,
    pub context_unavailable: Vec<String>,
}

/// Mirror of the binary's `crate::EvidenceSignal`.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub struct EvidenceSignal {
    pub title: String,
    pub description: String,
    pub source_url: String,
    pub signal_type: String,
    pub extracted_facts: Vec<String>,
    pub date_context: Option<String>,
    pub relevance_score: f32,
}

/// The model is never invoked by these tests; this shim only satisfies the
/// module's call site so the loaders compile unmodified.
#[cfg(feature = "llm")]
pub async fn generate_llm_insight(
    _llm_client: &InferenceLlmClient,
    _entity_ctx: &EntityContext,
    _category: &str,
    _evidence_signals: &[EvidenceSignal],
) -> Result<(String, String, String, f64, serde_json::Value), String> {
    Err("generate_llm_insight is not exercised by the loader integration tests".to_string())
}

#[path = "../src/job_execution/insights.rs"]
mod insights;

// ─── Test harness ────────────────────────────────────────────────────────────

static DB_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to postgres")
}

async fn insert_company(pool: &sqlx::PgPool, id: Uuid, name: &str) {
    sqlx::query("INSERT INTO companies (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .expect("insert company");
}

// ─── N6: POI loading must reach the `current_role` column ────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn load_company_pois_returns_primary_org_person_with_current_role() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let company_id = Uuid::new_v4();
    let person_id = Uuid::new_v4();

    // Re-entrant: clear rows left behind by a previously interrupted run.
    sqlx::query("DELETE FROM persons WHERE primary_org_id = $1")
        .bind(company_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM companies WHERE id = $1")
        .bind(company_id)
        .execute(&pool)
        .await
        .unwrap();

    insert_company(&pool, company_id, "N6 Procurement Target").await;
    sqlx::query(
        "INSERT INTO persons (id, name, primary_org_id, \"current_role\", role_family, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(person_id)
    .bind("Dana Buyer")
    .bind(company_id)
    .bind("VP Procurement")
    .bind("procurement")
    .bind(serde_json::json!({ "role_family": "procurement" }))
    .execute(&pool)
    .await
    .unwrap();

    // Before the N6 fix the raw string made PostgreSQL parse `p.\"current_role\"`
    // and the query failed, so this call returned Err and every company was
    // skipped. The assertion below is therefore the regression guard.
    let pois = insights::load_company_pois(&store, &company_id)
        .await
        .expect("load_company_pois must succeed against a migrated PostgreSQL schema");
    assert_eq!(pois.len(), 1, "the primary-org person must be returned");
    assert_eq!(pois[0].id, person_id);
    assert_eq!(pois[0].name, "Dana Buyer");
    assert_eq!(
        pois[0].role, "VP Procurement",
        "the person's current_role must be returned, not the COALESCE fallback"
    );
    assert!(
        pois[0].is_buyer_relevant,
        "a procurement role must be classified as buyer-relevant"
    );

    sqlx::query("DELETE FROM persons WHERE primary_org_id = $1")
        .bind(company_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM companies WHERE id = $1")
        .bind(company_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ─── #99: newest 12 observations per company ─────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn selects_the_newest_twelve_observations_per_company() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let company_a = Uuid::new_v4();
    let company_b = Uuid::new_v4();
    let entity_ids = [company_a, company_b];

    sqlx::query("DELETE FROM observations WHERE entity_id = ANY($1)")
        .bind(&entity_ids[..])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM companies WHERE id = ANY($1)")
        .bind(&entity_ids[..])
        .execute(&pool)
        .await
        .unwrap();

    insert_company(&pool, company_a, "Noisy Company").await;
    insert_company(&pool, company_b, "Quiet Company").await;

    let base: DateTime<Utc> = Utc::now();
    let since = base - Duration::hours(24);

    let mut newest_a = Vec::new();
    for index in 0..15i64 {
        let observation_id = Uuid::new_v4();
        let created_at = base - Duration::minutes(index);
        sqlx::query(
            "INSERT INTO observations \
                 (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, created_at) \
             VALUES ($1, 'news', $2, 'company', $3, $4, $5, $3)",
        )
        .bind(observation_id)
        .bind(company_a)
        .bind(created_at)
        .bind(serde_json::json!({
            "content": format!("company-a observation {index:02} {}", "x".repeat(64)),
        }))
        .bind(serde_json::json!({ "url": format!("https://example.com/a/{index}") }))
        .execute(&pool)
        .await
        .unwrap();
        if index < 12 {
            newest_a.push(observation_id);
        }
    }

    let mut all_b = Vec::new();
    for index in 0..3i64 {
        let observation_id = Uuid::new_v4();
        let created_at = base - Duration::minutes(index);
        sqlx::query(
            "INSERT INTO observations \
                 (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, created_at) \
             VALUES ($1, 'news', $2, 'company', $3, $4, $5, $3)",
        )
        .bind(observation_id)
        .bind(company_b)
        .bind(created_at)
        .bind(serde_json::json!({
            "content": format!("company-b observation {index:02} {}", "y".repeat(64)),
        }))
        .bind(serde_json::json!({ "url": format!("https://example.com/b/{index}") }))
        .execute(&pool)
        .await
        .unwrap();
        all_b.push(observation_id);
    }

    let loaded = insights::load_recent_observations_by_company(&store, since)
        .await
        .expect("loading recent observations must succeed");

    let a = loaded.get(&company_a).expect("company A must be present");
    assert_eq!(
        a.len(),
        12,
        "the loader must keep exactly the newest 12 observations per company"
    );
    let a_ids: Vec<Uuid> = a.iter().map(|observation| observation.id).collect();
    assert_eq!(
        a_ids, newest_a,
        "the newest 12 must be kept, newest first; the 3 oldest rows are dropped"
    );

    let b = loaded.get(&company_b).expect("company B must be present");
    let b_ids: Vec<Uuid> = b.iter().map(|observation| observation.id).collect();
    assert_eq!(
        b_ids, all_b,
        "a quiet company must keep all of its (fewer than 12) observations"
    );

    sqlx::query("DELETE FROM observations WHERE entity_id = ANY($1)")
        .bind(&entity_ids[..])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM companies WHERE id = ANY($1)")
        .bind(&entity_ids[..])
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ─── #100: prompt-bound cache and no write on failed grounding ──────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn prompt_bound_cache_never_stores_ungrounded_output() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let workflow = "insight_poi_loading_integration";
    sqlx::query("DELETE FROM llm_cache WHERE workflow = $1")
        .bind(workflow)
        .execute(&pool)
        .await
        .unwrap();

    let base_key = format!("base-{}", Uuid::new_v4());
    let prompt_a = "rendered prompt A";
    let prompt_b = "rendered prompt B";
    let no_evidence: Vec<Uuid> = Vec::new();

    let wrote = store
        .put_llm_cache_grounded(
            &base_key,
            workflow,
            "test-model",
            "v1",
            &no_evidence,
            prompt_a,
            true,
            Duration::minutes(5),
            r#"{"headline":"grounded"}"#,
        )
        .await
        .unwrap();
    assert!(wrote, "a grounded output must be cached");

    assert!(
        store
            .get_llm_cache_for_prompt(&base_key, prompt_a)
            .await
            .unwrap()
            .is_some(),
        "the grounded output must be readable under the same rendered prompt"
    );
    assert!(
        store
            .get_llm_cache_for_prompt(&base_key, prompt_b)
            .await
            .unwrap()
            .is_none(),
        "a different rendered prompt must derive a different cache key"
    );

    let wrote_ungrounded = store
        .put_llm_cache_grounded(
            &base_key,
            workflow,
            "test-model",
            "v1",
            &no_evidence,
            prompt_b,
            false,
            Duration::minutes(5),
            r#"{"headline":"ungrounded"}"#,
        )
        .await
        .unwrap();
    assert!(
        !wrote_ungrounded,
        "an ungrounded output must never be written to the cache"
    );
    assert!(
        store
            .get_llm_cache_for_prompt(&base_key, prompt_b)
            .await
            .unwrap()
            .is_none(),
        "a failed grounding must leave no cache row behind"
    );

    sqlx::query("DELETE FROM llm_cache WHERE workflow = $1")
        .bind(workflow)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
