-- ApexIntel Seed Data — Real EMS/Electronics Supply Chain Companies
-- Run with: PGPASSWORD="${PGPASSWORD}" psql -h 127.0.0.1 -U apexintel -d apexintel -f seed_data.sql
--         or: PGPASSWORD="${DB_PASSWORD}" psql -h 127.0.0.1 -U apexintel -d apexintel -f seed_data.sql

BEGIN;

-- ══════════════════════════════════════════════════════════════════
-- 1. COMPANIES — Real EMS, OEM, Distributors, Component Manufacturers
-- ══════════════════════════════════════════════════════════════════

INSERT INTO companies (name, legal_name, domain, country_code, region, company_type, industry_tags, employee_estimate, revenue_estimate_usd, risk_score, threat_score, overlap_score, strategic_relevance, metadata)
VALUES
-- Tier-1 EMS Providers
('Foxconn', 'Hon Hai Precision Industry Co., Ltd.', 'foxconn.com', 'TW', 'Asia-Pacific', 'EMS', ARRAY['electronics','manufacturing','assembly','PCB'], 878000, 215000000000, 0.3, 0.85, 0.7, 0.95, '{"ticker":"2317.TW","hq":"New Taipei City"}'),
('Jabil', 'Jabil Inc.', 'jabil.com', 'US', 'North America', 'EMS', ARRAY['electronics','manufacturing','healthcare','automotive'], 260000, 34500000000, 0.25, 0.8, 0.65, 0.92, '{"ticker":"JBL","hq":"St. Petersburg, FL"}'),
('Flex Ltd', 'Flex Ltd.', 'flex.com', 'SG', 'Asia-Pacific', 'EMS', ARRAY['electronics','manufacturing','automotive','medical'], 160000, 26200000000, 0.28, 0.78, 0.6, 0.9, '{"ticker":"FLEX","hq":"Singapore"}'),
('Celestica', 'Celestica Inc.', 'celestica.com', 'CA', 'North America', 'EMS', ARRAY['electronics','aerospace','defense','telecom'], 27000, 7960000000, 0.22, 0.72, 0.55, 0.85, '{"ticker":"CLS","hq":"Toronto, ON"}'),
('Sanmina', 'Sanmina Corporation', 'sanmina.com', 'US', 'North America', 'EMS', ARRAY['electronics','defense','medical','industrial'], 35000, 8050000000, 0.2, 0.7, 0.5, 0.82, '{"ticker":"SANM","hq":"San Jose, CA"}'),
('Benchmark Electronics', 'Benchmark Electronics, Inc.', 'bench.com', 'US', 'North America', 'EMS', ARRAY['electronics','aerospace','defense','semiconductor'], 13000, 2850000000, 0.18, 0.65, 0.45, 0.78, '{"ticker":"BHE","hq":"Tempe, AZ"}'),
('Plexus Corp', 'Plexus Corp.', 'plexus.com', 'US', 'North America', 'EMS', ARRAY['electronics','healthcare','aerospace','defense'], 20000, 3770000000, 0.19, 0.68, 0.48, 0.8, '{"ticker":"PLXS","hq":"Neenah, WI"}'),
('Venture Corporation', 'Venture Corporation Limited', 'venture.com.sg', 'SG', 'Asia-Pacific', 'EMS', ARRAY['electronics','test','measurement','networking'], 12000, 3300000000, 0.15, 0.6, 0.4, 0.72, '{"ticker":"V03.SI","hq":"Singapore"}'),
('Pegatron', 'Pegatron Corporation', 'pegatroncorp.com', 'TW', 'Asia-Pacific', 'EMS', ARRAY['electronics','computing','consumer','assembly'], 200000, 45000000000, 0.32, 0.82, 0.68, 0.88, '{"ticker":"4938.TW","hq":"Taipei"}'),
('Wistron', 'Wistron Corporation', 'wistron.com', 'TW', 'Asia-Pacific', 'EMS', ARRAY['electronics','computing','servers','display'], 80000, 28000000000, 0.27, 0.75, 0.58, 0.84, '{"ticker":"3231.TW","hq":"New Taipei City"}'),

-- European EMS
('Zollner Elektronik', 'Zollner Elektronik AG', 'zollner.de', 'DE', 'Europe', 'EMS', ARRAY['electronics','automotive','medical','industrial'], 13200, 2100000000, 0.12, 0.55, 0.35, 0.65, '{"hq":"Zandt, Bavaria"}'),
('Lacroix Electronics', 'Lacroix Group SA', 'lacroix-group.com', 'FR', 'Europe', 'EMS', ARRAY['electronics','smart-city','automotive','defense'], 5200, 820000000, 0.1, 0.48, 0.3, 0.55, '{"ticker":"LACR.PA","hq":"Saint-Herblain"}'),
('Scanfil', 'Scanfil Oyj', 'scanfil.com', 'FI', 'Europe', 'EMS', ARRAY['electronics','telecom','defense','energy'], 3800, 920000000, 0.08, 0.42, 0.25, 0.5, '{"ticker":"SCANFL.HE","hq":"Sievi"}'),
('NOTE AB', 'NOTE AB', 'note-ems.com', 'SE', 'Europe', 'EMS', ARRAY['electronics','cleantech','medical','defense'], 2600, 380000000, 0.07, 0.38, 0.2, 0.45, '{"ticker":"NOTE.ST","hq":"Danderyd"}'),
('Cicor Group', 'Cicor Group AG', 'cicor.com', 'CH', 'Europe', 'EMS', ARRAY['electronics','medical','aerospace','defense'], 2400, 450000000, 0.09, 0.45, 0.28, 0.52, '{"ticker":"CICN.SW","hq":"Boudry"}'),
('Katek SE', 'Katek SE', 'katek-group.com', 'DE', 'Europe', 'EMS', ARRAY['electronics','automotive','renewable','e-mobility'], 4200, 680000000, 0.11, 0.5, 0.32, 0.58, '{"hq":"Munich"}'),
('GPV International', 'GPV International A/S', 'gpv-international.com', 'DK', 'Europe', 'EMS', ARRAY['electronics','medical','defense','industrial'], 8500, 1200000000, 0.13, 0.52, 0.33, 0.62, '{"hq":"Vejle"}'),

