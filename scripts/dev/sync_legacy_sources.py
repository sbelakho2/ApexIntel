#!/usr/bin/env python3
"""Sync legacy crawl-daemon source lists into config/sources_supplement.yaml.

The legacy daemon (experiments/legacy-python-crawl-daemon/crawl_daemon.py) holds
hundreds of OSINT sources that were never carried into the Rust source registry
(crates/crawl/src/sources_registry.rs).  This script converts NEWS_SOURCES,
SOCIAL_MEDIA_SOURCES and ONION_SOURCES into the registry's YAML schema, dedups
them against the built-in registry (by host), and writes the supplement file
that crates/crawl merges at startup.  Re-run after any legacy-list edit.

Usage:
    python3 scripts/dev/sync_legacy_sources.py
"""

from __future__ import annotations

import ast
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DAEMON = ROOT / "experiments/legacy-python-crawl-daemon/crawl_daemon.py"
REGISTRY_RS = ROOT / "crates/crawl/src/sources_registry.rs"
OUT = ROOT / "config/sources_supplement.yaml"

REGION_MAP = {
    "government_tn": "Africa", "government_ma": "Africa", "government_dz": "Africa",
    "government_ae": "MiddleEast", "government_sa": "MiddleEast", "government_qa": "MiddleEast",
    "government_jp": "AsiaPacific", "government_kr": "AsiaPacific", "government_tw": "AsiaPacific",
    "government_eu": "Europe", "government_us": "NorthAmerica", "government_ca": "NorthAmerica",
    "mena": "MiddleEast", "middle_east": "MiddleEast", "china": "China", "india": "India",
    "south_asia": "India", "central_asia": "AsiaPacific", "europe": "Europe",
    "africa": "Africa", "russia": "Russia", "eastern_europe": "EasternEurope",
    "conflict": "EasternEurope", "ukraine": "EasternEurope",
}

CATEGORY_MAP = {
    "news": "News", "media": "News", "pr": "News", "conference": "News",
    "association": "News", "awards": "News", "succession": "News", "jobs": "News",
    "diplomatic": "News", "press_release": "News",
    "gov": "GovernmentRegistry", "government": "GovernmentRegistry",
    "regulatory": "LegalRegulatory", "legal": "LegalRegulatory",
    "financial": "Finance", "finance": "Finance",
    "patent": "Patents", "patents": "Patents",
    "academic": "AcademicResearch", "research": "AcademicResearch",
    "think_tank": "GeopoliticsThinkTank", "conflict_data": "GeopoliticsThinkTank",
    "geopolitics": "GeopoliticsThinkTank", "osint": "GeopoliticsThinkTank",
    "sanctions": "Sanctions", "trade": "Trade", "trade_policy": "Trade",
    "arms_trade": "Defence", "defense": "Defence", "defense_tech": "Defence",
    "military": "Defence", "nuclear": "Defence",
    "maritime": "SupplyChain", "supply_chain": "SupplyChain", "logistics": "SupplyChain",
    "manufacturing": "SupplyChain", "PCB": "SupplyChain", "semiconductors": "Technology",
    "electronics": "Technology", "technology": "Technology",
    "energy": "EnergyResources", "energy_storage": "EnergyResources",
    "battery": "EnergyResources", "minerals": "EnergyResources",
    "cyber": "Cybersecurity", "cybersecurity": "Cybersecurity", "cyber_conflict": "Cybersecurity",
    "threat_intel": "Cybersecurity", "netsec": "Cybersecurity",
    "forum": "Forum", "social": "SocialMedia",
    "reddit": "SocialMedia", "telegram": "SocialMedia", "bluesky": "SocialMedia",
    "mastodon": "SocialMedia", "hackernews": "SocialMedia", "youtube": "SocialMedia",
}

TIER_MAP = {"T1": 2, "T2": 3, "T3": 4, "T4": 5, "T5": 5}


