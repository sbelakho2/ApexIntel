#!/usr/bin/env bash
# Single migration lineage gate (audit #175).
#
# The repository used to carry two migration lineages:
#   * migrations/                 (authoritative, embedded by sqlx)
#   * crates/store/migrations/    (stale secondary lineage, deleted by #175)
#
# Two lineages meant two sources of truth: a schema fix could land in the one
# the binary does not embed, and a fresh `sqlx::migrate!` call in another crate
# could silently apply a divergent chain. This gate enforces:
#   1. the stale crates/store/migrations/ directory is gone, and
#   2. the entire workspace contains at most one distinct sqlx::migrate! source
#      path (comments/docs are not scanned; only *.rs sources).
#
# Usage: scripts/ci/check_single_migrations_folder.sh
#
# Environment: none.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

failed=0

# ── 1. The stale secondary lineage must not exist. ───────────────────────────
if [ -e "${ROOT}/crates/store/migrations" ]; then
  echo "SINGLE-MIGRATIONS: stale secondary lineage still exists: crates/store/migrations" >&2
  echo "SINGLE-MIGRATIONS: the authoritative lineage is the top-level migrations/ directory" >&2
  failed=1
fi

# ── 2. At most one sqlx::migrate! source path across the workspace. ──────────
# One batched grep over every .rs file (heavy build directories pruned) keeps
# this gate cheap on large workspaces; only bash/find/grep/sed/sort are needed.
if ! command -v find >/dev/null 2>&1; then
  echo "SINGLE-MIGRATIONS: find is required to scan the workspace" >&2
  exit 1
fi

matches="$(
  find "${ROOT}" -type d \( -name target -o -name .git -o -name node_modules \) -prune -o \
    -type f -name '*.rs' -exec grep -Hn 'sqlx::migrate!' {} + 2>/dev/null || true
)"
if [ -z "${matches}" ]; then
  echo "SINGLE-MIGRATIONS: WARN no sqlx::migrate! invocation found under ${ROOT}/**/*.rs" >&2
  if [ "${failed}" -ne 0 ]; then
    echo "single migrations folder check FAILED" >&2
    exit 1
  fi
  echo "single migrations folder check passed (no sqlx::migrate! source to compare)"
  exit 0
fi

paths="$(printf '%s\n' "${matches}" \
  | grep -oE 'sqlx::migrate!\([[:space:]]*r?#?"[^"]*"' \
  | sed -E 's/.*"([^"]*)".*/\1/' \
  | sort -u)"

if [ -z "${paths}" ]; then
  echo "SINGLE-MIGRATIONS: sqlx::migrate! found but no string-literal path could be extracted:" >&2
  printf '%s\n' "${matches}" >&2
  echo "single migrations folder check FAILED" >&2
  exit 1
fi

path_count="$(printf '%s\n' "${paths}" | grep -c . || true)"
if [ "${path_count}" -gt 1 ]; then
  echo "SINGLE-MIGRATIONS: more than one sqlx::migrate! source path is used in the workspace:" >&2
  while IFS= read -r p; do
    [ -z "${p}" ] && continue
    echo "SINGLE-MIGRATIONS:   path: ${p}" >&2
    printf '%s\n' "${matches}" | grep -F "\"${p}\"" | sed 's/^/SINGLE-MIGRATIONS:     /' >&2
  done <<< "${paths}"
  failed=1
fi

if [ "${failed}" -ne 0 ]; then
  echo "single migrations folder check FAILED" >&2
  exit 1
fi

echo "single migrations folder check passed (crates/store/migrations absent; 1 sqlx::migrate! source: $(printf '%s' "${paths}"))"
