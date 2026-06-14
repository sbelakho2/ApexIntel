//! Error types for the geopolitical intelligence module

use thiserror::Error;

/// Result type alias for the geopolitical module
pub type Result<T> = std::result::Result<T, GeopoliticalError>;

/// Errors that can occur in the geopolitical intelligence module
#[derive(Error, Debug)]
pub enum GeopoliticalError {
    /// Network-related errors
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    
    /// Parsing errors
    #[error("Failed to parse data: {0}")]
    ParseError(String),
    
    /// Validation errors
    #[error("Validation error: {0}")]
    ValidationError(String),
    
    /// Data not found
    #[error("Not found: {0}")]
    NotFound(String),
    
    /// API errors from external services
    #[error("External API error: {0}")]
    ExternalApiError(String),
    
    /// Rate limiting errors
    #[error("Rate limit exceeded, retry after {0} seconds")]
    RateLimitExceeded(u64),
    
    /// Configuration errors
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    /// Storage errors
    #[error("Storage error: {0}")]
    StorageError(String),
    
    /// Sanctions-specific errors
    #[error("Sanctions error: {0}")]
    SanctionsError(String),
    
    /// Trade-related errors
    #[error("Trade error: {0}")]
    TradeError(String),
    
    /// Political risk errors
    #[error("Political risk error: {0}")]
    PoliticalRiskError(String),
    
    /// Regulatory errors
    #[error("Regulatory error: {0}")]
    RegulatoryError(String),
    
    /// XML parsing errors
    #[error("XML parsing error: {0}")]
    XmlError(String),
    
    /// CSV parsing errors
    #[error("CSV parsing error: {0}")]
    CsvError(String),
    
    /// Internal errors
    #[error("Internal error: {0}")]
    Internal(String),
}

impl GeopoliticalError {
    /// Check if this is a retryable error
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            GeopoliticalError::Network(_) |
            GeopoliticalError::RateLimitExceeded(_) |
            GeopoliticalError::ExternalApiError(_)
        )
    }

    /// Get error code for logging
    pub fn error_code(&self) -> &'static str {
        match self {
            GeopoliticalError::Network(_) => "GEO001",
            GeopoliticalError::ParseError(_) => "GEO002",
            GeopoliticalError::ValidationError(_) => "GEO003",
            GeopoliticalError::NotFound(_) => "GEO004",
            GeopoliticalError::ExternalApiError(_) => "GEO005",
            GeopoliticalError::RateLimitExceeded(_) => "GEO006",
            GeopoliticalError::ConfigError(_) => "GEO007",
            GeopoliticalError::StorageError(_) => "GEO008",
            GeopoliticalError::SanctionsError(_) => "GEO009",
            GeopoliticalError::TradeError(_) => "GEO010",
            GeopoliticalError::PoliticalRiskError(_) => "GEO011",
            GeopoliticalError::RegulatoryError(_) => "GEO012",
            GeopoliticalError::XmlError(_) => "GEO013",
            GeopoliticalError::CsvError(_) => "GEO014",
            GeopoliticalError::Internal(_) => "GEO015",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        let err = GeopoliticalError::ParseError("test".to_string());
        assert_eq!(err.error_code(), "GEO002");
        
        let err = GeopoliticalError::NotFound("entity".to_string());
        assert_eq!(err.error_code(), "GEO004");
    }

    #[test]
    fn test_retryable() {
        let err = GeopoliticalError::RateLimitExceeded(60);
        assert!(err.is_retryable());

        let err = GeopoliticalError::NotFound("test".to_string());
        assert!(!err.is_retryable());
    }
}
