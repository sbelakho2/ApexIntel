use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use aho_corasick::AhoCorasick;
#[cfg(feature = "llm")]
use apex_core::entities::{Company, CompanyType};
use apex_crawl::client::{CrawlClient, CrawlClientConfig, CrawlRequest};
use apex_crawl::sources::select_sources_for_crawl;
#[cfg(feature = "llm")]
use apex_insights::dynamic_poi_discovery::{
    DiscoveredEntity, DynamicPoiDiscovery, EntityType, PoiCandidate, Recommendation,
};

use crate::*;

const NIGHTLY_STAGE_TIMEOUT: Duration = Duration::from_secs(30);
const NIGHTLY_STAGE_ATTEMPTS: usize = 3;

/// Precomputed entity matching index combining Aho-Corasick automaton with
/// a UUID lookup table.  The automaton matches all known entity names in a
/// single O(n + matches) pass over the text, replacing the previous O(n*m)
/// approach.
struct EntityMatcher {
    automaton: AhoCorasick,
    /// Ordered parallel to the patterns fed to the automaton:
    /// `ids[pattern_index]` gives the UUID for that entity.
    ids: Vec<Uuid>,
    /// Lengths of the original patterns so we can pick the longest match.
    lengths: Vec<usize>,
}

/// Build a case-insensitive lookup table mapping known entity name tokens to
/// their company UUID.  When a crawled page body contains one of these names
/// the resulting observation is linked to that entity so that downstream jobs
/// (recipe_fire, pattern_mining, etc.) can attribute the data.
///
/// Also constructs an Aho-Corasick automaton for efficient single-pass
/// matching.
async fn build_entity_name_lookup(store: &Arc<PgStore>) -> (HashMap<String, Uuid>, EntityMatcher) {
    let index = store.list_entity_name_index().await.unwrap_or_default();
    let mut lookup: HashMap<String, Uuid> = HashMap::with_capacity(index.len());
    for (id, name) in &index {
        // Skip very short names (≤2 chars) to avoid false-positive matches
        // against common words.
        if name.len() > 2 {
            lookup.entry(name.clone()).or_insert(*id);
        }
    }

    // Build Aho-Corasick automaton from the lookup keys.
    let patterns: Vec<&str> = lookup.keys().map(|s| s.as_str()).collect();
    let ids: Vec<Uuid> = lookup.keys().map(|k| lookup[k]).collect();
    let lengths: Vec<usize> = patterns.iter().map(|p| p.len()).collect();
    let automaton = AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(&patterns)
        .unwrap_or_else(|error| panic!("entity name patterns should be valid: {error}"));

    let matcher = EntityMatcher {
        automaton,
        ids,
        lengths,
    };
    (lookup, matcher)
}

/// Scan a text blob for the best matching entity name.  Uses the precomputed
/// Aho-Corasick automaton for O(n + matches) matching.  Returns the entity
/// UUID of the longest match so that more specific names win over shorter
/// prefixes.
fn match_entity_in_text(text: &str, matcher: &EntityMatcher) -> Option<Uuid> {
    let mut best: Option<(usize, Uuid)> = None;
    for mat in matcher.automaton.find_iter(text) {
        let idx = mat.pattern().as_usize();
        let len = matcher.lengths[idx];
        match best {
            Some((best_len, _)) if len <= best_len => {}
            _ => best = Some((len, matcher.ids[idx])),
        }
    }
    best.map(|(_, id)| id)
}

#[cfg(feature = "llm")]
fn normalize_dynamic_company_name(raw: &str) -> Option<String> {
    let normalized = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|character: char| {
            matches!(
                character,
                '-' | '|' | ',' | ':' | ';' | '.' | '(' | ')' | '[' | ']'
            )
        })
        .trim()
        .to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(feature = "llm")]