def extract_assignments(path: Path) -> dict[str, ast.AST]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    found: dict[str, ast.AST] = {}
    for node in tree.body:
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id in {
                    "NEWS_SOURCES", "SOCIAL_MEDIA_SOURCES", "ONION_SOURCES"
                }:
                    found[target.id] = node.value
    return found


def literal(node: ast.AST) -> object:
    return ast.literal_eval(node)


def slugify(text: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return slug[:64] or "source"


def host_of(url: str) -> str:
    m = re.match(r"https?://([^/]+)", url)
    return (m.group(1) if m else url).lower()


def builtin_hosts(registry_rs: Path) -> set[str]:
    return {normalize_url(u).split("/")[0] for u in builtin_urls(registry_rs)}


def normalize_url(url: str) -> str:
    return re.sub(r"^https?://", "", url).rstrip("/").lower()


def builtin_urls(registry_rs: Path) -> set[str]:
    text = registry_rs.read_text(encoding="utf-8")
    return {
        normalize_url(u)
        for u in re.findall(r'https?://[^\s"\'\)]+', text)
    }


def short_hash(value: str) -> str:
    return hashlib.sha1(value.encode("utf-8")).hexdigest()[:8]


def make_entry(slug: str, name: str, url: str, region: str, category: str,
               tier: int, needs_proxy: bool = False, rss_url: str | None = None,
               notes: str = "", interval: int = 60) -> dict:
    entry = {
        "slug": slug,
        "name": name,
        "url": url,
        "search_param": None,
        "region": region,
        "category": category,
        "tier": tier,
        "needs_proxy": needs_proxy,
        "rss_url": rss_url,
        "enabled": True,
        "min_interval_minutes": interval,
        "notes": notes or None,
    }
    return entry


def name_from_host(url: str, fallback: str = "") -> str:
    h = host_of(url).replace("www.", "").split(".")[0]
    return h.replace("-", " ").title() or fallback


def convert_news(entries: list[dict], used: set[str]) -> list[dict]:
    out, warns = [], []
    for i, e in enumerate(entries):
        url = e["url"]
        typ = e.get("type", "news")
        topic = e.get("topic", "general")
        host = host_of(url).replace("www.", "")
        category = CATEGORY_MAP.get(typ, CATEGORY_MAP.get(topic, "News"))
        region = REGION_MAP.get(topic, "Global")
        tier = TIER_MAP.get(e.get("tier", ""), 3)
        slug = f"legacy_news_{slugify(host)}"
        if slug in used or host in EXCLUDED_HOSTS:
            slug = f"{slug}_{short_hash(url)}"
        if slug in used:
            slug = f"{slug}_{i}"
        used.add(slug)
        out.append(make_entry(
            slug, name_from_host(url, topic), url, region, category, tier,
            notes=f"legacy daemon: type={typ} topic={topic} poi_signal={e.get('poi_signal', False)}",
            interval=120,
        ))
    return out


EXCLUDED_HOSTS: set[str] = set()
EXCLUDED_URLS: set[str] = set()


def convert_social(entries: list[dict], used: set[str]) -> list[dict]:
    out = []
    counters: dict[str, int] = {}
    for i, e in enumerate(entries):
        url = e["url"]
        platform = e.get("platform", "social")
        topic = e.get("topic", "general")
        tier = TIER_MAP.get(e.get("tier", ""), 3)
        category = CATEGORY_MAP.get(platform, "SocialMedia")
        region = REGION_MAP.get(topic, "Global")
        if platform == "telegram":
            m = re.search(r"t\.me/s/([A-Za-z0-9_]+)", url)
            handle = m.group(1) if m else f"ch{i}"
            slug = f"telegram_{handle.lower()}"
            name = f"Telegram {handle}"
            notes = f"telegram public channel; topic={topic}"
            interval = 30
            rss = None
        elif platform == "reddit":
            m = re.search(r"reddit\.com/r/([A-Za-z0-9_]+)", url)
            sub = m.group(1) if m else f"sub{i}"
            slug = f"reddit_{sub.lower()}"
            name = f"Reddit r/{sub}"
            notes = f"reddit subreddit new.json; topic={topic}"
            interval = 30
            rss = None
        elif platform == "youtube":
            m = re.search(r"channel_id=([A-Za-z0-9_\-]+)", url)
            cid = m.group(1) if m else f"ch{i}"
            slug = f"youtube_{slugify(cid)}"
            name = f"YouTube feed {topic}"
            notes = f"youtube channel RSS; topic={topic}"
            interval = 60
            rss = url
        elif platform == "mastodon":
            m = re.search(r"https?://([^/]+)/api/v1/timelines/tag/([A-Za-z0-9_\-]+)", url)
            inst = m.group(1) if m else f"inst{i}"
            tag = m.group(2) if m else f"tag{i}"
            slug = f"mastodon_{slugify(inst)}_{tag.lower()}"
            name = f"Mastodon {inst} #{tag}"
            notes = f"mastodon tag timeline; topic={topic}"
            interval = 30
            rss = None
        elif platform == "hackernews":
            kind = "newstories" if "newstories" in url else "topstories"
            slug = f"hackernews_{kind}"
            name = f"Hacker News {kind}"
            notes = f"hackernews firebase {kind}"
            interval = 15
            rss = None
        elif platform == "bluesky":
            counters["bluesky"] = counters.get("bluesky", 0) + 1
            n = counters["bluesky"]
            slug = f"bluesky_{topic}_{n}"
            name = f"Bluesky search: {topic} ({n})"
            notes = f"bluesky searchPosts; topic={topic}"
            interval = 30
            rss = None
        else:
            host = host_of(url).replace("www.", "")
            slug = f"legacy_{platform}_{slugify(host)}"
            name = f"{platform.title()} {host}"
            notes = f"platform={platform} topic={topic}"
            interval = 60
            rss = None
        if slug in used:
            slug = f"{slug}_{short_hash(url)}"
        used.add(slug)
        out.append(make_entry(slug, name, url, region, category, tier,
                              needs_proxy=False, rss_url=rss, notes=notes,
                              interval=interval))
    return out


def convert_onion(onion: dict[str, str], used: set[str]) -> list[dict]:
    # Onion endpoints verified live over Tor (SOCKS5h, 200, 2026-10-05) are
    # appended to the legacy set so the registry ships dark-web sources with
    # current proof, not just historical addresses.
    onion = dict(onion)
    onion.setdefault("duckduckgo_onion",
                     "https://duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion")
    onion.setdefault("torproject_onion",
                     "http://2gzyxa5ihm7nsggfxnu52rck2vv4rvmdlkiu3zzui5du4xyclen53wid.onion")
    onion.setdefault("ahmia_onion",
                     "http://juhanurmihxlp77nkq76byazcldy2hlmovfu2epvl5ankdibsot4csyd.onion")
    out = []
    meta = {
        "pwndb": ("PwnDB breach index", "Cybersecurity",
                  "breach credential index (Tor); queried via SOCKS5h"),
        "exposed_vc": ("Exposed.vc onion mirror", "Cybersecurity",
                       "breach listing mirror (Tor)"),
        "dread_osint": ("Dread forum OSINT boards", "Forum",
                        "dark-web forum OSINT board (Tor)"),
        "ransomware_index": ("Ransomware leak index", "Cybersecurity",
                             "ransomware leak-site aggregate index (Tor)"),
        "duckduckgo_onion": ("DuckDuckGo onion", "News",
                             "private search engine over Tor (verified live 2026-10)"),
        "torproject_onion": ("Tor Project onion", "Cybersecurity",
                             "Tor Project site over Tor (verified live 2026-10)"),
        "ahmia_onion": ("Ahmia search", "Cybersecurity",
                        "dark-web search index over Tor (verified live 2026-10)"),
    }
    for key, url in onion.items():
        name, category, notes = meta.get(key, (key.replace("_", " ").title(), "Cybersecurity", "onion source"))
        slug = f"darkweb_{key}"
        used.add(slug)
        out.append(make_entry(slug, name, url, "Global", category, 4,
                              needs_proxy=True, notes=notes, interval=180))
    return out


def convert_dark_clearnet(used: set[str]) -> list[dict]:
    """Active dark-web-adjacent clearnet monitors from dark_web.rs::default_forums.

    Criminal forums (BreachForums, Exploit.in) stay `is_active: false` in the
    DarkWebMonitor by design — a Tor-proxy safety control, not a registry
    exclusion — so they are deliberately not registered here either.
    """
    entries = [
        ("darkweb_hibp", "Have I Been Pwned", "https://haveibeenpwned.com",
         "breach notification service (clearnet)", "Cybersecurity"),
        ("darkweb_pastebin", "Pastebin", "https://pastebin.com",
         "paste/leak archive (clearnet)", "Cybersecurity"),
        ("darkweb_darkfeed", "Ransomware Blog Aggregator", "https://darkfeed.io",
         "ransomware leak/victim aggregator (clearnet)", "Cybersecurity"),
    ]
    out = []
    for slug, name, url, notes, category in entries:
        used.add(slug)
        out.append(make_entry(slug, name, url, "Global", category, 3,
                              notes=notes, interval=120))
    return out


def main() -> int:
    assigns = extract_assignments(DAEMON)
    if not assigns:
        print("FATAL: could not find source lists in crawl_daemon.py", file=sys.stderr)
        return 1

    global EXCLUDED_HOSTS, EXCLUDED_URLS
    EXCLUDED_HOSTS = {re.sub(r"^www\.", "", h) for h in builtin_hosts(REGISTRY_RS)}
    EXCLUDED_URLS = builtin_urls(REGISTRY_RS)

    news = literal(assigns["NEWS_SOURCES"])
    social = literal(assigns["SOCIAL_MEDIA_SOURCES"])
    onion = literal(assigns["ONION_SOURCES"])

    used: set[str] = set()
    entries: list[dict] = []
    entries += convert_news(news, used)
    entries += convert_social(social, used)
    entries += convert_onion(onion, used)
    entries += convert_dark_clearnet(used)

    # Drop supplement entries already covered by the built-in registry: news
    # entries by host, social entries by exact URL (a host like t.me or
    # reddit.com carries many distinct channels, each of which must remain).
    kept = []
    dropped = 0
    for e in entries:
        if e["url"].endswith(".onion"):
            kept.append(e)
            continue
        if e["category"] == "SocialMedia" or e["category"] == "Forum":
            if normalize_url(e["url"]) in EXCLUDED_URLS:
                dropped += 1
                continue
            kept.append(e)
            continue
        if host_of(e["url"]) in EXCLUDED_HOSTS:
            dropped += 1
            continue
        kept.append(e)

    header = (
        "# Source supplement registry — generated by scripts/dev/sync_legacy_sources.py.\n"
        "# Do not edit by hand; edit the generator or the legacy daemon lists instead.\n"
        "# Sources here are merged into the built-in registry at startup\n"
        "# (dedup by slug; hosts already present in the built-in registry are\n"
        "# treated as already-registered, never as excluded).  Every entry is\n"
        "# enabled: true and starts Unvalidated until a successful fetch/parser\n"
        "# contract check promotes it to Operational.\n"
        f"# Generated from: {DAEMON.relative_to(ROOT)}\n"
        f"# news={len(news)} social={len(social)} onion={len(onion)} "
        f"emitted={len(kept)} host-dedup-dropped={dropped}\n"
    )
    import yaml

    body = yaml.safe_dump(
        {"sources": kept}, sort_keys=False, default_flow_style=False, width=100
    )
    OUT.write_text(header + body, encoding="utf-8")
    print(f"Wrote {OUT} ({len(kept)} sources; {dropped} host-deduped against built-in registry)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
