mod custom;
mod intelligence;
mod nightly;
mod poi;
mod recipes;
mod resilience;
mod security;
mod weekly;

use std::sync::Arc;

use crate::{JobKind, JobRun, PgStore};

#[tracing::instrument(skip(kind, store), fields(job = %kind.as_str()))]
pub(crate) async fn execute_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    tracing::debug!(job = %kind.as_str(), "job_start");
    match kind {
        JobKind::CrawlCycle => nightly::run_crawl_cycle(store).await,
        JobKind::PatternMining => nightly::run_pattern_mining(kind, store).await,
        JobKind::HypothesisGeneration => nightly::run_hypothesis_generation(kind, store).await,
        JobKind::PoiRefresh => poi::run_poi_refresh(kind, store).await,
        JobKind::FeatureDriftCheck => nightly::run_feature_drift_check(kind, store).await,
        JobKind::PromotionBoard | JobKind::RecipeDeprecation => {
            weekly::run_weekly_recipe_job(kind, store).await
        }
        JobKind::StrategyMemo => weekly::run_strategy_memo(kind, store).await,
        JobKind::SourceScoring => intelligence::run_source_scoring(kind, store).await,
        JobKind::CrossDomainMining => intelligence::run_cross_domain_mining(kind, store).await,
        JobKind::OutcomeTracking => intelligence::run_outcome_tracking(kind, store).await,
        JobKind::BreachScan => security::run_breach_scan(kind, store).await,
        JobKind::SanctionsScreen => security::run_sanctions_screen(kind, store).await,
        JobKind::SlaEnforcement => security::run_sla_enforcement(kind, store).await,
        JobKind::DnsPostureScan => security::run_dns_posture_scan(kind, store).await,
        JobKind::KevCatalogFetch => security::run_kev_catalog_fetch(kind).await,
        JobKind::LookalikeDomainScan => security::run_lookalike_domain_scan(kind, store).await,
        JobKind::SelfImprovementCycle => {
            intelligence::run_self_improvement_cycle(kind, store).await
        }
        JobKind::RecipeFire => recipes::run_recipe_fire(kind, store).await,
        JobKind::PoiDiscovery => poi::run_poi_discovery(store).await,
        JobKind::UpdateEmailDigest => weekly::run_update_email_digest(store).await,
        JobKind::Custom(name) => custom::run_custom_job(name).await,
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn custom_job_without_env_is_skipped() {
        let name = "missing_dispatch_env";
        let key = format!("CUSTOM_JOB_COMMAND_{}", name.to_uppercase());
        std::env::remove_var(&key);

        let run = super::custom::run_custom_job(name).await;

        assert!(matches!(run.status, crate::JobStatus::Skipped { .. }));
        match &run.status {
            crate::JobStatus::Skipped { reason } => {
                assert!(reason.contains("custom command env missing"));
                assert!(reason.contains(&key));
            }
            status => panic!("expected skipped status, got {status:?}"),
        }
    }
}
