//! Static-asset handlers that must work before any session exists.
//!
//! These are deliberately not part of the catalogued API surface: browsers
//! request them implicitly, so a missing handler shows up as a 404 on every
//! page rather than as a failed API call.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

const FAVICON_SVG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/static/icons/icon-192.svg"
));

/// GET /favicon.ico — browsers request this implicitly even when a
/// `<link rel="icon">` is present (older clients, cached pages, some
/// crawlers). Serving the brand SVG with the correct content type keeps every
/// page free of a failed resource load.
pub async fn favicon() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        FAVICON_SVG,
    )
        .into_response()
}
