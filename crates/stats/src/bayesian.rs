/// Bayesian evidence fusion.

/// Fuse multiple signals using Bayesian odds updating.
///
/// `prior`: Prior probability of the hypothesis (0-1).
/// `likelihoods`: Vec of (P(signal|true), P(signal|false)) pairs.
/// Returns posterior probability.
pub fn fuse_signals(prior: f64, likelihoods: &[(f64, f64)]) -> f64 {
    if prior <= 0.0 {
        return 0.0;
    }
    if prior >= 1.0 {
        return 1.0;
    }

    let mut log_odds = (prior / (1.0 - prior)).ln();
    for &(p_true, p_false) in likelihoods {
        if p_false > 1e-12 {
            log_odds += (p_true / p_false).ln();
        }
    }
    1.0 / (1.0 + (-log_odds).exp())
}

/// Beta-Binomial Bayesian updater for binary outcomes.
#[derive(Debug, Clone)]
pub struct BetaUpdater {
    alpha: f64,
    beta: f64,
}

impl BetaUpdater {
    /// Create with a uniform prior (alpha=1, beta=1).
    pub fn uniform_prior() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
        }
    }

    /// Create with custom prior parameters.
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self { alpha, beta }
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

    /// Total observations.
    pub fn total_observations(&self) -> f64 {
        self.alpha + self.beta - 2.0 // subtract prior parameters
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
        "barely_worth_mentioning"
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
        assert!(posterior > 0.9, "Strong evidence should push posterior high, got {}", posterior);
    }

    #[test]
    fn test_fuse_signals_against() {
        // Evidence against the hypothesis
        let posterior = fuse_signals(0.5, &[(0.1, 0.9), (0.2, 0.8)]);
        assert!(posterior < 0.1, "Counter-evidence should push posterior low, got {}", posterior);
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
        assert_eq!(interpret_bayes_factor(1.5), "barely_worth_mentioning");
        assert_eq!(interpret_bayes_factor(0.5), "against");
    }
}
