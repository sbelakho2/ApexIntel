//! Session cookie middleware for HTML pages.
//!
//! Reads the `apex_session` cookie, verifies HMAC SHA-256 signature,
//! checks 24-hour expiry, and injects `WebSession` into request extensions.
//! Unauthenticated requests are redirected to `/login`.

use axum::{
    extract::Request,
    http::header,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use sha2::Sha256;
use hmac::{Hmac, Mac};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Session data extracted from the cookie and injected into request extensions.
#[derive(Clone, Debug)]
pub struct WebSession {
    pub username: String,
    pub issued_at: i64,
}

/// Axum middleware: require a valid session cookie, redirect to `/login` otherwise.
pub async fn require_session(
    request: Request,
    next: Next,
) -> Response {
    let session_secret = std::env::var("SESSION_SECRET").unwrap_or_default();
    if session_secret.is_empty() {
        tracing::error!("SESSION_SECRET not set — rejecting all web sessions");
        return Redirect::to("/login").into_response();
    }

    // Extract cookie
    let cookie_header = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = cookie_header
        .split(';')
        .map(|c| c.trim())
        .find(|c| c.starts_with("apex_session="))
        .map(|c| &c["apex_session=".len()..]);

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

    // Inject session into extensions
    let mut request = request;
    request.extensions_mut().insert(WebSession { username, issued_at });

    next.run(request).await
}
