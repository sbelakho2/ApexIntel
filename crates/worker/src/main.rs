mod job_execution;
mod runtime;

#[cfg(feature = "llm")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "llm")]
use apex_core::company_names::normalize_company_name;
use apex_core::config::AppConfig;
#[cfg(feature = "llm")]
use apex_core::entities::{ArtifactType, PoiArtifact};
#[cfg(feature = "llm")]
use apex_core::entities::{Company, CompanyType, Person, PriorityVector};
use apex_core::entities::{Observation, ObservationType};
use apex_core::env::parse_truthy_flag;
use apex_core::schemas::{Recipe, RecipeStatus, SignalSpec};
use apex_core::similarity::jaccard_similarity;
pub(crate) use apex_core::text::truncate_utf8 as truncate_text;
use apex_crawl::breach::BreachMonitor;
#[cfg(feature = "llm")]
use apex_crawl::person_scraper::{PersonOsintScraper, RawPersonArtifact};
#[cfg(feature = "llm")]
use apex_crawl::poi_expansion::{DiscoveredPoi, PoiExpansionEngine, SeedPoi};
use apex_crawl::proxy::ProxyRotator;
use apex_crawl::sanctions::SanctionsList;
use apex_crawl::sanctions::SanctionsScreener;
use apex_crawl::source_scoring::{score_and_rank, ScoringConfig, SourceTelemetry};
use apex_crawl::sources::all_sources;
#[cfg(feature = "llm")]
use apex_crawl::tor_client::{DarkWebPersonIntel, TorClient};
#[cfg(feature = "llm")]
use apex_insights::renderer::{Citation, InsightCard};
#[cfg(feature = "llm")]
use apex_insights::weekly_pipeline::{WeeklyPipelineConfig, WeeklyPipelineRunner};
#[cfg(feature = "llm")]
use apex_learning::cross_domain::{mine_signal_combinations, CrossDomainConfig, TypedEvent};
#[cfg(feature = "llm")]
use apex_learning::feedback::{
    rank_observation_types, score_sources, ObsTypeStats, SourceScoringWeights, SourceYield,
};
#[cfg(feature = "llm")]
use apex_llm::evaluation::{standard_eval_suite, EvalRunner};
#[cfg(feature = "llm")]
use apex_llm::inference::LlmClient as InferenceLlmClient;
#[cfg(feature = "llm")]
use apex_llm::self_improvement::{
    ImprovementCycleReport, OutputCapture, SelfImprovementConfig, SelfImprovementLoop, TaskCategory,
};
#[cfg(feature = "llm")]
use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
#[cfg(any(feature = "parse", feature = "llm"))]
use apex_parse::html::extract_page;
#[cfg(feature = "llm")]
use apex_poi::model::{
    InfluenceProfile, PoiProfile, PriorityVector as PoiPriorityVector, PsychProfile, RoleFamily,
};
#[cfg(feature = "llm")]
use apex_poi::updater::refresh_profile;
use apex_recipes::engine::{FeatureMap, RecipeEngine};
use apex_store::postgres::{InsightListFilters, PgStore};
#[cfg(feature = "llm")]
use apex_store::postgres::{PersonListFilters, PersonOrderBy, WarningListFilters};
#[cfg(not(feature = "llm"))]
use apex_worker::nightly::process_poi_stage;
use apex_worker::nightly::{
    process_drift_stage, process_mining_stage, CrawlStageResult, DriftCheckStageResult,
    MiningStageResult, PoiRefreshStageResult,
};
#[cfg(feature = "llm")]
use apex_worker::nightly::{process_hypothesis_generation_stage, HypothesisGenerationStageResult};
use apex_worker::notifications::{NotificationDispatcher, SlaEnforcer, SlaWarningRecord};
use apex_worker::recipe_loader::{
    insert_seed_recipes, load_default_seed_recipes, print_recipe_stats, validate_seed_recipes,
};
use apex_worker::scheduler::{
    default_scheduler, validate_custom_command, JobKind, JobRun, JobStatus, Scheduler,
};
use apex_worker::storage::{
    build_memo_inputs, load_production_recipes, load_staged_recipes, StorageContext,
};
use apex_worker::weekly::{
    run_weekly_pipeline, DeprecationPolicy, MemoInputs, ProductionRecipe, PromotionPolicy,
    StagedRecipe,
};
use chrono::{Datelike, Timelike, Utc};
use chrono_tz::Europe::Berlin;
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "llm")]
use std::sync::Mutex;
use tokio::sync::{Mutex as TokioMutex, Semaphore};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// File size guard (B293)
// ────────────────────────────────────────────────────────────────────────────

/// Maximum file size for input JSON files (B293).
///
/// If `load_nightly_inputs()` or `load_weekly_inputs()` encounters a file
/// larger than this limit, it aborts immediately **before** allocating any
/// heap memory to read the file.  This prevents a misconfigured or maliciously
/// crafted input file from causing the worker to exhaust available RAM.
///
/// Rationale for 16 MiB:
/// - Production nightly input JSON files are typically 10–50 KiB (compressed stage results).
/// - Weekly input JSON files are typically 100–500 KiB (recipe + memo metadata).
/// - 16 MiB is a 50× safety margin; exceeding it is almost certainly a bug (e.g.,
///   accidentally pointed at a database dump, a raw model checkpoint, a binary blob).
///
/// If legitimate production workloads require larger inputs, increase this value
/// after verifying the memory impact on the worker container.
pub const MAX_INPUT_FILE_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct NightlyInputs {
    crawl: CrawlStageResult,
    mining: MiningStageResult,
    poi: PoiRefreshStageResult,
    drift: DriftCheckStageResult,
}

#[derive(Debug, Deserialize)]
struct WeeklyInputs {
    staged_recipes: Vec<StagedRecipe>,
    production_recipes: Vec<ProductionRecipe>,
    memo_inputs: MemoInputs,
    promotion_policy: Option<PromotionPolicy>,
    deprecation_policy: Option<DeprecationPolicy>,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| parse_truthy_flag(&v))
        .unwrap_or(false)
}

fn build_paid_proxy_url_from_env() -> Option<String> {
    let host = std::env::var("PROXY_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let port = std::env::var("PROXY_PORT")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let username = std::env::var("PROXY_USERNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("PROXY_USER")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    let password = std::env::var("PROXY_PASSWORD")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("PROXY_PASS")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    Some(format!("http://{username}:{password}@{host}:{port}"))
}

fn build_proxy_rotator_from_env() -> Option<ProxyRotator> {
    if !env_flag("ENABLE_PROXY_ROTATION") {
        return None;
    }

    let paid_proxy = build_paid_proxy_url_from_env();
    if paid_proxy.is_none() {
        tracing::warn!(
            "proxy rotation enabled, but PROXY_HOST/PROXY_PORT and PROXY_USERNAME|PROXY_USER / PROXY_PASSWORD|PROXY_PASS are incomplete"
        );
    }

    let mut rotator = ProxyRotator::new(true, paid_proxy);

    if let Ok(proxy_list) = std::env::var("PROXY_LIST") {
        let parsed = ProxyRotator::parse_proxy_list(&proxy_list);
        if !parsed.is_empty() {
            rotator.add_proxies(parsed);
        }
    }

    Some(rotator)
}

// ────────────────────────────────────────────────────────────────────────────
// Recipe-fire helpers
// ────────────────────────────────────────────────────────────────────────────

/// Resolve `{{evidence:KEY}}` placeholders in a template string.
/// Replaces each `{{evidence:key}}` with the corresponding value from `slots`,
/// or with a contextual fallback (e.g. "(entity)" for company_name, "N/A" for others).
fn resolve_evidence_placeholders(template: &str, slots: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{evidence:") {
        let prefix = &rest[..start];
        let after = &rest[start + 11..]; // skip "{{evidence:"
        if let Some(end) = after.find("}}") {
            let key = &after[..end];
            if let Some(val) = slots.get(key) {
                result.push_str(prefix);
                result.push_str(val);
            } else {
                // The slot is missing.  Instead of leaving dangling prepositions
                // like "notice for." or "opportunity:", strip the preceding
                // preposition / colon from the already-accumulated text so the
                // sentence reads naturally.
                let trimmed = prefix.trim_end();
                let lower = trimmed.to_ascii_lowercase();
                let strip_prep = lower.ends_with(" for")
                    || lower.ends_with(" from")
                    || lower.ends_with(" at")
                    || lower.ends_with(" in")
                    || lower.ends_with(" via")
                    || lower.ends_with(" of")
                    || lower.ends_with(" to")
                    || lower.ends_with(" by")
                    || lower.ends_with(" with")
                    || lower.ends_with(" toward")
                    || lower.ends_with(" towards")
                    || lower.ends_with(" against")
                    || lower.ends_with(" between")
                    || lower.ends_with(" up")
                    || lower.ends_with(" down")
                    || lower.ends_with(" about")
                    || lower.ends_with(" over")
                    || trimmed.ends_with(':');
                if strip_prep {
                    // Walk back to remove the dangling preposition/colon
                    let cut = if trimmed.ends_with(':') {
                        trimmed.rfind(':').unwrap_or(trimmed.len())
                    } else {
                        trimmed.rfind(' ').unwrap_or(0)
                    };
                    result.push_str(&trimmed[..cut]);
                } else {
                    result.push_str(prefix);
                }
            }
            rest = &after[end + 2..]; // skip "}}"
                                      // If the slot was missing and the next char is '%', skip it too
                                      // (handles patterns like "up {{evidence:pct}}%")
            if slots.get(key).is_none() && rest.starts_with('%') {
                rest = &rest[1..];
            }
        } else {
            // No closing "}}" — emit remainder as-is.
            result.push_str(prefix);
            result.push_str(&rest[start..]);
            rest = "";
        }
    }
    result.push_str(rest);
    result
}

/// Post-process rendered template text: collapse repeated whitespace, strip orphan
/// punctuation artefacts that appear when placeholders are silently omitted, and trim.
fn clean_rendered_text(s: &str) -> String {
    let mut t = s.to_string();
    // Remove empty parentheses / brackets that result from omitted placeholders
    for pat in &["()", "( )", "[]", "[ ]"] {
        t = t.replace(pat, "");
    }
    // Collapse multiple spaces into one
    while t.contains("  ") {
        t = t.replace("  ", " ");
    }
    // Remove orphan punctuation sequences
    for _ in 0..3 {
        t = t.replace(", .", ".").replace(",,", ",").replace(", ,", ",");
        t = t.replace(". .", ".").replace("..", ".");
        t = t.replace(" .", ".").replace(" ,", ",");
        t = t.replace(":.", ".").replace(": .", ".").replace(":,", ",");
        t = t.replace("( ", "(").replace(" )", ")");
        // Sentences that became just "." after stripping
        t = t.replace(". . ", ". ");
    }
    // Remove remaining double spaces from all the substitutions
    while t.contains("  ") {
        t = t.replace("  ", " ");
    }
    // Remove sentences that are just a period
    while t.starts_with(". ") {
        t = t[2..].to_string();
    }
    if t == "." {
        t.clear();
    }
    t.trim().to_string()
}

/// Capitalize the first character of a string (for warning_type → ObsType mapping).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Check if rendered narrative text is too low quality to use.
/// Returns true when the text is mostly placeholder artifacts.
fn is_low_quality_narrative(text: &str) -> bool {
    let s = text.trim();
    if s.len() < 20 {
        return true;
    }
    let lower = s.to_ascii_lowercase();

    // Dashes check (placeholder junk)
    let dash_count = s.matches('—').count();
    let word_count = s.split_whitespace().count();
    if word_count > 0 && dash_count as f64 / word_count as f64 > 0.25 {
        return true;
    }

    // Sentences that are just field names strung together
    let short_tokens: usize = s
        .split(". ")
        .filter(|seg| seg.split_whitespace().count() <= 2)
        .count();
    let total_sentences = s.split(". ").count();
    if total_sentences >= 3 && short_tokens as f64 / total_sentences as f64 > 0.5 {
        return true;
    }

    // Detect double prepositions indicating missing slots between them
    // e.g., "supply from under threat" (missing resource_type between "from" and "under")
    let double_prep_patterns = [
        "from under",
        "from via",
        "from through",
        "from by",
        "via via",
        "via by",
        "via from",
        "via through",
        "by by",
        "by from",
        "by via",
        "in in",
        "to to",
        "at at",
        "for for",
        "of of",
        "with with",
        "between between",
        "supply from under", // specific pattern from bad template
        "showing conflict",
        "showing including", // unfilled {{evidence:signal_count}}
    ];
    for pat in &double_prep_patterns {
        if lower.contains(pat) {
            return true;
        }
    }

    // Vague claims without specifics - all-caps alarmist labels with no data
    // These templates fire but provide no actual evidence
    let vague_alarm_patterns = [
        ("resource weaponization", "which resource"), // must specify what resource
        ("conflict escalation", "conflict indicators"),
        ("sanctions cascade", "sanction"),
        ("technology theft", "technology"),
    ];
    for (alarm, must_have) in &vague_alarm_patterns {
        if lower.contains(alarm) {
            // Check if the narrative actually has the required specifics
            // (entity names, quantities, or the must_have detail)
            let has_number = s.chars().any(|c| c.is_ascii_digit());
            let has_specific = lower.contains(must_have) || has_number;
            if !has_specific {
                return true;
            }
        }
    }

    // Check for grammatically broken phrases from missing slots
    let broken_grammar = [
        ". supply from", // sentence starting without subject
        ". including.",  // empty list
        ": .",           // empty clause after colon
        " via .",        // trailing preposition before period
        " from .",
        " at .",
        " by .",
        " to .",
        " in .",
        " of .",
        " with .",
    ];
    for pat in &broken_grammar {
        if lower.contains(pat) {
            return true;
        }
    }

    false
}

#[derive(Debug, Default, Clone, Copy)]
struct StrategySignalFlags {
    procurement: bool,
    qualification: bool,
    shortage: bool,
    regulatory: bool,
    innovation: bool,
    expansion: bool,
    pricing: bool,
    competitor: bool,
    personnel: bool,
    security: bool,
    geopolitical: bool,
}

fn detect_strategy_signal_flags(
    category: &str,
    signal_details: &[String],
    action_hint: &str,
) -> StrategySignalFlags {
    let corpus =
        format!("{} {} {}", category, signal_details.join(" "), action_hint).to_ascii_lowercase();

    StrategySignalFlags {
        procurement: corpus.contains("procurement")
            || corpus.contains("supplier portal")
            || corpus.contains("tender")
            || corpus.contains("rfq")
            || corpus.contains("sourcing")
            || corpus.contains("sqe"),
        qualification: corpus.contains("qualification")
            || corpus.contains("cert")
            || corpus.contains("audit")
            || corpus.contains("ppap")
            || corpus.contains("imds")
            || corpus.contains("iatf")
            || corpus.contains("as9100")
            || corpus.contains("iso "),
        shortage: corpus.contains("shortage")
            || corpus.contains("allocation")
            || corpus.contains("lead time")
            || corpus.contains("delay")
            || corpus.contains("disruption")
            || corpus.contains("supply chain")
            || corpus.contains("force majeure"),
        regulatory: corpus.contains("regulation")
            || corpus.contains("policy")
            || corpus.contains("tariff")
            || corpus.contains("sanction")
            || corpus.contains("export control")
            || corpus.contains("embargo")
            || corpus.contains("compliance"),
        innovation: corpus.contains("patent")
            || corpus.contains("innovation")
            || corpus.contains("r&d")
            || corpus.contains("npi")
            || corpus.contains("prototype")
            || corpus.contains("engineering")
            || corpus.contains("technology"),
        expansion: corpus.contains("expansion")
            || corpus.contains("capacity")
            || corpus.contains("facility")
            || corpus.contains("new plant")
            || corpus.contains("new site")
            || corpus.contains("trade show"),
        pricing: corpus.contains("pricing")
            || corpus.contains("price")
            || corpus.contains("margin")
            || corpus.contains("landed cost")
            || corpus.contains("cost"),
        competitor: corpus.contains("competitor")
            || corpus.contains("market")
            || corpus.contains("displaced")
            || corpus.contains("replacement opportunity")
            || corpus.contains("customer overlap"),
        personnel: corpus.contains("poi")
            || corpus.contains("leadership")
            || corpus.contains("executive")
            || corpus.contains("project")
            || corpus.contains("hiring")
            || corpus.contains("job posting")
            || corpus.contains("appointment"),
        security: corpus.contains("security")
            || corpus.contains("cyber")
            || corpus.contains("breach")
            || corpus.contains("incident")
            || corpus.contains("cmmc")
            || corpus.contains("iso 27001"),
        geopolitical: corpus.contains("geopolitical")
            || corpus.contains("trade lane")
            || corpus.contains("nearshor")
            || corpus.contains("morocco")
            || corpus.contains("tunisia")
            || corpus.contains("country")
            || corpus.contains("routing"),
    }
}

fn push_unique_suggestion(suggestions: &mut Vec<String>, suggestion: String) {
    let normalized = suggestion.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return;
    }
    if suggestions
        .iter()
        .any(|existing| existing.trim().to_ascii_lowercase() == normalized)
    {
        return;
    }
    suggestions.push(suggestion);
}

fn is_public_sector_entity(entity_label: &str, entity_type: Option<&str>) -> bool {
    let haystack = format!(
        "{} {}",
        entity_type.unwrap_or_default().to_ascii_lowercase(),
        entity_label.to_ascii_lowercase()
    );

    [
        "government",
        "public sector",
        "public-sector",
        "ministry",
        "commission",
        "parliament",
        "council",
        "agency",
        "authority",
        "department",
        "municipality",
        "state ",
        "embassy",
        "consulate",
        "regulator",
    ]
    .iter()
    .any(|marker| haystack.contains(marker))
}

#[cfg(feature = "llm")]
fn public_sector_procurement_or_program_case(evidence_signals: &[EvidenceSignal]) -> bool {
    let corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    let has_procurement_marker = [
        "procurement",
        "tender",
        "rfq",
        "rfp",
        "solicitation",
        "bid",
        "framework agreement",
        "supplier registration",
        "buyer",
        "sourcing",
        "purchase",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_hardware_or_program_marker = [
        "equipment",
        "hardware",
        "device",
        "devices",
        "electronics",
        "camera",
        "cameras",
        "sensor",
        "sensors",
        "streaming",
        "broadcast",
        "control room",
        "media system",
        "manufacturing",
        "assembly",
        "pcba",
        "pcb",
        "box build",
        "program",
        "platform",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    has_procurement_marker && has_hardware_or_program_marker
}

#[cfg(feature = "llm")]
fn contains_public_sector_artifact_marker(text: &str) -> bool {
    [
        "procurement",
        "tender",
        "consultation",
        "notice",
        "official statement",
        "press release",
        "oversight",
        "approval",
        "directive",
        "regulation",
        "tariff",
        "sanction",
        "export control",
        "program",
        "reserve",
        "framework",
        "ministry",
        "commission",
        "agency",
        "department",
        "portal",
        "licensing",
        "permit",
        "review",
        "hearing",
        "decree",
        "guidance",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn is_generic_action_hint(fragment: &str) -> bool {
    let trimmed = fragment.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    let normalized = lower.replace('-', " ");
    let generic_starters = [
        "monitor",
        "identify",
        "assess",
        "track",
        "review",
        "evaluate",
        "analyze",
        "analyse",
        "watch",
        "map",
        "brief",
        "gather",
        "update",
        "compare",
        "convene",
        "coordinate",
        "prepare",
        "scenario",
        "escalate",
        "align",
        "document",
        "prioritize",
        "initiate",
        "alert",
        "verify",
        "adjust",
        "reprioritize",
    ];
    let starts_generic = generic_starters
        .iter()
        .any(|verb| normalized.starts_with(&format!("{} ", verb)));
    if !starts_generic {
        return false;
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let has_domain_like_marker = words.iter().any(|word| {
        let candidate = word
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '-')
            .to_ascii_lowercase();
        candidate.contains('.')
            && candidate.split('.').count() >= 2
            && candidate
                .rsplit('.')
                .next()
                .map(|suffix| {
                    suffix.len() >= 2 && suffix.chars().all(|ch| ch.is_ascii_alphabetic())
                })
                .unwrap_or(false)
    });

    let has_specific_marker = trimmed.chars().any(|ch| ch.is_ascii_digit())
        || trimmed.contains('%')
        || trimmed.contains("http://")
        || trimmed.contains("https://")
        || lower.contains("cve-")
        || has_domain_like_marker
        || words.iter().skip(1).any(|word| {
            word.chars()
                .next()
                .map(|ch| ch.is_ascii_uppercase())
                .unwrap_or(false)
                && word.len() > 2
        });

    if has_specific_marker {
        return false;
    }

    let generic_business_markers = [
        "risk assessment",
        "cross functional",
        "cross-functional",
        "operational disruption",
        "scenario plan",
        "scenario-plan",
        "contingency planning",
        "leadership review",
        "stakeholder alignment",
        "customer impact",
        "topic drivers",
        "monitoring cadence",
        "suspicious domains",
        "takedown procedures",
        "customer facing teams",
        "customer-facing teams",
        "authentication scrutiny",
        "vendor security assessment",
        "data handling practices",
        "incident response procedures",
        "network segmentation",
        "security frameworks",
        "official domains",
        "procurement portals",
        "citizen facing services",
        "citizen-facing services",
    ];

    words.len() <= 8
        || generic_business_markers
            .iter()
            .any(|marker| normalized.contains(marker))
}

fn concrete_signal_details(signal_details: &[String]) -> Vec<String> {
    signal_details
        .iter()
        .filter(|detail| !is_aggregate_metric_signal_detail(detail))
        .map(|detail| detail.trim())
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.trim_end_matches('.').to_string())
        .collect()
}

fn extract_fallback_signal_details(summary: &str) -> Vec<String> {
    let lower_summary = summary.to_ascii_lowercase();
    let Some(start) = lower_summary.find("our monitoring detected:") else {
        return Vec::new();
    };

    let detected = &summary[start + "Our monitoring detected:".len()..];
    let first_sentence = detected.split('\n').next().unwrap_or(detected);
    let first_sentence = first_sentence.split(". ").next().unwrap_or(first_sentence);

    first_sentence
        .split(';')
        .map(str::trim)
        .map(|detail| detail.trim_end_matches('.').trim())
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.to_string())
        .collect()
}

fn is_aggregate_metric_signal_detail(detail: &str) -> bool {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let aggregate_suffixes = [
        "job posting(s) observed",
        "patent(s) filed",
        "tender(s) identified",
        "certification(s) on record",
        "competitor signal(s)",
        "news article(s) referenced",
        "web change(s) detected",
        "regulatory filing(s)",
        "sanction entry(ies)",
        "trade show participation(s)",
        "person(s) of interest tracked",
    ];

    trimmed
        .chars()
        .next()
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(false)
        && aggregate_suffixes
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

fn count_concrete_signal_details(signal_details: &[String]) -> usize {
    concrete_signal_details(signal_details).len()
}

fn concrete_action_fragments(action_hint: &str) -> Vec<String> {
    action_hint
        .split([';', '.'])
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .filter(|fragment| !is_generic_action_hint(fragment))
        .map(|fragment| fragment.trim_end_matches('.').to_string())
        .take(2)
        .collect()
}

fn should_emit_fallback_insight(
    signal_details: &[String],
    rendered_action: &str,
    confidence: f64,
    evidence_count: usize,
) -> bool {
    let concrete_signals = count_concrete_signal_details(signal_details);
    let concrete_actions = concrete_action_fragments(rendered_action);

    // Aggregate counters alone do not justify a user-facing fallback insight.
    if concrete_signals == 0 {
        return false;
    }

    if concrete_actions.is_empty() && confidence < 0.60 && evidence_count < 3 {
        return false;
    }

    true
}

fn build_goal_oriented_suggestions(
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    signal_details: &[String],
    action_hint: Option<&str>,
) -> Vec<String> {
    let flags =
        detect_strategy_signal_flags(category, signal_details, action_hint.unwrap_or_default());
    let is_public_sector = is_public_sector_entity(entity_label, entity_type);
    let entity = if entity_label.is_empty() {
        "this account"
    } else {
        entity_label
    };
    let region_note = if entity_region.is_empty() {
        String::new()
    } else {
        format!(" in {}", entity_region)
    };

    let mut suggestions = Vec::new();

    match (is_public_sector, category) {
        (true, "brand_sentiment") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Treat sentiment around {} as actionable only if it starts to change tender scrutiny, policy credibility, oversight pressure, or approval timing{}.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Separate a short media cycle from a real institutional issue by checking whether the discussion around {} is leading to formal review, parliamentary attention, procurement caution, or stakeholder messaging changes.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Brief account teams only on bids, regulated customers, or public-sector programs that depend on decisions from {} and could slow down if scrutiny hardens.",
                entity
            ));
        }
        (true, "regulatory_policy") | (true, "geopolitical_analysis") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Treat the signal around {}{} as material only when it resolves into a concrete policy artifact such as an official statement, tender amendment, export-control move, or oversight action.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Escalate only when the evidence names the affected institution, program, border measure, or approval flow tied to {} rather than translating broad geopolitical noise into an operations playbook.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Use the next review to confirm what specific rule, corridor, or procurement process could move next at {} before changing routing, qualification, or supply assumptions.",
                entity
            ));
        }
        (true, "security_compliance")
        | (true, "cybersecurity_threat")
        | (true, "quality_compliance") => {
            push_unique_suggestion(&mut suggestions, format!(
                "Check whether the signal around {} reaches official domains, citizen-facing services, procurement portals, or supplier access paths before escalating it as a government-wide incident.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, "If a concrete domain, host, CVE, or ministry system is involved, route that exact artifact into takedown, remediation, or access-control work rather than relying on a generic cyber playbook.".to_string());
            push_unique_suggestion(&mut suggestions, format!(
                "Brief stakeholders only on the specific public services, agencies, or vendor workflows that would be affected if the security signal around {} is confirmed.",
                entity
            ));
        }
        (true, _) => {
            push_unique_suggestion(&mut suggestions, format!(
                "Use this signal around {} to test whether procurement scrutiny, approval timing, stakeholder access, or policy posture is actually changing rather than assuming it is a direct sales trigger.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Map which bids, regulated customers, or public programs are exposed to decisions from {}{} and prioritize only those with real timing or compliance consequences.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "Brief teams on what concrete institutional decision could move next at {}, who owns that decision, and what evidence would justify escalation beyond background monitoring.",
                entity
            ));
        }
        (false, "demand_procurement") | (false, "customer_rfq") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is revenue capture, use the current signal window to get in front of {} with a capability-led offer{} before the sourcing shortlist hardens.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is qualification readiness, line up the audit pack, certification evidence, and sector-specific onboarding material that {} is likely to request.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is footprint positioning, test whether an EU or North Africa manufacturing angle gives {} a better resilience, tariff, or lead-time story than incumbent supply.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is early design influence, use prototype, NPI, or engineering-support language rather than waiting for a fully formal RFQ cycle at {}.",
                entity
            ));
        }
        (false, "supply_chain_risk") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is continuity protection, map the single-source and long-lead exposures around {} first and decide where dual-source qualification or inventory buffers matter most.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is customer assurance, turn the signal into a concrete fallback story: what can be rerouted, requalified, or migrated before schedules slip for programs touching {}{}?",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is commercial upside, use the disruption around {} to open conversations where incumbents cannot currently promise supply continuity or timeline confidence.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the issue is structural rather than temporary, consider design migration, alternative technology qualification, and contractual reset paths rather than only short-term firefighting around {}.",
                entity
            ));
        }
        (false, "competitor_market") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is competitive displacement, identify which accounts near {} are most exposed to missed qualifications, slower ramps, or weaker service and build a targeted rescue narrative around that pain.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is account defense, review where {} overlaps with your highest-value customers and prepare a sharper proof set on responsiveness, certification depth, and execution reliability.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is pricing leverage, decide whether {} is signaling margin pressure, aggressive expansion, or a tactical quote reset, then match the response to total-cost value rather than list price alone.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is regional strategy, look for places where {} is thin on local footprint, sector credibility, or customer intimacy and press that angle directly.",
                entity
            ));
        }
        (false, "regulatory_policy") | (false, "geopolitical_analysis") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is regulatory posture, turn this into a concrete checklist for export control, certification scope, contract language, and customer communication affecting {}{}.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is routing and footprint, model whether Morocco, Tunisia, or EU-qualified alternatives now solve a policy or trade problem more cleanly than the current setup around {}.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is executive planning, brief leadership on which programs around {} could need re-sourcing, repricing, or customer reassurance first rather than treating this as a generic macro event.",
                entity
            ));
        }
        (false, "strategic_poi") | (false, "talent_ip") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is relationship leverage, update the power map around {} and decide which individual or team now controls project timing, supplier qualification, or partnership appetite.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is early project entry, use the current personnel or project signal to anchor a discussion around capability alignment, design support, or fast-start execution rather than a generic intro.",
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is competitive positioning, look for dissatisfaction, transition risk, or new mandate areas where the POI signal around {} creates permission for a differentiated pitch.",
                entity
            ));
        }
        (false, "security_compliance")
        | (false, "cybersecurity_threat")
        | (false, "quality_compliance") => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is assurance, gather the evidence that would reassure auditors, customers, and procurement teams that {} still clears the relevant security or quality gate.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is supplier governance, update scorecards, audit timing, and exception handling for programs exposed to {}{} rather than waiting for a formal customer escalation.",
                entity, region_note
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the priority is commercial positioning, use the compliance gap around {} to show why stronger process control, cert depth, or cyber hygiene changes supplier choice today.",
                entity
            ));
        }
        (false, _) => {
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is commercial upside, decide whether this signal around {} is best used for pipeline capture, relationship expansion, or competitive displacement and tailor the outreach accordingly.",
                entity
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is resilience, translate the current evidence into specific sourcing, routing, or qualification choices instead of treating it as background monitoring.",
            ));
            push_unique_suggestion(&mut suggestions, format!(
                "If the goal is executive planning, brief stakeholders on which accounts, programs, or regions deserve action first and what concrete decision each team needs to make next.",
            ));
        }
    }

    if flags.pricing {
        push_unique_suggestion(&mut suggestions, format!(
            "If pricing or cost pressure is part of the pattern, compare margin, lead-time, and resilience trade-offs explicitly before {} resets expectations in the market.",
            entity
        ));
    }
    if flags.innovation || flags.expansion {
        push_unique_suggestion(&mut suggestions, format!(
            "If technology or capacity build is underway, treat this as a timing problem: the best opening is usually before the new capability at {} becomes fully standardized and crowded.",
            entity
        ));
    }
    if flags.shortage || flags.regulatory || flags.geopolitical {
        push_unique_suggestion(&mut suggestions, format!(
            "If regional exposure is rising, pressure-test alternative sites, suppliers, and customer commitments now rather than assuming the current footprint around {} remains stable.",
            entity
        ));
    }
    if flags.personnel {
        push_unique_suggestion(&mut suggestions, format!(
            "If the people signal is real, refresh stakeholder maps and project ownership before deciding who to brief, who to sell to, and who may block progress around {}.",
            entity
        ));
    }

    if let Some(action_hint) = action_hint {
        for fragment in action_hint
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(3)
        {
            if is_generic_action_hint(fragment) {
                continue;
            }
            push_unique_suggestion(
                &mut suggestions,
                format!(
                    "One concrete lane worth testing is this: {}.",
                    fragment.trim_end_matches('.')
                ),
            );
        }
    }

    suggestions.truncate(5);
    suggestions
}

