-- ════════════════════════════════════════════════════════════════════════════
-- Purge generated / fabricated content, preserving companies and POIs.
-- ════════════════════════════════════════════════════════════════════════════
-- Usage:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f scripts/ops/purge_generated_content.sql
--
-- Scope:
--   Deletes every row from the generated-content surfaces: warnings and their
--   analyses/evidence, insights and claims/bookmarks/feedback, triage queue,
--   observations and crawl/telemetry rows, notifications and delivery state,
--   activity feed, memos, weekly recipe metrics, trends, graph/dedup rows,
--   psych/engagement profiles, buying centers, competition/sales/supplier
--   content, security findings, investigations/workspaces, tags/notes,
--   learning/eval rows, LLM runs and caches, and runtime/audit artifacts.
--
-- Preserved (never touched):
--   companies, company_assets, persons, poi_artifacts, contact_methods,
--   app_users, user_preferences, recipes (definitions), sources/sites/
--   regulations/logistics_nodes/product_families/capabilities (reference
--   catalogs), alert_rules/alert_preferences/entity_alert_configs/user
--   preferences and other configuration, saved_searches, _sqlx_migrations.
--
-- The whole purge runs in one transaction: if any referenced table is missing
-- or a delete is blocked, nothing is committed (no partial wipe).
-- ════════════════════════════════════════════════════════════════════════════

\timing on

BEGIN;

DO $$
DECLARE
    target text;
    purged bigint;
    total  bigint := 0;
    -- Children before parents. CASCADE would handle most dependents, but the
    -- explicit order keeps RESTRICT constraints (e.g. insight_claim_evidence
    -- -> observations) from aborting the purge.
    targets text[] := ARRAY[
        -- warning analysis + evidence
        'warning_analysis_claim_evidence',
        'warning_analysis_claims',
        'warning_analysis_runs',
        'warning_evidence',
        'warnings',
        -- insight claims / evidence / bookmarks / feedback.
        -- `insight_claims` MUST precede `insight_claim_evidence`: an
        -- observed/inference claim may not lose its last evidence row
        -- (constraint trigger `insight_claim_evidence_policy_guard`), so the
        -- claims are deleted first and their links cascade with them.
        'insight_claims',
        'insight_claim_evidence',
        'insight_bookmarks',
        'insight_feedback_events',
        'insight_firings',
        'insight_generation_log',
        'insight_outcomes',
        'insights',
        -- triage
        'triage_decisions',
        'triage_queue',
        -- observations and everything derived from the crawl
        'observation_entity_graph',
        'observations',
        'evidence_lineage_edges',
        'evidence_lineage_nodes',
        'source_evidence',
        'source_reliability_stats',
        'source_runtime_state',
        'kev_observations',
        'crawl_logs',
        'crawl_metrics',
        'crawl_runs',
        'crawl_telemetry',
        'page_fingerprints',
        -- notifications / outbox / dead letters
        'notification_delivery_attempts',
        'notification_delivery_state',
        'notification_events',
        'event_outbox',
        'dead_letter_queue',
        'replay_jobs',
        'analyst_notifications',
        'notifications',
        -- activity / memos / metrics / trends
        'activity_feed',
        'weekly_memos',
        'weekly_memo_recipients',
        'recipe_weekly_metrics',
        'recipe_quality_benchmarks',
        'feature_rows',
        'trend_rollups',
        'sentiment_time_series',
        'social_signals',
        'stats_alert_calibration_events',
        'sla_reminder_state',
        'review_cycle_items',
        'review_cycles',
        -- graph / dedup / identity
        'graph_edges',
        'entity_merges',
        'semantic_dedup_items',
        'semantic_dedup_state',
        'entity_review_queue',
        'entity_verification_evidence',
        'identity_orphan_quarantine',
        'entity_orphan_quarantine',
        -- people/company profiles derived from content
        'psychological_profiles',
        'psych_profile_sources',
        'behavioral_pattern_events',
        'engagement_events',
        'engagement_profiles',
        'role_history',
        'dossier_entries',
        'buying_center_members',
        'buying_centers',
        'poi_engagements',
        -- competition / sales / supply content
        'competitor_changes',
        'competitor_pricing',
        'competitive_positions',
        'company_changes',
        'person_changes',
        'battlecards',
        'strategic_opportunities',
        'strategic_predictions',
        'pipeline_opportunities',
        'closed_deals',
        'starzcrm_deals',
        'starzcrm_sync_state',
        'crm_sync_state',
        'supplier_contacts',
        'supplier_capacities',
        'supplier_risk_entries',
        'supplier_risk_assessments',
        'supplier_risk',
        'company_supply_chain_relationships',
        'supply_chain_relationships',
        -- security findings
        'lookalike_domains',
        'dns_posture_entries',
        'critical_threats',
        'threat_actors',
        'threat_campaigns',
        -- investigations / workspaces / collaboration content
        'investigation_shares',
        'investigations',
        'workspace_assignments',
        'investigation_workspaces',
        'annotations',
        'tag_assignments',
        'tags',
        'bookmarks',
        -- queues and scoring
        'daily_priority_queue',
        'priority_queue',
        'quality_score_breakdown',
        'pattern_candidates',
        -- learning / LLM artifacts
        'learning_eval_examples',
        'learning_eval_metrics',
        'learning_eval_runs',
        'learning_eval_sets',
        'llm_improvement_runs',
        'llm_response_cache',
        'llm_retry_stats',
        'llm_training_datasets',
        'llm_workflow_runs',
        'llm_cache',
        'prompt_versions',
        'gate_decisions',
        'certifications',
        -- runtime artifacts
        'alert_engine_state',
        'race'
    ];
