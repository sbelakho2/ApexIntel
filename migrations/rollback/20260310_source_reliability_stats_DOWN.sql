-- Rollback: 20260310_source_reliability_stats
-- Drops the source reliability stats table and its indexes.

DROP INDEX IF EXISTS idx_source_reliability_stats_promotion;
DROP INDEX IF EXISTS idx_source_reliability_stats_effective;
DROP TABLE IF EXISTS source_reliability_stats;
