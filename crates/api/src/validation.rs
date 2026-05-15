use anyhow::Error as AnyhowError;
use uuid::Uuid;

use crate::responses::ApiError;
use apex_core::entities::ObservationType;

pub fn parse_optional_uuid_filter(
    field: &str,
    raw: Option<&str>,
) -> Result<Option<Uuid>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    Uuid::parse_str(raw)
        .map(Some)
        .map_err(|_| ApiError::validation(field, format!("{field} must be a valid UUID")))
}

pub fn parse_observation_type_filter(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    ObservationType::from_str(raw)
        .map(|value| Some(value.as_str().to_string()))
        .ok_or_else(|| {
            ApiError::validation(
                "observation_type",
                format!("Unsupported observation_type '{raw}'"),
            )
        })
}

pub fn parse_country_code_filter(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if raw.len() != 2 {
        return Err(ApiError::validation(
            "country_code",
            "country_code must be a 2-letter uppercase code",
        ));
    }

    if !raw.chars().all(|ch| ch.is_ascii_uppercase()) {
        return Err(ApiError::validation(
            "country_code",
            "country_code must contain only uppercase A-Z letters",
        ));
    }

    Ok(Some(raw.to_string()))
}

pub fn internal_error(operation: &str) -> ApiError {
    ApiError::internal(operation)
}

pub fn map_internal_error(operation: &str, error: &AnyhowError) -> ApiError {
    tracing::error!(error = ?error, operation, "request handling failed");
    internal_error(operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::responses::{error_response, ErrorCode};

    #[test]
    fn list_observations_rejects_unknown_observation_type() {
        let err = parse_observation_type_filter(Some("UnknownType"))
            .expect_err("unknown observation type should fail");
        assert_eq!(err.http_status(), 422);
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("Unsupported observation_type"));
    }

    #[test]
    fn list_observations_accepts_known_observation_type() {
        let observation_type = parse_observation_type_filter(Some("CompetitorEvent"))
            .expect("known observation type should succeed");
        assert_eq!(observation_type.as_deref(), Some("CompetitorEvent"));
    }

    #[test]
    fn list_observations_returns_api_error_shape_for_validation_failures() {
        let err = parse_observation_type_filter(Some("invalid"))
            .expect_err("invalid observation type should fail");
        let payload = error_response::<()>(err);
        let error = payload.error.expect("error payload");
        assert_eq!(error.code, ErrorCode::ValidationError);
        assert_eq!(
            error.details.expect("details").get("field"),
            Some(&"observation_type".to_string())
        );
    }

    #[test]
    fn list_logistics_nodes_rejects_invalid_country_code_length() {
        let err =
            parse_country_code_filter(Some("USA")).expect_err("3-letter country code should fail");
        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("2-letter uppercase code"));
    }

    #[test]
    fn list_logistics_nodes_rejects_non_alpha_country_code() {
        let err =
            parse_country_code_filter(Some("U1")).expect_err("non-alpha country code should fail");
        assert_eq!(err.http_status(), 422);
        assert!(err.message.contains("uppercase A-Z"));
    }

    #[test]
    fn list_logistics_nodes_accepts_valid_country_code() {
        let country_code =
            parse_country_code_filter(Some("US")).expect("valid country code should succeed");
        assert_eq!(country_code.as_deref(), Some("US"));
    }

    #[test]
    fn validation_failures_return_uniform_api_error_payload() {
        let observation_error = error_response::<()>(
            parse_observation_type_filter(Some("bad-value")).expect_err("validation error"),
        )
        .error
        .expect("observation error");
        let country_error = error_response::<()>(
            parse_country_code_filter(Some("us")).expect_err("validation error"),
        )
        .error
        .expect("country error");

        assert_eq!(observation_error.code, ErrorCode::ValidationError);
        assert_eq!(country_error.code, ErrorCode::ValidationError);
        assert!(observation_error.details.is_some());
        assert!(country_error.details.is_some());
    }

    #[test]
    fn handler_internal_errors_map_to_consistent_500_response() {
        let err = internal_error("Failed to list observations");
        let payload = error_response::<()>(err);
        let error = payload.error.expect("error payload");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert_eq!(error.message, "Failed to list observations");
    }

    #[test]
    fn query_parsing_errors_do_not_leak_anyhow_strings() {
        let err = parse_optional_uuid_filter("entity_id", Some("not-a-uuid"))
            .expect_err("invalid UUID should fail");
        assert_eq!(err.http_status(), 422);
        assert!(!err.message.contains("invalid length"));
        assert_eq!(err.message, "entity_id must be a valid UUID");
    }
}
