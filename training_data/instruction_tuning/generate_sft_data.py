#!/usr/bin/env python3
"""
Generate instruction fine-tuning examples for ApexIntel LLM training.
Produces JSONL files for all 7 task types per IMPLEMENTATION.md Section 8.0.2 Phase 2.

Total: 2,700 examples
- recipe_hypothesis_generation: 500
- poi_synthesis: 300
- weekly_memo_generation: 100
- company_dossier_synthesis: 200
- narrative_rendering: 500
- entity_extraction: 1000
- competitive_analysis: 100
"""
import json
import os
import random
import uuid
from datetime import datetime, timedelta

random.seed(42)
DEST = os.path.join(os.path.dirname(os.path.abspath(__file__)), "instruction_tuning")
os.makedirs(DEST, exist_ok=True)

# ── Domain Data ─────────────────────────────────────────────────────────────

EMS_COMPANIES = [
    ("Jabil Inc.", "JBL", "US", "Ems", "St. Petersburg, FL"),
    ("Flex Ltd.", "FLEX", "SG", "Ems", "Singapore"),
    ("Celestica Inc.", "CLS", "CA", "Ems", "Toronto, ON"),
    ("Benchmark Electronics", "BHE", "US", "Ems", "Tempe, AZ"),
    ("Plexus Corp.", "PLXS", "US", "Ems", "Neenah, WI"),
    ("Sanmina Corp.", "SANM", "US", "Ems", "San Jose, CA"),
    ("TTM Technologies", "TTMI", "US", "Ems", "Santa Ana, CA"),
    ("Key Tronic Corp.", "KTEC", "US", "Ems", "Spokane, WA"),
    ("Fabrinet", "FN", "TH", "Ems", "Bangkok"),
    ("Foxconn", "2317.TW", "TW", "Ems", "New Taipei City"),
    ("Pegatron", "4938.TW", "TW", "Ems", "Taipei"),
    ("Wistron", "3231.TW", "TW", "Ems", "Taipei"),
    ("Quanta Computer", "2382.TW", "TW", "Ems", "Taoyuan"),
    ("Compal Electronics", "2324.TW", "TW", "Ems", "Taipei"),
    ("BYD Electronic", "285.HK", "CN", "Ems", "Shenzhen"),
    ("Luxshare Precision", "002475.SZ", "CN", "Ems", "Dongguan"),
    ("GoerTek", "002241.SZ", "CN", "Ems", "Weifang"),
    ("Venture Corp.", "V03.SI", "SG", "Ems", "Singapore"),
    ("USI (Universal Scientific Industrial)", "601231.SS", "TW", "Ems", "Shanghai"),
    ("Zollner Elektronik", "private", "DE", "Ems", "Zandt"),
    ("LACROIX Group", "LACR.PA", "FR", "Ems", "Nantes"),
    ("Starz Electronics", "private", "TN", "Ems", "Tunis"),
    ("Telnet Holding", "TLNET.TN", "TN", "Ems", "Ben Arous"),
    ("All Circuits", "private", "TN", "Ems", "Medjez el-Bab"),
    ("SMEI (Sorekor)", "private", "TN", "Ems", "Sousse"),
    ("IAI (Israel Aerospace Industries)", "IAI", "IL", "Oem", "Ben Gurion"),
    ("Elbit Systems", "ESLT", "IL", "Oem", "Haifa"),
    ("Rafael Advanced Defense Systems", "RAFAEL", "IL", "Oem", "Haifa"),
    ("Tower Semiconductor", "TSEM", "IL", "Tier1", "Migdal HaEmek"),
    ("Orbotech (KLA)", "KLAC", "IL", "Tier1", "Yavne"),
]

OEM_COMPANIES = [
    ("Apple Inc.", "AAPL", "US", "Oem", "Cupertino, CA"),
    ("Samsung Electronics", "005930.KS", "KR", "Oem", "Suwon"),
    ("Siemens AG", "SIE.DE", "DE", "Oem", "Munich"),
    ("Bosch", "private", "DE", "Oem", "Gerlingen"),
    ("Continental AG", "CON.DE", "DE", "Oem", "Hanover"),
    ("Thales Group", "HO.PA", "FR", "Oem", "Paris"),
    ("Safran", "SAF.PA", "FR", "Oem", "Paris"),
    ("Airbus Defence", "AIR.PA", "FR", "Oem", "Toulouse"),
    ("BAE Systems", "BA.L", "GB", "Oem", "London"),
    ("Leonardo S.p.A.", "LDO.MI", "IT", "Oem", "Rome"),
    ("Honeywell", "HON", "US", "Oem", "Charlotte, NC"),
    ("Northrop Grumman", "NOC", "US", "Oem", "Falls Church, VA"),
    ("Raytheon Technologies", "RTX", "US", "Oem", "Arlington, VA"),
    ("Lockheed Martin", "LMT", "US", "Oem", "Bethesda, MD"),
    ("General Electric", "GE", "US", "Oem", "Boston, MA"),
    ("ZF Friedrichshafen", "private", "DE", "Oem", "Friedrichshafen"),
    ("Schneider Electric", "SU.PA", "FR", "Oem", "Rueil-Malmaison"),
    ("STMicroelectronics", "STM", "CH", "Tier1", "Geneva"),
    ("Infineon Technologies", "IFX.DE", "DE", "Tier1", "Neubiberg"),
    ("NXP Semiconductors", "NXPI", "NL", "Tier1", "Eindhoven"),
]

CAPABILITIES = [
    "SMT Assembly", "Through-Hole Assembly", "BGA/CSP Assembly", "Micro-BGA",
    "Box Build", "Cable Assembly", "Wire Harness", "Conformal Coating",
    "Selective Soldering", "Wave Soldering", "Reflow Soldering", "Vapor Phase Soldering",
    "Flying Probe Testing", "ICT (In-Circuit Test)", "Functional Test", "X-Ray Inspection",
    "AOI (Automated Optical Inspection)", "SPI (Solder Paste Inspection)",
    "Potting & Encapsulation", "Press-Fit Assembly", "Die Bonding", "Wire Bonding",
    "Flip Chip Assembly", "System Integration", "Firmware Programming",
    "PCB Fabrication (Rigid)", "PCB Fabrication (Flex)", "PCB Fabrication (Rigid-Flex)",
    "HDI PCB", "RF/Microwave PCB", "Metal Core PCB", "Ceramic Substrate",
    "Prototype Services", "NPI (New Product Introduction)", "DFM Analysis",
    "Supply Chain Management", "Component Sourcing", "Obsolescence Management",
    "Depot Repair", "RMA Management", "Field Service",
    "ESD-Sensitive Handling", "Clean Room Assembly (ISO 7)", "Clean Room Assembly (ISO 5)",
    "MIL-STD Compliance", "J-STD-001 Class 3", "IPC-A-610 Class 3",
    "Thermal Management", "Power Electronics Assembly", "LED Assembly",
    "Automotive Electronics", "Medical Device Assembly", "Aerospace Electronics",
    "Defense Electronics", "Industrial Control Systems", "IoT Device Assembly",
    "5G Infrastructure Assembly", "EV Battery Management Systems",
]

CERTIFICATIONS = [
    ("ISO 9001:2015", "Active"), ("ISO 14001:2015", "Active"),
    ("IATF 16949:2016", "Active"), ("AS9100D", "Active"),
    ("ISO 13485:2016", "Active"), ("ISO 45001:2018", "Active"),
    ("NADCAP", "Active"), ("IPC-A-610 CIS", "Active"),
    ("J-STD-001 CIS", "Active"), ("UL Listed", "Active"),
    ("ISO 27001:2022", "Active"), ("SOC 2 Type II", "Active"),
    ("NIST 800-171", "Pending"), ("CMMC Level 2", "Pending"),
    ("ISO 22301", "Active"), ("Mil-PRF-55110", "Active"),
    ("Mil-PRF-31032", "Active"), ("IPC-6012 Class 3", "Active"),
    ("ISO 13485:2016", "Expired"), ("IATF 16949:2016", "Expired"),
]

