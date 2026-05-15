-- Rollback: 20260313_performance_indexes
-- Drops all indexes created by the performance index migration.
-- Note: These indexes reference core schema columns that differ between
-- migration lineages. Indexes referencing entity_ids or title_hash are
-- dropped if they exist.

DROP INDEX IF EXISTS idx_obs_entity_ts;
DROP INDEX IF EXISTS idx_warnings_created_at_active;
DROP INDEX IF EXISTS idx_warnings_active_type_severity;
DROP INDEX IF EXISTS idx_warnings_entity_ids_gin;
DROP INDEX IF EXISTS idx_insights_entity_ids_gin;
DROP INDEX IF EXISTS idx_insights_dedup_window;
DROP INDEX IF EXISTS idx_insights_updated_at;
DROP INDEX IF EXISTS idx_insights_title_hash;
