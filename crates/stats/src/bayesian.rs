/// Bayesian evidence fusion.

const LOG_BAYES_FACTOR_CAP: f64 = 30.0;

fn stable_logistic(log_odds: f64) -> f64 {
    if log_odds >= 0.0 {
        let z = (-log_odds).exp();
        1.0 / (1.0 + z)
    } else {
        let z = log_odds.exp();
        z / (1.0 + z)
    }
}

/// Fuse multiple signals using Bayesian odds updating.
///
/// `prior`: Prior probability of the hypothesis (0-1).
/// `likelihoods`: Vec of (P(signal|true), P(signal|false)) pairs.
/// Returns posterior probability.
pub fn fuse_signals(prior: f64, likelihoods: &[(f64, f64)]) -> f64 {
    fuse_signals_detailed(prior, likelihoods).posterior
}

#[derive(Debug, Clone)]
pub struct BayesianFusionResult {
    pub posterior: f64,
    pub capped_updates: usize,
    pub cap_trigger_rate: f64,
    pub interpretation: &'static str,
    pub combined_bayes_factor: f64,
}

pub fn fuse_signals_detailed(prior: f64, likelihoods: &[(f64, f64)]) -> BayesianFusionResult {
    if prior <= 0.0 {
        return BayesianFusionResult {
            posterior: 0.0,
            capped_updates: 0,
            cap_trigger_rate: 0.0,
            interpretation: "against",
            combined_bayes_factor: 0.0,
        };
    }
    if prior >= 1.0 {
        return BayesianFusionResult {
            posterior: 1.0,
            capped_updates: 0,
            cap_trigger_rate: 0.0,
            interpretation: "decisive",
            combined_bayes_factor: f64::INFINITY,
        };
    }

    let prior_log_odds = (prior / (1.0 - prior)).ln();
    let mut log_odds = prior_log_odds;
    let mut capped_updates = 0usize;
    for &(p_true, p_false) in likelihoods {
        if p_true.abs() < 1e-12 && p_false.abs() < 1e-12 {
            continue; // both near zero — uninformative signal
        }
        let log_bf = if p_false < 1e-12 {
            LOG_BAYES_FACTOR_CAP // decisive support: cap log Bayes factor
        } else if p_true < 1e-12 {
            -LOG_BAYES_FACTOR_CAP // decisive refutation: cap log Bayes factor
        } else {
            let raw = (p_true / p_false).ln();
            if raw > LOG_BAYES_FACTOR_CAP {
                capped_updates += 1;
                LOG_BAYES_FACTOR_CAP
            } else if raw < -LOG_BAYES_FACTOR_CAP {
                capped_updates += 1;
                -LOG_BAYES_FACTOR_CAP
            } else {
                raw
            }
        };
        if p_false < 1e-12 || p_true < 1e-12 {
            capped_updates += 1;
        }
        log_odds += log_bf;
    }
    let combined_log_bf = log_odds - prior_log_odds;
    let combined_bayes_factor = combined_log_bf.exp();

    BayesianFusionResult {
        posterior: stable_logistic(log_odds),
        capped_updates,
        cap_trigger_rate: if likelihoods.is_empty() {
            0.0
        } else {
            capped_updates as f64 / likelihoods.len() as f64
        },
        interpretation: interpret_bayes_factor(combined_bayes_factor),
        combined_bayes_factor,
    }
}

/// Beta-Binomial Bayesian updater for binary outcomes.
#[derive(Debug, Clone)]
pub struct BetaUpdater {
    alpha: f64,
    beta: f64,
    prior_alpha: f64,
    prior_beta: f64,
}

impl BetaUpdater {
    /// Create with a uniform prior (alpha=1, beta=1).
    pub fn uniform_prior() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
            prior_alpha: 1.0,
            prior_beta: 1.0,
        }
    }

    /// Create with custom prior parameters.
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha,
            beta,
            prior_alpha: alpha,
            prior_beta: beta,
        }
    }

    /// Update with a success observation.
    pub fn observe_success(&mut self) {
        self.alpha += 1.0;
    }

    /// Update with a failure observation.
    pub fn observe_failure(&mut self) {
        self.beta += 1.0;
    }

    /// Posterior mean.
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Posterior variance.
    pub fn variance(&self) -> f64 {
        let ab = self.alpha + self.beta;
        (self.alpha * self.beta) / (ab * ab * (ab + 1.0))
    }

    /// 95% credible interval (approximate using normal for large n).
    pub fn credible_interval_95(&self) -> (f64, f64) {
        let m = self.mean();
        let sd = self.variance().sqrt();
        ((m - 1.96 * sd).max(0.0), (m + 1.96 * sd).min(1.0))
    }

    /// Total observations (excludes the prior).
    pub fn total_observations(&self) -> f64 {
        (self.alpha - self.prior_alpha) + (self.beta - self.prior_beta)
    }
}

