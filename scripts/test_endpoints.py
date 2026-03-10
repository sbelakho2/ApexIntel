#!/usr/bin/env python3
import json
import os
import ssl
import unittest
import urllib.error
import urllib.request

DEFAULT_API_BASE = "http://localhost:8080"


def resolve_api_base() -> str:
    return os.getenv("APEX_API_BASE_URL", DEFAULT_API_BASE).rstrip("/")


def resolve_api_key() -> str:
    for env_name in ("APEX_API_KEY", "API_KEY"):
        value = os.getenv(env_name, "").strip()
        if value:
            return value
    raise RuntimeError("Set APEX_API_KEY or API_KEY before running scripts/test_endpoints.py")


def run_endpoint_probe(api: str, key: str) -> int:
    ctx = ssl.create_default_context()

    req = urllib.request.Request(f"{api}/api/endpoints", headers={"X-API-Key": key})
    with urllib.request.urlopen(req, context=ctx) as resp:
        data = json.loads(resp.read())
        eps = data if isinstance(data, list) else data.get("data", [])
        print(f"Total endpoints listed: {len(eps)}")
        for endpoint in eps:
            method = endpoint["method"]
            path = endpoint["path"]
            print(f"  {method} {path}")

    print()
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
    for route in routes:
        try:
            req = urllib.request.Request(f"{api}{route}", headers={"X-API-Key": key})
            with urllib.request.urlopen(req, context=ctx) as resp:
                print(f"  {resp.status}  {route}")
                ok += 1
        except urllib.error.HTTPError as err:
            print(f"  {err.code}  {route}")
            if err.code < 500:
                ok += 1
            else:
                fail += 1
        except Exception as err:
            print(f"  ERR  {route} ({err})")
            fail += 1

    print(f"\n{ok} OK, {fail} FAILED out of {len(routes)} routes tested")
    return fail


class SecretLookupTests(unittest.TestCase):
    def test_resolve_api_key_prefers_apex_api_key(self):
        old_apex = os.environ.get("APEX_API_KEY")
        old_api = os.environ.get("API_KEY")
        try:
            os.environ["APEX_API_KEY"] = "primary-key"
            os.environ["API_KEY"] = "fallback-key"
            self.assertEqual(resolve_api_key(), "primary-key")
        finally:
            if old_apex is None:
                os.environ.pop("APEX_API_KEY", None)
            else:
                os.environ["APEX_API_KEY"] = old_apex
            if old_api is None:
                os.environ.pop("API_KEY", None)
            else:
                os.environ["API_KEY"] = old_api

    def test_resolve_api_key_requires_environment(self):
        old_apex = os.environ.get("APEX_API_KEY")
        old_api = os.environ.get("API_KEY")
        try:
            os.environ.pop("APEX_API_KEY", None)
            os.environ.pop("API_KEY", None)
            with self.assertRaisesRegex(RuntimeError, "APEX_API_KEY"):
                resolve_api_key()
        finally:
            if old_apex is not None:
                os.environ["APEX_API_KEY"] = old_apex
            if old_api is not None:
                os.environ["API_KEY"] = old_api


if __name__ == "__main__":
    api = resolve_api_base()
    key = resolve_api_key()
    raise SystemExit(run_endpoint_probe(api, key))
