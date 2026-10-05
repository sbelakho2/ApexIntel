//! Opt-in integration tests for "whole set" list reads and page aggregates.
//!
//! Every list call clamps its limit to 500 rows, so callers that need *every*
//! match (sanctions/breach screening, autocomplete, page stats and filter
//! chips) must page or aggregate in SQL. These tests pin that the `list_all_*`
//! readers cross the clamp and that the page summaries are exact.
//!
//! `#[ignore]`d by default; CI runs it with `--ignored` against a Postgres
//! service through `scripts/ci/run_pg_integration_suites.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use apex_core::entities::{Company, CompanyType, Person, RoleFamily};
use apex_store::postgres::{
    CompanyListFilters, CompanyOrderBy, InsightListFilters, PersonListFilters, PersonOrderBy,
    PgStore, WarningListFilters, WarningOrderBy,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

/// One more than the list clamp, so a single capped call cannot pass.
const PAST_CAP: usize = 501;

async fn setup() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to postgres");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations apply");
    pool
}

fn prefix(tag: &str) -> String {
    format!("{tag}{}", Uuid::new_v4().simple())
}

/// A unique letters-only word per index (`tokaaa`, `tokaab`, ...).
fn alpha_token(index: usize) -> String {
    let letter = |n: usize| char::from(b'a' + (n % 26) as u8);
    format!(
        "tok{}{}{}",
        letter(index / 676),
        letter(index / 26),
        letter(index)
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn list_all_readers_cross_the_list_clamp() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("LALL");

    let mut company_ids = HashSet::new();
    let mut person_ids = HashSet::new();
    for i in 0..PAST_CAP {
        let company = Company::new(format!("{p} Co {i:04}"), CompanyType::Oem);
        store
            .insert_company(&company)
            .await
            .expect("insert company");
        company_ids.insert(company.id);

        let person = Person::new(format!("{p} Person {i:04}"), RoleFamily::Procurement);
        store.insert_person(&person).await.expect("insert person");
        person_ids.insert(person.id);

        store
            .insert_insight(
                &format!("{p} Insight {i:04}"),
                // Distinct alphabetic tokens: the story dedup signature ignores
                // numbers, so identical-but-for-a-number summaries merge.
                &format!("summary {} for {p}", alpha_token(i)),
                Some("competitive"),
                None,
                Some(0.5),
                None,
                None,
                None,
                None,
            )
            .await
            .expect("insert insight");
    }

    let filters = CompanyListFilters {
        search: Some(p.clone()),
        ..Default::default()
    };
    let capped = store
        .list_companies(&filters, Some(CompanyOrderBy::Name), false, 10_000, 0)
        .await
        .expect("capped list");
    assert_eq!(capped.len(), 500, "single list calls clamp to 500 rows");
    let matching = store
        .list_all_companies_matching(&filters, Some(CompanyOrderBy::Name), false)
        .await
        .expect("all matching companies");
    assert_eq!(matching.len(), PAST_CAP);
    let names: Vec<&str> = matching.iter().map(|c| c.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "the requested order holds across pages");
    let unique: HashSet<Uuid> = matching.iter().map(|c| c.id).collect();
    assert_eq!(unique, company_ids, "no row is duplicated or skipped");

    let all_companies: HashSet<Uuid> = store
        .list_all_companies()
        .await
        .expect("all companies")
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert!(company_ids.is_subset(&all_companies));

    let all_persons: HashSet<Uuid> = store
        .list_all_persons()
        .await
        .expect("all persons")
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert!(person_ids.is_subset(&all_persons));
    let names: HashSet<String> = store
        .list_all_person_names()
        .await
        .expect("all person names")
        .into_iter()
        .collect();
    assert!(names.contains(&format!("{p} Person {:04}", PAST_CAP - 1)));

    let insights = store
        .list_all_insights_matching(&InsightListFilters {
            search: Some(p.clone()),
            exclude_internal: true,
            ..Default::default()
        })
        .await
        .expect("all matching insights");
    assert_eq!(insights.len(), PAST_CAP);
    let unique: HashSet<Uuid> = insights.iter().map(|i| i.id).collect();
    assert_eq!(unique.len(), PAST_CAP);

    let pattern = format!("{p}%");
    sqlx::query("DELETE FROM insights WHERE title LIKE $1")
        .bind(&pattern)
        .execute(&pool)
        .await
        .expect("cleanup insights");
    sqlx::query("DELETE FROM persons WHERE name LIKE $1")
        .bind(&pattern)
        .execute(&pool)
        .await
        .expect("cleanup persons");
    sqlx::query("DELETE FROM companies WHERE name LIKE $1")
        .bind(&pattern)
        .execute(&pool)
        .await
        .expect("cleanup companies");
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn person_summary_is_exact_and_search_covers_roles() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("PSUM");

    let mut ids = Vec::new();
    for (i, influence) in [0.1, 0.5, 0.95].into_iter().enumerate() {
        let mut person = Person::new(format!("{p} Person {i}"), RoleFamily::Procurement);
        person.influence_score = influence;
        store.insert_person(&person).await.expect("insert person");
        ids.push(person.id);
    }
    // Matched only through its role, not its name.
    let mut by_role = Person::new(
        format!("Unrelated {}", Uuid::new_v4().simple()),
        RoleFamily::Procurement,
    );
    by_role.current_role = Some(format!("{p} Buyer"));
    by_role.influence_score = 0.3;
    store.insert_person(&by_role).await.expect("insert person");
    ids.push(by_role.id);

    let vector = serde_json::json!({
        "decision_power": 0.8,
        "domain_relevance": 0.4,
        "network_centrality": 0.6,
        "engagement_potential": 0.2,
        "intelligence_value": 1.0,
    });
    let priority =
        apex_core::priority::priority_score_from_json(&vector).expect("canonical vector parses");
    store
        .update_person_psych_profile(
            ids[0],
            Some(&vector.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("write priority vector");

    let filters = PersonListFilters {
        search: Some(p.clone()),
        ..Default::default()
    };
    let summary = store
        .summarize_persons(&filters)
        .await
        .expect("summarize persons");
    assert_eq!(summary.total, 4, "search matches names and roles");
    assert_eq!(store.count_persons(&filters).await.expect("count"), 4);
    assert_eq!(summary.priority_a, i64::from(priority >= 0.8));
    assert_eq!(
        summary.priority_b,
        i64::from((0.5..0.8).contains(&priority))
    );
    assert_eq!(summary.influence_measured, 4);
    assert_eq!(summary.influence_pct_sum, 10 + 50 + 95 + 30);
    assert_eq!(summary.influence_0_20, 1);
    assert_eq!(summary.influence_20_40, 1);
    assert_eq!(summary.influence_40_60, 1);
    assert_eq!(summary.influence_60_80, 0);
    assert_eq!(summary.influence_80_100, 1);

    // Pages are disjoint and cover the whole match set.
    let first = store
        .list_persons(&filters, Some(PersonOrderBy::UpdatedAt), true, 2, 0)
        .await
        .expect("page 1");
    let second = store
        .list_persons(&filters, Some(PersonOrderBy::UpdatedAt), true, 2, 2)
        .await
        .expect("page 2");
    let paged: HashSet<Uuid> = first.iter().chain(second.iter()).map(|p| p.id).collect();
    assert_eq!(paged, ids.iter().copied().collect::<HashSet<_>>());

    for id in ids {
        sqlx::query("DELETE FROM persons WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup person");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn warning_summary_is_exact_past_the_list_clamp() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("WSUM");

    store
        .insert_warning(
            "aggtest_alpha",
            &format!("{p} critical"),
            Some("critical description"),
            "critical",
            Some("Tunisia"),
            None,
            None,
            None,
            Some(0.9),
            true,
        )
        .await
        .expect("insert critical");
    let acked = store
        .insert_warning(
            "aggtest_beta",
            &format!("{p} high acked"),
            Some("high description"),
            "high",
            Some("EU"),
            None,
            None,
            None,
            Some(0.8),
            true,
        )
        .await
        .expect("insert high");
    store
        .acknowledge_warning(acked, "aggtest", None, None)
        .await
        .expect("acknowledge");
    for i in 0..PAST_CAP {
        store
            .insert_warning(
                "aggtest_gamma",
                &format!("{p} medium {i:04}"),
                Some(&format!("medium description {i}")),
                "medium",
                None,
                None,
                None,
                None,
                Some(0.5),
                true,
            )
            .await
            .expect("insert medium");
    }

    let filters = WarningListFilters {
        search: Some(p.clone()),
        ..Default::default()
    };
    let expected_total = (PAST_CAP + 2) as i64;
    assert_eq!(
        store.count_warnings(&filters).await.expect("count"),
        expected_total
    );
    let page = store
        .list_warnings(&filters, Some(WarningOrderBy::CreatedAt), true, 10, 0)
        .await
        .expect("list page");
    assert_eq!(page.len(), 10);

    let trend_since = chrono::Utc::now().date_naive() - chrono::Duration::days(29);
    let summary = store
        .summarize_warnings(&filters, trend_since)
        .await
        .expect("summarize warnings");
    let unacked = |severity: &str| {
        summary
            .unacked_by_severity
            .get(severity)
            .copied()
            .unwrap_or(0)
    };
    assert_eq!(unacked("critical"), 1);
    assert_eq!(unacked("high"), 0, "acknowledged warnings are not open");
    assert_eq!(unacked("medium"), PAST_CAP as i64);
    assert_eq!(
        summary.warning_types,
        vec!["aggtest_alpha", "aggtest_beta", "aggtest_gamma"]
    );
    assert_eq!(summary.regions, vec!["EU", "Tunisia"]);
    let trend_total: i64 = summary.daily_by_severity.iter().map(|(_, _, n)| n).sum();
    assert_eq!(trend_total, expected_total, "the trend counts every match");
    assert!(summary
        .daily_by_severity
        .iter()
        .all(|(day, _, _)| *day >= trend_since));

    // Filters narrow the summary exactly like the list.
    let critical_only = WarningListFilters {
        search: Some(p.clone()),
        severities: vec!["critical".to_string()],
        ..Default::default()
    };
    let narrowed = store
        .summarize_warnings(&critical_only, trend_since)
        .await
        .expect("summarize narrowed");
    assert_eq!(narrowed.warning_types, vec!["aggtest_alpha"]);
    assert_eq!(narrowed.regions, vec!["Tunisia"]);
    assert!(!narrowed.unacked_by_severity.contains_key("medium"));

    sqlx::query("DELETE FROM warnings WHERE title LIKE $1")
        .bind(format!("{p}%"))
        .execute(&pool)
        .await
        .expect("cleanup warnings");
}

/// The newest observations per company are fetched in one partitioned SQL
/// query (ROW_NUMBER), not with one query per company.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn recent_observations_per_entity_are_bounded_in_sql() {
    use apex_core::entities::{Observation, ObservationType};
    use chrono::{Duration, Utc};

    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let entity_a = Uuid::new_v4();
    let entity_b = Uuid::new_v4();
    let base = Utc::now() - Duration::days(1);
    let mut inserted: Vec<Uuid> = Vec::new();

    for (entity, label) in [(entity_a, "a"), (entity_b, "b")] {
        for i in 0..15 {
            let observation = Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::WebChange,
                entity_id: Some(entity),
                entity_type: Some("company".to_string()),
                ts_utc: base + Duration::minutes(i),
                value: serde_json::json!({"label": label, "seq": i}),
                provenance: serde_json::json!({"source": "test"}),
                confidence: 0.5,
                // Distinct created_at ordering: newest is i = 14.
                created_at: base + Duration::minutes(i),
            };
            store.insert_observation(&observation).await.unwrap();
            inserted.push(observation.id);
        }
    }

    let rows = store
        .get_recent_observations_for_entities(&[entity_a, entity_b], 12)
        .await
        .expect("partitioned observations query");
    assert_eq!(rows.len(), 24, "12 newest per entity, two entities");
    for entity in [entity_a, entity_b] {
        let for_entity: Vec<_> = rows
            .iter()
            .filter(|r| r.entity_id == Some(entity))
            .collect();
        assert_eq!(for_entity.len(), 12);
        let seqs: Vec<i64> = for_entity
            .iter()
            .map(|r| r.value.get("seq").and_then(|v| v.as_i64()).unwrap())
            .collect();
        assert!(
            seqs.contains(&14) && !seqs.contains(&2),
            "the newest observations must win: {seqs:?}"
        );
    }

    assert!(store
        .get_recent_observations_for_entities(&[], 12)
        .await
        .expect("empty entity set")
        .is_empty());

    for id in inserted {
        sqlx::query("DELETE FROM observations WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

/// Store-side triage support: bounded scoring attempts, dimension updates
/// only when supplied, reopening on a new sighting, SQL band counts from the
/// configured thresholds, transition enforcement and override attribution.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn triage_store_support_enforces_attempts_and_transitions() {
    use apex_core::triage::{TriageDimensions, TriageThresholds, TriageWeights};

    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let source_a = Uuid::new_v4();
    let source_b = Uuid::new_v4();
    // `triage_queue.item_type` is constrained to insight|warning|alert.
    let item_type = "alert".to_string();
    let mut inserted = Vec::new();

    let insert = |source: Uuid, attempts: i32, score: f64| {
        let pool = pool.clone();
        let item_type = item_type.clone();
        async move {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO triage_queue \
                     (item_type, source_id, title, description, composite_score, status, triage_attempts) \
                 VALUES ($1, $2, 'title', 'description', $3, 'pending', $4) \
                 RETURNING id",
            )
            .bind(&item_type)
            .bind(source)
            .bind(score)
            .bind(attempts)
            .fetch_one(&pool)
            .await
            .unwrap();
            id
        }
    };

    // Exhausted item is not claimable; fresh item is, and consumes an attempt.
    let exhausted = insert(source_a, 3, 0.0).await;
    let fresh = insert(source_b, 0, 0.0).await;
    inserted.push(exhausted);
    inserted.push(fresh);

    let claimed = store
        .claim_unscored_triage_items(500, 3)
        .await
        .expect("claim unscored items");
    let claimed_ids: Vec<Uuid> = claimed.iter().map(|item| item.id).collect();
    assert!(
        claimed_ids.contains(&fresh) && !claimed_ids.contains(&exhausted),
        "only items below the attempt cap are claimable: {claimed_ids:?}"
    );
    let fresh_attempts: i32 =
        sqlx::query_scalar("SELECT triage_attempts FROM triage_queue WHERE id = $1")
            .bind(fresh)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fresh_attempts, 1, "a claim consumes exactly one attempt");

    // Normative transition: pending -> acknowledged is valid.
    assert!(
        store
            .transition_triage_item(fresh, "acknowledged")
            .await
            .unwrap()
            .is_some(),
        "pending -> acknowledged must be accepted"
    );
    // Terminal transition: acknowledged -> pending is not a valid edge.
    assert!(
        store
            .transition_triage_item(fresh, "pending")
            .await
            .unwrap()
            .is_none(),
        "acknowledged -> pending must be rejected"
    );
    // Unknown id is None (404, not an error).
    assert!(store
        .transition_triage_item(Uuid::new_v4(), "resolved")
        .await
        .unwrap()
        .is_none());

    // Resolving then a new sighting reopens the row and bumps occurrences.
    assert!(store
        .transition_triage_item(fresh, "resolved")
        .await
        .unwrap()
        .is_some());
    assert!(store.reopen_triage_item_on_sighting(fresh).await.unwrap());
    let (status, occurrences, resolved_at): (String, i32, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as(
            "SELECT status::text, occurrence_count, resolved_at FROM triage_queue WHERE id = $1",
        )
        .bind(fresh)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(occurrences, 2);
    assert!(resolved_at.is_none(), "reopening clears resolved_at");

    // A failed scoring pass (None dimensions) must not touch dimensions...
    let before: (f64, f64, f64, String) = sqlx::query_as(
        "SELECT urgency, impact, composite_score, status::text FROM triage_queue WHERE id = $1",
    )
    .bind(fresh)
    .fetch_one(&pool)
    .await
    .unwrap();
    let weights = TriageWeights::default();
    assert!(!store
        .apply_triage_scores(fresh, None, &weights)
        .await
        .expect("None dimensions is a recorded failure, not an error"));
    let after_failure: (f64, f64, f64, String) = sqlx::query_as(
        "SELECT urgency, impact, composite_score, status::text FROM triage_queue WHERE id = $1",
    )
    .bind(fresh)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        after_failure, before,
        "None dimensions must not zero scores"
    );

    // ... and supplied dimensions are applied and recorded.
    let dimensions = TriageDimensions {
        urgency: 0.9,
        impact: 0.8,
        actionability: 0.7,
        novelty: 0.6,
        confidence: 0.9,
    };
    assert!(store
        .apply_triage_scores(fresh, Some(&dimensions), &weights)
        .await
        .expect("apply supplied dimensions"));
    let composite: f64 =
        sqlx::query_scalar("SELECT composite_score FROM triage_queue WHERE id = $1")
            .bind(fresh)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(composite > 0.0, "supplied dimensions must set the score");

    // Override attribution is recorded.
    assert!(store
        .override_triage_score(fresh, 0.95, "analyst-7")
        .await
        .unwrap()
        .is_some());
    let overridden_by: Option<String> =
        sqlx::query_scalar("SELECT overridden_by FROM triage_queue WHERE id = $1")
            .bind(fresh)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(overridden_by.as_deref(), Some("analyst-7"));

    // Band counts are computed in SQL from the configured thresholds and only
    // count pending rows; return the item to pending first.
    assert!(store
        .transition_triage_item(fresh, "resolved")
        .await
        .unwrap()
        .is_some());
    assert!(store.reopen_triage_item_on_sighting(fresh).await.unwrap());
    let bands = store
        .count_triage_bands(&TriageThresholds::default())
        .await
        .expect("band counts");
    let total_pending: i64 = bands.iter().map(|(_, count)| count).sum();
    assert!(total_pending >= 1, "the reopened item is pending");
    let expected_band = apex_core::triage::score_to_band(0.95, &TriageThresholds::default());
    assert!(
        bands
            .iter()
            .find(|(band, _)| band.as_str() == expected_band)
            .map(|(_, count)| *count)
            .unwrap_or(0)
            >= 1,
        "the overridden item is counted in its configured band: {bands:?}"
    );

    for id in inserted {
        sqlx::query("DELETE FROM triage_queue WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

/// DNS posture is a per-domain property: repeated scans must render once per
/// domain (newest measurement wins), including rows written through the
/// canonical `DnsPosture` observation type.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn dns_posture_reads_deduplicate_domains() {
    use apex_core::entities::{Observation, ObservationType};
    use chrono::{Duration, Utc};

    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let suffix = Uuid::new_v4().simple().to_string();
    let domains = [
        format!("a-{suffix}.test"),
        format!("b-{suffix}.test"),
        format!("c-{suffix}.test"),
    ];

    let base = Utc::now() - Duration::hours(1);
    let mut inserted = Vec::new();
    let mut step = 0i64;
    // > 20 rows so the dedicated-table fallback is not selected and the
    // observation path is what is asserted.
    for domain in &domains {
        for _ in 0..9 {
            step += 1;
            let observation = Observation {
                id: Uuid::new_v4(),
                observation_type: ObservationType::DnsPosture,
                entity_id: Some(Uuid::new_v4()),
                entity_type: Some("company".to_string()),
                ts_utc: base + Duration::seconds(step),
                value: serde_json::json!({
                    "domain": domain,
                    "has_spf": true,
                    "has_dkim": false,
                    "dkim_status": "unknown",
                    "dkim_unknown_reason": "resolver error",
                    "has_dmarc": false,
                    "posture_score": 0.3,
                }),
                provenance: serde_json::json!({"source": "test", "content_hash": format!("dns_{step}")}),
                confidence: 0.9,
                created_at: base + Duration::seconds(step),
            };
            store
                .insert_observation(&observation)
                .await
                .expect("insert dns posture observation");
            inserted.push(observation.id);
        }
    }

    let rows = store
        .get_dns_posture_entries(200)
        .await
        .expect("read dns posture");
    for domain in &domains {
        let matching: Vec<_> = rows
            .iter()
            .filter(|row| row.value.get("domain").and_then(|v| v.as_str()) == Some(domain.as_str()))
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "each domain must render exactly once: {domain}"
        );
        assert_eq!(
            matching[0]
                .value
                .get("dkim_status")
                .and_then(|v| v.as_str()),
            Some("unknown"),
            "an unmeasured DKIM lookup stays unknown, never absent"
        );
    }

    for id in inserted {
        sqlx::query("DELETE FROM observations WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

/// The LLM cache binds the rendered prompt into the key, refuses to store an
/// ungrounded response, and treats expired entries as misses.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn llm_cache_is_grounded_prompt_bound_and_expiring() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let workflow = format!("cache_test_{}", Uuid::new_v4().simple());
    let base_key = "evidence-key-1";
    let prompt = "rendered prompt {{entity}}";
    let evidence = [Uuid::new_v4()];

    // Ungrounded responses are never cached.
    let stored = store
        .put_llm_cache_grounded(
            base_key,
            &workflow,
            "model-v1",
            "prompt-v1",
            &evidence,
            prompt,
            false,
            chrono::Duration::hours(1),
            r#"{"headline":"ungrounded"}"#,
        )
        .await
        .expect("ungrounded put");
    assert!(!stored, "an ungrounded response must not be cached");
    assert!(store
        .get_llm_cache_for_prompt(base_key, prompt)
        .await
        .expect("read ungrounded")
        .is_none());

    // A grounded response is cached under the prompt-bound key.
    let stored = store
        .put_llm_cache_grounded(
            base_key,
            &workflow,
            "model-v1",
            "prompt-v1",
            &evidence,
            prompt,
            true,
            chrono::Duration::hours(1),
            r#"{"headline":"grounded"}"#,
        )
        .await
        .expect("grounded put");
    assert!(stored);
    assert_eq!(
        store
            .get_llm_cache_for_prompt(base_key, prompt)
            .await
            .expect("read grounded")
            .as_deref(),
        Some(r#"{"headline":"grounded"}"#)
    );
    // The bare evidence key must not hit: the rendered prompt is part of the key.
    assert!(
        store
            .get_llm_cache(base_key)
            .await
            .expect("bare key read")
            .is_none(),
        "the prompt must be hashed into the effective cache key"
    );
    // A different rendered prompt is a different entry.
    assert!(store
        .get_llm_cache_for_prompt(base_key, "a different rendering")
        .await
        .expect("different prompt read")
        .is_none());

    // Expiry makes the entry a miss.
    sqlx::query(
        "UPDATE llm_cache SET expires_at = NOW() - INTERVAL '1 second' WHERE workflow = $1",
    )
    .bind(&workflow)
    .execute(&pool)
    .await
    .unwrap();
    assert!(store
        .get_llm_cache_for_prompt(base_key, prompt)
        .await
        .expect("expired read")
        .is_none());
    assert_eq!(
        store
            .count_expired_llm_cache_entries(&workflow)
            .await
            .expect("count expired"),
        1
    );

    sqlx::query("DELETE FROM llm_cache WHERE workflow = $1")
        .bind(&workflow)
        .execute(&pool)
        .await
        .unwrap();
}

/// #132: the company list's region/tier filters, LIMIT/OFFSET and the whole-
/// filtered-set totals run in SQL. The old handler loaded every match and then
/// filtered/paged it in memory, so these assertions pin the SQL predicates.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn company_list_region_tier_filters_and_pagination_run_in_sql() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("CLST");

    let scores = [0.90_f64, 0.72, 0.60, 0.40, 0.10];
    let mut inserted = Vec::new();
    for i in 0..30usize {
        let mut company = Company::new(format!("{p} Co {i:03}"), CompanyType::Oem);
        company.region = Some(match i % 3 {
            0 => "TN".to_string(),
            1 => "europe".to_string(),
            _ => "us".to_string(),
        });
        company.risk_score = scores[i % 5];
        company.metadata = serde_json::json!({"is_competitor": i % 2 == 0});
        store
            .insert_company(&company)
            .await
            .expect("insert company");
        inserted.push(company.id);
    }

    let filters = CompanyListFilters {
        search: Some(p.clone()),
        ..Default::default()
    };
    let all = store
        .list_companies_filtered(
            &filters,
            None,
            None,
            Some(CompanyOrderBy::Name),
            false,
            500,
            0,
        )
        .await
        .expect("all filtered companies");
    assert_eq!(all.len(), 30);

    // Region filter through canonical aliases: "Tunisia" matches "TN", "EU"
    // matches "europe", "United States" matches "us".
    for (requested, expected_len) in [("Tunisia", 10), ("EU", 10), ("United States", 10)] {
        let rows = store
            .list_companies_filtered(
                &filters,
                Some(requested),
                None,
                Some(CompanyOrderBy::Name),
                false,
                500,
                0,
            )
            .await
            .expect("region-filtered companies");
        assert_eq!(rows.len(), expected_len, "region={requested}");
        assert!(rows.iter().all(|row| row.name.starts_with(p.as_str())));
    }

    // Tier filter bands on the truncated 0–100 score: 0.90 → T1.
    let t1 = store
        .list_companies_filtered(
            &filters,
            None,
            Some("T1"),
            Some(CompanyOrderBy::Name),
            false,
            500,
            0,
        )
        .await
        .expect("tier T1");
    assert_eq!(t1.len(), 6);
    assert!(t1.iter().all(|row| {
        let score = (row.risk_score.expect("seeded score") * 100.0) as i64;
        score >= 85
    }));

    // Combined filters intersect.
    let combined = store
        .list_companies_filtered(
            &filters,
            Some("Tunisia"),
            Some("T1"),
            Some(CompanyOrderBy::Name),
            false,
            500,
            0,
        )
        .await
        .expect("combined filters");
    assert_eq!(combined.len(), 2);
    for row in &combined {
        assert_eq!(row.region.as_deref(), Some("TN"));
        assert!((row.risk_score.expect("seeded score") * 100.0) as i64 >= 85);
    }

    // Pagination: disjoint SQL pages that concatenate to the full ordered set.
    let mut paged = Vec::new();
    for offset in (0..30).step_by(7) {
        let page = store
            .list_companies_filtered(
                &filters,
                None,
                None,
                Some(CompanyOrderBy::Name),
                false,
                7,
                offset as i64,
            )
            .await
            .expect("page");
        assert!(page.len() <= 7);
        paged.extend(page);
    }
    assert_eq!(paged.len(), 30);
    let paged_ids: HashSet<Uuid> = paged.iter().map(|row| row.id).collect();
    assert_eq!(paged_ids.len(), 30, "pages must not overlap");
    let page_names: Vec<&str> = paged.iter().map(|row| row.name.as_str()).collect();
    let mut sorted = page_names.clone();
    sorted.sort_unstable();
    assert_eq!(page_names, sorted, "SQL ordering holds across pages");

    // Totals describe the whole filtered set, not one page: 30 rows, 15
    // competitors (even indexes), 12 high-risk (0.90/0.72 bands), average
    // truncated risk 1632/30 = 54.
    let (total, competitors, high_risk, avg_risk) = store
        .summarize_companies_filtered(&filters, None, None)
        .await
        .expect("summary");
    assert_eq!((total, competitors, high_risk, avg_risk), (30, 15, 12, 54));

    let (region_total, _, region_high_risk, _) = store
        .summarize_companies_filtered(&filters, Some("Tunisia"), None)
        .await
        .expect("region summary");
    // Tunisia rows are i % 3 == 0; their 0.90/0.72 scores land on i = 0, 6,
    // 15, 21.
    assert_eq!((region_total, region_high_risk), (10, 4));

    let (combined_total, ..) = store
        .summarize_companies_filtered(&filters, Some("Tunisia"), Some("T1"))
        .await
        .expect("combined summary");
    assert_eq!(combined_total, 2, "combined totals match the filtered set");

    // Region breakdown groups the filtered set by raw stored region.
    let regions = store
        .count_companies_by_region_filtered(&filters, None, None)
        .await
        .expect("region counts");
    let by_region: std::collections::HashMap<String, i64> = regions.into_iter().collect();
    assert_eq!(by_region.get("TN"), Some(&10));
    assert_eq!(by_region.get("europe"), Some(&10));
    assert_eq!(by_region.get("us"), Some(&10));

    // Per-entity counts are bounded to the requested ids (the current page).
    let counted_id = inserted[0];
    for (idx, title) in [(0, "one"), (1, "two")] {
        store
            .insert_warning(
                "clst_metric",
                &format!("{p} warning {title}"),
                Some(&format!("clst description {idx}")),
                "high",
                Some("Tunisia"),
                None,
                Some(vec![counted_id]),
                None,
                Some(0.8),
                true,
            )
            .await
            .expect("insert warning");
    }
    store
        .insert_insight(
            &format!("{p} insight one"),
            &format!("summary {} for {p}", alpha_token(0)),
            Some("competitive"),
            None,
            Some(0.5),
            None,
            Some(vec![counted_id]),
            None,
            None,
        )
        .await
        .expect("insert insight");

    let page_ids = vec![counted_id, inserted[1]];
    let counts = store
        .get_warning_counts_for_entity_ids(&page_ids)
        .await
        .expect("warning counts");
    assert_eq!(counts, vec![(counted_id, 2)]);
    let counts = store
        .get_insight_counts_for_entity_ids(&page_ids)
        .await
        .expect("insight counts");
    assert_eq!(counts, vec![(counted_id, 1)]);

    for id in &inserted {
        sqlx::query("DELETE FROM warnings WHERE $1 = ANY(entity_ids)")
            .bind(*id)
            .execute(&pool)
            .await
            .expect("cleanup warnings");
        sqlx::query("DELETE FROM insights WHERE $1 = ANY(entity_ids)")
            .bind(*id)
            .execute(&pool)
            .await
            .expect("cleanup insights");
        sqlx::query("DELETE FROM companies WHERE id = $1")
            .bind(*id)
            .execute(&pool)
            .await
            .expect("cleanup companies");
    }
}

/// Calibration samples must decode: the record includes `predicted_at` and
/// `expected_by`, and a production incident showed the query selecting only
/// some of the struct's columns ("no column found for name: predicted_at") —
/// zero-row tests cannot catch that, so this test seeds a resolved row.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn resolved_calibration_samples_decode_every_struct_field() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("CAL");

    sqlx::query(
        "INSERT INTO stats_alert_calibration_events (
             id, entity_id, feature_vector, alert_level, predicted_at, expected_by,
             actual_outcome_within_30d, resolved_at, metadata
         ) VALUES (
             gen_random_uuid(), gen_random_uuid(), '{}'::jsonb, $1, now() - interval '35 days',
             now() - interval '5 days', TRUE, now() - interval '4 days', '{\"k\":1}'::jsonb
         )",
    )
    .bind(format!("{p}_level"))
    .execute(&pool)
    .await
    .expect("insert calibration event");

    let samples = store
        .list_resolved_stats_alert_calibration_samples(None, 100)
        .await
        .expect("resolved samples decode without missing-column errors");
    assert!(
        samples
            .iter()
            .any(|row| row.alert_level == format!("{p}_level")),
        "the seeded resolved sample must be returned"
    );
}

/// Embedding source text must be NULL-safe: production had 496
/// `failed to fetch person source text` errors because `COALESCE(narrative,
/// full_name)` is NULL for rows lacking both, and decoding NULL into `String`
/// errors. The lookup falls back to `name` and returns `None` only when every
/// candidate column is NULL.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn person_source_text_is_null_safe_and_falls_back_to_name() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("ESRC");

    // A normal person: `name` is populated, narrative/full_name are not.
    let person = Person::new(format!("{p} Person"), RoleFamily::Procurement);
    store.insert_person(&person).await.expect("insert person");
    let text = store
        .entity_source_text("person", &person.id.to_string())
        .await
        .expect("source text lookup must not error on NULL narrative/full_name")
        .expect("name fallback yields source text");
    assert!(text.contains(&p), "expected the person name, got {text:?}");

    // Explicitly NULL out every candidate column: the lookup returns None
    // (skip), never a decode error.
    sqlx::query("UPDATE persons SET narrative = NULL, full_name = NULL, name = '' WHERE id = $1")
        .bind(person.id)
        .execute(&pool)
        .await
        .expect("null out person text");
    let empty = store
        .entity_source_text("person", &person.id.to_string())
        .await
        .expect("empty source text must not error");
    assert_eq!(empty.as_deref(), Some(""));
}

/// #133: the detail page's warning/insight totals and description are real
/// store reads — a COUNT(*) beyond the 50-row display cap and
/// `companies.narrative`, never the capped list length or the legal name.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn company_detail_counts_and_narrative_are_real_reads() {
    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());
    let p = prefix("CDET");
    let company = Company::new(format!("{p} Co"), CompanyType::Oem);
    store
        .insert_company(&company)
        .await
        .expect("insert company");

    for i in 0..55usize {
        store
            .insert_warning(
                "cdet_metric",
                &format!("{p} warning {i:04}"),
                Some(&format!("cdet description {i}")),
                "medium",
                None,
                None,
                Some(vec![company.id]),
                None,
                Some(0.5),
                true,
            )
            .await
            .expect("insert warning");
        store
            .insert_insight(
                &format!("{p} Insight {i:04}"),
                &format!("summary {} for {p}", alpha_token(i)),
                Some("competitive"),
                None,
                Some(0.5),
                None,
                Some(vec![company.id]),
                None,
                None,
            )
            .await
            .expect("insert insight");
    }
    // An internal insight must not inflate the visible count.
    store
        .insert_insight(
            &format!("{p} internal llm row"),
            &format!("summary {} for {p}", alpha_token(200)),
            Some("llm_narrative"),
            None,
            Some(0.5),
            None,
            Some(vec![company.id]),
            None,
            None,
        )
        .await
        .expect("insert internal insight");

    let capped_warnings = store
        .get_warnings_by_entity_ids(&[company.id], 50)
        .await
        .expect("capped warnings");
    assert_eq!(capped_warnings.len(), 50, "the display list is capped");
    assert_eq!(
        store
            .count_warnings_for_entity(company.id)
            .await
            .expect("warning count"),
        55,
        "the total is a real count, not the capped page length"
    );

    let capped_insights = store
        .get_insights_by_entity_ids(&[company.id], 50)
        .await
        .expect("capped insights");
    assert!(capped_insights.len() <= 50, "the display list is capped");
    assert_eq!(
        store
            .count_insights_for_entity(company.id)
            .await
            .expect("insight count"),
        55,
        "the visible total ignores capped pages and internal rows"
    );

    // The description is the stored narrative, not the legal name.
    assert_eq!(
        store
            .get_company_narrative(company.id)
            .await
            .expect("narrative read"),
        None
    );
    sqlx::query("UPDATE companies SET narrative = $1 WHERE id = $2")
        .bind("Real stored narrative.")
        .bind(company.id)
        .execute(&pool)
        .await
        .expect("set narrative");
    assert_eq!(
        store
            .get_company_narrative(company.id)
            .await
            .expect("narrative read"),
        Some("Real stored narrative.".to_string())
    );

    sqlx::query("DELETE FROM warnings WHERE $1 = ANY(entity_ids)")
        .bind(company.id)
        .execute(&pool)
        .await
        .expect("cleanup warnings");
    sqlx::query("DELETE FROM insights WHERE $1 = ANY(entity_ids)")
        .bind(company.id)
        .execute(&pool)
        .await
        .expect("cleanup insights");
    sqlx::query("DELETE FROM companies WHERE id = $1")
        .bind(company.id)
        .execute(&pool)
        .await
        .expect("cleanup company");
}

