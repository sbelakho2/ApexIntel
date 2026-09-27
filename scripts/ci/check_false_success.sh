#!/usr/bin/env bash
# False-success classification guard.
#
# Scans production Rust in the worker job-execution and API web/handler
# packages for fallible store/database reads and ignored writes:
#
#   * `.unwrap_or_default()` / `.unwrap_or(false)` / `.ok().flatten()` in a
#     database/store/row-extraction context
#   * `let _ = <...>.await` ignored write/read results
#
# Every such occurrence must carry an explicit adjacent classification
# comment so a reviewer can tell best-effort telemetry from authoritative
# persistence:
#
#   // false-success-classification: best-effort — <why>
#   // false-success-classification: authoritative — <why>
#
# Authoritative persistence is expected to be fail/degrade with a distinct
# counter rather than classified as best-effort; this guard only enforces
# that the decision was made and written down.
#
# Run from anywhere; operates on the repository tree.
# Usage: check_false_success.sh [--self-test]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

python3 - "$@" <<'PY'
import pathlib
import re
import subprocess
import sys
import tempfile

SCAN_DIRS = [
    "crates/worker/src/job_execution",
    "crates/api/src/web",
    "crates/api/src/api_handlers",
]

FALLBACK = re.compile(r"\.unwrap_or_default\(\)|\.unwrap_or\(false\)|\.ok\(\)\.flatten\(\)")
DB_HINT = re.compile(r"\bstore\b|\bsqlx\b|\bquery_as\b|\bquery\(|try_get\(|fetch_one|fetch_all|fetch_optional")
IGNORED_WRITE = re.compile(r"^\s*let _ =")
MARKER = re.compile(r"false-success-classification:\s*(best-effort|authoritative)")
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]")


def scan_lines(lines):
    """Return [(line_number, stripped_line)] for unclassified matches."""
    cut = len(lines)
    for index, line in enumerate(lines):
        if CFG_TEST.match(line):
            cut = index
            break

    findings = []
    for index in range(cut):
        line = lines[index]
        stripped = line.strip()
        if stripped.startswith("//"):
            continue

        target = False
        if FALLBACK.search(line) and any(
            DB_HINT.search(lines[lookback])
            for lookback in range(max(0, index - 8), index + 1)
        ):
            target = True
        elif IGNORED_WRITE.match(line) and ".await" in "".join(lines[index:index + 8]):
            target = True

        if not target:
            continue

        window = lines[max(0, index - 3):index + 1]
        if any(MARKER.search(candidate) for candidate in window):
            continue
        findings.append((index + 1, stripped[:140]))
    return findings


def scan_repo(root):
    findings = []
    for directory in SCAN_DIRS:
        for path in sorted(pathlib.Path(root, directory).rglob("*.rs")):
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            for line_number, text in scan_lines(lines):
                findings.append(f"{path.relative_to(root)}:{line_number}: {text}")
    return findings


def self_test(root):
    """Prove the guard fails on an unclassified match and passes once classified."""
    fixture = tempfile.TemporaryDirectory()
    base = pathlib.Path(fixture.name)
    unclassified = base / "unclassified.rs"
    classified = base / "classified.rs"
    unclassified.write_text(
        "async fn f(store: &Store) {\n"
        "    let rows = store.list_things().await.unwrap_or_default();\n"
        "    let _ = store.record_thing(rows).await;\n"
        "}\n"
    )
    classified.write_text(
        "async fn f(store: &Store) {\n"
        "    // false-success-classification: best-effort — optional telemetry only\n"
        "    let rows = store.list_things().await.unwrap_or_default();\n"
        "    // false-success-classification: authoritative — audit write must be counted\n"
        "    let _ = store.record_thing(rows).await;\n"
        "}\n"
    )

    unclassified_findings = scan_lines(unclassified.read_text().splitlines())
    classified_findings = scan_lines(classified.read_text().splitlines())
    fixture.cleanup()

    if len(unclassified_findings) < 2:
        print(
            "SELF-TEST FAILED: expected the unclassified fixture to produce >=2 findings, "
            f"got {unclassified_findings}",
            file=sys.stderr,
        )
        return 1
    if classified_findings:
        print(
            f"SELF-TEST FAILED: classified fixture still reports {classified_findings}",
            file=sys.stderr,
        )
        return 1
    print("false-success guard self-test passed")
    return 0


def main():
    args = sys.argv[1:]
    # This script is executed from the repository root (see the bash wrapper).
    root = pathlib.Path.cwd()
    if "--self-test" in args:
        return self_test(root)

    findings = scan_repo(root)
    if findings:
        for finding in findings:
            print(f"FALSE-SUCCESS: unclassified fallible read/write: {finding}", file=sys.stderr)
        print(
            f"false-success guard FAILED: {len(findings)} unclassified occurrence(s); "
            "add a 'false-success-classification: best-effort|authoritative — <reason>' comment",
            file=sys.stderr,
        )
        return 1
    print("false-success guard passed (all fallible reads/writes classified)")
    return 0


sys.exit(main())
PY
