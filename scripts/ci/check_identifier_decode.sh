#!/usr/bin/env bash
# Identifier-decode guard.
#
# A UUID primary key or identifier that fails to decode must be an error, not
# the nil UUID, a fresh random UUID, or an empty/zero default. Fabricated
# identity is worse than a visible failure: it links records to the wrong row
# and reads as success downstream.
#
# This rule fails on:
#   * `Uuid::parse_str(..)` followed by `.unwrap_or_default()`,
#     `.unwrap_or(Uuid::nil())`, `.unwrap_or_else(|_| Uuid::new_v4())` or
#     `.unwrap_or_else(|_| Uuid::nil())` — including nested call arguments.
#   * `row.try_get(..)` / `try_get::<Uuid, _>(..)` followed by
#     `.unwrap_or_default()` / `.unwrap_or(0)` / `.unwrap_or("")` /
#     `.unwrap_or(Uuid::nil())` when the column or target type is
#     identifier-ish (`id`, `*_id`, `uuid`, `*_uuid`, `pk`) — including
#     non-null String/bool columns on identity rows.
#
# Patterns run against masked source (comments and string literals blanked for
# the parse rule, comments-only for the try_get rule, which needs the column
# name literal), so documentation can never change correctness status.
#
# Identifier rows must be decoded with a typed `query_as` struct (or the error
# propagated), so a decode failure produces an error instead of a nil id.
#
# Run from anywhere; operates on the repository tree.
# Usage: check_identifier_decode.sh [--self-test]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export GUARD_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${ROOT}"

python3 - "$@" <<'PY'
import os
import pathlib
import re
import sys
import tempfile

sys.path.insert(0, os.environ.get("GUARD_SCRIPTS_DIR", "."))
from rust_source_mask import mask_comments, mask_noncode  # noqa: E402

SCAN_ROOT = pathlib.Path("crates")
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]")

# Defaults that fabricate an identifier after a failed parse or row decode.
FABRICATED_DEFAULT = (
    r"(?:"
    r"\.unwrap_or_default\(\)"
    r"|\.unwrap_or\(\s*(?:uuid::)?Uuid::nil\(\)\s*\)"
    r"|\.unwrap_or_else\(\s*\|\s*_\s*\|\s*(?:uuid::)?Uuid::new_v4\(\)\s*\)"
    r"|\.unwrap_or_else\(\s*\|\s*_\s*\|\s*(?:uuid::)?Uuid::nil\(\)\s*\)"
    r")"
)
PARSE_FABRICATION_TAIL = re.compile(r"^\s*" + FABRICATED_DEFAULT)
TRY_GET = re.compile(
    r"try_get\s*(?:::<([^>]*)>)?\s*\(\s*(?:\"([^\"]*)\"|[A-Za-z_][A-Za-z0-9_]*)\s*\)"
)
FALSE_DEFAULT = re.compile(
    r"\.unwrap_or_default\(\)|\.unwrap_or\(0\)|\.unwrap_or\(\"\"\)"
    r"|\.unwrap_or\(\s*(?:uuid::)?Uuid::nil\(\)\s*\)"
)

IDENTIFIER_COLUMN = re.compile(r"(^|_)(id|uuid|pk)$", re.IGNORECASE)


def is_identifier(type_text, column):
    if column and IDENTIFIER_COLUMN.search(column):
        return True
    if type_text and "Uuid" in type_text:
        return True
    return False


def parse_str_fabrications(masked):
    """Find `Uuid::parse_str(...)` calls followed by a fabricated default.

    Uses a balanced-parenthesis scan for the argument so nested calls such as
    `Uuid::parse_str(&normalize(strip(&raw))).unwrap_or_default()` are caught.
    """
    findings = []
    i = 0
    while True:
        idx = masked.find("Uuid::parse_str", i)
        if idx == -1:
            break
        paren = masked.find("(", idx)
        if paren == -1:
            break
        depth = 0
        j = paren
        while j < len(masked):
            c = masked[j]
            if c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        tail = masked[j + 1: j + 1 + 200]
        match = PARSE_FABRICATION_TAIL.match(tail)
        if match:
            findings.append((idx, match.group(0).strip()))
        i = j + 1
    return findings


def line_of(text, offset):
    return text[:offset].count("\n") + 1


def scan_text(text):
    """Return findings for one Rust source text as (line, snippet) tuples."""
    findings = []

    # Parse fabrications read code only: comments and string arguments are
    # masked so a documented anti-pattern cannot fail the guard.
    masked_code = mask_noncode(text)
    for offset, snippet in parse_str_fabrications(masked_code):
        findings.append((line_of(masked_code, offset), f"Uuid::parse_str(...){snippet}"))

    # try_get needs the column-name literal, so only comments are masked.
    masked_no_comments = mask_comments(text)
    for match in TRY_GET.finditer(masked_no_comments):
        type_text, column = match.group(1), match.group(2)
        if not is_identifier(type_text, column):
            continue
        tail = masked_no_comments[match.end(): match.end() + 80]
        false_default = FALSE_DEFAULT.search(tail)
        if false_default:
            findings.append(
                (
                    line_of(masked_no_comments, match.start()),
                    f"{match.group(0)}...{false_default.group(0)}",
                )
            )
    return findings


def scan_repo(root):
    findings = []
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        cfg_cut = len(lines)
        for index, line in enumerate(lines):
            if CFG_TEST.match(line):
                cfg_cut = index
                break
        rel = path.relative_to(pathlib.Path.cwd())
        for line, snippet in scan_text(text):
            if line > cfg_cut:
                continue
            findings.append(f"{rel}:{line}: {snippet}")
    return findings


def self_test(root):
    fixture = tempfile.TemporaryDirectory()
    base = pathlib.Path(fixture.name)
    bad = base / "bad.rs"
    good = base / "good.rs"
    commented = base / "commented.rs"
    bad.write_text(
        "fn f(row: &Row) -> Uuid {\n"
        "    let id: uuid::Uuid = row.try_get(\"id\").unwrap_or_default();\n"
        "    let other = Uuid::parse_str(&raw).unwrap_or_default();\n"
        "    let fresh = Uuid::parse_str(&raw)\n"
        "        .unwrap_or_else(|_| Uuid::new_v4());\n"
        "    let name: String = row.try_get(\"user_id\").unwrap_or_default();\n"
        "    let nested = Uuid::parse_str(&normalize(strip(&raw))).unwrap_or_default();\n"
        "    let nil = Uuid::parse_str(&raw).unwrap_or(uuid::Uuid::nil());\n"
        "    let nil_lazy = Uuid::parse_str(&raw)\n"
        "        .unwrap_or_else(|_| uuid::Uuid::nil());\n"
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
    commented.write_text(
        "// never write Uuid::parse_str(&raw).unwrap_or_default()\n"
        "// or row.try_get(\"id\").unwrap_or_default()\n"
        "fn f(rows: Vec<Row>) -> Uuid {\n"
        "    rows[0].id\n"
        "}\n"
    )

    bad_findings = scan_text(bad.read_text())
    good_findings = scan_text(good.read_text())
    commented_findings = scan_text(commented.read_text())
    fixture.cleanup()

    if len(bad_findings) < 7:
        print(
            f"SELF-TEST FAILED: expected >=7 findings for fabricated identifiers, got {bad_findings}",
            file=sys.stderr,
        )
        return 1
    if good_findings:
        print(
            f"SELF-TEST FAILED: typed decode reported findings {good_findings}",
            file=sys.stderr,
        )
        return 1
    if commented_findings:
        print(
            f"SELF-TEST FAILED: commented-out patterns reported findings {commented_findings}",
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
