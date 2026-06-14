-- ApexIntel Phase 4.3: User Experience Enhancement - Collaboration Features
-- Migration: 0042_create_collaboration_tables
-- Description: Creates tables for investigation workspaces, activity feeds, 
--              priority queues, supplier risk monitoring, and pipeline tracking

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- Investigation Workspaces
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS investigation_workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    description TEXT,
    workspace_type VARCHAR(50) NOT NULL DEFAULT 'ad-hoc',
    owner_id VARCHAR(100) NOT NULL,
    team_id VARCHAR(100),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    visibility VARCHAR(50) NOT NULL DEFAULT 'team',
    tags TEXT[] DEFAULT '{}',
    entity_focus JSONB DEFAULT '[]',
    findings TEXT,
    conclusions TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    
    CONSTRAINT chk_workspace_type CHECK (workspace_type IN ('ad-hoc', 'structured', 'incident', 'ongoing')),
    CONSTRAINT chk_workspace_status CHECK (status IN ('active', 'closed', 'archived')),
    CONSTRAINT chk_workspace_visibility CHECK (visibility IN ('private', 'team', 'organization', 'public'))
);

CREATE INDEX idx_workspaces_owner ON investigation_workspaces(owner_id);
CREATE INDEX idx_workspaces_team ON investigation_workspaces(team_id) WHERE team_id IS NOT NULL;
CREATE INDEX idx_workspaces_status ON investigation_workspaces(status);
CREATE INDEX idx_workspaces_created ON investigation_workspaces(created_at DESC);
CREATE INDEX idx_workspaces_type ON investigation_workspaces(workspace_type);

-- ─────────────────────────────────────────────────────────────────────────────
-- Workspace Assignments
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS workspace_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES investigation_workspaces(id) ON DELETE CASCADE,
    user_id VARCHAR(100) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'contributor',
    assigned_by VARCHAR(100) NOT NULL,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_assignment_role CHECK (role IN ('owner', 'lead', 'contributor', 'viewer', 'reviewer')),
    UNIQUE (workspace_id, user_id)
);

CREATE INDEX idx_workspace_assignments_workspace ON workspace_assignments(workspace_id);
CREATE INDEX idx_workspace_assignments_user ON workspace_assignments(user_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Investigation Shares
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS investigation_shares (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES investigation_workspaces(id) ON DELETE CASCADE,
    shared_by VARCHAR(100) NOT NULL,
    shared_with VARCHAR(255) NOT NULL,
    share_type VARCHAR(50) NOT NULL DEFAULT 'view',
    access_level VARCHAR(50) NOT NULL DEFAULT 'read',
    message TEXT,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_share_type CHECK (share_type IN ('view', 'collaborate', 'embed')),
    CONSTRAINT chk_access_level CHECK (access_level IN ('read', 'read_write', 'admin'))
);

CREATE INDEX idx_investigation_shares_workspace ON investigation_shares(workspace_id);
CREATE INDEX idx_investigation_shares_shared_by ON investigation_shares(shared_by);
CREATE INDEX idx_investigation_shares_shared_with ON investigation_shares(shared_with);
CREATE INDEX idx_investigation_shares_expires ON investigation_shares(expires_at) WHERE expires_at IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Activity Feed
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS activity_feed (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id VARCHAR(100) NOT NULL,
    actor_name VARCHAR(255) NOT NULL,
    action_type VARCHAR(50) NOT NULL,
    entity_type VARCHAR(50),
    entity_id VARCHAR(100),
    entity_name VARCHAR(255),
    details JSONB DEFAULT '{}',
    workspace_id UUID REFERENCES investigation_workspaces(id) ON DELETE SET NULL,
    team_id VARCHAR(100),
    visibility VARCHAR(50) NOT NULL DEFAULT 'team',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_activity_action_type CHECK (action_type IN (
        'create', 'update', 'delete', 'share', 'assign', 'comment', 'resolve', 
        'reopen', 'escalate', 'deescalate', 'approve', 'reject', 'merge', 'split'
    )),
    CONSTRAINT chk_activity_visibility CHECK (visibility IN ('private', 'team', 'organization', 'public'))
);

CREATE INDEX idx_activity_feed_actor ON activity_feed(actor_id);
CREATE INDEX idx_activity_feed_entity ON activity_feed(entity_type, entity_id);
CREATE INDEX idx_activity_feed_workspace ON activity_feed(workspace_id) WHERE workspace_id IS NOT NULL;
CREATE INDEX idx_activity_feed_team ON activity_feed(team_id) WHERE team_id IS NOT NULL;
CREATE INDEX idx_activity_feed_created ON activity_feed(created_at DESC);
CREATE INDEX idx_activity_feed_action ON activity_feed(action_type);

-- ─────────────────────────────────────────────────────────────────────────────
-- Priority Queue
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS priority_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id VARCHAR(100) NOT NULL,
    queue_date DATE NOT NULL DEFAULT CURRENT_DATE,
    item_type VARCHAR(50) NOT NULL,
    item_id VARCHAR(100) NOT NULL,
    item_title VARCHAR(500) NOT NULL,
    priority INTEGER NOT NULL DEFAULT 50,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    notes TEXT,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_queue_priority CHECK (priority >= 1 AND priority <= 100),
    CONSTRAINT chk_queue_status CHECK (status IN ('pending', 'in_progress', 'completed', 'cancelled')),
    CONSTRAINT chk_queue_item_type CHECK (item_type IN ('warning', 'insight', 'investigation', 'task', 'review'))
);

