//! Read-only, store-backed tools and the agentic hypothesis-generation path.
//!
//! The manual `JobKind::HypothesisGeneration` run drives the LLM through
//! [`apex_llm::function_calling::run_agent_loop`] instead of a single JSON
//! completion: the model may inspect store state through the tools registered
//! here (recipe code catalog, mining counters, observation event-stream
//! summaries, company rows/dossiers) before producing a recipe hypothesis.
//!
//! Design constraints:
//! * Every tool is **strictly read-only** — no tool writes to the store.
//! * Arguments are validated defensively (required, typed, range-limited,
//!   unknown keys rejected) on top of the registry's required-field check.
//! * Every result is bounded (cardinality caps plus a serialized-size guard)
//!   so a large table cannot fan a tool result out into an unbounded prompt.
//! * The staged recipe always goes through the same
//!   [`apex_learning::hypothesis::validate_hypothesis`] and
//!   [`namespace_recipe_code`] logic as the scheduled batch path; the agentic
//!   path only changes how the model is invoked, never what is accepted.
//!
//! This module intentionally resolves everything through absolute crate paths
//! (`apex_llm::`, `apex_learning::`, `apex_store::`) rather than `crate::` or
//! `super::`, so integration tests can include the exact production source via
//! `#[path]` without shimming the worker binary.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use apex_learning::generate::{generate_hypotheses_batch, HypothesisResult};
use apex_learning::hypothesis::{
    build_system_prompt, build_user_prompt, parse_hypothesis_response, validate_hypothesis,
    RecipeHypothesis,
};
use apex_learning::miner::PatternCandidate;
use apex_llm::function_calling::{
    run_agent_loop, FunctionSpec, ParamSchema, ParamType, Tool, ToolError, ToolRegistry,
};
use apex_llm::LlmClient;
use apex_store::postgres::{CompanyDossier, CompanyRow, PgStore};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

/// Maximum serialized bytes a single tool result may produce. This sits below
/// the agent loop's own 16 KiB feedback cap so the tool — not the loop — is the
/// component that refuses an oversized projection.
const MAX_TOOL_RESULT_BYTES: usize = 12_288;

/// Longest accepted company name argument.
const MAX_COMPANY_NAME_ARG_BYTES: usize = 256;
/// Longest accepted recipe code echoed back by `list_recipe_codes`.
const MAX_RECIPE_CODE_BYTES: usize = 160;
/// Cardinality cap for the recipe-code list.
const MAX_RECIPE_CODES: usize = 200;
/// Byte budget for the serialized recipe-code list.
const MAX_RECIPE_CODES_BYTES: usize = 8_192;
/// Longest accepted observation-type name echoed back by the stream summary.
const MAX_STREAM_NAME_BYTES: usize = 96;
/// Cardinality cap for observation-stream summaries.
const MAX_STREAM_SUMMARIES: usize = 50;
/// Longest metadata blob embedded verbatim in a company projection.
const MAX_METADATA_BYTES: usize = 2_048;
/// Longest analysis summary embedded in a dossier projection.
const MAX_ANALYSIS_SUMMARY_BYTES: usize = 2_048;
/// Number of rows included inline in dossier projections.
const MAX_DOSSIER_ITEMS: usize = 10;
/// Upper bound for the mining-stats lookback argument (one year).
const MAX_MINING_STATS_HOURS: i64 = 8_760;
/// Upper bound for the observation-stream lookback argument.
const MAX_STREAM_LOOKBACK_DAYS: i64 = 365;
/// Upper bound for the minimum-events argument.
const MAX_STREAM_MIN_EVENTS: i64 = 10_000;
/// Upper bound for the observation-row argument.
const MAX_STREAM_ROWS: i64 = 20_000;

/// Maximum agent-loop iterations per candidate.
pub(super) const AGENTIC_MAX_ITERATIONS: u32 = 6;

/// Hard cap on the number of candidates the manual agentic run submits to the
/// model. Each candidate costs up to [`AGENTIC_MAX_ITERATIONS`] model calls,
/// so the manual path processes a bounded head of the ranked candidate list;
/// the scheduled batch path is not affected.
pub(super) const MAX_AGENTIC_CANDIDATES: usize = 4;

