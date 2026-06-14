-- ApexIntel Phase 4.3: Comprehensive Tests
-- Migration: 0044_add_comprehensive_tests
-- Description: Add comprehensive test data and validation functions for collaboration features

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Strategic Opportunities
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO strategic_opportunities (id, title, description, opportunity_type, priority_score, confidence, region, estimated_value, status)
VALUES
    ('11111111-1111-1111-1111-111111111101', 'APAC Market Expansion', 'Enter high-growth APAC markets', 'market_expansion', 0.92, 0.85, 'APAC', '$15M', 'active'),
    ('11111111-1111-1111-1111-111111111102', 'Strategic Acquisition Target', 'Acquire complementary tech firm', 'acquisition', 0.88, 0.78, 'NA', '$50M', 'active'),
    ('11111111-1111-1111-1111-111111111103', 'Partnership with Industry Leader', 'Joint venture opportunity', 'partnership', 0.75, 0.82, 'EMEA', '$25M', 'active'),
    ('11111111-1111-1111-1111-111111111104', 'New Product Launch - Q2', 'Launch innovative product line', 'product_launch', 0.70, 0.65, 'NA', '$10M', 'active'),
    ('11111111-1111-1111-1111-111111111105', 'Cost Optimization Initiative', 'Reduce operational costs by 20%', 'cost_reduction', 0.68, 0.90, 'GLOBAL', '$8M', 'pursued'),
    ('11111111-1111-1111-1111-111111111106', 'Talent Acquisition - Engineering', 'Hire top engineering talent', 'talent_acquisition', 0.65, 0.72, 'EMEA', '$2M', 'active'),
    ('11111111-1111-1111-1111-111111111107', 'Digital Transformation Project', 'Modernize legacy systems', 'technology_adoption', 0.60, 0.55, 'NA', '$12M', 'active'),
    ('11111111-1111-1111-1111-111111111108', 'Supply Chain Optimization', 'Streamline global supply chain', 'market_expansion', 0.58, 0.80, 'GLOBAL', '$5M', 'completed'),
    ('11111111-1111-1111-1111-111111111109', 'Customer Experience Enhancement', 'Improve customer satisfaction', 'market_expansion', 0.55, 0.70, 'APAC', '$3M', 'active'),
    ('11111111-1111-1111-1111-111111111110', 'Sustainability Initiative', 'Implement green manufacturing', 'cost_reduction', 0.52, 0.60, 'EMEA', '$6M', 'abandoned');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Critical Threats
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO critical_threats (id, title, description, threat_type, severity, impact_score, confidence, region, sla_deadline, status)
VALUES
    ('22222222-2222-2222-2222-222222222201', 'Supply Chain Disruption Risk', 'Critical supplier may fail in Q2', 'supply_chain', 'critical', 0.95, 0.92, 'APAC', NOW() + INTERVAL '7 days', 'active'),
    ('22222222-2222-2222-2222-222222222202', 'Regulatory Compliance Gap', 'Missing GDPR compliance requirements', 'regulatory', 'high', 0.88, 0.85, 'EMEA', NOW() + INTERVAL '14 days', 'active'),
    ('22222222-2222-2222-2222-222222222203', 'Cybersecurity Vulnerability', 'Critical zero-day exploit detected', 'cybersecurity', 'critical', 0.92, 0.95, 'GLOBAL', NOW() + INTERVAL '2 days', 'escalated'),
    ('22222222-2222-2222-2222-222222222204', 'Competitor Market Entry', 'Major competitor launching similar product', 'competitive', 'high', 0.78, 0.70, 'NA', NOW() + INTERVAL '30 days', 'active'),
    ('22222222-2222-2222-2222-222222222205', 'Currency Fluctuation Impact', 'EUR/USD volatility affecting margins', 'financial', 'medium', 0.65, 0.75, 'EMEA', NULL, 'monitoring'),
    ('22222222-2222-2222-2222-222222222206', 'Key Personnel Departure Risk', 'VP Engineering considering leaving', 'operational', 'high', 0.72, 0.60, 'NA', NULL, 'active'),
    ('22222222-2222-2222-2222-222222222207', 'Geopolitical Risk - Trade War', 'Potential tariff increases', 'geopolitical', 'medium', 0.58, 0.65, 'APAC', NULL, 'monitoring'),
    ('22222222-2222-2222-2222-222222222208', 'Environmental Compliance Issue', 'EPA audit findings', 'environmental', 'high', 0.75, 0.80, 'NA', NOW() + INTERVAL '21 days', 'active'),
    ('22222222-2222-2222-2222-222222222209', 'Reputational Risk - Social Media', 'Negative viral campaign', 'reputational', 'medium', 0.68, 0.72, 'GLOBAL', NULL, 'monitoring'),
    ('22222222-2222-2222-2222-222222222210', 'Technology Obsolescence', 'Legacy system end-of-life', 'technological', 'low', 0.45, 0.85, 'NA', NULL, 'resolved');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Investigation Workspaces
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO investigation_workspaces (id, name, description, workspace_type, owner_id, team_id, status, visibility, tags)
VALUES
    ('33333333-3333-3333-3333-333333333301', 'Supply Chain Risk Assessment', 'Comprehensive analysis of supply chain vulnerabilities', 'structured', 'analyst-1', 'team-ops', 'active', 'team', ARRAY['supply-chain', 'risk', 'critical']),
    ('33333333-3333-3333-3333-333333333302', 'Competitor Analysis Q1', 'Monitor competitor activities and market position', 'ongoing', 'analyst-2', 'team-strategy', 'active', 'organization', ARRAY['competitor', 'market', 'quarterly']),
    ('33333333-3333-3333-3333-333333333303', 'Security Incident Response', 'Investigate reported security vulnerability', 'incident', 'analyst-1', 'team-security', 'active', 'team', ARRAY['security', 'incident', 'urgent']),
    ('33333333-3333-3333-3333-333333333304', 'Market Entry Strategy - APAC', 'Develop market entry plan for APAC region', 'ad-hoc', 'analyst-3', 'team-expansion', 'active', 'team', ARRAY['market-entry', 'strategy', 'apac']),
    ('33333333-3333-3333-3333-333333333305', 'Due Diligence - Acquisition Target', 'Technical due diligence for acquisition candidate', 'structured', 'analyst-2', 'team-mna', 'active', 'private', ARRAY['m&a', 'due-diligence', 'confidential']);

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Priority Queue
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO priority_queue (id, user_id, item_type, item_id, item_title, priority, status, notes)
VALUES
    ('44444444-4444-4444-4444-444444444401', 'analyst-1', 'warning', 'warning-001', 'Critical security alert - patch required', 95, 'pending', 'Apply emergency patch within 24 hours'),
    ('44444444-4444-4444-4444-444444444402', 'analyst-1', 'insight', 'insight-042', 'Follow up on competitor announcement', 75, 'in_progress', 'Analyze competitive implications'),
    ('44444444-4444-4444-4444-444444444403', 'analyst-2', 'investigation', 'ws-333', 'Complete market analysis report', 60, 'pending', 'Due Friday by EOD'),
    ('44444444-4444-4444-4444-444444444404', 'analyst-1', 'task', 'task-008', 'Update threat intelligence database', 50, 'completed', 'Completed batch update'),
    ('44444444-4444-4444-4444-444444444405', 'analyst-3', 'review', 'review-015', 'Review quarterly risk assessment', 40, 'pending', 'Board meeting preparation');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Supplier Risks
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO supplier_risk (id, supplier_id, supplier_name, risk_category, risk_score, risk_factors, mitigation, status, next_review)
VALUES
    ('55555555-5555-5555-5555-555555555501', 'sup-001', 'TechParts Global', 'operational', 0.85, '["Single source dependency", "Geographic concentration", "Capacity constraints"]'::jsonb, 'Identify backup suppliers; diversify manufacturing locations', 'active', NOW() + INTERVAL '7 days'),
    ('55555555-5555-5555-5555-555555555502', 'sup-002', 'Rapid Logistics', 'financial', 0.72, '["High debt ratio", "Recent leadership changes"]'::jsonb, 'Review financial health quarterly; establish credit terms', 'monitoring', NOW() + INTERVAL '14 days'),
    ('55555555-5555-5555-5555-555555555503', 'sup-003', 'SecureData Solutions', 'compliance', 0.68, '["SOC2 audit overdue", "Missing certifications"]'::jsonb, 'Require compliance certification before contract renewal', 'active', NOW() + INTERVAL '30 days'),
    ('55555555-5555-5555-5555-555555555504', 'sup-004', 'Global Components Ltd', 'geopolitical', 0.55, '["Manufacturing in trade-restricted region", "Currency exposure"]'::jsonb, 'Evaluate alternative regions; hedge currency risk', 'monitoring', NOW() + INTERVAL '60 days'),
    ('55555555-5555-5555-5555-555555555505', 'sup-005', 'EcoMaterials Inc', 'environmental', 0.35, '["Recent EPA notice", "Equipment age"]'::jsonb, 'Conduct environmental audit; plan equipment upgrade', 'active', NOW() + INTERVAL '90 days');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Pipeline Opportunities
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO pipeline_opportunities (id, opportunity_id, title, stage, value_estimate, probability, expected_close)
VALUES
    ('66666666-6666-6666-6666-666666666601', 'opp-101', 'Enterprise License - Global Corp', 'negotiation', 2500000.00, 0.75, '2024-03-15'),
    ('66666666-6666-6666-6666-666666666602', 'opp-102', 'Platform Subscription - TechStart', 'proposal', 500000.00, 0.60, '2024-04-30'),
    ('66666666-6666-6666-6666-666666666603', 'opp-103', 'Custom Integration - MegaCorp', 'discovery', 1500000.00, 0.30, '2024-06-30'),
    ('66666666-6666-6666-6666-666666666604', 'opp-104', 'Annual Renewal - IndustryCo', 'qualification', 800000.00, 0.55, '2024-04-15'),
    ('66666666-6666-6666-6666-666666666605', 'opp-105', 'Expansion Deal - BigBiz Inc', 'closed_won', 1200000.00, 1.00, '2024-02-01'),
    ('66666666-6666-6666-6666-666666666606', 'opp-106', 'Pilot Program - StartupX', 'closed_lost', 100000.00, 0.00, '2024-01-15');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Activity Feed
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO activity_feed (actor_id, actor_name, action_type, entity_type, entity_id, entity_name, details, workspace_id, visibility)
VALUES
    ('analyst-1', 'John Smith', 'create', 'workspace', 'ws-333301', 'Supply Chain Risk Assessment', '{"type": "investigation"}'::jsonb, '33333333-3333-3333-3333-333333333301', 'team'),
    ('analyst-2', 'Jane Doe', 'update', 'opportunity', 'opp-101', 'Enterprise License - Global Corp', '{"field": "stage", "old": "proposal", "new": "negotiation"}'::jsonb, NULL, 'organization'),
    ('analyst-1', 'John Smith', 'share', 'workspace', 'ws-333303', 'Security Incident Response', '{"shared_with": "team-security", "access": "read_write"}'::jsonb, '33333333-3333-3333-3333-333333333303', 'team'),
    ('analyst-3', 'Bob Wilson', 'comment', 'threat', 'threat-203', 'Cybersecurity Vulnerability', '{"comment": "Patch deployed successfully"}'::jsonb, NULL, 'organization'),
    ('analyst-2', 'Jane Doe', 'assign', 'workspace', 'ws-333305', 'Due Diligence - Acquisition Target', '{"assigned_to": "analyst-4", "role": "contributor"}'::jsonb, '33333333-3333-3333-3333-333333333305', 'private'),
    ('analyst-1', 'John Smith', 'resolve', 'warning', 'warning-001', 'Critical security alert', '{"resolution": "Emergency patch applied"}'::jsonb, NULL, 'team'),
    ('analyst-3', 'Bob Wilson', 'escalate', 'threat', 'threat-201', 'Supply Chain Disruption Risk', '{"escalated_to": "executive-team", "reason": "Critical timeline"}'::jsonb, NULL, 'organization');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Team Assignments
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO team_assignments (id, team_id, team_name, entity_type, entity_id, assigned_by, assigned_to, role)
VALUES
    ('77777777-7777-7777-7777-777777777701', 'team-security', 'Security Response Team', 'warning', '22222222-2222-2222-2222-222222222203', 'admin-1', 'analyst-1', 'lead'),
    ('77777777-7777-7777-7777-777777777702', 'team-ops', 'Operations Team', 'workspace', '33333333-3333-3333-3333-333333333301', 'admin-1', 'analyst-2', 'contributor'),
    ('77777777-7777-7777-7777-777777777703', 'team-strategy', 'Strategy Team', 'opportunity', '11111111-1111-1111-1111-111111111101', 'admin-2', 'analyst-3', 'lead'),
    ('77777777-7777-7777-7777-777777777704', 'team-mna', 'M&A Team', 'workspace', '33333333-3333-3333-3333-333333333305', 'admin-2', 'analyst-2', 'lead'),
    ('77777777-7777-7777-7777-777777777705', 'team-compliance', 'Compliance Team', 'threat', '22222222-2222-2222-2222-222222222202', 'admin-1', 'analyst-4', 'reviewer');

