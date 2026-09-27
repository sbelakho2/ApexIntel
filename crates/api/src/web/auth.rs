//! Login / logout handlers for HTML pages.
//!
//! Password verification supports Argon2id PHC strings (`$argon2id$...`,
//! preferred) and the legacy 64-char SHA-256 hex digest (accepted with a
//! deprecation warning so existing deployments keep working).
//!
//! `app_users` is canonical: logins resolve against the database record (id,
//! username, password_hash, role, enabled, session_version). Environment
//! credentials — `WEB_USERS_JSON` (JSON array of
//! `{id, username, password_hash, role}`) or the legacy single
//! `APEX_ADMIN_USERNAME` / `APEX_ADMIN_PASSWORD_HASH` pair (admin role) — are
//! bootstrap only: they seed rows that do not yet carry a password hash. Once
//! a row has credentials the environment is ignored. Without a store (unit
//! tests, no-database deployments) the environment list is used directly.
//!
//! Every attempt is throttled through [`LoginThrottle`] under a key built from
//! the normalised login name plus a trusted client fingerprint: evaluated
//! before password verification and recorded after it. Login names are
//! canonical (`lower(username)`, migration 070) and a configured user whose
//! role is missing or unknown is rejected — never silently downgraded to a
//! default role.
//!
//! ## Browser-login policy
//!
//! Logging in only establishes identity; it grants no capability by itself.
//! All four canonical roles may authenticate, and the browser router
//! (`crate::web::routes`) then enforces the role on every request:
//! `require_session` on all pages, `require_web_write` on every mutating page
//! and `require_web_admin` on `/admin`. A Viewer or Service session can read
//! but never mutate, and an Analyst can mutate but never administer. The role
//! is re-resolved from `app_users` on each request, so the session cookie's
//! claims never outlive a role change or a disabled account.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Form,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use apex_core::identity::{UserId, Username};
use apex_store::postgres::{AppUserSeed, PgStore};

