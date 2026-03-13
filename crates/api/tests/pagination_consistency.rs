#![allow(clippy::disallowed_methods)]

mod support;

use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn clamp_page_does_not_return_empty_page_when_prior_page_exists() {
    let (app, store) = support::build_test_router();
    store.set_warning_count_sequence(vec![2, 1]);

    let mut request = support::request("GET", "/api/warnings?page=2&per_page=1");
    request.headers_mut().insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);

    let body = response.into_body().collect().await.expect("body").to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(payload["data"]["page"], 1);
    assert_eq!(payload["data"]["items"].as_array().expect("items").len(), 1);
}

#[tokio::test]
async fn warnings_pagination_total_and_items_are_consistent_under_concurrent_change() {
    let (app, store) = support::build_test_router();
    store.set_warning_count_sequence(vec![26, 1]);

    let mut request = support::request("GET", "/api/warnings?page=2&per_page=25");
    request.headers_mut().insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);

    let body = response.into_body().collect().await.expect("body").to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(payload["data"]["page"], 1);
    assert_eq!(payload["data"]["total"], 1);
    assert_eq!(payload["data"]["items"].as_array().expect("items").len(), 1);
}