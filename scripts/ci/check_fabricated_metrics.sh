#!/usr/bin/env bash
# Fabricated-metrics guard (audit P0-10 backstop).
#
# Production web handlers and templates must never manufacture analytical
# values. The compiler cannot see a sinusoidal "trend" or a "seeded" array, so
# this grep gate is the backstop while type-level truthfulness
# (`Measurement<T>`, `DataState<T>`) is the primary mechanism.
#
# Fails when a production handler/template contains:
#   1. explicit fabrication vocabulary ("static seeded data", "demo data",
#      "fake data", "synthetic chart", ...);
#   2. trigonometric synthesis of a metric (`sin(`/`cos(` on a value used as a
#      chart/metric) inside web handlers or API handlers.
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

if [ "${failed}" -ne 0 ]; then
  echo "fabricated-metrics guard FAILED" >&2
  exit 1
fi
echo "fabricated-metrics guard passed (no seeded/demo values or trig metrics in production handlers)"
