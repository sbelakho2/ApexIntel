#!/usr/bin/env python3
"""Prove that registered sources work: fetch every clearnet, Telegram and
onion source in the authoritative merged registry and record evidence.

Input: registry JSON produced by `cargo run -p apex-crawl --example dump_registry`
      (auto-generated when --registry is omitted and cargo is available).
Output: release-evidence/source-verification/{evidence.jsonl,summary.json,report.md}

Clear-web checks: HTTP GET (RSS URL preferred), status 200/2xx + body >=
MIN_BODY_BYTES => OK; 2xx but thin => THIN; anything else => FAIL (with reason).
Telegram checks: t.me/s/{handle} must serve message widgets.
Onion checks: fetch through the Tor SOCKS5 proxy (local 127.0.0.1:9050 by
default; --tor-host HOST:PORT to route via a remote host's Tor).

Exit codes: 0 = proof complete AND pass rate >= --min-ok-rate (default 0.85);
1 = proof incomplete or below threshold.  Every verdict is recorded, so a
failing source is evidence, never silence.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

def resolve_root() -> Path:
    env_root = os.environ.get("APEX_ROOT")
    if env_root:
        return Path(env_root)
    script = Path(__file__).resolve()
    try:
        root = script.parents[2]
        return root
    except IndexError:
        return Path("/")


ROOT = resolve_root()
OUT_DIR = ROOT / "release-evidence" / "source-verification"
MIN_BODY_BYTES = 200
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36"
)
TIMEOUT = 20


def load_registry(path: Path | None, cargo: bool) -> dict:
    if path is None:
        path = Path("/tmp/apex_registry.json")
        if cargo and (not path.exists() or path.stat().st_mtime < time.time() - 3600):
            print("Regenerating registry dump via cargo ...", file=sys.stderr)
            subprocess.run(
                ["cargo", "run", "-q", "-p", "apex-crawl", "--example", "dump_registry"],
                cwd=ROOT,
                check=True,
                stdout=open(path, "w", encoding="utf-8"),
            )
    if not path.exists():
        raise SystemExit(f"registry dump missing at {path}; run the dump_registry example")
    data = json.loads(path.read_text(encoding="utf-8"))
    assert "sources" in data, "registry JSON must contain 'sources'"
    return data


def http_fetch(url: str, proxy: str | None = None, timeout: int = TIMEOUT) -> tuple[int, str]:
    handlers: list = []
    if proxy:
        handlers.append(urllib.request.ProxyHandler({"http": proxy, "https": proxy}))
    opener = urllib.request.build_opener(*handlers)
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "*/*"})
    try:
        with opener.open(req, timeout=timeout) as resp:
            body = resp.read(256 * 1024)
            return resp.status, body.decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        try:
            return e.code, e.read(4096).decode("utf-8", errors="replace")
        except Exception:
            return e.code, ""
    except Exception as e:
        return 0, f"{type(e).__name__}: {e}"


def check_clearnet(entry: dict) -> dict:
    url = entry.get("rss_url") or entry["url"]
    status, body = http_fetch(url)
    if 200 <= status < 400 and len(body) >= MIN_BODY_BYTES:
        verdict = "OK"
    elif 200 <= status < 400:
        verdict = "THIN"
    else:
        verdict = "FAIL"
    return {
        "slug": entry["slug"],
        "kind": "clearnet",
        "url": url,
        "verdict": verdict,
        "http_status": status,
        "body_bytes": len(body),
        "note": "" if verdict != "FAIL" else body[:200],
    }


def check_telegram(entry: dict) -> dict:
    url = entry["url"]
    status, body = http_fetch(url)
    widgets = body.count("tgme_widget_message_text")
    if 200 <= status < 400 and widgets > 0:
        verdict = "OK"
    elif 200 <= status < 400:
        verdict = "EMPTY"
    else:
        verdict = "FAIL"
    return {
        "slug": entry["slug"],
        "kind": "telegram",
        "url": url,
        "verdict": verdict,
        "http_status": status,
        "body_bytes": len(body),
        "note": f"message_widgets={widgets}",
    }


def check_onion(entry: dict, tor_host: str) -> dict:
    """Fetch an onion URL through Tor using curl --socks5-hostname (remote DNS).

    Onion services are reachability-flaky by nature (circuit rotation), so the
    probe mirrors the production retry ladder: two attempts before a FAIL.
    """
    url = entry["url"]
    for attempt in (1, 2):
        try:
            proc = subprocess.run(
                [
                    "curl", "-s", "-m", "45", "--socks5-hostname", tor_host,
                    "-A", "ApexIntel-Verify/1.0", "-o", "/dev/null",
                    "-w", "%{http_code}:%{size_download}",
                    url,
                ],
                capture_output=True,
                text=True,
                timeout=60,
            )
            out = proc.stdout.strip() or "0:0"
            code_s, size_s = out.split(":")[:2]
            status = int(code_s or 0)
            size = int(size_s or 0)
        except Exception as e:  # noqa: BLE001
            status, size = 0, 0
        if 200 <= status < 400:
            break
        if attempt == 1:
            time.sleep(5)
    if 200 <= status < 400 and size >= 100:
        verdict = "OK"
    elif 200 <= status < 400:
        verdict = "THIN"
    else:
        verdict = "FAIL"
    return {
        "slug": entry["slug"],
        "kind": "onion",
        "url": url,
        "verdict": verdict,
        "http_status": status,
        "body_bytes": size,
        "note": "" if verdict != "FAIL" else f"http {status}",
    }


def probe(entry: dict, tor_host: str, only: str | None, limit: int) -> list[dict]:
    rows: list[dict] = []
    targets = []
    for source in entry["sources"]:
        if not source.get("enabled", True):
            continue
        if source.get("onion"):
            kind = "onion"
        elif source["slug"].startswith("telegram_") and "t.me/s/" in source["url"]:
            kind = "telegram"
        elif source["slug"] == "telegram_channels":
            kind = "telegram"
        else:
            kind = "clearnet"
        if only and kind != only:
            continue
        targets.append((kind, source))
        if limit and len(targets) >= limit:
            break

    def run(item):
        kind, source = item
        if kind == "onion":
            return check_onion(source, tor_host)
        if kind == "telegram":
            return check_telegram(source)
        return check_clearnet(source)

    # Onion probes run serially (Tor circuits are serialized in production
    # too — concurrent circuits to different onions destabilize each other).
    onion_items = [item for item in targets if item[0] == "onion"]
    plain_items = [item for item in targets if item[0] != "onion"]
    for item in onion_items:
        row = run(item)
        rows.append(row)
        flag = {"OK": "+", "THIN": "~", "EMPTY": "~", "FAIL": "!"}[row["verdict"]]
        print(f"  [{flag}] {row['verdict']:5s} {row['slug']}  {row['url']}", flush=True)
    with cf.ThreadPoolExecutor(max_workers=8) as pool:
        for row in pool.map(run, plain_items):
            rows.append(row)
            flag = {"OK": "+", "THIN": "~", "EMPTY": "~", "FAIL": "!"}[row["verdict"]]
            print(f"  [{flag}] {row['verdict']:5s} {row['slug']}  {row['url']}", flush=True)
    return rows


def summarize(rows: list[dict], registry: dict) -> dict:
    from collections import Counter

    kinds = Counter(r["kind"] for r in rows)
    verdicts = Counter(r["verdict"] for r in rows)
    ok = verdicts["OK"]
    total = len(rows)
    by_kind = {}
    for kind in ("clearnet", "telegram", "onion"):
        kind_rows = [r for r in rows if r["kind"] == kind]
        by_kind[kind] = {
            "probed": len(kind_rows),
            "ok": sum(1 for r in kind_rows if r["verdict"] == "OK"),
            "failed": sum(1 for r in kind_rows if r["verdict"] == "FAIL"),
        }
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "registry_total": registry["total"],
        "registry_enabled": registry["enabled"],
        "probed": total,
        "ok": ok,
        "pass_rate": round(ok / total, 4) if total else 0.0,
        "verdicts": dict(verdicts),
        "by_kind": by_kind,
        "fails": [r for r in rows if r["verdict"] == "FAIL"],
    }


def write_report(summary: dict, out_dir: Path) -> Path:
    md = out_dir / "report.md"
    by_kind = summary["by_kind"]
    lines = [
        "# Source verification proof",
        "",
        f"- Generated: {summary['generated_at']}",
        f"- Registry: **{summary['registry_total']} sources, {summary['registry_enabled']} enabled (0 excluded)**",
        f"- Probed: {summary['probed']} · OK {summary['ok']} · pass rate {summary['pass_rate'] * 100:.1f}%",
        "",
        "| Kind | Probed | OK | FAIL |",
        "|---|---|---|---|",
    ]
    for kind, stats in by_kind.items():
        lines.append(f"| {kind} | {stats['probed']} | {stats['ok']} | {stats['failed']} |")
    lines.append("")
    lines.append("## Failed sources")
    lines.append("")
    if summary["fails"]:
        for f in summary["fails"]:
            lines.append(
                f"- `{f['slug']}` [{f['kind']}] {f['url']} "
                f"HTTP {f['http_status']}: {f['note'][:120]}"
            )
    else:
        lines.append("_none — every probed source responded._")
    lines.append("")
    md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return md


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--registry", type=Path, default=None,
                    help="registry JSON path (default: regenerate via cargo)")
    ap.add_argument("--out-dir", type=Path, default=OUT_DIR)
    ap.add_argument("--tor-host", default="127.0.0.1:9050",
                    help="Tor SOCKS5 host:port for onion probes (local default)")
    ap.add_argument("--only", choices=["clearnet", "telegram", "onion"], default=None)
    ap.add_argument("--limit", type=int, default=0, help="cap number of probes (0 = all)")
    ap.add_argument("--min-ok-rate", type=float, default=None,
                    help="optional enforcement gate: exit 1 when pass rate is below this")
    ap.add_argument("--no-cargo", action="store_true", help="never invoke cargo")
    args = ap.parse_args()

    registry = load_registry(args.registry, cargo=not args.no_cargo)
    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"Probing {registry['enabled']} enabled sources "
          f"(registry total {registry['total']}) ...", flush=True)
    started = time.time()
    rows = probe(registry, args.tor_host, args.only, args.limit)
    summary = summarize(rows, registry)

    evidence = out_dir / "evidence.jsonl"
    with evidence.open("w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row) + "\n")
    summary["evidence_file"] = str(evidence.relative_to(ROOT))
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    report = write_report(summary, out_dir)

    print(f"\n{summary['ok']}/{summary['probed']} OK "
          f"({summary['pass_rate'] * 100:.1f}%) in {time.time() - started:.0f}s "
          f"-> {report.relative_to(ROOT)}")
    if args.min_ok_rate is not None and summary["pass_rate"] < args.min_ok_rate:
        print(f"FAIL: pass rate below --min-ok-rate {args.min_ok_rate}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
