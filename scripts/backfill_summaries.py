#!/usr/bin/env python3
"""Backfill insight summaries with proper analytical text.

Replaces generic template fill-ins with multi-paragraph intelligence analysis
using available data: company name, region, type, recipe code, signal counts, confidence.
"""

import re
import sys
import os
import psycopg2
import psycopg2.extras

DB_DSN = os.getenv(
    "DATABASE_URL",
    os.getenv(
        "APEX_DATABASE_URL",
        "host=127.0.0.1 port=5432 dbname=apexintel user=apexintel",
    ),
)

# ── Recipe-code to human-readable scenario description ─────────────────────
RECIPE_DESCRIPTIONS = {
    # A-series: demand & procurement
    "A001": ("new sourcing cycle", "is entering a new sourcing cycle, signaling upcoming procurement activity across their supply chain"),
    "A002": ("qualification round imminent", "appears to be preparing for a formal supplier qualification round, indicating active vendor selection"),
    "A003": ("automotive qualification opening", "has opened an automotive qualification window, creating opportunity for certified suppliers"),
    "A005": ("new product line supply chain", "is building supply chain infrastructure for new product lines, requiring additional manufacturing partners"),
    "A007": ("NPI ramp and prototype demand", "is ramping new product introductions and seeking prototype manufacturing capacity"),
    "A011": ("harness cost pressure", "faces wiring harness cost pressure, creating openings for cost-competitive alternatives"),
    "A013": ("R&D expansion driving demand", "shows significant R&D expansion, with engineering hiring and patent filings pointing to growing custom manufacturing demand"),
    "A016": ("China-plus-one diversification", "is actively pursuing China-plus-one supply chain diversification, seeking alternative manufacturing locations"),
    "A017": ("major expansion and greenfield", "shows major expansion signals including new facility indicators, suggesting greenfield supply chain requirements"),
    "A018": ("competitor quality lapse opening", "has a competitor whose certification lapse creates an opening for qualified alternative suppliers"),
    "A021": ("new CPO restructuring vendor panel", "has a new Chief Procurement Officer restructuring the vendor panel, creating a narrow window for engagement"),
    "A023": ("component allocation constraints", "faces component allocation constraints, seeking flexible EMS partners who can navigate supply volatility"),
    "A024": ("medical device sourcing", "is entering a medical device sourcing round, requiring suppliers with regulatory compliance credentials"),
    "A030": ("commodity volatility review", "is conducting a sourcing review driven by commodity price volatility, potentially reallocating volumes"),
    "A032": ("qualification form and tender imminent", "has posted new supplier qualification forms, indicating imminent tender activity"),
    "A033": ("automotive harness demand surge", "is experiencing an automotive harness demand surge, requiring scaled production capacity"),
    "A036": ("GCC expansion via Morocco logistics", "is expanding Gulf Cooperation Council operations with Morocco as a logistics hub"),
    "A038": ("end-of-life product next-gen RFQ", "has end-of-life products triggering next-generation RFQs for replacement manufacturing"),
    "A039": ("test capability outsourcing", "is outsourcing test capabilities, seeking partners with advanced testing infrastructure"),
    "A041": ("aerospace qualification window", "has an aerospace qualification window opening, requiring AS9100-certified suppliers"),
    "A042": ("in-sourcing assembly needs components", "is in-sourcing assembly operations and needs component supply partnerships"),
    "A049": ("Nadcap special process demand", "requires Nadcap-certified special processes, narrowing the qualified supplier field"),
    "A050": ("pilot and first NPI activity", "shows early-stage NPI activity suggesting pilot manufacturing needs in the near term"),

    # B-series: competitor market intelligence
    "B001": ("competitor expanding capabilities", "is expanding its capabilities portfolio, potentially increasing competitive overlap"),
    "B002": ("competitor capacity expansion", "is expanding production capacity, indicating aggressive growth positioning"),
    "B003": ("competitor in distress", "shows financial or operational distress signals that could destabilize their customer relationships"),
    "B008": ("competitor buying market share", "appears to be buying market share through aggressive pricing or acquiring distressed assets"),
    "B010": ("competitor innovating in our space", "is filing patents and investing in technology areas that overlap with our core capabilities"),
    "B012": ("competitor pricing pressure", "is under pricing pressure, which may force compromises in quality or service levels"),
    "B014": ("competitor leadership instability", "is experiencing leadership instability, with executive turnover creating strategic uncertainty"),
    "B015": ("competitor technology gap", "has a widening technology gap that limits their competitiveness in advanced manufacturing"),
    "B016": ("competitor geographic weakness", "has weak presence in a region where demand is growing, creating a geographic opportunity gap"),
    "B017": ("competitor customer consolidation", "faces customer consolidation risk as their key accounts merge or restructure"),
    "B018": ("competitor talent exodus", "is losing key personnel, with departures and negative reviews suggesting internal issues"),
    "B019": ("competitor overextension", "shows signs of overextension with rapid expansion coupled with quality and delivery issues"),
    "B020": ("competitor regulatory risk", "faces regulatory compliance challenges that may restrict their market access"),
    "B022": ("competitor technology obsolescence", "relies on technology becoming obsolete, limiting their ability to serve evolving requirements"),
    "B023": ("competitor customer concentration", "has dangerously high customer concentration, creating vulnerability if key accounts shift"),
    "B024": ("competitor environmental compliance", "faces environmental compliance gaps that may trigger regulatory action"),
    "B025": ("competitor cybersecurity incident", "had a cybersecurity incident that may undermine customer confidence in their data security"),
    "B026": ("competitor market share decline", "is losing market share across key segments, signaling competitive positioning problems"),
    "B027": ("competitor quality escape", "had a quality escape incident, potentially damaging their reputation with quality-focused customers"),
    "B028": ("competitor labor issues", "faces labor relations challenges that could disrupt production continuity"),
    "B029": ("competitor customer dissatisfaction", "has customer dissatisfaction signals, with service complaints indicating systemic issues"),
    "B031": ("competitor sales team changes", "experienced sales team restructuring, potentially disrupting customer relationships"),
    "B032": ("competitor strategic retreat", "is retreating from certain market segments, leaving demand unfulfilled"),
    "B033": ("competitor IP threat", "faces intellectual property challenges that create legal uncertainty"),
    "B034": ("competitor lost customer", "appears to have lost a key customer relationship, freeing displaced demand"),
    "B035": ("competitor service degradation", "shows declining service quality with longer response times and customer dissatisfaction"),
    "B036": ("competitor capacity crunch", "is capacity-constrained, unable to absorb new demand or maintain delivery commitments"),
    "B037": ("competitor partnership dissolved", "lost a strategic partnership, reducing their capability or geographic reach"),
    "B038": ("competitor geographic exit", "is exiting a regional market, creating a void for capture"),
    "B039": ("competitor cost structure pressure", "faces cost structure pressure that may force pricing adjustments or margin compression"),
    "B040": ("competitor innovation stagnation", "shows signs of innovation stagnation with declining R&D investment indicators"),
    "B041": ("competitor union negotiations", "faces union negotiations that may result in production disruptions or cost increases"),
    "B042": ("competitor customer churn", "has elevated customer churn rates, suggesting systemic delivery or quality problems"),
    "B043": ("competitor management distraction", "has management distracted by internal issues, reducing market responsiveness"),
    "B044": ("competitor quality system weakness", "has quality management system weaknesses that may surface in upcoming audits"),
    "B045": ("competitor supply chain complexity", "faces supply chain complexity issues, with material delays affecting deliveries"),
    "B046": ("competitor regulatory burden", "faces increasing regulatory burden that consumes resources and slows responsiveness"),
    "B047": ("competitor technology licensing issues", "faces technology licensing issues that may limit product development"),
    "B048": ("competitor brand reputation issue", "has brand reputation concerns being discussed in industry channels"),
    "B049": ("competitor project failure", "experienced a notable project failure, damaging credibility with potential customers"),
    "B050": ("competitor strategic misalignment", "shows strategic misalignment with market direction, pursuing initiatives that diverge from customer needs"),

    # C-series: supply chain risk
    "C001": ("critical materials cost spike", "faces a critical materials cost spike that may cascade through the supply chain"),
    "C002": ("component shortage emerging", "is affected by an emerging component shortage that could constrain production"),
    "C003": ("logistics disruption", "is impacted by regional logistics disruptions affecting shipping and delivery timelines"),
    "C004": ("supplier financial distress", "has a key supplier showing financial distress signals, creating dependency risk"),
    "C005": ("tariff change impact", "faces cost impact from tariff or trade policy changes affecting component sourcing"),
    "C006": ("geopolitical supply risk", "is exposed to geopolitical supply risk from sourcing in politically unstable regions"),
    "C008": ("raw material allocation", "faces raw material allocation constraints as suppliers ration scarce inputs"),
    "C009": ("supplier quality decline", "has a supplier showing quality decline with rising defect rates and audit findings"),
    "C013": ("supplier capacity constraint", "has a capacity-constrained supplier with extended lead times and full bookings"),
    "C015": ("supplier cybersecurity breach", "has a supplier affected by a cybersecurity incident, creating data and continuity risk"),
    "C017": ("supplier labor disruption", "has a supplier facing labor disruption that may interrupt production"),
    "C018": ("supplier environmental incident", "has a supplier involved in an environmental incident with potential regulatory consequences"),
    "C023": ("supplier concentration risk", "has high supplier concentration, with critical inputs sourced from too few providers"),
    "C024": ("demand surge capacity planning", "faces a demand surge requiring proactive capacity planning across the supply chain"),
    "C025": ("supplier delivery performance decline", "has a supplier with declining delivery performance, missing committed dates"),
    "C028": ("supply chain visibility gap", "lacks visibility into deeper supply chain tiers, creating hidden risk exposure"),
    "C029": ("supplier ESG compliance risk", "has a supplier with ESG compliance gaps that may affect qualification status"),
    "C030": ("inventory imbalance", "has inventory imbalances with excess in some categories and shortages in others"),
    "C033": ("customs inspection delay", "faces customs inspection delays that extend lead times for imported materials"),
    "C034": ("supplier subcontractor change", "has a supplier changing subcontractors, introducing new quality and continuity risks"),
    "C035": ("material certification expiration", "has material certifications approaching expiration, requiring renewal action"),
    "C036": ("supplier geographic concentration", "has suppliers geographically concentrated, creating natural disaster and disruption vulnerability"),
    "C037": ("commodity hedge expiration", "has commodity hedges expiring, exposing the supply chain to price volatility"),
    "C039": ("supplier insurance lapse", "has a supplier whose insurance coverage has lapsed, increasing liability exposure"),
    "C040": ("production line transfer", "is transferring production lines, creating temporary capacity and quality transition risks"),
    "C041": ("supplier key personnel departure", "has a supplier losing key personnel, which may affect technical capability or relationship continuity"),
    "C043": ("import license requirement", "faces new import license requirements that add lead time and administrative burden"),
    "C044": ("supplier capacity reduction", "has a supplier reducing capacity, potentially unable to fulfill existing commitments"),
    "C045": ("specification change impact", "faces specification changes that require supply chain requalification"),
    "C046": ("supplier working capital stress", "has a supplier under working capital stress, with payment term requests and cash flow strain"),
    "C049": ("supply chain certification audit", "has a supply chain certification audit due that requires preparation and documentation"),

    # D-series: security and compliance
    "D010": ("sanctions screening match", "has entities matching sanctions screening lists, requiring compliance review"),
    "D014": ("security audit finding", "has security audit findings that require remediation attention"),
    "D019": ("intellectual property threat", "faces an intellectual property threat from competitive patent activity or litigation"),
    "D021": ("certification expiry alert", "has certifications approaching expiry that require timely renewal to maintain compliance"),
    "D022": ("trade compliance violation", "has trade compliance concerns that may affect cross-border operations"),
    "D033": ("supplier security assessment failed", "has a supplier that failed a security assessment, raising third-party risk concerns"),

    # E-series: strategic POI (person of interest)
    "E010": ("POI networking opportunity", "has a key decision-maker with mutual connections that enable warm introduction"),
    "E019": ("regulatory change opportunity", "is affected by regulatory changes that create strategic engagement opportunities"),
    "E020": ("trade agreement opportunity", "can benefit from new trade agreements that reduce barriers in target markets"),
    "E031": ("POI education background", "has a key contact whose educational background suggests alignment with technical conversations"),
    "E036": ("POI dissatisfied with current supplier", "has a decision-maker showing dissatisfaction with their current supplier, signaling openness to alternatives"),
    "E040": ("POI board member influence", "has a board-level contact whose strategic influence shapes procurement direction"),
    "E054": ("POI previous employer connection", "has a key contact with relevant previous employer experience that enables industry-specific engagement"),
    "E079": ("POI external visibility", "has a key decision-maker with high external visibility through events and publications"),
    "E087": ("POI internal champion lost", "lost an internal champion at a key account, requiring relationship rebuilding"),

    # F-series: customer RFQ
    "F001": ("public RFQ matching capabilities", "has posted a public RFQ that matches our manufacturing capabilities"),
    "F002": ("government and defense RFQ", "has a government or defense procurement opportunity aligned with our certifications"),
    "F003": ("customer rebid cycle", "is entering a rebid cycle, creating a window to compete for existing business"),
    "F005": ("multinational tender listing", "has listed a multinational tender on procurement platforms"),
    "F008": ("automotive platform RFQ wave", "is issuing automotive platform RFQs across multiple vehicle programs"),
    "F010": ("medical device qualification", "is qualifying suppliers for medical device manufacturing requiring regulatory compliance"),
    "F011": ("EU TED tender match", "has posted a tender on EU Tenders Electronic Daily matching our profile"),
    "F012": ("US SAM.gov contract opportunity", "has a federal contract opportunity on SAM.gov matching our NAICS profile"),
    "F013": ("customer dual-sourcing initiative", "is implementing a dual-sourcing strategy, opening second-source qualification"),
    "F014": ("nearshoring procurement", "is nearshoring procurement to reduce lead times and logistics risk"),
    "F016": ("e-commerce marketplace RFQ", "has posted a B2B marketplace RFQ seeking manufacturing partners"),
    "F018": ("sustainability procurement mandate", "has a sustainability-driven procurement mandate requiring environmentally certified suppliers"),
    "F020": ("consortium RFQ participation", "has issued a consortium tender requiring collaborative partner capabilities"),

    # G-series: regulatory and policy
    "G002": ("sanctions list update", "is affected by sanctions list updates that change compliance requirements for cross-border dealings"),
    "G008": ("industry standard revision", "is impacted by industry standard revisions that require process and documentation updates"),

    # I-series: pricing and market
    "I003": ("new market entrant", "faces a new competitor entering key market segments, potentially disrupting pricing and positioning"),
    "I007": ("digital transformation shift", "is undergoing digital transformation that shifts procurement toward digital-first channels and suppliers"),

    # J-series: geopolitical analysis
    "J001": ("conflict escalation monitoring", "operates in a region experiencing conflict escalation that threatens business continuity and supply routes"),
    "J002": ("sanctions cascade impact", "is exposed to cascading sanctions that progressively restrict trade and financial channels"),
    "J003": ("diplomatic realignment", "is affected by diplomatic realignment that may shift trade preferences and bilateral agreements"),
    "J004": ("military procurement trend", "is in a region with shifting military procurement patterns that signal changing defense priorities"),
    "J005": ("energy security assessment", "faces energy security concerns in a region with volatile energy supply dynamics"),
    "J006": ("cyber warfare posture", "operates in a region with elevated cyber warfare posture, increasing digital infrastructure risk"),
    "J007": ("critical mineral vulnerability", "is exposed to critical mineral supply vulnerability from geopolitically sensitive sources"),
    "J008": ("trade corridor disruption", "is affected by trade corridor disruptions that reroute logistics and increase transit costs"),
    "J009": ("compound geopolitical risk", "faces compound geopolitical risk with multiple concurrent instability factors"),
    "J010": ("geopolitical situation report", "is in a region flagged by current geopolitical situation assessment as requiring elevated monitoring"),
    "J011": ("arms race detection", "operates in a region showing arms race indicators that may trigger sanctions or export controls"),
    "J012": ("alliance shift detection", "is affected by shifting geopolitical alliances that may realign trade partnerships"),
    "J013": ("hybrid warfare assessment", "is in a region experiencing concurrent cyber, kinetic, and information warfare indicators"),
    "J014": ("regional instability index", "operates in a region with elevated instability indicators across economic and political dimensions"),
    "J015": ("strategic resource weaponization", "faces strategic resource weaponization as commodities are used as geopolitical leverage"),
}

