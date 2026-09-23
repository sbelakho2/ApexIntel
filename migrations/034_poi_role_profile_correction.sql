-- POI Role & Profile Correction Migration
-- Fixes 36/106 POIs with placeholder/generic/wrong roles and thin bios.
-- All corrections are based on verified public sources (company leadership
-- pages, SEC filings, press releases). See commit message for sources.

BEGIN;

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 1: Remove false-positive "persons" that are actually place names,
-- provinces, or broken seed records with a ROLE as the person NAME.
-- ═══════════════════════════════════════════════════════════════════════

-- First, clean up dependent records (FK constraints: poi_artifacts,
-- role_history, person_changes all reference persons.id).
DELETE FROM poi_artifacts WHERE person_id IN (
    SELECT id FROM persons WHERE name IN (
        'Nguyễn Hồng Sâm', 'Thái Nguyên', 'Tuyên Quang', 'Cao Bằng',
        'Chief Procurement Officer', 'VP Global Procurement',
        'VP Global Supply Chain', 'VP Procurement', 'VP Supply Chain Management',
        'Aioliki Aderes'
    )
);
DELETE FROM role_history WHERE person_id IN (
    SELECT id FROM persons WHERE name IN (
        'Nguyễn Hồng Sâm', 'Thái Nguyên', 'Tuyên Quang', 'Cao Bằng',
        'Chief Procurement Officer', 'VP Global Procurement',
        'VP Global Supply Chain', 'VP Procurement', 'VP Supply Chain Management',
        'Aioliki Aderes'
    )
);
DELETE FROM person_changes WHERE person_id IN (
    SELECT id FROM persons WHERE name IN (
        'Nguyễn Hồng Sâm', 'Thái Nguyên', 'Tuyên Quang', 'Cao Bằng',
        'Chief Procurement Officer', 'VP Global Procurement',
        'VP Global Supply Chain', 'VP Procurement', 'VP Supply Chain Management',
        'Aioliki Aderes'
    )
);

-- Now safe to delete the false-positive persons.
-- Vietnamese provinces/cities incorrectly added as persons
DELETE FROM persons WHERE name IN ('Nguyễn Hồng Sâm', 'Thái Nguyên', 'Tuyên Quang', 'Cao Bằng');

-- Broken seed records: role title stored as person name (no real person)
DELETE FROM persons WHERE name IN (
    'Chief Procurement Officer',
    'VP Global Procurement',
    'VP Global Supply Chain',
    'VP Procurement',
    'VP Supply Chain Management'
);

-- "Aioliki Aderes" — Greek phrase ("Αιολική Αδέρες"), not a person
DELETE FROM persons WHERE name = 'Aioliki Aderes';

-- Andrej Babis (without háček) — duplicate of Andrej Babiš. Remove the
-- non-diacritic copy that has an empty bio.
DELETE FROM poi_artifacts WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'Andrej Babis' AND (public_bio IS NULL OR public_bio = '')
);
DELETE FROM role_history WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'Andrej Babis' AND (public_bio IS NULL OR public_bio = '')
);
DELETE FROM person_changes WHERE person_id IN (
    SELECT id FROM persons WHERE name = 'Andrej Babis' AND (public_bio IS NULL OR public_bio = '')
);
DELETE FROM persons WHERE name = 'Andrej Babis' AND (public_bio IS NULL OR public_bio = '');

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 2: Update verified executives with correct titles + bios.
-- ═══════════════════════════════════════════════════════════════════════

-- ── Flex Ltd ──────────────────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'Chief Technology Officer, Health Solutions',
    role_family = 'C-Suite',
    public_bio = 'CTO and Head of Global Design and Engineering for Flex''s Health Solutions business unit. Over 25 years in MedTech leadership; featured speaker at MEDevice Silicon Valley and MD+DI Women in Medtech 2021.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.75),
    updated_at = now()
WHERE name = 'Jennifer Samproni';

UPDATE persons SET
    "current_role" = 'SVP, Value-Added Services',
    role_family = 'Operations',
    public_bio = 'Senior Vice President and Business Unit Leader for Flex Value-Added Services. Focuses on general management, strategy, business development, and end-to-end supply chain for manufacturing services. Based in Austin, TX.',
    updated_at = now()
WHERE name = 'Colin Chapman';

UPDATE persons SET
    "current_role" = 'President, Lifestyle & Consumer Devices',
    role_family = 'C-Suite',
    public_bio = 'President of Flex''s Lifestyle, Consumer Devices, and Core Industrial business groups. Seasoned global business leader in industrial and technology sectors; publicly quoted in Flex''s 2023 strategic manufacturing partnership with Husqvarna.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.75),
    updated_at = now()
WHERE name = 'Dennis Kirkpatrick';

UPDATE persons SET
    "current_role" = 'SVP, Deputy General Counsel',
    role_family = 'Legal',
    public_bio = 'Senior Vice President and Deputy General Counsel at Flex since 2015. Georgetown Law graduate; named 2021 Silicon Valley Business Journal Woman of Influence for leveraging legal AI as business strategy.',
    updated_at = now()
