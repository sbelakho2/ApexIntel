//! Routes — endpoint request/response types and handler logic.
//!
//! Each sub-module defines the request/response structures and
//! pure validation/transformation logic for a group of endpoints.
//! Actual HTTP wiring (Axum extractors, Router) is done in the binary.

use serde::{Deserialize, Serialize};

pub mod admin;
pub mod companies;
pub mod dossiers;
pub mod graph;
pub mod insights;
pub mod persons;
pub mod recipes;
pub mod search;
pub mod security;
pub mod warnings;
pub mod ws;

/// Shared route path constants.
pub mod paths {
    pub const API_PREFIX: &str = "/api";

    pub const WARNINGS: &str = "/api/warnings";
    pub const INSIGHTS: &str = "/api/insights";
    pub const COMPANIES: &str = "/api/companies";
    pub const PERSONS: &str = "/api/persons";
    pub const COMPETITORS: &str = "/api/competitors";
    pub const RECIPES: &str = "/api/recipes";
    pub const GRAPH: &str = "/api/graph";
    pub const SEARCH: &str = "/api/search";
    pub const SECURITY: &str = "/api/security";
    pub const ADMIN: &str = "/api/admin";
    pub const HEALTH: &str = "/api/health";
    pub const WS_WARNINGS: &str = "/ws/warnings";
}

/// All endpoint definitions for documentation/discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDef {
    pub method: HttpMethod,
    pub path: &'static str,
    pub description: &'static str,
    pub auth_required: bool,
    pub min_role: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Registry of all API endpoints (for OpenAPI generation / admin UI).
pub fn all_endpoints() -> Vec<EndpointDef> {
    vec![
        // Warnings
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/warnings",
            description: "List warnings with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/warnings/:id",
            description: "Get warning detail",
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
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/insights/weekly-memo",
            description: "Get latest weekly strategy memo",
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
            description: "Get company detail",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/companies/:id/dossier",
            description: "Get company dossier",
            auth_required: true,
            min_role: "viewer",
        },
        // Persons
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons",
            description: "List persons/POIs with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id",
            description: "Get POI detail",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/dossier",
            description: "Get POI dossier",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/persons/:id/engagement",
            description: "Get POI engagement guide",
            auth_required: true,
            min_role: "analyst",
        },
        // Competitors
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/competitors",
            description: "List competitors",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/competitors/:id/changes",
            description: "Get competitor change events",
            auth_required: true,
            min_role: "viewer",
        },
        // Recipes
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/recipes",
            description: "List recipes with filters",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/recipes/staging",
            description: "List staging recipes",
            auth_required: true,
            min_role: "analyst",
        },
        EndpointDef {
            method: HttpMethod::Post,
            path: "/api/recipes/:id/promote",
            description: "Promote a recipe to production",
            auth_required: true,
            min_role: "admin",
        },
        // Graph
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/graph/neighborhood/:id",
            description: "Get entity neighborhood in graph",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/graph/path/:from/:to",
            description: "Find shortest path between entities",
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
        // Security
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/dns-posture",
            description: "Get DNS posture checks",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/lookalike-domains",
            description: "Get lookalike domain detections",
            auth_required: true,
            min_role: "viewer",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/security/kev-relevance",
            description: "Get KEV relevance analysis",
            auth_required: true,
            min_role: "viewer",
        },
        // Admin
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/crawl-status",
            description: "Get crawl system status",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/recipe-performance",
            description: "Get recipe performance summary",
            auth_required: true,
            min_role: "admin",
        },
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/admin/poi-coverage",
            description: "Get POI coverage summary",
            auth_required: true,
            min_role: "admin",
        },
        // Health
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/health",
            description: "Health check",
            auth_required: false,
            min_role: "none",
        },
        // WebSocket
        EndpointDef {
            method: HttpMethod::Get,
            path: "/ws/warnings",
            description: "WebSocket warning stream",
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
        assert!(eps.len() >= 20);
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
        for ep in eps.iter().filter(|e| e.path.starts_with("/api/admin")) {
            assert_eq!(ep.min_role, "admin", "Non-admin role on: {}", ep.path);
        }
    }

    #[test]
    fn test_promote_is_post() {
        let eps = all_endpoints();
        let promote = eps
            .iter()
            .find(|e| e.path.contains("promote"))
            .unwrap();
        assert_eq!(promote.method, HttpMethod::Post);
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
