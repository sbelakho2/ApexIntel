# ApexIntel — Full Implementation Document

> **System**: ApexIntel — OSINT Intelligence Platform for EMS & ApexMail  
> **Stack**: Rust (backend) + Next.js/TypeScript (frontend, Sensei OS template)  
> **Date**: 2026-02-23  
> **Regions**: Tunisia, Morocco, Europe, EMEA, US, Israel, China & East Asia 
> **Purpose**: Aggressive OSINT collection, deep POI profiling, competitive intelligence, security monitoring, and continuous LLM-driven pattern discovery for Starz Electronics (EMS) and ApexMail

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Core Outputs](#2-core-outputs)
3. [OSINT Sources & Extraction](#3-osint-sources--extraction)
4. [Data Model](#4-data-model)
5. [Scraping & Collection Engine](#5-scraping--collection-engine)
6. [Analytics Engine](#6-analytics-engine)
7. [Human Intel / POI Synthesis Module](#7-human-intel--poi-synthesis-module)
8. [LLM Continuous Learning Loop](#8-llm-continuous-learning-loop)
9. [Infrastructure](#9-infrastructure)
10. [Quality Gates & Tests](#10-quality-gates--tests)
11. [240 OSINT Combinational Insights](#11-240-osint-combinational-insights)
12. [Full Rust Implementation](#12-full-rust-implementation)
13. [Frontend Implementation](#13-frontend-implementation)
14. [API Keys & External Services](#14-api-keys--external-services)
15. [Deployment](#15-deployment)

---

## 1. System Overview

ApexIntel is a fully autonomous OSINT intelligence platform built in Rust with a Next.js frontend (cloned from the Sensei OS / Management-Software template). It serves two product lines:

- **Starz Electronics (EMS)**: Competitive intelligence, demand timing, supply chain risk, POI dossiers, security monitoring
- **ApexMail**: Domain/email security posture monitoring, brand impersonation detection, deliverability intelligence

### Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                        FRONTEND (Next.js)                      │
│  Dashboard │ POI Dossiers │ Warnings │ Strategy Memos │ Graph  │
└────────────────────────────┬─────────────────────────────────┘
                             │ REST/WebSocket
┌────────────────────────────┴─────────────────────────────────┐
│                      API GATEWAY (Axum)                        │
│  Auth │ Rate Limit │ CORS │ OpenTelemetry │ WebSocket Hub      │
└────────┬──────┬──────┬──────┬──────┬──────┬─────────────────┘
         │      │      │      │      │      │
    ┌────┴┐ ┌──┴──┐ ┌─┴──┐ ┌┴───┐ ┌┴──┐ ┌─┴────┐
    │Crawl│ │Parse│ │Store│ │Graph│ │Stats│ │Insight│
    │Svc  │ │Farm │ │Svc  │ │Svc  │ │Eng │ │Gen   │
    └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘ └──┬─┘ └──┬───┘
       │       │       │       │       │       │
    ┌──┴───────┴───────┴───────┴───────┴───────┴──┐
    │              MESSAGE BUS (NATS/Redis)         │
    └──┬───────┬───────┬───────┬───────┬──────────┘
       │       │       │       │       │
  ┌────┴┐ ┌───┴──┐ ┌──┴──┐ ┌─┴───┐ ┌─┴────┐
  │POI  │ │Learn │ │LLM  │ │Recipe│ │Worker│
  │Svc  │ │Loop  │ │Client│ │Reg  │ │Sched │
  └─────┘ └──────┘ └─────┘ └─────┘ └──────┘
       │       │       │       │       │
  ┌────┴───────┴───────┴───────┴───────┴──────┐
  │         STORAGE LAYER                       │
  │  Postgres(+Timescale) │ S3/MinIO │ Tantivy  │
  │  Graph (pgraph/Neo4j) │ Redis    │ Feature  │
  └───────────────────────────────────────────┘
```

---

## 2. Core Outputs

### 2.1 Weekly Strategy Memo (GM/Board Level)
- Ranked actions by impact × confidence × urgency
- Regional breakdown: Tunisia, Morocco, Israel, China & East Asia, EU, EMEA, US
- POI-informed approach guidance per target
- Security posture summary

### 2.2 Real-Time Warnings

**Business Warnings:**
| Warning Type | Trigger | SLA |
|---|---|---|
| Outsourcing Window | Job post Δ + supplier page change-point | 4h |
| Competitor Move | Capability page Δ + cert change + hiring | 12h |
| Supply Chain Shock | PCN burst + allocation keywords + port congestion | 2h |
| Margin Regime Shift | Commodity vol CP + FX vol + demand proxy | 6h |
| Regulatory Shock | Tariff/customs rule change + HS movement | 4h |

**Security Warnings:**
| Warning Type | Trigger | SLA |
|---|---|---|
| Brand Impersonation | Lookalike domain + CT cert issuance | 1h |
| DNS Posture Drift | DMARC/SPF/DKIM weakening + MX change | 2h |
| Third-party Compromise | Supplier breach notice + graph propagation | 1h |
| Phishing Campaign | Bulk registrations + procurement season | 1h |
| KEV Relevance | New KEV entry + EMS stack mapping | 4h |

### 2.3 Target Dossiers
- Company profile with inferred capabilities, risk posture, supply chain position
- Site/plant profiles with logistics lane inference
- Certification timeline and gap analysis

### 2.4 Stakeholder & Influence Dossiers
- Full POI profiles with psychological/professional inference
- Priority vectors, decision mode, influence networks
- Approach guidance: what to say, what proof to show

### 2.5 Evidence-Backed Recommendations
- Minimum 2 concrete actions per recommendation
- Minimum 2 independent evidence sources
- Confidence score with calibration
- Impact estimate (High/Med/Low + numeric band)

---

## 3. OSINT Sources & Extraction

### 3.A Business Demand & Procurement

| Source Category | Targets | Extraction | Frequency |
|---|---|---|---|
| Company websites | Press, product, supplier, case-study pages | Topic drift, keyword extraction, change detection | Daily |
| Procurement portals | Tunisia: TUNEPS; Morocco: marchespublics.gov.ma; Israel: mr.gov.il; China: China Government Procurement (ccgp.gov.cn); EU: TED (ted.europa.eu); US: SAM.gov, FBO | Tender parsing, entity extraction | 6h |
| Trade shows | Electronica, PCIM, IPC APEX, GITEX, MedPi, SIB Casablanca, SIAT Tunis, CES, embedded world, NEPCON China, SEMICON China, SEMICON Taiwan, CEATEC Japan, Korea Electronics Show | Exhibitor lists, speaker bios, agenda topics | Weekly |
| Patents | USPTO, EPO (data.epo.org), WIPO, INNORPI (Tunisia), OMPIC (Morocco) | Assignees, inventors, tech tags, citation graph | Weekly |
| Certification registries | IATF public DB, AS9100 OASIS, ISO survey, ANCSMP (Tunisia), IMANOR (Morocco) | Cert status, dates, scope changes | Daily |
| Import/export data | UN Comtrade, US Census BECI, EU Eurostat, Morocco OC, Tunisia INS | HS-code families (8534, 8542, 8536, 8544), volumes, origins | Weekly |

**Region-Specific Procurement Portals:**

| Region | Portal | URL |
|---|---|---|
| Tunisia | TUNEPS | tuneps.tn |
| Tunisia | HAICOP | haicop.tn |
| Morocco | Marchés Publics | marchespublics.gov.ma |
| Morocco | AMDIE Investment | invest.gov.ma |
| EU | TED Tenders | ted.europa.eu |
| EU | eSender portals per country | Various |
| France | BOAMP | boamp.fr |
| Germany | bund.de Vergabe | service.bund.de |
| US | SAM.gov | sam.gov |
| US | Defense contracts | fpds.gov |
| GCC | UAE Tejari | tejari.com |
| GCC | KSA Etimad | etimad.sa |
| Israel | Misrad HaKalkala (MoE) Tenders | mr.gov.il |
| Israel | Israel Government Procurement | online.mr.gov.il |
| Israel | Mapa (Planning Administration) | mavat.moin.gov.il |
| Israel | IIA (Innovation Authority) | innovationisrael.org.il |
| China | China Government Procurement | ccgp.gov.cn |
| China | China Bidding | chinabidding.com |
| China | China Fire-Sale / Alibaba Tenders | 1688.com |
| China | MOFCOM (Ministry of Commerce) | mofcom.gov.cn |
| Taiwan | Government eProcurement | web.pcc.gov.tw |
| South Korea | KONEPS (Korea ON-line E-Procurement) | g2b.go.kr |
| Japan | e-Gov Procurement Portal | chotatsu.e-gov.go.jp |

### 3.B Supply Chain & Macro

| Source | Data | Frequency |
|---|---|---|
| Commodity APIs (LME, FRED, Yahoo Finance) | Cu, Al, Sn, Ni, Li, Au, Pd prices | Hourly |
| Energy indices (EIA, IEX) | Industrial electricity, natural gas, oil | Daily |
| FX (ECB, FRED, fixer.io) | EUR/USD, EUR/MAD, EUR/TND, EUR/ILS, EUR/CNY, USD/CNY, USD/JPY, USD/KRW, USD/TWD, GBP, CHF | Hourly |
| MarineTraffic / port data | Tanger Med, Rades, Haifa, Ashdod, Rotterdam, Hamburg, LA/LB, Shanghai, Shenzhen/Yantian, Ningbo-Zhoushan, Busan, Kaohsiung, Kobe congestion | 6h |
| FreightWaves/SONAR (public) | Truck/ocean spot rates, container availability | Daily |
| NOAA/weather advisories | Hurricane/storm disruption forecasts | 6h |

### 3.C Competitor OSINT

Based on the CRM-v2 CompCrawler architecture, extended for ApexIntel:

| Source | What We Track | Detection Method |
|---|---|---|
| Capability pages | Service additions/removals, equipment lists | Hash-based change detection (SHA-256 fingerprinting) |
| Careers/hiring pages | Role family shifts, volume changes, salary signals | Structured extraction + NLP classification |
| Certification announcements | New certs, cert losses, scope changes | Registry polling + page monitoring |
| Press releases | Partnerships, expansions, new facilities | RSS + change detection |
| Trade show presence | Booth size changes, speaker topics, sponsorship level | Exhibitor list diffing |

**Seed Competitors (from CRM-v2):**

```yaml
# Tier 1 — Direct (North Africa)
- { domain: telnet-group.com, name: Telnet Holding, region: Tunisia, type: ems }
- { domain: all-circuits.com, name: All Circuits, region: Tunisia/Morocco, type: ems }
- { domain: actia.com, name: Actia Group, region: Tunisia, type: ems }
- { domain: eolane.com, name: Eolane, region: Morocco, type: ems }
- { domain: premo-group.com, name: Premo Group, region: Morocco, type: magnetics }
- { domain: coficab.com, name: Coficab, region: Tunisia, type: cable_harness }
- { domain: kromberg-schubert.com, name: "Kromberg & Schubert", region: Morocco, type: cable_harness }
- { domain: matis-aerospace.com, name: Matis Aerospace, region: Morocco, type: ems }

# Tier 2 — Eastern Europe
- { domain: fideltronik.com, name: Fideltronik, region: Poland, type: ems }
- { domain: videoton.hu, name: Videoton, region: Hungary, type: ems }
- { domain: katek.de, name: KATEK SE, region: Germany/EEurope, type: ems }
- { domain: kitron.com, name: Kitron ASA, region: Norway/Lithuania, type: ems }
- { domain: scanfil.com, name: Scanfil, region: Finland/Poland, type: ems }
- { domain: zollner.de, name: Zollner Elektronik, region: Germany/Romania, type: ems }
- { domain: cicor.com, name: Cicor Group, region: Switzerland, type: ems }

# Tier 3 — Global
- { domain: flex.com, name: Flex Ltd, type: ems }
- { domain: jabil.com, name: Jabil Inc, type: ems }
- { domain: celestica.com, name: Celestica, type: ems }
- { domain: sanmina.com, name: Sanmina, type: ems }
- { domain: plexus.com, name: Plexus Corp, type: ems }

# Tier 4 — Israel
- { domain: towersc.com, name: Tower Semiconductor, region: Israel, type: semiconductor_foundry }
- { domain: elbitamerica.com, name: Elbit Systems of America, region: Israel/US, type: defense_ems }
- { domain: iai.co.il, name: Israel Aerospace Industries, region: Israel, type: aerospace_ems }
- { domain: rafael.co.il, name: Rafael Advanced Defense Systems, region: Israel, type: defense_ems }
- { domain: orbotech.com, name: Orbotech (KLA), region: Israel, type: pcb_inspection }
- { domain: nano-di.com, name: Nano Dimension, region: Israel, type: additive_pcb }
- { domain: camtek.com, name: Camtek, region: Israel, type: inspection_equipment }
- { domain: prontoagilty.com, name: Pronto Agility, region: Israel, type: ems }
- { domain: nistec.com, name: Nistec, region: Israel, type: ems_distribution }
- { domain: rh-technologies.com, name: RH Technologies, region: Israel, type: ems }
- { domain: safecom.co.il, name: Safecom, region: Israel, type: defense_ems }
- { domain: marvell.com, name: Marvell (Israel Design Center), region: Israel, type: semiconductor_design }
- { domain: mellanox.com, name: Mellanox/NVIDIA Israel, region: Israel, type: semiconductor }
- { domain: given-imaging.com, name: Given Imaging (Medtronic IL), region: Israel, type: medical_electronics }

# Tier 5 — China & East Asia
- { domain: foxconn.com, name: Foxconn (Hon Hai), region: Taiwan/China, type: ems_global }
- { domain: pegatroncorp.com, name: Pegatron, region: Taiwan, type: ems }
- { domain: wistron.com, name: Wistron, region: Taiwan, type: ems }
- { domain: inventec.com, name: Inventec, region: Taiwan, type: ems }
- { domain: qunfei.com, name: BYD Electronic / Lens Tech, region: China, type: ems }
- { domain: luxshare-ict.com, name: Luxshare Precision, region: China, type: ems_connectors }
- { domain: goertek.com, name: GoerTek, region: China, type: ems_acoustics }
- { domain: wingtech.com, name: Wingtech Technology, region: China, type: ems_semiconductor }
- { domain: samsungsem.com, name: Samsung Electro-Mechanics, region: South Korea, type: pcb_components }
- { domain: lg.com, name: LG Innotek, region: South Korea, type: components }
- { domain: murata.com, name: Murata Manufacturing, region: Japan, type: components }
- { domain: tdk.com, name: TDK Corporation, region: Japan, type: components_magnetics }
- { domain: yageo.com, name: Yageo Corporation, region: Taiwan, type: passives }
- { domain: unimicron.com, name: Unimicron, region: Taiwan, type: pcb }
- { domain: zhen-ding.com, name: Zhen Ding Technology, region: Taiwan, type: pcb }
- { domain: compal.com, name: Compal Electronics, region: Taiwan, type: ems }
- { domain: deltaelectronics.com, name: Delta Electronics, region: Taiwan, type: power_thermal }
- { domain: tsmc.com, name: TSMC, region: Taiwan, type: semiconductor_foundry }
- { domain: smic.com, name: SMIC, region: China, type: semiconductor_foundry }
- { domain: hua-hong.com, name: Hua Hong Semiconductor, region: China, type: semiconductor_foundry }
```

### 3.D Security OSINT

| Source | Data | Method |
|---|---|---|
| DNS resolvers (Cloudflare DoH, Google DoH) | SPF, DKIM, DMARC, MX, NS, TTL, DNSSEC | Direct DNS queries via `hickory-dns` |
| Certificate Transparency | New certs for watched domains + lookalikes | crt.sh API + CT log streaming |
| CISA KEV | Exploited vulnerabilities | RSS/JSON feed polling (kev.cisa.gov) |
| Vendor PSIRTs | Cisco, Fortinet, Palo Alto, Microsoft advisories | RSS + page scraping |
| Have I Been Breached (public) | Breach notifications | News scraping + official feeds |
| Shodan (if licensed) | Exposed services, banners | API integration |
| VirusTotal (if licensed) | Domain reputation, malware associations | API integration |

### 3.E Human Intel / POI Sources

| Source | What We Extract | Region Focus |
|---|---|---|
| Company leadership/team pages | Names, titles, bios, photos (hash only) | All |
| Press releases | Named executives, role changes, quotes | All |
| Trade show speaker bios | Topics, panels, frequency | All |
| Patent inventor lists | Names, tech themes, co-invention networks | All |
| Standards body rosters | ISO TC committees, IPC task groups, IATF witness auditors | EU/US |
| Industry associations | AMICA (Morocco), FEDELEC (Tunisia), FME (NL), ZVEI (DE), IPC, SMTA | All |
| Published interviews/podcasts | Decision language, priorities, pain points | All |
| Government registers | Investissement.gov.tn, invest.gov.ma, FIPA Tunisia, AMDIE Morocco, IIA Israel, MOFCOM China | TN/MA/IL/CN |
| Free-zone authorities | Tanger Med, TAC, AFZ Kenitra, ZIEF Ben Arous, El Fejja, Haifa FTZ, Ashdod FTZ, Shenzhen SEZ, Suzhou SIP, Shanghai FTZ, Kunshan ETZ | TN/MA/IL/CN |

**For Tunisia — Key POI Target Categories:**
- FIPA (Foreign Investment Promotion Agency) directors
- Free-zone authority heads (El Fejja, Bizerte, Sousse, Monastir)
- Ministry of Industry key program officers
- ANCSMP (standards) leadership
- TUNEPS procurement platform administrators
- Major OEM plant managers (Leoni, Aptiv, Yazaki, Sumitomo in TN)
- Banking/finance leaders involved in industrial lending (BIAT, ATB, Amen Bank)
- UTICA (employers federation) electronics sector leaders

**For Morocco — Key POI Target Categories:**
- AMDIE (Moroccan Investment Agency) directors
- TAC (Tangier Automotive City) management
- AMICA (Moroccan automotive industry association) leaders
- Tanger Med port authority executives
- Free-zone directors (TFZ, AFZ Kenitra, Midparc, Nouaceur)
- IMANOR (standards) leadership
- Ministry of Industry and Trade key officials
- Major OEM plant directors (Renault, PSA, Valeo, Aptiv in MA)
- CRI (Regional Investment Centers) directors per region

**For Europe — Key POI Target Categories:**
- Procurement directors at target OEMs (by sector: auto, aero, industrial, energy)
- SQE/Supplier Quality leads at Tier 1s
- IPC European committee members
- ZVEI, FME, ACSIEL, ANIE association leaders
- Trade show organizers (Electronica, PCIM, embedded world)
- Key distributor leadership (Arrow, Avnet, Farnell, Mouser EU heads)

**For Israel — Key POI Target Categories:**
- Israel Innovation Authority (IIA) directors and program managers
- Israel Ministry of Economy and Industry officials
- Israel Export Institute (IEI) directors
- Israel Advanced Technology Industries (IATI) association leaders
- Israel Manufacturers Association (MAI) electronics division
- Israel Defense Ministry procurement (SIBAT / DSDE) officers
- Rafael, Elbit Systems, IAI, Elta procurement/SQE leads
- Tower Semiconductor, Intel Israel, Mobileye engineering leads
- Haifa / Yokneam / Herzliya tech corridor executives
- Free-trade zone authorities (Haifa, Ashdod, Eilat)
- Port authority executives (Haifa Port, Ashdod Port, future Hadarom)
- Israel Standards Institution (SII) auditors and committee members
- Bank Hapoalim, Bank Leumi industrial lending officers
- Technion, Tel Aviv University research partnership contacts
- Israel Aerospace Industries (IAI) supply chain leadership
- IDF technology unit alumni (8200, Talpiot) now in industry leadership
- Kibbutz-based electronics manufacturers (historical EMS operators)
- Israel Venture Capital (IVC) data on electronics investments

**For China — Key POI Target Categories:**
- MOFCOM (Ministry of Commerce) officials involved in electronics/EMS policy
- MIIT (Ministry of Industry and Information Technology) officials
- Shenzhen Municipal Government Industry & Innovation Bureau directors
- China Electronics Standardization Institute (CESI) leadership
- China Semiconductor Industry Association (CSIA) executives
- China Printed Circuit Association (CPCA) leadership
- Foxconn / Hon Hai executive leadership and site GMs (Shenzhen, Zhengzhou, Chengdu)
- Luxshare Precision, GoerTek, Wingtech senior management
- BYD Electronic, Lens Technology leadership
- SMIC, Hua Hong semiconductor foundry procurement/technical leads
- Shenzhen SEZ / Suzhou SIP / Shanghai FTZ authority directors
- Kunshan Electronics Technology Zone management
- Key distributor heads (Arrow China, Avnet China, WPG Holdings, Edom Technology)
- Chinese customs and export control officials (electronics HS codes)
- CCPIT (China Council for the Promotion of International Trade) electronics desk
- Chinese university electronics research leads (Tsinghua, Zhejiang, SJTU, HUST)

**For Taiwan — Key POI Target Categories:**
- TSMC, UMC, ASE Technology procurement and technical leads
- Foxconn/Hon Hai, Pegatron, Wistron, Compal senior operations managers
- Unimicron, Zhen Ding, Compeq PCB leadership
- Delta Electronics, Yageo, Lite-On engineering/procurement leads
- TPCA (Taiwan Printed Circuit Association) leadership
- TEEMA (Taiwan Electrical and Electronic Manufacturers' Association) officials
- ITRI (Industrial Technology Research Institute) researchers
- Hsinchu Science Park administration
- Taiwan Ministry of Economic Affairs (MOEA) officials

**For South Korea — Key POI Target Categories:**
- Samsung Electro-Mechanics procurement and SQE leads
- LG Innotek, SK Hynix, Samsung SDI technical leads
- KEIA (Korea Electronics Industry Association) leadership
- KATS (Korean Agency for Technology and Standards) officials
- Korea Semiconductor Industry Association (KSIA) executives

**For Japan — Key POI Target Categories:**
- Murata, TDK, Nidec, Kyocera, Panasonic Connect procurement leads
- Sony Semiconductor, Renesas Electronics engineering leads
- JEITA (Japan Electronics and IT Industries Association) officials
- JPCA (Japan Electronics Packaging and Circuits Association) leadership
- NEDO (New Energy and Industrial Technology Development Org) officials

**For US — Key POI Target Categories:**
- IPC leadership and standards committee chairs
- SMTA chapter leaders
- DoD procurement officers for EMS-relevant programs
- Key OEM procurement (defense, automotive, medical)
- ITAR/export control compliance officers at target companies

### 3.F Social Media Intelligence

#### 3.F.1 Twitter / X

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| Public company accounts | Timeline scraping (headless browser via `chromiumoxide` crate) | Posts, engagement metrics, topics, sentiment | 4h |
| Executive personal accounts | Public timeline monitoring (no auth needed for public) | Opinions, event attendance, mood shifts, priorities | 6h |
| Keyword firehose (public search) | Twitter search API v2 / scrape `search.twitter.com` | Mentions of competitors, EMS keywords, tender announcements | 2h |
| Industry & country hashtags (TN, MA, IL, CN, JP, KR, TW, GCC, EU, US) | Topic search + hashtag monitoring | Trend volume, sentiment shift, new entrants | 4h |
| List tracking | Custom lists of EMS executives, procurement leaders, industry analysts | Opinion shifts, event signals, partnership hints | 6h |
| Thread analysis | Follow reply chains on procurement/quality topics | Pain point articulation, vendor complaints, praise | Daily |
| Space/audio events | Twitter Spaces mentioning EMS/manufacturing topics | Speaker identification, topic extraction | Daily |

**Twitter/X Scraping Strategy:**
- Primary: Headless Chromium via `chromiumoxide` crate (Rust) → parse rendered HTML
- Fallback: Nitter instances (public Twitter frontend, no JS) → RSS feeds
- Rate limiting: 1 req/5s per account, rotate 50+ residential proxies
- Data retention: Store tweet text hash + metadata, not full tweet (avoid TOS issues)
- Bot detection evasion: Random scroll delays (3-8s), mouse movement simulation, session cookie persistence

**Key Twitter Accounts to Monitor (Seed List):**

```yaml
twitter_watchlist:
  # Industry organizations
  - { handle: "@IaborGlobal", category: industry, desc: "IPC/WHMA organization" }
  - { handle: "@TheIPCorg", category: industry, desc: "IPC Electronics" }
  - { handle: "@SMABORDEAUX", category: industry }
  - { handle: "@SMTAorg", category: industry }
  - { handle: "@zvabordelligenz", category: industry, region: DE }
  
  # Competitors
  - { handle: "@TelnetHolding", category: competitor, region: TN }
  - { handle: "@AllCircuits", category: competitor, region: TN }
  - { handle: "@ActiaGroup", category: competitor, region: TN }
  - { handle: "@FlexLtd", category: competitor, region: global }
  - { handle: "@JabilInc", category: competitor, region: global }
  - { handle: "@CelesticaInc", category: competitor, region: global }
  - { handle: "@Sanmina", category: competitor, region: global }
  - { handle: "@PlexusCorp", category: competitor, region: global }
  - { handle: "@KATEKSE", category: competitor, region: DE }
  - { handle: "@KitronASA", category: competitor, region: NO }
  
  # OEM procurement / engineering
  - { handle: "@BoschGlobal", category: oem, sector: automotive }
  - { handle: "@ContinentalAG", category: oem, sector: automotive }
  - { handle: "@Valeo_Group", category: oem, sector: automotive }
  - { handle: "@Aptiv", category: oem, sector: automotive }
  - { handle: "@Airbus", category: oem, sector: aerospace }
  - { handle: "@Safran", category: oem, sector: aerospace }
  - { handle: "@ThalesGroup", category: oem, sector: aerospace }
  - { handle: "@SchneiderElec", category: oem, sector: industrial }
  - { handle: "@ABBgroupnews", category: oem, sector: industrial }
  - { handle: "@SiemensIndustry", category: oem, sector: industrial }
  
  # Government / investment
  - { handle: "@InvestMorocco", category: government, region: MA }
  - { handle: "@FABORIPA_Tunisia", category: government, region: TN }
  - { handle: "@AMDIEMaroc", category: government, region: MA }
  - { handle: "@TangerMed", category: logistics, region: MA }
  
  # Israel
  - { handle: "@IsraelInnovate", category: government, region: IL }
  - { handle: "@IsraelMOE", category: government, region: IL }
  - { handle: "@TowerSemiCo", category: competitor, region: IL }
  - { handle: "@RafaelDefense", category: competitor, region: IL }
  - { handle: "@ElbitSystemsLtd", category: competitor, region: IL }
  - { handle: "@IAaborI", category: competitor, region: IL }
  - { handle: "@NanoDimension", category: competitor, region: IL }
  - { handle: "@CamtekLtd", category: competitor, region: IL }
  - { handle: "@StartUpIsrael", category: ecosystem, region: IL }
  - { handle: "@CTaborech", category: news, region: IL }
  - { handle: "@calcaboralist", category: news, region: IL }
  - { handle: "@HaifaPort", category: logistics, region: IL }
  
  # China & East Asia
  - { handle: "@FoxconnTech", category: competitor, region: TW/CN }
  - { handle: "@Aboruxshare", category: competitor, region: CN }
  - { handle: "@ABORTSMC", category: competitor, region: TW }
  - { handle: "@SEMIconnects", category: industry, region: global }
  - { handle: "@TrendForce", category: analyst, region: TW }
  - { handle: "@DigiTimes_RealTime", category: news, region: TW }
  - { handle: "@NikkeiAsia", category: news, region: JP }
  - { handle: "@scabormpcom", category: news, region: CN }
  - { handle: "@MurataMfg", category: competitor, region: JP }
  - { handle: "@TDKCorporation", category: competitor, region: JP }
  - { handle: "@SamsungSemiUS", category: competitor, region: KR }
  - { handle: "@YageoGlobal", category: competitor, region: TW }
  - { handle: "@DeltaEMEA", category: competitor, region: TW }
  - { handle: "@CaixinGlobal", category: news, region: CN }
  
  # Supply chain analysts
  - { handle: "@scabordhainbrain", category: analyst }
  - { handle: "@MitchMacDonald", category: analyst }
  - { handle: "@supplychainmgmt", category: analyst }
```

#### 3.F.2 Facebook / Meta

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| Company pages (public) | Graph API (limited) + headless scraping | Posts, events, hiring, product announcements | 6h |
| Industry groups (public) | Group feed scraping (public groups only) | Discussions, vendor complaints, recommendations | Daily |
| Marketplace (public) | Equipment listings for EMS machinery | Competitor capacity changes (selling = downsizing) | Daily |
| Events | Public events from EMS/electronics companies | Trade shows, open houses, hiring events | Daily |
| Job posts | Facebook Jobs integration | Hiring signals, role families, locations | 6h |

**Key Facebook Pages/Groups:**

```yaml
facebook_watchlist:
  pages:
    - { page: "TelnetHolding", category: competitor, region: TN }
    - { page: "AllCircuitsTunisie", category: competitor, region: TN }
    - { page: "eolanemaroc", category: competitor, region: MA }
    - { page: "TangerMedPort", category: logistics, region: MA }
    - { page: "FIPATunisia", category: government, region: TN }
    - { page: "AMDIEMaroc", category: government, region: MA }
    - { page: "AMICAMaroc", category: industry, region: MA }
    - { page: "UTICA.Officiel", category: industry, region: TN }
  
  groups:
    - { group: "electronicsmanufacturing", desc: "EMS professionals worldwide" }
    - { group: "pcb.assembly.professionals", desc: "PCB Assembly" }
    - { group: "automotiveelectronics", desc: "Auto electronics" }
    - { group: "MadeInMorocco.Industry", desc: "Morocco manufacturing" }
    - { group: "IndustrieTunisie", desc: "Tunisia industry" }
    - { group: "smt.professionals", desc: "SMT Process" }
    - { group: "IsraelHighTech", desc: "Israel high-tech manufacturing" }
    - { group: "IsraelDefenseIndustry", desc: "Israel defense sector" }
    - { group: "IsraelElectronics", desc: "Israel electronics community" }
    - { group: "ShenzhenElectronics", desc: "Shenzhen electronics manufacturing" }
    - { group: "ChinaEMSIndustry", desc: "China EMS contract manufacturing" }
    - { group: "AsiaSupplyChain", desc: "Asia-Pacific supply chain professionals" }
```

#### 3.F.3 LinkedIn (Public Data Only)

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| Company pages (public) | Headless scraping of public company profiles | Employee count changes, job postings, company updates | Daily |
| Job postings | LinkedIn Jobs RSS + public search scraping | Role families, locations, salary signals, volume changes | 6h |
| Public posts | Public article/post scraping (no login) | Executive opinions, company news, partnership announcements | Daily |
| People search (public) | Google `site:linkedin.com/in` scraping | POI discovery, role changes, career history | Weekly |
| Company followers / growth | Public company page metrics | Brand momentum, investor interest | Weekly |

**LinkedIn Scraping Constraints:**
- No logged-in scraping (TOS compliance)
- Use Google dorking: `site:linkedin.com/in "procurement" "automotive" "morocco"`
- Use Google dorking: `site:linkedin.com/jobs "EMS" OR "PCBA" OR "cable harness"`
- Cache results aggressively (LinkedIn blocks repeated access)
- Proxy rotation mandatory (50+/pool)

#### 3.F.4 YouTube

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| Company channels | YouTube Data API v3 (free tier: 10k units/day) | Factory tours, capability demos, product launches | Daily |
| Industry channels | Subscribe/poll approach | Trend analysis, technology shifts | Daily |
| Conference talks | Keyword search for `IPC APEX`, `Electronica`, `PCIM`, `SupplyChain` | Speaker identification, topic trends | Weekly |
| Webinars (recorded) | Keyword search + transcript extraction | Expert opinions, market analysis | Weekly |

**Key YouTube Channels:**
```yaml
youtube_watchlist:
  - { channel: "IPC_org", desc: "IPC standards body" }
  - { channel: "CircuitInsight", desc: "PCB industry analysis" }
  - { channel: "EEVblog", desc: "Electronics engineering" }
  - { channel: "MoroccoNow", desc: "Morocco investment promotion" }
  - { channel: "SMTAorg", desc: "SMTA soldering/assembly" }
  - { channel: "PickPlace", desc: "SMT equipment" }
  - { channel: "JukiAmericas", desc: "SMT equipment OEM" }
  - { channel: "YamahaRoboticsSMT", desc: "SMT equipment OEM" }
```

#### 3.F.5 Reddit

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| r/electronics | Reddit JSON API (public, no auth needed) | Component trends, industry sentiment, technical issues | 6h |
| r/supplychain | Subreddit monitoring | Disruption early warnings, logistics issues | 6h |
| r/manufacturing | Topic monitoring | Manufacturing trends, reshoring discussions | Daily |
| r/AskEngineers | Keyword monitoring | Engineering pain points, vendor opinions | Daily |
| r/ECE | Topic tracking | Electronics engineering trends | Daily |
| r/PrintedCircuitBoard | Direct competitor domain | PCB/PCBA discussion, vendor recommendations | 6h |

#### 3.F.6 Telegram

| Source | Extraction Method | Data Extracted | Frequency |
|---|---|---|---|
| Industry channels (public) | Teloxide library (Rust Telegram client) | Breaking news, supply disruptions, pricing leaks | 2h |
| Regional business channels | Monitor TN/MA/IL/CN/TW/KR/JP business channels | Local market intelligence, government policy | 4h |
| Component shortage channels | Track shortage/allocation channels | Component availability signals | 2h |

### 3.G Academic & Research Sources

| Source | URL | Data Extracted | Extraction Method | Frequency |
|---|---|---|---|---|
| IEEE Xplore | ieeexplore.ieee.org | EMS/PCB/SMT/harness research papers, authors, affiliations | API + web scraping | Weekly |
| Scopus | scopus.com | Citation networks, researcher profiles, institutional output | API (Elsevier) | Weekly |
| Google Scholar | scholar.google.com | Papers, patents, citation counts for EMS keywords | Headless scraping | Weekly |
| arXiv | arxiv.org | Pre-prints on manufacturing, ML/AI for quality, IoT | OAI-PMH API | Daily |
| ResearchGate | researchgate.net | Researcher profiles, paper full-texts (public) | Web scraping | Weekly |
| MDPI | mdpi.com | Open-access journals (Sensors, Electronics, Machines) | RSS + API | Weekly |
| Springer Nature | springerlink.com | Manufacturing technology, materials science | API (limited free) | Weekly |
| Wiley | onlinelibrary.wiley.com | Electronics packaging, reliability journals | Web scraping | Weekly |
| ASME Digital Collection | asmedigitalcollection.asme.org | Manufacturing processes, thermal management | Web scraping | Monthly |
| IPC EDGE (IPC standards) | edge.ipc.org | IPC standards updates, technical papers | RSS + scraping | Weekly |
| SAE International | sae.org | Automotive electronics standards, technical papers | RSS + web scraping | Weekly |
| CIRP Annals | sciencedirect.com/journal/cirp-annals | Manufacturing technology research | RSS | Monthly |
| SME (Society of Mfg Engineers) | sme.org | Manufacturing engineering research | Web scraping | Monthly |
| MIT Sloan / CSCMP | cscmp.org | Supply chain management research | RSS + scraping | Monthly |
| Fraunhofer IZM | izm.fraunhofer.de | Electronics packaging research (Germany) | Web scraping | Monthly |
| IMEC | imec-int.com | Semiconductor/nanoelectronics research (Belgium) | Web scraping | Monthly |
| LETI | leti-cea.fr | Electronics & IT research (France) | Web scraping | Monthly |
| VTT Finland | vttresearch.com | Electronics manufacturing research | Web scraping | Monthly |
| TUBITAK | tubitak.gov.tr | Turkish electronics research | Web scraping | Monthly |
| Moroccan universities | um5.ac.ma, uir.ac.ma, emi.ac.ma | Local research output on manufacturing/electronics | Google Scholar scraping | Monthly |
| Tunisian universities | enit.rnu.tn, enis.rnu.tn, insat.rnu.tn | Local research output | Google Scholar scraping | Monthly |
| Technion — Israel Institute of Technology | technion.ac.il | Electronics manufacturing, materials, robotics research | Google Scholar scraping | Monthly |
| Weizmann Institute | weizmann.ac.il | Materials science, nanoelectronics | Google Scholar scraping | Monthly |
| Tel Aviv University | tau.ac.il | Engineering, supply chain, cyber security research | Google Scholar scraping | Monthly |
| Hebrew University | huji.ac.il | Computer science, physics, materials | Google Scholar scraping | Monthly |
| Ben-Gurion University | bgu.ac.il | Desert tech, electronics reliability, manufacturing | Google Scholar scraping | Monthly |
| Israel Institute for Advanced Studies | ias.huji.ac.il | Cross-disciplinary research | Google Scholar scraping | Monthly |
| Tsinghua University | tsinghua.edu.cn | Electronics, semiconductors, manufacturing automation | Google Scholar scraping | Monthly |
| Zhejiang University | zju.edu.cn | Microelectronics, packaging, materials | Google Scholar scraping | Monthly |
| Shanghai Jiao Tong University | sjtu.edu.cn | Electronic engineering, power electronics | Google Scholar scraping | Monthly |
| HUST (Huazhong Univ. Sci. & Tech.) | hust.edu.cn | Optoelectronics, IC design, manufacturing | Google Scholar scraping | Monthly |
| Peking University | pku.edu.cn | Semiconductor physics, nanoelectronics | Google Scholar scraping | Monthly |
| National Taiwan University | ntu.edu.tw | Electronics, semiconductor, IC design | Google Scholar scraping | Monthly |
| NCTU / NYCU (Taiwan) | nycu.edu.tw | Semiconductor, photonics, electronics | Google Scholar scraping | Monthly |
| KAIST (South Korea) | kaist.ac.kr | Semiconductor, electronics engineering | Google Scholar scraping | Monthly |
| Seoul National University | snu.ac.kr | Electronics, materials science | Google Scholar scraping | Monthly |
| University of Tokyo | u-tokyo.ac.jp | Electronics, robotics, manufacturing | Google Scholar scraping | Monthly |
| Osaka University | osaka-u.ac.jp | Welding/joining research, manufacturing | Google Scholar scraping | Monthly |

**Academic Keyword Monitoring:**
```yaml
academic_keywords:
  primary:
    - "surface mount technology"
    - "PCB assembly reliability"
    - "cable harness manufacturing"
    - "overmolding process"
    - "copper winding automation"
    - "EMS quality control"
    - "automotive electronics manufacturing"
    - "aerospace electronics assembly"
    - "IPC-A-610"
    - "IPC-J-STD-001"
    - "selective soldering"
    - "conformal coating"
    - "AOI automated optical inspection"
    - "ICT in-circuit test"
    - "supply chain resilience manufacturing"
    - "nearshoring electronics"
    - "North Africa manufacturing"
    - "China Plus One strategy"
    - "semiconductor supply chain resilience"
    - "Shenzhen electronics manufacturing"
  secondary:
    - "Industry 4.0 electronics"
    - "digital twin manufacturing"
    - "predictive maintenance SMT"
    - "machine learning defect detection"
    - "X-ray inspection BGA"
    - "lead-free soldering reliability"
    - "thermal management PCB"
    - "EMC electromagnetic compatibility"
```

### 3.H News & Media Sources

#### 3.H.1 Tunisia — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| L'Economiste Maghrébin | leconomistemaghrebin.com | FR | Business/economy | RSS + scraping |
| La Presse de Tunisie | lapresse.tn | FR | General + business | RSS + scraping |
| Webmanagercenter | webmanagercenter.com | FR | Business/finance | RSS |
| African Manager | africanmanager.com | FR/EN | Business | RSS |
| Kapitalis | kapitalis.com | FR | Economy/politics | RSS |
| Leaders.com.tn | leaders.com.tn | FR | Business leaders | Scraping |
| Tunisie Numérique | tunisienumerique.com | FR | Tech/digital | RSS |
| Nawaat | nawaat.org | FR/AR | Investigative | RSS |
| Business News Tunisia | businessnews.com.tn | FR | Business | RSS |
| Espace Manager | espacemanager.com | FR | Management/industry | RSS |
| TAP (Tunis Afrique Presse) | tap.info.tn | FR/AR/EN | Official news agency | RSS |
| Réalités Online | realites.com.tn | FR | Business/politics | RSS |
| Assabahnews | assabahnews.tn | AR | Arabic business news | RSS + scraping |
| Alchourouk | alchourouk.com | AR | General + economy | RSS + scraping |

#### 3.H.2 Morocco — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| L'Economiste | leconomiste.com | FR | Business/economy (top) | RSS + scraping |
| le360 | le360.ma | FR/AR | Business/economy | RSS |
| Medias24 | medias24.com | FR | Business/finance | RSS |
| Challenge.ma | challenge.ma | FR | Business | RSS |
| La Vie Éco | lavieeco.com | FR | Economy | RSS |
| Finances News | fnh.ma | FR | Finance/economy | RSS |
| LesEco.ma | leseco.ma | FR | Economy/industry | RSS |
| Hespress | hespress.com | AR/FR | General + economy | RSS |
| Le Matin | lematin.ma | FR | General + industry | RSS |
| MAP (Maghreb Arab Press) | mapnews.ma | FR/AR/EN | Official news agency | RSS |
| Morocco World News | moroccoworldnews.com | EN | Morocco in English | RSS |
| Telquel | telquel.ma | FR | Investigative/business | RSS |
| EcoActu.ma | ecoactu.ma | FR | Economy/industry | RSS |
| Usine Nouvelle Maroc | usinenouvelle.com/maroc | FR | Industry/manufacturing (crucial) | RSS + scraping |

#### 3.H.3 Europe — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| Reuters | reuters.com | EN | Global business/industry | RSS + API |
| Financial Times | ft.com | EN | Business/finance | RSS (limited free) |
| Bloomberg | bloomberg.com | EN | Finance/supply chain | RSS (limited) |
| Handelsblatt | handelsblatt.com | DE | German business | RSS |
| Les Echos | lesechos.fr | FR | French business | RSS |
| L'Usine Nouvelle | usinenouvelle.com | FR | **French manufacturing (critical)** | RSS + scraping |
| Il Sole 24 Ore | ilsole24ore.com | IT | Italian business | RSS |
| Expansion | expansion.com | ES | Spanish business | RSS |
| Automotive News Europe | europe.autonews.com | EN | **Auto industry (critical)** | RSS |
| EE Times | eetimes.com | EN | **Electronics industry trade** | RSS |
| Electronics Weekly | electronicsweekly.com | EN | **UK electronics trade** | RSS |
| SMT Magazine / PCB007 | iconnect007.com | EN | **SMT/PCB industry (critical)** | RSS |
| Evertiq | evertiq.com | EN | **EMS industry news (critical)** | RSS |
| EPSNews | epsnews.com | EN | **Electronics purchasing** | RSS |
| The Register | theregister.com | EN | Tech/security | RSS |
| Markit/S&P Global PMI | spglobal.com/marketintelligence | EN | Manufacturing PMI data | API |
| Euractiv | euractiv.com | EN | EU policy/regulation | RSS |
| POLITICO Europe | politico.eu | EN | EU trade policy | RSS |
| EurActiv | euractiv.com | EN | EU industrial policy | RSS |
| Jeune Afrique | jeuneafrique.com | FR | **Africa business (critical)** | RSS |
| Africa Intelligence | africaintelligence.com | FR/EN | Africa business intelligence | Scraping |

#### 3.H.4 Israel — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| Globes | globes.co.il | HE/EN | **Business/finance (top Israel business daily)** | RSS + scraping |
| Calcalist (Ynet Business) | calcalist.co.il | HE/EN | **Tech/business (critical)** | RSS + scraping |
| The Marker (Haaretz Business) | themarker.com | HE | Business/economy | RSS + scraping |
| Haaretz | haaretz.com | HE/EN | General + business/politics | RSS |
| The Times of Israel | timesofisrael.com | EN | Israel news in English | RSS |
| Ynet News | ynetnews.com | EN | Israel news in English | RSS |
| Israel Hayom | israelhayom.com | HE/EN | General + economy | RSS |
| Walla! Business | business.walla.co.il | HE | Business/finance | RSS + scraping |
| CTech (Calcalist Tech) | ctech.calcalist.co.il | EN | **Israel tech/startups (critical)** | RSS |
| GeekTime | geektime.co.il | HE/EN | Tech/startups | RSS |
| Start-Up Nation Central | startupnationcentral.org | EN | Israel startup/tech ecosystem | RSS + scraping |
| No Camels | nocamels.com | EN | Israel innovation/tech | RSS |
| Israel Defense | israeldefense.co.il | HE/EN | **Defense industry (critical for defense EMS)** | RSS + scraping |
| Maariv Online | maariv.co.il | HE | General + economy | RSS |
| i24NEWS | i24news.tv | EN/FR/AR | Israeli news multi-language | RSS |
| Israel National News | israelnationalnews.com | EN | Politics/economy | RSS |
| Jerusalem Post | jpost.com | EN | General + business/tech | RSS |
| TASE (Tel Aviv Stock Exchange) | tase.co.il | HE/EN | Listed company filings, market data | API + scraping |

#### 3.H.5 China & East Asia — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| Caixin Global | caixinglobal.com | EN/ZH | **Business/finance (top China business)** | RSS + scraping |
| South China Morning Post | scmp.com | EN | **China/Asia business (critical)** | RSS |
| Nikkei Asia | asia.nikkei.com | EN | **Asia-wide business/tech (critical)** | RSS |
| 36Kr | 36kr.com | ZH/EN | China tech/startup ecosystem | RSS + scraping |
| Yicai Global | yicaiglobal.com | EN | China finance/economy | RSS |
| China Daily | chinadaily.com.cn | EN/ZH | Official China news | RSS |
| Global Times | globaltimes.cn | EN | China policy/economy | RSS |
| Xinhua Finance | xinhua.net/english | EN/ZH | Official news agency finance desk | RSS |
| EE Times China | eet-china.com | ZH | **China electronics trade (critical)** | Scraping |
| China Semiconductor Industry News | csic.org.cn | ZH | **China semiconductor policy & industry** | Scraping |
| IC Insights / TrendForce | trendforce.com | EN/ZH | **Semiconductor market intelligence (critical)** | RSS + scraping |
| DigiTimes Asia | digitimes.com | EN/ZH | **Taiwan/Asia electronics supply chain (critical)** | RSS + scraping |
| Taipei Times | taipeitimes.com | EN | Taiwan politics/business | RSS |
| Taiwan News | taiwannews.com.tw | EN | Taiwan general news | RSS |
| EE Times Japan | eetimes.jp | JP | Japan electronics trade | Scraping |
| Nikkei Electronics | nikkei.com/atcl/nxt | JP | Japan electronics industry | RSS + scraping |
| The Korea Herald | koreaherald.com | EN | South Korea business | RSS |
| Pulse by Maeil Business | pulsenews.co.kr | EN | **Korea business/tech (critical)** | RSS |
| Business Korea | businesskorea.co.kr | EN | Korea industry | RSS |
| SEMI Global | semi.org | EN | **Semiconductor equipment industry** | RSS |

#### 3.H.6 US — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| Wall Street Journal | wsj.com | EN | Business/finance | RSS (limited) |
| Bloomberg | bloomberg.com/technology | EN | Tech/supply chain | RSS |
| CNBC (Supply Chain) | cnbc.com/supply-chain | EN | Supply chain | RSS |
| Industry Week | industryweek.com | EN | **Manufacturing (critical)** | RSS |
| Assembly Magazine | assemblymag.com | EN | **Assembly/EMS (critical)** | RSS |
| Circuits Assembly | circuitsassembly.com | EN | **PCB/EMS (critical)** | RSS |
| Supply Chain Dive | supplychaindive.com | EN | Supply chain | RSS |
| Manufacturing Dive | manufacturingdive.com | EN | Manufacturing | RSS |
| Defense News | defensenews.com | EN | Defense procurement | RSS |
| Aerospace Daily | aviationweek.com | EN | Aerospace procurement | RSS |
| ECN Magazine | ecnmag.com | EN | Electronics components | RSS |
| Design World | designworldonline.com | EN | Engineering/manufacturing | RSS |

#### 3.H.7 GCC & Africa — News

| Source | URL | Language | Focus | Method |
|---|---|---|---|---|
| The National (UAE) | thenationalnews.com | EN | UAE business | RSS |
| Gulf Business | gulfbusiness.com | EN | GCC business | RSS |
| Arab News | arabnews.com | EN | Saudi business | RSS |
| Construction Week | constructionweekonline.com | EN | Infrastructure | RSS |
| Africa Business+ | africabusiness.com | EN | Africa investment | RSS |
| The Africa Report | theafricareport.com | FR/EN | Africa business | RSS |

### 3.I Job Boards & Employment Intelligence

| Source | URL | Regions | Extraction Method | Frequency |
|---|---|---|---|---|
| LinkedIn Jobs | linkedin.com/jobs | Global | Google dorking + headless | 6h |
| Indeed | indeed.com / indeed.fr / indeed.ma | Global | API + scraping | 6h |
| Glassdoor | glassdoor.com | Global | Headless scraping | Daily |
| Bayt.com | bayt.com | MENA | Web scraping | Daily |
| Emploi.ma | emploi.ma | Morocco | Scraping | 6h |
| Rekrute.com | rekrute.com | Morocco | Scraping | 6h |
| Menaraemploi | menaraemploi.ma | Morocco | Scraping | Daily |
| Emploi.tn | emploi.tn | Tunisia | Scraping | 6h |
| Tanitjobs | tanitjobs.com | Tunisia | Scraping | 6h |
| Keejob | keejob.com | Tunisia | Scraping | 6h |
| Tanqeeb | tanqeeb.com | MENA | Scraping | Daily |
| StepStone | stepstone.de | Germany | Scraping | Daily |
| Monster | monster.fr / monster.de | EU | Scraping | Daily |
| Bumeran / InfoJobs | infojobs.net | Spain | Scraping | Daily |
| APEC | apec.fr | France | Scraping | Daily |
| Pôle Emploi | pole-emploi.fr | France | API (public) | Daily |
| Xing Jobs | xing.com/jobs | DACH | Scraping | Daily |
| Totaljobs | totaljobs.com | UK | Scraping | Daily |
| USAJobs | usajobs.gov | US (gov) | API | Daily |
| CareerBuilder | careerbuilder.com | US | Scraping | Daily |
| AllJobs | alljobs.co.il | Israel | Scraping | 6h |
| Drushim | drushim.co.il | Israel | Scraping | 6h |
| Jobmaster | jobmaster.co.il | Israel | Scraping | 6h |
| GotFriends | gotfriends.co.il | Israel | Scraping | Daily |
| Israel Hi-Tech Jobs | linkedin.com/jobs (Israel geo filter) | Israel (tech) | Google dorking | Daily |
| Israel MOD Careers | mod.gov.il/Careers | Israel (defense) | Scraping | Weekly |
| 51job | 51job.com | China | Scraping | 6h |
| Zhaopin | zhaopin.com | China | Scraping | 6h |
| Boss Zhipin | zhipin.com | China (tech) | Scraping | 6h |
| Liepin | liepin.com | China (senior) | Scraping | Daily |
| 104 Job Bank | 104.com.tw | Taiwan | Scraping | 6h |
| Saramin | saramin.co.kr | South Korea | Scraping | Daily |
| JobKorea | jobkorea.co.kr | South Korea | Scraping | Daily |
| Rikunabi Next | next.rikunabi.com | Japan | Scraping | Daily |
| Mynavi | mynavi.jp | Japan | Scraping | Daily |

**Job Intelligence Extraction Features:**
- Role family classification (Procurement, Quality, Engineering, Operations, Executive, IT/Security)
- Volume tracking per company per role family (demand signals)
- Salary range extraction and normalization (purchasing power comparison)
- Experience level inference
- Skill/certification requirements (IATF, AS9100, IPC, specific equipment)
- Location analysis (new site indicator if unusual location)
- Urgency signals ("immediate start", "ASAP", multiple similar listings)

### 3.J Company Registries & Legal

| Source | URL | Region | Data | Frequency |
|---|---|---|---|---|
| RNE (Registre National des Entreprises) | rne.tn | Tunisia | Company registration, ownership, capital | Weekly |
| INNORPI company search | innorpi.tn | Tunisia | Standards/patents | Weekly |
| RC (Registre de Commerce) | directinfo.ma | Morocco | Company registration, filings | Weekly |
| CTRF (Casablanca Commerce Court) | N/A (via directinfo) | Morocco | Company filings | Weekly |
| Companies House | companieshouse.gov.uk | UK | Company filings, directors, accounts | Weekly |
| Handelsregister | handelsregister.de | Germany | Company registry | Weekly |
| Infogreffe | infogreffe.fr | France | French company filings | Weekly |
| KvK (Chamber of Commerce) | kvk.nl | Netherlands | Dutch company registry | Weekly |
| Registro Imprese | registroimprese.it | Italy | Italian company registry | Weekly |
| RMC (Registro Mercantil Central) | rmc.es | Spain | Spanish company registry | Weekly |
| SEC EDGAR | sec.gov/edgar | US | Public company filings (10-K, 10-Q) | Daily |
| OpenCorporates | opencorporates.com | Global | Aggregated company data | Weekly |
| GLEIF (LEI database) | gleif.org | Global | Legal Entity Identifiers | Weekly |
| D-U-N-S (Dun & Bradstreet) | dnb.com | Global | Company profiles, risk scores | Weekly |
| Bureau van Dijk / Orbis | bvdinfo.com | Global | Company financials, ownership | Monthly |
| Israel Companies Registrar | ica.justice.gov.il | Israel | Company registration, directors, ownership | Weekly |
| Maya (TASE filings) | maya.tase.co.il | Israel | Public company filings (Israeli market) | Daily |
| SAIC (China company info) | gsxt.gov.cn | China | Company registration, directors, ownership, annual reports | Weekly |
| Tianyancha | tianyancha.com | China | Company intelligence, litigation, shareholders (comprehensive) | Weekly |
| Qichacha | qcc.com | China | Company data, networks, risk | Weekly |
| TWSE (Taiwan Stock Exchange) | twse.com.tw | Taiwan | Public company filings (Taiwan market) | Daily |
| DART (Korea corp. filings) | dart.fss.or.kr | South Korea | Corporate filings, financial statements | Daily |
| EDINET (Japan filings) | disclosure.edinet-fss.go.jp | Japan | Corporate filings, financial reports | Daily |

### 3.K Financial & Economic Data Sources

| Source | URL | Data | Frequency |
|---|---|---|---|
| Yahoo Finance | finance.yahoo.com | Share prices, financials for public EMS companies | Hourly |
| Alpha Vantage | alphavantage.co | Stock data, sector performance | Hourly |
| FRED (Federal Reserve) | fred.stlouisfed.org | Economic indicators, FX, interest rates | Daily |
| ECB Statistical Data | sdw.ecb.europa.eu | Euro area economic data, FX | Daily |
| Bank Al-Maghrib | bkam.ma | MAD exchange rates, monetary policy | Daily |
| BCT (Central Bank of Tunisia) | bct.gov.tn | TND exchange rates, monetary policy | Daily |
| Bank of Israel | boi.org.il | ILS exchange rates, monetary policy, FX reserves | Daily |
| PBOC (People's Bank of China) | pbc.gov.cn | CNY exchange rates, monetary policy, capital flow data | Daily |
| Bank of Japan | boj.or.jp | JPY rates, monetary policy | Daily |
| Bank of Korea | bok.or.kr | KRW rates, monetary policy | Daily |
| Taiwan CBC | cbc.gov.tw | TWD rates, monetary policy | Daily |
| LME (London Metal Exchange) | lme.com | Copper, tin, nickel, aluminium prices | Hourly |
| Kitco | kitco.com | Gold, palladium, precious metals | Hourly |
| NYMEX/CME | cmegroup.com | Oil, natural gas futures | Hourly |
| EIA | eia.gov | US energy data (oil, gas, electricity) | Daily |
| Eurex | eurex.com | European derivatives | Daily |
| World Bank Open Data | data.worldbank.org | GDP, manufacturing output by country | Monthly |
| IMF Data | data.imf.org | Country economic health | Monthly |
| OECD Data | data.oecd.org | Manufacturing indices, trade data | Monthly |
| S&P Global Market Intelligence | spglobal.com | PMI indices, sector analytics | Monthly |
| Platts | spglobal.com/platts | Industrial metals pricing | Daily |
| Fastmarkets | fastmarkets.com | Base metals, battery materials | Daily |

### 3.L Sanctions, Export Control & Compliance

| Source | URL | Data | Frequency |
|---|---|---|---|
| OFAC SDN List | treasury.gov/ofac | US sanctions | Daily |
| EU Consolidated Sanctions | data.europa.eu/euodp/sanction | EU sanctions | Daily |
| UN Security Council Sanctions | un.org/securitycouncil/sanctions | UN sanctions | Daily |
| UK Sanctions List | gov.uk/government/publications/financial-sanctions | UK sanctions | Daily |
| BIS Entity List | bis.doc.gov | US export control (critical for defense EMS) | Daily |
| ITAR/EAR regulations | ecfr.gov | US export classifications | Weekly |
| Denied Persons List | bis.doc.gov/dpl | Denied parties | Daily |
| EU Dual-Use List | trade.ec.europa.eu/doclib | EU export control | Weekly |
| Wassenaar Arrangement | wassenaar.org | Multilateral export controls | Monthly |
| World Bank Debarment | worldbank.org/debarment | Debarred entities | Weekly |
| EPLS/SAM.gov exclusions | sam.gov | US government exclusions | Daily |
| Transparency International CPI | transparency.org | Country corruption indices | Annual |

### 3.M Government & Regulatory Sources

#### 3.M.1 Tunisia

| Source | Data | Frequency |
|---|---|---|
| JORT (Journal Officiel) — jort.gov.tn | Official gazette: new laws, regulations, appointments | Daily |
| FIPA — investintunisia.tn | Investment incentives, sector strategies, FDI data | Weekly |
| APII — tunisieindustrie.nat.tn | Industrial promotions, factory approvals | Weekly |
| APIA — apia.com.tn | Agricultural & agro-industrial investment | Monthly |
| Ministry of Industry — industrie.gov.tn | Industrial policy, sector plans | Weekly |
| Ministry of Trade — commerce.gov.tn | Trade regulations, import/export rules | Weekly |
| Central Bank of Tunisia (BCT) | Monetary policy, banking regulations | Daily |
| INS — ins.tn | National statistics, manufacturing indices, trade data | Monthly |
| Customs — douane.gov.tn | HS code classifications, tariff changes | Weekly |
| CEPEX — cepex.nat.tn | Export promotion data | Monthly |

#### 3.M.2 Morocco

| Source | Data | Frequency |
|---|---|---|
| Bulletin Officiel — sgg.gov.ma | Official gazette | Daily |
| AMDIE — invest.gov.ma | Investment data, sector strategies | Weekly |
| Ministry of Industry — mcinet.gov.ma | Industrial acceleration plan, ecosystem strategies | Weekly |
| HCP — hcp.ma | National statistics, manufacturing data | Monthly |
| OMPIC — ompic.ma | Trademark/patent registry, company names | Weekly |
| ANAPEC — anapec.org | Employment/job market data | Monthly |
| ANRT — anrt.ma | Telecom/digital regulation | Monthly |
| Office des Changes | FX regulations, capital flow data | Weekly |
| Customs — douane.gov.ma | Tariff schedules, trade data | Weekly |
| MASEN — masen.ma | Renewable energy projects (EMS opportunities) | Monthly |
| PortNet — portnet.ma | Port logistics, trade facilitation | Weekly |

#### 3.M.3 EU & International

| Source | Data | Frequency |
|---|---|---|
| EUR-Lex — eur-lex.europa.eu | EU regulations, directives (RoHS, REACH, WEEE, CE marking) | Daily |
| EU Official Journal | New regulations, directives | Daily |
| European Commission DG Trade | Trade agreements, tariff schedules | Weekly |
| WTO | Trade disputes, tariff changes | Weekly |
| UNCTAD | Investment trends, technology transfer | Monthly |
| AfCFTA — au-afcfta.org | African Continental Free Trade Area updates | Monthly |
| EU-Morocco Association Agreement updates | Trade preferences, rules of origin | Monthly |
| EU-Tunisia Association Agreement updates | Trade preferences, rules of origin | Monthly |

#### 3.M.4 Israel

| Source | Data | Frequency |
|---|---|---|
| Reshumot (Official Gazette) — nevo.co.il | Laws, regulations, government orders | Daily |
| Israel Innovation Authority (IIA) — innovationisrael.org.il | R&D grants, innovation programs, incentive frameworks | Weekly |
| Ministry of Economy — economy.gov.il | Industrial policy, trade regulations, import/export rules | Weekly |
| Israel Investment Center (IIC) — investinisrael.gov.il | Investment incentives, approved enterprise benefits | Weekly |
| Central Bureau of Statistics (CBS) — cbs.gov.il | Manufacturing indices, trade data, employment stats | Monthly |
| Bank of Israel — boi.org.il | ILS exchange rates, monetary policy, financial stability | Daily |
| Israel Customs — taxes.gov.il | Tariff schedules, HS classifications, trade data | Weekly |
| Israel Tax Authority — taxes.gov.il | Tax incentives, R&D tax credits | Monthly |
| Israel Standards Institution (SII) — sii.org.il | Standards certifications, mandatory standards (SI marks) | Weekly |
| Israel Export Institute — export.gov.il | Export promotion programs, trade missions, market data | Weekly |
| MATIMOP — matimop.org.il | Bi-national R&D programs (BIRD, BSF) | Monthly |
| Israel Securities Authority (ISA) — isa.gov.il | Public company filings, financial disclosures | Weekly |
| Israel Companies Registrar — ica.justice.gov.il | Company registration, directors, ownership | Weekly |
| SIBAT (Defense Export) — sibat.mod.gov.il | Defense export licensing, approved defense contractors | Monthly |
| Israel Ports Authority — israports.co.il | Port operations, logistics data (Haifa, Ashdod) | Weekly |
| Israel Free Trade Zone Authority | Haifa, Ashdod, Eilat FTZ data | Monthly |

#### 3.M.5 China & East Asia

| Source | Data | Frequency |
|---|---|---|
| MOFCOM (Ministry of Commerce) — mofcom.gov.cn | Trade policy, export controls, FDI guidelines | Weekly |
| MIIT (Ministry of Industry and IT) — miit.gov.cn | Electronics industry policy, 5G/semiconductor strategy | Weekly |
| China Government Procurement (CCGP) — ccgp.gov.cn | Government tenders, electronics procurement | Daily |
| SAMR (State Admin. Market Regulation) — samr.gov.cn | Standards, certifications, market regulation | Weekly |
| China Customs (GACC) — customs.gov.cn | HS code classifications, tariff data, trade statistics | Weekly |
| NBS (National Bureau of Statistics) — stats.gov.cn | Manufacturing indices, industrial output, PMI | Monthly |
| PBOC (People's Bank of China) — pbc.gov.cn | CNY exchange rates, monetary policy, capital controls | Daily |
| CESI (China Electronics Standardization Institute) — cesi.cn | Electronics standards, certification (CCC mark) | Weekly |
| Shenzhen Government Portal — sz.gov.cn | SEZ policy, electronics industry incentives | Weekly |
| CCPIT — ccpit.org | International trade promotion, exhibition calendar | Monthly |
| Taiwan MOEA — moea.gov.tw | Industrial policy, semiconductor strategy | Weekly |
| Taiwan BSMI — bsmi.gov.tw | Standards and certification (Taiwan) | Weekly |
| Taiwan Customs — eweb.customs.gov.tw | Taiwan trade data, tariff schedules | Weekly |
| MOTIE (Korea) — motie.go.kr | Korea industry/trade policy | Weekly |
| KATS (Korea Agency for Technology and Standards) — kats.go.kr | Korean standards (KS mark) | Weekly |
| Korea Customs — customs.go.kr | Korea trade/tariff data | Weekly |
| METI (Japan) — meti.go.jp | Japan industrial/trade policy | Weekly |
| JISC (Japanese Industrial Standards) — jisc.go.jp | JIS standards | Weekly |
| Japan Customs — customs.go.jp | Japan trade data, HS classifications | Weekly |

### 3.N Court & Legal Databases

| Source | URL | Region | Data | Frequency |
|---|---|---|---|---|
| PACER | pacer.gov | US | Federal court filings (patent disputes, trade secrets, contract) | Weekly |
| EU Court of Justice (CURIA) | curia.europa.eu | EU | EU-level trade/IP disputes | Weekly |
| UK Courts | judiciary.uk | UK | Commercial court filings | Weekly |
| WIPO Arbitration | wipo.int/amc | Global | IP disputes, domain disputes | Weekly |
| ICC Arbitration (public dockets) | iccwbo.org | Global | Commercial arbitration | Monthly |
| Tribunal de commerce (France) | infogreffe.fr | France | Commercial disputes | Weekly |
| Bankruptcy/insolvency registers | Various per country | EU | Competitor financial distress signals | Weekly |

### 3.O Industry-Specific Databases

| Source | URL | Data | Frequency |
|---|---|---|---|
| IPC standards database | ipc.org | Standards updates, committee changes | Weekly |
| UL Product iQ | productiq.ulprospector.com | UL certifications for equipment/materials | Weekly |
| IATF OASIS | iatfglobaloversight.org | IATF 16949 certified sites worldwide (critical for competitors) | Daily |
| AS9100 OASIS | iaqg.org/oasis | AS9100 certified aerospace sites | Daily |
| NADCAP | eAuditNet.com | Special process accreditations (solder, coating, NDT) | Weekly |
| IPC Validation Services | ipc.org/validation-services | IPC certification/validation for EMS | Weekly |
| PCB Directory | pcbdirectory.com | PCB manufacturer database | Monthly |
| EMSNow | emsnow.com | EMS industry news and directory (critical) | Daily |
| Global SMT & Packaging | globalsmt.net | SMT industry analysis | Weekly |
| CALCE (UMD) | calce.umd.edu | Electronics reliability data | Monthly |
| IHS Markit (S&P Global) | ihsmarkit.com | Component lifecycle, compliance data | Weekly |
| SiliconExpert | siliconexpert.com | Component data, obsolescence, compliance | Weekly |
| Z2Data | z2data.com | Supply chain risk, component analytics | Weekly |
| Digi-Key product data | digikey.com | Component availability, pricing signals | Daily |
| Mouser product data | mouser.com | Component availability, pricing signals | Daily |
| Arrow product data | arrow.com | Component availability, distributor inventory | Daily |
| Avnet product data | avnet.com | Component availability, distributor inventory | Daily |
| Farnell/element14 | farnell.com | Component availability, EU pricing | Daily |
| ERAI (independent distributors) | erai.com | Counterfeit component alerts, broker data | Weekly |
| GIDEP | gidep.org | Government/industry data exchange (US defense) | Weekly |

### 3.P Satellite & Geospatial Intelligence

| Source | URL | Data | Frequency |
|---|---|---|---|
| Google Earth Engine | earthengine.google.com | Satellite imagery for factory expansion detection | Monthly |
| Sentinel Hub (ESA) | sentinel-hub.com | Free satellite imagery (EU Copernicus program) | Monthly |
| Planet Labs (public data) | planet.com | High-res imagery (commercial, limited free) | Monthly |
| OpenStreetMap | openstreetmap.org | Building footprints, road networks near factories | Monthly |
| MarineTraffic | marinetraffic.com | Ship tracking (Tanger Med, Rades, Sfax, Shanghai, Shenzhen, Busan, Kaohsiung) | 6h |
| FlightAware / Flightradar24 | flightradar24.com | Cargo flight patterns to manufacturing hubs | Daily |
| AIS data (open) | aisstream.io | Vessel tracking for supply chain lanes | 6h |
| NOAA Weather | weather.gov | Storm/weather disruption to logistics | 6h |
| EU Copernicus EMS | emergency.copernicus.eu | Natural disaster monitoring | Real-time |

### 3.Q Messaging, Forums & Dark Web

| Source | Type | Data | Method | Frequency |
|---|---|---|---|---|
| Telegram (public channels) | Messaging | Supply chain disruptions, component shortage alerts | Teloxide crate | 2h |
| Discord (public servers) | Messaging | Electronics engineering communities, supply chain | Web scraping | Daily |
| Stack Exchange (electronics) | Forum | Technical discussions, component issues | API | Daily |
| EEVblog forum | Forum | Electronics engineering, equipment reviews | Scraping | Daily |
| SMTnet forum | Forum | **SMT/EMS industry forum (critical)** | Scraping | Daily |
| PCB forum (various) | Forum | PCB design and manufacturing | Scraping | Daily |
| Alibaba | Marketplace | EMS vendor profiles, pricing signals, new entrants | Scraping | Weekly |
| Made-in-China | Marketplace | Chinese EMS competition intelligence | Scraping | Weekly |
| Global Sources | Marketplace | Asian supplier data | Scraping | Weekly |
| Paste sites (Pastebin, etc.) | Dark web | Credential leaks mentioning target domains | Monitor services | Hourly |
| Have I Been Pwned | Breach DB | Breach notifications for target companies | API | Daily |
| IntelX | Dark web | Leaked documents, credentials | API (if licensed) | Daily |
| BreachDirectory | Breach DB | Domain-level breach lookups | API | Daily |
| Censys | Infrastructure | Exposed infrastructure, SSL certificates | API | Daily |
| Shodan | Infrastructure | Exposed services, banners, IoT devices | API | Daily |
| GreyNoise | Threat intel | Background noise IP classification | API | Daily |
| VirusTotal | Threat intel | Domain reputation, malware connections | API | Daily |
| URLScan.io | Threat intel | URL scanning, phishing detection | API | Daily |
| PhishTank | Threat intel | Known phishing URLs | API | Daily |

### 3.R Podcast & Audio Intelligence

| Source | Platform | Focus | Method | Frequency |
|---|---|---|---|---|
| Manufacturing Happy Hour | Apple/Spotify | Manufacturing industry insights | RSS + transcript | Weekly |
| Supply Chain Now | Various | Supply chain trends | RSS + transcript | Weekly |
| ASSEMBLY Audible | Assembly Mag | Assembly/EMS industry | RSS + transcript | Weekly |
| The Lean Industry | Various | Lean manufacturing | RSS + transcript | Weekly |
| Avnet's Chasing the Rabbit | Various | Electronics supply chain | RSS + transcript | Weekly |
| IPC TechConnect | IPC | Electronics standards | RSS + transcript | Monthly |
| EMS industry webinars | Various | EMS-specific topics | Calendar monitoring + recording | As available |

**Transcript Processing:**
- Whisper (medium model) for audio → text transcription
- Speaker diarization for POI identification
- NLP topic extraction aligned with insight keywords

### 3.S Total Source Summary

| Category | Subcategory | Source Count |
|---|---|---|
| 3.A | Business Demand & Procurement | 35 |
| 3.B | Supply Chain & Macro | 14 |
| 3.C | Competitor OSINT | 65+ (seed companies × source types, incl. Israel + China/EA tiers) |
| 3.D | Security OSINT | 14 |
| 3.E | Human Intel / POI | 22 |
| 3.F | Social Media Intelligence | 90 |
| 3.G | Academic & Research | 40 |
| 3.H | News & Media | 107 |
| 3.I | Job Boards & Employment | 36 |
| 3.J | Company Registries & Legal | 24 |
| 3.K | Financial & Economic Data | 22 |
| 3.L | Sanctions & Export Control | 12 |
| 3.M | Government & Regulatory | 67 |
| 3.N | Court & Legal Databases | 7 |
| 3.O | Industry-Specific Databases | 20 |
| 3.P | Satellite & Geospatial | 9 |
| 3.Q | Messaging, Forums & Dark Web | 18 |
| 3.R | Podcast & Audio | 7 |
| **TOTAL** | | **612+** |

---

## 4. Data Model

### 4.1 Entity Schema

```sql
-- Core Entities
CREATE TABLE companies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    legal_name TEXT,
    domain TEXT UNIQUE,
    country_code TEXT,
    region TEXT, -- TN, MA, EU, EMEA, US
    company_type TEXT, -- oem, ems, distributor, tier1, tier2, authority
    industry_tags TEXT[], -- automotive, aerospace, industrial, medical, energy
    employee_estimate INT,
    revenue_estimate_usd BIGINT,
    risk_score FLOAT DEFAULT 0,
    threat_score FLOAT DEFAULT 0,
    overlap_score FLOAT DEFAULT 0,
    strategic_relevance FLOAT DEFAULT 0,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE sites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    name TEXT NOT NULL,
    address TEXT,
    city TEXT,
    country_code TEXT,
    region TEXT,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    site_type TEXT, -- plant, warehouse, office, lab, hq
    capabilities TEXT[],
    certifications TEXT[],
    employee_estimate INT,
    free_zone TEXT, -- TAC, TFZ, AFZ, El_Fejja, etc.
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE product_families (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    name TEXT NOT NULL,
    hs_codes TEXT[],
    tech_tags TEXT[],
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE capabilities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    site_id UUID REFERENCES sites(id),
    capability TEXT NOT NULL, -- SMT, THT, BGA, AOI, ICT, box_build, cable_harness, etc.
    proof_grade TEXT, -- A (cert/registry), B (capability page), C (marketing), D (inferred)
    evidence_urls TEXT[],
    first_seen TIMESTAMPTZ DEFAULT now(),
    last_confirmed TIMESTAMPTZ DEFAULT now(),
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE certifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID REFERENCES companies(id),
    site_id UUID REFERENCES sites(id),
    standard TEXT NOT NULL, -- ISO_9001, IATF_16949, AS9100, ISO_13485, IPC_A_610
    status TEXT DEFAULT 'active', -- active, expired, suspended, pending
    issuing_body TEXT,
    valid_from DATE,
    valid_until DATE,
    scope TEXT,
    evidence_url TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE logistics_nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    node_type TEXT, -- port, airport, rail_terminal, border_crossing, free_zone_gate
    country_code TEXT,
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE regulations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    regulation_type TEXT, -- tariff, customs, export_control, environmental, labor
    jurisdiction TEXT, -- TN, MA, EU, US, etc.
    effective_date DATE,
    summary TEXT,
    source_url TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Person of Interest (POI)
CREATE TABLE persons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    name_ar TEXT, -- Arabic name variant
    name_fr TEXT, -- French name variant
    primary_org_id UUID REFERENCES companies(id),
    current_role TEXT,
    role_family TEXT, -- procurement, quality, engineering, operations, security, executive, government, logistics
    region TEXT,
    country_code TEXT,
    public_bio TEXT,
    public_email TEXT, -- only if role-based and publicly listed
    photo_hash TEXT, -- sha256 of photo for change detection, never store photo
    
    -- Derived features (computed nightly)
    priority_vector JSONB DEFAULT '{"cost":0,"quality":0,"speed":0,"resilience":0,"compliance":0,"security":0}',
    decision_mode TEXT, -- rfp_formal, relationship_driven, pilot_first, audit_first
    influence_score FLOAT DEFAULT 0,
    role_drift_score FLOAT DEFAULT 0,
    change_risk FLOAT DEFAULT 0,
    pain_index FLOAT DEFAULT 0,
    preferred_proof_type TEXT, -- kpi_metrics, certifications, case_studies, audit_readiness, demos
    trigger_topics TEXT[],
    
    -- Psychological / professional profile
    decision_style TEXT, -- cost_first, risk_first, quality_first, speed_first
    risk_tolerance TEXT, -- conservative, moderate, aggressive
    change_appetite TEXT, -- early_adopter, pragmatist, conservative, laggard
    communication_style TEXT, -- data_driven, narrative, visual, relationship
    
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE poi_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id UUID REFERENCES persons(id),
    artifact_type TEXT NOT NULL, -- press_quote, speaker_bio, patent, standards_role, interview, podcast, article, role_change, social_post
    title TEXT,
    content_summary TEXT,
    url TEXT NOT NULL,
    source_domain TEXT,
    language TEXT, -- en, fr, ar, de, es, it, nl
    topics TEXT[],
    sentiment_score FLOAT, -- -1 to 1  
    key_phrases TEXT[],
    ts_utc TIMESTAMPTZ NOT NULL,
    provenance JSONB NOT NULL, -- {url, fetch_ts, hash, extractor_version}
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Create index for fast artifact lookups
CREATE INDEX idx_poi_artifacts_person ON poi_artifacts(person_id, ts_utc DESC);
CREATE INDEX idx_poi_artifacts_type ON poi_artifacts(artifact_type, ts_utc DESC);
```

### 4.2 Observation Types (Time-Stamped Atomic Facts)

```sql
CREATE TABLE observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    observation_type TEXT NOT NULL,
    entity_id UUID, -- company, site, person, etc.
    entity_type TEXT,
    ts_utc TIMESTAMPTZ NOT NULL,
    value JSONB NOT NULL, -- type-specific payload
    provenance JSONB NOT NULL, -- {url, fetch_ts, content_hash, extractor_version}
    confidence FLOAT DEFAULT 1.0,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- TimescaleDB hypertable (if using Timescale)
-- SELECT create_hypertable('observations', 'ts_utc');

CREATE INDEX idx_obs_type_entity ON observations(observation_type, entity_id, ts_utc DESC);
CREATE INDEX idx_obs_type_ts ON observations(observation_type, ts_utc DESC);

-- Observation types and their JSONB value schemas:
-- JobPost:            {role, role_family, seniority, location, salary_range, source_url}
-- TenderPosted:       {title, buyer, value_estimate, sector, deadline, portal}
-- WebChange:          {url, old_hash, new_hash, change_type, diff_summary}
-- CertificationUpdate:{company_id, standard, old_status, new_status, issuer}
-- PatentPublished:    {patent_id, assignees, inventors, title, tech_tags, filing_date}
-- PortMetric:         {port, metric_type, value, unit} -- dwell_time, congestion, vessel_count
-- CommodityPrice:     {commodity, price, currency, exchange}
-- FxRate:             {pair, rate, source}
-- DnsPosture:         {domain, record_type, values, ttl, drift_detected}
-- NewDomain:          {domain, registrar, creation_date, similarity_score, target_brand}
-- VulnNotice:         {cve_id, severity, affected_products, kev_status, relevance_tags}
-- PersonMention:      {person_id, source_url, context, mention_type}
-- RoleChange:         {person_id, old_role, new_role, old_org, new_org}
-- SpeakerAppearance:  {person_id, event, topic, date, co_speakers}
-- ProcurementSignal:  {entity_id, signal_type, language_drift_score, keywords}
-- CompetitorEvent:    {competitor_id, event_type, severity, detail}
```

### 4.3 Graph Edges

```sql
CREATE TABLE graph_edges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id UUID NOT NULL,
    source_type TEXT NOT NULL,
    target_id UUID NOT NULL,
    target_type TEXT NOT NULL,
    edge_type TEXT NOT NULL,
    weight FLOAT DEFAULT 1.0,
    confidence FLOAT DEFAULT 1.0,
    evidence_ids UUID[],
    metadata JSONB DEFAULT '{}',
    first_seen TIMESTAMPTZ DEFAULT now(),
    last_seen TIMESTAMPTZ DEFAULT now(),
    
    UNIQUE(source_id, source_type, target_id, target_type, edge_type)
);

CREATE INDEX idx_graph_source ON graph_edges(source_id, source_type, edge_type);
CREATE INDEX idx_graph_target ON graph_edges(target_id, target_type, edge_type);

-- Edge types:
-- company_site:           Company ↔ Site
-- company_person:         Company ↔ Person (employment)
-- person_person:          Person ↔ Person (co-speaks, co-invents, co-authors, co-appears)
-- company_capability:     Company ↔ Capability (claimed or inferred)
-- company_company:        Company ↔ Company (supplier, customer, competitor, partner)
-- site_logistics:         Site ↔ LogisticsNode (lane inference)
-- vuln_product:           Vuln/CVE ↔ ProductFamily (relevance mapping)
-- product_company:        ProductFamily ↔ Company
-- person_event:           Person ↔ TradeShow/Conference
-- company_regulation:     Company ↔ Regulation (exposure)
-- person_patent:          Person ↔ Patent (inventor)
-- person_standard:        Person ↔ Standard (committee member)
```

---

## 5. Scraping & Collection Engine

### 5.1 Multi-Engine Search (Ported from CRM-v2)

The CRM-v2 system uses 27+ search engine scrapers with intelligent rotation. Port to Rust:

```rust
// crates/crawl/src/search/engines.rs

/// Supported search engines, ported from CRM-v2 ScrapingSearchProvider
pub enum SearchEngine {
    // Primary (most reliable)
    Google,           // via CSE API fallback
    Bing,
    DuckDuckGo,
    Brave,
    
    // Secondary
    Qwant,
    Ecosia,
    Startpage,
    Mojeek,
    
    // Tertiary (broader coverage)
    Yahoo,
    Yandex,
    Naver,
    Baidu,
    Seznam,
    
    // Meta/privacy engines
    MetaGer,
    Swisscows,
    Presearch,
    
    // Niche
    Marginalia,
    Alexandria,
    RightDao,
    Gigablast,
    Yep,
    You,
    Exalead,
    Lycos,
    Aol,
    Ask,
    Dogpile,
    Info,
    
    // Self-hosted
    SearXNG,  // multiple instances
}
```

### 5.2 Rate Limit Avoidance (Ported from CRM-v2 RateLimitManager)

```rust
// crates/crawl/src/rate_limit.rs

use std::collections::HashMap;
use std::time::{Duration, Instant};
use rand::Rng;
use serde::{Deserialize, Serialize};

const BASE_DELAY: Duration = Duration::from_secs(3);
const MAX_DELAY: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(1800); // 30 min
const FAILURE_THRESHOLD: u32 = 3;
const SUCCESS_STREAK_RESET: u32 = 2;
const JITTER_FACTOR: f64 = 0.3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub failures: f64, // float to support soft failures at 0.5
    pub last_failure: Option<Instant>,
    pub backoff: Duration,
    pub success_streak: u32,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            failures: 0.0,
            last_failure: None,
            backoff: Duration::ZERO,
            success_streak: 0,
        }
    }
}

pub struct RateLimitManager {
    engine_states: HashMap<String, EngineState>,
    state_file: String,
}

impl RateLimitManager {
    pub fn new(state_dir: &str) -> Self {
        let mut mgr = Self {
            engine_states: HashMap::new(),
            state_file: format!("{}/rate_limit_state.json", state_dir),
        };
        mgr.load_state();
        mgr
    }

    pub fn record_success(&mut self, engine: &str) {
        let state = self.engine_states.entry(engine.to_string()).or_default();
        state.success_streak += 1;
        if state.success_streak >= SUCCESS_STREAK_RESET {
            state.failures = 0.0;
            state.backoff = Duration::ZERO;
        }
        self.save_state();
    }

    pub fn record_failure(&mut self, engine: &str, is_soft: bool) {
        let state = self.engine_states.entry(engine.to_string()).or_default();
        state.success_streak = 0;
        state.last_failure = Some(Instant::now());
        state.failures += if is_soft { 0.5 } else { 1.0 };

        if state.failures >= FAILURE_THRESHOLD as f64 {
            let multiplier = 2f64.powf((state.failures - FAILURE_THRESHOLD as f64).min(6.0));
            let backoff_secs = (BASE_DELAY.as_secs_f64() * multiplier * 10.0)
                .min(MAX_COOLDOWN.as_secs_f64());
            state.backoff = Duration::from_secs_f64(backoff_secs);
        }
        self.save_state();
    }

    pub fn is_available(&self, engine: &str) -> bool {
        if let Some(state) = self.engine_states.get(engine) {
            if let Some(last_failure) = state.last_failure {
                if last_failure.elapsed() < state.backoff {
                    return false;
                }
            }
        }
        true
    }

    pub fn get_recommended_delay(&self, engine: &str) -> Duration {
        let state = self.engine_states.get(engine);
        let mut delay = BASE_DELAY.as_secs_f64();

        if let Some(s) = state {
            if s.failures > 0.0 {
                delay *= 1.0 + s.failures * 0.5;
            }
        }
        delay = delay.min(MAX_DELAY.as_secs_f64());

        let mut rng = rand::thread_rng();
        let jitter = delay * JITTER_FACTOR * rng.gen::<f64>();
        Duration::from_secs_f64(delay + jitter)
    }

    pub fn health_score(&self, engine: &str) -> u8 {
        let state = self.engine_states.get(engine);
        let mut score: i32 = 100;
        if let Some(s) = state {
            score -= (s.failures * 15.0).min(60.0) as i32;
            score += (s.success_streak * 10).min(20) as i32;
            if s.backoff > Duration::ZERO { score -= 30; }
        }
        score.clamp(0, 100) as u8
    }

    pub fn get_engines_by_health(&self, engines: &[String]) -> Vec<String> {
        let mut available: Vec<_> = engines.iter()
            .filter(|e| self.is_available(e))
            .cloned()
            .collect();
        available.sort_by(|a, b| self.health_score(b).cmp(&self.health_score(a)));
        available
    }

    fn load_state(&mut self) { /* load from state_file JSON */ }
    fn save_state(&self) { /* save to state_file JSON */ }
}
```

### 5.3 Header Randomization (Ported from CRM-v2 HeaderRandomizer)

```rust
// crates/crawl/src/headers.rs

use rand::seq::SliceRandom;
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue};

const USER_AGENTS: &[&str] = &[
    // Chrome on Windows
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 11.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    // Chrome on Mac
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_3) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    // Chrome on Linux
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    // Firefox
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:123.0) Gecko/20100101 Firefox/123.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:123.0) Gecko/20100101 Firefox/123.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:123.0) Gecko/20100101 Firefox/123.0",
    // Safari
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15",
    // Edge
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0",
    // Brave
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Brave/122",
    // Opera
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 OPR/108.0.0.0",
];

const ACCEPT_LANGUAGES: &[(&str, &[&str])] = &[
    ("default", &["en-US,en;q=0.9", "en-US,en;q=0.9,es;q=0.8", "en-GB,en;q=0.9,en-US;q=0.8"]),
    ("FR", &["fr-FR,fr;q=0.9,en;q=0.8", "fr,fr-FR;q=0.9,en-US;q=0.8,en;q=0.7"]),
    ("DE", &["de-DE,de;q=0.9,en;q=0.8", "de,de-DE;q=0.9,en-US;q=0.8,en;q=0.7"]),
    ("AR", &["ar,ar-SA;q=0.9,en;q=0.8,fr;q=0.7", "ar-TN,ar;q=0.9,fr;q=0.8,en;q=0.7", "ar-MA,ar;q=0.9,fr;q=0.8,en;q=0.7"]),
    ("ES", &["es-ES,es;q=0.9,en;q=0.8"]),
    ("IT", &["it-IT,it;q=0.9,en;q=0.8"]),
    ("NL", &["nl-NL,nl;q=0.9,en;q=0.8"]),
];

const REFERERS: &[&str] = &[
    "https://www.google.com/",
    "https://www.google.fr/",
    "https://www.google.de/",
    "https://www.google.co.ma/",
    "https://www.google.tn/",
    "https://www.bing.com/",
    "https://duckduckgo.com/",
    "", // direct
];

pub fn random_headers(region: Option<&str>) -> HeaderMap {
    let mut rng = rand::thread_rng();
    let mut headers = HeaderMap::new();

    let ua = USER_AGENTS.choose(&mut rng).unwrap();
    headers.insert("User-Agent", HeaderValue::from_str(ua).unwrap());

    headers.insert("Accept", HeaderValue::from_static(
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"
    ));

    let lang_key = region.unwrap_or("default");
    let langs = ACCEPT_LANGUAGES.iter()
        .find(|(k, _)| *k == lang_key)
        .map(|(_, v)| *v)
        .unwrap_or(ACCEPT_LANGUAGES[0].1);
    let lang = langs.choose(&mut rng).unwrap();
    headers.insert("Accept-Language", HeaderValue::from_str(lang).unwrap());

    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));
    headers.insert("Upgrade-Insecure-Requests", HeaderValue::from_static("1"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
    headers.insert("Cache-Control", HeaderValue::from_static("max-age=0"));

    // DNT ~30% of time
    if rng.gen_range(0..100) < 30 {
        headers.insert("DNT", HeaderValue::from_static("1"));
    }

    // Sec-CH-UA for Chrome
    if ua.contains("Chrome") {
        let version = ua.split("Chrome/").nth(1)
            .and_then(|s| s.split('.').next())
            .unwrap_or("122");
        let ch_ua = format!("\"Chromium\";v=\"{version}\", \"Google Chrome\";v=\"{version}\", \"Not-A.Brand\";v=\"99\"");
        headers.insert("Sec-CH-UA", HeaderValue::from_str(&ch_ua).unwrap());
        headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        let platform = if ua.contains("Windows") { "\"Windows\"" }
            else if ua.contains("Mac") { "\"macOS\"" }
            else { "\"Linux\"" };
        headers.insert("Sec-CH-UA-Platform", HeaderValue::from_str(platform).unwrap());
    }

    // Referer
    let referer = REFERERS.choose(&mut rng).unwrap();
    if !referer.is_empty() {
        headers.insert("Referer", HeaderValue::from_str(referer).unwrap());
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("cross-site"));
    } else {
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    }

    headers
}
```

### 5.4 Proxy Rotation (Ported from CRM-v2 ProxyRotator)

```rust
// crates/crawl/src/proxy.rs

use std::collections::HashMap;
use std::time::{Duration, Instant};
use anyhow::Result;

const FREE_PROXY_SOURCES: &[&str] = &[
    "https://raw.githubusercontent.com/TheSpeedX/PROXY-List/master/http.txt",
    "https://raw.githubusercontent.com/ShiftyTR/Proxy-List/master/http.txt",
    "https://raw.githubusercontent.com/monosans/proxy-list/main/proxies/http.txt",
    "https://raw.githubusercontent.com/hookzof/socks5_list/master/proxy.txt",
    "https://raw.githubusercontent.com/jetkai/proxy-list/main/online-proxies/txt/proxies-http.txt",
];

#[derive(Debug, Clone)]
pub struct ProxyHealth {
    pub failures: u32,
    pub backoff_until: Option<Instant>,
    pub last_success: Option<Instant>,
    pub success_count: u64,
}

pub struct ProxyRotator {
    proxies: Vec<String>,
    health: HashMap<String, ProxyHealth>,
    current_idx: usize,
    enabled: bool,
    paid_proxy_url: Option<String>,
}

impl ProxyRotator {
    pub fn new(enabled: bool, paid_proxy_url: Option<String>) -> Self {
        Self {
            proxies: Vec::new(),
            health: HashMap::new(),
            current_idx: 0,
            enabled,
            paid_proxy_url,
        }
    }

    pub fn get_next(&mut self) -> Option<String> {
        if !self.enabled { return None; }
        if let Some(ref url) = self.paid_proxy_url { return Some(url.clone()); }
        if self.proxies.is_empty() { return None; }

        for _ in 0..self.proxies.len().min(10) {
            let proxy = &self.proxies[self.current_idx];
            self.current_idx = (self.current_idx + 1) % self.proxies.len();

            if let Some(health) = self.health.get(proxy) {
                if let Some(until) = health.backoff_until {
                    if Instant::now() < until { continue; }
                }
            }
            return Some(proxy.clone());
        }
        // fallback
        Some(self.proxies[rand::random::<usize>() % self.proxies.len()].clone())
    }

    pub fn report_success(&mut self, proxy: &str) {
        self.health.insert(proxy.to_string(), ProxyHealth {
            failures: 0,
            backoff_until: None,
            last_success: Some(Instant::now()),
            success_count: self.health.get(proxy).map(|h| h.success_count + 1).unwrap_or(1),
        });
    }

    pub fn report_failure(&mut self, proxy: &str) {
        let failures = self.health.get(proxy).map(|h| h.failures + 1).unwrap_or(1);
        let backoff = Duration::from_secs(30 * 2u64.pow(failures.min(5) - 1));
        self.health.insert(proxy.to_string(), ProxyHealth {
            failures,
            backoff_until: Some(Instant::now() + backoff),
            last_success: self.health.get(proxy).and_then(|h| h.last_success),
            success_count: self.health.get(proxy).map(|h| h.success_count).unwrap_or(0),
        });
        if failures >= 5 {
            self.proxies.retain(|p| p != proxy);
        }
    }

    pub async fn refresh_list(&mut self, client: &reqwest::Client) -> Result<()> {
        let mut proxies = Vec::new();
        for source in FREE_PROXY_SOURCES {
            if let Ok(resp) = client.get(*source).send().await {
                if let Ok(text) = resp.text().await {
                    for line in text.lines() {
                        let line = line.trim();
                        if !line.is_empty() && line.contains(':') {
                            proxies.push(format!("http://{}", line));
                        }
                    }
                }
            }
        }
        self.proxies = proxies;
        Ok(())
    }
}
```

### 5.5 Polite Crawl Governor

```rust
// crates/crawl/src/governor.rs

use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DefaultKeyedStateStore};
use std::num::NonZeroU32;
use std::sync::Arc;

pub type DomainLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

pub struct CrawlGovernor {
    domain_limiter: Arc<DomainLimiter>,
    global_limiter: Arc<RateLimiter<governor::state::NotKeyed, governor::state::InMemoryState, DefaultClock>>,
}

impl CrawlGovernor {
    pub fn new() -> Self {
        // Per-domain: 1 request per 2.5 seconds
        let domain_quota = Quota::per_second(NonZeroU32::new(1).unwrap())
            .allow_burst(NonZeroU32::new(1).unwrap());
        
        // Global: 3 concurrent domains
        let global_quota = Quota::per_second(NonZeroU32::new(3).unwrap());

        Self {
            domain_limiter: Arc::new(RateLimiter::keyed(domain_quota)),
            global_limiter: Arc::new(RateLimiter::direct(global_quota)),
        }
    }

    pub async fn wait_for_slot(&self, domain: &str) {
        self.global_limiter.until_ready().await;
        self.domain_limiter.until_key_ready(&domain.to_string()).await;
    }
}
```

### 5.6 Change Detection (Hash-Based)

```rust
// crates/crawl/src/change_detection.rs

use sha2::{Sha256, Digest};
use sqlx::PgPool;

pub struct ChangeDetector {
    pool: PgPool,
}

impl ChangeDetector {
    /// Return true if content has changed since last fetch
    pub async fn has_changed(&self, url: &str, content: &[u8]) -> anyhow::Result<bool> {
        let new_hash = hex::encode(Sha256::digest(content));

        let row = sqlx::query_scalar::<_, String>(
            "SELECT content_hash FROM page_fingerprints WHERE url = $1 ORDER BY ts DESC LIMIT 1"
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;

        let changed = match row {
            None => true,
            Some(old_hash) => old_hash != new_hash,
        };

        if changed {
            sqlx::query(
                "INSERT INTO page_fingerprints (url, content_hash, ts) VALUES ($1, $2, now())"
            )
            .bind(url)
            .bind(&new_hash)
            .execute(&self.pool)
            .await?;
        }

        Ok(changed)
    }
}
```

### 5.7 Multilingual Extraction

```rust
// crates/parse/src/multilingual.rs

use whatlang::{detect, Lang};

/// Detect language and return ISO 639-1 code
pub fn detect_language(text: &str) -> String {
    detect(text)
        .map(|info| match info.lang() {
            Lang::Eng => "en",
            Lang::Fra => "fr",
            Lang::Ara => "ar",
            Lang::Deu => "de",
            Lang::Spa => "es",
            Lang::Ita => "it",
            Lang::Nld => "nl",
            Lang::Pol => "pl",
            Lang::Por => "pt",
            Lang::Tur => "tr",
            Lang::Cmn => "zh",
            Lang::Jpn => "ja",
            Lang::Kor => "ko",
            Lang::Heb => "he",
            _ => "en",
        })
        .unwrap_or("en")
        .to_string()
}

/// Region-aware keyword sets (from CRM-v2 crawler_config.yaml)
pub fn procurement_keywords(lang: &str) -> Vec<&'static str> {
    match lang {
        "fr" => vec![
            "fournisseur", "achats", "approvisionnement", "portail fournisseur",
            "appel d'offres", "demande de devis", "qualité fournisseur", "PPAP", "IMDS",
        ],
        "de" => vec![
            "Lieferant", "Einkauf", "Beschaffung", "Lieferantenportal",
            "Ausschreibung", "Anfrage", "Lieferantenqualität",
        ],
        "ar" => vec![
            "مورد", "موردين", "مشتريات", "بوابة الموردين",
            "طلب عرض أسعار", "جودة الموردين", "عطاء", "مناقصة",
        ],
        "es" => vec![
            "proveedor", "compras", "abastecimiento", "portal de proveedores",
            "licitación", "solicitud de oferta",
        ],
        "it" => vec![
            "fornitore", "acquisti", "approvvigionamento", "portale fornitori",
            "gara", "richiesta di offerta",
        ],
        "nl" => vec![
            "leverancier", "inkoop", "leveranciersportaal", "aanbesteding",
            "offerteaanvraag",
        ],
        "zh" => vec![
            "供应商", "采购", "供应商门户", "招标", "询价",
            "供应商质量", "电子制造", "合同制造", "政府采购",
        ],
        "ja" => vec![
            "サプライヤー", "調達", "入札", "見積依頼", "品質管理",
            "電子製造", "受託製造",
        ],
        "ko" => vec![
            "공급업체", "조달", "입찰", "견적요청", "공급업체품질",
            "전자제조", "계약제조",
        ],
        _ => vec![
            "supplier", "procurement", "vendor registration", "rfq", "rfp",
            "supplier quality", "ppap", "sourcing",
        ],
    }
}
```

### 5.8 Regional Configuration

```yaml
# config/regions.yaml

regions:
  tunisia:
    name: Tunisia
    country_code: TN
    languages: [ar, fr, en]
    tlds: [".tn", ".com.tn"]
    currencies: [TND]
    free_zones:
      - { name: "El Fejja Free Zone", city: "Tunis" }
      - { name: "Bizerte Free Zone", city: "Bizerte" }
      - { name: "Sousse Free Zone", city: "Sousse" }
      - { name: "Zarzis Free Zone", city: "Zarzis" }
      - { name: "ZIEF Ben Arous", city: "Ben Arous" }
    key_cities: [Tunis, Sousse, Sfax, Bizerte, Monastir, Gabès, Nabeul]
    investment_agencies:
      - { name: FIPA, url: "investintunisia.tn" }
      - { name: APII, url: "tunisieindustrie.nat.tn" }
    standards_body: { name: INNORPI, url: "innorpi.tn" }
    procurement_portals:
      - { name: TUNEPS, url: "tuneps.tn" }
      - { name: HAICOP, url: "haicop.tn" }
    trade_associations:
      - { name: FEDELEC, desc: "Electronic industries federation" }
      - { name: UTICA, desc: "Employers confederation" }

  morocco:
    name: Morocco
    country_code: MA
    languages: [ar, fr, en]
    tlds: [".ma", ".com.ma"]
    currencies: [MAD]
    free_zones:
      - { name: "Tangier Free Zone (TFZ)", city: "Tangier" }
      - { name: "Tangier Automotive City (TAC)", city: "Tangier" }
      - { name: "Atlantic Free Zone (AFZ)", city: "Kenitra" }
      - { name: "Midparc", city: "Casablanca" }
      - { name: "Nouaceur Aerospace", city: "Casablanca" }
      - { name: "Tanger Med Zones", city: "Tangier" }
    key_cities: [Casablanca, Tangier, Kenitra, Rabat, Fes, Marrakech, Mohammedia]
    investment_agencies:
      - { name: AMDIE, url: "invest.gov.ma" }
      - { name: "CRI Tanger-Tetouan", url: "crittangertetouan.ma" }
    standards_body: { name: IMANOR, url: "imanor.gov.ma" }
    procurement_portals:
      - { name: "Marchés Publics", url: "marchespublics.gov.ma" }
    trade_associations:
      - { name: AMICA, url: "amica.ma", desc: "Automotive industry association" }
      - { name: CGEM, desc: "Employers confederation" }

  europe:
    name: Europe
    country_codes: [DE, FR, IT, ES, NL, BE, AT, SE, FI, DK, NO, PL, CZ, SK, HU, RO, SI, IE, PT, LU]
    languages: [en, de, fr, it, es, nl, pl, cs, sv, da, no, fi, pt, hu, ro]
    currencies: [EUR, GBP, SEK, NOK, DKK, PLN, CZK, HUF, RON, CHF]
    key_procurement:
      - { name: TED, url: "ted.europa.eu" }
      - { name: BOAMP, url: "boamp.fr", country: FR }
      - { name: "bund.de Vergabe", url: "service.bund.de", country: DE }
    trade_shows:
      - { name: Electronica, city: Munich, month: 11 }
      - { name: PCIM, city: Nuremberg, month: 5 }
      - { name: "embedded world", city: Nuremberg, month: 3 }
      - { name: "Productronica", city: Munich, month: 11 }
      - { name: "Global Industrie", city: Paris/Lyon, month: 3 }
    trade_associations:
      - { name: ZVEI, country: DE }
      - { name: FME, country: NL }
      - { name: ACSIEL, country: FR }
      - { name: ANIE, country: IT }

  uk:
    name: United Kingdom
    country_code: GB
    languages: [en]
    tlds: [".uk", ".co.uk"]
    currencies: [GBP]

  gcc:
    name: "GCC / Gulf States"
    country_codes: [AE, SA, QA, KW, OM, BH]
    languages: [ar, en]
    tlds: [".ae", ".sa", ".qa", ".kw", ".om", ".bh"]
    currencies: [AED, SAR, QAR, KWD, OMR, BHD]
    free_zones:
      - { name: JAFZA, city: Dubai }
      - { name: KIZAD, city: "Abu Dhabi" }
      - { name: KAEC, city: "King Abdullah Economic City" }
    procurement_portals:
      - { name: Tejari, url: "tejari.com", country: AE }
      - { name: Etimad, url: "etimad.sa", country: SA }

  china:
    name: "China (PRC)"
    country_code: CN
    languages: [zh, en]
    tlds: [".cn", ".com.cn", ".net.cn"]
    currencies: [CNY]
    free_zones:
      - { name: "Shenzhen SEZ", city: "Shenzhen" }
      - { name: "Suzhou Industrial Park (SIP)", city: "Suzhou" }
      - { name: "Shanghai FTZ", city: "Shanghai" }
      - { name: "Kunshan ETZ", city: "Kunshan" }
      - { name: "Chengdu Hi-Tech Zone", city: "Chengdu" }
      - { name: "Zhengzhou Airport Economy Zone", city: "Zhengzhou" }
    key_cities: [Shenzhen, Shanghai, Suzhou, Kunshan, Dongguan, Guangzhou, Chengdu, Zhengzhou, Beijing, Wuhan]
    investment_agencies:
      - { name: MOFCOM, url: "mofcom.gov.cn" }
      - { name: InvestChina, url: "investchina.org.cn" }
    standards_body: { name: CESI, url: "cesi.cn" }
    procurement_portals:
      - { name: CCGP, url: "ccgp.gov.cn" }
      - { name: ChinaBidding, url: "chinabidding.com" }
    trade_associations:
      - { name: CPCA, desc: "China Printed Circuit Association" }
      - { name: CSIA, desc: "China Semiconductor Industry Association" }
      - { name: CEIA, desc: "China Electronics Industry Association" }

  taiwan:
    name: Taiwan
    country_code: TW
    languages: [zh, en]
    tlds: [".tw", ".com.tw"]
    currencies: [TWD]
    free_zones:
      - { name: "Hsinchu Science Park", city: "Hsinchu" }
      - { name: "Southern Taiwan Science Park", city: "Tainan" }
      - { name: "Central Taiwan Science Park", city: "Taichung" }
    key_cities: [Taipei, Hsinchu, Taichung, Tainan, Kaohsiung, Taoyuan]
    investment_agencies:
      - { name: InvesTaiwan, url: "investtaiwan.nat.gov.tw" }
    standards_body: { name: BSMI, url: "bsmi.gov.tw" }
    procurement_portals:
      - { name: "Gov eProcurement", url: "web.pcc.gov.tw" }
    trade_associations:
      - { name: TPCA, desc: "Taiwan Printed Circuit Association" }
      - { name: TEEMA, desc: "Taiwan Electrical & Electronic Manufacturers' Assoc" }
      - { name: SEMI Taiwan, desc: "Semiconductor Equipment & Materials" }

  south_korea:
    name: "South Korea"
    country_code: KR
    languages: [ko, en]
    tlds: [".kr", ".co.kr"]
    currencies: [KRW]
    key_cities: [Seoul, Suwon, Yongin, Icheon, Asan, Pyeongtaek, Gumi]
    investment_agencies:
      - { name: KOTRA, url: "kotra.or.kr" }
    standards_body: { name: KATS, url: "kats.go.kr" }
    procurement_portals:
      - { name: KONEPS, url: "g2b.go.kr" }
    trade_associations:
      - { name: KEIA, desc: "Korea Electronics Industry Association" }
      - { name: KSIA, desc: "Korea Semiconductor Industry Association" }

  japan:
    name: Japan
    country_code: JP
    languages: [ja, en]
    tlds: [".jp", ".co.jp"]
    currencies: [JPY]
    key_cities: [Tokyo, Osaka, Nagoya, Yokohama, Kyoto, Kobe, Fukuoka]
    investment_agencies:
      - { name: JETRO, url: "jetro.go.jp" }
    standards_body: { name: JISC, url: "jisc.go.jp" }
    procurement_portals:
      - { name: "e-Gov Procurement", url: "chotatsu.e-gov.go.jp" }
    trade_associations:
      - { name: JEITA, desc: "Japan Electronics and IT Industries Association" }
      - { name: JPCA, desc: "Japan Electronics Packaging and Circuits Association" }

  us:
    name: "United States"
    country_code: US
    languages: [en]
    tlds: [".com", ".us", ".gov"]
    currencies: [USD]
    key_procurement:
      - { name: "SAM.gov", url: "sam.gov" }
      - { name: FPDS, url: "fpds.gov" }
    trade_shows:
      - { name: "IPC APEX EXPO", month: 1 }
      - { name: CES, city: "Las Vegas", month: 1 }
      - { name: "SMTA International", month: 10 }
    east_coast_states: [MA, RI, CT, NY, NJ, PA, DE, MD, DC, VA, NC, SC, GA, FL]
    texas_metros: [Dallas, Houston, Austin, San Antonio, "Fort Worth"]
```

### 5.9 Twitter/X Scraper (Headless Chromium)

```rust
// crates/crawl/src/social/twitter.rs

use anyhow::Result;
use chromiumoxide::{Browser, BrowserConfig, Page};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    pub tweet_id: String,
    pub author_handle: String,
    pub author_name: String,
    pub text: String,
    pub timestamp_utc: i64,
    pub retweet_count: u64,
    pub like_count: u64,
    pub reply_count: u64,
    pub quote_count: u64,
    pub language: String,
    pub urls: Vec<String>,
    pub hashtags: Vec<String>,
    pub mentions: Vec<String>,
    pub is_retweet: bool,
    pub is_reply: bool,
    pub media_urls: Vec<String>,
}

pub struct TwitterScraper {
    browser: Browser,
    proxy_pool: crate::proxy::ProxyRotator,
    governor: crate::governor::CrawlGovernor,
    nitter_instances: Vec<String>,
}

impl TwitterScraper {
    pub async fn new(proxy_pool: crate::proxy::ProxyRotator) -> Result<Self> {
        let config = BrowserConfig::builder()
            .no_sandbox()
            .window_size(1920, 1080)
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-infobars")
            .arg("--disable-dev-shm-usage")
            .build()
            .map_err(|e| anyhow::anyhow!("Browser config error: {}", e))?;
        
        let (browser, mut handler) = Browser::launch(config).await?;
        tokio::spawn(async move { while let Some(_) = handler.next().await {} });

        let nitter_instances = vec![
            "https://nitter.privacydev.net".into(),
            "https://nitter.poast.org".into(),
            "https://nitter.1d4.us".into(),
            "https://nitter.kavin.rocks".into(),
        ];

        Ok(Self {
            browser,
            proxy_pool,
            governor: crate::governor::CrawlGovernor::new(),
            nitter_instances,
        })
    }

    /// Scrape a public user's timeline via Nitter (no JS, fast, low detection risk)
    pub async fn scrape_timeline_nitter(
        &self,
        handle: &str,
        max_tweets: usize,
    ) -> Result<Vec<Tweet>> {
        let instance = &self.nitter_instances[rand::random::<usize>() % self.nitter_instances.len()];
        let url = format!("{}/{}", instance, handle.trim_start_matches('@'));
        
        self.governor.wait_for_slot("nitter").await;
        
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(crate::headers::random_ua())
            .build()?;
        
        let resp = client.get(&url).send().await?;
        let html = resp.text().await?;
        let document = scraper::Html::parse_document(&html);
        
        let tweet_selector = scraper::Selector::parse(".timeline-item").unwrap();
        let content_selector = scraper::Selector::parse(".tweet-content").unwrap();
        let date_selector = scraper::Selector::parse(".tweet-date a").unwrap();
        let stats_selector = scraper::Selector::parse(".tweet-stat").unwrap();
        
        let mut tweets = Vec::new();
        
        for item in document.select(&tweet_selector).take(max_tweets) {
            let text = item.select(&content_selector)
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_default();
            
            let timestamp = item.select(&date_selector)
                .next()
                .and_then(|el| el.value().attr("title"))
                .unwrap_or("");
            
            let hashtags: Vec<String> = text.split_whitespace()
                .filter(|w| w.starts_with('#'))
                .map(|w| w.to_string())
                .collect();
            
            let mentions: Vec<String> = text.split_whitespace()
                .filter(|w| w.starts_with('@'))
                .map(|w| w.to_string())
                .collect();
            
            let mut stats = Vec::new();
            for stat in item.select(&stats_selector) {
                let val: u64 = stat.text().collect::<String>()
                    .chars().filter(|c| c.is_digit(10))
                    .collect::<String>()
                    .parse().unwrap_or(0);
                stats.push(val);
            }
            
            tweets.push(Tweet {
                tweet_id: format!("nitter_{}", tweets.len()),
                author_handle: handle.to_string(),
                author_name: handle.to_string(),
                text,
                timestamp_utc: parse_nitter_date(timestamp),
                retweet_count: stats.get(1).copied().unwrap_or(0),
                like_count: stats.get(2).copied().unwrap_or(0),
                reply_count: stats.get(0).copied().unwrap_or(0),
                quote_count: stats.get(3).copied().unwrap_or(0),
                language: String::new(), // detected later
                urls: Vec::new(),
                hashtags,
                mentions,
                is_retweet: false,
                is_reply: false,
                media_urls: Vec::new(),
            });
        }

        Ok(tweets)
    }

    /// Scrape Twitter search via headless Chromium (full JS rendering)
    pub async fn scrape_search_chromium(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<Tweet>> {
        self.governor.wait_for_slot("twitter.com").await;
        
        let page = self.browser.new_page("about:blank").await?;
        
        // Anti-detection: override navigator properties
        page.execute(chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::builder()
            .expression(r#"
                Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
                Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en', 'fr'] });
                Object.defineProperty(navigator, 'plugins', { get: () => [1, 2, 3] });
                window.chrome = { runtime: {} };
            "#.to_string())
            .build()
            .unwrap()
        ).await?;

        let search_url = format!(
            "https://twitter.com/search?q={}&src=typed_query&f=live",
            urlencoding::encode(query)
        );
        
        page.goto(&search_url).await?;
        
        // Wait for content to load
        sleep(Duration::from_millis(3000 + rand::random::<u64>() % 2000)).await;
        
        let mut tweets = Vec::new();
        let mut scroll_count = 0;
        
        while tweets.len() < max_results && scroll_count < 20 {
            // Extract tweets from current viewport
            let html = page.content().await?;
            let new_tweets = self.parse_twitter_html(&html)?;
            
            for tweet in new_tweets {
                if !tweets.iter().any(|t: &Tweet| t.tweet_id == tweet.tweet_id) {
                    tweets.push(tweet);
                }
            }
            
            // Human-like scroll
            page.execute(chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::builder()
                .expression("window.scrollBy(0, window.innerHeight * (0.7 + Math.random() * 0.3))".to_string())
                .build()
                .unwrap()
            ).await?;
            
            // Random delay between scrolls (3-7 seconds)
            sleep(Duration::from_millis(3000 + rand::random::<u64>() % 4000)).await;
            scroll_count += 1;
        }
        
        page.close().await?;
        Ok(tweets.into_iter().take(max_results).collect())
    }
    
    fn parse_twitter_html(&self, html: &str) -> Result<Vec<Tweet>> {
        let document = scraper::Html::parse_document(html);
        let article_selector = scraper::Selector::parse("article[data-testid='tweet']").unwrap();
        let text_selector = scraper::Selector::parse("[data-testid='tweetText']").unwrap();
        let user_selector = scraper::Selector::parse("[data-testid='User-Name']").unwrap();
        let time_selector = scraper::Selector::parse("time").unwrap();
        
        let mut tweets = Vec::new();
        
        for article in document.select(&article_selector) {
            let text = article.select(&text_selector)
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_default();
            
            let author = article.select(&user_selector)
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_default();
            
            let timestamp = article.select(&time_selector)
                .next()
                .and_then(|el| el.value().attr("datetime"))
                .unwrap_or("");
            
            let handle = author.split('@').nth(1).unwrap_or("").split_whitespace().next().unwrap_or("");
            
            tweets.push(Tweet {
                tweet_id: format!("tw_{}_{}", handle, timestamp),
                author_handle: format!("@{}", handle),
                author_name: author.split('@').next().unwrap_or("").trim().to_string(),
                text: text.clone(),
                timestamp_utc: chrono::DateTime::parse_from_rfc3339(timestamp)
                    .map(|dt| dt.timestamp())
                    .unwrap_or(0),
                retweet_count: 0,
                like_count: 0,
                reply_count: 0,
                quote_count: 0,
                language: crate::multilingual::detect_language(&text),
                urls: extract_urls(&text),
                hashtags: text.split_whitespace().filter(|w| w.starts_with('#')).map(String::from).collect(),
                mentions: text.split_whitespace().filter(|w| w.starts_with('@')).map(String::from).collect(),
                is_retweet: text.starts_with("RT @"),
                is_reply: false,
                media_urls: Vec::new(),
            });
        }
        
        Ok(tweets)
    }
}

fn parse_nitter_date(s: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(s, "%b %d, %Y · %I:%M %p %Z")
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0)
}

fn extract_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(String::from)
        .collect()
}
```

### 5.10 Facebook / Meta Scraper

```rust
// crates/crawl/src/social/facebook.rs

use anyhow::Result;
use chromiumoxide::{Browser, Page};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacebookPost {
    pub post_id: String,
    pub page_name: String,
    pub page_id: String,
    pub text: String,
    pub timestamp_utc: i64,
    pub reaction_count: u64,
    pub comment_count: u64,
    pub share_count: u64,
    pub post_type: FacebookPostType,
    pub urls: Vec<String>,
    pub image_count: u32,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FacebookPostType {
    Status,
    Photo,
    Video,
    Link,
    Event,
    JobPosting,
    Other,
}

pub struct FacebookScraper {
    browser: Browser,
    governor: crate::governor::CrawlGovernor,
}

impl FacebookScraper {
    pub async fn new(browser: Browser) -> Self {
        Self {
            browser,
            governor: crate::governor::CrawlGovernor::new(),
        }
    }

    /// Scrape public Facebook page posts via mobile site (lighter, more accessible)
    pub async fn scrape_page_mbasic(
        &self,
        page_name: &str,
        max_posts: usize,
    ) -> Result<Vec<FacebookPost>> {
        self.governor.wait_for_slot("facebook.com").await;
        
        // Use mbasic.facebook.com — less JS, more accessible without login
        let url = format!("https://mbasic.facebook.com/{}", page_name);
        
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1")
            .build()?;
        
        let resp = client.get(&url).send().await?;
        let html = resp.text().await?;
        let document = scraper::Html::parse_document(&html);
        
        let post_selector = scraper::Selector::parse("article, div[data-ft]").unwrap();
        let text_selector = scraper::Selector::parse("p, div.story_body_container").unwrap();
        
        let mut posts = Vec::new();
        
        for post_el in document.select(&post_selector).take(max_posts) {
            let text = post_el.select(&text_selector)
                .map(|el| el.text().collect::<String>())
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            
            if text.is_empty() { continue; }
            
            posts.push(FacebookPost {
                post_id: format!("fb_{}_{}", page_name, posts.len()),
                page_name: page_name.to_string(),
                page_id: String::new(),
                text: text.clone(),
                timestamp_utc: chrono::Utc::now().timestamp(),
                reaction_count: 0,
                comment_count: 0,
                share_count: 0,
                post_type: infer_post_type(&text),
                urls: extract_urls_fb(&text),
                image_count: 0,
                language: crate::multilingual::detect_language(&text),
            });
        }
        
        Ok(posts)
    }

    /// Scrape public Facebook page via headless Chromium (full rendering)
    pub async fn scrape_page_chromium(
        &self,
        page_name: &str,
        max_posts: usize,
    ) -> Result<Vec<FacebookPost>> {
        self.governor.wait_for_slot("facebook.com").await;
        
        let page = self.browser.new_page("about:blank").await?;
        
        // Anti-detection
        page.execute(chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::builder()
            .expression(r#"
                Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
                window.chrome = { runtime: {} };
            "#.to_string())
            .build()
            .unwrap()
        ).await?;

        let url = format!("https://www.facebook.com/{}", page_name);
        page.goto(&url).await?;
        sleep(Duration::from_millis(4000 + rand::random::<u64>() % 3000)).await;
        
        let mut posts = Vec::new();
        let mut scroll_count = 0;
        
        while posts.len() < max_posts && scroll_count < 15 {
            let html = page.content().await?;
            let new_posts = self.parse_facebook_html(&html, page_name)?;
            
            for post in new_posts {
                if !posts.iter().any(|p: &FacebookPost| p.post_id == post.post_id) {
                    posts.push(post);
                }
            }
            
            // Human-like scroll
            page.execute(chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::builder()
                .expression("window.scrollBy(0, window.innerHeight * (0.6 + Math.random() * 0.4))".to_string())
                .build()
                .unwrap()
            ).await?;
            
            sleep(Duration::from_millis(4000 + rand::random::<u64>() % 5000)).await;
            scroll_count += 1;
        }
        
        page.close().await?;
        Ok(posts.into_iter().take(max_posts).collect())
    }

    /// Scrape Facebook public group feed (public groups only)
    pub async fn scrape_group(
        &self,
        group_id: &str,
        max_posts: usize,
    ) -> Result<Vec<FacebookPost>> {
        self.governor.wait_for_slot("facebook.com").await;
        
        let url = format!("https://mbasic.facebook.com/groups/{}", group_id);
        
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X)")
            .build()?;
        
        let resp = client.get(&url).send().await?;
        let html = resp.text().await?;
        
        self.parse_mbasic_group_html(&html, group_id, max_posts)
    }

    /// Scrape Facebook Marketplace for EMS equipment (competitor capacity signals)
    pub async fn scrape_marketplace_equipment(
        &self,
        keywords: &[&str],
        location: &str,
    ) -> Result<Vec<MarketplaceListing>> {
        self.governor.wait_for_slot("facebook.com").await;
        
        let page = self.browser.new_page("about:blank").await?;
        
        let query = keywords.join(" ");
        let url = format!(
            "https://www.facebook.com/marketplace/search/?query={}",
            urlencoding::encode(&query)
        );
        
        page.goto(&url).await?;
        sleep(Duration::from_millis(5000 + rand::random::<u64>() % 3000)).await;
        
        let html = page.content().await?;
        let listings = self.parse_marketplace_html(&html)?;
        
        page.close().await?;
        Ok(listings)
    }

    fn parse_facebook_html(&self, html: &str, page_name: &str) -> Result<Vec<FacebookPost>> {
        let document = scraper::Html::parse_document(html);
        let post_selector = scraper::Selector::parse("[data-ad-preview='message']").unwrap();
        
        let mut posts = Vec::new();
        for post_el in document.select(&post_selector) {
            let text = post_el.text().collect::<String>().trim().to_string();
            if text.is_empty() { continue; }
            
            posts.push(FacebookPost {
                post_id: format!("fb_{}_{}", page_name, sha256_short(&text)),
                page_name: page_name.to_string(),
                page_id: String::new(),
                text: text.clone(),
                timestamp_utc: chrono::Utc::now().timestamp(),
                reaction_count: 0,
                comment_count: 0,
                share_count: 0,
                post_type: infer_post_type(&text),
                urls: extract_urls_fb(&text),
                image_count: 0,
                language: crate::multilingual::detect_language(&text),
            });
        }
        
        Ok(posts)
    }

    fn parse_mbasic_group_html(&self, html: &str, group_id: &str, max: usize) -> Result<Vec<FacebookPost>> {
        let document = scraper::Html::parse_document(html);
        let story_selector = scraper::Selector::parse("div.story_body_container, article").unwrap();
        
        let mut posts = Vec::new();
        for story in document.select(&story_selector).take(max) {
            let text = story.text().collect::<String>().trim().to_string();
            if text.len() < 10 { continue; }
            
            posts.push(FacebookPost {
                post_id: format!("fb_group_{}_{}", group_id, posts.len()),
                page_name: format!("group:{}", group_id),
                page_id: group_id.to_string(),
                text: text.clone(),
                timestamp_utc: chrono::Utc::now().timestamp(),
                reaction_count: 0,
                comment_count: 0,
                share_count: 0,
                post_type: FacebookPostType::Status,
                urls: extract_urls_fb(&text),
                image_count: 0,
                language: crate::multilingual::detect_language(&text),
            });
        }
        
        Ok(posts)
    }

    fn parse_marketplace_html(&self, html: &str) -> Result<Vec<MarketplaceListing>> {
        let document = scraper::Html::parse_document(html);
        // Marketplace parsing is highly dynamic; extract what we can
        let item_selector = scraper::Selector::parse("[data-testid='marketplace_feed_item']").unwrap();
        
        let mut listings = Vec::new();
        for item in document.select(&item_selector) {
            let text = item.text().collect::<String>();
            listings.push(MarketplaceListing {
                title: text.lines().next().unwrap_or("").to_string(),
                price_text: text.lines().nth(1).unwrap_or("").to_string(),
                location: String::new(),
                url: String::new(),
                relevant_keywords: vec![],
            });
        }
        
        Ok(listings)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListing {
    pub title: String,
    pub price_text: String,
    pub location: String,
    pub url: String,
    pub relevant_keywords: Vec<String>,
}

fn infer_post_type(text: &str) -> FacebookPostType {
    let lower = text.to_lowercase();
    if lower.contains("hiring") || lower.contains("job") || lower.contains("recrutement") {
        FacebookPostType::JobPosting
    } else if lower.contains("event") || lower.contains("événement") {
        FacebookPostType::Event
    } else if lower.contains("http") {
        FacebookPostType::Link
    } else {
        FacebookPostType::Status
    }
}

fn extract_urls_fb(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(String::from)
        .collect()
}

fn sha256_short(text: &str) -> String {
    use sha2::{Sha256, Digest};
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    hash[..12].to_string()
}
```

### 5.11 Social Media Intelligence Aggregator

```rust
// crates/crawl/src/social/mod.rs

pub mod twitter;
pub mod facebook;
pub mod reddit;
pub mod telegram;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialSignal {
    pub platform: Platform,
    pub source_id: String,
    pub entity_name: String,
    pub entity_type: String, // company, person, topic
    pub signal_type: SocialSignalType,
    pub text: String,
    pub sentiment: f64, // -1.0 to 1.0
    pub engagement_score: f64,
    pub language: String,
    pub timestamp_utc: i64,
    pub url: String,
    pub topics: Vec<String>,
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Platform {
    Twitter,
    Facebook,
    LinkedIn,
    Reddit,
    Telegram,
    YouTube,
    Discord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SocialSignalType {
    SentimentShift,      // Positive/negative change in tone
    VolumeSpike,         // Unusual posting frequency
    TopicEmergence,      // New topic appearing
    ExecutiveOpinion,    // POI expressing views
    CompetitorMention,   // Competitor being discussed
    PainPointExpression, // Complaints, frustrations
    PartnershipHint,     // Collaboration signals
    HiringSignal,        // Job-related posts
    EventAnnouncement,   // Trade shows, launches
    CrisisSignal,        // Quality issues, recalls, scandals
    TechnologyShift,     // New tech adoption discussion
    RegulatoryDiscussion,// Policy/regulation talk
}

pub struct SocialIntelAggregator {
    twitter: twitter::TwitterScraper,
    facebook: facebook::FacebookScraper,
    pool: sqlx::PgPool,
}

impl SocialIntelAggregator {
    /// Run a complete social media intelligence cycle
    pub async fn run_cycle(&self, watchlists: &SocialWatchlists) -> Result<Vec<SocialSignal>> {
        let mut all_signals = Vec::new();

        // Twitter: company accounts + keyword search
        for account in &watchlists.twitter_accounts {
            match self.twitter.scrape_timeline_nitter(&account.handle, 20).await {
                Ok(tweets) => {
                    for tweet in tweets {
                        if let Some(signal) = self.classify_tweet(&tweet, &account.category).await {
                            all_signals.push(signal);
                        }
                    }
                }
                Err(e) => tracing::warn!("Twitter scrape failed for {}: {}", account.handle, e),
            }
        }

        // Twitter: keyword monitoring
        for query in &watchlists.twitter_keywords {
            match self.twitter.scrape_search_chromium(query, 50).await {
                Ok(tweets) => {
                    for tweet in tweets {
                        if let Some(signal) = self.classify_tweet(&tweet, "keyword").await {
                            all_signals.push(signal);
                        }
                    }
                }
                Err(e) => tracing::warn!("Twitter search failed for '{}': {}", query, e),
            }
        }

        // Facebook: page monitoring
        for page in &watchlists.facebook_pages {
            match self.facebook.scrape_page_mbasic(&page.page_id, 10).await {
                Ok(posts) => {
                    for post in posts {
                        if let Some(signal) = self.classify_fb_post(&post, &page.category).await {
                            all_signals.push(signal);
                        }
                    }
                }
                Err(e) => tracing::warn!("Facebook scrape failed for {}: {}", page.page_id, e),
            }
        }

        // Facebook: marketplace equipment monitoring
        let equipment_keywords = vec![
            "SMT machine", "pick and place", "reflow oven", "wave soldering",
            "AOI machine", "ICT tester", "wire harness machine", "cable assembly",
            "overmolding press", "winding machine", "PCB assembly line",
        ];
        
        match self.facebook.scrape_marketplace_equipment(
            &equipment_keywords.iter().map(|s| *s).collect::<Vec<_>>(),
            "Morocco"
        ).await {
            Ok(listings) => {
                for listing in listings {
                    all_signals.push(SocialSignal {
                        platform: Platform::Facebook,
                        source_id: format!("marketplace_{}", sha256_short(&listing.title)),
                        entity_name: listing.title.clone(),
                        entity_type: "equipment".into(),
                        signal_type: SocialSignalType::TechnologyShift,
                        text: format!("{} - {}", listing.title, listing.price_text),
                        sentiment: 0.0,
                        engagement_score: 0.0,
                        language: "en".into(),
                        timestamp_utc: chrono::Utc::now().timestamp(),
                        url: listing.url,
                        topics: vec!["used_equipment".into(), "capacity_signal".into()],
                        mentions: vec![],
                    });
                }
            }
            Err(e) => tracing::warn!("Marketplace scrape failed: {}", e),
        }

        // Store signals to database
        for signal in &all_signals {
            self.store_signal(signal).await?;
        }

        tracing::info!("Social intel cycle: {} signals collected", all_signals.len());
        Ok(all_signals)
    }

    async fn classify_tweet(
        &self,
        tweet: &twitter::Tweet,
        category: &str,
    ) -> Option<SocialSignal> {
        let text_lower = tweet.text.to_lowercase();
        
        let signal_type = if text_lower.contains("hiring") || text_lower.contains("job") {
            SocialSignalType::HiringSignal
        } else if text_lower.contains("partnership") || text_lower.contains("collaboration") {
            SocialSignalType::PartnershipHint
        } else if text_lower.contains("recall") || text_lower.contains("defect") || text_lower.contains("failure") {
            SocialSignalType::CrisisSignal
        } else if text_lower.contains("trade show") || text_lower.contains("exhibition") || text_lower.contains("salon") {
            SocialSignalType::EventAnnouncement
        } else if tweet.retweet_count > 100 || tweet.like_count > 500 {
            SocialSignalType::VolumeSpike
        } else {
            SocialSignalType::TopicEmergence
        };
        
        let engagement = (tweet.retweet_count + tweet.like_count * 2 + tweet.reply_count * 3) as f64;
        let sentiment = simple_sentiment(&tweet.text);
        
        Some(SocialSignal {
            platform: Platform::Twitter,
            source_id: tweet.tweet_id.clone(),
            entity_name: tweet.author_handle.clone(),
            entity_type: category.to_string(),
            signal_type,
            text: tweet.text.clone(),
            sentiment,
            engagement_score: engagement,
            language: tweet.language.clone(),
            timestamp_utc: tweet.timestamp_utc,
            url: format!("https://twitter.com/{}/status/{}", tweet.author_handle, tweet.tweet_id),
            topics: tweet.hashtags.clone(),
            mentions: tweet.mentions.clone(),
        })
    }

    async fn classify_fb_post(
        &self,
        post: &facebook::FacebookPost,
        category: &str,
    ) -> Option<SocialSignal> {
        let text_lower = post.text.to_lowercase();
        
        let signal_type = if matches!(post.post_type, facebook::FacebookPostType::JobPosting) {
            SocialSignalType::HiringSignal
        } else if text_lower.contains("partnership") || text_lower.contains("partenariat") {
            SocialSignalType::PartnershipHint
        } else if text_lower.contains("event") || text_lower.contains("salon") || text_lower.contains("exposition") {
            SocialSignalType::EventAnnouncement
        } else {
            SocialSignalType::TopicEmergence
        };
        
        Some(SocialSignal {
            platform: Platform::Facebook,
            source_id: post.post_id.clone(),
            entity_name: post.page_name.clone(),
            entity_type: category.to_string(),
            signal_type,
            text: post.text.clone(),
            sentiment: simple_sentiment(&post.text),
            engagement_score: (post.reaction_count + post.comment_count * 2 + post.share_count * 3) as f64,
            language: post.language.clone(),
            timestamp_utc: post.timestamp_utc,
            url: format!("https://facebook.com/{}", post.page_name),
            topics: vec![],
            mentions: vec![],
        })
    }

    async fn store_signal(&self, signal: &SocialSignal) -> Result<()> {
        sqlx::query(
            "INSERT INTO social_signals (platform, source_id, entity_name, entity_type, signal_type, \
             content, sentiment, engagement_score, language, ts_utc, url, topics, mentions) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, to_timestamp($10), $11, $12, $13) \
             ON CONFLICT (platform, source_id) DO NOTHING"
        )
        .bind(format!("{:?}", signal.platform))
        .bind(&signal.source_id)
        .bind(&signal.entity_name)
        .bind(&signal.entity_type)
        .bind(format!("{:?}", signal.signal_type))
        .bind(&signal.text)
        .bind(signal.sentiment)
        .bind(signal.engagement_score)
        .bind(&signal.language)
        .bind(signal.timestamp_utc)
        .bind(&signal.url)
        .bind(&signal.topics)
        .bind(&signal.mentions)
        .execute(&self.pool).await?;
        Ok(())
    }
}

fn simple_sentiment(text: &str) -> f64 {
    let positive = ["excellent", "great", "innovative", "growth", "success", "award", "partnership",
        "expansion", "nouveau", "investissement", "croissance", "succès", "réussite"];
    let negative = ["delay", "shortage", "defect", "recall", "failure", "lawsuit", "bankruptcy",
        "retard", "pénurie", "défaut", "rappel", "faillite", "crise"];
    
    let lower = text.to_lowercase();
    let pos: f64 = positive.iter().map(|w| lower.matches(w).count() as f64).sum();
    let neg: f64 = negative.iter().map(|w| lower.matches(w).count() as f64).sum();
    
    if pos + neg == 0.0 { return 0.0; }
    (pos - neg) / (pos + neg)
}

fn sha256_short(text: &str) -> String {
    use sha2::{Sha256, Digest};
    hex::encode(Sha256::digest(text.as_bytes()))[..12].to_string()
}

#[derive(Debug)]
pub struct SocialWatchlists {
    pub twitter_accounts: Vec<SocialAccount>,
    pub twitter_keywords: Vec<String>,
    pub facebook_pages: Vec<SocialAccount>,
    pub facebook_groups: Vec<String>,
    pub reddit_subreddits: Vec<String>,
    pub telegram_channels: Vec<String>,
}

#[derive(Debug)]
pub struct SocialAccount {
    pub handle: String,
    pub page_id: String,
    pub category: String,
    pub region: String,
}
```

---

## 6. Analytics Engine

### 6.1 Core Statistical Methods

```rust
// crates/stats/src/lib.rs

pub mod changepoint;
pub mod anomaly;
pub mod correlation;
pub mod mutual_info;
pub mod fisher;
pub mod hazard;
pub mod bayesian;
pub mod graph_risk;
pub mod fdr;

/// Change-point detection (PELT algorithm)
pub mod changepoint {
    pub struct PeltConfig {
        pub penalty: f64,
        pub min_segment: usize,
    }

    pub fn detect_changepoints(data: &[f64], config: &PeltConfig) -> Vec<usize> {
        // PELT (Pruned Exact Linear Time) algorithm
        let n = data.len();
        if n < config.min_segment * 2 { return vec![]; }
        
        let mut cost = vec![0.0f64; n + 1];
        let mut changepoints: Vec<Vec<usize>> = vec![vec![]; n + 1];
        
        for t in config.min_segment..=n {
            let mut best_cost = f64::MAX;
            let mut best_cp = 0;
            
            for s in 0..=(t - config.min_segment) {
                let segment_cost = gaussian_cost(&data[s..t]);
                let total = cost[s] + segment_cost + config.penalty;
                if total < best_cost {
                    best_cost = total;
                    best_cp = s;
                }
            }
            cost[t] = best_cost;
            changepoints.push({
                let mut cp = changepoints[best_cp].clone();
                if best_cp > 0 { cp.push(best_cp); }
                cp
            });
        }
        changepoints[n].clone()
    }

    fn gaussian_cost(data: &[f64]) -> f64 {
        if data.is_empty() { return 0.0; }
        let n = data.len() as f64;
        let mean = data.iter().sum::<f64>() / n;
        let var = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        if var < 1e-12 { return 0.0; }
        n * (var.ln() + 1.0)
    }
}

/// Anomaly detection (MAD z-score + EWMA)
pub mod anomaly {
    pub fn mad_zscore(data: &[f64], threshold: f64) -> Vec<(usize, f64)> {
        if data.len() < 3 { return vec![]; }
        let mut sorted = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];
        let mut abs_devs: Vec<f64> = data.iter().map(|x| (x - median).abs()).collect();
        abs_devs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mad = abs_devs[abs_devs.len() / 2] * 1.4826; // consistency constant
        if mad < 1e-12 { return vec![]; }
        
        data.iter().enumerate()
            .filter_map(|(i, x)| {
                let z = (x - median) / mad;
                if z.abs() > threshold { Some((i, z)) } else { None }
            })
            .collect()
    }

    pub fn ewma_control(data: &[f64], alpha: f64, sigma_mult: f64) -> Vec<(usize, f64)> {
        if data.len() < 10 { return vec![]; }
        let mean = data.iter().sum::<f64>() / data.len() as f64;
        let var = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / data.len() as f64;
        let sigma = var.sqrt();
        
        let mut ewma = mean;
        let mut anomalies = vec![];
        
        for (i, &x) in data.iter().enumerate() {
            ewma = alpha * x + (1.0 - alpha) * ewma;
            let limit = sigma * sigma_mult * (alpha / (2.0 - alpha)).sqrt();
            if (ewma - mean).abs() > limit {
                anomalies.push((i, (ewma - mean) / sigma));
            }
        }
        anomalies
    }
}

/// Lagged cross-correlation
pub mod correlation {
    pub fn lagged_xcorr(x: &[f64], y: &[f64], max_lag: i32) -> Vec<(i32, f64)> {
        let n = x.len().min(y.len());
        let mx = x.iter().sum::<f64>() / n as f64;
        let my = y.iter().sum::<f64>() / n as f64;
        let sx: f64 = x.iter().map(|v| (v - mx).powi(2)).sum::<f64>().sqrt();
        let sy: f64 = y.iter().map(|v| (v - my).powi(2)).sum::<f64>().sqrt();
        if sx < 1e-12 || sy < 1e-12 { return vec![]; }

        (-max_lag..=max_lag).map(|lag| {
            let mut num = 0.0;
            let mut count = 0;
            for i in 0..n {
                let j = i as i32 + lag;
                if j >= 0 && (j as usize) < n {
                    num += (x[i] - mx) * (y[j as usize] - my);
                    count += 1;
                }
            }
            let r = if count > 0 { num / (sx * sy) } else { 0.0 };
            (lag, r)
        }).collect()
    }
}

/// Mutual information (binned estimation)
pub mod mutual_info {
    pub fn estimate(x: &[f64], y: &[f64], bins: usize) -> f64 {
        let n = x.len().min(y.len());
        if n < bins * 2 { return 0.0; }
        
        let (x_min, x_max) = min_max(x);
        let (y_min, y_max) = min_max(y);
        let x_step = (x_max - x_min) / bins as f64;
        let y_step = (y_max - y_min) / bins as f64;
        
        let mut joint = vec![vec![0u64; bins]; bins];
        let mut mx = vec![0u64; bins];
        let mut my = vec![0u64; bins];
        
        for i in 0..n {
            let xi = ((x[i] - x_min) / x_step).min((bins - 1) as f64) as usize;
            let yi = ((y[i] - y_min) / y_step).min((bins - 1) as f64) as usize;
            joint[xi][yi] += 1;
            mx[xi] += 1;
            my[yi] += 1;
        }
        
        let nf = n as f64;
        let mut mi = 0.0;
        for i in 0..bins {
            for j in 0..bins {
                if joint[i][j] > 0 && mx[i] > 0 && my[j] > 0 {
                    let pxy = joint[i][j] as f64 / nf;
                    let px = mx[i] as f64 / nf;
                    let py = my[j] as f64 / nf;
                    mi += pxy * (pxy / (px * py)).ln();
                }
            }
        }
        mi
    }
    
    fn min_max(data: &[f64]) -> (f64, f64) {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for &v in data { if v < min { min = v; } if v > max { max = v; } }
        (min, max + 1e-12)
    }
}

/// Fisher exact test
pub mod fisher {
    pub fn p_value(a: u64, b: u64, c: u64, d: u64) -> f64 {
        let n = a + b + c + d;
        let log_p_cutoff = log_hypergeometric(a, b, c, d, n);
        
        let mut p = 0.0;
        let row1 = a + b;
        let col1 = a + c;
        
        for x in 0..=row1.min(col1) {
            let y = row1 - x;
            let z = col1 - x;
            let w = n - row1 - z;
            if w > n { continue; } // overflow check
            let log_p = log_hypergeometric(x, y, z, w, n);
            if log_p <= log_p_cutoff + 1e-10 {
                p += log_p.exp();
            }
        }
        p.min(1.0)
    }

    fn log_hypergeometric(a: u64, b: u64, c: u64, d: u64, n: u64) -> f64 {
        log_factorial(a + b) + log_factorial(c + d) + log_factorial(a + c) + log_factorial(b + d)
            - log_factorial(n) - log_factorial(a) - log_factorial(b) - log_factorial(c) - log_factorial(d)
    }

    fn log_factorial(n: u64) -> f64 {
        if n <= 1 { return 0.0; }
        (2..=n).map(|i| (i as f64).ln()).sum()
    }
}

/// Benjamini-Hochberg FDR correction
pub mod fdr {
    pub fn bh_correct(pvals: &[f64]) -> Vec<f64> {
        let m = pvals.len() as f64;
        let mut indexed: Vec<(usize, f64)> = pvals.iter().cloned().enumerate().collect();
        indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let mut q = vec![0.0; pvals.len()];
        let mut prev = 1.0;

        for (rank, (i, p)) in indexed.into_iter().rev().enumerate() {
            let r = (pvals.len() - rank) as f64;
            let val = (p * m / r).min(prev);
            prev = val;
            q[i] = val;
        }
        q
    }
}

/// Bayesian evidence fusion
pub mod bayesian {
    pub fn fuse_signals(prior: f64, likelihoods: &[(f64, f64)]) -> f64 {
        // likelihoods: Vec<(p(signal|true), p(signal|false))>
        let mut log_odds = (prior / (1.0 - prior)).ln();
        for &(p_true, p_false) in likelihoods {
            if p_false > 1e-12 {
                log_odds += (p_true / p_false).ln();
            }
        }
        1.0 / (1.0 + (-log_odds).exp())
    }
}

/// Graph risk propagation
pub mod graph_risk {
    use std::collections::HashMap;

    pub fn propagate(
        adjacency: &HashMap<String, Vec<(String, f64)>>, // node -> [(neighbor, weight)]
        initial_risk: &HashMap<String, f64>,
        hops: u8,
        decay: f64,
    ) -> HashMap<String, f64> {
        let mut risk = initial_risk.clone();
        
        for _ in 0..hops {
            let mut new_risk = risk.clone();
            for (node, neighbors) in adjacency {
                for (neighbor, weight) in neighbors {
                    let propagated = risk.get(node).unwrap_or(&0.0) * weight * decay;
                    let entry = new_risk.entry(neighbor.clone()).or_insert(0.0);
                    *entry = (*entry + propagated).min(1.0);
                }
            }
            risk = new_risk;
        }
        risk
    }
}
```

### 6.2 Recipe Engine

```rust
// crates/recipes/src/engine.rs

use crate::schemas::*;
use anyhow::Result;

pub struct RecipeEngine {
    recipes: Vec<Recipe>,
}

impl RecipeEngine {
    pub fn load(recipes: Vec<Recipe>) -> Self {
        Self { recipes }
    }

    pub async fn evaluate_all(
        &self,
        features: &FeatureStore,
        entity_id: &str,
    ) -> Vec<InsightCandidate> {
        let mut candidates = Vec::new();

        for recipe in &self.recipes {
            if let Some(insight) = self.evaluate_recipe(recipe, features, entity_id).await {
                candidates.push(insight);
            }
        }

        // Rank by impact × confidence
        candidates.sort_by(|a, b| {
            let score_a = a.impact * a.confidence;
            let score_b = b.impact * b.confidence;
            score_b.partial_cmp(&score_a).unwrap()
        });

        candidates
    }

    async fn evaluate_recipe(
        &self,
        recipe: &Recipe,
        features: &FeatureStore,
        entity_id: &str,
    ) -> Option<InsightCandidate> {
        // 1. Check signal presence
        let mut signal_values = Vec::new();
        for signal in &recipe.signals {
            let val = features.get(entity_id, signal)?;
            signal_values.push(val);
        }

        // 2. Apply transforms
        let transformed = apply_transforms(&signal_values, &recipe.transforms);

        // 3. Run statistical test
        let test_result = run_test(&recipe.test, &transformed)?;

        // 4. Check thresholds
        if test_result.effect < recipe.thresholds.min_effect { return None; }
        if test_result.p_value > recipe.thresholds.max_p_value { return None; }

        // 5. Build insight
        Some(InsightCandidate {
            recipe_id: recipe.id.clone(),
            entity_id: entity_id.to_string(),
            confidence: 1.0 - test_result.p_value,
            impact: test_result.effect,
            narrative_template: recipe.narrative_template.clone(),
            actions: recipe.action_playbook.clone(),
            evidence_ids: test_result.evidence_ids,
        })
    }
}
```

---

## 7. Human Intel / POI Synthesis Module

### 7.0 HUMINT Taxonomy & Intelligence Architecture

This module is the most sensitive and highest-value component of ApexIntel. It operates exclusively on **publicly available data** but synthesizes it into actionable human intelligence that rivals what a senior business development consultant would produce after weeks of manual research.

#### 7.0.1 POI Classifications

**A. Buyer-Side POI Types:**

| POI Type | Role Families | Why They Matter | Engagement Priority |
|---|---|---|---|
| **Procurement Decision-Maker** | VP Procurement, CPO, Director of Purchasing, Sourcing Manager | Signs contracts, allocates spend, manages supplier panel | P0 — Critical |
| **Supplier Quality Engineer (SQE)** | SQE Manager, Director Quality, VP Quality | Qualifies/disqualifies suppliers, audits, sets score thresholds | P0 — Critical |
| **Engineering Gatekeeper** | Director Engineering, NPI Manager, VP R&D | Approves DFM capability, specifies process requirements | P1 — High |
| **Operations Leader** | VP Operations, Plant Manager, COO | Drives capacity planning, make/buy decisions | P1 — High |
| **Security/Compliance** | CISO, VP Compliance, Export Control Officer, ITAR Officer | Vendor security requirements, information sharing rules | P1 — High |
| **Executive Sponsor** | CEO, CTO, SVP Supply Chain, Board Members | Strategic sourcing decisions, partnership approvals | P2 — Strategic |
| **Finance Gatekeeper** | CFO, VP Finance, Controller | Budget approval, payment terms, credit assessment | P2 — Strategic |

**B. Ecosystem POI Types:**

| POI Type | Examples | Why They Matter |
|---|---|---|
| **Government/Investment Agency** | FIPA directors, AMDIE officers, CRI directors, Ministry officials | Incentive programs, regulatory guidance, introductions to OEMs |
| **Free-Zone Authority** | TFZ management, TAC directors, El Fejja directors | Land/infrastructure access, tax incentives, neighbor intel |
| **Port/Logistics Authority** | Tanger Med, Rades, port authority execs | Logistics lane intelligence, port congestion, expansion plans |
| **Certification Body** | IATF witness auditors, ISO registrar contacts, NADCAP assessors | Audit scheduling, scope guidance, competitor audit status |
| **Industry Association** | AMICA, FEDELEC, UTICA, ZVEI, IPC, SMTA leaders | Industry trend intelligence, introductions, event access |
| **Standards Committee** | IPC task group chairs, ISO TC committee members, SAE committee | Standards roadmap intelligence, influence on upcoming requirements |
| **Banking/Finance** | Industrial lending officers (BIAT, ATB, Amen Bank, BMCE, Attijariwafa) | Customer financial health signals, investment indicators |
| **Distributor Key Accounts** | Arrow, Avnet, Farnell, Mouser regional directors | Component allocation intelligence, customer forward orders |
| **Consultant/Analyst** | Supply chain consultants, EMS industry analysts | Market intelligence, project leads, customer pain points |
| **Trade Show Organizer** | Electronica, IPC APEX, PCIM program committees | Speaker slot access, exhibitor intelligence, attendee trends |
| **Academic/Research** | University professors working on manufacturing/electronics | Technology trends, graduate recruitment, research partnerships |

#### 7.0.2 Human Insight Definitions

For each POI, ApexIntel derives the following insight categories:

**A. Personal Professional Insights (derived from public artifacts):**

| Insight | Source Signals | Derived Feature | Business Value |
|---|---|---|---|
| Priority Vector | Keyword frequency in speeches/articles/patents | Cost-Quality-Speed-Resilience-Compliance-Security weights | Know what to emphasize in pitch |
| Decision Mode | RFP language, past procurement patterns, public process descriptions | RfpFormal / RelationshipDriven / PilotFirst / AuditFirst / PriceFirst / SpeedFirst | Know how to structure engagement |
| Risk Tolerance | Language analysis ("proven" vs "innovative") | Conservative / Moderate / Aggressive | Know how to position offering |
| Change Appetite | Tenure at company, role change frequency, language patterns | EarlyAdopter / Pragmatist / Conservative / Laggard | Know whether disruptive pitch works |
| Pain Index | Problem/complaint language frequency, negative sentiment ratio | 0.0 to 1.0 scale with topic breakdown | Know which problems to solve |
| Proof Preference | Role family + decision style + past RFP requirements | KPI / Certification / CaseStudy / AuditReady / TechDemo / CostModel | Know what evidence to prepare |
| Communication Style | Artifact analysis (data-heavy vs narrative vs visual) | DataDriven / Narrative / Visual / Relationship | Know how to present |
| Negotiation Profile | Competitive language analysis, public deal strategies | Collaborative / Competitive / Analytical / Accommodating | Know how to negotiate |

**B. OSINT Signals Feeding POI Profiles:**

| Signal | Source | Update Trigger | Impact on Profile |
|---|---|---|---|
| Role change | LinkedIn (public), company pages, press releases | New title/org detected | Reset engagement timing, update priority |
| Speaking engagement | Conference programs, YouTube, trade show agendas | New speaker listing | Update topics of interest, recurrence score |
| Patent filing | USPTO, EPO, WIPO, OMPIC, INNORPI | New patent published | Update tech focus, co-inventor network |
| Article/interview | News sources, industry publications, podcasts | New publication | Update pain index, priority vector, views |
| Standards committee | IPC/ISO/SAE committee rosters | Role addition/change | Update influence score, standards knowledge |
| Co-appearance | Multiple events, papers, panels | New co-occurrence detected | Update relationship graph |
| Company news | Press releases, filings, news | Employer makes news | Update risk/opportunity assessment |
| Social media post | Twitter/X, LinkedIn (public) | New public post | Update views, sentiment, current focus |
| Government appointment | Official gazette, ministry announcements | New role | Update influence, access, authority level |
| Certification audit | IATF OASIS, AS9100 OASIS, NADCAP | Audit schedule/result | Update timing for outreach |

**C. Derived Features (Composites):**

| Feature | Computation | Output |
|---|---|---|
| Influence Score (0-100) | 0.3 × GraphCentrality + 0.4 × RoleSeniority + 0.3 × PublicRecurrence | Numeric score |
| Approach Probability | f(influence, pain_index, change_appetite, recent_trigger) | 0-1 probability of positive reception |
| Optimal Timing | Role tenure + budget cycle + recent events + pain recency | Recommended timing window |
| Channel Recommendation | f(risk_tolerance, change_appetite, influence) | Best contact method |
| Role Drift Score | Δ(role_seniority) + Δ(org) over time | 0-1 (high = recent change) |
| Network Leverage | Shared connections with Starz ecosystem | List of referral paths |

#### 7.0.3 HUMINT Outputs

**Output 1: Stakeholder Map**
- Per-target-company graph of all known POIs
- Roles, influence scores, relationships
- Personality and background profiles, including criminal/deceptive tendencies
- Full psychometric analysis based on all available public and scraped information
- Recommended engagement sequence (who to contact first, who to avoid)
- Gatekeeper identification (who must approve before others engage)
- Vulnerability assessment (financial pressure, legal exposure, loyalty indicators)

**Output 2: Role Drift Alerts**
- Real-time alerts when a tracked POI changes role/company
- "Window of opportunity" flag (new role = new supplier evaluation)
- Risk flag (champion leaves = relationship at risk)

**Output 3: "What They Want To Hear" Briefing**
- Per-POI customized talking points
- Proof pack recommendation (which documents to prepare)
- Topics to emphasize, topics to avoid
- Best opening approach (data-first, relationship-first, demo-first)
- Cultural/regional communication norms

**Output 4: Relationship Graph**
- Interactive graph showing POI-POI connections
- Edge weights: co-appearance count, context (co-speaker, co-inventor, co-author)
- Cluster detection: which POIs form decision committees
- Bridge identification: which POIs connect us to target organizations

**Output 5: Competition Positioning per POI**
- What each POI likely knows about Starz competitors
- Which competitor has strongest relationship with each POI
- Where Starz has advantage vs disadvantage per POI's priority vector
- Recommended differentiation strategy per POI

### 7.1 POI Profile Schema (Full)

```rust
// crates/poi/src/model.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiProfile {
    pub person_id: String,
    pub name: String,
    pub name_variants: Vec<String>, // Arabic, Hebrew, French, transliterations
    pub org: String,
    pub org_id: Option<String>,
    pub current_role: String,
    pub role_family: RoleFamily,
    pub region: String,
    pub country_code: String,
    pub public_bio: String,
    pub public_email: Option<String>,
    pub phone_numbers: Vec<String>,         // scraped from public directories, WHOIS, company pages
    pub personal_email: Option<String>,     // if discoverable from breaches, public records
    
    // Deep background
    pub background: PoiBackground,
    
    // Public artifacts
    pub artifacts: Vec<PoiArtifact>,
    
    // Derived features
    pub priority_vector: PriorityVector,
    pub decision_mode: DecisionMode,
    pub influence: InfluenceProfile,
    pub psychological: PsychProfile,
    pub engagement: EngagementProfile,
    
    // Temporal
    pub role_history: Vec<RoleHistoryEntry>,
    pub last_updated_utc: i64,
    pub profile_completeness: f64, // 0-1
    pub data_freshness_days: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoleFamily {
    Procurement,
    SupplierQuality,
    Engineering,
    Operations,
    Security,
    Executive, 
    Government,
    FreeZoneAuthority,
    PortLogistics,
    CertificationBody,
    IndustryAssociation,
    Distributor,
    Finance,
    Legal,
    Military,        // IDF/defense establishment (Israel, NATO countries)
    Intelligence,    // Former intelligence community (8200, DGST, etc.)
}

/// Deep background profile — all available info for psychometric analysis.
/// Sources: government registries, court records, corporate filings, social
/// media archaeology, breach data, public records, news archives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiBackground {
    // Family & personal network
    pub family_members: Vec<FamilyMember>,       // spouse, children, siblings, parents (from public records)
    pub known_associates: Vec<String>,            // non-professional associates
    pub residential_history: Vec<AddressRecord>,  // from public property records, WHOIS, company filings
    pub nationality: Vec<String>,                 // dual citizenships
    pub languages_spoken: Vec<String>,            // inferred from artifacts, education, postings
    pub education: Vec<EducationRecord>,
    pub military_service: Option<MilitaryRecord>, // especially relevant for Israel (IDF unit, rank)
    
    // Financial background
    pub known_directorships: Vec<Directorship>,   // from company registries (RNE, Companies House, etc.)
    pub beneficial_ownership: Vec<String>,         // UBO registries where available
    pub property_records: Vec<PropertyRecord>,     // public property registries
    pub bankruptcy_history: Vec<LegalRecord>,      // court/registry records
    pub financial_sanctions_match: bool,           // OFAC/EU/UN screening result
    pub pep_status: bool,                          // Politically Exposed Person flag
    
    // Legal & criminal background
    pub court_cases: Vec<CourtCase>,              // from PACER, local court DBs, news
    pub police_reports: Vec<PoliceRecord>,         // where publicly accessible
    pub regulatory_actions: Vec<RegulatoryAction>, // fines, bans, debarments
    pub fraud_indicators: Vec<FraudIndicator>,     // deceptive tendencies from patterns
    pub integrity_score: f64,                      // 0-1, composite integrity assessment
    
    // Digital footprint
    pub breach_exposure: Vec<BreachRecord>,        // HIBP, breach directories
    pub domain_registrations: Vec<String>,          // WHOIS data, domain ownership
    pub social_accounts: Vec<SocialAccount>,        // all discovered social handles
    pub online_aliases: Vec<String>,                // alternative identities discovered
    
    // Behavioral indicators (for psychometric modeling)
    pub lifestyle_indicators: Vec<String>,          // luxury, frugal, risk-taking (from social)
    pub political_leanings: Option<String>,         // inferred from public posts/donations
    pub religious_indicators: Option<String>,        // inferred from public activity
    pub travel_patterns: Vec<String>,               // inferred from check-ins, conference attendance
    pub hobbies_interests: Vec<String>,             // from social media, club memberships
    
    pub background_completeness: f64,               // 0-1
    pub last_deep_scan: i64,                        // timestamp of last background investigation
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyMember {
    pub name: String,
    pub relationship: String,   // "spouse", "child", "sibling", "parent"
    pub occupation: Option<String>,
    pub employer: Option<String>,
    pub source: String,         // where we found this information
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressRecord {
    pub address: String,
    pub country: String,
    pub date_range: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationRecord {
    pub institution: String,
    pub degree: Option<String>,
    pub field: Option<String>,
    pub year: Option<i32>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryRecord {
    pub branch: String,        // "IDF", "French Army", etc.
    pub unit: Option<String>,  // "8200", "Talpiot", "Unit 81", "DGST"
    pub rank: Option<String>,
    pub years: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directorship {
    pub company_name: String,
    pub role: String,          // "Director", "Secretary", "Shareholder"
    pub registry_source: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub company_status: String, // "Active", "Dissolved", "Struck off"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyRecord {
    pub description: String,
    pub location: String,
    pub estimated_value: Option<f64>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourtCase {
    pub case_number: String,
    pub court: String,
    pub case_type: String,    // "civil", "criminal", "bankruptcy", "ip", "commercial"
    pub role: String,         // "plaintiff", "defendant", "witness"
    pub status: String,       // "pending", "resolved", "dismissed", "convicted"
    pub summary: String,
    pub date: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoliceRecord {
    pub jurisdiction: String,
    pub record_type: String,  // "arrest", "investigation", "charge", "conviction"
    pub description: String,
    pub date: Option<String>,
    pub source: String,       // public record source
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegulatoryAction {
    pub agency: String,       // "SEC", "OFAC", "AMF", "ISA", "CMF"
    pub action_type: String,  // "fine", "ban", "warning", "debarment"
    pub description: String,
    pub date: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudIndicator {
    pub indicator_type: String, // "shell_companies", "name_variations", "address_inconsistencies", 
                                // "frequent_company_dissolution", "offshore_structures"
    pub description: String,
    pub confidence: f64,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreachRecord {
    pub breach_name: String,
    pub breach_date: Option<String>,
    pub data_types_exposed: Vec<String>, // "email", "password", "phone", "address"
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialAccount {
    pub platform: String,     // "twitter", "facebook", "linkedin", "instagram", "telegram"
    pub handle: String,
    pub url: String,
    pub follower_count: Option<u64>,
    pub last_active: Option<i64>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityVector {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
    pub resilience: f64,
    pub compliance: f64,
    pub security: f64,
    pub confidence: f64, // how confident we are in this vector
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionMode {
    RfpFormal,          // Structured RFP process, scoring matrix
    RelationshipDriven, // Trust-based, track record matters
    PilotFirst,         // Wants proof via small batch first
    AuditFirst,         // Must pass audit before any business
    PriceFirst,         // Lowest compliant bid wins
    SpeedFirst,         // Fastest delivery wins
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluenceProfile {
    pub overall_score: f64, // 0-100
    pub graph_centrality: f64,
    pub role_seniority: f64,
    pub public_recurrence: f64, // how often they appear publicly
    pub co_appearance_network: Vec<CoAppearance>,
    pub likely_internal_influence: String, // "gatekeeper", "champion", "blocker", "influencer"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoAppearance {
    pub person_id: String,
    pub person_name: String,
    pub context: String, // "co_speaker", "co_inventor", "co_author", "co_panel"
    pub count: u32,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsychProfile {
    // Decision style
    pub decision_style: DecisionStyle,
    pub risk_tolerance: RiskTolerance,
    pub change_appetite: ChangeAppetite,
    pub communication_preference: CommunicationPref,
    
    // Pain points (inferred from public artifacts)
    pub pain_index: f64, // 0-1, how often they mention problems
    pub pain_topics: Vec<PainTopic>,
    
    // Trigger sensitivities
    pub trigger_topics: Vec<String>,
    pub trigger_events: Vec<String>, // "recall", "allocation", "audit_finding", "cyber_incident"
    
    // What convinces them
    pub preferred_proof: Vec<ProofType>,
    
    // Negotiation style (inferred)
    pub negotiation_style: String, // "collaborative", "competitive", "analytical", "accommodating"
    
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionStyle { CostFirst, RiskFirst, QualityFirst, SpeedFirst, ComplianceFirst, BalancedAnalytical }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskTolerance { Conservative, Moderate, Aggressive }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeAppetite { EarlyAdopter, Pragmatist, Conservative, Laggard }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommunicationPref { DataDriven, Narrative, Visual, Relationship }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PainTopic {
    pub topic: String, // "delays", "shortages", "defects", "cyber", "cost_overrun"
    pub frequency: f64,
    pub recency_days: i32,
    pub sentiment: f64, // -1 to 1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofType {
    KpiMetrics,        // "show me your PPM, OTD, yield"
    Certifications,    // "show me your IATF/AS9100"
    CaseStudies,       // "who else do you work with"
    AuditReadiness,    // "can you pass our audit"
    TechDemos,         // "show me your equipment/process"
    CostTransparency,  // "show me your cost breakdown"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementProfile {
    pub what_they_want_to_hear: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String, // "email_formal", "trade_show", "referral", "direct"
    pub best_timing: String,  // "pre_audit", "post_allocation", "budget_cycle", "npi_phase"
    pub recommended_proof_pack: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleHistoryEntry {
    pub role: String,
    pub org: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiArtifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    pub content_summary: String,
    pub url: String,
    pub source_domain: String,
    pub language: String,
    pub ts_utc: i64,
    pub topics: Vec<String>,
    pub key_phrases: Vec<String>,
    pub sentiment: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArtifactKind {
    PressQuote,
    SpeakerBio,
    Patent,
    StandardsRole,
    Interview,
    Podcast,
    Article,
    RoleChange,
    CompanyPageMention,
    GovernmentAppointment,
    AssociationRoster,
    TradeShowPanel,
    PublicPresentation,
}
```

### 7.2 POI Feature Computation

```rust
// crates/poi/src/features.rs

use crate::model::*;

/// Compute priority vector from artifact analysis
pub fn compute_priority_vector(artifacts: &[PoiArtifact]) -> PriorityVector {
    let keyword_weights = [
        ("cost", &["cost", "price", "budget", "savings", "TCO", "should-cost", "تكلفة", "coût", "prix"][..]),
        ("quality", &["quality", "PPM", "defect", "yield", "zero defects", "جودة", "qualité"][..]),
        ("speed", &["speed", "lead time", "fast", "agile", "NPI", "time-to-market", "سرعة", "rapidité"][..]),
        ("resilience", &["resilience", "risk", "disruption", "continuity", "dual source", "مرونة", "résilience"][..]),
        ("compliance", &["compliance", "audit", "regulation", "standard", "certification", "امتثال", "conformité"][..]),
        ("security", &["security", "cyber", "DMARC", "breach", "zero trust", "أمن", "sécurité"][..]),
    ];

    let total_text: String = artifacts.iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let mut scores = [0.0f64; 6];
    let mut total = 0.0;

    for (i, (_, keywords)) in keyword_weights.iter().enumerate() {
        for kw in *keywords {
            scores[i] += total_text.matches(&kw.to_lowercase()).count() as f64;
        }
        // Weight by recency
        for artifact in artifacts {
            let recency_weight = 1.0 / (1.0 + (artifact.ts_utc as f64 / 86400.0 / 365.0));
            let text = format!("{} {}", artifact.title, artifact.content_summary).to_lowercase();
            for kw in *keywords {
                if text.contains(&kw.to_lowercase()) {
                    scores[i] += recency_weight;
                }
            }
        }
        total += scores[i];
    }

    if total > 0.0 {
        for s in &mut scores { *s /= total; }
    }

    PriorityVector {
        cost: scores[0],
        quality: scores[1],
        speed: scores[2],
        resilience: scores[3],
        compliance: scores[4],
        security: scores[5],
        confidence: (artifacts.len() as f64 / 20.0).min(1.0),
    }
}

/// Compute influence score via graph centrality
pub fn compute_influence(
    person_id: &str,
    co_appearances: &[CoAppearance],
    role: &str,
    artifact_count: usize,
) -> InfluenceProfile {
    let graph_centrality = co_appearances.len() as f64 * 5.0; // simple degree centrality
    
    let role_seniority = match role.to_lowercase().as_str() {
        r if r.contains("director") || r.contains("vp") => 80.0,
        r if r.contains("manager") || r.contains("head") => 60.0,
        r if r.contains("lead") || r.contains("senior") => 40.0,
        r if r.contains("minister") || r.contains("secretary") => 90.0,
        r if r.contains("ceo") || r.contains("cto") || r.contains("ciso") => 95.0,
        _ => 20.0,
    };

    let public_recurrence = (artifact_count as f64 * 3.0).min(100.0);

    let overall = (graph_centrality * 0.3 + role_seniority * 0.4 + public_recurrence * 0.3).min(100.0);

    let likely_influence = if overall > 75.0 { "champion" }
        else if role_seniority > 70.0 { "gatekeeper" }
        else if graph_centrality > 50.0 { "influencer" }
        else { "contributor" };

    InfluenceProfile {
        overall_score: overall,
        graph_centrality,
        role_seniority,
        public_recurrence,
        co_appearance_network: co_appearances.to_vec(),
        likely_internal_influence: likely_influence.to_string(),
    }
}

/// Compute psychological profile from artifacts
pub fn compute_psych_profile(artifacts: &[PoiArtifact]) -> PsychProfile {
    let text: String = artifacts.iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    // Decision style inference
    let cost_signals = text.matches("cost").count() + text.matches("price").count() + text.matches("budget").count();
    let quality_signals = text.matches("quality").count() + text.matches("defect").count() + text.matches("ppm").count();
    let speed_signals = text.matches("speed").count() + text.matches("lead time").count() + text.matches("agile").count();
    let risk_signals = text.matches("risk").count() + text.matches("resilien").count() + text.matches("continuity").count();
    let compliance_signals = text.matches("compliance").count() + text.matches("audit").count() + text.matches("regulat").count();

    let max_signal = [cost_signals, quality_signals, speed_signals, risk_signals, compliance_signals]
        .iter().cloned().max().unwrap_or(0);

    let decision_style = if max_signal == cost_signals { DecisionStyle::CostFirst }
        else if max_signal == quality_signals { DecisionStyle::QualityFirst }
        else if max_signal == speed_signals { DecisionStyle::SpeedFirst }
        else if max_signal == risk_signals { DecisionStyle::RiskFirst }
        else if max_signal == compliance_signals { DecisionStyle::ComplianceFirst }
        else { DecisionStyle::BalancedAnalytical };

    // Risk tolerance from language
    let conservative_words = text.matches("proven").count() + text.matches("established").count() + text.matches("reliable").count();
    let aggressive_words = text.matches("innovative").count() + text.matches("disrupt").count() + text.matches("first mover").count();
    
    let risk_tolerance = if conservative_words > aggressive_words * 2 { RiskTolerance::Conservative }
        else if aggressive_words > conservative_words * 2 { RiskTolerance::Aggressive }
        else { RiskTolerance::Moderate };

    // Change appetite
    let change_words = text.matches("transform").count() + text.matches("change").count() + text.matches("new approach").count();
    let stability_words = text.matches("stable").count() + text.matches("consistent").count() + text.matches("proven partner").count();

    let change_appetite = if change_words > stability_words * 2 { ChangeAppetite::EarlyAdopter }
        else if stability_words > change_words * 2 { ChangeAppetite::Conservative }
        else { ChangeAppetite::Pragmatist };

    // Pain index
    let pain_words = ["delay", "shortage", "defect", "failure", "problem", "challenge", "costly", "breach"];
    let pain_count: usize = pain_words.iter().map(|w| text.matches(w).count()).sum();
    let pain_index = (pain_count as f64 / artifacts.len().max(1) as f64 / 5.0).min(1.0);

    PsychProfile {
        decision_style,
        risk_tolerance,
        change_appetite,
        communication_preference: CommunicationPref::DataDriven, // default, refined by LLM
        pain_index,
        pain_topics: vec![], // computed separately
        trigger_topics: vec![],
        trigger_events: vec![],
        preferred_proof: infer_proof_preference(&decision_style),
        negotiation_style: "analytical".to_string(),
        confidence: (artifacts.len() as f64 / 10.0).min(1.0),
    }
}

fn infer_proof_preference(style: &DecisionStyle) -> Vec<ProofType> {
    match style {
        DecisionStyle::CostFirst => vec![ProofType::CostTransparency, ProofType::KpiMetrics],
        DecisionStyle::QualityFirst => vec![ProofType::KpiMetrics, ProofType::Certifications, ProofType::AuditReadiness],
        DecisionStyle::SpeedFirst => vec![ProofType::TechDemos, ProofType::CaseStudies],
        DecisionStyle::RiskFirst => vec![ProofType::CaseStudies, ProofType::Certifications, ProofType::AuditReadiness],
        DecisionStyle::ComplianceFirst => vec![ProofType::Certifications, ProofType::AuditReadiness],
        DecisionStyle::BalancedAnalytical => vec![ProofType::KpiMetrics, ProofType::CaseStudies, ProofType::Certifications],
    }
}
```

### 7.3 "What To Say" Recommendation Generator

```rust
// crates/poi/src/engagement.rs

use crate::model::*;

pub fn generate_engagement_profile(poi: &PoiProfile) -> EngagementProfile {
    let role = &poi.role_family;
    let psych = &poi.psychological;
    let priority = &poi.priority_vector;

    let what_they_want_to_hear = match role {
        RoleFamily::Procurement => vec![
            "Total cost of ownership breakdown".into(),
            "Lead time certainty with SLA".into(),
            "Risk removal: dual source, buffer stock".into(),
            "Compliance readiness (IATF/ISO/RoHS)".into(),
        ],
        RoleFamily::SupplierQuality => vec![
            "PPM performance data".into(),
            "Control plan & process capability (Cpk)".into(),
            "Traceability system overview".into(),
            "Audit readiness package".into(),
            "8D/corrective action responsiveness".into(),
        ],
        RoleFamily::Engineering => vec![
            "DFM/DFT collaboration process".into(),
            "Fast iteration on prototypes".into(),
            "Test strategy (ICT/AOI/functional)".into(),
            "BOM optimization capability".into(),
        ],
        RoleFamily::Operations => vec![
            "Line stability and changeover discipline".into(),
            "Escalation path clarity".into(),
            "OTD performance metrics".into(),
            "Capacity flexibility (surge/ramp)".into(),
        ],
        RoleFamily::Security => vec![
            "DMARC/SPF/DKIM posture".into(),
            "Vendor access control policy".into(),
            "Incident response and transparency".into(),
            "Third-party risk management".into(),
        ],
        RoleFamily::Executive => vec![
            "Strategic partnership value".into(),
            "Growth capacity and roadmap".into(),
            "Regional advantage (nearshore/compliance)".into(),
        ],
        RoleFamily::Government | RoleFamily::FreeZoneAuthority => vec![
            "Job creation and investment plans".into(),
            "Technology transfer potential".into(),
            "Export growth contribution".into(),
            "Compliance with local content requirements".into(),
        ],
        _ => vec!["Reliability and competence".into()],
    };

    let opening_topics = match &psych.decision_style {
        DecisionStyle::CostFirst => vec!["TCO analysis".into(), "cost optimization track record".into()],
        DecisionStyle::QualityFirst => vec!["quality metrics".into(), "zero-defect philosophy".into()],
        DecisionStyle::SpeedFirst => vec!["rapid proto capability".into(), "NPI speed".into()],
        DecisionStyle::RiskFirst => vec!["supply chain resilience".into(), "dual-source strategy".into()],
        DecisionStyle::ComplianceFirst => vec!["certification portfolio".into(), "audit history".into()],
        DecisionStyle::BalancedAnalytical => vec!["data-driven partnership".into(), "balanced scorecard".into()],
    };

    let avoid_topics = if psych.pain_index > 0.5 {
        vec!["Don't remind them of recent failures".into()]
    } else {
        vec![]
    };

    let best_channel = match &psych.change_appetite {
        ChangeAppetite::EarlyAdopter => "direct_outreach",
        ChangeAppetite::Pragmatist => "trade_show_referral",
        ChangeAppetite::Conservative => "referral_trusted_partner",
        ChangeAppetite::Laggard => "existing_relationship_only",
    };

    let recommended_proof_pack: Vec<String> = psych.preferred_proof.iter().map(|p| match p {
        ProofType::KpiMetrics => "PPM/OTD/yield dashboard snapshot".into(),
        ProofType::Certifications => "Certificate portfolio (IATF/AS9100/ISO13485)".into(),
        ProofType::CaseStudies => "Relevant customer case studies".into(),
        ProofType::AuditReadiness => "Pre-audit self-assessment results".into(),
        ProofType::TechDemos => "Equipment list + process capability demo video".into(),
        ProofType::CostTransparency => "Should-cost model breakdown".into(),
    }).collect();

    EngagementProfile {
        what_they_want_to_hear,
        opening_topics,
        avoid_topics,
        best_channel: best_channel.to_string(),
        best_timing: infer_best_timing(poi),
        recommended_proof_pack,
    }
}

fn infer_best_timing(poi: &PoiProfile) -> String {
    // Check if they recently changed role -> early now (first 90 days)
    if poi.psychological.pain_index > 0.6 {
        return "immediately_pain_driven".to_string();
    }
    match &poi.role_family {
        RoleFamily::Procurement => "budget_cycle_q4_q1".to_string(),
        RoleFamily::SupplierQuality => "pre_audit_season".to_string(),
        RoleFamily::Engineering => "npi_phase_early".to_string(),
        _ => "anytime_with_trigger".to_string(),
    }
}
```

---

## 8. LLM Continuous Learning Loop

### 8.0 Model Selection & Training Strategy

#### 8.0.1 Primary Model: Qwen3-30B-A3B (CPU-Only Local Deployment)

**Why Qwen3-30B-A3B:**
- **Best-in-class for multilingual**: Native Arabic, Hebrew, French, English, German, Chinese, Japanese, Korean support — critical for TN/MA/IL/CN/JP/KR/EU regions
- **Benchmark performance**: Outperforms Llama-3.1-70B on multilingual tasks and structured generation
- **MoE architecture (30B total, 3B active per token)**: Only 3B parameters computed per forward pass — excellent for CPU inference. Comparable latency to dense 3–7B models while retaining 30B-quality reasoning.

**Target Hardware: Hetzner EX44**
- Intel Core i5-13500 (6P + 8E cores, 20 threads)
- 64 GB DDR4 ECC RAM
- 2× 512 GB NVMe SSD (RAID-1 or separate OS + data)
- No GPU — fully CPU-bound inference

**Deployment Stack:**
```yaml
llm_deployment:
  primary:
    model: Qwen3-30B-A3B-Q4_K_M.gguf
    engine: llama-server  # llama.cpp HTTP server, OpenAI-compatible API
    hardware: Hetzner EX44 (i5-13500, 64 GB RAM, no GPU)
    quantization: GGUF Q4_K_M  # ~17 GB on disk, ~20 GB resident
    max_context: 8192   # limited by RAM headroom for OS + Postgres + services
    n_threads: 14       # all P+E cores
    batch_size: 512     # llama.cpp -b flag, prompt processing batch
    n_parallel: 2       # concurrent request slots (RAM-limited)
    mlock: true         # pin model in RAM, avoid swap
    estimated_throughput: ~15-25 tok/s per slot (prompt ~100+ tok/s with flash-attn)
    port: 8080
    
  lightweight:
    model: Qwen3-30B-A3B-Q4_K_M.gguf
    engine: same llama-server instance (port 8080)
    config: max_tokens=1024, timeout=120s  # shorter output for fast tasks
    use_for:
      - simple_classification_tasks
      - entity_extraction
      - real_time_low_latency_tasks
```

**llama-server launch command:**
```bash
llama-server \\
  --model /models/Qwen3-30B-A3B-Q4_K_M.gguf \\
  --host 0.0.0.0 --port 8080 \\
  --ctx-size 8192 \\
  --n-gpu-layers 0 \\
  --threads 14 \\
  --batch-size 512 \\
  --parallel 2 \\
  --mlock \\
  --flash-attn \\
  --cont-batching
```

#### 8.0.2 Fine-Tuning Strategy

**Phase 1: Domain Corpus Pre-Training (LoRA)**

Fine-tune Qwen3-30B-A3B with LoRA (Low-Rank Adaptation) on an EMS/manufacturing domain corpus:

```yaml
finetuning_phase1:
  method: LoRA
  rank: 64
  alpha: 128
  dropout: 0.05
  target_modules: [q_proj, k_proj, v_proj, o_proj, gate_proj, up_proj, down_proj]
  learning_rate: 2e-5
  epochs: 3
  batch_size: 4
  gradient_accumulation: 8
  
  training_data:
    sources:
      # Domain-specific text corpus (~500K documents)
      - type: "ipc_standards_text"
        desc: "IPC-A-610, IPC-J-STD-001, IPC-7711/7721 text content"
        size: "~2000 pages"
      
      - type: "iatf_16949_text"
        desc: "IATF 16949 quality management standard text"
        size: "~500 pages"
      
      - type: "ems_industry_articles"
        desc: "Articles from CircuitsAssembly, SMTnet, EMSNow, Evertiq (2019-2024)"
        size: "~50K articles"
      
      - type: "trade_show_proceedings"
        desc: "IPC APEX EXPO, Electronica, PCIM technical proceedings"
        size: "~5K papers"
      
      - type: "patent_abstracts"
        desc: "EMS/PCB/SMT patent abstracts from USPTO/EPO (2015-2024)"
        size: "~100K abstracts"
      
      - type: "procurement_documents"
        desc: "Public tender documents from TED, TUNEPS, SAM.gov"
        size: "~20K documents"
      
      - type: "north_africa_business"
        desc: "Business articles from L'Economiste, Medias24, Leaders.com.tn (FR/AR)"
        size: "~30K articles"
      
      - type: "israel_tech_business"
        desc: "Articles from Globes, Calcalist, CTech, TheMarker (HE/EN)"
        size: "~40K articles"
      
      - type: "israel_defense_industry"
        desc: "Israel Defense magazine, SIBAT reports, IAI/Elbit/Rafael public docs"
        size: "~5K documents"
      
      - type: "hebrew_procurement_corpus"
        desc: "Israeli government tenders (mr.gov.il), IIA grants, TASE filings"
        size: "~15K documents"
      
      - type: "china_electronics_industry"
        desc: "Articles from Caixin, SCMP, 36Kr, EE Times China, DigiTimes (ZH/EN)"
        size: "~60K articles"
      
      - type: "china_semiconductor_policy"
        desc: "MIIT policy documents, CSIA reports, China Semicon News, Made in China 2025"
        size: "~10K documents"
      
      - type: "china_procurement_corpus"
        desc: "Chinese government tenders (ccgp.gov.cn), MOFCOM regulations, Tianyancha/Qichacha data"
        size: "~25K documents"
      
      - type: "east_asia_supply_chain"
        desc: "DigiTimes Asia, Nikkei Asia, TrendForce, SEMI reports (EN/ZH/JP/KO)"
        size: "~30K articles"
      
      - type: "competitor_web_content"
        desc: "Crawled capability pages, press releases from 30+ competitors"
        size: "~10K pages"
      
      - type: "iso_standards_guides"
        desc: "ISO 9001, 14001, 45001, 13485 guidance documents"
        size: "~1000 pages"
      
      - type: "supply_chain_reports"
        desc: "McKinsey, Deloitte, BCG supply chain reports (public)"
        size: "~500 reports"
```

**Phase 2: Instruction Fine-Tuning (Task-Specific)**

```yaml
finetuning_phase2:
  method: LoRA (on top of Phase 1 adapter)
  learning_rate: 1e-5
  epochs: 5
  
  instruction_datasets:
    # Hand-curated training examples for each LLM task
    
    - task: recipe_hypothesis_generation
      format: |
        SYSTEM: You are an OSINT analyst generating insight recipes...
        USER: Pattern candidate: {signals, effects, stats}
        ASSISTANT: {complete recipe JSON}
      examples: 500  # manually curated
      
    - task: poi_synthesis
      format: |
        SYSTEM: You are synthesizing professional intelligence about a POI...
        USER: Recent artifacts: {list of public artifacts}
        ASSISTANT: {What Changed / What It Implies / How To Approach}
      examples: 300  # manually curated
      
    - task: weekly_memo_generation
      format: |
        SYSTEM: You are writing a weekly strategy memo for an EMS GM...
        USER: This week's insights: {ranked list with evidence}
        ASSISTANT: {structured memo with executive summary, key findings, recommendations}
      examples: 100  # manually curated from template memos
      
    - task: company_dossier_synthesis
      format: |
        SYSTEM: You are generating a company intelligence dossier...
        USER: Entity data: {company profile, capabilities, certifications, events, graph}
        ASSISTANT: {structured dossier with capabilities assessment, risk, opportunity, approach}
      examples: 200  # manually curated
      
    - task: narrative_rendering
      format: |
        SYSTEM: You are rendering an insight narrative from recipe + evidence...
        USER: Recipe: {recipe}, Evidence: {evidence slots filled}
        ASSISTANT: {natural language insight with citations}
      examples: 500  # generated from production recipes + reviewed
      
    - task: entity_extraction
      format: |
        SYSTEM: Extract structured entities from this text...
        USER: {raw text from web page}
        ASSISTANT: {JSON with companies, persons, roles, capabilities, certifications}
      examples: 1000  # from annotated web pages
      
    - task: competitive_analysis
      format: |
        SYSTEM: You are comparing Starz Electronics capabilities vs a competitor...
        USER: Starz profile: {...}, Competitor profile: {...}
        ASSISTANT: {structured comparison with advantages, gaps, recommendations}
      examples: 100
```

#### 8.0.3 Evaluation & Testing Framework

```yaml
llm_evaluation:
  # Automated evaluation runs weekly on held-out test sets
  
  recipe_quality:
    test_set_size: 100
    metrics:
      - json_validity_rate          # target: >99%
      - schema_compliance_rate      # target: >98%
      - narrative_quality_score     # human eval 1-5, target: >3.5
      - action_specificity_score    # human eval 1-5, target: >3.5
      - evidence_grounding_rate     # does narrative cite evidence?, target: >95%
      - no_hallucination_rate       # manual check for fabricated data, target: >99%
      - regional_accuracy           # correct geo/industry applicability, target: >95%
  
  poi_synthesis_quality:
    test_set_size: 50
    metrics:
      - factual_accuracy           # only cites provided artifacts, target: >99%
      - background_depth           # completeness of deep background profile, target: >80%
      - actionability_score        # useful for engagement, target: >3.5/5
      - psychometric_accuracy      # personality/decision-style prediction, target: >70%
      - multilingual_quality       # FR/AR/HE/ZH/JA/KO/EN output quality, target: >3.5/5
  
  memo_quality:
    test_set_size: 20
    metrics:
      - executive_readability      # Flesch-Kincaid score, target: grade 12-14
      - insight_density            # insights per page, target: >3
      - action_concreteness        # vague vs specific, target: >80% specific
      - evidence_citation_rate     # target: >90%
  
  entity_extraction:
    test_set_size: 200
    metrics:
      - precision                  # target: >90%
      - recall                     # target: >85%
      - f1_score                   # target: >87%
      - multilingual_f1            # per-language F1, target: >80%
  
  regression_tests:
    # Golden examples that must always pass
    - "Known competitor page → correct capabilities extracted"
    - "Known POI articles → correct priority vector direction"
    - "Known tender document → correct entity extraction"
    - "Template recipe → valid JSON matching schema"
    - "Arabic text input → correct language handling"
    - "Hebrew text input → correct language handling"
    - "French procurement text → correct keyword extraction"
    - "Israeli defense procurement text → correct entity extraction"
    - "Chinese text input → correct language handling"
    - "Japanese text input → correct language handling"
    - "Korean text input → correct language handling"
    - "Chinese government tender → correct entity extraction"

  adversarial_tests:
    # Edge cases and attack vectors
    - "Prompt injection in web page text → model ignores injection"
    - "Contradictory evidence → model flags uncertainty"
    - "Empty/minimal input → model produces valid null response"
    - "Extremely long context → model handles gracefully"
    - "Mixed language input (FR+AR+HE+ZH+EN) → correct processing"
```

#### 8.0.4 Data Structures to Enable Learning

Two key stores enable the continuous learning loop:

**A) Outcome Events (OSINT-Based "Labels")**

Even without Starz internal data, outcomes are defined from OSINT:

```rust
// crates/core/src/outcomes.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutcomeEvent {
    RfQPosted,                       // public tender/RFQ detected
    CertificationUpdate,             // cert status change (IATF, AS9100, ISO)
    PlantExpansionSignal,            // permits, hiring surge, construction
    DistressSignal,                  // financial trouble, layoffs, negative news
    SecurityImpersonationDetected,   // lookalike domain, phishing
    SupplierBreachDisclosed,         // data breach, cyber incident
    PortShock,                       // congestion change-point, closure
    CompetitorCapabilityShift,       // cert change, new service page
    RoleChange,                      // POI title/org change
    AllocationWave,                  // component shortage signals
    RegulatoryShift,                 // tariff, sanctions, compliance change
    MnAEvent,                        // merger/acquisition announcement
    PriceWarSignal,                  // competitor pricing aggression
    QualityEscapeEvent,              // recall, defect notice
    ContractAward,                   // public contract award notice
}
```

These are extracted from public sources and serve as supervised labels for pattern mining.

**B) Feature Store (Materialized Transforms)**

For each entity/site/segment and time bucket:

```rust
// crates/store/src/feature_store.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRow {
    pub entity_id: String,
    pub entity_type: String,      // "company", "site", "person", "domain"
    pub time_bucket: i64,          // epoch of bucket start
    pub bucket_size_days: i32,     // 1, 7, or 30
    
    // Raw signal counts per observation type
    pub signal_counts: HashMap<String, f64>,
    
    // Diffs and % changes
    pub diffs: HashMap<String, f64>,
    pub pct_changes: HashMap<String, f64>,
    
    // Regime flags (from change-point detection)
    pub regime_flags: HashMap<String, bool>,
    
    // Volatility windows
    pub volatility: HashMap<String, f64>,
    
    // Topic drift scores
    pub topic_drift: HashMap<String, f64>,
    
    // Graph neighbor aggregation (1-hop and 2-hop)
    pub neighbor_agg_1hop: HashMap<String, f64>,
    pub neighbor_agg_2hop: HashMap<String, f64>,
    
    // POI-specific features
    pub poi_pain_index: Option<f64>,
    pub poi_role_drift: Option<f64>,
    pub poi_influence_delta: Option<f64>,
}
```

This is what PatternMiner scans nightly.

#### 8.0.5 Pattern Discovery (Statistical Robustness First)

**Candidate Pattern Families:**

| Family | Example | Statistical Test |
|---|---|---|
| Lead-lag | "SQE job posts (lag 30–60d) → RFQ probability" | Cross-correlation + Fisher exact |
| Interaction effects | "Port volatility × product launch window → late delivery risk" | Conditional mutual information |
| Segment-specific | "Pattern holds in automotive tier-2 in EU AND Israel defense, not in industrial" | Stratified Fisher exact |
| Graph propagation | "Supplier breach → increases distress probability for linked customers" | Hazard uplift with graph neighbor features |
| POI-driven | "New CPO hire (lag 60–90d) → supplier panel restructuring" | Survival analysis with POI triggers |
| Regime interaction | "Commodity volatility regime × FX regime → pricing pressure" | Conditional hazard model |

**Robustness Gates (Mandatory):**

A candidate must pass ALL of the following:

| Gate | Threshold | Rationale |
|---|---|---|
| Effect size | Uplift > 1.5× OR MI > 0.1 | Must be materially significant |
| Significance | p < 0.01 (raw) | Standard statistical significance |
| FDR correction | q < 0.05 (Benjamini–Hochberg) | Controls false discovery rate |
| Temporal stability | Works in ≥3 of 4 time slices | Not a fluke |
| Entity stability | Works across ≥5 entities | Not driven by one outlier |
| Negative control | Effect vanishes when time/entities shuffled | Confirms causal direction |
| False alarm simulation | Does not exceed daily/weekly FP budget | Operationally acceptable |
| Counterfactual | The insight changes meaningfully if one signal is removed | Not redundant |

Only after all gates pass does the LLM write a recipe/narrative.

#### 8.0.6 Continuous Learning Loop Architecture

```
                    ┌──────────────────────────────────────────────────────────┐
                    │                  CONTINUOUS LEARNING LOOP                 │
                    └──────────────────────────────────────────────────────────┘
                    
NIGHTLY CYCLE:
  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
  │  Feature     │────▶│  Pattern    │────▶│  Qwen3-Next │────▶│  StatGate   │
  │  Store       │     │  Miner      │     │  Hypothesis │     │  + Backtest │
  │  (Postgres)  │     │  (Rust)     │     │  Builder    │     │  (Rust)     │
  └─────────────┘     └─────────────┘     └─────────────┘     └──────┬──────┘
                                                                       │
                                                              ┌────────▼────────┐
                                                              │  STAGING POOL   │
                                                              │  (observe 4wk)  │
                                                              └────────┬────────┘
                                                                       │
WEEKLY:                                                       ┌────────▼────────┐
  ┌─────────────┐                                             │  PROMOTION      │
  │  Human       │◀───── review flagged cases ─────────────── │  BOARD          │
  │  Analyst     │                                             │  (auto + human) │
  │  (optional)  │                                             └────────┬────────┘
  └─────────────┘                                                       │
                                                                       ▼
                                                              ┌─────────────────┐
                                                              │  PRODUCTION     │
                                                              │  RECIPE REGISTRY│
                                                              └─────────────────┘
                    
ALWAYS RUNNING:
  ┌─────────────────────────────────────────────────────────┐
  │  Performance Monitor: track precision/recall/FPR        │
  │  per recipe per week                                     │
  │  Auto-deprecate: precision < 0.5 → deprecated           │
  │  Auto-promote: precision > 0.85 for 4+ weeks → promote │
  │  Drift detector: KL-divergence on feature distributions │
  │  Model stale alert: if drift > threshold → retrain      │
  └─────────────────────────────────────────────────────────┘

MONTHLY:
  ┌─────────────────────────────────────────────────────────┐
  │  Full evaluation suite on held-out test sets             │
  │  Compare local Qwen3-Next vs GPT-4o on same tasks       │
  │  If GPT-4o significantly better → consider API migration │
  │  Retrain LoRA if domain corpus updated significantly     │
  │  Human review of top-20 highest-impact recipes           │
  └─────────────────────────────────────────────────────────┘
```

#### 8.0.5 Model Serving Configuration

```rust
// crates/llm/src/config.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub primary: ModelConfig,
    pub fallback: Option<ModelConfig>,
    pub lightweight: Option<ModelConfig>,
    pub routing: RoutingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model_name: String,
    pub provider: LlmProvider,
    pub base_url: String,
    pub api_key: Option<String>,
    pub max_tokens: u32,
    pub temperature: f64,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmProvider {
    LlamaCpp,      // llama-server running Qwen3-30B-A3B on CPU (GGUF Q4_K_M)
    OpenAi,        // OpenAI API (GPT-4o fallback)
    AzureOpenAi,   // Azure OpenAI (alternative API)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Tasks that always use local model (data never leaves premises)
    pub local_only_tasks: Vec<String>,
    /// Tasks that can fall back to API if local is overloaded
    pub api_fallback_tasks: Vec<String>,
    /// Maximum concurrent API requests
    pub max_api_concurrent: u32,
    /// Monthly API spend cap in USD
    pub monthly_api_budget_usd: f64,
    /// Current month spend (tracked in Redis)
    pub spend_tracking_key: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            primary: ModelConfig {
                model_name: "Qwen3-30B-A3B-Q4_K_M".into(),
                provider: LlmProvider::LlamaCpp,
                base_url: "http://localhost:8080".into(),
                api_key: None,
                max_tokens: 4096,
                temperature: 0.2,
                timeout_seconds: 300,
            },
            fallback: Some(ModelConfig {
                model_name: "gpt-4o".into(),
                provider: LlmProvider::OpenAi,
                base_url: "https://api.openai.com/v1".into(),
                api_key: None, // from env
                max_tokens: 4096,
                temperature: 0.2,
                timeout_seconds: 60,
            }),
            lightweight: Some(ModelConfig {
                model_name: "Qwen3-30B-A3B-Q4_K_M".into(),
                provider: LlmProvider::LlamaCpp,
                base_url: "http://localhost:8080".into(),
                api_key: None,
                max_tokens: 1024,
                temperature: 0.1,
                timeout_seconds: 120,
            }),
            routing: RoutingConfig {
                local_only_tasks: vec![
                    "poi_synthesis".into(),
                    "entity_extraction".into(),
                    "competitive_analysis".into(),
                ],
                api_fallback_tasks: vec![
                    "recipe_hypothesis".into(),
                    "memo_generation".into(),
                    "narrative_rendering".into(),
                ],
                max_api_concurrent: 5,
                monthly_api_budget_usd: 500.0,
                spend_tracking_key: "llm:monthly_spend".into(),
            },
        }
    }
}
```

### 8.1 Architecture

```
NIGHTLY:
  ┌──────────────┐     ┌──────────────────┐     ┌──────────────┐
  │ PatternMiner │────▶│ LLM Hypothesis   │────▶│ StatGate +   │
  │ (stats-first)│     │ Builder          │     │ Backtest     │
  └──────────────┘     └──────────────────┘     └──────┬───────┘
                                                        │
                                                        ▼ staging
WEEKLY:                                          ┌──────────────┐
  Evaluate staged ──────────────────────────────▶│ Promotion    │
  recipes over N weeks                           │ Board        │
                                                 └──────┬───────┘
                                                        │
                                                        ▼ production
                                                 ┌──────────────┐
                                                 │ Recipe       │
                                                 │ Registry     │
                                                 └──────────────┘
ALWAYS:
  Track precision/recall per recipe
  Auto-deprecate degrading recipes
  Retire recipes below threshold
```

### 8.2 LLM Client (Model-Agnostic)

```rust
// crates/llm/src/lib.rs

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String>;
    async fn generate_text(&self, system: &str, user: &str) -> Result<String>;
}

/// OpenAI-compatible implementation
pub struct OpenAiClient {
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
            model: model.unwrap_or_else(|| "gpt-4o".into()),
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let resp = client.post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": &self.model,
                "response_format": { "type": "json_object" },
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user },
                ],
                "temperature": 0.2,
                "max_tokens": 4096,
            }))
            .send().await?;
        
        let body: serde_json::Value = resp.json().await?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("{}").to_string())
    }

    async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let resp = client.post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": &self.model,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user },
                ],
                "temperature": 0.3,
                "max_tokens": 8192,
            }))
            .send().await?;
        
        let body: serde_json::Value = resp.json().await?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }
}

/// Local model (llama-server, same instance for primary + lightweight)
pub struct LocalLlmClient {
    base_url: String,
    model: String,
}

#[async_trait]
impl LlmClient for LocalLlmClient {
    async fn generate_json(&self, system: &str, user: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let resp = client.post(format!("{}/v1/chat/completions", self.base_url))
            .json(&serde_json::json!({
                "model": &self.model,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user },
                ],
                "temperature": 0.2,
            }))
            .send().await?;
        let body: serde_json::Value = resp.json().await?;
        Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("{}").to_string())
    }

    async fn generate_text(&self, system: &str, user: &str) -> Result<String> {
        self.generate_json(system, user).await // same endpoint
    }
}
```

### 8.3 Pattern Miner

```rust
// crates/learning/src/miner.rs

use anyhow::Result;
use crate::stats;

pub struct MinerConfig {
    pub max_lag_days: i32,
    pub min_effect: f64,
    pub max_p: f64,
    pub min_stability: f64,
    pub time_splits: usize,
    pub entity_min_count: usize,
}

impl Default for MinerConfig {
    fn default() -> Self {
        Self {
            max_lag_days: 90,
            min_effect: 1.5, // odds ratio
            max_p: 0.01,
            min_stability: 0.6,
            time_splits: 4,
            entity_min_count: 5,
        }
    }
}

pub struct PatternCandidate {
    pub outcome: String,
    pub signals: Vec<String>,
    pub best_lag_days: i32,
    pub effect_size: f64,
    pub p_value: f64,
    pub q_value: f64,
    pub stability: f64,
    pub entity_coverage: f64,
    pub segments: Vec<String>,
    pub example_evidence_ids: Vec<String>,
}

pub async fn mine_all_candidates(
    pool: &sqlx::PgPool,
    config: &MinerConfig,
) -> Result<Vec<PatternCandidate>> {
    let mut candidates = Vec::new();

    // Define outcome-signal pairs to test
    let outcome_signal_pairs = vec![
        ("RfQPosted", vec!["JobPost.role_family=Procurement", "JobPost.role_family=SupplierQuality"]),
        ("CertificationUpdate", vec!["JobPost.role_family=Quality", "WebChange.page_type=compliance"]),
        ("PlantExpansion", vec!["JobPost.volume_regime_shift", "WebChange.page_type=facilities"]),
        ("SecurityIncident", vec!["DnsPosture.drift", "NewDomain.similarity>0.8"]),
        ("SupplierDistress", vec!["JobPost.decline", "WebChange.negative_news"]),
        ("PriceWarSignal", vec!["CommodityPrice.volatility_spike", "CompetitorEvent.capacity_expansion"]),
    ];

    for (outcome, signals) in outcome_signal_pairs {
        for signal in &signals {
            if let Ok(mut cands) = mine_one_pair(pool, outcome, signal, config).await {
                candidates.append(&mut cands);
            }
        }
    }

    // FDR correction across all candidates
    let p_values: Vec<f64> = candidates.iter().map(|c| c.p_value).collect();
    let q_values = stats::fdr::bh_correct(&p_values);
    for (i, q) in q_values.into_iter().enumerate() {
        candidates[i].q_value = q;
    }

    // Filter by FDR-corrected threshold
    candidates.retain(|c| c.q_value <= config.max_p);

    // Sort by effect size descending
    candidates.sort_by(|a, b| b.effect_size.partial_cmp(&a.effect_size).unwrap());

    Ok(candidates)
}

async fn mine_one_pair(
    pool: &sqlx::PgPool,
    outcome: &str,
    signal: &str,
    config: &MinerConfig,
) -> Result<Vec<PatternCandidate>> {
    // Load outcome events
    let outcomes: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entity_id::text, extract(epoch from ts_utc)::bigint FROM observations WHERE observation_type = $1"
    ).bind(outcome).fetch_all(pool).await?;

    // Load signal events
    let signals: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entity_id::text, extract(epoch from ts_utc)::bigint FROM observations WHERE observation_type LIKE $1"
    ).bind(format!("%{}%", signal)).fetch_all(pool).await?;

    if outcomes.len() < config.entity_min_count || signals.len() < config.entity_min_count {
        return Ok(vec![]);
    }

    // Sweep lags
    let mut best_candidate: Option<PatternCandidate> = None;

    for lag in -config.max_lag_days..=config.max_lag_days {
        let (a, b, c, d) = build_contingency(&outcomes, &signals, lag, 30); // 30-day window
        if a + b + c + d < 20 { continue; } // not enough data 

        let p = stats::fisher::p_value(a, b, c, d);
        let effect = odds_ratio(a, b, c, d);

        if effect >= config.min_effect && p <= config.max_p {
            // Stability check: test on time splits
            let stability = compute_stability(&outcomes, &signals, lag, config.time_splits);

            if stability >= config.min_stability {
                let candidate = PatternCandidate {
                    outcome: outcome.to_string(),
                    signals: vec![signal.to_string()],
                    best_lag_days: lag,
                    effect_size: effect,
                    p_value: p,
                    q_value: 0.0, // set later by FDR
                    stability,
                    entity_coverage: 0.0,
                    segments: vec![],
                    example_evidence_ids: vec![],
                };

                if best_candidate.as_ref().map_or(true, |b| effect > b.effect_size) {
                    best_candidate = Some(candidate);
                }
            }
        }
    }

    Ok(best_candidate.into_iter().collect())
}

fn build_contingency(
    outcomes: &[(String, i64)],
    signals: &[(String, i64)],
    lag_days: i32,
    window_days: i32,
) -> (u64, u64, u64, u64) {
    use std::collections::{HashMap, HashSet};

    let lag_secs = lag_days as i64 * 86400;
    let window_secs = window_days as i64 * 86400;

    // Group by entity
    let mut outcome_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in outcomes { outcome_map.entry(eid).or_default().push(*ts); }
    
    let mut signal_map: HashMap<&str, Vec<i64>> = HashMap::new();
    for (eid, ts) in signals { signal_map.entry(eid).or_default().push(*ts); }

    let all_entities: HashSet<&str> = outcome_map.keys().chain(signal_map.keys()).cloned().collect();

    let (mut a, mut b, mut c, mut d) = (0u64, 0u64, 0u64, 0u64);

    for entity in all_entities {
        let has_signal = signal_map.get(entity).map_or(false, |ts_list| !ts_list.is_empty());
        let has_outcome = outcome_map.get(entity).map_or(false, |o_ts| {
            if let Some(s_ts) = signal_map.get(entity) {
                o_ts.iter().any(|ot| {
                    s_ts.iter().any(|st| {
                        let shifted = st + lag_secs;
                        (*ot - shifted).abs() <= window_secs
                    })
                })
            } else {
                false
            }
        });

        match (has_signal, has_outcome) {
            (true, true) => a += 1,
            (true, false) => b += 1,
            (false, true) => c += 1,
            (false, false) => d += 1,
        }
    }

    (a, b, c, d)
}

fn odds_ratio(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let num = (a as f64) * (d as f64);
    let den = (b as f64) * (c as f64);
    if den < 1e-12 { return f64::MAX; }
    num / den
}

fn compute_stability(
    outcomes: &[(String, i64)],
    signals: &[(String, i64)],
    lag: i32,
    splits: usize,
) -> f64 {
    if outcomes.is_empty() || signals.is_empty() { return 0.0; }
    
    let min_ts = outcomes.iter().chain(signals.iter()).map(|(_, ts)| *ts).min().unwrap();
    let max_ts = outcomes.iter().chain(signals.iter()).map(|(_, ts)| *ts).max().unwrap();
    let split_size = (max_ts - min_ts) / splits as i64;
    
    let mut positive_splits = 0;
    
    for i in 0..splits {
        let start = min_ts + i as i64 * split_size;
        let end = start + split_size;
        
        let split_outcomes: Vec<_> = outcomes.iter().filter(|(_, ts)| *ts >= start && *ts < end).cloned().collect();
        let split_signals: Vec<_> = signals.iter().filter(|(_, ts)| *ts >= start && *ts < end).cloned().collect();
        
        if split_outcomes.len() < 3 || split_signals.len() < 3 { continue; }
        
        let (a, b, c, d) = build_contingency(&split_outcomes, &split_signals, lag, 30);
        let p = stats::fisher::p_value(a, b, c, d);
        let effect = odds_ratio(a, b, c, d);
        
        if effect > 1.0 && p < 0.1 { positive_splits += 1; }
    }
    
    positive_splits as f64 / splits as f64
}
```

### 8.4 LLM Hypothesis Builder

```rust
// crates/learning/src/hypothesis.rs

use anyhow::Result;
use crate::miner::PatternCandidate;

pub async fn propose_recipe(
    llm: &dyn crate::llm::LlmClient,
    candidate: &PatternCandidate,
    existing_recipes: &[String], // existing recipe IDs to avoid duplicates
) -> Result<serde_json::Value> {
    let system = r#"You are an OSINT intelligence analyst generating insight recipes for an EMS company (Starz Electronics, Morocco/Tunisia/Israel).

OUTPUT: Valid JSON matching this schema:
{
  "id": "string (unique, descriptive)",
  "join": "Entity|Site|Geo|Industry|Lane|Domain",
  "outcome": "string",
  "signals": ["array of signal names"],
  "transforms": [{"type": "Lag", "days": N}, ...],
  "test": {"type": "FisherExact|CrossCorrelation|MutualInformation|HazardUplift"},
  "thresholds": {"min_effect": N, "max_p_value": N, "min_stability": N, "max_false_alarm_rate": N},
  "narrative_template": "string with {{evidence:ID}} placeholders",
  "action_playbook": ["array of concrete actions"],
  "applicability": {"geos": [], "industries": [], "notes": ""}
}

RULES:
- Narrative must use evidence slots: {{evidence:signal_name}}
- Actions must be concrete and EMS-specific
- Include regional applicability (Tunisia, Morocco, Israel, China, East Asia, EU, US, EMEA)
- Thresholds must be consistent with the candidate's actual statistics
- Do not duplicate existing recipe IDs"#;

    let user = format!(
        r#"Pattern candidate:
- outcome: {}
- signals: {:?}
- best_lag_days: {}
- effect_size: {:.3}
- p_value: {:.6}
- stability: {:.2}
- segments: {:?}

Existing recipe IDs to avoid: {:?}

Generate a Recipe JSON for this pattern."#,
        candidate.outcome, candidate.signals, candidate.best_lag_days,
        candidate.effect_size, candidate.p_value, candidate.stability,
        candidate.segments, existing_recipes
    );

    let json_str = llm.generate_json(system, &user).await?;
    let recipe: serde_json::Value = serde_json::from_str(&json_str)?;
    
    // Validate required fields
    assert!(recipe.get("id").is_some(), "Recipe must have id");
    assert!(recipe.get("signals").is_some(), "Recipe must have signals");
    assert!(recipe.get("narrative_template").is_some(), "Recipe must have narrative");
    assert!(recipe.get("action_playbook").is_some(), "Recipe must have actions");
    
    
    Ok(recipe)
}
```

### 8.5 POI Continuous Update Loop

```rust
// crates/poi/src/updater.rs

use anyhow::Result;
use crate::model::*;
use crate::features;

pub struct PoiUpdater {
    pool: sqlx::PgPool,
    llm: Box<dyn crate::llm::LlmClient>,
}

impl PoiUpdater {
    /// Nightly POI profile refresh
    pub async fn nightly_refresh(&self) -> Result<Vec<PoiUpdateReport>> {
        let mut reports = Vec::new();

        // Get all active POIs
        let pois: Vec<(String, String)> = sqlx::query_as(
            "SELECT id::text, name FROM persons WHERE updated_at < now() - interval '1 day'"
        ).fetch_all(&self.pool).await?;

        for (person_id, name) in pois {
            match self.refresh_one(&person_id).await {
                Ok(report) => reports.push(report),
                Err(e) => tracing::warn!("Failed to refresh POI {}: {}", name, e),
            }
        }

        Ok(reports)
    }

    async fn refresh_one(&self, person_id: &str) -> Result<PoiUpdateReport> {
        // 1. Fetch all artifacts for this person
        let artifacts: Vec<PoiArtifact> = sqlx::query_as(
            "SELECT * FROM poi_artifacts WHERE person_id = $1::uuid ORDER BY ts_utc DESC"
        ).bind(person_id).fetch_all(&self.pool).await?;

        // 2. Recompute features
        let priority = features::compute_priority_vector(&artifacts);
        let co_appearances = self.get_co_appearances(person_id).await?;
        let role = self.get_current_role(person_id).await?;
        let influence = features::compute_influence(person_id, &co_appearances, &role, artifacts.len());
        let psych = features::compute_psych_profile(&artifacts);

        // 3. LLM synthesis: what changed, what it implies, how to approach
        let recent_artifacts: Vec<_> = artifacts.iter()
            .filter(|a| a.ts_utc > (chrono::Utc::now().timestamp() - 7 * 86400))
            .collect();

        let synthesis = if !recent_artifacts.is_empty() {
            self.llm_synthesize(person_id, &recent_artifacts).await?
        } else {
            None
        };

        // 4. Update database
        sqlx::query(
            "UPDATE persons SET priority_vector = $2, influence_score = $3, \
             role_drift_score = $4, pain_index = $5, updated_at = now() WHERE id = $1::uuid"
        )
        .bind(person_id)
        .bind(serde_json::to_value(&priority)?)
        .bind(influence.overall_score)
        .bind(0.0) // role drift computed separately
        .bind(psych.pain_index)
        .execute(&self.pool).await?;

        Ok(PoiUpdateReport {
            person_id: person_id.to_string(),
            new_artifacts: recent_artifacts.len(),
            priority_changed: true,
            synthesis,
        })
    }

    async fn llm_synthesize(
        &self,
        person_id: &str,
        recent_artifacts: &[&PoiArtifact],
    ) -> Result<Option<String>> {
        let artifacts_text: String = recent_artifacts.iter()
            .map(|a| format!("- [{}] {} ({}): {}", 
                a.kind_str(), a.title, a.url, a.content_summary))
            .collect::<Vec<_>>()
            .join("\n");

        let system = r#"You are an OSINT analyst synthesizing professional intelligence about a person of interest (POI) for an EMS company.

OUTPUT: A brief professional intelligence update with three sections:
1. **What Changed**: factual summary of new public artifacts (cite URLs)
2. **What It Implies**: professional implications for supplier engagement
3. **How To Approach**: specific, actionable guidance for outreach

RULES:
- Only reference the provided artifacts
- Do not speculate about private life
- Focus on professional decision-making implications
- Be specific about EMS/manufacturing context"#;

        let user = format!("Recent public artifacts for POI:\n{}\n\nSynthesize professional intelligence update.", artifacts_text);

        let text = self.llm.generate_text(system, &user).await?;
        Ok(Some(text))
    }

    async fn get_co_appearances(&self, person_id: &str) -> Result<Vec<CoAppearance>> {
        // Query graph_edges for person-person co-appearance edges
        let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
            "SELECT ge.target_id::text, p.name, ge.edge_type, COUNT(*)::bigint \
             FROM graph_edges ge JOIN persons p ON ge.target_id = p.id \
             WHERE ge.source_id = $1::uuid AND ge.source_type = 'person' AND ge.target_type = 'person' \
             GROUP BY ge.target_id, p.name, ge.edge_type"
        ).bind(person_id).fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|(id, name, context, count)| CoAppearance {
            person_id: id,
            person_name: name,
            context,
            count: count as u32,
            last_seen: 0,
        }).collect())
    }

    async fn get_current_role(&self, person_id: &str) -> Result<String> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT current_role FROM persons WHERE id = $1::uuid"
        ).bind(person_id).fetch_optional(&self.pool).await?;
        Ok(role.unwrap_or_default())
    }
}

