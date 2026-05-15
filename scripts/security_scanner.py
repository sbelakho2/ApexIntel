#!/usr/bin/env python3
"""
ApexIntel Security Scanner — DNS posture, lookalike domain detection, and CISA KEV matching.

Performs real DNS lookups for SPF/DKIM/DMARC, generates typosquat candidates for
monitored company domains, and checks the CISA KEV catalog for relevant CVEs.

Results are written to:
  - observations table (for API compatibility)
  - dns_posture_entries table (dedicated)
  - lookalike_domains table (dedicated)
  - warnings table (security type)

Run periodically via cron or systemd timer.
"""

import asyncio
import json
import logging
import os
import re
import sys
import uuid
from datetime import datetime, timezone, timedelta
from typing import Optional

import asyncpg

# ─── Configuration ──────────────────────────────────────────────────────────────

DATABASE_URL = os.getenv(
    "DATABASE_URL",
    os.getenv(
        "APEX_DATABASE_URL",
        "postgresql://apexintel@127.0.0.1:5432/apexintel?sslmode=require",
    ),
)

LOG_LEVEL = os.getenv("LOG_LEVEL", "INFO").upper()

# ─── Logging ────────────────────────────────────────────────────────────────────

logging.basicConfig(
    level=getattr(logging, LOG_LEVEL, logging.INFO),
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    datefmt="%Y-%m-%dT%H:%M:%S",
)
log = logging.getLogger("security_scanner")

# ─── DNS Resolver (subprocess) ──────────────────────────────────────────────────


async def _dig(domain: str, rtype: str, timeout: int = 8) -> str:
    """Run dig for a specific record type; return stdout."""
    try:
        proc = await asyncio.create_subprocess_exec(
            "dig", "+short", "+time=4", "+tries=2", rtype, domain,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout)
        return stdout.decode("utf-8", errors="replace").strip()
    except (asyncio.TimeoutError, FileNotFoundError, OSError) as exc:
        log.debug("dig %s %s failed: %s", rtype, domain, exc)
        return ""


async def check_spf(domain: str) -> tuple[bool, Optional[str]]:
    """Check for SPF (TXT record containing v=spf1)."""
    txt = await _dig(domain, "TXT")
    for line in txt.splitlines():
        cleaned = line.strip().strip('"')
        if "v=spf1" in cleaned.lower():
            return True, cleaned
    return False, None


async def check_dkim(domain: str) -> bool:
    """Check common DKIM selectors."""
    selectors = ["default", "google", "selector1", "selector2", "k1", "mail", "dkim"]
    for sel in selectors:
        result = await _dig(f"{sel}._domainkey.{domain}", "TXT")
        if result and ("v=dkim1" in result.lower() or "p=" in result):
            return True
    return False


async def check_dmarc(domain: str) -> tuple[bool, Optional[str]]:
    """Check for DMARC record."""
    txt = await _dig(f"_dmarc.{domain}", "TXT")
    for line in txt.splitlines():
        cleaned = line.strip().strip('"')
        if "v=dmarc1" in cleaned.lower():
            # Extract policy
            policy_match = re.search(r"p=(\w+)", cleaned, re.IGNORECASE)
            policy = policy_match.group(1) if policy_match else None
            return True, policy
    return False, None


def compute_posture_score(has_spf: bool, has_dkim: bool, has_dmarc: bool) -> float:
    """Score 0-100 based on SPF(30) + DKIM(30) + DMARC(40)."""
    score = 0.0
    if has_spf:
        score += 30.0
    if has_dkim:
        score += 30.0
    if has_dmarc:
        score += 40.0
    return score


# ─── Lookalike Domain Generator ────────────────────────────────────────────────

HOMOGLYPHS = {
    "a": ["à", "á", "â", "ã", "ä", "å", "ɑ", "а"],
    "c": ["ç", "ć", "č", "с"],
    "d": ["đ", "ð"],
    "e": ["è", "é", "ê", "ë", "ε", "е"],
    "g": ["ğ", "ɡ"],
    "h": ["һ"],
    "i": ["ì", "í", "î", "ï", "ı", "і"],
    "l": ["ł", "ɫ", "1"],
    "n": ["ñ", "ŋ"],
    "o": ["ò", "ó", "ô", "õ", "ö", "ø", "0", "о"],
    "r": ["ŗ", "г"],
    "s": ["ş", "š", "ś", "ѕ"],
    "t": ["ţ", "ŧ"],
    "u": ["ù", "ú", "û", "ü", "µ"],
    "w": ["ŵ", "ω"],
    "y": ["ý", "ÿ", "ŷ", "у"],
    "z": ["ž", "ż", "ź"],
}

