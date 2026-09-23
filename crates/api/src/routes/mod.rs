//! Routes — endpoint request/response types and handler logic.
//!
//! Each sub-module defines the request/response structures and
//! pure validation/transformation logic for a group of endpoints.
//! Actual HTTP wiring (Axum extractors, Router) is done in the binary.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub mod admin;
pub mod battlecards;
pub mod collaboration;
pub mod companies;
pub mod dossiers;
pub mod export;
pub mod graph;
pub mod health;
pub mod insights;
pub mod llm;
pub mod persons;
pub mod preferences;
pub mod recipes;
pub mod replay;
pub mod search;
pub mod security;
pub mod semantic_search;
pub mod vector_search;
pub mod warnings;
pub mod ws;

pub const API_VERSION: &str = "v1";

// ─── Path constants ─────────────────────────────────────────────────────

pub mod paths {
    pub const API_PREFIX: &str = "/api";
    pub const API_V1_PREFIX: &str = "/api/v1";
    pub const HEALTH: &str = "/api/health";
    pub const HEALTH_LIVE: &str = "/api/health/live";
    pub const HEALTH_READY: &str = "/api/health/ready";
    pub const HEALTH_DEEP: &str = "/api/health/deep";
    pub const ENDPOINTS: &str = "/api/endpoints";
    pub const OPENAPI_JSON: &str = "/api/openapi.json";
    pub const DOCS: &str = "/api/docs";
    pub const FEATURES: &str = "/api/features";
    pub const WARNINGS: &str = "/api/warnings";
    pub const WARNING_DETAIL: &str = "/api/warnings/:id";
    pub const WARNING_ACKNOWLEDGE: &str = "/api/warnings/:id/acknowledge";
    pub const WARNING_ANALYZE: &str = "/api/warnings/:id/analyze";
    pub const WARNING_BULK_DELETE: &str = "/api/warnings/bulk-delete";
    pub const INSIGHTS: &str = "/api/insights";
    pub const INSIGHT_DETAIL: &str = "/api/insights/:id";
    pub const INSIGHT_ANALYZE: &str = "/api/insights/:id/analyze";
    pub const INSIGHT_BOOKMARK: &str = "/api/insights/:id/bookmark";
    pub const INSIGHT_FEEDBACK: &str = "/api/insights/:id/feedback";
    pub const INSIGHTS_EXPORT: &str = "/api/insights/export";
    pub const MEMOS: &str = "/api/memos";
    pub const WEEKLY_MEMO: &str = "/api/insights/weekly-memo";
    pub const COMPANIES: &str = "/api/companies";
    pub const COMPANIES_EXPORT: &str = "/api/companies/export";
    pub const COMPANY_DETAIL: &str = "/api/companies/:id";
    pub const SEARCH: &str = "/api/search";
    pub const SEMANTIC_SEARCH: &str = "/api/search/semantic";
    pub const PERSONS: &str = "/api/persons";
    pub const PERSONS_EXPORT: &str = "/api/persons/export";
    pub const PERSON_DETAIL: &str = "/api/persons/:id";
    pub const RECIPES: &str = "/api/recipes";
    pub const GRAPH: &str = "/api/graph";
    pub const SECURITY: &str = "/api/security";
    pub const ADMIN: &str = "/api/admin";
    pub const ADMIN_CRAWL_STATUS: &str = "/api/admin/crawl-status";
    pub const ADMIN_RECIPE_PERFORMANCE: &str = "/api/admin/recipe-performance";
    pub const ADMIN_POI_COVERAGE: &str = "/api/admin/poi-coverage";
    pub const ADMIN_CALIBRATION: &str = "/api/admin/calibration";
    pub const ADMIN_LLM_GOVERNANCE: &str = "/api/admin/llm-governance";
    pub const ADMIN_TRIGGER_SCAN: &str = "/api/admin/trigger-scan";
    pub const REPLAY: &str = "/api/admin/replay";
    pub const REPLAY_STATUS: &str = "/api/admin/replay/:job_id";
    pub const LLM_EXTRACT_ENTITIES: &str = "/api/llm/extract-entities";
    pub const LLM_GENERATE_RECIPE: &str = "/api/llm/generate-recipe";
    pub const LLM_SYNTHESIZE_POI: &str = "/api/llm/synthesize-poi";
    pub const LLM_GENERATE_MEMO: &str = "/api/llm/generate-memo";
    pub const EXPORT: &str = "/api/export";
    pub const PREFERENCES: &str = "/api/preferences";
    pub const USERS: &str = "/api/users";
    pub const USER_DETAIL: &str = "/api/users/:id";
    pub const SAVED_SEARCHES: &str = "/api/saved-searches";
    pub const SAVED_SEARCH_DETAIL: &str = "/api/saved-searches/:id";
    pub const WATCHLISTS: &str = "/api/watchlists";
    pub const WATCHLIST_DETAIL: &str = "/api/watchlists/:id";
    pub const ANNOTATIONS: &str = "/api/annotations";
    pub const ANNOTATION_DETAIL: &str = "/api/annotations/:id";
    pub const EXPORT_HISTORY: &str = "/api/export-history";