fn is_discoverable_dynamic_company_name(name: &str) -> bool {
    let Some(normalized) = normalize_dynamic_company_name(name) else {
        return false;
    };
    if normalized.len() < 3 || normalized.len() > 96 {
        return false;
    }

    let lower = normalized.to_ascii_lowercase();
    let blocked_exact = [
        "conference program",
        "registration",
        "speaker list",
        "download brochure",
        "event partners",
        "more exhibitors",
    ];
    if blocked_exact.iter().any(|blocked| lower == *blocked) {
        return false;
    }

    let blocked_fragments = [
        "click here",
        "learn more",
        "read more",
        "sponsor",
        "speaker",
        "conference",
        "summit",
        "expo 202",
        "2026 exhibitors",
        "2025 exhibitors",
    ];
    if blocked_fragments
        .iter()
        .any(|blocked| lower.contains(blocked))
    {
        return false;
    }

    let alpha_count = normalized
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    let token_count = normalized.split_whitespace().count();
    alpha_count >= 2 && token_count <= 8
}

#[cfg(feature = "llm")]
fn recommendation_label(recommendation: &Recommendation) -> &'static str {
    match recommendation {
        Recommendation::AddNow => "add_now",
        Recommendation::ReviewRequired => "review_required",
        Recommendation::MonitorFurther => "monitor_further",
        Recommendation::Discard => "discard",
    }
}

