//! Deterministic-first, tiered model routing (audit P0 #24).
//!
//! Not every task deserves the 30B model. The pipeline is routed in tiers:
//!
//! | Workflow                        | Tier           | Rationale |
//! |---------------------------------|----------------|-----------|
//! | `rss_html_parse`                | Deterministic  | Parser, never an LLM |
//! | `entity_alias`                  | Deterministic  | Alias table first; embeddings only as fallback |
//! | `dedup_rerank`                  | Embedding      | Cosine similarity, no completion model |
//! | `classification`                | Small model    | Short, bounded labels |
//! | `triage_dimensions`             | Small model    | Structured scoring dimensions |
//! | `final_synthesis`               | Large model    | Cross-evidence reasoning |
//! | `battlecard`                    | Large model    | External-facing prose |
//! | `executive_memo`                | Large model    | External-facing prose |
//!
//! Model names come from the environment (`LLM_SMALL_MODEL`, `LLM_LARGE_MODEL`;
//! `LLM_MODEL` is accepted as the large-model fallback for backwards
//! compatibility) with sane defaults for the local llama.cpp deployment.

/// A unit of LLM-adjacent work in the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Workflow {
    RssHtmlParse,
    EntityAlias,
    Classification,
    TriageDimensions,
    DedupRerank,
    FinalSynthesis,
    Battlecard,
    ExecutiveMemo,
}

impl Workflow {
    pub const ALL: [Workflow; 8] = [
        Workflow::RssHtmlParse,
        Workflow::EntityAlias,
        Workflow::Classification,
        Workflow::TriageDimensions,
        Workflow::DedupRerank,
        Workflow::FinalSynthesis,
        Workflow::Battlecard,
        Workflow::ExecutiveMemo,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Workflow::RssHtmlParse => "rss_html_parse",
            Workflow::EntityAlias => "entity_alias",
            Workflow::Classification => "classification",
            Workflow::TriageDimensions => "triage_dimensions",
            Workflow::DedupRerank => "dedup_rerank",
            Workflow::FinalSynthesis => "final_synthesis",
            Workflow::Battlecard => "battlecard",
            Workflow::ExecutiveMemo => "executive_memo",
        }
    }
}

/// Execution tier for a workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Pure Rust parsing/normalisation; no model and no embedding call.
    Deterministic,
    /// Vector embeddings only; no completion model.
    Embedding,
    /// Small local completion model.
    SmallModel,
    /// Large local completion model (the 30B).
    LargeModel,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Deterministic => "deterministic",
            Tier::Embedding => "embedding",
            Tier::SmallModel => "small_model",
            Tier::LargeModel => "large_model",
        }
    }
}

/// The execution tier for a workflow. This is the routing contract the
/// pipeline (and its tests) rely on.
pub fn route_tier(workflow: Workflow) -> Tier {
    match workflow {
        Workflow::RssHtmlParse | Workflow::EntityAlias => Tier::Deterministic,
        Workflow::DedupRerank => Tier::Embedding,
        Workflow::Classification | Workflow::TriageDimensions => Tier::SmallModel,
        Workflow::FinalSynthesis | Workflow::Battlecard | Workflow::ExecutiveMemo => {
            Tier::LargeModel
        }
    }
}

/// True when the workflow may consult embeddings (hybrid deterministic work).
pub fn uses_embeddings(workflow: Workflow) -> bool {
    matches!(workflow, Workflow::EntityAlias | Workflow::DedupRerank)
}

/// True when the workflow invokes a completion model at all.
pub fn uses_completion_model(workflow: Workflow) -> bool {
    matches!(route_tier(workflow), Tier::SmallModel | Tier::LargeModel)
}

/// Configured model names per tier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TieredModels {
    pub small_model: String,
    pub large_model: String,
}

impl Default for TieredModels {
    fn default() -> Self {
        Self {
            small_model: "Qwen3-4B-Instruct-2507-Q4_K_M".to_string(),
            large_model: "Qwen3-30B-A3B-Q4_K_M".to_string(),
        }
    }
}

