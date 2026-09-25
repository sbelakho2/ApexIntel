-- 057_user_private_rls_force.sql
--
-- Enables FORCE ROW LEVEL SECURITY on the identity-scoped user-private tables.
--
-- Why this is now safe (it was not before):
--   * 051 enabled RLS and the scoped code paths set `app.current_user_id` /
--     `app.current_user_role` through `PgStore::begin_scoped`.
--   * The connection pool now assumes a default `service` identity
--     (`set_config('app.current_user_role','service',false)`) so legitimate
--     unscoped paths — email-digest preferences scan, admin/system reads —
--     are explicitly identified instead of running with no identity.
--   * This migration adds a `service` policy to each table, so the
--     application role still has the access it needs while every other role
--     is constrained by the user-scoped policies.
--
-- Verified by crates/store/tests/rls_scoped_integration.rs (scoped isolation,
-- service access, and FORCE present).

DO $$
DECLARE
    t TEXT;
    user_private_tables TEXT[] := ARRAY[
        'user_preferences',
        'watchlists',
        'saved_searches',
        'notifications'
    ];
BEGIN
    FOREACH t IN ARRAY user_private_tables
    LOOP
        IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = t) THEN
            EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
            EXECUTE format('DROP POLICY IF EXISTS %I ON %I', t || '_service_all', t);
            EXECUTE format(
                'CREATE POLICY %I ON %I FOR ALL
                   USING (current_setting(''app.current_user_role'', true) = ''service'')
                   WITH CHECK (current_setting(''app.current_user_role'', true) = ''service'')',
                t || '_service_all', t
            );
            EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        END IF;
    END LOOP;
END $$;
