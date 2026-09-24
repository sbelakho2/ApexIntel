//! Session cookie middleware for HTML pages.
//!
//! Reads the session cookie (`__Host-apex_session` when `COOKIE_SECURE=1`,
//! `apex_session` otherwise), verifies the HMAC SHA-256 signature, checks the
//! expiry, and injects [`WebSession`] into request extensions. Unauthenticated
//! requests are redirected to `/login`.
//!
//! Sessions signed before the P0 principal change (payload without `uid`,
//! `role`, `exp` or `sv`) still validate: the role defaults to
//! [`ApiRole::Analyst`] and the legacy 24-hour `iat` expiry rule applies.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::auth::{ApiKey, ApiRole, Principal};
use crate::destructive_actions::ApiAuthContext;
use crate::middleware::auth::{auth_error_response, authenticate_api_request};
use crate::responses::ApiError;

type HmacSha256 = Hmac<Sha256>;
const CSRF_COOKIE_NAME: &str = "apex_csrf";
const LEGACY_SESSION_COOKIE_NAME: &str = "apex_session";
const SECURE_SESSION_COOKIE_NAME: &str = "__Host-apex_session";

/// Session payload version signed into new cookies. Missing `sv` in old
/// cookies means version 0.
pub const SESSION_VERSION: u32 = 1;
/// Lifetime of a session cookie and its signed `exp` claim (24 hours).
pub const SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

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
    pub user_id: String,
    pub username: String,
    pub role: ApiRole,
    pub session_version: u32,
    /// Stable principal UUID used to key real-time connections and to address alerts.
    pub principal_id: Uuid,
    pub issued_at: i64,
    /// Signed expiry (`exp`) claim; `None` for legacy sessions that predate it.
    pub expires_at: Option<i64>,
}

impl WebSession {
    /// The principal this browser session acts as.
    pub fn principal(&self) -> Principal {
        Principal::new(
            self.user_id.clone(),
            self.username.clone(),
            self.role.clone(),
            self.session_version,
        )
    }

    /// Can this session access admin-only surfaces?
    pub fn can_admin(&self) -> bool {
        self.role.can_admin()
    }
}

/// Claims signed into a browser session cookie.
#[derive(Debug, Clone)]
pub struct SessionClaims {
    pub user_id: String,
    pub username: String,
    pub role: ApiRole,
    pub issued_at: i64,
    pub expires_at: i64,
    pub session_version: u32,
}

/// Sign a session payload: `base64url(JSON) + "." + HMAC-SHA256 hex`.
///
/// The JSON carries `uid`, `sub`, `role`, `iat`, `exp` and `sv`.
pub fn create_session_token(claims: &SessionClaims, session_secret: &str) -> Option<String> {
    #[derive(serde::Serialize)]
    struct SessionPayload<'a> {
        uid: &'a str,
        sub: &'a str,
        role: &'a str,
        iat: i64,
        exp: i64,
        sv: u32,
    }

    let payload = SessionPayload {
        uid: &claims.user_id,
        sub: &claims.username,
        role: claims.role.as_str(),
        iat: claims.issued_at,
        exp: claims.expires_at,
        sv: claims.session_version,
    };
    let payload_bytes = serde_json::to_vec(&payload).ok()?;

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);

    let mut mac = HmacSha256::new_from_slice(session_secret.as_bytes()).ok()?;
    mac.update(&payload_bytes);
    let sig = hex::encode(mac.finalize().into_bytes());

    Some(format!("{}.{}", payload_b64, sig))
}

/// Payload shape accepted by [`validate_session`]. Every field except `sub`
/// and `iat` is optional so pre-principal cookies keep validating.
#[derive(serde::Deserialize)]
struct StoredSessionPayload {
    #[serde(default)]
    uid: Option<String>,
    sub: String,
    #[serde(default)]
    role: Option<String>,
    iat: i64,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    sv: u32,
}