use crate::auth::{client_fingerprint, ApiRole, AuthThrottleStatus};
use crate::login_throttle::LoginThrottle;
use crate::middleware::session::{
    clear_session_cookie_headers, create_session_token, session_cookie_header,
    session_ttl_ms_for_hours, SessionClaims, SESSION_TTL_MS, SESSION_VERSION,
};

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginPage {
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

/// A configured web login. `WEB_USERS_JSON` entries carry `id`, `username`,
/// `password_hash` and `role`; `id` defaults to the username when omitted.
#[derive(Debug, Clone, Deserialize)]
pub struct WebUser {
    #[serde(default)]
    pub id: String,
    pub username: String,
    pub password_hash: String,
    #[serde(default)]
    pub role: String,
}

impl WebUser {
    /// Principal user id — falls back to the username when not configured.
    pub fn user_id(&self) -> &str {
        if self.id.is_empty() {
            &self.username
        } else {
            &self.id
        }
    }

    /// Parsed role. A missing or malformed role is an error — unknown roles
    /// must never silently fall back to a privilege (not even to the old
    /// `analyst` default).
    pub fn api_role(&self) -> anyhow::Result<ApiRole> {
        let role = self.role.trim();
        if role.is_empty() {
            anyhow::bail!("web user '{}' has no role", self.username);
        }
        role.parse::<ApiRole>()
            .map_err(|_| anyhow::anyhow!("web user '{}' has unknown role '{role}'", self.username))
    }
}

/// Canonical login name: trimmed and case-folded, matching the
/// `lower(username)` uniqueness enforced by migration 070.
pub fn normalize_login_name(username: &str) -> String {
    username.trim().to_lowercase()
}

/// Validate environment-configured web users before any of them is trusted.
///
/// Rejects malformed roles, empty identifiers/hashes, duplicate ids and
/// duplicate login names (case-insensitive). An ambiguous or malformed
/// `WEB_USERS_JSON` must not bootstrap credentials or authenticate.
pub fn validate_web_users(users: &[WebUser]) -> Result<(), String> {
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut names: HashMap<String, usize> = HashMap::new();

    for (index, user) in users.iter().enumerate() {
        let id = user.user_id().trim().to_string();
        if id.is_empty() {
            return Err(format!("entry {index}: id/username must not be empty"));
        }
        if user.username.trim().is_empty() {
            return Err(format!("entry {index}: username must not be empty"));
        }
        if user.password_hash.trim().is_empty() {
            return Err(format!(
                "entry {index} ('{}'): password_hash must not be empty",
                user.username
            ));
        }
        user.api_role().map_err(|error| error.to_string())?;

        if let Some(previous) = ids.insert(id.clone(), index) {
            return Err(format!(
                "duplicate id '{id}' in WEB_USERS_JSON entries {previous} and {index}"
            ));
        }
        let normalized = normalize_login_name(&user.username);
        if let Some(previous) = names.insert(normalized.clone(), index) {
            return Err(format!(
                "duplicate username '{normalized}' in WEB_USERS_JSON entries {previous} and {index}"
            ));
        }
    }
    Ok(())
}

/// Load configured web users: `WEB_USERS_JSON` when set, otherwise the single
/// `APEX_ADMIN_USERNAME` / `APEX_ADMIN_PASSWORD_HASH` admin pair. Returns an
/// empty list (with an error log) when configuration is missing, malformed or
/// ambiguous.
pub fn load_web_users() -> Vec<WebUser> {
    if let Ok(raw) = std::env::var("WEB_USERS_JSON") {
        let raw = raw.trim();
        if !raw.is_empty() {
            match serde_json::from_str::<Vec<WebUser>>(raw) {
                Ok(users) if !users.is_empty() => match validate_web_users(&users) {
                    Ok(()) => return users,
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "WEB_USERS_JSON rejected; no web users loaded from the environment"
                        );
                        return Vec::new();
                    }
                },
                Ok(_) => {
                    tracing::error!("WEB_USERS_JSON is an empty array; no web users configured")
                }
                Err(err) => {
                    tracing::error!(%err, "WEB_USERS_JSON is not a valid JSON array of web users")
                }
            }
            return Vec::new();
        }
    }

    let username = std::env::var("APEX_ADMIN_USERNAME").unwrap_or_default();
    let password_hash = std::env::var("APEX_ADMIN_PASSWORD_HASH").unwrap_or_default();
    if username.is_empty() || password_hash.is_empty() {
        return Vec::new();
    }
    vec![WebUser {
        id: username.clone(),
        username,
        password_hash,
        role: ApiRole::Admin.as_str().to_string(),
    }]
}

/// The authenticated principal resolved by a login attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginPrincipal {
    pub user_id: UserId,
    pub username: Username,
    pub role: ApiRole,
    pub session_version: u32,
}

fn bootstrap_seeds() -> Vec<AppUserSeed> {
    let mut seeds = Vec::new();
    for user in load_web_users() {
        let role = match user.api_role() {
            Ok(role) => role,
            Err(error) => {
                tracing::error!(username = %user.username, %error, "skipping web user with unusable role");
                continue;
            }
        };
        seeds.push(AppUserSeed {
            id: user.user_id().to_string(),
            // Store the canonical (trimmed) name so it always matches the
            // normalized lookup; a padded configured name cannot authenticate.
            username: user.username.trim().to_string(),
            password_hash: user.password_hash,
            role: role.as_str().to_string(),
        });
    }
    seeds
}

/// Seed environment-configured credentials into `app_users`.
///
/// Returns the number of rows inserted or adopted; rows that already carry a
/// password hash are left untouched. Failures are logged rather than fatal:
/// existing database users must stay able to log in even when the bootstrap
/// configuration is broken.
pub async fn bootstrap_app_users_from_env(store: &PgStore) -> usize {
    let seeds = bootstrap_seeds();
    if seeds.is_empty() {
        return 0;
    }
    match store.bootstrap_app_users(&seeds).await {
        Ok(applied) => applied,
        Err(error) => {
            tracing::warn!(%error, "failed to bootstrap app_users credentials from environment");
            0
        }
    }
}

