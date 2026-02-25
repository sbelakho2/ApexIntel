// ─── ApexIntel Mock Data ────────────────────────────────────────
// Realistic seed data for all dashboard views.
// In production, these are replaced by API calls to the Rust backend.

// ─── Warnings ───────────────────────────────────────────────────

export type Severity = "critical" | "high" | "medium" | "low";
export type WarningType =
  | "outsourcing_window"
  | "competitor_move"
  | "supply_chain_shock"
  | "margin_regime_shift"
  | "regulatory_shock"
  | "brand_impersonation"
  | "dns_posture_drift"
  | "third_party_compromise"
  | "phishing_campaign"
  | "kev_relevance";

export interface Warning {
  id: string;
  severity: Severity;
  type: WarningType;
  title: string;
  region: string;
  status: "active" | "acknowledged" | "resolved";
  created: string;
  sla_hours: number;
}

export const WARNINGS: Warning[] = [
  { id: "W-001", severity: "critical", type: "brand_impersonation", title: "Lookalike domain starzelectronics.org registered via Namecheap", region: "Global", status: "active", created: "2026-02-23T08:14:00Z", sla_hours: 1 },
  { id: "W-002", severity: "critical", type: "supply_chain_shock", title: "PCN burst detected: TDK capacitor series C3225 end-of-life", region: "East Asia", status: "active", created: "2026-02-23T06:30:00Z", sla_hours: 2 },
  { id: "W-003", severity: "high", type: "competitor_move", title: "All Circuits added IATF 16949 automotive cert — scope expansion", region: "Tunisia", status: "active", created: "2026-02-22T22:10:00Z", sla_hours: 12 },
  { id: "W-004", severity: "high", type: "outsourcing_window", title: "Valeo Tunis posted 3 SMT operator roles + supplier portal update", region: "Tunisia", status: "acknowledged", created: "2026-02-22T18:45:00Z", sla_hours: 4 },
  { id: "W-005", severity: "high", type: "dns_posture_drift", title: "DMARC policy weakened to p=none on apexmail.com", region: "Global", status: "active", created: "2026-02-22T14:22:00Z", sla_hours: 2 },
  { id: "W-006", severity: "medium", type: "margin_regime_shift", title: "Copper LME +8.2% in 5 sessions, TND/EUR weakening 1.4%", region: "EMEA", status: "active", created: "2026-02-22T12:00:00Z", sla_hours: 6 },
  { id: "W-007", severity: "medium", type: "regulatory_shock", title: "EU CBAM Phase 2 tariff adjustment on HS 8534 PCB imports", region: "EU", status: "acknowledged", created: "2026-02-22T09:30:00Z", sla_hours: 4 },
  { id: "W-008", severity: "medium", type: "kev_relevance", title: "CVE-2026-1842 added to CISA KEV — Fortinet FortiGate RCE", region: "Global", status: "active", created: "2026-02-21T20:00:00Z", sla_hours: 4 },
  { id: "W-009", severity: "low", type: "competitor_move", title: "Fideltronik Poland updated capabilities page — added box-build", region: "EU", status: "resolved", created: "2026-02-21T16:40:00Z", sla_hours: 12 },
  { id: "W-010", severity: "low", type: "phishing_campaign", title: "Bulk .xyz registrations matching 'apexmail' keyword pattern", region: "Global", status: "active", created: "2026-02-21T11:15:00Z", sla_hours: 1 },
  { id: "W-011", severity: "high", type: "third_party_compromise", title: "Supplier breach: PCB vendor ChinaPCB data leak reported", region: "China", status: "active", created: "2026-02-21T08:00:00Z", sla_hours: 1 },
  { id: "W-012", severity: "medium", type: "outsourcing_window", title: "Schneider Electric Morocco RFQ portal showing new EMS tender", region: "Morocco", status: "active", created: "2026-02-20T14:30:00Z", sla_hours: 4 },
  { id: "W-013", severity: "critical", type: "supply_chain_shock", title: "Tanger Med port congestion index surged to 94/100", region: "Morocco", status: "acknowledged", created: "2026-02-20T10:00:00Z", sla_hours: 2 },
  { id: "W-014", severity: "low", type: "competitor_move", title: "Kitron ASA press release: new Lithuania facility expansion", region: "EU", status: "resolved", created: "2026-02-19T17:20:00Z", sla_hours: 12 },
  { id: "W-015", severity: "high", type: "brand_impersonation", title: "CT log: certificate issued for starz-electronics.com on Let's Encrypt", region: "Global", status: "active", created: "2026-02-19T09:45:00Z", sla_hours: 1 },
];

// ─── Warning time series (last 30 days) ─────────────────────────
// Deterministic seeded PRNG to avoid SSR/client hydration mismatches.
function seededRandom(seed: number): number {
  const x = Math.sin(seed + 1) * 10000;
  return x - Math.floor(x);
}

