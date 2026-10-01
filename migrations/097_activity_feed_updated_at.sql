-- ════════════════════════════════════════════════════════════════════════════
-- Migration 097: fix the broken activity_feed timestamp trigger
-- ════════════════════════════════════════════════════════════════════════════
--
-- The collaboration schema attaches `update_activity_feed_created` (BEFORE
-- INSERT) whose function assigns `NEW.updated_at` — but the `activity_feed`
-- table was created without an `updated_at` column, so EVERY insert fails with
-- `record "new" has no field "updated_at"`. Activity logging was therefore
-- completely broken.
--
-- Fix: add the missing column (consistent with every sibling collaboration
-- table) and replace the insert-only trigger with an insert-or-update trigger
-- so updates also refresh the timestamp.
--
-- Idempotent: safe to re-apply.

ALTER TABLE activity_feed
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS update_activity_feed_created ON activity_feed;

DROP TRIGGER IF EXISTS update_activity_feed_updated_at ON activity_feed;

CREATE TRIGGER update_activity_feed_updated_at
    BEFORE INSERT OR UPDATE ON activity_feed
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
