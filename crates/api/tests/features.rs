#![allow(clippy::disallowed_methods)]

mod support;

use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn api_feature_matrix_matches_documented_llm_modes() {
    let (app, _) = support::build_test_router();
    let response = app
        .oneshot(support::request("GET", "/api/features"))
        .await
        .expect("features response");

    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("feature json");

    assert_eq!(payload["llm"], apex_api::API_LLM_FEATURE_ENABLED);
    assert_eq!(payload["experimental_llm_tool_calling"], false);
    assert_eq!(payload["openapi"], true);
    assert_eq!(payload["versioned_api_alias"], true);
}

#[tokio::test]
async fn experimental_only_capabilities_are_unavailable_without_flag() {
    let (app, _) = support::build_test_router();
    let response = app
        .oneshot(support::request("GET", "/api/features"))
        .await
        .expect("features response");

    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("feature json");

    assert_eq!(payload["experimental_llm_tool_calling"], false);
}
