//! Worker crate — job scheduling, nightly and weekly pipelines.
//!
//! Provides a generic scheduler framework and typed job definitions for
//! the nightly (crawl + mine + POI refresh) and weekly (promotion board +
//! strategy memo) cycles, plus the self-improvement feedback loop.

pub mod scheduler;
pub mod nightly;
pub mod weekly;
pub mod self_improvement;