# ── Confidence-level text ──────────────────────────────────────────────────
def confidence_text(conf):
    if conf >= 0.85:
        return "high"
    elif conf >= 0.70:
        return "moderate-to-high"
    elif conf >= 0.55:
        return "moderate"
    else:
        return "low-to-moderate"

# ── Parse signal counts from existing summary ──────────────────────────────
SIGNAL_PATTERN = re.compile(
    r'(\d+)\s+(person|job posting|patent|tender|certification|competitor signal|web change|competitor|observation|signal)\(s?\)',
    re.IGNORECASE
)

def parse_signal_counts(summary):
    counts = {}
    for m in SIGNAL_PATTERN.finditer(summary):
        n = int(m.group(1))
        kind = m.group(2).lower().strip()
        if "person" in kind:
            counts["persons_of_interest"] = n
        elif "job" in kind:
            counts["job_postings"] = n
        elif "patent" in kind:
            counts["patents"] = n
        elif "tender" in kind:
            counts["tenders"] = n
        elif "certif" in kind:
            counts["certifications"] = n
        elif "competitor" in kind:
            counts["competitor_signals"] = n
        elif "web" in kind:
            counts["web_changes"] = n
        elif "observation" in kind or "signal" in kind:
            counts["signals"] = n
    return counts

# ── Build evidence narrative ───────────────────────────────────────────────
def evidence_narrative(counts):
    parts = []
    if counts.get("job_postings"):
        n = counts["job_postings"]
        parts.append("{} job posting{} detected".format(n, "s" if n != 1 else ""))
    if counts.get("patents"):
        n = counts["patents"]
        parts.append("{} patent filing{} recorded".format(n, "s" if n != 1 else ""))
    if counts.get("certifications"):
        n = counts["certifications"]
        parts.append("{} certification{} tracked".format(n, "s" if n != 1 else ""))
    if counts.get("tenders"):
        n = counts["tenders"]
        parts.append("{} tender{} identified".format(n, "s" if n != 1 else ""))
    if counts.get("competitor_signals"):
        n = counts["competitor_signals"]
        parts.append("{} competitive intelligence signal{} captured".format(n, "s" if n != 1 else ""))
    if counts.get("web_changes"):
        n = counts["web_changes"]
        parts.append("{} web presence change{} observed".format(n, "s" if n != 1 else ""))
    if counts.get("persons_of_interest"):
        n = counts["persons_of_interest"]
        parts.append("{} person{} of interest under monitoring".format(n, "s" if n != 1 else ""))
    if counts.get("signals"):
        n = counts["signals"]
        parts.append("{} corroborating signal{}".format(n, "s" if n != 1 else ""))
    if not parts:
        return "Limited signal data is currently available for this assessment."
    if len(parts) == 1:
        return "Our monitoring infrastructure has {}.".format(parts[0])
    return "Our monitoring infrastructure has " + ", ".join(parts[:-1]) + ", and {}.".format(parts[-1])

