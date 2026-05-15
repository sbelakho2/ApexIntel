-- Rollback: 20260301_data_retention
-- Drops all data retention functions.

DROP FUNCTION IF EXISTS run_data_retention();
DROP FUNCTION IF EXISTS flag_stale_entities(INT);
DROP FUNCTION IF EXISTS mark_stale_graph_edges(INT);
DROP FUNCTION IF EXISTS cleanup_audit_log(INT);
DROP FUNCTION IF EXISTS cleanup_feature_rows(INT);
DROP FUNCTION IF EXISTS cleanup_page_fingerprints();
DROP FUNCTION IF EXISTS cleanup_social_signals(INT);
DROP FUNCTION IF EXISTS cleanup_crawl_telemetry(INT);
DROP FUNCTION IF EXISTS cleanup_old_observations(INT);
DROP FUNCTION IF EXISTS cleanup_gate_decisions(INT);
DROP FUNCTION IF EXISTS cleanup_llm_retry_stats(INT);
DROP FUNCTION IF EXISTS cleanup_insight_outcomes(INT);
DROP FUNCTION IF EXISTS cleanup_quality_score_breakdown(INT);