/// #157: snapshot metrics compare the latest measured value per period and
/// per entity; additive metrics keep summing their buckets.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn snapshot_trend_metrics_use_latest_per_period_not_sum() {
    use apex_store::postgres::trends::{is_snapshot_metric, TrendComparisonQuery};

    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let previous_start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let current_start = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let current_latest = chrono::NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let range_end = chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

    let comparison = |metric: &str| {
        let store = store.clone();
        let metric = metric.to_string();
        async move {
            store
                .get_trend_comparison(&TrendComparisonQuery {
                    metric_name: metric,
                    current_period_start: current_start,
                    previous_period_start: previous_start,
                    period_duration_days: 30,
                    entity_type: None,
                    entity_id: None,
                })
                .await
                .expect("trend comparison")
        }
    };

    for metric in [
        "companies_tracked",
        "persons_tracked",
        "active_recipes",
        "unacknowledged_warnings",
    ] {
        assert!(is_snapshot_metric(metric), "{metric} must be a gauge");
        store
            .upsert_trend_rollup(previous_start, "monthly", None, None, metric, 40)
            .await
            .expect("seed previous gauge");
        store
            .upsert_trend_rollup(current_start, "monthly", None, None, metric, 50)
            .await
            .expect("seed current gauge");
        // The later measurement inside the period must win; 50+55 (or
        // 40+50+55) would be the old SUM-style fabrication.
        store
            .upsert_trend_rollup(current_latest, "monthly", None, None, metric, 55)
            .await
            .expect("seed latest gauge");

        let result = comparison(metric).await;
        assert_eq!(
            result.current_total, 55,
            "{metric}: latest current-period value, not the sum"
        );
        assert_eq!(
            result.previous_total, 40,
            "{metric}: latest previous-period value, not the sum"
        );
    }

    // Additive metrics keep summing their buckets.
    store
        .upsert_trend_rollup(current_start, "monthly", None, None, "warnings", 3)
        .await
        .expect("seed current additive");
    store
        .upsert_trend_rollup(current_latest, "monthly", None, None, "warnings", 4)
        .await
        .expect("seed latest additive");
    assert_eq!(
        comparison("warnings").await.current_total,
        7,
        "additive metrics still sum all buckets"
    );

    // Entity breakdown: gauges take each entity's latest value, additive
    // metrics keep summing.
    let entity_a = Uuid::new_v4().to_string();
    let entity_b = Uuid::new_v4().to_string();
    store
        .upsert_trend_rollup(
            current_start,
            "monthly",
            Some("company"),
            Some(&entity_a),
            "companies_tracked",
            10,
        )
        .await
        .expect("seed gauge entity a");
    store
        .upsert_trend_rollup(
            current_latest,
            "monthly",
            Some("company"),
            Some(&entity_a),
            "companies_tracked",
            13,
        )
        .await
        .expect("seed latest gauge entity a");
    store
        .upsert_trend_rollup(
            current_start,
            "monthly",
            Some("company"),
            Some(&entity_b),
            "companies_tracked",
            7,
        )
        .await
        .expect("seed gauge entity b");

    let breakdown = store
        .get_entity_metric_breakdown("companies_tracked", "monthly", current_start, range_end, 10)
        .await
        .expect("gauge breakdown");
    assert_eq!(breakdown.len(), 2);
    assert_eq!(breakdown[0].entity_id, entity_a);
    assert_eq!(
        breakdown[0].value, 13,
        "each entity's latest value, not 10+13"
    );
    assert_eq!(breakdown[1].value, 7);

    store
        .upsert_trend_rollup(
            current_start,
            "monthly",
            Some("company"),
            Some(&entity_a),
            "warnings",
            3,
        )
        .await
        .expect("seed additive entity");
    store
        .upsert_trend_rollup(
            current_latest,
            "monthly",
            Some("company"),
            Some(&entity_a),
            "warnings",
            4,
        )
        .await
        .expect("seed later additive entity");
    let breakdown = store
        .get_entity_metric_breakdown("warnings", "monthly", current_start, range_end, 10)
        .await
        .expect("additive breakdown");
    assert_eq!(breakdown[0].value, 7, "additive entity breakdown sums");

    sqlx::query(
        "DELETE FROM trend_rollups \
         WHERE bucket_date >= $1 AND bucket_date <= $2 AND metric_name = ANY($3)",
    )
    .bind(previous_start)
    .bind(range_end)
    .bind(vec![
        "companies_tracked",
        "persons_tracked",
        "active_recipes",
        "unacknowledged_warnings",
        "warnings",
    ])
    .execute(&pool)
    .await
    .expect("cleanup trend rollups");
}