impl TieredModels {
    pub fn new(small_model: impl Into<String>, large_model: impl Into<String>) -> Self {
        Self {
            small_model: small_model.into(),
            large_model: large_model.into(),
        }
    }

    /// Read model names from the environment, falling back to defaults.
    ///
    /// `LLM_MODEL` is honoured for the large tier when `LLM_LARGE_MODEL` is
    /// unset, so pre-existing deployments keep their configured model.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let small_model = non_empty_env("LLM_SMALL_MODEL").unwrap_or(defaults.small_model);
        let large_model = non_empty_env("LLM_LARGE_MODEL")
            .or_else(|| non_empty_env("LLM_MODEL"))
            .unwrap_or(defaults.large_model);
        Self {
            small_model,
            large_model,
        }
    }

    /// The model to use for a workflow, or `None` when no completion model is
    /// needed (deterministic/embedding tiers).
    pub fn model_for(&self, workflow: Workflow) -> Option<&str> {
        match route_tier(workflow) {
            Tier::SmallModel => Some(self.small_model.as_str()),
            Tier::LargeModel => Some(self.large_model.as_str()),
            Tier::Deterministic | Tier::Embedding => None,
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workflow_routes_to_its_configured_tier() {
        assert_eq!(route_tier(Workflow::RssHtmlParse), Tier::Deterministic);
        assert_eq!(route_tier(Workflow::EntityAlias), Tier::Deterministic);
        assert_eq!(route_tier(Workflow::Classification), Tier::SmallModel);
        assert_eq!(route_tier(Workflow::TriageDimensions), Tier::SmallModel);
        assert_eq!(route_tier(Workflow::DedupRerank), Tier::Embedding);
        assert_eq!(route_tier(Workflow::FinalSynthesis), Tier::LargeModel);
        assert_eq!(route_tier(Workflow::Battlecard), Tier::LargeModel);
        assert_eq!(route_tier(Workflow::ExecutiveMemo), Tier::LargeModel);
    }

    #[test]
    fn model_for_routes_tasks_to_small_or_large_or_no_model() {
        let models = TieredModels::new("small-x", "large-y");
        assert_eq!(models.model_for(Workflow::Classification), Some("small-x"));
        assert_eq!(
            models.model_for(Workflow::TriageDimensions),
            Some("small-x")
        );
        assert_eq!(models.model_for(Workflow::FinalSynthesis), Some("large-y"));
        assert_eq!(models.model_for(Workflow::Battlecard), Some("large-y"));
        assert_eq!(models.model_for(Workflow::ExecutiveMemo), Some("large-y"));
        assert_eq!(models.model_for(Workflow::RssHtmlParse), None);
        assert_eq!(models.model_for(Workflow::EntityAlias), None);
        assert_eq!(models.model_for(Workflow::DedupRerank), None);
    }

    #[test]
    fn deterministic_and_embedding_work_never_requires_a_completion_model() {
        for workflow in Workflow::ALL {
            if uses_completion_model(workflow) {
                assert!(matches!(
                    route_tier(workflow),
                    Tier::SmallModel | Tier::LargeModel
                ));
            } else {
                assert!(TieredModels::default().model_for(workflow).is_none());
            }
        }
        assert!(!uses_embeddings(Workflow::RssHtmlParse));
        assert!(uses_embeddings(Workflow::EntityAlias));
        assert!(uses_embeddings(Workflow::DedupRerank));
    }

    #[test]
    fn workflow_names_are_stable_cache_keys() {
        assert_eq!(Workflow::FinalSynthesis.as_str(), "final_synthesis");
        assert_eq!(Workflow::RssHtmlParse.as_str(), "rss_html_parse");
        let mut names: Vec<&str> = Workflow::ALL.iter().map(|w| w.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Workflow::ALL.len());
    }
}
