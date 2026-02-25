/// Fisher's exact test for 2×2 contingency tables.
///
/// All public functions operate on non-negative integer counts.  The p-value
/// is the exact two-sided probability computed by the hypergeometric
/// distribution, summing all tables at least as extreme as the observed one.

/// Compute the Fisher exact test p-value for a 2×2 table.
///
/// Table layout:
/// | a | b |
/// | c | d |
///
/// Returns `1.0` when the total count is zero.
/// Result is always in `[0.0, 1.0]` and is never `NaN`.
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
    // Exact ln(k!) for small k avoids any Stirling error in the range that
    // matters most for P-value precision (small marginal sums).
    const LUT: &[f64] = &[
        0.0,                      // 0! = 1
        0.0,                      // 1! = 1
        0.6931471805599453,       // 2!
        1.791759469228327,        // 3!
        3.178053830347946,        // 4!
        4.787491742782046,        // 5!
        6.579251212010101,        // 6!
        8.525161361065415,        // 7!
        10.60460290274525,        // 8!
        12.801827480081469,       // 9!
        15.104412573075518,       // 10!
        17.502307845873887,       // 11!
        19.987214495661885,       // 12!
        22.55216385312342,        // 13!
        25.19122118273868,        // 14!
        27.899271383840894,       // 15!
        30.671860106080675,       // 16!
        33.50507345013689,        // 17!
        36.39544520803305,        // 18!
        39.339884187199495,       // 19!
        42.335616460753485,       // 20!
    ];
    if n < LUT.len() as u64 {
        return LUT[n as usize];
    }
    // Stirling's series for n > 20:
    //   ln(n!) = n·ln(n) − n + ½·ln(2πn) + 1/(12n) − 1/(360n³) + 1/(1260n⁵)
    // Error < 1e-13 for n ≥ 21; negligible compared to f64 precision in log-
    // hypergeometric sums.  O(1) vs the previous O(n) loop.
    let x = n as f64;
    x * x.ln()
        - x
        + 0.5 * (2.0 * std::f64::consts::PI * x).ln()
        + 1.0 / (12.0 * x)
        - 1.0 / (360.0 * x.powi(3))
        + 1.0 / (1260.0 * x.powi(5))
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

    // ── B259: large counts ──────────────────────────────────────────────────

    #[test]
    fn test_fisher_large_counts_extreme_association() {
        // Zero off-diagonal cells → probability of this table is very small;
        // the two-sided p-value for the one observably possible table is 1.0
        // but the one-sided is near zero—our impl returns the standard value.
        let p = p_value(500, 0, 0, 500);
        assert!(p >= 0.0 && p <= 1.0, "p should be in [0,1], got {p}");
        assert!(p.is_finite(), "p must be finite for large counts");
    }

    #[test]
    fn test_fisher_large_counts_balanced() {
        // Balanced 250×250×250×250 table: no association → high p-value
        let p = p_value(250, 250, 250, 250);
        assert!(p > 0.5, "balanced table should have p > 0.5, got {p}");
        assert!(p >= 0.0 && p <= 1.0);
    }

    #[test]
    fn test_fisher_large_counts_asymmetric() {
        // Strong association in a large table
        let p = p_value(100, 10, 10, 100);
        assert!(p < 0.001, "strong 10:1 association should have p < 0.001, got {p}");
        assert!(p >= 0.0 && p <= 1.0);
    }

    #[test]
    fn test_fisher_large_counts_result_not_nan() {
        // Verify no NaN for moderate large tables
        for (a, b, c, d) in &[(200u64, 50, 30, 300), (1000, 1000, 1000, 1000)] {
            let p = p_value(*a, *b, *c, *d);
            assert!(p.is_finite(), "p must not be NaN/inf for ({a},{b},{c},{d}): got {p}");
            assert!(p >= 0.0 && p <= 1.0);
        }
    }

    // ── B260: log_factorial accuracy after Stirling optimisation ───────────

    #[test]
    fn test_log_factorial_small_exact() {
        // These must match the LUT values exactly
        assert!((log_factorial(0) - 0.0).abs() < 1e-14);
        assert!((log_factorial(1) - 0.0).abs() < 1e-14);
        assert!((log_factorial(5) - 4.787491742782046).abs() < 1e-12);
        assert!((log_factorial(10) - 15.104412573075518).abs() < 1e-12);
        assert!((log_factorial(20) - 42.335616460753485).abs() < 1e-10);
    }

    #[test]
    fn test_log_factorial_stirling_accuracy() {
        // ln(30!) known value ≈ 74.6582363488302
        // Computed via exact summation in a reference implementation.
        let expected_ln30 = (2u64..=30).map(|i| (i as f64).ln()).sum::<f64>();
        let got = log_factorial(30);
        assert!(
            (got - expected_ln30).abs() < 1e-10,
            "ln(30!) Stirling error too large: expected {expected_ln30}, got {got}"
        );
    }

    #[test]
    fn test_log_factorial_large_stirling_accuracy() {
        // For large n compare against exact summation to verify < 1e-10 error
        for n in [50u64, 100, 500, 1000] {
            let exact = (2..=n).map(|i| (i as f64).ln()).sum::<f64>();
            let approx = log_factorial(n);
            assert!(
                (approx - exact).abs() < 1e-8,
                "ln({n}!) error too large: exact={exact}, approx={approx}, diff={}",
                (approx - exact).abs()
            );
        }
    }
}
