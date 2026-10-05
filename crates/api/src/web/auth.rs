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
    clear_session_cookie_headers, create_session_token, current_session_secret, same_site_post,
    session_cookie_header, session_ttl_ms_for_hours, SessionClaims, SESSION_TTL_MS,
    SESSION_VERSION,
};

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginPage {
    error: Option<String>,
    next: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    #[serde(default)]
    next: Option<String>,
}

/// Only local paths are accepted as a post-login destination: `//host`,
/// `/\host` and control-character obfuscation (`"/\t/evil"` is normalized by
/// browsers to `"//evil"`, a protocol-relative external URL) all leave the
/// site. Reuses [`crate::web::safe_relative_href`], whose control-character
/// rejection also keeps `Redirect::to` from panicking on a non-header value.
fn safe_next(next: Option<&str>) -> Option<String> {
    crate::web::safe_relative_href(next?)
}

/// A configured web login. `WEB_USERS_JSON` entries carry `id`, `username`,
/// `password_hash` and `role`; `id` defaults to the username when omitted.
#[derive(Clone, Deserialize)]
pub struct WebUser {
    #[serde(default)]
    pub id: String,
    pub username: String,
    pub password_hash: String,
    #[serde(default)]
    pub role: String,
}

impl std::fmt::Debug for WebUser {
    /// Redacts the credential: `Debug` output of configuration must never
    /// print a password hash.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebUser")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("password_hash", &"[redacted]")
            .field("role", &self.role)
            .finish()
    }
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
/// True when `hash` is a credential format the verifier can actually accept.
///
/// A malformed hash (typically `$argon2id$…` mangled by shell/dotenv expansion)
/// would seed an account that can never authenticate, so it is rejected at
/// load time rather than disclosed only at the first failed login.
fn validate_password_hash(hash: &str) -> Result<(), String> {
    let hash = hash.trim();
    if hash.starts_with("$argon2id$") {
        use argon2::password_hash::PasswordHash;
        return PasswordHash::new(hash)
            .map(|_| ())
            .map_err(|error| format!("password_hash is not a valid Argon2id PHC string: {error}"));
    }
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        // Structurally valid legacy digest: loading is allowed so one legacy
        // account cannot disable every other account. Authentication still
        // rejects it unless ALLOW_LEGACY_PASSWORD_HASHES is set (see
        // verify_password), which is where the audit requirement lives.
        return Ok(());
    }
    Err(
        "password_hash must be an Argon2id PHC string or a 64-character SHA-256 hex digest"
            .to_string(),
    )
}

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
        // A hash that is not a well-formed PHC string (typically a shell-mangled
        // value where `$argon2id$...` was expanded) would seed a credential
        // that can never authenticate. Reject it at load time instead of
        // silently locking the account out.
        validate_password_hash(&user.password_hash)
            .map_err(|error| format!("entry {index} ('{}'): {error}", user.username))?;
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
    if let Err(error) = validate_password_hash(&password_hash) {
        tracing::error!(
            %error,
            "APEX_ADMIN_PASSWORD_HASH rejected; no admin web user loaded from the environment"
        );
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
///
/// Every rejection path that does not verify a real hash pays the
/// [`password_verification_equalizer`] cost instead, so response timing does
/// not distinguish unknown, disabled, hashless and wrong-password logins.
pub async fn resolve_login(
    store: Option<&PgStore>,
    username: &str,
    password: &str,
) -> Option<LoginPrincipal> {
    let normalized = normalize_login_name(username);

    let Some(store) = store else {
        let Some(user) = load_web_users()
            .into_iter()
            .find(|user| normalize_login_name(&user.username) == normalized)
        else {
            password_verification_equalizer(password).await;
            return None;
        };
        let role = match user.api_role() {
            Ok(role) => role,
            Err(error) => {
                tracing::error!(
                    username = %user.username,
                    %error,
                    "login rejected: unknown role in WEB_USERS_JSON"
                );
                password_verification_equalizer(password).await;
                return None;
            }
        };
        if !verify_password_async(password, &user.password_hash).await {
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

    let record = match store.find_app_user_by_username(username).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            password_verification_equalizer(password).await;
            return None;
        }
        Err(error) => {
            // A storage failure must never read as "no such user": fail closed
            // and record the real reason.
            tracing::error!(%error, "login rejected: app_users lookup failed");
            return None;
        }
    };
    if !record.enabled {
        tracing::warn!(user_id = %record.id, "login rejected: account disabled");
        password_verification_equalizer(password).await;
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
            password_verification_equalizer(password).await;
            return None;
        }
    };
    let Some(password_hash) = record.password_hash.as_deref() else {
        password_verification_equalizer(password).await;
        return None;
    };
    if !verify_password_async(password, password_hash).await {
        return None;
    }

    // The update re-checks `enabled`, so a row disabled between the lookup and
    // the credential check still fails closed.
    let record = match store.record_app_user_login(&record.id).await {
        Ok(Some(record)) => record,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, "login rejected: recording the login failed");
            return None;
        }
    };

    Some(LoginPrincipal {
        user_id: UserId::from(record.id),
        username: Username::from(record.username),
        role,
        session_version: record.session_version.max(0) as u32,
    })
}

