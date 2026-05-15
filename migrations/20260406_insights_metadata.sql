-- Backfill migration for older deployments whose `insights` table predated the
-- metadata column now consumed by the API and worker.

ALTER TABLE insights
    ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;

UPDATE insights
SET metadata = '{}'::jsonb
WHERE metadata IS NULL;