-- MENA / Africa EMS
('Adetel Group', 'Adetel Group SAS', 'adetel-group.com', 'TN', 'MENA', 'EMS', ARRAY['electronics','railway','defense','industrial'], 1200, 180000000, 0.06, 0.35, 0.15, 0.4, '{"hq":"Sousse, Tunisia"}'),
('Actia Group', 'Actia Group SA', 'actia.com', 'FR', 'Europe', 'EMS', ARRAY['electronics','automotive','telecom','aerospace'], 4100, 580000000, 0.1, 0.47, 0.27, 0.53, '{"ticker":"ATI.PA","hq":"Toulouse"}'),
('Telnet Holding', 'Telnet Holding SA', 'groupe-telnet.com', 'TN', 'MENA', 'EMS', ARRAY['electronics','aerospace','automotive','embedded'], 2000, 120000000, 0.05, 0.32, 0.12, 0.38, '{"hq":"Tunis, Tunisia"}'),

-- Component Distributors
('Arrow Electronics', 'Arrow Electronics, Inc.', 'arrow.com', 'US', 'North America', 'Distributor', ARRAY['distribution','semiconductors','passives','connectors'], 22300, 33100000000, 0.15, 0.5, 0.35, 0.7, '{"ticker":"ARW","hq":"Centennial, CO"}'),
('Avnet', 'Avnet, Inc.', 'avnet.com', 'US', 'North America', 'Distributor', ARRAY['distribution','semiconductors','interconnect','electromechanical'], 15400, 25700000000, 0.14, 0.48, 0.33, 0.68, '{"ticker":"AVT","hq":"Phoenix, AZ"}'),
('Mouser Electronics', 'Mouser Electronics, Inc.', 'mouser.com', 'US', 'North America', 'Distributor', ARRAY['distribution','semiconductors','passives','new-products'], 3500, 4200000000, 0.08, 0.35, 0.2, 0.55, '{"parent":"Berkshire Hathaway","hq":"Mansfield, TX"}'),
('Digi-Key', 'Digi-Key Electronics', 'digikey.com', 'US', 'North America', 'Distributor', ARRAY['distribution','semiconductors','passives','prototyping'], 5500, 5800000000, 0.09, 0.38, 0.22, 0.58, '{"hq":"Thief River Falls, MN"}'),
('RS Components', 'RS Group plc', 'rs-online.com', 'GB', 'Europe', 'Distributor', ARRAY['distribution','industrial','electronics','maintenance'], 8800, 3600000000, 0.1, 0.4, 0.25, 0.52, '{"ticker":"RS1.L","hq":"London"}'),
('Rutronik', 'Rutronik Elektronische Bauelemente GmbH', 'rutronik.com', 'DE', 'Europe', 'Distributor', ARRAY['distribution','semiconductors','displays','wireless'], 2100, 2200000000, 0.07, 0.33, 0.18, 0.48, '{"hq":"Ispringen"}'),

-- Semiconductor / Component OEMs
('Texas Instruments', 'Texas Instruments Incorporated', 'ti.com', 'US', 'North America', 'OEM', ARRAY['semiconductors','analog','embedded','processors'], 34000, 17500000000, 0.18, 0.6, 0.4, 0.75, '{"ticker":"TXN","hq":"Dallas, TX"}'),
('STMicroelectronics', 'STMicroelectronics N.V.', 'st.com', 'CH', 'Europe', 'OEM', ARRAY['semiconductors','MEMS','power','automotive'], 51000, 16100000000, 0.2, 0.62, 0.42, 0.78, '{"ticker":"STM","hq":"Geneva"}'),
('Infineon Technologies', 'Infineon Technologies AG', 'infineon.com', 'DE', 'Europe', 'OEM', ARRAY['semiconductors','power','security','automotive'], 56000, 16300000000, 0.22, 0.65, 0.45, 0.8, '{"ticker":"IFX.DE","hq":"Neubiberg"}'),
('NXP Semiconductors', 'NXP Semiconductors N.V.', 'nxp.com', 'NL', 'Europe', 'OEM', ARRAY['semiconductors','automotive','IoT','security'], 34500, 13300000000, 0.19, 0.58, 0.38, 0.73, '{"ticker":"NXPI","hq":"Eindhoven"}'),
('Renesas Electronics', 'Renesas Electronics Corporation', 'renesas.com', 'JP', 'Asia-Pacific', 'OEM', ARRAY['semiconductors','MCU','automotive','analog'], 21500, 14200000000, 0.17, 0.55, 0.35, 0.7, '{"ticker":"6723.T","hq":"Tokyo"}'),
('Microchip Technology', 'Microchip Technology Incorporated', 'microchip.com', 'US', 'North America', 'OEM', ARRAY['semiconductors','MCU','analog','memory'], 22600, 8450000000, 0.16, 0.53, 0.32, 0.68, '{"ticker":"MCHP","hq":"Chandler, AZ"}'),
('TE Connectivity', 'TE Connectivity Ltd.', 'te.com', 'CH', 'Europe', 'OEM', ARRAY['connectors','sensors','automotive','industrial'], 85000, 16000000000, 0.2, 0.6, 0.4, 0.75, '{"ticker":"TEL","hq":"Schaffhausen"}'),
('Amphenol', 'Amphenol Corporation', 'amphenol.com', 'US', 'North America', 'OEM', ARRAY['connectors','sensors','aerospace','defense'], 95000, 15200000000, 0.18, 0.58, 0.38, 0.72, '{"ticker":"APH","hq":"Wallingford, CT"}'),

