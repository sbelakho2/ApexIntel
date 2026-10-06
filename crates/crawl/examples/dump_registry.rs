//! Dump the authoritative merged source registry (built-in + supplement) as
//! JSON so ops tooling (scripts/verify_sources.py, exclusion audits) can work
//! from the exact list the worker schedules.
//!
//! Usage:
//!   cargo run -p apex-crawl --example dump_registry > registry.json

use apex_crawl::sources::{all_sources, Source};
use serde_json::json;

fn main() {
    let sources: Vec<Source> = all_sources();
    let payload = json!({
        "total": sources.len(),
        "enabled": sources.iter().filter(|s| s.enabled).count(),
        "sources": sources
            .iter()
            .map(|source| {
                json!({
                    "slug": source.slug,
                    "name": source.name,
                    "url": source.url,
                    "rss_url": source.rss_url,
                    "region": source.region.as_str(),
                    "category": source.category.as_str(),
                    "tier": source.tier,
                    "needs_proxy": source.needs_proxy,
                    "enabled": source.enabled,
                    "onion": source.url.ends_with(".onion"),
                })
            })
            .collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload)
            .unwrap_or_else(|error| panic!("serialize registry: {error}"))
    );
}
