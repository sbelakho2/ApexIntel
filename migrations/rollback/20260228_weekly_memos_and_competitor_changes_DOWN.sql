-- Rollback: 20260228_weekly_memos_and_competitor_changes
-- Drops weekly_memos, competitor_changes tables, and removes seed data.

DELETE FROM competitor_changes WHERE source_url LIKE 'https://example.com/%';
DELETE FROM weekly_memos WHERE week_start = DATE '2026-02-23' AND week_end = DATE '2026-03-01';

DROP INDEX IF EXISTS idx_competitor_changes_type;
DROP INDEX IF EXISTS idx_competitor_changes_detected;
DROP INDEX IF EXISTS idx_competitor_changes_competitor;
DROP TABLE IF EXISTS competitor_changes;

DROP INDEX IF EXISTS idx_weekly_memos_generated;
DROP INDEX IF EXISTS idx_weekly_memos_week;
DROP TABLE IF EXISTS weekly_memos;
