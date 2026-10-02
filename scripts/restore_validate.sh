#!/usr/bin/env bash
# scripts/restore_validate.sh — restore a backup into a scratch database and
# check it is usable.
# Usage: scripts/restore_validate.sh <backup_dir_or_pgdump>
#
# RESTORE_VALIDATE_DATABASE_URL names the scratch database; it is dropped and
# recreated on every run, so it must never be the live DATABASE_URL database.
# Encrypted dumps (.age) need BACKUP_AGE_IDENTITY.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/backup_common.sh
source "${SCRIPT_DIR}/lib/backup_common.sh"

BACKUP_PATH="${1:-}"
: "${BACKUP_PATH:?Usage: scripts/restore_validate.sh <backup_dir_or_pgdump>}"
: "${DATABASE_URL:?Set DATABASE_URL before running restore validation}"
: "${RESTORE_VALIDATE_DATABASE_URL:?Set RESTORE_VALIDATE_DATABASE_URL before running restore validation}"
: "${PAGE_WEBHOOK_URL:=}"

notify_failure() {
  local message="$1"
  echo "restore validation failed: ${message}" >&2
  if [[ -n "${PAGE_WEBHOOK_URL}" ]]; then
    local payload
    # Keep the payload valid JSON whatever the message (paths) contains.
    payload=$(printf '{"text":"ApexIntel restore validation failed: %s"}' \
      "$(printf '%s' "${message}" | tr -d '"\\' | tr '\n\r\t' '   ')")
    curl -fsS -X POST -H 'Content-Type: application/json' -d "${payload}" \
      "${PAGE_WEBHOOK_URL}" >/dev/null || true
  fi
}

pg_url_split "${DATABASE_URL}"
LIVE_TARGET="${PG_SERVER}/${PG_DB_NAME}"
pg_url_split "${RESTORE_VALIDATE_DATABASE_URL}"
SCRATCH_DB="${PG_DB_NAME}"
SCRATCH_MAINT_URL="${PG_MAINT_URL}"
if [[ "${PG_SERVER}/${SCRATCH_DB}" == "${LIVE_TARGET}" ]]; then
  echo "ERROR: RESTORE_VALIDATE_DATABASE_URL points at the live database; refusing to drop it" >&2
  exit 1
fi

resolve_dump_path "${BACKUP_PATH}"

# dropdb/createdb take a database *name*; the server comes from the
# maintenance URL.
cleanup_db() {
  dropdb --if-exists --maintenance-db="${SCRATCH_MAINT_URL}" "${SCRATCH_DB}" >/dev/null 2>&1 || true
}
trap cleanup_db EXIT

cleanup_db
createdb --maintenance-db="${SCRATCH_MAINT_URL}" "${SCRATCH_DB}"

if ! stream_dump | pg_restore --no-owner --no-privileges -d "${RESTORE_VALIDATE_DATABASE_URL}"; then
  notify_failure "pg_restore failed for ${DUMP_PATH}"
  exit 1
fi

TABLE_COUNT=$(psql "${RESTORE_VALIDATE_DATABASE_URL}" -Atc "select count(*) from information_schema.tables where table_schema = 'public';")
if [[ "${TABLE_COUNT}" -lt 10 ]]; then
  notify_failure "restored schema only contains ${TABLE_COUNT} public tables"
  exit 1
fi

MIGRATIONS=$(psql "${RESTORE_VALIDATE_DATABASE_URL}" -Atc "select count(*) from _sqlx_migrations where success;" 2>/dev/null || echo 0)
if [[ "${MIGRATIONS}" -lt 1 ]]; then
  notify_failure "restored database has no applied migrations recorded"
  exit 1
fi

echo "restore validation ok: ${TABLE_COUNT} public tables, ${MIGRATIONS} migrations present"
