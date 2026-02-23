//! Pattern mining, candidate ranking, backtesting, and negative controls.
//!
//! Implements the stats-first pattern discovery loop:
//! 1. Mine candidate patterns from observation data (miner)
//! 2. Build LLM hypothesis prompts from candidates (hypothesis)
//! 3. Backtest candidates against historical data (backtest)
//! 4. Run negative controls to verify signal validity (negative_control)

pub mod miner;
pub mod hypothesis;
pub mod backtest;
pub mod negative_control;