/// Build an analytical narrative paragraph from structured signal data.
fn build_analytical_narrative(
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    signal_details: &[String],
    _confidence: f64,
    _evidence_ids: &[String],
) -> String {
    let category_desc = match category {
        "competitor_market" => "competitive market activity",
        "demand_procurement" => "demand and procurement developments",
        "supply_chain_risk" => "supply chain risk indicators",
        "security_compliance" => "security and compliance concerns",
        "regulatory_policy" => "regulatory and policy changes",
        "strategic_poi" => "strategic developments involving key personnel",
        "pricing_market" => "pricing and market dynamics",
        "customer_rfq" => "a potential customer procurement opportunity",
        "geopolitical_analysis" => "geopolitical developments affecting operations",
        "veracity_analysis" => "cross-referenced intelligence reporting",
        "talent_ip" => "talent movement and intellectual property activity",
        "technology_innovation" => "technology and R&D developments",
        "ma_partnerships" => "merger, acquisition, or partnership activity",
        "market_expansion" => "market expansion and capacity investments",
        "cybersecurity_threat" => "cybersecurity threats and vulnerabilities",
        "quality_compliance" => "quality certification and compliance changes",
        "brand_sentiment" => "brand sentiment and reputation signals",
        _ => "notable developments",
    };

    let entity_ctx = if !entity_label.is_empty() {
        if !entity_region.is_empty() {
            format!("{} ({})", entity_label, entity_region)
        } else {
            entity_label.to_string()
        }
    } else {
        "An entity under monitoring".to_string()
    };

    let mut parts = Vec::new();
    let flags = detect_strategy_signal_flags(category, signal_details, "");
    let is_public_sector = is_public_sector_entity(entity_label, entity_type);
    let concrete_details = concrete_signal_details(signal_details);

    // Opening analytical sentence
    parts.push(format!(
        "{entity_ctx} has been flagged for {category_desc}."
    ));

    // Signal evidence paragraph
    if !signal_details.is_empty() {
        parts.push(format!(
            "Our monitoring detected: {}.",
            signal_details.join("; ")
        ));
    }
    if !concrete_details.is_empty() {
        parts.push(format!(
            "The clearest current evidence is {}.",
            concrete_details
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    let mut implications: Vec<String> = Vec::new();
    if flags.procurement {
        implications.push(
            "The signal mix points to an active supplier-evaluation window rather than routine background noise, which means early positioning can shape shortlist, prototype, or qualification scope before it becomes a price-only contest.".to_string()
        );
    }
    if flags.qualification {
        implications.push(format!(
            "Qualification and certification clues tighten the supplier envelope around {entity_ctx}; in regulated sectors, audit readiness and process proof usually matter before commercial terms are finalized."
        ));
    }
    if flags.shortage {
        implications.push(
            "Supply disruption indicators shift the conversation from optimization to continuity: if the constraint persists, customers will look for dual-source coverage, buffer stock, migration paths, or faster escalation before schedules move.".to_string()
        );
    }
    if flags.regulatory || flags.geopolitical {
        implications.push(
            "Policy and trade exposure can quickly make footprint, routing, and export eligibility more decisive than nominal unit cost, especially where EU and North Africa positioning changes the compliance or resilience story.".to_string()
        );
    }
    if flags.competitor {
        implications.push(
            "Competitive movement here creates both a defense problem and a displacement opportunity; the key question is which accounts will feel pain first and what proof would make them switch.".to_string()
        );
    }
    if flags.innovation || flags.expansion {
        implications.push(format!(
            "Capacity, innovation, or program-build signals around {entity_ctx} usually surface months before operations stabilize, so the real leverage is in early influence, allocation access, and partnership framing rather than late reactive outreach."
        ));
    }
    if flags.personnel {
        implications.push(
            "POI and hiring clues add a power-mapping dimension: they often reveal who is building budget, launching a program, or quietly changing supplier criteria before the organization says so publicly.".to_string()
        );
    }
    if flags.pricing {
        implications.push(
            "Pricing signals imply a quoting or margin reset is underway, which can be used either to protect current business or to present a stronger total-cost and resilience case to buyers under pressure.".to_string()
        );
    }
    if flags.security && is_public_sector {
        implications.push(
            "For a government entity, the security question is whether the signal touches official domains, procurement portals, citizen-facing services, or supplier access paths; that determines whether this is a real operational issue or just background cyber noise.".to_string()
        );
    }
    if flags.security && !is_public_sector {
        implications.push(
            "Security and assurance issues rarely stay isolated; they spill into vendor eligibility, customer audits, and executive risk discussions much faster than routine operational changes.".to_string()
        );
    }
    if implications.is_empty() && is_public_sector && category == "brand_sentiment" {
        implications.push(
            "For a public-sector body, sentiment is only actionable if it starts to change institutional credibility, oversight pressure, procurement scrutiny, or decision timing; otherwise it should be treated as background noise rather than a direct commercial trigger.".to_string()
        );
    }
    if implications.is_empty() {
        implications.push(
            "Taken together, these signals matter because they change commercial timing, supplier choice, or executive priorities rather than representing isolated informational noise.".to_string()
        );
    }
    parts.push(
        implications
            .into_iter()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
    );

    parts.join(" ")
}

/// Build an analytical title from entity and category data.
fn build_analytical_title(
    entity_label: &str,
    entity_region: &str,
    category: &str,
    signal_details: &[String],
    entity_type: Option<&str>,
) -> String {
    // Check if this is a government entity
    let is_government = is_public_sector_entity(entity_label, entity_type);

    // Use different action verbs for government entities
    let action = if is_government {
        match category {
            "competitor_market" => "regulatory activity detected",
            "demand_procurement" => "tender/solicitation identified",
            "supply_chain_risk" => "policy risk flagged",
            "security_compliance" => "regulatory update detected",
            "regulatory_policy" => "policy change detected",
            "strategic_poi" => "official stakeholder activity surfaced",
            "pricing_market" => "economic policy shift detected",
            "customer_rfq" => "procurement signal emerging",
            "geopolitical_analysis" => "policy and trade exposure shift",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "official appointment detected",
            "technology_innovation" => "research initiative detected",
            "ma_partnerships" => "bilateral agreement detected",
            "market_expansion" => "expansion signal with sourcing implications",
            "cybersecurity_threat" => "security advisory detected",
            "quality_compliance" => "standards update detected",
            "brand_sentiment" => "public sentiment signal detected",
            _ => "notable development detected",
        }
    } else {
        match category {
            "competitor_market" => "competitive activity detected",
            "demand_procurement" => "procurement opportunity identified",
            "supply_chain_risk" => "supply chain risk flagged",
            "security_compliance" => "compliance gap identified",
            "regulatory_policy" => "regulatory change detected",
            "strategic_poi" => "stakeholder activity surfaced",
            "pricing_market" => "market pricing shift detected",
            "customer_rfq" => "procurement signal emerging",
            "geopolitical_analysis" => "geopolitical exposure assessment required",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "talent/IP activity detected",
            "technology_innovation" => "R&D activity detected",
            "ma_partnerships" => "M&A/partnership activity detected",
            "market_expansion" => "capacity or market expansion signal",
            "cybersecurity_threat" => "cybersecurity threat detected",
            "quality_compliance" => "quality certification change detected",
            "brand_sentiment" => "brand sentiment signal detected",
            _ => "notable development detected",
        }
    };

    let region_tag = if !entity_region.is_empty() {
        format!(" ({})", entity_region)
    } else {
        String::new()
    };

    // Add one concrete detail if we have signal data
    let detail = concrete_signal_details(signal_details)
        .first()
        .map(|s| format!(". {}", s))
        .unwrap_or_default();

    let title = format!("{}{}: {}{}", entity_label, region_tag, action, detail);
    if title.chars().count() > 200 {
        let t: String = title.chars().take(197).collect();
        format!("{t}...")
    } else {
        title
    }
}

/// Rich evidence signal with category and structured data.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct EvidenceSignal {
    title: String,
    description: String,
    source_url: String,
    signal_type: String, // e.g., "certification", "capability", "news", "poi", "warning"
    extracted_facts: Vec<String>, // Key facts extracted from the evidence
    date_context: Option<String>, // When this happened/detected
    relevance_score: f32, // How relevant to the insight category
}

/// Entity context with rich structured data about the company/org.
#[cfg(feature = "llm")]
#[derive(Clone)]
struct EntityContext {
    name: String,
    region: String,
    entity_type: Option<String>,
    /// True = this entity is a direct EMS competitor; False = customer/partner/prospect
    is_competitor: bool,
    industry_tags: Vec<String>,
    certifications: Vec<String>, // e.g., "AS9100D (valid until 2027-03)", "ISO 13485"
    capabilities: Vec<String>,   // e.g., "High-Volume SMT", "Medical Device Manufacturing"
    key_persons: Vec<String>,    // e.g., "CEO: Jensen Huang (C-Suite, influential)"
    recent_changes: Vec<String>, // e.g., "New facility detected", "Leadership change"
    // Rich competitive context
    threat_score: Option<f64>,
    overlap_score: Option<f64>,
    strategic_relevance: Option<f64>,
    revenue_estimate_usd: Option<i64>,
    employee_estimate: Option<i32>,
    competitor_names: Vec<String>, // Linked competitors via graph edges
    sites_summary: Vec<String>,    // e.g., "Manufacturing plant in Tunis, Tunisia"
    competitor_events: Vec<String>, // Recent competitor moves
    domain: Option<String>,
}

/// Extract key facts from evidence text using pattern matching.
/// Returns a list of specific facts (names, dates, numbers, locations).
#[cfg(feature = "llm")]
fn extract_facts_from_text(text: &str) -> Vec<String> {
    use regex::Regex;
    lazy_static::lazy_static! {
        // Match monetary values
        static ref MONEY_RE: Regex = Regex::new(r"\$[\d,.]+\s*(?:M|B|million|billion|thousand)?|\d+[\d,]*\s*(?:USD|EUR|GBP)").unwrap();
        // Match percentages
        static ref PERCENT_RE: Regex = Regex::new(r"\d+(?:\.\d+)?%").unwrap();
        // Match dates
        static ref DATE_RE: Regex = Regex::new(r"(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.?\s+\d{1,2},?\s+\d{4}|\d{4})").unwrap();
        // Match certification standards
        static ref CERT_RE: Regex = Regex::new(r"(?:ISO|AS|IATF|IEC|MIL-STD|NADCAP|FDA|CE|UL|RoHS)\s*[\d:-]+(?:\s*[A-Z])?").unwrap();
        // Match employee counts
        static ref EMPLOYEE_RE: Regex = Regex::new(r"\d+[\d,]*\s*(?:employees|workers|staff|headcount)").unwrap();
        // Match acquisition/merger phrases
        static ref ACQUISITION_RE: Regex = Regex::new(r"(?:acquir(?:ed|es|ing)|merger|bought|purchas(?:ed|e))\s+(?:by\s+)?([A-Z][a-zA-Z\s&]+)").unwrap();
        // Match facility/location signals
        static ref FACILITY_RE: Regex = Regex::new(r"(?:new\s+)?(?:facility|plant|factory|site|headquarters)\s+(?:in\s+)?([A-Z][a-zA-Z\s,]+)").unwrap();
    }

    let mut facts = Vec::new();

    // Extract monetary values
    for m in MONEY_RE.find_iter(text).take(3) {
        facts.push(format!("Value: {}", m.as_str()));
    }

    // Extract percentages
    for m in PERCENT_RE.find_iter(text).take(2) {
        facts.push(format!("Change: {}", m.as_str()));
    }

    // Extract certifications
    for m in CERT_RE.find_iter(text).take(3) {
        facts.push(format!("Certification: {}", m.as_str()));
    }

    // Extract dates
    for m in DATE_RE.find_iter(text).take(2) {
        let date_str = m.as_str();
        // Skip years that are part of cert standards
        if date_str.len() > 4 || !text.contains(&format!(":{}", date_str)) {
            facts.push(format!("Date: {}", date_str));
        }
    }

    // Extract employee counts
    for m in EMPLOYEE_RE.find_iter(text).take(1) {
        facts.push(m.as_str().to_string());
    }

    // Extract acquisition mentions
    for caps in ACQUISITION_RE.captures_iter(text).take(1) {
        if let Some(m) = caps.get(1) {
            facts.push(format!("Acquisition involving {}", m.as_str().trim()));
        }
    }

    // Extract facility mentions
    for caps in FACILITY_RE.captures_iter(text).take(1) {
        if let Some(m) = caps.get(1) {
            facts.push(format!("Facility in {}", m.as_str().trim()));
        }
    }

    facts
}

/// Calculate relevance score of evidence to a category.
#[cfg(feature = "llm")]
fn calculate_relevance(title: &str, description: &str, signal_type: &str, category: &str) -> f32 {
    let text = format!("{} {} {}", title, description, signal_type).to_lowercase();

    // Category-specific keywords with weights
    let keywords: &[(&str, f32)] = match category {
        "demand_procurement" => &[
            ("rfq", 1.0),
            ("tender", 1.0),
            ("procurement", 0.9),
            ("sourcing", 0.8),
            ("bid", 0.8),
            ("contract", 0.7),
            ("supplier", 0.6),
            ("vendor", 0.6),
        ],
        "supply_chain_risk" => &[
            ("shortage", 1.0),
            ("delay", 0.9),
            ("disruption", 0.9),
            ("risk", 0.8),
            ("constraint", 0.8),
            ("lead time", 0.7),
            ("allocation", 0.7),
            ("single source", 0.9),
        ],
        "competitor_market" => &[
            ("competitor", 1.0),
            ("market share", 0.9),
            ("pricing", 0.8),
            ("win", 0.8),
            ("lost", 0.8),
            ("expansion", 0.7),
            ("capability", 0.6),
            ("capacity", 0.6),
        ],
        "security_compliance" => &[
            ("certification", 1.0),
            ("iso", 0.9),
            ("as9100", 1.0),
            ("iatf", 1.0),
            ("compliance", 0.9),
            ("audit", 0.8),
            ("accreditation", 0.9),
            ("security", 0.7),
        ],
        "regulatory_policy" => &[
            ("regulation", 1.0),
            ("tariff", 0.9),
            ("sanction", 1.0),
            ("export control", 1.0),
            ("policy", 0.8),
            ("legislation", 0.8),
            ("compliance", 0.7),
            ("itar", 1.0),
        ],
        "strategic_poi" => &[
            ("ceo", 1.0),
            ("cto", 1.0),
            ("cfo", 1.0),
            ("executive", 0.9),
            ("appointed", 0.9),
            ("resigned", 0.9),
            ("leadership", 0.8),
            ("vp", 0.7),
        ],
        "ma_partnerships" => &[
            ("acquisition", 1.0),
            ("merger", 1.0),
            ("partnership", 0.9),
            ("joint venture", 0.9),
            ("acquired", 1.0),
            ("divest", 0.9),
            ("spin-off", 0.8),
            ("alliance", 0.7),
        ],
        "technology_innovation" => &[
            ("patent", 1.0),
            ("r&d", 0.9),
            ("innovation", 0.9),
            ("breakthrough", 0.9),
            ("technology", 0.7),
            ("launch", 0.7),
            ("product", 0.6),
            ("development", 0.6),
        ],
        "cybersecurity_threat" => &[
            ("breach", 1.0),
            ("vulnerability", 1.0),
            ("cyber", 0.9),
            ("attack", 0.9),
            ("security", 0.7),
            ("malware", 1.0),
            ("ransomware", 1.0),
            ("incident", 0.8),
        ],
        _ => &[("signal", 0.5), ("detected", 0.5), ("update", 0.4)],
    };

    let mut score: f32 = 0.3; // Base relevance
    for (keyword, weight) in keywords {
        if text.contains(keyword) {
            score += weight;
        }
    }

    // Boost for specific signal types
    match signal_type {
        "certification" if category.contains("compliance") || category.contains("security") => {
            score += 0.5
        }
        "capability" if category.contains("procurement") || category.contains("competitor") => {
            score += 0.4
        }
        "poi" if category.contains("strategic_poi") || category.contains("talent") => score += 0.6,
        "warning" => score += 0.3, // Warnings are generally relevant
        _ => {}
    }

    score.min(2.0) // Cap at 2.0
}

/// Extract article titles from warning description text.
/// Descriptions often contain "Articles: Title One; Title Two" patterns.
#[allow(dead_code)]
fn extract_article_titles(description: &str) -> Vec<String> {
    let mut articles = Vec::new();
    // Pattern: "Articles: ..." or "Related articles: ..."
    if let Some(idx) = description
        .find("Articles:")
        .or_else(|| description.find("articles:"))
    {
        let after = &description[idx..];
        // Find the article list — it ends at "Source:" or "See:" or end of string
        let end = after
            .find(". Source:")
            .or_else(|| after.find(". See:"))
            .or_else(|| after.find(". Page"))
            .unwrap_or(after.len());
        let articles_text = &after[after.find(':').map(|i| i + 1).unwrap_or(0)..end].trim();
        for title in articles_text.split(';') {
            let t = title.trim();
            if t.len() > 10 && !t.starts_with("http") {
                articles.push(t.to_string());
            }
        }
    }
    articles
}

/// Clean a signal title: remove entity name prefixes like "Flex Ltd: Career detected"
/// and filter out low-value generic titles.
#[allow(dead_code)]
fn clean_signal_title(title: &str, entity_name: &str) -> String {
    let mut cleaned = title.to_string();
    // Strip "EntityName: " prefix
    if let Some(rest) = cleaned.strip_prefix(&format!("{}: ", entity_name)) {
        cleaned = rest.to_string();
    }
    // Strip "Signal 'type' detected" patterns — not useful as-is
    if cleaned.ends_with(" detected") && cleaned.len() < 40 {
        return String::new();
    }
    // Filter out generic observation type titles
    let lower = cleaned.to_lowercase();
    if lower.contains("update") && lower.len() < 30 {
        return String::new(); // "SocialPost update", "WebChange update" etc.
    }
    if lower == "lookalike_domain" || lower.starts_with("dns_") || lower.starts_with("webchange") {
        return String::new();
    }
    cleaned
}

/// Deduplicate signals: keep only unique signals based on title similarity.
#[cfg(feature = "llm")]
fn dedup_signals<'a>(signals: &[&'a EvidenceSignal]) -> Vec<&'a EvidenceSignal> {
    let mut seen_titles: Vec<String> = Vec::new();
    let mut result = Vec::new();
    for sig in signals {
        let lower = sig.title.to_lowercase();
        let dominated = seen_titles.iter().any(|seen| {
            // Simple substring overlap check
            seen.contains(&lower)
                || lower.contains(seen.as_str())
                || (lower.len() > 15 && seen.len() > 15 && lower[..15] == seen[..15])
        });
        if !dominated {
            seen_titles.push(lower);
            result.push(*sig);
        }
    }
    result
}

/// Describe entity type in human language (with correct article: "a" or "an")
#[allow(dead_code)]
fn describe_entity_type_with_article(entity_type: Option<&str>) -> &str {
    match entity_type {
        Some(t) if t.to_lowercase().contains("ems") => {
            "an electronics manufacturing services (EMS) provider"
        }
        Some(t) if t.to_lowercase().contains("oem") => "an original equipment manufacturer (OEM)",
        Some(t) if t.to_lowercase().contains("semiconductor") => "a semiconductor manufacturer",
        Some(t) if t.to_lowercase().contains("defense") || t.to_lowercase().contains("defence") => {
            "a defense and aerospace company"
        }
        Some(t) if t.to_lowercase().contains("government") => "a government entity",
        Some(t) if t.to_lowercase().contains("automotive") => "an automotive tier supplier",
        Some(t) if t.to_lowercase().contains("distributor") => {
            "an electronic components distributor"
        }
        _ => "a company",
    }
}

