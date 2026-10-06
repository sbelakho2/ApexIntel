//! SupplierPricingRefresh — crawls component pricing for the default BOM
//! mapping's critical components through the r.jina.ai reader (Alibaba, 1688,
//! LCSC marketplaces with a Baidu discovery fallback) and persists the lowest
//! quotable price per part as a `CommodityPrice` observation.
//!
//! The job is trigger-only (manual or admin trigger surface): crawling
//! marketplaces costs reader quota, so it never runs on the default schedule.

use std::sync::Arc;

use apex_core::entities::{Observation, ObservationType};
use apex_crawl::supplier_pricing::{
    quote_observation_id, LowestQuote, SupplierPricePipeline, SupplierPricingClient,
    SupplierPricingConfig,
};
use apex_store::postgres::PgStore;
use chrono::Utc;
use serde_json::json;

use crate::{JobKind, JobRun};

/// Default target quantity for a price quote (pieces). Tiered marketplaces
/// list per-quantity prices; quoting at this quantity avoids teaser singles.
pub(crate) const DEFAULT_TARGET_QUANTITY: u64 = 100;

/// Per-part crawl outcome, kept free of store types for unit tests.
#[derive(Debug)]
pub(crate) enum PartOutcome {
    Quoted(LowestQuote),
    TeaserOnly,
    Failed,
}

/// Refresh statistics surfaced in the job run.
#[derive(Debug, Default, Clone)]
pub(crate) struct RefreshStats {
    pub parts_attempted: usize,
    pub quoted: usize,
    pub teaser_only: usize,
    pub failed: usize,
}

/// Deduplicated part numbers from the default BOM mapping's critical
/// components, capped at `max_parts`.
pub(crate) fn collect_part_numbers(max_parts: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in apex_insights::shortage_correlation::default_bom_mapping() {
        for part in entry.critical_components {
            let part = part.trim().to_string();
            if !part.is_empty() && seen.insert(part.clone()) {
                parts.push(part);
                if parts.len() >= max_parts.max(1) {
                    return parts;
                }
            }
        }
    }
    parts
}

/// `APEX_SUPPLIER_PRICING_MAX_PARTS`, clamped to a positive value.
pub(crate) fn max_parts_from_env() -> usize {
    std::env::var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10)
}

/// One persisted price observation for a quoted part. The observation id is
/// deterministic per (part, listing, price, currency) so a re-run for the same
/// quote never duplicates the row (`ON CONFLICT (id) DO NOTHING`).
pub(crate) fn quote_observation(part_number: &str, quote: &LowestQuote) -> Observation {
    let value = json!({
        "part_number": part_number,
        "source": quote.source,
        "listing_url": quote.listing_url,
        "unit_price": quote.unit_price,
        "currency": quote.currency,
        "moq": quote.moq,
        "max_qty": quote.max_qty,
        "verified": !quote.reconfirmation_required,
        "needs_reconfirmation": quote.reconfirmation_required,
        "teaser_only_listings": quote.teaser_only_listings,
        "candidates": quote.candidates.len(),
        "median_unit_usd": quote.median_unit_usd,
        "sample_size": quote.sample_size,
        "fx_applied": quote.fx_applied,
        "outlier_risk": quote.outlier_risk,
        "evidence": quote.evidence_raw.chars().take(200).collect::<String>(),
    });
    let provenance = json!({
        "pipeline": "supplier_pricing",
        "reader": "r.jina.ai",
    });
    let id = quote_observation_id(
        part_number,
        &quote.listing_url,
        quote.unit_price,
        &quote.currency,
    );
    let mut observation = Observation::new(
        ObservationType::CommodityPrice,
        Utc::now(),
        value,
        provenance,
    );
    observation.id = id;
    observation
}

/// Crawl one part through the pipeline. `None` client (e.g. network disabled)
/// fails cleanly rather than pretending to have priced anything.
pub(crate) async fn price_part(
    pipeline: &SupplierPricePipeline,
    part_number: &str,
    target_quantity: u64,
) -> PartOutcome {
    match pipeline
        .price_for_part(part_number, target_quantity, Some("USD"))
        .await
    {
        Ok(quote) => PartOutcome::Quoted(quote),
        Err(apex_crawl::supplier_pricing::SupplierError::NoListings { .. }) => {
            PartOutcome::TeaserOnly
        }
        Err(error) => {
            tracing::warn!(part_number, %error, "supplier pricing crawl failed for part");
            PartOutcome::Failed
        }
    }
}