-- ─────────────────────────────────────────────────────────────────────────────
-- Test Data for Source Evidence
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO source_evidence (id, entity_type, entity_id, evidence_type, source_url, source_name, reliability_score, excerpt)
VALUES
    ('88888888-8888-8888-8888-888888888801', 'company', 'comp-001', 'news_article', 'https://news.example.com/tech-merger', 'Tech News Daily', 0.85, 'Leading tech company announces strategic merger with innovative startup'),
    ('88888888-8888-8888-8888-888888888802', 'company', 'comp-002', 'financial_report', 'https://sec.gov/filings/annual-report-2023', 'SEC Annual Report', 0.95, 'Revenue increased 25% year-over-year, driven by enterprise segment growth'),
    ('88888888-8888-8888-8888-888888888803', 'person', 'person-042', 'social_media', 'https://linkedin.com/in/exec-profile', 'LinkedIn Profile', 0.65, 'Executive with 15+ years experience in technology sector'),
    ('88888888-8888-8888-8888-888888888804', 'company', 'comp-003', 'regulatory_filing', 'https://eca.gov/environmental-compliance', 'EPA Filing', 0.92, 'Environmental compliance certificate renewed for manufacturing facility'),
    ('88888888-8888-8888-8888-888888888805', 'warning', 'warning-001', 'analyst_report', 'https://threat-intel.example.com/advisory', 'Security Advisory', 0.88, 'Critical vulnerability identified in widely-used software component');

