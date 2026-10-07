//! Whole-repo dogfood manifest.
//!
//! Every worker job and every subsystem is registered here with how it is
//! dogfooded: a CI check, a live assertion against real state, or an explicit
//! classification (`Manual`, `SubStep`, `IdleOk`) with the reason. The audit
//! harness (`source_pipeline_dogfood --db`) executes the live checks against
//! production; unit tests enforce that the manifest itself never grows a hole
//! (unique names, cadence present for everything scheduled, complete
//! subsystem list).
//!
//! This exists because the analytical dogfood and source dogfood together
//! still missed whole subsystems (2026-10-06: a 40-failure KEV circuit, a
//! perpetually degraded dark-web scan, and a sanctions job timing out all sat
//! green in aggregate health while their own job rows knew they were dead).

/// How a job or subsystem is dogfooded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DogfoodMode {
    /// Scheduled job asserted live: latest terminal run within `max_age_hours`,
    /// status not failed, failures below the circuit policy, circuit closed.
    Live,
    /// Manual/trigger-only job (enqueued by an API or operator action).
    /// Asserted live: no failure streak, circuit closed. Execution proof comes
    /// from the trigger path's own tests.
    Manual,
    /// Runs as a stage inside another job; liveness is asserted through the
    /// parent (`check` names it).
    SubStep,
    /// Scheduled but legitimately idle in some deployments (disabled feature,
    /// empty queue, missing optional integration). Asserted live: failures
    /// below the circuit policy and circuit closed; skips are healthy.
    IdleOk,
    /// Enforced by CI gates rather than a live query (static guards, unit
    /// suites, browser gates).
    Ci,
}

/// One job's dogfood registration.
#[derive(Debug, Clone, Copy)]
pub struct JobDogfood {
    /// Must equal `JobKind::as_str()` for the job.
    pub name: &'static str,
    pub mode: DogfoodMode,
    /// Live-mode freshness bound: the latest terminal run must be newer than
    /// this. Sized from the scheduler cadence with restart/backlog slack.
    pub max_age_hours: Option<u32>,
    /// What the job check asserts (or why the mode is what it is).
    pub check: &'static str,
}

/// One subsystem's dogfood registration.
#[derive(Debug, Clone, Copy)]
pub struct SubsystemDogfood {
    pub name: &'static str,
    pub mode: DogfoodMode,
    /// The executable check(s) guarding this subsystem.
    pub check: &'static str,
}