-- PCB Manufacturers
('TTM Technologies', 'TTM Technologies, Inc.', 'ttm.com', 'US', 'North America', 'PCB', ARRAY['PCB','HDI','rigid-flex','aerospace'], 22000, 2300000000, 0.15, 0.52, 0.35, 0.65, '{"ticker":"TTMI","hq":"Santa Ana, CA"}'),
('AT&S', 'AT&S Austria Technologie & Systemtechnik AG', 'ats.net', 'AT', 'Europe', 'PCB', ARRAY['PCB','IC-substrate','HDI','semiconductor-packaging'], 14500, 1800000000, 0.14, 0.5, 0.32, 0.62, '{"ticker":"AUS.VI","hq":"Leoben"}'),
('Unimicron', 'Unimicron Technology Corporation', 'unimicron.com', 'TW', 'Asia-Pacific', 'PCB', ARRAY['PCB','IC-substrate','HDI','flip-chip'], 25000, 3200000000, 0.16, 0.54, 0.36, 0.67, '{"ticker":"3037.TW","hq":"Taoyuan"}'),
('Schweizer Electronic', 'Schweizer Electronic AG', 'schweizer.ag', 'DE', 'Europe', 'PCB', ARRAY['PCB','automotive','power-electronics','embedding'], 1100, 150000000, 0.06, 0.3, 0.15, 0.4, '{"ticker":"SCE.DE","hq":"Schramberg"}'),

-- Defense / Aerospace Electronics
('L3Harris Technologies', 'L3Harris Technologies, Inc.', 'l3harris.com', 'US', 'North America', 'Defense', ARRAY['defense','aerospace','communications','cybersecurity'], 50000, 19400000000, 0.35, 0.72, 0.45, 0.88, '{"ticker":"LHX","hq":"Melbourne, FL"}'),
('Thales Group', 'Thales SA', 'thalesgroup.com', 'FR', 'Europe', 'Defense', ARRAY['defense','aerospace','space','cybersecurity'], 81000, 20200000000, 0.38, 0.75, 0.48, 0.9, '{"ticker":"HO.PA","hq":"Paris"}'),
('BAE Systems', 'BAE Systems plc', 'baesystems.com', 'GB', 'Europe', 'Defense', ARRAY['defense','aerospace','electronics','cybersecurity'], 90500, 28200000000, 0.4, 0.78, 0.5, 0.92, '{"ticker":"BA.L","hq":"London"}'),
('Elbit Systems', 'Elbit Systems Ltd.', 'elbitsystems.com', 'IL', 'MENA', 'Defense', ARRAY['defense','drones','C4ISR','electro-optics'], 18500, 5850000000, 0.3, 0.68, 0.42, 0.82, '{"ticker":"ESLT","hq":"Haifa"}'),
('Rafael Advanced Defense', 'Rafael Advanced Defense Systems Ltd.', 'rafael.co.il', 'IL', 'MENA', 'Defense', ARRAY['defense','missiles','C4ISR','electronics'], 8500, 3200000000, 0.28, 0.65, 0.4, 0.78, '{"hq":"Haifa"}'),

-- Test & Measurement
('Keysight Technologies', 'Keysight Technologies, Inc.', 'keysight.com', 'US', 'North America', 'T&M', ARRAY['test','measurement','5G','semiconductors'], 15000, 5420000000, 0.12, 0.45, 0.28, 0.6, '{"ticker":"KEYS","hq":"Santa Rosa, CA"}'),
('Teledyne Technologies', 'Teledyne Technologies Incorporated', 'teledyne.com', 'US', 'North America', 'T&M', ARRAY['test','imaging','aerospace','defense'], 49500, 5660000000, 0.15, 0.5, 0.32, 0.65, '{"ticker":"TDY","hq":"Thousand Oaks, CA"}')
ON CONFLICT (domain) DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 2. SITES — Key manufacturing facilities
-- ══════════════════════════════════════════════════════════════════

INSERT INTO sites (company_id, name, address, city, country_code, region, lat, lon, site_type, capabilities, certifications, employee_estimate, free_zone)
SELECT c.id, s.name, s.addr, s.city, s.cc, s.region, s.lat, s.lon, s.stype, s.caps, s.certs, s.emp, s.fz
FROM (VALUES
  ('foxconn.com', 'Shenzhen Longhua Campus', 'Longhua District', 'Shenzhen', 'CN', 'Asia-Pacific', 22.65, 114.02, 'Factory', ARRAY['SMT','Assembly','Testing'], ARRAY['ISO 9001','ISO 14001'], 300000, NULL),
  ('foxconn.com', 'Zhengzhou iPhone City', 'Zhengzhou Airport Economy Zone', 'Zhengzhou', 'CN', 'Asia-Pacific', 34.52, 113.84, 'Factory', ARRAY['SMT','Final Assembly','Pack-out'], ARRAY['ISO 9001','IATF 16949'], 250000, 'Zhengzhou FTZ'),
  ('jabil.com', 'Penang Campus', 'Bayan Lepas FIZ', 'Penang', 'MY', 'Asia-Pacific', 5.30, 100.28, 'Factory', ARRAY['SMT','Box Build','Test'], ARRAY['ISO 9001','ISO 13485','AS9100'], 12000, 'Bayan Lepas FIZ'),
  ('jabil.com', 'St. Petersburg HQ', '10560 Dr. MLK St N', 'St. Petersburg', 'US', 'North America', 27.83, -82.63, 'HQ', ARRAY['Design','NPI','Prototyping'], ARRAY['ISO 9001'], 3500, NULL),
  ('flex.com', 'Zhuhai Factory', 'Zhuhai Hi-Tech Zone', 'Zhuhai', 'CN', 'Asia-Pacific', 22.27, 113.58, 'Factory', ARRAY['SMT','Assembly','Testing'], ARRAY['ISO 9001','ISO 14001'], 15000, NULL),
  ('celestica.com', 'Galway Plant', 'Parkmore East', 'Galway', 'IE', 'Europe', 53.28, -8.99, 'Factory', ARRAY['SMT','Box Build','Medical'], ARRAY['ISO 13485','ISO 9001'], 2500, NULL),
  ('zollner.de', 'Zandt HQ Factory', 'Manfred-Zollner-Str.', 'Zandt', 'DE', 'Europe', 49.10, 12.89, 'Factory', ARRAY['SMT','THT','Wire Harness','Box Build'], ARRAY['ISO 9001','IATF 16949','ISO 13485'], 4500, NULL),
  ('lacroix-group.com', 'Saint-Pierre-Montlimart', 'ZI La Lande', 'Saint-Pierre-Montlimart', 'FR', 'Europe', 47.13, -1.08, 'Factory', ARRAY['SMT','Conformal Coating','IoT Assembly'], ARRAY['ISO 9001','ISO 14001','EN 9100'], 800, NULL),
  ('lacroix-group.com', 'Tunis Plant', 'Zone Industrielle', 'Tunis', 'TN', 'MENA', 36.81, 10.17, 'Factory', ARRAY['SMT','Cable Assembly','Testing'], ARRAY['ISO 9001'], 600, 'Tunis FTZ'),
  ('scanfil.com', 'Sievi Factory', 'Sievi', 'Sievi', 'FI', 'Europe', 63.91, 24.51, 'Factory', ARRAY['SMT','Box Build','System Integration'], ARRAY['ISO 9001','ISO 14001'], 900, NULL),
  ('st.com', 'Crolles Fab', 'Rue Jean Monnet', 'Crolles', 'FR', 'Europe', 45.28, 5.88, 'Fab', ARRAY['300mm Wafer','FDSOI','BCD'], ARRAY['ISO 9001','IATF 16949','ISO 14001'], 4500, NULL),
  ('infineon.com', 'Dresden Fab', 'Koenigsbruecker Str.', 'Dresden', 'DE', 'Europe', 51.09, 13.72, 'Fab', ARRAY['300mm Wafer','Power Semiconductors','IGBT'], ARRAY['ISO 9001','IATF 16949'], 3200, NULL),
  ('elbitsystems.com', 'Haifa HQ', 'Advanced Technology Center', 'Haifa', 'IL', 'MENA', 32.79, 34.99, 'HQ', ARRAY['C4ISR','EO/IR','UAS'], ARRAY['ISO 9001','AS9100'], 6000, NULL),
  ('thalesgroup.com', 'Gennevilliers Site', 'Avenue du General de Gaulle', 'Gennevilliers', 'FR', 'Europe', 48.93, 2.30, 'Factory', ARRAY['Radar','Avionics','Comms'], ARRAY['ISO 9001','EN 9100'], 4000, NULL),
  ('groupe-telnet.com', 'Tunis R&D Center', 'Technopole El Ghazala', 'Ariana', 'TN', 'MENA', 36.90, 10.19, 'R&D', ARRAY['Embedded Systems','Satellite','Automotive ECU'], ARRAY['ISO 9001','CMMI Level 3'], 800, 'El Ghazala Technopark')
) AS s(dom, name, addr, city, cc, region, lat, lon, stype, caps, certs, emp, fz)
JOIN companies c ON c.domain = s.dom
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 3. CERTIFICATIONS
-- ══════════════════════════════════════════════════════════════════

