#!/usr/bin/env bash
# Identifier-decode guard.
#
# A UUID primary key or identifier that fails to decode must be an error, not
# the nil UUID, a fresh random UUID, or an empty/zero default. Fabricated
# identity is worse than a visible failure: it links records to the wrong row
# and reads as success downstream.
#
# This rule fails on:
#   * `Uuid::parse_str(..).unwrap_or_default()`            (nil UUID)
#   * `Uuid::parse_str(..).unwrap_or_else(|_| Uuid::new_v4())` (fresh identity)
#   * `row.try_get(..)` / `try_get::<Uuid, _>(..)` followed by
#     `.unwrap_or_default()` / `.unwrap_or(0)` / `.unwrap_or("")` when the
#     column or target type is identifier-ish (`id`, `*_id`, `uuid`, `*_uuid`,
#     `pk`) — including non-null String/bool columns on identity rows.
#
# Identifier rows must be decoded with a typed `query_as` struct (or the error
# propagated), so a decode failure produces an error instead of a nil id.
#
# Run from anywhere; operates on the repository tree.
# Usage: check_identifier_decode.sh [--self-test]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

python3 - "$@" <<'PY'
import pathlib
import re
import sys
import tempfile

SCAN_ROOT = pathlib.Path("crates")
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]")

# `Uuid::parse_str(...)` followed by a fabricated default.
PARSE_FABRICATION_MULTILINE = re.compile(
    r"Uuid::parse_str\s*\((?:[^()]|\([^()]*\))*\)\s*"
    r"(\.unwrap_or_default\(\)|\.unwrap_or_else\(\s*\|\s*_\s*\|\s*(?:uuid::)?Uuid::new_v4\(\)\s*\))",
    re.S,
)

TRY_GET = re.compile(r"try_get\s*(?:::<([^>]*)>)?\s*\(\s*(?:\"([^\"]*)\"|[A-Za-z_][A-Za-z0-9_]*)\s*\)")
FALSE_DEFAULT = re.compile(r"\.unwrap_or_default\(\)|\.unwrap_or\(0\)|\.unwrap_or\(\"\"\)")

IDENTIFIER_COLUMN = re.compile(r"(^|_)(id|uuid|pk)$", re.IGNORECASE)


def is_identifier(type_text, column):
    if column and IDENTIFIER_COLUMN.search(column):
        return True
    if type_text and "Uuid" in type_text:
        return True
    return False


def line_is_test_only(lines, index):
    """True when the line sits inside a `#[cfg(test)]` module."""
    for lookback in range(index, -1, -1):
        if CFG_TEST.match(lines[lookback]):
            # Everything after the first cfg(test) is test code for this file
            # (the workspace convention is one trailing test module).
            return True
    return False


def scan_text(text):
    findings = []
    for match in PARSE_FABRICATION_MULTILINE.finditer(text):
        line = text[: match.start()].count("\n") + 1
        findings.append((line, match.group(0).splitlines()[0].strip()))
    for match in TRY_GET.finditer(text):
        type_text, column = match.group(1), match.group(2)
        if not is_identifier(type_text, column):
            continue
        tail = text[match.end(): match.end() + 80]
        false_default = FALSE_DEFAULT.search(tail)
        if false_default:
            line = text[: match.start()].count("\n") + 1
            findings.append(
                (line, f"{match.group(0)}...{false_default.group(0)}")
            )
    return findings


def scan_repo(root):
    findings = []
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        for line, text_match in scan_text(text):
            if line <= len(lines) and line_is_test_only(lines, line - 1):
                continue
            findings.append(f"{path.relative_to(pathlib.Path.cwd())}:{line}: {text_match}")
    return findings


def self_test(root):
    fixture = tempfile.TemporaryDirectory()
    base = pathlib.Path(fixture.name)
    bad = base / "bad.rs"
    good = base / "good.rs"
    bad.write_text(
        "fn f(row: &Row) -> Uuid {\n"
        "    let id: uuid::Uuid = row.try_get(\"id\").unwrap_or_default();\n"
        "    let other = Uuid::parse_str(&raw).unwrap_or_default();\n"
        "    let fresh = Uuid::parse_str(&raw)\n"
        "        .unwrap_or_else(|_| Uuid::new_v4());\n"
        "    let name: String = row.try_get(\"user_id\").unwrap_or_default();\n"
        "    id\n"
        "}\n"
    )
    good.write_text(
        "#[derive(sqlx::FromRow)]\n"
        "struct Row { id: uuid::Uuid, name: String }\n"
        "fn f(rows: Vec<Row>) -> Uuid {\n"
        "    rows[0].id\n"
        "}\n"
    )
    bad_findings = scan_text(bad.read_text())
    good_findings = scan_text(good.read_text())
    fixture.cleanup()

    if len(bad_findings) < 4:
        print(
            f"SELF-TEST FAILED: expected >=4 findings for fabricated identifiers, got {bad_findings}",
            file=sys.stderr,
        )
        return 1
    if good_findings:
        print(
            f"SELF-TEST FAILED: typed decode reported findings {good_findings}",
            file=sys.stderr,
        )
        return 1
    print("identifier-decode guard self-test passed")
    return 0


def main():
    root = pathlib.Path.cwd()
    if "--self-test" in sys.argv[1:]:
        return self_test(root)

    findings = scan_repo(root / SCAN_ROOT)
    if findings:
        for finding in findings:
            print(f"IDENTIFIER-DECODE: fabricated/default identifier: {finding}", file=sys.stderr)
        print(
            f"identifier-decode guard FAILED: {len(findings)} fabricated identifier decode(s); "
            "use a typed query_as row and propagate the decode error",
            file=sys.stderr,
        )
        return 1
    print("identifier-decode guard passed (no fabricated identifier defaults)")
    return 0


sys.exit(main())
PY
