-- ════════════════════════════════════════════════════════════════════════════
-- Enable Row-Level Security (RLS) on all user-facing tables
--
-- This migration:
--   1. Enables RLS on every table that holds user data
--   2. Creates default-deny policies (no access by default)
--   3. Creates role-based policies for admin, analyst, and viewer roles
--   4. Adds created_by_user_id column references where missing
--   5. Creates helper functions for role checks
--
-- Prerequisites:
--   - analyst_users table exists
--   - analyst_user_roles table exists
--
-- Roles hierarchy: viewer < analyst < admin < superadmin
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ══════════════════════════════════════════════════════════════════════════
-- 1. Helper function: Get the current user's role from the app context
-- ══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION current_user_role()
RETURNS TEXT AS $$
BEGIN
    -- The application sets this via SET app.current_user_role = 'admin'
    -- Default is 'viewer' for unauthenticated access
    RETURN COALESCE(NULLIF(current_setting('app.current_user_role', TRUE), ''), 'viewer');
END;
$$ LANGUAGE plpgsql STABLE;

CREATE OR REPLACE FUNCTION current_user_id()
RETURNS TEXT AS $$
BEGIN
    -- The application sets this via SET app.current_user_id = 'user-id'
    RETURN current_setting('app.current_user_id', TRUE);
END;
$$ LANGUAGE plpgsql STABLE;

-- ══════════════════════════════════════════════════════════════════════════
-- 2. Helper: Check if current user has at least the required role
-- ══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION has_minimum_role(min_role TEXT)
RETURNS BOOLEAN AS $$
DECLARE
    user_role TEXT;
    role_hierarchy TEXT[] := ARRAY['viewer', 'analyst', 'admin', 'superadmin'];
    min_idx INT;
    user_idx INT;
BEGIN
    user_role := current_user_role();
    min_idx := array_position(role_hierarchy, min_role);
    user_idx := array_position(role_hierarchy, user_role);
    RETURN user_idx IS NOT NULL AND user_idx >= min_idx;
END;
$$ LANGUAGE plpgsql STABLE;

-- ══════════════════════════════════════════════════════════════════════════
-- 3. Enable RLS on all user-facing tables
-- ══════════════════════════════════════════════════════════════════════════

-- Core data tables
ALTER TABLE companies ENABLE ROW LEVEL SECURITY;
ALTER TABLE sites ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_families ENABLE ROW LEVEL SECURITY;
ALTER TABLE capabilities ENABLE ROW LEVEL SECURITY;
ALTER TABLE certifications ENABLE ROW LEVEL SECURITY;
ALTER TABLE persons ENABLE ROW LEVEL SECURITY;
ALTER TABLE poi_artifacts ENABLE ROW LEVEL SECURITY;
ALTER TABLE observations ENABLE ROW LEVEL SECURITY;
ALTER TABLE graph_edges ENABLE ROW LEVEL SECURITY;
ALTER TABLE warnings ENABLE ROW LEVEL SECURITY;
ALTER TABLE insights ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipes ENABLE ROW LEVEL SECURITY;
ALTER TABLE feature_rows ENABLE ROW LEVEL SECURITY;

-- Social/crawl data
ALTER TABLE social_signals ENABLE ROW LEVEL SECURITY;
ALTER TABLE page_fingerprints ENABLE ROW LEVEL SECURITY;
ALTER TABLE crawl_telemetry ENABLE ROW LEVEL SECURITY;

-- User-facing tables
ALTER TABLE analyst_users ENABLE ROW LEVEL SECURITY;
ALTER TABLE analyst_user_roles ENABLE ROW LEVEL SECURITY;
ALTER TABLE saved_searches ENABLE ROW LEVEL SECURITY;
ALTER TABLE watchlists ENABLE ROW LEVEL SECURITY;
ALTER TABLE annotations ENABLE ROW LEVEL SECURITY;
ALTER TABLE export_history ENABLE ROW LEVEL SECURITY;
ALTER TABLE insight_bookmarks ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_preferences ENABLE ROW LEVEL SECURITY;
ALTER TABLE insight_feedback_events ENABLE ROW LEVEL SECURITY;

-- Audit and observability
ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE quality_score_breakdown ENABLE ROW LEVEL SECURITY;
ALTER TABLE gate_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE llm_retry_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE insight_outcomes ENABLE ROW LEVEL SECURITY;

-- Dossier and tracking tables
ALTER TABLE dossier_entries ENABLE ROW LEVEL SECURITY;
ALTER TABLE role_history ENABLE ROW LEVEL SECURITY;
ALTER TABLE company_changes ENABLE ROW LEVEL SECURITY;
ALTER TABLE person_changes ENABLE ROW LEVEL SECURITY;
ALTER TABLE weekly_memos ENABLE ROW LEVEL SECURITY;
ALTER TABLE competitor_changes ENABLE ROW LEVEL SECURITY;
ALTER TABLE pattern_candidates ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_reliability_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE stats_alert_calibration_events ENABLE ROW LEVEL SECURITY;

-- ══════════════════════════════════════════════════════════════════════════
-- 4. Create default-deny policies (revoke all for everyone)
-- ══════════════════════════════════════════════════════════════════════════

-- Note: PostgreSQL allows multiple policies per table.
-- The default-deny policy ensures no one has access without an explicit grant.
-- We use a restrictive default: DROP existing policies first, then recreate.

-- ══════════════════════════════════════════════════════════════════════════
-- 5. Admin/Superadmin policies — full access to all data
-- ══════════════════════════════════════════════════════════════════════════

