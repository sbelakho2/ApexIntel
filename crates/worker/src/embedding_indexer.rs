//! Embedding indexer — generates vector embeddings for entities and persists them to pgvector.
//!
//! Supports both initial backfill (all entities) and incremental indexing (entities
//! that lack embeddings). The nightly rebuild job truncates and re-embeds all entities
//! to ensure consistency when the embedding model changes.
//!
//! # Multi-language support (Phase 2.4)
//!
//! The indexer detects the language of each entity's source text and selects
//! an appropriate chunking strategy per language.  Language distribution is
//! logged in the indexing stats for monitoring.

use std::sync::Arc;

use anyhow::Result;
use apex_llm::embeddings::{chunk_text, EmbeddingClient};
use apex_store::postgres::PgStore;
use tracing::{error, info, warn};

use crate::scheduler::{JobKind, JobRun};

/// Maximum number of entities to process per indexing run (avoids unbounded loops).
const MAX_ENTITIES_PER_RUN: usize = 500;

/// Maximum chunk size in tokens (Qwen3-30B-A3B supports up to 32K context, but
/// embedding quality is best with focused chunks).
const CHUNK_MAX_TOKENS: usize = 512;

/// Overlap between consecutive chunks (sentence-boundary-aware).
const CHUNK_OVERLAP_TOKENS: usize = 64;

/// Languages that use CJK characters (Chinese, Japanese, Korean) — require
/// smaller chunk sizes because each character carries more semantic density
/// and tokenizers tend to split CJK into 1–2 tokens per character.
const CJK_LANGUAGES: &[&str] = &["zh", "ja", "ko"];

/// Languages written in Arabic script (Arabic, Persian, Urdu) — chunk at
/// standard size but mark for RTL-aware processing.
#[allow(dead_code)]
const RTL_LANGUAGES: &[&str] = &["ar", "fa", "he"];

/// Entity types that support embedding indexing, in priority order.
const INDEXED_ENTITY_TYPES: &[&str] = &["company", "person", "insight", "warning", "observation"];

// ─── Public entry point ────────────────────────────────────────────────────

/// Run the embedding reindex job.
///
/// This job is scheduled nightly at 03:00 UTC. It performs incremental indexing:
/// finds entities without embeddings and generates them.
pub async fn run_embedding_reindex(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let embedding_client = match create_embedding_client() {
        Ok(client) => client,
        Err(e) => {
            error!("failed to create embedding client: {e}");
            run.fail(&format!("failed to create embedding client: {e}"));
            return run;
        }
    };

    let model_name = embedding_client.model_name().to_string();
    let mut total_indexed: u64 = 0;
    let mut total_skipped: u64 = 0;
    let mut errors: Vec<String> = Vec::new();
    // Multi-language tracking (Phase 2.4)
    let mut lang_distribution: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for entity_type in INDEXED_ENTITY_TYPES {
        // Check if we've hit the per-run limit
        if total_indexed >= MAX_ENTITIES_PER_RUN as u64 {
            info!(
                "embedding indexer: reached per-run limit of {MAX_ENTITIES_PER_RUN}, stopping"
            );
            break;
        }

        let remaining = (MAX_ENTITIES_PER_RUN as u64).saturating_sub(total_indexed);
        let entity_ids = match store
            .entities_missing_embeddings(entity_type, remaining as i64)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                warn!("failed to query {entity_type} entities for indexing: {e}");
                errors.push(format!("{entity_type}: query failed: {e}"));
                continue;
            }
        };

        if entity_ids.is_empty() {
            info!("embedding indexer: no {entity_type} entities need indexing");
            continue;
        }

        info!(
            "embedding indexer: indexing {} {entity_type} entities",
            entity_ids.len()
        );

        for entity_id in &entity_ids {
            if total_indexed >= MAX_ENTITIES_PER_RUN as u64 {
                break;
            }

            match index_single_entity(store, &embedding_client, entity_type, entity_id, &model_name)
                .await
            {
                Ok(Some(lang)) => {
                    total_indexed += 1;
                    // Track language distribution (Phase 2.4)
                    *lang_distribution.entry(lang).or_insert(0) += 1;
                }
                Ok(None) => {
                    total_skipped += 1;
                }
                Err(e) => {
                    warn!("failed to index {entity_type}/{entity_id}: {e}");
                    errors.push(format!("{entity_type}/{entity_id}: {e}"));
                }
            }
        }
    }

    // Build language distribution summary
    let lang_summary: String = {
        let mut langs: Vec<_> = lang_distribution.into_iter().collect();
        langs.sort_by(|a, b| b.1.cmp(&a.1));
        langs.iter().map(|(lang, count)| format!("{lang}:{count}")).collect::<Vec<_>>().join(", ")
    };

    let notes = if errors.is_empty() {
        format!(
            "indexed {total_indexed} entities (skipped {total_skipped}) using {model_name} | langs: [{lang_summary}]"
        )
    } else {
        format!(
            "indexed {total_indexed} entities (skipped {total_skipped}) with {} errors: {} | langs: [{lang_summary}]",
            errors.len(),
            errors.join("; ")
        )
    };

    if errors.is_empty() && total_indexed > 0 {
        run.succeed(total_indexed, &notes);
    } else if errors.is_empty() && total_indexed == 0 {
        run.skip("no entities needed indexing");
    } else {
        run.fail(&notes);
    }

    run
}

