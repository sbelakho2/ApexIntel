#!/usr/bin/env bash
# Repository contamination gate.
#
# Fails when artifacts from unrelated projects or generated test output are
# tracked in this repository:
#   1. KiwiCaptcha / ApexMail artifacts (sibling projects): tracked paths or
#      file contents that reference them.
#   2. PNGs committed at the repository root (audit/Screenshot debris).
#   3. Playwright output directories (playwright-report/, test-results/).
#
# Run from anywhere; operates on `git ls-files` / `git grep` at the repo root.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

SELF="scripts/ci/check_repo_contamination.sh"
failed=0

fail() {
  echo "CONTAMINATION: $*" >&2
  failed=1
}

# ── 1. Unrelated project artifacts ────────────────────────────────────────
while IFS= read -r path; do
  [ -z "${path}" ] && continue
  fail "unrelated KiwiCaptcha/ApexMail artifact is tracked: ${path}"
done < <(git ls-files | grep -Ei 'kiwicaptcha|apexmail|kiwi-rust-test|n-wasm' || true)

while IFS= read -r path; do
  [ -z "${path}" ] && continue
  [ "${path}" = "${SELF}" ] && continue
  fail "tracked file references an unrelated ApexMail/KiwiCaptcha checkout: ${path}"
done < <(git grep -l -E '/Users/[^/]+/IdeaProjects/(ApexMail|KiwiCaptcha)|ApexMail/packages|kiwicaptcha' -- . ":!${SELF}" 2>/dev/null || true)

# ── 2. Root-level PNGs ────────────────────────────────────────────────────
while IFS= read -r path; do
  [ -z "${path}" ] && continue
  fail "PNG committed at the repository root: ${path}"
done < <(git ls-files | grep -E '^[^/]+\.png$' || true)

# ── 3. Playwright output ──────────────────────────────────────────────────
while IFS= read -r path; do
  [ -z "${path}" ] && continue
  fail "Playwright output directory is tracked: ${path}"
done < <(git ls-files | grep -E '^(playwright-report|test-results)/' || true)

if [ "${failed}" -ne 0 ]; then
  echo "repo contamination check FAILED" >&2
  exit 1
fi
echo "repo contamination check passed (no unrelated artifacts tracked)"