-- ─────────────────────────────────────────────────────────────────────────────
-- Validation Functions
-- ─────────────────────────────────────────────────────────────────────────────

-- Function to validate priority queue data
CREATE OR REPLACE FUNCTION validate_priority_queue_data()
RETURNS TABLE (validation_result TEXT) AS $$
DECLARE
    v_count INTEGER;
BEGIN
    -- Check for items with priority > 100
    SELECT COUNT(*) INTO v_count FROM priority_queue WHERE priority > 100 OR priority < 1;
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' items with invalid priority (must be 1-100)';
    ELSE
        RETURN NEXT 'PASS: All priority values are valid (1-100)';
    END IF;

    -- Check for items with status not in allowed list
    SELECT COUNT(*) INTO v_count FROM priority_queue 
    WHERE status NOT IN ('pending', 'in_progress', 'completed', 'cancelled');
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' items with invalid status';
    ELSE
        RETURN NEXT 'PASS: All status values are valid';
    END IF;

    -- Check for duplicate items in queue
    SELECT COUNT(*) INTO v_count FROM (
        SELECT user_id, item_type, item_id, COUNT(*) as cnt
        FROM priority_queue
        WHERE status IN ('pending', 'in_progress')
        GROUP BY user_id, item_type, item_id
        HAVING COUNT(*) > 1
    ) duplicates;
    IF v_count > 0 THEN
        RETURN NEXT 'WARN: Found ' || v_count || ' duplicate queue items';
    ELSE
        RETURN NEXT 'PASS: No duplicate queue items found';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- Function to validate strategic opportunities data
