-- Delete existing POIs and re-insert with full data
DELETE FROM persons;

-- Insert comprehensive POI data with organization links
INSERT INTO persons (name, primary_org_id, "current_role", role_family, region, country_code, 
                     influence_score, public_email, public_bio,
                     decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
SELECT 
    v.name,
    c.id as primary_org_id,
    v.role,
    v.role_family,
    v.region,
    v.country_code,
    v.influence_score,
    v.email,
    v.bio,
    v.decision_style,
    v.risk_tolerance,
    v.change_appetite,
    v.trigger_topics,
    v.metadata::jsonb
FROM (VALUES
    ('Jensen Huang', 'nvidia.com', 'CEO & Founder', 'C-Suite', 'North America', 'US', 0.99,
     'investor@nvidia.com',
     'Jensen Huang co-founded NVIDIA in 1993 and has served as CEO since inception. Under his leadership, NVIDIA pioneered GPU computing and has become the dominant force in AI accelerators, data center AI, and autonomous systems. Forbes ranked him among the world''s best CEOs.',
     'Visionary', 'High', 'High',
     ARRAY['AI', 'GPU', 'datacenter', 'autonomous-vehicles', 'robotics', 'gaming', 'metaverse'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/jenhsunhuang/","phone":"+1-408-486-2000","twitter":"@nvidia","education":"Oregon State University, Stanford University","board_seats":["NVIDIA"],"net_worth":"$100B+","communication_style":"visionary","preferred_proof_type":"technical_demo"}'
    ),
    ('Lisa Su', 'amd.com', 'CEO & Chair', 'C-Suite', 'North America', 'US', 0.96,
     'investor.relations@amd.com',
     'Dr. Lisa Su has served as AMD CEO since 2014, leading one of the most remarkable turnarounds in semiconductor history. She holds a PhD from MIT and previously worked at IBM, Texas Instruments, and Freescale. Named Fortune''s Businessperson of the Year 2020.',
     'Analytical', 'Moderate', 'High',
     ARRAY['AI', 'GPU', 'CPU', 'datacenter', 'HPC', 'gaming', 'embedded'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/lisasu/","phone":"+1-408-749-4000","twitter":"@LisaSu","education":"MIT PhD Electrical Engineering","board_seats":["AMD","Cisco"],"communication_style":"analytical","preferred_proof_type":"performance_data"}'
    ),
    ('Young Liu', 'foxconn.com', 'Chairman & CEO', 'C-Suite', 'Asia-Pacific', 'TW', 0.95,
     'investor@foxconn.com',
     'Young Liu became Chairman & CEO of Foxconn (Hon Hai Precision Industry) in 2019, overseeing the world''s largest electronics contract manufacturer with over 1M employees. He leads Foxconn''s diversification into EVs, semiconductors, and digital health.',
     'Analytical', 'Moderate', 'High',
     ARRAY['AI', 'EV', 'semiconductors', 'India-expansion', 'manufacturing', 'supply-chain'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/young-liu-foxconn/","phone":"+886-2-2268-3466","education":"National Taiwan University","board_seats":["Hon Hai Precision","Sharp"],"communication_style":"pragmatic","preferred_proof_type":"operational_efficiency"}'
    ),
    ('Patrice Caine', 'thalesgroup.com', 'Chairman', 'Board', 'Europe', 'FR', 0.90,
     'investor.relations@thalesgroup.com',
     'Patrice Caine served as CEO of Thales from 2015-2023 and remains Chairman. He transformed Thales into a global leader in digital security, aerospace, and defense. Graduate of Ecole Polytechnique and Corps des Mines.',
     'Visionary', 'Moderate', 'High',
     ARRAY['AI', 'cybersecurity', 'space', 'defense', 'digital-identity', 'aerospace'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/patrice-caine/","phone":"+33-1-57-77-80-00","education":"Ecole Polytechnique","board_seats":["Thales"],"communication_style":"strategic","preferred_proof_type":"strategic_alignment"}'
    ),
    ('Jochen Hanebeck', 'infineon.com', 'CEO', 'C-Suite', 'Europe', 'DE', 0.88,
     'investor.relations@infineon.com',
     'Jochen Hanebeck became Infineon CEO in 2022 after serving as COO. He joined Infineon in 1994 and has led its transformation into a leader in automotive semiconductors, power electronics (SiC/GaN), and IoT security.',
     'Analytical', 'Moderate', 'High',
     ARRAY['SiC', 'GaN', 'automotive', 'AI', 'IoT', 'power-electronics', 'security'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/jochen-hanebeck/","phone":"+49-89-234-0","education":"University of Stuttgart","board_seats":["Infineon"],"communication_style":"engineering-focused","preferred_proof_type":"technical_specifications"}'
    ),
    ('Kenny Wilson', 'jabil.com', 'CEO', 'C-Suite', 'North America', 'US', 0.88,
     'ir@jabil.com',
     'Kenny Wilson has served as Jabil CEO since 2024, previously serving as COO. He has 25+ years experience in manufacturing and supply chain. Jabil is a Fortune 500 diversified manufacturing solutions provider.',
     'Pragmatic', 'Moderate', 'High',
     ARRAY['healthcare', 'cloud', '5G', 'reshoring', 'automotive', 'defense'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/kenny-wilson-jabil/","phone":"+1-727-577-9749","education":"MBA","board_seats":["Jabil"],"communication_style":"operational","preferred_proof_type":"cost_analysis"}'
    ),
    ('Revathi Advaithi', 'flex.com', 'CEO', 'C-Suite', 'Asia-Pacific', 'SG', 0.87,
     'investor.relations@flex.com',
     'Revathi Advaithi has served as Flex CEO since 2019. She previously held leadership roles at Eaton and Honeywell. Named one of Fortune''s Most Powerful Women. Leads Flex''s strategy in circular economy and EV manufacturing.',
     'Visionary', 'High', 'High',
     ARRAY['circular-economy', 'EV', 'next-gen-mobility', 'sustainability', 'healthcare'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/revathi-advaithi/","phone":"+65-6890-7188","education":"Thunderbird School of Global Management","board_seats":["Flex","Uber"],"communication_style":"transformational","preferred_proof_type":"sustainability_metrics"}'
    ),
    ('Jean-Marc Chery', 'st.com', 'President & CEO', 'C-Suite', 'Europe', 'CH', 0.86,
     'investor.relations@st.com',
     'Jean-Marc Chery has served as STMicroelectronics CEO since 2018. He joined ST in 1986 and led the company''s expansion in automotive, industrial, and IoT semiconductors. Champion of European semiconductor sovereignty.',
     'Pragmatic', 'Moderate', 'High',
     ARRAY['SiC', 'FDSOI', 'automotive', 'industrial', 'IoT', 'MEMS'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/jean-marc-chery/","phone":"+41-22-929-29-29","education":"ENSEEIHT Toulouse","board_seats":["STMicroelectronics"],"communication_style":"technical","preferred_proof_type":"technology_roadmap"}'
    ),
    ('Bezhalel Machlis', 'elbitsystems.com', 'President & CEO', 'C-Suite', 'MENA', 'IL', 0.85,
     'ir@elbitsystems.com',
     'Bezhalel Machlis has served as Elbit Systems CEO since 2013. He joined Elbit in 1991 and led its growth into a $6B+ defense electronics company. Expert in C4ISR, drones, and military avionics.',
     'Directive', 'High', 'High',
     ARRAY['drones', 'C4ISR', 'autonomy', 'cyber', 'defense', 'avionics'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/bezhalel-machlis/","phone":"+972-4-831-6948","education":"Technion Israel Institute of Technology","board_seats":["Elbit Systems"],"communication_style":"direct","preferred_proof_type":"defense_capabilities"}'
    ),
    ('Kurt Sievers', 'nxp.com', 'President & CEO', 'C-Suite', 'Europe', 'NL', 0.84,
     'investor.relations@nxp.com',
     'Kurt Sievers became NXP CEO in 2020 after serving as President. He joined NXP in 1995 and led its automotive and IoT divisions. Advocates for automotive semiconductor innovation and RISC-V adoption.',
     'Analytical', 'Moderate', 'High',
     ARRAY['automotive', 'edge-AI', 'security', 'RISC-V', 'IoT', 'NFC'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/kurt-sievers/","phone":"+31-40-272-9999","education":"University of Munich","board_seats":["NXP Semiconductors"],"communication_style":"analytical","preferred_proof_type":"market_data"}'
    ),
    ('Steve Sanghi', 'microchip.com', 'Executive Chair', 'Board', 'North America', 'US', 0.83,
     'investor.relations@microchip.com',
     'Steve Sanghi served as Microchip CEO from 1990-2021 and remains Executive Chairman. He led Microchip from startup to Fortune 500, completing over 20 acquisitions including Atmel and Microsemi.',
     'Directive', 'Moderate', 'Moderate',
     ARRAY['MCU', 'analog', 'FPGA', 'automotive', 'aerospace', 'defense'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/steve-sanghi/","phone":"+1-480-792-7200","education":"University of Texas, Purdue University","board_seats":["Microchip Technology"],"communication_style":"decisive","preferred_proof_type":"financial_performance"}'
    ),
    ('Ganesh Moorthy', 'microchip.com', 'CEO', 'C-Suite', 'North America', 'US', 0.82,
     'investor.relations@microchip.com',
     'Ganesh Moorthy became Microchip CEO in 2021 after serving as President and COO. He joined Microchip in 2001 and led product development across MCU, analog, and FPGA divisions.',
     'Analytical', 'Moderate', 'Moderate',
     ARRAY['MCU', 'analog', 'security', 'IoT', 'automotive'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/ganesh-moorthy/","phone":"+1-480-792-7200","education":"Washington State University","board_seats":["Microchip Technology"],"communication_style":"methodical","preferred_proof_type":"product_roadmap"}'
    ),
    ('Rob Mionis', 'celestica.com', 'President & CEO', 'C-Suite', 'North America', 'CA', 0.82,
     'investor@celestica.com',
     'Rob Mionis has served as Celestica CEO since 2015. He previously held leadership roles at Vitesse Semiconductor and IBM. Leads Celestica''s focus on HPS (Hardware Platform Solutions) and cloud infrastructure.',
     'Analytical', 'Moderate', 'Moderate',
     ARRAY['HPS', 'cloud', 'defense', 'nearshoring', 'AI-infrastructure'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/rob-mionis/","phone":"+1-416-448-5800","education":"University of Toronto","board_seats":["Celestica"],"communication_style":"strategic","preferred_proof_type":"technical_capability"}'
    ),
    ('Jure Sola', 'sanmina.com', 'Chairman & CEO', 'C-Suite', 'North America', 'US', 0.78,
     'ir@sanmina.com',
     'Jure Sola co-founded Sanmina in 1980 and has served as Chairman and CEO. He built Sanmina into a $7B+ EMS company focused on complex industrial, defense, medical, and optical systems.',
     'Directive', 'Low', 'Low',
     ARRAY['defense', 'medical', 'optical', 'industrial', 'aerospace'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/jure-sola/","phone":"+1-408-964-3500","education":"University of Ljubljana","board_seats":["Sanmina"],"communication_style":"founder-driven","preferred_proof_type":"operational_track_record"}'
    ),
    ('Ludwig Baerlein', 'zollner.de', 'CEO', 'C-Suite', 'Europe', 'DE', 0.72,
     'info@zollner.de',
     'Ludwig Baerlein joined Zollner Elektronik leadership and oversees one of Europe''s largest privately-owned EMS companies. Zollner specializes in automotive, medical, and industrial electronics.',
     'Consultative', 'Low', 'Moderate',
     ARRAY['automotive', 'medical', 'Industry-4.0', 'smart-factory'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/company/zollner-elektronik/","phone":"+49-9461-952-0","education":"German engineering background","board_seats":["Zollner Elektronik"],"communication_style":"collaborative","preferred_proof_type":"quality_certifications"}'
    ),
    ('Mohamed Fouzri', 'groupe-telnet.com', 'CEO & Founder', 'C-Suite', 'MENA', 'TN', 0.65,
     'contact@groupe-telnet.com',
     'Mohamed Fouzri founded Telnet Holding in 1994, building Tunisia''s leading technology company. He pioneered Tunisia''s aerospace industry with Challenge One satellite program and focuses on embedded systems and NewSpace.',
     'Visionary', 'High', 'High',
     ARRAY['space', 'embedded', 'NewSpace', 'Africa-tech', 'aerospace'],
     '{"engagement_status":"tracked","linkedin":"https://www.linkedin.com/in/mohamed-fouzri/","phone":"+216-70-835-500","education":"ENSI Tunisia","board_seats":["Telnet Holding"],"communication_style":"entrepreneurial","preferred_proof_type":"innovation_showcase"}'
    )
) AS v(name, domain, role, role_family, region, country_code, influence_score, email, bio, decision_style, risk_tolerance, change_appetite, trigger_topics, metadata)
JOIN companies c ON c.domain = v.domain;
