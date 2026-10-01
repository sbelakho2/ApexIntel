-- ════════════════════════════════════════════════════════════════════════════
-- Migration 094: workspace assignment unique index
-- ════════════════════════════════════════════════════════════════════════════
--
-- `create_workspace_assignment` now upserts with
-- `ON CONFLICT (workspace_id, user_id)`, which requires a unique index on that
-- column pair. One revision of the collaboration schema declares the pair as a
-- table constraint with an auto-generated name; deployments reconciled from
-- the other revision have no matching arbiter, so it is created here under a
-- stable name. Re-assigning a member then updates the role in place instead of
-- failing (or duplicating) the assignment.
--
-- The legacy revision's table was created without the pair constraint, so a
-- reconciled deployment can already hold duplicate assignments. Collapse them
-- before creating the index — otherwise the unique index build fails and the
-- whole migration chain (and API startup) aborts. The most recently updated
-- row per pair survives and the upsert then owns it.
DELETE FROM workspace_assignments duplicate
USING workspace_assignments keeper
WHERE duplicate.workspace_id = keeper.workspace_id
  AND duplicate.user_id = keeper.user_id
  AND (duplicate.updated_at, duplicate.assigned_at, duplicate.id)
      < (keeper.updated_at, keeper.assigned_at, keeper.id);

-- Idempotent: safe to re-apply.

CREATE UNIQUE INDEX IF NOT EXISTS idx_workspace_assignments_workspace_user
    ON workspace_assignments (workspace_id, user_id);