// ─────────────────────────────────────────────────────────────────────────────
// Tool argument validation
// ─────────────────────────────────────────────────────────────────────────────

/// Small typed view over the model-supplied arguments object.
///
/// [`ToolRegistry::execute_call`] only checks that required keys are present;
/// these helpers add type, emptiness, length, and range checks and let each
/// tool reject keys it never declared.
struct ToolArgs<'a> {
    tool: &'static str,
    map: &'a serde_json::Map<String, Value>,
}

impl<'a> ToolArgs<'a> {
    fn parse(tool: &'static str, arguments: &'a Value) -> Result<Self, ToolError> {
        let Some(map) = arguments.as_object() else {
            return Err(ToolError::InvalidArguments {
                name: tool.to_string(),
                errors: vec!["arguments must be a JSON object".to_string()],
            });
        };
        Ok(Self { tool, map })
    }

    /// Reject any key not in `allowed` — a model echoing unrelated fields must
    /// not silently steer the query.
    fn deny_unknown(&self, allowed: &[&str]) -> Result<(), ToolError> {
        let unknown: Vec<String> = self
            .map
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .cloned()
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(self.invalid(format!("unexpected argument(s): {unknown:?}")))
        }
    }

    fn required_str(&self, key: &str, max_bytes: usize) -> Result<&'a str, ToolError> {
        match self.map.get(key) {
            Some(Value::String(value)) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    Err(self.invalid(format!("`{key}` must not be empty")))
                } else if trimmed.len() > max_bytes {
                    Err(self.invalid(format!("`{key}` exceeds {max_bytes} bytes")))
                } else {
                    Ok(trimmed)
                }
            }
            Some(_) => Err(self.invalid(format!("`{key}` must be a string"))),
            None => Err(self.invalid(format!("missing required argument `{key}`"))),
        }
    }

    fn required_i64(&self, key: &str, min: i64, max: i64) -> Result<i64, ToolError> {
        let value = match self.map.get(key) {
            Some(Value::Number(number)) => number
                .as_i64()
                .ok_or_else(|| self.invalid(format!("`{key}` must be an integer")))?,
            Some(_) => return Err(self.invalid(format!("`{key}` must be an integer"))),
            None => return Err(self.invalid(format!("missing required argument `{key}`"))),
        };
        if value < min || value > max {
            return Err(self.invalid(format!("`{key}` must be in {min}..={max}")));
        }
        Ok(value)
    }

    fn invalid(&self, message: String) -> ToolError {
        ToolError::InvalidArguments {
            name: self.tool.to_string(),
            errors: vec![message],
        }
    }
}

fn execution_failed(name: &'static str, error: impl std::fmt::Display) -> ToolError {
    ToolError::ExecutionFailed {
        name: name.to_string(),
        message: error.to_string(),
    }
}

/// Final serialized-size guard: refuses (rather than truncates) a projection
/// that still exceeds the budget, so the model never receives invalid JSON.
fn bounded_result(name: &'static str, value: Value) -> Result<Value, ToolError> {
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| execution_failed(name, format!("result serialization failed: {error}")))?;
    if bytes.len() > MAX_TOOL_RESULT_BYTES {
        return Err(execution_failed(
            name,
            format!(
                "result is {} bytes (max {MAX_TOOL_RESULT_BYTES}); narrow the query",
                bytes.len()
            ),
        ));
    }
    Ok(value)
}

