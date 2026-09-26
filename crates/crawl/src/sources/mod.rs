//! OSINT Source Modules
//!
//! Domain-specific intelligence collection modules covering social media
//! (LinkedIn, Twitter/X, forums, executive tracking), financial data,
//! geopolitical feeds, technical/infrastructure sources, and dark-web
//! monitoring.  Each submodule provides typed models and HTTP clients
//! for its respective data domain.

pub mod dark_web;
pub mod financial;
pub mod geopolitical;
pub mod social_intel;
pub mod social_media;
pub mod technical;

// Re-export types from sources_registry for convenience
pub use crate::sources_registry::{Category, Region, Source};

// Re-export helper functions used by worker
pub use crate::sources_registry::{
    all_sources, coverage_debt_remaining, dispatch_source_fetch, effective_capability,
    filter_by_tier, is_source_due, select_due_sources, source_coverage_summary, ApiAdapter,
    DeploymentCapabilities, FetchDispatch, FetchDispatchError, FetchStrategy, SourceCapability,
    SourceCoverageSummary, SourceRuntimeStateProvider, SourceScheduleCandidate, SourceSelection,
    CREDENTIALED_API_ADAPTERS_ENV, FORCED_SOURCE_SLUGS,
};
