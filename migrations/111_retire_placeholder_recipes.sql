-- Migration 111: retire placeholder recipes without executable definitions
-- ════════════════════════════════════════════════════════════════════════════
-- SUPPLY_CHAIN_DISRUPTION, COMPETITOR_EXPANSION and CERT_EXPIRY_RISK are
-- legacy placeholder rows: active, but with definitions that carry neither
-- signals nor a narrative template, so every recipe_fire excludes them and
-- reports a degraded run ("3 recipe(s) were excluded for a missing/invalid
-- definition"). They cannot fire by construction; keeping them active is a
-- data-quality defect, not coverage. The executable equivalents live in the
-- seeded recipe catalog (A/B/C/D series).

UPDATE recipes
SET status = 'deprecated',
    updated_at = now()
WHERE code IN ('SUPPLY_CHAIN_DISRUPTION', 'COMPETITOR_EXPANSION', 'CERT_EXPIRY_RISK')
  AND status IN ('active', 'promoted', 'staging')
  AND (
      definition IS NULL
      OR definition::text NOT LIKE '%signals%'
      OR definition::text NOT LIKE '%narrative_template%'
  );
