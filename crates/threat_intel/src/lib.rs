//! # ApexIntel Threat Intelligence Module
//!
//! Phase 3.2: Comprehensive threat intelligence capabilities for the ApexIntel OSINT platform.
//!
//! ## Modules
//!
//! - [`threat_actor_database`]: Known threat groups, attack patterns, TTPs, historical campaigns
//! - [`attack_surface`]: External exposure detection, vulnerability correlation, misconfiguration detection
//! - [`supply_chain_threats`]: Supplier risk scoring, geographic concentration, disruption planning
//! - [`competitive_intelligence`]: Market analysis, technology positioning, pricing intelligence

pub mod attack_surface;
pub mod competitive_intelligence;
pub mod supply_chain_threats;
pub mod threat_actor_database;

pub mod error;
pub mod mitre_attck;
pub mod models;

pub use error::{Result, ThreatIntelError};
pub use models::*;
