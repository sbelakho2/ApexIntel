-- Migration 078: claim-level evidence integrity (normalized join table + counts)
-- Migration 078: 
--
-- Claim-level evidence integrity (audit P0 #23).
--
-- 055_insight_claims.sql stored each claim's evidence as a UUID array with no
-- referential integrity: an id could be invented, point at a deleted
-- observation, or be stale forever, and nothing stopped an `observed` claim
-- from citing no evidence at all. This migration normalizes that array into a
-- real join table and enforces the claim-kind policy in the database:
--
--   observed       — must cite at least one existing observation
--   inference      — must cite at least one existing observation
--   recommendation — may cite zero direct evidence (its support comes from
--                    the claims it links to)
--   unknown        — cites no evidence (provenance was not established)
--
-- A plain CHECK cannot reference another table, so the policy is enforced by
-- a denormalized `evidence_count` on `insight_claims` maintained by triggers,
-- constrained by CHECK, plus a DEFERRABLE constraint trigger that validates
-- the join table itself. The deferred trigger lets an application insert a
-- claim and its evidence in the same transaction in any order while still
-- rejecting a commit whose final state violates the policy.
--
-- Idempotent: safe to re-apply (IF NOT EXISTS / guarded reconciliation).

CREATE TABLE IF NOT EXISTS insight_claim_evidence (
    claim_id    UUID NOT NULL REFERENCES insight_claims(id) ON DELETE CASCADE,
    evidence_id UUID NOT NULL REFERENCES observations(id) ON DELETE RESTRICT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (claim_id, evidence_id)
);

-- Reverse lookups (which claims cite this observation).
CREATE INDEX IF NOT EXISTS idx_insight_claim_evidence_evidence
    ON insight_claim_evidence (evidence_id);

-- Backfill from the legacy UUID arrays, but only where the cited observation
-- actually exists. Dangling ids are deliberately NOT fabricated into links;
-- the reconciliation below turns those claims into `unknown`.
INSERT INTO insight_claim_evidence (claim_id, evidence_id)
SELECT c.id, cited.evidence_id
FROM insight_claims c
CROSS JOIN LATERAL unnest(c.evidence_ids) AS cited(evidence_id)
JOIN observations o ON o.id = cited.evidence_id
ON CONFLICT (claim_id, evidence_id) DO NOTHING;

-- Denormalized count so the CHECK below can enforce the policy table-locally.
ALTER TABLE insight_claims
    ADD COLUMN IF NOT EXISTS evidence_count INTEGER NOT NULL DEFAULT 0;

UPDATE insight_claims c
SET evidence_count = (
    SELECT COUNT(*)::int FROM insight_claim_evidence e WHERE e.claim_id = c.id
);

-- `unknown` claims must not carry links: an unestablished provenance cannot
-- cite anything. Clear the join rows for those (there are none from the 055
-- backfill, but generation could have written kind=unknown with ids).
DELETE FROM insight_claim_evidence e
USING insight_claims c
WHERE e.claim_id = c.id AND c.claim_kind = 'unknown';

-- Reconcile pre-existing claims: observed/inference claims whose cited
-- evidence does not exist cannot be vouched for. Downgrade them to `unknown`
-- and drop the dangling ids instead of keeping a fabricated citation.
UPDATE insight_claims
SET claim_kind = 'unknown',
    evidence_ids = '{}'::uuid[],
    evidence_count = 0
WHERE claim_kind IN ('observed', 'inference')
  AND evidence_count = 0;

-- Policy enforcement (table-local part):
--   observed / inference  => at least one evidence link
--   unknown               => no evidence links
--   recommendation        => any count (zero direct evidence allowed)
ALTER TABLE insight_claims DROP CONSTRAINT IF EXISTS insight_claims_evidence_policy;
ALTER TABLE insight_claims ADD CONSTRAINT insight_claims_evidence_policy CHECK (
    (claim_kind IN ('observed', 'inference') AND evidence_count >= 1)
    OR (claim_kind = 'unknown' AND evidence_count = 0)
    OR claim_kind = 'recommendation'
);

-- Keep evidence_count consistent with the join table. Recompute (rather than
-- increment) so the count cannot drift under retries or concurrent inserts.
CREATE OR REPLACE FUNCTION refresh_insight_claim_evidence_count() RETURNS trigger AS $$
DECLARE
    target_id UUID;
BEGIN
    target_id := COALESCE(NEW.claim_id, OLD.claim_id);
    UPDATE insight_claims
    SET evidence_count = (
        SELECT COUNT(*)::int FROM insight_claim_evidence e WHERE e.claim_id = target_id
    )
    WHERE id = target_id;
    RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS insight_claim_evidence_count ON insight_claim_evidence;
CREATE TRIGGER insight_claim_evidence_count
    AFTER INSERT OR DELETE ON insight_claim_evidence
    FOR EACH ROW EXECUTE FUNCTION refresh_insight_claim_evidence_count();

-- Cross-table enforcement for writers that bypass the application path.
-- Deferred: checked once per transaction at COMMIT, so claim and evidence can
-- be written in any order and the final state is what must satisfy the policy.
CREATE OR REPLACE FUNCTION insight_claim_evidence_policy_guard() RETURNS trigger AS $$
DECLARE
    target_id UUID;
    target_kind TEXT;
    link_count INTEGER;
BEGIN
    IF TG_TABLE_NAME = 'insight_claim_evidence' THEN
        target_id := COALESCE(NEW.claim_id, OLD.claim_id);
    ELSE
        target_id := COALESCE(NEW.id, OLD.id);
    END IF;

    SELECT claim_kind INTO target_kind FROM insight_claims WHERE id = target_id;
    IF target_kind IS NULL THEN
        RETURN NULL; -- claim deleted; its links cascade away
    END IF;

    SELECT COUNT(*)::int INTO link_count
    FROM insight_claim_evidence WHERE claim_id = target_id;

    IF target_kind IN ('observed', 'inference') AND link_count < 1 THEN
        RAISE EXCEPTION
            'insight claim % of kind % requires at least one evidence row',
            target_id, target_kind
            USING ERRCODE = 'check_violation';
    END IF;
    IF target_kind = 'unknown' AND link_count > 0 THEN
        RAISE EXCEPTION
            'insight claim % is unknown and must not cite evidence',
            target_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS insight_claims_evidence_policy_guard ON insight_claims;
CREATE CONSTRAINT TRIGGER insight_claims_evidence_policy_guard
    AFTER INSERT OR UPDATE OF claim_kind ON insight_claims
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION insight_claim_evidence_policy_guard();

DROP TRIGGER IF EXISTS insight_claim_evidence_policy_guard ON insight_claim_evidence;
CREATE CONSTRAINT TRIGGER insight_claim_evidence_policy_guard
    AFTER INSERT OR UPDATE OR DELETE ON insight_claim_evidence
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION insight_claim_evidence_policy_guard();

-- Runtime role grants (production connects as a non-owner role; see 050/060).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON insight_claim_evidence TO apexintel_app;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON insight_claim_evidence TO apexintel;
    END IF;
END $$;