BEGIN
    -- Frozen evaluation sets deliberately refuse DELETE/UPDATE so a scored
    -- run can never be rewritten. A purge is exactly the "create a new
    -- version instead" case taken to its limit, so the two freeze guards are
    -- disabled for this transaction only and restored below (a rollback also
    -- restores them, because ALTER TABLE ... DISABLE TRIGGER is transactional).
    IF to_regclass('public.learning_eval_sets') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM pg_trigger
           WHERE tgname = 'trg_learning_eval_sets_freeze'
             AND tgrelid = 'public.learning_eval_sets'::regclass
             AND NOT tgisinternal
       )
    THEN
        EXECUTE 'ALTER TABLE learning_eval_sets DISABLE TRIGGER trg_learning_eval_sets_freeze';
    END IF;
    IF to_regclass('public.learning_eval_examples') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM pg_trigger
           WHERE tgname = 'trg_learning_eval_examples_freeze'
             AND tgrelid = 'public.learning_eval_examples'::regclass
             AND NOT tgisinternal
       )
    THEN
        EXECUTE 'ALTER TABLE learning_eval_examples DISABLE TRIGGER trg_learning_eval_examples_freeze';
    END IF;

    FOREACH target IN ARRAY targets LOOP
        IF to_regclass('public.' || quote_ident(target)) IS NULL THEN
            RAISE NOTICE 'skip (absent): %', target;
            CONTINUE;
        END IF;
        EXECUTE format('DELETE FROM %I', target);
        GET DIAGNOSTICS purged = ROW_COUNT;
        total := total + purged;
        IF purged > 0 THEN
            RAISE NOTICE 'purged % rows from %', purged, target;
        END IF;
    END LOOP;

    -- Restore the freeze guards immediately; the purge transaction is the
    -- only context in which they may be lifted.
    IF to_regclass('public.learning_eval_examples') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM pg_trigger
           WHERE tgname = 'trg_learning_eval_examples_freeze'
             AND tgrelid = 'public.learning_eval_examples'::regclass
       )
    THEN
        EXECUTE 'ALTER TABLE learning_eval_examples ENABLE TRIGGER trg_learning_eval_examples_freeze';
    END IF;
    IF to_regclass('public.learning_eval_sets') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM pg_trigger
           WHERE tgname = 'trg_learning_eval_sets_freeze'
             AND tgrelid = 'public.learning_eval_sets'::regclass
       )
    THEN
        EXECUTE 'ALTER TABLE learning_eval_sets ENABLE TRIGGER trg_learning_eval_sets_freeze';
    END IF;

    RAISE NOTICE 'purge complete: % rows deleted', total;
END $$;

-- Post-conditions: the preserved surfaces must still exist with their rows.
DO $$
DECLARE
    companies_count bigint;
    persons_count   bigint;
    poi_count       bigint;
BEGIN
    SELECT COUNT(*) INTO companies_count FROM companies;
    SELECT COUNT(*) INTO persons_count FROM persons;
    SELECT COUNT(*) INTO poi_count FROM poi_artifacts;
    RAISE NOTICE 'preserved: % companies, % persons, % poi_artifacts',
        companies_count, persons_count, poi_count;
END $$;

COMMIT;