/// Compose a rich analytical insight from structured evidence data.
/// Returns (title, summary) — no LLM required.
#[cfg(feature = "llm")]
fn compose_analytical_insight(
    entity_ctx: &EntityContext,
    category: &str,
    evidence_signals: &[EvidenceSignal],
    severity: &str,
    confidence: f64,
) -> (String, String) {
    let name = &entity_ctx.name;
    let region = if entity_ctx.region.is_empty() {
        "undisclosed region".to_string()
    } else {
        entity_ctx.region.clone()
    };

    // ── Deduplicate and rank signals ──
    let mut sorted: Vec<&EvidenceSignal> = evidence_signals.iter().collect();
    sorted.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let deduped = dedup_signals(&sorted);
    let top_signals: Vec<&EvidenceSignal> = deduped.into_iter().take(5).collect();

    // ── Extract article titles from descriptions (the richest data source) ──
    let mut article_titles: Vec<String> = Vec::new();
    for sig in &top_signals {
        let mut titles = extract_article_titles(&sig.description);
        article_titles.append(&mut titles);
    }
    // Dedup articles: strip trailing dots, then fuzzy-dedup by prefix
    article_titles = article_titles
        .into_iter()
        .map(|a| a.trim_end_matches('.').trim().to_string())
        .filter(|a| a.len() > 10)
        .collect();
    article_titles.sort();
    article_titles.dedup();
    // Remove articles that are substrings of other articles
    let articles_clone = article_titles.clone();
    article_titles.retain(|a| {
        let a_lower = a.to_lowercase();
        !articles_clone.iter().any(|other| {
            let o_lower = other.to_lowercase();
            o_lower != a_lower && o_lower.contains(&a_lower)
        })
    });
    let article_titles: Vec<String> = article_titles.into_iter().take(4).collect();

    // ── Classify signal types present (scan ALL signals, not just top) ──
    let all_text: String = evidence_signals
        .iter()
        .map(|s| {
            format!(
                "{} {} {}",
                s.title,
                s.description,
                s.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let has_acquisition = all_text.contains("acquisition")
        || all_text.contains("acquir")
        || all_text.contains("merger")
        || all_text.contains("joint venture");
    let has_certification = evidence_signals
        .iter()
        .any(|s| s.signal_type == "certification")
        || all_text.contains("iso ")
        || all_text.contains("as9100")
        || all_text.contains("iatf");
    let has_capability = evidence_signals
        .iter()
        .any(|s| s.signal_type == "capability");
    let has_innovation = all_text.contains("innovation")
        || all_text.contains("award")
        || all_text.contains("patent")
        || all_text.contains("r&d");
    let has_expansion = all_text.contains("facility")
        || all_text.contains("expansion")
        || all_text.contains("groundbreaking")
        || all_text.contains("new plant")
        || all_text.contains("new site");
    let has_hiring = all_text.contains("career")
        || all_text.contains("hiring")
        || all_text.contains("recruit")
        || all_text.contains("job opening");
    let has_compliance = all_text.contains("compliance")
        || all_text.contains("audit")
        || all_text.contains("regulation");
    let has_tariff = all_text.contains("tariff")
        || all_text.contains("sanction")
        || all_text.contains("trade war")
        || all_text.contains("embargo")
        || all_text.contains("export control");
    let has_shortage = all_text.contains("shortage")
        || all_text.contains("allocation")
        || all_text.contains("lead time")
        || all_text.contains("supply constraint")
        || all_text.contains("force majeure");
    let has_financial = all_text.contains("revenue")
        || all_text.contains("quarter")
        || all_text.contains("earnings")
        || all_text.contains("fiscal")
        || all_text.contains("investor");

    // ── Gather entity data ──
    let cert_list: Vec<&str> = entity_ctx
        .certifications
        .iter()
        .take(3)
        .map(|s| s.as_str())
        .collect();
    let cap_list: Vec<&str> = entity_ctx
        .capabilities
        .iter()
        .take(4)
        .map(|s| s.as_str())
        .collect();
    let poi_list: Vec<&str> = entity_ctx
        .key_persons
        .iter()
        .take(2)
        .map(|s| s.as_str())
        .collect();

    // Extract all concrete facts
    let all_facts: Vec<String> = top_signals
        .iter()
        .flat_map(|s| s.extracted_facts.iter().cloned())
        .filter(|f| !f.is_empty())
        .take(6)
        .collect();

    // ── Build headline ──
    let headline = build_rich_headline(
        name,
        &region,
        category,
        &top_signals,
        &article_titles,
        &cert_list,
        &cap_list,
        has_acquisition,
        has_expansion,
        has_innovation,
        has_tariff,
        has_shortage,
        has_financial,
        has_certification,
    );

    // ── Build narrative paragraphs ──
    let mut paragraphs: Vec<String> = Vec::new();

    // Paragraph 1: What happened
    let what_happened = build_what_happened(
        name,
        &region,
        &top_signals,
        &article_titles,
        &all_facts,
        &cert_list,
        &cap_list,
        entity_ctx.entity_type.as_deref(),
    );
    if !what_happened.is_empty() {
        paragraphs.push(what_happened);
    }

    // Paragraph 2: Why it matters — strategic analysis
    let why_matters = build_why_it_matters(
        name,
        &region,
        category,
        &cert_list,
        &cap_list,
        &poi_list,
        has_acquisition,
        has_certification,
        has_capability,
        has_innovation,
        has_expansion,
        has_hiring,
        has_compliance,
        has_tariff,
        has_shortage,
        has_financial,
        &article_titles,
    );
    paragraphs.push(why_matters);

    // Paragraph 3: Actionable recommendation
    let recommendation = build_recommendation(
        name,
        category,
        &cert_list,
        &cap_list,
        &poi_list,
        has_acquisition,
        has_expansion,
        has_tariff,
        has_shortage,
        has_compliance,
        has_innovation,
        has_financial,
        has_certification,
    );
    paragraphs.push(format!("Recommended action: {}", recommendation));

    // Priority callout for critical/elevated
    if severity == "critical" {
        paragraphs.push("Priority: CRITICAL — immediate executive attention required. Escalate within 48 hours.".into());
    } else if severity == "warning" {
        paragraphs.push(
            "Priority: Elevated — requires analyst review within the current planning cycle."
                .into(),
        );
    }

    // Confidence assessment
    let sig_count = evidence_signals.len().min(20);
    let conf_label = if confidence >= 0.85 {
        "high"
    } else if confidence >= 0.65 {
        "moderate"
    } else {
        "preliminary"
    };
    let conf_basis = if sig_count >= 5 {
        format!(
            "{} corroborating signals across multiple source types",
            sig_count
        )
    } else if sig_count >= 2 {
        format!("{} corroborating signals", sig_count)
    } else {
        "single-source intelligence".to_string()
    };
    paragraphs.push(format!(
        "Confidence: {} ({:.0}%), based on {}.",
        conf_label,
        confidence * 100.0,
        conf_basis
    ));

    (headline, paragraphs.join("\n\n"))
}

#[cfg(feature = "llm")]
fn build_rich_headline(
    name: &str,
    region: &str,
    category: &str,
    top_signals: &[&EvidenceSignal],
    article_titles: &[String],
    certs: &[&str],
    caps: &[&str],
    has_acquisition: bool,
    has_expansion: bool,
    has_innovation: bool,
    has_tariff: bool,
    has_shortage: bool,
    has_financial: bool,
    has_certification: bool,
) -> String {
    // Priority 1: Use article titles for specific, descriptive headlines
    if let Some(best_article) = article_titles.first() {
        let short = truncate_text(best_article, 80);
        return format!("{name}: {short}");
    }

    // Priority 2: Use clean signal title if informative
    for sig in top_signals {
        let cleaned = clean_signal_title(&sig.title, name);
        if cleaned.len() > 15 {
            return format!("{name}: {}", truncate_text(&cleaned, 80));
        }
    }

    // Priority 3: Signal-type + entity-data specific headline
    if has_acquisition {
        return format!("{name}: acquisition activity signals competitive repositioning");
    }
    if has_shortage {
        return format!("{name}: supply allocation constraints — assess component availability");
    }
    if has_tariff {
        return format!("{name}: trade policy exposure requires sourcing review in {region}");
    }
    if has_expansion {
        if !caps.is_empty() {
            return format!(
                "{name}: capacity expansion in {} manufacturing",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            );
        }
        return format!("{name}: facility expansion underway in {region}");
    }
    if has_innovation {
        if !caps.is_empty() {
            return format!(
                "{name}: technology evolution in {} — early engagement window",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            );
        }
        return format!("{name}: innovation and R&D signals indicate technology pivot");
    }
    if has_financial {
        return format!("{name}: financial activity signals — review business trajectory");
    }
    if has_certification && !certs.is_empty() {
        return format!(
            "{name}: {} certification update — verify qualification status",
            certs[0].split('(').next().unwrap_or(certs[0]).trim()
        );
    }

    // Priority 4: Category-specific with entity data enrichment
    match category {
        "demand_procurement" => {
            if !caps.is_empty() {
                format!(
                    "{name}: procurement signals in {} — sourcing opportunity",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name} ({region}): procurement activity indicates emerging sourcing opportunity")
            }
        }
        "supply_chain_risk" => {
            format!("{name}: supply chain risk indicators — mitigation planning required")
        }
        "competitor_market" => {
            format!("{name}: competitive positioning shift detected in {region}")
        }
        "security_compliance" => {
            if !certs.is_empty() {
                format!(
                    "{name}: {} compliance update — supplier qualification impact",
                    certs[0].split('(').next().unwrap_or(certs[0]).trim()
                )
            } else {
                format!("{name}: security and compliance posture update")
            }
        }
        "regulatory_policy" => format!("{name}: regulatory changes affect operations in {region}"),
        "strategic_poi" => {
            if !caps.is_empty() {
                format!(
                    "{name}: strategic signals in {} domain",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name}: strategic activity signals direction change in {region}")
            }
        }
        "technology_innovation" => {
            if !caps.is_empty() {
                format!(
                    "{name}: {} technology evolution — capability development signals",
                    caps[0].split('(').next().unwrap_or(caps[0]).trim()
                )
            } else {
                format!("{name}: technology investment signals emerging capability")
            }
        }
        "ma_partnerships" => {
            format!("{name}: M&A or partnership activity reshaping competitive landscape")
        }
        "market_expansion" => {
            if !certs.is_empty() {
                format!(
                    "{name}: market expansion with {} qualification in {region}",
                    certs[0].split('(').next().unwrap_or(certs[0]).trim()
                )
            } else {
                format!("{name}: market expansion activity detected in {region}")
            }
        }
        "quality_compliance" => {
            format!("{name}: quality system update — supplier requalification required")
        }
        "talent_ip" => {
            format!("{name}: talent and IP investment signals strategic capability build")
        }
        "customer_rfq" => format!("{name}: RFQ or customer engagement activity in {region}"),
        "geopolitical_analysis" => {
            format!("{name}: geopolitical exposure assessment required for {region}")
        }
        "pricing_market" => format!("{name}: pricing signals indicate market dynamics shift"),
        "cybersecurity_threat" => {
            format!("{name}: cybersecurity posture change — vendor risk review")
        }
        "brand_sentiment" => format!("{name}: brand and reputation signals require monitoring"),
        _ => format!("{name} ({region}): new intelligence signals require assessment"),
    }
}

#[cfg(feature = "llm")]
fn build_what_happened(
    name: &str,
    region: &str,
    top_signals: &[&EvidenceSignal],
    article_titles: &[String],
    all_facts: &[String],
    certs: &[&str],
    caps: &[&str],
    entity_type: Option<&str>,
) -> String {
    let type_desc = describe_entity_type_with_article(entity_type);

    let mut parts: Vec<String> = Vec::new();

    // If we have article titles from descriptions, lead with those (most specific data)
    if !article_titles.is_empty() {
        let articles_formatted: Vec<String> = article_titles
            .iter()
            .map(|a| format!("\"{}\"", truncate_text(a, 100)))
            .collect();
        if articles_formatted.len() == 1 {
            parts.push(format!(
                "Recent reporting on {name}, {type_desc} based in {region}, highlights: {}.",
                articles_formatted[0],
            ));
        } else {
            parts.push(format!(
                "Multiple recent developments at {name}, {type_desc} based in {region}. Key reports include: {}.",
                articles_formatted.join("; "),
            ));
        }
    } else if !top_signals.is_empty() {
        // No article titles — describe what types of signals we're seeing
        let signal_types: Vec<String> = {
            let mut types: Vec<String> = top_signals
                .iter()
                .map(|s| {
                    let cleaned = clean_signal_title(&s.title, name);
                    if cleaned.len() > 10 {
                        cleaned
                    } else {
                        // Fallback: use signal_type as description
                        match s.signal_type.as_str() {
                            "warning" => format!(
                                "{} activity signal",
                                s.title
                                    .split(':')
                                    .last()
                                    .unwrap_or("change")
                                    .trim()
                                    .to_lowercase()
                                    .replace(" detected", "")
                            ),
                            "certification" => format!(
                                "{} certification",
                                s.title.replace("Certification", "").trim()
                            ),
                            "capability" => s.title.replace("Capability: ", "").to_string(),
                            _ => s.signal_type.clone(),
                        }
                    }
                })
                .collect();
            types.sort();
            types.dedup();
            types.into_iter().take(4).collect()
        };
        if signal_types.len() == 1 {
            parts.push(format!(
                "Intelligence monitoring has flagged activity at {name}, {type_desc} in {region}: {}.",
                signal_types[0],
            ));
        } else {
            parts.push(format!(
                "Intelligence monitoring has flagged multiple activity vectors at {name}, {type_desc} in {region}, including: {}.",
                signal_types.join(", "),
            ));
        }
    }

    // Add certification context if present
    if !certs.is_empty() {
        parts.push(format!(
            "{name} holds active certifications: {}.",
            certs.join(", "),
        ));
    }

    // Add capability context
    if !caps.is_empty() && caps.len() >= 2 {
        parts.push(format!(
            "Verified manufacturing capabilities span {}, establishing {name}'s competency base.",
            caps.join(", "),
        ));
    } else if !caps.is_empty() {
        parts.push(format!("Verified capability: {}.", caps[0],));
    }

    // Add extracted facts (monetary values, percentages, dates, etc.)
    if !all_facts.is_empty() {
        let concrete_facts: Vec<&String> = all_facts
            .iter()
            .filter(|f| {
                f.contains('$')
                    || f.contains('%')
                    || f.contains("202")
                    || f.contains("employee")
                    || f.contains("facility")
            })
            .take(3)
            .collect();
        if !concrete_facts.is_empty() {
            parts.push(format!(
                "Extracted data points: {}.",
                concrete_facts
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
    }

    // Date context
    let dates: Vec<&str> = top_signals
        .iter()
        .filter_map(|s| s.date_context.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .take(2)
        .collect();
    if !dates.is_empty() {
        parts.push(format!(
            "Signal observation period: {}.",
            dates.join(" to ")
        ));
    }

    parts.join(" ")
}

#[allow(dead_code)]
fn build_why_it_matters(
    name: &str,
    region: &str,
    category: &str,
    certs: &[&str],
    caps: &[&str],
    pois: &[&str],
    has_acquisition: bool,
    _has_certification: bool,
    _has_capability: bool,
    has_innovation: bool,
    has_expansion: bool,
    has_hiring: bool,
    has_compliance: bool,
    has_tariff: bool,
    has_shortage: bool,
    has_financial: bool,
    article_titles: &[String],
) -> String {
    let mut analysis: Vec<String> = Vec::new();

    // ── Signal-type-specific strategic analysis ──
    if has_acquisition {
        analysis.push(format!(
            "Acquisition activity at {name} creates a 6-12 month integration window where procurement terms, \
             qualified supplier lists, and manufacturing site allocations are commonly restructured. \
             Existing programs face requalification risk if production lines are consolidated or relocated. \
             Competitors often exploit this transition period to capture displaced customers."
        ));
    }
    if has_shortage {
        analysis.push(format!(
            "Allocation signals from {name} indicate constrained supply capacity. In electronics supply chains, \
             formal allocation notices typically lag internal constraints by 2-4 weeks — this signal represents \
             an early warning. Downstream impact includes extended lead times, possible price escalation, \
             and risk of production line shutdowns for customers without dual-source qualification."
        ));
    }
    if has_tariff {
        analysis.push(format!(
            "Trade policy changes directly affect {name}'s cost structure and logistics routing. Tariff exposure \
             can shift landed costs by 5-25% depending on product classification. For {region}-based operations, \
             this may accelerate nearshoring decisions or trigger contract renegotiations. Programs with ITAR \
             or dual-use components face additional export control complexity."
        ));
    }
    if has_expansion {
        let cap_context = if !caps.is_empty() {
            format!(
                ", building on established {} capabilities",
                caps.iter()
                    .map(|c| c.split('(').next().unwrap_or(c).trim())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        analysis.push(format!(
            "Facility expansion by {name}{cap_context} signals either growing order book or strategic investment \
             in next-generation capacity. New manufacturing sites typically require 12-18 months for full \
             qualification (including customer audits, process validation, and regulatory certification). \
             This creates a narrow engagement window for procurement teams to secure preferred allocation \
             before capacity fills."
        ));
    }
    if has_innovation && !has_acquisition {
        let innovation_context = if !article_titles.is_empty() {
            format!(
                " Recent developments — including {} — suggest",
                truncate_text(&article_titles[0], 70)
            )
        } else {
            " Signals suggest".to_string()
        };
        analysis.push(format!(
            "Innovation activity at {name} indicates technology portfolio evolution.{innovation_context} \
             investment in capabilities that typically take 6-12 months to reach production readiness. \
             For procurement teams, this represents an early-mover window: engaging during development \
             phase can secure favorable terms and influence specification alignment."
        ));
    }
    if has_financial {
        analysis.push(format!(
            "Financial activity signals from {name} provide insight into business trajectory and investment capacity. \
             Revenue trends and investor communications often precede operational changes by 1-2 quarters, \
             making this a leading indicator for capacity planning and supplier risk assessment."
        ));
    }
    if has_hiring && !has_expansion && !has_acquisition {
        analysis.push(format!(
            "Recruitment patterns at {name} indicate either capacity ramp or capability build. \
             Engineering and manufacturing hiring signals typically precede new service offerings \
             or production line upgrades by 3-6 months. This is particularly relevant for qualification \
             teams tracking supplier capability evolution."
        ));
    }
    if has_compliance && !has_shortage && !has_tariff {
        let cert_context = if !certs.is_empty() {
            format!(" Current certifications ({}) define the qualification envelope for programs routed through {name}.", certs.join(", "))
        } else {
            String::new()
        };
        analysis.push(format!(
            "Compliance and regulatory activity at {name} directly affects supplier qualification status.{cert_context} \
             Any certification gap creates immediate procurement risk — programs requiring these standards \
             must have contingency routing or risk production interruption."
        ));
    }

    // ── POI context (adds power-mapping dimension) ──
    if !pois.is_empty() {
        analysis.push(format!(
            "Key decision-makers at {name} include {}. Changes in leadership or organizational structure \
             can signal strategic pivots — tracking these individuals provides early warning of \
             directional shifts in technology investment, market focus, and partnership strategy.",
            pois.join(" and "),
        ));
    }

    // ── Category-specific analysis if no strong signal-type match ──
    if analysis.is_empty() {
        let category_context = match category {
            "demand_procurement" => format!(
                "Procurement signals from {name} suggest active or upcoming sourcing activity in {region}. \
                 In the electronics manufacturing sector, early identification of procurement intent provides \
                 a 2-4 week positioning advantage for RFQ responses. The competitive window narrows quickly — \
                 suppliers engaging within the first week of a sourcing cycle are 3x more likely to be shortlisted."
            ),
            "supply_chain_risk" => format!(
                "These risk indicators from {name} require supply chain exposure assessment. Tier-1 and tier-2 \
                 dependency mapping should prioritize programs with single-source components or extended lead times. \
                 In the current market, supply disruption at a single node can cascade within 4-6 weeks to \
                 affect downstream production schedules."
            ),
            "competitor_market" => format!(
                "Competitive intelligence from {name} in {region} indicates market positioning activity. \
                 Review customer overlap and evaluate whether these movements create defensive requirements \
                 (protecting existing accounts) or offensive opportunities (capturing share from displaced programs). \
                 Market repositioning by a peer typically triggers 90-day response windows for affected suppliers."
            ),
            "regulatory_policy" => format!(
                "Regulatory changes affecting {name} in {region} may trigger compliance reassessment across \
                 the shared supply chain. Organizations under the same regulatory framework should review \
                 their own posture for alignment. Non-compliance at a tier-1 supplier can result in \
                 program hold or customer audit escalation within 30-60 days."
            ),
            "security_compliance" => {
                let cert_note = if !certs.is_empty() {
                    format!(" Current certifications ({}) define the compliance baseline.", certs.join(", "))
                } else {
                    String::new()
                };
                format!(
                    "Security and compliance posture at {name} is a critical qualifier for defense, aerospace, \
                     and regulated electronics manufacturing programs.{cert_note} Changes in security posture \
                     can trigger customer audit cascades and may affect active program eligibility."
                )
            }
            "technology_innovation" => {
                let cap_note = if !caps.is_empty() {
                    format!(" Current verified capabilities include {}.", caps.join(", "))
                } else {
                    String::new()
                };
                format!(
                    "Technology signals from {name} indicate R&D investment direction.{cap_note} \
                     New capabilities typically take 12-24 months from signal to production readiness, \
                     creating a strategic planning window for procurement teams evaluating next-generation \
                     sourcing options."
                )
            }
            "strategic_poi" => {
                let cap_note = if !caps.is_empty() {
                    format!(" with demonstrated capabilities in {}", caps.iter().map(|c| c.split('(').next().unwrap_or(c).trim()).collect::<Vec<_>>().join(", "))
                } else {
                    String::new()
                };
                format!(
                    "Strategic activity signals from {name}{cap_note} warrant executive-level review. \
                     These signals often precede material changes in business direction, partnership strategy, \
                     or market focus. For organizations with active commercial relationships, early awareness \
                     enables proactive positioning rather than reactive response."
                )
            }
            "market_expansion" => {
                let cert_note = if !certs.is_empty() {
                    format!(" Their certification portfolio ({}) supports market entry into regulated sectors.", certs.join(", "))
                } else {
                    String::new()
                };
                format!(
                    "Market expansion signals from {name} in {region} indicate geographic or sector diversification.{cert_note} \
                     Expansion-phase companies often offer competitive pricing to establish market presence, \
                     creating procurement cost optimization opportunities for early engagers."
                )
            }
            "ma_partnerships" => format!(
                "M&A and partnership activity around {name} reshapes the competitive landscape in {region}. \
                 Structural changes in the supply base take 6-18 months to fully materialize, but early signals \
                 enable strategic positioning — either securing continuity with changing entities or capturing \
                 business from disrupted competitors."
            ),
            "customer_rfq" => format!(
                "RFQ and customer engagement signals from {name} indicate active procurement cycles. \
                 Response timing is critical — in competitive environments, proposals submitted within \
                 the first 5 business days of an RFQ window receive disproportionate evaluation attention."
            ),
            "geopolitical_analysis" => format!(
                "Geopolitical signals affecting {name}'s operations in {region} may impact supply chain routing, \
                 export licensing, and partner eligibility. Organizations with cross-border dependencies \
                 should conduct scenario planning for potential trade disruption or regulatory change."
            ),
            "pricing_market" => format!(
                "Pricing and market dynamics signals from {name} indicate cost structure evolution. \
                 These shifts often propagate through the supply chain within 1-2 quarters, affecting \
                 program margins and competitive positioning for existing contracts."
            ),
            "cybersecurity_threat" => format!(
                "Cybersecurity signals from {name} require vendor risk assessment review. \
                 In interconnected supply chains, a security breach at one node can expose partner \
                 networks within hours. Verify {name}'s incident response posture and data handling \
                 practices for programs involving sensitive or controlled information."
            ),
            _ => format!(
                "Multiple intelligence signals from {name} in {region} indicate activity across supply chain, \
                 procurement, and competitive dimensions. The pattern warrants focused analyst review \
                 to determine whether these signals represent a transient fluctuation or sustained trend \
                 requiring strategic response."
            ),
        };
        analysis.push(category_context);
    }

    analysis.join(" ")
}

#[allow(dead_code)]
fn build_recommendation(
    name: &str,
    category: &str,
    certs: &[&str],
    caps: &[&str],
    pois: &[&str],
    has_acquisition: bool,
    has_expansion: bool,
    has_tariff: bool,
    has_shortage: bool,
    has_compliance: bool,
    has_innovation: bool,
    has_financial: bool,
    has_certification: bool,
) -> String {
    // Signal-type-specific high-priority recommendations
    if has_shortage {
        return format!(
            "URGENT: Contact {name}'s allocation desk within 48 hours to confirm supply commitment \
             for active programs. Simultaneously, review safety stock levels for all components sourced \
             through {name} and identify alternate qualified sources. If allocation impacts production \
             schedule, escalate to VP-Supply Chain with a dual-sourcing acceleration request."
        );
    }
    if has_tariff {
        let cert_note = if !certs.is_empty() {
            format!(
                " Ensure any alternative sources hold equivalent {} certification.",
                certs[0].split('(').next().unwrap_or(certs[0]).trim()
            )
        } else {
            String::new()
        };
        return format!(
            "Request {name}'s updated landed-cost analysis within 2 weeks reflecting current tariff \
             rates. Evaluate manufacturing sites in tariff-exempt or preferential-trade regions.{cert_note} \
             Brief procurement leadership on total exposure for contracts up for renewal in the next 6 months."
        );
    }
    if has_acquisition {
        let poi_note = if !pois.is_empty() {
            format!(
                " Coordinate with {} as primary contact during transition.",
                pois[0]
            )
        } else {
            String::new()
        };
        return format!(
            "Schedule a business continuity review with {name} within 30 days.{poi_note} \
             Request updated organizational chart, manufacturing site plan, and qualified supplier \
             list changes. Identify and begin pre-qualifying backup suppliers for any single-sourced \
             components routed through {name}."
        );
    }
    if has_expansion {
        let cap_mention = if !caps.is_empty() {
            format!(
                " specifically for {} programs",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            )
        } else {
            String::new()
        };
        return format!(
            "Engage {name}'s business development team to discuss early allocation at the new \
             facility{cap_mention}. Request site qualification timeline and add to the approved \
             supplier audit schedule. Target engagement within 30 days to secure position before \
             general capacity fill."
        );
    }
    if has_compliance || has_certification {
        let cert_mention = if !certs.is_empty() {
            format!(" specifically for {} validity and scope", certs[0])
        } else {
            String::new()
        };
        return format!(
            "Request {name}'s current certification status and upcoming audit schedule{cert_mention}. \
             Cross-reference with active program qualification requirements. Update supplier scorecard \
             with latest compliance data and flag any expiring certifications for proactive renewal \
             tracking within 2 weeks."
        );
    }
    if has_innovation {
        let cap_mention = if !caps.is_empty() {
            format!(
                ", building on their existing {} base",
                caps[0].split('(').next().unwrap_or(caps[0]).trim()
            )
        } else {
            String::new()
        };
        return format!(
            "Schedule a technical capability review with {name}'s engineering team within 30 days{cap_mention}. \
             Assess alignment with upcoming program requirements and next-generation specifications. \
             If capabilities match future needs, initiate pre-qualification process to establish \
             preferred status before competitors engage."
        );
    }
    if has_financial {
        return format!(
            "Include {name}'s financial trajectory in the next quarterly supplier risk review. \
             Monitor for follow-on announcements (restructuring, capex changes, guidance updates) \
             that could impact manufacturing capacity or pricing. Update Altman-Z or equivalent \
             financial health scoring within 2 weeks."
        );
    }

    // Category-specific recommendations
    match category {
        "demand_procurement" => format!(
            "Engage {name}'s procurement team within 2 weeks to assess the sourcing opportunity. \
             Prepare a capability brief emphasizing relevant certifications{} and manufacturing \
             track record. Identify internal champion for RFQ positioning.",
            if !certs.is_empty() { format!(" ({})", certs.join(", ")) } else { String::new() },
        ),
        "supply_chain_risk" => format!(
            "Conduct a supply chain exposure audit for {name} within 2 weeks. Map tier-1 and \
             tier-2 dependencies, identifying single-source and long-lead-time components. \
             Prepare contingency sourcing plan for the top-3 critical items and schedule \
             quarterly resilience reviews."
        ),
        "competitor_market" => format!(
            "Brief sales leadership on {name}'s competitive movements within 1 week. Conduct \
             a customer overlap analysis and prepare defensive positioning for at-risk accounts. \
             Update competitive intelligence database and set monitoring triggers for follow-on activity."
        ),
        "strategic_poi" => {
            let who = if !pois.is_empty() { format!("{} at ", pois[0]) } else { String::new() };
            format!(
                "Schedule an executive relationship check with {who}{name} within 30 days. \
                 Assess strategic alignment and explore partnership or preferred-supplier positioning. \
                 Update the strategic account plan with current intelligence."
            )
        }
        "market_expansion" => format!(
            "Evaluate {name}'s expansion trajectory for procurement cost optimization opportunities. \
             Expansion-phase suppliers often offer competitive terms. Request a capability presentation \
             and assess fit with upcoming program requirements within 3 weeks."
        ),
        "geopolitical_analysis" => format!(
            "Brief leadership on geopolitical exposure through {name} within 1 week. Conduct scenario \
             planning for potential trade disruption and review export licensing requirements. \
             Identify alternative routing or sourcing options for affected programs."
        ),
        "cybersecurity_threat" => format!(
            "Initiate vendor security assessment for {name} within 2 weeks. Review data handling \
             practices, incident response procedures, and network segmentation for shared systems. \
             Verify compliance with relevant security frameworks (NIST, ISO 27001, CMMC as applicable)."
        ),
        _ => format!(
            "Assign an analyst to conduct a focused assessment of {name}'s signals within 2 weeks. \
             Determine whether the pattern represents a one-time event or emerging trend. \
             Prepare a briefing for the relevant business unit stakeholders with specific \
             impact estimates and recommended responses."
        ),
    }
}

#[cfg(feature = "llm")]
fn count_numbered_references(text: &str, max_ref: usize) -> usize {
    (1..=max_ref)
        .filter(|i| text.contains(&format!("[{}]", i)))
        .count()
}

fn source_domain_from_url(url: &str) -> String {
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

fn format_sources_footer_from_urls(urls: &[String], max_sources: usize) -> String {
    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for url in urls
        .iter()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
    {
        let key = url.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(url.to_string());
            if deduped.len() >= max_sources {
                break;
            }
        }
    }

    if deduped.is_empty() {
        return String::new();
    }

    let items = deduped
        .iter()
        .enumerate()
        .map(|(index, url)| format!("[{}] {} — {}", index + 1, source_domain_from_url(url), url))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Sources:\n{}", items)
}

#[cfg(feature = "llm")]
fn ranked_source_urls(evidence_signals: &[EvidenceSignal], max_sources: usize) -> Vec<String> {
    let mut sorted: Vec<_> = evidence_signals.iter().collect();
    sorted.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for signal in sorted {
        let url = signal.source_url.trim();
        if url.is_empty() {
            continue;
        }
        let key = url.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(url.to_string());
            if out.len() >= max_sources {
                break;
            }
        }
    }
    out
}

#[cfg(feature = "llm")]
fn format_sources_footer(evidence_signals: &[EvidenceSignal], max_sources: usize) -> String {
    format_sources_footer_from_urls(
        &ranked_source_urls(evidence_signals, max_sources),
        max_sources,
    )
}

#[cfg(feature = "llm")]
fn low_signal_security_hygiene_case(category: &str, evidence_signals: &[EvidenceSignal]) -> bool {
    if !matches!(category, "security_compliance" | "cybersecurity_threat") {
        return false;
    }

    let corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    let has_hygiene_markers = [
        "dns posture",
        "dkim",
        "dmarc",
        "spf",
        "lookalike",
        "typosquat",
        "spoof",
        "spoofing",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_hard_incident_markers = [
        "breach",
        "compromise",
        "compromised",
        "ransomware",
        "malware",
        "incident",
        "outage",
        "exfiltrat",
        "unauthorized access",
        "account takeover",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_direct_business_markers = [
        "audit finding",
        "nonconformance",
        "tender exclusion",
        "contract loss",
        "customer complaint",
        "regulator action",
        "program delay",
        "production halt",
        "supplier removal",
        "export control action",
        "disqualified",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    has_hygiene_markers && !has_hard_incident_markers && !has_direct_business_markers
}

#[cfg(feature = "llm")]
fn contains_causal_link(text: &str) -> bool {
    [
        "because",
        "therefore",
        "which means",
        "leads to",
        "resulting in",
        "undermines",
        "creates",
        "causing",
        "could face",
        "pushing them to",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "llm")]
fn contains_security_hygiene_marker(text: &str) -> bool {
    [
        "dns posture",
        "dkim",
        "dmarc",
        "spf",
        "lookalike",
        "spoof",
        "typosquat",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "llm")]
fn contains_qualification_or_program_marker(text: &str) -> bool {
    [
        "as9100",
        "iso 13485",
        "iatf 16949",
        "defense program",
        "eu defense",
        "medical device",
        "program eligibility",
        "qualification",
        "compliance delays",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "llm")]
fn contains_customer_disruption_marker(text: &str) -> bool {
    [
        "supply chain disruption",
        "supply chain disruptions",
        "production interruption",
        "production halt",
        "customer audit cascade",
        "seek ems providers",
        "capture clients",
        "downstream customers",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "llm")]
fn contains_competitive_displacement_marker(text: &str) -> bool {
    [
        "opens door",
        "open the door",
        "opportunity for",
        "opportunities for",
        "target their",
        "seek alternatives",
        "switch suppliers",
        "switch campaign",
        "customers at risk",
        "nearshore shift",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "llm")]
fn has_unsupported_security_escalation(
    category: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !low_signal_security_hygiene_case(category, evidence_signals) {
        return false;
    }

    let combined = format!("{} {}", narrative, recommendation).to_ascii_lowercase();
    let conflates_hygiene_and_qualification = contains_security_hygiene_marker(&combined)
        && contains_qualification_or_program_marker(&combined)
        && contains_causal_link(&combined);
    let unsupported_customer_impact = contains_security_hygiene_marker(&combined)
        && contains_customer_disruption_marker(&combined);

    conflates_hygiene_and_qualification || unsupported_customer_impact
}

#[cfg(feature = "llm")]
fn low_signal_certification_warning_case(evidence_signals: &[EvidenceSignal]) -> bool {
    let corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    let has_certification_markers = [
        "certification",
        "accreditation",
        "iso ",
        "as9100",
        "iatf",
        "compliance",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_soft_warning_markers = [
        "warning",
        "flagged",
        "outdated",
        "update",
        "updated",
        "reaffirmation",
        "reaffirmed",
        "renewal",
        "renewed",
        "valid until",
        "detected",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    let has_hard_failure_markers = [
        "revoked",
        "suspended",
        "withdrawn",
        "decertified",
        "failed audit",
        "audit finding",
        "major nonconformance",
        "major non-conformance",
        "nonconformity",
        "non-conformity",
        "tender exclusion",
        "regulator action",
        "warning letter",
        "certificate expired",
        "certification expired",
        "expired on",
        "supplier removal",
        "disqualified",
    ]
    .iter()
    .any(|marker| corpus.contains(marker));

    has_certification_markers && has_soft_warning_markers && !has_hard_failure_markers
}

#[cfg(feature = "llm")]
fn has_unsupported_certification_escalation(
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !low_signal_certification_warning_case(evidence_signals) {
        return false;
    }

    let combined = format!("{} {}", narrative, recommendation).to_ascii_lowercase();
    let unsupported_qualification =
        contains_qualification_or_program_marker(&combined) && contains_causal_link(&combined);
    let unsupported_customer_impact = contains_customer_disruption_marker(&combined);
    let unsupported_competitive_displacement = contains_competitive_displacement_marker(&combined);

    unsupported_qualification || unsupported_customer_impact || unsupported_competitive_displacement
}

#[cfg(feature = "llm")]
fn has_unnamed_customer_targeting(narrative: &str, recommendation: &str) -> bool {
    let combined = format!("{} {}", narrative, recommendation).to_ascii_lowercase();

    [
        "target their ",
        "target its ",
        "their medical clients",
        "their industrial clients",
        "their aerospace clients",
        "their defense clients",
        "their automotive customers",
        "their medical device customers",
        "eu medical device customers",
        "industrial clients",
        "aerospace clients",
        "defense customers",
        "medical customers",
        "medical device customers",
        "automotive customers",
        "customers at risk",
        "underserved customers",
    ]
    .iter()
    .any(|marker| combined.contains(marker))
}

#[cfg(feature = "llm")]
fn has_unsupported_public_sector_commercialization(
    entity_ctx: &EntityContext,
    category: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref()) {
        return false;
    }

    if !matches!(
        category,
        "geopolitical_analysis" | "regulatory_policy" | "brand_sentiment"
    ) {
        return false;
    }

    if public_sector_procurement_or_program_case(evidence_signals) {
        return false;
    }

    let combined = format!("{} {}", narrative, recommendation).to_ascii_lowercase();
    let sales_pitch_markers = [
        "pcba",
        "pcb assembly",
        "box build",
        "electronics manufacturing services",
        "qualified to supply",
        "submit qualification package",
        "procurement team",
        "offer pcba",
        "offer supply chain management",
        "offer manufacturing",
        "defense-adjacent",
        "north african eu trade corridors",
        "north african eu trade corridor",
        "local manufacturing",
    ];

    sales_pitch_markers
        .iter()
        .any(|marker| combined.contains(marker))
}

#[cfg(feature = "llm")]
fn has_low_usefulness_public_sector_analysis(
    entity_ctx: &EntityContext,
    category: &str,
    headline: &str,
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> bool {
    if !is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref()) {
        return false;
    }

    if !matches!(
        category,
        "geopolitical_analysis" | "regulatory_policy" | "brand_sentiment"
    ) {
        return false;
    }

    if public_sector_procurement_or_program_case(evidence_signals) {
        return false;
    }

    let evidence_corpus = evidence_signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} {} {}",
                signal.title,
                signal.description,
                signal.signal_type,
                signal.extracted_facts.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let output_corpus =
        format!("{} {} {}", headline, narrative, recommendation).to_ascii_lowercase();
    let recommendation_lower = recommendation.to_ascii_lowercase();

    let evidence_has_artifact = contains_public_sector_artifact_marker(&evidence_corpus);
    let output_has_artifact = contains_public_sector_artifact_marker(&output_corpus)
        || recommendation_lower.contains("account dependency")
        || recommendation_lower.contains("stakeholder")
        || recommendation_lower.contains("qualification planning")
        || recommendation_lower.contains("policy impact")
        || recommendation_lower.contains("approval timing")
        || recommendation_lower.contains("procurement scrutiny");

    let generic_opportunity_markers = [
        "strategic outreach opportunity",
        "nearshoring opportunity",
        "opens ems opportunities",
        "ems opportunities",
        "open for ems partnerships",
        "supply chain partnerships",
        "partners for defense supply chain resilience",
        "opens nearshoring",
        "opportunity for eu/na ems",
    ];
    let generic_opportunity_language = generic_opportunity_markers
        .iter()
        .any(|marker| output_corpus.contains(marker));

    !evidence_has_artifact || !output_has_artifact || generic_opportunity_language
}

/// Generate insight narrative and headline using LLM with rich context.
/// Returns (headline, narrative, recommendation, confidence).
#[cfg(feature = "llm")]
async fn generate_llm_insight(
    llm_client: &InferenceLlmClient,
    entity_ctx: &EntityContext,
    category: &str,
    evidence_signals: &[EvidenceSignal],
) -> Result<(String, String, String, f64)> {
    use apex_llm::inference::{ChatMessage, InferenceConfig};

    if evidence_signals.is_empty() {
        anyhow::bail!("No evidence signals provided for LLM insight generation");
    }

    let is_public_sector =
        is_public_sector_entity(&entity_ctx.name, entity_ctx.entity_type.as_deref());
    let has_public_sector_procurement_program =
        public_sector_procurement_or_program_case(evidence_signals);

    // ── Build rich entity profile with competitive context ──
    let mut profile_parts: Vec<String> = Vec::new();
    let type_str = entity_ctx.entity_type.as_deref().unwrap_or("company");
    if type_str.to_lowercase().contains("government") {
        profile_parts.push(format!(
            "{} is a government/public sector entity in {}.",
            entity_ctx.name, entity_ctx.region
        ));
    } else {
        let mut desc = format!(
            "{} is a {} based in {}.",
            entity_ctx.name, type_str, entity_ctx.region
        );
        if let Some(rev) = entity_ctx.revenue_estimate_usd {
            desc.push_str(&format!(
                " Estimated revenue: ~${:.0}M.",
                rev as f64 / 1_000_000.0
            ));
        }
        if let Some(emp) = entity_ctx.employee_estimate {
            desc.push_str(&format!(" ~{} employees.", emp));
        }
        profile_parts.push(desc);
    }
    if let Some(domain) = &entity_ctx.domain {
        profile_parts.push(format!("Domain: {}", domain));
    }
    if !entity_ctx.industry_tags.is_empty() {
        profile_parts.push(format!(
            "Industry focus: {}.",
            entity_ctx.industry_tags.join(", ")
        ));
    }
    if !entity_ctx.certifications.is_empty() {
        let cert_str = entity_ctx
            .certifications
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Certifications: {}", cert_str));
    }
    if !entity_ctx.capabilities.is_empty() {
        let cap_str = entity_ctx
            .capabilities
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Capabilities: {}", cap_str));
    }
    if !entity_ctx.key_persons.is_empty() {
        let poi_str = entity_ctx
            .key_persons
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Key personnel: {}", poi_str));
    }
    if !entity_ctx.sites_summary.is_empty() {
        let sites_str = entity_ctx
            .sites_summary
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Sites/Facilities: {}", sites_str));
    }

    // Competitive positioning
    let mut comp_parts: Vec<String> = Vec::new();
    if let Some(ts) = entity_ctx.threat_score {
        comp_parts.push(format!("Threat score: {:.2}", ts));
    }
    if let Some(os) = entity_ctx.overlap_score {
        comp_parts.push(format!("Market overlap: {:.2}", os));
    }
    if let Some(sr) = entity_ctx.strategic_relevance {
        comp_parts.push(format!("Strategic relevance: {:.2}", sr));
    }
    if !entity_ctx.competitor_names.is_empty() {
        comp_parts.push(format!(
            "Linked competitors: {}",
            entity_ctx.competitor_names.join(", ")
        ));
    }
    if !comp_parts.is_empty() {
        profile_parts.push(format!("Competitive profile: {}", comp_parts.join(". ")));
    }
    if !entity_ctx.recent_changes.is_empty() {
        let changes_str = entity_ctx
            .recent_changes
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Recent changes: {}", changes_str));
    }
    if !entity_ctx.competitor_events.is_empty() {
        let events_str = entity_ctx
            .competitor_events
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        profile_parts.push(format!("Competitor intelligence: {}", events_str));
    }
    // Competitor classification — tells the LLM how to treat this entity
    if entity_ctx.is_competitor {
        profile_parts.push(
            "⚠️ ENTITY CLASSIFICATION: DIRECT EMS COMPETITOR — Do NOT recommend offering our services \
to this company. Analyse their weaknesses, identify which of their customers are underserved, \
and recommend approaching those customers instead.".to_string()
        );
    } else if is_public_sector {
        profile_parts.push(
            "🏛️ ENTITY CLASSIFICATION: PUBLIC-SECTOR BODY — treat this as an institutional account, not a default manufacturing prospect. Direct supplier outreach is only justified when the evidence explicitly names a procurement, tender, supplier qualification event, or hardware/equipment program.".to_string()
        );
    } else {
        profile_parts.push(
            "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity."
                .to_string(),
        );
    }
    let entity_profile = profile_parts.join("\n");

    // ── Build sorted evidence text ──
    let mut sorted_evidence: Vec<_> = evidence_signals.iter().collect();
    sorted_evidence.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let evidence_text: String = sorted_evidence
        .iter()
        .take(12)
        .enumerate()
        .map(|(i, sig)| {
            let mut parts = vec![format!("[{}] {} ({})", i + 1, sig.title, sig.signal_type)];
            if !sig.extracted_facts.is_empty() {
                parts.push(format!("   Key facts: {}", sig.extracted_facts.join("; ")));
            }
            if !sig.description.is_empty() {
                parts.push(format!(
                    "   Detail: {}",
                    crate::truncate_text(&sig.description, 420)
                ));
            }
            if let Some(date) = &sig.date_context {
                parts.push(format!("   When: {}", date));
            }
            if !sig.source_url.is_empty() {
                parts.push(format!("   Source: {}", sig.source_url));
            }
            parts.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // ── Category-specific guidance tuned for actionable intelligence ──
    let (category_label, analysis_focus, action_focus) = if entity_ctx.is_competitor {
        // For competitors: always focus on exploiting their weaknesses via their customers
        (
            "Competitive Intelligence",
            "Identify specific competitive vulnerabilities in this rival: lost certifications, \
delayed projects, leadership gaps, supply problems, customer complaints, capability gaps. \
Flag which of their customers are likely dissatisfied or underserved and ready to switch. \
For North African and EU competitors, identify market entry/exit signals, M&A activity, \
and regions where their coverage is thin.",
            "Name 2-3 specific companies that are CUSTOMERS OF THIS COMPETITOR and explain why \
they are now vulnerable to switching. Format: '<Customer company name from evidence> — \
<reason they are underserved by competitor right now> — <what we offer them> — by <deadline>'. \
NEVER address the competitor itself as a target.",
        )
    } else if is_public_sector {
        match category {
        "brand_sentiment" => (
            "Public-Sector Sentiment Brief",
            "Assess whether the signal changes institutional credibility, oversight pressure, procurement scrutiny, stakeholder messaging, or decision timing. Distinguish a media cycle from a formal review, public inquiry, procurement caution, or program delay. Do not infer direct electronics demand or supplier fit unless the evidence explicitly names a procurement or hardware program.",
            "Recommend account-planning, dependency mapping, evidence verification, or scenario-planning actions. Only propose direct outreach if the evidence explicitly names a procurement, tender, hardware requirement, or supplier qualification event tied to this entity.",
        ),
        "geopolitical_analysis" | "regulatory_policy" => (
            "Public-Sector Policy Impact Brief",
            "Explain what concrete policy, institutional, or trade artifact changed and which approvals, tenders, supplier pathways, or customer programs it could affect. Do not convert macro policy or innovation signals into a manufacturing sales pitch unless the evidence explicitly names procurement, equipment need, tender language, or supplier qualification requirements.",
            "Recommend concrete next steps such as policy-impact mapping, account dependency review, qualification planning, stakeholder outreach to named institutional roles, or scenario planning. If there is no explicit procurement or hardware program in evidence, do not propose PCBA, box build, or generic EMS outreach.",
        ),
        _ => (
            "Public-Sector Intelligence Brief",
            "Identify what institutional decision, review, procurement path, or stakeholder process is actually moving and what that changes for account planning. Keep direct evidence separate from commercial inference and avoid treating public-sector activity as a sales trigger without explicit buying or program evidence.",
            "Recommend evidence-based account actions, named stakeholder follow-up, or qualification planning. Direct supplier outreach requires explicit procurement or program evidence.",
        ),
    }
    } else {
        match category {
        "competitor_market" => (
            "Competitive Intelligence",
            "Identify specific competitive vulnerabilities: lost certifications, delayed projects, leadership gaps, supply problems, customer complaints. Flag which of their customers are likely dissatisfied or underserved. For North African and EU competitors, identify market entry/exit signals, M&A activity, and capability gaps we can exploit.",
            "Name 2-3 specific companies or contacts to call, with the exact pitch angle (e.g., 'their AS9100 lapsed — offer compliance consulting as a door-opener'). Include competitive displacement actions targeting specific accounts.",
        ),
        "demand_procurement" => (
            "Procurement Opportunity",
            "Identify what this entity is buying, building, or expanding. Map their procurement signals to products/services we can offer. Assess: buying timeline, budget signals (from revenue/hiring data), qualification requirements (from cert signals), and decision-makers (from POI data). Look for RFQ/tender signals in the evidence.",
            "Name the exact person or role to call, what to offer, and by when. If they posted jobs in engineering or procurement, infer what capability they're building and propose a supply partnership. Give a specific dollar-value opportunity estimate if evidence supports it.",
        ),
        "supply_chain_risk" => (
            "Supply Chain Opportunity",
            "Map this entity's supply chain vulnerabilities to our opportunities: their supplier delays become our pitch for alternative sourcing; their shortage signals become our capacity-availability play; their tariff exposure becomes our nearshoring pitch for North African production sites. Quantify disruption impact using any revenue/employee data available.",
            "Specify which of their supply chain gaps we can fill, which facility/site to propose, and which buyer to contact. If this is a competitor's problem, explain how to approach their downstream customers with reliability messaging.",
        ),
        "security_compliance" | "cybersecurity_threat" => (
            "Security Assurance Intelligence",
            "Separate observed security facts from commercial inference. Low-severity hygiene findings such as missing DKIM/SPF/DMARC records, low DNS posture scores, or isolated lookalike domains are vendor-assurance and fraud-risk signals, not proof of customer churn, defense-program exclusion, medical-device ineligibility, or supply disruption. Only escalate to audit failure, program eligibility, contract loss, or downstream operational impact if the evidence explicitly names a breach, outage, regulator action, tender requirement, customer response, or failed certification.",
            "Recommend measured actions tied to the evidence: assurance questions, remediation requests, or targeted outreach only where a named customer, program, or qualification requirement appears in the evidence. Every recommendation must cite the evidence it depends on.",
        ),
        "geopolitical_analysis" => (
            "Geopolitical Opportunity",
            "Translate geopolitical/regulatory shifts into commercial actions: new tariffs → nearshoring opportunity in Morocco/Tunisia; export control changes → qualification opportunity for EU-based alternatives; sanctions → market gap to fill. Focus on North African and EU angles. Map affected trade lanes to specific entities and their likely procurement pivots.",
            "Name the specific companies that will need to re-source and when. Specify which North African or EU production site to position. Include regulatory deadlines and qualification windows.",
        ),
        "regulatory_policy" | "pricing_market" => (
            "Market Intelligence",
            "Extract actionable business intelligence: explicit failed, revoked, or time-bound certification changes can create qualification windows; pricing signals reveal margin pressure at competitors; regulatory shifts create compliance consulting opportunities. Generic website certification notices, stale dates, or unspecified warning markers are not enough on their own to claim qualification failure, customer loss, or a switch opportunity. Cross-reference with known competitor capabilities and recent changes to identify where rivals are weak.",
            "Identify 2-3 specific commercial actions: companies to approach with compliance offers, pricing advantages to highlight, or capability gaps to fill. Name the buyer role and a specific deadline tied to the regulatory change.",
        ),
        _ => (
            "Business Intelligence",
            "Produce case-specific competitive intelligence. Identify who is winning, losing, hiring, cutting, expanding, or retreating. Cross-reference evidence to find exploitable patterns: a company hiring procurement staff likely has upcoming RFQs; a company with explicit certification loss or failed audit may have a compliance gap we can fill. Do not treat generic certification warnings, brochure dates, or accreditation mentions as proof of qualification failure.",
            "Name 2-3 specific actions with company names, contact roles, pitch angles, and deadlines. Every recommendation must answer: 'who do we call, what do we say, and by when?'",
        ),
    }
    };

    // ── System prompt: competitive intelligence operator, not passive analyst ──
    // Load our company profile from env so the model knows what we offer.
    let our_profile = std::env::var("COMPANY_PROFILE").unwrap_or_else(|_|
        "An electronics manufacturing services (EMS) company with certified production facilities in \
North Africa (Morocco, Tunisia) and Europe. Holds AS9100, ISO 9001, ISO 13485, and IATF 16949 \
certifications. Capabilities: PCBA assembly, box build, test & inspection, supply chain management. \
Focus markets: defense, aerospace, automotive, industrial, medical electronics."
        .to_string()
    );

    let system = format!("You are a competitive intelligence operator. Your task is to convert raw OSINT signals about a company into specific, commercially actionable intelligence for the following organization:

--- OUR COMPANY ---
{our_profile}
-------------------

{competitor_mode_instruction}

{public_sector_mode_instruction}

{brief_outcome_instruction}

Rules:
- Every claim must cite a numbered evidence reference [1], [2], etc.
- Our company profile is context, not proof of fit. Do not claim our certifications, footprint, or EMS capabilities are relevant unless the evidence explicitly names a matching procurement, hardware/equipment program, supplier qualification need, or manufacturing requirement.
- Keep direct evidence separate from inference. Do not turn minor hygiene findings such as DNS posture, missing DKIM/SPF/DMARC, or isolated lookalike domains into claims about defense-program exclusion, medical-device qualification failure, customer churn, or supply disruption unless the evidence explicitly links them.
- Do not turn generic certification or accreditation warnings, stale certificate dates, reaffirmation notices, or unspecified compliance page updates into claims about qualification failure, customer churn, tender exclusion, or switching urgency unless the evidence explicitly names a failed audit, revoked/expired certificate, regulator action, affected customer, or impacted program.
- Do not recommend targeting unnamed customer cohorts such as 'their medical clients' or 'aerospace customers'. If the evidence does not name a downstream company or program, keep the action on assurance, verification, remediation, or direct account mapping rather than invented switching outreach.
- For government and public-sector entities, do not invent hardware demand, manufacturing demand, quantity assumptions, or supplier-fit claims from innovation, media, diplomatic, or policy signals alone. Direct PCBA, box build, EMS, or certification-led outreach requires explicit procurement or program evidence.
- Never write passive analysis. Every paragraph must drive toward a commercial action.
- Reason hard: explicitly explain causality (what changed -> why it matters -> who is impacted -> what action follows).
- Include second-order effects and at least one counterfactual scenario ('if X worsens / if Y reverses, then ...').
- Name specific companies, people, facilities, certifications, dates, and dollar figures from the evidence.
- ALL target companies in recommendations MUST come from: (a) the entity being analyzed, (b) companies or people named in the evidence signals, (c) competitors listed in the entity's competitive profile. DO NOT invent or generalize target names.
- When a competitor has a weakness (delayed project, lost cert, supply problem), immediately name which of their customers from the evidence we should approach.
- For EU and North African entities: these are either competitors to monitor or commercial targets. State which — and why.
- FORBIDDEN phrases: 'continue monitoring', 'monitor the situation', 'remains to be seen', 'time will tell', 'various developments', 'warranting focused analysis', 'further developments', 'stay informed'
- NEVER use bracket placeholders like [Company X], [specific service], [date], [competitor weakness], [our services], etc. Use real names from the evidence and entity profile. If no specific contact is known, name the company + a realistic title.",
        our_profile = our_profile,
        competitor_mode_instruction = if entity_ctx.is_competitor {
            "⚠️ COMPETITOR ANALYSIS MODE: The entity you are analysing is a DIRECT EMS COMPETITOR — NOT a customer.\n\
STRICT RULES:\n\
1. NEVER recommend offering our services TO this competitor.\n\
2. Your job is to find their weaknesses, capability gaps, and customer pain points.\n\
3. Name which of THEIR customers we should approach RIGHT NOW because of this competitor's current problem.\n\
4. Frame every recommendation as 'approach [Customer X] — [why they're at risk from this competitor's problem]'.\n\
5. Any recommendation that addresses the competitor directly is WRONG — address their downstream customers instead."
        } else {
            "✅ CUSTOMER/PROSPECT MODE: This entity is a potential customer or partner. Recommend direct outreach, \
service proposals, qualification bids, and strategic partnership opportunities with this entity."
        },
        public_sector_mode_instruction = if is_public_sector {
            if has_public_sector_procurement_program {
                "🏛️ PUBLIC-SECTOR MODE: procurement or program evidence is present, so direct outreach is allowed only if it maps to the named tender, hardware/equipment need, or supplier qualification path in the evidence."
            } else {
                "🏛️ PUBLIC-SECTOR MODE: no explicit procurement or hardware program evidence is present. Do not turn this into a direct EMS sales pitch, qualification package, or manufacturing offer. Keep recommendations on policy impact, account exposure, stakeholder mapping, qualification planning, or scenario planning."
            }
        } else {
            ""
        },
        brief_outcome_instruction = if is_public_sector {
            "Every brief you write must lead to a specific account-planning, qualification, policy-response, or stakeholder action. Direct commercial outreach is allowed only when the evidence explicitly names a procurement, tender, supplier qualification event, or hardware/equipment program."
        } else {
            "Every brief you write must lead to a specific commercial action — a call to make, a bid to prepare, a competitor's customer to approach, or a market gap to fill."
        }
    );

    let user = format!(
        r#"Write a {category_label} brief about {entity_name}.

{entity_profile}

Evidence:
{evidence_text}

Analysis guidance: {analysis_focus}
Action guidance: {action_focus}
Strategic suggestion lanes to consider: {suggestion_axes}

Respond with valid JSON only:
{{
  "headline": "Action-oriented headline (<=140 chars) that names {entity_name} and the specific opportunity or threat",
        "narrative": "120-320 words of commercially-driven analysis in natural prose. Cite evidence as [1], [2], [3]. Explain what changed, why it matters, the causal chain, and which commercial choices are opened or constrained now. Do not use section labels or template headings.",
        "recommendation": "2-4 strategic suggestions in plain prose spanning at least two distinct lanes from {suggestion_axes}. Suggestions should be option-oriented rather than canned playbook text. Name REAL companies, roles, facilities, or programs from the evidence when possible, explain why each lane fits now, include timing when the evidence supports it, and cite at least one supporting evidence reference such as [1] or [2].",
  "confidence": 0.0
}}

MANDATORY:
- Reference at least 2 evidence items as [1], [2], etc.
- Recommendation text must also cite at least 1 supporting evidence item as [1], [2], etc.
- Include at least 3 concrete facts from the evidence (names, dates, numbers, places, standards).
- Include at least one explicit cause-effect statement (for example, 'because ... therefore ...').
- Include at least one counterfactual statement using 'if ... would/could ...'.
- Every recommendation target MUST be drawn from the evidence signals, entity profile, or competitive profile — never from your general knowledge. The entity being analyzed ({entity_name}) is always a valid target.
- Focus on what to DO, not what to observe.
- Treat any recurring action playbooks or operating patterns as suggestion sources, not scripts to be copied.
- READABILITY GATE: write plain business prose with complete sentences, no templates, no boilerplate labels, no heading prefixes like 'Assessment:' or 'Additional source reporting:'.
- USEFULNESS GATE: every sentence must add new information (fact, implication, or action); do not repeat the same claim with paraphrases.
"#,
        category_label = category_label,
        entity_name = entity_ctx.name,
        entity_profile = entity_profile,
        evidence_text = evidence_text,
        analysis_focus = analysis_focus,
        action_focus = action_focus,
        suggestion_axes = match (entity_ctx.is_competitor, is_public_sector, category) {
            (true, _, _) => "competitive displacement, customer rescue, qualification wedge, pricing wedge, regional footprint positioning, executive account planning",
            (_, true, "brand_sentiment") => "institutional credibility assessment, procurement scrutiny mapping, stakeholder messaging review, account dependency review, scenario planning, executive briefing",
            (_, true, "regulatory_policy") | (_, true, "geopolitical_analysis") => "policy impact mapping, qualification planning, account dependency review, stakeholder outreach, executive scenario planning, procurement path verification",
            (_, true, "security_compliance") | (_, true, "cybersecurity_threat") | (_, true, "quality_compliance") => "official-surface validation, supplier access review, containment planning, stakeholder brief, assurance planning, executive escalation",
            (_, true, _) => "account planning, stakeholder mapping, institutional process review, evidence verification, qualification planning, executive briefing",
            (_, false, "demand_procurement") | (_, false, "customer_rfq") => "revenue capture, qualification readiness, prototype or NPI entry, pricing leverage, regional footprint positioning, executive sponsor mapping",
            (_, false, "supply_chain_risk") => "continuity protection, dual-source qualification, customer assurance, design migration, regional rerouting, executive risk briefing",
            (_, false, "regulatory_policy") | (_, false, "geopolitical_analysis") => "regulatory posture, export-control routing, nearshoring, customer communication, qualification planning, executive scenario planning",
            (_, false, "strategic_poi") | (_, false, "talent_ip") => "POI mapping, early project engagement, partnership proposal, competitive positioning, executive outreach, stakeholder timing",
            (_, false, "security_compliance") | (_, false, "cybersecurity_threat") | (_, false, "quality_compliance") => "audit readiness, security assurance, supplier governance, containment planning, customer reassurance, executive escalation",
            _ => "revenue capture, resilience, competitive positioning, pricing leverage, regional expansion, executive planning",
        },
    );

    let config = InferenceConfig {
        temperature: 0.3,
        max_tokens: 3072,
        json_mode: true,
        suppress_thinking: false,
        timeout: std::time::Duration::from_secs(180),
        ..Default::default()
    };

    #[derive(serde::Deserialize)]
    struct LlmInsightResponse {
        headline: String,
        narrative: String,
        #[serde(default, deserialize_with = "deserialize_recommendation")]
        recommendation: Option<String>,
        confidence: f64,
    }

    /// Accept recommendation as string, array of strings, or array of objects.
    fn deserialize_recommendation<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::Deserialize;
        let val: Option<serde_json::Value> = Option::deserialize(deserializer)?;
        match val {
            Some(serde_json::Value::String(s)) => Ok(Some(s)),
            Some(serde_json::Value::Array(arr)) => {
                let items: Vec<String> = arr
                    .into_iter()
                    .filter_map(|v| match v {
                        serde_json::Value::String(s) => Some(s),
                        serde_json::Value::Object(map) => {
                            // Handle {"action":"...", "owner":"...", "deadline":"..."}
                            let parts: Vec<String> =
                                ["action", "owner", "deadline", "description", "task"]
                                    .iter()
                                    .filter_map(|key| {
                                        map.get(*key)
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .collect();
                            if parts.is_empty() {
                                // Fallback: join all string values
                                let all_strings: Vec<String> = map
                                    .values()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                                if all_strings.is_empty() {
                                    None
                                } else {
                                    Some(all_strings.join(" — "))
                                }
                            } else {
                                Some(parts.join(" — "))
                            }
                        }
                        _ => None,
                    })
                    .collect();
                if items.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(items.join(". ")))
                }
            }
            Some(serde_json::Value::Object(map)) => {
                // Single object recommendation
                let parts: Vec<String> = map
                    .values()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if parts.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(parts.join(" — ")))
                }
            }
            Some(serde_json::Value::Null) | None => Ok(None),
            _ => Ok(None),
        }
    }

    // Ban truly formulaic/filler phrases that indicate passive analysis.
    let generic_phrases: &[&str] = &[
        "continue monitoring",
        "monitor the situation",
        "various developments",
        "warranting focused analysis",
        "further developments",
        "remains to be seen",
        "time will tell",
        "developments warrant attention",
        "stay informed",
        "keep an eye on",
        "in conclusion",
        "it is important to note",
        "overall",
    ];

    let malformed_fragments: &[&str] = &[
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
        "[object object]",
        "undefined",
        "{{",
        "}}",
    ];

    for attempt in 1..=3 {
        let messages = vec![
            ChatMessage::system(system.as_str()),
            ChatMessage::user(&user),
        ];
        let resp = llm_client
            .complete_with_config(messages, &config)
            .await
            .with_context(|| format!("LLM insight generation failed for {}", entity_ctx.name))?;

        tracing::info!(
            entity = %entity_ctx.name,
            attempt,
            raw_len = resp.text.len(),
            raw_preview = %crate::truncate_text(&resp.text, 300),
            "LLM raw response"
        );

        let parsed: LlmInsightResponse = match resp.parse_json() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(entity = %entity_ctx.name, attempt, error = %e, "LLM JSON parse failed");
                continue;
            }
        };

        let recommendation = parsed.recommendation.unwrap_or_default().trim().to_string();
        let narrative = parsed.narrative.trim().to_string();
        let headline = parsed.headline.trim().to_string();

        let narrative_lower = narrative.to_lowercase();
        let recommendation_lower = recommendation.to_lowercase();
        let is_generic = generic_phrases
            .iter()
            .any(|p| narrative_lower.contains(p) || recommendation_lower.contains(p));

        let words = narrative.split_whitespace().count();
        let reference_count = count_numbered_references(&narrative, 12);
        let has_refs = reference_count >= 2;
        let recommendation_reference_count = count_numbered_references(&recommendation, 12);
        let recommendation_has_refs = recommendation_reference_count >= 1;
        let has_digits = narrative.chars().any(|c| c.is_ascii_digit());
        let has_recommendation = recommendation.split_whitespace().count() >= 12;
        let is_headline_ok = !headline.is_empty() && headline.len() <= 160;
        let has_causal_language = [
            "because",
            "therefore",
            "as a result",
            "which means",
            "implies",
            "drives",
            "leads to",
        ]
        .iter()
        .any(|p| narrative_lower.contains(p));
        let has_counterfactual = narrative_lower.contains("if ")
            && (narrative_lower.contains(" would ") || narrative_lower.contains(" could "));
        let has_reasoning_depth = has_causal_language || has_counterfactual;
        let recommendation_has_deadline = recommendation_lower.contains(" by ")
            || recommendation_lower.contains(" within ")
            || recommendation_lower.contains(" before ");
        let malformed = malformed_fragments.iter().any(|f| {
            headline.to_ascii_lowercase().contains(f)
                || narrative_lower.contains(f)
                || recommendation_lower.contains(f)
        });

        let readable_narrative = is_readable_and_useful_digest_text(&narrative)
            && !has_excessive_phrase_repetition(&recommendation);
        let readable_recommendation = recommendation
            .split(['.', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .count()
            >= 2;
        let unsupported_security_escalation = has_unsupported_security_escalation(
            category,
            &narrative,
            &recommendation,
            evidence_signals,
        );
        let unsupported_certification_escalation =
            has_unsupported_certification_escalation(&narrative, &recommendation, evidence_signals);
        let unsupported_public_sector_commercialization =
            has_unsupported_public_sector_commercialization(
                entity_ctx,
                category,
                &narrative,
                &recommendation,
                evidence_signals,
            );
        let low_usefulness_public_sector_analysis = has_low_usefulness_public_sector_analysis(
            entity_ctx,
            category,
            &headline,
            &narrative,
            &recommendation,
            evidence_signals,
        );
        let unnamed_customer_targeting =
            has_unnamed_customer_targeting(&narrative, &recommendation);

        // Reject if recommendation contains bracket placeholders like [Company X], [specific service], [date]
        let placeholder_patterns: &[&str] = &[
            "[company ",
            "[specific ",
            "[date]",
            "[competitor ",
            "[our ",
            "[their ",
            "[service",
            "[product",
            "[client ",
            "[customer ",
            "[contact ",
        ];
        let has_placeholders = placeholder_patterns
            .iter()
            .any(|p| recommendation_lower.contains(p));

        let passes = !is_generic
            && !malformed
            && !has_placeholders
            && words >= 85
            && has_refs
            && recommendation_has_refs
            && has_digits
            && has_recommendation
            && has_reasoning_depth
            && recommendation_has_deadline
            && readable_narrative
            && readable_recommendation
            && !unsupported_security_escalation
            && !unsupported_certification_escalation
            && !unsupported_public_sector_commercialization
            && !low_usefulness_public_sector_analysis
            && !unnamed_customer_targeting
            && is_headline_ok;

        if passes {
            return Ok((
                headline,
                narrative,
                recommendation,
                parsed.confidence.clamp(0.0, 1.0),
            ));
        }

        tracing::warn!(
            entity = %entity_ctx.name,
            attempt,
            words,
            reference_count,
            has_refs,
            recommendation_reference_count,
            recommendation_has_refs,
            has_digits,
            has_recommendation,
            has_reasoning_depth,
            recommendation_has_deadline,
            readable_narrative,
            readable_recommendation,
            unsupported_security_escalation,
            unsupported_certification_escalation,
            unsupported_public_sector_commercialization,
            low_usefulness_public_sector_analysis,
            unnamed_customer_targeting,
            recommendation_words = recommendation.split_whitespace().count(),
            recommendation_preview = %crate::truncate_text(&recommendation, 120),
            generic = is_generic,
            has_placeholders,
            malformed,
            headline = %headline,
            narrative_preview = %crate::truncate_text(&narrative, 180),
            "LLM quality gate: rejected"
        );
    }

    anyhow::bail!(
        "LLM failed quality checks after retries for {}",
        entity_ctx.name
    )
}

/// Build template-based fallback summary when LLM is unavailable.
fn build_fallback_summary(
    analytical_narrative: &str,
    rendered_action: &str,
    signal_details: &[String],
    evidence_urls: &[String],
    entity_label: &str,
    entity_region: &str,
    entity_type: Option<&str>,
    category: &str,
    severity: &str,
    confidence: f64,
    evidence_count: usize,
) -> String {
    let mut summary_parts: Vec<String> = Vec::new();
    let concrete_details = concrete_signal_details(signal_details);
    let concrete_signals = concrete_details.len();

    summary_parts.push(analytical_narrative.to_string());

    if !concrete_details.is_empty() {
        summary_parts.push(format!(
            "Key evidence: {}.",
            concrete_details
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    if concrete_signals > 0 {
        let suggestion_sentences = build_goal_oriented_suggestions(
            entity_label,
            entity_region,
            entity_type,
            category,
            signal_details,
            Some(rendered_action).filter(|s| !s.trim().is_empty()),
        );
        if !suggestion_sentences.is_empty() {
            summary_parts.push(suggestion_sentences.join(" "));
        }
    }

    let direct_actions = concrete_action_fragments(rendered_action);
    let suppress_direct_actions = is_public_sector_entity(entity_label, entity_type)
        && matches!(
            category,
            "regulatory_policy" | "geopolitical_analysis" | "brand_sentiment"
        );
    if !direct_actions.is_empty() && !suppress_direct_actions {
        summary_parts.push(format!(
            "Immediate next step: {}.",
            direct_actions.join(". ")
        ));
    }

    let sources_footer = format_sources_footer_from_urls(evidence_urls, 6);
    if !sources_footer.is_empty() {
        summary_parts.push(sources_footer);
    }

    let _ = (category, severity, confidence, evidence_count);

    summary_parts.join("\n\n")
}

fn push_entity_source_url(
    urls_by_entity: &mut HashMap<String, Vec<String>>,
    entity_id: Uuid,
    url: &str,
) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }

    let entry = urls_by_entity.entry(entity_id.to_string()).or_default();
    if entry
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        return;
    }

    entry.push(trimmed.to_string());
}

#[cfg(not(feature = "llm"))]
async fn collect_entity_evidence_urls(
    store: &PgStore,
    entity_ids: &[Uuid],
) -> HashMap<String, Vec<String>> {
    let unique_entity_ids: std::collections::HashSet<Uuid> = entity_ids.iter().copied().collect();
    let query_entity_ids: Vec<Uuid> = unique_entity_ids.iter().copied().collect();
    let mut urls_by_entity: HashMap<String, Vec<String>> = HashMap::new();

    if let Ok(rows) = store
        .get_warnings_by_entity_ids(&query_entity_ids, 500)
        .await
    {
        for warning in rows {
            let Some(entity_ids) = warning.entity_ids.as_ref() else {
                continue;
            };
            let Some(source_urls) = warning.source_urls.as_ref() else {
                continue;
            };
            for entity_id in entity_ids {
                for url in source_urls.iter().take(3) {
                    push_entity_source_url(&mut urls_by_entity, *entity_id, url);
                }
            }
        }
    }

    for entity_id in query_entity_ids {
        if let Ok(certs) = store.get_certifications_for_company(entity_id).await {
            for cert in certs.iter().take(8) {
                if let Some(url) = cert.evidence_url.as_deref() {
                    push_entity_source_url(&mut urls_by_entity, entity_id, url);
                }
            }
        }

        if let Ok(capabilities) = store.list_capabilities(Some(entity_id), 12, 0).await {
            for capability in capabilities.iter().take(8) {
                if let Some(urls) = capability.evidence_urls.as_ref() {
                    for url in urls.iter().take(2) {
                        push_entity_source_url(&mut urls_by_entity, entity_id, url);
                    }
                }
            }
        }

        if let Ok(observations) = store.get_observations_by_entity(entity_id, 10).await {
            for observation in observations {
                if let Some(url) = observation
                    .provenance
                    .as_object()
                    .and_then(|provenance| {
                        provenance
                            .get("source_url")
                            .or_else(|| provenance.get("url"))
                    })
                    .and_then(|value| value.as_str())
                {
                    push_entity_source_url(&mut urls_by_entity, entity_id, url);
                }
            }
        }
    }

    urls_by_entity
}

/// Parse a YAML signal string such as `"JobPost.role_family=Procurement.increase"` into a
/// [`SignalSpec`].  The expected format is `{ObsType}.{field[=value]}.{operator}`, where the
/// trailing segment is compared against the list of known operators; if it does not match,
/// `"contains"` is used as the default and the whole right-hand side is treated as the field.
fn parse_signal_str(s: &str) -> SignalSpec {
    const KNOWN_OPS: &[&str] = &[
        "increase", "decrease", "above", "below", "equals", "contains",
    ];
    let (obs_type, rest) = match s.find('.') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => {
            return SignalSpec {
                observation_type: s.to_string(),
                field: "count".to_string(),
                operator: "above".to_string(),
                threshold: Some(0.0),
                window_days: Some(30),
                value: None,
            }
        }
    };
    let (field_part, operator) = match rest.rfind('.') {
        Some(pos) => {
            let maybe_op = &rest[pos + 1..];
            if KNOWN_OPS.contains(&maybe_op) {
                (&rest[..pos], maybe_op.to_string())
            } else {
                (rest, "contains".to_string())
            }
        }
        None => (rest, "contains".to_string()),
    };
    let (field, value) = if let Some(eq) = field_part.find('=') {
        (
            field_part[..eq].to_string(),
            Some(field_part[eq + 1..].to_string()),
        )
    } else {
        (field_part.to_string(), None)
    };
    SignalSpec {
        observation_type: obs_type.to_string(),
        field: if field.is_empty() {
            "count".to_string()
        } else {
            field
        },
        operator,
        threshold: Some(0.0),
        window_days: Some(30),
        value,
    }
}

/// Convert a seed recipe definition into an engine-ready [`Recipe`].
fn seed_recipe_to_engine_recipe(sr: &apex_worker::recipe_loader::SeedRecipe) -> Recipe {
    let signals: Vec<SignalSpec> = sr
        .signals
        .iter()
        .filter_map(|sv| sv.as_str().map(parse_signal_str))
        .collect();
    let action = if sr.action_playbook.is_empty() {
        String::new()
    } else {
        sr.action_playbook.join("; ")
    };
    let severity = if sr.category.contains("security") || sr.category.contains("risk") {
        "warning"
    } else if sr.category.contains("supply") || sr.category.contains("sanction") {
        "warning"
    } else {
        "info"
    };
    let mut r = Recipe::new(sr.id.clone(), sr.name.clone());
    r.description = sr.category.clone();
    r.status = RecipeStatus::Seed;
    r.signals = signals;
    r.insight_template = sr.narrative_template.clone();
    r.action_template = action;
    r.severity = severity.to_string();
    r.category = sr.category.clone();
    r.min_uplift = 1.0;
    r.min_entities = 1;
    r
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let log_level = std::env::var("WORKER_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let log_filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| format!("{},apex_worker={}", log_level, log_level));
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::new(log_filter))
        .init();

    // B294: Validate config at startup — fail fast if env vars are misconfigured.
    // AppConfig::from_env() already checks for required fields (DATABASE_URL),
    // and validate() checks numeric ranges, URL formats, etc.
    let config = AppConfig::from_env()?;
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        tracing::error!(
            "Configuration validation failed ({} errors):",
            validation_errors.len()
        );
        for err in &validation_errors {
            tracing::error!("  - {}", err);
        }
        anyhow::bail!(
            "Refusing to start worker with invalid config. Fix the errors above and restart."
        );
    }
    tracing::info!("config validated successfully");

    // Create database pool for recipe insertion
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(config.database_url_value())
        .await?;
    tracing::info!("connected to database");

    // Load seed recipes from config/recipes_seed.yaml
    match load_default_seed_recipes() {
        Ok(recipes) => {
            if !recipes.is_empty() {
                let validation_errors = validate_seed_recipes(&recipes);
                if !validation_errors.is_empty() {
                    tracing::warn!(
                        "Recipe validation found {} issues (recipes will still be loaded)",
                        validation_errors.len()
                    );
                    for err in &validation_errors {
                        tracing::warn!("  - {}", err);
                    }
                }
                print_recipe_stats(&recipes);
                tracing::info!("loaded {} seed recipes from YAML", recipes.len());

                // Insert seed recipes into database
                match insert_seed_recipes(&pool, &recipes).await {
                    Ok(result) => {
                        tracing::info!(
                            "recipe insertion: {} inserted, {} skipped (already exist), {} errors",
                            result.inserted,
                            result.skipped,
                            result.errors.len()
                        );
                    }
                    Err(e) => {
                        tracing::error!("failed to insert seed recipes: {}", e);
                    }
                }
            } else {
                tracing::debug!("no seed recipes found");
            }
        }
        Err(e) => {
            tracing::warn!("failed to load seed recipes: {}", e);
        }
    }

    // Wrap pool in a shared PgStore so every job handler can query the DB
    // without creating its own connection pool.  Using from_pool() avoids
    // opening a second connection when the pool was already created above.
    let store = Arc::new(PgStore::from_pool(pool.clone()));
    tracing::info!("store initialized");

    let mut scheduler_state = default_scheduler();
    match store.list_worker_job_states().await {
        Ok(states) => {
            runtime::restore_scheduler_state(&mut scheduler_state, &states);
            tracing::info!(states = states.len(), "restored persisted scheduler state");
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to restore persisted scheduler state");
        }
    }

    let scheduler = Arc::new(TokioMutex::new(scheduler_state));
    let tick_guard = Arc::new(TokioMutex::new(()));
    let trigger_guard = Arc::new(TokioMutex::new(()));
    let manual_trigger_concurrency = std::env::var("MANUAL_TRIGGER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);
    let manual_max_claims_per_poll = std::env::var("MANUAL_TRIGGER_MAX_CLAIMS_PER_POLL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(manual_trigger_concurrency);
    let manual_trigger_timeout_secs = std::env::var("MANUAL_TRIGGER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2 * 60 * 60);
    let manual_trigger_semaphore = Arc::new(Semaphore::new(manual_trigger_concurrency));
    let jobs_count = scheduler.lock().await.jobs.len();
    tracing::info!(
        jobs = jobs_count,
        manual_trigger_concurrency,
        manual_max_claims_per_poll,
        manual_trigger_timeout_secs,
        "worker started"
    );

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut trigger_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let store = Arc::clone(&store);
                let scheduler = Arc::clone(&scheduler);
                let tick_guard = Arc::clone(&tick_guard);
                tokio::spawn(async move {
                    let Ok(_guard) = tick_guard.try_lock() else {
                        tracing::warn!("tick_scheduler: previous run still active; skipping tick");
                        return;
                    };

                    let mut scheduler = scheduler.lock().await;
                    tick_scheduler(&mut scheduler, &store).await;
                });
            }
            _ = trigger_interval.tick() => {
                let store = Arc::clone(&store);
                let trigger_guard = Arc::clone(&trigger_guard);
                let manual_trigger_semaphore = Arc::clone(&manual_trigger_semaphore);
                tokio::spawn(async move {
                    let Ok(_guard) = trigger_guard.try_lock() else {
                        tracing::debug!("poll_trigger_queue: previous poll still active; skipping tick");
                        return;
                    };

                    poll_trigger_queue(
                        &store,
                        &manual_trigger_semaphore,
                        manual_max_claims_per_poll,
                        manual_trigger_timeout_secs,
                    )
                    .await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received shutdown signal, exiting gracefully");
                break;
            }
        }
    }
    Ok(())
}

#[tracing::instrument(skip(scheduler, store))]
async fn tick_scheduler(scheduler: &mut Scheduler, store: &Arc<PgStore>) {
    runtime::tick_scheduler(scheduler, store).await;
}

fn parse_digest_recipients(raw: &str) -> Vec<String> {
    raw.split(|c| c == ',' || c == ';' || c == '\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_hhmm(raw: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = raw.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour = parts[0].parse::<u32>().ok()?;
    let minute = parts[1].parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

fn weekday_matches(weekday: &str, now_weekday: chrono::Weekday) -> bool {
    match weekday {
        "Mon" => now_weekday == chrono::Weekday::Mon,
        "Tue" => now_weekday == chrono::Weekday::Tue,
        "Wed" => now_weekday == chrono::Weekday::Wed,
        "Thu" => now_weekday == chrono::Weekday::Thu,
        "Fri" => now_weekday == chrono::Weekday::Fri,
        "Sat" => now_weekday == chrono::Weekday::Sat,
        "Sun" => now_weekday == chrono::Weekday::Sun,
        _ => false,
    }
}

fn is_digest_due(
    now_cet: chrono::DateTime<chrono_tz::Tz>,
    prefs: &apex_store::postgres::UserSettingsPrefs,
) -> bool {
    if !prefs.email_digest_enabled {
        return false;
    }
    let Some((target_hour, target_minute)) = parse_hhmm(&prefs.email_digest_time_cet) else {
        return false;
    };
    if now_cet.hour() < target_hour
        || (now_cet.hour() == target_hour && now_cet.minute() < target_minute)
    {
        return false;
    }

    let last_sent_cet = prefs
        .email_digest_last_sent_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Berlin));

    if prefs.notification_frequency.eq_ignore_ascii_case("weekly") {
        if !weekday_matches(&prefs.email_digest_weekday, now_cet.weekday()) {
            return false;
        }
        if let Some(last) = last_sent_cet {
            let now_week = now_cet.iso_week();
            let last_week = last.iso_week();
            if now_week.year() == last_week.year() && now_week.week() == last_week.week() {
                return false;
            }
        }
        true
    } else {
        if let Some(last) = last_sent_cet {
            if last.date_naive() == now_cet.date_naive() {
                return false;
            }
        }
        true
    }
}

fn build_digest_html(
    base_url: &str,
    insights: &[apex_store::postgres::InsightRow],
    category_label: &str,
) -> String {
    let mut cards = String::new();
    for insight in insights {
        let id = insight.id;
        let confidence = ((insight.confidence.unwrap_or(0.0) * 100.0).round() as i64).clamp(0, 100);
        let category = insight
            .insight_type
            .clone()
            .unwrap_or_else(|| "general".to_string())
            .replace('_', " ");
        let updated = insight
            .updated_at
            .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "n/a".to_string());
        let summary = insight
            .summary
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br/>");
        let title = insight.title.replace('<', "&lt;").replace('>', "&gt;");
        cards.push_str(&format!(
                        "<div style=\"margin:0 0 14px;padding:14px 16px 12px;border:1px solid #BCBCBC;border-radius:2px;background:#FFFFFF;box-shadow:inset 0 1px 0 rgba(255,255,255,.75);\">\
                         <div style=\"margin:0 0 6px;font-size:11px;line-height:1.35;color:#606060;font-weight:700;text-transform:uppercase;letter-spacing:.08em;\">{category} • {confidence}% confidence</div>\
                         <a href=\"{base_url}/insights/{id}\" style=\"display:block;margin:0 0 9px;color:#101010;font-size:17px;line-height:1.35;font-weight:700;text-decoration:none;\">{title}</a>\
                         <p style=\"margin:0 0 10px;color:#303030;font-size:14px;line-height:1.55;\">{summary}</p>\
                         <a href=\"{base_url}/insights/{id}\" style=\"display:inline-block;padding:8px 12px;border-radius:4px;background:#FFBE00;color:#121212;font-size:12px;font-weight:700;text-decoration:none;\">Open insight</a>\
                         <span style=\"float:right;padding-top:8px;color:#606060;font-size:12px;\">Updated: {updated}</span>\
                         </div>"
        ));
    }

    let generated_at = Utc::now().format("%Y-%m-%d %H:%M UTC");

    format!(
                "<!doctype html><html><body style=\"margin:0;padding:0;background:#F2F2F2;font-family:Inter,Segoe UI,Arial,sans-serif;color:#101010;\">\
                     <div style=\"max-width:760px;margin:24px auto;padding:0 12px;\">\
                         <div style=\"background:#101010;padding:18px 20px;border-radius:4px 4px 0 0;\">\
                             <div style=\"font-size:11px;letter-spacing:.12em;text-transform:uppercase;font-weight:800;color:#FFBE00;\">ApexIntel</div>\
                             <h1 style=\"margin:6px 0 0;font-size:24px;line-height:1.2;color:#F4F6FA;\">Top Insights Digest</h1>\
                             <p style=\"margin:8px 0 0;color:#B6BCC7;font-size:13px;line-height:1.45;\">Generated {generated_at} • Categories: {category_label} • {insights_len} insights</p>\
                         </div>\
                         <div style=\"background:#FFFFFF;padding:16px;border:1px solid #BCBCBC;border-top:none;border-radius:0 0 4px 4px;\">\
                             {cards}\
                             <div style=\"margin-top:12px;padding-top:10px;border-top:1px solid #D9D9D9;\">\
                                 <a href=\"{base_url}/insights\" style=\"display:inline-block;padding:10px 14px;border-radius:4px;background:#111111;color:#F3F4F8;font-size:12px;font-weight:700;text-decoration:none;\">View all insights</a>\
                             </div>\
                         </div>\
                     </div>\
                 </body></html>",
                insights_len = insights.len()
    )
}

fn build_digest_text(
    base_url: &str,
    insights: &[apex_store::postgres::InsightRow],
    category_label: &str,
) -> String {
    let mut out = format!(
        "ApexIntel Top Insights Digest\nCategories: {}\n\n",
        category_label
    );
    for (idx, insight) in insights.iter().enumerate() {
        let confidence = ((insight.confidence.unwrap_or(0.0) * 100.0).round() as i64).clamp(0, 100);
        out.push_str(&format!(
            "{}. {} ({}%)\n{}\n{}/insights/{}\n\n",
            idx + 1,
            insight.title,
            confidence,
            insight.summary,
            base_url,
            insight.id
        ));
    }
    out
}

fn expand_digest_categories(categories: &[String]) -> Vec<String> {
    fn push_unique(out: &mut Vec<String>, value: &str) {
        if !out.iter().any(|v| v == value) {
            out.push(value.to_string());
        }
    }

    let mut out = Vec::new();
    for category in categories {
        match category.trim().to_ascii_lowercase().as_str() {
            "demand_signal" => {
                push_unique(&mut out, "demand_procurement");
                push_unique(&mut out, "customer_rfq");
                push_unique(&mut out, "pricing_market");
            }
            "supply_risk" => {
                push_unique(&mut out, "supply_chain_risk");
                push_unique(&mut out, "quality_compliance");
            }
            "competitive_intel" => {
                push_unique(&mut out, "competitor_market");
                push_unique(&mut out, "ma_partnerships");
                push_unique(&mut out, "market_expansion");
            }
            "security_posture" => {
                push_unique(&mut out, "cybersecurity_threat");
                push_unique(&mut out, "security_compliance");
            }
            "macro_shift" => {
                push_unique(&mut out, "geopolitical_analysis");
                push_unique(&mut out, "regulatory_policy");
                push_unique(&mut out, "brand_sentiment");
            }
            "poi_movement" => {
                push_unique(&mut out, "strategic_poi");
                push_unique(&mut out, "talent_ip");
            }
            // Backward compatible passthrough for direct insight_type values.
            other if !other.is_empty() => push_unique(&mut out, other),
            _ => {}
        }
    }
    out
}

fn canonical_digest_key(raw: &str, max_words: usize) -> String {
    let normalized = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>();

    normalized
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ")
}

fn digest_tokens(raw: &str, max_tokens: usize) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "into", "over", "under", "into",
        "onto", "after", "before", "about", "their", "there", "they", "them", "were", "have",
        "has", "been", "being", "will", "would", "could", "should", "a", "an", "of", "to", "in",
        "on", "by", "at", "as", "is", "are", "or",
    ];

    raw.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .take(max_tokens)
        .map(|w| w.to_string())
        .collect()
}

fn token_jaccard_similarity(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_set: std::collections::HashSet<&str> = left.iter().map(String::as_str).collect();
    let right_set: std::collections::HashSet<&str> = right.iter().map(String::as_str).collect();
    jaccard_similarity(&left_set, &right_set)
}

fn has_excessive_phrase_repetition(text: &str) -> bool {
    let tokens = digest_tokens(text, 220);
    if tokens.len() < 10 {
        return false;
    }

    // Flag repeated 4-token windows that indicate templated/mangled outputs.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for window in tokens.windows(4) {
        let phrase = window.join(" ");
        let entry = counts.entry(phrase).or_insert(0);
        *entry += 1;
        if *entry >= 3 {
            return true;
        }
    }
    false
}

fn is_readable_and_useful_digest_text(summary: &str) -> bool {
    if has_excessive_phrase_repetition(summary) {
        return false;
    }

    let sentence_count = summary
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .count();
    if sentence_count == 0 || sentence_count > 8 {
        return false;
    }

    let tokens = digest_tokens(summary, 240);
    if tokens.len() < 12 {
        return false;
    }
    let unique: std::collections::HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let unique_ratio = unique.len() as f64 / tokens.len() as f64;
    if unique_ratio < 0.48 {
        return false;
    }

    let lower = summary.to_ascii_lowercase();
    let usefulness_markers = [
        "impact",
        "risk",
        "opportunity",
        "because",
        "therefore",
        "drives",
        "leads to",
        "supply",
        "pricing",
        "compliance",
        "customer",
        "action",
    ];
    usefulness_markers
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_internal_insight_type(insight_type: Option<&str>) -> bool {
    insight_type
        .map(|insight_type| {
            let normalized = insight_type.trim().to_ascii_lowercase();
            normalized.starts_with("llm_")
                || matches!(
                    normalized.as_str(),
                    "llm_eval_report" | "llm_self_improvement"
                )
        })
        .unwrap_or(false)
}

fn count_template_markers(text: &str) -> usize {
    const TEMPLATE_MARKERS: &[&str] = &[
        "assessment:",
        "recommended action:",
        "additional source reporting:",
        "signal themes detected:",
        "actionable:",
        "watch closely:",
        "early signal:",
        "low confidence:",
        "analysis:",
        "impact:",
        "recommendation:",
    ];

    let lower = text.to_ascii_lowercase();
    TEMPLATE_MARKERS
        .iter()
        .filter(|marker| lower.contains(**marker))
        .count()
}

fn passes_shared_insight_quality_gate(
    title: &str,
    summary: &str,
    insight_type: Option<&str>,
) -> bool {
    if is_internal_insight_type(insight_type) {
        return true;
    }

    if title.trim().len() < 12 || summary.trim().len() < 80 {
        return false;
    }
    if is_low_quality_narrative(summary) {
        return false;
    }
    if has_excessive_phrase_repetition(summary) {
        return false;
    }

    let lower_title = title.to_ascii_lowercase();
    let lower_summary = summary.to_ascii_lowercase();
    let malformed_fragments = [
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
        "[object object]",
        "undefined",
        "{{",
        "}}",
    ];
    if malformed_fragments
        .iter()
        .any(|fragment| lower_title.contains(fragment) || lower_summary.contains(fragment))
    {
        return false;
    }

    if count_template_markers(summary) >= 2 {
        return false;
    }

    let generic_fallback_markers = [
        "if the priority is ",
        "if the goal is ",
        "this deserves action inside the current planning cycle",
        "overall confidence is ",
        "grounded in ",
    ];
    let generic_fallback_count = generic_fallback_markers
        .iter()
        .filter(|marker| lower_summary.contains(**marker))
        .count();

    let fallback_signal_details = extract_fallback_signal_details(summary);
    if lower_summary.contains("our monitoring detected:")
        && lower_summary.contains("has been flagged for")
        && !fallback_signal_details.is_empty()
        && count_concrete_signal_details(&fallback_signal_details) == 0
    {
        return false;
    }

    if lower_summary.contains("our monitoring detected:")
        && lower_summary.contains("has been flagged for")
        && generic_fallback_count >= 2
    {
        return false;
    }

    if matches!(insight_type, Some("veracity_analysis")) {
        return is_readable_and_useful_digest_text(summary)
            && lower_summary.contains("source")
            && (lower_summary.contains("evidence")
                || lower_summary.contains("corroborat")
                || lower_summary.contains("reported"));
    }

    is_readable_and_useful_digest_text(summary)
}

fn is_digest_insight_quality(title: &str, summary: &str) -> bool {
    if title.len() < 12 || summary.len() < 40 {
        return false;
    }
    if is_low_quality_narrative(summary) {
        return false;
    }
    if !is_readable_and_useful_digest_text(summary) {
        return false;
    }

    // Guard against placeholder-like and broken outputs.
    let bad_fragments = [
        "{{",
        "}}",
        "[object object]",
        "undefined",
        "null null",
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
    ];
    let t = title.to_ascii_lowercase();
    let s = summary.to_ascii_lowercase();
    !bad_fragments.iter().any(|f| t.contains(f) || s.contains(f))
}

async fn send_digest_email(
    recipients: &[String],
    subject: &str,
    html_body: String,
    text_body: String,
) -> Result<()> {
    let from_address = "contact@apexmediation.ee";
    let smtp_host = std::env::var("EMAIL_DIGEST_SMTP_HOST")
        .unwrap_or_else(|_| "mail.apexmediation.ee".to_string());
    let smtp_port: u16 = std::env::var("EMAIL_DIGEST_SMTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(25);
    let smtp_user = std::env::var("EMAIL_DIGEST_SMTP_USER").unwrap_or_default();
    let smtp_pass = std::env::var("EMAIL_DIGEST_SMTP_PASS").unwrap_or_default();
    let smtp_starttls = std::env::var("EMAIL_DIGEST_SMTP_STARTTLS")
        .ok()
        .map(|v| parse_truthy_flag(&v))
        .unwrap_or(false);

    let mut builder = Message::builder()
        .from(from_address.parse::<Mailbox>()?)
        .subject(subject);
    for to in recipients {
        builder = builder.to(to.parse::<Mailbox>()?);
    }

    let email = builder.multipart(
        MultiPart::alternative()
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(text_body),
            )
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(html_body),
            ),
    )?;

    let mailer = if smtp_starttls {
        // Submission on 587 expects STARTTLS upgrade, not implicit TLS.
        let mut transport =
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)?.port(smtp_port);
        if !smtp_user.trim().is_empty() {
            transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
        }
        transport.build()
    } else {
        let mut transport =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp_host).port(smtp_port);
        if !smtp_user.trim().is_empty() {
            transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
        }
        transport.build()
    };

    mailer.send(email).await?;
    Ok(())
}

async fn run_update_email_digest_job(store: &Arc<PgStore>) -> Result<(u64, u64)> {
    let subscribers = store.list_user_settings_prefs_for_email_digest().await?;
    if subscribers.is_empty() {
        return Ok((0, 0));
    }

    let now_utc = Utc::now();
    let now_cet = now_utc.with_timezone(&Berlin);
    let base_url = std::env::var("EMAIL_DIGEST_BASE_URL")
        .unwrap_or_else(|_| "https://starzerp.fi".to_string())
        .trim_end_matches('/')
        .to_string();

    let mut sent_count: u64 = 0;
    let mut users_due: u64 = 0;

    for (user_id, prefs) in subscribers {
        if !is_digest_due(now_cet, &prefs) {
            continue;
        }
        users_due += 1;

        let recipients = parse_digest_recipients(&prefs.email_digest_recipients);
        if recipients.is_empty() {
            tracing::warn!(user_id = %user_id, "email digest enabled but recipients empty");
            continue;
        }

        let since = if prefs.notification_frequency.eq_ignore_ascii_case("weekly") {
            now_utc - chrono::Duration::days(7)
        } else {
            now_utc - chrono::Duration::days(1)
        };

        let mut filters = InsightListFilters {
            date_from: Some(since),
            ..Default::default()
        };
        let mapped_insight_types = expand_digest_categories(&prefs.email_digest_categories);
        if !mapped_insight_types.is_empty() {
            filters.insight_types = mapped_insight_types;
        }

        let mut rows = store.list_insights(&filters, 200, 0).await?;
        rows.retain(|r| {
            !r.insight_type
                .as_deref()
                .map(|t| t.trim().to_ascii_lowercase().starts_with("llm_"))
                .unwrap_or(false)
        });
        if prefs.critical_only_enabled {
            rows.retain(|r| r.confidence.unwrap_or(0.0) >= 0.7);
        }
        rows.sort_by(|a, b| {
            b.confidence
                .unwrap_or(0.0)
                .total_cmp(&a.confidence.unwrap_or(0.0))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        let mut curated = Vec::new();
        let mut seen_title_keys: Vec<String> = Vec::new();
        let mut seen_summary_keys: Vec<String> = Vec::new();
        let mut seen_signature_tokens: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        for mut row in rows {
            row.title = clean_rendered_text(&row.title);
            row.summary = clean_rendered_text(&row.summary);

            if !is_digest_insight_quality(&row.title, &row.summary) {
                continue;
            }

            let title_key = canonical_digest_key(&row.title, 10);
            if title_key.is_empty() {
                continue;
            }
            if seen_title_keys.iter().any(|k| k == &title_key) {
                continue;
            }

            let summary_key = canonical_digest_key(&row.summary, 14);
            if !summary_key.is_empty() && seen_summary_keys.iter().any(|k| k == &summary_key) {
                continue;
            }

            // Allow multiple updates per company, but suppress near-duplicate variants.
            let title_tokens = digest_tokens(&row.title, 20);
            let summary_tokens = digest_tokens(&row.summary, 80);
            let near_duplicate = seen_signature_tokens
                .iter()
                .any(|(seen_title, seen_summary)| {
                    let title_sim = token_jaccard_similarity(&title_tokens, seen_title);
                    let summary_sim = token_jaccard_similarity(&summary_tokens, seen_summary);
                    title_sim >= 0.78 && summary_sim >= 0.72
                });
            if near_duplicate {
                continue;
            }

            seen_title_keys.push(title_key);
            if !summary_key.is_empty() {
                seen_summary_keys.push(summary_key);
            }
            seen_signature_tokens.push((title_tokens, summary_tokens));
            curated.push(row);
        }

        let category_label = if prefs.email_digest_categories.is_empty() {
            "All".to_string()
        } else {
            prefs.email_digest_categories.join(", ")
        };
        let top: Vec<_> = curated.into_iter().take(8).collect();
        if top.is_empty() {
            tracing::info!(user_id = %user_id, "digest due but no matching insights");
            continue;
        }

        let subject = format!(
            "ApexIntel Update: {} top insights ({})",
            top.len(),
            now_cet.format("%Y-%m-%d")
        );
        let html = build_digest_html(&base_url, &top, &category_label);
        let text = build_digest_text(&base_url, &top, &category_label);

        send_digest_email(&recipients, &subject, html, text).await?;
        store.mark_email_digest_sent(&user_id, now_utc).await?;
        sent_count += 1;
        tracing::info!(user_id = %user_id, recipients = recipients.len(), insights = top.len(), "email digest sent");
    }

    Ok((sent_count, users_due))
}

async fn poll_trigger_queue(
    store: &Arc<PgStore>,
    manual_trigger_semaphore: &Arc<Semaphore>,
    max_claims_per_poll: usize,
    manual_trigger_timeout_secs: i64,
) {
    runtime::poll_trigger_queue(
        store,
        manual_trigger_semaphore,
        max_claims_per_poll,
        manual_trigger_timeout_secs,
    )
    .await;
}

#[allow(dead_code)]
#[tracing::instrument(skip(kind, store), fields(job = %kind.as_str()))]
async fn execute_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    job_execution::execute_job(kind, store).await
}

// ────────────────────────────────────────────────────────────────────────────
// LLM-based POI validation
// ────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
fn discovery_method_priority(method: &str) -> u8 {
    match method {
        "org_leadership" | "org_leadership_fallback" => 3,
        "opencorporates_officer" => 2,
        "gov_directory" => 2,
        "gdelt_co_mention" => 0,
        _ => 1,
    }
}

#[cfg(feature = "llm")]
fn looks_like_person_name(name: &str) -> bool {
    let cleaned = name.trim();
    if cleaned.is_empty() || cleaned.len() > 80 {
        return false;
    }
    if !apex_core::person_names::looks_like_person_name(cleaned) {
        return false;
    }
    if cleaned.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }

    let blocked_terms = [
        "holdings",
        "limited",
        "ltd",
        "inc",
        "corp",
        "group",
        "reports",
        "transcript",
        "revenue",
        "demand",
        "guidance",
        "quarter",
        "deep dive",
        "boost",
        "street",
        "expectations",
        "call",
        "earnings",
        "meet",
        "cloud",
    ];
    let lower = cleaned.to_lowercase();
    if blocked_terms.iter().any(|t| lower.contains(t)) {
        return false;
    }

    let parts: Vec<&str> = cleaned
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphabetic() && c != '-' && c != '\''))
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }

    parts.iter().all(|p| {
        let mut chars = p.chars();
        match chars.next() {
            Some(first) if first.is_uppercase() => {
                chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
            }
            _ => false,
        }
    })
}

#[cfg(feature = "llm")]
fn company_name_matches_seed(inferred_org: &str, seed_org: &str) -> bool {
    let inferred = normalize_company_name(inferred_org);
    let seed = normalize_company_name(seed_org);
    !inferred.is_empty() && inferred == seed
}

#[cfg(feature = "llm")]
fn is_public_email_domain(domain: &str) -> bool {
    matches!(
        domain,
        "gmail.com"
            | "googlemail.com"
            | "outlook.com"
            | "hotmail.com"
            | "live.com"
            | "yahoo.com"
            | "icloud.com"
            | "aol.com"
            | "proton.me"
            | "protonmail.com"
    )
}

#[cfg(feature = "llm")]
fn normalize_company_domain(domain: &str) -> Option<String> {
    let normalized = domain
        .trim()
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty() || !normalized.contains('.') {
        return None;
    }
    Some(normalized)
}

#[cfg(feature = "llm")]
fn extract_candidate_company_domain(disc: &DiscoveredPoi) -> Option<String> {
    if let Some(email) = disc.contact_email.as_deref() {
        if let Some((_, domain)) = email.rsplit_once('@') {
            let normalized = normalize_company_domain(domain)?;
            if !is_public_email_domain(&normalized) {
                return Some(normalized);
            }
        }
    }

    if matches!(
        disc.discovery_method.as_str(),
        "org_leadership" | "org_leadership_fallback"
    ) {
        if let Ok(url) = reqwest::Url::parse(&disc.source_url) {
            if let Some(host) = url.host_str() {
                return normalize_company_domain(host);
            }
        }
    }

    None
}

#[cfg(feature = "llm")]
async fn resolve_discovered_company_id(
    store: &PgStore,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    now: chrono::DateTime<Utc>,
) -> Result<Option<Uuid>> {
    let inferred_org = disc
        .inferred_org
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let (Some(seed), Some(org_name)) = (parent_seed, inferred_org) {
        if company_name_matches_seed(org_name, &seed.org_name) {
            return Ok(seed.primary_org_id);
        }
    }

    let inferred_domain = extract_candidate_company_domain(disc);
    if let Some(domain) = inferred_domain.as_deref() {
        if let Some(existing) = store.get_company_by_domain(domain).await? {
            return Ok(Some(existing.id));
        }
    }

    if let Some(org_name) = inferred_org {
        if let Some(existing) = store.get_company_by_name_ci(org_name).await? {
            return Ok(Some(existing.id));
        }

        let mut company = Company::new(
            org_name.to_string(),
            CompanyType::Other("poi_discovered".to_string()),
        );
        company.domain = inferred_domain;
        company.metadata = serde_json::json!({
            "discovered_via": "poi_discovery",
            "discovery_method": disc.discovery_method,
            "source_url": disc.source_url,
            "seed_person_id": disc.seed_person_id,
            "confidence": disc.confidence,
            "seed_org_name": parent_seed.map(|seed| seed.org_name.as_str()),
            "seed_org_id": parent_seed.and_then(|seed| seed.primary_org_id).map(|id| id.to_string()),
            "seed_is_competitor": parent_seed.map(|seed| seed.is_competitor).unwrap_or(false),
        });
        company.created_at = now;
        company.updated_at = now;
        store.insert_company(&company).await?;
        tracing::info!(
            company = %company.name,
            domain = ?company.domain,
            method = %disc.discovery_method,
            "poi_discovery: inserted new company"
        );
        return Ok(Some(company.id));
    }

    Ok(parent_seed.and_then(|seed| seed.primary_org_id))
}

/// Classify an inferred role title into a canonical `RoleFamily`.
#[cfg(feature = "llm")]
fn classify_role_family(role: Option<&str>) -> RoleFamily {
    let r = match role {
        Some(s) if !s.is_empty() => s.to_lowercase(),
        _ => return RoleFamily::Other("Unknown".to_string()),
    };
    // C-suite / executive
    if r.contains("ceo")
        || r.contains("chief executive")
        || r.contains("chairman")
        || r.contains("chairwoman")
        || r.contains("president")
    {
        return RoleFamily::Executive;
    }
    if r.contains("cfo")
        || r.contains("chief financial")
        || r.contains("treasurer")
        || r.contains("controller")
        || r.contains("comptroller")
    {
        return RoleFamily::Finance;
    }
    if r.contains("cto") || r.contains("chief technology") || r.contains("chief information") {
        return RoleFamily::Engineering;
    }
    if r.contains("coo") || r.contains("chief operating") || r.contains("chief supply") {
        return RoleFamily::Operations;
    }
    if r.contains("ciso") || r.contains("chief security") {
        return RoleFamily::Security;
    }
    if r.contains("chief")
        || r.contains("director")
        || r.contains("board")
        || r.contains("executive vice")
        || r.contains("senior vice")
    {
        return RoleFamily::Executive;
    }
    // VP-level roles — classify by functional area
    if r.contains("vp") || r.contains("vice president") {
        if r.contains("finance") || r.contains("financial") {
            return RoleFamily::Finance;
        }
        if r.contains("engineer") || r.contains("technology") || r.contains("r&d") {
            return RoleFamily::Engineering;
        }
        if r.contains("operation") || r.contains("supply chain") || r.contains("manufacturing") {
            return RoleFamily::Operations;
        }
        if r.contains("procurement") || r.contains("sourcing") {
            return RoleFamily::Procurement;
        }
        if r.contains("quality") {
            return RoleFamily::Quality;
        }
        if r.contains("security") {
            return RoleFamily::Security;
        }
        if r.contains("legal") || r.contains("counsel") {
            return RoleFamily::Legal;
        }
        if r.contains("logistics") {
            return RoleFamily::Logistics;
        }
        return RoleFamily::Executive;
    }
    // Government
    if r.contains("minister")
        || r.contains("secretary")
        || r.contains("governor")
        || r.contains("commissioner")
        || r.contains("ambassador")
    {
        return RoleFamily::Government;
    }
    // Military
    if r.contains("general")
        || r.contains("admiral")
        || r.contains("colonel")
        || r.contains("military")
        || r.contains("commander")
    {
        return RoleFamily::Military;
    }
    // Functional keywords
    if r.contains("procurement") || r.contains("sourcing") || r.contains("purchasing") {
        return RoleFamily::Procurement;
    }
    if r.contains("quality") {
        return RoleFamily::Quality;
    }
    if r.contains("engineer") || r.contains("architect") {
        return RoleFamily::Engineering;
    }
    if r.contains("operation") || r.contains("manufacturing") || r.contains("plant manager") {
        return RoleFamily::Operations;
    }
    if r.contains("finance") || r.contains("accounting") || r.contains("audit") {
        return RoleFamily::Finance;
    }
    if r.contains("legal") || r.contains("counsel") || r.contains("compliance") {
        return RoleFamily::Legal;
    }
    if r.contains("security") || r.contains("cyber") {
        return RoleFamily::Security;
    }
    if r.contains("logistics") || r.contains("warehouse") || r.contains("shipping") {
        return RoleFamily::Logistics;
    }
    RoleFamily::Other(role.unwrap_or("Unknown").to_string())
}

/// Validates that a discovered POI candidate is a real person name (not a topic,
/// navigation element, or garbage text). Returns `Some(validated)` with potentially
/// enriched role/org if valid, or `None` if not a real person.
#[cfg(feature = "llm")]
async fn validate_person_via_llm(
    llm: &OpenAiCompatibleClient,
    candidate: &DiscoveredPoi,
) -> anyhow::Result<Option<DiscoveredPoi>> {
    const SYSTEM_PROMPT: &str = r#"You are a strict POI validator for strategic intelligence.
Your task has TWO gates:
1) Is this a real person?
2) Is this person a TARGET middle-management decision-maker profile?