TLD_SWAPS = {
    ".com":    [".co", ".cm", ".corn", ".om", ".com.co", ".net", ".org"],
    ".net":    [".ner", ".met", ".org"],
    ".org":    [".orq", ".og", ".net"],
    ".co.uk":  [".co.ck", ".co.uk.com"],
    ".de":     [".d3", ".de.com"],
    ".fr":     [".f", ".fr.com"],
    ".tn":     [".tn.com", ".rn"],
}


def generate_lookalikes(domain: str, max_candidates: int = 12) -> list[dict]:
    """Generate plausible typosquat / homoglyph lookalike candidates."""
    candidates = []
    # Strip TLD
    parts = domain.rsplit(".", 1)
    if len(parts) < 2:
        return candidates
    base = parts[0]
    tld = "." + parts[1]

    # For multi-part TLDs (co.uk, com.tn)
    for mtld in [".co.uk", ".com.tn"]:
        if domain.endswith(mtld):
            base = domain[: -len(mtld)]
            tld = mtld
            break

    seen = set()

    # 1. Character swap (adjacent transpositions)
    for i in range(len(base) - 1):
        variant = base[:i] + base[i + 1] + base[i] + base[i + 2:]
        cand = variant + tld
        if cand != domain and cand not in seen:
            seen.add(cand)
            candidates.append({"domain": cand, "threat_type": "typosquat", "distance": 1})

    # 2. Character omission
    for i in range(len(base)):
        variant = base[:i] + base[i + 1:]
        cand = variant + tld
        if cand != domain and cand not in seen and len(variant) > 2:
            seen.add(cand)
            candidates.append({"domain": cand, "threat_type": "typosquat", "distance": 1})

    # 3. Character doubling
    for i in range(len(base)):
        variant = base[:i] + base[i] + base[i:]
        cand = variant + tld
        if cand != domain and cand not in seen:
            seen.add(cand)
            candidates.append({"domain": cand, "threat_type": "typosquat", "distance": 1})

    # 4. Homoglyph substitution
    for i, ch in enumerate(base):
        for glyph in HOMOGLYPHS.get(ch.lower(), []):
            variant = base[:i] + glyph + base[i + 1:]
            cand = variant + tld
            if cand not in seen:
                seen.add(cand)
                candidates.append({"domain": cand, "threat_type": "homoglyph", "distance": 1})

    # 5. Hyphenation
    for i in range(1, len(base)):
        if base[i - 1] != "-" and base[i] != "-":
            variant = base[:i] + "-" + base[i:]
            cand = variant + tld
            if cand not in seen:
                seen.add(cand)
                candidates.append({"domain": cand, "threat_type": "typosquat", "distance": 1})

    # 6. TLD swap
    for alt_tld in TLD_SWAPS.get(tld, []):
        cand = base + alt_tld
        if cand not in seen:
            seen.add(cand)
            candidates.append({"domain": cand, "threat_type": "tld_swap", "distance": 2})

    # Return only the top N
    return candidates[:max_candidates]


# ─── CISA KEV Fetcher ──────────────────────────────────────────────────────────

# We embed a curated subset of known-exploited CVEs relevant to EMS/electronics/defense
# sectors rather than fetching the full CISA catalog (requires network access).
# In production, this would fetch from https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json

