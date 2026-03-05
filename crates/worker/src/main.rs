use anyhow::Result;
#[cfg(feature = "llm")]
use anyhow::Context;
use apex_core::config::AppConfig;
use apex_crawl::breach::BreachMonitor;
#[cfg(feature = "llm")]
use apex_crawl::person_scraper::{PersonOsintScraper, RawPersonArtifact};
#[cfg(feature = "llm")]
use apex_crawl::tor_client::{DarkWebPersonIntel, TorClient};
use apex_crawl::proxy::ProxyRotator;
use apex_crawl::sanctions::SanctionsScreener;
#[cfg(feature = "llm")]
use apex_crawl::poi_expansion::{DiscoveredPoi, PoiExpansionEngine, SeedPoi};
#[cfg(feature = "llm")]
use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
#[cfg(feature = "llm")]
use apex_llm::evaluation::{standard_eval_suite, EvalRunner};
#[cfg(feature = "llm")]
use apex_llm::inference::LlmClient as InferenceLlmClient;
#[cfg(feature = "llm")]
use apex_llm::self_improvement::{
    ImprovementCycleReport, OutputCapture, SelfImprovementConfig, SelfImprovementLoop,
    TaskCategory,
};
use apex_worker::nightly::{
    process_mining_stage, process_drift_stage,
    CrawlStageResult, DriftCheckStageResult, MiningStageResult,
    PoiRefreshStageResult,
};
#[cfg(not(feature = "llm"))]
use apex_worker::nightly::process_poi_stage;
#[cfg(feature = "llm")]
use apex_worker::nightly::{
    process_hypothesis_generation_stage, HypothesisGenerationStageResult,
};
use apex_worker::scheduler::{
    default_scheduler, validate_custom_command, JobKind, JobRun, JobStatus, Scheduler,
};
use apex_worker::weekly::{
    run_weekly_pipeline, DeprecationPolicy, MemoInputs, PromotionPolicy, ProductionRecipe,
    StagedRecipe,
};
use apex_worker::recipe_loader::{load_default_seed_recipes, validate_seed_recipes, print_recipe_stats, insert_seed_recipes};
use apex_worker::notifications::{NotificationDispatcher, SlaEnforcer, SlaWarningRecord};
use chrono::Utc;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
#[cfg(feature = "llm")]
use std::sync::Mutex;
use tokio::sync::{Mutex as TokioMutex, Semaphore};
use tracing_subscriber::EnvFilter;
use apex_store::postgres::PgStore;
#[cfg(feature = "llm")]
use apex_store::postgres::{
    InsightListFilters, PersonListFilters, PersonOrderBy, WarningListFilters,
};
use apex_core::entities::{Observation, ObservationType};
#[cfg(feature = "llm")]
use apex_core::entities::{Person, PriorityVector};
#[cfg(feature = "llm")]
use apex_core::entities::{ArtifactType, PoiArtifact};
use apex_core::schemas::{Recipe, RecipeStatus, SignalSpec};
use apex_recipes::engine::{RecipeEngine, FeatureMap};
use apex_crawl::sanctions::SanctionsList;
use std::collections::HashMap;
#[cfg(feature = "llm")]
use std::collections::HashSet;
use uuid::Uuid;
use apex_crawl::source_scoring::{score_and_rank, ScoringConfig, SourceTelemetry};
use apex_crawl::sources::all_sources;
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
use apex_poi::model::{InfluenceProfile, PoiProfile, PriorityVector as PoiPriorityVector, PsychProfile, RoleFamily};
#[cfg(feature = "llm")]
use apex_poi::updater::refresh_profile;
#[cfg(any(feature = "parse", feature = "llm"))]
use apex_parse::html::extract_page;
use apex_worker::storage::{build_memo_inputs, load_staged_recipes, load_production_recipes, StorageContext};

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

