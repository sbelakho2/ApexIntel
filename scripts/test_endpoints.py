#!/usr/bin/env python3
import urllib.request, json, ssl

api = "https://starzerp.fi"
key = "sk-apex-prod-2026-starzerp"

ctx = ssl.create_default_context()

# Get endpoints listing
req = urllib.request.Request(f"{api}/api/endpoints", headers={"X-API-Key": key})
with urllib.request.urlopen(req, context=ctx) as resp:
    data = json.loads(resp.read())
    eps = data if isinstance(data, list) else data.get("data", [])
    print(f"Total endpoints listed: {len(eps)}")
    for e in eps:
        m = e["method"]
        p = e["path"]
        print(f"  {m} {p}")

print()
# Test all routes
routes = [
    "/api/health", "/api/endpoints", "/api/warnings?limit=1", "/api/insights?limit=1",
    "/api/insights/weekly-memo", "/api/companies?limit=1", "/api/persons?limit=1",
    "/api/search?q=test", "/api/graph", "/api/recipes?limit=1", "/api/security",
    "/api/sites?limit=1", "/api/capabilities?limit=1", "/api/certifications?limit=1",
    "/api/observations?limit=1", "/api/product-families?limit=1", "/api/logistics-nodes?limit=1",
    "/api/regulations?limit=1", "/api/poi-artifacts?limit=1", "/api/dashboard",
    "/api/competitors?limit=1", "/api/recipes/staging?limit=1",
    "/api/security/dns-posture", "/api/security/lookalike-domains", "/api/security/kev-relevance",
    "/api/admin/crawl-status", "/api/admin/recipe-performance", "/api/admin/poi-coverage",
    "/api/graph/neighborhood/00000000-0000-0000-0000-000000000001",
    "/api/graph/path/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002",
]

ok = 0
fail = 0
for r in routes:
    try:
        req = urllib.request.Request(f"{api}{r}", headers={"X-API-Key": key})
        with urllib.request.urlopen(req, context=ctx) as resp:
            print(f"  {resp.status}  {r}")
            ok += 1
    except urllib.error.HTTPError as e:
        print(f"  {e.code}  {r}")
        if e.code < 500:
            ok += 1
        else:
            fail += 1
    except Exception as e:
        print(f"  ERR  {r} ({e})")
        fail += 1

print(f"\n{ok} OK, {fail} FAILED out of {len(routes)} routes tested")
