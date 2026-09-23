-- POI Bio Enrichment & Title Correction Migration (Round 2)
-- Fixes mislabeled titles discovered during OSINT verification, enriches
-- placeholder bios with researched profiles, and removes remaining false
-- positives. All corrections sourced from company leadership pages, SEC
-- filings, and official press releases.

BEGIN;

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 1: Remove remaining false positives and duplicates
-- ═══════════════════════════════════════════════════════════════════════

-- "West Bakr Lekela" — Egyptian wind farm project (Lekela Power), NOT a person
DELETE FROM poi_artifacts WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'West Bakr Lekela'
);
DELETE FROM role_history WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'West Bakr Lekela'
);
DELETE FROM person_changes WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'West Bakr Lekela'
);
DELETE FROM persons WHERE name = 'West Bakr Lekela';

-- "David A. Moezidis" — duplicate of "David Moezidis" (already corrected)
DELETE FROM poi_artifacts WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'David A. Moezidis'
);
DELETE FROM role_history WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'David A. Moezidis'
);
DELETE FROM person_changes WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'David A. Moezidis'
);
DELETE FROM persons WHERE name = 'David A. Moezidis';

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 2: Correct MISLABELED titles (verified wrong in seed data)
-- ═══════════════════════════════════════════════════════════════════════

-- Jure Sola: was "VP, Global Supply Chain" → actually Chairman & CEO
UPDATE persons SET
    "current_role" = 'Chairman & CEO',
    role_family = 'C-Suite',
    public_bio = 'Co-founder of Sanmina (1980). Has served as Chairman and CEO since 1991, leading the company through its 1993 IPO to become one of the world''s largest EMS providers. One of the longest-serving CEOs in the electronics manufacturing industry.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.95),
    updated_at = now()
WHERE name = 'Jure Sola';

-- Alan Reid: was "Chief Financial Officer" → actually EVP, Global HR
UPDATE persons SET
    "current_role" = 'EVP, Global Human Resources',
    role_family = 'general',
    public_bio = 'Executive Vice President, Global Human Resources at Sanmina. With Sanmina since 2001 in senior HR roles. Leads global talent, workforce strategy, and organizational development.',
    updated_at = now()
WHERE name = 'Alan Reid' AND primary_org_id IN (SELECT id FROM companies WHERE name = 'Sanmina');

-- Christoph Feddersen: was "VP, Global Supply Chain" → actually SVP, General Counsel
UPDATE persons SET
    "current_role" = 'SVP, General Counsel & Secretary',
    role_family = 'Legal',
    public_bio = 'Senior Vice President, General Counsel & Secretary at L3Harris Technologies since August 2024. Oversees all legal, ethics, compliance, and global trade matters; reports directly to CEO Christopher Kubasik. Previously VP & General Counsel for L3Harris''s Space and Airborne Systems segment.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Christoph Feddersen';

-- Tania Hanna: was "Director, Global Procurement" → actually VP, Government Relations
UPDATE persons SET
    "current_role" = 'VP, Government & Customer Relations',
    role_family = 'general',
    public_bio = 'Vice President, Government & Customer Relations at L3Harris Technologies. Leads U.S. government relations and is a registered lobbyist. Listed on the official L3Harris leadership page.',
    updated_at = now()
WHERE name = 'Tania Hanna';

-- Nicole Nagy: was "Director of Procurement" → actually Managing Director/CFO
UPDATE persons SET
    "current_role" = 'Managing Director & CFO, KATEK',
    role_family = 'Finance',
    public_bio = 'Managing Director and CFO of KATEK GmbH (Kontron Group). Finance executive (TU Wien; ex-Raiffeisen Bank International). Listed as a Corporate Officer/Principal of Kontron AG.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.65),
    updated_at = now()
WHERE name = 'Nicole Nagy';

-- Teija Kettunen: was "Director General" → actually Project Leader, Communications
UPDATE persons SET
    "current_role" = 'Project Leader, Global Corporate Communication',
    role_family = 'general',
    public_bio = 'Project Leader for Global Corporate Communication at HANZA Group. Serves as HANZA''s official press/media contact. Communications professional working with large Finnish industrial firms since 2000.',
    updated_at = now()
WHERE name = 'Teija Kettunen';

-- Jeroen Tuik: was "Director-General" → actually CEO & Board Director
UPDATE persons SET
    "current_role" = 'CEO & Board Director',
    role_family = 'C-Suite',
    public_bio = 'Chief Executive Officer and Board Director of Connect Group N.V. (European EMS, ~2,700 employees) since May 2016. Based in Enschede, Netherlands; prior roles at Enovates, IPTE, and RoodMicrotec.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.75),
    updated_at = now()
WHERE name = 'Jeroen Tuik';

-- Andy McLeod: was "Chief Executive Officer" → formal title is Managing Director
UPDATE persons SET
    "current_role" = 'Managing Director',
    role_family = 'C-Suite',
    public_bio = 'Managing Director of Texcel Technology plc (Crayford/Dartford, Kent, UK). Heads the UK-based EMS provider, in business 40+ years. UK Companies House registered Director.',
    updated_at = now()
