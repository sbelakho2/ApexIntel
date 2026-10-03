//! Error handlers — 404 Not Found and 500 Internal Server Error pages.
//!
//! Covers: user-friendly HTML error pages that fit the ApexIntel layout.
//!
//! All pages are built from the requesting page's [`PageContext`] so the
//! principal, theme and status strip stay consistent with the rest of the
//! product. A 500 always carries an incident id and never renders the raw
//! error: internal details belong in the server log, not in the browser
//! (audit #152).

use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::PageContext;

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/404.html")]
pub struct NotFoundPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    pub requested_path: String,
}

#[derive(Template)]
#[template(path = "pages/403.html")]
pub struct ForbiddenPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
}

#[derive(Template)]
#[template(path = "pages/500.html")]
pub struct InternalErrorPage {
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,
    /// Always present: the id an operator quotes to support.
    pub incident_id: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// Fallback handler for unmatched routes — renders the 404 page for an
/// unauthenticated (or not-yet-resolved) request.
pub async fn not_found() -> impl IntoResponse {
    let tpl = NotFoundPage {
        current_path: String::new(),
        status_strip: crate::system_status::StatusStrip::current(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        can_admin: false,
        can_write: false,
        requested_path: String::new(),
    };

    super::render_template_with_status(StatusCode::NOT_FOUND, &tpl)
}

/// Styled 403 page for non-HTMX form posts that hit a write/admin guard.
pub fn forbidden() -> Response {
    let tpl = ForbiddenPage {
        current_path: String::new(),
        status_strip: crate::system_status::StatusStrip::current(),
        username: "anonymous".into(),
        warning_count: 0,
        theme: String::new(),
        can_admin: false,
        can_write: false,
    };

    super::render_template_with_status(StatusCode::FORBIDDEN, &tpl)
}

/// 404 for a missing resource, rendered from the requesting page's context.
pub fn not_found_for(ctx: &PageContext, requested_path: &str) -> Response {
    let tpl = NotFoundPage {
        current_path: ctx.current_path.clone(),
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username.clone(),
        warning_count: ctx.warning_count,
        theme: ctx.theme.clone(),
        status_strip: ctx.status_strip.clone(),
        requested_path: requested_path.to_string(),
    };

    super::render_template_with_status(StatusCode::NOT_FOUND, &tpl)
}

/// 500 for an internal failure, rendered from the requesting page's context.
///
/// `incident_id` is normalized to a non-empty identifier (one is generated
/// when the caller has none), and the raw error is never part of the page.
pub fn internal_error_for(ctx: &PageContext, incident_id: &str) -> Response {
    let incident_id = normalize_incident_id(incident_id);
    tracing::error!(incident_id = %incident_id, "rendering internal error page");
    let tpl = InternalErrorPage {
        current_path: ctx.current_path.clone(),
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username.clone(),
        warning_count: ctx.warning_count,
        theme: ctx.theme.clone(),
        status_strip: ctx.status_strip.clone(),
        incident_id,
    };

    super::render_template_with_status(StatusCode::INTERNAL_SERVER_ERROR, &tpl)
}

/// Guarantee a non-empty incident id: callers may pass a short correlation
/// token, an empty string, or an id colliding with nothing; a generated id is
/// used when none is supplied.
fn normalize_incident_id(incident_id: &str) -> String {
    let trimmed = incident_id.trim();
    if trimmed.is_empty() {
        let generated = uuid::Uuid::new_v4().simple().to_string();
        format!("inc-{}", &generated[..12])
    } else {
        trimmed.to_string()
    }
}

/// Compatibility constructor: build a 404 from the pieces available before a
/// `PageContext` exists (handlers that fail while loading navigation state).
pub fn not_found_with_context(username: &str, path: &str, warning_count: i64) -> Response {
    not_found_for(&error_context(username, path, warning_count), path)
}

/// Compatibility constructor for 500s. `error_message` is for the log only:
/// the rendered page carries the incident id, never the raw error (#152).
pub fn internal_error_with_context(
    username: &str,
    warning_count: i64,
    error_message: &str,
    request_id: &str,
) -> Response {
    let ctx = error_context(username, "", warning_count);
    let incident_id = normalize_incident_id(request_id);
    tracing::warn!(
        incident_id = %incident_id,
        context = %error_message,
        "rendering internal error page"
    );
    let tpl = InternalErrorPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        status_strip: ctx.status_strip,
        incident_id,
    };

    super::render_template_with_status(StatusCode::INTERNAL_SERVER_ERROR, &tpl)
}

/// A context for handlers that fail before the real page context is known.
/// Role-derived flags stay false (an error page exposes no write controls).
fn error_context(username: &str, path: &str, warning_count: i64) -> PageContext {
    PageContext {
        current_path: path.to_string(),
        username: username.to_string(),
        warning_count,
        theme: String::new(),
        role: String::new(),
        can_admin: false,
        can_write: false,
        status_strip: crate::system_status::StatusStrip::current(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn page(can_admin: bool) -> NotFoundPage {
        NotFoundPage {
            current_path: "/missing".to_string(),
            username: "tester".to_string(),
            warning_count: 0,
            theme: String::new(),
            can_admin,
            // A test page with an admin principal is also writable.
            can_write: can_admin,
            status_strip: crate::system_status::StatusStrip::unknown(),
            requested_path: "/missing".to_string(),
        }
    }

    /// Non-writing principals get the `apex-readonly` body class that hides
    /// write controls (CSS fallback); the route guards remain enforcement.
    #[test]
    fn readonly_class_marks_non_writing_principals() {
        let viewer_html = page(false).render().expect("render 404 page");
        assert!(viewer_html.contains("<body data-sse class=\"apex-readonly\">"));
        let admin_html = page(true).render().expect("render 404 page");
        assert!(!admin_html.contains("<body data-sse class=\"apex-readonly\">"));
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

    // ── #152: PageContext-built errors with a guaranteed incident id ─────

    fn ctx(can_admin: bool) -> PageContext {
        PageContext {
            current_path: "/insights".to_string(),
            username: "analyst".to_string(),
            warning_count: 3,
            theme: "dark".to_string(),
            role: "admin".to_string(),
            can_admin,
            can_write: can_admin,
            status_strip: crate::system_status::StatusStrip::unknown(),
        }
    }

    #[test]
    fn not_found_for_keeps_the_page_context() {
        let response = not_found_for(&ctx(true), "/insights/xyz");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn incident_id_is_always_present_and_never_empty() {
        assert_eq!(normalize_incident_id("inc-123"), "inc-123");
        let generated = normalize_incident_id("   ");
        assert!(generated.starts_with("inc-"));
        assert!(generated.len() > 4);
        let generated_again = normalize_incident_id("");
        assert_ne!(generated, generated_again, "generated ids are unique");
    }

    #[test]
    fn internal_error_page_renders_only_the_incident_id() {
        let tpl = InternalErrorPage {
            current_path: "/x".to_string(),
            can_admin: false,
            can_write: false,
            username: "analyst".to_string(),
            warning_count: 0,
            theme: String::new(),
            status_strip: crate::system_status::StatusStrip::unknown(),
            incident_id: normalize_incident_id("inc-abc123"),
        };
        let html = tpl.render().expect("render 500 page");
        assert!(html.contains("inc-abc123"));
        assert!(html.contains("Incident ID"));
        // The raw-error slot no longer exists in the template.
        assert!(!html.contains("Error Context"));
    }
}