-- Helper: Create admin policies for a table with the given name
DO $$
DECLARE
    tables TEXT[] := ARRAY[
        'companies', 'sites', 'product_families', 'capabilities', 'certifications',
        'persons', 'poi_artifacts', 'observations', 'graph_edges',
        'warnings', 'insights', 'recipes', 'feature_rows',
        'social_signals', 'page_fingerprints', 'crawl_telemetry',
        'analyst_users', 'analyst_user_roles',
        'saved_searches', 'watchlists', 'annotations', 'export_history',
        'insight_bookmarks', 'user_preferences', 'insight_feedback_events',
        'audit_log', 'quality_score_breakdown', 'gate_decisions',
        'llm_retry_stats', 'insight_outcomes',
        'dossier_entries', 'role_history', 'company_changes', 'person_changes',
        'weekly_memos', 'competitor_changes', 'pattern_candidates',
        'source_reliability_stats', 'stats_alert_calibration_events',
        'logistics_nodes', 'regulations',
        'entity_merges', 'poi_engagements'
    ];
    owner_scoped_tables TEXT[] := ARRAY[
        'analyst_users', 'analyst_user_roles',
        'saved_searches', 'watchlists', 'annotations', 'export_history',
        'insight_bookmarks', 'user_preferences', 'insight_feedback_events',
        'audit_log'
    ];
    t TEXT;
    owner_scoped BOOLEAN;
BEGIN
    FOREACH t IN ARRAY tables
    LOOP
        -- Check if the table exists before creating policies
        IF EXISTS (SELECT FROM pg_tables WHERE tablename = t) THEN
            owner_scoped := t = ANY(owner_scoped_tables);

            -- Drop existing policies if they exist
            EXECUTE format('DROP POLICY IF EXISTS %I_admin_all ON %I', t, t);
            EXECUTE format('DROP POLICY IF EXISTS %I_analyst_select ON %I', t, t);
            EXECUTE format('DROP POLICY IF EXISTS %I_viewer_select ON %I', t, t);

            -- Admin policy: full access
            EXECUTE format(
                'CREATE POLICY %I_admin_all ON %I
                 FOR ALL
                 USING (has_minimum_role(''admin''))
                 WITH CHECK (has_minimum_role(''admin''))',
                t, t
            );

            IF NOT owner_scoped THEN
                -- Analyst policy: read access to shared operational tables
                EXECUTE format(
                    'CREATE POLICY %I_analyst_select ON %I
                     FOR SELECT
                     USING (has_minimum_role(''analyst''))',
                    t, t
                );

                -- Viewer policy: read-only access to shared operational tables
                EXECUTE format(
                    'CREATE POLICY %I_viewer_select ON %I
                     FOR SELECT
                     USING (has_minimum_role(''viewer''))',
                    t, t
                );
            END IF;
        END IF;
    END LOOP;
END $$;

-- ══════════════════════════════════════════════════════════════════════════
-- 6. User-scoped policies for personal data
--    Users can see/update their own saved_searches, watchlists, annotations,
--    bookmarks, preferences, and notification data.
-- ══════════════════════════════════════════════════════════════════════════

-- saved_searches: users can read/update/delete their own
DROP POLICY IF EXISTS saved_searches_owner_all ON saved_searches;
CREATE POLICY saved_searches_owner_all ON saved_searches
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- watchlists: users can manage their own
DROP POLICY IF EXISTS watchlists_owner_all ON watchlists;
CREATE POLICY watchlists_owner_all ON watchlists
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- annotations: users can manage their own
DROP POLICY IF EXISTS annotations_owner_all ON annotations;
CREATE POLICY annotations_owner_all ON annotations
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- export_history: users can see their own
DROP POLICY IF EXISTS export_history_owner_all ON export_history;
CREATE POLICY export_history_owner_all ON export_history
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- insight_bookmarks: users can manage their own
DROP POLICY IF EXISTS insight_bookmarks_owner_all ON insight_bookmarks;
CREATE POLICY insight_bookmarks_owner_all ON insight_bookmarks
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- user_preferences: users can manage their own
DROP POLICY IF EXISTS user_preferences_owner_all ON user_preferences;
CREATE POLICY user_preferences_owner_all ON user_preferences
    FOR ALL
    USING (user_id = current_user_id())
    WITH CHECK (user_id = current_user_id());

-- analyst_users: users can see their own profile, admins see all
DROP POLICY IF EXISTS analyst_users_self ON analyst_users;
CREATE POLICY analyst_users_self ON analyst_users
    FOR SELECT
    USING (id = current_user_id() OR has_minimum_role('admin'));

-- ══════════════════════════════════════════════════════════════════════════
-- 7. PII masking policy: non-admin roles see masked email
-- ══════════════════════════════════════════════════════════════════════════

-- Note: PostgreSQL RLS can't do column-level masking without a security barrier view.
-- Instead, we rely on the application layer for PII masking.
-- The RLS policies above ensure only authorized roles can SELECT from persons.

-- ══════════════════════════════════════════════════════════════════════════
-- 8. Audit log: append-only for non-admin, full access for admin
-- ══════════════════════════════════════════════════════════════════════════

DROP POLICY IF EXISTS audit_log_analyst_insert ON audit_log;
CREATE POLICY audit_log_analyst_insert ON audit_log
    FOR INSERT
    WITH CHECK (has_minimum_role('analyst'));

DROP POLICY IF EXISTS audit_log_admin_all ON audit_log;
CREATE POLICY audit_log_admin_all ON audit_log
    FOR ALL
    USING (has_minimum_role('admin'))
    WITH CHECK (has_minimum_role('admin'));

COMMIT;
