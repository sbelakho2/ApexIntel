#!/usr/bin/env bash
# Migration transaction gate (audit #174).
#
# sqlx wraps every migration file in its own transaction before handing it to
# PostgreSQL. A migration that opens its own transaction (`BEGIN;` ... `COMMIT;`)
# therefore either nests the wrapper transaction (a warning-free no-op at best)
# or, on a runner that executes the file outside the wrapper, commits state that
# the wrapper later rolls back. Either way the migration's atomicity contract is
# broken, and the two styles must not be mixed inside one embedded migrator.
#
# This gate scans the migrations that are still PENDING relative to the frozen
# production revision (the same boundary scripts/ci/check_migration_headers.sh
# uses: FROZEN_MAX, default 78) and fails on any top-level `BEGIN;` or `COMMIT;`.
# The scanner is quote-aware: transaction keywords inside dollar-quoted bodies
# (`DO $$ ... $$`, `CREATE FUNCTION ... AS $$ ... $$`) and inside single-quoted
# string literals or comments are ignored, so PL/pgSQL `BEGIN` blocks are legal.
#
# One pre-gate file is grandfathered: 082_app_users_session_version_positive.sql
# already contains a top-level BEGIN;/COMMIT; and is recorded in the
# `_sqlx_migrations` checksum history of every environment that ran it, so it
# cannot be edited without invalidating those histories. It is listed in
# GRANDFATHERED_EXPLICIT_TX below and reported as a warning, not a failure.
# New migrations must not be added to that list.
#
# Usage: scripts/ci/check_migration_no_explicit_tx.sh [migrations_dir]
#
# Environment:
#   FROZEN_MAX                  highest production-applied migration number
#                               (default: 78, matching check_migration_headers.sh)
#   GRANDFATHERED_EXPLICIT_TX   space-separated allowlist of already-applied
#                               migration filenames (default: the 082 file above)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIR="${1:-${ROOT}/migrations}"
FROZEN_MAX="${FROZEN_MAX:-78}"
GRANDFATHERED_EXPLICIT_TX="${GRANDFATHERED_EXPLICIT_TX:-082_app_users_session_version_positive.sql}"

shopt -s nullglob
files=("${DIR}"/*.sql)
shopt -u nullglob
if [ "${#files[@]}" -eq 0 ]; then
  echo "MIGRATION-NO-TX: no *.sql migrations found in ${DIR}" >&2
  exit 1
fi

# Quote/comment-aware scanner. Tracks dollar-quoted blocks ($tag$ ... $tag$),
# single-quoted literals and /* */ comments across lines; only code outside
# those is matched for a top-level transaction statement. Emits
# "<file>:<line>: <original text>" for each match.
scan_program='
function trim(s) { gsub(/^[ \t]+/, "", s); gsub(/[ \t]+$/, "", s); return s }
BEGIN { q = sprintf("%c", 39); in_dollar = 0; tag = ""; in_sq = 0; in_block = 0 }
{
  line = $0; out = ""; i = 1; n = length(line)
  while (i <= n) {
    if (in_dollar) {
      if (substr(line, i, length(tag)) == tag) { in_dollar = 0; i += length(tag) } else { i++ }
    } else if (in_sq) {
      c = substr(line, i, 1)
      if (c == q) { in_sq = 0; out = out c; i++ } else { i++ }
    } else if (in_block) {
      if (substr(line, i, 2) == "*/") { in_block = 0; i += 2 } else { i++ }
    } else {
      c = substr(line, i, 1)
      if (substr(line, i, 2) == "--") { i = n + 1 }
      else if (substr(line, i, 2) == "/*") { in_block = 1; i += 2; out = out " " }
      else if (c == q) { in_sq = 1; out = out c; i++ }
      else if (c == "$") {
        j = i + 1
        while (j <= n && substr(line, j, 1) ~ /[A-Za-z0-9_]/) j++
        if (j <= n && substr(line, j, 1) == "$") { tag = substr(line, i, j - i + 1); in_dollar = 1; i = j + 1 }
        else { out = out c; i++ }
      }
      else { out = out c; i++ }
    }
  }
  t = tolower(out)
  if (t ~ /(^|;)[ \t]*(begin|commit)([ \t]+(work|transaction))?[ \t]*;/) {
    printf "%s:%d: %s\n", FILENAME, FNR, trim(line)
  }
}
'

failed=0
checked=0
frozen=0
grandfathered=0
hits=""

is_grandfathered() {
  local candidate="$1" entry
  for entry in ${GRANDFATHERED_EXPLICIT_TX}; do
    [ "${entry}" = "${candidate}" ] && return 0
  done
  return 1
}

for f in "${files[@]}"; do
  base="$(basename "$f")"
  if ! printf '%s' "${base}" | grep -Eq '^[0-9]+_'; then
    echo "MIGRATION-NO-TX: ${base}: filename does not start with a migration number" >&2
    failed=1
    continue
  fi
  num="$(printf '%s' "${base}" | grep -oE '^[0-9]+')"
  norm="$(printf '%d' "$((10#${num}))")"

  if [ "${norm}" -le "${FROZEN_MAX}" ]; then
    frozen=$((frozen + 1))
    continue
  fi

  checked=$((checked + 1))

  file_hits=""
  while IFS= read -r hit; do
    [ -z "${hit}" ] && continue
    file_hits="${file_hits}${hit}"$'\n'
  done < <(awk "${scan_program}" "$f")

  if [ -n "${file_hits}" ]; then
    if is_grandfathered "${base}"; then
      grandfathered=$((grandfathered + 1))
      echo "MIGRATION-NO-TX: WARN ${base}: grandfathered explicit transaction statement(s);" >&2
      printf '%s' "${file_hits}" | sed 's/^/MIGRATION-NO-TX: WARN   /' >&2
    else
      failed=1
      hits="${hits}${file_hits}"
    fi
  fi
done

if [ "${failed}" -ne 0 ]; then
  printf '%s' "${hits}" | sed 's/^/MIGRATION-NO-TX: /' >&2
  echo "MIGRATION-NO-TX: top-level BEGIN;/COMMIT; is forbidden in pending migrations (sqlx wraps each migration in a transaction)" >&2
  echo "migration no-explicit-transaction check FAILED" >&2
  exit 1
fi

if [ "${grandfathered}" -ne 0 ]; then
  echo "migration no-explicit-transaction check passed (${checked} pending checked, ${frozen} frozen, ${grandfathered} grandfathered)"
else
  echo "migration no-explicit-transaction check passed (${checked} pending checked, ${frozen} frozen)"
fi
