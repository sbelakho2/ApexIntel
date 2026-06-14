-- ════════════════════════════════════════════════════════════════════
-- ApexIntel: BESS / Battery Energy Storage Pivot — Reference Entities Seed
-- Purpose: anchor the BESS pivot with real battery pack/BMS competitors
--          (size-matched to Starz) and upstream cell-supplier references,
--          so competitor-customer discovery, POI expansion and the BESS
--          recipe family have battery seed entities to work from.
-- Source: public corporate information. Financials left NULL (not fabricated).
-- Run with:
--   PGPASSWORD="${PGPASSWORD}" psql -h 127.0.0.1 -U apexintel -d apexintel -f scripts/seed_bess_entities.sql
-- Idempotent: ON CONFLICT (domain) DO UPDATE; safe to re-run.
-- ════════════════════════════════════════════════════════════════════

BEGIN;

-- ─── 1. BESS PACK / SYSTEM + BMS COMPETITORS (size-matched to Starz) ─────────
-- Flagged is_competitor (column + metadata) so the wired competitor-customer
-- discovery scraper (metadata->>'is_competitor' = 'true') crawls their customer
-- pages and surfaces medium-sized BESS buyers.
INSERT INTO companies
  (name, legal_name, domain, country_code, region, company_type, industry_tags,
   employee_estimate, revenue_estimate_usd, risk_score, threat_score, overlap_score,
   strategic_relevance, is_competitor, metadata)
VALUES
-- Residential / commercial LFP pack & system makers
('Pylontech', 'Pylon Technologies Co., Ltd.', 'pylontech.com', 'CN', 'Asia-Pacific', 'BESS',
  ARRAY['energy_storage','battery','lifepo4','residential_storage','commercial_storage'],
  NULL, NULL, 0.0, 0.42, 0.58, 0.62, TRUE,
  '{"is_competitor": true, "bess_role": "pack_competitor", "segment": "residential_commercial_storage", "hq": "Shanghai, China"}'),
('Dyness', 'Jiangsu Dyness Technology Co., Ltd.', 'dyness.com', 'CN', 'Asia-Pacific', 'BESS',
  ARRAY['energy_storage','battery','lifepo4','residential_storage','commercial_storage'],
  NULL, NULL, 0.0, 0.40, 0.56, 0.60, TRUE,
  '{"is_competitor": true, "bess_role": "pack_competitor", "segment": "residential_commercial_storage", "hq": "Taizhou, China"}'),
('Pytes', 'Pytes Energy Co., Ltd.', 'pytesgroup.com', 'CN', 'Asia-Pacific', 'BESS',
  ARRAY['energy_storage','battery','lifepo4','residential_storage'],
  NULL, NULL, 0.0, 0.38, 0.55, 0.58, TRUE,
  '{"is_competitor": true, "bess_role": "pack_competitor", "segment": "residential_storage", "hq": "Jiangsu, China"}'),
('BSLBATT', 'Huizhou BSLBATT Smart Energy Technology Co., Ltd.', 'bslbatt.com', 'CN', 'Asia-Pacific', 'BESS',
  ARRAY['energy_storage','battery','lifepo4','battery_pack','telecom_backup'],
  NULL, NULL, 0.0, 0.38, 0.55, 0.58, TRUE,
  '{"is_competitor": true, "bess_role": "pack_competitor", "segment": "residential_commercial_telecom_storage", "hq": "Huizhou, China"}'),
('Fox ESS', 'FOX ESS Co., Ltd.', 'fox-ess.com', 'CN', 'Asia-Pacific', 'BESS',
  ARRAY['energy_storage','battery','lifepo4','residential_storage','hybrid_inverter'],
  NULL, NULL, 0.0, 0.39, 0.54, 0.59, TRUE,
  '{"is_competitor": true, "bess_role": "pack_competitor", "segment": "residential_commercial_storage", "hq": "Wuxi, China"}'),
-- Battery Management System (BMS) competitors — Starz makes/sells BMS in-house
('Nuvation Energy', 'Nuvation Energy', 'nuvationenergy.com', 'US', 'North America', 'BESS',
  ARRAY['battery','bms','energy_storage','bess_controls'],
  NULL, NULL, 0.0, 0.36, 0.60, 0.62, TRUE,
  '{"is_competitor": true, "bess_role": "bms_competitor", "segment": "battery_management_systems", "hq": "Sunnyvale, CA, USA"}'),
('Ewert Energy Systems', 'Ewert Energy Systems, Inc.', 'ewertenergy.com', 'US', 'North America', 'BESS',
  ARRAY['battery','bms','energy_storage','bess_controls'],
  NULL, NULL, 0.0, 0.34, 0.58, 0.60, TRUE,
  '{"is_competitor": true, "bess_role": "bms_competitor", "segment": "battery_management_systems", "hq": "Chicago, IL, USA"}')
ON CONFLICT (domain) DO UPDATE SET
  region              = EXCLUDED.region,
  company_type        = EXCLUDED.company_type,
  industry_tags       = EXCLUDED.industry_tags,
  threat_score        = EXCLUDED.threat_score,
  overlap_score       = EXCLUDED.overlap_score,
  strategic_relevance = EXCLUDED.strategic_relevance,
  is_competitor       = EXCLUDED.is_competitor,
  metadata            = companies.metadata || EXCLUDED.metadata,
  updated_at          = now();

-- ─── 2. BATTERY CELL SUPPLIERS — upstream reference entities (NOT competitors) ─
-- Starz BUYS cells; these are supply-chain partners / supply-risk anchors.
-- High strategic_relevance, low overlap, is_competitor = FALSE.
INSERT INTO companies
  (name, legal_name, domain, country_code, region, company_type, industry_tags,
   employee_estimate, revenue_estimate_usd, risk_score, threat_score, overlap_score,
   strategic_relevance, is_competitor, metadata)