#[derive(Debug)]
pub struct PoiUpdateReport {
    pub person_id: String,
    pub new_artifacts: usize,
    pub priority_changed: bool,
    pub synthesis: Option<String>,
}
```

### 8.6 Recipe Promotion & Retirement

```rust
// crates/recipes/src/lifecycle.rs

use anyhow::Result;
use chrono::{DateTime, Utc};

pub struct RecipeLifecycle {
    pool: sqlx::PgPool,
}

#[derive(Debug, Clone)]
pub struct RecipePerformance {
    pub recipe_id: String,
    pub weeks_in_staging: u32,
    pub true_positives: u32,
    pub false_positives: u32,
    pub precision: f64,
    pub recall: f64,
    pub evidence_coverage: f64,
}

impl RecipeLifecycle {
    /// Weekly promotion board — move staging recipes to production
    pub async fn run_promotion_board(&self) -> Result<Vec<String>> {
        let staged = self.get_staged_recipes().await?;
        let mut promoted = Vec::new();

        for recipe in staged {
            let perf = self.compute_performance(&recipe.id).await?;

            if perf.weeks_in_staging >= 4
                && perf.precision >= 0.85
                && perf.evidence_coverage >= 0.7
                && perf.false_positives as f64 / (perf.true_positives + perf.false_positives).max(1) as f64 <= 0.02
            {
                self.promote(&recipe.id).await?;
                promoted.push(recipe.id.clone());
                tracing::info!("Promoted recipe {} to production (precision={:.2})", recipe.id, perf.precision);
            }
        }

        Ok(promoted)
    }

