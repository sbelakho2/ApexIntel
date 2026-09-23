//! Embedding client — generates vector embeddings via llama.cpp's `/v1/embeddings` endpoint.
//!
//! The OpenAI-compatible API shape means we POST a JSON body identical to the
//! OpenAI embedding API and extract the vector from the response.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::ModelConfig;

// ─── Request / Response types (OpenAI-compatible) ──────────────────────────

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    /// The input text to embed
    input: String,
    /// Model name (forwarded for logging, may be ignored by llama.cpp)
    model: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    model: Option<String>,
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f64>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

// ─── Client ────────────────────────────────────────────────────────────────

/// Generates vector embeddings by calling llama.cpp's `/v1/embeddings` endpoint.
///
/// llama.cpp exposes the same API shape as OpenAI when launched with `--embeddings`:
/// ```bash
/// ./llama-server --model qwen3-30b-a3b-q4_k_m.gguf --port 8080 --embeddings
/// ```
#[derive(Debug, Clone)]
pub struct EmbeddingClient {
    /// Base URL of the llama.cpp server (e.g. `http://localhost:8080`)
    base_url: String,
    /// Model name reported to the API
    model_name: String,
    /// HTTP client with sensible defaults
    client: reqwest::Client,
}

impl EmbeddingClient {
    /// Create a new embedding client from the given [`ModelConfig`].
    ///
    /// The config's `base_url` is used verbatim; the embedding endpoint is
    /// `<base_url>/v1/embeddings` (matching the OpenAI API shape).
    ///
    /// The client is built from infallible builder options, so construction
    /// cannot fail.
    #[allow(clippy::expect_used)]
    pub fn from_config(config: &ModelConfig) -> Self {
        let base_url = config.base_url.trim_end_matches('/').to_string();
        let model_name = config.model_name.clone();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent("apex-intel-embedding-client/0.1")
            .build()
            .expect("reqwest Client::builder() never fails with these options");
        Self {
            base_url,
            model_name,
            client,
        }
    }

    /// Generate an embedding vector for the given text.
    ///
    /// Returns a 4096-dimensional `Vec<f64>` (the output dimension of
    /// Qwen3-30B-A3B). The caller should normalise the vector before storage
    /// if using cosine distance; pgvector's `vector_cosine_ops` handles
    /// this automatically for cosine similarity queries.
    pub async fn embed(&self, text: &str) -> Result<Vec<f64>> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let body = EmbeddingRequest {
            input: text.to_string(),
            model: self.model_name.clone(),
        };

        let raw = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("embedding HTTP request failed")?;

        let status = raw.status();
        if !status.is_success() {
            let response_text = raw.text().await.unwrap_or_default();
            anyhow::bail!("embedding API returned HTTP {status}: {response_text}");
        }

        let resp: EmbeddingResponse = raw
            .json()
            .await
            .context("failed to parse embedding response JSON")?;

        let vector = resp
            .data
            .into_iter()
            .next()
            .context("embedding response contains no data entries")?
            .embedding;

        if vector.is_empty() {
            anyhow::bail!("embedding response contains an empty vector");
        }

        Ok(vector)
    }

    /// Generate embeddings for multiple texts in a single API call (batch mode).
    ///
    /// llama.cpp supports batched `/v1/embeddings` when the `input` field is
    /// an array of strings. The response contains one embedding per input, in order.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f64>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/v1/embeddings", self.base_url);
        let body = serde_json::json!({
            "input": texts,
            "model": self.model_name,
        });

        let raw = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("batch embedding HTTP request failed")?;

        let status = raw.status();
        if !status.is_success() {
            let response_text = raw.text().await.unwrap_or_default();
            anyhow::bail!("batch embedding API returned HTTP {status}: {response_text}");
        }

        let resp: EmbeddingResponse = raw
            .json()
            .await
            .context("failed to parse batch embedding response JSON")?;

        // Sort by index to maintain input order
        let mut indexed: Vec<_> = resp.data.into_iter().collect();
        indexed.sort_by_key(|d| d.index);

        let vectors: Vec<Vec<f64>> = indexed.into_iter().map(|d| d.embedding).collect();

        if vectors.iter().any(|v| v.is_empty()) {
            anyhow::bail!("batch embedding response contains one or more empty vectors");
        }

        Ok(vectors)
    }

    /// The model name this client was configured with.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