// ─────────────────────────────────────────────────────────────────────────────
// Store-backed projection helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Compact company projection: every scalar column, with `metadata` embedded
/// only when it is small enough to keep the projection bounded.
fn company_json(row: &CompanyRow) -> Value {
    let mut value = json!({
        "id": row.id,
        "name": row.name,
        "legal_name": row.legal_name,
        "domain": row.domain,
        "country_code": row.country_code,
        "region": row.region,
        "company_type": row.company_type,
        "industry_tags": row.industry_tags,
        "employee_estimate": row.employee_estimate,
        "revenue_estimate_usd": row.revenue_estimate_usd,
        "risk_score": row.risk_score,
        "threat_score": row.threat_score,
        "overlap_score": row.overlap_score,
        "strategic_relevance": row.strategic_relevance,
        "is_competitor": row.is_competitor,
    });
    if let Some(metadata) = &row.metadata {
        let small_enough = serde_json::to_vec(metadata)
            .map(|bytes| bytes.len() <= MAX_METADATA_BYTES)
            .unwrap_or(false);
        if small_enough {
            value["metadata"] = metadata.clone();
        } else {
            value["metadata_omitted"] = Value::Bool(true);
        }
    }
    value
}

/// Compact dossier projection: counts for every collection plus bounded inline
/// samples, so even a heavily populated company cannot flood the prompt.
fn dossier_json(dossier: &CompanyDossier) -> Value {
    let sites: Vec<Value> = dossier
        .sites
        .iter()
        .take(MAX_DOSSIER_ITEMS)
        .map(|site| {
            json!({
                "name": site.name,
                "city": site.city,
                "country_code": site.country_code,
                "region": site.region,
                "site_type": site.site_type,
            })
        })
        .collect();
    let recent_changes: Vec<Value> = dossier
        .recent_changes
        .iter()
        .take(MAX_DOSSIER_ITEMS)
        .map(|change| {
            json!({
                "change_type": change.change_type,
                "field_name": change.field_name,
                "detected_at": change.detected_at,
            })
        })
        .collect();

    json!({
        "company": company_json(&dossier.company),
        "counts": {
            "sites": dossier.sites.len(),
            "capabilities": dossier.capabilities.len(),
            "certifications": dossier.certifications.len(),
            "product_families": dossier.product_families.len(),
            "edges": dossier.edges.len(),
            "dossier_entries": dossier.dossier_entries.len(),
            "recent_changes": dossier.recent_changes.len(),
        },
        "sites": sites,
        "recent_changes": recent_changes,
        "analysis": {
            "summary": apex_llm::truncate_utf8(
                &dossier.analysis.summary,
                MAX_ANALYSIS_SUMMARY_BYTES,
            ),
            "correlated_signal_count": dossier.analysis.correlated_signals.len(),
            "competing_hypothesis_count": dossier.analysis.competing_hypotheses.len(),
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tools
// ─────────────────────────────────────────────────────────────────────────────

fn string_param(description: &str) -> ParamSchema {
    ParamSchema {
        kind: ParamType::String,
        description: Some(description.to_string()),
        enum_values: None,
    }
}

fn integer_param(description: &str) -> ParamSchema {
    ParamSchema {
        kind: ParamType::Integer,
        description: Some(description.to_string()),
        enum_values: None,
    }
}

/// `list_recipe_codes` — existing recipe codes (used to avoid duplicate ids).
pub(super) struct ListRecipeCodesTool {
    store: Arc<PgStore>,
}

impl ListRecipeCodesTool {
    const NAME: &'static str = "list_recipe_codes";
}

#[async_trait]
impl Tool for ListRecipeCodesTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: Self::NAME.to_string(),
            description: "List recipe codes that already exist in the store. Use this to avoid \
                          proposing a hypothesis id that collides with an existing recipe."
                .to_string(),
            parameters: HashMap::new(),
            required: Vec::new(),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        let args = ToolArgs::parse(Self::NAME, arguments)?;
        args.deny_unknown(&[])?;
        let mut codes = self
            .store
            .list_recipe_codes()
            .await
            .map_err(|error| execution_failed(Self::NAME, error))?;
        let total = codes.len();
        codes.sort();
        let mut returned: Vec<Value> = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        for code in codes {
            let snippet = apex_llm::truncate_utf8(&code, MAX_RECIPE_CODE_BYTES);
            bytes += snippet.len() + 3;
            if returned.len() >= MAX_RECIPE_CODES || bytes > MAX_RECIPE_CODES_BYTES {
                truncated = true;
                break;
            }
            returned.push(Value::String(snippet.to_string()));
        }
        bounded_result(
            Self::NAME,
            json!({
                "total": total,
                "returned": returned.len(),
                "truncated": truncated,
                "codes": returned,
            }),
        )
    }
}

/// `get_mining_stats` — mined/validated/staged counters for a lookback window.
pub(super) struct GetMiningStatsTool {
    store: Arc<PgStore>,
}

impl GetMiningStatsTool {
    const NAME: &'static str = "get_mining_stats";
}

#[async_trait]
impl Tool for GetMiningStatsTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: Self::NAME.to_string(),
            description: "Read mining funnel counters (candidates found/passed gates, hypotheses \
                          generated, recipes staged) recorded over a lookback window."
                .to_string(),
            parameters: HashMap::from([(
                "since_hours".to_string(),
                integer_param("Lookback window in hours, 1..=8760 (e.g. 24 for the last day)."),
            )]),
            required: vec!["since_hours".to_string()],
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        let args = ToolArgs::parse(Self::NAME, arguments)?;
        args.deny_unknown(&["since_hours"])?;
        let since_hours = args.required_i64("since_hours", 1, MAX_MINING_STATS_HOURS)?;
        let since = Utc::now() - chrono::Duration::hours(since_hours);
        let stats = self
            .store
            .get_mining_stats(since)
            .await
            .map_err(|error| execution_failed(Self::NAME, error))?;
        bounded_result(
            Self::NAME,
            json!({
                "since_hours": since_hours,
                "candidates_found": stats.candidates_found,
                "candidates_passed_gates": stats.candidates_passed_gates,
                "hypotheses_generated": stats.hypotheses_generated,
                "recipes_staged": stats.recipes_staged,
            }),
        )
    }
}

