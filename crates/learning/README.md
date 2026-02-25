# apex-learning

Stats-first pattern discovery — mining, hypothesis generation, backtesting, and negative controls.

## Pipeline

```
Observations
    │
    ▼
miner::run_sweep()          — contingency tables, odds ratios, stability across time-splits
    │
    ▼
hypothesis::build_prompt()  — LLM-ready prompt from top PatternCandidates
    │
    ▼
backtest::backtest_candidate()  — walk-forward accuracy on held-out history
    │
    ▼
negative_control::run()     — verify signal is non-random (permutation test)
    │
    ▼
Ranked, validated PatternCandidates
```

## Modules

### `miner.rs`
Pure functions over in-memory `EventRecord` (`(entity_id, epoch_secs)`) slices.

| Symbol | Purpose |
|--------|---------|
| `MinerConfig` | Thresholds: `min_effect`, `max_p`, `min_stability`, `entity_min_count`, `time_splits` |
| `PatternCandidate` | Mined pattern: signal list, best lag, effect size, p-value, q-value, stability score, contingency 2×2 |
| `DEFAULT_WINDOW_DAYS` | 30 days (default observation window) |
| `MAX_SWEEP_LAG_DAYS` | 365 days (upper bound to prevent runaway sweeps B222) |

Multiple lags (1 day … `max_lag_days`) are swept; only the best lag per signal-outcome pair is retained.  Stability is measured by repeating the contingency test across `time_splits` non-overlapping windows and computing the fraction that pass the effect threshold.

### `hypothesis.rs`
Converts `PatternCandidate` lists into structured LLM prompts for analyst review.  Output is JSON-serialisable for traceability.

### `backtest.rs`
Walk-forward evaluation: split history at a cutoff, train on the left partition, score on the right.  Reports precision, recall, and lift vs. base-rate.

### `negative_control.rs`
Permutation test: shuffle the outcome labels N times and recompute the statistic.  The empirical p-value is `(# permutations ≥ observed) / N`.  Candidates that do not survive the permutation test are filtered out before hypothesis generation.

## Statistical guarantees

- **FDR control**: q-values are computed using the Benjamini-Hochberg method.
- **Stability filter**: a candidate must pass the effect threshold in ≥ `min_stability` fraction of time windows.
- **Entity coverage**: candidates covering fewer than `entity_min_count` entities are dropped.

## Threshold tuning guide

Use this order when tuning thresholds so changes are interpretable and reversible:

1. **Start with data volume**
    - Increase `entity_min_count` when candidate volume is noisy.
    - Decrease only when recall is too low and sample size is still statistically meaningful.

2. **Tune effect significance before stability**
    - `min_effect`: raise to reduce weak correlations.
    - `max_p`: lower to require stronger significance.
    - Keep these two stable for at least one backtest cycle before changing `min_stability`.

3. **Tune temporal sensitivity last**
    - `max_lag_days`: widen only if domain dynamics are truly slow-moving.
    - `window_days`: increase for long-cycle outcomes, decrease for faster reactions.

4. **Validate with walk-forward metrics**
    - Prefer parameter sets that improve both precision and recall over a single-metric gain.
    - Reject changes that only improve one fold while degrading aggregate F1.

Practical defaults for most deployments remain the current crate defaults (`MinerConfig::default`, `BacktestConfig::default`) unless domain-specific drift is observed.

## No database dependencies

All miner, backtest, and negative-control logic operates on typed vectors in memory.  The `store` crate handles loading and saving; `apex-learning` is purely computational and fully unit-testable without a running database.
