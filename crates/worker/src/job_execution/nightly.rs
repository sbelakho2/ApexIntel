use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "llm")]
use crate::entity_admission::EntityAdmissionResult;
use crate::intelligence_ingress::NewWarning;
use aho_corasick::{AhoCorasick, MatchKind};
use apex_crawl::browser::{BrowserFetcher, BrowserRequest};
use apex_crawl::client::{CrawlClient, CrawlClientConfig, CrawlRequest};
use apex_crawl::errors::CrawlError;
use apex_crawl::governor_limiter::CrawlGovernor;
use apex_crawl::sources::{
    crawl_source_budget_from_env, dispatch_source_fetch, scheduler_backlog, select_due_sources,
    FetchDispatch, Source, FORCED_SOURCE_SLUGS,
};
#[cfg(feature = "llm")]
use apex_insights::company_discovery::{normalize_company_name, CompanyCandidate, DiscoverySource};
#[cfg(feature = "llm")]
use apex_insights::dynamic_poi_discovery::{
    DiscoveredEntity, DynamicPoiDiscovery, EntityType, PoiCandidate, Recommendation,
};
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::Semaphore;

use super::JobExecutionContext;
use crate::*;

const NIGHTLY_STAGE_TIMEOUT: Duration = Duration::from_secs(30);
const NIGHTLY_STAGE_ATTEMPTS: usize = 3;

/// Maximum concurrent in-flight HTTP fetches for the crawl cycle.
const GLOBAL_HTTP_CONCURRENCY: usize = 12;
/// Maximum concurrent browser-driven fetches (headless Chromium is expensive).
const BROWSER_CONCURRENCY: usize = 2;
/// Per-domain request rate enforced on top of the concurrency caps.
const CRAWL_DOMAIN_RPS: u32 = 1;

const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const CRAWL_USER_AGENT: &str = "ApexIntelBot/1.0 (+https://apex-intel.io/bot)";

/// Result of fetching one scheduled source.
struct SourceFetchOutcome {
    source_index: usize,
    result: Result<FetchedSource, FailedSource>,
}

struct FetchedSource {
    body: String,
    http_status: i32,
    latency_ms: f64,
}

/// How a source fetch failed. `BrowserUnavailable` is a deployment capability
/// gap, not a crawl attempt: it must mark the source unavailable instead of
/// counting as a normal failure or falling back to HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFailureKind {
    Fetch,
    BrowserUnavailable,
}

