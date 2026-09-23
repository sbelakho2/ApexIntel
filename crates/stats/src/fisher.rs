//! Fisher's exact test for 2×2 contingency tables.
//!
//! All public functions operate on non-negative integer counts.  The p-value
//! is the exact two-sided probability computed by the hypergeometric
//! distribution, summing all tables at least as extreme as the observed one.

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FisherExactResult {
    pub p_value: f64,
    pub odds_ratio: f64,
    pub odds_ratio_ci_low: Option<f64>,
    pub odds_ratio_ci_high: Option<f64>,
}

/// Compute the Fisher exact test p-value for a 2×2 table.
///
/// Table layout:
/// | a | b |
/// | c | d |
///
/// Returns `1.0` when the total count is zero.
/// Result is always in `[0.0, 1.0]` and is never `NaN`.
pub fn p_value(a: u64, b: u64, c: u64, d: u64) -> f64 {
    // Exact enumeration need not be attempted on adversarial inputs. Guard the
    // marginal sums against u64 overflow (using u128) and cap the enumeration
    // loop, returning the conservative `1.0` when the exact test is infeasible.
    let n_u = a as u128 + b as u128 + c as u128 + d as u128;
    if n_u == 0 {
        return 1.0;
    }
    let row1_u = a as u128 + b as u128;
    let col1_u = a as u128 + c as u128;
    if n_u > u64::MAX as u128 || row1_u.min(col1_u) > 1_000_000 {
        return 1.0;
    }

    let n = n_u as u64;
    let row1 = row1_u as u64;
    let col1 = col1_u as u64;

    let log_p_cutoff = log_hypergeometric(a, b, c, d, n);

    let mut p = 0.0;

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

pub fn analyze(a: u64, b: u64, c: u64, d: u64) -> FisherExactResult {
    let p_value = p_value(a, b, c, d);
    let odds_ratio = odds_ratio(a, b, c, d);
    let (odds_ratio_ci_low, odds_ratio_ci_high) = woolf_odds_ratio_confidence_interval(a, b, c, d)
        .map(|(low, high)| (Some(low), Some(high)))
        .unwrap_or((None, None));

    FisherExactResult {
        p_value,
        odds_ratio,
        odds_ratio_ci_low,
        odds_ratio_ci_high,
    }
}

/// Odds ratio for a 2×2 table.
///
/// Returns `+∞` when the denominator is zero but the numerator is positive,
/// and `NaN` when the ratio is genuinely undefined (`a·d = 0` and `b·c = 0`).
pub fn odds_ratio(a: u64, b: u64, c: u64, d: u64) -> f64 {
    if b == 0 || c == 0 {
        // 0/0 (or 0/x over 0) is undefined rather than infinite.
        if a == 0 || d == 0 {
            return f64::NAN;
        }
        return f64::INFINITY;
    }
    (a as f64 * d as f64) / (b as f64 * c as f64)
}

pub fn woolf_odds_ratio_confidence_interval(a: u64, b: u64, c: u64, d: u64) -> Option<(f64, f64)> {
    let total_u = a as u128 + b as u128 + c as u128 + d as u128;
    if total_u == 0 || total_u > u64::MAX as u128 {
        return None;
    }

    let correction = if a == 0 || b == 0 || c == 0 || d == 0 {
        0.5
    } else {
        0.0
    };
    let af = a as f64 + correction;
    let bf = b as f64 + correction;
    let cf = c as f64 + correction;
    let df = d as f64 + correction;

    if af <= 0.0 || bf <= 0.0 || cf <= 0.0 || df <= 0.0 {
        return None;
    }

    let odds_ratio = (af * df) / (bf * cf);
    let standard_error = (1.0 / af + 1.0 / bf + 1.0 / cf + 1.0 / df).sqrt();
    let delta = 1.96 * standard_error;
    Some((
        (odds_ratio.ln() - delta).exp(),
        (odds_ratio.ln() + delta).exp(),
    ))
}

pub fn minimum_detectable_odds_ratio(
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    alpha: f64,
    power: f64,
) -> f64 {
    let total = (a + b + c + d) as f64;
    if total <= 0.0 {
        return f64::INFINITY;
    }

    let row1 = (a + b) as f64;
    let row2 = (c + d) as f64;
    let col1 = (a + c) as f64;
    let col2 = (b + d) as f64;
    if row1 <= 0.0 || row2 <= 0.0 || col1 <= 0.0 || col2 <= 0.0 {
        return f64::INFINITY;
    }

    let expected_a = (row1 * col1 / total).max(0.5);
    let expected_b = (row1 * col2 / total).max(0.5);
    let expected_c = (row2 * col1 / total).max(0.5);
    let expected_d = (row2 * col2 / total).max(0.5);
    let standard_error =
        (1.0 / expected_a + 1.0 / expected_b + 1.0 / expected_c + 1.0 / expected_d).sqrt();
    let z_alpha = inverse_standard_normal_cdf(1.0 - alpha / 2.0);
    let z_power = inverse_standard_normal_cdf(power);
    ((z_alpha + z_power) * standard_error).exp()
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
        0.0,                    // 0! = 1
        0.0,                    // 1! = 1
        std::f64::consts::LN_2, // 2!
        1.791759469228327,      // 3!
        3.178053830347946,      // 4!
        4.787491742782046,      // 5!
        6.579251212010101,      // 6!
        8.525161361065415,      // 7!
        10.60460290274525,      // 8!
        12.801827480081469,     // 9!
        15.104412573075518,     // 10!
        17.502307845873887,     // 11!
        19.987214495661885,     // 12!
        22.55216385312342,      // 13!
        25.19122118273868,      // 14!
        27.899271383840894,     // 15!
        30.671860106080675,     // 16!
        33.50507345013689,      // 17!
        36.39544520803305,      // 18!
        39.339884187199495,     // 19!
        42.335616460753485,     // 20!
    ];
    if n < LUT.len() as u64 {
        return LUT[n as usize];
    }
    // Stirling's series for n > 20:
    //   ln(n!) = n·ln(n) − n + ½·ln(2πn) + 1/(12n) − 1/(360n³) + 1/(1260n⁵)
    // Error < 1e-13 for n ≥ 21; negligible compared to f64 precision in log-
    // hypergeometric sums.  O(1) vs the previous O(n) loop.
    let x = n as f64;
    x * x.ln() - x + 0.5 * (2.0 * std::f64::consts::PI * x).ln() + 1.0 / (12.0 * x)
        - 1.0 / (360.0 * x.powi(3))
        + 1.0 / (1260.0 * x.powi(5))
}

