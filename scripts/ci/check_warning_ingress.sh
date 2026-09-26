#!/usr/bin/env bash
# Warning-ingress source guard.
#
# Every warning produced by the worker must go through the shared
# `IntelligenceIngress` (`crates/worker/src/intelligence_ingress.rs`) so it is
# deterministically deduplicated, inserted, submitted to semantic triage,
# published as an alert/domain event, and recorded in the activity feed.
#
# A direct `store.insert_warning(...)` call bypasses semantic triage dedup and
# alert publication, which is exactly the class of bug this guard prevents.
#
# Rule: `insert_warning(` may appear in `crates/worker/src/` ONLY inside
# `intelligence_ingress.rs` (the single ingress module). In particular, no file
# under `crates/worker/src/job_execution/` may call it.
#
# Run from anywhere; operates on the working tree.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

SCAN_ROOT="crates/worker/src"
INGRESS="crates/worker/src/intelligence_ingress.rs"
JOBS_DIR="crates/worker/src/job_execution"

failed=0

# ── 1. job_execution must be completely clean ─────────────────────────────
while IFS= read -r match; do
  [ -z "${match}" ] && continue
  echo "WARNING INGRESS: job calls insert_warning(...) directly: ${match}" >&2
  failed=1
done < <(grep -RIn --include='*.rs' 'insert_warning(' "${JOBS_DIR}" 2>/dev/null || true)

# ── 2. The rest of the worker source: only the ingress module ─────────────
while IFS= read -r match; do
  [ -z "${match}" ] && continue
  echo "WARNING INGRESS: insert_warning(...) called outside ${INGRESS}: ${match}" >&2
  failed=1
done < <(grep -RIn --include='*.rs' 'insert_warning(' "${SCAN_ROOT}" 2>/dev/null | grep -v "^${INGRESS}:" || true)

if [ "${failed}" -ne 0 ]; then
  echo "warning ingress check FAILED — submit warnings through IntelligenceIngress instead" >&2
  exit 1
fi
echo "warning ingress check passed (no direct insert_warning calls outside ${INGRESS})"
