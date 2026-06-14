-- ════════════════════════════════════════════════════════════════════
-- ApexIntel: BESS Target-Market Demand Anchors Seed
-- Purpose: anchor Starz's go-to-market geography with REAL demand-side
--          reference entities (utilities, energy agencies, EPCs and project
--          developers) in the PRIMARY sales markets — Morocco, Tunisia and
--          Egypt — so the geo-weighted insight ranking, the demand-procurement
--          recipes (BESS001-003) and the strategic-POI recipes have in-market
--          buyer entities to surface and prioritise from day one.
--
-- Scope: DEMAND SIDE ONLY. Every entity here is a potential battery-pack buyer
--        / project owner — NOT a competitor and NOT a cell supplier. Therefore
--        is_competitor = FALSE and company_type deliberately avoids supply-chain
--        markers ("cell", "distributor", …) so the geo targeting applies the
--        PRIMARY-market demand boost rather than treating them as the globally
--        monitored supply / competitor cohort.
--
-- Geography: country_code is set explicitly (MA / TN / EG). The geo classifier
--        keys on the ISO country code first, so each entity resolves to the
--        PRIMARY market tier deterministically, independent of the region label.
--
-- Source: public corporate / institutional information; every domain was
--         verified live and identity-checked before inclusion. Financials are
--         left NULL (never fabricated).
-- Run with:
--   PGPASSWORD="${PGPASSWORD}" psql -h 127.0.0.1 -U apexintel -d apexintel -f scripts/seed_bess_target_markets.sql
-- Idempotent: ON CONFLICT (domain) DO UPDATE; safe to re-run.
-- ════════════════════════════════════════════════════════════════════

BEGIN;

-- ─── Primary sales markets: Morocco (MA), Tunisia (TN), Egypt (EG) ───────────
INSERT INTO companies
  (name, legal_name, domain, country_code, region, company_type, industry_tags,
   employee_estimate, revenue_estimate_usd, risk_score, threat_score, overlap_score,
   strategic_relevance, is_competitor, metadata)
VALUES
-- Morocco ────────────────────────────────────────────────────────────────────
('Masen', 'Moroccan Agency for Sustainable Energy (Agence Marocaine pour l''Énergie Durable)', 'masen.ma', 'MA', 'North Africa', 'Public Energy Developer',
  ARRAY['renewables','solar','energy_storage','project_developer','bess_buyer','utility_scale'],
  NULL, NULL, 0.0, 0.0, 0.08, 0.82, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "public_developer", "market_tier": "primary", "segment": "utility_scale_storage", "hq": "Rabat, Morocco"}'),
('Nareva', 'Nareva Holding', 'nareva.ma', 'MA', 'North Africa', 'Energy Developer',
  ARRAY['renewables','wind','solar','energy_storage','ipp','project_developer','bess_buyer'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.74, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "ipp", "market_tier": "primary", "segment": "utility_commercial_storage", "hq": "Casablanca, Morocco"}'),
-- Tunisia ─────────────────────────────────────────────────────────────────────
('STEG', 'Société Tunisienne de l''Électricité et du Gaz', 'steg.com.tn', 'TN', 'North Africa', 'Utility',
  ARRAY['utility','electricity','grid','energy_storage','bess_buyer','utility_scale'],
  NULL, NULL, 0.0, 0.0, 0.08, 0.80, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "utility", "market_tier": "primary", "segment": "grid_storage", "hq": "Tunis, Tunisia"}'),
('ANME', 'Agence Nationale pour la Maîtrise de l''Énergie', 'anme.tn', 'TN', 'North Africa', 'Government Energy Agency',
  ARRAY['energy_efficiency','renewables','solar','energy_storage','policy','bess_buyer'],
  NULL, NULL, 0.0, 0.0, 0.06, 0.70, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "government_agency", "market_tier": "primary", "segment": "distributed_storage_programs", "hq": "Tunis, Tunisia"}'),
-- Egypt ───────────────────────────────────────────────────────────────────────
('Elsewedy Electric', 'Elsewedy Electric S.A.E.', 'elsewedyelectric.com', 'EG', 'North Africa', 'Energy EPC',
  ARRAY['epc','electrical','renewables','solar','energy_storage','integration','bess_buyer'],
  NULL, NULL, 0.0, 0.0, 0.12, 0.80, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "epc_integrator", "market_tier": "primary", "segment": "commercial_utility_storage", "hq": "Cairo, Egypt"}'),
('NREA', 'New and Renewable Energy Authority', 'nrea.gov.eg', 'EG', 'North Africa', 'Government Energy Agency',
  ARRAY['renewables','solar','wind','energy_storage','project_developer','bess_buyer','utility_scale'],
  NULL, NULL, 0.0, 0.0, 0.07, 0.78, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "government_developer", "market_tier": "primary", "segment": "utility_scale_storage", "hq": "Cairo, Egypt"}'),
('KarmSolar', 'KarmSolar', 'karmsolar.com', 'EG', 'North Africa', 'Solar Developer',
  ARRAY['solar','off_grid','energy_storage','mini_grid','project_developer','bess_buyer','commercial'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.72, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "solar_developer", "market_tier": "primary", "segment": "commercial_offgrid_storage", "hq": "Cairo, Egypt"}'),
('TAQA Arabia', 'TAQA Arabia', 'taqa.com.eg', 'EG', 'North Africa', 'Energy Solutions',
  ARRAY['energy','solar','renewables','energy_storage','project_developer','bess_buyer','commercial'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.70, FALSE,
  '{"bess_role": "demand_anchor", "buyer_type": "energy_solutions", "market_tier": "primary", "segment": "commercial_industrial_storage", "hq": "Cairo, Egypt"}')
ON CONFLICT (domain) DO UPDATE SET
  country_code        = EXCLUDED.country_code,
  region              = EXCLUDED.region,
  company_type        = EXCLUDED.company_type,
  industry_tags       = EXCLUDED.industry_tags,
  strategic_relevance = EXCLUDED.strategic_relevance,
  overlap_score       = EXCLUDED.overlap_score,
  is_competitor       = EXCLUDED.is_competitor,
  metadata            = companies.metadata || EXCLUDED.metadata,
  updated_at          = now();

COMMIT;

-- ─── Verification (run manually after seeding) ──────────────────────────────
-- SELECT name, country_code, company_type, is_competitor, metadata->>'market_tier' AS tier
--   FROM companies WHERE metadata->>'bess_role' = 'demand_anchor'
--   ORDER BY country_code, name;
