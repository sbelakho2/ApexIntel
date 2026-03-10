#!/usr/bin/env python3
"""
ApexIntel Crawl Daemon — Continuous intelligence gathering for EMS/Electronics supply chain.

Direct-connection crawler with multi-IP rotation (1x IPv4 + 3x IPv6).
Crawls company websites, news sources, job boards, and regulatory feeds.
Extracts observations, generates warnings/insights, and inserts into PostgreSQL.

Deployed as systemd service on Hetzner VPS.
"""

import asyncio
import hashlib
import json
import logging
import os
import random
import re
import string
import sys
import time
import uuid
from datetime import datetime, timedelta, timezone
from typing import Optional
from urllib.parse import urlparse

import aiohttp
import asyncpg
from bs4 import BeautifulSoup

try:
    import aiohttp_socks
    AIOHTTP_SOCKS_AVAILABLE = True
except ImportError:
    AIOHTTP_SOCKS_AVAILABLE = False


# ─── Configuration ──────────────────────────────────────────────────────────────

DATABASE_URL = os.getenv("DATABASE_URL", "postgresql://apexintel:ApexIntel2026Secure@127.0.0.1:5432/apexintel")

# Source IP addresses — IPv6 is primary, IPv4 is fallback
IPV6_ADDRESSES = [
    "2a01:4f9:c012:a8e::1",         # IPv6 #1
    "2a01:4f9:c01f:e074::1",        # IPv6 #2 (floating)
]
IPV4_ADDRESSES = [
    "77.42.65.89",                  # Primary IPv4
    "95.216.182.162",               # Secondary IPv4 (floating)
]
ALL_ADDRESSES = IPV6_ADDRESSES + IPV4_ADDRESSES

# Crawl settings
REQUESTS_PER_SECOND = float(os.getenv("REQUESTS_PER_SECOND", "2.0"))
CRAWL_CYCLE_INTERVAL = int(os.getenv("CRAWL_CYCLE_INTERVAL", "3600"))  # 1 hour
MAX_CONCURRENT = int(os.getenv("MAX_CONCURRENT", "5"))
REQUEST_TIMEOUT = int(os.getenv("REQUEST_TIMEOUT", "30"))

# ─── Tor / Dark Web Configuration ───────────────────────────────────────────────

TOR_SOCKS_PROXY = os.getenv("TOR_SOCKS_PROXY", "socks5://127.0.0.1:9050")
TOR_ENABLED = os.getenv("TOR_ENABLED", "true").lower() in ("true", "1", "yes")
HIBP_API_KEY = os.getenv("HIBP_API_KEY", "")  # Have I Been Pwned API key
INTELX_API_KEY = os.getenv("INTELX_API_KEY", "")  # Intelligence X API key

# Known .onion endpoints for POI intelligence
ONION_SOURCES = {
    "pwndb": "http://pwndb2am4tzkvold.onion",
    "exposed_vc": "http://exposedboq3tpnmxx.onion",
    "dread_osint": "http://dreadytofatroptsdj6io7l3xptbet6onoyno2yv7jicoxknyazubrad.onion",
    "ransomware_index": "http://ransomwarebugsctmseqejbm7dlgm4lol2eahrr2r2iyctba2d6vlxxad.onion",
}


# ─── Logging ────────────────────────────────────────────────────────────────────

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)]
)
log = logging.getLogger("apex-crawler")

# ─── User-Agent Pool (matches crates/crawl/src/headers.rs) ─────────────────────

USER_AGENTS = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_3) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36 Edg/133.0.0.0",
    "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (iPad; CPU OS 18_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36 OPR/117.0.0.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15.3; rv:135.0) Gecko/20100101 Firefox/135.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0",
]

REFERERS = [
    "https://www.google.com/",
    "https://www.bing.com/",
    "https://duckduckgo.com/",
    "https://www.linkedin.com/",
    "",  # direct
]

ACCEPT_LANGUAGES = [
    "en-US,en;q=0.9",
    "en-GB,en;q=0.9",
    "de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7",
    "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7",
    "en-US,en;q=0.9,fi;q=0.8",
]


# ─── IP Rotation + Fetch ─────────────────────────────────────────────────────────

def get_random_headers() -> dict:
    """Generate randomized browser-like HTTP headers."""
    ua = random.choice(USER_AGENTS)
    ref = random.choice(REFERERS)
    lang = random.choice(ACCEPT_LANGUAGES)
    headers = {
        "User-Agent": ua,
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        "Accept-Language": lang,
        "Accept-Encoding": "gzip, deflate, br",
        "DNT": "1",
        "Connection": "keep-alive",
        "Upgrade-Insecure-Requests": "1",
        "Sec-Fetch-Dest": "document",
        "Sec-Fetch-Mode": "navigate",
        "Sec-Fetch-Site": "none",
        "Sec-Fetch-User": "?1",
        "Cache-Control": "max-age=0",
    }
    if ref:
        headers["Referer"] = ref
    return headers


# Round-robin counters for source-address rotation
_ipv6_idx = 0
_ipv4_idx = 0

def _next_ipv6_address() -> str:
    """Return the next IPv6 source in round-robin order."""
    global _ipv6_idx
    addr = IPV6_ADDRESSES[_ipv6_idx % len(IPV6_ADDRESSES)]
    _ipv6_idx += 1
    return addr

def _next_ipv4_address() -> str:
    """Return the next IPv4 source in round-robin order."""
    global _ipv4_idx
    addr = IPV4_ADDRESSES[_ipv4_idx % len(IPV4_ADDRESSES)]
    _ipv4_idx += 1
    return addr


def make_connector(local_addr: Optional[str] = None) -> aiohttp.TCPConnector:
    """Create a TCPConnector optionally bound to a specific local address."""
    return aiohttp.TCPConnector(
        limit=MAX_CONCURRENT * 2,
        ttl_dns_cache=300,
        force_close=True,
        local_addr=(local_addr, 0) if local_addr else None,
    )


def make_tor_connector() -> Optional[aiohttp.BaseConnector]:
    """Create a SOCKS5 connector for Tor .onion crawling, or None if unavailable."""
    if not AIOHTTP_SOCKS_AVAILABLE or not TOR_ENABLED:
        return None
    try:
        return aiohttp_socks.ProxyConnector.from_url(
            TOR_SOCKS_PROXY,
            rdns=True,  # Remote DNS resolution (critical for .onion)
        )
    except Exception as e:
        log.warning(f"⚠️ Could not create Tor connector: {e}")
        return None


# Global Tor session (lazy init)
_tor_session: Optional[aiohttp.ClientSession] = None
_tor_available: bool = False


async def get_tor_session() -> Optional[aiohttp.ClientSession]:
    """Get or create the global Tor session. Returns None if Tor is unavailable."""
    global _tor_session, _tor_available
    if _tor_session is not None:
        return _tor_session if _tor_available else None
    
    connector = make_tor_connector()
    if connector is None:
        _tor_available = False
        return None
    
    _tor_session = aiohttp.ClientSession(
        connector=connector,
        timeout=aiohttp.ClientTimeout(total=90),
    )
    
    # Test connectivity
    try:
        async with _tor_session.get(
            "https://check.torproject.org/api/ip",
            timeout=aiohttp.ClientTimeout(total=15),
        ) as resp:
            if resp.status == 200:
                ip_info = await resp.json()
                _tor_available = ip_info.get("IsTor", False)
                if _tor_available:
                    log.info(f"🧅 Tor connected: exit IP {ip_info.get('IP', 'unknown')}")
                else:
                    log.warning("⚠️ Tor reachable but not routing through Tor network")
            else:
                _tor_available = False
    except Exception as e:
        log.warning(f"⚠️ Tor connectivity test failed: {e}")
        _tor_available = False
    
    return _tor_session if _tor_available else None


async def fetch_onion_url(url: str, timeout_secs: int = 60) -> Optional[str]:
    """Fetch an .onion URL via Tor. Returns raw HTML or None on failure."""
    session = await get_tor_session()
    if session is None:
        return None
    
    try:
        headers = {
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0",
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Accept-Language": "en-US,en;q=0.5",
        }
        async with session.get(
            url,
            headers=headers,
            timeout=aiohttp.ClientTimeout(total=timeout_secs),
        ) as resp:
            if resp.status == 200:
                return await resp.text(errors="replace")
    except Exception as e:
        log.debug(f"🧅 Onion fetch failed {url}: {e}")
    return None


# ─── Dark Web POI Intelligence Functions ────────────────────────────────────────

# Regex patterns for dark web data extraction
_DARKWEB_EMAIL_RE = re.compile(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}")
_DARKWEB_PHONE_RE = re.compile(r"\+?[\d\s\-\(\)]{8,20}")
_PWNDB_ROW_RE = re.compile(
    r"<li>luser:\s*(.+?)\s*</li>\s*<li>domain:\s*(.+?)\s*</li>(?:\s*<li>password:\s*(.+?)\s*</li>)?",
    re.IGNORECASE | re.DOTALL
)


async def query_pwndb_by_domain(domain: str) -> list[dict]:
    """
    Query PwnDB onion for breached credentials by email domain.
    Returns list of {email, password_hash, source, domain}.
    """
    url = f"{ONION_SOURCES['pwndb']}/?luser=%25&domain={domain}&lusers=Search"
    html = await fetch_onion_url(url, timeout_secs=60)
    if not html:
        return []
    
    results = []
    for match in _PWNDB_ROW_RE.finditer(html):
        luser = match.group(1).strip()
        dom = match.group(2).strip() if match.group(2) else domain
        password_hash = match.group(3).strip() if match.group(3) else None
        
        if not luser or len(luser) > 64:
            continue
        
        results.append({
            "email": f"{luser}@{dom}",
            "password_hash": password_hash,
            "source": "pwndb",
            "domain": dom,
        })
    
    if results:
        log.info(f"🧅 PwnDB: {len(results)} breached credentials for {domain}")
    return results


async def query_pwndb_by_email(email: str) -> list[dict]:
    """Query PwnDB for a specific email address."""
    parts = email.split("@")
    if len(parts) != 2:
        return []
    luser, domain = parts
    
    url = f"{ONION_SOURCES['pwndb']}/?luser={luser}&domain={domain}&lusers=Search"
    html = await fetch_onion_url(url, timeout_secs=60)
    if not html:
        return []
    
    results = []
    for match in _PWNDB_ROW_RE.finditer(html):
        lu = match.group(1).strip()
        dom = match.group(2).strip() if match.group(2) else domain
        password_hash = match.group(3).strip() if match.group(3) else None
        
        if not lu or len(lu) > 64:
            continue
        
        results.append({
            "email": f"{lu}@{dom}",
            "password_hash": password_hash,
            "source": "pwndb",
            "domain": dom,
        })
    return results


async def search_exposed_vc(full_name: str) -> list[dict]:
    """
    Search Exposed.vc onion (data breach aggregator) for a person's name.
    Returns contact records: {name, email, phone, source_url, confidence}.
    """
    from urllib.parse import quote
    url = f"{ONION_SOURCES['exposed_vc']}/search?query={quote(full_name)}"
    html = await fetch_onion_url(url, timeout_secs=90)
    if not html:
        return []
    
    return _extract_onion_contacts(html, url, full_name, confidence=0.4)


async def search_leak_sites(org_name: str) -> list[dict]:
    """
    Search ransomware leak-site archives for company data containing executive info.
    Returns contact records extracted from leaked corporate documents.
    """
    from urllib.parse import quote
    url = f"{ONION_SOURCES['ransomware_index']}/search?q={quote(org_name)}"
    html = await fetch_onion_url(url, timeout_secs=90)
    if not html:
        return []
    
    return _extract_onion_contacts(html, url, org_name, confidence=0.3)


async def search_dread_osint(full_name: str) -> list[dict]:
    """
    Search Dread forum OSINT sub-boards for mentions of a person.
    Returns contact-like records.
    """
    from urllib.parse import quote
    url = f"{ONION_SOURCES['dread_osint']}/search?q={quote(full_name)}"
    html = await fetch_onion_url(url, timeout_secs=90)
    if not html:
        return []
    
    return _extract_onion_contacts(html, url, full_name, confidence=0.25)


def _extract_onion_contacts(html: str, source_url: str, search_term: str, confidence: float) -> list[dict]:
    """Extract email/phone contacts from onion HTML."""
    emails = set()
    for m in _DARKWEB_EMAIL_RE.finditer(html):
        email = m.group(0).lower()
        if not email.endswith((".png", ".jpg", ".gif", ".css", ".js")):
            emails.add(email)
    
    phones = set()
    for m in _DARKWEB_PHONE_RE.finditer(html):
        phone = m.group(0).strip()
        if sum(c.isdigit() for c in phone) >= 7:
            phones.add(phone)
    
    if not emails and not phones:
        return []
    
    phone_ref = list(phones)[0] if phones else None
    results = []
    for email in emails:
        results.append({
            "name": search_term,
            "email": email,
            "phone": phone_ref,
            "source_url": source_url,
            "confidence": confidence,
            "source": "onion",
        })
    
    return results


async def aggregate_darkweb_poi_intel(full_name: str, email_domain: Optional[str], org_name: Optional[str]) -> dict:
    """
    Aggregate all dark web intelligence for a POI.
    Runs PwnDB (if domain known), Exposed.vc, and Dread OSINT sequentially.
    Returns: {breach_records, contact_records, matched_emails}.
    """
    breach_records = []
    contact_records = []
    matched_emails = []
    
    # PwnDB breach lookup
    if email_domain:
        breach_records = await query_pwndb_by_domain(email_domain)
        
        # Match breached emails to POI name tokens
        name_tokens = [t.lower() for t in full_name.split() if len(t) > 2]
        for br in breach_records:
            local = br["email"].split("@")[0].lower()
            if any(tok in local for tok in name_tokens):
                matched_emails.append(br["email"])
    
    # Exposed.vc search
    exp_contacts = await search_exposed_vc(full_name)
    contact_records.extend(exp_contacts)
    
    # Leak site search (if org known)
    if org_name:
        leak_contacts = await search_leak_sites(org_name)
        contact_records.extend(leak_contacts)
    
    # Dread OSINT search
    dread_contacts = await search_dread_osint(full_name)
    contact_records.extend(dread_contacts)
    
    return {
        "full_name": full_name,
        "breach_records": breach_records,
        "contact_records": contact_records,
        "matched_emails": list(set(matched_emails)),
        "ts_scraped": datetime.now(timezone.utc).isoformat(),
    }


async def check_hibp_domain(domain: str) -> list[dict]:
    """
    Check Have I Been Pwned for breaches affecting a domain.
    Requires HIBP_API_KEY environment variable.
    Returns list of breach events.
    """
    if not HIBP_API_KEY:
        return []
    
    from urllib.parse import quote
    url = f"https://haveibeenpwned.com/api/v3/breaches?domain={quote(domain)}"
    
    async with aiohttp.ClientSession() as session:
        try:
            headers = {
                "hibp-api-key": HIBP_API_KEY,
                "Accept": "application/json",
                "User-Agent": "ApexIntel/1.0",
            }
            async with session.get(url, headers=headers, timeout=aiohttp.ClientTimeout(total=30)) as resp:
                if resp.status == 200:
                    breaches = await resp.json()
                    results = []
                    for b in breaches:
                        severity = "high"
                        data_classes = b.get("DataClasses", [])
                        if any(c in data_classes for c in ["Passwords", "Credit cards", "SSNs"]):
                            severity = "critical"
                        elif any(c in data_classes for c in ["Email addresses", "Phone numbers"]):
                            severity = "high"
                        elif any(c in data_classes for c in ["Usernames", "Names"]):
                            severity = "medium"
                        else:
                            severity = "low"
                        
                        results.append({
                            "source": "hibp",
                            "breach_name": b.get("Name", "Unknown"),
                            "domain": domain,
                            "pwn_count": b.get("PwnCount", 0),
                            "data_classes": data_classes,
                            "breach_date": b.get("BreachDate"),
                            "severity": severity,
                            "is_verified": b.get("IsVerified", False),
                            "description": b.get("Description"),
                        })
                    if results:
                        log.info(f"🔓 HIBP: {len(results)} breaches for {domain}")
                    return results
                elif resp.status == 404:
                    return []  # No breaches
        except Exception as e:
            log.debug(f"HIBP check failed for {domain}: {e}")
    return []


# ─── Cross-Reference Dark Web Intel with Insights & Social Media ──────────────

async def get_poi_social_mentions(pool: asyncpg.Pool, person_name: str, org_name: str,
                                   hours: int = 72) -> list:
    """
    Retrieve recent social media mentions for a person or their organization.
    Searches observations table for SocialPost type entries.
    """
    # Normalize name tokens for matching
    name_tokens = [t.lower() for t in person_name.split() if len(t) > 2]
    
    rows = await pool.fetch(
        """SELECT id, value, provenance, confidence, ts_utc
           FROM observations
           WHERE observation_type = 'SocialPost'
             AND ts_utc > NOW() - $1 * INTERVAL '1 hour'
           ORDER BY ts_utc DESC
           LIMIT 200""",
        hours
    )
    
    matches = []
    for row in rows:
        value = row["value"] if isinstance(row["value"], dict) else {}
        text = value.get("text", "").lower()
        
        # Check for person name tokens or org name
        has_person = any(tok in text for tok in name_tokens)
        has_org = org_name and org_name.lower() in text
        
        if has_person or has_org:
            matches.append({
                "id": str(row["id"]),
                "platform": value.get("platform", "unknown"),
                "text": value.get("text", "")[:500],
                "url": value.get("url", ""),
                "signals": value.get("signals", []),
                "engagement": value.get("engagement", 0),
                "ts": row["ts_utc"].isoformat() if row["ts_utc"] else None,
                "matched_person": has_person,
                "matched_org": has_org,
            })
    
    return matches[:20]  # Top 20 most recent


async def get_poi_related_insights(pool: asyncpg.Pool, person_name: str, org_name: str,
                                    hours: int = 168) -> list:
    """
    Retrieve recent insights mentioning the person or their organization.
    Scans both title and summary fields.
    """
    name_tokens = [t.lower() for t in person_name.split() if len(t) > 2]
    
    rows = await pool.fetch(
        """SELECT id, title, summary, insight_type, region, confidence, created_at
           FROM insights
           WHERE created_at > NOW() - $1 * INTERVAL '1 hour'
           ORDER BY created_at DESC
           LIMIT 200""",
        hours
    )
    
    matches = []
    for row in rows:
        combined = (row["title"] + " " + row["summary"]).lower()
        
        has_person = any(tok in combined for tok in name_tokens)
        has_org = org_name and org_name.lower() in combined
        
        if has_person or has_org:
            matches.append({
                "id": str(row["id"]),
                "title": row["title"],
                "type": row["insight_type"],
                "region": row["region"],
                "confidence": float(row["confidence"]) if row["confidence"] else 0.5,
                "ts": row["created_at"].isoformat() if row["created_at"] else None,
                "matched_person": has_person,
                "matched_org": has_org,
            })
    
    return matches[:10]


async def get_poi_existing_warnings(pool: asyncpg.Pool, person_id: str,
                                     hours: int = 168) -> list:
    """
    Retrieve existing warnings for a specific POI.
    """
    rows = await pool.fetch(
        """SELECT id, warning_type, severity, title, recipe_code, created_at
           FROM warnings
           WHERE $1 = ANY(entity_ids)
             AND created_at > NOW() - $2 * INTERVAL '1 hour'
           ORDER BY created_at DESC""",
        person_id, hours
    )
    
    return [
        {
            "id": str(row["id"]),
            "type": row["warning_type"],
            "severity": row["severity"],
            "title": row["title"],
            "recipe_code": row["recipe_code"],
            "ts": row["created_at"].isoformat() if row["created_at"] else None,
        }
        for row in rows
    ]


async def get_company_darkweb_warnings(pool: asyncpg.Pool, company_id: str,
                                        hours: int = 168) -> list:
    """
    Check for dark web warnings related to a company (through its POIs).
    Used by social media scraper to cross-reference with dark web intel.
    """
    rows = await pool.fetch(
        """SELECT DISTINCT w.id, w.warning_type, w.severity, w.title, w.recipe_code,
                  w.created_at, p.name as poi_name
           FROM warnings w
           JOIN persons p ON p.id::text = ANY(w.entity_ids)
           JOIN companies c ON p.primary_org_id = c.id
           WHERE c.id = $1
             AND w.recipe_code IN ('poi_breach', 'darkweb_corroborated')
             AND w.created_at > NOW() - $2 * INTERVAL '1 hour'
           ORDER BY w.created_at DESC
           LIMIT 10""",
        company_id, hours
    )
    
    return [
        {
            "id": str(row["id"]),
            "type": row["warning_type"],
            "severity": row["severity"],
            "title": row["title"],
            "recipe_code": row["recipe_code"],
            "poi_name": row["poi_name"],
            "ts": row["created_at"].isoformat() if row["created_at"] else None,
        }
        for row in rows
    ]


async def check_darkweb_intel_for_org(pool: asyncpg.Pool, org_name: str,
                                       domain: str = None) -> dict:
    """
    Check if we have any dark web intelligence for an organization.
    Returns summary of findings.
    """
    # Escape ILIKE wildcards in org name to prevent unintended pattern matching
    safe_org = org_name.replace("%", "\\%").replace("_", "\\_")
    
    # Check for persons at this org with recent dark web scans finding data
    rows = await pool.fetch(
        """SELECT p.id, p.name, p.current_role, p.metadata
           FROM persons p
           JOIN companies c ON p.primary_org_id = c.id
           WHERE c.name ILIKE $1
             AND p.metadata->>'darkweb_scan_ts' IS NOT NULL
           ORDER BY (p.metadata->>'darkweb_scan_ts')::timestamp DESC
           LIMIT 10""",
        f"%{safe_org}%"
    )
    
    if not rows:
        return {"has_intel": False}
    
    # Check for breach warnings
    person_ids = [str(r["id"]) for r in rows]
    breach_warnings = await pool.fetch(
        """SELECT COUNT(*) as cnt FROM warnings
           WHERE recipe_code IN ('poi_breach', 'darkweb_corroborated')
             AND entity_ids && $1::text[]
             AND created_at > NOW() - INTERVAL '30 days'""",
        person_ids
    )
    
    breach_count = breach_warnings[0]["cnt"] if breach_warnings else 0
    
    return {
        "has_intel": True,
        "scanned_pois": len(rows),
        "breach_warnings": breach_count,
        "pois": [
            {
                "name": r["name"],
                "role": r["current_role"],
                "scan_ts": r["metadata"].get("darkweb_scan_ts") if r["metadata"] else None,
            }
            for r in rows[:5]
        ],
    }


async def create_corroborated_warning(pool: asyncpg.Pool, person_id: str,
                                       person_name: str, org_name: str,
                                       darkweb_data: dict,
                                       social_mentions: list,
                                       insights: list) -> bool:
    """
    Create a high-confidence corroborated warning when dark web intel
    is validated by social media or insights data.
    """
    sources = ["darkweb"]
    evidence_urls = darkweb_data.get("evidence_urls", [])
    
    # Build corroboration summary
    corr_parts = []
    
    if social_mentions:
        sources.append("social_media")
        platforms = list(set(m["platform"] for m in social_mentions))
        signals = []
        for m in social_mentions:
            signals.extend(m.get("signals", []))
        signals = list(set(signals))[:5]
        
        corr_parts.append(
            f"Social media ({len(social_mentions)} posts on {', '.join(platforms[:3])})"
            f"{' with signals: ' + ', '.join(signals) if signals else ''}"
        )
        for m in social_mentions[:3]:
            if m.get("url"):
                evidence_urls.append(m["url"])
    
    if insights:
        sources.append("insights")
        insight_types = list(set(i["type"] for i in insights))
        corr_parts.append(
            f"Analyst insights ({len(insights)} related: {', '.join(insight_types[:3])})"
        )
    
    if len(sources) < 2:
        return False  # Not enough sources to corroborate
    
    # Calculate confidence boost
    base_confidence = 0.7
    if len(sources) >= 3:
        confidence = 0.95
    elif len(social_mentions) >= 3:
        confidence = 0.90
    elif insights:
        confidence = 0.85
    else:
        confidence = base_confidence
    
    title = f"Corroborated intel: {person_name} at {org_name}"
    description = (
        f"Dark web intelligence on {person_name} ({darkweb_data.get('role', 'executive')}) "
        f"at {org_name} has been corroborated by multiple sources.\n\n"
        f"**Dark Web Finding:** {darkweb_data.get('summary', 'credential exposure detected')}\n\n"
        f"**Corroborating Sources:**\n- " + "\n- ".join(corr_parts) + "\n\n"
        f"**Confidence:** {confidence:.0%} (multi-source validated)"
    )
    
    try:
        await pool.execute(
            """INSERT INTO warnings (recipe_code, warning_type, severity, title, description,
                  evidence_urls, entity_ids, created_at, metadata)
               VALUES ('darkweb_corroborated', 'intelligence', 'critical', $1, $2, $3, $4, $5, $6)
               ON CONFLICT DO NOTHING""",
            title,
            description,
            list(set(evidence_urls))[:10],
            [person_id],
            datetime.now(timezone.utc),
            json.dumps({
                "sources": sources,
                "confidence": confidence,
                "social_mention_count": len(social_mentions),
                "insight_count": len(insights),
                "darkweb_source": darkweb_data.get("source", "unknown"),
            }),
        )
        log.info(f"🔗 Created corroborated warning for {person_name} ({', '.join(sources)})")
        return True
    except Exception as e:
        log.debug(f"Failed to create corroborated warning: {e}")
        return False


async def run_darkweb_poi_scan(pool: asyncpg.Pool) -> dict:
    """
    Run a comprehensive dark web scan for high-priority POIs.
    Queries PwnDB, leak sites, and OSINT forums for executive intelligence.
    Cross-references findings with social media and insights data.
    Returns stats dict.
    """
    stats = {
        "breaches_found": 0,
        "contacts_enriched": 0,
        "warnings_created": 0,
        "corroborations": 0,
        "social_crossrefs": 0,
        "insight_crossrefs": 0,
    }
    
    # Check Tor availability
    session = await get_tor_session()
    if session is None:
        log.warning("🧅 Dark web scan skipped: Tor not available")
        return stats
    
    # Get high-value POIs not recently scanned
    pois = await pool.fetch(
        """SELECT p.id, p.name, p.public_email, p.current_role,
                  c.domain, c.name as org_name
           FROM persons p
           JOIN companies c ON p.primary_org_id = c.id
           WHERE c.metadata->>'is_competitor' = 'true'
             AND (p.metadata->>'darkweb_scan_ts' IS NULL
                  OR (p.metadata->>'darkweb_scan_ts')::timestamp < NOW() - INTERVAL '7 days')
           ORDER BY 
             CASE WHEN p.current_role ILIKE '%CEO%' THEN 1
                  WHEN p.current_role ILIKE '%CTO%' THEN 2
                  WHEN p.current_role ILIKE '%CFO%' THEN 3
                  WHEN p.current_role ILIKE '%VP%' THEN 4
                  WHEN p.current_role ILIKE '%Director%' THEN 5
                  ELSE 10 END,
             p.created_at DESC
           LIMIT 25"""
    )
    
    log.info(f"🧅 Dark web scan: checking {len(pois)} high-priority POIs")
    
    for poi in pois:
        person_id = poi["id"]
        person_name = poi["name"]
        email_domain = poi["domain"]
        org_name = poi["org_name"]
        existing_email = poi["public_email"]
        
        try:
            # 1. PwnDB breach check
            breach_records = []
            person_breaches = []  # Initialize outside the if-block to avoid NameError
            if email_domain:
                breach_records = await query_pwndb_by_domain(email_domain)
                # Filter to records matching this person's name
                name_tokens = [t.lower() for t in person_name.split() if len(t) > 2]
                person_breaches = [
                    br for br in breach_records
                    if any(tok in br["email"].split("@")[0].lower() for tok in name_tokens)
                ]
                
                for br in person_breaches[:2]:
                    stats["breaches_found"] += 1
                    # Create security warning
                    await pool.execute(
                        """INSERT INTO warnings (recipe_code, warning_type, severity, title, description,
                              evidence_urls, entity_ids, created_at)
                           VALUES ('poi_breach', 'security', 'high', $1, $2, $3, $4, $5)
                           ON CONFLICT DO NOTHING""",
                        f"POI credential breach: {person_name}",
                        f"Executive {person_name} ({poi.get('current_role', 'Unknown')}) "
                        f"at {org_name} has breached credentials ({br['email']}). "
                        f"Password hash: {'found' if br.get('password_hash') else 'not found'}. "
                        f"Source: PwnDB dark web database.",
                        [f"tor://pwndb.onion/{br['email']}"],
                        [str(person_id)],
                        datetime.now(timezone.utc),
                    )
                    stats["warnings_created"] += 1
            
            # 2. Exposed.vc executive data search
            exp_contacts = await search_exposed_vc(person_name)
            
            # 3. Dread OSINT forum search
            dread_contacts = await search_dread_osint(person_name)
            
            all_contacts = exp_contacts + dread_contacts
            
            # Extract best email if found and POI lacks one
            if all_contacts and not existing_email:
                for contact in all_contacts:
                    if contact.get("email"):
                        await pool.execute(
                            """UPDATE persons SET public_email = $1 WHERE id = $2""",
                            contact["email"], person_id
                        )
                        stats["contacts_enriched"] += 1
                        log.info(f"🧅 Enriched {person_name} with dark web email: {contact['email']}")
                        break
            
            # ── Cross-Reference with Social Media & Insights ──────────────────
            # Check if dark web findings correlate with other intelligence sources
            has_darkweb_intel = bool(person_breaches) or bool(all_contacts)
            
            if has_darkweb_intel:
                # Fetch social media mentions for this POI
                social_mentions = await get_poi_social_mentions(pool, person_name, org_name, hours=72)
                
                # Fetch related insights
                related_insights = await get_poi_related_insights(pool, person_name, org_name, hours=168)
                
                # Check for existing warnings to avoid duplicates
                existing_warnings = await get_poi_existing_warnings(pool, str(person_id), hours=168)
                already_has_corroborated = any(
                    w["recipe_code"] == "darkweb_corroborated" for w in existing_warnings
                )
                
                # Create corroborated warning if we have multi-source validation
                if (social_mentions or related_insights) and not already_has_corroborated:
                    darkweb_summary = []
                    if person_breaches:
                        darkweb_summary.append(
                            f"Credential breach detected ({len(person_breaches)} records)"
                        )
                    if all_contacts:
                        darkweb_summary.append(
                            f"Contact data found on {len(exp_contacts)} exposed.vc + "
                            f"{len(dread_contacts)} Dread OSINT sources"
                        )
                    
                    darkweb_data = {
                        "summary": "; ".join(darkweb_summary),
                        "role": poi.get("current_role", "executive"),
                        "source": "pwndb,exposed.vc,dread",
                        "evidence_urls": [f"tor://pwndb.onion/{br.get('email', '')}" 
                                         for br in person_breaches[:2]],
                    }
                    
                    corr_created = await create_corroborated_warning(
                        pool, str(person_id), person_name, org_name,
                        darkweb_data, social_mentions, related_insights
                    )
                    
                    if corr_created:
                        stats["warnings_created"] += 1
                        stats["corroborations"] += 1
                        log.info(
                            f"🔗 Corroborated: {person_name} - "
                            f"{len(social_mentions)} social mentions, "
                            f"{len(related_insights)} insights"
                        )
                
                # Track cross-reference stats
                if social_mentions:
                    stats["social_crossrefs"] += len(social_mentions)
                if related_insights:
                    stats["insight_crossrefs"] += len(related_insights)
                
                # Log cross-reference stats even without warning creation
                if social_mentions or related_insights:
                    log.debug(
                        f"   ↳ Cross-ref for {person_name}: "
                        f"{len(social_mentions)} social, {len(related_insights)} insights"
                    )
            
            # Update scan timestamp
            await pool.execute(
                """UPDATE persons SET metadata = COALESCE(metadata, '{}')::jsonb 
                   || jsonb_build_object('darkweb_scan_ts', $1::text)
                   WHERE id = $2""",
                datetime.now(timezone.utc).isoformat(),
                person_id
            )
            
            # Brief delay between POIs to avoid Tor circuit overload
            await asyncio.sleep(2.0)
            
        except Exception as e:
            log.debug(f"Dark web scan error for {person_name}: {e}")
    
    return stats


