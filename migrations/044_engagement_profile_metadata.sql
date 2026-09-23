-- 044_engagement_profile_metadata.sql
-- The engagement-refresh worker persists a per-person engagement summary. It
-- previously wrote to a non-existent `metadata` column inside a `let _ = ...`,
-- so the feedback loop silently stored nothing. Add the column the worker
-- (crates/worker/src/job_execution/sales.rs) expects.

ALTER TABLE engagement_profiles
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}';
