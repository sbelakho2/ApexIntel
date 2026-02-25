# apex-stats

Statistical analysis, anomaly detection, Bayesian inference, and correlation utilities for the ApexIntel signal-processing pipeline.

## Modules

| Module | Responsibilities |
|--------|-----------------|
| `anomaly.rs` | `mad_zscore`, `ewma_control` — outlier detection; EWMA validated for α ∈ (0, 1] (B251/B252) |
| `bayesian.rs` | `fuse_signals` (log-odds fusion), `BetaUpdater` (Beta-Binomial posterior), `bayes_factor`, `interpret_bayes_factor` |
| `changepoint.rs` | `PeltConfig`, `detect_changepoints` — PELT algorithm for regime shift detection |
| `correlation.rs` | `lagged_xcorr` — time-lagged cross-correlation between signal vectors |
| `fdr.rs` | False Discovery Rate correction (Benjamini-Hochberg) |
| `fisher.rs` | Fisher's exact test for independence testing on 2×2 contingency tables |
| `graph_risk.rs` | Risk aggregation across supply-chain graph paths |
| `hazard.rs` | Discrete-time hazard and survival functions |
| `mutual_info.rs` | Histogram-based mutual information estimation |

## Key invariants

- All functions accept `&[f64]` slices — no owned allocations on input.
- Empty slices always return a valid (neutral) result, never panic.
- `ewma_control` with `alpha` outside `(0, 1]` or `sigma_mult ≤ 0` returns `vec![]` rather than producing nonsensical output.
- `BetaUpdater::total_observations()` counts only post-construction updates, not the prior.

## Test coverage

```bash
cargo test -p apex-stats --lib
# ~98 tests covering all branches including edge cases
```
