-- ════════════════════════════════════════════════════════════════════════════
-- Schema Integrity: Provenance, Crawl Runs, Supply Chain Relationships
-- ════════════════════════════════════════════════════════════════════════════
-- PURPOSE: Adds first-class provenance, crawl-batch tracking, and typed supply
-- chain relationships that were missing from the original schema. These tables
-- enable citation/replay (full source provenance), lineage tracking (crawl runs
-- as first-class objects), and a structured supply graph (beyond the polymorphic
-- graph_edges table).
--
-- All statements are idempotent (IF NOT EXISTS). No existing data is modified.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── 1. sources — full provenance for every fetched document ──────────────────
-- Replaces the fragmented provenance story (crawl_telemetry has no raw_payload,
-- source_evidence has no fetched_at). This is the canonical provenance table:
-- every URL the crawler fetches gets a row here with the raw payload, content
-- hash, HTTP status, and parser version — enabling full citation and replay.
CREATE TABLE IF NOT EXISTS sources (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    url             TEXT NOT NULL,
    source_kind     TEXT NOT NULL DEFAULT 'web',
    -- 'rss', 'web', 'rdap', 'sec_edgar', 'openalex', 'cve', 'dns', 'whois', 'gdelt'
    fetched_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    http_status     INTEGER,
    content_hash    VARCHAR(64),
    content_type    TEXT,
    raw_payload     JSONB,
    excerpt         TEXT,
    parser_version  TEXT,
    crawl_run_id    UUID,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sources_url ON sources(url);
CREATE INDEX IF NOT EXISTS idx_sources_fetched ON sources(fetched_at DESC);
CREATE INDEX IF NOT EXISTS idx_sources_hash ON sources(content_hash) WHERE content_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_sources_kind ON sources(source_kind);
CREATE INDEX IF NOT EXISTS idx_sources_crawl_run ON sources(crawl_run_id) WHERE crawl_run_id IS NOT NULL;

-- ── 2. crawl_runs — first-class crawl batch objects ───────────────────────────
-- Tracks each crawl batch (a worker invocation) as a first-class object with
-- its inputs, outputs, and lineage. This is what was missing — previously there
-- was no way to answer "which crawl produced this observation?".
CREATE TABLE IF NOT EXISTS crawl_runs (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    started_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at           TIMESTAMPTZ,
    sources_attempted     INTEGER NOT NULL DEFAULT 0,
    sources_succeeded     INTEGER NOT NULL DEFAULT 0,
    sources_failed        INTEGER NOT NULL DEFAULT 0,
    bytes_fetched         BIGINT NOT NULL DEFAULT 0,
    observations_produced INTEGER NOT NULL DEFAULT 0,
    status                TEXT NOT NULL DEFAULT 'running'
                              CHECK (status IN ('running', 'succeeded', 'failed', 'partial', 'cancelled')),
    error_summary         TEXT,
    metadata              JSONB DEFAULT '{}'::jsonb,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_crawl_runs_started ON crawl_runs(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_crawl_runs_status ON crawl_runs(status);

-- ── 3. company_supply_chain_relationships — typed supply graph ───────────────
-- A structured, typed relationship between a supplier and a buyer company.
-- This is distinct from the polymorphic graph_edges table: it captures real,
-- evidence-backed supply chain links with criticality and single-source flags.
CREATE TABLE IF NOT EXISTS company_supply_chain_relationships (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    supplier_company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    buyer_company_id    UUID REFERENCES companies(id) ON DELETE CASCADE,
    -- supplier_name/buyer_name denormalized for companies not yet in the DB
    supplier_name       TEXT,
    buyer_name          TEXT,
    relationship_type   TEXT NOT NULL DEFAULT 'tier1_supplier'
                            CHECK (relationship_type IN (
                                'tier1_supplier', 'tier2_supplier', 'tier3_supplier',
                                'customer', 'distributor', 'contract_manufacturer',
                                'foundry', 'subcontractor', 'joint_venture', 'other'
                            )),
    component_category  TEXT,
    criticality         TEXT NOT NULL DEFAULT 'medium'
                            CHECK (criticality IN ('low', 'medium', 'high', 'critical')),
    single_source       BOOLEAN NOT NULL DEFAULT false,
    evidence_url        TEXT,
    evidence_source     TEXT,
    confidence          FLOAT NOT NULL DEFAULT 0.5 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    notes               TEXT,
    first_seen_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_verified_at    TIMESTAMPTZ,
    metadata            JSONB DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_supply_rel_supplier ON company_supply_chain_relationships(supplier_company_id)
    WHERE supplier_company_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_supply_rel_buyer ON company_supply_chain_relationships(buyer_company_id)
    WHERE buyer_company_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_supply_rel_criticality ON company_supply_chain_relationships(criticality);
CREATE INDEX IF NOT EXISTS idx_supply_rel_single_source ON company_supply_chain_relationships(single_source)
    WHERE single_source = true;

-- ── 4. investigations — persistent investigation records ─────────────────────
-- Persists the results of InvestigationEngine runs so analysts can view past
-- investigations without re-running them. Bridges the investigation crate to
-- the database.
CREATE TABLE IF NOT EXISTS investigations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title           TEXT NOT NULL,
    investigation_type TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'created'
                        CHECK (status IN ('created', 'in_progress', 'completed', 'suspended', 'cancelled', 'failed')),
    priority        TEXT NOT NULL DEFAULT 'normal',
    target_entity_id    UUID,
    target_entity_type  TEXT,
    target_entity_name  TEXT NOT NULL,
    source_insight_id   UUID,
    overall_confidence  FLOAT,
    hypothesis_count    INTEGER NOT NULL DEFAULT 0,
    gap_count           INTEGER NOT NULL DEFAULT 0,
    result          JSONB,
    executive_summary TEXT,
    recommendations JSONB DEFAULT '[]'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_investigations_target ON investigations(target_entity_id, target_entity_type)
    WHERE target_entity_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_investigations_status ON investigations(status);
CREATE INDEX IF NOT EXISTS idx_investigations_insight ON investigations(source_insight_id)
    WHERE source_insight_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_investigations_created ON investigations(created_at DESC);

-- ── 5. updated_at triggers for new tables ────────────────────────────────────
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_supply_rel_updated ON company_supply_chain_relationships;
CREATE TRIGGER trg_supply_rel_updated BEFORE UPDATE ON company_supply_chain_relationships
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS trg_investigations_updated ON investigations;
CREATE TRIGGER trg_investigations_updated BEFORE UPDATE ON investigations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMIT;