INSERT INTO certifications (company_id, standard, status, issuing_body, valid_from, valid_until, scope)
SELECT c.id, v.std, 'active', v.body, v.vf::date, v.vu::date, v.scope
FROM (VALUES
  ('foxconn.com', 'ISO 9001:2015', 'SGS', '2023-06-01', '2026-05-31', 'Electronics manufacturing'),
  ('foxconn.com', 'ISO 14001:2015', 'SGS', '2023-06-01', '2026-05-31', 'Environmental management'),
  ('foxconn.com', 'IATF 16949:2016', 'TUV', '2023-01-15', '2025-12-31', 'Automotive quality'),
  ('jabil.com', 'ISO 9001:2015', 'Bureau Veritas', '2024-01-10', '2027-01-09', 'EMS global operations'),
  ('jabil.com', 'ISO 13485:2016', 'BSI', '2023-09-01', '2026-08-31', 'Medical devices'),
  ('jabil.com', 'AS9100D', 'PRI', '2024-03-15', '2027-03-14', 'Aerospace quality'),
  ('flex.com', 'ISO 9001:2015', 'DNV', '2024-02-01', '2027-01-31', 'Global manufacturing'),
  ('celestica.com', 'ISO 9001:2015', 'LRQA', '2023-11-01', '2026-10-31', 'Electronics assembly'),
  ('celestica.com', 'ISO 13485:2016', 'LRQA', '2023-11-01', '2026-10-31', 'Medical devices'),
  ('sanmina.com', 'ISO 9001:2015', 'DNV', '2024-06-01', '2027-05-31', 'Manufacturing services'),
  ('zollner.de', 'IATF 16949:2016', 'TUV', '2024-04-01', '2027-03-31', 'Automotive EMS'),
  ('zollner.de', 'ISO 13485:2016', 'TUV', '2024-04-01', '2027-03-31', 'Medical devices'),
  ('thalesgroup.com', 'ISO 27001:2022', 'BSI', '2024-01-01', '2026-12-31', 'Information security'),
  ('baesystems.com', 'AS9100D', 'LRQA', '2023-07-01', '2026-06-30', 'Aerospace & defense'),
  ('infineon.com', 'IATF 16949:2016', 'TUV', '2024-05-01', '2027-04-30', 'Automotive semiconductors'),
  ('st.com', 'ISO 9001:2015', 'AFNOR', '2024-03-01', '2027-02-28', 'Semiconductor manufacturing')
) AS v(dom, std, body, vf, vu, scope)
JOIN companies c ON c.domain = v.dom
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 4. CAPABILITIES
-- ══════════════════════════════════════════════════════════════════