REGIONS = ["TN", "MA", "IL", "CN", "TW", "KR", "JP", "DE", "FR", "US", "SG", "TH", "GB", "IT"]
REGION_LABELS = {
    "TN": "Tunisia", "MA": "Morocco", "IL": "Israel", "CN": "China",
    "TW": "Taiwan", "KR": "South Korea", "JP": "Japan", "DE": "Germany",
    "FR": "France", "US": "United States", "SG": "Singapore", "TH": "Thailand",
    "GB": "United Kingdom", "IT": "Italy",
}

INDUSTRIES = ["automotive", "aerospace", "defense", "medical", "industrial", "telecom", "consumer", "energy", "iot", "semiconductor"]

RECIPE_CATEGORIES = ["demand", "supply_chain", "competitor", "security", "poi"]
SEVERITIES = ["critical", "warning", "info"]
RECIPE_CODES = [f"{cat[0].upper()}{i:03d}" for cat in RECIPE_CATEGORIES for i in range(1, 101)]

OBSERVATION_TYPES = [
    "JobPost", "TenderPosted", "WebChange", "CertificationUpdate",
    "PatentPublished", "PortMetric", "CommodityPrice", "FxRate",
    "DnsPosture", "NewDomain", "VulnNotice", "PersonMention",
    "RoleChange", "SpeakerAppearance", "ProcurementSignal", "CompetitorEvent",
]

SIGNAL_OPERATORS = ["increase", "decrease", "above", "below", "equals", "contains"]
TRANSFORM_TYPES = ["zscore", "pct_change", "rolling_mean", "count", "diff"]
TEST_TYPES = ["fisher_exact", "cross_correlation", "mutual_information", "hazard_uplift"]

ROLE_FAMILIES = [
    "Procurement", "SupplierQuality", "Engineering", "Operations",
    "Security", "Executive", "Government", "FreeZoneAuthority",
    "PortLogistics", "CertificationBody", "IndustryAssociation",
    "Distributor", "Finance", "Legal", "Military", "Intelligence",
]

DECISION_STYLES = ["CostFirst", "QualityFirst", "SpeedFirst", "RiskFirst", "ComplianceFirst", "BalancedAnalytical"]
CHANGE_APPETITES = ["EarlyAdopter", "Pragmatist", "Conservative", "Laggard"]
PROOF_TYPES = ["KpiMetrics", "Certifications", "CaseStudies", "AuditReadiness", "TechDemos", "CostTransparency"]

FIRST_NAMES_EN = ["James", "Sarah", "Michael", "Emily", "David", "Jennifer", "Robert", "Lisa", "William", "Jessica"]
FIRST_NAMES_AR = ["أحمد", "فاطمة", "محمد", "عائشة", "خالد", "مريم", "يوسف", "نور", "عمر", "ليلى"]
FIRST_NAMES_HE = ["אבי", "מירב", "יוסי", "רונית", "דוד", "שרה", "משה", "רחל", "אריה", "נעמי"]
FIRST_NAMES_FR = ["Pierre", "Marie", "Jean", "Sophie", "Philippe", "Isabelle", "François", "Catherine", "Laurent", "Nathalie"]
FIRST_NAMES_ZH = ["伟", "芳", "强", "丽", "军", "静", "明", "洁", "勇", "敏"]
FIRST_NAMES_JA = ["太郎", "花子", "健太", "由美", "大輔", "美咲", "翔太", "愛", "拓也", "さくら"]
FIRST_NAMES_KO = ["민준", "서연", "도윤", "지우", "하준", "수아", "시우", "지민", "준우", "하은"]

LAST_NAMES_EN = ["Smith", "Johnson", "Williams", "Brown", "Jones", "Davis", "Miller", "Wilson", "Moore", "Taylor"]
LAST_NAMES_AR = ["بن علي", "الشريف", "المنصوري", "بوعزيزي", "الأمين", "بن يوسف", "العياري", "الجبالي", "بن سعيد", "التونسي"]
LAST_NAMES_HE = ["כהן", "לוי", "מזרחי", "פרץ", "ביטון", "דהן", "אברהם", "פרידמן", "אזולאי", "שפירא"]
LAST_NAMES_FR = ["Dubois", "Martin", "Bernard", "Dupont", "Moreau", "Laurent", "Simon", "Michel", "Lefebvre", "Leroy"]
LAST_NAMES_ZH = ["王", "李", "张", "刘", "陈", "杨", "赵", "黄", "周", "吴"]
LAST_NAMES_JA = ["田中", "鈴木", "佐藤", "高橋", "伊藤", "渡辺", "山本", "中村", "小林", "加藤"]
LAST_NAMES_KO = ["김", "이", "박", "최", "정", "강", "조", "윤", "장", "임"]

CHANNELS = ["Email", "LinkedIn", "Conference", "Phone", "In-person meeting", "Video call", "WhatsApp"]

# ── Helpers ─────────────────────────────────────────────────────────────────

def uid():
    return str(uuid.uuid4())

def rand_date(start_year=2023, end_year=2025):
    d = datetime(start_year, 1, 1) + timedelta(days=random.randint(0, (end_year - start_year) * 365))
    return d.strftime("%Y-%m-%dT%H:%M:%SZ")

def rand_company(pool=None):
    return random.choice(pool or EMS_COMPANIES)

def rand_capabilities(n=None):
    n = n or random.randint(3, 12)
    return random.sample(CAPABILITIES, min(n, len(CAPABILITIES)))

def rand_certs(n=None):
    n = n or random.randint(2, 8)
    return random.sample(CERTIFICATIONS, min(n, len(CERTIFICATIONS)))

def rand_name(region="US"):
    mapping = {
        "TN": (FIRST_NAMES_AR, LAST_NAMES_AR), "MA": (FIRST_NAMES_AR, LAST_NAMES_AR),
        "IL": (FIRST_NAMES_HE, LAST_NAMES_HE), "CN": (FIRST_NAMES_ZH, LAST_NAMES_ZH),
        "TW": (FIRST_NAMES_ZH, LAST_NAMES_ZH), "JP": (FIRST_NAMES_JA, LAST_NAMES_JA),
        "KR": (FIRST_NAMES_KO, LAST_NAMES_KO), "FR": (FIRST_NAMES_FR, LAST_NAMES_FR),
        "DE": (FIRST_NAMES_EN, LAST_NAMES_EN), "US": (FIRST_NAMES_EN, LAST_NAMES_EN),
        "SG": (FIRST_NAMES_EN, LAST_NAMES_EN), "TH": (FIRST_NAMES_EN, LAST_NAMES_EN),
        "GB": (FIRST_NAMES_EN, LAST_NAMES_EN), "IT": (FIRST_NAMES_EN, LAST_NAMES_EN),
    }
    firsts, lasts = mapping.get(region, (FIRST_NAMES_EN, LAST_NAMES_EN))
    return f"{random.choice(firsts)} {random.choice(lasts)}"

def rand_priority_vector():
    v = {k: round(random.uniform(0.1, 0.95), 2) for k in ["cost", "quality", "speed", "resilience", "compliance", "security"]}
    dominant = max(v, key=v.get)
    v["confidence"] = round(random.uniform(0.5, 0.95), 2)
    return v, dominant