CREATE INDEX idx_priority_queue_user ON priority_queue(user_id);
CREATE INDEX idx_priority_queue_date ON priority_queue(queue_date DESC);
CREATE INDEX idx_priority_queue_status ON priority_queue(status);
CREATE INDEX idx_priority_queue_priority ON priority_queue(priority DESC);
CREATE INDEX idx_priority_queue_item ON priority_queue(item_type, item_id);
CREATE UNIQUE INDEX idx_priority_queue_user_date_type_item ON priority_queue(user_id, queue_date, item_type, item_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Supplier Risk
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS supplier_risk (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    supplier_id VARCHAR(100) NOT NULL,
    supplier_name VARCHAR(255),
    risk_category VARCHAR(50) NOT NULL,
    risk_score DECIMAL(5,4) NOT NULL,
    risk_factors JSONB DEFAULT '[]',
    mitigation TEXT,
    owner_id VARCHAR(100),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    last_reviewed TIMESTAMPTZ,
    next_review TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    
    CONSTRAINT chk_risk_category CHECK (risk_category IN (
        'financial', 'operational', 'compliance', 'geopolitical', 'environmental', 
        'technological', 'reputational', 'strategic'
    )),
    CONSTRAINT chk_risk_score CHECK (risk_score >= 0 AND risk_score <= 1),
    CONSTRAINT chk_supplier_risk_status CHECK (status IN ('active', 'monitoring', 'resolved', 'escalated'))
);

CREATE INDEX idx_supplier_risk_supplier ON supplier_risk(supplier_id);
CREATE INDEX idx_supplier_risk_category ON supplier_risk(risk_category);
CREATE INDEX idx_supplier_risk_score ON supplier_risk(risk_score DESC);
CREATE INDEX idx_supplier_risk_status ON supplier_risk(status);
CREATE INDEX idx_supplier_risk_owner ON supplier_risk(owner_id) WHERE owner_id IS NOT NULL;
CREATE INDEX idx_supplier_risk_next_review ON supplier_risk(next_review) WHERE next_review IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Pipeline Opportunities
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pipeline_opportunities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    opportunity_id VARCHAR(100),
    title VARCHAR(500) NOT NULL,
    stage VARCHAR(50) NOT NULL DEFAULT 'discovery',
    value_estimate DECIMAL(15,2),
    probability DECIMAL(5,4) NOT NULL DEFAULT 0,
    owner_id VARCHAR(100),
    expected_close DATE,
    actual_close DATE,
    notes TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    
    CONSTRAINT chk_pipeline_stage CHECK (stage IN (
        'discovery', 'qualification', 'proposal', 'negotiation', 'closed_won', 'closed_lost'
    )),
    CONSTRAINT chk_probability CHECK (probability >= 0 AND probability <= 1)
);