export const WARNING_TREND = Array.from({ length: 30 }, (_, i) => {
  const d = new Date(2026, 1, 23);
  d.setDate(d.getDate() - (29 - i));
  return {
    date: d.toISOString().slice(5, 10),
    critical: Math.floor(seededRandom(i * 4 + 1) * 3),
    high: Math.floor(seededRandom(i * 4 + 2) * 5) + 1,
    medium: Math.floor(seededRandom(i * 4 + 3) * 4) + 2,
    low: Math.floor(seededRandom(i * 4 + 4) * 3) + 1,
  };
});

// ─── Insights ───────────────────────────────────────────────────

export interface Insight {
  id: string;
  title: string;
  type: "demand_signal" | "supply_risk" | "competitive_intel" | "security_posture" | "macro_shift" | "poi_movement";
  region: string;
  confidence: number;
  impact: "high" | "medium" | "low";
  sources: number;
  created: string;
  summary: string;
}

export const INSIGHTS: Insight[] = [
  { id: "I-001", title: "Automotive EMS demand surge in Morocco — 3 new OEM tenders in 14 days", type: "demand_signal", region: "Morocco", confidence: 0.87, impact: "high", sources: 4, created: "2026-02-23", summary: "TUNEPS + marchespublics.gov.ma + Valeo hiring + Renault press release converge on Q2 production ramp." },
  { id: "I-002", title: "Copper price regime shift may compress PCB margins by 6-9%", type: "macro_shift", region: "Global", confidence: 0.78, impact: "high", sources: 3, created: "2026-02-22", summary: "LME copper 8.2% rally + TND weakening + China smelter cutbacks signal sustained upward pressure." },
  { id: "I-003", title: "All Circuits expanding automotive capability — direct competitive threat", type: "competitive_intel", region: "Tunisia", confidence: 0.92, impact: "high", sources: 5, created: "2026-02-22", summary: "IATF cert + 2 automotive job posts + trade show presence at SIAT + capability page update." },
  { id: "I-004", title: "DMARC enforcement relaxation exposes ApexMail to spoofing risk", type: "security_posture", region: "Global", confidence: 0.95, impact: "medium", sources: 2, created: "2026-02-22", summary: "DNS query shows DMARC moved from reject to none. SPF record unchanged but MX updated." },
  { id: "I-005", title: "Israel defense electronics procurement cycle opening — SIBAT tenders", type: "demand_signal", region: "Israel", confidence: 0.72, impact: "medium", sources: 3, created: "2026-02-21", summary: "mr.gov.il listing + IIA newsletter + Rafael LinkedIn posts signal FY27 budget allocation." },
  { id: "I-006", title: "TDK capacitor EOL will affect 23% of active BOM positions", type: "supply_risk", region: "East Asia", confidence: 0.88, impact: "high", sources: 2, created: "2026-02-21", summary: "PCN cross-reference against active BOMs shows 14 affected assemblies across 3 customers." },
  { id: "I-007", title: "Foxconn restructuring Zhengzhou plant — potential overflow opportunity", type: "competitive_intel", region: "China", confidence: 0.65, impact: "medium", sources: 3, created: "2026-02-20", summary: "DigiTimes report + job postings shift + Shenzhen facility expansion signal capacity rebalance." },
  { id: "I-008", title: "EU CBAM tariff Phase 2 impacts North Africa PCB exports", type: "macro_shift", region: "EU", confidence: 0.82, impact: "medium", sources: 2, created: "2026-02-20", summary: "Official Journal publication + HS code impact analysis on 8534/8542 families." },
  { id: "I-009", title: "POI movement: Actia Group CTO departed — succession uncertainty", type: "poi_movement", region: "Tunisia", confidence: 0.90, impact: "medium", sources: 2, created: "2026-02-19", summary: "LinkedIn profile update + press release confirm departure. No successor named." },
  { id: "I-010", title: "Shenzhen port congestion easing — logistics window opening", type: "supply_risk", region: "China", confidence: 0.74, impact: "low", sources: 2, created: "2026-02-18", summary: "MarineTraffic congestion index down 18 points. Container availability improving." },
];

export const INSIGHT_TREND = Array.from({ length: 30 }, (_, i) => {
  const d = new Date(2026, 1, 23);
  d.setDate(d.getDate() - (29 - i));
  return {
    date: d.toISOString().slice(5, 10),
    demand_signal: Math.floor(seededRandom(i * 5 + 100) * 3) + 1,
    competitive_intel: Math.floor(seededRandom(i * 5 + 101) * 4) + 1,
    supply_risk: Math.floor(seededRandom(i * 5 + 102) * 2) + 1,
    security_posture: Math.floor(seededRandom(i * 5 + 103) * 2),
    macro_shift: Math.floor(seededRandom(i * 5 + 104) * 2),
  };
});

