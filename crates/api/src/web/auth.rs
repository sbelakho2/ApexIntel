//! Login / logout handlers for HTML pages.

use askama::Template;
use axum::{
    extract::Form,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

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

/// GET /login — render login page.
pub async fn login_page() -> impl IntoResponse {
    LoginPage { error: None }
}

/// POST /login — validate credentials, set session cookie, redirect.
pub async fn login_submit(Form(form): Form<LoginForm>) -> Response {
    let valid_username = std::env::var("APEX_ADMIN_USERNAME").unwrap_or_default();
    let valid_password_hash = std::env::var("APEX_ADMIN_PASSWORD_HASH").unwrap_or_default();
    let session_secret = std::env::var("SESSION_SECRET").unwrap_or_default();

    if valid_username.is_empty() || valid_password_hash.is_empty() || session_secret.is_empty() {
        tracing::error!("Auth environment variables not configured");
        return LoginPage {
            error: Some("Server misconfiguration — contact administrator".into()),
        }
        .into_response();
    }

    // Validate credentials
    if form.username != valid_username {
        return LoginPage {
            error: Some("Invalid credentials".into()),
        }
        .into_response();
    }

    let mut hasher = Sha256::new();
    hasher.update(form.password.as_bytes());
    let password_hash = hex::encode(hasher.finalize());

    // Constant-time comparison
    use subtle::ConstantTimeEq;
    if password_hash
        .as_bytes()
        .ct_eq(valid_password_hash.as_bytes())
        .unwrap_u8()
        != 1
    {
        return LoginPage {
            error: Some("Invalid credentials".into()),
        }
        .into_response();
    }

    // Create session token: base64url(JSON payload) + "." + HMAC-SHA256 hex
    #[derive(serde::Serialize)]
    struct SessionPayload<'a> {
        sub: &'a str,
        iat: i64,
    }

    let payload = SessionPayload {
        sub: &form.username,
        iat: chrono::Utc::now().timestamp_millis(),
    };
    let payload_bytes = serde_json::to_vec(&payload)
        .unwrap_or_else(|err| panic!("failed to serialize session payload: {err}"));

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);

    let mut mac = HmacSha256::new_from_slice(session_secret.as_bytes())
        .unwrap_or_else(|err| panic!("failed to initialize session HMAC: {err}"));
    mac.update(&payload_bytes);
    let sig = hex::encode(mac.finalize().into_bytes());

    let token = format!("{}.{}", payload_b64, sig);

    // Set cookie and redirect
    let cookie = format!(
        "apex_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400",
        token
    );

    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/"),
            (header::SET_COOKIE, cookie.as_str()),
        ],
    )
        .into_response()
}

/// POST /logout — clear cookie, redirect to login.
pub async fn logout() -> impl IntoResponse {
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/login"),
            (
                header::SET_COOKIE,
                "apex_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
            ),
        ],
    )
}
