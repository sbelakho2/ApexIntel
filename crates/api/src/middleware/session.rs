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
const CSRF_HEADER_NAME: &str = "x-csrf-token";
const CSRF_FORM_FIELD: &str = "csrf_token";
const MAX_CSRF_FORM_BYTES: usize = 16 * 1024;

/// Session data extracted from the cookie and injected into request extensions.
#[derive(Clone, Debug)]
pub struct WebSession {
    pub username: String,
    pub issued_at: i64,
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

fn issue_csrf_cookie(response: &mut Response, existing: Option<&str>) {
    let generated_token;
    let token = match existing {
        Some(token) => token,
        None => {
            generated_token = Uuid::new_v4().to_string();
            &generated_token
        }
    };
    let cookie = format!("{CSRF_COOKIE_NAME}={token}; Path=/; SameSite=Lax; Max-Age=86400");
    if let Ok(header_value) = HeaderValue::from_str(&cookie) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

fn requires_csrf(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
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
    let session_secret = std::env::var("SESSION_SECRET").unwrap_or_default();
    if session_secret.is_empty() {
        tracing::error!("SESSION_SECRET not set — rejecting all web sessions");
        return Redirect::to("/login").into_response();
    }

    // Extract cookie
    let token = extract_cookie_value(request.headers(), "apex_session");

    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => return Redirect::to("/login").into_response(),
    };

    // Verify token: base64url(payload).hmac_hex
    let parts: Vec<&str> = token.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Redirect::to("/login").into_response();
    }

    let payload_b64 = parts[0];
    let sig_hex = parts[1];

    // Decode payload
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let payload_bytes = match URL_SAFE_NO_PAD.decode(payload_b64) {
        Ok(b) => b,
        Err(_) => return Redirect::to("/login").into_response(),
    };

    // Compute expected HMAC
    let mut mac = match HmacSha256::new_from_slice(session_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return Redirect::to("/login").into_response(),
    };
    mac.update(&payload_bytes);
    let expected = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison
    if expected.as_bytes().ct_eq(sig_hex.as_bytes()).unwrap_u8() != 1 {
        return Redirect::to("/login").into_response();
    }

    // Parse payload JSON
    let payload: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
        Ok(v) => v,
        Err(_) => return Redirect::to("/login").into_response(),
    };

    let username = match payload.get("sub").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return Redirect::to("/login").into_response(),
    };

    let issued_at = match payload.get("iat").and_then(|v| v.as_i64()) {
        Some(ts) => ts,
        None => return Redirect::to("/login").into_response(),
    };

    // Check 24-hour expiry
    let now_ms = chrono::Utc::now().timestamp_millis();
    if now_ms - issued_at > 24 * 60 * 60 * 1000 {
        return Redirect::to("/login").into_response();
    }

    let method = request.method().clone();
    let csrf_cookie = extract_cookie_value(request.headers(), CSRF_COOKIE_NAME).map(str::to_string);

    let mut request = request;
    request.extensions_mut().insert(WebSession {
        username,
        issued_at,
    });

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

    fn headers_with_cookie(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        headers
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