// ─── Companies ──────────────────────────────────────────────────

export interface Company {
  id: string;
  name: string;
  domain: string;
  region: string;
  type: string;
  tier: number;
  risk_score: number;
  capabilities: string[];
  updated: string;
}

export const COMPANIES: Company[] = [
  { id: "C-001", name: "Telnet Holding", domain: "telnet-group.com", region: "Tunisia", type: "EMS", tier: 1, risk_score: 72, capabilities: ["SMT", "PTH", "Box Build", "Testing"], updated: "2026-02-22" },
  { id: "C-002", name: "All Circuits", domain: "all-circuits.com", region: "Tunisia/Morocco", type: "EMS", tier: 1, risk_score: 81, capabilities: ["SMT", "BGA", "Automotive", "Conformal Coating"], updated: "2026-02-23" },
  { id: "C-003", name: "Actia Group", domain: "actia.com", region: "Tunisia", type: "EMS", tier: 1, risk_score: 65, capabilities: ["Automotive Electronics", "Telematics", "IoT"], updated: "2026-02-21" },
  { id: "C-004", name: "Eolane", domain: "eolane.com", region: "Morocco", type: "EMS", tier: 1, risk_score: 58, capabilities: ["SMT", "Industrial", "Defense"], updated: "2026-02-20" },
  { id: "C-005", name: "Flex Ltd", domain: "flex.com", region: "Global", type: "EMS", tier: 3, risk_score: 34, capabilities: ["Full Lifecycle", "Design", "Manufacturing", "Logistics"], updated: "2026-02-22" },
  { id: "C-006", name: "Jabil Inc", domain: "jabil.com", region: "Global", type: "EMS", tier: 3, risk_score: 38, capabilities: ["Healthcare", "Automotive", "Cloud", "Consumer"], updated: "2026-02-22" },
  { id: "C-007", name: "Tower Semiconductor", domain: "towersc.com", region: "Israel", type: "Semiconductor Foundry", tier: 4, risk_score: 45, capabilities: ["Analog", "Mixed-Signal", "RF", "Power"], updated: "2026-02-21" },
  { id: "C-008", name: "Foxconn (Hon Hai)", domain: "foxconn.com", region: "Taiwan/China", type: "EMS", tier: 5, risk_score: 52, capabilities: ["Consumer Electronics", "Server", "Automotive", "PCB"], updated: "2026-02-23" },
  { id: "C-009", name: "Luxshare Precision", domain: "luxshare-ict.com", region: "China", type: "EMS / Connectors", tier: 5, risk_score: 61, capabilities: ["Connectors", "Acoustics", "AR/VR", "Wearables"], updated: "2026-02-22" },
  { id: "C-010", name: "Nano Dimension", domain: "nano-di.com", region: "Israel", type: "Additive PCB", tier: 4, risk_score: 29, capabilities: ["3D Printed Electronics", "AME", "PCB Prototyping"], updated: "2026-02-19" },
  { id: "C-011", name: "KATEK SE", domain: "katek.de", region: "Germany", type: "EMS", tier: 2, risk_score: 47, capabilities: ["Automotive", "Industrial", "Renewable Energy"], updated: "2026-02-20" },
  { id: "C-012", name: "Coficab", domain: "coficab.com", region: "Tunisia", type: "Cable / Harness", tier: 1, risk_score: 55, capabilities: ["Automotive Cable", "Harness Assembly", "Energy Cable"], updated: "2026-02-18" },
];

export const COMPANY_REGION_DIST = [
  { name: "Tunisia", value: 4, fill: "#FFBE00" },
  { name: "Morocco", value: 2, fill: "#D62D2D" },
  { name: "Israel", value: 2, fill: "#4A90E2" },
  { name: "EU", value: 2, fill: "#2D8C3C" },
  { name: "East Asia", value: 3, fill: "#999999" },
  { name: "Global", value: 2, fill: "#666666" },
];

// ─── Persons (POIs) ─────────────────────────────────────────────

export interface Person {
  id: string;
  name: string;
  role: string;
  organization: string;
  region: string;
  priority: "A" | "B" | "C";
  influence_score: number;
  last_signal: string;
  tags: string[];
}

