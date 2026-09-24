//! AI Triage Engine — LLM-powered intelligent triage for insights, warnings, and alerts.
//!
//! This crate provides the full triage pipeline:
//! - [`config`] — Configuration loading from env / YAML
//! - [`scoring`] — LLM-based dimension scoring (urgency, impact, actionability, novelty, confidence)
//! - [`queue`] — DB-backed priority queue management
//! - [`history`] — Triage decision history and override tracking
//! - [`feedback_integration`] — Feedback loop with InsightFeedbackTracker
//! - [`router_integration`] — Real-time alert routing and SSE dispatch
//! - [`semantic_dedup`] — Embedding-based near-duplicate detection
//! - [`types`] — Re-exports from `apex_core::triage`

pub mod config;
pub mod feedback_integration;
pub mod history;
pub mod queue;
pub mod router_integration;
pub mod scoring;
pub mod semantic_dedup;
pub mod types;

pub use config::TriageConfig;
pub use queue::{QueueEnqueueRequest, TriageQueue};
pub use router_integration::TriageAlertRequest;
pub use scoring::TriageScorer;
pub use semantic_dedup::{
    IngestConfig, IngestOutcome, IngestQueue, IngestQueueItem, MergeReason, TriageIngestor,
    TriageSubmission,
};