WHERE name = 'Heather Childress';

UPDATE persons SET
    "current_role" = 'EVP & General Counsel',
    role_family = 'Legal',
    public_bio = 'Executive Vice President and General Counsel at Flex since September 2016. LSE law graduate; previously held GC roles at Lenovo, Google, and Motorola. Oversees Flex''s global legal function.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Scott Offer';

-- ── L3Harris Technologies ─────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'Chair & Chief Executive Officer',
    role_family = 'C-Suite',
    public_bio = 'Chair and CEO of L3Harris Technologies since June 2021. Joined L3 Technologies as President and COO in 2015; became Vice Chair, President and COO of L3Harris after the merger. 30+ years in aerospace and defense.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.90),
    updated_at = now()
WHERE name = 'Christopher Kubasik';

UPDATE persons SET
    "current_role" = 'Independent Director (Board)',
    role_family = 'C-Suite',
    public_bio = 'Independent Director at L3Harris since 2001; chairs the Nominating and Governance Committee. Former Chairman, President and CEO of Cooper Tire & Rubber Company. Previously held senior roles at Dana Corporation and Cerberus Capital Management.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.65),
    updated_at = now()
WHERE name = 'Thomas Dattilo';

UPDATE persons SET
    "current_role" = 'Independent Director (Board)',
    role_family = 'C-Suite',
    public_bio = 'Retired U.S. Air Force General (four-star). Elected to L3Harris Board in February 2023; serves on the Audit Committee. Former Commander of Air Education and Training Command (AETC). Brings national-security and Asia-Pacific expertise.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.60),
    updated_at = now()
WHERE name = 'Edward Rice';

UPDATE persons SET
    "current_role" = 'Independent Director (Board)',
    role_family = 'C-Suite',
    public_bio = 'CEO of JetBlue Airways (first woman to lead a major U.S. airline, since 2024). Independent Director on the L3Harris board since May 2022, elected while JetBlue President and COO. With JetBlue since 2005.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Joanna Geraghty';

-- ── Benchmark Electronics ─────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'President & CEO',
    role_family = 'C-Suite',
    public_bio = 'President and CEO of Benchmark Electronics, effective March 2026. Previously EVP and Chief Commercial Officer from July 2023. 30+ years of industry experience.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.80),
    updated_at = now()
WHERE name = 'David Moezidis';

-- ── KATEK SE / Kontron Group ──────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'Managing Director, KATEK Grassau',
    role_family = 'C-Suite',
    public_bio = 'Managing Director (CEO) of KATEK Grassau GmbH and COO Kontron Europe. Leads KATEK operations within the Kontron Group after the 2024 Kontron AG acquisition of KATEK SE.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Walter Kroupa';

UPDATE persons SET
    "current_role" = 'Managing Director, KATEK Hungary',
    role_family = 'Operations',
    public_bio = 'Managing Director of KATEK Hungary Kft., the Hungarian subsidiary of KATEK SE / Kontron Group. Heads local manufacturing operations.',
    updated_at = now()
WHERE name = 'Csaba Sebesi';

UPDATE persons SET
    "current_role" = 'Managing Director, KATEK Czech Republic',
    role_family = 'Operations',
    public_bio = 'Managing Director of KATEK Czech Republic s.r.o. (Kontron Group). Brno University of Technology graduate. Leads Czech manufacturing operations.',
    updated_at = now()
WHERE name = 'Miroslav Zajíc';

UPDATE persons SET
    "current_role" = 'Director Sales, KATEK Group',
    role_family = 'Sales/Marketing',
    public_bio = 'Director of Sales for the KATEK Group (Kontron). Based at KATEK Grassau HQ. Also registered as authorized signatory (Prokurist) at KATEK GmbH, Grassau as of November 2024.',
    updated_at = now()
WHERE name = 'Manfred Hois';

-- ── cms electronics ───────────────────────────────────────────────────
-- Note: original data spelled "Polligger"; correct spelling is "Pollinger"
UPDATE persons SET
    "current_role" = 'Director Production & Technology',
    role_family = 'Operations',
    public_bio = 'Director of Production and Technology at CMS Electronics Group, Austria. Heads manufacturing technology and production operations. HTL Lastenstrasse graduate. Appointed during management restructuring.',
    updated_at = now()
WHERE name = 'Mario Damej';

UPDATE persons SET
    name = 'Michael Pollinger',
    "current_role" = 'Director, Plant Management & Development',
    role_family = 'Operations',
    public_bio = 'Senior executive at cms electronics responsible for plant management, development, core customer development, and marketing. Career progressed from Production Manager to CCO to current leadership role.',
    updated_at = now()
WHERE name = 'Michael Polligger';