CREATE INDEX idx_pipeline_opportunities_stage ON pipeline_opportunities(stage);
CREATE INDEX idx_pipeline_opportunities_owner ON pipeline_opportunities(owner_id) WHERE owner_id IS NOT NULL;
CREATE INDEX idx_pipeline_opportunities_expected_close ON pipeline_opportunities(expected_close);
CREATE INDEX idx_pipeline_opportunities_value ON pipeline_opportunities(value_estimate DESC) WHERE value_estimate IS NOT NULL;
CREATE INDEX idx_pipeline_opportunities_created ON pipeline_opportunities(created_at DESC);

-- ─────────────────────────────────────────────────────────────────────────────
-- Source Evidence
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS source_evidence (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type VARCHAR(50) NOT NULL,
    entity_id VARCHAR(100) NOT NULL,
    evidence_type VARCHAR(50) NOT NULL,
    source_url TEXT NOT NULL,
    source_domain VARCHAR(255),
    source_name VARCHAR(255),
    reliability_score DECIMAL(5,4) NOT NULL DEFAULT 0.5,
    content_hash VARCHAR(64),
    excerpt TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_evidence_type CHECK (evidence_type IN (
        'web_content', 'document', 'financial_report', 'news_article', 'social_media',
        'regulatory_filing', 'patent', 'court_record', 'public_record', 'analyst_report'
    )),
    CONSTRAINT chk_reliability_score CHECK (reliability_score >= 0 AND reliability_score <= 1),
    CONSTRAINT chk_entity_type CHECK (entity_type IN ('company', 'person', 'warning', 'insight', 'workspace'))
);

CREATE INDEX idx_source_evidence_entity ON source_evidence(entity_type, entity_id);
CREATE INDEX idx_source_evidence_type ON source_evidence(evidence_type);
CREATE INDEX idx_source_evidence_reliability ON source_evidence(reliability_score DESC);
CREATE INDEX idx_source_evidence_domain ON source_evidence(source_domain) WHERE source_domain IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Team Assignments
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS team_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    team_id VARCHAR(100) NOT NULL,
    team_name VARCHAR(255),
    entity_type VARCHAR(50) NOT NULL,
    entity_id VARCHAR(100) NOT NULL,
    assigned_by VARCHAR(100) NOT NULL,
    assigned_to VARCHAR(100) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'contributor',
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_team_entity_type CHECK (entity_type IN ('warning', 'insight', 'investigation', 'opportunity', 'threat')),
    CONSTRAINT chk_team_role CHECK (role IN ('lead', 'contributor', 'reviewer', 'observer'))
);

CREATE INDEX idx_team_assignments_team ON team_assignments(team_id);
CREATE INDEX idx_team_assignments_entity ON team_assignments(entity_type, entity_id);
CREATE INDEX idx_team_assignments_assigned_to ON team_assignments(assigned_to);

-- ─────────────────────────────────────────────────────────────────────────────
-- Strategic Opportunities
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS strategic_opportunities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title VARCHAR(500) NOT NULL,
    description TEXT,
    opportunity_type VARCHAR(50) NOT NULL,
    priority_score DECIMAL(5,4) NOT NULL,
    confidence DECIMAL(5,4) NOT NULL DEFAULT 0.5,
    entity_id VARCHAR(100),
    entity_type VARCHAR(50),
    region VARCHAR(10),
    estimated_value VARCHAR(100),
    recommended_actions JSONB DEFAULT '[]',
    owner_id VARCHAR(100),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    due_date TIMESTAMPTZ,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_opportunity_type CHECK (opportunity_type IN (
        'market_expansion', 'acquisition', 'partnership', 'product_launch', 
        'cost_reduction', 'talent_acquisition', 'technology_adoption'
    )),
    CONSTRAINT chk_priority_score CHECK (priority_score >= 0 AND priority_score <= 1),
    CONSTRAINT chk_confidence CHECK (confidence >= 0 AND confidence <= 1),
    CONSTRAINT chk_opportunity_status CHECK (status IN ('active', 'pursued', 'completed', 'abandoned'))
);

