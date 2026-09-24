//! LLM work cache (audit P0 #24).
//!
//! Identical inference must never re-run: the cache key is the SHA-256 of the
//! exact inputs that determine an output — workflow, model version, prompt
//! version, and the sorted evidence ids. Unchanged evidence hits the cache;
//! changed evidence misses and re-runs the model once.
//!
//! The trait is storage-agnostic. [`MemoryLlmCache`] serves tests and
//! single-process use; the Postgres-backed implementation lives in
//! `apex-store` (`PgStore::get_llm_cache` / `put_llm_cache`), keeping this
//! crate free of database dependencies.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::{LlmClient, Result};

/// Deterministic cache key for an LLM request.
///
/// Evidence ids are sorted and deduplicated so the same evidence set always
/// produces the same key regardless of iteration order. The four inputs are
/// domain-separated while hashing to avoid ambiguity between fields.
pub fn cache_key(
    workflow: &str,
    model_version: &str,
    prompt_version: &str,
    evidence_ids: &[String],
) -> String {
    let mut sorted: Vec<&str> = evidence_ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();

    let mut hasher = Sha256::new();
    for part in [workflow, model_version, prompt_version] {
        hasher.update(part.as_bytes());
        hasher.update(b"\x1f"); // unit separator
    }
    for id in sorted {
        hasher.update(id.as_bytes());
        hasher.update(b"\x1e"); // record separator
    }
    format!("{:x}", hasher.finalize())
}

/// Storage for cached LLM responses.
#[async_trait]
pub trait LlmCache: Send + Sync {
    /// Return the cached response for `key`, if present.
    async fn get(&self, key: &str) -> Option<String>;

    /// Store `response` under `key`. Existing entries are left untouched.
    async fn put(&self, key: &str, workflow: &str, response: &str);
}

/// In-process cache used by tests and as a fallback.
#[derive(Default)]
pub struct MemoryLlmCache {
    entries: Mutex<HashMap<String, String>>,
}

impl MemoryLlmCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        match self.entries.lock() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl LlmCache for MemoryLlmCache {
    async fn get(&self, key: &str) -> Option<String> {
        match self.entries.lock() {
            Ok(guard) => guard.get(key).cloned(),
            Err(poisoned) => poisoned.into_inner().get(key).cloned(),
        }
    }

    async fn put(&self, key: &str, _workflow: &str, response: &str) {
        let mut guard = match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .entry(key.to_string())
            .or_insert_with(|| response.to_string());
    }
}

/// An [`LlmClient`] wrapper that serves identical requests from a cache.
///
/// On a hit the inner client is **not** called; on a miss the response is
/// stored under the deterministic key before being returned.
pub struct CachedLlmClient {
    inner: Arc<dyn LlmClient>,
    cache: Arc<dyn LlmCache>,
    workflow: String,
    model_version: String,
    prompt_version: String,
    evidence_ids: Vec<String>,
}

impl CachedLlmClient {
    pub fn new(
        inner: Arc<dyn LlmClient>,
        cache: Arc<dyn LlmCache>,
        workflow: &str,
        model_version: impl Into<String>,
        prompt_version: impl Into<String>,
        evidence_ids: Vec<String>,
    ) -> Self {
        Self {
            inner,
            cache,
            workflow: workflow.to_string(),
            model_version: model_version.into(),
            prompt_version: prompt_version.into(),
            evidence_ids,
        }
    }

    /// Cache key for one call variant (`json`/`text`) — the variant is part of
    /// the workflow domain string so JSON and free-text outputs never collide.
    pub fn key_for(&self, variant: &str) -> String {
        cache_key(
            &format!("{}:{variant}", self.workflow),
            &self.model_version,
            &self.prompt_version,
            &self.evidence_ids,
        )
    }

    async fn cached_or_generate<F, Fut>(&self, variant: &str, generate: F) -> Result<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<String>>,
    {
        let key = self.key_for(variant);
        if let Some(cached) = self.cache.get(&key).await {
            return Ok(cached);
        }
        let response = generate().await?;
        self.cache
            .put(&key, &format!("{}:{variant}", self.workflow), &response)
            .await;
        Ok(response)
    }
}

