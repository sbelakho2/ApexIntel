-- ApexIntel Schema Migration – triage_queue dimension columns
--
-- The triage code (crates/triage/src/queue.rs) expects individual DOUBLE PRECISION
-- columns for each scoring dimension (urgency, impact, actionability, novelty,
-- confidence) but the table only had a single `dimensions` JSONB column.
--
-- This caused triage_processing job to fail with:
--   "failed to fetch unscored items: column 'urgency' does not exist"
--
-- The columns are NOT NULL DEFAULT 0.0 because:
--   - TriageQueueItemRow uses f64 (not Option<f64>)
--   - New items start unscored (all dimensions = 0.0)
--   - composite_score already defaults to 0.0
-- ==============================================================================

ALTER TABLE triage_queue
    ADD COLUMN IF NOT EXISTS urgency        DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS impact         DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS actionability  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS novelty        DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS confidence     DOUBLE PRECISION NOT NULL DEFAULT 0.0;
