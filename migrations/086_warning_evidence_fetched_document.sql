-- ════════════════════════════════════════════════════════════════════════════
-- Migration 086: resolved source-document evidence must carry fetched content
-- ════════════════════════════════════════════════════════════════════════════
--
-- The 085 contract says `resolved` means "a real observation or fetched source
-- document". The runtime linker now enforces that for explicit document
-- references, and this constraint makes the database enforce it too:
--
--   evidence_kind = 'source_document' AND status = 'resolved'
--       ⇒ source_id IS NOT NULL AND content_hash IS NOT NULL
--
-- A metadata-only sources row (no body hash) can therefore never masquerade
-- as resolved evidence. Pre-existing rows that violate the rule are demoted to
-- `unresolved` with a machine-readable reason rather than deleted (they still
-- record that the warning claims a source).
--
-- Idempotent: safe to re-apply.

-- ── 1. Demote any resolved document links without a real hash ───────────────
UPDATE warning_evidence
SET status = 'unresolved',
    unresolved_reason = COALESCE(
        unresolved_reason,
        'recorded before 086: source document had no fetched content hash'
    )
WHERE status = 'resolved'
  AND evidence_kind = 'source_document'
  AND (source_id IS NULL OR content_hash IS NULL OR btrim(content_hash) = '');

-- ── 2. Enforce the contract ─────────────────────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'warning_evidence_resolved_document_is_fetched'
    ) THEN
        ALTER TABLE warning_evidence ADD CONSTRAINT warning_evidence_resolved_document_is_fetched
            CHECK (
                NOT (status = 'resolved' AND evidence_kind = 'source_document')
                OR (source_id IS NOT NULL
                    AND content_hash IS NOT NULL
                    AND btrim(content_hash) <> '')
            );
    END IF;
END $$;

COMMENT ON CONSTRAINT warning_evidence_resolved_document_is_fetched ON warning_evidence IS
    'A resolved source-document link must carry a fetched body hash; a metadata-only row is unresolved, not evidence';

-- Runtime role grants (production connects as a non-owner role).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_evidence TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_evidence TO apexintel;
    END IF;
END $$;
