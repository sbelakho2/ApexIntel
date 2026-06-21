-- ════════════════════════════════════════════════════════════════════════════
-- Seed buyer-relevant POIs (Procurement / Supply Chain / Quality / Operations)
-- ════════════════════════════════════════════════════════════════════════════
-- PURPOSE: The existing seed scripts (reseed_persons.sql, seed_competitors_and_pois.sql)
-- only seed C-Suite executives — every single person is tagged role_family='C-Suite'
-- with a CEO/Chairman/President title. There are ZERO procurement managers, sourcing
-- leads, quality engineers, or operations directors in the seed corpus.
--
-- This is the ROOT CAUSE of the "insights always say contact the CEO" symptom:
-- the database literally has no buyer-relevant contacts to recommend. The
-- buying_center model and the canonical role_classifier are correct, but they
-- can only work with data that exists.
--
-- This migration seeds the FUNCTIONAL buyer-relevant roles that every major EMS /
-- semiconductor company has in its org structure. These are standard, publicly-
-- documented org-chart positions (VP Procurement, VP Supply Chain, Director of
-- Quality, Plant GM, etc.) — the NAMED individuals are discovered at runtime by
-- the crawler's person_scraper + GDELT + OpenCorporates pipeline, but the ROLE
-- SLOTS must exist so insights have buyer-relevant contacts to recommend.
--
-- Each seed gets a real, classifier-accurate role_family (Procurement, Operations,
-- SupplierQuality, etc.) — NOT 'C-Suite'. The PoiRoleReclassify daily job keeps
-- these in sync.
--
-- Idempotent: uses ON CONFLICT DO NOTHING / NOT EXISTS guards.
-- ════════════════════════════════════════════════════════════════════════════

BEGIN;

-- ── Helper: insert a buyer-relevant role slot for a company (by domain) ──────
-- Seeds a role slot only if the company exists AND no person with that exact
-- role slot name+org combination already exists. Named individuals get
-- discovered/enriched by the crawler at runtime.

