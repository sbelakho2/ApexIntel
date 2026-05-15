#![allow(clippy::disallowed_methods)]

mod support;

use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn frontend_api_contract_smoke_uses_current_base_paths() {
    let (app, _) = support::build_test_router();

    let health = app
        .clone()
        .oneshot(support::request("GET", "/api/health"))
        .await
        .expect("health response");
    assert_eq!(health.status(), 200);

    let features = app
        .clone()
        .oneshot(support::request("GET", "/api/features"))
        .await
        .expect("features response");
    assert_eq!(features.status(), 200);

    let openapi = app
        .oneshot(support::request("GET", "/api/openapi.json"))
        .await
        .expect("openapi response");
    assert_eq!(openapi.status(), 200);

    let body = openapi
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let spec: Value = serde_json::from_slice(&body).expect("openapi json");
    let paths = spec["paths"].as_object().expect("paths object");

    assert!(paths.contains_key("/api/health"));
    assert!(paths.contains_key("/api/warnings"));
    assert!(paths.contains_key("/api/openapi.json"));
    assert!(paths.contains_key("/api/features"));
}
