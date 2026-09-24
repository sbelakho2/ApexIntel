//! Web (HTML) routes — server-rendered pages via Askama + HTMX.

pub mod admin;
pub mod alert_settings;
pub mod auth;
pub mod battlecards;
pub mod collaboration;
pub mod companies;
pub mod competitors;
pub mod dashboard;
pub mod errors;
pub mod executive;
pub mod graph;
pub mod insights;
pub mod memos;
pub mod notifications;
pub mod persons;
pub mod recipes;
pub mod search;
pub mod security;
pub mod settings;
pub mod trends;
pub mod triage;
pub mod warnings;

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::middleware::session::WebSession;

/// HTML-escape text for hand-built `Html(format!(...))` fragments (B301).
///
/// Askama auto-escapes template output, but every raw `Html(format!())`
/// interpolation of crawled or user-supplied text must go through this —
/// warning titles, insight titles, review notes, and form echo-backs are all
/// attacker-controllable upstream content.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Helper: extract session from request extensions.
pub fn get_session(extensions: &axum::http::Extensions) -> Option<WebSession> {
    extensions.get::<WebSession>().cloned()
}

/// Check if request is an HTMX **partial** request (filter chips, pagination,
/// explicit hx-get with hx-target). Returns false for hx-boost navigation
/// because boost sends HX-Boosted: true and needs a full-page response.
pub fn is_htmx_request(headers: &axum::http::HeaderMap) -> bool {
    headers.contains_key("hx-request") && !headers.contains_key("hx-boosted")
}

pub fn render_template<T: Template>(template: &T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            tracing::error!("failed to render template: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Failed to render page".to_string()),
            )
                .into_response()
        }
    }
}

pub fn render_template_with_status<T: Template>(status: StatusCode, template: &T) -> Response {
    let mut response = render_template(template);
    *response.status_mut() = status;
    response
}

/// Common fields injected into every authenticated page template.
pub struct PageContext {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    /// Measured system/data status for the `base.html` status strip. Replaces
    /// the previously hard-coded "System Online" / "Data Fresh" labels.
    pub status_strip: crate::system_status::StatusStrip,
}

impl PageContext {
    pub fn from_session(session: &WebSession, path: &str, warning_count: i64) -> Self {
        Self {
            current_path: path.to_string(),
            username: session.username.clone(),
            warning_count,
            theme: String::new(), // client-side via JS
            status_strip: crate::system_status::StatusStrip::current(),
        }
    }
}
