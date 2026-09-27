-- ════════════════════════════════════════════════════════════════════════════
-- Migration 082: explicit warning evidence links (audit warning-evidence item)
-- ════════════════════════════════════════════════════════════════════════════
--
-- Warning source URLs used to exist only as strings inside
-- `warnings.source_urls`. Warning analysis gathered observations by
-- `entity_ids` and used the URLs only for domain/reliability aggregation, so a
-- warning with a primary source URL and no entity ids had no citable evidence
-- at all: the model could cite nothing, and no claim could be traced back to
-- the document that produced the warning.
--
-- This migration installs the upstream chain the analysis must consume:
--
--   sources (source document, content_hash) -> observations (observation
--   extracted from that document) -> warning_evidence (link to the warning)
--
-- Each row of `warning_evidence` is one citable evidence link for a warning:
-- the resolved source document (`source_id`, nullable because `sources` rows
-- can be pruned by retention), the observation extracted from it, the exact
-- source URL, and the preserved `content_hash` of the document/claim content.
-- The link is idempotent per `(warning_id, source_url)`, so a warning that is
-- deterministically deduplicated and merged on recurrence re-affirms its
-- existing evidence instead of duplicating it.
--
-- Analysis consumes these links as its direct evidence set and only falls back
-- to entity observations when a warning has no explicit links.
--
-- Idempotent: safe to re-apply (IF NOT EXISTS).

CREATE TABLE IF NOT EXISTS warning_evidence (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    warning_id     UUID NOT NULL REFERENCES warnings(id) ON DELETE CASCADE,
    -- Resolved source document. Nullable: retention can prune `sources`
    -- rows; the observation and the preserved hash keep the citation usable.
    source_id      UUID REFERENCES sources(id) ON DELETE SET NULL,
    observation_id UUID NOT NULL REFERENCES observations(id) ON DELETE CASCADE,
    source_url     TEXT NOT NULL CHECK (btrim(source_url) <> ''),
    -- Content hash preserved at link time (sha256 hex of the canonical
    -- citation content: warning title + description + source URL).
    content_hash   TEXT NOT NULL CHECK (btrim(content_hash) <> ''),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (warning_id, source_url)
);

-- Reverse lookups: which warnings cite this observation / source document.
CREATE INDEX IF NOT EXISTS idx_warning_evidence_observation
    ON warning_evidence (observation_id);
CREATE INDEX IF NOT EXISTS idx_warning_evidence_source
    ON warning_evidence (source_id)
    WHERE source_id IS NOT NULL;

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
