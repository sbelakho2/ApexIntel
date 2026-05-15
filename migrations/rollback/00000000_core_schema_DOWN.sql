-- Rollback: 00000000_core_schema
-- Drops all core schema tables and their indexes.
-- WARNING: This will destroy all data. Use with extreme caution.

DROP INDEX IF EXISTS idx_merge_survivor;
DROP TABLE IF EXISTS entity_merges;

DROP INDEX IF EXISTS idx_poi_eng_person;
DROP TABLE IF EXISTS poi_engagements;

DROP TABLE IF EXISTS user_preferences;

DROP INDEX IF EXISTS idx_crawl_source;
DROP TABLE IF EXISTS crawl_telemetry;

DROP INDEX IF EXISTS idx_audit_type;
DROP INDEX IF EXISTS idx_audit_ts;
DROP TABLE IF EXISTS audit_log;

DROP INDEX IF EXISTS idx_feature_entity;
DROP TABLE IF EXISTS feature_rows;

DROP INDEX IF EXISTS idx_recipes_status;
DROP TABLE IF EXISTS recipes;

DROP INDEX IF EXISTS idx_social_platform;
DROP TABLE IF EXISTS social_signals;

DROP INDEX IF EXISTS idx_pf_url;
DROP TABLE IF EXISTS page_fingerprints;

DROP INDEX IF EXISTS idx_insights_entity;
DROP INDEX IF EXISTS idx_insights_type;
DROP TABLE IF EXISTS insights;

DROP INDEX IF EXISTS idx_recipe_weekly_metrics_week;
DROP INDEX IF EXISTS idx_recipe_weekly_metrics_recipe_week;
DROP TABLE IF EXISTS recipe_weekly_metrics;

DROP INDEX IF EXISTS idx_warnings_unack;
DROP INDEX IF EXISTS idx_warnings_entity;
DROP INDEX IF EXISTS idx_warnings_severity;
DROP INDEX IF EXISTS idx_warnings_type;
DROP TABLE IF EXISTS warnings;

DROP INDEX IF EXISTS idx_graph_not_stale;
DROP INDEX IF EXISTS idx_graph_target;
DROP INDEX IF EXISTS idx_graph_source;
DROP TABLE IF EXISTS graph_edges;

DROP INDEX IF EXISTS idx_obs_type_ts;
DROP INDEX IF EXISTS idx_obs_type_entity;
DROP TABLE IF EXISTS observations;

DROP INDEX IF EXISTS idx_poi_artifacts_type;
DROP INDEX IF EXISTS idx_poi_artifacts_person;
DROP TABLE IF EXISTS poi_artifacts;

DROP INDEX IF EXISTS idx_persons_name_trgm;
DROP INDEX IF EXISTS idx_persons_region;
DROP INDEX IF EXISTS idx_persons_org;
DROP TABLE IF EXISTS persons;

DROP TABLE IF EXISTS regulations;

DROP TABLE IF EXISTS logistics_nodes;

DROP INDEX IF EXISTS idx_cert_expiry;
DROP INDEX IF EXISTS idx_cert_standard;
DROP INDEX IF EXISTS idx_cert_company;
DROP TABLE IF EXISTS certifications;

DROP INDEX IF EXISTS idx_cap_capability;
DROP INDEX IF EXISTS idx_cap_company;
DROP TABLE IF EXISTS capabilities;

DROP INDEX IF EXISTS idx_pf_company;
DROP TABLE IF EXISTS product_families;

DROP INDEX IF EXISTS idx_sites_region;
DROP INDEX IF EXISTS idx_sites_company;
DROP TABLE IF EXISTS sites;

DROP INDEX IF EXISTS idx_companies_name_trgm;
DROP INDEX IF EXISTS idx_companies_competitor;
DROP INDEX IF EXISTS idx_companies_region;
DROP INDEX IF EXISTS idx_companies_domain;
DROP TABLE IF EXISTS companies;
