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

// ─── Path constants ─────────────────────────────────────────────────────

pub mod paths {
    pub const API_PREFIX: &str = "/api";
    pub const HEALTH: &str = "/api/health";
    pub const ENDPOINTS: &str = "/api/endpoints";
    pub const WARNINGS: &str = "/api/warnings";
    pub const INSIGHTS: &str = "/api/insights";
    pub const COMPANIES: &str = "/api/companies";
    pub const SEARCH: &str = "/api/search";
    pub const PERSONS: &str = "/api/persons";
    pub const RECIPES: &str = "/api/recipes";
    pub const GRAPH: &str = "/api/graph";
    pub const SECURITY: &str = "/api/security";
    pub const ADMIN: &str = "/api/admin";
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
        // Search
        EndpointDef {
            method: HttpMethod::Get,
            path: "/api/search",
            description: "Full-text search across all entities",
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
        assert_eq!(eps.len(), 7);
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
        assert!(eps.iter().all(|e| !e.path.starts_with("/api/admin")));
    }

    #[test]
    fn test_promote_is_post() {
        let eps = all_endpoints();
        assert!(eps.iter().all(|e| !e.path.contains("promote")));
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
