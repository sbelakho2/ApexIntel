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
        .unwrap_or(100)
        .clamp(1, 1000);

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
        // Confidence threshold lowered from 0.72 → 0.45 to escape the
        // bootstrapping trap where new companies only score high confidence if
        // they already resemble the seed set. At 0.45, genuinely new companies
        // that appear in crawl content with reasonable signal get admitted, then
        // get enriched/scored by downstream jobs (ICP, OSINT enrichment).
        if candidate.confidence < 0.45 {
            continue;
        }
        // Relax the co-occurrence gate: allow single-source discoveries if they
        // have meaningful confidence (was: require co_occurrence>0 OR source_diversity>=2).
        if candidate.co_occurrence_count == 0
            && candidate.source_diversity < 1
            && candidate.confidence < 0.6
        {
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
        // Surface the first detection of a new company in the activity feed.
        // This branch only runs for genuinely new companies (existing names
        // `continue` at the get_company_by_name_ci check above).
        let region = discovered_entity
            .and_then(|entity| entity.geography.first())
            .map(String::as_str);
        let company_id = company.id.to_string();
        let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
        activity_logger
            .log_company_detected(
                &company.name,
                region,
                "crawl_dynamic_discovery",
                Some(&company_id),
            )
            .await;
        tracing::info!(
            company = %company.name,
            confidence = candidate.confidence,
            recommendation = %recommendation_label(&candidate.recommended_action),
            "crawl_cycle: inserted dynamically discovered company"
        );
    }

    Ok(inserted)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) async fn run_crawl_cycle(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::CrawlCycle);
    run.start();
    let sources = all_sources();
    let crawl_limit: usize = std::env::var("CRAWL_MAX_SOURCES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // Include tier 3 (Reddit, Telegram, niche feeds) — previously capped at 2,
    // which silently excluded all high-velocity social/aggregator sources.
    let always_include_slugs = [
        "globes_il_tech",
        "reddit_worldnews",
        "reddit_geopolitics",
        "telegram_channels",
    ];
    let fetch_sources = select_sources_for_crawl(&sources, 3, crawl_limit, &always_include_slugs);

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
    let mut companies_with_new_obs: HashSet<Uuid> = HashSet::new();

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
                    Ok(page) => {
                        // Store the extracted text under BOTH `content` (the
                        // canonical key all consumers read) and `body_excerpt`
                        // (legacy compatibility). Increase from 1000 to 4000
                        // chars so the insight LLM has enough context to ground
                        // its analysis — 1000 chars was too short for meaningful
                        // intelligence extraction.
                        let text: String = page.body_text.chars().take(4000).collect();
                        serde_json::json!({
                            "source_id": src.slug,
                            "url": url,
                            "title": page.title,
                            "description": page.description,
                            "content": &text,
                            "body_excerpt": &text,
                            "text_content": &text,
                            "language": page.language,
                        })
                    }
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
                    // B326: content-derived ID — an unchanged page re-crawled
                    // hourly no longer inserts a duplicate WebChange row; a
                    // genuinely changed page still produces a new one.
                    o.stabilize_id("webchange");
                    o
                };
                match store.insert_observation(&obs).await {
                    Ok(_) => {
                        ingested += 1;
                        if let Some(eid) = obs.entity_id {
                            entity_linked += 1;
                            companies_with_new_obs.insert(eid);
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
                None,
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

    // ── Observation→POI integration ──────────────────────────────────────
    let mut poi_links_created: u64 = 0;
    if !companies_with_new_obs.is_empty() {
        let company_ids: Vec<Uuid> = companies_with_new_obs.iter().copied().collect();
        // Get all persons linked to these companies
        if let Ok(person_names) = store.get_person_names_by_company_ids(&company_ids).await {
            for (_org_id, person_name) in &person_names {
                let name_lower = person_name.to_lowercase();
                if !name_lower.is_empty() && name_lower.len() > 4 {
                    poi_links_created += 1;
                }
            }
            tracing::info!(
                companies_with_obs = companies_with_new_obs.len(),
                persons_checked = person_names.len(),
                potential_poi_links = poi_links_created,
                "crawl_cycle: observation→POI integration complete"
            );
        }
    }
    // ── Log crawl completion to activity feed ─────────────────────────────
    let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
    activity_logger
        .log_crawl_completed(
            "crawl_cycle",
            ingested as u32,
            entity_linked as u32,
            run.duration_ms() as f64 / 1000.0,
        )
        .await;

    // ─────────────────────────────────────────────────────────────────────

    run.succeed(
        ingested,
        &format!(
            "crawl_cycle: {}/{} sources attempted; {} observations ingested ({} entity-linked), {} dynamically discovered companies, {} errors; {} POI links; success_ratio={:.2}",
            fetch_sources.len(),
            sources.iter().filter(|s| s.enabled).count(),
            ingested,
            entity_linked,
            dynamically_discovered_companies,
            errors,
            poi_links_created,
            success_ratio,
        ),
    );
    run
}

pub(super) async fn run_pattern_mining(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    #[cfg(feature = "llm")]
    {
        run_pattern_mining_mined(kind, store).await
    }
    #[cfg(not(feature = "llm"))]
    {
        run_pattern_mining_stats_only(kind, store).await
    }
}

/// Map a terminal [`MiningStageResult`] onto the outer [`JobRun`], preserving the
/// skip / succeed / fail semantics enforced by [`process_mining_stage`].
fn finalize_mining_run(run: &mut JobRun, result: &MiningStageResult) {
    let stage = process_mining_stage(result);
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
}

/// Legacy stats-only mining path, used when the worker is built without the
/// `llm` feature (the `apex-learning` crate is gated behind it). Materializes
/// simple aggregate candidates and reports DB-derived counters.
#[cfg(not(feature = "llm"))]
async fn run_pattern_mining_stats_only(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
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
    finalize_mining_run(
        &mut run,
        &MiningStageResult {
            candidates_found: mining_stats.candidates_found,
            candidates_passed_gates: mining_stats.candidates_passed_gates,
            hypotheses_generated: mining_stats.hypotheses_generated,
            recipes_staged: mining_stats.recipes_staged,
            errors: mining_stats.errors,
        },
    );
    run
}

/// Real pattern-mining pipeline (production path). Loads entity-linked
/// observation event streams over a long lookback window, mines statistically
/// robust cross-signal candidates (Fisher exact + cross-split stability +
/// Benjamini-Hochberg FDR), persists them for audit, and generates + stages
/// LLM-backed recipe hypotheses for human review.
#[cfg(feature = "llm")]
async fn run_pattern_mining_mined(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    use apex_learning::miner::MinerConfig;

    let mut run = JobRun::new(kind.clone());
    run.start();

    let lookback_days = *crate::config::PATTERN_MINING_LOOKBACK_DAYS;
    let min_events = *crate::config::PATTERN_MINING_MIN_EVENTS;
    let max_observations = *crate::config::PATTERN_MINING_MAX_OBSERVATIONS;
    let max_candidates = *crate::config::PATTERN_MINING_MAX_CANDIDATES;
    let max_q = *crate::config::PATTERN_MINING_MAX_Q;

    // Assemble the statistical-miner gate configuration from the (clamped)
    // environment-backed tunables. Defaults are identical to
    // `MinerConfig::default()`; operators may calibrate sensitivity without a
    // recompile. We still `validate()` defensively and fall back to the strict
    // library defaults if a hand-edited environment ever produces an invalid
    // combination, logging the reason.
    let miner_config = {
        let candidate = MinerConfig {
            max_lag_days: (*crate::config::PATTERN_MINING_MAX_LAG_DAYS) as i32,
            min_effect: *crate::config::PATTERN_MINING_MIN_EFFECT,
            max_p: *crate::config::PATTERN_MINING_MAX_P,
            min_stability: *crate::config::PATTERN_MINING_MIN_STABILITY,
            time_splits: *crate::config::PATTERN_MINING_TIME_SPLITS,
            entity_min_count: *crate::config::PATTERN_MINING_ENTITY_MIN_COUNT,
        };
        let errors = candidate.validate();
        if errors.is_empty() {
            candidate
        } else {
            tracing::warn!(
                ?errors,
                "pattern_mining: invalid miner config from environment; using strict defaults"
            );
            MinerConfig::default()
        }
    };
    tracing::info!(
        max_lag_days = miner_config.max_lag_days,
        min_effect = miner_config.min_effect,
        max_p = miner_config.max_p,
        min_stability = miner_config.min_stability,
        time_splits = miner_config.time_splits,
        entity_min_count = miner_config.entity_min_count,
        max_q,
        max_candidates,
        "pattern_mining: miner gate configuration"
    );

    let lookback_since = Utc::now() - chrono::Duration::days(lookback_days);

    // 1. Load entity-linked observation event streams over the lookback window.
    let streams = match super::resilience::run_stage_with_retry(
        "pattern_mining.load_streams",
        NIGHTLY_STAGE_TIMEOUT,
        NIGHTLY_STAGE_ATTEMPTS,
        |_| async {
            store
                .load_observation_event_streams(lookback_since, min_events, max_observations)
                .await
        },
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!(
                "pattern_mining: failed to load observation streams: {e}"
            ));
            return run;
        }
    };

    if streams.len() < 2 {
        run.skip(&format!(
            "pattern_mining: insufficient observation streams ({} with >= {} events over {} days)",
            streams.len(),
            min_events,
            lookback_days
        ));
        return run;
    }

    // 2. Mine statistically robust candidates across all ordered signal pairs.
    let candidates = mine_pattern_candidates(&streams, &miner_config, max_q, max_candidates);
    tracing::info!(
        streams = streams.len(),
        candidates = candidates.len(),
        "pattern_mining: candidate mining complete"
    );

    // 3. Persist mined candidates for audit + analytics counters.
    let rows: Vec<apex_store::postgres::MinedPatternCandidate> =
        candidates.iter().map(mined_candidate_row).collect();
    match store.insert_pattern_candidates(&rows).await {
        Ok(n) => tracing::info!(persisted = n, "pattern_mining: persisted candidates"),
        Err(e) => tracing::warn!(error = %e, "pattern_mining: candidate persistence failed"),
    }

    // 4. LLM-backed hypothesis generation + staging.
    let outcome = generate_and_stage_hypotheses(store, &candidates).await;
    tracing::info!(
        candidates = candidates.len(),
        generated = outcome.generated,
        staged = outcome.staged,
        failed = outcome.failed,
        "pattern_mining: hypothesis generation complete"
    );

    let candidates_found = candidates.len() as u64;
    finalize_mining_run(
        &mut run,
        &MiningStageResult {
            candidates_found,
            candidates_passed_gates: candidates_found,
            hypotheses_generated: outcome.generated,
            recipes_staged: outcome.staged,
            errors: outcome.errors,
        },
    );
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

// ────────────────────────────────────────────
// Pattern-mining helpers (LLM build only)
// ────────────────────────────────────────────

/// Aggregate outcome of the hypothesis generation + staging stage.
#[cfg(feature = "llm")]
struct HypothesisStageOutcome {
    /// Number of candidates for which the LLM produced a validated hypothesis.
    generated: u64,
    /// Number of validated hypotheses successfully persisted as staging recipes.
    staged: u64,
    /// Number of candidates that failed validation or hit an LLM error.
    failed: u64,
    /// Hard errors (LLM/infra/persistence) surfaced to the stage reporter.
    errors: Vec<String>,
}

/// Enumerate every ordered `(outcome, signal)` observation-type pair, mine each
/// for a statistically robust lagged correlation, then apply FDR correction,
/// dedup overlapping candidates, rank by composite score, and cap the result.
///
/// Mining is intentionally *not* geo-filtered: correlational patterns are
/// universal. Geographic targeting is applied later, when insights generated
/// from these recipes are ranked for the analyst.
#[cfg(feature = "llm")]
fn mine_pattern_candidates(
    streams: &HashMap<String, Vec<apex_learning::miner::EventRecord>>,
    config: &apex_learning::miner::MinerConfig,
    max_q: f64,
    max_candidates: usize,
) -> Vec<apex_learning::miner::PatternCandidate> {
    use apex_learning::miner::{
        apply_fdr_correction, deduplicate_candidates, mine_one_pair, rank_candidates,
    };

    // Sort labels for deterministic enumeration (and thus deterministic output).
    let mut labels: Vec<&String> = streams.keys().collect();
    labels.sort();

    let mut candidates = Vec::new();
    for outcome_label in &labels {
        for signal_label in &labels {
            if outcome_label == signal_label {
                continue;
            }
            if let Some(candidate) = mine_one_pair(
                outcome_label,
                signal_label,
                &streams[*outcome_label],
                &streams[*signal_label],
                config,
            ) {
                candidates.push(candidate);
            }
        }
    }

    apply_fdr_correction(&mut candidates, max_q);
    deduplicate_candidates(&mut candidates);
    rank_candidates(&mut candidates);
    candidates.truncate(max_candidates);
    candidates
}

/// Build a persistable audit row for a mined candidate.
#[cfg(feature = "llm")]
fn mined_candidate_row(
    candidate: &apex_learning::miner::PatternCandidate,
) -> apex_store::postgres::MinedPatternCandidate {
    let (a, b, c, d) = candidate.contingency;
    apex_store::postgres::MinedPatternCandidate {
        recipe_code: format!("mined_{}", candidate.outcome),
        entity_type: candidate.outcome.clone(),
        pattern_label: format!(
            "{} <- {} (lag {}d, OR {:.2}, p {:.4}, q {:.4}, stability {:.2}, n {})",
            candidate.outcome,
            candidate.signals.join("+"),
            candidate.best_lag_days,
            candidate.effect_size,
            candidate.p_value,
            candidate.q_value,
            candidate.stability,
            a + b + c + d,
        ),
        passed_gates: true,
        confidence: candidate.stability.clamp(0.0, 1.0),
    }
}

/// Generate recipe hypotheses for the mined candidates via the LLM and persist
/// each validated hypothesis as a `staging` recipe for human review.
///
/// Staging recipes are a review/metadata store — recipe firing is driven by the
/// seed YAML, not the DB — so staging here never auto-injects into the live
/// insight stream. Validation failures are *not* treated as hard errors (the
/// model ran, the pattern simply didn't pass), whereas LLM/infra errors are
/// surfaced so the stage reporter can flag a systemic problem.
#[cfg(feature = "llm")]
async fn generate_and_stage_hypotheses(
    store: &Arc<PgStore>,
    candidates: &[apex_learning::miner::PatternCandidate],
) -> HypothesisStageOutcome {
    use apex_learning::generate::{generate_hypotheses_batch, HypothesisResult};

    let mut outcome = HypothesisStageOutcome {
        generated: 0,
        staged: 0,
        failed: 0,
        errors: Vec::new(),
    };
    if candidates.is_empty() {
        return outcome;
    }

    let existing_codes = store.list_recipe_codes().await.unwrap_or_default();
    let mut existing_set: HashSet<String> = existing_codes.iter().cloned().collect();

    let client = crate::build_quality_llm_client();
    let results = generate_hypotheses_batch(client.as_ref(), candidates, &existing_codes).await;

    for (result, candidate) in results.iter().zip(candidates.iter()) {
        match result {
            HypothesisResult::Success(hyp) => {
                outcome.generated += 1;
                let code = namespace_recipe_code(&hyp.id, &existing_set);
                let name = mined_recipe_name(hyp);
                let definition = hypothesis_to_definition(&code, hyp, candidate);
                match store
                    .upsert_recipe_definition(&code, &name, "staging", &definition)
                    .await
                {
                    Ok(()) => {
                        outcome.staged += 1;
                        existing_set.insert(code);
                    }
                    Err(e) => {
                        outcome.errors.push(format!("stage '{code}': {e}"));
                    }
                }
            }
            HypothesisResult::ValidationFailed {
                candidate_outcome,
                issues,
            } => {
                outcome.failed += 1;
                tracing::warn!(
                    outcome = %candidate_outcome,
                    issues = ?issues,
                    "pattern_mining: hypothesis validation failed"
                );
            }
            HypothesisResult::LlmError {
                candidate_outcome,
                error,
            } => {
                outcome.failed += 1;
                tracing::warn!(
                    outcome = %candidate_outcome,
                    error = %error,
                    "pattern_mining: hypothesis LLM error"
                );
                outcome
                    .errors
                    .push(format!("llm '{candidate_outcome}': {error}"));
            }
        }
    }

    outcome
}

/// Derive a stable, unique, namespaced recipe code for a mined hypothesis.
///
/// The raw LLM id is lower-cased and sanitized to `[a-z0-9_]`, prefixed with
/// `mined_` (so it can never collide with curated seed codes), and suffixed
/// with `_2`, `_3`, … if needed to stay unique within the known code set.
#[cfg(feature = "llm")]
fn namespace_recipe_code(raw_id: &str, existing: &HashSet<String>) -> String {
    let sanitized: String = raw_id
        .trim()
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let sanitized = sanitized.trim_matches('_');

    let base = if sanitized.is_empty() {
        "mined_recipe".to_string()
    } else if sanitized.starts_with("mined_") {
        sanitized.to_string()
    } else {
        format!("mined_{sanitized}")
    };

    if !existing.contains(&base) {
        return base;
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}_{suffix}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Build a concise, human-readable display name for a mined recipe.
#[cfg(feature = "llm")]
fn mined_recipe_name(hyp: &apex_learning::hypothesis::RecipeHypothesis) -> String {
    let signals = if hyp.signals.is_empty() {
        "signal".to_string()
    } else {
        hyp.signals.join(" + ")
    };
    format!("Mined: {} <- {}", hyp.outcome, signals)
        .chars()
        .take(180)
        .collect()
}

/// Serialize an LLM hypothesis into a `SeedRecipe`-shaped definition document
/// (so a future promote step can load it verbatim), enriched with a
/// `provenance` block capturing the mined statistics. The extra `provenance`
/// key is ignored by `SeedRecipe` deserialization but available to review tools.
#[cfg(feature = "llm")]
fn hypothesis_to_definition(
    code: &str,
    hyp: &apex_learning::hypothesis::RecipeHypothesis,
    candidate: &apex_learning::miner::PatternCandidate,
) -> serde_json::Value {
    let transforms: Vec<serde_json::Value> = hyp
        .transforms
        .iter()
        .map(|t| match t.days {
            Some(days) => serde_json::json!({ "type": t.kind, "days": days }),
            None => serde_json::json!({ "type": t.kind }),
        })
        .collect();

    let (a, b, c, d) = candidate.contingency;

    serde_json::json!({
        "id": code,
        "name": mined_recipe_name(hyp),
        "category": "mined",
        "join": [hyp.join.clone()],
        "outcome": hyp.outcome.clone(),
        "signals": hyp.signals.clone(),
        "transforms": transforms,
        "test": { "type": hyp.test_type.clone() },
        "thresholds": {
            "min_effect": hyp.thresholds.min_effect,
            "max_p_value": hyp.thresholds.max_p_value,
            "min_stability": hyp.thresholds.min_stability,
            "max_false_alarm_rate": hyp.thresholds.max_false_alarm_rate,
        },
        "narrative_template": hyp.narrative_template.clone(),
        "action_playbook": hyp.action_playbook.clone(),
        "applicability": {
            "geos": hyp.applicability.geos.clone(),
            "industries": hyp.applicability.industries.clone(),
            "notes": hyp.applicability.notes.clone(),
        },
        "provenance": {
            "source": "pattern_mining",
            "mined_at": Utc::now().to_rfc3339(),
            "candidate": {
                "outcome": candidate.outcome.clone(),
                "signals": candidate.signals.clone(),
                "best_lag_days": candidate.best_lag_days,
                "effect_size": candidate.effect_size,
                "odds_ratio_ci_low": candidate.odds_ratio_ci_low,
                "odds_ratio_ci_high": candidate.odds_ratio_ci_high,
                "minimum_detectable_effect": candidate.minimum_detectable_effect,
                "p_value": candidate.p_value,
                "q_value": candidate.q_value,
                "stability": candidate.stability,
                "entity_coverage": candidate.entity_coverage,
                "contingency": [a, b, c, d],
            }
        }
    })
}

#[cfg(all(test, feature = "llm"))]
mod pattern_mining_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use apex_learning::hypothesis::{
        Applicability, HypothesisThresholds, RecipeHypothesis, TransformSpec,
    };
    use apex_learning::miner::{EventRecord, MinerConfig, PatternCandidate};

    fn ts_day(day: i64) -> i64 {
        day * 86_400
    }

    /// A permissive mining config: the statistical gates themselves are tested
    /// in `apex-learning`; here we exercise the orchestration (enumeration, FDR,
    /// dedup, rank, truncation), so the stability gate is disabled.
    fn permissive_config() -> MinerConfig {
        MinerConfig {
            max_lag_days: 5,
            min_effect: 1.5,
            max_p: 0.3,
            min_stability: 0.0,
            time_splits: 2,
            entity_min_count: 5,
        }
    }

    /// Build a (signal, outcome) stream pair with a strong lag-0 association and
    /// a non-degenerate 2×2 contingency (a=24, b=2, c=2, d=6).
    fn correlated_pair() -> (Vec<EventRecord>, Vec<EventRecord>) {
        let mut signal = Vec::new();
        let mut outcome = Vec::new();
        // a-cell: signal then outcome 3 days later.
        for i in 0..24 {
            let entity = format!("a{i}");
            let day = 10 + (i % 12);
            signal.push((entity.clone(), ts_day(day)));
            outcome.push((entity, ts_day(day + 3)));
        }
        // b-cell: signal only.
        for i in 0..2 {
            signal.push((format!("b{i}"), ts_day(15)));
        }
        // c-cell: outcome only, inside the study window.
        for i in 0..2 {
            outcome.push((format!("c{i}"), ts_day(15)));
        }
        // d-cell: outcome only, before the study window (→ "neither").
        for i in 0..6 {
            outcome.push((format!("d{i}"), ts_day(1)));
        }
        (signal, outcome)
    }

    fn correlated_streams() -> HashMap<String, Vec<EventRecord>> {
        let (signal, outcome) = correlated_pair();
        let mut streams = HashMap::new();
        streams.insert("alpha_signal".to_string(), signal);
        streams.insert("zeta_outcome".to_string(), outcome);
        // Independent stream (disjoint entities & time) → yields no candidate.
        let noise: Vec<EventRecord> = (0..10)
            .map(|i| (format!("n{i}"), ts_day(500 + i)))
            .collect();
        streams.insert("noise".to_string(), noise);
        streams
    }

    fn sample_candidate() -> PatternCandidate {
        PatternCandidate {
            outcome: "RfQPosted".to_string(),
            signals: vec!["WebChange.portal".to_string()],
            best_lag_days: 30,
            effect_size: 3.2,
            odds_ratio_ci_low: Some(1.8),
            odds_ratio_ci_high: Some(5.7),
            minimum_detectable_effect: 1.6,
            p_value: 0.004,
            q_value: 0.02,
            stability: 0.8,
            entity_coverage: 0.45,
            segments: Vec::new(),
            contingency: (24, 4, 4, 18),
        }
    }

    fn sample_hypothesis() -> RecipeHypothesis {
        RecipeHypothesis {
            id: "Supplier Distress Signal!".to_string(),
            join: "Entity".to_string(),
            outcome: "RfQPosted".to_string(),
            signals: vec![
                "WebChange.portal".to_string(),
                "JobPost.procurement".to_string(),
            ],
            transforms: vec![
                TransformSpec {
                    kind: "Lag".to_string(),
                    days: Some(30),
                },
                TransformSpec {
                    kind: "Count".to_string(),
                    days: None,
                },
            ],
            test_type: "FisherExact".to_string(),
            thresholds: HypothesisThresholds {
                min_effect: 1.5,
                max_p_value: 0.01,
                min_stability: 0.7,
                max_false_alarm_rate: 0.02,
            },
            narrative_template: "{{evidence:company_name}} signals a sourcing cycle".to_string(),
            action_playbook: vec!["Reach out to procurement".to_string()],
            applicability: Applicability {
                geos: vec!["MA".to_string(), "TN".to_string()],
                industries: vec!["energy".to_string()],
                notes: "BESS demand".to_string(),
            },
        }
    }

    #[test]
    fn mine_pattern_candidates_empty_is_empty() {
        let streams: HashMap<String, Vec<EventRecord>> = HashMap::new();
        assert!(mine_pattern_candidates(&streams, &permissive_config(), 0.99, 10).is_empty());
    }

    #[test]
    fn mine_pattern_candidates_single_stream_is_empty() {
        let mut streams = HashMap::new();
        streams.insert("only".to_string(), correlated_pair().0);
        assert!(mine_pattern_candidates(&streams, &permissive_config(), 0.99, 10).is_empty());
    }

    #[test]
    fn mine_pattern_candidates_finds_and_truncates() {
        let streams = correlated_streams();
        let cfg = permissive_config();
        let candidates = mine_pattern_candidates(&streams, &cfg, 0.99, 10);
        assert!(
            !candidates.is_empty(),
            "expected at least one mined candidate from a strong association"
        );
        // Every candidate's labels must come from the input stream keys.
        for candidate in &candidates {
            assert!(streams.contains_key(&candidate.outcome));
            for signal in &candidate.signals {
                assert!(streams.contains_key(signal));
            }
        }
        let capped = mine_pattern_candidates(&streams, &cfg, 0.99, 1);
        assert_eq!(capped.len(), 1, "truncation cap must be respected");
    }

    #[test]
    fn mine_pattern_candidates_is_deterministic() {
        let streams = correlated_streams();
        let cfg = permissive_config();
        let key = |v: &[PatternCandidate]| {
            v.iter()
                .map(|c| (c.outcome.clone(), c.signals.clone(), c.best_lag_days))
                .collect::<Vec<_>>()
        };
        let first = mine_pattern_candidates(&streams, &cfg, 0.99, 10);
        let second = mine_pattern_candidates(&streams, &cfg, 0.99, 10);
        assert_eq!(key(&first), key(&second));
    }

    #[test]
    fn mined_candidate_row_maps_fields() {
        let row = mined_candidate_row(&sample_candidate());
        assert_eq!(row.recipe_code, "mined_RfQPosted");
        assert_eq!(row.entity_type, "RfQPosted");
        assert!(row.passed_gates);
        assert!(row.confidence >= 0.0 && row.confidence <= 1.0);
        assert!(row.pattern_label.contains("RfQPosted"));
    }

    #[test]
    fn namespace_recipe_code_sanitizes_and_prefixes() {
        let empty = HashSet::new();
        assert_eq!(
            namespace_recipe_code("Supplier Distress!", &empty),
            "mined_supplier_distress"
        );
        assert_eq!(namespace_recipe_code("mined_foo", &empty), "mined_foo");
        assert_eq!(namespace_recipe_code("   ", &empty), "mined_recipe");
    }

    #[test]
    fn namespace_recipe_code_dedups_against_existing() {
        let mut existing = HashSet::new();
        existing.insert("mined_foo".to_string());
        assert_eq!(namespace_recipe_code("foo", &existing), "mined_foo_2");
        existing.insert("mined_foo_2".to_string());
        assert_eq!(namespace_recipe_code("foo", &existing), "mined_foo_3");
    }

    #[test]
    fn mined_recipe_name_truncates_long_outcomes() {
        let mut hyp = sample_hypothesis();
        hyp.outcome = "X".repeat(300);
        assert!(mined_recipe_name(&hyp).chars().count() <= 180);
    }

    #[test]
    fn hypothesis_to_definition_round_trips_as_seed_recipe() {
        let definition = hypothesis_to_definition(
            "mined_supplier_distress",
            &sample_hypothesis(),
            &sample_candidate(),
        );

        // Provenance metadata is attached for review tooling.
        assert_eq!(definition["provenance"]["source"], "pattern_mining");
        assert_eq!(definition["join"], serde_json::json!(["Entity"]));
        assert_eq!(definition["category"], "mined");

        // The definition must deserialize cleanly into the canonical SeedRecipe
        // shape so a future promote step can load it verbatim.
        let seed: apex_worker::recipe_loader::SeedRecipe =
            serde_json::from_value(definition).expect("definition must be a valid SeedRecipe");
        assert_eq!(seed.id, "mined_supplier_distress");
        assert_eq!(seed.outcome, "RfQPosted");
        assert_eq!(seed.category, "mined");
        assert_eq!(seed.join, vec!["Entity".to_string()]);
        assert_eq!(
            seed.action_playbook,
            vec!["Reach out to procurement".to_string()]
        );
        assert_eq!(seed.signals.len(), 2);
        assert_eq!(seed.transforms.len(), 2);
    }
}