INSERT INTO capabilities (company_id, capability, proof_grade, evidence_urls)
SELECT c.id, v.cap, v.grade, v.urls
FROM (VALUES
  ('foxconn.com', 'High-Volume SMT', 'Verified', ARRAY['https://foxconn.com/capabilities']),
  ('foxconn.com', 'Final Assembly & Pack-out', 'Verified', ARRAY['https://foxconn.com/services']),
  ('foxconn.com', 'Die Casting & CNC', 'Verified', ARRAY['https://foxconn.com/mechanical']),
  ('jabil.com', 'Advanced SMT (01005)', 'Verified', ARRAY['https://jabil.com/capabilities']),
  ('jabil.com', 'Medical Device Manufacturing', 'Verified', ARRAY['https://jabil.com/industries/healthcare']),
  ('jabil.com', 'Additive Manufacturing', 'Claimed', ARRAY['https://jabil.com/capabilities/additive']),
  ('flex.com', 'Automotive Electronics', 'Verified', ARRAY['https://flex.com/automotive']),
  ('flex.com', 'Industrial IoT Assembly', 'Verified', ARRAY['https://flex.com/industrial']),
  ('celestica.com', 'Aerospace Electronics', 'Verified', ARRAY['https://celestica.com/aerospace-defense']),
  ('celestica.com', 'HPS (Hardware Platform Solutions)', 'Verified', ARRAY['https://celestica.com/hps']),
  ('zollner.de', 'Through-Hole Technology', 'Verified', ARRAY['https://zollner.de/en/technologies']),
  ('zollner.de', 'Wire Harness Assembly', 'Verified', ARRAY['https://zollner.de/en/wiring']),
  ('thalesgroup.com', 'Radar Systems', 'Verified', ARRAY['https://thalesgroup.com/radar']),
  ('thalesgroup.com', 'Avionics', 'Verified', ARRAY['https://thalesgroup.com/avionics']),
  ('elbitsystems.com', 'Unmanned Aerial Systems', 'Verified', ARRAY['https://elbitsystems.com/uas']),
  ('elbitsystems.com', 'Electro-Optic Systems', 'Verified', ARRAY['https://elbitsystems.com/eo']),
  ('infineon.com', 'Power Semiconductors (IGBT/MOSFET)', 'Verified', ARRAY['https://infineon.com/power']),
  ('infineon.com', 'Automotive Microcontrollers', 'Verified', ARRAY['https://infineon.com/automotive']),
  ('groupe-telnet.com', 'Satellite Subsystems', 'Claimed', ARRAY['https://groupe-telnet.com/space']),
  ('groupe-telnet.com', 'Embedded Software', 'Verified', ARRAY['https://groupe-telnet.com/embedded'])
) AS v(dom, cap, grade, urls)
JOIN companies c ON c.domain = v.dom
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 5. PERSONS (Key Industry POIs)
-- ══════════════════════════════════════════════════════════════════

INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics)
SELECT v.name, c.id, v.role, v.fam, v.region, v.cc, v.infl, v.ds, v.rt, v.ca, v.topics
FROM (VALUES
  ('Young Liu', 'foxconn.com', 'Chairman & CEO', 'C-Suite', 'Asia-Pacific', 'TW', 0.95, 'Analytical', 'Moderate', 'High', ARRAY['AI','EV','semiconductors','India expansion']),
  ('Kenny Wilson', 'jabil.com', 'CEO', 'C-Suite', 'North America', 'US', 0.88, 'Pragmatic', 'Moderate', 'High', ARRAY['healthcare','cloud','5G','reshoring']),
  ('Revathi Advaithi', 'flex.com', 'CEO', 'C-Suite', 'Asia-Pacific', 'SG', 0.87, 'Visionary', 'High', 'High', ARRAY['circular-economy','EV','next-gen-mobility']),
  ('Rob Mionis', 'celestica.com', 'President & CEO', 'C-Suite', 'North America', 'CA', 0.82, 'Analytical', 'Moderate', 'Moderate', ARRAY['HPS','cloud','defense','nearshoring']),
  ('Jure Sola', 'sanmina.com', 'Chairman & CEO', 'C-Suite', 'North America', 'US', 0.78, 'Directive', 'Low', 'Low', ARRAY['defense','medical','optical']),
  ('Ludwig Baerlein', 'zollner.de', 'CEO', 'C-Suite', 'Europe', 'DE', 0.72, 'Consultative', 'Low', 'Moderate', ARRAY['automotive','medical','Industry-4.0']),
  ('Patrice Caine', 'thalesgroup.com', 'Chairman', 'Board', 'Europe', 'FR', 0.9, 'Visionary', 'Moderate', 'High', ARRAY['AI','cybersecurity','space','defense']),
  ('Bezhalel Machlis', 'elbitsystems.com', 'President & CEO', 'C-Suite', 'MENA', 'IL', 0.85, 'Directive', 'High', 'High', ARRAY['drones','C4ISR','autonomy','cyber']),
  ('Jochen Hanebeck', 'infineon.com', 'CEO', 'C-Suite', 'Europe', 'DE', 0.88, 'Analytical', 'Moderate', 'High', ARRAY['SiC','GaN','automotive','AI']),
  ('Jean-Marc Chery', 'st.com', 'President & CEO', 'C-Suite', 'Europe', 'CH', 0.86, 'Pragmatic', 'Moderate', 'High', ARRAY['SiC','FDSOI','automotive','industrial']),
  ('Kurt Sievers', 'nxp.com', 'President & CEO', 'C-Suite', 'Europe', 'NL', 0.84, 'Analytical', 'Moderate', 'High', ARRAY['automotive','edge-AI','security','RISC-V']),
  ('Mohamed Fouzri', 'groupe-telnet.com', 'CEO & Founder', 'C-Suite', 'MENA', 'TN', 0.65, 'Visionary', 'High', 'High', ARRAY['space','embedded','NewSpace','Africa-tech'])
) AS v(name, dom, role, fam, region, cc, infl, ds, rt, ca, topics)
JOIN companies c ON c.domain = v.dom
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 6. RECIPES — Insight detection patterns
-- ══════════════════════════════════════════════════════════════════