    /// Auto-deprecate underperforming production recipes
    pub async fn deprecate_stale(&self) -> Result<Vec<String>> {
        let production = self.get_production_recipes().await?;
        let mut deprecated = Vec::new();

        for recipe in production {
            let perf = self.compute_performance(&recipe.id).await?;

            if perf.precision < 0.5 || perf.evidence_coverage < 0.3 {
                self.deprecate(&recipe.id).await?;
                deprecated.push(recipe.id.clone());
                tracing::warn!("Deprecated recipe {} (precision={:.2})", recipe.id, perf.precision);
            }
        }

        Ok(deprecated)
    }

    async fn get_staged_recipes(&self) -> Result<Vec<RecipeRecord>> {
        // Pull staged recipes from the backing store.
        self.repo.list_by_status("staged").await
    }

    async fn get_production_recipes(&self) -> Result<Vec<RecipeRecord>> {
        // Pull production recipes from the backing store.
        self.repo.list_by_status("production").await
    }

    async fn compute_performance(&self, recipe_id: &str) -> Result<RecipePerformance> {
        // Evaluate model metrics for the given recipe.
        self.metrics_client.performance(recipe_id).await
    }

    async fn promote(&self, recipe_id: &str) -> Result<()> {
        // Move a staged recipe into production and record provenance.
        self.repo.promote(recipe_id).await?;
        self.audit.log("recipe.promoted", recipe_id).await
    }

