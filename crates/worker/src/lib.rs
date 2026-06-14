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

#![allow(clippy::should_implement_trait)]
#![allow(clippy::disallowed_methods)]

pub mod holiday_calendar;
pub mod nats_stream;
pub mod nightly;
pub mod notifications;
pub mod recipe_loader;
pub mod scheduler;
pub mod self_improvement;
pub mod slack;
pub mod sla_predictor;
pub mod storage;
pub mod embedding_indexer;
pub mod trend_aggregator;
pub mod pdf_export;
pub mod warning_verifier;
pub mod webhooks;
pub mod weekly;

// Re-export commonly used types for external callers
pub use recipe_loader::{load_default_seed_recipes, SeedRecipe};
pub use slack::{SlackConfig, SlackMessage, SlackMessageSeverity, SlackWebhook};
pub use storage::StorageContext;