#[async_trait]
impl LlmClient for CachedLlmClient {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
        self.cached_or_generate("json", || self.inner.generate_json(system, user))
            .await
    }

    async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
        self.cached_or_generate("text", || self.inner.generate_text(system, user))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingLlmClient {
        calls: AtomicUsize,
        response: String,
    }

    impl CountingLlmClient {
        fn new(response: &str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: response.to_string(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LlmClient for CountingLlmClient {
        async fn generate_json(&self, _system: &str, _user: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }

        async fn generate_text(&self, _system: &str, _user: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn evidence(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[tokio::test]
    async fn cache_hit_avoids_the_model_call() {
        let inner = Arc::new(CountingLlmClient::new(r#"{"answer":42}"#));
        let cache = Arc::new(MemoryLlmCache::new());
        let client = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "final_synthesis",
            "model-v1",
            "prompt-v1",
            evidence(&["ev-1", "ev-2"]),
        );

        let first = client.generate_json("sys", "user").await.expect("first");
        let second = client.generate_json("sys", "user").await.expect("second");

        assert_eq!(first, second);
        assert_eq!(inner.calls(), 1, "second identical request must be cached");
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn different_evidence_ids_miss_the_cache() {
        let inner = Arc::new(CountingLlmClient::new("same-answer"));
        let cache = Arc::new(MemoryLlmCache::new());

        let first = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "final_synthesis",
            "model-v1",
            "prompt-v1",
            evidence(&["ev-1"]),
        );
        let second = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "final_synthesis",
            "model-v1",
            "prompt-v1",
            evidence(&["ev-2"]),
        );

        first.generate_text("sys", "user").await.expect("first");
        second.generate_text("sys", "user").await.expect("second");

        assert_eq!(inner.calls(), 2, "changed evidence must re-run the model");
        assert_eq!(cache.len(), 2);
    }

    #[tokio::test]
    async fn cache_key_is_order_insensitive_but_content_sensitive() {
        let a = cache_key("w", "m", "p", &evidence(&["b", "a"]));
        let b = cache_key("w", "m", "p", &evidence(&["a", "b", "a"]));
        assert_eq!(a, b, "identity is the evidence set, not its order");

        assert_ne!(a, cache_key("w", "m2", "p", &evidence(&["a", "b"])));
        assert_ne!(a, cache_key("w", "m", "p2", &evidence(&["a", "b"])));
        assert_ne!(a, cache_key("w2", "m", "p", &evidence(&["a", "b"])));
        assert_ne!(a, cache_key("w", "m", "p", &evidence(&["a", "c"])));
    }

    #[tokio::test]
    async fn different_model_or_prompt_version_misses() {
        let inner = Arc::new(CountingLlmClient::new("x"));
        let cache = Arc::new(MemoryLlmCache::new());

        let v1 = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "battlecard",
            "model-v1",
            "prompt-v1",
            evidence(&["ev-1"]),
        );
        let v2 = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "battlecard",
            "model-v2",
            "prompt-v1",
            evidence(&["ev-1"]),
        );
        let p2 = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "battlecard",
            "model-v1",
            "prompt-v2",
            evidence(&["ev-1"]),
        );

        v1.generate_json("s", "u").await.expect("v1");
        v2.generate_json("s", "u").await.expect("v2");
        p2.generate_json("s", "u").await.expect("p2");
        assert_eq!(inner.calls(), 3);
    }

    #[tokio::test]
    async fn json_and_text_variants_do_not_collide() {
        let inner = Arc::new(CountingLlmClient::new("payload"));
        let cache = Arc::new(MemoryLlmCache::new());
        let client = CachedLlmClient::new(
            inner.clone(),
            cache.clone(),
            "classification",
            "model-v1",
            "prompt-v1",
            evidence(&["ev-1"]),
        );

        client.generate_json("s", "u").await.expect("json");
        client.generate_text("s", "u").await.expect("text");
        assert_eq!(inner.calls(), 2);
        assert_eq!(cache.len(), 2);
    }
}
