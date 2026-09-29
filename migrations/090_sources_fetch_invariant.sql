-- ════════════════════════════════════════════════════════════════════════════
-- Migration 090: a content hash on `sources` implies the document was fetched
-- ════════════════════════════════════════════════════════════════════════════
--
-- The warning-evidence contract (migrations 085/086) says a resolved
-- source-document link must carry fetched content. The warning_evidence CHECK
-- can only see its own columns, so it cannot prove the referenced `sources`
-- row has `fetched_at`. The cleanest invariant lives on `sources` itself:
--
--     content_hash IS NOT NULL  ⇒  fetched_at IS NOT NULL
--
-- A recorded body hash without a fetch time is a contradiction; any such rows
-- are demoted to "no hash" (metadata-only) rather than deleted, and the
-- constraint prevents new ones.
--
-- Idempotent: safe to re-apply.

UPDATE sources
SET content_hash = NULL
WHERE content_hash IS NOT NULL
  AND fetched_at IS NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'sources_content_hash_requires_fetch'
    ) THEN
        ALTER TABLE sources ADD CONSTRAINT sources_content_hash_requires_fetch
            CHECK (content_hash IS NULL OR fetched_at IS NOT NULL);
    END IF;
END $$;

COMMENT ON CONSTRAINT sources_content_hash_requires_fetch ON sources IS
    'A recorded content hash means the document was fetched; a metadata-only row has no hash';
