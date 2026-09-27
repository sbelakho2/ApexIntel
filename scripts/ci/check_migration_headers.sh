#!/usr/bin/env bash
# Migration identity gate (audit #40; extended for stale body comments).
#
# Every migration that is still pending relative to the production revision
# must state its own migration number:
#   1. in its header comment (the first self-identification in the first 40
#      comment lines, e.g. "Migration 069" or "069_name.sql", must equal the
#      number in the filename), and
#   2. everywhere else in its comments and SQL string literals. A renumbered
#      file used to keep a stale body claim such as "-- Migration 061: ..." or
#      "(migration 069)" below the header; the header check alone could not see
#      it. Every singular "migration NNN" token must name the file's own
#      number. Cross-references to other revisions must stay visibly
#      historical: plural "Migrations 059/065", bare numbers "054/063", or
#      filenames "030_port_missing_tables.sql".
#
# Production-applied migrations (000..065 and 068 — the revisions recorded in
# `_sqlx_migrations`) are FROZEN: their bytes are the sqlx checksums every
# deployed database has on record, so editing one would make production refuse
# to start. They are allowlisted explicitly in scripts/ci/frozen_migrations.txt
# and skipped; several of them still describe an older revision (054, 055,
# 063, 064, 065, 068), which is exactly why the allowlist is explicit rather
# than a silent numeric threshold.
#
# Usage: scripts/ci/check_migration_headers.sh [migrations_dir]
#
# Environment:
#   FROZEN_MANIFEST  path to the frozen allowlist (default: the repo file)
#   FROZEN_MAX       highest production-applied migration number (default 68);
#                    every migration at or below it must be allowlisted, and
#                    nothing above it may be.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIR="${1:-${ROOT}/migrations}"
FROZEN_MANIFEST="${FROZEN_MANIFEST:-${ROOT}/scripts/ci/frozen_migrations.txt}"
FROZEN_MAX="${FROZEN_MAX:-68}"

failed=0
checked=0
frozen=0

shopt -s nullglob
files=("${DIR}"/*.sql)
shopt -u nullglob
if [ "${#files[@]}" -eq 0 ]; then
  echo "MIGRATION-HEADER: no *.sql migrations found in ${DIR}" >&2
  exit 1
fi

if [ ! -f "${FROZEN_MANIFEST}" ]; then
  echo "MIGRATION-HEADER: frozen allowlist not found: ${FROZEN_MANIFEST}" >&2
  exit 1
fi
frozen_names="$(
  sed -e 's/[[:space:]]*#.*$//' -e '/^[[:space:]]*$/d' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
    "${FROZEN_MANIFEST}"
)"
is_frozen() {
  printf '%s\n' "${frozen_names}" | grep -Fxq -- "$1"
}

# normalize <digits> -> base-10 integer without leading zeros.
normalize() {
  printf '%d' "$((10#$1))"
}

# first_identity_claim <file>: the first "Migration NNN" / "NNN_name.sql"
# mention in the file's header comment block, or empty.
first_identity_claim() {
  sed -n '1,40p' "$1" | grep -E '^[[:space:]]*--' \
    | grep -oE 'Migration[[:space:]]+[0-9]+|[0-9]{3}_[A-Za-z0-9_]*\.sql' \
    | head -n1 || true
}

# body_identity_claims <file>: "line-number<TAB>claimed-number" for every
# body identity claim in the whole file:
#   * a leading comment that is itself an identity claim:
#     "-- Migration NNN:" or a bare "-- NNN_name.sql"
#   * the singular token "migration NNN" anywhere (comments or SQL string
#     literals). Cross-references to other revisions must stay visibly
#     historical: plural "Migrations 059/065", bare numbers "054/063", or
#     filenames "030_port_missing_tables.sql".
body_identity_claims() {
  awk '
    {
      line = $0
      sub(/^[[:space:]]*--[[:space:]]*/, "", line)
      if (line ~ /^Migration[[:space:]]+[0-9]+[[:space:]]*:/) {
        number = line
        sub(/^Migration[[:space:]]+/, "", number)
        sub(/[^0-9].*$/, "", number)
        printf "%d\t%s\n", NR, number
      } else if (line ~ /^[0-9][0-9][0-9]_[A-Za-z0-9_]*\.sql[[:space:]]*:?[[:space:]]*$/) {
        number = line
        sub(/_.*$/, "", number)
        printf "%d\t%s\n", NR, number
      }
    }
  ' "$1"
  # "migration NNN" (singular) anywhere on the line; grep -o yields one
  # "line:match" record per occurrence.
  grep -noE '[Mm]igration[[:space:]]+[0-9]{3}([^0-9]|$)' "$1" \
    | sed -E 's/^([0-9]+):[^0-9]*([0-9]{3}).*/\1\t\2/' || true
}

for f in "${files[@]}"; do
  base="$(basename "$f")"

  if ! printf '%s' "${base}" | grep -Eq '^[0-9]+_'; then
    echo "MIGRATION-HEADER: ${base}: filename does not start with a migration number" >&2
    failed=1
    continue
  fi

  num="$(printf '%s' "${base}" | grep -oE '^[0-9]+')"
  norm="$(normalize "${num}")"

  if is_frozen "${base}"; then
    if [ "${norm}" -gt "${FROZEN_MAX}" ]; then
      echo "MIGRATION-HEADER: ${base}: allowlisted as frozen but is above the applied head ${FROZEN_MAX}" >&2
      failed=1
      continue
    fi
    frozen=$((frozen + 1))
    continue
  fi

  if [ "${norm}" -le "${FROZEN_MAX}" ]; then
    echo "MIGRATION-HEADER: ${base}: production-applied migration (<= ${FROZEN_MAX}) is missing from ${FROZEN_MANIFEST}" >&2
    failed=1
    continue
  fi

  checked=$((checked + 1))

  claim="$(first_identity_claim "$f")"
  if [ -z "${claim}" ]; then
    echo "MIGRATION-HEADER: ${base}: header does not state its migration number" >&2
    failed=1
  else
    claim_num="$(printf '%s' "${claim}" | grep -oE '[0-9]+' | head -n1)"
    claim_norm="$(normalize "${claim_num}")"
    if [ "${claim_norm}" -ne "${norm}" ]; then
      echo "MIGRATION-HEADER: ${base}: header claims '${claim}' (migration ${claim_norm}), filename says ${num}" >&2
      failed=1
    fi
  fi

  while IFS=$'\t' read -r line_no body_num; do
    [ -z "${line_no}" ] && continue
    body_norm="$(normalize "${body_num}")"
    if [ "${body_norm}" -ne "${norm}" ]; then
      echo "MIGRATION-HEADER: ${base}: comment on line ${line_no} claims migration ${body_num}, filename says ${num}" >&2
      failed=1
    fi
  done < <(body_identity_claims "$f")
done

if [ "${failed}" -ne 0 ]; then
  echo "migration header check FAILED" >&2
  exit 1
fi

echo "migration header check passed (${checked} pending checked, ${frozen} production-applied frozen)"
