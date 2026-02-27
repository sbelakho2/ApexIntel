//! Worker crate — job scheduling, nightly and weekly pipelines.
//!
//! Provides a generic scheduler framework and typed job definitions for
//! the nightly (crawl + mine + POI refresh) and weekly (promotion board +
//! strategy memo) cycles, plus the self-improvement feedback loop.
//!
//! # Storage Integration
//!
//! The `storage` module provides functions to build pipeline inputs from
//! the actual database, enabling real production operation. When `DATABASE_URL`
//! is set, the worker will query the database for stage inputs instead of
//! reading from JSON stub files.

pub mod scheduler;
pub mod nightly;
pub mod weekly;
pub mod self_improvement;
pub mod storage;

// Re-export commonly used types for external callers
pub use storage::StorageContext;
