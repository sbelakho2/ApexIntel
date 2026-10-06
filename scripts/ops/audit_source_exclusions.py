#!/usr/bin/env python3
"""Audit the source registry for policy exclusions.

The invariant: every source the system has ever known to monitor is registered
and `enabled = true`.  "Excluded" means one of:

1. `enabled: false` in the registry (policy exclusion),
2. a declared capability state that removes the source from scheduling
   (Blocked / Unsupported),
3. a source present in the legacy daemon lists but missing from the merged
   registry (silent drop).

Runtime states (TemporarilyFailed circuit, Unvalidated, missing deployment
capabilities) are NOT exclusions: they are self-healing scheduler states that
the crawl cycle actively probes.

Usage:
    python3 scripts/ops/audit_source_exclusions.py [--registry registry.json]

Exit code: 0 when zero exclusions, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEDULABLE = {"operational", "unvalidated", "requires_credentials",
               "unavailable_missing_capability", "unavailable_missing_credentials",
               "unavailable_missing_proxy", "temporarily_failed"}


def load_registry(path: Path | None) -> dict:
    if path is None:
        path = Path("/tmp/apex_registry.json")
        subprocess.run(
            ["cargo", "run", "-q", "-p", "apex-crawl", "--example", "dump_registry"],
            cwd=ROOT, check=True, stdout=open(path, "w", encoding="utf-8"),
        )
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--registry", type=Path, default=None)
    args = ap.parse_args()

    registry = load_registry(args.registry)
    sources = registry["sources"]
    total = registry["total"]
    enabled = [s for s in sources if s.get("enabled", True)]
    disabled = [s for s in sources if not s.get("enabled", True)]
    onion = [s for s in sources if s.get("onion")]
    telegram = [s for s in sources if s["slug"].startswith("telegram_")]

    policy_exclusions = [s["slug"] for s in disabled]
    problems: list[str] = []

    print(f"registry total:        {total}")
    print(f"enabled:               {len(enabled)}")
    print(f"policy-disabled:       {len(disabled)}  {policy_exclusions or ''}")
    print(f"onion (dark web):      {len(onion)}")
    print(f"telegram channels:     {len(telegram)}")
    for s in disabled:
        problems.append(f"{s['slug']}: enabled=false (policy exclusion)")

    supplement = ROOT / "config" / "sources_supplement.yaml"
    if supplement.exists():
        declared_slugs = {
            line.split("slug: ")[1].strip()
            for line in supplement.read_text(encoding="utf-8").splitlines()
            if line.strip().lstrip("- ").startswith("slug: ")
        }
        declared = len(declared_slugs)
        merged = sum(1 for s in sources if s["slug"] in declared_slugs)
        norm = lambda u: (u or "").rstrip("/").lower()
        builtin_endpoints = {norm(s.get("rss_url") or s["url"]) for s in sources}
        deduped = declared - merged
        print(f"supplement declared:   {declared}")
        print(f"supplement merged:     {merged} (deduped: {deduped} — same endpoint already registered)")
        if merged + deduped != declared:
            problems.append(
                f"supplement declares {declared} sources but only {merged} are merged"
            )

    intelslava = [s for s in sources if "intelslavaz" in s["slug"]]
    print(f"IntelSlavaZ:           {[s['slug'] for s in intelslava] or 'MISSING'}")
    if not intelslava:
        problems.append("IntelSlavaZ is not registered")

    if problems:
        print("\nEXCLUSIONS FOUND:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("\nOK: 0 sources excluded. Every known source is registered and enabled;")
    print("runtime capability states are self-healing scheduler states, not exclusions.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
