#![allow(clippy::disallowed_methods)]

#[test]
fn request_id_is_present_in_handler_and_store_logs() {
    let warnings = include_str!("../src/api_handlers/warnings.rs");
    let insights = include_str!("../src/api_handlers/insights.rs");

    assert!(warnings.contains("request_id = %request_id"));
    assert!(warnings.contains("info_span!(\"db.count_warnings\""));
    assert!(warnings.contains("info_span!(\"db.list_warnings\""));
    assert!(insights.contains("request_id = %request_id"));
    assert!(insights.contains("info_span!(\"db.count_insights\""));
    assert!(insights.contains("info_span!(\"db.list_insights\""));
}

#[test]
fn critical_mutations_emit_consistent_span_fields() {
    let warnings = include_str!("../src/api_handlers/warnings.rs");
    let admin = include_str!("../src/api_handlers/admin.rs");

    assert!(warnings.contains("acknowledge_warning"));
    assert!(warnings.contains("request_id = %request_id"));
    assert!(admin.contains("request_id = %request_id"));
    assert!(admin.contains("manual job trigger queued"));
    assert!(admin.contains("API key auth lockout cleared"));
}