pub fn validate_session(headers: &HeaderMap, session_secret: &str) -> Option<WebSession> {
    if session_secret.is_empty() {
        return None;
    }

    let token = extract_session_cookie(headers)?;

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

    let payload: StoredSessionPayload = serde_json::from_slice(&payload_bytes).ok()?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    if let Some(expires_at) = payload.exp {
        if now_ms > expires_at {
            return None;
        }
    } else if now_ms - payload.iat > SESSION_TTL_MS {
        // Legacy cookies (no `exp`) keep the historical 24-hour TTL.
        return None;
    }

    let role = match payload.role.as_deref() {
        // Pre-principal cookies carried no role; Analyst was the fallback then
        // and remains the safe default now.
        None => ApiRole::Analyst,
        Some(raw) => raw.parse::<ApiRole>().unwrap_or_else(|_| {
            tracing::warn!(role = %raw, "unknown role in session payload; defaulting to analyst");
            ApiRole::Analyst
        }),
    };

    let username = payload.sub;
    let principal_id = apex_core::alert_config::user_principal_id(&username);

    Some(WebSession {
        user_id: payload.uid.unwrap_or_else(|| username.clone()),
        username,
        role,
        session_version: payload.sv,
        principal_id,
        issued_at: payload.iat,
        expires_at: payload.exp,
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

/// Read the session cookie. Under `COOKIE_SECURE=1` the `__Host-`-prefixed
/// cookie is preferred, with the pre-rename `apex_session` cookie still
/// accepted so logged-in users are not evicted by the rename.
fn extract_session_cookie(headers: &HeaderMap) -> Option<&str> {
    if cookie_secure_enabled() {
        extract_cookie_value(headers, SECURE_SESSION_COOKIE_NAME)
            .or_else(|| extract_cookie_value(headers, LEGACY_SESSION_COOKIE_NAME))
    } else {
        extract_cookie_value(headers, LEGACY_SESSION_COOKIE_NAME)
    }
}

/// `; Secure` when the deployment opts in via `COOKIE_SECURE=1` (required
/// behind HTTPS). Defaults off so local HTTP development keeps working.
pub(crate) fn cookie_secure_suffix() -> &'static str {
    if cookie_secure_enabled() {
        "; Secure"
    } else {
        ""
    }
}

fn cookie_secure_enabled() -> bool {
    matches!(std::env::var("COOKIE_SECURE"), Ok(value) if value == "1" || value.eq_ignore_ascii_case("true"))
}

/// Cookie name for the browser session. The `__Host-` prefix is only valid
/// over HTTPS with `Secure`, so it is used exactly when `COOKIE_SECURE=1`.
fn session_cookie_name_for(secure: bool) -> &'static str {
    if secure {
        SECURE_SESSION_COOKIE_NAME
    } else {
        LEGACY_SESSION_COOKIE_NAME
    }
}

/// Cookie name for the browser session under the current deployment settings.
pub fn session_cookie_name() -> &'static str {
    session_cookie_name_for(cookie_secure_enabled())
}

/// `Set-Cookie` value issuing the session cookie.
pub fn session_cookie_header(token: &str) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400{}",
        session_cookie_name(),
        token,
        cookie_secure_suffix()
    )
}