    async fn deprecate(&self, recipe_id: &str) -> Result<()> {
        // Disable a production recipe that no longer meets thresholds.
        self.repo.deprecate(recipe_id).await?;
        self.audit.log("recipe.deprecated", recipe_id).await
    }
}

struct RecipeRecord {
    id: String,
    status: String,
    created_at: DateTime<Utc>,
}
```

---

## 9. Infrastructure

### 9.1 Crate Layout

```
apex-intel/
├── Cargo.toml                # workspace
├── config/
│   ├── regions.yaml
│   ├── competitors.yaml
│   ├── recipes_seed.yaml     # 240 seed recipes
│   └── poi_targets.yaml      # initial POI watchlist
├── crates/
│   ├── core/                 # types, provenance, config, schemas
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs
│   │       ├── schemas.rs    # Recipe, PatternCandidate, etc.
│   │       ├── entities.rs   # Company, Site, Person, etc.
│   │       └── provenance.rs
│   ├── crawl/                # fetch, robots, throttle, cache, change detection
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── fetcher.rs         # main HTTP fetcher
│   │       ├── robots.rs          # robots.txt parser
│   │       ├── rate_limit.rs      # from CRM-v2 RateLimitManager
│   │       ├── headers.rs         # from CRM-v2 HeaderRandomizer
│   │       ├── proxy.rs           # from CRM-v2 ProxyRotator
│   │       ├── governor.rs        # domain-level rate limiting
│   │       ├── change_detection.rs # hash-based change detection
│   │       ├── search/
│   │       │   ├── mod.rs
│   │       │   ├── engines.rs     # 27+ search engine scrapers
│   │       │   ├── bing.rs
│   │       │   ├── duckduckgo.rs
│   │       │   ├── google_cse.rs  # paid API fallback
│   │       │   ├── brave.rs
│   │       │   ├── qwant.rs
│   │       │   └── ...
│   │       ├── dns.rs             # DNS posture checking
│   │       └── ct.rs              # Certificate Transparency
│   ├── parse/                # extractors -> observations
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── html.rs            # HTML content extraction
│   │       ├── tender.rs          # procurement portal parsing
│   │       ├── patent.rs          # patent publication parsing
│   │       ├── job_post.rs        # job posting extraction & classification
│   │       ├── cert.rs            # certification registry parsing
│   │       ├── trade_show.rs      # exhibitor/speaker extraction
│   │       ├── press.rs           # press release extraction
│   │       ├── person.rs          # POI extraction from pages
│   │       ├── commodity.rs       # price feed parsing
│   │       ├── multilingual.rs    # language detection + keyword maps
│   │       └── normalizer.rs      # text normalization
│   ├── store/                # postgres + object store + search index
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── postgres.rs
│   │       ├── s3.rs              # MinIO/S3 raw doc storage
│   │       ├── tantivy_index.rs   # full-text search
│   │       └── feature_store.rs   # materialized feature store
│   ├── graph/                # entity resolution, adjacency, neighbor agg
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── entity_resolution.rs
│   │       ├── adjacency.rs
│   │       └── neighbor_agg.rs
│   ├── stats/                # statistical tests
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── changepoint.rs
│   │       ├── anomaly.rs
│   │       ├── correlation.rs
│   │       ├── mutual_info.rs
│   │       ├── fisher.rs
│   │       ├── hazard.rs
│   │       ├── bayesian.rs
│   │       ├── graph_risk.rs
│   │       └── fdr.rs
│   ├── learning/             # pattern mining, candidate ranking, backtests
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── miner.rs
│   │       ├── hypothesis.rs
│   │       ├── backtest.rs
│   │       └── negative_control.rs
│   ├── llm/                  # model-agnostic LLM client
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs        # trait + implementations
│   │       └── validators.rs # JSON schema validation
│   ├── recipes/              # recipe registry, staging, promotion
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── engine.rs
│   │       ├── gates.rs
│   │       └── lifecycle.rs
│   ├── poi/                  # POI resolution, profile features, synthesis
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── model.rs
│   │       ├── features.rs
│   │       ├── engagement.rs
│   │       ├── updater.rs
│   │       └── resolver.rs   # person entity resolution
│   ├── insights/             # narrative rendering from recipes + evidence
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── renderer.rs
│   │       ├── memo.rs       # weekly strategy memo generator
│   │       └── dossier.rs    # company/POI dossier generator
│   ├── api/                  # Axum APIs
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── routes/
│   │       │   ├── mod.rs
│   │       │   ├── warnings.rs
│   │       │   ├── insights.rs
│   │       │   ├── companies.rs
│   │       │   ├── persons.rs
│   │       │   ├── recipes.rs
│   │       │   ├── graph.rs
│   │       │   ├── dossiers.rs
│   │       │   ├── search.rs
│   │       │   └── admin.rs
│   │       ├── auth.rs
│   │       └── ws.rs         # WebSocket for real-time warnings
│   └── worker/               # schedulers
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── scheduler.rs
│           ├── nightly.rs    # nightly jobs (crawl, mine, poi refresh)
│           └── weekly.rs     # weekly jobs (memo, promotion board)
├── frontend/                 # Next.js (Sensei OS clone)
│   ├── package.json
│   ├── next.config.js
│   ├── tailwind.config.ts
│   └── src/
│       └── app/
│           ├── layout.tsx
│           ├── globals.css
│           ├── (auth)/
│           ├── (dashboard)/
│           │   ├── layout.tsx
│           │   ├── overview/
│           │   ├── warnings/
│           │   ├── insights/
│           │   ├── companies/
│           │   ├── persons/         # POI browser
│           │   ├── dossiers/
│           │   ├── competitors/
│           │   ├── security/
│           │   ├── graph/           # interactive graph explorer
│           │   ├── recipes/
│           │   ├── memos/           # strategy memos
│           │   └── settings/
│           └── api/
├── migrations/               # SQL migrations
└── docker-compose.yml
```

### 9.2 Workspace Cargo.toml

```toml
[workspace]
members = [
    "crates/core",
    "crates/crawl",
    "crates/parse",
    "crates/store",
    "crates/graph",
    "crates/stats",
    "crates/learning",
    "crates/llm",
    "crates/recipes",
    "crates/poi",
    "crates/insights",
    "crates/api",
    "crates/worker",
]
resolver = "2"

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# HTTP
reqwest = { version = "0.12", features = ["json", "gzip", "brotli", "socks", "cookies"] }
axum = { version = "0.7", features = ["ws", "multipart"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace", "compression-gzip"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# Database
sqlx = { version = "0.7", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }

# HTML parsing
scraper = "0.19"
quick-xml = "0.31"

# Hashing
sha2 = "0.10"
hex = "0.4"

# Rate limiting
governor = "0.6"

# DNS
hickory-dns = { version = "0.24", package = "hickory-resolver" }

# Search
tantivy = "0.22"

# Crypto / TLS
rustls = "0.23"

# Tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
opentelemetry = "0.22"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Random
rand = "0.8"

# Error handling
anyhow = "1"
thiserror = "1"

# Async traits
async-trait = "0.1"

# UUID
uuid = { version = "1", features = ["v4", "serde"] }

# Language detection
whatlang = "0.16"

# Message queue
async-nats = "0.34"

# Object storage
aws-sdk-s3 = "1"
aws-config = "1"
```

### 9.3 Docker Compose

```yaml
# docker-compose.yml
version: "3.9"
services:
  postgres:
    image: timescale/timescaledb:latest-pg16
    environment:
      POSTGRES_DB: apexintel
      POSTGRES_USER: apexintel
      POSTGRES_PASSWORD: ${DB_PASSWORD}
    ports: ["5432:5432"]
    volumes: ["pgdata:/var/lib/postgresql/data"]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U apexintel"]
      interval: 5s

  redis:
    image: redis:7-alpine
    ports: ["6379:6379"]
    command: redis-server --appendonly yes

  nats:
    image: nats:latest
    ports: ["4222:4222", "8222:8222"]
    command: -js -sd /data
    volumes: ["natsdata:/data"]

  minio:
    image: minio/minio:latest
    environment:
      MINIO_ROOT_USER: apexintel
      MINIO_ROOT_PASSWORD: ${MINIO_PASSWORD}
    ports: ["9000:9000", "9001:9001"]
    volumes: ["miniodata:/data"]
    command: server /data --console-address ":9001"

  api:
    build:
      context: .
      dockerfile: Dockerfile.api
    environment:
      DATABASE_URL: postgres://apexintel:${DB_PASSWORD}@postgres/apexintel
      REDIS_URL: redis://redis:6379
      NATS_URL: nats://nats:4222
      MINIO_URL: http://minio:9000
      GOOGLE_API_KEY: ${GOOGLE_API_KEY}
      GOOGLE_SEARCH_ENGINE_ID: ${GOOGLE_SEARCH_ENGINE_ID}
      NEXAR_CLIENT_ID: ${NEXAR_CLIENT_ID}
      NEXAR_CLIENT_SECRET: ${NEXAR_CLIENT_SECRET}
      LLM_API_KEY: ${LLM_API_KEY}
      LLM_BASE_URL: ${LLM_BASE_URL}
    ports: ["8080:8080"]
    depends_on: [postgres, redis, nats, minio]

  worker:
    build:
      context: .
      dockerfile: Dockerfile.worker
    environment:
      DATABASE_URL: postgres://apexintel:${DB_PASSWORD}@postgres/apexintel
      REDIS_URL: redis://redis:6379
      NATS_URL: nats://nats:4222
      MINIO_URL: http://minio:9000
      GOOGLE_API_KEY: ${GOOGLE_API_KEY}
      GOOGLE_SEARCH_ENGINE_ID: ${GOOGLE_SEARCH_ENGINE_ID}
      NEXAR_CLIENT_ID: ${NEXAR_CLIENT_ID}
      NEXAR_CLIENT_SECRET: ${NEXAR_CLIENT_SECRET}
      LLM_API_KEY: ${LLM_API_KEY}
      LLM_BASE_URL: ${LLM_BASE_URL}
      ENABLE_PROXY_ROTATION: "true"
    depends_on: [postgres, redis, nats, minio]

  frontend:
    build:
      context: ./frontend
      dockerfile: Dockerfile
    environment:
      NEXT_PUBLIC_API_URL: http://api:8080
    ports: ["3000:3000"]
    depends_on: [api]

volumes:
  pgdata:
  natsdata:
  miniodata:
```

---

## 10. Quality Gates & Tests

### 10.1 Data Collection Gates

| Gate | Threshold | Action on Failure |
|---|---|---|
| Fetch success | ≥98% on allowed targets | Alert + retry with backoff |
| Change detection efficiency | >15% pages actually changed per cycle | Reduce crawl frequency for stable pages |
| Freshness SLA | 24h for critical, 72h for standard | Priority re-queue |
| Near-duplicate control | <5% | Dedup before storage |

### 10.2 Extraction Gates

| Gate | Threshold | Action on Failure |
|---|---|---|
| Parse success | ≥95% per extractor | Dead-letter + alert |
| Schema validation | 100% (mandatory) | Reject to dead-letter |
| Provenance completeness | 100% (mandatory) | Reject |
| Language coverage | ≥90% for each target region | Add extractors |

### 10.3 Entity Resolution Gates

| Gate | Threshold | Action on Failure |
|---|---|---|
| Precision (false merge rate) | ≥99% | Block merge + human review |
| Recall (watchlist entities) | ≥90% | Add resolution rules |
| Explainability | 100% (every merge has a reason) | Reject |

### 10.4 Statistical / Model Gates

| Gate | Threshold | Action on Failure |
|---|---|---|
| Calibration | Reliability curve within ±5% | Recalibrate |
| False alarm budget | <2% daily FP per warning type | Tighten thresholds |
| Perturbation stability | Top recipes stable under ±10% noise | Flag as fragile |
| Drift detection | KL divergence < threshold | "Model stale" alert |

### 10.5 Insight Acceptance Gates (Consultant Bar)

| Gate | Threshold | Action on Failure |
|---|---|---|
| Actionability | ≥2 concrete actions | Suppress |
| Evidence | ≥2 independent sources OR 1 high-authority | Suppress |
| Counterfactual | Insight survives signal removal | Downgrade confidence |
| Impact estimate | Must have rationale | Reject |

### 10.6 Test Matrix

```rust
// Tests by category

#[cfg(test)]
mod unit_tests {
    // Parsers: each extractor has input→expected output tests
    // Transforms: idempotent normalization
    // Scoring: known inputs → known scores
    // Hashing: stable hashing across versions
}

#[cfg(test)]
mod property_tests {
    // proptest or quickcheck:
    // - Normalization is idempotent
    // - Hashing is deterministic
    // - Bayesian fusion is monotonic (more evidence → stronger posterior)
    // - Entity resolution is transitive
}

#[cfg(test)]
mod integration_tests {
    // fetch → parse → store pipeline with fixtures
    // Recipe evaluation end-to-end
    // POI profile computation end-to-end
}

#[cfg(test)]
mod golden_tests {
    // Curated web pages → expected observations
    // Known competitor pages → expected capability extraction
    // Known POI pages → expected profile features
}

#[cfg(test)]
mod backtest_tests {
    // Historical data replay
    // "Would we have warned within SLA?"
    // Recipe precision/recall on held-out periods
}
```

---

## 11. 240 OSINT Combinational Insights

All 240 insight rules are implemented as seed recipes in `config/recipes_seed.yaml`. Each recipe follows the standard schema (signals → transforms → statistical test → thresholds → narrative → actions → applicability). Below is the complete catalog.

### Category A — Demand & Procurement Intelligence (A001–A050)

| ID | Signal Combination | Insight | Actions |
|---|---|---|---|
| A001 | JobPost.role=Procurement↑ + WebChange.supplier_portal | New sourcing cycle starting | Register on portal, send capabilities deck |
| A002 | Tender.public.posted + JobPost.role=SQE↑ | Qualification round imminent | Prepare audit package, submit bid |
| A003 | CertUpdate.new(IATF) + Tender.automotive | Automotive qualification opening | Fast-track IATF prep, contact SQE |
| A004 | TradeShow.presence↑ + PressRelease.expansion | Capacity expansion = new suppliers needed | Book meeting at show, pitch capacity |
| A005 | WebChange.product_page.new_category + Patent.filed | New product line = new supply chain | DFM collaboration pitch |
| A006 | ImportData.volume↑(HS:8534) + FxRate.favorable | PCB import surge + cost window | Price competitively, highlight proximity |
| A007 | JobPost.role=NPI↑ + Patent.recent + WebChange.RnD_page | NPI ramp = prototype demand | Offer rapid proto, small batch capability |
| A008 | Tender.defense + CompanyProfile.clearable | Defense procurement opportunity | ITAR/export control readiness pitch |
| A009 | ConferenceAgenda.topic=nearshoring + PressRelease.reshoring | Reshoring trend adoption | Morocco/Tunisia/Israel/Vietnam proximity pitch |
| A010 | SupplierPage.vendor_removed + JobPost.role=Procurement↑ | Supplier panel restructuring | Position as replacement, audit-ready pitch |
| A011 | Commodity.copper↑25% + ImportData.cable_harness↓ | Cost pressure on harness = sourcing review | Offer cost-optimized harness solution |
| A012 | FxRate.EUR/TND↑ + Tender.posted(TN) | Tunisia cost advantage opening | Submit competitive TND-denominated bid |
| A013 | WebChange.careers_page.engineer↑ + Patent.surge | R&D expansion = future custom PCBA demand | Position as design-to-manufacturing partner |
| A014 | TradeShow.speaker(POI) + POI.pain_index > 0.6 | Decision-maker publicly frustrated = opportunity | Prepare targeted solution pitch |
| A015 | Tender.framework_agreement + CompanyProfile.incumbent_expiring | Framework renewal = competitive bidding | Aggressive bid with differentiation |
| A016 | ImportData.origin_shift(CN→other) + Commodity.tariff↑ | China+1 sourcing shift | Position as nearshore alternative |
| A017 | JobPost.volume > 3σ + WebChange.facility_page.new | Major expansion = greenfield supply needs | Proactive outreach with full capability deck |
| A018 | CertUpdate.expired(ISO9001) + WebChange.quality_page↓ | Competitor quality lapse = opening | Highlight own cert currency and audit readiness |
| A019 | PressRelease.JV_announced + JobPost.bilingual(FR+EN) | Joint venture with NA presence needed | Pitch bilingual team + regional logistics |
| A020 | Tender.lot_splitting + ImportData.multi_origin | Customer fragmenting supply chain | Offer consolidated multi-capability solution |
| A021 | POI.role_change(new_CPO) + WebChange.supplier_page.restructured | New CPO restructuring vendor panel | Fast outreach within 90-day window |
| A022 | ConferenceAgenda.topic=Industry4.0 + JobPost.role=DigitalMfg | Industry 4.0 adoption = smart factory demand | Pitch IoT-ready assembly, traceability systems |
| A023 | ImportData.component_shortage(HS:8542) + WebChange.allocation_notice | Component allocation = need flexible EMS | Offer buffer stock management, BOM optimization |
| A024 | Tender.posted(medical) + CertUpdate.new(ISO13485) | Medical device sourcing round | ISO 13485 capability pitch, cleanroom readiness |
| A025 | PressRelease.acquisition + JobPost.integration_roles | Post-M&A integration = supply chain consolidation | Pitch as consolidated partner for merged entity |
| A026 | FxRate.EUR/MAD.stable + ImportData.EU→MA↑ | Morocco FDI inflow = co-location demand | Offer proximity to new EU plants in Morocco |
| A027 | WebChange.sustainability_page.new + PressRelease.ESG | ESG mandate = green supply chain requirements | Pitch ISO 14001, green manufacturing, LCA readiness |
| A028 | TradeShow.exhibitor_list.new_entrant + WebChange.capability_page.new | New market entrant = early partnership opportunity | Proactive capability pitch to new player |
| A029 | Tender.energy_sector + ImportData.solar/wind↑ | Renewable energy growth = power electronics demand | Pitch high-reliability PCBA for energy sector |
| A030 | JobPost.role=BuyerCommodity↑ + Commodity.volatility > 2σ | Commodity volatility = active sourcing review | Offer fixed-price contracts with hedging |
| A031 | POI.co_appearance(OEM+Starz_competitor) + TradeShow.joint_booth | Competitor deepening relationship | Counter with differentiated value prop |
| A032 | WebChange.supplier_qualification_form.updated + Tender.pending | New qualification form = imminent tender | Complete qualification immediately |
| A033 | ImportData.HS8544↑(automotive) + CertUpdate.IATF.site_added | Automotive wire harness demand surge | Pitch harness capability with IATF coverage |
| A034 | PressRelease.factory_closure(competitor) + JobPost.competitor.↓ | Competitor plant closure = displaced demand | Contact affected customers immediately |
| A035 | ConferenceAgenda.panel(POI) + POI.topic=quality_challenges | POI publicly discussing quality pain | Offer quality improvement partnership |
| A036 | Tender.GCC.posted + WebChange.GCC_office_opened | Customer expanding to GCC region | Pitch as established Morocco-based supplier with GCC logistics |
| A037 | ImportData.origin_shift(competitor_country→) + FxRate.unfavorable(competitor) | Competitor losing cost advantage | Competitive pricing pitch leveraging FX |
| A038 | WebChange.EoL_notice + Patent.expiring | Product end-of-life = last-time-buy = next-gen RFQ | Position for next-gen product supply |
| A039 | JobPost.role=TestEngineer↑ + WebChange.test_capability_page | Customer expanding test capability = outsourcing test | Offer ICT/functional test outsourcing |
| A040 | PressRelease.award_won(quality) + POI.role=QualityDirector.stable | Quality-focused organization with stable leadership | Pitch long-term quality partnership |
| A041 | Tender.posted(aerospace) + CertUpdate.AS9100.renewal | Aerospace qualification window | AS9100 capability + NADCAP readiness pitch |
| A042 | ImportData.PCB_bare_board↑ + JobPost.role=ProcessEngineer↑ | Customer insourcing assembly = need component supply | Pivot pitch to component kitting or sub-assembly |
| A043 | FxRate.GBP/EUR↓ + Tender.UK.posted | UK cost pressure = nearshore sourcing attractive | Competitive GBP-denominated proposal |
| A044 | WebChange.RoHS_compliance_page.updated + PressRelease.EU_regulation | Regulatory compliance push | Pitch RoHS/REACH compliance expertise |
| A045 | POI.pain_topic=lead_time + POI.priority.speed=high | Decision-maker frustrated with lead times | Pitch rapid turnaround capability with SLAs |
| A046 | TradeShow.cancelled + WebChange.virtual_event | Physical event cancelled = virtual engagement | Prepare virtual demo package, schedule webinar |
| A047 | Tender.multi_lot + ImportData.diversification_pattern | Supply chain diversification mandate | Pitch as additional qualified source |
| A048 | JobPost.role=SupplyChainRisk↑ + PressRelease.disruption_mentioned | Customer institutionalizing supply chain risk | Pitch dual-source strategy + geographic diversification |
| A049 | CertUpdate.NADCAP.new + Tender.special_process | NADCAP qualification → specialized process demand | Selective soldering, conformal coating pitch |
| A050 | POI.decision_mode=PilotFirst + WebChange.NPI_page.updated | Decision-maker prefers pilot → NPI activity detected | Propose small pilot batch with fast turnaround |

### Category B — Competitor & Market Intelligence (B001–B050)

| ID | Signal Combination | Insight | Actions |
|---|---|---|---|
| B001 | Competitor.capability_page.changed + Competitor.cert.new | Competitor expanding capabilities | Assess overlap, differentiate offering |
| B002 | Competitor.careers↑(role=SMT) + Competitor.press.expansion | Competitor capacity expansion | Preemptive customer outreach, capacity assurance |
| B003 | Competitor.careers↓ + Competitor.press.restructuring | Competitor distress signal | Position as stable alternative to affected customers |
| B004 | Competitor.cert.expired(IATF) + OASIS.site_suspended | Competitor loses automotive qualification | Target their automotive customers |
| B005 | Competitor.cert.expired(AS9100) + OASIS.site_suspended | Competitor loses aerospace qualification | Target their aerospace customers |
| B006 | Competitor.website.down > 48h + DNS.changes | Competitor infrastructure issues | Investigate, alert sales team |
| B007 | Competitor.social.negative_sentiment↑ + News.negative | Competitor facing public criticism | Discreet outreach to competitor's customers |
| B008 | Competitor.pricing.undercut_detected + Competitor.careers↓ | Competitor buying market share while distressed | Warn customers about sustainability risk |
| B009 | New_entrant.detected + New_entrant.capabilities.overlap | New competitor entering our segments | Assess threat, protect key accounts |
| B010 | Competitor.patent.filed(our_domain) + Competitor.press.tech_investment | Competitor innovating in our space | Accelerate own tech development |
| B011 | Competitor.social.partnership_announced + OEM.press.alliance | Competitor forming OEM alliance | Strengthen own OEM relationships |
| B012 | Competitor.trade_show.upgraded_booth + Competitor.marketing↑ | Competitor increasing market visibility | Match or exceed at next show |
| B013 | Competitor.acquisition.announced + Competitor.JobPost.integration | Competitor M&A activity | Target integration disruption period |
| B014 | Competitor.FX.advantage(currency) + Competitor.pricing↓ | Competitor gaining FX advantage | Adjust pricing, highlight non-price value |
| B015 | Competitor.cert.added(ISO13485) + Competitor.website.medical | Competitor entering medical segment | Assess medical market opportunity, accelerate own 13485 |
| B016 | Competitor.leadership.change(CEO) + Competitor.strategy.shift | Competitor strategic reorientation | Monitor direction, exploit transition period |
| B017 | Competitor.site.closed + News.layoffs | Competitor facility closure | Recruit their talent, target displaced customers |
| B018 | Competitor.delivery_complaints(social) + Customer.frustration | Competitor delivery failures | Proactive outreach with OTD guarantees |
| B019 | Competitor.equipment.sold(marketplace) + Competitor.careers↓ | Competitor downsizing capacity | Capacity gap in market = opportunity |
| B020 | Competitor.social.hiring_freeze + Competitor.financial.distress | Competitor financial trouble | Alert all shared customers |
| B021 | Competitor.supplier_qualification.rejected(POI) + POI.public_complaint | Target company rejected competitor | Immediate qualified alternative pitch |
| B022 | Competitor.press.cybersecurity_incident + Customer.security_concerns | Competitor security breach | Pitch own security posture (DMARC/DKIM/SPF) |
| B023 | Market.entry_barrier.lowered(regulation) + New_entrant.registered | Regulatory change enabling new competition | Strengthen customer relationships preemptively |
| B024 | Competitor.patent.infringement_suit + Competitor.press.legal | Competitor in legal trouble | Monitor impact, prepare for market shift |
| B025 | Market.consolidation(M&A) + Competitor.count↓ | Market consolidation = fewer options for buyers | Position as independent alternative |
| B026 | Competitor.review.negative(Glassdoor) + Competitor.turnover↑ | Competitor talent drain | Recruit their engineers, pitch stability |
| B027 | Competitor.ESG.violation + Competitor.press.penalty | Competitor ESG/compliance failure | Pitch own ESG credentials |
| B028 | Competitor.logistics.disruption + Port.congestion | Competitor logistics disruption | Offer alternative logistics routing |
| B029 | Competitor.technology.obsolete + Industry.shift(new_tech) | Competitor technology lagging | Pitch technology leadership, modernization |
| B030 | Competitor.social.exec_departure + Competitor.strategy.uncertainty | Key competitor executive leaving | Exploit leadership vacuum period |
| B031 | Competitor.pricing↑ + Commodity.stable | Competitor raising prices without commodity justification | Competitive pricing opportunity |
| B032 | Competitor.delivery_time↑ + Customer.lead_time_sensitivity | Competitor lead times extending | Pitch shorter lead times with evidence |
| B033 | Competitor.quality_issue(recall) + Industry.safety_alert | Competitor quality/safety event | Safety-focused quality pitch to their customers |
| B034 | Competitor.social.partnership_ended + Customer.seeking_alternative | Competitor-customer relationship ended | Direct outreach to freed customer |
| B035 | Competitor.regional_exit + Market.demand.persistent | Competitor exits region but demand remains | Fill the regional gap |
| B036 | Competitor.IP.expired + Market.barrier↓ | Competitor IP expired → market opens | Enter newly accessible market segment |
| B037 | Market.growth.segment↑ + Competitor.absence | Growing segment with no strong competitor | First-mover opportunity |
| B038 | Competitor.overcapacity + Market.demand↓ | Market downturn + competitor overcapacity | Price war risk warning |
| B039 | Competitor.union_issues + Competitor.production_disruption | Competitor labor issues | Offer capacity backup to their customers |
| B040 | Competitor.key_customer_lost + News.contract_change | Competitor lost major customer | Target the freed customer |
| B041 | Market.regulation.upcoming + Competitor.compliance_gap | Upcoming regulation where competitor lags | First-to-comply advantage pitch |
| B042 | Competitor.investment.greenfield + Region.incentives | Competitor investing in new region with our presence | Leverage existing infrastructure advantage |
| B043 | Competitor.social.product_launch + Market.reception.lukewarm | Competitor product launch underperforming | Exploit customer disappointment |
| B044 | Competitor.cert.scope_reduction + Customer.requirements↑ | Competitor cert scope shrinking while requirements grow | Fill growing compliance gap |
| B045 | Market.technology_shift + Competitor.investment↓ | Market shifting technology, competitor not investing | Technology leadership opportunity |
| B046 | Competitor.social.CEO_quote.negative_market + Market.uncertainty | Competitor leadership bearish on market | Contrarian confident pitch |
| B047 | Competitor.supply_chain_visibility_poor + Customer.transparency_demand | Customer demanding transparency competitor can't provide | Pitch full traceability system |
| B048 | Region.political_risk↑(competitor_location) + Customer.risk_avoidance | Political risk in competitor's region | Pitch geographic risk diversification |
| B049 | Competitor.quality_cert.upgraded + Market.quality_bar↑ | Competitor quality improvement narrows our advantage | Innovate beyond current quality bar |
| B050 | Competitor.social.sustainability_claim + Competitor.ESG.actual_gap | Competitor greenwashing detected | Genuine ESG credentials pitch |

### Category C — Supply Chain & Risk Intelligence (C001–C050)

| ID | Signal Combination | Insight | Actions |
|---|---|---|---|
| C001 | Commodity.Cu↑20% + Commodity.Sn↑15% | Critical materials cost spike | Notify customers, hedge forward, adjust quotes |
| C002 | Port.TangerMed.congestion↑ + Weather.storm.Mediterranean | Logistics disruption imminent | Pre-ship inventory, activate alt routes |
| C003 | FxRate.EUR/MAD.volatility > 2σ + BCT.policy_change | Currency risk escalation | Hedge FX exposure, offer EUR fixed pricing |
| C004 | Component.allocation(IC_family) + LeadTime.extension > 30% | Component shortage developing | Alert customers, propose BOM alternatives |
| C005 | Energy.price↑ + Government.subsidy_ended | Production cost increase | Adjust pricing, optimize energy usage |
| C006 | Import.duty.change + TradeAgreement.updated | Tariff/trade rule change | Recalculate landed costs, alert customers |
| C007 | Shipping.rate↑(Asia→EU) + Port.congestion(Rotterdam) | Inbound logistics cost spike | Source locally where possible, pre-order |
| C008 | Supplier.distress_signal + News.financial_trouble | Key material supplier at risk | Dual-source immediately, stock safety buffer |
| C009 | Weather.extreme(supplier_region) + Production.disruption_risk | Weather disruption to supply | Activate contingency suppliers |
| C010 | Geopolitical.tension↑(region) + Trade.restriction_risk | Geopolitical supply risk | Diversify supply origins, stockpile |
| C011 | Sanction.new(entity) + SupplyChain.exposure | Sanctions compliance risk | Screen supply chain, remove exposed entities |
| C012 | Component.EOL_notice + Product.lifecycle.active | End-of-life component in active products | Last-time-buy, design BOM alternative |
| C013 | Carrier.strike + LogisticsLane.affected | Transport strike disrupting deliveries | Activate alternative carriers/routes |
| C014 | Quality.incoming_reject↑ + Supplier.audit.overdue | Supplier quality deteriorating | Accelerate audit, enforce SCAR |
| C015 | Inventory.WIP↑ + Demand.forecast↓ | Work-in-progress building up vs declining demand | Slow production, communicate with customer |
| C016 | Lead_time.customer_request↓ + Capacity.utilization > 85% | Customer demanding faster while near capacity | Negotiate realistic timelines or add shift |
| C017 | Commodity.palladium↑ + PCB.surface_finish.ENIG.cost↑ | Precious metal driving PCB cost up | Propose alternative surface finishes |
| C018 | Shipping.container_shortage + Season.peak_shipping | Container availability crisis | Book containers early, consolidate shipments |
| C019 | Customs.delay↑(port=Rades) + Government.regulation_change | Customs clearance issues | Engage customs broker, prepare documentation |
| C020 | Supplier.cybersecurity_incident + DataBreach.exposure | Supply chain cyber risk | Assess own exposure, strengthen vendor security |
| C021 | Component.counterfeit_alert(ERAI) + Supplier.broker.flagged | Counterfeit component risk | Quarantine suspect lots, test authentication |
| C022 | Demand.surge_order + Material.lead_time > buffer | Rush order vs material availability gap | Negotiate delivery, source spot market |
| C023 | Energy.grid.instability(TN) + Season.summer_peak | Power reliability risk for production | Activate UPS/generator, schedule off-peak |
| C024 | Trade.embargo_risk + Customer.export_controlled | Export control compliance risk | Verify EAR/ITAR classification, consult legal |
| C025 | Logistics.cost_per_unit↑ + Customer.price_sensitivity | Logistics cost eating margin | Optimize packaging, consolidate shipments |
| C026 | Supplier.capacity_reduction + Demand.forecast↑ | Supplier capacity vs increasing demand gap | Qualify alternative suppliers urgently |
| C027 | Component.price_increase_notice + BOM.cost_impact > 5% | Material cost increase impacting BOM | Customer negotiation, BOM optimization |
| C028 | Port.new_route(TangerMed→) + Shipping.cost↓ | New logistics route opportunity | Evaluate cost saving, propose to customers |
| C029 | Quality.field_return↑ + Component.lot.common | Field returns linked to component lot | Root cause analysis, supplier SCAR |
| C030 | Warehouse.utilization > 90% + Demand.seasonal_peak | Storage capacity pressure | Arrange overflow storage, JIT optimization |
| C031 | Insurance.premium↑ + Region.risk_score↑ | Insurance cost increase signal | Review coverage, mitigate risk factors |
| C032 | PressRelease.factory_fire(supplier) + SupplyChain.single_source | Force majeure at single-source supplier | Activate emergency procurement |
| C033 | Commodity.tin_price.breakout + Solder.cost.projected↑ | Solder cost projected to spike | Forward buy solder, optimize paste usage |
| C034 | RegulatoryCertificationBody.audit_scheduled + Quality.finding_open | Upcoming audit with open findings | Close findings urgently before audit |
| C035 | Government.minimum_wage↑ + Production.labor_intensive | Labor cost increase mandate | Automate where possible, adjust pricing |
| C036 | Component.allocation.resolved + LeadTime.normalized | Shortage resolved, market normalizing | Resume standard procurement, reduce safety stock |
| C037 | Logistics.new_FTZ_benefit + Trade.preferential_origin | New free trade zone benefit available | Restructure logistics to capture benefit |
| C038 | Supplier.financial.downgrade(D&B) + Supplier.payment_delay | Supplier creditworthiness declining | Reduce exposure, qualify backup |
| C039 | Commodity.multi_metal.correlated_spike + BOM.multi_exposure | Correlated commodity super-cycle | Portfolio-level hedging strategy |
| C040 | Weather.El_Nino_forming + Commodity.agri_affected + Energy.hydro↓ | Climate pattern threatening supply chains | Multi-factor contingency planning |
| C041 | Port.infrastructure_upgrade + Logistics.efficiency_gain | Port improvement = logistics opportunity | Plan to capture efficiency gains |
| C042 | Trade.agreement.new(AfCFTA_implementation) + Market.access_improved | Trade agreement expanding market access | Evaluate new sourcing/selling routes |
| C043 | Component.obsolescence_wave(MCU_family) + BOM.affected_products↑ | Wide obsolescence event | Coordinate cross-customer redesign |
| C044 | Fuel.price↑ + Logistics.ground_transport.cost↑ | Ground transport cost increase | Optimize delivery routes, consolidate |
| C045 | Currency.TND.devaluation_risk + BCT.reserves↓ | Tunisia currency risk for local operations | FX hedging, EUR-denominated contracts |
| C046 | Supplier.natural_disaster_zone + Season.monsoon/hurricane | Seasonal supplier disruption risk | Pre-build safety stock before season |
| C047 | Shipping.Suez_Canal.disruption + LeadTime.Asia_sourced↑ | Suez disruption extending Asia lead times | Source alternative routes or local suppliers |
| C048 | Component.AI_chip.allocation + Demand.AI_products↑ | AI chip shortage affecting electronics broadly | Assess BOM exposure, secure allocations |
| C049 | Raw_material.conflict_mineral.regulation↑ + SupplyChain.audit_required | Conflict minerals compliance tightening | Supply chain mapping and certification |
| C050 | Pandemic.variant.emerging + Government.lockdown_risk | Pandemic resurgence risk | Activate business continuity plan |

### Category D — Security & Compliance Intelligence (D001–D050)

| ID | Signal Combination | Insight | Actions |
|---|---|---|---|
| D001 | DNS.DMARC_missing(target) + DNS.SPF_permissive | Target email security weak | Alert account team, offer guidance |
| D002 | CT.new_cert(lookalike_domain) + DNS.MX.suspicious | Phishing infrastructure targeting us/customer | Block domain, alert SOC, warn customer |
| D003 | CISA_KEV.new + TechStack.match(target) | Customer exposed to exploited vulnerability | Alert security team, offer mitigation help |
| D004 | DNS.DMARC_policy.downgraded + WebChange.email_config | Email security regression at target | Heightened phishing risk, verify communications |
| D005 | BreachDB.new(target_domain) + Credential.leak_count > 100 | Target company in major data breach | Assess supply chain data exposure |
| D006 | Shodan.exposed_service(target) + CVE.critical.matching | Customer infrastructure exposed | Responsible disclosure, security partner pitch |
| D007 | DNS.nameserver.changed + WHOIS.registrar.changed | Domain infrastructure change (potential compromise) | Verify legitimacy, heightened monitoring |
| D008 | CT.cert_expired(target) + WebChange.SSL_error | Customer TLS certificate management failure | Alert, offer security consulting |
| D009 | Competitor.breach + Industry.security_awareness↑ | Industry-wide security concern after competitor breach | Highlight own security posture |
| D010 | Sanction.screening.new_match + Customer.entity | Customer or vendor appearing on sanctions list | Immediate compliance review, legal consult |
| D011 | ExportControl.classification_change + Product.affected | Export classification changed for our product | Update compliance, alert affected customers |
| D012 | GDPR_enforcement.new(industry) + DataProcessing.questionable | Data protection enforcement in our industry | Audit data handling, update agreements |
| D013 | CyberAttack.industry_sector + ThreatIntel.TTPs_matching | Threat campaign targeting our sector | Implement recommended mitigations |
| D014 | DNS.typosquat.new_registration + Brand.impersonation_risk | Typosquatting domain registered against us | Takedown request, block at email gateway |
| D015 | VendorPSIRT.advisory(critical) + TechStack.match(own) | Critical vendor security advisory affecting us | Patch immediately, assess exposure |
| D016 | PasteDB.credentials(own_domain) + BreachDB.match | Our credentials leaked on paste site | Force password resets, investigate source |
| D017 | RegulatoryChange.NIS2 + Compliance.gap_analysis | NIS2 directive compliance requirement | Prepare compliance roadmap |
| D018 | Insurance.cyber_premium↑ + Incident.frequency↑(sector) | Cyber insurance market hardening | Improve security posture for better rates |
| D019 | ThirdPartyRisk.vendor_score↓ + Vendor.security_questionnaire.overdue | Vendor security posture declining | Escalate vendor assessment, request evidence |
| D020 | PhishTank.targeted(our_brand) + Social.impersonation_detected | Active phishing using our brand | Incident response, takedown requests |
| D021 | ITAR.violation_enforcement(industry) + Product.defense_related | ITAR enforcement action in our industry | Audit all defense-related processes |
| D022 | CertificateTransparency.wildcard_issued(suspicious) + DNS.subdomain_new | Suspicious wildcard cert for related domain | Investigate for MitM risk |
| D023 | DataLeakage.public_repo(code) + Codebase.match | Code or credentials in public repository | Rotate credentials, DMCA takedown |
| D024 | REACH.substance_added + Product.material.affected | REACH substance of concern in our products | Reformulate, notify customers |
| D025 | Competitor.data_breach + Customer.vendor_security_audit↑ | Industry security bar rising after breach | Proactive security posture update to customers |
| D026 | DNS.SPF.includes > 10 + Email.deliverability↓ | SPF record too permissive/complex | Fix email authentication |
| D027 | RoHS.exemption.expiring + Product.lead_containing | RoHS exemption expiring for our product category | Transition to lead-free, timeline planning |
| D028 | PhysicalSecurity.site_incident(region) + Insurance.requirement | Physical security event near our facility | Review site security, update protocols |
| D029 | AI_regulation.new(EU_AI_Act) + Product.AI_component | AI regulation affecting product compliance | Legal review, compliance preparation |
| D030 | SupplyChain.tampering_alert + Component.suspect_source | Supply chain integrity concern | Authenticate components, inspect lots |
| D031 | WEEE.regulation_update + Product.disposal_affected | E-waste regulation change | Update take-back programs, compliance docs |
| D032 | CountryRisk.sanctions_approaching + Customer.entity_exposed | Customer at risk of sanctions designation | Assess continued engagement, legal review |
| D033 | CyberInsurance.claim.sector↑ + OwnSecurity.gap_identified | Sector cyber claims rising with own gap | Urgently close identified security gap |
| D034 | ExportControl.denied_party.new + Vendor.match_possible | Vendor potentially matching denied party | Enhanced due diligence, screening |
| D035 | DMARC.aggregate_report.spoofing↑ + Email.phishing_volume↑ | Our domain being actively spoofed | Tighten DMARC policy to reject |
| D036 | PersonalData.breach_notification_requirement + Incident.detected | Personal data breach requiring notification | Legal/DPO notification, customer disclosure |
| D037 | TechStack.EOL_component + CVE.unpatched | End-of-life tech with unpatched vulnerabilities | Replace/upgrade, compensating controls |
| D038 | CompetitorIP.claim_against_us + Patent.litigation_risk | IP infringement claim risk | Legal review, design-around assessment |
| D039 | TradeSecret.employee_departure + Competitor.hiring(our_ex) | Trade secret leakage risk via ex-employee | Enforce NDA, monitor for IP use |
| D040 | CERTAdvisory.industry_specific + Infrastructure.matching | Industry-targeted advisory matching our setup | Apply recommendations immediately |
| D041 | Sanction.secondary_risk + Supplier.country_exposed | Secondary sanctions risk through supplier | Evaluate supply chain sanctions exposure |
| D042 | PrivacyRegulation.new(country) + Operations.country_present | New privacy law where we operate | Legal compliance assessment |
| D043 | EnvironmentalRegulation.fine(competitor) + OwnCompliance.review | Competitor fined for environmental violation | Self-audit, ensure compliance |
| D044 | WorkplaceRegulation.change + Labor.compliance_gap | Workplace regulation change with compliance gap | Update policies, train management |
| D045 | DNS.DNSSEC.disabled + DNS.hijacking_risk | DNSSEC disabled on critical domains | Enable DNSSEC, assess hijacking risk |
| D046 | CustomsCompliance.audit_announced + Documentation.gap | Customs audit with documentation gaps | Prepare documentation urgently |
| D047 | QualityRegulation.recall_mandate + Product.affected_lot | Mandatory recall impacting our product | Execute recall, root cause, CAPA |
| D048 | CyberThreat.ransomware_gang.targeting_sector + Backup.untested | Ransomware threat targeting our sector | Test backups, drill response plan |
| D049 | Certificate.code_signing.compromise(vendor) + Software.supply_chain | Software supply chain compromise risk | Verify software integrity, scan systems |
| D050 | Compliance.audit.surprise + MultiRegulation.overlap | Surprise multi-regulation audit | All-hands compliance preparation |

### Category E — Strategic & POI-Enriched Intelligence (E001–E090)

| ID | Signal Combination | Insight | Actions |
|---|---|---|---|
| E001 | POI.CPO.role_change + Company.supplier_review_cycle | New CPO starting supplier review | 90-day engagement window, tailored pitch |
| E002 | POI.SQE.pain_index > 0.7 + Company.quality_incident | SQE publicly frustrated about quality issues | Offer quality improvement partnership |
| E003 | POI.Engineering.patent_filed + Company.NPI_active | Engineering leader in active NPI phase | DFM collaboration pitch |
| E004 | POI.CEO.public_speech(reshoring) + Company.sourcing_strategy_shift | CEO publicly endorsing reshoring | Nearshore manufacturing pitch |
| E005 | POI.Procurement.conference_speaker + POI.topic=cost_optimization | Procurement leader focused on cost | TCO analysis pitch with evidence |
| E006 | POI.role_drift > 0.8 + POI.influence > 70 | High-influence POI recently changed role | Priority outreach during transition window |
| E007 | POI.co_appearance(Starz_contact) + POI.decision_mode=RelationshipDriven | Shared connection with relationship-driven POI | Leverage introduction from mutual contact |
| E008 | POI.pain_topic=delivery + POI.priority.speed=high | POI with delivery pain and speed priority | Lead-time guarantee pitch with SLA data |
| E009 | POI.negotiation_style=analytical + POI.proof_pref=KPI | Analytical negotiator wanting data proof | Prepare heavy KPI/metrics package |
| E010 | POI.decision_mode=AuditFirst + Company.audit_schedule_Q3 | Audit-first decision-maker with upcoming audit | Proactive audit readiness package |
| E011 | POI.change_appetite=EarlyAdopter + Technology.new_process_available | Innovation-friendly POI + new tech available | First-mover technology pitch |
| E012 | POI.influence=gatekeeper + POI.blocker_risk=high | POI is potential gatekeeper/blocker | Identify champion to bypass, address concerns |
| E013 | POI.network.bridge(target_company) + POI.willingness_indicator | POI bridges us to target company | Cultivate bridge POI relationship |
| E014 | POI.government.appointment_new + Government.incentive_program | New government appointment + active incentive | Engage government POI for program access |
| E015 | POI.free_zone.director + FreeZone.expansion_planned | Free zone director during expansion phase | Negotiate favorable terms, early access |
| E016 | POI.banking.industrial_lending + Company.investment_planned | Bank lending officer + customer investment | Facilitate financing introduction |
| E017 | POI.distributor.allocation_control + Component.shortage | Distributor POI controlling allocation | Cultivate for preferential allocation |
| E018 | POI.certification_body.auditor + Company.audit_upcoming | Cert body auditor + customer audit | Prepare audit coaching package |
| E019 | POI.consultant.project_active + Customer.transformation | Active consultant engagement at target | Partner with consultant on transformation |
| E020 | POI.association.president + Industry.standard_draft | Association president during standard drafting | Participate in standard development |
| E021 | POI.academic.research_grant + Technology.relevant | Academic with funded research in our domain | Research partnership opportunity |
| E022 | POI.social.sentiment_shift(negative→positive) + Company.improvement | POI sentiment improving about company category | Timing alignment for outreach |
| E023 | POI.competitor_relationship.weakening + POI.alternative_seeking | POI's relationship with competitor weakening | Positioned alternative pitch |
| E024 | POI.budget_authority + Budget.cycle(Q4_planning) | Budget holder during planning cycle | Ensure inclusion in next year's budget |
| E025 | POI.multiple_roles(decision_committee) + Company.consensus_buying | POI on buying committee of consensus organization | Multi-stakeholder engagement strategy |
| E026 | POI.public_presentation(YouTube) + POI.visibility_seeking | POI actively building public profile | Offer co-branding / joint case study |
| E027 | POI.retirement_approaching + POI.successor_identified | POI succession planning underway | Engage successor, maintain continuity |
| E028 | POI.cultural_alignment(Morocco_connection) + Company.FDI_interest | POI with North Africa cultural ties + FDI interest | Cultural bridge engagement strategy |
| E029 | POI.LinkedIn.endorsements_for_us + POI.social_proof_willing | POI willing to publicly endorse | Request testimonial/case study permission |
| E030 | POI.pain_index↑ + POI.public_complaints↑ | POI frustration escalating publicly | Urgent problem-solving outreach |
| E031 | WebChange.sustainability_report + POI.ESG_champion | ESG-focused POI + new sustainability report | Green manufacturing proposition |
| E032 | POI.CTO.blog_post(new_tech) + Company.R&D_investment↑ | CTO blogging about new technology + R&D spend | Early technology partnership pitch |
| E033 | POI.departing_champion + Replacement.unknown | Our champion leaving with no replacement | Urgent relationship preservation |
| E034 | POI.multiple_co_appearances(target) + Event.upcoming(shared) | Multiple connections visible at upcoming event | Maximize event networking |
| E035 | POI.language=FR + Company.TN_MA_operation | French-speaking POI at company with TN/MA ops | French-language engagement, cultural alignment |
| E070 | POI.language=ZH + Company.CN_TW_operation | Chinese-speaking POI at company with CN/TW ops | Mandarin-language engagement, cultural alignment |
| E071 | ImportData.origin_shift(CN→other) + POI.procurement.active + Trade.China_Plus_One | China+1 sourcing shift detected + active procurement POI | Position as nearshore alternative, reference tariff/risk reduction |
| E036 | POI.previous_employer=Starz_customer + POI.current_employer=target | POI previously worked with Starz at another company | Leverage prior relationship |
| E037 | POI.standards_committee_chair + Standard.revision_active | Standards committee chair during revision | Influence standard direction |
| E038 | POI.trade_show_organizer + Event.CFP_open | Event organizer during call for papers | Submit speaking proposal, sponsor |
| E039 | POI.investment_agency_director + Region.investment_incentive_new | Agency director with new incentive program | Apply for incentive, deepen relationship |
| E040 | POI.port_authority_exec + Port.capacity_expansion | Port authority exec during expansion | Negotiate logistics terms |
| E041 | POI.ITAR_officer + Company.defense_program_new | ITAR officer at company starting defense program | ITAR compliance consultation offer |
| E042 | POI.social.crisis_response + Company.brand_risk | POI managing crisis at target company | Demonstrate reliability during crisis |
| E043 | POI.role_history(multi_company) + Industry.connector | POI with broad industry network | Network leverage engagement |
| E044 | POI.publication(industry_report) + Trend.alignment | POI authored relevant industry report | Reference their work, build relationship |
| E045 | POI.competitor.co_appearance↓ + Relationship.cooling | POI reducing competitor engagement | Opportunity to fill relationship gap |
| E046 | POI.stakeholder_map.isolated + Decision.influence.hidden | POI isolated in stakeholder map but influential | Identify hidden influence channel |
| E047 | POI.conference_attendance.pattern + Event.calendar | POI's conference attendance pattern mapped | Plan encounters at likely events |
| E048 | POI.article.coauthored(Starz_topic) + POI.thought_leadership | POI writing about topics we excel at | Thought leadership collaboration |
| E049 | POI.government.policy_influence + Regulation.draft.favorable | POI influencing favorable policy | Support/facilitate policy direction |
| E050 | POI.FIPA.director + Investment.project_pipeline | FIPA director with active project pipeline | Early project intelligence access |
| E051 | POI.AMDIE.officer + Morocco.ecosystem_strategy | AMDIE officer driving ecosystem strategy | Align with Morocco industrial strategy |
| E052 | POI.certification_registrar + Audit.cycle.industry | Certification registrar contact during audit season | Audit intelligence and preparation support |
| E053 | POI.competitor.executive_hired + Competitor.strategy_shift | Our competitor hired target company exec | Assess intel leakage risk, counter-strategy |
| E054 | Market.PMI↑ + POI.procurement.budget_released | Rising PMI + procurement budget cycle | Multi-customer expansion push |
| E055 | POI.pair(CPO + SQE).aligned + Company.shortlisting | Both buying-side POIs aligned favorably | Coordinated multi-stakeholder pitch |
| E056 | Government.FDI_incentive + Company.expansion_signal + POI.lobby_contact | Incentive + expansion + government contact | Three-way facilitation play |
| E057 | Market.nearshoring_trend↑ + Trade.agreement_update + Competitor.price↑ | Multi-factor nearshoring opportunity window | Aggressive regional marketing campaign |
| E058 | POI.social.podcast_appearance + Topic.relevant | POI appearing on industry podcast | Listen, reference in engagement |
| E059 | POI.patent_coauthor(industry_leader) + Technology.relevant | POI co-inventing with industry leader | Technology credibility leverage |
| E060 | POI.event_organizer.networking_event + Industry.social_gathering | POI organizing industry networking | Attend, maximize face-time |
| E061 | Component.EOL + Customer.BOM.affected + POI.engineering_lead | Component EOL affecting customer's BOM, engineering lead identified | Offer redesign support through engineering POI |
| E062 | Competitor.cert_loss + Customer.search.alternative + POI.SQE.active | Competitor lost cert, customer searching, SQE identified | Targeted pitch to specific SQE |
| E063 | Market.segment.emerging + POI.early_adopter + Company.budget_cycle | Emerging market segment + early-adopter POI + budget timing | Timed innovation pitch |
| E064 | POI.pain_index=0.9 + POI.decision_authority=high + Trigger.recent | Highly frustrated high-authority POI with recent trigger | Maximum priority engagement |
| E065 | POI.cluster(3+stakeholders).identified + Company.committee_buying | Decision committee cluster identified | Coordinated multi-POI engagement plan |
| E066 | TradeAgreement.EU_Morocco_update + Customer.EU_sourcing + POI.procurement | EU-Morocco trade update + EU customer sourcing + procurement POI | Rules of origin advantage pitch |
| E067 | POI.UTICA.leader + IndustryPolicy.TN + Starz.advocacy_need | UTICA federation leader + policy development | Industry advocacy engagement |
| E068 | POI.CRI_director + Region.MA.investment + Company.MA_expansion | CRI director in region where target expanding | Facilitate customer expansion in Morocco |
| E069 | POI.bank_lender + Customer.credit_need + Relationship.indirect | Banking POI, customer needs credit, indirect connection | Facilitate financing introduction |
| E070 | POI.competitor.board_member + Competitor.strategy.visible | Competitor board member's public statements | Competitor strategy intelligence source |
| E071 | POI.academic.visiting_scholar + Technology.research_frontier | Academic visiting industry, research frontier relevant | Research-to-manufacturing bridge |
| E072 | POI.trade_association.working_group + Standard.development | Working group member during standard development | Participate and influence standard |
| E073 | POI.media.journalist.industry + Company.story_worthy | Industry journalist + newsworthy event | PR opportunity facilitation |
| E074 | POI.government.customs_officer + Trade.facilitation_need | Customs officer contact + trade facilitation opportunity | Streamline import/export processes |
| E075 | POI.logistics.freight_forwarder + Shipping.rate_negotiation | Freight forwarder POI during rate negotiation | Logistics cost optimization |
| E076 | POI.industry.keynote_speaker + Trend.directional | Industry thought leader in keynote | Trend intelligence extraction |
| E077 | Social.sentiment(company).shift_negative + POI.comms_officer | Negative sentiment shift + comms officer identified | Crisis communication support offer |
| E078 | POI.decision_timeline.compressed + Budget.year_end | Compressed decision timeline at year-end | Accelerated proposal with urgency framing |
| E079 | POI.technology_evangelist + Innovation.demonstration_ready | Tech evangelist POI + innovation demo available | Technology demonstration invitation |
| E080 | POI.risk_officer + Risk.event.sector | Risk officer at target during sector risk event | Risk mitigation partnership proposal |
| E081 | FreeZone.new_tenant(competitor) + FreeZone.POI.contact | Competitor entering our free zone | Intelligence from free zone authority POI |
| E082 | POI.procurement.digital_transformation + Platform.adoption | Procurement digitalization + platform adoption | Integrate with customer procurement system |
| E083 | POI.sustainability_officer + Regulation.carbon_reporting | Sustainability officer + carbon reporting mandate | Carbon footprint data package |
| E084 | POI.quality_manager + Recall.industry_event | Quality manager during industry recall event | Quality assurance reinforcement pitch |
| E085 | Academic.publication(relevant) + POI.academic.cited + Industry.application | Academic research with industry application, cited POI | Research-to-production pitch through academic |
| E086 | POI.departing(competitor) + Talent.available + Skills.relevant | Competitor talent becoming available | Recruit key talent, gain intelligence |
| E087 | POI.investor_relations + Company.earnings_call + Guidance.supply_chain | IR POI + earnings call mentioning supply chain | Public guidance intelligence extraction |
| E088 | POI.legal_counsel + IP.dispute_risk + Competitor.patent_aggressive | Legal counsel identified + IP risk | Proactive IP protection engagement |
| E089 | POI.operations.plant_manager + Capacity.constraint + Season.peak | Plant manager facing capacity constraint at peak | Overflow capacity offer |
| E090 | Multi_POI.aligned(5+signals) + Opportunity.score > 0.9 | Multiple high-confidence signals converging | Maximum priority coordinated pursuit |

### 11.1 Insight Category Summary

| Category | Range | Count | Focus Area |
|---|---|---|---|
| A — Demand & Procurement | A001–A050 | 50 | Business opportunity detection |
| B — Competitor & Market | B001–B050 | 50 | Competitive intelligence |
| C — Supply Chain & Risk | C001–C050 | 50 | Risk mitigation and supply chain |
| D — Security & Compliance | D001–D050 | 50 | Cybersecurity and regulatory |
| E — Strategic & POI-Enriched | E001–E090 | 90 | POI-driven strategic intelligence |
| **TOTAL** | | **290** | |

> Note: The expanded total is 290 (exceeding the original 240 target) because POI-enriched variants in Category E warranted deeper coverage. All 290 recipes are loaded from `config/recipes_seed.yaml` at startup and continuously improved by the LLM learning loop.

### 11.2 Recipe Priority & Execution

```yaml
recipe_execution:
  priority_tiers:
    P0_critical:  # Real-time evaluation (minutes)
      - D001-D050  # Security insights (time-sensitive)
      - C032       # Force majeure
      - C050       # Pandemic
      - B034       # Competitor-customer split
    
    P1_high:  # Every 4 hours
      - A001-A050  # Demand insights
      - B001-B020  # Core competitor insights
      - C001-C030  # Core supply chain
      - E064       # Maximum-priority POI engagement
    
    P2_standard:  # Every 12 hours
      - B021-B050  # Extended competitor insights
      - C031-C049  # Extended supply chain
      - E001-E063  # POI strategic insights
    
    P3_strategic:  # Weekly
      - E065-E090  # Complex multi-POI insights
      - A046-A050  # Long-term strategic demand
      
  deduplication:
    # Same entity + same insight category within 72h → suppress
    window_hours: 72
    merge_strategy: keep_highest_confidence
    
  escalation:
    confidence > 0.9 AND impact = High:
      - SMS to GM
      - Email to sales leadership
      - Slack webhook to #intel-critical
    confidence > 0.7 AND impact = High:
      - Email to relevant account manager
      - Dashboard notification
    default:
      - Dashboard only
      - Weekly memo inclusion
```

---

## 12. Full Rust Implementation

### 12.1 Worker Scheduler (Entry Point)

```rust
// crates/worker/src/main.rs

use anyhow::Result;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let config = core::config::load()?;
    let pool = sqlx::PgPool::connect(&config.database_url).await?;
    let nats = async_nats::connect(&config.nats_url).await?;
    
    // Initialize services
    let crawl_svc = crawl::CrawlService::new(&config, pool.clone());
    let parse_svc = parse::ParseService::new();
    let store_svc = store::StoreService::new(pool.clone());
    let graph_svc = graph::GraphService::new(pool.clone());
    let stats_svc = stats::StatsService::new();
    let recipe_engine = recipes::RecipeEngine::load_from_db(&pool).await?;
    let poi_updater = poi::PoiUpdater::new(pool.clone(), config.llm_client()?);
    let miner = learning::PatternMiner::new(pool.clone());
    let lifecycle = recipes::RecipeLifecycle::new(pool.clone());
    
    tracing::info!("ApexIntel worker started");
    
    // Spawn scheduled tasks
    let pool_c = pool.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(3600)); // hourly
        loop {
            ticker.tick().await;
            if let Err(e) = crawl_svc.run_crawl_cycle().await {
                tracing::error!("Crawl cycle failed: {}", e);
            }
        }
    });
    
    // Nightly: pattern mining + POI refresh
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(86400)); // daily
        loop {
            ticker.tick().await;

            // POI refresh
            match poi_updater.nightly_refresh().await {
                Ok(reports) => tracing::info!("POI refresh: {} profiles updated", reports.len()),
                Err(e) => tracing::error!("POI refresh failed: {}", e),
            }
            
            // Pattern mining
            let config = learning::MinerConfig::default();
            match miner.mine_all_candidates(&config).await {
                Ok(candidates) => {
                    tracing::info!("Mined {} pattern candidates", candidates.len());
                    // LLM hypothesis generation for top candidates
                    for cand in candidates.iter().take(20) {
                        // ... propose recipe, stat gate, stage
                    }
                }
                Err(e) => tracing::error!("Pattern mining failed: {}", e),
            }
        }
    });
    
    // Weekly: promotion board + strategy memo
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(604800)); // weekly
        loop {
            ticker.tick().await;
            match lifecycle.run_promotion_board().await {
                Ok(promoted) => tracing::info!("Promoted {} recipes", promoted.len()),
                Err(e) => tracing::error!("Promotion board failed: {}", e),
            }
            match lifecycle.deprecate_stale().await {
                Ok(deprecated) => tracing::info!("Deprecated {} recipes", deprecated.len()),
                Err(e) => tracing::error!("Deprecation failed: {}", e),
            }
            // Generate weekly memo
            // insights::memo::generate_weekly(&pool).await
        }
    });
    
    // Keep main alive
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

