#![allow(clippy::disallowed_methods)]

use apex_api::validation::parse_country_code_filter;

#[test]
fn list_logistics_nodes_rejects_invalid_country_code_length() {
    let err =
        parse_country_code_filter(Some("USA")).expect_err("3-letter country code should fail");
    assert_eq!(err.http_status(), 422);
}

#[test]
fn list_logistics_nodes_rejects_non_alpha_country_code() {
    let err =
        parse_country_code_filter(Some("U1")).expect_err("non-alpha country code should fail");
    assert_eq!(err.http_status(), 422);
}

#[test]
fn list_logistics_nodes_accepts_valid_country_code() {
    let country_code =
        parse_country_code_filter(Some("US")).expect("valid country code should pass");
    assert_eq!(country_code.as_deref(), Some("US"));
}