#[cfg(feature = "llm")]
async fn persist_dynamic_discovery_candidates(
    store: &Arc<PgStore>,
    candidates: &[PoiCandidate],
    discovered: &[DiscoveredEntity],
    inserted_names: &mut HashSet<String>,
    now: chrono::DateTime<Utc>,
) -> Result<u64> {
    let insert_limit = std::env::var("CRAWL_DYNAMIC_DISCOVERY_INSERT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 200);

    let discovered_by_name: HashMap<String, &DiscoveredEntity> = discovered
        .iter()
        .map(|entity| (entity.normalized_name.to_ascii_lowercase(), entity))
        .collect();

    let mut inserted = 0_u64;
    for candidate in candidates {
        if inserted as usize >= insert_limit {
            break;
        }
        if !matches!(
            candidate.entity_type,
            EntityType::Company | EntityType::Organization
        ) {
            continue;
        }
        if !matches!(
            candidate.recommended_action,
            Recommendation::AddNow | Recommendation::ReviewRequired
        ) {
            continue;
        }
        if candidate.confidence < 0.72 {
            continue;
        }
        if candidate.co_occurrence_count == 0 && candidate.source_diversity < 2 {
            continue;
        }

        let Some(name) = normalize_dynamic_company_name(&candidate.name) else {
            continue;
        };
        if !is_discoverable_dynamic_company_name(&name) {
            continue;
        }

        let dedup_key = name.to_ascii_lowercase();
        if !inserted_names.insert(dedup_key.clone()) {
            continue;
        }
        if store.get_company_by_name_ci(&name).await?.is_some() {
            continue;
        }

        let discovered_entity = discovered_by_name.get(&dedup_key).copied();
        let mut company = Company::new(
            name.clone(),
            CompanyType::Other("crawl_discovered".to_string()),
        );
        company.metadata = serde_json::json!({
            "discovered_via": "crawl_dynamic_discovery",
            "recommended_action": recommendation_label(&candidate.recommended_action),
            "confidence": candidate.confidence,
            "emergence_score": candidate.emergence_score,
            "co_occurrence_count": candidate.co_occurrence_count,
            "source_diversity": candidate.source_diversity,
            "context": discovered_entity
                .map(|entity| crate::truncate_text(&entity.context, 220))
                .unwrap_or_default(),
            "source_url": discovered_entity
                .map(|entity| entity.source_url.clone())
                .unwrap_or_default(),
            "topics": discovered_entity
                .map(|entity| entity.topics.clone())
                .unwrap_or_default(),
            "geography": discovered_entity
                .map(|entity| entity.geography.clone())
                .unwrap_or_default(),
            "associated_entities": discovered_entity
                .map(|entity| entity.associated_entities.clone())
                .unwrap_or_default(),
            "is_competitor": false,
            "discovery_track": "crawl_dynamic",
        });
        company.created_at = now;
        company.updated_at = now;

        store.insert_company(&company).await?;
        inserted += 1;
        tracing::info!(
            company = %company.name,
            confidence = candidate.confidence,
            recommendation = %recommendation_label(&candidate.recommended_action),
            "crawl_cycle: inserted dynamically discovered company"
        );
    }

    Ok(inserted)
}

#[allow(clippy::disallowed_methods)]
pub(super) async fn run_crawl_cycle(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::CrawlCycle);
    run.start();
    let sources = all_sources();
    let crawl_limit: usize = std::env::var("CRAWL_MAX_SOURCES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let always_include_slugs = ["globes_il_tech"];
    let fetch_sources = select_sources_for_crawl(&sources, 2, crawl_limit, &always_include_slugs);

    if fetch_sources.is_empty() {
        run.skip("crawl_cycle: no enabled tier-1/2 sources configured");
        return run;
    }

    tracing::info!(
        selected = fetch_sources.len(),
        limit = crawl_limit,
        forced_sources = always_include_slugs.join(","),
        "crawl_cycle: source selection complete"
    );

    let proxy_rotator =
        build_proxy_rotator_from_env().map(|rotator| Arc::new(tokio::sync::Mutex::new(rotator)));
    if let Some(rotator) = proxy_rotator.as_ref() {
        let proxy_health = rotator.lock().await.health_summary();
        tracing::info!(proxy_health = %proxy_health, "crawl_cycle: proxy rotation enabled");
    }

    let crawl_client = match CrawlClient::new(CrawlClientConfig {
        proxy_rotator: proxy_rotator.clone(),
        ..CrawlClientConfig::default()
    }) {
        Ok(client) => client,
        Err(error) => {
            run.fail(&format!(
                "crawl_cycle: failed to build crawl client: {error}"
            ));
            return run;
        }
    };

    // Pre-load entity name index so we can link observations to known
    // companies/competitors.  Without this mapping recipe_fire sees zero
    // entity-linked observations and skips, producing no warnings or insights.
    let (entity_lookup, entity_matcher) = build_entity_name_lookup(store).await;
    tracing::info!(
        entity_names = entity_lookup.len(),
        "crawl_cycle: entity name index loaded for observation linking"
    );

    #[cfg(feature = "llm")]
    let mut dynamic_discovery =
        DynamicPoiDiscovery::new(entity_lookup.keys().cloned().collect::<Vec<_>>());
    #[cfg(feature = "llm")]
    let mut discovered_company_names: HashSet<String> = HashSet::new();

    let mut ingested: u64 = 0;
    let mut entity_linked: u64 = 0;
    #[allow(unused_mut)]
    let mut dynamically_discovered_companies: u64 = 0;
    let mut errors: u64 = 0;
    let mut successful_sources: HashSet<String> = HashSet::new();
    let mut failed_sources: HashSet<String> = HashSet::new();

    for src in &fetch_sources {
        let url = src.rss_url.as_deref().unwrap_or(src.url.as_str());
        let prefers_browser_ua = src.slug == "globes_il_tech";
        let request = CrawlRequest::new(url)
            .source_id(&src.slug)
            .requires_proxy(src.needs_proxy)
            .prefer_browser_user_agent(prefers_browser_ua)
            .override_user_agent(if prefers_browser_ua {
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
            } else {
                "ApexIntelBot/1.0 (+https://apex-intel.io/bot)"
            });

        match crawl_client.fetch_text(&request).await {
            Ok(response) => {
                let body = response.body;
                #[cfg(any(feature = "parse", feature = "llm"))]
                let obs_value = match extract_page(&body) {
                    Ok(page) => serde_json::json!({
                        "source_id": src.slug,
                        "url": url,
                        "title": page.title,
                        "description": page.description,
                        "body_excerpt": page.body_text.chars().take(1000).collect::<String>(),
                        "language": page.language,
                    }),
                    Err(_) => serde_json::json!({
                        "source_id": src.slug,
                        "url": url,
                    }),
                };
                #[cfg(not(any(feature = "parse", feature = "llm")))]
                let obs_value = serde_json::json!({
                    "source_id": src.slug,
                    "url": url,
                    "body_len": body.len(),
                });

                let obs = {
                    let mut o = Observation::new(
                        ObservationType::WebChange,
                        Utc::now(),
                        obs_value.clone(),
                        serde_json::json!({
                            "source": src.slug,
                            "tier": src.tier,
                            "category": format!("{:?}", src.category),
                        }),
                    );

                    // Attempt entity linking: scan the crawled page text for a
                    // known company/competitor name and attach its UUID so
                    // downstream jobs (recipe_fire) can attribute the observation.
                    let linkable_text = format!(
                        "{} {} {}",
                        obs_value
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        obs_value
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        obs_value
                            .get("body_excerpt")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    );
                    if let Some(eid) = match_entity_in_text(&linkable_text, &entity_matcher) {
                        o.entity_id = Some(eid);
                        o.entity_type = Some("company".to_string());
                    }
                    o
                };
                match store.insert_observation(&obs).await {
                    Ok(_) => {
                        ingested += 1;
                        if obs.entity_id.is_some() {
                            entity_linked += 1;
                        }
                        #[cfg(feature = "llm")]
                        {
                            let discovery_text = format!(
                                "{}\n{}\n{}",
                                obs_value
                                    .get("title")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or(""),
                                obs_value
                                    .get("description")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or(""),
                                obs_value
                                    .get("body_excerpt")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or(""),
                            );
                            let discovered = dynamic_discovery.process_content(
                                &discovery_text,
                                url,
                                Utc::now().timestamp(),
                            );
                            if !discovered.is_empty() {
                                let candidates = dynamic_discovery.generate_candidates(&discovered);
                                match persist_dynamic_discovery_candidates(
                                    store,
                                    &candidates,
                                    &discovered,
                                    &mut discovered_company_names,
                                    Utc::now(),
                                )
                                .await
                                {
                                    Ok(inserted) => {
                                        dynamically_discovered_companies += inserted;
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            source = %src.slug,
                                            error = %error,
                                            "crawl_cycle: dynamic discovery persistence failed"
                                        );
                                    }
                                }
                            }
                        }
                        successful_sources.insert(src.slug.clone());
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %src.slug,
                            error = %e,
                            "crawl_cycle: failed to store observation"
                        );
                        errors += 1;
                        failed_sources.insert(src.slug.clone());
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    source = %src.slug,
                    category = %error.category().as_str(),
                    error = %error,
                    "crawl_cycle: fetch error"
                );
                errors += 1;
                failed_sources.insert(src.slug.clone());
            }
        }
    }

    let attempted_sources = fetch_sources.len().max(1);
    let success_ratio = ingested as f64 / attempted_sources as f64;
    let min_success_ratio = std::env::var("CRAWL_MIN_SUCCESS_RATIO")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(0.30);

    let failed_sources_list = {
        let mut v: Vec<_> = failed_sources.iter().cloned().collect();
        v.sort();
        v
    };
    let successful_sources_list = {
        let mut v: Vec<_> = successful_sources.iter().cloned().collect();
        v.sort();
        v
    };

    if ingested == 0 || success_ratio < min_success_ratio {
        let failure_summary = format!(
            "crawl_cycle degraded: ingested={} attempted_sources={} success_ratio={:.2} min_success_ratio={:.2} failed_sources={} successful_sources={}",
            ingested,
            fetch_sources.len(),
            success_ratio,
            min_success_ratio,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
            if successful_sources_list.is_empty() { "none".to_string() } else { successful_sources_list.join(",") },
        );

        let _ = store
            .insert_warning(
                "crawl_health",
                "Crawl reliability degraded",
                Some(&failure_summary),
                "high",
                None,
                Some("crawl_cycle"),
                None,
                None,
                Some((1.0 - success_ratio).clamp(0.0, 1.0)),
            )
            .await;

        run.fail(&format!(
            "crawl_cycle: {}/{} sources attempted; {} observations ingested, {} errors; failed_sources=[{}]",
            fetch_sources.len(),
            sources.iter().filter(|s| s.enabled).count(),
            ingested,
            errors,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
        ));
        return run;
    }

    run.succeed(
        ingested,
        &format!(
            "crawl_cycle: {}/{} sources attempted; {} observations ingested ({} entity-linked), {} dynamically discovered companies, {} errors; success_ratio={:.2}",
            fetch_sources.len(),
            sources.iter().filter(|s| s.enabled).count(),
            ingested,
            entity_linked,
            dynamically_discovered_companies,
            errors,
            success_ratio,
        ),
    );
    run
}

