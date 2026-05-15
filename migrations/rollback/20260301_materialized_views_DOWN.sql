-- Rollback: 20260301_materialized_views
-- Drops all materialized views and the refresh function.

DROP FUNCTION IF EXISTS refresh_all_materialized_views();

DROP INDEX IF EXISTS idx_mv_cert_expiry;
DROP MATERIALIZED VIEW IF EXISTS mv_cert_expiry_tracker;

DROP INDEX IF EXISTS idx_mv_obs_vol;
DROP MATERIALIZED VIEW IF EXISTS mv_observation_volume;

DROP INDEX IF EXISTS idx_mv_recipe_perf;
DROP MATERIALIZED VIEW IF EXISTS mv_recipe_performance;

DROP INDEX IF EXISTS idx_mv_poi_coverage;
DROP MATERIALIZED VIEW IF EXISTS mv_poi_coverage;

DROP INDEX IF EXISTS idx_mv_company_risk;
DROP MATERIALIZED VIEW IF EXISTS mv_company_risk_leaderboard;

DROP INDEX IF EXISTS idx_mv_warn_summary;
DROP MATERIALIZED VIEW IF EXISTS mv_warning_summary;
