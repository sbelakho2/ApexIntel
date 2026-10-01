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
pub mod routes;
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

/// True only for a root-relative local path that stays local after browser
/// URL normalization.
///
/// Browsers strip ASCII tab/newline (and leading/trailing C0 controls) from a
/// URL before parsing it. A path such as `"/\t/evil"` therefore becomes
/// `"//evil"` — a protocol-relative external URL — and `"/\t\\evil"` becomes
/// `//evil` through backslash-to-slash conversion. Reject any control
/// character outright so no stored value can be promoted out of the origin.
fn is_safe_local_path(trimmed: &str) -> bool {
    trimmed.starts_with('/')
        && !trimmed.starts_with("//")
        && !trimmed.starts_with("/\\")
        && !trimmed.chars().any(|c| c.is_ascii_control())
}

/// Sanitize a URL for use in an `href`.
///
/// Askama HTML-escapes attribute values but does not care about the URL
/// scheme, so a stored `javascript:` URL still executes on click. Local
/// absolute paths pass through; absolute http(s) URLs are normalized; anything
/// else becomes `#`.
pub fn safe_href(raw: &str) -> String {
    let trimmed = raw.trim();
    if is_safe_local_path(trimmed) {
        return trimmed.to_string();
    }
    match url::Url::parse(trimmed) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url.into(),
        _ => "#".to_string(),
    }
}

/// A stored *internal* destination (notification action links): relative paths
/// only. Anything else is rejected rather than neutralized, because callers
/// need to decide how to render the absence.
pub fn safe_relative_href(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    is_safe_local_path(trimmed).then(|| trimmed.to_string())
}

/// Common fields injected into every authenticated page template.
pub struct PageContext {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    /// Principal role (e.g. `admin`, `analyst`).
    pub role: String,
    /// True when the principal's role passes `ApiRole::can_admin()`.
    pub can_admin: bool,
    /// True when the principal's role passes `ApiRole::can_write()`; templates
    /// hide write controls from Viewers/Service instead of rendering buttons
    /// that will be refused.
    pub can_write: bool,
    /// Measured system/data status for the `base.html` status strip. Replaces
    /// the previously hard-coded "System Online" / "Data Fresh" labels.
    pub status_strip: crate::system_status::StatusStrip,
}