-- ═══ Foxconn (Hon Hai) ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Global Procurement', c.id, 'VP Global Procurement', 'Procurement', 'Asia-Pacific', 'TW',
  'Head of global procurement and strategic sourcing at Foxconn. Owns supplier selection and component qualification across all business groups.',
  0.78, 'analytical', 'moderate', 'moderate', ARRAY['component sourcing','supplier qualification','cost reduction'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'foxconn.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Global Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Supply Chain Management', c.id, 'VP Supply Chain Management', 'Procurement', 'Asia-Pacific', 'TW',
  'VP of Supply Chain Management at Foxconn. Manages end-to-end supply chain including logistics, inventory, and supplier risk.',
  0.76, 'data_driven', 'moderate', 'high', ARRAY['supply chain resilience','logistics','inventory optimization'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'foxconn.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Supply Chain Management');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Supplier Quality', c.id, 'Director of Supplier Quality', 'SupplierQuality', 'Asia-Pacific', 'TW',
  'Director of Supplier Quality Engineering at Foxconn. Owns SQE team, supplier audits, and incoming quality control.',
  0.70, 'analytical', 'low', 'moderate', ARRAY['supplier audits','IPC standards','quality systems'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'foxconn.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Supplier Quality');

-- ═══ Jabil ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Global Supply Chain', c.id, 'VP Global Supply Chain', 'Procurement', 'North America', 'US',
  'VP of Global Supply Chain at Jabil. Leads strategic sourcing, commodity management, and supplier development.',
  0.77, 'collaborative', 'moderate', 'high', ARRAY['commodity management','strategic sourcing','nearshoring'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'jabil.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Global Supply Chain');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Procurement', c.id, 'Director of Procurement', 'Procurement', 'North America', 'US',
  'Director of Procurement at Jabil. Manages direct materials purchasing, supplier contracts, and RFQ process.',
  0.72, 'pragmatic', 'moderate', 'moderate', ARRAY['RFQ management','supplier contracts','cost reduction'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'jabil.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Operations', c.id, 'VP Operations', 'Operations', 'North America', 'US',
  'VP of Operations at Jabil. Oversees manufacturing operations across global plant network.',
  0.74, 'data_driven', 'moderate', 'moderate', ARRAY['manufacturing operations','lean manufacturing','plant management'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'jabil.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Operations');

-- ═══ Flex ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'SVP Supply Chain', c.id, 'Senior Vice President, Supply Chain', 'Procurement', 'Asia-Pacific', 'SG',
  'SVP of Supply Chain at Flex. Drives global sourcing strategy, supplier diversity, and circular-economy supply initiatives.',
  0.79, 'visionary', 'moderate', 'high', ARRAY['circular economy sourcing','supplier diversity','ESG compliance'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'flex.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Senior Vice President, Supply Chain');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Commodity Management', c.id, 'Director of Commodity Management', 'Procurement', 'Asia-Pacific', 'SG',
  'Director of Commodity Management at Flex. Manages semiconductor, passive component, and PCB commodity strategies.',
  0.71, 'analytical', 'moderate', 'moderate', ARRAY['semiconductor sourcing','passive components','commodity hedging'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'flex.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Commodity Management');

-- ═══ Celestica ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Supply Chain & Procurement', c.id, 'VP Supply Chain and Procurement', 'Procurement', 'North America', 'CA',
  'VP of Supply Chain and Procurement at Celestica. Leads ATS (Advanced Technology Solutions) sourcing and supply base management.',
  0.76, 'collaborative', 'moderate', 'high', ARRAY['ATS sourcing','HPS supply chain','defense procurement'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'celestica.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Supply Chain and Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Supplier Quality Engineering', c.id, 'Director of Supplier Quality Engineering', 'SupplierQuality', 'North America', 'CA',
  'Director of Supplier Quality Engineering at Celestica. Owns AS9100 quality system, supplier audits, and PPAP process.',
  0.70, 'analytical', 'low', 'moderate', ARRAY['AS9100','PPAP','supplier audits','CAPA'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'celestica.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Supplier Quality Engineering');

-- ═══ Sanmina ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Global Procurement', c.id, 'VP Global Procurement', 'Procurement', 'North America', 'US',
  'VP of Global Procurement at Sanmina. Manages optical, medical, and defense commodity sourcing.',
  0.73, 'analytical', 'moderate', 'moderate', ARRAY['optical components','medical device sourcing','defense procurement'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'sanmina.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Global Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Manufacturing Operations', c.id, 'VP Manufacturing Operations', 'Operations', 'North America', 'US',
  'VP of Manufacturing Operations at Sanmina. Oversees high-mix/low-volume production across US, Mexico, and Asia plants.',
  0.71, 'data_driven', 'moderate', 'low', ARRAY['high-mix manufacturing','lean six sigma','plant operations'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'sanmina.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Manufacturing Operations');

-- ═══ Zollner Elektronik ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Head of Strategic Sourcing', c.id, 'Head of Strategic Sourcing', 'Procurement', 'Europe', 'DE',
  'Head of Strategic Sourcing at Zollner Elektronik. Leads component sourcing for automotive and medical EMS customers.',
  0.69, 'analytical', 'low', 'moderate', ARRAY['automotive electronics','medical EMS','Industry 4.0'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'zollner.de'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Head of Strategic Sourcing');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Quality Management', c.id, 'Director of Quality Management', 'SupplierQuality', 'Europe', 'DE',
  'Director of Quality Management at Zollner. Owns ISO 13485 and IATF 16949 quality systems.',
  0.67, 'analytical', 'low', 'low', ARRAY['ISO 13485','IATF 16949','quality management'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'zollner.de'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Quality Management');

-- ═══ Infineon Technologies ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Procurement', c.id, 'VP Procurement', 'Procurement', 'Europe', 'DE',
  'VP of Procurement at Infineon Technologies. Manages wafer sourcing, substrate supply, and assembly subcontractor management.',
  0.78, 'analytical', 'moderate', 'high', ARRAY['wafer sourcing','SiC supply','GaN supply','assembly subcontractors'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'infineon.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Head of Supply Chain Management', c.id, 'Head of Supply Chain Management', 'Procurement', 'Europe', 'DE',
  'Head of Supply Chain Management at Infineon. Owns demand planning, S&OP, and semiconductor capacity allocation.',
  0.76, 'data_driven', 'moderate', 'moderate', ARRAY['demand planning','S&OP','capacity allocation','automotive semiconductors'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'infineon.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Head of Supply Chain Management');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Operations', c.id, 'VP Operations', 'Operations', 'Europe', 'DE',
  'VP of Operations at Infineon. Oversees frontend wafer fab and backend assembly/test operations.',
  0.74, 'data_driven', 'moderate', 'moderate', ARRAY['wafer fab operations','backend assembly','test operations'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'infineon.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Operations');

-- ═══ STMicroelectronics ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Chief Procurement Officer', c.id, 'Chief Procurement Officer', 'Procurement', 'Europe', 'CH',
  'CPO at STMicroelectronics. Owns global procurement including SiC wafer sourcing and substrate partnerships.',
  0.80, 'collaborative', 'moderate', 'high', ARRAY['SiC wafer sourcing','substrate partnerships','strategic procurement'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'st.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Chief Procurement Officer');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Supply Chain', c.id, 'VP Supply Chain', 'Procurement', 'Europe', 'CH',
  'VP of Supply Chain at STMicroelectronics. Manages global logistics, distribution, and customer fulfillment.',
  0.75, 'analytical', 'moderate', 'moderate', ARRAY['global logistics','SiC supply chain','automotive fulfillment'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'st.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Supply Chain');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Quality', c.id, 'VP Quality', 'SupplierQuality', 'Europe', 'CH',
  'VP of Quality at STMicroelectronics. Owns automotive-grade (AEC-Q100) qualification and supplier quality.',
  0.73, 'analytical', 'low', 'moderate', ARRAY['AEC-Q100','automotive qualification','supplier quality'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'st.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Quality');

-- ═══ NXP Semiconductors ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Global Procurement', c.id, 'VP Global Procurement', 'Procurement', 'Europe', 'NL',
  'VP of Global Procurement at NXP. Manages automotive and IoT semiconductor sourcing, foundry partnerships.',
  0.77, 'analytical', 'moderate', 'high', ARRAY['foundry partnerships','automotive semiconductors','edge-AI sourcing'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'nxp.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Global Procurement');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Manufacturing Operations', c.id, 'VP Manufacturing Operations', 'Operations', 'Europe', 'NL',
  'VP of Manufacturing Operations at NXP. Oversees frontend and backend manufacturing across global fabs.',
  0.74, 'data_driven', 'moderate', 'moderate', ARRAY['frontend manufacturing','backend assembly','RISC-V production'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'nxp.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Manufacturing Operations');

-- ═══ Elbit Systems ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Supply Chain Management', c.id, 'VP Supply Chain Management', 'Procurement', 'MENA', 'IL',
  'VP of Supply Chain Management at Elbit Systems. Manages defense procurement, ITAR compliance, and supplier security.',
  0.75, 'analytical', 'low', 'low', ARRAY['defense procurement','ITAR compliance','supplier security','C4ISR'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'elbitsystems.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Supply Chain Management');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Procurement', c.id, 'Director of Procurement', 'Procurement', 'MENA', 'IL',
  'Director of Procurement at Elbit Systems. Manages subcontractor sourcing and defense export compliance.',
  0.70, 'pragmatic', 'low', 'moderate', ARRAY['subcontractor sourcing','defense exports','MIL-SPEC components'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'elbitsystems.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Procurement');

-- ═══ Microchip Technology ═══
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'VP Supply Chain', c.id, 'VP Supply Chain', 'Procurement', 'North America', 'US',
  'VP of Supply Chain at Microchip Technology. Manages MCU, analog, and FPGA supply, including foundry capacity.',
  0.76, 'analytical', 'moderate', 'moderate', ARRAY['MCU supply','foundry capacity','FPGA sourcing','analog sourcing'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'microchip.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'VP Supply Chain');

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 'Director of Supplier Quality', c.id, 'Director of Supplier Quality', 'SupplierQuality', 'North America', 'US',
  'Director of Supplier Quality at Microchip. Owns AEC-Q100 qualification and automotive supplier audits.',
  0.69, 'analytical', 'low', 'moderate', ARRAY['AEC-Q100','automotive supplier audits','quality systems'], '{"engagement_status":"tracked","source":"seed_buyer_roles","role_slot":true}'::jsonb
FROM companies c WHERE c.domain = 'microchip.com'
  AND NOT EXISTS (SELECT 1 FROM persons p WHERE p.primary_org_id = c.id AND p.current_role = 'Director of Supplier Quality');

COMMIT;

-- Index to speed up role_slot lookups (used by the reclassify + insight generation).
CREATE INDEX IF NOT EXISTS idx_persons_role_family
    ON persons(role_family)
    WHERE role_family IS NOT NULL;
