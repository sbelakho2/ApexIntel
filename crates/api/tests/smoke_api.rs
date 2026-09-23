#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn api_smoke_list_warnings_returns_200_and_pagination_metadata() {
    let (app, _) = support::build_test_router();
    let mut request = support::request("GET", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["data"]["page"], 1);
    assert!(payload["data"]["total"].as_u64().unwrap_or(0) >= 1);
    assert!(payload["data"]["items"].as_array().is_some());
}

#[tokio::test]
async fn api_smoke_list_insights_returns_200_and_items() {
    let (app, _) = support::build_test_router();
    let mut request = support::request("GET", "/api/insights");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["data"]["items"].as_array().expect("items").len(), 1);
}

#[tokio::test]
async fn api_smoke_company_detail_returns_expected_shape() {
    let (app, store) = support::build_test_router();
    let mut request = support::request(
        "GET",
        &format!("/api/companies/{}", store.seeded_company_id()),
    );
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["data"]["name"], "Acme EMS");
    assert!(payload["data"]["sites"].as_array().is_some());
    assert!(payload["data"]["key_persons"].as_array().is_some());
}

#[tokio::test]
async fn api_smoke_health_endpoints_return_expected_contract() {
    let (app, _) = support::build_test_router();

    let health_response = app
        .clone()
        .oneshot(support::request("GET", "/api/health"))
        .await
        .expect("health response");
    assert_eq!(health_response.status(), 200);
    let health_body = health_response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let health_payload: Value = serde_json::from_slice(&health_body).expect("json");
    assert!(health_payload["checks"].as_array().is_some());

    let live_response = app
        .clone()
        .oneshot(support::request("GET", "/api/health/live"))
        .await
        .expect("live response");
    assert_eq!(live_response.status(), 200);

    let ready_response = app
        .oneshot(support::request("GET", "/api/health/ready"))
        .await
        .expect("ready response");
    assert_eq!(ready_response.status(), 200);
}

#[tokio::test]
async fn api_smoke_requires_auth_for_protected_routes() {
    let (app, _) = support::build_test_router();
    let response = app
        .oneshot(support::request("GET", "/api/warnings"))
        .await
        .expect("response");

    assert_eq!(response.status(), 401);
}
