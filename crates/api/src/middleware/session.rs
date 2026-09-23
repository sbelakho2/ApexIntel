//! Session cookie middleware for HTML pages.
//!
//! Reads the `apex_session` cookie, verifies HMAC SHA-256 signature,
//! checks 24-hour expiry, and injects `WebSession` into request extensions.
//! Unauthenticated requests are redirected to `/login`.

use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const CSRF_COOKIE_NAME: &str = "apex_csrf";

/// Cached session secret — read from env once at first use instead of on every request.
static SESSION_SECRET: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| std::env::var("SESSION_SECRET").unwrap_or_default());
const CSRF_HEADER_NAME: &str = "x-csrf-token";
const CSRF_FORM_FIELD: &str = "csrf_token";
// Must match the router's DefaultBodyLimit (64 KiB) so a legitimately large
// form is rejected as over-limit rather than a misleading CSRF 403.
const MAX_CSRF_FORM_BYTES: usize = 64 * 1024;

/// Session data extracted from the cookie and injected into request extensions.
#[derive(Clone, Debug)]
pub struct WebSession {
    pub username: String,
    pub issued_at: i64,
}

pub fn validate_session(headers: &HeaderMap, session_secret: &str) -> Option<WebSession> {
    if session_secret.is_empty() {
        return None;
    }

    let token = extract_cookie_value(headers, "apex_session")?;

    let parts: Vec<&str> = token.splitn(2, '.').collect();
    if parts.len() != 2 {
        return None;
    }

    let payload_b64 = parts[0];
    let sig_hex = parts[1];

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;

    let mut mac = HmacSha256::new_from_slice(session_secret.as_bytes()).ok()?;
    mac.update(&payload_bytes);
    let expected = hex::encode(mac.finalize().into_bytes());

    if expected.as_bytes().ct_eq(sig_hex.as_bytes()).unwrap_u8() != 1 {
        return None;
    }

    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let username = payload.get("sub").and_then(|v| v.as_str())?.to_string();
    let issued_at = payload.get("iat").and_then(|v| v.as_i64())?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    if now_ms - issued_at > 24 * 60 * 60 * 1000 {
        return None;
    }

    Some(WebSession {
        username,
        issued_at,
    })
}

fn extract_cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookie_header| {
            cookie_header
                .split(';')
                .map(|c| c.trim())
                .find_map(|cookie| {
                    let (cookie_name, value) = cookie.split_once('=')?;
                    (cookie_name == name).then_some(value)
                })
        })
        .filter(|value| !value.is_empty())
}

/// `; Secure` when the deployment opts in via `COOKIE_SECURE=1` (required
/// behind HTTPS). Defaults off so local HTTP development keeps working.
pub(crate) fn cookie_secure_suffix() -> &'static str {
    match std::env::var("COOKIE_SECURE") {
        Ok(value) if value == "1" || value.eq_ignore_ascii_case("true") => "; Secure",
        _ => "",
    }
}

fn issue_csrf_cookie(response: &mut Response, existing: Option<&str>) {
    let generated_token;
    let token = match existing {
        Some(token) => token,
        None => {
            generated_token = Uuid::new_v4().to_string();
            &generated_token
        }
    };
    let cookie = format!(
        "{CSRF_COOKIE_NAME}={token}; Path=/; SameSite=Lax; Max-Age=86400{}",
        cookie_secure_suffix()
    );
    if let Ok(header_value) = HeaderValue::from_str(&cookie) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

fn requires_csrf(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The process-wide session secret (empty string when `SESSION_SECRET` is unset).
pub fn current_session_secret() -> &'static str {
    &SESSION_SECRET
}

/// Double-submit CSRF check for cookie-authenticated `/api/*` requests.
///
/// The JSON API accepts web-session cookies as a fallback principal (see
/// `require_auth` in main.rs). Bearer tokens are immune to CSRF, cookies are
/// not, so unsafe methods must echo the `apex_csrf` cookie in the
/// `x-csrf-token` header before the session fallback may be used.
pub fn api_session_csrf_ok(headers: &HeaderMap, method: &Method) -> bool {
    if !requires_csrf(method) {
        return true;
    }
    let Some(cookie_token) = extract_cookie_value(headers, CSRF_COOKIE_NAME) else {
        return false;
    };
    let header_token = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    !header_token.is_empty()
        && cookie_token
            .as_bytes()
            .ct_eq(header_token.as_bytes())
            .unwrap_u8()
            == 1
}

fn extract_form_csrf_token(body: &[u8]) -> Option<String> {
    url::form_urlencoded::parse(body)
        .find(|(key, _)| key == CSRF_FORM_FIELD)
        .map(|(_, value)| value.into_owned())
}

fn request_uses_urlencoded_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("application/x-www-form-urlencoded"))
        .unwrap_or(false)
}