Target profiles:
- Government: deputy/director-general, department directors, program/policy directors/managers,
  procurement/acquisition officials, licensing/regulatory/compliance leaders in ministries/agencies.
- Company: procurement/purchasing/sourcing/category/commodity managers and directors,
  operations/supply-chain/quality/engineering/legal/compliance decision managers.

Non-target profiles (reject):
- C-level/board/top-most leadership (CEO/CFO/CTO/COO/chairman/president/founder)
- Ceremonial or political top offices (minister, governor, ambassador, senator)
- Generic text, concepts, product labels, navigation strings.

Respond ONLY valid JSON:
{"is_person":true/false,"target_fit":true/false,"name":"...","role":"...|null","org":"...|null","seniority_band":"mid|senior_mid|unknown"}
If invalid/non-target: {"is_person":false,"target_fit":false}"#;

    let user_prompt = format!(
        "Candidate name: \"{}\"\nSource: {}\nDiscovery method: {}\nInferred role: {}\nInferred org: {}",
        candidate.name,
        candidate.source_url,
        candidate.discovery_method,
        candidate.inferred_role.as_deref().unwrap_or("unknown"),
        candidate.inferred_org.as_deref().unwrap_or("unknown"),
    );

    let response = llm.generate_json(SYSTEM_PROMPT, &user_prompt).await?;

    // Parse LLM response
    #[derive(serde::Deserialize)]
    struct LlmResponse {
        is_person: bool,
        #[serde(default)]
        target_fit: Option<bool>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        org: Option<String>,
    }

    let parsed: LlmResponse = match serde_json::from_str(&response) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                candidate = %candidate.name,
                response = %response,
                error = %e,
                "poi_validation: failed to parse LLM response, rejecting candidate"
            );
            return Ok(None);
        }
    };

    if !parsed.is_person || parsed.target_fit.unwrap_or(false) == false {
        return Ok(None);
    }

    // Return validated/enriched candidate
    let mut validated = candidate.clone();
    if let Some(name) = parsed.name {
        if !name.is_empty() {
            validated.name = name;
        }
    }
    if parsed.role.is_some() {
        validated.inferred_role = parsed.role;
    }
    if parsed.org.is_some() {
        validated.inferred_org = parsed.org;
    }

    Ok(Some(validated))
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct LlmContinuousImprovementStats {
    eval_pass_rate: f64,
    eval_avg_score: f64,
    eval_hallucination_rate: f64,
    captures_seeded: usize,
    captures_analysed: usize,
    qualifying_examples: usize,
    avg_critique_score: f64,
}