WHERE name = 'Andy McLeod';

-- Sven Skjellet: was "COO" → now CSO (Chief Strategy Officer) since July 2024
UPDATE persons SET
    "current_role" = 'Chief Strategy Officer',
    role_family = 'C-Suite',
    public_bio = 'Chief Strategy Officer at cms electronics since July 2024. Leads Strategic Development & Corporate Services including New Business Development. Previously served as COO. Legal representative of cms electronics'' Asian subsidiaries.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Sven Skjellet';

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 3: Enrich placeholder bios with verified profiles
-- ═══════════════════════════════════════════════════════════════════════

-- Michael Velmeden — CEO of cms electronics
UPDATE persons SET
    "current_role" = 'CEO (Geschäftsführer)',
    role_family = 'C-Suite',
    public_bio = 'CEO (Geschäftsführer) of cms electronics, headquartered in Klagenfurt, Austria. Oversees global EMS operations including Austrian headquarters and German subsidiary (Freiburg). Active voice in the electronics manufacturing services industry.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.75),
    updated_at = now()
WHERE name = 'Michael Velmeden';

-- Bryan Schumaker — Benchmark EVP & CFO
UPDATE persons SET
    public_bio = 'Executive Vice President, Chief Financial Officer and Principal Accounting Officer at Benchmark Electronics since October 2024. 20+ years of financial leadership across public and private companies. Oversees Corporate Accounting, Internal Audit, IR, Regional Finance, Tax and Treasury.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Bryan Schumaker';

-- Josh Hollin — Benchmark SVP & CTO
UPDATE persons SET
    public_bio = 'Senior Vice President and Chief Technology Officer at Benchmark Electronics since January 2026. 25+ years in engineering leadership, automation, and advanced manufacturing. Previously held engineering leadership roles at GoPro and AMP.',
    updated_at = now()
WHERE name = 'Josh Hollin';

-- Dave Clark — Benchmark SVP & CPO
UPDATE persons SET
    public_bio = 'Senior Vice President and Chief Procurement Officer at Benchmark Electronics since 2021. Leads all aspects of global supply chain management. Previously CPO for a PE-backed electronics manufacturer with China/SE Asia operations, and at Alvarez & Marsal (2018-2021).',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.65),
    updated_at = now()
WHERE name = 'Dave Clark';

-- David Cummings — Benchmark SVP & CCO
UPDATE persons SET
    public_bio = 'Senior Vice President and Chief Commercial Officer at Benchmark Electronics since December 2025 (succeeding David Moezidis who became CEO). 20+ years in global commercial strategy, customer management, and supply chain transformation.',
    updated_at = now()
WHERE name = 'David Cummings';

-- Bipin Jayaraj — Benchmark SVP & CDIO
UPDATE persons SET
    public_bio = 'Senior Vice President & Chief Digital and Information Officer at Benchmark Electronics. Leads enterprise digital transformation, data analytics, and advanced digital technology deployment. Previously Global VP/CIO at Rogers Corporation. Board member, Arizona Technology Council.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.65),
    updated_at = now()
WHERE name = 'Bipin Jayaraj';

-- Rhonda Buseman — Benchmark SVP & CHRO
UPDATE persons SET
    "current_role" = 'SVP & Chief Human Resources Officer',
    role_family = 'general',
    public_bio = 'Senior Vice President and Chief Human Resources Officer at Benchmark Electronics. Leads talent management, culture transformation, and workforce diversity across Benchmark''s global organization.',
    updated_at = now()
WHERE name = 'Rhonda Buseman';

-- Jon Faust — Sanmina EVP & CFO
UPDATE persons SET
    public_bio = 'Executive Vice President and Chief Financial Officer at Sanmina since December 2023. ~25 years in finance, accounting, controls, and operations. Spent 19+ years at Hewlett Packard Enterprise, most recently as SVP and CFO of HPE''s Hybrid Cloud business.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Jon Faust';

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 4: Fix Skeleton Technologies partner records
-- These are real people but PARTNERS (customers/distributors), not employees.
-- ═══════════════════════════════════════════════════════════════════════
UPDATE persons SET
    "current_role" = 'Partner: MD, MJR Power & Automation',
    role_family = 'general',
    public_bio = 'Managing Director of MJR Power & Automation, a Skeleton Technologies customer/partner. Quoted on Skeleton''s site praising ultracapacitor performance for fuel savings and reduced maintenance. Not a Skeleton Technologies employee.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.30),
    updated_at = now()
WHERE name = 'Paul Cairns';

UPDATE persons SET
    "current_role" = 'Partner: CEO, DIMAC RED',
    role_family = 'general',
    public_bio = 'Chief Executive Officer of DIMAC RED S.p.A., an Italian distribution partner of Skeleton Technologies. Signed a contract to scale up ultracapacitor distribution. Based in Lombardia, Italy; Politecnico di Milano graduate.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.30),
    updated_at = now()
WHERE name = 'Valter Arosio';

COMMIT;