/// `Set-Cookie` values clearing the session cookie. Under `COOKIE_SECURE=1`
/// the legacy pre-rename cookie is cleared as well so a stale duplicated
/// cookie cannot linger after logout.
pub fn clear_session_cookie_headers() -> Vec<String> {
    let mut names = vec![session_cookie_name()];
    if cookie_secure_enabled() {
        names.push(LEGACY_SESSION_COOKIE_NAME);
    }
    names
        .into_iter()
        .map(|name| {
            format!(
                "{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
                cookie_secure_suffix()
            )
        })
        .collect()
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
/// `require_api_auth`). Bearer tokens are immune to CSRF, cookies are not, so
/// unsafe methods must echo the `apex_csrf` cookie in the `x-csrf-token`
/// header before the session fallback may be used.
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

/// Build an [`ApiAuthContext`] from the browser session cookie — the
/// cookie-authenticated fallback for `/api/*` endpoints. Returns `None` when a
/// Bearer key was presented (that path is handled by the caller), the session
/// is missing/expired, or an unsafe method fails the CSRF check. The principal
/// role comes from the signed session payload.
pub fn session_api_context(headers: &HeaderMap, method: &Method) -> Option<ApiAuthContext> {
    if headers.contains_key(header::AUTHORIZATION) {
        return None;
    }
    let secret = current_session_secret();
    if secret.is_empty() {
        return None;
    }
    let session = validate_session(headers, secret)?;
    if !api_session_csrf_ok(headers, method) {
        tracing::warn!(
            username = %session.username,
            "session-authenticated API request rejected: CSRF verification failed"
        );
        return None;
    }
    Some(ApiAuthContext {
        key_id: "web-session".to_string(),
        user_id: session.user_id,
        role: session.role,
    })
}

/// State for [`require_api_auth`] — the API key registry.
#[derive(Clone)]
pub struct ApiAuthState {
    pub api_keys: Arc<HashMap<String, ApiKey>>,
}

/// API authentication middleware with browser-session fallback.
///
/// Bearer API keys are validated first. When no usable Authorization header is
/// present, a valid browser session is accepted as its signed role — but only
/// when the double-submit CSRF check passes for unsafe methods. An invalid
/// Bearer key fails instead of silently downgrading to the session.
pub async fn require_api_auth(
    State(state): State<ApiAuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let result = authenticate_api_request(
        request.headers(),
        request.method(),
        &state.api_keys,
        chrono::Utc::now(),
    );

    match result {
        Ok(auth) => {
            request.extensions_mut().insert(auth.auth_context);
            next.run(request).await
        }
        Err(api_err) => {
            if let Some(ctx) = session_api_context(request.headers(), request.method()) {
                request.extensions_mut().insert(ctx);
                return next.run(request).await;
            }
            auth_error_response(api_err)
        }
    }
}

/// Admin-only route guard: restricted to principals whose role passes
/// [`ApiRole::can_admin`]. Runs after [`require_api_auth`], which inserts the
/// `ApiAuthContext` extension.
pub async fn require_admin(
    axum::Extension(auth): axum::Extension<ApiAuthContext>,
    request: Request,
    next: Next,
) -> Response {
    if !auth.role.can_admin() {
        return auth_error_response(ApiError::forbidden("Admin role required"));
    }
    next.run(request).await
}

/// Web-page admin guard: restricted to browser sessions whose role passes
/// [`ApiRole::can_admin`]. Runs after [`require_session`].
pub async fn require_web_admin(request: Request, next: Next) -> Response {
    let Some(session) = request.extensions().get::<WebSession>() else {
        return Redirect::to("/login").into_response();
    };
    if !session.role.can_admin() {
        tracing::warn!(
            username = %session.username,
            role = %session.role.as_str(),
            "web page access denied: admin role required"
        );
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }
    next.run(request).await
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

    fn signed_principal_cookie(
        user_id: &str,
        username: &str,
        role: ApiRole,
        secret: &str,
    ) -> String {
        let now = chrono::Utc::now().timestamp_millis();
        let claims = SessionClaims {
            user_id: user_id.to_string(),
            username: username.to_string(),
            role,
            issued_at: now,
            expires_at: now + SESSION_TTL_MS,
            session_version: SESSION_VERSION,
        };
        let token = create_session_token(&claims, secret).unwrap();
        format!("{}={token}", session_cookie_name())
    }

    #[test]
    fn test_validate_session_accepts_valid_signed_cookie() {
        let secret = "test-secret";
        let cookie = signed_session_cookie("alice", chrono::Utc::now().timestamp_millis(), secret);
        let headers = headers_with_cookie(&cookie);

        let session = validate_session(&headers, secret).expect("session should validate");

        assert_eq!(session.username, "alice");
        assert_eq!(
            session.principal_id,
            apex_core::alert_config::user_principal_id("alice"),
            "session must carry the stable principal ID"
        );
    }

    #[test]
    fn legacy_session_without_role_defaults_to_analyst() {
        let secret = "test-secret";
        let cookie = signed_session_cookie("legacy", chrono::Utc::now().timestamp_millis(), secret);
        let headers = headers_with_cookie(&cookie);

        let session = validate_session(&headers, secret).expect("legacy session should validate");

        assert_eq!(session.role, ApiRole::Analyst);
        assert_eq!(session.user_id, "legacy");
        assert_eq!(session.session_version, 0);
        assert!(!session.can_admin());
    }

    #[test]
    fn principal_session_round_trips_role_and_user_id() {
        let secret = "test-secret";
        let cookie = signed_principal_cookie("usr-admin", "admin", ApiRole::Admin, secret);
        let headers = headers_with_cookie(&cookie);

        let session = validate_session(&headers, secret).expect("session should validate");

        assert_eq!(session.user_id, "usr-admin");
        assert_eq!(session.username, "admin");
        assert_eq!(session.role, ApiRole::Admin);
        assert_eq!(session.session_version, SESSION_VERSION);
        assert!(session.can_admin());
        assert!(session.expires_at.is_some());
        assert_eq!(session.principal().role, ApiRole::Admin);
    }

    #[test]
    fn expired_principal_session_is_rejected() {
        let secret = "test-secret";
        let now = chrono::Utc::now().timestamp_millis();
        let token = create_session_token(
            &SessionClaims {
                user_id: "usr-1".to_string(),
                username: "alice".to_string(),
                role: ApiRole::Admin,
                issued_at: now - 2 * SESSION_TTL_MS,
                expires_at: now - SESSION_TTL_MS,
                session_version: SESSION_VERSION,
            },
            secret,
        )
        .unwrap();
        let headers = headers_with_cookie(&format!("{}={token}", session_cookie_name()));

        assert!(validate_session(&headers, secret).is_none());
    }

    #[test]
    fn test_validate_session_rejects_bad_signature() {
        let headers = headers_with_cookie("apex_session=bad.token");
        assert!(validate_session(&headers, "test-secret").is_none());
    }

    #[test]
    fn legacy_cookie_name_still_valid_under_secure_deployment() {
        // Signed with the legacy cookie name; validation must fall back to it
        // even when the deployment has switched to `__Host-apex_session`.
        let secret = "test-secret";
        let cookie = signed_principal_cookie("usr-admin", "admin", ApiRole::Admin, secret);
        let headers = headers_with_cookie(&cookie.replace("__Host-apex_session=", "apex_session="));

        assert!(validate_session(&headers, secret).is_some());
        assert_eq!(session_cookie_name_for(true), "__Host-apex_session");
        assert_eq!(session_cookie_name_for(false), "apex_session");
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
