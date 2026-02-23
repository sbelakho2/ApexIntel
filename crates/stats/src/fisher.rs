/// Fisher's exact test for 2×2 contingency tables.

/// Compute the Fisher exact test p-value for a 2×2 table.
///
/// Table layout:
/// | a | b |
/// | c | d |
pub fn p_value(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let n = a + b + c + d;
    if n == 0 {
        return 1.0;
    }

    let log_p_cutoff = log_hypergeometric(a, b, c, d, n);

    let mut p = 0.0;
    let row1 = a + b;
    let col1 = a + c;

    for x in 0..=row1.min(col1) {
        let y = row1.saturating_sub(x);
        let z = col1.saturating_sub(x);
        let w_raw = (c + d).checked_sub(z);
        let w = match w_raw {
            Some(w) => w,
            None => continue,
        };
        // Verify the table sums correctly
        if x + y + z + w != n {
            continue;
        }
        let log_p = log_hypergeometric(x, y, z, w, n);
        if log_p <= log_p_cutoff + 1e-10 {
            p += log_p.exp();
        }
    }
    p.min(1.0)
}

/// Odds ratio for a 2×2 table.
pub fn odds_ratio(a: u64, b: u64, c: u64, d: u64) -> f64 {
    if b == 0 || c == 0 {
        return f64::INFINITY;
    }
    (a as f64 * d as f64) / (b as f64 * c as f64)
}

fn log_hypergeometric(a: u64, b: u64, c: u64, d: u64, n: u64) -> f64 {
    log_factorial(a + b) + log_factorial(c + d) + log_factorial(a + c) + log_factorial(b + d)
        - log_factorial(n)
        - log_factorial(a)
        - log_factorial(b)
        - log_factorial(c)
        - log_factorial(d)
}

fn log_factorial(n: u64) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    (2..=n).map(|i| (i as f64).ln()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fisher_no_association() {
        // Equal proportions
        let p = p_value(10, 10, 10, 10);
        assert!(p > 0.5, "No association should have p > 0.5, got {}", p);
    }

    #[test]
    fn test_fisher_strong_association() {
        // Very strong association
        let p = p_value(20, 0, 0, 20);
        assert!(p < 0.001, "Strong association should have p < 0.001, got {}", p);
    }

    #[test]
    fn test_fisher_moderate() {
        let p = p_value(8, 2, 3, 7);
        assert!(p < 0.1); // Should be somewhat significant
    }

    #[test]
    fn test_fisher_empty() {
        let p = p_value(0, 0, 0, 0);
        assert!((p - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_odds_ratio_basic() {
        let or = odds_ratio(10, 5, 3, 12);
        // (10*12)/(5*3) = 8.0
        assert!((or - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_odds_ratio_zero_cell() {
        let or = odds_ratio(10, 0, 5, 5);
        assert!(or.is_infinite());
    }

    #[test]
    fn test_p_value_range() {
        let p = p_value(5, 3, 2, 8);
        assert!(p >= 0.0 && p <= 1.0, "p-value should be in [0,1], got {}", p);
    }
}