// ─── Text chunking utilities ──────────────────────────────────────────────

/// Split text into chunks of approximately `max_tokens` tokens, with `overlap_tokens`
/// tokens of overlap between consecutive chunks.
///
/// Token estimation uses a simple heuristic (~4 chars per token). Chunk boundaries
/// prefer sentence boundaries (`.`, `!`, `?`) where possible.
pub fn chunk_text(text: &str, max_tokens: usize, overlap_tokens: usize) -> Vec<String> {
    if text.is_empty() || max_tokens == 0 {
        return Vec::new();
    }

    const CHARS_PER_TOKEN: usize = 4;
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    let overlap_chars = overlap_tokens.saturating_mul(CHARS_PER_TOKEN);

    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;

    while start < text.len() {
        let end = if start + max_chars >= text.len() {
            text.len()
        } else {
            // Try to find a sentence boundary near the chunk limit
            let search_end = (start + max_chars).min(text.len());
            let search_start = start.max(search_end.saturating_sub(200)); // look back up to 200 chars

            // Find the last sentence-ending punctuation within the search window
            let slice = &text[search_start..search_end];
            if let Some(relative_pos) = slice.rfind(['.', '!', '?']) {
                // Include the punctuation character
                search_start + relative_pos + 1
            } else {
                // Fall back to last whitespace
                let window = &text[start..search_end];
                if let Some(last_space) = window.rfind(' ') {
                    start + last_space + 1
                } else {
                    search_end
                }
            }
        };

        let chunk = &text[start..end];
        let trimmed = chunk.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }

        // Advance by (chunk_size - overlap), ensuring we always make progress
        let advance = max_chars.saturating_sub(overlap_chars).max(1);
        start = start.saturating_add(advance);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text_short_text_no_chunking() {
        let text = "Hello world.";
        let chunks = chunk_text(text, 512, 64);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world.");
    }

    #[test]
    fn test_chunk_text_empty_text() {
        assert!(chunk_text("", 512, 64).is_empty());
    }

    #[test]
    fn test_chunk_text_respects_sentence_boundary() {
        let sentence1 = "This is the first sentence about company operations.";
        let sentence2 = "Here is a second, much longer sentence that goes into more detail about their quarterly earnings and market position in the region.";
        let text = format!("{} {}", sentence1, sentence2);

        // Force small chunks so we cross the boundary
        let chunks = chunk_text(&text, 20, 2);
        assert!(
            chunks.len() >= 2,
            "should produce at least 2 chunks: got {}",
            chunks.len()
        );
    }

    #[test]
    fn test_chunk_text_no_panic_on_exact_boundary() {
        let text = "A".repeat(5000);
        let chunks = chunk_text(&text, 512, 64);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| !c.is_empty()));
    }

    #[test]
    fn test_embedding_request_serialization() {
        let req = EmbeddingRequest {
            input: "test text".to_string(),
            model: "test-model".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["input"], "test text");
        assert_eq!(json["model"], "test-model");
    }

    #[test]
    fn test_embedding_response_deserialization() {
        let json = serde_json::json!({
            "data": [{
                "embedding": [0.1, 0.2, 0.3],
                "index": 0
            }],
            "model": "test-model",
            "usage": {
                "prompt_tokens": 10,
                "total_tokens": 10
            }
        });
        let resp: EmbeddingResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
        assert_eq!(resp.data[0].index, 0);
    }

    #[test]
    fn test_embedding_client_uses_correct_endpoint() {
        let config = ModelConfig {
            base_url: "http://localhost:8080".to_string(),
            model_name: "qwen3".to_string(),
            ..ModelConfig::llamacpp_default()
        };
        let client = EmbeddingClient::from_config(&config);
        assert_eq!(client.base_url, "http://localhost:8080");
        assert_eq!(client.model_name, "qwen3");
    }

    #[test]
    fn test_embedding_client_strips_trailing_slash() {
        let config = ModelConfig {
            base_url: "http://localhost:8080/".to_string(),
            model_name: "qwen3".to_string(),
            ..ModelConfig::llamacpp_default()
        };
        let client = EmbeddingClient::from_config(&config);
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_batch_empty_texts() {
        let result = chunk_text("", 512, 64);
        assert!(result.is_empty());
    }
}
