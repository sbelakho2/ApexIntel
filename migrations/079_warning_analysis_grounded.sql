-- ════════════════════════════════════════════════════════════════════════════
-- Migration 079: grounded warning-analysis runs + claim evidence integrity
-- ════════════════════════════════════════════════════════════════════════════
--
-- Audit P0-3/P0-4/P1-5: warning analysis used to be a free-prose LLM call whose
-- paragraphs were parsed by index, labelled with count-derived "reliability"
-- strings, and discarded after the response — nothing was persisted, nothing
-- deduplicated, and no claim could be traced back to an observation.
--
-- This migration makes analysis durable and evidence-bound:
--
--   warning_analysis_runs
--     one row per analysis attempt. Carries the model, prompt version and an
--     evidence digest so repeated clicks on identical evidence deduplicate
--     (partial unique indexes below) and results are cacheable/comparable.
--
--   warning_analysis_claims
--     the validated claims/impact/actions of a run, through the same
--     claim-kind policy as insight claims (migration 078):
--       observed       — must cite at least one existing evidence row
--       inference      — must cite at least one existing evidence row
--       recommendation — may cite zero direct evidence
--       unknown        — cites no evidence (never produced by generation)
--
--   warning_analysis_claim_evidence
--     normalized, foreign-key-checked citation join. A citation is either an
--     observation id or an insight id (the two kinds of evidence the analysis
--     can be grounded in); exactly one of the two columns is set.
--
-- Idempotent: safe to re-apply (IF NOT EXISTS / guarded reconciliation).

-- ── Analysis runs ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS warning_analysis_runs (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    warning_id             UUID NOT NULL REFERENCES warnings(id) ON DELETE CASCADE,
    status                 TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    requested_by           TEXT,
    model                  TEXT NOT NULL,
    prompt_version         TEXT NOT NULL,
    evidence_digest        TEXT NOT NULL,
    observations_available INTEGER NOT NULL DEFAULT 0
        CHECK (observations_available >= 0),
    observations_sent      INTEGER NOT NULL DEFAULT 0
        CHECK (observations_sent >= 0 AND observations_sent <= observations_available),
    insights_available     INTEGER NOT NULL DEFAULT 0
        CHECK (insights_available >= 0),
    insights_sent          INTEGER NOT NULL DEFAULT 0
        CHECK (insights_sent >= 0 AND insights_sent <= insights_available),
    output                 JSONB,
    error                  TEXT,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at             TIMESTAMPTZ,
    finished_at            TIMESTAMPTZ,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT warning_analysis_runs_success_has_output
        CHECK (status <> 'succeeded' OR output IS NOT NULL),
    CONSTRAINT warning_analysis_runs_failure_has_error
        CHECK (status <> 'failed' OR error IS NOT NULL)
);

-- Dedupe identical in-flight work: at most one queued/running run per
-- (warning, evidence digest), regardless of prompt version.
CREATE UNIQUE INDEX IF NOT EXISTS uq_warning_analysis_runs_inflight
    ON warning_analysis_runs (warning_id, evidence_digest)
    WHERE status IN ('queued', 'running');

-- Cache/comparability: one succeeded run per (warning, evidence digest, prompt
-- version). A new prompt version produces a new comparable run.
CREATE UNIQUE INDEX IF NOT EXISTS uq_warning_analysis_runs_succeeded
    ON warning_analysis_runs (warning_id, evidence_digest, prompt_version)
    WHERE status = 'succeeded';

CREATE INDEX IF NOT EXISTS idx_warning_analysis_runs_warning
    ON warning_analysis_runs (warning_id, created_at DESC, id DESC);

-- ── Analysis claims ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS warning_analysis_claims (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id         UUID NOT NULL REFERENCES warning_analysis_runs(id) ON DELETE CASCADE,
    section        TEXT NOT NULL CHECK (section IN ('claim', 'impact', 'action')),
    claim          TEXT NOT NULL CHECK (btrim(claim) <> ''),
    claim_kind     TEXT NOT NULL
        CHECK (claim_kind IN ('observed', 'inference', 'recommendation', 'unknown')),
    confidence     DOUBLE PRECISION
        CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    evidence_count INTEGER NOT NULL DEFAULT 0 CHECK (evidence_count >= 0),
    claim_hash     TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, claim_hash)
);

-- Table-local policy enforcement (see 078 for the rationale).
ALTER TABLE warning_analysis_claims
    DROP CONSTRAINT IF EXISTS warning_analysis_claims_evidence_policy;
ALTER TABLE warning_analysis_claims
    ADD CONSTRAINT warning_analysis_claims_evidence_policy CHECK (
        (claim_kind IN ('observed', 'inference') AND evidence_count >= 1)
        OR (claim_kind = 'unknown' AND evidence_count = 0)
        OR claim_kind = 'recommendation'
    );