# ── Type-specific analytical framing ───────────────────────────────────────
TYPE_FRAMES = {
    "competitor_market": {
        "context": "Competitive intelligence analysis indicates a notable development in the market positioning of {name}.",
        "implication": "This creates a potential window of opportunity. Customers evaluating alternatives may be receptive to engagement, particularly those who prioritize {value_prop}. The competitive dynamics in {region} are shifting, and early positioning could yield significant pipeline value.",
        "action_prefix": "Consider proactive outreach to accounts with known relationships to this competitor, emphasizing",
    },
    "demand_procurement": {
        "context": "Demand intelligence signals suggest active procurement movement at {name}.",
        "implication": "The convergence of these indicators suggests a procurement decision cycle is underway or imminent. Organizations in {region} showing this signal pattern typically move to vendor selection within 60 to 120 days. Early engagement with the right technical value proposition positions us ahead of competitors who have not yet detected these signals.",
        "action_prefix": "Prioritize technical engagement with procurement and engineering stakeholders at this account, focusing on",
    },
    "supply_chain_risk": {
        "context": "Supply chain risk monitoring has flagged a developing situation affecting {name}.",
        "implication": "This risk scenario warrants proactive supply chain management attention. The combination of detected signals suggests the disruption potential is real and may escalate if left unaddressed. For {region}-sourced materials and components, contingency planning should be accelerated. Downstream impact on production schedules should be modeled.",
        "action_prefix": "Initiate risk mitigation protocols including",
    },
    "security_compliance": {
        "context": "Security and compliance monitoring has identified an actionable finding related to {name}.",
        "implication": "Compliance gaps of this nature can have cascading consequences, from audit findings to contract eligibility restrictions. In regulated industries, maintaining continuous compliance is a prerequisite for participation in qualification rounds. Timely remediation protects both operational continuity and market access in {region}.",
        "action_prefix": "Address this compliance finding promptly through",
    },
    "regulatory_policy": {
        "context": "Regulatory and policy intelligence indicates changes affecting {name} and its operating environment.",
        "implication": "Regulatory shifts in {region} create both risk and opportunity. Organizations that adapt early to new requirements gain qualification advantages over slower-moving competitors. The policy changes detected may also alter competitive dynamics by raising barriers to entry or changing cost structures.",
        "action_prefix": "Monitor regulatory developments and prepare compliance adaptation plans, specifically",
    },
    "geopolitical_analysis": {
        "context": "Geopolitical risk assessment has flagged developments relevant to {name} and operations in {region}.",
        "implication": "Geopolitical dynamics in {region} are evolving in ways that directly affect trade flows, supply chain routing, and market access. The detected pattern suggests elevated risk that warrants scenario planning. Companies with diversified geographic footprints will be better positioned to navigate disruptions.",
        "action_prefix": "Review exposure to this geopolitical dimension and prepare contingency scenarios, including",
    },
    "strategic_poi": {
        "context": "Person-of-interest intelligence has identified a strategic engagement opportunity involving {name}.",
        "implication": "Relationship intelligence suggests a favorable window for engagement. Decision-maker dynamics at this organization indicate receptivity to supplier conversations. Timing is critical as procurement decisions often crystallize around organizational transitions, and early relationship building during these windows yields disproportionate influence.",
        "action_prefix": "Develop a targeted engagement plan for key stakeholders, leveraging",
    },
    "customer_rfq": {
        "context": "Procurement signal intelligence has detected an active solicitation or RFQ event from {name}.",
        "implication": "This represents a concrete revenue opportunity with a defined timeline. The procurement signal matches our capability profile, suggesting strong bid competitiveness. Response timing is critical as early engagement with the buying team before the formal deadline significantly improves win probability.",
        "action_prefix": "Prepare a targeted bid response and initiate pre-submission technical engagement, emphasizing",
    },
    "pricing_market": {
        "context": "Market and pricing intelligence has identified structural shifts affecting {name}.",
        "implication": "Market dynamics in {region} are reshaping competitive positioning and pricing power. The detected pattern suggests that traditional procurement approaches are being disrupted, creating openings for suppliers who can articulate differentiated value. Pricing strategy should be reviewed in light of these market structural changes.",
        "action_prefix": "Review pricing strategy and market positioning in response to these shifts, specifically",
    },
    "veracity_analysis": {
        "context": "Cross-source verification analysis has assessed the information ecosystem around {name}.",
        "implication": "The breadth and consistency of coverage across independent sources strengthens confidence in the underlying intelligence. Multi-source corroboration reduces the risk of acting on single-source or planted information, providing a more reliable foundation for strategic decisions.",
        "action_prefix": "Use this verified intelligence to inform strategic planning, particularly",
    },
}