### 12.2 API Server (Entry Point)

```rust
// crates/api/src/main.rs

use axum::{Router, routing::{get, post}};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    let config = core::config::load()?;
    let pool = sqlx::PgPool::connect(&config.database_url).await?;
    
    let app = Router::new()
        // Warnings
        .route("/api/warnings", get(routes::warnings::list))
        .route("/api/warnings/:id", get(routes::warnings::get))
        .route("/api/warnings/:id/acknowledge", post(routes::warnings::acknowledge))
        
        // Insights
        .route("/api/insights", get(routes::insights::list))
        .route("/api/insights/weekly-memo", get(routes::insights::weekly_memo))
        
        // Companies
        .route("/api/companies", get(routes::companies::list))
        .route("/api/companies/:id", get(routes::companies::get))
        .route("/api/companies/:id/dossier", get(routes::dossiers::company))
        
        // Persons (POI)
        .route("/api/persons", get(routes::persons::list))
        .route("/api/persons/:id", get(routes::persons::get))
        .route("/api/persons/:id/dossier", get(routes::dossiers::person))
        .route("/api/persons/:id/engagement", get(routes::persons::engagement))
        
        // Competitors
        .route("/api/competitors", get(routes::companies::competitors))
        .route("/api/competitors/:id/changes", get(routes::companies::competitor_changes))
        
        // Recipes
        .route("/api/recipes", get(routes::recipes::list))
        .route("/api/recipes/staging", get(routes::recipes::staging))
        .route("/api/recipes/:id/promote", post(routes::recipes::promote))
        
        // Graph
        .route("/api/graph/neighborhood/:id", get(routes::graph::neighborhood))
        .route("/api/graph/path/:from/:to", get(routes::graph::shortest_path))
        
        // Search
        .route("/api/search", get(routes::search::search))
        
        // Security
        .route("/api/security/dns-posture", get(routes::security::dns_posture))
        .route("/api/security/lookalike-domains", get(routes::security::lookalikes))
        .route("/api/security/kev-relevance", get(routes::security::kev_relevance))
        
        // Admin
        .route("/api/admin/crawl-status", get(routes::admin::crawl_status))
        .route("/api/admin/recipe-performance", get(routes::admin::recipe_performance))
        .route("/api/admin/poi-coverage", get(routes::admin::poi_coverage))
        
        // WebSocket for real-time warnings
        .route("/ws/warnings", get(routes::ws::warnings_ws))
        
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(pool);
    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("ApexIntel API listening on :8080");
    axum::serve(listener, app).await?;
    Ok(())
}
```