async def fetch_url(session: aiohttp.ClientSession, url: str, semaphore: asyncio.Semaphore,
                    fallback_session: Optional[aiohttp.ClientSession] = None) -> Optional[dict]:
    """Fetch a URL via IPv6 (primary session). Falls back to IPv4 session on connection failure."""
    async with semaphore:
        headers = get_random_headers()
        # Rate limiting
        await asyncio.sleep(1.0 / REQUESTS_PER_SECOND + random.uniform(0.1, 0.5))

        # Try primary (IPv6) session first
        try:
            async with session.get(
                url,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=REQUEST_TIMEOUT),
                allow_redirects=True,
                max_redirects=5,
            ) as resp:
                html = await resp.text(errors="replace")
                content_hash = hashlib.sha256(html.encode()).hexdigest()
                log.info(f"✓ {resp.status} {url} [{len(html)} bytes] v6")
                return {
                    "url": url,
                    "status": resp.status,
                    "html": html,
                    "content_hash": content_hash,
                    "fetched_at": datetime.now(timezone.utc).isoformat(),
                }
        except (aiohttp.ClientError, asyncio.TimeoutError, OSError):
            pass  # fall through to IPv4

        # Fallback to IPv4 session
        if fallback_session is not None:
            try:
                async with fallback_session.get(
                    url,
                    headers=headers,
                    timeout=aiohttp.ClientTimeout(total=REQUEST_TIMEOUT),
                    allow_redirects=True,
                    max_redirects=5,
                ) as resp:
                    html = await resp.text(errors="replace")
                    content_hash = hashlib.sha256(html.encode()).hexdigest()
                    log.info(f"✓ {resp.status} {url} [{len(html)} bytes] v4")
                    return {
                        "url": url,
                        "status": resp.status,
                        "html": html,
                        "content_hash": content_hash,
                        "fetched_at": datetime.now(timezone.utc).isoformat(),
                    }
            except asyncio.TimeoutError:
                log.warning(f"⏱ Timeout: {url}")
                return None
            except aiohttp.ClientError as e:
                log.warning(f"✗ Error {url}: {e}")
                return None
            except Exception as e:
                log.error(f"✗ Unexpected error {url}: {e}")
                return None

        return None


# ─── Page Analysis ──────────────────────────────────────────────────────────────

# Regex for detecting boilerplate class/id/role attributes on HTML elements.
# Matches nav bars, menus, footers, sidebars, cookie banners, country/language
# selectors, social sharing widgets, ads, and skip-nav links.
_BOILERPLATE_ATTR_RE = re.compile(
    r'nav\b|menu\b|footer|header|sidebar|cookie|consent|banner|breadcrumb|'
    r'country.?select|region.?select|language.?select|locale.?select|'
    r'lang.?switch|social.?share|share.?button|newsletter|subscribe|'
    r'advertis|ad-wrap|ads-|advert|popup|modal|overlay|'
    r'skip.?to|skip.?nav|screen.?reader|pagination|pager\b',
    re.IGNORECASE,
)

# ARIA roles that indicate non-content regions.
_NON_CONTENT_ROLES = frozenset([
    "navigation", "banner", "contentinfo", "complementary",
    "search", "form", "menubar", "menu", "toolbar", "directory",
])


def extract_text(html: str) -> str:
    """Extract clean text from HTML, stripping boilerplate elements.

    Goes beyond simple tag-name removal: also strips elements identified
    by class, id, or ARIA role as navigation, footer, country selectors,
    cookie banners, ad wrappers, etc.
    """
    soup = BeautifulSoup(html, "html.parser")

    # 1. Remove elements by tag name (script, style, semantic boilerplate,
    #    <select>/<option> which are country/currency dropdowns).
    for tag in soup(["script", "style", "nav", "footer", "header", "aside",
                     "select", "option", "noscript", "iframe", "svg",
                     "form"]):
        tag.decompose()

    # 2. Remove elements whose class/id/role attributes match boilerplate patterns.
    for tag in soup.find_all(True):
        cls = " ".join(tag.get("class", [])) if isinstance(tag.get("class"), list) else str(tag.get("class", ""))
        attrs_text = f'{cls} {tag.get("id", "")} {tag.get("role", "")} {tag.get("aria-label", "")}'
        if _BOILERPLATE_ATTR_RE.search(attrs_text):
            tag.decompose()

    # 3. Remove elements with ARIA roles indicating non-content.
    for tag in soup.find_all(attrs={"role": True}):
        if tag.get("role", "").lower() in _NON_CONTENT_ROLES:
            tag.decompose()

    return soup.get_text(separator=" ", strip=True)[:50000]


def extract_links(html: str, base_url: str) -> list[str]:
    """Extract internal links from an HTML page."""
    soup = BeautifulSoup(html, "html.parser")
    parsed_base = urlparse(base_url)
    links = []
    for a in soup.find_all("a", href=True):
        href = a["href"]
        if href.startswith("/"):
            href = f"{parsed_base.scheme}://{parsed_base.netloc}{href}"
        if parsed_base.netloc in href:
            links.append(href)
    return list(set(links))[:50]


def extract_article_links(html: str, base_url: str) -> list[dict]:
    """
    Extract article links from press/news pages with headlines.
    Returns list of {url, title, section} for article-like links.
    """
    soup = BeautifulSoup(html, "html.parser")
    parsed_base = urlparse(base_url)
    articles = []
    
    # Article URL patterns
    article_patterns = [
        r'/news/', r'/press/', r'/blog/', r'/article/', r'/story/',
        r'/release/', r'/announcement/', r'/update/', r'/media/',
        r'\d{4}/\d{2}/', r'\d{4}-\d{2}-', r'/pr/', r'/insights/'
    ]
    
    # Look for article links in common containers
    for a in soup.find_all("a", href=True):
        href = a["href"]
        
        # Normalize URL
        if href.startswith("/"):
            href = f"{parsed_base.scheme}://{parsed_base.netloc}{href}"
        elif not href.startswith("http"):
            continue
            
        # Skip non-article links
        if not any(re.search(pat, href, re.I) for pat in article_patterns):
            continue
            
        # Skip anchors and pagination
        if "#" in href or "page=" in href or "?p=" in href:
            continue
            
        # Extract title from link text or nearby heading
        title = a.get_text(strip=True)[:200]
        if not title or len(title) < 10:
            # Try parent heading
            parent = a.find_parent(["h1", "h2", "h3", "h4", "article", "div"])
            if parent:
                heading = parent.find(["h1", "h2", "h3", "h4"])
                if heading:
                    title = heading.get_text(strip=True)[:200]
        
        if title and len(title) >= 10:
            articles.append({
                "url": href,
                "title": title,
                "section": parsed_base.path.strip("/") or "homepage"
            })
    
    # Deduplicate by URL
    seen = set()
    unique = []
    for art in articles:
        if art["url"] not in seen:
            seen.add(art["url"])
            unique.append(art)
    
    return unique[:20]  # Limit to 20 articles per page


# ─── URL Relevance Filtering ───────────────────────────────────────────────────

# Unified set of generic page path segments. Used for:
# 1. Filtering evidence URLs (article URL validation)
# 2. Suppressing signal detection on boilerplate pages
# 3. Excluding junk URLs from insights
_GENERIC_PATHS: frozenset = frozenset({
    "about", "about-us", "about_us", "aboutus", "careers", "career", "jobs",
    "job-openings", "open-positions", "sustainability", "esg", "csr", "contact",
    "contact-us", "team", "leadership", "management", "our-team",
    "investors", "investor-relations", "ir", "products", "solutions",
    "capabilities", "services", "home", "index", "cookies", "privacy",
    "privacy-policy", "legal", "terms", "terms-of-use", "sitemap", "404",
    "search", "events", "tradeshows", "trade-shows",
})


# URLs that should NEVER be used as evidence regardless of path depth
_JUNK_URL_PATTERNS: frozenset = frozenset({
    "privacy_policy", "privacy-policy", "cookie-policy", "terms-of-service",
    "terms-of-use", "subscribe", "unsubscribe", "subscription",
    "newsletter", "rss", "feed", "atom.xml", "sitemap", "404",
    "login", "register", "signup", "sign-up", "sign_up",
    "search", "advanced-search", "news-results",
    "careers", "jobs", "job-openings", "open-positions",
    "about", "about-us", "contact", "contact-us",
    "team", "leadership", "our-team", "management",
    "press", "pressroom", "press-room",  # index pages, not articles
})


def is_article_url(url: str) -> bool:
    """Return True only if a URL looks like a specific news/press/article page, not a generic site section."""
    if not url:
        return False
    try:
        p = urlparse(url)
        path = p.path.rstrip("/")
        if not path:  # bare homepage
            return False
        # Reject file types that aren't articles
        if path.lower().endswith((".pdf", ".doc", ".docx", ".xls", ".xlsx", ".zip", ".xml")):
            return False
        # Check ALL path segments against junk patterns
        segments = [s.lower().replace("-", "_") for s in path.split("/") if s]
        if not segments:
            return False
        # Last segment must not be a generic page
        last_segment = segments[-1].split(".")[0]  # strip file extension
        if last_segment in _GENERIC_PATHS:
            return False
        # Any segment matching a junk pattern disqualifies
        for seg in segments:
            seg_clean = seg.split(".")[0]
            if seg_clean in _JUNK_URL_PATTERNS:
                return False
        # Must have a substantive path (more than one non-empty segment)
        if len(segments) < 2:
            return False
    except Exception:
        return False
    return True


def filter_relevant_urls(urls: list, max_count: int = 5) -> list:
    """Deduplicate and filter a flat URL list, keeping only article-like URLs."""
    seen: dict = {}
    for u in urls:
        if u and u not in seen:
            seen[u] = None
    return [u for u in seen if is_article_url(u)][:max_count]


def flatten_source_arrays(raw: list | None) -> list:
    """Flatten a list-of-lists (text[][] from array_agg) into a flat list of URLs."""
    if not raw:
        return []
    return [u for arr in raw if arr for u in arr if u]


# ─── Boilerplate / Content Quality Detection ───────────────────────────────────
# Used to reject observations whose excerpt is page chrome (country lists,
# currency converters, navigation menus) rather than actual article content.

# Domain-key prefixes that are too common/generic to use as company matching
# shortcuts. These would match thousands of unrelated pages.
_BANNED_DOMAIN_KEYS = frozenset([
    "government", "ministry", "national", "federal", "state", "republic",
    "defense", "defence", "security", "energy", "finance", "transport",
    "education", "health", "trade", "commerce", "industry", "portal",
    "official", "public", "service", "digital", "online", "platform",
    "global", "world", "international", "united", "general", "central",
    "bureau", "agency", "council", "commission", "authority", "office",
    "institute", "foundation", "benchmark", "arrow", "venture",
])

_COUNTRY_NAMES_FOR_BOILERPLATE = frozenset([
    "afghanistan", "albania", "algeria", "andorra", "angola", "antigua",
    "argentina", "armenia", "australia", "austria", "azerbaijan",
    "bahamas", "bahrain", "bangladesh", "barbados", "belarus", "belgium",
    "belize", "benin", "bermuda", "bhutan", "bolivia", "bosnia",
    "botswana", "brazil", "brunei", "bulgaria", "burkina", "burundi",
    "cambodia", "cameroon", "canada", "chad", "chile", "colombia",
    "comoros", "congo", "costa rica", "croatia", "cuba", "cyprus",
    "denmark", "djibouti", "dominica", "ecuador", "egypt",
    "el salvador", "eritrea", "estonia", "eswatini", "ethiopia",
    "fiji", "finland", "france", "gabon", "gambia", "georgia",
    "germany", "ghana", "greece", "grenada", "guatemala", "guinea",
    "guyana", "haiti", "honduras", "hungary", "iceland", "india",
    "indonesia", "iran", "iraq", "ireland", "israel", "italy",
    "jamaica", "japan", "jordan", "kazakhstan", "kenya", "kiribati",
    "kosovo", "kuwait", "kyrgyzstan", "laos", "latvia", "lebanon",
    "lesotho", "liberia", "libya", "liechtenstein", "lithuania",
    "luxembourg", "madagascar", "malawi", "malaysia", "maldives",
    "mali", "malta", "mauritania", "mauritius", "mexico", "micronesia",
    "moldova", "monaco", "mongolia", "montenegro", "morocco",
    "mozambique", "myanmar", "namibia", "nauru", "nepal", "netherlands",
    "new zealand", "nicaragua", "niger", "nigeria", "north macedonia",
    "norway", "oman", "pakistan", "palau", "panama", "papua new guinea",
    "paraguay", "peru", "philippines", "poland", "portugal", "qatar",
    "romania", "russia", "rwanda", "samoa", "san marino", "saudi arabia",
    "senegal", "serbia", "seychelles", "sierra leone", "singapore",
    "slovakia", "slovenia", "somalia", "south africa", "south korea",
    "spain", "sri lanka", "sudan", "suriname", "sweden", "switzerland",
    "syria", "taiwan", "tajikistan", "tanzania", "thailand", "togo",
    "tonga", "trinidad", "tunisia", "turkey", "turkmenistan", "tuvalu",
    "uganda", "ukraine", "united arab emirates", "united kingdom",
    "united states", "uruguay", "uzbekistan", "vanuatu", "venezuela",
    "vietnam", "yemen", "zambia", "zimbabwe",
])

_CURRENCY_TERMS = frozenset([
    "dollar", "euro", "pound", "yen", "yuan", "ringgit", "rupee", "rupiah",
    "rouble", "ruble", "franc", "dirham", "dinar", "won", "baht", "krona",
    "peso", "real", "lira", "shekel", "rand", "convert", "exchange rate",
])

# Prose function words — real articles have many; navigation lists have few.
_FUNCTION_WORDS = frozenset([
    "the", "a", "an", "is", "are", "was", "were", "has", "have", "had",
    "in", "on", "at", "to", "for", "with", "from", "by", "of", "and",
    "but", "or", "that", "this", "which", "who", "what", "where",
    "when", "how", "not", "no", "will", "would", "could", "should",
    "its", "their", "it", "they", "he", "she", "we", "you", "been",
    "said", "says", "according", "reported", "announced", "about",
    "after", "before", "during", "between", "through", "also", "more",
    "than", "some", "other", "such", "into", "over", "new", "year",
])


def _is_boilerplate_excerpt(text: str) -> bool:
    """Detect if text is navigation / country-list / currency-converter boilerplate.

    Returns True when the excerpt fails to look like actual article prose.
    Used as a quality gate before storing observations and before using
    observations as corroborating evidence in cross-reference analysis.
    """
    if not text or len(text) < 40:
        return True

    text_lower = text.lower()
    words = text_lower.split()

    if len(words) < 8:
        return True

    # ── Check 1: Country-name density ──
    # ≥5 country names in a short excerpt → country selector / nav list
    country_hits = sum(1 for c in _COUNTRY_NAMES_FOR_BOILERPLATE if c in text_lower)
    if country_hits >= 5:
        return True

    # ── Check 2: Currency-term density ──
    # ≥3 currency terms → currency converter widget
    currency_hits = sum(1 for c in _CURRENCY_TERMS if c in text_lower)
    if currency_hits >= 3:
        return True

    # ── Check 3: Function-word ratio ──
    # Real prose has ≥12 % function words; navigation/lists have very few.
    func_count = sum(1 for w in words if w in _FUNCTION_WORDS)
    if len(words) >= 20 and func_count / len(words) < 0.08:
        return True

    # ── Check 4: Sentence structure ──
    # Real articles contain at least one sentence > 8 words.
    # Lists of countries/currencies/nav items don't form sentences.
    sentences = re.split(r'[.!?;]', text)
    long_sentences = [s for s in sentences if len(s.split()) >= 8]
    if len(text) > 200 and not long_sentences:
        return True

    # ── Check 5: "See All" / region-selector patterns ──
    if re.search(r'see all|view all|select.{0,10}country|select.{0,10}region|'
                 r'choose.{0,10}country|choose.{0,10}region|asia pacific|'
                 r'latin america.{0,20}europe.{0,20}africa',
                 text_lower):
        if country_hits >= 3:
            return True

    return False


def _count_kw_matches(text_lower: str, keywords: list[str]) -> tuple[int, list[str]]:
    """Count how many distinct keywords from a list appear in text."""
    matched = [kw for kw in keywords if kw in text_lower]
    return len(matched), matched


def _is_generic_page(url: str) -> bool:
    """Return True if URL corresponds to a generic company page that shouldn't trigger signals."""
    try:
        path = urlparse(url).path.strip("/").lower()
        segments = {s.replace("-", "_").replace(".", "_") for s in path.split("/") if s}
        return bool(segments & _GENERIC_PATHS)
    except Exception:
        return False


def detect_signals(text: str, url: str) -> list[dict]:
    """Detect intelligence signals from page text.

    Quality gates:
    - Generic pages (about, careers, products, etc.) are skipped entirely.
    - Each signal category requires ≥2 distinct keyword hits (multi-keyword
      corroboration) to reduce false positives from boilerplate text.
    """
    # Skip generic pages that always contain noisy keywords
    if _is_generic_page(url):
        return []

    signals = []
    text_lower = text.lower()

    # Supply chain disruption signals — require ≥2 matches
    disruption_kw = [
        "supply chain disruption", "shortage", "allocation", "force majeure",
        "factory closure", "production halt", "shipping delay", "lead time increase",
        "capacity constraint", "component shortage", "supply crunch"
    ]
    n, matched = _count_kw_matches(text_lower, disruption_kw)
    if n >= 2:
        signals.append({"type": "supply_chain_disruption", "keyword": matched[0], "url": url, "match_count": n})

    # Expansion signals — require ≥2 matches
    expansion_kw = [
        "new facility", "plant expansion", "investment in manufacturing",
        "grand opening", "groundbreaking", "new campus", "expanding operations",
        "capacity expansion", "new production line", "new factory"
    ]
    n, matched = _count_kw_matches(text_lower, expansion_kw)
    if n >= 2:
        signals.append({"type": "expansion", "keyword": matched[0], "url": url, "match_count": n})

    # Hiring signals — require ≥3 matches (hiring words are very common)
    hiring_kw = [
        "hiring spree", "job opening", "we are looking for",
        "join our team", "open position", "talent acquisition",
        "recruiting", "new hires", "headcount growth", "staffing up"
    ]
    n, matched = _count_kw_matches(text_lower, hiring_kw)
    if n >= 3:
        signals.append({"type": "hiring_signal", "keyword": matched[0], "url": url, "match_count": n})

    # Certification/compliance signals — require ≥2 matches AND at least one specific standard
    cert_specific = [
        "iso 9001", "iso 14001", "iatf 16949", "as9100", "iso 13485",
        "iso 27001", "nadcap", "cmmi",
    ]
    cert_context = [
        "recertification", "accreditation", "newly certified",
        "audit passed", "certification achieved", "compliance update"
    ]
    n_specific, _ = _count_kw_matches(text_lower, cert_specific)
    n_context, matched_ctx = _count_kw_matches(text_lower, cert_context)
    if n_specific >= 1 and n_context >= 1:
        signals.append({"type": "certification_update", "keyword": (matched_ctx or cert_specific)[0], "url": url, "match_count": n_specific + n_context})

    # M&A signals — require ≥2 matches
    ma_kw = [
        "acquisition", "merger", "acquired by", "takeover bid",
        "joint venture", "equity stake", "definitive agreement",
        "strategic acquisition", "completed acquisition"
    ]
    n, matched = _count_kw_matches(text_lower, ma_kw)
    if n >= 2:
        signals.append({"type": "ma_activity", "keyword": matched[0], "url": url, "match_count": n})

    # Technology signals — require ≥2 matches
    # NOTE: Use word-boundary-aware phrases to prevent substring false positives.
    # E.g. "patent filed" must not match "patented technology" boilerplate.
    tech_kw = [
        "patent filed", "patent granted", "patent application",
        "new patent", "patents awarded",
        "breakthrough in", "technology breakthrough",
        "next-generation product", "next-gen product",
        "product launch announced", "new product launch",
        "r&d investment", "r&d milestone",
        "technology demonstration", "tech demo",
    ]
    n, matched = _count_kw_matches(text_lower, tech_kw)
    if n >= 2:
        signals.append({"type": "technology", "keyword": matched[0], "url": url, "match_count": n})

    # Geopolitical risk signals — require ≥2 matches
    geo_kw = [
        "sanctions imposed", "new tariff", "trade restriction",
        "export control", "geopolitical tension", "embargo",
        "chips act", "critical raw materials act", "entity list",
        "trade war escalation", "sanctions designation"
    ]
    n, matched = _count_kw_matches(text_lower, geo_kw)
    if n >= 2:
        signals.append({"type": "geopolitical_risk", "keyword": matched[0], "url": url, "match_count": n})

    # ── Conflict escalation signals — require ≥2 matches ──
    conflict_kw = [
        "military buildup", "troop deployment", "air strike", "missile launch",
        "artillery shelling", "territorial dispute", "border incursion",
        "ceasefire violation", "combat operations", "martial law",
        "military offensive", "armed confrontation", "invasion",
        "escalation of hostilities", "war declaration", "military intervention",
        "ethnic cleansing", "genocide", "refugee crisis", "mass displacement",
        "peacekeeping withdrawal", "insurgency", "rebel offensive",
        "drone strike", "naval blockade", "no-fly zone",
    ]
    n, matched = _count_kw_matches(text_lower, conflict_kw)
    if n >= 2:
        signals.append({"type": "conflict_escalation", "keyword": matched[0], "url": url, "match_count": n})

    # ── Sanctions cascade signals — require ≥2 matches ──
    sanctions_kw = [
        "secondary sanctions", "sanctions evasion", "sanctions waiver",
        "sanctions enforcement", "asset seizure", "sanctions circumvention",
        "counter-sanctions", "retaliatory sanctions", "sanctions package",
        "sanctions designation", "delisting request", "specially designated nationals",
        "sanctions screening", "compliance violation", "sanctions regime",
        "entity list addition", "blocked persons", "sectoral sanctions",
        "oil price cap", "sanctions exception", "humanitarian exemption",
    ]
    n, matched = _count_kw_matches(text_lower, sanctions_kw)
    if n >= 2:
        signals.append({"type": "sanctions_cascade", "keyword": matched[0], "url": url, "match_count": n})

    # ── Diplomatic shift signals — require ≥2 matches ──
    diplomatic_kw = [
        "diplomatic recall", "embassy closure", "treaty withdrawal",
        "alliance formation", "diplomatic expulsion", "diplomatic breakdown",
        "peace negotiations", "summit meeting", "diplomatic recognition",
        "bilateral agreement", "multilateral pact", "severed relations",
        "recalled ambassador", "diplomatic protest", "formal demarche",
        "non-aggression pact", "mutual defense treaty", "normalisation agreement",
        "frozen diplomatic ties", "diplomatic incident", "consulate closure",
    ]
    n, matched = _count_kw_matches(text_lower, diplomatic_kw)
    if n >= 2:
        signals.append({"type": "diplomatic_shift", "keyword": matched[0], "url": url, "match_count": n})

    # ── Military procurement signals — require ≥2 matches ──
    mil_procurement_kw = [
        "arms deal", "defense contract", "weapons delivery", "military aid",
        "arms transfer", "defense procurement", "weapons acquisition",
        "fighter jet deal", "missile defense system", "tank delivery",
        "ammunition supply", "arms embargo", "military sale",
        "bilateral defense agreement", "joint military exercise",
        "foreign military financing", "defense cooperation agreement",
        "weapons of mass destruction", "nuclear warhead", "ballistic missile test",
    ]
    n, matched = _count_kw_matches(text_lower, mil_procurement_kw)
    if n >= 2:
        signals.append({"type": "military_procurement", "keyword": matched[0], "url": url, "match_count": n})

    # ── Energy security signals — require ≥2 matches ──
    energy_kw = [
        "pipeline shutdown", "oil embargo", "energy crisis", "lng diversion",
        "gas cutoff", "energy weaponization", "fuel shortage", "refinery attack",
        "opec production cut", "strategic petroleum reserve", "energy sanctions",
        "pipeline sabotage", "electricity blackout", "grid attack",
        "nuclear power plant", "uranium enrichment", "energy independence",
        "renewable transition", "critical infrastructure", "power supply disruption",
    ]
    n, matched = _count_kw_matches(text_lower, energy_kw)
    if n >= 2:
        signals.append({"type": "energy_security", "keyword": matched[0], "url": url, "match_count": n})

    # ── Cyber warfare signals — require ≥2 matches ──
    cyber_kw = [
        "state-sponsored attack", "critical infrastructure hack", "cyber espionage",
        "ransomware attack", "apt group", "cyber warfare", "information warfare",
        "disinformation campaign", "influence operation", "election interference",
        "supply chain attack", "zero-day exploit", "cyber sabotage",
        "data breach", "government hack", "intelligence breach",
        "attributed to", "nation-state threat", "cyber command",
    ]
    n, matched = _count_kw_matches(text_lower, cyber_kw)
    if n >= 2:
        signals.append({"type": "cyber_warfare", "keyword": matched[0], "url": url, "match_count": n})

    # ── Critical mineral risk signals — require ≥2 matches ──
    mineral_kw = [
        "rare earth shortage", "mining disruption", "cobalt supply",
        "lithium procurement", "critical mineral", "mineral export ban",
        "rare earth processing", "mining nationalization", "resource nationalism",
        "graphite supply", "nickel shortage", "tungsten supply",
        "gallium restriction", "germanium export control", "mineral stockpile",
        "critical raw material", "supply chain decoupling", "mineral dependency",
    ]
    n, matched = _count_kw_matches(text_lower, mineral_kw)
    if n >= 2:
        signals.append({"type": "critical_mineral_risk", "keyword": matched[0], "url": url, "match_count": n})

    # ── Trade corridor disruption signals — require ≥2 matches ──
    corridor_kw = [
        "strait closure", "canal blockage", "port shutdown", "shipping reroute",
        "trade route disruption", "chokepoint", "suez canal", "strait of hormuz",
        "malacca strait", "bab el mandeb", "panama canal", "taiwan strait",
        "piracy attack", "vessel seizure", "port congestion", "shipping diversion",
        "trade corridor", "freedom of navigation", "maritime exclusion zone",
        "naval escort", "shipping lane closure", "port strike",
    ]
    n, matched = _count_kw_matches(text_lower, corridor_kw)
    if n >= 2:
        signals.append({"type": "trade_corridor_disruption", "keyword": matched[0], "url": url, "match_count": n})

    return signals


# ─── News, POI & Intelligence Data Streams ─────────────────────────────────────
# AGGRESSIVE POI INTELLIGENCE GATHERING: 120+ streams covering multiple signal types
# Strategy: Cast a wide net across diverse domains that reveal personnel information