struct FailedSource {
    message: String,
    http_status: Option<i32>,
    kind: SourceFailureKind,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
async fn fetch_source(
    crawl_client: &CrawlClient,
    browser: Option<&Arc<dyn BrowserFetcher>>,
    governor: &CrawlGovernor,
    browser_permits: &Arc<Semaphore>,
    source: &Source,
    source_index: usize,
) -> SourceFetchOutcome {
    let endpoint = source.rss_url.as_deref().unwrap_or(source.url.as_str());
    let prefers_browser_ua = source.slug == "globes_il_tech";

    // Single dispatch decision: `Browser` sources either render with the
    // shared Chromium renderer or fail as a capability gap — never HTTP.
    let dispatch = match dispatch_source_fetch(source, browser.is_some()) {
        Ok(dispatch) => dispatch,
        Err(error) => {
            return SourceFetchOutcome {
                source_index,
                result: Err(FailedSource {
                    message: error.to_string(),
                    http_status: None,
                    kind: SourceFailureKind::BrowserUnavailable,
                }),
            };
        }
    };

    let needs_browser_slot = dispatch == FetchDispatch::Browser;
    let _browser_permit = if needs_browser_slot || prefers_browser_ua {
        Some(
            browser_permits
                .clone()
                .acquire_owned()
                .await
                .expect("browser concurrency semaphore is never closed"),
        )
    } else {
        None
    };

    if let Some(domain) = source.domain() {
        governor.wait_for_slot(&domain).await;
    }

    if dispatch == FetchDispatch::Browser {
        let Some(browser) = browser else {
            // `dispatch_source_fetch` guarantees this is unreachable: keep a
            // defensive capability failure instead of an HTTP downgrade.
            return SourceFetchOutcome {
                source_index,
                result: Err(FailedSource {
                    message: format!(
                        "source '{}' requires the headless browser renderer, but no browser is available",
                        source.slug
                    ),
                    http_status: None,
                    kind: SourceFailureKind::BrowserUnavailable,
                }),
            };
        };
        let started = std::time::Instant::now();
        return match browser.fetch(BrowserRequest::new(endpoint)).await {
            Ok(page) => SourceFetchOutcome {
                source_index,
                result: Ok(FetchedSource {
                    body: page.html,
                    http_status: 200,
                    latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                }),
            },
            Err(error) => SourceFetchOutcome {
                source_index,
                result: Err(FailedSource {
                    message: error.to_string(),
                    http_status: None,
                    kind: SourceFailureKind::Fetch,
                }),
            },
        };
    }

    let request = CrawlRequest::new(endpoint)
        .source_id(&source.slug)
        .requires_proxy(source.needs_proxy)
        .prefer_browser_user_agent(prefers_browser_ua)
        .override_user_agent(if prefers_browser_ua {
            BROWSER_USER_AGENT
        } else {
            CRAWL_USER_AGENT
        });

    let started = std::time::Instant::now();
    match crawl_client.fetch_text(&request).await {
        Ok(response) => SourceFetchOutcome {
            source_index,
            result: Ok(FetchedSource {
                body: response.body,
                http_status: i32::from(response.status),
                latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            }),
        },
        Err(error) => SourceFetchOutcome {
            source_index,
            result: Err(FailedSource {
                http_status: match &error {
                    CrawlError::HttpStatus { status, .. } => Some(i32::from(*status)),
                    _ => None,
                },
                message: error.to_string(),
                kind: SourceFailureKind::Fetch,
            }),
        },
    }
}

/// Persist the failure state of one source.
///
/// Returns the persistence result instead of swallowing it: a failed write
/// means the scheduler's source runtime state is now wrong (the source may be
/// retried immediately or its backoff ladder is never advanced), so callers
/// must count it and degrade the crawl instead of reporting a clean success.
async fn persist_source_failure(
    store: &PgStore,
    source_slug: &str,
    message: &str,
    http_status: Option<i32>,
    min_interval: chrono::Duration,
    now: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    store
        .record_source_attempt_failure(source_slug, message, http_status, min_interval, now)
        .await
        .map(|_state| ())
        .map_err(|error| {
            tracing::warn!(
                source = %source_slug,
                error = %error,
                "crawl_cycle: failed to persist source failure state"
            );
            error
        })
}

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

/// Source of the entity-name index used for crawl observation linking.
///
/// Abstracted so the crawl-cycle failure path can be exercised with an
/// injected failing store without a live database.
#[async_trait::async_trait]
trait EntityIndexSource {
    async fn entity_name_index(&self) -> anyhow::Result<Vec<(Uuid, String)>>;
}

#[async_trait::async_trait]
impl EntityIndexSource for PgStore {
    async fn entity_name_index(&self) -> anyhow::Result<Vec<(Uuid, String)>> {
        self.list_entity_name_index()
            .await
            .map_err(|error| anyhow::anyhow!("entity name index query failed: {error}"))
    }
}

/// Build a case-insensitive lookup table mapping known entity name tokens to
/// their company UUID.  When a crawled page body contains one of these names
/// the resulting observation is linked to that entity so that downstream jobs
/// (recipe_fire, pattern_mining, etc.) can attribute the data.
///
/// Also constructs an Aho-Corasick automaton for efficient single-pass
/// matching. Lookup or automaton-construction failure is returned as an error:
/// pretending the index is empty would silently disable entity linking for the
/// whole crawl and make every downstream job see zero entity-linked data.
async fn build_entity_name_lookup<S: EntityIndexSource + ?Sized>(
    store: &S,
) -> anyhow::Result<(HashMap<String, Uuid>, EntityMatcher)> {
    let index = store.entity_name_index().await?;
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
        // At a given position the longest entity name wins, so "Acme
        // Batteries" is preferred over a prefix "Acme" (the matcher's
        // documented longest-match contract).
        .match_kind(MatchKind::LeftmostLongest)
        .build(&patterns)
        .map_err(|error| anyhow::anyhow!("failed to build entity name automaton: {error}"))?;

    let matcher = EntityMatcher {
        automaton,
        ids,
        lengths,
    };
    Ok((lookup, matcher))
}

/// Load the entity index for the crawl cycle, degrading `run` instead of
/// proceeding with zero entity names when the lookup fails.
async fn entity_index_or_fail<S: EntityIndexSource + ?Sized>(
    store: &S,
    run: &mut JobRun,
) -> Option<(HashMap<String, Uuid>, EntityMatcher)> {
    match build_entity_name_lookup(store).await {
        Ok(index) => Some(index),
        Err(error) => {
            run.fail(&format!(
                "crawl_cycle degraded: failed to load entity name index \
                 (refusing to crawl without entity linking): {error}"
            ));
            None
        }
    }
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

/// A dynamically discovered candidate staged until the whole crawl cycle has
/// been processed, so [`IndependentSourceDomainsProvider`] can see every
/// source domain that mentioned it.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct StagedDynamicCandidate {
    candidate: PoiCandidate,
    discovered: DiscoveredEntity,
    source_domains: BTreeSet<String>,
}

/// Stage (deduplicate + merge) dynamic discoveries from one source. Nothing
/// is persisted here; candidates are only admitted after the cycle completes.
#[cfg(feature = "llm")]
fn stage_dynamic_discovery_candidates(
    staged: &mut HashMap<String, StagedDynamicCandidate>,
    candidates: &[PoiCandidate],
    discovered: &[DiscoveredEntity],
    source_domain: Option<&str>,
) {
    let discovered_by_name: HashMap<String, &DiscoveredEntity> = discovered
        .iter()
        .map(|entity| (entity.normalized_name.to_ascii_lowercase(), entity))
        .collect();

    for candidate in candidates {
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
        // 0.45 is only the "worth verifying" pre-filter. Admission itself
        // decides via evidence; this threshold can never admit a company.
        if candidate.confidence < 0.45 {
            continue;
        }
        // Relax the co-occurrence gate: allow single-source discoveries if
        // they have meaningful confidence (was: require co_occurrence>0 OR
        // source_diversity>=2).
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
        let discovered_entity = discovered_by_name.get(&dedup_key).copied();
        let staged_candidate = staged
            .entry(dedup_key)
            .or_insert_with(|| StagedDynamicCandidate {
                candidate: PoiCandidate {
                    name: name.clone(),
                    ..candidate.clone()
                },
                discovered: discovered_entity
                    .cloned()
                    .unwrap_or_else(|| DiscoveredEntity {
                        raw_name: name.clone(),
                        normalized_name: normalize_company_name(&name),
                        source_url: String::new(),
                        context: String::new(),
                        confidence: candidate.confidence,
                        entity_type: EntityType::Company,
                        associated_entities: Vec::new(),
                        topics: Vec::new(),
                        geography: Vec::new(),
                    }),
                source_domains: BTreeSet::new(),
            });

        staged_candidate.candidate.confidence = staged_candidate
            .candidate
            .confidence
            .max(candidate.confidence);
        staged_candidate.candidate.emergence_score = staged_candidate
            .candidate
            .emergence_score
            .max(candidate.emergence_score);
        staged_candidate.candidate.co_occurrence_count = staged_candidate
            .candidate
            .co_occurrence_count
            .max(candidate.co_occurrence_count);
        staged_candidate.candidate.source_diversity = staged_candidate
            .candidate
            .source_diversity
            .max(candidate.source_diversity);
        if let Some(entity) = discovered_entity {
            if entity.confidence > staged_candidate.discovered.confidence {
                staged_candidate.discovered = entity.clone();
            }
        }
        if let Some(domain) = source_domain {
            let domain = domain.trim().to_ascii_lowercase();
            if !domain.is_empty() {
                staged_candidate.source_domains.insert(domain);
            }
        }
    }
}

/// Admit staged dynamic discoveries through the canonical
/// `EntityAdmissionService`: evidence-backed verification or analyst review,
/// never a direct company insert.
#[cfg(feature = "llm")]
async fn admit_dynamic_discovery_candidates(
    store: &Arc<PgStore>,
    staged: &HashMap<String, StagedDynamicCandidate>,
    limit: usize,
) -> Result<u64> {
    let mut admission = crate::entity_admission::build_entity_admission_service(
        store.as_ref(),
        "crawl_dynamic_discovery",
        "crawl_discovered",
    );
    let mut ordered: Vec<&StagedDynamicCandidate> = staged.values().collect();
    ordered.sort_by(|a, b| {
        b.candidate
            .confidence
            .partial_cmp(&a.candidate.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut inserted = 0_u64;
    let mut review_required = 0_u64;
    for staged_candidate in ordered {
        if inserted as usize >= limit {
            break;
        }

        let mut metadata: HashMap<String, String> = HashMap::new();
        let discovered = &staged_candidate.discovered;
        metadata.insert("source_url".to_string(), discovered.source_url.clone());
        metadata.insert(
            "context".to_string(),
            crate::truncate_text(&discovered.context, 220).to_string(),
        );
        metadata.insert("topics".to_string(), discovered.topics.join(", "));
        metadata.insert("geography".to_string(), discovered.geography.join(", "));
        metadata.insert(
            "associated_entities".to_string(),
            discovered.associated_entities.join(", "),
        );
        metadata.insert(
            "recommended_action".to_string(),
            recommendation_label(&staged_candidate.candidate.recommended_action).to_string(),
        );
        metadata.insert(
            "emergence_score".to_string(),
            staged_candidate.candidate.emergence_score.to_string(),
        );
        metadata.insert(
            "co_occurrence_count".to_string(),
            staged_candidate.candidate.co_occurrence_count.to_string(),
        );
        metadata.insert(
            "source_diversity".to_string(),
            staged_candidate.candidate.source_diversity.to_string(),
        );
        metadata.insert("is_competitor".to_string(), "false".to_string());
        metadata.insert("discovery_track".to_string(), "crawl_dynamic".to_string());
        if !staged_candidate.source_domains.is_empty() {
            metadata.insert(
                "source_domains".to_string(),
                staged_candidate
                    .source_domains
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }

        let company_candidate = CompanyCandidate {
            raw_name: staged_candidate.candidate.name.clone(),
            normalized_name: normalize_company_name(&staged_candidate.candidate.name),
            source: DiscoverySource::WebCrawl,
            extraction_confidence: staged_candidate.candidate.confidence,
            context_snippet: crate::truncate_text(&discovered.context, 220).to_string(),
            metadata,
        };

        match admission
            .evaluate_company_candidate(&company_candidate)
            .await
        {
            Ok(EntityAdmissionResult::Created(company_id)) => {
                inserted += 1;
                let region = discovered.geography.first().map(String::as_str);
                let activity_logger =
                    apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
                activity_logger
                    .log_company_detected(
                        &company_candidate.raw_name,
                        region,
                        "crawl_dynamic_discovery",
                        Some(&company_id.to_string()),
                    )
                    .await;
                tracing::info!(
                    company = %company_candidate.raw_name,
                    confidence = company_candidate.extraction_confidence,
                    "crawl_cycle: admitted verified dynamically discovered company"
                );
            }
            Ok(EntityAdmissionResult::Existing(_)) => {}
            Ok(EntityAdmissionResult::ReviewRequired(review_id)) => {
                review_required += 1;
                tracing::debug!(
                    company = %company_candidate.raw_name,
                    review_id = %review_id,
                    "crawl_cycle: dynamic discovery candidate queued for analyst review"
                );
            }
            Ok(EntityAdmissionResult::Rejected) => {}
            Err(error) => {
                tracing::warn!(
                    company = %company_candidate.raw_name,
                    error = %error,
                    "crawl_cycle: entity admission failed"
                );
            }
        }
    }

    tracing::info!(
        inserted,
        review_required,
        staged = staged.len(),
        "crawl_cycle: dynamic discovery admission complete"
    );
    Ok(inserted)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) async fn run_crawl_cycle(store: &Arc<PgStore>, ctx: &JobExecutionContext) -> JobRun {
    let ingress = &ctx.ingress;
    let mut run = JobRun::new(JobKind::CrawlCycle);
    run.start();
    let sources = all_sources();
    let crawl_limit: usize = crawl_source_budget_from_env();

    let selection =
        match select_due_sources(store.as_ref(), &sources, crawl_limit, Utc::now()).await {
            Ok(selection) => selection,
            Err(error) => {
                run.fail(&format!(
                    "crawl_cycle: failed to load source runtime state: {error}"
                ));
                return run;
            }
        };
    let sources_due = selection.due;
    let selection_due_remaining = selection.due_sources_remaining;
    let selection_coverage_debt = selection.total_coverage_debt;
    let fetch_sources = selection.selected;

    if fetch_sources.is_empty() {
        run.skip(&format!(
            "crawl_cycle: no due sources (due={sources_due}, declared={}, enabled={})",
            sources.len(),
            sources.iter().filter(|source| source.enabled).count(),
        ));
        return run;
    }

    tracing::info!(
        sources_due,
        selected = fetch_sources.len(),
        limit = crawl_limit,
        forced_sources = %FORCED_SOURCE_SLUGS.join(","),
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
    // A lookup failure degrades the crawl instead of silently pretending the
    // index has zero entities.
    let Some((entity_lookup, entity_matcher)) =
        entity_index_or_fail(store.as_ref(), &mut run).await
    else {
        return run;
    };
    tracing::info!(
        entity_names = entity_lookup.len(),
        "crawl_cycle: entity name index loaded for observation linking"
    );

    #[cfg(feature = "llm")]
    let mut dynamic_discovery =
        DynamicPoiDiscovery::new(entity_lookup.keys().cloned().collect::<Vec<_>>());
    #[cfg(feature = "llm")]
    let mut staged_dynamic_candidates: HashMap<String, StagedDynamicCandidate> = HashMap::new();

    let mut ingested: u64 = 0;
    let mut entity_linked: u64 = 0;
    #[allow(unused_mut)]
    let mut dynamically_discovered_companies: u64 = 0;
    let mut errors: u64 = 0;
    let mut sources_attempted: u64 = 0;
    let mut sources_succeeded: u64 = 0;
    let mut sources_failed: u64 = 0;
    let mut sources_browser_unavailable: u64 = 0;
    let mut successful_sources: HashSet<String> = HashSet::new();
    let mut failed_sources: HashSet<String> = HashSet::new();
    let mut browser_unavailable_sources: HashSet<String> = HashSet::new();
    let mut companies_with_new_obs: HashSet<Uuid> = HashSet::new();
    // Source runtime state is scheduler state. A failed write leaves the
    // scheduler's view of the source stale, so a run with any such failure is
    // degraded, never a clean success.
    let mut scheduler_state_write_failures: u64 = 0;

    let governor = CrawlGovernor::with_limits(CRAWL_DOMAIN_RPS, GLOBAL_HTTP_CONCURRENCY as u32);
    let browser_permits = Arc::new(Semaphore::new(BROWSER_CONCURRENCY));
    let browser = ctx.browser();
    let mut in_flight: FuturesUnordered<BoxFuture<'_, SourceFetchOutcome>> =
        FuturesUnordered::new();
    let mut pending = fetch_sources.iter().enumerate().peekable();
    while in_flight.len() < GLOBAL_HTTP_CONCURRENCY {
        let Some((source_index, source)) = pending.next() else {
            break;
        };
        in_flight.push(Box::pin(fetch_source(
            &crawl_client,
            browser,
            &governor,
            &browser_permits,
            source,
            source_index,
        )));
    }

    while let Some(outcome) = in_flight.next().await {
        if let Some((source_index, source)) = pending.next() {
            in_flight.push(Box::pin(fetch_source(
                &crawl_client,
                browser,
                &governor,
                &browser_permits,
                source,
                source_index,
            )));
        }

        let src = &fetch_sources[outcome.source_index];
        let url = src.rss_url.as_deref().unwrap_or(src.url.as_str());
        let min_interval = chrono::Duration::minutes(i64::from(src.min_interval_minutes));

        match outcome.result {
            Ok(fetched) => {
                sources_attempted += 1;
                let body = fetched.body;
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
                                // Stage only; admission (evidence-backed
                                // verification or analyst review) runs once
                                // the whole cycle has collected every source
                                // domain that mentioned each candidate.
                                stage_dynamic_discovery_candidates(
                                    &mut staged_dynamic_candidates,
                                    &candidates,
                                    &discovered,
                                    src.domain().as_deref(),
                                );
                            }
                        }
                        match store
                            .record_source_success(
                                &src.slug,
                                min_interval,
                                Some(fetched.latency_ms),
                                Some(fetched.http_status),
                                Utc::now(),
                            )
                            .await
                        {
                            Ok(_) => {
                                sources_succeeded += 1;
                                successful_sources.insert(src.slug.clone());
                            }
                            Err(error) => {
                                tracing::warn!(
                                    source = %src.slug,
                                    error = %error,
                                    "crawl_cycle: failed to persist source success state"
                                );
                                sources_failed += 1;
                                failed_sources.insert(src.slug.clone());
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %src.slug,
                            error = %e,
                            "crawl_cycle: failed to store observation"
                        );
                        errors += 1;
                        sources_failed += 1;
                        failed_sources.insert(src.slug.clone());
                        if persist_source_failure(
                            store.as_ref(),
                            &src.slug,
                            &e.to_string(),
                            None,
                            min_interval,
                            Utc::now(),
                        )
                        .await
                        .is_err()
                        {
                            scheduler_state_write_failures += 1;
                        }
                    }
                }
            }
            Err(failure) if failure.kind == SourceFailureKind::BrowserUnavailable => {
                // Deployment capability gap, not a crawl attempt: record the
                // unavailable runtime state, back the scheduler off, and
                // surface a dedicated counter. Never retried over HTTP.
                sources_browser_unavailable += 1;
                browser_unavailable_sources.insert(src.slug.clone());
                tracing::warn!(
                    source = %src.slug,
                    error = %failure.message,
                    "crawl_cycle: source requires the browser renderer, which is unavailable; marking source unavailable"
                );
                let retry_after = if min_interval > chrono::Duration::zero() {
                    min_interval
                } else {
                    chrono::Duration::minutes(30)
                };
                if let Err(error) = store
                    .mark_source_unavailable(&src.slug, &failure.message, retry_after, Utc::now())
                    .await
                {
                    tracing::warn!(
                        source = %src.slug,
                        error = %error,
                        "crawl_cycle: failed to persist unavailable source state"
                    );
                }
            }
            Err(failure) => {
                sources_attempted += 1;
                tracing::warn!(
                    source = %src.slug,
                    http_status = ?failure.http_status,
                    error = %failure.message,
                    "crawl_cycle: fetch error"
                );
                errors += 1;
                sources_failed += 1;
                failed_sources.insert(src.slug.clone());
                if persist_source_failure(
                    store.as_ref(),
                    &src.slug,
                    &failure.message,
                    failure.http_status,
                    min_interval,
                    Utc::now(),
                )
                .await
                .is_err()
                {
                    scheduler_state_write_failures += 1;
                }
            }
        }
    }

    #[cfg(feature = "llm")]
    if !staged_dynamic_candidates.is_empty() {
        let admission_limit = std::env::var("CRAWL_DYNAMIC_DISCOVERY_INSERT_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100)
            .clamp(1, 1000);
        match admit_dynamic_discovery_candidates(store, &staged_dynamic_candidates, admission_limit)
            .await
        {
            Ok(inserted) => dynamically_discovered_companies = inserted,
            Err(error) => tracing::warn!(
                error = %error,
                "crawl_cycle: dynamic discovery admission failed"
            ),
        }
    }

    // Capability-gap sources are excluded from the reliability ratio: they
    // were never attempted, so counting them as failures would misreport the
    // crawl path's health.
    let attempted_sources = sources_attempted.max(1);
    let success_ratio = sources_succeeded as f64 / attempted_sources as f64;
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
    let browser_unavailable_sources_list = {
        let mut v: Vec<_> = browser_unavailable_sources.iter().cloned().collect();
        v.sort();
        v
    };

    let (due_sources_remaining_count, total_coverage_debt) =
        match store.load_source_runtime_states().await {
            Ok(states) => {
                let backlog = scheduler_backlog(&sources, &states, crawl_limit, Utc::now());
                (backlog.due_sources_remaining, backlog.total_coverage_debt)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "crawl_cycle: failed to reload source runtime state for scheduler backlog"
                );
                (selection_due_remaining, selection_coverage_debt)
            }
        };

    tracing::info!(
        sources_due,
        sources_attempted,
        sources_succeeded,
        sources_failed,
        sources_browser_unavailable,
        due_sources_remaining = due_sources_remaining_count,
        total_coverage_debt,
        ingested,
        errors,
        scheduler_state_write_failures,
        "crawl_cycle: scheduler metrics"
    );

    if sources_succeeded == 0 || success_ratio < min_success_ratio {
        let failure_summary = format!(
            "crawl_cycle degraded: due={} attempted={} succeeded={} failed={} browser_unavailable={} ingested={} errors={} due_sources_remaining={} total_coverage_debt={:.2} scheduler_state_write_failures={} success_ratio={:.2} min_success_ratio={:.2} failed_sources={} successful_sources={} browser_unavailable_sources={}",
            sources_due,
            sources_attempted,
            sources_succeeded,
            sources_failed,
            sources_browser_unavailable,
            ingested,
            errors,
            due_sources_remaining_count,
            total_coverage_debt,
            scheduler_state_write_failures,
            success_ratio,
            min_success_ratio,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
            if successful_sources_list.is_empty() { "none".to_string() } else { successful_sources_list.join(",") },
            if browser_unavailable_sources_list.is_empty() { "none".to_string() } else { browser_unavailable_sources_list.join(",") },
        );

        if let Err(error) = ingress
            .submit_warning(
                NewWarning::new("crawl_health", "Crawl reliability degraded", "high")
                    .description(&failure_summary)
                    .confidence((1.0 - success_ratio).clamp(0.0, 1.0))
                    // Crawl health is operational and system-wide.
                    .system_broadcast(),
            )
            .await
        {
            tracing::warn!(%error, "crawl_cycle: failed to record crawl health warning");
        }

        run.fail(&format!(
            "crawl_cycle degraded: due={} attempted={} succeeded={} failed={} browser_unavailable={} ingested={} errors={} due_sources_remaining={} total_coverage_debt={:.2} scheduler_state_write_failures={} failed_sources=[{}] browser_unavailable_sources=[{}]",
            sources_due,
            sources_attempted,
            sources_succeeded,
            sources_failed,
            sources_browser_unavailable,
            ingested,
            errors,
            due_sources_remaining_count,
            total_coverage_debt,
            scheduler_state_write_failures,
            if failed_sources_list.is_empty() { "none".to_string() } else { failed_sources_list.join(",") },
            if browser_unavailable_sources_list.is_empty() { "none".to_string() } else { browser_unavailable_sources_list.join(",") },
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

    if scheduler_state_write_failures > 0 {
        // Ingestion succeeded, but the scheduler's source runtime state could
        // not be fully persisted: report a degraded run, not a clean success.
        run.fail(&format!(
            "crawl_cycle degraded: due={} attempted={} succeeded={} failed={}; {} observations ingested; \
             scheduler_state_write_failures={} (source failure state not persisted)",
            sources_due,
            sources_attempted,
            sources_succeeded,
            sources_failed,
            ingested,
            scheduler_state_write_failures,
        ));
        return run;
    }

    run.succeed(
        ingested,
        &format!(
            "crawl_cycle: due={} attempted={} succeeded={} failed={} browser_unavailable={}; {} observations ingested ({} entity-linked), {} dynamically discovered companies, {} errors; {} POI links; due_sources_remaining={}; total_coverage_debt={:.2}; success_ratio={:.2}",
            sources_due,
            sources_attempted,
            sources_succeeded,
            sources_failed,
            sources_browser_unavailable,
            ingested,
            entity_linked,
            dynamically_discovered_companies,
            errors,
            poi_links_created,
            due_sources_remaining_count,
            total_coverage_debt,
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
        apex_worker::scheduler::JobStatus::Degraded { ref reason, .. } => {
            run.degrade(
                stage.items,
                &format!("mining degraded: {reason} ({})", stage.run.notes),
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
            apex_worker::scheduler::JobStatus::Degraded { ref reason, .. } => {
                run.degrade(
                    stage.items,
                    &format!("hypothesis gen degraded: {reason} ({})", stage.run.notes),
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
        apex_worker::scheduler::JobStatus::Degraded { ref reason, .. } => {
            run.degrade(
                stage.items,
                &format!("drift check degraded: {reason} ({})", stage.run.notes),
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

#[cfg(test)]
mod entity_index_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Injected store whose entity-name index query always fails.
    struct FailingEntityIndex;

    #[async_trait::async_trait]
    impl EntityIndexSource for FailingEntityIndex {
        async fn entity_name_index(&self) -> anyhow::Result<Vec<(Uuid, String)>> {
            anyhow::bail!("simulated entity index outage")
        }
    }

    /// Injected store with a valid, empty entity index.
    struct EmptyEntityIndex;

    #[async_trait::async_trait]
    impl EntityIndexSource for EmptyEntityIndex {
        async fn entity_name_index(&self) -> anyhow::Result<Vec<(Uuid, String)>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn entity_lookup_failure_degrades_crawl_instead_of_using_zero_entities() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();

        let index = entity_index_or_fail(&FailingEntityIndex, &mut run).await;

        assert!(
            index.is_none(),
            "a failed entity lookup must not produce an empty index"
        );
        match &run.status {
            JobStatus::Failed { error, .. } => {
                assert!(
                    error.contains("failed to load entity name index"),
                    "failure note must name the failed stage: {error}"
                );
                assert!(
                    error.contains("simulated entity index outage"),
                    "failure note must carry the underlying error: {error}"
                );
            }
            other => panic!("expected a failed (degraded) crawl run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn entity_lookup_empty_index_is_not_a_failure() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();

        let (lookup, matcher) = entity_index_or_fail(&EmptyEntityIndex, &mut run)
            .await
            .expect("an empty (but successful) index is valid");

        assert!(lookup.is_empty());
        assert!(match_entity_in_text("no known names here", &matcher).is_none());
        assert!(matches!(run.status, JobStatus::Running));
    }

    #[tokio::test]
    async fn entity_lookup_builds_longest_match_matcher() {
        struct FixedEntityIndex;

        #[async_trait::async_trait]
        impl EntityIndexSource for FixedEntityIndex {
            async fn entity_name_index(&self) -> anyhow::Result<Vec<(Uuid, String)>> {
                Ok(vec![
                    (Uuid::nil(), "Acme".to_string()),
                    (Uuid::from_u128(1), "Acme Batteries".to_string()),
                    (Uuid::from_u128(2), "Sh".to_string()),
                ])
            }
        }

        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        let (lookup, matcher) = entity_index_or_fail(&FixedEntityIndex, &mut run)
            .await
            .expect("fixed index loads");

        // Names of length <= 2 are skipped.
        assert_eq!(lookup.len(), 2);
        assert_eq!(
            match_entity_in_text("ACME BATTERIES wins a contract", &matcher),
            Some(Uuid::from_u128(1)),
            "longest entity name must win"
        );
    }
}

#[cfg(test)]
mod browser_dispatch_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use apex_crawl::sources::{Category, Region, SourceCapability};

    fn browser_source(url: &str) -> Source {
        Source {
            slug: "js_only_forum".to_string(),
            name: "JS-only forum".to_string(),
            url: url.to_string(),
            search_param: None,
            region: Region::Global,
            category: Category::Forum,
            tier: 4,
            needs_proxy: false,
            rss_url: None,
            enabled: true,
            min_interval_minutes: 60,
            fetch_strategy: None,
            capability: SourceCapability::Operational,
            notes: None,
        }
    }

    #[tokio::test]
    async fn browser_strategy_without_capability_fails_instead_of_http_fallback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind endpoint the HTTP fallback would hit");
        let addr = listener.local_addr().expect("test server addr");
        let source = browser_source(&format!("http://{addr}/js-only"));

        let client = CrawlClient::new(CrawlClientConfig::default())
            .expect("crawl client builds without network access");
        let governor = CrawlGovernor::with_limits(CRAWL_DOMAIN_RPS, GLOBAL_HTTP_CONCURRENCY as u32);
        let permits = Arc::new(Semaphore::new(BROWSER_CONCURRENCY));

        let outcome = fetch_source(&client, None, &governor, &permits, &source, 0).await;
        match outcome.result {
            Err(failure) => {
                assert_eq!(failure.kind, SourceFailureKind::BrowserUnavailable);
                assert!(
                    failure.message.contains("browser"),
                    "capability gap must name the missing renderer: {}",
                    failure.message
                );
                assert_eq!(failure.http_status, None);
            }
            Ok(_) => panic!("Browser-strategy source must not succeed without a renderer"),
        }

        // The endpoint must never be contacted: the only permitted outcomes
        // are a real render or an explicit capability failure.
        let accepted =
            tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept()).await;
        assert!(
            accepted.is_err(),
            "Browser-strategy source silently fell back to plain HTTP"
        );
    }
}