/// Authenticate a login against the canonical `app_users` table.
///
/// Environment credentials are only ever used to bootstrap rows without a
/// password hash; once a row has credentials the database is authoritative and
/// the environment is ignored. When `store` is `None` (unit tests, no-database
/// deployments) the environment list is the credential source.
pub async fn resolve_login(
    store: Option<&PgStore>,
    username: &str,
    password: &str,
) -> Option<LoginPrincipal> {
    let normalized = normalize_login_name(username);

    let Some(store) = store else {
        let user = load_web_users()
            .into_iter()
            .find(|user| normalize_login_name(&user.username) == normalized)?;
        let role = match user.api_role() {
            Ok(role) => role,
            Err(error) => {
                tracing::error!(
                    username = %user.username,
                    %error,
                    "login rejected: unknown role in WEB_USERS_JSON"
                );
                return None;
            }
        };
        if !verify_password_hash(password, &user.password_hash) {
            return None;
        }
        let user_id = UserId::from(user.user_id());
        let username = Username::from(user.username.trim());
        return Some(LoginPrincipal {
            user_id,
            username,
            role,
            session_version: SESSION_VERSION,
        });
    };

    bootstrap_app_users_from_env(store).await;

    let record = store.find_app_user_by_username(username).await.ok()??;
    if !record.enabled {
        tracing::warn!(user_id = %record.id, "login rejected: account disabled");
        return None;
    }
    // An unknown role must fail authentication, never fall back to a default
    // role. The database CHECK (migration 071) is the backstop; this is the
    // runtime guard for schemas migrated out-of-band.
    let role = match record.role.trim().parse::<ApiRole>() {
        Ok(role) => role,
        Err(_) => {
            tracing::error!(
                user_id = %record.id,
                role = %record.role,
                "login rejected: unknown role in app_users"
            );
            return None;
        }
    };
    let password_hash = record.password_hash.as_deref()?;
    if !verify_password_hash(password, password_hash) {
        return None;
    }

    // The update re-checks `enabled`, so a row disabled between the lookup and
    // the credential check still fails closed.
    let record = store
        .record_app_user_login(&record.id)
        .await
        .ok()
        .flatten()?;

    Some(LoginPrincipal {
        user_id: UserId::from(record.id),
        username: Username::from(record.username),
        role,
        session_version: record.session_version.max(0) as u32,
    })
}

/// Verify a password against a stored hash. Accepts Argon2id PHC strings and
/// the legacy SHA-256 hex digest (with a deprecation warning).
pub fn verify_password_hash(password: &str, stored_hash: &str) -> bool {
    let stored_hash = stored_hash.trim();

    if stored_hash.starts_with("$argon2id$") {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        use argon2::Argon2;

        return match PasswordHash::new(stored_hash) {
            Ok(parsed) => Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok(),
            Err(err) => {
                tracing::error!(%err, "invalid Argon2 password hash in configuration");
                false
            }
        };
    }

    if stored_hash.len() == 64 && stored_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        tracing::warn!("legacy SHA-256 password hash accepted — migrate to an Argon2id PHC hash");
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let computed = hex::encode(hasher.finalize());
        let stored_normalized = stored_hash.to_ascii_lowercase();
        return computed
            .as_bytes()
            .ct_eq(stored_normalized.as_bytes())
            .unwrap_u8()
            == 1;
    }

    tracing::error!("unsupported password hash format in configuration");
    false
}

/// Hash a password with Argon2id, returning a PHC string suitable for
/// `APEX_ADMIN_PASSWORD_HASH` / `WEB_USERS_JSON`. Exposed for provisioning
/// tooling (see `cargo run -p apex-api --example hash_password`).
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    use argon2::Argon2;

    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

/// GET /login — render login page.
pub async fn login_page() -> impl IntoResponse {
    LoginPage { error: None }
}