/// Compute Bayes factor for two hypotheses.
///
/// BF = P(data|H1) / P(data|H0)
/// Positive log BF = evidence for H1.
pub fn bayes_factor(likelihood_h1: f64, likelihood_h0: f64) -> f64 {
    if likelihood_h0 < 1e-12 {
        return f64::INFINITY;
    }
    likelihood_h1 / likelihood_h0
}

/// Interpret Bayes factor according to Jeffreys' scale.
pub fn interpret_bayes_factor(bf: f64) -> &'static str {
    if bf > 100.0 {
        "decisive"
    } else if bf > 30.0 {
        "very_strong"
    } else if bf > 10.0 {
        "strong"
    } else if bf > 3.0 {
        "substantial"
    } else if bf > 1.0 {
        "barely"
    } else {
        "against"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuse_signals_prior_only() {
        let posterior = fuse_signals(0.5, &[]);
        assert!((posterior - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_fuse_signals_strong_evidence() {
        // Strong evidence for the hypothesis
        let posterior = fuse_signals(0.5, &[(0.9, 0.1), (0.8, 0.2)]);
        assert!(
            posterior > 0.9,
            "Strong evidence should push posterior high, got {}",
            posterior
        );
    }

    #[test]
    fn test_fuse_signals_against() {
        // Evidence against the hypothesis
        let posterior = fuse_signals(0.5, &[(0.1, 0.9), (0.2, 0.8)]);
        assert!(
            posterior < 0.1,
            "Counter-evidence should push posterior low, got {}",
            posterior
        );
    }

    #[test]
    fn test_fuse_signals_extreme_positive_log_odds_stays_finite() {
        let mut likes = Vec::new();
        for _ in 0..2_000 {
            likes.push((1.0, 1e-20));
        }
        let posterior = fuse_signals(0.5, &likes);
        assert!(posterior.is_finite());
        assert!(posterior > 0.999_999);
    }

    #[test]
    fn bayesian_strong_evidence_posterior() {
        let posterior = fuse_signals(0.5, &[(1.0, 1e-20); 5]);
        assert!(posterior > 0.9999, "expected posterior > 0.9999, got {posterior}");
    }

    #[test]
    fn test_fuse_signals_extreme_negative_log_odds_stays_finite() {
        let mut likes = Vec::new();
        for _ in 0..2_000 {
            likes.push((1e-20, 1.0));
        }
        let posterior = fuse_signals(0.5, &likes);
        assert!(posterior.is_finite());
        assert!(posterior < 0.000_001);
    }

    #[test]
    fn test_fuse_signals_edge_prior_zero() {
        let posterior = fuse_signals(0.0, &[(0.9, 0.1)]);
        assert!((posterior - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_beta_updater_uniform_prior() {
        let u = BetaUpdater::uniform_prior();
        assert!((u.mean() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_beta_updater_updates() {
        let mut u = BetaUpdater::uniform_prior();
        for _ in 0..8 {
            u.observe_success();
        }
        for _ in 0..2 {
            u.observe_failure();
        }
        // With 8 successes and 2 failures + uniform prior: alpha=9, beta=3
        // Mean = 9/12 = 0.75
        assert!((u.mean() - 0.75).abs() < 1e-10);
    }

    #[test]
    fn test_beta_updater_credible_interval() {
        let mut u = BetaUpdater::uniform_prior();
        for _ in 0..50 {
            u.observe_success();
        }
        for _ in 0..50 {
            u.observe_failure();
        }
        let (lo, hi) = u.credible_interval_95();
        assert!(lo > 0.0 && lo < 0.5);
        assert!(hi > 0.5 && hi <= 1.0);
    }

    #[test]
    fn test_bayes_factor_basic() {
        let bf = bayes_factor(0.8, 0.2);
        assert!((bf - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_interpret_bayes_factor() {
        assert_eq!(interpret_bayes_factor(200.0), "decisive");
        assert_eq!(interpret_bayes_factor(50.0), "very_strong");
        assert_eq!(interpret_bayes_factor(15.0), "strong");
        assert_eq!(interpret_bayes_factor(5.0), "substantial");
        assert_eq!(interpret_bayes_factor(1.5), "barely");
        assert_eq!(interpret_bayes_factor(0.5), "against");
    }

    #[test]
    fn test_interpret_bayes_factor_threshold_boundaries() {
        assert_eq!(interpret_bayes_factor(100.0), "very_strong");
        assert_eq!(interpret_bayes_factor(30.0), "strong");
        assert_eq!(interpret_bayes_factor(10.0), "substantial");
        assert_eq!(interpret_bayes_factor(3.0), "barely");
        assert_eq!(interpret_bayes_factor(1.0), "against");
    }

    #[test]
    fn capped_updates_are_reported() {
        let detailed = fuse_signals_detailed(0.5, &[(1.0, 1e-20); 10]);

        assert!(detailed.capped_updates > 0);
        assert!(detailed.cap_trigger_rate > 0.05);
        assert_eq!(detailed.interpretation, "decisive");
    }

    #[test]
    fn bayesian_posterior_bounded() {
        let sequences = [
            vec![(0.9, 0.1), (0.8, 0.2), (0.7, 0.3)],
            vec![(0.2, 0.8), (0.1, 0.9), (0.4, 0.6)],
            vec![(1.0, 1e-20); 5],
        ];

        for likelihoods in sequences {
            let posterior = fuse_signals(0.5, &likelihoods);
            assert!((0.0..=1.0).contains(&posterior));
        }
    }

    // ── B270: previously uncovered utility methods ──────────────────────────

    #[test]
    fn test_beta_updater_variance_uniform_prior() {
        // Uniform prior: alpha=1, beta=1
        // variance = (1*1) / (2^2 * 3) = 1/12
        let u = BetaUpdater::uniform_prior();
        let expected = 1.0 / 12.0;
        assert!(
            (u.variance() - expected).abs() < 1e-10,
            "uniform prior variance = {:.6}, expected {:.6}",
            u.variance(),
            expected
        );
    }

    #[test]
    fn test_beta_updater_variance_after_updates() {
        // 8 successes + 2 failures on uniform prior → alpha=9, beta=3, ab=12
        // variance = (9*3) / (12^2 * 13) = 27 / 1872 ≈ 0.01442
        let mut u = BetaUpdater::uniform_prior();
        for _ in 0..8 {
            u.observe_success();
        }
        for _ in 0..2 {
            u.observe_failure();
        }
        let expected = (9.0 * 3.0) / (12.0_f64.powi(2) * 13.0);
        assert!(
            (u.variance() - expected).abs() < 1e-10,
            "variance after updates = {:.6}, expected {:.6}",
            u.variance(),
            expected
        );
    }

    #[test]
    fn test_beta_updater_variance_symmetric_peak() {
        // Symmetric Beta(50,50): variance peaks near mean=0.5
        let mut u = BetaUpdater::uniform_prior();
        for _ in 0..49 {
            u.observe_success();
        }
        for _ in 0..49 {
            u.observe_failure();
        }
        // Both mean and variance should be close to symmetric values
        assert!((u.mean() - 0.5).abs() < 1e-10);
        // variance should be small (concentrated distribution)
        assert!(
            u.variance() < 0.005,
            "variance should be small for n=99, got {}",
            u.variance()
        );
    }

    #[test]
    fn test_beta_updater_total_observations_zero_on_init() {
        // Fresh updater has seen no data — prior is not counted as observations
        let u = BetaUpdater::uniform_prior();
        assert!(
            (u.total_observations() - 0.0).abs() < 1e-10,
            "fresh updater should have 0 observations, got {}",
            u.total_observations()
        );
    }

    #[test]
    fn test_beta_updater_total_observations_counts_both() {
        let mut u = BetaUpdater::uniform_prior();
        for _ in 0..7 {
            u.observe_success();
        }
        for _ in 0..3 {
            u.observe_failure();
        }
        assert!(
            (u.total_observations() - 10.0).abs() < 1e-10,
            "expected 10 observations, got {}",
            u.total_observations()
        );
    }

    #[test]
    fn test_beta_updater_total_observations_custom_prior() {
        // Custom prior (alpha=2, beta=5) — observations start at 0
        let mut u = BetaUpdater::new(2.0, 5.0);
        assert!((u.total_observations() - 0.0).abs() < 1e-10);
        for _ in 0..3 {
            u.observe_success();
        }
        assert!(
            (u.total_observations() - 3.0).abs() < 1e-10,
            "expected 3 obs after 3 successes, got {}",
            u.total_observations()
        );
        for _ in 0..2 {
            u.observe_failure();
        }
        assert!(
            (u.total_observations() - 5.0).abs() < 1e-10,
            "expected 5 total obs, got {}",
            u.total_observations()
        );
    }

    #[test]
    fn test_bayes_factor_zero_denominator_returns_infinity() {
        // When likelihood_h0 is effectively zero, BF must be infinite
        let bf = bayes_factor(0.9, 0.0);
        assert!(bf.is_infinite() && bf > 0.0, "expected +∞, got {bf}");
    }

    #[test]
    fn test_bayes_factor_both_small_returns_ratio() {
        // Both likelihoods above epsilon — simple ratio
        let bf = bayes_factor(0.3, 0.1);
        assert!((bf - 3.0).abs() < 1e-10, "expected 3.0, got {bf}");
    }
}