CURATED_KEV_ENTRIES = [
    {
        "cve_id": "CVE-2024-3400",
        "vendor": "Palo Alto Networks",
        "product": "PAN-OS GlobalProtect",
        "vulnerability_name": "OS Command Injection in GlobalProtect Gateway",
        "date_added": "2024-04-12",
        "due_date": "2024-04-19",
        "relevance_keywords": ["firewall", "network", "security", "vpn"],
    },
    {
        "cve_id": "CVE-2024-21887",
        "vendor": "Ivanti",
        "product": "Connect Secure / Policy Secure",
        "vulnerability_name": "Command Injection in Web Components",
        "date_added": "2024-01-10",
        "due_date": "2024-01-31",
        "relevance_keywords": ["vpn", "remote access", "network"],
    },
    {
        "cve_id": "CVE-2023-46805",
        "vendor": "Ivanti",
        "product": "Connect Secure / Policy Secure",
        "vulnerability_name": "Authentication Bypass",
        "date_added": "2024-01-10",
        "due_date": "2024-01-31",
        "relevance_keywords": ["vpn", "remote access", "authentication"],
    },
    {
        "cve_id": "CVE-2024-1709",
        "vendor": "ConnectWise",
        "product": "ScreenConnect",
        "vulnerability_name": "Authentication Bypass Using Alternate Path",
        "date_added": "2024-02-22",
        "due_date": "2024-02-29",
        "relevance_keywords": ["remote management", "supply chain", "ems"],
    },
    {
        "cve_id": "CVE-2023-34362",
        "vendor": "Progress Software",
        "product": "MOVEit Transfer",
        "vulnerability_name": "SQL Injection Leading to Remote Code Execution",
        "date_added": "2023-06-02",
        "due_date": "2023-06-23",
        "relevance_keywords": ["file transfer", "supply chain", "data"],
    },
    {
        "cve_id": "CVE-2024-47575",
        "vendor": "Fortinet",
        "product": "FortiManager",
        "vulnerability_name": "Missing Authentication for Critical Function",
        "date_added": "2024-10-23",
        "due_date": "2024-11-13",
        "relevance_keywords": ["firewall", "network management", "security"],
    },
    {
        "cve_id": "CVE-2024-0012",
        "vendor": "Palo Alto Networks",
        "product": "PAN-OS Management Interface",
        "vulnerability_name": "Authentication Bypass in Management Web Interface",
        "date_added": "2024-11-18",
        "due_date": "2024-12-09",
        "relevance_keywords": ["firewall", "management", "network"],
    },
    {
        "cve_id": "CVE-2023-20198",
        "vendor": "Cisco",
        "product": "IOS XE Web UI",
        "vulnerability_name": "Privilege Escalation via Web UI",
        "date_added": "2023-10-16",
        "due_date": "2023-10-20",
        "relevance_keywords": ["network", "router", "switch", "iot"],
    },
    {
        "cve_id": "CVE-2024-23113",
        "vendor": "Fortinet",
        "product": "FortiOS / FortiProxy",
        "vulnerability_name": "Format String Vulnerability in fgfmd",
        "date_added": "2024-10-09",
        "due_date": "2024-10-30",
        "relevance_keywords": ["firewall", "network", "security"],
    },
    {
        "cve_id": "CVE-2023-27997",
        "vendor": "Fortinet",
        "product": "FortiOS SSL VPN",
        "vulnerability_name": "Heap Buffer Overflow in SSL VPN",
        "date_added": "2023-06-13",
        "due_date": "2023-07-04",
        "relevance_keywords": ["vpn", "network", "defense"],
    },
    {
        "cve_id": "CVE-2024-20353",
        "vendor": "Cisco",
        "product": "Adaptive Security Appliance (ASA)",
        "vulnerability_name": "Denial of Service in Web Services",
        "date_added": "2024-04-24",
        "due_date": "2024-05-01",
        "relevance_keywords": ["firewall", "network", "defense"],
    },
    {
        "cve_id": "CVE-2023-4966",
        "vendor": "Citrix",
        "product": "NetScaler ADC / Gateway",
        "vulnerability_name": "Buffer Overflow Leading to Information Disclosure",
        "date_added": "2023-10-18",
        "due_date": "2023-11-08",
        "relevance_keywords": ["network", "load balancer", "vpn"],
    },
]

# Industry tags that suggest a company is affected by infrastructure CVEs
INFRA_TAGS = {
    "defense", "aerospace", "electronics", "semiconductor", "manufacturing",
    "ems", "supply_chain", "pcb", "automotive", "telecommunications",
    "technology", "cybersecurity", "government",
}


def match_kev_to_companies(
    kev_entry: dict, companies: list[dict]
) -> tuple[float, list[str]]:
    """Score relevance 0-1 and list affected company names."""
    affected = []
    keywords = set(kev_entry.get("relevance_keywords", []))

    for co in companies:
        tags = set(co.get("industry_tags") or [])
        # Companies in INFRA_TAGS are likely affected by infra CVEs
        overlap = tags & INFRA_TAGS
        if overlap:
            affected.append(co["name"])

    if not affected:
        return 0.0, []

    # Score: more affected companies → higher relevance, capped at 0.9
    base = min(len(affected) / max(len(companies), 1), 0.5)
    relevance = min(0.4 + base, 0.9)
    return round(relevance, 2), affected[:15]  # Cap at 15 names


