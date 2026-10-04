//! PostgreSQL integration tests for the POI resolver/merge/photo wiring.
//!
//! Covers the two database-level defects at HEAD:
//!
//! * **#164** — `photo_change_detection_sql` selected the previous photo hash
//!   without requiring the same source document, so a photo seen on one URL
//!   could be associated with a hash from a different URL. These tests insert
//!   artifacts with the same and different source URLs and assert the
//!   previous-hash association never crosses source documents.
//! * **#165** — the generated merge SQL must rewrite `warnings.entity_ids` and
//!   `insights.entity_ids` with `array_replace`, and the store helper must
//!   apply every generated statement in one transaction while recording the
//!   merge in `entity_merges`.
//!
//! `#[ignore]`d by default (CI runs DB suites with `--ignored`); reads
//! `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_graph::entity_merge::{build_merge_event, generate_merge_sql, EntityType, MergeReason};
use apex_store::postgres::PgStore;
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

async fn migrate(pool: &sqlx::PgPool) {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .expect("apply migrations");
}

#[derive(Debug, sqlx::FromRow)]
struct PhotoDetectionRow {
    person_id: Uuid,
    person_name: String,
    source_url: String,
    current_hash: String,
    previous_hash: Option<String>,
    crawled_at: DateTime<Utc>,
}

async fn insert_person(pool: &sqlx::PgPool, id: Uuid, name: &str) {
    sqlx::query("DELETE FROM persons WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO persons (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
}

async fn insert_photo_artifact(
    pool: &sqlx::PgPool,
    person_id: Uuid,
    url: &str,
    photo_hash: &str,
    ts_utc: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO poi_artifacts (person_id, artifact_type, url, ts_utc, metadata) \
         VALUES ($1, 'profile_photo', $2, $3, $4)",
    )
    .bind(person_id)
    .bind(url)
    .bind(ts_utc)
    .bind(serde_json::json!({ "photo_hash": photo_hash }))
    .execute(pool)
    .await
    .unwrap();
}

async fn detection_rows(pool: &sqlx::PgPool, person_id: Uuid) -> Vec<PhotoDetectionRow> {
    sqlx::query_as::<_, PhotoDetectionRow>(apex_poi::photo_detector::photo_change_detection_sql())
        .fetch_all(pool)
        .await
        .expect("photo detection query must run against the real schema")
        .into_iter()
        .filter(|row| row.person_id == person_id)
        .collect()
}

