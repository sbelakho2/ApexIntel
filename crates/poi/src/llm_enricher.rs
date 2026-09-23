//! LLM-powered enrichment for Person-of-Interest profiles.
//!
//! Wraps [`apex_llm::poi_profiler::PoiProfiler`] with POI-domain context,
//! merging LLM-inferred psychometric data with the existing [`PoiProfile`]
//! and providing ready-to-use engagement copy.
//!
//! # Design
//! - All methods are `async` (HTTP to the local llama-server).
//! - Errors are returned as `anyhow::Result`; callers decide whether to fall back
//!   to the rule-based profile from `crate::features`.
//! - Deterministic rule-based data is always computed first; LLM output is
//!   **additive** — it never overwrites existing high-confidence fields.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use apex_llm::{
    inference::LlmClient,
    poi_profiler::{EngagementCopy, LlmPsychProfile, PoiBackgroundSummary, PoiProfiler},
};

// ────────────────────────────────────────────
// Enriched output types
// ────────────────────────────────────────────

/// A fully LLM-enriched POI profile.
///
/// This is separate from the core [`PoiProfile`] to avoid tight coupling
/// between the llm crate and the data model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedPoiProfile {
    /// POI identifier (UUID string).
    pub poi_id: String,
    /// Display name.
    pub name: String,
    /// LLM-inferred psychometric / behavioural profile.
    pub psych_profile: Option<LlmPsychProfile>,
    /// Tailored engagement copy for sales/business-development outreach.
    pub engagement_copy: Option<EngagementCopy>,
    /// Synthesised background narrative.
    pub background_summary: Option<PoiBackgroundSummary>,
    /// Whether LLM enrichment succeeded for at least one stage.
    pub llm_enriched: bool,
}

/// Compact batch input tuple for POI enrichment.
pub type BatchPoiInput<'a> = (&'a str, &'a str, &'a str, &'a str, Vec<&'a str>, &'a str);

impl EnrichedPoiProfile {
    /// Construct a bare (unenriched) profile.
    pub fn bare(poi_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            poi_id: poi_id.into(),
            name: name.into(),
            psych_profile: None,
            engagement_copy: None,
            background_summary: None,
            llm_enriched: false,
        }
    }
}

// ────────────────────────────────────────────
// Enricher
// ────────────────────────────────────────────

/// High-level LLM enricher for POI profiles.
pub struct PoiLlmEnricher {
    profiler: PoiProfiler,
}

impl PoiLlmEnricher {
    /// Build from environment variables.
    pub fn from_env() -> Result<Self> {
        let client = LlmClient::from_env()?;
        Ok(Self {
            profiler: PoiProfiler::new(client),
        })
    }

    /// Build with an explicit client.
    pub fn new(client: LlmClient) -> Self {
        Self {
            profiler: PoiProfiler::new(client),
        }
    }

    // ── Full enrichment ────────────────────────────────────────────

    /// Fully enrich a POI profile through all three LLM stages:
    /// 1. Psychometric profiling
    /// 2. Engagement copy generation (requires successful psych profile)
    /// 3. Background narrative synthesis
    ///
    /// `artifacts` — `(source_type, text_content)` pairs collected for this POI.
    /// `product_or_service` — what ApexIntel offers (used in engagement copy).
    /// Individual stage failures are logged but do not abort the others.
    pub async fn enrich_full(
        &self,
        poi_id: &str,
        name: &str,
        role: &str,
        company: &str,
        artifacts: &[(String, String)],
        product_or_service: &str,
    ) -> EnrichedPoiProfile {
        let mut profile = EnrichedPoiProfile::bare(poi_id, name);

        // Stage 1: psychometric profile
        let psych = self
            .profiler
            .infer_psych_profile(name, role, artifacts)
            .await;
        match psych {
            Ok(p) => {
                debug!(poi=%poi_id, "Psych profile inferred");
                profile.psych_profile = Some(p);
                profile.llm_enriched = true;
            }
            Err(e) => warn!(poi=%poi_id, error=%e, "Psych profile inference failed"),
        }

        // Stage 2: engagement copy (uses inferred psych profile if available)
        if let Some(ref psych_profile) = profile.psych_profile {
            let copy = self
                .profiler
                .generate_engagement_copy(
                    name,
                    role,
                    company,
                    psych_profile,
                    product_or_service,
                    None,
                )
                .await;
            match copy {
                Ok(c) => {
                    debug!(poi=%poi_id, "Engagement copy generated");
                    profile.engagement_copy = Some(c);
                }
                Err(e) => warn!(poi=%poi_id, error=%e, "Engagement copy generation failed"),
            }
        }

        // Stage 3: background synthesis
        let bg = self
            .profiler
            .synthesize_background(name, role, artifacts)
            .await;
        match bg {
            Ok(b) => {
                debug!(poi=%poi_id, "Background synthesised");
                profile.background_summary = Some(b);
                profile.llm_enriched = true;
            }
            Err(e) => warn!(poi=%poi_id, error=%e, "Background synthesis failed"),
        }

        profile
    }

