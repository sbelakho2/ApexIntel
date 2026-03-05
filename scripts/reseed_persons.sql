-- Reseed proper POI data with real executives
-- This replaces the garbage TED series names with actual industry leaders

-- First get company IDs we need
DO $$
DECLARE
    foxconn_id UUID;
    jabil_id UUID;
    flex_id UUID;
    celestica_id UUID;
    sanmina_id UUID;
    zollner_id UUID;
    thales_id UUID;
    elbit_id UUID;
    infineon_id UUID;
    st_id UUID;
    nxp_id UUID;
    telnet_id UUID;
    microchip_id UUID;
BEGIN
    SELECT id INTO foxconn_id FROM companies WHERE domain = 'foxconn.com' LIMIT 1;
    SELECT id INTO jabil_id FROM companies WHERE domain = 'jabil.com' LIMIT 1;
    SELECT id INTO flex_id FROM companies WHERE domain = 'flex.com' LIMIT 1;
    SELECT id INTO celestica_id FROM companies WHERE domain = 'celestica.com' LIMIT 1;
    SELECT id INTO sanmina_id FROM companies WHERE domain = 'sanmina.com' LIMIT 1;
    SELECT id INTO zollner_id FROM companies WHERE domain = 'zollner.de' LIMIT 1;
    SELECT id INTO thales_id FROM companies WHERE domain = 'thalesgroup.com' LIMIT 1;
    SELECT id INTO elbit_id FROM companies WHERE domain = 'elbitsystems.com' LIMIT 1;
    SELECT id INTO infineon_id FROM companies WHERE domain = 'infineon.com' LIMIT 1;
    SELECT id INTO st_id FROM companies WHERE domain = 'st.com' LIMIT 1;
    SELECT id INTO nxp_id FROM companies WHERE domain = 'nxp.com' LIMIT 1;
    SELECT id INTO telnet_id FROM companies WHERE domain = 'groupe-telnet.com' LIMIT 1;
    SELECT id INTO microchip_id FROM companies WHERE domain = 'microchip.com' LIMIT 1;

    -- Insert executives with proper roles
    INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, influence_score, decision_style, risk_tolerance, change_appetite, trigger_topics, public_email, metadata)
    VALUES
        ('Young Liu', foxconn_id, 'Chairman & CEO', 'C-Suite', 'Asia-Pacific', 'TW', 0.95, 'Analytical', 'Moderate', 'High', ARRAY['AI','EV','semiconductors','India expansion'], 'investor@foxconn.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Kenny Wilson', jabil_id, 'CEO', 'C-Suite', 'North America', 'US', 0.88, 'Pragmatic', 'Moderate', 'High', ARRAY['healthcare','cloud','5G','reshoring'], 'ir@jabil.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Revathi Advaithi', flex_id, 'CEO', 'C-Suite', 'Asia-Pacific', 'SG', 0.87, 'Visionary', 'High', 'High', ARRAY['circular-economy','EV','next-gen-mobility'], 'investor.relations@flex.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Rob Mionis', celestica_id, 'President & CEO', 'C-Suite', 'North America', 'CA', 0.82, 'Analytical', 'Moderate', 'Moderate', ARRAY['HPS','cloud','defense','nearshoring'], 'investor@celestica.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Jure Sola', sanmina_id, 'Chairman & CEO', 'C-Suite', 'North America', 'US', 0.78, 'Directive', 'Low', 'Low', ARRAY['defense','medical','optical'], 'ir@sanmina.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Ludwig Baerlein', zollner_id, 'CEO', 'C-Suite', 'Europe', 'DE', 0.72, 'Consultative', 'Low', 'Moderate', ARRAY['automotive','medical','Industry-4.0'], 'info@zollner.de', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Patrice Caine', thales_id, 'Chairman', 'Board', 'Europe', 'FR', 0.90, 'Visionary', 'Moderate', 'High', ARRAY['AI','cybersecurity','space','defense'], 'investor.relations@thalesgroup.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Bezhalel Machlis', elbit_id, 'President & CEO', 'C-Suite', 'MENA', 'IL', 0.85, 'Directive', 'High', 'High', ARRAY['drones','C4ISR','autonomy','cyber'], 'ir@elbitsystems.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Jochen Hanebeck', infineon_id, 'CEO', 'C-Suite', 'Europe', 'DE', 0.88, 'Analytical', 'Moderate', 'High', ARRAY['SiC','GaN','automotive','AI'], 'investor.relations@infineon.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Jean-Marc Chery', st_id, 'President & CEO', 'C-Suite', 'Europe', 'CH', 0.86, 'Pragmatic', 'Moderate', 'High', ARRAY['SiC','FDSOI','automotive','industrial'], 'investor.relations@st.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Kurt Sievers', nxp_id, 'President & CEO', 'C-Suite', 'Europe', 'NL', 0.84, 'Analytical', 'Moderate', 'High', ARRAY['automotive','edge-AI','security','RISC-V'], 'investor.relations@nxp.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Mohamed Fouzri', telnet_id, 'CEO & Founder', 'C-Suite', 'MENA', 'TN', 0.65, 'Visionary', 'High', 'High', ARRAY['space','embedded','NewSpace','Africa-tech'], 'contact@groupe-telnet.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Steve Sanghi', microchip_id, 'Executive Chair', 'Board', 'North America', 'US', 0.83, 'Directive', 'Moderate', 'Moderate', ARRAY['MCU','analog','FPGA','automotive'], 'investor.relations@microchip.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb),
        ('Ganesh Moorthy', microchip_id, 'CEO', 'C-Suite', 'North America', 'US', 0.82, 'Analytical', 'Moderate', 'Moderate', ARRAY['MCU','analog','security'], 'investor.relations@microchip.com', '{"engagement_status":"tracked","source":"seed_data"}'::jsonb)
    ON CONFLICT DO NOTHING;
END $$;