UPDATE persons SET
    name = 'Ulrike Pollinger',
    "current_role" = 'Director Finance & Corporate Services',
    role_family = 'Finance',
    public_bio = 'Director of Finance and Corporate Services at cms electronics. Leadership philosophy: "If you want to grow your business, start it by growing your people."',
    updated_at = now()
WHERE name = 'Ulrike Polligger';

-- ── Kimball Electronics ───────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'CEO & Director',
    role_family = 'C-Suite',
    public_bio = 'Chief Executive Officer and Director of Kimball Electronics, effective March 2023. Succeeded retiring Chairman/CEO Don Charron. 20+ years of executive experience.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.80),
    updated_at = now()
WHERE name = 'Ric Phillips';

-- ── Texcel Technology ─────────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'Business Development Manager',
    role_family = 'Sales/Marketing',
    public_bio = 'Business Development Manager at Texcel Technology Plc (Dartford/Crayford, Kent, UK). Long-serving BD executive at the UK-based EMS provider.',
    updated_at = now()
WHERE name = 'Paul Dickson';

UPDATE persons SET
    "current_role" = 'Sales Director',
    role_family = 'Sales/Marketing',
    public_bio = 'Sales Director at Texcel Technology PLC. Formally appointed as statutory Director of the company in February 2024 (UK Companies House). Previously served as Business Development Manager.',
    updated_at = now()
WHERE name = 'Sarah McNamara';

-- ── Venture Corporation ───────────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'Group CEO',
    role_family = 'C-Suite',
    public_bio = 'Group Chief Executive Officer of Venture Corporation Limited, effective November 2024. Joined Venture Group in 2003; previously CEO of the Advanced Manufacturing & Design Solutions (AMDS) division and led Group IT and Global Supplier Base.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.85),
    updated_at = now()
WHERE name = 'Wong Chee Kheong';

-- ── Asteelflash / USI Group ───────────────────────────────────────────
UPDATE persons SET
    "current_role" = 'EVP, Asteelflash Asia',
    role_family = 'C-Suite',
    public_bio = 'Executive Vice President of Asteelflash Asia and Corporate SVP at Universal Scientific Industrial (USI). Dual-role leader following USI''s acquisition of Asteelflash. Based in Suzhou; California State University graduate.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.70),
    updated_at = now()
WHERE name = 'Andrew Wu';

UPDATE persons SET
    "current_role" = 'VP Americas',
    role_family = 'Operations',
    public_bio = 'Vice President, Americas at Asteelflash. Heads Asteelflash''s Americas operations; represented the company at the 2023 Nextracker/Asteelflash-USI manufacturing line opening in Fremont, CA.',
    updated_at = now()
WHERE name = 'Mat Behringer';

UPDATE persons SET
    "current_role" = 'CEO, Asteelflash',
    role_family = 'C-Suite',
    public_bio = 'Chief Executive Officer of Asteelflash, effective December 2023. Succeeded founder Gilles Benhamou. Previously EVP for WEMEA (Western Europe, Middle East, Africa) at Asteelflash.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.80),
    updated_at = now()
WHERE name = 'Nicolas Denis';

-- ── Skeleton Technologies ─────────────────────────────────────────────
-- Mark Nodder is NOT a Skeleton Technologies employee — he is a customer
-- (former Wrightbus CEO). Correct his record to reflect reality.
UPDATE persons SET
    "current_role" = 'Customer Reference (former Wrightbus CEO)',
    role_family = 'general',
    public_bio = 'Former Chairman and CEO of Wrightbus / Wrights Group (Belfast). Currently Joint Chief Executive of Makers Alliance. Appeared as a Skeleton Technologies customer reference for ultracapacitor deployments in buses, not as an employee or board member.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.40),
    updated_at = now()
WHERE name = 'Mark Nodder';

-- ── Andrej Babiš (Government of Czech Republic) ───────────────────────
UPDATE persons SET
    "current_role" = 'Former Prime Minister',
    role_family = 'director-general',
    public_bio = 'Former Prime Minister of the Czech Republic (2017–2021). Founder and owner of Agrofert conglomerate. Prominent figure in Czech politics and agribusiness.',
    influence_score = GREATEST(COALESCE(influence_score, 0), 0.80),
    updated_at = now()
WHERE name = 'Andrej Babiš';

-- ═══════════════════════════════════════════════════════════════════════
-- SECTION 3: Fix remaining generic roles that have enough context.
-- ═══════════════════════════════════════════════════════════════════════

-- Andrew Wu already handled above.

COMMIT;

-- ═══════════════════════════════════════════════════════════════════════
-- VERIFICATION QUERIES (run manually to confirm)
-- ═══════════════════════════════════════════════════════════════════════
-- SELECT count(*) AS total,
--        count(*) FILTER (WHERE "current_role" LIKE 'Discovered%') AS still_placeholder,
--        count(*) FILTER (WHERE name = "current_role") AS name_equals_role,
--        count(DISTINCT "current_role") AS distinct_roles
-- FROM persons;
