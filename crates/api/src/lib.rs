//! API crate — request/response types, auth, pagination, filtering, route definitions.
//!
//! All route handler types, query parameter validation, pagination logic,
//! auth token verification, and API response enveloping live here.
//! Actual Axum wiring (`Router::new().route(...)`) is done at the binary level.

pub mod auth;
pub mod pagination;
pub mod filters;
pub mod responses;
pub mod routes;