fn validate_csrf_request(method: &Method, headers: &HeaderMap, body: &[u8]) -> bool {
    if !requires_csrf(method) {
        return true;
    }

    let Some(cookie_token) = extract_cookie_value(headers, CSRF_COOKIE_NAME) else {
        return false;
    };

    if let Some(header_token) = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    {
        return cookie_token
            .as_bytes()
            .ct_eq(header_token.as_bytes())
            .unwrap_u8()
            == 1;
    }

    if let Some(form_token) = extract_form_csrf_token(body) {
        return cookie_token
            .as_bytes()
            .ct_eq(form_token.as_bytes())
            .unwrap_u8()
            == 1;
    }

    false
}

/// Axum middleware: require a valid session cookie, redirect to `/login` otherwise.
pub async fn require_session(request: Request, next: Next) -> Response {
    let session_secret = &*SESSION_SECRET;
    if session_secret.is_empty() {
        tracing::error!("SESSION_SECRET not set — rejecting all web sessions");
        return Redirect::to("/login").into_response();
    }

    let session = match validate_session(request.headers(), session_secret) {
        Some(session) => session,
        None => return Redirect::to("/login").into_response(),
    };

    let method = request.method().clone();
    let csrf_cookie = extract_cookie_value(request.headers(), CSRF_COOKIE_NAME).map(str::to_string);

    let mut request = request;
    request.extensions_mut().insert(session);

    if !requires_csrf(&method) {
        let mut response = next.run(request).await;
        issue_csrf_cookie(&mut response, csrf_cookie.as_deref());
        return response;
    }

    if request
        .headers()
        .get(CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .is_some()
    {
        if !validate_csrf_request(&method, request.headers(), b"") {
            return (StatusCode::FORBIDDEN, "CSRF verification failed").into_response();
        }
        return next.run(request).await;
    }

    if !request_uses_urlencoded_form(request.headers()) {
        return (StatusCode::FORBIDDEN, "CSRF verification failed").into_response();
    }

    let (parts, body) = request.into_parts();
    let body_bytes = match to_bytes(body, MAX_CSRF_FORM_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::FORBIDDEN, "CSRF verification failed").into_response(),
    };

    if !validate_csrf_request(&method, &parts.headers, &body_bytes) {
        return (StatusCode::FORBIDDEN, "CSRF verification failed").into_response();
    }

    let request = Request::from_parts(parts, Body::from(body_bytes));
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    fn headers_with_cookie(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        headers
    }

    fn signed_session_cookie(username: &str, issued_at: i64, secret: &str) -> String {
        let payload = serde_json::json!({
            "sub": username,
            "iat": issued_at,
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&payload_bytes);
        let sig = hex::encode(mac.finalize().into_bytes());

        format!("apex_session={payload_b64}.{sig}")
    }

    #[test]
    fn test_validate_session_accepts_valid_signed_cookie() {
        let secret = "test-secret";
        let cookie = signed_session_cookie("alice", chrono::Utc::now().timestamp_millis(), secret);
        let headers = headers_with_cookie(&cookie);

        let session = validate_session(&headers, secret).expect("session should validate");

        assert_eq!(session.username, "alice");
    }

    #[test]
    fn test_validate_session_rejects_bad_signature() {
        let headers = headers_with_cookie("apex_session=bad.token");
        assert!(validate_session(&headers, "test-secret").is_none());
    }

    #[test]
    fn test_validate_csrf_request_rejects_post_without_matching_token() {
        let headers = headers_with_cookie("apex_session=test-session");
        assert!(!validate_csrf_request(&Method::POST, &headers, b""));
    }

    #[test]
    fn test_validate_csrf_request_accepts_matching_header_token() {
        let mut headers = headers_with_cookie("apex_session=test-session; apex_csrf=known-token");
        headers.insert(CSRF_HEADER_NAME, HeaderValue::from_static("known-token"));

        assert!(validate_csrf_request(&Method::POST, &headers, b""));
    }

    #[test]
    fn test_validate_csrf_request_accepts_matching_form_token() {
        let headers = headers_with_cookie("apex_session=test-session; apex_csrf=known-token");
        let body = b"username=alice&csrf_token=known-token";

        assert!(validate_csrf_request(&Method::POST, &headers, body));
    }

    #[test]
    fn test_validate_csrf_request_rejects_mismatched_header_token() {
        let mut headers = headers_with_cookie("apex_session=test-session; apex_csrf=known-token");
        headers.insert(CSRF_HEADER_NAME, HeaderValue::from_static("wrong-token"));

        assert!(!validate_csrf_request(&Method::POST, &headers, b""));
    }

    #[test]
    fn test_validate_csrf_request_accepts_safe_methods_without_token() {
        let headers = headers_with_cookie("apex_session=test-session");

        assert!(validate_csrf_request(&Method::GET, &headers, b""));
    }

    #[test]
    fn test_extract_form_csrf_token_reads_urlencoded_field() {
        let body = b"username=alice&csrf_token=known-token";
        assert_eq!(
            extract_form_csrf_token(body).as_deref(),
            Some("known-token")
        );
    }
}