/// All 44 worker jobs. `max_age_hours` is derived from the scheduler
/// definitions (daily → 50h, weekly → 200h, intervals → 2× period + slack).
pub const JOBS: &[JobDogfood] = &[
    JobDogfood { name: "crawl_cycle", mode: DogfoodMode::Live, max_age_hours: Some(12), check: "cycle metrics; observation ingestion; source state writes" },
    JobDogfood { name: "pattern_mining", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "mining gates persist validated candidates; stage outcome in notes" },
    JobDogfood { name: "hypothesis_generation", mode: DogfoodMode::SubStep, max_age_hours: None, check: "runs inside pattern_mining (worker_job_history row per stage); parent liveness" },
    JobDogfood { name: "poi_refresh", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "POI upserts and discovery counters" },
    JobDogfood { name: "promotion_board", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "weekly eval-gated promotions; lifecycle actions applied" },
    JobDogfood { name: "recipe_deprecation", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "weekly deprecation audit applied" },
    JobDogfood { name: "strategy_memo", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "memo rendered and persisted" },
    JobDogfood { name: "feature_drift_check", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "drift metrics recorded" },
    JobDogfood { name: "source_scoring", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "source scores persisted; also runs inside self_improvement_cycle" },
    JobDogfood { name: "cross_domain_mining", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "combinations mined, correlations persisted, deepening staged" },
    JobDogfood { name: "outcome_tracking", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "source/prediction outcomes recorded" },
    JobDogfood { name: "breach_scan", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "breach observations/warnings counters" },
    JobDogfood { name: "sanctions_screen", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "token-bucket screening completes within timeout; consolidated per-entity warnings" },
    JobDogfood { name: "sla_enforcement", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "SLA breaches queued" },
    JobDogfood { name: "dns_posture_scan", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "SPF/DKIM/DMARC observations for tracked domains" },
    JobDogfood { name: "kev_catalog_fetch", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "KEV catalog fetched via Tor; entry count parsed" },
    JobDogfood { name: "lookalike_domain_scan", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "RDAP lookups; warnings only on impersonation evidence" },
    JobDogfood { name: "update_email_digest", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "digest queue processed" },
    JobDogfood { name: "self_improvement_cycle", mode: DogfoodMode::Live, max_age_hours: Some(200), check: "component failures named in notes; quality review snapshots" },
    JobDogfood { name: "recipe_fire", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "candidates evaluated; zero invalid active definitions after migration 111" },
    JobDogfood { name: "poi_discovery", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "discovered POIs staged" },
    JobDogfood { name: "supplier_pricing_refresh", mode: DogfoodMode::Manual, max_age_hours: None, check: "trigger-only (reader-quota cost); enqueue path covered by supplier-pricing dogfood eval" },
    JobDogfood { name: "starzcrm_sync", mode: DogfoodMode::IdleOk, max_age_hours: None, check: "optional MySQL integration; skip is healthy when unconfigured" },
    JobDogfood { name: "embedding_reindex", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "embedding backlog shrinks; retracted insights excluded" },
    JobDogfood { name: "observation_index", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "Tantivy checkpoint advances" },
    JobDogfood { name: "dark_web_scan", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "monitors tracked portfolio when no env rules; Tor forum scan" },
    JobDogfood { name: "triage_processing", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "queue scored; skip is healthy when empty" },
    JobDogfood { name: "trend_aggregation", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "rollups materialized" },
    JobDogfood { name: "insight_generation", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "insights produced; insight-quality audit (no boilerplate, actionable)" },
    JobDogfood { name: "insight_analysis", mode: DogfoodMode::Manual, max_age_hours: None, check: "enqueued by POST /api/insights/:id/analyze; run row per request" },
    JobDogfood { name: "threat_intel_refresh", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "threat actor profiles refreshed" },
    JobDogfood { name: "psych_profile_compute", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "profiles computed for tracked entities" },
    JobDogfood { name: "poi_role_reclassify", mode: DogfoodMode::Live, max_age_hours: Some(50), check: "role families reclassified" },
    JobDogfood { name: "osint_enrichment", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "enrichment observations; source failures recorded, not fatal" },
    JobDogfood { name: "adversarial_analysis", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "adversarial checks recorded" },
    JobDogfood { name: "anomaly_scan", mode: DogfoodMode::Live, max_age_hours: Some(9), check: "stable-title, floor-gated anomalies; auto-resolution of recovered warnings" },
    JobDogfood { name: "social_scan", mode: DogfoodMode::Live, max_age_hours: Some(5), check: "platform observations incl. Telegram IntelSlavaZ" },
    JobDogfood { name: "tender_scan", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "MENA portal tenders; per-portal outcomes recorded" },
    JobDogfood { name: "contact_enrichment", mode: DogfoodMode::Live, max_age_hours: Some(3), check: "provider fetch policy; enrich counters; skips when no provider credentials configured (configuration gap, not failure)" },
    JobDogfood { name: "icp_scoring", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "ICP scores refreshed" },
    JobDogfood { name: "engagement_refresh", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "engagement rollups; skip is healthy when idle" },
    JobDogfood { name: "buying_center_derivation", mode: DogfoodMode::Live, max_age_hours: Some(14), check: "buying-center roles derived" },
    JobDogfood { name: "person_mention_materialization", mode: DogfoodMode::Live, max_age_hours: Some(3), check: "person mentions materialized" },
    JobDogfood { name: "notification_delivery", mode: DogfoodMode::Live, max_age_hours: Some(2), check: "outbox drain; dead-letter counters" },
];

