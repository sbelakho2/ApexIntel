//! Contract tests for the per-user entity alert-subscription surface.
//!
//! The bug this guards against: `user_alert_subscriptions` (migration 048) was
//! written by the alert router but had no API/UI management path, and the
//! command palette's "Watch" action silently posted to `/api/queue` instead.
//! These tests pin the route registry, the palette wiring, the entity-detail
//! control, and the identity rules without needing a database.

use apex_api::auth::ApiRole;
use apex_api::destructive_actions::ApiAuthContext;
use apex_api::responses::ErrorCode;
use apex_api::routes::alert_subscriptions::{
    resolve_subscription_actor, validate_category, validate_min_severity,
};
use apex_api::routes::{all_endpoints, openapi_spec, paths, HttpMethod};

const APP_ROUTER_RS: &str = include_str!("../src/app_router.rs");
const SUBSCRIPTION_HANDLERS_RS: &str = include_str!("../src/api_handlers/alert_subscriptions.rs");
const PALETTE_JS: &str = include_str!("../static/js/command-palette.js");
const SUBSCRIPTION_JS: &str = include_str!("../static/js/alert-subscription.js");
const PALETTE_HTML: &str = include_str!("../templates/partials/command_palette.html");
const MACROS_HTML: &str = include_str!("../templates/macros.html");
const COMPANY_DETAIL_HTML: &str = include_str!("../templates/pages/company_detail.html");
const PERSON_DETAIL_HTML: &str = include_str!("../templates/pages/person_detail.html");

fn auth(user_id: &str, role: ApiRole) -> ApiAuthContext {
    ApiAuthContext {
        key_id: "test".to_string(),
        user_id: user_id.to_string(),
        role,
    }
}

#[test]
fn subscription_routes_are_registered_with_get_put_delete() {
    assert!(
        APP_ROUTER_RS.contains("\"/api/entities/:id/alert-subscription\""),
        "the entity alert-subscription route must be registered in app_router.rs"
    );
    for handler in [
        "get_entity_alert_subscription",
        "upsert_entity_alert_subscription",
        "delete_entity_alert_subscription",
    ] {
        assert!(
            APP_ROUTER_RS.contains(handler),
            "app_router.rs must wire {handler}"
        );
    }
}

#[test]
fn subscription_endpoints_are_in_the_catalog_and_openapi() {
    let catalog: Vec<_> = all_endpoints()
        .into_iter()
        .filter(|endpoint| endpoint.path == paths::ENTITY_ALERT_SUBSCRIPTION)
        .collect();
    assert_eq!(catalog.len(), 3, "GET, PUT and DELETE must be catalogued");

    let mut methods: Vec<String> = catalog
        .iter()
        .map(|e| e.method.label().to_string())
        .collect();
    methods.sort();
    assert_eq!(methods, vec!["DELETE", "GET", "PUT"]);

    let put = catalog
        .iter()
        .find(|e| e.method == HttpMethod::Put)
        .expect("PUT catalogued");
    assert_eq!(put.min_role, "analyst");
    assert!(put.auth_required);

    let spec = openapi_spec();
    let path = &spec["paths"][paths::ENTITY_ALERT_SUBSCRIPTION];
    for method in ["get", "put", "delete"] {
        assert!(
            path[method].is_object(),
            "openapi must document {method} on the subscription path"
        );
    }
}

#[test]
fn subscription_identity_is_the_authenticated_principal() {
    // A normal user is pinned to its own identity, with or without an override.
    assert_eq!(
        resolve_subscription_actor(&auth("alice", ApiRole::Analyst), None).unwrap(),
        "alice"
    );
    assert_eq!(
        resolve_subscription_actor(&auth("alice", ApiRole::Analyst), Some("alice")).unwrap(),
        "alice"
    );

    // Impersonation is rejected for non-privileged roles ...
    for role in [ApiRole::Analyst, ApiRole::Viewer] {
        let error = resolve_subscription_actor(&auth("alice", role), Some("bob")).unwrap_err();
        assert_eq!(error.code, ErrorCode::Forbidden);
    }

    // ... and allowed for admin/service principals only.
    for role in [ApiRole::Admin, ApiRole::Service] {
        assert_eq!(
            resolve_subscription_actor(&auth("ops", role), Some("bob")).unwrap(),
            "bob"
        );
    }
}