    // ─── Battlecards ────────────────────────────────────────────────────
    pub const BATTLECARDS: &str = "/api/battlecards";
    pub const BATTLECARD_DETAIL: &str = "/api/battlecards/:id";
    pub const BATTLECARD_REGENERATE: &str = "/api/battlecards/:id/regenerate";
    pub const BATTLECARD_EXPORT: &str = "/api/battlecards/:id/export";

    // ─── Alert Settings ─────────────────────────────────────────────────
    pub const VECTOR_SEARCH: &str = "/api/search/vector";
    pub const SIMILAR_ENTITIES: &str = "/api/entities/:entity_type/:entity_id/similar";
    pub const ADMIN_EMBEDDINGS_REINDEX: &str = "/api/admin/embeddings/reindex";
    pub const SETTINGS_ALERTS: &str = "/api/settings/alerts";
    pub const SETTINGS_ALERTS_ENTITY: &str = "/api/settings/alerts/entity/:entity_id";
    pub const SETTINGS_ALERTS_GLOBAL: &str = "/api/settings/alerts/global";

    // ─── Adversarial ──────────────────────────────────────────────────────
    pub const ADVERSARIAL_PLACEMENTS: &str = "/api/adversarial/placements";
    pub const ADVERSARIAL_QUARANTINE: &str = "/api/adversarial/quarantine";
    pub const ADVERSARIAL_SOURCE_RELIABILITY: &str = "/api/adversarial/source-reliability";

    // ─── Trends & Strategic Intelligence ───────────────────────────────────
    pub const TRENDS: &str = "/api/trends";
    pub const STRATEGIC_RADAR: &str = "/api/strategic-radar";
    pub const COMPETITIVE_LANDSCAPE: &str = "/api/competitive-landscape";
}

// ─── Endpoint catalogue ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl HttpMethod {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDef {
    pub method: HttpMethod,
    pub path: &'static str,
    pub description: &'static str,
    pub auth_required: bool,
    pub min_role: &'static str,
}

pub fn strip_version_prefix(path: &str) -> &str {
    if let Some(suffix) = path.strip_prefix(paths::API_V1_PREFIX) {
        if suffix.is_empty() {
            paths::API_PREFIX
        } else {
            suffix
        }
    } else {
        path
    }
}

pub fn versioned_path(path: &'static str) -> String {
    if let Some(suffix) = path.strip_prefix(paths::API_PREFIX) {
        format!("{}{}", paths::API_V1_PREFIX, suffix)
    } else {
        format!("{}{}", paths::API_V1_PREFIX, path)
    }
}