-- ── Citation join (FK integrity per evidence kind) ───────────────────────────
--
-- The evidence FKs cascade rather than restrict: production deletes
-- observations (DNS-posture / lookalike delete-then-insert scans and the
-- `cleanup_old_observations` retention function), and blocking those deletes
-- would break existing cleanup paths. Instead, deleting cited evidence
-- cascades its citation rows and the count trigger below downgrades any
-- observed/inference claim that loses its last citation to `unknown` — an
-- explicit "provenance no longer verifiable" state rather than a stale claim
-- or a failed delete.
CREATE TABLE IF NOT EXISTS warning_analysis_claim_evidence (
    claim_id       UUID NOT NULL
        REFERENCES warning_analysis_claims(id) ON DELETE CASCADE,
    observation_id UUID REFERENCES observations(id) ON DELETE CASCADE,
    insight_id     UUID REFERENCES insights(id) ON DELETE CASCADE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT warning_analysis_claim_evidence_exactly_one CHECK (
        (observation_id IS NOT NULL) <> (insight_id IS NOT NULL)
    ),
    evidence_key   TEXT GENERATED ALWAYS AS (
        CASE
            WHEN observation_id IS NOT NULL THEN 'observation:' || observation_id::text
            ELSE 'insight:' || insight_id::text
        END
    ) STORED,
    PRIMARY KEY (claim_id, evidence_key)
);

-- Reverse lookups (which analysis claims cite this observation / insight).
CREATE INDEX IF NOT EXISTS idx_warning_analysis_claim_evidence_observation
    ON warning_analysis_claim_evidence (observation_id)
    WHERE observation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_warning_analysis_claim_evidence_insight
    ON warning_analysis_claim_evidence (insight_id)
    WHERE insight_id IS NOT NULL;

-- Keep evidence_count consistent with the join table. Recompute (rather than
-- increment) so the count cannot drift under retries or concurrent inserts.
-- A claim whose last citation disappears (evidence deleted, or a re-run that
-- no longer cites it) is downgraded to `unknown`: its provenance can no
-- longer be vouched for, and the policy CHECK forbids observed/inference
-- claims with zero evidence.
CREATE OR REPLACE FUNCTION refresh_warning_analysis_claim_evidence_count()
RETURNS trigger AS $$
DECLARE
    target_id UUID;
    link_count INTEGER;
BEGIN
    target_id := COALESCE(NEW.claim_id, OLD.claim_id);
    SELECT COUNT(*)::int INTO link_count
    FROM warning_analysis_claim_evidence
    WHERE claim_id = target_id;
    UPDATE warning_analysis_claims
    SET evidence_count = link_count,
        claim_kind = CASE
            WHEN link_count = 0 AND claim_kind IN ('observed', 'inference')
                THEN 'unknown'
            ELSE claim_kind
        END
    WHERE id = target_id;
    RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS warning_analysis_claim_evidence_count
    ON warning_analysis_claim_evidence;
CREATE TRIGGER warning_analysis_claim_evidence_count
    AFTER INSERT OR DELETE ON warning_analysis_claim_evidence
    FOR EACH ROW EXECUTE FUNCTION refresh_warning_analysis_claim_evidence_count();

-- Cross-table enforcement for writers that bypass the application path.
-- Deferred: checked once per transaction at COMMIT, so claim and evidence can
-- be written in any order and the final state is what must satisfy the policy.
CREATE OR REPLACE FUNCTION warning_analysis_claim_evidence_policy_guard()
RETURNS trigger AS $$
DECLARE
    target_id UUID;
    target_kind TEXT;
    link_count INTEGER;
BEGIN
    IF TG_TABLE_NAME = 'warning_analysis_claim_evidence' THEN
        target_id := COALESCE(NEW.claim_id, OLD.claim_id);
    ELSE
        target_id := COALESCE(NEW.id, OLD.id);
    END IF;

    SELECT claim_kind INTO target_kind
    FROM warning_analysis_claims WHERE id = target_id;
    IF target_kind IS NULL THEN
        RETURN NULL; -- claim deleted; its links cascade away
    END IF;

    SELECT COUNT(*)::int INTO link_count
    FROM warning_analysis_claim_evidence WHERE claim_id = target_id;

    IF target_kind IN ('observed', 'inference') AND link_count < 1 THEN
        RAISE EXCEPTION
            'warning analysis claim % of kind % requires at least one evidence row',
            target_id, target_kind
            USING ERRCODE = 'check_violation';
    END IF;
    IF target_kind = 'unknown' AND link_count > 0 THEN
        RAISE EXCEPTION
            'warning analysis claim % is unknown and must not cite evidence',
            target_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS warning_analysis_claims_evidence_policy_guard
    ON warning_analysis_claims;
CREATE CONSTRAINT TRIGGER warning_analysis_claims_evidence_policy_guard
    AFTER INSERT OR UPDATE OF claim_kind ON warning_analysis_claims
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION warning_analysis_claim_evidence_policy_guard();

DROP TRIGGER IF EXISTS warning_analysis_claim_evidence_policy_guard
    ON warning_analysis_claim_evidence;
CREATE CONSTRAINT TRIGGER warning_analysis_claim_evidence_policy_guard
    AFTER INSERT OR UPDATE OR DELETE ON warning_analysis_claim_evidence
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION warning_analysis_claim_evidence_policy_guard();

-- Runtime role grants (production connects as a non-owner role; see 050/060).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_runs TO apexintel_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_claims TO apexintel_app;
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_claim_evidence TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_runs TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_claims TO apexintel;
        GRANT SELECT, INSERT, UPDATE, DELETE ON warning_analysis_claim_evidence TO apexintel;
    END IF;
END $$;