CREATE INDEX idx_strategic_opportunities_priority ON strategic_opportunities(priority_score DESC);
CREATE INDEX idx_strategic_opportunities_type ON strategic_opportunities(opportunity_type);
CREATE INDEX idx_strategic_opportunities_region ON strategic_opportunities(region) WHERE region IS NOT NULL;
CREATE INDEX idx_strategic_opportunities_status ON strategic_opportunities(status);
CREATE INDEX idx_strategic_opportunities_owner ON strategic_opportunities(owner_id) WHERE owner_id IS NOT NULL;
CREATE INDEX idx_strategic_opportunities_due_date ON strategic_opportunities(due_date) WHERE due_date IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- Critical Threats
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS critical_threats (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title VARCHAR(500) NOT NULL,
    description TEXT,
    threat_type VARCHAR(50) NOT NULL,
    severity VARCHAR(20) NOT NULL,
    impact_score DECIMAL(5,4) NOT NULL,
    confidence DECIMAL(5,4) NOT NULL DEFAULT 0.5,
    entity_id VARCHAR(100),
    entity_type VARCHAR(50),
    region VARCHAR(10),
    mitigation_steps JSONB DEFAULT '[]',
    owner_id VARCHAR(100),
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    sla_deadline TIMESTAMPTZ,
    resolved_at TIMESTAMPTZ,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    CONSTRAINT chk_threat_type CHECK (threat_type IN (
        'financial', 'operational', 'regulatory', 'cybersecurity', 'geopolitical',
        'environmental', 'supply_chain', 'competitive', 'market', 'reputational'
    )),
    CONSTRAINT chk_severity CHECK (severity IN ('low', 'medium', 'high', 'critical')),
    CONSTRAINT chk_impact_score CHECK (impact_score >= 0 AND impact_score <= 1),
    CONSTRAINT chk_confidence_threat CHECK (confidence >= 0 AND confidence <= 1),
    CONSTRAINT chk_threat_status CHECK (status IN ('active', 'monitoring', 'resolved', 'escalated'))
);

CREATE INDEX idx_critical_threats_severity ON critical_threats(severity);
CREATE INDEX idx_critical_threats_type ON critical_threats(threat_type);
CREATE INDEX idx_critical_threats_impact ON critical_threats(impact_score DESC);
CREATE INDEX idx_critical_threats_region ON critical_threats(region) WHERE region IS NOT NULL;
CREATE INDEX idx_critical_threats_status ON critical_threats(status);
CREATE INDEX idx_critical_threats_sla ON critical_threats(sla_deadline) WHERE sla_deadline IS NOT NULL;
CREATE INDEX idx_critical_threats_owner ON critical_threats(owner_id) WHERE owner_id IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────────────
-- View Definitions for Executive Dashboard
-- ─────────────────────────────────────────────────────────────────────────────

-- Executive summary view combining opportunities, threats, and actions
CREATE OR REPLACE VIEW executive_summary_view AS
SELECT 
    'opportunity' AS category,
    id,
    title,
    description,
    priority_score AS score,
    confidence,
    region,
    owner_id,
    status,
    created_at
FROM strategic_opportunities
WHERE status = 'active'
UNION ALL
SELECT 
    'threat' AS category,
    id,
    title,
    description,
    impact_score AS score,
    confidence,
    region,
    owner_id,
    status,
    created_at
FROM critical_threats
WHERE status IN ('active', 'escalated');

