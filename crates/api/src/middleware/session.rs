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

use apex_core::identity::{UserId, Username};

use crate::auth::{ApiKey, ApiRole, Principal};
use crate::destructive_actions::ApiAuthContext;
use crate::middleware::auth::{auth_error_response, authenticate_api_request};
use crate::responses::ApiError;

type HmacSha256 = Hmac<Sha256>;
const CSRF_COOKIE_NAME: &str = "apex_csrf";
const LEGACY_SESSION_COOKIE_NAME: &str = "apex_session";
const SECURE_SESSION_COOKIE_NAME: &str = "__Host-apex_session";
/// Appearance cookies mirror the user's persisted `user_preferences` row so
/// the very first HTML response on a new device can apply the saved theme and
/// table layout without a client-side round-trip.
const THEME_COOKIE_NAME: &str = "apex_theme";
const TABLE_LAYOUT_COOKIE_NAME: &str = "apex_table_layout";
const APPEARANCE_COOKIE_MAX_AGE_SECS: i64 = 365 * 24 * 60 * 60;

/// Session payload version signed into new cookies. Missing `sv` in old
/// cookies means version 0.
pub const SESSION_VERSION: u32 = 1;
/// Default lifetime of a session cookie and its signed `exp` claim (24 hours)
/// when the user has not chosen a session length.
pub const SESSION_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// Bounds for the per-user session length preference (8h / 24h / 72h are the
/// offered choices; the clamp protects a tampered form post).
pub const MIN_SESSION_HOURS: i64 = 1;
pub const MAX_SESSION_HOURS: i64 = 168;
/// The offered session-length choices in the personal preferences form.
pub const SESSION_HOURS_CHOICES: [i64; 3] = [8, 24, 72];

/// Signed session lifetime for a stored `session_timeout_hours` preference.
///
/// The login handler signs `exp` with this value and the cookie `Max-Age`
/// derives from it, so the control is enforced server-side rather than being
/// a display-only setting.
pub fn session_ttl_ms_for_hours(hours: i64) -> i64 {
    hours.clamp(MIN_SESSION_HOURS, MAX_SESSION_HOURS) * 60 * 60 * 1000
}

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
    /// Canonical `app_users.id`; every ownership write uses this.
    pub user_id: UserId,
    /// Login/display name; never an ownership key.
    pub username: Username,
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
    pub user_id: UserId,
    pub username: Username,
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
        uid: claims.user_id.as_str(),
        sub: claims.username.as_str(),
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

    let username = Username::from(payload.sub);
    // Legacy pre-principal cookies carried no `uid`; the login name was the
    // identity then, so fall back to it. Sessions issued by the current login
    // always carry the canonical `app_users.id`.
    let user_id = payload
        .uid
        .map(UserId::from)
        .unwrap_or_else(|| UserId::from(username.as_str()));
    let principal_id = apex_core::alert_config::principal_uuid_from_user_id(&user_id);

    Some(WebSession {
        user_id,
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

/// `Set-Cookie` value issuing the session cookie with the given lifetime
/// (seconds). The login handler derives this from the user's persisted session
/// length preference so `Max-Age` and the signed `exp` claim always agree.
pub fn session_cookie_header(token: &str, max_age_secs: i64) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        session_cookie_name(),
        token,
        max_age_secs.max(0),
        cookie_secure_suffix()
    )
}

/// `Set-Cookie` values that mirror the user's persisted appearance
/// preferences (theme + table layout) to the browser.
///
/// They are deliberately readable by the client script in `base.html` so the
/// first paint on a new device uses the saved theme/layout; they contain no
/// secrets.
pub fn appearance_cookie_headers(theme: &str, table_layout: &str) -> Vec<String> {
    let suffix = cookie_secure_suffix();
    let mut cookies = Vec::new();
    if !theme.is_empty() {
        cookies.push(format!(
            "{THEME_COOKIE_NAME}={theme}; Path=/; SameSite=Lax; Max-Age={APPEARANCE_COOKIE_MAX_AGE_SECS}{suffix}"
        ));
    }
    if !table_layout.is_empty() {
        cookies.push(format!(
            "{TABLE_LAYOUT_COOKIE_NAME}={table_layout}; Path=/; SameSite=Lax; Max-Age={APPEARANCE_COOKIE_MAX_AGE_SECS}{suffix}"
        ));
    }
    cookies
}