def jsonl_write(filepath, examples):
    with open(filepath, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")
    print(f"  {os.path.basename(filepath)}: {len(examples)} examples, {os.path.getsize(filepath):,}b")


# ── Task 1: Recipe Hypothesis Generation (500 examples) ────────────────────

RECIPE_PATTERNS = [
    ("Certification expiry approaching for {entity} in {region}",
     "When a company's key certification (IATF 16949, AS9100D, ISO 13485) is within 90 days of expiry and no renewal filing is detected, this may indicate compliance risk or potential business disruption.",
     ["CertificationUpdate"], ["competitor", "supply_chain"]),
    ("Unusual hiring surge at {entity} suggests capacity expansion",
     "A 3x increase in job postings for production roles (SMT operator, quality inspector, process engineer) within 30 days signals potential new contract win or capacity expansion.",
     ["JobPost"], ["demand", "competitor"]),
    ("Port congestion spike at {port} threatens {region} supply chain",
     "When port dwell times exceed 2 standard deviations from 90-day mean, it signals potential supply chain disruption for companies dependent on that corridor.",
     ["PortMetric"], ["supply_chain"]),
    ("New competitor capability page detected for {entity}",
     "A web change on a competitor's capability page adding new service lines (e.g., EV battery management, 5G infrastructure) signals strategic pivot or new market entry.",
     ["WebChange", "CompetitorEvent"], ["competitor"]),
    ("Key procurement contact role change at {entity}",
     "When a known procurement decision-maker changes title or organization, it may create an engagement window or signal organizational restructuring.",
     ["RoleChange", "PersonMention"], ["poi"]),
    ("DNS posture degradation detected for {entity}",
     "Missing DMARC/DKIM/SPF records or new lookalike domains registered targeting a supplier signals potential phishing or business email compromise risk.",
     ["DnsPosture", "NewDomain"], ["security"]),
    ("Commodity price volatility affecting {component} supply",
     "When copper/palladium/tin prices show >15% month-over-month change, EMS companies with high exposure face margin pressure or may pass costs to customers.",
     ["CommodityPrice", "FxRate"], ["supply_chain"]),
    ("Patent filing surge in {technology} by {entity}",
     "A >200% increase in patent filings in a specific technology area (e.g., SiP, advanced packaging, thermal management) signals R&D investment direction.",
     ["PatentPublished"], ["competitor", "demand"]),
    ("Tender volume increase in {region} for {industry}",
     "When public tender postings in a region increase >50% quarter-over-quarter for a specific industry vertical, it signals growing demand.",
     ["TenderPosted", "ProcurementSignal"], ["demand"]),
    ("Vulnerability disclosure affecting {product} used by {entity}",
     "A critical CVE affecting industrial control systems, SCADA, or manufacturing equipment used by a subject company creates immediate operational risk.",
     ["VulnNotice"], ["security"]),
    ("FX rate shift disadvantaging {region} manufacturing",
     "When local currency appreciates >5% vs USD in 30 days, export-oriented EMS manufacturers face competitiveness erosion vs other low-cost regions.",
     ["FxRate", "CommodityPrice"], ["supply_chain"]),
    ("Speaker appearance at {event} by {entity} executive",
     "A C-level executive from a target company presenting at a major trade show creates an engagement opportunity and signals strategic priorities.",
     ["SpeakerAppearance", "PersonMention"], ["poi", "demand"]),
    ("Manufacturing distress signals at {entity}",
     "Combination of layoff announcements, facility closure notices, and credit rating downgrades within 60 days signals potential supplier distress.",
     ["JobPost", "WebChange", "CompetitorEvent"], ["supply_chain", "competitor"]),
    ("Sanctions list change affecting {entity} or {region}",
     "When a company, individual, or region is added to OFAC SDN, EU sanctions, or UN sanctions lists, immediate compliance review is required.",
     ["CompetitorEvent"], ["security", "supply_chain"]),
    ("Automotive OEM sourcing shift detected",
     "When multiple automotive OEMs simultaneously issue RFQs for the same capability area (e.g., EV charging modules), it signals market-wide demand shift.",
     ["TenderPosted", "ProcurementSignal", "WebChange"], ["demand"]),
]

def gen_recipe_hypothesis(n=500):
    examples = []
    for i in range(n):
        pat = random.choice(RECIPE_PATTERNS)
        company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
        region = random.choice(REGIONS)
        port = random.choice(["Tanger Med", "Haifa", "Shanghai", "Rotterdam", "Singapore", "Long Beach", "Hamburg", "Rades"])
        technology = random.choice(["System-in-Package", "Advanced Packaging", "Thermal Management", "5G mmWave", "SiC Power Electronics", "Embedded Components", "HDI via-in-pad"])
        component = random.choice(["copper", "palladium", "tin", "MLCC capacitors", "power MOSFETs", "MCU units", "connectors"])
        event = random.choice(["IPC APEX EXPO 2025", "Electronica 2024", "PCIM Europe 2025", "SEMICON West 2024", "embedded world 2025"])
        industry = random.choice(INDUSTRIES)

        title = pat[0].format(entity=company[0], region=REGION_LABELS.get(region, region), port=port, technology=technology, component=component, event=event, industry=industry, product=component)
        description = pat[1]
        obs_types = pat[2]
        categories = pat[3]

        signals = []
        for obs in obs_types:
            signals.append({
                "observation_type": obs,
                "field": random.choice(["count", "value", "change_pct", "zscore", "binary"]),
                "operator": random.choice(SIGNAL_OPERATORS),
                "threshold": round(random.uniform(0.5, 5.0), 1) if random.random() > 0.3 else None,
                "window_days": random.choice([7, 14, 30, 60, 90]),
                "value": None
            })

        recipe = {
            "id": uid(),
            "code": random.choice(RECIPE_CODES),
            "name": title[:80],
            "description": description,
            "status": "Seed",
            "signals": signals,
            "transforms": [
                {
                    "transform_type": random.choice(TRANSFORM_TYPES),
                    "field": random.choice(["count", "value", "change_pct"]),
                    "window_days": random.choice([7, 14, 30]),
                    "params": {}
                }
            ],
            "statistical_test": {
                "test_type": random.choice(TEST_TYPES),
                "params": {"confidence_level": 0.99}
            },
            "min_uplift": round(random.uniform(1.2, 3.0), 1),
            "max_p_value": 0.01,
            "min_time_slices": random.choice([3, 4, 5]),
            "min_entities": random.choice([3, 5, 7, 10]),
            "insight_template": f"**{title}**: {description} Evidence: {{evidence_summary}}. Impact assessment: {{impact_score}}.",
            "action_template": "; ".join(random.sample([
                "Alert account manager for {entity_name}",
                "Schedule capability review meeting",
                "Update risk score in entity profile",
                "Monitor for follow-up signals in next 14 days",
                "Prepare competitive positioning brief",
                "Flag for weekly memo inclusion",
                "Trigger dossier refresh for {entity_name}",
                "Cross-reference with sanctions watchlist",
                "Notify security team for threat assessment",
                "Prepare engagement talking points",
            ], random.randint(2, 4))),
            "applicability": {
                "geos": random.sample(REGIONS, random.randint(1, 4)),
                "industries": random.sample(INDUSTRIES, random.randint(1, 3)),
                "notes": f"Applicable to {random.choice(['all', 'Tier 1', 'strategic', 'monitored'])} entities in scope"
            },
            "severity": random.choice(SEVERITIES),
            "category": random.choice(categories),
        }

        system = "You are an OSINT analyst generating insight recipes for an EMS (Electronics Manufacturing Services) competitive intelligence platform. Generate a complete recipe JSON that defines signal detection patterns, statistical validation criteria, and action playbooks. The recipe must be specific, actionable, and grounded in observable public signals."
        user = f"Pattern candidate: Signals={obs_types}, Effects observed in entity_type={company[3]}, region={region}, industry={industry}. Describe: {title}"
        assistant = json.dumps(recipe, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "recipe_hypothesis_generation.jsonl"), examples)
    return examples


# ── Task 2: POI Synthesis (300 examples) ────────────────────────────────────