INSERT INTO recipes (code, name, status, definition, precision_score)
VALUES
  ('SUPPLY_CHAIN_DISRUPTION', 'Supply Chain Disruption Detection', 'active', '{"signals":["facility_closure","shipping_delay","component_shortage"],"transforms":["aggregate_by_region","severity_scoring"],"statistical_test":"mann_whitney","insight_template":"Detected supply chain disruption in {region}: {narrative}","action_template":"Review alternative suppliers in {region}. Contact procurement team.","severity":"high","category":"supply_chain"}', 0.82),
  ('COMPETITOR_EXPANSION', 'Competitor Expansion Alert', 'active', '{"signals":["new_facility","hiring_surge","certification_new"],"transforms":["entity_resolution","timeline_analysis"],"statistical_test":"chi_squared","insight_template":"{company} expanding into {region} with new {capability}","action_template":"Assess competitive impact. Brief sales team on positioning.","severity":"medium","category":"competitive"}', 0.78),
  ('CERT_EXPIRY_RISK', 'Certification Expiry Risk', 'active', '{"signals":["certification_approaching_expiry","audit_schedule"],"transforms":["days_to_expiry","severity_by_standard"],"statistical_test":"threshold","insight_template":"{company} {standard} certification expires {days_until} days. Audit status: {status}","action_template":"Verify renewal status with quality team. Prepare contingency supplier.","severity":"medium","category":"compliance"}', 0.91),
  ('HIRING_SIGNAL', 'Strategic Hiring Signal', 'active', '{"signals":["job_posting","linkedin_growth","new_role"],"transforms":["role_clustering","capability_inference"],"statistical_test":"poisson","insight_template":"{company} hiring {count} roles in {capability_area}, suggesting expansion in {market}","action_template":"Monitor capability buildout. Consider preemptive customer engagement.","severity":"low","category":"market_intelligence"}', 0.74),
  ('GEOPOLITICAL_RISK', 'Geopolitical Risk Monitor', 'active', '{"signals":["sanctions_update","trade_restriction","political_event"],"transforms":["entity_exposure_mapping","severity_scoring"],"statistical_test":"bayesian_update","insight_template":"Geopolitical risk elevated for {region}: {event}. Exposure: {entities}","action_template":"Activate supply-chain diversification plan for {region}. Brief leadership.","severity":"critical","category":"geopolitical"}', 0.85),
  ('PRICE_VOLATILITY', 'Component Price Volatility', 'active', '{"signals":["commodity_price_change","allocation_notice","lead_time_change"],"transforms":["time_series_anomaly","impact_scoring"],"statistical_test":"grubbs","insight_template":"Abnormal price movement detected for {component_class}: {pct_change}% in {period}","action_template":"Lock in pricing with distributors. Review BOM alternatives.","severity":"high","category":"procurement"}', 0.79),
  ('TECH_CONVERGENCE', 'Technology Convergence Pattern', 'active', '{"signals":["patent_filing","standard_update","product_launch"],"transforms":["technology_clustering","convergence_scoring"],"statistical_test":"jaccard_similarity","insight_template":"Convergence detected: {tech_a} and {tech_b} across {count} companies","action_template":"Brief R&D on convergence opportunity. Assess capability gap.","severity":"low","category":"technology"}', 0.71),
  ('POI_ROLE_CHANGE', 'Person of Interest Role Change', 'active', '{"signals":["role_change","company_change","public_statement"],"transforms":["relationship_impact","influence_recalc"],"statistical_test":"threshold","insight_template":"{person} moved from {old_role} to {new_role} at {company}. Impact: {assessment}","action_template":"Update engagement strategy. Schedule introduce-meeting if new contact.","severity":"medium","category":"relationship"}', 0.88)
ON CONFLICT (code) DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 7. WARNINGS — Seed initial intelligence warnings
-- ══════════════════════════════════════════════════════════════════

INSERT INTO warnings (recipe_code, warning_type, title, description, severity, region, source_urls, entity_ids, confidence, ts_utc)
SELECT
  v.recipe, v.wtype, v.title, v.descr, v.sev, v.region, v.urls,
  ARRAY(SELECT id FROM companies WHERE domain = ANY(v.company_domains) LIMIT 3),
  v.conf, v.ts::timestamptz
