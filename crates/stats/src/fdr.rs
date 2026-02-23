/// Benjamini-Hochberg False Discovery Rate correction.

/// Apply Benjamini-Hochberg FDR correction to a vector of p-values.
///
/// Returns the adjusted q-values.
pub fn bh_correct(pvals: &[f64]) -> Vec<f64> {
    if pvals.is_empty() {
        return vec![];
    }

    let m = pvals.len() as f64;
    let mut indexed: Vec<(usize, f64)> = pvals.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    let mut q = vec![0.0; pvals.len()];
    let mut prev = 1.0;

    for (rank, (i, p)) in indexed.into_iter().rev().enumerate() {
        let r = (pvals.len() - rank) as f64;
        let val = (p * m / r).min(prev);
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
            assert!(w[0] <= w[1] + 1e-10, "q-values not monotonic: {} > {}", w[0], w[1]);
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
            assert!(*qv >= 0.0 && *qv <= 1.0, "q-value out of range: {}", qv);
        }
    }
}