export const PERSONS: Person[] = [
  { id: "P-001", name: "Anis Ben Salah", role: "VP Procurement", organization: "Valeo Tunisia", region: "Tunisia", priority: "A", influence_score: 92, last_signal: "2026-02-22", tags: ["automotive", "procurement", "OEM"] },
  { id: "P-002", name: "Leila Gharbi", role: "Director, FIPA Tunisia", organization: "FIPA", region: "Tunisia", priority: "A", influence_score: 88, last_signal: "2026-02-21", tags: ["government", "investment", "regulation"] },
  { id: "P-003", name: "Youssef Alaoui", role: "GM, Tanger Med", organization: "TMSA", region: "Morocco", priority: "A", influence_score: 85, last_signal: "2026-02-20", tags: ["logistics", "port", "free-zone"] },
  { id: "P-004", name: "Moshe Levy", role: "CTO", organization: "Tower Semiconductor", region: "Israel", priority: "B", influence_score: 78, last_signal: "2026-02-19", tags: ["semiconductor", "technology", "R&D"] },
  { id: "P-005", name: "Chen Wei", role: "VP Supply Chain", organization: "Luxshare Precision", region: "China", priority: "B", influence_score: 75, last_signal: "2026-02-22", tags: ["supply-chain", "EMS", "connectors"] },
  { id: "P-006", name: "Hans Müller", role: "SQE Director", organization: "Continental AG", region: "EU", priority: "A", influence_score: 90, last_signal: "2026-02-23", tags: ["quality", "automotive", "supplier-dev"] },
  { id: "P-007", name: "Fatima Ouazzani", role: "Director, AMDIE Morocco", organization: "AMDIE", region: "Morocco", priority: "B", influence_score: 82, last_signal: "2026-02-18", tags: ["government", "investment", "trade"] },
  { id: "P-008", name: "Takeshi Yamada", role: "Component Procurement Lead", organization: "Murata Manufacturing", region: "Japan", priority: "C", influence_score: 68, last_signal: "2026-02-17", tags: ["components", "passives", "supply"] },
  { id: "P-009", name: "Sarah Cohen", role: "Procurement Officer, SIBAT", organization: "Israel MoD", region: "Israel", priority: "A", influence_score: 94, last_signal: "2026-02-21", tags: ["defense", "procurement", "government"] },
  { id: "P-010", name: "Marc Dupont", role: "VP Operations", organization: "Eolane", region: "Morocco", priority: "B", influence_score: 71, last_signal: "2026-02-15", tags: ["EMS", "operations", "competitor"] },
  { id: "P-011", name: "Li Jing", role: "GM, Shenzhen SEZ", organization: "Shenzhen Municipal Gov", region: "China", priority: "C", influence_score: 64, last_signal: "2026-02-20", tags: ["government", "free-zone", "policy"] },
  { id: "P-012", name: "Rami Khoury", role: "Program Manager", organization: "Rafael Defense", region: "Israel", priority: "B", influence_score: 79, last_signal: "2026-02-22", tags: ["defense", "electronics", "program-mgmt"] },
];

// ─── Competitors ────────────────────────────────────────────────

export interface Competitor {
  id: string;
  name: string;
  region: string;
  tier: number;
  threat_score: number;
  overlap_pct: number;
  recent_changes: string[];
  updated: string;
  metrics: { capability: number; pricing: number; quality: number; delivery: number; geography: number };
}

export const COMPETITORS: Competitor[] = [
  { id: "X-001", name: "All Circuits", region: "Tunisia/Morocco", tier: 1, threat_score: 89, overlap_pct: 78, recent_changes: ["IATF 16949 cert added", "3 new automotive hires", "SIAT trade show speaker"], updated: "2026-02-23", metrics: { capability: 82, pricing: 75, quality: 85, delivery: 70, geography: 90 } },
  { id: "X-002", name: "Telnet Holding", region: "Tunisia", tier: 1, threat_score: 76, overlap_pct: 72, recent_changes: ["New box-build capacity page", "IPC APEX exhibitor"], updated: "2026-02-22", metrics: { capability: 78, pricing: 80, quality: 72, delivery: 68, geography: 88 } },
  { id: "X-003", name: "Actia Group", region: "Tunisia", tier: 1, threat_score: 68, overlap_pct: 55, recent_changes: ["CTO departure announced", "Telematics contract win"], updated: "2026-02-21", metrics: { capability: 74, pricing: 65, quality: 80, delivery: 72, geography: 82 } },
  { id: "X-004", name: "Eolane", region: "Morocco", tier: 1, threat_score: 62, overlap_pct: 48, recent_changes: ["Morocco facility expansion", "Defense contract renewal"], updated: "2026-02-20", metrics: { capability: 70, pricing: 68, quality: 75, delivery: 65, geography: 78 } },
  { id: "X-005", name: "KATEK SE", region: "Germany", tier: 2, threat_score: 54, overlap_pct: 35, recent_changes: ["Renewable energy SMT line", "Poland site investment"], updated: "2026-02-19", metrics: { capability: 85, pricing: 55, quality: 88, delivery: 80, geography: 45 } },
  { id: "X-006", name: "Flex Ltd", region: "Global", tier: 3, threat_score: 45, overlap_pct: 22, recent_changes: ["Automotive division restructuring", "India plant ramp"], updated: "2026-02-22", metrics: { capability: 95, pricing: 40, quality: 90, delivery: 85, geography: 30 } },
  { id: "X-007", name: "Foxconn", region: "Taiwan/China", tier: 5, threat_score: 38, overlap_pct: 12, recent_changes: ["Zhengzhou restructuring", "Vietnam expansion"], updated: "2026-02-23", metrics: { capability: 98, pricing: 30, quality: 82, delivery: 70, geography: 15 } },
];

