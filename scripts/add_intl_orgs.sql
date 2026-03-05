-- Add EU and international organizations
INSERT INTO companies (name, company_type, region, country_code) VALUES
('European Commission', 'Government', 'Europe', 'EU'),
('European Defence Agency', 'Government', 'Europe', 'EU'),
('European Investment Bank', 'Government', 'Europe', 'EU'),
('EBRD', 'Government', 'Europe', 'EU'),
('Bank Al-Maghrib', 'Government', 'MENA', 'MA'),
('NATO DIANA', 'Government', 'Europe', 'EU'),
('Bpifrance', 'Government', 'Europe', 'FR'),
('FIPA Tunisia', 'Government', 'MENA', 'TN'),
('Tunisia Investment Authority', 'Government', 'MENA', 'TN'),
('CEPEX Tunisia', 'Government', 'MENA', 'TN'),
('Conect Tunisia', 'Trade_Association', 'MENA', 'TN'),
('UTICA Tunisia', 'Trade_Association', 'MENA', 'TN'),
('AMDIE Morocco', 'Government', 'MENA', 'MA'),
('TMSA Morocco', 'Government', 'MENA', 'MA'),
('CGEM Morocco', 'Trade_Association', 'MENA', 'MA'),
('AMICA Morocco', 'Trade_Association', 'MENA', 'MA')
ON CONFLICT (name) DO NOTHING;

-- Link EU officials to correct organizations
UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'European Commission')
WHERE current_role ILIKE '%European Commission%' OR current_role ILIKE '%DG GROW%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'European Defence Agency')
WHERE current_role ILIKE '%European Defence Agency%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'European Investment Bank')
WHERE current_role ILIKE '%European Investment Bank%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'EBRD')
WHERE current_role ILIKE '%EBRD%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'Bank Al-Maghrib')
WHERE current_role ILIKE '%Bank Al-Maghrib%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'NATO DIANA')
WHERE current_role ILIKE '%NATO DIANA%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'Bpifrance')
WHERE current_role ILIKE '%Bpifrance%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'FIPA Tunisia')
WHERE current_role ILIKE '%FIPA%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'Tunisia Investment Authority')
WHERE current_role ILIKE '%Tunisia Investment Authority%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'CEPEX Tunisia')
WHERE current_role ILIKE '%CEPEX%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'Conect Tunisia')
WHERE current_role ILIKE '%President of Conect%' OR full_name ILIKE '%Conect%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'UTICA Tunisia')
WHERE current_role ILIKE '%UTICA%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'AMDIE Morocco')
WHERE current_role ILIKE '%AMDIE%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'TMSA Morocco')
WHERE current_role ILIKE '%TMSA%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'CGEM Morocco')
WHERE current_role ILIKE '%CGEM%';

UPDATE persons SET primary_org_id = (SELECT id FROM companies WHERE name = 'AMICA Morocco')
WHERE current_role ILIKE '%AMICA%';
