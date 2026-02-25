//! Pattern mining, candidate ranking, backtesting, and negative controls.
//!
//! Implements the stats-first pattern discovery loop:
//! 1. Mine candidate patterns from observation data (miner)
//! 2. Build LLM hypothesis prompts from candidates (hypothesis)
//! 3. Backtest candidates against historical data (backtest)
//! 4. Run negative controls to verify signal validity (negative_control)

pub mod miner;
pub mod backtest;

// ─────────────────────────────────────────────────────────────────────────────
// Experimental modules (B290)
//
// Compiled only when the `experimental` Cargo feature is active:
//   cargo build -p apex-learning --features experimental
//
// `hypothesis` — builds LLM prompt templates from mined pattern candidates.
//   Output quality is tightly coupled to the underlying model; prompts and
//   JSON schema evolve alongside model upgrades.
//
// `negative_control` — synthetic null-signal back-tests for FPR calibration.
//   The statistical methodology is under active refinement and is not yet
//   suitable for fully-automated promotion decisions without human review.
// ─────────────────────────────────────────────────────────────────────────────

/// LLM prompt template generation from mined pattern candidates.
///
/// Enable with `--features experimental`.
#[cfg(feature = "experimental")]
pub mod hypothesis;

/// Negative-control back-tests to calibrate false-positive rates.
///
/// Enable with `--features experimental`.
#[cfg(feature = "experimental")]
pub mod negative_control;
