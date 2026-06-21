-- ════════════════════════════════════════════════════════════════════════════
-- Security Intelligence Tables
-- ════════════════════════════════════════════════════════════════════════════
-- Creates the dedicated tables used by the security intelligence module
-- (crates/store/src/postgres/security.rs) for DNS posture monitoring,
-- CISA KEV catalog tracking, and lookalike/typosquat domain detection.
--
-- These tables were referenced by application code but had no corresponding
-- migration, causing "relation does not exist" errors on the /security page.
-- ════════════════════════════════════════════════════════════════════════════

-- ─────────────────────────────────────────────────────────────────────────────
-- DNS Posture Entries
-- Stores SPF/DKIM/DMARC check results per domain.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dns_posture_entries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id      UUID REFERENCES companies(id) ON DELETE SET NULL,
    domain          TEXT NOT NULL,
    has_spf         BOOLEAN NOT NULL DEFAULT FALSE,
    has_dkim        BOOLEAN NOT NULL DEFAULT FALSE,
    has_dmarc       BOOLEAN NOT NULL DEFAULT FALSE,
    dmarc_policy    TEXT,
    spf_record      TEXT,
    posture_score   DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    checked_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_dns_posture_domain_checked UNIQUE (domain, checked_at)
);

CREATE INDEX IF NOT EXISTS idx_dns_posture_company  ON dns_posture_entries(company_id);
CREATE INDEX IF NOT EXISTS idx_dns_posture_domain   ON dns_posture_entries(domain);
CREATE INDEX IF NOT EXISTS idx_dns_posture_checked  ON dns_posture_entries(checked_at DESC);

-- ─────────────────────────────────────────────────────────────────────────────
-- KEV (Known Exploited Vulnerabilities) Observations
-- Stores CISA KEV catalog entries relevant to tracked vendors/products.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kev_observations (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cve_id              TEXT NOT NULL,
    vulnerability_name  TEXT NOT NULL,
    vendor              TEXT,
    product             TEXT,
    date_added          DATE,
    due_date            DATE,
    notes               TEXT,
    relevance_score     DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    catalog_fetched_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_kev_cve_id UNIQUE (cve_id)
);

CREATE INDEX IF NOT EXISTS idx_kev_vendor     ON kev_observations(vendor) WHERE vendor IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_kev_product    ON kev_observations(product) WHERE product IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_kev_relevance  ON kev_observations(relevance_score DESC);
CREATE INDEX IF NOT EXISTS idx_kev_date_added ON kev_observations(date_added DESC) WHERE date_added IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Lookalike / Typosquat Domains
-- Stores detected lookalike domains that impersonate tracked companies.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS lookalike_domains (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id        UUID REFERENCES companies(id) ON DELETE SET NULL,
    original_domain   TEXT NOT NULL,
    lookalike_domain  TEXT NOT NULL,
    threat_type       TEXT NOT NULL DEFAULT 'typosquat',
    distance          INTEGER NOT NULL DEFAULT 1,
    active            BOOLEAN NOT NULL DEFAULT TRUE,
    detected_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_lookalike_pair UNIQUE (original_domain, lookalike_domain)
);

CREATE INDEX IF NOT EXISTS idx_lookalike_company  ON lookalike_domains(company_id);
CREATE INDEX IF NOT EXISTS idx_lookalike_original ON lookalike_domains(original_domain);
CREATE INDEX IF NOT EXISTS idx_lookalike_active   ON lookalike_domains(active) WHERE active = TRUE;
CREATE INDEX IF NOT EXISTS idx_lookalike_detected ON lookalike_domains(detected_at DESC);
