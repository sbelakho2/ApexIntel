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
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub requested_path: String,
}

#[derive(Template)]
#[template(path = "pages/500.html")]
pub struct InternalErrorPage {
    pub current_path: String,
    pub can_admin: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub error_message: String,
    pub request_id: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// Fallback handler for unmatched routes — renders the 404 page.
pub async fn not_found() -> impl IntoResponse {
    let tpl = NotFoundPage {
        current_path: String::new(),
        status_strip: crate::system_status::StatusStrip::current(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        can_admin: false,
        requested_path: String::new(),
    };

    super::render_template_with_status(StatusCode::NOT_FOUND, &tpl)
}

/// Handler for internal server errors — render 500 page with a request ID
/// so users can reference it in support requests.
pub async fn internal_error(error_message: &str, request_id: &str) -> Response {
    let tpl = InternalErrorPage {
        current_path: String::new(),
        status_strip: crate::system_status::StatusStrip::current(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        can_admin: false,
        error_message: error_message.to_string(),
        request_id: request_id.to_string(),
    };

    super::render_template_with_status(StatusCode::INTERNAL_SERVER_ERROR, &tpl)
}

/// Convenience function: build a 404 response with session context.
pub fn not_found_with_context(username: &str, path: &str, warning_count: i64) -> Response {
    let tpl = NotFoundPage {
        current_path: path.to_string(),
        status_strip: crate::system_status::StatusStrip::current(),
        username: username.to_string(),
        warning_count,
        theme: String::new(),
        can_admin: false,
        requested_path: path.to_string(),
    };

    super::render_template_with_status(StatusCode::NOT_FOUND, &tpl)
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
        status_strip: crate::system_status::StatusStrip::current(),
        username: username.to_string(),
        warning_count,
        theme: String::new(),
        can_admin: false,
        error_message: error_message.to_string(),
        request_id: request_id.to_string(),
    };

    super::render_template_with_status(StatusCode::INTERNAL_SERVER_ERROR, &tpl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(can_admin: bool) -> NotFoundPage {
        NotFoundPage {
            current_path: "/missing".to_string(),
            username: "tester".to_string(),
            warning_count: 0,
            theme: String::new(),
            can_admin,
            status_strip: crate::system_status::StatusStrip::unknown(),
            requested_path: "/missing".to_string(),
        }
    }

    #[test]
    fn admin_nav_is_hidden_without_admin_role() {
        let html = page(false).render().expect("render 404 page");
        assert!(!html.contains("href=\"/admin\""));
    }

    #[test]
    fn admin_nav_is_visible_for_admin_principal() {
        let html = page(true).render().expect("render 404 page");
        assert!(html.contains("href=\"/admin\""));
    }
}
