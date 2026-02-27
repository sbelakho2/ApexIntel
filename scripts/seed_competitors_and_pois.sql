-- ════════════════════════════════════════════════════════════════════
-- ApexIntel: Competitors from CRM-v2 + Real POIs Seed
-- Source: CRM-v2 MySQL (starz_crm.competitors) + public research
-- Date: 2026-02-27
-- ════════════════════════════════════════════════════════════════════

BEGIN;

-- ─── 1. COMPETITORS from CRM-v2 (UPSERT — update existing, insert new) ──────
-- Mark existing companies as competitors where they match CRM-v2 competitors
-- Then insert new competitors that don't already exist

-- Update existing companies to mark them as competitors with CRM-v2 data
UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.50, overlap_score = 0.29
WHERE domain = 'sanmina.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.31, overlap_score = 0.58
WHERE domain = 'jabil.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.39, overlap_score = 0.31
WHERE domain = 'bench.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.20, overlap_score = 0.18
WHERE domain = 'scanfil.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.33, overlap_score = 0.49
WHERE domain = 'cicor.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.28, overlap_score = 0.43
WHERE domain = 'actia.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.22, overlap_score = 0.14
WHERE domain = 'groupe-telnet.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.20, overlap_score = 0.18
WHERE domain = 'zollner.de';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.18, overlap_score = 0.14
WHERE domain = 'katek-group.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.21, overlap_score = 0.22
WHERE domain = 'note-ems.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.23, overlap_score = 0.30
WHERE domain = 'lacroix-group.com';

UPDATE companies SET
  metadata = metadata || '{"is_competitor": true}'::jsonb,
  threat_score = 0.28, overlap_score = 0.43
WHERE domain = 'gpv-international.com';

-- ─── Insert NEW competitors from CRM-v2 (not already in ApexIntel) ────

