#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use axum::http::HeaderValue;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn delete_all_warnings_rejects_without_confirmation_header() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 422);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_rejects_wrong_confirmation_value() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("not-warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 422);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_rejects_without_reason() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 422);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_rejects_readonly_principal() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::viewer_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 403);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_allows_admin_principal() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);
    assert_eq!(store.delete_all_call_count(), 1);
}

#[tokio::test]
async fn delete_all_warnings_returns_422_before_store_delete_when_confirmation_missing() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 422);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_does_not_touch_store_on_forbidden_request() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::viewer_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 403);
    assert_eq!(store.delete_all_call_count(), 0);
}

#[tokio::test]
async fn delete_all_warnings_emits_audit_event_with_actor_reason_and_count() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);

    let audit_events = store.audit_events();
    assert_eq!(audit_events.len(), 1);
    let (actor, event_type, detail) = &audit_events[0];
    assert_eq!(actor, "user-admin");
    assert_eq!(event_type, "warnings_deleted_all");
    assert_eq!(detail["reason"], "cleanup stale rows");
    assert_eq!(detail["deleted_count"], 1);
}

#[tokio::test]
async fn delete_all_warnings_logs_audit_event_before_success_response() {
    let (app, store) = support::build_test_router();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);
    assert_eq!(store.audit_events().len(), 1);
}

#[tokio::test]
async fn delete_all_warnings_marks_rows_deleted_without_physical_removal() {
    let (app, store) = support::build_test_router();
    let original_len = store.warnings_snapshot().len();
    let mut request = support::request("DELETE", "/api/warnings");
    request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);

    let snapshot = store.warnings_snapshot();
    assert_eq!(snapshot.len(), original_len);
    assert!(snapshot.iter().all(|warning| warning.deleted_at.is_some()));
}

#[tokio::test]
async fn list_warnings_excludes_soft_deleted_rows_by_default() {
    let (app, _) = support::build_test_router();

    let mut delete_request = support::request("DELETE", "/api/warnings");
    delete_request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    delete_request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    delete_request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );
    app.clone()
        .oneshot(delete_request)
        .await
        .expect("delete response");

    let mut list_request = support::request("GET", "/api/warnings");
    list_request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());

    let response = app.oneshot(list_request).await.expect("response");
    assert_eq!(response.status(), 200);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["data"]["items"].as_array().expect("items").len(), 0);
}

#[tokio::test]
async fn admin_list_warnings_can_include_soft_deleted_rows() {
    let (app, _) = support::build_test_router();

    let mut delete_request = support::request("DELETE", "/api/warnings");
    delete_request
        .headers_mut()
        .insert("authorization", support::admin_auth_header());
    delete_request.headers_mut().insert(
        "x-apex-confirm-delete",
        HeaderValue::from_static("warnings"),
    );
    delete_request.headers_mut().insert(
        "x-apex-delete-reason",
        HeaderValue::from_static("cleanup stale rows"),
    );
    app.clone()
        .oneshot(delete_request)
        .await
        .expect("delete response");

    let mut request = support::request("GET", "/api/warnings?include_deleted=true");
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
    let items = payload["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert!(items[0]["deleted_at"].is_string());
}