// ─── Full re-index (truncate + rebuild) ────────────────────────────────────

/// Truncate all embeddings for the configured model and re-index every entity.
///
/// This is the "full rebuild" operation used when the model changes or on manual
/// trigger. Returns the total number of chunks indexed.
pub async fn run_full_reindex(
    store: &Arc<PgStore>,
    embedding_client: &EmbeddingClient,
) -> Result<u64> {
    let model_name = embedding_client.model_name();

    info!("embedding full reindex: deleting existing embeddings for model {model_name}");
    let deleted = store.delete_embeddings_by_model(model_name).await?;
    info!("embedding full reindex: deleted {deleted} existing embeddings");

    let mut total_chunks: u64 = 0;

    for entity_type in INDEXED_ENTITY_TYPES {
        // Get all entity IDs for this type (no limit for full reindex)
        let entity_ids = store
            .entities_missing_embeddings(entity_type, 100_000)
            .await?;

        info!(
            "embedding full reindex: indexing {} {entity_type} entities",
            entity_ids.len()
        );

        for entity_id in &entity_ids {
            match index_single_entity(store, embedding_client, entity_type, entity_id, model_name)
                .await
            {
                Ok(Some(_lang)) => {
                    total_chunks += 1;
                }
                Ok(None) => {} // skipped, no text available
                Err(e) => {
                    warn!("full reindex: failed for {entity_type}/{entity_id}: {e}");
                }
            }
        }
    }

    info!("embedding full reindex: completed with {total_chunks} chunks indexed");
    Ok(total_chunks)
}

// ─── Internal helpers ──────────────────────────────────────────────────────

/// Index a single entity: fetch source text, chunk it, generate embeddings, store them.
///
/// Returns `Ok(Some(lang))` with the detected language if at least one chunk was indexed,
/// `Ok(None)` if skipped (no source text), or `Err` on failure.
async fn index_single_entity(
    store: &PgStore,
    client: &EmbeddingClient,
    entity_type: &str,
    entity_id: &str,
    model_name: &str,
) -> Result<Option<String>> {
    let source_text = match store.entity_source_text(entity_type, entity_id).await? {
        Some(text) => text,
        None => {
            warn!("no source text for {entity_type}/{entity_id}, skipping");
            return Ok(None);
        }
    };

    let trimmed = source_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // Phase 2.4: Detect language for adaptive chunking
    let detected_lang = apex_parse::multilingual::detect_language(trimmed);

    // Adaptive chunking: CJK languages use smaller chunks; RTL languages use
    // standard chunking but the embedding model handles them natively.
    let (chunk_max, chunk_overlap) = if CJK_LANGUAGES.contains(&detected_lang.as_str()) {
        // CJK text is denser: 256 tokens per chunk, 32 token overlap
        (CHUNK_MAX_TOKENS / 2, CHUNK_OVERLAP_TOKENS / 2)
    } else {
        (CHUNK_MAX_TOKENS, CHUNK_OVERLAP_TOKENS)
    };

    let chunks = chunk_text(trimmed, chunk_max, chunk_overlap);
    if chunks.is_empty() {
        return Ok(None);
    }

    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let embedding = client.embed(chunk).await?;
        store
            .upsert_embedding(entity_type, entity_id, chunk_index as i32, &embedding, chunk, model_name)
            .await?;
    }

    Ok(Some(detected_lang))
}

/// Create an [`EmbeddingClient`] from environment configuration.
///
/// Expects `LLM_BASE_URL` (default: `http://localhost:8080`) and
/// `LLM_MODEL_NAME` (default: `Qwen3-30B-A3B-Q4_K_M`).
fn create_embedding_client() -> Result<EmbeddingClient> {
    let base_url = std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let model_name = std::env::var("LLM_MODEL_NAME").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".to_string());

    let mut config = apex_llm::ModelConfig::llamacpp_default();
    config.base_url = base_url;
    config.model_name = model_name;

    Ok(EmbeddingClient::from_config(&config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_embedding_client_env_vars() {
        // Test default fallback (env vars unset)
        std::env::remove_var("LLM_BASE_URL");
        std::env::remove_var("LLM_MODEL_NAME");
        let client = create_embedding_client().unwrap();
        assert!(
            !client.model_name().is_empty(),
            "model name should not be empty even without env vars"
        );

        // Test custom env vars
        std::env::set_var("LLM_BASE_URL", "http://llama:8080");
        std::env::set_var("LLM_MODEL_NAME", "custom-model");
        let client2 = create_embedding_client().unwrap();
        assert_eq!(client2.model_name(), "custom-model");
    }

    #[test]
    fn test_indexed_entity_types_include_all() {
        assert!(INDEXED_ENTITY_TYPES.contains(&"company"));
        assert!(INDEXED_ENTITY_TYPES.contains(&"person"));
        assert!(INDEXED_ENTITY_TYPES.contains(&"insight"));
        assert!(INDEXED_ENTITY_TYPES.contains(&"warning"));
        assert!(INDEXED_ENTITY_TYPES.contains(&"observation"));
        assert_eq!(INDEXED_ENTITY_TYPES.len(), 5);
    }
}
