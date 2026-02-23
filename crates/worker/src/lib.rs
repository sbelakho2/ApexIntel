//! Worker crate — job scheduling, nightly and weekly pipelines.
//!
//! Provides a generic scheduler framework and typed job definitions for
//! the nightly (crawl + mine + POI refresh) and weekly (promotion board +
//! strategy memo) cycles.

pub mod scheduler;
pub mod nightly;
pub mod weekly;
