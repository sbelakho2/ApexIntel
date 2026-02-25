/// Shared numeric safety helpers for statistical computations.

const SAFE_DIV_EPS: f64 = 1e-12;

/// Safely divide two floating-point numbers.
///
/// Returns `0.0` when either operand is non-finite or the denominator is too
/// close to zero to avoid unstable spikes and infinities.
pub fn safe_div(numerator: f64, denominator: f64) -> f64 {
    if !numerator.is_finite() || !denominator.is_finite() || denominator.abs() < SAFE_DIV_EPS {
        return 0.0;
    }
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_div_normal() {
        assert!((safe_div(10.0, 2.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn test_safe_div_zero_denominator_returns_zero() {
        assert_eq!(safe_div(10.0, 0.0), 0.0);
    }

    #[test]
    fn test_safe_div_non_finite_returns_zero() {
        assert_eq!(safe_div(f64::INFINITY, 2.0), 0.0);
        assert_eq!(safe_div(2.0, f64::NAN), 0.0);
    }
}