pub(super) async fn run_pattern_mining(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let since = Utc::now() - chrono::Duration::hours(24);

    // Materialize candidates from recent observations before reading stats
    match store.materialize_pattern_candidates(since).await {
        Ok(n) => tracing::info!(
            candidates_materialized = n,
            "pattern_mining: materialized candidates"
        ),
        Err(e) => tracing::warn!(error = %e, "pattern_mining: candidate materialization failed"),
    }

    let mining_stats = match super::resilience::run_stage_with_retry(
        "pattern_mining.load_stats",
        NIGHTLY_STAGE_TIMEOUT,
        NIGHTLY_STAGE_ATTEMPTS,
        |_| async { store.get_mining_stats(since).await },
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!(
                "pattern_mining: failed to load mining stats from DB: {e}"
            ));
            return run;
        }
    };
    let stage = process_mining_stage(&MiningStageResult {
        candidates_found: mining_stats.candidates_found,
        candidates_passed_gates: mining_stats.candidates_passed_gates,
        hypotheses_generated: mining_stats.hypotheses_generated,
        recipes_staged: mining_stats.recipes_staged,
        errors: mining_stats.errors,
    });
    match stage.run.status {
        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
            run.succeed(
                stage.items,
                &format!("mining completed: {}", stage.run.notes),
            );
        }
        apex_worker::scheduler::JobStatus::Failed { .. } => {
            run.fail(&format!("mining failed: {}", stage.run.notes));
        }
        _ => {
            run.skip(&format!("mining stage not terminal: {}", stage.run.notes));
        }
    }
    run
}