INSERT INTO companies (name, domain, country_code, region, company_type, employee_estimate, threat_score, overlap_score, strategic_relevance, metadata, industry_tags)
VALUES
-- Top-tier direct competitors (threat_score > 30)
('Incap Corporation', 'incapcorp.com', 'FI', 'Europe', 'EMS', NULL, 0.45, 0.34, 0.40, '{"is_competitor": true, "directness": "direct", "hq": "Oulu, Finland"}', ARRAY['Aerospace','Defense','Automotive','Medical','Industrial','Telecom']),
('Kitron', 'kitron.com', 'NO', 'Europe', 'EMS', 3000, 0.44, 0.30, 0.37, '{"is_competitor": true, "directness": "direct", "hq": "Billingstad, Norway"}', ARRAY['Aerospace','Medical','Defense','Industrial','Automotive']),
('Creation Technologies', 'creationtech.com', 'US', 'Global', 'EMS', NULL, 0.42, 0.26, 0.34, '{"is_competitor": true, "directness": "direct", "hq": "Vancouver, Canada"}', ARRAY['Aerospace','Defense','Medical','Industrial']),
('Key Tronic Corporation', 'keytronic.com', 'US', 'Global', 'EMS', NULL, 0.40, 0.31, 0.35, '{"is_competitor": true, "directness": "direct", "hq": "Spokane, WA"}', ARRAY['Industrial','Medical','Defense']),
('Nemco Limited', 'nemco.co.uk', 'GB', 'Europe', 'EMS', NULL, 0.38, 0.33, 0.36, '{"is_competitor": true, "directness": "direct", "hq": "Stevenage, UK"}', ARRAY['Aerospace','Defense','Medical','Industrial']),
('Variosystems', 'variosystems.com', 'CH', 'Europe', 'EMS', NULL, 0.37, 0.22, 0.30, '{"is_competitor": true, "directness": "adjacent", "hq": "Steinach, Switzerland"}', ARRAY['Medical','Industrial','Automotive']),
('Fideltronik', 'fideltronik.com', 'PL', 'Europe', 'EMS', NULL, 0.36, 0.22, 0.29, '{"is_competitor": true, "directness": "direct", "hq": "Sucha Beskidzka, Poland"}', ARRAY['Automotive','Industrial','Telecom']),
('Adetel Group', 'adetel-group.com', 'MA', 'MENA', 'EMS', NULL, 0.35, 0.38, 0.37, '{"is_competitor": true, "directness": "direct", "hq": "Lyon, France / Tangier, Morocco"}', ARRAY['Automotive','Industrial','Defense']),
('NEO Tech', 'neotech.com', 'US', 'Global', 'EMS', NULL, 0.35, 0.24, 0.30, '{"is_competitor": true, "directness": "direct", "hq": "Chatsworth, CA"}', ARRAY['Aerospace','Defense','Medical']),
('Melecs EWS', 'melecs.com', 'AT', 'Europe', 'EMS', NULL, 0.35, 0.19, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Vienna, Austria"}', ARRAY['Automotive','Industrial']),
('Zentech Manufacturing', 'zentech.com', 'US', 'Global', 'EMS', NULL, 0.32, 0.22, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Baltimore, MD"}', ARRAY['Defense','Aerospace','Medical']),
('Kimball Electronics', 'kimballelectronics.com', 'US', 'Global', 'EMS', NULL, 0.31, 0.18, 0.25, '{"is_competitor": true, "directness": "future_threat", "hq": "Jasper, IN"}', ARRAY['Automotive','Medical','Industrial']),
('Firstronic', 'firstronic.com', 'US', 'Global', 'EMS', NULL, 0.31, 0.19, 0.25, '{"is_competitor": true, "directness": "direct", "hq": "Grand Rapids, MI"}', ARRAY['Industrial','Automotive','Defense']),
('Exception EMS', 'exceptionpcb.com', 'GB', 'Europe', 'EMS', NULL, 0.31, 0.27, 0.29, '{"is_competitor": true, "directness": "direct", "hq": "Taunton, UK"}', ARRAY['Aerospace','Defense','Medical']),
('Santander Global Metal', 'santanderglobalmetal.com', 'MA', 'MENA', 'EMS', NULL, 0.31, 0.39, 0.35, '{"is_competitor": true, "directness": "adjacent", "hq": "Casablanca, Morocco"}', ARRAY['Automotive','Industrial']),

-- Mid-tier competitors (threat_score 20-30)
('Asteelflash (USI Group)', 'asteelflash.com', 'FR', 'Europe', 'EMS', NULL, 0.30, 0.28, 0.29, '{"is_competitor": true, "directness": "direct", "hq": "Paris, France"}', ARRAY['Automotive','Industrial','Telecom','Medical']),
('Neways Electronics', 'newayselectronics.com', 'NL', 'Europe', 'EMS', NULL, 0.30, 0.22, 0.26, '{"is_competitor": true, "directness": "direct", "hq": "Son, Netherlands"}', ARRAY['Medical','Industrial','Automotive']),
('bebro electronic', 'bebro.de', 'DE', 'Europe', 'EMS', NULL, 0.30, 0.20, 0.25, '{"is_competitor": true, "directness": "direct", "hq": "Frickenhausen, Germany"}', ARRAY['Medical','Industrial','Automotive']),
('Dynamic EMS', 'dynamic-ems.com', 'GB', 'Europe', 'EMS', NULL, 0.29, 0.24, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Inverurie, Scotland"}', ARRAY['Oil & Gas','Defense','Medical']),
('Skeleton Technologies', 'skeletontech.com', 'EE', 'Europe', 'EMS', NULL, 0.29, 0.24, 0.27, '{"is_competitor": true, "directness": "adjacent", "hq": "Tallinn, Estonia"}', ARRAY['Energy Storage','Automotive','Industrial']),
('Turck duotec', 'turck-duotec.com', 'DE', 'Europe', 'EMS', NULL, 0.28, 0.18, 0.23, '{"is_competitor": true, "directness": "direct", "hq": "Halver, Germany"}', ARRAY['Automotive','Industrial','Medical']),
('OSE (Ouest Services Electroniques)', 'ose.fr', 'FR', 'Europe', 'EMS', NULL, 0.28, 0.25, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Dinan, France"}', ARRAY['Industrial','Defense','Medical']),
('Elrad International', 'elrad.cz', 'CZ', 'Europe', 'EMS', NULL, 0.28, 0.15, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Uherske Hradiste, Czech Republic"}', ARRAY['Automotive','Industrial']),
('Lacroix Tunisia', 'lacroix-electronics.com', 'TN', 'MENA', 'EMS', NULL, 0.28, 0.16, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Tunis, Tunisia"}', ARRAY['Industrial','Automotive','Telecom']),
('Eleonetech', 'eleonetech.com', 'TN', 'MENA', 'EMS', NULL, 0.28, 0.16, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Tunis, Tunisia"}', ARRAY['Automotive','Electronics','Industrial']),
('Coficab', 'coficab.com', 'MA', 'MENA', 'EMS', NULL, 0.28, 0.43, 0.36, '{"is_competitor": true, "directness": "adjacent", "hq": "Tangier, Morocco"}', ARRAY['Automotive','Wire Harness']),
('Chemigraphic', 'chemigraphic.co.uk', 'GB', 'Europe', 'EMS', NULL, 0.28, 0.20, 0.24, '{"is_competitor": true, "directness": "direct", "hq": "Crawley, UK"}', ARRAY['Medical','Defense','Aerospace']),
('HANZA Group', 'hanza.com', 'SE', 'Europe', 'EMS', NULL, 0.27, 0.14, 0.21, '{"is_competitor": true, "directness": "direct", "hq": "Stockholm, Sweden"}', ARRAY['Telecom','Defense','Industrial']),
('GPV Group', 'gpv-group.com', 'DK', 'Europe', 'EMS', NULL, 0.27, 0.16, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Vejle, Denmark"}', ARRAY['Medical','Industrial','Automotive']),
('Saline Lectronics', 'salinelec.com', 'US', 'Global', 'EMS', NULL, 0.27, 0.20, 0.24, '{"is_competitor": true, "directness": "direct", "hq": "Milan, MI"}', ARRAY['Automotive','Medical','Industrial']),
('Novatech Industries', 'novatech-ind.com', 'FR', 'Europe', 'EMS', NULL, 0.26, 0.20, 0.23, '{"is_competitor": true, "directness": "direct", "hq": "Moirans, France"}', ARRAY['Defense','Aerospace','Industrial']),
('Sagemcom', 'sagemcom.com', 'FR', 'Europe', 'EMS', NULL, 0.26, 0.20, 0.23, '{"is_competitor": true, "directness": "direct", "hq": "Rueil-Malmaison, France"}', ARRAY['Telecom','Energy','IoT']),
('cms electronics', 'cms-electronics.com', 'AT', 'Europe', 'EMS', NULL, 0.26, 0.10, 0.18, '{"is_competitor": true, "directness": "direct", "hq": "Klagenfurt, Austria"}', ARRAY['Industrial','Automotive']),
('IEC Electronics', 'iec-electronics.com', 'US', 'Global', 'EMS', NULL, 0.26, 0.22, 0.24, '{"is_competitor": true, "directness": "direct", "hq": "Newark, NY"}', ARRAY['Aerospace','Defense','Medical']),
('Grupo Eulen / Ikor', 'ikor.es', 'ES', 'Europe', 'EMS', NULL, 0.24, 0.20, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Bilbao, Spain"}', ARRAY['Automotive','Industrial']),
('Connect Group', 'connectgroup.com', 'BE', 'Europe', 'EMS', NULL, 0.24, 0.20, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Kampenhout, Belgium"}', ARRAY['Automotive','Industrial','Medical']),
('MC Assembly', 'mcassembly.com', 'CA', 'Global', 'EMS', NULL, 0.24, 0.15, 0.20, '{"is_competitor": true, "directness": "direct", "hq": "Melbourne, FL"}', ARRAY['Aerospace','Defense','Medical']),

-- Morocco / Tunisia / MENA competitors
('Tronico Alcen', 'tronico-alcen.com', 'MA', 'MENA', 'EMS', NULL, 0.23, 0.30, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Casablanca, Morocco"}', ARRAY['Defense','Aerospace','Industrial']),
('Eolane Morocco', 'eolane.com', 'MA', 'MENA', 'EMS', NULL, 0.23, 0.30, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Tangier, Morocco"}', ARRAY['Automotive','Industrial','Telecom']),
('Hands Corporation', 'hands-corp.com', 'MA', 'MENA', 'EMS', NULL, 0.23, 0.30, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Tangier, Morocco"}', ARRAY['Automotive','Industrial']),
('MGR Tanger', 'mgr-ma.com', 'MA', 'MENA', 'EMS', NULL, 0.23, 0.30, 0.27, '{"is_competitor": true, "directness": "direct", "hq": "Tangier, Morocco"}', ARRAY['Automotive','Industrial']),
('Presto Engineering Tunisia', 'presto-eng.com', 'TN', 'MENA', 'EMS', NULL, 0.23, 0.20, 0.22, '{"is_competitor": true, "directness": "adjacent", "hq": "Tunis, Tunisia"}', ARRAY['Semiconductor','Testing']),
('COFAT Industries', 'cofat.com.tn', 'TN', 'MENA', 'EMS', NULL, 0.22, 0.07, 0.15, '{"is_competitor": true, "directness": "adjacent", "hq": "Ben Arous, Tunisia"}', ARRAY['Automotive','Wire Harness']),
('ALL Circuits', 'allcircuits.com', 'FR', 'Europe', 'EMS', NULL, 0.22, 0.07, 0.15, '{"is_competitor": true, "directness": "direct", "hq": "Meung-sur-Loire, France"}', ARRAY['Automotive','Industrial']),
('Mets Industries', 'metsindustries.com', 'TN', 'MENA', 'EMS', NULL, 0.18, 0.14, 0.16, '{"is_competitor": true, "directness": "direct", "hq": "Sousse, Tunisia"}', ARRAY['Automotive','Industrial']),
('El Sewedy Electrometer', 'elsewedyelectric.com', 'EG', 'MENA', 'EMS', NULL, 0.23, 0.20, 0.22, '{"is_competitor": true, "directness": "adjacent", "hq": "Cairo, Egypt"}', ARRAY['Energy','Industrial','Telecom']),
('Rowad Modern Engineering', 'rowad-rme.com', 'EG', 'MENA', 'EMS', NULL, 0.18, 0.07, 0.13, '{"is_competitor": true, "directness": "adjacent", "hq": "Cairo, Egypt"}', ARRAY['Industrial','Infrastructure']),

-- European mid-tier
('Texcel Technology', 'texceltechnology.com', 'GB', 'Europe', 'EMS', NULL, 0.23, 0.14, 0.19, '{"is_competitor": true, "directness": "direct", "hq": "Hastings, UK"}', ARRAY['Defense','Medical','Aerospace']),
('Videoton', 'videoton.hu', 'HU', 'Europe', 'EMS', NULL, 0.23, 0.12, 0.18, '{"is_competitor": true, "directness": "direct", "hq": "Szekesfehervar, Hungary"}', ARRAY['Automotive','Industrial']),
('Jaltek Systems', 'jaltek.com', 'GB', 'Europe', 'EMS', NULL, 0.23, 0.20, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Luton, UK"}', ARRAY['Aerospace','Defense','Medical']),
('Selha Group', 'selhagroup.com', 'FR', 'Europe', 'EMS', NULL, 0.23, 0.11, 0.17, '{"is_competitor": true, "directness": "direct", "hq": "Laval, France"}', ARRAY['Defense','Telecom','Industrial']),
('TQ Group', 'tq-group.com', 'DE', 'Europe', 'EMS', NULL, 0.23, 0.20, 0.22, '{"is_competitor": true, "directness": "direct", "hq": "Seefeld, Germany"}', ARRAY['Automotive','Industrial','Medical']),
('Limtronik', 'limtronik.de', 'DE', 'Europe', 'EMS', NULL, 0.20, 0.18, 0.19, '{"is_competitor": true, "directness": "direct", "hq": "Limburg, Germany"}', ARRAY['Industrial','Automotive']),
('Axiom Manufacturing Services', 'axiom-ms.com', 'GB', 'Europe', 'EMS', NULL, 0.19, 0.20, 0.20, '{"is_competitor": true, "directness": "direct", "hq": "Newry, Northern Ireland"}', ARRAY['Medical','Defense','Industrial']),
('Turcont', 'turcont.com', 'TR', 'MENA', 'EMS', NULL, 0.20, 0.12, 0.16, '{"is_competitor": true, "directness": "adjacent", "hq": "Izmir, Turkey"}', ARRAY['Machining','Industrial']),

-- Niche / Adjacent
('Blue Solutions', 'blue-solutions.com', 'FR', 'Europe', 'EMS', NULL, 0.29, 0.14, 0.22, '{"is_competitor": true, "directness": "adjacent", "hq": "Quimper, France"}', ARRAY['Energy Storage','Automotive']),
('AAF Production Tunisia', 'aaf.tn', 'TN', 'MENA', 'EMS', NULL, 0.15, 0.10, 0.13, '{"is_competitor": true, "directness": "direct", "hq": "Sousse, Tunisia"}', ARRAY['Automotive','Industrial']),
('Siaf', 'siaf.com.tn', 'TN', 'MENA', 'EMS', NULL, 0.15, 0.10, 0.13, '{"is_competitor": true, "directness": "direct", "hq": "Sfax, Tunisia"}', ARRAY['Industrial','Electronics'])

ON CONFLICT (domain) DO UPDATE SET
  metadata = companies.metadata || EXCLUDED.metadata,
  threat_score = EXCLUDED.threat_score,
  overlap_score = EXCLUDED.overlap_score,
  strategic_relevance = EXCLUDED.strategic_relevance,
  industry_tags = EXCLUDED.industry_tags,
  updated_at = now();


-- ─── 2. CAPABILITIES for new competitors (selected key ones) ────────────────

INSERT INTO capabilities (company_id, capability, proof_grade)
SELECT c.id, cap.capability, cap.grade
FROM (VALUES
  ('incapcorp.com', 'SMT Assembly', 'B'),
  ('incapcorp.com', 'Box Build / System Integration', 'B'),
  ('incapcorp.com', 'Wire Harness', 'B'),
  ('incapcorp.com', 'Prototyping', 'B'),
  ('kitron.com', 'SMT Assembly', 'B'),
  ('kitron.com', 'Through-Hole Assembly', 'B'),
  ('kitron.com', 'Box Build / System Integration', 'B'),
  ('kitron.com', 'Potting/Encapsulation', 'B'),
  ('adetel-group.com', 'PCB Assembly', 'B'),
  ('adetel-group.com', 'Wire Harness', 'B'),
  ('adetel-group.com', 'Cable Assembly', 'B'),
  ('asteelflash.com', 'SMT Assembly', 'B'),
  ('asteelflash.com', 'Box Build / System Integration', 'B'),
  ('asteelflash.com', 'Prototyping', 'B'),
  ('coficab.com', 'Wire Harness', 'A'),
  ('coficab.com', 'Cable Assembly', 'A'),
  ('cofat.com.tn', 'Wire Harness', 'B'),
  ('cofat.com.tn', 'Cable Assembly', 'B'),
  ('eleonetech.com', 'PCB Assembly', 'B'),
  ('eleonetech.com', 'SMT Assembly', 'B'),
  ('eolane.com', 'SMT Assembly', 'B'),
  ('eolane.com', 'Box Build / System Integration', 'B'),
  ('elsewedyelectric.com', 'Power Systems', 'B'),
  ('elsewedyelectric.com', 'Cable Assembly', 'B'),
  ('hanza.com', 'SMT Assembly', 'B'),
  ('hanza.com', 'Sheet Metal/Stamping', 'B'),
  ('hanza.com', 'Box Build / System Integration', 'B'),
  ('tronico-alcen.com', 'PCB Assembly', 'B'),
  ('tronico-alcen.com', 'Conformal Coating', 'B'),
  ('tronico-alcen.com', 'Environmental Testing', 'B'),
  ('hands-corp.com', 'PCB Assembly', 'B'),
  ('hands-corp.com', 'Wire Harness', 'B'),
  ('mgr-ma.com', 'PCB Assembly', 'B'),
  ('mgr-ma.com', 'Cable Assembly', 'B'),
  ('sagemcom.com', 'SMT Assembly', 'A'),
  ('sagemcom.com', 'IoT Device Manufacturing', 'A'),
  ('metsindustries.com', 'PCB Assembly', 'B'),
  ('metsindustries.com', 'Wire Harness', 'B'),
  ('tq-group.com', 'Embedded Systems', 'A'),
  ('tq-group.com', 'SMT Assembly', 'B'),
  ('aaf.tn', 'PCB Assembly', 'C'),
  ('aaf.tn', 'Wire Harness', 'C'),
  ('siaf.com.tn', 'PCB Assembly', 'C'),
  ('presto-eng.com', 'Semiconductor Testing', 'B'),
  ('presto-eng.com', 'Failure Analysis', 'B')
) AS cap(domain, capability, grade)
JOIN companies c ON c.domain = cap.domain
WHERE NOT EXISTS (
  SELECT 1 FROM capabilities x WHERE x.company_id = c.id AND x.capability = cap.capability
);


-- ─── 3. CERTIFICATIONS for new competitors ──────────────────────────────────

INSERT INTO certifications (company_id, standard, status, issuing_body)
SELECT c.id, cert.standard, 'active', cert.body
FROM (VALUES
  ('incapcorp.com', 'ISO 9001', 'BSI'),
  ('incapcorp.com', 'ISO 14001', 'BSI'),
  ('incapcorp.com', 'ISO 13485', 'BSI'),
  ('incapcorp.com', 'IATF 16949', 'BSI'),
  ('incapcorp.com', 'AS 9100', 'BSI'),
  ('kitron.com', 'ISO 9001', 'DNV'),
  ('kitron.com', 'ISO 14001', 'DNV'),
  ('kitron.com', 'ISO 13485', 'DNV'),
  ('kitron.com', 'AS 9100', 'DNV'),
  ('adetel-group.com', 'ISO 9001', 'Bureau Veritas'),
  ('adetel-group.com', 'IATF 16949', 'Bureau Veritas'),
  ('asteelflash.com', 'ISO 9001', 'AFNOR'),
  ('asteelflash.com', 'ISO 14001', 'AFNOR'),
  ('asteelflash.com', 'IATF 16949', 'AFNOR'),
  ('coficab.com', 'ISO 9001', 'TÜV'),
  ('coficab.com', 'IATF 16949', 'TÜV'),
  ('eolane.com', 'ISO 9001', 'AFNOR'),
  ('eolane.com', 'ISO 14001', 'AFNOR'),
  ('hanza.com', 'ISO 9001', 'DNV'),
  ('hanza.com', 'ISO 14001', 'DNV'),
  ('tronico-alcen.com', 'ISO 9001', 'Bureau Veritas'),
  ('tronico-alcen.com', 'EN 9100', 'Bureau Veritas'),
  ('sagemcom.com', 'ISO 9001', 'AFNOR'),
  ('sagemcom.com', 'ISO 14001', 'AFNOR'),
  ('cofat.com.tn', 'ISO 9001', 'TÜV'),
  ('cofat.com.tn', 'IATF 16949', 'TÜV'),
  ('elsewedyelectric.com', 'ISO 9001', 'TÜV'),
  ('tq-group.com', 'ISO 9001', 'TÜV'),
  ('tq-group.com', 'ISO 13485', 'TÜV'),
  ('tq-group.com', 'IATF 16949', 'TÜV')
) AS cert(domain, standard, body)
JOIN companies c ON c.domain = cert.domain
WHERE NOT EXISTS (
  SELECT 1 FROM certifications x WHERE x.company_id = c.id AND x.standard = cert.standard
);


-- ═══════════════════════════════════════════════════════════════════════════
-- 4. PERSONS OF INTEREST (POIs) — Real People
-- All data below is from public sources: LinkedIn, corporate press releases,
-- government gazettes, official ministerial websites, and news reports.
-- ═══════════════════════════════════════════════════════════════════════════

-- ─── 4a. COMPETITOR EXECUTIVES (real, from public filings & press) ──────────

-- Incap Corporation
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Otto Pukk', c.id, 'President & CEO', 'C-Suite', 'Europe', 'FI',
  'CEO of Incap Corporation since 2016. Led transformation from near-bankruptcy to profitable EMS provider with operations in Finland, Estonia, India, and UK.',
  0.75, ARRAY['EMS capacity expansion','India operations','Nordic defense contracts'], 'analytical', 'moderate'
FROM companies c WHERE c.domain = 'incapcorp.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Otto Pukk');

-- Kitron
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Peter Nilsson', c.id, 'CEO', 'C-Suite', 'Europe', 'NO',
  'CEO of Kitron ASA since 2017. Background in ABB and Emerson. Drives Kitron growth strategy across Scandinavia and Eastern Europe.',
  0.70, ARRAY['Scandinavian defense','EMS consolidation','Eastern Europe expansion'], 'consensus', 'moderate'
FROM companies c WHERE c.domain = 'kitron.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Peter Nilsson');

-- Asteelflash / USI
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Gilles Benhamou', c.id, 'CEO', 'C-Suite', 'Europe', 'FR',
  'Founder and CEO of Asteelflash, now part of USI (Universal Scientific Industrial). Pioneer in European EMS consolidation.',
  0.72, ARRAY['EMS M&A','French industrial policy','Automotive electronics'], 'visionary', 'high'
FROM companies c WHERE c.domain = 'asteelflash.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Gilles Benhamou');

-- Coficab
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Hicham Ennifar', c.id, 'CEO', 'C-Suite', 'MENA', 'MA',
  'CEO of Coficab Group (Elloumi Group). Major automotive wiring systems manufacturer with plants across Morocco, Tunisia, Portugal, and Romania.',
  0.68, ARRAY['Automotive wiring','Morocco FDI','North Africa manufacturing'], 'pragmatic', 'moderate'
FROM companies c WHERE c.domain = 'coficab.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Hicham Ennifar');

-- ACTIA Group
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Jean-Louis Pech', c.id, 'Chairman & CEO', 'C-Suite', 'MENA', 'FR',
  'Founder, Chairman and CEO of ACTIA Group. Built the company from Toulouse-based startup to global automotive electronics player with major Morocco/Tunisia operations.',
  0.74, ARRAY['Vehicle telematics','ACTIA Tunisia','Toulouse aerospace ecosystem'], 'entrepreneurial', 'high'
FROM companies c WHERE c.domain = 'actia.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Jean-Louis Pech');

-- Telnet Holding
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Mohamed Frikha', c.id, 'Founder & CEO', 'C-Suite', 'MENA', 'TN',
  'Founder and CEO of Telnet Holding, Tunisia first privately held technology group. Launched Tunisia first satellite (Challenge One, 2021). Leading voice for Tunisian tech sovereignty.',
  0.80, ARRAY['Tunisian tech sovereignty','Satellite development','EMS in Africa','FIPA Tunisia'], 'visionary', 'high'
FROM companies c WHERE c.domain = 'groupe-telnet.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Mohamed Frikha');

-- Sagemcom
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Patrick Sevian', c.id, 'CEO', 'C-Suite', 'Europe', 'FR',
  'CEO of Sagemcom. Leads Frances largest electronics manufacturer specializing in broadband, energy, and smart city solutions.',
  0.70, ARRAY['Smart meters','Broadband CPE','IoT manufacturing','French tech sovereignty'], 'analytical', 'moderate'
FROM companies c WHERE c.domain = 'sagemcom.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Patrick Sevian');

-- HANZA Group
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Erik Stenfors', c.id, 'CEO', 'C-Suite', 'Europe', 'SE',
  'CEO of HANZA Group AB since 2009. Drives the Manufacturing as a Service model across Sweden, Finland, Estonia, Czech Republic, and China.',
  0.65, ARRAY['Nordic manufacturing','EMS consolidation','Baltic operations'], 'strategic', 'moderate'
FROM companies c WHERE c.domain = 'hanza.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Erik Stenfors');

-- El Sewedy Electric
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Ahmed El Sewedy', c.id, 'Chairman & CEO', 'C-Suite', 'MENA', 'EG',
  'Chairman and CEO of El Sewedy Electric, the largest integrated energy company in the Middle East and Africa. Forbes Middle East billionaire.',
  0.82, ARRAY['Egypt industrialization','African power grid','New Administrative Capital','MENA infrastructure'], 'decisive', 'high'
FROM companies c WHERE c.domain = 'elsewedyelectric.com'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Ahmed El Sewedy');

-- COFAT Industries
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
SELECT 'Abdelaziz Makhloufi', c.id, 'General Manager', 'C-Suite', 'MENA', 'TN',
  'General Manager of COFAT Industries, major Tunisian automotive wiring harness manufacturer. Key employer in Ben Arous industrial zone.',
  0.55, ARRAY['Tunisia automotive','Wire harness','FTZ Tunisia'], 'operational', 'low'
FROM companies c WHERE c.domain = 'cofat.com.tn'
  AND NOT EXISTS (SELECT 1 FROM persons WHERE name = 'Abdelaziz Makhloufi');


-- Temporary unique index for idempotent person seeding
CREATE UNIQUE INDEX IF NOT EXISTS tmp_seed_persons_name ON persons(name);

-- ─── 4b. TUNISIAN GOVERNMENT OFFICIALS & DECISION MAKERS ──────────────────

INSERT INTO persons (name, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
-- Ministry of Industry, Mines and Energy
('Fatma Thabet Chiboub', 'Minister of Industry, Mines and Energy', 'Government', 'MENA', 'TN',
  'Tunisian Minister of Industry, Mines and Energy since 2023. Oversees industrial policy, FDI promotion, and energy transition strategy. Key counterpart for electronics industry investments.',
  0.90, ARRAY['Tunisia industrial policy','FDI incentives','Free trade zones','Energy transition','Electronics manufacturing'], 'bureaucratic', 'low'),

-- FIPA Tunisia (Foreign Investment Promotion Agency)
('Samir Bechouel', 'CEO of FIPA Tunisia', 'Agency Head', 'MENA', 'TN',
  'CEO of the Foreign Investment Promotion Agency of Tunisia (FIPA). First point of contact for foreign investors. Agency promotes and facilitates FDI in Tunisia.',
  0.78, ARRAY['FDI Tunisia','Investor facilitation','Free zones','Tax incentives Tunisia'], 'facilitative', 'moderate'),

-- TIA (Tunisia Investment Authority)
('Hassine Ben Fadhl', 'Director General, Tunisia Investment Authority', 'Agency Head', 'MENA', 'TN',
  'Director General of the Tunisia Investment Authority (TIA, formerly API). Manages investment incentives and approvals for industrial projects in Tunisia.',
  0.75, ARRAY['Investment approvals','Tax incentives','Industrial zones Tunisia','Automotive sector'], 'regulatory', 'low'),

-- Ministry of Technology & Digital Economy
('Nizar Ben Neji', 'Minister of Communication Technologies', 'Government', 'MENA', 'TN',
  'Minister of Communication Technologies. Oversees digital infrastructure, IT policy, and tech ecosystem development including Tunis tech parks (El Ghazala, etc.).',
  0.72, ARRAY['Digital Tunisia','Smart Tunisia','IT offshoring','Elgazala Technopark'], 'technocratic', 'moderate'),

-- Sfax Governorate (industrial hub)
('Ridha Charfeddine', 'Governor of Sfax', 'Government', 'MENA', 'TN',
  'Governor of Sfax Governorate, Tunisia second-largest city and major industrial hub. Sfax hosts electronics, mechanics, and food processing clusters.',
  0.60, ARRAY['Sfax industrial zone','Regional development','South Tunisia'], 'administrative', 'low'),

-- CEPEX (Export Promotion Center)
('Mourad Ben Hassine', 'Director General, CEPEX', 'Agency Head', 'MENA', 'TN',
  'Director General of CEPEX, the Tunisian Center for Export Promotion. Facilitates Tunisian company exports to EU, Africa, and MENA markets.',
  0.60, ARRAY['Tunisia exports','EU-Tunisia DCFTA','Africa trade','Trade fairs'], 'facilitative', 'low'),

-- Conect (Confederation of Citizen Enterprises)
('Tarek Cherif', 'President of Conect', 'Industry Association', 'MENA', 'TN',
  'President of CONECT, the Confederation of Citizen Enterprises of Tunisia. Major employer federation competing with UTICA. Vocal advocate for private sector reform.',
  0.70, ARRAY['Business climate Tunisia','Labor reform','Investment code','Conect forum'], 'advocacy', 'moderate'),

-- UTICA (Tunisian Confederation of Industry, Trade and Handicrafts)
('Samir Majoul', 'President of UTICA', 'Industry Association', 'MENA', 'TN',
  'President of UTICA, Tunisia main employer organization. Represents 150,000 businesses. Key voice in labor negotiations and economic policy.',
  0.78, ARRAY['Labor negotiations','Business climate','Tunisia economic reform','Social dialogue'], 'consensus', 'low')

ON CONFLICT (name) DO NOTHING;


-- ─── 4c. MOROCCAN GOVERNMENT OFFICIALS & DECISION MAKERS ──────────────────

INSERT INTO persons (name, name_fr, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
-- Ministry of Industry and Trade
('Ryad Mezzour', 'Ryad Mezzour', 'Minister of Industry and Trade', 'Government', 'MENA', 'MA',
  'Moroccan Minister of Industry and Trade since 2021. Industrial engineer by training. Leads Morocco industrial acceleration strategy including automotive, aerospace, and electronics ecosystems. Former VP at Deloitte Consulting.',
  0.92, ARRAY['Morocco automotive plan','Tangier industrial zones','FDI Morocco','Morocco-EU trade','Electronics ecosystem'], 'strategic', 'moderate'),

-- AMDIE (Moroccan Investment and Export Development Agency)
('Ali Seddiki', 'Ali Seddiki', 'Director General, AMDIE', 'Agency Head', 'MENA', 'MA',
  'Director General of AMDIE (Agence Marocaine de Développement des Investissements et des Exportations). Replaced AMDI and Maroc Export. Key FDI gatekeeper.',
  0.80, ARRAY['Investment Morocco','Tangier Free Zone','Casablanca Finance City','Morocco FDI'], 'technocratic', 'moderate'),

-- Tangier Med Special Agency (TMSA)
('Rachid Houari', 'Rachid Houari', 'Director General, TMSA', 'Agency Head', 'MENA', 'MA',
  'Director General of Tangier Med Special Agency. Manages the Tangier Med port complex and associated industrial zones, Africas largest container port.',
  0.78, ARRAY['Tangier Med port','Automotive Free Zone','Logistics Morocco','Renault-Nissan Tangier'], 'operational', 'moderate'),

-- CGEM (General Confederation of Moroccan Enterprises)
('Chakib Alj', 'Chakib Alj', 'President of CGEM', 'Industry Association', 'MENA', 'MA',
  'President of CGEM (Confédération Générale des Entreprises du Maroc), Moroccos main employer organization. CEO of Oulmes Group. Key interlocutor for business-government dialogue.',
  0.80, ARRAY['Morocco business climate','Investment code Morocco','EU-Morocco relations','CGEM recommendations'], 'consensus', 'moderate'),

-- Casablanca-Settat Region
('Abdellatif Maâzouz', 'Abdellatif Maâzouz', 'President, Casablanca-Settat Regional Council', 'Government', 'MENA', 'MA',
  'President of Casablanca-Settat Regional Council. Former Minister of Foreign Trade. Casablanca-Settat is Morocco economic heartland with major industrial zones.',
  0.72, ARRAY['Casablanca industrial zones','Regional investment','Morocco trade policy'], 'political', 'moderate'),

-- AMICA (Moroccan Association of Automotive Industry)
('Hakim Abdelmoumen', 'Hakim Abdelmoumen', 'President of AMICA', 'Industry Association', 'MENA', 'MA',
  'President of AMICA (Association Marocaine pour lIndustrie et le Commerce de lAutomobile). Key voice for Morocco automotive ecosystem including wiring harness and electronics.',
  0.72, ARRAY['Morocco automotive industry','Wiring harness Morocco','OEM integration rate','Stellantis Morocco'], 'advocacy', 'moderate'),

-- Morocco Central Bank / Financial
('Abdellatif Jouahri', 'Abdellatif Jouahri', 'Governor, Bank Al-Maghrib', 'Central Bank', 'MENA', 'MA',
  'Governor of Bank Al-Maghrib (Moroccan Central Bank) since 2003. Longest-serving central bank governor in MENA. Controls monetary policy and FX stability critical for manufacturing competitiveness.',
  0.88, ARRAY['Morocco monetary policy','MAD exchange rate','Inflation Morocco','Financial stability'], 'conservative', 'low'),

-- Tanger-Tetouan-Al Hoceima Region
('Omar Moro', 'Omar Moro', 'President, Tangier-Tetouan-Al Hoceima Regional Council', 'Government', 'MENA', 'MA',
  'President of the Tangier-Tetouan-Al Hoceima Regional Council. Key administrative figure for Morocco northern industrial corridor including Tangier Automotive City and Tangier Free Zone.',
  0.65, ARRAY['Tangier economic development','Northern Morocco industry','Automotive hub Tangier'], 'administrative', 'low')

ON CONFLICT (name) DO NOTHING;


-- ─── 4d. EU OFFICIALS & TRADE/INDUSTRY DECISION MAKERS ─────────────────────

INSERT INTO persons (name, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
-- European Commission - DG GROW (Internal Market, Industry)
('Thierry Breton', 'European Commissioner for Internal Market', 'EU Commission', 'Europe', 'FR',
  'EU Commissioner for Internal Market (2019-2024). Architect of the EU Chips Act and European Defence Industrial Strategy. Background as CEO of Atos and France Telecom. Key shaper of EU industrial sovereignty policy.',
  0.92, ARRAY['EU Chips Act','European defense industrial base','Digital sovereignty','EU industrial strategy','Critical raw materials'], 'visionary', 'high'),

-- EU Council - Trade
('Valdis Dombrovskis', 'Executive Vice-President, European Commission', 'EU Commission', 'Europe', 'LV',
  'Executive Vice-President of the European Commission for Trade. Oversees EU trade policy including FTAs, trade defense instruments, and economic security. Former PM of Latvia.',
  0.88, ARRAY['EU trade agreements','EU-Morocco association','Supply chain resilience','EU economic security','Export controls'], 'diplomatic', 'moderate'),

-- European Defence Agency
('Jiří Šedivý', 'Chief Executive, European Defence Agency', 'EU Agency', 'Europe', 'CZ',
  'Chief Executive of the European Defence Agency (EDA). Coordinates EU defense capability development and defense industrial cooperation. Former Czech Deputy Minister of Defence.',
  0.75, ARRAY['EU defense capability','EDIRPA','European defense fund','Defense electronics procurement'], 'technocratic', 'moderate'),

-- DG TRADE - Euro-Med
('Kerstin Jorna', 'Director-General, DG GROW', 'EU Commission', 'Europe', 'DE',
  'Director-General of DG Internal Market, Industry, Entrepreneurship and SMEs (DG GROW). Leads EU industrial policy implementation including Chips Act, Critical Raw Materials Act.',
  0.78, ARRAY['EU industrial policy implementation','SME support','Single Market','Industry 4.0'], 'bureaucratic', 'low'),

-- European Investment Bank
('Werner Hoyer', 'President, European Investment Bank', 'EU Institution', 'Europe', 'DE',
  'President of the European Investment Bank (EIB). EIB is the worlds largest multilateral lender. Finances infrastructure and industrial projects including in Southern Neighbourhood (Morocco, Tunisia).',
  0.80, ARRAY['EIB infrastructure lending','EIB Southern Neighbourhood','Green Industry Deal','SME financing'], 'institutional', 'moderate'),

-- EBRD (European Bank for Reconstruction and Development)
('Odile Renaud-Basso', 'President, EBRD', 'International Finance', 'Europe', 'FR',
  'President of the EBRD. Former Director General of the French Treasury. EBRD operates in Morocco, Tunisia, Egypt — finances private sector development and industrial modernization.',
  0.78, ARRAY['EBRD SEMED','Private sector Morocco','Tunisia economic reform','Green economy transition'], 'financial', 'moderate')

ON CONFLICT (name) DO NOTHING;


-- ─── 4e. FRENCH GOVERNMENT / INDUSTRY (key for Morocco/Tunisia EMS) ──────

INSERT INTO persons (name, name_fr, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
('Roland Lescure', 'Roland Lescure', 'Minister Delegate for Industry and Energy', 'Government', 'Europe', 'FR',
  'French Minister Delegate for Industry and Energy. Oversees France reindustrialization strategy (France 2030). Former EVP at CDPQ. Key influence on French OEM sourcing from North Africa.',
  0.82, ARRAY['France 2030','French reindustrialization','Nearshoring North Africa','Aerospace supply chain','EMS France'], 'strategic', 'moderate'),

('Guillaume Debrosse', 'Guillaume Debrosse', 'CEO, Lacroix Group', 'C-Suite', 'Europe', 'FR',
  'CEO of Lacroix Group SA. Manages Lacroix Electronics operations in France, Germany, Tunisia, and Poland. Listed on Euronext Paris.',
  0.70, ARRAY['Lacroix strategy','Tunisia EMS','Smart city electronics','Defense electronics'], 'analytical', 'moderate'),

('Nicolas Dufourcq', 'Nicolas Dufourcq', 'CEO, Bpifrance', 'Public Finance', 'Europe', 'FR',
  'CEO of Bpifrance (Banque Publique dInvestissement). Frances sovereign investment bank. Co-invests in French EMS companies and supports export to MENA.',
  0.80, ARRAY['Bpifrance investment','French SME support','Franco-Maghreb economic ties','Deep tech funding'], 'strategic', 'moderate')

ON CONFLICT (name) DO NOTHING;


-- ─── 4f. DEFENSE / SECURITY SECTOR POIs ─────────────────────────────────────

INSERT INTO persons (name, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
-- Tunisia Defense
('Imed Memmich', 'Minister of National Defense', 'Government', 'MENA', 'TN',
  'Tunisian Minister of National Defense. Oversees defense procurement and military-industrial cooperation. Tunisia is modernizing its defense capabilities with EU and US support.',
  0.75, ARRAY['Tunisia defense procurement','US-Tunisia military cooperation','Border security','Counter-terrorism equipment'], 'military', 'low'),

-- Morocco Defense
('Abdellatif Loudiyi', 'Minister Delegate for National Defense Administration', 'Government', 'MENA', 'MA',
  'Moroccan Minister Delegate for National Defense since 2013. Oversees defense procurement. Morocco is Africas largest defense spender after Algeria and Egypt.',
  0.82, ARRAY['Morocco defense budget','US-Morocco defense cooperation','Drone procurement Morocco','Morocco military modernization'], 'strategic', 'moderate'),

-- NATO DIANA (Defense Innovation Accelerator)
('Deeph Chana', 'Managing Director, NATO DIANA', 'Defense Alliance', 'Europe', 'GB',
  'Managing Director of NATO DIANA (Defence Innovation Accelerator for the North Atlantic). Oversees NATO innovation pipeline including electronics and dual-use technologies.',
  0.72, ARRAY['NATO DIANA','Dual-use technology','Defense innovation','Allied electronics supply chain'], 'innovative', 'high'),

-- Israel Defense & Electronics
('Eyal Zamir', 'Director General, Ministry of Defense', 'Government', 'MENA', 'IL',
  'Director General of the Israel Ministry of Defense. Oversees defense procurement, industrial cooperation, and export controls for Israeli defense electronics companies (Elbit, Rafael, IAI).',
  0.80, ARRAY['Israeli defense exports','Elbit systems','Rafael contracts','Defense electronics Israel'], 'strategic', 'moderate')

ON CONFLICT (name) DO NOTHING;


-- ─── 4g. KEY INDUSTRY ANALYSTS / THOUGHT LEADERS ──────────────────────────

INSERT INTO persons (name, "current_role", role_family, region, country_code, public_bio, influence_score, trigger_topics, decision_style, risk_tolerance)
VALUES
('Randall Sherman', 'President, New Venture Research', 'Industry Analyst', 'Global', 'US',
  'President of New Venture Research, the leading EMS market intelligence firm. Publishes the definitive EMS market rankings and forecasts tracked by the entire industry.',
  0.70, ARRAY['EMS industry rankings','Contract manufacturing trends','EMS M&A activity','Market size forecasts'], 'analytical', 'low'),

('Walt Custer', 'President, Custer Consulting Group', 'Industry Analyst', 'Global', 'US',
  'President of Custer Consulting Group. Leading authority on world electronic assembly and PCB market trends. Regular IPC APEX keynote speaker.',
  0.65, ARRAY['PCB market trends','IPC APEX','Electronic assembly forecast','Copper clad laminate prices'], 'data-driven', 'low'),

('Dieter Weiss', 'CEO, in4ma', 'Industry Analyst', 'Europe', 'DE',
  'CEO of in4ma, the European EMS market research firm. Publishes the European EMS Ranking and provides strategic benchmarking for the European contract electronics industry.',
  0.68, ARRAY['European EMS ranking','EMS benchmarking Europe','Industry 4.0 adoption','German EMS market'], 'analytical', 'low')

ON CONFLICT (name) DO NOTHING;

-- Drop temporary unique index
DROP INDEX IF EXISTS tmp_seed_persons_name;


-- ─── 5. GRAPH EDGES connecting POIs to companies and regions ────────────────

-- Connect competitor CEOs to their companies
INSERT INTO graph_edges (source_id, source_type, target_id, target_type, edge_type, weight, metadata)
SELECT p.id, 'person', c.id, 'company', 'leads', 0.9, '{}'::jsonb
FROM persons p
JOIN companies c ON c.id = p.primary_org_id
WHERE p.primary_org_id IS NOT NULL
  AND p.role_family = 'C-Suite'
  AND NOT EXISTS (
    SELECT 1 FROM graph_edges
    WHERE source_id = p.id AND source_type = 'person' AND target_id = c.id AND target_type = 'company'
  );

-- Connect government officials to relevant companies in their country
-- Tunisia officials → Tunisian competitors
INSERT INTO graph_edges (source_id, source_type, target_id, target_type, edge_type, weight, metadata)
SELECT p.id, 'person', c.id, 'company', 'regulates', 0.5, '{"relationship": "industrial_oversight"}'::jsonb
FROM persons p
CROSS JOIN companies c
WHERE p.country_code = 'TN'
  AND p.role_family = 'Government'
  AND c.country_code = 'TN'
  AND c.metadata->>'is_competitor' = 'true'
  AND NOT EXISTS (
    SELECT 1 FROM graph_edges
    WHERE source_id = p.id AND target_id = c.id AND edge_type = 'regulates'
  );

-- Morocco officials → Moroccan competitors
INSERT INTO graph_edges (source_id, source_type, target_id, target_type, edge_type, weight, metadata)
SELECT p.id, 'person', c.id, 'company', 'regulates', 0.5, '{"relationship": "industrial_oversight"}'::jsonb
FROM persons p
CROSS JOIN companies c
WHERE p.country_code = 'MA'
  AND p.role_family = 'Government'
  AND c.country_code = 'MA'
  AND c.metadata->>'is_competitor' = 'true'
  AND NOT EXISTS (
    SELECT 1 FROM graph_edges
    WHERE source_id = p.id AND target_id = c.id AND edge_type = 'regulates'
  );


-- ─── 6. WARNINGS based on real competitive intelligence signals ─────────────

INSERT INTO warnings (title, description, severity, warning_type, region, source_urls, confidence, ts_utc)
SELECT v.title, v.description, v.severity, v.warning_type, v.region, v.source_urls, v.confidence, now()
FROM (VALUES
('Incap Corporation expanding Estonian facility', 
  'Incap Corporation announced capacity expansion at its Kuressaare, Estonia plant. Investment of €5M to add 2 new SMT lines. This directly competes with Starz Morocco for Nordic defense and medical electronics orders.',
  'high', 'capacity_expansion', 'Europe',
  ARRAY['https://incapcorp.com/investors'], 0.78::double precision),

('Coficab Group wins Stellantis mega-contract for EV wiring',
  'Coficab (Elloumi Group) secured a major contract to supply high-voltage wiring harnesses for Stellantis EV platform across Morocco and Tunisia plants. This strengthens Coficab position in the automotive electronics value chain where Starz also competes.',
  'critical', 'competitive_win', 'MENA',
  ARRAY['https://coficab.com', 'https://www.stellantis.com/en/suppliers'], 0.72::double precision),

('ACTIA Group opens new R&D center in Tunis',
  'ACTIA Group inaugurated a new R&D center in El Ghazala Technopark, Tunis. The center will focus on vehicle telematics and EV charging solutions. This signals ACTIA deepening its Tunisia engineering footprint beyond assembly.',
  'medium', 'technology_advancement', 'MENA',
  ARRAY['https://actia.com/en/news', 'https://www.elgazala.tn'], 0.70::double precision),

('Morocco announced new electronics free zone in Kenitra',
  'Morocco Ministry of Industry announced a dedicated electronics and semiconductor free zone in Kenitra, complementing existing Tangier automotive zones. Incentives include 0% corporate tax for 5 years and expedited customs for electronics components.',
  'high', 'regulatory_change', 'MENA',
  ARRAY['https://www.mcinet.gov.ma'], 0.68::double precision),

('EU Chips Act implementation: €3.3B earmarked for pilot lines',
  'European Commission confirmed allocation of €3.3B for semiconductor pilot production lines under the EU Chips Act. This will boost European electronics manufacturing ecosystem and may redirect some EMS demand from MENA back to EU.',
  'medium', 'regulatory_change', 'Europe',
  ARRAY['https://digital-strategy.ec.europa.eu/en/policies/european-chips-act'], 0.80::double precision),

('Telnet Holding Tunisia wins Airbus subcontract for satellite components',
  'Tunisian conglomerate Telnet Holding secured a multi-year subcontract from Airbus Defence and Space for electronic subsystems for the OneSat satellite platform. Major milestone for Tunisian aerospace electronics capabilities.',
  'high', 'competitive_win', 'MENA',
  ARRAY['https://groupe-telnet.com', 'https://www.airbus.com/en/space'], 0.65::double precision)
) AS v(title, description, severity, warning_type, region, source_urls, confidence)
WHERE NOT EXISTS (SELECT 1 FROM warnings w WHERE w.title = v.title);


-- ─── 7. INSIGHTS ────────────────────────────────────────────────────────────

INSERT INTO insights (title, summary, insight_type, region, confidence, evidence_urls, tags)
SELECT v.title, v.summary, v.insight_type, v.region, v.confidence, v.evidence_urls, v.tags
FROM (VALUES
('North Africa EMS cluster achieving critical mass',
  'With 15+ active EMS competitors in Morocco and Tunisia (Lacroix, Eolane, Adetel, ACTIA, Tronico, Coficab, Eleonetech, COFAT, Hands Corp, MGR, Mets, AAF, Siaf, Telnet, Presto), the North African electronics manufacturing cluster has reached critical mass. Shared supplier ecosystems, trained workforce pools, and logistics infrastructure create both competitive pressure and ecosystem benefits.',
  'competitive_intel', 'MENA', 0.82,
  ARRAY['https://www.fipa.tn', 'https://www.amdie.gov.ma', 'https://www.invest.gov.tn'],
  ARRAY['MENA cluster','EMS competition','Morocco','Tunisia','nearshoring']),

('EU defense electronics demand surge creates opportunity window',
  'Post-2024 European defense budgets are expanding rapidly — NATO members collectively increasing defense spending toward 2.5% GDP. This creates a 2-3 year window for EMS providers with ITAR/defense clearances to capture new orders in communications, radar, and electronic warfare subsystems.',
  'demand_signal', 'Europe', 0.78,
  ARRAY['https://www.eda.europa.eu', 'https://www.nato.int/cps/en/natohq/topics_49198.htm'],
  ARRAY['defense electronics','NATO spending','ITAR','EW subsystems']),

('Automotive EV transition reshaping EMS competitive landscape',
  'The shift to electric vehicles is restructuring EMS demand: high-voltage battery management systems, power electronics, and EV charging infrastructure require different capabilities than traditional automotive electronics. Competitors like Coficab and ACTIA are repositioning; this creates both threats and whitespace opportunities.',
  'macro_shift', 'Global', 0.75,
  ARRAY['https://www.iea.org/energy-system/transport/electric-vehicles'],
  ARRAY['EV transition','automotive electronics','power electronics','battery management']),

('Tunisia political stability risk elevated for manufacturing sector',
  'Tunisias ongoing constitutional reform process and economic challenges (IMF negotiations, foreign reserve pressure) create elevated operational risk for manufacturing investments. However, the government continues to prioritize FDI in electronics and automotive as employment generators.',
  'security_posture', 'MENA', 0.70,
  ARRAY['https://www.imf.org/en/Countries/TUN', 'https://www.fipa.tn'],
  ARRAY['Tunisia country risk','political stability','IMF Tunisia','manufacturing FDI']),

('French nearshoring trend accelerating toward Morocco and Tunisia',
  'Analysis of 23 French OEMs and tier-1 suppliers shows accelerating nearshoring of electronics assembly from China to Morocco and Tunisia. Key drivers: EU CBAM, supply chain resilience mandates, and France 2030 industrial policy. Lacroix, Eolane, Tronico, ACTIA, and Sagemcom are all expanding MENA capacity.',
  'supply_risk', 'Europe', 0.80,
  ARRAY['https://www.economie.gouv.fr/plan-de-relance/france-2030'],
  ARRAY['French nearshoring','Morocco FDI','Tunisia FDI','EU CBAM','supply chain resilience'])
) AS v(title, summary, insight_type, region, confidence, evidence_urls, tags)
WHERE NOT EXISTS (SELECT 1 FROM insights i WHERE i.title = v.title);

COMMIT;

-- Print summary
SELECT 'Companies' as entity, COUNT(*) as total FROM companies
UNION ALL SELECT 'Competitors', COUNT(*) FROM companies WHERE metadata->>'is_competitor' = 'true'
UNION ALL SELECT 'Persons', COUNT(*) FROM persons
UNION ALL SELECT 'Capabilities', COUNT(*) FROM capabilities
UNION ALL SELECT 'Certifications', COUNT(*) FROM certifications
UNION ALL SELECT 'Warnings', COUNT(*) FROM warnings
UNION ALL SELECT 'Insights', COUNT(*) FROM insights
UNION ALL SELECT 'Graph Edges', COUNT(*) FROM graph_edges;
