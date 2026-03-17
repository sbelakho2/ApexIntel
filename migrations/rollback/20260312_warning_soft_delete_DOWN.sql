-- Rollback: 20260312_warning_soft_delete
-- Removes the soft-delete column and associated indexes from warnings.

DROP INDEX IF EXISTS idx_warnings_active_created_at;
DROP INDEX IF EXISTS idx_warnings_deleted_at;
ALTER TABLE warnings DROP COLUMN IF EXISTS deleted_at;