/// `load_observation_event_streams` — bounded summary of entity-linked streams.
pub(super) struct LoadObservationEventStreamsTool {
    store: Arc<PgStore>,
}

impl LoadObservationEventStreamsTool {
    const NAME: &'static str = "load_observation_event_streams";
}

#[async_trait]
impl Tool for LoadObservationEventStreamsTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: Self::NAME.to_string(),
            description: "Load entity-linked observation event streams over a lookback window and \
                          return a per-observation-type summary (event count, first/last event \
                          timestamp). Individual events are never returned."
                .to_string(),
            parameters: HashMap::from([
                (
                    "lookback_days".to_string(),
                    integer_param("Lookback window in days, 1..=365."),
                ),
                (
                    "min_events".to_string(),
                    integer_param("Minimum events per stream to keep it, 1..=10000."),
                ),
                (
                    "max_rows".to_string(),
                    integer_param("Maximum observation rows loaded, 1..=20000."),
                ),
            ]),
            required: vec![
                "lookback_days".to_string(),
                "min_events".to_string(),
                "max_rows".to_string(),
            ],
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        let args = ToolArgs::parse(Self::NAME, arguments)?;
        args.deny_unknown(&["lookback_days", "min_events", "max_rows"])?;
        let lookback_days = args.required_i64("lookback_days", 1, MAX_STREAM_LOOKBACK_DAYS)?;
        let min_events = args.required_i64("min_events", 1, MAX_STREAM_MIN_EVENTS)?;
        let max_rows = args.required_i64("max_rows", 1, MAX_STREAM_ROWS)?;

        let since = Utc::now() - chrono::Duration::days(lookback_days);
        let streams = self
            .store
            .load_observation_event_streams(since, min_events as usize, max_rows)
            .await
            .map_err(|error| execution_failed(Self::NAME, error))?;

        let mut summaries: Vec<Value> = streams
            .iter()
            .map(|(observation_type, events)| {
                let first = events.iter().map(|(_, ts)| *ts).min();
                let last = events.iter().map(|(_, ts)| *ts).max();
                json!({
                    "observation_type": apex_llm::truncate_utf8(
                        observation_type,
                        MAX_STREAM_NAME_BYTES,
                    ),
                    "events": events.len(),
                    "first_event_unix": first,
                    "last_event_unix": last,
                })
            })
            .collect();
        summaries.sort_by(|a, b| {
            a["observation_type"]
                .as_str()
                .cmp(&b["observation_type"].as_str())
        });

        let total_streams = summaries.len();
        summaries.truncate(MAX_STREAM_SUMMARIES);
        bounded_result(
            Self::NAME,
            json!({
                "lookback_days": lookback_days,
                "min_events": min_events,
                "max_rows": max_rows,
                "total_streams": total_streams,
                "streams": summaries,
                "truncated": total_streams > MAX_STREAM_SUMMARIES,
                "note": "Streams are summarized (counts and timestamps), not returned in full.",
            }),
        )
    }
}