/// POST /login — validate credentials, set session cookie, redirect.
///
/// The session lifetime comes from the user's persisted personal preferences
/// (`user_preferences.settings_page.session_timeout_hours`): both the signed
/// `exp` claim and the cookie `Max-Age` derive from it, so the setting is
/// enforced server-side instead of being a display-only control.
pub async fn login_submit(
    parts: axum::http::request::Parts,
    Form(form): Form<LoginForm>,
) -> Response {
    let session_secret = std::env::var("SESSION_SECRET").unwrap_or_default();
    if session_secret.is_empty() {
        tracing::error!("Auth environment variables not configured");
        return LoginPage {
            error: Some("Server misconfiguration — contact administrator".into()),
        }
        .into_response();
    }

    let store = parts.extensions.get::<std::sync::Arc<PgStore>>().cloned();
    // The app router always provides the configured throttle (durable
    // PostgreSQL/Redis). A bare router — unit tests — gets a per-request
    // in-memory throttle, which keeps the decision deterministic without
    // failing open or leaking state between tests.
    let throttle = parts
        .extensions
        .get::<Arc<LoginThrottle>>()
        .cloned()
        .unwrap_or_else(|| Arc::new(LoginThrottle::in_memory()));

    // Throttle key: normalised login name + trusted client fingerprint. The
    // name is normalised so case/whitespace variants cannot dodge a lockout.
    let attempt_key = login_attempt_key(&form.username, &parts);

    // Evaluate before any password verification: a locked key is rejected
    // without touching the credential store.
    let pre_check = throttle.evaluate(&attempt_key, Utc::now()).await;
    if !pre_check.allowed {
        tracing::warn!(
            username = %normalize_login_name(&form.username),
            retry_after_secs = pre_check.retry_after_secs,
            admin_unlock_required = pre_check.admin_unlock_required,
            "login rejected: throttle locked"
        );
        return locked_response(&pre_check);
    }

    let Some(principal) = resolve_login(store.as_deref(), &form.username, &form.password).await
    else {
        let post_failure = throttle.record_failure(&attempt_key, Utc::now()).await;
        if post_failure.is_lockout() {
            tracing::warn!(
                username = %normalize_login_name(&form.username),
                retry_after_secs = post_failure.retry_after_secs,
                admin_unlock_required = post_failure.admin_unlock_required,
                "login locked after failed attempt"
            );
            return locked_response(&post_failure);
        }
        return LoginPage {
            error: Some("Invalid credentials".into()),
        }
        .into_response();
    };

    throttle.record_success(&attempt_key).await;

    // The session length preference is user-private data read under the
    // canonical user id, not the login name.
    let session_ttl_ms = match &store {
        Some(store) => store
            .get_user_settings_prefs(&principal.user_id)
            .await
            .ok()
            .flatten()
            .map(|prefs| session_ttl_ms_for_hours(prefs.session_timeout_hours))
            .unwrap_or(SESSION_TTL_MS),
        None => SESSION_TTL_MS,
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let claims = SessionClaims {
        user_id: principal.user_id,
        username: principal.username,
        role: principal.role,
        issued_at: now_ms,
        expires_at: now_ms + session_ttl_ms,
        session_version: principal.session_version,
    };

    let Some(token) = create_session_token(&claims, &session_secret) else {
        tracing::error!("failed to serialize session payload");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response();
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    if let Ok(cookie) = HeaderValue::from_str(&session_cookie_header(&token, session_ttl_ms / 1000))
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    (StatusCode::SEE_OTHER, headers).into_response()
}

/// Build the throttle key for one login attempt.
///
/// The key is the normalised login name plus a fingerprint of the trusted
/// client address only:
///   * the TCP peer address from `ConnectInfo` is trusted;
///   * `X-Forwarded-For` is only honored under `API_TRUST_PROXY=1`, where the
///     deployment's reverse proxy overwrites it (same contract as the API
///     rate limiter, B298);
///   * the User-Agent is deliberately excluded: a client controls that header
///     and could otherwise rotate it to land in a fresh lockout bucket;
///   * the hash keeps the durable key fixed-size regardless of input size.
fn login_attempt_key(username: &str, parts: &axum::http::request::Parts) -> String {
    let peer_ip = parts
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.ip().to_string());
    let trusted_ip = if std::env::var("API_TRUST_PROXY")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        parts
            .headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or(peer_ip)
    } else {
        peer_ip
    };
    format!(
        "{}|{}",
        normalize_login_name(username),
        client_fingerprint(trusted_ip.as_deref(), None)
    )
}

