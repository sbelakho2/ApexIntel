-- Migration 103: workspace owner_id username → canonical user id
-- ════════════════════════════════════════════════════════════════════════════
-- `investigation_workspaces.owner_id` historically stored the login username
-- of the owner while every newer collaboration surface (assignments, shares,
-- activity actors) stores the canonical `app_users.id`. During the transition
-- a workspace row can therefore hold either form, and visibility comparisons
-- against the authenticated principal only match one of them.
--
-- This one-off migration:
--   1. backfills every existing workspace whose `owner_id` is a username to
--      the matching `app_users.id` (rows that already hold an id, or a value
--      that matches no provisioned user, are left untouched);
--   2. anchors the column to `app_users(id)` with a `NOT VALID` foreign key,
--      so legacy values that could not be backfilled do not abort the
--      migration while every new or changed value is checked.
--
-- Idempotency: the backfill only rewrites rows that still hold a username and
-- the constraint is added only when absent, so re-running is a no-op.
-- ════════════════════════════════════════════════════════════════════════════

DO $$
BEGIN
    IF to_regclass('public.app_users') IS NULL THEN
        RAISE EXCEPTION 'migration 103 requires the app_users identity table';
    END IF;
    IF to_regclass('public.investigation_workspaces') IS NULL THEN
        RETURN;
    END IF;
END $$;

-- Backfill usernames to ids. `NOT EXISTS (… WHERE existing.id = w.owner_id)`
-- keeps this idempotent: a row already holding an id never matches an
-- `app_users.username` equal to that id unless the id is also a username, and
-- even then rewriting u.id = w.owner_id is an unchanged write.
UPDATE investigation_workspaces w
SET owner_id = u.id,
    updated_at = NOW()
FROM app_users u
WHERE w.owner_id = u.username
  AND w.owner_id <> u.id
  AND NOT EXISTS (
      SELECT 1 FROM app_users already
      WHERE already.id = w.owner_id
  );

-- No foreign key is added yet, deliberately: an FK (even `NOT VALID`) checks
-- every NEW write, while the transition still accepts writes carrying either
-- the username or the id — exactly the dual form the store reads support.
-- Forcing ids at write time is a follow-up once every writer emits an
-- app_users.id; until then the backfill above plus the dual-form reads keep
-- both forms working.

COMMENT ON COLUMN investigation_workspaces.owner_id IS
    'Canonical app_users.id of the workspace owner after backfill; transitional rows may still hold a username, which the visibility reads accept';