#[test]
fn subscription_input_validation_rejects_bad_severity_and_category() {
    assert_eq!(validate_min_severity(" Critical ").unwrap(), "critical");
    assert!(validate_min_severity("urgent").is_err());
    assert_eq!(
        validate_category(Some(" Warning ")).unwrap(),
        Some("warning".to_string())
    );
    assert_eq!(validate_category(Some("")).unwrap(), None);
    assert!(validate_category(Some(&"c".repeat(65))).is_err());
}

#[test]
fn palette_watch_action_targets_the_subscription_endpoint() {
    let watch_branch = PALETTE_JS
        .split("if (action === \"watch\")")
        .nth(1)
        .expect("palette has a watch action branch");
    let watch_branch = watch_branch
        .split("if (action ===")
        .next()
        .expect("watch branch terminator");

    assert!(
        watch_branch.contains("/api/entities/") && watch_branch.contains("/alert-subscription"),
        "the palette Watch action must call the subscription endpoint, got: {watch_branch}"
    );
    assert!(
        !watch_branch.contains("postJson(\"/api/queue\""),
        "the palette Watch action must no longer post to the work queue"
    );
    assert!(
        watch_branch.contains("putJson"),
        "the subscription upsert must be a PUT"
    );

    // The palette still exposes a distinct label and distinguishes the actions.
    assert!(PALETTE_JS.contains("label: \"Watch alerts\""));
    assert!(PALETTE_HTML.contains("Watch alerts only opts into notifications"));
    assert!(PALETTE_HTML.contains("Add to work queue"));
}

#[test]
fn all_three_verbs_resolve_the_authenticated_principal() {
    // Each handler must route its identity choice through the shared resolver;
    // the middlewares authenticate, the resolver enforces the admin/service
    // override rule. GET/DELETE take the override from the query string, PUT
    // accepts it from the body (falling back to the query string).
    let resolver_calls = SUBSCRIPTION_HANDLERS_RS
        .matches("resolve_subscription_actor")
        .count();
    assert!(
        resolver_calls >= 3,
        "GET, PUT and DELETE must each resolve the acting principal, found {resolver_calls}"
    );
    assert!(
        SUBSCRIPTION_HANDLERS_RS.contains("query.user_id.as_deref()"),
        "GET/DELETE must support the admin/service user_id override"
    );
    assert!(
        SUBSCRIPTION_HANDLERS_RS.contains("body.user_id.as_deref()"),
        "PUT must support the admin/service user_id override"
    );
}

#[test]
fn entity_detail_pages_include_the_watch_alerts_control() {
    assert!(
        MACROS_HTML.contains("macro watch_alerts_control"),
        "the shared macros file must define the Watch alerts control"
    );
    assert!(MACROS_HTML.contains("data-alert-subscription"));
    for (template, name) in [
        (COMPANY_DETAIL_HTML, "company_detail.html"),
        (PERSON_DETAIL_HTML, "person_detail.html"),
    ] {
        assert!(
            template.contains("m::watch_alerts_control"),
            "{name} must render the Watch alerts control"
        );
        assert!(
            template.contains("alert-subscription.js"),
            "{name} must load the Watch alerts script"
        );
    }

    // The UI copy must be explicit that the four entity actions differ.
    for label in [
        "Add to work queue",
        "Investigate",
        "Add to pipeline",
        "separate from",
    ] {
        assert!(
            MACROS_HTML.contains(label),
            "Watch alerts copy must distinguish {label}"
        );
    }
}

#[test]
fn subscription_widget_drives_the_three_verbs() {
    assert!(SUBSCRIPTION_JS.contains("\"/api/entities/\""));
    assert!(SUBSCRIPTION_JS.contains("\"/alert-subscription\""));
    assert!(SUBSCRIPTION_JS.contains("\"PUT\""));
    assert!(SUBSCRIPTION_JS.contains("\"DELETE\""));
    assert!(
        SUBSCRIPTION_JS.contains("?category="),
        "DELETE must target the category-scoped natural key"
    );
}
