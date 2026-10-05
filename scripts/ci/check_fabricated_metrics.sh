#!/usr/bin/env bash
# Fabricated-metrics guard (audit P0-10 backstop, broadened for audit item
# 177).
#
# Production web handlers and templates must never manufacture analytical
# values. The compiler cannot see a sinusoidal "trend" or a "seeded" array, so
# this gate is the backstop while type-level truthfulness (`Measurement<T>`,
# `DataState<T>`) is the primary mechanism.
#
# Fails when a production handler/template contains:
#   1. explicit fabrication vocabulary ("static seeded data", "demo data",
#      "fake data", "synthetic chart", ...);
#   2. trigonometric synthesis of a metric (`sin(`/`cos(` on a value used as a
#      chart/metric) inside web handlers or API handlers;
#   3. hard-coded operational labels the audit called out as passing:
#      analyst priority labels ("P0".."P3"), week-end/EOW deadline vocabulary,
#      and a "win" result emitted without any won-status gate in the same file;
#   4. hard-coded metric literals: information-gain/relevance percentages and
#      zeroed admin stat values assigned as constants instead of read from the
#      store. Rust `#[cfg(test)]` modules are masked before matching, so
#      fixtures remain legal.
#
# Resolved residual (was reported as a known gap here): `crates/api/src/web/
# companies.rs` no longer maps every product family to `status: "active"`.
# `ProductFamilyRow` carries no status column, so `CompanyProduct` exposes only
# the stored name/tech tags and the view renders no status claim. A future
# regression would reintroduce a `status: "active"` literal in that handler; it
# is deliberately not regex-enforced yet because a legitimate stored status
# column may add the same literal.
#
# Test modules are exempt: fixtures are allowed to be synthetic.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

# Container steps run as root while the agent checks the workspace out as the
# host user; without this, every git call fails with "dubious ownership" and
# git-based checks silently see an empty repository.
git config --global --add safe.directory '*' 2>/dev/null || true


failed=0
fail() {
  echo "FABRICATED-METRICS: $*" >&2
  failed=1
}

# ── 1. Fabrication vocabulary ───────────────────────────────────────────────
PHRASES='static seeded data|seeded data|demo data|fake data|placeholder data|synthetic chart|mock (trend|performance)'

hits=$(git grep -nEi "${PHRASES}" -- \
    'crates/api/src/web/**' \
    'crates/api/src/api_handlers/**' \
    'crates/api/templates/**' \
    ':!**/tests/**' 2>/dev/null || true)
if [ -n "${hits}" ]; then
  while IFS= read -r line; do
    fail "fabrication vocabulary in production surface: ${line}"
  done <<< "${hits}"
fi

# ── 2. Trigonometric metric synthesis in production handlers ────────────────
# `sin(`/`cos(` in a web/API handler has no legitimate use in this product:
# every chart is built from queried data. (Rendering math for geometry is not
# expected here; if it ever is, it belongs in a template macro, not a metric.)
trig=$(git grep -nE '\.(sin|cos)\(' -- \
    'crates/api/src/web/**/*.rs' \
    'crates/api/src/api_handlers/**/*.rs' \
    ':!**/tests/**' 2>/dev/null || true)
if [ -n "${trig}" ]; then
  while IFS= read -r line; do
    fail "trigonometric metric synthesis in a production handler: ${line}"
  done <<< "${trig}"
fi

# ── 3. Hard-coded operational labels ────────────────────────────────────────
# Analyst priority labels are computed from the priority vector; an "P1"
# literal is a fabricated work-item priority. Week-end/EOW deadlines are not
# stored anywhere: due dates come from `sla_deadline`/`due_date` columns.
priority_hits=$(git grep -nE '"(P0|P1|P2|P3)"' -- \
    'crates/api/src/web/**/*.rs' \
    'crates/api/src/api_handlers/**/*.rs' \
    'crates/api/templates/**' 2>/dev/null || true)
if [ -n "${priority_hits}" ]; then
  while IFS= read -r line; do
    fail "hard-coded priority label in a production surface: ${line}"
  done <<< "${priority_hits}"
fi

