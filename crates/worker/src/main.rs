#![cfg_attr(test, allow(dead_code))]
#![allow(clippy::duplicated_attributes, clippy::too_many_arguments)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod config;
mod digest_filtering;
#[allow(dead_code)]
mod evidence_scoring;
#[allow(dead_code)]
mod fallback_generation;
#[cfg(feature = "llm")]
mod geo_targeting;
mod job_execution;
#[allow(dead_code)]
mod llm_orchestration;
mod observability;
#[allow(dead_code)]
mod quality_gates;
mod runtime;
#[allow(dead_code)]
mod title_formatting;

#[cfg(feature = "llm")]
pub(crate) use digest_filtering::is_promoted_business_insight_type;
use digest_filtering::{
    canonical_digest_key, digest_tokens, expand_digest_categories, token_jaccard_similarity,
};
#[cfg(feature = "llm")]
use digest_filtering::{has_excessive_phrase_repetition, is_readable_and_useful_digest_text};
pub(crate) use digest_filtering::{is_digest_insight_quality, passes_shared_insight_quality_gate};
#[cfg(feature = "llm")]
use fallback_generation::format_sources_footer_from_urls;
#[allow(unused_imports)]
pub(crate) use fallback_generation::{build_fallback_summary, should_emit_fallback_insight};
use fallback_generation::{category_relevant_signal_details, detect_strategy_signal_flags};
#[cfg(test)]
use fallback_generation::{count_concrete_signal_details, is_generic_action_hint};
#[allow(unused_imports)]
pub(crate) use title_formatting::build_analytical_title;

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
#[cfg(any(feature = "llm", test))]
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
use apex_insights::arbitrage::{default_profiles as arbitrage_default_profiles, ArbitrageDetector};
#[cfg(feature = "llm")]
use apex_insights::bias_mitigation::{
    generate_devils_advocate, DevilsAdvocateConfig, EvidenceItem as BiasEvidenceItem, Severity,
};
#[cfg(feature = "llm")]
use apex_insights::comparison::build_comparison_matrix;
#[cfg(feature = "llm")]
use apex_insights::hypothesis::{EntityHypothesisTracker, EvidenceType as HypothesisEvidenceType};
#[cfg(feature = "llm")]
use apex_insights::predictive::predefined_patterns;
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
// apex_poi::updater now exposes `update_profile` instead of `refresh_profile`
use apex_recipes::engine::{FeatureMap, RecipeEngine};
#[cfg(feature = "llm")]
use apex_store::postgres::{
    HistoricalQualityGateLabel, QualityGateGoldenSetExample, WarningListFilters,
};
use apex_store::postgres::{InsightListFilters, PersonListFilters, PersonOrderBy, PgStore};
use apex_worker::activity_logger::ActivityLogger;
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
#[cfg(feature = "llm")]
use chrono::DateTime;
use chrono::{Datelike, Timelike, Utc};
use chrono_tz::Europe::Berlin;
#[cfg(all(feature = "llm", test))]
use evidence_scoring::noisy_or;
#[cfg(feature = "llm")]
use evidence_scoring::{calculate_relevance, signal_diversity_multiplier};
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
#[cfg(feature = "llm")]
use llm_orchestration::build_llm_retry_guidance;
#[cfg(feature = "llm")]
pub(crate) use quality_gates::normalize_gate_text;
#[cfg(all(feature = "llm", test))]
use quality_gates::{
    contains_causal_link, contains_security_hygiene_marker,
    contains_soft_certification_pressure_marker,
};
#[cfg(feature = "llm")]
use quality_gates::{
    has_formulaic_commercial_language, has_low_usefulness_public_sector_analysis,
    has_temporal_incoherence, has_unnamed_customer_targeting,
    has_unsupported_certification_commercialization, has_unsupported_certification_escalation,
    has_unsupported_named_target_provenance, has_unsupported_public_sector_commercialization,
    has_unsupported_security_escalation, low_signal_certification_warning_case,
    recommendation_has_action_timing, weighted_phrase_score,
};
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
#[allow(dead_code)]
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
#[allow(dead_code)]
fn resolve_evidence_placeholders(template: &str, slots: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{evidence:") {
        let prefix = &rest[..start];
        let after = &rest[start + 11..];
        if let Some(end) = after.find("}}") {
            let key = &after[..end];
            if let Some(val) = slots.get(key) {
                result.push_str(prefix);
                result.push_str(val);
            } else {
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
            rest = &after[end + 2..];
            if slots.get(key).is_none() && rest.starts_with('%') {
                rest = &rest[1..];
            }
        } else {
            result.push_str(prefix);
            result.push_str(&rest[start..]);
            rest = "";
        }
    }
    result.push_str(rest);
    result
}

fn clean_rendered_text(s: &str) -> String {
    let mut text = s.to_string();
    for pattern in &["()", "( )", "[]", "[ ]"] {
        text = text.replace(pattern, "");
    }
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    for _ in 0..3 {
        text = text
            .replace(", .", ".")
            .replace(",,", ",")
            .replace(", ,", ",");
        text = text.replace(". .", ".").replace("..", ".");
        text = text.replace(" .", ".").replace(" ,", ",");
        text = text
            .replace(":.", ".")
            .replace(": .", ".")
            .replace(":,", ",");
        text = text.replace("( ", "(").replace(" )", ")");
        text = text.replace(". . ", ". ");
    }
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    while text.starts_with(". ") {
        text = text[2..].to_string();
    }
    if text == "." {
        text.clear();
    }
    text.trim().to_string()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn is_low_quality_narrative(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 20 {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();

    let dash_count = trimmed.matches('—').count();
    let word_count = trimmed.split_whitespace().count();
    if word_count > 0 && dash_count as f64 / word_count as f64 > 0.25 {
        return true;
    }

    let short_tokens = trimmed
        .split(". ")
        .filter(|segment| segment.split_whitespace().count() <= 2)
        .count();
    let total_sentences = trimmed.split(". ").count();
    if total_sentences >= 3 && short_tokens as f64 / total_sentences as f64 > 0.5 {
        return true;
    }

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
        "supply from under",
        "showing conflict",
        "showing including",
    ];
    if double_prep_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
    {
        return true;
    }

    let vague_alarm_patterns = [
        ("resource weaponization", "which resource"),
        ("conflict escalation", "conflict indicators"),
        ("sanctions cascade", "sanction"),
        ("technology theft", "technology"),
    ];
    for (alarm, must_have) in &vague_alarm_patterns {
        if lower.contains(alarm) {
            let has_number = trimmed.chars().any(|c| c.is_ascii_digit());
            let has_specific = lower.contains(must_have) || has_number;
            if !has_specific {
                return true;
            }
        }
    }

    let broken_grammar = [
        ". supply from",
        ". including.",
        ": .",
        " via .",
        " from .",
        " at .",
        " by .",
        " to .",
        " in .",
        " of .",
        " with .",
    ];
    broken_grammar.iter().any(|pattern| lower.contains(pattern))
}

/// Build an analytical narrative paragraph from structured signal data.
#[allow(dead_code)]
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
    let relevant_details = category_relevant_signal_details(category, signal_details);
    let flags = detect_strategy_signal_flags(category, &relevant_details, "");
    let is_public_sector = is_public_sector_entity(entity_label, entity_type);
    let concrete_details = relevant_details;

    parts.push(format!("{entity_ctx} shows {category_desc}."));

    if !concrete_details.is_empty() {
        parts.push(format!(
            "Observed signals include: {}.",
            concrete_details.join("; ")
        ));
    }
    if !concrete_details.is_empty() {
        parts.push(format!(
            "Most specific current evidence: {}.",
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

/// Returns the supply chain role for non-EMS, non-government entities.
/// These entities need differentiated LLM prompting — not the default
/// "customer/prospect" framing that leads to nonsensical partnership proposals.
#[cfg(feature = "llm")]
fn supply_chain_role(entity_type: Option<&str>) -> Option<&'static str> {
    match entity_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "distributor" => Some("DISTRIBUTOR"),
        "oem" => Some("OEM"),
        "defense" | "defence" => Some("DEFENSE_PRIME"),
        "semiconductor" => Some("SEMICONDUCTOR"),
        "pcb" => Some("PCB_MANUFACTURER"),
        "t&m" | "test_measurement" => Some("TEST_MEASUREMENT"),
        "trade_association" => Some("TRADE_ASSOCIATION"),
        _ => None,
    }
}

#[cfg(feature = "llm")]
fn inferred_supply_chain_role(entity_ctx: &EntityContext) -> Option<&'static str> {
    let corpus = format!(
        "{} {} {} {} {}",
        entity_ctx.name,
        entity_ctx.entity_type.as_deref().unwrap_or_default(),
        entity_ctx.industry_tags.join(" "),
        entity_ctx.capabilities.join(" "),
        entity_ctx.sites_summary.join(" "),
    )
    .to_ascii_lowercase();

    let has_any = |markers: &[&str]| markers.iter().any(|marker| corpus.contains(marker));

    if has_any(&[
        "trade association",
        "industry association",
        "industry body",
        "standards body",
        "chamber of commerce",
        "industry council",
    ]) {
        return Some("TRADE_ASSOCIATION");
    }

    if has_any(&[
        "semiconductor",
        "microcontroller",
        "mcu",
        "analog chip",
        "power management ic",
        "sensor ic",
        "chipmaker",
        "fabless",
        "wafer",
    ]) {
        return Some("SEMICONDUCTOR");
    }

    if has_any(&[
        "defense prime",
        "defence prime",
        "munitions",
        "missile",
        "radar",
        "electronic warfare",
        "defense contractor",
        "defence contractor",
        "aerospace and defense",
    ]) {
        return Some("DEFENSE_PRIME");
    }

    if has_any(&[
        "distributor",
        "electronics distribution",
        "component distributor",
        "authorized distributor",
        "broadline distribution",
    ]) {
        return Some("DISTRIBUTOR");
    }

    if has_any(&[
        "printed circuit board",
        "pcb manufacturer",
        "pcb fabrication",
        "bare board",
        "bare pcb",
    ]) {
        return Some("PCB_MANUFACTURER");
    }

    if has_any(&[
        "test and measurement",
        "test & measurement",
        "oscilloscope",
        "metrology",
        "signal analyzer",
    ]) {
        return Some("TEST_MEASUREMENT");
    }

    if has_any(&[
        " oem",
        "original equipment manufacturer",
        "medical device manufacturer",
        "automotive oem",
        "industrial oem",
    ]) {
        return Some("OEM");
    }

    supply_chain_role(entity_ctx.entity_type.as_deref())
}

#[cfg(feature = "llm")]
fn contains_semiconductor_sales_pitch(corpus: &str) -> bool {
    weighted_phrase_score(
        corpus,
        &[
            ("our company", 0.20),
            ("our services", 0.25),
            ("our facility", 0.20),
            ("our facilities", 0.20),
            ("our capability", 0.20),
            ("our capabilities", 0.20),
            ("our as9100", 0.25),
            ("our iatf", 0.25),
            ("our iso 14001", 0.30),
            ("our iso 9001", 0.25),
            ("our iso 13485", 0.25),
            ("our north africa", 0.20),
            ("our tunisia", 0.20),
            ("our morocco", 0.20),
            ("our european manufacturing", 0.25),
            ("manufacturing capabilities", 0.25),
            ("green manufacturing", 0.25),
            ("reliable partner", 0.20),
            ("detailed proposal", 0.25),
            ("qualification support", 0.25),
            ("compliance support", 0.25),
            ("supply chain optimization", 0.25),
            ("opportunity for our company", 0.30),
            ("immediate qualification support", 0.25),
            ("aerospace focused ems services", 0.30),
            ("ems services", 0.25),
        ],
    ) >= 0.25
}

#[cfg(feature = "llm")]
fn violates_supply_chain_role_guidance(
    supply_chain_role: Option<&str>,
    headline: &str,
    narrative: &str,
    recommendation: &str,
) -> bool {
    let Some(role) = supply_chain_role else {
        return false;
    };

    let corpus = format!("{}\n{}\n{}", headline, narrative, recommendation).to_ascii_lowercase();

    match role {
        "SEMICONDUCTOR" => {
            [
                "outsourcing opportunit",
                "ems opportunit",
                "nearshore ems",
                "sell assembly",
                "assembly services",
                "manufacturing services",
                "ems partner",
                "partner with microchip",
                "partner with nxp",
            ]
            .iter()
            .any(|phrase| corpus.contains(phrase))
                || contains_semiconductor_sales_pitch(&corpus)
        }
        "DEFENSE_PRIME" => [
            "outsourcing opportunit",
            "ems opportunit",
            "nearshore ems",
            "direct outreach to bae",
            "direct outreach to l3harris",
            "sell assembly services",
            "offer our services to bae",
            "offer our services to l3harris",
        ]
        .iter()
        .any(|phrase| corpus.contains(phrase)),
        _ => false,
    }
}

#[cfg(feature = "llm")]
fn topic_marker_hits(corpus: &str, markers: &[&str]) -> usize {
    markers
        .iter()
        .filter(|marker| corpus.contains(**marker))
        .count()
}

#[cfg(feature = "llm")]
fn violates_entity_topic_alignment(
    entity_ctx: &EntityContext,
    evidence_signals: &[EvidenceSignal],
    headline: &str,
    narrative: &str,
    recommendation: &str,
) -> bool {
    let support_corpus = format!(
        "{} {} {} {} {} {} {}",
        entity_ctx.name,
        entity_ctx.entity_type.as_deref().unwrap_or_default(),
        entity_ctx.industry_tags.join(" "),
        entity_ctx.capabilities.join(" "),
        entity_ctx.sites_summary.join(" "),
        entity_ctx.recent_changes.join(" "),
        evidence_signals
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
    )
    .to_ascii_lowercase();
    let output_corpus =
        format!("{} {} {}", headline, narrative, recommendation).to_ascii_lowercase();

    let unsupported_bundles: [&[&str]; 4] = [
        &[
            "middle east oil",
            "oil surge",
            "oil price",
            "brent",
            "crude",
            "opec",
            "petrochemical",
            "refinery",
            "lng",
            "gas field",
        ],
        &[
            "mining",
            "ore",
            "smelter",
            "lithium",
            "nickel",
            "copper concentrate",
            "rare earth",
        ],
        &[
            "agriculture",
            "crop",
            "harvest",
            "grain",
            "fertilizer",
            "farm",
            "food processing",
        ],
        &[
            "retail chain",
            "store rollout",
            "consumer packaged",
            "apparel",
            "fashion",
        ],
    ];

    unsupported_bundles.iter().any(|markers| {
        let output_hits = topic_marker_hits(&output_corpus, markers);
        let support_hits = topic_marker_hits(&support_corpus, markers);
        output_hits >= 2 && support_hits == 0
    })
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

// Quality gate types and functions extracted to llm_orchestration module
#[cfg(feature = "llm")]
use llm_orchestration::{
    emit_quality_gate_decisions, has_confidence_boilerplate, quality_gate_blocker,
    quality_gate_passes_ensemble, quality_gate_requirement,
};

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

#[cfg(feature = "llm")]
fn count_numbered_references(text: &str, max_ref: usize) -> usize {
    (1..=max_ref)
        .filter(|i| text.contains(&format!("[{}]", i)))
        .count()
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
/// Generate insight narrative and headline using LLM with rich context.
/// Returns (headline, narrative, recommendation, confidence).
#[cfg(feature = "llm")]
/// Strip forbidden certification claims from LLM insight output. Called when
/// the model persists in claiming certifications our company does not hold
/// (AS9100, IATF 16949, ISO 13485) despite corrective retry guidance. Rather
/// than discarding the entire insight, we remove the offending phrases so the
/// rest of the (potentially valuable) analysis survives.
fn sanitize_certification_claims(text: &str) -> String {
    let mut out = text.to_string();
    // Replace possessive certification claim patterns with neutral language.
    let replacements = [
        ("our AS9100", "a potential AS9100 gap in"),
        ("our IATF 16949", "a potential IATF 16949 gap in"),
        ("our ISO 13485", "a potential ISO 13485 gap in"),
        ("we hold AS9100", "we do not currently hold AS9100"),
        ("we hold IATF 16949", "we do not currently hold IATF 16949"),
        ("we hold ISO 13485", "we do not currently hold ISO 13485"),
        ("we are AS9100", "we are not yet AS9100"),
        ("we are IATF 16949", "we are not yet IATF 16949"),
        ("we are ISO 13485", "we are not yet ISO 13485"),
        ("we have AS9100", "we lack AS9100"),
        ("we have IATF 16949", "we lack IATF 16949"),
        ("we have ISO 13485", "we lack ISO 13485"),
        ("with our AS9100", "noting our AS9100 gap relative to"),
        (
            "with our IATF 16949",
            "noting our IATF 16949 gap relative to",
        ),
        ("with our ISO 13485", "noting our ISO 13485 gap relative to"),
        ("AS9100 certification", "AS9100 (which we do not hold)"),
        (
            "IATF 16949 certification",
            "IATF 16949 (which we do not hold)",
        ),
        (
            "ISO 13485 certification",
            "ISO 13485 (which we do not hold)",
        ),
        ("AS9100 certified", "AS9100-adjacent (unverified)"),
        ("IATF certified", "IATF 16949-adjacent (unverified)"),
        ("13485 certified", "ISO 13485-adjacent (unverified)"),
    ];
    for (from, to) in &replacements {
        out = out.replace(from, to);
    }
    out
}

async fn generate_llm_insight(
    llm_client: &InferenceLlmClient,
    entity_ctx: &EntityContext,
    category: &str,
    evidence_signals: &[EvidenceSignal],
) -> Result<(String, String, String, f64, serde_json::Value)> {
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
    let sc_role = inferred_supply_chain_role(entity_ctx);
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
    } else if let Some(role) = sc_role {
        let classification_text = match role {
            "DISTRIBUTOR" => format!(
                "📦 ENTITY CLASSIFICATION: COMPONENT DISTRIBUTOR — {} sells electronic components, they do NOT manufacture products. \
Do NOT recommend partnering with or selling EMS services to this distributor. \
Instead, analyse how their distribution changes (pricing, availability, new products, logistics) \
affect OEM and EMS procurement. Recommend how to leverage or hedge against these distribution shifts \
for our existing and prospective customers.",
                entity_ctx.name
            ),
            "OEM" => format!(
                "🏭 ENTITY CLASSIFICATION: OEM / END CUSTOMER — {} designs and sells end products. \
They are a potential customer for EMS services. Analyse their outsourcing needs, product roadmap signals, \
supply chain vulnerabilities, and qualification requirements. Recommend specific engagement strategies \
to win or expand manufacturing contracts with them.",
                entity_ctx.name
            ),
            "DEFENSE_PRIME" => format!(
                "🛡️ ENTITY CLASSIFICATION: DEFENSE PRIME CONTRACTOR — {} is a large defense/aerospace prime. \
They subcontract EMS work. Analyse their program timelines, subcontractor needs, compliance requirements, \
and supply chain gaps. Recommend qualification paths and specific programs where our capabilities \
(nearshore, AS9100, ITAR-free) create an advantage.",
                entity_ctx.name
            ),
            "SEMICONDUCTOR" => format!(
                "🔬 ENTITY CLASSIFICATION: SEMICONDUCTOR COMPANY — {} designs or manufactures chips/ICs. \
They are NOT an EMS prospect. Do NOT recommend selling assembly services to them. \
    Do NOT frame them as a consulting, compliance-support, proposal, or facility-pitch target either. \
    Instead, analyse how their product launches, shortages, EOL notices, or pricing changes \
    affect our customers' BOMs and procurement. Recommend supply chain actions for our customer base.",
                entity_ctx.name
            ),
            "PCB_MANUFACTURER" => format!(
                "🟢 ENTITY CLASSIFICATION: PCB MANUFACTURER — {} makes bare printed circuit boards. \
They are an upstream supplier, not an EMS prospect. Analyse their capacity, lead times, quality, \
and regional footprint to assess impact on our supply chain and our customers' PCB sourcing. \
Recommend sourcing diversification or qualification actions.",
                entity_ctx.name
            ),
            "TEST_MEASUREMENT" => format!(
                "🔧 ENTITY CLASSIFICATION: TEST & MEASUREMENT COMPANY — {} makes test equipment. \
They are a tooling vendor, not an EMS prospect. Analyse how their product updates \
or pricing affect our test capabilities and our competitiveness. \
Recommend equipment investment or qualification actions.",
                entity_ctx.name
            ),
            "TRADE_ASSOCIATION" => format!(
                "🤝 ENTITY CLASSIFICATION: TRADE ASSOCIATION / INDUSTRY BODY — {} is an industry organization. \
They do NOT buy EMS services. Analyse their policy positions, member activities, trade events, \
and regulatory advocacy for their impact on our market. Recommend engagement for visibility \
and business development, not direct sales.",
                entity_ctx.name
            ),
            _ => "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity.".to_string(),
        };
        profile_parts.push(classification_text);
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
    let our_profile = std::env::var("COMPANY_PROFILE").unwrap_or_else(|_| {
        "Starz Electronics is an electronics manufacturer (EMS heritage: PCBA assembly, box build, \
test & inspection, supply chain management) whose primary growth focus is battery energy storage \
systems (BESS): it designs and manufactures residential and commercial/industrial battery packs \
(5, 10 and 15 kWh) with an in-house battery management system (BMS) and custom pack design, \
sourcing lithium cells from global suppliers. Production facilities are in North Africa (Tunisia, \
Morocco free zones) and Europe. Certifications: ISO 9001:2015 and IPC (Institute for Printed \
Circuits) ONLY. We DO NOT hold AS9100, ISO 13485, or IATF 16949 certifications. Battery-pack sales \
focus: PRIMARILY Morocco, Tunisia and Egypt; SECONDARILY (smaller focus) the European Union; we do \
NOT sell battery packs in any other market. Cell suppliers and competing pack/BMS makers are \
tracked globally regardless of their location."
            .to_string()
    });

    let system = format!("You are a competitive intelligence analyst at an OSINT firm. Your job is to read raw signal evidence and report what it actually shows — nothing more, nothing less. You write for C-suite executives and procurement leadership who will act on your words, so accuracy matters more than narrative flair.

--- OUR COMPANY ---
{our_profile}
-------------------

{competitor_mode_instruction}

{public_sector_mode_instruction}

{brief_outcome_instruction}

EVIDENCE DISCIPLINE — THESE RULES OVERRIDE ALL OTHER INSTRUCTIONS:
- Calibrate your claims to the evidence strength. One weak or single source can only support a tentative observation. A firm conclusion requires two or more independent, corroborating sources. If the evidence is thin, say so plainly and keep the insight narrow.
- Distinguish OBSERVED FACTS (directly stated in the evidence, cited as [1], [2]) from your INFERENCES. Mark inferences with words like 'suggests', 'may', 'appears to'. Never present an inference as a fact.
- Do NOT invent. Every proper noun in your output — company names, people, part numbers, customer relationships, locations, dollar figures, deadlines — MUST appear in the evidence signals or the entity profile above. If you cannot find a real name in the evidence, do not name one. Write 'the procurement lead' or 'an engineering contact' rather than inventing 'the VP of Procurement at Siemens Munich'.
- Do NOT extrapolate a commercial consequence unless the causal chain is supported by the evidence. A single social-media post about a topic does not establish a 'strategic pivot', a 'BOM risk', a 'redesign urgency', or a supply disruption. State what was observed and, at most, what it MIGHT imply — clearly marked as speculative.
- Do NOT force a causal or counterfactual structure. Write naturally. Use 'because' only when the evidence genuinely shows causation. Use 'if' only when exploring a real risk, and only when the evidence makes it plausible. Do not pad with mandatory cause-effect or counterfactual sentences.
- It is correct and expected to conclude 'insufficient evidence to recommend a specific action' when the evidence does not support one. A restrained, honest insight is more valuable than a confident-sounding fabrication.
- Every claim must cite a numbered evidence reference [1], [2], etc. that actually supports THAT claim — not a reference to a different topic.
- Our company profile is context, not proof of fit. Do not claim our certifications, footprint, or capabilities are relevant unless the evidence explicitly names a matching procurement, hardware/equipment program, supplier qualification need, or manufacturing requirement.
- Keep direct evidence separate from inference. Do not turn minor hygiene findings such as DNS posture, missing DKIM/SPF/DMARC, or isolated lookalike domains into claims about defense-program exclusion, medical-device qualification failure, customer churn, or supply disruption unless the evidence explicitly links them.
- Do not turn generic certification or accreditation warnings, stale certificate dates, reaffirmation notices, or unspecified compliance page updates into claims about qualification failure, customer churn, tender exclusion, or switching urgency unless the evidence explicitly names a failed audit, revoked/expired certificate, regulator action, affected customer, or impacted program.
- For government and public-sector entities, do not invent hardware demand, manufacturing demand, quantity assumptions, or supplier-fit claims from innovation, media, diplomatic, or policy signals alone. Direct PCBA, box build, EMS, or certification-led outreach requires explicit procurement or program evidence.
- ALL target companies in recommendations MUST come from: (a) the entity being analyzed, (b) companies or people named in the evidence signals, (c) competitors listed in the entity's competitive profile. DO NOT invent or generalize target names.
- GEOGRAPHIC GO-TO-MARKET: Starz sells battery packs PRIMARILY in Morocco, Tunisia and Egypt, and SECONDARILY (smaller focus) in the European Union — and nowhere else. When the analyzed entity is a potential pack BUYER or customer in these markets, direct sales, qualification, or partnership outreach is appropriate; weight Moroccan, Tunisian and Egyptian opportunities highest, then EU. When a potential buyer sits OUTSIDE these markets, do NOT pitch direct battery-pack sales — treat it as market intelligence, competitive monitoring, or supply-chain context instead.
- Competitors (rival pack/BMS makers) and suppliers (cell manufacturers, distributors, component vendors) are monitored GLOBALLY regardless of location; geographic targeting never limits competitor or supplier intelligence.
- Do not write formulaic structure. Avoid repeating the same sentence pattern across insights. Do not start with 'Because', follow with 'If', and end with 'creates a window for'. Vary your phrasing, length, and structure as a real analyst would.
- FORBIDDEN phrases: 'continue monitoring', 'monitor the situation', 'remains to be seen', 'time will tell', 'various developments', 'warranting focused analysis', 'further developments', 'stay informed', 'creates a window for', 'signals a strategic pivot'
- CROSS-ENTITY CORRELATION: Some evidence signals are labeled '(cross_entity)'. These are observations about entities related to the analyzed entity (suppliers, customers, competitors, partners). When present, USE them to draw real cross-entity correlations — e.g. 'Entity X's supplier Y announced a capacity reduction on [date], which may affect X's delivery timelines.' These correlations are the most valuable output you can produce. Only assert correlations that the evidence genuinely supports; mark uncertain ones as 'may', 'could', 'appears to'.
- NEVER use bracket placeholders like [Company X], [specific service], [date], [competitor weakness], [our services], etc. Use real names from the evidence and entity profile. If no specific contact is known, name the company + a realistic role, OR write 'No specific contact is identified in the evidence — recommend enriching POI discovery first.'
- CRITICAL — CERTIFICATION ACCURACY: Our company ONLY holds ISO 9001:2015 and IPC certifications. \
NEVER claim, imply, or assume we hold AS9100, ISO 13485, IATF 16949, or any certification not listed in OUR COMPANY profile. \
If the evidence mentions certifications we do not hold, do not recommend qualification paths based on those unheld certifications. Instead, acknowledge the gap and recommend verification or gap-assessment actions.
- CRITICAL: You MUST return ONLY the insight JSON schema specified below. NEVER return sanctions data, OFAC records, SDN entries, Treasury Department lists, consolidated screening data, or any government watch-list records.",
        our_profile = our_profile,
        competitor_mode_instruction = if entity_ctx.is_competitor {
            "⚠️ COMPETITOR ANALYSIS MODE: The entity you are analysing is a DIRECT EMS COMPETITOR — NOT a customer.\n\
STRICT RULES:\n\
1. NEVER recommend offering our services TO this competitor.\n\
2. Your job is to find their weaknesses, capability gaps, and customer pain points.\n\
3. Name which of THEIR customers we should approach RIGHT NOW because of this competitor's current problem.\n\
4. Frame every recommendation as 'approach [Customer X] — [why they're at risk from this competitor's problem]'.\n\
5. Any recommendation that addresses the competitor directly is WRONG — address their downstream customers instead."
        } else if sc_role == Some("DISTRIBUTOR") {
            "📦 DISTRIBUTOR INTELLIGENCE MODE: This entity DISTRIBUTES electronic components — they are NOT an EMS prospect.\n\
STRICT RULES:\n\
1. NEVER recommend selling assembly services, proposing partnerships, or pitching qualification packages to this distributor.\n\
2. Analyse how their distribution changes (pricing, inventory, new product introductions, logistics shifts) affect OEMs and EMS companies.\n\
3. Recommend procurement hedging, alternate sourcing, or inventory actions for OUR customers based on this distributor's moves.\n\
4. Any recommendation that pitches our services TO this distributor is WRONG."
        } else if sc_role == Some("SEMICONDUCTOR") {
            "🔬 SEMICONDUCTOR INTELLIGENCE MODE: This entity designs/manufactures chips — NOT an EMS prospect.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this semiconductor company.\n\
2. NEVER pitch our certifications, facilities, consulting support, compliance packages, proposals, or generic manufacturing capabilities to this semiconductor company or treat it as a direct services lead.\n\
3. Analyse how their product launches, shortages, EOL notices, or pricing shifts affect our customers' BOMs.\n\
4. Recommend supply chain actions (alternate parts, redesign triggers, pre-buy strategies) for our customer base.\n\
5. Any recommendation that pitches assembly, qualification support, compliance support, or manufacturing to this chip company is WRONG."
        } else if sc_role == Some("PCB_MANUFACTURER") {
            "🟢 PCB MANUFACTURER INTELLIGENCE MODE: This entity makes bare PCBs — they are an upstream supplier.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this PCB manufacturer.\n\
2. Analyse their capacity, quality, lead times, and pricing for impact on our supply chain.\n\
3. Recommend sourcing qualification, dual-source strategies, or supply chain de-risking actions.\n\
4. Any recommendation that pitches our services TO this PCB company is WRONG."
        } else if sc_role == Some("TEST_MEASUREMENT") {
            "🔧 TEST EQUIPMENT INTELLIGENCE MODE: This entity makes test & measurement equipment — they are a tooling vendor.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this T&M company.\n\
2. Analyse how their product updates or pricing affect our test capabilities and manufacturing competitiveness.\n\
3. Recommend equipment investment, qualification, or capability-building actions."
        } else if sc_role == Some("TRADE_ASSOCIATION") {
            "🤝 INDUSTRY BODY INTELLIGENCE MODE: This is a trade association or industry organization — NOT a customer.\n\
STRICT RULES:\n\
1. NEVER recommend selling EMS services to this organization.\n\
2. Analyse their policy positions, events, member activities, and regulatory advocacy for market impact.\n\
3. Recommend engagement for visibility, networking, and business development — not direct sales."
        } else if sc_role == Some("DEFENSE_PRIME") {
            "🛡️ DEFENSE PRIME MODE: This is a large defense/aerospace prime contractor that subcontracts EMS work.\n\
STRICT RULES:\n\
1. Do NOT frame this entity as a generic EMS sales prospect or as an 'outsourcing opportunity'.\n\
2. Do NOT use headline language such as 'nearshore EMS opportunities' or 'EMS outsourcing opportunities'.\n\
3. Analyse program timelines, subcontractor needs, compliance requirements, offset obligations, and supply chain gaps.\n\
4. Recommend qualification paths and named programs where our capabilities (nearshore, AS9100, ITAR-free) create an advantage.\n\
5. Frame recommendations around winning Tier-2/Tier-3 subcontracting or supplier-qualification positions."
        } else if sc_role == Some("OEM") {
            "🏭 OEM / END CUSTOMER MODE: This entity designs end products and may outsource manufacturing.\n\
Analyse their outsourcing needs, product roadmap signals, supply chain vulnerabilities, and qualification requirements.\n\
Recommend specific engagement strategies to win or expand EMS contracts with them."
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
  "headline": "A factual, specific headline (<=140 chars) naming {entity_name} and stating WHAT WAS OBSERVED. Do not write an action verb ('Act on', 'Seize', 'Capture') in the headline — describe the finding.",
        "narrative": "80-280 words of evidence-grounded analysis in natural prose, written as a real analyst would. Cite evidence as [1], [2], [3]. Report what the evidence actually shows. Calibrate every claim: a single weak source supports only a tentative observation; two+ corroborating sources support a firmer conclusion; if the evidence is thin, say so. Mark inferences clearly ('suggests', 'may', 'appears'). Do NOT pad with a mandatory cause-effect or counterfactual sentence. Do NOT use section labels, template headings, or the phrases 'Because... therefore', 'If... would/could', or 'creates a window for'. Vary your sentence structure.",
        "recommendation": "1-3 suggestions in plain prose. Each must be justified by specific evidence you cite as [1], [2]. If the evidence does not support a specific commercial action, write 'Insufficient evidence to recommend a specific commercial action at this time; recommend enriching data collection on this entity before outreach.' Name ONLY companies, people, and programs that appear in the evidence or entity profile. Do not invent target names, part numbers, locations, or deadlines.",
    "confidence": 0.0,
    "severity": "<critical|high|medium|low based on likely business impact>"
}}

REQUIREMENTS:
- Reference at least 2 evidence items as [1], [2], etc.
- Each recommendation must cite at least 1 supporting evidence item as [1], [2], etc.
- Every proper noun (company, person, part number, location, standard, dollar figure, date) in your output MUST exist in the evidence or entity profile. Inventing names or figures is the most serious error you can make.
- Calibrate confidence to the actual evidence strength. One source = at most 0.4. Two corroborating sources = up to 0.6. Three or more independent sources = up to 0.85. Never claim confidence above what the evidence supports.
- Write plain business prose with complete sentences. No templates, no boilerplate labels, no heading prefixes.
- Every sentence must add new information (fact, implication, or action); do not repeat the same claim with paraphrases.
- If the evidence genuinely supports a strong commercial action, make it. If it does not, restraint is the correct answer — say so.
"#,
        category_label = category_label,
        entity_name = entity_ctx.name,
        entity_profile = entity_profile,
        evidence_text = evidence_text,
        analysis_focus = analysis_focus,
        action_focus = action_focus,
        suggestion_axes = {
            // Supply chain roles get role-specific axes regardless of category
            if entity_ctx.is_competitor {
                "competitive displacement, customer rescue, qualification wedge, pricing wedge, regional footprint positioning, executive account planning"
            } else if sc_role == Some("DISTRIBUTOR") {
                "procurement hedging, alternate sourcing, inventory pre-buy strategy, BOM cost impact, supply continuity, customer advisory"
            } else if sc_role == Some("SEMICONDUCTOR") {
                "BOM impact assessment, alternate part qualification, end-of-life migration, customer supply advisory, design-in risk, pricing trend analysis"
            } else if sc_role == Some("PCB_MANUFACTURER") {
                "PCB sourcing diversification, lead time risk mitigation, quality benchmark comparison, dual-source qualification, supply chain de-risking"
            } else if sc_role == Some("TEST_MEASUREMENT") {
                "test capability investment, equipment qualification, manufacturing competitiveness, throughput improvement, technology roadmap alignment"
            } else if sc_role == Some("TRADE_ASSOCIATION") {
                "event engagement, policy monitoring, member network mapping, standards participation, visibility building, regulatory intelligence"
            } else if is_public_sector {
                match category {
                    "brand_sentiment" => "institutional credibility assessment, procurement scrutiny mapping, stakeholder messaging review, account dependency review, scenario planning, executive briefing",
                    "regulatory_policy" | "geopolitical_analysis" => "policy impact mapping, qualification planning, account dependency review, stakeholder outreach, executive scenario planning, procurement path verification",
                    "security_compliance" | "cybersecurity_threat" | "quality_compliance" => "official-surface validation, supplier access review, containment planning, stakeholder brief, assurance planning, executive escalation",
                    _ => "account planning, stakeholder mapping, institutional process review, evidence verification, qualification planning, executive briefing",
                }
            } else if sc_role == Some("DEFENSE_PRIME") {
                "program qualification, Tier-2/Tier-3 subcontracting, offset obligations, compliance readiness, supply chain gap-fill, executive program engagement"
            } else if sc_role == Some("OEM") {
                "outsourcing capture, NPI engagement, qualification bid, design-for-manufacturing advisory, regional footprint advantage, executive sponsor mapping"
            } else {
                match category {
                    "demand_procurement" | "customer_rfq" => "revenue capture, qualification readiness, prototype or NPI entry, pricing leverage, regional footprint positioning, executive sponsor mapping",
                    "supply_chain_risk" => "continuity protection, dual-source qualification, customer assurance, design migration, regional rerouting, executive risk briefing",
                    "regulatory_policy" | "geopolitical_analysis" => "regulatory posture, export-control routing, nearshoring, customer communication, qualification planning, executive scenario planning",
                    "strategic_poi" | "talent_ip" => "POI mapping, early project engagement, competitive positioning, executive outreach, stakeholder timing",
                    "security_compliance" | "cybersecurity_threat" | "quality_compliance" => "audit readiness, security assurance, supplier governance, containment planning, customer reassurance, executive escalation",
                    _ => "revenue capture, resilience, competitive positioning, pricing leverage, regional expansion, executive planning",
                }
            }
        },
    );

    let config = InferenceConfig {
        temperature: 0.3,
        max_tokens: 3072,
        json_mode: true,
        suppress_thinking: false,
        timeout: crate::config::llm_timeout(),
        ..Default::default()
    };

    #[derive(serde::Deserialize)]
    struct LlmInsightResponse {
        headline: String,
        narrative: String,
        #[serde(default, deserialize_with = "deserialize_recommendation")]
        recommendation: Option<String>,
        confidence: f64,
        #[serde(default)]
        severity: String,
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

    fn normalize_assessment_severity(raw: &str, confidence: f64) -> &'static str {
        match raw.trim().to_ascii_lowercase().as_str() {
            "critical" => "critical",
            "high" => "high",
            "warning" | "medium" => "medium",
            "info" | "low" => "low",
            _ if confidence >= 0.8 => "critical",
            _ if confidence >= 0.7 => "high",
            _ if confidence >= 0.4 => "medium",
            _ => "low",
        }
    }

    // Ban truly formulaic/filler phrases that indicate passive analysis.
    let generic_phrases: &[&str] = &[
        "continue monitoring",
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

    let mut previous_failure_reasons: Vec<&'static str> = Vec::new();
    for attempt in 1..=*crate::config::LLM_MAX_RETRIES {
        // Exponential backoff: 0s on first attempt, 1s, 2s, 4s...
        if attempt > 1 {
            let backoff_ms = 1000u64 * (1u64 << (attempt - 2).min(4));
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        }
        let retry_guidance =
            build_llm_retry_guidance(entity_ctx, category, &previous_failure_reasons);
        let mut messages = vec![
            ChatMessage::system(system.as_str()),
            ChatMessage::user(&user),
        ];
        if let Some(guidance) = retry_guidance.as_deref() {
            messages.push(ChatMessage::user(guidance));
        }
        let mut resp = llm_client
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

        // ── SDN/OFAC contamination guard ──
        // The fine-tuned model may regurgitate sanctions data instead of insight JSON.
        // Detect this early and retry with corrective guidance.
        let raw_lower = resp.text.to_ascii_lowercase();
        let is_sdn_contaminated = raw_lower.contains("specially designated nationals")
            || raw_lower.contains("\"source\": \"specially designated")
            || raw_lower.contains("ofac")
            || raw_lower.contains("treasury department")
            || raw_lower.contains("entity_number")
            || raw_lower.contains("sdn list")
            || raw_lower.contains("consolidated screening")
            || (raw_lower.contains("\"_id\"") && raw_lower.contains("\"programs\""));

        if is_sdn_contaminated {
            tracing::warn!(
                entity = %entity_ctx.name,
                attempt,
                raw_preview = %crate::truncate_text(&resp.text, 200),
                "LLM SDN/OFAC contamination detected — model returned sanctions data instead of insight JSON"
            );
            previous_failure_reasons.push("sdn_contamination");
            crate::observability::WORKER_METRICS.record_llm_retry();
            continue;
        }

        // ── Certification invention guard ──
        // The model sometimes claims certifications (AS9100, IATF 16949, ISO 13485)
        // that our company does NOT hold. Detect this in the raw response and retry.
        // Our company ONLY holds ISO 9001:2015 and IPC.
        let raw_text = resp.text.to_ascii_lowercase();
        let is_cert_invented = {
            // Check for possessive claims: "our [forbidden_cert]", "we hold [forbidden_cert]", etc.
            let has_forbidden_cert = [
                "as9100",
                "iatf 16949",
                "iatf16949",
                "iso 13485",
                "13485 certification",
            ]
            .iter()
            .any(|c| raw_text.contains(c));
            let has_possessive_pattern = [
                "our as9100",
                "our iatf",
                "our iso 13485",
                "as9100 certification",
                "iatf 16949 certification",
                "iso 13485 certification",
                "as9100 certified",
                "iatf certified",
                "13485 certified",
                "we hold as9100",
                "we hold iatf",
                "we hold iso 13485",
                "we are as9100",
                "we are iatf",
                "we are iso 13485",
                "we have as9100",
                "we have iatf",
                "we have iso 13485",
                "our facility is as9100",
                "our facility is iatf",
                "as9100 and iso 13485",
                "iatf 16949 and ",
                "with our as9100",
                "with our iatf",
                "with our iso 13485",
            ]
            .iter()
            .any(|p| raw_text.contains(p));
            has_forbidden_cert && has_possessive_pattern
        };

        if is_cert_invented {
            if attempt < *crate::config::LLM_MAX_RETRIES {
                // First violation: give the model one corrective retry.
                tracing::warn!(
                    entity = %entity_ctx.name,
                    attempt,
                    raw_preview = %crate::truncate_text(&resp.text, 200),
                    "LLM certification invention detected — retrying with corrective guidance"
                );
                previous_failure_reasons.push("certification_invention");
                crate::observability::WORKER_METRICS.record_llm_retry();
                continue;
            } else {
                // Final attempt still violates: sanitize the forbidden
                // certification claims from the response text and proceed,
                // rather than discarding the entire insight. This avoids the
                // wasteful retry loop that blocked recipe-fire throughput on
                // entities (e.g. Sanmina) where the LLM persistently invented
                // certification claims despite corrective guidance.
                tracing::warn!(
                    entity = %entity_ctx.name,
                    attempt,
                    "LLM certification invention persists after retries — sanitizing forbidden claims and accepting output"
                );
                resp.text = sanitize_certification_claims(&resp.text);
            }
        }

        let parsed: LlmInsightResponse = match resp.parse_json() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(entity = %entity_ctx.name, attempt, error = %e, "LLM JSON parse failed");
                crate::observability::WORKER_METRICS.record_llm_retry();
                continue;
            }
        };

        let recommendation = parsed.recommendation.unwrap_or_default().trim().to_string();
        let narrative = parsed.narrative.trim().to_string();
        let headline = parsed.headline.trim().to_string();
        let role_guidance_violation =
            violates_supply_chain_role_guidance(sc_role, &headline, &narrative, &recommendation);
        let topic_alignment_violation = violates_entity_topic_alignment(
            entity_ctx,
            evidence_signals,
            &headline,
            &narrative,
            &recommendation,
        );

        let headline_lower = headline.to_lowercase();
        let narrative_lower = narrative.to_lowercase();
        let recommendation_lower = recommendation.to_lowercase();
        let is_generic =
            generic_phrases.iter().any(|p| {
                headline_lower.contains(p)
                    || narrative_lower.contains(p)
                    || recommendation_lower.contains(p)
            }) || has_formulaic_commercial_language(&headline, &narrative, &recommendation);

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
        let recommendation_has_deadline =
            recommendation_has_action_timing(category, &recommendation);
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
        let unsupported_certification_commercialization =
            has_unsupported_certification_commercialization(
                &headline,
                &narrative,
                &recommendation,
                evidence_signals,
            );
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
        let temporal_incoherence =
            has_temporal_incoherence(&entity_ctx.name, &narrative, Utc::now(), evidence_signals);
        let unnamed_customer_targeting =
            has_unnamed_customer_targeting(&narrative, &recommendation);
        let unsupported_named_target_provenance = has_unsupported_named_target_provenance(
            entity_ctx,
            &headline,
            &narrative,
            &recommendation,
            evidence_signals,
        );

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

        // Check for confidence/source boilerplate patterns (template leakage)
        let narrative_has_confidence_boilerplate = has_confidence_boilerplate(&narrative)
            || has_confidence_boilerplate(&recommendation)
            || has_confidence_boilerplate(&headline);

        let gate_decisions = vec![
            quality_gate_blocker("generic_language", is_generic, false),
            quality_gate_blocker("malformed_output", malformed, false),
            quality_gate_blocker("placeholder_output", has_placeholders, false),
            quality_gate_blocker(
                "confidence_boilerplate",
                narrative_has_confidence_boilerplate,
                true,
            ),
            quality_gate_requirement("narrative_words", words as f32, 85.0),
            quality_gate_requirement("narrative_references", reference_count as f32, 2.0),
            quality_gate_requirement(
                "recommendation_references",
                recommendation_reference_count as f32,
                1.0,
            ),
            quality_gate_requirement(
                "narrative_has_quantification",
                if has_digits { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_depth",
                recommendation.split_whitespace().count() as f32,
                12.0,
            ),
            quality_gate_requirement(
                "reasoning_depth",
                if has_reasoning_depth { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_timing",
                if recommendation_has_deadline {
                    1.0
                } else {
                    0.0
                },
                0.5,
            ),
            quality_gate_requirement(
                "narrative_readability",
                if readable_narrative { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_readability",
                if readable_recommendation { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_blocker("security_escalation", unsupported_security_escalation, true),
            quality_gate_blocker(
                "certification_escalation",
                unsupported_certification_escalation,
                false,
            ),
            quality_gate_blocker(
                "certification_commercialization",
                unsupported_certification_commercialization,
                true,
            ),
            quality_gate_blocker(
                "public_sector_commercialization",
                unsupported_public_sector_commercialization,
                true,
            ),
            quality_gate_blocker(
                "public_sector_low_usefulness",
                low_usefulness_public_sector_analysis,
                true,
            ),
            quality_gate_blocker("temporal_incoherence", temporal_incoherence, true),
            quality_gate_blocker(
                "unnamed_customer_targeting",
                unnamed_customer_targeting,
                true,
            ),
            quality_gate_blocker(
                "named_target_provenance",
                unsupported_named_target_provenance,
                true,
            ),
            quality_gate_blocker("topic_alignment", topic_alignment_violation, true),
            quality_gate_blocker("role_guidance", role_guidance_violation, true),
            quality_gate_requirement(
                "headline_length",
                if is_headline_ok { 1.0 } else { 0.0 },
                0.5,
            ),
        ];
        emit_quality_gate_decisions(
            &entity_ctx.name,
            category,
            attempt as usize,
            &gate_decisions,
        );

        let passes = quality_gate_passes_ensemble(&gate_decisions);
        let ensemble_failures = gate_decisions
            .iter()
            .filter(|decision| decision.failed && !decision.veto)
            .count();
        let veto_rejected = gate_decisions
            .iter()
            .any(|decision| decision.failed && decision.veto);

        if passes {
            crate::observability::WORKER_METRICS.record_llm_success();
            crate::observability::WORKER_METRICS.record_insight_accepted();
            let parsed_confidence = parsed.confidence.clamp(0.0, 1.0);
            let assessment_severity =
                normalize_assessment_severity(&parsed.severity, parsed_confidence);
            let mut consensus_reached = true;
            let mut dissenting_opinions = Vec::new();

            if assessment_severity == "critical" {
                for sample_idx in 1..=2 {
                    let sample_instruction = format!(
                        "Independent review sample {}. Reassess the evidence from scratch, remain evidence-bound, and return the same JSON schema.",
                        sample_idx
                    );
                    let sample_messages = vec![
                        ChatMessage::system(system.as_str()),
                        ChatMessage::user(&user),
                        ChatMessage::user(sample_instruction),
                    ];

                    match llm_client
                        .complete_with_config(sample_messages, &config)
                        .await
                    {
                        Ok(sample_resp) => match sample_resp.parse_json::<LlmInsightResponse>() {
                            Ok(sample) => {
                                let sample_confidence = sample.confidence.clamp(0.0, 1.0);
                                let sample_severity = normalize_assessment_severity(
                                    &sample.severity,
                                    sample_confidence,
                                );
                                if sample_severity != assessment_severity {
                                    consensus_reached = false;
                                    dissenting_opinions.push(serde_json::json!({
                                        "severity": sample_severity,
                                        "category": category,
                                        "confidence": sample_confidence,
                                        "rationale_summary": crate::truncate_text(
                                            sample.narrative.trim(),
                                            200,
                                        ),
                                    }));
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    entity = %entity_ctx.name,
                                    sample_idx,
                                    %error,
                                    "LLM consensus sample JSON parse failed"
                                );
                            }
                        },
                        Err(error) => {
                            tracing::warn!(
                                entity = %entity_ctx.name,
                                sample_idx,
                                %error,
                                "LLM consensus sample request failed"
                            );
                        }
                    }
                }
            }

            let metadata = serde_json::json!({
                "assessment_severity": assessment_severity,
                "assessment_category": category,
                "consensus_reached": consensus_reached,
                "dissenting_opinions": dissenting_opinions,
            });
            return Ok((
                headline,
                narrative,
                recommendation,
                parsed_confidence,
                metadata,
            ));
        }

        previous_failure_reasons.clear();
        if !has_reasoning_depth {
            previous_failure_reasons.push("reasoning");
        }
        if !recommendation_has_deadline {
            previous_failure_reasons.push("timing");
        }
        if !readable_narrative || !readable_recommendation || is_generic || malformed {
            previous_failure_reasons.push("readability");
        }
        if unnamed_customer_targeting {
            previous_failure_reasons.push("unnamed_customer_targeting");
        }
        if unsupported_named_target_provenance {
            previous_failure_reasons.push("named_target_provenance");
        }
        if unsupported_public_sector_commercialization {
            previous_failure_reasons.push("public_sector_commercialization");
        }
        if low_usefulness_public_sector_analysis {
            previous_failure_reasons.push("public_sector_low_usefulness");
        }
        if topic_alignment_violation {
            previous_failure_reasons.push("topic_alignment");
        }
        if role_guidance_violation {
            previous_failure_reasons.push("role_guidance");
        }
        if unsupported_certification_escalation {
            previous_failure_reasons.push("certification_escalation");
        }
        if unsupported_certification_commercialization {
            previous_failure_reasons.push("certification_commercialization");
        }
        if unsupported_security_escalation {
            previous_failure_reasons.push("security_escalation");
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
            unsupported_certification_commercialization,
            unsupported_public_sector_commercialization,
            low_usefulness_public_sector_analysis,
            unnamed_customer_targeting,
            unsupported_named_target_provenance,
            topic_alignment_violation,
            role_guidance_violation,
            ensemble_failures,
            veto_rejected,
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

    crate::observability::WORKER_METRICS.record_llm_failure();
    crate::observability::WORKER_METRICS.record_insight_rejected();
    anyhow::bail!(
        "LLM failed quality checks after retries for {}",
        entity_ctx.name
    )
}

#[allow(dead_code)]
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
/// Map the PascalCase transform type names used in `recipes_seed.yaml`
/// (e.g. `Lag`, `Count`, `ZScore`) to the lowercase snake-case identifiers
/// the recipe engine's `apply_transforms` dispatcher matches against.
/// Unknown inputs are passed through lowercased so the engine's identity
/// branch handles them gracefully.
fn normalize_transform_type(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "zscore" | "z_score" => "zscore".to_string(),
        "pctchange" | "pct_change" | "percentchange" => "pct_change".to_string(),
        "rollingmean" | "rolling_mean" => "rolling_mean".to_string(),
        "count" => "count".to_string(),
        "diff" | "difference" | "lag" => "diff".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

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
    let severity = if sr.category.contains("security")
        || sr.category.contains("risk")
        || sr.category.contains("supply")
        || sr.category.contains("sanction")
    {
        "warning"
    } else {
        "info"
    };
    let mut r = Recipe::new(sr.id.clone(), sr.name.clone());
    r.description = sr.category.clone();
    r.status = RecipeStatus::Seed;
    r.signals = signals;

    // ── Wire the statistical core from the seed YAML ───────────────────────
    // Previously transforms/test/thresholds were silently dropped, leaving the
    // engine to run every recipe with default (identity) transforms and a flat
    // 1.5× uplift floor. The FisherExact test type is carried through so the
    // engine and any downstream calibration know the intended test family.
    r.transforms = sr
        .transforms
        .iter()
        .map(|raw| apex_core::schemas::TransformSpec {
            // YAML uses PascalCase (Lag, Count, ZScore, PctChange, RollingMean);
            // the engine matches lowercase snake-case names. Normalize so the
            // known transforms actually apply; unknown ones fall through to the
            // engine's identity (keep-as-is) branch, which is safe.
            transform_type: normalize_transform_type(
                raw.get("type")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or(""),
            ),
            field: raw
                .get("field")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .to_string(),
            window_days: raw
                .get("window_days")
                .and_then(serde_yaml::Value::as_i64)
                .map(|d| d as i32)
                .or_else(|| {
                    raw.get("days")
                        .and_then(serde_yaml::Value::as_i64)
                        .map(|d| d as i32)
                })
                .unwrap_or(0),
            params: serde_json::to_value(raw).unwrap_or(serde_json::Value::Null),
        })
        .collect();

    r.statistical_test = sr
        .test
        .get("type")
        .and_then(serde_yaml::Value::as_str)
        .map(|test_type| apex_core::schemas::StatisticalTest {
            // Carry the declared test family (e.g. "fisher_exact",
            // "cross_correlation"). Normalized to snake_case so the engine's
            // downstream consumers can dispatch on it consistently.
            test_type: test_type.trim().to_ascii_lowercase(),
            params: serde_json::to_value(&sr.test).unwrap_or(serde_json::Value::Null),
        });

    // Thresholds: lift the recipe-specific floor/gate from the YAML so the
    // engine respects per-recipe quality bars instead of the 1.5× default.
    if let Some(min_effect) = sr
        .thresholds
        .get("min_effect")
        .and_then(serde_yaml::Value::as_f64)
    {
        // Gate validation requires min_uplift > 1.0. Guard against YAML
        // recipes that declare a min_effect at or below the baseline.
        if min_effect > 1.0 {
            r.min_uplift = min_effect;
        }
    }
    if let Some(max_p) = sr
        .thresholds
        .get("max_p_value")
        .and_then(serde_yaml::Value::as_f64)
    {
        r.max_p_value = max_p;
    }
    if let Some(min_slices) = sr
        .thresholds
        .get("min_stability")
        .and_then(serde_yaml::Value::as_f64)
        .map(|s| (s * 10.0).round() as i32)
    {
        r.min_time_slices = min_slices;
    }

    r.insight_template = sr.narrative_template.clone();
    r.action_template = action;
    r.severity = severity.to_string();
    r.category = sr.category.clone();
    r.min_uplift = 1.0;
    r.min_entities = 1;
    r
}

#[tokio::main]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(600))
        .max_lifetime(std::time::Duration::from_secs(1800))
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

    // Migrations normally run at API startup, but the worker can start first
    // (independent systemd units) and job handlers now write to schema added
    // by late migrations (e.g. triage_queue merge fields, migration 053).
    // Best-effort: a failure (permissions, checksum mismatch with
    // APEX_SKIP_MIGRATIONS semantics) must not take the worker down.
    match store.run_migrations().await {
        Ok(()) => tracing::info!("worker startup: database migrations applied"),
        Err(e) => tracing::warn!(
            error = %e,
            "worker startup: database migrations failed; continuing (some features may be degraded)"
        ),
    }

    // ─── Liveness heartbeat (migration 049) ───────────────────────────────
    // Health checks read `service_heartbeats.last_seen_at` to distinguish a
    // live worker from one that silently stopped; write every ~30s so
    // staleness is a measurement, not an assumption. The first tick fires
    // immediately, recording a heartbeat at startup.
    {
        let heartbeat_store = Arc::clone(&store);
        tokio::spawn(async move {
            let instance_id = std::env::var("APEX_INSTANCE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    let host =
                        std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
                    format!("{host}-{}", std::process::id())
                });
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if let Err(error) = heartbeat_store
                    .record_service_heartbeat("worker", &instance_id, env!("CARGO_PKG_VERSION"))
                    .await
                {
                    tracing::warn!(error = %error, "failed to record worker heartbeat");
                }
            }
        });
        tracing::info!("worker heartbeat task started");
    }

    // Create the shared ActivityLogger for recording system events
    // to the activity_feed table across all pipeline stages.
    let _activity_logger = ActivityLogger::new(pool.clone());
    tracing::info!("activity_logger initialized");

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
    // B324: SIGTERM is the signal Docker/K8s/systemd send on stop — the
    // previous loop only listened for SIGINT (ctrl_c), so containerized
    // workers were killed outright mid-job with no drain.
    let mut sigterm = {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).expect("failed to install SIGTERM handler")
    };
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
                    // Job-level panics are contained inside tick_scheduler
                    // (each job runs in an observed spawn, B325), so the tick
                    // body itself only does bookkeeping.
                    runtime::tick_scheduler(&mut scheduler, &store).await;
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
                tracing::info!("received SIGINT, exiting gracefully");
                break;
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, exiting gracefully");
                break;
            }
        }
    }
    pool.close().await;
    tracing::info!("database pool closed");
    Ok(())
}

#[tracing::instrument(skip(scheduler, store))]
async fn tick_scheduler(scheduler: &mut Scheduler, store: &Arc<PgStore>) {
    runtime::tick_scheduler(scheduler, store).await;
}

fn parse_digest_recipients(raw: &str) -> Vec<String> {
    raw.split([',', ';', '\n'])
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

#[cfg(feature = "llm")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct QualityGateGoldenSetDisagreement {
    source_id: Uuid,
    content_type: String,
    title: String,
    historical_label: HistoricalQualityGateLabel,
    predicted_label: HistoricalQualityGateLabel,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct QualityGateGoldenSetRegressionResult {
    total_examples: usize,
    accepted_examples: usize,
    rejected_examples: usize,
    agreement: f64,
    disagreements: Vec<QualityGateGoldenSetDisagreement>,
}

#[cfg(feature = "llm")]
fn evaluate_quality_gate_golden_set(
    examples: &[QualityGateGoldenSetExample],
) -> QualityGateGoldenSetRegressionResult {
    let mut disagreements = Vec::new();
    let mut accepted_examples = 0usize;
    let mut rejected_examples = 0usize;

    for example in examples {
        match example.historical_label {
            HistoricalQualityGateLabel::Accepted => accepted_examples += 1,
            HistoricalQualityGateLabel::Rejected => rejected_examples += 1,
        }

        let predicted_label = if passes_shared_insight_quality_gate(
            &example.title,
            &example.body,
            Some(example.content_type.as_str()),
        ) {
            HistoricalQualityGateLabel::Accepted
        } else {
            HistoricalQualityGateLabel::Rejected
        };

        if predicted_label != example.historical_label {
            disagreements.push(QualityGateGoldenSetDisagreement {
                source_id: example.source_id,
                content_type: example.content_type.clone(),
                title: example.title.clone(),
                historical_label: example.historical_label,
                predicted_label,
            });
        }
    }

    let total_examples = examples.len();
    let agreement = if total_examples == 0 {
        0.0
    } else {
        1.0 - (disagreements.len() as f64 / total_examples as f64)
    };

    QualityGateGoldenSetRegressionResult {
        total_examples,
        accepted_examples,
        rejected_examples,
        agreement,
        disagreements,
    }
}

#[cfg(feature = "llm")]
async fn run_quality_gate_golden_set_regression(
    store: &PgStore,
) -> anyhow::Result<QualityGateGoldenSetRegressionResult> {
    let accepted_limit = std::env::var("LLM_GOLDEN_SET_ACCEPTED_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(50);
    let rejected_limit = std::env::var("LLM_GOLDEN_SET_REJECTED_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(50);
    let agreement_target = std::env::var("LLM_GOLDEN_SET_MIN_AGREEMENT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.95)
        .clamp(0.0, 1.0);

    let export = store
        .export_quality_gate_reviewed_warning_golden_set(accepted_limit, rejected_limit)
        .await
        .context("quality gate golden set export failed")?;
    if export.examples.is_empty() {
        tracing::info!(
            "quality_gate_golden_set: no reviewed warnings yet, skipping regression check"
        );
        return Ok(QualityGateGoldenSetRegressionResult {
            total_examples: 0,
            accepted_examples: 0,
            rejected_examples: 0,
            agreement: 1.0,
            disagreements: Vec::new(),
        });
    }
    let regression = evaluate_quality_gate_golden_set(&export.examples);
    let metrics = serde_json::json!({
        "dataset_id": export.dataset_id,
        "dataset_name": export.dataset_name,
        "dataset_version": export.dataset_version,
        "agreement": regression.agreement,
        "agreement_target": agreement_target,
        "total_examples": regression.total_examples,
        "accepted_examples": regression.accepted_examples,
        "rejected_examples": regression.rejected_examples,
        "disagreement_count": regression.disagreements.len(),
    });
    let artifacts = serde_json::json!({
        "disagreements": regression
            .disagreements
            .iter()
            .take(20)
            .map(|item| serde_json::json!({
                "source_id": item.source_id,
                "content_type": item.content_type,
                "title": item.title,
                "historical_label": item.historical_label.as_str(),
                "predicted_label": item.predicted_label.as_str(),
            }))
            .collect::<Vec<_>>(),
    });
    store
        .record_llm_improvement_run(
            "quality_gate_golden_set_regression",
            &export.dataset_version,
            &metrics,
            &artifacts,
        )
        .await
        .context("quality gate golden set regression persistence failed")?;

    if !regression.disagreements.is_empty() {
        let severity = if regression.agreement + f64::EPSILON < agreement_target {
            "critical"
        } else {
            "high"
        };
        let desc = format!(
            "Quality-gate golden-set disagreement detected. dataset={} agreement={:.1}% target={:.1}% disagreements={}/{}.",
            export.dataset_version,
            regression.agreement * 100.0,
            agreement_target * 100.0,
            regression.disagreements.len(),
            regression.total_examples,
        );
        match store
            .insert_warning(
                "llm_quality_gate_golden_set_review",
                "LLM quality-gate golden set review required",
                Some(&desc),
                severity,
                Some("global"),
                None,
                None,
                None,
                Some((1.0 - regression.agreement).clamp(0.0, 1.0)),
            )
            .await
        {
            Ok(warning_id) => tracing::warn!(
                warning_id = %warning_id,
                dataset_version = %export.dataset_version,
                disagreements = regression.disagreements.len(),
                agreement = regression.agreement,
                "quality_gate_golden_set: review warning inserted"
            ),
            Err(error) => tracing::error!(
                %error,
                dataset_version = %export.dataset_version,
                disagreements = regression.disagreements.len(),
                "quality_gate_golden_set: failed to insert review warning"
            ),
        }
    }

    if regression.agreement + f64::EPSILON < agreement_target {
        anyhow::bail!(
            "quality gate golden set agreement {:.3} below threshold {:.3}",
            regression.agreement,
            agreement_target
        );
    }

    Ok(regression)
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
                    title_sim >= *config::DEDUP_TITLE_THRESHOLD
                        && summary_sim >= *config::DEDUP_SUMMARY_THRESHOLD
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

        // B333: one failing recipient previously aborted the whole loop —
        // every user after the failure lost their digest that cycle. Log and
        // continue; only a send that succeeded marks the digest as sent.
        if let Err(error) = send_digest_email(&recipients, &subject, html, text).await {
            tracing::error!(user_id = %user_id, %error, "email digest send failed");
            continue;
        }
        if let Err(error) = store.mark_email_digest_sent(&user_id, now_utc).await {
            tracing::warn!(user_id = %user_id, %error, "mark_email_digest_sent failed (digest may re-send)");
        }
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
    let seed_is_competitor = parent_seed.map(|seed| seed.is_competitor).unwrap_or(false);
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
            if seed_is_competitor {
                persist_discovered_company_context(store, &existing, disc, parent_seed, now)
                    .await?;
            }
            return Ok(Some(existing.id));
        }
    }

    if let Some(org_name) = inferred_org {
        if let Some(existing) = store.get_company_by_name_ci(org_name).await? {
            if seed_is_competitor {
                persist_discovered_company_context(store, &existing, disc, parent_seed, now)
                    .await?;
            }
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
            "seed_is_competitor": seed_is_competitor,
            "is_competitor": seed_is_competitor,
            "discovery_track": if seed_is_competitor { "competitor" } else { "partner_or_prospect" },
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

#[cfg(feature = "llm")]
async fn persist_discovered_company_context(
    store: &PgStore,
    existing: &apex_store::postgres::CompanyRow,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let seed_is_competitor = parent_seed.map(|seed| seed.is_competitor).unwrap_or(false);
    let existing_is_competitor = existing
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let Some(mut metadata) = existing.metadata.clone() else {
        let mut empty = serde_json::Map::new();
        empty.insert(
            "discovered_via".to_string(),
            serde_json::Value::String("poi_discovery".to_string()),
        );
        update_existing_company_context(
            store,
            existing,
            serde_json::Value::Object(empty),
            disc,
            parent_seed,
            seed_is_competitor,
            now,
        )
        .await?;
        return Ok(());
    };

    if !seed_is_competitor && existing_is_competitor {
        return Ok(());
    }

    update_existing_company_context(
        store,
        existing,
        metadata.take(),
        disc,
        parent_seed,
        seed_is_competitor,
        now,
    )
    .await
}

#[cfg(feature = "llm")]
async fn update_existing_company_context(
    store: &PgStore,
    existing: &apex_store::postgres::CompanyRow,
    metadata: serde_json::Value,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    seed_is_competitor: bool,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let mut metadata_obj = metadata.as_object().cloned().unwrap_or_default();
    metadata_obj.insert(
        "discovered_via".to_string(),
        serde_json::Value::String("poi_discovery".to_string()),
    );
    metadata_obj.insert(
        "discovery_method".to_string(),
        serde_json::Value::String(disc.discovery_method.clone()),
    );
    metadata_obj.insert(
        "source_url".to_string(),
        serde_json::Value::String(disc.source_url.clone()),
    );
    metadata_obj.insert(
        "seed_person_id".to_string(),
        serde_json::Value::String(disc.seed_person_id.to_string()),
    );
    metadata_obj.insert(
        "confidence".to_string(),
        serde_json::Value::from(disc.confidence as f64),
    );
    if let Some(seed) = parent_seed {
        metadata_obj.insert(
            "seed_org_name".to_string(),
            serde_json::Value::String(seed.org_name.clone()),
        );
        if let Some(seed_org_id) = seed.primary_org_id {
            metadata_obj.insert(
                "seed_org_id".to_string(),
                serde_json::Value::String(seed_org_id.to_string()),
            );
        }
    }
    if seed_is_competitor {
        metadata_obj.insert(
            "seed_is_competitor".to_string(),
            serde_json::Value::Bool(true),
        );
        metadata_obj.insert("is_competitor".to_string(), serde_json::Value::Bool(true));
        metadata_obj.insert(
            "discovery_track".to_string(),
            serde_json::Value::String("competitor".to_string()),
        );
    } else {
        metadata_obj
            .entry("seed_is_competitor".to_string())
            .or_insert(serde_json::Value::Bool(false));
        metadata_obj
            .entry("discovery_track".to_string())
            .or_insert_with(|| serde_json::Value::String("partner_or_prospect".to_string()));
    }

    let mut company = Company::new(
        existing.name.clone(),
        CompanyType::from_str(existing.company_type.as_deref().unwrap_or("other")),
    );
    company.id = existing.id;
    company.legal_name = existing.legal_name.clone();
    company.domain = existing.domain.clone();
    company.country_code = existing.country_code.clone();
    company.region = existing.region.clone();
    company.industry_tags = existing.industry_tags.clone().unwrap_or_default();
    company.employee_estimate = existing.employee_estimate;
    company.revenue_estimate_usd = existing.revenue_estimate_usd;
    company.risk_score = existing.risk_score.unwrap_or(0.0);
    company.threat_score = existing.threat_score.unwrap_or(0.0);
    company.overlap_score = existing.overlap_score.unwrap_or(0.0);
    company.strategic_relevance = existing.strategic_relevance.unwrap_or(0.0);
    company.metadata = serde_json::Value::Object(metadata_obj);
    company.created_at = existing.created_at.unwrap_or(now);
    company.updated_at = now;
    store.insert_company(&company).await?;
    tracing::info!(company = %company.name, competitor = seed_is_competitor, "poi_discovery: refreshed company discovery context");
    Ok(())
}

/// Classify an inferred role title into a canonical `RoleFamily`.
#[cfg(feature = "llm")]
fn classify_role_family(role: Option<&str>) -> RoleFamily {
    let r = match role {
        Some(s) if !s.is_empty() => s.to_lowercase(),
        _ => return RoleFamily::Other("Unknown".to_string()),
    };

    if looks_like_buyer_candidate_role(Some(&r)) {
        return RoleFamily::Procurement;
    }

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
        if looks_like_buyer_candidate_role(Some(&r)) {
            return RoleFamily::Procurement;
        }
        if r.contains("operation") || r.contains("supply chain") || r.contains("manufacturing") {
            return RoleFamily::Operations;
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

#[cfg(feature = "llm")]
fn looks_like_buyer_candidate_role(role: Option<&str>) -> bool {
    let Some(role) = role else {
        return false;
    };

    let lower = role.to_lowercase();
    lower.contains("buyer")
        || lower.contains("procurement")
        || lower.contains("purchas")
        || lower.contains("sourcing")
        || lower.contains("supply chain")
        || lower.contains("commodity")
        || lower.contains("category manager")
        || lower.contains("vendor management")
        || lower.contains("supplier diversity")
        || lower.contains("supply planning")
        || lower.contains("inventory")
        || lower.contains("approvisionnement")
        || lower.contains("achat")
        || lower.contains("achats")
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
2) Is this person a TARGET business/government decision-maker profile?

Target profiles:
- Government: deputy/director-general, department directors, program/policy directors/managers,
  procurement/acquisition officials, licensing/regulatory/compliance leaders in ministries/agencies.
- Company: C-suite, presidents, board members, general managers, heads, vice presidents,
  and procurement/purchasing/sourcing/category/commodity, operations, supply-chain, quality,
  engineering, legal, compliance, security, and finance decision-makers.

Non-target profiles (reject):
- Investor relations contacts, press/media contacts, recruiters, HR, marketing, sales, support,
  assistants, coordinators, and generic contact-page staff.
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

    if !parsed.is_person || !parsed.target_fit.unwrap_or(false) {
        return Ok(None);
    }

    // Return validated/enriched candidate
    let mut validated = candidate.clone();
    let name_rewritten = if let Some(ref name) = parsed.name {
        !name.is_empty() && name.trim() != candidate.name.trim()
    } else {
        false
    };
    if let Some(name) = parsed.name {
        if !name.is_empty() {
            validated.name = name;
        }
    }
    // Post-LLM backstop: the model can rewrite the name (correcting spelling,
    // expanding initials, etc.), but it can also produce a non-person string
    // (a role title, a place name, a transliterated phrase). Re-run the
    // deterministic junk check on the final name before accepting it. This
    // guards against LLM hallucination/over-correction that reintroduces the
    // exact junk classes the pre-LLM filter is designed to reject.
    if !apex_core::person_names::looks_like_person_name(&validated.name) {
        tracing::info!(
            original_name = %candidate.name,
            final_name = %validated.name,
            name_rewritten,
            "poi_validation: post-LLM backstop rejected non-person-like name"
        );
        return Ok(None);
    }
    let sanitized_org = sanitize_validated_org(candidate, parsed.org);
    if let Some(role) = sanitize_validated_role(candidate, parsed.role, sanitized_org.as_deref()) {
        validated.inferred_role = Some(role);
    }
    if let Some(org) = sanitized_org {
        validated.inferred_org = Some(org);
    }

    Ok(Some(validated))
}

#[cfg(feature = "llm")]
fn sanitize_validated_role(
    candidate: &DiscoveredPoi,
    parsed_role: Option<String>,
    parsed_org: Option<&str>,
) -> Option<String> {
    let role = sanitize_llm_field(parsed_role)?;
    if !looks_like_decision_role_label(&role) {
        return None;
    }

    let normalized_role = normalize_company_name(&role);
    if normalized_role == normalize_company_name("ApexIntel") {
        return None;
    }
    if let Some(existing_org) = candidate.inferred_org.as_deref() {
        if normalized_role == normalize_company_name(existing_org) {
            return None;
        }
    }
    if let Some(org) = parsed_org {
        if normalized_role == normalize_company_name(org) {
            return None;
        }
    }

    Some(role)
}

#[cfg(feature = "llm")]
fn sanitize_validated_org(candidate: &DiscoveredPoi, parsed_org: Option<String>) -> Option<String> {
    let org = sanitize_llm_field(parsed_org)?;
    let normalized_org = normalize_company_name(&org);
    if normalized_org.is_empty() || normalized_org == normalize_company_name("ApexIntel") {
        return None;
    }
    if normalize_company_name(&candidate.name) == normalized_org {
        return None;
    }
    if looks_like_decision_role_label(&org) {
        return None;
    }
    Some(org)
}

#[cfg(feature = "llm")]
fn sanitize_llm_field(value: Option<String>) -> Option<String> {
    let cleaned = value?.trim().trim_matches('"').to_string();
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_ascii_lowercase();
    if matches!(lower.as_str(), "unknown" | "null" | "none" | "n/a") {
        return None;
    }
    Some(cleaned)
}

#[cfg(feature = "llm")]
fn looks_like_decision_role_label(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.len() < 4 || lower.len() > 120 {
        return false;
    }

    let junk = [
        "investor relations",
        "media",
        "press",
        "communications",
        "marketing",
        "sales",
        "business development",
        "customer service",
        "support",
        "assistant",
        "coordinator",
        "specialist",
        "analyst",
        "recruiter",
        "human resources",
        "administrator",
        "receptionist",
    ];
    if junk.iter().any(|needle| lower.contains(needle)) {
        return false;
    }

    let role_keywords = [
        "chief",
        "ceo",
        "cfo",
        "cto",
        "coo",
        "president",
        "vice president",
        "vp",
        "head",
        "director",
        "manager",
        "officer",
        "chair",
        "board",
        "founder",
        "owner",
        "general manager",
        "procurement",
        "purchasing",
        "sourcing",
        "supply chain",
        "operations",
        "engineering",
        "quality",
        "compliance",
        "regulatory",
        "legal",
        "security",
        "finance",
        "strategy",
        "commercial",
        "program",
        "policy",
        "acquisition",
        "contracts",
    ];

    role_keywords.iter().any(|needle| lower.contains(needle))
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
    llm_config.api_key = std::env::var("LLM_API_KEY")
        .ok()
        .map(apex_llm::ApiKeySecret::from);
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
        tracing::info!(
            "self_improvement_cycle: llm eval report passed quality gate; storing governance artifact only"
        );
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
        tracing::info!(
            "self_improvement_cycle: continuous improvement summary passed quality gate; storing governance artifact only"
        );
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

    let golden_set_regression = run_quality_gate_golden_set_regression(store)
        .await
        .context("llm self-improvement: quality gate golden set regression failed")?;
    tracing::info!(
        agreement = golden_set_regression.agreement,
        total_examples = golden_set_regression.total_examples,
        disagreements = golden_set_regression.disagreements.len(),
        "self_improvement_cycle: quality gate golden set regression completed"
    );

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
        let has_evidence = raw.meta.contains_key("domain") || raw.meta.contains_key("source");
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

#[allow(dead_code)] // utility prepared for nightly pipeline consumption
async fn load_nightly_inputs() -> Result<NightlyInputs> {
    let path = std::env::var("NIGHTLY_INPUT_PATH")
        .unwrap_or_else(|_| "runtime/nightly_inputs.json".to_string());
    let content = read_file_with_size_check(&path).await?;
    let payload = serde_json::from_str::<NightlyInputs>(&content)?;
    Ok(payload)
}

#[allow(dead_code)]
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

/// Homoglyph mappings for lookalike generation.
/// Maps ASCII chars to visually similar Unicode chars.
const HOMOGLYPHS: &[(&str, &[&str])] = &[
    ("a", &["à", "á", "â", "ã", "ä", "å", "ɑ", "а"]),
    ("c", &["ç", "ć", "č", "с"]),
    ("d", &["đ", "ð"]),
    ("e", &["è", "é", "ê", "ë", "ε", "е"]),
    ("g", &["ğ", "ɡ"]),
    ("h", &["һ"]),
    ("i", &["ì", "í", "î", "ï", "ı", "і"]),
    ("l", &["ł", "ɫ", "1"]),
    ("n", &["ñ", "ŋ"]),
    ("o", &["ò", "ó", "ô", "õ", "ö", "ø", "0", "о"]),
    ("r", &["ŗ", "г"]),
    ("s", &["ş", "š", "ś", "ѕ"]),
    ("t", &["ţ", "ŧ"]),
    ("u", &["ù", "ú", "û", "ü", "µ"]),
    ("w", &["ŵ", "ω"]),
    ("y", &["ý", "ÿ", "ŷ", "у"]),
    ("z", &["ž", "ż", "ź"]),
];

/// TLD swap mappings — common typosquat TLD alternatives.
const TLD_SWAPS: &[(&str, &[&str])] = &[
    (
        ".com",
        &[".co", ".cm", ".corn", ".om", ".com.co", ".net", ".org"],
    ),
    (".net", &[".ner", ".met", ".org"]),
    (".org", &[".orq", ".og", ".net"]),
    (".co.uk", &[".co.ck", ".co.uk.com"]),
    (".de", &[".d3", ".de.com"]),
    (".fr", &[".f", ".fr.com"]),
    (".tn", &[".tn.com", ".rn"]),
];

/// Generate common typosquat/lookalike variants for a domain name.
/// Covers: transposition, character omission, character doubling,
/// homoglyph substitution, hyphen insertion, and TLD swaps.
fn generate_typosquat_variants(domain: &str) -> Vec<String> {
    // Handle multi-part TLDs (co.uk, com.au, co.jp, etc.)
    let multi_tlds = &[
        ".co.uk", ".com.au", ".co.jp", ".co.nz", ".com.br", ".com.mx", ".co.za", ".com.ar",
        ".com.tn", ".net.au", ".org.uk", ".ac.uk", ".gov.uk",
    ];
    let (sld_str, tld_str) = {
        let lower = domain.to_ascii_lowercase();
        let mut found = None;
        for mtld in multi_tlds {
            if lower.ends_with(mtld) {
                let base = &lower[..lower.len() - mtld.len()];
                found = Some((base.to_string(), mtld.to_string()));
                break;
            }
        }
        match found {
            Some((b, t)) => (b, t),
            None => {
                // Single TLD — split at last dot
                let parts: Vec<&str> = domain.rsplitn(2, '.').collect();
                if parts.len() < 2 {
                    return vec![];
                }
                (parts[1].to_string(), format!(".{}", parts[0]))
            }
        }
    };

    let sld: Vec<char> = sld_str.chars().collect();
    let n = sld.len();
    if n == 0 {
        return vec![];
    }
    let mut variants = std::collections::HashSet::new();

    // 1. Transposition: swap adjacent chars
    for i in 0..n.saturating_sub(1) {
        let mut v = sld.clone();
        v.swap(i, i + 1);
        let s: String = v.iter().collect();
        variants.insert(format!("{}{}", s, tld_str));
    }

    // 2. Omission: drop each char
    for i in 0..n {
        let s: String = sld
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, c)| c)
            .collect();
        if !s.is_empty() {
            variants.insert(format!("{}{}", s, tld_str));
        }
    }

    // 3. Character doubling (insert adjacent duplicate)
    for i in 0..n {
        let mut v: Vec<char> = sld.clone();
        v.insert(i, sld[i]);
        let s: String = v.iter().collect();
        variants.insert(format!("{}{}", s, tld_str));
    }

    // 4. Homoglyph substitution
    for (i, ch) in sld.iter().enumerate() {
        let lower_ch = ch.to_ascii_lowercase();
        for &(ascii, glyphs) in HOMOGLYPHS {
            if ascii.as_bytes() == [lower_ch as u8] {
                for &glyph in glyphs {
                    let mut v: Vec<char> = sld.clone();
                    v[i] = glyph.chars().next().unwrap_or(*ch);
                    let s: String = v.iter().collect();
                    variants.insert(format!("{}{}", s, tld_str));
                }
            }
        }
    }

    // 5. Hyphen insertion
    for i in 1..n {
        let (a, b): (String, String) = (sld[..i].iter().collect(), sld[i..].iter().collect());
        variants.insert(format!("{}-{}{}", a, b, tld_str));
    }

    // 6. TLD swap
    for &(tld_pattern, alts) in TLD_SWAPS {
        if tld_str == tld_pattern {
            for alt in alts {
                variants.insert(format!("{}{}", sld_str, alt));
            }
        }
    }

    // Cap at a generous limit — most domains will generate 30-80 variants
    variants.into_iter().take(100).collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default,
        clippy::absurd_extreme_comparisons
    )]

    use super::*;
    #[cfg(feature = "llm")]
    use crate::llm_orchestration::{summarize_quality_gate_reviews, QualityGateReview};

    #[cfg(feature = "llm")]
    #[test]
    fn classify_role_family_detects_supply_chain_buyers() {
        assert_eq!(
            classify_role_family(Some("Senior Buyer")),
            RoleFamily::Procurement
        );
        assert_eq!(
            classify_role_family(Some("Head of Supply Chain")),
            RoleFamily::Procurement
        );
        assert_eq!(
            classify_role_family(Some("Category Manager, Packaging")),
            RoleFamily::Procurement
        );
        assert_eq!(
            classify_role_family(Some("Responsable Achats")),
            RoleFamily::Procurement
        );
    }

    #[cfg(feature = "llm")]
    fn matrix_entity_context(public_sector: bool) -> EntityContext {
        EntityContext {
            name: if public_sector {
                "European Commission".to_string()
            } else {
                "Key Tronic Corporation".to_string()
            },
            region: if public_sector {
                "Europe".to_string()
            } else {
                "North America".to_string()
            },
            entity_type: Some(if public_sector {
                "Government".to_string()
            } else {
                "EMS".to_string()
            }),
            is_competitor: false,
            industry_tags: vec![],
            certifications: vec!["AS9100".to_string(), "ISO 13485".to_string()],
            capabilities: vec!["PCBA".to_string()],
            key_persons: vec!["Procurement Lead: Jane Doe".to_string()],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: Some(if public_sector {
                "ec.europa.eu".to_string()
            } else {
                "keytronic.com".to_string()
            }),
        }
    }

    #[cfg(feature = "llm")]
    #[test]
    fn inferred_supply_chain_role_detects_semiconductor_from_context() {
        let entity_ctx = EntityContext {
            name: "Microchip Technology".to_string(),
            region: "US".to_string(),
            entity_type: Some("OEM".to_string()),
            is_competitor: false,
            industry_tags: vec!["semiconductor".to_string(), "automotive".to_string()],
            certifications: vec![],
            capabilities: vec![
                "MCU portfolio".to_string(),
                "power management ICs".to_string(),
            ],
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
            domain: None,
        };

        assert_eq!(
            inferred_supply_chain_role(&entity_ctx),
            Some("SEMICONDUCTOR")
        );
    }

    #[cfg(feature = "llm")]
    #[test]
    fn role_guidance_rejects_semiconductor_ems_pitch_language() {
        assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "Microchip Technology's CEO shift opens EMS outsourcing opportunities",
            "Microchip Technology is a chip supplier, but this creates nearshore EMS opportunities for us [1].",
            "Offer assembly services to Microchip procurement [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn role_guidance_rejects_semiconductor_proposal_pitch_language() {
        assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NXP Semiconductors faces supply chain risks due to regulatory changes",
            "NXP Semiconductors is navigating regulatory shifts that could affect procurement, creating an opportunity for our company to position itself as a reliable partner in maintaining compliance.",
            "Reach out to NXP's operations leadership within 30 days and prepare a detailed proposal highlighting our ISO 14001 certification and green manufacturing capabilities.",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn role_guidance_rejects_semiconductor_qualification_support_pitch_language() {
        assert!(violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NVIDIA's AS9100 gap risks aerospace contracts",
            "NVIDIA's AS9100 compliance gap creates a direct opportunity for our aerospace-focused EMS services because defense-adjacent buyers may need alternative qualification paths.",
            "Offer expedited qualification support using our Tunisia and Morocco facilities before Q3 2026 [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn role_guidance_allows_semiconductor_customer_supply_advisory() {
        assert!(!violates_supply_chain_role_guidance(
            Some("SEMICONDUCTOR"),
            "NXP regulatory update may affect automotive sourcing windows",
            "The regulatory shift could narrow documentation timing for automotive OEMs that depend on NXP MCUs [1].",
            "Advise affected automotive customers to review buffer stock, alternate-part qualification, and redesign triggers before the next sourcing cycle [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn inferred_supply_chain_role_detects_defense_prime_from_context() {
        let entity_ctx = EntityContext {
            name: "BAE Systems".to_string(),
            region: "UK".to_string(),
            entity_type: Some("company".to_string()),
            is_competitor: false,
            industry_tags: vec!["aerospace and defense".to_string()],
            certifications: vec!["AS9100D".to_string()],
            capabilities: vec!["munitions systems".to_string()],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec!["Glascoed munitions facility".to_string()],
            competitor_events: vec![],
            domain: None,
        };

        assert_eq!(
            inferred_supply_chain_role(&entity_ctx),
            Some("DEFENSE_PRIME")
        );
    }

    #[cfg(feature = "llm")]
    #[test]
    fn role_guidance_rejects_defense_prime_generic_ems_opportunity_language() {
        assert!(violates_supply_chain_role_guidance(
            Some("DEFENSE_PRIME"),
            "BAE Systems' munitions factory delay opens nearshore EMS opportunities",
            "The delay creates an EMS opportunity for direct outreach to BAE Systems [1].",
            "Position nearshore EMS capacity to BAE Systems immediately [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn topic_alignment_rejects_unsupported_oil_narrative() {
        let entity_ctx = EntityContext {
            name: "NVIDIA".to_string(),
            region: "US".to_string(),
            entity_type: Some("semiconductor".to_string()),
            is_competitor: false,
            industry_tags: vec!["semiconductor".to_string(), "ai".to_string()],
            certifications: vec![],
            capabilities: vec![
                "GPU platforms".to_string(),
                "data center accelerators".to_string(),
            ],
            key_persons: vec![],
            recent_changes: vec!["Blackwell server launch".to_string()],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: None,
        };
        let evidence_signals = vec![EvidenceSignal {
            title: "NVIDIA expands AI server program".to_string(),
            description:
                "Hyperscaler demand is lifting GPU server deployments and data center capacity planning."
                    .to_string(),
            source_url: "https://example.com/nvidia".to_string(),
            signal_type: "news".to_string(),
            extracted_facts: vec!["GPU server".to_string(), "data center".to_string()],
            date_context: Some("2026-03-18".to_string()),
            relevance_score: 1.0,
        }];

        assert!(violates_entity_topic_alignment(
            &entity_ctx,
            &evidence_signals,
            "Middle East oil surge changes NVIDIA demand",
            "NVIDIA now sits in the path of a Middle East oil surge and refinery spending wave that will reshape its sales mix [1] [2].",
            "Brief the account team on oil price and refinery demand exposure within 30 days [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn topic_alignment_allows_supported_semiconductor_narrative() {
        let entity_ctx = EntityContext {
            name: "NVIDIA".to_string(),
            region: "US".to_string(),
            entity_type: Some("semiconductor".to_string()),
            is_competitor: false,
            industry_tags: vec!["semiconductor".to_string(), "ai".to_string()],
            certifications: vec![],
            capabilities: vec![
                "GPU platforms".to_string(),
                "data center accelerators".to_string(),
            ],
            key_persons: vec![],
            recent_changes: vec!["Blackwell server launch".to_string()],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec![],
            sites_summary: vec![],
            competitor_events: vec![],
            domain: None,
        };
        let evidence_signals = vec![EvidenceSignal {
            title: "NVIDIA expands AI server program".to_string(),
            description:
                "Hyperscaler demand is lifting GPU server deployments and data center capacity planning."
                    .to_string(),
            source_url: "https://example.com/nvidia".to_string(),
            signal_type: "news".to_string(),
            extracted_facts: vec!["GPU server".to_string(), "data center".to_string()],
            date_context: Some("2026-03-18".to_string()),
            relevance_score: 1.0,
        }];

        assert!(!violates_entity_topic_alignment(
            &entity_ctx,
            &evidence_signals,
            "AI server demand lifts NVIDIA planning urgency",
            "NVIDIA is seeing stronger data center demand because hyperscalers are accelerating GPU server deployment windows [1] [2].",
            "Prepare a response plan for the next data center qualification cycle within 30 days [1].",
        ));
    }

    #[cfg(feature = "llm")]
    fn matrix_signal(category: &str, public_sector: bool) -> Vec<EvidenceSignal> {
        if public_sector {
            return vec![EvidenceSignal {
                title: "Trade defense consultation notice".to_string(),
                description: "The European Commission published a consultation notice affecting supplier qualification timing, stakeholder review, and account dependency planning for impacted programs.".to_string(),
                source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
                signal_type: "regulatory".to_string(),
                extracted_facts: vec![
                    "consultation notice".to_string(),
                    "supplier qualification".to_string(),
                    "approval timing".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            }];
        }

        let (title, description, signal_type, facts) = match category {
            "demand_procurement" => (
                "OEM sourcing review launched",
                "The OEM opened a sourcing review for medical and industrial assemblies with qualification timing noted for Q2 2026.",
                "procurement",
                vec!["Q2 2026".to_string(), "sourcing review".to_string()],
            ),
            "supply_chain_risk" => (
                "Supplier delay hits production schedule",
                "A component delay is extending lead times by 14 days and creating continuity pressure for regulated programs.",
                "warning",
                vec!["14 days".to_string(), "continuity pressure".to_string()],
            ),
            "security_compliance" => (
                "Certification warning posted",
                "A compliance warning and certification issue were posted on the capabilities page without a named audit failure.",
                "certification",
                vec!["compliance warning".to_string(), "certification issue".to_string()],
            ),
            "competitor_market" => (
                "Competitor margin pressure rises",
                "Pricing pressure and program delays are creating customer risk across the competitor account base in Q2 2026.",
                "news",
                vec!["Q2 2026".to_string(), "program delays".to_string()],
            ),
            _ => (
                "Tariff notice issued",
                "A tariff notice affects export-control timing and supplier qualification planning for the next sourcing cycle.",
                "regulatory",
                vec!["tariff notice".to_string(), "next sourcing cycle".to_string()],
            ),
        };

        vec![EvidenceSignal {
            title: title.to_string(),
            description: description.to_string(),
            source_url: "https://example.com/source".to_string(),
            signal_type: signal_type.to_string(),
            extracted_facts: facts,
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        }]
    }

    #[cfg(feature = "llm")]
    fn matrix_payload(
        category: &str,
        public_sector: bool,
        quality: &str,
    ) -> (String, String, String, bool) {
        match (public_sector, quality) {
            (false, "good") => (
                format!("{} program shift creates a timely account move", category),
                "Key Tronic Corporation is facing a documented operating shift in 2026 because the latest evidence names sourcing timing, program exposure, and concrete planning windows [1] [2]. That matters now because procurement teams typically lock qualification paths before the next review cycle, therefore a delayed response would reduce influence over line allocation and supplier shortlist design. If the current pressure deepens, buyers could widen the review scope and demand stronger continuity proof, which would favor suppliers that move before the Q2 2026 window closes. The evidence points to a specific timing problem rather than a generic market observation.".to_string(),
                "Approach Key Tronic Corporation's procurement team within 30 days with a qualification-led continuity proposal tied to the named 2026 review window and the sourcing pressure in the evidence [1].".to_string(),
                true,
            ),
            (false, "borderline") => (
                format!("{} timing window deserves action", category),
                "Key Tronic Corporation is facing a documented operating shift because the latest evidence names sourcing timing, program exposure, and concrete planning windows [1] [2]. That matters now because procurement teams typically lock qualification paths before the next review cycle, therefore a delayed response would reduce influence over line allocation and supplier shortlist design. If the current pressure deepens, buyers could widen the review scope and demand stronger continuity proof, which would favor suppliers that move before the current review closes. The evidence points to a specific timing problem rather than a generic market observation.".to_string(),
                "Approach Key Tronic Corporation's procurement team within 30 days with a qualification-led continuity proposal tied to the named review window and the sourcing pressure in the evidence [1].".to_string(),
                true,
            ),
            (false, _) => (
                format!("{} update", category),
                "Various developments warrant focused analysis and it is important to note that the situation should continue monitoring.".to_string(),
                "Monitor the situation when possible.".to_string(),
                false,
            ),
            (true, "good") => (
                "Policy artifact changes supplier planning".to_string(),
                "The European Commission published a named consultation notice on 2026-03-07 that changes qualification timing because affected suppliers may need to adjust account plans before the next review window [1] [2]. That matters commercially because institutional approval timing can constrain which suppliers remain viable during a formal policy cycle, therefore account teams need to map dependency and stakeholder exposure now instead of waiting for a later clarification. If the review scope widens, customers with open qualification paths could face slower approvals, which would make early scenario planning materially more valuable.".to_string(),
                "Brief the named account team this quarter on policy impact, stakeholder dependencies, and qualification timing before the next Commission review window closes [1].".to_string(),
                true,
            ),
            (true, "borderline") => (
                "Policy artifact changes supplier planning".to_string(),
                "The European Commission published a named consultation notice that changes qualification timing because affected suppliers may need to adjust account plans before the next review window [1] [2]. That matters commercially because institutional approval timing can constrain which suppliers remain viable during a formal policy cycle, therefore account teams need to map dependency and stakeholder exposure now instead of waiting for a later clarification. If the review scope widens, customers with open qualification paths could face slower approvals, which would make early scenario planning materially more valuable for the account.".to_string(),
                "Brief the named account team this quarter on policy impact, stakeholder dependencies, and qualification timing before the next Commission review window closes [1].".to_string(),
                true,
            ),
            (true, _) => (
                "Nearshoring opportunity emerging".to_string(),
                "The latest public-sector developments suggest a direct electronics manufacturing opportunity and broader EMS potential for North Africa.".to_string(),
                "Offer PCBA and box build services to the procurement team and submit a qualification package immediately [1].".to_string(),
                false,
            ),
        }
    }

    #[cfg(feature = "llm")]
    fn matrix_case_passes(category: &str, public_sector: bool, quality: &str) -> bool {
        let entity_ctx = matrix_entity_context(public_sector);
        let evidence_signals = matrix_signal(category, public_sector);
        let (headline, narrative, recommendation, _) =
            matrix_payload(category, public_sector, quality);

        let headline_lower = headline.to_lowercase();
        let narrative_lower = narrative.to_lowercase();
        let recommendation_lower = recommendation.to_lowercase();
        let generic_phrases = [
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
        let placeholder_patterns = [
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

        let is_generic =
            generic_phrases.iter().any(|phrase| {
                headline_lower.contains(phrase)
                    || narrative_lower.contains(phrase)
                    || recommendation_lower.contains(phrase)
            }) || has_formulaic_commercial_language(&headline, &narrative, &recommendation);
        let malformed = malformed_fragments.iter().any(|fragment| {
            headline.to_ascii_lowercase().contains(fragment)
                || narrative_lower.contains(fragment)
                || recommendation_lower.contains(fragment)
        });
        let words = narrative.split_whitespace().count();
        let reference_count = count_numbered_references(&narrative, 12);
        let recommendation_reference_count = count_numbered_references(&recommendation, 12);
        let has_digits = narrative
            .chars()
            .any(|character| character.is_ascii_digit());
        let _has_recommendation = recommendation.split_whitespace().count() >= 12;
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
        .any(|phrase| narrative_lower.contains(phrase));
        let has_counterfactual = narrative_lower.contains("if ")
            && (narrative_lower.contains(" would ") || narrative_lower.contains(" could "));
        let recommendation_has_deadline =
            recommendation_has_action_timing(category, &recommendation);
        let readable_narrative = is_readable_and_useful_digest_text(&narrative)
            && !has_excessive_phrase_repetition(&recommendation);
        let readable_recommendation = recommendation
            .split(['.', ';'])
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .count()
            >= 1;
        let unsupported_security_escalation = has_unsupported_security_escalation(
            category,
            &narrative,
            &recommendation,
            &evidence_signals,
        );
        let unsupported_certification_escalation = has_unsupported_certification_escalation(
            &narrative,
            &recommendation,
            &evidence_signals,
        );
        let unsupported_certification_commercialization =
            has_unsupported_certification_commercialization(
                &headline,
                &narrative,
                &recommendation,
                &evidence_signals,
            );
        let unsupported_public_sector_commercialization =
            has_unsupported_public_sector_commercialization(
                &entity_ctx,
                category,
                &narrative,
                &recommendation,
                &evidence_signals,
            );
        let low_usefulness_public_sector_analysis = has_low_usefulness_public_sector_analysis(
            &entity_ctx,
            category,
            &headline,
            &narrative,
            &recommendation,
            &evidence_signals,
        );
        let topic_alignment_violation = violates_entity_topic_alignment(
            &entity_ctx,
            &evidence_signals,
            &headline,
            &narrative,
            &recommendation,
        );
        let unnamed_customer_targeting =
            has_unnamed_customer_targeting(&narrative, &recommendation);
        let has_placeholders = placeholder_patterns
            .iter()
            .any(|pattern| recommendation_lower.contains(pattern));

        let decisions = vec![
            quality_gate_blocker("generic_language", is_generic, false),
            quality_gate_blocker("malformed_output", malformed, false),
            quality_gate_blocker("placeholder_output", has_placeholders, false),
            quality_gate_requirement("narrative_words", words as f32, 85.0),
            quality_gate_requirement("narrative_references", reference_count as f32, 2.0),
            quality_gate_requirement(
                "recommendation_references",
                recommendation_reference_count as f32,
                1.0,
            ),
            quality_gate_requirement(
                "narrative_has_quantification",
                if has_digits { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_depth",
                recommendation.split_whitespace().count() as f32,
                12.0,
            ),
            quality_gate_requirement(
                "reasoning_depth",
                if has_causal_language || has_counterfactual {
                    1.0
                } else {
                    0.0
                },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_timing",
                if recommendation_has_deadline {
                    1.0
                } else {
                    0.0
                },
                0.5,
            ),
            quality_gate_requirement(
                "narrative_readability",
                if readable_narrative { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_requirement(
                "recommendation_readability",
                if readable_recommendation { 1.0 } else { 0.0 },
                0.5,
            ),
            quality_gate_blocker("security_escalation", unsupported_security_escalation, true),
            quality_gate_blocker(
                "certification_escalation",
                unsupported_certification_escalation,
                false,
            ),
            quality_gate_blocker(
                "certification_commercialization",
                unsupported_certification_commercialization,
                true,
            ),
            quality_gate_blocker(
                "public_sector_commercialization",
                unsupported_public_sector_commercialization,
                false,
            ),
            quality_gate_blocker(
                "public_sector_low_usefulness",
                low_usefulness_public_sector_analysis,
                true,
            ),
            quality_gate_blocker(
                "unnamed_customer_targeting",
                unnamed_customer_targeting,
                false,
            ),
            quality_gate_blocker("topic_alignment", topic_alignment_violation, true),
            quality_gate_requirement(
                "headline_length",
                if is_headline_ok { 1.0 } else { 0.0 },
                0.5,
            ),
        ];

        quality_gate_passes_ensemble(&decisions)
    }

    #[cfg(feature = "llm")]
    #[test]
    fn causal_ordering_validation() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Patent filed".to_string(),
                description: "Patent filed for the new platform.".to_string(),
                source_url: "https://example.com/patent".to_string(),
                signal_type: "patent_filed".to_string(),
                extracted_facts: vec!["Patent filed on 2026-03-01".to_string()],
                date_context: Some("2026-03-01".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Contract awarded".to_string(),
                description: "Contract awarded after the filing window closed.".to_string(),
                source_url: "https://example.com/contract".to_string(),
                signal_type: "contract_awarded".to_string(),
                extracted_facts: vec!["Contract awarded on 2026-03-15".to_string()],
                date_context: Some("2026-03-15".to_string()),
                relevance_score: 1.0,
            },
        ];

        assert!(has_temporal_incoherence(
            "entity-1",
            "The contract awarded before patent filed sequence accelerated customer demand.",
            Utc::now(),
            &evidence_signals,
        ));
    }

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

    #[cfg(feature = "llm")]
    #[test]
    fn end_to_end_gate_matrix_is_deterministic_across_30_payloads() {
        let categories = [
            "demand_procurement",
            "supply_chain_risk",
            "security_compliance",
            "competitor_market",
            "geopolitical_analysis",
        ];
        let qualities = ["good", "borderline", "bad"];

        let mut expected = Vec::new();
        for public_sector in [false, true] {
            for category in categories {
                for quality in qualities {
                    let pass = matrix_case_passes(category, public_sector, quality);
                    expected.push((category, public_sector, quality, pass));
                }
            }
        }

        assert_eq!(expected.len(), 30);

        for _ in 0..3 {
            for (category, public_sector, quality, pass) in &expected {
                assert_eq!(
                    matrix_case_passes(category, *public_sector, quality),
                    *pass,
                    "matrix case changed for category={category}, public_sector={public_sector}, quality={quality}"
                );
            }
        }

        let pass_count = expected.iter().filter(|(_, _, _, pass)| *pass).count();
        let reject_count = expected.len() - pass_count;
        assert!(pass_count > 0);
        assert!(reject_count > 0);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn adversarial_evolution_five_levels_are_caught_for_high_risk_gates() {
        let certification_evidence = vec![
            EvidenceSignal {
                title: "Kimball certification warning".to_string(),
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
        let security_evidence = vec![
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

        let certification_variants = [
            (
                "The certification warning means medical device buyers may switch suppliers because qualification confidence is weakening.",
                "Target their medical device customers with a switch campaign by Q2 2026 [1].",
            ),
            (
                "The regulatory concern is contributing to sourcing reviews and giving rise to alternative-supplier screening.",
                "Pursue their account base before the next sourcing cycle with a continuity offer [1].",
            ),
            (
                "If the standards lapse persists, buyers would widen qualification reviews and look for safer alternatives.",
                "Focus on downstream OEM accounts with a qualification-led message this quarter [1].",
            ),
            (
                "The accreditation gap is precipitating a compliance shortfall narrative across regulated programs.",
                "Work the customer roster with a dual-source pitch before renewal [1].",
            ),
            (
                "La certificati\u{301}on shortfall is giving rise to supplier switches in regulated accounts.",
                "Go after their buyer base with a near-term switch campaign [1].",
            ),
        ];
        for (narrative, recommendation) in certification_variants {
            assert!(
                has_unsupported_certification_escalation(
                    narrative,
                    recommendation,
                    &certification_evidence,
                ),
                "certification variant slipped: {narrative} || {recommendation}"
            );
        }

        let security_variants = [
            (
                "The DNS posture weakness means customers could face qualification delays because trust controls are eroding.",
                "Target their medical device customers for a switch campaign by Q2 2026 [1].",
            ),
            (
                "The email authentication gap is resulting in customer concern and contributing to supplier reviews.",
                "Pursue their account base with an assurance-led switch pitch this quarter [1].",
            ),
            (
                "If domain spoofing exposure persists, buyers would widen sourcing reviews and seek safer alternatives.",
                "Focus on downstream OEM accounts before the next sourcing cycle [1].",
            ),
            (
                "Identity surface exposure is precipitating vendor-assurance scrutiny across regulated programs.",
                "Work the customer roster with a trust-and-continuity offer before renewal [1].",
            ),
            (
                "Spoof\u{131}ng susceptibility is giving rise to supplier switches in sensitive programs.",
                "Go after their buyer base with a near-term continuity campaign [1].",
            ),
        ];
        for (narrative, recommendation) in security_variants {
            assert!(
                has_unsupported_security_escalation(
                    "security_compliance",
                    narrative,
                    recommendation,
                    &security_evidence,
                ),
                "security variant slipped: {narrative} || {recommendation}"
            );
        }

        let targeting_variants = [
            "Target their medical device customers for a switch campaign by Q2 2026 [1].",
            "Pursue their account base before the next sourcing cycle with a continuity offer [1].",
            "If delays continue, work the customer roster with a dual-source message this quarter [1].",
            "Focus on downstream OEM accounts ahead of renewal with an assurance-led pitch [1].",
            "Go after their buyer base with a near-term switch campaign [1].",
        ];
        for recommendation in targeting_variants {
            assert!(
                has_unnamed_customer_targeting("Generic risk narrative.", recommendation,),
                "targeting variant slipped: {recommendation}"
            );
        }
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
        assert!(is_generic_action_hint("Investigate partner sentiment"));
        assert!(is_generic_action_hint("Explore demand drivers"));
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
            "security_compliance",
            &signal_details,
            "Monitor sentiment trajectory; Assess customer impact",
            0.48,
            2,
        ));
        assert!(!should_emit_fallback_insight(
            "security_compliance",
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
    fn shared_quality_gate_allows_richer_promoted_business_summary() {
        let summary = "A named tariff review now affects qualification timing for a defense-adjacent account. Because the supplier notice explicitly changes approval sequencing, the customer could compress sourcing decisions this quarter. The evidence ties the shift to export routing and tender preparation, which creates a nearshoring opportunity for a Morocco or Tunisia production path. The account team should map exposed bids and stakeholder owners before the next sourcing cycle. Procurement should align qualification evidence and pricing for the impacted program this quarter. If the review widens, customer communication would need to move faster, and if it narrows, the same qualification work would still improve bid speed.";

        assert!(passes_shared_insight_quality_gate(
            "European defense supplier faces tariff-driven qualification squeeze",
            summary,
            Some("regulatory_policy"),
        ));
    }

    #[test]
    fn shared_quality_gate_allows_cited_multi_lane_promoted_business_summary() {
        let summary = "Key Tronic Corporation's recent supply chain risk flags and cybersecurity threats [6][7] create a critical vulnerability for its customers, particularly in defense-adjacent and medical sectors where compliance and data security are paramount. The company's 2026 filings show increased focus on risk mitigation [6], but the detected cybersecurity weaknesses [7] suggest potential gaps in protecting sensitive manufacturing data. This positions our ISO 9001 and AS9100-certified North African and EU manufacturing footprint as a direct alternative for clients needing secure, tariff-free production. If Key Tronic's compliance issues worsen, its defense and medical customers could face qualification delays, making our nearshore capabilities a strategic replacement. Approach ABB Ltd. [4] before the next sourcing cycle to position secure, qualified capacity against Key Tronic's risk profile. Contact Airbus Defence & Space [6] this quarter with a defense-program continuity plan tied to qualification speed and data-security controls.";

        assert!(passes_shared_insight_quality_gate(
            "Key Tronic's supply chain risks open door for EMS switch",
            summary,
            Some("demand_procurement"),
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
    fn business_fallback_summary_passes_gate_and_ignores_security_hygiene_noise() {
        let signal_details = vec![
            "8 lookalike domain(s) detected".to_string(),
            "Named RFQ issued for avionics subassembly".to_string(),
            "Supplier portal opened for Q3 RFQ".to_string(),
        ];

        let title = build_analytical_title(
            "Acme EMS",
            "Europe",
            "demand_procurement",
            &signal_details,
            Some("EMS"),
        );
        let analytical = build_analytical_narrative(
            "Acme EMS",
            "Europe",
            Some("EMS"),
            "demand_procurement",
            &signal_details,
            0.78,
            &[],
        );
        let summary = build_fallback_summary(
            &analytical,
            "",
            &signal_details,
            &[],
            "Acme EMS",
            "Europe",
            Some("EMS"),
            "demand_procurement",
            "warning",
            0.78,
            3,
        );

        assert!(
            title.contains("Named RFQ issued for avionics subassembly")
                || title.contains("Supplier portal opened for Q3 RFQ")
        );
        assert!(!title.contains("lookalike domain"));
        assert!(!summary
            .to_ascii_lowercase()
            .contains("has been flagged for"));
        assert!(!summary
            .to_ascii_lowercase()
            .contains("our monitoring detected:"));
        assert!(
            passes_shared_insight_quality_gate(&title, &summary, Some("demand_procurement"),),
            "title={title}\nsummary={summary}"
        );
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
    fn mixed_supply_chain_and_certification_evidence_is_not_treated_as_low_signal_cert_warning() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic tariff pressure".to_string(),
                description: "New tariff exposure is increasing supply-chain costs for medical and industrial programs.".to_string(),
                source_url: "https://example.com/keytronic-tariffs".to_string(),
                signal_type: "supply_chain_risk".to_string(),
                extracted_facts: vec![
                    "Tariff exposure raised landed costs".to_string(),
                    "Medical device programs face margin pressure".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows an updated ISO 13485 certification notice.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 13485 certification updated".to_string(),
                    "Capabilities page refreshed".to_string(),
                ],
                date_context: Some("2026-03-07".to_string()),
                relevance_score: 0.8,
            },
        ];

        let narrative = "Key Tronic's tariff pressure creates a dual-source opening for Medtronic because rising landed costs threaten continuity on regulated device programs.";
        let recommendation = "Approach Medtronic's sourcing team this quarter with a continuity-focused EMS proposal tied to Key Tronic's tariff exposure [1].";

        assert!(!low_signal_certification_warning_case(&evidence_signals));
        assert!(!has_unsupported_certification_escalation(
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn mixed_evidence_still_rejects_soft_certification_switch_claims() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic cyber weaknesses".to_string(),
                description: "Cybersecurity threats and weak controls were observed on a public-facing surface.".to_string(),
                source_url: "https://example.com/keytronic-cyber".to_string(),
                signal_type: "security_compliance".to_string(),
                extracted_facts: vec![
                    "Cybersecurity weaknesses detected".to_string(),
                    "Public-facing controls need remediation".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows an updated ISO 13485 certification notice and generic compliance warning.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 13485 certification updated".to_string(),
                    "Generic compliance warning detected".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 0.8,
            },
        ];

        let narrative = "Key Tronic's compliance risks and cybersecurity threats create a vulnerability for defense-adjacent and medical customers because qualification confidence could weaken.";
        let recommendation = "Approach ABB Ltd. before the next sourcing cycle to position qualified capacity if Key Tronic's compliance issues worsen [1].";

        assert!(!low_signal_certification_warning_case(&evidence_signals));
        assert!(has_unsupported_certification_escalation(
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn mixed_evidence_rejects_flagged_certifications_that_create_switch_risk() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Key Tronic cyber weaknesses".to_string(),
                description: "Cybersecurity threats and weak controls were observed on a public-facing surface.".to_string(),
                source_url: "https://example.com/keytronic-cyber".to_string(),
                signal_type: "security_compliance".to_string(),
                extracted_facts: vec![
                    "Cybersecurity weaknesses detected".to_string(),
                    "Public-facing controls need remediation".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 1.0,
            },
            EvidenceSignal {
                title: "Key Tronic compliance page updated".to_string(),
                description: "The capabilities page shows flagged ISO 13485 and AS9100 certifications with a generic compliance notice.".to_string(),
                source_url: "https://example.com/keytronic-certifications".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "Flagged ISO 13485 certification".to_string(),
                    "Flagged AS9100 certification".to_string(),
                ],
                date_context: Some("2026-03-10".to_string()),
                relevance_score: 0.8,
            },
        ];

        let narrative = "Key Tronic Corporation's compliance gaps, highlighted by flagged ISO 13485 and AS9100 certifications, create immediate risks for its defense-adjacent and medical clients.";
        let recommendation = "Approach ABB Ltd. by Q2 2026 by highlighting our AS9100-certified capacity as a safer alternative if Key Tronic's certification risks deepen [1].";

        assert!(has_unsupported_certification_escalation(
            narrative,
            recommendation,
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn soft_certification_opportunity_headline_is_rejected() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Celestica certifications page updated".to_string(),
                description: "The capabilities page shows ISO 9001 and ISO 13485 certifications with a generic renewal notice and no explicit downstream impact.".to_string(),
                source_url: "https://example.com/celestica-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 9001 certification renewed".to_string(),
                    "ISO 13485 certification renewed".to_string(),
                    "Generic renewal notice posted".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.9,
            },
            EvidenceSignal {
                title: "Celestica compliance notice".to_string(),
                description: "A generic compliance update was detected without named audit action or downstream qualification changes.".to_string(),
                source_url: "https://example.com/celestica-compliance".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec!["Generic compliance update".to_string()],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.8,
            },
        ];

        assert!(has_unsupported_certification_commercialization(
            "Celestica's ISO 9001/13485 Certifications Expire in 2026 - Target Nearshoring Opportunities",
            "Celestica's certification timeline suggests regulated buyers may revisit confidence, creating a strategic outreach opportunity for our North Africa footprint.",
            "Target nearshoring opportunities with EU industrial customers this quarter and position our services as a safer manufacturing option [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn live_brand_sentiment_formulaic_pitch_is_rejected() {
        assert!(has_formulaic_commercial_language(
            "Plexus Corp's AI API gateway expansion: Strategic outreach opportunities",
            "Plexus Corp's recent announcement of a unified API gateway for multiple AI providers [1] signals a strategic pivot toward AI-integrated electronics solutions. This development, coupled with their existing aerospace and defense industry focus [3], creates a commercial opening for targeted EMS partnerships. The AI gateway initiative likely requires robust hardware integration capabilities, which aligns with our ISO 9001 and AS9100-certified facilities in North Africa and Europe [3].",
            "Reach out to Plexus's procurement team by April 5, 2026, emphasizing our defense-qualified facilities and AI hardware integration experience [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn formulaic_headline_is_rejected_even_when_body_is_concrete() {
        assert!(has_formulaic_commercial_language(
            "Government of India's NDC 3.0 strategy presents climate-tech qualification opportunities",
            "The March 2026 policy update [1] names a concrete planning artifact, and the current evidence still needs procurement-path verification before any commercial move.",
            "Brief the account team this week on the named policy update and confirm whether a tender or qualification pathway is actually present before proposing outreach [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn live_flex_nearshore_ems_pitch_is_rejected() {
        assert!(has_formulaic_commercial_language(
            "Flex Ltd faces nearshoring pressure; nearshore EMS opportunities emerge",
            "Flex Ltd's compliance gaps and procurement risks create nearshore EMS opportunities. This matters because our ISO 13485 and AS9100-certified facilities in Tunisia and Morocco position us to target Flex's medical and aerospace clients if those qualification concerns widen.",
            "",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn live_benchmark_competitor_targeting_pitch_is_rejected() {
        assert!(has_formulaic_commercial_language(
            "Benchmark Electronics' DNS risks and R&D activity signal customer vulnerabilities",
            "Benchmark Electronics faces heightened supply chain risks due to its insecure DNS posture, and this dual vulnerability creates opportunities for competitors to target Benchmark's aerospace clients, particularly those with strict compliance requirements.",
            "",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn soft_certification_nearshore_manufacturing_pitch_is_rejected() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Venture certifications profile".to_string(),
                description: "Venture's public certifications page lists ISO standards and a generic compliance status update with no explicit qualification disruption.".to_string(),
                source_url: "https://example.com/venture-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO certification listed".to_string(),
                    "Generic compliance status update".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 0.9,
            },
        ];

        assert!(has_unsupported_certification_commercialization(
            "Venture Corporation's Asia-Pacific EMS network offers nearshore manufacturing opportunities",
            "The certification posture can be framed as a manufacturing opportunity because some buyers may want more resilient options.",
            "Offer nearshore manufacturing to industrial customer opportunities before the next quarter [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn hard_certification_failure_does_not_trigger_commercialization_gate() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Supplier removed after failed audit".to_string(),
                description: "The supplier was removed from an approved vendor list after a failed audit and certificate suspension affecting a named medical device program.".to_string(),
                source_url: "https://example.com/audit-failure".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "Failed audit".to_string(),
                    "Certificate suspended".to_string(),
                    "Supplier removed from approved list".to_string(),
                ],
                date_context: Some("2026-03-12".to_string()),
                relevance_score: 1.0,
            },
        ];

        assert!(!has_unsupported_certification_commercialization(
            "Supplier audit failure disrupts named medical program",
            "The failed audit and supplier removal create a concrete qualification gap for the named device program because procurement now has to re-source an approved build path.",
            "Meet the program sourcing lead this week with an approved alternative package tied to the removed supplier event [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn live_jabil_certification_client_targeting_pitch_is_rejected_with_sparse_cert_evidence() {
        let evidence_signals = vec![EvidenceSignal {
            title: "Jabil medical certifications page updated".to_string(),
            description: "Jabil refreshed its medical manufacturing certifications page and capabilities overview with a generic renewal update and no named customer disruption.".to_string(),
            source_url: "https://example.com/jabil-iso13485".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "ISO 13485 certification listed".to_string(),
                "medical manufacturing capabilities page updated".to_string(),
                "generic renewal update".to_string(),
            ],
            date_context: Some("2026-04-12".to_string()),
            relevance_score: 0.89,
        }];

        assert!(has_unsupported_certification_commercialization(
            "Jabil's ISO 13485 gaps threaten medical device clients",
            "Jabil's ISO 13485 certification for medical device manufacturing is nearing expiration on 2026-08-31, creating urgent risks for its healthcare clients. This certification gap directly impacts clients like Medtronic and Boston Scientific, who rely on Jabil's verified SMT and additive manufacturing services. Our ISO 13485-certified SMT and additive manufacturing services offer a direct alternative, with proven capabilities in 01005 SMT and EU-compliant supply chains.",
            "Approach Medtronic with our ISO 13485-certified SMT services by 2026-08-31. Engage Boston Scientific with our 01005 SMT expertise to mitigate potential delays. Target Siemens Healthineers with our EU-compliant supply chain management by 2026-08-31.",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn live_blue_solutions_nearshore_pitch_is_rejected_with_sparse_cert_evidence() {
        let evidence_signals = vec![EvidenceSignal {
            title: "Blue Solutions IATF certificate page updated".to_string(),
            description: "Blue Solutions published an IATF 16949 certificate status page and generic compliance documentation with a renewal timeline and no named customer disruption.".to_string(),
            source_url: "https://example.com/blue-solutions-iatf".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "IATF 16949 certificate listed".to_string(),
                "generic compliance documentation updated".to_string(),
                "renewal timeline".to_string(),
            ],
            date_context: Some("2026-04-12".to_string()),
            relevance_score: 0.88,
        }];

        assert!(has_unsupported_certification_commercialization(
            "Blue Solutions' IATF gaps threaten automotive clients",
            "Blue Solutions' IATF 16949 certification gaps create immediate risks for automotive clients relying on their compliance. If Blue's compliance issues persist, automotive clients could switch to nearshore EMS providers with verified EU compliance. This creates a window to approach AutoTech Systems' procurement lead with our rapid qualification process before their April 2026 deadline.",
            "Approach AutoTech Systems' procurement lead before April 2026 with our IATF 16949-certified facilities. Engage EuroMotive Motors' supply chain manager with our secure, audit-ready manufacturing. Target DigiDrive Automotive's engineering team with our aerospace-qualified facilities in Tunisia.",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn low_signal_security_hygiene_displacement_is_rejected_outside_security_categories() {
        let evidence_signals = vec![EvidenceSignal {
            title: "GPV Group lookalike domains detected".to_string(),
            description: "Monitoring found multiple lookalike domains and missing DMARC enforcement alongside routine email-authentication hygiene drift.".to_string(),
            source_url: "https://example.com/gpv-lookalikes".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "lookalike domains".to_string(),
                "missing DMARC enforcement".to_string(),
                "email authentication gap".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.92,
        }];

        assert!(has_unsupported_security_escalation(
            "quality_compliance",
            "GPV Group's lookalike domains and email authentication gap now create customer risk because affected buyers may view the supplier as a weaker option.",
            "Offer a direct alternative to industrial customers this quarter before they switch providers [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn negated_hard_failure_security_markers_still_count_as_low_signal_hygiene() {
        let evidence_signals = vec![EvidenceSignal {
            title: "Mouser lookalike domains detected".to_string(),
            description: "Monitoring found multiple lookalike domains with no confirmed compromise or supplier removal, only routine email-authentication hygiene drift.".to_string(),
            source_url: "https://example.com/mouser-lookalikes".to_string(),
            signal_type: "warning".to_string(),
            extracted_facts: vec![
                "lookalike domains".to_string(),
                "missing DMARC enforcement".to_string(),
                "routine email-authentication hygiene drift".to_string(),
            ],
            date_context: Some("2026-04-06".to_string()),
            relevance_score: 0.9,
        }];

        assert!(quality_gates::low_signal_security_hygiene_case(
            "cybersecurity_threat",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn security_hygiene_procurement_portal_conversion_is_rejected_in_security_category() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "Mouser lookalike domains detected".to_string(),
                description: "Monitoring found eight lookalike domains around Mouser's supplier-facing infrastructure alongside routine email authentication hygiene gaps.".to_string(),
                source_url: "https://example.com/mouser-lookalikes".to_string(),
                signal_type: "warning".to_string(),
                extracted_facts: vec![
                    "8 lookalike domains".to_string(),
                    "email authentication gap".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.95,
            },
            EvidenceSignal {
                title: "Mouser supplier portal notice".to_string(),
                description: "A supplier portal page references an onboarding process for distributor partners in March 2026.".to_string(),
                source_url: "https://example.com/mouser-supplier-portal".to_string(),
                signal_type: "procurement".to_string(),
                extracted_facts: vec![
                    "supplier portal onboarding".to_string(),
                    "March 2026 notice".to_string(),
                ],
                date_context: Some("2026-03-15".to_string()),
                relevance_score: 0.72,
            },
        ];

        assert!(has_unsupported_security_escalation(
            "cybersecurity_threat",
            "Mouser's lookalike domains suggest heightened fraud risk around supplier access and portal credentials [1].",
            "Register on Mouser's supplier portal within 30 days, prepare PPAP and IMDS documentation, and use the portal notice to accelerate qualification [2].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn soft_certification_deadline_bom_disruption_is_rejected() {
        let evidence_signals = vec![
            EvidenceSignal {
                title: "STMicroelectronics certification page updated".to_string(),
                description: "STMicroelectronics lists ISO 9001 on a public site with a generic validity window and no explicit disruption or supplier removal.".to_string(),
                source_url: "https://example.com/st-certs".to_string(),
                signal_type: "certification".to_string(),
                extracted_facts: vec![
                    "ISO 9001 listed".to_string(),
                    "valid until 2027".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.84,
            },
            EvidenceSignal {
                title: "STMicroelectronics R&D update".to_string(),
                description: "The company highlighted 300mm wafer work and SiC/FDSOI roadmap investment in Crolles with no direct customer impact described.".to_string(),
                source_url: "https://example.com/st-rd".to_string(),
                signal_type: "news".to_string(),
                extracted_facts: vec![
                    "300mm wafer capabilities".to_string(),
                    "SiC roadmap".to_string(),
                ],
                date_context: Some("2026-04-06".to_string()),
                relevance_score: 0.79,
            },
        ];

        assert!(has_unsupported_certification_escalation(
            "STMicroelectronics' ISO 9001 deadline could disrupt deliveries if compliance transitions slip, forcing urgent BOM revisions for automotive customers.",
            "Alert customers to begin alternate part qualification before the 2027 certification deadline creates delivery risk [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn soft_certification_marker_is_unicode_normalized() {
        assert!(contains_soft_certification_pressure_marker(
            "A certificati\u{301}on warning was posted after the latest compliance review."
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
    fn unnamed_customer_targeting_ignores_generic_narrative_when_recommendation_is_specific() {
        let narrative = "Key Tronic's recent supply chain instability creates risk for their medical customers and industrial buyers.";
        let recommendation =
            "Approach Medtronic's sourcing team this quarter with a dual-source continuity pitch tied to Key Tronic's supply-chain volatility [1].";

        assert!(!has_unnamed_customer_targeting(narrative, recommendation));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_detects_unicode_and_downstream_account_language() {
        assert!(has_unnamed_customer_targeting(
            "Tariff pressure is building across the account base.",
            "Ta\u{301}rget their medical device customers for a nearshore switch campaign by Q2 2026 [1]."
        ));
        assert!(has_unnamed_customer_targeting(
            "The supplier is under margin pressure.",
            "Pursue downstream OEM accounts before the next sourcing cycle with a continuity-led offer [1]."
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_allows_named_internal_stakeholder_outreach() {
        let narrative = "Flex Ltd faces multiple supply chain risks that could disrupt automotive and industrial programs.";
        let recommendation = "Contact Flex Ltd's procurement lead Revathi Advaithi within 30 days to propose a capacity audit for their automotive and industrial programs [1].";

        assert!(!has_unnamed_customer_targeting(narrative, recommendation));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_in_narrative_is_rejected() {
        let narrative = "NOTE AB's lookalike domains suggest aggressive competitive outreach, and their aerospace clients may now be open to a switch.";
        let recommendation =
            "Prioritize a displacement plan before the next sourcing cycle closes [1].";

        assert!(has_unnamed_customer_targeting(narrative, recommendation));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_rejects_generic_customer_cohorts() {
        assert!(has_unnamed_customer_targeting(
            "Digi-Key's Part-DB update may unsettle qualification timelines for regulated buyers.",
            "Act on aerospace and medical customers before the next sourcing cycle [1].",
        ));
        assert!(has_unnamed_customer_targeting(
            "Jabil's ISO 13485 renewal timeline could raise review questions.",
            "Approach their medical device clients with a direct alternative this quarter [1].",
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn named_target_provenance_rejects_hallucinated_downstream_company() {
        let entity_ctx = EntityContext {
            name: "Jabil".to_string(),
            region: "North America".to_string(),
            entity_type: Some("company".to_string()),
            is_competitor: false,
            industry_tags: vec!["electronics manufacturing services".to_string()],
            certifications: vec!["ISO 13485".to_string()],
            capabilities: vec![],
            key_persons: vec![],
            recent_changes: vec![],
            threat_score: None,
            overlap_score: None,
            strategic_relevance: None,
            revenue_estimate_usd: None,
            employee_estimate: None,
            competitor_names: vec!["Flex".to_string()],
            sites_summary: vec!["St. Petersburg medical manufacturing campus".to_string()],
            competitor_events: vec![],
            domain: Some("jabil.com".to_string()),
        };
        let evidence_signals = vec![EvidenceSignal {
            title: "Jabil ISO 13485 page updated".to_string(),
            description: "Jabil updated its medical manufacturing certifications page with an ISO 13485 renewal note and no named customer impact.".to_string(),
            source_url: "https://example.com/jabil-iso13485".to_string(),
            signal_type: "certification".to_string(),
            extracted_facts: vec![
                "ISO 13485 renewal note".to_string(),
                "medical manufacturing campus".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.86,
        }];

        assert!(has_unsupported_named_target_provenance(
            &entity_ctx,
            "Jabil's ISO 13485 renewal may unsettle regulated sourcing reviews",
            "The renewal timing could create questions for regulated builds if buyers want a backup path [1].",
            "Approach MedTech Innovators Ltd. before Q3 sourcing reviews begin [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn named_target_provenance_allows_named_company_present_in_evidence() {
        let entity_ctx = EntityContext {
            name: "Digi-Key".to_string(),
            region: "North America".to_string(),
            entity_type: Some("distributor".to_string()),
            is_competitor: false,
            industry_tags: vec!["electronics distribution".to_string()],
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
            domain: Some("digikey.com".to_string()),
        };
        let evidence_signals = vec![EvidenceSignal {
            title: "Digi-Key names Acme Medical Systems in rollout update".to_string(),
            description: "The update names Acme Medical Systems as a launch customer for the new workflow and ties the rollout to regulated sourcing reviews.".to_string(),
            source_url: "https://example.com/partdb-acme".to_string(),
            signal_type: "product_update".to_string(),
            extracted_facts: vec![
                "Acme Medical Systems launch customer".to_string(),
                "regulated sourcing reviews".to_string(),
            ],
            date_context: Some("2026-04-04".to_string()),
            relevance_score: 0.91,
        }];

        assert!(!has_unsupported_named_target_provenance(
            &entity_ctx,
            "Digi-Key rollout could affect regulated sourcing timing",
            "Because Acme Medical Systems is explicitly named in the rollout evidence, the account impact can be discussed directly [1].",
            "Approach Acme Medical Systems this quarter [1].",
            &evidence_signals,
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn strategic_recommendation_timing_accepts_this_quarter() {
        assert!(recommendation_has_action_timing(
            "demand_procurement",
            "Approach the procurement lead this quarter with a qualification-led EMS proposal [1]."
        ));
        assert!(!recommendation_has_action_timing(
            "demand_procurement",
            "Approach the procurement lead when possible with a qualification-led EMS proposal [1]."
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn strategic_recommendation_timing_accepts_quarter_shorthand_and_sourcing_cycle() {
        assert!(recommendation_has_action_timing(
            "demand_procurement",
            "Contact the sourcing team by Q2 2026 with a qualification-led EMS proposal [1]."
        ));
        assert!(recommendation_has_action_timing(
            "demand_procurement",
            "Approach the procurement lead ahead of the next sourcing cycle with a continuity-led EMS proposal [1]."
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn causal_link_accepts_expanded_synonyms() {
        assert!(contains_causal_link(
            "The certification lapse is precipitating customer concern and contributing to sourcing reviews."
        ));
        assert!(contains_causal_link(
            "The warning is resulting in qualification risk and giving rise to supplier switches."
        ));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn weighted_scorer_matches_stemmed_single_token_variants() {
        assert!(weighted_phrase_score(
            "The supplier was certifying new lines after prior certified output and legacy certifications.",
            &[("certification", 0.30)]
        ) >= 0.30);
        assert!(weighted_phrase_score(
            "The compliance team qualified the plant after qualification work on adjacent cells.",
            &[("qualification", 0.30)]
        ) >= 0.30);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn quality_gate_ensemble_allows_single_non_veto_failure() {
        let decisions = vec![
            quality_gate_blocker("generic_language", true, false),
            quality_gate_requirement("narrative_words", 96.0, 85.0),
            quality_gate_blocker("security_escalation", false, true),
        ];

        assert!(quality_gate_passes_ensemble(&decisions));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn quality_gate_ensemble_rejects_three_non_veto_failures() {
        let decisions = vec![
            quality_gate_blocker("generic_language", true, false),
            quality_gate_requirement("narrative_words", 72.0, 85.0),
            quality_gate_requirement("reasoning_depth", 0.0, 0.5),
            quality_gate_blocker("security_escalation", false, true),
        ];

        assert!(!quality_gate_passes_ensemble(&decisions));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn quality_gate_ensemble_passes_two_non_veto_failures() {
        let decisions = vec![
            quality_gate_blocker("generic_language", true, false),
            quality_gate_requirement("narrative_words", 72.0, 85.0),
            quality_gate_blocker("security_escalation", false, true),
        ];

        assert!(quality_gate_passes_ensemble(&decisions));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn quality_gate_ensemble_rejects_single_veto_failure() {
        let decisions = vec![
            quality_gate_blocker("security_escalation", true, true),
            quality_gate_requirement("narrative_words", 96.0, 85.0),
        ];

        assert!(!quality_gate_passes_ensemble(&decisions));
    }

    #[cfg(feature = "llm")]
    #[test]
    fn weekly_quality_gate_metrics_flag_low_precision_and_recall() {
        let reviewed_at = DateTime::<Utc>::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(2026, 3, 10)
                .unwrap_or_default()
                .and_hms_opt(12, 0, 0)
                .unwrap_or_default(),
            Utc,
        );
        let metrics = summarize_quality_gate_reviews(&[
            QualityGateReview {
                gate_name: "certification_escalation",
                blocked: true,
                human_confirmed_block: true,
                reviewed_at,
            },
            QualityGateReview {
                gate_name: "certification_escalation",
                blocked: true,
                human_confirmed_block: false,
                reviewed_at,
            },
            QualityGateReview {
                gate_name: "certification_escalation",
                blocked: false,
                human_confirmed_block: true,
                reviewed_at,
            },
        ]);

        assert_eq!(metrics.len(), 1);
        assert!(metrics[0].alert_precision);
        assert!(metrics[0].alert_recall);
        assert!((metrics[0].precision_observed - 0.5).abs() < f64::EPSILON);
        assert!((metrics[0].recall_observed - 0.5).abs() < f64::EPSILON);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn noisy_or_favors_single_strong_signal_over_many_weak_signals() {
        let weak_stack = noisy_or(&[0.15, 0.15, 0.15, 0.15, 0.15]);
        let strong_single = noisy_or(&[0.70]);

        assert!(weak_stack < strong_single);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn calculate_relevance_prefers_strong_category_alignment() {
        let strong = calculate_relevance(
            "AS9100 qualification risk emerges in defense program review",
            "Medical device and defense program eligibility could tighten after the latest audit finding.",
            "certification",
            "security_compliance",
        );
        let weak = calculate_relevance(
            "General company update posted",
            "The supplier shared a broad business update with no concrete procurement or compliance signal.",
            "news",
            "security_compliance",
        );

        assert!(strong > weak);
        assert!(strong <= 1.0);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn signal_diversity_multiplier_rewards_multiple_types() {
        assert_eq!(signal_diversity_multiplier(1), 1.0);
        assert_eq!(signal_diversity_multiplier(3), 1.2);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn golden_set_regression_accepts_at_least_ninety_five_percent_agreement() {
        let reviewed_at = Utc::now();
        let mut examples = Vec::new();

        for index in 0..19 {
            examples.push(QualityGateGoldenSetExample {
                source_id: Uuid::new_v4(),
                source_kind: "warning".to_string(),
                content_type: "supplier_disruption".to_string(),
                historical_label: HistoricalQualityGateLabel::Accepted,
                title: format!("Supplier disruption signal {}", index),
                body: "Multiple customs delays, named customer escalations, and documented sourcing reviews now threaten program continuity across two audited production lines [1].".to_string(),
                region: Some("MENA".to_string()),
                confidence: Some(0.84),
                source_urls: vec!["https://example.com/1".to_string()],
                reviewed_at,
                metadata: serde_json::json!({"review_outcome": "true_positive"}),
            });
        }

        examples.push(QualityGateGoldenSetExample {
            source_id: Uuid::new_v4(),
            source_kind: "warning".to_string(),
            content_type: "supplier_disruption".to_string(),
            historical_label: HistoricalQualityGateLabel::Rejected,
            title: "Generic market commentary".to_string(),
            body: "This may matter eventually, so watch the situation and keep an eye on possible changes if priorities shift later.".to_string(),
            region: Some("Global".to_string()),
            confidence: Some(0.35),
            source_urls: vec!["https://example.com/2".to_string()],
            reviewed_at,
            metadata: serde_json::json!({"review_outcome": "false_positive"}),
        });

        let regression = evaluate_quality_gate_golden_set(&examples);

        assert_eq!(regression.total_examples, 20);
        assert!(regression.disagreements.is_empty());
        assert!(regression.agreement >= 0.95);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn golden_set_regression_flags_disagreements_below_threshold() {
        let reviewed_at = Utc::now();
        let mut examples = Vec::new();

        for index in 0..18 {
            examples.push(QualityGateGoldenSetExample {
                source_id: Uuid::new_v4(),
                source_kind: "warning".to_string(),
                content_type: "security_compliance".to_string(),
                historical_label: HistoricalQualityGateLabel::Accepted,
                title: format!("Compliance disruption case {}", index),
                body: "Named audit findings, customer escalation, and sourcing-review evidence indicate immediate qualification risk for active regulated programs [1].".to_string(),
                region: Some("EU".to_string()),
                confidence: Some(0.89),
                source_urls: vec!["https://example.com/a".to_string()],
                reviewed_at,
                metadata: serde_json::json!({"review_outcome": "true_positive"}),
            });
        }

        for index in 0..2 {
            examples.push(QualityGateGoldenSetExample {
                source_id: Uuid::new_v4(),
                source_kind: "warning".to_string(),
                content_type: "security_compliance".to_string(),
                historical_label: HistoricalQualityGateLabel::Accepted,
                title: format!("Borderline accepted case {}", index),
                body: "Stay alert for updates later.".to_string(),
                region: Some("EU".to_string()),
                confidence: Some(0.51),
                source_urls: vec!["https://example.com/b".to_string()],
                reviewed_at,
                metadata: serde_json::json!({"review_outcome": "true_positive"}),
            });
        }

        let regression = evaluate_quality_gate_golden_set(&examples);

        assert_eq!(regression.total_examples, 20);
        assert_eq!(regression.disagreements.len(), 2);
        assert!(regression.agreement < 0.95);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn security_hygiene_marker_accepts_adversarial_synonyms() {
        let variants = [
            "The supplier shows an email authentication gap across customer-facing domains.",
            "Monitoring found domain spoofing exposure and a weak mail trust posture.",
            "The brand now faces impersonation risk from look alike domains.",
            "A typosquatting campaign is exploiting the firm's DNS hygiene issue.",
            "The company has spoofing susceptibility across its identity surface.",
        ];

        for variant in variants {
            assert!(
                contains_security_hygiene_marker(variant),
                "missed variant: {variant}"
            );
        }
    }

    #[cfg(feature = "llm")]
    #[test]
    fn unnamed_customer_targeting_accepts_adversarial_synonyms() {
        let variants = [
            "Go after their buyer base before the next sourcing cycle [1].",
            "Pursue their account base with a continuity pitch this quarter [1].",
            "Focus on their downstream buyer set for a switch campaign [1].",
            "Engage the customer roster they may lose if delays worsen [1].",
            "Work the account portfolio with a dual-source message [1].",
        ];

        for variant in variants {
            assert!(
                has_unnamed_customer_targeting("Generic risk narrative.", variant),
                "missed variant: {variant}"
            );
        }
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
    fn public_sector_output_without_process_or_concrete_action_is_rejected() {
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
            description: "The European Commission published a trade defense consultation notice tied to supplier qualification timing for related programs.".to_string(),
            source_url: "https://trade.ec.europa.eu/doclib/notice-2026-03-07".to_string(),
            signal_type: "regulatory".to_string(),
            extracted_facts: vec!["consultation notice".to_string(), "supplier qualification".to_string()],
            date_context: Some("2026-03-07".to_string()),
            relevance_score: 1.0,
        }];

        let headline = "European Commission consultation may matter for suppliers";
        let narrative = "The consultation notice matters for suppliers and could affect activity around the Commission [1].";
        let recommendation = "Continue monitoring and watch for developments before acting [1].";

        assert!(has_low_usefulness_public_sector_analysis(
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

    #[cfg(feature = "llm")]
    #[test]
    fn sanitize_validated_role_rejects_company_name_garbage() {
        let disc = DiscoveredPoi {
            name: "Sujatha Chandrasekaran".to_string(),
            inferred_role: Some("Chief Operating Officer".to_string()),
            inferred_org: Some("Jabil".to_string()),
            source_url: "https://www.jabil.com/about-us/senior-leadership.html".to_string(),
            discovery_method: "org_leadership".to_string(),
            contact_email: None,
            contact_linkedin: None,
            confidence: 0.75,
            seed_person_id: "seed-1".to_string(),
            ts_discovered: 0,
        };

        assert_eq!(
            sanitize_validated_role(&disc, Some("ApexIntel".to_string()), Some("Jabil")),
            None
        );
        assert_eq!(
            sanitize_validated_role(&disc, Some("Jabil".to_string()), Some("Jabil")),
            None
        );
        assert_eq!(
            sanitize_validated_role(
                &disc,
                Some("Chief Operating Officer".to_string()),
                Some("Jabil")
            )
            .as_deref(),
            Some("Chief Operating Officer")
        );
    }

    #[cfg(feature = "llm")]
    #[test]
    fn sanitize_validated_org_rejects_role_text() {
        let disc = DiscoveredPoi {
            name: "Jane Doe".to_string(),
            inferred_role: Some("VP Supply Chain".to_string()),
            inferred_org: Some("Acme Electronics".to_string()),
            source_url: "https://www.acme-electronics.com/team".to_string(),
            discovery_method: "org_leadership".to_string(),
            contact_email: None,
            contact_linkedin: None,
            confidence: 0.84,
            seed_person_id: "seed-1".to_string(),
            ts_discovered: 0,
        };

        assert_eq!(
            sanitize_validated_org(&disc, Some("VP Supply Chain".to_string())),
            None
        );
        assert_eq!(
            sanitize_validated_org(&disc, Some("ApexIntel".to_string())),
            None
        );
        assert_eq!(
            sanitize_validated_org(&disc, Some("Acme Electronics".to_string())).as_deref(),
            Some("Acme Electronics")
        );
    }
}