/// Whether legacy SHA-256 password hashes are accepted.
///
/// Production must reject them by default: only an explicit
/// `ALLOW_LEGACY_PASSWORD_HASHES=true` (or `1`) enables the legacy path, so a
/// deployment cannot silently keep accepting weak hashes.
fn legacy_password_hashes_enabled() -> bool {
    legacy_password_hashes_enabled_value(
        std::env::var("ALLOW_LEGACY_PASSWORD_HASHES")
            .ok()
            .as_deref(),
    )
}

/// Pure decision function for [`legacy_password_hashes_enabled`] (testable
/// without mutating the process environment).
fn legacy_password_hashes_enabled_value(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("true") | Some("1"))
}

/// A real Argon2id hash used to equalize timing for unknown, disabled and
/// hashless users: the credential check costs the same whether or not the
/// account exists, so response time does not enumerate usernames.
static DUMMY_PASSWORD_HASH: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| hash_password("apex-timing-equalizer").unwrap_or_default());

/// Verify a password off the async runtime: Argon2 is tens of milliseconds of
/// CPU, and running it directly on a Tokio worker thread lets a login flood
/// stall every other request.
pub(crate) async fn verify_password_async(password: &str, stored_hash: &str) -> bool {
    let (password, stored_hash) = (password.to_owned(), stored_hash.to_owned());
    match tokio::task::spawn_blocking(move || verify_password_hash(&password, &stored_hash)).await {
        Ok(verified) => verified,
        Err(error) => {
            // A cancelled or panicked verification task must never
            // authenticate; report it explicitly instead of defaulting silent.
            tracing::error!(%error, "password verification task failed");
            false
        }
    }
}