    /// Infer psychometric profile only.
    ///
    /// * `artifacts` — `(source_type, text_content)` pairs.
    pub async fn infer_psych_profile(
        &self,
        name: &str,
        role: &str,
        artifacts: &[(String, String)],
    ) -> Result<LlmPsychProfile> {
        self.profiler
            .infer_psych_profile(name, role, artifacts)
            .await
    }

    /// Generate engagement copy only.
    ///
    /// * `psych_profile` — must be obtained first via `infer_psych_profile`.
    /// * `product_or_service` — what to pitch.
    /// * `recent_news` — optional recent news snippet.
    pub async fn generate_engagement_copy(
        &self,
        name: &str,
        role: &str,
        company: &str,
        psych_profile: &LlmPsychProfile,
        product_or_service: &str,
        recent_news: Option<&str>,
    ) -> Result<EngagementCopy> {
        self.profiler
            .generate_engagement_copy(
                name,
                role,
                company,
                psych_profile,
                product_or_service,
                recent_news,
            )
            .await
    }

    /// Synthesise background narrative only.
    ///
    /// * `artifacts` — `(source_type, text_content)` pairs.
    pub async fn synthesize_background(
        &self,
        name: &str,
        role: &str,
        artifacts: &[(String, String)],
    ) -> Result<PoiBackgroundSummary> {
        self.profiler
            .synthesize_background(name, role, artifacts)
            .await
    }

    // ── Batch enrichment ───────────────────────────────────────────

    /// Enrich a batch of POIs sequentially (to avoid overloading the local LLM).
    ///
    /// Each element is `(poi_id, name, title, company, bio_snippets, product_or_service)`.
    /// `bio_snippets` are plain-text strings tagged as `"bio"` source type internally.
    pub async fn enrich_batch(&self, pois: &[BatchPoiInput<'_>]) -> Vec<EnrichedPoiProfile> {
        let mut results = Vec::with_capacity(pois.len());
        for (id, name, title, company, bio, product) in pois {
            // Convert &str bio snippets to (source_type, content) pairs
            let artifacts: Vec<(String, String)> = bio
                .iter()
                .map(|s| ("bio".to_string(), s.to_string()))
                .collect();
            let enriched = self
                .enrich_full(id, name, title, company, &artifacts, product)
                .await;
            results.push(enriched);
        }
        results
    }
}

// ────────────────────────────────────────────
// Unit tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::assertions_on_constants
    )]

    use super::*;

    #[test]
    fn bare_profile_not_enriched() {
        let p = EnrichedPoiProfile::bare("uuid-1", "Jane Doe");
        assert!(!p.llm_enriched);
        assert!(p.psych_profile.is_none());
    }

    #[test]
    fn enriched_profile_serializes() {
        let p = EnrichedPoiProfile {
            poi_id: "uuid-2".to_string(),
            name: "John Smith".to_string(),
            psych_profile: None,
            engagement_copy: None,
            background_summary: None,
            llm_enriched: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("John Smith"));
    }
}
