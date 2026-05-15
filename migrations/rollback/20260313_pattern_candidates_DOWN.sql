-- Rollback: 20260313_pattern_candidates
-- Drops the pattern_candidates table and indexes.

DROP INDEX IF EXISTS idx_pattern_candidates_passed_gates;
DROP INDEX IF EXISTS idx_pattern_candidates_created_at;
DROP TABLE IF EXISTS pattern_candidates;