/// Repo subsystems and where their dogfood assertion lives.
pub const SUBSYSTEMS: &[SubsystemDogfood] = &[
    SubsystemDogfood { name: "source acquisition", mode: DogfoodMode::Live, check: "registry invariants + UA policy tests + live endpoint sample (source_pipeline_dogfood)" },
    SubsystemDogfood { name: "source health detection", mode: DogfoodMode::Live, check: "ingestion-stall/failing-source cross-checks + pg decision-table tests" },
    SubsystemDogfood { name: "crawl scheduling", mode: DogfoodMode::Live, check: "crawl_cycle liveness + selection/backlog metrics + per-source state writes" },
    SubsystemDogfood { name: "feed parsing/ingest", mode: DogfoodMode::Live, check: "freshness capture, cap ordering, insert accounting tests; new/duplicate counts" },
    SubsystemDogfood { name: "dark web / Tor", mode: DogfoodMode::Live, check: "onion transport tests + dark_web_scan liveness with portfolio targets" },
    SubsystemDogfood { name: "sanctions screening", mode: DogfoodMode::Live, check: "token-gated matcher tests (prod false-positive pairs) + screening liveness" },
    SubsystemDogfood { name: "analytical pipeline", mode: DogfoodMode::Live, check: "analytical adversarial dogfood (12 attacks + 400 mutations) + editorial gate" },
    SubsystemDogfood { name: "insight generation", mode: DogfoodMode::Live, check: "insight-quality audit: banned boilerplate, liveness, actionability floor" },
    SubsystemDogfood { name: "insight surfaces", mode: DogfoodMode::Live, check: "retraction filtering on every read path (store list, dashboard, executive, graph, embeddings)" },
    SubsystemDogfood { name: "warning lifecycle", mode: DogfoodMode::Live, check: "warning-hygiene budgets + title stability + auto-resolution of recovered conditions" },
    SubsystemDogfood { name: "recipes", mode: DogfoodMode::Live, check: "seed validation, lifecycle gates, recipe_fire liveness, no invalid actives (migration 111)" },
    SubsystemDogfood { name: "learning/calibration", mode: DogfoodMode::Live, check: "mining gates, negative controls, promotion eval gate, calibration refit" },
    SubsystemDogfood { name: "triage/feedback", mode: DogfoodMode::Live, check: "triage liveness + feedback events persisted on acknowledge/dismiss/resolve" },
    SubsystemDogfood { name: "search", mode: DogfoodMode::Live, check: "bounded async search tests + search page/browser gates" },
    SubsystemDogfood { name: "API routes", mode: DogfoodMode::Live, check: "route sweep and journey specs (browser-integration + ui-journey gates)" },
    SubsystemDogfood { name: "web UI", mode: DogfoodMode::Live, check: "server-ui contract (nav groups, mobile tabs, overflow) + visual snapshots in CI" },
    SubsystemDogfood { name: "notifications/outbox", mode: DogfoodMode::Live, check: "notification_delivery liveness + dead-letter counters" },
    SubsystemDogfood { name: "jobs/scheduler", mode: DogfoodMode::Live, check: "this manifest: every JobKind registered and liveness-asserted (job coverage tests)" },
    SubsystemDogfood { name: "store/schema", mode: DogfoodMode::Live, check: "migration checksum boot, pg-canonical suites, RLS classification tests, retention pruning" },
    SubsystemDogfood { name: "observability", mode: DogfoodMode::Live, check: "health/ready probes, degraded-state reporting, source coverage trend" },
    SubsystemDogfood { name: "model portability", mode: DogfoodMode::Live, check: "model onboarding eval, per-model calibration registry, dogfood live probe" },
    SubsystemDogfood { name: "exports/artifacts", mode: DogfoodMode::Live, check: "artifact job runs recorded in worker history; report endpoints smoke-tested" },
    SubsystemDogfood { name: "config/env contract", mode: DogfoodMode::Ci, check: ".env.example vs code usage guards; sensitive-defaults guard; docs contract" },
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn every_job_has_a_unique_non_empty_check_and_cadence() {
        assert!(
            JOBS.len() >= 44,
            "manifest covers {} jobs (floor 44)",
            JOBS.len()
        );
        let mut seen = std::collections::HashSet::new();
        for job in JOBS {
            assert!(seen.insert(job.name), "duplicate job {}", job.name);
            assert!(!job.check.trim().is_empty(), "empty check for {}", job.name);
            if job.mode == DogfoodMode::Live {
                assert!(
                    job.max_age_hours.is_some(),
                    "live job {} must declare max_age_hours",
                    job.name
                );
                assert!(
                    job.max_age_hours.unwrap_or(0) >= 2,
                    "live job {} cadence too tight",
                    job.name
                );
            }
        }
    }

    #[test]
    fn subsystem_table_is_complete_and_unique() {
        assert!(SUBSYSTEMS.len() >= 20, "subsystems: {}", SUBSYSTEMS.len());
        let mut seen = std::collections::HashSet::new();
        for subsystem in SUBSYSTEMS {
            assert!(
                seen.insert(subsystem.name),
                "duplicate subsystem {}",
                subsystem.name
            );
            assert!(!subsystem.check.trim().is_empty());
        }
    }
}