#[cfg(feature = "llm")]
fn build_quality_llm_client() -> Arc<dyn LlmClient> {
    let mut llm_config = ModelConfig::llamacpp_lightweight();
    if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
        llm_config.base_url = base_url;
    }
    if let Ok(model) = std::env::var("LLM_MODEL") {
        if !model.trim().is_empty() {
            llm_config.model_name = model;
        }
    }
    llm_config.api_key = std::env::var("LLM_API_KEY").ok();
    Arc::new(OpenAiCompatibleClient::new(llm_config))
}

#[cfg(feature = "llm")]
async fn run_llm_continuous_improvement_cycle(
    store: &PgStore,
) -> anyhow::Result<LlmContinuousImprovementStats> {
    let llm = build_quality_llm_client();

    let min_eval_pass_rate = std::env::var("LLM_SELF_IMPROVEMENT_MIN_PASS_RATE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.75)
        .clamp(0.0, 1.0);
    let min_eval_score = std::env::var("LLM_SELF_IMPROVEMENT_MIN_SCORE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.62)
        .clamp(0.0, 1.0);
    let max_hallucination_rate = std::env::var("LLM_SELF_IMPROVEMENT_MAX_HALLUCINATION_RATE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.30)
        .clamp(0.0, 1.0);

    let eval_runner = EvalRunner::new(llm.clone(), llm.clone());
    let eval_suite = standard_eval_suite();
    let eval_report = eval_runner
        .run(&eval_suite)
        .await
        .context("llm self-improvement: standard eval suite failed")?;

    let eval_pass_rate = eval_report.pass_rate();
    let eval_avg_score = eval_report.avg_judge_score;
    let eval_hallucination_rate = eval_report.estimated_hallucination_rate();

    let failure_ids = eval_report
        .failures()
        .into_iter()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>();
    let failure_preview = if failure_ids.is_empty() {
        "none".to_string()
    } else {
        failure_ids
            .into_iter()
            .take(6)
            .collect::<Vec<_>>()
            .join(", ")
    };

    let eval_summary = format!(
        "suite={} run_id={} pass_rate={:.1}% avg_judge_score={:.3} hallucination_rate={:.1}% total_cases={} failed_cases={} ({})",
        eval_report.suite_name,
        eval_report.run_id,
        eval_pass_rate * 100.0,
        eval_avg_score,
        eval_hallucination_rate * 100.0,
        eval_report.total_cases,
        eval_report.failed,
        failure_preview,
    );
    let eval_metrics = serde_json::json!({
        "suite_name": eval_report.suite_name,
        "run_id": eval_report.run_id,
        "pass_rate": eval_pass_rate,
        "avg_judge_score": eval_avg_score,
        "hallucination_rate": eval_hallucination_rate,
        "total_cases": eval_report.total_cases,
        "passed": eval_report.passed,
        "failed": eval_report.failed,
        "failure_preview": failure_preview,
    });
    let eval_artifacts =
        serde_json::to_value(&eval_report).unwrap_or_else(|_| serde_json::json!({}));
    if let Err(error) = store
        .record_llm_improvement_run(
            "standard_eval_suite",
            &eval_report.run_id,
            &eval_metrics,
            &eval_artifacts,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist eval run artifact");
    }
    if passes_shared_insight_quality_gate(
        "LLM Eval Gate Report",
        &eval_summary,
        Some("llm_eval_report"),
    ) {
        if let Err(e) = store
            .insert_insight(
                "LLM Eval Gate Report",
                &eval_summary,
                Some("llm_eval_report"),
                Some("global"),
                Some(eval_pass_rate),
                None,
                None,
                Some(vec![
                    "llm".to_string(),
                    "self_improvement".to_string(),
                    "eval".to_string(),
                ]),
            )
            .await
        {
            tracing::warn!(error = %e, "self_improvement_cycle: failed to persist llm eval report insight");
        }
    }

    if eval_pass_rate < min_eval_pass_rate
        || eval_avg_score < min_eval_score
        || eval_hallucination_rate > max_hallucination_rate
    {
        tracing::warn!(
            eval_pass_rate,
            min_eval_pass_rate,
            eval_avg_score,
            min_eval_score,
            eval_hallucination_rate,
            max_hallucination_rate,
            failed_cases = eval_report.failed,
            "self_improvement_cycle: llm eval quality gate breached"
        );
        let severity = if eval_pass_rate < (min_eval_pass_rate - 0.15)
            || eval_hallucination_rate > (max_hallucination_rate + 0.15)
        {
            "critical"
        } else {
            "high"
        };
        let desc = format!(
            "LLM quality gate breached. pass_rate={:.1}% (min {:.1}%), avg_score={:.3} (min {:.3}), hallucination={:.1}% (max {:.1}%). failed_cases={}.",
            eval_pass_rate * 100.0,
            min_eval_pass_rate * 100.0,
            eval_avg_score,
            min_eval_score,
            eval_hallucination_rate * 100.0,
            max_hallucination_rate * 100.0,
            eval_report.failed,
        );
        match store
            .insert_warning(
                "llm_quality_regression",
                "LLM quality regression detected",
                Some(&desc),
                severity,
                Some("global"),
                None,
                None,
                None,
                Some((1.0 - eval_pass_rate).clamp(0.0, 1.0)),
            )
            .await
        {
            Ok(warning_id) => tracing::warn!(
                warning_id = %warning_id,
                severity,
                "self_improvement_cycle: inserted llm quality regression warning"
            ),
            Err(error) => tracing::error!(
                %error,
                severity,
                "self_improvement_cycle: failed to insert llm quality regression warning"
            ),
        }
    }

    let insights = store
        .list_insights(
            &InsightListFilters {
                date_from: Some(Utc::now() - chrono::Duration::days(14)),
                ..Default::default()
            },
            120,
            0,
        )
        .await
        .context("llm self-improvement: failed to load recent insights")?;
    let warnings = store
        .list_warnings(
            &WarningListFilters {
                date_from: Some(Utc::now() - chrono::Duration::days(14)),
                ..Default::default()
            },
            None,
            true,
            120,
            0,
        )
        .await
        .context("llm self-improvement: failed to load recent warnings")?;

    let mut loop_runner = SelfImprovementLoop::new(
        llm,
        SelfImprovementConfig {
            min_quality_score: std::env::var("LLM_SELF_IMPROVEMENT_MIN_EXAMPLE_QUALITY")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.72)
                .clamp(0.0, 1.0),
            max_history_size: 500,
            critique_batch_size: 30,
            generate_prompt_proposals: true,
            analyse_failures: true,
        },
    );

    let mut captures_seeded = 0usize;
    for row in insights {
        let system_prompt =
            "Generate a strategic OSINT insight with explicit evidence and quantified confidence.";
        let user_prompt = format!(
            "title={} | type={} | region={} | confidence={:.3}",
            row.title,
            row.insight_type.as_deref().unwrap_or("unknown"),
            row.region.as_deref().unwrap_or("global"),
            row.confidence.unwrap_or(0.0),
        );
        let mut capture = OutputCapture::new(
            TaskCategory::InsightGeneration,
            system_prompt,
            user_prompt,
            row.summary,
            0,
        );
        capture.quality_score = row.confidence;
        capture.was_used = true;
        capture.captured_at = row.updated_at.or(row.created_at).unwrap_or_else(Utc::now);
        loop_runner.record(capture);
        captures_seeded += 1;
    }
    for row in warnings {
        let system_prompt =
            "Generate concise, evidence-grounded security or risk warnings without speculation.";
        let user_prompt = format!(
            "warning_type={} | severity={} | region={} | title={}",
            row.warning_type,
            row.severity,
            row.region.as_deref().unwrap_or("global"),
            row.title,
        );
        let mut capture = OutputCapture::new(
            TaskCategory::EvidenceChain,
            system_prompt,
            user_prompt,
            row.description.unwrap_or_default(),
            0,
        );
        capture.quality_score = row.confidence;
        capture.was_used = true;
        capture.captured_at = row.updated_at.or(row.created_at).unwrap_or(row.ts_utc);
        loop_runner.record(capture);
        captures_seeded += 1;
    }

    let cycle_report = loop_runner
        .run_cycle()
        .await
        .context("llm self-improvement: critique cycle failed")?;
    let training_examples = loop_runner.export_training_examples();

    let improvement_summary = format!(
        "cycle_id={} captures_seeded={} analysed={} qualifying_examples={} avg_critique={:.3} prompt_improvements={} failure_hypotheses={} training_examples={}",
        cycle_report.cycle_id,
        captures_seeded,
        cycle_report.captures_analysed,
        cycle_report.examples_qualifying,
        cycle_report.avg_critique_score,
        cycle_report.prompt_improvements.len(),
        cycle_report.failure_hypotheses.len(),
        training_examples.len(),
    );
    if passes_shared_insight_quality_gate(
        "LLM Continuous Improvement Cycle",
        &improvement_summary,
        Some("llm_self_improvement"),
    ) {
        if let Err(e) = store
            .insert_insight(
                "LLM Continuous Improvement Cycle",
                &improvement_summary,
                Some("llm_self_improvement"),
                Some("global"),
                Some(cycle_report.avg_critique_score.clamp(0.0, 1.0)),
                None,
                None,
                Some(vec![
                    "llm".to_string(),
                    "self_improvement".to_string(),
                    "continuous_learning".to_string(),
                ]),
            )
            .await
        {
            tracing::warn!(error = %e, "self_improvement_cycle: failed to persist continuous improvement insight");
        }
    }

    let jsonl_examples = ImprovementCycleReport::to_jsonl(&training_examples);
    let cycle_metrics = serde_json::json!({
        "cycle_id": cycle_report.cycle_id,
        "captures_seeded": captures_seeded,
        "captures_analysed": cycle_report.captures_analysed,
        "examples_qualifying": cycle_report.examples_qualifying,
        "avg_critique_score": cycle_report.avg_critique_score,
        "prompt_improvements": cycle_report.prompt_improvements.len(),
        "failure_hypotheses": cycle_report.failure_hypotheses.len(),
        "training_examples": training_examples.len(),
    });
    let cycle_report_value =
        serde_json::to_value(&cycle_report).unwrap_or_else(|_| serde_json::json!({}));
    let cycle_artifacts = serde_json::json!({
        "report": cycle_report_value,
        "training_examples_preview_chars": jsonl_examples.chars().count(),
    });
    if let Err(error) = store
        .record_llm_improvement_run(
            "continuous_self_improvement_cycle",
            &cycle_report.cycle_id,
            &cycle_metrics,
            &cycle_artifacts,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist continuous improvement run artifact");
    }

    let dataset_version = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        cycle_report.cycle_id
    );
    let dataset_manifest = serde_json::json!({
        "dataset_name": "llm_self_improvement_examples",
        "dataset_version": dataset_version,
        "source_cycle_id": cycle_report.cycle_id,
        "eval_suite": eval_report.suite_name,
        "eval_run_id": eval_report.run_id,
        "example_count": training_examples.len(),
        "schema": "alpaca_chat_jsonl_v1",
        "tasks": ["insight_generation", "evidence_chain"],
    });
    if let Err(error) = store
        .record_llm_training_dataset(
            "llm_self_improvement_examples",
            &dataset_version,
            "continuous_self_improvement_cycle",
            &cycle_report.cycle_id,
            &dataset_manifest,
            training_examples.len() as i64,
            &jsonl_examples,
        )
        .await
    {
        tracing::warn!(%error, "self_improvement_cycle: failed to persist training dataset artifact");
    }

    if !jsonl_examples.is_empty() {
        tracing::info!(
            training_examples = training_examples.len(),
            jsonl_chars = jsonl_examples.chars().count(),
            dataset_version = %dataset_version,
            "self_improvement_cycle: persisted training examples dataset"
        );
    }

    let min_critique = std::env::var("LLM_SELF_IMPROVEMENT_MIN_CRITIQUE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.60)
        .clamp(0.0, 1.0);
    if cycle_report.avg_critique_score < min_critique {
        tracing::warn!(
            avg_critique = cycle_report.avg_critique_score,
            min_critique,
            captures_analysed = cycle_report.captures_analysed,
            qualifying_examples = cycle_report.examples_qualifying,
            "self_improvement_cycle: llm critique quality gate breached"
        );
        let desc = format!(
            "Continuous self-improvement critique score is below threshold: avg_critique={:.3} < min={:.3}. analysed={} qualifying_examples={}",
            cycle_report.avg_critique_score,
            min_critique,
            cycle_report.captures_analysed,
            cycle_report.examples_qualifying,
        );
        match store
            .insert_warning(
                "llm_self_improvement_degradation",
                "LLM self-improvement cycle quality below threshold",
                Some(&desc),
                "high",
                Some("global"),
                None,
                None,
                None,
                Some((1.0 - cycle_report.avg_critique_score).clamp(0.0, 1.0)),
            )
            .await
        {
            Ok(warning_id) => tracing::warn!(
                warning_id = %warning_id,
                "self_improvement_cycle: inserted llm self-improvement degradation warning"
            ),
            Err(error) => tracing::error!(
                %error,
                "self_improvement_cycle: failed to insert llm self-improvement degradation warning"
            ),
        }
    }

    Ok(LlmContinuousImprovementStats {
        eval_pass_rate,
        eval_avg_score,
        eval_hallucination_rate,
        captures_seeded,
        captures_analysed: cycle_report.captures_analysed,
        qualifying_examples: cycle_report.examples_qualifying,
        avg_critique_score: cycle_report.avg_critique_score,
    })
}

#[cfg(feature = "llm")]
fn raw_to_poi_artifact(
    person_id: Uuid,
    raw: RawPersonArtifact,
    fallback_url: &str,
    now: chrono::DateTime<Utc>,
) -> Option<PoiArtifact> {
    let url = raw.url.clone().unwrap_or_else(|| fallback_url.to_string());
    if url.trim().is_empty() {
        return None;
    }

    let ts_utc = chrono::DateTime::from_timestamp(raw.ts_utc, 0).unwrap_or(now);
    let mut artifact = PoiArtifact::new(
        person_id,
        map_raw_artifact_type(&raw.artifact_type),
        url.clone(),
        ts_utc,
    );
    artifact.title = Some(raw.title);
    artifact.content_summary = Some(raw.content);
    artifact.source_domain = extract_domain(&url);
    artifact.language = raw.language;
    artifact.topics = raw
        .meta
        .get("topic")
        .map(|t| vec![t.clone()])
        .unwrap_or_default();
    artifact.sentiment_score = Some(raw.confidence as f64);
    artifact.key_phrases = raw.meta.keys().cloned().collect();
    artifact.provenance = serde_json::json!({
        "source": raw.source,
        "artifact_type": raw.artifact_type,
    });
    artifact.metadata = serde_json::json!(raw.meta);
    Some(artifact)
}

#[cfg(feature = "llm")]
fn is_high_quality_raw_artifact(raw: &RawPersonArtifact) -> bool {
    if !raw.confidence.is_finite() || raw.confidence < 0.55 {
        return false;
    }

    let title_ok = raw.title.trim().len() >= 8;
    let content_len = raw.content.trim().len();
    if !title_ok && content_len < 40 {
        return false;
    }

    if raw.source.starts_with("social_") {
        let credibility = raw.confidence >= 0.65;
        let has_engagement = raw
            .meta
            .get("engagement")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v >= 20.0)
            .unwrap_or(false);
        let has_url = raw
            .url
            .as_deref()
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);
        return (credibility || has_engagement) && has_url && content_len >= 60;
    }

    if raw.source.starts_with("darkweb_") {
        let has_evidence = raw.meta.get("domain").is_some() || raw.meta.get("source").is_some();
        let has_url = raw
            .url
            .as_deref()
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);
        return raw.confidence >= 0.70 && has_evidence && has_url;
    }

    true
}

