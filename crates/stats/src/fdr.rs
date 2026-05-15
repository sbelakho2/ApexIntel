//! Benjamini-Hochberg False Discovery Rate correction.
//!
//! The BH procedure ranks non-`NaN` p-values from smallest to largest and
//! computes q-values as `min(p × m / rank, prev_q)` over the `m` non-`NaN`
//! tests.  `NaN` p-values are preserved at their original positions in the
//! output (B258).  All non-`NaN` q-values are clamped to `[0.0, 1.0]`.

/// Apply Benjamini-Hochberg FDR correction to a vector of p-values.
///
/// Returns the adjusted q-values.  `NaN` entries in `pvals` are propagated
/// as-is to the output without inflating the correction denominator (B258).
/// Output length always equals input length.
pub fn bh_correct(pvals: &[f64]) -> Vec<f64> {
    if pvals.is_empty() {
        return vec![];
    }

    // Separate NaN positions: BH correction should use m = number of
    // *non-NaN* tests and assign ranks only among them (R's p.adjust
    // semantics).  Previously NaN values consumed rank positions and
    // inflated m, over-correcting every non-NaN q-value.
    let mut q = vec![0.0_f64; pvals.len()];
    let mut non_nan: Vec<(usize, f64)> = pvals
        .iter()
        .cloned()
        .enumerate()
        .filter(|(i, p)| {
            if p.is_nan() {
                q[*i] = f64::NAN;
                false
            } else {
                true
            }
        })
        .collect();

    if non_nan.is_empty() {
        return q;
    }

    let m = non_nan.len() as f64;
    non_nan.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut prev = 1.0;
    for (rank, (i, p)) in non_nan.into_iter().rev().enumerate() {
        let r = (m as usize - rank) as f64;
        let val = (p * m / r).min(prev).clamp(0.0, 1.0);
        prev = val;
        q[i] = val;
    }
    q
}

/// Count how many hypotheses are significant at a given FDR threshold.
pub fn count_significant(pvals: &[f64], fdr_threshold: f64) -> usize {
    let q = bh_correct(pvals);
    q.iter().filter(|&&v| v < fdr_threshold).count()
}

