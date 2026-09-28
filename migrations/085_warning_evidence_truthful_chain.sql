-- ════════════════════════════════════════════════════════════════════════════
-- Migration 085: truthful warning evidence chain (audit P0-5/P0-6)
-- ════════════════════════════════════════════════════════════════════════════
--
-- The 083 `warning_evidence` link was implemented
-- by *fabricating* a `WarningSourceCitation` observation from the warning's
-- own title/description text and timestamping it `NOW()`. That is circular
-- evidence: a warning saying X created an "observation" saying X, which
-- analysis then cited as direct evidence for X — and every rebuild made an old
-- fact look freshly observed.
--
-- The truthful chain is:
--
--   FetchResult (fetched body + its hash) -> sources (source document) ->
--   parser/extractor -> observations (real extracted content) ->
--   warning_evidence (link) -> warning
--
-- This migration makes the link table represent that:
--
--   * `observation_id` is nullable — an evidence link may cite a source
--     document that has no extracted observation yet;
--   * `content_hash` is nullable and only ever carries the hash of actual
--     fetched source content (from `sources.content_hash` or the observation's
--     recorded hash), never a hash of warning text;
--   * `source_url` is nullable — an explicitly referenced observation may have
--     no URL, and inventing one would be fabrication;
--   * `status` is `resolved` (a real observation or fetched source document
--     backs the link) or `unresolved` (only a URL is known; acquisition has
--     not succeeded), with a machine-readable reason;
--   * `evidence_kind` names which reference resolved the link.
--
-- Any fabricated `WarningSourceCitation` evidence from the 083 linker is
-- removed: the link rows first, then the synthetic observation rows (never
-- deleting rows that a real claim still cites).
--
-- Idempotent: safe to re-apply.

-- ── 1. Remove the fabricated evidence rows ───────────────────────────────────

DELETE FROM warning_evidence we
USING observations o
WHERE we.observation_id = o.id
  AND o.observation_type = 'WarningSourceCitation';

-- Only delete synthetic observations nothing real depends on. `insight_claim_
-- evidence.evidence_id` is ON DELETE RESTRICT, so re-check it explicitly; the
-- other referencing tables cascade.
DELETE FROM observations o
WHERE o.observation_type = 'WarningSourceCitation'
  AND NOT EXISTS (
      SELECT 1 FROM insight_claim_evidence e WHERE e.evidence_id = o.id
  );

-- ── 2. Make the link table express resolved vs unresolved truthfully ────────

ALTER TABLE warning_evidence
    ALTER COLUMN observation_id DROP NOT NULL,
    ALTER COLUMN content_hash DROP NOT NULL,
    ALTER COLUMN source_url DROP NOT NULL;

ALTER TABLE warning_evidence
    ADD COLUMN IF NOT EXISTS evidence_kind TEXT NOT NULL DEFAULT 'source_document',
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'resolved',
    ADD COLUMN IF NOT EXISTS unresolved_reason TEXT;

-- The legacy uniqueness constraint keyed on a fabricated non-null source_url;
-- replace it with reference-scoped uniqueness that tolerates SQL NULLs.
ALTER TABLE warning_evidence DROP CONSTRAINT IF EXISTS warning_evidence_warning_id_source_url_key;

CREATE UNIQUE INDEX IF NOT EXISTS uq_warning_evidence_observation
    ON warning_evidence (warning_id, observation_id)
    WHERE observation_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_warning_evidence_source
    ON warning_evidence (warning_id, source_id)
    WHERE source_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_warning_evidence_url
    ON warning_evidence (warning_id, source_url)
    WHERE source_url IS NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'warning_evidence_status_values'
    ) THEN
        ALTER TABLE warning_evidence ADD CONSTRAINT warning_evidence_status_values
            CHECK (status IN ('resolved', 'unresolved'));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'warning_evidence_kind_values'
    ) THEN
        ALTER TABLE warning_evidence ADD CONSTRAINT warning_evidence_kind_values
            CHECK (evidence_kind IN ('observation', 'source_document'));
    END IF;
    -- Resolved means a real object backs the link; unresolved means only a
    -- URL is known.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'warning_evidence_resolved_reference'
    ) THEN
        ALTER TABLE warning_evidence ADD CONSTRAINT warning_evidence_resolved_reference
            CHECK (
                status <> 'resolved'
                OR observation_id IS NOT NULL
                OR source_id IS NOT NULL
            );
    END IF;
    -- An unresolved link must say why and must not carry a fabricated hash.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'warning_evidence_unresolved_shape'
    ) THEN
        ALTER TABLE warning_evidence ADD CONSTRAINT warning_evidence_unresolved_shape
            CHECK (
                status <> 'unresolved'
                OR (observation_id IS NULL AND source_id IS NULL
                    AND content_hash IS NULL AND unresolved_reason IS NOT NULL)
            );
    END IF;
END $$;

COMMENT ON COLUMN warning_evidence.status IS
    'resolved = backed by a real observation or fetched source document; unresolved = only a URL is known and acquisition has not succeeded';
COMMENT ON COLUMN warning_evidence.content_hash IS
    'sha256 of actual fetched source content (sources.content_hash or the observation''s recorded hash). NULL when unresolved. Never a hash of warning text.';
COMMENT ON COLUMN warning_evidence.unresolved_reason IS
    'Why the link is unresolved (for example: no fetched source document for URL)';

-- Runtime role grants (production connects as a non-owner role; see 050/060).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_evidence TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_evidence TO apexintel;
    END IF;
END $$;