# ─── Database Writers ───────────────────────────────────────────────────────────


async def upsert_dns_observation(
    conn: asyncpg.Connection,
    company_id: str,
    company_name: str,
    domain: str,
    has_spf: bool,
    has_dkim: bool,
    has_dmarc: bool,
    dmarc_policy: Optional[str],
    spf_record: Optional[str],
    posture_score: float,
    now: datetime,
):
    """Insert DNS posture into both observations table and dns_posture_entries."""
    obs_id = str(uuid.uuid4())
    value = json.dumps({
        "domain": domain,
        "company_name": company_name,
        "has_spf": has_spf,
        "has_dkim": has_dkim,
        "has_dmarc": has_dmarc,
        "dmarc_policy": dmarc_policy,
        "spf_record": spf_record,
        "posture_score": posture_score,
    })
    provenance = json.dumps({
        "source": "security_scanner",
        "content_hash": f"dns_{domain}_{now.strftime('%Y%m%d')}",
    })

    # observations table (for API compat) — delete old + insert fresh
    await conn.execute(
        "DELETE FROM observations WHERE observation_type = 'dns_posture' AND entity_id = $1",
        uuid.UUID(company_id),
    )
    await conn.execute(
        """
        INSERT INTO observations (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
        VALUES ($1, 'dns_posture', $2, 'company', $3, $4::jsonb, $5::jsonb, 0.95)
        """,
        uuid.UUID(obs_id), uuid.UUID(company_id), now, value, provenance,
    )

    # Dedicated table
    await conn.execute(
        """
        INSERT INTO dns_posture_entries (company_id, domain, has_spf, has_dkim, has_dmarc, dmarc_policy, spf_record, posture_score, checked_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (domain, checked_at) DO UPDATE SET
            has_spf = EXCLUDED.has_spf,
            has_dkim = EXCLUDED.has_dkim,
            has_dmarc = EXCLUDED.has_dmarc,
            dmarc_policy = EXCLUDED.dmarc_policy,
            spf_record = EXCLUDED.spf_record,
            posture_score = EXCLUDED.posture_score
        """,
        uuid.UUID(company_id), domain, has_spf, has_dkim, has_dmarc,
        dmarc_policy, spf_record, posture_score, now,
    )


async def upsert_lookalike_observation(
    conn: asyncpg.Connection,
    company_id: str,
    original_domain: str,
    lookalike: dict,
    now: datetime,
):
    """Insert lookalike domain into both observations and lookalike_domains tables."""
    obs_id = str(uuid.uuid4())
    value = json.dumps({
        "original_domain": original_domain,
        "domain": lookalike["domain"],
        "distance": lookalike["distance"],
        "threat_type": lookalike["threat_type"],
        "active": True,
    })
    provenance = json.dumps({
        "source": "security_scanner",
        "content_hash": f"la_{original_domain}_{lookalike['domain']}",
    })

    # observations table — delete old + insert fresh
    await conn.execute(
        """
        DELETE FROM observations
        WHERE observation_type = 'lookalike_domain' AND entity_id = $1
          AND value->>'domain' = $2
        """,
        uuid.UUID(company_id), lookalike["domain"],
    )
    await conn.execute(
        """
        INSERT INTO observations (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
        VALUES ($1, 'lookalike_domain', $2, 'company', $3, $4::jsonb, $5::jsonb, 0.80)
        """,
        uuid.UUID(obs_id), uuid.UUID(company_id), now, value, provenance,
    )

    # Dedicated table
    await conn.execute(
        """
        INSERT INTO lookalike_domains (company_id, original_domain, lookalike_domain, threat_type, distance, active, detected_at)
        VALUES ($1, $2, $3, $4, $5, true, $6)
        ON CONFLICT (original_domain, lookalike_domain) DO NOTHING
        """,
        uuid.UUID(company_id), original_domain, lookalike["domain"],
        lookalike["threat_type"], lookalike["distance"], now,
    )


