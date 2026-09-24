-- 052_entity_verification_evidence.sql
--
-- Evidence-backed entity verification (P0 audit #14).
--
-- Verification decisions must be auditable: every signal a provider observed
-- (official registry, GLEIF LEI, SEC EDGAR, verified corporate domain, RDAP,
-- corporate-site structured metadata, >= 2 independent source domains,
-- address/region agreement) is persisted row-for-row with its source and
-- confidence. Candidates only auto-register when a sufficient, non
-- contradictory combination is present; everything else goes to analyst
-- review and no canonical company is created.
--
-- Idempotent (IF NOT EXISTS guards); safe to run against existing
-- deployments. Does not touch any applied migration (<= 051).

CREATE TABLE IF NOT EXISTS entity_verification_evidence (
    id                BIGSERIAL PRIMARY KEY,
    candidate_id      UUID NOT NULL,
    verification_type TEXT NOT NULL,
    source_url        TEXT NOT NULL DEFAULT '',
    source_name       TEXT NOT NULL,
    matched_value     TEXT NOT NULL,
    confidence        DOUBLE PRECISION NOT NULL,
    observed_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT entity_verification_evidence_signal_key
        UNIQUE (candidate_id, verification_type, source_url, matched_value)
);

-- Look up all evidence for a candidate (verification + analyst review).
CREATE INDEX IF NOT EXISTS idx_entity_verification_evidence_candidate
    ON entity_verification_evidence (candidate_id, verification_type);

-- Freshness audits: which evidence was observed recently.
CREATE INDEX IF NOT EXISTS idx_entity_verification_evidence_observed
    ON entity_verification_evidence (observed_at DESC);

-- The application connects as a non-owner role in production; new tables need
-- explicit grants (same pattern as 050_heartbeat_grants.sql).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON entity_verification_evidence TO apexintel;
        GRANT USAGE, SELECT ON SEQUENCE entity_verification_evidence_id_seq TO apexintel;
    END IF;
END $$;
