-- 050_heartbeat_grants.sql
--
-- In production the migrations are applied by an administrative role while the
-- application connects as a separate, non-owner role (good practice; the role
-- is not the table owner and does not bypass RLS). New tables therefore need
-- explicit grants.
--
-- 049 created `service_heartbeats` with a BIGSERIAL primary key; without a
-- grant on its sequence the API/worker heartbeat writes fail with
-- "permission denied for sequence service_heartbeats_id_seq".

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'apexintel') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON service_heartbeats TO apexintel;
        GRANT USAGE, SELECT ON SEQUENCE service_heartbeats_id_seq TO apexintel;
    END IF;
END $$;
