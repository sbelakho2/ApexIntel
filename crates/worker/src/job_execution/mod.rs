mod adversarial;
mod anomaly_scan;
mod custom;
mod dark_web;
mod insights;
mod intelligence;
mod nightly;
mod osint_enrichment;
mod poi;
mod psych_profile;
mod recipes;
mod resilience;
mod sales;
mod security;
mod social_scan;
mod starzcrm;
mod template_variation;
mod tender_scan;
mod threat_intel;
mod triage;
mod weekly;

use std::sync::Arc;

use apex_store::postgres::PgStore;
use apex_worker::activity_logger::ActivityLogger;
use apex_worker::scheduler::JobStatus;
use apex_worker::scheduler::{JobKind, JobRun};

/// Log a general job-completion event to the activity feed.
async fn log_job_completion(kind: &JobKind, run: &JobRun, logger: &ActivityLogger) {
    let status_str = match &run.status {
        JobStatus::Succeeded { .. } => "succeeded",
        JobStatus::Failed { .. } => "failed",
        JobStatus::Skipped { .. } => "skipped",
        JobStatus::Running => "running",
        JobStatus::Pending => "pending",
    };

    let action = match status_str {
        "succeeded" => "job_completed",
        "failed" => "job_failed",
        "skipped" => "job_skipped",
        _ => "job_unknown",
    };

    let details = serde_json::json!({
        "job_kind": kind.as_str(),
        "status": status_str,
        "duration_ms": run.duration_ms(),
        "items_processed": run.items_processed,
        "notes": run.notes,
    });

    logger
        .insert(apex_worker::activity_logger::ActivityEvent {
            actor_id: "system",
            actor_name: "Worker Engine",
            action_type: action,
            entity_type: Some("job"),
            entity_id: Some(&run.run_id),
            entity_name: Some(kind.as_str()),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
}

#[tracing::instrument(skip(kind, store), fields(job = %kind.as_str()))]
pub(crate) async fn execute_job(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    tracing::debug!(job = %kind.as_str(), "job_start");
    let logger = ActivityLogger::new(store.pool.clone());
    let run = match kind {
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
        JobKind::KevCatalogFetch => security::run_kev_catalog_fetch(kind, store).await,
        JobKind::LookalikeDomainScan => security::run_lookalike_domain_scan(kind, store).await,
        JobKind::SelfImprovementCycle => {
            intelligence::run_self_improvement_cycle(kind, store).await
        }
        JobKind::RecipeFire => recipes::run_recipe_fire(kind, store).await,
        JobKind::PoiDiscovery => poi::run_poi_discovery(store).await,
        JobKind::UpdateEmailDigest => weekly::run_update_email_digest(store).await,
        JobKind::StarzCrmSync => starzcrm::run_starzcrm_sync(store).await,
        JobKind::EmbeddingReindex => {
            apex_worker::embedding_indexer::run_embedding_reindex(kind, store).await
        }
        JobKind::DarkWebScan => dark_web::run_dark_web_scan(kind, store).await,
        JobKind::TriageProcessing => triage::run_triage_processing(kind, store).await,
        JobKind::TrendAggregation => {
            apex_worker::trend_aggregator::run_trend_aggregation(kind, store).await
        }
        JobKind::InsightGeneration => insights::run_insight_generation(kind, store).await,
        JobKind::ThreatIntelRefresh => threat_intel::run_threat_intel_refresh(kind, store).await,
        JobKind::PsychProfileCompute => psych_profile::run_psych_profile_compute(kind, store).await,
        JobKind::PoiRoleReclassify => poi::run_poi_role_reclassify(kind, store).await,
        JobKind::OsintEnrichment => osint_enrichment::run_osint_enrichment(kind, store).await,
        JobKind::AdversarialAnalysis => adversarial::run_adversarial_analysis(kind, store).await,
        JobKind::AnomalyScan => anomaly_scan::run_anomaly_scan(kind, store).await,
        JobKind::SocialScan => social_scan::run_social_scan(kind, store).await,
        JobKind::TenderScan => tender_scan::run_tender_scan(kind, store).await,
        JobKind::ContactEnrichment => sales::run_contact_enrichment(kind, store).await,
        JobKind::IcpScoring => sales::run_icp_scoring(kind, store).await,
        JobKind::EngagementRefresh => sales::run_engagement_refresh(kind, store).await,
        JobKind::BuyingCenterDerivation => sales::run_buying_center_derivation(kind, store).await,
        JobKind::PersonMentionMaterialization => {
            sales::run_person_mention_materialization(kind, store).await
        }
        JobKind::Custom(name) => custom::run_custom_job(name).await,
    };
    log_job_completion(kind, &run, &logger).await;
    run
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default
    )]

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
