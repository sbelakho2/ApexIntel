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
