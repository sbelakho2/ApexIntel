//! Crate-specific triage types and re-exports from `apex-core::triage`.
//!
//! The core types (`TriageDimensions`, `TriageQueueItem`, `TriageWeights`, etc.)
//! are defined in [`apex_core::triage`] to avoid circular dependencies.  This module
//! re-exports them and adds crate-local types.

pub use apex_core::triage::{
    composite_score, score_band_color, score_to_band, TriageDecision, TriageDecisionType,
    TriageDimensions, TriageItemType, TriageQueueItem, TriageStats, TriageStatus, TriageThresholds,
    TriageWeights,
};