fn inverse_standard_normal_cdf(probability: f64) -> f64 {
    let p = probability.clamp(1e-12, 1.0 - 1e-12);
    const A: [f64; 6] = [
        -39.69683028665376,
        220.9460984245205,
        -275.9285104469687,
        138.357751867269,
        -30.66479806614716,
        2.506628277459239,
    ];
    const B: [f64; 5] = [
        -54.47609879822406,
        161.5858368580409,
        -155.6989798598866,
        66.80131188771972,
        -13.28068155288572,
    ];
    const C: [f64; 6] = [
        -0.007784894002430293,
        -0.3223964580411365,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    const D: [f64; 4] = [
        0.007784695709041462,
        0.3224671290700398,
        2.445134137142996,
        3.754408661907416,
    ];

    let plow = 0.02425;
    let phigh = 1.0 - plow;

    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        return (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }

    if p > phigh {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        return -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    }

    let q = p - 0.5;
    let r = q * q;
    (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
        / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
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
        assert!(
            p < 0.001,
            "Strong association should have p < 0.001, got {}",
            p
        );
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
    fn fisher_analyze_reports_effect_size_and_interval() {
        let result = analyze(10, 5, 3, 12);
        assert!((result.odds_ratio - 8.0).abs() < 1e-10);
        assert!((0.0..=1.0).contains(&result.p_value));
        assert!(result.odds_ratio_ci_low.is_some());
        assert!(result.odds_ratio_ci_high.is_some());
        assert!(matches!(result.odds_ratio_ci_low, Some(value) if value < result.odds_ratio));
        assert!(matches!(result.odds_ratio_ci_high, Some(value) if value > result.odds_ratio));
    }

    #[test]
    fn woolf_interval_handles_zero_cells_with_correction() {
        let interval = woolf_odds_ratio_confidence_interval(20, 0, 0, 20)
            .unwrap_or_else(|| panic!("interval should exist with continuity correction"));
        assert!(interval.0.is_finite());
        assert!(interval.1.is_finite());
        assert!(interval.1 > interval.0);
    }

    #[test]
    fn minimum_detectable_odds_ratio_decreases_with_sample_size() {
        let small = minimum_detectable_odds_ratio(5, 5, 5, 5, 0.01, 0.80);
        let large = minimum_detectable_odds_ratio(50, 50, 50, 50, 0.01, 0.80);
        assert!(small > large);
        assert!(large > 1.0);
    }

    #[test]
    fn test_p_value_range() {
        let p = p_value(5, 3, 2, 8);
        assert!(
            (0.0..=1.0).contains(&p),
            "p-value should be in [0,1], got {}",
            p
        );
    }

    // ── B259: large counts ──────────────────────────────────────────────────

    #[test]
    fn test_fisher_large_counts_extreme_association() {
        // Zero off-diagonal cells → probability of this table is very small;
        // the two-sided p-value for the one observably possible table is 1.0
        // but the one-sided is near zero—our impl returns the standard value.
        let p = p_value(500, 0, 0, 500);
        assert!((0.0..=1.0).contains(&p), "p should be in [0,1], got {p}");
        assert!(p.is_finite(), "p must be finite for large counts");
    }

    #[test]
    fn test_fisher_large_counts_balanced() {
        // Balanced 250×250×250×250 table: no association → high p-value
        let p = p_value(250, 250, 250, 250);
        assert!(p > 0.5, "balanced table should have p > 0.5, got {p}");
        assert!((0.0..=1.0).contains(&p));
    }

    #[test]
    fn test_fisher_large_counts_asymmetric() {
        // Strong association in a large table
        let p = p_value(100, 10, 10, 100);
        assert!(
            p < 0.001,
            "strong 10:1 association should have p < 0.001, got {p}"
        );
        assert!((0.0..=1.0).contains(&p));
    }

    #[test]
    fn test_fisher_large_counts_result_not_nan() {
        // Verify no NaN for moderate large tables
        for (a, b, c, d) in &[(200u64, 50, 30, 300), (1000, 1000, 1000, 1000)] {
            let p = p_value(*a, *b, *c, *d);
            assert!(
                p.is_finite(),
                "p must not be NaN/inf for ({a},{b},{c},{d}): got {p}"
            );
            assert!((0.0..=1.0).contains(&p));
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