def gen_poi_synthesis(n=300):
    examples = []
    for i in range(n):
        region = random.choice(REGIONS)
        name = rand_name(region)
        company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
        role_family = random.choice(ROLE_FAMILIES)
        roles = {
            "Procurement": ["VP Procurement", "Director of Sourcing", "Chief Procurement Officer", "Procurement Manager", "Strategic Sourcing Lead"],
            "SupplierQuality": ["VP Supplier Quality", "Director SQE", "Supplier Quality Manager", "Quality Audit Lead"],
            "Engineering": ["VP Engineering", "Director of R&D", "CTO", "Principal Engineer", "NPI Manager"],
            "Operations": ["COO", "VP Operations", "Plant Manager", "Director of Manufacturing", "Production Manager"],
            "Security": ["CISO", "VP Information Security", "Director Cybersecurity", "Security Operations Manager"],
            "Executive": ["CEO", "President", "Managing Director", "General Manager", "Division President"],
            "Government": ["Director General", "Minister", "Deputy Minister", "Commissioner", "Secretary General"],
            "FreeZoneAuthority": ["CEO Free Zone", "Director of Operations", "Investment Director", "Zone Manager"],
            "Military": ["Brigadier General", "Colonel", "Director of Acquisitions", "Program Manager"],
        }
        current_role = random.choice(roles.get(role_family, ["Senior Director", "VP", "Director", "Manager"]))

        priority_vec, dominant = rand_priority_vector()
        decision_style = random.choice(DECISION_STYLES)
        change_appetite = random.choice(CHANGE_APPETITES)
        proof_types = random.sample(PROOF_TYPES, random.randint(1, 3))
        pain_index = round(random.uniform(0.1, 0.9), 2)
        influence_score = round(random.uniform(0.2, 0.95), 2)

        n_artifacts = random.randint(3, 12)
        artifact_types = ["PressQuote", "SpeakerBio", "Patent", "StandardsRole", "Interview", "Podcast", "Article", "RoleChange", "SocialPost"]
        artifacts = []
        for _ in range(n_artifacts):
            at = random.choice(artifact_types)
            artifacts.append({
                "type": at,
                "title": f"{at} - {name} at {random.choice(['IPC APEX', 'Electronica', 'SEMICON', 'Industry Forum', 'LinkedIn', 'Press Release'])}",
                "date": rand_date(),
                "source": random.choice(["linkedin.com", "ems-now.com", "circuits-assembly.com", "ipc.org", "reuters.com", "globes.co.il", "medias24.com"]),
                "snippet": f"{name} discussed {random.choice(['supply chain resilience', 'quality standards', 'cost optimization', 'digital transformation', 'sustainability', 'nearshoring strategy', 'component shortage mitigation'])} in the context of {random.choice(INDUSTRIES)} sector."
            })

        synthesis = {
            "person_id": uid(),
            "name": name,
            "org": company[0],
            "current_role": current_role,
            "role_family": role_family,
            "region": region,
            "country_code": region,
            "priority_vector": priority_vec,
            "dominant_priority": dominant,
            "psychological_profile": {
                "decision_style": decision_style,
                "change_appetite": change_appetite,
                "pain_index": pain_index,
                "preferred_proof": proof_types,
                "risk_tolerance": round(random.uniform(0.2, 0.8), 2)
            },
            "influence_assessment": {
                "influence_score": influence_score,
                "influence_label": "High" if influence_score > 0.7 else "Medium" if influence_score > 0.4 else "Low",
                "graph_centrality": round(random.uniform(0.1, 0.8), 2),
                "public_recurrence": random.randint(1, 50),
                "network_size": random.randint(5, 200),
            },
            "what_changed": f"{name} was recently {'promoted to' if random.random() > 0.5 else 'appointed as'} {current_role} at {company[0]}. {'Previously held similar role at ' + rand_company()[0] + '.' if random.random() > 0.5 else 'This represents a significant organizational change.'}",
            "what_it_implies": f"As a {decision_style.replace('First', '-first').replace('Balanced', 'balanced ')} decision-maker with {change_appetite.lower()} change appetite, {name.split()[0]} is likely to prioritize {dominant} in supplier evaluations. Pain index of {pain_index:.2f} suggests {'acute' if pain_index > 0.7 else 'moderate' if pain_index > 0.4 else 'low'} pressure on current supplier relationships.",
            "how_to_approach": {
                "recommended_proof_type": proof_types[0],
                "talking_points": [
                    f"Lead with {proof_types[0].replace('_', ' ').lower()} demonstrating {dominant} advantages",
                    f"Reference {company[0]}'s {random.choice(INDUSTRIES)} exposure",
                    f"Prepare {random.choice(['audit readiness documentation', 'cost comparison analysis', 'quality metrics dashboard', 'case study from similar engagement'])}",
                ],
                "avoid_topics": random.sample([
                    "price pressure tactics", "competitor criticism", "unverified claims",
                    "political topics", "aggressive timeline commitments", "quality shortcuts",
                ], 2),
                "best_channel": random.choice(CHANNELS),
                "timing_recommendation": random.choice(["Pre-trade show outreach", "Post-quarterly results", "During certification renewal window", "After organizational change settles (30-60 days)"]),
                "engagement_priority": random.choice(["Immediate", "High", "Medium", "Monitor"]),
            }
        }

        system = "You are synthesizing professional intelligence about a Person of Interest (POI) for an EMS competitive intelligence platform. Analyze the provided public artifacts to produce a structured profile including priority vectors, psychological assessment, influence scoring, and actionable engagement guidance. All analysis must be based solely on provided public artifacts — never fabricate information."
        user = f"Recent artifacts for {name} ({current_role} at {company[0]}, {REGION_LABELS.get(region, region)}):\n{json.dumps(artifacts, ensure_ascii=False, indent=2)}"
        assistant = json.dumps(synthesis, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "poi_synthesis.jsonl"), examples)
    return examples


# ── Task 3: Weekly Memo Generation (100 examples) ──────────────────────────

def gen_weekly_memo(n=100):
    examples = []
    for i in range(n):
        year = random.choice([2024, 2025])
        week = random.randint(1, 52)
        n_insights = random.randint(5, 25)
        critical = random.randint(0, min(3, n_insights))
        warning = random.randint(1, min(8, n_insights - critical))
        info = n_insights - critical - warning

        insights_list = []
        for j in range(n_insights):
            sev = "critical" if j < critical else "warning" if j < critical + warning else "info"
            company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
            region = random.choice(REGIONS)
            cat = random.choice(RECIPE_CATEGORIES)
            pat_title = random.choice(RECIPE_PATTERNS)[0]
            try:
                pat_title = pat_title.format(entity=company[0], region=REGION_LABELS.get(region, region), port="Tanger Med", technology="SiP", component="MLCC", event="IPC APEX", industry="automotive", product="connectors")
            except (KeyError, IndexError):
                pass
            insights_list.append({
                "recipe_code": random.choice(RECIPE_CODES),
                "entity_name": company[0],
                "entity_type": company[3],
                "region": region,
                "severity": sev,
                "category": cat,
                "title": pat_title[:80],
                "confidence": round(random.uniform(0.6, 0.99), 2),
                "evidence_count": random.randint(1, 8),
            })

        # Build the memo
        top_actions = []
        for j, ins in enumerate(insights_list[:min(5, n_insights)]):
            top_actions.append({
                "priority": j + 1,
                "action": random.choice([
                    f"Schedule urgent review of {ins['entity_name']} supplier status",
                    f"Initiate engagement with {ins['entity_name']} procurement team",
                    f"Update risk assessment for {ins['entity_name']}",
                    f"Prepare competitive response for {REGION_LABELS.get(ins['region'], ins['region'])} market",
                    f"Brief security team on {ins['entity_name']} DNS posture findings",
                    f"Refresh dossier for key contacts at {ins['entity_name']}",
                ]),
                "source_recipe": ins["recipe_code"],
                "entity_name": ins["entity_name"],
                "impact_label": random.choice(["High", "Medium", "Low"]),
                "confidence": ins["confidence"],
            })

        regional_groups = {}
        for ins in insights_list:
            r = ins["region"]
            if r not in regional_groups:
                regional_groups[r] = []
            regional_groups[r].append(ins)

        regional_sections = []
        for r, group in regional_groups.items():
            regional_sections.append({
                "region": r,
                "region_label": REGION_LABELS.get(r, r),
                "insight_count": len(group),
                "top_insights": [{"recipe_code": g["recipe_code"], "entity_name": g["entity_name"], "title": g["title"], "severity": g["severity"]} for g in group[:3]]
            })

        security_insights = [ins for ins in insights_list if ins["category"] == "security"]

        memo = {
            "id": uid(),
            "week_number": week,
            "year": year,
            "generated_at": rand_date(year, year),
            "executive_summary": f"Week {week}/{year}: {n_insights} new insights detected across {len(regional_groups)} regions. {critical} critical alerts require immediate attention. Key themes: {', '.join(random.sample(['supply chain disruption', 'competitor capability shift', 'certification risk', 'demand signal', 'security posture change', 'regulatory update', 'key personnel movement'], min(3, 7)))}.",
            "total_insights": n_insights,
            "critical_count": critical,
            "warning_count": warning,
            "info_count": info,
            "top_actions": top_actions,
            "regional_sections": regional_sections,
            "security_summary": {
                "total_security_insights": len(security_insights),
                "critical_security": len([s for s in security_insights if s["severity"] == "critical"]),
                "top_threats": random.sample(["Phishing domain detected", "DNS posture degradation", "Vulnerability in supplier stack", "Data breach disclosure", "Lookalike domain registration"], min(3, 5)),
                "posture_assessment": random.choice(["Stable", "Elevated", "Critical", "Improving"]),
            },
            "category_breakdown": {cat: len([i for i in insights_list if i["category"] == cat]) for cat in RECIPE_CATEGORIES},
            "full_text": f"# Weekly Intelligence Memo — W{week}/{year}\n\n## Executive Summary\n\n{critical} critical, {warning} warning, and {info} informational insights were generated this week across {len(regional_groups)} regions...\n\n## Priority Actions\n\n" + "\n".join([f"{a['priority']}. **{a['action']}** (Source: {a['source_recipe']}, Confidence: {a['confidence']:.0%})" for a in top_actions]) + "\n\n## Regional Analysis\n\n" + "\n".join([f"### {s['region_label']} ({s['insight_count']} insights)\n" + "\n".join([f"- [{i['severity'].upper()}] {i['title']}" for i in s['top_insights']]) for s in regional_sections])
        }

        system = "You are writing a weekly strategy memo for an EMS General Manager. The memo synthesizes all insights generated this week into an executive-readable format with clear priority actions, regional analysis, and security posture assessment. Be concise, specific, and actionable. Every claim must be grounded in the provided insight data."
        user = f"This week's insights (W{week}/{year}):\n{json.dumps(insights_list, ensure_ascii=False, indent=2)}"
        assistant = json.dumps(memo, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "weekly_memo_generation.jsonl"), examples)
    return examples


