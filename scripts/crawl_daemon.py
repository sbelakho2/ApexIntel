#!/usr/bin/env python3
"""
ApexIntel Crawl Daemon — Continuous intelligence gathering for EMS/Electronics supply chain.

Uses IPRoyal residential proxy with session-based rotation (identical to CRM-v2 infrastructure).
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

# ─── Configuration ──────────────────────────────────────────────────────────────

DATABASE_URL = os.getenv("DATABASE_URL", "postgresql://apexintel:ApexIntel2026Secure@127.0.0.1:5432/apexintel")

# IPRoyal proxy credentials (same as CRM-v2)
PROXY_USER = os.getenv("PROXY_USER", "pHPV5jEpXqHtlpzc")
PROXY_PASS = os.getenv("PROXY_PASS", "NdL4xVheMTy36UyA")
PROXY_HOST = os.getenv("PROXY_HOST", "geo.iproyal.com")
PROXY_PORT = int(os.getenv("PROXY_PORT", "12321"))

# EU exit countries for proxy rotation (same list as CRM-v2)
EXIT_COUNTRIES = ["ee", "de", "nl", "fr", "pl", "cz", "fi", "se", "at", "be", "dk", "no", "es", "it", "pt", "ro"]

# Crawl settings
REQUESTS_PER_SECOND = float(os.getenv("REQUESTS_PER_SECOND", "2.0"))
CRAWL_CYCLE_INTERVAL = int(os.getenv("CRAWL_CYCLE_INTERVAL", "3600"))  # 1 hour
MAX_CONCURRENT = int(os.getenv("MAX_CONCURRENT", "5"))
REQUEST_TIMEOUT = int(os.getenv("REQUEST_TIMEOUT", "30"))

# ─── Logging ────────────────────────────────────────────────────────────────────

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)]
)
log = logging.getLogger("apex-crawler")

# ─── User-Agent Pool (matches crates/crawl/src/headers.rs) ─────────────────────

USER_AGENTS = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4_1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36 Edg/123.0.0.0",
    "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:125.0) Gecko/20100101 Firefox/125.0",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (iPad; CPU OS 17_4_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 OPR/108.0.0.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.4; rv:125.0) Gecko/20100101 Firefox/125.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:125.0) Gecko/20100101 Firefox/125.0",
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


# ─── Proxy Rotation (identical to CRM-v2 getRotatingProxyUrl) ──────────────────

def get_rotating_proxy_url() -> str:
    """Generate a unique proxy URL with random EU exit country + unique session ID.
    Format: http://USER:PASS_country-XX_session-HEXID@HOST:PORT
    This ensures each request gets a different IP address.
    """
    country = random.choice(EXIT_COUNTRIES)
    session_id = "".join(random.choices(string.hexdigits[:16], k=8))
    return f"http://{PROXY_USER}:{PROXY_PASS}_country-{country}_session-{session_id}@{PROXY_HOST}:{PROXY_PORT}"


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


# ─── Fetch with Proxy ──────────────────────────────────────────────────────────

async def fetch_url(session: aiohttp.ClientSession, url: str, semaphore: asyncio.Semaphore) -> Optional[dict]:
    """Fetch a URL through the rotating proxy. Returns dict with url, status, html, headers."""
    async with semaphore:
        proxy_url = get_rotating_proxy_url()
        headers = get_random_headers()
        # Rate limiting
        await asyncio.sleep(1.0 / REQUESTS_PER_SECOND + random.uniform(0.1, 0.5))
        try:
            async with session.get(
                url,
                proxy=proxy_url,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=REQUEST_TIMEOUT),
                ssl=False,
                allow_redirects=True,
                max_redirects=5,
            ) as resp:
                html = await resp.text(errors="replace")
                content_hash = hashlib.sha256(html.encode()).hexdigest()
                log.info(f"✓ {resp.status} {url} [{len(html)} bytes] via {proxy_url.split('@')[1]}")
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
            log.warning(f"✗ Error fetching {url}: {e}")
            return None
        except Exception as e:
            log.error(f"✗ Unexpected error fetching {url}: {e}")
            return None


# ─── Page Analysis ──────────────────────────────────────────────────────────────

def extract_text(html: str) -> str:
    """Extract clean text from HTML."""
    soup = BeautifulSoup(html, "html.parser")
    for tag in soup(["script", "style", "nav", "footer", "header", "aside"]):
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


def detect_signals(text: str, url: str) -> list[dict]:
    """Detect intelligence signals from page text."""
    signals = []
    text_lower = text.lower()

    # Supply chain disruption signals
    disruption_kw = [
        "supply chain disruption", "shortage", "allocation", "force majeure",
        "factory closure", "production halt", "shipping delay", "lead time",
        "capacity constraint", "component shortage"
    ]
    for kw in disruption_kw:
        if kw in text_lower:
            signals.append({"type": "supply_chain_disruption", "keyword": kw, "url": url})
            break

    # Expansion signals
    expansion_kw = [
        "new facility", "plant expansion", "investment in", "new manufacturing",
        "grand opening", "groundbreaking", "new campus", "expanding operations",
        "capacity expansion", "new production line"
    ]
    for kw in expansion_kw:
        if kw in text_lower:
            signals.append({"type": "expansion", "keyword": kw, "url": url})
            break

    # Hiring signals
    hiring_kw = [
        "hiring", "job opening", "career", "we are looking for",
        "join our team", "open position", "talent acquisition"
    ]
    for kw in hiring_kw:
        if kw in text_lower:
            signals.append({"type": "hiring_signal", "keyword": kw, "url": url})
            break

    # Certification/compliance signals
    cert_kw = [
        "iso 9001", "iso 14001", "iatf 16949", "as9100", "iso 13485",
        "iso 27001", "certification", "accreditation", "compliance",
        "audit", "recertification"
    ]
    for kw in cert_kw:
        if kw in text_lower:
            signals.append({"type": "certification_update", "keyword": kw, "url": url})
            break

    # M&A signals
    ma_kw = [
        "acquisition", "merger", "acquired", "takeover", "joint venture",
        "strategic partnership", "equity stake"
    ]
    for kw in ma_kw:
        if kw in text_lower:
            signals.append({"type": "ma_activity", "keyword": kw, "url": url})
            break

    # Technology signals
    tech_kw = [
        "patent", "innovation", "breakthrough", "next-generation",
        "r&d", "research and development", "new technology", "product launch"
    ]
    for kw in tech_kw:
        if kw in text_lower:
            signals.append({"type": "technology", "keyword": kw, "url": url})
            break

    # Geopolitical risk signals
    geo_kw = [
        "sanctions", "tariff", "trade restriction", "export control",
        "geopolitical", "embargo", "chips act", "critical raw materials"
    ]
    for kw in geo_kw:
        if kw in text_lower:
            signals.append({"type": "geopolitical_risk", "keyword": kw, "url": url})
            break

    return signals


# ─── News & RSS Feeds ───────────────────────────────────────────────────────────

# Industry-specific news and data sources
NEWS_SOURCES = [
    # Electronics industry news
    {"url": "https://www.eetimes.com/", "type": "news", "topic": "electronics"},
    {"url": "https://www.edn.com/", "type": "news", "topic": "electronics"},
    {"url": "https://www.electronicdesign.com/", "type": "news", "topic": "electronics"},
    {"url": "https://www.electronicsweekly.com/", "type": "news", "topic": "electronics"},
    {"url": "https://www.fierceelectronics.com/", "type": "news", "topic": "electronics"},

    # Supply chain news
    {"url": "https://www.supplychaindive.com/", "type": "news", "topic": "supply_chain"},
    {"url": "https://www.scmr.com/", "type": "news", "topic": "supply_chain"},

    # Semiconductor news
    {"url": "https://www.semiconductorengineering.com/", "type": "news", "topic": "semiconductors"},
    {"url": "https://www.anandtech.com/", "type": "news", "topic": "semiconductors"},

    # Defense electronics
    {"url": "https://www.defensenews.com/", "type": "news", "topic": "defense"},
    {"url": "https://www.janes.com/", "type": "news", "topic": "defense"},

    # Trade publications
    {"url": "https://www.assemblymag.com/", "type": "news", "topic": "manufacturing"},
    {"url": "https://www.smt007.com/", "type": "news", "topic": "manufacturing"},
    {"url": "https://www.circuitsassembly.com/", "type": "news", "topic": "PCB"},
]


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
    """Insert an observation into the database."""
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
}

SEVERITY_MAP = {
    "supply_chain_disruption": "high",
    "expansion": "medium",
    "hiring_signal": "low",
    "certification_update": "medium",
    "ma_activity": "high",
    "technology": "low",
    "geopolitical_risk": "critical",
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

        # Generate warning title
        title = f"{company_name}: {signal['keyword'].title()} detected"
        description = (
            f"Signal '{signal['keyword']}' detected on {url}. "
            f"Page text excerpt: {text[:300]}..."
        )

        entity_ids = [company_id] if company_id else []

        await store_warning(
            pool, recipe_code, sig_type, title, description,
            severity, region, [url], entity_ids,
            confidence=0.7 + random.uniform(0, 0.2)
        )
        log.info(f"⚠️  Warning: {title} [{severity}]")

        # Store signal observation
        if company_id:
            await store_observation(
                pool,
                obs_type="CompetitorEvent" if sig_type in ("expansion", "ma_activity") else "WebChange",
                entity_id=company_id,
                entity_type="company",
                value={
                    "signal_type": sig_type,
                    "keyword": signal["keyword"],
                    "url": url,
                    "excerpt": text[:500],
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
    """Process a news source page — look for articles mentioning tracked companies."""
    html = result["html"]
    text = extract_text(html)
    text_lower = text.lower()

    # Get all company names for matching
    rows = await pool.fetch("SELECT id, name, domain FROM companies")
    company_map = {row["name"].lower(): (str(row["id"]), row["name"]) for row in rows}

    # Also map domain keywords
    for row in rows:
        domain_key = row["domain"].split(".")[0].lower()
        if len(domain_key) > 3:
            company_map[domain_key] = (str(row["id"]), row["name"])

    # Check which companies are mentioned
    mentioned = set()
    for key, (cid, cname) in company_map.items():
        if key in text_lower:
            mentioned.add((cid, cname))

    if not mentioned:
        return

    log.info(f"📰 News mentions {len(mentioned)} companies: {[m[1] for m in mentioned]}")

    # Detect signals across the news article
    signals = detect_signals(text, result["url"])

    for company_id, company_name in mentioned:
        # Store as observation
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
                "excerpt": text[:500],
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
    connector = aiohttp.TCPConnector(limit=MAX_CONCURRENT * 2, ttl_dns_cache=300, force_close=True)

    stats = {"fetched": 0, "changed": 0, "errors": 0, "signals": 0}

    async with aiohttp.ClientSession(connector=connector) as session:
        # Process in batches
        batch_size = 20
        for i in range(0, len(targets), batch_size):
            batch = targets[i:i + batch_size]
            tasks = [fetch_url(session, t["url"], semaphore) for t in batch]
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
    log.info(f"   Proxy: {PROXY_HOST}:{PROXY_PORT} with {len(EXIT_COUNTRIES)} EU exit countries")
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
