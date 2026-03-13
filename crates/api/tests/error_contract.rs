#![allow(clippy::disallowed_methods)]

use apex_api::responses::{error_response, ErrorCode};
use apex_api::validation::{internal_error, parse_country_code_filter, parse_optional_uuid_filter};

#[test]
fn validation_failures_return_uniform_api_error_payload() {
    let uuid_error = error_response::<()>(
        parse_optional_uuid_filter("entity_id", Some("not-a-uuid"))
            .expect_err("invalid UUID should fail"),
    )
    .error
    .expect("uuid error");
    let country_error = error_response::<()>(
        parse_country_code_filter(Some("us")).expect_err("invalid country code should fail"),
    )
    .error
    .expect("country error");

    assert_eq!(uuid_error.code, ErrorCode::ValidationError);
    assert_eq!(country_error.code, ErrorCode::ValidationError);
}

#[test]
fn handler_internal_errors_map_to_consistent_500_response() {
    let payload = error_response::<()>(internal_error("Failed to list observations"));
    let error = payload.error.expect("error payload");
    assert_eq!(error.code, ErrorCode::InternalError);
    assert_eq!(error.message, "Failed to list observations");
}

#[test]
fn query_parsing_errors_do_not_leak_anyhow_strings() {
    let err = parse_optional_uuid_filter("entity_id", Some("not-a-uuid"))
        .expect_err("invalid UUID should fail");
    assert!(!err.message.contains("invalid length"));
    assert_eq!(err.message, "entity_id must be a valid UUID");
}