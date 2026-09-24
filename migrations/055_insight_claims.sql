-- 051_insight_claims.sql
--
-- Claim-level evidence for insights (audit P0 #23).
--
-- Each generated insight is decomposed into discrete claims. Every claim
-- carries the ids of the evidence rows it is grounded in (`evidence_ids`), a
-- confidence value, and a kind distinguishing what the system actually knows:
--
--   observed       — a fact stated in the cited evidence
--   inference      — the system's interpretation of the evidence
--   recommendation — a suggested action, not a fact
--   unknown        — the claim's provenance could not be established
--
-- Rendering only cites evidence_ids that exist; claims without evidence are
-- explicitly marked `unknown`, never given fabricated ids.
--
-- Idempotent: safe to re-apply (CREATE ... IF NOT EXISTS + guarded backfill).

CREATE TABLE IF NOT EXISTS insight_claims (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    insight_id   UUID NOT NULL REFERENCES insights(id) ON DELETE CASCADE,
    claim        TEXT NOT NULL,
    evidence_ids UUID[] NOT NULL DEFAULT '{}'::uuid[],
    confidence   DOUBLE PRECISION,
    claim_kind   TEXT NOT NULL DEFAULT 'unknown'
        CHECK (claim_kind IN ('observed', 'inference', 'recommendation', 'unknown')),
    claim_hash   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_insight_claims_insight_hash
    ON insight_claims (insight_id, claim_hash);

CREATE INDEX IF NOT EXISTS idx_insight_claims_insight
    ON insight_claims (insight_id, created_at ASC, id ASC);

-- Backfill existing insights. Insights generated before claim extraction have
-- no structured claims; they are marked UNKNOWN with no evidence ids rather
-- than being given fabricated claims or citations. Idempotent: only insights
-- that currently have zero claims are touched.
INSERT INTO insight_claims (insight_id, claim, evidence_ids, confidence, claim_kind, claim_hash)
SELECT i.id,
       left(COALESCE(NULLIF(btrim(i.title), ''), 'Untitled insight'), 500),
       '{}'::uuid[],
       NULL,
       'unknown',
       md5('backfill:unknown:' || i.id::text)
FROM insights i
WHERE NOT EXISTS (SELECT 1 FROM insight_claims c WHERE c.insight_id = i.id);

-- Runtime role grants (production connects as a non-owner role; see 050).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON insight_claims TO apexintel;
    END IF;
END $$;