CREATE OR REPLACE FUNCTION validate_opportunities_data()
RETURNS TABLE (validation_result TEXT) AS $$
DECLARE
    v_count INTEGER;
BEGIN
    -- Check for opportunities with score out of range
    SELECT COUNT(*) INTO v_count FROM strategic_opportunities 
    WHERE priority_score < 0 OR priority_score > 1;
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' opportunities with invalid priority_score';
    ELSE
        RETURN NEXT 'PASS: All priority_score values are valid (0-1)';
    END IF;

    -- Check for opportunities with confidence out of range
    SELECT COUNT(*) INTO v_count FROM strategic_opportunities 
    WHERE confidence < 0 OR confidence > 1;
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' opportunities with invalid confidence';
    ELSE
        RETURN NEXT 'PASS: All confidence values are valid (0-1)';
    END IF;

    -- Check for expired opportunities (past due_date)
    SELECT COUNT(*) INTO v_count FROM strategic_opportunities 
    WHERE status = 'active' AND due_date < NOW();
    IF v_count > 0 THEN
        RETURN NEXT 'WARN: Found ' || v_count || ' active opportunities past their due date';
    ELSE
        RETURN NEXT 'PASS: No overdue active opportunities';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- Function to validate critical threats data
CREATE OR REPLACE FUNCTION validate_threats_data()
RETURNS TABLE (validation_result TEXT) AS $$
DECLARE
    v_count INTEGER;