// ─── Security ───────────────────────────────────────────────────

export interface SecurityFinding {
  id: string;
  domain: string;
  risk: "critical" | "high" | "medium" | "low";
  signal: string;
  type: "dns_posture" | "lookalike" | "kev" | "breach" | "phishing";
  detected: string;
  details: string;
}

export const SECURITY_FINDINGS: SecurityFinding[] = [
  { id: "S-001", domain: "starzelectronics.org", risk: "critical", signal: "Lookalike domain registration", type: "lookalike", detected: "2026-02-23", details: "Domain registered 24h ago on Namecheap. A record points to Cloudflare. No content yet." },
  { id: "S-002", domain: "apexmail.com", risk: "high", signal: "DMARC policy weakened to p=none", type: "dns_posture", detected: "2026-02-22", details: "Previous: p=reject. Current: p=none. Change detected in DNS TXT record." },
  { id: "S-003", domain: "starz-electronics.com", risk: "high", signal: "Certificate issued via Let's Encrypt", type: "lookalike", detected: "2026-02-19", details: "CT log shows cert issuance. Domain resolves to hosting provider in Ukraine." },
  { id: "S-004", domain: "starzelectronics.site", risk: "medium", signal: "CVE-2026-1842 affects FortiGate stack", type: "kev", detected: "2026-02-21", details: "CISA KEV addition. FortiGate version in use may be affected. Patch available." },
  { id: "S-005", domain: "chinapcb-supplier.com", risk: "high", signal: "Supplier data breach reported", type: "breach", detected: "2026-02-21", details: "Third-party vendor ChinaPCB reported breach affecting customer PO data." },
  { id: "S-006", domain: "apexmail-login.xyz", risk: "medium", signal: "Phishing domain cluster detected", type: "phishing", detected: "2026-02-21", details: "3 .xyz domains matching apexmail pattern registered in 48h window." },
  { id: "S-007", domain: "starzelectronics.site", risk: "low", signal: "SPF record includes soft-fail", type: "dns_posture", detected: "2026-02-18", details: "SPF record uses ~all instead of -all. Recommend tightening." },
  { id: "S-008", domain: "starzelectronics.site", risk: "medium", signal: "DKIM key rotation overdue (180+ days)", type: "dns_posture", detected: "2026-02-20", details: "DKIM selector key last rotated 192 days ago. Best practice is 90 days." },
];

export interface DnsPosture {
  check: string;
  status: "pass" | "warn" | "fail";
  value: string;
}

export const DNS_POSTURE: DnsPosture[] = [
  { check: "SPF Record", status: "warn", value: "v=spf1 include:_spf.google.com ~all" },
  { check: "DKIM Signing", status: "warn", value: "Key age: 192 days (rotate at 90)" },
  { check: "DMARC Policy", status: "fail", value: "p=none (was p=reject)" },
  { check: "MX Records", status: "pass", value: "2 MX records, priority 10/20" },
  { check: "DNSSEC", status: "pass", value: "DNSSEC enabled, chain valid" },
  { check: "CAA Record", status: "pass", value: "issue: letsencrypt.org" },
  { check: "BIMI Record", status: "warn", value: "Not configured" },
  { check: "MTA-STS", status: "fail", value: "No MTA-STS policy found" },
];

// ─── Recipes ────────────────────────────────────────────────────

export interface Recipe {
  id: string;
  name: string;
  description: string;
  status: "production" | "staging" | "deprecated";
  precision: number;
  recall: number;
  fpr: number;
  alerts_7d: number;
  created: string;
  promoted: string | null;
}