---

## 13. Frontend Implementation

The frontend is built using the same Next.js + Tailwind + Radix UI stack as the Management-Software (Sensei OS) project at `/Users/sabelakhoua/IdeaProjects/Management-Software/frontend/`.

### 13.1 Key Dashboard Pages

| Route | Page | Description |
|---|---|---|
| `/` | Overview | KPI dashboard: active warnings, insights, POI coverage, recipe health |
| `/warnings` | Warning Center | Real-time warnings with severity, type, region filters |
| `/insights` | Insight Feed | Ranked insights with evidence links |
| `/memos` | Strategy Memos | Weekly memos for GM/board |
| `/companies` | Company Browser | Searchable company profiles with dossier links |
| `/companies/[id]` | Company Dossier | Full company profile, capabilities, sites, graph |
| `/persons` | POI Browser | Searchable POI profiles with filters by region/role |
| `/persons/[id]` | POI Dossier | Full POI profile, priority vector, engagement guide |
| `/competitors` | Competitor Dashboard | Threat/overlap scores, change events, comparison |
| `/security` | Security Center | DNS posture, lookalike domains, KEV relevance |
| `/graph` | Graph Explorer | Interactive network visualization (D3/vis.js) |
| `/recipes` | Recipe Manager | Production/staging/deprecated recipes, performance |
| `/settings` | Settings | API keys, regions, watchlist, schedules |