BEGIN
    -- Check for threats with impact score out of range
    SELECT COUNT(*) INTO v_count FROM critical_threats 
    WHERE impact_score < 0 OR impact_score > 1;
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' threats with invalid impact_score';
    ELSE
        RETURN NEXT 'PASS: All impact_score values are valid (0-1)';
    END IF;

    -- Check for threats with invalid severity
    SELECT COUNT(*) INTO v_count FROM critical_threats 
    WHERE severity NOT IN ('low', 'medium', 'high', 'critical');
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' threats with invalid severity';
    ELSE
        RETURN NEXT 'PASS: All severity values are valid';
    END IF;

    -- Check for critical/high threats past SLA
    SELECT COUNT(*) INTO v_count FROM critical_threats 
    WHERE status IN ('active', 'escalated') 
        AND severity IN ('critical', 'high')
        AND sla_deadline < NOW();
    IF v_count > 0 THEN
        RETURN NEXT 'FAIL: Found ' || v_count || ' critical/high threats past SLA deadline';
    ELSE
        RETURN NEXT 'PASS: No critical/high threats past SLA deadline';
    END IF;

    -- Check for threats with expired SLA but still active
    SELECT COUNT(*) INTO v_count FROM critical_threats 
    WHERE status = 'active' AND sla_deadline < NOW() - INTERVAL '7 days';
    IF v_count > 0 THEN
        RETURN NEXT 'WARN: Found ' || v_count || ' active threats with expired SLA (>7 days)';
    ELSE
        RETURN NEXT 'PASS: No active threats with expired SLA';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- Function to generate executive summary statistics
CREATE OR REPLACE FUNCTION get_executive_summary_stats()
RETURNS TABLE (
    metric_name TEXT,
    metric_value TEXT
) AS $$
BEGIN
    RETURN QUERY SELECT 'total_active_opportunities'::TEXT, COUNT(*)::TEXT 
    FROM strategic_opportunities WHERE status = 'active';
    
    RETURN QUERY SELECT 'avg_priority_score'::TEXT, 
        ROUND(AVG(priority_score)::numeric, 2)::TEXT 
    FROM strategic_opportunities WHERE status = 'active';
    
    RETURN QUERY SELECT 'total_active_threats'::TEXT, COUNT(*)::TEXT 
    FROM critical_threats WHERE status IN ('active', 'escalated');
    
    RETURN QUERY SELECT 'critical_threat_count'::TEXT, COUNT(*)::TEXT 
    FROM critical_threats WHERE severity = 'critical' AND status IN ('active', 'escalated');
    
    RETURN QUERY SELECT 'threats_past_sla'::TEXT, COUNT(*)::TEXT 
    FROM critical_threats WHERE status IN ('active', 'escalated') AND sla_deadline < NOW();
    
    RETURN QUERY SELECT 'active_workspaces'::TEXT, COUNT(*)::TEXT 
    FROM investigation_workspaces WHERE status = 'active';
    
    RETURN QUERY SELECT 'pending_queue_items'::TEXT, COUNT(*)::TEXT 
    FROM priority_queue WHERE status IN ('pending', 'in_progress');
    
    RETURN QUERY SELECT 'high_risk_suppliers'::TEXT, COUNT(*)::TEXT 
    FROM supplier_risk WHERE risk_score > 0.7 AND status = 'active';
    
    RETURN QUERY SELECT 'pipeline_value'::TEXT, 
        COALESCE(SUM(value_estimate)::text, '0') 
    FROM pipeline_opportunities WHERE stage NOT IN ('closed_won', 'closed_lost');
    
    RETURN QUERY SELECT 'won_this_month'::TEXT, COUNT(*)::TEXT 
    FROM pipeline_opportunities 
    WHERE stage = 'closed_won' 
        AND actual_close >= DATE_TRUNC('month', NOW());
END;
$$ LANGUAGE plpgsql;

COMMIT;