export const RECIPES: Recipe[] = [
  { id: "R-001", name: "outsourcing-window-detect", description: "Detects job post Δ + supplier page change-point convergence", status: "production", precision: 0.84, recall: 0.79, fpr: 0.06, alerts_7d: 12, created: "2025-11-15", promoted: "2026-01-03" },
  { id: "R-002", name: "competitor-capability-shift", description: "Tracks capability page changes + cert additions + hiring patterns", status: "production", precision: 0.91, recall: 0.72, fpr: 0.03, alerts_7d: 8, created: "2025-10-22", promoted: "2025-12-15" },
  { id: "R-003", name: "supply-chain-pcn-burst", description: "Monitors PCN/PDN burst + allocation keywords + port congestion", status: "production", precision: 0.88, recall: 0.85, fpr: 0.05, alerts_7d: 5, created: "2025-09-10", promoted: "2025-11-20" },
  { id: "R-004", name: "margin-regime-detector", description: "Commodity vol change-point + FX vol + demand proxy correlation", status: "production", precision: 0.76, recall: 0.82, fpr: 0.08, alerts_7d: 3, created: "2025-12-01", promoted: "2026-02-01" },
  { id: "R-005", name: "brand-impersonation-v2", description: "CT log + lookalike domain + registrar pattern detection", status: "staging", precision: 0.92, recall: 0.68, fpr: 0.02, alerts_7d: 4, created: "2026-01-20", promoted: null },
  { id: "R-006", name: "dns-posture-monitor", description: "DMARC/SPF/DKIM drift detection + MX change tracking", status: "production", precision: 0.96, recall: 0.91, fpr: 0.01, alerts_7d: 2, created: "2025-08-05", promoted: "2025-10-01" },
  { id: "R-007", name: "poi-role-change-detect", description: "LinkedIn + press + patent inventor cross-reference for role shifts", status: "staging", precision: 0.73, recall: 0.65, fpr: 0.11, alerts_7d: 6, created: "2026-02-10", promoted: null },
  { id: "R-008", name: "tender-opportunity-scorer", description: "Multi-portal tender parsing + entity extraction + relevance scoring", status: "production", precision: 0.80, recall: 0.88, fpr: 0.07, alerts_7d: 15, created: "2025-07-18", promoted: "2025-09-28" },
  { id: "R-009", name: "phishing-campaign-cluster", description: "Bulk registration + keyword pattern + procurement season correlation", status: "deprecated", precision: 0.62, recall: 0.55, fpr: 0.18, alerts_7d: 0, created: "2025-06-01", promoted: "2025-08-15" },
  { id: "R-010", name: "kev-stack-mapper", description: "New KEV entry + EMS technology stack cross-reference", status: "production", precision: 0.89, recall: 0.78, fpr: 0.04, alerts_7d: 1, created: "2025-11-28", promoted: "2026-01-10" },
];

export const RECIPE_PERF_TREND = Array.from({ length: 12 }, (_, i) => {
  const d = new Date(2026, 1, 23);
  d.setMonth(d.getMonth() - (11 - i));
  return {
    month: d.toISOString().slice(0, 7),
    precision: +(0.72 + seededRandom(i * 3 + 200) * 0.18).toFixed(2),
    recall: +(0.65 + seededRandom(i * 3 + 201) * 0.20).toFixed(2),
    fpr: +(0.02 + seededRandom(i * 3 + 202) * 0.10).toFixed(2),
  };
});

// ─── Graph Nodes/Edges ──────────────────────────────────────────

export interface GraphNode {
  id: string;
  label: string;
  type: "company" | "person" | "region" | "domain" | "cert" | "tender";
  x: number;
  y: number;
  size: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  label: string;
  weight: number;
}

export const GRAPH_NODES: GraphNode[] = [
  { id: "n1", label: "Starz Electronics", type: "company", x: 400, y: 300, size: 28 },
  { id: "n2", label: "All Circuits", type: "company", x: 200, y: 150, size: 22 },
  { id: "n3", label: "Telnet Holding", type: "company", x: 150, y: 400, size: 20 },
  { id: "n4", label: "Valeo Tunisia", type: "company", x: 600, y: 180, size: 24 },
  { id: "n5", label: "Anis Ben Salah", type: "person", x: 650, y: 280, size: 16 },
  { id: "n6", label: "Leila Gharbi", type: "person", x: 350, y: 500, size: 16 },
  { id: "n7", label: "Tunisia", type: "region", x: 300, y: 100, size: 18 },
  { id: "n8", label: "Morocco", type: "region", x: 100, y: 280, size: 18 },
  { id: "n9", label: "IATF 16949", type: "cert", x: 250, y: 250, size: 14 },
  { id: "n10", label: "TUNEPS Tender #4821", type: "tender", x: 500, y: 450, size: 14 },
  { id: "n11", label: "Tower Semi", type: "company", x: 650, y: 420, size: 20 },
  { id: "n12", label: "Israel", type: "region", x: 700, y: 350, size: 18 },
  { id: "n13", label: "Moshe Levy", type: "person", x: 720, y: 480, size: 16 },
  { id: "n14", label: "Eolane", type: "company", x: 80, y: 450, size: 18 },
  { id: "n15", label: "Schneider Elec.", type: "company", x: 500, y: 100, size: 22 },
];

