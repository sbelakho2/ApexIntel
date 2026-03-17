-- Rollback: 20260309_collaboration_and_replay
-- Drops all collaboration and replay tables and their indexes.

DROP INDEX IF EXISTS idx_replay_jobs_requested;
DROP TABLE IF EXISTS replay_jobs;

DROP INDEX IF EXISTS idx_export_history_user_requested;
DROP TABLE IF EXISTS export_history;

DROP INDEX IF EXISTS idx_annotations_entity_updated;
DROP TABLE IF EXISTS annotations;

DROP INDEX IF EXISTS idx_watchlists_user_updated;
DROP TABLE IF EXISTS watchlists;

DROP INDEX IF EXISTS idx_saved_searches_user_updated;
DROP TABLE IF EXISTS saved_searches;

DROP TABLE IF EXISTS analyst_users;