/// `get_company_by_name_ci` — case-insensitive company lookup by name.
pub(super) struct GetCompanyByNameTool {
    store: Arc<PgStore>,
}

impl GetCompanyByNameTool {
    const NAME: &'static str = "get_company_by_name_ci";
}

#[async_trait]
impl Tool for GetCompanyByNameTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: Self::NAME.to_string(),
            description: "Look up a company by exact name (case-insensitive, also matches the \
                          legal name) and return its profile row."
                .to_string(),
            parameters: HashMap::from([(
                "name".to_string(),
                string_param("Company name or legal name (max 256 bytes)."),
            )]),
            required: vec!["name".to_string()],
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        let args = ToolArgs::parse(Self::NAME, arguments)?;
        args.deny_unknown(&["name"])?;
        let name = args.required_str("name", MAX_COMPANY_NAME_ARG_BYTES)?;
        let row = self
            .store
            .get_company_by_name_ci(name)
            .await
            .map_err(|error| execution_failed(Self::NAME, error))?;
        match row {
            Some(row) => bounded_result(
                Self::NAME,
                json!({ "found": true, "company": company_json(&row) }),
            ),
            None => bounded_result(Self::NAME, json!({ "found": false, "name": name })),
        }
    }
}

/// `get_company_dossier` — bounded analytical dossier for a company UUID.
pub(super) struct GetCompanyDossierTool {
    store: Arc<PgStore>,
}

impl GetCompanyDossierTool {
    const NAME: &'static str = "get_company_dossier";
}

#[async_trait]
impl Tool for GetCompanyDossierTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: Self::NAME.to_string(),
            description: "Load a company dossier by UUID: profile, collection counts, bounded \
                          site/change samples and the analyst summary."
                .to_string(),
            parameters: HashMap::from([(
                "company_id".to_string(),
                string_param("Company UUID (canonical hyphenated form)."),
            )]),
            required: vec!["company_id".to_string()],
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        let args = ToolArgs::parse(Self::NAME, arguments)?;
        args.deny_unknown(&["company_id"])?;
        let raw_id = args.required_str("company_id", 64)?;
        let company_id = Uuid::parse_str(raw_id)
            .map_err(|error| args.invalid(format!("`company_id` must be a UUID: {error}")))?;
        let dossier = self
            .store
            .get_company_dossier(company_id)
            .await
            .map_err(|error| execution_failed(Self::NAME, error))?;
        match dossier {
            Some(dossier) => bounded_result(Self::NAME, dossier_json(&dossier)),
            None => bounded_result(
                Self::NAME,
                json!({ "found": false, "company_id": company_id }),
            ),
        }
    }
}

/// Build the production registry of read-only, store-backed tools.
pub(super) fn build_store_tool_registry(store: Arc<PgStore>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ListRecipeCodesTool {
        store: store.clone(),
    }));
    registry.register(Arc::new(GetMiningStatsTool {
        store: store.clone(),
    }));
    registry.register(Arc::new(LoadObservationEventStreamsTool {
        store: store.clone(),
    }));
    registry.register(Arc::new(GetCompanyByNameTool {
        store: store.clone(),
    }));
    registry.register(Arc::new(GetCompanyDossierTool { store }));
    registry
}

// ─────────────────────────────────────────────────────────────────────────────
// Agentic hypothesis generation
// ─────────────────────────────────────────────────────────────────────────────