FROM (VALUES
  ('SUPPLY_CHAIN_DISRUPTION', 'supply_chain', 'TSMC Advanced Packaging Capacity Constraints', 'CoWoS advanced packaging capacity at TSMC remains constrained through Q3 2026. Major impact on AI accelerator supply chain. Jabil, Foxconn, and Flex affected through their server/datacenter lines.', 'critical', 'Asia-Pacific', ARRAY['https://digitimes.com','https://reuters.com/technology'], ARRAY['foxconn.com','jabil.com','flex.com'], 0.89, '2026-02-25T08:30:00Z'),
  ('COMPETITOR_EXPANSION', 'competitive', 'Foxconn Expands India Operations — New Chennai Facility', 'Foxconn announced 500M USD investment in new manufacturing campus near Chennai, targeting iPhone and server assembly. Expected operational Q1 2027. Reduces China concentration risk.', 'high', 'Asia-Pacific', ARRAY['https://economictimes.indiatimes.com','https://foxconn.com/news'], ARRAY['foxconn.com'], 0.92, '2026-02-24T14:15:00Z'),
  ('GEOPOLITICAL_RISK', 'geopolitical', 'EU Critical Raw Materials Act — New Compliance Requirements', 'EU Critical Raw Materials Act effective March 2026 introduces supply-chain due-diligence requirements for electronics manufacturers operating in EU. Affects rare-earth dependent components and battery materials.', 'high', 'Europe', ARRAY['https://ec.europa.eu/crma','https://reuters.com/business'], ARRAY['infineon.com','st.com','thalesgroup.com'], 0.85, '2026-02-23T10:00:00Z'),
  ('CERT_EXPIRY_RISK', 'compliance', 'Foxconn IATF 16949 Certification Approaching Renewal', 'Foxconn automotive quality certification IATF 16949:2016 expires December 2025. Renewal audit scheduled but no confirmation of pass. Impacts automotive EMS contracts.', 'medium', 'Asia-Pacific', ARRAY['https://foxconn.com/quality'], ARRAY['foxconn.com'], 0.78, '2026-02-22T09:00:00Z'),
  ('HIRING_SIGNAL', 'market_intelligence', 'Celestica Aggressive AI/HPC Hiring in Toronto', 'Celestica posted 45+ engineering roles in Toronto focused on AI server design, HPC thermal management, and liquid cooling. Signals major investment in hyperscaler hardware platform solutions.', 'medium', 'North America', ARRAY['https://celestica.com/careers','https://linkedin.com'], ARRAY['celestica.com'], 0.81, '2026-02-21T16:30:00Z'),
  ('PRICE_VOLATILITY', 'procurement', 'MLCC Prices Rising — 15% Increase Across Automotive Grade', 'Automotive-grade MLCC (Multi-Layer Ceramic Capacitor) prices increased 15% in Q1 2026 due to capacity reallocation toward AI server demand. Murata and TDK leading price increases. 26-week lead times.', 'high', 'Asia-Pacific', ARRAY['https://digitimes.com','https://passivecomponentindustry.com'], ARRAY['foxconn.com','jabil.com','flex.com'], 0.87, '2026-02-20T11:00:00Z'),
  ('SUPPLY_CHAIN_DISRUPTION', 'supply_chain', 'Red Sea Shipping Disruptions Impact European EMS Supply', 'Continued Houthi attacks on Red Sea shipping routes causing 2-3 week delays for components shipped Asia-to-Europe. Rerouting via Cape of Good Hope adds 8-12 days transit. Affects European EMS facilities.', 'high', 'Europe', ARRAY['https://reuters.com','https://ft.com/logistics'], ARRAY['zollner.de','lacroix-group.com','scanfil.com'], 0.91, '2026-02-19T07:45:00Z'),
  ('TECH_CONVERGENCE', 'technology', 'GaN Power + AI Edge Inference Convergence Emerging', 'Multiple patent filings and product launches indicate convergence of GaN power semiconductor technology with edge AI inference. Infineon, STMicro, and TI all active. New application space for EMS.', 'low', 'Europe', ARRAY['https://infineon.com/gan','https://ti.com/power'], ARRAY['infineon.com','st.com'], 0.72, '2026-02-18T13:20:00Z'),
  ('GEOPOLITICAL_RISK', 'geopolitical', 'US CHIPS Act Guardrails — Impact on Taiwan Fabs', 'Updated CHIPS Act guardrails restrict CHIPS-funded companies from expanding advanced node capacity in countries of concern. TSMC, Samsung, and Intel affected. Potential supply rebalancing toward US/EU.', 'critical', 'North America', ARRAY['https://commerce.gov/chips','https://semiconductors.org'], ARRAY['foxconn.com','pegatroncorp.com','wistron.com'], 0.88, '2026-02-17T15:00:00Z'),
  ('POI_ROLE_CHANGE', 'relationship', 'New CTO Appointment at Jabil — AI Background', 'Jabil appointed Dr. Sarah Chen as CTO, formerly VP of AI/ML at Google Cloud. Signals strategic pivot toward AI-driven manufacturing optimization and smart factory capabilities.', 'medium', 'North America', ARRAY['https://jabil.com/news','https://linkedin.com'], ARRAY['jabil.com'], 0.83, '2026-02-16T10:30:00Z'),
  ('COMPETITOR_EXPANSION', 'competitive', 'Flex Opens New EV Battery Module Line in Hungary', 'Flex inaugurated dedicated EV battery module assembly line at Zalaegerszeg, Hungary facility. 200M EUR investment. Targeting European OEM contracts for next-gen battery packs.', 'medium', 'Europe', ARRAY['https://flex.com/news','https://reuters.com/autos'], ARRAY['flex.com'], 0.86, '2026-02-15T12:00:00Z'),
  ('SUPPLY_CHAIN_DISRUPTION', 'supply_chain', 'Taiwan Earthquake M6.2 — Semiconductor Fab Operations Monitoring', 'Magnitude 6.2 earthquake struck eastern Taiwan. No immediate damage reported at TSMC/UMC fabs in Hsinchu/Tainan. Monitoring for aftershock impacts on wafer production yield.', 'high', 'Asia-Pacific', ARRAY['https://reuters.com','https://cwb.gov.tw'], ARRAY['foxconn.com','pegatroncorp.com','unimicron.com'], 0.76, '2026-02-14T02:30:00Z')
) AS v(recipe, wtype, title, descr, sev, region, urls, company_domains, conf, ts)
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 8. INSIGHTS — Derived strategic insights
-- ══════════════════════════════════════════════════════════════════

INSERT INTO insights (title, summary, insight_type, region, confidence, evidence_urls, tags)
VALUES
  ('Asia-Pacific EMS Capacity Shift Toward India', 'Foxconn, Tata Electronics, and Pegatron collectively investing >2B USD in Indian manufacturing facilities. Analysis indicates 15-20% of consumer electronics assembly will shift from China to India by 2028. Key driver: Apple supplier diversification requirements.', 'strategic', 'Asia-Pacific', 0.87, ARRAY['https://economictimes.indiatimes.com','https://foxconn.com/news','https://reuters.com/technology'], ARRAY['reshoring','India','consumer-electronics','supply-chain-diversification']),
  ('European EMS Consolidation Wave Accelerating', 'M&A activity in European EMS sector up 40% YoY. Katek, GPV, and Cicor all made acquisitions in 2025. Driver: scale requirements for automotive/defense programs and supply-chain sovereignty mandates under EU CRMA.', 'strategic', 'Europe', 0.82, ARRAY['https://reuters.com/business','https://ft.com/european-industry'], ARRAY['M&A','consolidation','automotive','defense','CRMA']),
  ('SiC Power Semiconductor Supply-Demand Imbalance', 'Global SiC wafer supply meeting only 65% of projected 2026 demand. STMicro, Infineon, and Wolfspeed expanding capacity but lead times remain 30+ weeks. Price premium over silicon: 3-5x. Critical for EV and industrial sectors.', 'market', 'Europe', 0.9, ARRAY['https://infineon.com/sic','https://st.com/sic-mosfet','https://semianalysis.com'], ARRAY['SiC','power-semiconductors','EV','supply-gap']),
  ('Defense Electronics Budget Surge — NATO Member States', 'NATO members collectively increasing defense electronics procurement by 25% in 2026. Focus areas: secure communications, radar modernization, drone countermeasures. Thales, BAE Systems, and Elbit Systems primary beneficiaries.', 'market', 'Europe', 0.88, ARRAY['https://nato.int/budget','https://janes.com/defense-budgets'], ARRAY['defense','NATO','radar','communications','procurement']),
  ('MENA EMS Ecosystem Emerging — Tunisia as Electronics Hub', 'Tunisia positioning as nearshore EMS hub for European OEMs. Adetel, Telnet Holding, and Lacroix Tunis facility growing 20%+ annually. Advantages: proximity to EU, French/Arabic bilingual workforce, free-trade zones.', 'strategic', 'MENA', 0.75, ARRAY['https://tunisiaindustry.nat.tn','https://lacroix-group.com/tunisia'], ARRAY['Tunisia','nearshoring','EMS','free-trade-zone','MENA']),
  ('AI Server Demand Driving PCB Technology Shift', 'AI server motherboards require advanced HDI PCB with 20+ layers, fine-pitch BGA, and enhanced thermal management. TTM, Unimicron, and AT&S investing in next-gen substrate capacity. EMS providers need to upgrade SMT capabilities for 01005 and 008004 components.', 'technology', 'Asia-Pacific', 0.84, ARRAY['https://ttm.com/technology','https://unimicron.com/products','https://ats.net/technology'], ARRAY['AI-servers','PCB','HDI','substrate','advanced-packaging']),
  ('Automotive ADAS Sensor Fusion Creating New EMS Opportunity', 'Convergence of radar, LiDAR, camera, and ultrasonic sensors for L3+ autonomous driving creates complex multi-board assembly challenge. EMS providers with Class-3 IPC and IATF 16949 positioned to capture 8B USD ADAS assembly market by 2028.', 'technology', 'North America', 0.8, ARRAY['https://keysight.com/adas','https://nxp.com/automotive','https://infineon.com/adas'], ARRAY['ADAS','sensor-fusion','automotive','L3-autonomy']),
  ('Cybersecurity Compliance Driving Electronics Redesign', 'EU Cyber Resilience Act and US NIST CSF 2.0 requiring security-by-design in connected products. Electronics manufacturers must implement secure boot, hardware root-of-trust, and SBOM tracking. Timeline: compliance required by Q4 2026 for EU market.', 'regulatory', 'Europe', 0.86, ARRAY['https://ec.europa.eu/cyber-resilience','https://nist.gov/csf'], ARRAY['cybersecurity','CRA','NIST','secure-boot','compliance'])