# ── Task 4: Company Dossier Synthesis (200 examples) ──────────────────────

def gen_company_dossier(n=200):
    examples = []
    for i in range(n):
        company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
        caps = rand_capabilities()
        certs = rand_certs()
        n_sites = random.randint(1, 6)
        regions_used = random.sample(REGIONS, min(n_sites, len(REGIONS)))

        sites = []
        for j in range(n_sites):
            r = regions_used[j % len(regions_used)]
            sites.append({
                "name": f"{company[0]} {REGION_LABELS.get(r, r)} Plant",
                "site_type": random.choice(["Plant", "Warehouse", "Office", "Lab", "Hq"]),
                "city": company[4] if j == 0 else f"Site {j+1}",
                "country": r,
                "capabilities": random.sample(caps, min(random.randint(2, 5), len(caps))),
                "certifications": [c[0] for c in random.sample(certs, min(random.randint(1, 3), len(certs)))],
            })

        coverage_score = round(random.uniform(0.3, 0.95), 2)
        risk_score = round(random.uniform(0.05, 0.85), 2)
        risk_label = "Critical" if risk_score > 0.7 else "High" if risk_score > 0.5 else "Medium" if risk_score > 0.3 else "Low" if risk_score > 0.15 else "Minimal"
        overlap_score = round(random.uniform(0.1, 0.9), 2)
        strategic_relevance = round(random.uniform(0.2, 0.95), 2)

        entity_data = {
            "company_id": uid(),
            "name": company[0],
            "ticker": company[1],
            "country": company[2],
            "company_type": company[3],
            "hq": company[4],
            "capabilities": [{"name": c, "grade": random.choice(["A", "B", "C", "D"])} for c in caps],
            "certifications": [{"name": c[0], "status": c[1]} for c in certs],
            "sites": sites,
            "employee_estimate": random.choice([500, 1000, 2500, 5000, 10000, 25000, 50000, 100000]),
            "revenue_estimate_usd": random.choice([50_000_000, 100_000_000, 500_000_000, 1_000_000_000, 5_000_000_000, 10_000_000_000]),
            "recent_events": random.sample([
                "New plant announced in Vietnam",
                "CEO appointment change",
                "Quarterly revenue beat estimates",
                "IATF 16949 audit passed",
                "Major contract win with automotive OEM",
                "Acquisition of smaller EMS company",
                "Layoffs announced (5% workforce)",
                "New capability added: EV battery management",
                "ISO 27001 certification obtained",
                "Supply chain disruption reported",
            ], random.randint(2, 5)),
        }

        dossier = {
            "id": uid(),
            "company_id": entity_data["company_id"],
            "company_name": company[0],
            "generated_at": rand_date(),
            "profile_section": {
                "name": company[0],
                "company_type": company[3],
                "country": company[2],
                "region": company[2],
                "industry_tags": random.sample(INDUSTRIES, random.randint(1, 4)),
                "employee_estimate": entity_data["employee_estimate"],
                "revenue_estimate_usd": entity_data["revenue_estimate_usd"],
                "strategic_relevance": strategic_relevance,
            },
            "capability_assessment": {
                "total_capabilities": len(caps),
                "grade_a_count": len([c for c in entity_data["capabilities"] if c["grade"] == "A"]),
                "grade_b_count": len([c for c in entity_data["capabilities"] if c["grade"] == "B"]),
                "grade_c_count": len([c for c in entity_data["capabilities"] if c["grade"] == "C"]),
                "grade_d_count": len([c for c in entity_data["capabilities"] if c["grade"] == "D"]),
                "top_capabilities": [c for c in entity_data["capabilities"] if c["grade"] in ("A", "B")][:5],
                "coverage_score": coverage_score,
            },
            "certification_analysis": {
                "total": len(certs),
                "active": len([c for c in certs if c[1] == "Active"]),
                "expired": len([c for c in certs if c[1] == "Expired"]),
                "pending": len([c for c in certs if c[1] == "Pending"]),
                "certifications": [{"name": c[0], "status": c[1]} for c in certs],
                "gaps": random.sample(["CMMC Level 2 not obtained", "NADCAP pending", "AS9100D gap for aerospace", "ISO 13485 needed for medical"], random.randint(0, 2)),
                "cert_health_score": round(random.uniform(0.5, 0.98), 2),
            },
            "risk_assessment": {
                "overall_risk": risk_score,
                "risk_label": risk_label,
                "threat_score": round(random.uniform(0.0, 0.5), 2),
                "factors": random.sample([
                    {"factor": "Single-source dependency", "severity": "High", "description": "Key components sourced from single supplier"},
                    {"factor": "Geopolitical exposure", "severity": "Medium", "description": f"Operations in {random.choice(['China', 'Taiwan', 'Israel', 'Tunisia'])} face regional risks"},
                    {"factor": "Certification expiry", "severity": "High", "description": "IATF 16949 expires in 60 days"},
                    {"factor": "Financial health", "severity": "Low", "description": "Strong balance sheet with adequate cash reserves"},
                    {"factor": "Cybersecurity posture", "severity": "Medium", "description": "DNS configuration gaps identified"},
                    {"factor": "Labor market", "severity": "Low", "description": "Stable workforce with low turnover"},
                ], random.randint(2, 4)),
            },
            "opportunity_analysis": {
                "overlap_score": overlap_score,
                "strategic_relevance": strategic_relevance,
                "opportunities": random.sample([
                    "Cross-sell advanced packaging capabilities",
                    "Propose nearshoring arrangement for {region} operations",
                    "Partner on NPI for new product line",
                    "Offer certification gap remediation consulting",
                    "Supply chain redundancy partnership",
                    "Joint venture for new market entry",
                ], random.randint(2, 4)),
                "approach_recommendation": f"{'Strategic partnership' if strategic_relevance > 0.7 else 'Competitive monitoring' if strategic_relevance > 0.4 else 'Low-priority tracking'} — {company[0]} represents a {'high-value' if overlap_score > 0.6 else 'moderate'} opportunity based on {overlap_score:.0%} capability overlap."
            },
            "site_summaries": sites,
            "competitive_position": {
                "position_label": random.choice(["Market Leader", "Strong Contender", "Niche Player", "Emerging Competitor", "Regional Leader"]),
                "strengths": random.sample(["Strong certification portfolio", "Multi-site redundancy", "Deep automotive expertise", "Cost-competitive manufacturing", "Advanced NPI capabilities", "Robust supply chain management"], random.randint(2, 4)),
                "weaknesses": random.sample(["Limited aerospace presence", "Single-region concentration", "Aging equipment", "Certification gaps", "Limited HDI capability", "No clean room"], random.randint(1, 3)),
            },
            "full_text": f"# Company Intelligence Dossier: {company[0]}\n\nGenerated: {rand_date()}\n\n## Company Profile\n\n{company[0]} is a {company[3]} headquartered in {company[4]}, {REGION_LABELS.get(company[2], company[2])}..."
        }

        system = "You are generating a company intelligence dossier for an EMS competitive intelligence platform. Synthesize all provided entity data into a structured dossier covering capabilities assessment, certification health, risk factors, opportunities, and competitive positioning. Every assessment must be grounded in the provided data — never fabricate metrics or claims."
        user = f"Entity data:\n{json.dumps(entity_data, ensure_ascii=False, indent=2)}"
        assistant = json.dumps(dossier, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "company_dossier_synthesis.jsonl"), examples)
    return examples


