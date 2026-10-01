#!/usr/bin/env bash
# Repository contamination gate.
#
# Fails when artifacts from unrelated projects or generated test output are
# tracked in this repository:
#   1. KiwiCaptcha / ApexMail artifacts (sibling projects): tracked paths or
#      file contents that reference them.
#   2. PNGs committed at the repository root (audit/Screenshot debris).
#   3. Playwright output directories (playwright-report/, test-results/).
#   4. Personal absolute paths (/Users/<name>/..., /home/<name>/...) and
#      unrelated production hosts (app.apexmail.ee / apexmail.ee /
#      starzerp.fi) hardcoded in tracked scripts.
#
# scripts/ci/check_*.sh guard scripts are exempt from rule 4: they deliberately
# encode the strings they detect (e.g. the docs-contract gate names
# starzerp.fi), so scanning them would flag the detector instead of a
# regression.
#
# Run from anywhere; operates on `git ls-files` / `git grep` at the repo root.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

# Container steps run as root while the agent checks the workspace out as the
# host user; without this, every git call fails with "dubious ownership" and
# git-based checks silently see an empty repository.
git config --global --add safe.directory '*' 2>/dev/null || true


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

# ── 4. Personal paths / production hosts in tracked scripts ───────────────
# The check runs on every push: a dev helper committed with one developer's
# absolute path or another deployment's hostname must fail CI, not rot in the
# tree. Guard scripts are exempt (they encode these patterns on purpose).
script_paths() {
  git ls-files \
    | grep -E '\.(sh|bash|zsh|py|js|mjs|cjs|ts)$' \
    | grep -Ev '^scripts/ci/check_[^/]*\.sh$' \
    | grep -Fvx "${SELF}" || true
}

while IFS= read -r path; do
  [ -z "${path}" ] && continue
  while IFS= read -r hit; do
    [ -z "${hit}" ] && continue
    fail "tracked script hardcodes a personal absolute path: ${hit}"
  done < <(git grep -n -E '(/Users|/home)/[A-Za-z0-9._-]+/' -- "${path}" 2>/dev/null || true)
  while IFS= read -r hit; do
    [ -z "${hit}" ] && continue
    fail "tracked script references an unrelated production host: ${hit}"
  done < <(git grep -n -E 'app\.apexmail\.ee|apexmail\.ee|starzerp\.fi' -- "${path}" 2>/dev/null || true)
done < <(script_paths)

if [ "${failed}" -ne 0 ]; then
  echo "repo contamination check FAILED" >&2
  exit 1
fi
echo "repo contamination check passed (no unrelated artifacts tracked)"