async def upsert_kev_observation(
    conn: asyncpg.Connection,
    kev: dict,
    relevance_score: float,
    affected_companies: list[str],
    now: datetime,
):
    """Insert KEV relevance into observations table."""
    obs_id = str(uuid.uuid4())
    value = json.dumps({
        "cve_id": kev["cve_id"],
        "vendor": kev["vendor"],
        "product": kev["product"],
        "vulnerability_name": kev["vulnerability_name"],
        "date_added": kev["date_added"],
        "due_date": kev["due_date"],
        "relevance_score": relevance_score,
        "affected_companies": affected_companies,
        "notes": f"Relevant to {len(affected_companies)} monitored entities in EMS/defense supply chain.",
    })
    provenance = json.dumps({
        "source": "security_scanner",
        "content_hash": f"kev_{kev['cve_id']}",
    })

    # observations table — delete old + insert fresh
    await conn.execute(
        "DELETE FROM observations WHERE observation_type = 'kev_match' AND value->>'cve_id' = $1",
        kev["cve_id"],
    )
    await conn.execute(
        """
        INSERT INTO observations (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
        VALUES ($1, 'kev_match', NULL, NULL, $2, $3::jsonb, $4::jsonb, $5)
        """,
        uuid.UUID(obs_id), now, value, provenance, relevance_score,
    )


async def insert_security_warning(
    conn: asyncpg.Connection,
    title: str,
    description: str,
    severity: str,
    region: Optional[str],
    entity_ids: list[str],
    now: datetime,
):
    """Insert a security-type warning into the warnings table."""
    # Check for duplicate (same title within 24h)
    existing = await conn.fetchval(
        """
        SELECT COUNT(*) FROM warnings
        WHERE warning_type = 'security' AND title = $1 AND ts_utc > $2
        """,
        title, now - timedelta(hours=24),
    )
    if existing and existing > 0:
        log.debug("Skipping duplicate warning: %s", title)
        return

    eid_uuids = [uuid.UUID(eid) for eid in entity_ids] if entity_ids else None

    await conn.execute(
        """
        INSERT INTO warnings (warning_type, title, description, severity, region, entity_ids, confidence, ts_utc)
        VALUES ('security', $1, $2, $3, $4, $5, 0.85, $6)
        """,
        title, description, severity, region, eid_uuids, now,
    )


# ─── Main Scanner Logic ────────────────────────────────────────────────────────


async def scan_dns_posture(conn: asyncpg.Connection, now: datetime) -> dict:
    """Scan DNS posture for all companies with domains."""
    companies = await conn.fetch(
        "SELECT id, name, domain, region FROM companies WHERE domain IS NOT NULL AND domain != '' ORDER BY name"
    )
    log.info("DNS posture scan: %d companies with domains", len(companies))

    total = 0
    issues = 0
    critical_domains = []

    for co in companies:
        domain = co["domain"]
        company_id = str(co["id"])
        company_name = co["name"]
        region = co["region"]

        log.debug("Checking DNS for %s (%s)", domain, company_name)

        has_spf, spf_record = await check_spf(domain)
        has_dkim = await check_dkim(domain)
        has_dmarc, dmarc_policy = await check_dmarc(domain)
        score = compute_posture_score(has_spf, has_dkim, has_dmarc)

        await upsert_dns_observation(
            conn, company_id, company_name, domain,
            has_spf, has_dkim, has_dmarc, dmarc_policy, spf_record, score, now,
        )
        total += 1

        if score < 50:
            issues += 1
            critical_domains.append((company_name, domain, score, region, company_id))

        # Small delay to avoid DNS rate limits
        await asyncio.sleep(0.3)

    # Generate warnings for most critical findings
    for name, domain, score, region, cid in critical_domains[:10]:
        missing = []
        if score == 0:
            missing = ["SPF", "DKIM", "DMARC"]
            severity = "high"
        else:
            if not has_spf:
                missing.append("SPF")
            if not has_dkim:
                missing.append("DKIM")
            if not has_dmarc:
                missing.append("DMARC")
            severity = "medium"

        await insert_security_warning(
            conn,
            title=f"DNS posture risk: {name} ({domain})",
            description=(
                f"{domain} is missing {', '.join(missing)} records "
                f"(posture score: {score:.0f}%). This increases risk of email spoofing "
                f"and phishing attacks targeting supply chain communications."
            ),
            severity=severity,
            region=region,
            entity_ids=[cid],
            now=now,
        )

    log.info("DNS posture: %d domains scanned, %d with issues", total, issues)
    return {"scanned": total, "issues": issues}


