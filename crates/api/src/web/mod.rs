//! Web (HTML) routes — server-rendered pages via Askama + HTMX.

pub mod auth;
pub mod dashboard;
pub mod warnings;
pub mod insights;
pub mod companies;
pub mod persons;
pub mod competitors;
pub mod memos;
pub mod search;
pub mod graph;
pub mod security;
pub mod settings;
pub mod admin;
pub mod recipes;
pub mod errors;

use crate::middleware::session::WebSession;

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

/// Common fields injected into every authenticated page template.
pub struct PageContext {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
}

impl PageContext {
    pub fn from_session(session: &WebSession, path: &str, warning_count: i64) -> Self {
        Self {
            current_path: path.to_string(),
            username: session.username.clone(),
            warning_count,
            theme: String::new(), // client-side via JS
        }
    }
}
