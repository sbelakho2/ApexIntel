//! API crate — request/response types, auth, pagination, filtering, route definitions.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//!
//! All route handler types, query parameter validation, pagination logic,
//! auth token verification, and API response enveloping live here.
//! Actual Axum wiring (`Router::new().route(...)`) is done at the binary level.

pub mod alert_router;
pub mod api_keys;
pub mod auth;
pub mod config;
pub mod destructive_actions;
pub mod filters;
pub mod middleware;
pub mod pagination;
pub mod pdf_writer;
pub mod phase01;
pub mod rate_limit;
pub mod responses;
pub mod routes;
pub mod sse;
pub mod validation;
pub mod web;

pub const API_LLM_FEATURE_ENABLED: bool = cfg!(feature = "llm");
pub const API_EXPERIMENTAL_LLM_TOOL_CALLING_ENABLED: bool = cfg!(feature = "llm-tool-calling");
pub const API_VERSIONED_ALIAS_ENABLED: bool = true;
pub const API_OPENAPI_ENABLED: bool = true;
