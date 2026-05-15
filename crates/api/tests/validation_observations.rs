#![allow(clippy::disallowed_methods)]

use apex_api::responses::{error_response, ErrorCode};
use apex_api::validation::parse_observation_type_filter;

#[test]
fn list_observations_rejects_unknown_observation_type() {
    let err = parse_observation_type_filter(Some("UnknownType"))
        .expect_err("unknown observation type should fail");
    assert_eq!(err.http_status(), 422);
}

#[test]
fn list_observations_accepts_known_observation_type() {
    let observation_type =
        parse_observation_type_filter(Some("JobPost")).expect("known observation type should pass");
    assert_eq!(observation_type.as_deref(), Some("JobPost"));
}

#[test]
fn list_observations_returns_api_error_shape_for_validation_failures() {
    let payload = error_response::<()>(
        parse_observation_type_filter(Some("bad-value")).expect_err("validation error"),
    );
    let error = payload.error.expect("error payload");
    assert_eq!(error.code, ErrorCode::ValidationError);
    assert_eq!(
        error.details.expect("details").get("field"),
        Some(&"observation_type".to_string())
    );
}