/// Read the persisted table-layout preference out of a `user_preferences`
/// JSON blob (`preferences.settings_page.table_layout`).
pub fn table_layout_from_preferences(preferences: &serde_json::Value) -> Option<String> {
    preferences
        .get("settings_page")
        .and_then(|page| page.get("table_layout"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
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

    // Mirror the persisted personal appearance preferences (theme, table
    // layout) into cookies on safe navigations so a new device renders the
    // saved theme immediately. Only queried when a cookie is missing.
    let appearance = if !requires_csrf(&method)
        && (extract_cookie_value(request.headers(), THEME_COOKIE_NAME).is_none()
            || extract_cookie_value(request.headers(), TABLE_LAYOUT_COOKIE_NAME).is_none())
    {
        match request
            .extensions()
            .get::<Arc<apex_store::postgres::PgStore>>()
        {
            Some(store) => match store.get_user_preferences_record(&session.user_id).await {
                Ok(Some(record)) => Some((
                    record.theme,
                    table_layout_from_preferences(&record.preferences)
                        .unwrap_or_else(|| "comfortable".to_string()),
                )),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(%error, "failed to load appearance preferences for session");
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };

    let mut request = request;
    request.extensions_mut().insert(session);

    if !requires_csrf(&method) {
        let mut response = next.run(request).await;
        issue_csrf_cookie(&mut response, csrf_cookie.as_deref());
        if let Some((theme, table_layout)) = appearance {
            for cookie in appearance_cookie_headers(&theme, &table_layout) {
                if let Ok(value) = HeaderValue::from_str(&cookie) {
                    response.headers_mut().append(header::SET_COOKIE, value);
                }
            }
        }
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
            user_id: user_id.into(),
            username: username.into(),
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
            apex_core::alert_config::principal_uuid_from_user_id(&UserId::from("alice")),
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
                user_id: "usr-1".into(),
                username: "alice".into(),
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

    #[test]
    fn session_cookie_header_derives_max_age_from_preference() {
        let eight_hours = session_cookie_header("token", session_ttl_ms_for_hours(8) / 1000);
        assert!(eight_hours.contains("Max-Age=28800"));

        let default = session_cookie_header("token", SESSION_TTL_MS / 1000);
        assert!(default.contains("Max-Age=86400"));
    }

    #[test]
    fn session_ttl_is_clamped_to_safe_bounds() {
        assert_eq!(
            session_ttl_ms_for_hours(0),
            MIN_SESSION_HOURS * 60 * 60 * 1000
        );
        assert_eq!(
            session_ttl_ms_for_hours(10_000),
            MAX_SESSION_HOURS * 60 * 60 * 1000
        );
        assert_eq!(session_ttl_ms_for_hours(72), 72 * 60 * 60 * 1000);
    }

    #[test]
    fn appearance_cookies_carry_theme_and_layout() {
        let cookies = appearance_cookie_headers("dark", "compact");
        assert_eq!(cookies.len(), 2);
        assert!(cookies
            .iter()
            .any(|cookie| cookie.contains("apex_theme=dark")));
        assert!(cookies
            .iter()
            .any(|cookie| cookie.contains("apex_table_layout=compact")));
        // No HttpOnly: the base layout script must be able to read them.
        assert!(cookies.iter().all(|cookie| !cookie.contains("HttpOnly")));
    }

    #[test]
    fn appearance_cookies_skip_empty_values() {
        assert!(appearance_cookie_headers("", "").is_empty());
    }
}