pub(super) async fn run_hypothesis_generation(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let since = Utc::now() - chrono::Duration::hours(24);
        let mining_stats = match super::resilience::run_stage_with_retry(
            "hypothesis_generation.load_stats",
            NIGHTLY_STAGE_TIMEOUT,
            NIGHTLY_STAGE_ATTEMPTS,
            |_| async { store.get_mining_stats(since).await },
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                run.fail(&format!(
                    "hypothesis_generation: failed to load mining stats from DB: {e}"
                ));
                return run;
            }
        };
        let stage = process_hypothesis_generation_stage(&HypothesisGenerationStageResult {
            candidates_submitted: mining_stats.candidates_passed_gates,
            hypotheses_generated: mining_stats.hypotheses_generated,
            hypotheses_failed: mining_stats
                .candidates_passed_gates
                .saturating_sub(mining_stats.hypotheses_generated),
            recipes_staged: mining_stats.recipes_staged,
            errors: mining_stats.errors,
        });
        match stage.run.status {
            apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                run.succeed(
                    stage.items,
                    &format!("hypothesis gen completed: {}", stage.run.notes),
                );
            }
            apex_worker::scheduler::JobStatus::Failed { .. } => {
                run.fail(&format!("hypothesis gen failed: {}", stage.run.notes));
            }
            _ => {
                run.skip(&format!("hypothesis gen not terminal: {}", stage.run.notes));
            }
        }
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("hypothesis generation requires the `llm` feature");
    }
    run
}

pub(super) async fn run_feature_drift_check(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // Materialize feature rows from recent observations before checking drift
    match store.materialize_feature_rows().await {
        Ok(n) => tracing::info!(
            features_materialized = n,
            "feature_drift_check: materialized features"
        ),
        Err(e) => tracing::warn!(error = %e, "feature_drift_check: feature materialization failed"),
    }

    let drift_stats = match super::resilience::run_stage_with_retry(
        "feature_drift_check.load_stats",
        NIGHTLY_STAGE_TIMEOUT,
        NIGHTLY_STAGE_ATTEMPTS,
        |_| async { store.get_drift_stats().await },
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!(
                "feature_drift_check: failed to load drift stats from DB: {e}"
            ));
            return run;
        }
    };
    let stage = process_drift_stage(&DriftCheckStageResult {
        features_checked: drift_stats.features_checked,
        features_drifted: drift_stats.features_drifted,
        drift_scores: drift_stats.drift_scores,
        alerts_raised: drift_stats.alerts_raised,
        errors: drift_stats.errors,
    });
    match stage.run.status {
        apex_worker::scheduler::JobStatus::Succeeded { .. } => {
            run.succeed(
                stage.items,
                &format!("drift check completed: {}", stage.run.notes),
            );
        }
        apex_worker::scheduler::JobStatus::Failed { .. } => {
            run.fail(&format!("drift check failed: {}", stage.run.notes));
        }
        _ => {
            run.skip(&format!("drift stage not terminal: {}", stage.run.notes));
        }
    }
    run
}