/// Execute the refresh against the real pipeline and store.
pub(crate) async fn run_supplier_pricing_refresh(
    store: &Arc<PgStore>,
    pipeline: &SupplierPricePipeline,
    max_parts: usize,
) -> JobRun {
    let mut run = JobRun::new(JobKind::SupplierPricingRefresh);
    let mut stats = RefreshStats::default();
    let parts = collect_part_numbers(max_parts);

    if parts.is_empty() {
        run.fail("supplier_pricing: BOM mapping exposes no critical components");
        return run;
    }

    for part in parts {
        stats.parts_attempted += 1;
        match price_part(pipeline, &part, DEFAULT_TARGET_QUANTITY).await {
            PartOutcome::Quoted(quote) => {
                match store
                    .insert_observation(&quote_observation(&part, &quote))
                    .await
                {
                    Ok(_) => stats.quoted += 1,
                    Err(error) => {
                        stats.failed += 1;
                        tracing::error!(
                            part_number = %part,
                            %error,
                            "supplier_pricing: failed to persist quote observation"
                        );
                    }
                }
            }
            PartOutcome::TeaserOnly => {
                stats.teaser_only += 1;
                tracing::info!(part_number = %part, "supplier_pricing: teaser-only listings; nothing quotable persisted");
            }
            PartOutcome::Failed => stats.failed += 1,
        }
    }

    run.items_processed = stats.parts_attempted as u64;
    let quoted = stats.quoted;
    let teaser_only = stats.teaser_only;
    let failed = stats.failed;
    if quoted == 0 {
        // Nothing quotable was persisted: the run must not claim success.
        run.fail(&format!(
            "supplier_pricing: 0 of {} parts quoted (teaser_only={teaser_only}, failed={failed})",
            stats.parts_attempted
        ));
    } else if failed > 0 || teaser_only > 0 {
        run.degrade(
            stats.parts_attempted as u64,
            &format!(
                "supplier_pricing: quoted={quoted} teaser_only={teaser_only} failed={failed} of {} parts",
                stats.parts_attempted
            ),
        );
    } else {
        run.succeed(
            stats.parts_attempted as u64,
            &format!(
                "supplier_pricing: quoted {quoted} of {} parts",
                stats.parts_attempted
            ),
        );
    }
    run
}

/// Entry point used by the runtime dispatch.
pub(crate) async fn execute_supplier_pricing_refresh(store: &Arc<PgStore>) -> JobRun {
    let config = SupplierPricingConfig::from_env();
    match SupplierPricingClient::new(config) {
        Ok(client) => {
            let pipeline = SupplierPricePipeline::new(Arc::new(client));
            run_supplier_pricing_refresh(store, &pipeline, max_parts_from_env()).await
        }
        Err(error) => {
            let mut run = JobRun::new(JobKind::SupplierPricingRefresh);
            run.fail(&format!(
                "supplier_pricing: reader client construction failed: {error}"
            ));
            run
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_crawl::supplier_pricing::{ListingQuote, PriceOffer};

    #[test]
    fn part_numbers_are_deduplicated_and_capped() {
        let parts = collect_part_numbers(4);
        assert!(!parts.is_empty());
        assert!(parts.len() <= 4);
        let unique: std::collections::HashSet<_> = parts.iter().collect();
        assert_eq!(unique.len(), parts.len(), "no duplicate part numbers");
    }

    #[test]
    fn quote_observation_is_deterministic_and_complete() {
        let quote = LowestQuote {
            listing_url: "https://www.lcsc.com/product-detail/C12345.html".to_string(),
            source: "lcsc".to_string(),
            unit_price: 0.07,
            currency: "USD".to_string(),
            moq: 100,
            max_qty: Some(999),
            candidates: vec![ListingQuote {
                listing_url: "https://www.lcsc.com/product-detail/C12345.html".to_string(),
                source: "lcsc".to_string(),
                offers: vec![PriceOffer {
                    unit_price: 0.07,
                    currency: "USD".to_string(),
                    moq: 100,
                    max_qty: Some(999),
                    teaser: false,
                    price_max: None,
                    raw: String::new(),
                }],
                quotable: None,
                reconfirmation_required: false,
            }],
            teaser_only_listings: 0,
            median_unit_usd: Some(0.07),
            sample_size: 1,
            fx_applied: false,
            outlier_risk: false,
            evidence_raw: "100 - 999 : US $0.07".to_string(),
            reconfirmation_required: false,
            low_confidence: false,
        };
        let a = quote_observation("C12345", &quote);
        let b = quote_observation("C12345", &quote);
        assert_eq!(a.id, b.id, "observation id is deterministic");
        assert_eq!(a.observation_type.as_str(), "CommodityPrice");
        assert_eq!(a.value["unit_price"], 0.07);
        assert_eq!(a.value["source"], "lcsc");
        assert_eq!(a.provenance["reader"], "r.jina.ai");
        assert_eq!(a.value["verified"], true);
    }

    #[test]
    fn max_parts_from_env_clamps() {
        let original = std::env::var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS).ok();
        std::env::set_var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS, "3");
        assert_eq!(max_parts_from_env(), 3);
        std::env::set_var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS, "0");
        assert_eq!(max_parts_from_env(), 10, "zero falls back to the default");
        std::env::remove_var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS);
        assert_eq!(max_parts_from_env(), 10, "unset falls back to the default");
        if let Some(value) = original {
            std::env::set_var(apex_core::env::APEX_SUPPLIER_PRICING_MAX_PARTS, value);
        }
    }
}