/// Inserts one rollup row with an explicit `updated_at`, so "latest per
/// period" is deterministic regardless of statement timing.
async fn insert_trend_rollup_at(
    pool: &PgPool,
    bucket_date: chrono::NaiveDate,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    metric_name: &str,
    metric_value: i64,
    updated_at: &str,
) {
    sqlx::query(
        "INSERT INTO trend_rollups \
             (bucket_date, bucket_type, entity_type, entity_id, metric_name, \
              metric_value, created_at, updated_at) \
         VALUES ($1, 'monthly', $2, $3, $4, $5, $6::timestamptz, $6::timestamptz)",
    )
    .bind(bucket_date)
    .bind(entity_type)
    .bind(entity_id)
    .bind(metric_name)
    .bind(metric_value)
    .bind(updated_at)
    .execute(pool)
    .await
    .expect("insert trend rollup row");
}

/// #157 residual: the summary cards must fold snapshot metrics to the latest
/// measurement per period before aggregating; additive metrics still sum.
#[tokio::test]
#[ignore = "requires PostgreSQL; run with --ignored"]
async fn trend_summary_uses_latest_per_period_for_gauges() {
    use apex_store::postgres::trends::TrendQuery;

    let pool = setup().await;
    let store = PgStore::from_pool(pool.clone());

    let first_period = chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let second_period = chrono::NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
    let range_start = first_period;
    let range_end = chrono::NaiveDate::from_ymd_opt(2025, 4, 30).unwrap();

    let gauges = [
        "companies_tracked",
        "persons_tracked",
        "active_recipes",
        "unacknowledged_warnings",
    ];
    for metric in gauges {
        // Two measurements in the first period: only the freshest (130, at
        // 02:00) counts; the stale 100 from 01:00 must not be added.
        insert_trend_rollup_at(
            &pool,
            first_period,
            Some("company"),
            Some("a"),
            metric,
            100,
            "2025-03-01T01:00:00Z",
        )
        .await;
        insert_trend_rollup_at(
            &pool,
            first_period,
            Some("company"),
            Some("b"),
            metric,
            130,
            "2025-03-01T02:00:00Z",
        )
        .await;
        insert_trend_rollup_at(
            &pool,
            second_period,
            Some("company"),
            Some("a"),
            metric,
            120,
            "2025-04-01T01:00:00Z",
        )
        .await;

        let summary = store
            .get_trend_summary(&TrendQuery {
                bucket_type: "monthly".to_string(),
                metric_name: metric.to_string(),
                from_date: Some(range_start),
                to_date: Some(range_end),
                entity_type: None,
                entity_id: None,
                limit: Some(500),
            })
            .await
            .expect("gauge summary");
        assert_eq!(
            summary.data_points, 2,
            "{metric}: one point per period, not per raw row"
        );
        assert_eq!(
            summary.total, 250,
            "{metric}: latest per period (130+120), not 100+130+120"
        );
        assert_eq!(summary.min, 120, "{metric}");
        assert_eq!(summary.max, 130, "{metric}");
        assert!(
            (summary.average - 125.0).abs() < f64::EPSILON,
            "{metric}: average over per-period latest values"
        );

        // An entity-scoped summary follows the same gauge rule: entity a has
        // 100 in March and 120 in April.
        let scoped = store
            .get_trend_summary(&TrendQuery {
                bucket_type: "monthly".to_string(),
                metric_name: metric.to_string(),
                from_date: Some(range_start),
                to_date: Some(range_end),
                entity_type: Some("company".to_string()),
                entity_id: Some("a".to_string()),
                limit: Some(500),
            })
            .await
            .expect("entity-scoped gauge summary");
        assert_eq!(scoped.data_points, 2, "{metric}: entity a has two periods");
        assert_eq!(scoped.total, 220, "{metric}: entity a latest per period");
        assert_eq!(scoped.max, 120, "{metric}");
    }

    // Additive metrics keep summing every row in the range.
    insert_trend_rollup_at(
        &pool,
        first_period,
        Some("company"),
        Some("a"),
        "warnings",
        30,
        "2025-03-01T01:00:00Z",
    )
    .await;
    insert_trend_rollup_at(
        &pool,
        first_period,
        Some("company"),
        Some("b"),
        "warnings",
        20,
        "2025-03-01T02:00:00Z",
    )
    .await;
    insert_trend_rollup_at(
        &pool,
        second_period,
        Some("company"),
        Some("a"),
        "warnings",
        10,
        "2025-04-01T01:00:00Z",
    )
    .await;
    let summary = store
        .get_trend_summary(&TrendQuery {
            bucket_type: "monthly".to_string(),
            metric_name: "warnings".to_string(),
            from_date: Some(range_start),
            to_date: Some(range_end),
            entity_type: None,
            entity_id: None,
            limit: Some(500),
        })
        .await
        .expect("additive summary");
    assert_eq!(summary.data_points, 3, "additive metrics count every row");
    assert_eq!(summary.total, 60, "additive metrics still sum");
    assert!((summary.average - 20.0).abs() < f64::EPSILON);

    sqlx::query(
        "DELETE FROM trend_rollups \
         WHERE bucket_date >= $1 AND bucket_date <= $2 AND metric_name = ANY($3)",
    )
    .bind(range_start)
    .bind(range_end)
    .bind(vec![
        "companies_tracked",
        "persons_tracked",
        "active_recipes",
        "unacknowledged_warnings",
        "warnings",
    ])
    .execute(&pool)
    .await
    .expect("cleanup trend rollups");
}