/// Which generator the hypothesis stage uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HypothesisGenerationMode {
    /// Scheduled pattern-mining path: one JSON completion per candidate via
    /// [`apex_learning::generate::generate_hypotheses_batch`] (cost profile
    /// unchanged by this module).
    Batch,
    /// Manual hypothesis-generation path: a bounded multi-turn agent loop with
    /// the read-only store tools registered above.
    Agentic,
}

/// Dispatch to the batch or agentic generator. Kept as one entry point so the
/// scheduled and manual paths share the mode decision (and tests can prove the
/// scheduled mode never touches the multi-turn path).
pub(super) async fn generate_hypotheses_for_mode(
    mode: HypothesisGenerationMode,
    client: &dyn LlmClient,
    registry: &ToolRegistry,
    candidates: &[PatternCandidate],
    existing_ids: &[String],
) -> Vec<HypothesisResult> {
    match mode {
        HypothesisGenerationMode::Batch => {
            generate_hypotheses_batch(client, candidates, existing_ids).await
        }
        HypothesisGenerationMode::Agentic => {
            generate_hypotheses_agentic(client, registry, candidates, existing_ids).await
        }
    }
}

/// Task prompt handed to the agent loop. `run_agent_loop` appends the live tool
/// catalog to the system prompt, so the tool instructions themselves live
/// there; this adds the "final answer is the recipe JSON" contract.
fn build_agent_task(candidate: &PatternCandidate, existing_ids: &[String]) -> String {
    format!(
        "{}\n\nYou may call the available read-only tools to ground your reasoning. \
         When you are ready, reply with ONLY the final Recipe JSON object — no prose, \
         no markdown fences, no commentary.",
        build_user_prompt(candidate, existing_ids)
    )
}

/// Run the bounded agent loop for each candidate and map every outcome onto
/// the batch path's [`HypothesisResult`] vocabulary (success / validation
/// failure / LLM error). Callers must pass at most [`MAX_AGENTIC_CANDIDATES`].
pub(super) async fn generate_hypotheses_agentic(
    client: &dyn LlmClient,
    registry: &ToolRegistry,
    candidates: &[PatternCandidate],
    existing_ids: &[String],
) -> Vec<HypothesisResult> {
    debug_assert!(
        candidates.len() <= MAX_AGENTIC_CANDIDATES,
        "agentic generation must be called with at most {MAX_AGENTIC_CANDIDATES} candidates"
    );

    let system_prompt = build_system_prompt();
    let mut existing_ids: Vec<String> = existing_ids.to_vec();
    let mut results = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let task = build_agent_task(candidate, &existing_ids);
        let outcome_label = candidate.outcome.clone();

        let result = match run_agent_loop(
            client,
            registry,
            &system_prompt,
            &task,
            AGENTIC_MAX_ITERATIONS,
        )
        .await
        {
            Ok(agent) => match parse_hypothesis_response(&agent.final_answer) {
                Ok(hypothesis) => {
                    let issues = validate_hypothesis(&hypothesis, candidate, &existing_ids);
                    if issues.is_empty() {
                        tracing::info!(
                            id = %hypothesis.id,
                            iterations = agent.iterations,
                            tool_calls = agent.tool_trace.len(),
                            "hypothesis_generation: agentic hypothesis accepted"
                        );
                        existing_ids.push(hypothesis.id.clone());
                        HypothesisResult::Success(Box::new(hypothesis))
                    } else {
                        tracing::warn!(
                            outcome = %outcome_label,
                            issues = ?issues,
                            "hypothesis_generation: agentic validation failed"
                        );
                        HypothesisResult::ValidationFailed {
                            candidate_outcome: outcome_label,
                            issues,
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        outcome = %outcome_label,
                        error = %error,
                        "hypothesis_generation: agentic final answer parse failed"
                    );
                    HypothesisResult::ValidationFailed {
                        candidate_outcome: outcome_label,
                        issues: vec![format!("Parse error: {error}")],
                    }
                }
            },
            Err(error) => {
                tracing::warn!(
                    outcome = %outcome_label,
                    error = %error,
                    "hypothesis_generation: agent loop failed"
                );
                HypothesisResult::LlmError {
                    candidate_outcome: outcome_label,
                    error: error.to_string(),
                }
            }
        };

        results.push(result);
    }

    results
}

