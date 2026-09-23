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
pub use crate::sources_registry::{all_sources, filter_by_tier, select_sources_for_crawl};
