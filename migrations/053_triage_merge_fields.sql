-- 053_triage_merge_fields.sql
--
-- Merge-aware triage ingress (P0 audit #15).
--
-- Semantic dedup can now merge a duplicate submission into an existing queue
-- row instead of inserting a second one. Merging must not discard evidence:
-- the queue row accumulates the merged observation ids and source urls,
-- tracks how many times the item was seen, when it was last seen, and can be
-- escalated in severity when repeats justify it.
--
-- Idempotent (IF NOT EXISTS guards); safe to run against existing
-- deployments. Does not touch any applied migration (<= 052).

ALTER TABLE triage_queue
    ADD COLUMN IF NOT EXISTS occurrence_count       INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS last_seen_at           TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS merged_observation_ids UUID[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS merged_source_urls     TEXT[] NOT NULL DEFAULT '{}';

-- Existing rows were seen at creation time.
UPDATE triage_queue
SET last_seen_at = created_at
WHERE last_seen_at IS NULL;

-- Recent-window dedup lookups scan by (item_type, last_seen_at).
CREATE INDEX IF NOT EXISTS idx_triage_queue_item_type_last_seen
    ON triage_queue (item_type, last_seen_at DESC);

-- Grants for the non-owner application role (pattern from 050).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON triage_queue TO apexintel;
    END IF;
END $$;