// ─────────────────────────────────────────────────────────────────────────────
// Staging (shared verbatim by the batch and agentic paths)
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregate outcome of the hypothesis generation + staging stage.
pub(super) struct HypothesisStageOutcome {
    /// Number of candidates actually handed to the generator (after any cap).
    pub(super) submitted: u64,
    /// Number of candidates for which the LLM produced a validated hypothesis.
    pub(super) generated: u64,
    /// Number of validated hypotheses successfully persisted as staging recipes.
    pub(super) staged: u64,
    /// Number of candidates that failed validation or hit an LLM error.
    pub(super) failed: u64,
    /// Hard errors (LLM/infra/persistence) surfaced to the stage reporter.
    pub(super) errors: Vec<String>,
}

/// Persist every validated hypothesis as a `staging` recipe, using the same
/// validation vocabulary and code namespacing as the scheduled batch path.
///
/// Staging recipes are a review/metadata store — recipe firing is driven by the
/// seed YAML, not the DB — so staging here never auto-injects into the live
/// insight stream. Validation failures are *not* treated as hard errors (the
/// model ran, the pattern simply didn't pass), whereas LLM/infra errors are
/// surfaced so the stage reporter can flag a systemic problem.
pub(super) async fn stage_hypothesis_results(
    store: &PgStore,
    candidates: &[PatternCandidate],
    results: &[HypothesisResult],
    existing_codes: &[String],
) -> HypothesisStageOutcome {
    let mut outcome = HypothesisStageOutcome {
        submitted: candidates.len() as u64,
        generated: 0,
        staged: 0,
        failed: 0,
        errors: Vec::new(),
    };
    if results.len() != candidates.len() {
        outcome.errors.push(format!(
            "generator produced {} result(s) for {} candidate(s)",
            results.len(),
            candidates.len()
        ));
    }

    let mut existing_set: HashSet<String> = existing_codes.iter().cloned().collect();

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
                    Err(error) => {
                        outcome.errors.push(format!("stage '{code}': {error}"));
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
                    "hypothesis_generation: hypothesis validation failed"
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
                    "hypothesis_generation: hypothesis LLM error"
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
pub(super) fn namespace_recipe_code(raw_id: &str, existing: &HashSet<String>) -> String {
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
pub(super) fn mined_recipe_name(hyp: &RecipeHypothesis) -> String {
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
pub(super) fn hypothesis_to_definition(
    code: &str,
    hyp: &RecipeHypothesis,
    candidate: &PatternCandidate,
) -> Value {
    let transforms: Vec<Value> = hyp
        .transforms
        .iter()
        .map(|t| match t.days {
            Some(days) => json!({ "type": t.kind, "days": days }),
            None => json!({ "type": t.kind }),
        })
        .collect();

    let (a, b, c, d) = candidate.contingency;

    json!({
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests (no database access)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A store over a lazy pool: construction succeeds without a live server
    /// and every argument-validation test returns before a query is issued.
    fn disconnected_store() -> Arc<PgStore> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://apex:apex@127.0.0.1:1/apexintel_no_db")
            .expect("lazy pool construction must not connect");
        Arc::new(PgStore::from_pool(pool))
    }

    fn registry() -> ToolRegistry {
        build_store_tool_registry(disconnected_store())
    }

    #[tokio::test]
    async fn registry_advertises_exactly_the_five_read_only_store_tools() {
        let registry = registry();
        let names: Vec<String> = registry
            .tool_specs_json()
            .iter()
            .map(|spec| {
                spec["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "get_company_by_name_ci",
                "get_company_dossier",
                "get_mining_stats",
                "list_recipe_codes",
                "load_observation_event_streams",
            ],
            "the tool catalog must be stable and name-sorted"
        );
    }

    #[tokio::test]
    async fn every_tool_spec_is_schema_valid_with_described_parameters() {
        let registry = registry();
        for name in [
            "list_recipe_codes",
            "get_mining_stats",
            "load_observation_event_streams",
            "get_company_by_name_ci",
            "get_company_dossier",
        ] {
            let tool = registry.get(name).expect("registered tool");
            let spec = tool.spec();
            assert_eq!(spec.name, name);
            assert!(
                spec.validate().is_empty(),
                "{name} spec must be schema-valid: {:?}",
                spec.validate()
            );
            assert!(!spec.description.is_empty(), "{name} needs a description");
            for (param, schema) in &spec.parameters {
                assert!(
                    schema.description.as_ref().is_some_and(|d| !d.is_empty()),
                    "{name}.{param} needs a parameter description"
                );
            }
        }
    }

    async fn expect_invalid_arguments(name: &str, arguments: Value) {
        let registry = registry();
        let tool = registry.get(name).expect("registered tool");
        let error = tool
            .execute(&arguments)
            .await
            .expect_err("invalid arguments must be rejected before any query");
        assert!(
            matches!(error, ToolError::InvalidArguments { .. }),
            "{name} with {arguments} must yield InvalidArguments, got {error:?}"
        );
    }

    #[tokio::test]
    async fn list_recipe_codes_rejects_unknown_arguments() {
        expect_invalid_arguments("list_recipe_codes", json!({ "limit": 5 })).await;
        expect_invalid_arguments("list_recipe_codes", json!("not-an-object")).await;
    }

    #[tokio::test]
    async fn get_mining_stats_validates_type_and_range() {
        expect_invalid_arguments("get_mining_stats", json!({})).await;
        expect_invalid_arguments("get_mining_stats", json!({ "since_hours": "24" })).await;
        expect_invalid_arguments("get_mining_stats", json!({ "since_hours": 0 })).await;
        expect_invalid_arguments("get_mining_stats", json!({ "since_hours": 8_761 })).await;
        expect_invalid_arguments(
            "get_mining_stats",
            json!({ "since_hours": 24, "extra": true }),
        )
        .await;
    }

    #[tokio::test]
    async fn load_observation_streams_validates_every_bound() {
        let name = "load_observation_event_streams";
        expect_invalid_arguments(name, json!({ "lookback_days": 30, "min_events": 8 })).await;
        expect_invalid_arguments(
            name,
            json!({ "lookback_days": 0, "min_events": 8, "max_rows": 100 }),
        )
        .await;
        expect_invalid_arguments(
            name,
            json!({ "lookback_days": 366, "min_events": 8, "max_rows": 100 }),
        )
        .await;
        expect_invalid_arguments(
            name,
            json!({ "lookback_days": 30, "min_events": 10_001, "max_rows": 100 }),
        )
        .await;
        expect_invalid_arguments(
            name,
            json!({ "lookback_days": 30, "min_events": 8, "max_rows": 20_001 }),
        )
        .await;
        expect_invalid_arguments(
            name,
            json!({ "lookback_days": 30, "min_events": 8, "max_rows": 100, "offset": 1 }),
        )
        .await;
    }

    #[tokio::test]
    async fn company_name_tool_validates_empty_and_oversized_names() {
        expect_invalid_arguments("get_company_by_name_ci", json!({})).await;
        expect_invalid_arguments("get_company_by_name_ci", json!({ "name": "   " })).await;
        expect_invalid_arguments(
            "get_company_by_name_ci",
            json!({ "name": "x".repeat(MAX_COMPANY_NAME_ARG_BYTES + 1) }),
        )
        .await;
        expect_invalid_arguments(
            "get_company_by_name_ci",
            json!({ "name": "Acme", "with_dossier": true }),
        )
        .await;
    }

    #[tokio::test]
    async fn dossier_tool_requires_a_canonical_uuid() {
        expect_invalid_arguments("get_company_dossier", json!({})).await;
        expect_invalid_arguments("get_company_dossier", json!({ "company_id": "not-a-uuid" }))
            .await;
        expect_invalid_arguments(
            "get_company_dossier",
            json!({ "company_id": Uuid::nil().to_string(), "full": true }),
        )
        .await;
    }
}
