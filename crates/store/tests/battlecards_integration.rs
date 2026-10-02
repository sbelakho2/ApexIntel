//! Opt-in integration test for the battlecard store methods behind the web
//! editor, comparison view and API: create outcomes, transactional detail
//! edits with optimistic concurrency, atomic regeneration, SQL-NULL sections,
//! order-preserving multi-get and the editor company options.
//!
//! `#[ignore]`d by default (CI runs it with `--ignored` against a Postgres
//! service), reads `TEST_DATABASE_URL` or `DATABASE_URL`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_store::postgres::{BattlecardWriteOutcome, CreateBattlecardOutcome, PgStore};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn connect() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres")
}

async fn insert_company(pool: &sqlx::PgPool, name: &str, is_competitor: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO companies (id, name, is_competitor) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(is_competitor)
        .execute(pool)
        .await
        .expect("insert company");
    id
}

async fn cleanup(pool: &sqlx::PgPool, companies: &[Uuid]) {
    sqlx::query(
        "DELETE FROM battlecards WHERE our_company_id = ANY($1) OR competitor_id = ANY($1)",
    )
    .bind(companies)
    .execute(pool)
    .await
    .expect("delete battlecards");
    sqlx::query("DELETE FROM companies WHERE id = ANY($1)")
        .bind(companies)
        .execute(pool)
        .await
        .expect("delete companies");
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn battlecard_lifecycle_round_trip() {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let store = PgStore::from_pool(pool.clone());

    let marker = Uuid::new_v4().simple().to_string();
    let ours = insert_company(&pool, &format!("bc-ours-{marker}"), false).await;
    let rival = insert_company(&pool, &format!("bc-rival-{marker}"), true).await;
    let rival_two = insert_company(&pool, &format!("bc-rival2-{marker}"), true).await;
    let companies = [ours, rival, rival_two];

    // ── Create: Created / Duplicate / UnknownCompany ───────────────────
    let first = match store
        .create_battlecard(ours, rival, "Ours vs Rival")
        .await
        .unwrap()
    {
        CreateBattlecardOutcome::Created(id) => id,
        other => panic!("expected Created, got {other:?}"),
    };
    assert_eq!(
        store.create_battlecard(ours, rival, "Again").await.unwrap(),
        CreateBattlecardOutcome::Duplicate(first),
        "the (our company, competitor) pair is unique"
    );
    assert_eq!(
        store
            .create_battlecard(ours, Uuid::new_v4(), "Ghost")
            .await
            .unwrap(),
        CreateBattlecardOutcome::UnknownCompany,
        "a missing company maps to UnknownCompany, not a raw FK error"
    );
    let second = match store
        .create_battlecard(ours, rival_two, "Ours vs Rival Two")
        .await
        .unwrap()
    {
        CreateBattlecardOutcome::Created(id) => id,
        other => panic!("expected Created, got {other:?}"),
    };

    let created = store.get_battlecard(first).await.unwrap().unwrap();
    assert_eq!(created.status, "draft");
    assert!(created.regenerated_at.is_none());
    assert!(created.positioning.is_none());

    // ── Details edit: title + status + sections in one transaction ─────
    let outcome = store
        .update_battlecard_details(
            first,
            Some("Renamed card"),
            Some("published"),
            &[
                ("positioning", json!("We win on uptime")),
                ("strengths", json!(["Coverage", "Support"])),
            ],
            "analyst-1",
            Some(created.updated_at),
        )
        .await
        .unwrap();
    assert_eq!(outcome, BattlecardWriteOutcome::Updated);
    let edited = store.get_battlecard(first).await.unwrap().unwrap();
    assert_eq!(edited.title, "Renamed card");
    assert_eq!(edited.status, "published");
    assert_eq!(edited.updated_by.as_deref(), Some("analyst-1"));
    assert_eq!(edited.positioning, Some(json!("We win on uptime")));
    assert_eq!(edited.strengths, Some(json!(["Coverage", "Support"])));
    assert!(edited.updated_at > created.updated_at);

    // A stale version token is rejected without writing anything.
    let stale = store
        .update_battlecard_details(
            first,
            Some("Lost update"),
            None,
            &[("positioning", json!("overwritten"))],
            "analyst-2",
            Some(created.updated_at),
        )
        .await
        .unwrap();
    assert_eq!(stale, BattlecardWriteOutcome::Conflict);
    let unchanged = store.get_battlecard(first).await.unwrap().unwrap();
    assert_eq!(unchanged.title, "Renamed card");
    assert_eq!(unchanged.positioning, Some(json!("We win on uptime")));
    assert_eq!(unchanged.updated_at, edited.updated_at);

    assert_eq!(
        store
            .update_battlecard_details(Uuid::new_v4(), Some("x"), None, &[], "analyst-1", None)
            .await
            .unwrap(),
        BattlecardWriteOutcome::NotFound
    );

    // Invalid input is rejected before any write.
    assert!(store
        .update_battlecard_details(first, None, Some("bogus"), &[], "analyst-1", None)
        .await
        .is_err());
    assert!(store
        .update_battlecard_details(first, Some("   "), None, &[], "analyst-1", None)
        .await
        .is_err());
    assert!(store
        .update_battlecard_details(
            first,
            None,
            None,
            &[("title; DROP TABLE battlecards", json!("x"))],
            "analyst-1",
            None
        )
        .await
        .is_err());
    assert_eq!(
        store
            .get_battlecard(first)
            .await
            .unwrap()
            .unwrap()
            .updated_at,
        edited.updated_at,
        "rejected edits must not touch the row"
    );

    // Clearing a section stores SQL NULL, not a JSONB null literal.
    store
        .update_battlecard_details(
            first,
            None,
            None,
            &[("strengths", serde_json::Value::Null)],
            "analyst-1",
            None,
        )
        .await
        .unwrap();
    let (strengths_is_sql_null,): (bool,) =
        sqlx::query_as("SELECT strengths IS NULL FROM battlecards WHERE id = $1")
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(strengths_is_sql_null);
    assert!(store
        .update_battlecard_section(first, "pricing", &serde_json::Value::Null, "analyst-1")
        .await
        .unwrap());
    let (pricing_is_sql_null,): (bool,) =
        sqlx::query_as("SELECT pricing IS NULL FROM battlecards WHERE id = $1")
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(pricing_is_sql_null);

    // ── Regeneration: all sections + regenerated_at, atomically ─────────
    let before_regen = store.get_battlecard(first).await.unwrap().unwrap();
    assert!(store
        .apply_battlecard_regeneration(
            first,
            &[
                ("positioning", json!({"summary": "regenerated"})),
                ("kill_shots", json!(["Faster onboarding"])),
            ],
            "system",
        )
        .await
        .unwrap());
    let regenerated = store.get_battlecard(first).await.unwrap().unwrap();
    assert!(regenerated.regenerated_at.is_some());
    assert_eq!(
        regenerated.positioning,
        Some(json!({"summary": "regenerated"}))
    );
    assert_eq!(regenerated.kill_shots, Some(json!(["Faster onboarding"])));
    assert_eq!(
        regenerated.title, "Renamed card",
        "regeneration keeps the title"
    );

    // An editor holding the pre-regeneration version cannot overwrite it.
    assert_eq!(
        store
            .update_battlecard_details(
                first,
                None,
                None,
                &[("positioning", json!("stale editor"))],
                "analyst-1",
                Some(before_regen.updated_at),
            )
            .await
            .unwrap(),
        BattlecardWriteOutcome::Conflict
    );

    // An invalid section aborts the whole regeneration.
    assert!(store
        .apply_battlecard_regeneration(
            first,
            &[("positioning", json!("partial")), ("nope", json!("x"))],
            "system",
        )
        .await
        .is_err());
    assert_eq!(
        store
            .get_battlecard(first)
            .await
            .unwrap()
            .unwrap()
            .positioning,
        Some(json!({"summary": "regenerated"}))
    );
    assert!(!store
        .apply_battlecard_regeneration(Uuid::new_v4(), &[], "system")
        .await
        .unwrap());

    // ── Multi-get preserves the requested order and skips unknown ids ───
    let missing = Uuid::new_v4();
    let rows = store
        .get_battlecards_by_ids(&[second, missing, first])
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![second, first]
    );
    assert!(store.get_battlecards_by_ids(&[]).await.unwrap().is_empty());

    // ── Filters and status counts ───────────────────────────────────────
    let published = store
        .list_battlecards(Some("published"), Some(rival), 1, 50)
        .await
        .unwrap();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].id, first);
    assert_eq!(
        store
            .count_battlecards(Some("draft"), Some(rival_two))
            .await
            .unwrap(),
        1
    );
    let by_status = store.count_battlecards_by_status().await.unwrap();
    assert!(by_status.iter().any(|(s, n)| s == "published" && *n >= 1));
    assert!(by_status.iter().any(|(s, n)| s == "draft" && *n >= 1));

    // ── Company options flag competitors ────────────────────────────────
    let options = store.list_battlecard_company_options(10_000).await.unwrap();
    let flag = |id: Uuid| options.iter().find(|o| o.id == id).map(|o| o.is_competitor);
    assert_eq!(flag(ours), Some(false));
    assert_eq!(flag(rival), Some(true));
    let first_non_competitor = options.iter().position(|o| !o.is_competitor);
    let last_competitor = options.iter().rposition(|o| o.is_competitor);
    if let (Some(first_other), Some(last_comp)) = (first_non_competitor, last_competitor) {
        assert!(last_comp < first_other, "competitors are listed first");
    }

    // ── Delete ──────────────────────────────────────────────────────────
    assert!(store.delete_battlecard(second).await.unwrap());
    assert!(!store.delete_battlecard(second).await.unwrap());
    assert!(store.get_battlecard(second).await.unwrap().is_none());

    cleanup(&pool, &companies).await;
}
