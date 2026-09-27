#!/usr/bin/env bash
# Migration header gate (audit #40).
#
# Every migration that is still pending relative to the production revision
# must state its own migration number in its header comment, so a renumbered
# file can no longer keep describing the revision it used to have (e.g. 063
# describing 061). The first self-identification found in the header is the
# one that counts, and it must equal the number in the filename.
#
# Migrations 000..068 are the revisions production has recorded as applied
# (checksums in `_sqlx_migrations`). Their bytes (and
# therefore their sqlx checksums) are immutable: editing one would make every
# deployed database refuse to start. They are counted as frozen and skipped.
#
# Usage: scripts/ci/check_migration_headers.sh [migrations_dir]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIR="${1:-${ROOT}/migrations}"
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

for f in "${files[@]}"; do
  base="$(basename "$f")"

  if ! printf '%s' "${base}" | grep -Eq '^[0-9]+_'; then
    echo "MIGRATION-HEADER: ${base}: filename does not start with a migration number" >&2
    failed=1
    continue
  fi

  num="$(printf '%s' "${base}" | grep -oE '^[0-9]+')"
  norm="$((10#${num}))"
  if [ "${norm}" -le "${FROZEN_MAX}" ]; then
    frozen=$((frozen + 1))
    continue
  fi
  checked=$((checked + 1))

  header="$(sed -n '1,40p' "$f" | grep -E '^[[:space:]]*--' || true)"
  if [ -z "${header}" ]; then
    echo "MIGRATION-HEADER: ${base}: no header comment found" >&2
    failed=1
    continue
  fi

  # First self-identification: "Migration 069" or "069_name.sql".
  claim="$(printf '%s\n' "${header}" \
    | grep -oE 'Migration[[:space:]]+[0-9]+|[0-9]{3}_[A-Za-z0-9_]*\.sql' \
    | head -n1 || true)"
  if [ -z "${claim}" ]; then
    echo "MIGRATION-HEADER: ${base}: header does not state its migration number" >&2
    failed=1
    continue
  fi

  claim_num="$(printf '%s' "${claim}" | grep -oE '[0-9]+' | head -n1)"
  claim_norm="$((10#${claim_num}))"
  if [ "${claim_norm}" -ne "${norm}" ]; then
    echo "MIGRATION-HEADER: ${base}: header claims '${claim}' (migration ${claim_norm}), filename says ${num}" >&2
    failed=1
  fi
done

if [ "${failed}" -ne 0 ]; then
  echo "migration header check FAILED" >&2
  exit 1
fi

echo "migration header check passed (${checked} pending checked, ${frozen} production-applied frozen)"
