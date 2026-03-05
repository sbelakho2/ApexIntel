//! Error handlers — 404 Not Found and 500 Internal Server Error pages.
//!
//! Covers: user-friendly HTML error pages that fit the ApexIntel layout.

use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/404.html")]
pub struct NotFoundPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub requested_path: String,
}

#[derive(Template)]
#[template(path = "pages/500.html")]
pub struct InternalErrorPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub error_message: String,
    pub request_id: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// Fallback handler for unmatched routes — renders the 404 page.
pub async fn not_found() -> impl IntoResponse {
    let tpl = NotFoundPage {
        current_path: String::new(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        requested_path: String::new(),
    };

    (StatusCode::NOT_FOUND, tpl).into_response()
}

/// Handler for internal server errors — render 500 page with a request ID
/// so users can reference it in support requests.
pub async fn internal_error(error_message: &str, request_id: &str) -> Response {
    let tpl = InternalErrorPage {
        current_path: String::new(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        error_message: error_message.to_string(),
        request_id: request_id.to_string(),
    };

    (StatusCode::INTERNAL_SERVER_ERROR, tpl).into_response()
}

/// Convenience function: build a 404 response with session context.
pub fn not_found_with_context(username: &str, path: &str, warning_count: i64) -> Response {
    let tpl = NotFoundPage {
        current_path: path.to_string(),
        username: username.to_string(),
        warning_count,
        theme: String::new(),
        requested_path: path.to_string(),
    };

    (StatusCode::NOT_FOUND, tpl).into_response()
}

/// Convenience function: build a 500 response with session context.
pub fn internal_error_with_context(
    username: &str,
    warning_count: i64,
    error_message: &str,
    request_id: &str,
) -> Response {
    let tpl = InternalErrorPage {
        current_path: String::new(),
        username: username.to_string(),
        warning_count,
        theme: String::new(),
        error_message: error_message.to_string(),
        request_id: request_id.to_string(),
    };

    (StatusCode::INTERNAL_SERVER_ERROR, tpl).into_response()
}
