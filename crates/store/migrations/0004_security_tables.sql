-- ApexIntel Schema Migration – security scan result tables
-- DNS posture, CISA KEV observations, and lookalike domain detections.

CREATE TABLE IF NOT EXISTS dns_posture_entries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    domain TEXT NOT NULL,
    has_spf BOOLEAN NOT NULL DEFAULT FALSE,
    has_dkim BOOLEAN NOT NULL DEFAULT FALSE,
    has_dmarc BOOLEAN NOT NULL DEFAULT FALSE,
    dmarc_policy TEXT,
    spf_record TEXT,
    posture_score FLOAT NOT NULL DEFAULT 0,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (domain, checked_at)
);

CREATE INDEX IF NOT EXISTS idx_dns_posture_company ON dns_posture_entries (company_id);
CREATE INDEX IF NOT EXISTS idx_dns_posture_domain ON dns_posture_entries (domain);

CREATE TABLE IF NOT EXISTS kev_observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cve_id TEXT NOT NULL,
    vulnerability_name TEXT NOT NULL,
    vendor TEXT,
    product TEXT,
    date_added DATE,
    due_date DATE,
    notes TEXT,
    relevance_score FLOAT NOT NULL DEFAULT 0,
    affected_company_ids UUID[],
    catalog_fetched_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (cve_id)
);

CREATE INDEX IF NOT EXISTS idx_kev_observations_cve ON kev_observations (cve_id);
CREATE INDEX IF NOT EXISTS idx_kev_observations_relevance ON kev_observations (relevance_score DESC);

CREATE TABLE IF NOT EXISTS lookalike_domains (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    original_domain TEXT NOT NULL,
    lookalike_domain TEXT NOT NULL,
    threat_type TEXT NOT NULL DEFAULT 'typosquat',
    distance INT NOT NULL DEFAULT 1,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (original_domain, lookalike_domain)
);

CREATE INDEX IF NOT EXISTS idx_lookalike_company ON lookalike_domains (company_id);
CREATE INDEX IF NOT EXISTS idx_lookalike_original ON lookalike_domains (original_domain);
