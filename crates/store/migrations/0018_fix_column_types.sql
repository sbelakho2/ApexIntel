-- ApexIntel Phase 4.3: Fix Column Types
-- Migration: 0018_fix_column_types
-- Description: Fix type mismatches between Rust code (f64, Uuid) and DB (NUMERIC, VARCHAR)
-- The original migration 0042 used DECIMAL(5,4) for score fields and VARCHAR for ID fields,
-- but the Rust structs expect DOUBLE PRECISION (f64) and UUID types.
-- Since CREATE TABLE IF NOT EXISTS left existing tables unchanged, the types stayed as DECIMAL/VARCHAR.

BEGIN;

-- ─────────────────────────────────────────────────────────────────────────────
-- Drop dependent views before altering column types
-- executive_summary_view uses priority_score, impact_score, and confidence
-- ─────────────────────────────────────────────────────────────────────────────

DROP VIEW IF EXISTS executive_summary_view CASCADE;

-- ─────────────────────────────────────────────────────────────────────────────
-- Fix NUMERIC → DOUBLE PRECISION for score/confidence/probability columns
-- These must match Rust f64 type (sqlx maps DOUBLE PRECISION ↔ f64, not NUMERIC)
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE strategic_opportunities 
    ALTER COLUMN priority_score TYPE DOUBLE PRECISION USING priority_score::double precision,
    ALTER COLUMN confidence TYPE DOUBLE PRECISION USING confidence::double precision;

ALTER TABLE critical_threats 
    ALTER COLUMN impact_score TYPE DOUBLE PRECISION USING impact_score::double precision,
    ALTER COLUMN confidence TYPE DOUBLE PRECISION USING confidence::double precision;

ALTER TABLE supplier_risk 
    ALTER COLUMN risk_score TYPE DOUBLE PRECISION USING risk_score::double precision;

ALTER TABLE pipeline_opportunities 
    ALTER COLUMN probability TYPE DOUBLE PRECISION USING probability::double precision;

ALTER TABLE source_evidence 
    ALTER COLUMN reliability_score TYPE DOUBLE PRECISION USING reliability_score::double precision;

-- ─────────────────────────────────────────────────────────────────────────────
-- Recreate executive_summary_view
-- ─────────────────────────────────────────────────────────────────────────────

CREATE OR REPLACE VIEW executive_summary_view AS
 SELECT 'opportunity'::text AS category,
    strategic_opportunities.id,
    strategic_opportunities.title,
    strategic_opportunities.description,
    strategic_opportunities.priority_score AS score,
    strategic_opportunities.confidence,
    strategic_opportunities.region,
    strategic_opportunities.owner_id,
    strategic_opportunities.status,
    strategic_opportunities.created_at
   FROM strategic_opportunities
  WHERE ((strategic_opportunities.status)::text = 'active'::text)
UNION ALL
 SELECT 'threat'::text AS category,
    critical_threats.id,
    critical_threats.title,
    critical_threats.description,
    critical_threats.impact_score AS score,
    critical_threats.confidence,
    critical_threats.region,
    critical_threats.owner_id,
    critical_threats.status,
    critical_threats.created_at
   FROM critical_threats
  WHERE ((critical_threats.status)::text = ANY ((ARRAY['active'::character varying, 'escalated'::character varying])::text[]));

-- ─────────────────────────────────────────────────────────────────────────────
-- Note: VARCHAR columns (supplier_id, opportunity_id, item_id, entity_id)
-- are kept as VARCHAR because they contain string IDs like 'sup-001', 'opp-101', etc.
-- The Rust structs have been updated to use String instead of Uuid for these fields.
-- ─────────────────────────────────────────────────────────────────────────────

COMMIT;
