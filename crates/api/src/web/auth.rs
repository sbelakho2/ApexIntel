//! Login / logout handlers for HTML pages.
//!
//! Password verification supports Argon2id PHC strings (`$argon2id$...`,
//! preferred) and the legacy 64-char SHA-256 hex digest (accepted with a
//! deprecation warning so existing deployments keep working).
//!
//! Users come from `WEB_USERS_JSON` (JSON array of
//! `{id, username, password_hash, role}`) when set, otherwise from the single
//! legacy `APEX_ADMIN_USERNAME` / `APEX_ADMIN_PASSWORD_HASH` pair (admin role).

use askama::Template;
use axum::{
    extract::Form,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::auth::ApiRole;
use crate::middleware::session::{
    clear_session_cookie_headers, create_session_token, session_cookie_header, SessionClaims,
    SESSION_TTL_MS, SESSION_VERSION,
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

    pub fn api_role(&self) -> ApiRole {
        self.role.parse().unwrap_or_default()
    }
}

/// Load configured web users: `WEB_USERS_JSON` when set, otherwise the single
/// `APEX_ADMIN_USERNAME` / `APEX_ADMIN_PASSWORD_HASH` admin pair. Returns an
/// empty list (with an error log) when configuration is missing or invalid.
pub fn load_web_users() -> Vec<WebUser> {
    if let Ok(raw) = std::env::var("WEB_USERS_JSON") {
        let raw = raw.trim();
        if !raw.is_empty() {
            match serde_json::from_str::<Vec<WebUser>>(raw) {
                Ok(users) if !users.is_empty() => {
                    for user in &users {
                        if user.role.is_empty() {
                            tracing::warn!(username = %user.username, "WEB_USERS_JSON entry has no role; defaulting to analyst");
                        } else if user.role.parse::<ApiRole>().is_err() {
                            tracing::warn!(username = %user.username, role = %user.role, "WEB_USERS_JSON entry has an unknown role; defaulting to analyst");
                        }
                    }
                    return users;
                }
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
pub async fn login_submit(Form(form): Form<LoginForm>) -> Response {
    let session_secret = std::env::var("SESSION_SECRET").unwrap_or_default();
    let users = load_web_users();

    if users.is_empty() || session_secret.is_empty() {
        tracing::error!("Auth environment variables not configured");
        return LoginPage {
            error: Some("Server misconfiguration — contact administrator".into()),
        }
        .into_response();
    }

    let Some(user) = users
        .iter()
        .find(|user| user.username == form.username)
        .cloned()
    else {
        return LoginPage {
            error: Some("Invalid credentials".into()),
        }
        .into_response();
    };

    if !verify_password_hash(&form.password, &user.password_hash) {
        return LoginPage {
            error: Some("Invalid credentials".into()),
        }
        .into_response();
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let claims = SessionClaims {
        user_id: user.user_id().to_string(),
        username: user.username.clone(),
        role: user.api_role(),
        issued_at: now_ms,
        expires_at: now_ms + SESSION_TTL_MS,
        session_version: SESSION_VERSION,
    };

    let Some(token) = create_session_token(&claims, &session_secret) else {
        tracing::error!("failed to serialize session payload");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response();
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    if let Ok(cookie) = HeaderValue::from_str(&session_cookie_header(&token)) {
        headers.append(header::SET_COOKIE, cookie);
    }
    (StatusCode::SEE_OTHER, headers).into_response()
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
}