// ─── #164: same-source association ───────────────────────────────────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn photo_change_detection_associates_previous_hash_within_same_source() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    migrate(&pool).await;

    let person_id = Uuid::new_v4();
    let now = Utc::now();
    let url = "https://example.com/team/ahmed-ben-ali";

    insert_person(&pool, person_id, "Same Source Person").await;
    insert_photo_artifact(&pool, person_id, url, "hash-old", now - Duration::days(3)).await;
    insert_photo_artifact(&pool, person_id, url, "hash-new", now).await;

    let rows = detection_rows(&pool, person_id).await;
    assert_eq!(
        rows.len(),
        1,
        "exactly the changed newest row must be returned"
    );
    assert_eq!(rows[0].source_url, url);
    assert_eq!(rows[0].current_hash, "hash-new");
    assert_eq!(
        rows[0].previous_hash.as_deref(),
        Some("hash-old"),
        "the previous hash from the SAME source document must be associated"
    );

    sqlx::query("DELETE FROM persons WHERE id = $1")
        .bind(person_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ─── #164: cross-source non-association (regression for the missing predicate) ─

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn photo_change_detection_never_associates_hashes_across_source_documents() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    migrate(&pool).await;

    let now = Utc::now();
    let url_a = "https://example.com/team/ahmed-ben-ali";
    let url_b = "https://directory.example.org/people/ahmed-ben-ali";

    // Case 1: identical hash on two different source documents. The buggy
    // lateral lookup used the hash from url_a as "previous" for url_b, which
    // cancelled the change and returned zero rows. With the fix, url_b is a
    // first sighting on its own source document (previous_hash = NULL).
    let same_hash_person = Uuid::new_v4();
    insert_person(&pool, same_hash_person, "Same Hash Person").await;
    insert_photo_artifact(
        &pool,
        same_hash_person,
        url_a,
        "shared-hash",
        now - Duration::days(3),
    )
    .await;
    insert_photo_artifact(&pool, same_hash_person, url_b, "shared-hash", now).await;

    let rows = detection_rows(&pool, same_hash_person).await;
    assert_eq!(
        rows.len(),
        1,
        "the newest artifact on url_b must be reported as a first sighting"
    );
    assert_eq!(rows[0].source_url, url_b);
    assert_eq!(
        rows[0].previous_hash, None,
        "a hash from a different source document must never become the previous hash"
    );

    // Case 2: different hashes on different source documents. The buggy lookup
    // associated url_a's hash with url_b; the fixed lookup must not.
    let different_hash_person = Uuid::new_v4();
    insert_person(&pool, different_hash_person, "Different Hash Person").await;
    insert_photo_artifact(
        &pool,
        different_hash_person,
        url_a,
        "hash-from-a",
        now - Duration::days(3),
    )
    .await;
    insert_photo_artifact(&pool, different_hash_person, url_b, "hash-from-b", now).await;

    let rows = detection_rows(&pool, different_hash_person).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_url, url_b);
    assert_eq!(
        rows[0].previous_hash, None,
        "cross-source hash must not be associated even when hashes differ"
    );

    sqlx::query("DELETE FROM persons WHERE id = ANY($1)")
        .bind(&[same_hash_person, different_hash_person][..])
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ─── #165: apply merges and rewrite warning/insight entity arrays ────────────

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn apply_entity_merge_statements_rewrites_entity_arrays_and_records_merge() {
    let _guard = DB_TEST_LOCK.lock().await;
    let pool = connect().await;
    migrate(&pool).await;
    let store = PgStore::from_pool(pool.clone());

    let survivor = Uuid::new_v4();
    let merged = Uuid::new_v4();
    let co_referenced = Uuid::new_v4();
    let warning_id = Uuid::new_v4();
    let insight_id = Uuid::new_v4();

    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO warnings (id, warning_type, title, entity_id, entity_ids) \
         VALUES ($1, 'test', 'POI merge test warning', $2, $3)",
    )
    .bind(warning_id)
    .bind(merged)
    // The survivor is already co-referenced here: after `merged -> survivor`
    // the rewrite must deduplicate instead of yielding [survivor, survivor, …].
    .bind(&[merged, survivor, co_referenced][..])
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO insights (id, insight_type, title, entity_id, entity_ids) \
         VALUES ($1, 'test', 'POI merge test insight', $2, $3)",
    )
    .bind(insight_id)
    .bind(merged)
    .bind(&[co_referenced, merged, survivor][..])
    .execute(&pool)
    .await
    .unwrap();

    let event = build_merge_event(
        EntityType::Person,
        vec![merged.to_string()],
        survivor.to_string(),
        MergeReason::AutoResolution {
            similarity_score: 0.9,
        },
        0.9,
        "poi_resolver_integration",
    );
    let statements = generate_merge_sql(&event);

    store
        .apply_entity_merge_statements(&statements)
        .await
        .expect("merge statements must apply transactionally");

    let warning_entities: Vec<Uuid> =
        sqlx::query_scalar("SELECT entity_ids FROM warnings WHERE id = $1")
            .bind(warning_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(warning_entities, vec![survivor, co_referenced]);
    let warning_entity: Option<Uuid> =
        sqlx::query_scalar("SELECT entity_id FROM warnings WHERE id = $1")
            .bind(warning_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(warning_entity, Some(survivor));

    let insight_entities: Vec<Uuid> =
        sqlx::query_scalar("SELECT entity_ids FROM insights WHERE id = $1")
            .bind(insight_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(insight_entities, vec![co_referenced, survivor]);
    let insight_entity: Option<Uuid> =
        sqlx::query_scalar("SELECT entity_id FROM insights WHERE id = $1")
            .bind(insight_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(insight_entity, Some(survivor));

    let recorded_arrays: Vec<Vec<Uuid>> = sqlx::query_scalar(
        "SELECT merged_ids FROM entity_merges \
         WHERE survivor_id = $1 AND entity_type = 'person' \
         ORDER BY merged_at DESC",
    )
    .bind(survivor)
    .fetch_all(&pool)
    .await
    .unwrap();
    let recorded_merged: Vec<Uuid> = recorded_arrays.into_iter().flatten().collect();
    assert!(
        recorded_merged.contains(&merged),
        "the merge must stay auditable in entity_merges"
    );

    sqlx::query("DELETE FROM entity_merges WHERE survivor_id = $1")
        .bind(survivor)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM warnings WHERE id = $1")
        .bind(warning_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM insights WHERE id = $1")
        .bind(insight_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