# ── Task 5: Narrative Rendering (500 examples) ─────────────────────────────

def gen_narrative_rendering(n=500):
    examples = []
    for i in range(n):
        pat = random.choice(RECIPE_PATTERNS)
        company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
        region = random.choice(REGIONS)
        recipe_code = random.choice(RECIPE_CODES)
        confidence = round(random.uniform(0.6, 0.99), 2)
        severity = random.choice(SEVERITIES)

        try:
            title = pat[0].format(
                entity=company[0], region=REGION_LABELS.get(region, region),
                port="Tanger Med", technology="SiP", component="MLCC",
                event="IPC APEX EXPO", industry="automotive", product="connectors"
            )[:80]
        except (KeyError, IndexError):
            title = pat[0][:80]

        evidence_items = []
        n_ev = random.randint(2, 6)
        evidence_sources = [
            ("Job posting data", f"{company[0]} posted {random.randint(5, 50)} new positions for {random.choice(['SMT operators', 'quality engineers', 'process engineers', 'supply chain analysts'])} in the last {random.choice([7, 14, 30])} days"),
            ("Certification registry", f"{company[0]} {random.choice(['ISO 9001', 'IATF 16949', 'AS9100D'])} certification {'expires in' if random.random() > 0.5 else 'was renewed on'} {rand_date()}"),
            ("Port metrics feed", f"{random.choice(['Tanger Med', 'Shanghai', 'Rotterdam', 'Haifa'])} port dwell time increased {random.randint(20, 200)}% over 90-day mean"),
            ("Web change detector", f"{company[0]} capability page updated: added {random.choice(CAPABILITIES)}"),
            ("DNS monitoring", f"New domain registered: {company[0].lower().replace(' ', '').replace('.', '')}-{'portal' if random.random() > 0.5 else 'invoice'}.com"),
            ("Commodity feed", f"{random.choice(['Copper', 'Palladium', 'Tin', 'Gold'])} price changed {random.choice(['+', '-'])}{random.randint(5, 25)}% month-over-month"),
            ("Patent database", f"{company[0]} filed {random.randint(2, 15)} new patents in {random.choice(['advanced packaging', 'thermal management', '5G', 'SiC'])}"),
            ("Tender feed", f"{random.choice(['TED', 'SAM.gov', 'TUNEPS'])} new tender: {random.choice(['PCB assembly services', 'electronic manufacturing', 'defense electronics supply'])} in {REGION_LABELS.get(region, region)}"),
        ]
        selected_evidence = random.sample(evidence_sources, min(n_ev, len(evidence_sources)))
        for src, desc in selected_evidence:
            evidence_items.append({"source": src, "description": desc, "timestamp": rand_date()})

        recipe = {
            "code": recipe_code,
            "name": title,
            "insight_template": pat[1],
            "action_template": "Alert account manager; Update risk score; Monitor for follow-up signals",
        }

        narrative = f"**{title}**\n\n{pat[1]}\n\n### Evidence\n\n" + "\n".join([f"- **{e['source']}** ({e['timestamp'][:10]}): {e['description']}" for e in evidence_items]) + f"\n\n### Impact Assessment\n\nConfidence: {confidence:.0%} | Severity: {severity.upper()}\n\nThis insight affects {company[0]} ({company[3]}, {REGION_LABELS.get(company[2], company[2])}) with implications for {random.choice(INDUSTRIES)} operations in the {REGION_LABELS.get(region, region)} region.\n\n### Recommended Actions\n\n1. {random.choice(['Alert account manager', 'Schedule review meeting', 'Update risk dashboard'])}\n2. {random.choice(['Prepare competitive positioning brief', 'Refresh entity dossier', 'Cross-reference sanctions list'])}\n3. {random.choice(['Monitor for follow-up signals (14 days)', 'Include in weekly memo', 'Trigger POI engagement workflow'])}"

        actions = [
            f"Alert {random.choice(['account manager', 'BD team', 'security team'])} for {company[0]}",
            f"Update risk score in entity profile (current: {round(random.uniform(0.1, 0.8), 2)})",
            f"Monitor for follow-up signals in next {random.choice([7, 14, 30])} days",
        ]

        insight_card = {
            "id": uid(),
            "recipe_code": recipe_code,
            "entity_id": uid(),
            "entity_name": company[0],
            "severity": severity,
            "category": random.choice(RECIPE_CATEGORIES),
            "title": title,
            "narrative": narrative,
            "actions": actions,
            "evidence": evidence_items,
            "confidence": confidence,
            "region": region,
            "generated_at": rand_date(),
        }

        system = "You are rendering an insight narrative from a recipe and its evidence slots. Produce a clear, executive-readable narrative that explains what was detected, why it matters, and what actions should be taken. Every claim must cite specific evidence. Use markdown formatting for structure."
        user = f"Recipe: {json.dumps(recipe, ensure_ascii=False)}\nEvidence: {json.dumps(evidence_items, ensure_ascii=False, indent=2)}\nEntity: {company[0]} ({company[3]}, {REGION_LABELS.get(company[2], company[2])})\nRegion: {REGION_LABELS.get(region, region)}"
        assistant = json.dumps(insight_card, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "narrative_rendering.jsonl"), examples)
    return examples


# ── Task 6: Entity Extraction (1000 examples) ──────────────────────────────