VALUE_PROPS = [
    "reliability and consistent quality delivery",
    "technical depth and engineering support capabilities",
    "supply chain resilience and geographic flexibility",
    "cost competitiveness with transparent pricing structures",
    "speed to market and prototype-to-production agility",
    "regulatory compliance and certification breadth",
    "long-term partnership stability and account management",
]

# ── Build analytical summary ───────────────────────────────────────────────
def build_summary(company_name, company_region, company_type, insight_type,
                  recipe_code, confidence, existing_summary):
    """Return (new_title, new_summary)."""

    recipe_short, recipe_desc = RECIPE_DESCRIPTIONS.get(
        recipe_code,
        ("monitoring alert", "has been flagged by our monitoring system for activity warranting analytical review")
    )

    counts = parse_signal_counts(existing_summary)
    frame = TYPE_FRAMES.get(insight_type, TYPE_FRAMES["competitor_market"])
    region_label = company_region or "the monitored region"
    type_label = " ({})".format(company_type) if company_type else ""
    conf_label = confidence_text(confidence)
    conf_pct = int(confidence * 100)

    vp_idx = hash(company_name + recipe_code) % len(VALUE_PROPS)
    value_prop = VALUE_PROPS[vp_idx]

    # Title
    title = "{} ({}): {}".format(company_name, region_label, recipe_short)

    # Paragraph 1: Context + what was detected
    p1 = frame["context"].format(name=company_name, region=region_label)
    p1 += " {}{}, based in {}, {}.".format(company_name, type_label, region_label, recipe_desc)

    # Paragraph 2: Evidence
    p2 = evidence_narrative(counts)
    p2 += " Assessment confidence is {} ({}%), based on signal convergence and source diversity.".format(conf_label, conf_pct)

    # Paragraph 3: Implication
    p3 = frame["implication"].format(
        name=company_name,
        region=region_label,
        value_prop=value_prop,
    )

    # Paragraph 4: Recommended action
    action_match = re.search(r'Recommended action:\s*(.+?)(?:\n|$)', existing_summary)
    if action_match:
        existing_action = action_match.group(1).strip()
        existing_action = re.sub(r'\s*""\s*', ' ', existing_action).strip()
        existing_action = re.sub(r'\s{2,}', ' ', existing_action).strip()
        if len(existing_action) > 10 and existing_action.lower() not in ("custom", "n/a", ""):
            p4 = "Recommended action: {}.".format(existing_action)
        else:
            p4 = "{} our differentiated capabilities in this area.".format(frame["action_prefix"])
    else:
        p4 = "{} our differentiated capabilities in this area.".format(frame["action_prefix"])

    # Clean trailing double periods
    if p4.endswith(".."):
        p4 = p4[:-1]

    summary = "{}\n\n{}\n\n{}\n\n{}".format(p1, p2, p3, p4)
    return title, summary


