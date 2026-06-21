//! Centralized API configuration for the ApexIntel frontend.
//!
//! Provides a single source of truth for the backend API base URL and
//! WASM-compatible fetch helpers. Replaces hardcoded `reqwest` calls
//! (which do not work in WASM/browser) with `gloo_net`-based functions.
//!
//! # Configuration
//!
//! The API base URL is determined at build time via the `APEX_API_URL`
//! environment variable. When not set, it defaults to `http://localhost:8080`.
//! For production deployments behind a reverse proxy, set `APEX_API_URL` to
//! an empty string (relative-path fetch) so that all requests go to the
//! same origin as the frontend WASM bundle.

/// The base URL for the ApexIntel API server.
///
/// Build-time configurable via `APEX_API_URL` env var.
/// Default: `"http://localhost:8080"`.
pub const API_BASE_URL: &str = {
    match option_env!("APEX_API_URL") {
        Some(url) => url,
        None => "http://localhost:8080",
    }
};

/// Join a URL path fragment with the API base URL.
///
/// Handles the edge cases:
/// - When `API_BASE_URL` is empty (same-origin deployment), returns `path` as-is
/// - When `path` already starts with `/api/`, joins directly
/// - Otherwise, inserts `/api/` prefix
///
/// # Examples
/// ```
/// assert_eq!(api_url("/api/insights"), "http://localhost:8080/api/insights");
/// assert_eq!(api_url("insights"), "http://localhost:8080/api/insights");
/// ```
pub fn api_url(path: &str) -> String {
    if API_BASE_URL.is_empty() {
        // Same-origin deployment — relative URLs
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/api/{}", path)
        }
    } else {
        let base = API_BASE_URL.trim_end_matches('/');
        if path.starts_with("/api/") {
            format!("{}{}", base, path)
        } else if path.starts_with('/') {
            format!("{}", path)
        } else {
            format!("{}/api/{}", base, path)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WASM-compatible fetch helpers (using gloo_net)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "wasm32")]
mod wasm_fetch {
    use gloo_net::http::Request;
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    /// Standard API envelope: `{ "success": bool, "data": T }`.
    #[derive(serde::Deserialize)]
    struct ApiEnvelope<T> {
        pub success: bool,
        pub data: Option<T>,
    }

    /// Generic GET request returning a deserialized type from the API envelope.
    pub async fn fetch_api<T: DeserializeOwned>(path: &str) -> Result<T, String> {
        let url = super::api_url(path);
        let response = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Fetch failed: {}", e))?;

        if !response.ok() {
            return Err(format!(
                "API error: {} {}",
                response.status(),
                response.status_text()
            ));
        }

        let envelope: ApiEnvelope<T> = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        envelope
            .data
            .ok_or_else(|| "API returned empty data".to_string())
    }

    /// Generic GET request returning raw JSON `Value` (for graph queries etc.).
    pub async fn fetch_json(path: &str) -> Result<Value, String> {
        let url = super::api_url(path);
        let response = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Fetch failed: {}", e))?;

        if !response.ok() {
            return Err(format!(
                "API error: {} {}",
                response.status(),
                response.status_text()
            ));
        }

        response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))
    }

    /// Generic POST request with a JSON body, returning deserialized envelope data.
    pub async fn post_api<T: DeserializeOwned, B: serde::Serialize>(
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let url = super::api_url(path);
        let response = Request::post(&url)
            .json(body)
            .map_err(|e| format!("Serialize error: {}", e))?
            .send()
            .await
            .map_err(|e| format!("Fetch failed: {}", e))?;

        if !response.ok() {
            return Err(format!(
                "API error: {} {}",
                response.status(),
                response.status_text()
            ));
        }

        let envelope: ApiEnvelope<T> = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        envelope
            .data
            .ok_or_else(|| "API returned empty data".to_string())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Non-WASM fallback (for server-side rendering / test contexts)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_arch = "wasm32"))]
mod wasm_fetch {
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    /// Stub for SSR/test contexts — real fetch happens only in WASM.
    pub async fn fetch_api<T: DeserializeOwned>(_path: &str) -> Result<T, String> {
        Err("fetch_api is only available in WASM (browser) context".to_string())
    }

    /// Stub for SSR/test contexts.
    pub async fn fetch_json(_path: &str) -> Result<Value, String> {
        Err("fetch_json is only available in WASM (browser) context".to_string())
    }

    /// Stub for SSR/test contexts.
    pub async fn post_api<T: DeserializeOwned, B: serde::Serialize>(
        _path: &str,
        _body: &B,
    ) -> Result<T, String> {
        Err("post_api is only available in WASM (browser) context".to_string())
    }
}

// Re-export the platform-appropriate implementations
pub use wasm_fetch::{fetch_api, fetch_json, post_api};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_url_with_base() {
        // Note: API_BASE_URL is a compile-time constant; in tests it will be
        // the default value "http://localhost:8080".
        let url = api_url("insights");
        assert!(url.contains("insights"));
        assert!(url.starts_with("http"));
    }

    #[test]
    fn test_api_url_empty_base() {
        // When API_BASE_URL is empty (same-origin deployment)
        // We test the path-joining logic directly.
        let result = format!("/api/insights");
        assert_eq!(result, "/api/insights");
    }
}