-- ─────────────────────────────────────────────────────────────────────────────
-- Helper Functions
-- ─────────────────────────────────────────────────────────────────────────────

-- Function to update timestamps automatically
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply update triggers
CREATE TRIGGER update_workspaces_updated_at
    BEFORE UPDATE ON investigation_workspaces
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_workspace_assignments_updated_at
    BEFORE UPDATE ON workspace_assignments
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_activity_feed_created
    BEFORE INSERT ON activity_feed
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_priority_queue_updated_at
    BEFORE UPDATE ON priority_queue
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_supplier_risk_updated_at
    BEFORE UPDATE ON supplier_risk
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_pipeline_opportunities_updated_at
    BEFORE UPDATE ON pipeline_opportunities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_team_assignments_updated_at
    BEFORE UPDATE ON team_assignments
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_strategic_opportunities_updated_at
    BEFORE UPDATE ON strategic_opportunities
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_critical_threats_updated_at
    BEFORE UPDATE ON critical_threats
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Function to record activity automatically
CREATE OR REPLACE FUNCTION record_activity(
    p_actor_id VARCHAR,
    p_actor_name VARCHAR,
    p_action_type VARCHAR,
    p_entity_type VARCHAR DEFAULT NULL,
    p_entity_id VARCHAR DEFAULT NULL,
    p_entity_name VARCHAR DEFAULT NULL,
    p_details JSONB DEFAULT '{}',
    p_workspace_id UUID DEFAULT NULL,
    p_team_id VARCHAR DEFAULT NULL,
    p_visibility VARCHAR DEFAULT 'team'
) RETURNS UUID AS $$
DECLARE
    v_activity_id UUID;
BEGIN
    INSERT INTO activity_feed (
        actor_id, actor_name, action_type, entity_type, entity_id,
        entity_name, details, workspace_id, team_id, visibility
    ) VALUES (
        p_actor_id, p_actor_name, p_action_type, p_entity_type, p_entity_id,
        p_entity_name, p_details, p_workspace_id, p_team_id, p_visibility
    ) RETURNING id INTO v_activity_id;
    
    RETURN v_activity_id;
END;
$$ LANGUAGE plpgsql;

-- Function to get high-priority items for executive dashboard
CREATE OR REPLACE FUNCTION get_executive_dashboard_data(
    p_include_threats BOOLEAN DEFAULT TRUE,
    p_include_opportunities BOOLEAN DEFAULT TRUE,
    p_region_filter VARCHAR DEFAULT NULL,
    p_priority_threshold DECIMAL DEFAULT 0.7
) RETURNS TABLE (
    category VARCHAR,
    id UUID,
    title VARCHAR,
    description TEXT,
    score DECIMAL,
    confidence DECIMAL,
    region VARCHAR,
    owner_id VARCHAR,
    status VARCHAR,
    created_at TIMESTAMPTZ
) AS $$
BEGIN
    RETURN QUERY
    SELECT 
        'opportunity'::VARCHAR AS category,
        so.id,
        so.title,
        so.description,
        so.priority_score AS score,
        so.confidence,
        so.region,
        so.owner_id,
        so.status,
        so.created_at
    FROM strategic_opportunities so
    WHERE p_include_opportunities
        AND so.status = 'active'
        AND so.priority_score >= p_priority_threshold
        AND (p_region_filter IS NULL OR so.region = p_region_filter)
    
    UNION ALL
    
    SELECT 
        'threat'::VARCHAR AS category,
        ct.id,
        ct.title,
        ct.description,
        ct.impact_score AS score,
        ct.confidence,
        ct.region,
        ct.owner_id,
        ct.status,
        ct.created_at
    FROM critical_threats ct
    WHERE p_include_threats
        AND ct.status IN ('active', 'escalated')
        AND ct.impact_score >= p_priority_threshold
        AND (p_region_filter IS NULL OR ct.region = p_region_filter)
    
    ORDER BY score DESC, created_at DESC
    LIMIT 20;
END;
$$ LANGUAGE plpgsql;

COMMIT;