### 13.2 Dependencies (from Sensei OS template)

```json
{
  "dependencies": {
    "next": "^14",
    "@radix-ui/react-alert-dialog": "1.0.5",
    "@radix-ui/react-avatar": "1.0.4",
    "@radix-ui/react-checkbox": "1.0.4",
    "@radix-ui/react-collapsible": "^1.1.12",
    "@radix-ui/react-dialog": "1.0.5",
    "@radix-ui/react-dropdown-menu": "2.0.6",
    "@radix-ui/react-hover-card": "1.0.7",
    "@radix-ui/react-label": "2.0.2",
    "@radix-ui/react-popover": "1.0.7",
    "@radix-ui/react-select": "2.0.0",
    "@radix-ui/react-separator": "1.0.3",
    "@radix-ui/react-slider": "1.1.2",
    "@radix-ui/react-switch": "1.0.3",
    "@radix-ui/react-tabs": "1.0.4",
    "@radix-ui/react-tooltip": "1.0.7",
    "tailwindcss": "^3.4",
    "recharts": "^2.10",
    "d3": "^7",
    "lucide-react": "^0.300",
    "class-variance-authority": "^0.7",
    "clsx": "^2",
    "tailwind-merge": "^2",
    "react-force-graph-2d": "^1.25"
  }
}
```

