//! Construction of the LLM clients used by the worker runtime.

use apex_llm::{LlmClient, ModelConfig, OpenAiCompatibleClient};
use std::sync::Arc;
#[cfg(feature = "llm")]
pub(crate) fn build_quality_llm_client() -> Arc<dyn LlmClient> {
    let mut llm_config = ModelConfig::llamacpp_lightweight();
    if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
        llm_config.base_url = base_url;
    }
    if let Ok(model) = std::env::var("LLM_MODEL") {
        if !model.trim().is_empty() {
            llm_config.model_name = model;
        }
    }
    llm_config.api_key = std::env::var("LLM_API_KEY")
        .ok()
        .map(apex_llm::ApiKeySecret::from);
    Arc::new(OpenAiCompatibleClient::new(llm_config))
}