/// Generic rejection for a throttled login: 429 with `Retry-After`, and a
/// message that never reveals whether the account exists or whether the
/// password was checked.
fn locked_response(status: &AuthThrottleStatus) -> Response {
    let message = if status.admin_unlock_required {
        "Too many failed sign-in attempts. This client is locked for security reasons; try again later."
    } else {
        "Too many failed sign-in attempts. Try again later."
    };
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        LoginPage {
            error: Some(message.into()),
        },
    )
        .into_response();
    // Admin locks are bounded too, so a known remaining time is advertised.
    if status.retry_after_secs > 0 {
        if let Ok(value) = HeaderValue::from_str(&status.retry_after_secs.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

/// POST /logout — clear cookie(s), redirect to login.
pub async fn logout() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_static("/login"));
    for cookie in clear_session_cookie_headers() {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            headers.append(header::SET_COOKIE, value);
        }
    }
    (StatusCode::SEE_OTHER, headers).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_sha256_hex(password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    #[test]
    fn argon2id_hash_verifies() {
        let hash = hash_password("correct horse battery staple").expect("hash password");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password_hash("correct horse battery staple", &hash));
    }

    #[test]
    fn argon2id_hash_rejects_wrong_password() {
        let hash = hash_password("right-password").expect("hash password");
        assert!(!verify_password_hash("wrong-password", &hash));
    }

    #[test]
    fn legacy_sha256_hash_still_verifies() {
        let hash = legacy_sha256_hex("legacy-password");
        assert!(verify_password_hash("legacy-password", &hash));
        assert!(!verify_password_hash("other-password", &hash));
    }

    #[test]
    fn unsupported_hash_formats_are_rejected() {
        assert!(!verify_password_hash("pw", ""));
        assert!(!verify_password_hash("pw", "not-a-hash"));
        assert!(!verify_password_hash(
            "pw",
            "$argon2i$v=19$m=16,t=2,p=1$c2FsdA$aGFzaA"
        ));
        // 64 chars, but not hex.
        assert!(!verify_password_hash("pw", &"z".repeat(64)));
    }

    fn web_user(username: &str, role: &str) -> WebUser {
        WebUser {
            id: String::new(),
            username: username.to_string(),
            password_hash: "hash".to_string(),
            role: role.to_string(),
        }
    }

    #[test]
    fn api_role_rejects_missing_and_unknown_roles() {
        assert_eq!(
            web_user("alice", "admin").api_role().unwrap(),
            ApiRole::Admin
        );
        assert!(web_user("alice", "").api_role().is_err());
        assert!(web_user("alice", "superuser").api_role().is_err());
        assert!(web_user("alice", "ANALYST").api_role().is_err());
    }

    #[test]
    fn validate_web_users_rejects_unknown_roles() {
        let users = vec![web_user("alice", "admin"), web_user("bob", "superuser")];
        let error = validate_web_users(&users).expect_err("unknown role must be rejected");
        assert!(error.contains("superuser"), "unexpected error: {error}");
    }

    #[test]
    fn validate_web_users_rejects_duplicate_usernames_case_insensitively() {
        let users = vec![web_user("Alice", "admin"), web_user(" alice ", "viewer")];
        let error = validate_web_users(&users).expect_err("duplicate login name must be rejected");
        assert!(
            error.contains("duplicate username"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_web_users_rejects_duplicate_ids() {
        let mut first = web_user("alice", "admin");
        first.id = "usr-1".to_string();
        let mut second = web_user("bob", "viewer");
        second.id = "usr-1".to_string();
        let error =
            validate_web_users(&[first, second]).expect_err("duplicate id must be rejected");
        assert!(error.contains("duplicate id"), "unexpected error: {error}");
    }

    #[test]
    fn load_web_users_rejects_duplicate_and_unknown_role_configuration() {
        // Startup fail-closed: a malformed or ambiguous environment list is
        // never trusted, so none of it can bootstrap credentials.
        let duplicates = serde_json::json!([
            {"id": "u1", "username": "Alice", "password_hash": "h1", "role": "admin"},
            {"id": "u2", "username": " alice ", "password_hash": "h2", "role": "viewer"},
        ]);
        std::env::set_var("WEB_USERS_JSON", duplicates.to_string());
        assert!(
            load_web_users().is_empty(),
            "duplicate login names must reject the whole configuration"
        );

        let duplicate_ids = serde_json::json!([
            {"id": "same", "username": "alice", "password_hash": "h1", "role": "admin"},
            {"id": "same", "username": "bob", "password_hash": "h2", "role": "viewer"},
        ]);
        std::env::set_var("WEB_USERS_JSON", duplicate_ids.to_string());
        assert!(
            load_web_users().is_empty(),
            "duplicate ids must reject the whole configuration"
        );

        let unknown_role = serde_json::json!([
            {"id": "u1", "username": "alice", "password_hash": "h1", "role": "superuser"},
        ]);
        std::env::set_var("WEB_USERS_JSON", unknown_role.to_string());
        assert!(
            load_web_users().is_empty(),
            "an unknown role must reject the whole configuration"
        );

        // A padded configured name is accepted but bootstrapped trimmed, so
        // the stored name always matches the normalized lookup.
        let padded = serde_json::json!([
            {"id": "u1", "username": " alice ", "password_hash": "h1", "role": "admin"},
        ]);
        std::env::set_var("WEB_USERS_JSON", padded.to_string());
        let seeds = bootstrap_seeds();
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].username, "alice");

        std::env::remove_var("WEB_USERS_JSON");
    }

    #[test]
    fn validate_web_users_accepts_distinct_valid_entries() {
        let mut admin = web_user("Alice", "admin");
        admin.id = "usr-1".to_string();
        let mut viewer = web_user("bob", "viewer");
        viewer.id = "usr-2".to_string();
        assert!(validate_web_users(&[admin, viewer]).is_ok());
    }

    #[test]
    fn normalize_login_name_trims_and_case_folds() {
        assert_eq!(normalize_login_name("  Alice  "), "alice");
        assert_eq!(normalize_login_name("BOB"), "bob");
    }

    fn request_parts(
        user_agent: Option<&str>,
        peer: Option<std::net::SocketAddr>,
    ) -> axum::http::request::Parts {
        let mut builder = axum::http::Request::builder().method("POST").uri("/login");
        if let Some(user_agent) = user_agent {
            builder = builder.header(header::USER_AGENT, user_agent);
        }
        let (mut parts, _) = builder.body(()).expect("request").into_parts();
        if let Some(peer) = peer {
            parts.extensions.insert(axum::extract::ConnectInfo(peer));
        }
        parts
    }

    #[test]
    fn throttle_key_normalizes_username_and_trusts_only_the_client_address() {
        let parts = request_parts(Some("test-agent"), None);
        let upper = login_attempt_key(" Alice ", &parts);
        let lower = login_attempt_key("alice", &parts);
        assert_eq!(upper, lower, "case/whitespace variants share one bucket");

        // The User-Agent is client-controlled: rotating it must NOT give a new
        // lockout bucket.
        let other_agent = request_parts(Some("other-agent"), None);
        assert_eq!(
            login_attempt_key("alice", &parts),
            login_attempt_key("alice", &other_agent),
            "rotating the User-Agent must not reset the lockout bucket"
        );

        let peer = std::net::SocketAddr::from(([127, 0, 0, 1], 4040));
        let with_peer = request_parts(Some("test-agent"), Some(peer));
        assert_ne!(
            login_attempt_key("alice", &parts),
            login_attempt_key("alice", &with_peer),
            "the trusted peer address is part of the fingerprint"
        );
    }

    #[test]
    fn locked_response_is_generic_429_with_retry_after() {
        let temporary = locked_response(&AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 5,
            failure_count_10m: 10,
            failure_count_1h: 10,
            admin_unlock_required: false,
        });
        assert_eq!(temporary.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(temporary.headers().get(header::RETRY_AFTER).unwrap(), "5");

        // The strongest lock is bounded, so its remaining time is advertised.
        let admin = locked_response(&AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 86_400,
            failure_count_10m: 10,
            failure_count_1h: 20,
            admin_unlock_required: true,
        });
        assert_eq!(admin.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(admin.headers().get(header::RETRY_AFTER).unwrap(), "86400");

        // State without a known deadline stays generic with no header.
        let undated = locked_response(&AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 0,
            failure_count_10m: 10,
            failure_count_1h: 20,
            admin_unlock_required: true,
        });
        assert!(undated.headers().get(header::RETRY_AFTER).is_none());
    }
}
