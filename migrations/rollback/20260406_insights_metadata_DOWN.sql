-- Rollback: 20260406_insights_metadata
-- Reverts the metadata column addition on insights.
-- Note: This is a destructive rollback. If data exists in metadata, use
-- a transaction to back it up first.

ALTER TABLE insights DROP COLUMN IF EXISTS metadata;