/// Pay the same verification cost as a real credential check without
/// revealing whether the account exists.
async fn password_verification_equalizer(password: &str) {
    let _ = verify_password_async(password, &DUMMY_PASSWORD_HASH).await;
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
        if !legacy_password_hashes_enabled() {
            tracing::error!(
                "legacy SHA-256 password hash rejected — migrate to an Argon2id PHC hash, \
                 or set ALLOW_LEGACY_PASSWORD_HASHES=true to accept it during an upgrade"
            );
            return false;
        }
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

/// GET /login — render login page, carrying the `next` destination.
pub async fn login_page(
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    LoginPage {
        error: None,
        next: safe_next(params.get("next").map(String::as_str)),
    }
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
    // Reject cross-site form posts: a forged login would sign the victim into
    // the attacker's account (session fixation/login CSRF).
    if !same_site_post(&parts.headers) {
        tracing::warn!(
            username = %normalize_login_name(&form.username),
            "login rejected: cross-site form submission"
        );
        return (StatusCode::FORBIDDEN, "Cross-site form submission rejected").into_response();
    }

    // Use the same cached secret as validation: reading SESSION_SECRET again
    // here could sign with a different value than the middleware verifies.
    let session_secret = current_session_secret();
    if session_secret.is_empty() {
        tracing::error!("Auth environment variables not configured");
        return LoginPage {
            error: Some("Server misconfiguration — contact administrator".into()),
            next: safe_next(form.next.as_deref()),
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

    // Atomically reserve the attempt *before* password verification. A
    // check-then-record pair let N concurrent requests all pass the check
    // before any failure was recorded, so one burst got N guesses. A locked
    // key is rejected here, and an allowed reservation already counts the
    // attempt (cleared by record_success below).
    let reservation = throttle.reserve(&attempt_key, Utc::now()).await;
    if !reservation.allowed {
        tracing::warn!(
            username = %normalize_login_name(&form.username),
            retry_after_secs = reservation.retry_after_secs,
            admin_unlock_required = reservation.admin_unlock_required,
            "login rejected: throttle locked"
        );
        return locked_response(&reservation);
    }

    let Some(principal) = resolve_login(store.as_deref(), &form.username, &form.password).await
    else {
        // The failure was already recorded by the reservation; answer 429 when
        // this attempt tripped a lock, otherwise the generic page.
        if reservation.is_lockout() {
            tracing::warn!(
                username = %normalize_login_name(&form.username),
                retry_after_secs = reservation.retry_after_secs,
                admin_unlock_required = reservation.admin_unlock_required,
                "login locked after failed attempt"
            );
            return locked_response(&reservation);
        }
        return LoginPage {
            error: Some("Invalid credentials".into()),
            next: safe_next(form.next.as_deref()),
        }
        .into_response();
    };

    throttle.record_success(&attempt_key).await;

    // The session length preference is user-private data read under the
    // canonical user id, not the login name.
    let session_ttl_ms = match &store {
        Some(store) => match store.get_user_settings_prefs(&principal.user_id).await {
            Ok(Some(prefs)) => session_ttl_ms_for_hours(prefs.session_timeout_hours),
            Ok(None) => SESSION_TTL_MS,
            Err(error) => {
                // A failed preference read must not silently look like "user
                // has no preference": log it and fall back to the default TTL.
                tracing::warn!(
                    user_id = %principal.user_id,
                    error = %error,
                    "failed to load session timeout preference; using default TTL"
                );
                SESSION_TTL_MS
            }
        },
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
        // Revocation id: logout records it in `revoked_sessions`.
        session_id: uuid::Uuid::new_v4(),
    };

    let Some(token) = create_session_token(&claims, session_secret) else {
        tracing::error!("failed to serialize session payload");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response();
    };

    let destination = safe_next(form.next.as_deref()).unwrap_or_else(|| "/".to_string());
    let mut headers = HeaderMap::new();
    if let Ok(location) = HeaderValue::from_str(&destination) {
        headers.insert(header::LOCATION, location);
    } else {
        headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    }
    if let Ok(cookie) = HeaderValue::from_str(&session_cookie_header(&token, session_ttl_ms / 1000))
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    (StatusCode::SEE_OTHER, headers).into_response()
}

/// Build the throttle key for one login attempt.
///
/// The key is the normalised login name plus a fingerprint of the trusted
/// client address only (see [`crate::middleware::client_ip`] — under
/// `API_TRUST_PROXY=1` the proxy-appended rightmost `X-Forwarded-For` entry
/// wins, so a client cannot rotate a spoofed leftmost value into a fresh
/// lockout bucket):
///   * the TCP peer address from `ConnectInfo` is trusted;
///   * the User-Agent is deliberately excluded: a client controls that header
///     and could otherwise rotate it to land in a fresh lockout bucket;
///   * the hash keeps the durable key fixed-size regardless of input size.
fn login_attempt_key(username: &str, parts: &axum::http::request::Parts) -> String {
    let peer_ip = parts
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.ip());
    // IPv6 is bucketed by /64 so a rotating prefix cannot win fresh lockout
    // buckets (see `rate_limit_identity`).
    let trusted_ip = crate::middleware::client_ip::client_ip(&parts.headers, peer_ip)
        .map(crate::middleware::client_ip::rate_limit_identity);
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
            next: None,
        },
    )
        .into_response();
    // A 429 always advertises a positive Retry-After: the reserved attempt
    // that tripped the lock reports retry 0 (it was allowed), so floor at 1.
    let retry_after = status.retry_after_secs.max(1);
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

/// POST /logout — revoke the session id, clear cookie(s), redirect to login.
///
/// Clearing the cookie alone left a copied cookie valid until its signed
/// expiry (up to 168h); the `jti` is recorded as revoked so the authority
/// rejects it on the next request.
pub async fn logout(parts: axum::http::request::Parts) -> Response {
    if !same_site_post(&parts.headers) {
        return (StatusCode::FORBIDDEN, "Cross-site form submission rejected").into_response();
    }

    if let Some(session) =
        crate::middleware::session::validate_session(&parts.headers, current_session_secret())
    {
        if let Some(store) = parts.extensions.get::<Arc<PgStore>>() {
            let expires_at = chrono::DateTime::<Utc>::from_timestamp_millis(session.expires_at)
                .unwrap_or_else(Utc::now);
            if let Err(error) = store.revoke_session(session.session_id, expires_at).await {
                tracing::warn!(%error, session_id = %session.session_id, "logout revocation write failed");
            }
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_static("/login"));
    // Purge the service-worker HTTP cache on logout so cached sensitive
    // responses (PDFs, reports) do not survive a session on a shared machine.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::HeaderName::from_static("clear-site-data"),
        HeaderValue::from_static("\"cache\""),
    );
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
    fn safe_next_allows_only_local_paths() {
        assert_eq!(
            safe_next(Some("/warnings/42")).as_deref(),
            Some("/warnings/42")
        );
        assert_eq!(
            safe_next(Some(" /search?q=x ")).as_deref(),
            Some("/search?q=x")
        );
        assert!(safe_next(None).is_none());
        assert!(safe_next(Some("")).is_none());
        assert!(safe_next(Some("https://evil.example")).is_none());
        assert!(safe_next(Some("//evil.example")).is_none());
        assert!(safe_next(Some("/\\evil.example")).is_none());
    }

    #[test]
    fn safe_next_rejects_control_character_open_redirect_obfuscation() {
        // Browsers strip tab/newline before parsing a URL, so `"/\t/evil"`
        // becomes the protocol-relative `"//evil"` and leaves the origin.
        assert!(safe_next(Some("/\t/evil")).is_none());
        assert!(safe_next(Some("/\n/evil")).is_none());
        assert!(safe_next(Some("/\r/evil")).is_none());
        assert!(safe_next(Some("/\u{000B}/evil")).is_none());
        assert!(safe_next(Some("/evil\r\nSet-Cookie: x=1")).is_none());
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
    fn legacy_sha256_hashes_are_rejected_by_default_and_require_explicit_opt_in() {
        std::env::remove_var("ALLOW_LEGACY_PASSWORD_HASHES");
        let hash = legacy_sha256_hex("legacy-password");
        assert!(
            !verify_password_hash("legacy-password", &hash),
            "SHA-256 must be rejected unless ALLOW_LEGACY_PASSWORD_HASHES is set"
        );

        std::env::set_var("ALLOW_LEGACY_PASSWORD_HASHES", "true");
        assert!(verify_password_hash("legacy-password", &hash));
        assert!(!verify_password_hash("other-password", &hash));
        std::env::remove_var("ALLOW_LEGACY_PASSWORD_HASHES");
    }

    #[test]
    fn legacy_hash_flag_parsing_is_strict() {
        assert!(!legacy_password_hashes_enabled_value(None));
        assert!(!legacy_password_hashes_enabled_value(Some("false")));
        assert!(!legacy_password_hashes_enabled_value(Some("yes")));
        assert!(!legacy_password_hashes_enabled_value(Some("")));
        assert!(legacy_password_hashes_enabled_value(Some("true")));
        assert!(legacy_password_hashes_enabled_value(Some(" 1 ")));
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

    /// A real Argon2id hash; cheap enough to compute once per test process and
    /// required now that bootstrap rejects malformed credential material.
    fn valid_hash() -> String {
        static HASH: std::sync::LazyLock<String> =
            std::sync::LazyLock::new(|| hash_password("test-password").expect("hash password"));
        HASH.clone()
    }

    fn web_user(username: &str, role: &str) -> WebUser {
        WebUser {
            id: String::new(),
            username: username.to_string(),
            password_hash: valid_hash(),
            role: role.to_string(),
        }
    }

    #[test]
    fn malformed_password_hashes_are_rejected_at_bootstrap() {
        // A shell-mangled `$argon2id$...` value must not seed a credential no
        // login can ever satisfy.
        let malformed = WebUser {
            id: String::new(),
            username: "alice".to_string(),
            password_hash: "=19=19456,t=2,p=1+IXfXXJoWM".to_string(),
            role: "admin".to_string(),
        };
        let error = validate_web_users(&[malformed]).expect_err("malformed hash must be rejected");
        assert!(error.contains("Argon2id"), "unexpected error: {error}");
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
            {"id": "u1", "username": " alice ", "password_hash": valid_hash(), "role": "admin"},
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

        // State without a known deadline still advertises a positive floor:
        // a 429 must never tell the client to retry immediately (0).
        let undated = locked_response(&AuthThrottleStatus {
            allowed: false,
            retry_after_secs: 0,
            failure_count_10m: 10,
            failure_count_1h: 20,
            admin_unlock_required: true,
        });
        assert_eq!(undated.headers().get(header::RETRY_AFTER).unwrap(), "1");
    }
}
