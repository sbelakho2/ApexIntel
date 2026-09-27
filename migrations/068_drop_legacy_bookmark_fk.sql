-- 061_drop_legacy_bookmark_fk.sql
--
-- `insight_bookmarks` carried a legacy foreign key to `analyst_users(id)`
-- from before `app_users` became the canonical identity (migration 059).
-- The legacy constraint rejects bookmark writes from principals that only
-- exist in `app_users` (e.g. the admin web session), while the canonical
-- `insight_bookmarks_user_id_fkey` already enforces identity and cascades.
--
-- Drop only the legacy constraint; the app_users FK stays in place.
--
-- Idempotent: the catalog guard makes re-application a no-op.

DO $$
BEGIN
    IF to_regclass('public.insight_bookmarks') IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM pg_constraint
           WHERE conname = 'fk_insight_bookmarks_user'
             AND conrelid = to_regclass('public.insight_bookmarks')
       )
    THEN
        ALTER TABLE insight_bookmarks DROP CONSTRAINT fk_insight_bookmarks_user;
    END IF;
END $$;