ON CONFLICT DO NOTHING;


-- ══════════════════════════════════════════════════════════════════
-- 9. GRAPH EDGES — Key relationships
-- ══════════════════════════════════════════════════════════════════

-- Company-Person edges
INSERT INTO graph_edges (source_id, source_type, target_id, target_type, edge_type, weight, confidence)
SELECT c.id, 'company', p.id, 'person', 'CompanyPerson', 1.0, 0.95
FROM companies c
JOIN persons p ON p.primary_org_id = c.id
ON CONFLICT DO NOTHING;

-- Company-Company competitive edges (EMS competitors)
INSERT INTO graph_edges (source_id, source_type, target_id, target_type, edge_type, weight, confidence)
SELECT c1.id, 'company', c2.id, 'company', 'CompanyCompany', 0.8, 0.85
FROM companies c1, companies c2
WHERE c1.company_type = 'EMS' AND c2.company_type = 'EMS'
  AND c1.id < c2.id
  AND c1.region = c2.region
ON CONFLICT DO NOTHING;

-- Observations — recent web-change signals
INSERT INTO observations (observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
SELECT 'WebChange', c.id, 'company', v.ts::timestamptz, v.val::jsonb, v.prov::jsonb, v.conf
FROM (VALUES
  ('foxconn.com', '2026-02-26T04:00:00Z', '{"url":"https://foxconn.com/about","change_type":"content_update","diff_pct":0.12}', '{"url":"https://foxconn.com/about","fetch_ts":"2026-02-26T04:00:00Z","content_hash":"abc123","extractor_version":"1.0"}', 0.95),
  ('jabil.com', '2026-02-25T10:00:00Z', '{"url":"https://jabil.com/news","change_type":"new_page","diff_pct":0.85}', '{"url":"https://jabil.com/news","fetch_ts":"2026-02-25T10:00:00Z","content_hash":"def456","extractor_version":"1.0"}', 0.92),
  ('flex.com', '2026-02-24T15:30:00Z', '{"url":"https://flex.com/press-releases","change_type":"content_update","diff_pct":0.08}', '{"url":"https://flex.com/press-releases","fetch_ts":"2026-02-24T15:30:00Z","content_hash":"ghi789","extractor_version":"1.0"}', 0.88),
  ('celestica.com', '2026-02-23T09:15:00Z', '{"url":"https://celestica.com/investors","change_type":"content_update","diff_pct":0.15}', '{"url":"https://celestica.com/investors","fetch_ts":"2026-02-23T09:15:00Z","content_hash":"jkl012","extractor_version":"1.0"}', 0.9),
  ('infineon.com', '2026-02-22T12:00:00Z', '{"url":"https://infineon.com/products/new","change_type":"new_page","diff_pct":0.92}', '{"url":"https://infineon.com/products/new","fetch_ts":"2026-02-22T12:00:00Z","content_hash":"mno345","extractor_version":"1.0"}', 0.93),
  ('thalesgroup.com', '2026-02-21T08:30:00Z', '{"url":"https://thalesgroup.com/press-releases","change_type":"content_update","diff_pct":0.2}', '{"url":"https://thalesgroup.com/press-releases","fetch_ts":"2026-02-21T08:30:00Z","content_hash":"pqr678","extractor_version":"1.0"}', 0.87),
  ('zollner.de', '2026-02-20T14:00:00Z', '{"url":"https://zollner.de/en/news","change_type":"content_update","diff_pct":0.05}', '{"url":"https://zollner.de/en/news","fetch_ts":"2026-02-20T14:00:00Z","content_hash":"stu901","extractor_version":"1.0"}', 0.82),
  ('elbitsystems.com', '2026-02-19T16:45:00Z', '{"url":"https://elbitsystems.com/media-room","change_type":"new_page","diff_pct":0.7}', '{"url":"https://elbitsystems.com/media-room","fetch_ts":"2026-02-19T16:45:00Z","content_hash":"vwx234","extractor_version":"1.0"}', 0.91)
) AS v(dom, ts, val, prov, conf)
JOIN companies c ON c.domain = v.dom
ON CONFLICT DO NOTHING;

COMMIT;
