//! Routes — endpoint request/response types and handler logic.
//!
//! Each sub-module defines the request/response structures and
//! pure validation/transformation logic for a group of endpoints.
//! Actual HTTP wiring (Axum extractors, Router) is done in the binary.

use serde::{Deserialize, Serialize};

pub mod admin;
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
pub mod warnings;
pub mod ws;

// ─── Path constants ─────────────────────────────────────────────────────

pub mod paths {
    pub const API_PREFIX: &str = "/api";
    pub const HEALTH: &str = "/api/health";
    pub const ENDPOINTS: &str = "/api/endpoints";
    pub const WARNINGS: &str = "/api/warnings";
    pub const INSIGHTS: &str = "/api/insights";
    pub const COMPANIES: &str = "/api/companies";
    pub const COMPANY_DETAIL: &str = "/api/companies/:id";
    pub const SEARCH: &str = "/api/search";
    pub const PERSONS: &str = "/api/persons";
    pub const PERSON_DETAIL: &str = "/api/persons/:id";
    pub const RECIPES: &str = "/api/recipes";
    pub const GRAPH: &str = "/api/graph";
    pub const SECURITY: &str = "/api/security";
    pub const ADMIN: &str = "/api/admin";
    pub const HEALTH_DEEP: &str = "/api/health/deep";
    pub const LLM_EXTRACT_ENTITIES: &str = "/api/llm/extract-entities";
    pub const LLM_GENERATE_RECIPE: &str = "/api/llm/generate-recipe";
    pub const LLM_SYNTHESIZE_POI: &str = "/api/llm/synthesize-poi";
    pub const LLM_GENERATE_MEMO: &str = "/api/llm/generate-memo";
    pub const EXPORT: &str = "/api/export";
    pub const PREFERENCES: &str = "/api/preferences";
    pub const SEMANTIC_SEARCH: &str = "/api/semantic-search";
    pub const REPLAY: &str = "/api/replay";
    pub const REPLAY_STATUS: &str = "/api/replay/:id/status";
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

/// Returns the full list of endpoints that the API exposes.
/// Keep this in sync with the actual Axum router in `main.rs`.
pub fn all_endpoints() -> Vec<EndpointDef> {
    vec![
        // Health / meta
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/health",
            description: "Service health check",
            auth_required: false,
            min_role: "public",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/endpoints",
            description: "List available API endpoints",
            auth_required: false,
            min_role: "public",
        },
        // Warnings
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/warnings",
            description: "List warnings with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/warnings/:id/acknowledge",
            description: "Acknowledge a warning",
            auth_required: true,
            min_role: "analyst",
        },
        // Insights
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/insights",
            description: "List insights with filters",
            auth_required: true,
            min_role: "viewer",
        },
        // Companies
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/companies",
            description: "List companies with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/companies/:id",
            description: "Get a company profile",
            auth_required: true,
            min_role: "viewer",
        },
        // Persons
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons",
            description: "List persons of interest with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id",
            description: "Get a person-of-interest profile",
            auth_required: true,
            min_role: "viewer",
        },
        // Search
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/search",
            description: "Full-text search across all entities",
            auth_required: true,
            min_role: "viewer",
        },
        // Graph
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/graph",
            description: "Graph overview counts",
            auth_required: true,
            min_role: "viewer",
        },
        // Recipes
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/recipes",
            description: "List recipe signals and metadata",
            auth_required: true,
            min_role: "viewer",
        },
        // Security
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security",
            description: "Security overview",
            auth_required: true,
            min_role: "viewer",
        },
        // LLM
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/llm/extract-entities",
            description: "Extract named entities from freeform text",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/llm/generate-recipe",
            description: "Generate a recipe hypothesis from a pattern description",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/llm/synthesize-poi",
            description: "Synthesize a POI dossier from fragments",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/llm/generate-memo",
            description: "Generate a strategic intelligence memo",
            auth_required: true,
            min_role: "analyst",
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
        // Admin endpoints
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/crawl-status",
            description: "Crawl pipeline status overview",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/recipe-performance",
            description: "Recipe performance metrics",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/poi-coverage",
            description: "Person-of-interest data coverage stats",
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_endpoints_count() {
        let eps = all_endpoints();
        assert_eq!(eps.len(), 44);
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
                assert_eq!(ep.min_role, "admin", "Admin endpoint {} should require admin role", ep.path);
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
}