WEB_PAGE_TEMPLATES = [
    # Capability page
    "About {company}\n\n{company} is a leading {company_type} provider headquartered in {hq}. With over {employees:,} employees across {n_sites} manufacturing facilities, we deliver world-class {cap1}, {cap2}, and {cap3} services.\n\nOur certifications include {cert1}, {cert2}, and {cert3}. We serve the {ind1}, {ind2}, and {ind3} industries.\n\nKey contacts:\n- {person1}, {role1}\n- {person2}, {role2}\n\nRecent news: {company} announced {event}.",

    # Press release
    "FOR IMMEDIATE RELEASE\n\n{company} Expands Manufacturing Capabilities in {region}\n\n{hq}, {date} — {company}, a global {company_type} company, today announced the expansion of its {cap1} capabilities at its {city} facility. The investment of ${investment}M will add {cap2} and {cap3} capacity.\n\n\"{quote},\" said {person1}, {role1} of {company}.\n\nThe new facility has received {cert1} and {cert2} certifications. {company} serves major {ind1} and {ind2} OEMs worldwide.\n\nAbout {company}\n{company} (ticker: {ticker}) reported revenue of ${revenue}B in FY{year}. The company employs {employees:,} people across {n_sites} sites in {n_countries} countries.\n\nContact: {person2}, {role2}, {email}",

    # Tender document
    "TENDER NOTICE\n\nReference: {tender_ref}\nTitle: Supply of {cap1} and {cap2} Services\nIssuing Authority: {authority}\nCountry: {country}\nDeadline: {deadline}\n\nDescription: The {authority} invites qualified suppliers to submit bids for the provision of {cap1}, {cap2}, and {cap3} services for {ind1} applications.\n\nRequirements:\n- {cert1} certification mandatory\n- {cert2} preferred\n- Minimum {employees:,} employees\n- Experience in {ind1} and {ind2} sectors\n- Facility in {region} preferred\n\nContact: {person1}, {role1}, {email}\nEstimated Value: {currency}{value}M",

    # LinkedIn profile
    "{person1}\n{role1} at {company}\n{city}, {country}\n\nExperience:\n- {role1} at {company} ({year}-Present)\n- {prev_role} at {prev_company} (2018-{year})\n- Quality Manager at {company2} (2015-2018)\n\nEducation:\n- MBA, {university}\n- B.Eng Electrical Engineering\n\nCertifications: {cert1}, IPC CIS Trainer\n\nSkills: {cap1}, {cap2}, Supply Chain Management, Quality Assurance\n\nAbout: Experienced {role_family} professional with 15+ years in {ind1} electronics manufacturing. Passionate about {topic}.\n\nRecent activity: Spoke at {event} about {topic}.",

    # News article
    "{company} Wins Major {ind1} Contract Worth ${value}M\n\nBy {journalist}, {publication}\n{date}\n\n{company}, headquartered in {hq}, has secured a multi-year contract with {oem} for {cap1} and {cap2} services. The deal, valued at approximately ${value}M, will be fulfilled at {company}'s {city} facility.\n\n{person1}, {role1} of {company}, stated: \"{quote}\"\n\nThe contract requires {cert1} and {cert2} compliance. Analysts note that this positions {company} as a key supplier in the {ind1} space, competing directly with {competitor}.\n\n{company} ({ticker}) shares rose {pct}% on the news.",

    # Arabic business article
    "أعلنت شركة {company} عن توسيع مصنعها في {city}\n\nأكد {person1}، {role1} في شركة {company}، أن الشركة ستستثمر {value} مليون دولار في توسيع قدرات {cap1} و{cap2}.\n\nتمتلك الشركة شهادات {cert1} و{cert2}، وتخدم قطاعات {ind1} و{ind2}.\n\nيعمل في الشركة أكثر من {employees:,} موظف في {n_sites} مواقع.\n\nللتواصل: {person2}، {role2}، {email}",

    # French procurement notice
    "AVIS D'APPEL D'OFFRES\n\nRéférence: {tender_ref}\nObjet: Fourniture de services {cap1} et {cap2}\nPouvoir adjudicateur: {authority}\nPays: {country}\nDate limite: {deadline}\n\nLa {authority} lance un appel d'offres pour la fourniture de services de {cap1}, {cap2} et {cap3} pour des applications {ind1}.\n\nExigences:\n- Certification {cert1} obligatoire\n- {cert2} souhaitée\n- Minimum {employees:,} employés\n- Expérience dans les secteurs {ind1} et {ind2}\n\nContact: {person1}, {role1}\nValeur estimée: {value}M EUR",

    # Hebrew tech article
    "חברת {company} מרחיבה את פעילותה ב{city}\n\n{person1}, {role1} ב{company}, הכריז על השקעה של {value} מיליון דולר בהרחבת יכולות {cap1} ו-{cap2}.\n\nלחברה הסמכות {cert1} ו-{cert2}, והיא משרתת את ענפי ה{ind1} וה{ind2}.\n\n{company} מעסיקה {employees:,} עובדים ב-{n_sites} אתרים.",

    # Chinese tech article
    "{company}宣布扩大{city}工厂产能\n\n{person1}，{company}{role1}，宣布公司将投资{value}百万美元扩大{cap1}和{cap2}产能。\n\n该公司拥有{cert1}和{cert2}认证，服务于{ind1}和{ind2}行业。\n\n公司在{n_sites}个地点拥有超过{employees:,}名员工。\n\n联系人：{person2}，{role2}，{email}",
]

def gen_entity_extraction(n=1000):
    examples = []
    for i in range(n):
        company = rand_company(EMS_COMPANIES + OEM_COMPANIES)
        company2 = rand_company(EMS_COMPANIES)
        oem = rand_company(OEM_COMPANIES)
        competitor = rand_company(EMS_COMPANIES)
        region = company[2]
        caps = rand_capabilities(5)
        certs = rand_certs(3)
        person1 = rand_name(region)
        person2 = rand_name(region)
        role1 = random.choice(["CEO", "VP Operations", "Director of Quality", "Chief Procurement Officer", "Plant Manager", "VP Engineering", "CTO"])
        role2 = random.choice(["CFO", "Director of Business Development", "VP Sales", "Marketing Director", "Head of Investor Relations"])
        prev_role = random.choice(["VP Supply Chain", "Director of Operations", "Senior Quality Manager", "Engineering Director"])
        role_family = random.choice(ROLE_FAMILIES)

        template = random.choice(WEB_PAGE_TEMPLATES)
        text = template.format(
            company=company[0], company_type=company[3], hq=company[4], ticker=company[1],
            city=company[4].split(",")[0], country=REGION_LABELS.get(company[2], company[2]),
            region=REGION_LABELS.get(region, region),
            employees=random.choice([500, 1000, 5000, 10000, 50000]),
            n_sites=random.randint(2, 15), n_countries=random.randint(2, 10),
            cap1=caps[0], cap2=caps[1], cap3=caps[2],
            cert1=certs[0][0], cert2=certs[1][0], cert3=certs[2][0],
            ind1=random.choice(INDUSTRIES), ind2=random.choice(INDUSTRIES), ind3=random.choice(INDUSTRIES),
            person1=person1, person2=person2, role1=role1, role2=role2,
            prev_role=prev_role, prev_company=company2[0], company2=rand_company()[0],
            role_family=role_family.lower(),
            event=random.choice(["a new $50M plant expansion", "strategic partnership with " + oem[0], "acquisition of a test equipment company"]),
            date=rand_date()[:10], year=random.choice([2022, 2023, 2024]),
            investment=random.choice([10, 25, 50, 100, 250]),
            revenue=random.choice([0.5, 1.0, 2.5, 5.0, 10.0, 25.0]),
            quote=random.choice([
                f"This expansion reinforces our commitment to {random.choice(INDUSTRIES)} excellence",
                f"We're investing in next-generation {caps[0]} capabilities",
                f"Our customers demand {certs[0][0]}-certified quality and we deliver",
            ]),
            email=f"{person2.split()[0].lower()}@{company[0].lower().replace(' ', '').replace('.', '')}.com",
            value=random.choice([5, 10, 25, 50, 100, 250]),
            currency=random.choice(["$", "€", "£"]),
            pct=random.choice([2, 3, 5, 7, 12]),
            tender_ref=f"TED-{random.randint(100000, 999999)}-{random.choice(['2024', '2025'])}",
            authority=random.choice(["Ministry of Defence", "Agence Tunisienne de l'Investissement", "Direction Générale de l'Armement", "Israel Ministry of Defense (SIBAT)", "European Defence Agency"]),
            deadline=rand_date()[:10],
            journalist=rand_name("US"),
            publication=random.choice(["EMS Now", "Circuits Assembly", "DigiTimes", "Reuters", "Bloomberg"]),
            oem=oem[0], competitor=competitor[0],
            university=random.choice(["INSAT Tunis", "Technion", "TU Munich", "École Polytechnique", "MIT", "Tsinghua University"]),
            topic=random.choice(["supply chain resilience", "Industry 4.0", "quality excellence", "digital transformation", "sustainability"]),
        )

        extracted = {
            "companies": [
                {"name": company[0], "type": company[3], "country": company[2], "ticker": company[1]},
            ],
            "persons": [
                {"name": person1, "role": role1, "organization": company[0], "role_family": role_family},
                {"name": person2, "role": role2, "organization": company[0], "role_family": "Finance" if "CF" in role2 else "Executive"},
            ],
            "capabilities": [{"name": c, "entity": company[0]} for c in caps[:3]],
            "certifications": [{"name": c[0], "status": c[1], "entity": company[0]} for c in certs],
            "locations": [{"city": company[4].split(",")[0], "country": REGION_LABELS.get(company[2], company[2])}],
            "industries": random.sample(INDUSTRIES, 2),
        }

        # Add referenced companies
        if "oem" in template.lower() or "competitor" in template.lower():
            extracted["companies"].append({"name": oem[0], "type": "Oem", "country": oem[2]})
        if "prev_company" in template:
            extracted["companies"].append({"name": company2[0], "type": company2[3], "country": company2[2]})

        system = "Extract structured entities from this text. Identify all companies, persons (with roles), capabilities, certifications, locations, and industry sectors. Return valid JSON. Only extract entities that are explicitly mentioned in the text — never infer or fabricate."
        user = text
        assistant = json.dumps(extracted, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "entity_extraction.jsonl"), examples)
    return examples


