-- Rollback: 20260302_insight_bookmarks
-- Drops the insight_bookmarks table and indexes.

DROP INDEX IF EXISTS idx_bookmarks_created;
DROP INDEX IF EXISTS idx_bookmarks_insight;
DROP INDEX IF EXISTS idx_bookmarks_user;
DROP TABLE IF EXISTS insight_bookmarks;