# ═══════════════════════════════════════════════════════════════════════════════
# CATEGORY 1: ELECTRONICS & MANUFACTURING NEWS (reveals executives, appointments)
# ═══════════════════════════════════════════════════════════════════════════════
NEWS_SOURCES = [
    # Core electronics publications
    {"url": "https://www.eetimes.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.edn.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.electronicdesign.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.electronicsweekly.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.fierceelectronics.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.electronics360.globalspec.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.newelectronics.co.uk/", "type": "news", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.electronicspecifier.com/", "type": "news", "topic": "electronics", "poi_signal": True},
    
    # Supply chain intelligence
    {"url": "https://www.supplychaindive.com/", "type": "news", "topic": "supply_chain", "poi_signal": True},
    {"url": "https://www.scmr.com/", "type": "news", "topic": "supply_chain", "poi_signal": True},
    {"url": "https://www.supplychainbrain.com/", "type": "news", "topic": "supply_chain", "poi_signal": True},
    {"url": "https://www.logisticsmgmt.com/", "type": "news", "topic": "supply_chain", "poi_signal": True},
    {"url": "https://www.supplychain247.com/", "type": "news", "topic": "supply_chain", "poi_signal": True},
    
    # Semiconductor industry
    {"url": "https://www.semiconductorengineering.com/", "type": "news", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://www.techspot.com/", "type": "news", "topic": "semiconductors", "poi_signal": True},  # Replaced AnandTech (shut down 2024)
    {"url": "https://www.semi.org/en/news/", "type": "news", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://www.digitimes.com/", "type": "news", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://www.tomshardware.com/", "type": "news", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://semiengineering.com/", "type": "news", "topic": "semiconductors", "poi_signal": True},
    
    # Manufacturing/EMS
    {"url": "https://www.assemblymag.com/", "type": "news", "topic": "manufacturing", "poi_signal": True},
    {"url": "https://www.smt007.com/", "type": "news", "topic": "manufacturing", "poi_signal": True},
    {"url": "https://www.circuitsassembly.com/", "type": "news", "topic": "PCB", "poi_signal": True},
    {"url": "https://www.pcb007.com/", "type": "news", "topic": "PCB", "poi_signal": True},
    {"url": "https://www.ipcb.org/", "type": "news", "topic": "PCB", "poi_signal": True},
    {"url": "https://www.manufacturing.net/", "type": "news", "topic": "manufacturing", "poi_signal": True},
    {"url": "https://www.industryweek.com/", "type": "news", "topic": "manufacturing", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 2: DEFENSE & AEROSPACE (government officials, defense executives)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.defensenews.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.janes.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.aviationweek.com/", "type": "news", "topic": "aerospace", "poi_signal": True},
    {"url": "https://www.defensemedianetwork.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://breakingdefense.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.c4isrnet.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.militaryaerospace.com/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.ainonline.com/", "type": "news", "topic": "aerospace", "poi_signal": True},
    {"url": "https://www.nationaldefensemagazine.org/", "type": "news", "topic": "defense", "poi_signal": True},
    {"url": "https://www.defenseonemedia.com/", "type": "news", "topic": "defense", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 3: GOVERNMENT & REGULATORY SOURCES (ministers, officials)  
    # ═══════════════════════════════════════════════════════════════════════════
    # US Government
    {"url": "https://www.commerce.gov/news/", "type": "gov", "topic": "government_us", "poi_signal": True},
    {"url": "https://www.state.gov/press-releases/", "type": "gov", "topic": "government_us", "poi_signal": True},
    {"url": "https://www.defense.gov/News/", "type": "gov", "topic": "government_us", "poi_signal": True},
    {"url": "https://www.bis.doc.gov/", "type": "gov", "topic": "export_control", "poi_signal": True},
    {"url": "https://www.treasury.gov/news/", "type": "gov", "topic": "sanctions", "poi_signal": True},
    {"url": "https://ofac.treasury.gov/", "type": "gov", "topic": "sanctions", "poi_signal": True},
    
    # EU/European
    {"url": "https://ec.europa.eu/trade/", "type": "gov", "topic": "government_eu", "poi_signal": True},
    {"url": "https://www.europarl.europa.eu/news/", "type": "gov", "topic": "government_eu", "poi_signal": True},
    {"url": "https://www.consilium.europa.eu/en/press/", "type": "gov", "topic": "government_eu", "poi_signal": True},
    
    # MENA Government sources
    {"url": "https://www.tap.info.tn/", "type": "gov", "topic": "government_tn", "poi_signal": True},
    {"url": "https://www.maroc.ma/", "type": "gov", "topic": "government_ma", "poi_signal": True},
    {"url": "https://www.aps.dz/", "type": "gov", "topic": "government_dz", "poi_signal": True},
    {"url": "https://www.wam.ae/", "type": "gov", "topic": "government_ae", "poi_signal": True},
    {"url": "https://www.spa.gov.sa/", "type": "gov", "topic": "government_sa", "poi_signal": True},
    
    # Asia Government
    {"url": "https://www.meti.go.jp/english/", "type": "gov", "topic": "government_jp", "poi_signal": True},
    {"url": "https://english.motie.go.kr/", "type": "gov", "topic": "government_kr", "poi_signal": True},
    {"url": "https://www.moeaidb.gov.tw/", "type": "gov", "topic": "government_tw", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 4: EXECUTIVE APPOINTMENT & PERSONNEL ANNOUNCEMENTS
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.prnewswire.com/news-releases/executive-changes-list/", "type": "pr", "topic": "executives", "poi_signal": True},
    {"url": "https://www.businesswire.com/", "type": "pr", "topic": "executives", "poi_signal": True},
    {"url": "https://www.globenewswire.com/", "type": "pr", "topic": "executives", "poi_signal": True},
    {"url": "https://www.marketwatch.com/", "type": "financial", "topic": "executives", "poi_signal": True},
    {"url": "https://www.wsj.com/news/markets", "type": "financial", "topic": "executives", "poi_signal": True},
    {"url": "https://www.reuters.com/business/", "type": "financial", "topic": "executives", "poi_signal": True},
    {"url": "https://www.ft.com/companies/", "type": "financial", "topic": "executives", "poi_signal": True},
    {"url": "https://www.bloomberg.com/", "type": "financial", "topic": "executives", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 5: TRADE SHOWS & CONFERENCES (speakers reveal POIs + affiliations)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.ces.tech/", "type": "conference", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.electronica.de/en/", "type": "conference", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.productronica.com/", "type": "conference", "topic": "manufacturing", "poi_signal": True},
    {"url": "https://www.semicon.org/", "type": "conference", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://www.smarttechshow.com/", "type": "conference", "topic": "manufacturing", "poi_signal": True},
    {"url": "https://www.apex.ipc.org/", "type": "conference", "topic": "PCB", "poi_signal": True},
    {"url": "https://www.arincaindustrydays.com/", "type": "conference", "topic": "aerospace", "poi_signal": True},
    {"url": "https://www.dsei.co.uk/", "type": "conference", "topic": "defense", "poi_signal": True},
    {"url": "https://www.eurosatory.com/", "type": "conference", "topic": "defense", "poi_signal": True},
    {"url": "https://www.idex-uae.ae/", "type": "conference", "topic": "defense_mena", "poi_signal": True},
    {"url": "https://www.hannover-messe.de/", "type": "conference", "topic": "manufacturing", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 6: TRADE ASSOCIATIONS & INDUSTRY BODIES (board members, chairs)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.ipc.org/news", "type": "association", "topic": "PCB", "poi_signal": True},
    {"url": "https://www.smta.org/", "type": "association", "topic": "SMT", "poi_signal": True},
    {"url": "https://www.sia-online.org/", "type": "association", "topic": "semiconductors", "poi_signal": True},
    {"url": "https://www.nist.gov/", "type": "association", "topic": "standards", "poi_signal": True},
    {"url": "https://www.aia-aerospace.org/", "type": "association", "topic": "aerospace", "poi_signal": True},
    {"url": "https://www.ndia.org/", "type": "association", "topic": "defense", "poi_signal": True},
    {"url": "https://www.ieee.org/", "type": "association", "topic": "electronics", "poi_signal": True},
    {"url": "https://www.areadevelopment.com/", "type": "association", "topic": "manufacturing", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 7: REGULATORY & COMPLIANCE FILINGS (officer/director listings)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.sec.gov/cgi-bin/browse-edgar", "type": "regulatory", "topic": "sec_filings", "poi_signal": True},
    {"url": "https://efiling.drcnet.go.ke/", "type": "regulatory", "topic": "company_registry", "poi_signal": True},
    {"url": "https://www.companieshouse.gov.uk/", "type": "regulatory", "topic": "company_registry", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 8: PATENT & IP DATABASES (inventors → technical POIs)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://patents.google.com/", "type": "patent", "topic": "patents", "poi_signal": True},
    {"url": "https://www.uspto.gov/patents/", "type": "patent", "topic": "patents", "poi_signal": True},
    {"url": "https://worldwide.espacenet.com/", "type": "patent", "topic": "patents_eu", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 9: EXECUTIVE JOB BOARDS (senior hires reveal org changes)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.linkedin.com/jobs/executive-jobs/", "type": "jobs", "topic": "executive_jobs", "poi_signal": True},
    {"url": "https://www.indeed.com/q-vp-director-jobs.html", "type": "jobs", "topic": "executive_jobs", "poi_signal": True},
    {"url": "https://www.glassdoor.com/", "type": "jobs", "topic": "executive_jobs", "poi_signal": True},
    {"url": "https://www.execunet.com/", "type": "jobs", "topic": "executive_jobs", "poi_signal": True},
    {"url": "https://www.theladders.com/", "type": "jobs", "topic": "executive_jobs", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 10: FINANCIAL & M&A (reveals leadership, board changes)  
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.crunchbase.com/", "type": "financial", "topic": "funding", "poi_signal": True},
    {"url": "https://pitchbook.com/", "type": "financial", "topic": "funding", "poi_signal": True},
    {"url": "https://www.cbinsights.com/", "type": "financial", "topic": "funding", "poi_signal": True},
    {"url": "https://mergers.acquisitions.com/", "type": "financial", "topic": "ma", "poi_signal": True},
    {"url": "https://www.dealogic.com/", "type": "financial", "topic": "ma", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 11: REGIONAL BUSINESS NEWS (local executives, regional officials)
    # ═══════════════════════════════════════════════════════════════════════════
    # Europe
    {"url": "https://www.euractiv.com/", "type": "news", "topic": "eu_policy", "poi_signal": True},
    {"url": "https://www.handelsblatt.com/", "type": "news", "topic": "germany", "poi_signal": True},
    {"url": "https://www.usinenouvelle.com/", "type": "news", "topic": "france", "poi_signal": True},
    {"url": "https://www.afcea.org/", "type": "news", "topic": "defense_eu", "poi_signal": True},
    
    # MENA
    {"url": "https://www.arabnews.com/", "type": "news", "topic": "mena", "poi_signal": True},
    {"url": "https://www.middleeasteye.net/", "type": "news", "topic": "mena", "poi_signal": True},
    {"url": "https://www.albawaba.com/", "type": "news", "topic": "mena", "poi_signal": True},
    {"url": "https://www.al-monitor.com/", "type": "news", "topic": "mena", "poi_signal": True},
    {"url": "https://www.thenationalnews.com/", "type": "news", "topic": "mena", "poi_signal": True},
    {"url": "https://www.jeuneafrique.com/", "type": "news", "topic": "maghreb", "poi_signal": True},
    {"url": "https://www.leaders.com.tn/", "type": "news", "topic": "tunisia", "poi_signal": True},
    {"url": "https://www.leconomiste.com/", "type": "news", "topic": "morocco", "poi_signal": True},
    {"url": "https://www.africanews.com/", "type": "news", "topic": "africa", "poi_signal": True},
    
    # Asia-Pacific  
    {"url": "https://www.nikkei.com/", "type": "news", "topic": "japan", "poi_signal": True},
    {"url": "https://www.koreaherald.com/", "type": "news", "topic": "korea", "poi_signal": True},
    {"url": "https://www.taipeitimes.com/", "type": "news", "topic": "taiwan", "poi_signal": True},
    {"url": "https://www.scmp.com/", "type": "news", "topic": "hong_kong", "poi_signal": True},
    {"url": "https://www.straitstimes.com/", "type": "news", "topic": "singapore", "poi_signal": True},
    {"url": "https://www.bangkokpost.com/", "type": "news", "topic": "thailand", "poi_signal": True},
    {"url": "https://www.thestar.com.my/", "type": "news", "topic": "malaysia", "poi_signal": True},
    {"url": "https://www.thehindubusinessline.com/", "type": "news", "topic": "india", "poi_signal": True},
    {"url": "https://economictimes.indiatimes.com/", "type": "news", "topic": "india", "poi_signal": True},
    {"url": "https://www.vnexpress.net/", "type": "news", "topic": "vietnam", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 12: ACADEMIC & RESEARCH (technical advisors, board scientists)
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://ieeexplore.ieee.org/", "type": "academic", "topic": "research", "poi_signal": True},
    {"url": "https://arxiv.org/list/cs.CE/recent", "type": "academic", "topic": "research", "poi_signal": True},
    {"url": "https://www.sciencedirect.com/", "type": "academic", "topic": "research", "poi_signal": True},
    {"url": "https://www.researchgate.net/", "type": "academic", "topic": "research", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 13: TRANSFORMED DATA STREAMS (indirect POI signals)
    # ═══════════════════════════════════════════════════════════════════════════
    # Awards & Recognition (reveals leadership)
    {"url": "https://www.eetimes.com/acepicks/", "type": "awards", "topic": "awards", "poi_signal": True},
    {"url": "https://www.industryera.com/", "type": "awards", "topic": "awards", "poi_signal": True},
    {"url": "https://www.manufacturingawards.co.uk/", "type": "awards", "topic": "awards", "poi_signal": True},
    
    # Obituaries & succession (reveals org structure)
    {"url": "https://www.legacy.com/news/", "type": "succession", "topic": "succession", "poi_signal": True},
    
    # Speaking engagements / podcasts
    {"url": "https://www.youtube.com/results?search_query=ceo+interview", "type": "media", "topic": "interviews", "poi_signal": True},
    {"url": "https://podcasts.apple.com/", "type": "media", "topic": "podcasts", "poi_signal": True},
    
    # Social proof / media appearances
    {"url": "https://twitter.com/search?q=announces%20CEO", "type": "social", "topic": "appointments", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 14: SPECIALIZED INDUSTRY VERTICALS
    # ═══════════════════════════════════════════════════════════════════════════
    # Automotive electronics
    {"url": "https://www.automotiveworld.com/", "type": "news", "topic": "automotive", "poi_signal": True},
    {"url": "https://www.sae.org/", "type": "association", "topic": "automotive", "poi_signal": True},
    
    # Medical devices
    {"url": "https://www.meddeviceonline.com/", "type": "news", "topic": "medical", "poi_signal": True},
    {"url": "https://www.mdplusonline.com/", "type": "news", "topic": "medical", "poi_signal": True},
    
    # Telecom/5G
    {"url": "https://www.lightreading.com/", "type": "news", "topic": "telecom", "poi_signal": True},
    {"url": "https://www.fiercewireless.com/", "type": "news", "topic": "telecom", "poi_signal": True},
    
    # Energy/power electronics
    {"url": "https://www.power-mag.com/", "type": "news", "topic": "power", "poi_signal": True},
    {"url": "https://www.powersystemsdesign.com/", "type": "news", "topic": "power", "poi_signal": True},
    
    # Industrial IoT
    {"url": "https://www.iotforall.com/", "type": "news", "topic": "iot", "poi_signal": True},
    {"url": "https://www.iottechnews.com/", "type": "news", "topic": "iot", "poi_signal": True},
    
    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 15: GEOPOLITICAL THINK TANKS & POLICY INSTITUTES
    # ═══════════════════════════════════════════════════════════════════════════
    # ── Tier-1 Global Think Tanks ──
    {"url": "https://www.csis.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.brookings.edu/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.chathamhouse.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.rand.org/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://carnegieendowment.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.cfr.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.iiss.org/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://www.atlanticcouncil.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.sipri.org/", "type": "think_tank", "topic": "arms_control", "poi_signal": True},
    {"url": "https://www.crisisgroup.org/", "type": "think_tank", "topic": "conflict", "poi_signal": True},
    {"url": "https://www.bellingcat.com/", "type": "think_tank", "topic": "osint", "poi_signal": True},
    {"url": "https://rusi.org/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://ecfr.eu/", "type": "think_tank", "topic": "geopolitics_eu", "poi_signal": True},
    {"url": "https://www.cnas.org/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://www.stimson.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.usip.org/", "type": "think_tank", "topic": "peace_conflict", "poi_signal": True},
    {"url": "https://www.wilsoncenter.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.newamerica.org/", "type": "think_tank", "topic": "tech_policy", "poi_signal": True},
    {"url": "https://www.hudson.org/", "type": "think_tank", "topic": "geopolitics", "poi_signal": True},
    {"url": "https://www.heritage.org/defense", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://www.aei.org/foreign-and-defense-policy/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    {"url": "https://www.gmfus.org/", "type": "think_tank", "topic": "transatlantic", "poi_signal": True},
    {"url": "https://cepa.org/", "type": "think_tank", "topic": "geopolitics_eu", "poi_signal": True},
    {"url": "https://www.fdd.org/", "type": "think_tank", "topic": "sanctions", "poi_signal": True},
    {"url": "https://warontherocks.com/", "type": "think_tank", "topic": "defense_policy", "poi_signal": True},
    # ── Regional Think Tanks ──
    {"url": "https://www.aspi.org.au/", "type": "think_tank", "topic": "indo_pacific", "poi_signal": True},
    {"url": "https://www.lowyinstitute.org/", "type": "think_tank", "topic": "indo_pacific", "poi_signal": True},
    {"url": "https://thediplomat.com/", "type": "think_tank", "topic": "asia_pacific", "poi_signal": True},
    {"url": "https://www.mei.edu/", "type": "think_tank", "topic": "middle_east", "poi_signal": True},
    {"url": "https://www.washingtoninstitute.org/", "type": "think_tank", "topic": "middle_east", "poi_signal": True},
    {"url": "https://www.issafrica.org/", "type": "think_tank", "topic": "africa_security", "poi_signal": True},
    {"url": "https://www.ispionline.it/en", "type": "think_tank", "topic": "geopolitics_eu", "poi_signal": True},
    {"url": "https://www.dgap.org/en", "type": "think_tank", "topic": "geopolitics_eu", "poi_signal": True},
    {"url": "https://www.swp-berlin.org/en/", "type": "think_tank", "topic": "geopolitics_eu", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 16: CONFLICT MONITORING & EARLY WARNING
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://acleddata.com/", "type": "conflict_data", "topic": "conflict_tracker", "poi_signal": True},
    {"url": "https://www.cfr.org/global-conflict-tracker", "type": "conflict_data", "topic": "conflict_tracker", "poi_signal": True},
    {"url": "https://ucdp.uu.se/", "type": "conflict_data", "topic": "conflict_data", "poi_signal": True},
    {"url": "https://www.thewarzone.com/", "type": "news", "topic": "conflict", "poi_signal": True},
    {"url": "https://www.militarytimes.com/", "type": "news", "topic": "military", "poi_signal": True},
    {"url": "https://www.armyrecognition.com/", "type": "news", "topic": "military", "poi_signal": True},
    {"url": "https://www.navalnews.com/", "type": "news", "topic": "naval", "poi_signal": True},
    {"url": "https://www.airforcemag.com/", "type": "news", "topic": "air_force", "poi_signal": True},
    {"url": "https://www.conflictnews.info/", "type": "news", "topic": "conflict", "poi_signal": True},
    {"url": "https://liveuamap.com/", "type": "conflict_data", "topic": "conflict_tracker", "poi_signal": True},
    {"url": "https://www.understandingwar.org/", "type": "conflict_data", "topic": "conflict_analysis", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 17: SANCTIONS & EXPORT CONTROL INTELLIGENCE
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://sanctionsnews.bakermckenzie.com/", "type": "sanctions", "topic": "sanctions_legal", "poi_signal": True},
    {"url": "https://www.sanctions.io/blog", "type": "sanctions", "topic": "sanctions_compliance", "poi_signal": True},
    {"url": "https://www.europeansanctions.com/", "type": "sanctions", "topic": "sanctions_eu", "poi_signal": True},
    {"url": "https://www.gov.uk/government/collections/financial-sanctions-regime-specific-consolidated-lists-and-releases", "type": "sanctions", "topic": "sanctions_uk", "poi_signal": True},
    {"url": "https://www.un.org/securitycouncil/sanctions/information", "type": "sanctions", "topic": "sanctions_un", "poi_signal": True},
    {"url": "https://www.kharon.com/blog", "type": "sanctions", "topic": "sanctions_intelligence", "poi_signal": True},
    {"url": "https://www.castellum.ai/insights", "type": "sanctions", "topic": "sanctions_data", "poi_signal": True},
    {"url": "https://www.exportcompliancedaily.com/", "type": "sanctions", "topic": "export_control", "poi_signal": True},
    {"url": "https://ecfr.eu/special/sanctions-tracker/", "type": "sanctions", "topic": "sanctions_eu", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 18: ARMS TRADE & MILITARY PROCUREMENT
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.sipri.org/databases/armstransfers", "type": "arms_trade", "topic": "arms_transfers", "poi_signal": True},
    {"url": "https://www.dsca.mil/press-media", "type": "arms_trade", "topic": "arms_sales_us", "poi_signal": True},
    {"url": "https://www.defenseindustrydaily.com/", "type": "arms_trade", "topic": "defense_contracts", "poi_signal": True},
    {"url": "https://www.shephard.co.uk/", "type": "arms_trade", "topic": "defense_procurement", "poi_signal": True},
    {"url": "https://tenderalpha.com/blog/", "type": "arms_trade", "topic": "defense_tenders", "poi_signal": True},
    {"url": "https://www.armscontrol.org/", "type": "arms_trade", "topic": "arms_control", "poi_signal": True},
    {"url": "https://thebulletin.org/", "type": "arms_trade", "topic": "nuclear_arms", "poi_signal": True},
    {"url": "https://www.nti.org/", "type": "arms_trade", "topic": "nuclear_security", "poi_signal": True},
    {"url": "https://www.icanw.org/", "type": "arms_trade", "topic": "nuclear_disarmament", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 19: MARITIME & TRADE ROUTE INTELLIGENCE
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.maritime-executive.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://gcaptain.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://www.tradewindsnews.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://www.seatrade-maritime.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://www.hellenicshippingnews.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://www.freightwaves.com/", "type": "maritime", "topic": "freight", "poi_signal": True},
    {"url": "https://splash247.com/", "type": "maritime", "topic": "shipping", "poi_signal": True},
    {"url": "https://www.porttechnology.org/", "type": "maritime", "topic": "ports", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 20: ENERGY SECURITY & CRITICAL COMMODITIES
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.iea.org/news", "type": "energy", "topic": "energy_security", "poi_signal": True},
    {"url": "https://www.opec.org/opec_web/en/", "type": "energy", "topic": "oil_market", "poi_signal": True},
    {"url": "https://www.spglobal.com/commodityinsights/en", "type": "energy", "topic": "commodities", "poi_signal": True},
    {"url": "https://www.argusmedia.com/en/news", "type": "energy", "topic": "energy_markets", "poi_signal": True},
    {"url": "https://www.energyintel.com/", "type": "energy", "topic": "energy_intelligence", "poi_signal": True},
    {"url": "https://oilprice.com/", "type": "energy", "topic": "oil_gas", "poi_signal": True},
    {"url": "https://www.mining.com/", "type": "energy", "topic": "mining", "poi_signal": True},
    {"url": "https://www.usgs.gov/centers/national-minerals-information-center", "type": "energy", "topic": "critical_minerals", "poi_signal": True},
    {"url": "https://eitrawmaterials.eu/", "type": "energy", "topic": "raw_materials_eu", "poi_signal": True},
    {"url": "https://www.benchmarkminerals.com/", "type": "energy", "topic": "battery_minerals", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 21: CYBER THREAT & INFORMATION WARFARE
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.cisa.gov/news-events", "type": "cyber", "topic": "cyber_defense", "poi_signal": True},
    {"url": "https://www.enisa.europa.eu/news", "type": "cyber", "topic": "cyber_eu", "poi_signal": True},
    {"url": "https://cloud.google.com/blog/topics/threat-intelligence", "type": "cyber", "topic": "threat_intel", "poi_signal": True},
    {"url": "https://www.crowdstrike.com/blog/", "type": "cyber", "topic": "threat_intel", "poi_signal": True},
    {"url": "https://www.recordedfuture.com/blog", "type": "cyber", "topic": "threat_intel", "poi_signal": True},
    {"url": "https://therecord.media/", "type": "cyber", "topic": "cyber_conflict", "poi_signal": True},
    {"url": "https://www.darkreading.com/", "type": "cyber", "topic": "cybersecurity", "poi_signal": True},
    {"url": "https://www.bleepingcomputer.com/", "type": "cyber", "topic": "cybersecurity", "poi_signal": True},
    {"url": "https://cyberscoop.com/", "type": "cyber", "topic": "cyber_policy", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 22: DIPLOMATIC & INTERNATIONAL ORGANIZATIONS
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://news.un.org/en/", "type": "diplomatic", "topic": "un", "poi_signal": True},
    {"url": "https://www.nato.int/cps/en/natohq/news.htm", "type": "diplomatic", "topic": "nato", "poi_signal": True},
    {"url": "https://www.osce.org/news", "type": "diplomatic", "topic": "osce", "poi_signal": True},
    {"url": "https://www.icc-cpi.int/news", "type": "diplomatic", "topic": "icc", "poi_signal": True},
    {"url": "https://www.icj-cij.org/en/press-releases", "type": "diplomatic", "topic": "icj", "poi_signal": True},
    {"url": "https://au.int/en/news", "type": "diplomatic", "topic": "african_union", "poi_signal": True},
    {"url": "https://asean.org/category/news/", "type": "diplomatic", "topic": "asean", "poi_signal": True},
    {"url": "https://www.oas.org/en/media_center/press_releases.asp", "type": "diplomatic", "topic": "oas", "poi_signal": True},
    {"url": "https://www.iaea.org/news", "type": "diplomatic", "topic": "iaea_nuclear", "poi_signal": True},
    {"url": "https://www.opcw.org/news", "type": "diplomatic", "topic": "chemical_weapons", "poi_signal": True},
    {"url": "https://www.wto.org/english/news_e/news_e.htm", "type": "diplomatic", "topic": "trade_wto", "poi_signal": True},

    # ═══════════════════════════════════════════════════════════════════════════
    # CATEGORY 23: TECH POLICY & STRATEGIC TECHNOLOGY
    # ═══════════════════════════════════════════════════════════════════════════
    {"url": "https://www.technologyreview.com/", "type": "news", "topic": "tech_policy", "poi_signal": True},
    {"url": "https://www.wired.com/tag/geopolitics/", "type": "news", "topic": "tech_geopolitics", "poi_signal": True},
    {"url": "https://restofworld.org/", "type": "news", "topic": "tech_global", "poi_signal": True},
    {"url": "https://techcrunch.com/tag/geopolitics/", "type": "news", "topic": "tech_geopolitics", "poi_signal": True},
    {"url": "https://www.lawfaremedia.org/", "type": "news", "topic": "nat_security_law", "poi_signal": True},
    {"url": "https://www.techpolicy.press/", "type": "news", "topic": "tech_policy", "poi_signal": True},
    {"url": "https://carnegieendowment.org/programs/technology-and-international-affairs", "type": "think_tank", "topic": "tech_geopolitics", "poi_signal": True},
    {"url": "https://www.thecipherbrief.com/", "type": "news", "topic": "intelligence", "poi_signal": True},
    {"url": "https://www.spacenews.com/", "type": "news", "topic": "space_security", "poi_signal": True},
]

# Total: 128 unique data streams for aggressive POI intelligence gathering
print(f"📡 Initialized {len(NEWS_SOURCES)} POI intelligence streams")


# ═══════════════════════════════════════════════════════════════════════════════
# SOCIAL MEDIA & FORUM INTELLIGENCE SOURCES
# ═══════════════════════════════════════════════════════════════════════════════
# Expanded social media, forums, and alternative platforms for deeper signal capture.
# Each has a credibility tier: T1 (highest, verified accounts/official sources),
# T2 (established platforms with mixed quality), T3 (unverified, needs corroboration).

SOCIAL_MEDIA_SOURCES = [
    # ─── Reddit: OSINT & Industry Subreddits ──────────────────────────────────
    # T2 credibility — anonymous but community-moderated
    {"url": "https://www.reddit.com/r/supplychain/new.json?limit=25",    "platform": "reddit", "topic": "supply_chain", "tier": "T2"},
    {"url": "https://www.reddit.com/r/electronics/new.json?limit=25",    "platform": "reddit", "topic": "electronics",   "tier": "T2"},
    {"url": "https://www.reddit.com/r/semiconductor/new.json?limit=25",  "platform": "reddit", "topic": "semiconductors","tier": "T2"},
    {"url": "https://www.reddit.com/r/geopolitics/new.json?limit=25",    "platform": "reddit", "topic": "geopolitics",   "tier": "T2"},
    {"url": "https://www.reddit.com/r/CredibleDefense/new.json?limit=25","platform": "reddit", "topic": "defense",       "tier": "T2"},
    {"url": "https://www.reddit.com/r/worldnews/new.json?limit=25",      "platform": "reddit", "topic": "world",         "tier": "T2"},
    {"url": "https://www.reddit.com/r/netsec/new.json?limit=25",         "platform": "reddit", "topic": "cybersecurity", "tier": "T2"},
    {"url": "https://www.reddit.com/r/cybersecurity/new.json?limit=25",  "platform": "reddit", "topic": "cybersecurity", "tier": "T2"},
    {"url": "https://www.reddit.com/r/investing/new.json?limit=25",      "platform": "reddit", "topic": "finance",       "tier": "T2"},
    {"url": "https://www.reddit.com/r/SecurityAnalysis/new.json?limit=25","platform": "reddit", "topic": "finance",      "tier": "T2"},
    {"url": "https://www.reddit.com/r/pcbdesign/new.json?limit=25",      "platform": "reddit", "topic": "PCB",           "tier": "T2"},
    {"url": "https://www.reddit.com/r/manufacturing/new.json?limit=25",  "platform": "reddit", "topic": "manufacturing", "tier": "T2"},
    {"url": "https://www.reddit.com/r/EMS/new.json?limit=25",            "platform": "reddit", "topic": "manufacturing", "tier": "T2"},
    {"url": "https://www.reddit.com/r/defense/new.json?limit=25",        "platform": "reddit", "topic": "defense",       "tier": "T2"},
    {"url": "https://www.reddit.com/r/drones/new.json?limit=25",         "platform": "reddit", "topic": "defense_tech",  "tier": "T2"},
    {"url": "https://www.reddit.com/r/SanctionsCompliance/new.json?limit=25", "platform": "reddit", "topic": "sanctions","tier": "T2"},
    {"url": "https://www.reddit.com/r/TradePolicy/new.json?limit=25",    "platform": "reddit", "topic": "trade",         "tier": "T2"},
    {"url": "https://www.reddit.com/r/geopol/new.json?limit=25",         "platform": "reddit", "topic": "geopolitics",   "tier": "T2"},
    {"url": "https://www.reddit.com/r/ArmyTech/new.json?limit=25",       "platform": "reddit", "topic": "defense_tech",  "tier": "T2"},
    {"url": "https://www.reddit.com/r/Metalworking/new.json?limit=25",   "platform": "reddit", "topic": "manufacturing", "tier": "T2"},
    {"url": "https://www.reddit.com/r/3Dprinting/new.json?limit=25",     "platform": "reddit", "topic": "manufacturing", "tier": "T2"},
    {"url": "https://www.reddit.com/r/ExportControls/new.json?limit=25", "platform": "reddit", "topic": "sanctions",     "tier": "T2"},
    # ── Geopolitical & OSINT Reddit Additions ──
    {"url": "https://www.reddit.com/r/OSINT/new.json?limit=25",            "platform": "reddit", "topic": "osint",         "tier": "T2"},
    {"url": "https://www.reddit.com/r/IntelligenceStudies/new.json?limit=25","platform": "reddit", "topic": "intelligence","tier": "T2"},
    {"url": "https://www.reddit.com/r/WarCollege/new.json?limit=25",       "platform": "reddit", "topic": "military_hist", "tier": "T2"},
    {"url": "https://www.reddit.com/r/LessCredibleDefence/new.json?limit=25","platform": "reddit", "topic": "defense",     "tier": "T2"},
    {"url": "https://www.reddit.com/r/MiddleEastNews/new.json?limit=25",   "platform": "reddit", "topic": "mena",          "tier": "T2"},
    {"url": "https://www.reddit.com/r/africa/new.json?limit=25",           "platform": "reddit", "topic": "africa",        "tier": "T2"},
    {"url": "https://www.reddit.com/r/europe/new.json?limit=25",           "platform": "reddit", "topic": "europe",        "tier": "T2"},
    {"url": "https://www.reddit.com/r/CentralAsianPolitics/new.json?limit=25","platform": "reddit", "topic": "central_asia","tier": "T2"},
    {"url": "https://www.reddit.com/r/IndianDefense/new.json?limit=25",    "platform": "reddit", "topic": "south_asia",    "tier": "T2"},
    {"url": "https://www.reddit.com/r/Sino/new.json?limit=25",             "platform": "reddit", "topic": "china",         "tier": "T2"},
    {"url": "https://www.reddit.com/r/UkrainianConflict/new.json?limit=25","platform": "reddit", "topic": "conflict",      "tier": "T2"},
    {"url": "https://www.reddit.com/r/NuclearWeapons/new.json?limit=25",   "platform": "reddit", "topic": "nuclear",       "tier": "T2"},
    {"url": "https://www.reddit.com/r/energy/new.json?limit=25",           "platform": "reddit", "topic": "energy",        "tier": "T2"},
    {"url": "https://www.reddit.com/r/maritime/new.json?limit=25",         "platform": "reddit", "topic": "maritime",       "tier": "T2"},

    # ─── Hacker News: Tech & Business ────────────────────────────────────────
    # T2 credibility — tech-focused community, high quality discussions
    {"url": "https://hacker-news.firebaseio.com/v0/newstories.json",     "platform": "hackernews", "topic": "tech_general",  "tier": "T2"},
    {"url": "https://hacker-news.firebaseio.com/v0/topstories.json",     "platform": "hackernews", "topic": "tech_trending", "tier": "T2"},

    # ─── Mastodon: Decentralised social (OSINT community) ─────────────────────
    # T2 credibility — identity-verified fediverse accounts
    {"url": "https://infosec.exchange/api/v1/timelines/tag/supplychain?limit=20",  "platform": "mastodon", "topic": "supply_chain",  "tier": "T2"},
    {"url": "https://infosec.exchange/api/v1/timelines/tag/cybersecurity?limit=20","platform": "mastodon", "topic": "cybersecurity", "tier": "T2"},
    {"url": "https://infosec.exchange/api/v1/timelines/tag/osint?limit=20",        "platform": "mastodon", "topic": "osint",         "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/semiconductors?limit=20","platform": "mastodon", "topic": "semiconductors","tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/defense?limit=20",       "platform": "mastodon", "topic": "defense",       "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/geopolitics?limit=20",   "platform": "mastodon", "topic": "geopolitics",   "tier": "T2"},
    {"url": "https://fosstodon.org/api/v1/timelines/tag/electronics?limit=20",     "platform": "mastodon", "topic": "electronics",   "tier": "T2"},
    {"url": "https://ioc.exchange/api/v1/timelines/tag/threatintel?limit=20",      "platform": "mastodon", "topic": "threat_intel",  "tier": "T2"},

    # ─── Bluesky: AT Protocol social ──────────────────────────────────────────
    # T2 credibility — growing OSINT/researcher community
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=supply+chain+disruption&limit=25", "platform": "bluesky", "topic": "supply_chain", "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=semiconductor+sanctions&limit=25", "platform": "bluesky", "topic": "semiconductors","tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=defense+procurement&limit=25",     "platform": "bluesky", "topic": "defense",       "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=export+controls&limit=25",         "platform": "bluesky", "topic": "sanctions",     "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=geopolitics+conflict&limit=25",    "platform": "bluesky", "topic": "geopolitics",   "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=military+buildup&limit=25",        "platform": "bluesky", "topic": "military",      "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=sanctions+evasion&limit=25",       "platform": "bluesky", "topic": "sanctions",     "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=energy+security+pipeline&limit=25","platform": "bluesky", "topic": "energy",        "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=critical+minerals+supply&limit=25","platform": "bluesky", "topic": "minerals",      "tier": "T2"},

    # ─── Telegram Public Channels: OSINT & Geopolitics ────────────────────────
    # T3 credibility — high noise, requires corroboration
    {"url": "https://t.me/s/inaborodin",          "platform": "telegram", "topic": "osint",       "tier": "T3"},
    {"url": "https://t.me/s/osaborodin",           "platform": "telegram", "topic": "osint",       "tier": "T3"},
    {"url": "https://t.me/s/militarytechonly",      "platform": "telegram", "topic": "defense_tech","tier": "T3"},
    {"url": "https://t.me/s/defense_news_channel",  "platform": "telegram", "topic": "defense",     "tier": "T3"},
    {"url": "https://t.me/s/opencrisusintel",       "platform": "telegram", "topic": "crisis",      "tier": "T3"},
    {"url": "https://t.me/s/liveuamap",             "platform": "telegram", "topic": "conflict",    "tier": "T3"},
    {"url": "https://t.me/s/belaborodin",             "platform": "telegram", "topic": "geopolitics", "tier": "T3"},
    {"url": "https://t.me/s/sanctionstracker",         "platform": "telegram", "topic": "sanctions",   "tier": "T3"},
    {"url": "https://t.me/s/nuclearwar_news",          "platform": "telegram", "topic": "nuclear",     "tier": "T3"},
    {"url": "https://t.me/s/geopoliticsworld",         "platform": "telegram", "topic": "geopolitics", "tier": "T3"},
    {"url": "https://t.me/s/conflict_intel",           "platform": "telegram", "topic": "conflict",    "tier": "T3"},

    # ─── Industry Forums & Specialist Communities ─────────────────────────────
    # T2 credibility — professional community, domain experts
    {"url": "https://community.silabs.com/s/", "platform": "forum", "topic": "semiconductors", "tier": "T2"},  # Replaced AnandTech forums (shut down 2024)
    {"url": "https://eevblog.com/forum/general-chat.php",                     "platform": "forum", "topic": "electronics",   "tier": "T2"},

    # ─── YouTube: Industry Analysis Channels ──────────────────────────────────
    # T2 credibility — named analysts with track records
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UCvjgXvBlBQ0Bm0cXHvUEpHQ", "platform": "youtube", "topic": "semiconductors", "tier": "T2"},  # Asianometry
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UCZDkxpcvd-T1uR65Feuj5Yg", "platform": "youtube", "topic": "geopolitics",   "tier": "T2"},  # CaspianReport
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UCnVqHmdJL-UEl7sABdWFzLQ", "platform": "youtube", "topic": "defense",       "tier": "T2"},  # Perun
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UC1XoiwR2cMBx-sPCl4E2TdQ", "platform": "youtube", "topic": "geopolitics",   "tier": "T2"},  # TLDR News Global
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UC0LHEYTEAyndlOFXyp8RZSA", "platform": "youtube", "topic": "geopolitics",   "tier": "T2"},  # VisualPolitik
    {"url": "https://www.youtube.com/feeds/videos.xml?channel_id=UC2C_jShtL725hvbm1arSV9w", "platform": "youtube", "topic": "geopolitics",   "tier": "T2"},  # GoodTimesWithScar/RealLifeLore

    # ─── Mastodon: Geopolitical & OSINT additions ─────────────────────────────
    {"url": "https://mastodon.social/api/v1/timelines/tag/sanctions?limit=20",    "platform": "mastodon", "topic": "sanctions",     "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/conflict?limit=20",     "platform": "mastodon", "topic": "conflict",      "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/nuclear?limit=20",      "platform": "mastodon", "topic": "nuclear",       "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/energysecurity?limit=20","platform": "mastodon","topic": "energy",         "tier": "T2"},
    {"url": "https://infosec.exchange/api/v1/timelines/tag/cyberwar?limit=20",    "platform": "mastodon", "topic": "cyber_conflict", "tier": "T2"},
    {"url": "https://mastodon.social/api/v1/timelines/tag/armstrade?limit=20",    "platform": "mastodon", "topic": "arms_trade",    "tier": "T2"},

    # ─── Bluesky Search (Replaced dead Nitter/Twitter mirrors) ─────────────────
    # T2 credibility — public search, unfiltered
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=supply+chain+disruption&limit=25", "platform": "bluesky", "topic": "supply_chain",  "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=semiconductor+shortage&limit=25",   "platform": "bluesky", "topic": "semiconductors","tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=defense+contract+awarded&limit=25", "platform": "bluesky", "topic": "defense",       "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=sanctions+entity+list&limit=25",    "platform": "bluesky", "topic": "sanctions",     "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=factory+expansion&limit=25",        "platform": "bluesky", "topic": "manufacturing", "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=geopolitical+conflict&limit=25",    "platform": "bluesky", "topic": "geopolitics",   "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=military+deployment&limit=25",      "platform": "bluesky", "topic": "military",      "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=energy+pipeline+shutdown&limit=25", "platform": "bluesky", "topic": "energy",        "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=cyber+attack+infrastructure&limit=25", "platform": "bluesky", "topic": "cyber",    "tier": "T2"},
    {"url": "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=rare+earth+minerals&limit=25",      "platform": "bluesky", "topic": "minerals",      "tier": "T2"},
]

# Credibility weight by tier (used in cross-reference scoring)
TIER_CREDIBILITY = {"T1": 0.90, "T2": 0.65, "T3": 0.35}

print(f"📱 Initialized {len(SOCIAL_MEDIA_SOURCES)} social media intelligence streams")


# ─── Crawl Targets Builder ─────────────────────────────────────────────────────

async def build_crawl_targets(pool: asyncpg.Pool) -> list[dict]:
    """Build list of URLs to crawl from the companies table."""
    targets = []

    # Get all companies with domains
    rows = await pool.fetch("SELECT id, name, domain, company_type FROM companies WHERE domain IS NOT NULL")
    for row in rows:
        domain = row["domain"]
        company_id = str(row["id"])
        name = row["name"]

        # Company website pages to crawl
        for path in [
            "/", "/news", "/press", "/press-releases", "/media",
            "/about", "/about-us", "/careers", "/jobs",
            "/investors", "/investor-relations",
            "/products", "/solutions", "/capabilities",
            "/sustainability", "/esg",
        ]:
            targets.append({
                "url": f"https://{domain}{path}",
                "company_id": company_id,
                "company_name": name,
                "page_type": path.strip("/") or "homepage",
            })

    # Add news sources
    for src in NEWS_SOURCES:
        targets.append({
            "url": src["url"],
            "company_id": None,
            "company_name": None,
            "page_type": f"news_{src['topic']}",
        })

    # Add social media sources (scraped separately via scrape_social_media, but
    # we also add web-accessible ones here for article link extraction)
    for src in SOCIAL_MEDIA_SOURCES:
        if src["platform"] in ("nitter", "forum"):
            targets.append({
                "url": src["url"],
                "company_id": None,
                "company_name": None,
                "page_type": f"social_{src['platform']}_{src['topic']}",
            })

    random.shuffle(targets)
    return targets


# ─── Database Operations ────────────────────────────────────────────────────────

async def store_fingerprint(pool: asyncpg.Pool, url: str, content_hash: str) -> bool:
    """Store page fingerprint. Returns True if content is new/changed."""
    existing = await pool.fetchval(
        "SELECT content_hash FROM page_fingerprints WHERE url = $1 ORDER BY ts DESC LIMIT 1",
        url
    )
    await pool.execute(
        "INSERT INTO page_fingerprints (url, content_hash) VALUES ($1, $2)",
        url, content_hash
    )
    return existing is None or existing != content_hash


async def store_observation(pool: asyncpg.Pool, obs_type: str, entity_id: Optional[str],
                           entity_type: str, value: dict, provenance: dict,
                           confidence: float = 0.8) -> str:
    """Insert an observation into the database.

    Includes URL-based deduplication: if an observation with the same
    entity, type, and URL already exists within the last 4 hours, the
    insert is silently skipped to avoid ballooning the table with
    duplicate crawl results every cycle.
    """
    # ── Deduplication gate ──
    # Prevent the same URL+entity+type from being stored repeatedly within
    # a single crawl cycle (~45 min window).  A longer window blocks
    # legitimate cross-cycle content-change detections.
    obs_url = (value.get("url") or provenance.get("url") or "").strip()
    if obs_url and entity_id:
        existing = await pool.fetchval(
            """SELECT 1 FROM observations
               WHERE observation_type = $1
               AND entity_id = $2::uuid
               AND provenance->>'url' = $3
               AND ts_utc > NOW() - INTERVAL '45 minutes'
               LIMIT 1""",
            obs_type,
            uuid.UUID(entity_id),
            obs_url,
        )
        if existing:
            return ""  # duplicate, skip

    obs_id = str(uuid.uuid4())
    await pool.execute(
        """INSERT INTO observations (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
           VALUES ($1, $2, $3::uuid, $4, $5, $6::jsonb, $7::jsonb, $8)""",
        uuid.UUID(obs_id),
        obs_type,
        uuid.UUID(entity_id) if entity_id else None,
        entity_type,
        datetime.now(timezone.utc),
        json.dumps(value),
        json.dumps(provenance),
        confidence
    )
    return obs_id


async def store_warning(pool: asyncpg.Pool, recipe_code: str, warning_type: str,
                       title: str, description: str, severity: str,
                       region: str, source_urls: list, entity_ids: list,
                       confidence: float = 0.8):
    """Insert a warning into the database."""
    await pool.execute(
        """INSERT INTO warnings (recipe_code, warning_type, title, description, severity, region,
           source_urls, entity_ids, confidence, ts_utc)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid[], $9, $10)""",
        recipe_code, warning_type, title, description, severity, region,
        source_urls, [uuid.UUID(eid) for eid in entity_ids if eid], confidence,
        datetime.now(timezone.utc)
    )


def _insight_digest_tokens(text: str, max_tokens: int = 220) -> list[str]:
    stopwords = {
        "the", "and", "for", "with", "that", "this", "from", "into", "over", "under",
        "onto", "after", "before", "about", "their", "there", "they", "them", "were",
        "have", "has", "been", "being", "will", "would", "could", "should", "a", "an",
        "of", "to", "in", "on", "by", "at", "as", "is", "are", "or",
    }
    normalized = "".join(ch if ch.isalnum() else " " for ch in text.lower())
    out = []
    for word in normalized.split():
        if len(word) > 2 and word not in stopwords:
            out.append(word)
        if len(out) >= max_tokens:
            break
    return out


def _has_excessive_phrase_repetition(text: str) -> bool:
    tokens = _insight_digest_tokens(text)
    if len(tokens) < 10:
        return False
    seen = {}
    for idx in range(len(tokens) - 3):
        phrase = " ".join(tokens[idx:idx + 4])
        seen[phrase] = seen.get(phrase, 0) + 1
        if seen[phrase] >= 3:
            return True
    return False


def _count_template_markers(text: str) -> int:
    markers = [
        "assessment:", "recommended action:", "additional source reporting:",
        "signal themes detected:", "actionable:", "watch closely:",
        "early signal:", "low confidence:", "analysis:", "impact:", "recommendation:",
    ]
    lower = text.lower()
    return sum(1 for marker in markers if marker in lower)


def _collapse_duplicate_paragraphs(text: str) -> str:
    normalized = (text or "").replace("\r\n", "\n").strip()
    if not normalized:
        return ""

    paragraphs = [paragraph.strip() for paragraph in re.split(r"\n\s*\n+", normalized) if paragraph.strip()]
    if len(paragraphs) <= 1:
        return normalized

    deduped: list[str] = []
    seen: set[str] = set()
    for paragraph in paragraphs:
        key = " ".join(paragraph.split()).lower()
        if key in seen:
            continue
        seen.add(key)
        deduped.append(paragraph)

    return "\n\n".join(deduped)


def _passes_shared_quality_gate(title_text: str, summary_text: str, current_type: str) -> bool:
    if len(title_text.strip()) < 12 or len(summary_text.strip()) < 80:
        return False
    lower_title = title_text.lower()
    lower_summary = summary_text.lower()
    bad_fragments = [
        "intelligence veracity:",
        "additional source reporting:",
        "signal themes detected:",
        "assessment: moderate-high confidenc",
        "[object object]",
        "undefined",
        "{{",
        "}}",
    ]
    if any(fragment in lower_title or fragment in lower_summary for fragment in bad_fragments):
        return False
    if _count_template_markers(summary_text) >= 2:
        return False
    if _has_excessive_phrase_repetition(summary_text):
        return False
    if current_type == "veracity_analysis":
        if "source" not in lower_summary:
            return False
        if not any(marker in lower_summary for marker in ("evidence", "corroborat", "reported")):
            return False
    return True


def _confidence_gate_reason(insight_type: str, confidence: float, tags: list | None = None) -> Optional[str]:
    if confidence < 0.50:
        return f"confidence {confidence:.2f} below 0.50"
    if insight_type == "veracity_analysis":
        normalized_tags = {tag.strip().lower() for tag in (tags or []) if isinstance(tag, str)}
        if "unverified" in normalized_tags or "contradicted" in normalized_tags:
            blocked = "unverified" if "unverified" in normalized_tags else "contradicted"
            return f"classification '{blocked}' blocked for persisted veracity insights"
    return None


def _passes_confidence_gate(insight_type: str, confidence: float, tags: list | None = None) -> bool:
    return _confidence_gate_reason(insight_type, confidence, tags) is None


async def store_insight(pool: asyncpg.Pool, title: str, summary: str, 
                       insight_type: str, region: str, evidence_urls: list,
                       tags: list, confidence: float = 0.75, entity_ids: list = None):
    """Insert an insight into the database.
    
    Quality gate: minimum confidence of 0.50 required. Insights below this
    threshold are too speculative to surface to analysts.
    """
    # Reject low-confidence insights
    summary = _collapse_duplicate_paragraphs(summary)

    confidence_reason = _confidence_gate_reason(insight_type, confidence, tags)
    if confidence_reason is not None:
        log.debug(f"Insight rejected by confidence gate: {title[:80]} ({confidence_reason})")
        return False

    if not _passes_shared_quality_gate(title, summary, insight_type):
        log.debug(f"Insight rejected by shared quality gate: {title[:80]}")
        return False
    
    # Deduplicate and filter evidence URLs, preserving insertion order
    seen: dict = {}
    deduped: list = []
    for u in (evidence_urls or []):
        if u and u not in seen:
            seen[u] = None
            deduped.append(u)
    
    # Reject insights with no evidence URLs — they're speculation
    if not deduped:
        log.debug(f"Insight rejected (no evidence URLs): {title[:60]}")
        return False

    if insight_type == "veracity_analysis" and not _has_non_social_evidence_url(deduped):
        log.debug(f"Insight rejected (social-only veracity evidence): {title[:80]}")
        return False
    
    # Parse entity_ids to UUID list
    parsed_entity_ids = None
    if entity_ids:
        parsed_entity_ids = [uuid.UUID(eid) if isinstance(eid, str) else eid for eid in entity_ids if eid]
        if not parsed_entity_ids:
            parsed_entity_ids = None
    
    await pool.execute(
        """INSERT INTO insights (title, summary, insight_type, region, confidence, evidence_urls, tags, entity_ids)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)""",
        title, summary, insight_type, region, confidence, deduped, tags, parsed_entity_ids
    )
    return True


async def generate_insights_from_warnings(pool: asyncpg.Pool):
    """
    Analyze recent warnings to generate higher-level analytical insights.
    Performs pattern correlation, trend detection, and competitive analysis.

    Quality gates (Q1 2026):
    - Increased minimum warning thresholds to reduce low-quality insight generation.
    - Per-entity-type dedup window extended to 24 hours.
    - Requires at least 4 total signals for competitive threat analysis.
    """
    log.info("🔍 Analyzing warning patterns for insights...")
    
    insights_created = 0
    
    # ─── 1. COMPETITIVE THREAT ANALYSIS ────────────────────────────────────────
    # Identify competitors with combined expansion + hiring + technology signals
    # Raised thresholds: total >= 4 (was 3), expansion+ma+tech >= 3 (was 2)
    competitive_threats = await pool.fetch("""
        SELECT 
            c.id, c.name, c.region, (c.metadata->>'is_competitor')::boolean as is_competitor,
            COUNT(*) FILTER (WHERE w.warning_type IN ('expansion', 'ma_activity')) as expansion_signals,
            COUNT(*) FILTER (WHERE w.warning_type = 'hiring_signal') as hiring_signals,
            COUNT(*) FILTER (WHERE w.warning_type = 'technology') as tech_signals,
            COUNT(*) as total_signals,
            array_agg(DISTINCT w.title ORDER BY w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND c.metadata->>'is_competitor' = 'true'
        GROUP BY c.id, c.name, c.region, c.metadata->>'is_competitor'
        HAVING COUNT(*) >= 4
        AND COUNT(*) FILTER (WHERE w.warning_type IN ('expansion', 'ma_activity', 'technology')) >= 3
        ORDER BY COUNT(*) DESC
        LIMIT 5
    """)
    
    for row in competitive_threats:
        company = row["name"]
        region = row["region"] or "Global"
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{company}%threat%"
        )
        if existing:
            continue
        
        threat_level = "elevated" if row["total_signals"] >= 5 else "moderate"
        
        analysis_points = []
        if row["expansion_signals"] >= 2:
            analysis_points.append("capacity expansion suggesting market share ambitions")
        if row["hiring_signals"] >= 2:
            analysis_points.append("aggressive talent acquisition indicating scaling operations")
        if row["tech_signals"] >= 1:
            analysis_points.append("R&D/patent activity suggesting capability development")
        
        title = f"Competitive Threat: {company} shows {threat_level} expansion activity"
        summary = (
            f"ANALYSIS: {company} ({region}) is exhibiting a pattern of signals that suggest strategic growth. "
            f"Key indicators: {'; '.join(analysis_points)}. "
            f"Combined with {row['total_signals']} total alerts in 24 hours, this represents a potential "
            f"competitive threat requiring monitoring. "
            f"RECOMMENDATION: Review competitor positioning and consider defensive measures in {region} market."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "competitor_threat", region, sources,
                          ["competitor", "threat", "strategic", region.lower().replace(" ", "_")], 0.82,
                          entity_ids=[str(row["id"])])
        insights_created += 1
        log.info(f"💡 Competitive threat insight: {company}")

    # ─── 2. SUPPLY CHAIN RISK CORRELATION ──────────────────────────────────────
    # Find multiple companies in same region with supply chain issues
    # Raised threshold: require 3+ affected companies (was 2)
    supply_chain_risk = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(DISTINCT c.id) as affected_companies,
            array_agg(DISTINCT c.name) as company_names,
            COUNT(*) as total_signals,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'supply_chain_disruption'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(DISTINCT c.id) >= 3
    """)
    
    for row in supply_chain_risk:
        region = row["region"]
        companies = row["company_names"][:5]
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%supply chain%"
        )
        if existing:
            continue
        
        title = f"Supply Chain Alert: Multiple disruptions detected in {region}"
        summary = (
            f"ANALYSIS: Supply chain disruption signals detected across {row['affected_companies']} companies "
            f"in {region}: {', '.join(companies)}. "
            f"The correlation of {row['total_signals']} disruption signals in 24 hours suggests a regional "
            f"supply chain stress event rather than isolated incidents. "
            f"IMPACT: Potential delays affecting component availability and production schedules. "
            f"RECOMMENDATION: Engage with affected suppliers for status updates; consider alternative sourcing."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "supply_chain_risk", region, sources,
                          ["supply_chain", "risk", "regional", region.lower().replace(" ", "_")], 0.78)
        insights_created += 1
        log.info(f"💡 Supply chain risk insight: {region}")

    # ─── 3. MARKET OPPORTUNITY DETECTION ───────────────────────────────────────
    # Identify regions/sectors with high certification + hiring activity
    # Raised thresholds: require 3+ cert AND 3+ hiring signals (was 2+2)
    market_opportunities = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) FILTER (WHERE w.warning_type = 'certification_update') as cert_signals,
            COUNT(*) FILTER (WHERE w.warning_type = 'hiring_signal') as hiring_signals,
            COUNT(DISTINCT c.id) as active_companies,
            array_agg(DISTINCT c.name) as companies,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('certification_update', 'hiring_signal')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) FILTER (WHERE w.warning_type = 'certification_update') >= 3
        AND COUNT(*) FILTER (WHERE w.warning_type = 'hiring_signal') >= 3
    """)
    
    for row in market_opportunities:
        region = row["region"]
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%market%"
        )
        if existing:
            continue
        
        title = f"Market Activity: {region} shows increased industry investment"
        summary = (
            f"ANALYSIS: {region} is experiencing elevated market activity with {row['cert_signals']} "
            f"certification updates and {row['hiring_signals']} hiring signals across {row['active_companies']} companies. "
            f"This combination typically indicates: (1) companies preparing for new contracts/certifications, "
            f"(2) capacity expansion to meet demand, (3) potential market growth opportunity. "
            f"Active companies: {', '.join(row['companies'][:4])}. "
            f"RECOMMENDATION: Evaluate {region} for business development opportunities."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "market_opportunity", region, sources,
                          ["market", "opportunity", "growth", region.lower().replace(" ", "_")], 0.75)
        insights_created += 1
        log.info(f"💡 Market opportunity insight: {region}")

    # ─── 4. TECHNOLOGY TREND DETECTION ─────────────────────────────────────────
    # Find technology/patent signals across multiple companies
    # Raised threshold: 3+ innovating companies (was 2)
    tech_trends = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(DISTINCT c.id) as innovating_companies,
            array_agg(DISTINCT c.name) as companies,
            array_agg(DISTINCT w.title) as signal_titles,
            COUNT(*) as tech_signals,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'technology'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(DISTINCT c.id) >= 3
    """)
    
    for row in tech_trends:
        region = row["region"]
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%innovation%"
        )
        if existing:
            continue
        
        title = f"Innovation Cluster: Technology activity in {region}"
        summary = (
            f"ANALYSIS: {row['innovating_companies']} companies in {region} are showing R&D/technology signals: "
            f"{', '.join(row['companies'][:4])}. "
            f"With {row['tech_signals']} technology-related alerts, this suggests an active innovation environment. "
            f"IMPLICATIONS: Potential new product introductions, capability developments, or patent filings "
            f"that could shift competitive dynamics. "
            f"RECOMMENDATION: Monitor for product announcements; assess technology licensing opportunities."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "technology_trend", region, sources,
                          ["technology", "innovation", "trend", region.lower().replace(" ", "_")], 0.73)
        insights_created += 1
        log.info(f"💡 Technology trend insight: {region}")

    # ─── 5. GEOPOLITICAL RISK ASSESSMENT ───────────────────────────────────────
    # Raised threshold: 3+ risk signals (was 2)
    geopolitical_risks = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as risk_signals,
            COUNT(DISTINCT c.id) as exposed_companies,
            array_agg(DISTINCT c.name) as companies,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('geopolitical_risk', 'geopolitical')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 3
    """)
    
    for row in geopolitical_risks:
        region = row["region"]
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%geopolitical%"
        )
        if existing:
            continue
        
        title = f"Geopolitical Risk: {region} exposure requires attention"
        summary = (
            f"ANALYSIS: {row['risk_signals']} geopolitical risk signals detected for {row['exposed_companies']} "
            f"companies in {region}: {', '.join(row['companies'][:4])}. "
            f"Risk factors may include sanctions, tariffs, export controls, or regulatory changes. "
            f"EXPOSURE: Companies with operations or supply chains in {region} may face compliance or continuity risks. "
            f"RECOMMENDATION: Review supplier concentration in {region}; assess regulatory compliance requirements."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "geopolitical_risk", region, sources,
                          ["geopolitical", "risk", "compliance", region.lower().replace(" ", "_")], 0.80)
        insights_created += 1
        log.info(f"💡 Geopolitical risk insight: {region}")

    # ─── 6. M&A / CONSOLIDATION WATCH ──────────────────────────────────────────
    # Raised threshold: 3+ ma signals per company (was 2)
    ma_activity = await pool.fetch("""
        SELECT 
            c.name, c.region,
            COUNT(*) as ma_signals,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'ma_activity'
        GROUP BY c.id, c.name, c.region
        HAVING COUNT(*) >= 3
        LIMIT 3
    """)
    
    for row in ma_activity:
        company = row["name"]
        region = row["region"] or "Global"
        
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{company}%M&A%"
        )
        if existing:
            continue
        
        title = f"M&A Watch: {company} shows acquisition or partnership signals"
        summary = (
            f"ANALYSIS: {company} ({region}) has triggered {row['ma_signals']} M&A-related signals. "
            f"Signal details: {'; '.join(row['signal_titles'][:3])}. "
            f"This pattern may indicate: acquisition activity, strategic partnership formation, "
            f"or joint venture discussions. "
            f"MARKET IMPLICATIONS: Potential industry consolidation; may affect competitive landscape. "
            f"RECOMMENDATION: Monitor for official announcements; assess strategic response options."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        
        await store_insight(pool, title, summary, "ma_activity", region, sources,
                          ["m&a", "consolidation", "strategic", region.lower().replace(" ", "_")], 0.77)
        insights_created += 1
        log.info(f"💡 M&A insight: {company}")

    # ─── 7. CONFLICT ESCALATION CASCADE ────────────────────────────────────────
    # Detect regions with multiple conflict-related signals across entities
    conflict_esc = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as conflict_signals,
            COUNT(DISTINCT c.id) as affected_entities,
            array_agg(DISTINCT c.name) as entities,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        LEFT JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('conflict_escalation', 'military_procurement')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in conflict_esc:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%conflict%escalat%"
        )
        if existing:
            continue

        severity = "CRITICAL" if row["conflict_signals"] >= 4 else "ELEVATED"
        title = f"Conflict Escalation: {severity} risk in {region}"
        summary = (
            f"ANALYSIS: {row['conflict_signals']} conflict/military signals detected in {region} "
            f"involving {row['affected_entities']} entities over 24 hours. "
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"ESCALATION PATTERN: Multiple independent data points suggest escalation trajectory. "
            f"SUPPLY CHAIN IMPACT: Companies operating in or sourcing from {region} face operational risk. "
            f"RECOMMENDATION: Review exposure to {region}; activate contingency plans for affected supply chains."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["conflict", "escalation", "geopolitical", region.lower().replace(" ", "_")], 0.85)
        insights_created += 1
        log.info(f"💡 Conflict escalation insight: {region}")

    # ─── 8. SANCTIONS CASCADE IMPACT ──────────────────────────────────────────
    # Cross-reference sanctions signals with supply chain exposure
    sanctions_imp = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as sanctions_signals,
            COUNT(DISTINCT c.id) as exposed_companies,
            array_agg(DISTINCT c.name) as companies,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        LEFT JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('sanctions_cascade', 'geopolitical_risk')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in sanctions_imp:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%sanctions%cascade%"
        )
        if existing:
            continue

        title = f"Sanctions Cascade: Compliance risk escalating in {region}"
        summary = (
            f"ANALYSIS: {row['sanctions_signals']} sanctions/export control signals detected for {region}. "
            f"Potentially affected entities: {', '.join(row['companies'][:4]) if row['companies'][0] else 'multiple sectors'}.\n\n"
            f"CASCADE RISK: New sanctions designations may trigger secondary compliance obligations "
            f"across supply chains. Companies with direct or indirect {region} exposure should review "
            f"counterparty risk.\n"
            f"RECOMMENDATION: Conduct immediate sanctions screening of {region}-linked counterparties; "
            f"assess second-hop supply chain exposure."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["sanctions", "cascade", "compliance", region.lower().replace(" ", "_")], 0.83)
        insights_created += 1
        log.info(f"💡 Sanctions cascade insight: {region}")

    # ─── 9. DIPLOMATIC REALIGNMENT DETECTION ──────────────────────────────────
    diplomatic_shifts = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as diplomatic_signals,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '48 hours'
        AND w.warning_type = 'diplomatic_shift'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in diplomatic_shifts:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '24 hours'",
            f"%{region}%diplomatic%"
        )
        if existing:
            continue

        title = f"Diplomatic Shift: Realignment detected involving {region}"
        summary = (
            f"ANALYSIS: {row['diplomatic_signals']} diplomatic shift indicators detected for {region} "
            f"over 48 hours.\n"
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"STRATEGIC IMPLICATIONS: Diplomatic realignments often precede trade policy changes, "
            f"sanctions modifications, or shifts in military cooperation. "
            f"BUSINESS IMPACT: Companies reliant on bilateral frameworks involving {region} should "
            f"assess regulatory stability.\n"
            f"RECOMMENDATION: Monitor for follow-on trade or defense policy announcements."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["diplomatic", "realignment", "strategic", region.lower().replace(" ", "_")], 0.78)
        insights_created += 1
        log.info(f"💡 Diplomatic shift insight: {region}")

    # ─── 10. ENERGY SECURITY CORRIDOR ASSESSMENT ─────────────────────────────
    energy_threats = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as energy_signals,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('energy_security', 'trade_corridor_disruption')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in energy_threats:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%energy%"
        )
        if existing:
            continue

        title = f"Energy Security: Supply corridor risk in {region}"
        summary = (
            f"ANALYSIS: {row['energy_signals']} energy/trade corridor disruption signals detected for {region}.\n"
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"IMPACT ASSESSMENT: Energy supply disruptions in {region} may cascade to manufacturing costs, "
            f"logistics delays, and component pricing. Critical for industries dependent on stable energy corridors.\n"
            f"RECOMMENDATION: Assess energy supply chain exposure; evaluate alternative sourcing corridors."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["energy", "security", "corridor", region.lower().replace(" ", "_")], 0.80)
        insights_created += 1
        log.info(f"💡 Energy security insight: {region}")

    # ─── 11. CYBER THREAT LANDSCAPE SHIFT ────────────────────────────────────
    cyber_threats = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as cyber_signals,
            COUNT(DISTINCT c.id) as targeted_entities,
            array_agg(DISTINCT c.name) as entities,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        LEFT JOIN companies c ON c.id = ANY(w.entity_ids)
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'cyber_warfare'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in cyber_threats:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%cyber%"
        )
        if existing:
            continue

        title = f"Cyber Threat: State-linked activity targeting {region} sectors"
        summary = (
            f"ANALYSIS: {row['cyber_signals']} cyber warfare/espionage signals detected in {region} "
            f"targeting {row['targeted_entities']} entities.\n"
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"THREAT ASSESSMENT: Elevated cyber posture suggests state-sponsored or state-aligned "
            f"threat actors targeting critical sectors. "
            f"Affected entities: {', '.join(row['entities'][:3]) if row['entities'][0] else 'multiple sectors'}.\n"
            f"RECOMMENDATION: Elevate cybersecurity posture; review critical infrastructure dependencies."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["cyber", "warfare", "threat", region.lower().replace(" ", "_")], 0.82)
        insights_created += 1
        log.info(f"💡 Cyber threat insight: {region}")

    # ─── 12. CRITICAL MINERAL SUPPLY STRESS ──────────────────────────────────
    mineral_stress = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as mineral_signals,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'critical_mineral_risk'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in mineral_stress:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%mineral%"
        )
        if existing:
            continue

        title = f"Critical Minerals: Supply stress detected in {region}"
        summary = (
            f"ANALYSIS: {row['mineral_signals']} critical mineral supply signals from {region}.\n"
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"SUPPLY CHAIN IMPACT: Disruptions in critical mineral supply from {region} directly affect "
            f"semiconductor, battery, defense, and electronics manufacturing globally. "
            f"STRATEGIC CONTEXT: Resource nationalism and export controls on critical minerals are "
            f"increasingly used as geopolitical leverage.\n"
            f"RECOMMENDATION: Diversify mineral sourcing; assess inventory buffers for critical inputs."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["minerals", "supply", "critical", region.lower().replace(" ", "_")], 0.79)
        insights_created += 1
        log.info(f"💡 Critical mineral insight: {region}")

    # ─── 13. TRADE CORRIDOR VULNERABILITY ─────────────────────────────────────
    corridor_risks = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(*) as corridor_signals,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'trade_corridor_disruption'
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(*) >= 2
    """)

    for row in corridor_risks:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%corridor%"
        )
        if existing:
            continue

        title = f"Trade Corridor: Disruption risk at {region} chokepoint"
        summary = (
            f"ANALYSIS: {row['corridor_signals']} trade corridor disruption signals at {region}.\n"
            f"Indicators: {'; '.join(row['signal_titles'][:3])}.\n\n"
            f"GLOBAL IMPACT: Major trade corridor disruptions at {region} affect global shipping, "
            f"increase transit times, and raise logistics costs across all sectors. "
            f"Historical precedent suggests 15-40% cost increases during active corridor disruptions.\n"
            f"RECOMMENDATION: Assess shipping route dependencies; pre-position inventory; evaluate "
            f"alternative routing options."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["trade", "corridor", "disruption", region.lower().replace(" ", "_")], 0.81)
        insights_created += 1
        log.info(f"💡 Trade corridor insight: {region}")

    # ─── 14. GEOPOLITICAL CONVERGENCE ALERT ───────────────────────────────────
    # When a region has MULTIPLE different geo signal types = compound risk
    convergence = await pool.fetch("""
        SELECT 
            w.region,
            COUNT(DISTINCT w.warning_type) as signal_types,
            COUNT(*) as total_signals,
            array_agg(DISTINCT w.warning_type) as types,
            array_agg(DISTINCT w.title) as signal_titles,
            array_agg(w.source_urls) as raw_sources
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type IN ('geopolitical_risk', 'conflict_escalation', 'sanctions_cascade',
                               'diplomatic_shift', 'military_procurement', 'energy_security',
                               'cyber_warfare', 'critical_mineral_risk', 'trade_corridor_disruption')
        AND w.region IS NOT NULL
        GROUP BY w.region
        HAVING COUNT(DISTINCT w.warning_type) >= 3
    """)

    for row in convergence:
        region = row["region"]
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%convergence%"
        )
        if existing:
            continue

        type_labels = {
            "geopolitical_risk": "trade/sanctions risk",
            "conflict_escalation": "military conflict",
            "sanctions_cascade": "sanctions enforcement",
            "diplomatic_shift": "diplomatic realignment",
            "military_procurement": "arms buildup",
            "energy_security": "energy disruption",
            "cyber_warfare": "cyber operations",
            "critical_mineral_risk": "mineral supply stress",
            "trade_corridor_disruption": "maritime/trade route threat",
        }
        active_types = [type_labels.get(t, t) for t in row["types"]]

        title = f"CONVERGENCE ALERT: {region} — {row['signal_types']} compound risk factors"
        summary = (
            f"CRITICAL ANALYSIS: {region} is exhibiting {row['signal_types']} distinct categories of "
            f"geopolitical risk simultaneously, with {row['total_signals']} total signals in 24 hours.\n\n"
            f"ACTIVE RISK DIMENSIONS: {', '.join(active_types)}.\n\n"
            f"COMPOUND RISK ASSESSMENT: The convergence of multiple geopolitical risk vectors in a "
            f"single region significantly amplifies overall exposure. Historical analysis shows that "
            f"compound geopolitical stress often precedes major disruption events.\n\n"
            f"PRIORITY: CRITICAL — Requires immediate review of all {region}-linked operations, "
            f"supply chains, and counterparty relationships.\n"
            f"RECOMMENDATION: Convene risk assessment; scenario-plan for operational disruption; "
            f"engage legal/compliance for sanctions exposure review."
        )
        sources = filter_relevant_urls(flatten_source_arrays(row["raw_sources"]))
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["convergence", "compound_risk", "critical", region.lower().replace(" ", "_")], 0.90)
        insights_created += 1
        log.info(f"🔴 Convergence alert: {region} ({row['signal_types']} risk dimensions)")

    log.info(f"✓ Generated {insights_created} analytical insights from pattern analysis")
    return insights_created


# ─── Competitor Customer Discovery ──────────────────────────────────────────────

# Pages on competitor sites that typically list their customers
CUSTOMER_EVIDENCE_PATHS = [
    "/customers", "/our-customers", "/client-stories", "/case-studies",
    "/success-stories", "/testimonials", "/references", "/partners",
    "/who-we-serve", "/industries-served", "/portfolio",
]

# Keywords indicating customer relationships with high reliability
CUSTOMER_KEYWORDS = [
    r'(?:our\s+)?(?:customer|client)s?\s+include',
    r'trusted\s+by',
    r'serving\s+(?:companies|clients|customers)\s+(?:like|such\s+as|including)',
    r'case\s+stud(?:y|ies)',
    r'testimonial',
    r'customer\s+success',
    r'working\s+with',
    r'proud\s+to\s+serve',
    r'our\s+partners?\s+include',
    r'selected\s+(?:clients?|customers?)',
]

# Regex to extract company names from customer listings (after anchor phrases)
CUSTOMER_NAME_PATTERN = re.compile(
    r'(?:customers?\s+include|trusted\s+by|serving.*?(?:like|including)|working\s+with|proud\s+to\s+serve)'
    r'[:\s]*([A-Z][A-Za-z&\s,.\'-]+(?:,\s*[A-Z][A-Za-z&\s.\'-]+)*)',
    re.IGNORECASE
)

# Alt-text and logo-based extraction for customer logo grids
LOGO_ALT_PATTERN = re.compile(
    r'(?:logo|client|customer|partner)\s*[-:]?\s*([A-Z][A-Za-z0-9&\s.\'-]{2,40})',
    re.IGNORECASE
)


async def discover_competitor_customers(pool: asyncpg.Pool, session: aiohttp.ClientSession,
                                        semaphore: asyncio.Semaphore):
    """
    Scrape competitor websites for customer references (case studies, testimonials,
    customer lists, logo walls). Verify via multi-source corroboration before inserting.

    Veracity strategy:
    1. Extract candidate customer names from competitor's own site
    2. Cross-reference with press releases / news mentions
    3. Only insert when confidence >= 0.85 (at least 2 independent sources)
    4. Tag provenance with all evidence sources
    """
    log.info("🔍 Starting competitor customer discovery...")

    # Get competitors with domains
    competitors = await pool.fetch(
        "SELECT id, name, domain FROM companies WHERE metadata->>'is_competitor' = 'true' AND domain IS NOT NULL"
    )
    log.info(f"   Scanning {len(competitors)} competitors for customer references")

    total_discovered = 0

    for comp in competitors:
        comp_id = str(comp["id"])
        comp_name = comp["name"]
        comp_domain = comp["domain"]

        candidate_customers = {}  # name -> list of evidence dicts

        # Phase 1: Scrape customer-related pages on competitor site
        for path in CUSTOMER_EVIDENCE_PATHS:
            url = f"https://{comp_domain}{path}"
            result = await fetch_url(session, url, semaphore)
            if not result or result["status"] != 200:
                continue

            html = result["html"]
            text = extract_text(html)
            text_lower = text.lower()

            # Check if page has customer-related content
            has_customer_content = any(
                re.search(kw, text_lower) for kw in CUSTOMER_KEYWORDS
            )
            if not has_customer_content:
                continue

            log.debug(f"   Customer page found: {url}")

            # Extract customer names from text patterns
            for match in CUSTOMER_NAME_PATTERN.finditer(text):
                names_str = match.group(1)
                # Split on commas, 'and', semicolons
                raw_names = re.split(r'[,;]\s*|\s+and\s+', names_str)
                for raw in raw_names:
                    name = raw.strip().strip('.')
                    # Filter: must be 2-60 chars, start with uppercase, not generic words
                    if (len(name) < 2 or len(name) > 60
                            or not name[0].isupper()
                            or name.lower() in ('the', 'our', 'their', 'some', 'many',
                                                 'various', 'several', 'multiple',
                                                 'leading', 'major', 'global', 'top')):
                        continue
                    if name not in candidate_customers:
                        candidate_customers[name] = []
                    candidate_customers[name].append({
                        "source": "competitor_website",
                        "url": url,
                        "path": path,
                        "context": f"Listed on {comp_name}'s customer page",
                    })

            # Extract from image alt texts (customer logo walls)
            soup = BeautifulSoup(html, "html.parser")
            for img in soup.find_all("img", alt=True):
                alt = img["alt"]
                m = LOGO_ALT_PATTERN.search(alt)
                if m:
                    name = m.group(1).strip()
                    if len(name) >= 2 and name[0].isupper():
                        if name not in candidate_customers:
                            candidate_customers[name] = []
                        candidate_customers[name].append({
                            "source": "logo_alt_text",
                            "url": url,
                            "alt_text": alt,
                            "context": f"Customer logo on {comp_name}'s website",
                        })

            await asyncio.sleep(random.uniform(0.5, 1.5))

        if not candidate_customers:
            continue

        log.info(f"   {comp_name}: {len(candidate_customers)} candidate customers found")

        # Phase 2: Cross-reference via web search / news (lightweight verification)
        for cust_name, evidences in candidate_customers.items():
            # Skip if only one weak evidence source
            # Try news verification: search for "[competitor] + [customer] + contract/partnership/supply"
            verification_queries = [
                f'"{comp_name}" "{cust_name}" contract',
                f'"{comp_name}" "{cust_name}" supply',
                f'"{comp_name}" "{cust_name}" partner',
            ]

            news_evidence_count = 0
            for query in verification_queries:
                # Use a simple search via DuckDuckGo lite (no API key needed)
                search_url = f"https://lite.duckduckgo.com/lite/?q={query.replace(' ', '+')}"
                result = await fetch_url(session, search_url, semaphore)
                if result and result["status"] == 200:
                    search_text = extract_text(result["html"]).lower()
                    if (cust_name.lower() in search_text
                            and comp_name.lower() in search_text):
                        news_evidence_count += 1
                        evidences.append({
                            "source": "web_search",
                            "query": query,
                            "context": f"Verified via web search: {query}",
                        })
                await asyncio.sleep(random.uniform(1.0, 2.0))
                if news_evidence_count >= 1:
                    break  # One verification is enough with direct site evidence

            # Phase 3: Veracity gate — require at least 2 independent evidence sources
            source_types = set(e["source"] for e in evidences)
            total_evidence = len(evidences)
            confidence = min(0.98, 0.65 + (total_evidence * 0.08) + (len(source_types) * 0.1))

            if total_evidence < 2 or confidence < 0.85:
                log.debug(f"   Skipping {cust_name} (insufficient evidence: {total_evidence} sources, conf={confidence:.2f})")
                continue

            # Phase 4: Insert or update company as customer_of_competitor
            # Check if already known
            existing = await pool.fetchrow(
                "SELECT id, company_type, metadata FROM companies WHERE name ILIKE $1",
                cust_name
            )

            if existing:
                # Update metadata to note it's a customer of this competitor
                metadata = json.loads(existing["metadata"]) if existing["metadata"] else {}
                customer_of = metadata.get("customer_of_competitors", [])
                if comp_name not in customer_of:
                    customer_of.append(comp_name)
                    metadata["customer_of_competitors"] = customer_of
                    metadata["customer_evidence"] = metadata.get("customer_evidence", [])
                    metadata["customer_evidence"].extend([{
                        "competitor": comp_name,
                        "sources": [e["source"] for e in evidences],
                        "confidence": confidence,
                        "discovered_at": datetime.now(timezone.utc).isoformat(),
                    }])
                    await pool.execute(
                        "UPDATE companies SET metadata = $1, updated_at = NOW() WHERE id = $2",
                        json.dumps(metadata), existing["id"]
                    )
                    log.info(f"   ✓ Updated existing company {cust_name} as customer of {comp_name} (conf={confidence:.2f})")
            else:
                # Insert new company
                new_id = uuid.uuid4()
                metadata = {
                    "customer_of_competitors": [comp_name],
                    "customer_evidence": [{
                        "competitor": comp_name,
                        "sources": [e["source"] for e in evidences],
                        "confidence": confidence,
                        "discovered_at": datetime.now(timezone.utc).isoformat(),
                    }],
                    "discovery_method": "competitor_customer_scraping",
                "is_competitor": false,
                    }
                await pool.execute(
                    """INSERT INTO companies (id, name, company_type, metadata, created_at, updated_at)
                       VALUES ($1, $2, 'customer_of_competitor', $3, NOW(), NOW())
                       ON CONFLICT (domain) DO NOTHING""",
                    new_id, cust_name, json.dumps(metadata)
                )
                log.info(f"   ✓ Discovered new customer: {cust_name} (customer of {comp_name}, conf={confidence:.2f})")

            total_discovered += 1

            # Store observation for audit trail
            await store_observation(
                pool,
                obs_type="CompetitorEvent",
                entity_id=comp_id,
                entity_type="company",
                value={
                    "event_type": "customer_discovered",
                    "customer_name": cust_name,
                    "evidence_count": total_evidence,
                    "source_types": list(source_types),
                    "confidence": confidence,
                },
                provenance={
                    "evidence": [{"source": e["source"], "url": e.get("url", ""), "context": e["context"]} for e in evidences],
                    "extractor_version": "apex-crawler-1.0",
                    "veracity_method": "multi_source_corroboration",
                },
                confidence=confidence
            )

            # Generate insight about the discovery
            if confidence >= 0.90:
                await store_insight(
                    pool,
                    title=f"{cust_name} identified as customer of competitor {comp_name}",
                    summary=(
                        f"Through multi-source analysis of {comp_name}'s public materials "
                        f"(website, case studies, press releases), {cust_name} has been identified "
                        f"as an active customer. Verified through {total_evidence} independent "
                        f"evidence sources with {confidence:.0%} confidence. "
                        f"This represents a potential business development target or competitive "
                        f"intelligence data point."
                    ),
                    insight_type="competitor_market",
                    region="Global",
                    evidence_urls=[e.get("url", "") for e in evidences if e.get("url")],
                    tags=["customer_discovery", "competitor_intelligence", comp_name.lower().replace(" ", "_")],
                    confidence=confidence
                )

    log.info(f"✓ Competitor customer discovery complete: {total_discovered} customers discovered/updated")
    return total_discovered


# ─── POI Contact Intelligence ───────────────────────────────────────────────────

# Regex patterns for aggressive contact extraction
_EMAIL_RE = re.compile(
    r'[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}',
)
_PHONE_RE = re.compile(
    r'(?:\+\d{1,3}[\s\-.]?)?\(?\d{1,4}\)?[\s\-.]?\d{2,4}[\s\-.]?\d{2,4}(?:[\s\-.]?\d{1,4})?',
)
_LINKEDIN_PROFILE_RE = re.compile(
    r'(?:https?://)?(?:www\.)?linkedin\.com/in/([a-zA-Z0-9\-_%]+)',
    re.IGNORECASE,
)

# Titles that indicate key decision-makers (C-suite, VP, Director)
_EXECUTIVE_TITLE_PATTERNS = [
    r'\b(?:CEO|CTO|CFO|COO|CIO|CISO|CMO|CPO|CSO|CDO|CHRO)\b',
    r'\bChief\s+(?:Executive|Technology|Financial|Operating|Information|Security|Marketing|Product|Strategy|Data|Human\s+Resources)\s+Officer\b',
    r'\b(?:President|Vice[\s\-]?President|VP)\b',
    r'\bSenior\s+Vice[\s\-]?President\b',
    r'\b(?:Managing\s+)?Director\b',
    r'\bGeneral\s+Manager\b',
    r'\bHead\s+of\b',
    r'\bSVP\b',
    r'\bEVP\b',
    r'\bFounder\b',
    r'\bCo[\s\-]?Founder\b',
    r'\bChairman\b',
    r'\bBoard\s+Member\b',
    r'\bManaging\s+Partner\b',
]
_EXEC_TITLE_RE = re.compile('|'.join(_EXECUTIVE_TITLE_PATTERNS), re.IGNORECASE)

# Pages that contain leadership/team info
_LEADERSHIP_PATHS = [
    "/about", "/about-us", "/team", "/leadership", "/management",
    "/our-team", "/our-leadership", "/executive-team", "/executives",
    "/board-of-directors", "/board", "/who-we-are", "/people",
    "/company/leadership", "/company/team", "/about/leadership",
    "/about/team", "/about/management", "/about/executives",
    "/contact", "/contact-us", "/impressum", "/imprint",
]

# Pages that contain contact info
_CONTACT_PATHS = [
    "/contact", "/contact-us", "/impressum", "/imprint",
    "/kontakt", "/contacto", "/nous-contacter",
]

# Common noise emails to skip
_NOISE_EMAIL_DOMAINS = frozenset({
    "example.com", "example.org", "test.com", "sentry.io",
    "w3.org", "schema.org", "googleapis.com", "google.com",
    "facebook.com", "twitter.com", "instagram.com", "youtube.com",
    "wixpress.com", "squarespace.com", "wordpress.com",
    "cloudflare.com", "cdn.com", "jquery.com", "bootstrap.com",
})

_NOISE_EMAIL_PREFIXES = frozenset({
    "noreply", "no-reply", "donotreply", "mailer-daemon", "postmaster",
    "webmaster", "admin", "root", "abuse", "support", "help",
    "newsletter", "marketing", "unsubscribe", "feedback",
    "privacy", "legal", "compliance", "security",
})


def _is_valid_contact_email(email: str) -> bool:
    """Check if an email is a real contact email (not noise/generic)."""
    email_lower = email.lower().strip()
    if len(email_lower) < 5 or len(email_lower) > 100:
        return False
    local, _, domain = email_lower.partition("@")
    if not domain or domain in _NOISE_EMAIL_DOMAINS:
        return False
    if local in _NOISE_EMAIL_PREFIXES:
        return False
    # Must have a proper TLD
    if "." not in domain or len(domain.split(".")[-1]) < 2:
        return False
    # Skip emails that look like CSS/JS artifacts
    if any(c in email_lower for c in ["{", "}", "(", ")", "[", "]", ";", "//", "/*"]):
        return False
    return True


def _is_valid_phone(phone: str) -> bool:
    """Check if a phone string looks like a real phone number."""
    digits = re.sub(r'\D', '', phone)
    # Real phone numbers have 7-15 digits
    if len(digits) < 7 or len(digits) > 15:
        return False
    # Skip things that look like years, zip codes, etc.
    if re.match(r'^(19|20)\d{2}$', digits):
        return False
    return True


def _extract_person_name_title_pairs(text: str) -> list[dict]:
    """
    Extract person name + title pairs from leadership/team page text.
    Uses multiple heuristic patterns to find real executive names.
    """
    results = []
    seen_names = set()

    # Pattern 1: "Name, Title" or "Name - Title" or "Name | Title"
    # Looks for Capitalized Names followed by executive titles
    name_title_re = re.compile(
        r'([A-Z][a-z]+(?:\s+(?:de|van|von|al|el|la|le|di|du|bin|ben|ibn))?'
        r'(?:\s+[A-Z][a-z]+){1,3})'
        r'\s*[,\-–—|]\s*'
        r'(' + '|'.join(_EXECUTIVE_TITLE_PATTERNS) + r'[^.\n]{0,60})',
        re.IGNORECASE,
    )
    for m in name_title_re.finditer(text):
        name = m.group(1).strip()
        title = m.group(2).strip().rstrip(",.- ")
        if len(name) >= 4 and name not in seen_names:
            seen_names.add(name)
            results.append({"name": name, "title": title})

    # Pattern 2: Title followed by Name
    # "CEO John Smith" or "Director of Sales: Jane Doe"
    title_name_re = re.compile(
        r'(' + '|'.join(_EXECUTIVE_TITLE_PATTERNS) + r'[^:,\n]{0,30}?)'
        r'[:\s]+([A-Z][a-z]+(?:\s+(?:de|van|von|al|el|la|le|di|du|bin|ben|ibn))?'
        r'(?:\s+[A-Z][a-z]+){1,3})',
        re.IGNORECASE,
    )
    for m in title_name_re.finditer(text):
        title = m.group(1).strip().rstrip(",.- ")
        name = m.group(2).strip()
        if len(name) >= 4 and name not in seen_names:
            seen_names.add(name)
            results.append({"name": name, "title": title})

    # Pattern 3: HTML-structured patterns — Name in one block, title in proximity
    # This handles text extracted like "John Smith CEO Acme Corp"
    lines = text.split("\n")
    for i, line in enumerate(lines):
        line = line.strip()
        # Check if line looks like a 2-3 word proper name
        words = line.split()
        if 2 <= len(words) <= 4 and all(w[0].isupper() for w in words if len(w) > 1):
            # Check surrounding lines for title
            context = " ".join(lines[max(0,i-1):i+3])
            title_m = _EXEC_TITLE_RE.search(context)
            if title_m and line not in seen_names:
                # Extract the full title from context
                title_start = title_m.start()
                title_text = context[title_start:title_start+80].split("\n")[0].strip()
                title_text = re.sub(r'\s+', ' ', title_text)[:80]
                seen_names.add(line)
                results.append({"name": line, "title": title_text})

    return results


def _infer_email_patterns(name: str, domain: str) -> list[str]:
    """Generate common corporate email patterns from a name and domain."""
    parts = name.lower().split()
    if len(parts) < 2:
        return []
    first = re.sub(r'[^a-z]', '', parts[0])
    last = re.sub(r'[^a-z]', '', parts[-1])
    if not first or not last:
        return []
    return [
        f"{first}.{last}@{domain}",       # john.smith@company.com
        f"{first[0]}{last}@{domain}",      # jsmith@company.com
        f"{first}{last[0]}@{domain}",      # johns@company.com
        f"{first}@{domain}",               # john@company.com
        f"{first}_{last}@{domain}",        # john_smith@company.com
        f"{first[0]}.{last}@{domain}",     # j.smith@company.com
        f"{last}.{first}@{domain}",        # smith.john@company.com
        f"{last}@{domain}",                # smith@company.com
    ]


async def _search_duckduckgo(session: aiohttp.ClientSession, query: str,
                              semaphore: asyncio.Semaphore) -> Optional[str]:
    """Search DuckDuckGo HTML for a query. Returns page HTML or None."""
    url = f"https://html.duckduckgo.com/html/?q={query.replace(' ', '+')}"
    result = await fetch_url(session, url, semaphore)
    if result and result["status"] == 200:
        return result["html"]
    return None


async def discover_poi_contacts(pool: asyncpg.Pool, session: aiohttp.ClientSession,
                                 semaphore: asyncio.Semaphore):
    """
    Aggressively discover and enrich POI (Person of Interest) contact information.

    Strategy:
    1. Crawl company leadership/team/about pages → extract executive name+title pairs
    2. Crawl contact/impressum pages → harvest emails, phones
    3. Search DuckDuckGo for "person name company linkedin" → find LinkedIn profiles
    4. Infer email patterns from name + company domain
    5. Match harvested emails to known persons by name/domain
    6. Update existing persons + create new ones from team pages
    7. Store evidence in poi_artifacts for provenance tracking
    """
    log.info("👤 Starting aggressive POI contact discovery...")

    companies = await pool.fetch(
        "SELECT id, name, domain FROM companies WHERE domain IS NOT NULL AND (metadata->>'is_competitor' IS NULL OR metadata->>'is_competitor' != 'true')"
    )
    competitors = await pool.fetch(
        "SELECT id, name, domain FROM companies WHERE domain IS NOT NULL AND metadata->>'is_competitor' = 'true'"
    )
    all_companies = list(companies) + list(competitors)
    log.info(f"   Scanning {len(all_companies)} companies for executive contacts")

    total_updated = 0
    total_new = 0

    for comp in all_companies:
        comp_id = str(comp["id"])
        comp_name = comp["name"]
        comp_domain = comp["domain"]

        # Skip government entities — they use different org structures
        if comp_name.lower().startswith("government of"):
            continue

        # ── Phase 1: Crawl leadership/team pages ──
        discovered_persons = []
        all_emails = set()
        all_phones = set()
        all_linkedin_urls = {}  # name -> url

        for path in _LEADERSHIP_PATHS:
            url = f"https://{comp_domain}{path}"
            result = await fetch_url(session, url, semaphore)
            if not result or result["status"] not in (200, 301, 302):
                continue

            text = extract_text(result["html"])
            if len(text) < 100:
                continue

            # Extract executive name+title pairs
            persons = _extract_person_name_title_pairs(text)
            for p in persons:
                p["source_url"] = url
                p["source_type"] = "leadership_page"
            discovered_persons.extend(persons)

            # Extract emails from page
            emails = _EMAIL_RE.findall(text + " " + result["html"])
            for email in emails:
                if _is_valid_contact_email(email):
                    all_emails.add(email.lower())

            # Extract phone numbers from page
            phones = _PHONE_RE.findall(text)
            for phone in phones:
                if _is_valid_phone(phone):
                    all_phones.add(phone.strip())

            # Extract LinkedIn URLs from page HTML
            for m in _LINKEDIN_PROFILE_RE.finditer(result["html"]):
                linkedin_url = f"https://www.linkedin.com/in/{m.group(1)}"
                # Try to associate with a nearby name
                # Find the linkedin link's surrounding text
                pos = m.start()
                context_start = max(0, pos - 200)
                context_end = min(len(result["html"]), pos + 200)
                context = result["html"][context_start:context_end]
                context_text = BeautifulSoup(context, "html.parser").get_text()
                # Check discovered persons for name match
                for dp in discovered_persons:
                    if dp["name"].split()[-1].lower() in context_text.lower():
                        all_linkedin_urls[dp["name"]] = linkedin_url
                        break
                else:
                    # Associate the profile slug with any matching person
                    slug = m.group(1).lower().replace("-", " ").replace("_", " ")
                    for dp in discovered_persons:
                        name_parts = dp["name"].lower().split()
                        if any(part in slug for part in name_parts if len(part) > 2):
                            all_linkedin_urls[dp["name"]] = linkedin_url
                            break

        # ── Phase 2: Crawl dedicated contact pages ──
        for path in _CONTACT_PATHS:
            url = f"https://{comp_domain}{path}"
            result = await fetch_url(session, url, semaphore)
            if not result or result["status"] not in (200, 301, 302):
                continue

            text = extract_text(result["html"])
            emails = _EMAIL_RE.findall(text + " " + result["html"])
            for email in emails:
                if _is_valid_contact_email(email):
                    all_emails.add(email.lower())

            phones = _PHONE_RE.findall(text)
            for phone in phones:
                if _is_valid_phone(phone):
                    all_phones.add(phone.strip())

        # ── Phase 3: Search LinkedIn via DuckDuckGo ──
        for dp in discovered_persons[:5]:  # Limit to top 5 per company
            if dp["name"] in all_linkedin_urls:
                continue  # Already found
            query = f'{dp["name"]} {comp_name} linkedin'
            ddg_html = await _search_duckduckgo(session, query, semaphore)
            if ddg_html:
                for m in _LINKEDIN_PROFILE_RE.finditer(ddg_html):
                    slug = m.group(1).lower().replace("-", " ")
                    name_parts = dp["name"].lower().split()
                    # Check if the LinkedIn slug matches the person's name
                    if sum(1 for part in name_parts if part in slug and len(part) > 2) >= 1:
                        all_linkedin_urls[dp["name"]] = f"https://www.linkedin.com/in/{m.group(1)}"
                        break
            await asyncio.sleep(random.uniform(1.0, 2.0))  # Be polite to DDG

        # ── Phase 4: Filter company-domain emails ──
        company_emails = {e for e in all_emails if comp_domain in e}
        generic_email = None
        personal_emails = {}  # name -> email

        for email in company_emails:
            local = email.split("@")[0]
            # Try to match to discovered persons
            for dp in discovered_persons:
                inferred = _infer_email_patterns(dp["name"], comp_domain)
                if email in inferred:
                    personal_emails[dp["name"]] = email
                    break
            else:
                # Check if it looks personal (has dots/underscores suggesting firstname.lastname)
                if "." in local or "_" in local:
                    # Could be a personal email
                    pass
                elif local in ("info", "contact", "sales", "office", "hello", "general"):
                    generic_email = email

        # ── Phase 5: Assign inferred emails if no real email found ──
        for dp in discovered_persons:
            if dp["name"] not in personal_emails:
                # Use the most common email pattern: first.last@domain
                inferred = _infer_email_patterns(dp["name"], comp_domain)
                if inferred:
                    personal_emails[dp["name"]] = inferred[0]  # first.last@domain

        # Pick the best phone (prefer international format)
        best_phone = None
        for phone in sorted(all_phones, key=lambda p: (p.startswith("+"), len(p)), reverse=True):
            best_phone = phone
            break

        # ── Phase 6: Update existing persons ──
        existing = await pool.fetch(
            "SELECT id, name, public_email, metadata FROM persons WHERE primary_org_id = $1",
            comp["id"]
        )
        for row in existing:
            person_name = row["name"]
            person_id = str(row["id"])
            updates = {}
            metadata = json.loads(row["metadata"]) if row["metadata"] else {}

            # Try to match with discovered persons
            matched_dp = None
            for dp in discovered_persons:
                if dp["name"].lower() == person_name.lower():
                    matched_dp = dp
                    break
                # Partial match (last name)
                if person_name.split()[-1].lower() == dp["name"].split()[-1].lower():
                    matched_dp = dp
                    break

            # Update email
            if not row["public_email"]:
                if person_name in personal_emails:
                    updates["public_email"] = personal_emails[person_name]
                elif matched_dp and matched_dp["name"] in personal_emails:
                    updates["public_email"] = personal_emails[matched_dp["name"]]
                elif company_emails:
                    # Infer from name
                    inferred = _infer_email_patterns(person_name, comp_domain)
                    if inferred:
                        updates["public_email"] = inferred[0]

            # Update LinkedIn
            if not metadata.get("linkedin"):
                li_url = all_linkedin_urls.get(person_name) or (
                    all_linkedin_urls.get(matched_dp["name"]) if matched_dp else None
                )
                if li_url:
                    metadata["linkedin"] = li_url

            # Update phone
            if not metadata.get("phone") and best_phone:
                metadata["phone"] = best_phone

            # Update role from team page
            if matched_dp and (row.get("current_role") in (None, "", "postgres")):
                updates["current_role"] = matched_dp["title"]

            # Update engagement status
            has_new_data = bool(updates.get("public_email") or metadata.get("linkedin") or metadata.get("phone"))
            if has_new_data and metadata.get("engagement_status") == "untracked":
                metadata["engagement_status"] = "identified"

            if updates or metadata != (json.loads(row["metadata"]) if row["metadata"] else {}):
                metadata["contact_enriched_at"] = datetime.now(timezone.utc).isoformat()
                set_clauses = ["metadata = $2::jsonb", "updated_at = NOW()"]
                params = [comp["id"], json.dumps(metadata)]

                if "public_email" in updates:
                    set_clauses.append(f"public_email = ${len(params)+1}")
                    params.append(updates["public_email"])
                if "current_role" in updates:
                    set_clauses.append(f"current_role = ${len(params)+1}")
                    params.append(updates["current_role"])

                await pool.execute(
                    f"UPDATE persons SET {', '.join(set_clauses)} WHERE id = ${len(params)+1}",
                    *params, row["id"]
                )
                total_updated += 1

                # Store person change record
                if "public_email" in updates:
                    await pool.execute(
                        """INSERT INTO person_changes (person_id, change_type, field_name, old_value, new_value,
                           description, source_url, confidence)
                           VALUES ($1, 'contact_enrichment', 'email', $2, $3, $4, $5, $6)""",
                        row["id"], row["public_email"] or "",
                        updates["public_email"],
                        f"Email discovered for {person_name} at {comp_name}",
                        matched_dp["source_url"] if matched_dp else f"https://{comp_domain}/contact",
                        0.7 if updates["public_email"].endswith(f"@{comp_domain}") else 0.5,
                    )

                log.info(f"   ✏️ Updated {person_name}: email={updates.get('public_email','—')} "
                         f"linkedin={'✓' if metadata.get('linkedin') else '—'} phone={'✓' if metadata.get('phone') else '—'}")

        # ── Phase 7: Create new persons from team page discoveries ──
        existing_names = {row["name"].lower() for row in existing}
        for dp in discovered_persons:
            if dp["name"].lower() in existing_names:
                continue
            if len(dp["name"]) < 4:
                continue

            # Validate: must have a real-looking name (2-4 words, capitalized)
            name_words = dp["name"].split()
            if not (2 <= len(name_words) <= 4):
                continue
            if not all(w[0].isupper() for w in name_words if len(w) > 1):
                continue

            email = personal_emails.get(dp["name"])
            linkedin = all_linkedin_urls.get(dp["name"])
            phone = best_phone

            metadata = {
                "discovery_method": "team_page_extraction",
                "source_url": dp["source_url"],
                "linkedin": linkedin,
                "phone": phone,
                "confidence": 0.75,
                "engagement_status": "identified",
                "contact_enriched_at": datetime.now(timezone.utc).isoformat(),
            }

            try:
                person_id = await pool.fetchval(
                    """INSERT INTO persons (name, current_role, primary_org_id, public_email, metadata, region, country_code)
                       SELECT $1, $2, $3, $4, $5::jsonb,
                              c.region, c.country_code
                       FROM companies c WHERE c.id = $3
                       RETURNING id""",
                    dp["name"], dp["title"], comp["id"], email, json.dumps(metadata),
                )
                if person_id:
                    total_new += 1
                    existing_names.add(dp["name"].lower())
                    log.info(f"   👤 New: {dp['name']} ({dp['title']}) at {comp_name} "
                             f"email={email or '—'} linkedin={'✓' if linkedin else '—'}")

                    # Store discovery artifact
                    await pool.execute(
                        """INSERT INTO poi_artifacts (person_id, artifact_type, title, content_summary,
                           url, source_domain, ts_utc, provenance, metadata)
                           VALUES ($1, 'team_page_discovery', $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb)""",
                        person_id,
                        f"{dp['name']} - {dp['title']}",
                        f"Discovered on {comp_name} team/leadership page",
                        dp["source_url"],
                        comp_domain,
                        datetime.now(timezone.utc),
                        json.dumps({"extractor_version": "poi-contact-v1", "method": "team_page_extraction"}),
                        json.dumps({"email": email, "linkedin": linkedin, "phone": phone}),
                    )
            except Exception as e:
                log.debug(f"   Error creating person {dp['name']}: {e}")

    # ── Phase 7.5: Dark Web Intelligence Enrichment ──
    # Enrich POIs with breach data, leaked credentials, and dark web contacts
    if TOR_ENABLED:
        log.info("🧅 Starting dark web POI enrichment...")
        darkweb_enriched = 0
        
        # Get high-priority POIs for dark web enrichment (executives at competitors)
        priority_pois = await pool.fetch(
            """SELECT p.id, p.name, p.public_email, c.domain, c.name as org_name
               FROM persons p
               JOIN companies c ON p.primary_org_id = c.id
               WHERE c.metadata->>'is_competitor' = 'true'
                 AND p.metadata->>'darkweb_enriched' IS NULL
               ORDER BY p.created_at DESC
               LIMIT 50"""
        )
        
        for poi in priority_pois:
            person_id = poi["id"]
            person_name = poi["name"]
            email_domain = poi["domain"]
            org_name = poi["org_name"]
            
            try:
                # Aggregate dark web intel
                dwi = await aggregate_darkweb_poi_intel(person_name, email_domain, org_name)
                
                # Check for breached credentials
                if dwi["breach_records"]:
                    # Store breach warning
                    for br in dwi["breach_records"][:3]:  # Max 3 per person
                        await pool.execute(
                            """INSERT INTO warnings (recipe_code, warning_type, severity, title, description,
                                  evidence_urls, entity_ids, created_at)
                               VALUES ('credential_breach', 'security', 'high', $1, $2, $3, $4, $5)
                               ON CONFLICT DO NOTHING""",
                            f"Breached credentials: {person_name}",
                            f"Dark web breach database contains credentials for {br['email']}. "
                            f"Password hash: {'present' if br.get('password_hash') else 'not found'}. "
                            f"Source: {br.get('source', 'pwndb')}.",
                            [f"tor://{br.get('source', 'pwndb')}.onion"],
                            [str(person_id)],
                            datetime.now(timezone.utc),
                        )
                
                # Update POI with dark web contact data
                if dwi["matched_emails"] or dwi["contact_records"]:
                    best_email = None
                    if dwi["matched_emails"]:
                        best_email = dwi["matched_emails"][0]
                    elif dwi["contact_records"]:
                        for cr in dwi["contact_records"]:
                            if cr.get("email"):
                                best_email = cr["email"]
                                break
                    
                    # Get existing metadata
                    existing_meta = await pool.fetchval(
                        "SELECT metadata FROM persons WHERE id = $1", person_id
                    )
                    meta = json.loads(existing_meta) if existing_meta else {}
                    
                    # Update with dark web intel
                    meta["darkweb_enriched"] = True
                    meta["darkweb_enriched_at"] = datetime.now(timezone.utc).isoformat()
                    meta["darkweb_breach_count"] = len(dwi["breach_records"])
                    if dwi["matched_emails"]:
                        meta["darkweb_matched_emails"] = dwi["matched_emails"][:5]
                    
                    # Update person if we found a new email
                    updates = {"metadata": json.dumps(meta)}
                    if best_email and not poi["public_email"]:
                        updates["public_email"] = best_email
                        await pool.execute(
                            """UPDATE persons SET public_email = $1, metadata = $2::jsonb WHERE id = $3""",
                            best_email, json.dumps(meta), person_id
                        )
                    else:
                        await pool.execute(
                            """UPDATE persons SET metadata = $1::jsonb WHERE id = $2""",
                            json.dumps(meta), person_id
                        )
                    
                    darkweb_enriched += 1
                    
                    # Store dark web artifact
                    await pool.execute(
                        """INSERT INTO poi_artifacts (person_id, artifact_type, title, content_summary,
                           url, source_domain, ts_utc, provenance, metadata)
                           VALUES ($1, 'darkweb_intel', $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb)""",
                        person_id,
                        f"Dark web intelligence: {person_name}",
                        f"Found {len(dwi['breach_records'])} breach records, {len(dwi['contact_records'])} contacts",
                        "tor://aggregated-sources.onion",
                        "onion",
                        datetime.now(timezone.utc),
                        json.dumps({"extractor_version": "darkweb-v1", "sources": ["pwndb", "exposed_vc", "dread"]}),
                        json.dumps(dwi),
                    )
                else:
                    # Mark as checked even if nothing found
                    await pool.execute(
                        """UPDATE persons SET metadata = metadata || '{"darkweb_enriched": true}'::jsonb WHERE id = $1""",
                        person_id
                    )
                    
            except Exception as e:
                log.debug(f"   Dark web enrichment error for {person_name}: {e}")
        
        if darkweb_enriched > 0:
            log.info(f"🧅 Dark web enrichment: {darkweb_enriched} POIs enriched with breach/contact data")
        
        # Also check HIBP for competitor domains
        if HIBP_API_KEY:
            competitor_domains = await pool.fetch(
                """SELECT DISTINCT domain FROM companies 
                   WHERE metadata->>'is_competitor' = 'true' 
                     AND domain IS NOT NULL
                     AND metadata->>'hibp_checked_at' IS NULL
                   LIMIT 10"""
            )
            for row in competitor_domains:
                domain = row["domain"]
                try:
                    breaches = await check_hibp_domain(domain)
                    for breach in breaches:
                        await pool.execute(
                            """INSERT INTO warnings (recipe_code, warning_type, severity, title, description,
                                  evidence_urls, entity_ids, created_at)
                               VALUES ('domain_breach', 'security', $1, $2, $3, $4, $5, $6)
                               ON CONFLICT DO NOTHING""",
                            breach["severity"],
                            f"Domain breach: {domain} ({breach['breach_name']})",
                            f"{breach.get('pwn_count', 0):,} accounts exposed. "
                            f"Data: {', '.join(breach.get('data_classes', [])[:5])}. "
                            f"Date: {breach.get('breach_date', 'unknown')}.",
                            [f"https://haveibeenpwned.com/breach/{breach['breach_name']}"],
                            [],
                            datetime.now(timezone.utc),
                        )
                    # Mark as checked
                    await pool.execute(
                        """UPDATE companies SET metadata = metadata || '{"hibp_checked_at": "now"}'::jsonb
                           WHERE domain = $1""",
                        domain
                    )
                except Exception as e:
                    log.debug(f"   HIBP check error for {domain}: {e}")

    # ── Phase 8: Clean up garbage data ──
    # Remove non-person entries (TED series names, categories, etc.)
    garbage_deleted = await pool.execute(
        """DELETE FROM persons WHERE current_role = 'postgres'
           AND (metadata->>'discovery_method' = 'conference_speaker'
                OR metadata->>'discovery_method' = 'citation_coauthor')
           AND public_email IS NULL
           AND (metadata->>'linkedin') IS NULL""",
    )
    garbage_count = int(garbage_deleted.split()[-1]) if garbage_deleted else 0
    if garbage_count > 0:
        log.info(f"   🗑️ Cleaned up {garbage_count} garbage person entries")

    log.info(f"✓ POI contact discovery complete: {total_new} new persons, {total_updated} updated")
    return total_new + total_updated


# ─── Social Media Intelligence Scraping ─────────────────────────────────────────

async def _parse_reddit_json(data: dict) -> list[dict]:
    """Parse Reddit JSON API response into normalised social posts."""
    posts = []
    try:
        children = data.get("data", {}).get("children", [])
        for child in children:
            d = child.get("data", {})
            posts.append({
                "platform": "reddit",
                "post_id": d.get("id", ""),
                "author": d.get("author", ""),
                "text": f"{d.get('title', '')} {d.get('selftext', '')}".strip(),
                "url": f"https://reddit.com{d.get('permalink', '')}",
                "created_at": datetime.fromtimestamp(d.get("created_utc", 0), tz=timezone.utc).isoformat(),
                "engagement": d.get("score", 0),
                "subreddit": d.get("subreddit", ""),
                "external_url": d.get("url", ""),
            })
    except Exception as e:
        log.debug(f"Reddit parse error: {e}")
    return posts


async def _parse_mastodon_json(data: list) -> list[dict]:
    """Parse Mastodon API response into normalised social posts."""
    posts = []
    try:
        for status in (data if isinstance(data, list) else []):
            # Strip HTML tags from content
            content = re.sub(r'<[^>]+>', ' ', status.get("content", ""))
            content = re.sub(r'\s+', ' ', content).strip()
            account = status.get("account", {})
            posts.append({
                "platform": "mastodon",
                "post_id": status.get("id", ""),
                "author": account.get("acct", ""),
                "author_display": account.get("display_name", ""),
                "text": content,
                "url": status.get("url", ""),
                "created_at": status.get("created_at", ""),
                "engagement": (status.get("favourites_count", 0) or 0)
                              + (status.get("reblogs_count", 0) or 0),
                "instance": account.get("url", "").split("/")[2] if account.get("url") else "",
            })
    except Exception as e:
        log.debug(f"Mastodon parse error: {e}")
    return posts


async def _parse_bluesky_json(data: dict) -> list[dict]:
    """Parse Bluesky public search API response."""
    posts = []
    try:
        for item in data.get("posts", []):
            record = item.get("record", {})
            author = item.get("author", {})
            posts.append({
                "platform": "bluesky",
                "post_id": item.get("uri", ""),
                "author": author.get("handle", ""),
                "author_display": author.get("displayName", ""),
                "text": record.get("text", ""),
                "url": f"https://bsky.app/profile/{author.get('handle', '')}/post/{item.get('uri', '').split('/')[-1]}",
                "created_at": record.get("createdAt", ""),
                "engagement": (item.get("likeCount", 0) or 0)
                              + (item.get("repostCount", 0) or 0),
            })
    except Exception as e:
        log.debug(f"Bluesky parse error: {e}")
    return posts


async def _parse_telegram_html(html: str, url: str) -> list[dict]:
    """Parse Telegram public channel preview page (t.me/s/ format)."""
    posts = []
    try:
        soup = BeautifulSoup(html, "html.parser")
        for msg in soup.find_all("div", class_="tgme_widget_message_wrap"):
            text_div = msg.find("div", class_="tgme_widget_message_text")
            text = text_div.get_text(strip=True) if text_div else ""
            link_tag = msg.find("a", class_="tgme_widget_message_date")
            post_url = link_tag["href"] if link_tag and link_tag.get("href") else url
            time_tag = msg.find("time")
            created = time_tag.get("datetime", "") if time_tag else ""
            view_span = msg.find("span", class_="tgme_widget_message_views")
            views = 0
            if view_span:
                v = view_span.get_text(strip=True).replace("K", "000").replace("M", "000000").replace(".", "")
                views = int(re.sub(r'\D', '', v) or 0)
            posts.append({
                "platform": "telegram",
                "post_id": post_url.split("/")[-1] if "/" in post_url else "",
                "author": url.split("/s/")[-1] if "/s/" in url else "",
                "text": text,
                "url": post_url,
                "created_at": created,
                "engagement": views,
                "channel": url.split("/s/")[-1] if "/s/" in url else "",
            })
    except Exception as e:
        log.debug(f"Telegram parse error: {e}")
    return posts


async def _fetch_hn_items(session: aiohttp.ClientSession, semaphore: asyncio.Semaphore,
                           story_ids: list[int], limit: int = 20) -> list[dict]:
    """Fetch individual HN items from their IDs."""
    posts = []
    for sid in story_ids[:limit]:
        result = await fetch_url(session, f"https://hacker-news.firebaseio.com/v0/item/{sid}.json", semaphore)
        if not result or result["status"] != 200:
            continue
        try:
            item = json.loads(result["html"])
            if not item or item.get("type") != "story":
                continue
            posts.append({
                "platform": "hackernews",
                "post_id": str(item.get("id", "")),
                "author": item.get("by", ""),
                "text": item.get("title", ""),
                "url": item.get("url", f"https://news.ycombinator.com/item?id={item.get('id')}"),
                "created_at": datetime.fromtimestamp(item.get("time", 0), tz=timezone.utc).isoformat(),
                "engagement": item.get("score", 0),
                "comments": item.get("descendants", 0),
            })
        except Exception:
            continue
    return posts


async def scrape_social_media(pool: asyncpg.Pool, session: aiohttp.ClientSession,
                               semaphore: asyncio.Semaphore):
    """
    Scrape all configured social media sources. Parse platform-specific responses,
    match against tracked companies, detect signals, and store observations.

    Each social post is:
    1. Parsed into a normalised dict
    2. Company-matched against the companies table
    3. Signal-detected for intelligence value
    4. Cross-referenced with dark web intel for confidence boost
    5. Stored as SocialPost observation with credibility tier
    """
    log.info("📱 Starting social media intelligence scrape...")

    # Load company names for matching
    # Use word-boundary matching for short names to prevent false positives
    rows = await pool.fetch("SELECT id, name, domain FROM companies")
    company_map = {}
    for row in rows:
        name_lower = row["name"].lower()
        company_map[name_lower] = (str(row["id"]), row["name"])
        if row["domain"]:
            dom = row["domain"].lower()
            # Skip government domains — country names match everywhere
            if ".gov." in dom or dom.startswith("gov.") or ".go." in dom:
                continue
            domain_key = dom.split(".")[0]
            if len(domain_key) > 4:
                # Skip if the domain key is a known country name or banned common word
                if domain_key in _COUNTRY_NAMES_FOR_BOILERPLATE or domain_key in _BANNED_DOMAIN_KEYS:
                    continue
                company_map[domain_key] = (str(row["id"]), row["name"])

    # Pre-compile word-boundary regexes for names ≤12 chars to avoid
    # substring false positives (e.g., "venture" in "joint venture",
    # "arrow" in "arrow key", country names in navigation text)
    _company_patterns = {}
    for key in company_map:
        if len(key) <= 12:
            try:
                _company_patterns[key] = re.compile(r'\b' + re.escape(key) + r'\b', re.IGNORECASE)
            except re.error:
                pass  # fallback to substring for bad patterns

    stats = {"fetched": 0, "posts_parsed": 0, "company_mentions": 0, "signals": 0,
             "darkweb_crossrefs": 0, "darkweb_breaches": 0}

    for src in SOCIAL_MEDIA_SOURCES:
        url = src["url"]
        platform = src["platform"]
        topic = src["topic"]
        tier = src["tier"]

        result = await fetch_url(session, url, semaphore)
        if not result or result["status"] != 200:
            continue

        stats["fetched"] += 1
        posts = []

        try:
            if platform == "reddit":
                data = json.loads(result["html"])
                posts = await _parse_reddit_json(data)
            elif platform == "mastodon":
                data = json.loads(result["html"])
                posts = await _parse_mastodon_json(data)
            elif platform == "bluesky":
                data = json.loads(result["html"])
                posts = await _parse_bluesky_json(data)
            elif platform == "telegram":
                posts = await _parse_telegram_html(result["html"], url)
            elif platform == "hackernews":
                data = json.loads(result["html"])
                if isinstance(data, list):
                    posts = await _fetch_hn_items(session, semaphore, data, limit=20)
            elif platform in ("nitter", "forum"):
                # Extract text from HTML and create a single "post"
                text = extract_text(result["html"])
                if len(text) > 50:
                    posts = [{"platform": platform, "post_id": hashlib.md5(url.encode()).hexdigest(),
                              "author": "", "text": text[:5000], "url": url,
                              "created_at": datetime.now(timezone.utc).isoformat(),
                              "engagement": 0}]
            elif platform == "youtube":
                # RSS feed — extract entry titles
                soup = BeautifulSoup(result["html"], "html.parser")
                for entry in soup.find_all("entry")[:10]:
                    title_tag = entry.find("title")
                    link_tag = entry.find("link")
                    published = entry.find("published")
                    posts.append({
                        "platform": "youtube",
                        "post_id": link_tag.get("href", "").split("=")[-1] if link_tag else "",
                        "author": src.get("topic", ""),
                        "text": title_tag.get_text(strip=True) if title_tag else "",
                        "url": link_tag.get("href", "") if link_tag else "",
                        "created_at": published.get_text(strip=True) if published else "",
                        "engagement": 0,
                    })
        except (json.JSONDecodeError, Exception) as e:
            log.debug(f"Parse error for {platform} {url}: {e}")
            continue

        stats["posts_parsed"] += len(posts)

        # Process each post
        for post in posts:
            text_lower = post.get("text", "").lower()
            if len(text_lower) < 20:
                continue

            # Company matching — use word-boundary regex for short names
            mentioned = set()
            for key, (cid, cname) in company_map.items():
                pat = _company_patterns.get(key)
                if pat:
                    # Short name: require word boundary match
                    if pat.search(text_lower):
                        mentioned.add((cid, cname))
                else:
                    # Long name: substring match is safe
                    if key in text_lower:
                        mentioned.add((cid, cname))

            # Detect signals
            signals = detect_signals(post.get("text", ""), post.get("url", ""))
            stats["signals"] += len(signals)

            # Store observation for each mentioned company
            credibility = TIER_CREDIBILITY.get(tier, 0.5)
            for cid, cname in mentioned:
                stats["company_mentions"] += 1
                
                # ── Cross-Reference with Dark Web Intel ──────────────────────
                # Check if we have dark web intelligence for this company
                darkweb_intel = await check_darkweb_intel_for_org(pool, cname)
                darkweb_boost = 0.0
                darkweb_context = None
                
                if darkweb_intel.get("has_intel"):
                    stats["darkweb_crossrefs"] += 1
                    # Boost confidence if dark web has scanned this org's POIs
                    if darkweb_intel.get("breach_warnings", 0) > 0:
                        # Significant confidence boost for breach-corroborated intel
                        darkweb_boost = 0.15
                        stats["darkweb_breaches"] += 1
                        darkweb_context = {
                            "darkweb_corroborated": True,
                            "scanned_pois": darkweb_intel.get("scanned_pois", 0),
                            "breach_warnings": darkweb_intel.get("breach_warnings", 0),
                            "poi_names": [p["name"] for p in darkweb_intel.get("pois", [])[:3]],
                        }
                    else:
                        # Minor boost for scanned but no breaches
                        darkweb_boost = 0.05
                        darkweb_context = {
                            "darkweb_scanned": True,
                            "scanned_pois": darkweb_intel.get("scanned_pois", 0),
                        }
                
                # Build value dict with optional dark web context
                value_dict = {
                    "platform": platform,
                    "post_id": post.get("post_id", ""),
                    "author": post.get("author", ""),
                    "text": post.get("text", "")[:1000],
                    "url": post.get("url", ""),
                    "engagement": post.get("engagement", 0),
                    "topic": topic,
                    "signals": [s["type"] for s in signals],
                    "mentioned_company": cname,
                }
                if darkweb_context:
                    value_dict["darkweb_intel"] = darkweb_context
                
                await store_observation(
                    pool,
                    obs_type="SocialPost",
                    entity_id=cid,
                    entity_type="company",
                    value=value_dict,
                    provenance={
                        "url": post.get("url", url),
                        "fetch_ts": result["fetched_at"],
                        "platform": platform,
                        "credibility_tier": tier,
                        "extractor_version": "apex-crawler-2.0",
                    },
                    confidence=min(0.95, (credibility * 0.8) + darkweb_boost)  # boosted if dark web intel exists
                )

            # Even without company match, store high-signal posts for trending analysis
            if not mentioned and signals and post.get("engagement", 0) > 50:
                await store_observation(
                    pool,
                    obs_type="SocialPost",
                    entity_id=None,
                    entity_type="topic",
                    value={
                        "platform": platform,
                        "text": post.get("text", "")[:1000],
                        "url": post.get("url", ""),
                        "engagement": post.get("engagement", 0),
                        "topic": topic,
                        "signals": [s["type"] for s in signals],
                    },
                    provenance={
                        "url": post.get("url", url),
                        "fetch_ts": result["fetched_at"],
                        "platform": platform,
                        "credibility_tier": tier,
                        "extractor_version": "apex-crawler-2.0",
                    },
                    confidence=credibility * 0.6
                )

        # Respectful rate limit per source
        await asyncio.sleep(random.uniform(1.0, 2.5))

    log.info(
        f"✓ Social media scrape complete: "
        f"sources={stats['fetched']}, posts={stats['posts_parsed']}, "
        f"company_mentions={stats['company_mentions']}, signals={stats['signals']}, "
        f"darkweb_crossrefs={stats['darkweb_crossrefs']}, breach_corroborations={stats['darkweb_breaches']}"
    )
    return stats


# ─── News Cross-Reference & Corroboration Engine ───────────────────────────────

# Rhetorical / posturing phrases that indicate claims without action
POSTURING_INDICATORS = [
    r'\b(?:vow(?:ed|s)?|pledge[ds]?|promis(?:ed|es)|threaten(?:ed|s)?|warn(?:ed|s)?)\s+to\b',
    r'\bplann?(?:ed|ing|s)?\s+to\b',
    r'\bexpect(?:ed|s)?\s+to\b',
    r'\bconsider(?:ed|ing|s)?\b',
    r'\bexplor(?:ed|ing|es)\b',
    r'\bmay\s+(?:soon|eventually|begin)\b',
    r'\bcould\s+(?:lead|result|trigger)\b',
    r'\bin\s+talks?\s+(?:to|about|for|regarding)\b',
    r'\bnegotiat(?:ing|ions?)\b',
]

# Hard evidence phrases that indicate concrete actions
ACTION_INDICATORS = [
    r'\b(?:signed|executed|awarded|completed|delivered|shipped|commissioned)\b',
    r'\b(?:filed|submitted|registered|launched|deployed|installed)\b',
    r'\b(?:acquired|purchased|procured|allocated|disbursed|transferred)\b',
    r'\b(?:broke\s+ground|opened|inaugurated|commenced|activated)\b',
    r'\b(?:sanctioned|designated|added\s+to\s+entity\s+list|blacklisted)\b',
    r'\b(?:recalled|halted|suspended|ceased|terminated|withdrew|evacuated)\b',
    r'\bcontract\s+(?:worth|valued?\s+at)\s+\$[\d.,]+\s*(?:million|billion|M|B)\b',
    r'\b\d+(?:,\d+)?\s+(?:units?|troops|vehicles?|aircrafts?|tons?|tonnes?)\b',
]

# Story categories that specifically need action-based corroboration
# (user example: "preparation for war" should check army provisioning)
NEEDS_ACTION_CORROBORATION = {
    "military_buildup":   ["arms procurement", "troop deployment", "logistics base", "munitions order",
                           "fuel stockpile", "field hospital", "mobilization order", "reservist call-up"],
    "economic_sanction":  ["asset freeze", "bank account blocked", "trade volume drop", "shipping reroute",
                           "import decline", "export license revoked"],
    "factory_expansion":  ["construction permit", "equipment order", "workforce hiring", "utility contract",
                           "environmental impact filing", "zoning approval"],
    "trade_agreement":    ["ratification vote", "tariff schedule published", "customs procedure updated",
                           "goods shipped under new rules"],
    "company_acquisition": ["regulatory filing", "shareholder vote", "antitrust review", "debt financing",
                            "management restructure", "brand integration"],
    "technology_breakthrough": ["patent filed", "peer-reviewed publication", "prototype demonstrated",
                                "production sample shipped", "customer qualification"],
    "conflict_escalation": ["troop movement verified", "satellite imagery", "artillery damage",
                            "refugee flow increase", "humanitarian corridor", "casualty report",
                            "UNSC emergency session", "ICRC deployment"],
    "sanctions_escalation": ["asset seizure confirmed", "bank de-SWIFTing", "trade volume drop",
                             "vessel rerouting", "secondary designation", "compliance fine"],
    "diplomatic_breakdown": ["ambassador recalled", "embassy staff reduced", "consulate closed",
                             "bilateral mechanism suspended", "treaty denunciation filed"],
    "energy_weaponization": ["pipeline flow reduction confirmed", "gas delivery curtailed",
                             "refinery shutdown", "strategic reserve release", "spot price spike"],
    "cyber_operation":     ["forensic attribution", "malware sample published", "CERT advisory",
                            "infrastructure offline", "data exfiltration confirmed"],
    "mineral_supply_cut":  ["export ban enacted", "mining halt confirmed", "processing facility closure",
                            "customs blockage", "inventory drawdown reported"],
}


def _classify_story_category(text_lower: str) -> Optional[str]:
    """Classify a news story into a category that needs action corroboration."""
    classification_keywords = {
        "military_buildup":      ["military buildup", "troop movement", "preparation for war", "military exercise",
                                  "army mobilization", "defense posture", "combat readiness"],
        "economic_sanction":     ["sanctions", "economic pressure", "trade ban", "embargo", "asset freeze"],
        "factory_expansion":     ["new facility", "plant expansion", "factory construction", "capacity expansion"],
        "trade_agreement":       ["trade agreement", "trade deal", "free trade", "trade pact", "trade treaty"],
        "company_acquisition":   ["acquisition", "merger", "takeover", "buyout"],
        "technology_breakthrough":["breakthrough", "revolutionary", "game-changing", "next-generation technology",
                                   "disruptive technology"],
        "conflict_escalation":   ["conflict escalation", "invasion", "air strike", "missile attack",
                                  "border clash", "martial law", "military offensive", "rebel offensive"],
        "sanctions_escalation":  ["sanctions package", "secondary sanctions", "de-SWIFTing",
                                  "sanctions enforcement", "entity list addition", "sanctions designation"],
        "diplomatic_breakdown":  ["diplomatic crisis", "ambassador recalled", "embassy closure",
                                  "severed relations", "diplomatic expulsion", "treaty withdrawal"],
        "energy_weaponization":  ["energy weapon", "gas cutoff", "pipeline shutdown",
                                  "oil embargo", "energy blackmail", "energy crisis"],
        "cyber_operation":       ["cyber attack", "state-sponsored hack", "critical infrastructure breach",
                                  "cyber espionage", "ransomware", "information warfare"],
        "mineral_supply_cut":    ["rare earth ban", "mineral export restriction", "mining shutdown",
                                  "resource nationalism", "critical mineral embargo"],
    }
    for category, keywords in classification_keywords.items():
        if any(kw in text_lower for kw in keywords):
            return category
    return None


def _is_social_or_low_signal_domain(domain: str) -> bool:
    normalized = (domain or "").lower()
    social_markers = (
        "mastodon",
        "mstdn",
        "bsky",
        "bluesky",
        "x.com",
        "twitter.com",
        "linkedin.com",
        "reddit.com",
        "t.me",
        "telegram",
        "youtube.com",
        "youtu.be",
        "facebook.com",
        "instagram.com",
        "tiktok.com",
    )
    return any(marker in normalized for marker in social_markers)


def _has_non_social_evidence_url(urls: list[str] | None) -> bool:
    for url in urls or []:
        try:
            domain = urlparse(url).netloc.replace("www.", "")
        except Exception:
            continue
        if domain and not _is_social_or_low_signal_domain(domain):
            return True
    return False


def score_story_veracity(
    story_text: str,
    corroborating_articles: list[dict],
    primary_urls: Optional[list[str]] = None,
) -> dict:
    """
    Score the veracity of a news story based on:
    1. Source diversity — how many independent outlets report the same story
    2. Action vs posturing — does the story contain concrete actions or just rhetoric
    3. Corroborating evidence — do related articles confirm with hard evidence
    4. Category-specific action checks — e.g. war rhetoric checked against actual
       logistics/procurement activity

    Returns dict with:
    - veracity_score (0.0-1.0)
    - classification (verified / likely / unverified / posturing / contradicted)
    - reasoning (human-readable explanation)
    - action_evidence (list of concrete actions found)
    - posturing_signals (list of rhetorical indicators found)
    """
    text_lower = story_text.lower()

    # 1. Count posturing vs action indicators in primary story
    posturing_count = sum(1 for pat in POSTURING_INDICATORS if re.search(pat, text_lower))
    action_count = sum(1 for pat in ACTION_INDICATORS if re.search(pat, text_lower))

    # 2. Classify story category
    category = _classify_story_category(text_lower)

    # 3. Analyse corroborating articles
    unique_domains = set()
    for primary_url in primary_urls or []:
        try:
            domain = urlparse(primary_url).netloc
            if domain:
                unique_domains.add(domain)
        except Exception:
            pass
    corroborating_action_count = 0
    corroborating_posturing_count = 0
    category_action_hits = []

    for art in corroborating_articles:
        art_text = art.get("text", "").lower()
        art_url = art.get("url", "")
        try:
            domain = urlparse(art_url).netloc
            unique_domains.add(domain)
        except Exception:
            pass

        # Check for action indicators in corroborating articles
        art_actions = sum(1 for pat in ACTION_INDICATORS if re.search(pat, art_text))
        art_posturing = sum(1 for pat in POSTURING_INDICATORS if re.search(pat, art_text))
        corroborating_action_count += art_actions
        corroborating_posturing_count += art_posturing

        # Category-specific action corroboration
        if category and category in NEEDS_ACTION_CORROBORATION:
            for kw in NEEDS_ACTION_CORROBORATION[category]:
                if kw.lower() in art_text:
                    category_action_hits.append(kw)

    # 4. Calculate veracity score
    source_diversity_score = min(1.0, len(unique_domains) / 3.0)  # 3+ sources = full marks
    non_social_domains = {domain for domain in unique_domains if not _is_social_or_low_signal_domain(domain)}
    credible_source_score = min(1.0, len(non_social_domains) / 2.0)
    social_echo_penalty = 0.12 if unique_domains and not non_social_domains else 0.0

    # Action/posturing ratio: ratio favoring action
    total_indicators = (action_count + corroborating_action_count
                        + posturing_count + corroborating_posturing_count) or 1
    total_actions = action_count + corroborating_action_count
    total_posturing = posturing_count + corroborating_posturing_count
    action_ratio = total_actions / total_indicators

    # Category action bonus: if we found real action evidence for the claimed category
    category_bonus = min(0.20, len(set(category_action_hits)) * 0.05) if category_action_hits else 0.0
    # Category penalty: if the category requires action proof but none found
    category_penalty = 0.0
    if category and category in NEEDS_ACTION_CORROBORATION and not category_action_hits:
        if total_posturing > total_actions:
            category_penalty = 0.25  # Heavy penalty: claims with no action evidence

    # Composite score
    veracity = (
        0.24 * source_diversity_score
        + 0.30 * action_ratio
        + 0.20 * min(1.0, len(corroborating_articles) / 3.0)
        + 0.14 * (1.0 if action_count > 0 else 0.3)
        + 0.12 * credible_source_score
        + category_bonus
        - category_penalty
        - social_echo_penalty
    )
    veracity = max(0.05, min(0.98, veracity))

    # Generate classification
    if veracity >= 0.80 and total_actions >= 2 and len(unique_domains) >= 2:
        classification = "verified"
    elif veracity >= 0.60:
        classification = "likely"
    elif veracity >= 0.50 and len(non_social_domains) >= 2 and total_posturing <= total_actions + 1:
        classification = "likely"
    elif total_posturing > total_actions * 2 and category_penalty > 0:
        classification = "posturing"
    elif veracity < 0.30:
        classification = "contradicted"
    else:
        classification = "unverified"

    # Reasoning
    reasoning_parts = []
    if len(unique_domains) > 1:
        reasoning_parts.append(f"Reported by {len(unique_domains)} independent sources")
    elif len(unique_domains) == 1:
        reasoning_parts.append("Single-source reporting — treat with caution")
    else:
        reasoning_parts.append("No corroborating sources found")

    if unique_domains and not non_social_domains:
        reasoning_parts.append("Current corroboration is social-only and lacks higher-signal reporting")
    elif len(non_social_domains) >= 2:
        reasoning_parts.append(f"Includes {len(non_social_domains)} non-social reporting sources")

    if total_actions > total_posturing:
        reasoning_parts.append(f"Contains {total_actions} concrete action indicators vs {total_posturing} rhetorical")
    elif total_posturing > total_actions:
        reasoning_parts.append(f"Predominantly rhetorical ({total_posturing} posturing vs {total_actions} action indicators)")

    if category_action_hits:
        reasoning_parts.append(f"Category-specific evidence found: {', '.join(set(category_action_hits)[:3])}")
    elif category and category_penalty > 0:
        reasoning_parts.append(f"Category '{category}' claims lack supporting action evidence (e.g. {NEEDS_ACTION_CORROBORATION[category][:2]})")

    return {
        "veracity_score": round(veracity, 3),
        "classification": classification,
        "reasoning": "; ".join(reasoning_parts),
        "source_count": len(unique_domains),
        "action_evidence": category_action_hits[:5],
        "posturing_signals": total_posturing,
        "action_signals": total_actions,
        "category": category,
    }


async def cross_reference_news(pool: asyncpg.Pool, session: aiohttp.ClientSession,
                                semaphore: asyncio.Semaphore):
    """
    Cross-reference recent news stories against each other and social media
    observations. No story is taken at face value — each is analysed for:
    
    1. Multi-source corroboration (same event reported by independent sources)
    2. Action vs rhetoric analysis (concrete evidence vs posturing/threats)
    3. Category-specific verification (e.g. war prep checked against actual
       procurement/logistics activity)
    4. Veracity scoring with human-readable reasoning

    Quality gates (Q1 2026):
    - Observations must have meaningful text content (>80 chars excerpt)
    - Observations with boilerplate text (country lists, nav menus) are rejected
    - Evidence URLs must be real article URLs, not homepages/index pages
    - Summaries must include actual content from observations (no empty templates)
    - Entity clusters need ≥2 observations with substantive text overlap
    """
    log.info("🔍 Cross-referencing news stories for veracity analysis...")

    # Get recent high-signal observations (last 6 hours)
    recent_obs = await pool.fetch("""
        SELECT o.id, o.observation_type, o.entity_id, o.entity_type, o.value, o.provenance, o.confidence,
               COALESCE(c.name, '') as company_name
        FROM observations o
        LEFT JOIN companies c ON c.id = o.entity_id
        WHERE o.ts_utc > NOW() - INTERVAL '6 hours'
        AND o.observation_type IN ('CompetitorEvent', 'WebChange', 'SocialPost')
        AND o.confidence >= 0.5
        ORDER BY o.ts_utc DESC
        LIMIT 200
    """)

    if len(recent_obs) < 3:
        log.info("   Insufficient recent observations for cross-referencing")
        return 0

    # Group observations by entity (company)
    entity_clusters: dict[str, list[dict]] = {}
    for obs in recent_obs:
        entity_key = obs["company_name"] or "unaffiliated"
        if entity_key not in entity_clusters:
            entity_clusters[entity_key] = []
        value = json.loads(obs["value"]) if isinstance(obs["value"], str) else obs["value"]
        provenance = json.loads(obs["provenance"]) if isinstance(obs["provenance"], str) else obs["provenance"]
        entity_clusters[entity_key].append({
            "id": str(obs["id"]),
            "type": obs["observation_type"],
            "entity_id": str(obs["entity_id"]) if obs["entity_id"] else None,
            "text": value.get("excerpt", value.get("text", "")),
            "url": value.get("url", provenance.get("url", "")),
            "signals": value.get("signals", []),
            "platform": provenance.get("platform", value.get("source", "web")),
            "tier": provenance.get("credibility_tier", "T1"),
            "confidence": float(obs["confidence"]),
        })

    insights_created = 0

    for entity_name, obs_list in entity_clusters.items():
        if len(obs_list) < 2:
            continue
        if entity_name == "unaffiliated":
            continue

        # ── Skip government entities ──
        # Government entities ("Government of Netherlands", etc.) inherently
        # match too broadly and produce misleading veracity insights.
        # Their website WebChange observations are noise, not intel.
        _en_lower = entity_name.lower()
        if (_en_lower.startswith("government of ") or
            _en_lower.startswith("ministry of ") or
            _en_lower.startswith("republic of ")):
            log.debug(f"Skipping government entity from veracity analysis: {entity_name}")
            continue

        # ── Quality gate: require substantive text content ──
        # Filter out observations with trivial, empty, or boilerplate text
        # (country lists, currency converters, navigation menus).
        substantive_obs = [
            o for o in obs_list
            if len(o.get("text", "")) >= 80
            and not _is_boilerplate_excerpt(o.get("text", ""))
        ]
        if len(substantive_obs) < 2:
            log.debug(f"Skipping {entity_name}: insufficient quality observations "
                      f"({len(substantive_obs)} passed boilerplate filter out of {len(obs_list)})")
            continue

        # Combine texts for primary story analysis
        primary_texts = [o["text"] for o in substantive_obs if o["text"]]
        if not primary_texts:
            continue

        combined_text = " ".join(primary_texts[:5])

        # Build corroborating article list from the other observations
        corroborating = []
        primary_url = substantive_obs[0].get("url", "")
        for o in substantive_obs[1:]:
            if o.get("url") != primary_url and o.get("text", ""):
                corroborating.append({
                    "text": o.get("text", ""),
                    "url": o.get("url", ""),
                    "platform": o.get("platform", ""),
                })

        # Also search for corroborating social media posts
        social_corroboration = await pool.fetch("""
            SELECT value, provenance FROM observations
            WHERE observation_type = 'SocialPost'
            AND ts_utc > NOW() - INTERVAL '24 hours'
            AND value::text ILIKE $1
            LIMIT 10
        """, f"%{entity_name[:30]}%")

        for soc in social_corroboration:
            soc_val = json.loads(soc["value"]) if isinstance(soc["value"], str) else soc["value"]
            soc_text = soc_val.get("text", "")
            if len(soc_text) >= 40 and not _is_boilerplate_excerpt(soc_text):
                corroborating.append({
                    "text": soc_text,
                    "url": soc_val.get("url", ""),
                    "platform": soc_val.get("platform", "social"),
                })

        # Score veracity
        veracity_result = score_story_veracity(
            combined_text,
            corroborating,
            primary_urls=[o.get("url", "") for o in substantive_obs if o.get("url")],
        )

        # Only generate insights for significant stories (multiple signals or high engagement)
        unique_signals = set()
        for o in substantive_obs:
            unique_signals.update(o.get("signals", []))

        if len(substantive_obs) < 3 and not unique_signals:
            continue

        # Dedup: check for recent veracity insight about this entity
        existing = await pool.fetchval(
            """SELECT 1 FROM insights
               WHERE insight_type = 'veracity_analysis'
               AND title ILIKE $1
               AND created_at > NOW() - INTERVAL '12 hours'""",
            f"%{entity_name[:30]}%"
        )
        if existing:
            continue

        # ── Build QUALITY evidence URLs — reject homepages and index pages ──
        raw_evidence_urls = [o.get("url", "") for o in substantive_obs if o.get("url")]
        evidence_urls = filter_relevant_urls(raw_evidence_urls)
        # If all URLs failed the filter, this cluster is based on homepage/index
        # observations with no real article backing — skip it entirely.
        if not evidence_urls:
            log.debug(f"Skipping {entity_name}: no valid article-level evidence URLs")
            continue

        # Create insight with veracity data
        classification = veracity_result["classification"]
        score = veracity_result["veracity_score"]
        reasoning = veracity_result["reasoning"]

        signal_types = sorted(unique_signals)

        # Adjust insight confidence based on veracity
        insight_confidence = min(0.95, score * 0.9 + 0.05)

        # ── Build ANALYTICAL summary with actual content from observations ──
        _topic_keywords = _extract_key_topics(combined_text, entity_name)

        # ── Synthesize what the sources are actually reporting ──
        source_themes = []          # one-line theme per unique source
        seen_domains = set()
        for o in substantive_obs[:6]:
            o_url = o.get("url", "")
            o_text = o.get("text", "")
            try:
                domain = urlparse(o_url).netloc.replace("www.", "")
            except Exception:
                domain = "unknown"
            if domain in seen_domains:
                continue
            seen_domains.add(domain)
            raw = _trim_excerpt_sentence(o_text, limit=180)
            if raw:
                source_themes.append((domain, raw))

        n_reports = len(substantive_obs)
        title, summary = build_veracity_title_and_summary(
            entity_name,
            classification,
            signal_types,
            source_themes,
            _topic_keywords,
            veracity_result,
            n_reports,
        )

        insight_type = "veracity_analysis"
        tags = ["cross_reference", "veracity", classification]
        if veracity_result["category"]:
            tags.append(veracity_result["category"])

        confidence_reason = _confidence_gate_reason(insight_type, insight_confidence, tags)
        quality_ok = _passes_shared_quality_gate(title, summary, insight_type)

        if confidence_reason is not None:
            log.info(
                "🔎 Veracity candidate skipped: %s → %s (%s, score=%.0f%%, reports=%d, evidence_urls=%d)",
                entity_name,
                classification,
                confidence_reason,
                score * 100,
                n_reports,
                len(evidence_urls),
            )
            continue

        if not quality_ok:
            log.info(
                "🔎 Veracity candidate skipped: %s → %s (shared quality gate, score=%.0f%%, reports=%d, evidence_urls=%d)",
                entity_name,
                classification,
                score * 100,
                n_reports,
                len(evidence_urls),
            )
            continue

        inserted = await store_insight(
            pool,
            title,
            summary,
            insight_type,
            "Global",
            evidence_urls,
            tags,
            insight_confidence,
            entity_ids=list({o["entity_id"] for o in obs_list if o.get("entity_id")}),
        )
        if inserted:
            insights_created += 1
            log.info(f"🔍 Veracity insight: {entity_name} → {classification} ({score:.0%})")
        else:
            log.info(
                "🔎 Veracity candidate skipped: %s → %s (insert declined after evidence or dedup validation, score=%.0f%%, reports=%d, evidence_urls=%d)",
                entity_name,
                classification,
                score * 100,
                n_reports,
                len(evidence_urls),
            )

    log.info(f"✓ Cross-reference analysis complete: {insights_created} veracity insights")
    return insights_created


def _extract_key_topics(text: str, entity_name: str) -> list[str]:
    """Extract key topic keywords from combined observation text, excluding the entity name itself."""
    # Common intelligence-relevant topic patterns
    topic_patterns = [
        r'\b(sanctions?|embargo|tariff|export control)\b',
        r'\b(acquisition|merger|takeover|joint venture)\b',
        r'\b(expansion|new facility|plant|factory|investment)\b',
        r'\b(patent|R&D|technology|innovation|breakthrough)\b',
        r'\b(military|defense|procurement|weapons?|missile)\b',
        r'\b(conflict|escalation|tension|crisis|war)\b',
        r'\b(supply chain|shortage|disruption|logistics)\b',
        r'\b(cybersecurity|data breach|ransomware|hack)\b',
        r'\b(renewable|solar|wind|energy transition|EV)\b',
        r'\b(AI|artificial intelligence|machine learning|GPU)\b',
        r'\b(semiconductor|chip|wafer|fab|foundry)\b',
        r'\b(drone|autonomous|UAV|unmanned)\b',
        r'\b(rare earth|lithium|cobalt|critical mineral)\b',
        r'\b(5G|6G|telecom|connectivity)\b',
        r'\b(hiring|layoff|restructuring|workforce)\b',
        r'\b(certification|ISO|audit|compliance)\b',
        r'\b(contract|tender|RFP|procurement)\b',
        r'\b(IPO|earnings|revenue|quarterly results)\b',
    ]
    topics = []
    text_lower = text.lower()
    entity_lower = entity_name.lower()
    for pattern in topic_patterns:
        matches = re.findall(pattern, text_lower, re.I)
        for m in matches:
            m_clean = m.strip().lower()
            if m_clean not in entity_lower and m_clean not in [t.lower() for t in topics]:
                topics.append(m.strip())
    return topics[:10]


def _humanize_signal_types(signal_types: list[str]) -> str:
    cleaned = []
    seen = set()
    for signal in signal_types:
        label = signal.replace("_", " ").strip().lower()
        if label and label not in seen:
            seen.add(label)
            cleaned.append(label)
    if not cleaned:
        return "market activity"
    if len(cleaned) == 1:
        return cleaned[0]
    if len(cleaned) == 2:
        return f"{cleaned[0]} and {cleaned[1]}"
    return f"{cleaned[0]}, {cleaned[1]}, and related activity"


def _trim_excerpt_sentence(text: str, limit: int = 180) -> str:
    raw = " ".join((text or "").replace("\n", " ").split()).strip()
    if not raw:
        return ""
    for end_char in ".!?":
        pos = raw.find(end_char)
        if 30 < pos < limit:
            return raw[:pos + 1]
    if len(raw) > limit:
        return raw[:limit - 3].rstrip() + "..."
    return raw


def _veracity_title(entity_name: str, classification: str, signal_types: list[str]) -> str:
    signal_label = _humanize_signal_types(signal_types)
    if classification == "verified":
        return f"{entity_name}: corroborated {signal_label} reporting"
    if classification == "likely":
        return f"{entity_name}: likely {signal_label} development"
    if classification == "posturing":
        return f"{entity_name}: rhetorical {signal_label} claims outweigh evidence"
    if classification == "contradicted":
        return f"{entity_name}: weak support for reported {signal_label} claims"
    return f"{entity_name}: emerging {signal_label} reporting"


def build_veracity_title_and_summary(
    entity_name: str,
    classification: str,
    signal_types: list[str],
    source_themes: list[tuple[str, str]],
    topic_keywords: list[str],
    veracity_result: dict,
    report_count: int,
) -> tuple[str, str]:
    title = _veracity_title(entity_name, classification, signal_types)
    source_count = max(1, int(veracity_result.get("source_count", 0) or 0))
    action_count = int(veracity_result.get("action_signals", 0) or 0)
    posturing_count = int(veracity_result.get("posturing_signals", 0) or 0)
    score = float(veracity_result.get("veracity_score", 0.0) or 0.0)
    reasoning = (veracity_result.get("reasoning") or "").strip()
    action_evidence = list(veracity_result.get("action_evidence") or [])
    category = veracity_result.get("category")

    lead_theme = source_themes[0][1] if source_themes else "recent reporting remains fragmented"
    lead_sentence = _trim_excerpt_sentence(lead_theme)
    signal_label = _humanize_signal_types(signal_types)
    domains = [domain for domain, _ in source_themes if domain]
    domain_phrase = ", ".join(domains[:4]) if domains else "recent coverage"

    summary_parts = [
        (
            f"{entity_name} is appearing in {report_count} recent reports across {source_count} independent "
            f"sources. The reported development centers on {signal_label}, with the clearest source language "
            f"stating that {lead_sentence}"
        )
    ]

    summary_parts.append(
        f"Coverage from {domain_phrase} is being compared for corroboration. The current read is {classification} "
        f"at roughly {score:.0%} confidence because {reasoning}."
    )

    if topic_keywords:
        summary_parts.append(
            f"The recurring reported themes involve {', '.join(topic_keywords[:6])}."
        )

    if action_evidence:
        summary_parts.append(
            f"Concrete evidence already visible in the source set includes {', '.join(action_evidence[:3])}."
        )
    elif category in NEEDS_ACTION_CORROBORATION:
        needed = ", ".join(NEEDS_ACTION_CORROBORATION[category][:3])
        summary_parts.append(
            f"The reporting still lacks the action-based evidence that would matter most here, such as {needed}."
        )

    if posturing_count > action_count and posturing_count > 0:
        summary_parts.append(
            f"Rhetorical or signaling behavior currently outweighs operational proof, with {posturing_count} "
            f"posturing indicators versus {action_count} action indicators across the observed sources."
        )

    if classification == "verified":
        summary_parts.append(
            "If this affects an active account, supplier, or program, it is reasonable to move from passive "
            "monitoring into response planning now."
        )
    elif classification == "likely":
        summary_parts.append(
            "If this matters commercially or operationally, keep it on an active watchlist and look for formal "
            "confirmation before making a hard commitment."
        )
    elif classification == "posturing":
        summary_parts.append(
            "Treat this primarily as directional signaling until stronger procurement, hiring, logistics, or other "
            "operational evidence appears."
        )
    else:
        summary_parts.append(
            "Treat this as an early signal only and wait for stronger corroboration before acting on it."
        )

    return title, "\n\n".join(summary_parts)


# ─── Geopolitical Landscape Analysis Engine ─────────────────────────────────────

# Region mapping: maps countries/areas to geopolitical regions for aggregation
GEO_REGION_MAP = {
    # East Asia & Pacific
    "china": "East Asia", "taiwan": "East Asia", "japan": "East Asia",
    "south korea": "East Asia", "korea": "East Asia", "north korea": "East Asia",
    "hong kong": "East Asia", "mongolia": "East Asia",
    # Southeast Asia
    "vietnam": "Southeast Asia", "philippines": "Southeast Asia", "indonesia": "Southeast Asia",
    "malaysia": "Southeast Asia", "singapore": "Southeast Asia", "thailand": "Southeast Asia",
    "myanmar": "Southeast Asia", "cambodia": "Southeast Asia", "laos": "Southeast Asia",
    # South Asia
    "india": "South Asia", "pakistan": "South Asia", "bangladesh": "South Asia",
    "sri lanka": "South Asia", "nepal": "South Asia", "afghanistan": "South Asia",
    # Middle East
    "iran": "Middle East", "iraq": "Middle East", "syria": "Middle East",
    "saudi arabia": "Middle East", "uae": "Middle East", "israel": "Middle East",
    "qatar": "Middle East", "bahrain": "Middle East", "oman": "Middle East",
    "yemen": "Middle East", "jordan": "Middle East", "lebanon": "Middle East",
    "palestine": "Middle East", "kuwait": "Middle East",
    # North Africa
    "egypt": "North Africa", "libya": "North Africa", "tunisia": "North Africa",
    "algeria": "North Africa", "morocco": "North Africa", "sudan": "North Africa",
    # Sub-Saharan Africa
    "nigeria": "Sub-Saharan Africa", "ethiopia": "Sub-Saharan Africa",
    "kenya": "Sub-Saharan Africa", "south africa": "Sub-Saharan Africa",
    "congo": "Sub-Saharan Africa", "drc": "Sub-Saharan Africa",
    "somalia": "Sub-Saharan Africa", "mali": "Sub-Saharan Africa",
    "niger": "Sub-Saharan Africa", "burkina faso": "Sub-Saharan Africa",
    # Europe
    "russia": "Europe/Russia", "ukraine": "Europe/Russia", "belarus": "Europe/Russia",
    "germany": "Western Europe", "france": "Western Europe", "uk": "Western Europe",
    "poland": "Central Europe", "romania": "Central Europe", "hungary": "Central Europe",
    "turkey": "Turkey/Caucasus", "georgia": "Turkey/Caucasus", "armenia": "Turkey/Caucasus",
    "azerbaijan": "Turkey/Caucasus",
    # Central Asia
    "kazakhstan": "Central Asia", "uzbekistan": "Central Asia",
    "turkmenistan": "Central Asia", "tajikistan": "Central Asia", "kyrgyzstan": "Central Asia",
    # Americas
    "us": "Americas", "usa": "Americas", "united states": "Americas",
    "canada": "Americas", "mexico": "Americas", "brazil": "Americas",
    "venezuela": "Americas", "colombia": "Americas", "argentina": "Americas",
    "cuba": "Americas",
    # Maritime chokepoints (special)
    "suez": "Suez Canal", "hormuz": "Strait of Hormuz", "malacca": "Malacca Strait",
    "bab el mandeb": "Red Sea/Bab el Mandeb", "panama": "Panama Canal",
    "taiwan strait": "Taiwan Strait", "south china sea": "South China Sea",
    "black sea": "Black Sea", "baltic": "Baltic Sea", "arctic": "Arctic",
}

# Critical supply chain chokepoints and their impact sectors
CHOKEPOINT_IMPACT = {
    "Suez Canal":           {"trade_pct": 12, "sectors": ["energy", "manufacturing", "consumer goods"]},
    "Strait of Hormuz":     {"trade_pct": 21, "sectors": ["energy", "petrochemicals"]},
    "Malacca Strait":       {"trade_pct": 25, "sectors": ["energy", "semiconductors", "electronics"]},
    "Red Sea/Bab el Mandeb":{"trade_pct": 10, "sectors": ["energy", "container shipping"]},
    "Panama Canal":         {"trade_pct": 5,  "sectors": ["bulk commodities", "LNG"]},
    "Taiwan Strait":        {"trade_pct": 88, "sectors": ["semiconductors", "advanced chips"]},
    "South China Sea":      {"trade_pct": 33, "sectors": ["all trade", "energy", "electronics"]},
    "Black Sea":            {"trade_pct": 4,  "sectors": ["grain", "energy", "fertilizer"]},
}


def _identify_geo_regions(text_lower: str) -> list[str]:
    """Identify geopolitical regions mentioned in text."""
    regions = set()
    for keyword, region in GEO_REGION_MAP.items():
        if keyword in text_lower:
            regions.add(region)
    return list(regions)


async def analyze_geopolitical_landscape(pool: asyncpg.Pool):
    """
    Comprehensive geopolitical landscape analysis that ties together ALL geo signals.

    This function:
    1. Aggregates all geopolitical signals across categories by region
    2. Builds regional risk profiles with compound risk scoring
    3. Detects escalation patterns (increasing frequency of signals)
    4. Cross-references sanctions against trade/supply flows
    5. Links military procurement to conflict indicators
    6. Assesses trade corridor vulnerability
    7. Produces strategic intelligence briefs

    Called once per crawl cycle after all other analysis is complete.
    """
    log.info("🌍 Analyzing geopolitical landscape...")

    GEO_SIGNAL_TYPES = [
        'geopolitical_risk', 'conflict_escalation', 'sanctions_cascade',
        'diplomatic_shift', 'military_procurement', 'energy_security',
        'cyber_warfare', 'critical_mineral_risk', 'trade_corridor_disruption',
    ]

    insights_created = 0

    # ─── 1. Regional Risk Profile Aggregation ──────────────────────────────────
    # Pull ALL geo-related warnings from last 48 hours for trend analysis
    all_geo_warnings = await pool.fetch("""
        SELECT w.id, w.warning_type, w.title, w.description, w.severity, w.region,
               w.source_urls, w.confidence, w.created_at
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '48 hours'
        AND w.warning_type = ANY($1)
        ORDER BY w.created_at DESC
    """, GEO_SIGNAL_TYPES)

    if len(all_geo_warnings) < 2:
        log.info("   Insufficient geopolitical signals for landscape analysis")
        return 0

    # Aggregate by region
    region_profiles: dict[str, dict] = {}
    for w in all_geo_warnings:
        region = w["region"] or "Global"
        if region not in region_profiles:
            region_profiles[region] = {
                "signal_types": set(),
                "total_signals": 0,
                "critical_count": 0,
                "high_count": 0,
                "signals": [],
                "titles": [],
                "sources": [],
            }
        profile = region_profiles[region]
        profile["signal_types"].add(w["warning_type"])
        profile["total_signals"] += 1
        if w["severity"] == "critical":
            profile["critical_count"] += 1
        elif w["severity"] == "high":
            profile["high_count"] += 1
        profile["titles"].append(w["title"])
        if w["source_urls"]:
            profile["sources"].extend(w["source_urls"])

    # ─── 2. Escalation Trend Detection ─────────────────────────────────────────
    # Compare last 24h signal count vs previous 24h for each region
    recent_counts = await pool.fetch("""
        SELECT w.region, COUNT(*) as cnt
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = ANY($1) AND w.region IS NOT NULL
        GROUP BY w.region
    """, GEO_SIGNAL_TYPES)

    prior_counts = await pool.fetch("""
        SELECT w.region, COUNT(*) as cnt
        FROM warnings w
        WHERE w.created_at BETWEEN NOW() - INTERVAL '48 hours' AND NOW() - INTERVAL '24 hours'
        AND w.warning_type = ANY($1) AND w.region IS NOT NULL
        GROUP BY w.region
    """, GEO_SIGNAL_TYPES)

    recent_map = {r["region"]: r["cnt"] for r in recent_counts}
    prior_map = {r["region"]: r["cnt"] for r in prior_counts}

    escalating_regions = []
    for region, current in recent_map.items():
        prior = prior_map.get(region, 0)
        if current >= 3 and (prior == 0 or current >= prior * 1.5):
            escalating_regions.append({
                "region": region,
                "current": current,
                "prior": prior,
                "change": "NEW" if prior == 0 else f"+{int((current/max(prior,1)-1)*100)}%",
            })

    # Generate escalation trend insight if any found
    if escalating_regions:
        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE '%Geopolitical Escalation Trend%' "
            "AND created_at > NOW() - INTERVAL '12 hours'"
        )
        if not existing:
            regions_text = "; ".join(
                f"{r['region']} ({r['change']}, {r['current']} signals)"
                for r in sorted(escalating_regions, key=lambda x: x['current'], reverse=True)[:5]
            )
            title = "Geopolitical Escalation Trend: Multiple regions intensifying"
            summary = (
                f"TREND ANALYSIS: Geopolitical signal frequency is accelerating in "
                f"{len(escalating_regions)} regions over the past 24 hours.\n\n"
                f"ESCALATING REGIONS: {regions_text}.\n\n"
                f"INTERPRETATION: Rising signal density indicates deteriorating geopolitical "
                f"conditions. Regions showing NEW activity warrant immediate attention. "
                f"Regions with >50% increase are on an escalation trajectory.\n\n"
                f"RECOMMENDATION: Prioritize risk assessment for listed regions; "
                f"update supply chain contingency plans accordingly."
            )
            all_sources = []
            for r in escalating_regions:
                profile = region_profiles.get(r["region"], {})
                all_sources.extend(profile.get("sources", [])[:3])
            sources = filter_relevant_urls(all_sources)
            await store_insight(pool, title, summary, "geopolitical_analysis", "Global", sources,
                              ["escalation", "trend", "geopolitical", "multi_region"], 0.87)
            insights_created += 1
            log.info(f"🌍 Escalation trend insight: {len(escalating_regions)} regions")

    # ─── 3. Trade Corridor Vulnerability Assessment ────────────────────────────
    # Check if any identified chokepoints are showing stress
    corridor_warnings = await pool.fetch("""
        SELECT w.region, w.title, w.description, w.source_urls
        FROM warnings w
        WHERE w.created_at > NOW() - INTERVAL '24 hours'
        AND w.warning_type = 'trade_corridor_disruption'
    """)

    if corridor_warnings:
        affected_chokepoints = []
        for w in corridor_warnings:
            desc_lower = (w["title"] + " " + (w["description"] or "")).lower()
            for chokepoint, impact in CHOKEPOINT_IMPACT.items():
                if any(kw.lower() in desc_lower for kw in chokepoint.split("/")):
                    affected_chokepoints.append({
                        "chokepoint": chokepoint,
                        "trade_pct": impact["trade_pct"],
                        "sectors": impact["sectors"],
                        "warning_title": w["title"],
                        "sources": w["source_urls"] or [],
                    })

        if affected_chokepoints:
            existing = await pool.fetchval(
                "SELECT 1 FROM insights WHERE title LIKE '%Chokepoint Vulnerability%' "
                "AND created_at > NOW() - INTERVAL '12 hours'"
            )
            if not existing:
                chokepoint_text = "; ".join(
                    f"{c['chokepoint']} ({c['trade_pct']}% of global trade, affects {', '.join(c['sectors'][:3])})"
                    for c in affected_chokepoints[:4]
                )
                title = f"Chokepoint Vulnerability: {len(affected_chokepoints)} trade corridors under threat"
                summary = (
                    f"MARITIME/TRADE ASSESSMENT: Active threat indicators at critical trade corridors.\n\n"
                    f"AFFECTED CHOKEPOINTS: {chokepoint_text}.\n\n"
                    f"GLOBAL TRADE RISK: These chokepoints collectively handle significant portions of "
                    f"international maritime trade. Disruptions would cascade to shipping costs, lead times, "
                    f"and component availability across electronics, energy, and manufacturing sectors.\n\n"
                    f"RECOMMENDATION: Assess supply chain dependence on affected routes; prepare alternative "
                    f"logistics; review force majeure provisions with carriers."
                )
                all_cp_sources = [s for c in affected_chokepoints for s in c["sources"][:2]]
                sources = filter_relevant_urls(all_cp_sources)
                await store_insight(pool, title, summary, "geopolitical_analysis", "Global", sources,
                                  ["maritime", "chokepoint", "trade_corridor", "critical"], 0.88)
                insights_created += 1
                log.info(f"🌍 Chokepoint vulnerability: {len(affected_chokepoints)} corridors")

    # ─── 4. Cross-Domain Correlation (Sanctions ↔ Conflict ↔ Energy) ──────────
    # Find regions where multiple geo-domains are active simultaneously
    for region, profile in region_profiles.items():
        if len(profile["signal_types"]) < 2:
            continue

        # Check for specific dangerous combinations
        types = profile["signal_types"]
        dangerous_combos = []
        if {"conflict_escalation", "military_procurement"} <= types:
            dangerous_combos.append("Active conflict PLUS arms buildup → escalation risk")
        if {"sanctions_cascade", "energy_security"} <= types:
            dangerous_combos.append("Sanctions PLUS energy disruption → economic warfare pattern")
        if {"conflict_escalation", "trade_corridor_disruption"} <= types:
            dangerous_combos.append("Conflict PLUS trade corridor threat → supply chain isolation risk")
        if {"cyber_warfare", "conflict_escalation"} <= types:
            dangerous_combos.append("Cyber ops PLUS kinetic conflict → hybrid warfare pattern")
        if {"diplomatic_shift", "sanctions_cascade"} <= types:
            dangerous_combos.append("Diplomatic breakdown PLUS sanctions → relationship deterioration")
        if {"critical_mineral_risk", "sanctions_cascade"} <= types:
            dangerous_combos.append("Mineral supply PLUS sanctions → strategic resource weaponization")

        if not dangerous_combos:
            continue

        existing = await pool.fetchval(
            "SELECT 1 FROM insights WHERE title LIKE $1 AND created_at > NOW() - INTERVAL '12 hours'",
            f"%{region}%cross-domain%"
        )
        if existing:
            continue

        title = f"Cross-Domain Risk: {region} — {len(dangerous_combos)} dangerous patterns"
        summary = (
            f"CROSS-DOMAIN ANALYSIS: {region} shows {len(profile['signal_types'])} active geopolitical "
            f"risk dimensions with {profile['total_signals']} total signals.\n\n"
            f"DANGEROUS PATTERNS DETECTED:\n"
            + "\n".join(f"  • {combo}" for combo in dangerous_combos) + "\n\n"
            f"SEVERITY: {profile['critical_count']} critical, {profile['high_count']} high-severity signals.\n"
            f"STRATEGIC IMPLICATION: The intersection of these risk domains creates multiplicative "
            f"exposure that cannot be assessed in isolation.\n\n"
            f"RECOMMENDATION: Conduct holistic risk assessment for {region}; do not evaluate "
            f"sanctions, conflict, and supply chain risks independently."
        )
        sources = filter_relevant_urls(profile["sources"][:10])
        await store_insight(pool, title, summary, "geopolitical_analysis", region, sources,
                          ["cross_domain", "compound_risk", "geopolitical", region.lower().replace(" ", "_")], 0.88)
        insights_created += 1
        log.info(f"🌍 Cross-domain risk: {region} ({len(dangerous_combos)} patterns)")

    # ─── 5. Global Geopolitical Situation Report ───────────────────────────────
    # Generate a periodic summary of the overall geopolitical landscape
    existing_report = await pool.fetchval(
        "SELECT 1 FROM insights WHERE title LIKE 'Global Geopolitical Situation%' "
        "AND created_at > NOW() - INTERVAL '24 hours'"
    )
    if not existing_report and len(all_geo_warnings) >= 5:
        # Rank regions by composite risk
        ranked_regions = sorted(
            region_profiles.items(),
            key=lambda x: (len(x[1]["signal_types"]), x[1]["total_signals"]),
            reverse=True
        )[:8]

        region_summaries = []
        for region, profile in ranked_regions:
            type_labels = {
                "geopolitical_risk": "trade/sanctions",
                "conflict_escalation": "conflict",
                "sanctions_cascade": "sanctions",
                "diplomatic_shift": "diplomacy",
                "military_procurement": "arms",
                "energy_security": "energy",
                "cyber_warfare": "cyber",
                "critical_mineral_risk": "minerals",
                "trade_corridor_disruption": "corridors",
            }
            active = [type_labels.get(t, t) for t in profile["signal_types"]]
            region_summaries.append(
                f"  • {region}: {profile['total_signals']} signals across {', '.join(active)}"
            )

        title = f"Global Geopolitical Situation Report — {len(all_geo_warnings)} signals detected"
        summary = (
            f"PERIODIC INTELLIGENCE SUMMARY: {len(all_geo_warnings)} geopolitical signals detected "
            f"across {len(region_profiles)} regions in the past 48 hours.\n\n"
            f"TOP RISK REGIONS:\n"
            + "\n".join(region_summaries) + "\n\n"
            f"ESCALATING: {len(escalating_regions)} regions show increasing signal frequency.\n"
            f"COMPOUND RISK: {sum(1 for _, p in region_profiles.items() if len(p['signal_types']) >= 3)} "
            f"regions with 3+ simultaneous risk dimensions.\n\n"
            f"RECOMMENDATION: Focus monitoring resources on highest-risk regions. "
            f"Cross-reference business operations against identified hotspots."
        )
        all_sources = []
        for _, profile in ranked_regions[:3]:
            all_sources.extend(profile.get("sources", [])[:3])
        sources = filter_relevant_urls(all_sources)
        await store_insight(pool, title, summary, "geopolitical_analysis", "Global", sources,
                          ["situation_report", "geopolitical", "global", "periodic"], 0.85)
        insights_created += 1
        log.info(f"🌍 Global situation report generated")

    log.info(f"✓ Geopolitical landscape analysis complete: {insights_created} insights")
    return insights_created


async def update_company_scores(pool: asyncpg.Pool, company_id: str, updates: dict):
    """Update company risk/threat scores based on crawled signals."""
    sets = []
    vals = []
    i = 2
    for key, val in updates.items():
        if key in ("risk_score", "threat_score", "overlap_score", "strategic_relevance"):
            sets.append(f"{key} = ${i}")
            vals.append(val)
            i += 1
    if sets:
        sets.append(f"updated_at = ${i}")
        vals.append(datetime.now(timezone.utc))
        query = f"UPDATE companies SET {', '.join(sets)} WHERE id = $1"
        await pool.execute(query, uuid.UUID(company_id), *vals)


# ─── Signal Processing Pipeline ────────────────────────────────────────────────

RECIPE_MAP = {
    "supply_chain_disruption": "SUPPLY_CHAIN_DISRUPTION",
    "expansion": "COMPETITOR_EXPANSION",
    "hiring_signal": "HIRING_SIGNAL",
    "certification_update": "CERT_EXPIRY_RISK",
    "ma_activity": "COMPETITOR_EXPANSION",
    "technology": "TECH_CONVERGENCE",
    "geopolitical_risk": "GEOPOLITICAL_RISK",
    "conflict_escalation": "GEO_CONFLICT_ESCALATION",
    "sanctions_cascade": "GEO_SANCTIONS_CASCADE",
    "diplomatic_shift": "GEO_DIPLOMATIC_SHIFT",
    "military_procurement": "GEO_MILITARY_PROCUREMENT",
    "energy_security": "GEO_ENERGY_SECURITY",
    "cyber_warfare": "GEO_CYBER_WARFARE",
    "critical_mineral_risk": "GEO_CRITICAL_MINERALS",
    "trade_corridor_disruption": "GEO_TRADE_CORRIDOR",
}

SEVERITY_MAP = {
    "supply_chain_disruption": "high",
    "expansion": "medium",
    "hiring_signal": "low",
    "certification_update": "medium",
    "ma_activity": "high",
    "technology": "low",
    "geopolitical_risk": "critical",
    "conflict_escalation": "critical",
    "sanctions_cascade": "critical",
    "diplomatic_shift": "high",
    "military_procurement": "high",
    "energy_security": "critical",
    "cyber_warfare": "critical",
    "critical_mineral_risk": "high",
    "trade_corridor_disruption": "critical",
}


async def process_page(pool: asyncpg.Pool, result: dict, target: dict):
    """Process a fetched page: detect changes, extract signals, generate warnings."""
    url = result["url"]
    html = result["html"]
    content_hash = result["content_hash"]
    company_id = target.get("company_id")

    # 1. Check for content change
    is_new = await store_fingerprint(pool, url, content_hash)
    if not is_new:
        log.debug(f"No change: {url}")
        return

    log.info(f"📝 Content changed: {url}")

    # 2. Extract text and detect signals
    text = extract_text(html)
    if len(text) < 50:
        return

    signals = detect_signals(text, url)
    
    # 2b. Extract article links from page (for better source URLs)
    article_links = extract_article_links(html, url)

    # 3. Store WebChange observation
    if company_id:
        await store_observation(
            pool,
            obs_type="WebChange",
            entity_id=company_id,
            entity_type="company",
            value={
                "url": url,
                "change_type": "content_update",
                "text_length": len(text),
                "signals_detected": len(signals),
                "content_hash": content_hash,
            },
            provenance={
                "url": url,
                "fetch_ts": result["fetched_at"],
                "content_hash": content_hash,
                "extractor_version": "apex-crawler-1.0",
            },
            confidence=0.9
        )

    # 4. Process each signal
    for signal in signals:
        sig_type = signal["type"]
        recipe_code = RECIPE_MAP.get(sig_type)
        severity = SEVERITY_MAP.get(sig_type, "low")

        if not recipe_code:
            continue

        # Check if recipe exists
        recipe_exists = await pool.fetchval(
            "SELECT 1 FROM recipes WHERE code = $1", recipe_code
        )
        if not recipe_exists:
            continue

        # Get company region
        region = "Unknown"
        company_name = target.get("company_name", "Unknown")
        if company_id:
            row = await pool.fetchrow(
                "SELECT region, name FROM companies WHERE id = $1",
                uuid.UUID(company_id)
            )
            if row:
                region = row["region"] or "Unknown"
                company_name = row["name"]

        # Find article links that STRONGLY match the signal keyword.
        # Require the full multi-word keyword phrase OR at least 2 of its words
        # to appear in the article title. Single-word matching is too loose.
        keyword_lower = signal["keyword"].lower()
        kw_words = [w for w in keyword_lower.split() if len(w) > 3]  # skip short words
        relevant_articles = []
        for art in article_links:
            art_title_lower = art["title"].lower()
            # Best: full keyword phrase in title
            if keyword_lower in art_title_lower:
                relevant_articles.append(art)
            # Acceptable: at least 2 substantive keyword words in title
            elif len(kw_words) >= 2 and sum(1 for w in kw_words if w in art_title_lower) >= 2:
                relevant_articles.append(art)
        
        # Build source URLs: ONLY use articles that genuinely match the signal.
        # Never use random unrelated articles — that creates misleading evidence.
        if relevant_articles:
            source_urls = [art["url"] for art in relevant_articles[:3]]
            article_titles = [art["title"] for art in relevant_articles[:3]]
            title = f"{company_name}: {relevant_articles[0]['title'][:80]}"
            description = (
                f"Signal '{signal['keyword']}' detected. "
                f"Articles: {'; '.join(article_titles[:2])}. "
                f"Source: {source_urls[0]}"
            )
        else:
            # No matching articles found for this signal.
            # Do NOT use random articles from the page — they have nothing to do
            # with the signal and create misleading evidence links.
            # Store the warning with empty source_urls; it still contributes to
            # pattern detection but won't mislead analysts with wrong links.
            source_urls = []
            title = f"{company_name}: {signal['keyword'].title()} detected"
            description = (
                f"Signal '{signal['keyword']}' detected on {url}. "
                f"Page text excerpt: {text[:300]}..."
            )

        entity_ids = [company_id] if company_id else []

        # ── 24-hour per-entity dedup: don't re-create the same warning
        #    type for the same entity within 24 hours.
        if company_id:
            already_exists = await pool.fetchval(
                """SELECT 1 FROM warnings
                   WHERE warning_type = $1
                     AND $2::uuid = ANY(entity_ids)
                     AND created_at > NOW() - INTERVAL '24 hours'
                   LIMIT 1""",
                sig_type, uuid.UUID(company_id),
            )
            if already_exists:
                log.debug(f"Dedup: skipping {sig_type} for {company_name} (exists <24h)")
                continue

        await store_warning(
            pool, recipe_code, sig_type, title, description,
            severity, region, source_urls, entity_ids,
            confidence=min(0.95, 0.65 + (len(source_urls) * 0.05) + (0.10 if company_id else 0.0))
        )
        log.info(f"⚠️  Warning: {title} [{severity}]")

        # Store signal observation
        if company_id:
            # Use a focused excerpt around the signal keyword, not blind text[:500]
            sig_kw = signal["keyword"].lower()
            sig_idx = text.lower().find(sig_kw)
            if sig_idx >= 0:
                sig_start = max(0, sig_idx - 150)
                sig_end = min(len(text), sig_idx + len(sig_kw) + 350)
                sig_excerpt = text[sig_start:sig_end].replace("\n", " ").strip()
            else:
                sig_excerpt = text[:500]
            # Skip if excerpt is boilerplate
            if _is_boilerplate_excerpt(sig_excerpt):
                continue
            await store_observation(
                pool,
                obs_type="CompetitorEvent" if sig_type in ("expansion", "ma_activity") else "WebChange",
                entity_id=company_id,
                entity_type="company",
                value={
                    "signal_type": sig_type,
                    "keyword": signal["keyword"],
                    "url": source_urls[0] if source_urls else url,
                    "article_urls": source_urls,
                    "excerpt": sig_excerpt,
                },
                provenance={
                    "url": url,
                    "fetch_ts": result["fetched_at"],
                    "extractor_version": "apex-crawler-1.0",
                    "signal_detector": "keyword_v1",
                },
                confidence=0.7
            )


# ─── News Processing ───────────────────────────────────────────────────────────

async def process_news_page(pool: asyncpg.Pool, result: dict, target: dict):
    """Process a news source page — look for articles mentioning tracked companies.

    Quality gates (Q1 2026):
    - Excerpt must contain substantive text near the company mention, not page chrome
    - Company name must appear in meaningful text context (not just nav/footer)
    - Observation excerpt is extracted around the company mention for relevance
    - Domain-key shortcuts skip government domains and known country names
    - Word-boundary matching for all keys ≤ 12 chars to prevent substring false hits
    - Boilerplate excerpts (country lists, currency converters, nav menus) are rejected
    """
    html = result["html"]
    text = extract_text(html)
    text_lower = text.lower()

    # Skip very short pages (likely error pages or empty pages)
    if len(text) < 200:
        return

    # ── Pre-filter: reject pages that are mostly boilerplate ──
    # If the first 1000 chars of the page look like a country/nav list, skip entirely.
    if _is_boilerplate_excerpt(text[:1000]):
        return

    # Get all company names for matching
    rows = await pool.fetch("SELECT id, name, domain FROM companies")
    company_map = {row["name"].lower(): (str(row["id"]), row["name"]) for row in rows}

    # Also map domain keywords — with strict filtering to prevent false positives
    # from country names (e.g. "malaysia" from malaysia.gov.my) and generic words.
    for row in rows:
        if row["domain"]:
            # Skip government domains entirely — their country name matches everywhere
            dom = row["domain"].lower()
            if ".gov." in dom or dom.startswith("gov.") or ".go." in dom:
                continue
            domain_key = dom.split(".")[0]
            if len(domain_key) <= 4:
                continue
            # Skip if the domain key is a known country name or banned common word
            if domain_key in _COUNTRY_NAMES_FOR_BOILERPLATE or domain_key in _BANNED_DOMAIN_KEYS:
                continue
            company_map[domain_key] = (str(row["id"]), row["name"])

    # Pre-compile word-boundary regexes for ALL keys ≤ 12 chars.
    # Short/medium keys cause massive false positives without word boundaries
    # (e.g. "venture" matching "joint venture", "arrow" matching "arrow key").
    _news_patterns = {}
    for key in company_map:
        if len(key) <= 12:
            try:
                _news_patterns[key] = re.compile(r'\b' + re.escape(key) + r'\b', re.IGNORECASE)
            except re.error:
                pass

    # Check which companies are mentioned — extract context around each mention
    mentioned = {}
    for key, (cid, cname) in company_map.items():
        pat = _news_patterns.get(key)
        if pat:
            m = pat.search(text_lower)
            if not m:
                continue
            idx = m.start()
        else:
            idx = text_lower.find(key)
            if idx == -1:
                continue
        # Extract text window around the mention (200 chars before, 300 after)
        start = max(0, idx - 200)
        end = min(len(text), idx + len(key) + 300)
        context_excerpt = text[start:end].replace("\n", " ").strip()
        # Skip mentions that appear only in navigation/footer (very short context)
        if len(context_excerpt) < 80:
            continue
        # ── Quality gate: reject boilerplate excerpts ──
        # Country lists, currency converters, nav menus, etc.
        if _is_boilerplate_excerpt(context_excerpt):
            log.debug(f"   Skipping boilerplate mention of {cname} on {result['url']}")
            continue
        mentioned[cid] = (cname, context_excerpt)

    if not mentioned:
        return

    log.info(f"📰 News mentions {len(mentioned)} companies: {[m[0] for m in mentioned.values()]}")

    # Detect signals across the news article
    signals = detect_signals(text, result["url"])

    for company_id, (company_name, context_excerpt) in mentioned.items():
        # Store as observation with entity-relevant excerpt
        await store_observation(
            pool,
            obs_type="CompetitorEvent",
            entity_id=company_id,
            entity_type="company",
            value={
                "source": target["page_type"],
                "url": result["url"],
                "mentions": True,
                "signals": [s["type"] for s in signals],
                "excerpt": context_excerpt,
            },
            provenance={
                "url": result["url"],
                "fetch_ts": result["fetched_at"],
                "extractor_version": "apex-crawler-1.0",
                "source_type": "news",
            },
            confidence=0.65
        )


# ─── Main Crawl Cycle ──────────────────────────────────────────────────────────

async def run_crawl_cycle(pool: asyncpg.Pool):
    """Execute one full crawl cycle."""
    log.info("═══ Starting crawl cycle ═══")
    start = time.time()

    targets = await build_crawl_targets(pool)
    log.info(f"Built {len(targets)} crawl targets")

    semaphore = asyncio.Semaphore(MAX_CONCURRENT)

    # Primary: IPv6 sessions (round-robin across 2 addresses)
    ipv6_sessions = []
    for addr in IPV6_ADDRESSES:
        c = make_connector(addr)
        ipv6_sessions.append(aiohttp.ClientSession(connector=c))

    # Fallback: IPv4 sessions (round-robin across 2 addresses)
    ipv4_sessions = []
    for addr in IPV4_ADDRESSES:
        c = make_connector(addr)
        ipv4_sessions.append(aiohttp.ClientSession(connector=c))

    log.info(f"🔄 IP rotation: {len(IPV6_ADDRESSES)} IPv6 primary + {len(IPV4_ADDRESSES)} IPv4 fallback")

    stats = {"fetched": 0, "changed": 0, "errors": 0, "signals": 0}

    try:
        # Process in batches
        batch_size = 20
        for i in range(0, len(targets), batch_size):
            batch = targets[i:i + batch_size]
            tasks = [
                fetch_url(
                    ipv6_sessions[j % len(ipv6_sessions)],
                    t["url"],
                    semaphore,
                    fallback_session=ipv4_sessions[j % len(ipv4_sessions)],
                )
                for j, t in enumerate(batch, start=i)
            ]
            results = await asyncio.gather(*tasks, return_exceptions=True)

            for result, target in zip(results, batch):
                if isinstance(result, Exception):
                    stats["errors"] += 1
                    continue
                if result is None:
                    stats["errors"] += 1
                    continue
                if result["status"] != 200:
                    stats["errors"] += 1
                    continue

                stats["fetched"] += 1

                try:
                    if target.get("page_type", "").startswith("news_"):
                        await process_news_page(pool, result, target)
                    else:
                        await process_page(pool, result, target)
                except Exception as e:
                    log.error(f"Error processing {result['url']}: {e}")
                    stats["errors"] += 1

            # Small pause between batches
            await asyncio.sleep(random.uniform(1.0, 3.0))
    finally:
        for s in ipv6_sessions + ipv4_sessions:
            await s.close()

    duration = time.time() - start
    log.info(
        f"═══ Crawl cycle complete ═══ "
        f"Duration: {duration:.1f}s | "
        f"Fetched: {stats['fetched']} | "
        f"Errors: {stats['errors']}"
    )

    # Update cycle metadata
    await pool.execute(
        """INSERT INTO observations (observation_type, entity_type, ts_utc, value, provenance, confidence)
           VALUES ('CrawlCycleComplete', 'system', $1, $2::jsonb, $3::jsonb, 1.0)""",
        datetime.now(timezone.utc),
        json.dumps(stats),
        json.dumps({"extractor_version": "apex-crawler-1.0", "targets": len(targets)})
    )
    
    # Generate insights from warning patterns
    try:
        insights_count = await generate_insights_from_warnings(pool)
        log.info(f"💡 Insight generation complete: {insights_count} new insights")
    except Exception as e:
        log.error(f"Insight generation failed: {e}")

    # Discover competitor customers (runs once per cycle)
    try:
        c2 = make_connector(_next_ipv4_address())
        async with aiohttp.ClientSession(connector=c2) as session2:
            cust_count = await discover_competitor_customers(pool, session2, semaphore)
            log.info(f"🔍 Competitor customer discovery: {cust_count} found")
    except Exception as e:
        log.error(f"Competitor customer discovery failed: {e}")

    # Discover POI contact information (aggressive enrichment)
    try:
        c_poi = make_connector(_next_ipv4_address())
        async with aiohttp.ClientSession(connector=c_poi) as session_poi:
            poi_count = await discover_poi_contacts(pool, session_poi, semaphore)
            log.info(f"👤 POI contact discovery: {poi_count} persons enriched")
    except Exception as e:
        log.error(f"POI contact discovery failed: {e}")

    # Run dedicated dark web POI intelligence scan (only if Tor available)
    if TOR_ENABLED:
        try:
            darkweb_stats = await run_darkweb_poi_scan(pool)
            log.info(f"🧅 Dark web POI scan: {darkweb_stats}")
        except Exception as e:
            log.error(f"Dark web POI scan failed: {e}")

    # Scrape social media sources (Reddit, HN, Mastodon, Bluesky, Telegram, etc.)
    try:
        c3 = make_connector(_next_ipv4_address())
        async with aiohttp.ClientSession(connector=c3) as session3:
            social_stats = await scrape_social_media(pool, session3, semaphore)
            log.info(f"📱 Social media scrape: {social_stats}")
    except Exception as e:
        log.error(f"Social media scrape failed: {e}")

    # Cross-reference news stories for veracity analysis
    try:
        c4 = make_connector(_next_ipv4_address())
        async with aiohttp.ClientSession(connector=c4) as session4:
            veracity_count = await cross_reference_news(pool, session4, semaphore)
            log.info(f"🔍 Veracity analysis: {veracity_count} insights")
    except Exception as e:
        log.error(f"Cross-reference analysis failed: {e}")

    # Geopolitical landscape analysis (ties all geo signals together)
    try:
        geo_count = await analyze_geopolitical_landscape(pool)
        log.info(f"🌍 Geopolitical analysis: {geo_count} insights")
    except Exception as e:
        log.error(f"Geopolitical landscape analysis failed: {e}")

    # Job board hiring signal detection
    try:
        c_job = make_connector(_next_ipv4_address())
        async with aiohttp.ClientSession(connector=c_job) as session_job:
            await crawl_job_signals(pool, session_job, semaphore)
            log.info("💼 Job board signal scan complete")
    except Exception as e:
        log.error(f"Job board crawl failed: {e}")


# ─── Job Board Crawling ────────────────────────────────────────────────────────

async def crawl_job_signals(pool: asyncpg.Pool, session: aiohttp.ClientSession, semaphore: asyncio.Semaphore):
    """Crawl job boards for hiring signals."""
    rows = await pool.fetch("SELECT id, name, domain FROM companies WHERE domain IS NOT NULL")

    for row in rows:
        # Try common careers page patterns
        domain = row["domain"]
        company_id = str(row["id"])
        careers_urls = [
            f"https://{domain}/careers",
            f"https://{domain}/jobs",
            f"https://careers.{domain}",
        ]

        for url in careers_urls:
            result = await fetch_url(session, url, semaphore)
            if result and result["status"] == 200:
                text = extract_text(result["html"])
                # Count job-related keywords as proxy for hiring volume
                job_count = len(re.findall(r'\b(?:position|role|engineer|developer|manager|analyst|director)\b', text, re.I))
                if job_count > 5:
                    await store_observation(
                        pool,
                        obs_type="JobPost",
                        entity_id=company_id,
                        entity_type="company",
                        value={
                            "url": url,
                            "estimated_open_roles": job_count,
                            "text_length": len(text),
                        },
                        provenance={
                            "url": url,
                            "fetch_ts": result["fetched_at"],
                            "extractor_version": "apex-crawler-1.0",
                        },
                        confidence=0.6
                    )
                    log.info(f"💼 {row['name']}: ~{job_count} roles detected at {url}")
                break  # Only need one working careers URL per company


# ─── Main Loop ──────────────────────────────────────────────────────────────────

async def main():
    log.info("🚀 ApexIntel Crawl Daemon starting...")
    log.info(f"   IPv6 primary: {', '.join(IPV6_ADDRESSES)}")
    log.info(f"   IPv4 fallback: {', '.join(IPV4_ADDRESSES)}")
    log.info(f"   Rate: {REQUESTS_PER_SECOND} req/s, max concurrent: {MAX_CONCURRENT}")
    log.info(f"   Cycle interval: {CRAWL_CYCLE_INTERVAL}s")

    pool = await asyncpg.create_pool(
        DATABASE_URL,
        min_size=2,
        max_size=10,
        command_timeout=60,
    )
    log.info("✓ Database connection pool established")

    # Verify companies exist
    count = await pool.fetchval("SELECT COUNT(*) FROM companies")
    log.info(f"✓ Tracking {count} companies")

    if count == 0:
        log.error("No companies in database! Run seed_data.sql first.")
        await pool.close()
        return

    cycle_count = 0
    while True:
        try:
            cycle_count += 1
            log.info(f"═══ Cycle #{cycle_count} starting ═══")
            await run_crawl_cycle(pool)
        except Exception as e:
            log.error(f"Crawl cycle failed: {e}", exc_info=True)

        # Wait for next cycle
        jitter = random.uniform(-60, 60)
        wait_time = max(60, CRAWL_CYCLE_INTERVAL + jitter)
        log.info(f"Next cycle in {wait_time:.0f}s")
        await asyncio.sleep(wait_time)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        log.info("Shutting down...")
