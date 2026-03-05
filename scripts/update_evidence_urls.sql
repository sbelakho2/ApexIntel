-- Update insights evidence_urls with better, more specific URLs

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.fipa.tn/en/why-tunisia/leading-sectors/',
  'https://www.amdie.gov.ma/en/sectors/automotive/',
  'https://investintunisia.tn/en/sectors/electronics'
] WHERE title LIKE '%North Africa EMS%';

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.eda.europa.eu/what-we-do/our-current-priorities/defence-spending-in-europe',
  'https://www.nato.int/cps/en/natohq/topics_49198.htm'
] WHERE title LIKE '%EU defense%';

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.iea.org/reports/global-ev-outlook-2024'
] WHERE title LIKE '%Automotive EV%';

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.imf.org/en/Countries/TUN',
  'https://www.fipa.tn/en/investment-climate/'
] WHERE title LIKE '%Tunisia political%';

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.economie.gouv.fr/plan-de-relance/france-2030',
  'https://lacroix.group/en/news/',
  'https://www.actia.com/en/actia-industry/'
] WHERE title LIKE '%French nearshoring%';

-- Update more generic URLs
UPDATE insights SET evidence_urls = ARRAY[
  'https://economictimes.indiatimes.com/industry/cons-products/electronics',
  'https://www.foxconn.com/en-us/press-center/press-releases',
  'https://www.reuters.com/technology/'
] WHERE evidence_urls @> ARRAY['https://economictimes.indiatimes.com'];

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.reuters.com/markets/deals/',
  'https://www.ft.com/european-industry'
] WHERE evidence_urls @> ARRAY['https://reuters.com/business'];

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.infineon.com/cms/en/product/power/mosfet/silicon-carbide/',
  'https://www.st.com/en/power-transistors/sic-mosfets.html',
  'https://semianalysis.com/'
] WHERE evidence_urls @> ARRAY['https://infineon.com/sic'];

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.nato.int/cps/en/natohq/topics_49198.htm',
  'https://www.janes.com/defence-news'
] WHERE evidence_urls @> ARRAY['https://nato.int/budget'];

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.tunisieindustrie.nat.tn/en/',
  'https://lacroix.group/en/sites/tunisia/'
] WHERE evidence_urls @> ARRAY['https://tunisiaindustry.nat.tn'];

UPDATE insights SET evidence_urls = ARRAY[
  'https://www.infineon.com/cms/en/product/power/mosfet/automotive-mosfet/',
  'https://www.keysight.com/us/en/solutions/automotive.html'
] WHERE evidence_urls @> ARRAY['https://infineon.com/adas'];

-- Update warnings source_urls with better URLs
UPDATE warnings SET source_urls = ARRAY[
  'https://incapcorp.com/investors/reports-and-presentations/'
] WHERE source_urls @> ARRAY['https://kimballelectronics.com/investors'];

UPDATE warnings SET source_urls = ARRAY[
  'https://www.coficab.com/news/'
] WHERE title LIKE '%Coficab%';

UPDATE warnings SET source_urls = ARRAY[
  'https://www.actia.com/en/news/'
] WHERE title LIKE '%ACTIA%';

UPDATE warnings SET source_urls = ARRAY[
  'https://www.mcinet.gov.ma/en/Pages/default.aspx'
] WHERE title LIKE '%Morocco%free zone%';

UPDATE warnings SET source_urls = ARRAY[
  'https://digital-strategy.ec.europa.eu/en/policies/european-chips-act'
] WHERE title LIKE '%EU Chips Act%';

UPDATE warnings SET source_urls = ARRAY[
  'https://www.telnetholding.tn/en/',
  'https://www.airbus.com/en/products-services/space'
] WHERE title LIKE '%Telnet%Airbus%';

-- Verify updates
SELECT 'Insights updated:' as info, COUNT(*) as count FROM insights;
SELECT 'Warnings updated:' as info, COUNT(*) as count FROM warnings;