---

## 14. API Keys & External Services

### 14.1 Available Keys (from CRM-v2)

```env
# .env (ApexIntel)

# === SEARCH ===
GOOGLE_API_KEY=AIzaSyCu5vHsBXbgAVposqicRNIv4ZQc27ZY-jM
GOOGLE_SEARCH_ENGINE_ID=32c9c316cea6a43fa
ENABLE_GOOGLE_API_FALLBACK=true

# === COMPONENT DATA (Nexar/Mouser/DigiKey for supply chain intel) ===
NEXAR_CLIENT_ID=bcb4b562-1251-4143-a63c-325796936db2
NEXAR_CLIENT_SECRET=DJYJf4wNtF8zkaG5kX74dt1IBl7yB-CNap7F
MOUSER_API_KEY=8b99a0c4-0f67-4cf4-8192-40b168d1f3e0
DIGIKEY_CLIENT_ID=AY4ZD9nZAsRSGnRfbYR1DOmYKG62rhn6rRkSDlmnAEaPIz2y
DIGIKEY_CLIENT_SECRET=cmbiC5a62wm5CzZUCYPvmFZUPTGAMLo5ZX498iYjJfaupfWuu6iU6WMPsVraBLKn

# === EMAIL (reuse from CRM-v2 for alerts) ===
SMTP_HOST=smtp.infomaniak.com
SMTP_PORT=587
SMTP_USER=contact@starzelectronics.site
SMTP_PASS=RiRrvI3t5XvK_
ALERT_FROM=intel@starzelectronics.site

# === PROXY ===
ENABLE_PROXY_ROTATION=true
# SCRAPER_PROXY_URL=http://user:pass@brd.superproxy.io:33335  # if using BrightData

# === LLM ===
LLM_API_KEY=       # OpenAI or local model key
LLM_BASE_URL=https://api.openai.com/v1
LLM_MODEL=gpt-4o

# === DATABASE ===
DATABASE_URL=postgres://apexintel:CHANGE_ME@localhost:5432/apexintel

# === OBJECT STORAGE ===
MINIO_ENDPOINT=http://localhost:9000
MINIO_ACCESS_KEY=apexintel
MINIO_SECRET_KEY=CHANGE_ME
MINIO_BUCKET=apexintel-raw

# === MESSAGE QUEUE ===
NATS_URL=nats://localhost:4222

# === REDIS ===
REDIS_URL=redis://localhost:6379
```

### 14.2 External APIs Used

| Service | Purpose | Cost |
|---|---|---|
| Google Custom Search | Fallback search when 27 free engines exhausted | $5/1000 queries |
| Nexar (Octopart) | Component supply chain data, PCN/PDN tracking | Free tier + paid |
| Mouser API | Component availability, pricing signals | Free |
| DigiKey API | Component lifecycle data | Free |
| crt.sh | Certificate Transparency logs | Free |
| CISA KEV | Known Exploited Vulnerabilities | Free |
| LME/FRED | Commodity prices, FX rates | Free |
| OpenAI / local LLM | Hypothesis generation, POI synthesis | Variable |

---

## 15. Deployment

### 15.1 Build

```bash
# Backend
cargo build --release --workspace

# Frontend (from Sensei OS template)
cd frontend
npm install
npm run build
```

### 15.2 Production Deployment

```bash
# Start infrastructure
docker compose up -d postgres redis nats minio

# Run migrations
sqlx migrate run --source crates/store/migrations/

# Start services
docker compose up -d api worker frontend
```

### 15.3 Monitoring

- **Tracing**: OpenTelemetry → Jaeger/Grafana Tempo
- **Metrics**: Prometheus endpoint on `/metrics`
- **Logs**: Structured JSON → Loki/CloudWatch
- **Dashboards**: Grafana dashboards for:
  - Crawl success rates per domain/region
  - Warning latency (trigger → delivery)
  - Recipe precision/recall trends
  - POI coverage by region
  - Engine health scores
  - False alarm rates

### 15.4 Operational Schedule

| Job | Frequency | Runtime |
|---|---|---|
| Critical source crawl (KEV, DNS, CT) | Hourly | ~5 min |
| Standard crawl cycle (company pages, tenders) | 6h | ~30 min |
| Commodity/FX refresh | Hourly | ~1 min |
| Port/logistics refresh | 6h | ~5 min |
| Feature store materialization | 4h | ~10 min |
| Recipe evaluation (all entities) | 4h | ~15 min |
| POI artifact ingestion | Daily | ~30 min |
| POI profile recomputation | Daily | ~20 min |
| Pattern mining | Nightly | ~2h |
| LLM hypothesis generation | Nightly | ~30 min |
| Weekly strategy memo | Weekly (Sun 06:00) | ~15 min |
| Recipe promotion board | Weekly (Mon 03:00) | ~5 min |
| Recipe deprecation | Weekly (Mon 03:15) | ~5 min |

---

*ApexIntel — Complete OSINT Intelligence Platform for EMS & Email Security*  
*Built with Rust + Next.js, continuously learning, region-aware, POI-deep.*
