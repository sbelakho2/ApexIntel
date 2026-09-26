#![allow(clippy::unwrap_used, clippy::expect_used)]

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
    assert!(
        paths.contains_key("/api/saved-searches") && paths.contains_key("/api/saved-searches/:id"),
        "the served OpenAPI document must include the saved-search surface"
    );
}

/// The served OpenAPI document is generated from the endpoint catalogue, so
/// every catalogued route (including the saved-search CRUD) is published and
/// the verified count cannot drift from the served contract.
#[tokio::test]
async fn served_openapi_covers_the_verified_endpoint_catalogue() {
    let (app, _) = support::build_test_router();

    let response = app
        .oneshot(support::request("GET", "/api/openapi.json"))
        .await
        .expect("openapi response");
    assert_eq!(response.status(), 200);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let spec: Value = serde_json::from_slice(&body).expect("openapi json");
    let paths = spec["paths"].as_object().expect("paths object");

    let catalogue = apex_api::routes::all_endpoints();
    assert_eq!(catalogue.len(), 102, "verified endpoint count");
    for endpoint in &catalogue {
        assert!(
            paths.contains_key(endpoint.path),
            "served OpenAPI is missing catalogued path {}",
            endpoint.path
        );
    }
    assert!(paths["/api/saved-searches"]["get"].is_object());
    assert!(paths["/api/saved-searches"]["post"].is_object());
    assert!(paths["/api/saved-searches/:id"]["put"].is_object());
    assert!(paths["/api/saved-searches/:id"]["delete"].is_object());
}