/// Determine which indices are significant after FDR correction.
pub fn significant_indices(pvals: &[f64], fdr_threshold: f64) -> Vec<usize> {
    let q = bh_correct(pvals);
    q.iter()
        .enumerate()
        .filter(|(_, &v)| v < fdr_threshold)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bh_correct_basic() {
        let pvals = vec![0.01, 0.03, 0.05, 0.10, 0.50];
        let q = bh_correct(&pvals);
        // q-values should be >= p-values
        for (p, qv) in pvals.iter().zip(q.iter()) {
            assert!(*qv >= *p - 1e-10, "q ({}) should be >= p ({})", qv, p);
        }
    }

    #[test]
    fn test_bh_correct_monotonic() {
        let pvals = vec![0.001, 0.01, 0.03, 0.05, 0.20, 0.80];
        let q = bh_correct(&pvals);
        // When original p-values are sorted, q-values should also be monotonic
        for w in q.windows(2) {
            assert!(
                w[0] <= w[1] + 1e-10,
                "q-values not monotonic: {} > {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn test_bh_correct_all_significant() {
        let pvals = vec![0.001, 0.002, 0.003];
        let q = bh_correct(&pvals);
        for qv in &q {
            assert!(*qv < 0.01);
        }
    }

    #[test]
    fn test_bh_correct_none_significant() {
        let pvals = vec![0.5, 0.6, 0.7, 0.8];
        let q = bh_correct(&pvals);
        for qv in &q {
            assert!(*qv > 0.05);
        }
    }

    #[test]
    fn test_bh_correct_empty() {
        let q = bh_correct(&[]);
        assert!(q.is_empty());
    }

    #[test]
    fn test_count_significant() {
        let pvals = vec![0.001, 0.01, 0.03, 0.30, 0.80];
        let n = count_significant(&pvals, 0.05);
        assert!(n >= 2, "At least first 2 should be significant, got {}", n);
    }

    #[test]
    fn test_significant_indices() {
        let pvals = vec![0.001, 0.5, 0.002, 0.9];
        let sig = significant_indices(&pvals, 0.05);
        assert!(sig.contains(&0));
        assert!(sig.contains(&2));
        assert!(!sig.contains(&1));
        assert!(!sig.contains(&3));
    }

    #[test]
    fn test_bh_values_bounded() {
        let pvals = vec![0.01, 0.05, 0.10, 0.50, 0.99];
        let q = bh_correct(&pvals);
        for qv in &q {
            assert!((0.0..=1.0).contains(qv), "q-value out of range: {}", qv);
        }
    }

    #[test]
    fn test_bh_correct_handles_nan_without_panic() {
        let pvals = vec![0.01, f64::NAN, 0.20, 0.03];
        let q = bh_correct(&pvals);
        assert_eq!(q.len(), pvals.len());
    }

    // ── B258: NaN positions preserved in BH output ──────────────────────────

    #[test]
    fn test_bh_correct_nan_position_preserved_at_index() {
        // NaN at index 1 must survive to output index 1; other positions must
        // be finite and correctly BH-adjusted over the 3 non-NaN tests.
        let pvals = vec![0.01, f64::NAN, 0.20, 0.03];
        let q = bh_correct(&pvals);

        assert_eq!(q.len(), 4, "output length must equal input length");
        assert!(
            q[1].is_nan(),
            "NaN at position 1 must be preserved, got {}",
            q[1]
        );

        // Non-NaN positions must be finite
        for (i, &qv) in q.iter().enumerate() {
            if i != 1 {
                assert!(qv.is_finite(), "q[{i}] should be finite; got {qv}");
                assert!(
                    (0.0..=1.0).contains(&qv),
                    "q[{i}] must be in [0,1]; got {qv}"
                );
            }
        }
    }

    #[test]
    fn test_bh_correct_nan_does_not_inflate_correction() {
        // With NaN at one position, correction should be over m=3 tests, not m=4.
        // Without the NaN-exclusion fix, q-values would be inflated (over-corrected).
        let pvals_with_nan = vec![0.01, f64::NAN, 0.03, 0.05];
        let pvals_clean = vec![0.01, 0.03, 0.05]; // same tests, no NaN

        let q_nan = bh_correct(&pvals_with_nan);
        let q_clean = bh_correct(&pvals_clean);

        // q[0] from NaN version (m=3) must equal q_clean[0] (also m=3)
        assert!(
            (q_nan[0] - q_clean[0]).abs() < 1e-10,
            "NaN must not inflate m; q_nan[0]={}, q_clean[0]={}",
            q_nan[0],
            q_clean[0]
        );
    }

    #[test]
    fn test_bh_correct_all_nan_returns_all_nan() {
        let pvals = vec![f64::NAN; 5];
        let q = bh_correct(&pvals);
        assert_eq!(q.len(), 5);
        for (i, &qv) in q.iter().enumerate() {
            assert!(qv.is_nan(), "all-NaN input: q[{i}] should be NaN; got {qv}");
        }
    }

    #[test]
    fn test_bh_correct_nan_at_every_other_position() {
        // Alternating NaN and valid values
        let pvals = vec![0.01, f64::NAN, 0.05, f64::NAN, 0.10];
        let q = bh_correct(&pvals);
        assert_eq!(q.len(), 5);
        assert!(q[1].is_nan(), "NaN at index 1 must survive");
        assert!(q[3].is_nan(), "NaN at index 3 must survive");
        assert!(q[0].is_finite() && q[2].is_finite() && q[4].is_finite());
    }

    #[test]
    fn bh_fdr_monotonicity() {
        let pvals = vec![0.001, 0.01, 0.025, 0.05, 0.1, 0.4, 0.8];
        let qvals = bh_correct(&pvals);
        for (p, q) in pvals.iter().zip(qvals.iter()) {
            assert!(*q >= *p);
        }
    }
}
