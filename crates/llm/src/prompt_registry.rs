use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKey {
    EntityExtraction,
    RecipeHypothesis,
    PoiSynthesis,
    MemoGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredPrompt {
    pub workflow: &'static str,
    pub prompt_id: &'static str,
    pub version: &'static str,
    pub system_prompt: &'static str,
}

impl RegisteredPrompt {
    pub fn metadata_value(&self) -> serde_json::Value {
        serde_json::json!({
            "workflow": self.workflow,
            "prompt_id": self.prompt_id,
            "version": self.version,
        })
    }
}

pub fn workflow_prompt(key: PromptKey) -> RegisteredPrompt {
    match key {
        PromptKey::EntityExtraction => RegisteredPrompt {
            workflow: "entity_extraction",
            prompt_id: "entity_extraction_osint",
            version: "v1",
            system_prompt: "You are an OSINT analyst. Extract named entities from the user text.\nReturn JSON ONLY with field 'entities' as an array of objects: {name, entity_type, confidence (0-1), span_start, span_end, canonical}.\nUse null for unknown spans. Confidence must be between 0 and 1.\nDo not output schema field names as entities. Only real entities from the text.\nExample output:\n{\"entities\":[{\"name\":\"Starz Electronics\",\"entity_type\":\"company\",\"confidence\":0.9,\"span_start\":0,\"span_end\":17,\"canonical\":\"Starz Electronics\"},{\"name\":\"Tangier\",\"entity_type\":\"location\",\"confidence\":0.8,\"span_start\":33,\"span_end\":40,\"canonical\":\"Tangier\"}]}.",
        },
        PromptKey::RecipeHypothesis => RegisteredPrompt {
            workflow: "recipe_hypothesis",
            prompt_id: "recipe_hypothesis_osint",
            version: "v1",
            system_prompt: "You are an OSINT analyst generating detection recipes.\nReturn JSON ONLY with fields: id, signals, narrative_template, action_playbook.\nFields: id is a unique snake_case string, signals is a non-empty array.\nSignals must be objects with at least {name, description}.\nnarrative_template and action_playbook must be non-empty strings.",
        },
        PromptKey::PoiSynthesis => RegisteredPrompt {
            workflow: "poi_synthesis",
            prompt_id: "poi_dossier_synthesis",
            version: "v1",
            system_prompt: "You are an OSINT analyst building POI dossiers.\nReturn JSON ONLY with fields: summary, roles, affiliations, key_facts, risk_indicators.\nAll fields required; arrays must be non-empty.",
        },
        PromptKey::MemoGeneration => RegisteredPrompt {
            workflow: "memo_generation",
            prompt_id: "strategic_memo_generation",
            version: "v1",
            system_prompt: "You are an intelligence analyst writing concise strategic memos.\nReturn JSON ONLY with fields: title, executive_summary, sections, recommendations.\nsections is an array of {heading, content}. recommendations is an array of strings.",
        },
    }
}