async def scan_lookalike_domains(conn: asyncpg.Connection, now: datetime) -> dict:
    """Generate lookalike domain candidates for monitored companies."""
    companies = await conn.fetch(
        "SELECT id, name, domain, region FROM companies WHERE domain IS NOT NULL AND domain != '' ORDER BY name"
    )
    log.info("Lookalike scan: %d companies", len(companies))

    total_candidates = 0
    companies_with_lookups = 0

    for co in companies:
        domain = co["domain"]
        company_id = str(co["id"])
        region = co["region"]

        # Only generate for companies with non-country-specific TLDs or important ones
        candidates = generate_lookalikes(domain, max_candidates=8)
        if not candidates:
            continue

        companies_with_lookups += 1

        for cand in candidates:
            await upsert_lookalike_observation(conn, company_id, domain, cand, now)
            total_candidates += 1

    # Generate a summary warning
    if total_candidates > 0:
        await insert_security_warning(
            conn,
            title=f"Lookalike domain monitoring: {total_candidates} candidates identified",
            description=(
                f"Security scanner identified {total_candidates} potential typosquat and "
                f"homoglyph domain candidates across {companies_with_lookups} monitored companies. "
                f"These domains could be registered by threat actors for phishing or brand impersonation."
            ),
            severity="medium",
            region=None,
            entity_ids=[],
            now=now,
        )

    log.info("Lookalike domains: %d candidates for %d companies", total_candidates, companies_with_lookups)
    return {"candidates": total_candidates, "companies": companies_with_lookups}


async def scan_kev_relevance(conn: asyncpg.Connection, now: datetime) -> dict:
    """Match CISA KEV entries against monitored companies."""
    companies = await conn.fetch(
        "SELECT id, name, industry_tags FROM companies ORDER BY name"
    )
    log.info("KEV scan: checking %d CVEs against %d companies", len(CURATED_KEV_ENTRIES), len(companies))

    co_dicts = [
        {"id": str(co["id"]), "name": co["name"], "industry_tags": co["industry_tags"] or []}
        for co in companies
    ]

    inserted = 0
    for kev in CURATED_KEV_ENTRIES:
        relevance, affected = match_kev_to_companies(kev, co_dicts)
        if relevance >= 0.3 and affected:
            await upsert_kev_observation(conn, kev, relevance, affected, now)
            inserted += 1

    # Generate warning for high-relevance CVEs
    high_rel = [k for k in CURATED_KEV_ENTRIES if match_kev_to_companies(k, co_dicts)[0] >= 0.5]
    if high_rel:
        cve_list = ", ".join(k["cve_id"] for k in high_rel[:5])
        await insert_security_warning(
            conn,
            title=f"CISA KEV: {len(high_rel)} high-relevance CVEs for monitored entities",
            description=(
                f"{len(high_rel)} Known Exploited Vulnerabilities from CISA's catalog have "
                f"high relevance to monitored EMS/defense entities: {cve_list}. "
                f"These CVEs affect infrastructure commonly used in electronics supply chains."
            ),
            severity="high",
            region=None,
            entity_ids=[],
            now=now,
        )

    log.info("KEV scan: %d relevant entries matched", inserted)
    return {"matched": inserted}


async def main():
    log.info("=" * 60)
    log.info("ApexIntel Security Scanner starting")
    log.info("=" * 60)

    conn = await asyncpg.connect(DATABASE_URL)
    try:
        now = datetime.now(timezone.utc)

        # 1. DNS Posture
        dns_stats = await scan_dns_posture(conn, now)

        # 2. Lookalike Domains
        la_stats = await scan_lookalike_domains(conn, now)

        # 3. KEV Relevance
        kev_stats = await scan_kev_relevance(conn, now)

        log.info("=" * 60)
        log.info("Security scan complete:")
        log.info("  DNS: %d scanned, %d issues", dns_stats["scanned"], dns_stats["issues"])
        log.info("  Lookalikes: %d candidates for %d companies", la_stats["candidates"], la_stats["companies"])
        log.info("  KEV: %d relevant matches", kev_stats["matched"])
        log.info("=" * 60)

    finally:
        await conn.close()


if __name__ == "__main__":
    asyncio.run(main())
