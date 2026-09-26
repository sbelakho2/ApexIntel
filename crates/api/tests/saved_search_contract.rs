//! Contract tests for the saved-search surface.
//!
//! The bug this guards against: `routes::all_endpoints()` advertised
//! `SAVED_SEARCHES` / `SAVED_SEARCH_DETAIL` (GET/POST/PUT/DELETE) and the
//! OpenAPI document published them, but `app_router.rs` registered no handlers,
//! so every call 404'd — and `saved_searches` (FORCE RLS, migration 057) had
//! no identity-scoped call path. These tests pin the route registry, the store
//! scoping, the server-rendered UI wiring, and the endpoint count.

use apex_api::routes::{all_endpoints, openapi_spec, paths, HttpMethod};

const APP_ROUTER_RS: &str = include_str!("../src/app_router.rs");
const COLLABORATION_HANDLERS_RS: &str = include_str!("../src/api_handlers/collaboration.rs");
const COLLABORATION_STORE_RS: &str = include_str!("../../store/src/postgres/collaboration.rs");
const SEARCH_WEB_RS: &str = include_str!("../src/web/search.rs");
const SEARCH_HTML: &str = include_str!("../templates/pages/search.html");

#[test]
fn saved_search_routes_are_registered_with_all_four_methods() {
    assert!(
        APP_ROUTER_RS.contains("\"/api/saved-searches\"")
            && APP_ROUTER_RS.contains("\"/api/saved-searches/:id\""),
        "the saved-search paths must be registered in app_router.rs"
    );
    for handler in [
        "list_saved_searches",
        "create_saved_search",
        "update_saved_search",
        "delete_saved_search",
    ] {
        assert!(
            APP_ROUTER_RS.contains(handler),
            "app_router.rs must wire {handler}"
        );
    }
}

#[test]
fn saved_search_endpoints_are_in_the_catalog_and_openapi() {
    let catalog: Vec<_> = all_endpoints()
        .into_iter()
        .filter(|endpoint| {
            endpoint.path == paths::SAVED_SEARCHES || endpoint.path == paths::SAVED_SEARCH_DETAIL
        })
        .collect();
    assert_eq!(
        catalog.len(),
        4,
        "GET, POST, PUT and DELETE must be catalogued"
    );

    let mut methods: Vec<String> = catalog
        .iter()
        .map(|endpoint| endpoint.method.label().to_string())
        .collect();
    methods.sort();
    assert_eq!(methods, vec!["DELETE", "GET", "POST", "PUT"]);

    for endpoint in &catalog {
        assert!(endpoint.auth_required);
        let expected_role = if endpoint.method == HttpMethod::Get {
            "viewer"
        } else {
            "analyst"
        };
        assert_eq!(endpoint.min_role, expected_role);
    }

    let spec = openapi_spec();
    assert!(spec["paths"][paths::SAVED_SEARCHES]["get"].is_object());
    assert!(spec["paths"][paths::SAVED_SEARCHES]["post"].is_object());
    assert!(spec["paths"][paths::SAVED_SEARCH_DETAIL]["put"].is_object());
    assert!(spec["paths"][paths::SAVED_SEARCH_DETAIL]["delete"].is_object());
}

#[test]
fn saved_search_handlers_are_identity_scoped() {
    for scoped in [
        "list_saved_searches_scoped",
        "upsert_saved_search_scoped",
        "update_saved_search_scoped",
        "delete_saved_search_scoped",
    ] {
        assert!(
            COLLABORATION_HANDLERS_RS.contains(scoped),
            "handlers must use {scoped} so RLS identity is set"
        );
    }
    for scoped in [
        "pub async fn upsert_saved_search_scoped",
        "pub async fn update_saved_search_scoped",
        "pub async fn list_saved_searches_scoped",
        "pub async fn delete_saved_search_scoped",
    ] {
        assert!(
            COLLABORATION_STORE_RS.contains(scoped),
            "store must expose {scoped} via PgStore::begin_scoped"
        );
    }
}

#[test]
fn saved_search_ui_is_part_of_the_server_rendered_page() {
    assert!(
        APP_ROUTER_RS.contains("\"/search/saved-searches\"")
            && APP_ROUTER_RS.contains("\"/search/saved-searches/:id/delete\""),
        "the server-rendered saved-search form targets must be registered"
    );
    assert!(SEARCH_WEB_RS.contains("save_search"));
    assert!(SEARCH_WEB_RS.contains("delete_saved_search"));
    assert!(SEARCH_HTML.contains("Saved searches"));
    assert!(SEARCH_HTML.contains("action=\"/search/saved-searches\""));
}

#[test]
fn endpoint_count_matches_the_catalogued_surface() {
    // Verified after wiring the saved-search handlers; the catalogue already
    // listed them, so the exposed count is unchanged at 102.
    assert_eq!(all_endpoints().len(), 102);
}
