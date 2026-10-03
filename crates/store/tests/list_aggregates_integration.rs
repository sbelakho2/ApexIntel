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