impl PageContext {
    pub fn from_session(session: &WebSession, path: &str, warning_count: i64) -> Self {
        Self {
            current_path: path.to_string(),
            username: session.username.to_string(),
            warning_count,
            theme: String::new(), // client-side via JS
            role: session.role.as_str().to_string(),
            can_admin: session.role.can_admin(),
            can_write: session.role.can_write(),
            status_strip: crate::system_status::StatusStrip::current(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{safe_href, safe_relative_href};

    #[test]
    fn safe_href_allows_only_http_https_and_local_paths() {
        assert_eq!(safe_href("/warnings/123"), "/warnings/123");
        assert_eq!(safe_href("https://example.com/x"), "https://example.com/x");
        assert_eq!(safe_href("http://example.com/x"), "http://example.com/x");
        assert_eq!(
            safe_href("  https://example.com/x  "),
            "https://example.com/x"
        );

        // Hostile schemes become inert.
        assert_eq!(safe_href("javascript:alert(1)"), "#");
        assert_eq!(safe_href("JavaScript:alert(1)"), "#");
        assert_eq!(safe_href("data:text/html,<script>1</script>"), "#");
        assert_eq!(safe_href("vbscript:msgbox(1)"), "#");
        assert_eq!(safe_href("file:///etc/passwd"), "#");
        // Protocol-relative and backslash tricks do not pass as local paths.
        assert_eq!(safe_href("//evil.example/x"), "#");
        assert_eq!(safe_href("/\\evil.example"), "#");
        assert_eq!(safe_href(""), "#");
    }

    #[test]
    fn safe_relative_href_rejects_absolute_and_external() {
        assert_eq!(
            safe_relative_href("/notifications/42").as_deref(),
            Some("/notifications/42")
        );
        assert!(safe_relative_href("//evil.example/x").is_none());
        assert!(safe_relative_href("https://evil.example").is_none());
        assert!(safe_relative_href("javascript:alert(1)").is_none());
    }

    // ── Audit #52 bypass attempts ────────────────────────────────────────
    //
    // Browsers strip ASCII tab/newline from URLs before parsing. A value that
    // passes the naive `/`-prefix check but contains such a character can
    // therefore be promoted to a protocol-relative (external) URL:
    // `"/\t/evil"` is parsed by the browser as `"//evil"`. Every output of
    // safe_href/safe_relative_href must be inert after browser normalization.

    #[test]
    fn safe_href_rejects_control_character_path_obfuscation() {
        assert_eq!(safe_href("/\t/evil"), "#", "tab promotes path to //evil");
        assert_eq!(safe_href("/\n/evil"), "#", "LF promotes path to //evil");
        assert_eq!(safe_href("/\r/evil"), "#", "CR promotes path to //evil");
        assert_eq!(
            safe_href("/\t\\evil"),
            "#",
            "tab+backslash promotes to //evil"
        );
        assert_eq!(safe_href("/\u{000B}/evil"), "#", "VT must not pass");
        assert_eq!(safe_href("/\u{000C}/evil"), "#", "FF must not pass");
        assert_eq!(safe_href("/\u{0000}evil"), "#", "NUL must not pass");
    }

    #[test]
    fn safe_relative_href_rejects_control_character_path_obfuscation() {
        assert!(safe_relative_href("/\t/evil").is_none());
        assert!(safe_relative_href("/\n/evil").is_none());
        assert!(safe_relative_href("/\r/evil").is_none());
        assert!(safe_relative_href("/\n//evil").is_none());
        assert!(safe_relative_href("/\u{000B}/evil").is_none());
    }

    #[test]
    fn safe_href_bypass_attempts_stay_inert() {
        // Leading/trailing ASCII whitespace variants.
        assert_eq!(safe_href(" //evil.example"), "#");
        assert_eq!(safe_href("\t//evil.example"), "#");
        assert_eq!(safe_href("\n//evil.example"), "#");
        assert_eq!(safe_href("\\/evil.example"), "#");
        assert_eq!(safe_href("\\\\evil.example"), "#");

        // Empty host / malformed absolute URLs.
        assert_eq!(safe_href("http://"), "#");
        assert_eq!(safe_href("https://"), "#");

        // Scheme obfuscation inside the string.
        assert_eq!(safe_href("java\tscript:alert(1)"), "#");
        assert_eq!(safe_href("java\nscript:alert(1)"), "#");
        assert_eq!(safe_href("\u{0000}javascript:alert(1)"), "#");
        assert_eq!(safe_href("javascript\u{FF1A}alert(1)"), "#");
        assert_eq!(safe_href("data:text/html,x"), "#");
        assert_eq!(safe_href("file:///etc/passwd"), "#");
        assert_eq!(safe_href("vbscript:msgbox(1)"), "#");

        // `https:/evil` parses as the special-scheme URL `https://evil/`,
        // which is allowed (it is still https). It must never come back as a
        // protocol-relative or javascript value.
        let out = safe_href("https:/evil");
        assert!(
            out == "#" || out.starts_with("https://") || out.starts_with("http://"),
            "https:/evil produced {out:?}"
        );

        // Percent-encoded payloads stay a local path: browsers resolve the
        // literal leading `/` as a path and do not decode `%09` before scheme
        // detection, so this is inert (documented, not neutralized).
        assert_eq!(
            safe_href("/ %09javascript:alert(1)"),
            "/ %09javascript:alert(1)"
        );
        assert_eq!(safe_href("/ javascript:alert(1)"), "/ javascript:alert(1)");
    }

    #[test]
    fn safe_href_output_is_always_inert_after_browser_normalization() {
        let inputs = [
            "https://example.com/x",
            "HTTPS://EXAMPLE.COM/x",
            "http://example.com/x",
            "/warnings/123",
            "/\t/evil",
            "/\n/evil",
            "/\t\\evil",
            "/\\evil",
            "//evil",
            " //evil",
            "\t//evil",
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "\u{0000}javascript:alert(1)",
            "javascript\u{FF1A}alert(1)",
            "data:text/html,x",
            "file:///etc/passwd",
            "vbscript:x",
            "https:/evil",
            "http://",
            "https://",
            "",
            "   ",
            "/ %09javascript:alert(1)",
        ];
        for raw in inputs {
            let out = safe_href(raw);
            assert!(
                out == "#"
                    || out.starts_with("http://")
                    || out.starts_with("https://")
                    || out.starts_with('/'),
                "safe_href({raw:?}) = {out:?} is neither #, http(s), nor a local path"
            );
            // No output may be protocol-relative or backslash-relative.
            assert!(
                !out.starts_with("//") && !out.starts_with("/\\"),
                "safe_href({raw:?}) = {out:?} is protocol-relative"
            );
            // No output may carry browser-stripped control characters.
            assert!(
                !out.chars().any(|c| c.is_ascii_control()),
                "safe_href({raw:?}) = {out:?} contains control characters"
            );
            // No output may carry a non-http(s) scheme.
            if let Ok(url) = url::Url::parse(&out) {
                assert!(
                    url.scheme() == "http" || url.scheme() == "https" || url.scheme().is_empty(),
                    "safe_href({raw:?}) = {out:?} has scheme {}",
                    url.scheme()
                );
            }
        }
    }
}
