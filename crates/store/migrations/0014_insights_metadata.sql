BEGIN;

ALTER TABLE insights
    ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;

UPDATE insights
SET metadata = '{}'::jsonb
WHERE metadata IS NULL;

COMMIT;