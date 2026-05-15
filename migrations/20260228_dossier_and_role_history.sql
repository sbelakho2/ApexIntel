-- ═══════════════════════════════════════════════════════════════════════════════
-- Migration: Dossier files & role history for POIs and Companies
-- Date: 2026-02-28
-- Purpose:
--   1. Track POI role/job changes over time (role_history)
--   2. Create persistent "dossier entries" (file entries) for POIs & companies
--   3. Track company profile changes over time (company_changes)
-- ═══════════════════════════════════════════════════════════════════════════════

-- ─── 1. Role History: tracks every known position a POI has held ─────────────
CREATE TABLE IF NOT EXISTS role_history (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id   UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    org_id      UUID REFERENCES companies(id) ON DELETE SET NULL,
    org_name    TEXT NOT NULL,
    title       TEXT NOT NULL,
    role_family TEXT,
    start_date  TIMESTAMPTZ,
    end_date    TIMESTAMPTZ,           -- NULL = current position
    source_url  TEXT,                   -- evidence URL
    confidence  FLOAT DEFAULT 0.8,     -- how sure we are (0-1)
    verified    BOOLEAN DEFAULT FALSE,  -- human-verified?
    metadata    JSONB DEFAULT '{}',
    created_at  TIMESTAMPTZ DEFAULT now(),
    updated_at  TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_role_history_person ON role_history(person_id, start_date DESC);
CREATE INDEX IF NOT EXISTS idx_role_history_org    ON role_history(org_id);

-- ─── 2. Dossier Entries: persistent, versioned "file" entries ────────────────
-- Each entry is a verified fact or intelligence item attached to an entity.
-- entity_type: 'person' or 'company'
-- category: financial, leadership, capability, risk, competitive, strategic, etc.
CREATE TABLE IF NOT EXISTS dossier_entries (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type   TEXT NOT NULL CHECK (entity_type IN ('person', 'company')),
    entity_id     UUID NOT NULL,
    category      TEXT NOT NULL,        -- e.g. 'leadership', 'financial', 'capability', 'risk', 'competitive'
    title         TEXT NOT NULL,
    content       TEXT NOT NULL,
    source_urls   TEXT[],               -- evidence links
    confidence    FLOAT DEFAULT 0.7,
    verified      BOOLEAN DEFAULT FALSE,
    supersedes_id UUID REFERENCES dossier_entries(id) ON DELETE SET NULL, -- previous version of this entry
    valid_from    TIMESTAMPTZ DEFAULT now(),
    valid_until   TIMESTAMPTZ,          -- NULL = still current
    author        TEXT DEFAULT 'system', -- 'system', 'crawl', 'llm', 'analyst'
    metadata      JSONB DEFAULT '{}',
    created_at    TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_dossier_entity     ON dossier_entries(entity_type, entity_id, valid_until);
CREATE INDEX IF NOT EXISTS idx_dossier_category   ON dossier_entries(entity_type, entity_id, category);
CREATE INDEX IF NOT EXISTS idx_dossier_supersedes ON dossier_entries(supersedes_id);

-- ─── 3. Company Changes: changelog tracking for competitor intelligence ──────
CREATE TABLE IF NOT EXISTS company_changes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id   UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    change_type  TEXT NOT NULL,         -- 'leadership', 'financial', 'expansion', 'contraction', 'capability', 'certification', 'risk_score', 'merger', 'name'
    field_name   TEXT,                  -- which field changed
    old_value    TEXT,
    new_value    TEXT,
    description  TEXT,
    source_url   TEXT,
    detected_at  TIMESTAMPTZ DEFAULT now(),
    confidence   FLOAT DEFAULT 0.7,
    metadata     JSONB DEFAULT '{}',
    created_at   TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_company_changes_company ON company_changes(company_id, detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_company_changes_type    ON company_changes(change_type, detected_at DESC);

-- ─── 4. Person Changes: changelog for POI profile updates ───────────────────
CREATE TABLE IF NOT EXISTS person_changes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id    UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    change_type  TEXT NOT NULL,         -- 'job_change', 'role_change', 'org_change', 'contact_update', 'score_shift', 'new_artifact'
    field_name   TEXT,
    old_value    TEXT,
    new_value    TEXT,
    description  TEXT,
    source_url   TEXT,
    detected_at  TIMESTAMPTZ DEFAULT now(),
    confidence   FLOAT DEFAULT 0.7,
    metadata     JSONB DEFAULT '{}',
    created_at   TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_person_changes_person ON person_changes(person_id, detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_person_changes_type   ON person_changes(change_type, detected_at DESC);