export const GRAPH_EDGES: GraphEdge[] = [
  { source: "n1", target: "n7", label: "headquartered_in", weight: 3 },
  { source: "n2", target: "n7", label: "headquartered_in", weight: 3 },
  { source: "n3", target: "n7", label: "headquartered_in", weight: 3 },
  { source: "n4", target: "n7", label: "operates_in", weight: 2 },
  { source: "n14", target: "n8", label: "operates_in", weight: 2 },
  { source: "n1", target: "n2", label: "competes_with", weight: 4 },
  { source: "n1", target: "n3", label: "competes_with", weight: 3 },
  { source: "n2", target: "n9", label: "holds_cert", weight: 2 },
  { source: "n1", target: "n9", label: "holds_cert", weight: 2 },
  { source: "n5", target: "n4", label: "works_at", weight: 3 },
  { source: "n6", target: "n7", label: "governs", weight: 2 },
  { source: "n1", target: "n10", label: "bid_on", weight: 2 },
  { source: "n4", target: "n10", label: "issuer_of", weight: 3 },
  { source: "n11", target: "n12", label: "headquartered_in", weight: 3 },
  { source: "n13", target: "n11", label: "works_at", weight: 3 },
  { source: "n1", target: "n4", label: "supplier_to", weight: 4 },
  { source: "n15", target: "n8", label: "operates_in", weight: 2 },
  { source: "n14", target: "n1", label: "competes_with", weight: 2 },
  { source: "n1", target: "n15", label: "supplier_to", weight: 3 },
];

// ─── Memo ───────────────────────────────────────────────────────

export interface MemoSection {
  heading: string;
  bullets: string[];
}

export interface Memo {
  id: string;
  title: string;
  period: string;
  published: string;
  author: string;
  kpis: { label: string; value: string; delta: string; direction: "up" | "down" | "flat" }[];
  sections: MemoSection[];
  actions: { priority: "P1" | "P2" | "P3"; action: string; owner: string; deadline: string }[];
}

export const LATEST_MEMO: Memo = {
  id: "M-2026-08",
  title: "Weekly Strategy Memo — Week 8, 2026",
  period: "Feb 17–23, 2026",
  published: "2026-02-23T06:00:00Z",
  author: "ApexIntel LLM Synthesis Engine",
  kpis: [
    { label: "Active Warnings", value: "11", delta: "+3", direction: "up" },
    { label: "New Insights", value: "10", delta: "+2", direction: "up" },
    { label: "Companies Tracked", value: "12", delta: "0", direction: "flat" },
    { label: "POIs Profiled", value: "12", delta: "+1", direction: "up" },
    { label: "Recipe Precision (avg)", value: "85%", delta: "+2%", direction: "up" },
    { label: "Crawl Success Rate", value: "94.2%", delta: "-0.8%", direction: "down" },
  ],
  sections: [
    {
      heading: "Demand & Procurement",
      bullets: [
        "Morocco automotive EMS demand surging: 3 new tenders from Valeo, Schneider Electric, and Renault in 14-day window. Recommend immediate bid preparation for Schneider RFQ.",
        "Israel defense electronics procurement cycle opening—SIBAT tenders visible on mr.gov.il. Tower Semiconductor and Rafael indicate FY27 budget allocation.",
        "TUNEPS showing increased activity in industrial electronics category. 2 tenders match Starz capability profile.",
      ],
    },
    {
      heading: "Competitive Intelligence",
      bullets: [
        "All Circuits obtained IATF 16949 certification, expanding into automotive—direct threat to Starz automotive pipeline. Monitor their Renault/PSA engagement closely.",
        "Actia Group CTO departed. Succession uncertainty creates potential window for Starz to approach Actia's customers.",
        "Foxconn restructuring Zhengzhou plant. Potential overflow opportunity in consumer electronics assembly.",
      ],
    },
    {
      heading: "Supply Chain & Macro",
      bullets: [
        "Copper LME rally (+8.2% in 5 sessions) plus TND weakening could compress PCB margins by 6-9%. Activate hedging protocol.",
        "TDK capacitor C3225 series EOL impacts 23% of active BOM positions across 14 assemblies. Initiate alternative qualification immediately.",
        "Tanger Med congestion index at 94/100. Recommend routing Morocco-bound shipments through Casablanca port temporarily.",
      ],
    },
    {
      heading: "Security Posture",
      bullets: [
        "Lookalike domain starzelectronics.org registered—potential brand impersonation. Takedown request initiated.",
        "DMARC policy weakened to p=none on apexmail.com—immediate remediation required.",
        "CVE-2026-1842 (Fortinet RCE) added to CISA KEV. Verify FortiGate patch status.",
      ],
    },
  ],
  actions: [
    { priority: "P1", action: "Restore DMARC to p=reject on apexmail.com", owner: "Security Team", deadline: "2026-02-24" },
    { priority: "P1", action: "Initiate TDK C3225 alternative qualification", owner: "Supply Chain", deadline: "2026-02-28" },
    { priority: "P1", action: "Submit bid for Schneider Electric Morocco RFQ", owner: "BD Team", deadline: "2026-02-27" },
    { priority: "P2", action: "Request takedown of starzelectronics.org", owner: "Legal/Security", deadline: "2026-02-26" },
    { priority: "P2", action: "Activate copper hedging protocol", owner: "Finance", deadline: "2026-02-25" },
    { priority: "P2", action: "Verify FortiGate patch for CVE-2026-1842", owner: "IT Security", deadline: "2026-02-25" },
    { priority: "P3", action: "Approach Actia customers during CTO transition", owner: "BD Team", deadline: "2026-03-07" },
    { priority: "P3", action: "Evaluate Foxconn overflow opportunity", owner: "Strategy", deadline: "2026-03-14" },
  ],
};

