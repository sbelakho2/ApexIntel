use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApexError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("entity not found: {kind} id={id}")]
    NotFound { kind: String, id: String },

    #[error("validation error: {0}")]
    Validation(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, ApexError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let e = ApexError::Config("missing DB url".into());
        assert_eq!(e.to_string(), "configuration error: missing DB url");
    }

    #[test]
    fn test_not_found_display() {
        let e = ApexError::NotFound {
            kind: "Company".into(),
            id: "abc-123".into(),
        };
        assert!(e.to_string().contains("Company"));
        assert!(e.to_string().contains("abc-123"));
    }

    #[test]
    fn test_validation_error() {
        let e = ApexError::Validation("domain cannot be empty".into());
        assert!(e.to_string().contains("domain cannot be empty"));
    }

    #[test]
    fn test_result_alias() {
        let ok: Result<i32> = Ok(42);
        assert_eq!(ok.unwrap(), 42);

        let err: Result<i32> = Err(ApexError::Internal("boom".into()));
        assert!(err.is_err());
    }
}