# ── Task 7: Competitive Analysis (100 examples) ───────────────────────────

def gen_competitive_analysis(n=100):
    examples = []
    starz = EMS_COMPANIES[21]  # Starz Electronics
    for i in range(n):
        competitor = rand_company(EMS_COMPANIES[:20] + EMS_COMPANIES[22:])
        starz_caps = rand_capabilities(random.randint(5, 10))
        comp_caps = rand_capabilities(random.randint(5, 10))
        starz_certs = rand_certs(random.randint(3, 7))
        comp_certs = rand_certs(random.randint(3, 7))

        starz_profile = {
            "name": starz[0],
            "country": starz[2],
            "company_type": starz[3],
            "hq": starz[4],
            "capabilities": starz_caps,
            "certifications": [c[0] for c in starz_certs],
            "employee_estimate": random.choice([200, 500, 1000]),
            "revenue_estimate_usd": random.choice([20_000_000, 50_000_000, 100_000_000]),
            "regions_served": random.sample(["TN", "MA", "FR", "DE", "IL"], random.randint(2, 4)),
            "key_industries": random.sample(INDUSTRIES, random.randint(2, 4)),
        }

        comp_profile = {
            "name": competitor[0],
            "country": competitor[2],
            "company_type": competitor[3],
            "hq": competitor[4],
            "capabilities": comp_caps,
            "certifications": [c[0] for c in comp_certs],
            "employee_estimate": random.choice([1000, 5000, 10000, 50000, 100000]),
            "revenue_estimate_usd": random.choice([100_000_000, 500_000_000, 1_000_000_000, 5_000_000_000]),
            "regions_served": random.sample(REGIONS, random.randint(3, 8)),
            "key_industries": random.sample(INDUSTRIES, random.randint(2, 5)),
        }

        shared_caps = set(starz_caps) & set(comp_caps)
        starz_unique = set(starz_caps) - set(comp_caps)
        comp_unique = set(comp_caps) - set(starz_caps)

        shared_certs = set(c[0] for c in starz_certs) & set(c[0] for c in comp_certs)
        starz_cert_unique = set(c[0] for c in starz_certs) - set(c[0] for c in comp_certs)
        comp_cert_unique = set(c[0] for c in comp_certs) - set(c[0] for c in starz_certs)

        analysis = {
            "comparison_id": uid(),
            "starz_entity": starz[0],
            "competitor_entity": competitor[0],
            "generated_at": rand_date(),
            "capability_comparison": {
                "shared_capabilities": list(shared_caps),
                "starz_unique": list(starz_unique),
                "competitor_unique": list(comp_unique),
                "overlap_score": round(len(shared_caps) / max(len(set(starz_caps) | set(comp_caps)), 1), 2),
            },
            "certification_comparison": {
                "shared": list(shared_certs),
                "starz_unique": list(starz_cert_unique),
                "competitor_unique": list(comp_cert_unique),
            },
            "scale_comparison": {
                "starz_employees": starz_profile["employee_estimate"],
                "competitor_employees": comp_profile["employee_estimate"],
                "starz_revenue": starz_profile["revenue_estimate_usd"],
                "competitor_revenue": comp_profile["revenue_estimate_usd"],
                "size_ratio": round(comp_profile["employee_estimate"] / max(starz_profile["employee_estimate"], 1), 1),
            },
            "advantages": random.sample([
                f"Geographic proximity to {random.choice(['European', 'North African', 'Middle Eastern'])} customers",
                f"Cost-competitive labor in {REGION_LABELS.get(starz[2], starz[2])}",
                f"Unique capability: {list(starz_unique)[0] if starz_unique else starz_caps[0]}",
                "Agility and responsiveness for NPI/prototype",
                f"Free zone benefits in {REGION_LABELS.get(starz[2], starz[2])}",
                "Strong French-language capability for Francophone clients",
                "Dual Arabic/French/English language support",
                "Nearshoring alternative to Asian manufacturing",
            ], random.randint(2, 4)),
            "gaps": random.sample([
                f"Scale gap: {competitor[0]} is {round(comp_profile['employee_estimate'] / max(starz_profile['employee_estimate'], 1), 0):.0f}x larger",
                f"Missing capability: {list(comp_unique)[0] if comp_unique else comp_caps[0]}",
                f"Missing certification: {list(comp_cert_unique)[0] if comp_cert_unique else 'NADCAP'}",
                f"Limited {random.choice(['Asian', 'American', 'South Asian'])} presence vs {competitor[0]}",
                f"Revenue gap: {competitor[0]} revenue is {round(comp_profile['revenue_estimate_usd'] / max(starz_profile['revenue_estimate_usd'], 1), 0):.0f}x higher",
            ], random.randint(2, 4)),
            "recommendations": [
                f"Focus competitive messaging on {random.choice(['nearshoring value', 'cost advantage', 'agility', 'multilingual support', 'regional expertise'])}",
                f"Invest in {list(comp_unique)[0] if comp_unique else random.choice(CAPABILITIES)} to close capability gap",
                f"Pursue {list(comp_cert_unique)[0] if comp_cert_unique else 'NADCAP'} certification to match {competitor[0]}",
                f"Target {random.choice(['French', 'MENA', 'European'])} clients where {competitor[0]}'s reach is weaker",
                f"Leverage {random.choice(['free zone incentives', 'bilateral trade agreements', 'EU association agreement'])} for cost positioning",
            ],
            "win_probability": round(random.uniform(0.15, 0.65), 2),
            "strategic_recommendation": random.choice([
                f"Direct competition viable in {random.choice(INDUSTRIES)} segment where scale is less critical",
                f"Avoid head-to-head; position as regional specialist and nearshore partner",
                f"Partner strategy: propose as {competitor[0]}'s regional sub-contractor",
                f"Niche focus: target {random.choice(INDUSTRIES)} applications where Starz capabilities are strongest",
            ]),
        }

        system = "You are comparing Starz Electronics capabilities versus a competitor for strategic positioning. Produce a structured competitive analysis including capability overlap, certification gaps, scale comparison, competitive advantages, and actionable recommendations. Base all analysis solely on provided profile data."
        user = f"Starz profile:\n{json.dumps(starz_profile, ensure_ascii=False, indent=2)}\n\nCompetitor profile:\n{json.dumps(comp_profile, ensure_ascii=False, indent=2)}"
        assistant = json.dumps(analysis, ensure_ascii=False, indent=2)

        examples.append({"messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]})

    jsonl_write(os.path.join(DEST, "competitive_analysis.jsonl"), examples)
    return examples


# ── Main ────────────────────────────────────────────────────────────────────

def main():
    print(f"Generating instruction fine-tuning examples in {DEST}/\n")

    total = 0
    for name, gen_fn, count in [
        ("recipe_hypothesis_generation", gen_recipe_hypothesis, 500),
        ("poi_synthesis", gen_poi_synthesis, 300),
        ("weekly_memo_generation", gen_weekly_memo, 100),
        ("company_dossier_synthesis", gen_company_dossier, 200),
        ("narrative_rendering", gen_narrative_rendering, 500),
        ("entity_extraction", gen_entity_extraction, 1000),
        ("competitive_analysis", gen_competitive_analysis, 100),
    ]:
        print(f"\n--- {name} ({count} examples) ---")
        examples = gen_fn(count)
        total += len(examples)

    print(f"\n{'='*60}")
    print(f"Total: {total} instruction fine-tuning examples")
    total_bytes = sum(os.path.getsize(os.path.join(DEST, f)) for f in os.listdir(DEST) if f.endswith(".jsonl"))
    print(f"Total size: {total_bytes:,} bytes ({total_bytes/1024/1024:.1f} MB)")
    print(f"\nFiles:")
    for f in sorted(os.listdir(DEST)):
        if f.endswith(".jsonl"):
            fp = os.path.join(DEST, f)
            lines = sum(1 for _ in open(fp))
            sz = os.path.getsize(fp)
            print(f"  {f}: {lines} examples, {sz:,}b")


if __name__ == "__main__":
    main()