export const PAST_MEMOS = [
  { id: "M-2026-07", title: "Week 7, 2026", period: "Feb 10–16", published: "2026-02-16", highlights: 8, actions: 6 },
  { id: "M-2026-06", title: "Week 6, 2026", period: "Feb 3–9", published: "2026-02-09", highlights: 6, actions: 5 },
  { id: "M-2026-05", title: "Week 5, 2026", period: "Jan 27 – Feb 2", published: "2026-02-02", highlights: 7, actions: 7 },
  { id: "M-2026-04", title: "Week 4, 2026", period: "Jan 20–26", published: "2026-01-26", highlights: 5, actions: 4 },
];

// ─── Settings / Config ──────────────────────────────────────────

export interface ConfigSection {
  label: string;
  items: { key: string; value: string; type: "text" | "toggle" | "select"; options?: string[] }[];
}

export const SETTINGS_SECTIONS: ConfigSection[] = [
  {
    label: "API Keys & Services",
    items: [
      { key: "Google Custom Search", value: "Configured", type: "text" },
      { key: "Nexar (Octopart)", value: "Configured", type: "text" },
      { key: "Mouser API", value: "Configured", type: "text" },
      { key: "DigiKey API", value: "Configured", type: "text" },
      { key: "OpenAI LLM", value: "gpt-4o", type: "select", options: ["gpt-4o", "gpt-4o-mini", "local-llama"] },
    ],
  },
  {
    label: "Region Coverage",
    items: [
      { key: "Tunisia", value: "true", type: "toggle" },
      { key: "Morocco", value: "true", type: "toggle" },
      { key: "Israel", value: "true", type: "toggle" },
      { key: "China & East Asia", value: "true", type: "toggle" },
      { key: "EU / EMEA", value: "true", type: "toggle" },
      { key: "US", value: "true", type: "toggle" },
    ],
  },
  {
    label: "Scheduler",
    items: [
      { key: "Critical source crawl", value: "Hourly", type: "select", options: ["30min", "Hourly", "2h", "4h"] },
      { key: "Standard crawl cycle", value: "6h", type: "select", options: ["4h", "6h", "12h", "Daily"] },
      { key: "Commodity/FX refresh", value: "Hourly", type: "select", options: ["30min", "Hourly", "2h"] },
      { key: "Weekly memo generation", value: "Sun 06:00", type: "text" },
      { key: "Recipe evaluation", value: "4h", type: "select", options: ["2h", "4h", "6h", "8h"] },
    ],
  },
  {
    label: "Alerting",
    items: [
      { key: "Email notifications", value: "true", type: "toggle" },
      { key: "Alert email address", value: "intel@starzelectronics.site", type: "text" },
      { key: "Slack webhook", value: "false", type: "toggle" },
      { key: "Critical SLA threshold", value: "1h", type: "select", options: ["30min", "1h", "2h"] },
    ],
  },
];

// ─── Dashboard KPIs ─────────────────────────────────────────────

export const DASHBOARD_KPIS = {
  warnings_active: 11,
  warnings_resolved_7d: 4,
  insights_generated_7d: 10,
  companies_tracked: 12,
  pois_profiled: 12,
  recipes_production: 7,
  recipes_staging: 2,
  crawl_success_rate: 94.2,
  avg_recipe_precision: 0.85,
  avg_recipe_recall: 0.80,
  regions_covered: 6,
  sources_monitored: 48,
};

// ─── Crawl Activity ─────────────────────────────────────────────

export const CRAWL_ACTIVITY = Array.from({ length: 24 }, (_, i) => ({
  hour: `${String(i).padStart(2, "0")}:00`,
  pages: Math.floor(seededRandom(i * 3 + 300) * 120) + 30,
  success: Math.floor(seededRandom(i * 3 + 301) * 110) + 30,
  errors: Math.floor(seededRandom(i * 3 + 302) * 12),
}));