deadline_hits=$(git grep -nEi 'end[ -]of[ -]week|\bEOW\b|next week|next friday|this friday' -- \
    'crates/api/src/web/**/*.rs' \
    'crates/api/src/api_handlers/**/*.rs' \
    'crates/api/templates/**' 2>/dev/null || true)
if [ -n "${deadline_hits}" ]; then
  while IFS= read -r line; do
    fail "fabricated week-end deadline in a production surface: ${line}"
  done <<< "${deadline_hits}"
fi

# A "win" badge must be gated on a stored won/closed status; otherwise the
# wins/losses panel can never contain a win (or, worse, reports losses as
# wins). We check that any file emitting `result_type: "win"` also compares a
# status against "won" somewhere.
while IFS= read -r file; do
  [ -z "${file}" ] && continue
  if ! git grep -qE 'status[[:space:]]*==[[:space:]]*"won"' -- "${file}"; then
    fail "a \"win\" result is emitted without a won-status gate in ${file}"
  fi
done < <(git grep -lE 'result_type[[:space:]]*:[[:space:]]*"win"' -- \
    'crates/api/src/web/**/*.rs' \
    'crates/api/src/api_handlers/**/*.rs' 2>/dev/null || true)

# ── 4. Hard-coded metric literals (test modules masked) ─────────────────────
metric_hits=$(python3 - "${ROOT}" <<'PY'
import re
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])


def tracked(*paths: str) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "--", *paths],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [line for line in out.splitlines() if line.endswith(".rs")]


def blank_test_modules(text: str) -> str:
    """Blank every `#[cfg(test)]` item, preserving offsets and line numbers."""
    out = list(text)
    cursor = 0
    while True:
        marker = text.find("#[cfg(test)]", cursor)
        if marker < 0:
            break
        opening = text.find("{", marker)
        if opening < 0:
            break
        depth = 0
        end = opening
        while end < len(text):
            char = text[end]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    break
            end += 1
        for index in range(marker, min(end + 1, len(text))):
            if out[index] != "\n":
                out[index] = " "
        cursor = end + 1
    return "".join(out)


# Import the shared comment masker so a pattern named in a comment cannot fire.
sys.path.insert(0, str(root / "scripts" / "ci"))
from rust_source_mask import mask_comments  # noqa: E402

checks = [
    (
        "hard-coded information-gain metric",
        re.compile(r"information_gain_bits\s*:\s*Some\s*\(\s*[0-9]"),
    ),
    (
        "hard-coded relevance/quality percentage",
        re.compile(r"relevance(_pct|_percent|_score)?\s*:\s*(Some\s*\(\s*)?[0-9]"),
    ),
    (
        "hard-coded zero admin stat",
        re.compile(r"\b(value|records|count|total)\s*:\s*\"0\""),
    ),
    (
        "hard-coded uptime literal",
        re.compile(r"uptime\s*(=|:)\s*\""),
    ),
    (
        "fabricated uptime provenance",
        re.compile(r"since\s+first\s+page|first\s+page\s+view", re.IGNORECASE),
    ),
]

for relative in tracked("crates/api/src/web", "crates/api/src/api_handlers"):
    path = root / relative
    if not path.exists():
        # Deleted-but-still-tracked files (e.g. superseded handlers removed
        # during an audit) are not part of the checked surface.
        continue
    raw = path.read_text(encoding="utf-8", errors="replace")
    masked = mask_comments(blank_test_modules(raw))
    original_lines = raw.splitlines()
    for name, pattern in checks:
        for lineno, line in enumerate(masked.splitlines(), 1):
            if pattern.search(line):
                source = (
                    original_lines[lineno - 1].strip()
                    if lineno <= len(original_lines)
                    else line.strip()
                )
                print(f"{name}: {relative}:{lineno}: {source[:160]}")
PY
)
if [ -n "${metric_hits}" ]; then
  while IFS= read -r line; do
    fail "${line}"
  done <<< "${metric_hits}"
fi

if [ "${failed}" -ne 0 ]; then
  echo "fabricated-metrics guard FAILED" >&2
  exit 1
fi
echo "fabricated-metrics guard passed (no seeded/demo/trig values, hard-coded priorities, week-end deadlines, or metric literals in production surfaces)"
