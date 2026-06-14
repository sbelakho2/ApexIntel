-- ════════════════════════════════════════════════════════════════════════════
-- Phase 4.3: User Experience Enhancement - Database Schema Extensions
-- ApexIntel OSINT Platform
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ─── Strategic Opportunities ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS strategic_opportunities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title           TEXT NOT NULL,
    description     TEXT,
    opportunity_type TEXT NOT NULL,
    priority_score  FLOAT NOT NULL DEFAULT 0.0,
    confidence      FLOAT NOT NULL DEFAULT 0.5,
    entity_id       UUID,
    entity_type     TEXT,
    region          TEXT,
    estimated_value TEXT,
    recommended_actions JSONB DEFAULT '[]'::jsonb,
    owner_id        TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    due_date        TIMESTAMPTZ,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_opportunities_priority ON strategic_opportunities(priority_score DESC);
CREATE INDEX IF NOT EXISTS idx_opportunities_type    ON strategic_opportunities(opportunity_type);
CREATE INDEX IF NOT EXISTS idx_opportunities_status  ON strategic_opportunities(status);
CREATE INDEX IF NOT EXISTS idx_opportunities_owner  ON strategic_opportunities(owner_id);

-- ─── Critical Threats ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS critical_threats (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title           TEXT NOT NULL,
    description     TEXT,
    threat_type     TEXT NOT NULL,
    severity        TEXT NOT NULL DEFAULT 'medium',
    impact_score    FLOAT NOT NULL DEFAULT 0.5,
    confidence      FLOAT NOT NULL DEFAULT 0.5,
    entity_id       UUID,
    entity_type     TEXT,
    region          TEXT,
    mitigation_steps JSONB DEFAULT '[]'::jsonb,
    owner_id        TEXT,
    status          TEXT NOT NULL DEFAULT 'active',
    sla_deadline    TIMESTAMPTZ,
    resolved_at     TIMESTAMPTZ,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_threats_severity    ON critical_threats(severity);
CREATE INDEX IF NOT EXISTS idx_threats_status      ON critical_threats(status);
CREATE INDEX IF NOT EXISTS idx_threats_sla        ON critical_threats(sla_deadline) WHERE status = 'active';

-- ─── Investigation Workspaces ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS investigation_workspaces (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    description     TEXT,
    workspace_type   TEXT NOT NULL DEFAULT 'ad-hoc',
    owner_id        TEXT NOT NULL,
    team_id         TEXT,
    status          TEXT NOT NULL DEFAULT 'active',
    visibility      TEXT NOT NULL DEFAULT 'private',
    tags            TEXT[] DEFAULT '{}',
    entity_focus    JSONB DEFAULT '[]'::jsonb,
    findings        TEXT,
    conclusions     TEXT,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_workspaces_owner   ON investigation_workspaces(owner_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_team    ON investigation_workspaces(team_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_status  ON investigation_workspaces(status);
CREATE INDEX IF NOT EXISTS idx_workspaces_updated ON investigation_workspaces(updated_at DESC);

-- ─── Workspace Assignments ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS workspace_assignments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL REFERENCES investigation_workspaces(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'contributor',
    assigned_by     TEXT NOT NULL,
    assigned_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_assignments_workspace ON workspace_assignments(workspace_id);
CREATE INDEX IF NOT EXISTS idx_assignments_user      ON workspace_assignments(user_id);

-- ─── Activity Feed ───────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS activity_feed (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id        TEXT NOT NULL,
    actor_name      TEXT NOT NULL,
    action_type     TEXT NOT NULL,
    entity_type     TEXT,
    entity_id       UUID,
    entity_name     TEXT,
    details         JSONB DEFAULT '{}'::jsonb,
    workspace_id    UUID,
    team_id         TEXT,
    visibility      TEXT NOT NULL DEFAULT 'team',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_feed_workspace ON activity_feed(workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_team      ON activity_feed(team_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_actor     ON activity_feed(actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_recent   ON activity_feed(created_at DESC);

-- ─── Investigation Shares ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS investigation_shares (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL REFERENCES investigation_workspaces(id) ON DELETE CASCADE,
    shared_by       TEXT NOT NULL,
    shared_with     TEXT NOT NULL,
    share_type      TEXT NOT NULL DEFAULT 'view',
    access_level    TEXT NOT NULL DEFAULT 'read',
    message         TEXT,
    expires_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_shares_workspace ON investigation_shares(workspace_id);
CREATE INDEX IF NOT EXISTS idx_shares_shared    ON investigation_shares(shared_with);

-- ─── Daily Priority Queue ────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS daily_priority_queue (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         TEXT NOT NULL,
    queue_date      DATE NOT NULL,
    item_type       TEXT NOT NULL,
    item_id         UUID NOT NULL,
    item_title      TEXT NOT NULL,
    priority        INT NOT NULL DEFAULT 50,
    status          TEXT NOT NULL DEFAULT 'pending',
    notes           TEXT,
    completed_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, queue_date, item_type, item_id)
);

CREATE INDEX IF NOT EXISTS idx_queue_user_date  ON daily_priority_queue(user_id, queue_date);
CREATE INDEX IF NOT EXISTS idx_queue_priority   ON daily_priority_queue(user_id, queue_date, priority);

-- ─── Supplier Risk Monitor ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS supplier_risk_entries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    supplier_id     UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    risk_category   TEXT NOT NULL,
    risk_score      FLOAT NOT NULL DEFAULT 0.5,
    risk_factors    JSONB DEFAULT '[]'::jsonb,
    mitigation      TEXT,
    owner_id        TEXT,
    status          TEXT NOT NULL DEFAULT 'active',
    last_reviewed   TIMESTAMPTZ,
    next_review     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_supplier_risk_supplier ON supplier_risk_entries(supplier_id);
CREATE INDEX IF NOT EXISTS idx_supplier_risk_score    ON supplier_risk_entries(risk_score DESC);

-- ─── Pipeline Opportunity Tracker ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pipeline_opportunities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    opportunity_id  UUID REFERENCES strategic_opportunities(id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    stage           TEXT NOT NULL DEFAULT 'discovery',
    value_estimate  BIGINT,
    probability     FLOAT NOT NULL DEFAULT 0.5,
    owner_id        TEXT,
    expected_close  DATE,
    actual_close    DATE,
    notes           TEXT,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_pipeline_stage   ON pipeline_opportunities(stage);
CREATE INDEX IF NOT EXISTS idx_pipeline_owner   ON pipeline_opportunities(owner_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_close    ON pipeline_opportunities(expected_close);

-- ─── Alert Preferences ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS alert_preferences (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         TEXT NOT NULL,
    alert_type      TEXT NOT NULL,
    channel         TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    threshold       JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, alert_type, channel)
);

CREATE INDEX IF NOT EXISTS idx_alerts_user ON alert_preferences(user_id);

-- ─── Source Evidence Links ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS source_evidence (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type     TEXT NOT NULL,
    entity_id       UUID NOT NULL,
    evidence_type   TEXT NOT NULL,
    source_url      TEXT NOT NULL,
    source_domain   TEXT,
    source_name     TEXT,
    reliability_score FLOAT DEFAULT 0.5,
    content_hash    TEXT,
    excerpt         TEXT,
    metadata        JSONB DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_evidence_entity ON source_evidence(entity_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_evidence_source ON source_evidence(source_domain);

-- ─── Team Assignments ──────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS team_assignments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    team_id         TEXT NOT NULL,
    team_name       TEXT NOT NULL,
    entity_type     TEXT NOT NULL,
    entity_id       UUID NOT NULL,
    assigned_by     TEXT NOT NULL,
    assigned_to     TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'contributor',
    notes           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(entity_type, entity_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_team_assignments_entity ON team_assignments(entity_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_team_assignments_team   ON team_assignments(team_id);
CREATE INDEX IF NOT EXISTS idx_team_assignments_user   ON team_assignments(assigned_to);

COMMIT;
