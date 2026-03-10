#!/usr/bin/env bash

set -euo pipefail

BACKUP_PATH="${1:-}"
: "${BACKUP_PATH:?Usage: scripts/restore_validate.sh <backup_dir_or_pgdump>}"
: "${DATABASE_URL:?Set DATABASE_URL before running restore validation}"
: "${RESTORE_VALIDATE_DATABASE_URL:?Set RESTORE_VALIDATE_DATABASE_URL before running restore validation}"

if [[ -d "${BACKUP_PATH}" ]]; then
  DUMP_PATH="${BACKUP_PATH}/apexintel.pgdump"
else
  DUMP_PATH="${BACKUP_PATH}"
fi

: "${PAGE_WEBHOOK_URL:=}"

notify_failure() {
  local message="$1"
  if [[ -n "${PAGE_WEBHOOK_URL}" ]]; then
    curl -fsS -X POST -H 'Content-Type: application/json' \
      -d "{\"text\":\"ApexIntel restore validation failed: ${message}\"}" \
      "${PAGE_WEBHOOK_URL}" >/dev/null || true
  fi
}

cleanup_db() {
  dropdb --if-exists "${RESTORE_VALIDATE_DATABASE_URL}" >/dev/null 2>&1 || true
}

trap cleanup_db EXIT

cleanup_db
createdb "${RESTORE_VALIDATE_DATABASE_URL}"
pg_restore --clean --if-exists --no-owner --no-privileges -d "${RESTORE_VALIDATE_DATABASE_URL}" "${DUMP_PATH}"

TABLE_COUNT=$(psql "${RESTORE_VALIDATE_DATABASE_URL}" -Atc "select count(*) from information_schema.tables where table_schema = 'public';")
if [[ "${TABLE_COUNT}" -lt 10 ]]; then
  notify_failure "restored schema only contains ${TABLE_COUNT} public tables"
  echo "restore validation failed: too few tables restored" >&2
  exit 1
fi

echo "restore validation ok: ${TABLE_COUNT} public tables present"