VALUES
('CATL', 'Contemporary Amperex Technology Co., Limited', 'catl.com', 'CN', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','lifepo4','nmc','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.85, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["LFP","NMC"], "ticker": "300750.SZ", "hq": "Ningde, China"}'),
('BYD', 'BYD Company Limited', 'byd.com', 'CN', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','lifepo4','blade_battery','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.82, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["LFP"], "ticker": "002594.SZ", "hq": "Shenzhen, China"}'),
('LG Energy Solution', 'LG Energy Solution, Ltd.', 'lgensol.com', 'KR', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','nmc','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.80, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["NMC"], "ticker": "373220.KS", "hq": "Seoul, South Korea"}'),
('Samsung SDI', 'Samsung SDI Co., Ltd.', 'samsungsdi.com', 'KR', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','nmc','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.78, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["NMC"], "ticker": "006400.KS", "hq": "Yongin, South Korea"}'),
('Panasonic Energy', 'Panasonic Holdings Corporation', 'panasonic.com', 'JP', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','nca','nmc','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.76, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["NCA","NMC"], "ticker": "6752.T", "hq": "Osaka, Japan"}'),
('EVE Energy', 'EVE Energy Co., Ltd.', 'evebattery.com', 'CN', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','lifepo4','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.76, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["LFP"], "ticker": "300014.SZ", "hq": "Huizhou, China"}'),
('Gotion High-tech', 'Gotion High-tech Co., Ltd.', 'gotion.com', 'CN', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','lifepo4','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.74, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["LFP"], "ticker": "002074.SZ", "hq": "Hefei, China"}'),
('Sunwoda', 'Sunwoda Electronic Co., Ltd.', 'sunwoda.com', 'CN', 'Asia-Pacific', 'Cell Manufacturer',
  ARRAY['battery','cell_manufacturing','lifepo4','nmc','battery_pack','energy_storage'],
  NULL, NULL, 0.0, 0.0, 0.10, 0.74, FALSE,
  '{"bess_role": "cell_supplier", "chemistry": ["LFP","NMC"], "ticker": "300207.SZ", "hq": "Shenzhen, China"}')
ON CONFLICT (domain) DO UPDATE SET
  region              = EXCLUDED.region,
  company_type        = EXCLUDED.company_type,
  industry_tags       = EXCLUDED.industry_tags,
  strategic_relevance = EXCLUDED.strategic_relevance,
  metadata            = companies.metadata || EXCLUDED.metadata,
  updated_at          = now();

-- ─── 3. CAPABILITIES (real, chemistry-accurate) ─────────────────────────────
INSERT INTO capabilities (company_id, capability, proof_grade)
SELECT c.id, cap.capability, cap.grade
FROM (VALUES
  -- Pack / system competitors
  ('pylontech.com', 'Battery Pack Assembly', 'B'),
  ('pylontech.com', 'Battery Management System', 'B'),
  ('pylontech.com', 'Energy Storage System Integration', 'B'),
  ('dyness.com', 'Battery Pack Assembly', 'B'),
  ('dyness.com', 'Battery Management System', 'B'),
  ('dyness.com', 'Energy Storage System Integration', 'B'),
  ('pytesgroup.com', 'Battery Pack Assembly', 'B'),
  ('pytesgroup.com', 'Battery Management System', 'B'),
  ('bslbatt.com', 'Battery Pack Assembly', 'B'),
  ('bslbatt.com', 'Battery Management System', 'B'),
  ('fox-ess.com', 'Battery Pack Assembly', 'B'),
  ('fox-ess.com', 'Energy Storage System Integration', 'B'),
  -- BMS competitors
  ('nuvationenergy.com', 'Battery Management System', 'A'),
  ('nuvationenergy.com', 'BESS Controls', 'B'),
  ('ewertenergy.com', 'Battery Management System', 'A'),
  ('ewertenergy.com', 'BESS Controls', 'B'),
  -- Cell suppliers (chemistry-accurate)
  ('catl.com', 'Lithium-Ion Cell Manufacturing', 'A'),
  ('catl.com', 'LFP Cell Manufacturing', 'A'),
  ('byd.com', 'LFP Cell Manufacturing', 'A'),
  ('lgensol.com', 'Lithium-Ion Cell Manufacturing', 'A'),
  ('lgensol.com', 'NMC Cell Manufacturing', 'A'),
  ('samsungsdi.com', 'NMC Cell Manufacturing', 'A'),
  ('panasonic.com', 'Lithium-Ion Cell Manufacturing', 'A'),
  ('evebattery.com', 'LFP Cell Manufacturing', 'A'),
  ('gotion.com', 'LFP Cell Manufacturing', 'A'),
  ('sunwoda.com', 'Lithium-Ion Cell Manufacturing', 'A'),
  ('sunwoda.com', 'Battery Pack Assembly', 'A')
) AS cap(domain, capability, grade)
JOIN companies c ON c.domain = cap.domain
WHERE NOT EXISTS (
  SELECT 1 FROM capabilities x WHERE x.company_id = c.id AND x.capability = cap.capability
);

COMMIT;

-- ─── Verification (run manually after seeding) ──────────────────────────────
-- SELECT company_type, COUNT(*) FROM companies
--   WHERE metadata->>'bess_role' IS NOT NULL GROUP BY company_type;
-- SELECT name, domain, is_competitor, metadata->>'bess_role' AS bess_role
--   FROM companies WHERE metadata->>'bess_role' IS NOT NULL ORDER BY is_competitor DESC, name;