#[cfg(feature = "llm")]
fn dark_web_to_raw_artifacts(
    intel: &DarkWebPersonIntel,
    org_domain: Option<&str>,
) -> Vec<RawPersonArtifact> {
    let mut out = Vec::new();
    let domain_lc = org_domain.map(|d| d.to_ascii_lowercase());

    for rec in intel.contact_records.iter().take(20) {
        let email_match = rec
            .email
            .as_deref()
            .map(|e| {
                domain_lc
                    .as_deref()
                    .map(|d| e.to_ascii_lowercase().ends_with(&format!("@{d}")))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if rec.confidence < 0.35 && !email_match {
            continue;
        }

        let mut meta = std::collections::HashMap::new();
        meta.insert("source".to_string(), "onion_contact".to_string());
        meta.insert("confidence".to_string(), format!("{:.2}", rec.confidence));
        if let Some(org) = &rec.org {
            meta.insert("org".to_string(), org.clone());
        }
        if let Some(email) = &rec.email {
            meta.insert("email".to_string(), email.clone());
        }
        if let Some(phone) = &rec.phone {
            meta.insert("phone".to_string(), phone.clone());
        }

        let mut artifact = RawPersonArtifact {
            source: "darkweb_contact".to_string(),
            artifact_type: "contact_leak".to_string(),
            title: format!("Onion contact signal for {}", intel.full_name),
            content: format!(
                "Name: {} | Email: {} | Phone: {} | Org: {}",
                rec.name.clone().unwrap_or_default(),
                rec.email.clone().unwrap_or_default(),
                rec.phone.clone().unwrap_or_default(),
                rec.org.clone().unwrap_or_default()
            ),
            url: Some(rec.source_url.clone()),
            ts_utc: rec.ts_scraped,
            language: None,
            confidence: if email_match {
                0.80
            } else {
                rec.confidence.max(0.70)
            },
            meta,
        };

        if email_match {
            artifact
                .meta
                .insert("domain_match".to_string(), "true".to_string());
        }
        out.push(artifact);
    }

    for rec in intel.breach_records.iter().take(20) {
        let email_lc = rec.email.to_ascii_lowercase();
        let domain_match = domain_lc
            .as_deref()
            .map(|d| email_lc.ends_with(&format!("@{d}")))
            .unwrap_or(false);
        if !domain_match {
            continue;
        }

        let mut meta = std::collections::HashMap::new();
        meta.insert("source".to_string(), rec.source.clone());
        meta.insert("domain".to_string(), rec.domain.clone());
        if let Some(date_posted) = &rec.date_posted {
            meta.insert("date_posted".to_string(), date_posted.clone());
        }

        out.push(RawPersonArtifact {
            source: "darkweb_breach".to_string(),
            artifact_type: "breach_credential".to_string(),
            title: format!("Onion breach match for {}", intel.full_name),
            content: format!("Breach email match for monitored domain: {}", rec.email),
            url: Some("https://onion.local/breach-match".to_string()),
            ts_utc: intel.ts_scraped,
            language: None,
            confidence: 0.82,
            meta,
        });
    }

    out
}

#[cfg(feature = "llm")]
fn map_raw_artifact_type(kind: &str) -> ArtifactType {
    match kind {
        "quote" => ArtifactType::PressQuote,
        "talk" => ArtifactType::SpeakerBio,
        "publication" => ArtifactType::Article,
        "social_mention" => ArtifactType::Article,
        "contact_leak" | "breach_credential" => ArtifactType::Other("security_intel".to_string()),
        "role" | "board_seat" => ArtifactType::RoleChange,
        "bio" => ArtifactType::SpeakerBio,
        "event_mention" => ArtifactType::Article,
        "award" => ArtifactType::Article,
        other => ArtifactType::Other(other.to_string()),
    }
}

#[cfg(feature = "llm")]
fn extract_domain(url: &str) -> Option<String> {
    let no_scheme = url.split("//").nth(1).unwrap_or(url);
    let host = no_scheme.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Read a local file to string with a file-size guard (B293).
///
/// **Never use `tokio::fs::read_to_string` directly on user-controlled or
/// environment-controlled paths.**  This function checks the file size before
/// attempting to read, preventing unbounded memory allocation.
///
/// # Errors
/// - Returns `Err` if the file does not exist, is not readable, or exceeds
///   `MAX_INPUT_FILE_BYTES`.
/// - Error messages are safe for logging (do not expose file content).
///
/// # Example
/// ```ignore
/// let content = read_file_with_size_check("runtime/job_input.json").await?;
/// let data: MyStruct = serde_json::from_str(&content)?;
/// ```
async fn read_file_with_size_check(path: &str) -> Result<String> {
    // Fetch file metadata before attempting to read
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|err| anyhow::anyhow!("failed to stat file '{}': {}", path, err))?;

    let file_size = metadata.len();
    if file_size > MAX_INPUT_FILE_BYTES {
        anyhow::bail!(
            "file '{}' is {} bytes, which exceeds MAX_INPUT_FILE_BYTES ({}). \
             Refusing to read to prevent memory exhaustion.",
            path,
            file_size,
            MAX_INPUT_FILE_BYTES
        );
    }

    // Size is within limit; proceed with the read
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|err| anyhow::anyhow!("failed to read file '{}': {}", path, err))?;

    Ok(content)
}

async fn load_nightly_inputs() -> Result<NightlyInputs> {
    let path = std::env::var("NIGHTLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/nightly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<NightlyInputs>(&content)?;
    Ok(payload)
}

async fn load_weekly_inputs() -> Result<WeeklyInputs> {
    let path = std::env::var("WEEKLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/weekly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<WeeklyInputs>(&content)?;
    Ok(payload)
}

fn format_status(run: &JobRun) -> &'static str {
    match run.status {
        JobStatus::Succeeded { .. } => "succeeded",
        JobStatus::Failed { .. } => "failed",
        JobStatus::Skipped { .. } => "skipped",
        JobStatus::Running => "running",
        JobStatus::Pending => "pending",
    }
}

/// Generate common typosquat/lookalike variants for a domain name.
/// Covers: transposition, character omission, common substitutions, homoglyphs.
fn generate_typosquat_variants(domain: &str) -> Vec<String> {
    // Split off TLD so we only mutate the SLD
    let parts: Vec<&str> = domain.rsplitn(2, '.').collect();
    if parts.len() < 2 {
        return vec![];
    }
    let tld = parts[0];
    let sld: Vec<char> = parts[1].chars().collect();
    let n = sld.len();
    let mut variants = std::collections::HashSet::new();

    // Transposition: swap adjacent chars
    for i in 0..n.saturating_sub(1) {
        let mut v = sld.clone();
        v.swap(i, i + 1);
        let s: String = v.iter().collect();
        variants.insert(format!("{}.{}", s, tld));
    }

    // Omission: drop each char
    for i in 0..n {
        let s: String = sld
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, c)| c)
            .collect();
        if !s.is_empty() {
            variants.insert(format!("{}.{}", s, tld));
        }
    }

    // Hyphen insertion
    for i in 1..n {
        let (a, b): (String, String) = (sld[..i].iter().collect(), sld[i..].iter().collect());
        variants.insert(format!("{}-{}.{}", a, b, tld));
    }

    variants.into_iter().take(50).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_quality_gate_accepts_natural_veracity_summary() {
        let title = "NVIDIA: likely gpu and procurement development";
        let summary = "NVIDIA is appearing in 4 recent reports across 3 independent sources. The reported development centers on gpu and procurement activity, and source coverage is being compared for corroboration. The current read is likely because reported hiring and supply-chain evidence are showing up across multiple articles. If this matters commercially, keep it on an active watchlist while stronger confirmation arrives.";

        assert!(passes_shared_insight_quality_gate(
            title,
            summary,
            Some("veracity_analysis")
        ));
    }

    #[test]
    fn shared_quality_gate_rejects_template_markers() {
        let title = "Intelligence Veracity: NVIDIA";
        let summary = "Additional source reporting: source one. Signal themes detected: gpu, procurement. Assessment: MODERATE-HIGH CONFIDENCE. Actionable: move now.";

        assert!(!passes_shared_insight_quality_gate(
            title,
            summary,
            Some("veracity_analysis")
        ));
    }

    #[test]
    fn shared_quality_gate_allows_internal_llm_reports() {
        assert!(passes_shared_insight_quality_gate(
            "LLM Eval Gate Report",
            "Assessment: internal evaluator output is intentionally structured for debugging.",
            Some("llm_eval_report")
        ));
    }

    #[test]
    fn generic_action_hints_are_not_treated_as_concrete_lanes() {
        assert!(is_generic_action_hint("Monitor sentiment trajectory"));
        assert!(is_generic_action_hint("Identify topic drivers"));
        assert!(is_generic_action_hint(
            "Convene cross-functional risk assessment"
        ));
        assert!(is_generic_action_hint(
            "Scenario-plan for operational disruption"
        ));
        assert!(is_generic_action_hint(
            "Document and classify suspicious domains"
        ));
        assert!(is_generic_action_hint("Initiate takedown procedures"));
        assert!(!is_generic_action_hint(
            "Review DG GROW supplier notice from 2026-03-07"
        ));
        assert!(!is_generic_action_hint(
            "Initiate takedown for tunisia-gov-alert.com"
        ));
    }

    #[test]
    fn token_jaccard_similarity_ignores_duplicate_tokens() {
        let left = vec!["nvidia".to_string(), "gpu".to_string(), "gpu".to_string()];
        let right = vec![
            "nvidia".to_string(),
            "supply".to_string(),
            "gpu".to_string(),
        ];

        let similarity = token_jaccard_similarity(&left, &right);

        assert!((similarity - (2.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn env_flag_uses_shared_truthy_parser() {
        std::env::set_var("APEX_TEST_ENV_FLAG", " yes ");

        assert!(env_flag("APEX_TEST_ENV_FLAG"));

        std::env::remove_var("APEX_TEST_ENV_FLAG");
    }

    #[test]
    fn truncate_text_uses_shared_utf8_boundary_logic() {
        assert_eq!(truncate_text("hello 😀", 7), "hello ");
    }

    #[test]
    fn aggregate_count_signal_details_are_not_treated_as_concrete_evidence() {
        let signal_details = vec![
            "26 job posting(s) observed".to_string(),
            "2 patent(s) filed".to_string(),
            "16 certification(s) on record".to_string(),
            "1 competitor signal(s)".to_string(),
            "45 web change(s) detected".to_string(),
        ];

        assert_eq!(count_concrete_signal_details(&signal_details), 0);
        assert!(!should_emit_fallback_insight(
            &signal_details,
            "Monitor sentiment trajectory; Assess customer impact",
            0.48,
            2,
        ));
        assert!(!should_emit_fallback_insight(
            &signal_details,
            "Convene cross-functional risk assessment; Scenario-plan for operational disruption",
            0.82,
            4,
        ));
    }

    #[test]
    fn shared_quality_gate_rejects_generic_count_only_fallback_summary() {
        let summary = "Arrow Electronics (North America) has been flagged for security and compliance concerns. Our monitoring detected: 26 job posting(s) observed; 2 patent(s) filed; 16 certification(s) on record; 1 competitor signal(s); 45 web change(s) detected. If the priority is assurance, gather the evidence that would reassure auditors, customers, and procurement teams that Arrow Electronics still clears the relevant security or quality gate. This deserves action inside the current planning cycle rather than being left as passive watch-list material. Overall confidence is preliminary (48%), grounded in 2 supporting signal(s).";

        assert!(!passes_shared_insight_quality_gate(
            "Arrow Electronics: security and compliance concerns",
            summary,
            Some("security_compliance"),
        ));
    }

    #[test]
    fn shared_quality_gate_rejects_public_sector_single_count_fallback_summary() {
        let summary = "Government of Czech Republic (Europe) has been flagged for geopolitical developments affecting operations. Our monitoring detected: 1 web change(s) detected. Policy and trade exposure can quickly make footprint, routing, and export eligibility more decisive than nominal unit cost, especially where EU and North Africa positioning changes the compliance or resilience story. Immediate next step: Convene cross-functional risk assessment. Scenario-plan for operational disruption.";

        assert!(!passes_shared_insight_quality_gate(
            "Government of Czech Republic: geopolitical developments affecting operations",
            summary,
            Some("geopolitical_analysis"),
        ));
    }

    #[test]
    fn nonsecurity_titles_skip_aggregate_counts_and_legacy_phrases() {
        let title = build_analytical_title(
            "Government of Netherlands",
            "Europe",
            "geopolitical_analysis",
            &["86 web change(s) detected".to_string()],
            Some("Government"),
        );

        assert!(title.contains("policy and trade exposure shift"));
        assert!(!title.contains("diplomatic development flagged"));
        assert!(!title.contains("86 web change(s) detected"));
    }

    #[test]
    fn customer_rfq_titles_use_emerging_procurement_language() {
        let title = build_analytical_title(
            "Jabil",
            "North America",
            "customer_rfq",
            &["18 job posting(s) observed".to_string()],
            Some("Company"),
        );

        assert!(title.contains("procurement signal emerging"));
        assert!(!title.contains("RFQ or customer engagement activity"));
        assert!(!title.contains("18 job posting(s) observed"));
    }

    #[test]
    fn public_sector_brand_sentiment_avoids_commercial_template_language() {
        let summary = build_fallback_summary(
            "European Commission (Europe) has been flagged for brand sentiment and reputation signals.",
            "Monitor sentiment trajectory; Identify topic drivers; Assess customer impact",
            &["Press coverage increased after an EU policy dispute".to_string()],
            &[],
            "European Commission",
            "Europe",
            Some("Government"),
            "brand_sentiment",
            "warning",
            0.79,
            3,
        );

        let lower = summary.to_ascii_lowercase();
        assert!(!lower.contains("pipeline capture"));
        assert!(!lower.contains("commercial upside"));
        assert!(!lower.contains("one concrete lane worth testing is this"));
        assert!(
            lower.contains("tender scrutiny")
                || lower.contains("oversight")
                || lower.contains("approval timing")
        );
    }

    #[test]
    fn public_sector_security_fallback_uses_specific_artifacts_not_generic_playbook() {
        let summary = build_fallback_summary(
            "Government of Tunisia (MENA) has been flagged for cybersecurity threats and vulnerabilities.",
            "Document and classify suspicious domains; Initiate takedown procedures",
            &[
                "3 lookalike domain(s) detected".to_string(),
                "DNS posture degradation detected".to_string(),
            ],
            &[],
            "Government of Tunisia",
            "MENA",
            Some("Government"),
            "cybersecurity_threat",
            "warning",
            0.83,
            3,
        );

        let lower = summary.to_ascii_lowercase();
        assert!(lower.contains(
            "key evidence: 3 lookalike domain(s) detected; dns posture degradation detected"
        ));
        assert!(
            lower.contains("official domains")
                || lower.contains("procurement portals")
                || lower.contains("citizen-facing services")
        );
        assert!(!lower.contains("vendor eligibility"));
        assert!(!lower.contains("customer audits"));
        assert!(!lower.contains("immediate next step:"));
    }

    #[test]
    fn public_sector_geopolitical_fallback_avoids_generic_supply_chain_actions() {
        let summary = build_fallback_summary(
            "European Commission (Europe) has been flagged for geopolitical developments affecting operations.",
            "Adjust supply chain monitoring priorities; Review partner exposure",
            &["European Commission published an updated trade defense consultation notice".to_string()],
            &[],
            "European Commission",
            "Europe",
            Some("Government"),
            "geopolitical_analysis",
            "warning",
            0.81,
            2,
        );

        let lower = summary.to_ascii_lowercase();
        assert!(!lower.contains("immediate next step:"));
        assert!(!lower.contains("adjust supply chain monitoring priorities"));
        assert!(
            lower.contains("official statement")
                || lower.contains("tender amendment")
                || lower.contains("oversight action")
        );
    }

    #[test]
    fn fallback_summary_appends_numbered_sources_footer() {
        let summary = build_fallback_summary(
            "European Commission (Europe) has been flagged for geopolitical developments affecting operations.",
            "Review consultation scope",
            &["European Commission published an updated trade defense consultation notice".to_string()],
            &[
                "https://ec.europa.eu/commission/presscorner/detail/en/ip_26_1234".to_string(),
                "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            ],
            "European Commission",
            "Europe",
            Some("Government"),
            "geopolitical_analysis",
            "warning",
            0.81,
            2,
        );

        assert!(summary.contains("Sources:"));
        assert!(summary.contains(
            "[1] ec.europa.eu — https://ec.europa.eu/commission/presscorner/detail/en/ip_26_1234"
        ));
        assert!(summary.contains(
            "[2] trade.ec.europa.eu — https://trade.ec.europa.eu/doclib/notice-2026-03-07"
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn low_signal_security_hygiene_rejects_overstated_program_impact() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "El Sewedy DNS posture degradation".to_string(),
                description:
                    "DNS posture score at 70 with missing DKIM record and elevated spoofing risk."
                        .to_string(),
                source_url: "https://example.com/dns".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "30% DNS posture score deficiency".to_string(),
                    "Missing DKIM record".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "El Sewedy certifications".to_string(),
                description: "Observed certification scope includes ISO 9001 only.".to_string(),
                source_url: "https://example.com/certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec!["ISO 9001".to_string()],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.9,
            },
        ];

        let narrative = "El Sewedy's DNS posture and missing DKIM undermine its ability to qualify for EU defense programs and medical device manufacturing because customers could face compliance delays.";
        let recommendation = "Approach named customers and position for supply chain disruption response by Q2 2026 [1].";

        assert!(has_unsupported_security_escalation(
            "security_compliance",
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn low_signal_certification_warning_rejects_switching_claims() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Kimball Electronics certification warning".to_string(),
                description: "Certification warning detected on the capabilities page, with only a generic compliance notice and no named customer or program impact.".to_string(),
                source_url: "https://example.com/kimball-warning".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "Certification warning detected".to_string(),
                    "Capabilities page update".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Kimball certifications".to_string(),
                description: "Observed certifications include ISO 13485 and IATF 16949.".to_string(),
                source_url: "https://example.com/kimball-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec!["ISO 13485".to_string(), "IATF 16949".to_string()],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.9,
            },
        ];

        let narrative = "Kimball's certification warning means medical device customers could face qualification delays and may switch suppliers because the company appears to be struggling with compliance.";
        let recommendation = "Target Kimball's medical and automotive customers for a nearshore switch campaign by Q2 2026 [1].";

        assert!(has_unsupported_certification_escalation(
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_is_rejected() {
        let narrative = "Key Tronic's tariff and compliance warnings create a vulnerability for EU medical device customers because rising cross-border costs could pressure delivery commitments.";
        let recommendation =
            "Target their medical device customers for a nearshore switch campaign by Q2 2026 [1].";

        assert!(has_unnamed_customer_targeting(narrative, recommendation));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn public_sector_macro_signal_without_procurement_rejects_hardware_sales_pitch() {
        let entity_ctx = EntityContext {
            name: "Government of UAE".to_string(),
            region: "MENA".to_string(),
            entity_type: Some("Government".to_string()),
            is_competitor: false,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: Some("government.ae".to_string()),
        };

        let evidence_signals = vec![
            EvidenceSignal {
                title: "government.ae innovation update".to_string(),
                description: "Innovation signals were detected on government.ae, highlighting strategic investment in media infrastructure and digital transformation.".to_string(),
                source_url: "https://government.ae/en/media/innovation".to_string(),
                signal_type: "web_change".to_string(),
                extracted_facts: vec!["government.ae".to_string(), "innovation".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "New Media Academy announced".to_string(),
                description: "The Government of UAE announced the New Media Academy to train media professionals and social media content creators.".to_string(),
                source_url: "https://government.ae/en/news/new-media-academy".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec!["New Media Academy".to_string(), "media professionals".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 0.9,
            },
        ];

        let narrative = "These developments suggest growing demand for electronics manufacturing services to support UAE's digital transformation. The New Media Academy implies need for advanced hardware, including cameras, streaming devices, and AI-driven analytics tools. Given our ISO 9001 and AS9100 certifications, we are qualified to supply reliable electronics for government projects.";
        let recommendation = "Target the New Media Academy's procurement team to offer PCBA assembly and box build services for media equipment, and submit a qualification package for defense-adjacent electronics manufacturing [1].";

        assert!(has_unsupported_public_sector_commercialization(
            &entity_ctx,
            "geopolitical_analysis",
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn public_sector_explicit_tender_for_hardware_does_not_trigger_sales_pitch_guard() {
        let entity_ctx = EntityContext {
            name: "Government of UAE".to_string(),
            region: "MENA".to_string(),
            entity_type: Some("Government".to_string()),
            is_competitor: false,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: Some("government.ae".to_string()),
        };

        let evidence_signals = vec![EvidenceSignal {
            title: "Ministry tender for broadcast equipment".to_string(),
            description: "The ministry procurement portal published a tender for broadcast cameras, streaming devices, and control-room electronics for the New Media Academy campus.".to_string(),
            source_url: "https://procurement.gov.ae/tenders/media-equipment".to_string(),
            signal_type: "tender".to_string(),
            extracted_facts: vec!["tender".to_string(), "broadcast cameras".to_string(), "control-room electronics".to_string()],
            date_context: Some("2026-02-28".to_string()),
            relevance_score: 1.0,
        }];

        let narrative = "A named ministry tender now creates an explicit equipment procurement path for the New Media Academy campus [1].";
        let recommendation = "Approach the named tender contact with a qualification-led hardware manufacturing response [1].";

        assert!(!has_unsupported_public_sector_commercialization(
            &entity_ctx,
            "regulatory_policy",
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn public_sector_generic_opportunity_language_is_rejected_as_low_usefulness() {
        let entity_ctx = EntityContext {
            name: "Government of Canada".to_string(),
            region: "North America".to_string(),
            entity_type: Some("Government".to_string()),
            is_competitor: false,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: Some("canada.ca".to_string()),
        };

        let evidence_signals = vec![
            EvidenceSignal {
                title: "Government tariff update".to_string(),
                description: "A government tariff update was posted on the official site together with general hiring signals.".to_string(),
                source_url: "https://canada.ca/trade/tariff-update".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec!["tariff update".to_string(), "hiring".to_string()],
                date_context: Some("2026-02-28".to_string()),
                relevance_score: 1.0,
            },
        ];

        let headline = "Government of Canada's Tariff & Hiring Signals: Nearshoring Opportunity in North Africa";
        let narrative = "The tariff update and hiring signals point to a nearshoring opportunity in North Africa for Starz because public-sector buyers may need resilient supply alternatives.";
        let recommendation = "Pursue the nearshoring opportunity and position a North Africa manufacturing response this quarter [1].";

        assert!(has_low_usefulness_public_sector_analysis(
            &entity_ctx,
            "geopolitical_analysis",
            headline,
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn public_sector_concrete_policy_artifact_and_account_action_remain_allowed() {
        let entity_ctx = EntityContext {
            name: "European Commission".to_string(),
            region: "Europe".to_string(),
            entity_type: Some("Government".to_string()),
            is_competitor: false,
            industry_tags: vec![],
            certifications: vec![],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: Some("ec.europa.eu".to_string()),
        };

        let evidence_signals = vec![EvidenceSignal {
            title: "Trade defense consultation notice".to_string(),
            description: "The European Commission published a trade defense consultation notice that could affect supplier qualification and approval timing for related programs.".to_string(),
            source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            signal_type: "regulatory".to_string(),
            extracted_facts: vec!["consultation notice".to_string(), "supplier qualification".to_string(), "approval timing".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        }];

        let headline = "European Commission trade defense consultation could affect supplier qualification timing";
        let narrative = "Because the consultation notice explicitly affects supplier qualification and approval timing, regulated bids tied to the Commission may face a narrower response window [1].";
        let recommendation = "Review the consultation notice, map exposed bids and stakeholder owners, and update qualification planning before approval timing moves [1].";

        assert!(!has_low_usefulness_public_sector_analysis(
            &entity_ctx,
            "regulatory_policy",
            headline,
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn sources_footer_lists_ranked_evidence_urls() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Most relevant".to_string(),
                description: "Primary evidence".to_string(),
                source_url: "https://alpha.example.com/report".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![],
                date_context: None,
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Secondary".to_string(),
                description: "Backup evidence".to_string(),
                source_url: "https://beta.example.com/article".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec![],
                date_context: None,
                relevance_score: 0.7,
            },
        ];

        let footer = format_sources_footer(&evidence_signals, 4);
        assert!(footer.contains("Sources:"));
        assert!(footer.contains("[1] alpha.example.com — https://alpha.example.com/report"));
        assert!(footer.contains("[2] beta.example.com — https://beta.example.com/article"));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn company_name_matching_normalizes_punctuation() {
        assert!(company_name_matches_seed(
            "STMicroelectronics N.V.",
            "STMicroelectronics NV"
        ));
        assert!(company_name_matches_seed(
            "Young Poong Electronics Co., Ltd.",
            "Young Poong Electronics Co Ltd"
        ));
        assert!(!company_name_matches_seed("NVIDIA", "AMD"));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn company_name_matching_uses_shared_canonicalization() {
        let mixed = format!("{}cme", '\u{0410}');
        assert!(company_name_matches_seed(
            "Café Société S.A.",
            "Cafe Societe SA"
        ));
        assert!(company_name_matches_seed(&mixed, "Acme"));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn candidate_company_domain_prefers_corporate_email() {
        let disc = DiscoveredPoi {
            name: "Jane Doe".to_string(),
            inferred_role: Some("VP Supply Chain".to_string()),
            inferred_org: Some("Acme Electronics".to_string()),
            source_url: "https://news.example.com/article".to_string(),
            discovery_method: "gdelt_co_mention".to_string(),
            contact_email: Some("jane.doe@acme-electronics.com".to_string()),
            contact_linkedin: None,
            confidence: 0.84,
            seed_person_id: "seed-1".to_string(),
            ts_discovered: 0,
        };

        assert_eq!(
            extract_candidate_company_domain(&disc).as_deref(),
            Some("acme-electronics.com")
        );
    }

    #[cfg(feature = "llm")]
    #[test]
    fn candidate_company_domain_ignores_public_mailboxes() {
        let disc = DiscoveredPoi {
            name: "Jane Doe".to_string(),
            inferred_role: Some("VP Supply Chain".to_string()),
            inferred_org: Some("Acme Electronics".to_string()),
            source_url: "https://www.acme-electronics.com/team".to_string(),
            discovery_method: "org_leadership".to_string(),
            contact_email: Some("janedoe@gmail.com".to_string()),
            contact_linkedin: None,
            confidence: 0.84,
            seed_person_id: "seed-1".to_string(),
            ts_discovered: 0,
        };

        assert_eq!(
            extract_candidate_company_domain(&disc).as_deref(),
            Some("acme-electronics.com")
        );
    }
}