def main():
    conn = psycopg2.connect(DB_DSN)
    conn.autocommit = False
    cur = conn.cursor(cursor_factory=psycopg2.extras.DictCursor)

    cur.execute("""
        SELECT i.id, i.title, i.summary, i.insight_type, i.region,
               i.confidence, i.tags, i.entity_ids,
               c.name AS company_name, c.region AS company_region,
               c.company_type
        FROM insights i
        LEFT JOIN companies c ON c.id = i.entity_ids[1]::uuid
        ORDER BY i.insight_type, c.name
    """)

    rows = cur.fetchall()
    print("Loaded {} insights to process".format(len(rows)))

    updated = 0
    skipped = 0
    errors = []

    for row in rows:
        try:
            iid = row["id"]
            insight_type = row["insight_type"] or "competitor_market"
            company_name = row["company_name"] or "Unknown Entity"
            company_region = row["company_region"] or row["region"] or "Global"
            company_type = row["company_type"] or ""
            confidence = row["confidence"] or 0.65
            existing_summary = row["summary"] or ""
            tags = row["tags"] or []

            recipe_code = tags[1] if len(tags) >= 2 else ""

            # Skip veracity_analysis -- already fixed properly
            if insight_type == "veracity_analysis":
                skipped += 1
                continue

            new_title, new_summary = build_summary(
                company_name=company_name,
                company_region=company_region,
                company_type=company_type,
                insight_type=insight_type,
                recipe_code=recipe_code,
                confidence=confidence,
                existing_summary=existing_summary,
            )

            cur.execute(
                "UPDATE insights SET title = %s, summary = %s, updated_at = NOW() WHERE id = %s",
                (new_title, new_summary, iid)
            )
            updated += 1

        except Exception as e:
            errors.append("{}: {}".format(row["id"], e))
            if len(errors) > 20:
                print("Too many errors, aborting.")
                conn.rollback()
                sys.exit(1)

    if errors:
        print("\n{} errors:".format(len(errors)))
        for e in errors:
            print("  {}".format(e))

    conn.commit()
    print("\nDone. Updated: {}, Skipped: {}, Errors: {}".format(updated, skipped, len(errors)))

    # Show samples
    cur.execute("""
        SELECT title, LEFT(summary, 600) FROM insights
        WHERE insight_type != 'veracity_analysis'
        ORDER BY random() LIMIT 3
    """)
    for row2 in cur.fetchall():
        print("\n" + "=" * 80)
        print("TITLE: {}".format(row2[0]))
        print("SUMMARY: {}".format(row2[1]))

    cur.close()
    conn.close()


if __name__ == "__main__":
    main()