/// Returns the full list of endpoints that the API exposes.
/// Keep this in sync with the actual Axum router in `main.rs`.
pub fn all_endpoints() -> Vec<EndpointDef> {
    vec![
        // Health / meta
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::HEALTH,
            description: "Service health check",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::HEALTH_LIVE,
            description: "Liveness probe",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::HEALTH_READY,
            description: "Readiness probe",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ENDPOINTS,
            description: "List available API endpoints",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::OPENAPI_JSON,
            description: "OpenAPI 3.1 JSON document",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::DOCS,
            description: "Human-readable API documentation",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::FEATURES,
            description: "Public feature flags for client rollout",
            auth_required: false,
            min_role: "public",
        },
        // Warnings
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::WARNINGS,
            description: "List warnings with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::WARNING_ACKNOWLEDGE,
            description: "Acknowledge a warning",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Delete,
            path: paths::WARNING_DETAIL,
            description: "Delete a warning",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ADMIN_CALIBRATION,
            description: "Inspect live alert calibration curve",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::WARNING_BULK_DELETE,
            description: "Delete warnings in bulk",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::WARNING_ANALYZE,
            description: "Trigger AI analysis of a warning",
            auth_required: true,
            min_role: "analyst",
        },
        // Insights
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::INSIGHTS,
            description: "List insights with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::INSIGHTS_EXPORT,
            description: "Export insights as CSV",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::INSIGHT_DETAIL,
            description: "Get a single insight by ID",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::INSIGHT_ANALYZE,
            description: "Trigger AI analysis of an insight",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::INSIGHT_BOOKMARK,
            description: "Bookmark or unbookmark an insight",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::INSIGHT_FEEDBACK,
            description: "Record feedback for an insight",
            auth_required: true,
            min_role: "analyst",
        },
        // Companies
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::COMPANIES,
            description: "List companies with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::COMPANIES_EXPORT,
            description: "Export companies as CSV",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::COMPANY_DETAIL,
            description: "Get a company profile",
            auth_required: true,
            min_role: "viewer",
        },
        // Persons
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::PERSONS,
            description: "List persons of interest with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::PERSONS_EXPORT,
            description: "Export persons as CSV",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::PERSON_DETAIL,
            description: "Get a person-of-interest profile",
            auth_required: true,
            min_role: "viewer",
        },
        // Search
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::SEARCH,
            description: "Full-text search across all entities",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::SEMANTIC_SEARCH,
            description: "Ranked semantic-style search with boosting and facets",
            auth_required: true,
            min_role: "viewer",
        },
        // Graph
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::GRAPH,
            description: "Graph overview counts",
            auth_required: true,
            min_role: "viewer",
        },
        // Recipes
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::RECIPES,
            description: "List recipe signals and metadata",
            auth_required: true,
            min_role: "viewer",
        },
        // Security
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::SECURITY,
            description: "Security overview",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::HEALTH_DEEP,
            description: "Deep health checks for dependent services",
            auth_required: true,
            min_role: "admin",
        },
        // LLM
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::LLM_EXTRACT_ENTITIES,
            description: "Extract named entities from freeform text",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::LLM_GENERATE_RECIPE,
            description: "Generate a recipe hypothesis from a pattern description",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::LLM_SYNTHESIZE_POI,
            description: "Synthesize a POI dossier from fragments",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::LLM_GENERATE_MEMO,
            description: "Generate a strategic intelligence memo",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::PREFERENCES,
            description: "Read caller preferences",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::PREFERENCES,
            description: "Update caller preferences",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::USERS,
            description: "List analyst users",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::USERS,
            description: "Create or update an analyst user",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Put,
            path: paths::USER_DETAIL,
            description: "Update an analyst user",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::SAVED_SEARCHES,
            description: "List saved searches for the caller",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::SAVED_SEARCHES,
            description: "Create a saved search",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Put,
            path: paths::SAVED_SEARCH_DETAIL,
            description: "Update a saved search",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Delete,
            path: paths::SAVED_SEARCH_DETAIL,
            description: "Delete a saved search",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::WATCHLISTS,
            description: "List watchlists for the caller",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::WATCHLISTS,
            description: "Create a watchlist",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Put,
            path: paths::WATCHLIST_DETAIL,
            description: "Update a watchlist",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Delete,
            path: paths::WATCHLIST_DETAIL,
            description: "Delete a watchlist",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ANNOTATIONS,
            description: "List annotations with optional entity filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::ANNOTATIONS,
            description: "Create an annotation",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Put,
            path: paths::ANNOTATION_DETAIL,
            description: "Update an annotation",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Delete,
            path: paths::ANNOTATION_DETAIL,
            description: "Delete an annotation",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::EXPORT_HISTORY,
            description: "List export history for the caller",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ADMIN_CRAWL_STATUS,
            description: "Inspect crawl pipeline status",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ADMIN_RECIPE_PERFORMANCE,
            description: "Inspect recipe performance",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ADMIN_POI_COVERAGE,
            description: "Inspect POI coverage",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::ADMIN_TRIGGER_SCAN,
            description: "Queue an admin scan",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::REPLAY,
            description: "Replay historical observations through recipes",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::REPLAY_STATUS,
            description: "Fetch replay job status",
            auth_required: true,
            min_role: "admin",
        },
        // Entity list endpoints
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/sites",
            description: "List manufacturing/operational sites",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/capabilities",
            description: "List company capabilities",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/certifications",
            description: "List certifications",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/observations",
            description: "List intelligence observations",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/product-families",
            description: "List product families",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/logistics-nodes",
            description: "List logistics nodes",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/regulations",
            description: "List tracked regulations",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/poi-artifacts",
            description: "List person-of-interest artifacts",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/dashboard",
            description: "Dashboard aggregated statistics",
            auth_required: true,
            min_role: "viewer",
        },
        // Warning detail
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/warnings/:id",
            description: "Get a single warning by ID",
            auth_required: true,
            min_role: "viewer",
        },
        // Weekly memo
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/insights/weekly-memo",
            description: "Latest weekly intelligence memo",
            auth_required: true,
            min_role: "viewer",
        },
        // Company dossier
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/companies/:id/dossier",
            description: "Full company dossier with sites, capabilities, certifications",
            auth_required: true,
            min_role: "viewer",
        },
        // Person dossier & engagement
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/dossier",
            description: "Full POI dossier with artifacts and observations",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/engagement",
            description: "POI engagement guide with talking points",
            auth_required: true,
            min_role: "analyst",
        },
        // Psychological profiling (canonical psych tables)
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/psych",
            description: "Latest psychological profile for a person",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/behavioral-patterns",
            description: "Recent behavioral pattern events for a person",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/engagement-profile",
            description: "Latest psych engagement profile for a person",
            auth_required: true,
            min_role: "viewer",
        },
        // Competitors
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/competitors",
            description: "List competitor companies",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/competitors/changes",
            description: "All competitor changes across all competitors",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/competitors/:id/changes",
            description: "Recent changes for a competitor",
            auth_required: true,
            min_role: "viewer",
        },
        // Graph sub-endpoints
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/graph/neighborhood/:id",
            description: "Graph neighborhood around an entity",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/graph/path/:from/:to",
            description: "Shortest path between two entities",
            auth_required: true,
            min_role: "viewer",
        },
        // Recipes staging / promote
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/recipes/staging",
            description: "List staging (unpromoted) recipes",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/recipes/:id/promote",
            description: "Promote a staging recipe to active",
            auth_required: true,
            min_role: "analyst",
        },
        // Security sub-endpoints
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/dns-posture",
            description: "DNS security posture analysis",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/lookalike-domains",
            description: "Lookalike domain detection results",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/kev-relevance",
            description: "KEV relevance analysis",
            auth_required: true,
            min_role: "viewer",
        },
        // LLM Governance
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::ADMIN_LLM_GOVERNANCE,
            description: "LLM governance overview for prompts, runs, and datasets",
            auth_required: true,
            min_role: "admin",
        },
        // Vector search
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::VECTOR_SEARCH,
            description: "Vector/embedding-based similarity search across all entities",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: paths::SIMILAR_ENTITIES,
            description: "Find entities similar to a given entity by embedding",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: paths::ADMIN_EMBEDDINGS_REINDEX,
            description: "Trigger full or incremental embedding reindex (admin)",
            auth_required: true,
            min_role: "admin",
        },
        // WebSocket
        EndpointDef {
            method: HttpMethod::Get,
            path: "/ws/warnings",
            description: "Real-time warning stream via WebSocket",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/ws/calibration",
            description: "Live calibration curve stream via WebSocket",
            auth_required: false,
            min_role: "public",
        },
        // Battlecards
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/battlecards",
            description: "List battlecards with optional filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/battlecards",
            description: "Create a new battlecard",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/battlecards/:id",
            description: "Get a single battlecard by ID",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Put,
            path: "/api/battlecards/:id/section",
            description: "Update a battlecard section",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Delete,
            path: "/api/battlecards/:id",
            description: "Delete a battlecard",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/battlecards/:id/regenerate",
            description: "Regenerate a battlecard or section",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/battlecards/:id/export",
            description: "Export a battlecard (markdown/slack/pdf)",
            auth_required: true,
            min_role: "viewer",
        },
    ]
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub fn openapi_spec() -> Value {
    let mut paths = Map::new();
    for endpoint in all_endpoints() {
        let method = endpoint.method.label().to_ascii_lowercase();
        let versioned_method = method.clone();
        let mut operation = Map::new();
        operation.insert(
            "summary".to_string(),
            Value::String(endpoint.description.to_string()),
        );
        operation.insert(
            "operationId".to_string(),
            Value::String(
                endpoint
                    .description
                    .to_ascii_lowercase()
                    .replace([' ', '-'], "_"),
            ),
        );
        operation.insert(
            "tags".to_string(),
            json!([endpoint
                .path
                .trim_start_matches("/api/")
                .split('/')
                .next()
                .unwrap_or("meta")]),
        );
        if endpoint.auth_required {
            operation.insert(
                "security".to_string(),
                json!([{"bearerAuth": []}, {"apiKeyAuth": []}]),
            );
            operation.insert(
                "x-min-role".to_string(),
                Value::String(endpoint.min_role.to_string()),
            );
        }
        operation.insert(
            "responses".to_string(),
            json!({
                "200": {"description": "Successful response"},
                "400": {"description": "Bad request", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ApiErrorResponse"}}}},
                "401": {"description": "Unauthorized", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ApiErrorResponse"}}}},
                "403": {"description": "Forbidden", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ApiErrorResponse"}}}},
                "500": {"description": "Internal error", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ApiErrorResponse"}}}}
            }),
        );

        paths
            .entry(endpoint.path.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(obj) = paths.get_mut(endpoint.path).and_then(Value::as_object_mut) {
            obj.insert(method, Value::Object(operation.clone()));
        }

        let versioned = versioned_path(endpoint.path);
        paths
            .entry(versioned.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(obj) = paths.get_mut(&versioned).and_then(Value::as_object_mut) {
            obj.insert(versioned_method, Value::Object(operation));
        }
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "ApexIntel API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Operational intelligence API with versioned aliases at /api/v1"
        },
        "servers": [
            {"url": "/api", "description": "Current API surface"},
            {"url": "/api/v1", "description": "Stable v1 alias"}
        ],
        "components": {
            "securitySchemes": {
                "bearerAuth": {"type": "http", "scheme": "bearer"},
                "apiKeyAuth": {"type": "apiKey", "in": "header", "name": "Authorization"}
            },
            "schemas": {
                "ApiError": {
                    "type": "object",
                    "required": ["code", "message"],
                    "properties": {
                        "code": {"type": "string"},
                        "message": {"type": "string"},
                        "details": {"type": ["string", "null"]}
                    }
                },
                "ApiErrorResponse": {
                    "type": "object",
                    "required": ["success", "error"],
                    "properties": {
                        "success": {"type": "boolean", "enum": [false]},
                        "error": {"$ref": "#/components/schemas/ApiError"},
                        "meta": {
                            "type": ["object", "null"],
                            "properties": {
                                "request_id": {"type": ["string", "null"]},
                                "generated_at": {"type": ["string", "null"]},
                                "duration_ms": {"type": ["integer", "null"]}
                            }
                        }
                    }
                }
            }
        },
        "paths": Value::Object(paths)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_endpoints_count() {
        let eps = all_endpoints();
        assert_eq!(eps.len(), 99);
    }

    #[test]
    fn test_all_endpoints_have_api_prefix() {
        for ep in all_endpoints() {
            assert!(
                ep.path.starts_with("/api") || ep.path.starts_with("/ws"),
                "Missing prefix: {}",
                ep.path
            );
        }
    }

    #[test]
    fn test_health_no_auth() {
        let eps = all_endpoints();
        let health = eps.iter().find(|e| e.path == "/api/health").unwrap();
        assert!(!health.auth_required);
    }

    #[test]
    fn test_admin_endpoints_require_admin() {
        let eps = all_endpoints();
        for ep in &eps {
            if ep.path.starts_with("/api/admin") {
                assert_eq!(
                    ep.min_role, "admin",
                    "Admin endpoint {} should require admin role",
                    ep.path
                );
            }
        }
    }

    #[test]
    fn test_promote_is_post() {
        let eps = all_endpoints();
        let promote = eps.iter().find(|e| e.path.contains("promote")).unwrap();
        assert!(matches!(promote.method, HttpMethod::Post));
    }

    #[test]
    fn test_http_method_labels() {
        assert_eq!(HttpMethod::Get.label(), "GET");
        assert_eq!(HttpMethod::Post.label(), "POST");
        assert_eq!(HttpMethod::Put.label(), "PUT");
        assert_eq!(HttpMethod::Delete.label(), "DELETE");
    }

    #[test]
    fn test_paths_constants() {
        assert_eq!(paths::API_PREFIX, "/api");
        assert!(paths::WARNINGS.starts_with("/api"));
        assert!(paths::ADMIN.starts_with("/api"));
    }

    #[test]
    fn openapi_doc_includes_authenticated_warning_routes() {
        let spec = openapi_spec();
        let warning_get = &spec["paths"][paths::WARNINGS]["get"];

        assert_eq!(warning_get["summary"], "List warnings with filters");
        assert_eq!(warning_get["x-min-role"], "viewer");
        assert!(warning_get["security"].is_array());
    }

    #[test]
    fn openapi_doc_includes_error_schemas() {
        let spec = openapi_spec();
        let schemas = &spec["components"]["schemas"];

        assert!(schemas["ApiError"].is_object());
        assert!(schemas["ApiErrorResponse"].is_object());
        assert_eq!(
            spec["paths"][paths::WARNINGS]["get"]["responses"]["400"]["content"]
                ["application/json"]["schema"]["$ref"],
            "#/components/schemas/ApiErrorResponse"
        );
    }

    #[test]
    fn openapi_doc_build_fails_when_handler_schema_is_missing() {
        let spec = openapi_spec();
        let paths_obj = spec["paths"].as_object().expect("paths object");

        for endpoint in all_endpoints() {
            assert!(
                paths_obj.contains_key(endpoint.path),
                "missing current path {}",
                endpoint.path
            );
            assert!(
                paths_obj.contains_key(&versioned_path(endpoint.path)),
                "missing versioned path {}",
                endpoint.path
            );
        }
    }
}