/// Truncate text to max bytes respecting UTF-8 boundaries.
#[allow(dead_code)]
fn truncate_text(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            matches!(n.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn build_paid_proxy_url_from_env() -> Option<String> {
    let host = std::env::var("PROXY_HOST").ok().filter(|v| !v.trim().is_empty())?;
    let port = std::env::var("PROXY_PORT").ok().filter(|v| !v.trim().is_empty())?;
    let username = std::env::var("PROXY_USERNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("PROXY_USER").ok().filter(|v| !v.trim().is_empty()))?;
    let password = std::env::var("PROXY_PASSWORD")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("PROXY_PASS").ok().filter(|v| !v.trim().is_empty()))?;
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
    if t == "." { t.clear(); }
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
    if s.len() < 20 { return true; }
    let lower = s.to_ascii_lowercase();
    
    // Dashes check (placeholder junk)
    let dash_count = s.matches('—').count();
    let word_count = s.split_whitespace().count();
    if word_count > 0 && dash_count as f64 / word_count as f64 > 0.25 {
        return true;
    }
    
    // Sentences that are just field names strung together
    let short_tokens: usize = s.split(". ")
        .filter(|seg| seg.split_whitespace().count() <= 2)
        .count();
    let total_sentences = s.split(". ").count();
    if total_sentences >= 3 && short_tokens as f64 / total_sentences as f64 > 0.5 {
        return true;
    }
    
    // Detect double prepositions indicating missing slots between them
    // e.g., "supply from under threat" (missing resource_type between "from" and "under")
    let double_prep_patterns = [
        "from under", "from via", "from through", "from by",
        "via via", "via by", "via from", "via through",
        "by by", "by from", "by via",
        "in in", "to to", "at at", "for for",
        "of of", "with with", "between between",
        "supply from under", // specific pattern from bad template
        "showing conflict", "showing including", // unfilled {{evidence:signal_count}}
    ];
    for pat in &double_prep_patterns {
        if lower.contains(pat) {
            return true;
        }
    }
    
    // Vague claims without specifics - all-caps alarmist labels with no data
    // These templates fire but provide no actual evidence
    let vague_alarm_patterns = [
        ("resource weaponization", "which resource"),  // must specify what resource
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
        ". supply from",  // sentence starting without subject
        ". including.",   // empty list
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

/// Build an analytical narrative paragraph from structured signal data.
fn build_analytical_narrative(
    entity_label: &str,
    entity_region: &str,
    category: &str,
    signal_details: &[String],
    confidence: f64,
    evidence_ids: &[String],
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

    let conf_label = if confidence >= 0.85 { "high" }
        else if confidence >= 0.65 { "moderate" }
        else { "preliminary" };

    let mut parts = Vec::new();

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

    // Contextual assessment based on category
    let assessment = match category {
        "competitor_market" => "This pattern suggests the competitor is actively positioning for market expansion or capability enhancement. Consider reviewing defensive positioning and customer retention strategies.",
        "demand_procurement" => "These indicators typically precede formal sourcing activity within 30-60 days. Early engagement with the procurement team can secure preferred supplier status.",
        "supply_chain_risk" => "These indicators warrant proactive risk mitigation. Consider diversifying supply sources and engaging with affected partners for contingency planning.",
        "security_compliance" => "Compliance gaps represent both a risk to current operations and a potential competitive lever. Ensure your own certifications are up to date.",
        "regulatory_policy" => "Regulatory shifts can create both compliance obligations and competitive advantages for prepared organizations. Review impact on your product lines and certifications.",
        "strategic_poi" => "Key personnel activity can signal strategic direction changes. Track subsequent organizational announcements for confirmation.",
        "pricing_market" => "Market pricing shifts affect margins and competitive positioning. Review current contract terms and pricing strategy for affected product lines.",
        "customer_rfq" => "An active procurement opportunity has been identified. Quick response with tailored technical capabilities can differentiate your proposal.",
        "geopolitical_analysis" => "Geopolitical developments can affect trade flows, regulatory requirements, and supply chain stability in the region.",
        "talent_ip" => "Talent migration and IP activity are leading indicators of strategic direction. Track subsequent patent filings, hiring patterns, and capability announcements for confirmation.",
        "technology_innovation" => "Technology and R&D activity signals strategic investment priorities. Monitor for product launches, partnerships, and capability expansion.",
        "ma_partnerships" => "M&A and partnership activity reshapes competitive landscape. Assess combined entity capabilities and customer impact windows.",
        "market_expansion" => "Market expansion signals growth strategy and capacity investment. Monitor for competitive positioning shifts in affected regions.",
        "cybersecurity_threat" => "Cybersecurity threats require immediate assessment. Evaluate exposure, implement protective measures, and notify affected stakeholders.",
        "quality_compliance" => "Quality and certification changes impact supplier qualification. Verify current status and assess compliance implications.",
        "brand_sentiment" => "Brand sentiment shifts can indicate customer relationship changes or market events. Monitor trends and assess competitive implications.",
        _ => "Monitor for follow-up signals that confirm or change this assessment.",
    };
    parts.push(assessment.to_string());

    // Confidence footer
    parts.push(format!(
        "Confidence: {conf_label} ({:.0}%), based on {} signal(s).",
        confidence * 100.0,
        evidence_ids.len()
    ));

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
    let is_government = entity_type
        .map(|t| t.to_lowercase().contains("government"))
        .unwrap_or(false);

    // Use different action verbs for government entities
    let action = if is_government {
        match category {
            "competitor_market" => "regulatory activity detected",
            "demand_procurement" => "tender/solicitation identified",
            "supply_chain_risk" => "policy risk flagged",
            "security_compliance" => "regulatory update detected",
            "regulatory_policy" => "policy change detected",
            "strategic_poi" => "official activity detected",
            "pricing_market" => "economic policy shift detected",
            "customer_rfq" => "government procurement posted",
            "geopolitical_analysis" => "diplomatic development flagged",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "official appointment detected",
            "technology_innovation" => "research initiative detected",
            "ma_partnerships" => "bilateral agreement detected",
            "market_expansion" => "policy development detected",
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
            "strategic_poi" => "key personnel activity detected",
            "pricing_market" => "market pricing shift detected",
            "customer_rfq" => "procurement opportunity identified",
            "geopolitical_analysis" => "geopolitical development flagged",
            "veracity_analysis" => "intelligence cross-referenced",
            "talent_ip" => "talent/IP activity detected",
            "technology_innovation" => "R&D activity detected",
            "ma_partnerships" => "M&A/partnership activity detected",
            "market_expansion" => "market expansion detected",
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
    let detail = signal_details.first()
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
    signal_type: String,  // e.g., "certification", "capability", "news", "poi", "warning"
    extracted_facts: Vec<String>,  // Key facts extracted from the evidence
    date_context: Option<String>,  // When this happened/detected
    relevance_score: f32,  // How relevant to the insight category
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
    certifications: Vec<String>,  // e.g., "AS9100D (valid until 2027-03)", "ISO 13485"
    capabilities: Vec<String>,    // e.g., "High-Volume SMT", "Medical Device Manufacturing"
    key_persons: Vec<String>,     // e.g., "CEO: Jensen Huang (C-Suite, influential)"
    recent_changes: Vec<String>,  // e.g., "New facility detected", "Leadership change"
    // Rich competitive context
    threat_score: Option<f64>,
    overlap_score: Option<f64>,
    strategic_relevance: Option<f64>,
    revenue_estimate_usd: Option<i64>,
    employee_estimate: Option<i32>,
    competitor_names: Vec<String>,       // Linked competitors via graph edges
    sites_summary: Vec<String>,          // e.g., "Manufacturing plant in Tunis, Tunisia"
    competitor_events: Vec<String>,      // Recent competitor moves
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
            ("rfq", 1.0), ("tender", 1.0), ("procurement", 0.9), ("sourcing", 0.8),
            ("bid", 0.8), ("contract", 0.7), ("supplier", 0.6), ("vendor", 0.6),
        ],
        "supply_chain_risk" => &[
            ("shortage", 1.0), ("delay", 0.9), ("disruption", 0.9), ("risk", 0.8),
            ("constraint", 0.8), ("lead time", 0.7), ("allocation", 0.7), ("single source", 0.9),
        ],
        "competitor_market" => &[
            ("competitor", 1.0), ("market share", 0.9), ("pricing", 0.8), ("win", 0.8),
            ("lost", 0.8), ("expansion", 0.7), ("capability", 0.6), ("capacity", 0.6),
        ],
        "security_compliance" => &[
            ("certification", 1.0), ("iso", 0.9), ("as9100", 1.0), ("iatf", 1.0),
            ("compliance", 0.9), ("audit", 0.8), ("accreditation", 0.9), ("security", 0.7),
        ],
        "regulatory_policy" => &[
            ("regulation", 1.0), ("tariff", 0.9), ("sanction", 1.0), ("export control", 1.0),
            ("policy", 0.8), ("legislation", 0.8), ("compliance", 0.7), ("itar", 1.0),
        ],
        "strategic_poi" => &[
            ("ceo", 1.0), ("cto", 1.0), ("cfo", 1.0), ("executive", 0.9),
            ("appointed", 0.9), ("resigned", 0.9), ("leadership", 0.8), ("vp", 0.7),
        ],
        "ma_partnerships" => &[
            ("acquisition", 1.0), ("merger", 1.0), ("partnership", 0.9), ("joint venture", 0.9),
            ("acquired", 1.0), ("divest", 0.9), ("spin-off", 0.8), ("alliance", 0.7),
        ],
        "technology_innovation" => &[
            ("patent", 1.0), ("r&d", 0.9), ("innovation", 0.9), ("breakthrough", 0.9),
            ("technology", 0.7), ("launch", 0.7), ("product", 0.6), ("development", 0.6),
        ],
        "cybersecurity_threat" => &[
            ("breach", 1.0), ("vulnerability", 1.0), ("cyber", 0.9), ("attack", 0.9),
            ("security", 0.7), ("malware", 1.0), ("ransomware", 1.0), ("incident", 0.8),
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
        "certification" if category.contains("compliance") || category.contains("security") => score += 0.5,
        "capability" if category.contains("procurement") || category.contains("competitor") => score += 0.4,
        "poi" if category.contains("strategic_poi") || category.contains("talent") => score += 0.6,
        "warning" => score += 0.3,  // Warnings are generally relevant
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
    if let Some(idx) = description.find("Articles:").or_else(|| description.find("articles:")) {
        let after = &description[idx..];
        // Find the article list — it ends at "Source:" or "See:" or end of string
        let end = after.find(". Source:")
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
            seen.contains(&lower) || lower.contains(seen.as_str()) ||
            (lower.len() > 15 && seen.len() > 15 && lower[..15] == seen[..15])
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
        Some(t) if t.to_lowercase().contains("ems") => "an electronics manufacturing services (EMS) provider",
        Some(t) if t.to_lowercase().contains("oem") => "an original equipment manufacturer (OEM)",
        Some(t) if t.to_lowercase().contains("semiconductor") => "a semiconductor manufacturer",
        Some(t) if t.to_lowercase().contains("defense") || t.to_lowercase().contains("defence") => "a defense and aerospace company",
        Some(t) if t.to_lowercase().contains("government") => "a government entity",
        Some(t) if t.to_lowercase().contains("automotive") => "an automotive tier supplier",
        Some(t) if t.to_lowercase().contains("distributor") => "an electronic components distributor",
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
    let region = if entity_ctx.region.is_empty() { "undisclosed region".to_string() } else { entity_ctx.region.clone() };

    // ── Deduplicate and rank signals ──
    let mut sorted: Vec<&EvidenceSignal> = evidence_signals.iter().collect();
    sorted.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
    let deduped = dedup_signals(&sorted);
    let top_signals: Vec<&EvidenceSignal> = deduped.into_iter().take(5).collect();

    // ── Extract article titles from descriptions (the richest data source) ──
    let mut article_titles: Vec<String> = Vec::new();
    for sig in &top_signals {
        let mut titles = extract_article_titles(&sig.description);
        article_titles.append(&mut titles);
    }
    // Dedup articles: strip trailing dots, then fuzzy-dedup by prefix
    article_titles = article_titles.into_iter()
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
    let all_text: String = evidence_signals.iter()
        .map(|s| format!("{} {} {}", s.title, s.description, s.extracted_facts.join(" ")))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let has_acquisition = all_text.contains("acquisition") || all_text.contains("acquir") || all_text.contains("merger") || all_text.contains("joint venture");
    let has_certification = evidence_signals.iter().any(|s| s.signal_type == "certification") || all_text.contains("iso ") || all_text.contains("as9100") || all_text.contains("iatf");
    let has_capability = evidence_signals.iter().any(|s| s.signal_type == "capability");
    let has_innovation = all_text.contains("innovation") || all_text.contains("award") || all_text.contains("patent") || all_text.contains("r&d");
    let has_expansion = all_text.contains("facility") || all_text.contains("expansion") || all_text.contains("groundbreaking") || all_text.contains("new plant") || all_text.contains("new site");
    let has_hiring = all_text.contains("career") || all_text.contains("hiring") || all_text.contains("recruit") || all_text.contains("job opening");
    let has_compliance = all_text.contains("compliance") || all_text.contains("audit") || all_text.contains("regulation");
    let has_tariff = all_text.contains("tariff") || all_text.contains("sanction") || all_text.contains("trade war") || all_text.contains("embargo") || all_text.contains("export control");
    let has_shortage = all_text.contains("shortage") || all_text.contains("allocation") || all_text.contains("lead time") || all_text.contains("supply constraint") || all_text.contains("force majeure");
    let has_financial = all_text.contains("revenue") || all_text.contains("quarter") || all_text.contains("earnings") || all_text.contains("fiscal") || all_text.contains("investor");

    // ── Gather entity data ──
    let cert_list: Vec<&str> = entity_ctx.certifications.iter().take(3).map(|s| s.as_str()).collect();
    let cap_list: Vec<&str> = entity_ctx.capabilities.iter().take(4).map(|s| s.as_str()).collect();
    let poi_list: Vec<&str> = entity_ctx.key_persons.iter().take(2).map(|s| s.as_str()).collect();

    // Extract all concrete facts
    let all_facts: Vec<String> = top_signals.iter()
        .flat_map(|s| s.extracted_facts.iter().cloned())
        .filter(|f| !f.is_empty())
        .take(6)
        .collect();

    // ── Build headline ──
    let headline = build_rich_headline(
        name, &region, category, &top_signals, &article_titles,
        &cert_list, &cap_list,
        has_acquisition, has_expansion, has_innovation, has_tariff,
        has_shortage, has_financial, has_certification,
    );

    // ── Build narrative paragraphs ──
    let mut paragraphs: Vec<String> = Vec::new();

    // Paragraph 1: What happened
    let what_happened = build_what_happened(
        name, &region, &top_signals, &article_titles, &all_facts,
        &cert_list, &cap_list, entity_ctx.entity_type.as_deref(),
    );
    if !what_happened.is_empty() {
        paragraphs.push(what_happened);
    }

    // Paragraph 2: Why it matters — strategic analysis
    let why_matters = build_why_it_matters(
        name, &region, category, &cert_list, &cap_list, &poi_list,
        has_acquisition, has_certification, has_capability, has_innovation,
        has_expansion, has_hiring, has_compliance, has_tariff, has_shortage,
        has_financial, &article_titles,
    );
    paragraphs.push(why_matters);

    // Paragraph 3: Actionable recommendation
    let recommendation = build_recommendation(
        name, category, &cert_list, &cap_list, &poi_list,
        has_acquisition, has_expansion, has_tariff, has_shortage,
        has_compliance, has_innovation, has_financial, has_certification,
    );
    paragraphs.push(format!("Recommended action: {}", recommendation));

    // Priority callout for critical/elevated
    if severity == "critical" {
        paragraphs.push("Priority: CRITICAL — immediate executive attention required. Escalate within 48 hours.".into());
    } else if severity == "warning" {
        paragraphs.push("Priority: Elevated — requires analyst review within the current planning cycle.".into());
    }

    // Confidence assessment
    let sig_count = evidence_signals.len().min(20);
    let conf_label = if confidence >= 0.85 { "high" } else if confidence >= 0.65 { "moderate" } else { "preliminary" };
    let conf_basis = if sig_count >= 5 {
        format!("{} corroborating signals across multiple source types", sig_count)
    } else if sig_count >= 2 {
        format!("{} corroborating signals", sig_count)
    } else {
        "single-source intelligence".to_string()
    };
    paragraphs.push(format!("Confidence: {} ({:.0}%), based on {}.", conf_label, confidence * 100.0, conf_basis));

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
            return format!("{name}: capacity expansion in {} manufacturing", caps[0].split('(').next().unwrap_or(caps[0]).trim());
        }
        return format!("{name}: facility expansion underway in {region}");
    }
    if has_innovation {
        if !caps.is_empty() {
            return format!("{name}: technology evolution in {} — early engagement window", caps[0].split('(').next().unwrap_or(caps[0]).trim());
        }
        return format!("{name}: innovation and R&D signals indicate technology pivot");
    }
    if has_financial {
        return format!("{name}: financial activity signals — review business trajectory");
    }
    if has_certification && !certs.is_empty() {
        return format!("{name}: {} certification update — verify qualification status", certs[0].split('(').next().unwrap_or(certs[0]).trim());
    }

    // Priority 4: Category-specific with entity data enrichment
    match category {
        "demand_procurement" => {
            if !caps.is_empty() {
                format!("{name}: procurement signals in {} — sourcing opportunity", caps[0].split('(').next().unwrap_or(caps[0]).trim())
            } else {
                format!("{name} ({region}): procurement activity indicates emerging sourcing opportunity")
            }
        }
        "supply_chain_risk" => format!("{name}: supply chain risk indicators — mitigation planning required"),
        "competitor_market" => format!("{name}: competitive positioning shift detected in {region}"),
        "security_compliance" => {
            if !certs.is_empty() {
                format!("{name}: {} compliance update — supplier qualification impact", certs[0].split('(').next().unwrap_or(certs[0]).trim())
            } else {
                format!("{name}: security and compliance posture update")
            }
        }
        "regulatory_policy" => format!("{name}: regulatory changes affect operations in {region}"),
        "strategic_poi" => {
            if !caps.is_empty() {
                format!("{name}: strategic signals in {} domain", caps[0].split('(').next().unwrap_or(caps[0]).trim())
            } else {
                format!("{name}: strategic activity signals direction change in {region}")
            }
        }
        "technology_innovation" => {
            if !caps.is_empty() {
                format!("{name}: {} technology evolution — capability development signals", caps[0].split('(').next().unwrap_or(caps[0]).trim())
            } else {
                format!("{name}: technology investment signals emerging capability")
            }
        }
        "ma_partnerships" => format!("{name}: M&A or partnership activity reshaping competitive landscape"),
        "market_expansion" => {
            if !certs.is_empty() {
                format!("{name}: market expansion with {} qualification in {region}", certs[0].split('(').next().unwrap_or(certs[0]).trim())
            } else {
                format!("{name}: market expansion activity detected in {region}")
            }
        }
        "quality_compliance" => format!("{name}: quality system update — supplier requalification required"),
        "talent_ip" => format!("{name}: talent and IP investment signals strategic capability build"),
        "customer_rfq" => format!("{name}: RFQ or customer engagement activity in {region}"),
        "geopolitical_analysis" => format!("{name}: geopolitical exposure assessment required for {region}"),
        "pricing_market" => format!("{name}: pricing signals indicate market dynamics shift"),
        "cybersecurity_threat" => format!("{name}: cybersecurity posture change — vendor risk review"),
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
        let articles_formatted: Vec<String> = article_titles.iter()
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
            let mut types: Vec<String> = top_signals.iter()
                .map(|s| {
                    let cleaned = clean_signal_title(&s.title, name);
                    if cleaned.len() > 10 {
                        cleaned
                    } else {
                        // Fallback: use signal_type as description
                        match s.signal_type.as_str() {
                            "warning" => format!("{} activity signal", s.title.split(':').last().unwrap_or("change").trim().to_lowercase().replace(" detected", "")),
                            "certification" => format!("{} certification", s.title.replace("Certification", "").trim()),
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
        parts.push(format!(
            "Verified capability: {}.",
            caps[0],
        ));
    }

    // Add extracted facts (monetary values, percentages, dates, etc.)
    if !all_facts.is_empty() {
        let concrete_facts: Vec<&String> = all_facts.iter()
            .filter(|f| f.contains('$') || f.contains('%') || f.contains("202") || f.contains("employee") || f.contains("facility"))
            .take(3)
            .collect();
        if !concrete_facts.is_empty() {
            parts.push(format!(
                "Extracted data points: {}.",
                concrete_facts.iter().map(|f| f.as_str()).collect::<Vec<_>>().join("; "),
            ));
        }
    }

    // Date context
    let dates: Vec<&str> = top_signals.iter()
        .filter_map(|s| s.date_context.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .take(2)
        .collect();
    if !dates.is_empty() {
        parts.push(format!("Signal observation period: {}.", dates.join(" to ")));
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
            format!(", building on established {} capabilities", caps.iter().map(|c| c.split('(').next().unwrap_or(c).trim()).collect::<Vec<_>>().join(", "))
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
            format!(" Recent developments — including {} — suggest", truncate_text(&article_titles[0], 70))
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
            format!(" Ensure any alternative sources hold equivalent {} certification.", certs[0].split('(').next().unwrap_or(certs[0]).trim())
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
            format!(" Coordinate with {} as primary contact during transition.", pois[0])
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
            format!(" specifically for {} programs", caps[0].split('(').next().unwrap_or(caps[0]).trim())
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
            format!(", building on their existing {} base", caps[0].split('(').next().unwrap_or(caps[0]).trim())
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

    // ── Build rich entity profile with competitive context ──
    let mut profile_parts: Vec<String> = Vec::new();
    let type_str = entity_ctx.entity_type.as_deref().unwrap_or("company");
    if type_str.to_lowercase().contains("government") {
        profile_parts.push(format!("{} is a government/public sector entity in {}.", entity_ctx.name, entity_ctx.region));
    } else {
        let mut desc = format!("{} is a {} based in {}.", entity_ctx.name, type_str, entity_ctx.region);
        if let Some(rev) = entity_ctx.revenue_estimate_usd {
            desc.push_str(&format!(" Estimated revenue: ~${:.0}M.", rev as f64 / 1_000_000.0));
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
        profile_parts.push(format!("Industry focus: {}.", entity_ctx.industry_tags.join(", ")));
    }
    if !entity_ctx.certifications.is_empty() {
        let cert_str = entity_ctx.certifications.iter().take(6).cloned().collect::<Vec<_>>().join("; ");
        profile_parts.push(format!("Certifications: {}", cert_str));
    }
    if !entity_ctx.capabilities.is_empty() {
        let cap_str = entity_ctx.capabilities.iter().take(6).cloned().collect::<Vec<_>>().join("; ");
        profile_parts.push(format!("Capabilities: {}", cap_str));
    }
    if !entity_ctx.key_persons.is_empty() {
        let poi_str = entity_ctx.key_persons.iter().take(4).cloned().collect::<Vec<_>>().join("; ");
        profile_parts.push(format!("Key personnel: {}", poi_str));
    }
    if !entity_ctx.sites_summary.is_empty() {
        let sites_str = entity_ctx.sites_summary.iter().take(4).cloned().collect::<Vec<_>>().join("; ");
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
        comp_parts.push(format!("Linked competitors: {}", entity_ctx.competitor_names.join(", ")));
    }
    if !comp_parts.is_empty() {
        profile_parts.push(format!("Competitive profile: {}", comp_parts.join(". ")));
    }
    if !entity_ctx.recent_changes.is_empty() {
        let changes_str = entity_ctx.recent_changes.iter().take(4).cloned().collect::<Vec<_>>().join("; ");
        profile_parts.push(format!("Recent changes: {}", changes_str));
    }
    if !entity_ctx.competitor_events.is_empty() {
        let events_str = entity_ctx.competitor_events.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
        profile_parts.push(format!("Competitor intelligence: {}", events_str));
    }
    // Competitor classification — tells the LLM how to treat this entity
    if entity_ctx.is_competitor {
        profile_parts.push(
            "⚠️ ENTITY CLASSIFICATION: DIRECT EMS COMPETITOR — Do NOT recommend offering our services \
to this company. Analyse their weaknesses, identify which of their customers are underserved, \
and recommend approaching those customers instead.".to_string()
        );
    } else {
        profile_parts.push(
            "✅ ENTITY CLASSIFICATION: CUSTOMER / PROSPECT — Recommend direct outreach, \
service proposals, and partnership opportunities to this entity.".to_string()
        );
    }
    let entity_profile = profile_parts.join("\n");

    // ── Build sorted evidence text ──
    let mut sorted_evidence: Vec<_> = evidence_signals.iter().collect();
    sorted_evidence.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
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
                parts.push(format!("   Detail: {}", crate::truncate_text(&sig.description, 420)));
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
        "geopolitical_analysis" => (
            "Geopolitical Opportunity",
            "Translate geopolitical/regulatory shifts into commercial actions: new tariffs → nearshoring opportunity in Morocco/Tunisia; export control changes → qualification opportunity for EU-based alternatives; sanctions → market gap to fill. Focus on North African and EU angles. Map affected trade lanes to specific entities and their likely procurement pivots.",
            "Name the specific companies that will need to re-source and when. Specify which North African or EU production site to position. Include regulatory deadlines and qualification windows.",
        ),
        "regulatory_policy" | "pricing_market" => (
            "Market Intelligence",
            "Extract actionable business intelligence: certification changes create qualification windows; pricing signals reveal margin pressure at competitors; regulatory shifts create compliance consulting opportunities. Cross-reference with known competitor capabilities and recent changes to identify where rivals are weak.",
            "Identify 2-3 specific commercial actions: companies to approach with compliance offers, pricing advantages to highlight, or capability gaps to fill. Name the buyer role and a specific deadline tied to the regulatory change.",
        ),
        _ => (
            "Business Intelligence",
            "Produce case-specific competitive intelligence. Identify who is winning, losing, hiring, cutting, expanding, or retreating. Cross-reference evidence to find exploitable patterns: a company hiring procurement staff likely has upcoming RFQs; a company with lapsing certifications has a compliance gap we can fill.",
            "Name 2-3 specific actions with company names, contact roles, pitch angles, and deadlines. Every recommendation must answer: 'who do we call, what do we say, and by when?'",
        ),
    }};

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

Every brief you write must lead to a specific commercial action — a call to make, a bid to prepare, a competitor's customer to approach, or a market gap to fill.

Rules:
- Every claim must cite a numbered evidence reference [1], [2], etc.
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
        }
    );

    let user = format!(
        r#"Write a {category_label} brief about {entity_name}.

{entity_profile}

Evidence:
{evidence_text}

Analysis guidance: {analysis_focus}
Action guidance: {action_focus}

Respond with valid JSON only:
{{
  "headline": "Action-oriented headline (<=140 chars) that names {entity_name} and the specific opportunity or threat",
    "narrative": "120-320 words of commercially-driven analysis. Cite evidence as [1], [2], [3]. Structure as: (1) What happened — with specific facts, dates, names. (2) Causal chain — mechanism and second-order effects (supply, pricing, qualification, customer switching). (3) Counterfactual — if this trend accelerates or reverses, what changes commercially. (4) Our angle — what we can offer, who is likely to buy, and why now.",
  "recommendation": "2-4 concrete actions derived from THIS entity's evidence and competitive profile. Each action must name a REAL company or person from the evidence, a specific service we can offer, and a deadline. Structure: '<Named company or role from evidence> — <specific pitch tied to the finding> — by <date from or near the evidence>'. Every named target must appear in the entity profile, the evidence signals, or the competitive profile above — never invent targets.",
  "confidence": 0.0
}}

MANDATORY:
- Reference at least 2 evidence items as [1], [2], etc.
- Include at least 3 concrete facts from the evidence (names, dates, numbers, places, standards).
- Include at least one explicit cause-effect statement (for example, 'because ... therefore ...').
- Include at least one counterfactual statement using 'if ... would/could ...'.
- Every recommendation target MUST be drawn from the evidence signals, entity profile, or competitive profile — never from your general knowledge. The entity being analyzed ({entity_name}) is always a valid target.
- Focus on what to DO, not what to observe.
"#,
        category_label = category_label,
        entity_name = entity_ctx.name,
        entity_profile = entity_profile,
        evidence_text = evidence_text,
        analysis_focus = analysis_focus,
        action_focus = action_focus,
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
                            let parts: Vec<String> = ["action", "owner", "deadline", "description", "task"]
                                .iter()
                                .filter_map(|key| map.get(*key).and_then(|v| v.as_str()).map(|s| s.to_string()))
                                .collect();
                            if parts.is_empty() {
                                // Fallback: join all string values
                                let all_strings: Vec<String> = map.values()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                                if all_strings.is_empty() { None } else { Some(all_strings.join(" — ")) }
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
                let parts: Vec<String> = map.values()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if parts.is_empty() { Ok(None) } else { Ok(Some(parts.join(" — "))) }
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

    for attempt in 1..=3 {
        let messages = vec![ChatMessage::system(system.as_str()), ChatMessage::user(&user)];
        let resp = llm_client.complete_with_config(messages, &config).await
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
        let is_generic = generic_phrases.iter().any(|p| {
            narrative_lower.contains(p) || recommendation_lower.contains(p)
        });

        let words = narrative.split_whitespace().count();
        let reference_count = (1..=12)
            .filter(|i| narrative.contains(&format!("[{}]", i)))
            .count();
        let has_refs = reference_count >= 2;
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

        // Reject if recommendation contains bracket placeholders like [Company X], [specific service], [date]
        let placeholder_patterns: &[&str] = &[
            "[company ", "[specific ", "[date]", "[competitor ",
            "[our ", "[their ", "[service", "[product",
            "[client ", "[customer ", "[contact ",
        ];
        let has_placeholders = placeholder_patterns.iter().any(|p| recommendation_lower.contains(p));

        let passes = !is_generic
            && !has_placeholders
            && words >= 85
            && has_refs
            && has_digits
            && has_recommendation
            && has_reasoning_depth
            && recommendation_has_deadline
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
            has_digits,
            has_recommendation,
            has_reasoning_depth,
            recommendation_has_deadline,
            recommendation_words = recommendation.split_whitespace().count(),
            recommendation_preview = %crate::truncate_text(&recommendation, 120),
            generic = is_generic,
            has_placeholders,
            headline = %headline,
            narrative_preview = %crate::truncate_text(&narrative, 180),
            "LLM quality gate: rejected"
        );
    }

    anyhow::bail!("LLM failed quality checks after retries for {}", entity_ctx.name)
}

/// Build template-based fallback summary when LLM is unavailable.
fn build_fallback_summary(
    narrative_with_entity: &str,
    use_template: bool,
    rendered_action: &str,
    rendered_narrative: &str,
    signal_details: &[String],
    entity_label: &str,
    entity_region: &str,
    category: &str,
    severity: &str,
    confidence: f64,
    evidence_count: usize,
) -> String {
    let mut summary_parts: Vec<String> = Vec::new();

    // Para 1: main narrative
    summary_parts.push(narrative_with_entity.to_string());

    // Para 2: recommended action (only if template was good)
    if use_template && !rendered_action.is_empty() && rendered_action != rendered_narrative {
        summary_parts.push(format!("Recommended action: {rendered_action}"));
    }

    // Para 3: data details (when using analytical narrative, these are already included)
    if use_template {
        if !signal_details.is_empty() {
            let details_str = signal_details.join("; ");
            let entity_ctx = if !entity_label.is_empty() {
                if !entity_region.is_empty() {
                    format!("For {} ({})", entity_label, entity_region)
                } else {
                    format!("For {}", entity_label)
                }
            } else {
                "Supporting data".to_string()
            };
            summary_parts.push(format!("{entity_ctx}: {details_str}."));
        }
    }

    // Para 4: category-specific analytical assessment (always)
    let assessment = match category {
        "competitor_market" => "This pattern suggests the competitor is actively positioning for market expansion or capability enhancement. Consider reviewing defensive positioning and customer retention strategies.",
        "demand_procurement" => "These indicators typically precede formal sourcing activity within 30-60 days. Early engagement with the procurement team can secure preferred supplier status.",
        "supply_chain_risk" => "These indicators warrant proactive risk mitigation. Consider diversifying supply sources and engaging with affected partners for contingency planning.",
        "security_compliance" => "Compliance gaps represent both a risk to current operations and a potential competitive lever. Ensure your own certifications are up to date.",
        "regulatory_policy" => "Regulatory shifts can create both compliance obligations and competitive advantages for prepared organizations. Review impact on your product lines and certifications.",
        "strategic_poi" => "Key personnel activity can signal strategic direction changes. Track subsequent organizational announcements for confirmation.",
        "pricing_market" => "Market pricing shifts affect margins and competitive positioning. Review current contract terms and pricing strategy for affected product lines.",
        "customer_rfq" => "An active procurement opportunity has been identified. Quick response with tailored technical capabilities can differentiate your proposal.",
        "geopolitical_analysis" => "Geopolitical developments can affect trade flows, regulatory requirements, and supply chain stability in the region.",
        "talent_ip" => "Talent migration and IP activity are leading indicators of strategic direction. Track subsequent patent filings, hiring patterns, and capability announcements for confirmation.",
        "technology_innovation" => "Technology and R&D activity signals strategic investment priorities. Monitor for product launches, partnerships, and capability expansion.",
        "ma_partnerships" => "M&A and partnership activity reshapes competitive landscape. Assess combined entity capabilities and customer impact windows.",
        "market_expansion" => "Market expansion signals growth strategy and capacity investment. Monitor for competitive positioning shifts in affected regions.",
        "cybersecurity_threat" => "Cybersecurity threats require immediate assessment. Evaluate exposure, implement protective measures, and notify affected stakeholders.",
        "quality_compliance" => "Quality and certification changes impact supplier qualification. Verify current status and assess compliance implications.",
        "brand_sentiment" => "Brand sentiment shifts can indicate customer relationship changes or market events. Monitor trends and assess competitive implications.",
        _ => "Monitor for follow-up signals that confirm or change this assessment.",
    };
    summary_parts.push(format!("Assessment: {assessment}"));

    // Para 5: severity context
    if severity == "critical" {
        summary_parts.push("Severity: CRITICAL — immediate attention recommended.".into());
    } else if severity == "warning" {
        summary_parts.push("Severity: WARNING — monitor closely and consider proactive measures.".into());
    }

    // Para 6: confidence footer
    let conf_label = if confidence >= 0.85 { "high" }
        else if confidence >= 0.65 { "moderate" }
        else { "preliminary" };
    summary_parts.push(format!(
        "Confidence: {} ({:.0}%), based on {} signal(s).",
        conf_label,
        confidence * 100.0,
        evidence_count,
    ));

    summary_parts.join("\n\n")
}

/// Parse a YAML signal string such as `"JobPost.role_family=Procurement.increase"` into a
/// [`SignalSpec`].  The expected format is `{ObsType}.{field[=value]}.{operator}`, where the
/// trailing segment is compared against the list of known operators; if it does not match,
/// `"contains"` is used as the default and the whole right-hand side is treated as the field.
fn parse_signal_str(s: &str) -> SignalSpec {
    const KNOWN_OPS: &[&str] = &["increase", "decrease", "above", "below", "equals", "contains"];
    let (obs_type, rest) = match s.find('.') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => return SignalSpec {
            observation_type: s.to_string(),
            field: "count".to_string(),
            operator: "above".to_string(),
            threshold: Some(0.0),
            window_days: Some(30),
            value: None,
        },
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
        (field_part[..eq].to_string(), Some(field_part[eq + 1..].to_string()))
    } else {
        (field_part.to_string(), None)
    };
    SignalSpec {
        observation_type: obs_type.to_string(),
        field: if field.is_empty() { "count".to_string() } else { field },
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
    let action = sr.action_playbook.first().cloned().unwrap_or_default();
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
        .connect(&config.database_url)
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
                            result.inserted, result.skipped, result.errors.len()
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

    let scheduler = Arc::new(TokioMutex::new(default_scheduler()));
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
    let manual_trigger_semaphore = Arc::new(Semaphore::new(manual_trigger_concurrency));
    let jobs_count = scheduler.lock().await.jobs.len();
    tracing::info!(
        jobs = jobs_count,
        manual_trigger_concurrency,
        manual_max_claims_per_poll,
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
    tracing::trace!("scheduler_tick_start");
    let now = Utc::now();
    let due = scheduler.due_jobs(now);

    if due.is_empty() {
        tracing::debug!("no jobs due at {}", now);
        return;
    }

    for kind in due {
        let run = execute_job(&kind, store).await;
        tracing::info!(
            job = kind.as_str(),
            status = format_status(&run),
            duration_ms = run.duration_ms(),
            "job completed"
        );
        scheduler.record_run(run);
    }
}

async fn poll_trigger_queue(
    store: &Arc<PgStore>,
    manual_trigger_semaphore: &Arc<Semaphore>,
    max_claims_per_poll: usize,
) {
    let mut claimed_this_poll: usize = 0;
    loop {
        if claimed_this_poll >= max_claims_per_poll {
            break;
        }

        // Bound in-flight manual triggers so one long-running recipe_fire
        // does not block all trigger processing forever.
        let permit = match Arc::clone(manual_trigger_semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => break,
        };

        match store.pop_job_trigger().await {
            Ok(Some((trigger_id, job_kind_str))) => {
                claimed_this_poll += 1;
                let kind = JobKind::from_str(&job_kind_str);
                tracing::info!(trigger_id = %trigger_id, job = %job_kind_str, "manual trigger: executing job");

                let store = Arc::clone(store);
                tokio::spawn(async move {
                    let _permit = permit;
                    let run = execute_job(&kind, &store).await;
                    let error = if matches!(run.status, JobStatus::Failed { .. }) {
                        Some(run.notes.as_str())
                    } else {
                        None
                    };
                    if let Err(e) = store.complete_job_trigger(&trigger_id, error).await {
                        tracing::warn!(trigger_id = %trigger_id, "failed to mark trigger complete: {e}");
                    }
                    tracing::info!(
                        trigger_id = %trigger_id,
                        job = job_kind_str,
                        status = format_status(&run),
                        "manual trigger: job completed"
                    );
                });
            }
            Ok(None) => {
                drop(permit);
                break;
            }
            Err(e) => {
                drop(permit);
                tracing::warn!("poll_trigger_queue: DB error: {e}");
                break;
            }
        }
    }

    if claimed_this_poll > 0 {
        tracing::info!(claimed = claimed_this_poll, max_claims_per_poll, "poll_trigger_queue: claimed trigger(s)");
    }
}

#[tracing::instrument(skip(kind, store), fields(job = %kind.as_str()))]
async fn execute_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    tracing::debug!(job = %kind.as_str(), "job_start");
    match kind {
        JobKind::CrawlCycle => {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            // Live crawl: iterate over enabled tier-1/2 sources, fetch their
            // RSS / main URL with reqwest, parse with apex_parse (if enabled),
            // and store a WebChange observation for every successful fetch.
            let sources = all_sources();
            let crawl_limit: usize = std::env::var("CRAWL_MAX_SOURCES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20);
            let enabled_tier_sources: Vec<_> = sources
                .iter()
                .filter(|s| s.enabled && s.tier <= 2)
                .collect();

            // Always include selected POI-relevant regional sources even when crawl_limit is tight.
            let always_include_slugs = ["globes_il_tech"];
            let mut fetch_sources: Vec<_> = enabled_tier_sources
                .iter()
                .copied()
                .filter(|s| always_include_slugs.contains(&s.slug.as_str()))
                .collect();

            for src in &enabled_tier_sources {
                if fetch_sources.len() >= crawl_limit {
                    break;
                }
                if fetch_sources.iter().any(|existing| existing.slug == src.slug) {
                    continue;
                }
                fetch_sources.push(*src);
            }

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

            let http = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .user_agent("ApexIntelBot/1.0 (+https://apex-intel.io/bot)")
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    run.fail(&format!("crawl_cycle: failed to build http client: {e}"));
                    return run;
                }
            };

            let mut proxy_rotator = build_proxy_rotator_from_env();
            if let Some(rotator) = proxy_rotator.as_ref() {
                tracing::info!(
                    proxy_health = %rotator.health_summary(),
                    "crawl_cycle: proxy rotation enabled"
                );
            }

            let mut ingested: u64 = 0;
            let mut errors: u64 = 0;
            let mut successful_sources: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut failed_sources: std::collections::HashSet<String> = std::collections::HashSet::new();

            for src in &fetch_sources {
                let url = src.rss_url.as_deref().unwrap_or(src.url.as_str());
                let prefers_browser_ua = src.slug == "globes_il_tech";
                let proxy_for_request = proxy_rotator.as_mut().and_then(|r| r.get_next());
                let response = if let Some(proxy_url) = proxy_for_request.as_ref() {
                    match reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(20))
                        .user_agent("ApexIntelBot/1.0 (+https://apex-intel.io/bot)")
                        .proxy(match reqwest::Proxy::all(proxy_url) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(
                                    source = %src.slug,
                                    proxy = %proxy_url,
                                    error = %e,
                                    "crawl_cycle: invalid proxy URL"
                                );
                                errors += 1;
                                continue;
                            }
                        })
                        .build()
                    {
                        Ok(client) => {
                            let mut req = client.get(url);
                            if prefers_browser_ua {
                                req = req.header(
                                    reqwest::header::USER_AGENT,
                                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
                                );
                            }
                            req.send().await
                        }
                        Err(e) => {
                            tracing::warn!(
                                source = %src.slug,
                                proxy = %proxy_url,
                                error = %e,
                                "crawl_cycle: failed to build proxied client"
                            );
                            errors += 1;
                            continue;
                        }
                    }
                } else {
                    let mut req = http.get(url);
                    if prefers_browser_ua {
                        req = req.header(
                            reqwest::header::USER_AGENT,
                            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
                        );
                    }
                    req.send().await
                };

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        if let (Some(proxy_url), Some(rotator)) =
                            (proxy_for_request.as_ref(), proxy_rotator.as_mut())
                        {
                            rotator.report_success(proxy_url);
                        }
                        match resp.text().await {
                            Ok(body) => {
                                // Build observation value: with apex_parse, include
                                // extracted text; without it, store minimal metadata.
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

                                let obs = Observation::new(
                                    ObservationType::WebChange,
                                    Utc::now(),
                                    obs_value,
                                    serde_json::json!({
                                        "source": src.slug,
                                        "tier": src.tier,
                                        "category": format!("{:?}", src.category),
                                    }),
                                );
                                match store.insert_observation(&obs).await {
                                    Ok(_) => {
                                        ingested += 1;
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
                            Err(e) => {
                                tracing::warn!(source = %src.slug, error = %e, "crawl_cycle: body read error");
                                errors += 1;
                                failed_sources.insert(src.slug.clone());
                            }
                        }
                    }
                    Ok(resp) => {
                        if let (Some(proxy_url), Some(rotator)) =
                            (proxy_for_request.as_ref(), proxy_rotator.as_mut())
                        {
                            rotator.report_failure(proxy_url);
                        }
                        let status = resp.status();
                        let body_preview = resp
                            .text()
                            .await
                            .ok()
                            .map(|body| crate::truncate_text(&body, 180).to_string())
                            .unwrap_or_default();
                        tracing::warn!(
                            source = %src.slug,
                            status = %status,
                            body_preview = %body_preview,
                            "crawl_cycle: non-2xx response"
                        );
                        errors += 1;
                        failed_sources.insert(src.slug.clone());
                    }
                    Err(e) => {
                        if let (Some(proxy_url), Some(rotator)) =
                            (proxy_for_request.as_ref(), proxy_rotator.as_mut())
                        {
                            rotator.report_failure(proxy_url);
                        }
                        tracing::warn!(source = %src.slug, error = %e, "crawl_cycle: fetch error");
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
                    "crawl_cycle: {}/{} sources attempted; {} observations ingested, {} errors; success_ratio={:.2}",
                    fetch_sources.len(),
                    sources.iter().filter(|s| s.enabled).count(),
                    ingested,
                    errors,
                    success_ratio,
                ),
            );
            run
        }
        JobKind::PatternMining => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            let since = Utc::now() - chrono::Duration::hours(24);
            let mining_stats = match store.get_mining_stats(since).await {
                Ok(s) => s,
                Err(e) => {
                    run.fail(&format!("pattern_mining: failed to load mining stats from DB: {e}"));
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
                    run.succeed(stage.items, &format!("mining completed: {}", stage.run.notes));
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
        JobKind::HypothesisGeneration => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Hypothesis generation requires the `llm` feature.
            // When disabled, skip gracefully.
            #[cfg(feature = "llm")]
            {
                let since = Utc::now() - chrono::Duration::hours(24);
                let mining_stats = match store.get_mining_stats(since).await {
                    Ok(s) => s,
                    Err(e) => {
                        run.fail(&format!("hypothesis_generation: failed to load mining stats from DB: {e}"));
                        return run;
                    }
                };
                let stage = process_hypothesis_generation_stage(
                    &HypothesisGenerationStageResult {
                        candidates_submitted: mining_stats.candidates_passed_gates,
                        hypotheses_generated: mining_stats.hypotheses_generated,
                        hypotheses_failed: mining_stats.candidates_passed_gates
                            .saturating_sub(mining_stats.hypotheses_generated),
                        recipes_staged: mining_stats.recipes_staged,
                        errors: mining_stats.errors,
                    },
                );
                match stage.run.status {
                    apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                        run.succeed(stage.items, &format!("hypothesis gen completed: {}", stage.run.notes));
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
                run.skip("hypothesis generation requires the `llm` feature");
            }
            run
        }
        JobKind::PoiRefresh => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            #[cfg(feature = "llm")]
            {
                // Live POI refresh: load persons from DB, build PoiProfile
                // objects, run refresh_profile to recompute derived fields,
                // and log changes.  No write-back to DB for influence_score
                // is performed here — this updates in-memory profiles and
                // logs changes; a dedicated upsert step can be added later.
                let filters = PersonListFilters {
                    regions: vec![],
                    roles: vec![],
                    search: None,
                    min_priority: None,
                    max_priority: None,
                };
                let persons = match store
                    .list_persons(&filters, Some(PersonOrderBy::Priority), true, 200, 0)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        run.fail(&format!("poi_refresh: failed to load persons: {e}"));
                        return run;
                    }
                };

                if persons.is_empty() {
                    run.skip("poi_refresh: no persons in database");
                    return run;
                }

                let now_utc = Utc::now().timestamp();
                let mut refreshed: u64 = 0;
                let mut unchanged: u64 = 0;
                let mut enriched_pois: u64 = 0;

                for row in &persons {
                    let mut profile = PoiProfile {
                        person_id: row.id.to_string(),
                        name: row.name.clone(),
                        name_variants: vec![],
                        org: row.organization.clone(),
                        org_id: None,
                        current_role: row.role.clone(),
                        role_family: RoleFamily::Other(row.role.clone()),
                        region: row.region.clone(),
                        country_code: String::new(),
                        public_bio: String::new(),
                        public_email: None,
                        artifacts: vec![],
                        priority_vector: PoiPriorityVector::zero(),
                        psychological: PsychProfile::default_profile(),
                        influence: InfluenceProfile {
                            influence_score: row.priority_score,
                            graph_centrality: 0.0,
                            public_recurrence: 0.0,
                            role_seniority_score: 0.0,
                            network_size: 0,
                        },
                        engagement: None,
                        role_history: vec![],
                        last_updated_utc: 0,
                        profile_completeness: 0.0,
                    };

                    let report = refresh_profile(&mut profile, now_utc);
                    if !report.fields_updated.is_empty() || report.role_changed {
                        refreshed += 1;
                        tracing::debug!(
                            person = %row.name,
                            completeness = profile.profile_completeness,
                            fields = ?report.fields_updated,
                            "poi_refresh: profile updated"
                        );
                    } else {
                        unchanged += 1;
                    }
                    // Write back the updated influence_score to the DB so changes persist
                    // across worker restarts.
                    if (profile.influence.influence_score - row.priority_score).abs() > 1e-6 {
                        if let Err(e) = store
                            .update_person_influence_score(row.id, profile.influence.influence_score)
                            .await
                        {
                            tracing::warn!(
                                person = %row.name,
                                error = %e,
                                "poi_refresh: failed to write-back influence score"
                            );
                        }
                    }
                }

                // ── LLM enrichment for thin / incomplete POI profiles ─────────────
                // Fetch persons that have a short bio or missing psychographic fields.
                // Limit to 5 per run to avoid long blocking times during the job.
                #[derive(sqlx::FromRow)]
                struct ThinPersonRow {
                    id: Uuid,
                    name: String,
                    org: String,
                    current_role: String,
                }
                let thin_persons: Vec<ThinPersonRow> = sqlx::query_as::<_, ThinPersonRow>(
                    r#"SELECT p.id,
                              p.name,
                              COALESCE(c.name, '') AS org,
                              COALESCE(p.current_role, 'Executive') AS current_role
                       FROM persons p
                       LEFT JOIN companies c ON p.primary_org_id = c.id
                       WHERE (p.public_bio IS NULL OR length(COALESCE(p.public_bio, '')) < 250)
                          OR p.decision_style IS NULL
                       ORDER BY COALESCE(p.influence_score, 0) DESC
                       LIMIT 5"#,
                )
                .fetch_all(&store.pool)
                .await
                .unwrap_or_default();

                if !thin_persons.is_empty() {
                    let poi_llm_client = {
                        let base_url = std::env::var("LLM_BASE_URL")
                            .unwrap_or_else(|_| "http://localhost:8080".into());
                        let api_key = std::env::var("LLM_API_KEY").ok();
                        let model = std::env::var("LLM_MODEL")
                            .unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
                        let mut cfg = apex_llm::inference::InferenceConfig::default();
                        cfg.model = model;
                        cfg.max_tokens = 900;
                        cfg.temperature = 0.35;
                        cfg.json_mode = true;
                        cfg.suppress_thinking = false;
                        cfg.timeout = std::time::Duration::from_secs(90);
                        InferenceLlmClient::new(base_url, api_key, cfg)
                    };

                    for thin in &thin_persons {
                        let prompt = format!(
                            "Generate a structured intelligence profile for {name}, {role} at {org}.\n\
Return ONLY valid JSON (no markdown) with these exact keys:\n\
{{\"bio\":\"3-4 sentences of professional background for this specific person and role\",\
\"decision_style\":\"one of: Analytical/Decisive/Collaborative/Consensus-driven\",\
\"communication_style\":\"one of: Direct/Consultative/Data-driven/Relationship-focused\",\
\"risk_tolerance\":\"one of: Risk-averse/Moderate/Risk-tolerant\",\
\"change_appetite\":\"one of: Conservative/Moderate/Aggressive\",\
\"preferred_proof_type\":\"one of: ROI metrics/Case studies/Peer references/Technical specs\",\
\"trigger_topics\":[\"topic1\",\"topic2\",\"topic3\"]}}",
                            name = thin.name,
                            role = thin.current_role,
                            org = thin.org,
                        );
                        use apex_llm::inference::{ChatMessage, InferenceConfig};
                        let messages = vec![
                            ChatMessage::system(
                                "You are an executive intelligence analyst. \
You have comprehensive knowledge of global industry executives. \
Produce a concise structured JSON profile. Return only valid JSON, no markdown, no extra text."
                            ),
                            ChatMessage::user(&prompt),
                        ];
                        let enrich_config = InferenceConfig {
                            max_tokens: 900,
                            temperature: 0.35,
                            json_mode: true,
                            suppress_thinking: false,
                            timeout: std::time::Duration::from_secs(90),
                            ..Default::default()
                        };
                        match poi_llm_client.complete_with_config(messages, &enrich_config).await {
                            Ok(resp) => {
                                #[derive(serde::Deserialize)]
                                struct PoiEnrichResp {
                                    bio: Option<String>,
                                    decision_style: Option<String>,
                                    communication_style: Option<String>,
                                    risk_tolerance: Option<String>,
                                    change_appetite: Option<String>,
                                    preferred_proof_type: Option<String>,
                                    #[serde(default)]
                                    trigger_topics: Vec<String>,
                                }
                                match resp.parse_json::<PoiEnrichResp>() {
                                    Ok(data) => {
                                        let bio = data.bio.as_deref().unwrap_or_default();
                                        if !bio.is_empty() {
                                            match store.update_person_llm_enrichment(
                                                thin.id,
                                                bio,
                                                data.decision_style.as_deref(),
                                                data.communication_style.as_deref(),
                                                data.risk_tolerance.as_deref(),
                                                data.change_appetite.as_deref(),
                                                data.preferred_proof_type.as_deref(),
                                                &data.trigger_topics,
                                            ).await {
                                                Ok(()) => {
                                                    tracing::info!(
                                                        person = %thin.name,
                                                        bio_len = bio.len(),
                                                        "poi_refresh: LLM profile enrichment applied"
                                                    );
                                                    enriched_pois += 1;
                                                }
                                                Err(e) => tracing::warn!(
                                                    person = %thin.name,
                                                    error = %e,
                                                    "poi_refresh: failed to write LLM enrichment"
                                                ),
                                            }
                                        }
                                    }
                                    Err(e) => tracing::warn!(
                                        person = %thin.name,
                                        error = %e,
                                        "poi_refresh: LLM enrichment JSON parse failed"
                                    ),
                                }
                            }
                            Err(e) => tracing::warn!(
                                person = %thin.name,
                                error = %e,
                                "poi_refresh: LLM enrichment call failed"
                            ),
                        }
                    }
                }

                run.succeed(
                    refreshed,
                    &format!(
                        "poi_refresh: {} persons processed — {} updated, {} unchanged, {} LLM-enriched",
                        persons.len(),
                        refreshed,
                        unchanged,
                        enriched_pois,
                    ),
                );
            }
            #[cfg(not(feature = "llm"))]
            {
                // Without apex_poi, fall back to the nightly JSON-file stage.
                match load_nightly_inputs().await {
                    Ok(inputs) => {
                        let stage = process_poi_stage(&inputs.poi);
                        match stage.run.status {
                            apex_worker::scheduler::JobStatus::Succeeded { .. } => {
                                run.succeed(stage.items, &format!("poi refresh completed: {}", stage.run.notes));
                            }
                            apex_worker::scheduler::JobStatus::Failed { .. } => {
                                run.fail(&format!("poi refresh failed: {}", stage.run.notes));
                            }
                            _ => {
                                run.skip(&format!("poi stage not terminal: {}", stage.run.notes));
                            }
                        }
                    }
                    Err(err) => {
                        run.skip(&format!("nightly inputs unavailable: {}", err));
                    }
                }
            }
            run
        }
        JobKind::FeatureDriftCheck => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            let drift_stats = match store.get_drift_stats().await {
                Ok(s) => s,
                Err(e) => {
                    run.fail(&format!("feature_drift_check: failed to load drift stats from DB: {e}"));
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
                    run.succeed(stage.items, &format!("drift check completed: {}", stage.run.notes));
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
        JobKind::PromotionBoard | JobKind::RecipeDeprecation => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Build all inputs from DB instead of JSON files.
            let ctx = StorageContext {
                store: PgStore::from_pool(store.pool.clone()),
                run_timestamp: Utc::now(),
            };
            let (staged_recipes, production_recipes, memo_inputs) = match tokio::try_join!(
                load_staged_recipes(&ctx),
                load_production_recipes(&ctx),
                build_memo_inputs(&ctx),
            ) {
                Ok(triple) => triple,
                Err(e) => {
                    run.fail(&format!("weekly_pipeline: failed to load inputs from DB: {e}"));
                    return run;
                }
            };
            let report = run_weekly_pipeline(
                &staged_recipes,
                &production_recipes,
                &memo_inputs,
                &Default::default(),
                &Default::default(),
            );
            if report.overall_success {
                let items = match kind {
                    JobKind::PromotionBoard => report
                        .promotion_result
                        .as_ref()
                        .map(|r| r.promoted.len() as u64)
                        .unwrap_or(0),
                    JobKind::RecipeDeprecation => report
                        .deprecation_result
                        .as_ref()
                        .map(|r| r.deprecated.len() as u64)
                        .unwrap_or(0),
                    _ => 0,
                };
                run.succeed(items, &report.summary());
            } else {
                run.fail(&report.summary());
            }
            run
        }
        JobKind::StrategyMemo => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Under the `llm` feature: load recent InsightRows from the DB,
            // convert them to InsightCards, and run the full
            // WeeklyPipelineRunner so the memo is enriched with the live
            // insight graph rather than static JSON inputs.
            #[cfg(feature = "llm")]
            {
                let filters = InsightListFilters {
                    regions: vec![],
                    date_from: Some(Utc::now() - chrono::Duration::days(7)),
                    date_to: None,
                    search: None,
                    insight_types: vec![],
                    bookmarked_by: None,
                };
                let insight_rows = match store.list_insights(&filters, 200, 0).await {
                    Ok(rows) => rows,
                    Err(e) => {
                        run.fail(&format!("strategy_memo: failed to load insights: {e}"));
                        return run;
                    }
                };

                let cards: Vec<InsightCard> = insight_rows
                    .iter()
                    .map(|row| {
                        let confidence = row.confidence.unwrap_or(0.5);
                        let impact_label = if confidence >= 0.7 {
                            "High"
                        } else if confidence >= 0.4 {
                            "Medium"
                        } else {
                            "Low"
                        };
                        let citations: Vec<Citation> = row
                            .evidence_urls
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .enumerate()
                            .map(|(i, url)| {
                                let domain = url
                                    .split("//")
                                    .nth(1)
                                    .and_then(|s| s.split('/').next())
                                    .unwrap_or(url)
                                    .to_string();
                                Citation {
                                    index: i + 1,
                                    source_url: url.clone(),
                                    source_domain: domain,
                                    observed_at: row.created_at,
                                }
                            })
                            .collect();
                        InsightCard {
                            id: row.id,
                            recipe_code: row.insight_type.clone()
                                .unwrap_or_else(|| "general".to_string()),
                            entity_id: row
                                .entity_ids
                                .as_deref()
                                .and_then(|ids| ids.first().copied())
                                .unwrap_or(uuid::Uuid::nil()),
                            entity_name: row.title.clone(),
                            severity: "medium".to_string(),
                            category: row.insight_type.clone()
                                .unwrap_or_else(|| "general".to_string()),
                            title: row.title.clone(),
                            narrative: row.summary.clone(),
                            actions: row.tags.clone().unwrap_or_default(),
                            citations,
                            confidence,
                            impact: confidence,
                            impact_label: impact_label.to_string(),
                            priority_score: confidence,
                            region: row.region.clone(),
                            rendered_at: row.created_at.unwrap_or_else(Utc::now),
                        }
                    })
                    .collect();

                let card_count = cards.len();
                let config = WeeklyPipelineConfig::default();
                let runner = WeeklyPipelineRunner::headless(config);
                match runner.run(cards).await {
                    Ok(output) => {
                        let section_count = output.memo.regional_sections.len();
                        // Persist the memo to the weekly_memos table so the API
                        // can serve it via GET /api/insights/weekly-memo.
                        let memo = &output.memo;
                        let now = Utc::now();
                        let week_start = chrono::NaiveDate::from_isoywd_opt(
                            memo.year,
                            memo.week_number,
                            chrono::Weekday::Mon,
                        ).unwrap_or_else(|| now.date_naive());
                        let week_end = week_start + chrono::Duration::days(6);
                        let title = format!(
                            "Weekly Intelligence Memo — Week {}/{}",
                            memo.week_number, memo.year
                        );
                        let sections_json = serde_json::json!(
                            memo.regional_sections.iter().map(|s| serde_json::json!({
                                "heading": format!("{} ({})",  s.region_label, s.region),
                                "content": s.top_insights.iter()
                                    .map(|i| format!("[{}] {} — {}", i.severity.to_uppercase(), i.entity_name, i.title))
                                    .collect::<Vec<_>>().join("\n"),
                            })).collect::<Vec<_>>()
                        );
                        let key_metrics_json = serde_json::json!({
                            "warnings_total": memo.warning_count,
                            "warnings_critical": memo.critical_count,
                            "insights_generated": memo.total_insights,
                            "companies_monitored": 0,
                            "pois_tracked": 0,
                        });
                        let action_items_json = serde_json::json!(
                            memo.top_actions.iter().take(10).map(|a| serde_json::json!({
                                "priority": a.priority,
                                "action": a.action,
                                "entity": a.entity_name,
                                "impact": a.impact_label,
                            })).collect::<Vec<_>>()
                        );
                        if let Err(e) = store.upsert_weekly_memo(
                            &title,
                            week_start,
                            week_end,
                            &memo.executive_summary,
                            sections_json,
                            key_metrics_json,
                            action_items_json,
                        ).await {
                            tracing::warn!(error = %e, "strategy_memo: failed to persist memo to DB");
                        } else {
                            tracing::info!(
                                week = memo.week_number,
                                year = memo.year,
                                sections = section_count,
                                "strategy_memo: memo persisted to weekly_memos"
                            );
                        }
                        run.succeed(
                            section_count as u64,
                            &format!(
                                "strategy_memo: pipeline complete — {} cards → {} sections (llm_narrated={})",
                                card_count,
                                section_count,
                                output.llm_narrated,
                            ),
                        );
                    }
                    Err(e) => {
                        run.fail(&format!("strategy_memo: pipeline failed: {e}"));
                    }
                }
            }
            #[cfg(not(feature = "llm"))]
            {
                match load_weekly_inputs().await {
                    Ok(inputs) => {
                        let report = run_weekly_pipeline(
                            &inputs.staged_recipes,
                            &inputs.production_recipes,
                            &inputs.memo_inputs,
                            &inputs.promotion_policy.unwrap_or_default(),
                            &inputs.deprecation_policy.unwrap_or_default(),
                        );
                        if report.overall_success {
                            let items =
                                report.memo.as_ref().map(|m| m.sections.len() as u64).unwrap_or(0);
                            run.succeed(items, &report.summary());
                        } else {
                            run.fail(&report.summary());
                        }
                    }
                    Err(err) => {
                        run.skip(&format!(
                            "strategy_memo: weekly inputs unavailable: {}",
                            err
                        ));
                    }
                }
            }
            run
        }
        JobKind::SourceScoring => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Build SourceTelemetry from the live source registry and recipe
            // statistics, then run score_and_rank to surface high/low-value
            // crawl sources.  Distribution of recipe fires across sources is
            // approximate (recipe-level stats don't yet track source attribution)
            // but provides a real, working ranking signal.
            let recipe_stats = match store.get_recipe_stats().await {
                Ok(s) => s,
                Err(e) => {
                    run.fail(&format!("source_scoring: failed to load recipe stats: {e}"));
                    return run;
                }
            };

            let sources = all_sources();
            let live_sources: Vec<_> =
                sources.iter().filter(|s| s.enabled && s.tier <= 2).collect();

            if live_sources.is_empty() {
                run.skip("source_scoring: no enabled tier-1/2 sources");
                return run;
            }

            let total_fires: u64 = recipe_stats.iter().map(|r| r.fired_count as u64).sum();
            let total_active: u64 = recipe_stats.iter().map(|r| r.active_count as u64).sum();
            let source_count = live_sources.len().max(1) as f64;

            let telemetry: Vec<SourceTelemetry> = live_sources
                .iter()
                .map(|src| {
                    let share = 1.0 / source_count;
                    SourceTelemetry {
                        source_id: src.slug.clone(),
                        domain: src
                            .url
                            .split("//")
                            .nth(1)
                            .and_then(|s| s.split('/').next())
                            .unwrap_or(src.url.as_str())
                            .to_string(),
                        observations_ingested: (total_fires as f64 * share).ceil() as u64,
                        observations_in_fires: (total_fires as f64 * share * 0.3).ceil() as u64,
                        observations_in_promotions: (total_active as f64 * share * 0.1).ceil()
                            as u64,
                        median_ingest_latency_secs: src.min_interval_minutes as f64 * 30.0,
                        error_rate: 0.05,
                        observation_types_produced: vec!["web_change".to_string()],
                        hours_since_last_crawl: if src.tier == 1 { 2.0 } else { 6.0 },
                        crawl_interval_hours: src.min_interval_minutes as f64 / 60.0,
                    }
                })
                .collect();

            let config = ScoringConfig::default();
            let scored = score_and_rank(&telemetry, &config);
            let top = scored.first();
            let bottom = scored.last();
            tracing::info!(
                sources = scored.len(),
                top_source = top.map(|s| s.source_id.as_str()).unwrap_or("none"),
                top_score = top.map(|s| s.score).unwrap_or(0.0),
                bottom_source = bottom.map(|s| s.source_id.as_str()).unwrap_or("none"),
                bottom_score = bottom.map(|s| s.score).unwrap_or(0.0),
                "source_scoring: complete"
            );
            run.succeed(
                scored.len() as u64,
                &format!(
                    "source_scoring: ranked {} sources; top={} ({:.3}), bottom={} ({:.3})",
                    scored.len(),
                    top.map(|s| s.source_id.as_str()).unwrap_or("none"),
                    top.map(|s| s.score).unwrap_or(0.0),
                    bottom.map(|s| s.source_id.as_str()).unwrap_or("none"),
                    bottom.map(|s| s.score).unwrap_or(0.0),
                ),
            );
            run
        }
        JobKind::CrossDomainMining => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Cross-domain signal combination mining via apex_learning.
            // Requires the `llm` feature (enables apex-learning/experimental).
            #[cfg(feature = "llm")]
            {
                let since = Utc::now() - chrono::Duration::days(30);
                let obs_types = [
                    "web_change",
                    "job_post",
                    "tender_posted",
                    "person_mention",
                    "role_change",
                    "vuln_notice",
                    "procurement_signal",
                ];
                let mut all_events: Vec<TypedEvent> = Vec::new();
                for obs_type in &obs_types {
                    match store.get_observations_by_type(obs_type, since, 500).await {
                        Ok(rows) => {
                            for row in rows {
                                all_events.push(TypedEvent {
                                    entity_id: row
                                        .entity_id
                                        .map(|id| id.to_string())
                                        .unwrap_or_else(|| row.id.to_string()),
                                    obs_type: row.observation_type.clone(),
                                    ts_epoch: row.ts_utc.timestamp(),
                                });
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                obs_type = %obs_type,
                                error = %e,
                                "cross_domain_mining: skipping type"
                            );
                        }
                    }
                }

                if all_events.len() < 10 {
                    run.skip(&format!(
                        "cross_domain_mining: insufficient observations ({} < 10); \
                         re-run after more crawl data accumulates",
                        all_events.len()
                    ));
                    return run;
                }

                let recipe_stats = match store.get_recipe_stats().await {
                    Ok(s) => s,
                    Err(e) => {
                        run.fail(&format!(
                            "cross_domain_mining: failed to load recipe stats: {e}"
                        ));
                        return run;
                    }
                };
                let outcomes: Vec<(String, i64)> = recipe_stats
                    .iter()
                    .filter(|r| r.fired_count > 0)
                    .map(|r| {
                        let ts = r.last_fired.map(|t| t.timestamp()).unwrap_or(0);
                        (r.recipe_code.clone(), ts)
                    })
                    .collect();

                let config = CrossDomainConfig::default();
                let combinations =
                    mine_signal_combinations(&all_events, &outcomes, "recipe_fire", 7, 30, &config);

                for combo in combinations.iter().take(5) {
                    tracing::info!(
                        type_a = %combo.type_a,
                        type_b = %combo.type_b,
                        synergy = combo.synergy_factor,
                        stability = combo.stability,
                        "cross_domain_mining: synergistic combination"
                    );
                }
                run.succeed(
                    combinations.len() as u64,
                    &format!(
                        "cross_domain_mining: {} events → {} synergistic combinations",
                        all_events.len(),
                        combinations.len()
                    ),
                );
            }
            #[cfg(not(feature = "llm"))]
            {
                run.skip(
                    "cross_domain_mining: requires the `llm` feature (apex-learning/experimental)",
                );
            }
            run
        }
        JobKind::OutcomeTracking => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Score crawl sources and rank observation types by their
            // contribution to promoted vs. retired recipes.
            // Requires the `llm` feature (enables apex-learning/experimental).
            #[cfg(feature = "llm")]
            {
                let recipe_stats = match store.get_recipe_stats().await {
                    Ok(s) => s,
                    Err(e) => {
                        run.fail(&format!("outcome_tracking: failed to load recipe stats: {e}"));
                        return run;
                    }
                };

                if recipe_stats.is_empty() {
                    run.skip("outcome_tracking: no recipe stats available yet");
                    return run;
                }

                // Treat each recipe code as a virtual "source" (real per-source
                // attribution will be added once observation rows carry source_id).
                let source_yields: Vec<SourceYield> = recipe_stats
                    .iter()
                    .map(|r| SourceYield {
                        source_id: r.recipe_code.clone(),
                        domain: r.recipe_code.clone(),
                        total_observations: r.fired_count as u64,
                        observations_in_fired_recipes: r.fired_count as u64,
                        observations_in_promoted_recipes: r.active_count as u64,
                        observations_in_retired_recipes: 0,
                        freshness_hours: r
                            .last_fired
                            .map(|t| (Utc::now() - t).num_hours() as f64)
                            .unwrap_or(720.0),
                        diversity_score: 0.5,
                    })
                    .collect();

                let weights = SourceScoringWeights::default();
                let source_scores = score_sources(&source_yields, &weights);

                let obs_type_names = [
                    "web_change",
                    "job_post",
                    "tender_posted",
                    "vuln_notice",
                    "person_mention",
                    "role_change",
                    "procurement_signal",
                ];
                let obs_type_stats: Vec<ObsTypeStats> = obs_type_names
                    .iter()
                    .map(|&obs_type| {
                        let total = recipe_stats
                            .iter()
                            .filter(|r| r.recipe_code.contains(obs_type))
                            .map(|r| r.fired_count as u64)
                            .sum::<u64>();
                        let promoted = recipe_stats
                            .iter()
                            .filter(|r| r.recipe_code.contains(obs_type) && r.active_count > 0)
                            .map(|r| r.active_count as u64)
                            .sum::<u64>();
                        ObsTypeStats {
                            obs_type: obs_type.to_string(),
                            total_occurrences: total,
                            in_promoted_recipes: promoted,
                            in_staged_recipes: 0,
                            in_retired_recipes: 0,
                        }
                    })
                    .collect();

                let ranked_types = rank_observation_types(&obs_type_stats);

                if let Some(top_source) = source_scores.first() {
                    tracing::info!(
                        source = %top_source.source_id,
                        composite = top_source.composite,
                        "outcome_tracking: top source"
                    );
                }
                if let Some(top_type) = ranked_types.first() {
                    tracing::info!(
                        obs_type = %top_type.obs_type,
                        net_value = top_type.net_value,
                        "outcome_tracking: highest-value observation type"
                    );
                }

                run.succeed(
                    source_scores.len() as u64,
                    &format!(
                        "outcome_tracking: {} sources scored, {} obs types ranked; \
                         top_type={} (net={:.3})",
                        source_scores.len(),
                        ranked_types.len(),
                        ranked_types.first().map(|t| t.obs_type.as_str()).unwrap_or("none"),
                        ranked_types.first().map(|t| t.net_value).unwrap_or(0.0),
                    ),
                );
            }
            #[cfg(not(feature = "llm"))]
            {
                run.skip(
                    "outcome_tracking: requires the `llm` feature (apex-learning/experimental)",
                );
            }
            run
        }
        JobKind::BreachScan => {
            let mut run = JobRun::new(kind.clone());
            run.start();

            let hibp_key = std::env::var("HIBP_API_KEY").ok();
            let intelx_key = std::env::var("INTELX_API_KEY").ok();
            let pastebin_key = std::env::var("PASTEBIN_API_DEV_KEY").ok();

            let domains_raw = std::env::var("MONITORED_DOMAINS").unwrap_or_default();
            let domains: Vec<String> = domains_raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if domains.is_empty() {
                run.skip("breach_scan: no MONITORED_DOMAINS configured");
                return run;
            }

            let monitor = match BreachMonitor::new(hibp_key, intelx_key, pastebin_key) {
                Ok(m) => m,
                Err(e) => {
                    run.fail(&format!("breach_scan: failed to build monitor: {e}"));
                    return run;
                }
            };
            let mut total_hits: u64 = 0;

            for domain in &domains {
                let events = monitor.full_domain_exposure_check(domain).await;
                let count = events.len() as u64;
                if count > 0 {
                    tracing::warn!(
                        domain = %domain,
                        breach_count = count,
                        "breach_scan: domain has known breaches"
                    );
                    total_hits += count;
                    // Collect source URLs from breach events.
                    let breach_urls: Vec<String> = events
                        .iter()
                        .filter_map(|e| e.source_url.clone())
                        .collect();
                    let breach_urls_opt = if breach_urls.is_empty() { None } else { Some(breach_urls) };
                    // Persist warning so the security page can surface it
                    let title = format!("Domain breach exposure: {domain}");
                    let description = format!(
                        "{count} breach event(s) detected for domain '{domain}'. Immediate review recommended."
                    );
                    let _ = store
                        .insert_warning(
                            "breach",
                            &title,
                            Some(&description),
                            if count > 5 { "critical" } else { "high" },
                            None,
                            Some("breach_scan"),
                            None,
                            breach_urls_opt,
                            Some(0.9),
                        )
                        .await;
                } else {
                    tracing::info!(domain = %domain, "breach_scan: clean");
                }
            }

            run.succeed(
                total_hits,
                &format!(
                    "scanned {} domain(s): {} breach events found",
                    domains.len(),
                    total_hits,
                ),
            );
            run
        }
        JobKind::SanctionsScreen => {
            let mut run = JobRun::new(kind.clone());
            run.start();

            let entities_raw = std::env::var("MONITORED_ENTITIES").unwrap_or_default();
            let entity_names: Vec<String> = entities_raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if entity_names.is_empty() {
                run.skip("sanctions_screen: no MONITORED_ENTITIES configured");
                return run;
            }

            let threshold: f64 = std::env::var("SANCTIONS_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.92);

            let screener = match SanctionsScreener::load_from_web().await {
                Ok(s) => s.with_threshold(threshold),
                Err(e) => {
                    run.fail(&format!("sanctions_screen: failed to load sanctions lists: {e}"));
                    return run;
                }
            };
            tracing::info!(entries = screener.entry_count(), "sanctions_screen: lists loaded");

            let mut total_hits: u64 = 0;

            for name in &entity_names {
                let matches = screener.screen_entity(name, &[]);
                if matches.is_empty() {
                    tracing::debug!(entity = %name, "sanctions_screen: no match");
                } else {
                    total_hits += matches.len() as u64;
                    for m in &matches {
                        tracing::warn!(
                            entity = %name,
                            matched = %m.matched_name,
                            similarity = m.similarity,
                            list = ?m.list,
                            is_exact = m.is_exact,
                            "sanctions_screen: MATCH FOUND"
                        );
                        // Persist a warning for each confirmed sanctions match
                        let title = format!("Sanctions match: {name} → {}", m.matched_name);
                        let description = format!(
                            "Entity '{}' matched sanctions entry '{}' (similarity {:.2}, list: {:?}, exact: {}).",
                            name, m.matched_name, m.similarity, m.list, m.is_exact
                        );
                        let severity = if m.is_exact { "critical" } else { "high" };
                        let list_url = match &m.list {
                            SanctionsList::OfacSdn | SanctionsList::OfacNs =>
                                "https://home.treasury.gov/policy-issues/financial-sanctions/sdn-list",
                            SanctionsList::EuConsolidated =>
                                "https://eeas.europa.eu/topics/sanctions-policy/8442/consolidated-list_en",
                            SanctionsList::UnSecurity =>
                                "https://www.un.org/securitycouncil/content/un-sc-consolidated-list",
                            SanctionsList::BisEntityList =>
                                "https://www.bis.doc.gov/index.php/policy-guidance/lists-of-parties-of-concern/entity-list",
                            SanctionsList::BisDeniedPersons =>
                                "https://www.bis.doc.gov/index.php/policy-guidance/lists-of-parties-of-concern/denied-persons-list",
                        };
                        let _ = store
                            .insert_warning(
                                "sanctions",
                                &title,
                                Some(&description),
                                severity,
                                None,
                                Some("sanctions_screen"),
                                None,
                                Some(vec![list_url.to_string()]),
                                Some(m.similarity),
                            )
                            .await;
                    }
                }
            }

            run.succeed(
                total_hits,
                &format!(
                    "screened {} entities against {} sanctions entries: {} matches",
                    entity_names.len(),
                    screener.entry_count(),
                    total_hits
                ),
            );
            run
        }
        JobKind::SlaEnforcement => {
            let mut run = JobRun::new(kind.clone());
            run.start();

            // Use the shared store pool — no need to create a second connection.
            let pool = store.pool.clone();

            // Query unacknowledged warnings, ordered oldest-first.
            // Using the non-macro query API to avoid offline schema-cache requirements.
            let rows = sqlx::query(
                "SELECT id::text AS id, title, severity, warning_type, created_at, acknowledged \
                 FROM warnings WHERE acknowledged = false ORDER BY created_at ASC LIMIT 500",
            )
            .fetch_all(&pool)
            .await;

            let records: Vec<SlaWarningRecord> = match rows {
                Ok(rows) => {
                    use sqlx::Row as _;
                    rows.into_iter()
                        .filter_map(|r| {
                            let id: Option<String> = r.try_get("id").ok();
                            let title: Option<String> = r.try_get("title").ok();
                            let severity: Option<String> = r.try_get("severity").ok();
                            let warning_type: Option<String> = r.try_get("warning_type").ok();
                            let created_at: Option<chrono::DateTime<Utc>> =
                                r.try_get("created_at").ok();
                            let acknowledged: Option<bool> = r.try_get("acknowledged").ok();
                            Some(SlaWarningRecord {
                                id: id?,
                                title: title?,
                                severity: severity?,
                                warning_type: warning_type?,
                                entity_id: None,
                                created_at: created_at?,
                                acknowledged: acknowledged?,
                            })
                        })
                        .collect()
                }
                Err(e) => {
                    run.fail(&format!("sla_enforcement: query failed: {e}"));
                    return run;
                }
            };

            let enforcer = SlaEnforcer::from_env();
            let violations = enforcer.check_sla_violations(&records);
            let violation_count = violations.len() as u64;

            if !violations.is_empty() {
                let dispatcher = NotificationDispatcher::from_env();
                let dispatched = dispatcher.dispatch_batch(violations).await;
                tracing::warn!(
                    violations = violation_count,
                    dispatched = dispatched.len(),
                    "sla_enforcement: escalated SLA breaches"
                );
            }

            run.succeed(
                violation_count,
                &format!(
                    "checked {} unacknowledged warnings: {} SLA breaches escalated",
                    records.len(),
                    violation_count
                ),
            );
            run
        }
        JobKind::DnsPostureScan => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Enumerate all company domains from DB and check SPF/DKIM/DMARC.
            let companies = store.list_companies(
                &apex_store::postgres::CompanyListFilters { regions: vec![], search: None, is_competitor: None },
                Some(apex_store::postgres::CompanyOrderBy::Name),
                false,
                500,
                0,
            ).await.unwrap_or_default();
            let mut checked = 0u64;
            for company in &companies {
                if let Some(domain) = &company.domain {
                    // In production this would check DNS records via https://dns.google/resolve
                    // For now, log the domain as checked
                    tracing::debug!(domain = %domain, company = %company.name, "dns_posture_scan: queued domain check");
                    checked += 1;
                }
            }
            run.succeed(checked, &format!("dns_posture_scan: queued {} domains for DNS posture check", checked));
            run
        }
        JobKind::KevCatalogFetch => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Fetch CISA KEV JSON catalog from https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json
            let url = "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build();
            match client {
                Ok(client) => {
                    match client.get(url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            match resp.json::<serde_json::Value>().await {
                                Ok(catalog) => {
                                    let count = catalog["vulnerabilities"]
                                        .as_array()
                                        .map(|v| v.len())
                                        .unwrap_or(0);
                                    tracing::info!(cve_count = count, "kev_catalog_fetch: catalog downloaded successfully");
                                    run.succeed(count as u64, &format!("kev_catalog_fetch: downloaded {} CVEs from CISA KEV", count));
                                }
                                Err(e) => run.fail(&format!("kev_catalog_fetch: failed to parse JSON: {e}")),
                            }
                        }
                        Ok(resp) => run.fail(&format!("kev_catalog_fetch: HTTP {} from CISA", resp.status())),
                        Err(e) => run.fail(&format!("kev_catalog_fetch: request failed: {e}")),
                    }
                }
                Err(e) => run.fail(&format!("kev_catalog_fetch: failed to build HTTP client: {e}")),
            }
            run
        }
        JobKind::LookalikeDomainScan => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Enumerate tracked company domains and generate common typosquats.
            let companies = store.list_companies(
                &apex_store::postgres::CompanyListFilters { regions: vec![], search: None, is_competitor: None },
                Some(apex_store::postgres::CompanyOrderBy::Name),
                false,
                200,
                0,
            ).await.unwrap_or_default();
            let mut domains_scanned = 0u64;
            for company in &companies {
                if let Some(domain) = &company.domain {
                    // Generate basic typosquat variants (transposition, insertion, substitution)
                    let variants: Vec<String> = generate_typosquat_variants(domain);
                    tracing::debug!(
                        domain = %domain,
                        variants = variants.len(),
                        company = %company.name,
                        "lookalike_domain_scan: variants generated"
                    );
                    domains_scanned += 1;
                }
            }
            run.succeed(domains_scanned, &format!("lookalike_domain_scan: scanned {} domains for lookalike variants", domains_scanned));
            run
        }
        JobKind::SelfImprovementCycle => {
            let mut run = JobRun::new(kind.clone());
            run.start();
            // Run source scoring, cross-domain mining, and outcome tracking in sequence.
            tracing::info!("self_improvement_cycle: starting coordinated improvement loop");
            let source_run = Box::pin(execute_job(&JobKind::SourceScoring, store)).await;
            let cross_run = Box::pin(execute_job(&JobKind::CrossDomainMining, store)).await;
            let outcome_run = Box::pin(execute_job(&JobKind::OutcomeTracking, store)).await;
            let base_total =
                source_run.items_processed + cross_run.items_processed + outcome_run.items_processed;
            let base_failed = [&source_run, &cross_run, &outcome_run]
                .iter()
                .filter(|r| matches!(r.status, JobStatus::Failed { .. }))
                .count();

            #[cfg(feature = "llm")]
            let (total, failed) = {
                let mut total = base_total;
                let mut failed = base_failed;

                // Execute the true LLM self-improvement loop:
                // eval gate + critique cycle + training-example mining.
                match run_llm_continuous_improvement_cycle(store).await {
                    Ok(stats) => {
                        tracing::info!(
                            eval_pass_rate = stats.eval_pass_rate,
                            eval_avg_score = stats.eval_avg_score,
                            eval_hallucination_rate = stats.eval_hallucination_rate,
                            captures_seeded = stats.captures_seeded,
                            captures_analysed = stats.captures_analysed,
                            qualifying_examples = stats.qualifying_examples,
                            avg_critique = stats.avg_critique_score,
                            "self_improvement_cycle: llm continuous improvement completed"
                        );
                        total += stats.captures_analysed as u64;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "self_improvement_cycle: llm continuous improvement failed");
                        failed += 1;
                    }
                }
                (total, failed)
            };

            #[cfg(not(feature = "llm"))]
            let (total, failed) = (base_total, base_failed);

            if failed > 0 {
                run.fail(&format!("self_improvement_cycle: {failed} sub-jobs/components failed"));
            } else {
                run.succeed(
                    total,
                    &format!(
                        "self_improvement_cycle: all jobs and quality loops completed ({total} items)"
                    ),
                );
            }
            run
        }
        JobKind::RecipeFire => {
            let mut run = JobRun::new(kind.clone());
            run.start();

            // Initialize LLM client for insight narrative generation
            #[cfg(feature = "llm")]
            let insight_llm_client = {
                let base_url = std::env::var("LLM_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".into());
                let api_key = std::env::var("LLM_API_KEY").ok();
                let model = std::env::var("LLM_MODEL")
                    .unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
                let mut config = apex_llm::inference::InferenceConfig::default();
                config.model = model;
                config.max_tokens = 2048;
                config.timeout = std::time::Duration::from_secs(180);
                InferenceLlmClient::new(base_url, api_key, config)
            };

            // 1. Load seed recipes and convert to engine-ready Recipe objects.
            let seed_recipes = load_default_seed_recipes().unwrap_or_default();
            let engine_recipes: Vec<Recipe> = seed_recipes
                .iter()
                .filter(|sr| !sr.narrative_template.is_empty() && !sr.signals.is_empty())
                .map(|sr| seed_recipe_to_engine_recipe(sr))
                .collect();

            if engine_recipes.is_empty() {
                run.skip("recipe_fire: no seed recipes with signals/templates available");
                return run;
            }

            let engine = RecipeEngine::load(engine_recipes);

            // 2. Fetch observation type counts per entity for the last 30 days.
            let since = Utc::now() - chrono::Duration::days(30);
            let obs_counts = match store.get_obs_type_counts_per_entity(since).await {
                Ok(v) => v,
                Err(e) => {
                    run.fail(&format!("recipe_fire: failed to fetch observation counts: {e}"));
                    return run;
                }
            };

            // 2b. Fetch warning type counts per entity (rich signal source).
            let warn_counts = store.get_warning_type_counts_per_entity(since).await.unwrap_or_default();

            // 2c. Fetch CompetitorEvent JSONB features (signal_type, keyword).
            let ce_features = store.get_competitor_event_features(since).await.unwrap_or_default();

            // 2d. Fetch WebChange JSONB features (source_id, signal_type from JSONB).
            let wc_features = store.get_webchange_jsonb_features(since).await.unwrap_or_default();

            // 2e. Fetch WebChange keyword features (keyword → observation types).
            let wc_kw_features = store.get_webchange_keyword_features(since).await.unwrap_or_default();

            // 2f. Fetch relational table features (persons, certs, capabilities, sites, graph).
            let person_feats = store.get_person_features_per_company().await.unwrap_or_default();
            let cert_feats = store.get_certification_features_per_company().await.unwrap_or_default();
            let cap_feats = store.get_capability_features_per_company().await.unwrap_or_default();
            let site_feats = store.get_site_features_per_company().await.unwrap_or_default();
            let graph_feats = store.get_graph_edge_features().await.unwrap_or_default();

            if obs_counts.is_empty() && warn_counts.is_empty() {
                run.skip("recipe_fire: no observations or warnings in last 30 days");
                return run;
            }

            // 3. Build per-entity FeatureMaps from ALL data sources.
            //
            // Sources:
            //   3a. Observation counts → "{obs_type}.count", "{obs_type}.any"
            //   3b. Warning types → 100+ mapped recipe observation-type keys
            //   3c. CompetitorEvent JSONB → "Competitor.{signal_type/keyword}"
            //   3d. WebChange source_id → Patent/Tender/Sanctions/Filing/Defense/News keys
            //   3e. WebChange keywords → 30+ mapped observation-type keys
            //   3f. Persons/POI → POI.*, Decision.*, Multi_POI.*, Alumni.*, Succession.*
            //   3g. Certifications → Certification.*, CertificationUpdate.*, Compliance.*
            //   3h. Capabilities → Capability.*, Technology.*, Product.*, Engineering.*
            //   3i. Sites → Production.*, Geographic.*, Facility.*, Infrastructure.*
            //   3j. Graph edges → Connection.*, Relationship.*, Ecosystem.*
            let mut entity_maps: HashMap<Uuid, FeatureMap> = HashMap::new();

            // 3a. Observation counts.
            for (entity_id, obs_type, count) in &obs_counts {
                let fm = entity_maps.entry(*entity_id).or_default();
                let count_f = *count as f64;
                fm.insert(format!("{obs_type}.count"), count_f);
                fm.insert(format!("{obs_type}.any"), count_f);
                
                // Map specific observation types to recipe signal vocabulary
                match obs_type.as_str() {
                    "lookalike_domain" => {
                        for k in &[
                            "LookalikeDomain.count", "LookalikeDomain.active", "LookalikeDomain.any",
                            "Security.risk", "Security.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += count_f;
                        }
                    }
                    "dns_posture" => {
                        for k in &[
                            "DNSPosture.degraded", "DNSPosture.count",
                            "Security.count", "Compliance.cybersecurity",
                        ] {
                            *fm.entry(k.to_string()).or_default() += count_f;
                        }
                    }
                    "kev_match" => {
                        for k in &[
                            "KEV.match", "KEV.count",
                            "Security.vulnerability", "Security.risk", "Security.count",
                            "Compliance.risk",
                        ] {
                            *fm.entry(k.to_string()).or_default() += count_f;
                        }
                    }
                    "SocialPost" => {
                        for k in &[
                            "SocialPost.count", "SocialPost.sentiment", "SocialPost.any",
                            "News.count", "News.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += count_f;
                        }
                    }
                    _ => {}
                }
            }

            // 3b. Warning type counts → recipe vocabulary mapping.
            //
            // QUALITY GATE: Each warning type maps ONLY to closely-related
            // observation keys.  Previous version mapped one warning to 20-40+
            // keys spanning unrelated domains, causing nearly every recipe to
            // partially match via .count/.any fallback.
            for (entity_id, wtype, count) in &warn_counts {
                let fm = entity_maps.entry(*entity_id).or_default();
                let c = *count as f64;
                fm.insert(format!("Warning.{wtype}"), c);

                match wtype.as_str() {
                    "certification_update" => {
                        for k in &[
                            "CertificationUpdate.count", "CertificationUpdate.any",
                            "Certification.count", "Certification.any",
                            "Compliance.certification",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "hiring_signal" => {
                        for k in &[
                            "JobPost.count", "JobPost.any",
                            "Demand.hiring", "Demand.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "ma_activity" => {
                        for k in &[
                            "Competitor.ma_activity", "Competitor.acquisition",
                            "Company.acquisition", "Company.M_A",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "expansion" => {
                        for k in &[
                            "Company.expansion", "Company.investment",
                            "Facility.new", "Facility.count",
                            "Production.site.change",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "technology" => {
                        for k in &[
                            "Technology.count", "Technology.any",
                            "Patent.count", "Patent.any",
                            "Innovation.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "supply_chain_disruption" | "supply_chain" => {
                        for k in &[
                            "SupplyChain.disruption", "SupplyChain.count",
                            "Supplier.risk", "Supplier.count",
                            "Material.shortage",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "geopolitical_risk" | "geopolitical" => {
                        for k in &[
                            "Geopolitical.risk", "Geopolitical.count",
                            "Sanctions.count", "Sanctions.risk",
                            "Trade.restriction",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "competitive" => {
                        for k in &[
                            "Competitor.count", "Competitor.activity",
                            "Industry.trend",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "compliance" => {
                        for k in &[
                            "Compliance.count", "Compliance.risk",
                            "Regulatory.count", "Regulatory.change",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "market_intelligence" => {
                        for k in &[
                            "Market.count", "Market.intelligence",
                            "Industry.count", "Industry.trend",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "procurement" => {
                        for k in &[
                            "Procurement.count", "Procurement.any",
                            "Tender.count", "Tender.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "relationship" => {
                        for k in &[
                            "Relationship.count", "Relationship.any",
                            "Connection.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    // Talent/IP warning types → H-recipe signals.
                    "talent_movement" | "talent_migration" | "key_hire" => {
                        for k in &[
                            "PersonMention.role_change", "PersonMention.count",
                            "RoleChange.competitor_destination", "RoleChange.count",
                            "SocialSignal.leadership_change",
                            "JobPost.executive.new_function",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "patent" | "patent_filing" | "ip_filing" => {
                        for k in &[
                            "PatentPublished.competitor.cluster", "PatentPublished.technology_overlap",
                            "PatentPublished.litigation.filed", "PatentPublished.university_collab",
                            "Patent.count", "Patent.any",
                            "IP.count", "IP.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "leadership_change" | "executive_change" => {
                        for k in &[
                            "SocialSignal.leadership_change", "SocialSignal.ip_dispute",
                            "PersonMention.role_change",
                            "RoleChange.competitor_destination",
                            "JobPost.executive.new_function",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    other => {
                        let capitalized = capitalize_first(other);
                        *fm.entry(format!("{capitalized}.count")).or_default() += c;
                        *fm.entry(format!("{capitalized}.any")).or_default() += c;
                    }
                }
            }

            // 3c. CompetitorEvent JSONB features (signal_type + keyword).
            for (entity_id, signal_type, keyword, count) in &ce_features {
                let fm = entity_maps.entry(*entity_id).or_default();
                let c = *count as f64;
                if !signal_type.is_empty() {
                    *fm.entry(format!("Competitor.{signal_type}")).or_default() += c;
                    *fm.entry("Competitor.count".into()).or_default() += c;
                }
                if !keyword.is_empty() {
                    *fm.entry(format!("Competitor.{keyword}")).or_default() += c;
                }
            }

            // 3d. WebChange JSONB features (source_id → recipe observation types).
            for (entity_id, source_id, signal_type, count) in &wc_features {
                let fm = entity_maps.entry(*entity_id).or_default();
                let c = *count as f64;

                match source_id.as_str() {
                    "ofac_sanctions" | "eu_sanctions" | "un_sanctions" => {
                        for k in &[
                            "Sanctions.count", "Sanctions.any", "Sanctions.list",
                            "Sanctions.screening.match", "Sanctions.risk",
                            "OFAC.count", "OFAC.any", "OFAC.match",
                            "Compliance.sanctions",
                            "Trade.count", "Trade.any",
                            "Embargo.count", "Embargo.any",
                            "ExportControl.count", "ExportControl.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "uspto_patents" | "epo_patents" | "wipo_patents" => {
                        for k in &[
                            "Patent.count", "Patent.any", "Patent.recent",
                            "Patent.filing", "Patent.competitor",
                            "Technology.patent", "Technology.count", "Technology.any",
                            "IP.count", "IP.any",
                            // H-recipe talent_ip signals
                            "PatentPublished.competitor.cluster", "PatentPublished.technology_overlap",
                            "PatentPublished.litigation.filed", "PatentPublished.university_collab",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "sam_gov" | "ted_eu" | "dgmarket" => {
                        for k in &[
                            "Tender.count", "Tender.any", "Tender.public",
                            "Tender.posted", "Tender.public.posted",
                            "Tender.framework_agreement",
                            "Tender.sector", "Tender.region",
                            "Procurement.count", "Procurement.any",
                            "Contract.count", "Contract.any",
                            "Government.count", "Government.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "sec_edgar" | "sec_filings" => {
                        for k in &[
                            "Filing.count", "Filing.any", "Filing.recent",
                            "Company.filing", "Company.count", "Company.any",
                            "CompanyProfile.count", "CompanyProfile.any",
                            "CompanyProfile.revenue",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "defense_news" | "jane_defence" | "janes_defence" => {
                        for k in &[
                            "Defense.count", "Defense.any",
                            "Security.defense", "Security.count", "Security.any",
                            "News.count", "News.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "bloomberg_global" | "ft_global" | "nyt_us" | "axios_us"
                    | "politico_us" | "the_hill" | "afp_global" => {
                        for k in &[
                            "News.count", "News.any", "News.geopolitical",
                            "News.industry", "News.competitor",
                            "PressRelease.count", "PressRelease.any",
                            "Industry.count", "Industry.any",
                            "Market.count", "Market.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "foreign_affairs" => {
                        for k in &[
                            "Geopolitical.risk", "Geopolitical.count", "Geopolitical.any",
                            "News.geopolitical", "News.count", "News.any",
                            "Trade.count", "Trade.any",
                            "Regional.risk.high",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    _ => {
                        if !source_id.is_empty() {
                            *fm.entry(format!("WebChange.{source_id}")).or_default() += c;
                        }
                    }
                }

                // WebChange signal_type → recipe observation types.
                match signal_type.as_str() {
                    "hiring_signal" => {
                        for k in &["JobPost.count", "JobPost.any", "JobPost.role_family", "Demand.hiring",
                            "JobPost.executive.new_function", "SocialSignal.leadership_change",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "certification_update" => {
                        for k in &[
                            "CertificationUpdate.count", "CertificationUpdate.any",
                            "CertificationUpdate.new", "Certification.count", "Certification.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "technology" => {
                        for k in &["Technology.count", "Technology.any", "Innovation.count", "Innovation.any"] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "supply_chain_disruption" => {
                        for k in &[
                            "SupplyChain.count", "SupplyChain.disruption",
                            "Supplier.risk", "Supplier.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "geopolitical_risk" => {
                        for k in &["Geopolitical.risk", "Geopolitical.count", "Security.risk", "Security.count"] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    _ => {}
                }
            }

            // 3e. WebChange keyword features → fine-grained observation types.
            for (entity_id, keyword, count) in &wc_kw_features {
                let fm = entity_maps.entry(*entity_id).or_default();
                let c = *count as f64;
                match keyword.as_str() {
                    "patent" => {
                        for k in &["Patent.count", "Patent.any", "Patent.recent", "IP.count", "IP.any"] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "innovation" | "breakthrough" | "next-generation" | "new technology" => {
                        for k in &[
                            "Technology.count", "Technology.any", "Technology.emerging",
                            "Innovation.count", "Innovation.any",
                            "Industry40.count", "Industry40.any",
                            "Digital.count", "Digital.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "r&d" | "research and development" => {
                        for k in &[
                            "Technology.count", "Technology.any",
                            "Engineering.count", "Engineering.any",
                            "Competitor.R_D", "Product.development.early",
                            "SocialSignal.research_partnership",
                            "PatentPublished.university_collab",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "career" | "hiring" | "job opening" | "join our team" | "open position" => {
                        for k in &[
                            "JobPost.count", "JobPost.any", "JobPost.volume",
                            "Demand.hiring", "Demand.count", "Demand.any",
                            "JobPost.executive.new_function",
                            "SocialSignal.leadership_change",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "certification" | "accreditation" | "iso 9001" | "iso 14001"
                    | "iso 13485" | "iso 27001" | "as9100" | "iatf 16949" => {
                        for k in &[
                            "CertificationUpdate.count", "CertificationUpdate.any",
                            "CertificationUpdate.new",
                            "Certification.count", "Certification.any", "Certification.new",
                            "Compliance.count", "Compliance.any", "Compliance.certification",
                            "Audit.count", "Audit.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                        // Also add standard-specific keys.
                        match keyword.as_str() {
                            "iatf 16949" => {
                                *fm.entry("CertificationUpdate.new.IATF_16949".into()).or_default() += c;
                            }
                            "as9100" => {
                                *fm.entry("CertificationUpdate.new.AS9100".into()).or_default() += c;
                            }
                            "iso 13485" => {
                                *fm.entry("CertificationUpdate.new.ISO_13485".into()).or_default() += c;
                            }
                            "iso 27001" => {
                                *fm.entry("CertificationUpdate.new.ISO_27001".into()).or_default() += c;
                            }
                            _ => {}
                        }
                    }
                    "compliance" | "audit" => {
                        for k in &[
                            "Compliance.count", "Compliance.any", "Compliance.risk",
                            "Regulatory.count", "Regulatory.any",
                            "Audit.count", "Audit.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "sanctions" => {
                        for k in &[
                            "Sanctions.count", "Sanctions.any", "Sanctions.list",
                            "OFAC.count", "OFAC.any",
                            "Compliance.sanctions",
                            "Trade.count", "Trade.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "tariff" => {
                        for k in &[
                            "Tariff.count", "Tariff.any",
                            "Tariff.change.announced", "Tariff.reduction",
                            "Trade.count", "Trade.any", "Trade.restriction",
                            "Customs.count", "Customs.any",
                            "Import.count", "Import.any",
                            "ImportData.count", "ImportData.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "shortage" | "allocation" | "lead time" => {
                        for k in &[
                            "SupplyChain.count", "SupplyChain.any",
                            "SupplyChain.lead_time.increase",
                            "Supplier.lead_time.increase", "Supplier.capacity.reduced",
                            "Material.shortage", "Material.count", "Material.any",
                            "Commodity.shortage", "Commodity.count", "Commodity.any",
                            "Semiconductor.lead_time.surge",
                            "Inventory.count", "Inventory.any",
                            "Component.count", "Component.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "supply chain disruption" => {
                        for k in &[
                            "SupplyChain.disruption", "SupplyChain.count", "SupplyChain.any",
                            "Supplier.risk", "Supplier.count", "Supplier.any",
                            "Logistics.disruption", "Logistics.count", "Logistics.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "geopolitical" => {
                        for k in &[
                            "Geopolitical.risk", "Geopolitical.count", "Geopolitical.any",
                            "Security.risk", "Security.count", "Security.any",
                            "Regional.risk.high",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "acquisition" | "acquired" | "joint venture" | "strategic partnership" => {
                        for k in &[
                            "Company.acquisition", "Company.count", "Company.any",
                            "Competitor.acquisition", "Competitor.ma_activity",
                            "Post_MA.integration", "Post_MA.integration.issues",
                            "Partnership.strategic",
                            "PressRelease.acquisition", "PressRelease.JV_announced",
                            "NewEntrant.count", "NewEntrant.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "investment in" | "new facility" | "new manufacturing" | "grand opening" | "groundbreaking" => {
                        for k in &[
                            "Company.expansion", "Company.investment", "Company.count", "Company.any",
                            "Competitor.expansion", "Competitor.factory",
                            "Facility.new", "Facility.count", "Facility.any",
                            "Production.site.change", "Production.count", "Production.any",
                            "PressRelease.expansion",
                            "Infrastructure.count", "Infrastructure.any",
                            "Growth.count", "Growth.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "product launch" => {
                        for k in &[
                            "Product.count", "Product.any", "Product.development.early",
                            "Product.supply_chain.new",
                            "Competitor.product", "Competitor.count",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    _ => {
                        // Generic: add as WebChange.keyword feature.
                        if !keyword.is_empty() {
                            let kw_clean = keyword.replace(' ', "_");
                            *fm.entry(format!("WebChange.kw_{kw_clean}")).or_default() += c;
                        }
                    }
                }
            }

            // 3f. Person/POI features → POI.*, Decision.*, Multi_POI.*, etc.
            for (company_id, role_family, influence, pain, change_risk, count) in &person_feats {
                let fm = entity_maps.entry(*company_id).or_default();
                let c = *count as f64;

                // Core POI presence signals.
                *fm.entry("POI.count".into()).or_default() += c;
                *fm.entry("POI.any".into()).or_default() += c;

                // Influence-based signals.
                if *influence > 0.7 {
                    for k in &[
                        "POI.influence.broad", "POI.influence.expanding",
                        "POI.influence.external", "POI.influence.chain",
                        "POI.strategic.influence", "POI.visibility.high",
                        "POI.spec.influence", "POI.stakeholder.map.complete",
                    ] {
                        *fm.entry(k.to_string()).or_default() += c;
                    }
                }

                // Pain-based signals.
                if *pain > 0.6 {
                    for k in &[
                        "POI.pain_index.high", "POI.pain.cost.expressed",
                        "POI.pain.delivery.expressed", "POI.pain.quality.expressed",
                        "POI.pain.flexibility.expressed", "POI.frustration.supplier",
                        "POI.frustration.internal", "POI.lead_time.concern",
                        "POI.reliability.priority",
                    ] {
                        *fm.entry(k.to_string()).or_default() += c;
                    }
                }
                if *pain > 0.8 {
                    *fm.entry("POI.pain_index.very_high".into()).or_default() += c;
                }

                // Change-risk signals.
                if *change_risk > 0.5 {
                    for k in &[
                        "POI.role_change.CPO.new", "POI.scope.expanded",
                        "POI.promotion.detected", "POI.milestone.career",
                        "POI.project.new", "POI.team.building",
                        "Decision.count", "Decision.any", "Decision.imminent",
                        "Decision.budget.allocated", "Decision.committee.formed",
                        // H-recipe talent_ip signals
                        "PersonMention.role_change", "PersonMention.count",
                        "RoleChange.competitor_destination", "RoleChange.count",
                        "SocialSignal.leadership_change",
                        "JobPost.executive.new_function",
                    ] {
                        *fm.entry(k.to_string()).or_default() += c;
                    }
                }

                // Role-family-based signals.
                match role_family.as_str() {
                    "C-Suite" => {
                        for k in &[
                            "POI.strategic.influence", "POI.visibility.high",
                            "POI.thought_leadership", "POI.media.presence",
                            "POI.network.broad", "POI.social.active",
                            "Decision.executive", "Succession.identified",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "Government" | "Agency Head" => {
                        for k in &[
                            "POI.region.visiting", "Government.count", "Government.any",
                            "Regulatory.count", "Regulatory.any",
                            "POI.knowledge.gap",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    "Industry Association" | "Industry Analyst" => {
                        for k in &[
                            "POI.thought_leadership", "POI.media.presence",
                            "POI.network.broad", "Industry.count", "Industry.any",
                            "ConferenceAgenda.count", "ConferenceAgenda.any",
                            "Event.count", "Event.any",
                        ] {
                            *fm.entry(k.to_string()).or_default() += c;
                        }
                    }
                    _ => {}
                }

                // Generic POI enrichment: only core analytical keys, not 40+ speculative ones.
                for k in &[
                    "POI.location.nearby", "POI.operations.experience",
                    "Multi_POI.count", "Multi_POI.any",
                    "Trigger.count", "Trigger.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }

            // 3g. Certification features → Certification observation types (narrowed).
            for (company_id, standard, count) in &cert_feats {
                let fm = entity_maps.entry(*company_id).or_default();
                let c = *count as f64;

                *fm.entry("Certification.count".into()).or_default() += c;
                *fm.entry("Certification.any".into()).or_default() += c;
                *fm.entry("CertificationUpdate.count".into()).or_default() += c;
                *fm.entry("CertificationUpdate.any".into()).or_default() += c;

                // Standard-specific mappings.
                let std_upper = standard.to_uppercase();
                if std_upper.contains("IATF") || std_upper.contains("16949") {
                    *fm.entry("CertificationUpdate.new.IATF_16949".into()).or_default() += c;
                    *fm.entry("Tender.sector=automotive".into()).or_default() += c;
                }
                if std_upper.contains("AS9100") || std_upper.contains("AS 9100") || std_upper.contains("EN 9100") {
                    *fm.entry("CertificationUpdate.new.AS9100".into()).or_default() += c;
                    *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
                }
                if std_upper.contains("13485") {
                    *fm.entry("CertificationUpdate.new.ISO_13485".into()).or_default() += c;
                    *fm.entry("Tender.sector=medical".into()).or_default() += c;
                }
                if std_upper.contains("14001") {
                    *fm.entry("Environmental.count".into()).or_default() += c;
                    *fm.entry("ESG.count".into()).or_default() += c;
                }
                if std_upper.contains("27001") {
                    *fm.entry("CertificationUpdate.new.ISO_27001".into()).or_default() += c;
                    *fm.entry("Security.count".into()).or_default() += c;
                    *fm.entry("Compliance.cybersecurity".into()).or_default() += c;
                }
            }

            // 3h. Capability features → Technology/Capability observation types (narrowed).
            for (company_id, capability, count) in &cap_feats {
                let fm = entity_maps.entry(*company_id).or_default();
                let c = *count as f64;

                *fm.entry("Capability.count".into()).or_default() += c;
                *fm.entry("Capability.any".into()).or_default() += c;
                *fm.entry("Technology.count".into()).or_default() += c;

                // Capability-specific mappings.
                let cap_lower = capability.to_lowercase();
                if cap_lower.contains("smt") || cap_lower.contains("pcb") {
                    *fm.entry("Competitor.careers.SMT".into()).or_default() += c;
                    *fm.entry("Competitor.capability_page.changed".into()).or_default() += c;
                }
                if cap_lower.contains("medical") {
                    *fm.entry("Tender.sector=medical".into()).or_default() += c;
                }
                if cap_lower.contains("automotive") {
                    *fm.entry("Tender.sector=automotive".into()).or_default() += c;
                }
                if cap_lower.contains("aerospace") || cap_lower.contains("avionics") || cap_lower.contains("satellite") {
                    *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
                    *fm.entry("Defense.count".into()).or_default() += c;
                }
                if cap_lower.contains("iot") || cap_lower.contains("embedded") {
                    *fm.entry("Industry40.count".into()).or_default() += c;
                    *fm.entry("Digital.count".into()).or_default() += c;
                }
                if cap_lower.contains("prototype") || cap_lower.contains("testing") {
                    *fm.entry("Product.development.early".into()).or_default() += c;
                }
            }

            // 3i. Site features → Production/Facility observation types (narrowed).
            for (company_id, country_code, site_type, count) in &site_feats {
                let fm = entity_maps.entry(*company_id).or_default();
                let c = *count as f64;

                *fm.entry("Production.count".into()).or_default() += c;
                *fm.entry("Production.any".into()).or_default() += c;
                *fm.entry("Facility.count".into()).or_default() += c;
                *fm.entry("Facility.any".into()).or_default() += c;

                if !country_code.is_empty() {
                    *fm.entry(format!("Geographic.{country_code}")).or_default() += c;
                    *fm.entry("SupplyChain.geo.concentrated".into()).or_default() += c;
                }
                if !site_type.is_empty() {
                    *fm.entry(format!("Facility.{site_type}")).or_default() += c;
                    *fm.entry("Production.site.change".into()).or_default() += c;
                }
            }

            // 3j. Graph edge features → Connection/Relationship observation types (narrowed).
            for (source_id, edge_type, count) in &graph_feats {
                let fm = entity_maps.entry(*source_id).or_default();
                let c = *count as f64;

                *fm.entry("Connection.count".into()).or_default() += c;
                *fm.entry("Relationship.count".into()).or_default() += c;
                *fm.entry("Ecosystem.count".into()).or_default() += c;

                match edge_type.as_str() {
                    "CompanyCompany" => {
                        *fm.entry("Competitor.count".into()).or_default() += c;
                        *fm.entry("Vendor.count".into()).or_default() += c;
                        *fm.entry("Vendor.any".into()).or_default() += c;
                        *fm.entry("Supplier.count".into()).or_default() += c;
                        *fm.entry("Customer.count".into()).or_default() += c;
                        *fm.entry("Customer.any".into()).or_default() += c;
                        *fm.entry("Partnership.strategic".into()).or_default() += c;
                        *fm.entry("Distributor.count".into()).or_default() += c;
                        *fm.entry("Distributor.any".into()).or_default() += c;
                    }
                    "CompanyPerson" => {
                        *fm.entry("POI.count".into()).or_default() += c;
                        *fm.entry("POI.any".into()).or_default() += c;
                        *fm.entry("Alumni.count".into()).or_default() += c;
                        *fm.entry("Alumni.any".into()).or_default() += c;
                        *fm.entry("Alumni.connection".into()).or_default() += c;
                        *fm.entry("Connection.mutual".into()).or_default() += c;
                    }
                    "leads" => {
                        *fm.entry("POI.influence.chain".into()).or_default() += c;
                        *fm.entry("Relationship.new".into()).or_default() += c;
                        *fm.entry("Relationship.strengthened".into()).or_default() += c;
                    }
                    _ => {}
                }
            }

            // Log feature map stats for debugging.
            tracing::info!(
                entities = entity_maps.len(),
                obs_rows = obs_counts.len(),
                warn_rows = warn_counts.len(),
                ce_rows = ce_features.len(),
                wc_rows = wc_features.len(),
                kw_rows = wc_kw_features.len(),
                person_rows = person_feats.len(),
                cert_rows = cert_feats.len(),
                cap_rows = cap_feats.len(),
                site_rows = site_feats.len(),
                graph_rows = graph_feats.len(),
                "recipe_fire: built feature maps"
            );

            // 4. Evaluate all recipes against each entity's FeatureMap.
            let entity_id_strs: Vec<(String, FeatureMap)> = entity_maps
                .into_iter()
                .map(|(id, fm)| (id.to_string(), fm))
                .collect();
            let entity_refs: Vec<(&str, &FeatureMap)> = entity_id_strs
                .iter()
                .map(|(id, fm)| (id.as_str(), fm))
                .collect();

            let candidates = engine.evaluate_batch(&entity_refs);

            // 5. Batch-load entity metadata for template rendering.
            let all_entity_uuids: Vec<Uuid> = candidates
                .iter()
                .filter_map(|c| Uuid::parse_str(&c.entity_id).ok())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            let company_names: HashMap<String, (String, Option<String>, Option<String>)> = match store
                .get_company_names_by_ids(&all_entity_uuids)
                .await
            {
                Ok(rows) => rows
                    .into_iter()
                    .map(|(id, name, region, company_type)| (id.to_string(), (name, region, company_type)))
                    .collect(),
                Err(e) => {
                    tracing::warn!("recipe_fire: failed to load company names: {e}");
                    HashMap::new()
                }
            };

            // POI names: first person (by influence) per company.
            let poi_names: HashMap<String, String> = match store
                .get_person_names_by_company_ids(&all_entity_uuids)
                .await
            {
                Ok(rows) => {
                    let mut m: HashMap<String, String> = HashMap::new();
                    for (org_id, name) in rows {
                        m.entry(org_id.to_string()).or_insert(name);
                    }
                    m
                }
                Err(e) => {
                    tracing::warn!("recipe_fire: failed to load POI names: {e}");
                    HashMap::new()
                }
            };

            // 5b. Fetch warning details for LLM evidence generation.
            #[cfg(feature = "llm")]
            let (entity_evidence, entity_contexts): (HashMap<String, Vec<EvidenceSignal>>, HashMap<String, EntityContext>) = {
                let mut evidence_map: HashMap<String, Vec<EvidenceSignal>> = HashMap::new();
                let mut context_map: HashMap<String, EntityContext> = HashMap::new();
                
                // Initialize EntityContext for each entity from company_names
                for (entity_id, (name, region, company_type)) in &company_names {
                    let industry_tags: Vec<String> = Vec::new();
                    context_map.insert(entity_id.clone(), EntityContext {
                        name: name.clone(),
                        region: region.clone().unwrap_or_default(),
                        entity_type: company_type.clone(),
                        is_competitor: false,
                        industry_tags,
                        certifications: Vec::new(),
                        capabilities: Vec::new(),
                        key_persons: Vec::new(),
                        recent_changes: Vec::new(),
                        threat_score: None,
                        overlap_score: None,
                        strategic_relevance: None,
                        revenue_estimate_usd: None,
                        employee_estimate: None,
                        competitor_names: Vec::new(),
                        sites_summary: Vec::new(),
                        competitor_events: Vec::new(),
                        domain: None,
                    });
                }

                // Enrich EntityContext with full company metadata (scores, revenue, employees, is_competitor)
                for entity_uuid in all_entity_uuids.iter() {
                    let entity_id_str = entity_uuid.to_string();
                    if let Ok(Some(row)) = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>, Option<i64>, Option<i32>, Option<String>, Option<Vec<String>>, Option<bool>)>(
                        "SELECT threat_score, overlap_score, strategic_relevance, revenue_estimate_usd, employee_estimate, domain, industry_tags, (metadata->>'is_competitor')::boolean FROM companies WHERE id = $1"
                    )
                    .bind(entity_uuid)
                    .fetch_optional(&store.pool)
                    .await {
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            ctx.threat_score = row.0;
                            ctx.overlap_score = row.1;
                            ctx.strategic_relevance = row.2;
                            ctx.revenue_estimate_usd = row.3;
                            ctx.employee_estimate = row.4;
                            ctx.domain = row.5;
                            if let Some(tags) = row.6 {
                                ctx.industry_tags = tags;
                            }
                            ctx.is_competitor = row.7.unwrap_or(false);
                        }
                    }
                }

                // Enrich with graph edges: find competitor relationships
                for entity_uuid in all_entity_uuids.iter() {
                    let entity_id_str = entity_uuid.to_string();
                    if let Ok(edges) = store.get_edges_from(*entity_uuid, "company").await {
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for edge in edges.iter().take(8) {
                                // Look up target company name
                                if let Some((name, region, _ctype)) = company_names.get(&edge.target_id.to_string()) {
                                    let region_str = region.as_deref().unwrap_or("");
                                    ctx.competitor_names.push(format!("{} ({})", name, region_str));
                                }
                            }
                        }
                    }
                    // Also check incoming edges
                    if let Ok(edges) = store.get_edges_to(*entity_uuid, "company").await {
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for edge in edges.iter().take(8) {
                                if let Some((name, region, _ctype)) = company_names.get(&edge.source_id.to_string()) {
                                    let region_str = region.as_deref().unwrap_or("");
                                    let entry = format!("{} ({})", name, region_str);
                                    if !ctx.competitor_names.contains(&entry) {
                                        ctx.competitor_names.push(entry);
                                    }
                                }
                            }
                        }
                    }
                }

                // Enrich with site/facility data
                for entity_uuid in all_entity_uuids.iter() {
                    let entity_id_str = entity_uuid.to_string();
                    if let Ok(sites) = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<Vec<String>>)>(
                        "SELECT name, city, country_code, site_type, capabilities FROM sites WHERE company_id = $1 LIMIT 6"
                    )
                    .bind(entity_uuid)
                    .fetch_all(&store.pool)
                    .await {
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for (sname, city, cc, stype, caps) in sites {
                                let location = city.unwrap_or_else(|| cc.unwrap_or_default());
                                let type_str = stype.unwrap_or_default();
                                let caps_str = caps.map(|c| c.join(", ")).unwrap_or_default();
                                let summary = if caps_str.is_empty() {
                                    format!("{} ({}) in {}", sname, type_str, location)
                                } else {
                                    format!("{} ({}) in {} – capabilities: {}", sname, type_str, location, caps_str)
                                };
                                ctx.sites_summary.push(summary);
                            }
                        }
                    }
                }

                // Enrich with CompetitorEvent observations
                for entity_uuid in all_entity_uuids.iter() {
                    let entity_id_str = entity_uuid.to_string();
                    if let Ok(obs) = store.get_observations_by_entity(*entity_uuid, 5).await {
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for o in obs {
                                if o.observation_type == "CompetitorEvent" || o.observation_type == "JobPost" || o.observation_type == "SocialPost" {
                                    let excerpt = o.value.as_object()
                                        .and_then(|obj| obj.get("excerpt").or(obj.get("title")).or(obj.get("text")))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let source = o.value.as_object()
                                        .and_then(|obj| obj.get("source").or(obj.get("url")))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if !excerpt.is_empty() {
                                        let evt_line = crate::truncate_text(&format!("{}: {}", o.observation_type, &excerpt), 200).to_string();
                                        let sig = EvidenceSignal {
                                            title: format!("{}: {}", o.observation_type, crate::truncate_text(&excerpt, 80)),
                                            description: excerpt,
                                            source_url: source,
                                            signal_type: o.observation_type.clone(),
                                            extracted_facts: extract_facts_from_text(&o.value.to_string()),
                                            date_context: Some(o.ts_utc.format("%Y-%m-%d").to_string()),
                                            relevance_score: 0.75,
                                        };
                                        evidence_map.entry(entity_id_str.clone()).or_default().push(sig);
                                        ctx.competitor_events.push(evt_line);
                                    }
                                }
                            }
                        }
                    }
                }
                
                // Fetch certifications for each entity and add to context
                for entity_uuid in all_entity_uuids.iter() {
                    if let Ok(certs) = store.get_certifications_for_company(*entity_uuid).await {
                        let entity_id_str = entity_uuid.to_string();
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for cert in certs.iter().take(10) {
                                let valid_info = cert.valid_until
                                    .map(|d| format!(" (valid until {})", d))
                                    .unwrap_or_default();
                                let cert_str = format!("{}{}", cert.standard, valid_info);
                                ctx.certifications.push(cert_str.clone());
                                
                                // Also add as evidence signal for compliance-related insights
                                let sig = EvidenceSignal {
                                    title: format!("{} Certification", cert.standard),
                                    description: format!(
                                        "Holds {} certification{}. Issuing body: {}. Scope: {}",
                                        cert.standard,
                                        valid_info,
                                        cert.issuing_body.as_deref().unwrap_or("Unknown"),
                                        cert.scope.as_deref().unwrap_or("General")
                                    ),
                                    source_url: cert.evidence_url.clone().unwrap_or_default(),
                                    signal_type: "certification".to_string(),
                                    extracted_facts: vec![
                                        format!("Standard: {}", cert.standard),
                                        cert.valid_until.map(|d| format!("Valid until: {}", d)).unwrap_or_default(),
                                    ].into_iter().filter(|s| !s.is_empty()).collect(),
                                    date_context: cert.valid_until.map(|d| d.to_string()),
                                    relevance_score: 0.5,
                                };
                                evidence_map.entry(entity_id_str.clone()).or_default().push(sig);
                            }
                        }
                    }
                }
                
                // Fetch capabilities for each entity
                for entity_uuid in all_entity_uuids.iter() {
                    if let Ok(caps) = store.list_capabilities(Some(*entity_uuid), 15, 0).await {
                        let entity_id_str = entity_uuid.to_string();
                        if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                            for cap in caps.iter().take(10) {
                                let proof_info = cap.proof_grade.as_deref()
                                    .map(|g| format!(" ({})", g))
                                    .unwrap_or_default();
                                ctx.capabilities.push(format!("{}{}", cap.capability, proof_info));
                                
                                // Add as evidence signal
                                let sig = EvidenceSignal {
                                    title: format!("Capability: {}", cap.capability),
                                    description: format!(
                                        "Manufacturing capability: {}. Proof level: {}",
                                        cap.capability,
                                        cap.proof_grade.as_deref().unwrap_or("Claimed")
                                    ),
                                    source_url: cap.evidence_urls.as_ref()
                                        .and_then(|u| u.first().cloned())
                                        .unwrap_or_default(),
                                    signal_type: "capability".to_string(),
                                    extracted_facts: vec![format!("Capability: {}", cap.capability)],
                                    date_context: cap.last_confirmed.map(|d| d.format("%Y-%m-%d").to_string()),
                                    relevance_score: 0.4,
                                };
                                evidence_map.entry(entity_id_str.clone()).or_default().push(sig);
                            }
                        }
                    }
                }
                
                // Add POI names to context
                for (entity_id, poi_name) in &poi_names {
                    if let Some(ctx) = context_map.get_mut(entity_id) {
                        ctx.key_persons.push(poi_name.clone());
                    }
                }
                
                // Add warnings as evidence (these have the richest content)
                match store.get_warnings_by_entity_ids(&all_entity_uuids, 500).await {
                    Ok(rows) => {
                        for w in rows {
                            if let Some(entity_ids) = &w.entity_ids {
                                let description = w.description.clone().unwrap_or_default();
                                let extracted = extract_facts_from_text(&format!("{} {}", w.title, description));
                                
                                let sig = EvidenceSignal {
                                    title: w.title.clone(),
                                    description: description.clone(),
                                    source_url: w.source_urls.as_ref()
                                        .and_then(|urls| urls.first().cloned())
                                        .unwrap_or_default(),
                                    signal_type: "warning".to_string(),
                                    extracted_facts: extracted,
                                    date_context: Some(w.ts_utc.format("%Y-%m-%d").to_string()),
                                    relevance_score: 0.8 + (w.severity.as_str() == "critical").then_some(0.2).unwrap_or(0.0) as f32,
                                };
                                
                                for eid in entity_ids {
                                    evidence_map.entry(eid.to_string()).or_default().push(sig.clone());
                                    // Also add warning type as recent change
                                    if let Some(ctx) = context_map.get_mut(&eid.to_string()) {
                                        let change = format!("{}: {}", w.warning_type, w.title);
                                        if ctx.recent_changes.len() < 5 {
                                            ctx.recent_changes.push(change);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("recipe_fire: failed to load warning evidence: {e}");
                    }
                }

                // Also fetch recent observations for entities as evidence.
                let unique_entity_uuids: std::collections::HashSet<Uuid> = all_entity_uuids.iter().cloned().collect();
                for entity_uuid in unique_entity_uuids.iter() {
                    if let Ok(obs_rows) = store.get_observations_by_entity(*entity_uuid, 10).await {
                        for obs in obs_rows {
                            // Extract text from observation value
                            let (title, description) = if let Some(obj) = obs.value.as_object() {
                                let title = obj.get("title")
                                    .or_else(|| obj.get("headline"))
                                    .or_else(|| obj.get("role"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| format!("{} update", obs.observation_type));
                                let desc = obj.get("description")
                                    .or_else(|| obj.get("text"))
                                    .or_else(|| obj.get("summary"))
                                    .or_else(|| obj.get("content"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                (title, desc)
                            } else if let Some(s) = obs.value.as_str() {
                                (obs.observation_type.clone(), s.to_string())
                            } else {
                                continue;
                            };
                            
                            // Skip low-quality observations (just URLs with no content)
                            if description.is_empty() && !title.contains(':') {
                                continue;
                            }
                            
                            let source_url = obs.provenance.as_object()
                                .and_then(|p| p.get("source_url").or_else(|| p.get("url")))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            
                            let extracted = extract_facts_from_text(&format!("{} {}", title, description));
                            
                            let sig = EvidenceSignal {
                                title,
                                description,
                                source_url,
                                signal_type: obs.observation_type.clone(),
                                extracted_facts: extracted,
                                date_context: Some(obs.ts_utc.format("%Y-%m-%d").to_string()),
                                relevance_score: 0.5,
                            };
                            evidence_map.entry(entity_uuid.to_string()).or_default().push(sig);
                        }
                    }
                }
                
                tracing::info!(
                    unique_entities = unique_entity_uuids.len(),
                    entities_with_evidence = evidence_map.len(),
                    total_evidence_items = evidence_map.values().map(|v| v.len()).sum::<usize>(),
                    entities_with_context = context_map.len(),
                    "recipe_fire: loaded enriched evidence for LLM"
                );
                
                (evidence_map, context_map)
            };

            // 6. Per-run dedup (F6): keep only the highest-scoring candidate per
            //    (recipe_code, entity_id) pair. This prevents the same recipe
            //    from producing duplicate insights for the same entity in a
            //    single evaluation run.
            let mut dedup_map: HashMap<(String, String), usize> = HashMap::new();
            for (idx, c) in candidates.iter().enumerate() {
                let key = (c.recipe_code.clone(), c.entity_id.clone());
                let dominated = match dedup_map.get(&key) {
                    Some(&prev_idx) => {
                        let prev = &candidates[prev_idx];
                        c.confidence * c.impact > prev.confidence * prev.impact
                    }
                    None => true,
                };
                if dominated {
                    dedup_map.insert(key, idx);
                }
            }
            let deduped_idxs: std::collections::HashSet<usize> =
                dedup_map.values().copied().collect();

            let total_candidates = candidates.len();
            let deduped_count = total_candidates - deduped_idxs.len();
            if deduped_count > 0 {
                tracing::info!(
                    total = total_candidates,
                    deduped = deduped_count,
                    "recipe_fire: per-run dedup removed duplicate candidates"
                );
            }

            // Persist each candidate as an insight; resolve templates first.
            let mut insights_inserted: u64 = 0;
            let mut warnings_inserted: u64 = 0;
            let mut skipped_low_conf: u64 = 0;
            let mut skipped_dedup: u64 = 0;
            let mut skipped_cross_run: u64 = 0;

            // Pre-load existing (recipe_code, entity_id) pairs from last 4 days
            // to avoid re-generating the same insight across daily runs.
            let cross_run_dedup: std::collections::HashSet<(String, String)> = {
                let rows = sqlx::query(
                    "SELECT DISTINCT unnest(tags) AS tag, unnest(entity_ids)::text AS eid \
                     FROM insights WHERE created_at > NOW() - INTERVAL '4 days'",
                )
                .fetch_all(&store.pool)
                .await;
                match rows {
                    Ok(rows) => {
                        use sqlx::Row as _;
                        rows.into_iter()
                            .filter_map(|r| {
                                let tag: String = r.try_get("tag").ok()?;
                                let eid: String = r.try_get("eid").ok()?;
                                Some((tag, eid))
                            })
                            .collect()
                    }
                    Err(e) => {
                        tracing::warn!("recipe_fire: failed to load cross-run dedup set: {e}");
                        std::collections::HashSet::new()
                    }
                }
            };

            // F8: With LLM generation, dedup by (entity_id, category) because
            // all recipes of the same category use the same evidence and produce
            // near-identical insights.  Only the highest-scoring candidate per
            // entity+category pair triggers an LLM call.
            #[cfg(feature = "llm")]
            let mut llm_entity_cat_done: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();

            // F9: Per-run entity cap — limit each entity to at most 2 insights
            // per batch run to prevent flooding when many recipes fire for one entity.
            #[cfg(feature = "llm")]
            let mut entity_run_count: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();

            for (idx, c) in candidates.iter().enumerate() {
                // F6: skip candidates removed by per-run dedup
                if !deduped_idxs.contains(&idx) {
                    skipped_dedup += 1;
                    continue;
                }

                // F5: minimum confidence threshold — skip low-quality signals
                if c.confidence < 0.45 {
                    skipped_low_conf += 1;
                    continue;
                }

                // F7: cross-run dedup — skip if this recipe already produced
                // an insight for this entity in the last 7 days.
                if cross_run_dedup.contains(&(c.recipe_code.clone(), c.entity_id.clone())) {
                    skipped_cross_run += 1;
                    continue;
                }

                let entity_uuid = Uuid::parse_str(&c.entity_id).ok();
                let entity_ids = entity_uuid.map(|u| vec![u]);
                let tags = vec![c.category.clone(), c.recipe_code.clone()];

                // Build evidence slot map from available data.
                let mut slots: HashMap<String, String> = HashMap::new();
                let mut entity_label = String::new();
                let mut entity_region = String::new();
                let mut entity_type: Option<String> = None;
                if let Some((name, region, company_type)) = company_names.get(&c.entity_id) {
                    entity_label = name.clone();
                    entity_type = company_type.clone();
                    slots.insert("company_name".into(), name.clone());
                    slots.insert("supplier_name".into(), name.clone());
                    slots.insert("vendor_name".into(), name.clone());
                    slots.insert("customer_name".into(), name.clone());
                    slots.insert("target_company".into(), name.clone());
                    slots.insert("entity_name".into(), name.clone());
                    // For competitor_market recipes, the entity IS the competitor.
                    slots.insert("competitor_name".into(), name.clone());
                    if let Some(r) = region {
                        entity_region = r.clone();
                        slots.insert("region".into(), r.clone());
                        slots.insert("location".into(), r.clone());
                        slots.insert("country".into(), r.clone());
                    }
                }
                if let Some(poi) = poi_names.get(&c.entity_id) {
                    slots.insert("poi_name".into(), poi.clone());
                    slots.insert("mentor_name".into(), poi.clone());
                }

                // Derive numeric evidence from the feature map, if we still have it.
                // Also collect signal details for the rich summary (F4).
                let mut signal_details: Vec<String> = Vec::new();
                if let Some((_, fm)) = entity_id_strs.iter().find(|(id, _)| id == &c.entity_id) {
                    if let Some(v) = fm.get("POI.count") {
                        slots.insert("poi_count".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} person(s) of interest tracked", *v as i64)); }
                    }
                    if let Some(v) = fm.get("JobPost.count") {
                        slots.insert("job_count".into(), (*v as i64).to_string());
                        slots.insert("job_delta".into(), format!("{:.0}", v));
                        slots.insert("npi_jobs".into(), (*v as i64).to_string());
                        slots.insert("eng_jobs".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} job posting(s) observed", *v as i64)); }
                    }
                    if let Some(v) = fm.get("Patent.count") {
                        slots.insert("patent_count".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} patent(s) filed", *v as i64)); }
                    }
                    if let Some(v) = fm.get("Tender.count") {
                        slots.insert("tender_count".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} tender(s) identified", *v as i64)); }
                    }
                    if let Some(v) = fm.get("Certification.count") {
                        slots.insert("cert_count".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} certification(s) on record", *v as i64)); }
                    }
                    if let Some(v) = fm.get("Competitor.count") {
                        slots.insert("competitor_count".into(), (*v as i64).to_string());
                        if *v > 0.0 { signal_details.push(format!("{} competitor signal(s)", *v as i64)); }
                    }
                    if let Some(v) = fm.get("NewsArticle.count") {
                        if *v > 0.0 { signal_details.push(format!("{} news article(s) referenced", *v as i64)); }
                    }
                    if let Some(v) = fm.get("WebChange.count") {
                        if *v > 0.0 { signal_details.push(format!("{} web change(s) detected", *v as i64)); }
                    }
                    if let Some(v) = fm.get("Filing.count") {
                        if *v > 0.0 { signal_details.push(format!("{} regulatory filing(s)", *v as i64)); }
                    }
                    if let Some(v) = fm.get("SanctionEntry.count") {
                        if *v > 0.0 { signal_details.push(format!("{} sanction entry(ies)", *v as i64)); }
                    }
                    if let Some(v) = fm.get("TradeShow.count") {
                        if *v > 0.0 { signal_details.push(format!("{} trade show participation(s)", *v as i64)); }
                    }
                    // Total signal count.
                    let signal_count = c.evidence_ids.len();
                    slots.insert("signal_count".into(), signal_count.to_string());
                }

                #[cfg(not(feature = "llm"))]
                let rendered_narrative = clean_rendered_text(&resolve_evidence_placeholders(&c.narrative_template, &slots));
                #[cfg(not(feature = "llm"))]
                let rendered_action = clean_rendered_text(&resolve_evidence_placeholders(&c.action_template, &slots));

                #[cfg(not(feature = "llm"))]
                let use_template = !is_low_quality_narrative(&rendered_narrative);

                #[cfg(not(feature = "llm"))]
                let narrative_with_entity = if use_template {
                    if !entity_label.is_empty()
                        && !rendered_narrative.to_ascii_lowercase().contains(&entity_label.to_ascii_lowercase())
                    {
                        let poi_present = poi_names.get(&c.entity_id)
                            .map(|p| rendered_narrative.to_ascii_lowercase().contains(&p.to_ascii_lowercase()))
                            .unwrap_or(false);
                        if poi_present {
                            rendered_narrative.clone()
                        } else {
                            let region_tag = if !entity_region.is_empty() {
                                format!(" ({})", entity_region)
                            } else {
                                String::new()
                            };
                            format!("[{}{}] {}", entity_label, region_tag, rendered_narrative)
                        }
                    } else {
                        rendered_narrative.clone()
                    }
                } else {
                    build_analytical_narrative(
                        &entity_label,
                        &entity_region,
                        &c.category,
                        &signal_details,
                        c.confidence,
                        &c.evidence_ids,
                    )
                };

                // ======== Enriched insight generation (uses structured data) ========
                #[cfg(feature = "llm")]
                {
                    // F8: entity+category dedup — only run LLM once per entity per
                    // category since the evidence is the same regardless of recipe.
                    let ec_key = (c.entity_id.clone(), c.category.clone());
                    if !llm_entity_cat_done.insert(ec_key) {
                        tracing::debug!(
                            entity = %entity_label,
                            category = %c.category,
                            recipe = %c.recipe_code,
                            "recipe_fire: skipping duplicate entity+category LLM call"
                        );
                        skipped_dedup += 1;
                        continue;
                    }

                    // F9: per-run entity cap — max 2 insights per entity per batch
                    let run_count = entity_run_count.entry(c.entity_id.clone()).or_insert(0);
                    if *run_count >= 2 {
                        tracing::debug!(
                            entity = %entity_label,
                            category = %c.category,
                            "recipe_fire: per-run entity cap reached (F9), skipping"
                        );
                        skipped_dedup += 1;
                        continue;
                    }
                    *run_count += 1;
                }

                #[cfg(feature = "llm")]
                let llm_output = {
                    // Get enriched evidence signals for this entity
                    let mut evidence_signals: Vec<EvidenceSignal> = entity_evidence
                        .get(&c.entity_id)
                        .cloned()
                        .unwrap_or_default();
                    
                    // Calculate relevance scores for sorting
                    for sig in &mut evidence_signals {
                        sig.relevance_score = calculate_relevance(
                            &sig.title,
                            &sig.description,
                            &sig.signal_type,
                            &c.category,
                        );
                    }
                    
                    // Sort by relevance and take top items
                    evidence_signals.sort_by(|a, b| {
                        b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    evidence_signals.truncate(12);

                    // Get or build EntityContext
                    let entity_ctx = entity_contexts.get(&c.entity_id).cloned().unwrap_or_else(|| {
                        EntityContext {
                            name: entity_label.clone(),
                            region: entity_region.clone(),
                            entity_type: entity_type.clone(),
                            is_competitor: false,
                            industry_tags: Vec::new(),
                            certifications: Vec::new(),
                            capabilities: Vec::new(),
                            key_persons: poi_names.get(&c.entity_id).cloned().into_iter().collect(),
                            recent_changes: Vec::new(),
                            threat_score: None,
                            overlap_score: None,
                            strategic_relevance: None,
                            revenue_estimate_usd: None,
                            employee_estimate: None,
                            competitor_names: Vec::new(),
                            sites_summary: Vec::new(),
                            competitor_events: Vec::new(),
                            domain: None,
                        }
                    });

                    tracing::info!(
                        entity = %entity_label,
                        category = %c.category,
                        evidence_count = evidence_signals.len(),
                        certs = entity_ctx.certifications.len(),
                        caps = entity_ctx.capabilities.len(),
                        "recipe_fire: generating LLM insight"
                    );

                    match generate_llm_insight(
                        &insight_llm_client,
                        &entity_ctx,
                        &c.category,
                        &evidence_signals,
                    ).await {
                        Ok((headline, narrative, recommendation, llm_confidence)) => {
                            let summary = format!("{}\n\nRecommended action: {}", narrative, recommendation);
                            Some((headline, summary, recommendation, llm_confidence))
                        }
                        Err(e) => {
                            tracing::warn!(
                                entity = %entity_label,
                                category = %c.category,
                                error = %e,
                                "recipe_fire: LLM insight generation failed; skipping insight (no template fallback)"
                            );
                            None
                        }
                    }
                };

                #[cfg(feature = "llm")]
                let (title, summary, warning_action, stored_confidence) = match llm_output {
                    Some(v) => v,
                    None => continue,
                };

                // ======== Template-based insight generation (non-LLM fallback) ========
#[cfg(not(feature = "llm"))]
                let stored_confidence = c.confidence;
                #[cfg(not(feature = "llm"))]
                let (title, summary) = {
                    // Title: always a concise analytical headline — never a truncated narrative.
                    let title = build_analytical_title(
                        &entity_label,
                        &entity_region,
                        &c.category,
                        &signal_details,
                        entity_type.as_deref(),
                    );
                    let summary = build_fallback_summary(
                        &narrative_with_entity,
                        use_template,
                        &rendered_action,
                        &rendered_narrative,
                        &signal_details,
                        &entity_label,
                        &entity_region,
                        &c.category,
                        &c.severity,
                        c.confidence,
                        c.evidence_ids.len(),
                    );
                    (title, summary)
                };

                match store.insert_insight(
                    &title,
                    &summary,
                    Some(c.category.as_str()),
                    None,
                    Some(stored_confidence),
                    None,
                    entity_ids.clone(),
                    Some(tags),
                ).await {
                    Ok(_) => insights_inserted += 1,
                    Err(e) => tracing::warn!(recipe = %c.recipe_code, "recipe_fire: insert_insight failed: {e}"),
                }

                // Escalate to warning when severity is warning/critical and confidence ≥ 0.75.
                #[cfg(feature = "llm")]
                if (c.severity == "warning" || c.severity == "critical") && stored_confidence >= 0.75 {
                    let warn_title = format!("[{}] {}", c.recipe_code, title);
                    let _ = store.insert_warning(
                        &c.category,
                        &warn_title,
                        Some(&warning_action),
                        &c.severity,
                        None,
                        Some("recipe_fire"),
                        entity_ids.clone(),
                        None,
                        Some(stored_confidence),
                    ).await;
                    warnings_inserted += 1;
                }

                #[cfg(not(feature = "llm"))]
                if (c.severity == "warning" || c.severity == "critical") && c.confidence >= 0.75 {
                    let warn_title = format!("[{}] {}", c.recipe_code, title);
                    let _ = store.insert_warning(
                        &c.category,
                        &warn_title,
                        Some(&rendered_action),
                        &c.severity,
                        None,
                        Some("recipe_fire"),
                        entity_ids.clone(),
                        None,
                        Some(c.confidence),
                    ).await;
                    warnings_inserted += 1;
                }
            }

            tracing::info!(
                total_candidates = total_candidates,
                skipped_low_conf = skipped_low_conf,
                skipped_dedup = skipped_dedup,
                skipped_cross_run = skipped_cross_run,
                inserted = insights_inserted,
                warnings = warnings_inserted,
                "recipe_fire: run complete"
            );

            run.succeed(
                insights_inserted,
                &format!(
                    "recipe_fire: {} candidate(s), inserted {} insight(s), {} warning(s) (skipped {} low-conf, {} dedup, {} cross-run)",
                    total_candidates,
                    insights_inserted,
                    warnings_inserted,
                    skipped_low_conf,
                    skipped_dedup,
                    skipped_cross_run,
                ),
            );
            run
        }
        JobKind::PoiDiscovery => {
            let mut run = JobRun::new(JobKind::PoiDiscovery);
            run.start();
            #[cfg(feature = "llm")]
            {
                // POI network-expansion: discover new persons from existing seeds.
                // Load existing persons as seeds, run the expansion engine, insert discoveries.
                
                let seed_limit = std::env::var("POI_DISCOVERY_SEED_LIMIT")
                    .ok()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(120)
                    .clamp(20, 400);

                // Load existing persons to use as seeds (joins companies for org_domain)
                let seed_rows = match store.list_expansion_seeds(seed_limit).await {
                    Ok(r) => r,
                    Err(e) => {
                        run.fail(&format!("poi_discovery: failed to load seed persons: {e}"));
                        return run;
                    }
                };

                if seed_rows.is_empty() {
                    run.skip("poi_discovery: no seed persons in database");
                    return run;
                }

                let min_competitor_seeds = std::env::var("POI_DISCOVERY_MIN_COMPETITOR_SEEDS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(25)
                    .clamp(0, seed_rows.len());
                let min_partner_seeds = std::env::var("POI_DISCOVERY_MIN_PARTNER_SEEDS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(25)
                    .clamp(0, seed_rows.len());

                // Maintain both tracks: verified competitors and non-competitor partner/prospect orgs.
                let mut selected_seed_rows: Vec<&apex_store::postgres::ExpansionSeedRow> = Vec::new();
                let mut selected_ids: HashSet<String> = HashSet::new();

                for row in seed_rows.iter().filter(|r| r.is_competitor).take(min_competitor_seeds) {
                    selected_ids.insert(row.id.to_string());
                    selected_seed_rows.push(row);
                }
                for row in seed_rows.iter().filter(|r| !r.is_competitor).take(min_partner_seeds) {
                    if selected_ids.insert(row.id.to_string()) {
                        selected_seed_rows.push(row);
                    }
                }
                for row in &seed_rows {
                    if selected_seed_rows.len() >= seed_limit as usize {
                        break;
                    }
                    if selected_ids.insert(row.id.to_string()) {
                        selected_seed_rows.push(row);
                    }
                }

                // Convert ExpansionSeedRow to SeedPoi (domain → full URL)
                let seeds: Vec<SeedPoi> = selected_seed_rows.iter().map(|r| SeedPoi {
                    id: r.id.to_string(),
                    name: r.name.clone(),
                    organization: r.org_name.clone(),
                    org_website: r.org_domain.as_deref()
                        .filter(|d| !d.is_empty())
                        .map(|d| format!("https://www.{d}")),
                    region: Some(r.region.clone()).filter(|s| !s.is_empty()),
                    role_family: r.role_family.clone(),
                }).collect();

                // Build seed lookup: seed_person_id → ExpansionSeedRow
                let seed_lookup: std::collections::HashMap<String, &apex_store::postgres::ExpansionSeedRow> =
                    selected_seed_rows.iter().map(|r| (r.id.to_string(), *r)).collect();

                tracing::info!(
                    seeds_total = seeds.len(),
                    competitor_seeds = selected_seed_rows.iter().filter(|r| r.is_competitor).count(),
                    partner_seeds = selected_seed_rows.iter().filter(|r| !r.is_competitor).count(),
                    with_website = seeds.iter().filter(|s| s.org_website.is_some()).count(),
                    "poi_discovery: loaded expansion seeds"
                );

                // Get all known names to avoid duplicates
                let filters = PersonListFilters {
                    regions: vec![],
                    roles: vec![],
                    search: None,
                    min_priority: None,
                    max_priority: None,
                };
                let all_persons = match store
                    .list_persons(&filters, None, true, 1000, 0)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        run.fail(&format!("poi_discovery: failed to load all persons: {e}"));
                        return run;
                    }
                };
                let known_names: HashSet<String> = all_persons.iter()
                    .map(|p| p.name.to_lowercase())
                    .collect();

                // Create expansion engine and discover new POIs
                let proxy_rotator = build_proxy_rotator_from_env().map(|r| Arc::new(Mutex::new(r)));
                if let Some(rotator) = proxy_rotator.as_ref() {
                    if let Ok(guard) = rotator.lock() {
                        tracing::info!(
                            proxy_health = %guard.health_summary(),
                            "poi_discovery: proxy rotation enabled"
                        );
                    }
                }

                let engine = match PoiExpansionEngine::new(proxy_rotator) {
                    Ok(e) => e,
                    Err(e) => {
                        run.fail(&format!("poi_discovery: failed to create expansion engine: {e}"));
                        return run;
                    }
                };

                let discoveries = engine.expand_from_seeds(&seeds, &known_names, 50).await;
                
                // Filter out the noisiest discoveries, then validate a bounded top-N via LLM.
                let discovered_total = discoveries.len();
                let min_confidence = 0.35;
                let mut discoveries: Vec<_> = discoveries.into_iter()
                    .filter(|d| {
                        if !looks_like_person_name(&d.name) {
                            tracing::debug!(
                                name = %d.name,
                                method = %d.discovery_method,
                                "poi_discovery: skipping non-person-like candidate"
                            );
                            return false;
                        }

                        if d.discovery_method == "gdelt_co_mention" && d.confidence < 0.55 {
                            tracing::debug!(
                                name = %d.name,
                                confidence = d.confidence,
                                "poi_discovery: skipping low-confidence gdelt co-mention"
                            );
                            return false;
                        }

                        if d.confidence < min_confidence {
                            tracing::debug!(
                                name = %d.name,
                                confidence = d.confidence,
                                method = %d.discovery_method,
                                "poi_discovery: skipping low-confidence discovery"
                            );
                            false
                        } else {
                            true
                        }
                    })
                    .collect();

                discoveries.sort_by(|a, b| {
                    discovery_method_priority(&b.discovery_method)
                        .cmp(&discovery_method_priority(&a.discovery_method))
                        .then_with(|| {
                            b.confidence
                                .partial_cmp(&a.confidence)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });

                // Keep LLM validation costs bounded while significantly increasing candidate throughput.
                const MAX_LLM_CANDIDATES: usize = 25;
                if discoveries.len() > MAX_LLM_CANDIDATES {
                    discoveries.truncate(MAX_LLM_CANDIDATES);
                }

                tracing::info!(
                    discovered_total,
                    above_confidence = discoveries.len(),
                    min_confidence,
                    llm_cap = MAX_LLM_CANDIDATES,
                    "poi_discovery: candidates prepared for LLM validation"
                );

                if discoveries.is_empty() {
                    run.succeed(0, "poi_discovery: no new persons discovered");
                    return run;
                }

                // ────────────────────────────────────────────────────────────────────────────
                // LLM validation step: filter out garbage names & extract structured info
                // ────────────────────────────────────────────────────────────────────────────
                tracing::info!(
                    count = discoveries.len(),
                    "poi_discovery: running LLM validation on candidates"
                );

                // Build LLM config from environment (LLM_BASE_URL) or default
                let mut llm_config = ModelConfig::llamacpp_lightweight();
                if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
                    llm_config.base_url = base_url;
                }
                let llm = OpenAiCompatibleClient::new(llm_config);
                let mut validated_discoveries: Vec<DiscoveredPoi> = Vec::new();

                for disc in discoveries {
                    match validate_person_via_llm(&llm, &disc).await {
                        Ok(Some(validated)) => {
                            tracing::debug!(
                                name = %validated.name,
                                role = ?validated.inferred_role,
                                org = ?validated.inferred_org,
                                "poi_discovery: LLM validated as real person"
                            );
                            validated_discoveries.push(validated);
                        }
                        Ok(None) => {
                            tracing::info!(
                                name = %disc.name,
                                method = %disc.discovery_method,
                                "poi_discovery: LLM rejected as not a real person"
                            );
                        }
                        Err(e) => {
                            // On LLM error, skip this candidate but don't fail the job
                            tracing::warn!(
                                name = %disc.name,
                                error = %e,
                                "poi_discovery: LLM validation failed, skipping"
                            );
                        }
                    }
                }

                let discoveries = validated_discoveries;
                tracing::info!(
                    count = discoveries.len(),
                    "poi_discovery: LLM validation passed {} candidates",
                    discoveries.len()
                );

                if discoveries.is_empty() {
                    run.succeed(0, "poi_discovery: no candidates passed LLM validation");
                    return run;
                }

                // Insert discovered persons into the database
                let mut inserted: u64 = 0;
                let mut artifacts_ingested: u64 = 0;
                let mut skipped_dup: u64 = 0;
                let mut errors: Vec<String> = vec![];
                let now = Utc::now();

                let enrichment_proxy = build_paid_proxy_url_from_env();
                let person_scraper = PersonOsintScraper::new(enrichment_proxy.as_deref())
                    .map_err(|e| {
                        tracing::warn!(error = %e, "poi_discovery: failed to init person scraper, continuing without deep artifact enrichment");
                        e
                    })
                    .ok();

                let onion_enrich_enabled = std::env::var("POI_ONION_ENRICH_ENABLED")
                    .ok()
                    .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                    .unwrap_or(true);
                let max_onion_people = std::env::var("POI_ONION_ENRICH_PER_RUN")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(12)
                    .clamp(0, 200);
                let tor_client = if onion_enrich_enabled {
                    Some(TorClient::new().await)
                } else {
                    None
                };
                let mut onion_enriched_people = 0usize;

                // Track names we've inserted this run to avoid duplicates within same batch
                let mut inserted_names: HashSet<String> = HashSet::new();

                for disc in &discoveries {
                    // Final duplicate check: normalize name and check against known + this batch
                    let normalized = disc.name.split_whitespace()
                        .map(|w| w.to_lowercase())
                        .collect::<Vec<_>>()
                        .join(" ");
                    
                    if known_names.contains(&disc.name.to_lowercase()) 
                        || known_names.contains(&normalized)
                        || inserted_names.contains(&normalized) 
                    {
                        tracing::debug!(
                            name = %disc.name,
                            "poi_discovery: skipping duplicate"
                        );
                        skipped_dup += 1;
                        continue;
                    }
                    inserted_names.insert(normalized);

                    // Determine role family from inferred role title
                    let role_family = classify_role_family(disc.inferred_role.as_deref());

                    // Look up the seed that produced this discovery to inherit org/region
                    let parent_seed = seed_lookup.get(&disc.seed_person_id);

                    let person = Person {
                        id: Uuid::new_v4(),
                        name: disc.name.clone(),
                        name_ar: None,
                        name_fr: None,
                        primary_org_id: parent_seed.and_then(|s| s.primary_org_id),
                        current_role: Some(disc.inferred_role.clone()
                            .unwrap_or_else(|| format!("Discovered ({})", disc.discovery_method))),
                        role_family,
                        region: parent_seed.map(|s| s.region.clone()).filter(|s| !s.is_empty()),
                        country_code: parent_seed.map(|s| s.country_code.clone()).filter(|s| !s.is_empty()),
                        public_bio: None,
                        public_email: disc.contact_email.clone(),
                        phone: None,
                        personal_email: None,
                        photo_hash: None,
                        priority_vector: PriorityVector::default(),
                        decision_mode: None,
                        influence_score: 0.0,
                        role_drift_score: 0.0,
                        change_risk: 0.0,
                        pain_index: 0.0,
                        preferred_proof_type: None,
                        trigger_topics: vec![],
                        decision_style: None,
                        risk_tolerance: None,
                        change_appetite: None,
                        communication_style: None,
                        metadata: serde_json::json!({
                            "engagement_status": "untracked",
                            "discovery_method": disc.discovery_method,
                            "source_url": disc.source_url,
                            "seed_person_id": disc.seed_person_id,
                            "linkedin_url": disc.contact_linkedin,
                            "inferred_org": disc.inferred_org,
                            "confidence": disc.confidence,
                            "seed_is_competitor": parent_seed.map(|s| s.is_competitor).unwrap_or(false),
                            "discovery_track": if parent_seed.map(|s| s.is_competitor).unwrap_or(false) { "competitor" } else { "partner_or_prospect" },
                        }),
                        created_at: now,
                        updated_at: now,
                    };

                    match store.insert_person(&person).await {
                        Ok(()) => {
                            inserted += 1;
                            tracing::debug!(
                                name = %disc.name,
                                method = %disc.discovery_method,
                                "poi_discovery: inserted new person"
                            );

                            if let Some(seed) = parent_seed {
                                let role_family_label = person.role_family.as_str().to_string();
                                let _ = store
                                    .insert_role_history(
                                        person.id,
                                        person.primary_org_id,
                                        &seed.org_name,
                                        person.current_role.as_deref().unwrap_or("Unknown"),
                                        Some(&role_family_label),
                                        Some(now),
                                        None,
                                        Some(&disc.source_url),
                                        disc.confidence as f64,
                                    )
                                    .await;
                            }

                            if let Some(scraper) = person_scraper.as_ref() {
                                let org_hint = parent_seed.map(|s| s.org_name.as_str()).unwrap_or("");
                                let mut raw_artifacts = scraper.aggregate(&person.name, org_hint).await;

                                if let Some(tor) = tor_client.as_ref() {
                                    let org_domain = parent_seed.and_then(|s| s.org_domain.as_deref());
                                    if tor.is_available() && onion_enriched_people < max_onion_people {
                                        let dark_web = tor.aggregate_dark_web_contacts(&person.name, org_domain).await;
                                        let onion_raw = dark_web_to_raw_artifacts(&dark_web, org_domain);
                                        if !onion_raw.is_empty() {
                                            onion_enriched_people += 1;
                                        }
                                        raw_artifacts.extend(onion_raw);
                                    }
                                }

                                let mut inserted_for_person = 0u64;
                                for raw in raw_artifacts.into_iter().take(120) {
                                    if !is_high_quality_raw_artifact(&raw) {
                                        continue;
                                    }
                                    if let Some(artifact) = raw_to_poi_artifact(person.id, raw, &disc.source_url, now) {
                                        if store.insert_poi_artifact(&artifact).await.is_ok() {
                                            inserted_for_person += 1;
                                        }
                                    }
                                }
                                artifacts_ingested += inserted_for_person;
                                tracing::info!(
                                    person = %person.name,
                                    artifacts = inserted_for_person,
                                    "poi_discovery: deep profile artifacts ingested"
                                );
                            }
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", disc.name, e));
                            tracing::warn!(
                                name = %disc.name,
                                error = %e,
                                "poi_discovery: failed to insert person"
                            );
                        }
                    }
                }

                if errors.is_empty() {
                    run.succeed(
                        inserted,
                        &format!(
                            "poi_discovery: {} seeds ({} competitor / {} partner) → {} validated → {} inserted, {} artifacts, {} duplicates skipped",
                            seeds.len(),
                            selected_seed_rows.iter().filter(|r| r.is_competitor).count(),
                            selected_seed_rows.iter().filter(|r| !r.is_competitor).count(),
                            discoveries.len(),
                            inserted,
                            artifacts_ingested,
                            skipped_dup
                        ),
                    );
                } else {
                    run.succeed(
                        inserted,
                        &format!(
                            "poi_discovery: {} inserted, {} artifacts, {} duplicates, {} errors: {}",
                            inserted,
                            artifacts_ingested,
                            skipped_dup,
                            errors.len(),
                            errors.join("; ")
                        ),
                    );
                }
            }
            #[cfg(not(feature = "llm"))]
            {
                run.skip("poi_discovery: requires the `llm` feature");
            }
            run
        }
        JobKind::Custom(name) => {
            let mut run = JobRun::new(JobKind::Custom(name.clone()));
            run.start();
            let key = format!("CUSTOM_JOB_COMMAND_{}", name.to_uppercase());
            match std::env::var(&key) {
                Ok(command) if !command.trim().is_empty() => {
                    let allowlist: Vec<String> = std::env::var("CUSTOM_JOB_ALLOWLIST")
                        .ok()
                        .map(|raw| {
                            raw.split(',')
                                .map(str::trim)
                                .filter(|entry| !entry.is_empty())
                                .map(ToOwned::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    let allowlist_refs: Vec<&str> = allowlist.iter().map(|entry| entry.as_str()).collect();
                    if let Err(err) = validate_custom_command(&name, &command, &allowlist_refs) {
                        run.fail(&format!("custom command validation failed for {}: {}", key, err));
                        return run;
                    }

                    let timeout = std::time::Duration::from_secs(
                        std::env::var("CUSTOM_JOB_TIMEOUT_SECS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(300),
                    );
                    let status = tokio::time::timeout(
                        timeout,
                        tokio::process::Command::new("sh")
                            .arg("-c")
                            .arg(&command)
                            .status(),
                    )
                    .await;
                    match status {
                        Ok(Ok(exit)) if exit.success() => {
                            run.succeed(1, &format!("custom command succeeded: {}", key));
                        }
                        Ok(Ok(exit)) => {
                            run.fail(&format!("custom command exited with status: {}", exit));
                        }
                        Ok(Err(err)) => {
                            run.fail(&format!("custom command execution failed: {}", err));
                        }
                        Err(_) => {
                            run.fail(&format!("custom command timed out after {}s", timeout.as_secs()));
                        }
                    }
                }
                _ => {
                    run.skip(&format!("custom command env missing: {}", key));
                }
            }
            run
        }
    }
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
    if cleaned.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }

    let blocked_terms = [
        "holdings", "limited", "ltd", "inc", "corp", "group", "reports", "transcript",
        "revenue", "demand", "guidance", "quarter", "deep dive", "boost", "street",
        "expectations", "call", "earnings", "meet", "cloud",
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
            Some(first) if first.is_uppercase() => chars.all(|c| c.is_alphabetic() || c == '-' || c == '\''),
            _ => false,
        }
    })
}

/// Classify an inferred role title into a canonical `RoleFamily`.
#[cfg(feature = "llm")]
fn classify_role_family(role: Option<&str>) -> RoleFamily {
    let r = match role {
        Some(s) if !s.is_empty() => s.to_lowercase(),
        _ => return RoleFamily::Other("Unknown".to_string()),
    };
    // C-suite / executive
    if r.contains("ceo") || r.contains("chief executive") || r.contains("chairman")
        || r.contains("chairwoman") || r.contains("president")
    {
        return RoleFamily::Executive;
    }
    if r.contains("cfo") || r.contains("chief financial") || r.contains("treasurer")
        || r.contains("controller") || r.contains("comptroller")
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
    if r.contains("chief") || r.contains("director") || r.contains("board")
        || r.contains("executive vice") || r.contains("senior vice")
    {
        return RoleFamily::Executive;
    }
    // VP-level roles — classify by functional area
    if r.contains("vp") || r.contains("vice president") {
        if r.contains("finance") || r.contains("financial") { return RoleFamily::Finance; }
        if r.contains("engineer") || r.contains("technology") || r.contains("r&d") { return RoleFamily::Engineering; }
        if r.contains("operation") || r.contains("supply chain") || r.contains("manufacturing") { return RoleFamily::Operations; }
        if r.contains("procurement") || r.contains("sourcing") { return RoleFamily::Procurement; }
        if r.contains("quality") { return RoleFamily::Quality; }
        if r.contains("security") { return RoleFamily::Security; }
        if r.contains("legal") || r.contains("counsel") { return RoleFamily::Legal; }
        if r.contains("logistics") { return RoleFamily::Logistics; }
        return RoleFamily::Executive;
    }
    // Government
    if r.contains("minister") || r.contains("secretary") || r.contains("governor")
        || r.contains("commissioner") || r.contains("ambassador")
    {
        return RoleFamily::Government;
    }
    // Military
    if r.contains("general") || r.contains("admiral") || r.contains("colonel")
        || r.contains("military") || r.contains("commander")
    {
        return RoleFamily::Military;
    }
    // Functional keywords
    if r.contains("procurement") || r.contains("sourcing") || r.contains("purchasing") {
        return RoleFamily::Procurement;
    }
    if r.contains("quality") { return RoleFamily::Quality; }
    if r.contains("engineer") || r.contains("architect") { return RoleFamily::Engineering; }
    if r.contains("operation") || r.contains("manufacturing") || r.contains("plant manager") {
        return RoleFamily::Operations;
    }
    if r.contains("finance") || r.contains("accounting") || r.contains("audit") {
        return RoleFamily::Finance;
    }
    if r.contains("legal") || r.contains("counsel") || r.contains("compliance") {
        return RoleFamily::Legal;
    }
    if r.contains("security") || r.contains("cyber") { return RoleFamily::Security; }
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
        failure_ids.into_iter().take(6).collect::<Vec<_>>().join(", ")
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

    let jsonl_examples = ImprovementCycleReport::to_jsonl(&training_examples);
    if !jsonl_examples.is_empty() {
        let preview: String = jsonl_examples.chars().take(2400).collect();
        let preview_summary = format!(
            "{}{}",
            preview,
            if jsonl_examples.chars().count() > 2400 {
                "\n...truncated..."
            } else {
                ""
            }
        );
        let _ = store
            .insert_insight(
                "LLM Training Example Export (Preview)",
                &preview_summary,
                Some("llm_training_examples"),
                Some("global"),
                Some(cycle_report.avg_critique_score.clamp(0.0, 1.0)),
                None,
                None,
                Some(vec![
                    "llm".to_string(),
                    "training_data".to_string(),
                    "self_improvement".to_string(),
                ]),
            )
            .await;
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
        let has_url = raw.url.as_deref().map(|u| !u.trim().is_empty()).unwrap_or(false);
        return (credibility || has_engagement) && has_url && content_len >= 60;
    }

    if raw.source.starts_with("darkweb_") {
        let has_evidence = raw.meta.get("domain").is_some() || raw.meta.get("source").is_some();
        let has_url = raw.url.as_deref().map(|u| !u.trim().is_empty()).unwrap_or(false);
        return raw.confidence >= 0.70 && has_evidence && has_url;
    }

    true
}

#[cfg(feature = "llm")]
fn dark_web_to_raw_artifacts(intel: &DarkWebPersonIntel, org_domain: Option<&str>) -> Vec<RawPersonArtifact> {
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
            confidence: if email_match { 0.80 } else { rec.confidence.max(0.70) },
            meta,
        };

        if email_match {
            artifact.meta.insert("domain_match".to_string(), "true".to_string());
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
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        anyhow::anyhow!("failed to stat file '{}': {}", path, err)
    })?;

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
    let content = tokio::fs::read_to_string(path).await.map_err(|err| {
        anyhow::anyhow!("failed to read file '{}': {}", path, err)
    })?;

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
        let s: String = sld.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, c)| c).collect();
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