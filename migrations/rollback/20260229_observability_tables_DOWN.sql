-- Rollback: 20260229_observability_tables
-- Drops observability tables, views, and indexes.

DROP VIEW IF EXISTS insight_acceptance_ratios;
DROP VIEW IF EXISTS gate_fire_rates;

DROP INDEX IF EXISTS idx_insight_outcomes_outcome;
DROP INDEX IF EXISTS idx_insight_outcomes_recorded;
DROP TABLE IF EXISTS insight_outcomes;

DROP INDEX IF EXISTS idx_llm_retry_entity;
DROP INDEX IF EXISTS idx_llm_retry_recorded;
DROP TABLE IF EXISTS llm_retry_stats;

DROP INDEX IF EXISTS idx_gate_decisions_entity;
DROP INDEX IF EXISTS idx_gate_decisions_recorded;
DROP INDEX IF EXISTS idx_gate_decisions_name;
DROP TABLE IF EXISTS gate_decisions;

DROP INDEX IF EXISTS idx_quality_breakdown_entity;
DROP INDEX IF EXISTS idx_quality_breakdown_recorded;
DROP INDEX IF EXISTS idx_quality_breakdown_obs;
DROP TABLE IF EXISTS quality_score_breakdown;
